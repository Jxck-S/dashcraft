//! The M language service through `Registry::execute` (`m.diagnostics`):
//! Power BI Desktop's messages for unknown names and argument counts on the
//! Acme Outfitters example, suggestions and positions, checks without a
//! document, edits seen at once, and no false error on any M query of the
//! example and the Microsoft samples.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::PathBuf;

use common::{PBIP, Runner, fixture, reference_root, s};
use pbi_model::{Document, ExpressionKind, PartitionSource};
use serde_json::{Value, json};

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
                out.push((
                    format!("table '{}' partition '{}'", t.name, p.name),
                    expression.clone(),
                ));
            }
        }
    }
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

fn diagnostics(out: &Value) -> Vec<(String, String)> {
    out["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["code"].as_str().unwrap().to_owned(),
                d["message"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn no_false_errors_on_any_fixture_query() {
    let r = Runner::new();
    let mut checked = 0;
    for path in fixtures() {
        let doc = r.open(json!({"path": s(&path)})).await;
        let document = r.ws.get(&doc).unwrap().session().document();
        let texts = m_texts(&document);
        let mut problems = Vec::new();
        let mut warnings = Vec::new();
        for (at, text) in &texts {
            let out = r
                .run("m.diagnostics", json!({"docId": doc, "text": text}))
                .await;
            for d in out["diagnostics"].as_array().unwrap() {
                if d["severity"] == "warning" {
                    warnings.push(format!("{}: {}", d["code"], d["message"]));
                } else {
                    problems.push(format!("{at}: {text}\n  {d}"));
                }
            }
            assert_eq!(out["ok"], problems.is_empty(), "{at}");
        }
        assert!(
            problems.is_empty(),
            "{}: false errors in {} queries:\n{}",
            path.display(),
            texts.len(),
            problems.join("\n")
        );
        warnings.sort();
        warnings.dedup();
        // The example refreshes in DashCraft: nothing it uses is unsupported.
        if path.ends_with("AcmeOutfitters.pbip") {
            assert!(warnings.is_empty(), "{warnings:?}");
            assert!(texts.len() >= 10, "only {} queries", texts.len());
        }
        println!(
            "{}: {} queries, no errors; warnings: {warnings:?}",
            path.display(),
            texts.len()
        );
        checked += texts.len();
        r.run("doc.close", json!({"docId": doc})).await;
    }
    println!("{checked} queries checked");
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_names_and_argument_counts_against_the_model() {
    let Some(path) = fixture(PBIP) else { return };
    let r = Runner::new();
    let doc = r.open(json!({"path": s(&path)})).await;
    let text = "let\n    Source = Table.RowCnt(Orders),\n    Key = fnCleanKy(\"a\"),\n    Url = Text.Upper(StorageContainerUrl, \"en-US\", 3)\nin\n    Custmers";
    let out = r
        .run("m.diagnostics", json!({"docId": doc, "text": text}))
        .await;
    assert_eq!(out["ok"], false);
    assert!(out["documentRevision"].is_u64(), "{out}");
    let d = out["diagnostics"].as_array().unwrap();
    let got: Vec<(&str, &str, Option<&str>)> = d
        .iter()
        .map(|d| {
            (
                d["code"].as_str().unwrap(),
                d["message"].as_str().unwrap(),
                d["suggestion"].as_str(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (
                "unknownName",
                "The name 'Table.RowCnt' wasn't recognized.  Make sure it's spelled correctly.",
                Some("Table.RowCount"),
            ),
            (
                "unknownName",
                "The name 'fnCleanKy' wasn't recognized.  Make sure it's spelled correctly.",
                Some("fnCleanKey"),
            ),
            (
                "argumentCount",
                "3 arguments were passed to a function which expects between 1 and 2.",
                None,
            ),
            (
                "unknownName",
                "The name 'Custmers' wasn't recognized.  Make sure it's spelled correctly.",
                Some("Customers"),
            ),
        ]
    );
    // Positions: line 2, after `Source = `; the end is exclusive.
    assert_eq!(d[0]["start"]["line"], 2);
    assert_eq!(d[0]["start"]["column"], 14);
    assert_eq!(d[0]["end"]["column"], 26);
    assert_eq!(d[0]["severity"], "error");
    // The call covers the callee to the closing parenthesis.
    let call = &text[d[2]["start"]["offset"].as_u64().unwrap() as usize
        ..d[2]["end"]["offset"].as_u64().unwrap() as usize];
    assert_eq!(call, "Text.Upper(StorageContainerUrl, \"en-US\", 3)");

    // Valid queries of the model: other queries, parameters and functions.
    let ok = r
        .run(
            "m.diagnostics",
            json!({"docId": doc, "text": "Table.RowCount(Orders) + Text.Length(fnCleanKey(StorageContainerUrl))"}),
        )
        .await;
    assert_eq!(ok["ok"], true, "{ok}");
    assert_eq!(ok["diagnostics"], json!([]));

    // A query DashCraft cannot refresh is valid M: warnings, still ok.
    let web = r
        .run(
            "m.diagnostics",
            json!({"docId": doc, "text": "Table.Combine({Web.Contents(StorageContainerUrl), Contoso.Contents()})"}),
        )
        .await;
    assert_eq!(web["ok"], true, "{web}");
    let codes: Vec<&str> = web["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["unsupported", "extensionName"]);

    // Edits are seen at once: a renamed parameter is no longer known.
    r.run(
        "model.object.rename",
        json!({"docId": doc, "expression": "StorageContainerUrl", "newName": "Container"}),
    )
    .await;
    let after = r
        .run(
            "m.diagnostics",
            json!({"docId": doc, "text": "Web.Contents(StorageContainerUrl)"}),
        )
        .await;
    assert!(
        after["documentRevision"].as_u64() > out["documentRevision"].as_u64(),
        "{after}"
    );
    let unknown = &after["diagnostics"][1];
    assert_eq!(unknown["code"], "unknownName", "{after}");
    assert_eq!(unknown["suggestion"], json!(null), "{after}");
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_document_plain_names_may_be_other_queries() {
    let r = Runner::new();
    let check = |text: &'static str| {
        let r = &r;
        async move {
            let out = r.run("m.diagnostics", json!({"text": text})).await;
            assert!(out.get("documentRevision").is_none(), "{out}");
            out
        }
    };
    // Plain names may be queries; library typos and argument counts are still errors.
    assert_eq!(
        diagnostics(&check("let a = Orders, b = text.upper(a) in Text.Upper()").await),
        [
            (
                "unknownName".to_owned(),
                "The name 'text.upper' wasn't recognized.  Make sure it's spelled correctly."
                    .to_owned()
            ),
            (
                "argumentCount".to_owned(),
                "0 arguments were passed to a function which expects between 1 and 2.".to_owned()
            ),
        ]
    );
    // Exclusive scoping: a step's own name is not in scope in its initializer,
    // so the name is another query; recursion needs `@`.
    assert_eq!(
        diagnostics(&check("let Orders = Table.Buffer(Orders) in Orders").await),
        []
    );
    // A section binds its members.
    assert_eq!(
        diagnostics(&check("section Section1; shared A = B + 1; shared B = 2;").await),
        []
    );
    let syntax = check("let a = 1,  in a").await;
    assert_eq!(syntax["ok"], false);
    assert_eq!(syntax["diagnostics"][0]["code"], "syntax");
    assert_eq!(syntax["diagnostics"].as_array().unwrap().len(), 1);
    // Unknown parameters are refused.
    let err = r
        .try_run("m.diagnostics", json!({"text": "1", "kind": "query"}))
        .await
        .expect_err("unknown parameter");
    assert_eq!(err.code, pbi_ops::OpErrorCode::InvalidParams);
}
