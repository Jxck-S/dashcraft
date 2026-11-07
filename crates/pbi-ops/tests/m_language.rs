//! The M language service through `Registry::execute`: `m.diagnostics`
//! (Power BI Desktop's messages for unknown names and argument counts on the
//! Acme Outfitters example, suggestions and positions, checks without a
//! document, edits seen at once, and no false error on any M query of the
//! example and the Microsoft samples), and `m.hover`, `m.signatureHelp` and
//! `m.complete` (the example's parameters, functions, tables, steps and the
//! library; UTF-16 positions; bad parameters; every query of the example).

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

/// The UTF-8 byte offset just inside the `n`th (0-based) `needle` of `text`, plus `delta`.
fn at(text: &str, needle: &str, n: usize, delta: usize) -> usize {
    text.match_indices(needle).nth(n).unwrap().0 + delta
}

#[tokio::test(flavor = "multi_thread")]
async fn hover_signature_help_and_completion_against_the_model() {
    let Some(path) = fixture(PBIP) else { return };
    let r = Runner::new();
    let doc = r.open(json!({"path": s(&path)})).await;
    let text = "let\n    Source = Orders,\n    Url = Web.Contents(StorageContainerUrl),\n    Key = fnCleanKey(\"a\"),\n    Rows = Table.RowCount(Source)\nin\n    Rows";
    let hover = |offset: usize| {
        let r = &r;
        let doc = doc.clone();
        async move {
            let out = r
                .run(
                    "m.hover",
                    json!({"docId": doc, "text": text, "position": {"offset": offset}}),
                )
                .await;
            assert!(out["documentRevision"].is_u64(), "{out}");
            out["hover"].clone()
        }
    };

    // A Power Query parameter: its type, description and definition.
    let h = hover(at(text, "StorageContainerUrl", 0, 3)).await;
    assert_eq!(h["kind"], "queryParameter", "{h}");
    assert_eq!(h["name"], "StorageContainerUrl");
    assert_eq!(h["typeName"], "Text");
    assert!(
        h["description"]
            .as_str()
            .unwrap()
            .starts_with("Fictional Azure Blob container"),
        "{h}"
    );
    assert!(
        h["expression"]
            .as_str()
            .unwrap()
            .contains("IsParameterQuery=true"),
        "{h}"
    );
    assert!(
        h["markdown"]
            .as_str()
            .unwrap()
            .starts_with("`StorageContainerUrl`: query parameter (Text)"),
        "{h}"
    );
    // The hovered range is the whole name, line 3.
    assert_eq!(h["start"]["line"], 3);
    assert_eq!(h["start"]["column"], 24);
    assert_eq!(h["end"]["column"], 43);

    // A query that is a function, with its signature from its definition.
    let h = hover(at(text, "fnCleanKey", 0, 0)).await;
    assert_eq!(h["kind"], "queryFunction", "{h}");
    assert_eq!(h["signature"], "fnCleanKey(x as text) as text");
    assert_eq!(
        h["description"],
        "Trims and upper-cases a business key (customer, store and product ids)."
    );

    // A table loaded by an M query, a step and a library function.
    let h = hover(at(text, "Orders", 0, 6)).await;
    assert_eq!(h["kind"], "table", "{h}");
    assert!(
        h["expression"]
            .as_str()
            .unwrap()
            .contains("AzureStorage.Blobs(StorageContainerUrl)"),
        "{h}"
    );
    let h = hover(at(text, "Source", 1, 2)).await;
    assert_eq!(h["kind"], "step", "{h}");
    assert_eq!(h["expression"], "Orders");
    let h = hover(at(text, "RowCount", 0, 0)).await;
    assert_eq!(h["kind"], "function", "{h}");
    assert_eq!(h["name"], "Table.RowCount");
    assert_eq!(h["supported"], true);
    assert_eq!(h["signature"], "Table.RowCount(table as table) as number");
    // A cursor just after a name is on it; a function DashCraft does not evaluate says so.
    let h = hover(at(text, "(", 0, 0)).await;
    assert_eq!(h["name"], "Web.Contents", "{h}");
    assert_eq!(h["supported"], false);
    assert!(
        h["unsupported"]
            .as_str()
            .unwrap()
            .starts_with("DashCraft does not evaluate 'Web.Contents'."),
        "{h}"
    );
    // Nothing on white space or in a text literal.
    assert_eq!(hover(at(text, "= ", 0, 1)).await, json!(null));
    assert_eq!(hover(at(text, "\"a\"", 0, 1)).await, json!(null));

    // Signature help for a query function and a library function.
    let help = |text: &'static str, offset: usize| {
        let r = &r;
        let doc = doc.clone();
        async move {
            r.run(
                "m.signatureHelp",
                json!({"docId": doc, "text": text, "position": {"offset": offset}}),
            )
            .await["signature"]
                .clone()
        }
    };
    let sig = help("fnCleanKey(", 11).await;
    assert_eq!(sig["kind"], "queryFunction", "{sig}");
    assert_eq!(sig["label"], "fnCleanKey(x as text) as text");
    assert_eq!(sig["returns"], "text");
    assert_eq!(sig["argument"], 0);
    assert_eq!(sig["activeParameter"], 0);
    assert_eq!(
        sig["parameters"],
        json!([{"name": "x", "type": "text", "optional": false, "labelStart": 11, "labelEnd": 20}])
    );
    let src = "Table.AddColumn(Source, ";
    let sig = help(src, src.len()).await;
    assert_eq!(sig["name"], "Table.AddColumn", "{sig}");
    assert_eq!(sig["argument"], 1);
    assert_eq!(sig["activeParameter"], 1);
    let label = sig["label"].as_str().unwrap();
    let p = &sig["parameters"][1];
    assert_eq!(p["name"], "newColumnName");
    let (a, b) = (
        p["labelStart"].as_u64().unwrap() as usize,
        p["labelEnd"].as_u64().unwrap() as usize,
    );
    assert_eq!(&label[a..b], "newColumnName as text");
    assert_eq!(sig["nameStart"]["offset"], 0);
    assert_eq!(sig["nameEnd"]["offset"], 15);
    // Outside any call there is none.
    assert_eq!(help("Table.RowCount(Source) + ", 25).await, json!(null));

    // Completion: the model's names, ranked and described.
    let complete = |text: &'static str, max: usize| {
        let r = &r;
        let doc = doc.clone();
        async move {
            r.run(
                "m.complete",
                json!({"docId": doc, "text": text, "position": {"offset": text.len()}, "maxItems": max}),
            )
            .await
        }
    };
    let c = complete("let Source = Ord", 100).await;
    assert_eq!(c["prefix"], "Ord");
    assert_eq!(c["start"]["offset"], 13);
    assert_eq!(c["end"]["offset"], 16);
    assert_eq!(c["items"][0]["label"], "Orders", "{c}");
    assert_eq!(c["items"][0]["kind"], "table");
    let c = complete("fnClean", 100).await;
    assert_eq!(c["items"][0]["kind"], "queryFunction", "{c}");
    assert_eq!(c["items"][0]["detail"], "fnCleanKey(x as text) as text");
    let c = complete("Table.", 1).await;
    assert_eq!(c["items"].as_array().unwrap().len(), 1);
    assert_eq!(c["incomplete"], true);
    let c = complete("let #\"Changed Type\" = 1 in Chan", 100).await;
    assert_eq!(c["items"][0]["insertText"], "#\"Changed Type\"", "{c}");

    // Edits are seen at once.
    r.run(
        "model.object.rename",
        json!({"docId": doc, "expression": "StorageContainerUrl", "newName": "Container"}),
    )
    .await;
    let c = complete("Containe", 100).await;
    assert_eq!(c["items"][0]["label"], "Container", "{c}");
    assert_eq!(c["items"][0]["kind"], "queryParameter");
    assert_eq!(c["items"][0]["detail"], "query parameter (Text)");
}

#[tokio::test(flavor = "multi_thread")]
async fn language_service_without_a_document_and_bad_params() {
    let r = Runner::new();
    // Without a document the text's own names and the library are known.
    let text = "let Source = 1, Sorted = Sou";
    let c = r
        .run(
            "m.complete",
            json!({"text": text, "position": {"line": 1, "column": 29}}),
        )
        .await;
    assert!(c.get("documentRevision").is_none(), "{c}");
    assert_eq!(c["items"][0]["label"], "Source", "{c}");
    assert_eq!(c["items"][0]["kind"], "step");
    assert!(c["items"].as_array().unwrap().len() <= 100);

    // Positions in UTF-16 code units: a name with characters outside the BMP.
    let text = "let 𝔣 = (né as text, optional 𝔵 as number) => né in 𝔣(\"a\", ";
    let utf16 = text.encode_utf16().count();
    let sig = r
        .run(
            "m.signatureHelp",
            json!({"text": text, "position": {"utf16Offset": utf16}}),
        )
        .await["signature"]
        .clone();
    assert_eq!(sig["kind"], "step", "{sig}");
    assert_eq!(sig["argument"], 1);
    let label: Vec<u16> = sig["label"].as_str().unwrap().encode_utf16().collect();
    let slice = |p: &Value| {
        let (a, b) = (
            p["labelStart"].as_u64().unwrap() as usize,
            p["labelEnd"].as_u64().unwrap() as usize,
        );
        String::from_utf16(&label[a..b]).unwrap()
    };
    assert_eq!(slice(&sig["parameters"][0]), "né as text");
    assert_eq!(slice(&sig["parameters"][1]), "𝔵 as number");
    assert_eq!(sig["parameters"][1]["optional"], true);
    let h = r
        .run(
            "m.hover",
            json!({"text": text, "position": {"offset": text.rfind("né").unwrap()}}),
        )
        .await["hover"]
        .clone();
    assert_eq!(h["kind"], "parameter", "{h}");
    assert_eq!(h["typeName"], "text");

    // Bad parameters are refused.
    for (id, params) in [
        (
            "m.complete",
            json!({"text": "a", "position": {"offset": 1}, "maxItems": 0}),
        ),
        (
            "m.complete",
            json!({"text": "a", "position": {"offset": 1}, "maxItems": 1001}),
        ),
        (
            "m.complete",
            json!({"text": "a", "position": {"offset": 2}}),
        ),
        (
            "m.hover",
            json!({"text": "a", "position": {"offset": 1}, "kind": "x"}),
        ),
        ("m.hover", json!({"text": "a", "position": {"line": 1}})),
        ("m.signatureHelp", json!({"text": "a"})),
        (
            "m.hover",
            json!({"docId": "nope", "text": "a", "position": {"offset": 0}}),
        ),
    ] {
        let err = r
            .try_run(id, params.clone())
            .await
            .expect_err("bad parameters");
        assert!(
            matches!(
                err.code,
                pbi_ops::OpErrorCode::InvalidParams | pbi_ops::OpErrorCode::NoDocument
            ),
            "{id} {params}: {err:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn language_service_on_every_example_query() {
    let Some(path) = fixture(PBIP) else { return };
    let r = Runner::new();
    let doc = r.open(json!({"path": s(&path)})).await;
    let document = r.ws.get(&doc).unwrap().session().document();
    let mut runs = 0;
    for (at, text) in m_texts(&document) {
        let mut offset = 0;
        while offset <= text.len() {
            if text.is_char_boundary(offset) {
                let position = json!({"offset": offset});
                for id in ["m.hover", "m.signatureHelp", "m.complete"] {
                    let out = r
                        .try_run(
                            id,
                            json!({"docId": doc, "text": text, "position": position}),
                        )
                        .await
                        .unwrap_or_else(|e| panic!("{at} {id} @{offset}: {e:?}"));
                    if id == "m.complete" {
                        let (a, b) = (
                            out["start"]["offset"].as_u64().unwrap() as usize,
                            out["end"]["offset"].as_u64().unwrap() as usize,
                        );
                        assert!(
                            a <= offset && offset <= b && b <= text.len(),
                            "{at} @{offset}"
                        );
                    }
                    runs += 1;
                }
            }
            offset += 7;
        }
    }
    assert!(runs > 300, "{runs}");
}
