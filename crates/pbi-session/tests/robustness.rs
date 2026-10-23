#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Refresh robustness: offline M sources, "Load CSV manually…",
//! cancellation, unsupported M / DAX, failed dependencies, and the query
//! API on small in-memory models. Uses only committed fixtures.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use arrow::array::AsArray;
use arrow::datatypes::Int64Type;
use chrono::TimeZone;
use pbi_session::{
    CancellationToken, ColumnFilterSpec, CsvExportOptions, CsvOptions, DerivedColumn, DerivedKind,
    FilterSpec, FixedClock, ModelDocOptions, OFFLINE_HINT, PageRequest, RefreshEvent, RefreshKind,
    RefreshOptions, RefreshSource, Session, SessionError, SessionOptions, TableStatus, Value,
    is_derived, model_doc,
};

fn reference_root() -> PathBuf {
    std::env::var_os("PBIVIEW_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
}

fn options() -> SessionOptions {
    SessionOptions {
        clock: Some(Arc::new(FixedClock::utc(
            chrono::Utc
                .with_ymd_and_hms(2026, 10, 21, 14, 5, 0)
                .single()
                .unwrap(),
        ))),
        ..SessionOptions::default()
    }
}

/// The example PBIP without a blob map: its CSVs cannot be reached.
fn offline_pbip() -> Option<Session> {
    let path = reference_root().join("examples/acme-outfitters/AcmeOutfitters.pbip");
    if !path.exists() {
        println!("SKIP: {} missing", path.display());
        return None;
    }
    Some(Session::open(&path, options()).unwrap())
}

/// A session over an in-memory `model.bim` with the given tables JSON.
fn bim_session(tables: &str) -> Session {
    let bim = format!(
        r#"{{"compatibilityLevel": 1567, "model": {{"culture": "en-US", "tables": {tables}}}}}"#
    );
    let opened = pbi_core::open_bytes(
        "model.bim",
        bim.into_bytes(),
        &pbi_core::OpenOptions::default(),
    )
    .unwrap();
    assert!(
        opened.document.model.is_some(),
        "{:?}",
        opened.document.diagnostics
    );
    Session::from_opened(opened, options()).unwrap()
}

const SALES: &str = r##"{
  "name": "Sales",
  "columns": [
    {"name": "Id", "dataType": "int64", "sourceColumn": "id"},
    {"name": "Product", "dataType": "string", "sourceColumn": "product"},
    {"name": "Amount", "dataType": "decimal", "sourceColumn": "amount"},
    {"type": "calculated", "name": "Double", "dataType": "decimal", "expression": "Sales[Amount] * 2"},
    {"type": "calculated", "name": "Bad", "dataType": "dateTime", "expression": "PMT(0.05, 12, 1000)"},
    {"type": "calculated", "name": "After Bad", "dataType": "string", "expression": "FORMAT(Sales[Bad], \"yyyy\")"}
  ],
  "partitions": [{"name": "Sales", "mode": "import", "source": {"type": "m",
    "expression": "#table(type table [id = Int64.Type, product = text, amount = number, unused = text], {{1, \"a\", 1.5, \"x\"}, {2, \"b\", 2.25, \"y\"}, {3, \"a\", 3, \"z\"}})"}}],
  "measures": [
    {"name": "Total", "expression": "SUM(Sales[Amount])", "formatString": "\\$#,0.00"},
    {"name": "Rows", "expression": "COUNTROWS(Sales)", "formatString": "#,0"},
    {"name": "Ranked", "expression": "XNPV(0.1, Sales, Sales[Amount], Sales[Id])"},
    {"name": "Broken", "expression": "1 +"}
  ]
}"##;

const PRODUCTS: &str = r##"{
  "name": "Products",
  "columns": [{"type": "calculatedTableColumn", "name": "Year", "dataType": "string", "sourceColumn": "[Year]"}],
  "partitions": [{"name": "Products", "source": {"type": "calculated",
    "expression": "DISTINCT(SELECTCOLUMNS(Sales, \"Year\", Sales[After Bad]))"}}]
}"##;

const DB: &str = r##"{
  "name": "Db",
  "columns": [{"name": "x", "dataType": "string", "sourceColumn": "x"}],
  "partitions": [{"name": "Db", "mode": "import", "source": {"type": "m", "expression": "Sql.Database(\"server\", \"db\")"}}]
}"##;

const SLOW: &str = r##"{
  "name": "Slow",
  "columns": [{"name": "n", "dataType": "int64", "sourceColumn": "n"}],
  "partitions": [{"name": "Slow", "mode": "import", "source": {"type": "m",
    "expression": "#table({\"n\"}, {{List.Count(List.Generate(() => 0, each _ < 9000000, each _ + 1))}})"}}]
}"##;

/// Account managers as Python-style lists, like the example's customers.
const ACCOUNTS: &str = r##"{
  "name": "Accounts",
  "columns": [
    {"name": "Id", "dataType": "int64", "sourceColumn": "id"},
    {"name": "Managers", "dataType": "string", "sourceColumn": "managers"}
  ],
  "partitions": [{"name": "Accounts", "mode": "import", "source": {"type": "m",
    "expression": "#table(type table [id = Int64.Type, managers = text], {{1, \"['Ana Ruiz']\"}, {2, \"['Lena Ford', \"\"O'Neil Park\"\"]\"}, {3, \"Jo Kim\"}, {4, \"[]\"}})"}}],
  "measures": [
    {"name": "Parsed Managers", "expression": "COUNTROWS(FILTER(Accounts, NOT ISBLANK(Accounts[Managers (parsed)])))"},
    {"name": "Taken", "expression": "1"}
  ]
}"##;

async fn kpis(session: &Session, filter: &FilterSpec) -> BTreeMap<String, String> {
    session
        .measures(filter, CancellationToken::new())
        .await
        .unwrap()
        .into_iter()
        .map(|m| (m.name.clone(), m.display()))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn offline_refresh_fails_m_tables_and_skips_dependants() {
    let Some(session) = offline_pbip() else {
        return;
    };
    assert!(!session.has_blob_map());
    let report = session.refresh(RefreshOptions::all()).await.unwrap();
    for name in [
        "Orders",
        "Customers",
        "Products",
        "Stores",
        "Targets",
        "Returns",
        "Web Sessions",
    ] {
        let t = report.table(name).unwrap();
        assert_eq!(t.kind, RefreshKind::M);
        match &t.status {
            TableStatus::Error {
                message,
                hint,
                csv_fallback,
                unsupported,
            } => {
                assert!(message.contains("offline"), "{message}");
                assert_eq!(hint.as_deref(), Some(OFFLINE_HINT));
                assert!(*csv_fallback);
                assert!(!*unsupported);
            }
            other => panic!("{name}: {other:?}"),
        }
        assert_eq!(t.rows, 0);
    }
    let orders = report.table("Orders").unwrap();
    assert_eq!(orders.columns.len(), 4);
    assert!(orders.columns.iter().all(|c| c.status.is_skipped()));
    let never_sold = report.table("Products Never Sold").unwrap();
    assert_eq!(never_sold.kind, RefreshKind::Skipped);
    let reason = never_sold.status.message().unwrap_or_default();
    assert!(
        reason.starts_with("depends on ") && reason.ends_with("which failed to refresh"),
        "{reason}"
    );
    // What does not read the CSVs still loads: the M #table, the DAX
    // calendar and its calculated column, the calculation group.
    let last = report.table("Last Refreshed").unwrap();
    assert!(last.status.is_ok());
    assert_eq!(last.rows, 1);
    let date = report.table("Date").unwrap();
    assert_eq!((date.kind, date.rows), (RefreshKind::Calculated, 731));
    assert!(date.is_ok(), "{date:?}");
    assert!(
        report
            .table("Time Calc")
            .is_some_and(|t| t.is_ok() && t.rows == 4)
    );
    assert!(!report.is_ok());

    // Measures still evaluate (over empty tables) without errors or panics.
    let values = kpis(&session, &FilterSpec::new()).await;
    assert_eq!(
        values.get("Products Never Sold Count").map(String::as_str),
        Some("0")
    );
    assert_eq!(
        values.get("Unknown Product Orders").map(String::as_str),
        Some("0")
    );
    assert_eq!(values.get("Total Sales").map(String::as_str), Some(""));
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
    let status = session.table_status("orders").unwrap();
    assert!(status.status.is_error());
    let info = session.tables();
    assert!(
        info.iter()
            .any(|t| t.name == "Last Refreshed" && t.loaded && t.rows == 1)
    );
    assert!(info.iter().any(|t| t.name == "Orders" && !t.loaded));

    assert!(matches!(
        session.refresh(RefreshOptions::tables(["Nope"])).await,
        Err(SessionError::UnknownTable(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn load_csv_fallback_recomputes_dependants() {
    let Some(session) = offline_pbip() else {
        return;
    };
    session.refresh(RefreshOptions::all()).await.unwrap();

    let stores = "Store ID,Store Name,Region,Channel\n\
                  S1,Seattle Flagship,West,Retail\n\
                  S2,Boston Back Bay,East,Retail\n\
                  S3,Web Store,West,Online\n";
    let report = session
        .load_csv("stores", stores, CsvOptions::default())
        .await
        .unwrap();
    let t = report.table("Stores").unwrap();
    assert_eq!(t.kind, RefreshKind::Manual);
    assert_eq!(t.rows, 3);
    assert!(t.is_ok(), "{t:?}");
    assert!(t.column("Store Label").is_some_and(|c| c.status.is_ok()));
    assert_eq!(
        report
            .diagnostics
            .iter()
            .filter(|d| d.code == pbi_session::refresh::codes::CSV_MISSING_COLUMN)
            .count(),
        7
    );
    let products = "Product ID,Product Name,Category,List Price,Unit Cost\n\
                    P1,Juniper Tent,Camping,200,120\n\
                    P2,Alder Pack,Hiking,100,50\n\
                    P3,Cedar Stove,Camping,40,25\n\
                    P4,Birch Poles,Hiking,30,12\n";
    session
        .load_csv("Products", products, CsvOptions::default())
        .await
        .unwrap();

    let orders = b"order id;order date;customer id;store id;product id;quantity;unit price;discount;status;line amount\n\
                   O1;2025-01-10;C1;S1;P1;2;200;0;Completed;400\n\
                   O2;2025-02-05;C2;S2;P2;1;100;0;Completed;100\n\
                   O3;2025-03-15;C1;S3;P3;3;40;0;Cancelled;120\n\
                   O4;2025-04-01;C3;S1;P9;1;50;0;Pending;50\n";
    let report = session
        .load_csv(
            "Orders",
            orders.as_slice(),
            CsvOptions {
                delimiter: ';',
                ..CsvOptions::default()
            },
        )
        .await
        .unwrap();
    let t = report.table("Orders").unwrap();
    assert_eq!((t.kind, t.rows), (RefreshKind::Manual, 4));
    assert!(t.column("Line Cost").is_some_and(|c| c.status.is_ok()));
    let products = report.table("Products").unwrap();
    assert_eq!(products.kind, RefreshKind::Calculated);
    assert!(
        products
            .column("Units Sold (All Time)")
            .is_some_and(|c| c.status.is_ok())
    );
    assert_eq!(report.table("Products Never Sold").map(|t| t.rows), Some(1));

    let main = kpis(&session, &FilterSpec::new()).await;
    for (name, want) in [
        ("Total Sales", "$670.00"),
        ("Order Count", "4"),
        ("Total Cost", "$365.00"),
        ("Margin %", "45.5%"),
        ("Unknown Product Orders", "1"),
        ("Products Never Sold Count", "1"),
        ("Completed Sales", "$500.00"),
    ] {
        assert_eq!(main.get(name).map(String::as_str), Some(want), "{name}");
    }
    let west = FilterSpec::new().with(ColumnFilterSpec::include("Stores", "Region", ["west"]));
    let west = kpis(&session, &west).await;
    for (name, want) in [
        ("Total Sales", "$570.00"),
        ("Order Count", "3"),
        ("Unknown Product Orders", "1"),
        ("Products Never Sold Count", "1"),
    ] {
        assert_eq!(west.get(name).map(String::as_str), Some(want), "{name}");
    }

    let calc = session
        .load_csv(
            "Products Never Sold",
            "Product ID\nx\n",
            CsvOptions::default(),
        )
        .await;
    assert!(matches!(calc, Err(SessionError::Csv { .. })), "{calc:?}");
    let unmatched = session
        .load_csv("Orders", "nothing,matches\n1,2\n", CsvOptions::default())
        .await;
    assert!(
        matches!(unmatched, Err(SessionError::Csv { .. })),
        "{unmatched:?}"
    );
    assert_eq!(session.table_batch("Orders").unwrap().num_rows(), 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn unsupported_m_and_dax_are_reported_not_fatal() {
    let session = bim_session(&format!("[{SALES}, {PRODUCTS}, {DB}]"));
    let report = session.refresh(RefreshOptions::all()).await.unwrap();

    let sales = report.table("Sales").unwrap();
    assert!(sales.status.is_ok());
    assert_eq!(sales.rows, 3);
    assert!(sales.column("Double").unwrap().status.is_ok());
    match &sales.column("Bad").unwrap().status {
        TableStatus::Error {
            message,
            unsupported,
            csv_fallback,
            ..
        } => {
            assert!(message.contains("PMT"), "{message}");
            assert!(*unsupported);
            assert!(!*csv_fallback);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        sales.column("After Bad").unwrap().status,
        TableStatus::Skipped {
            reason: "depends on Sales[Bad], which failed to refresh".into()
        }
    );
    let products = report.table("Products").unwrap();
    assert_eq!(products.kind, RefreshKind::Skipped);
    // SELECTCOLUMNS(Sales, …) reads the whole table, so every calculated
    // column of Sales is a dependency; the first failed one is named.
    assert_eq!(
        products.status.message(),
        Some("depends on Sales[Bad], which failed to refresh")
    );
    match &report.table("Db").unwrap().status {
        TableStatus::Error {
            message,
            unsupported,
            csv_fallback,
            hint,
        } => {
            assert!(message.contains("Sql.Database"), "{message}");
            assert!(*unsupported && *csv_fallback && hint.is_some());
        }
        other => panic!("{other:?}"),
    }

    // Extra query columns are ignored; the stored table follows the model.
    let batch = session.table_batch("Sales").unwrap();
    let names: Vec<String> = batch
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect();
    assert_eq!(names, ["Id", "Product", "Amount", "Double"]);

    let measures = session
        .measures(&FilterSpec::new(), CancellationToken::new())
        .await
        .unwrap();
    let get = |n: &str| measures.iter().find(|m| m.name == n).unwrap();
    assert_eq!(get("Total").formatted.as_deref(), Some("$6.75"));
    assert_eq!(get("Total").value, Some(Value::Decimal(67_500)));
    assert_eq!(get("Rows").display(), "3");
    let ranked = get("Ranked");
    assert!(ranked.unsupported && ranked.value.is_none());
    assert_eq!(ranked.dax, "XNPV(0.1, Sales, Sales[Amount], Sales[Id])");
    assert!(ranked.display().starts_with("⚠ "), "{}", ranked.display());
    let broken = get("Broken");
    assert!(broken.is_error() && !broken.unsupported);
    assert_eq!(broken.dax, "1 +");

    let filtered = session
        .evaluate_measure(
            "Sales[Rows]",
            &FilterSpec::new().with(ColumnFilterSpec::exclude("Sales", "Product", ["A"])),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(filtered.value, Some(Value::Integer(1)));
    assert!(matches!(
        session
            .evaluate_measure("Nope", &FilterSpec::new(), CancellationToken::new())
            .await,
        Err(SessionError::UnknownMeasure(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn grid_sql_and_dax_queries() {
    let session = bim_session(&format!("[{SALES}]"));
    session.refresh(RefreshOptions::all()).await.unwrap();
    let cancel = CancellationToken::new;

    let page = session
        .page(
            &PageRequest::new("sales", 0, 2).sort_by("Amount", false),
            cancel(),
        )
        .await
        .unwrap();
    assert_eq!(page.num_rows(), 2);
    assert_eq!(page.column(0).as_primitive::<Int64Type>().value(0), 3);
    assert_eq!(
        session
            .row_count("Sales", Some("\"Product\" = 'a'"), cancel())
            .await
            .unwrap(),
        2
    );
    let stats = session.column_stats("Sales", "Product").await.unwrap();
    assert_eq!((stats.rows, stats.distinct), (3, 2));
    let sql = session
        .sql(
            "SELECT COUNT(*) FROM \"Sales\" WHERE \"Double\" > 4",
            cancel(),
        )
        .await
        .unwrap();
    assert_eq!(sql[0].column(0).as_primitive::<Int64Type>().value(0), 2);
    let dax = session
        .dax_query(
            "EVALUATE ROW(\"n\", COUNTROWS(Sales), \"f\", FORMAT(SUM(Sales[Amount]), \"0.0\"))",
            &FilterSpec::new(),
            cancel(),
        )
        .await
        .unwrap();
    assert_eq!(dax[0].num_rows(), 1);
    assert_eq!(dax[0].column(1).as_string::<i32>().value(0), "6.8");
    assert!(matches!(
        session
            .page(&PageRequest::new("Nope", 0, 1), cancel())
            .await,
        Err(SessionError::Engine(_))
    ));
    assert!(matches!(
        session.table_batch("Nope"),
        Err(SessionError::UnknownTable(_))
    ));
    assert_eq!(session.format_value(&Value::Double(0.5), Some("0%")), "50%");
}

#[tokio::test(flavor = "multi_thread")]
async fn cancellation_keeps_the_previous_data() {
    let session = bim_session(&format!("[{SALES}, {SLOW}]"));
    // Load the fast table first.
    session
        .refresh(RefreshOptions::tables(["Sales"]))
        .await
        .unwrap();
    assert_eq!(session.table_batch("Sales").unwrap().num_rows(), 3);

    // Already cancelled: nothing runs.
    let token = CancellationToken::new();
    token.cancel();
    assert!(matches!(
        session
            .refresh(RefreshOptions::all().with_cancel(token))
            .await,
        Err(SessionError::Cancelled)
    ));

    // Cancelled while the slow M query runs.
    let token = CancellationToken::new();
    let trigger = token.clone();
    let start = Instant::now();
    let result = session
        .refresh(
            RefreshOptions::all()
                .with_cancel(token)
                .with_progress(move |e| {
                    if matches!(&e, RefreshEvent::TableStarted { name, .. } if name == "Slow") {
                        let t = trigger.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_millis(200)).await;
                            t.cancel();
                        });
                    }
                }),
        )
        .await;
    assert!(matches!(result, Err(SessionError::Cancelled)), "{result:?}");
    assert!(start.elapsed() < Duration::from_secs(60));
    assert_eq!(session.table_batch("Sales").unwrap().num_rows(), 3);
    assert!(
        session
            .tables()
            .iter()
            .any(|t| t.name == "Slow" && !t.loaded)
    );

    // Cancelled measures.
    let token = CancellationToken::new();
    token.cancel();
    assert!(matches!(
        session.measures(&FilterSpec::new(), token).await,
        Err(SessionError::Cancelled)
    ));
}

#[test]
fn session_futures_are_send() {
    fn assert_send<T: Send>(_: &T) {}
    let session = bim_session(&format!("[{SALES}]"));
    let filter = FilterSpec::new();
    let refresh = session.refresh(RefreshOptions::all());
    assert_send(&refresh);
    let measures = session.measures(&filter, CancellationToken::new());
    assert_send(&measures);
    let csv = session.load_csv("Sales", "Id\n1\n", CsvOptions::default());
    assert_send(&csv);
    let request = PageRequest::new("Sales", 0, 1);
    let page = session.page(&request, CancellationToken::new());
    assert_send(&page);
    let sql = session.sql("SELECT 1", CancellationToken::new());
    assert_send(&sql);
    let stats = session.column_stats("Sales", "Id");
    assert_send(&stats);
    let derived =
        session.add_derived_column("Sales", "X", DerivedColumn::Expression { dax: "1".into() });
    assert_send(&derived);
    let removed = session.remove_derived_column("Sales", "X");
    assert_send(&removed);
    let cached = session.load_cached_data();
    assert_send(&cached);
}

/// Strings of `table[column]`.
fn texts(session: &Session, table: &str, column: &str) -> Vec<Option<String>> {
    let batch = session.table_batch(table).unwrap();
    let array = batch.column_by_name(column).unwrap();
    let array = arrow::compute::cast(array, &arrow::datatypes::DataType::Utf8).unwrap();
    array
        .as_string::<i32>()
        .iter()
        .map(|v| v.map(str::to_owned))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn derived_columns_are_visible_everywhere_and_survive_refreshes() {
    let session = bim_session(&format!("[{ACCOUNTS}]"));
    let report = session.refresh(RefreshOptions::all()).await.unwrap();
    assert!(report.is_ok(), "{report:#?}");
    assert!(report.derived.is_empty());
    let parsed = DerivedColumn::People {
        source_column: "managers".into(),
    };

    // Before: the measure cannot see the column.
    let before = kpis(&session, &FilterSpec::new()).await;
    assert!(before["Parsed Managers"].starts_with('⚠'), "{before:?}");

    let info = session
        .add_derived_column("accounts", "Managers (parsed)", parsed.clone())
        .await
        .unwrap();
    assert_eq!(info.table, "Accounts");
    assert_eq!(info.kind, DerivedKind::People);
    assert!(info.active && info.status.is_ok());
    assert_eq!(
        texts(&session, "Accounts", "Managers (parsed)"),
        [
            Some("Ana Ruiz".to_owned()),
            Some("Lena Ford; O'Neil Park".to_owned()),
            Some("Jo Kim".to_owned()),
            None
        ]
    );

    // DAX measures, DAX queries, SQL, statistics and exports see it.
    let after = kpis(&session, &FilterSpec::new()).await;
    assert_eq!(after["Parsed Managers"], "3");
    let cancel = CancellationToken::new;
    let sql = session
        .sql(
            "SELECT COUNT(*) FROM \"Accounts\" WHERE \"Managers (parsed)\" LIKE '%Park'",
            cancel(),
        )
        .await
        .unwrap();
    assert_eq!(sql[0].column(0).as_primitive::<Int64Type>().value(0), 1);
    let stats = session
        .column_stats("Accounts", "Managers (parsed)")
        .await
        .unwrap();
    assert_eq!((stats.rows, stats.blanks, stats.distinct), (4, 1, 3));
    let mut csv = Vec::new();
    session
        .export_csv("Accounts", &mut csv, &CsvExportOptions::default())
        .unwrap();
    let csv = String::from_utf8(csv).unwrap();
    assert!(csv.starts_with("Id,Managers,Managers (parsed)\n"), "{csv}");
    assert!(csv.contains(",Ana Ruiz\n"), "{csv}");
    let exported = session.export_batch("Accounts").unwrap();
    assert_eq!(
        exported
            .schema()
            .field_with_name("Managers (parsed)")
            .unwrap()
            .metadata()
            .get(pbi_session::export::meta::DERIVED)
            .map(String::as_str),
        Some("true")
    );

    // The session model has a marked synthetic column; the document model
    // and model exports do not.
    let model = session.model();
    let column = model.column("Accounts", "Managers (parsed)").unwrap();
    assert!(is_derived(column) && column.is_calculated());
    assert!(
        session
            .base_model()
            .column("Accounts", "Managers (parsed)")
            .is_none()
    );
    let doc = model_doc(&model, &ModelDocOptions::default());
    assert_eq!(doc.tables[0].columns.len(), 2);
    assert_eq!(doc.summary.calculated_columns, 0);

    // A DAX expression (using the derived column) and an array.
    let len = session
        .add_derived_column(
            "Accounts",
            "Name Length",
            DerivedColumn::Expression {
                dax: "LEN(Accounts[Managers (parsed)]) + Accounts[Id] * 0".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(len.data_type, pbi_model::DataType::Int64);
    assert_eq!(
        texts(&session, "Accounts", "Name Length"),
        [
            Some("8".to_owned()),
            Some("22".to_owned()),
            Some("6".to_owned()),
            Some("0".to_owned())
        ]
    );
    let flags: arrow::array::ArrayRef = Arc::new(arrow::array::BooleanArray::from(vec![
        true, false, true, false,
    ]));
    session
        .add_derived_column("Accounts", "Flag", DerivedColumn::Array(flags))
        .await
        .unwrap();
    let dax = session
        .dax_query(
            "EVALUATE ROW(\"n\", CALCULATE(COUNTROWS(Accounts), Accounts[Flag] = TRUE()))",
            &FilterSpec::new(),
            cancel(),
        )
        .await
        .unwrap();
    assert_eq!(dax[0].column(0).as_primitive::<Int64Type>().value(0), 2);

    // Errors: name collisions, a bad source, bad DAX, a short array.
    for (name, column) in [
        ("MANAGERS", parsed.clone()),
        ("Taken", parsed.clone()),
        ("flag", parsed.clone()),
        (" ", parsed.clone()),
        (
            "From Id",
            DerivedColumn::People {
                source_column: "Id".into(),
            },
        ),
        (
            "Nope",
            DerivedColumn::People {
                source_column: "Nope".into(),
            },
        ),
        (
            "Bad DAX",
            DerivedColumn::Expression {
                dax: "Accounts[Nope] +".into(),
            },
        ),
        (
            "Short",
            DerivedColumn::Array(Arc::new(arrow::array::Int64Array::from(vec![1]))),
        ),
    ] {
        let result = session.add_derived_column("Accounts", name, column).await;
        assert!(
            matches!(result, Err(SessionError::Derived { .. })),
            "{name}: {result:?}"
        );
    }
    assert!(matches!(
        session
            .add_derived_column("Nope", "X", parsed.clone())
            .await,
        Err(SessionError::UnknownTable(_))
    ));
    assert_eq!(session.derived_columns().len(), 3);

    // A refresh re-applies them in order.
    let report = session.refresh(RefreshOptions::all()).await.unwrap();
    assert_eq!(report.derived.len(), 3);
    assert!(report.derived.iter().all(|d| d.active && d.status.is_ok()));
    assert_eq!(
        session
            .table_batch("Accounts")
            .unwrap()
            .schema()
            .fields()
            .iter()
            .map(|f| f.name().as_str())
            .collect::<Vec<_>>(),
        ["Id", "Managers", "Managers (parsed)", "Name Length", "Flag"]
    );

    // A CSV load with another row count: the array no longer fits; the
    // other columns are recomputed.
    let report = session
        .load_csv(
            "Accounts",
            "id,managers\n7,['Mia Chen']\n",
            CsvOptions::default(),
        )
        .await
        .unwrap();
    let flag = report.derived.iter().find(|d| d.name == "Flag").unwrap();
    assert!(!flag.active);
    assert!(
        matches!(&flag.status, TableStatus::Error { message, .. } if message.contains("4 values"))
    );
    assert_eq!(
        texts(&session, "Accounts", "Managers (parsed)"),
        [Some("Mia Chen".to_owned())]
    );
    assert!(
        session
            .table_batch("Accounts")
            .unwrap()
            .column_by_name("Flag")
            .is_none()
    );
    assert!(session.model().column("Accounts", "Flag").is_none());
    assert_eq!(session.derived_columns().len(), 3);

    // Removing a source: the dependant expression fails on the next run and
    // is taken out of the table.
    let removed = session
        .remove_derived_column("accounts", "managers (PARSED)")
        .await
        .unwrap();
    assert_eq!(removed.name, "Managers (parsed)");
    assert!(
        session
            .model()
            .column("Accounts", "Managers (parsed)")
            .is_none()
    );
    assert!(
        session
            .table_batch("Accounts")
            .unwrap()
            .column_by_name("Managers (parsed)")
            .is_none()
    );
    assert!(matches!(
        session
            .remove_derived_column("Accounts", "Managers (parsed)")
            .await,
        Err(SessionError::Derived { .. })
    ));
    assert!(matches!(
        session.remove_derived_column("Accounts", "Managers").await,
        Err(SessionError::Derived { .. })
    ));
    let report = session.refresh(RefreshOptions::all()).await.unwrap();
    let length = report
        .derived
        .iter()
        .find(|d| d.name == "Name Length")
        .unwrap();
    assert!(!length.active && length.status.is_error(), "{length:?}");
    let flag = report.derived.iter().find(|d| d.name == "Flag").unwrap();
    assert!(flag.active, "the array fits again after the refresh");
}

#[tokio::test(flavor = "multi_thread")]
async fn derived_columns_on_tables_not_loaded_yet() {
    let session = bim_session(&format!("[{ACCOUNTS}]"));
    let parsed = DerivedColumn::People {
        source_column: "Managers".into(),
    };
    let info = session
        .add_derived_column("Accounts", "Managers (parsed)", parsed)
        .await
        .unwrap();
    assert!(info.active);
    assert!(!session.tables()[0].loaded);
    let report = session.refresh(RefreshOptions::all()).await.unwrap();
    assert!(report.derived[0].active);
    assert_eq!(texts(&session, "Accounts", "Managers (parsed)").len(), 4);

    let other = bim_session(&format!("[{ACCOUNTS}]"));
    let parsed = DerivedColumn::People {
        source_column: "Managers".into(),
    };
    other
        .add_derived_column("Accounts", "P", parsed)
        .await
        .unwrap();
    other.remove_derived_column("Accounts", "P").await.unwrap();
    let info = &other.tables()[0];
    assert!(!info.loaded);
    assert_eq!(info.columns, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn documents_without_a_cache() {
    let session = bim_session(&format!("[{SALES}]"));
    assert!(!session.has_cached_data());
    assert!(session.cache_info().is_none());
    assert!(matches!(
        session.load_cached_data().await,
        Err(SessionError::NoCache)
    ));
    let report = session
        .refresh(RefreshOptions::all().with_source(RefreshSource::Auto))
        .await
        .unwrap();
    assert_eq!(report.table("Sales").map(|t| t.kind), Some(RefreshKind::M));
}
