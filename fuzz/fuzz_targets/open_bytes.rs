//! The whole reader pipeline on arbitrary bytes (`pbi_core::open_bytes`) with
//! strict limits: detection, ZIP containers (PBIX/PBIT/...), TMSL, TMDL,
//! PBIR, legacy layouts, `DataModel` (XPress9/ABF/metadata) and redaction,
//! then the in-memory savers (PBIP files, PBIT bytes).
//!
//! The file-name hint is picked from the input length so seeds stay plain
//! files. Invariants: never panics; a PBIT written from an opened document
//! opens again (unless it exceeds the strict limits).
#![no_main]

use libfuzzer_sys::fuzz_target;
use pbi_core::container::{ContainerError, Limits};
use pbi_core::{CoreError, OpenOptions, open_bytes, pbip_files, pbit_bytes};

const HINTS: &[&str] = &[
    "",
    "report.pbix",
    "template.pbit",
    "model.bim",
    "table.tmdl",
    "DataModel",
    "cache.abf",
    "report.json",
    "Layout",
    "DataModelSchema",
    "model.vpax",
    "book.xlsx",
];

fuzz_target!(|data: &[u8]| {
    let hint = HINTS[data.len() % HINTS.len()];
    let opts = OpenOptions {
        limits: Limits::strict(),
        ..OpenOptions::default()
    };
    let Ok(doc) = open_bytes(hint, data.to_vec(), &opts) else {
        return;
    };
    let _ = doc.inspect_json();
    let _ = doc.package_version();
    let _ = pbip_files(&doc);
    if let Ok(pbit) = pbit_bytes(&doc) {
        match open_bytes("roundtrip.pbit", pbit, &opts) {
            Ok(_) | Err(CoreError::Container(ContainerError::LimitExceeded { .. })) => {}
            Err(e) => panic!("a PBIT written from an opened document does not open: {e}"),
        }
    }
});
