//! Canonical Huffman decoding and the XPress9 code-length table format.
//!
//! XPress9 uses length-limited (≤ 27 bit) canonical prefix codes. Codewords are
//! assigned in increasing numeric order to symbols sorted by (length, symbol value) —
//! the same convention as DEFLATE — and are stored most-significant bit first inside
//! the LSB-first bit stream, so a table index is the bit-reversed codeword.
//!
//! Code lengths are transmitted either "stored" (implicit near-uniform lengths) or with a
//! small 33-symbol meta-code whose symbols mean "length N", "zeros to the next 16-symbol
//! row", "run of zeros", "repeat previous length", "same as 16 symbols back", or "one more
//! than 16 symbols back". Semantics follow Microsoft's MIT-licensed XPress9 reference
//! decoder (see `crates/pbi-vertipaq/REFERENCES.md`).

use super::bitreader::BitReader;
use super::error::Xpress9Error;

/// Longest codeword the format allows.
pub(crate) const MAX_CODEWORD_LEN: u32 = 27;
const MAX_LEN: usize = MAX_CODEWORD_LEN as usize;

/// Width of the single-level lookup table; longer codewords take the slow path.
const FAST_BITS: u32 = 12;
const FAST_SIZE: usize = 1 << FAST_BITS;
const FAST_MASK: u32 = (1 << FAST_BITS) - 1;

/// Packed fast-table entry: `symbol << 5 | length`; 0 means "not in the fast table".
const LEN_FIELD_BITS: u32 = 5;
const LEN_FIELD_MASK: u16 = (1 << LEN_FIELD_BITS) - 1;
/// Largest alphabet whose symbols fit next to the 5-bit length in a `u16` entry.
const MAX_ALPHABET: usize = 1 << (16 - LEN_FIELD_BITS);

/// Meta-alphabet used to transmit code lengths: 0..=27 are literal lengths, then 5 opcodes.
const META_ALPHABET_SIZE: usize = MAX_LEN + 6;
const META_FILL_ROW: u16 = MAX_CODEWORD_LEN as u16 + 1;
const META_ZERO_RUN: u16 = MAX_CODEWORD_LEN as u16 + 2;
const META_REPEAT_PREV: u16 = MAX_CODEWORD_LEN as u16 + 3;
const META_ROW_SAME: u16 = MAX_CODEWORD_LEN as u16 + 4;
const META_ROW_PLUS_ONE: u16 = MAX_CODEWORD_LEN as u16 + 5;
const ZERO_RUN_MIN: usize = 5;
/// Code-length tables are organised in rows of 16 symbols.
const ROW: usize = 16;
/// Meta-code lengths are limited to 8 bits.
const META_MAX_LEN: u32 = 8;

/// A decoding table for one canonical prefix code.
pub(crate) struct Huffman {
    fast: Box<[u16; FAST_SIZE]>,
    /// First canonical (MSB-first) codeword of each length.
    first_code: [u32; MAX_LEN + 1],
    /// Number of symbols of each length.
    count: [u32; MAX_LEN + 1],
    /// Index into `sorted` of the first symbol of each length.
    start: [u32; MAX_LEN + 1],
    /// Symbols ordered by (length, value).
    sorted: Vec<u16>,
}

impl std::fmt::Debug for Huffman {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Huffman")
            .field("count", &self.count)
            .field("symbols", &self.sorted.len())
            .finish_non_exhaustive()
    }
}

impl Huffman {
    pub(crate) fn new() -> Self {
        Self {
            fast: Box::new([0; FAST_SIZE]),
            first_code: [0; MAX_LEN + 1],
            count: [0; MAX_LEN + 1],
            start: [0; MAX_LEN + 1],
            sorted: Vec::new(),
        }
    }

    /// Builds the decoding table for `lengths[symbol]` (0 = symbol unused).
    ///
    /// The code must be complete (Kraft sum exactly 1), except that a code with a single
    /// symbol is accepted; that symbol then consumes its declared length on every decode.
    pub(crate) fn build(&mut self, lengths: &[u8]) -> Result<(), Xpress9Error> {
        if lengths.len() > MAX_ALPHABET {
            return Err(Xpress9Error::BadHuffmanTable("alphabet too large"));
        }
        self.fast.fill(0);
        self.count = [0; MAX_LEN + 1];
        let mut total = 0u32;
        for &len in lengths {
            let len = usize::from(len);
            if len > MAX_LEN {
                return Err(Xpress9Error::BadHuffmanTable(
                    "codeword length exceeds 27 bits",
                ));
            }
            if len != 0 {
                self.count[len] += 1;
                total += 1;
            }
        }
        match total {
            0 => return Err(Xpress9Error::BadHuffmanTable("code has no symbols")),
            1 => return self.build_single(lengths),
            _ => {}
        }

        let kraft: u64 = (1..=MAX_LEN)
            .map(|len| u64::from(self.count[len]) << (MAX_LEN - len))
            .sum();
        if kraft > 1 << MAX_LEN {
            return Err(Xpress9Error::BadHuffmanTable("code is over-subscribed"));
        }
        if kraft < 1 << MAX_LEN {
            return Err(Xpress9Error::BadHuffmanTable("code is incomplete"));
        }

        let mut code = 0u32;
        let mut index = 0u32;
        for len in 1..=MAX_LEN {
            code = (code + self.count[len - 1]) << 1;
            self.first_code[len] = code;
            self.start[len] = index;
            index += self.count[len];
        }

        self.sorted.clear();
        self.sorted.resize(total as usize, 0);
        let mut next = self.start;
        for (symbol, &len) in lengths.iter().enumerate() {
            if len != 0 {
                let slot = &mut next[usize::from(len)];
                if let Some(entry) = self.sorted.get_mut(*slot as usize) {
                    *entry = symbol as u16;
                }
                *slot += 1;
            }
        }

        for len in 1..=FAST_BITS as usize {
            let first = self.start[len] as usize;
            let symbols = self
                .sorted
                .get(first..first + self.count[len] as usize)
                .unwrap_or_default();
            for (rank, &symbol) in symbols.iter().enumerate() {
                let codeword = self.first_code[len] + rank as u32;
                let reversed = codeword.reverse_bits() >> (32 - len);
                let entry = (symbol << LEN_FIELD_BITS) | len as u16;
                let mut index = reversed as usize;
                while index < FAST_SIZE {
                    self.fast[index] = entry;
                    index += 1 << len;
                }
            }
        }
        Ok(())
    }

    fn build_single(&mut self, lengths: &[u8]) -> Result<(), Xpress9Error> {
        let Some((symbol, &len)) = lengths.iter().enumerate().find(|(_, len)| **len != 0) else {
            return Err(Xpress9Error::BadHuffmanTable("code has no symbols"));
        };
        // The reference decoder packs the length into a 4-bit field here.
        if len > 15 {
            return Err(Xpress9Error::BadHuffmanTable(
                "single-symbol code has an unsupported length",
            ));
        }
        self.sorted.clear();
        self.fast
            .fill(((symbol as u16) << LEN_FIELD_BITS) | u16::from(len));
        Ok(())
    }

    /// Decodes one symbol.
    #[inline(always)]
    pub(crate) fn decode(&self, reader: &mut BitReader<'_>) -> Result<u16, Xpress9Error> {
        reader.ensure(MAX_CODEWORD_LEN);
        let entry = self.fast[(reader.peek32() & FAST_MASK) as usize];
        if entry != 0 {
            reader.consume(u32::from(entry & LEN_FIELD_MASK));
            Ok(entry >> LEN_FIELD_BITS)
        } else {
            self.decode_slow(reader)
        }
    }

    #[cold]
    fn decode_slow(&self, reader: &mut BitReader<'_>) -> Result<u16, Xpress9Error> {
        // Next 27 stream bits, re-ordered so the first stream bit is the most significant.
        let msb_first = reader.peek32().reverse_bits() >> (32 - MAX_CODEWORD_LEN);
        for len in (FAST_BITS as usize + 1)..=MAX_LEN {
            let code = msb_first >> (MAX_LEN - len);
            let rank = code.wrapping_sub(self.first_code[len]);
            if rank < self.count[len] {
                reader.consume(len as u32);
                return self
                    .sorted
                    .get((self.start[len] + rank) as usize)
                    .copied()
                    .ok_or(Xpress9Error::Corrupt("invalid Huffman codeword"));
            }
        }
        Err(Xpress9Error::Corrupt("invalid Huffman codeword"))
    }
}

fn check_zero_run(i: usize, run: usize, alphabet: usize) -> Result<(), Xpress9Error> {
    let end = i + run;
    // A run must stay inside the current 16-symbol row (FILL_ROW covers the rest).
    if end > alphabet || (i ^ end) >= ROW {
        return Err(Xpress9Error::BadHuffmanTable(
            "zero run crosses a row boundary",
        ));
    }
    Ok(())
}

/// Reads an encoded code-length table into `out` (`out.len()` is the alphabet size).
///
/// `meta` is scratch space for the meta-code decoder.
pub(crate) fn read_code_lengths(
    reader: &mut BitReader<'_>,
    meta: &mut Huffman,
    out: &mut [u8],
) -> Result<(), Xpress9Error> {
    let alphabet = out.len();
    if alphabet == 0 {
        return Err(Xpress9Error::BadHuffmanTable("empty alphabet"));
    }
    match reader.read(3) {
        0 => {
            // "Stored": with M = floor(log2(alphabet)), the first 2^(M+1) - alphabet
            // symbols get M-bit codes and the remaining symbols M+1-bit codes.
            let msb = usize::BITS - 1 - alphabet.leading_zeros();
            let short = (1usize << (msb + 1)) - alphabet;
            for (i, len) in out.iter_mut().enumerate() {
                *len = if i < short {
                    msb as u8
                } else {
                    (msb + 1) as u8
                };
            }
            return Ok(());
        }
        1 => {}
        _ => {
            return Err(Xpress9Error::BadHuffmanTable(
                "unknown code-length table encoding",
            ));
        }
    }

    // Meta-code lengths: a 0 bit repeats the previous length, otherwise 3 bits give a
    // new length with the previous value skipped (so 8 is reachable).
    let mut meta_lengths = [0u8; META_ALPHABET_SIZE];
    let mut prev = 4u32;
    for len in &mut meta_lengths {
        if reader.read(1) == 0 {
            *len = prev as u8;
        } else {
            let mut value = reader.read(3);
            if value >= prev {
                value += 1;
                if value > META_MAX_LEN {
                    return Err(Xpress9Error::BadHuffmanTable("meta-code length exceeds 8"));
                }
            }
            *len = value as u8;
            prev = value;
        }
    }
    meta.build(&meta_lengths)?;

    let mut prev = 8u8;
    let mut i = 0usize;
    while i < alphabet {
        let symbol = meta.decode(reader)?;
        match symbol {
            0..=27 => {
                out[i] = symbol as u8;
                i += 1;
                if symbol != 0 {
                    prev = symbol as u8;
                }
            }
            META_FILL_ROW => loop {
                out[i] = 0;
                i += 1;
                if i.is_multiple_of(ROW) || i >= alphabet {
                    break;
                }
            },
            META_ZERO_RUN => {
                let mut extra = reader.read(2) as usize;
                let mut run = ZERO_RUN_MIN + extra;
                if extra == 3 {
                    loop {
                        extra = reader.read(3) as usize;
                        run += extra;
                        check_zero_run(i, run, alphabet)?;
                        if extra != 7 {
                            break;
                        }
                    }
                }
                check_zero_run(i, run, alphabet)?;
                out[i..i + run].fill(0);
                i += run;
            }
            META_REPEAT_PREV => {
                out[i] = prev;
                i += 1;
            }
            META_ROW_SAME | META_ROW_PLUS_ONE => {
                let Some(above) = i.checked_sub(ROW).map(|j| out[j]) else {
                    return Err(Xpress9Error::BadHuffmanTable(
                        "row reference in the first row",
                    ));
                };
                let len = if symbol == META_ROW_SAME {
                    above
                } else {
                    above.saturating_add(1)
                };
                if len == 0 || u32::from(len) > MAX_CODEWORD_LEN {
                    return Err(Xpress9Error::BadHuffmanTable(
                        "row reference yields an invalid length",
                    ));
                }
                out[i] = len;
                prev = len;
                i += 1;
            }
            _ => return Err(Xpress9Error::BadHuffmanTable("invalid meta-code symbol")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xpress9::test_support::BitWriter;

    fn canonical_codes(lengths: &[u8]) -> Vec<(u32, u32)> {
        // Independent DEFLATE-style assignment: (codeword, length) per symbol.
        let mut order: Vec<usize> = (0..lengths.len()).filter(|&s| lengths[s] != 0).collect();
        order.sort_by_key(|&s| (lengths[s], s));
        let mut codes = vec![(0, 0); lengths.len()];
        let mut code = 0u32;
        let mut prev_len = 0u32;
        for s in order {
            let len = u32::from(lengths[s]);
            code <<= len - prev_len;
            codes[s] = (code, len);
            code += 1;
            prev_len = len;
        }
        codes
    }

    fn write_symbol(w: &mut BitWriter, codes: &[(u32, u32)], symbol: usize) {
        let (code, len) = codes[symbol];
        w.put(code.reverse_bits() >> (32 - len), len);
    }

    #[test]
    fn decodes_short_and_long_codewords() {
        // Lengths 1, 2, 3, ..., 26, 27, 27: complete, with codes far beyond the fast table.
        let mut lengths: Vec<u8> = (1..=27).collect();
        lengths.push(27);
        let codes = canonical_codes(&lengths);
        let mut w = BitWriter::new();
        let sequence = [0usize, 27, 13, 12, 11, 26, 1, 0, 20];
        for &s in &sequence {
            write_symbol(&mut w, &codes, s);
        }
        let total = w.bit_len();
        let data = w.finish();
        let mut h = Huffman::new();
        h.build(&lengths).unwrap();
        let mut r = BitReader::new(&data);
        for &s in &sequence {
            assert_eq!(usize::from(h.decode(&mut r).unwrap()), s);
        }
        assert_eq!(r.bits_consumed(), total);
    }

    #[test]
    fn rejects_bad_codes() {
        let mut h = Huffman::new();
        assert!(matches!(
            h.build(&[0, 0, 0]),
            Err(Xpress9Error::BadHuffmanTable(_))
        ));
        assert!(matches!(
            h.build(&[1, 1, 1]),
            Err(Xpress9Error::BadHuffmanTable(_))
        ));
        assert!(matches!(
            h.build(&[1, 2, 0]),
            Err(Xpress9Error::BadHuffmanTable(_))
        ));
        assert!(matches!(
            h.build(&[28, 1]),
            Err(Xpress9Error::BadHuffmanTable(_))
        ));
        assert!(matches!(
            h.build(&[0, 16]),
            Err(Xpress9Error::BadHuffmanTable(_))
        ));
    }

    #[test]
    fn single_symbol_code_consumes_its_length() {
        let mut h = Huffman::new();
        h.build(&[0, 0, 3, 0]).unwrap();
        let data = [0u8; 4];
        let mut r = BitReader::new(&data);
        assert_eq!(h.decode(&mut r).unwrap(), 2);
        assert_eq!(h.decode(&mut r).unwrap(), 2);
        assert_eq!(r.bits_consumed(), 6);
    }

    #[test]
    fn stored_lengths() {
        let mut w = BitWriter::new();
        w.put(0, 3);
        let data = w.finish();
        let mut out = vec![0u8; 704];
        read_code_lengths(&mut BitReader::new(&data), &mut Huffman::new(), &mut out).unwrap();
        assert!(out[..320].iter().all(|&l| l == 9));
        assert!(out[320..].iter().all(|&l| l == 10));
        let mut out = vec![0u8; 256];
        read_code_lengths(&mut BitReader::new(&data), &mut Huffman::new(), &mut out).unwrap();
        assert!(out.iter().all(|&l| l == 8));
    }

    /// Writes a meta-code table where 31 meta symbols have length 5 and the last two have
    /// length 6, giving a complete code over the 33-symbol meta alphabet.
    fn write_meta_header(w: &mut BitWriter) -> Vec<(u32, u32)> {
        let mut meta = [5u8; META_ALPHABET_SIZE];
        meta[META_ALPHABET_SIZE - 2] = 6;
        meta[META_ALPHABET_SIZE - 1] = 6;
        w.put(1, 3);
        let mut prev = 4u32;
        for &len in &meta {
            let len = u32::from(len);
            if len == prev {
                w.put(0, 1);
            } else {
                w.put(1, 1);
                w.put(if len < prev { len } else { len - 1 }, 3);
                prev = len;
            }
        }
        canonical_codes(&meta)
    }

    #[test]
    fn meta_code_opcodes() {
        let mut w = BitWriter::new();
        let meta = write_meta_header(&mut w);
        let sym = |w: &mut BitWriter, s: u16| write_symbol(w, &meta, usize::from(s));
        // Row 0: 3, prev, 0, zero run of 5, fill to end of row.
        sym(&mut w, 3);
        sym(&mut w, META_REPEAT_PREV);
        sym(&mut w, 0);
        sym(&mut w, META_ZERO_RUN);
        w.put(0, 2);
        sym(&mut w, META_FILL_ROW);
        // Row 1: same-as-above (3), plus-one (4), extended zero run of 5 + 3 + 2 = 10,
        // then 27, then fill.
        sym(&mut w, META_ROW_SAME);
        sym(&mut w, META_ROW_PLUS_ONE);
        sym(&mut w, META_ZERO_RUN);
        w.put(3, 2);
        w.put(2, 3);
        sym(&mut w, 27);
        sym(&mut w, META_FILL_ROW);
        let data = w.finish();

        let mut out = vec![0xAAu8; 32];
        read_code_lengths(&mut BitReader::new(&data), &mut Huffman::new(), &mut out).unwrap();
        let mut want = vec![0u8; 32];
        want[0] = 3;
        want[1] = 3;
        want[16] = 3;
        want[17] = 4;
        want[28] = 27;
        assert_eq!(out, want);
    }

    #[test]
    fn meta_code_errors() {
        type Case<'a> = (&'a [u16], &'a [(u32, u32)]);
        let cases: [Case<'_>; 4] = [
            (&[META_ROW_SAME], &[]),
            (&[0, META_ROW_SAME], &[]),
            (&[META_ZERO_RUN], &[(3, 2), (7, 3), (7, 3)]),
            (&[META_ROW_PLUS_ONE], &[]),
        ];
        for (symbols, extra) in cases {
            let mut w = BitWriter::new();
            let meta = write_meta_header(&mut w);
            for &s in symbols {
                write_symbol(&mut w, &meta, usize::from(s));
            }
            for &(v, n) in extra {
                w.put(v, n);
            }
            let data = w.finish();
            let mut out = vec![0u8; 32];
            let err = read_code_lengths(&mut BitReader::new(&data), &mut Huffman::new(), &mut out);
            assert!(
                matches!(err, Err(Xpress9Error::BadHuffmanTable(_))),
                "{symbols:?}"
            );
        }

        // Unknown table encoding.
        let mut w = BitWriter::new();
        w.put(2, 3);
        let data = w.finish();
        let err = read_code_lengths(&mut BitReader::new(&data), &mut Huffman::new(), &mut [0; 8]);
        assert!(matches!(err, Err(Xpress9Error::BadHuffmanTable(_))));

        // All-zero 3-bit fields alternate meta lengths 0 and 1: an over-subscribed meta code.
        let mut w = BitWriter::new();
        w.put(1, 3);
        for _ in 0..META_ALPHABET_SIZE {
            w.put(1, 1);
            w.put(0, 3);
        }
        let data = w.finish();
        let err = read_code_lengths(&mut BitReader::new(&data), &mut Huffman::new(), &mut [0; 8]);
        assert!(matches!(err, Err(Xpress9Error::BadHuffmanTable(_))));
    }
}
