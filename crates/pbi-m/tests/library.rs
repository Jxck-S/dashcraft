//! Power BI Desktop's M standard library (ground-truth suite `m-library`, ADR-0011).
//!
//! `src/service/library.txt` is generated from `reference/groundtruth/recordings/m-library.json`
//! (`DASHCRAFT_BLESS=1 cargo test -p pbi-m --test library` rewrites it), and every library name
//! DashCraft implements must exist in Desktop's library with the same kind, parameter names and
//! counts, type and constant value.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use pbi_m::service::{LibraryKind, catalog, catalog_desktop_version, catalog_entry};
use pbi_m::{Engine, TypeValue, Value, library_names};
use serde_json::Value as Json;

fn reference_dir() -> PathBuf {
    std::env::var_os("DASHCRAFT_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
}

fn catalog_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/service/library.txt")
}

/// The recording, or `None` (after printing SKIP) when it is missing.
fn recording() -> Option<Json> {
    let path = reference_dir().join("groundtruth/recordings/m-library.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        println!("SKIP: {} missing", path.display());
        return None;
    };
    Some(serde_json::from_str(&text).expect("m-library.json parses"))
}

fn recorded_entries(rec: &Json) -> Vec<Json> {
    rec["cases"]
        .as_array()
        .and_then(|cases| cases.iter().find(|c| c["id"] == "library"))
        .and_then(|c| c["value"].as_array())
        .cloned()
        .expect("the recording has the library case's value")
}

/// An M number literal: integers without a fraction, `#nan`, `#infinity`.
fn number_literal(n: f64) -> String {
    if n.is_nan() {
        "#nan".to_owned()
    } else if n.is_infinite() {
        if n > 0.0 { "#infinity" } else { "-#infinity" }.to_owned()
    } else if n == 0.0 || (1e-5..1e16).contains(&n.abs()) {
        format!("{n}")
    } else {
        format!("{n:e}")
    }
}

fn text_literal(t: &str) -> String {
    let mut out = String::from("\"");
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => out.push_str("\"\""),
            '#' if chars.peek() == Some(&'(') => out.push_str("#(#)"),
            c if c.is_control() => out.push_str(&format!("#({:04X})", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn recorded_literal(kind: &str, v: &Json) -> String {
    match (kind, v) {
        ("number", Json::Object(o)) => match o.get("#number").and_then(Json::as_str) {
            Some("NaN") => "#nan".to_owned(),
            Some("Infinity") => "#infinity".to_owned(),
            Some("-Infinity") => "-#infinity".to_owned(),
            other => panic!("unexpected tagged number {other:?}"),
        },
        ("number", v) => number_literal(v.as_f64().expect("a number")),
        ("text", Json::String(s)) => text_literal(s),
        ("logical", Json::Bool(b)) => b.to_string(),
        (kind, v) => panic!("unexpected {kind} constant {v}"),
    }
}

fn render_catalog(rec: &Json) -> String {
    let version = rec["desktop"]["version"].as_str().unwrap_or("unknown");
    let mut out = String::from(
        "# Power BI Desktop's M standard library: every dotted name of #shared.\n\
         # Generated from reference/groundtruth/recordings/m-library.json by\n\
         # `DASHCRAFT_BLESS=1 cargo test -p pbi-m --test library`; do not edit by hand.\n\
         # Tab-separated: name, kind, then for functions the required parameter count, the\n\
         # parameters (name:type, comma-separated) and the return type; for types their\n\
         # primitive kind; for number, text and logical constants their value as an M literal.\n",
    );
    out.push_str(&format!("# desktop {version}\n"));
    let mut entries = recorded_entries(rec);
    entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    for e in &entries {
        let name = e["name"].as_str().expect("name");
        let kind = e["kind"].as_str().expect("kind");
        assert!(
            !name.contains(['\t', '\n']),
            "{name}: unexpected character in name"
        );
        let mut cols = vec![name.to_owned(), kind.to_owned()];
        match kind {
            "function" => {
                let params = e["params"].as_array().expect("params");
                let params: Vec<String> = params
                    .iter()
                    .map(|p| {
                        let pname = p[0].as_str().expect("parameter name");
                        assert!(
                            !pname.contains([',', ':', '\t', '\n']),
                            "{name}: parameter {pname:?} cannot be written"
                        );
                        format!("{pname}:{}", p[1].as_str().expect("parameter type"))
                    })
                    .collect();
                cols.push(e["required"].as_u64().expect("required").to_string());
                cols.push(params.join(","));
                cols.push(e["returns"].as_str().expect("returns").to_owned());
            }
            "type" => cols.push(e["type"].as_str().expect("type").to_owned()),
            _ => {
                if let Some(v) = e.get("value") {
                    cols.push(recorded_literal(kind, v));
                }
            }
        }
        out.push_str(&cols.join("\t"));
        out.push('\n');
    }
    out
}

#[test]
fn catalog_is_generated_from_the_desktop_recording() {
    let Some(rec) = recording() else { return };
    let expected = render_catalog(&rec);
    if std::env::var_os("DASHCRAFT_BLESS").is_some() {
        std::fs::write(catalog_path(), &expected).expect("write library.txt");
        println!("wrote {}", catalog_path().display());
        return;
    }
    let actual = std::fs::read_to_string(catalog_path()).expect("library.txt");
    assert!(
        actual.replace("\r\n", "\n") == expected,
        "src/service/library.txt is stale: run DASHCRAFT_BLESS=1 cargo test -p pbi-m --test library"
    );
    assert_eq!(
        catalog().len(),
        recorded_entries(&rec).len(),
        "every line of library.txt parses"
    );
    assert_eq!(
        catalog_desktop_version(),
        rec["desktop"]["version"].as_str()
    );
}

/// `type nullable text` → `nullable text`, as the recording names types.
fn type_name(t: &TypeValue) -> String {
    let base = t.base().name();
    match base {
        "any" | "null" | "none" => base.to_owned(),
        _ if t.is_nullable() => format!("nullable {base}"),
        _ => base.to_owned(),
    }
}

#[test]
fn implemented_library_matches_desktop() {
    let engine = Engine::default();
    let mut problems = Vec::new();
    for name in library_names() {
        let Some(entry) = catalog_entry(name) else {
            problems.push(format!("{name}: not in Desktop's library"));
            continue;
        };
        let value = match engine.evaluate(name) {
            Ok(v) => v,
            Err(e) => {
                problems.push(format!("{name}: does not evaluate: {e}"));
                continue;
            }
        };
        match (&entry.kind, &value) {
            (LibraryKind::Function(f), Value::Function(func)) => {
                let ours: Vec<String> = func.params().iter().map(|(n, _)| n.to_string()).collect();
                let desktop: Vec<&str> = f.params.iter().map(|p| p.name).collect();
                if func.arity() != (f.required, f.params.len()) || ours != desktop {
                    problems.push(format!(
                        "{name}: parameters {ours:?} ({} required), Desktop {desktop:?} ({} required)",
                        func.arity().0,
                        f.required
                    ));
                }
            }
            (LibraryKind::Type(kind), Value::Type(t)) => {
                if type_name(t) != *kind {
                    problems.push(format!("{name}: type {}, Desktop {kind}", type_name(t)));
                }
            }
            (LibraryKind::Constant { kind, value: lit }, v) => {
                let ours = match v {
                    Value::Number(n) if *kind == "number" => number_literal(*n),
                    Value::Text(t) if *kind == "text" => text_literal(t),
                    Value::Logical(b) if *kind == "logical" => b.to_string(),
                    other => format!("{:?}", other.kind()),
                };
                if ours != *lit {
                    problems.push(format!("{name}: {ours}, Desktop {lit}"));
                }
            }
            (LibraryKind::Value(kind), v) => {
                if !v.kind().name().eq_ignore_ascii_case(kind) {
                    problems.push(format!("{name}: {:?}, Desktop {kind}", v.kind()));
                }
            }
            (kind, v) => problems.push(format!("{name}: {:?}, Desktop {kind:?}", v.kind())),
        }
    }
    assert!(
        problems.is_empty(),
        "{} implemented names differ from Desktop's library:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

#[test]
fn catalog_lookup_and_signatures() {
    if recording().is_none() {
        return;
    }
    let upper = catalog_entry("Text.Upper").expect("Text.Upper");
    assert!(upper.implemented());
    assert_eq!(
        upper.signature().as_deref(),
        Some(
            "Text.Upper(text as nullable text, optional culture as nullable text) as nullable text"
        )
    );
    let sql = catalog_entry("Sql.Database").expect("Sql.Database");
    assert!(!sql.implemented());
    assert!(
        catalog_entry("text.upper").is_none(),
        "names are case-sensitive"
    );
    assert!(matches!(
        catalog_entry("JoinKind.Inner").map(|e| &e.kind),
        Some(LibraryKind::Constant {
            kind: "number",
            value: "0"
        })
    ));
    // Parameter names that are not identifiers are quoted.
    let quoted = catalog()
        .iter()
        .filter_map(|e| e.signature())
        .find(|s| s.contains("#\""));
    assert!(
        quoted.is_some(),
        "some connector has a quoted parameter name"
    );
}
