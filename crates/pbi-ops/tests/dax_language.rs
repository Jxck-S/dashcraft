//! `dax.diagnostics` through `Registry::execute`: positions and
//! suggestions on the Acme Outfitters example, checks without a document,
//! edits seen at once, and no false error on any DAX expression of the
//! example and the Microsoft samples (M7 acceptance 2).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::PathBuf;

use common::{PBIP, Runner, fixture, reference_root, s};
use pbi_model::{Document, PartitionSource, PropValue};
use serde_json::{Value, json};

/// The DAX a measure keeps in untyped properties: the key and, for a KPI,
/// the member.
const MEASURE_DAX: [(&str, Option<&str>); 5] = [
    ("formatStringDefinition", None),
    ("detailRowsDefinition", None),
    ("kpi", Some("targetExpression")),
    ("kpi", Some("statusExpression")),
    ("kpi", Some("trendExpression")),
];

/// The DAX text of an untyped property: an expression, an object with an
/// `expression` (or the `member`), or TMSL's list of lines.
fn prop_text(value: &PropValue, member: Option<&str>) -> Option<String> {
    match (value, member) {
        (PropValue::Expression(s) | PropValue::Text(s), None) => Some(s.clone()),
        (PropValue::Object(o), None) => o.expression.clone(),
        (PropValue::Object(o), Some(member)) => prop_text(o.properties.get(member)?, None),
        (PropValue::Json(v), member) => json_text(v, member),
        (PropValue::List(items), member) => prop_text(items.first()?, member),
        _ => None,
    }
}

fn json_text(value: &Value, member: Option<&str>) -> Option<String> {
    match (value, member) {
        (Value::Object(map), Some(member)) => json_text(map.get(member)?, None),
        (Value::Object(map), None) => json_text(map.get("expression")?, None),
        (Value::String(s), None) => Some(s.clone()),
        (Value::Array(lines), None) => lines
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>()
            .map(|lines| lines.join("\n")),
        _ => None,
    }
}

/// Every DAX expression of `doc`: `(where, text)`.
fn dax_texts(doc: &Document) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(model) = doc.model.as_ref() else {
        return out;
    };
    for t in &model.tables {
        for m in &t.measures {
            let at = format!("measure '{}'[{}]", t.name, m.name);
            out.push((at.clone(), m.expression.clone()));
            for (key, member) in MEASURE_DAX {
                if let Some(text) = m.extra.get(key).and_then(|v| prop_text(v, member)) {
                    out.push((format!("{at} {key}"), text));
                }
            }
        }
        if let Some(text) = t
            .extra
            .get("defaultDetailRowsDefinition")
            .and_then(|v| prop_text(v, None))
        {
            out.push((format!("table '{}' detail rows", t.name), text));
        }
        for c in &t.columns {
            if let Some(e) = c.expression() {
                out.push((format!("column '{}'[{}]", t.name, c.name), e.to_owned()));
            }
        }
        for p in &t.partitions {
            if let PartitionSource::Calculated { expression } = &p.source {
                out.push((format!("calculated table '{}'", t.name), expression.clone()));
            }
        }
        if let Some(group) = &t.calculation_group {
            for item in &group.calculation_items {
                let at = format!("calculation item '{}'[{}]", t.name, item.name);
                out.push((at.clone(), item.expression.clone()));
                if let Some(f) = &item.format_string_definition {
                    out.push((format!("{at} format string"), f.clone()));
                }
            }
            for selection in [
                &group.multiple_or_empty_selection_expression,
                &group.no_selection_expression,
            ]
            .into_iter()
            .flatten()
            {
                out.push((
                    format!("calculation group '{}' selection", t.name),
                    selection.expression.clone(),
                ));
            }
        }
    }
    for role in &model.roles {
        for p in &role.table_permissions {
            if let Some(f) = &p.filter_expression {
                out.push((format!("role '{}' on '{}'", role.name, p.table), f.clone()));
            }
        }
    }
    for r in &doc.reports {
        for m in &r.measures {
            out.push((
                format!("report measure '{}'[{}]", m.table, m.name),
                m.expression.clone(),
            ));
        }
    }
    out.retain(|(_, text)| !text.trim().is_empty());
    out
}

/// The example, the committed Microsoft samples and, when fetched, the
/// larger ones under `samples/downloads/`.
fn fixtures() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = [
        PBIP,
        "samples/microsoft/Corporate Spend.pbix",
        "samples/microsoft/COVID-19 US Tracking Sample.pbit",
        "samples/microsoft/Regional Sales Sample.pbix",
        "samples/microsoft/Revenue Opportunities.pbix",
    ]
    .into_iter()
    .filter_map(fixture)
    .collect();
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

#[tokio::test(flavor = "multi_thread")]
async fn no_false_errors_on_any_fixture_expression() {
    let r = Runner::new();
    let mut checked = 0;
    for path in fixtures() {
        let doc = r.open(json!({"path": s(&path)})).await;
        let document = r.ws.get(&doc).unwrap().session().document();
        let texts = dax_texts(&document);
        let mut problems = Vec::new();
        let mut warnings = Vec::new();
        for (at, text) in &texts {
            let out = r
                .run(
                    "dax.diagnostics",
                    json!({"docId": doc, "text": text, "kind": "expression"}),
                )
                .await;
            for d in out["diagnostics"].as_array().unwrap() {
                if d["severity"] == "warning" {
                    // A function DashCraft cannot evaluate yet: true, not a
                    // false error.
                    assert_eq!(d["code"], "unsupportedFunction", "{at}: {d}");
                    warnings.push(d["message"].as_str().unwrap().to_owned());
                } else {
                    problems.push(format!("{at}: {text}\n  {d}"));
                }
            }
        }
        assert!(
            problems.is_empty(),
            "{}: false errors in {} expressions:\n{}",
            path.display(),
            texts.len(),
            problems.join("\n")
        );
        warnings.sort();
        warnings.dedup();
        // Every function the committed fixtures use is supported (the
        // optional downloads may still use one that is not).
        assert!(
            warnings.is_empty() || path.starts_with(reference_root().join("samples/downloads")),
            "{}: {warnings:?}",
            path.display()
        );
        println!(
            "{}: {} expressions, no errors; warnings: {warnings:?}",
            path.display(),
            texts.len()
        );
        checked += texts.len();
        r.run("doc.close", json!({"docId": doc})).await;
    }
    if checked > 0 {
        // The example alone has measures, calculated columns and tables,
        // calculation items and RLS filters.
        assert!(checked >= 40, "only {checked} expressions found");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn diagnostics_point_at_unknown_names_with_suggestions() {
    let Some(path) = fixture(PBIP) else {
        return;
    };
    let r = Runner::new();
    let doc = r.open(json!({"path": s(&path)})).await;
    let text = "CALCULATE([Total Sales], Stores[Regoin] = \"West\") + [Total Sale]";
    let out = r
        .run("dax.diagnostics", json!({"docId": doc, "text": text}))
        .await;
    assert_eq!(out["kind"], "expression");
    assert_eq!(out["ok"], false);
    assert!(out["documentRevision"].is_u64());
    let d = out["diagnostics"].as_array().unwrap();
    assert_eq!(d.len(), 2, "{out}");
    assert_eq!(
        d[0],
        json!({
            "severity": "error",
            "code": "unknownColumn",
            "message": "unknown column `Stores[Regoin]`",
            "start": {"offset": 31, "utf16Offset": 31, "line": 1, "column": 32},
            "end": {"offset": 39, "utf16Offset": 39, "line": 1, "column": 40},
            "suggestion": "did you mean `Stores[Region]`?"
        })
    );
    assert_eq!(d[1]["code"], "unknownMeasure");
    assert_eq!(d[1]["suggestion"], "did you mean `[Total Sales]`?");

    // Offsets for JavaScript count UTF-16 code units.
    let out = r
        .run(
            "dax.diagnostics",
            json!({"docId": doc, "text": "\"😀\" & [Nope]"}),
        )
        .await;
    assert_eq!(
        out["diagnostics"][0]["start"],
        json!({"offset": 9, "utf16Offset": 7, "line": 1, "column": 7})
    );

    // An edit is seen at once: the new measure is known.
    let before = out["documentRevision"].as_u64().unwrap();
    r.run(
        "model.measure.create",
        json!({"docId": doc, "table": "Orders", "name": "Nope", "expression": "1"}),
    )
    .await;
    let out = r
        .run(
            "dax.diagnostics",
            json!({"docId": doc, "text": "\"😀\" & [Nope]"}),
        )
        .await;
    assert_eq!(out["diagnostics"], json!([]));
    assert_eq!(out["ok"], true);
    assert!(out["documentRevision"].as_u64().unwrap() > before);
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_document_only_syntax_and_functions_are_checked() {
    let r = Runner::new();
    let out = r
        .run("dax.diagnostics", json!({"text": "SUM(Nope[x]) + SUMM(1)"}))
        .await;
    assert_eq!(out["ok"], true, "{out}");
    assert!(out.get("documentRevision").is_none());
    let d = out["diagnostics"].as_array().unwrap();
    assert_eq!(d.len(), 1, "{out}");
    assert_eq!(
        (&d[0]["severity"], &d[0]["code"], &d[0]["suggestion"]),
        (
            &json!("warning"),
            &json!("unsupportedFunction"),
            &json!("did you mean `SUM`?")
        )
    );

    let out = r
        .run(
            "dax.diagnostics",
            json!({"text": "EVALUATE ROW(\"n\", 1 +)"}),
        )
        .await;
    assert_eq!((&out["kind"], &out["ok"]), (&json!("query"), &json!(false)));
    assert_eq!(out["diagnostics"][0]["code"], "syntax");

    // `kind` overrides the detection.
    let out = r
        .run(
            "dax.diagnostics",
            json!({"text": "EVALUATE {1}", "kind": "expression"}),
        )
        .await;
    assert_eq!(out["diagnostics"][0]["code"], "syntax");
    let out = r.run("dax.diagnostics", json!({"text": "   "})).await;
    assert_eq!(
        (&out["ok"], &out["diagnostics"]),
        (&json!(true), &json!([]))
    );

    let err = r
        .try_run("dax.diagnostics", json!({"docId": "d404", "text": "1"}))
        .await
        .unwrap_err();
    assert_eq!(err.code, pbi_ops::OpErrorCode::NoDocument, "{err:?}");
}
