//! Decompresses the `DataModel` stream of the Microsoft sample PBIX files
//! (`reference/samples/microsoft`, MIT) and compares it with the reference
//! output of Microsoft's XPress9 library (via pbixray's `xpress9` binding,
//! run read-only).
//!
//! Fixtures are located through `$PBIVIEW_REFERENCE`, falling back to the
//! repository's `reference/` directory; a missing file prints `SKIP`.

mod common;

use std::io::Read;
use std::time::Instant;

use common::{REVENUE_OPPORTUNITIES, Sample, datamodel};
use pbi_vertipaq::xpress9::{
    DataModelDecoder, DataModelFormat, Limits, STREAM_STORAGE_SIGNATURE, decompress_datamodel,
};
use sha2::{Digest, Sha256};

/// `(sample, compressed DataModel bytes, decompressed bytes, SHA-256 of the
/// decompressed bytes)` from pbixray 0.15.5 / xpress9 0.3.9 (Microsoft
/// XPress9).
const REFERENCE: [(Sample, usize, usize, &str); 4] = [
    (
        REVENUE_OPPORTUNITIES,
        210_698,
        1_572_864,
        "192622713e64c5ed70a9be3bb24a06e207e41c13411f24fc43340c1477c31dbd",
    ),
    (
        common::CORPORATE_SPEND,
        669_460,
        1_888_256,
        "d043b75081dc448514956298e18538a7d700fd7f5496086ca6eeecefb6475d4f",
    ),
    (
        common::REGIONAL_SALES,
        673_169,
        3_411_968,
        "b6c71bdcd7d78ac0376d676d4e270bd99bc0eadb064bdb4a5ef903677999eed7",
    ),
    (
        common::ADVENTURE_WORKS,
        8_360_255,
        22_925_312,
        "ab88eb023995f11675de4f92f9d3193cb672ce86322aa70fceeb9280e3d0b13e",
    ),
];

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn fixture_datamodel_matches_reference_output() {
    for (sample, compressed, expected_len, expected_sha) in REFERENCE {
        let Some(bytes) = datamodel(sample) else {
            continue;
        };
        assert_eq!(
            bytes.len(),
            compressed,
            "unexpected {} version",
            sample.file
        );

        let start = Instant::now();
        let out = decompress_datamodel(&bytes, &Limits::default()).unwrap();
        let elapsed = start.elapsed();
        eprintln!(
            "{}: decompressed {} -> {} bytes in {:.2?} ({:.0} MB/s)",
            sample.file,
            bytes.len(),
            out.len(),
            elapsed,
            out.len() as f64 / elapsed.as_secs_f64() / 1e6
        );

        assert_eq!(out.len(), expected_len, "{}", sample.file);
        assert_eq!(sha256_hex(&out), expected_sha, "{}", sample.file);

        // The output is an ABF backup: BOM + UTF-16LE stream-storage signature (72 bytes),
        // followed by the UTF-16LE <BackupLog> header XML in the rest of the first 4 KiB page.
        let mut signature = vec![0xFF, 0xFE];
        signature.extend(
            STREAM_STORAGE_SIGNATURE
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        );
        assert!(out.starts_with(&signature));
        let header: Vec<u8> = "<BackupLog>"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(&out[72..72 + header.len()], &header[..]);
    }
}

#[test]
fn fixture_streaming_matches_in_memory() {
    let (sample, _, expected_len, expected_sha) = REFERENCE[0];
    let Some(bytes) = datamodel(sample) else {
        return;
    };
    let mut dec = DataModelDecoder::new(&bytes[..], Limits::default()).unwrap();
    assert_eq!(dec.format(), DataModelFormat::Xpress9SingleThreaded);
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = dec.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    assert_eq!(out.len(), expected_len);
    assert_eq!(sha256_hex(&out), expected_sha);
}

#[test]
fn fixture_respects_output_limit() {
    let (sample, _, expected_len, _) = REFERENCE[0];
    let Some(bytes) = datamodel(sample) else {
        return;
    };
    let limits = Limits::default().with_max_output_bytes(expected_len as u64 - 1);
    assert!(decompress_datamodel(&bytes, &limits).is_err());
    let exact = Limits::default().with_max_output_bytes(expected_len as u64);
    assert_eq!(
        decompress_datamodel(&bytes, &exact).unwrap().len(),
        expected_len
    );
}
