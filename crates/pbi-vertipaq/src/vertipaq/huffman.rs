//! Canonical Huffman decoding of compressed string-dictionary pages.
//!
//! [MS-XLDM] §2.3.2.1.2.4.2 and §2.7.4: a compressed page carries a 128-byte
//! `encodeArray` of 4-bit codeword lengths for a 256-symbol (byte) alphabet — symbol
//! `2i` in the low nibble of byte `i`, symbol `2i + 1` in the high nibble — and a bit
//! stream in which every string is a run of codewords starting at the bit offset given
//! by its record handle and ending at the next string's offset (or `StoreTotalBits`).
//!
//! Codes are canonical (shorter codes first, then by symbol value). The stream is a
//! sequence of 16-bit little-endian words read most-significant bit first, i.e. the
//! bytes of every pair are swapped and the result is read MSB-first (an odd trailing
//! byte is not swapped). These conventions follow xmhuffman (MIT, reference #11 in
//! `REFERENCES.md`), re-implemented here with a single lookup table of `2^max_len`
//! entries.

use super::error::VertipaqError;

const WHAT: &str = "dictionary Huffman page";

/// Longest codeword allowed by [MS-XLDM] §2.7.4.1.2.
pub(crate) const MAX_CODE_LEN: u8 = 15;

/// Decoding table built from an `encodeArray`.
#[derive(Debug, Clone)]
pub(crate) struct HuffmanTable {
    /// `(symbol << 8) | length`, indexed by the next `max_len` bits; 0 = unused code.
    table: Vec<u16>,
    max_len: u8,
}

impl HuffmanTable {
    /// Builds the table. Rejects lengths above 15 and over-subscribed codes; incomplete
    /// codes are accepted (unused codewords fail at decode time).
    pub(crate) fn new(encode_array: &[u8]) -> Result<Self, VertipaqError> {
        if encode_array.len() != 128 {
            return Err(VertipaqError::corrupt(
                WHAT,
                format!(
                    "encode array has {} bytes, expected 128",
                    encode_array.len()
                ),
            ));
        }
        let mut lengths = [0u8; 256];
        for (i, byte) in encode_array.iter().enumerate() {
            lengths[2 * i] = byte & 0x0F;
            lengths[2 * i + 1] = byte >> 4;
        }
        let mut count = [0u32; MAX_CODE_LEN as usize + 1];
        let mut max_len = 0u8;
        for &len in &lengths {
            if len > 0 {
                count[usize::from(len)] += 1;
                max_len = max_len.max(len);
            }
        }
        if max_len == 0 {
            return Ok(Self {
                table: Vec::new(),
                max_len: 0,
            });
        }
        // Kraft: the code must not be over-subscribed.
        let mut left: i64 = 1;
        for &n in count.iter().take(usize::from(max_len) + 1).skip(1) {
            left = left * 2 - i64::from(n);
            if left < 0 {
                return Err(VertipaqError::corrupt(
                    WHAT,
                    "codeword lengths are over-subscribed",
                ));
            }
        }
        let mut next_code = [0u32; MAX_CODE_LEN as usize + 2];
        let mut code = 0u32;
        for len in 1..=usize::from(max_len) {
            code = (code + count[len - 1]) << 1;
            next_code[len] = code;
        }
        let mut table = vec![0u16; 1usize << max_len];
        for (symbol, &len) in lengths.iter().enumerate() {
            if len == 0 {
                continue;
            }
            let len_usize = usize::from(len);
            let code = next_code[len_usize];
            next_code[len_usize] += 1;
            let pad = u32::from(max_len - len);
            let start = (code << pad) as usize;
            let span = 1usize << pad;
            let entry = ((symbol as u16) << 8) | u16::from(len);
            let slots = table.get_mut(start..start + span).ok_or_else(|| {
                VertipaqError::corrupt(WHAT, "codeword lengths are over-subscribed")
            })?;
            slots.fill(entry);
        }
        Ok(Self { table, max_len })
    }

    /// Decodes the symbols in bits `start .. end` of `stream` into `out`, failing once
    /// `out` would exceed `max_symbols`.
    pub(crate) fn decode(
        &self,
        stream: &BitStream<'_>,
        start: u64,
        end: u64,
        out: &mut Vec<u8>,
        max_symbols: usize,
    ) -> Result<(), VertipaqError> {
        if start >= end {
            return Ok(());
        }
        if self.max_len == 0 {
            return Err(VertipaqError::corrupt(
                WHAT,
                "non-empty string on a page without codewords",
            ));
        }
        let max_len = u32::from(self.max_len);
        let mask = (1u32 << max_len) - 1;
        let mut bit = start;
        while bit < end {
            let index = (stream.peek(bit, max_len) & mask) as usize;
            let entry = self.table.get(index).copied().unwrap_or(0);
            let len = entry & 0xFF;
            if len == 0 {
                return Err(VertipaqError::corrupt(
                    WHAT,
                    format!("invalid codeword at bit {bit}"),
                ));
            }
            if out.len() >= max_symbols {
                return Err(VertipaqError::LimitExceeded {
                    what: "decoded dictionary string bytes",
                    value: out.len() as u64 + 1,
                    limit: max_symbols as u64,
                });
            }
            out.push((entry >> 8) as u8);
            bit += u64::from(len);
        }
        Ok(())
    }
}

/// A compressed string buffer read as pair-swapped bytes, MSB first.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BitStream<'a> {
    bytes: &'a [u8],
    /// Bytes that belong to complete 16-bit pairs.
    paired: usize,
}

impl<'a> BitStream<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            paired: bytes.len() & !1,
        }
    }

    /// Length in bits.
    pub(crate) fn bit_len(&self) -> u64 {
        (self.bytes.len() as u64) * 8
    }

    /// Byte `i` of the swapped stream (zero past the end).
    fn byte(&self, i: u64) -> u32 {
        let Ok(i) = usize::try_from(i) else {
            return 0;
        };
        let source = if i < self.paired { i ^ 1 } else { i };
        u32::from(self.bytes.get(source).copied().unwrap_or(0))
    }

    /// The `n <= 25` bits starting at bit `bit`, right-aligned.
    fn peek(&self, bit: u64, n: u32) -> u32 {
        let first = bit / 8;
        let window = (self.byte(first) << 24)
            | (self.byte(first + 1) << 16)
            | (self.byte(first + 2) << 8)
            | self.byte(first + 3);
        let offset = (bit % 8) as u32;
        (window << offset) >> (32 - n)
    }
}
