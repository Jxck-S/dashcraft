//! End-to-end tests of `dashcraft refresh`, `measures` and `export` over the
//! synthetic Acme Outfitters example, against its `expected.json`.
//!
//! Like `tests/cli.rs`, the binary runs with the fixture root as its working
//! directory. Model exports and offline refreshes need only the model
//! definition; data tests also use the example's CSVs (through its
//! `blob-map.json`). The local data cache case uses a committed Microsoft
//! sample. Outputs are written under `target/tmp` (`CARGO_TARGET_TMPDIR`)
//! and deleted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::{BLOB_MAP, Expected, MS_PBIX, PBIP, expected, ms_expected};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use pretty_assertions::assert_eq;
use serde_json::Value;

/// The CSVs behind the example's blob map.
const CSVS: [&str; 7] = [
    "examples/acme-outfitters/data/orders.csv",
    "examples/acme-outfitters/data/customers.csv",
    "examples/acme-outfitters/data/products.csv",
    "examples/acme-outfitters/data/stores.csv",
    "examples/acme-outfitters/data/targets.csv",
    "examples/acme-outfitters/data/returns.csv",
    "examples/acme-outfitters/data/web_sessions.csv",
];

fn reference_root() -> PathBuf {
    std::env::var_os("DASHCRAFT_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
}

fn dashcraft(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dashcraft"))
        .args(args)
        .current_dir(reference_root())
        .env_remove("RUST_LOG")
        .output()
        .expect("run dashcraft")
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

fn json(o: &Output) -> Value {
    serde_json::from_slice(&o.stdout).expect("JSON stdout")
}

/// `true` if the example's PBIP is there (else prints `SKIP`).
fn have_pbip() -> bool {
    let ok = reference_root().join(PBIP).exists();
    if !ok {
        println!("SKIP: {PBIP} missing");
    }
    ok
}

/// The expected values if the example's data is there (else prints `SKIP`).
fn have_data() -> Option<Expected> {
    if !have_pbip() {
        return None;
    }
    for csv in CSVS {
        if !reference_root().join(csv).exists() {
            println!("SKIP: {csv} missing");
            return None;
        }
    }
    expected()
}

/// A fresh output folder under `target/tmp`, deleted on drop.
struct TmpDir(PathBuf);

impl TmpDir {
    fn new(name: &str) -> Self {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("dashcraft-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TmpDir(dir)
    }

    fn file(&self, name: &str) -> String {
        self.0.join(name).to_str().unwrap().to_owned()
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn measure<'a>(out: &'a Value, name: &str) -> &'a Value {
    out["measures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == name)
        .unwrap_or_else(|| panic!("no measure {name}"))
}

// ---------------------------------------------------------------------------
// Model documentation (model definition only)
// ---------------------------------------------------------------------------

#[test]
fn export_model_markdown_to_stdout() {
    if !have_pbip() {
        return;
    }
    let o = dashcraft(&["export", PBIP, "--model", "--format", "md", "--out", "-"]);
    assert_exit(&o, 0);
    let md = stdout(&o);
    assert!(md.starts_with("# AcmeOutfitters\n"), "{md}");
    for needle in [
        "| Orders[Customer ID] → Customers[Customer ID] | many-to-one | both directions | yes |",
        "| Targets[Region] → Stores[Region] | many-to-many | single | yes |",
        "| Orders[Ship Date] → Date[Date] | many-to-one | single | no |",
        "Website and app sessions. No relationship to any other table",
        "| Total Sales | `\\$#,0.00` | Sales | Sum of the order line amounts. |",
        "```dax\nSUM(Orders[Line Amount])\n```",
        "```powerquery\nlet\n",
        "## Parameters",
        "## Roles",
    ] {
        assert!(md.contains(needle), "missing {needle:?}");
    }
    assert!(stderr(&o).is_empty(), "{}", stderr(&o));

    let outline = dashcraft(&[
        "export",
        PBIP,
        "--model",
        "--out",
        "-",
        "--format",
        "markdown",
        "--no-expressions",
        "--no-hidden",
        "--title",
        "Outline",
    ]);
    assert_exit(&outline, 0);
    let outline = stdout(&outline);
    assert!(outline.starts_with("# Outline\n"));
    assert!(!outline.contains("```"));
    assert!(outline.contains("\n### Orders\n"));
    assert!(!outline.contains("\n### Last Refreshed\n"));
}

#[test]
fn export_model_json_refuses_to_overwrite_without_force() {
    if !have_pbip() {
        return;
    }
    let tmp = TmpDir::new("model-json");
    let out = tmp.file("docs/model.json");
    let o = dashcraft(&["export", PBIP, "--model", "--out", &out]);
    assert_exit(&o, 0);
    assert!(stderr(&o).contains("wrote the model documentation"));
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(doc["schemaVersion"], 1);
    assert_eq!(doc["summary"]["tables"], 11);
    assert_eq!(
        doc["tables"][0]["measures"][0]["expression"],
        "SUM(Orders[Line Amount])"
    );
    assert_eq!(
        doc["relationships"][0]["crossFilteringBehavior"],
        "bothDirections"
    );

    let again = dashcraft(&["export", PBIP, "--model", "--out", &out, "--no-expressions"]);
    assert_exit(&again, 1);
    assert!(
        stderr(&again).contains("already exists; use --force"),
        "{}",
        stderr(&again)
    );
    let unchanged: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(unchanged, doc);

    let forced = dashcraft(&[
        "export",
        PBIP,
        "--model",
        "--out",
        &out,
        "--no-expressions",
        "--force",
        "-q",
    ]);
    assert_exit(&forced, 0);
    assert!(stderr(&forced).is_empty());
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(!text.contains("\"expression\""), "{text}");
    assert!(!text.contains("SUM(Orders[Line Amount])"));
    // Only the output file: the temporary file was renamed into place.
    let names: Vec<_> = std::fs::read_dir(Path::new(&out).parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["model.json"]);
}

#[test]
fn export_usage_errors_are_fatal() {
    if !have_pbip() {
        return;
    }
    let tmp = TmpDir::new("usage");
    let out = tmp.file("x.out");
    for (args, message) in [
        (
            vec![
                "export", PBIP, "--table", "Orders", "--format", "json", "--out", &out,
            ],
            "--table exports csv or parquet",
        ),
        (
            vec!["export", PBIP, "--model", "--format", "csv", "--out", &out],
            "--model exports json or md",
        ),
        (
            vec!["export", PBIP, "--model", "--out", &out],
            "cannot tell the format",
        ),
        (
            vec![
                "export", PBIP, "--table", "Orders", "--format", "parquet", "--out", "-",
            ],
            "Parquet is a binary format",
        ),
        (
            vec![
                "export",
                PBIP,
                "--table",
                "Orders",
                "--format",
                "parquet",
                "--formatted",
                "--out",
                &out,
            ],
            "--formatted only applies to --format csv",
        ),
        (
            vec![
                "export", PBIP, "--table", "Nope", "--format", "csv", "--out", &out,
            ],
            "no table named 'Nope'; tables: 'Orders', 'Customers', 'Products'",
        ),
        (vec!["export", PBIP, "--out", &out], "required"),
        (
            vec![
                "export", PBIP, "--model", "--table", "Orders", "--out", &out,
            ],
            "cannot be used with",
        ),
        (
            vec![
                "export", PBIP, "--model", "--format", "md", "--bom", "--out", &out,
            ],
            "--bom only applies to --format csv",
        ),
        (
            vec![
                "export",
                PBIP,
                "--table",
                "Orders",
                "--no-expressions",
                "--out",
                &out,
            ],
            "--no-expressions only applies to --model",
        ),
        (
            vec![
                "export",
                PBIP,
                "--table",
                "Orders",
                "--format",
                "parquet",
                "--excel-safe",
                "--out",
                &out,
            ],
            "--excel-safe only applies to --format csv",
        ),
        (
            vec![
                "export", PBIP, "--table", "Orders", "--source", "nope", "--out", &out,
            ],
            "invalid value 'nope'",
        ),
    ] {
        let o = dashcraft(&args);
        assert_exit(&o, 1);
        assert!(stderr(&o).contains(message), "{args:?}: {}", stderr(&o));
    }
    assert!(!Path::new(&out).exists());
}

// ---------------------------------------------------------------------------
// Refresh
// ---------------------------------------------------------------------------

#[test]
fn refresh_without_a_blob_map_fails_with_an_offline_hint() {
    if !have_pbip() {
        return;
    }
    let o = dashcraft(&["refresh", PBIP, "--json"]);
    assert_exit(&o, 2);
    let out = json(&o);
    assert_eq!(out["schemaVersion"], 1);
    assert_eq!(out["ok"], false);
    let orders = &out["tables"][0];
    assert_eq!(orders["name"], "Orders");
    assert_eq!(orders["kind"], "m");
    assert_eq!(orders["status"]["state"], "error");
    assert_eq!(orders["status"]["csvFallback"], true);
    assert!(
        orders["status"]["hint"]
            .as_str()
            .unwrap()
            .contains("offline")
    );
    assert_eq!(orders["columns"][0]["status"]["state"], "skipped");
    let last = &out["tables"][10];
    assert_eq!(last["name"], "Last Refreshed");
    assert_eq!(
        (last["rows"].as_u64(), &last["status"]["state"]),
        (Some(1), &Value::from("ok"))
    );
    // The seven tables read from the blob container fail; `Date` and `Time Calc`
    // (DAX) and `Last Refreshed` (no source) load; `Products Never Sold` needs Orders.
    assert_eq!(
        (
            &out["summary"]["ok"],
            &out["summary"]["failed"],
            &out["summary"]["skipped"]
        ),
        (&Value::from(3), &Value::from(7), &Value::from(1))
    );

    let o = dashcraft(&["refresh", PBIP, "--tables", "Last Refreshed"]);
    assert_exit(&o, 0);
    assert!(stdout(&o).contains("1 table in"), "{}", stdout(&o));

    let o = dashcraft(&["refresh", PBIP]);
    assert_exit(&o, 2);
    let text = stdout(&o);
    assert!(
        text.contains("hint: offline: pass --blob-map FILE"),
        "{text}"
    );
    assert!(text.contains("3 ok, 7 failed, 1 skipped"), "{text}");

    let o = dashcraft(&["refresh", PBIP, "--tables", "Nope"]);
    assert_exit(&o, 1);
    assert!(stderr(&o).contains("no table named 'Nope'"));

    // No local data cache: `auto` evaluates M, `cache` fails.
    let o = dashcraft(&["refresh", PBIP, "--source", "auto", "--json"]);
    assert_exit(&o, 2);
    assert_eq!(json(&o)["tables"][0]["kind"], "m");
    let o = dashcraft(&["refresh", PBIP, "--source", "cache"]);
    assert_exit(&o, 1);
    assert!(
        stderr(&o).contains("no local data cache (.pbi/cache.abf)"),
        "{}",
        stderr(&o)
    );
}

/// A committed Microsoft sample PBIX saved as a PBIP whose `.pbi/cache.abf`
/// holds the PBIX data (as Power BI Desktop would leave it), with the
/// sample's facts, or `None` (`SKIP`).
fn cached_pbip(tmp: &TmpDir) -> Option<(PathBuf, Value)> {
    let pbix = reference_root().join(MS_PBIX);
    if !pbix.exists() {
        println!("SKIP: {} missing", pbix.display());
        return None;
    }
    let facts = ms_expected()?
        .into_iter()
        .find(|f| f["file"] == MS_PBIX)
        .unwrap();
    let opened = pbi_core::open_path(&pbix, &pbi_core::OpenOptions::default()).unwrap();
    let project = tmp.0.join("project");
    pbi_core::save_as_pbip(&opened, &project, &pbi_core::SaveOptions::default()).unwrap();
    let Some(pbi_core::ModelSource::Abf { archive, .. }) = &opened.model_source else {
        panic!("the PBIX model is not read from its DataModel");
    };
    let entries: Vec<PathBuf> = std::fs::read_dir(&project)
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
    Some((find(".pbip"), facts))
}

#[test]
fn refresh_measures_and_export_from_the_local_data_cache() {
    let tmp = TmpDir::new("cache");
    let Some((pbip, facts)) = cached_pbip(&tmp) else {
        return;
    };
    let pbip = pbip.to_str().unwrap();
    let rows_of = |table: &str| {
        facts["model"]["tables"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == table)
            .unwrap()["rows"]
            .as_u64()
            .unwrap()
    };

    // `auto` (the default) loads the cache: no M, no blob map.
    let o = dashcraft(&["refresh", pbip, "--json"]);
    assert_exit(&o, 0);
    assert!(
        stderr(&o).contains("using the local data cache"),
        "{}",
        stderr(&o)
    );
    let out = json(&o);
    assert_eq!(out["ok"], true);
    let tables = out["tables"].as_array().unwrap();
    assert!(tables.iter().all(|t| t["kind"] == "cache"), "{out:#}");
    for t in tables {
        let name = t["name"].as_str().unwrap();
        assert_eq!(t["rows"].as_u64(), Some(rows_of(name)), "{name}");
    }

    let o = dashcraft(&["measures", pbip, "--source", "cache", "--json"]);
    assert_exit(&o, 0);
    let out = json(&o);
    let measures = out["measures"].as_array().unwrap();
    assert_eq!(
        measures.len() as u64,
        facts["model"]["measures"]["count"].as_u64().unwrap()
    );
    assert!(measures.iter().all(|m| m["error"].is_null()), "{out:#}");
    for (key, want) in facts["model"]["measures"]["values"].as_object().unwrap() {
        let name = key.split_once('[').unwrap().1.trim_end_matches(']');
        assert_eq!(measure(&out, name)["formatted"], want["formatted"], "{key}");
    }

    // `--source m` evaluates the queries instead.
    let o = dashcraft(&["refresh", pbip, "--source", "m", "--json"]);
    let out = json(&o);
    assert!(
        out["tables"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["kind"] != "cache"),
        "{out:#}"
    );

    // `--excel-safe` only prefixes text cells that look like formulas.
    let opportunities = rows_of("Opportunity");
    let (raw, safe) = (tmp.file("raw.csv"), tmp.file("safe.csv"));
    for (out, extra) in [(&raw, None), (&safe, Some("--excel-safe"))] {
        let mut args = vec![
            "export",
            pbip,
            "--table",
            "Opportunity",
            "--out",
            out.as_str(),
        ];
        args.extend(extra);
        let o = dashcraft(&args);
        assert_exit(&o, 0);
        assert!(
            stderr(&o).contains(&format!("wrote {opportunities} rows")),
            "{}",
            stderr(&o)
        );
    }
    let read = |path: &str| -> Vec<csv::StringRecord> {
        csv::ReaderBuilder::new()
            .has_headers(false)
            .from_path(path)
            .unwrap()
            .records()
            .map(Result::unwrap)
            .collect()
    };
    let (raw, safe) = (read(&raw), read(&safe));
    assert_eq!(raw.len(), safe.len());
    for (r, s) in raw.iter().zip(&safe) {
        for (a, b) in r.iter().zip(s) {
            let risky = a.starts_with(['=', '+', '-', '@', '\t', '\r']);
            if risky && a.parse::<f64>().is_err() {
                assert_eq!(b, format!("'{a}"));
            } else if !risky {
                assert_eq!(a, b);
            }
        }
    }
}

#[test]
fn refresh_and_measures_match_the_expected_values() {
    let Some(exp) = have_data() else {
        return;
    };
    let o = dashcraft(&["refresh", PBIP, "--blob-map", BLOB_MAP, "--json"]);
    assert_exit(&o, 0);
    let out = json(&o);
    assert_eq!(out["ok"], true);
    let rows: HashMap<String, u64> = out["tables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["name"].as_str().unwrap().to_owned(),
                t["rows"].as_u64().unwrap(),
            )
        })
        .collect();
    for (table, want) in exp.row_counts() {
        assert_eq!(rows[table], want, "{table}");
    }

    let o = dashcraft(&["measures", PBIP, "--blob-map", BLOB_MAP, "--json"]);
    assert_exit(&o, 0);
    let out = json(&o);
    assert_eq!(out["schemaVersion"], 1);
    assert_eq!(out["filters"], serde_json::json!([]));
    for (name, value, formatted) in exp.measures() {
        assert_eq!(measure(&out, name)["formatted"], formatted, "{name}");
        if let Some(want) = value.as_f64() {
            let got = measure(&out, name)["value"].as_f64().unwrap();
            assert!(
                (got - want).abs() < 1e-6 * want.abs().max(1.0),
                "{name}: {got} vs {want}"
            );
        }
    }

    for report in ["Overview", "Stores", "Online"] {
        let o = dashcraft(&["measures", PBIP, "--blob-map", BLOB_MAP, "--report", report]);
        assert_exit(&o, 0);
        let text = stdout(&o);
        assert!(
            text.starts_with(&format!("Report: {}\n", exp.report_name(report))),
            "{text}"
        );
        let line = |name: &str| {
            text.lines()
                .find(|l| l.trim_start().starts_with(&format!("{name}  ")))
                .map(|l| l.split_whitespace().last().unwrap_or_default().to_owned())
        };
        for (name, _, formatted) in exp.report_measures(report) {
            assert_eq!(
                line(name).as_deref(),
                Some(formatted),
                "{report} {name}:\n{text}"
            );
        }
    }
    let o = dashcraft(&[
        "measures",
        PBIP,
        "--blob-map",
        BLOB_MAP,
        "--report",
        "Stores",
    ]);
    assert!(
        stdout(&o).contains("Filter: Stores[Channel] IN (\"Retail\", \"Outlet\")"),
        "{}",
        stdout(&o)
    );
}

// ---------------------------------------------------------------------------
// Table export
// ---------------------------------------------------------------------------

#[test]
fn export_table_to_csv_and_parquet() {
    let Some(exp) = have_data() else {
        return;
    };
    let orders = exp.rows("Orders");
    let total_sales = exp.value("Total Sales").as_f64().unwrap();
    let tmp = TmpDir::new("table");
    let csv_path = tmp.file("orders.csv");
    let o = dashcraft(&[
        "export",
        PBIP,
        "--table",
        "orders",
        "--format",
        "csv",
        "--out",
        &csv_path,
        "--blob-map",
        BLOB_MAP,
    ]);
    assert_exit(&o, 0);
    assert!(
        stderr(&o).contains(&format!("wrote {orders} rows × 15 columns of 'Orders'")),
        "{}",
        stderr(&o)
    );
    let mut reader = csv::Reader::from_path(&csv_path).unwrap();
    let header = reader.headers().unwrap().clone();
    assert_eq!(header.len(), 15);
    assert_eq!(&header[0], "Order ID");
    assert_eq!(&header[10], "Line Amount");
    assert_eq!(&header[14], "Days To Ship");
    let records: Vec<csv::StringRecord> = reader.records().map(Result::unwrap).collect();
    assert_eq!(records.len() as u64, orders);
    // The exported line amounts add up to the Total Sales measure.
    let sum: f64 = records
        .iter()
        .map(|r| r[10].parse::<f64>().unwrap_or(0.0))
        .sum();
    assert!((sum - total_sales).abs() < 0.005, "{sum} vs {total_sales}");

    let formatted = tmp.file("orders-excel.csv");
    let o = dashcraft(&[
        "export",
        PBIP,
        "--table",
        "Orders",
        "--out",
        &formatted,
        "--formatted",
        "--bom",
        "--delimiter",
        ";",
        "--blob-map",
        BLOB_MAP,
    ]);
    assert_exit(&o, 0);
    let bytes = std::fs::read(&formatted).unwrap();
    assert!(bytes.starts_with("\u{feff}Order ID;Order Date;".as_bytes()));

    let parquet_path = tmp.file("orders.parquet");
    let o = dashcraft(&[
        "export",
        PBIP,
        "--table",
        "Orders",
        "--format",
        "parquet",
        "--compression",
        "snappy",
        "--out",
        &parquet_path,
        "--blob-map",
        BLOB_MAP,
    ]);
    assert_exit(&o, 0);
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(File::open(&parquet_path).unwrap()).unwrap();
    assert_eq!(builder.metadata().file_metadata().num_rows() as u64, orders);
    let kv: HashMap<String, String> = builder
        .metadata()
        .file_metadata()
        .key_value_metadata()
        .unwrap()
        .iter()
        .filter_map(|kv| Some((kv.key.clone(), kv.value.clone()?)))
        .collect();
    assert_eq!(
        kv.get("dashcraft.table").map(String::as_str),
        Some("Orders")
    );
    let columns: Value = serde_json::from_str(&kv["dashcraft.columns"]).unwrap();
    assert_eq!(columns[0]["name"], "Order ID");
    assert_eq!(
        columns[0]["description"],
        "Order number (SO-…). Unique after Table.Distinct."
    );
    let schema = builder.schema().clone();
    assert_eq!(schema.fields().len(), 15);
    assert_eq!(
        schema.field(6).data_type(),
        &arrow::datatypes::DataType::Int64,
        "Quantity"
    );
    let rows: usize = builder
        .build()
        .unwrap()
        .map(|b| b.unwrap().num_rows())
        .sum();
    assert_eq!(rows as u64, orders);
}
