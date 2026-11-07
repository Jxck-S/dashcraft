//! The DAX language service through `Registry::execute` (`dax.diagnostics`,
//! `dax.signatureHelp`, `dax.hover`, `dax.complete`, `dax.tokens`,
//! `dax.references`, `dax.format`): positions and suggestions on the Acme
//! Outfitters example, checks without a document, edits seen at once,
//! cursor positions in every form, no false error on any DAX expression of
//! the example and the Microsoft samples (M7 acceptance 2), and formatting
//! that is idempotent and keeps them free of errors (M7 acceptance 1).

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
            // Highlighting agrees: no name it marks unknown.
            let out = r
                .run("dax.tokens", json!({"docId": doc, "text": text}))
                .await;
            for t in out["tokens"].as_array().unwrap() {
                if t["unresolved"] == true {
                    problems.push(format!("{at}: {text}\n  unresolved token {t}"));
                }
            }
            // Formatting is idempotent and the result has no errors either.
            if let Some(problem) = format_problem(&r, &doc, text, "expression").await {
                problems.push(format!("{at}: {text}\n  {problem}"));
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

/// Formats `text` narrow (and again with the defaults): `None` when every
/// layout is idempotent and has no error diagnostic against the document.
async fn format_problem(r: &Runner, doc: &str, text: &str, kind: &str) -> Option<String> {
    for options in [
        json!({}),
        json!({"maxWidth": 40, "keywordCase": "lower", "functionCase": "keep", "useTabs": true}),
    ] {
        let mut params = options.clone();
        params["text"] = json!(text);
        params["kind"] = json!(kind);
        let out = r.run("dax.format", params.clone()).await;
        if out["ok"] != true {
            return Some(format!("does not format: {out}"));
        }
        let formatted = out["text"].as_str().unwrap().to_owned();
        params["text"] = json!(formatted);
        let again = r.run("dax.format", params).await;
        if again["changed"] != false {
            return Some(format!(
                "not idempotent ({options}):\n{formatted}\n---\n{}",
                again["text"]
            ));
        }
        let check = r
            .run(
                "dax.diagnostics",
                json!({"docId": doc, "text": formatted, "kind": kind}),
            )
            .await;
        if let Some(d) = check["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["severity"] == "error")
        {
            return Some(format!("formatted ({options}):\n{formatted}\n  {d}"));
        }
    }
    None
}

#[tokio::test(flavor = "multi_thread")]
async fn format_lays_out_dax_and_reports_syntax_errors() {
    let r = Runner::new();
    let out = r
        .run(
            "dax.format",
            json!({"text": "calculate(sum(Sales[Amount]),Stores[Region]=\"West\") -- west"}),
        )
        .await;
    assert_eq!(
        out,
        json!({
            "kind": "expression",
            "ok": true,
            "text": "CALCULATE(SUM(Sales[Amount]), Stores[Region] = \"West\") -- west",
            "changed": true
        })
    );
    let out = r
        .run(
            "dax.format",
            json!({"text": "CALCULATE(SUM(Sales[Amount]), Stores[Region] = \"West\")", "maxWidth": 30}),
        )
        .await;
    assert_eq!(
        out["text"],
        "CALCULATE(\n    SUM(Sales[Amount]),\n    Stores[Region] = \"West\"\n)"
    );
    let out = r
        .run(
            "dax.format",
            json!({
                "text": "evaluate filter(Sales,Sales[Amount]>1) order by Sales[Amount] desc",
                "keywordCase": "lower",
                "functionCase": "keep",
                "indentWidth": 2,
                "useTabs": false
            }),
        )
        .await;
    assert_eq!(
        (&out["kind"], &out["text"]),
        (
            &json!("query"),
            &json!("evaluate\n  filter(Sales, Sales[Amount] > 1)\norder by Sales[Amount] desc")
        )
    );
    let out = r
        .run(
            "dax.format",
            json!({"text": "VAR a = 1\nRETURN\n\ta", "useTabs": true}),
        )
        .await;
    assert_eq!((&out["ok"], &out["changed"]), (&json!(true), &json!(false)));
    // `kind` overrides the detection.
    let out = r
        .run(
            "dax.format",
            json!({"text": "EVALUATE {1}", "kind": "expression"}),
        )
        .await;
    assert_eq!(out["kind"], "expression");

    // A syntax error leaves the text as it is and says where.
    let text = "SUM(\"😀\" +)";
    let out = r.run("dax.format", json!({"text": text})).await;
    assert_eq!(
        (&out["ok"], &out["changed"], &out["text"]),
        (&json!(false), &json!(false), &json!(text))
    );
    let d = &out["diagnostics"];
    assert!(d.is_null(), "{out}");
    let d = &out["diagnostic"];
    assert_eq!(
        (&d["severity"], &d["code"]),
        (&json!("error"), &json!("syntax"))
    );
    // At the `)`: the emoji is 4 UTF-8 bytes, 2 UTF-16 code units.
    assert_eq!(d["start"]["offset"], 12, "{d}");
    assert_eq!(d["start"]["utf16Offset"], 10, "{d}");

    for (params, needle) in [
        (json!({"text": "1", "maxWidth": 10_001}), "maxWidth"),
        (json!({"text": "1", "indentWidth": 17}), "indentWidth"),
        (json!({"text": "1", "keywordCase": "title"}), "title"),
        (json!({"text": "1", "docId": "d1"}), "docId"),
    ] {
        let err = r.try_run("dax.format", params).await.unwrap_err();
        assert_eq!(err.code, pbi_ops::OpErrorCode::InvalidParams, "{err:?}");
        assert!(err.message.contains(needle), "{err:?}");
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

#[tokio::test(flavor = "multi_thread")]
async fn signature_help_names_the_parameter_under_the_cursor() {
    let r = Runner::new();
    let text = "IF([Total Sales] > 0, ";
    let out = r
        .run(
            "dax.signatureHelp",
            json!({"text": text, "position": {"offset": text.len()}}),
        )
        .await;
    let sig = &out["signature"];
    assert_eq!(sig["function"], "IF", "{out}");
    assert_eq!(
        sig["signature"],
        "IF(logicalTest, valueIfTrue, [valueIfFalse])"
    );
    assert_eq!(
        sig["label"],
        "IF(logicalTest, valueIfTrue, [valueIfFalse]) → any"
    );
    assert_eq!((&sig["minArgs"], &sig["maxArgs"]), (&json!(2), &json!(3)));
    assert_eq!(
        (&sig["argument"], &sig["activeParameter"]),
        (&json!(1), &json!(1))
    );
    assert_eq!(
        (&sig["category"], &sig["supported"]),
        (&json!("Logical and information"), &json!(true))
    );
    let params = sig["parameters"].as_array().unwrap();
    assert_eq!(params.len(), 3);
    let signature = sig["signature"].as_str().unwrap();
    for p in params {
        let range =
            p["labelStart"].as_u64().unwrap() as usize..p["labelEnd"].as_u64().unwrap() as usize;
        assert_eq!(signature.get(range), p["name"].as_str());
    }
    assert_eq!(params[2]["optional"], true);
    assert_eq!(
        (&sig["nameStart"], &sig["nameEnd"]),
        (
            &json!({"offset": 0, "utf16Offset": 0, "line": 1, "column": 1}),
            &json!({"offset": 2, "utf16Offset": 2, "line": 1, "column": 3})
        )
    );

    // The same cursor as a UTF-16 offset and as a line and column, past an
    // emoji (2 UTF-16 units, 4 bytes) and a line break.
    let text = "\"😀\" &\n  ROUND(1, ";
    let end = text.encode_utf16().count();
    for position in [
        json!({"offset": text.len()}),
        json!({"utf16Offset": end}),
        json!({"line": 2, "column": 12}),
        json!({"line": 2, "column": 99}),
    ] {
        let out = r
            .run(
                "dax.signatureHelp",
                json!({"text": text, "position": position}),
            )
            .await;
        assert_eq!(out["signature"]["function"], "ROUND", "{position}: {out}");
        assert_eq!(out["signature"]["activeParameter"], 1, "{position}");
        assert_eq!(
            out["signature"]["nameStart"],
            json!({"offset": 11, "utf16Offset": 9, "line": 2, "column": 3})
        );
    }

    // The innermost call wins; a repeating group cycles; nothing outside a
    // call or after it closes.
    let cases = [
        ("SUMX(Orders, ROUND(", Some(("ROUND", 0))),
        ("SUMX(Orders, ROUND(1, 2) + ", Some(("SUMX", 1))),
        ("CALCULATE([Total Sales], A, B, ", Some(("CALCULATE", 1))),
        ("SUM(Orders[Line Amount]) ", None),
        ("1 + ", None),
        ("MyFunction(1, ", None),
    ];
    for (text, expected) in cases {
        let out = r
            .run(
                "dax.signatureHelp",
                json!({"text": text, "position": {"offset": text.len()}}),
            )
            .await;
        match expected {
            Some((function, parameter)) => {
                assert_eq!(out["signature"]["function"], function, "{text}: {out}");
                assert_eq!(
                    out["signature"]["activeParameter"], parameter,
                    "{text}: {out}"
                );
            }
            None => assert!(out.get("signature").is_none(), "{text}: {out}"),
        }
    }
    let out = r
        .run(
            "dax.signatureHelp",
            json!({"text": "SUMX(Orders, 1, 2, ", "position": {"offset": 19}}),
        )
        .await;
    assert_eq!(out["signature"]["argument"], 3);
    assert!(out["signature"].get("activeParameter").is_none(), "{out}");
}

#[tokio::test(flavor = "multi_thread")]
async fn positions_out_of_range_or_ambiguous_are_invalid_params() {
    let r = Runner::new();
    for position in [
        json!({"offset": 4}),
        json!({"utf16Offset": 4}),
        json!({"line": 2, "column": 1}),
        json!({"line": 0, "column": 1}),
        json!({"line": 1}),
        json!({"offset": 0, "utf16Offset": 0}),
        json!({}),
    ] {
        for id in ["dax.signatureHelp", "dax.hover", "dax.complete"] {
            let err = r
                .try_run(id, json!({"text": "SUM", "position": position}))
                .await
                .unwrap_err();
            assert_eq!(
                err.code,
                pbi_ops::OpErrorCode::InvalidParams,
                "{id} {position}: {err:?}"
            );
        }
    }
    // An offset inside a character rounds down to its start.
    let out = r
        .run(
            "dax.hover",
            json!({"text": "\"é\" & SUM(1)", "position": {"offset": 2}}),
        )
        .await;
    assert!(out.get("hover").is_none(), "{out}");
}

#[tokio::test(flavor = "multi_thread")]
async fn hover_describes_model_names_and_functions() {
    let Some(path) = fixture(PBIP) else {
        return;
    };
    let r = Runner::new();
    let doc = r.open(json!({"path": s(&path)})).await;
    let text =
        "VAR region = Stores[Region]\nRETURN CALCULATE([Total Sales], Stores[Region] = region)";
    let hover = |needle: &str, nth: usize| {
        let offset = text.match_indices(needle).nth(nth).unwrap().0 + 1;
        json!({"docId": doc, "text": text, "position": {"offset": offset}})
    };

    let out = r.run("dax.hover", hover("[Total Sales]", 0)).await;
    assert!(out["documentRevision"].is_u64(), "{out}");
    let h = &out["hover"];
    assert_eq!(
        (&h["kind"], &h["name"], &h["table"]),
        (&json!("measure"), &json!("[Total Sales]"), &json!("Orders"))
    );
    assert_eq!(h["expression"], "SUM(Orders[Line Amount])");
    assert!(
        h["formatString"].as_str().unwrap().contains("#,0.00"),
        "{h}"
    );
    let markdown = h["markdown"].as_str().unwrap();
    assert!(
        markdown.starts_with("`[Total Sales]`: measure in `Orders`"),
        "{markdown}"
    );
    assert!(
        markdown.contains("```dax\nSUM(Orders[Line Amount])\n```"),
        "{markdown}"
    );
    let start = text.find("[Total Sales]").unwrap();
    assert_eq!(h["start"]["offset"], start);
    assert_eq!(h["end"]["offset"], start + "[Total Sales]".len());

    let out = r.run("dax.hover", hover("[Region]", 1)).await;
    let h = &out["hover"];
    assert_eq!(
        (&h["kind"], &h["name"], &h["dataType"]),
        (&json!("column"), &json!("Stores[Region]"), &json!("string"))
    );

    let out = r.run("dax.hover", hover("Stores", 0)).await;
    let h = &out["hover"];
    assert_eq!(
        (&h["kind"], &h["name"]),
        (&json!("table"), &json!("Stores"))
    );
    assert!(h["columnCount"].as_u64().unwrap() > 1, "{h}");
    assert!(h["measureCount"].is_u64(), "{h}");

    let out = r.run("dax.hover", hover("region)", 0)).await;
    let h = &out["hover"];
    assert_eq!(
        (&h["kind"], &h["name"], &h["expression"]),
        (
            &json!("variable"),
            &json!("region"),
            &json!("Stores[Region]")
        )
    );

    let out = r.run("dax.hover", hover("CALCULATE", 0)).await;
    let h = &out["hover"];
    assert_eq!(
        (&h["kind"], &h["signature"], &h["returns"], &h["supported"]),
        (
            &json!("function"),
            &json!("CALCULATE(expression, [filter]...)"),
            &json!("any"),
            &json!(true)
        )
    );
    assert!(h["description"].as_str().unwrap().ends_with('.'), "{h}");

    let out = r.run("dax.hover", hover("RETURN", 0)).await;
    assert_eq!(
        (&out["hover"]["kind"], &out["hover"]["name"]),
        (&json!("keyword"), &json!("RETURN"))
    );
    assert!(out["hover"].get("supported").is_none(), "{out}");

    // Without a document, model names are not described; functions are.
    let out = r
        .run(
            "dax.hover",
            json!({"text": text, "position": {"offset": start + 1}}),
        )
        .await;
    assert!(
        out.get("hover").is_none() && out.get("documentRevision").is_none(),
        "{out}"
    );

    // A measure created by an edit is described at once.
    r.run(
        "model.measure.create",
        json!({"docId": doc, "table": "Stores", "name": "Store Count", "expression": "COUNTROWS(Stores)"}),
    )
    .await;
    let out = r
        .run(
            "dax.hover",
            json!({"docId": doc, "text": "[Store Count]", "position": {"offset": 1}}),
        )
        .await;
    assert_eq!(
        (&out["hover"]["table"], &out["hover"]["expression"]),
        (&json!("Stores"), &json!("COUNTROWS(Stores)"))
    );

    let err = r
        .try_run(
            "dax.hover",
            json!({"docId": "d404", "text": "1", "position": {"offset": 0}}),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, pbi_ops::OpErrorCode::NoDocument, "{err:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn completion_suggests_model_names_variables_and_functions() {
    let Some(path) = fixture(PBIP) else {
        return;
    };
    let r = Runner::new();
    let doc = r.open(json!({"path": s(&path)})).await;
    let at = |marked: &str| {
        let offset = marked.find('¦').unwrap();
        json!({"docId": doc, "text": marked.replace('¦', ""), "position": {"offset": offset}})
    };

    // A measure inside an unterminated bracket.
    let out = r.run("dax.complete", at("[Total Sa¦")).await;
    assert!(out["documentRevision"].is_u64(), "{out}");
    assert_eq!(
        (
            &out["start"]["offset"],
            &out["end"]["offset"],
            &out["prefix"]
        ),
        (&json!(0), &json!(9), &json!("Total Sa"))
    );
    let first = &out["items"][0];
    assert_eq!(
        (
            &first["label"],
            &first["kind"],
            &first["table"],
            &first["insertText"]
        ),
        (
            &json!("[Total Sales]"),
            &json!("measure"),
            &json!("Orders"),
            &json!("[Total Sales]")
        )
    );

    // A column of the qualifying table.
    let out = r.run("dax.complete", at("Stores[Reg¦]")).await;
    let first = &out["items"][0];
    assert_eq!(
        (&first["label"], &first["kind"], &first["detail"]),
        (
            &json!("[Region]"),
            &json!("column"),
            &json!("column (string)")
        )
    );
    assert!(
        out["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["table"] == "Stores"),
        "{out}"
    );

    // Variables in scope first.
    let out = r.run("dax.complete", at("VAR region = 1 RETURN re¦")).await;
    assert_eq!(
        (&out["items"][0]["label"], &out["items"][0]["kind"]),
        (&json!("region"), &json!("variable"))
    );

    // A table parameter ranks tables first; 100 items by default.
    let out = r.run("dax.complete", at("SUMX(¦")).await;
    assert_eq!(out["items"][0]["kind"], "table", "{out}");
    assert_eq!(out["items"].as_array().unwrap().len(), 100);
    assert_eq!(out["incomplete"], true);

    let out = r
        .run(
            "dax.complete",
            json!({"docId": doc, "text": "SU", "position": {"offset": 2}, "maxItems": 3}),
        )
        .await;
    assert_eq!(out["items"].as_array().unwrap().len(), 3);
    assert_eq!(out["incomplete"], true);

    // Without a document: functions, no model names.
    let out = r
        .run(
            "dax.complete",
            json!({"text": "[Total Sa", "position": {"offset": 9}}),
        )
        .await;
    assert!(out.get("documentRevision").is_none(), "{out}");
    assert_eq!(out["items"], json!([]));
    let out = r
        .run(
            "dax.complete",
            json!({"text": "sum", "position": {"line": 1, "column": 4}}),
        )
        .await;
    assert_eq!(
        out["items"][0],
        json!({
            "label": "SUM",
            "kind": "function",
            "insertText": "SUM(",
            "detail": "SUM(column) → number",
            "description": out["items"][0]["description"].clone(),
        })
    );

    for max in [0, 1001] {
        let err = r
            .try_run(
                "dax.complete",
                json!({"text": "S", "position": {"offset": 1}, "maxItems": max}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, pbi_ops::OpErrorCode::InvalidParams, "{max}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tokens_and_references_resolve_model_names() {
    let Some(path) = fixture(PBIP) else {
        return;
    };
    let r = Runner::new();
    let doc = r.open(json!({"path": s(&path)})).await;
    let text = "VAR \u{e9} = [Total Sales]\n\
                RETURN DIVIDE(\u{e9}, CALCULATE([total sales], ALL(Stores[Region]), Nope))";

    let out = r
        .run("dax.tokens", json!({"docId": doc, "text": text}))
        .await;
    assert!(out["documentRevision"].is_u64(), "{out}");
    let tokens = out["tokens"].as_array().unwrap();
    let summary: Vec<String> = tokens
        .iter()
        .map(|t| {
            let start = t["start"]["offset"].as_u64().unwrap() as usize;
            let end = t["end"]["offset"].as_u64().unwrap() as usize;
            format!(
                "{}:{}{}{}",
                &text[start..end],
                t["kind"].as_str().unwrap(),
                if t["declaration"] == true { "*" } else { "" },
                if t["unresolved"] == true { "?" } else { "" }
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            "VAR:keyword",
            "\u{e9}:variable*",
            "=:operator",
            "[Total Sales]:measure",
            "RETURN:keyword",
            "DIVIDE:function",
            "\u{e9}:variable",
            "CALCULATE:function",
            "[total sales]:measure",
            "ALL:function",
            "Stores:table",
            "[Region]:column",
            "Nope:table?",
        ]
    );
    // Line, column and UTF-16 offset of each position.
    assert_eq!(
        tokens[4]["start"],
        json!({"offset": 23, "utf16Offset": 22, "line": 2, "column": 1})
    );
    assert!(tokens[0].get("declaration").is_none(), "{out}");

    // A measure: every use, its model objectId, no definition in the text.
    let measures = r.run("model.measures", json!({"docId": doc})).await;
    let total_sales = measures["measures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "Total Sales")
        .map(|m| m["id"].clone())
        .unwrap();
    let at = |offset: usize| json!({"docId": doc, "text": text, "position": {"offset": offset}});
    let out = r
        .run("dax.references", at(text.find("total").unwrap()))
        .await;
    assert_eq!(
        out["symbol"],
        json!({"kind": "measure", "name": "[Total Sales]", "table": "Orders", "objectId": total_sales})
    );
    assert!(out.get("definition").is_none(), "{out}");
    let starts: Vec<&Value> = out["references"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| &r["start"]["offset"])
        .collect();
    assert_eq!(starts, [&json!(9), &json!(text.find("[total").unwrap())]);

    // A column and a table carry their objectIds too.
    let out = r
        .run("dax.references", at(text.find("Region").unwrap()))
        .await;
    assert_eq!(
        (&out["symbol"]["kind"], &out["symbol"]["name"]),
        (&json!("column"), &json!("Stores[Region]"))
    );
    assert!(out["symbol"]["objectId"].is_u64(), "{out}");
    let out = r
        .run("dax.references", at(text.find("Stores").unwrap()))
        .await;
    assert!(out["symbol"]["objectId"].is_u64(), "{out}");
    assert_ne!(out["symbol"]["objectId"], json!(total_sales));
    // An unknown name has none.
    let out = r
        .run("dax.references", at(text.find("Nope").unwrap()))
        .await;
    assert_eq!(out["symbol"]["kind"], "table");
    assert!(out["symbol"].get("objectId").is_none(), "{out}");

    // A variable: go to its declaration, by line and column.
    let out = r
        .run(
            "dax.references",
            json!({"text": text, "position": {"line": 2, "column": 15}}),
        )
        .await;
    assert!(out.get("documentRevision").is_none(), "{out}");
    assert_eq!(out["symbol"], json!({"kind": "variable", "name": "\u{e9}"}));
    assert_eq!(
        out["definition"],
        json!({
            "start": {"offset": 4, "utf16Offset": 4, "line": 1, "column": 5},
            "end": {"offset": 6, "utf16Offset": 5, "line": 1, "column": 6},
        })
    );
    assert_eq!(out["references"][0]["declaration"], true);
    assert_eq!(out["references"].as_array().unwrap().len(), 2);

    // Not on a name.
    let out = r.run("dax.references", at(1)).await;
    assert_eq!(out["references"], json!([]));
    assert!(out.get("symbol").is_none(), "{out}");
}

/// Every query Power BI Desktop ran without an error
/// (`reference/groundtruth/recordings/`, ADR-0011), checked against the
/// model it ran on: no error diagnostic and no name highlighting marks
/// unknown.
#[tokio::test(flavor = "multi_thread")]
async fn queries_desktop_runs_have_no_errors_or_unknown_names() {
    let root = reference_root().join("groundtruth");
    let corpus_dir = root.join("corpus");
    if !corpus_dir.is_dir() {
        println!("SKIP: {} missing", corpus_dir.display());
        return;
    }
    let r = Runner::new();
    let mut checked = 0;
    let mut wrong = Vec::new();
    let mut corpora: Vec<PathBuf> = std::fs::read_dir(&corpus_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    corpora.sort();
    for corpus_path in corpora {
        let read = |path: &std::path::Path| -> Value {
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
        };
        let corpus = read(&corpus_path);
        // DAX corpora have no `language`; M and TMDL corpora are checked elsewhere.
        if corpus.get("language").is_some_and(|l| l != "dax") {
            continue;
        }
        let recording_path = root
            .join("recordings")
            .join(corpus_path.file_name().unwrap());
        if !recording_path.exists() {
            println!("SKIP: {} missing", recording_path.display());
            continue;
        }
        let model = match corpus["fixture"].as_str().unwrap() {
            "acme" => reference_root().join(PBIP),
            "dax-lab" => root.join("fixtures/dax-lab/DaxLab.pbip"),
            other => reference_root()
                .join("samples/microsoft")
                .join(other.strip_prefix("sample:").unwrap()),
        };
        if !model.exists() {
            println!("SKIP: {} missing", model.display());
            continue;
        }
        let doc = r.open(json!({"path": s(&model)})).await;
        for case in read(&recording_path)["cases"].as_array().unwrap() {
            let (Some(query), None) = (case["query"].as_str(), case.get("error")) else {
                continue;
            };
            checked += 1;
            let at = format!("{}: {}", corpus_path.display(), case["id"]);
            let out = r
                .run("dax.diagnostics", json!({"docId": doc, "text": query}))
                .await;
            for d in out["diagnostics"].as_array().unwrap() {
                if d["severity"] == "error" {
                    wrong.push(format!("{at}\n  {d}"));
                }
            }
            let out = r
                .run("dax.tokens", json!({"docId": doc, "text": query}))
                .await;
            for t in out["tokens"].as_array().unwrap() {
                if t["unresolved"] == true {
                    let start = t["start"]["offset"].as_u64().unwrap() as usize;
                    let end = t["end"]["offset"].as_u64().unwrap() as usize;
                    wrong.push(format!(
                        "{at}\n  unresolved {}: {}",
                        &query[start..end],
                        t["kind"]
                    ));
                }
            }
            if let Some(problem) = format_problem(&r, &doc, query, "query").await {
                wrong.push(format!("{at}\n  {problem}"));
            }
        }
        r.run("doc.close", json!({"docId": doc})).await;
    }
    assert!(
        wrong.is_empty(),
        "{} of {checked}:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
    println!("{checked} queries Desktop ran: no errors, no unknown names, formatted idempotently");
}
