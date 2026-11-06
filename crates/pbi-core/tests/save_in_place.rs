//! In-place save of a PBIP project (`save_in_place`) over a copy of the
//! committed Acme Outfitters example.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pbi_core::{
    ChangeKind, CoreError, FileChange, OpenOptions, OpenedDocument, ProjectFingerprint, open_path,
    save_in_place,
};
use pbi_model::{Document, Rename};
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
        matches!(&err, CoreError::Save(m) if m.contains("cannot be saved in place")),
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
