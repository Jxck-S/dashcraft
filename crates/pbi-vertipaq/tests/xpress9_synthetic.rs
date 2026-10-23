//! XPress9 vectors produced by Microsoft's MIT-licensed XPress9 encoder from synthetic
//! plaintext (see `tests/data/gen_xpress9_vectors.py`). The plaintext is regenerated here
//! with the same deterministic generator and compared byte for byte.

use std::io::Read;

use pbi_vertipaq::xpress9::{
    DataModelDecoder, DataModelFormat, Limits, Xpress9Decoder, Xpress9Error, decompress_block,
    decompress_datamodel, fuzz_decompress,
};

const SINGLE: &[u8] = include_bytes!("data/xpress9_single_threaded.bin");
const MULTI: &[u8] = include_bytes!("data/xpress9_multi_threaded.bin");
const SKEWED: &[u8] = include_bytes!("data/xpress9_skewed.bin");

const WORDS: [&[u8]; 10] = [
    b"alpha",
    b"beta",
    b"gamma",
    b"delta",
    b"Outfitters",
    b"Trail Packs",
    b"Shipped",
    b"VertiPaq",
    b"Acme",
    b"zz",
];

fn xorshift(x: &mut u32) -> u32 {
    *x ^= *x << 13;
    *x ^= *x >> 17;
    *x ^= *x << 5;
    *x
}

fn text(seed: u32, n: usize) -> Vec<u8> {
    let mut x = seed;
    let mut out = Vec::new();
    while out.len() < n {
        let v = xorshift(&mut x);
        out.extend_from_slice(WORDS[(v % 10) as usize]);
        out.push(if (v >> 8).is_multiple_of(5) {
            b'\n'
        } else {
            b' '
        });
    }
    out.truncate(n);
    out
}

fn single_threaded_plaintext() -> Vec<u8> {
    let c0 = text(1, 3000);
    let mut c1 = vec![b'A'; 5000];
    c1.extend(text(2, 500));
    let mut c2 = text(3, 2500);
    c2.extend_from_slice(&c0[100..1100]);
    [c0, c1, c2].concat()
}

fn multi_threaded_plaintext() -> Vec<u8> {
    let mut last = text(32, 800);
    last.extend(vec![b'B'; 600]);
    [
        text(11, 1500),
        text(21, 1200),
        text(22, 900),
        text(31, 1300),
        last,
    ]
    .concat()
}

fn skewed_plaintext() -> Vec<u8> {
    let n = 8000usize;
    let mut x = 99u32;
    let mut data: Vec<u8> = (0..n)
        .map(|_| 0x40 + ((xorshift(&mut x) >> 7) % 32) as u8)
        .collect();
    for k in 0..9u32 {
        let count = if k == 0 { 1 } else { 1usize << (k - 1) };
        for _ in 0..count {
            let at = xorshift(&mut x) as usize % n;
            data[at] = 0x80 + k as u8;
        }
    }
    data
}

#[test]
fn single_threaded_session_spans_chunks() {
    let out = decompress_datamodel(SINGLE, &Limits::default()).unwrap();
    assert_eq!(out, single_threaded_plaintext());
}

#[test]
fn multi_threaded_groups_are_independent_sessions() {
    let mut dec = DataModelDecoder::new(MULTI, Limits::default()).unwrap();
    assert_eq!(dec.format(), DataModelFormat::Xpress9MultiThreaded);
    let header = dec.multi_threaded_header().unwrap();
    assert_eq!(
        (
            header.main_chunks_per_thread,
            header.prefix_chunks_per_thread,
            header.prefix_thread_count,
            header.main_thread_count
        ),
        (2, 1, 1, 2)
    );
    let mut out = Vec::new();
    dec.read_to_end(&mut out).unwrap();
    assert_eq!(out, multi_threaded_plaintext());
}

#[test]
fn long_huffman_codewords() {
    let out = decompress_datamodel(SKEWED, &Limits::default()).unwrap();
    assert_eq!(out, skewed_plaintext());
}

#[test]
fn read_adapter_with_tiny_buffers() {
    let mut dec = DataModelDecoder::new(SINGLE, Limits::default()).unwrap();
    let mut out = Vec::new();
    let mut buf = [0u8; 7];
    loop {
        let n = dec.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    assert_eq!(out, single_threaded_plaintext());
}

/// Splits a single-threaded stream into its (uncompressed size, payload) chunks.
fn chunks(stream: &[u8]) -> Vec<(usize, &[u8])> {
    let mut at = 102;
    let mut out = Vec::new();
    while at < stream.len() {
        let word = |i: usize| {
            u32::from_le_bytes([stream[i], stream[i + 1], stream[i + 2], stream[i + 3]]) as usize
        };
        let (u, c) = (word(at), word(at + 4));
        out.push((u, &stream[at + 8..at + 8 + c]));
        at += 8 + c;
    }
    out
}

#[test]
fn low_level_block_and_session_api() {
    let parts = chunks(SINGLE);
    let plain = single_threaded_plaintext();

    let first = decompress_block(parts[0].1, parts[0].0, &Limits::default()).unwrap();
    assert_eq!(first, plain[..parts[0].0]);

    // A later chunk alone lacks its session context.
    assert!(decompress_block(parts[1].1, parts[1].0, &Limits::default()).is_err());

    let mut dec = Xpress9Decoder::default();
    let mut out = Vec::new();
    for (size, payload) in &parts {
        out.extend_from_slice(dec.decompress_chunk(payload, *size).unwrap());
    }
    assert_eq!(out, plain);
    let params = dec.params().unwrap();
    assert_eq!(
        (
            params.mtf_entries,
            params.ptr_min_match,
            params.mtf_min_match
        ),
        (4, 4, 2)
    );
}

#[test]
fn output_limit_is_enforced() {
    let limits = Limits::default().with_max_output_bytes(10_000);
    let err = decompress_datamodel(SINGLE, &limits).unwrap_err();
    assert!(
        matches!(err.innermost(), Xpress9Error::LimitExceeded { .. }),
        "{err}"
    );
}

#[test]
fn corrupted_and_truncated_streams_never_panic() {
    for vector in [SINGLE, MULTI] {
        for cut in 0..vector.len() {
            let res = decompress_datamodel(&vector[..cut], &Limits::default());
            // Cutting exactly at a chunk boundary of a single-threaded stream is a valid,
            // shorter stream; any other cut must be an error.
            if let Ok(out) = res {
                assert!(out.len() < 11_500);
            }
            fuzz_decompress(&vector[..cut]);
        }
        let mut x = 0x9E37_79B9u32;
        for _ in 0..3000 {
            let mut data = vector.to_vec();
            let flips = 1 + xorshift(&mut x) % 4;
            for _ in 0..flips {
                let at = 102 + xorshift(&mut x) as usize % (data.len() - 102);
                data[at] ^= 1 << (xorshift(&mut x) % 8);
            }
            let _ = decompress_datamodel(&data, &Limits::default());
            fuzz_decompress(&data);
        }
    }
    let mut x = 7u32;
    for len in 0..512usize {
        let data: Vec<u8> = (0..len).map(|_| xorshift(&mut x) as u8).collect();
        fuzz_decompress(&data);
        let mut framed = SINGLE[..102].to_vec();
        framed.extend_from_slice(&data);
        fuzz_decompress(&framed);
    }
}
