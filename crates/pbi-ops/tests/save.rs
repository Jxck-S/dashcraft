//! `doc.save` through `Registry::execute` on a temporary PBIP project:
//! only changed files are written, renames follow into the report files
//! and back byte for byte, a reopen shows the saved document, and files
//! changed on disk are a conflict until `overwrite`. `doc.saveAs` writes a
//! new project from any source and moves the document there.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::sync::Arc;

use common::{
    ACME, MINI_TOTAL, MS_PBIX, PBIP, Runner, copy_dir, expected, fixture, mini_pbip, ms_expected,
    tmp_dir,
};
use pbi_ops::OpErrorCode;
use pbi_session::{SequentialIdentifiers, SessionOptions};
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
    assert!(err.message.contains("doc.saveAs"), "{err:?}");

    // Save As makes it a project that saves in place.
    let out = r
        .run(
            "doc.saveAs",
            json!({"docId": doc, "outDir": dir.path().join("project")}),
        )
        .await;
    assert_eq!(out["previousFormat"], "semanticModelFolder", "{out}");
    let measures = r.run("model.measures", json!({"docId": doc})).await;
    let total = measures["measures"][0]["id"].as_u64().unwrap();
    r.run(
        "model.object.setDescription",
        json!({"docId": doc, "objectId": total, "text": "Sum of all amounts"}),
    )
    .await;
    let saved = r.run("doc.save", json!({"docId": doc})).await;
    assert_eq!(strings(&saved["written"]), [SALES_TMDL], "{saved}");
}

const VISUALS: &str = "Mini.Report/definition/pages/overview/visuals";

/// The `x`, `y`, `width` and `height` stored in a `visual.json`.
fn rect_in(bytes: &[u8]) -> [f64; 4] {
    let v: Value = serde_json::from_slice(bytes).unwrap();
    ["x", "y", "width", "height"].map(|k| v["position"][k].as_f64().unwrap())
}

/// The `x`, `y`, `width` and `height` stored in a visual's file.
fn stored_rect(root: &Path, visual: &str) -> [f64; 4] {
    rect_in(&fs::read(root.join(format!("{VISUALS}/{visual}/visual.json"))).unwrap())
}

#[tokio::test(flavor = "multi_thread")]
async fn visual_groups_move_and_resize_with_their_members() {
    let dir = tmp_dir("save-group-");
    let pbip = mini_pbip(dir.path());
    // A group around the card (stored relative to the group, as Power BI
    // writes it) and the slicer (stored in page units).
    let group = json!({
        "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/visualContainer/2.4.0/schema.json",
        "name": "grp",
        "position": {"x": 10, "y": 50, "z": 0, "width": 620, "height": 110},
        "visualGroup": {"displayName": "Header", "groupMode": "ScaleMode"}
    });
    fs::create_dir_all(dir.path().join(format!("{VISUALS}/grp"))).unwrap();
    fs::write(
        dir.path().join(format!("{VISUALS}/grp/visual.json")),
        serde_json::to_string_pretty(&group).unwrap(),
    )
    .unwrap();
    for (visual, x, y) in [("total", 0.0, 10.0), ("region", 440.0, 60.0)] {
        let path = dir.path().join(format!("{VISUALS}/{visual}/visual.json"));
        let mut v: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        v["parentGroupName"] = json!("grp");
        v["position"]["x"] = json!(x);
        v["position"]["y"] = json!(y);
        fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    }
    let original = files(dir.path());
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    let rendered = r.run("render.page", json!({"docId": doc})).await;
    assert_eq!(rendered["ok"], true, "{}", rendered["visuals"]);

    // Moving the group carries the slicer; the card's offsets stay.
    let moved = r
        .run(
            "report.visual.move",
            json!({"docId": doc, "visual": "grp", "x": 0, "y": 20}),
        )
        .await;
    assert_eq!(moved["members"].as_array().unwrap().len(), 1, "{moved}");
    assert_eq!(moved["members"][0]["visual"], "region");
    assert_eq!(
        (
            &moved["members"][0]["position"]["x"],
            &moved["members"][0]["position"]["y"]
        ),
        (&json!(430.0), &json!(30.0))
    );

    // Halving the group scales both members; then a batched move and
    // resize of the same group compose.
    let resized = r
        .run(
            "report.visual.resize",
            json!({"docId": doc, "visual": "grp", "width": 310, "height": 55}),
        )
        .await;
    let mut members: Vec<&str> = resized["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["visual"].as_str().unwrap())
        .collect();
    members.sort_unstable();
    assert_eq!(members, ["region", "total"], "{resized}");
    let batch = r
        .run(
            "doc.applyEdits",
            json!({"docId": doc, "edits": [
                {"type": "setVisualPosition", "visual": "grp", "x": 100},
                {"type": "setVisualPosition", "visual": "grp", "width": 620}
            ]}),
        )
        .await;
    assert_eq!(batch["revision"], 3, "{batch}");

    let out = r.run("doc.save", json!({"docId": doc})).await;
    let mut written = strings(&out["written"]);
    written.sort_unstable();
    assert_eq!(
        written,
        ["grp", "region", "total"].map(|v| format!("{VISUALS}/{v}/visual.json")),
        "{out}"
    );
    let root = dir.path();
    assert_eq!(stored_rect(root, "grp"), [100.0, 20.0, 620.0, 55.0]);
    assert_eq!(stored_rect(root, "total"), [0.0, 5.0, 180.0, 45.0]);
    // 430 → 215 (halved at x 0) → 315 (moved by 100) → 530 (doubled at 100).
    assert_eq!(stored_rect(root, "region"), [530.0, 25.0, 190.0, 45.0]);
    let fresh = Runner::new();
    let reopened = fresh
        .open(json!({"path": pbip, "autoRefresh": false}))
        .await;
    let page = fresh.run("render.page", json!({"docId": reopened})).await;
    assert_eq!(page["ok"], true, "{}", page["visuals"]);

    // Three undo steps restore every position.
    for _ in 0..3 {
        r.run("doc.undo", json!({"docId": doc})).await;
    }
    r.run("doc.save", json!({"docId": doc})).await;
    for visual in ["grp", "total", "region"] {
        let path = format!("{VISUALS}/{visual}/visual.json");
        assert_eq!(
            stored_rect(root, visual),
            rect_in(&original[&path]),
            "{visual}"
        );
    }
}

/// `path` relative to the current folder (`..` up to the root, then down;
/// both are on the same drive here).
fn relative_to_cwd(path: &Path) -> std::path::PathBuf {
    use std::path::Component;
    let normal = |p: &Path| -> Vec<std::ffi::OsString> {
        p.components()
            .filter_map(|c| match c {
                Component::Normal(n) => Some(n.to_owned()),
                _ => None,
            })
            .collect()
    };
    let up = normal(&std::env::current_dir().unwrap()).len();
    let mut rel: std::path::PathBuf = std::iter::repeat_n("..", up).collect();
    rel.extend(normal(path));
    rel
}

/// The files of `out["files"]`, sorted.
fn sorted(v: &Value) -> Vec<&str> {
    let mut files = strings(v);
    files.sort_unstable();
    files
}

#[tokio::test(flavor = "multi_thread")]
async fn save_as_moves_the_document_to_a_new_project() {
    let dir = tmp_dir("save-as-");
    let source = dir.path().join("source");
    let pbip = mini_pbip(&source);
    let original = files(&source);
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    let first = total_expression(&r, &doc).await;
    let measures = r.run("model.measures", json!({"docId": doc})).await;
    let total = measures["measures"][0]["id"].as_u64().unwrap();
    r.run(
        "model.setMeasureExpression",
        json!({"docId": doc, "measureId": total, "expression": "SUM(Sales[Amount]) * 2"}),
    )
    .await;
    r.run(
        "model.object.rename",
        json!({"docId": doc, "table": "Sales", "newName": "Sales Data"}),
    )
    .await;

    let copy = dir.path().join("copy");
    let out = r
        .run("doc.saveAs", json!({"docId": doc, "outDir": copy}))
        .await;
    assert_eq!(
        (
            &out["pbipFile"],
            &out["previousFormat"],
            &out["revision"],
            &out["dirty"],
            &out["warnings"]
        ),
        (
            &json!("Mini.pbip"),
            &json!("pbip"),
            &json!(2),
            &json!(false),
            &json!([])
        ),
        "{out}"
    );
    let written = files(&copy);
    assert_eq!(
        sorted(&out["files"]),
        written.keys().map(String::as_str).collect::<Vec<_>>()
    );
    assert_eq!(
        out["bytes"],
        json!(written.values().map(Vec::len).sum::<usize>())
    );
    assert!(
        written.contains_key(SALES_DATA_TMDL) && !written.contains_key(SALES_TMDL),
        "{:?}",
        written.keys()
    );
    let project_dir = out["projectDir"].as_str().unwrap();
    assert!(
        Path::new(project_dir).is_absolute() && !project_dir.starts_with(r"\\?\"),
        "{project_dir}"
    );
    assert_eq!(files(&source), original, "the source is untouched");
    let history = r.run("doc.history", json!({"docId": doc})).await;
    assert_eq!(
        (&history["dirty"], &history["canUndo"]),
        (&json!(false), &json!(true)),
        "the history survives the Save As"
    );
    let list = r.run("doc.list", json!({})).await;
    assert_eq!(
        (
            &list["documents"][0]["path"],
            &list["documents"][0]["format"]
        ),
        (
            &json!(copy.join("Mini.pbip").to_string_lossy()),
            &json!("pbip")
        ),
        "{list}"
    );

    // Later saves write to the new project only.
    r.run(
        "report.visual.move",
        json!({"docId": doc, "page": "Overview", "visual": "total", "x": 20}),
    )
    .await;
    let out = r.run("doc.save", json!({"docId": doc})).await;
    assert_eq!(strings(&out["written"]), [TOTAL_CARD], "{out}");
    assert_eq!(out["projectDir"], json!(project_dir));
    assert_eq!(files(&source), original);

    // A fresh workspace opens the new project with the edits.
    let fresh = Runner::new();
    let reopened = fresh.open(json!({"path": copy.join("Mini.pbip")})).await;
    assert_eq!(
        total_expression(&fresh, &reopened).await,
        "SUM('Sales Data'[Amount]) * 2"
    );
    assert_eq!(
        scalar(&fresh, &reopened, "[Total]").await,
        json!(2 * MINI_TOTAL)
    );
    let card: Value = serde_json::from_slice(&files(&copy)[TOTAL_CARD]).unwrap();
    assert_eq!(card["position"]["x"], json!(20));

    // Undo reaches back past the Save As, and the undo is saved there too.
    for _ in 0..3 {
        r.run("doc.undo", json!({"docId": doc})).await;
    }
    let out = r.run("doc.save", json!({"docId": doc})).await;
    assert_eq!(strings(&out["deleted"]), [SALES_DATA_TMDL], "{out}");
    let fresh = Runner::new();
    let reopened = fresh.open(json!({"path": copy.join("Mini.pbip")})).await;
    assert_eq!(total_expression(&fresh, &reopened).await, first);
    assert_eq!(
        scalar(&fresh, &reopened, "[Total]").await,
        json!(MINI_TOTAL)
    );

    // A relative folder is reported as the absolute folder doc.save uses.
    let relative = relative_to_cwd(&copy);
    assert!(relative.is_relative(), "{}", relative.display());
    let out = r
        .run(
            "doc.saveAs",
            json!({"docId": doc, "outDir": relative, "overwrite": true}),
        )
        .await;
    assert_eq!(out["projectDir"], json!(project_dir), "{out}");
    let again = r.run("doc.save", json!({"docId": doc})).await;
    assert_eq!(again["projectDir"], json!(project_dir), "{again}");

    // A folder with files needs overwrite.
    let err = r
        .try_run("doc.saveAs", json!({"docId": doc, "outDir": source}))
        .await
        .unwrap_err();
    assert_eq!(err.code, OpErrorCode::Io, "{err:?}");
    assert_eq!(files(&source), original);
    let out = r
        .run(
            "doc.saveAs",
            json!({"docId": doc, "outDir": source, "overwrite": true}),
        )
        .await;
    assert_eq!(out["dirty"], false, "{out}");
    let list = r.run("doc.list", json!({})).await;
    assert_eq!(
        list["documents"][0]["path"],
        json!(source.join("Mini.pbip").to_string_lossy())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn save_as_from_a_pbix_keeps_the_loaded_data() {
    let (Some(pbix), Some(facts)) = (fixture(MS_PBIX), ms_expected(MS_PBIX)) else {
        return;
    };
    let tables = facts["model"]["tables"].as_array().unwrap();
    let largest = tables
        .iter()
        .max_by_key(|t| t["rows"].as_u64().unwrap())
        .unwrap();
    let count = format!("COUNTROWS('{}')", largest["name"].as_str().unwrap());
    let r = Runner::new();
    let doc = r.open(json!({"path": pbix})).await;
    assert_eq!(scalar(&r, &doc, &count).await, largest["rows"]);

    let dir = tmp_dir("save-as-pbix-");
    let project = dir.path().join("project");
    let out = r
        .run("doc.saveAs", json!({"docId": doc, "outDir": project}))
        .await;
    assert_eq!(out["previousFormat"], "pbix", "{out}");
    let warnings = out["warnings"].to_string();
    assert!(warnings.contains("imported data"), "{warnings}");
    assert!(
        project.join(out["pbipFile"].as_str().unwrap()).is_file(),
        "{out}"
    );
    assert_eq!(
        scalar(&r, &doc, &count).await,
        largest["rows"],
        "the loaded data stays in memory"
    );
    let list = r.run("doc.list", json!({})).await;
    let summary = &list["documents"][0];
    assert_eq!(
        (&summary["format"], &summary["embeddedData"]),
        (&json!("pbip"), &json!(false)),
        "{summary}"
    );
    let again = r.run("doc.save", json!({"docId": doc})).await;
    assert_eq!(
        (&again["written"], &again["deleted"]),
        (&json!([]), &json!([])),
        "nothing changed since the Save As: {again}"
    );

    let fresh = Runner::new();
    let reopened = fresh
        .run(
            "doc.open",
            json!({"path": project.join(out["pbipFile"].as_str().unwrap()), "autoRefresh": false}),
        )
        .await;
    assert_eq!(reopened["tables"], json!(tables.len()), "{reopened}");
    assert_eq!(reopened["format"], "pbip");
}

/// The lineage tag of the measure `name` in the document's model.
fn measure_tag(r: &Runner, doc: &str, name: &str) -> Option<String> {
    let document = r.ws.get(doc).unwrap().session().document();
    document
        .model
        .as_ref()
        .unwrap()
        .tables
        .iter()
        .flat_map(|t| &t.measures)
        .find(|m| m.name == name)
        .unwrap_or_else(|| panic!("no measure {name}"))
        .lineage_tag
        .clone()
}

/// New measures get a lineage tag from the session's identifier generator
/// when the model uses lineage tags (ADR-0014); undo/redo, the save and a
/// reopen keep it.
#[tokio::test(flavor = "multi_thread")]
async fn new_measures_get_lineage_tags_from_the_injected_generator() {
    const ORDERS_TMDL: &str = "AcmeOutfitters.SemanticModel/definition/tables/Orders.tmdl";
    const FIRST: &str = "0000acce-0000-4000-8000-000000000001";
    let Some(acme) = fixture(ACME) else {
        eprintln!("SKIP: {ACME} missing");
        return;
    };
    let dir = tmp_dir("tags-");
    copy_dir(&acme, dir.path());
    let pbip = dir.path().join("AcmeOutfitters.pbip");
    let r = Runner::new();
    r.ws.set_open_hook(Some(Arc::new(|o: &mut SessionOptions| {
        o.identifiers = Some(Arc::new(SequentialIdentifiers::new(0xacce)));
    })));
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    r.run(
        "model.measure.create",
        json!({"docId": doc, "table": "Orders", "name": "Tag Check", "expression": "COUNTROWS(Orders)"}),
    )
    .await;
    assert_eq!(measure_tag(&r, &doc, "Tag Check").as_deref(), Some(FIRST));
    r.run("doc.undo", json!({"docId": doc})).await;
    r.run("doc.redo", json!({"docId": doc})).await;
    assert_eq!(
        measure_tag(&r, &doc, "Tag Check").as_deref(),
        Some(FIRST),
        "redo keeps the tag"
    );
    r.run(
        "doc.applyEdits",
        json!({"docId": doc, "edits": [
            {"type": "createMeasure", "table": "Orders", "name": "Tag Check A", "expression": "1"},
            {"type": "createMeasure", "table": "Orders", "name": "Tag Check B", "expression": "2"}
        ]}),
    )
    .await;
    assert_eq!(
        [
            measure_tag(&r, &doc, "Tag Check A"),
            measure_tag(&r, &doc, "Tag Check B")
        ],
        [
            Some("0000acce-0000-4000-8000-000000000002".to_owned()),
            Some("0000acce-0000-4000-8000-000000000003".to_owned())
        ]
    );

    let out = r.run("doc.save", json!({"docId": doc})).await;
    assert_eq!(strings(&out["written"]), [ORDERS_TMDL], "{out}");
    let orders = fs::read_to_string(dir.path().join(ORDERS_TMDL)).unwrap();
    let at = orders
        .find("\tmeasure 'Tag Check' = ")
        .expect("the new measure");
    let block = &orders[at..];
    let block = &block[..block[1..]
        .find("\n\tmeasure ")
        .map_or(block.len(), |i| i + 1)];
    assert!(
        block.contains(&format!("\n\t\tlineageTag: {FIRST}\n")),
        "{block}"
    );
    let out = r.run("doc.save", json!({"docId": doc})).await;
    assert_eq!(out["written"], json!([]), "saving again writes nothing");

    let fresh = Runner::new();
    let reopened = fresh
        .open(json!({"path": pbip, "autoRefresh": false}))
        .await;
    assert_eq!(
        measure_tag(&fresh, &reopened, "Tag Check").as_deref(),
        Some(FIRST)
    );
}

/// A model without lineage tags gets none (no format upgrade); otherwise the
/// default generator gives random version 4 GUIDs.
#[tokio::test(flavor = "multi_thread")]
async fn lineage_tags_are_random_and_only_where_the_model_uses_them() {
    let dir = tmp_dir("notags-");
    let pbip = mini_pbip(dir.path());
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    r.run(
        "model.measure.create",
        json!({"docId": doc, "table": "Sales", "name": "Count", "expression": "COUNTROWS(Sales)"}),
    )
    .await;
    assert_eq!(measure_tag(&r, &doc, "Count"), None);
    r.run("doc.save", json!({"docId": doc})).await;
    let sales = fs::read_to_string(dir.path().join(SALES_TMDL)).unwrap();
    assert!(
        sales.contains("measure Count") && !sales.contains("lineageTag"),
        "{sales}"
    );

    let Some(acme) = fixture(PBIP) else {
        eprintln!("SKIP: {PBIP} missing");
        return;
    };
    let doc = r.open(json!({"path": acme, "autoRefresh": false})).await;
    let mut tags = Vec::new();
    for name in ["Tag Check A", "Tag Check B"] {
        r.run(
            "model.measure.create",
            json!({"docId": doc, "table": "Orders", "name": name, "expression": "1"}),
        )
        .await;
        tags.push(measure_tag(&r, &doc, name).unwrap());
    }
    assert_ne!(tags[0], tags[1]);
    for tag in &tags {
        let groups: Vec<usize> = tag.split('-').map(str::len).collect();
        assert_eq!(groups, [8, 4, 4, 4, 12], "{tag}");
        assert!(
            tag.as_bytes()[14] == b'4'
                && tag
                    .chars()
                    .all(|c| c == '-' || c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "a lowercase version 4 GUID: {tag}"
        );
    }
}

/// Relationship create and delete save into `relationships.tmdl` only: a new
/// relationship is named by the session's identifier generator, a reopen
/// shows it, and undoing both edits saves the original bytes again.
#[tokio::test(flavor = "multi_thread")]
async fn relationship_edits_save_and_reopen() {
    const RELATIONSHIPS_TMDL: &str = "AcmeOutfitters.SemanticModel/definition/relationships.tmdl";
    const FIRST: &str = "0000acce-0000-4000-8000-000000000001";
    const STORES: &str = "d4926a14-f8ee-5523-be56-0dd865b87eee";
    let Some(acme) = fixture(ACME) else {
        eprintln!("SKIP: {ACME} missing");
        return;
    };
    let dir = tmp_dir("relationships-");
    copy_dir(&acme, dir.path());
    let pbip = dir.path().join("AcmeOutfitters.pbip");
    let original = fs::read(dir.path().join(RELATIONSHIPS_TMDL)).unwrap();
    let r = Runner::new();
    r.ws.set_open_hook(Some(Arc::new(|o: &mut SessionOptions| {
        o.identifiers = Some(Arc::new(SequentialIdentifiers::new(0xacce)));
    })));
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    r.run(
        "model.relationship.delete",
        json!({"docId": doc, "name": STORES}),
    )
    .await;
    let created = r
        .run(
            "model.relationship.create",
            json!({"docId": doc, "from": {"table": "Orders", "column": "Store ID"}, "to": {"table": "Stores", "column": "Store ID"}}),
        )
        .await;
    assert_eq!(created["name"], FIRST, "{created}");

    let out = r.run("doc.save", json!({"docId": doc})).await;
    assert_eq!(strings(&out["written"]), [RELATIONSHIPS_TMDL], "{out}");
    let saved = fs::read_to_string(dir.path().join(RELATIONSHIPS_TMDL)).unwrap();
    assert!(!saved.contains(STORES), "{saved}");
    assert!(
        saved.contains(&format!(
            "relationship {FIRST}\n\tfromColumn: Orders.'Store ID'\n\ttoColumn: Stores.'Store ID'\n"
        )),
        "{saved}"
    );
    assert_eq!(saved.matches("\nrelationship ").count() + 1, 8, "{saved}");
    assert!(
        saved.contains("\tfromCardinality: one\n\ttoCardinality: one\n"),
        "untouched relationships keep their text: {saved}"
    );

    let fresh = Runner::new();
    let reopened = fresh
        .open(json!({"path": pbip, "autoRefresh": false}))
        .await;
    let relationships = fresh
        .run("model.relationships", json!({"docId": reopened}))
        .await;
    let names: Vec<&str> = relationships["relationships"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 8);
    assert!(
        names.contains(&FIRST) && !names.contains(&STORES),
        "{names:?}"
    );

    r.run("doc.undo", json!({"docId": doc})).await;
    r.run("doc.undo", json!({"docId": doc})).await;
    let out = r.run("doc.save", json!({"docId": doc})).await;
    assert_eq!(strings(&out["written"]), [RELATIONSHIPS_TMDL], "{out}");
    assert_eq!(
        fs::read(dir.path().join(RELATIONSHIPS_TMDL)).unwrap(),
        original,
        "undoing both edits saves the original bytes"
    );
}

/// Relationship updates save into `relationships.tmdl` only: changed
/// properties are written (a default one is left out), untouched
/// relationships keep their text, a reopen shows the same relationships, and
/// undoing saves the original bytes again.
#[tokio::test(flavor = "multi_thread")]
async fn relationship_updates_save_and_reopen() {
    const RELATIONSHIPS_TMDL: &str = "AcmeOutfitters.SemanticModel/definition/relationships.tmdl";
    const CUSTOMERS: &str = "c79d4066-724d-5669-b57b-f6bf11da6f40";
    const STORES: &str = "d4926a14-f8ee-5523-be56-0dd865b87eee";
    const ORDER_DATE: &str = "5fd8dd36-8b63-5abf-b8b9-50b8de54f509";
    const SHIP_DATE: &str = "976ded3b-d763-50ba-be93-faf5d31cd63c";
    let Some(acme) = fixture(ACME) else {
        eprintln!("SKIP: {ACME} missing");
        return;
    };
    let dir = tmp_dir("relationship-updates-");
    copy_dir(&acme, dir.path());
    let pbip = dir.path().join("AcmeOutfitters.pbip");
    let original = fs::read(dir.path().join(RELATIONSHIPS_TMDL)).unwrap();
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    r.run(
        "model.relationship.update",
        json!({"docId": doc, "name": CUSTOMERS, "crossFilteringBehavior": "oneDirection"}),
    )
    .await;
    r.run(
        "doc.applyEdits",
        json!({"docId": doc, "edits": [
            {"type": "updateRelationship", "name": ORDER_DATE, "isActive": false},
            {"type": "updateRelationship", "name": SHIP_DATE, "isActive": true}
        ]}),
    )
    .await;
    r.run(
        "model.relationship.update",
        json!({"docId": doc, "name": STORES, "toCardinality": "many", "crossFilteringBehavior": "bothDirections", "securityFilteringBehavior": "bothDirections"}),
    )
    .await;
    let edited = r.run("model.relationships", json!({"docId": doc})).await;

    let out = r.run("doc.save", json!({"docId": doc})).await;
    assert_eq!(strings(&out["written"]), [RELATIONSHIPS_TMDL], "{out}");
    let saved = fs::read_to_string(dir.path().join(RELATIONSHIPS_TMDL)).unwrap();
    for block in [
        format!(
            "relationship {CUSTOMERS}\n\tfromColumn: Orders.'Customer ID'\n\ttoColumn: Customers.'Customer ID'\n\n"
        ),
        format!(
            "relationship {STORES}\n\tcrossFilteringBehavior: bothDirections\n\tsecurityFilteringBehavior: bothDirections\n\ttoCardinality: many\n\tfromColumn: Orders.'Store ID'\n\ttoColumn: Stores.'Store ID'\n\n"
        ),
        format!(
            "relationship {ORDER_DATE}\n\tisActive: false\n\tfromColumn: Orders.'Order Date'\n\ttoColumn: Date.Date\n\n"
        ),
        format!(
            "relationship {SHIP_DATE}\n\tfromColumn: Orders.'Ship Date'\n\ttoColumn: Date.Date\n\n"
        ),
        "\tfromCardinality: one\n\ttoCardinality: one\n".to_owned(),
    ] {
        assert!(saved.contains(&block), "{block:?} missing from {saved}");
    }

    let fresh = Runner::new();
    let reopened = fresh
        .open(json!({"path": pbip, "autoRefresh": false}))
        .await;
    assert_eq!(
        fresh
            .run("model.relationships", json!({"docId": reopened}))
            .await,
        edited
    );

    for _ in 0..3 {
        r.run("doc.undo", json!({"docId": doc})).await;
    }
    let out = r.run("doc.save", json!({"docId": doc})).await;
    assert_eq!(strings(&out["written"]), [RELATIONSHIPS_TMDL], "{out}");
    assert_eq!(
        fs::read(dir.path().join(RELATIONSHIPS_TMDL)).unwrap(),
        original,
        "undoing the updates saves the original bytes"
    );
}

/// Deleting a calculated column and a table saves as removed TMDL: the
/// column's block leaves its table file; the table's file is deleted with
/// its `ref table` line, relationships and role row filters. A reopen no
/// longer shows them, and undoing both saves the original project again.
#[tokio::test(flavor = "multi_thread")]
async fn column_and_table_deletes_save_and_reopen() {
    const DEF: &str = "AcmeOutfitters.SemanticModel/definition";
    let Some(acme) = fixture(ACME) else {
        eprintln!("SKIP: {ACME} missing");
        return;
    };
    let dir = tmp_dir("deletes-");
    copy_dir(&acme, dir.path());
    let pbip = dir.path().join("AcmeOutfitters.pbip");
    let before = files(dir.path());
    let text = |f: &str| String::from_utf8(before[&format!("{DEF}/{f}")].clone()).unwrap();
    let saved = |f: &str| fs::read_to_string(dir.path().join(DEF).join(f)).unwrap();
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    r.run(
        "model.column.delete",
        json!({"docId": doc, "table": "Orders", "column": "Order Size"}),
    )
    .await;
    r.run(
        "model.table.delete",
        json!({"docId": doc, "table": "Stores"}),
    )
    .await;

    let out = r.run("doc.save", json!({"docId": doc})).await;
    let model_files = [
        "model.tmdl",
        "relationships.tmdl",
        "roles/Region West.tmdl",
        "roles/Store Manager.tmdl",
        "tables/Orders.tmdl",
    ]
    .map(|f| format!("{DEF}/{f}"));
    assert_eq!(sorted(&out["written"]), model_files, "{out}");
    assert_eq!(
        strings(&out["deleted"]),
        [format!("{DEF}/tables/Stores.tmdl")],
        "{out}"
    );
    assert!(!dir.path().join(DEF).join("tables/Stores.tmdl").exists());
    let orders = text("tables/Orders.tmdl");
    let start = orders.find("\tcolumn 'Order Size'").unwrap();
    let end = orders.find("\t/// Context transition").unwrap();
    assert_eq!(
        saved("tables/Orders.tmdl"),
        format!("{}{}", &orders[..start], &orders[end..])
    );
    assert_eq!(
        saved("model.tmdl"),
        text("model.tmdl").replace("ref table Stores\n", "")
    );
    let relationships = saved("relationships.tmdl");
    assert!(!relationships.contains("Stores"), "{relationships}");
    assert_eq!(relationships.matches("\nrelationship ").count() + 1, 6);
    for role in ["roles/Region West.tmdl", "roles/Store Manager.tmdl"] {
        let saved = saved(role);
        assert!(
            !saved.contains("tablePermission") && saved.contains("modelPermission: read"),
            "{saved}"
        );
    }

    let fresh = Runner::new();
    let reopened = fresh
        .open(json!({"path": pbip, "autoRefresh": false}))
        .await;
    let tables = fresh.run("model.tables", json!({"docId": reopened})).await;
    let names: Vec<&str> = tables["tables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 10);
    assert!(!names.contains(&"Stores"), "{names:?}");
    let orders = tables["tables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "Orders")
        .unwrap();
    assert!(!orders.to_string().contains("Order Size"), "{orders}");
    assert!(orders.to_string().contains("Is Returned"), "{orders}");

    r.run("doc.undo", json!({"docId": doc})).await;
    r.run("doc.undo", json!({"docId": doc})).await;
    let out = r.run("doc.save", json!({"docId": doc})).await;
    assert_eq!(sorted(&out["written"]).len(), 6, "{out}");
    assert!(
        files(dir.path()) == before,
        "undoing both deletes saves the original project"
    );
}

#[tokio::test]
async fn created_column_saves_and_reopens() {
    const DEF: &str = "AcmeOutfitters.SemanticModel/definition";
    let Some(acme) = fixture(ACME) else {
        eprintln!("SKIP: {ACME} missing");
        return;
    };
    let dir = tmp_dir("create-column-");
    copy_dir(&acme, dir.path());
    let pbip = dir.path().join("AcmeOutfitters.pbip");
    let before = files(dir.path());
    let text = |f: &str| String::from_utf8(before[&format!("{DEF}/{f}")].clone()).unwrap();
    let saved = |f: &str| fs::read_to_string(dir.path().join(DEF).join(f)).unwrap();
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    let out = r
        .run(
            "model.column.create",
            json!({"docId": doc, "table": "Orders", "name": "Margin",
                "expression": "Orders[Line Amount] - Orders[Line Cost]",
                "dataType": "double", "formatString": "#,0.00", "displayFolder": "Profit"}),
        )
        .await;
    assert_eq!(out["dataTypeInferred"], false, "{out}");

    let out = r.run("doc.save", json!({"docId": doc})).await;
    assert_eq!(
        sorted(&out["written"]),
        [format!("{DEF}/tables/Orders.tmdl")],
        "{out}"
    );
    // Like Power BI Desktop: after the table's columns, a lineage tag, sum
    // for a numeric column and the SummarizationSetBy annotation.
    let orders = saved("tables/Orders.tmdl");
    let tag = orders
        .split("column Margin = ")
        .nth(1)
        .and_then(|block| block.split("lineageTag: ").nth(1))
        .and_then(|rest| rest.lines().next())
        .unwrap();
    assert_eq!(tag.len(), 36, "{tag}");
    let block = format!(
        "\tcolumn Margin = Orders[Line Amount] - Orders[Line Cost]\n\
         \t\tdataType: double\n\
         \t\tformatString: #,0.00\n\
         \t\tdisplayFolder: Profit\n\
         \t\tlineageTag: {tag}\n\
         \t\tsummarizeBy: sum\n\
         \n\
         \t\tannotation SummarizationSetBy = Automatic\n\
         \n"
    );
    let original = text("tables/Orders.tmdl");
    let at = original.find("\tpartition Orders").unwrap();
    assert_eq!(
        orders,
        format!("{}{block}{}", &original[..at], &original[at..])
    );

    let fresh = Runner::new();
    let reopened = fresh
        .open(json!({"path": pbip, "autoRefresh": false}))
        .await;
    let tables = fresh.run("model.tables", json!({"docId": reopened})).await;
    let orders = tables["tables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "Orders")
        .unwrap();
    assert!(orders.to_string().contains("Margin"), "{orders}");

    r.run("doc.undo", json!({"docId": doc})).await;
    r.run("doc.save", json!({"docId": doc})).await;
    assert!(
        files(dir.path()) == before,
        "undoing the create saves the original project"
    );
}

#[tokio::test]
async fn updated_columns_save_and_reopen() {
    const DEF: &str = "AcmeOutfitters.SemanticModel/definition";
    const CUSTOMERS: &str = "tables/Customers.tmdl";
    let Some(acme) = fixture(ACME) else {
        eprintln!("SKIP: {ACME} missing");
        return;
    };
    let dir = tmp_dir("update-column-");
    copy_dir(&acme, dir.path());
    let pbip = dir.path().join("AcmeOutfitters.pbip");
    let before = files(dir.path());
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    let out = r
        .run(
            "doc.applyEdits",
            json!({"docId": doc, "edits": [
                {"type": "updateColumn", "table": "Customers", "column": "Lifetime Sales",
                    "expression": "FORMAT(CALCULATE(SUM(Orders[Line Amount])), \"0\")",
                    "dataType": "string", "formatString": ""},
                {"type": "updateColumn", "table": "Customers", "column": "Customer ID",
                    "isHidden": true},
            ]}),
        )
        .await;
    assert_eq!(out["changed"], true, "{out}");

    let out = r.run("doc.save", json!({"docId": doc})).await;
    assert_eq!(
        sorted(&out["written"]),
        [format!("{DEF}/{CUSTOMERS}")],
        "{out}"
    );
    // The expression and type change in place; an automatic summarization
    // follows the type, like in Power BI Desktop.
    let original = String::from_utf8(before[&format!("{DEF}/{CUSTOMERS}")].clone()).unwrap();
    let edited = [
        (
            "\tcolumn 'Lifetime Sales' = CALCULATE(SUM(Orders[Line Amount]))\n\
             \t\tdataType: double\n\
             \t\tformatString: \\$#,0\n",
            "\tcolumn 'Lifetime Sales' = FORMAT(CALCULATE(SUM(Orders[Line Amount])), \"0\")\n\
             \t\tdataType: string\n",
        ),
        (
            "\t\tlineageTag: d5939a53-f184-57f2-9044-f1673e1ec8fa\n\
             \t\tsummarizeBy: sum\n",
            "\t\tlineageTag: d5939a53-f184-57f2-9044-f1673e1ec8fa\n\
             \t\tsummarizeBy: none\n",
        ),
        (
            "\tcolumn 'Customer ID'\n\
             \t\tdataType: string\n",
            "\tcolumn 'Customer ID'\n\
             \t\tdataType: string\n\
             \t\tisHidden\n",
        ),
    ]
    .iter()
    .fold(original.replace("\r\n", "\n"), |text, (from, to)| {
        assert_eq!(text.matches(from).count(), 1, "{from}");
        text.replace(from, to)
    });
    let saved = fs::read_to_string(dir.path().join(DEF).join(CUSTOMERS)).unwrap();
    assert_eq!(saved.replace("\r\n", "\n"), edited);

    let fresh = Runner::new();
    let reopened = fresh
        .open(json!({"path": pbip, "autoRefresh": false}))
        .await;
    let tables = fresh.run("model.tables", json!({"docId": reopened})).await;
    let column = |name: &str| {
        tables["tables"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "Customers")
            .and_then(|t| t["columns"].as_array())
            .and_then(|c| c.iter().find(|c| c["name"] == name))
            .cloned()
            .unwrap()
    };
    let lifetime = column("Lifetime Sales");
    assert_eq!(
        (
            &lifetime["expression"],
            &lifetime["dataType"],
            &lifetime["summarizeBy"]
        ),
        (
            &json!("FORMAT(CALCULATE(SUM(Orders[Line Amount])), \"0\")"),
            &json!("string"),
            &json!("none")
        )
    );
    assert_eq!(column("Customer ID")["isHidden"], true);

    r.run("doc.undo", json!({"docId": doc})).await;
    r.run("doc.save", json!({"docId": doc})).await;
    assert!(
        files(dir.path()) == before,
        "undoing the update saves the original project"
    );
}

#[tokio::test]
async fn created_and_deleted_hierarchies_save_and_reopen() {
    const DEF: &str = "AcmeOutfitters.SemanticModel/definition";
    let Some(acme) = fixture(ACME) else {
        eprintln!("SKIP: {ACME} missing");
        return;
    };
    let dir = tmp_dir("hierarchies-");
    copy_dir(&acme, dir.path());
    let pbip = dir.path().join("AcmeOutfitters.pbip");
    let before = files(dir.path());
    let text = |f: &str| String::from_utf8(before[&format!("{DEF}/{f}")].clone()).unwrap();
    let saved = |f: &str| fs::read_to_string(dir.path().join(DEF).join(f)).unwrap();
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    r.run(
        "doc.applyEdits",
        json!({"docId": doc, "edits": [
            {"type": "deleteHierarchy", "table": "Stores", "hierarchy": "Geography"},
            {"type": "createHierarchy", "table": "Stores", "name": "Places",
                "levels": [{"column": "Region"}, {"column": "Store Name", "name": "Store"}]}
        ]}),
    )
    .await;
    let out = r.run("doc.save", json!({"docId": doc})).await;
    assert_eq!(
        sorted(&out["written"]),
        [format!("{DEF}/tables/Stores.tmdl")],
        "{out}"
    );
    // Like Power BI Desktop: lineage tags on the hierarchy and its levels,
    // levels in order without ordinals, where Geography was.
    let stores = saved("tables/Stores.tmdl");
    let tags: Vec<&str> = stores
        .split("hierarchy Places")
        .nth(1)
        .unwrap()
        .split("lineageTag: ")
        .skip(1)
        .take(3)
        .map(|rest| rest.lines().next().unwrap())
        .collect();
    assert!(tags.iter().all(|t| t.len() == 36), "{tags:?}");
    let block = format!(
        "\thierarchy Places\n\
         \t\tlineageTag: {}\n\
         \n\
         \t\tlevel Region\n\
         \t\t\tlineageTag: {}\n\
         \t\t\tcolumn: Region\n\
         \n\
         \t\tlevel Store\n\
         \t\t\tlineageTag: {}\n\
         \t\t\tcolumn: 'Store Name'\n\
         \n",
        tags[0], tags[1], tags[2]
    );
    let original = text("tables/Stores.tmdl");
    let (start, end) = (
        original.find("\thierarchy Geography").unwrap(),
        original.find("\tpartition Stores").unwrap(),
    );
    assert_eq!(
        stores,
        format!("{}{block}{}", &original[..start], &original[end..])
    );

    let fresh = Runner::new();
    let reopened = fresh
        .open(json!({"path": pbip, "autoRefresh": false}))
        .await;
    let out = fresh
        .run(
            "model.hierarchy.delete",
            json!({"docId": reopened, "table": "Stores", "hierarchy": "Places", "dryRun": true}),
        )
        .await;
    assert_eq!(out["name"], json!("Places"), "{out}");
    let gone = fresh
        .try_run(
            "model.hierarchy.delete",
            json!({"docId": reopened, "table": "Stores", "hierarchy": "Geography"}),
        )
        .await;
    assert_eq!(gone.unwrap_err().code, OpErrorCode::NotFound);
}

#[tokio::test]
async fn created_table_saves_and_reopens() {
    const DEF: &str = "AcmeOutfitters.SemanticModel/definition";
    let Some(acme) = fixture(ACME) else {
        eprintln!("SKIP: {ACME} missing");
        return;
    };
    let dir = tmp_dir("create-table-");
    copy_dir(&acme, dir.path());
    let pbip = dir.path().join("AcmeOutfitters.pbip");
    let before = files(dir.path());
    let text = |f: &str| String::from_utf8(before[&format!("{DEF}/{f}")].clone()).unwrap();
    let saved = |f: &str| fs::read_to_string(dir.path().join(DEF).join(f)).unwrap();
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    r.run(
        "model.table.create",
        json!({"docId": doc, "name": "Store Places",
            "expression": "SUMMARIZE(Stores, Stores[Region], Stores[Square Feet])"}),
    )
    .await;
    let out = r.run("doc.save", json!({"docId": doc})).await;
    assert_eq!(
        sorted(&out["written"]),
        [
            format!("{DEF}/model.tmdl"),
            format!("{DEF}/tables/Store Places.tmdl")
        ],
        "{out}"
    );
    // Like Power BI Desktop: lineage tags, inferred names and types, the
    // result's columns as source columns, sum for a numeric column, and a
    // calculated partition named after the table.
    let table = saved("tables/Store Places.tmdl");
    let tags: Vec<&str> = table
        .split("lineageTag: ")
        .skip(1)
        .map(|rest| rest.lines().next().unwrap())
        .collect();
    assert!(
        tags.len() == 3 && tags.iter().all(|t| t.len() == 36),
        "{tags:?}"
    );
    assert_eq!(
        table,
        format!(
            "table 'Store Places'\n\
             \tlineageTag: {}\n\
             \n\
             \tcolumn Region\n\
             \t\tdataType: string\n\
             \t\tlineageTag: {}\n\
             \t\tsummarizeBy: none\n\
             \t\tisNameInferred\n\
             \t\tisDataTypeInferred\n\
             \t\tsourceColumn: Stores[Region]\n\
             \n\
             \t\tannotation SummarizationSetBy = Automatic\n\
             \n\
             \tcolumn 'Square Feet'\n\
             \t\tdataType: int64\n\
             \t\tlineageTag: {}\n\
             \t\tsummarizeBy: sum\n\
             \t\tisNameInferred\n\
             \t\tisDataTypeInferred\n\
             \t\tsourceColumn: Stores[Square Feet]\n\
             \n\
             \t\tannotation SummarizationSetBy = Automatic\n\
             \n\
             \tpartition 'Store Places' = calculated\n\
             \t\tmode: import\n\
             \t\tsource = SUMMARIZE(Stores, Stores[Region], Stores[Square Feet])\n",
            tags[0], tags[1], tags[2]
        )
    );
    let model = text("model.tmdl");
    let last = "ref table 'Last Refreshed'\n";
    let at = model.find(last).unwrap() + last.len();
    assert_eq!(
        saved("model.tmdl"),
        format!("{}ref table 'Store Places'\n{}", &model[..at], &model[at..])
    );

    let fresh = Runner::new();
    let reopened = fresh
        .open(
            json!({"path": pbip, "blobMap": dir.path().join("blob-map.json"),
            "source": "query"}),
        )
        .await;
    let q = fresh
        .run(
            "dax.query",
            json!({"docId": reopened, "query": "EVALUATE ROW(\"a\", COUNTROWS('Store Places'), \"b\", COUNTROWS(SUMMARIZE(Stores, Stores[Region], Stores[Square Feet])))"}),
        )
        .await;
    let row = &q["tables"][0]["rows"][0];
    assert!(row[0].as_i64().unwrap() > 1, "{q}");
    assert_eq!(row[0], row[1], "{q}");

    r.run("doc.undo", json!({"docId": doc})).await;
    r.run("doc.save", json!({"docId": doc})).await;
    assert!(
        files(dir.path()) == before,
        "undoing the create saves the original project"
    );
}

#[tokio::test]
async fn updated_table_saves_and_reopens() {
    const DEF: &str = "AcmeOutfitters.SemanticModel/definition";
    const NEVER_SOLD: &str = "tables/Products Never Sold.tmdl";
    const EXPRESSION: &str = "SELECTCOLUMNS(FILTER(ALL(Products), ISEMPTY(RELATEDTABLE(Orders))), \"Product ID\", Products[Product ID], \"Product\", Products[Product Name], \"Unit Cost\", Products[Unit Cost])";
    let Some(acme) = fixture(ACME) else {
        eprintln!("SKIP: {ACME} missing");
        return;
    };
    let dir = tmp_dir("update-table-");
    copy_dir(&acme, dir.path());
    let pbip = dir.path().join("AcmeOutfitters.pbip");
    let before = files(dir.path());
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    r.run(
        "model.table.update",
        json!({"docId": doc, "table": "Products Never Sold", "expression": EXPRESSION}),
    )
    .await;
    let out = r.run("doc.save", json!({"docId": doc})).await;
    assert_eq!(
        sorted(&out["written"]),
        [format!("{DEF}/{NEVER_SOLD}")],
        "{out}"
    );
    // Kept columns stay as they are; the columns the expression no longer
    // returns go, the new one comes with an inferred name and type, a
    // lineage tag and sum, like a column of a created table.
    let saved = fs::read_to_string(dir.path().join(DEF).join(NEVER_SOLD)).unwrap();
    let tag = saved
        .split("column 'Unit Cost'")
        .nth(1)
        .and_then(|rest| rest.split("lineageTag: ").nth(1))
        .and_then(|rest| rest.lines().next())
        .unwrap();
    assert_eq!(tag.len(), 36, "{saved}");
    let original = String::from_utf8(before[&format!("{DEF}/{NEVER_SOLD}")].clone())
        .unwrap()
        .replace("\r\n", "\n");
    let (columns, partition) = (
        original.find("\tcolumn Category\n").unwrap(),
        original.find("\tpartition ").unwrap(),
    );
    assert!(original[partition..].starts_with("\tpartition 'Products Never Sold' = calculated\n"));
    let edited = format!(
        "{}\tcolumn 'Unit Cost'\n\
         \t\tdataType: decimal\n\
         \t\tlineageTag: {tag}\n\
         \t\tsummarizeBy: sum\n\
         \t\tisNameInferred\n\
         \t\tisDataTypeInferred\n\
         \t\tsourceColumn: [Unit Cost]\n\
         \n\
         \t\tannotation SummarizationSetBy = Automatic\n\
         \n\
         \tpartition 'Products Never Sold' = calculated\n\
         \t\tmode: import\n\
         \t\tsource = {EXPRESSION}\n",
        &original[..columns]
    );
    assert_eq!(saved.replace("\r\n", "\n"), edited);

    let fresh = Runner::new();
    let reopened = fresh
        .open(
            json!({"path": pbip, "blobMap": dir.path().join("blob-map.json"),
            "source": "query"}),
        )
        .await;
    let q = fresh
        .run(
            "dax.query",
            json!({"docId": reopened, "query": "EVALUATE ROW(\"a\", SUM('Products Never Sold'[Unit Cost]), \"b\", SUMX(FILTER(ALL(Products), ISEMPTY(RELATEDTABLE(Orders))), Products[Unit Cost]), \"c\", [Products Never Sold Count])"}),
        )
        .await;
    let row = &q["tables"][0]["rows"][0];
    assert!(row[2].as_i64().unwrap() > 0, "{q}");
    assert_eq!(row[0], row[1], "{q}");

    r.run("doc.undo", json!({"docId": doc})).await;
    r.run("doc.save", json!({"docId": doc})).await;
    assert!(
        files(dir.path()) == before,
        "undoing the update saves the original project"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn renamed_queries_save_and_reopen() {
    const DEF: &str = "AcmeOutfitters.SemanticModel/definition";
    let (Some(acme), Some(exp)) = (fixture(ACME), expected()) else {
        eprintln!("SKIP: {ACME} missing");
        return;
    };
    let dir = tmp_dir("rename-query-");
    copy_dir(&acme, dir.path());
    let pbip = dir.path().join("AcmeOutfitters.pbip");
    let before = files(dir.path());
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    for (old, new) in [
        ("StorageContainerUrl", "Storage URL"),
        ("fnCleanKey", "Clean Key"),
    ] {
        r.run(
            "model.object.rename",
            json!({"docId": doc, "expression": old, "newName": new}),
        )
        .await;
    }
    let out = r.run("doc.save", json!({"docId": doc})).await;
    let tables = [
        "Customers",
        "Orders",
        "Products",
        "Returns",
        "Stores",
        "Targets",
        "Web Sessions",
    ];
    let mut expected_files: Vec<String> = ["expressions.tmdl", "model.tmdl"]
        .iter()
        .map(|f| format!("{DEF}/{f}"))
        .chain(tables.iter().map(|t| format!("{DEF}/tables/{t}.tmdl")))
        .collect();
    expected_files.sort();
    assert_eq!(sorted(&out["written"]), expected_files, "{out}");

    // Only the names change: the declarations, the M references and the
    // query order.
    let text = |bytes: &[u8]| {
        String::from_utf8(bytes.to_vec())
            .unwrap()
            .replace("\r\n", "\n")
    };
    for path in &expected_files {
        let original = text(&before[path]);
        let edited = if path.ends_with("/expressions.tmdl") {
            original
                .replace(
                    "expression StorageContainerUrl = ",
                    "expression 'Storage URL' = ",
                )
                .replace("expression fnCleanKey = ", "expression 'Clean Key' = ")
        } else if path.ends_with("/model.tmdl") {
            original.replace(
                r#""StorageContainerUrl","fnCleanKey""#,
                r#""Storage URL","Clean Key""#,
            )
        } else {
            original
                .replace("StorageContainerUrl", "#\"Storage URL\"")
                .replace("fnCleanKey", "#\"Clean Key\"")
        };
        assert_ne!(edited, original, "{path}");
        let saved = text(&fs::read(dir.path().join(path)).unwrap());
        assert_eq!(saved, edited, "{path}");
    }

    let fresh = Runner::new();
    let reopened = fresh
        .open(
            json!({"path": pbip, "blobMap": dir.path().join("blob-map.json"),
            "source": "query", "autoRefresh": false}),
        )
        .await;
    let refresh = fresh.run("data.refresh", json!({"docId": reopened})).await;
    assert_eq!(refresh["ok"], true, "{refresh}");
    for (name, rows) in exp.row_counts() {
        let t = refresh["tables"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap();
        assert_eq!(t["rows"], rows, "{name}");
    }

    r.run("doc.undo", json!({"docId": doc})).await;
    r.run("doc.undo", json!({"docId": doc})).await;
    r.run("doc.save", json!({"docId": doc})).await;
    assert!(
        files(dir.path()) == before,
        "undoing the renames saves the original project"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn renames_rewrite_report_level_measures_and_save_them() {
    const EXT: &str = "Mini.Report/definition/reportExtensions.json";
    const DOUBLE_CARD: &str = "Mini.Report/definition/pages/overview/visuals/double/visual.json";
    let dir = tmp_dir("save-ext-");
    let pbip = mini_pbip(dir.path());
    let ext = json!({
        "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/reportExtension/1.0.0/schema.json",
        "name": "extension",
        "entities": [{
            "name": "Sales",
            "measures": [
                {
                    "name": "Double Total",
                    "dataType": "Integer",
                    "expression": "[Total] * 2",
                    "formatString": "0",
                    "references": {"measures": [{"entity": "Sales", "name": "Total"}]}
                },
                {"name": "Mean Amount", "dataType": "Double", "expression": "AVERAGE(Sales[Amount])"}
            ]
        }]
    });
    fs::write(
        dir.path().join(EXT),
        serde_json::to_string_pretty(&ext).unwrap(),
    )
    .unwrap();
    // A card showing the report-level measure.
    let mut card: Value =
        serde_json::from_slice(&fs::read(dir.path().join(TOTAL_CARD)).unwrap()).unwrap();
    card["name"] = json!("double");
    card["visual"]["query"]["queryState"]["Data"]["projections"][0] = json!({
        "field": {"Measure": {"Expression": {"SourceRef": {"Schema": "extension", "Entity": "Sales"}}, "Property": "Double Total"}},
        "queryRef": "Sales.Double Total",
        "nativeQueryRef": "Double Total"
    });
    fs::create_dir_all(dir.path().join(DOUBLE_CARD).parent().unwrap()).unwrap();
    fs::write(
        dir.path().join(DOUBLE_CARD),
        serde_json::to_string_pretty(&card).unwrap(),
    )
    .unwrap();
    let original = files(dir.path());
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    let lint = r.run("doc.lint", json!({"docId": doc})).await;
    assert!(lint["summary"]["byRule"].get("LINT003").is_none(), "{lint}");

    // The preview lists the report-level measure's DAX.
    let preview = r
        .run(
            "model.object.rename",
            json!({"docId": doc, "table": "Sales", "measure": "Total", "newName": "Grand Total", "dryRun": true}),
        )
        .await;
    let rewrites = preview["rewrites"].as_array().unwrap();
    assert!(
        rewrites.iter().any(|w| w["location"]
            .as_str()
            .unwrap()
            .starts_with("report-level measure 'Sales'[Double Total] of report ")
            && w["after"] == "[Grand Total] * 2"),
        "{preview}"
    );
    for mut params in [
        json!({"table": "Sales", "measure": "Total", "newName": "Grand Total"}),
        json!({"table": "Sales", "column": "Amount", "newName": "Revenue"}),
        json!({"table": "Sales", "newName": "Sales Data"}),
    ] {
        params["docId"] = json!(doc);
        r.run("model.object.rename", params).await;
    }
    let out = r.run("doc.save", json!({"docId": doc})).await;
    let written = strings(&out["written"]);
    assert!(
        written.contains(&EXT) && written.contains(&DOUBLE_CARD),
        "{out}"
    );
    let saved = files(dir.path());
    let mut expected = ext.clone();
    expected["entities"][0]["name"] = json!("Sales Data");
    let measures = &mut expected["entities"][0]["measures"];
    measures[0]["expression"] = json!("[Grand Total] * 2");
    measures[0]["references"]["measures"][0] =
        json!({"entity": "Sales Data", "name": "Grand Total"});
    measures[1]["expression"] = json!("AVERAGE('Sales Data'[Revenue])");
    assert_eq!(
        String::from_utf8(saved[EXT].clone()).unwrap(),
        serde_json::to_string_pretty(&expected).unwrap()
    );
    let card = String::from_utf8(saved[DOUBLE_CARD].clone()).unwrap();
    assert!(
        card.contains("\"Entity\": \"Sales Data\"")
            && card.contains("\"queryRef\": \"Sales Data.Double Total\""),
        "{card}"
    );

    // Reopened: the report-level measures still bind, and deleting their
    // table lists them.
    let fresh = Runner::new();
    let reopened = fresh
        .open(json!({"path": pbip, "autoRefresh": false}))
        .await;
    let lint = fresh.run("doc.lint", json!({"docId": reopened})).await;
    assert!(lint["summary"]["byRule"].get("LINT003").is_none(), "{lint}");
    let delete = fresh
        .run(
            "model.table.delete",
            json!({"docId": reopened, "table": "Sales Data", "dryRun": true}),
        )
        .await;
    let dependents = delete["dependents"].to_string();
    assert!(
        dependents.contains("report-level measure 'Sales Data'[Double Total]")
            && dependents.contains("report-level measure 'Sales Data'[Mean Amount]"),
        "{dependents}"
    );

    // Undo the renames and save: the original bytes.
    for _ in 0..3 {
        r.run("doc.undo", json!({"docId": doc})).await;
    }
    r.run("doc.save", json!({"docId": doc})).await;
    assert_eq!(files(dir.path()), original);
}

fn is_guid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

/// The words of `text` made of hex digits and `-` (GUIDs among them).
fn hex_words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_ascii_hexdigit() || c == '-'))
}

/// `text` with every GUID that is not in `known` replaced by `<new id>`.
fn new_ids(text: &str, known: &BTreeSet<&str>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if is_guid(word) && !known.contains(word.as_str()) {
            out.push_str("<new id>");
        } else {
            out.push_str(word);
        }
        word.clear();
    };
    for c in text.chars() {
        if c.is_ascii_hexdigit() || c == '-' {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// GT-4 (ADR-0014): Power BI Desktop 2.159 made the edits of
/// `reference/groundtruth/gt4/s24/edits.json` on a project it saved
/// (`before/`) and saved the files of `after/`. The same edits through the
/// registry save the same bytes, apart from new identifiers and the entries
/// of `gt4/deviations.json`.
#[tokio::test(flavor = "multi_thread")]
async fn edits_save_like_power_bi_desktop() {
    const DEF: &str = "AcmeOutfitters.SemanticModel/definition";
    const SESSION: &str = "s24";
    let (Some(acme), Some(gt4)) = (fixture(ACME), fixture("groundtruth/gt4")) else {
        return;
    };
    let session = gt4.join(SESSION);
    let dir = tmp_dir("gt4-");
    copy_dir(&acme, dir.path());
    let def = dir.path().join(DEF);
    copy_dir(&session.join("before"), &def);
    let before = files(&def);
    let after = files(&session.join("after"));
    let text = |b: &[u8]| String::from_utf8(b.to_vec()).unwrap();
    let before_text: Vec<String> = before.values().map(|b| text(b)).collect();
    let known: BTreeSet<&str> = before_text
        .iter()
        .flat_map(|t| hex_words(t))
        .filter(|w| is_guid(w))
        .collect();

    let r = Runner::new();
    let doc = r
        .open(json!({"path": dir.path().join("AcmeOutfitters.pbip"),
            "blobMap": dir.path().join("blob-map.json")}))
        .await;
    // Desktop's calculation items carry no `ordinal`: numbered by position, as Desktop answers.
    let recorded: Value = serde_json::from_str(
        &fs::read_to_string(gt4.parent().unwrap().join("recordings/acme-tables.json")).unwrap(),
    )
    .unwrap();
    let time_calc = recorded["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "time-calc")
        .unwrap();
    let items = r
        .run(
            "dax.query",
            json!({"docId": doc, "query": "EVALUATE 'Time Calc'"}),
        )
        .await;
    assert_eq!(items["tables"][0]["rows"], time_calc["rows"], "{items}");
    let edits: Value =
        serde_json::from_str(&fs::read_to_string(session.join("edits.json")).unwrap()).unwrap();
    let mut last = Value::Null;
    for step in edits.as_array().unwrap() {
        let mut params = step["params"].clone();
        params["docId"] = json!(doc);
        last = r.run(step["id"].as_str().unwrap(), params).await;
    }
    // The last step saves: the files Desktop changed and no other.
    let changed: Vec<String> = after.keys().map(|f| format!("{DEF}/{f}")).collect();
    assert_eq!(sorted(&last["written"]), changed, "{last}");
    assert_eq!(last["deleted"], json!([]), "{last}");

    let deviations: Value =
        serde_json::from_str(&fs::read_to_string(gt4.join("deviations.json")).unwrap()).unwrap();
    let deviations = deviations["deviations"].as_array().unwrap();
    assert!(
        deviations.len() <= 20,
        "ADR-0014 caps the deviation list at 20"
    );
    let mut desktop = before.clone();
    desktop.extend(after);
    for d in deviations.iter().filter(|d| d["session"] == SESSION) {
        assert!(
            desktop.contains_key(d["file"].as_str().unwrap()),
            "deviation for a file Desktop did not save: {d}"
        );
    }
    let saved = files(&def);
    assert_eq!(
        saved.keys().collect::<Vec<_>>(),
        desktop.keys().collect::<Vec<_>>()
    );
    for (file, bytes) in &desktop {
        let mut expected = new_ids(&text(bytes), &known);
        let eol = if expected.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        for d in deviations
            .iter()
            .filter(|d| d["session"] == SESSION && d["file"] == file.as_str())
        {
            let lines = |key: &str| -> String {
                d[key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|l| format!("{}{eol}", l.as_str().unwrap()))
                    .collect()
            };
            let (from, to) = (lines("desktop"), lines("dashcraft"));
            assert_eq!(
                expected.matches(&from).count(),
                1,
                "deviation no longer applies to {file}: {d}"
            );
            expected = expected.replace(&from, &to);
        }
        let actual = new_ids(&text(&saved[file]), &known);
        pretty_assertions::assert_eq!(actual, expected, "{file}");
    }
}
