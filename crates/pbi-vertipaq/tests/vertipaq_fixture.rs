//! VertiPaq column decoding on the `DataModel` of the Microsoft sample PBIX
//! files (`reference/samples/microsoft`, MIT), compared with pbixray.
//!
//! Every test prints `SKIP: ...` and returns when a sample is absent. Only
//! counts, types and SHA-256 fingerprints are asserted. The expected values
//! are the outputs of `tools/reference_hashes.py` (pbixray 0.15.5, run
//! read-only) in `tests/data/reference_hashes/`; `fingerprint` below
//! implements the same canonical serialization.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::{BTreeMap, HashSet};
use std::time::Instant;

use arrow::array::{
    Array, AsArray, BooleanArray, Decimal128Array, Float64Array, Int64Array, StringArray,
    TimestampMicrosecondArray,
};
use arrow::datatypes::{DataType as ArrowType, TimeUnit};
use common::{ALL_SAMPLES, REGIONAL_SALES, REVENUE_OPPORTUNITIES, datamodel, fingerprints};
use pbi_vertipaq::{
    DecodeLimits, Limits, ModelOptions, VertipaqError, decode_all, decode_column, decode_table,
    load_model, open_pbix_data, open_pbix_datamodel,
};
use sha2::{Digest, Sha256};

/// `(nulls, distinct, sha256)` of the canonical serialization (see
/// `tools/reference_hashes.py`).
fn fingerprint(array: &dyn Array) -> (usize, usize, String) {
    let mut digest = Sha256::new();
    let mut distinct = HashSet::new();
    let mut nulls = 0;
    for row in 0..array.len() {
        if array.is_null(row) {
            nulls += 1;
            digest.update(b"N\n");
            continue;
        }
        let text = canonical_text(array, row);
        digest.update(format!("V{}:", text.len()).as_bytes());
        digest.update(text.as_bytes());
        digest.update(b"\n");
        distinct.insert(text);
    }
    let hash = digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    (nulls, distinct.len(), hash)
}

fn canonical_text(array: &dyn Array, row: usize) -> String {
    match array.data_type() {
        ArrowType::Utf8 => array
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(row)
            .to_owned(),
        ArrowType::Int64 => array
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .value(row)
            .to_string(),
        ArrowType::Float64 => format!(
            "{:016x}",
            array
                .as_any()
                .downcast_ref::<Float64Array>()
                .unwrap()
                .value(row)
                .to_bits()
        ),
        ArrowType::Timestamp(TimeUnit::Microsecond, None) => array
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
            .unwrap()
            .value(row)
            .to_string(),
        ArrowType::Decimal128(_, _) => array
            .as_any()
            .downcast_ref::<Decimal128Array>()
            .unwrap()
            .value(row)
            .to_string(),
        ArrowType::Boolean => array
            .as_any()
            .downcast_ref::<BooleanArray>()
            .unwrap()
            .value(row)
            .to_string(),
        other => panic!("unexpected Arrow type {other}"),
    }
}

fn expected_arrow_type(tom_type: i64) -> ArrowType {
    match tom_type {
        2 => ArrowType::Utf8,
        6 => ArrowType::Int64,
        8 => ArrowType::Float64,
        9 => ArrowType::Timestamp(TimeUnit::Microsecond, None),
        10 => ArrowType::Decimal128(19, 4),
        11 => ArrowType::Boolean,
        other => panic!("no expectation for TOM type {other}"),
    }
}

#[test]
fn fixture_every_column_matches_pbixray() {
    let mut checked = BTreeMap::new();
    for sample in ALL_SAMPLES {
        let Some(bytes) = datamodel(sample) else {
            continue;
        };
        let reference = fingerprints(sample);
        let started = Instant::now();
        let (model, tables, diagnostics) =
            open_pbix_data(&bytes, &DecodeLimits::default()).unwrap();
        eprintln!("{}: open_pbix_data {:?}", sample.file, started.elapsed());
        assert!(diagnostics.is_empty(), "{}: {diagnostics:#?}", sample.file);

        let decoded: BTreeMap<&str, _> = tables.iter().map(|(n, b)| (n.as_str(), b)).collect();
        let expected: BTreeMap<&str, usize> = reference
            .tables
            .iter()
            .map(|(n, r)| (n.as_str(), *r))
            .collect();
        assert_eq!(
            decoded.keys().copied().collect::<Vec<_>>(),
            expected.keys().copied().collect::<Vec<_>>(),
            "{}",
            sample.file
        );
        for (name, rows) in &expected {
            assert_eq!(decoded[name].num_rows(), *rows, "rows of {name}");
        }

        for (table, column, tom_type, nulls, distinct, sha) in &reference.columns {
            let batch = decoded[table.as_str()];
            let array = batch
                .column_by_name(column)
                .unwrap_or_else(|| panic!("{table}[{column}] missing"));
            assert_eq!(
                array.data_type(),
                &expected_arrow_type(*tom_type),
                "type of {table}[{column}]"
            );
            let actual = fingerprint(array.as_ref());
            assert_eq!(
                actual,
                (*nulls, *distinct, sha.clone()),
                "fingerprint of {table}[{column}]"
            );
            *checked.entry(sample.id).or_insert(0) += 1;
        }

        // Batches have exactly the model's columns, in model order.
        for (name, batch) in &tables {
            let model_columns: Vec<&str> = model
                .table(name)
                .unwrap()
                .columns
                .iter()
                .map(|c| c.name.as_str())
                .collect();
            let batch_columns: Vec<&str> = batch
                .schema_ref()
                .fields()
                .iter()
                .map(|f| f.name().as_str())
                .collect();
            assert_eq!(batch_columns, model_columns, "columns of {name}");
        }
    }
    assert_eq!(checked.get("revenue-opportunities"), Some(&69));
    assert_eq!(checked.get("corporate-spend"), Some(&36));
    assert_eq!(checked.get("regional-sales"), Some(&70));
    if let Some(n) = checked.get("adventureworks-sales") {
        assert_eq!(*n, 62);
    }
}

#[test]
fn fixture_strict_apis() {
    let Some(bytes) = datamodel(REVENUE_OPPORTUNITIES) else {
        return;
    };
    let archive = open_pbix_datamodel(&bytes, &Limits::default()).unwrap();
    let loaded = load_model(&archive, &ModelOptions::default()).unwrap();
    let limits = DecodeLimits::default();

    let all = decode_all(&archive, &loaded.storage, &loaded.model, &limits).unwrap();
    assert_eq!(all.len(), 9);

    let opportunity = decode_table(
        &archive,
        &loaded.storage,
        &loaded.model,
        "opportunity",
        &limits,
    )
    .unwrap();
    assert_eq!(opportunity.num_rows(), 486);
    assert_eq!(opportunity.num_columns(), 5);

    let column = decode_column(&archive, &loaded.storage, "Account", "Account", &limits).unwrap();
    assert_eq!(column.len(), 357);
    assert_eq!(column.null_count(), 0);
    assert_eq!(column.as_string::<i32>().len(), 357);

    // The calculated table's stored rows decode like any other table.
    let calendar = decode_table(
        &archive,
        &loaded.storage,
        &loaded.model,
        "Calendar",
        &limits,
    )
    .unwrap();
    assert_eq!((calendar.num_rows(), calendar.num_columns()), (1096, 11));

    let err = decode_column(&archive, &loaded.storage, "Account", "Nope", &limits).unwrap_err();
    assert!(matches!(err, VertipaqError::ColumnNotFound { .. }), "{err}");
    let err = decode_table(&archive, &loaded.storage, &loaded.model, "Nope", &limits).unwrap_err();
    assert!(matches!(err, VertipaqError::TableNotFound(_)), "{err}");

    // Tight limits fail cleanly.
    let tiny = DecodeLimits::default().with_max_rows(100);
    let err = decode_column(&archive, &loaded.storage, "Account", "Region", &tiny).unwrap_err();
    assert!(
        matches!(err.innermost(), VertipaqError::LimitExceeded { .. }),
        "{err}"
    );
}

#[test]
fn fixture_all_blank_column_decodes_to_nulls() {
    let Some(bytes) = datamodel(REGIONAL_SALES) else {
        return;
    };
    let archive = open_pbix_datamodel(&bytes, &Limits::default()).unwrap();
    let loaded = load_model(&archive, &ModelOptions::default()).unwrap();
    let blank = decode_column(
        &archive,
        &loaded.storage,
        "Opportunities",
        "Blank",
        &DecodeLimits::default(),
    )
    .unwrap();
    assert_eq!((blank.len(), blank.null_count()), (20_000, 20_000));
}

#[test]
fn fixture_internal_files_parse() {
    let Some(bytes) = datamodel(REVENUE_OPPORTUNITIES) else {
        return;
    };
    let archive = open_pbix_datamodel(&bytes, &Limits::default()).unwrap();
    let loaded = load_model(&archive, &ModelOptions::default()).unwrap();
    let mut segments = 0;
    let mut dictionaries = 0;
    for table in loaded.storage.tables.iter().filter(|t| !t.is_internal) {
        for column in table.columns.iter().filter(|c| c.has_values()) {
            for partition in &column.partitions {
                let meta = pbi_vertipaq::vertipaq::parse_idfmeta(
                    archive.read(partition.idfmeta.as_deref().unwrap()).unwrap(),
                )
                .unwrap();
                assert_eq!(meta.records(), partition.record_count);
                for segment in &meta.segments {
                    assert!(matches!(
                        segment.encoding(),
                        pbi_vertipaq::vertipaq::SegmentEncoding::HybridRle { bit_width: 1..=32 }
                    ));
                    segments += 1;
                }
            }
            if let Some(path) = column.dictionary_path() {
                let dictionary = pbi_vertipaq::vertipaq::parse_dictionary(
                    archive.read(path).unwrap(),
                    &DecodeLimits::default(),
                )
                .unwrap();
                let distinct = column.statistics.distinct_states.unwrap();
                // Statistics count BLANK as a state when the column has nulls.
                let blank = i64::from(column.statistics.has_nulls == Some(true));
                assert!(
                    dictionary.len() as i64 == distinct
                        || dictionary.len() as i64 == distinct - blank,
                    "{}[{}]: {} entries, {distinct} states",
                    table.name,
                    column.name,
                    dictionary.len()
                );
                dictionaries += 1;
            }
        }
    }
    assert_eq!(segments, 69);
    assert_eq!(dictionaries, 36);
}
