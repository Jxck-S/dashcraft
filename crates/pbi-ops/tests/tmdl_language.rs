//! The TMDL language service through `Registry::execute`: `tmdl.diagnostics` (unknown
//! keywords with a suggestion, values, unresolved references across the files of a model
//! folder, positions, warnings that keep `ok`, bad parameters, and no error on the
//! Acme Outfitters model), `tmdl.complete` and `tmdl.hover` (keywords, values and names across
//! the files of the folder, positions, declarations, bad parameters).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::Path;

use common::{ACME, Runner, fixture};
use serde_json::{Value, json};

fn codes(out: &Value) -> Vec<String> {
    out["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap().to_owned())
        .collect()
}

/// Every `.tmdl` file under `dir`, with its path relative to `dir` (`/` separators).
fn folder(dir: &Path) -> Vec<Value> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<Value>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(root, &p, out);
            } else if p.extension().is_some_and(|e| e == "tmdl") {
                let rel = p.strip_prefix(root).unwrap().to_string_lossy();
                out.push(json!({
                    "path": rel.replace('\\', "/"),
                    "text": std::fs::read_to_string(&p).unwrap(),
                }));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn diagnostics_report_unknown_keywords_values_and_positions() {
    let r = Runner::new();
    let text = "table Stores\n\tcolumn Region\n\t\tdataTyp: string\n\t\tisHidden: yes\n";
    let out = r
        .run(
            "tmdl.diagnostics",
            json!({"files": [{"path": "tables/Stores.tmdl", "text": text}]}),
        )
        .await;
    assert_eq!(out["ok"], false, "{out}");
    assert_eq!(codes(&out), ["TMDL101", "TMDL105"], "{out}");
    let unknown = &out["diagnostics"][0];
    assert_eq!(unknown["path"], "tables/Stores.tmdl");
    assert_eq!(unknown["severity"], "error");
    assert_eq!(unknown["suggestion"], "dataType", "{unknown}");
    assert_eq!(unknown["start"]["line"], 3);
    assert_eq!(unknown["start"]["column"], 3);
    assert_eq!(unknown["end"]["column"], 10);
    let value = &out["diagnostics"][1];
    assert_eq!(value["start"]["line"], 4);
    assert!(
        value["message"].as_str().unwrap().contains("yes"),
        "{value}"
    );

    // A clean file, and a warning (an orphan description) that keeps ok.
    let out = r
        .run(
            "tmdl.diagnostics",
            json!({"files": [{"path": "t.tmdl", "text": "table Stores\n\tcolumn Region\n\t\tdataType: string\n"}]}),
        )
        .await;
    assert_eq!(out, json!({"ok": true, "diagnostics": []}));
    let out = r
        .run(
            "tmdl.diagnostics",
            json!({"files": [{"path": "t.tmdl", "text": "table Stores\n\tcolumn Region\n\t\tdataType: string\n\n/// lost\n"}]}),
        )
        .await;
    assert_eq!(out["ok"], true, "{out}");
    assert_eq!(codes(&out), ["TMDL006"], "{out}");
    assert_eq!(out["diagnostics"][0]["severity"], "warning");
}

#[tokio::test(flavor = "multi_thread")]
async fn diagnostics_resolve_references_across_a_model_folder() {
    let r = Runner::new();
    let files = |sort_by: &str| {
        json!({"files": [
            {"path": "model.tmdl", "text": "model Model\n\tculture: en-US\n"},
            {"path": "tables/Stores.tmdl", "text": format!(
                "table Stores\n\tcolumn Region\n\t\tdataType: string\n\t\tsortByColumn: {sort_by}\n\n\tcolumn RegionOrder\n\t\tdataType: int64\n"
            )},
        ]})
    };
    let out = r.run("tmdl.diagnostics", files("RegionOrder")).await;
    assert_eq!(out, json!({"ok": true, "diagnostics": []}));
    let out = r.run("tmdl.diagnostics", files("RegionOrdr")).await;
    assert_eq!(out["ok"], false, "{out}");
    assert_eq!(codes(&out), ["TMDL108"], "{out}");
    let d = &out["diagnostics"][0];
    assert_eq!(d["path"], "tables/Stores.tmdl");
    assert_eq!(d["suggestion"], "RegionOrder", "{d}");
    assert_eq!(d["start"]["line"], 4);

    // A single file: references are not resolved.
    let out = r
        .run(
            "tmdl.diagnostics",
            json!({"files": [files("RegionOrdr")["files"][1].clone()]}),
        )
        .await;
    assert_eq!(out["ok"], true, "{out}");
}

#[tokio::test(flavor = "multi_thread")]
async fn diagnostics_find_no_error_in_the_example_model() {
    let Some(dir) = fixture(&format!("{ACME}/AcmeOutfitters.SemanticModel/definition")) else {
        return;
    };
    let files = folder(&dir);
    assert!(files.len() >= 10, "only {} files", files.len());
    let out = Runner::new()
        .run("tmdl.diagnostics", json!({ "files": files }))
        .await;
    assert_eq!(out, json!({"ok": true, "diagnostics": []}));
}

#[tokio::test(flavor = "multi_thread")]
async fn diagnostics_reject_bad_parameters() {
    let r = Runner::new();
    let many: Vec<Value> = (0..10_001)
        .map(|i| json!({"path": format!("t{i}.tmdl"), "text": ""}))
        .collect();
    for params in [
        json!({ "files": many }),
        json!({"files": [{"path": "t.tmdl", "text": "", "kind": "table"}]}),
        json!({"text": "table T\n"}),
    ] {
        let err = r
            .try_run("tmdl.diagnostics", params)
            .await
            .expect_err("bad parameters");
        assert_eq!(err.code, pbi_ops::OpErrorCode::InvalidParams, "{err:?}");
    }
}

fn labels(out: &Value) -> Vec<String> {
    out["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn complete_offers_keywords_values_and_names_of_the_folder() {
    let r = Runner::new();
    // The properties of a column, with the replaced word and the text to insert.
    let out = r
        .run(
            "tmdl.complete",
            json!({"text": "table Stores\n\tcolumn Region\n\t\tdataT", "position": {"line": 3, "column": 8}}),
        )
        .await;
    assert_eq!(out["prefix"], "dataT", "{out}");
    assert_eq!(out["start"]["column"], 3);
    assert_eq!(out["end"]["column"], 8);
    let first = &out["items"][0];
    assert_eq!(first["label"], "dataType", "{out}");
    assert_eq!(first["kind"], "property");
    assert_eq!(first["insertText"], "dataType: ");
    assert!(
        first["description"].as_str().unwrap().contains("`column`"),
        "{first}"
    );
    assert_eq!(out["incomplete"], false);

    // A value of the property's enumeration.
    let out = r
        .run(
            "tmdl.complete",
            json!({"text": "table Stores\n\tcolumn Region\n\t\tsummarizeBy: no", "position": {"offset": 45}}),
        )
        .await;
    assert_eq!(labels(&out), ["none"], "{out}");
    assert_eq!(out["items"][0]["kind"], "value");

    // A column declared in another file of the folder; the edited file replaces its old text.
    let files = json!([
        {"path": "tables/Stores.tmdl", "text": "table Stores\n\tcolumn Gone\n"},
        {"path": "tables/More.tmdl", "text": "table Stores\n\t/// Display order.\n\tcolumn 'Region Order'\n"},
    ]);
    let text = "table Stores\n\tcolumn Region\n\t\tsortByColumn: ";
    let out = r
        .run(
            "tmdl.complete",
            json!({"text": text, "position": {"offset": text.len()}, "path": "tables/Stores.tmdl", "files": files}),
        )
        .await;
    assert_eq!(labels(&out), ["'Region Order'", "Region"], "{out}");
    assert_eq!(out["items"][0]["kind"], "column");
    assert_eq!(out["items"][0]["insertText"], "'Region Order'");
    assert_eq!(out["items"][0]["description"], "Display order.");

    // maxItems keeps the best matches and says more matched.
    let out = r
        .run(
            "tmdl.complete",
            json!({"text": "table Stores\n\t", "position": {"offset": 14}, "maxItems": 2}),
        )
        .await;
    assert_eq!(out["items"].as_array().unwrap().len(), 2, "{out}");
    assert_eq!(out["incomplete"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn hover_describes_keywords_and_declarations() {
    let r = Runner::new();
    let out = r
        .run(
            "tmdl.hover",
            json!({"text": "table Stores\n\tcolumn Region\n\t\tsummarizeBy: none\n", "position": {"line": 3, "column": 5}}),
        )
        .await;
    let hover = &out["hover"];
    assert_eq!(hover["kind"], "property", "{out}");
    assert_eq!(hover["name"], "summarizeBy");
    assert_eq!(hover["start"]["column"], 3);
    assert_eq!(hover["end"]["column"], 14);
    assert!(
        hover["markdown"]
            .as_str()
            .unwrap()
            .contains("AggregateFunction"),
        "{hover}"
    );

    // A column of another file: its declaration's path and line.
    let out = r
        .run(
            "tmdl.hover",
            json!({
                "text": "relationship r1\n\tfromColumn: Orders.Store\n",
                "position": {"line": 2, "column": 22},
                "path": "relationships.tmdl",
                "files": [{"path": "tables/Orders.tmdl", "text": "table Orders\n\n\tcolumn Store\n"}],
            }),
        )
        .await;
    let hover = &out["hover"];
    assert_eq!(hover["kind"], "column", "{out}");
    assert_eq!(hover["name"], "Orders.Store");
    assert_eq!(
        hover["declaration"],
        json!({"path": "tables/Orders.tmdl", "line": 3})
    );

    // Nothing to describe in an expression.
    let out = r
        .run(
            "tmdl.hover",
            json!({"text": "table T\n\tmeasure M = SUM(T[A])\n", "position": {"line": 2, "column": 16}}),
        )
        .await;
    assert_eq!(out, json!({}));
}

#[tokio::test(flavor = "multi_thread")]
async fn complete_and_hover_reject_bad_parameters() {
    let r = Runner::new();
    let many: Vec<Value> = (0..10_001)
        .map(|i| json!({"path": format!("t{i}.tmdl"), "text": ""}))
        .collect();
    for (id, params) in [
        (
            "tmdl.complete",
            json!({"text": "table T\n", "position": {"offset": 0}, "maxItems": 0}),
        ),
        (
            "tmdl.complete",
            json!({"text": "table T\n", "position": {"offset": 0}, "maxItems": 1001}),
        ),
        (
            "tmdl.complete",
            json!({"text": "table T\n", "position": {"offset": 99}}),
        ),
        (
            "tmdl.complete",
            json!({"text": "table T\n", "position": {"offset": 0}, "files": many.clone()}),
        ),
        (
            "tmdl.hover",
            json!({"text": "table T\n", "position": {"line": 9, "column": 1}}),
        ),
        (
            "tmdl.hover",
            json!({"text": "table T\n", "position": {"offset": 0}, "folder": "x"}),
        ),
        (
            "tmdl.hover",
            json!({"text": "table T\n", "position": {"offset": 0}, "files": many}),
        ),
    ] {
        let err = r
            .try_run(id, params.clone())
            .await
            .expect_err("bad parameters");
        assert_eq!(
            err.code,
            pbi_ops::OpErrorCode::InvalidParams,
            "{id} {params}: {err:?}"
        );
    }
}
