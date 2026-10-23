//! Acceptance tests: the Acme Outfitters partitions evaluated offline through its `blob-map.json`.
//!
//! The scripts under `tests/data/` are copies of the example's TMDL partitions and expressions
//! (`pbi-tmdl`'s fixture test checks that they stay equal); the CSVs are committed under
//! `reference/examples/acme-outfitters/data/`. When the fixture root (`$PBIVIEW_REFERENCE`) has no
//! example, the tests that need it print `SKIP: ...` and pass.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use arrow::array::{
    Array, Date32Array, Float64Array, Int64Array, StringArray, TimestampMicrosecondArray,
};
use arrow::datatypes::{DataType, TimeUnit};
use arrow::record_batch::RecordBatch;
use chrono::{TimeZone, Utc};
use indexmap::IndexMap;
use pbi_m::{
    BlobEntry, BlobResolver, Engine, EvalOptions, FixedClock, MError, MapBlobResolver,
    ResolverError, ResolverErrorKind, Value,
};

const ORDER_COLUMNS: [(&str, DataType); 11] = [
    ("Order ID", DataType::Utf8),
    ("Order Date", DataType::Date32),
    ("Ship Date", DataType::Date32),
    ("Customer ID", DataType::Utf8),
    ("Store ID", DataType::Utf8),
    ("Product ID", DataType::Utf8),
    ("Quantity", DataType::Int64),
    ("Unit Price", DataType::Float64),
    ("Discount", DataType::Float64),
    ("Status", DataType::Utf8),
    ("Line Amount", DataType::Float64),
];

const CUSTOMER_COLUMNS: [(&str, DataType); 11] = [
    ("Customer ID", DataType::Utf8),
    ("Customer Name", DataType::Utf8),
    ("Email", DataType::Utf8),
    ("Email Domain", DataType::Utf8),
    ("Segment", DataType::Utf8),
    ("City", DataType::Utf8),
    ("State", DataType::Utf8),
    ("Join Date", DataType::Date32),
    ("Loyalty Tier", DataType::Utf8),
    ("Account Managers", DataType::Utf8),
    ("Account Manager Count", DataType::Int64),
];

const STORE_COLUMNS: [&str; 11] = [
    "Store ID",
    "Store Name",
    "Region",
    "Channel",
    "City",
    "State",
    "Manager Email",
    "Open Date",
    "Square Feet",
    "Store Manager",
    "Region Code",
];

fn reference_dir() -> PathBuf {
    std::env::var_os("PBIVIEW_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
}

fn example_dir() -> PathBuf {
    reference_dir().join("examples/acme-outfitters")
}

fn data(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The model's shared expressions: the container parameter and the two M functions.
fn shared() -> IndexMap<String, String> {
    let mut shared = IndexMap::new();
    shared.insert(
        "StorageContainerUrl".to_owned(),
        data("storage_container_url.m"),
    );
    shared.insert("fnCleanKey".to_owned(), data("fn_clean_key.m"));
    shared.insert("fnParseNames".to_owned(), data("fn_parse_names.m"));
    shared
}

fn clock() -> FixedClock {
    FixedClock::utc(
        Utc.with_ymd_and_hms(2026, 10, 21, 17, 30, 15)
            .single()
            .unwrap_or_else(|| panic!("valid date")),
    )
}

/// An engine reading the example's blob map, or `None` (after printing SKIP) when it is missing.
fn fixture_engine() -> Option<Engine> {
    let map = example_dir().join("blob-map.json");
    if !map.is_file() {
        println!("SKIP: {} missing", map.display());
        return None;
    }
    let resolver = MapBlobResolver::from_map_file(&map).unwrap_or_else(|e| panic!("{e}"));
    Some(Engine::new(EvalOptions {
        resolver: Arc::new(resolver),
        clock: Arc::new(clock()),
        ..EvalOptions::default()
    }))
}

/// `rowCounts[table]` of the example's `expected.json`.
fn expected_rows(table: &str) -> usize {
    let path = example_dir().join("expected.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}"));
    json["rowCounts"][table]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or_else(|| panic!("no rowCounts.{table} in expected.json"))
}

fn columns(spec: &[(&str, DataType)]) -> Vec<(String, DataType)> {
    spec.iter()
        .map(|(n, t)| ((*n).to_owned(), t.clone()))
        .collect()
}

fn utf8(names: &[&str]) -> Vec<(String, DataType)> {
    names
        .iter()
        .map(|n| ((*n).to_owned(), DataType::Utf8))
        .collect()
}

fn column_names(batch: &RecordBatch) -> Vec<&str> {
    batch
        .schema_ref()
        .fields()
        .iter()
        .map(|f| f.name().as_str())
        .collect()
}

fn typed<'a, T: 'static>(batch: &'a RecordBatch, name: &str) -> &'a T {
    batch
        .column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<T>())
        .unwrap_or_else(|| panic!("no column {name} of the expected type"))
}

fn text_column<'a>(batch: &'a RecordBatch, name: &str) -> &'a StringArray {
    typed::<StringArray>(batch, name)
}

#[test]
fn orders_partition_matches_the_expected_row_count() {
    let Some(engine) = fixture_engine() else {
        return;
    };
    let batch = engine
        .evaluate_partition_to_arrow(&data("orders.m"), &shared(), &columns(&ORDER_COLUMNS))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(batch.num_rows(), expected_rows("Orders"));
    let names: Vec<&str> = ORDER_COLUMNS.iter().map(|(n, _)| *n).collect();
    assert_eq!(column_names(&batch), names);

    // The query's own column order (Table.RenameColumns) is exactly the model's.
    let v = engine
        .evaluate_query(&data("orders.m"), &shared())
        .unwrap_or_else(|e| panic!("{e}"));
    let table = v.as_table().unwrap_or_else(|| panic!("not a table: {v}"));
    let own: Vec<&str> = table.column_names().iter().map(|n| &**n).collect();
    assert_eq!(own, names);

    // Rows without an id and duplicates (also near duplicates after the clean-up) are gone.
    let ids = text_column(&batch, "Order ID");
    let mut seen = HashSet::new();
    for i in 0..ids.len() {
        assert!(!ids.is_null(i), "Order ID is null in row {i}");
        let id = ids.value(i);
        assert!(!id.is_empty() && id == id.trim(), "row {i}: {id:?}");
        assert!(seen.insert(id.to_owned()), "duplicate Order ID {id}");
    }
    // Keys went through fnCleanKey.
    for col in ["Customer ID", "Store ID", "Product ID"] {
        let keys = text_column(&batch, col);
        for i in 0..keys.len() {
            let k = keys.value(i);
            assert!(!k.is_empty(), "{col} row {i}");
            assert_eq!(k, k.trim().to_uppercase(), "{col} row {i}");
        }
    }
    let status = text_column(&batch, "Status");
    for i in 0..status.len() {
        let s = status.value(i);
        assert!(
            ["Completed", "Cancelled", "Pending"].contains(&s),
            "unexpected Status {s:?}"
        );
    }

    // Blank discounts became 0, and the line amount uses them.
    let quantity = typed::<Int64Array>(&batch, "Quantity");
    let price = typed::<Float64Array>(&batch, "Unit Price");
    let discount = typed::<Float64Array>(&batch, "Discount");
    let amount = typed::<Float64Array>(&batch, "Line Amount");
    assert_eq!(discount.null_count(), 0);
    assert!((0..discount.len()).any(|i| discount.value(i) == 0.0));
    for i in 0..amount.len() {
        let expected = quantity.value(i) as f64 * price.value(i) * (1.0 - discount.value(i));
        assert!((amount.value(i) - expected).abs() < 1e-9, "row {i}");
    }
    let order_date = typed::<Date32Array>(&batch, "Order Date");
    assert_eq!(order_date.null_count(), 0);
}

#[test]
fn customers_partition_parses_manager_lists() {
    let Some(engine) = fixture_engine() else {
        return;
    };
    let batch = engine
        .evaluate_partition_to_arrow(&data("customers.m"), &shared(), &columns(&CUSTOMER_COLUMNS))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(batch.num_rows(), expected_rows("Customers"));

    let tier = text_column(&batch, "Loyalty Tier");
    for i in 0..tier.len() {
        assert!(
            ["Bronze", "Silver", "Gold", "Unassigned"].contains(&tier.value(i)),
            "row {i}: {:?}",
            tier.value(i)
        );
    }
    assert!((0..tier.len()).any(|i| tier.value(i) == "Unassigned"));

    let email = text_column(&batch, "Email");
    let domain = text_column(&batch, "Email Domain");
    for i in 0..email.len() {
        let e = email.value(i);
        assert_eq!(e, e.trim().to_lowercase(), "row {i}");
        assert!(
            e.ends_with(&format!("@{}", domain.value(i))),
            "row {i}: {e}"
        );
    }

    // Python-style lists were reduced to "A; B"; empty lists became null.
    let managers = text_column(&batch, "Account Managers");
    let count = typed::<Int64Array>(&batch, "Account Manager Count");
    assert!(managers.null_count() > 0);
    let mut apostrophe = false;
    for i in 0..managers.len() {
        if managers.is_null(i) {
            assert_eq!(count.value(i), 0, "row {i}");
            continue;
        }
        let m = managers.value(i);
        assert!(
            !m.contains(['[', ']', '"']) && !m.starts_with('\'') && !m.ends_with('\''),
            "row {i} not parsed: {m}"
        );
        let names = m.split("; ").count();
        assert_eq!(
            count.value(i),
            i64::try_from(names).unwrap_or(-1),
            "row {i}"
        );
        apostrophe |= m.contains("O'Neil Park");
    }
    assert!(apostrophe, "a name with an apostrophe survives the parsing");
}

#[test]
fn stores_partition_derives_manager_and_region_code() {
    let Some(engine) = fixture_engine() else {
        return;
    };
    let batch = engine
        .evaluate_partition_to_arrow(&data("stores.m"), &shared(), &[])
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(batch.num_rows(), expected_rows("Stores"));
    assert_eq!(column_names(&batch), STORE_COLUMNS);

    let ids = text_column(&batch, "Store ID");
    let region = text_column(&batch, "Region");
    let code = text_column(&batch, "Region Code");
    let email = text_column(&batch, "Manager Email");
    let manager = text_column(&batch, "Store Manager");
    let mut seen = HashSet::new();
    for i in 0..ids.len() {
        assert!(seen.insert(ids.value(i).to_owned()), "duplicate store");
        let r = region.value(i);
        assert_eq!(r, r.trim(), "row {i}");
        assert_eq!(
            code.value(i),
            r.chars().take(3).collect::<String>().to_uppercase()
        );
        let e = email.value(i);
        assert_eq!(e, e.trim().to_lowercase(), "row {i}");
    }
    let row = (0..ids.len())
        .find(|&i| ids.value(i) == "S09")
        .unwrap_or_else(|| panic!("store S09 (trimmed and upper-cased)"));
    assert_eq!(manager.value(row), "Drew Kowalski");
    // Blank cells of the online stores became null.
    assert!(text_column(&batch, "City").null_count() > 0);
    assert!(typed::<Int64Array>(&batch, "Square Feet").null_count() > 0);
}

#[test]
fn missing_and_extra_columns_are_reported() {
    let Some(engine) = fixture_engine() else {
        return;
    };
    let mut expected = utf8(&STORE_COLUMNS[1..]);
    expected.push(("Not There".to_owned(), DataType::Utf8));
    let err = engine
        .evaluate_partition_to_arrow(&data("stores.m"), &shared(), &expected)
        .err()
        .unwrap_or_else(|| panic!("expected a column mismatch"));
    let msg = err.to_string();
    assert!(msg.contains("'Not There'"), "{msg}");
    assert!(msg.contains("'Store ID'"), "{msg}");
}

#[test]
fn last_refreshed_uses_the_injected_clock() {
    let engine = Engine::new(EvalOptions {
        clock: Arc::new(clock()),
        ..EvalOptions::default()
    });
    let expected = [(
        "Last Refreshed (UTC)".to_owned(),
        DataType::Timestamp(TimeUnit::Microsecond, None),
    )];
    let batch = engine
        .evaluate_partition_to_arrow(&data("last_refreshed.m"), &IndexMap::new(), &expected)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(batch.num_rows(), 1);
    let ts = batch
        .column(0)
        .as_any()
        .downcast_ref::<TimestampMicrosecondArray>()
        .unwrap_or_else(|| panic!("timestamp column"));
    assert_eq!(ts.value(0), clock().now.timestamp_micros());

    let v = engine
        .evaluate(&data("last_refreshed.m"))
        .unwrap_or_else(|e| panic!("{e}"));
    let table = v.as_table().unwrap_or_else(|| panic!("table"));
    let cell = table.cell(0, 0).map(|c| c.resolved());
    assert!(
        matches!(cell, Some(Ok(Value::DateTime(dt))) if dt == clock().now.naive_utc()),
        "{cell:?}"
    );
}

#[test]
fn partitions_fail_clearly_without_a_blob_mapping() {
    let err = Engine::default()
        .evaluate_query(&data("orders.m"), &shared())
        .err()
        .unwrap_or_else(|| panic!("expected an offline error"));
    assert!(matches!(err, MError::Resolver(_)), "{err:?}");
    assert!(err.to_string().contains("offline"), "{err}");
}

/// Invokes one of the model's shared M functions on a text literal.
fn call(function: &str, raw: &str) -> Value {
    let literal = format!("\"{}\"", raw.replace('"', "\"\""));
    Engine::default()
        .evaluate_query(&format!("{function}({literal})"), &shared())
        .unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn shared_functions_clean_keys_and_parse_names() {
    assert_eq!(call("fnCleanKey", "  c0042 "), Value::text("C0042"));
    assert_eq!(
        call("fnParseNames", "['Ana Ruiz', 'Jo Kim']"),
        Value::text("Ana Ruiz; Jo Kim")
    );
    // Names with an apostrophe are wrapped in double quotes.
    assert_eq!(
        call("fnParseNames", r#"['Ana Ruiz', "D'Arcy Lane"]"#),
        Value::text("Ana Ruiz; D'Arcy Lane")
    );
    assert_eq!(call("fnParseNames", "[]"), Value::text(""));
    assert_eq!(call("fnParseNames", "   "), Value::text(""));
    assert_eq!(
        call("fnParseNames", "Someone Else"),
        Value::text("Someone Else")
    );
}

const PROMOTIONS_CONTAINER: &str = "https://acmeoutfittersdemo.blob.core.windows.net/sample-data";
const PROMOTIONS_BLOB: &str = "marketing/promotions.csv";
const PROMOTIONS_CSV: &str = "Promo Code,Campaign Status,Sales Channel,List Price,Discount (%)\n\
SPRING10, Running ,retail,100,10\n\
TRAIL25,Running,\\,80,25\n\
,Running,online,50,5\n\
WINTER5,Ended,online,40,5\n\
CAMP15,Running,,200,15\n";

/// Serves the synthetic promotions CSV from memory.
#[derive(Debug)]
struct PromotionsBlob;

impl BlobResolver for PromotionsBlob {
    fn list(&self, container_url: &str) -> Result<Vec<BlobEntry>, ResolverError> {
        if container_url != PROMOTIONS_CONTAINER {
            return Err(ResolverError::new(
                ResolverErrorKind::NotFound,
                format!("unknown container {container_url}"),
            ));
        }
        Ok(vec![BlobEntry {
            name: PROMOTIONS_BLOB.to_owned(),
            size: u64::try_from(PROMOTIONS_CSV.len()).ok(),
            modified: None,
        }])
    }

    fn open(&self, _container_url: &str, name: &str) -> Result<Vec<u8>, ResolverError> {
        if name != PROMOTIONS_BLOB {
            return Err(ResolverError::new(
                ResolverErrorKind::NotFound,
                format!("unknown blob {name}"),
            ));
        }
        Ok(PROMOTIONS_CSV.as_bytes().to_vec())
    }
}

#[test]
fn promotions_query_evaluates_typed_functions_on_synthetic_data() {
    let engine = Engine::new(EvalOptions {
        resolver: Arc::new(PromotionsBlob),
        ..EvalOptions::default()
    });
    let batch = engine
        .evaluate_partition_to_arrow(
            &data("promotions.m"),
            &shared(),
            &[
                ("Promo Code".to_owned(), DataType::Utf8),
                ("Sales Channel".to_owned(), DataType::Utf8),
                ("List Price".to_owned(), DataType::Utf8),
                ("Discount (%)".to_owned(), DataType::Utf8),
                ("Sale Price".to_owned(), DataType::Float64),
            ],
        )
        .unwrap_or_else(|e| panic!("{e}"));
    // The ended campaign and the row without a code are dropped.
    let codes = text_column(&batch, "Promo Code");
    let codes: Vec<&str> = (0..codes.len()).map(|i| codes.value(i)).collect();
    assert_eq!(codes, ["SPRING10", "TRAIL25", "CAMP15"]);
    // "\" and blank channels become null; the others are proper-cased.
    let channel = text_column(&batch, "Sales Channel");
    let channel: Vec<Option<&str>> = (0..channel.len())
        .map(|i| (!channel.is_null(i)).then(|| channel.value(i)))
        .collect();
    assert_eq!(channel, [Some("Retail"), None, None]);
    let price = typed::<Float64Array>(&batch, "Sale Price");
    let price: Vec<f64> = (0..price.len()).map(|i| price.value(i)).collect();
    assert_eq!(price, [90.0, 60.0, 170.0]);
}
