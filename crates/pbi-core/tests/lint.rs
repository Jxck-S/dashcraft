//! Broken-reference linter over the committed Acme Outfitters example and the
//! Microsoft samples, plus mutation tests that break a copy of the example
//! project (under `target/tmp`) and check that the right rule fires with the
//! right origin and suggestion.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use pbi_core::lint::{Finding, LintOptions, LintReport, lint, lint_document, rules};
use pbi_core::{OpenOptions, OpenedDocument, open_path};
use pbi_model::{
    Column, ColumnRef, DataType, Document, FieldExpr, Model, ModelStatus, Page, Position,
    Projection, Relationship, Report, Severity, Table, Visual,
};

const OVERVIEW: &str = "Acme Outfitters - Overview.Report";
const STORES: &str = "Acme Outfitters - Stores.Report";
const ONLINE: &str = "Acme Outfitters - Online.Report";
const REPORTS: [&str; 3] = [OVERVIEW, STORES, ONLINE];
const SUMMARY_PAGE: &str = "ef796b39c816a4afa4f1";
const PERFORMANCE_PAGE: &str = "ef3ce6b6fadf22960137";
const DETAIL_PAGE: &str = "50c645140e8e10eea677";
const ONLINE_PAGE: &str = "a60af4d6244c3a6206a5";
const REGION_SLICER: &str = "ba3b1b7c5a9cede8452c";
const STORES_CARD: &str = "c81376feb163c3047446";
/// The first page's card with `Orders[Total Sales]` first, per report.
const SALES_CARDS: [(&str, &str, &str); 3] = [
    (OVERVIEW, SUMMARY_PAGE, "f7a9c9413fef68532313"),
    (STORES, PERFORMANCE_PAGE, STORES_CARD),
    (ONLINE, ONLINE_PAGE, "3d4874ac0bcfb9c0f53d"),
];

fn reference_root() -> PathBuf {
    std::env::var_os("DASHCRAFT_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
}

/// A path inside the committed Acme Outfitters example, if present.
fn example(rel: &str) -> Option<PathBuf> {
    let p = reference_root().join("examples/acme-outfitters").join(rel);
    if p.exists() {
        Some(p)
    } else {
        eprintln!("SKIP: {} missing", p.display());
        None
    }
}

fn open(path: &Path) -> OpenedDocument {
    open_path(path, &OpenOptions::default())
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()))
}

fn run(path: &Path) -> LintReport {
    lint(&open(path), &LintOptions::default())
}

fn show(report: &LintReport) -> String {
    report
        .findings
        .iter()
        .map(|f| {
            format!(
                "{} {:?} {} @ {}{}",
                f.rule,
                f.severity,
                f.message,
                f.origin
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                f.suggestion
                    .as_ref()
                    .map(|s| format!(" [{s}]"))
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn of<'a>(report: &'a LintReport, rule: &'a str) -> Vec<&'a Finding> {
    report.of_rule(rule).collect()
}

fn origin_of(f: &Finding) -> (String, Option<String>, Option<u32>) {
    let o = f.origin.as_ref().expect("finding has an origin");
    (o.path.clone(), o.pointer.clone(), o.line)
}

fn visual_json(report: &str, page: &str, visual: &str) -> String {
    format!("{report}/definition/pages/{page}/visuals/{visual}/visual.json")
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// A scratch copy of the example project (TMDL, reports; not the data).
struct Project {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Project {
    fn new() -> Option<Self> {
        let src = example("")?;
        let dir = tempfile::Builder::new()
            .prefix("pbi-core-lint-")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .unwrap();
        let root = dir.path().join("acme");
        fs::create_dir_all(&root).unwrap();
        fs::copy(
            src.join("AcmeOutfitters.pbip"),
            root.join("AcmeOutfitters.pbip"),
        )
        .unwrap();
        for item in ["AcmeOutfitters.SemanticModel"].iter().chain(&REPORTS) {
            copy_dir(&src.join(item), &root.join(item));
        }
        Some(Self { _dir: dir, root })
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// Replaces every occurrence of `from` (which must exist) in `rel`.
    fn edit(&self, rel: &str, from: &str, to: &str) {
        let path = self.path(rel);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains(from), "`{from}` not found in {rel}");
        fs::write(&path, text.replace(from, to)).unwrap();
    }

    /// Edits a JSON file as a value and writes it back pretty-printed.
    fn edit_json(&self, rel: &str, f: impl FnOnce(&mut serde_json::Value)) {
        let path = self.path(rel);
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        f(&mut value);
        fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }

    fn table(name: &str) -> String {
        format!("AcmeOutfitters.SemanticModel/definition/tables/{name}.tmdl")
    }

    fn lint(&self) -> LintReport {
        run(&self.root)
    }
}

// ---------------------------------------------------------------------------
// Committed fixtures
// ---------------------------------------------------------------------------

/// Every rule that points at a broken reference (not the stale-queryRef,
/// layout, dataset or type hints).
const REFERENCE_RULES: [&str; 13] = [
    rules::MISSING_TABLE,
    rules::MISSING_FIELD,
    rules::FILTER_REF,
    rules::SORT_REF,
    rules::INTERACTION_REF,
    rules::DAX_REF,
    rules::DAX_SYNTAX,
    rules::RELATIONSHIP_REF,
    rules::RELATIONSHIP_TYPE,
    rules::QUERY_GROUP_REF,
    rules::M_REF,
    rules::DUPLICATE_NAME,
    rules::MODEL_COLUMN_REF,
];

#[test]
fn committed_example_lints_clean() {
    let mut inputs = vec![
        String::new(),
        "AcmeOutfitters.pbip".to_owned(),
        "AcmeOutfitters.SemanticModel".to_owned(),
    ];
    inputs.extend(REPORTS.iter().map(|r| (*r).to_owned()));
    for rel in inputs {
        let Some(path) = example(&rel) else { continue };
        let report = run(&path);
        println!("{rel}: {:?}\n{}\n", report.summary, show(&report));
        assert_eq!(report.summary.errors, 0, "{rel}:\n{}", show(&report));
        for rule in REFERENCE_RULES {
            assert!(
                of(&report, rule).is_empty(),
                "{rel} {rule}:\n{}",
                show(&report)
            );
        }
        // The generator writes consistent queryRefs and a tidy layout.
        assert!(report.findings.is_empty(), "{rel}:\n{}", show(&report));
    }
}

/// The Microsoft samples lint without errors, except the broken references
/// recorded in `expected.json` (checked by hand: they are real defects of
/// the Regional Sales sample).
#[test]
fn microsoft_samples_have_only_their_known_broken_references() {
    let expected_path = reference_root().join("samples/microsoft/expected.json");
    let Ok(text) = fs::read_to_string(&expected_path) else {
        eprintln!("SKIP: {} missing", expected_path.display());
        return;
    };
    let expected: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut linted = 0;
    for file in expected["files"].as_array().unwrap() {
        let path = reference_root().join(file["file"].as_str().unwrap());
        if !path.is_file() {
            eprintln!(
                "SKIP: {} missing (run scripts/fetch-samples.sh)",
                path.display()
            );
            continue;
        }
        let report = run(&path);
        let id = &file["id"];
        let errors: Vec<_> = report
            .findings
            .iter()
            .filter(|f| f.severity == Severity::Error)
            .map(|f| {
                let (path, pointer, _) = origin_of(f);
                serde_json::json!({
                    "rule": f.rule,
                    "severity": "error",
                    "message": f.message,
                    "path": path,
                    "pointer": pointer,
                })
            })
            .collect();
        let known = file["knownBrokenReferences"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert_eq!(errors, known, "{id}:\n{}", show(&report));
        linted += 1;
    }
    if reference_root().join("samples/microsoft").is_dir() {
        assert!(linted >= 4);
    }
}

/// Changes the `queryRef` of each report's sales card to a name the measure
/// no longer has, as left behind by a rename in Desktop.
fn make_sales_card_query_refs_stale(p: &Project) {
    for (report, page, card) in SALES_CARDS {
        p.edit(
            &visual_json(report, page, card),
            "\"queryRef\": \"Orders.Total Sales\"",
            "\"queryRef\": \"Orders.Total Sales count\"",
        );
    }
}

#[test]
fn stale_query_refs_are_warnings_with_a_fix() {
    let Some(p) = Project::new() else { return };
    make_sales_card_query_refs_stale(&p);
    let opened = open(&p.root);
    let report = lint(&opened, &LintOptions::default());
    let stale = of(&report, rules::STALE_QUERY_REF);
    assert_eq!(stale.len(), 3, "{}", show(&report));
    let mut cards = SALES_CARDS;
    cards.sort_unstable();
    for (f, (r, page, card)) in stale.iter().zip(cards) {
        assert_eq!(f.severity, Severity::Warning);
        assert!(
            f.message.contains("`Orders.Total Sales count`"),
            "{}",
            f.message
        );
        assert_eq!(
            origin_of(f),
            (
                visual_json(r, page, card),
                Some("/visual/query/queryState/Data/projections/0/queryRef".to_owned()),
                None
            )
        );
        assert_eq!(
            f.suggestion.as_deref(),
            Some("set queryRef to `Orders.Total Sales`")
        );
        assert!(f.object.is_some());
    }
    assert!(of(&report, rules::VISUAL_OVERLAP).is_empty());
    assert!(of(&report, rules::VISUAL_BOUNDS).is_empty());
    assert_eq!(report.findings.len(), 3, "{}", show(&report));
    assert!(
        report.summary.notes.is_empty(),
        "{:?}",
        report.summary.notes
    );

    // The document-only entry point sees the same thing for a one-model project.
    assert_eq!(
        lint_document(&opened.document, &LintOptions::default()).findings,
        report.findings
    );

    let quiet = lint(
        &opened,
        &LintOptions {
            disabled_rules: vec!["lint003".into()],
            ..LintOptions::default()
        },
    );
    assert!(quiet.findings.is_empty());
    assert!(!quiet.fails(Severity::Info));
    assert!(report.fails(Severity::Warning));
    assert!(!report.fails(Severity::Error));

    let json = report.to_json();
    assert_eq!(json["schemaVersion"], 1);
    assert_eq!(json["summary"]["warnings"], 3);
    assert_eq!(json["findings"][0]["rule"], "LINT003");
    assert_eq!(json["findings"][0]["severity"], "warning");
    let diags = report.to_diagnostics();
    assert!(diags.iter().all(|d| d.code == "LINT003"));
    assert_eq!(report.by_file().len(), 3);
}

#[test]
fn remote_report_is_still_checked_for_query_refs() {
    let Some(p) = Project::new() else { return };
    make_sales_card_query_refs_stale(&p);
    fs::write(
        p.path(&format!("{STORES}/definition.pbir")),
        br#"{
  "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definitionProperties/2.0.0/schema.json",
  "version": "4.0",
  "datasetReference": {
    "byConnection": {
      "connectionString": "Data Source=\"powerbi://api.powerbi.com/v1.0/myorg/Acme Outfitters\";initial catalog=AcmeOutfitters;semanticmodelid=00000000-0000-4000-8000-0000000000ac"
    }
  }
}
"#,
    )
    .unwrap();
    let report = run(&p.path(STORES));
    let dataset = of(&report, rules::DATASET_REF);
    assert_eq!(dataset.len(), 1, "{}", show(&report));
    assert_eq!(dataset[0].severity, Severity::Info);
    assert!(dataset[0].message.contains("field references not checked"));
    assert_eq!(origin_of(dataset[0]).0, "definition.pbir");
    assert_eq!(
        of(&report, rules::STALE_QUERY_REF).len(),
        1,
        "{}",
        show(&report)
    );
    assert!(
        report
            .summary
            .notes
            .iter()
            .any(|n| n.contains("remote model"))
    );
}

// ---------------------------------------------------------------------------
// Mutations: report side
// ---------------------------------------------------------------------------

#[test]
fn renamed_column_breaks_a_visual_and_report_filters() {
    let Some(p) = Project::new() else { return };
    p.edit(
        &Project::table("Stores"),
        "\tcolumn Channel\n",
        "\tcolumn 'Channel Type'\n",
    );
    let report = p.lint();

    let missing = of(&report, rules::MISSING_FIELD);
    assert_eq!(missing.len(), 1, "{}", show(&report));
    let f = missing[0];
    assert_eq!(f.severity, Severity::Error);
    assert!(
        f.message
            .contains("column `Stores[Channel]` does not exist"),
        "{}",
        f.message
    );
    assert_eq!(
        origin_of(f),
        (
            visual_json(OVERVIEW, SUMMARY_PAGE, "c9f87905246642c35687"),
            Some("/visual/query/queryState/Category/projections/0/field".to_owned()),
            None
        )
    );
    assert_eq!(
        f.suggestion.as_deref(),
        Some("did you mean `Stores[Channel Type]`?")
    );

    // The Stores and Online reports filter on the column at report level.
    let filters = of(&report, rules::FILTER_REF);
    assert_eq!(filters.len(), 2, "{}", show(&report));
    for (f, (r, name)) in filters.iter().zip([
        (ONLINE, "af9cb4d60d7bbe61fcf6"),
        (STORES, "b0d63946d6634cc987ce"),
    ]) {
        assert_eq!(
            origin_of(f),
            (
                format!("{r}/definition/report.json"),
                Some("/filterConfig/filters/0".to_owned()),
                None
            )
        );
        assert!(
            f.message.starts_with(&format!("report filter `{name}`")),
            "{}",
            f.message
        );
        assert_eq!(
            f.suggestion.as_deref(),
            Some("did you mean `Stores[Channel Type]`?")
        );
    }
    assert_eq!(report.summary.errors, 3, "{}", show(&report));
}

#[test]
fn table_bound_to_an_old_measure_name_breaks_projection_and_sort() {
    let Some(p) = Project::new() else { return };
    let table = visual_json(STORES, DETAIL_PAGE, "4deb2424b9269f15f86c");
    p.edit(
        &table,
        "\"Property\": \"Returns Count\"",
        "\"Property\": \"Return Count\"",
    );
    p.edit(
        &table,
        "\"queryRef\": \"Returns.Returns Count\"",
        "\"queryRef\": \"Returns.Return Count\"",
    );
    let report = p.lint();

    let missing = of(&report, rules::MISSING_FIELD);
    assert_eq!(missing.len(), 1, "{}", show(&report));
    assert_eq!(
        origin_of(missing[0]),
        (
            table.clone(),
            Some("/visual/query/queryState/Values/projections/1/field".to_owned()),
            None
        )
    );
    assert!(
        missing[0]
            .message
            .contains("measure `Returns[Return Count]` does not exist")
    );
    assert_eq!(
        missing[0].suggestion.as_deref(),
        Some("did you mean `Returns[Returns Count]`?")
    );

    let sort = of(&report, rules::SORT_REF);
    assert_eq!(sort.len(), 1);
    assert_eq!(
        origin_of(sort[0]),
        (
            table.clone(),
            Some("/visual/query/sortDefinition/sort/0".to_owned()),
            None
        )
    );
    // Its queryRef now agrees with the (broken) field; only the
    // nativeQueryRef still names the measure that exists.
    let stale: Vec<&Finding> = of(&report, rules::STALE_QUERY_REF)
        .into_iter()
        .filter(|f| origin_of(f).0 == table)
        .collect();
    assert_eq!(stale.len(), 1, "{}", show(&report));
    assert_eq!(stale[0].severity, Severity::Info);
    assert!(stale[0].message.contains("nativeQueryRef `Returns Count`"));
}

#[test]
fn renamed_measure_in_tmdl_breaks_cards_in_two_reports() {
    let Some(p) = Project::new() else { return };
    p.edit(
        &Project::table("Orders"),
        "measure 'Active Customers' =",
        "measure 'Active Customers (Distinct)' =",
    );
    let report = p.lint();
    let missing = of(&report, rules::MISSING_FIELD);
    assert_eq!(missing.len(), 2, "{}", show(&report));
    let origins: Vec<_> = missing.iter().map(|f| origin_of(f)).collect();
    assert_eq!(
        origins,
        [
            (
                visual_json(ONLINE, ONLINE_PAGE, "3d4874ac0bcfb9c0f53d"),
                Some("/visual/query/queryState/Data/projections/2/field".to_owned()),
                None
            ),
            (
                visual_json(OVERVIEW, SUMMARY_PAGE, "f7a9c9413fef68532313"),
                Some("/visual/query/queryState/Data/projections/4/field".to_owned()),
                None
            ),
        ]
    );
    for f in &missing {
        assert_eq!(
            f.suggestion.as_deref(),
            Some("did you mean `Orders[Active Customers (Distinct)]`?")
        );
    }
    assert_eq!(report.summary.errors, 2, "{}", show(&report));
}

#[test]
fn edited_query_refs_are_stale() {
    let Some(p) = Project::new() else { return };
    let slicer = visual_json(STORES, PERFORMANCE_PAGE, REGION_SLICER);
    p.edit(
        &slicer,
        "\"queryRef\": \"Stores.Region\"",
        "\"queryRef\": \"Stores.Region Old\"",
    );
    p.edit(
        &slicer,
        "\"nativeQueryRef\": \"Region\"",
        "\"nativeQueryRef\": \"Reg\"",
    );
    let report = p.lint();
    let stale: Vec<&Finding> = of(&report, rules::STALE_QUERY_REF)
        .into_iter()
        .filter(|f| origin_of(f).0 == slicer)
        .collect();
    assert_eq!(stale.len(), 2, "{}", show(&report));
    assert_eq!(stale[0].severity, Severity::Info);
    assert_eq!(
        origin_of(stale[0]).1.as_deref(),
        Some("/visual/query/queryState/Values/projections/0/nativeQueryRef")
    );
    assert_eq!(
        stale[0].suggestion.as_deref(),
        Some("set nativeQueryRef to `Region`")
    );
    assert_eq!(stale[1].severity, Severity::Warning);
    assert!(stale[1].message.contains("`Stores.Region Old`"));
    assert_eq!(
        origin_of(stale[1]).1.as_deref(),
        Some("/visual/query/queryState/Values/projections/0/queryRef")
    );
    assert_eq!(
        stale[1].suggestion.as_deref(),
        Some("set queryRef to `Stores.Region`")
    );
    assert_eq!(report.summary.errors, 0);
}

#[test]
fn deleted_visual_leaves_a_dangling_interaction() {
    let Some(p) = Project::new() else { return };
    let waterfall = "9b4e31f5f08b8acaa5ed";
    fs::remove_dir_all(p.path(&format!(
        "{STORES}/definition/pages/{PERFORMANCE_PAGE}/visuals/{waterfall}"
    )))
    .unwrap();
    let report = p.lint();
    let dangling = of(&report, rules::INTERACTION_REF);
    assert_eq!(dangling.len(), 1, "{}", show(&report));
    assert_eq!(dangling[0].severity, Severity::Warning);
    assert!(
        dangling[0]
            .message
            .contains(&format!("source `{waterfall}`"))
    );
    assert_eq!(
        origin_of(dangling[0]),
        (
            format!("{STORES}/definition/pages/{PERFORMANCE_PAGE}/page.json"),
            Some("/visualInteractions/3/source".to_owned()),
            None
        )
    );
    assert!(dangling[0].suggestion.is_some());
}

#[test]
fn report_level_measures_are_not_missing() {
    let Some(p) = Project::new() else { return };
    let card = visual_json(STORES, PERFORMANCE_PAGE, STORES_CARD);
    p.edit_json(&card, |v| {
        v["visual"]["query"]["queryState"]["Data"]["projections"][0]["field"] = serde_json::json!({
            "Measure": {
                "Expression": {"SourceRef": {"Schema": "extension", "Entity": "Orders"}},
                "Property": "Total Sales (report)"
            }
        });
    });
    let ext = serde_json::json!({
        "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/reportExtension/1.0.0/schema.json",
        "name": "extension",
        "entities": [{
            "name": "Orders",
            "measures": [{"name": "Total Sales (report)", "dataType": "Double", "expression": "[Total Sales]"}]
        }]
    });
    let ext_path = format!("{STORES}/definition/reportExtensions.json");
    fs::write(p.path(&ext_path), serde_json::to_vec_pretty(&ext).unwrap()).unwrap();
    let report = p.lint();
    assert!(
        of(&report, rules::MISSING_FIELD).is_empty(),
        "{}",
        show(&report)
    );
    // Without the extension file the same binding is broken.
    fs::remove_file(p.path(&ext_path)).unwrap();
    let report = p.lint();
    assert_eq!(
        of(&report, rules::MISSING_FIELD).len(),
        1,
        "{}",
        show(&report)
    );
}

#[test]
fn unresolved_by_path_is_an_error() {
    let Some(p) = Project::new() else { return };
    p.edit(
        &format!("{STORES}/definition.pbir"),
        "../AcmeOutfitters.SemanticModel",
        "../Missing.SemanticModel",
    );
    let report = p.lint();
    let dataset = of(&report, rules::DATASET_REF);
    assert_eq!(dataset.len(), 1, "{}", show(&report));
    assert_eq!(dataset[0].severity, Severity::Error);
    assert!(dataset[0].message.contains("`../Missing.SemanticModel`"));
    assert_eq!(
        origin_of(dataset[0]),
        (
            format!("{STORES}/definition.pbir"),
            Some("/datasetReference/byPath/path".to_owned()),
            None
        )
    );
    assert!(
        report
            .summary
            .notes
            .iter()
            .any(|n| n.contains("`Acme Outfitters - Stores`: model not found")),
        "{:?}",
        report.summary.notes
    );
}

#[test]
fn duplicate_names_and_layout_problems() {
    let Some(p) = Project::new() else { return };
    let other_slicer = "6aecd77deab2cb76c2e5";
    let (map, gauge) = ("a7199dc503d24da7b45e", "6f647d31a8428ad62fb0");
    p.edit(
        &visual_json(STORES, PERFORMANCE_PAGE, REGION_SLICER),
        &format!("\"name\": \"{REGION_SLICER}\""),
        &format!("\"name\": \"{other_slicer}\""),
    );
    p.edit(
        &format!("{STORES}/definition/pages/{DETAIL_PAGE}/page.json"),
        "\"displayName\": \"Store Detail\"",
        "\"displayName\": \"Store Performance\"",
    );
    // The map pushed past the right edge; the gauge dragged over the
    // stacked column chart.
    p.edit(
        &visual_json(STORES, PERFORMANCE_PAGE, map),
        "\"x\": 972",
        "\"x\": 1000",
    );
    p.edit(
        &visual_json(STORES, PERFORMANCE_PAGE, gauge),
        "\"x\": 946",
        "\"x\": 900",
    );
    let report = p.lint();

    let dups = of(&report, rules::DUPLICATE_NAME);
    assert_eq!(dups.len(), 2, "{}", show(&report));
    let errors: Vec<_> = dups
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .collect();
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0]
            .message
            .contains(&format!("two visuals named `{other_slicer}`"))
    );
    assert!(
        dups.iter()
            .any(|f| f.severity == Severity::Warning && f.message.contains("two pages titled"))
    );

    let bounds = of(&report, rules::VISUAL_BOUNDS);
    assert_eq!(bounds.len(), 1, "{}", show(&report));
    assert_eq!(
        origin_of(bounds[0]),
        (
            visual_json(STORES, PERFORMANCE_PAGE, map),
            Some("/position".to_owned()),
            None
        )
    );

    let overlaps = of(&report, rules::VISUAL_OVERLAP);
    assert_eq!(overlaps.len(), 1, "{}", show(&report));
    assert_eq!(overlaps[0].severity, Severity::Info);
    assert!(overlaps[0].message.contains(gauge));
    assert!(overlaps[0].message.contains("05ac8028fb6e14374ec7"));

    let no_layout = lint(
        &open(&p.root),
        &LintOptions {
            layout: false,
            ..LintOptions::default()
        },
    );
    assert!(of(&no_layout, rules::VISUAL_BOUNDS).is_empty());
    assert!(of(&no_layout, rules::VISUAL_OVERLAP).is_empty());
}

// ---------------------------------------------------------------------------
// Mutations: model side
// ---------------------------------------------------------------------------

#[test]
fn bad_dax_references_and_syntax_errors() {
    let Some(p) = Project::new() else { return };
    let orders = Project::table("Orders");
    let never_sold = Project::table("Products Never Sold");
    p.edit(
        &orders,
        "DIVIDE([Gross Margin], [Total Sales])",
        "DIVIDE([Gross Margin], [Total Sale])",
    );
    p.edit(&orders, "MAX(Orders[Order Date])", "MAX(Orders[Order Dat])");
    p.edit(
        &orders,
        "KEEPFILTERS(Orders[Status] = \"Completed\")",
        "KEEPFILTERS(Orders[Status] = )",
    );
    // A multi-line measure: the expression starts on the next line.
    p.edit(
        &orders,
        "measure 'Units Sold' = SUM(Orders[Quantity])\n",
        "measure 'Units Sold' =\n\t\t\tVAR n = SUM(Order[Quantity])\n\t\t\tRETURN n + 0\n",
    );
    p.edit(
        &never_sold,
        "SELECTCOLUMNS(ALL(Products), \"Product ID\", Products[Product ID])",
        "SELECTCOLUMNS(ALL(Products), \"Product ID\", Products[Product IDD])",
    );
    let report = p.lint();
    let dax = of(&report, rules::DAX_REF);
    assert_eq!(dax.len(), 4, "{}", show(&report));
    let find = |needle: &str| {
        *dax.iter()
            .find(|f| f.message.contains(needle))
            .unwrap_or_else(|| panic!("no LINT007 for {needle}:\n{}", show(&report)))
    };

    let f = find("unknown measure or column `[Total Sale]`");
    assert_eq!(f.severity, Severity::Error);
    assert!(f.message.starts_with("measure `Orders[Margin %]`"));
    assert_eq!(origin_of(f), (orders.clone(), None, Some(27)));
    assert_eq!(
        f.suggestion.as_deref(),
        Some("did you mean `[Total Sales]`?")
    );

    // Line 95 in the example, + 2 for the multi-line measure above it.
    let f = find("unknown column `Orders[Order Dat]`");
    assert_eq!(origin_of(f).2, Some(97));
    assert_eq!(
        f.suggestion.as_deref(),
        Some("did you mean `Orders[Order Date]`?")
    );

    // `ORDER` is a DAX keyword, so the table name is quoted.
    let f = find("unknown table `'Order'`");
    assert!(f.message.contains("line 1, column 13"), "{}", f.message);
    assert_eq!(origin_of(f).2, Some(67));
    assert_eq!(
        f.suggestion.as_deref(),
        Some("did you mean table `Orders`?")
    );

    let f = find("unknown column `Products[Product IDD]`");
    assert!(f.message.starts_with("partition of `Products Never Sold`"));
    assert!(f.message.contains("(line 4, column"), "{}", f.message);
    assert_eq!(origin_of(f), (never_sold, None, Some(46)));
    assert_eq!(
        f.suggestion.as_deref(),
        Some("did you mean `Products[Product ID]`?")
    );

    let syntax = of(&report, rules::DAX_SYNTAX);
    assert_eq!(syntax.len(), 1, "{}", show(&report));
    assert!(
        syntax[0]
            .message
            .starts_with("measure `Orders[Completed Sales]`")
    );
    assert!(
        syntax[0]
            .message
            .contains("DAX syntax error at line 1, column")
    );
    assert_eq!(origin_of(syntax[0]), (orders, None, Some(39)));
}

#[test]
fn broken_model_structure() {
    let Some(p) = Project::new() else { return };
    let orders = Project::table("Orders");
    p.edit(
        "AcmeOutfitters.SemanticModel/definition/relationships.tmdl",
        "toColumn: Stores.'Store ID'",
        "toColumn: Stores.'Store IDs'",
    );
    p.edit(
        &Project::table("Last Refreshed"),
        "\tpartition 'Last Refreshed' = m\n\t\tmode: import\n\t\tqueryGroup: Sources\n",
        "\tpartition 'Last Refreshed' = m\n\t\tmode: import\n\t\tqueryGroup: Staging\n",
    );
    p.edit(
        &orders,
        "AzureStorage.Blobs(StorageContainerUrl)",
        "AzureStorage.Blobs(StorageContainerURL)",
    );
    p.edit(
        &orders,
        "Table.PromoteHeaders(#\"Source CSV\",",
        "Table.PromoteHeaders(#\"Source CVS\",",
    );
    p.edit(
        &Project::table("Stores"),
        "\tcolumn Channel\n",
        "\tcolumn Channel\n\t\tsortByColumn: 'Channel Order'\n",
    );
    let report = p.lint();

    let rel = of(&report, rules::RELATIONSHIP_REF);
    assert_eq!(rel.len(), 1, "{}", show(&report));
    assert!(
        rel[0]
            .message
            .contains("toColumn: column `Stores[Store IDs]` does not exist")
    );
    assert_eq!(
        origin_of(rel[0]),
        (
            "AcmeOutfitters.SemanticModel/definition/relationships.tmdl".to_owned(),
            None,
            Some(10)
        )
    );
    assert_eq!(
        rel[0].suggestion.as_deref(),
        Some("did you mean `Stores[Store ID]`?")
    );
    assert!(of(&report, rules::RELATIONSHIP_TYPE).is_empty());

    let groups = of(&report, rules::QUERY_GROUP_REF);
    assert_eq!(groups.len(), 1, "{}", show(&report));
    assert_eq!(groups[0].severity, Severity::Error);
    assert!(groups[0].message.contains("query group `Staging`"));
    assert_eq!(origin_of(groups[0]).0, Project::table("Last Refreshed"));

    let m = of(&report, rules::M_REF);
    assert_eq!(m.len(), 2, "{}", show(&report));
    let case = m
        .iter()
        .find(|f| f.message.contains("`StorageContainerURL`"))
        .unwrap();
    assert_eq!(case.severity, Severity::Warning);
    assert!(case.message.contains("case-sensitive"));
    assert!(case.message.contains("(line 2, column"), "{}", case.message);
    assert_eq!(
        case.suggestion.as_deref(),
        Some("use `StorageContainerUrl`")
    );
    assert_eq!(origin_of(case), (orders.clone(), None, Some(260)));
    let step = m
        .iter()
        .find(|f| f.message.contains("`Source CVS`"))
        .unwrap();
    assert_eq!(
        step.suggestion.as_deref(),
        Some("did you mean `Source CSV`?")
    );

    let sort_by = of(&report, rules::MODEL_COLUMN_REF);
    assert_eq!(sort_by.len(), 1, "{}", show(&report));
    assert!(
        sort_by[0]
            .message
            .contains("sorts by `Stores[Channel Order]`")
    );
    assert_eq!(
        sort_by[0].suggestion.as_deref(),
        Some("did you mean `Stores[Channel]`?")
    );
}

// ---------------------------------------------------------------------------
// In-memory documents
// ---------------------------------------------------------------------------

fn tiny_document() -> Document {
    let mut stores = Table::new("Stores");
    stores
        .columns
        .push(Column::data("StoreKey", DataType::String));
    stores
        .columns
        .push(Column::data("Region", DataType::String));
    let mut sessions = Table::new("Web Sessions");
    sessions
        .columns
        .push(Column::data("StoreKey", DataType::Int64));
    sessions
        .columns
        .push(Column::data("Device", DataType::String));
    let model = Model {
        tables: vec![stores, sessions],
        relationships: vec![Relationship::new(
            "r1",
            ColumnRef::new("Web Sessions", "StoreKey"),
            ColumnRef::new("Stores", "StoreKey"),
        )],
        ..Model::default()
    };
    let visual = Visual {
        name: "v1".into(),
        visual_type: "tableEx".into(),
        position: Position {
            width: 100.0,
            height: 100.0,
            ..Position::default()
        },
        projections: vec![
            Projection {
                role: "Values".into(),
                field: FieldExpr::column("Stors", "Device"),
                query_ref: "Stors.Device".into(),
                ..Projection::default()
            },
            Projection {
                role: "Values".into(),
                field: FieldExpr::column("Stores", "Device"),
                query_ref: "Stores.Device".into(),
                ..Projection::default()
            },
        ],
        ..Visual::default()
    };
    let mut doc = Document {
        model: Some(model),
        model_status: ModelStatus::Local,
        reports: vec![Report {
            name: "r".into(),
            pages: vec![Page {
                name: "p".into(),
                visuals: vec![visual],
                ..Page::default()
            }],
            ..Report::default()
        }],
        ..Document::default()
    };
    doc.assign_ids();
    doc
}

#[test]
fn lint_document_checks_in_memory_documents() {
    let doc = tiny_document();
    let report = lint_document(&doc, &LintOptions::default());

    let tables = of(&report, rules::MISSING_TABLE);
    assert_eq!(tables.len(), 1, "{}", show(&report));
    assert!(tables[0].message.contains("table `Stors` does not exist"));
    assert_eq!(
        tables[0].suggestion.as_deref(),
        Some("did you mean table `Stores`?")
    );
    assert!(tables[0].origin.is_none());
    assert!(tables[0].object.is_some());

    let fields = of(&report, rules::MISSING_FIELD);
    assert_eq!(fields.len(), 1, "{}", show(&report));
    assert_eq!(
        fields[0].suggestion.as_deref(),
        Some("`Device` exists on table `Web Sessions`: `'Web Sessions'[Device]`")
    );

    let types = of(&report, rules::RELATIONSHIP_TYPE);
    assert_eq!(types.len(), 1, "{}", show(&report));
    assert_eq!(types[0].severity, Severity::Warning);
    assert!(types[0].message.contains("(int64)"));
    assert!(types[0].message.contains("(string)"));

    let diags = report.to_diagnostics();
    assert_eq!(diags.len(), report.findings.len());
    assert!(
        diags
            .iter()
            .any(|d| d.code == "LINT001" && d.message.ends_with("(did you mean table `Stores`?)"))
    );

    let mut missing = doc;
    missing.model = None;
    missing.model_status = ModelStatus::Missing;
    let report = lint_document(&missing, &LintOptions::default());
    assert_eq!(report.summary.errors, 0, "{}", show(&report));
    assert!(
        report
            .summary
            .notes
            .iter()
            .any(|n| n.contains("field references not checked"))
    );
}
