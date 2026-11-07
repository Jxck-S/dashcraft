//! End to end through `Registry::execute` on the synthetic Acme Outfitters
//! example (`reference/examples/acme-outfitters/`): open → inspect → lint →
//! pages → render, then refresh → measures → DAX → canvas with the values of
//! its `expected.json`; and the local data cache of a PBIP built from a
//! committed Microsoft sample.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use common::{
    ACME, BLOB_MAP, MS_PBIX, ORDERS_CSV, PBIP, Runner, copy_dir, expected, fixture, ms_expected,
    reference_root, tmp_dir,
};
use pbi_ops::{CallerKind, Ctx, OpErrorCode};
use serde_json::{Value, json};

fn measure<'a>(out: &'a Value, name: &str) -> &'a Value {
    out["measures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == name)
        .unwrap_or_else(|| panic!("no measure {name}"))
}

fn table<'a>(tables: &'a Value, name: &str) -> &'a Value {
    tables
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == name)
        .unwrap_or_else(|| panic!("no table {name} in {tables}"))
}

/// `visual.json` files under `dir` (counted on disk, independently of DashCraft).
fn visual_files(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .map(|p| {
            if p.is_dir() {
                visual_files(&p)
            } else {
                usize::from(p.file_name().unwrap() == "visual.json")
            }
        })
        .sum()
}

/// A copy of the example with one stale `queryRef` (a LINT003 warning) on the
/// Executive Summary KPI card.
fn acme_with_stale_query_ref(dir: &Path) -> PathBuf {
    let root = dir.join("acme");
    copy_dir(&reference_root().join(ACME), &root);
    let visual = root.join(
        "Acme Outfitters - Overview.Report/definition/pages/ef796b39c816a4afa4f1/visuals/f7a9c9413fef68532313/visual.json",
    );
    let text = std::fs::read_to_string(&visual).unwrap();
    let stale = text.replacen(
        "\"queryRef\": \"Orders.Total Sales\"",
        "\"queryRef\": \"Orders.Sales Total\"",
        1,
    );
    assert_ne!(stale, text, "the KPI card projects Orders.Total Sales");
    std::fs::write(&visual, stale).unwrap();
    root.join("AcmeOutfitters.pbip")
}

#[tokio::test(flavor = "multi_thread")]
async fn pbip_structure_lint_and_render() {
    let (Some(pbip), Some(exp)) = (fixture(PBIP), expected()) else {
        return;
    };
    let acme = reference_root().join(ACME);
    let reports = exp.0["reports"].as_array().unwrap();
    let pages: usize = reports
        .iter()
        .map(|r| r["pages"].as_array().unwrap().len())
        .sum();
    let relationships = std::fs::read_to_string(
        acme.join("AcmeOutfitters.SemanticModel/definition/relationships.tmdl"),
    )
    .unwrap()
    .lines()
    .filter(|l| l.starts_with("relationship "))
    .count();

    let r = Runner::new();
    let opened = r
        .run("doc.open", json!({"path": pbip, "autoRefresh": false}))
        .await;
    let doc = opened["docId"].as_str().unwrap().to_owned();
    assert_eq!(doc, "d1");
    assert_eq!(opened["format"], "pbip");
    assert_eq!(opened["tables"], exp.row_counts().len());
    assert_eq!(opened["measures"], exp.measures().len());
    assert_eq!(opened["reports"].as_array().unwrap().len(), reports.len());
    assert_eq!(opened["loaded"], false);

    let inspect = r.run("doc.inspect", json!({"docId": doc})).await;
    assert_eq!(inspect["schemaVersion"], 1);
    let counts = &inspect["counts"];
    assert_eq!(
        (
            &counts["tables"],
            &counts["measures"],
            &counts["relationships"],
            &counts["reports"],
            &counts["pages"],
            &counts["visuals"]
        ),
        (
            &json!(exp.row_counts().len()),
            &json!(exp.measures().len()),
            &json!(relationships),
            &json!(reports.len()),
            &json!(pages),
            &json!(visual_files(&acme))
        )
    );
    assert!(inspect.to_string().contains("DISTINCTCOUNTNOBLANK"));
    let outline = r
        .run(
            "doc.inspect",
            json!({"docId": doc, "includeExpressions": false}),
        )
        .await;
    assert!(!outline.to_string().contains("DISTINCTCOUNTNOBLANK"));

    let lint = r.run("doc.lint", json!({"docId": doc})).await;
    assert_eq!(
        (&lint["summary"]["errors"], &lint["summary"]["warnings"]),
        (&json!(0), &json!(0)),
        "{lint}"
    );
    assert_eq!(lint["ok"], true);

    let measures = r.run("model.measures", json!({"docId": doc})).await;
    let total = measure(&measures, "Total Sales");
    assert_eq!(total["table"], "Orders");
    assert_eq!(total["expression"], "SUM(Orders[Line Amount])");
    let rels = r.run("model.relationships", json!({"docId": doc})).await;
    assert_eq!(
        rels["relationships"].as_array().unwrap().len(),
        relationships
    );
    assert_eq!(
        rels["relationships"][0]["crossFilteringBehavior"],
        "bothDirections"
    );
    let tables = r.run("model.tables", json!({"docId": doc})).await;
    assert_eq!(
        tables["tables"].as_array().unwrap().len(),
        exp.row_counts().len()
    );

    let list = r.run("report.list", json!({"docId": doc})).await;
    assert_eq!(list["reports"].as_array().unwrap().len(), reports.len());
    let stores = exp.report("Stores");
    let first = &stores["pages"][0];
    let page_dir = acme.join(format!(
        "{}/definition/pages/{}",
        stores["folder"].as_str().unwrap(),
        first["pageId"].as_str().unwrap()
    ));
    let on_first_page = visual_files(&page_dir);
    let pages = r
        .run("report.pages", json!({"docId": doc, "report": "Stores"}))
        .await;
    assert_eq!(pages["report"], stores["report"]);
    assert_eq!(
        pages["pages"].as_array().unwrap().len(),
        stores["pages"].as_array().unwrap().len()
    );
    assert_eq!(pages["pages"][0]["visuals"], on_first_page);
    let visuals = r
        .run(
            "report.visuals",
            json!({"docId": doc, "report": "Stores", "page": "1"}),
        )
        .await;
    assert_eq!(visuals["visuals"].as_array().unwrap().len(), on_first_page);

    let render = r
        .run("render.page", json!({"docId": doc, "report": "Stores"}))
        .await;
    let svg = render["svg"].as_str().unwrap();
    assert!(svg.starts_with("<svg"));
    assert!(svg.contains("data-hit"), "interactive groups");
    assert_eq!(render["summary"]["visuals"], on_first_page);
    assert_eq!(render["visuals"].as_array().unwrap().len(), on_first_page);
    assert_eq!(render["page"]["displayName"], first["page"]);

    let e = r
        .try_run("report.pages", json!({"docId": doc, "report": "Nope"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::NotFound);
    assert!(e.data.unwrap()["reports"].as_array().unwrap().len() == reports.len());

    let closed = r.run("doc.close", json!({"docId": doc})).await;
    assert_eq!(closed["closed"], true);
    assert_eq!(
        r.try_run("doc.inspect", json!({"docId": doc}))
            .await
            .unwrap_err()
            .code,
        OpErrorCode::NoDocument
    );

    // Lint warnings do not fail by default; `deny` makes them fail.
    let dir = tmp_dir("pbi-ops-lint-");
    let stale = acme_with_stale_query_ref(dir.path());
    let doc = r.open(json!({"path": stale, "autoRefresh": false})).await;
    let lint = r.run("doc.lint", json!({"docId": doc})).await;
    assert_eq!(lint["summary"]["warnings"], 1, "{lint}");
    assert_eq!(lint["summary"]["byRule"]["LINT003"], 1);
    assert_eq!(lint["ok"], true, "warnings do not fail by default");
    let strict = r
        .run("doc.lint", json!({"docId": doc, "deny": "warning"}))
        .await;
    assert_eq!(strict["ok"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn model_edit_commands_publish_revisions_and_history() {
    let dir = tmp_dir("edit");
    let pbip = common::mini_pbip(dir.path());
    let runner = Runner::new();
    let doc = runner
        .open(json!({"path": pbip, "autoRefresh": false}))
        .await;

    let measures = runner.run("model.measures", json!({"docId": doc})).await;
    let measure = measures["measures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|measure| measure["name"] == "Total")
        .unwrap();
    let measure_id = measure["id"].as_u64().unwrap();

    let initial = runner.run("doc.history", json!({"docId": doc})).await;
    assert_eq!(initial["revision"], 0);
    assert_eq!(initial["dirty"], false);
    assert_eq!(initial["canUndo"], false);

    let edited = runner
        .run(
            "model.setMeasureExpression",
            json!({"docId": doc, "measureId": measure_id, "expression": "42"}),
        )
        .await;
    assert_eq!(edited["changed"], true);
    assert_eq!(edited["revision"], 1);
    assert_eq!(edited["dirty"], true);
    assert_eq!(edited["changedObjectIds"], json!([measure_id]));

    let no_op = runner
        .run(
            "model.setMeasureExpression",
            json!({"docId": doc, "measureId": measure_id, "expression": "42"}),
        )
        .await;
    assert_eq!(no_op["changed"], false);
    assert_eq!(no_op["revision"], 1);

    let failed_group = runner
        .try_run(
            "doc.applyEdits",
            json!({
                "docId": doc,
                "edits": [
                    {"type": "setMeasureExpression", "measureId": measure_id, "expression": "99"},
                    {"type": "setDescription", "objectId": u64::MAX, "text": "Missing object"}
                ]
            }),
        )
        .await;
    assert!(failed_group.is_err());
    let after_failure = runner.run("doc.history", json!({"docId": doc})).await;
    assert_eq!(after_failure["revision"], 1);
    let unchanged = runner.run("model.measures", json!({"docId": doc})).await["measures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|measure| measure["id"] == measure_id)
        .unwrap()
        .clone();
    assert_eq!(unchanged["expression"], "42");

    let undone = runner.run("doc.undo", json!({"docId": doc})).await;
    assert_eq!(undone["revision"], 2);
    assert_eq!(undone["dirty"], false);
    assert_eq!(undone["changedObjectIds"], json!([measure_id]));

    let redone = runner.run("doc.redo", json!({"docId": doc})).await;
    assert_eq!(redone["revision"], 3);
    assert_eq!(redone["dirty"], true);
    assert_eq!(redone["changedObjectIds"], json!([measure_id]));

    let query = runner
        .run(
            "dax.query",
            json!({"docId": doc, "query": "EVALUATE ROW(\"n\", [Total])"}),
        )
        .await;
    assert_eq!(query["documentRevision"], 3);
    assert_eq!(query["tables"][0]["rows"][0][0], 42);

    let grouped = runner
        .run(
            "doc.applyEdits",
            json!({
                "docId": doc,
                "edits": [
                    {"type": "setMeasureExpression", "measureId": measure_id, "expression": "84"},
                    {"type": "setDescription", "objectId": measure_id, "text": "Grouped edit"}
                ]
            }),
        )
        .await;
    assert_eq!(grouped["changed"], true);
    assert_eq!(grouped["revision"], 4);

    let group_undo = runner.run("doc.undo", json!({"docId": doc})).await;
    assert_eq!(group_undo["revision"], 5);
    let restored = runner.run("model.measures", json!({"docId": doc})).await["measures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|measure| measure["id"] == measure_id)
        .unwrap()
        .clone();
    assert_eq!(restored["expression"], "42");
    assert!(restored.get("description").is_none());

    let group_redo = runner.run("doc.redo", json!({"docId": doc})).await;
    assert_eq!(group_redo["revision"], 6);
    let query = runner
        .run(
            "dax.query",
            json!({"docId": doc, "query": "EVALUATE ROW(\"n\", [Total])"}),
        )
        .await;
    assert_eq!(query["documentRevision"], 6);
    assert_eq!(query["tables"][0]["rows"][0][0], 84);

    let final_measures = runner.run("model.measures", json!({"docId": doc})).await;
    let final_measure = final_measures["measures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|measure| measure["name"] == "Total")
        .unwrap();
    assert_eq!(final_measure["expression"], "84");
    assert_eq!(final_measure["description"], "Grouped edit");

    // Conversions write the edited document without saving it.
    let pbip_conversion = runner
        .run(
            "convert.pbip",
            json!({"docId": doc, "outDir": dir.path().join("converted-pbip")}),
        )
        .await;
    assert!(
        pbip_conversion["files"].as_array().unwrap().len() > 3,
        "{pbip_conversion}"
    );
    runner
        .run(
            "convert.pbit",
            json!({"docId": doc, "outPath": dir.path().join("converted.pbit")}),
        )
        .await;
    let template = runner
        .open(json!({"path": dir.path().join("converted.pbit"), "autoRefresh": false}))
        .await;
    let converted = runner
        .run("model.measures", json!({"docId": template}))
        .await;
    let total = converted["measures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|measure| measure["name"] == "Total")
        .unwrap();
    assert_eq!(
        (&total["expression"], &total["description"]),
        (&json!("84"), &json!("Grouped edit"))
    );

    let close_without_discard = runner.try_run("doc.close", json!({"docId": doc})).await;
    assert!(close_without_discard.is_err());
    assert_eq!(
        runner.run("doc.history", json!({"docId": doc})).await["dirty"],
        true
    );
    let closed = runner
        .run("doc.close", json!({"docId": doc, "discardChanges": true}))
        .await;
    assert_eq!(closed["closed"], true);
}

/// The single value of `EVALUATE ROW("n", <dax>)`.
async fn scalar(r: &Runner, doc: &str, dax: &str) -> Value {
    let q = r
        .run(
            "dax.query",
            json!({"docId": doc, "query": format!("EVALUATE ROW(\"n\", {dax})")}),
        )
        .await;
    q["tables"][0]["rows"][0][0].clone()
}

fn error_code(result: Result<Value, pbi_ops::OpError>) -> OpErrorCode {
    result.expect_err("the command must fail").code
}

async fn visual_position(r: &Runner, doc: &str, page: &str, visual: &str) -> Value {
    let visuals = r
        .run("report.visuals", json!({"docId": doc, "page": page}))
        .await;
    visuals["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == visual)
        .unwrap_or_else(|| panic!("no visual {visual}"))["position"]
        .clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn rename_describe_and_move_through_the_registry() {
    let dir = tmp_dir("rename");
    let pbip = common::mini_pbip(dir.path());
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    let refresh = r.run("data.refresh", json!({"docId": doc})).await;
    assert_eq!(refresh["ok"], true, "{refresh}");
    let sum: i64 = (1..=60).sum();
    let amount = json!({"docId": doc, "table": "Sales", "column": "Amount", "newName": "Revenue"});

    // A dry run reports the rewrite and changes nothing.
    let mut dry = amount.clone();
    dry["dryRun"] = json!(true);
    let preview = r.run("model.object.rename", dry).await;
    assert_eq!(
        (
            &preview["dryRun"],
            &preview["changed"],
            &preview["revision"],
            &preview["kind"],
            &preview["oldName"]
        ),
        (
            &json!(true),
            &json!(false),
            &json!(0),
            &json!("column"),
            &json!("Amount")
        )
    );
    assert_eq!(
        preview["rewrites"].as_array().unwrap().len(),
        1,
        "{preview}"
    );
    assert_eq!(preview["rewrites"][0]["before"], "SUM(Sales[Amount])");
    assert_eq!(preview["rewrites"][0]["after"], "SUM(Sales[Revenue])");
    let measures = r.run("model.measures", json!({"docId": doc})).await;
    assert_eq!(
        measure(&measures, "Total")["expression"],
        "SUM(Sales[Amount])"
    );
    assert_eq!(
        r.run("doc.history", json!({"docId": doc})).await["dirty"],
        false
    );

    // The rename rewrites the measure and the report fields and keeps the data.
    let renamed = r.run("model.object.rename", amount.clone()).await;
    assert_eq!(
        (&renamed["changed"], &renamed["revision"], &renamed["dirty"]),
        (&json!(true), &json!(1), &json!(true))
    );
    assert_eq!(renamed["rewrites"], preview["rewrites"]);
    let measures = r.run("model.measures", json!({"docId": doc})).await;
    assert_eq!(
        measure(&measures, "Total")["expression"],
        "SUM(Sales[Revenue])"
    );
    assert_eq!(scalar(&r, &doc, "SUM(Sales[Revenue])").await, sum);
    assert_eq!(scalar(&r, &doc, "[Total]").await, sum);
    let tables = r.run("model.tables", json!({"docId": doc})).await;
    let sales = table(&tables["tables"], "Sales");
    assert_eq!(
        (&sales["rows"], &sales["loaded"]),
        (&json!(60), &json!(true))
    );
    let details = r
        .run("report.visuals", json!({"docId": doc, "page": "Details"}))
        .await;
    assert!(details.to_string().contains("\"Revenue\""), "{details}");
    let page = r
        .run("render.page", json!({"docId": doc, "page": "Details"}))
        .await;
    assert_eq!(page["ok"], true, "{}", page["visuals"]);

    // Invalid targets and names fail without a revision.
    let rename = |params: Value| r.try_run("model.object.rename", params);
    let conflict =
        json!({"docId": doc, "table": "Sales", "column": "Revenue", "newName": "Region"});
    assert_eq!(
        error_code(rename(conflict).await),
        OpErrorCode::InvalidParams
    );
    let unknown = json!({"docId": doc, "table": "Sales", "column": "Nope", "newName": "X"});
    assert_eq!(error_code(rename(unknown).await), OpErrorCode::NotFound);
    let both = json!({"docId": doc, "table": "Sales", "column": "Item", "measure": "Total", "newName": "X"});
    assert_eq!(error_code(rename(both).await), OpErrorCode::InvalidParams);
    let none = json!({"docId": doc, "newName": "X"});
    assert_eq!(error_code(rename(none).await), OpErrorCode::InvalidParams);
    let empty = json!({"docId": doc, "table": "Sales", "column": "Item", "newName": ""});
    assert_eq!(error_code(rename(empty).await), OpErrorCode::InvalidParams);
    assert_eq!(
        r.run("doc.history", json!({"docId": doc})).await["revision"],
        1
    );

    // By id: the measure, and the card that shows it.
    let total = measure(&measures, "Total")["id"].as_u64().unwrap();
    let by_id = r
        .run(
            "model.object.rename",
            json!({"docId": doc, "objectId": total, "newName": "Total Sales"}),
        )
        .await;
    assert_eq!(
        (&by_id["kind"], &by_id["revision"]),
        (&json!("measure"), &json!(2))
    );
    assert_eq!(scalar(&r, &doc, "[Total Sales]").await, sum);
    let overview = r
        .run("report.visuals", json!({"docId": doc, "page": "Overview"}))
        .await;
    assert!(
        overview.to_string().contains("\"Total Sales\""),
        "{overview}"
    );

    // Descriptions.
    let describe =
        json!({"docId": doc, "table": "Sales", "column": "Revenue", "text": "Net amount"});
    let described = r.run("model.object.setDescription", describe).await;
    assert_eq!(
        (&described["changed"], &described["revision"]),
        (&json!(true), &json!(3))
    );
    let column = |tables: &Value| {
        table(&tables["tables"], "Sales")["columns"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "Revenue")
            .unwrap()
            .clone()
    };
    let tables = r.run("model.tables", json!({"docId": doc})).await;
    assert_eq!(column(&tables)["description"], "Net amount");
    r.run(
        "model.object.setDescription",
        json!({"docId": doc, "table": "Sales", "column": "Revenue"}),
    )
    .await;
    let tables = r.run("model.tables", json!({"docId": doc})).await;
    assert!(column(&tables).get("description").is_none());

    // Move and resize.
    let start = visual_position(&r, &doc, "Overview", "total").await;
    let moved = r
        .run(
            "report.visual.move",
            json!({"docId": doc, "visual": "total", "x": 20, "y": 70}),
        )
        .await;
    assert_eq!(
        (&moved["changed"], &moved["revision"]),
        (&json!(true), &json!(5))
    );
    assert_eq!(
        (&moved["position"]["x"], &moved["position"]["y"]),
        (&json!(20.0), &json!(70.0))
    );
    let resized = r
        .run(
            "report.visual.resize",
            json!({"docId": doc, "visual": "total", "width": 200, "height": 100}),
        )
        .await;
    assert_eq!(resized["revision"], 6);
    let now = visual_position(&r, &doc, "Overview", "total").await;
    assert_eq!(
        (&now["x"], &now["y"], &now["width"], &now["height"]),
        (&json!(20.0), &json!(70.0), &json!(200.0), &json!(100.0))
    );
    let bad_size = json!({"docId": doc, "visual": "total", "width": 0});
    assert_eq!(
        error_code(r.try_run("report.visual.resize", bad_size).await),
        OpErrorCode::InvalidParams
    );
    let nothing = json!({"docId": doc, "visual": "total"});
    assert_eq!(
        error_code(r.try_run("report.visual.move", nothing).await),
        OpErrorCode::InvalidParams
    );
    let missing = json!({"docId": doc, "visual": "nope", "x": 1});
    assert_eq!(
        error_code(r.try_run("report.visual.move", missing).await),
        OpErrorCode::NotFound
    );
    let rendered = r.run("render.page", json!({"docId": doc})).await;
    assert_eq!(rendered["ok"], true, "{}", rendered["visuals"]);

    // One group, one undo step; a target that does not resolve applies nothing.
    let failing = json!({"docId": doc, "edits": [
        {"type": "renameObject", "table": "Sales", "newName": "Orders"},
        {"type": "setVisualPosition", "visual": "nope", "x": 1}
    ]});
    assert_eq!(
        error_code(r.try_run("doc.applyEdits", failing).await),
        OpErrorCode::NotFound
    );
    let grouped = r
        .run(
            "doc.applyEdits",
            json!({"docId": doc, "edits": [
                {"type": "renameObject", "table": "Sales", "newName": "Orders"},
                {"type": "setVisualPosition", "visual": "bars", "x": 12, "height": 180}
            ]}),
        )
        .await;
    assert_eq!(
        (&grouped["changed"], &grouped["revision"]),
        (&json!(true), &json!(7))
    );
    assert_eq!(scalar(&r, &doc, "SUM(Orders[Revenue])").await, sum);
    let measures = r.run("model.measures", json!({"docId": doc})).await;
    assert_eq!(
        measure(&measures, "Total Sales")["expression"],
        "SUM(Orders[Revenue])"
    );
    let history = r.run("doc.history", json!({"docId": doc})).await;
    assert_eq!(history["undoLabel"], "Apply document edits");

    // Undo everything: the original document, data and positions.
    for _ in 0..7 {
        r.run("doc.undo", json!({"docId": doc})).await;
    }
    let history = r.run("doc.history", json!({"docId": doc})).await;
    assert_eq!(
        (&history["revision"], &history["dirty"]),
        (&json!(14), &json!(false))
    );
    let measures = r.run("model.measures", json!({"docId": doc})).await;
    assert_eq!(
        measure(&measures, "Total")["expression"],
        "SUM(Sales[Amount])"
    );
    assert_eq!(scalar(&r, &doc, "[Total]").await, sum);
    assert_eq!(visual_position(&r, &doc, "Overview", "total").await, start);
}

#[tokio::test(flavor = "multi_thread")]
async fn measure_create_update_delete_through_the_registry() {
    let dir = tmp_dir("measures");
    let pbip = common::mini_pbip(dir.path());
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    let refresh = r.run("data.refresh", json!({"docId": doc})).await;
    assert_eq!(refresh["ok"], true, "{refresh}");
    let sum: i64 = (1..=60).sum();
    let original = r.run("model.measures", json!({"docId": doc})).await;
    let total = measure(&original, "Total")["id"].as_u64().unwrap();
    let number = |v: Value| v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"));

    // Dependents of a column (by name and by id) and of a table.
    let amount = r
        .run(
            "model.dependencies",
            json!({"docId": doc, "table": "Sales", "column": "Amount"}),
        )
        .await;
    assert_eq!(
        (
            &amount["kind"],
            &amount["table"],
            &amount["name"],
            &amount["revision"]
        ),
        (
            &json!("column"),
            &json!("Sales"),
            &json!("Amount"),
            &json!(0)
        )
    );
    let pairs = |out: &Value| -> Vec<(String, String)> {
        out["dependents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| {
                (
                    d["location"].as_str().unwrap().to_owned(),
                    d["property"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    };
    let found = pairs(&amount);
    for (location, property) in [
        ("measure 'Sales'[Total]", "expression"),
        (
            "visual 'bars' (clusteredBarChart) on page 'Overview'",
            "fields",
        ),
        ("visual 'table' (tableEx) on page 'Details'", "fields"),
    ] {
        assert!(
            found
                .iter()
                .any(|(l, p)| l.starts_with(location) && p == property),
            "{location}: {found:?}"
        );
    }
    let by_id = r
        .run(
            "model.dependencies",
            json!({"docId": doc, "objectId": amount["objectId"]}),
        )
        .await;
    assert_eq!(by_id["dependents"], amount["dependents"]);
    let sales = r
        .run(
            "model.dependencies",
            json!({"docId": doc, "table": "Sales"}),
        )
        .await;
    assert_eq!(sales["kind"], "table");
    assert!(
        pairs(&sales).contains(&("measure 'Sales'[Total]".to_owned(), "expression".to_owned())),
        "{sales}"
    );
    let dependencies = |params: Value| r.try_run("model.dependencies", params);
    let missing = json!({"docId": doc, "table": "Sales", "column": "Nope"});
    assert_eq!(
        error_code(dependencies(missing).await),
        OpErrorCode::NotFound
    );
    let nothing = json!({"docId": doc});
    assert_eq!(
        error_code(dependencies(nothing).await),
        OpErrorCode::InvalidParams
    );

    // Create: a new id, a revision, and a measure that evaluates.
    let twice = r
        .run(
            "model.measure.create",
            json!({"docId": doc, "table": "Sales", "name": "Twice", "expression": "[Total] * 2", "formatString": "#,0"}),
        )
        .await;
    assert_eq!(
        (&twice["changed"], &twice["revision"], &twice["dirty"]),
        (&json!(true), &json!(1), &json!(true))
    );
    assert_eq!(
        (&twice["table"], &twice["name"]),
        (&json!("Sales"), &json!("Twice"))
    );
    assert_eq!(twice["warnings"], json!([]));
    let twice_id = twice["measureId"].as_u64().unwrap();
    assert_ne!(twice_id, total);
    assert_eq!(twice["changedObjectIds"], json!([twice_id]));
    assert_eq!(number(scalar(&r, &doc, "[Twice]").await), (2 * sum) as f64);
    let measures = r.run("model.measures", json!({"docId": doc})).await;
    assert_eq!(measure(&measures, "Twice")["formatString"], "#,0");

    // Invalid creates change nothing.
    let create = |params: Value| r.try_run("model.measure.create", params);
    for (name, table, expression, code) in [
        ("total", "Sales", "1", OpErrorCode::InvalidParams),
        ("Amount", "Sales", "1", OpErrorCode::InvalidParams),
        ("New", "Nope", "1", OpErrorCode::NotFound),
        ("New", "Sales", " ", OpErrorCode::InvalidParams),
    ] {
        let params = json!({"docId": doc, "table": table, "name": name, "expression": expression});
        assert_eq!(error_code(create(params).await), code, "{name} in {table}");
    }
    assert_eq!(
        r.run("doc.history", json!({"docId": doc})).await["revision"],
        1
    );

    // DAX that does not parse is kept, with a warning; delete it by id.
    let broken = r
        .run(
            "model.measure.create",
            json!({"docId": doc, "table": "Sales", "name": "Broken", "expression": "SUM("}),
        )
        .await;
    assert_eq!(broken["revision"], 2);
    assert_eq!(broken["warnings"].as_array().unwrap().len(), 1, "{broken}");
    let broken_id = broken["measureId"].as_u64().unwrap();
    let deleted = r
        .run(
            "model.measure.delete",
            json!({"docId": doc, "measureId": broken_id}),
        )
        .await;
    assert_eq!(
        (
            &deleted["revision"],
            &deleted["name"],
            &deleted["dependents"]
        ),
        (&json!(3), &json!("Broken"), &json!([]))
    );

    // Update patches only the given properties; an empty text clears one.
    let updated = r
        .run(
            "model.measure.update",
            json!({"docId": doc, "table": "Sales", "measure": "Twice", "formatString": "0.00", "description": "Two totals", "isHidden": true}),
        )
        .await;
    assert_eq!(updated["revision"], 4);
    let measures = r.run("model.measures", json!({"docId": doc})).await;
    let m = measure(&measures, "Twice");
    assert_eq!(
        (
            &m["expression"],
            &m["formatString"],
            &m["description"],
            &m["isHidden"]
        ),
        (
            &json!("[Total] * 2"),
            &json!("0.00"),
            &json!("Two totals"),
            &json!(true)
        )
    );
    let cleared = r
        .run(
            "model.measure.update",
            json!({"docId": doc, "measureId": twice_id, "formatString": ""}),
        )
        .await;
    assert_eq!(cleared["revision"], 5);
    let measures = r.run("model.measures", json!({"docId": doc})).await;
    assert!(measure(&measures, "Twice").get("formatString").is_none());
    let update = |params: Value| r.try_run("model.measure.update", params);
    let nothing = json!({"docId": doc, "measureId": twice_id});
    assert_eq!(
        error_code(update(nothing).await),
        OpErrorCode::InvalidParams
    );
    let both = json!({"docId": doc, "measureId": twice_id, "table": "Sales", "measure": "Twice", "isHidden": false});
    assert_eq!(error_code(update(both).await), OpErrorCode::InvalidParams);
    let gone = json!({"docId": doc, "measureId": broken_id, "isHidden": false});
    assert_eq!(error_code(update(gone).await), OpErrorCode::NotFound);

    // A dry run lists what uses the measure and changes nothing.
    let dry = r
        .run(
            "model.measure.delete",
            json!({"docId": doc, "table": "Sales", "measure": "Total", "dryRun": true}),
        )
        .await;
    assert_eq!(
        (&dry["dryRun"], &dry["changed"], &dry["revision"]),
        (&json!(true), &json!(false), &json!(5))
    );
    let dependents: Vec<(&str, &str)> = dry["dependents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["location"].as_str().unwrap(),
                d["property"].as_str().unwrap(),
            )
        })
        .collect();
    assert!(
        dependents.contains(&("measure 'Sales'[Twice]", "expression")),
        "{dependents:?}"
    );
    assert!(
        dependents.iter().any(|(location, property)| location
            .starts_with("visual 'total' (cardVisual) on page 'Overview'")
            && *property == "fields"),
        "{dependents:?}"
    );
    assert_eq!(scalar(&r, &doc, "[Total]").await, sum);

    // Delete proceeds despite the dependents; undo restores it with its id.
    let deleted = r
        .run(
            "model.measure.delete",
            json!({"docId": doc, "table": "Sales", "measure": "Total"}),
        )
        .await;
    assert_eq!(
        (
            &deleted["changed"],
            &deleted["revision"],
            &deleted["dryRun"]
        ),
        (&json!(true), &json!(6), &json!(false))
    );
    assert_eq!(deleted["dependents"], dry["dependents"]);
    let measures = r.run("model.measures", json!({"docId": doc})).await;
    assert!(!measures.to_string().contains("\"Total\""), "{measures}");
    let delete = |params: Value| r.try_run("model.measure.delete", params);
    let unknown = json!({"docId": doc, "measureId": 999_999});
    assert_eq!(error_code(delete(unknown).await), OpErrorCode::NotFound);
    let region = r
        .run(
            "model.object.rename",
            json!({"docId": doc, "table": "Sales", "column": "Region", "newName": "Area", "dryRun": true}),
        )
        .await["objectId"]
        .clone();
    let column = json!({"docId": doc, "measureId": region});
    assert_eq!(error_code(delete(column).await), OpErrorCode::InvalidParams);
    r.run("doc.undo", json!({"docId": doc})).await;
    let measures = r.run("model.measures", json!({"docId": doc})).await;
    assert_eq!(measure(&measures, "Total")["id"], total);
    assert_eq!(scalar(&r, &doc, "[Total]").await, sum);

    // In a group, with a fresh id; a failing edit applies nothing.
    let failing = json!({"docId": doc, "edits": [
        {"type": "createMeasure", "table": "Sales", "name": "Half", "expression": "[Total] / 2"},
        {"type": "deleteMeasure", "table": "Sales", "measure": "Nope"}
    ]});
    assert_eq!(
        error_code(r.try_run("doc.applyEdits", failing).await),
        OpErrorCode::NotFound
    );
    let grouped = r
        .run(
            "doc.applyEdits",
            json!({"docId": doc, "edits": [
                {"type": "createMeasure", "table": "Sales", "name": "Half", "expression": "[Total] / 2"},
                {"type": "updateMeasure", "measureId": twice_id, "isHidden": false},
                {"type": "deleteMeasure", "measureId": total}
            ]}),
        )
        .await;
    assert_eq!(
        (&grouped["changed"], &grouped["revision"]),
        (&json!(true), &json!(8))
    );
    let measures = r.run("model.measures", json!({"docId": doc})).await;
    let half = measure(&measures, "Half")["id"].as_u64().unwrap();
    assert!(![total, twice_id, broken_id].contains(&half), "{half}");
    assert_eq!(measure(&measures, "Twice")["isHidden"], false);

    // Undo everything: the original measures.
    for _ in 0..6 {
        r.run("doc.undo", json!({"docId": doc})).await;
    }
    let history = r.run("doc.history", json!({"docId": doc})).await;
    assert_eq!(
        (&history["revision"], &history["dirty"], &history["canUndo"]),
        (&json!(14), &json!(false), &json!(false))
    );
    let measures = r.run("model.measures", json!({"docId": doc})).await;
    assert_eq!(measures["measures"], original["measures"]);
    assert_eq!(scalar(&r, &doc, "[Total]").await, sum);
}

#[tokio::test(flavor = "multi_thread")]
async fn renaming_line_amount_keeps_the_expected_values() {
    let (Some(pbip), Some(map), Some(exp)) = (fixture(PBIP), fixture(BLOB_MAP), expected()) else {
        return;
    };
    let r = Runner::new();
    let doc = r
        .open(json!({"path": pbip, "blobMap": map, "source": "query", "autoRefresh": false}))
        .await;
    let refresh = r.run("data.refresh", json!({"docId": doc})).await;
    assert_eq!(refresh["ok"], true, "{refresh}");

    let renamed = r
        .run(
            "model.object.rename",
            json!({"docId": doc, "table": "Orders", "column": "Line Amount", "newName": "Line Total"}),
        )
        .await;
    assert_eq!(renamed["changed"], true);
    let locations: Vec<&str> = renamed["rewrites"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["location"].as_str().unwrap())
        .collect();
    for object in ["Total Sales", "Lifetime Sales", "Order Size"] {
        assert!(
            locations.iter().any(|l| l.contains(object)),
            "{object} not rewritten: {locations:?}"
        );
    }
    // No DAX still names the old column.
    let measures = r.run("model.measures", json!({"docId": doc})).await;
    let tables = r.run("model.tables", json!({"docId": doc})).await;
    for text in [measures.to_string(), tables["tables"].to_string()] {
        assert!(!text.contains("[Line Amount]"), "a DAX reference was left");
    }

    // The loaded data follows the rename, and a refresh reloads it under the new name.
    let check = |all: &Value| {
        assert_eq!(all["ok"], true);
        for (name, _, formatted) in exp.measures() {
            assert_eq!(measure(all, name)["formatted"], formatted, "{name}");
        }
    };
    check(&r.run("measures.values", json!({"docId": doc})).await);
    let refresh = r.run("data.refresh", json!({"docId": doc})).await;
    assert_eq!(refresh["ok"], true, "{refresh}");
    for (name, rows) in exp.row_counts() {
        assert_eq!(table(&refresh["tables"], name)["rows"], rows, "{name}");
    }
    check(&r.run("measures.values", json!({"docId": doc})).await);
    let stores = r
        .run("measures.values", json!({"docId": doc, "report": "Stores"}))
        .await;
    for (name, _, formatted) in exp.report_measures("Stores") {
        assert_eq!(measure(&stores, name)["display"], formatted, "{name}");
    }
    let render = r
        .run("render.page", json!({"docId": doc, "report": "Stores"}))
        .await;
    assert_eq!(render["ok"], true, "{}", render["visuals"]);
    let card = exp.card("Stores", "Store Performance", "kpis", "Total Sales");
    assert!(
        render["svg"]
            .as_str()
            .unwrap()
            .contains(&format!(">{card}<")),
        "no {card} on the page"
    );
}

/// The relationship of `out` (`model.relationships`) between `from` and `to`
/// (`Table[Column]`).
fn relationship<'a>(out: &'a Value, from: (&str, &str), to: (&str, &str)) -> &'a Value {
    out["relationships"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| {
            (
                &r["from"]["table"],
                &r["from"]["column"],
                &r["to"]["table"],
                &r["to"]["column"],
            ) == (&json!(from.0), &json!(from.1), &json!(to.0), &json!(to.1))
        })
        .unwrap_or_else(|| panic!("no relationship {from:?} → {to:?}: {out}"))
}

/// Relationship create and delete on the example: filters follow at once,
/// undo restores them, and USERELATIONSHIP users are the dependents.
#[tokio::test(flavor = "multi_thread")]
async fn relationship_create_and_delete_through_the_registry() {
    let (Some(pbip), Some(map)) = (fixture(PBIP), fixture(BLOB_MAP)) else {
        eprintln!("SKIP: {PBIP} missing");
        return;
    };
    let r = Runner::new();
    let doc = r
        .open(json!({"path": pbip, "blobMap": map, "source": "query", "autoRefresh": false}))
        .await;
    let refresh = r.run("data.refresh", json!({"docId": doc})).await;
    assert_eq!(refresh["ok"], true, "{refresh}");
    let number = |v: Value| v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"));
    let total = number(scalar(&r, &doc, "[Total Sales]").await);
    // The smallest region's sales: below the total while Stores filters Orders.
    let smallest_region = "MINX(VALUES(Stores[Region]), CALCULATE([Total Sales]))";
    let filtered = number(scalar(&r, &doc, smallest_region).await);
    assert!(filtered < total, "{filtered} vs {total}");

    let all = r.run("model.relationships", json!({"docId": doc})).await;
    assert_eq!(all["relationships"].as_array().unwrap().len(), 8, "{all}");
    let stores = relationship(&all, ("Orders", "Store ID"), ("Stores", "Store ID")).clone();
    let stores_id = stores["relationshipId"].as_u64().unwrap();
    let shipped = relationship(&all, ("Orders", "Ship Date"), ("Date", "Date"));
    let shipped_id = shipped["relationshipId"].as_u64().unwrap();
    assert_eq!(shipped["isActive"], false);

    // The inactive relationship is used by USERELATIONSHIP: its dependent.
    let dry = r
        .run(
            "model.relationship.delete",
            json!({"docId": doc, "relationshipId": shipped_id, "dryRun": true}),
        )
        .await;
    assert_eq!(
        (&dry["changed"], &dry["revision"], &dry["dryRun"]),
        (&json!(false), &json!(0), &json!(true))
    );
    assert_eq!(
        dry["dependents"],
        json!([{"location": "measure 'Orders'[Shipped Sales]", "property": "expression"}])
    );
    let uses = r
        .run(
            "model.dependencies",
            json!({"docId": doc, "objectId": shipped_id}),
        )
        .await;
    assert_eq!(
        (&uses["kind"], &uses["table"], &uses["dependents"]),
        (&json!("relationship"), &json!("Orders"), &dry["dependents"])
    );

    // Delete by name: Stores no longer filters Orders; undo restores it.
    let deleted = r
        .run(
            "model.relationship.delete",
            json!({"docId": doc, "name": stores["name"].as_str().unwrap().to_uppercase()}),
        )
        .await;
    assert_eq!(
        (
            &deleted["changed"],
            &deleted["revision"],
            &deleted["relationshipId"],
            &deleted["dependents"]
        ),
        (&json!(true), &json!(1), &json!(stores_id), &json!([]))
    );
    assert_eq!(
        (&deleted["from"], &deleted["to"]),
        (&stores["from"], &stores["to"])
    );
    assert_eq!(number(scalar(&r, &doc, smallest_region).await), total);
    let after = r.run("model.relationships", json!({"docId": doc})).await;
    assert_eq!(after["relationships"].as_array().unwrap().len(), 7);
    r.run("doc.undo", json!({"docId": doc})).await;
    assert_eq!(number(scalar(&r, &doc, smallest_region).await), filtered);
    let restored = r.run("model.relationships", json!({"docId": doc})).await;
    assert_eq!(restored, all, "undo restores it in place with its id");

    // Delete, then create it again (names as typed, case-insensitive): a
    // new id and a GUID name; the filters follow at once.
    r.run(
        "model.relationship.delete",
        json!({"docId": doc, "relationshipId": stores_id}),
    )
    .await;
    let created = r
        .run(
            "model.relationship.create",
            json!({"docId": doc, "from": {"table": "orders", "column": "store id"}, "to": {"table": "STORES", "column": "Store ID"}}),
        )
        .await;
    assert_eq!(
        (
            &created["changed"],
            &created["revision"],
            &created["warnings"]
        ),
        (&json!(true), &json!(4), &json!([]))
    );
    assert_eq!(
        (&created["from"], &created["to"]),
        (&stores["from"], &stores["to"])
    );
    let created_id = created["relationshipId"].as_u64().unwrap();
    assert_ne!(created_id, stores_id);
    assert_eq!(created["changedObjectIds"], json!([created_id]));
    let name = created["name"].as_str().unwrap();
    assert_eq!(
        name.split('-').map(str::len).collect::<Vec<_>>(),
        [8, 4, 4, 4, 12],
        "{name}"
    );
    assert_eq!(number(scalar(&r, &doc, smallest_region).await), filtered);
    let now = r.run("model.relationships", json!({"docId": doc})).await;
    let new = relationship(&now, ("Orders", "Store ID"), ("Stores", "Store ID"));
    assert_eq!(
        (
            &new["relationshipId"],
            &new["fromCardinality"],
            &new["toCardinality"],
            &new["crossFilteringBehavior"],
            &new["isActive"]
        ),
        (
            &json!(created_id),
            &json!("many"),
            &json!("one"),
            &json!("oneDirection"),
            &json!(true)
        )
    );

    // Invalid relationships change nothing.
    let create = |params: Value| r.try_run("model.relationship.create", params);
    let end = |t: &str, c: &str| json!({"table": t, "column": c});
    for (from, to, active, why) in [
        (
            end("Orders", "Order Date"),
            end("Date", "Date"),
            true,
            "same columns",
        ),
        (
            end("Date", "Date"),
            end("Orders", "Ship Date"),
            false,
            "same columns, reversed",
        ),
        (
            end("Orders", "Due Date"),
            end("Date", "Date"),
            false,
            "no such column",
        ),
        (
            end("Orders", "Order ID"),
            end("Orders", "Store ID"),
            false,
            "one table",
        ),
        (
            end("Orders", "Customer ID"),
            end("Stores", "Store Name"),
            true,
            "second active relationship between two tables",
        ),
    ] {
        let params = json!({"docId": doc, "from": from, "to": to, "isActive": active});
        assert_eq!(
            error_code(create(params).await),
            OpErrorCode::InvalidParams,
            "{why}"
        );
    }
    let taken = json!({"docId": doc, "from": end("Customers", "State"), "to": end("Stores", "State"), "isActive": false, "name": name.to_uppercase()});
    assert_eq!(error_code(create(taken).await), OpErrorCode::InvalidParams);
    let delete = |params: Value| r.try_run("model.relationship.delete", params);
    let unknown = json!({"docId": doc, "relationshipId": 999_999});
    assert_eq!(error_code(delete(unknown).await), OpErrorCode::NotFound);
    let not_one = json!({"docId": doc, "relationshipId": created_id, "name": name});
    assert_eq!(
        error_code(delete(not_one).await),
        OpErrorCode::InvalidParams
    );
    let orders = r
        .run(
            "model.dependencies",
            json!({"docId": doc, "table": "Orders"}),
        )
        .await["objectId"]
        .clone();
    let a_table = json!({"docId": doc, "relationshipId": orders});
    assert_eq!(
        error_code(delete(a_table).await),
        OpErrorCode::InvalidParams
    );
    assert_eq!(
        r.run("doc.history", json!({"docId": doc})).await["revision"],
        4
    );

    // Several stores share a state: the engine reports duplicate keys on
    // the one side as a warning, and the relationship is still created.
    let states = r
        .run(
            "model.relationship.create",
            json!({"docId": doc, "from": end("Customers", "State"), "to": end("Stores", "State"), "isActive": false}),
        )
        .await;
    assert_eq!(states["changed"], true);
    let warnings = states["warnings"].to_string();
    assert!(warnings.contains("ENG002"), "{states}");

    // In a group: one undo step; a failing edit applies nothing.
    let failing = json!({"docId": doc, "edits": [
        {"type": "deleteRelationship", "relationshipId": created_id},
        {"type": "deleteRelationship", "name": "nope"}
    ]});
    assert_eq!(
        error_code(r.try_run("doc.applyEdits", failing).await),
        OpErrorCode::NotFound
    );
    let grouped = r
        .run(
            "doc.applyEdits",
            json!({"docId": doc, "edits": [
                {"type": "deleteRelationship", "relationshipId": created_id},
                {"type": "createRelationship", "from": end("Orders", "Store ID"), "to": end("Stores", "Store ID"), "crossFilteringBehavior": "bothDirections"}
            ]}),
        )
        .await;
    assert_eq!(
        (&grouped["changed"], &grouped["revision"]),
        (&json!(true), &json!(6))
    );
    let now = r.run("model.relationships", json!({"docId": doc})).await;
    let both = relationship(&now, ("Orders", "Store ID"), ("Stores", "Store ID"));
    assert_eq!(both["crossFilteringBehavior"], "bothDirections");
    assert_eq!(number(scalar(&r, &doc, smallest_region).await), filtered);

    // Undo everything: the original relationships and values.
    for _ in 0..4 {
        r.run("doc.undo", json!({"docId": doc})).await;
    }
    let history = r.run("doc.history", json!({"docId": doc})).await;
    assert_eq!(history["dirty"], false);
    assert_eq!(
        r.run("model.relationships", json!({"docId": doc})).await,
        all
    );
    assert_eq!(number(scalar(&r, &doc, smallest_region).await), filtered);
}

/// Relationship update on the example: cross-filter direction, the active
/// relationship and cardinality change what the engine computes at once.
#[tokio::test(flavor = "multi_thread")]
async fn relationship_update_through_the_registry() {
    let (Some(pbip), Some(map)) = (fixture(PBIP), fixture(BLOB_MAP)) else {
        eprintln!("SKIP: {PBIP} missing");
        return;
    };
    let r = Runner::new();
    let doc = r
        .open(json!({"path": pbip, "blobMap": map, "source": "query", "autoRefresh": false}))
        .await;
    let refresh = r.run("data.refresh", json!({"docId": doc})).await;
    assert_eq!(refresh["ok"], true, "{refresh}");
    let number = |v: Value| v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"));
    let all = r.run("model.relationships", json!({"docId": doc})).await;
    let customers = relationship(
        &all,
        ("Orders", "Customer ID"),
        ("Customers", "Customer ID"),
    );
    let customers_id = customers["relationshipId"].as_u64().unwrap();
    assert_eq!(customers["crossFilteringBehavior"], "bothDirections");
    let order_date = relationship(&all, ("Orders", "Order Date"), ("Date", "Date"));
    let order_date_name = order_date["name"].as_str().unwrap().to_owned();
    let ship_date = relationship(&all, ("Orders", "Ship Date"), ("Date", "Date"));
    let ship_date_id = ship_date["relationshipId"].as_u64().unwrap();
    let stores = relationship(&all, ("Orders", "Store ID"), ("Stores", "Store ID"));
    let stores_id = stores["relationshipId"].as_u64().unwrap();

    // Orders filters Customers both ways: a product's buyers are fewer than
    // all customers. One direction: every product sees every customer.
    let customers_all = number(scalar(&r, &doc, "COUNTROWS(Customers)").await);
    let fewest_buyers = "MINX(VALUES(Products[Product ID]), CALCULATE(COUNTROWS(Customers)))";
    let buyers = number(scalar(&r, &doc, fewest_buyers).await);
    assert!(buyers < customers_all, "{buyers} vs {customers_all}");
    let one_way = r
        .run(
            "model.relationship.update",
            json!({"docId": doc, "relationshipId": customers_id, "crossFilteringBehavior": "oneDirection"}),
        )
        .await;
    assert_eq!(
        (
            &one_way["changed"],
            &one_way["revision"],
            &one_way["changedObjectIds"],
            &one_way["crossFilteringBehavior"],
            &one_way["warnings"]
        ),
        (
            &json!(true),
            &json!(1),
            &json!([customers_id]),
            &json!("oneDirection"),
            &json!([])
        )
    );
    assert_eq!(number(scalar(&r, &doc, fewest_buyers).await), customers_all);
    r.run("doc.undo", json!({"docId": doc})).await;
    assert_eq!(number(scalar(&r, &doc, fewest_buyers).await), buyers);

    // Ship Date cannot be active next to Order Date; swapped in one group,
    // dates filter Orders by ship date, as USERELATIONSHIP did.
    let by_date = |m: &str| format!("SUMX(VALUES('Date'[Date]), CALCULATE({m}))");
    let ordered = number(scalar(&r, &doc, &by_date("[Total Sales]")).await);
    let shipped = number(scalar(&r, &doc, &by_date("[Shipped Sales]")).await);
    assert!(shipped < ordered, "{shipped} vs {ordered}");
    let activate = json!({"docId": doc, "relationshipId": ship_date_id, "isActive": true});
    assert_eq!(
        error_code(r.try_run("model.relationship.update", activate).await),
        OpErrorCode::InvalidParams
    );
    let swapped = r
        .run(
            "doc.applyEdits",
            json!({"docId": doc, "edits": [
                {"type": "updateRelationship", "name": order_date_name, "isActive": false},
                {"type": "updateRelationship", "relationshipId": ship_date_id, "isActive": true}
            ]}),
        )
        .await;
    assert_eq!(
        (&swapped["changed"], &swapped["revision"]),
        (&json!(true), &json!(3))
    );
    assert_eq!(
        number(scalar(&r, &doc, &by_date("[Total Sales]")).await),
        shipped
    );

    // Many-to-many: the same values here (Store ID is unique in Stores).
    let smallest_region = "MINX(VALUES(Stores[Region]), CALCULATE([Total Sales]))";
    let filtered = number(scalar(&r, &doc, smallest_region).await);
    let many = r
        .run(
            "model.relationship.update",
            json!({"docId": doc, "relationshipId": stores_id, "toCardinality": "many"}),
        )
        .await;
    assert_eq!(
        (
            &many["revision"],
            &many["fromCardinality"],
            &many["toCardinality"]
        ),
        (&json!(4), &json!("many"), &json!("many"))
    );
    assert_eq!(number(scalar(&r, &doc, smallest_region).await), filtered);

    // Invalid updates change nothing.
    let update = |params: Value| r.try_run("model.relationship.update", params);
    let security = json!({"docId": doc, "relationshipId": stores_id, "securityFilteringBehavior": "bothDirections"});
    assert_eq!(
        error_code(update(security).await),
        OpErrorCode::InvalidParams
    );
    let nothing = json!({"docId": doc, "relationshipId": stores_id});
    assert_eq!(
        error_code(update(nothing).await),
        OpErrorCode::InvalidParams
    );
    let unknown = json!({"docId": doc, "name": "nope", "isActive": false});
    assert_eq!(error_code(update(unknown).await), OpErrorCode::NotFound);
    let wrong =
        json!({"docId": doc, "relationshipId": stores_id, "crossFilteringBehavior": "automatic"});
    assert_eq!(error_code(update(wrong).await), OpErrorCode::InvalidParams);
    assert_eq!(
        r.run("doc.history", json!({"docId": doc})).await["revision"],
        4
    );

    // Both ways, with security filters both ways too, in one edit.
    let secure = r
        .run(
            "model.relationship.update",
            json!({"docId": doc, "relationshipId": stores_id, "crossFilteringBehavior": "bothDirections", "securityFilteringBehavior": "bothDirections"}),
        )
        .await;
    assert_eq!(
        (
            &secure["crossFilteringBehavior"],
            &secure["securityFilteringBehavior"]
        ),
        (&json!("bothDirections"), &json!("bothDirections"))
    );

    // Undo everything: the original relationships and values.
    for _ in 0..3 {
        r.run("doc.undo", json!({"docId": doc})).await;
    }
    let history = r.run("doc.history", json!({"docId": doc})).await;
    assert_eq!(history["dirty"], false);
    assert_eq!(
        r.run("model.relationships", json!({"docId": doc})).await,
        all
    );
    assert_eq!(
        number(scalar(&r, &doc, &by_date("[Total Sales]")).await),
        ordered
    );
    assert_eq!(number(scalar(&r, &doc, fewest_buyers).await), buyers);
}

/// Acceptance for `doc.saveAs`: the example with a column rename, saved as a new
/// project, reopened in a fresh workspace and refreshed through the blob map, gives
/// every expected value.
#[tokio::test(flavor = "multi_thread")]
async fn save_as_project_keeps_the_expected_values() {
    let (Some(pbip), Some(map), Some(exp)) = (fixture(PBIP), fixture(BLOB_MAP), expected()) else {
        return;
    };
    let r = Runner::new();
    let doc = r
        .open(json!({"path": pbip, "blobMap": map, "source": "query", "autoRefresh": false}))
        .await;
    r.run(
        "model.object.rename",
        json!({"docId": doc, "table": "Orders", "column": "Line Amount", "newName": "Line Total"}),
    )
    .await;
    let dir = tmp_dir("pbi-ops-save-as-");
    let out = r
        .run("doc.saveAs", json!({"docId": doc, "outDir": dir.path()}))
        .await;
    assert_eq!(
        (&out["previousFormat"], &out["warnings"]),
        (&json!("pbip"), &json!([])),
        "{out}"
    );

    let fresh = Runner::new();
    let saved = fresh
        .open(json!({
            "path": dir.path().join(out["pbipFile"].as_str().unwrap()),
            "blobMap": map,
            "source": "query",
            "autoRefresh": false
        }))
        .await;
    let measures = fresh.run("model.measures", json!({"docId": saved})).await;
    let text = measures.to_string();
    assert!(
        text.contains("[Line Total]") && !text.contains("[Line Amount]"),
        "the rename is saved"
    );
    let refresh = fresh.run("data.refresh", json!({"docId": saved})).await;
    assert_eq!(refresh["ok"], true, "{refresh}");
    for (name, rows) in exp.row_counts() {
        assert_eq!(table(&refresh["tables"], name)["rows"], rows, "{name}");
    }
    let all = fresh.run("measures.values", json!({"docId": saved})).await;
    assert_eq!(all["ok"], true);
    for (name, _, formatted) in exp.measures() {
        assert_eq!(measure(&all, name)["formatted"], formatted, "{name}");
    }
    let render = fresh
        .run("render.page", json!({"docId": saved, "report": "Stores"}))
        .await;
    assert_eq!(render["ok"], true, "{}", render["visuals"]);
    let card = exp.card("Stores", "Store Performance", "kpis", "Total Sales");
    assert!(
        render["svg"]
            .as_str()
            .unwrap()
            .contains(&format!(">{card}<")),
        "no {card} on the page"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn expected_values_through_the_registry() {
    let (Some(pbip), Some(map), Some(csv), Some(exp)) = (
        fixture(PBIP),
        fixture(BLOB_MAP),
        fixture(ORDERS_CSV),
        expected(),
    ) else {
        return;
    };
    let r = Runner::new();
    let doc = r
        .open(json!({"path": pbip, "blobMap": map, "source": "query", "autoRefresh": false}))
        .await;
    let refresh = r.run("data.refresh", json!({"docId": doc})).await;
    assert_eq!(refresh["ok"], true, "{refresh}");
    for (name, rows) in exp.row_counts() {
        assert_eq!(table(&refresh["tables"], name)["rows"], rows, "{name}");
    }

    let all = r.run("measures.values", json!({"docId": doc})).await;
    assert_eq!(all["ok"], true);
    assert_eq!(all["documentRevision"], 0);
    for (name, _, formatted) in exp.measures() {
        assert_eq!(measure(&all, name)["formatted"], formatted, "{name}");
    }
    let stores = r
        .run("measures.values", json!({"docId": doc, "report": "Stores"}))
        .await;
    for (name, _, formatted) in exp.report_measures("Stores") {
        assert_eq!(measure(&stores, name)["display"], formatted, "{name}");
    }

    let q = r
        .run(
            "dax.query",
            json!({"docId": doc, "query": "EVALUATE ROW(\"n\", [Order Count])"}),
        )
        .await;
    assert_eq!(
        q["tables"][0]["rows"][0][0],
        exp.0["measures"]["unfiltered"]["Order Count"]["value"]
    );
    assert_eq!(q["documentRevision"], 0);
    let one = r
        .run(
            "dax.evaluateMeasure",
            json!({"docId": doc, "measure": "[Total Sales]", "report": "Online"}),
        )
        .await;
    assert_eq!(
        one["measure"]["formatted"],
        exp.measure(Some("Online"), "Total Sales")
    );
    assert_eq!(one["documentRevision"], 0);

    // The West stores: the rows the "Region West" role sees.
    let west = exp.0["rls"][0]["visibleRows"]["Stores"].clone();
    assert_eq!(exp.0["rls"][0]["roles"], json!(["Region West"]));
    let page = r
        .run(
            "data.page",
            json!({"docId": doc, "table": "Stores", "limit": 2, "filter": "\"Region\" = 'West'"}),
        )
        .await;
    assert_eq!(page["totalRows"], west);
    assert_eq!(page["rows"].as_array().unwrap().len(), 2);
    let stats = r
        .run(
            "data.columnStats",
            json!({"docId": doc, "table": "Orders", "column": "Status"}),
        )
        .await;
    assert_eq!(stats["rows"], exp.rows("Orders"));

    let render = r
        .run("render.page", json!({"docId": doc, "report": "Stores"}))
        .await;
    assert_eq!(render["ok"], true, "{}", render["visuals"]);
    let card = exp.card("Stores", "Store Performance", "kpis", "Total Sales");
    assert!(
        render["svg"]
            .as_str()
            .unwrap()
            .contains(&format!(">{card}<")),
        "no {card} on the page"
    );

    // A CSV load replaces a table and recomputes what depends on it (do it
    // last: the raw CSV changes the values).
    let loaded = r
        .run(
            "data.loadCsv",
            json!({"docId": doc, "table": "Orders", "path": csv}),
        )
        .await;
    assert_eq!(loaded["source"], "manual");
    assert_eq!(loaded["tables"][0]["name"], "Orders");
    // The raw CSV, without the M steps (filters, de-duplication).
    assert_eq!(
        loaded["tables"][0]["rows"],
        exp.0["rawCsvRows"]["orders.csv"]
    );

    let dir = tmp_dir("pbi-ops-e2e-");
    let out = dir.path().join("orders.csv");
    let exported = r
        .run(
            "export.table",
            json!({"docId": doc, "table": "Orders", "format": "csv", "path": out}),
        )
        .await;
    assert_eq!(exported["rows"], loaded["tables"][0]["rows"]);
    assert!(exported["columns"].as_u64().unwrap() > 10);
    let e = r
        .try_run(
            "export.table",
            json!({"docId": doc, "table": "Orders", "format": "csv", "path": out}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::Io, "no overwrite without overwrite");
}

/// The tables whose M reads the example's blob container.
const BLOB_TABLES: [&str; 7] = [
    "Orders",
    "Customers",
    "Products",
    "Stores",
    "Targets",
    "Returns",
    "Web Sessions",
];

#[tokio::test(flavor = "multi_thread")]
async fn offline_refresh_reports_failures() {
    let Some(pbip) = fixture(PBIP) else { return };
    let r = Runner::new();
    let doc = r
        .open(json!({"path": pbip, "source": "query", "autoRefresh": false}))
        .await;
    let refresh = r.run("data.refresh", json!({"docId": doc})).await;
    assert_eq!(refresh["ok"], false);
    let orders = table(&refresh["tables"], "Orders");
    assert_eq!(orders["status"]["state"], "error");
    assert!(
        orders["status"]["hint"]
            .as_str()
            .unwrap()
            .contains("blobMap"),
        "{}",
        orders["status"]
    );
    for name in BLOB_TABLES {
        let status = &table(&refresh["tables"], name)["status"];
        assert_eq!(status["csvFallback"], true, "{name}: {status}");
        assert_eq!(status["offline"], true, "{name}: {status}");
    }
    let e = r
        .try_run(
            "data.refresh",
            json!({"docId": doc, "tables": ["No Such Table"]}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::NotFound);

    // "Load CSV manually…" stands in for a failed table (a synthetic CSV, `;`-separated).
    let status = r.run("data.tables", json!({"docId": doc})).await;
    assert_eq!(
        table(&status["tables"], "Products Never Sold")["canLoadCsv"],
        false
    );
    let dir = tmp_dir("pbi-ops-csv-");
    let csv = dir.path().join("stores.csv");
    std::fs::write(
        &csv,
        "Store ID;Store Name;Region\nS01;North Pier;West\nS02;Lakeside;East\n;(Online);Online\n",
    )
    .unwrap();
    let options = json!({"delimiter": ";", "hasHeader": true});
    let loaded = r
        .run(
            "data.loadCsv",
            json!({"docId": doc, "table": "Stores", "path": csv, "options": options}),
        )
        .await;
    let stores = table(&loaded["tables"], "Stores");
    assert_eq!(
        (&stores["rows"], &stores["kind"]),
        (&json!(3), &json!("manual"))
    );
    let status = r.run("data.tables", json!({"docId": doc})).await;
    let stores = table(&status["tables"], "Stores");
    assert_eq!(
        (&stores["rows"], &stores["loaded"]),
        (&json!(3), &json!(true))
    );
    assert_eq!(stores["lastRefresh"]["kind"], "manual");
    // The grid's page: formatted text, BLANK as null.
    let page = r
        .run(
            "data.page",
            json!({"docId": doc, "table": "Stores", "limit": 10, "formatted": true, "blankAsNull": true}),
        )
        .await;
    let column = |name: &str| {
        page["columns"]
            .as_array()
            .unwrap()
            .iter()
            .position(|c| c["name"] == name)
            .unwrap_or_else(|| panic!("no column {name}"))
    };
    let (names, city) = (column("Store Name"), column("City"));
    let texts: Vec<&Value> = page["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| &row[names])
        .collect();
    assert_eq!(
        texts,
        [&json!("North Pier"), &json!("Lakeside"), &json!("(Online)")]
    );
    assert!(page["rows"][0][city].is_null());

    for (table_name, options) in [
        ("Products Never Sold", options.clone()),
        ("Stores", json!({"delimiter": ";;"})),
    ] {
        let e = r
            .try_run(
                "data.loadCsv",
                json!({"docId": doc, "table": table_name, "path": csv, "options": options}),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(e.code, OpErrorCode::Query | OpErrorCode::InvalidParams),
            "{table_name}: {e:?}"
        );
    }
}

/// Texts of the `<text>` elements inside the SVG group with `data-hit="<hit>"`.
fn group_texts(svg: &str, hit: &str) -> Vec<String> {
    let attr = format!("data-hit=\"{hit}\"");
    let Some(at) = svg.find(&attr) else {
        return Vec::new();
    };
    let start = svg[..at].rfind("<g").unwrap();
    let (mut depth, mut i, mut out) = (0usize, start, Vec::new());
    while i < svg.len() {
        let rest = &svg[i..];
        if rest.starts_with("<g ") || rest.starts_with("<g>") {
            depth += 1;
        } else if rest.starts_with("</g>") {
            depth -= 1;
            if depth == 0 {
                break;
            }
        } else if rest.starts_with("<text") {
            let open = rest.find('>').unwrap();
            let close = rest.find("</text>").unwrap();
            out.push(rest[open + 1..close].replace("&amp;", "&"));
        }
        i += rest.chars().next().unwrap().len_utf8();
    }
    out
}

fn ids(page: &Value, key: &str) -> Vec<String> {
    page[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_owned())
        .collect()
}

/// The interactive canvas on the example's reports: SVG pages with the expected card values,
/// the Back button's action, slicer dropdown + item selection, chart cross-filtering (and
/// clearing it), the effective filters feeding `measures.values`, table scrolling, tooltips
/// and Reset to default.
#[tokio::test(flavor = "multi_thread")]
async fn canvas_on_the_example_reports() {
    let (Some(pbip), Some(map), Some(exp)) = (fixture(PBIP), fixture(BLOB_MAP), expected()) else {
        return;
    };
    let r = Runner::new();
    let doc = r
        .open(json!({"path": pbip, "blobMap": map, "source": "query", "autoRefresh": false}))
        .await;
    let refresh = r.run("data.refresh", json!({"docId": doc})).await;
    assert_eq!(refresh["ok"], true, "{refresh}");
    let page_of = |report: &str, page: &str, extra: Value| {
        let mut body = json!({"docId": doc, "report": exp.report_name(report), "page": page});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        body
    };

    // Overview, Product Detail: the KPI cards and the Back button.
    let detail = "Product Detail";
    let at = |extra: Value| page_of("Overview", detail, extra);
    let first = r.run("canvas.render", at(json!({}))).await;
    let svg = first["svg"].as_str().unwrap();
    assert!(svg.starts_with("<svg"));
    for measure in ["Total Sales", "Units Sold", "Margin %", "Revenue per Unit"] {
        let value = exp.card("Overview", detail, "kpis", measure);
        assert!(svg.contains(&format!(">{value}<")), "{measure}: no {value}");
    }
    assert!(!svg.contains("<script") && !svg.contains("@font-face"));
    assert_eq!(
        (first["width"].as_f64(), first["height"].as_f64()),
        (Some(1280.0), Some(1080.0))
    );
    assert!(first["overlaySvg"].is_null());
    assert_eq!(first["isDefault"], true);
    let again = r.run("canvas.render", at(json!({}))).await;
    assert_eq!(again["svg"], first["svg"], "renders are deterministic");
    assert!(again["generation"].as_u64() > first["generation"].as_u64());
    let fitted = r
        .run(
            "canvas.render",
            at(json!({"viewport": {"width": 640, "height": 360}})),
        )
        .await;
    assert!(
        fitted["width"].as_f64().unwrap() <= 640.0,
        "{}",
        fitted["width"]
    );
    let button = first["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["visualType"] == "actionButton")
        .unwrap();
    assert_eq!(button["hasAction"], true);
    let hit = format!("{}|button", button["name"].as_str().unwrap());
    assert!(ids(&first, "hitRegions").contains(&hit));
    let clicked = r
        .run(
            "canvas.click",
            at(json!({"hitId": hit, "renderUnchanged": false})),
        )
        .await;
    assert_eq!(clicked["action"]["kind"], "back", "{clicked}");
    assert!(clicked["page"].is_null());
    let e = r
        .try_run("canvas.click", at(json!({"hitId": "nope"})))
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::InvalidParams);
    let e = r
        .try_run("canvas.render", page_of("Overview", "999999", json!({})))
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::NotFound);

    // Stores, Store Performance: saved slicers Region = West and 2025.
    let perf = "Store Performance";
    let at = |extra: Value| page_of("Stores", perf, extra);
    let click = |hit: Value| {
        let body = at(json!({"hitId": hit}));
        let r = &r;
        async move { r.run("canvas.click", body).await }
    };
    let kpis = exp.visual_id("Stores", perf, "kpis");
    let total =
        |page: &Value| group_texts(page["svg"].as_str().unwrap(), &format!("{kpis}|visual"));
    let west = exp.card("Stores", perf, "kpis", "Total Sales").to_owned();
    // Without the Region selection: every Retail and Outlet store in 2025, which is
    // the year-to-date total at the end of the data (2025-12-31).
    let all_regions = exp.measure(Some("Stores"), "Sales YTD").to_owned();
    let page = r.run("canvas.render", at(json!({}))).await;
    assert!(total(&page).contains(&west), "{:?}", total(&page));

    let region = exp.visual_id("Stores", perf, "region");
    let cleared = r
        .run("canvas.clearSelections", at(json!({"visual": region})))
        .await;
    assert!(
        total(&cleared).contains(&all_regions),
        "{:?}",
        total(&cleared)
    );
    assert_eq!(cleared["isDefault"], false);

    // The Region dropdown opens into the overlay; "West" selects.
    let open = click(json!(format!("{region}|button"))).await["page"].clone();
    let overlay = open["overlaySvg"].as_str().unwrap();
    let item = ids(&open, "overlayHitRegions")
        .into_iter()
        .find(|id| group_texts(overlay, id) == ["West"])
        .unwrap_or_else(|| panic!("no West item in {overlay}"));
    let state = open["interactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["visual"] == region)
        .unwrap();
    assert_eq!(
        (&state["dropdownOpen"], &state["slicerMode"]),
        (&json!(true), &json!("dropdown"))
    );
    assert!(state["popup"]["height"].as_f64().unwrap() > 0.0);
    let selected = click(json!(item)).await["page"].clone();
    assert!(total(&selected).contains(&west), "{:?}", total(&selected));
    let selections = selected["selections"].as_array().unwrap();
    let slicer = selections
        .iter()
        .find(|s| s["visual"] == region)
        .unwrap_or_else(|| panic!("no Region selection in {selections:?}"));
    assert_eq!(slicer["kind"], "slicer");
    assert_eq!(slicer["summary"], "is West");
    // Clicking outside closes the dropdown; the selection stays.
    let closed = click(Value::Null).await["page"].clone();
    assert!(closed["overlaySvg"].is_null());
    assert!(total(&closed).contains(&west));

    // The table's "Seattle Flagship" row cross-filters the cards; clicking it again clears.
    let stores = exp.visual("Stores", perf, "stores");
    let table_id = stores["visual"].as_str().unwrap().to_owned();
    let row = &stores["rows"][0];
    let store = row["Store Name"].as_str().unwrap();
    let store_sales = row["Total Sales"]["formatted"].as_str().unwrap();
    let mut cell = None;
    for id in ids(&closed, "hitRegions") {
        if !id.starts_with(&format!("{table_id}|dataPoint|")) {
            continue;
        }
        let tip = r.run("canvas.tooltip", at(json!({"hitId": id}))).await;
        if tip["lines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["value"] == store)
        {
            cell = Some(id);
            break;
        }
    }
    let cell = cell.unwrap_or_else(|| panic!("no {store} row"));
    let filtered = click(json!(cell)).await["page"].clone();
    assert!(
        total(&filtered).contains(&store_sales.to_owned()),
        "{:?}",
        total(&filtered)
    );
    let cross = filtered["selections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["kind"] == "crossFilter")
        .unwrap_or_else(|| panic!("no cross-filter in {}", filtered["selections"]));
    assert_eq!(cross["visual"], table_id.as_str());
    let unfiltered = click(json!(cell)).await["page"].clone();
    assert!(total(&unfiltered).contains(&west));
    assert!(
        unfiltered["selections"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["kind"] != "crossFilter")
    );

    // Reset to default.
    let reset = r.run("canvas.reset", at(json!({}))).await;
    assert_eq!(reset["isDefault"], true);
    assert!(total(&reset).contains(&west));

    // Overview, Executive Summary: the Year list slicer (saved: 2025). The Measures panel
    // gets the page's effective filters (all categorical here): its values match the cards.
    let summary = "Executive Summary";
    let at = |extra: Value| page_of("Overview", summary, extra);
    let click = |hit: Value| {
        let body = at(json!({"hitId": hit}));
        let r = &r;
        async move { r.run("canvas.click", body).await }
    };
    let kpis = exp.visual_id("Overview", summary, "kpis");
    let total =
        |page: &Value| group_texts(page["svg"].as_str().unwrap(), &format!("{kpis}|visual"));
    let panel = |page: &Value| {
        let body = json!({"docId": doc, "filters": page["effectiveFilters"]["filters"]});
        let r = &r;
        async move {
            let out = r.run("measures.values", body).await;
            measure(&out, "Total Sales")["display"].clone()
        }
    };
    let in_2025 = exp
        .card("Overview", summary, "kpis", "Total Sales")
        .to_owned();
    let page = r.run("canvas.render", at(json!({}))).await;
    assert!(total(&page).contains(&in_2025), "{:?}", total(&page));
    assert!(
        page["effectiveFilters"]["unsupported"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(panel(&page).await, in_2025.as_str());
    // 2024: the Year 2024 row of the Time Intelligence matrix (same report filter).
    let in_2024 = exp.visual("Overview", "Time Intelligence", "calc_matrix")["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["Year"] == 2024 || row["Year"] == "2024")
        .unwrap()["Current"]["formatted"]
        .as_str()
        .unwrap()
        .to_owned();
    let year = exp.visual_id("Overview", summary, "year");
    let item = ids(&page, "hitRegions")
        .into_iter()
        .filter(|id| id.starts_with(&format!("{year}|")))
        .find(|id| group_texts(page["svg"].as_str().unwrap(), id) == ["2024"])
        .unwrap_or_else(|| panic!("no 2024 item in the Year slicer"));
    let selected = click(json!(item)).await["page"].clone();
    assert!(
        total(&selected).contains(&in_2024),
        "{:?}",
        total(&selected)
    );
    assert_eq!(panel(&selected).await, in_2024.as_str());
    // No year: the report filter only.
    let all_years = exp.measure(Some("Overview"), "Total Sales").to_owned();
    let cleared = r
        .run("canvas.clearSelections", at(json!({"visual": year})))
        .await;
    assert!(
        total(&cleared).contains(&all_years),
        "{:?}",
        total(&cleared)
    );
    assert_eq!(panel(&cleared).await, all_years.as_str());

    // Online, Online Overview: the device pie shows only the Mobile slice (report filter on
    // the unrelated Web Sessions table); cross-filtering by it changes no sales value.
    let online = "Online Overview";
    let at = |extra: Value| page_of("Online", online, extra);
    let pie = exp.visual_id("Online", online, "devices");
    let page = r.run("canvas.render", at(json!({}))).await;
    let slice = ids(&page, "hitRegions")
        .into_iter()
        .find(|id| id.starts_with(&format!("{pie}|dataPoint|")))
        .unwrap();
    let tip = r.run("canvas.tooltip", at(json!({"hitId": slice}))).await;
    let sessions = exp.card("Online", online, "sessions", "Sessions");
    let lines = tip["lines"].as_array().unwrap();
    assert!(lines.iter().any(|l| l["value"] == "Mobile"), "{tip}");
    assert!(lines.iter().any(|l| l["value"] == sessions), "{tip}");
    let filtered = r.run("canvas.click", at(json!({"hitId": slice}))).await["page"].clone();
    assert!(
        filtered["selections"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["kind"] == "crossFilter" && s["visual"] == pie.as_str()),
        "{}",
        filtered["selections"]
    );
    let values = r
        .run(
            "measures.values",
            json!({"docId": doc, "filters": filtered["effectiveFilters"]["filters"]}),
        )
        .await;
    for (key, name) in [
        ("kpis", "Total Sales"),
        ("kpis", "Order Count"),
        ("sessions", "Sessions"),
    ] {
        assert_eq!(
            measure(&values, name)["display"],
            exp.card("Online", online, key, name),
            "{name}"
        );
    }

    // Online, Customer Detail: the customer table scrolls; its first visible row moves
    // down, and stops at the top.
    let customers = "Customer Detail";
    let at = |extra: Value| page_of("Online", customers, extra);
    let table_id = exp.visual_id("Online", customers, "table");
    let first_row = |page: &Value| {
        ids(page, "hitRegions")
            .iter()
            .filter_map(|id| {
                id.strip_prefix(&format!("{table_id}|dataPoint|"))
                    .map(str::to_owned)
            })
            .map(|rest| rest.split('|').nth(1).unwrap().parse::<u64>().unwrap())
            .min()
            .unwrap()
    };
    let page = r.run("canvas.render", at(json!({}))).await;
    assert_eq!(first_row(&page), 0);
    let scroll = |dy: f64| {
        let body = at(json!({"visual": table_id, "dy": dy, "renderUnchanged": false}));
        let r = &r;
        async move { r.run("canvas.scroll", body).await }
    };
    let top = scroll(-50.0).await;
    assert_eq!(top["changed"], false, "already at the top");
    assert!(top["page"].is_null());
    let scrolled = scroll(200.0).await["page"].clone();
    assert!(first_row(&scrolled) > 0, "{}", first_row(&scrolled));
    let range = scrolled["interactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["visual"] == table_id)
        .unwrap()["scroll"]
        .clone();
    assert_eq!(range["y"].as_f64(), Some(200.0));
    assert!(range["maxY"].as_f64().unwrap() > 200.0);
    let reset = r.run("canvas.reset", at(json!({}))).await;
    assert_eq!(reset["isDefault"], true);
    assert_eq!(first_row(&reset), 0);
}

/// A PBIP whose `.pbi/cache.abf` holds data: a committed Microsoft sample PBIX saved as a
/// PBIP with its data model as the cache (as Power BI Desktop would leave it). `doc.open`
/// announces the cache, `data.refresh` with source `cache` loads it (no M), and `query`
/// evaluates the M queries again.
#[tokio::test(flavor = "multi_thread")]
async fn pbip_local_data_cache() {
    let (Some(pbix), Some(facts)) = (fixture(MS_PBIX), ms_expected(MS_PBIX)) else {
        return;
    };
    let dir = tmp_dir("pbi-ops-cache-");
    let opened = pbi_core::open_path(&pbix, &pbi_core::OpenOptions::default()).unwrap();
    pbi_core::save_as_pbip(&opened, dir.path(), &pbi_core::SaveOptions::default()).unwrap();
    let Some(pbi_core::ModelSource::Abf { archive, .. }) = &opened.model_source else {
        panic!("the PBIX model is not read from its DataModel");
    };
    let entries: Vec<PathBuf> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    let find = |suffix: &str| {
        entries
            .iter()
            .find(|p| p.to_string_lossy().ends_with(suffix))
            .cloned()
            .unwrap()
    };
    let cache = find(".SemanticModel").join(".pbi");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("cache.abf"), archive.as_bytes()).unwrap();

    let r = Runner::new();
    let summary = r
        .run(
            "doc.open",
            json!({"path": find(".pbip"), "autoRefresh": false}),
        )
        .await;
    assert_eq!(summary["hasCache"], true, "{summary}");
    let doc = summary["docId"].as_str().unwrap().to_owned();
    let status = r.run("data.tables", json!({"docId": doc})).await;
    let info = &status["cache"];
    assert!(
        info["entry"]
            .as_str()
            .unwrap()
            .ends_with(".SemanticModel/.pbi/cache.abf"),
        "{status}"
    );
    assert_eq!(info["size"], json!(archive.as_bytes().len()));

    let report = r
        .run("data.refresh", json!({"docId": doc, "source": "cache"}))
        .await;
    assert_eq!(report["ok"], true, "{report}");
    assert!(
        report["tables"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["kind"] == "cache"),
        "{report}"
    );
    let status = r.run("data.tables", json!({"docId": doc})).await;
    for t in facts["model"]["tables"].as_array().unwrap() {
        let name = t["name"].as_str().unwrap();
        let loaded = table(&status["tables"], name);
        assert_eq!(
            (&loaded["rows"], &loaded["loaded"]),
            (&t["rows"], &json!(true)),
            "{name}"
        );
    }
    let report = r
        .run(
            "data.refresh",
            json!({"docId": doc, "tables": ["Opportunity"], "source": "query"}),
        )
        .await;
    assert_eq!(report["tables"][0]["kind"], "m", "{report}");
}

#[tokio::test(flavor = "multi_thread")]
async fn exports_and_conversions_write_files() {
    let (Some(pbip), Some(exp)) = (fixture(PBIP), expected()) else {
        return;
    };
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    let dir = tmp_dir("pbi-ops-convert-");

    let md = r
        .run("export.model", json!({"docId": doc, "format": "markdown"}))
        .await;
    assert!(
        md["markdown"]
            .as_str()
            .unwrap()
            .starts_with("# AcmeOutfitters\n")
    );
    let json_doc = r
        .run(
            "export.model",
            json!({"docId": doc, "format": "json", "includeExpressions": false}),
        )
        .await;
    assert_eq!(json_doc["json"]["schemaVersion"], 1);

    // Conversions include unsaved edits and leave the document where it is.
    const NOTE: &str = "Edited before the conversion";
    r.run(
        "model.object.setDescription",
        json!({"docId": doc, "table": "Orders", "text": NOTE}),
    )
    .await;
    let project = dir.path().join("project");
    let pbip_out = r
        .run("convert.pbip", json!({"docId": doc, "outDir": project}))
        .await;
    assert!(pbip_out["files"].as_array().unwrap().len() > 10);
    let pbip_file = pbip_out["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .find(|f| f.ends_with(".pbip"))
        .unwrap();
    let converted = r
        .open(json!({"path": project.join(pbip_file), "autoRefresh": false}))
        .await;
    let model = r
        .run(
            "export.model",
            json!({"docId": converted, "format": "json"}),
        )
        .await;
    assert!(model.to_string().contains(NOTE), "the edit is converted");
    let history = r.run("doc.history", json!({"docId": doc})).await;
    assert_eq!(history["dirty"], true, "conversion is not a save");
    let list = r.run("doc.list", json!({})).await;
    assert_eq!(list["documents"][0]["format"], "pbip");
    assert_eq!(list["documents"][0]["path"], json!(pbip));
    let e = r
        .try_run("convert.pbip", json!({"docId": doc, "outDir": project}))
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::Io, "non-empty folder needs overwrite");

    let stores = exp.report("Stores");
    let pbit = dir.path().join("stores.pbit");
    let out = r
        .run(
            "convert.pbit",
            json!({"docId": doc, "outPath": pbit, "report": stores["report"]}),
        )
        .await;
    assert_eq!(out["report"], stores["report"]);
    let reports = exp.0["reports"].as_array().unwrap().len();
    assert_eq!(out["omittedReports"].as_array().unwrap().len(), reports - 1);
    let reopened = r
        .run("doc.open", json!({"path": pbit, "autoRefresh": false}))
        .await;
    assert_eq!(reopened["format"], "pbit");
    assert_eq!(reopened["reports"].as_array().unwrap().len(), 1);
    let template = r
        .run(
            "export.model",
            json!({"docId": reopened["docId"], "format": "json"}),
        )
        .await;
    assert!(
        template.to_string().contains(NOTE),
        "the edit is in the PBIT"
    );
    assert_eq!(
        reopened["reports"][0]["pages"],
        stores["pages"].as_array().unwrap().len(),
        "the Stores report"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn column_and_table_delete_through_the_registry() {
    let (Some(pbip), Some(map)) = (fixture(PBIP), fixture(BLOB_MAP)) else {
        eprintln!("SKIP: {PBIP} missing");
        return;
    };
    let r = Runner::new();
    let doc = r
        .open(json!({"path": pbip, "blobMap": map, "source": "query", "autoRefresh": false}))
        .await;
    let refresh = r.run("data.refresh", json!({"docId": doc})).await;
    assert_eq!(refresh["ok"], true, "{refresh}");
    let number = |v: Value| v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"));
    let locations = |out: &Value| -> Vec<String> {
        out["dependents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["location"].as_str().unwrap().to_owned())
            .collect()
    };
    let fails = |dax: &str| {
        let query = format!("EVALUATE ROW(\"n\", {dax})");
        r.try_run("dax.query", json!({"docId": doc, "query": query}))
    };

    // A dry run lists what uses the column and changes nothing.
    let dry = r
        .run(
            "model.column.delete",
            json!({"docId": doc, "table": "orders", "column": "line cost", "dryRun": true}),
        )
        .await;
    assert_eq!(
        (
            &dry["changed"],
            &dry["revision"],
            &dry["dryRun"],
            &dry["kind"],
            &dry["table"],
            &dry["name"],
            &dry["removed"]
        ),
        (
            &json!(false),
            &json!(0),
            &json!(true),
            &json!("column"),
            &json!("Orders"),
            &json!("Line Cost"),
            &json!([])
        )
    );
    assert!(
        locations(&dry).contains(&"measure 'Orders'[Total Cost]".to_owned()),
        "{dry}"
    );

    // Data columns come from the query: refused, in a dry run too.
    for dry_run in [true, false] {
        let quantity =
            json!({"docId": doc, "table": "Orders", "column": "Quantity", "dryRun": dry_run});
        assert_eq!(
            error_code(r.try_run("model.column.delete", quantity).await),
            OpErrorCode::InvalidParams
        );
    }
    let table_id = json!({"docId": doc, "columnId": dry["objectId"], "table": "Orders"});
    assert_eq!(
        error_code(r.try_run("model.column.delete", table_id).await),
        OpErrorCode::InvalidParams
    );

    // Deleted, the column cannot be queried; undo brings its values back.
    let large = "CALCULATE(COUNTROWS(Orders), Orders[Order Size] = \"Large\")";
    let large_orders = number(scalar(&r, &doc, large).await);
    assert!(large_orders > 0.0);
    let deleted = r
        .run(
            "model.column.delete",
            json!({"docId": doc, "table": "Orders", "column": "Order Size"}),
        )
        .await;
    assert_eq!(
        (
            &deleted["changed"],
            &deleted["revision"],
            &deleted["dryRun"],
            &deleted["changedObjectIds"]
        ),
        (
            &json!(true),
            &json!(1),
            &json!(false),
            &json!([deleted["objectId"]])
        )
    );
    assert!(fails(large).await.is_err());
    r.run("doc.undo", json!({"docId": doc})).await;
    assert_eq!(number(scalar(&r, &doc, large).await), large_orders);

    // A column and the measures that use it go in one undo step.
    let margin = number(scalar(&r, &doc, "[Gross Margin]").await);
    let group = r
        .run(
            "doc.applyEdits",
            json!({"docId": doc, "edits": [
                {"type": "deleteMeasure", "table": "Orders", "measure": "Gross Margin"},
                {"type": "deleteMeasure", "table": "Orders", "measure": "Total Cost"},
                {"type": "deleteColumn", "table": "Orders", "column": "Line Cost"}
            ]}),
        )
        .await;
    assert_eq!(group["changedObjectIds"].as_array().unwrap().len(), 3);
    assert!(fails("[Gross Margin]").await.is_err());
    assert!(fails("SUM(Orders[Line Cost])").await.is_err());
    r.run("doc.undo", json!({"docId": doc})).await;
    assert_eq!(number(scalar(&r, &doc, "[Gross Margin]").await), margin);

    // A table goes with its relationships and role row filters.
    let sales = number(scalar(&r, &doc, "[Total Sales]").await);
    let stores_rows = number(scalar(&r, &doc, "COUNTROWS(Stores)").await);
    let relationships = r.run("model.relationships", json!({"docId": doc})).await;
    let count = relationships["relationships"].as_array().unwrap().len();
    let dry = r
        .run(
            "model.table.delete",
            json!({"docId": doc, "table": "Stores", "dryRun": true}),
        )
        .await;
    assert_eq!(
        (&dry["changed"], &dry["kind"], &dry["removed"]),
        (
            &json!(false),
            &json!("table"),
            &json!([
                "relationship 'Orders'[Store ID] → 'Stores'[Store ID]",
                "relationship 'Targets'[Region] → 'Stores'[Region]",
                "row filter of role 'Region West'",
                "row filter of role 'Store Manager'"
            ])
        )
    );
    // Its own measures and columns go with it; others' references break.
    assert!(
        locations(&dry)
            .iter()
            .all(|l| !l.starts_with("measure 'Stores'") && !l.starts_with("column 'Stores'")),
        "{dry}"
    );
    assert!(
        locations(&dry).iter().any(|l| l.starts_with("visual ")),
        "{dry}"
    );
    let deleted = r
        .run(
            "model.table.delete",
            json!({"docId": doc, "table": "Stores"}),
        )
        .await;
    assert_eq!(
        (&deleted["changed"], &deleted["revision"]),
        (&json!(true), &json!(5))
    );
    let tables = r.run("model.tables", json!({"docId": doc})).await;
    assert!(
        tables["tables"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["name"] != "Stores"),
        "{tables}"
    );
    let after = r.run("model.relationships", json!({"docId": doc})).await;
    assert_eq!(after["relationships"].as_array().unwrap().len(), count - 2);
    assert!(fails("COUNTROWS(Stores)").await.is_err());
    assert_eq!(number(scalar(&r, &doc, "[Total Sales]").await), sales);
    r.run("doc.undo", json!({"docId": doc})).await;
    assert_eq!(
        number(scalar(&r, &doc, "COUNTROWS(Stores)").await),
        stores_rows
    );
    let smallest_region = "MINX(VALUES(Stores[Region]), CALCULATE([Total Sales]))";
    assert!(number(scalar(&r, &doc, smallest_region).await) < sales);
    let restored = r.run("model.relationships", json!({"docId": doc})).await;
    assert_eq!(restored["relationships"], relationships["relationships"]);
}

#[tokio::test]
async fn calculated_column_create_through_the_registry() {
    let (Some(pbip), Some(map)) = (fixture(PBIP), fixture(BLOB_MAP)) else {
        eprintln!("SKIP: {PBIP} missing");
        return;
    };
    let r = Runner::new();
    let doc = r
        .open(json!({"path": pbip, "blobMap": map, "source": "query", "autoRefresh": false}))
        .await;
    let number = |v: Value| v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"));
    let fails = |dax: &str| {
        let query = format!("EVALUATE ROW(\"n\", {dax})");
        r.try_run("dax.query", json!({"docId": doc, "query": query}))
    };
    let create = |params: Value| {
        let mut params = params;
        params["docId"] = json!(doc);
        r.try_run("model.column.create", params)
    };

    // Before a refresh nothing is loaded: the data type must be given, and
    // the refresh computes the column.
    let margin = json!({"table": "Orders", "name": "Margin",
        "expression": "Orders[Line Amount] - Orders[Line Cost]"});
    assert_eq!(
        error_code(create(margin.clone()).await),
        OpErrorCode::InvalidParams
    );
    let mut typed = margin.clone();
    typed["dataType"] = json!("double");
    let out = create(typed).await.unwrap();
    assert_eq!(
        (
            &out["changed"],
            &out["revision"],
            &out["table"],
            &out["name"],
            &out["dataType"],
            &out["dataTypeInferred"],
            &out["changedObjectIds"]
        ),
        (
            &json!(true),
            &json!(1),
            &json!("Orders"),
            &json!("Margin"),
            &json!("double"),
            &json!(false),
            &json!([out["columnId"]])
        )
    );
    let refresh = r.run("data.refresh", json!({"docId": doc})).await;
    assert_eq!(refresh["ok"], true, "{refresh}");
    let gross = number(scalar(&r, &doc, "[Gross Margin]").await);
    let summed = number(scalar(&r, &doc, "SUM(Orders[Margin])").await);
    assert!(
        (summed - gross).abs() <= gross.abs() * 1e-9,
        "{summed} {gross}"
    );

    // Over loaded data the type is inferred and the column computed at once.
    let size = create(json!({"table": "orders", "name": "Size Code",
        "expression": "LEFT(Orders[Order Size], 1)"}))
    .await
    .unwrap();
    assert_eq!(
        (
            &size["dataType"],
            &size["dataTypeInferred"],
            &size["revision"]
        ),
        (&json!("string"), &json!(true), &json!(2))
    );
    let large = "CALCULATE(COUNTROWS(Orders), Orders[Order Size] = \"Large\")";
    let l = "CALCULATE(COUNTROWS(Orders), Orders[Size Code] = \"L\")";
    assert_eq!(scalar(&r, &doc, l).await, scalar(&r, &doc, large).await);
    let units = create(json!({"table": "Orders", "name": "Double Units",
        "expression": "Orders[Quantity] * 2"}))
    .await
    .unwrap();
    assert_eq!(units["dataType"], "int64", "{units}");

    // Refusals: names taken by a column or measure, unknown columns, DAX
    // that does not parse, all-blank values without a type, unknown tables.
    for (name, expression) in [
        ("quantity", "1"),
        ("Total Sales", "1"),
        ("Bad", "Orders[Nope] + 1"),
        ("Bad", "1 +"),
        ("Bad", ""),
        ("Bad", "BLANK()"),
    ] {
        let params = json!({"table": "Orders", "name": name, "expression": expression});
        assert_eq!(
            error_code(create(params).await),
            OpErrorCode::InvalidParams,
            "{name} = {expression}"
        );
    }
    let nowhere = json!({"table": "Nowhere", "name": "X", "expression": "1"});
    assert_eq!(error_code(create(nowhere).await), OpErrorCode::NotFound);
    let blank = json!({"table": "Orders", "name": "Nothing", "expression": "BLANK()",
        "dataType": "string"});
    create(blank).await.unwrap();
    assert_eq!(
        number(scalar(&r, &doc, "COUNTBLANK(Orders[Nothing])").await),
        number(scalar(&r, &doc, "COUNTROWS(Orders)").await)
    );

    // Undo removes the column; redo brings back its values.
    r.run("doc.undo", json!({"docId": doc})).await;
    r.run("doc.undo", json!({"docId": doc})).await;
    assert!(fails("SUM(Orders[Double Units])").await.is_err());
    r.run("doc.redo", json!({"docId": doc})).await;
    assert_eq!(
        number(scalar(&r, &doc, "SUM(Orders[Double Units])").await),
        2.0 * number(scalar(&r, &doc, "[Units Sold]").await)
    );

    // In a group, a column can use one created before it when its type is
    // given, and a measure can use both; one undo step removes all three.
    let group = r
        .run(
            "doc.applyEdits",
            json!({"docId": doc, "edits": [
                {"type": "createColumn", "table": "Orders", "name": "Unit Count",
                 "expression": "Orders[Quantity] + 0"},
                {"type": "createColumn", "table": "Orders", "name": "Units Plus One",
                 "expression": "Orders[Unit Count] + 1", "dataType": "int64"},
                {"type": "createMeasure", "table": "Orders", "name": "Units Plus Rows",
                 "expression": "SUM(Orders[Units Plus One])"}
            ]}),
        )
        .await;
    assert_eq!(group["changedObjectIds"].as_array().unwrap().len(), 3);
    let rows = number(scalar(&r, &doc, "COUNTROWS(Orders)").await);
    let units = number(scalar(&r, &doc, "[Units Sold]").await);
    assert_eq!(
        number(scalar(&r, &doc, "[Units Plus Rows]").await),
        units + rows
    );
    let refresh = r.run("data.refresh", json!({"docId": doc})).await;
    assert_eq!(refresh["ok"], true, "{refresh}");
    assert_eq!(
        number(scalar(&r, &doc, "[Units Plus Rows]").await),
        units + rows
    );
    r.run("doc.undo", json!({"docId": doc})).await;
    assert!(fails("SUM(Orders[Unit Count])").await.is_err());
    assert!(fails("[Units Plus Rows]").await.is_err());

    // Inferring the type loads a document opened with autoRefresh first.
    let auto = r.open(json!({"path": pbip, "blobMap": map})).await;
    let out = r
        .run(
            "model.column.create",
            json!({"docId": auto, "table": "Customers", "name": "Order Lines",
                "expression": "COUNTROWS(RELATEDTABLE(Orders))"}),
        )
        .await;
    assert_eq!(
        (&out["dataType"], &out["dataTypeInferred"]),
        (&json!("int64"), &json!(true))
    );
    assert_eq!(
        scalar(&r, &auto, "SUM(Customers[Order Lines])").await,
        scalar(
            &r,
            &auto,
            "SUMX(Customers, COUNTROWS(RELATEDTABLE(Orders)))"
        )
        .await
    );
}

/// `table[column] state` of each calculated table or column an edit
/// recomputed, in order.
fn recalculated(out: &Value) -> Vec<String> {
    out["recalculated"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|x| {
            let column = x["column"]
                .as_str()
                .map(|c| format!("[{c}]"))
                .unwrap_or_default();
            format!(
                "{}{column} {}",
                x["table"].as_str().unwrap(),
                x["status"]["state"].as_str().unwrap()
            )
        })
        .collect()
}

/// M6.4: edits recompute the calculated tables and columns they reach, in
/// dependency order, without an M refresh (acceptance criterion 3); one
/// that can no longer be calculated loses its values, and undo computes
/// them again.
#[tokio::test]
async fn edits_recalculate_dependants_without_a_refresh() {
    let (Some(pbip), Some(map)) = (fixture(PBIP), fixture(BLOB_MAP)) else {
        eprintln!("SKIP: {PBIP} missing");
        return;
    };
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = Arc::clone(&events);
    let r = Runner {
        ctx: Ctx::new(CallerKind::Cli).with_progress(move |e| sink.lock().unwrap().push(e.message)),
        ..Runner::new()
    };
    let doc = r
        .open(json!({"path": pbip, "blobMap": map, "source": "query", "autoRefresh": false}))
        .await;
    let refresh = r.run("data.refresh", json!({"docId": doc})).await;
    assert_eq!(refresh["ok"], true, "{refresh}");
    assert!(
        !events.lock().unwrap().is_empty(),
        "a refresh reports progress"
    );
    events.lock().unwrap().clear();
    let number = |v: Value| v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"));
    let close = |a: f64, b: f64| (a - b).abs() <= b.abs().max(1.0) * 1e-9;
    let fails = |dax: &str| {
        let query = format!("EVALUATE ROW(\"n\", {dax})");
        r.try_run("dax.query", json!({"docId": doc, "query": query}))
    };
    let undo = || r.run("doc.undo", json!({"docId": doc}));
    let sales = number(scalar(&r, &doc, "[Total Sales]").await);

    // A calculated column's new expression recomputes it and then the
    // column that uses it; its type is inferred again. Undo recomputes both.
    let lifetime = "SUM(Customers[Lifetime Sales])";
    let lifetime_before = number(scalar(&r, &doc, lifetime).await);
    let high = "CALCULATE(COUNTROWS(Customers), Customers[Value Tier] = \"High\") + 0";
    let high_before = scalar(&r, &doc, high).await;
    let doubled = r
        .run(
            "model.column.update",
            json!({"docId": doc, "table": "customers", "column": "lifetime sales",
                "expression": "CALCULATE(SUM(Orders[Line Amount])) * 2"}),
        )
        .await;
    assert_eq!(
        (
            &doubled["changed"],
            &doubled["table"],
            &doubled["name"],
            &doubled["dataType"],
            &doubled["dataTypeInferred"]
        ),
        (
            &json!(true),
            &json!("Customers"),
            &json!("Lifetime Sales"),
            &json!("double"),
            &json!(true)
        )
    );
    assert_eq!(
        recalculated(&doubled),
        ["Customers[Lifetime Sales] ok", "Customers[Value Tier] ok"]
    );
    assert!(close(
        number(scalar(&r, &doc, lifetime).await),
        2.0 * lifetime_before
    ));
    assert_eq!(
        scalar(&r, &doc, high).await,
        scalar(
            &r,
            &doc,
            "COUNTROWS(FILTER(Customers, Customers[Lifetime Sales] >= 3000)) + 0"
        )
        .await
    );
    let undone = undo().await;
    assert_eq!(recalculated(&undone), recalculated(&doubled), "{undone}");
    assert!(close(
        number(scalar(&r, &doc, lifetime).await),
        lifetime_before
    ));
    assert_eq!(scalar(&r, &doc, high).await, high_before);

    // Refused: a circular dependency (also on itself), DAX that fails, an
    // expression or type on a data column, nothing to change. Nothing is
    // committed.
    let revision = r.run("doc.history", json!({"docId": doc})).await["revision"].clone();
    for params in [
        json!({"table": "Customers", "column": "Lifetime Sales", "expression": "LEN(Customers[Value Tier])"}),
        json!({"table": "Customers", "column": "Lifetime Sales", "expression": "Customers[Lifetime Sales] + 1"}),
        json!({"table": "Customers", "column": "Lifetime Sales", "expression": "Customers[Nope] + 1"}),
        json!({"table": "Customers", "column": "Lifetime Sales", "expression": "1 +"}),
        json!({"table": "Customers", "column": "Lifetime Sales", "expression": " "}),
        json!({"table": "Customers", "column": "Customer ID", "expression": "1"}),
        json!({"table": "Customers", "column": "Customer ID", "dataType": "string"}),
        json!({"table": "Customers", "column": "Lifetime Sales"}),
    ] {
        let mut params = params;
        params["docId"] = json!(doc);
        assert_eq!(
            error_code(r.try_run("model.column.update", params.clone()).await),
            OpErrorCode::InvalidParams,
            "{params}"
        );
    }
    let history = r.run("doc.history", json!({"docId": doc})).await;
    assert_eq!(history["revision"], revision);
    assert!(close(
        number(scalar(&r, &doc, lifetime).await),
        lifetime_before
    ));
    // Properties of a data column can change; nothing is recalculated.
    let formatted = r
        .run(
            "model.column.update",
            json!({"docId": doc, "table": "Customers", "column": "Customer ID", "description": "Key"}),
        )
        .await;
    assert_eq!(
        (&formatted["changed"], &formatted["dataTypeInferred"]),
        (&json!(true), &json!(false))
    );
    assert_eq!(recalculated(&formatted), Vec::<String>::new());
    undo().await;

    // A measure edit recomputes the columns that use the measure; the new
    // values need no M refresh (acceptance criterion 3).
    let spend = r
        .run(
            "model.column.create",
            json!({"docId": doc, "table": "Customers", "name": "Spend", "expression": "[Total Sales]"}),
        )
        .await;
    assert_eq!(recalculated(&spend), ["Customers[Spend] ok"]);
    let spend_sum = "SUM(Customers[Spend])";
    let spend_before = number(scalar(&r, &doc, spend_sum).await);
    let twice = r
        .run(
            "model.measure.update",
            json!({"docId": doc, "table": "Orders", "measure": "Total Sales",
                "expression": "SUM(Orders[Line Amount]) * 2"}),
        )
        .await;
    assert_eq!(recalculated(&twice), ["Customers[Spend] ok"]);
    assert!(close(
        number(scalar(&r, &doc, spend_sum).await),
        2.0 * spend_before
    ));
    let values = r.run("measures.values", json!({"docId": doc})).await;
    assert!(close(
        number(measure(&values, "Total Sales")["value"].clone()),
        2.0 * sales
    ));
    assert_eq!(*events.lock().unwrap(), Vec::<String>::new());
    // A format change recomputes nothing.
    let format = r
        .run(
            "model.measure.update",
            json!({"docId": doc, "table": "Orders", "measure": "Total Sales", "formatString": "0"}),
        )
        .await;
    assert_eq!(recalculated(&format), Vec::<String>::new());
    undo().await;
    undo().await;
    assert!(close(
        number(scalar(&r, &doc, spend_sum).await),
        spend_before
    ));
    undo().await;
    assert!(fails(spend_sum).await.is_err());

    // A relationship change recomputes every calculated table and column
    // that reads another table, in dependency order.
    let everything = |states: [&str; 7]| {
        [
            "Customers[Lifetime Sales]",
            "Customers[Value Tier]",
            "Orders[Line Cost]",
            "Orders[Is Returned]",
            "Customers[Has Orders]",
            "Products[Units Sold (All Time)]",
            "Products Never Sold",
        ]
        .iter()
        .zip(states)
        .map(|(node, state)| format!("{node} {state}"))
        .collect::<Vec<_>>()
    };
    let all_ok = everything(["ok"; 7]);
    // Deactivating Orders → Customers: a customer's lifetime sales are all
    // sales. Undo recomputes them.
    let all = r.run("model.relationships", json!({"docId": doc})).await;
    let customers = relationship(
        &all,
        ("Orders", "Customer ID"),
        ("Customers", "Customer ID"),
    )["relationshipId"]
        .as_u64()
        .unwrap();
    let products =
        relationship(&all, ("Orders", "Product ID"), ("Products", "Product ID"))["relationshipId"]
            .as_u64()
            .unwrap();
    let min_lifetime = "MIN(Customers[Lifetime Sales])";
    let min_before = number(scalar(&r, &doc, min_lifetime).await);
    let off = r
        .run(
            "model.relationship.update",
            json!({"docId": doc, "relationshipId": customers, "isActive": false}),
        )
        .await;
    assert_eq!(recalculated(&off), all_ok);
    assert!(close(number(scalar(&r, &doc, min_lifetime).await), sales));
    let on = undo().await;
    assert_eq!(recalculated(&on), all_ok);
    assert!(close(
        number(scalar(&r, &doc, min_lifetime).await),
        min_before
    ));

    // Deactivating Orders → Products: RELATED fails, so Line Cost loses its
    // values (and Total Cost errors), as do the objects that read the whole
    // Orders table, until undo computes them again.
    let cost = number(scalar(&r, &doc, "[Total Cost]").await);
    let off = r
        .run(
            "model.relationship.update",
            json!({"docId": doc, "relationshipId": products, "isActive": false}),
        )
        .await;
    let lost = ["ok", "ok", "error", "ok", "skipped", "skipped", "skipped"];
    assert_eq!(recalculated(&off), everything(lost));
    assert!(fails("[Total Cost]").await.is_err());
    assert!(fails("SUM(Products[Units Sold (All Time)])").await.is_err());
    let on = undo().await;
    assert_eq!(recalculated(&on), all_ok);
    assert!(close(number(scalar(&r, &doc, "[Total Cost]").await), cost));

    // Deleting a table a calculated column uses: that column errors, with
    // what reads all of Orders (its relationship goes too); undo brings the
    // table's data back and recomputes them.
    let returned = "CALCULATE(COUNTROWS(Orders), Orders[Is Returned] = \"Yes\") + 0";
    let returned_before = scalar(&r, &doc, returned).await;
    let deleted = r
        .run(
            "model.table.delete",
            json!({"docId": doc, "table": "Returns"}),
        )
        .await;
    assert_eq!(
        recalculated(&deleted),
        [
            "Orders[Is Returned] error",
            "Customers[Lifetime Sales] ok",
            "Customers[Value Tier] ok",
            "Orders[Line Cost] ok",
            "Customers[Has Orders] skipped",
            "Products[Units Sold (All Time)] skipped",
            "Products Never Sold skipped",
        ]
    );
    assert!(fails(returned).await.is_err());
    assert_eq!(recalculated(&undo().await), all_ok);
    assert_eq!(scalar(&r, &doc, returned).await, returned_before);

    // None of it re-evaluated an M partition.
    assert_eq!(*events.lock().unwrap(), Vec::<String>::new());
}

#[tokio::test]
async fn hierarchy_create_and_delete_through_the_registry() {
    let Some(pbip) = fixture(PBIP) else {
        eprintln!("SKIP: {PBIP} missing");
        return;
    };
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    let with_doc = |params: Value| {
        let mut params = params;
        params["docId"] = json!(doc);
        params
    };
    let create = |params: Value| r.try_run("model.hierarchy.create", with_doc(params));
    let delete = |params: Value| r.try_run("model.hierarchy.delete", with_doc(params));

    let out = create(json!({"table": "stores", "name": "Places",
        "levels": [{"column": "Region"}, {"column": "store name", "name": "Store"}]}))
    .await
    .unwrap();
    let id = out["hierarchyId"].as_u64().unwrap();
    assert_eq!(
        (
            &out["changed"],
            &out["revision"],
            &out["table"],
            &out["name"],
            &out["changedObjectIds"]
        ),
        (
            &json!(true),
            &json!(1),
            &json!("Stores"),
            &json!("Places"),
            &json!([id])
        ),
        "{out}"
    );
    assert_eq!(out["levelIds"], json!([id + 1, id + 2]), "{out}");

    // Desktop's checks: a free name, levels on different columns of the table.
    for bad in [
        json!({"table": "Stores", "name": "Region", "levels": [{"column": "Region"}]}),
        json!({"table": "Stores", "name": "geography", "levels": [{"column": "Region"}]}),
        json!({"table": "Stores", "name": "H", "levels": []}),
        json!({"table": "Stores", "name": "H", "levels": [{"column": "Nope"}]}),
        json!({"table": "Stores", "name": "H",
            "levels": [{"column": "Region"}, {"column": "Region", "name": "Again"}]}),
    ] {
        assert_eq!(
            error_code(create(bad.clone()).await),
            OpErrorCode::InvalidParams,
            "{bad}"
        );
    }

    // A dry run lists the report fields on the hierarchy and changes nothing.
    let out =
        delete(json!({"table": "Products", "hierarchy": "product hierarchy", "dryRun": true}))
            .await
            .unwrap();
    assert_eq!(
        (
            &out["changed"],
            &out["kind"],
            &out["name"],
            &out["revision"]
        ),
        (
            &json!(false),
            &json!("hierarchy"),
            &json!("Product Hierarchy"),
            &json!(1)
        ),
        "{out}"
    );
    let dependents = out["dependents"].as_array().unwrap();
    assert_eq!(dependents.len(), 1, "{out}");
    let location = dependents[0]["location"].as_str().unwrap();
    assert!(
        location.contains("(pivotTable) on page 'Product Detail'"),
        "{location}"
    );

    let out = delete(json!({"hierarchyId": id})).await.unwrap();
    assert_eq!(
        (&out["changed"], &out["revision"], &out["name"]),
        (&json!(true), &json!(2), &json!("Places")),
        "{out}"
    );
    assert_eq!(
        error_code(delete(json!({"hierarchyId": id})).await),
        OpErrorCode::NotFound
    );
    r.run("doc.undo", with_doc(json!({}))).await;
    let out = delete(json!({"table": "Stores", "hierarchy": "Places", "dryRun": true}))
        .await
        .unwrap();
    assert_eq!(out["objectId"], json!(id), "undo restores the same id");
    assert_eq!(
        error_code(delete(json!({"table": "Stores", "column": "Region"})).await),
        OpErrorCode::InvalidParams
    );

    // One group: create over a column and delete another hierarchy.
    let out = r
        .run(
            "doc.applyEdits",
            with_doc(json!({"edits": [
                {"type": "createHierarchy", "table": "Products", "name": "By Brand",
                    "levels": [{"column": "Category"}, {"column": "Product Name"}]},
                {"type": "deleteHierarchy", "table": "Products", "hierarchy": "Product Hierarchy"}
            ]})),
        )
        .await;
    assert_eq!(out["revision"], json!(4), "{out}");
    assert_eq!(
        out["changedObjectIds"].as_array().unwrap().len(),
        2,
        "{out}"
    );
    r.run("doc.undo", with_doc(json!({}))).await;
    let out =
        delete(json!({"table": "Products", "hierarchy": "Product Hierarchy", "dryRun": true}))
            .await
            .unwrap();
    assert_eq!(out["name"], json!("Product Hierarchy"), "{out}");
    assert_eq!(
        error_code(delete(json!({"table": "Products", "hierarchy": "By Brand"})).await),
        OpErrorCode::NotFound
    );
}

#[tokio::test]
async fn calculated_table_create_through_the_registry() {
    let (Some(pbip), Some(map)) = (fixture(PBIP), fixture(BLOB_MAP)) else {
        eprintln!("SKIP: {PBIP} missing");
        return;
    };
    let r = Runner::new();
    let doc = r
        .open(json!({"path": pbip, "blobMap": map, "source": "query", "autoRefresh": false}))
        .await;
    let with_doc = |params: Value| {
        let mut params = params;
        params["docId"] = json!(doc);
        params
    };
    let create = |params: Value| r.try_run("model.table.create", with_doc(params));
    let number = |v: Value| v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"));

    // Without data the types come from the columns the result's columns
    // stem from, and a refresh computes the table; a computed column needs
    // the data.
    let out = create(json!({"name": "Regions", "expression": "VALUES(Stores[Region])"}))
        .await
        .unwrap();
    let id = out["tableId"].as_u64().unwrap();
    assert_eq!(
        (
            &out["changed"],
            &out["revision"],
            &out["name"],
            &out["changedObjectIds"]
        ),
        (&json!(true), &json!(1), &json!("Regions"), &json!([id])),
        "{out}"
    );
    assert_eq!(
        out["columns"],
        json!([{"columnId": id + 1, "name": "Region", "dataType": "string",
            "sourceColumn": "Stores[Region]"}]),
        "{out}"
    );
    let device_pages = json!({"name": "Device Pages",
        "expression": "ADDCOLUMNS(VALUES('Web Sessions'[Device]), \"Pages\", CALCULATE(SUM('Web Sessions'[Pages Viewed])), \"Share\", DIVIDE(CALCULATE([Sessions]), [Sessions (All Devices)]))"});
    assert_eq!(
        error_code(create(device_pages.clone()).await),
        OpErrorCode::InvalidParams
    );
    let refresh = r.run("data.refresh", with_doc(json!({}))).await;
    assert_eq!(refresh["ok"], true, "{refresh}");
    let regions = number(scalar(&r, &doc, "COUNTROWS(Regions)").await);
    assert!(regions > 1.0, "{regions}");
    assert_eq!(
        regions,
        number(scalar(&r, &doc, "COUNTROWS(VALUES(Stores[Region]))").await)
    );

    // Over loaded data the types come from the values and the table is
    // computed at once.
    let out = create(device_pages).await.unwrap();
    let columns: Vec<_> = out["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["name"].clone(),
                c["dataType"].clone(),
                c["sourceColumn"].clone(),
            )
        })
        .collect();
    assert_eq!(
        columns,
        [
            (
                json!("Device"),
                json!("string"),
                json!("Web Sessions[Device]")
            ),
            (json!("Pages"), json!("int64"), json!("[Pages]")),
            (json!("Share"), json!("double"), json!("[Share]")),
        ],
        "{out}"
    );
    let pages = number(scalar(&r, &doc, "SUM('Device Pages'[Pages])").await);
    let all = number(scalar(&r, &doc, "SUM('Web Sessions'[Pages Viewed])").await);
    assert_eq!(pages, all);
    let share = number(scalar(&r, &doc, "SUM('Device Pages'[Share])").await);
    assert!((share - 1.0).abs() < 1e-9, "{share}");

    // A calculated column on the new table is computed too.
    let out = r
        .run(
            "model.column.create",
            with_doc(json!({"table": "Regions", "name": "Length",
                "expression": "LEN(Regions[Region])"})),
        )
        .await;
    assert_eq!(out["dataType"], json!("int64"), "{out}");
    assert_eq!(
        scalar(&r, &doc, "SUM(Regions[Length])").await,
        scalar(
            &r,
            &doc,
            "SUMX(VALUES(Stores[Region]), LEN(Stores[Region]))"
        )
        .await
    );

    // Refusals: a taken name, a broken or empty expression, a scalar, and
    // duplicate column names.
    for bad in [
        json!({"name": "orders", "expression": "VALUES(Stores[Region])"}),
        json!({"name": "Regions", "expression": "VALUES(Stores[Region])"}),
        json!({"name": "T", "expression": "VALUES(Stores[Region]"}),
        json!({"name": "T", "expression": "  "}),
        json!({"name": "T", "expression": "1 + 1"}),
        json!({"name": "T", "expression": "VALUES(Stores[Nope])"}),
        json!({"name": "T",
            "expression": "SELECTCOLUMNS(Stores, \"A\", Stores[Region], \"A\", Stores[City])"}),
        json!({"name": "", "expression": "VALUES(Stores[Region])"}),
    ] {
        assert_eq!(
            error_code(create(bad.clone()).await),
            OpErrorCode::InvalidParams,
            "{bad}"
        );
    }

    // Undo removes the column, the second table and the first; redo brings
    // the first back with the same id and computes it again.
    r.run("doc.undo", with_doc(json!({}))).await;
    r.run("doc.undo", with_doc(json!({}))).await;
    r.run("doc.undo", with_doc(json!({}))).await;
    let gone = r
        .try_run(
            "dax.query",
            with_doc(json!({"query": "EVALUATE ROW(\"n\", COUNTROWS(Regions))"})),
        )
        .await;
    assert!(gone.is_err(), "{gone:?}");
    r.run("doc.redo", with_doc(json!({}))).await;
    r.run("doc.redo", with_doc(json!({}))).await;
    assert_eq!(
        scalar(&r, &doc, "COUNTROWS(Regions)").await,
        scalar(&r, &doc, "COUNTROWS(VALUES(Stores[Region]))").await
    );
    let out = r
        .run(
            "model.table.delete",
            with_doc(json!({"table": "Regions", "dryRun": true})),
        )
        .await;
    assert_eq!(out["objectId"], json!(id), "redo keeps the id: {out}");

    // One group: a calculated table and the deletion of another.
    let out = r
        .run(
            "doc.applyEdits",
            with_doc(json!({"edits": [
                {"type": "createTable", "name": "Segments",
                    "expression": "DISTINCT(Customers[Segment])"},
                {"type": "deleteTable", "table": "Device Pages"}
            ]})),
        )
        .await;
    assert_eq!(
        out["changedObjectIds"].as_array().unwrap().len(),
        2,
        "{out}"
    );
    assert_eq!(
        scalar(&r, &doc, "COUNTROWS(Segments)").await,
        scalar(&r, &doc, "COUNTROWS(DISTINCT(Customers[Segment]))").await
    );
    r.run("doc.undo", with_doc(json!({}))).await;
    assert_eq!(
        number(scalar(&r, &doc, "SUM('Device Pages'[Pages])").await),
        all
    );
}

#[tokio::test]
async fn calculated_table_update_through_the_registry() {
    let (Some(pbip), Some(map)) = (fixture(PBIP), fixture(BLOB_MAP)) else {
        eprintln!("SKIP: {PBIP} missing");
        return;
    };
    let r = Runner::new();
    let doc = r
        .open(json!({"path": pbip, "blobMap": map, "source": "query", "autoRefresh": false}))
        .await;
    let with_doc = |params: Value| {
        let mut params = params;
        params["docId"] = json!(doc);
        params
    };
    let update = |params: Value| r.try_run("model.table.update", with_doc(params));
    let number = |v: Value| v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"));
    let table = |name: &'static str| {
        let r = &r;
        let doc = doc.clone();
        async move {
            let out = r.run("model.tables", json!({"docId": doc})).await;
            out["tables"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["name"] == name)
                .cloned()
                .unwrap_or_else(|| panic!("no table {name}: {out}"))
        }
    };
    let column = |t: &Value, name: &str| {
        t["columns"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .cloned()
            .unwrap_or_else(|| panic!("no column {name}: {t}"))
    };
    let names = |out: &Value| -> Vec<String> {
        out["columns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap().to_owned())
            .collect()
    };
    let refresh = r.run("data.refresh", with_doc(json!({}))).await;
    assert_eq!(refresh["ok"], true, "{refresh}");
    let never_sold = "COUNTROWS(FILTER(ALL(Products), ISEMPTY(RELATEDTABLE(Orders))))";
    let count = number(scalar(&r, &doc, never_sold).await);
    assert!(count > 0.0, "{count}");

    // Description and visibility of a data table: no recomputation.
    let out = update(json!({"table": "Orders", "description": "Order lines", "isHidden": true}))
        .await
        .unwrap();
    let orders = out["tableId"].as_u64().unwrap();
    assert_eq!(
        (&out["changed"], &out["changedObjectIds"], &out["name"]),
        (&json!(true), &json!([orders]), &json!("Orders")),
        "{out}"
    );
    assert!(out.get("recalculated").is_none(), "{out}");
    assert!(out.get("addedColumns").is_none(), "{out}");
    let t = table("Orders").await;
    assert_eq!(
        (&t["description"], &t["isHidden"]),
        (&json!("Order lines"), &json!(true)),
        "{t}"
    );
    let out = update(json!({"tableId": orders, "description": "", "isHidden": false}))
        .await
        .unwrap();
    assert_eq!(out["changed"], json!(true), "{out}");
    let t = table("Orders").await;
    assert!(t.get("description").is_none(), "{t}");

    // A new expression: kept columns keep their ids and properties, the
    // result's new column is added and the columns it no longer returns
    // are removed; the table and its measure are computed at once.
    let before = table("Products Never Sold").await;
    let out = update(json!({"table": "Products Never Sold",
        "expression": "SELECTCOLUMNS(FILTER(ALL(Products), ISEMPTY(RELATEDTABLE(Orders))), \"Product ID\", Products[Product ID], \"Product\", Products[Product Name], \"Unit Cost\", Products[Unit Cost])"}))
    .await
    .unwrap();
    let id = out["tableId"].as_u64().unwrap();
    assert_eq!(names(&out), ["Product ID", "Product", "Unit Cost"], "{out}");
    assert_eq!(out["addedColumns"], json!(["Unit Cost"]), "{out}");
    assert_eq!(
        out["removedColumns"],
        json!(["Category", "List Price"]),
        "{out}"
    );
    assert_eq!(
        out["recalculated"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| (c["objectId"].clone(), c["status"]["state"].clone()))
            .collect::<Vec<_>>(),
        [(json!(id), json!("ok"))],
        "{out}"
    );
    let after = table("Products Never Sold").await;
    for kept in ["Product ID", "Product"] {
        assert_eq!(column(&after, kept), column(&before, kept), "{kept}");
    }
    let unit_cost = column(&after, "Unit Cost");
    assert_eq!(
        (
            &unit_cost["dataType"],
            &unit_cost["summarizeBy"],
            &unit_cost["sourceColumn"]
        ),
        (
            &column(&table("Products").await, "Unit Cost")["dataType"],
            &json!("sum"),
            &json!("[Unit Cost]")
        ),
        "{unit_cost}"
    );
    assert_eq!(
        number(scalar(&r, &doc, "COUNTROWS('Products Never Sold')").await),
        count
    );
    assert_eq!(
        scalar(&r, &doc, "SUM('Products Never Sold'[Unit Cost])").await,
        scalar(
            &r,
            &doc,
            "SUMX(FILTER(ALL(Products), ISEMPTY(RELATEDTABLE(Orders))), Products[Unit Cost])"
        )
        .await
    );
    assert_eq!(
        number(scalar(&r, &doc, "[Products Never Sold Count]").await),
        count
    );

    // An inferred data type follows the values, and summarization with it.
    let created = r
        .run(
            "model.table.create",
            with_doc(json!({"name": "Regions", "expression": "VALUES(Stores[Region])"})),
        )
        .await;
    let region = created["columns"][0]["columnId"].clone();
    let out = update(json!({"table": "Regions",
        "expression": "SELECTCOLUMNS(VALUES(Stores[Region]), \"Region\", LEN(Stores[Region]))"}))
    .await
    .unwrap();
    assert_eq!(
        out["columns"],
        json!([{"columnId": region, "name": "Region", "dataType": "int64",
            "sourceColumn": "[Region]"}]),
        "{out}"
    );
    assert_eq!(
        column(&table("Regions").await, "Region")["summarizeBy"],
        json!("sum")
    );
    assert_eq!(
        scalar(&r, &doc, "SUM(Regions[Region])").await,
        scalar(
            &r,
            &doc,
            "SUMX(VALUES(Stores[Region]), LEN(Stores[Region]))"
        )
        .await
    );

    // Refusals: no change, a data table's expression, a broken, empty or
    // circular expression, a removed column still used (Date's hierarchy
    // and sort order), a column named like a measure, an unknown table.
    let refused = [
        json!({"table": "Regions"}),
        json!({"table": "Orders", "expression": "VALUES(Stores[Region])"}),
        json!({"table": "Regions", "expression": "VALUES(Stores[Region]"}),
        json!({"table": "Regions", "expression": " "}),
        json!({"table": "Regions", "expression": "VALUES(Stores[Nope])"}),
        json!({"table": "Regions", "expression": "1 + 1"}),
        json!({"table": "Regions", "expression": "ADDCOLUMNS(Regions, \"N\", 1)"}),
        json!({"table": "Date",
            "expression": "CALENDAR(DATE(2024, 1, 1), DATE(2025, 12, 31))"}),
        json!({"table": "Products Never Sold",
            "expression": "ROW(\"Products Never Sold Count\", 1)"}),
    ];
    for bad in refused {
        let e = update(bad.clone()).await.unwrap_err();
        assert_eq!(e.code, OpErrorCode::InvalidParams, "{bad}: {}", e.message);
        if bad["expression"]
            .as_str()
            .is_some_and(|x| x.contains("Regions,"))
        {
            assert!(e.message.contains("circular dependency"), "{}", e.message);
        }
        if bad["table"] == "Date" {
            assert!(
                e.message.contains("[Year], which is used by level 'Year'"),
                "{}",
                e.message
            );
        }
    }
    assert_eq!(
        error_code(update(json!({"table": "Nope", "isHidden": true})).await),
        OpErrorCode::NotFound
    );

    // Undo restores the columns and the values; redo changes them again
    // with the same ids.
    r.run("doc.undo", with_doc(json!({}))).await;
    r.run("doc.undo", with_doc(json!({}))).await;
    r.run("doc.undo", with_doc(json!({}))).await;
    assert_eq!(table("Products Never Sold").await, before);
    assert_eq!(
        number(scalar(&r, &doc, "COUNTROWS('Products Never Sold')").await),
        count
    );
    assert_eq!(
        scalar(&r, &doc, "SUM('Products Never Sold'[List Price])").await,
        scalar(
            &r,
            &doc,
            "SUMX(FILTER(ALL(Products), ISEMPTY(RELATEDTABLE(Orders))), Products[List Price])"
        )
        .await
    );
    r.run("doc.redo", with_doc(json!({}))).await;
    let redone = table("Products Never Sold").await;
    assert_eq!(
        column(&redone, "Unit Cost")["sourceColumn"],
        json!("[Unit Cost]")
    );
    r.run("doc.undo", with_doc(json!({}))).await;

    // In a group the expression's columns merge with the table as the
    // earlier edits leave it.
    let out = r
        .run(
            "doc.applyEdits",
            with_doc(json!({"edits": [
                {"type": "updateColumn", "table": "Products Never Sold",
                    "column": "Product", "description": "The product's name"},
                {"type": "updateTable", "tableId": id, "description": "Never ordered",
                    "expression": "SELECTCOLUMNS(FILTER(ALL(Products), ISEMPTY(RELATEDTABLE(Orders))), \"Product ID\", Products[Product ID], \"Product\", Products[Product Name])"}
            ]})),
        )
        .await;
    assert_eq!(out["changed"], json!(true), "{out}");
    let t = table("Products Never Sold").await;
    assert_eq!(names(&t), ["Product ID", "Product"], "{t}");
    assert_eq!(
        (&t["description"], &column(&t, "Product")["description"]),
        (&json!("Never ordered"), &json!("The product's name")),
        "{t}"
    );
    r.run("doc.undo", with_doc(json!({}))).await;
    assert_eq!(table("Products Never Sold").await, before);
}

#[tokio::test(flavor = "multi_thread")]
async fn open_and_detect_name_a_missing_input_once() {
    let runner = Runner::new();
    for id in ["doc.open", "doc.detect"] {
        let e = runner
            .try_run(id, json!({"path": "no/such.pbip"}))
            .await
            .unwrap_err();
        assert_eq!(e.code, OpErrorCode::NotFound, "{id}");
        assert_eq!(
            e.message, "cannot open no/such.pbip: no such file or directory",
            "{id}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn lint_findings_carry_the_object_id() {
    let dir = tmp_dir("lint-object");
    let pbip = common::mini_pbip(dir.path());
    let runner = Runner::new();
    let doc = runner
        .open(json!({"path": pbip, "autoRefresh": false}))
        .await;
    runner
        .run(
            "model.measure.create",
            json!({"docId": doc, "table": "Sales", "name": "Broken", "expression": "SUM(Sales[Nope])"}),
        )
        .await;
    let measures = runner.run("model.measures", json!({"docId": doc})).await;
    let id = &measures["measures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "Broken")
        .unwrap()["id"];
    let lint = runner.run("doc.lint", json!({"docId": doc})).await;
    let finding = lint["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["message"].as_str().unwrap_or_default().contains("Nope"))
        .unwrap_or_else(|| panic!("no finding for Sales[Nope]: {lint}"));
    assert_eq!(&finding["object"], id, "{finding}");
}
