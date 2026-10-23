#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]
//! Fixture helpers: the `DataModel` stream of the MIT-licensed Microsoft
//! sample PBIX files under `reference/samples/microsoft` and their facts in
//! `expected.json` (see that folder's README).

use std::path::PathBuf;

use serde_json::Value as Json;

/// Fixture root (`$PBIVIEW_REFERENCE` or `<workspace>/reference`).
pub fn reference_root() -> PathBuf {
    std::env::var_os("PBIVIEW_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
}

/// A sample: `expected.json` id, folder under the fixture root, file name.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub id: &'static str,
    pub dir: &'static str,
    pub file: &'static str,
}

pub const REVENUE_OPPORTUNITIES: Sample = Sample {
    id: "revenue-opportunities",
    dir: "samples/microsoft",
    file: "Revenue Opportunities.pbix",
};
pub const CORPORATE_SPEND: Sample = Sample {
    id: "corporate-spend",
    dir: "samples/microsoft",
    file: "Corporate Spend.pbix",
};
pub const REGIONAL_SALES: Sample = Sample {
    id: "regional-sales",
    dir: "samples/microsoft",
    file: "Regional Sales Sample.pbix",
};
/// Larger sample fetched by `scripts/fetch-samples.sh` (gitignored; tests
/// SKIP without it). Its model has calculation groups.
pub const ADVENTURE_WORKS: Sample = Sample {
    id: "adventureworks-sales",
    dir: "samples/downloads",
    file: "AdventureWorks Sales.pbix",
};
/// The committed sample PBIX files (all three carry a VertiPaq model).
pub const SAMPLES: [Sample; 3] = [REVENUE_OPPORTUNITIES, CORPORATE_SPEND, REGIONAL_SALES];
/// The committed samples plus the optional downloaded one.
pub const ALL_SAMPLES: [Sample; 4] = [
    REVENUE_OPPORTUNITIES,
    CORPORATE_SPEND,
    REGIONAL_SALES,
    ADVENTURE_WORKS,
];

/// The raw (compressed) `DataModel` entry of a sample PBIX, or `None`
/// (after printing `SKIP`) when the file is missing.
pub fn datamodel(sample: Sample) -> Option<Vec<u8>> {
    let path = reference_root().join(sample.dir).join(sample.file);
    match std::fs::read(&path) {
        Ok(zip) => Some(
            stored_zip_entry(&zip, "DataModel")
                .unwrap_or_else(|| panic!("{}: no stored DataModel entry", path.display())),
        ),
        Err(err) => {
            println!("SKIP: {} missing ({err})", path.display());
            None
        }
    }
}

/// The sample's entry in `samples/microsoft/expected.json`.
pub fn expected(sample: Sample) -> Json {
    let path = reference_root().join("samples/microsoft/expected.json");
    let all: Json = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    all["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"] == sample.id)
        .unwrap_or_else(|| panic!("{} not in {}", sample.id, path.display()))
        .clone()
}

fn u16_at(b: &[u8], at: usize) -> usize {
    usize::from(u16::from_le_bytes([b[at], b[at + 1]]))
}

fn u32_at(b: &[u8], at: usize) -> usize {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as usize
}

/// A stored (uncompressed) ZIP entry, located through the central
/// directory. Power BI stores the `DataModel` entry: it is already
/// XPress9-compressed.
pub fn stored_zip_entry(zip: &[u8], name: &str) -> Option<Vec<u8>> {
    let eocd = (0..zip.len().checked_sub(22)? + 1)
        .rev()
        .find(|&i| zip[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])?;
    let entries = u16_at(zip, eocd + 10);
    let mut at = u32_at(zip, eocd + 16);
    for _ in 0..entries {
        assert_eq!(
            zip[at..at + 4],
            [0x50, 0x4b, 0x01, 0x02],
            "central directory"
        );
        let method = u16_at(zip, at + 10);
        let size = u32_at(zip, at + 20);
        let name_len = u16_at(zip, at + 28);
        let extra_len = u16_at(zip, at + 30);
        let comment_len = u16_at(zip, at + 32);
        let local = u32_at(zip, at + 42);
        if &zip[at + 46..at + 46 + name_len] == name.as_bytes() {
            assert_eq!(method, 0, "{name} is not stored");
            assert_ne!(size, 0xFFFF_FFFF, "ZIP64 entries are not supported here");
            let start = local + 30 + u16_at(zip, local + 26) + u16_at(zip, local + 28);
            return Some(zip[start..start + size].to_vec());
        }
        at += 46 + name_len + extra_len + comment_len;
    }
    None
}

/// Table row counts and column fingerprints computed by pbixray (run
/// read-only through `tools/reference_hashes.py`; the outputs are in
/// `tests/data/reference_hashes/<id>.txt`).
pub struct Fingerprints {
    /// `(table, rows)`.
    pub tables: Vec<(String, usize)>,
    /// `(table, column, TOM data type, nulls, distinct, sha256)`.
    pub columns: Vec<(String, String, i64, usize, usize, String)>,
}

pub fn fingerprints(sample: Sample) -> Fingerprints {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/reference_hashes")
        .join(format!("{}.txt", sample.id));
    let text = std::fs::read_to_string(&path).unwrap();
    let mut out = Fingerprints {
        tables: Vec::new(),
        columns: Vec::new(),
    };
    let field = |part: &str, key: &str| -> String {
        part.strip_prefix(key)
            .and_then(|v| v.strip_prefix('='))
            .unwrap_or_else(|| panic!("{key} in {part:?}"))
            .to_owned()
    };
    for line in text.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        match parts.as_slice() {
            ["table", name, rows] => out
                .tables
                .push(((*name).to_owned(), field(rows, "rows").parse().unwrap())),
            ["column", table, column, ty, nulls, distinct, sha] => out.columns.push((
                (*table).to_owned(),
                (*column).to_owned(),
                field(ty, "type").parse().unwrap(),
                field(nulls, "nulls").parse().unwrap(),
                field(distinct, "distinct").parse().unwrap(),
                field(sha, "sha256"),
            )),
            _ => panic!("{}: unexpected line {line:?}", path.display()),
        }
    }
    out
}
