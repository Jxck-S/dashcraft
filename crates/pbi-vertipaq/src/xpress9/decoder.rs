//! XPress9 block format and the session decoder.
//!
//! An XPress9 *session* is a sequence of *blocks* that share one LZ77 history window.
//! Each block is a 32-byte header followed by an LSB-first bit stream:
//!
//! ```text
//! header (8 x u32 LE): magic 0x4E86D72A | original size (bytes) | encoded size (bits,
//!                      including the 256 header bits) | flags | reserved (0) |
//!                      session signature | block index | CRC-32C of the first 28 bytes
//! flags:  bits 0..13  encoded Huffman table size in bits (incl. MTF state)
//!         bits 13..16 log2(window) - 16
//!         bits 16..18 MTF entry count / 2 (0, 2 or 4)
//!         bit  18     minimum pointer match length - 3
//!         bit  19     minimum MTF match length - 2
//!         bits 20..32 reserved, zero
//! body:   [MTF state] short-symbol code lengths, long-length code lengths, symbols
//! ```
//!
//! Short symbols 0..256 are literals. A short symbol `256 + slot * 16 + l` is a match:
//! `l == 15` escapes to the long-length code. Slots below the MTF entry count reuse a
//! recent offset (move-to-front list); other slots carry `slot - mtf` extra offset bits.
//!
//! This is a clean Rust implementation of the decoding rules of Microsoft's MIT-licensed
//! XPress9 reference decoder (`Xpress9DecLz77.c`, `Xpress9Lz77Dec.i`); see
//! `crates/pbi-vertipaq/REFERENCES.md` and `FORMAT-NOTES.md`.

use super::Limits;
use super::bitreader::BitReader;
use super::crc32c::crc32c;
use super::error::Xpress9Error;
use super::huffman::{Huffman, read_code_lengths};

pub(crate) const BLOCK_MAGIC: u32 = 0x4E86_D72A;
pub(crate) const BLOCK_HEADER_LEN: usize = 32;
const HEADER_BITS: u64 = (BLOCK_HEADER_LEN as u64) * 8;

/// The alphabet layout reserves offset slots for windows up to 2^24 and 4 MTF entries.
const ALPHABET_WINDOW_LOG2: usize = 24;
const ALPHABET_MTF: usize = 4;
const SHORT_LENGTH_BITS: u32 = 4;
const SHORT_LENGTH_ESCAPE: usize = (1 << SHORT_LENGTH_BITS) - 1;
pub(crate) const SHORT_ALPHABET_SIZE: usize =
    256 + ((ALPHABET_WINDOW_LOG2 + ALPHABET_MTF) << SHORT_LENGTH_BITS);
pub(crate) const LONG_ALPHABET_SIZE: usize = 256;
/// Long-length symbols at or above this value carry `symbol - 232` extra bits.
const LONG_LENGTH_DIRECT: u32 = (LONG_ALPHABET_SIZE - ALPHABET_WINDOW_LOG2) as u32;

/// Upper bound for the up-front output reservation of a block; beyond it the buffer
/// grows (fallibly) as data is actually produced.
const INITIAL_RESERVE_CAP: usize = 16 << 20;

/// Stream parameters fixed for a whole XPress9 session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamParams {
    /// log2 of the LZ77 window size (16..=23).
    pub window_log2: u32,
    /// Number of move-to-front offset entries (0, 2 or 4).
    pub mtf_entries: u32,
    /// Minimum length of an explicit-offset match (3 or 4).
    pub ptr_min_match: u32,
    /// Minimum length of an MTF match (2 or 3).
    pub mtf_min_match: u32,
}

impl StreamParams {
    /// Window size in bytes.
    #[must_use]
    pub fn window_size(&self) -> usize {
        1usize << self.window_log2
    }
}

/// A parsed and validated XPress9 block header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BlockHeader {
    pub orig_size: u32,
    pub encoded_bits: u32,
    pub table_bits: u32,
    pub params: StreamParams,
    pub session_signature: u32,
    pub block_index: u32,
}

impl BlockHeader {
    pub(crate) fn parse(bytes: &[u8; BLOCK_HEADER_LEN]) -> Result<Self, Xpress9Error> {
        let mut w = [0u32; 8];
        for (word, raw) in w.iter_mut().zip(bytes.as_chunks::<4>().0) {
            *word = u32::from_le_bytes(*raw);
        }
        let [
            magic,
            orig_size,
            encoded_bits,
            flags,
            reserved,
            session,
            index,
            stored_crc,
        ] = w;
        if magic != BLOCK_MAGIC {
            return Err(Xpress9Error::BadHeader("bad block magic"));
        }
        if reserved != 0 {
            return Err(Xpress9Error::BadHeader("reserved header field is not zero"));
        }
        let computed = crc32c(&bytes[..28]);
        if computed != stored_crc {
            return Err(Xpress9Error::HeaderChecksum {
                stored: stored_crc,
                computed,
            });
        }
        if flags >> 20 != 0 {
            return Err(Xpress9Error::BadHeader("reserved flag bits are set"));
        }
        let mtf_entries = ((flags >> 16) & 3) * 2;
        if mtf_entries > ALPHABET_MTF as u32 {
            return Err(Xpress9Error::BadHeader("invalid MTF entry count"));
        }
        let table_bits = flags & 0x1FFF;
        if u64::from(encoded_bits) <= u64::from(table_bits) + HEADER_BITS {
            return Err(Xpress9Error::BadHeader("encoded size is too small"));
        }
        Ok(Self {
            orig_size,
            encoded_bits,
            table_bits,
            params: StreamParams {
                window_log2: ((flags >> 13) & 7) + 16,
                mtf_entries,
                ptr_min_match: ((flags >> 18) & 1) + 3,
                mtf_min_match: ((flags >> 19) & 1) + 2,
            },
            session_signature: session,
            block_index: index,
        })
    }

    /// Size of the whole block (header included) in bytes.
    pub(crate) fn block_len(&self) -> u64 {
        u64::from(self.encoded_bits).div_ceil(8)
    }
}

fn reserve(buf: &mut Vec<u8>, additional: usize) -> Result<(), Xpress9Error> {
    buf.try_reserve(additional)
        .map_err(|_| Xpress9Error::OutOfMemory(additional as u64))
}

/// Appends `len` bytes copied from `offset` bytes back (`1 <= offset <= buf.len()`),
/// with LZ77 overlap semantics.
fn copy_match(buf: &mut Vec<u8>, offset: usize, len: usize) -> Result<(), Xpress9Error> {
    reserve(buf, len)?;
    let start = buf.len() - offset;
    if offset >= len {
        buf.extend_from_within(start..start + len);
    } else if offset == 1 {
        let byte = buf[start];
        buf.resize(buf.len() + len, byte);
    } else {
        // The output is periodic from `start`, so the copyable source doubles each pass.
        let mut remaining = len;
        while remaining > 0 {
            let n = remaining.min(buf.len() - start);
            buf.extend_from_within(start..start + n);
            remaining -= n;
        }
    }
    Ok(())
}

/// A reusable XPress9 session decoder (the equivalent of one reference-library decoder
/// instance).
///
/// Chunks passed to [`Xpress9Decoder::decompress_chunk`] continue the same session and
/// may reference history from earlier chunks. Call [`Xpress9Decoder::reset`] to start a
/// new, independent session. After an error the decoder refuses further input until it
/// is reset.
pub struct Xpress9Decoder {
    limits: Limits,
    params: Option<StreamParams>,
    session_signature: u32,
    next_block: u32,
    /// Session history followed by the output of the most recent chunk.
    buf: Vec<u8>,
    /// Start of the most recent chunk's output in `buf`.
    out_start: usize,
    session_total: u64,
    failed: bool,
    short: Huffman,
    long: Huffman,
    meta: Huffman,
    short_lengths: Vec<u8>,
    long_lengths: Vec<u8>,
}

impl std::fmt::Debug for Xpress9Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Xpress9Decoder")
            .field("limits", &self.limits)
            .field("params", &self.params)
            .field("session_signature", &self.session_signature)
            .field("next_block", &self.next_block)
            .field("buffered", &self.buf.len())
            .field("session_total", &self.session_total)
            .field("failed", &self.failed)
            .finish_non_exhaustive()
    }
}

impl Default for Xpress9Decoder {
    fn default() -> Self {
        Self::new(Limits::default())
    }
}

impl Xpress9Decoder {
    /// Creates a decoder that enforces `limits` (per chunk, and in total per session).
    #[must_use]
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            params: None,
            session_signature: 0,
            next_block: 0,
            buf: Vec::new(),
            out_start: 0,
            session_total: 0,
            failed: false,
            short: Huffman::new(),
            long: Huffman::new(),
            meta: Huffman::new(),
            short_lengths: vec![0; SHORT_ALPHABET_SIZE],
            long_lengths: vec![0; LONG_ALPHABET_SIZE],
        }
    }

    /// Discards the session state and history so the next chunk starts a new session.
    pub fn reset(&mut self) {
        self.params = None;
        self.session_signature = 0;
        self.next_block = 0;
        self.buf.clear();
        self.out_start = 0;
        self.session_total = 0;
        self.failed = false;
    }

    /// Parameters of the current session, once its first block has been seen.
    #[must_use]
    pub fn params(&self) -> Option<StreamParams> {
        self.params
    }

    /// Output of the most recent successful [`Xpress9Decoder::decompress_chunk`] call.
    #[must_use]
    pub fn output(&self) -> &[u8] {
        self.buf.get(self.out_start..).unwrap_or_default()
    }

    /// Decodes one chunk — one or more consecutive whole XPress9 blocks of this session —
    /// that must expand to exactly `uncompressed_len` bytes.
    ///
    /// Returns the decoded bytes (valid until the next call).
    pub fn decompress_chunk(
        &mut self,
        compressed: &[u8],
        uncompressed_len: usize,
    ) -> Result<&[u8], Xpress9Error> {
        if self.failed {
            return Err(Xpress9Error::DecoderFailed);
        }
        match self.decompress_chunk_inner(compressed, uncompressed_len) {
            Ok(()) => Ok(self.output()),
            Err(err) => {
                self.failed = true;
                self.out_start = self.buf.len();
                Err(err)
            }
        }
    }

    fn decompress_chunk_inner(
        &mut self,
        compressed: &[u8],
        uncompressed_len: usize,
    ) -> Result<(), Xpress9Error> {
        let limits = self.limits;
        let comp_len = compressed.len() as u64;
        if comp_len > limits.max_chunk_compressed_bytes {
            return Err(Xpress9Error::LimitExceeded {
                what: "compressed chunk size",
                value: comp_len,
                limit: limits.max_chunk_compressed_bytes,
            });
        }
        let declared = uncompressed_len as u64;
        if declared > limits.max_chunk_uncompressed_bytes {
            return Err(Xpress9Error::LimitExceeded {
                what: "uncompressed chunk size",
                value: declared,
                limit: limits.max_chunk_uncompressed_bytes,
            });
        }
        let new_total = self.session_total.saturating_add(declared);
        if new_total > limits.max_output_bytes {
            return Err(Xpress9Error::LimitExceeded {
                what: "total decompressed size",
                value: new_total,
                limit: limits.max_output_bytes,
            });
        }

        self.trim_history();
        self.out_start = self.buf.len();

        let mut offset = 0usize;
        while offset < compressed.len() {
            let rest = compressed.get(offset..).unwrap_or_default();
            let header_bytes = rest
                .get(..BLOCK_HEADER_LEN)
                .and_then(|h| <&[u8; BLOCK_HEADER_LEN]>::try_from(h).ok())
                .ok_or(Xpress9Error::Truncated {
                    what: "XPress9 block header",
                    offset: offset as u64,
                })?;
            let header = BlockHeader::parse(header_bytes)?;
            self.check_session(&header)?;
            let block = usize::try_from(header.block_len())
                .ok()
                .and_then(|len| rest.get(..len))
                .ok_or(Xpress9Error::Truncated {
                    what: "XPress9 block body",
                    offset: offset as u64,
                })?;
            let produced = self.buf.len() - self.out_start;
            let would_be = produced as u64 + u64::from(header.orig_size);
            if would_be > declared {
                return Err(Xpress9Error::SizeMismatch {
                    what: "chunk",
                    declared,
                    actual: would_be,
                });
            }
            self.decode_block(block, &header)?;
            self.next_block = self.next_block.wrapping_add(1);
            offset += block.len();
        }

        let produced = (self.buf.len() - self.out_start) as u64;
        if produced != declared {
            return Err(Xpress9Error::SizeMismatch {
                what: "chunk",
                declared,
                actual: produced,
            });
        }
        self.session_total = new_total;
        Ok(())
    }

    /// Drops history that can no longer be referenced, amortised so that at most about
    /// one window of bytes is moved per window of output.
    fn trim_history(&mut self) {
        if let Some(params) = self.params {
            let window = params.window_size();
            if self.buf.len() >= window.saturating_mul(2) {
                let cut = self.buf.len() - window;
                self.buf.drain(..cut);
            }
        }
    }

    fn check_session(&mut self, header: &BlockHeader) -> Result<(), Xpress9Error> {
        match self.params {
            None => {
                if header.block_index != 0 {
                    return Err(Xpress9Error::BlockSequence {
                        expected: 0,
                        found: header.block_index,
                    });
                }
                self.params = Some(header.params);
                self.session_signature = header.session_signature;
                self.next_block = 0;
            }
            Some(params) => {
                if header.session_signature != self.session_signature {
                    return Err(Xpress9Error::SessionMismatch {
                        expected: self.session_signature,
                        found: header.session_signature,
                    });
                }
                if header.block_index != self.next_block {
                    return Err(Xpress9Error::BlockSequence {
                        expected: self.next_block,
                        found: header.block_index,
                    });
                }
                if header.params != params {
                    return Err(Xpress9Error::BadHeader(
                        "block parameters differ from the session's first block",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Decodes one block (`block` includes the 32-byte header) and appends its output.
    fn decode_block(&mut self, block: &[u8], header: &BlockHeader) -> Result<(), Xpress9Error> {
        let params = header.params;
        let body = block.get(BLOCK_HEADER_LEN..).unwrap_or_default();
        let mut reader = BitReader::new(body);
        let body_bits = u64::from(header.encoded_bits) - HEADER_BITS;
        let window = params.window_size();
        let mtf_count = params.mtf_entries as usize;
        let mtf_min = params.mtf_min_match as usize;
        let ptr_min = params.ptr_min_match as usize;

        // Initial move-to-front state.
        let mut mtf = [0usize; ALPHABET_MTF];
        let mut last_was_match = false;
        if mtf_count > 0 {
            last_was_match = reader.read(1) != 0;
            for entry in mtf.iter_mut().take(mtf_count) {
                let msb = reader.read(5);
                if msb >= params.window_log2 {
                    return Err(Xpress9Error::Corrupt("MTF offset exceeds the window"));
                }
                *entry = reader.read(msb) as usize + (1usize << msb);
            }
        }

        read_code_lengths(&mut reader, &mut self.meta, &mut self.short_lengths)?;
        self.short.build(&self.short_lengths)?;
        read_code_lengths(&mut reader, &mut self.meta, &mut self.long_lengths)?;
        self.long.build(&self.long_lengths)?;
        if reader.bits_consumed() != u64::from(header.table_bits) {
            return Err(Xpress9Error::BadHuffmanTable(
                "encoded table size does not match the block header",
            ));
        }

        let orig = usize::try_from(header.orig_size)
            .map_err(|_| Xpress9Error::OutOfMemory(u64::from(header.orig_size)))?;
        let buf = &mut self.buf;
        reserve(buf, orig.min(INITIAL_RESERVE_CAP))?;
        let target = buf
            .len()
            .checked_add(orig)
            .ok_or(Xpress9Error::OutOfMemory(u64::from(header.orig_size)))?;
        let short = &self.short;
        let long = &self.long;

        while buf.len() < target {
            if reader.bits_consumed() > body_bits {
                return Err(Xpress9Error::Corrupt("symbol stream overruns the block"));
            }
            let symbol = usize::from(short.decode(&mut reader)?);
            if symbol < 256 {
                if buf.len() == buf.capacity() {
                    reserve(buf, 1)?;
                }
                buf.push(symbol as u8);
                last_was_match = false;
                continue;
            }

            let code = symbol - 256;
            let slot = code >> SHORT_LENGTH_BITS;
            let mut len = code & SHORT_LENGTH_ESCAPE;
            if len == SHORT_LENGTH_ESCAPE {
                let mut long_len = u32::from(long.decode(&mut reader)?);
                if long_len >= LONG_LENGTH_DIRECT {
                    let extra_bits = long_len - LONG_LENGTH_DIRECT;
                    long_len =
                        reader.read(extra_bits) + (1u32 << extra_bits) + (LONG_LENGTH_DIRECT - 1);
                }
                len = long_len as usize + SHORT_LENGTH_ESCAPE;
            }

            let offset;
            if slot < mtf_count {
                // Directly after a match, entry 0 would only extend that match, so the
                // encoder skips it and slot k refers to entry k + 1.
                len += mtf_min;
                let index = slot + usize::from(last_was_match);
                if index >= mtf_count {
                    return Err(Xpress9Error::Corrupt("invalid MTF reference"));
                }
                offset = mtf[index];
                mtf.copy_within(0..index, 1);
                mtf[0] = offset;
            } else {
                len += ptr_min;
                let extra_bits = (slot - mtf_count) as u32;
                offset = reader.read(extra_bits) as usize + (1usize << extra_bits);
                if mtf_count > 0 {
                    mtf.copy_within(0..mtf_count - 1, 1);
                    mtf[0] = offset;
                }
            }
            last_was_match = true;

            if offset > window || offset > buf.len() {
                return Err(Xpress9Error::Corrupt(
                    "match offset reaches outside the LZ77 window",
                ));
            }
            if len > target - buf.len() {
                return Err(Xpress9Error::Corrupt(
                    "match runs past the end of the block",
                ));
            }
            copy_match(buf, offset, len)?;
        }

        if reader.bits_consumed() != body_bits {
            return Err(Xpress9Error::Corrupt(
                "decoded bit length does not match the block header",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xpress9::test_support::{BlockSpec, Tok, encode_block, expand};

    fn decode(chunks: &[Vec<u8>], sizes: &[usize]) -> Result<Vec<u8>, Xpress9Error> {
        let mut dec = Xpress9Decoder::default();
        let mut out = Vec::new();
        for (chunk, &size) in chunks.iter().zip(sizes) {
            out.extend_from_slice(dec.decompress_chunk(chunk, size)?);
        }
        Ok(out)
    }

    fn roundtrip(spec: &BlockSpec) {
        let block = encode_block(spec);
        let want = expand(&[], spec);
        let got = decode(&[block], &[want.len()]).unwrap();
        assert_eq!(got, want);
    }

    #[test]
    fn literals_and_matches() {
        let mut toks: Vec<Tok> = b"abcabc".iter().map(|&b| Tok::Lit(b)).collect();
        toks.push(Tok::Ptr { len: 9, offset: 3 }); // overlapping copy
        toks.push(Tok::Ptr { len: 40, offset: 1 }); // run
        toks.push(Tok::Lit(b'z'));
        toks.push(Tok::Ptr {
            len: 10,
            offset: 12,
        }); // non-overlapping
        roundtrip(&BlockSpec::new(toks));
    }

    #[test]
    fn long_lengths_with_extra_bits() {
        for len in [18, 19, 250, 251, 252, 253, 300, 5000, 70_000] {
            let toks = vec![Tok::Lit(7), Tok::Lit(9), Tok::Ptr { len, offset: 2 }];
            roundtrip(&BlockSpec::new(toks));
        }
    }

    #[test]
    fn mtf_semantics() {
        for mtf in [2u32, 4] {
            for ptr_min in [3u32, 4] {
                for mtf_min in [2u32, 3] {
                    let mut toks: Vec<Tok> = (0..64u8).map(Tok::Lit).collect();
                    toks.push(Tok::Ptr { len: 5, offset: 7 });
                    toks.push(Tok::Lit(1));
                    toks.push(Tok::Mtf { len: 4, slot: 0 });
                    toks.push(Tok::Mtf { len: 3, slot: 0 }); // after a match: entry 1
                    toks.push(Tok::Ptr { len: 6, offset: 33 });
                    toks.push(Tok::Mtf {
                        len: 3,
                        slot: mtf - 2,
                    });
                    toks.push(Tok::Lit(2));
                    toks.push(Tok::Mtf {
                        len: 3,
                        slot: mtf - 1,
                    });
                    let mut spec = BlockSpec::new(toks);
                    spec.mtf_entries = mtf;
                    spec.ptr_min = ptr_min;
                    spec.mtf_min = mtf_min;
                    spec.mtf_init = vec![1, 5, 9, 17][..mtf as usize].to_vec();
                    spec.mtf_last_match = mtf == 4;
                    roundtrip(&spec);
                }
            }
        }
    }

    #[test]
    fn history_spans_blocks_and_chunks() {
        let first = BlockSpec::new(b"0123456789".iter().map(|&b| Tok::Lit(b)).collect());
        let mut second = BlockSpec::new(vec![Tok::Ptr {
            len: 10,
            offset: 10,
        }]);
        second.block_index = 1;
        let mut third = BlockSpec::new(vec![Tok::Lit(b'!'), Tok::Ptr { len: 4, offset: 21 }]);
        third.block_index = 2;
        let b1 = encode_block(&first);
        let b2 = encode_block(&second);
        let b3 = encode_block(&third);
        // Two blocks in one chunk, then a third chunk.
        let chunk1 = [b1.clone(), b2.clone()].concat();
        let got = decode(&[chunk1, b3.clone()], &[20, 5]).unwrap();
        assert_eq!(got, b"01234567890123456789!0123".to_vec());

        // A reset decoder must not see the old history.
        let mut dec = Xpress9Decoder::default();
        dec.decompress_chunk(&b1, 10).unwrap();
        dec.reset();
        assert!(matches!(
            dec.decompress_chunk(&b2, 10),
            Err(Xpress9Error::BlockSequence {
                expected: 0,
                found: 1
            })
        ));
    }

    #[test]
    fn session_errors() {
        let first = encode_block(&BlockSpec::new(vec![Tok::Lit(1)]));
        let mut spec = BlockSpec::new(vec![Tok::Lit(2)]);
        spec.block_index = 2;
        let skipped = encode_block(&spec);
        assert!(matches!(
            decode(&[first.clone(), skipped], &[1, 1]),
            Err(Xpress9Error::BlockSequence {
                expected: 1,
                found: 2
            })
        ));

        let mut spec = BlockSpec::new(vec![Tok::Lit(2)]);
        spec.block_index = 1;
        spec.session = 0xDEAD;
        let other_session = encode_block(&spec);
        assert!(matches!(
            decode(&[first.clone(), other_session], &[1, 1]),
            Err(Xpress9Error::SessionMismatch { .. })
        ));

        let mut spec = BlockSpec::new(vec![Tok::Lit(2)]);
        spec.block_index = 1;
        spec.window_log2 = 17;
        let other_params = encode_block(&spec);
        assert!(matches!(
            decode(&[first.clone(), other_params], &[1, 1]),
            Err(Xpress9Error::BadHeader(_))
        ));

        // A failed decoder stays failed until reset.
        let mut dec = Xpress9Decoder::default();
        assert!(dec.decompress_chunk(&first, 2).is_err());
        assert!(matches!(
            dec.decompress_chunk(&first, 1),
            Err(Xpress9Error::DecoderFailed)
        ));
        dec.reset();
        assert_eq!(dec.decompress_chunk(&first, 1).unwrap(), &[1]);
    }

    #[test]
    fn header_errors() {
        let good = encode_block(&BlockSpec::new(vec![Tok::Lit(1), Tok::Lit(2)]));
        let patch = |at: usize, value: u32, fix_crc: bool| {
            let mut b = good.clone();
            b[at..at + 4].copy_from_slice(&value.to_le_bytes());
            if fix_crc {
                let crc = crc32c(&b[..28]);
                b[28..32].copy_from_slice(&crc.to_le_bytes());
            }
            b
        };
        let flags = u32::from_le_bytes(good[12..16].try_into().unwrap());
        let cases = [
            (patch(0, 0x1234_5678, true), "magic"),
            (patch(16, 1, true), "reserved"),
            (patch(28, 0, false), "crc"),
            (patch(12, flags | 1 << 25, true), "flag bits"),
            (patch(12, flags | 3 << 16, true), "mtf=6"),
            (patch(8, 256, true), "too small"),
        ];
        for (block, what) in cases {
            let err = decode(&[block], &[2]).unwrap_err();
            assert!(
                matches!(
                    err,
                    Xpress9Error::BadHeader(_) | Xpress9Error::HeaderChecksum { .. }
                ),
                "{what}: {err}"
            );
        }
    }

    #[test]
    fn corrupt_stream_errors() {
        // Offset reaching before the start of the session.
        let block = encode_block(&BlockSpec::new(vec![
            Tok::Lit(1),
            Tok::Ptr { len: 4, offset: 2 },
        ]));
        assert!(matches!(
            decode(&[block], &[5]),
            Err(Xpress9Error::Corrupt(_))
        ));

        // Offset larger than the 64 KiB window, even though the history is long enough.
        let mut toks = vec![
            Tok::Lit(1),
            Tok::Ptr {
                len: 70_000,
                offset: 1,
            },
        ];
        toks.push(Tok::Ptr {
            len: 4,
            offset: 65_537,
        });
        let spec = BlockSpec::new(toks);
        let block = encode_block(&spec);
        assert!(matches!(
            decode(&[block], &[70_005]),
            Err(Xpress9Error::Corrupt(_))
        ));
        let mut spec_ok = BlockSpec::new(vec![
            Tok::Lit(1),
            Tok::Ptr {
                len: 70_000,
                offset: 1,
            },
        ]);
        spec_ok.tokens.push(Tok::Ptr {
            len: 4,
            offset: 65_536,
        });
        roundtrip(&spec_ok);

        // A match that runs past the declared block size.
        let mut spec = BlockSpec::new(vec![Tok::Lit(1), Tok::Ptr { len: 8, offset: 1 }]);
        spec.orig_size = Some(5);
        let block = encode_block(&spec);
        assert!(matches!(
            decode(&[block], &[5]),
            Err(Xpress9Error::Corrupt(_))
        ));

        // Declared bit length longer than the symbols actually used.
        let mut spec = BlockSpec::new(vec![Tok::Lit(1), Tok::Lit(2)]);
        spec.extra_bits = 9;
        let block = encode_block(&spec);
        assert!(matches!(
            decode(&[block], &[2]),
            Err(Xpress9Error::Corrupt(_))
        ));

        // Declared table size that does not match the tables.
        let mut spec = BlockSpec::new(vec![Tok::Lit(1)]);
        spec.table_bits_delta = 1;
        let block = encode_block(&spec);
        assert!(matches!(
            decode(&[block], &[1]),
            Err(Xpress9Error::BadHuffmanTable(_))
        ));

        // Invalid MTF reference: with 2 entries, slot 1 right after a match.
        let mut spec = BlockSpec::new(vec![
            Tok::Lit(1),
            Tok::Lit(1),
            Tok::Ptr { len: 3, offset: 1 },
        ]);
        spec.mtf_entries = 2;
        spec.mtf_init = vec![1, 2];
        spec.tokens.push(Tok::Mtf { len: 2, slot: 1 });
        let block = encode_block(&spec);
        assert!(matches!(
            decode(&[block], &[7]),
            Err(Xpress9Error::Corrupt(_))
        ));

        // MTF initial offset beyond the window.
        let mut spec = BlockSpec::new(vec![Tok::Lit(1)]);
        spec.mtf_entries = 2;
        spec.mtf_init = vec![1, 1 << 16];
        let block = encode_block(&spec);
        assert!(matches!(
            decode(&[block], &[1]),
            Err(Xpress9Error::Corrupt(_))
        ));

        // An empty block can never satisfy the bit-length check.
        let block = encode_block(&BlockSpec::new(Vec::new()));
        assert!(decode(&[block], &[0]).is_err());
    }

    #[test]
    fn chunk_size_errors() {
        let block = encode_block(&BlockSpec::new(vec![Tok::Lit(1), Tok::Lit(2)]));
        assert!(matches!(
            decode(std::slice::from_ref(&block), &[3]),
            Err(Xpress9Error::SizeMismatch { .. })
        ));
        assert!(matches!(
            decode(std::slice::from_ref(&block), &[1]),
            Err(Xpress9Error::SizeMismatch { .. })
        ));
        for cut in 0..block.len() {
            let res = decode(&[block[..cut].to_vec()], &[2]);
            assert!(res.is_err(), "cut at {cut}");
        }
        let mut long = block.clone();
        long.push(0);
        assert!(matches!(
            decode(&[long], &[2]),
            Err(Xpress9Error::Truncated { .. })
        ));
        assert_eq!(decode(&[Vec::new()], &[0]).unwrap(), Vec::<u8>::new());

        let limits = Limits {
            max_chunk_uncompressed_bytes: 1,
            ..Limits::default()
        };
        let mut dec = Xpress9Decoder::new(limits);
        assert!(matches!(
            dec.decompress_chunk(&block, 2),
            Err(Xpress9Error::LimitExceeded { .. })
        ));
    }
}
