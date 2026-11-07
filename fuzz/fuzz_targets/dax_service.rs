//! DAX language service (`pbi_dax::service`): diagnostics, signature help,
//! hover and completion, as `dax.diagnostics`, `dax.signatureHelp`,
//! `dax.hover` and `dax.complete` run them on every keystroke.
//!
//! The first byte picks the completion item limit; the rest is the text.
//! Diagnostics run for both source kinds against a small model and an
//! unknown one; the cursor services run at every offset of a short text and
//! at 64 spread offsets of a long one (plus past the end).
//!
//! Invariants: never panics; every span lies inside the text on character
//! boundaries; at most `MAX_DIAGNOSTICS` diagnostics; at most the requested
//! number of completion items, none with an empty label or insert text.
#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use pbi_dax::service::{
    MAX_DIAGNOSTICS, NameDetails, Schema, SourceKind, Span, complete, diagnostics, hover,
    signature_help,
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
