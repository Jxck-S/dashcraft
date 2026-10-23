//! PBIX `Report/Layout` / PBIR-Legacy `report.json` (`pbi_pbir::read_legacy_layout`).
//!
//! Invariants: never panics (including the AST conversion); the typed outer
//! document re-serializes to JSON identical to the input.
#![no_main]

use libfuzzer_sys::fuzz_target;
use pbi_pbir::json::first_difference;
use pbi_pbir::legacy::decode_text;
use pbi_pbir::{legacy_to_report_ast, read_legacy_layout};
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
    let Ok(legacy) = read_legacy_layout(data) else {
        return;
    };
    let _ = legacy_to_report_ast(&legacy);

    let Ok((text, _)) = decode_text(data) else {
        panic!("read_legacy_layout accepted bytes that do not decode");
    };
    let Ok(original) = serde_json::from_str::<Value>(&text) else {
        panic!("read_legacy_layout accepted text that is not JSON");
    };
    let back = match serde_json::to_value(&legacy.layout) {
        Ok(v) => v,
        Err(e) => panic!("layout does not serialize: {e}"),
    };
    if let Some(at) = first_difference(&back, &original) {
        panic!("layout round-trip differs at {at}\noriginal: {original}\nwritten:  {back}");
    }
});
