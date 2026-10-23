#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Exports: CSV and Parquet writers on synthetic batches, and the model doc
//! (JSON / Markdown) of the synthetic Acme Outfitters example.
//!
//! Files are written under `target/tmp` (`CARGO_TARGET_TMPDIR`) and
//! deleted. The example's model definition is committed and synthetic, so
//! its model-doc snapshots are fine to commit.

use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

use arrow::array::{
    Array, ArrayRef, AsArray, BooleanArray, Decimal128Array, Float64Array, Int64Array, RecordBatch,
    StringArray, TimestampMicrosecondArray,
};
use arrow::datatypes::{DataType as ArrowType, Field, Schema, TimeUnit};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::basic::Compression;
use pbi_model::{Column, DataType, Model, Table};
use pbi_session::export::meta;
use pbi_session::{
    CsvExportOptions, MarkdownOptions, ModelDocOptions, ParquetCompression, ParquetExportOptions,
    annotate_batch, model_doc, model_to_json, model_to_markdown, write_csv, write_parquet,
};
use pretty_assertions::assert_eq;
use serde_json::Value as Json;

// ---------------------------------------------------------------------------
// Synthetic data
// ---------------------------------------------------------------------------

/// 2026-10-21 14:05:00 in microseconds since 1970-01-01.
const TS: i64 = 1_792_591_500_000_000;

fn sample_batch() -> RecordBatch {
    let schema = Schema::new(vec![
        Field::new("Name", ArrowType::Utf8, true),
        Field::new("Count", ArrowType::Int64, true),
        Field::new("Share", ArrowType::Float64, true),
        Field::new("Amount", ArrowType::Decimal128(19, 4), true),
        Field::new(
            "Seen",
            ArrowType::Timestamp(TimeUnit::Microsecond, None),
            true,
        ),
        Field::new("Active", ArrowType::Boolean, true),
    ]);
    let columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(vec![
            Some("plain"),
            Some("comma, \"quoted\""),
            Some("two\nlines"),
            None,
            Some("Zürich ☃"),
        ])),
        Arc::new(Int64Array::from(vec![
            Some(412),
            Some(-3),
            None,
            Some(0),
            Some(1_234_567),
        ])),
        Arc::new(Float64Array::from(vec![
            Some(0.453),
            Some(0.5),
            Some(1.0),
            None,
            Some(-2.25),
        ])),
        Arc::new(
            Decimal128Array::from(vec![
                Some(12_500),
                Some(-5_000),
                Some(0),
                Some(10_000_000),
                None,
            ])
            .with_precision_and_scale(19, 4)
            .unwrap(),
        ),
        Arc::new(TimestampMicrosecondArray::from(vec![
            Some(TS),
            None,
            Some(TS + 1_500_000),
            Some(0),
            Some(TS),
        ])),
        Arc::new(BooleanArray::from(vec![
            Some(true),
            Some(false),
            None,
            Some(true),
            Some(false),
        ])),
    ];
    RecordBatch::try_new(Arc::new(schema), columns).unwrap()
}

fn sample_table() -> Table {
    let mut t = Table::new("Sample");
    t.description = Some("Synthetic rows for export tests.".into());
    let mut columns = vec![
        Column::data("Name", DataType::String),
        Column::data("Count", DataType::Int64),
        Column::data("Share", DataType::Double),
        Column::data("Amount", DataType::Decimal),
        Column::data("Seen", DataType::DateTime),
        Column::calculated("Active", DataType::Boolean, "[Count] > 0"),
    ];
    columns[0].description = Some("Display name.".into());
    columns[1].format_string = Some("#,0".into());
    columns[2].format_string = Some("0.0%".into());
    columns[3].format_string = Some("0.00".into());
    columns[4].format_string = Some("yyyy-mm-dd hh:nn".into());
    t.columns = columns;
    t
}

fn csv_text(batch: &RecordBatch, options: &CsvExportOptions) -> String {
    let mut out = Vec::new();
    let summary = write_csv(batch, &mut out, options).unwrap();
    assert_eq!(summary.rows, batch.num_rows());
    assert_eq!(summary.columns, batch.num_columns());
    String::from_utf8(out).unwrap()
}

/// A fresh, empty directory under `target/tmp` for one test.
fn tmp_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("pbi-session-export-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ---------------------------------------------------------------------------
// CSV
// ---------------------------------------------------------------------------

#[test]
fn csv_raw_values_are_quoted_only_when_needed() {
    let text = csv_text(&sample_batch(), &CsvExportOptions::default());
    assert_eq!(
        text,
        "Name,Count,Share,Amount,Seen,Active\n\
         plain,412,0.453,1.2500,2026-10-21T14:05:00,true\n\
         \"comma, \"\"quoted\"\"\",-3,0.5,-0.5000,,false\n\
         \"two\nlines\",,1.0,0.0000,2026-10-21T14:05:01.500,\n\
         ,0,,1000.0000,1970-01-01T00:00:00,true\n\
         Zürich ☃,1234567,-2.25,,2026-10-21T14:05:00,false\n"
    );
}

#[test]
fn csv_bom_delimiter_and_no_header() {
    let options = CsvExportOptions {
        delimiter: ';',
        header: false,
        bom: true,
        ..CsvExportOptions::default()
    };
    let batch = sample_batch().slice(0, 2);
    let bytes = {
        let mut out = Vec::new();
        write_csv(&batch, &mut out, &options).unwrap();
        out
    };
    assert_eq!(&bytes[..3], b"\xEF\xBB\xBF");
    assert_eq!(
        String::from_utf8(bytes[3..].to_vec()).unwrap(),
        "plain;412;0.453;1.2500;2026-10-21T14:05:00;true\n\
         comma, \"\"\"quoted\"\"\";-3;0.5;-0.5000;;false\n"
            .replace("comma, \"\"\"quoted\"\"\"", "\"comma, \"\"quoted\"\"\"")
    );
    for bad in ['"', '\n', 'é'] {
        let options = CsvExportOptions {
            delimiter: bad,
            ..CsvExportOptions::default()
        };
        assert!(
            write_csv(&batch, &mut Vec::new(), &options).is_err(),
            "{bad:?}"
        );
    }
}

#[test]
fn csv_escape_formulas_guards_text_cells_only() {
    let schema = Schema::new(vec![
        Field::new("=Name", ArrowType::Utf8, true),
        Field::new("Count", ArrowType::Int64, true),
        Field::new("Amount", ArrowType::Decimal128(19, 4), true),
    ]);
    let columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(vec![
            Some("=HYPERLINK(\"http://x\",\"y\")"),
            Some("+1"),
            Some("-2"),
            Some("@SUM(A1:A2)"),
            Some("\tTab"),
            Some("\rCR"),
            Some("plain = text"),
            Some(" =spaced"),
            None,
        ])),
        Arc::new(Int64Array::from(vec![
            Some(-3),
            Some(1),
            None,
            Some(0),
            Some(-1),
            Some(2),
            Some(3),
            Some(4),
            Some(-5),
        ])),
        Arc::new(
            Decimal128Array::from(vec![Some(-5_000); 9])
                .with_precision_and_scale(19, 4)
                .unwrap(),
        ),
    ];
    let batch = RecordBatch::try_new(Arc::new(schema), columns).unwrap();
    let safe = CsvExportOptions {
        escape_formulas: true,
        ..CsvExportOptions::default()
    };
    assert_eq!(
        csv_text(&batch, &safe),
        "'=Name,Count,Amount\n\
         \"'=HYPERLINK(\"\"http://x\"\",\"\"y\"\")\",-3,-0.5000\n\
         '+1,1,-0.5000\n\
         '-2,,-0.5000\n\
         '@SUM(A1:A2),0,-0.5000\n\
         '\tTab,-1,-0.5000\n\
         \"'\rCR\",2,-0.5000\n\
         plain = text,3,-0.5000\n \
         =spaced,4,-0.5000\n\
         ,-5,-0.5000\n"
    );
    // Off by default: the data is written as stored.
    let raw = csv_text(&batch, &CsvExportOptions::default());
    assert!(
        raw.starts_with("=Name,Count,Amount\n\"=HYPERLINK("),
        "{raw}"
    );
    assert!(raw.contains("\n-2,,-0.5000\n"), "{raw}");
    // Formatted negative numbers are numbers, not formulas.
    let formatted = CsvExportOptions {
        formatted: true,
        ..safe
    };
    let text = csv_text(&batch, &formatted);
    assert!(text.contains("\n'+1,1,-0.5\n"), "{text}");
}

#[test]
fn csv_formatted_uses_model_format_strings() {
    let batch = annotate_batch(&sample_batch(), &sample_table(), "en-US").unwrap();
    let options = CsvExportOptions {
        formatted: true,
        ..CsvExportOptions::default()
    };
    let text = csv_text(&batch, &options);
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("Name,Count,Share,Amount,Seen,Active"));
    assert_eq!(
        lines.next(),
        Some("plain,412,45.3%,1.25,2026-10-21 14:05,True")
    );
    // Thousands separators force quoting; BLANK stays empty in every type.
    assert!(text.contains("\"1,234,567\""), "{text}");
    assert!(
        text.contains("\n,0,,\"1,000.00\"") || text.contains("\n,0,,1000.00"),
        "{text}"
    );
    // Raw output of the same annotated batch is unchanged by the metadata.
    assert_eq!(
        csv_text(&batch, &CsvExportOptions::default()),
        csv_text(&sample_batch(), &CsvExportOptions::default())
    );
}

// ---------------------------------------------------------------------------
// Parquet
// ---------------------------------------------------------------------------

#[test]
fn parquet_round_trips_values_and_metadata() {
    let dir = tmp_dir("parquet");
    let batch = annotate_batch(&sample_batch(), &sample_table(), "en-US").unwrap();
    for (compression, codec) in [
        (ParquetCompression::Zstd, "ZSTD"),
        (ParquetCompression::Snappy, "SNAPPY"),
        (ParquetCompression::None, "UNCOMPRESSED"),
    ] {
        let path = dir.join(format!("sample-{codec}.parquet"));
        let summary = write_parquet(
            &batch,
            File::create(&path).unwrap(),
            &ParquetExportOptions { compression },
        )
        .unwrap();
        assert_eq!((summary.rows, summary.columns), (5, 6));

        let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(&path).unwrap()).unwrap();
        let file_meta = builder.metadata().file_metadata().clone();
        assert_eq!(file_meta.num_rows(), 5);
        let column_codec = builder.metadata().row_group(0).column(0).compression();
        assert!(
            format!("{column_codec:?}").starts_with(codec),
            "{column_codec:?} for {compression:?}"
        );
        if compression == ParquetCompression::None {
            assert_eq!(column_codec, Compression::UNCOMPRESSED);
        }

        // Key-value metadata.
        let kv: HashMap<String, String> = file_meta
            .key_value_metadata()
            .unwrap()
            .iter()
            .filter_map(|kv| Some((kv.key.clone(), kv.value.clone()?)))
            .collect();
        assert_eq!(kv.get(meta::SCHEMA_VERSION).map(String::as_str), Some("1"));
        assert_eq!(kv.get(meta::TABLE).map(String::as_str), Some("Sample"));
        assert_eq!(
            kv.get(meta::TABLE_DESCRIPTION).map(String::as_str),
            Some("Synthetic rows for export tests.")
        );
        assert_eq!(kv.get(meta::CULTURE).map(String::as_str), Some("en-US"));
        let columns: Json = serde_json::from_str(&kv[meta::COLUMNS]).unwrap();
        assert_eq!(
            columns[0],
            serde_json::json!({
                "name": "Name", "dataType": "string", "kind": "data",
                "description": "Display name.", "sourceColumn": "Name"
            })
        );
        assert_eq!(columns[2]["formatString"], "0.0%");
        assert_eq!(columns[5]["kind"], "calculated");
        assert_eq!(columns.as_array().map(Vec::len), Some(6));

        // Arrow schema (field metadata included) and values.
        let schema = builder.schema().clone();
        let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(
            names,
            ["Name", "Count", "Share", "Amount", "Seen", "Active"]
        );
        assert_eq!(
            schema
                .field(0)
                .metadata()
                .get(meta::DESCRIPTION)
                .map(String::as_str),
            Some("Display name.")
        );
        assert_eq!(
            schema
                .field(4)
                .metadata()
                .get(meta::DATA_TYPE)
                .map(String::as_str),
            Some("dateTime")
        );
        let read: Vec<RecordBatch> = builder.build().unwrap().collect::<Result<_, _>>().unwrap();
        assert_eq!(read.iter().map(RecordBatch::num_rows).sum::<usize>(), 5);
        let read = &read[0];
        for (i, field) in batch.schema().fields().iter().enumerate() {
            assert_eq!(
                read.column(i).data_type(),
                field.data_type(),
                "{}",
                field.name()
            );
            assert_eq!(
                read.column(i).to_data(),
                batch.column(i).to_data(),
                "{}",
                field.name()
            );
        }
        assert_eq!(read.column(0).as_string::<i32>().value(2), "two\nlines");
        assert!(read.column(1).is_null(2));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

// ---------------------------------------------------------------------------
// Model doc (the Acme Outfitters example)
// ---------------------------------------------------------------------------

fn reference_root() -> PathBuf {
    std::env::var_os("PBIVIEW_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
}

fn fixture_model() -> Option<Model> {
    let path = reference_root().join("examples/acme-outfitters/AcmeOutfitters.pbip");
    if !path.exists() {
        println!("SKIP: {} missing", path.display());
        return None;
    }
    let opened = pbi_core::open_path(&path, &pbi_core::OpenOptions::default()).unwrap();
    Some(opened.document.model.expect("the PBIP has a model"))
}

fn markdown(model: &Model, include_expressions: bool, include_hidden: bool) -> String {
    model_to_markdown(
        model,
        &MarkdownOptions {
            include_expressions,
            include_hidden,
            title: Some("AcmeOutfitters".into()),
        },
    )
}

#[test]
fn model_markdown_is_a_data_dictionary() {
    let Some(model) = fixture_model() else {
        return;
    };
    let md = markdown(&model, true, true);
    for needle in [
        "# AcmeOutfitters\n",
        "| Tables | 11 |",
        "| Measures | 33 |",
        "| Calculated tables | 2 |",
        "| Roles | 2 |",
        // Relationship rows: both directions, inactive, one-to-one, many-to-many.
        "| Orders[Customer ID] → Customers[Customer ID] | many-to-one | both directions | yes |",
        "| Orders[Ship Date] → Date[Date] | many-to-one | single | no |",
        "| Returns[Order ID] → Orders[Order ID] | one-to-one | both directions | yes |",
        "| Targets[Region] → Stores[Region] | many-to-many | single | yes |",
        // Kinds.
        "| Time Calc | calculation group | 2 | 0 |",
        "| Last Refreshed | table (hidden) | 1 | 0 |",
        // `///` descriptions: table, column, measure.
        "### Orders\n\nOne row per order line (synthetic).",
        "| Ship Date | dateTime, format yyyy-mm-dd | Blank for cancelled and pending orders. Inactive relationship to 'Date' (USERELATIONSHIP). | Ship Date |",
        "| Customer ID (hidden) | string |  | Customer ID |",
        "| Total Sales | `\\$#,0.00` | Sales | Sum of the order line amounts. |",
        // Measures, calculated columns and tables as DAX blocks.
        "##### Margin %\n\n```dax\nDIVIDE([Gross Margin], [Total Sales])\n```",
        "##### Lifetime Sales\n",
        "| Lifetime Sales | double, format \\\\$#,0 | Context transition: every order of the customer, all statuses. | calculated (DAX) |",
        "Calculated table (DAX) partition `Products Never Sold`, import mode.\n\n```dax\nVAR SoldProducts =",
        // M partitions.
        "Power Query (M) partition `Orders`, import mode, query group `Sources`.\n\n```powerquery\nlet\n",
        // Escaping: the backslash of `\$` is kept, `>` is escaped in table cells.
        "| Unit Price | double, format \\\\$#,0.00 |  | Unit Price |",
        "Product catalog: Category \\> Subcategory \\> Product.",
        // Parameters, shared expressions and roles come last.
        "## Parameters",
        "| StorageContainerUrl | Text | yes | `\"https://acmeoutfittersdemo.blob.core.windows.net/sample-data\"` |",
        "## Shared expressions\n\n### fnCleanKey\n",
        "### Store Manager\n",
        "Row filter on `Stores`:\n\n```dax\nStores[Manager Email] = USERPRINCIPALNAME()\n```",
    ] {
        assert!(md.contains(needle), "missing {needle:?} in:\n{md}");
    }
    assert!(md.find("## Relationships") < md.find("## Tables"));
    assert!(md.find("## Tables") < md.find("## Parameters"));
    assert!(md.find("## Parameters") < md.find("## Roles"));
    assert!(
        md.ends_with("```\n") || md.ends_with("|\n"),
        "one final newline"
    );
    assert!(!md.contains("\n\n\n"), "no double blank lines");
    assert_eq!(md, markdown(&model, true, true), "deterministic");
    insta::assert_snapshot!("model_markdown", md);
}

#[test]
fn model_markdown_can_leave_out_expressions_and_hidden_objects() {
    let Some(model) = fixture_model() else {
        return;
    };
    let outline = markdown(&model, false, true);
    assert!(!outline.contains("```"), "{outline}");
    assert!(!outline.contains("DISTINCTCOUNTNOBLANK"));
    assert!(!outline.contains("USERPRINCIPALNAME"));
    assert!(
        !outline.contains("https://"),
        "parameter values are expressions"
    );
    assert!(outline.contains("DAX and M expressions are omitted."));
    assert!(
        outline.contains("| Total Sales | `\\$#,0.00` | Sales | Sum of the order line amounts. |")
    );
    assert!(
        outline.contains("Power Query (M) partition `Orders`, import mode, query group `Sources`.")
    );
    assert!(
        outline.contains("| StorageContainerUrl | Text | yes | Fictional Azure Blob container")
    );

    let visible = markdown(&model, true, false);
    assert!(visible.contains("Hidden objects are omitted."));
    assert!(!visible.contains("\n### Last Refreshed\n"), "hidden table");
    assert!(markdown(&model, true, true).contains("\n### Last Refreshed\n"));
    assert!(!visible.contains("Customer ID (hidden)"), "hidden column");
    assert!(!visible.contains("| Sales All Stores |"), "hidden measure");
    assert!(visible.contains("| Tables | 10 |"));
    assert!(visible.contains("| Orders[Customer ID] → Customers[Customer ID] |"));
}

/// Every key of `value`, recursively.
fn keys(value: &Json, out: &mut Vec<String>) {
    match value {
        Json::Object(map) => {
            for (k, v) in map {
                out.push(k.clone());
                keys(v, out);
            }
        }
        Json::Array(items) => items.iter().for_each(|v| keys(v, out)),
        _ => {}
    }
}

#[test]
fn model_json_doc_has_a_stable_schema() {
    let Some(model) = fixture_model() else {
        return;
    };
    let json = model_to_json(&model);
    assert_eq!(json["schemaVersion"], 1);
    assert_eq!(json["includesExpressions"], true);
    assert_eq!(
        json["summary"],
        serde_json::json!({
            "tables": 11, "columns": 81, "calculatedColumns": 11, "calculatedTables": 2,
            "measures": 33, "relationships": 8, "parameters": 1, "expressions": 2, "roles": 2
        })
    );
    let orders = &json["tables"][0];
    assert_eq!(orders["name"], "Orders");
    assert!(
        orders["description"]
            .as_str()
            .is_some_and(|d| d.starts_with("One row per order line (synthetic)."))
    );
    assert_eq!(
        orders["columns"][0],
        serde_json::json!({
            "name": "Order ID", "dataType": "string", "kind": "data", "hidden": false, "key": true,
            "description": "Order number (SO-…). Unique after Table.Distinct.",
            "sourceColumn": "Order ID", "summarizeBy": "none"
        })
    );
    assert_eq!(orders["columns"][3]["hidden"], true);
    let measure = &orders["measures"][3];
    assert_eq!(measure["name"], "Margin %");
    assert_eq!(
        measure["expression"],
        "DIVIDE([Gross Margin], [Total Sales])"
    );
    assert_eq!(measure["formatString"], "0.0%");
    assert_eq!(measure["displayFolder"], "Sales");
    assert_eq!(orders["partitions"][0]["language"], "m");
    assert_eq!(orders["partitions"][0]["queryGroup"], "Sources");
    let rel = &json["relationships"][0];
    assert_eq!(
        rel["from"],
        serde_json::json!({"table": "Orders", "column": "Customer ID"})
    );
    assert_eq!(
        rel["to"],
        serde_json::json!({"table": "Customers", "column": "Customer ID"})
    );
    assert_eq!(rel["crossFilteringBehavior"], "bothDirections");
    assert_eq!(json["parameters"][0]["name"], "StorageContainerUrl");
    assert_eq!(json["parameters"][0]["type"], "Text");
    assert_eq!(
        json["roles"][1]["tablePermissions"][0]["filterExpression"],
        "Stores[Manager Email] = USERPRINCIPALNAME()"
    );

    // The typed doc round-trips through its JSON.
    let typed: pbi_session::ModelDoc = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(typed, model_doc(&model, &ModelDocOptions::default()));

    let outline = serde_json::to_value(model_doc(
        &model,
        &ModelDocOptions {
            include_expressions: false,
            include_hidden: true,
        },
    ))
    .unwrap();
    let mut all = Vec::new();
    keys(&outline, &mut all);
    for gone in ["expression", "value", "filterExpression"] {
        assert!(
            !all.iter().any(|k| k == gone),
            "{gone} with include_expressions = false"
        );
    }
    assert_eq!(outline["includesExpressions"], false);
    insta::assert_snapshot!("model_json", serde_json::to_string_pretty(&json).unwrap());
}
