//! Content-based format detection over bytes (`pbi_core::detect::detect_bytes`).
//!
//! The file-name hint is picked from the input length so seeds stay plain
//! files. Invariant: never panics (ZIP input is opened with default limits).
#![no_main]

use libfuzzer_sys::fuzz_target;
use pbi_core::detect::detect_bytes;

const HINTS: &[Option<&str>] = &[
    None,
    Some("report.pbix"),
    Some("template.pbit"),
    Some("project.pbip"),
    Some("model.bim"),
    Some("table.tmdl"),
    Some("DataModel"),
    Some("cache.abf"),
    Some("model.vpax"),
    Some("report.json"),
    Some("definition.pbir"),
    Some("book.xlsx"),
    Some("Layout"),
    Some("DataModelSchema"),
    Some("query.pq"),
    Some("theme.json"),
];

fuzz_target!(|data: &[u8]| {
    let hint = HINTS[data.len() % HINTS.len()];
    let detection = detect_bytes(hint, data);
    let _ = format!("{detection:?}");
});
