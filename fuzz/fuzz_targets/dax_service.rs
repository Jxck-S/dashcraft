//! DAX language service (`pbi_dax::service`): diagnostics, signature help,
//! hover, completion, semantic tokens and references, as `dax.diagnostics`,
//! `dax.signatureHelp`, `dax.hover`, `dax.complete`, `dax.tokens` and
//! `dax.references` run them on every keystroke.
//!
//! The first byte picks the completion item limit; the rest is the text.
//! Diagnostics run for both source kinds against a small model and an
//! unknown one; the cursor services run at every offset of a short text and
//! at 64 spread offsets of a long one (plus past the end).
//!
//! Invariants: never panics; every span lies inside the text on character
//! boundaries; at most `MAX_DIAGNOSTICS` diagnostics; at most the requested
//! number of completion items, none with an empty label or insert text;
//! tokens in order without overlap; the references contain the name at the
//! cursor and their definition is one of them.
#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use pbi_dax::service::{
    MAX_DIAGNOSTICS, NameDetails, Schema, SourceKind, Span, complete, diagnostics, hover,
    references, semantic_tokens, signature_help,
};

const EVERY_OFFSET_UP_TO: usize = 256;
const SPREAD_OFFSETS: usize = 64;

fn schema() -> &'static Schema {
    static SCHEMA: OnceLock<Schema> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let mut s = Schema::new();
        s.add_table("Sales");
        s.add_column_with(
            "Sales",
            "Amount",
            NameDetails {
                data_type: Some("double".into()),
                ..NameDetails::default()
            },
        );
        s.add_column("Sales", "Order Date");
        s.add_measure_with(
            "Sales",
            "Total Sales",
            NameDetails {
                expression: Some("SUM(Sales[Amount])".into()),
                ..NameDetails::default()
            },
        );
        s.add_table("Product Category");
        s.add_column("Product Category", "Category");
        s.add_measure("Product Category", "Categories");
        s.add_table("Prödüct");
        s.add_column("Prödüct", "Ü");
        s.add_function("Twice");
        s
    })
}

fn check_span(src: &str, span: Span, what: &str) {
    assert!(
        span.start <= span.end && src.get(span.start..span.end).is_some(),
        "{what} span {span:?} outside the text or not on char boundaries ({} bytes): {src:?}",
        src.len()
    );
}

fn check_cursor(src: &str, offset: usize, schema: &Schema, max_items: usize) {
    if let Some(help) = signature_help(src, offset) {
        check_span(src, help.name_span, "signature help");
        if let Some(p) = help.active_parameter {
            assert!(
                p < help.function.params.len().max(1),
                "active parameter {p} of {} in {src:?} @ {offset}",
                help.function.name
            );
        }
    }
    if let Some(h) = hover(src, offset, schema) {
        check_span(src, h.span, "hover");
        let _ = h.markdown();
    }
    let c = complete(src, offset, schema, max_items);
    check_span(src, c.span, "completion");
    assert!(
        max_items == 0 || c.items.len() <= max_items,
        "{} items over the limit {max_items} in {src:?} @ {offset}",
        c.items.len()
    );
    for item in &c.items {
        assert!(
            !item.label.is_empty() && !item.insert_text.is_empty(),
            "empty completion {item:?} in {src:?} @ {offset}"
        );
    }
    if let Some(r) = references(src, offset, schema) {
        check_span(src, r.span, "reference");
        assert!(
            r.sites.iter().any(|site| site.span == r.span),
            "the name at the cursor is not a reference in {src:?} @ {offset}"
        );
        for site in &r.sites {
            check_span(src, site.span, "reference site");
        }
        if let Some(d) = r.definition {
            assert!(
                r.sites
                    .iter()
                    .any(|site| site.declaration && site.span == d),
                "definition {d:?} is not a declaring site in {src:?} @ {offset}"
            );
        }
    }
}

fn check_tokens(src: &str, schema: &Schema) {
    let mut end = 0;
    for t in semantic_tokens(src, schema) {
        check_span(src, t.span, "token");
        assert!(
            t.span.start >= end && t.span.start < t.span.end,
            "token {t:?} overlaps or is empty in {src:?}"
        );
        end = t.span.end;
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&first, rest)) = data.split_first() else {
        return;
    };
    let Ok(src) = std::str::from_utf8(rest) else {
        return;
    };
    let max_items = usize::from(first % 64);
    let unknown = Schema::unknown();
    for s in [schema(), &unknown] {
        for kind in [SourceKind::Expression, SourceKind::Query, SourceKind::detect(src)] {
            let found = diagnostics(src, kind, s);
            assert!(found.len() <= MAX_DIAGNOSTICS, "{} diagnostics", found.len());
            for d in &found {
                check_span(src, d.span, "diagnostic");
                let _ = d.range(src);
            }
        }
        check_tokens(src, s);
    }
    let len = src.len();
    if len <= EVERY_OFFSET_UP_TO {
        for offset in 0..=len + 1 {
            check_cursor(src, offset, schema(), max_items);
        }
    } else {
        for i in 0..=SPREAD_OFFSETS {
            check_cursor(src, len * i / SPREAD_OFFSETS, schema(), max_items);
        }
        check_cursor(src, len + 1, schema(), max_items);
    }
    check_cursor(src, usize::MAX, &unknown, max_items);
});
