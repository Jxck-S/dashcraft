//! Test-only XPress9 block writer.
//!
//! Emits blocks with "stored" Huffman tables (implicit fixed-length codes), which is
//! enough to hand-construct valid and deliberately malformed streams.

use super::crc32c::crc32c;
use super::decoder::{BLOCK_MAGIC, LONG_ALPHABET_SIZE, SHORT_ALPHABET_SIZE};

pub(crate) struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    pending: u32,
    total: u64,
}

impl BitWriter {
    pub(crate) fn new() -> Self {
        Self {
            out: Vec::new(),
            acc: 0,
            pending: 0,
            total: 0,
        }
    }

    pub(crate) fn put(&mut self, value: u32, bits: u32) {
        assert!(bits <= 32);
        if bits == 0 {
            return;
        }
        let masked = u64::from(value) & ((1u64 << bits) - 1);
        self.acc |= masked << self.pending;
        self.pending += bits;
        self.total += u64::from(bits);
        while self.pending >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.pending -= 8;
        }
    }

    pub(crate) fn bit_len(&self) -> u64 {
        self.total
    }

    pub(crate) fn finish(mut self) -> Vec<u8> {
        if self.pending > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Tok {
    Lit(u8),
    Ptr { len: usize, offset: usize },
    Mtf { len: usize, slot: u32 },
}

pub(crate) struct BlockSpec {
    pub tokens: Vec<Tok>,
    pub window_log2: u32,
    pub mtf_entries: u32,
    pub ptr_min: u32,
    pub mtf_min: u32,
    pub mtf_init: Vec<usize>,
    pub mtf_last_match: bool,
    pub session: u32,
    pub block_index: u32,
    pub orig_size: Option<u32>,
    pub extra_bits: u32,
    pub table_bits_delta: u32,
}

impl BlockSpec {
    pub(crate) fn new(tokens: Vec<Tok>) -> Self {
        Self {
            tokens,
            window_log2: 16,
            mtf_entries: 0,
            ptr_min: 3,
            mtf_min: 2,
            mtf_init: Vec::new(),
            mtf_last_match: false,
            session: 0x1234_5678,
            block_index: 0,
            orig_size: None,
            extra_bits: 0,
            table_bits_delta: 0,
        }
    }
}

fn msb(value: usize) -> u32 {
    usize::BITS - 1 - value.leading_zeros()
}

/// (codeword, length) of each symbol under the "stored" table encoding.
fn stored_codes(alphabet: usize) -> Vec<(u32, u32)> {
    let m = msb(alphabet);
    let short = (1usize << (m + 1)) - alphabet;
    (0..alphabet)
        .map(|s| {
            if s < short {
                (s as u32, m)
            } else {
                ((2 * short + (s - short)) as u32, m + 1)
            }
        })
        .collect()
}

fn put_code(w: &mut BitWriter, (code, len): (u32, u32)) {
    w.put(code.reverse_bits() >> (32 - len), len);
}

fn put_length(
    w: &mut BitWriter,
    short: &[(u32, u32)],
    long: &[(u32, u32)],
    slot: usize,
    len: usize,
) {
    if len < 15 {
        put_code(w, short[256 + slot * 16 + len]);
        return;
    }
    put_code(w, short[256 + slot * 16 + 15]);
    let l = len - 15;
    if l < 232 {
        put_code(w, long[l]);
    } else {
        let nb = msb(l - 231);
        put_code(w, long[232 + nb as usize]);
        w.put((l - 231 - (1 << nb)) as u32, nb);
    }
}

pub(crate) fn encode_block(spec: &BlockSpec) -> Vec<u8> {
    let short = stored_codes(SHORT_ALPHABET_SIZE);
    let long = stored_codes(LONG_ALPHABET_SIZE);
    let mut w = BitWriter::new();
    if spec.mtf_entries > 0 {
        w.put(u32::from(spec.mtf_last_match), 1);
        for &offset in &spec.mtf_init {
            let m = msb(offset);
            w.put(m, 5);
            w.put((offset - (1 << m)) as u32, m);
        }
    }
    w.put(0, 3);
    w.put(0, 3);
    let table_bits = w.bit_len() as u32 + spec.table_bits_delta;

    let mut orig = 0usize;
    for tok in &spec.tokens {
        match *tok {
            Tok::Lit(b) => {
                put_code(&mut w, short[usize::from(b)]);
                orig += 1;
            }
            Tok::Ptr { len, offset } => {
                let nb = msb(offset);
                let slot = nb as usize + spec.mtf_entries as usize;
                put_length(&mut w, &short, &long, slot, len - spec.ptr_min as usize);
                w.put((offset - (1 << nb)) as u32, nb);
                orig += len;
            }
            Tok::Mtf { len, slot } => {
                put_length(
                    &mut w,
                    &short,
                    &long,
                    slot as usize,
                    len - spec.mtf_min as usize,
                );
                orig += len;
            }
        }
    }
    for _ in 0..spec.extra_bits {
        w.put(0, 1);
    }
    let body_bits = w.bit_len();
    let body = w.finish();

    let flags = table_bits
        | (spec.window_log2 - 16) << 13
        | (spec.mtf_entries / 2) << 16
        | (spec.ptr_min - 3) << 18
        | (spec.mtf_min - 2) << 19;
    let words = [
        BLOCK_MAGIC,
        spec.orig_size.unwrap_or(orig as u32),
        (256 + body_bits) as u32,
        flags,
        0,
        spec.session,
        spec.block_index,
    ];
    let mut out: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    let crc = crc32c(&out);
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// Reference expansion of `spec.tokens` after `history` (returns only the new bytes).
pub(crate) fn expand(history: &[u8], spec: &BlockSpec) -> Vec<u8> {
    let mut out = history.to_vec();
    let start = out.len();
    let mut mtf = spec.mtf_init.clone();
    let mut last_match = spec.mtf_last_match;
    let copy = |out: &mut Vec<u8>, offset: usize, len: usize| {
        for _ in 0..len {
            out.push(out[out.len() - offset]);
        }
    };
    for tok in &spec.tokens {
        match *tok {
            Tok::Lit(b) => {
                out.push(b);
                last_match = false;
            }
            Tok::Ptr { len, offset } => {
                copy(&mut out, offset, len);
                if !mtf.is_empty() {
                    mtf.insert(0, offset);
                    mtf.pop();
                }
                last_match = true;
            }
            Tok::Mtf { len, slot } => {
                let index = slot as usize + usize::from(last_match);
                let offset = mtf.remove(index);
                mtf.insert(0, offset);
                copy(&mut out, offset, len);
                last_match = true;
            }
        }
    }
    out.split_off(start)
}
