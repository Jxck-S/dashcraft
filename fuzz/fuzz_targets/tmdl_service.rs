//! TMDL language service (`pbi_tmdl::service`): diagnostics over a set of files, as
//! `tmdl.diagnostics` runs them, and completion and hover in the first file, as
//! `tmdl.complete` and `tmdl.hover` run them.
//!
//! Input: one or more `.tmdl` files separated by NUL bytes. The first file is named
//! `model.tmdl`, so a fuzzed model folder also exercises reference resolution.
//!
//! Invariants: never panics; at most `MAX_DIAGNOSTICS` diagnostics; each names an input file
//! (index and path), its range lies inside that file's text on character boundaries, its line
//! and column are 1-based, and a suggestion is never empty; the result is the same on a second
//! run. Completion and hover at a few cursors of the first file (the others as the folder):
//! ranges inside the text on character boundaries, the prefix typed before the cursor, at most
//! the items asked for with distinct, non-empty labels, declarations in the input files.
#![no_main]

use libfuzzer_sys::fuzz_target;
use pbi_tmdl::service::{FileText, MAX_DIAGNOSTICS, Source, complete, diagnostics, hover};

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

    let Some((first, rest)) = sources.split_first() else {
        return;
    };
    let text = first.text.as_str();
    let others: Vec<FileText> = rest
        .iter()
        .map(|s| FileText {
            path: &s.path,
            text: &s.text,
        })
        .collect();
    let len = text.len();
    for offset in [0, len / 3, len / 2, len.saturating_sub(1), len, len + 1] {
        let c = complete(text, offset, &others, 16);
        assert!(
            c.start <= c.end && text.get(c.start..c.end).is_some(),
            "completion range at {offset}: {c:?}"
        );
        assert!(
            text.get(c.start..).is_some_and(|t| t.starts_with(&c.prefix)),
            "prefix at {offset}: {c:?}"
        );
        assert!(c.items.len() <= 16, "{} items", c.items.len());
        let mut labels: Vec<&str> = c.items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.iter().all(|l| !l.is_empty()), "{c:?}");
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), c.items.len(), "duplicate labels: {c:?}");
        if let Some(h) = hover(text, offset, &others) {
            assert!(
                h.start < h.end && text.get(h.start..h.end).is_some(),
                "hover range at {offset}: {h:?}"
            );
            if let Some(d) = &h.declaration {
                assert!(
                    d.line >= 1 && d.file.is_none_or(|f| f < others.len()),
                    "{h:?}"
                );
            }
        }
    }
});
