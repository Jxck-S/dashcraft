//! Semantic diff (`diff_documents`) of the Acme Outfitters example against
//! variants made by editing a copy of its files.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use pbi_core::{DiffChange, DiffKind, OpenOptions, SnippetLanguage, diff_documents, open_path};
use pbi_model::Document;
use pretty_assertions::assert_eq;

const TABLES: &str = "AcmeOutfitters.SemanticModel/definition/tables";
const OVERVIEW: &str = "Acme Outfitters - Overview.Report/definition/pages";
/// A page of the Overview report, renamed by the script.
const RENAMED_PAGE: &str = "59126dbf5023ae95f4dc";
/// A page of the Overview report losing a visual and reformatting another.
const EDITED_PAGE: &str = "de11804d145ce7e78111";
const REMOVED_VISUAL: &str = "0be665cce25f93e37e88";
const REFORMATTED_VISUAL: &str = "24f8ad03f5a0230faab0";

fn example() -> PathBuf {
    std::env::var_os("DASHCRAFT_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
        .join("examples/acme-outfitters")
}

/// A copy of the example project in a fresh folder under the target dir.
fn project() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix("pbi-core-diff-")
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

fn open(root: &Path) -> Document {
    open_path(&root.join("AcmeOutfitters.pbip"), &OpenOptions::default())
        .unwrap()
        .document
}

fn replace(root: &Path, file: &str, from: &str, to: &str) {
    let path = root.join(file);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains(from), "{file} lacks {from:?}");
    std::fs::write(&path, text.replacen(from, to, 1)).unwrap();
}

fn edit_json(root: &Path, file: &str, edit: impl FnOnce(&mut serde_json::Value)) {
    let path = root.join(file);
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    edit(&mut value);
    // Compact output: a formatting change on top of the edit.
    std::fs::write(&path, serde_json::to_string(&value).unwrap()).unwrap();
}

#[test]
fn diff_of_acme_variants_lists_exactly_the_scripted_changes() {
    let (_dir, root) = project();
    let before = open(&root);
    assert!(diff_documents(&before, &open(&root)).is_empty());

    // The script: measure changed and added, calculation group and item
    // changed, a page renamed, a visual removed, another only reformatted.
    let orders = format!("{TABLES}/Orders.tmdl");
    replace(
        &root,
        &orders,
        "measure 'Order Count' = COUNTROWS(Orders)",
        "measure 'Order Count' = COUNTROWS(Orders) + 0",
    );
    replace(
        &root,
        &orders,
        "\t/// Order number",
        "\tmeasure 'Diff Probe' = 1\n\n\t/// Order number",
    );
    let time_calc = format!("{TABLES}/Time Calc.tmdl");
    replace(&root, &time_calc, "precedence: 10", "precedence: 20");
    replace(
        &root,
        &time_calc,
        "calculationItem Current = SELECTEDMEASURE()",
        "calculationItem Current = SELECTEDMEASURE() + 0",
    );
    edit_json(
        &root,
        &format!("{OVERVIEW}/{RENAMED_PAGE}/page.json"),
        |page| page["displayName"] = "Renamed Page".into(),
    );
    std::fs::remove_dir_all(
        root.join(format!("{OVERVIEW}/{EDITED_PAGE}/visuals/{REMOVED_VISUAL}")),
    )
    .unwrap();
    edit_json(
        &root,
        &format!("{OVERVIEW}/{EDITED_PAGE}/visuals/{REFORMATTED_VISUAL}/visual.json"),
        |_| {},
    );

    let after = open(&root);
    let diff = diff_documents(&before, &after);
    let report = after
        .reports
        .iter()
        .find(|r| r.pages.iter().any(|p| p.name == RENAMED_PAGE))
        .unwrap();
    let edited_page = report.pages.iter().find(|p| p.name == EDITED_PAGE).unwrap();
    let mut got: Vec<_> = diff
        .objects
        .iter()
        .map(|o| (o.kind, o.path.clone(), o.change))
        .collect();
    got.sort_by(|a, b| a.1.cmp(&b.1));
    let mut want = vec![
        (
            DiffKind::Measure,
            "Orders[Order Count]".to_owned(),
            DiffChange::Changed,
        ),
        (
            DiffKind::Measure,
            "Orders[Diff Probe]".to_owned(),
            DiffChange::Added,
        ),
        (DiffKind::Table, "Time Calc".to_owned(), DiffChange::Changed),
        (
            DiffKind::CalculationItem,
            "Time Calc[Current]".to_owned(),
            DiffChange::Changed,
        ),
        (
            DiffKind::Page,
            format!("{}/Renamed Page", report.name),
            DiffChange::Changed,
        ),
        (
            DiffKind::Visual,
            format!(
                "{}/{}/{REMOVED_VISUAL}",
                report.name, edited_page.display_name
            ),
            DiffChange::Removed,
        ),
    ];
    want.sort_by(|a, b| a.1.cmp(&b.1));
    assert_eq!(got, want);

    let count = diff
        .objects
        .iter()
        .find(|o| o.path == "Orders[Order Count]")
        .unwrap();
    let names: Vec<_> = count.properties.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["expression"]);
    assert_eq!(
        count.properties[0].after,
        Some("COUNTROWS(Orders) + 0".into())
    );
    assert!(
        count
            .after
            .as_ref()
            .is_some_and(|o| o.path.ends_with("Orders.tmdl")),
        "{:?}",
        count.after
    );
    let page = diff
        .objects
        .iter()
        .find(|o| o.kind == DiffKind::Page)
        .unwrap();
    let names: Vec<_> = page.properties.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["displayName"]);
    assert_eq!(diff.count(DiffChange::Changed), 4);

    // Snippets: TMDL for model objects, written the same way on both sides
    // so only the edit differs; JSON for report objects.
    let snippet = |path: &str| {
        diff.objects
            .iter()
            .find(|o| o.path == path)
            .and_then(|o| o.snippet.clone())
            .unwrap_or_else(|| panic!("no snippet for {path}"))
    };
    let count = snippet("Orders[Order Count]");
    assert_eq!(count.language, SnippetLanguage::Tmdl);
    let (b, a) = (count.before.unwrap(), count.after.unwrap());
    assert!(
        b.starts_with("measure 'Order Count' = COUNTROWS(Orders)\n"),
        "{b}"
    );
    assert!(
        a.starts_with("measure 'Order Count' = COUNTROWS(Orders) + 0\n"),
        "{a}"
    );
    assert_eq!(
        b.lines().skip(1).collect::<Vec<_>>(),
        a.lines().skip(1).collect::<Vec<_>>()
    );
    assert!(b.lines().skip(1).all(|l| l.starts_with('\t')), "{b}");
    let probe = snippet("Orders[Diff Probe]");
    assert_eq!(probe.before, None);
    assert!(probe.after.unwrap().starts_with("measure 'Diff Probe' = 1"));
    let table = snippet("Time Calc");
    let (b, a) = (table.before.unwrap(), table.after.unwrap());
    assert!(
        b.contains("\ntable 'Time Calc'\n") && b.contains("precedence: 10"),
        "{b}"
    );
    assert!(a.contains("precedence: 20"), "{a}");
    for text in [&b, &a] {
        assert!(
            !text.contains("calculationItem") && !text.contains("column "),
            "{text}"
        );
    }
    let item = snippet("Time Calc[Current]").after.unwrap();
    assert_eq!(
        item,
        "/// The measure as is.\ncalculationItem Current = SELECTEDMEASURE() + 0\n\tordinal: 0"
    );
    let page = snippet(&format!("{}/Renamed Page", report.name));
    assert_eq!(page.language, SnippetLanguage::Json);
    let a = page.after.unwrap();
    assert!(a.contains("\"displayName\": \"Renamed Page\""), "{a}");
    assert!(!a.contains("\"visuals\""), "{a}");
    let visual = snippet(&format!(
        "{}/{}/{REMOVED_VISUAL}",
        report.name, edited_page.display_name
    ));
    assert_eq!(visual.after, None);
    assert!(visual.before.unwrap().contains(REMOVED_VISUAL));
}
