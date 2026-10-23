//! Unit tests for the metadata reader on synthetic `metadata.sqlitedb` images.

use pbi_model::{
    Cardinality, ColumnKind, CrossFilter, DataType, ExpressionKind, PartitionMode, PartitionSource,
    PropValue, SummarizeBy, TableKind,
};
use rusqlite::Connection;

use super::*;
use crate::abf::test_support::AbfBuilder;

const SCHEMA: &str = r#"
CREATE TABLE DBPROPERTIES (NAME STRING, VALUE INTEGER);
CREATE TABLE Model (ID INTEGER, Name TEXT, Culture TEXT, DefaultPowerBIDataSourceVersion INTEGER,
    DataAccessOptions TEXT, DiscourageImplicitMeasures INTEGER, DefaultMode INTEGER);
CREATE TABLE [Table] (ID INTEGER, Name TEXT, IsHidden INTEGER, SystemFlags INTEGER,
    ShowAsVariationsOnly INTEGER, IsPrivate INTEGER, LineageTag TEXT, Description TEXT,
    TableStorageID INTEGER, CalculationGroupID INTEGER);
CREATE TABLE [Column] (ID INTEGER, TableID INTEGER, ExplicitName TEXT, InferredName TEXT,
    ExplicitDataType INTEGER, InferredDataType INTEGER, Type INTEGER, SourceColumn TEXT,
    Expression TEXT, IsHidden INTEGER, SummarizeBy INTEGER, FormatString TEXT,
    SortByColumnID INTEGER, IsAvailableInMDX INTEGER, ColumnStorageID INTEGER,
    AttributeHierarchyID INTEGER, DisplayFolder TEXT, EncodingHint INTEGER);
CREATE TABLE Measure (ID INTEGER, TableID INTEGER, Name TEXT, Expression TEXT, FormatString TEXT,
    IsHidden INTEGER, DisplayFolder TEXT, FormatStringDefinitionID INTEGER);
CREATE TABLE [Partition] (ID INTEGER, TableID INTEGER, Name TEXT, Type INTEGER, Mode INTEGER,
    QueryDefinition TEXT, QueryGroupID INTEGER, PartitionStorageID INTEGER, DataView INTEGER);
CREATE TABLE Relationship (ID INTEGER, Name TEXT, IsActive INTEGER, CrossFilteringBehavior INTEGER,
    FromTableID INTEGER, FromColumnID INTEGER, FromCardinality INTEGER, ToTableID INTEGER,
    ToColumnID INTEGER, ToCardinality INTEGER, JoinOnDateBehavior INTEGER);
CREATE TABLE Expression (ID INTEGER, Name TEXT, Kind INTEGER, Expression TEXT, QueryGroupID INTEGER,
    Description TEXT);
CREATE TABLE QueryGroup (ID INTEGER, Folder TEXT);
CREATE TABLE Annotation (ID INTEGER, ObjectID INTEGER, ObjectType INTEGER, Name TEXT, Value TEXT);
CREATE TABLE Hierarchy (ID INTEGER, TableID INTEGER, Name TEXT);
CREATE TABLE Level (ID INTEGER, HierarchyID INTEGER, Ordinal INTEGER, Name TEXT, ColumnID INTEGER);
CREATE TABLE Role (ID INTEGER, Name TEXT, ModelPermission INTEGER);
CREATE TABLE TablePermission (ID INTEGER, RoleID INTEGER, TableID INTEGER, FilterExpression TEXT);
CREATE TABLE CalculationGroup (ID INTEGER, TableID INTEGER, Precedence INTEGER);
CREATE TABLE CalculationItem (ID INTEGER, CalculationGroupID INTEGER, Name TEXT, Expression TEXT,
    Ordinal INTEGER);
CREATE TABLE FormatStringDefinition (ID INTEGER, Expression TEXT);
CREATE TABLE TableStorage (ID INTEGER, TableID INTEGER, Name TEXT, StorageFolderID INTEGER);
CREATE TABLE PartitionStorage (ID INTEGER, PartitionID INTEGER, StoragePosition INTEGER,
    SegmentMapStorageID INTEGER, StorageFolderID INTEGER);
CREATE TABLE SegmentMapStorage (ID INTEGER, PartitionStorageID INTEGER, Type INTEGER,
    RecordCount INTEGER, SegmentCount INTEGER, RecordsPerSegment INTEGER);
CREATE TABLE ColumnStorage (ID INTEGER, ColumnID INTEGER, Name TEXT, StoragePosition INTEGER,
    DictionaryStorageID INTEGER, Statistics_DistinctStates INTEGER, Statistics_RowCount INTEGER);
CREATE TABLE ColumnPartitionStorage (ID INTEGER, ColumnStorageID INTEGER,
    PartitionStorageID INTEGER, SegmentStorageID INTEGER, StorageFileID INTEGER);
CREATE TABLE SegmentStorage (ID INTEGER, ColumnPartitionStorageID INTEGER, SegmentCount INTEGER,
    StorageFileID INTEGER);
CREATE TABLE DictionaryStorage (ID INTEGER, ColumnStorageID INTEGER, Type INTEGER, DataType INTEGER,
    BaseId INTEGER, Magnitude REAL, IsOperatingOn32 INTEGER, StorageFileID INTEGER);
CREATE TABLE AttributeHierarchy (ID INTEGER, ColumnID INTEGER, AttributeHierarchyStorageID INTEGER);
CREATE TABLE AttributeHierarchyStorage (ID INTEGER, StorageFileID INTEGER, SystemTableID INTEGER);
CREATE TABLE StorageFile (ID INTEGER, StorageFolderID INTEGER, FileName TEXT);
CREATE TABLE StorageFolder (ID INTEGER, Path TEXT);
"#;

const DATA: &str = r##"
INSERT INTO DBPROPERTIES VALUES ('SCHEMAVERSION', 123);
INSERT INTO Model VALUES (1, 'Model', 'en-US', 2, '{"legacyRedirects": true}', 1, 0);
INSERT INTO [Table] VALUES
    (10, 'Sales & Co', 0, 0, 0, 0, 'tag-10', 'Sales table', 210, 0),
    (30, 'Calc', 0, 2, 0, 0, NULL, NULL, 0, 0),
    (40, 'H$Sales & Co (10)$Amount (12)', 0, 1, 0, 0, NULL, NULL, 0, 0),
    (60, 'Dates', 1, 2, 1, 0, NULL, NULL, 0, 0),
    (110, 'Time Intelligence', 0, 0, 0, 0, NULL, NULL, 0, 111);
INSERT INTO [Column] VALUES
    (11, 10, 'RowNumber-2662979B', NULL, 6, 19, 3, NULL, NULL, 1, 1, NULL, 0, 1, 241, 0, NULL, 0),
    (12, 10, 'Amount', NULL, 8, 19, 1, 'amount', NULL, 0, 3, '#,0.00', 0, 1, 240, 0, 'Money', 2),
    (13, 10, 'Label', NULL, 1, 2, 2, NULL, 'FORMAT([Amount], "0")', 0, 2, NULL, 0, 0, 0, 0, NULL, 0),
    (14, 10, 'Key', NULL, 2, 19, 1, 'key', NULL, 1, 2, NULL, 12, 1, 242, 290, NULL, 0),
    (32, 30, NULL, 'Value', 1, 6, 4, '[Value]', NULL, 0, 1, NULL, 0, 1, 0, 0, NULL, 0),
    (41, 40, 'POS_TO_ID', NULL, 6, 19, 1, NULL, NULL, 0, 1, NULL, 0, 1, 0, 0, NULL, 0),
    (61, 60, NULL, 'Date', 9, 9, 4, '[Date]', NULL, 1, 2, NULL, 0, 1, 0, 0, NULL, 0),
    (62, 10, 'Weird', NULL, 2, 19, 77, 'weird', NULL, 0, 2, NULL, 0, 1, 0, 0, NULL, 0);
INSERT INTO Measure VALUES
    (15, 10, 'Total', 'SUM(''Sales & Co''[Amount])', '0.0', 0, 'KPIs', 16),
    (17, 10, 'Hidden', '1', NULL, 1, NULL, 0);
INSERT INTO FormatStringDefinition VALUES (16, '"#,0"');
INSERT INTO [Partition] VALUES
    (20, 10, 'Sales-p', 4, 0, 'let Source = 1 in Source', 50, 220, 3),
    (31, 30, 'Calc', 2, 0, 'SELECTCOLUMNS(''Sales & Co'', "Value", 1)', 0, 0, 0),
    (42, 40, 'H$', 3, 2, NULL, 0, 0, 3),
    (63, 60, 'Dates', 2, 0, 'CALENDARAUTO()', 0, 0, 3),
    (114, 110, 'TI', 7, 0, NULL, 0, 0, 3);
INSERT INTO Relationship VALUES
    (70, 'rel-1', 0, 2, 10, 14, 2, 30, 32, 1, 1),
    (71, 'rel-internal', 1, 1, 40, 41, 2, 10, 12, 1, 1),
    (72, 'rel-date', 1, 1, 10, 12, 2, 60, 61, 1, 2),
    (73, 'rel-dangling', 1, 1, 10, 999, 2, 30, 32, 1, 1);
INSERT INTO Expression VALUES
    (80, 'BlobUrl', 0, '"https://example.invalid" meta [IsParameterQuery=true, Type="Text"]', 50, 'Blob container'),
    (81, 'DaxExpr', 1, 'SUM(x[y])', 0, NULL);
INSERT INTO QueryGroup VALUES (50, 'Sources\Sales');
INSERT INTO Annotation VALUES
    (1, 1, 1, 'PBI_QueryOrder', '["BlobUrl"]'),
    (2, 10, 3, 'PBI_ResultType', 'Table'),
    (3, 12, 4, 'SummarizationSetBy', 'Automatic'),
    (4, 60, 3, '__PBI_LocalDateTable', 'true'),
    (5, 15, 8, 'Note', ''),
    (6, 999, 3, 'Orphan', 'x');
INSERT INTO Hierarchy VALUES (90, 10, 'H1');
INSERT INTO Level VALUES (91, 90, 1, 'Amount level', 12), (92, 90, 0, 'Key level', 14);
INSERT INTO Role VALUES (100, 'Readers', 2);
INSERT INTO TablePermission VALUES (101, 100, 10, '[Amount] > 0');
INSERT INTO CalculationGroup VALUES (111, 110, 5);
INSERT INTO CalculationItem VALUES (112, 111, 'YTD', 'TOTALYTD(1)', 1), (113, 111, 'Current', 'SELECTEDMEASURE()', 0);
INSERT INTO StorageFolder VALUES (200, 'Sales & Co (10).tbl'), (201, 'Sales & Co (10).tbl\20.prt');
INSERT INTO TableStorage VALUES (210, 10, 'Sales & Co (10)', 200);
INSERT INTO PartitionStorage VALUES (220, 20, 0, 230, 201);
INSERT INTO SegmentMapStorage VALUES (230, 220, 3, 5, 1, 5);
INSERT INTO ColumnStorage VALUES
    (240, 12, 'Amount (12)', 1, 250, 4, 5),
    (241, 11, 'RowNumber (11)', 0, 251, 5, 5),
    (242, 14, 'Key (14)', 2, 252, 3, 5);
INSERT INTO DictionaryStorage VALUES
    (250, 240, 2, 8, 10, 100.0, 0, 0),
    (251, 241, 0, 19, 0, 0.0, 0, 0),
    (252, 242, 1, 2, 2, 0.0, 0, 282);
INSERT INTO ColumnPartitionStorage VALUES (260, 240, 220, 270, 280), (261, 242, 220, 0, 283);
INSERT INTO SegmentStorage VALUES (270, 260, 1, 281);
INSERT INTO AttributeHierarchy VALUES (290, 14, 291);
INSERT INTO AttributeHierarchyStorage VALUES (291, 284, 40);
INSERT INTO StorageFile VALUES
    (280, 201, '1.Sales & Co (10).Amount (12).0.idf'),
    (281, 201, '1.Sales & Co (10).Amount (12).0.idfmeta'),
    (282, 200, '1.Sales & Co (10).Key (14).dictionary'),
    (283, 201, '1.Sales & Co (10).Key (14).0.idf'),
    (284, 200, 'H$Sales & Co (10)$Key (14).hidx');
"##;

fn sqlite_image(sql: &str) -> Vec<u8> {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(sql).unwrap();
    conn.serialize("main").unwrap().to_vec()
}

fn sample_db() -> Vec<u8> {
    sqlite_image(&format!("{SCHEMA}{DATA}"))
}

fn sample_archive() -> AbfArchive {
    let mut builder = AbfBuilder::new()
        .file(
            "testdb.1.db.xml",
            br#"<Load><ObjectDefinition><Database><Name>testdb</Name><CompatibilityLevel>1601</CompatibilityLevel></Database></ObjectDefinition></Load>"#.to_vec(),
        )
        .file("metadata.sqlitedb", sample_db());
    for path in [
        r"Sales & Co (10).tbl\20.prt\1.Sales & Co (10).Amount (12).0.idf",
        r"Sales & Co (10).tbl\20.prt\1.Sales & Co (10).Amount (12).0.idfmeta",
        r"Sales & Co (10).tbl\1.Sales & Co (10).Key (14).dictionary",
        r"Sales & Co (10).tbl\20.prt\1.Sales & Co (10).Key (14).0.idf",
        r"Sales & Co (10).tbl\20.prt\1.Sales & Co (10).Key (14).0.idfmeta",
        r"Sales & Co (10).tbl\H$Sales & Co (10)$Key (14).hidx",
    ] {
        builder = builder.file(path, vec![0u8; 8]);
    }
    AbfArchive::parse(builder.build()).unwrap()
}

#[test]
fn maps_tables_columns_measures_and_partitions() {
    let archive = sample_archive();
    let loaded = load_model(&archive, &ModelOptions::default()).unwrap();
    assert_eq!(loaded.schema_version, Some(123));
    let model = &loaded.model;

    assert_eq!(model.name, "Model");
    assert_eq!(model.culture.as_deref(), Some("en-US"));
    assert_eq!(
        model.default_power_bi_data_source_version.as_deref(),
        Some("powerBI_V3")
    );
    assert!(model.discourage_implicit_measures);
    assert!(matches!(
        model.extra.get("dataAccessOptions"),
        Some(PropValue::Json(v)) if v["legacyRedirects"] == true
    ));
    assert_eq!(model.annotations[0].name, "PBI_QueryOrder");
    assert_eq!(model.compatibility_level(), Some(1601));
    assert_eq!(
        model.database.as_ref().and_then(|d| d.name.as_deref()),
        Some("testdb")
    );

    let names: Vec<_> = model.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["Sales & Co", "Calc", "Dates", "Time Intelligence"]);

    let sales = model.table("sales & co").unwrap();
    assert_eq!(sales.kind(), TableKind::Regular);
    assert_eq!(sales.description.as_deref(), Some("Sales table"));
    assert_eq!(sales.lineage_tag.as_deref(), Some("tag-10"));
    assert_eq!(sales.annotations[0].value, "Table");
    let origin = sales.origin.as_ref().unwrap();
    assert_eq!(origin.to_string(), "metadata.sqlitedb#/Table/10");
    let column_names: Vec<_> = sales.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(column_names, ["Amount", "Label", "Key", "Weird"]);

    let amount = sales.column("Amount").unwrap();
    assert_eq!(amount.data_type, DataType::Double);
    assert_eq!(amount.source_column(), Some("amount"));
    assert_eq!(amount.summarize_by, Some(SummarizeBy::Sum));
    assert_eq!(amount.format_string.as_deref(), Some("#,0.00"));
    assert_eq!(amount.display_folder.as_deref(), Some("Money"));
    assert_eq!(amount.annotations[0].name, "SummarizationSetBy");
    assert_eq!(
        amount.extra.get("encodingHint").and_then(PropValue::as_str),
        Some("value")
    );

    let label = sales.column("Label").unwrap();
    assert!(label.is_calculated());
    assert_eq!(label.data_type, DataType::String);
    assert!(label.is_data_type_inferred);
    assert_eq!(label.expression(), Some(r#"FORMAT([Amount], "0")"#));
    assert_eq!(label.summarize_by, Some(SummarizeBy::None));
    assert_eq!(
        label
            .extra
            .get("isAvailableInMdx")
            .and_then(PropValue::as_bool),
        Some(false)
    );

    let key = sales.column("Key").unwrap();
    assert!(key.is_hidden);
    assert_eq!(key.sort_by_column.as_deref(), Some("Amount"));
    let weird = sales.column("Weird").unwrap();
    assert!(matches!(weird.kind, ColumnKind::Data { .. }));
    assert!(weird.extra.contains_key("vertipaq.columnType"));

    let total = sales.measure("Total").unwrap();
    assert_eq!(total.expression, "SUM('Sales & Co'[Amount])");
    assert_eq!(total.format_string.as_deref(), Some("0.0"));
    assert_eq!(total.display_folder.as_deref(), Some("KPIs"));
    assert!(matches!(
        total.extra.get("formatStringDefinition"),
        Some(PropValue::Expression(e)) if e == "\"#,0\""
    ));
    assert_eq!(total.annotations[0].value, "");
    assert!(sales.measure("Hidden").unwrap().is_hidden);

    let partition = &sales.partitions[0];
    assert_eq!(partition.name, "Sales-p");
    assert_eq!(partition.mode, PartitionMode::Import);
    assert_eq!(partition.query_group.as_deref(), Some(r"Sources\Sales"));
    assert!(matches!(
        &partition.source,
        PartitionSource::M { expression } if expression == "let Source = 1 in Source"
    ));

    let hierarchy = sales.hierarchy("H1").unwrap();
    let levels: Vec<_> = hierarchy
        .levels
        .iter()
        .map(|l| (l.name.as_str(), l.column.as_str()))
        .collect();
    assert_eq!(levels, [("Key level", "Key"), ("Amount level", "Amount")]);

    let calc = model.table("Calc").unwrap();
    assert_eq!(calc.kind(), TableKind::CalculatedTable);
    assert!(
        calc.calculated_expression()
            .unwrap()
            .starts_with("SELECTCOLUMNS")
    );
    let value = &calc.columns[0];
    assert_eq!(value.name, "Value");
    assert!(value.is_name_inferred && value.is_data_type_inferred);
    assert_eq!(value.data_type, DataType::Int64);
    assert!(matches!(
        &value.kind,
        ColumnKind::CalculatedTableColumn { source_column } if source_column == "[Value]"
    ));
    assert_eq!(
        calc.extra
            .get("vertipaq.systemFlags")
            .map(PropValue::to_json),
        Some(serde_json::json!(2))
    );

    let dates = model.table("Dates").unwrap();
    assert!(dates.is_hidden);
    assert_eq!(
        dates
            .extra
            .get("vertipaq.autoDateTable")
            .and_then(PropValue::as_str),
        Some("local")
    );
    assert_eq!(
        dates
            .extra
            .get("showAsVariationsOnly")
            .and_then(PropValue::as_bool),
        Some(true)
    );

    let ti = model.table("Time Intelligence").unwrap();
    assert_eq!(ti.kind(), TableKind::CalculationGroup);
    let group = ti.extra.get("calculationGroup").unwrap().to_json();
    assert_eq!(group["precedence"], 5);
    assert_eq!(group["calculationItem"][0]["name"], "Current");
    assert_eq!(group["calculationItem"][1]["expression"], "TOTALYTD(1)");
}

#[test]
fn maps_relationships_expressions_and_roles() {
    let archive = sample_archive();
    let model = MetadataDb::from_archive(&archive)
        .unwrap()
        .read_model(&ModelOptions::default())
        .unwrap();
    // The internal-table and dangling relationships are dropped.
    let names: Vec<_> = model
        .relationships
        .iter()
        .map(|r| r.name.as_str())
        .collect();
    assert_eq!(names, ["rel-1", "rel-date"]);
    let rel = &model.relationships[0];
    assert!(rel.from.matches("Sales & Co", "Key"));
    assert!(rel.to.matches("Calc", "Value"));
    assert_eq!(rel.from_cardinality, Cardinality::Many);
    assert_eq!(rel.to_cardinality, Cardinality::One);
    assert_eq!(rel.cross_filtering_behavior, CrossFilter::BothDirections);
    assert!(!rel.is_active);
    assert!(!rel.extra.contains_key("joinOnDateBehavior"));
    assert_eq!(
        model.relationships[1]
            .extra
            .get("joinOnDateBehavior")
            .and_then(PropValue::as_str),
        Some("datePartOnly")
    );

    let blob = model.expression("BlobUrl").unwrap();
    assert!(blob.is_parameter());
    assert_eq!(blob.expression, "\"https://example.invalid\"");
    assert_eq!(blob.query_group.as_deref(), Some(r"Sources\Sales"));
    assert_eq!(blob.description.as_deref(), Some("Blob container"));
    assert_eq!(model.parameters().count(), 1);
    assert_eq!(
        model.expression("DaxExpr").unwrap().kind,
        ExpressionKind::Dax
    );
    assert_eq!(model.query_groups[0].folder, r"Sources\Sales");

    let role = model.role("Readers").unwrap();
    assert_eq!(role.model_permission.as_deref(), Some("read"));
    assert_eq!(role.table_permissions[0].table, "Sales & Co");
    assert_eq!(
        role.table_permissions[0].filter_expression.as_deref(),
        Some("[Amount] > 0")
    );
}

#[test]
fn options_include_internal_tables_and_row_numbers() {
    let db = MetadataDb::open(&sample_db()).unwrap();
    let options = ModelOptions::default()
        .with_system_tables(true)
        .with_row_number_columns(true)
        .with_origin_path(None);
    let model = db.read_model(&options).unwrap();
    let internal = model.table("H$Sales & Co (10)$Amount (12)").unwrap();
    assert!(internal.is_hidden);
    assert_eq!(
        internal
            .extra
            .get("vertipaq.internalTable")
            .and_then(PropValue::as_bool),
        Some(true)
    );
    assert!(matches!(
        &internal.partitions[0].source,
        PartitionSource::Unknown { kind, .. } if kind == "none"
    ));
    assert_eq!(internal.partitions[0].mode, PartitionMode::Default);
    assert!(model.relationships.iter().any(|r| r.name == "rel-internal"));
    let sales = model.table("Sales & Co").unwrap();
    assert!(matches!(sales.columns[0].kind, ColumnKind::RowNumber));
    assert!(sales.origin.is_none());
}

#[test]
fn storage_map_resolves_files() {
    let archive = sample_archive();
    let storage = load_model(&archive, &ModelOptions::default())
        .unwrap()
        .storage;
    assert!(storage.missing_files(&archive).is_empty());
    assert_eq!(storage.tables.len(), 5);
    assert!(
        storage
            .table("H$Sales & Co (10)$Amount (12)")
            .unwrap()
            .is_internal
    );

    let sales = storage.table("Sales & Co").unwrap();
    assert_eq!(sales.folder.as_deref(), Some("Sales & Co (10).tbl"));
    assert_eq!(sales.storage_name.as_deref(), Some("Sales & Co (10)"));
    assert_eq!(sales.row_count, 5);
    assert_eq!(
        sales.partitions[0].folder.as_deref(),
        Some("Sales & Co (10).tbl/20.prt")
    );
    assert_eq!(sales.partitions[0].records_per_segment, 5);
    // Storage order: RowNumber (0), Amount (1), Key (2), then columns without storage.
    let order: Vec<_> = sales.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        order,
        ["RowNumber-2662979B", "Amount", "Key", "Label", "Weird"]
    );

    let amount = sales.column("Amount").unwrap();
    assert!(amount.has_values());
    let dictionary = amount.dictionary.as_ref().unwrap();
    assert_eq!(dictionary.encoding, DictionaryEncoding::Value);
    assert_eq!((dictionary.base_id, dictionary.magnitude), (10, 100.0));
    assert_eq!(amount.dictionary_path(), None);
    assert_eq!(amount.statistics.distinct_states, Some(4));
    let files = &amount.partitions[0];
    assert_eq!(
        files.idf.as_deref(),
        Some("Sales & Co (10).tbl/20.prt/1.Sales & Co (10).Amount (12).0.idf")
    );
    assert_eq!(
        files.idfmeta.as_deref(),
        Some("Sales & Co (10).tbl/20.prt/1.Sales & Co (10).Amount (12).0.idfmeta")
    );
    assert_eq!(
        (files.partition_id, files.record_count, files.segment_count),
        (20, Some(5), Some(1))
    );

    let key = sales.column("Key").unwrap();
    assert_eq!(key.data_type, DataType::String);
    assert_eq!(
        key.dictionary.as_ref().unwrap().encoding,
        DictionaryEncoding::Hash
    );
    assert_eq!(
        key.dictionary_path(),
        Some("Sales & Co (10).tbl/1.Sales & Co (10).Key (14).dictionary")
    );
    assert_eq!(
        key.hidx_path(),
        Some("Sales & Co (10).tbl/H$Sales & Co (10)$Key (14).hidx")
    );
    assert_eq!(
        key.attribute_hierarchy.as_ref().unwrap().system_table_id,
        Some(40)
    );
    // No SegmentStorage row: the idfmeta path falls back to `<idf>meta`.
    assert!(
        key.partitions[0]
            .idfmeta
            .as_deref()
            .unwrap()
            .ends_with(".0.idfmeta")
    );
    assert_eq!(key.files().count(), 4);
    for path in key.files() {
        assert!(archive.read(path).is_ok(), "{path}");
    }

    let row_number = sales.column("RowNumber-2662979B").unwrap();
    assert_eq!(row_number.kind, ColumnKindCode::RowNumber);
    assert!(!row_number.has_values());
    assert_eq!(
        row_number.dictionary.as_ref().unwrap().encoding,
        DictionaryEncoding::None
    );
    assert!(sales.column("Label").unwrap().partitions.is_empty());
}

#[test]
fn legacy_schema_columns_are_understood() {
    let image = sqlite_image(
        r#"
        CREATE TABLE [Table] (ID INTEGER, Name TEXT);
        CREATE TABLE [Column] (ID INTEGER, TableID INTEGER, ExplicitName TEXT,
            ExplicitDataType INTEGER, BindingType INTEGER);
        CREATE TABLE Relationship (ID INTEGER, Name TEXT, FromEndTableID INTEGER,
            FromEndColumnID INTEGER, FromEndCardinality INTEGER, ToEndTableID INTEGER,
            ToEndColumnID INTEGER, ToEndCardinality INTEGER);
        CREATE TABLE [Partition] (ID INTEGER, TableID INTEGER, Name TEXT, QueryDefinition TEXT);
        INSERT INTO [Table] VALUES (1, 'A'), (2, 'B');
        INSERT INTO [Column] VALUES (3, 1, 'x', 6, 1), (4, 2, 'y', 6, 2), (5, 2, 'rn', 6, 3);
        INSERT INTO Relationship VALUES (6, 'r', 1, 3, 1, 2, 4, 1);
        INSERT INTO [Partition] VALUES (7, 1, 'A', 'select 1');
        "#,
    );
    let db = MetadataDb::open(&image).unwrap();
    assert_eq!(db.schema_version(), None);
    let model = db.read_model(&ModelOptions::default()).unwrap();
    assert_eq!(model.name, "Model");
    assert!(
        model
            .table("B")
            .unwrap()
            .column("y")
            .unwrap()
            .is_calculated()
    );
    assert!(model.table("B").unwrap().column("rn").is_none());
    let rel = &model.relationships[0];
    assert!(rel.from.matches("A", "x") && rel.to.matches("B", "y"));
    assert_eq!(rel.from_cardinality, Cardinality::One);
    assert!(matches!(
        &model.table("A").unwrap().partitions[0].source,
        PartitionSource::Unknown { kind, props } if kind == "unknown" && props.contains_key("queryDefinition")
    ));
    assert_eq!(
        model.table("A").unwrap().partitions[0].mode,
        PartitionMode::Default
    );
    // The storage map works without any storage tables.
    let storage = db.storage_map().unwrap();
    assert_eq!(storage.tables.len(), 2);
    assert!(storage.files().next().is_none());
}

#[test]
fn invalid_databases_are_errors() {
    assert!(matches!(
        MetadataDb::open(b"not sqlite"),
        Err(MetadataError::NotSqlite)
    ));
    assert!(matches!(
        MetadataDb::open(b""),
        Err(MetadataError::NotSqlite)
    ));

    // A database without the TOM tables.
    let other = sqlite_image("CREATE TABLE t (a INTEGER); INSERT INTO t VALUES (1);");
    let db = MetadataDb::open(&other).unwrap();
    assert!(matches!(
        db.read_model(&ModelOptions::default()),
        Err(MetadataError::MissingTable("Table"))
    ));
    assert!(db.storage_map().is_err());

    // Views are not followed (they could hide expensive queries).
    let view = sqlite_image(
        "CREATE TABLE real (ID INTEGER, Name TEXT); CREATE VIEW [Table] AS SELECT * FROM real; CREATE TABLE [Column] (ID INTEGER);",
    );
    let db = MetadataDb::open(&view).unwrap();
    assert!(matches!(
        db.read_model(&ModelOptions::default()),
        Err(MetadataError::MissingTable("Table"))
    ));

    // Truncated and corrupted images fail cleanly (or load what is readable).
    let image = sample_db();
    for cut in [16, 100, 1024, image.len() / 2, image.len() - 1] {
        if let Ok(db) = MetadataDb::open(&image[..cut]) {
            let _ = db.load(&ModelOptions::default());
        }
    }
    let mut corrupt = image.clone();
    for (i, byte) in corrupt.iter_mut().enumerate().skip(100) {
        if i % 7 == 0 {
            *byte ^= 0x5A;
        }
    }
    if let Ok(db) = MetadataDb::open(&corrupt) {
        let _ = db.load(&ModelOptions::default());
    }

    // Archives without metadata.
    let archive = AbfArchive::parse(AbfBuilder::new().file("x.bin", vec![1]).build()).unwrap();
    assert!(matches!(
        load_model(&archive, &ModelOptions::default()),
        Err(MetadataError::MissingMetadata)
    ));
}

#[test]
fn database_is_read_only() {
    let db = MetadataDb::open(&sample_db()).unwrap();
    assert!(db.connection().execute("DELETE FROM [Table]", []).is_err());
    assert!(
        db.connection()
            .execute("CREATE TABLE evil (a INTEGER)", [])
            .is_err()
    );
}

#[test]
fn rows_tolerate_odd_values() {
    let image = sqlite_image(
        r#"
        CREATE TABLE [Table] (ID, Name, IsHidden);
        CREATE TABLE [Column] (ID, TableID, ExplicitName, ExplicitDataType, Type);
        INSERT INTO [Table] VALUES ('2', X'FFFE', 1.0), (1.0, 'T', '1'), (NULL, 'N', NULL);
        INSERT INTO [Column] VALUES (3, 1, 'c', '2', 1.0);
        "#,
    );
    let db = MetadataDb::open(&image).unwrap();
    let tables = db.rows("Table").unwrap();
    assert_eq!(tables.len(), 3);
    let model = db.read_model(&ModelOptions::default()).unwrap();
    let names: Vec<_> = model.tables.iter().map(|t| t.name.as_str()).collect();
    // Sorted by ID (NULL last); the blob name becomes empty.
    assert_eq!(names, ["T", "", "N"]);
    assert!(model.tables[0].is_hidden && model.tables[1].is_hidden);
    assert_eq!(model.tables[0].columns[0].data_type, DataType::String);
}

#[test]
fn mutated_archives_with_metadata_never_panic() {
    let bytes = sample_archive().as_bytes().to_vec();
    let archive = AbfArchive::parse(bytes.clone()).unwrap();
    let db = archive.metadata_file().unwrap();
    let (start, len) = (db.offset as usize, db.size as usize);
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..200 {
        let mut mutated = bytes.clone();
        for _ in 0..(next() % 16 + 1) {
            let at = start + (next() as usize) % len;
            mutated[at] = next() as u8;
        }
        crate::abf::fuzz_abf(&mutated);
    }
}

#[test]
fn power_pivot_xml_metadata_loads_schema_and_storage() {
    use pbi_model::{Cardinality, ColumnKind, DataType, PartitionSource};

    let archive = AbfArchive::parse(crate::abf::test_support::power_pivot_sample()).unwrap();
    assert!(archive.is_compressed());
    assert!(matches!(
        MetadataDb::from_archive(&archive),
        Err(MetadataError::MissingMetadata)
    ));
    let loaded = load_model(&archive, &ModelOptions::default()).unwrap();
    assert_eq!(loaded.format, MetadataFormat::Xml);
    assert_eq!(loaded.storage.tables.len(), 2);
    assert_eq!(loaded.diagnostics.len(), 1, "{:?}", loaded.diagnostics);
    assert_eq!(
        loaded.diagnostics[0].code,
        crate::vertipaq::codes::XML_METADATA
    );
    assert_eq!(loaded.diagnostics[0].severity, pbi_model::Severity::Info);
    let model = &loaded.model;
    assert_eq!(model.name, "Model");
    let db = model.database.as_ref().unwrap();
    assert_eq!(db.name.as_deref(), Some("PowerPivotDb"));
    assert_eq!(db.compatibility_level, Some(1103));
    let names: Vec<&str> = model.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["Sales", "Product"]);

    let sales = model.table("Sales").unwrap();
    assert_eq!(sales.description.as_deref(), Some("Fact & detail"));
    let cols: Vec<(&str, DataType, bool)> = sales
        .columns
        .iter()
        .map(|c| (c.name.as_str(), c.data_type.clone(), c.is_hidden))
        .collect();
    assert_eq!(
        cols,
        [
            ("ProductKey", DataType::Int64, false),
            ("Net Amount", DataType::Decimal, false),
            ("Date", DataType::DateTime, true),
            // The calculated column's type comes from its dictionary (`XM_Long`).
            ("Year", DataType::Int64, false),
        ]
    );
    assert_eq!(
        sales.column("Net Amount").unwrap().kind,
        ColumnKind::Data {
            source_column: "Net Amount".into()
        }
    );
    assert_eq!(
        sales.column("Year").unwrap().kind,
        ColumnKind::Calculated {
            expression: "YEAR([Date])".into()
        }
    );
    assert_eq!(sales.measures.len(), 1);
    assert_eq!(sales.measures[0].name, "Total Sales");
    assert_eq!(sales.measures[0].expression, "SUM('Sales'[Net Amount])");
    assert!(matches!(
        &sales.partitions[0].source,
        PartitionSource::Query { query, data_source: Some(ds) }
            if query == "SELECT [Sales$].* FROM [Sales$]" && ds == "Workbook"
    ));

    let product = model.table("Product").unwrap();
    assert!(product.is_hidden);
    assert_eq!(product.hierarchies[0].levels[0].column, "Product Name");

    let rel = &model.relationships[0];
    assert_eq!(
        (rel.from.table.as_str(), rel.from.column.as_str()),
        ("Sales", "ProductKey")
    );
    assert_eq!(
        (rel.to.table.as_str(), rel.to.column.as_str()),
        ("Product", "ProductKey")
    );
    assert_eq!(rel.from_cardinality, Cardinality::Many);
    assert_eq!(rel.to_cardinality, Cardinality::One);

    // The data source connection string (and its password) is never read.
    assert!(!format!("{model:?}").contains("hunter2"));
    // Row-number columns on request.
    let with_rows = load_model(
        &archive,
        &ModelOptions::default().with_row_number_columns(true),
    )
    .unwrap();
    assert_eq!(with_rows.model.table("Sales").unwrap().columns.len(), 5);

    // The storage decodes with the regular VertiPaq decoder.
    use arrow::array::{AsArray, Decimal128Array, Int64Array, StringArray};
    let limits = crate::vertipaq::DecodeLimits::default();
    let sales =
        crate::vertipaq::decode_table(&archive, &loaded.storage, model, "Sales", &limits).unwrap();
    assert_eq!(sales.num_rows(), 3);
    assert_eq!(
        sales
            .column(0)
            .as_primitive::<arrow::datatypes::Int64Type>(),
        &Int64Array::from(vec![10, 10, 20])
    );
    let amount = sales
        .column(1)
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .unwrap();
    assert_eq!(amount.value(0), 125_000);
    assert_eq!(amount.value(2), 72_500);
    assert_eq!(
        sales
            .column(3)
            .as_primitive::<arrow::datatypes::Int64Type>(),
        &Int64Array::from(vec![2023, 2023, 2023])
    );
    let date = sales
        .column(2)
        .as_primitive::<arrow::datatypes::TimestampMicrosecondType>();
    assert_eq!(date.value(1) - date.value(0), 86_400_000_000);
    let product =
        crate::vertipaq::decode_table(&archive, &loaded.storage, model, "Product", &limits)
            .unwrap();
    assert_eq!(
        product.column(1).as_string::<i32>(),
        &StringArray::from(vec!["Widget", "Gadget"])
    );
}

#[test]
fn malformed_xml_metadata_parts_are_skipped_with_a_warning() {
    use crate::abf::test_support::AbfBuilder;
    let cube = r#"<Load xmlns="http://schemas.microsoft.com/analysisservices/2003/engine"><ObjectDefinition><Cube><Name>Model</Name><Dimensions><Dimension><Name>T</Name><DimensionID>T1</DimensionID></Dimension></Dimensions></Cube></ObjectDefinition></Load>"#;
    let archive = AbfArchive::parse(
        AbfBuilder::new()
            .file("Model.0.cub.xml", cube.as_bytes().to_vec())
            .file("T1.0.dim.xml", b"<Load><Dimension>".to_vec())
            .file(
                "evil.0.dim.xml",
                b"<!DOCTYPE x [<!ENTITY a \"aaaa\">]><Load>&a;</Load>".to_vec(),
            )
            .build(),
    )
    .unwrap();
    let loaded = load_model(&archive, &ModelOptions::default()).unwrap();
    assert_eq!(loaded.model.tables.len(), 1);
    assert!(loaded.model.tables[0].columns.is_empty());
    assert!(loaded.storage.tables.is_empty());
    let skipped = loaded
        .diagnostics
        .iter()
        .filter(|d| d.code == crate::vertipaq::codes::XML_METADATA_PART)
        .count();
    assert_eq!(skipped, 2);
    // No storage for `T`: the summary is a warning naming it.
    let summary = &loaded.diagnostics[0];
    assert_eq!(summary.code, crate::vertipaq::codes::XML_METADATA);
    assert_eq!(summary.severity, pbi_model::Severity::Warning);
    assert!(summary.message.ends_with(": T"), "{}", summary.message);

    // A broken cube is an error.
    let broken = AbfArchive::parse(
        AbfBuilder::new()
            .file("Model.0.cub.xml", b"<Load><Cube>".to_vec())
            .build(),
    )
    .unwrap();
    assert!(load_model(&broken, &ModelOptions::default()).is_err());
}

#[test]
fn mutated_power_pivot_backups_never_panic() {
    let base = crate::abf::test_support::power_pivot_sample();
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let limits = crate::vertipaq::DecodeLimits::default();
    for _ in 0..300 {
        let mut bytes = base.clone();
        for _ in 0..1 + next() % 8 {
            let i = (next() % bytes.len() as u64) as usize;
            bytes[i] = (next() >> 24) as u8;
        }
        let Ok(archive) = AbfArchive::parse(bytes) else {
            continue;
        };
        if let Ok(loaded) = load_model(&archive, &ModelOptions::default()) {
            let _ = crate::vertipaq::decode_all_with_diagnostics(
                &archive,
                &loaded.storage,
                &loaded.model,
                &limits,
            );
        }
    }
}
