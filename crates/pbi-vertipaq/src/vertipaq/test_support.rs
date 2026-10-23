//! Writers for synthetic `.idfmeta`, `.idf` and `.dictionary` files (tests only; no
//! customer data). They mirror the layouts documented in the decoder modules.

#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::idfmeta::{HYBRID_RLE_CLASS, NO_SPLIT_CLASS_BASE};

/// One segment of a synthetic `.idfmeta`.
#[derive(Debug, Clone)]
pub(crate) struct SegSpec {
    pub records: u64,
    pub class: u32,
    pub sub_class: u32,
    pub min_data_id: u32,
    pub max_data_id: u32,
    pub has_nulls: bool,
    /// `Some(records)` writes a subsegment CS element.
    pub sub_records: Option<u64>,
}

impl SegSpec {
    pub(crate) fn hybrid(records: u64, bit_width: u8, min_data_id: u32, has_nulls: bool) -> Self {
        Self {
            records,
            class: HYBRID_RLE_CLASS,
            sub_class: NO_SPLIT_CLASS_BASE + u32::from(bit_width),
            min_data_id,
            max_data_id: min_data_id,
            has_nulls,
            sub_records: Some(records),
        }
    }

    pub(crate) fn packed(records: u64, bit_width: u8, min_data_id: u32) -> Self {
        Self {
            records,
            class: NO_SPLIT_CLASS_BASE + u32::from(bit_width),
            sub_class: 0,
            min_data_id,
            max_data_id: min_data_id,
            has_nulls: false,
            sub_records: None,
        }
    }
}

pub(crate) fn idfmeta_bytes(segments: &[SegSpec], sdos: bool) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(b"<1:CP\0");
    b.extend_from_slice(&(segments.len() as u64).to_le_bytes());
    for s in segments {
        b.extend_from_slice(b"<1:CS\0");
        b.extend_from_slice(&s.records.to_le_bytes());
        b.extend_from_slice(&1u64.to_le_bytes());
        b.extend_from_slice(&s.class.to_le_bytes());
        b.extend_from_slice(&s.sub_class.to_le_bytes());
        if s.class == HYBRID_RLE_CLASS {
            b.extend_from_slice(&3u64.to_le_bytes());
            b.extend_from_slice(&16u64.to_le_bytes());
            b.extend_from_slice(&2u64.to_le_bytes());
            b.push(0);
        }
        b.extend_from_slice(&s.min_data_id.to_le_bytes());
        b.extend_from_slice(b"<1:SS\0");
        b.extend_from_slice(&0u64.to_le_bytes());
        b.extend_from_slice(&s.min_data_id.to_le_bytes());
        b.extend_from_slice(&s.max_data_id.to_le_bytes());
        b.extend_from_slice(&s.min_data_id.to_le_bytes());
        b.extend_from_slice(&0i64.to_le_bytes());
        b.extend_from_slice(&s.records.to_le_bytes());
        b.push(u8::from(s.has_nulls));
        b.extend_from_slice(&1u64.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes());
        b.extend_from_slice(b"SS:1>\0");
        match s.sub_records {
            Some(records) => {
                b.push(1);
                b.extend_from_slice(b"<1:CS\0");
                b.extend_from_slice(&records.to_le_bytes());
                b.extend_from_slice(&0u64.to_le_bytes());
                b.push(0);
                b.extend_from_slice(b"CS:1>\0");
            }
            None => b.push(0),
        }
        b.extend_from_slice(b"CS:1>\0");
    }
    b.extend_from_slice(b"CP:1>\0");
    if sdos {
        b.extend_from_slice(b"<1:SDOs\0");
        for s in segments {
            b.extend_from_slice(b"<1:CSDOs\0");
            b.extend_from_slice(&16i64.to_le_bytes());
            b.extend_from_slice(&0i64.to_le_bytes());
            if s.sub_records.is_some() {
                b.extend_from_slice(b"<1:CSDOs\0");
                b.extend_from_slice(&8i64.to_le_bytes());
                b.extend_from_slice(&16i64.to_le_bytes());
                b.extend_from_slice(b"CSDOs:1>\0");
            }
            b.extend_from_slice(b"CSDOs:1>\0");
        }
        b.extend_from_slice(b"SDOs:1>\0");
    }
    b
}

/// Packs `values` (already minus the base) `64 / bit_width` per word, low bits first.
pub(crate) fn pack(values: &[u32], bit_width: u8) -> Vec<u64> {
    let width = u32::from(bit_width);
    let per_word = (64 / width) as usize;
    values
        .chunks(per_word)
        .map(|chunk| {
            chunk.iter().enumerate().fold(0u64, |word, (i, &v)| {
                word | (u64::from(v) << (i as u32 * width))
            })
        })
        .collect()
}

/// A hybrid segment: `u64 n`, `n` (value, repeat) entries, `u64 m`, `m` words.
pub(crate) fn hybrid_segment(entries: &[(u32, u32)], words: &[u64]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&(entries.len() as u64).to_le_bytes());
    for &(value, repeat) in entries {
        b.extend_from_slice(&value.to_le_bytes());
        b.extend_from_slice(&repeat.to_le_bytes());
    }
    b.extend_from_slice(&(words.len() as u64).to_le_bytes());
    for w in words {
        b.extend_from_slice(&w.to_le_bytes());
    }
    b
}

/// Encodes ids like the engine: runs of at least `min_run` equal ids become RLE
/// entries, everything else is bit-packed relative to `base`. Pads the primary segment
/// with `padding` zero entries.
pub(crate) fn encode_hybrid(
    ids: &[u32],
    base: u32,
    bit_width: u8,
    min_run: usize,
    padding: usize,
) -> Vec<u8> {
    let mut entries = Vec::new();
    let mut packed = Vec::new();
    let mut pending = 0u32;
    let mut i = 0;
    while i < ids.len() {
        let mut j = i;
        while j < ids.len() && ids[j] == ids[i] {
            j += 1;
        }
        if j - i >= min_run {
            if pending > 0 {
                entries.push((u32::MAX - (packed.len() as u32 - pending), pending));
                pending = 0;
            }
            entries.push((ids[i], (j - i) as u32));
        } else {
            for &id in &ids[i..j] {
                packed.push(id - base);
                pending += 1;
            }
        }
        i = j;
    }
    if pending > 0 {
        entries.push((u32::MAX - (packed.len() as u32 - pending), pending));
    }
    entries.extend(std::iter::repeat_n((0, 0), padding));
    let mut words = pack(&packed, bit_width);
    if words.is_empty() {
        words.push(0);
    }
    hybrid_segment(&entries, &words)
}

const HASH_INFO: [i32; 6] = [-1, 8, 64, 6, -1, -1];

fn dictionary_header(kind: i32) -> Vec<u8> {
    let mut b = kind.to_le_bytes().to_vec();
    for h in HASH_INFO {
        b.extend_from_slice(&h.to_le_bytes());
    }
    b
}

pub(crate) fn long_dictionary(values: &[i64], element: u32) -> Vec<u8> {
    let mut b = dictionary_header(0);
    b.extend_from_slice(&(values.len() as u64).to_le_bytes());
    b.extend_from_slice(&element.to_le_bytes());
    for &v in values {
        if element == 4 {
            b.extend_from_slice(&(v as i32).to_le_bytes());
        } else {
            b.extend_from_slice(&v.to_le_bytes());
        }
    }
    b
}

pub(crate) fn real_dictionary(values: &[f64]) -> Vec<u8> {
    let mut b = dictionary_header(1);
    b.extend_from_slice(&(values.len() as u64).to_le_bytes());
    b.extend_from_slice(&8u32.to_le_bytes());
    for v in values {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
}

/// Huffman character-set mode of a synthetic compressed page.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Charset {
    /// Single character set with this high byte.
    Single(u8),
    /// Raw UTF-16LE bytes.
    Multi,
}

/// A synthetic string-dictionary page.
#[derive(Debug, Clone)]
pub(crate) enum PageSpec {
    Plain(Vec<String>),
    Huffman(Vec<String>, Charset),
}

impl PageSpec {
    fn strings(&self) -> &[String] {
        match self {
            PageSpec::Plain(s) | PageSpec::Huffman(s, _) => s,
        }
    }
}

/// Huffman code lengths (≤ 15) for the byte frequencies `freq`.
pub(crate) fn huffman_lengths(freq: &[u64; 256]) -> [u8; 256] {
    let mut lengths = [0u8; 256];
    let used: Vec<usize> = (0..256).filter(|&s| freq[s] > 0).collect();
    match used.len() {
        0 => return lengths,
        1 => {
            lengths[used[0]] = 1;
            return lengths;
        }
        _ => {}
    }
    // Leaves are 0..256; internal nodes are appended. Depth = parent-chain length.
    let mut parent: Vec<usize> = vec![usize::MAX; 256];
    let mut heap: BinaryHeap<Reverse<(u64, usize)>> =
        used.iter().map(|&s| Reverse((freq[s], s))).collect();
    while heap.len() > 1 {
        let Reverse((fa, a)) = heap.pop().unwrap();
        let Reverse((fb, b)) = heap.pop().unwrap();
        let node = parent.len();
        parent.push(usize::MAX);
        parent[a] = node;
        parent[b] = node;
        heap.push(Reverse((fa + fb, node)));
    }
    for &s in &used {
        let mut depth = 0u8;
        let mut n = s;
        while parent[n] != usize::MAX {
            n = parent[n];
            depth += 1;
        }
        lengths[s] = depth;
    }
    assert!(lengths.iter().all(|&l| l <= 15), "test alphabet too skewed");
    lengths
}

/// Canonical codes for `lengths`.
pub(crate) fn canonical_codes(lengths: &[u8; 256]) -> [u32; 256] {
    let mut count = [0u32; 17];
    for &l in lengths {
        if l > 0 {
            count[usize::from(l)] += 1;
        }
    }
    let mut next = [0u32; 17];
    let mut code = 0u32;
    for len in 1..=16 {
        code = (code + count[len - 1]) << 1;
        next[len] = code;
    }
    let mut codes = [0u32; 256];
    for (s, &l) in lengths.iter().enumerate() {
        if l > 0 {
            codes[s] = next[usize::from(l)];
            next[usize::from(l)] += 1;
        }
    }
    codes
}

pub(crate) fn encode_array(lengths: &[u8; 256]) -> [u8; 128] {
    let mut out = [0u8; 128];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = lengths[2 * i] | (lengths[2 * i + 1] << 4);
    }
    out
}

fn symbols(s: &str, charset: Charset) -> Vec<u8> {
    match charset {
        Charset::Single(high) => s
            .encode_utf16()
            .map(|u| {
                assert_eq!(u >> 8, u16::from(high), "character outside the charset");
                (u & 0xFF) as u8
            })
            .collect(),
        Charset::Multi => s.encode_utf16().flat_map(u16::to_le_bytes).collect(),
    }
}

/// A compressed page: (swapped buffer, bit offsets, total bits, encode array).
pub(crate) fn huffman_page(
    strings: &[String],
    charset: Charset,
) -> (Vec<u8>, Vec<u32>, u32, [u8; 128]) {
    let mut freq = [0u64; 256];
    let encoded: Vec<Vec<u8>> = strings.iter().map(|s| symbols(s, charset)).collect();
    for e in &encoded {
        for &b in e {
            freq[usize::from(b)] += 1;
        }
    }
    let lengths = huffman_lengths(&freq);
    let codes = canonical_codes(&lengths);
    let mut bits: Vec<bool> = Vec::new();
    let mut offsets = Vec::new();
    for e in &encoded {
        offsets.push(bits.len() as u32);
        for &b in e {
            let len = lengths[usize::from(b)];
            let code = codes[usize::from(b)];
            for k in (0..len).rev() {
                bits.push((code >> k) & 1 == 1);
            }
        }
    }
    let total = bits.len() as u32;
    let mut stream = vec![0u8; bits.len().div_ceil(8)];
    for (i, &bit) in bits.iter().enumerate() {
        if bit {
            stream[i / 8] |= 0x80 >> (i % 8);
        }
    }
    (swap_pairs(&stream), offsets, total, encode_array(&lengths))
}

/// Swaps the bytes of every complete pair (an involution).
pub(crate) fn swap_pairs(bytes: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for pair in out.as_chunks_mut::<2>().0 {
        pair.swap(0, 1);
    }
    out
}

/// A string dictionary; handles are written in page order (ids follow the pages).
pub(crate) fn string_dictionary(pages: &[PageSpec]) -> Vec<u8> {
    let total: usize = pages.iter().map(|p| p.strings().len()).sum();
    let longest = pages
        .iter()
        .flat_map(|p| p.strings())
        .map(|s| s.encode_utf16().count())
        .max()
        .unwrap_or(0);
    let mut b = dictionary_header(2);
    b.extend_from_slice(&(total as i64).to_le_bytes());
    b.push(u8::from(
        pages.iter().any(|p| matches!(p, PageSpec::Huffman(..))),
    ));
    b.extend_from_slice(&(longest as i64).to_le_bytes());
    b.extend_from_slice(&(pages.len() as i64).to_le_bytes());
    let mut handles: Vec<(u32, u32)> = Vec::new();
    let mut start = 0u64;
    for (page_id, page) in pages.iter().enumerate() {
        let compressed = matches!(page, PageSpec::Huffman(..));
        b.extend_from_slice(&u64::from(compressed).to_le_bytes());
        b.push(0);
        b.extend_from_slice(&start.to_le_bytes());
        b.extend_from_slice(&(page.strings().len() as u64).to_le_bytes());
        b.push(u8::from(compressed));
        b.extend_from_slice(&[0xDD, 0xCC, 0xBB, 0xAA]);
        match page {
            PageSpec::Plain(strings) => {
                let mut units: Vec<u16> = Vec::new();
                for s in strings {
                    handles.push((units.len() as u32, page_id as u32));
                    units.extend(s.encode_utf16());
                    units.push(0);
                }
                let spare = 3u64;
                b.extend_from_slice(&spare.to_le_bytes());
                b.extend_from_slice(&(units.len() as u64).to_le_bytes());
                b.extend_from_slice(&((units.len() as u64 + spare) * 2).to_le_bytes());
                for u in &units {
                    b.extend_from_slice(&u.to_le_bytes());
                }
                b.extend(std::iter::repeat_n(0u8, spare as usize * 2));
            }
            PageSpec::Huffman(strings, charset) => {
                let (buffer, offsets, total_bits, encode) = huffman_page(strings, *charset);
                for o in offsets {
                    handles.push((o, page_id as u32));
                }
                b.extend_from_slice(&total_bits.to_le_bytes());
                let mode = match charset {
                    Charset::Single(_) => super::dictionary::HUFFMAN_SINGLE_CHARSET,
                    Charset::Multi => super::dictionary::HUFFMAN_MULTI_CHARSET,
                };
                b.extend_from_slice(&mode.to_le_bytes());
                b.extend_from_slice(&(buffer.len() as u64).to_le_bytes());
                if let Charset::Single(high) = charset {
                    b.push(*high);
                }
                b.extend_from_slice(&12u32.to_le_bytes());
                b.extend_from_slice(&encode);
                b.extend_from_slice(&(buffer.len() as u64).to_le_bytes());
                b.extend_from_slice(&buffer);
            }
        }
        b.extend_from_slice(&[0xCD, 0xAB, 0xCD, 0xAB]);
        start += page.strings().len() as u64;
    }
    b.extend_from_slice(&(handles.len() as u64).to_le_bytes());
    b.extend_from_slice(&8u32.to_le_bytes());
    for (offset, page) in handles {
        b.extend_from_slice(&offset.to_le_bytes());
        b.extend_from_slice(&page.to_le_bytes());
    }
    b
}

/// Deterministic xorshift generator for property-style tests.
#[derive(Debug, Clone)]
pub(crate) struct Rng(pub u64);

impl Rng {
    pub(crate) fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub(crate) fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
}
