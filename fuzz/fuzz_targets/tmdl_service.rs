//! TMDL language service (`pbi_tmdl::service`): diagnostics over a set of files, as
//! `tmdl.diagnostics` runs them.
//!
//! Input: one or more `.tmdl` files separated by NUL bytes. The first file is named
//! `model.tmdl`, so a fuzzed model folder also exercises reference resolution.
//!
//! Invariants: never panics; at most `MAX_DIAGNOSTICS` diagnostics; each names an input file
//! (index and path), its range lies inside that file's text on character boundaries, its line
//! and column are 1-based, and a suggestion is never empty; the result is the same on a second
//! run.
#![no_main]

use libfuzzer_sys::fuzz_target;
use pbi_tmdl::service::{MAX_DIAGNOSTICS, Source, diagnostics};

const MAX_FILES: usize = 8;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let sources: Vec<Source> = text
        .split('\0')
        .take(MAX_FILES)
        .enumerate()
        .map(|(i, t)| {
            let path = if i == 0 {
                "model.tmdl".to_owned()
            } else {
                format!("tables/t{i}.tmdl")
            };
            Source::new(path, t)
        })
        .collect();
    let found = diagnostics(&sources);
    assert!(found.len() <= MAX_DIAGNOSTICS, "{} diagnostics", found.len());
    for d in &found {
        let Some(source) = sources.get(d.file) else {
            panic!("diagnostic of file {} of {}: {d:?}", d.file, sources.len());
        };
        assert_eq!(d.path, source.path, "{d:?}");
        assert!(
            d.start <= d.end && source.text.get(d.start..d.end).is_some(),
            "range outside the text or not on char boundaries ({} bytes): {d:?}",
            source.text.len()
        );
        assert!(d.line >= 1 && d.column >= 1, "{d:?}");
        assert!(d.suggestion.as_deref() != Some(""), "{d:?}");
    }
    assert!(diagnostics(&sources) == found, "diagnostics are not deterministic");
});
