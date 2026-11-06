//! `doc.save` through `Registry::execute` on a temporary PBIP project:
//! only changed files are written, renames follow into the report files
//! and back byte for byte, a reopen shows the saved document, and files
//! changed on disk are a conflict until `overwrite`. `doc.saveAs` writes a
//! new project from any source and moves the document there.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use common::{
    ACME, MINI_TOTAL, MS_PBIX, PBIP, Runner, copy_dir, fixture, mini_pbip, ms_expected, tmp_dir,
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
            "relationship {STORES}\n\tcrossFilteringBehavior: bothDirections\n\ttoCardinality: many\n\tsecurityFilteringBehavior: bothDirections\n\tfromColumn: Orders.'Store ID'\n\ttoColumn: Stores.'Store ID'\n\n"
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
