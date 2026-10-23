#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! End-to-end acceptance on the synthetic Acme Outfitters example
//! (`reference/examples/acme-outfitters`, committed): the PBIP refreshed
//! offline (M over the blob-mapped CSVs → calculated columns → calculated
//! tables → calculation group) gives every value of its `expected.json`,
//! which the generator computed independently in plain Python: row counts,
//! every measure unfiltered and under each report's filters, every card and
//! visual total/row under each page's filters and saved slicer selections,
//! the calculation group, the unrelated-table report filter, row-level
//! security for both roles and the calculated tables.
//!
//! The Microsoft sample PBIX files (`reference/samples/microsoft`, MIT) load
//! their embedded data and evaluate their measures as their `expected.json`
//! says, and a PBIP saved from one of them loads its local data cache.
//! Optional files (the downloaded samples) print `SKIP: <file> missing`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use arrow::array::{Array, AsArray, RecordBatch};
use arrow::datatypes::Int64Type;
use chrono::TimeZone;
use pbi_model::Report;
use pbi_session::{
    CancellationToken, ColumnFilterSpec, DerivedColumn, FilterSpec, FixedClock, MeasureValue,
    RefreshEvent, RefreshKind, RefreshOptions, RefreshSource, Session, SessionOptions, TableStatus,
    Value, ViewAs, filters_from_page, filters_from_report,
};
use serde_json::Value as Json;

fn reference_root() -> PathBuf {
    std::env::var_os("PBIVIEW_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
}

/// The fixture path, or `None` (after printing `SKIP`) when missing.
fn fixture(rel: &str) -> Option<PathBuf> {
    let path = reference_root().join(rel);
    if path.exists() {
        Some(path)
    } else {
        println!("SKIP: {} missing", path.display());
        None
    }
}

fn fixed_clock() -> Arc<FixedClock> {
    Arc::new(FixedClock::utc(
        chrono::Utc
            .with_ymd_and_hms(2026, 10, 21, 14, 5, 0)
            .single()
            .unwrap(),
    ))
}

fn read_json(path: &Path) -> Json {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

// ---------------------------------------------------------------------------
// Acme Outfitters
// ---------------------------------------------------------------------------

const EXAMPLE: &str = "examples/acme-outfitters";

struct Example {
    pbip: PathBuf,
    blob_map: PathBuf,
    expected: Json,
}

fn example() -> Option<Example> {
    let dir = fixture(EXAMPLE)?;
    let expected = read_json(&dir.join("expected.json"));
    Some(Example {
        pbip: dir.join(expected["pbip"].as_str().unwrap()),
        blob_map: dir.join(expected["blobMap"].as_str().unwrap()),
        expected,
    })
}

impl Example {
    fn open(&self) -> Session {
        Session::open(
            &self.pbip,
            SessionOptions {
                blob_map: Some(self.blob_map.clone()),
                clock: Some(fixed_clock()),
                ..SessionOptions::default()
            },
        )
        .unwrap()
    }

    async fn refreshed(&self) -> Session {
        let session = self.open();
        let report = session.refresh(RefreshOptions::all()).await.unwrap();
        assert!(report.is_ok(), "{report:#?}");
        session
    }
}

/// Collects expected-vs-actual comparisons, prints them as a table and
/// fails with every mismatch at once.
#[derive(Default)]
struct Checks {
    rows: Vec<(String, String, String, bool)>,
}

impl Checks {
    fn check(&mut self, label: impl Into<String>, want: impl Into<String>, got: impl Into<String>) {
        let (want, got) = (want.into(), got.into());
        let ok = want == got;
        self.rows.push((label.into(), want, got, ok));
    }

    /// A measure value against an expected `{value, formatted}`: BLANK
    /// (`value: null`) compares the value, anything else the formatted text.
    fn measure(&mut self, label: impl Into<String>, want: &Json, got: &MeasureValue) {
        let label = label.into();
        if want["value"].is_null() {
            let got = match &got.value {
                Some(Value::Blank) => "(Blank)".to_owned(),
                _ => got.display(),
            };
            self.check(label, "(Blank)", got);
        } else {
            let want = want["formatted"]
                .as_str()
                .unwrap_or_else(|| panic!("{label}: {want}"));
            self.check(label, want, got.display());
        }
    }

    fn finish(self, title: &str) {
        println!("{title}: {} checks", self.rows.len());
        for (label, want, got, ok) in &self.rows {
            let mark = if *ok { "ok  " } else { "FAIL" };
            println!("  {mark} {label:<95} expected {want:>14}  actual {got:>14}");
        }
        let failed: Vec<_> = self.rows.iter().filter(|r| !r.3).collect();
        assert!(
            failed.is_empty(),
            "{title}: {} mismatches: {failed:#?}",
            failed.len()
        );
    }
}

/// `'Table'[Column] in ['a', 2]` / `not in`, as `expected.json` writes filters.
fn describe(f: &ColumnFilterSpec) -> String {
    let values: Vec<String> = f
        .values
        .iter()
        .map(|v| match v {
            Json::String(s) => format!("'{s}'"),
            other => other.to_string(),
        })
        .collect();
    format!(
        "'{}'[{}] {} [{}]",
        f.table,
        f.column,
        if f.exclude { "not in" } else { "in" },
        values.join(", ")
    )
}

/// `"'T'[C] …"` → (`T`, `C`, the rest).
fn split_field(text: &str) -> (String, String, &str) {
    let rest = text.strip_prefix('\'').unwrap();
    let (table, rest) = rest.split_once("'[").unwrap();
    let (column, rest) = rest.split_once("] ").unwrap();
    (table.to_owned(), column.to_owned(), rest)
}

/// `['a', 'b']` / `[2025]` → JSON values.
fn parse_values(list: &str) -> Vec<Json> {
    let inner = list
        .trim()
        .strip_prefix('[')
        .and_then(|l| l.strip_suffix(']'))
        .unwrap();
    let mut out = Vec::new();
    let mut chars = inner.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| *c == ' ' || *c == ',') {
            chars.next();
        }
        let Some(&first) = chars.peek() else { break };
        if first == '\'' {
            chars.next();
            out.push(Json::String(
                chars.by_ref().take_while(|c| *c != '\'').collect(),
            ));
        } else {
            let token: String = chars.by_ref().take_while(|c| *c != ',').collect();
            let token = token.trim();
            out.push(
                token
                    .parse::<i64>()
                    .map(Json::from)
                    .unwrap_or_else(|_| Json::from(token.parse::<f64>().unwrap())),
            );
        }
    }
    out
}

/// A saved slicer selection of `expected.json` as column filters. Ranges
/// become the `IN` list of the column's values in the range, found with
/// SQL over the loaded data; a relative date slicer keeps every row (the
/// data ends on 2025-12-31, `expected.json` notes).
async fn selection_filters(session: &Session, selection: &str) -> Vec<ColumnFilterSpec> {
    let (table, column, rest) = split_field(selection);
    if let Some(list) = rest.strip_prefix("not in ") {
        return vec![ColumnFilterSpec::exclude(table, column, parse_values(list))];
    }
    if let Some(list) = rest.strip_prefix("in [") {
        return vec![ColumnFilterSpec::include(
            table,
            column,
            parse_values(&format!("[{list}")),
        )];
    }
    if rest.starts_with("in the last ") && rest.ends_with("(relative date)") {
        return Vec::new();
    }
    // `>= a and <= b` / `>= a and < b`.
    let (lower, upper) = rest.split_once(" and ").unwrap();
    let bound = |b: &str| {
        let (op, literal) = b.trim().split_once(' ').unwrap();
        let literal = if literal.parse::<f64>().is_ok() {
            literal.to_owned()
        } else {
            format!("'{literal}'")
        };
        format!("\"{column}\" {op} {literal}")
    };
    let sql = format!(
        "SELECT DISTINCT \"{column}\" FROM \"{table}\" WHERE {} AND {}",
        bound(lower),
        bound(upper)
    );
    let batches = session.sql(&sql, CancellationToken::new()).await.unwrap();
    let mut values = Vec::new();
    for batch in &batches {
        let array = batch.column(0);
        for row in 0..array.len() {
            values.push(Json::String(
                arrow::util::display::array_value_to_string(array, row).unwrap(),
            ));
        }
    }
    assert!(!values.is_empty(), "{selection}: no values in range");
    vec![ColumnFilterSpec::include(table, column, values)]
}

/// The report of `expected.json` entry `report`.
fn find_report<'a>(session: &'a Session, name: &str) -> &'a Report {
    session
        .document()
        .reports
        .iter()
        .find(|r| r.name == name || r.name.contains(name))
        .unwrap_or_else(|| panic!("no report {name}"))
}

/// Table of a field used as a row/category key in `expected.json` visuals.
fn key_table(field: &str) -> &'static str {
    match field {
        "Category" | "Product Name" => "Products",
        "Store Name" | "Store Manager" | "City" => "Stores",
        "Reason" => "Returns",
        "Loyalty Tier" => "Customers",
        "Device" => "Web Sessions",
        "Year" => "Date",
        other => panic!("no table for row field {other}"),
    }
}

fn with(mut spec: FilterSpec, filter: ColumnFilterSpec) -> FilterSpec {
    spec.filters.push(filter);
    spec
}

async fn measure(session: &Session, name: &str, filter: &FilterSpec) -> MeasureValue {
    session
        .evaluate_measure(name, filter, CancellationToken::new())
        .await
        .unwrap_or_else(|e| panic!("[{name}]: {e}"))
}

/// Measure name → value in `filter`.
async fn measures(session: &Session, filter: &FilterSpec) -> BTreeMap<String, MeasureValue> {
    session
        .measures(filter, CancellationToken::new())
        .await
        .unwrap()
        .into_iter()
        .map(|m| (m.name.clone(), m))
        .collect()
}

fn int_value(batches: &[RecordBatch]) -> i64 {
    batches[0].column(0).as_primitive::<Int64Type>().value(0)
}

#[tokio::test(flavor = "multi_thread")]
async fn pbip_refresh_matches_expected_rows_and_measures() {
    let Some(example) = example() else {
        return;
    };
    let exp = &example.expected;
    let session = example.open();

    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let report = session
        .refresh(RefreshOptions::all().with_progress(move |e| {
            if let Ok(mut v) = sink.lock() {
                v.push(e);
            }
        }))
        .await
        .unwrap();

    println!("refresh: {} ms total", report.duration_ms);
    for t in &report.tables {
        println!(
            "  {:<22} {:?} {:>5} rows {:>5} ms {:?}",
            t.name, t.kind, t.rows, t.duration_ms, t.status
        );
        for c in &t.columns {
            println!(
                "    column {:<24} {:>5} ms {:?}",
                c.name, c.duration_ms, c.status
            );
        }
    }
    assert!(report.is_ok(), "{report:#?}");

    let mut checks = Checks::default();
    for (table, rows) in exp["rowCounts"].as_object().unwrap() {
        checks.check(
            format!("rows {table}"),
            rows.to_string(),
            report
                .table(table)
                .map_or("<missing>".to_owned(), |t| t.rows.to_string()),
        );
    }
    checks.finish("row counts");
    let kind = |name: &str| report.table(name).map(|t| t.kind);
    assert_eq!(kind("Orders"), Some(RefreshKind::M));
    assert_eq!(kind("Last Refreshed"), Some(RefreshKind::M));
    assert_eq!(kind("Date"), Some(RefreshKind::Calculated));
    assert_eq!(kind("Products Never Sold"), Some(RefreshKind::Calculated));
    let orders = report.table("Orders").unwrap();
    for column in ["Line Cost", "Order Size", "Is Returned", "Days To Ship"] {
        assert!(
            orders.column(column).is_some_and(|c| c.status.is_ok()),
            "{orders:?}"
        );
    }
    let customers = report.table("Customers").unwrap();
    assert!(
        customers
            .column("Lifetime Sales")
            .is_some_and(|c| c.status.is_ok())
    );

    // The stored table has every model column, in model order.
    let batch = session.table_batch("Orders").unwrap();
    let model = session.model();
    let model_columns: Vec<&str> = model
        .table("Orders")
        .unwrap()
        .columns
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    let stored: Vec<String> = batch
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect();
    assert_eq!(stored, model_columns);

    // SQL over the refreshed data: the Overview report filter drops the
    // cancelled orders.
    let all = exp["measures"]["unfiltered"]["Order Count"]["value"]
        .as_i64()
        .unwrap();
    let not_cancelled =
        exp["measures"]["byReport"]["Acme Outfitters - Overview"]["values"]["Order Count"]["value"]
            .as_i64()
            .unwrap();
    let cancelled = session
        .sql(
            "SELECT COUNT(*) AS n FROM \"Orders\" WHERE \"Status\" = 'Cancelled'",
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(int_value(&cancelled), all - not_cancelled);
    // Orders with an unknown customer relate to the blank row.
    let unmatched = session
        .sql(
            "SELECT COUNT(*) FROM \"Orders\" o LEFT JOIN \"Customers\" c \
             ON o.\"Customer ID\" = c.\"Customer ID\" WHERE c.\"Customer ID\" IS NULL",
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        int_value(&unmatched),
        exp["dataQuality"]["ordersWithUnmatchedCustomer"]
            .as_i64()
            .unwrap()
    );
    // The M step stamps the injected clock.
    let stamp = session
        .dax_query(
            "EVALUATE ROW(\"t\", FORMAT(MAX('Last Refreshed'[Last Refreshed (UTC)]), \"yyyy-mm-dd hh:nn\"))",
            &FilterSpec::new(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        stamp[0].column(0).as_string::<i32>().value(0),
        "2026-10-21 14:05"
    );

    {
        let events = events.lock().unwrap();
        assert!(matches!(events.first(), Some(RefreshEvent::Started { .. })));
        assert!(matches!(
            events.last(),
            Some(RefreshEvent::Finished { failed: 0, .. })
        ));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, RefreshEvent::ColumnFinished { column, .. } if column.name == "Lifetime Sales"))
        );
    }

    // Calculated tables.
    let never_sold = session
        .sql(
            "SELECT \"Product ID\" FROM \"Products Never Sold\" ORDER BY \"Product ID\"",
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let ids: Vec<String> = never_sold
        .iter()
        .flat_map(|b| {
            let a = b.column(0).as_string::<i32>();
            (0..a.len())
                .map(|i| a.value(i).to_owned())
                .collect::<Vec<_>>()
        })
        .collect();
    let want: Vec<String> = exp["calculatedTables"]["Products Never Sold"]["productIds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids, want);
    let range = session
        .sql(
            "SELECT MIN(\"Date\"), MAX(\"Date\") FROM \"Date\"",
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let date = &exp["calculatedTables"]["Date"];
    for (i, key) in ["from", "to"].into_iter().enumerate() {
        let text = arrow::util::display::array_value_to_string(range[0].column(i), 0).unwrap();
        assert!(text.starts_with(date[key].as_str().unwrap()), "{text}");
    }

    // Every measure unfiltered and under each report's report-level filters.
    let mut checks = Checks::default();
    let unfiltered = measures(&session, &FilterSpec::new()).await;
    for (name, want) in exp["measures"]["unfiltered"].as_object().unwrap() {
        checks.measure(format!("unfiltered [{name}]"), want, &unfiltered[name]);
    }
    for (report_name, entry) in exp["measures"]["byReport"].as_object().unwrap() {
        let filter = filters_from_report(find_report(&session, report_name));
        assert!(filter.unsupported.is_empty(), "{filter:?}");
        let described: Vec<String> = filter.filters.iter().map(describe).collect();
        let want: Vec<&str> = entry["reportFilters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_str().unwrap())
            .collect();
        assert_eq!(described, want, "{report_name}");
        let got = measures(&session, &filter).await;
        for (name, want) in entry["values"].as_object().unwrap() {
            checks.measure(format!("{report_name} [{name}]"), want, &got[name]);
        }
    }
    checks.finish("measures");

    // Refreshing one table recomputes exactly what depends on it: the
    // column reading Returns, then what reads the whole Orders table
    // (RELATEDTABLE, SUMMARIZE); not Customers[Lifetime Sales], which reads
    // only Orders[Line Amount].
    let partial = session
        .refresh(RefreshOptions::tables(["returns"]))
        .await
        .unwrap();
    assert!(partial.is_ok(), "{partial:#?}");
    let steps: Vec<(&str, RefreshKind, Vec<&str>)> = partial
        .tables
        .iter()
        .map(|t| {
            (
                t.name.as_str(),
                t.kind,
                t.columns.iter().map(|c| c.name.as_str()).collect(),
            )
        })
        .collect();
    assert_eq!(
        steps,
        [
            ("Orders", RefreshKind::Calculated, vec!["Is Returned"]),
            ("Customers", RefreshKind::Calculated, vec!["Has Orders"]),
            (
                "Products",
                RefreshKind::Calculated,
                vec!["Units Sold (All Time)"]
            ),
            ("Returns", RefreshKind::M, vec![]),
            ("Products Never Sold", RefreshKind::Calculated, vec![]),
        ]
    );
    let after = measures(&session, &FilterSpec::new()).await;
    assert_eq!(
        after["Returns Count"].display(),
        exp["measures"]["unfiltered"]["Returns Count"]["formatted"]
    );
}

/// The filters of one visual of a page: report and page filters plus the
/// saved selections of the page's slicers that filter it.
async fn visual_filter(
    session: &Session,
    base: &FilterSpec,
    page: &Json,
    visual_key: &str,
) -> FilterSpec {
    let mut spec = base.clone();
    for slicer in page["slicers"].as_array().unwrap() {
        let skip = slicer["noFilterTargets"]
            .as_array()
            .is_some_and(|t| t.iter().any(|k| k == visual_key));
        if !skip {
            let selection = slicer["selection"].as_str().unwrap();
            spec.filters
                .extend(selection_filters(session, selection).await);
        }
    }
    spec
}

#[tokio::test(flavor = "multi_thread")]
async fn report_pages_match_expected_cards_and_visuals() {
    let Some(example) = example() else {
        return;
    };
    let exp = &example.expected;
    let session = example.refreshed().await;
    let group = &exp["calculationGroup"];
    let (group_table, group_column) = (
        group["table"].as_str().unwrap(),
        group["column"].as_str().unwrap(),
    );
    let item_filter =
        |item: &str| ColumnFilterSpec::include(group_table, group_column, [item.to_owned()]);
    let is_item = |key: &str| group["items"].get(key).is_some();

    let mut checks = Checks::default();
    for report_exp in exp["reports"].as_array().unwrap() {
        let report_name = report_exp["report"].as_str().unwrap();
        let report = find_report(&session, report_name);
        let report_filter = filters_from_report(report);
        assert!(report_filter.unsupported.is_empty(), "{report_filter:?}");
        for page_exp in report_exp["pages"].as_array().unwrap() {
            let page_name = page_exp["page"].as_str().unwrap();
            let page = report
                .pages
                .iter()
                .find(|p| p.name == page_exp["pageId"].as_str().unwrap())
                .unwrap_or_else(|| panic!("{report_name}: no page {page_name}"));
            assert_eq!(page.display_name, page_name);
            let page_filter = filters_from_page(page);
            assert!(page_filter.unsupported.is_empty(), "{page_filter:?}");
            let described: Vec<String> = page_filter.filters.iter().map(describe).collect();
            let want: Vec<&str> = page_exp["pageFilters"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f.as_str().unwrap())
                .collect();
            assert_eq!(described, want, "{report_name} / {page_name}");
            let mut base = report_filter.clone();
            base.extend(page_filter);
            let at = format!(
                "{} / {page_name}",
                report_name.trim_start_matches("Acme Outfitters - ")
            );

            for card in page_exp["cards"].as_array().unwrap() {
                let key = card["visualKey"].as_str().unwrap();
                let name = card["measure"].as_str().unwrap();
                let mut filter = visual_filter(&session, &base, page_exp, key).await;
                let mut label = format!("{at} / {key} [{name}]");
                if let Some(item) = card["calculationItem"].as_str() {
                    filter = with(filter, item_filter(item));
                    label = format!("{label} ({item})");
                }
                checks.measure(label, card, &measure(&session, name, &filter).await);
            }

            for visual in page_exp["visuals"].as_array().unwrap() {
                let key = visual["visualKey"].as_str().unwrap();
                let filter = visual_filter(&session, &base, page_exp, key).await;
                let label = format!("{at} / {key}");
                // A cell: a measure, or a calculation item of the group's measure.
                let cell = |name: &str, filter: &FilterSpec| {
                    if is_item(name) {
                        (
                            group["measure"].as_str().unwrap().to_owned(),
                            with(filter.clone(), item_filter(name)),
                        )
                    } else {
                        (name.to_owned(), filter.clone())
                    }
                };
                let rows: Vec<&Json> = ["rows", "columns", "categories"]
                    .iter()
                    .filter_map(|k| visual[*k].as_array())
                    .flatten()
                    .collect();
                // Top N: the total covers the rows shown.
                let mut total_filter = filter.clone();
                if key == "top10" {
                    let names: Vec<Json> = rows.iter().map(|r| r["Product Name"].clone()).collect();
                    total_filter = with(
                        total_filter,
                        ColumnFilterSpec::include("Products", "Product Name", names),
                    );
                }
                if let Some(total) = visual["total"].as_object() {
                    for (name, want) in total {
                        let (m, f) = cell(name, &total_filter);
                        checks.measure(
                            format!("{label} total [{name}]"),
                            want,
                            &measure(&session, &m, &f).await,
                        );
                    }
                }
                for row in &rows {
                    let mut row_filter = filter.clone();
                    let mut keys = Vec::new();
                    for (field, value) in row.as_object().unwrap() {
                        if !value.is_object() {
                            row_filter = with(
                                row_filter,
                                ColumnFilterSpec::include(key_table(field), field, [value.clone()]),
                            );
                            keys.push(value.to_string());
                        }
                    }
                    for (name, want) in row.as_object().unwrap() {
                        if want.is_object() {
                            let (m, f) = cell(name, &row_filter);
                            checks.measure(
                                format!("{label} {} [{name}]", keys.join(" ")),
                                want,
                                &measure(&session, &m, &f).await,
                            );
                        }
                    }
                }
                // Visuals with named roles.
                let roles: &[(&str, &str)] = match visual["visualType"].as_str().unwrap() {
                    "gauge" => &[
                        ("value", "Total Sales"),
                        ("target", "Target"),
                        ("max", "Target Stretch"),
                    ],
                    "kpi" => &[("indicator", "Total Sales"), ("goal", "Target")],
                    _ => &[],
                };
                let mut role_filter = filter.clone();
                if let Some(point) = visual["trendPoint"].as_str() {
                    role_filter = with(
                        role_filter,
                        ColumnFilterSpec::include("Date", "Year Month", [point]),
                    );
                }
                for (role, name) in roles {
                    checks.measure(
                        format!("{label} {role} [{name}]"),
                        &visual[*role],
                        &measure(&session, name, &role_filter).await,
                    );
                }
                if let Some(count) = visual["rowCount"].as_u64() {
                    // The customer table: one row per customer (name, account
                    // managers) with sales.
                    let rows = session
                        .dax_query(
                            "EVALUATE ROW(\"n\", COUNTROWS(SUMMARIZECOLUMNS(\
                             Customers[Customer Name], Customers[Account Managers], \
                             Customers[Account Manager Count], \"s\", [Total Sales])))",
                            &filter,
                            CancellationToken::new(),
                        )
                        .await
                        .unwrap();
                    checks.check(
                        format!("{label} rows"),
                        count.to_string(),
                        int_value(&rows).to_string(),
                    );
                }
            }
        }
    }
    checks.finish("report pages");
}

#[tokio::test(flavor = "multi_thread")]
async fn calculation_group_and_unrelated_filter_match_expected() {
    let Some(example) = example() else {
        return;
    };
    let exp = &example.expected;
    let session = example.refreshed().await;
    let mut checks = Checks::default();

    // 'Time Calc' items over [Total Sales] for one month of the Overview report.
    let group = &exp["calculationGroup"];
    let overview = filters_from_report(find_report(&session, "Acme Outfitters - Overview"));
    let month = with(
        overview,
        ColumnFilterSpec::include("Date", "Year Month", [group["month"].clone()]),
    );
    let name = group["measure"].as_str().unwrap();
    for (item, want) in group["items"].as_object().unwrap() {
        let filter = with(
            month.clone(),
            ColumnFilterSpec::include(
                group["table"].as_str().unwrap(),
                group["column"].as_str().unwrap(),
                [item.clone()],
            ),
        );
        let got = measure(&session, name, &filter).await;
        checks.measure(format!("{} [{name}]", item), want, &got);
        if item == "YoY %" {
            // The item's own format string wins over the measure's.
            assert_eq!(got.format_string.as_deref(), Some("+0.0%;-0.0%;0.0%"));
        }
    }

    // A report filter on Web Sessions, which has no relationship: it changes
    // the session measures only.
    let unrelated = &exp["unrelatedReportFilter"];
    let with_filters =
        filters_from_report(find_report(&session, unrelated["report"].as_str().unwrap()));
    let mut without_device = with_filters.clone();
    without_device
        .filters
        .retain(|f| !(f.table == "Web Sessions" && f.column == "Device"));
    assert_eq!(without_device.filters.len() + 1, with_filters.filters.len());
    for name in ["Sessions", "Total Sales"] {
        checks.measure(
            format!("{name} with the report filters"),
            &unrelated[name]["withReportFilters"],
            &measure(&session, name, &with_filters).await,
        );
        checks.measure(
            format!("{name} without the Device filter"),
            &unrelated[name]["withoutDeviceFilter"],
            &measure(&session, name, &without_device).await,
        );
    }
    checks.measure(
        "Sessions (All Devices) with the report filters",
        &unrelated["Sessions (All Devices)"],
        &measure(&session, "Sessions (All Devices)", &with_filters).await,
    );
    checks.finish("calculation group and unrelated filter");
}

#[tokio::test(flavor = "multi_thread")]
async fn row_level_security_matches_expected() {
    let Some(example) = example() else {
        return;
    };
    let exp = &example.expected;
    let session = example.refreshed().await;
    let mut checks = Checks::default();
    for case in exp["rls"].as_array().unwrap() {
        let roles: Vec<&str> = case["roles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.as_str().unwrap())
            .collect();
        let user = case["userName"].as_str().map(str::to_owned);
        let label = format!("{} {}", roles.join(" + "), user.as_deref().unwrap_or(""));
        session
            .set_view_as(ViewAs::roles(&roles, user), CancellationToken::new())
            .await
            .unwrap();
        let visible: BTreeMap<String, usize> = session
            .tables()
            .into_iter()
            .map(|t| (t.name, t.rows))
            .collect();
        for (table, rows) in case["visibleRows"].as_object().unwrap() {
            checks.check(
                format!("{label} rows {table}"),
                rows.to_string(),
                visible
                    .get(table)
                    .map_or("<missing>".to_owned(), ToString::to_string),
            );
        }
        let got = measures(&session, &FilterSpec::new()).await;
        for (name, want) in case["measures"].as_object().unwrap() {
            checks.measure(format!("{label} [{name}]"), want, &got[name]);
        }
    }
    session.clear_view_as();
    assert_eq!(
        session
            .tables()
            .iter()
            .find(|t| t.name == "Orders")
            .map(|t| t.rows),
        exp["rowCounts"]["Orders"].as_u64().map(|n| n as usize)
    );
    checks.finish("row-level security");
}

// ---------------------------------------------------------------------------
// Microsoft samples (PBIX with embedded data)
// ---------------------------------------------------------------------------

/// `samples/microsoft/expected.json` entries that are PBIX files.
fn sample_entries() -> Vec<Json> {
    let Some(path) = fixture("samples/microsoft/expected.json") else {
        return Vec::new();
    };
    read_json(&path)["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["detect"]["format"] == "pbix")
        .cloned()
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn sample_pbix_files_load_embedded_data() {
    let mut loaded = BTreeSet::new();
    for entry in sample_entries() {
        let id = entry["id"].as_str().unwrap();
        let Some(path) = fixture(entry["file"].as_str().unwrap()) else {
            assert!(
                entry["committed"] == false,
                "{id}: committed sample missing"
            );
            continue;
        };
        let session = Session::open(&path, SessionOptions::default()).unwrap();
        assert!(session.is_pbix());
        let report = session.refresh(RefreshOptions::all()).await.unwrap();
        println!("{id}: PBIX refresh {} ms", report.duration_ms);
        let model = &entry["model"];
        let mut checks = Checks::default();
        for table in model["tables"].as_array().unwrap() {
            let name = table["name"].as_str().unwrap();
            let t = report.table(name).unwrap_or_else(|| panic!("{id}: {name}"));
            assert_eq!(t.kind, RefreshKind::Pbix, "{id}: {name}");
            assert!(t.is_ok(), "{id}: {t:?}");
            if let Some(rows) = table["rows"].as_u64() {
                checks.check(
                    format!("{id} rows {name}"),
                    rows.to_string(),
                    t.rows.to_string(),
                );
            }
        }
        let values = session
            .measures(&FilterSpec::new(), CancellationToken::new())
            .await
            .unwrap();
        let names: BTreeSet<String> = values
            .iter()
            .map(|m| format!("{}[{}]", m.table, m.name))
            .collect();
        let want: BTreeSet<String> = model["measures"]["names"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap().to_owned())
            .collect();
        assert_eq!(names, want, "{id}");
        assert_eq!(
            values.len() as u64,
            model["measures"]["count"].as_u64().unwrap()
        );
        let by_name: BTreeMap<String, &MeasureValue> = values
            .iter()
            .map(|m| (format!("{}[{}]", m.table, m.name), m))
            .collect();
        if let Some(expected_values) = model["measures"]["values"].as_object() {
            for (name, want) in expected_values {
                checks.measure(format!("{id} {name}"), want, by_name[name.as_str()]);
            }
        }
        let errors: Vec<_> = values.iter().filter(|m| m.is_error()).collect();
        println!(
            "{id}: {} measures, {} with errors ({} unsupported)",
            values.len(),
            errors.len(),
            errors.iter().filter(|m| m.unsupported).count()
        );
        for m in &errors {
            println!(
                "  ⚠ [{}]: {}",
                m.name,
                m.error.as_deref().unwrap_or_default()
            );
        }
        checks.finish(id);
        loaded.insert(id.to_owned());
    }
    if !sample_entries().is_empty() {
        for id in ["revenue-opportunities", "corporate-spend", "regional-sales"] {
            assert!(loaded.contains(id), "{id} was not checked");
        }
    }
}

/// Removes a scratch folder when dropped (also when the test fails).
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The only `*<suffix>` entry of `dir`.
fn entry_with_suffix(dir: &Path, suffix: &str) -> PathBuf {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().ends_with(suffix))
        .unwrap_or_else(|| panic!("no *{suffix} in {}", dir.display()))
}

/// Measure name → formatted value of a refreshed session.
async fn measure_values(session: &Session) -> BTreeMap<String, String> {
    measures(session, &FilterSpec::new())
        .await
        .into_iter()
        .map(|(name, m)| (name, m.display()))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn pbip_cache_abf_loads_without_m() {
    let Some(pbix) = fixture("samples/microsoft/Revenue Opportunities.pbix") else {
        return;
    };
    let scratch = Scratch(
        PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("pbi-session-cache-abf-{}", std::process::id())),
    );
    let _ = std::fs::remove_dir_all(&scratch.0);

    // A PBIP saved from the PBIX, with the PBIX data as its local cache.
    let opened = pbi_core::open_path(&pbix, &pbi_core::OpenOptions::default()).unwrap();
    pbi_core::save_as_pbip(&opened, &scratch.0, &pbi_core::SaveOptions::default()).unwrap();
    let datamodel = opened.container.read("DataModel").unwrap();
    let abf =
        pbi_vertipaq::decompress_datamodel(&datamodel, &pbi_vertipaq::Limits::default()).unwrap();
    let model_dir = entry_with_suffix(&scratch.0, ".SemanticModel");
    std::fs::create_dir_all(model_dir.join(".pbi")).unwrap();
    std::fs::write(model_dir.join(".pbi/cache.abf"), &abf).unwrap();
    let pbip = entry_with_suffix(&scratch.0, ".pbip");

    // Reference values: the PBIX's own embedded data.
    let direct = Session::open(&pbix, SessionOptions::default()).unwrap();
    direct.refresh(RefreshOptions::all()).await.unwrap();
    let expected = measure_values(&direct).await;

    let session = Session::open(&pbip, SessionOptions::default()).unwrap();
    assert!(!session.is_pbix());
    assert!(session.has_cached_data());
    let info = session.cache_info().unwrap();
    assert!(
        info.entry.ends_with(".SemanticModel/.pbi/cache.abf"),
        "{info:?}"
    );
    assert_eq!(info.size, abf.len() as u64);
    assert!(info.modified.is_some() && info.path.is_some_and(|p| p.is_file()));

    let report = session.load_cached_data().await.unwrap();
    println!("cache load: {} ms", report.duration_ms);
    for t in &report.tables {
        println!(
            "  {:<20} {:?} {:>5} rows ok={}",
            t.name,
            t.kind,
            t.rows,
            t.is_ok()
        );
    }
    assert!(report.is_ok(), "{report:#?}");
    let opportunity = report.table("Opportunity").unwrap();
    assert_eq!(
        (opportunity.kind, opportunity.rows),
        (RefreshKind::Cache, 486)
    );
    let actual = measure_values(&session).await;
    assert!(!actual.is_empty());
    assert_eq!(actual, expected, "measures from the cache vs the PBIX");
    assert_eq!(
        actual.get("Revenue").map(String::as_str),
        Some("$1,968,250,939")
    );
    println!(
        "{} measures evaluate from the cache ({} with errors, as with the PBIX)",
        actual.len(),
        actual.values().filter(|v| v.starts_with('⚠')).count()
    );

    // Auto uses the cache; derived columns are re-applied after it.
    session
        .add_derived_column(
            "Opportunity",
            "Row One",
            DerivedColumn::Expression { dax: "1".into() },
        )
        .await
        .unwrap();
    let auto = session
        .refresh(RefreshOptions::all().with_source(RefreshSource::Auto))
        .await
        .unwrap();
    assert_eq!(
        auto.table("Opportunity").map(|t| t.kind),
        Some(RefreshKind::Cache)
    );
    assert!(auto.derived.iter().all(|d| d.active), "{:?}", auto.derived);
    assert_eq!(
        session
            .sql(
                "SELECT SUM(\"Row One\") FROM \"Opportunity\"",
                CancellationToken::new()
            )
            .await
            .unwrap()[0]
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Int64Array>()
            .map(|a| a.value(0)),
        Some(486)
    );

    // A model that is newer than its cache: a table, a data column and a
    // calculated column the cache does not have.
    let tables = model_dir.join("definition/tables");
    std::fs::write(
        tables.join("Not Cached.tmdl"),
        "table 'Not Cached'\n\n\tcolumn Key\n\t\tdataType: string\n\t\tsourceColumn: Key\n\n\tpartition 'Not Cached' = m\n\t\tmode: import\n\t\tsource = #table({\"Key\"}, {{\"a\"}})\n",
    )
    .unwrap();
    let model_tmdl = model_dir.join("definition/model.tmdl");
    let text = std::fs::read_to_string(&model_tmdl).unwrap();
    std::fs::write(&model_tmdl, format!("{text}ref table 'Not Cached'\n")).unwrap();
    let file = tables.join("Opportunity.tmdl");
    let text = std::fs::read_to_string(&file).unwrap();
    let at = text.find("\tpartition ").unwrap();
    std::fs::write(
        &file,
        format!(
            "{}\tcolumn 'Not In Cache'\n\t\tdataType: string\n\t\tsourceColumn: Not In Cache\n\n\tcolumn 'Name Length' = LEN(Opportunity[Name])\n\t\tdataType: int64\n\n{}",
            &text[..at],
            &text[at..]
        ),
    )
    .unwrap();
    let session = Session::open(&pbip, SessionOptions::default()).unwrap();
    assert!(session.model().table("Not Cached").is_some());
    let report = session.load_cached_data().await.unwrap();
    let missing = report.table("Not Cached").unwrap();
    assert_eq!(missing.kind, RefreshKind::Cache);
    assert!(
        matches!(&missing.status, TableStatus::Error { csv_fallback: true, message, .. } if message.contains("not in the local data cache")),
        "{missing:?}"
    );
    let opportunity = report.table("Opportunity").unwrap();
    assert!(opportunity.status.is_ok(), "{opportunity:?}");
    assert_eq!(opportunity.rows, 486);
    assert!(
        opportunity
            .column("Not In Cache")
            .is_some_and(|c| c.status.is_skipped()),
        "{opportunity:?}"
    );
    assert!(
        opportunity
            .column("Name Length")
            .is_some_and(|c| c.status.is_ok()),
        "{opportunity:?}"
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == pbi_session::refresh::codes::CACHE_MISSING)
    );
    assert_eq!(report.table("Account").map(|t| t.rows), Some(357));
}
