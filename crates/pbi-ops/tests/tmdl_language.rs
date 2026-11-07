//! The TMDL language service through `Registry::execute`: `tmdl.diagnostics` (unknown
//! keywords with a suggestion, values, unresolved references across the files of a model
//! folder, positions, warnings that keep `ok`, bad parameters, and no error on the
//! Acme Outfitters model).

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
