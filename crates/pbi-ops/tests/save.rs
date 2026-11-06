//! `doc.save` through `Registry::execute` on a temporary PBIP project:
//! only changed files are written, renames follow into the report files
//! and back byte for byte, a reopen shows the saved document, and files
//! changed on disk are a conflict until `overwrite`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use common::{MINI_TOTAL, Runner, mini_pbip, tmp_dir};
use pbi_ops::OpErrorCode;
use serde_json::{Value, json};

const SALES_TMDL: &str = "Mini.SemanticModel/definition/tables/Sales.tmdl";
const SALES_DATA_TMDL: &str = "Mini.SemanticModel/definition/tables/Sales Data.tmdl";
const MODEL_TMDL: &str = "Mini.SemanticModel/definition/model.tmdl";
const TOTAL_CARD: &str = "Mini.Report/definition/pages/overview/visuals/total/visual.json";

/// Every file of the project, by path relative to `root` (`/`-separated).
fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_string_lossy();
                out.insert(rel.replace('\\', "/"), fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn strings(v: &Value) -> Vec<&str> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect()
}

async fn total_expression(r: &Runner, doc: &str) -> String {
    let measures = r.run("model.measures", json!({"docId": doc})).await;
    measures["measures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "Total")
        .unwrap()["expression"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn scalar(r: &Runner, doc: &str, dax: &str) -> Value {
    let out = r
        .run(
            "dax.query",
            json!({"docId": doc, "query": format!("EVALUATE ROW(\"n\", {dax})")}),
        )
        .await;
    out["tables"][0]["rows"][0][0].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn save_writes_the_edits_and_a_reopen_shows_them() {
    let dir = tmp_dir("save-");
    let pbip = mini_pbip(dir.path());
    let original = files(dir.path());
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    let save = json!({"docId": doc});

    // Nothing to save.
    let out = r.run("doc.save", save.clone()).await;
    assert_eq!(
        (&out["written"], &out["deleted"], &out["dirty"]),
        (&json!([]), &json!([]), &json!(false)),
        "{out}"
    );
    let project_dir = out["projectDir"].as_str().unwrap();
    assert!(
        Path::new(project_dir).is_absolute() && !project_dir.starts_with(r"\\?\"),
        "{project_dir}"
    );
    assert_eq!(files(dir.path()), original);

    // A measure edit and a move write only their files.
    let measures = r.run("model.measures", json!({"docId": doc})).await;
    let total = measures["measures"][0]["id"].as_u64().unwrap();
    r.run(
        "model.setMeasureExpression",
        json!({"docId": doc, "measureId": total, "expression": "SUM(Sales[Amount]) * 2"}),
    )
    .await;
    r.run(
        "report.visual.move",
        json!({"docId": doc, "page": "Overview", "visual": "total", "x": 20}),
    )
    .await;
    let out = r.run("doc.save", save.clone()).await;
    assert_eq!(strings(&out["written"]), [SALES_TMDL, TOTAL_CARD], "{out}");
    assert_eq!(
        (&out["deleted"], &out["revision"], &out["dirty"]),
        (&json!([]), &json!(2), &json!(false))
    );
    let history = r.run("doc.history", json!({"docId": doc})).await;
    assert_eq!(
        (&history["dirty"], &history["canUndo"]),
        (&json!(false), &json!(true)),
        "the history survives the save"
    );
    let saved = files(dir.path());
    let changed: Vec<&String> = saved
        .iter()
        .filter(|(path, bytes)| original.get(*path) != Some(*bytes))
        .map(|(path, _)| path)
        .collect();
    assert_eq!(changed, [TOTAL_CARD, SALES_TMDL]);
    let sales = String::from_utf8(saved[SALES_TMDL].clone()).unwrap();
    assert!(
        sales.contains("\tmeasure Total = SUM(Sales[Amount]) * 2\n"),
        "{sales}"
    );
    let card: Value = serde_json::from_slice(&saved[TOTAL_CARD]).unwrap();
    assert_eq!(card["position"]["x"], json!(20));
    assert_eq!(
        card["position"]["y"],
        json!(60.0),
        "untouched numbers keep their text"
    );

    // Renaming the table and a column moves the table file and rewrites the
    // model and every visual that uses them.
    r.run(
        "model.object.rename",
        json!({"docId": doc, "table": "Sales", "newName": "Sales Data"}),
    )
    .await;
    r.run(
        "model.object.rename",
        json!({"docId": doc, "table": "Sales Data", "column": "Amount", "newName": "Revenue"}),
    )
    .await;
    let out = r.run("doc.save", save.clone()).await;
    assert_eq!(strings(&out["deleted"]), [SALES_TMDL], "{out}");
    let written = strings(&out["written"]);
    for path in [SALES_DATA_TMDL, MODEL_TMDL, TOTAL_CARD] {
        assert!(written.contains(&path), "{path} not in {written:?}");
    }
    assert_eq!(
        written
            .iter()
            .filter(|p| p.ends_with("/visual.json"))
            .count(),
        4,
        "every visual uses Sales: {written:?}"
    );

    // A fresh workspace opens what was saved.
    let fresh = Runner::new();
    let reopened = fresh.open(json!({"path": pbip})).await;
    assert_eq!(
        total_expression(&fresh, &reopened).await,
        "SUM('Sales Data'[Revenue]) * 2"
    );
    assert_eq!(
        scalar(&fresh, &reopened, "[Total]").await,
        json!(2 * MINI_TOTAL)
    );
    let details = fresh
        .run(
            "report.visuals",
            json!({"docId": reopened, "page": "Details"}),
        )
        .await
        .to_string();
    assert!(
        details.contains("\"ref\":\"Sum('Sales Data'[Revenue])\"")
            && details.contains("\"nativeQueryRef\":\"Sum of Revenue\"")
            && !details.contains("\"table\":\"Sales\""),
        "{details}"
    );
    let lint = fresh.run("doc.lint", json!({"docId": reopened})).await;
    assert!(lint["summary"]["byRule"].get("LINT003").is_none(), "{lint}");
    let page = fresh
        .run(
            "render.page",
            json!({"docId": reopened, "page": "Overview"}),
        )
        .await;
    assert_eq!(page["ok"], true, "{}", page["visuals"]);

    // Undoing the renames and saving restores the files byte for byte.
    r.run("doc.undo", json!({"docId": doc})).await;
    r.run("doc.undo", json!({"docId": doc})).await;
    let out = r.run("doc.save", save).await;
    assert_eq!(strings(&out["deleted"]), [SALES_DATA_TMDL], "{out}");
    assert_eq!(files(dir.path()), saved);
}

#[tokio::test(flavor = "multi_thread")]
async fn files_changed_on_disk_are_a_conflict_until_overwrite() {
    let dir = tmp_dir("save-conflict-");
    let pbip = mini_pbip(dir.path());
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    let measures = r.run("model.measures", json!({"docId": doc})).await;
    let total = measures["measures"][0]["id"].as_u64().unwrap();
    r.run(
        "model.object.setDescription",
        json!({"docId": doc, "objectId": total, "text": "Sum of all amounts"}),
    )
    .await;

    // Another program edits a file and adds one.
    let sales = dir.path().join(SALES_TMDL);
    let external = fs::read_to_string(&sales)
        .unwrap()
        .replace("formatString: 0\n", "formatString: #,0\n");
    fs::write(&sales, &external).unwrap();
    fs::write(dir.path().join("Mini.Report/notes.txt"), "kept").unwrap();

    let err = r
        .try_run("doc.save", json!({"docId": doc}))
        .await
        .unwrap_err();
    assert_eq!(err.code, OpErrorCode::Conflict, "{err:?}");
    assert_eq!(
        err.data.unwrap()["conflicts"],
        json!([
            {"path": "Mini.Report/notes.txt", "change": "added"},
            {"path": SALES_TMDL, "change": "modified"}
        ])
    );
    assert_eq!(
        fs::read_to_string(&sales).unwrap(),
        external,
        "nothing written"
    );
    let history = r.run("doc.history", json!({"docId": doc})).await;
    assert_eq!(history["dirty"], true);

    let out = r
        .run("doc.save", json!({"docId": doc, "overwrite": true}))
        .await;
    assert_eq!(strings(&out["written"]), [SALES_TMDL], "{out}");
    assert_eq!(out["overwritten"].as_array().unwrap().len(), 2, "{out}");
    assert_eq!(out["dirty"], false);
    let saved = fs::read_to_string(&sales).unwrap();
    assert!(saved.contains("Sum of all amounts"), "{saved}");
    assert!(!saved.contains("#,0"), "the document's version wins");
    assert_eq!(
        fs::read_to_string(dir.path().join("Mini.Report/notes.txt")).unwrap(),
        "kept"
    );

    // The overwrite is the new baseline.
    let again = r.run("doc.save", json!({"docId": doc})).await;
    assert_eq!(
        (&again["written"], &again["overwritten"]),
        (&json!([]), &json!([]))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn only_pbip_projects_save_in_place() {
    let dir = tmp_dir("save-model-");
    mini_pbip(dir.path());
    let r = Runner::new();
    let doc = r
        .open(json!({"path": dir.path().join("Mini.SemanticModel"), "autoRefresh": false}))
        .await;
    let err = r
        .try_run("doc.save", json!({"docId": doc}))
        .await
        .unwrap_err();
    assert_eq!(err.code, OpErrorCode::Unsupported, "{err:?}");
}
