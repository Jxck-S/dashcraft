//! End-to-end tests of the `dashcraft` binary (and its `pbiview` alias) over
//! the committed fixtures: the synthetic Acme Outfitters example and the
//! Microsoft samples.
//!
//! The binary runs with the fixture root (`$PBIVIEW_REFERENCE`, else
//! `<workspace>/reference`) as its working directory and gets relative
//! paths, so `source.path` and every other path in the output (and in the
//! snapshots) is machine-independent. Missing fixtures are skipped with a
//! `SKIP:` message.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::{ACME, MS_PBIX, PBIP, copy_dir, ms_expected};
use pretty_assertions::assert_eq;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const SEMANTIC_MODEL: &str = "examples/acme-outfitters/AcmeOutfitters.SemanticModel";
const REPORTS: [(&str, &str); 3] = [
    (
        "Acme Outfitters - Overview",
        "examples/acme-outfitters/Acme Outfitters - Overview.Report",
    ),
    (
        "Acme Outfitters - Stores",
        "examples/acme-outfitters/Acme Outfitters - Stores.Report",
    ),
    (
        "Acme Outfitters - Online",
        "examples/acme-outfitters/Acme Outfitters - Online.Report",
    ),
];

fn reference_root() -> PathBuf {
    std::env::var_os("PBIVIEW_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
}

fn pbiview_cmd(args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dashcraft"));
    cmd.args(args)
        .current_dir(reference_root())
        .env_remove("RUST_LOG")
        .env_remove("DASHCRAFT_APP")
        .env_remove("PBIVIEW_APP");
    cmd
}

fn pbiview(args: &[&str]) -> Output {
    pbiview_cmd(args).output().expect("run pbiview")
}

fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).expect("UTF-8 stdout")
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn assert_exit(o: &Output, code: i32) {
    assert_eq!(
        o.status.code(),
        Some(code),
        "exit status; stderr:\n{}",
        stderr(o)
    );
}

/// `inspect --json <path> <extra…>`, which must succeed (exit 0).
fn inspect_json(path: &str, extra: &[&str]) -> Value {
    let mut args = vec!["inspect", "--json", path];
    args.extend_from_slice(extra);
    let o = pbiview(&args);
    assert_exit(&o, 0);
    serde_json::from_str(&stdout(&o)).expect("valid JSON")
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap()
}

fn optional_fixture(rel: &str) -> Option<PathBuf> {
    let p = reference_root().join(rel);
    if p.exists() {
        Some(p)
    } else {
        eprintln!("SKIP: {} missing", p.display());
        None
    }
}

/// A fresh folder under `target/tmp`, removed when dropped.
fn tmp_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("pbiview-cli-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap()
}

fn path_arg(p: &Path) -> &str {
    p.to_str().expect("UTF-8 path")
}

fn arr(v: &Value) -> &Vec<Value> {
    v.as_array()
        .unwrap_or_else(|| panic!("expected an array, got {v}"))
}

fn count(v: &Value, key: &str) -> u64 {
    v["counts"][key]
        .as_u64()
        .unwrap_or_else(|| panic!("counts.{key}"))
}

/// Every visual of every report has a non-empty `visualType`.
fn assert_visual_types(v: &Value) {
    let mut seen = 0;
    for r in arr(&v["reports"]) {
        for p in arr(&r["pages"]) {
            for vis in arr(&p["visuals"]) {
                let t = vis["visualType"].as_str().unwrap_or_default();
                assert!(!t.is_empty(), "visual without a type: {vis}");
                seen += 1;
            }
        }
    }
    assert_eq!(seen, count(v, "visuals"));
}

/// The parts of the output that compare across formats.
fn model_counts(v: &Value) -> Vec<(&'static str, u64)> {
    [
        "tables",
        "columns",
        "calculatedColumns",
        "calculatedTables",
        "measures",
        "relationships",
        "expressions",
        "parameters",
    ]
    .into_iter()
    .map(|k| (k, count(v, k)))
    .collect()
}

fn page_shape(report: &Value) -> Vec<(String, usize)> {
    arr(&report["pages"])
        .iter()
        .map(|p| {
            (
                p["displayName"].as_str().unwrap().to_owned(),
                arr(&p["visuals"]).len(),
            )
        })
        .collect()
}

fn find_report<'a>(v: &'a Value, name: &str) -> &'a Value {
    arr(&v["reports"])
        .iter()
        .find(|r| r["name"] == name)
        .unwrap_or_else(|| panic!("report {name} not found"))
}

// ---------------------------------------------------------------------------
// inspect --json
// ---------------------------------------------------------------------------

#[test]
fn inspect_json_pbip_lists_everything() {
    let v = inspect_json(PBIP, &[]);
    assert_eq!(v["schemaVersion"], 1);
    assert_eq!(v["source"]["path"], PBIP);
    assert_eq!(v["source"]["format"], "pbip");
    assert_eq!(v["source"]["detection"]["format"], "pbip");
    assert_eq!(v["modelStatus"]["status"], "local");

    let model = &v["model"];
    let tables = arr(&model["tables"]);
    let names: Vec<&str> = tables.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "Orders",
            "Customers",
            "Products",
            "Stores",
            "Targets",
            "Returns",
            "Web Sessions",
            "Date",
            "Time Calc",
            "Products Never Sold",
            "Last Refreshed"
        ]
    );
    assert_eq!(count(&v, "tables"), 11);
    assert_eq!(count(&v, "measures"), 33);
    let measures: Vec<(&str, usize)> = tables
        .iter()
        .map(|t| (t["name"].as_str().unwrap(), arr(&t["measures"]).len()))
        .filter(|(_, n)| *n > 0)
        .collect();
    assert_eq!(
        measures,
        [
            ("Orders", 20),
            ("Customers", 1),
            ("Targets", 3),
            ("Returns", 3),
            ("Web Sessions", 5),
            ("Products Never Sold", 1)
        ]
    );
    for t in tables {
        for m in arr(&t["measures"]) {
            assert!(m["expression"].as_str().is_some_and(|x| !x.is_empty()));
        }
    }
    let kinds: Vec<(&str, &str)> = tables
        .iter()
        .filter(|t| t["kind"] != "regular")
        .map(|t| (t["name"].as_str().unwrap(), t["kind"].as_str().unwrap()))
        .collect();
    assert_eq!(
        kinds,
        [
            ("Date", "calculatedTable"),
            ("Time Calc", "calculationGroup"),
            ("Products Never Sold", "calculatedTable")
        ]
    );
    let never_sold = &tables[9];
    assert_eq!(never_sold["partitions"][0]["kind"], "calculated");

    assert_eq!(count(&v, "relationships"), 8);
    let rel = &model["relationships"][0];
    assert_eq!(rel["from"]["table"], "Orders");
    assert_eq!(rel["to"]["table"], "Customers");
    assert_eq!(rel["fromCardinality"], "many");
    assert_eq!(rel["toCardinality"], "one");
    assert_eq!(rel["crossFilteringBehavior"], "bothDirections");

    let param = &model["expressions"][0];
    assert_eq!(param["name"], "StorageContainerUrl");
    assert_eq!(param["isParameter"], true);
    assert_eq!(param["parameterType"], "Text");

    assert_eq!(count(&v, "reports"), 3);
    assert_visual_types(&v);
    let stores = find_report(&v, "Acme Outfitters - Stores");
    let filter = arr(&stores["filters"])
        .iter()
        .find(|f| f["field"]["table"] == "Stores" && f["field"]["name"] == "Channel")
        .expect("report-level Channel filter");
    assert_eq!(filter["kind"], "Categorical");
    assert_eq!(filter["field"]["ref"], "Stores[Channel]");
    assert_eq!(
        filter["condition"],
        "Stores[Channel] IN ('Retail', 'Outlet')"
    );
    let overview = find_report(&v, "Acme Outfitters - Overview");
    assert_eq!(
        overview["filters"][0]["condition"],
        "NOT Orders[Status] IN ('Cancelled')"
    );
    assert_eq!(count(&v, "errors"), 0);
    assert_eq!(count(&v, "warnings"), 0);

    insta::assert_snapshot!("inspect_pbip", pretty(&v));
}

fn diagnostic_codes(v: &Value) -> Vec<&str> {
    arr(&v["diagnostics"])
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect()
}

#[test]
fn inspect_json_report_folders_load_their_byref_model() {
    let project = inspect_json(PBIP, &[]);
    for (name, path) in REPORTS {
        let v = inspect_json(path, &[]);
        assert_eq!(v["source"]["format"], "reportFolder", "{path}");
        assert_eq!(v["modelStatus"]["status"], "local", "{path}");
        assert_eq!(model_counts(&v), model_counts(&project), "{path}");
        assert_eq!(count(&v, "reports"), 1);
        assert_visual_types(&v);
        let report = &v["reports"][0];
        assert_eq!(report["name"], name);
        assert_eq!(report["datasetReference"]["kind"], "byPath");
        assert_eq!(
            report["datasetReference"]["path"],
            "../AcmeOutfitters.SemanticModel"
        );
        assert_eq!(report, find_report(&project, name), "{path}");
        // The model's own diagnostics, then "loaded the model by path".
        let mut codes = diagnostic_codes(&project);
        codes.push("CORE016");
        assert_eq!(diagnostic_codes(&v), codes, "{path}");
    }
}

#[test]
fn inspect_json_semantic_model_folder() {
    let v = inspect_json(SEMANTIC_MODEL, &[]);
    let project = inspect_json(PBIP, &[]);
    assert_eq!(v["source"]["format"], "semanticModelFolder");
    assert_eq!(v["model"]["source"]["kind"], "tmdl");
    assert_eq!(v["model"]["source"]["folder"], "");
    assert_eq!(model_counts(&v), model_counts(&project));
    assert_eq!(v["model"]["tables"], project["model"]["tables"]);
    assert_eq!(
        v["model"]["relationships"],
        project["model"]["relationships"]
    );
    assert_eq!(v["model"]["expressions"], project["model"]["expressions"]);
    assert_eq!(arr(&v["reports"]).len(), 0);
    assert_eq!(diagnostic_codes(&v), diagnostic_codes(&project));
    assert_eq!((count(&v, "errors"), count(&v, "warnings")), (0, 0));
}

/// `dashcraft <args>` run in `dir` (relative paths stay machine-independent).
fn pbiview_in(dir: &Path, args: &[&str]) -> Output {
    pbiview_cmd(args)
        .current_dir(dir)
        .output()
        .expect("run pbiview")
}

/// A copy of the example's Overview report folder whose `definition.pbir`
/// points at a semantic model in the service (`byConnection`), as a report
/// downloaded from the service has it: `<dir>/Remote.Report`.
fn remote_report_folder(dir: &Path) -> PathBuf {
    let report = dir.join("Remote.Report");
    copy_dir(
        &reference_root().join(format!("{ACME}/Acme Outfitters - Overview.Report")),
        &report,
    );
    std::fs::write(
        report.join("definition.pbir"),
        r#"{
  "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definitionProperties/2.0.0/schema.json",
  "version": "4.0",
  "datasetReference": {
    "byConnection": {
      "connectionString": "Data Source=\"powerbi://api.powerbi.com/v1.0/acme-workspace/Acme Outfitters\";initial catalog=AcmeOutfitters;integrated security=ClaimsToken;semanticmodelid=00000000-0000-4000-8000-000000000000"
    }
  }
}
"#,
    )
    .unwrap();
    report
}

#[test]
fn inspect_json_remote_report_folder() {
    let dir = tmp_dir();
    remote_report_folder(dir.path());
    let o = pbiview_in(dir.path(), &["inspect", "--json", "Remote.Report"]);
    assert_exit(&o, 0);
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["source"]["path"], "Remote.Report");
    assert_eq!(v["source"]["format"], "reportFolder");
    assert_eq!(v["modelStatus"]["status"], "remote");
    assert_eq!(
        v["modelStatus"]["description"],
        "remote, not available offline"
    );
    assert!(v["model"].is_null());
    assert_eq!(count(&v, "tables"), 0);
    assert_eq!(count(&v, "reports"), 1);
    assert_eq!(v["reports"][0]["datasetReference"]["kind"], "byConnection");
    assert_visual_types(&v);
    // The report itself reads the same as through the project.
    let project = inspect_json(PBIP, &[]);
    let overview = find_report(&project, "Acme Outfitters - Overview");
    assert_eq!(page_shape(&v["reports"][0]), page_shape(overview));
    assert_eq!(count(&v, "pages"), arr(&overview["pages"]).len() as u64);
}

/// A committed Microsoft sample PBIX extracted into `<dir>/Unzipped`.
fn unzipped_pbix(dir: &Path) -> PathBuf {
    let out = dir.join("Unzipped");
    let file = std::fs::File::open(reference_root().join(MS_PBIX)).unwrap();
    zip::ZipArchive::new(file).unwrap().extract(&out).unwrap();
    out
}

#[test]
fn inspect_json_unzipped_pbix_folder() {
    if optional_fixture(MS_PBIX).is_none() {
        return;
    }
    let dir = tmp_dir();
    unzipped_pbix(dir.path());
    let run = |extra: &[&str]| {
        let mut args = vec!["inspect", "--json", "Unzipped"];
        args.extend_from_slice(extra);
        let o = pbiview_in(dir.path(), &args);
        assert_exit(&o, 0);
        serde_json::from_str::<Value>(&stdout(&o)).unwrap()
    };
    // Without the schema the output does not depend on the `DataModel`
    // (except the detection signals and diagnostics).
    let v = run(&["--no-pbix-schema"]);
    assert_eq!(v["source"]["format"], "pbix");
    assert_eq!(v["source"]["packageVersion"], "1.32");
    assert!(v["model"].is_null());
    assert_eq!(count(&v, "reports"), 1);
    assert_eq!(v["reports"][0]["datasetReference"]["kind"], "embedded");
    assert!(count(&v, "pages") > 0);
    assert_visual_types(&v);
    insta::assert_snapshot!("inspect_pbix_unzipped_reports", pretty(&v["reports"]));

    let full = run(&[]);
    assert_eq!(full["modelStatus"]["status"], "local");
    assert_eq!(full["model"]["source"]["kind"], "abf");
    assert!(count(&full, "tables") > 0);
    assert!(count(&full, "columns") > 0);
    assert_eq!(full["reports"], v["reports"]);
    assert_eq!(
        model_counts(&full),
        model_counts(&inspect_json(MS_PBIX, &[]))
    );
}

/// Every committed Microsoft sample: format, model counts and report shape
/// as recorded in `samples/microsoft/expected.json`.
#[test]
fn inspect_json_microsoft_samples_match_their_facts() {
    let Some(files) = ms_expected() else { return };
    let mut seen = 0;
    for facts in &files {
        let file = facts["file"].as_str().unwrap();
        if facts["committed"] != true {
            continue;
        }
        seen += 1;
        let v = inspect_json(file, &[]);
        assert_eq!(v["source"]["format"], facts["detect"]["format"], "{file}");
        assert_eq!(
            v["model"]["source"]["kind"], facts["model"]["source"],
            "{file}"
        );
        for (key, want) in facts["model"]["counts"].as_object().unwrap() {
            assert_eq!(&v["counts"][key.as_str()], want, "{file}: {key}");
        }
        assert_eq!(count(&v, "errors"), 0, "{file}");
        assert_visual_types(&v);
        let report = &v["reports"][0];
        let pages: Vec<(String, usize)> = facts["report"]["pages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                (
                    p["displayName"].as_str().unwrap().to_owned(),
                    p["visuals"].as_u64().unwrap() as usize,
                )
            })
            .collect();
        assert_eq!(page_shape(report), pages, "{file}");
    }
    assert_eq!(seen, 4, "the four committed samples");
}

#[test]
fn no_expressions_removes_dax_and_m_text() {
    let leaks = [
        "DISTINCTCOUNTNOBLANK",
        "CALCULATE(",
        "RELATEDTABLE",
        "SELECTCOLUMNS",
        "AzureStorage.Blobs",
        "Csv.Document",
        "acmeoutfittersdemo.blob.core.windows.net",
    ];
    let full = stdout(&pbiview(&["inspect", "--json", PBIP]));
    for leak in leaks {
        assert!(full.contains(leak), "fixture should contain {leak}");
    }

    let o = pbiview(&["inspect", "--json", "--no-expressions", PBIP]);
    assert_exit(&o, 0);
    let text = stdout(&o);
    for leak in leaks {
        assert!(!text.contains(leak), "--no-expressions leaked {leak}");
    }
    let v: Value = serde_json::from_str(&text).unwrap();
    for t in arr(&v["model"]["tables"]) {
        for key in ["columns", "measures", "partitions"] {
            for item in arr(&t[key]) {
                assert!(item.get("expression").is_none(), "{item}");
            }
        }
    }
    for e in arr(&v["model"]["expressions"]) {
        assert!(e.get("expression").is_none(), "{e}");
    }
    assert_eq!(count(&v, "measures"), 33);

    let o = pbiview(&["inspect", "--no-expressions", PBIP]);
    assert_exit(&o, 0);
    let text = stdout(&o);
    assert!(text.contains("Total Sales"));
    for leak in leaks {
        assert!(!text.contains(leak), "--no-expressions leaked {leak}");
    }
}

// ---------------------------------------------------------------------------
// inspect (human-readable)
// ---------------------------------------------------------------------------

#[test]
fn inspect_tree_smoke() {
    let o = pbiview(&["inspect", PBIP]);
    assert_exit(&o, 0);
    let text = stdout(&o);
    for needle in [
        "examples/acme-outfitters/AcmeOutfitters.pbip",
        "Format: PBIP project (pbip)",
        "Table: Orders",
        "Table: Web Sessions",
        "Table: Products Never Sold (calculated table)",
        "Table: Time Calc (calculation group)",
        "Table: Last Refreshed (table, hidden)",
        "Total Sales = SUM(Orders[Line Amount])",
        "Orders[Customer ID] (many) → Customers[Customer ID] (one), cross-filter bothDirections",
        "Orders[Ship Date] (many) → Date[Date] (one), cross-filter oneDirection, inactive",
        "StorageContainerUrl (parameter, Text) = \"https://",
        "Roles (2):",
        "Store Manager (read)",
        "Reports (3):",
        "Report: Acme Outfitters - Stores",
        "Stores[Channel] (Categorical): Stores[Channel] IN ('Retail', 'Outlet')",
        "Page: Store Performance [",
        "cardVisual at (",
        "Data: Targets[Target Attainment %]",
        "Diagnostics: errors 0, warnings 0",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
    }
}

/// A synthetic sample of `pbi-core` (FORMATS.md §5 readers).
fn format_sample(rel: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../pbi-core/tests/data/formats")
        .join(rel)
        .to_string_lossy()
        .into_owned()
}

#[test]
fn inspect_other_formats_adds_their_sections() {
    // RDL: outline with the connection string redacted, no model, remote status.
    let rdl = inspect_json(&format_sample("Sales.rdl"), &[]);
    assert_eq!(rdl["source"]["format"], "rdl");
    assert_eq!(rdl["model"], Value::Null);
    assert_eq!(rdl["modelStatus"]["status"], "remote");
    let pr = &rdl["paginatedReport"];
    assert_eq!(pr["schema"], "2016/01");
    assert_eq!(pr["dataSources"][0]["connectionString"], "<redacted>");
    assert_eq!(pr["datasets"][0]["fields"][0]["name"], "Region");
    assert!(pr["datasets"][0]["commandText"].is_string());
    assert_eq!(pr["sections"][0]["body"][1]["kind"], "tablix");
    assert!(!rdl.to_string().contains("fake-test-password"));
    for absent in ["vpaxStats", "mashup", "customVisual", "connector"] {
        assert!(rdl.get(absent).is_none(), "{absent}");
    }
    let no_expr = inspect_json(&format_sample("Sales.rdl"), &["--no-expressions"]);
    assert!(
        no_expr["paginatedReport"]["datasets"][0]
            .get("commandText")
            .is_none()
    );

    // M script and dataflow: queries in the model, details in `mashup`.
    let pq = inspect_json(&format_sample("Queries.pq"), &[]);
    assert_eq!(pq["mashup"]["kind"], "script");
    assert_eq!(arr(&pq["model"]["expressions"]).len(), 3);
    assert_eq!(count(&pq, "parameters"), 1);
    let df = inspect_json(&format_sample("dataflow/model.json"), &[]);
    assert_eq!(df["mashup"]["kind"], "dataflowCdm");
    assert_eq!(df["mashup"]["entities"][0]["attributes"], 6);
    assert_eq!(df["model"]["tables"][0]["partitions"][0]["kind"], "m");
    assert!(!df.to_string().contains("SECRETSIGNATURE"));

    // VPAX statistics.
    let dir = tmp_dir();
    let vpax = dir.path().join("Stats.vpax");
    let mut w = zip::ZipWriter::new(std::fs::File::create(&vpax).unwrap());
    w.start_file("DaxModel.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    w.write_all(
        br#"{"$id":"1","ModelName":"M","Tables":{"$id":"2","$values":[{"$id":"3","TableName":"T","RowsCount":7,"Columns":{"$id":"4","$values":[{"$id":"5","ColumnName":"C","ColumnCardinality":7,"DictionarySize":10,"DataSize":20,"Encoding":"HASH"}]}}]}}"#,
    )
    .unwrap();
    w.finish().unwrap();
    let v = inspect_json(path_arg(&vpax), &[]);
    assert_eq!(v["source"]["format"], "vpax");
    let stats = &v["vpaxStats"];
    assert_eq!(stats["source"], "DaxModel.json");
    assert_eq!(stats["totalSize"], 30);
    assert_eq!(stats["tables"][0]["rowsCount"], 7);
    assert_eq!(stats["tables"][0]["columns"][0]["cardinality"], 7);
    let o = pbiview(&["inspect", path_arg(&vpax)]);
    assert_exit(&o, 0);
    let text = stdout(&o);
    assert!(
        text.contains("VertiPaq Analyzer statistics (DaxModel.json)"),
        "{text}"
    );
    assert!(text.contains("Table T: 7 rows, 30 bytes"), "{text}");
}

#[test]
fn inspect_tree_shows_paginated_and_mashup_outlines() {
    let o = pbiview(&["inspect", &format_sample("Sales.rdl")]);
    assert_exit(&o, 0);
    let text = stdout(&o);
    for needle in [
        "Paginated report (RDL 2016/01), outline only:",
        "SalesDb (SQL, connection string <redacted>)",
        "SalesByRegion on SalesDb: SELECT Region, SUM(Amount) AS Amount",
        "Year (Integer, prompt \"Year\", default 2026, values from Years)",
        "tablix SalesTable [SalesByRegion]",
        "textbox RegionCell = =Fields!Region.Value",
        "subreport Details → RegionDetails",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
    }
    let o = pbiview(&["inspect", &format_sample("dataflow/model.json")]);
    assert_exit(&o, 0);
    let text = stdout(&o);
    for needle in [
        "Power Query: Dataflow (CDM model.json) \"Sales Dataflow\", section Section1 (2 queries, not evaluated)",
        "Customers (entity)",
        "Entity Customers: 6 attribute(s), 1 partition(s)",
        "Part001 at https://store.example.test/df/Customers/",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
    }
}

// ---------------------------------------------------------------------------
// Errors and exit codes
// ---------------------------------------------------------------------------

#[test]
fn missing_path_is_a_fatal_error() {
    let o = pbiview(&["inspect", "--json", "examples/does-not-exist.pbix"]);
    assert_exit(&o, 1);
    assert!(o.stdout.is_empty());
    let err = stderr(&o);
    assert!(
        err.starts_with("dashcraft: error: cannot open examples/does-not-exist.pbix"),
        "{err}"
    );
    assert!(err.contains("no such file or directory"), "{err}");
}

#[test]
fn zip_slip_archive_is_refused() {
    let dir = tmp_dir();
    let evil = dir.path().join("evil.pbix");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&evil).unwrap());
    let opts = zip::write::SimpleFileOptions::default();
    zip.start_file("Version", opts).unwrap();
    zip.write_all(b"1\0.\x003\x002\0").unwrap();
    zip.start_file("../evil.txt", opts).unwrap();
    zip.write_all(b"owned").unwrap();
    zip.finish().unwrap();

    let o = pbiview(&["inspect", "--json", path_arg(&evil)]);
    assert_exit(&o, 1);
    assert!(o.stdout.is_empty());
    let err = stderr(&o);
    assert!(err.starts_with("dashcraft: error: cannot open"), "{err}");
    assert!(err.contains("refused"), "{err}");
    assert!(!dir.path().parent().unwrap().join("evil.txt").exists());
}

#[test]
fn error_diagnostics_exit_with_2() {
    let dir = tmp_dir();
    let folder = dir.path().join("Broken.SemanticModel");
    std::fs::create_dir_all(folder.join("definition")).unwrap();
    std::fs::write(folder.join("definition.pbism"), r#"{"version": "4.2"}"#).unwrap();
    std::fs::write(
        folder.join("definition/model.tmdl"),
        "model Model\n\t\t\tculture: en-US\n",
    )
    .unwrap();

    let o = pbiview(&["inspect", "--json", path_arg(&folder)]);
    assert_exit(&o, 2);
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert!(v["model"].is_null());
    assert!(count(&v, "errors") >= 1);
    let error = arr(&v["diagnostics"])
        .iter()
        .find(|d| d["severity"] == "error")
        .unwrap();
    assert_eq!(error["origin"]["path"], "definition/model.tmdl");
    assert!(stderr(&o).contains("error diagnostic"), "{}", stderr(&o));

    let o = pbiview(&["inspect", path_arg(&folder)]);
    assert_exit(&o, 2);
    assert!(stdout(&o).contains("  Errors:"));

    let o = pbiview(&["--quiet", "inspect", "--json", path_arg(&folder)]);
    assert_exit(&o, 2);
    assert!(o.stderr.is_empty(), "{}", stderr(&o));
}

#[test]
fn usage_errors_are_fatal_and_help_succeeds() {
    let o = pbiview(&["inspect"]);
    assert_exit(&o, 1);
    assert!(stderr(&o).contains("Usage"), "{}", stderr(&o));
    let o = pbiview(&["convert", PBIP, "--to", "pbix", "--out", "x"]);
    assert_exit(&o, 1);

    let o = pbiview(&["--help"]);
    assert_exit(&o, 0);
    let help = stdout(&o);
    for cmd in ["inspect", "detect", "open", "convert"] {
        assert!(help.contains(cmd), "{help}");
    }
    let o = pbiview(&["--version"]);
    assert_exit(&o, 0);
    assert!(stdout(&o).starts_with("dashcraft "));
}

#[test]
fn the_pbiview_alias_runs_the_same_program_under_its_name() {
    let alias = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_pbiview"))
            .args(args)
            .current_dir(reference_root())
            .env_remove("RUST_LOG")
            .env_remove("DASHCRAFT_APP")
            .env_remove("PBIVIEW_APP")
            .output()
            .expect("run pbiview")
    };
    let o = alias(&["--version"]);
    assert_exit(&o, 0);
    assert!(stdout(&o).starts_with("pbiview "), "{}", stdout(&o));
    let o = alias(&["--help"]);
    assert_exit(&o, 0);
    assert!(stdout(&o).contains("Usage: pbiview "), "{}", stdout(&o));

    let o = alias(&["detect", "--json", PBIP]);
    assert_exit(&o, 0);
    assert_eq!(stdout(&o), stdout(&pbiview(&["detect", "--json", PBIP])));

    let o = alias(&["inspect", "examples/does-not-exist.pbix"]);
    assert_exit(&o, 1);
    assert!(
        stderr(&o).starts_with("pbiview: error: cannot open"),
        "{}",
        stderr(&o)
    );
}

// ---------------------------------------------------------------------------
// detect
// ---------------------------------------------------------------------------

#[test]
fn detect_reports_format_and_items() {
    let o = pbiview(&["detect", "--json", PBIP]);
    assert_exit(&o, 0);
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["schemaVersion"], 1);
    assert_eq!(v["path"], PBIP);
    assert_eq!(v["sourceFormat"], "pbip");
    assert_eq!(v["detection"]["format"], "pbip");
    assert_eq!(v["detection"]["confidence"], "certain");
    assert_eq!(v["items"]["pbipFile"], "AcmeOutfitters.pbip");
    assert_eq!(arr(&v["items"]["semanticModels"]).len(), 1);
    assert_eq!(v["items"]["semanticModels"][0]["flavour"], "tmdl");
    let reports = arr(&v["items"]["reports"]);
    assert_eq!(reports.len(), 3);
    assert!(reports.iter().all(|r| r["dataset"] == "byPath"));

    let dir = tmp_dir();
    remote_report_folder(dir.path());
    let o = pbiview_in(dir.path(), &["detect", "Remote.Report"]);
    assert_exit(&o, 0);
    let text = stdout(&o);
    assert!(text.contains("PBIP report folder"), "{text}");
    assert!(text.contains("byConnection"), "{text}");

    let junk = dir.path().join("mystery.bin");
    std::fs::write(&junk, [0x00, 0x13, 0x37, 0xfe, 0x01, 0x02, 0xff]).unwrap();
    let o = pbiview(&["detect", "--json", path_arg(&junk)]);
    assert_exit(&o, 1);
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["detection"]["format"], "unknown");
    assert!(stderr(&o).contains("not a recognised Power BI format"));
}

// ---------------------------------------------------------------------------
// convert
// ---------------------------------------------------------------------------

#[test]
fn convert_pbip_to_pbit_round_trips_through_inspect() {
    let project = inspect_json(PBIP, &[]);
    let dir = tmp_dir();
    let pbit = dir.path().join("out/AcmeOutfitters.pbit");
    let o = pbiview(&["convert", PBIP, "--to", "pbit", "--out", path_arg(&pbit)]);
    assert_exit(&o, 0);
    assert!(
        stderr(&o).contains("a PBIT holds one report"),
        "{}",
        stderr(&o)
    );
    assert!(pbit.is_file());

    let v = inspect_json(path_arg(&pbit), &[]);
    assert_eq!(v["source"]["format"], "pbit");
    assert_eq!(model_counts(&v), model_counts(&project));
    assert_eq!(count(&v, "reports"), 1);
    let first = &project["reports"][0];
    assert_eq!(page_shape(&v["reports"][0]), page_shape(first));
    assert_eq!(count(&v, "errors"), 0);

    // Existing outputs are kept unless --force.
    let o = pbiview(&["convert", PBIP, "--to", "pbit", "--out", path_arg(&pbit)]);
    assert_exit(&o, 1);
    assert!(stderr(&o).contains("already exists; use --force"));

    let stores = "Acme Outfitters - Stores";
    let o = pbiview(&[
        "--quiet",
        "convert",
        PBIP,
        "--to",
        "pbit",
        "--out",
        path_arg(&pbit),
        "--force",
        "--report",
        "acme outfitters - stores",
    ]);
    assert_exit(&o, 0);
    assert!(o.stderr.is_empty(), "{}", stderr(&o));
    let v = inspect_json(path_arg(&pbit), &[]);
    let report = &v["reports"][0];
    assert_eq!(
        page_shape(report),
        page_shape(find_report(&project, stores))
    );
    assert_eq!(report["filters"][0]["field"]["ref"], "Stores[Channel]");

    let o = pbiview(&[
        "convert",
        PBIP,
        "--to",
        "pbit",
        "--out",
        path_arg(&pbit),
        "--force",
        "--report",
        "Nope",
    ]);
    assert_exit(&o, 1);
    assert!(stderr(&o).contains("no report named 'Nope'"));
}

#[test]
fn convert_to_pbip_writes_an_equivalent_project() {
    let project = inspect_json(PBIP, &[]);
    let dir = tmp_dir();
    let out = dir.path().join("project");
    let o = pbiview(&["convert", PBIP, "--to", "pbip", "--out", path_arg(&out)]);
    assert_exit(&o, 0);
    assert!(stderr(&o).contains("wrote PBIP project"));

    let v = inspect_json(path_arg(&out), &[]);
    assert_eq!(v["source"]["format"], "pbip");
    assert_eq!(model_counts(&v), model_counts(&project));
    assert_eq!(v["model"]["tables"], project["model"]["tables"]);
    assert_eq!(v["reports"], project["reports"]);

    let o = pbiview(&["convert", PBIP, "--to", "pbip", "--out", path_arg(&out)]);
    assert_exit(&o, 1);
    assert!(stderr(&o).contains("is not empty; use --force"));
    let o = pbiview(&[
        "convert",
        PBIP,
        "--to",
        "pbip",
        "--out",
        path_arg(&out),
        "--force",
    ]);
    assert_exit(&o, 0);
}

// ---------------------------------------------------------------------------
// open
// ---------------------------------------------------------------------------

#[test]
fn open_reports_a_missing_app_or_input() {
    let o = pbiview_cmd(&["open", PBIP])
        .env("DASHCRAFT_APP", "/nonexistent/dashcraft-app")
        .env("PBIVIEW_APP", "/nonexistent/pbiview-app")
        .output()
        .unwrap();
    assert_exit(&o, 1);
    let err = stderr(&o);
    assert!(err.starts_with("dashcraft: error: DASHCRAFT_APP"), "{err}");

    // The former variable still works when the new one is not set.
    let o = pbiview_cmd(&["open", PBIP])
        .env("PBIVIEW_APP", "/nonexistent/pbiview-app")
        .output()
        .unwrap();
    assert_exit(&o, 1);
    let err = stderr(&o);
    assert!(err.starts_with("dashcraft: error: PBIVIEW_APP"), "{err}");

    let o = pbiview(&["open", "examples/does-not-exist.pbix"]);
    assert_exit(&o, 1);
    assert!(stderr(&o).contains("no such file or directory"));
}

#[cfg(unix)]
#[test]
fn open_passes_the_path_as_one_argument() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tmp_dir();
    let app = dir.path().join("fake-app.sh");
    let record = dir.path().join("args.txt");
    std::fs::write(
        &app,
        "#!/bin/sh\nprintf '%s\\n' \"$#\" \"$1\" > \"$PBIVIEW_TEST_RECORD.tmp\"\nmv \"$PBIVIEW_TEST_RECORD.tmp\" \"$PBIVIEW_TEST_RECORD\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o755)).unwrap();

    let (_, report) = REPORTS[1];
    let o = pbiview_cmd(&["open", report])
        .env("DASHCRAFT_APP", &app)
        .env("PBIVIEW_TEST_RECORD", &record)
        .output()
        .unwrap();
    assert_exit(&o, 0);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !record.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let recorded = std::fs::read_to_string(&record).expect("fake app ran");
    let lines: Vec<&str> = recorded.lines().collect();
    assert_eq!(lines[0], "1", "exactly one argument");
    let passed = Path::new(lines[1]);
    assert!(passed.is_absolute(), "{recorded}");
    assert!(passed.ends_with(report), "{recorded}");
    assert!(passed.is_dir());
}
