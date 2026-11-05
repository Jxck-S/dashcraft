#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Differential tests against Power BI Desktop (ADR-0011).
//!
//! `reference/groundtruth/corpus/<suite>.json` lists DAX queries;
//! `reference/groundtruth/recordings/<suite>.json` holds what Power BI
//! Desktop's engine returned for them (recorded on Windows with
//! `scripts/groundtruth/Record-GroundTruth.ps1`). Each test runs a suite's
//! queries through DashCraft on the same fixture and compares the results
//! value by value. A mismatch fails the test unless
//! `reference/groundtruth/deviations.json` lists it with a reason; a listed
//! case that now matches fails too (the ledger only shrinks).
//!
//! Comparison: numbers across types with a 1e-9 relative/absolute
//! tolerance (decimals against decimals exactly), BLANK only equals BLANK,
//! text exactly, date/times to the millisecond, rows as a multiset unless
//! the query has `ORDER BY`, errors only by parity (both sides fail).
//! Volatile cases compare the columns and the row count only.
//!
//! A report per suite is written to `target/tmp/groundtruth/<suite>.md`,
//! and new mismatches as proposed ledger entries to
//! `target/tmp/groundtruth/<suite>.proposed.json`. A missing recording
//! prints `SKIP: <file> missing`.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{NaiveDateTime, TimeZone};
use pbi_model::{
    Cardinality, ColumnKind, CrossFilter, DataType, Model, SecurityFilteringBehavior, SummarizeBy,
};
use pbi_session::{
    CancellationToken, FilterSpec, FixedClock, RefreshOptions, Session, SessionOptions, Value,
    ViewAs,
};
use serde_json::Value as Json;

fn reference_root() -> PathBuf {
    std::env::var_os("DASHCRAFT_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
}

fn report_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp/groundtruth")
}

fn read_json(path: &Path) -> Json {
    serde_json::from_slice(&std::fs::read(path).unwrap())
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn fixed_clock() -> Arc<FixedClock> {
    Arc::new(FixedClock::utc(
        chrono::Utc
            .with_ymd_and_hms(2026, 10, 21, 14, 5, 0)
            .single()
            .unwrap(),
    ))
}

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

/// A result cell, from either side.
#[derive(Debug, Clone, PartialEq)]
enum Cell {
    Blank,
    Bool(bool),
    /// A number; `decimal` holds the exact value scaled by 10⁴ for
    /// currency/decimal values.
    Num {
        value: f64,
        decimal: Option<i128>,
    },
    Text(String),
    /// Milliseconds since 1970-01-01T00:00:00 (no time zone).
    DateTime(i64),
}

impl Cell {
    fn num(value: f64) -> Cell {
        Cell::Num {
            value,
            decimal: None,
        }
    }

    fn text(s: impl Into<String>) -> Cell {
        Cell::Text(s.into())
    }

    fn opt_text(s: Option<&str>) -> Cell {
        s.map_or(Cell::Blank, Cell::text)
    }

    fn from_value(v: &Value) -> Cell {
        match v {
            Value::Blank => Cell::Blank,
            Value::Boolean(b) => Cell::Bool(*b),
            Value::Integer(i) => Cell::num(*i as f64),
            Value::Double(d) => Cell::num(*d),
            Value::Decimal(d) => Cell::Num {
                value: *d as f64 / 10_000.0,
                decimal: Some(*d),
            },
            Value::String(s) => Cell::Text(s.to_string()),
            Value::DateTime(us) => Cell::DateTime(us.div_euclid(1000)),
        }
    }

    fn from_json(v: &Json) -> Result<Cell, String> {
        Ok(match v {
            Json::Null => Cell::Blank,
            Json::Bool(b) => Cell::Bool(*b),
            Json::Number(n) => Cell::num(n.as_f64().ok_or("number out of range")?),
            Json::String(s) => Cell::Text(s.clone()),
            Json::Object(o) => {
                if let Some(Json::String(d)) = o.get("decimal") {
                    Cell::Num {
                        value: d.parse().map_err(|_| format!("bad decimal {d}"))?,
                        decimal: parse_decimal(d),
                    }
                } else if let Some(Json::String(d)) = o.get("double") {
                    Cell::num(d.parse().map_err(|_| format!("bad double {d}"))?)
                } else if let Some(Json::String(d)) = o.get("dateTime") {
                    let dt = NaiveDateTime::parse_from_str(d, "%Y-%m-%dT%H:%M:%S%.f")
                        .map_err(|e| format!("bad dateTime {d}: {e}"))?;
                    Cell::DateTime(dt.and_utc().timestamp_millis())
                } else {
                    return Err(format!("unknown typed value {v}"));
                }
            }
            Json::Array(_) => return Err(format!("unexpected array {v}")),
        })
    }

    fn matches(&self, other: &Cell) -> bool {
        match (self, other) {
            (
                Cell::Num {
                    value: a,
                    decimal: da,
                },
                Cell::Num {
                    value: b,
                    decimal: db,
                },
            ) => match (da, db) {
                (Some(x), Some(y)) => x == y,
                _ => numbers_match(*a, *b),
            },
            (Cell::DateTime(a), Cell::DateTime(b)) => a == b,
            _ => self == other,
        }
    }

    /// Sort key for multiset comparison (numbers rounded so that values
    /// within the tolerance sort alike).
    fn sort_key(&self) -> String {
        match self {
            Cell::Blank => "0".into(),
            Cell::Bool(b) => format!("1{b}"),
            Cell::Num { value, .. } if value.is_finite() => format!("2{:+.6e}", round9(*value)),
            Cell::Num { value, .. } => format!("2{value}"),
            Cell::Text(s) => format!("3{s}"),
            Cell::DateTime(ms) => format!("4{ms:020}"),
        }
    }

    fn show(&self) -> String {
        match self {
            Cell::Blank => "BLANK".into(),
            Cell::Bool(b) => b.to_string().to_uppercase(),
            Cell::Num {
                decimal: Some(d), ..
            } => format!("{}d", *d as f64 / 10_000.0),
            Cell::Num { value, .. } => format!("{value}"),
            Cell::Text(s) => format!("{s:?}"),
            Cell::DateTime(ms) => chrono::DateTime::from_timestamp_millis(*ms).map_or_else(
                || format!("dt({ms})"),
                |d| d.naive_utc().format("%Y-%m-%dT%H:%M:%S%.3f").to_string(),
            ),
        }
    }
}

fn parse_decimal(s: &str) -> Option<i128> {
    let (neg, s) = s.strip_prefix('-').map_or((false, s), |r| (true, r));
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    if frac.len() > 4 {
        return None;
    }
    let mut v: i128 = int.parse().ok()?;
    v = v.checked_mul(10_000)?;
    let frac_v: i128 = format!("{frac:0<4}").parse().ok()?;
    v = v.checked_add(frac_v)?;
    Some(if neg { -v } else { v })
}

fn round9(v: f64) -> f64 {
    if v == 0.0 {
        return 0.0;
    }
    let digits = 9 - v.abs().log10().ceil() as i32;
    let f = 10f64.powi(digits.clamp(-300, 300));
    (v * f).round() / f
}

fn numbers_match(a: f64, b: f64) -> bool {
    if a.is_nan() || b.is_nan() {
        return a.is_nan() && b.is_nan();
    }
    if a.is_infinite() || b.is_infinite() {
        return a == b;
    }
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

// ---------------------------------------------------------------------------
// Results and comparison
// ---------------------------------------------------------------------------

struct Table {
    columns: Vec<String>,
    rows: Vec<Vec<Cell>>,
}

enum Outcome {
    Rows(Table),
    Error(String),
}

/// Compares DashCraft's result with Desktop's recorded case; `None` when
/// they agree, else `(kind, detail)`.
fn compare(case: &Json, recorded: &Json, ours: &Outcome) -> Option<(&'static str, String)> {
    let query = case["query"].as_str().unwrap_or_default();
    if let Some(err) = recorded.get("error").and_then(Json::as_str) {
        return match ours {
            Outcome::Error(_) => None,
            Outcome::Rows(t) => Some((
                "desktop-error",
                format!(
                    "Desktop fails ({}) but DashCraft returns {} row(s){}",
                    clip(err, 160),
                    t.rows.len(),
                    t.rows
                        .first()
                        .map(|r| format!(": {}", show_row(r)))
                        .unwrap_or_default()
                ),
            )),
        };
    }
    let ours = match ours {
        Outcome::Error(e) => return Some(("dashcraft-error", clip(e, 300))),
        Outcome::Rows(t) => t,
    };
    let want_cols: Vec<String> = recorded["columns"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|c| c.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    // Map Desktop's columns onto ours by name; fall back to position.
    let lower = |s: &str| s.to_lowercase();
    let by_name: Option<Vec<usize>> = want_cols
        .iter()
        .map(|w| ours.columns.iter().position(|c| lower(c) == lower(w)))
        .collect();
    let mapping = match by_name {
        Some(m) if ours.columns.len() == want_cols.len() => m,
        _ => {
            return Some((
                "columns",
                format!("Desktop {want_cols:?}, DashCraft {:?}", ours.columns),
            ));
        }
    };
    let recorded_count = recorded
        .get("rowCount")
        .and_then(Json::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .or_else(|| {
            case.get("volatile")
                .and(recorded["rows"].as_array().map(Vec::len))
        });
    if let Some(n) = recorded_count {
        return (n != ours.rows.len()).then(|| {
            (
                "row-count",
                format!("Desktop {n} row(s), DashCraft {}", ours.rows.len()),
            )
        });
    }
    let want_rows: Vec<Vec<Cell>> = match recorded["rows"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|r| {
                    r.as_array()
                        .ok_or_else(|| "row is not an array".to_owned())?
                        .iter()
                        .map(Cell::from_json)
                        .collect::<Result<Vec<_>, _>>()
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .unwrap_or_else(|| Err("no rows recorded".into()))
    {
        Ok(r) => r,
        Err(e) => panic!("{}: bad recording: {e}", case["id"]),
    };
    let mut got_rows: Vec<Vec<Cell>> = ours
        .rows
        .iter()
        .map(|r| mapping.iter().map(|&i| r[i].clone()).collect())
        .collect();
    if want_rows.len() != got_rows.len() {
        return Some((
            "row-count",
            format!(
                "Desktop {} row(s), DashCraft {}",
                want_rows.len(),
                got_rows.len()
            ),
        ));
    }
    let ordered = case.get("ordered").and_then(Json::as_bool).unwrap_or(false)
        || query.to_uppercase().contains("ORDER BY");
    let mut want_rows = want_rows;
    if !ordered {
        let key = |r: &Vec<Cell>| r.iter().map(Cell::sort_key).collect::<Vec<_>>();
        want_rows.sort_by_cached_key(key);
        got_rows.sort_by_cached_key(key);
    }
    let mut diffs = Vec::new();
    let mut count = 0;
    for (want, got) in want_rows.iter().zip(&got_rows) {
        for (c, (w, g)) in want.iter().zip(got).enumerate() {
            if !w.matches(g) {
                count += 1;
                if diffs.len() < 3 {
                    diffs.push(format!(
                        "{} = {} (Desktop) vs {} (DashCraft) in row {}",
                        want_cols[c],
                        w.show(),
                        g.show(),
                        show_row(want)
                    ));
                }
            }
        }
    }
    (count > 0).then(|| {
        (
            "values",
            format!("{count} value(s) differ: {}", diffs.join("; ")),
        )
    })
}

fn show_row(row: &[Cell]) -> String {
    let cells: Vec<String> = row.iter().map(Cell::show).collect();
    clip(&format!("[{}]", cells.join(", ")), 200)
}

fn clip(s: &str, max: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() <= max {
        s
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

// ---------------------------------------------------------------------------
// Metadata (the INFO.VIEW functions) from DashCraft's model
// ---------------------------------------------------------------------------

fn data_type_name(t: &DataType) -> String {
    match t {
        DataType::String => "Text".into(),
        DataType::Int64 => "Integer".into(),
        DataType::Double => "Number".into(),
        DataType::Decimal => "Currency".into(),
        DataType::DateTime => "Date".into(),
        DataType::Boolean => "True/False".into(),
        DataType::Binary => "Binary".into(),
        DataType::Variant => "Variant".into(),
        other => other.as_str().to_owned(),
    }
}

fn summarize_by_name(s: Option<&SummarizeBy>) -> String {
    match s {
        None | Some(SummarizeBy::Default) => "Default".into(),
        Some(SummarizeBy::None) => "None".into(),
        Some(SummarizeBy::Sum) => "Sum".into(),
        Some(SummarizeBy::Min) => "Min".into(),
        Some(SummarizeBy::Max) => "Max".into(),
        Some(SummarizeBy::Count) => "Count".into(),
        Some(SummarizeBy::Average) => "Average".into(),
        Some(SummarizeBy::DistinctCount) => "DistinctCount".into(),
        Some(other) => other.as_str().to_owned(),
    }
}

/// Desktop's query results intern text case-insensitively per column: a
/// case-variant shows the first spelling met (`INFO.VIEW.COLUMNS` lists
/// Territories' "State Or Province" as Accounts' "State or Province"). The
/// adapter emits rows in model order, which is Desktop's object-id order.
fn unify_text_case(mut table: Table) -> Table {
    let mut first: Vec<HashMap<String, String>> = vec![HashMap::new(); table.columns.len()];
    for row in &mut table.rows {
        for (cell, seen) in row.iter_mut().zip(first.iter_mut()) {
            if let Cell::Text(s) = cell {
                let canonical = seen.entry(s.to_lowercase()).or_insert_with(|| s.clone());
                if canonical != s {
                    *s = canonical.clone();
                }
            }
        }
    }
    table
}

fn metadata(model: &Model, kind: &str) -> Table {
    unify_text_case(metadata_rows(model, kind))
}

fn metadata_rows(model: &Model, kind: &str) -> Table {
    let cols = |names: &[&str]| names.iter().map(|n| format!("[{n}]")).collect::<Vec<_>>();
    let b = Cell::Bool;
    fn t(s: impl Into<String>) -> Cell {
        Cell::text(s)
    }
    let o = Cell::opt_text;
    match kind {
        "tables" => Table {
            columns: cols(&[
                "Name",
                "DataCategory",
                "Description",
                "IsHidden",
                "CalculationGroupPrecedence",
            ]),
            rows: model
                .tables
                .iter()
                .map(|tb| {
                    vec![
                        t(&tb.name),
                        t(tb.data_category.as_deref().unwrap_or("Regular")),
                        o(tb.description.as_deref()),
                        b(tb.is_hidden),
                        tb.calculation_group
                            .as_ref()
                            .map_or(Cell::Blank, |g| Cell::num(g.precedence as f64)),
                    ]
                })
                .collect(),
        },
        "columns" => Table {
            columns: cols(&[
                "Table",
                "Name",
                "DataType",
                "IsHidden",
                "FormatString",
                "SortByColumn",
                "SummarizeBy",
                "DataCategory",
                "DisplayFolder",
                "Description",
                "Expression",
            ]),
            rows: model
                .tables
                .iter()
                .flat_map(|tb| {
                    tb.columns
                        .iter()
                        .filter(|c| c.kind != ColumnKind::RowNumber)
                        .map(|c| {
                            vec![
                                t(&tb.name),
                                t(&c.name),
                                t(data_type_name(&c.data_type)),
                                b(c.is_hidden),
                                o(c.format_string.as_deref()),
                                o(c.sort_by_column.as_deref()),
                                t(summarize_by_name(c.summarize_by.as_ref())),
                                t(c.data_category.as_deref().unwrap_or("Regular")),
                                o(c.display_folder.as_deref()),
                                o(c.description.as_deref()),
                                o(c.expression()),
                            ]
                        })
                })
                .collect(),
        },
        "measures" => Table {
            columns: cols(&[
                "Table",
                "Name",
                "IsHidden",
                "DisplayFolder",
                "Description",
                "Expression",
            ]),
            rows: model
                .tables
                .iter()
                .flat_map(|tb| {
                    tb.measures.iter().map(|m| {
                        vec![
                            t(&tb.name),
                            t(&m.name),
                            b(m.is_hidden),
                            o(m.display_folder.as_deref()),
                            o(m.description.as_deref()),
                            t(&m.expression),
                        ]
                    })
                })
                .collect(),
        },
        "measure-formats" => Table {
            columns: cols(&["Name", "FormatString"]),
            rows: model
                .tables
                .iter()
                .flat_map(|tb| {
                    tb.measures
                        .iter()
                        .map(|m| vec![t(&m.name), o(m.format_string.as_deref())])
                })
                .collect(),
        },
        "relationships" => {
            let card = |c: &Cardinality| match c {
                Cardinality::One => "One".to_owned(),
                Cardinality::Many => "Many".to_owned(),
                other => other.as_str().to_owned(),
            };
            Table {
                columns: cols(&[
                    "FromTable",
                    "FromColumn",
                    "FromCardinality",
                    "ToTable",
                    "ToColumn",
                    "ToCardinality",
                    "IsActive",
                    "CrossFilteringBehavior",
                    "SecurityFilteringBehavior",
                ]),
                rows: model
                    .relationships
                    .iter()
                    .map(|r| {
                        vec![
                            t(&r.from.table),
                            t(&r.from.column),
                            t(card(&r.from_cardinality)),
                            t(&r.to.table),
                            t(&r.to.column),
                            t(card(&r.to_cardinality)),
                            b(r.is_active),
                            t(match &r.cross_filtering_behavior {
                                CrossFilter::OneDirection => "OneDirection".to_owned(),
                                CrossFilter::BothDirections => "BothDirections".to_owned(),
                                CrossFilter::Automatic => "Automatic".to_owned(),
                                other => other.as_str().to_owned(),
                            }),
                            t(match &r.security_filtering_behavior {
                                SecurityFilteringBehavior::OneDirection => "Single".to_owned(),
                                SecurityFilteringBehavior::BothDirections => "Both".to_owned(),
                                SecurityFilteringBehavior::None => "None".to_owned(),
                                other => other.as_str().to_owned(),
                            }),
                        ]
                    })
                    // INFO.VIEW.RELATIONSHIPS lists a one-to-one relationship
                    // from both sides (Desktop ground truth).
                    .flat_map(|row| {
                        let one_to_one =
                            row.get(2) == Some(&t("One")) && row.get(5) == Some(&t("One"));
                        let mut rows = vec![row.clone()];
                        if one_to_one {
                            let mut rev = row;
                            rev.swap(0, 3);
                            rev.swap(1, 4);
                            rows.push(rev);
                        }
                        rows
                    })
                    .collect(),
            }
        }
        other => panic!("unknown metadata kind {other}"),
    }
}

// ---------------------------------------------------------------------------
// Fixtures, ledger, runner
// ---------------------------------------------------------------------------

async fn open_fixture(fixture: &str) -> Option<Session> {
    let root = reference_root();
    let (pbip, blob_map) = match fixture {
        "acme" => {
            let dir = root.join("examples/acme-outfitters");
            let expected_path = dir.join("expected.json");
            if !expected_path.exists() {
                println!("SKIP: {} missing", expected_path.display());
                return None;
            }
            let expected = read_json(&expected_path);
            (
                dir.join(expected["pbip"].as_str().unwrap()),
                Some(dir.join(expected["blobMap"].as_str().unwrap())),
            )
        }
        "dax-lab" => (root.join("groundtruth/fixtures/dax-lab/DaxLab.pbip"), None),
        other => match other.strip_prefix("sample:") {
            Some(file) => (root.join("samples/microsoft").join(file), None),
            None => panic!("unknown fixture {other}"),
        },
    };
    if !pbip.exists() {
        println!("SKIP: {} missing", pbip.display());
        return None;
    }
    let session = Session::open(
        &pbip,
        SessionOptions {
            blob_map,
            clock: Some(fixed_clock()),
            ..SessionOptions::default()
        },
    )
    .unwrap();
    let report = session.refresh(RefreshOptions::all()).await.unwrap();
    assert!(report.is_ok(), "{fixture} refresh: {report:#?}");
    Some(session)
}

#[derive(Debug, Clone)]
struct Deviation {
    case: String,
    kind: String,
    reason: String,
}

impl Deviation {
    fn covers(&self, id: &str) -> bool {
        match self.case.strip_suffix('*') {
            Some(prefix) => id.starts_with(prefix),
            None => self.case == id,
        }
    }
}

const DEVIATION_KINDS: [&str; 4] = ["bug", "unsupported", "byDesign", "lenient"];

fn ledger(suite: &str) -> Vec<Deviation> {
    let path = reference_root().join("groundtruth/deviations.json");
    if !path.exists() {
        return Vec::new();
    }
    let doc = read_json(&path);
    doc["deviations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["suite"] == suite)
        .map(|d| {
            let dev = Deviation {
                case: d["case"].as_str().unwrap_or_default().to_owned(),
                kind: d["kind"].as_str().unwrap_or_default().to_owned(),
                reason: d["reason"].as_str().unwrap_or_default().to_owned(),
            };
            assert!(
                DEVIATION_KINDS.contains(&dev.kind.as_str()),
                "deviations.json: {suite}/{}: kind must be one of {DEVIATION_KINDS:?}",
                dev.case
            );
            assert!(
                !dev.reason.trim().is_empty(),
                "deviations.json: {suite}/{}: a reason is required",
                dev.case
            );
            dev
        })
        .collect()
}

fn view_key(case: &Json) -> Option<(Vec<String>, Option<String>)> {
    let v = case.get("viewAs")?;
    let roles = v["roles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r.as_str().map(str::to_owned))
        .collect();
    Some((roles, v["userName"].as_str().map(str::to_owned)))
}

async fn run_case(session: &Session, case: &Json) -> Outcome {
    if let Some(kind) = case.get("metadata").and_then(Json::as_str) {
        return Outcome::Rows(metadata(&session.model(), kind));
    }
    let query = case["query"].as_str().unwrap();
    match session
        .dax_query_tables(query, &FilterSpec::default(), CancellationToken::new())
        .await
    {
        Err(e) => Outcome::Error(e.to_string()),
        Ok(tables) if tables.len() != 1 => {
            Outcome::Error(format!("{} result tables, expected 1", tables.len()))
        }
        Ok(tables) => {
            let t = &tables[0];
            Outcome::Rows(Table {
                columns: t.columns().iter().map(|c| c.qualified_name()).collect(),
                rows: t
                    .rows()
                    .iter()
                    .map(|r| r.iter().map(Cell::from_value).collect())
                    .collect(),
            })
        }
    }
}

async fn run_suite(suite: &str) {
    let root = reference_root().join("groundtruth");
    let corpus_path = root.join(format!("corpus/{suite}.json"));
    let recording_path = root.join(format!("recordings/{suite}.json"));
    for p in [&corpus_path, &recording_path] {
        if !p.exists() {
            println!("SKIP: {} missing", p.display());
            return;
        }
    }
    let corpus = read_json(&corpus_path);
    let recording = read_json(&recording_path);
    let recorded: BTreeMap<&str, &Json> = recording["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["id"].as_str().unwrap(), c))
        .collect();
    let Some(session) = open_fixture(corpus["fixture"].as_str().unwrap()).await else {
        return;
    };
    let deviations = ledger(suite);

    let mut current_view: Option<(Vec<String>, Option<String>)> = None;
    let mut not_recorded = Vec::new();
    let mut stale_query = Vec::new();
    let mut passed = 0usize;
    let mut mismatches: Vec<(String, &'static str, String)> = Vec::new();
    let cases = corpus["cases"].as_array().unwrap();
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let Some(rec) = recorded.get(id) else {
            not_recorded.push(id.to_owned());
            continue;
        };
        if rec["query"] != case["query"] {
            stale_query.push(id.to_owned());
            continue;
        }
        let view = view_key(case);
        if view != current_view {
            match &view {
                Some((roles, user)) => session
                    .set_view_as(ViewAs::roles(roles, user.clone()), CancellationToken::new())
                    .await
                    .unwrap_or_else(|e| panic!("{id}: view as {roles:?}: {e}")),
                None => session.clear_view_as(),
            }
            current_view = view;
        }
        let ours = run_case(&session, case).await;
        match compare(case, rec, &ours) {
            None => passed += 1,
            Some((kind, detail)) => mismatches.push((id.to_owned(), kind, detail)),
        }
    }

    let new: Vec<_> = mismatches
        .iter()
        .filter(|(id, ..)| !deviations.iter().any(|d| d.covers(id)))
        .collect();
    let stale: Vec<_> = deviations
        .iter()
        .filter(|d| !mismatches.iter().any(|(id, ..)| d.covers(id)))
        .collect();

    // Report.
    let mut md = String::new();
    let _ = writeln!(md, "# Ground truth: {suite}\n");
    let _ = writeln!(
        md,
        "Desktop {} (engine {}), recorded {}.\n",
        recording["desktop"]["version"].as_str().unwrap_or("?"),
        recording["desktop"]["engine"].as_str().unwrap_or("?"),
        recording["recordedOn"].as_str().unwrap_or("?")
    );
    let _ = writeln!(
        md,
        "| cases | match | listed deviations | new mismatches | stale ledger entries | not recorded | query changed |\n|---|---|---|---|---|---|---|\n| {} | {passed} | {} | {} | {} | {} | {} |\n",
        cases.len(),
        mismatches.len() - new.len(),
        new.len(),
        stale.len(),
        not_recorded.len(),
        stale_query.len()
    );
    if !mismatches.is_empty() {
        let _ = writeln!(md, "| case | kind | ledger | detail |\n|---|---|---|---|");
        for (id, kind, detail) in &mismatches {
            let listed = deviations
                .iter()
                .find(|d| d.covers(id))
                .map_or("**new**".to_owned(), |d| d.kind.clone());
            let _ = writeln!(
                md,
                "| `{id}` | {kind} | {listed} | {} |",
                detail.replace('|', "\\|")
            );
        }
    }
    let dir = report_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(format!("{suite}.md")), &md);
    let proposed_path = dir.join(format!("{suite}.proposed.json"));
    if new.is_empty() {
        let _ = std::fs::remove_file(&proposed_path);
    } else {
        let proposed: Vec<Json> = new
            .iter()
            .map(|(id, kind, detail)| {
                serde_json::json!({
                    "suite": suite,
                    "case": id,
                    "kind": "bug",
                    "reason": format!("{kind}: {}", clip(detail, 240)),
                })
            })
            .collect();
        let _ = std::fs::write(
            &proposed_path,
            serde_json::to_string_pretty(&proposed).unwrap_or_default(),
        );
    }
    println!(
        "{suite}: {} cases, {passed} match, {} listed, {} new, {} stale; report {}",
        cases.len(),
        mismatches.len() - new.len(),
        new.len(),
        stale.len(),
        dir.join(format!("{suite}.md")).display()
    );

    let mut failures = Vec::new();
    for (id, kind, detail) in new.iter().take(40) {
        failures.push(format!("  NEW {id} [{kind}]: {}", clip(detail, 400)));
    }
    if new.len() > 40 {
        failures.push(format!("  … and {} more new mismatches", new.len() - 40));
    }
    for d in &stale {
        failures.push(format!(
            "  STALE ledger entry {} ({}): it matches Desktop now; remove it",
            d.case, d.kind
        ));
    }
    for id in &not_recorded {
        failures.push(format!(
            "  NOT RECORDED {id}: record the suite again (scripts/groundtruth/Record-GroundTruth.ps1)"
        ));
    }
    for id in &stale_query {
        failures.push(format!(
            "  QUERY CHANGED {id}: the corpus query differs from the recorded one; record again"
        ));
    }
    assert!(
        failures.is_empty(),
        "ground truth {suite}: {} problem(s) (new mismatches go in reference/groundtruth/deviations.json with a reason, or get fixed; proposals in {}):\n{}",
        failures.len(),
        proposed_path.display(),
        failures.join("\n")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn acme_metadata() {
    run_suite("acme-metadata").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn acme_tables() {
    run_suite("acme-tables").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn acme_measures() {
    run_suite("acme-measures").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn acme_visuals() {
    run_suite("acme-visuals").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn acme_calcgroup_rls() {
    run_suite("acme-calcgroup-rls").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn format() {
    run_suite("format").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dax_lab_tables() {
    run_suite("dax-lab-tables").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dax_functions() {
    run_suite("dax-functions").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn samples_revenue_opportunities() {
    run_suite("samples-revenue-opportunities").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn samples_corporate_spend() {
    run_suite("samples-corporate-spend").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn samples_regional_sales() {
    run_suite("samples-regional-sales").await;
}

#[test]
fn comparison_rules() {
    let d = |s: &str| Cell::from_json(&serde_json::json!({ "decimal": s })).unwrap();
    assert!(Cell::num(894_300.500_000_003_3).matches(&Cell::num(894_300.5)));
    assert!(!Cell::num(894_300.51).matches(&Cell::num(894_300.5)));
    assert!(d("1.5").matches(&Cell::from_value(&Value::Decimal(15_000))));
    assert!(!d("1.5").matches(&Cell::from_value(&Value::Decimal(15_001))));
    assert!(d("2").matches(&Cell::from_value(&Value::Integer(2))));
    assert!(!Cell::Blank.matches(&Cell::num(0.0)));
    assert!(!Cell::Blank.matches(&Cell::text("")));
    assert!(
        Cell::from_json(&serde_json::json!({ "double": "Infinity" }))
            .unwrap()
            .matches(&Cell::from_value(&Value::Double(f64::INFINITY)))
    );
    let dt =
        Cell::from_json(&serde_json::json!({ "dateTime": "2024-02-29T13:45:00.000" })).unwrap();
    let ours = chrono::NaiveDate::from_ymd_opt(2024, 2, 29)
        .unwrap()
        .and_hms_opt(13, 45, 0)
        .unwrap()
        .and_utc()
        .timestamp_micros();
    assert!(dt.matches(&Cell::from_value(&Value::DateTime(ours))));
    assert_eq!(parse_decimal("-0.25"), Some(-2_500));

    let case = serde_json::json!({ "id": "x", "query": "EVALUATE T" });
    let rec = serde_json::json!({ "columns": ["T[a]"], "rows": [[1], [2]] });
    let rows = |a: f64, b: f64| {
        Outcome::Rows(Table {
            columns: vec!["T[A]".into()],
            rows: vec![vec![Cell::num(a)], vec![Cell::num(b)]],
        })
    };
    assert!(compare(&case, &rec, &rows(2.0, 1.0)).is_none(), "multiset");
    let ordered = serde_json::json!({ "id": "x", "query": "EVALUATE T ORDER BY T[a]" });
    assert_eq!(
        compare(&ordered, &rec, &rows(2.0, 1.0)).unwrap().0,
        "values"
    );
    let err = serde_json::json!({ "error": "boom" });
    assert!(compare(&case, &err, &Outcome::Error("x".into())).is_none());
    assert_eq!(
        compare(&case, &err, &rows(1.0, 2.0)).unwrap().0,
        "desktop-error"
    );
}
