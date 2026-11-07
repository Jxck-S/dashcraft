//! M language service (`pbi_m::service`): diagnostics, hover, signature help
//! and completion, as `m.diagnostics`, `m.hover`, `m.signatureHelp` and
//! `m.complete` run them.
//!
//! The first byte picks the completion item limit; the rest is the text.
//! Diagnostics run against a small model and an unknown one; the cursor
//! services run at every offset of a short text and at 64 spread offsets of a
//! long one (plus past the end).
//!
//! Invariants: never panics; every span lies inside the text on character
//! boundaries; at most `MAX_DIAGNOSTICS` diagnostics; parameter labels lie
//! inside the signature label on character boundaries and the active
//! parameter is one of them; at most the requested number of completion
//! items, none with an empty label or insert text, and the completion span
//! contains the cursor.
#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use pbi_m::Span;
use pbi_m::service::{
    Environment, EnvironmentEntry, EnvironmentKind, MAX_DIAGNOSTICS, complete, diagnostics, hover,
    signature_help,
};

const EVERY_OFFSET_UP_TO: usize = 256;
const SPREAD_OFFSETS: usize = 64;

fn model() -> &'static Environment {
    static MODEL: OnceLock<Environment> = OnceLock::new();
    MODEL.get_or_init(|| {
        Environment::with_entries([
            EnvironmentEntry::new("Orders", EnvironmentKind::Table, "let Source = 1 in Source"),
            EnvironmentEntry::new(
                "Url",
                EnvironmentKind::Parameter,
                "\"https://example.com\" meta [IsParameterQuery = true, Type = \"Text\"]",
            )
            .with_type_name(Some("Text".to_owned())),
            EnvironmentEntry::new(
                "Clean",
                EnvironmentKind::Query,
                "(t as table, optional n as nullable number) as table => t",
            ),
            EnvironmentEntry::new("Prödüct Sales", EnvironmentKind::Query, "each _ + 1")
                .with_description(Some("A query with a quoted name.".to_owned())),
        ])
    })
}

fn check_span(src: &str, span: Span, what: &str) {
    assert!(
        span.start <= span.end && src.get(span.start..span.end).is_some(),
        "{what} span {span:?} outside the text or not on char boundaries ({} bytes): {src:?}",
        src.len()
    );
}

fn check_cursor(src: &str, offset: usize, env: &Environment, max_items: usize) {
    if let Some(help) = signature_help(src, offset, env) {
        check_span(src, help.name_span, "signature help");
        for p in &help.parameters {
            check_span(&help.label, p.label, "parameter label");
        }
        if let Some(p) = help.active_parameter {
            assert!(
                p < help.parameters.len(),
                "active parameter {p} of {} in {src:?} @ {offset}",
                help.label
            );
        }
    }
    if let Some(h) = hover(src, offset, env) {
        check_span(src, h.span, "hover");
        let _ = h.markdown();
    }
    let c = complete(src, offset, env, max_items);
    check_span(src, c.span, "completion");
    let cursor = offset.min(src.len());
    assert!(
        c.items.is_empty() || (c.span.start <= cursor && cursor <= c.span.end),
        "completion span {:?} misses the cursor in {src:?} @ {offset}",
        c.span
    );
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
    let unknown = Environment::unknown();
    for env in [model(), &unknown] {
        let found = diagnostics(src, env);
        assert!(found.len() <= MAX_DIAGNOSTICS, "{} diagnostics", found.len());
        for d in &found {
            check_span(src, d.span, "diagnostic");
        }
    }
    let len = src.len();
    if len <= EVERY_OFFSET_UP_TO {
        for offset in 0..=len + 1 {
            check_cursor(src, offset, model(), max_items);
        }
    } else {
        for i in 0..=SPREAD_OFFSETS {
            check_cursor(src, len * i / SPREAD_OFFSETS, model(), max_items);
        }
        check_cursor(src, len + 1, model(), max_items);
    }
    check_cursor(src, usize::MAX, &unknown, max_items);
});
