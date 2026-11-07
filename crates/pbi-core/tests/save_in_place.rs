//! In-place save of a PBIP project (`save_in_place`) over a copy of the
//! committed Acme Outfitters example.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pbi_core::{
    ChangeKind, CoreError, FileChange, OpenOptions, OpenedDocument, ProjectFingerprint, open_path,
    save_as_project, save_in_place,
};
use pbi_model::{Document, Rename, SourceFormat};
use pretty_assertions::assert_eq;
use serde_json::Value;

const MODEL: &str = "AcmeOutfitters.SemanticModel";
const ONLINE: &str = "Acme Outfitters - Online.Report";
const ORDERS_TMDL: &str = "AcmeOutfitters.SemanticModel/definition/tables/Orders.tmdl";
const TARGETS_TMDL: &str = "AcmeOutfitters.SemanticModel/definition/tables/Targets.tmdl";
/// A card of the Online report showing `'Web Sessions'[Sessions]`.
const SESSIONS_CARD: &str = "Acme Outfitters - Online.Report/definition/pages/a60af4d6244c3a6206a5/visuals/f251dbc74c2b67f5305d/visual.json";

fn example() -> PathBuf {
    std::env::var_os("DASHCRAFT_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
        .join("examples/acme-outfitters")
}

/// A copy of the example project in a fresh folder under the target dir.
fn project() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix("pbi-core-save-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let root = dir.path().join("acme");
    let source = example();
    for entry in walkdir::WalkDir::new(&source) {
        let entry = entry.unwrap();
        let rel = entry.path().strip_prefix(&source).unwrap();
        let to = root.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&to).unwrap();
        } else {
            std::fs::copy(entry.path(), &to).unwrap();
        }
    }
    (dir, root)
}

fn open(path: &Path) -> OpenedDocument {
    open_path(path, &OpenOptions::default()).unwrap()
}

/// Every project file (without `.pbi/`), by `/`-separated path.
fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".pbi")
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            let rel = e.path().strip_prefix(root).unwrap();
            let rel = rel.to_string_lossy().replace('\\', "/");
            (rel, std::fs::read(e.path()).unwrap())
        })
        .collect()
}

/// Paths whose bytes differ between two snapshots (added, removed or
/// modified).
fn changed(a: &BTreeMap<String, Vec<u8>>, b: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
    let mut out: Vec<String> = a
        .iter()
        .filter(|(p, bytes)| b.get(*p) != Some(*bytes))
        .map(|(p, _)| p.clone())
        .chain(b.keys().filter(|p| !a.contains_key(*p)).cloned())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Lines of `after` not in `before` (a cheap line diff).
fn new_lines(before: &[u8], after: &[u8]) -> Vec<String> {
    let before = String::from_utf8_lossy(before);
    let old: Vec<&str> = before.lines().collect();
    String::from_utf8_lossy(after)
        .lines()
        .filter(|l| !old.contains(l))
        .map(str::to_owned)
        .collect()
}

/// JSON of a value without `id` and `origin` keys.
fn normalized<T: serde::Serialize>(value: &T) -> Value {
    fn strip(v: &mut Value) {
        match v {
            Value::Object(map) => {
                map.remove("id");
                map.remove("origin");
                map.values_mut().for_each(strip);
            }
            Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut v = serde_json::to_value(value).unwrap();
    strip(&mut v);
    v
}

fn edit_measure(doc: &mut Document, table: &str, measure: &str, expression: &str) {
    let m = doc
        .model
        .as_mut()
        .unwrap()
        .table_mut(table)
        .unwrap()
        .measure_mut(measure)
        .unwrap();
    expression.clone_into(&mut m.expression);
}

/// Applies a rename to the model names and every report reference, as the
/// session's rename does (without DAX).
fn rename(doc: &mut Document, rename: &Rename) {
    let model = doc.model.as_mut().unwrap();
    match rename {
        Rename::Table { old, new } => new.clone_into(&mut model.table_mut(old).unwrap().name),
        Rename::Measure { table, old, new } => new.clone_into(
            &mut model
                .table_mut(table)
                .unwrap()
                .measure_mut(old)
                .unwrap()
                .name,
        ),
        Rename::Column { .. } => unreachable!("not used"),
    }
    for relationship in &mut model.relationships {
        rename.apply_to_relationship(relationship);
    }
    for report in &mut doc.reports {
        rename.apply_to_report_measures(&mut report.measures);
        rename.apply_to_filters(&mut report.filters);
        for page in &mut report.pages {
            rename.apply_to_filters(&mut page.filters);
            for visual in &mut page.visuals {
                rename.apply_to_visual(visual);
            }
        }
    }
}

#[test]
fn saving_an_unchanged_project_writes_nothing() {
    let (_dir, root) = project();
    let before = files(&root);
    let opened = open(&root);
    let mut fingerprint = ProjectFingerprint::scan(&opened).unwrap();
    assert!(fingerprint.len() > 50, "{}", fingerprint.len());
    let outcome = save_in_place(&opened, &mut fingerprint, &opened.document, false).unwrap();
    assert!(outcome.written.is_empty(), "{:?}", outcome.written);
    assert!(outcome.deleted.is_empty(), "{:?}", outcome.deleted);
    assert!(outcome.overwritten.is_empty());
    assert_eq!(changed(&before, &files(&root)), Vec::<String>::new());
}

#[test]
fn measure_edit_writes_only_its_lines_and_undo_restores_the_bytes() {
    let (_dir, root) = project();
    let original = files(&root);
    let opened = open(&root);
    let mut fingerprint = ProjectFingerprint::scan(&opened).unwrap();
    let mut edited = opened.document.clone();
    edit_measure(
        &mut edited,
        "Orders",
        "Total Cost",
        "SUM(Orders[Line Cost]) * 1",
    );

    let outcome = save_in_place(&opened, &mut fingerprint, &edited, false).unwrap();
    assert_eq!(outcome.written, [ORDERS_TMDL]);
    assert!(outcome.deleted.is_empty());
    let saved = files(&root);
    assert_eq!(changed(&original, &saved), [ORDERS_TMDL]);
    assert_eq!(
        new_lines(&original[ORDERS_TMDL], &saved[ORDERS_TMDL]),
        ["\tmeasure 'Total Cost' = SUM(Orders[Line Cost]) * 1"]
    );
    let reopened = open(&root);
    assert_eq!(
        reopened
            .document
            .model
            .as_ref()
            .unwrap()
            .measure("Orders", "Total Cost")
            .unwrap()
            .expression,
        "SUM(Orders[Line Cost]) * 1"
    );

    // Undo, then save again: the original bytes.
    let outcome = save_in_place(&outcome.saved, &mut fingerprint, &opened.document, false).unwrap();
    assert_eq!(outcome.written, [ORDERS_TMDL]);
    assert_eq!(changed(&original, &files(&root)), Vec::<String>::new());
}

#[test]
fn external_changes_are_conflicts_until_overwritten() {
    let (_dir, root) = project();
    let opened = open(&root);
    let mut fingerprint = ProjectFingerprint::scan(&opened).unwrap();
    let original = files(&root);

    // Another program edits one file and adds another.
    let targets = root.join(TARGETS_TMDL);
    let mut text = std::fs::read(&targets).unwrap();
    text.extend_from_slice(b"\n");
    std::fs::write(&targets, &text).unwrap();
    let notes = format!("{MODEL}/notes.txt");
    std::fs::write(root.join(&notes), "external").unwrap();

    let mut edited = opened.document.clone();
    edit_measure(
        &mut edited,
        "Orders",
        "Total Cost",
        "SUM(Orders[Line Cost]) * 2",
    );
    let err = save_in_place(&opened, &mut fingerprint, &edited, false).unwrap_err();
    let CoreError::Conflict { changes } = &err else {
        panic!("{err}");
    };
    let expected = vec![
        FileChange {
            path: TARGETS_TMDL.into(),
            change: ChangeKind::Modified,
        },
        FileChange {
            path: notes.clone(),
            change: ChangeKind::Added,
        },
    ];
    assert_eq!(*changes, expected);
    assert_eq!(err.to_string().split(' ').next(), Some("2"));
    // Nothing was written.
    assert_eq!(files(&root)[ORDERS_TMDL], original[ORDERS_TMDL]);

    // Overwrite: the edit is saved, the known file gets the document's
    // version back, the unknown file is kept.
    let outcome = save_in_place(&opened, &mut fingerprint, &edited, true).unwrap();
    assert_eq!(outcome.overwritten, expected);
    assert_eq!(outcome.written, [ORDERS_TMDL, TARGETS_TMDL]);
    let now = files(&root);
    assert_eq!(now[TARGETS_TMDL], original[TARGETS_TMDL]);
    assert_eq!(now[&notes], b"external");

    // The fingerprint follows the save: no conflict next time.
    let again = save_in_place(&outcome.saved, &mut fingerprint, &edited, false).unwrap();
    assert!(again.written.is_empty());
    assert!(again.overwritten.is_empty());

    // A deleted file is a conflict too.
    std::fs::remove_file(root.join(&notes)).unwrap();
    let err = save_in_place(&outcome.saved, &mut fingerprint, &edited, false).unwrap_err();
    assert!(
        matches!(&err, CoreError::Conflict { changes } if changes == &[FileChange { path: notes.clone(), change: ChangeKind::Removed }]),
        "{err:?}"
    );
}

#[test]
fn renames_move_the_table_file_and_follow_into_the_report() {
    let (_dir, root) = project();
    let original = files(&root);
    let opened = open(&root);
    let mut fingerprint = ProjectFingerprint::scan(&opened).unwrap();
    let mut edited = opened.document.clone();
    rename(
        &mut edited,
        &Rename::Measure {
            table: "Web Sessions".into(),
            old: "Sessions".into(),
            new: "Visits".into(),
        },
    );
    rename(
        &mut edited,
        &Rename::Table {
            old: "Web Sessions".into(),
            new: "Site Sessions".into(),
        },
    );

    let outcome = save_in_place(&opened, &mut fingerprint, &edited, false).unwrap();
    let old_file = format!("{MODEL}/definition/tables/Web Sessions.tmdl");
    let new_file = format!("{MODEL}/definition/tables/Site Sessions.tmdl");
    assert_eq!(outcome.deleted, std::slice::from_ref(&old_file));
    assert!(outcome.written.contains(&new_file), "{:?}", outcome.written);
    let saved = files(&root);
    assert!(!saved.contains_key(&old_file));
    let model_tmdl =
        String::from_utf8(saved[&format!("{MODEL}/definition/model.tmdl")].clone()).unwrap();
    assert!(
        model_tmdl.contains("ref table 'Site Sessions'\n")
            && !model_tmdl.contains("ref table 'Web Sessions'"),
        "{model_tmdl}"
    );

    // Only the model and the visuals using the table changed; every
    // visual of the table did.
    let using: Vec<String> = original
        .iter()
        .filter(|(p, b)| {
            p.starts_with(ONLINE)
                && p.ends_with(".json")
                && String::from_utf8_lossy(b).contains("\"Entity\": \"Web Sessions\"")
        })
        .map(|(p, _)| p.clone())
        .collect();
    assert!(
        using.contains(&format!("{ONLINE}/definition/report.json")),
        "{using:?}"
    );
    assert!(using.len() >= 4, "{using:?}");
    let report_changes: Vec<String> = changed(&original, &saved)
        .into_iter()
        .filter(|p| !p.starts_with(MODEL))
        .collect();
    assert_eq!(report_changes, using);
    let card = String::from_utf8(saved[SESSIONS_CARD].clone()).unwrap();
    assert!(card.contains("\"Entity\": \"Site Sessions\""), "{card}");
    assert!(card.contains("\"Property\": \"Visits\""), "{card}");
    assert!(
        card.contains("\"queryRef\": \"Site Sessions.Visits\""),
        "{card}"
    );
    assert!(card.contains("\"nativeQueryRef\": \"Visits\""), "{card}");
    assert!(!card.contains("Web Sessions"), "{card}");

    // Re-opened, the project reads as the edited document.
    let reopened = open(&root);
    let model = reopened.document.model.as_ref().unwrap();
    assert!(model.table("Web Sessions").is_none());
    assert!(model.measure("Site Sessions", "Visits").is_some());
    assert_eq!(
        normalized(&reopened.document.reports),
        normalized(&edited.reports)
    );

    // Renaming back restores the original files.
    let outcome = save_in_place(&outcome.saved, &mut fingerprint, &opened.document, false).unwrap();
    assert_eq!(outcome.deleted, [new_file]);
    assert_eq!(changed(&original, &files(&root)), Vec::<String>::new());
}

#[test]
fn renamed_then_deleted_objects_keep_their_new_names_in_the_report() {
    // A rename rewrites the report and a delete leaves it pointing at the
    // new name, as in Power BI Desktop: the save writes those references.
    for delete_table in [false, true] {
        let (_dir, root) = project();
        let opened = open(&root);
        let mut fingerprint = ProjectFingerprint::scan(&opened).unwrap();
        let mut edited = opened.document.clone();
        rename(
            &mut edited,
            &Rename::Measure {
                table: "Web Sessions".into(),
                old: "Sessions".into(),
                new: "Visits".into(),
            },
        );
        let entity = if delete_table {
            rename(
                &mut edited,
                &Rename::Table {
                    old: "Web Sessions".into(),
                    new: "Site Sessions".into(),
                },
            );
            let model = edited.model.as_mut().unwrap();
            model.tables.retain(|t| t.name != "Site Sessions");
            "Site Sessions"
        } else {
            let model = edited.model.as_mut().unwrap();
            let table = model.table_mut("Web Sessions").unwrap();
            table.measures.retain(|m| m.name != "Visits");
            "Web Sessions"
        };

        save_in_place(&opened, &mut fingerprint, &edited, false).unwrap();
        let card = std::fs::read_to_string(root.join(SESSIONS_CARD)).unwrap();
        assert!(
            card.contains(&format!("\"Entity\": \"{entity}\"")),
            "{card}"
        );
        assert!(card.contains("\"Property\": \"Visits\""), "{card}");
        assert!(
            card.contains(&format!("\"queryRef\": \"{entity}.Visits\"")),
            "{card}"
        );
        let reopened = open(&root);
        let model = reopened.document.model.as_ref().unwrap();
        assert!(model.measure(entity, "Visits").is_none());
        assert_eq!(model.table("Web Sessions").is_none(), delete_table);
        assert_eq!(
            normalized(&reopened.document.reports),
            normalized(&edited.reports)
        );
    }
}

#[test]
fn renames_follow_into_report_level_measures() {
    let (_dir, root) = project();
    let ext_path = format!("{ONLINE}/definition/reportExtensions.json");
    let ext = serde_json::json!({
        "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/reportExtension/1.0.0/schema.json",
        "name": "extension",
        "entities": [
            {
                "name": "Web Sessions",
                "measures": [{
                    "name": "Double Sessions",
                    "dataType": "Integer",
                    "expression": "[Sessions] * 2",
                    "dataCategory": "Uncategorized",
                    "references": {"measures": [{"entity": "Web Sessions", "name": "Sessions"}]}
                }]
            },
            {
                "name": "Orders",
                "measures": [{
                    "name": "Sessions per Order",
                    "dataType": "Double",
                    "expression": "DIVIDE([Double Sessions], COUNTROWS(Orders))",
                    "references": {"measures": [
                        {"schema": "extension", "entity": "Web Sessions", "name": "Double Sessions"}
                    ]}
                }]
            }
        ]
    });
    let ext_bytes = serde_json::to_vec_pretty(&ext).unwrap();
    std::fs::write(root.join(&ext_path), &ext_bytes).unwrap();
    let original = files(&root);
    let opened = open(&root);
    let mut fingerprint = ProjectFingerprint::scan(&opened).unwrap();
    let online = opened
        .document
        .reports
        .iter()
        .position(|r| r.measure("Double Sessions").is_some())
        .unwrap();
    assert_eq!(opened.document.reports[online].measures.len(), 2);
    let outcome = save_in_place(&opened, &mut fingerprint, &opened.document, false).unwrap();
    assert!(outcome.written.is_empty(), "{:?}", outcome.written);

    let mut edited = opened.document.clone();
    rename(
        &mut edited,
        &Rename::Measure {
            table: "Web Sessions".into(),
            old: "Sessions".into(),
            new: "Visits".into(),
        },
    );
    rename(
        &mut edited,
        &Rename::Table {
            old: "Web Sessions".into(),
            new: "Site Sessions".into(),
        },
    );
    // The DAX, as the session's rename rewrites it.
    "[Visits] * 2".clone_into(&mut edited.reports[online].measures[0].expression);
    let outcome = save_in_place(&outcome.saved, &mut fingerprint, &edited, false).unwrap();
    assert!(outcome.written.contains(&ext_path), "{:?}", outcome.written);
    let saved = files(&root);
    let mut expected = ext.clone();
    expected["entities"][0]["name"] = "Site Sessions".into();
    let first = &mut expected["entities"][0]["measures"][0];
    first["expression"] = "[Visits] * 2".into();
    first["references"]["measures"][0] =
        serde_json::json!({"entity": "Site Sessions", "name": "Visits"});
    expected["entities"][1]["measures"][0]["references"]["measures"][0]["entity"] =
        "Site Sessions".into();
    assert_eq!(
        String::from_utf8(saved[&ext_path].clone()).unwrap(),
        String::from_utf8(serde_json::to_vec_pretty(&expected).unwrap()).unwrap()
    );
    let reopened = open(&root);
    assert_eq!(
        normalized(&reopened.document.reports),
        normalized(&edited.reports)
    );

    // Undo: the original bytes.
    let outcome = save_in_place(&outcome.saved, &mut fingerprint, &opened.document, false).unwrap();
    assert!(outcome.written.contains(&ext_path), "{:?}", outcome.written);
    assert_eq!(changed(&original, &files(&root)), Vec::<String>::new());

    // Other changes to them cannot be written yet: refused, not dropped.
    let mut edited = opened.document.clone();
    edited.reports[online].measures[1].hidden = true;
    let err = save_in_place(&outcome.saved, &mut fingerprint, &edited, false).unwrap_err();
    assert!(
        err.to_string()
            .contains("the report-level measures of report"),
        "{err}"
    );
    assert_eq!(changed(&original, &files(&root)), Vec::<String>::new());
}

#[test]
fn case_only_table_rename_renames_the_file() {
    let (_dir, root) = project();
    let opened = open(&root);
    let mut fingerprint = ProjectFingerprint::scan(&opened).unwrap();
    let mut edited = opened.document.clone();
    rename(
        &mut edited,
        &Rename::Table {
            old: "Web Sessions".into(),
            new: "web sessions".into(),
        },
    );
    save_in_place(&opened, &mut fingerprint, &edited, false).unwrap();
    let tables: Vec<String> = std::fs::read_dir(root.join(MODEL).join("definition/tables"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.to_lowercase().contains("sessions"))
        .collect();
    assert_eq!(tables, ["web sessions.tmdl"]);
    let reopened = open(&root);
    let model = reopened.document.model.as_ref().unwrap();
    assert!(model.tables.iter().any(|t| t.name == "web sessions"));
    let card = std::fs::read_to_string(root.join(SESSIONS_CARD)).unwrap();
    assert!(card.contains("\"Entity\": \"web sessions\""), "{card}");
}

#[test]
fn moving_a_visual_patches_only_its_position() {
    let (_dir, root) = project();
    let original = files(&root);
    let opened = open(&root);
    let mut fingerprint = ProjectFingerprint::scan(&opened).unwrap();
    let mut edited = opened.document.clone();
    let visual = edited
        .reports
        .iter_mut()
        .flat_map(|r| &mut r.pages)
        .flat_map(|p| &mut p.visuals)
        .find(|v| v.name == "f251dbc74c2b67f5305d")
        .unwrap();
    visual.position.x = 300.5;
    visual.position.width = 260.0;

    let outcome = save_in_place(&opened, &mut fingerprint, &edited, false).unwrap();
    assert_eq!(outcome.written, [SESSIONS_CARD]);
    let saved = files(&root);
    assert_eq!(changed(&original, &saved), [SESSIONS_CARD]);
    assert_eq!(
        new_lines(&original[SESSIONS_CARD], &saved[SESSIONS_CARD]),
        ["    \"x\": 300.5,", "    \"width\": 260,"]
    );
    let reopened = open(&root);
    assert_eq!(
        normalized(&reopened.document.reports),
        normalized(&edited.reports)
    );
}

#[test]
fn report_changes_it_cannot_write_are_refused() {
    let (_dir, root) = project();
    let original = files(&root);
    let opened = open(&root);
    let mut fingerprint = ProjectFingerprint::scan(&opened).unwrap();
    let mut edited = opened.document.clone();
    edit_measure(&mut edited, "Orders", "Total Cost", "0");
    let report = edited
        .reports
        .iter_mut()
        .find(|r| r.name.contains("Online"))
        .unwrap();
    report.pages[0].display_name = "Renamed page".into();
    let err = save_in_place(&opened, &mut fingerprint, &edited, false).unwrap_err();
    assert!(
        matches!(&err, CoreError::Save(m) if m.contains("cannot write yet")),
        "{err}"
    );
    assert_eq!(changed(&original, &files(&root)), Vec::<String>::new());
    assert!(root.join(ONLINE).is_dir());
}

#[test]
fn only_pbip_projects_save_in_place() {
    let (_dir, root) = project();
    let model_only = open(&root.join(MODEL));
    let err = ProjectFingerprint::scan(&model_only).unwrap_err();
    assert!(
        matches!(&err, CoreError::Save(m) if m.contains("PBIP")),
        "{err}"
    );
    let mut fingerprint = ProjectFingerprint::default();
    let err =
        save_in_place(&model_only, &mut fingerprint, &model_only.document, false).unwrap_err();
    assert!(matches!(&err, CoreError::Save(_)), "{err}");
}

#[test]
fn save_as_writes_the_edited_project_and_later_saves_go_there() {
    let (dir, root) = project();
    let original = files(&root);
    let opened = open(&root);
    let mut edited = opened.document.clone();
    edit_measure(
        &mut edited,
        "Orders",
        "Total Cost",
        "SUMX(Orders, Orders[Line Cost])",
    );
    rename(
        &mut edited,
        &Rename::Table {
            old: "Web Sessions".into(),
            new: "Site Sessions".into(),
        },
    );
    let visual = edited
        .reports
        .iter_mut()
        .flat_map(|r| &mut r.pages)
        .flat_map(|p| &mut p.visuals)
        .find(|v| v.name == "f251dbc74c2b67f5305d")
        .unwrap();
    visual.position.x = 300.5;

    let out = dir.path().join("saved-as");
    let outcome = save_as_project(&opened, &edited, &out, false, &OpenOptions::default()).unwrap();
    assert_eq!(outcome.pbip_file, "AcmeOutfitters.pbip");
    let written = files(&out);
    let mut listed = outcome.written.clone();
    listed.sort();
    assert_eq!(listed, written.keys().cloned().collect::<Vec<_>>());
    assert_eq!(
        outcome.bytes,
        written.values().map(|b| b.len() as u64).sum::<u64>()
    );
    // The source project is untouched; the new one holds the edits.
    assert_eq!(changed(&original, &files(&root)), Vec::<String>::new());
    let card = String::from_utf8(written[SESSIONS_CARD].clone()).unwrap();
    assert!(card.contains("\"Entity\": \"Site Sessions\""), "{card}");
    assert!(card.contains("\"x\": 300.5"), "{card}");
    let reopened = open(&out.join("AcmeOutfitters.pbip"));
    assert_eq!(
        normalized(&reopened.document.model),
        normalized(&edited.model)
    );
    assert_eq!(
        normalized(&reopened.document.reports),
        normalized(&edited.reports)
    );

    // The new baseline carries the document itself (ids included) with the
    // new project as its source.
    let saved = &outcome.saved;
    assert_eq!(saved.document.model, edited.model);
    assert_eq!(saved.document.reports, edited.reports);
    assert_eq!(saved.document.source.format, SourceFormat::Pbip);
    assert_eq!(
        saved.container.root_dir().map(Path::to_path_buf),
        Some(std::fs::canonicalize(&out).unwrap())
    );

    // A no-op save of the new project writes nothing; the next edit writes
    // only its file, there.
    let mut fingerprint = outcome.fingerprint;
    let noop = save_in_place(saved, &mut fingerprint, &saved.document, false).unwrap();
    assert!(noop.written.is_empty(), "{:?}", noop.written);
    assert!(noop.deleted.is_empty(), "{:?}", noop.deleted);
    let mut next = saved.document.clone();
    edit_measure(&mut next, "Orders", "Total Cost", "0");
    let again = save_in_place(saved, &mut fingerprint, &next, false).unwrap();
    assert_eq!(again.written, [ORDERS_TMDL]);
    assert_eq!(changed(&original, &files(&root)), Vec::<String>::new());

    // A non-empty folder needs overwrite.
    let err = save_as_project(&opened, &edited, &out, false, &OpenOptions::default()).unwrap_err();
    assert!(
        matches!(&err, CoreError::Save(m) if m.contains("not empty")),
        "{err}"
    );
    save_as_project(&opened, &edited, &out, true, &OpenOptions::default()).unwrap();
}

/// The committed Microsoft samples (three PBIX, one PBIT).
fn microsoft_samples() -> Vec<PathBuf> {
    let dir = example().join("../../samples/microsoft");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("SKIP: {} missing", dir.display());
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("pbix") || e.eq_ignore_ascii_case("pbit"))
        })
        .collect();
    out.sort();
    out
}

/// The model's structure: tables with their columns, measures and
/// partitions, relationships and roles (extras keep a format-specific
/// shape, so they are left out).
fn structure(model: Option<&pbi_model::Model>) -> Value {
    let Some(model) = model else {
        return Value::Null;
    };
    let tables: Vec<Value> = model
        .tables
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "columns": t.columns.iter().map(|c| (c.name.clone(), normalized(&c.kind))).collect::<Vec<_>>(),
                "measures": t.measures.iter().map(|m| (m.name.clone(), m.expression.clone())).collect::<Vec<_>>(),
                "partitions": t.partitions.len(),
            })
        })
        .collect();
    serde_json::json!({
        "tables": tables,
        "relationships": model.relationships.iter().map(|r| normalized(&(&r.from, &r.to, r.is_active))).collect::<Vec<_>>(),
        "roles": model.roles.iter().map(|r| r.name.clone()).collect::<Vec<_>>(),
    })
}

#[test]
fn samples_save_as_pbip_with_an_equal_structure() {
    let samples = microsoft_samples();
    let dir = tempfile::Builder::new()
        .prefix("pbi-core-save-as-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    for (i, sample) in samples.iter().enumerate() {
        let name = sample.display().to_string();
        let opened = open(sample);
        let out = dir.path().join(format!("s{i}"));
        let outcome = save_as_project(
            &opened,
            &opened.document,
            &out,
            false,
            &OpenOptions::default(),
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        let reopened = open(&out.join(&outcome.pbip_file));
        assert_eq!(
            reopened.document.source.format,
            SourceFormat::Pbip,
            "{name}"
        );
        assert_eq!(
            structure(reopened.document.model.as_ref()),
            structure(opened.document.model.as_ref()),
            "{name}"
        );
        assert_eq!(
            reopened.document.reports.len(),
            opened.document.reports.len(),
            "{name}"
        );
        for (a, b) in reopened
            .document
            .reports
            .iter()
            .zip(&opened.document.reports)
        {
            assert_eq!(normalized(&a.pages), normalized(&b.pages), "{name}");
            assert_eq!(normalized(&a.filters), normalized(&b.filters), "{name}");
            assert_eq!(normalized(&a.theme), normalized(&b.theme), "{name}");
        }
        // Saving the new project without edits writes nothing.
        let saved = &outcome.saved;
        let mut fingerprint = outcome.fingerprint;
        let before = files(&out);
        let noop = save_in_place(saved, &mut fingerprint, &saved.document, false)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(noop.written.is_empty(), "{name}: {:?}", noop.written);
        assert_eq!(
            changed(&before, &files(&out)),
            Vec::<String>::new(),
            "{name}"
        );
    }
    if example().join("../../samples/microsoft").is_dir() {
        assert_eq!(samples.len(), 4);
    }
}

#[test]
fn legacy_layout_report_changes_are_refused() {
    let Some(pbix) = microsoft_samples()
        .into_iter()
        .find(|p| p.to_string_lossy().contains("Regional Sales"))
    else {
        return;
    };
    let opened = open(&pbix);
    let mut edited = opened.document.clone();
    let visual = edited
        .reports
        .iter_mut()
        .flat_map(|r| &mut r.pages)
        .flat_map(|p| &mut p.visuals)
        .next()
        .unwrap();
    visual.position.x += 10.0;
    let dir = tempfile::Builder::new()
        .prefix("pbi-core-save-as-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let out = dir.path().join("legacy");
    let err = save_as_project(&opened, &edited, &out, false, &OpenOptions::default()).unwrap_err();
    assert!(
        matches!(&err, CoreError::Save(m) if m.contains("legacy layout")),
        "{err}"
    );
    assert!(!out.exists());
}
