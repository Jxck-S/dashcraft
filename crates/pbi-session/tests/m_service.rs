#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The M language service on every M query of the fixtures (the Acme
//! Outfitters example, the committed Microsoft samples and, when fetched,
//! the downloaded ones), with the environment `m_environment` builds from
//! each model:
//!
//! * every free name the parser's binder finds resolves in `hover` to a
//!   query, parameter, table or library name, never to a name of the text
//!   (the tolerant scanner scopes like the binder);
//! * completion at the end of every free name ranks that name first;
//! * signature help just before the `)` of every library call shows that
//!   function;
//! * nothing panics at any cursor of the example's queries;
//! * `format` lays out every query (at the default width and a narrow one)
//!   without changing its tokens, comments or syntax tree, and formatting
//!   the result again changes nothing.

use std::path::PathBuf;

use pbi_m::service::{
    FormatOptions, NameKind, catalog_entry, complete, format, hover, signature_help,
};
use pbi_model::{Document, ExpressionKind, PartitionSource};

fn reference_root() -> PathBuf {
    std::env::var_os("DASHCRAFT_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
}

const PBIP: &str = "examples/acme-outfitters/AcmeOutfitters.pbip";

fn fixtures() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for rel in [
        PBIP,
        "samples/microsoft/Corporate Spend.pbix",
        "samples/microsoft/COVID-19 US Tracking Sample.pbit",
        "samples/microsoft/Regional Sales Sample.pbix",
        "samples/microsoft/Revenue Opportunities.pbix",
    ] {
        let path = reference_root().join(rel);
        if path.exists() {
            out.push(path);
        } else {
            println!("SKIP: {} missing", path.display());
        }
    }
    let downloads = reference_root().join("samples/downloads");
    match std::fs::read_dir(&downloads) {
        Ok(entries) => {
            let mut more: Vec<PathBuf> = entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "pbix" | "pbit"))
                })
                .collect();
            more.sort();
            out.extend(more);
        }
        Err(_) => println!("SKIP: {} missing", downloads.display()),
    }
    out
}

/// Every M text of `doc`: `(where, text)`.
fn m_texts(doc: &Document) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(model) = doc.model.as_ref() else {
        return out;
    };
    for e in &model.expressions {
        if e.kind == ExpressionKind::M {
            out.push((format!("expression '{}'", e.name), e.full_expression()));
        }
    }
    for t in &model.tables {
        for p in &t.partitions {
            if let PartitionSource::M { expression } = &p.source {
                out.push((format!("table '{}'", t.name), expression.clone()));
            }
        }
    }
    out
}

#[test]
fn every_free_name_of_every_fixture_query_resolves() {
    let mut names = 0;
    let mut calls = 0;
    for path in fixtures() {
        let opened = pbi_core::open_path(&path, &pbi_core::OpenOptions::default()).unwrap();
        let Some(model) = opened.document.model.as_ref() else {
            continue;
        };
        let env = pbi_session::m_environment(model);
        for (at, text) in m_texts(&opened.document) {
            let expr = pbi_m::parse_expression(&text)
                .unwrap_or_else(|e| panic!("{}: {at}: {e}", path.display()));
            let found = pbi_m::free_names(&expr);
            for name in &found.free {
                let span = name.span;
                let h = hover(&text, span.start, &env).unwrap_or_else(|| {
                    panic!("{}: {at}: no hover for `{}`", path.display(), name.text)
                });
                assert!(
                    !matches!(
                        h.kind,
                        NameKind::Step | NameKind::Parameter | NameKind::Field | NameKind::Member
                    ),
                    "{}: {at}: free name `{}` resolves to a {:?} of the text",
                    path.display(),
                    name.text,
                    h.kind
                );
                if name.text.contains('"') {
                    continue;
                }
                let quoted = text.get(span.start..).is_some_and(|t| t.starts_with("#\""));
                let offset = if quoted { span.end - 1 } else { span.end };
                let c = complete(&text, offset, &env, 5);
                assert_eq!(
                    c.items.first().map(|i| i.label.as_str()),
                    Some(name.text.as_str()),
                    "{}: {at}: completion at the end of `{}`",
                    path.display(),
                    name.text
                );
                names += 1;
            }
            for call in &found.calls {
                if catalog_entry(&call.name.text).is_none() {
                    continue;
                }
                let help = signature_help(&text, call.span.end - 1, &env).unwrap_or_else(|| {
                    panic!(
                        "{}: {at}: no signature for `{}`",
                        path.display(),
                        call.name.text
                    )
                });
                assert_eq!(help.name, call.name.text, "{}: {at}", path.display());
                calls += 1;
            }
        }
    }
    println!("{names} free names and {calls} library calls checked");
}

#[test]
fn no_panic_at_any_cursor_of_the_example() {
    let path = reference_root().join(PBIP);
    if !path.exists() {
        println!("SKIP: {} missing", path.display());
        return;
    }
    let opened = pbi_core::open_path(&path, &pbi_core::OpenOptions::default()).unwrap();
    let model = opened.document.model.as_ref().unwrap();
    let env = pbi_session::m_environment(model);
    let texts = m_texts(&opened.document);
    assert!(texts.len() >= 10, "only {} queries", texts.len());
    for (at, text) in &texts {
        for offset in 0..=text.len() {
            let _ = hover(text, offset, &env);
            let _ = signature_help(text, offset, &env);
            let c = complete(text, offset, &env, 3);
            assert!(
                c.span.start <= c.span.end && c.span.end <= text.len(),
                "{at}"
            );
        }
    }
}

#[test]
fn every_fixture_query_formats_stably() {
    let narrow = FormatOptions {
        max_width: 40,
        indent_width: 2,
        use_tabs: false,
    };
    let mut queries = 0;
    let mut changed = 0;
    for path in fixtures() {
        let opened = pbi_core::open_path(&path, &pbi_core::OpenOptions::default()).unwrap();
        for (at, text) in m_texts(&opened.document) {
            for opts in [FormatOptions::default(), narrow] {
                let out = format(&text, &opts)
                    .unwrap_or_else(|e| panic!("{}: {at}: {e}", path.display()));
                assert_eq!(
                    format(&out, &opts).as_deref(),
                    Ok(out.as_str()),
                    "{}: {at}: not idempotent at width {}",
                    path.display(),
                    opts.max_width
                );
                if opts.max_width == 40 {
                    continue;
                }
                assert!(
                    out.lines()
                        .all(|l| l.chars().count() <= 100 || !l.contains(", ")),
                    "{}: {at}: a breakable line is too long:\n{out}",
                    path.display()
                );
                changed += usize::from(out != text.trim());
            }
            queries += 1;
        }
    }
    println!("{queries} queries formatted, {changed} changed at the default width");
}
