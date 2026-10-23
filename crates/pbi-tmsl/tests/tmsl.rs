//! Reader/writer tests on the fixtures in `tests/data`: `acme.bim` (the
//! Acme Outfitters TMDL example converted to TMSL by DashCraft's own
//! TMDL → TMSL path) and the hand-written `features.bim` (rarer features and
//! unknown future properties). The Microsoft COVID-19 sample's UTF-16LE
//! `DataModelSchema` is covered in `pbi-core`'s `readers.rs`, which can open
//! the PBIT container.

use std::path::PathBuf;

use pbi_model::{
    Cardinality, Column, ColumnKind, ColumnRef, CrossFilter, DataType, ExpressionKind, Measure,
    Model, NamedExpression, PartitionMode, PartitionSource, PropValue, Relationship,
    SecurityFilteringBehavior, Severity, SummarizeBy, Table, TableKind,
};
use pbi_tmsl::{
    KeyOrder, LineEnding, ReadOptions, TextEncoding, TmslError, TmslFormat, WriteOptions,
    encode_text, read_bim, read_bytes, read_data_model_schema, read_text, to_json, write_bim,
    write_bytes, write_data_model_schema, write_text,
};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

fn data(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("invalid JSON: {e}"))
}

/// `write_bim(read_bim(x))` as a JSON value, with the source's database
/// name and compatibility level.
fn round_trip_value(text: &str) -> Value {
    let loaded = read_bim(text).unwrap_or_else(|e| panic!("read failed: {e}"));
    value(&write_bim(
        &loaded.model,
        loaded.database_name.as_deref(),
        loaded.compatibility_level,
    ))
}

fn table<'a>(model: &'a Model, name: &str) -> &'a Table {
    model.table(name).unwrap_or_else(|| panic!("table {name}"))
}

fn measure<'a>(model: &'a Model, table_name: &str, name: &str) -> &'a Measure {
    model
        .measure(table_name, name)
        .unwrap_or_else(|| panic!("measure {name}"))
}

fn column<'a>(model: &'a Model, table_name: &str, name: &str) -> &'a Column {
    model
        .column(table_name, name)
        .unwrap_or_else(|| panic!("column {name}"))
}

// ---------------------------------------------------------------------------
// acme.bim

#[test]
fn acme_maps_to_the_model() {
    let loaded = read_bim(&data("acme.bim")).unwrap();
    assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
    assert_eq!(loaded.database_name.as_deref(), Some("AcmeOutfitters"));
    assert_eq!(loaded.compatibility_level, Some(1601));
    let m = &loaded.model;
    assert_eq!(m.compatibility_level(), Some(1601));
    assert_eq!(m.culture.as_deref(), Some("en-US"));
    assert_eq!(
        m.default_power_bi_data_source_version.as_deref(),
        Some("powerBI_V3")
    );
    assert_eq!(m.source_query_culture.as_deref(), Some("en-US"));
    assert!(m.discourage_implicit_measures);
    assert!(
        m.description
            .as_deref()
            .unwrap()
            .starts_with("Acme Outfitters: a synthetic outdoor-gear retailer")
    );

    let names: Vec<_> = m.tables.iter().map(|t| t.name.as_str()).collect();
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

    // Orders: 11 data columns, 4 calculated columns, 20 measures.
    let orders = table(m, "Orders");
    assert_eq!(orders.kind(), TableKind::Regular);
    assert!(
        orders
            .description
            .as_deref()
            .unwrap()
            .starts_with("One row per order line (synthetic).")
    );
    assert_eq!(orders.columns.len(), 15);
    assert_eq!(
        orders.columns.iter().filter(|c| c.is_calculated()).count(),
        4
    );
    assert_eq!(orders.measures.len(), 20);
    let order_id = column(m, "Orders", "Order ID");
    assert_eq!(
        order_id.kind,
        ColumnKind::Data {
            source_column: "Order ID".into()
        }
    );
    assert!(order_id.is_key);
    assert_eq!(order_id.data_type, DataType::String);
    assert_eq!(order_id.summarize_by, Some(SummarizeBy::None));
    assert_eq!(
        order_id.description.as_deref(),
        Some("Order number (SO-…). Unique after Table.Distinct.")
    );
    let customer_id = column(m, "Orders", "Customer ID");
    assert!(customer_id.is_hidden);
    let returned = column(m, "Orders", "Is Returned");
    assert_eq!(
        returned.expression(),
        Some(r#"IF(ISEMPTY(RELATEDTABLE(Returns)), "No", "Yes")"#)
    );
    let line_cost = column(m, "Orders", "Line Cost");
    assert_eq!(line_cost.data_type, DataType::Double);
    assert_eq!(line_cost.summarize_by, Some(SummarizeBy::Sum));
    assert_eq!(line_cost.format_string.as_deref(), Some("\\$#,0.00"));
    assert_eq!(
        line_cost.expression(),
        Some("Orders[Quantity] * RELATED(Products[Unit Cost])")
    );

    let margin = measure(m, "Orders", "Margin %");
    assert_eq!(margin.expression, "DIVIDE([Gross Margin], [Total Sales])");
    assert_eq!(margin.format_string.as_deref(), Some("0.0%"));
    assert_eq!(margin.display_folder.as_deref(), Some("Sales"));
    assert_eq!(
        measure(m, "Orders", "Total Sales").description.as_deref(),
        Some("Sum of the order line amounts.")
    );
    assert_eq!(
        measure(m, "Orders", "Avg Order Value").expression,
        "AVERAGEX(\n    VALUES(Orders[Order ID]),\n    CALCULATE(SUM(Orders[Line Amount]))\n)"
    );
    assert_eq!(
        measure(m, "Orders", "Last Order Date")
            .format_string
            .as_deref(),
        Some("yyyy-mm-dd")
    );
    assert!(
        measure(m, "Orders", "Sales (Scaled)")
            .format_string_definition()
            .is_some_and(|d| d.starts_with("IF(ABS(SELECTEDMEASURE()) >= 1000000"))
    );
    // TOM JSON wraps a measure's dynamic format string in an object.
    let raw = value(&data("acme.bim"));
    let scaled = &raw["model"]["tables"][0]["measures"][4];
    assert_eq!(scaled["name"], "Sales (Scaled)");
    assert!(
        scaled["formatStringDefinition"]["expression"]
            .as_str()
            .unwrap()
            .starts_with("IF(ABS(SELECTEDMEASURE())")
    );
    let (home, _) = m.find_measure("products never sold count").unwrap();
    assert_eq!(home.name, "Products Never Sold");

    // M partitions are joined from line arrays.
    let partition = &orders.partitions[0];
    assert_eq!(partition.name, "Orders");
    assert_eq!(partition.mode, PartitionMode::Import);
    assert_eq!(partition.query_group.as_deref(), Some("Sources"));
    let PartitionSource::M { expression } = &partition.source else {
        panic!("expected an M partition, got {:?}", partition.source);
    };
    assert!(expression.starts_with("let\n    Source = AzureStorage.Blobs(StorageContainerUrl),\n"));
    assert!(expression.ends_with("\nin\n    #\"Renamed Columns\""));
    assert_eq!(expression.lines().count(), 13);

    // Calculated table with calculated-table columns.
    let never_sold = table(m, "Products Never Sold");
    assert_eq!(never_sold.kind(), TableKind::CalculatedTable);
    assert!(
        never_sold
            .calculated_expression()
            .unwrap()
            .starts_with("VAR SoldProducts =\n    SELECTCOLUMNS(")
    );
    let inferred = &never_sold.columns[0];
    assert_eq!(
        inferred.kind,
        ColumnKind::CalculatedTableColumn {
            source_column: "[Product ID]".into()
        }
    );
    assert!(inferred.is_name_inferred);
    assert_eq!(table(m, "Date").kind(), TableKind::CalculatedTable);

    // Calculation group with a dynamic format string on one item.
    let time_calc = table(m, "Time Calc");
    assert_eq!(time_calc.kind(), TableKind::CalculationGroup);
    let group = time_calc.calculation_group.as_ref().unwrap();
    assert_eq!(group.precedence, 10);
    let items: Vec<_> = group
        .calculation_items
        .iter()
        .map(|i| i.name.as_str())
        .collect();
    assert_eq!(items, ["Current", "YTD", "PY", "YoY %"]);

    let last_refreshed = table(m, "Last Refreshed");
    assert!(last_refreshed.is_hidden);
    assert_eq!(last_refreshed.columns[0].data_type, DataType::DateTime);

    // Relationships: explicit properties where set, TOM defaults otherwise.
    assert_eq!(m.relationships.len(), 8);
    let rel = &m.relationships[0];
    assert_eq!(rel.name, "c79d4066-724d-5669-b57b-f6bf11da6f40");
    assert_eq!(rel.from, ColumnRef::new("Orders", "Customer ID"));
    assert_eq!(rel.to, ColumnRef::new("Customers", "Customer ID"));
    assert_eq!(rel.cross_filtering_behavior, CrossFilter::BothDirections);
    assert_eq!(rel.from_cardinality, Cardinality::Many);
    assert_eq!(rel.to_cardinality, Cardinality::One);
    assert!(rel.is_active);
    assert_eq!(
        rel.security_filtering_behavior,
        SecurityFilteringBehavior::OneDirection
    );
    assert!(!rel.rely_on_referential_integrity);
    assert!(!m.relationships[4].is_active);
    let one_to_one = &m.relationships[5];
    assert_eq!(one_to_one.from_cardinality, Cardinality::One);
    assert_eq!(one_to_one.to_cardinality, Cardinality::One);
    assert_eq!(
        one_to_one.security_filtering_behavior,
        SecurityFilteringBehavior::BothDirections
    );
    let many_to_many = &m.relationships[6];
    assert_eq!(many_to_many.from, ColumnRef::new("Targets", "Region"));
    assert_eq!(many_to_many.to_cardinality, Cardinality::Many);

    // Parameter expression with its meta record split off; two M functions.
    let blob = m.expression("StorageContainerUrl").unwrap();
    assert_eq!(blob.kind, ExpressionKind::M);
    assert!(blob.is_parameter());
    assert_eq!(blob.parameter_type().as_deref(), Some("Text"));
    assert_eq!(
        blob.expression,
        r#""https://acmeoutfittersdemo.blob.core.windows.net/sample-data""#
    );
    assert_eq!(
        blob.meta.as_deref(),
        Some(r#"[IsParameterQuery=true, Type="Text", IsParameterQueryRequired=true]"#)
    );
    assert!(
        blob.description
            .as_deref()
            .unwrap()
            .starts_with("Fictional Azure Blob container")
    );
    assert_eq!(m.expressions.len(), 3);
    assert_eq!(m.parameters().count(), 1);

    let roles: Vec<_> = m.roles.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(roles, ["Region West", "Store Manager"]);
    assert_eq!(
        m.roles[0].table_permissions[0].filter_expression.as_deref(),
        Some(r#"Stores[Region] = "West""#)
    );
}

#[test]
fn acme_keeps_only_untyped_tom_properties_in_extra_bags() {
    // The file follows TOM conventions (multi-line text as arrays, defaults
    // omitted); only the properties DashCraft does not model end up in
    // `extra` bags.
    let model = read_bim(&data("acme.bim")).unwrap().model;
    let mut extras: Vec<String> = Vec::new();
    let mut collect = |path: &str, keys: Vec<&String>| {
        extras.extend(keys.into_iter().map(|k| format!("{path}.{k}")));
    };
    collect("model", model.extra.keys().collect());
    collect(
        "database",
        model.database.as_ref().unwrap().extra.keys().collect(),
    );
    for t in &model.tables {
        collect(&t.name, t.extra.keys().collect());
        for c in &t.columns {
            collect(&format!("{}[{}]", t.name, c.name), c.extra.keys().collect());
        }
        for ms in &t.measures {
            collect(
                &format!("{}[{}]", t.name, ms.name),
                ms.extra.keys().collect(),
            );
        }
        for p in &t.partitions {
            collect(&format!("partition {}", p.name), p.extra.keys().collect());
        }
    }
    for r in &model.relationships {
        collect(&r.name, r.extra.keys().collect());
    }
    for e in &model.expressions {
        collect(&e.name, e.extra.keys().collect());
    }
    assert_eq!(extras, EXPECTED_ACME_EXTRAS);
}

#[test]
fn acme_round_trips_byte_identically() {
    let text = data("acme.bim");
    let loaded = read_bim(&text).unwrap();
    assert_eq!(loaded.format.key_order, KeyOrder::Tom);
    assert_eq!(loaded.format.indent, "  ");
    assert_eq!(loaded.format.line_ending, LineEnding::Lf);
    assert!(loaded.format.trailing_newline);
    assert_eq!(write_text(&loaded.model, &loaded.write_options()), text);
    assert_eq!(round_trip_value(&text), value(&text));
    // `write_bim` uses the same layout minus the trailing newline.
    assert_eq!(
        write_bim(&loaded.model, None, None),
        text.trim_end_matches('\n')
    );
}

/// The DAX and M text of a TMDL table file, read with a few string rules:
/// `(measure name, DAX)` and the partition `source` bodies.
fn tmdl_expressions(tmdl: &str) -> (Vec<(String, String)>, Vec<String>) {
    let lines: Vec<&str> = tmdl.lines().collect();
    let block = |from: usize, indent: &str| -> String {
        lines[from..]
            .iter()
            .take_while(|l| l.starts_with(indent))
            .map(|l| &l[indent.len()..])
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut measures = Vec::new();
    let mut sources = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if let Some(rest) = line.strip_prefix("\tmeasure ") {
            let (name, dax) = match rest.split_once(" = ") {
                Some((name, dax)) => (name, dax.to_owned()),
                None => (rest.trim_end_matches(" ="), block(i + 1, "\t\t\t")),
            };
            measures.push((name.trim_matches('\'').to_owned(), dax));
        }
        if line.trim() == "source =" {
            sources.push(block(i + 1, "\t\t\t\t"));
        }
    }
    (measures, sources)
}

/// `acme.bim` carries the same DAX and M text as the example's TMDL files.
#[test]
fn acme_matches_the_tmdl_example() {
    let root = std::env::var_os("PBIVIEW_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"));
    let dir = root.join("examples/acme-outfitters/AcmeOutfitters.SemanticModel/definition/tables");
    if !dir.is_dir() {
        println!("SKIP: {} missing", dir.display());
        return;
    }
    let model = read_bim(&data("acme.bim")).unwrap().model;
    let (mut measures, mut sources) = (0, 0);
    for t in &model.tables {
        let tmdl = std::fs::read_to_string(dir.join(format!("{}.tmdl", t.name)))
            .unwrap()
            .replace("\r\n", "\n");
        let (tmdl_measures, tmdl_sources) = tmdl_expressions(&tmdl);
        for (name, dax) in &tmdl_measures {
            assert_eq!(&measure(&model, &t.name, name).expression, dax, "{name}");
        }
        for (partition, source) in t.partitions.iter().zip(&tmdl_sources) {
            assert_eq!(
                partition.source.expression().unwrap(),
                source,
                "partition of {}",
                t.name
            );
        }
        assert_eq!(t.measures.len(), tmdl_measures.len(), "{}", t.name);
        measures += tmdl_measures.len();
        sources += tmdl_sources.len();
    }
    assert_eq!((measures, sources), (33, 10));
}

// ---------------------------------------------------------------------------
// features.bim

#[test]
fn features_round_trip_value_and_byte_identically() {
    let text = data("features.bim");
    let loaded = read_bim(&text).unwrap();
    assert_eq!(round_trip_value(&text), value(&text));
    assert_eq!(write_text(&loaded.model, &loaded.write_options()), text);
}

#[test]
fn features_map_rarer_objects() {
    let loaded = read_bim(&data("features.bim")).unwrap();
    let m = &loaded.model;
    let db = m.database.as_ref().unwrap();
    assert_eq!(db.compatibility_mode.as_deref(), Some("powerBI"));
    assert_eq!(
        db.extra.keys().collect::<Vec<_>>(),
        ["id", "futureDatabaseProp"]
    );
    assert!(m.discourage_implicit_measures);
    assert_eq!(
        m.extra.keys().collect::<Vec<_>>(),
        [
            "dataAccessOptions",
            "futureModelSetting",
            "dataSources",
            "futureCollection"
        ]
    );
    // Numbers in unknown properties keep their exact text.
    let PropValue::Json(setting) = &m.extra["futureModelSetting"] else {
        panic!("expected JSON");
    };
    assert_eq!(
        setting.to_string(),
        r#"{"level":3,"ratio":0.10,"big":12345678901234567890123,"neg":-0.0,"exp":1e-300}"#
    );

    let sales = table(m, "Sales");
    assert_eq!(
        sales.description.as_deref(),
        Some("Fact table with one row per order line.\nSecond line of the description.")
    );
    assert!(sales.extra.contains_key("refreshPolicy"));
    assert!(sales.extra.contains_key("futureTableFlag"));
    let order_date = column(m, "Sales", "OrderDate");
    assert!(order_date.is_key);
    assert_eq!(order_date.annotations[0].value, "Date");
    assert!(order_date.extra.contains_key("variations"));
    let big = column(m, "Sales", "Big Order");
    assert_eq!(
        big.expression(),
        Some("\nVAR a = Sales[Amount]\nRETURN a > 1000")
    );
    assert!(big.is_data_type_inferred);
    let row_number = &sales.columns[3];
    assert_eq!(row_number.kind, ColumnKind::RowNumber);
    assert_eq!(
        row_number.extra.keys().collect::<Vec<_>>(),
        ["isUnique", "isNullable", "attributeHierarchy"]
    );
    // Unknown column type: read as a data column, kept verbatim, reported.
    let odd = column(m, "Sales", "Odd");
    assert_eq!(odd.source_column(), Some("Odd"));
    assert_eq!(
        odd.extra["type"],
        PropValue::Json(json!("futureColumnType"))
    );

    // Partitions: M with an unknown source key, and an unknown source type.
    let current = sales.partition("Sales-Current").unwrap();
    assert_eq!(current.query_group.as_deref(), Some("Facts"));
    assert!(matches!(current.source, PartitionSource::M { .. }));
    assert_eq!(current.extra["dataView"], PropValue::Json(json!("full")));
    assert_eq!(
        current.extra["source"],
        PropValue::Json(json!({"futureSourceProp": 1}))
    );
    let PartitionSource::Unknown { kind, props } = &sales.partition("Sales-2020").unwrap().source
    else {
        panic!("expected an unknown source type");
    };
    assert_eq!(kind, "policyRange");
    assert_eq!(
        props.keys().collect::<Vec<_>>(),
        ["start", "end", "granularity"]
    );

    let total = measure(m, "Sales", "Total Sales");
    assert_eq!(total.display_folder.as_deref(), Some("KPIs"));
    assert_eq!(total.annotations[0].value, "line one\nline two");
    assert!(total.extra.contains_key("kpi"));
    assert!(total.extra.contains_key("formatStringDefinition"));
    assert_eq!(
        measure(m, "Sales", "Multi As String").expression,
        "VAR x = 1\nRETURN x"
    );
    assert_eq!(measure(m, "Sales", "Single As Array").expression, "1 + 1");

    let calendar = sales.hierarchy("calendar").unwrap();
    assert_eq!(calendar.display_folder.as_deref(), Some("Dates"));
    assert_eq!(calendar.levels.len(), 2);
    assert_eq!(calendar.levels[1].ordinal, Some(1));
    assert_eq!(calendar.levels[1].column, "OrderDate");
    assert!(calendar.levels[1].extra.contains_key("futureLevelProp"));
    assert!(calendar.extra.contains_key("hideMembers"));

    // Legacy query partition without `type`.
    let legacy = table(m, "Legacy");
    assert!(!legacy.is_hidden);
    assert_eq!(
        legacy.partitions[0].source,
        PartitionSource::Query {
            query: "SELECT Id FROM dbo.Legacy".into(),
            data_source: Some("Legacy SQL".into()),
        }
    );
    assert!(legacy.annotations.is_empty());
    assert!(legacy.extra.contains_key("annotations"));

    let remote = table(m, "Remote Sales");
    assert_eq!(remote.kind(), TableKind::Remote);
    assert_eq!(remote.partitions[0].mode, PartitionMode::DirectLake);
    assert_eq!(remote.source_lineage_tag.as_deref(), Some("[dbo].[sales]"));
    assert!(matches!(
        &remote.partitions[0].source,
        PartitionSource::Entity { entity_name, schema_name, .. }
            if entity_name == "sales" && schema_name.as_deref() == Some("dbo")
    ));
    assert_eq!(
        table(m, "Time Intelligence").kind(),
        TableKind::CalculationGroup
    );

    let r1 = &m.relationships[0];
    assert_eq!(r1.from_cardinality, Cardinality::One);
    assert!(!r1.is_active);
    assert!(r1.extra.contains_key("joinOnDateBehavior"));
    let r2 = &m.relationships[1];
    assert_eq!(r2.to_cardinality, Cardinality::Many);
    assert!(r2.is_bidirectional());
    assert!(r2.rely_on_referential_integrity);
    assert_eq!(
        r2.security_filtering_behavior,
        SecurityFilteringBehavior::BothDirections
    );

    assert_eq!(m.perspectives[0].name, "Sales Only");
    assert!(m.perspectives[0].extra.contains_key("tables"));
    let en = &m.cultures[0];
    assert!(
        en.linguistic_metadata
            .as_deref()
            .unwrap()
            .contains("\"DynamicImprovement\": \"HighConfidence\"")
    );
    let de = &m.cultures[1];
    assert!(
        de.linguistic_metadata
            .as_deref()
            .unwrap()
            .starts_with("<LinguisticSchema")
    );
    assert!(de.extra.contains_key("translations"));

    let role = m.role("region managers").unwrap();
    assert_eq!(role.model_permission.as_deref(), Some("read"));
    assert!(role.extra.contains_key("members"));
    let permission = &role.table_permissions[0];
    assert_eq!(permission.table, "Sales");
    assert_eq!(
        permission.filter_expression.as_deref(),
        Some("VAR me = USERPRINCIPALNAME()\nRETURN Sales[Amount] > 0")
    );
    assert!(permission.extra.contains_key("columnPermissions"));

    let threshold = m.expression("Threshold").unwrap();
    assert!(threshold.is_parameter());
    assert_eq!(threshold.parameter_type().as_deref(), Some("Number"));
    assert_eq!(threshold.expression, "1000");
    let dax = m.expression("DaxHelper").unwrap();
    assert_eq!(dax.kind, ExpressionKind::Dax);
    assert_eq!(dax.meta, None);
    assert_eq!(m.query_groups[0].folder, "Facts");
    assert_eq!(m.annotations.len(), 2);

    let codes: Vec<_> = loaded
        .diagnostics
        .iter()
        .map(|d| (d.severity, d.code.as_str()))
        .collect();
    assert_eq!(
        codes,
        [
            (Severity::Warning, "TMSL003"),
            (Severity::Warning, "TMSL002")
        ]
    );
    assert_eq!(
        loaded.diagnostics[0]
            .origin
            .as_ref()
            .unwrap()
            .pointer
            .as_deref(),
        Some("/model/tables/0/columns/4/type")
    );
}

#[test]
fn unknown_keys_survive_in_any_position_by_value() {
    let text = r#"{
      "futureFirst": 1,
      "compatibilityLevel": 1567,
      "model": {
        "tables": [
          {
            "zeta": {"a": [1, 2]},
            "name": "T",
            "columns": [{"x-future": null, "name": "C", "dataType": "string", "sourceColumn": "C"}],
            "alpha": true
          }
        ],
        "culture": "en-US"
      }
    }"#;
    assert_eq!(round_trip_value(text), value(text));
}

/// Found by `cargo fuzz` (`fuzz/fuzz_targets/tmsl.rs`): an unknown key with
/// no shadowed key before it was written at the `*` slot, and reading that
/// output back anchored it to an earlier shadow, so it moved on every save.
#[test]
fn writing_what_was_read_is_a_fixed_point() {
    for text in [
        "{  \"model\": {\n \"relationships\": [\n {  \"n\": \"Am\",\"isActive\": true,\n \"crossFilteringBehavior\": \")\"}]\n}\n}\n",
        r#"{"model": {"tables": [{"zeta": 1, "name": "T", "isHidden": false, "lineageTag": "x"}]}}"#,
    ] {
        let loaded = read_bim(text).unwrap();
        let once = write_text(&loaded.model, &loaded.write_options());
        let again = read_bim(&once).unwrap();
        assert_eq!(write_text(&again.model, &again.write_options()), once);
        assert_eq!(value(&once), value(text));
    }
}

#[test]
fn edits_win_over_shadows() {
    let loaded = read_bim(&data("features.bim")).unwrap();
    let mut model = loaded.model.clone();
    let sales = model.table_mut("Sales").unwrap();
    sales.measure_mut("Total Sales").unwrap().is_hidden = true;
    sales.measure_mut("Multi As String").unwrap().expression = "VAR y = 2\nRETURN y".into();
    model.relationships[1].cross_filtering_behavior = CrossFilter::OneDirection;
    model.table_mut("Legacy").unwrap().is_hidden = true;
    let out = to_json(&model, &loaded.write_options());
    let measures = &out["model"]["tables"][0]["measures"];
    assert_eq!(measures[0]["isHidden"], json!(true));
    // An edited text drops its source form and uses the default (array)
    // form; an unedited one keeps its source form.
    assert_eq!(measures[1]["expression"], json!(["VAR y = 2", "RETURN y"]));
    assert_eq!(measures[2]["expression"], json!(["1 + 1"]));
    let rel = out["model"]["relationships"][1].as_object().unwrap();
    assert!(!rel.contains_key("crossFilteringBehavior"));
    assert_eq!(rel["isActive"], json!(true));
    assert_eq!(out["model"]["tables"][1]["isHidden"], json!(true));
}

#[test]
fn calculation_groups_are_typed_and_round_trip() {
    let text = r#"{
      "compatibilityLevel": 1601,
      "model": {
        "tables": [{
          "name": "Time Calc",
          "calculationGroup": {
            "description": "Time intelligence",
            "precedence": 20,
            "calculationItems": [
              {"name": "Current", "expression": "SELECTEDMEASURE()", "ordinal": 0},
              {
                "name": "YOY%",
                "expression": ["DIVIDE(", "  SELECTEDMEASURE(), 2)"],
                "ordinal": 1,
                "formatStringDefinition": {"expression": "\"0.00%\""},
                "state": "ready"
              },
              {"name": "Odd", "expression": "1", "formatStringDefinition": {"expression": "\"0\"", "x": 1}}
            ],
            "multipleOrEmptySelectionExpression": {
              "expression": "SELECTEDMEASURE()",
              "formatStringDefinition": {"expression": "\"0.0\""}
            },
            "noSelectionExpression": {"expression": "BLANK()"},
            "futureProperty": true
          },
          "columns": [
            {"name": "Name", "dataType": "string", "sourceColumn": "Name", "sortByColumn": "Ordinal"},
            {"name": "Ordinal", "dataType": "int64", "isHidden": true, "sourceColumn": "Ordinal"}
          ],
          "partitions": [{"name": "P", "source": {"type": "calculationGroup"}}]
        }, {
          "name": "Broken",
          "calculationGroup": {"calculationItems": [7]}
        }]
      }
    }"#;
    let loaded = read_bim(text).unwrap();
    let t = table(&loaded.model, "Time Calc");
    assert_eq!(t.kind(), TableKind::CalculationGroup);
    assert!(!t.extra.contains_key("calculationGroup"));
    let g = t.calculation_group.as_ref().unwrap();
    assert_eq!(g.description.as_deref(), Some("Time intelligence"));
    assert_eq!(g.precedence, 20);
    assert!(g.extra.contains_key("futureProperty"));
    let yoy = g.item("yoy%").unwrap();
    assert_eq!(yoy.expression, "DIVIDE(\n  SELECTEDMEASURE(), 2)");
    assert_eq!(yoy.ordinal, Some(1));
    assert_eq!(yoy.format_string_definition.as_deref(), Some("\"0.00%\""));
    assert!(yoy.extra.contains_key("state"));
    // A format string definition with unknown keys is kept verbatim.
    let odd = g.item("Odd").unwrap();
    assert_eq!(odd.format_string_definition, None);
    assert!(odd.extra.contains_key("formatStringDefinition"));
    assert!(loaded.diagnostics.iter().any(|d| d.code == "TMSL006"));
    let multi = g.multiple_or_empty_selection_expression.as_ref().unwrap();
    assert_eq!(multi.format_string_definition.as_deref(), Some("\"0.0\""));
    assert_eq!(
        g.no_selection_expression.as_ref().unwrap().expression,
        "BLANK()"
    );
    // A calculation group that cannot be typed stays verbatim.
    let broken = table(&loaded.model, "Broken");
    assert!(broken.calculation_group.is_none());
    assert!(broken.extra.contains_key("calculationGroup"));
    assert_eq!(broken.kind(), TableKind::CalculationGroup);
    assert!(loaded.diagnostics.iter().any(|d| d.code == "TMSL007"));
    assert_eq!(round_trip_value(text), value(text));

    // Edits are written in the TOM form.
    let mut model = loaded.model.clone();
    let g = model
        .table_mut("Time Calc")
        .unwrap()
        .calculation_group
        .as_mut()
        .unwrap();
    g.precedence = 0;
    g.calculation_items[0].format_string_definition = Some("\"#,0\"".into());
    let out = to_json(&model, &loaded.write_options());
    let group = &out["model"]["tables"][0]["calculationGroup"];
    assert!(group.get("precedence").is_none());
    assert_eq!(
        group["calculationItems"][0]["formatStringDefinition"],
        json!({"expression": "\"#,0\""})
    );
}

// ---------------------------------------------------------------------------
// String-vs-array forms, defaults, absent keys

#[test]
fn text_forms_are_preserved() {
    let text = r#"{
      "compatibilityLevel": 1567,
      "model": {
        "description": "first\nsecond",
        "tables": [{
          "name": "T",
          "description": ["only line"],
          "measures": [
            {"name": "A", "expression": ["", "VAR x = 1", "RETURN x"]},
            {"name": "B", "expression": "VAR x = 1\r\nRETURN x"},
            {"name": "C", "expression": []},
            {"name": "D", "expression": "1", "formatString": ["0.0"]}
          ]
        }]
      }
    }"#;
    let model = read_bim(text).unwrap().model;
    assert_eq!(model.description.as_deref(), Some("first\nsecond"));
    let t = &model.tables[0];
    assert_eq!(t.description.as_deref(), Some("only line"));
    assert_eq!(t.measures[0].expression, "\nVAR x = 1\nRETURN x");
    assert!(t.measures[0].extra.is_empty());
    assert_eq!(t.measures[1].expression, "VAR x = 1\r\nRETURN x");
    assert_eq!(t.measures[2].expression, "");
    assert_eq!(t.measures[3].format_string.as_deref(), Some("0.0"));
    assert_eq!(round_trip_value(text), value(text));
}

#[test]
fn explicit_defaults_and_absent_keys_round_trip() {
    let text = r#"{
      "compatibilityLevel": 1567,
      "model": {
        "name": "Model",
        "discourageImplicitMeasures": false,
        "tables": [{
          "name": "T",
          "isHidden": false,
          "columns": [
            {"type": "data", "name": "C", "dataType": "automatic", "isKey": false, "sourceColumn": ""},
            {"type": "Calculated", "name": "D", "dataType": "Int64", "expression": "1"}
          ],
          "partitions": [
            {"name": "P", "mode": "default", "source": {"type": "M", "expression": "1"}},
            {"name": "Q", "mode": "Import", "source": {"type": "query", "query": "SELECT 1"}},
            {"name": "NoSource"}
          ],
          "measures": [],
          "hierarchies": [],
          "annotations": []
        }],
        "relationships": [{
          "name": "R", "fromCardinality": "many", "fromTable": "T", "fromColumn": "C",
          "toTable": "T", "toColumn": "D", "isActive": true, "crossFilteringBehavior": "oneDirection",
          "toCardinality": "one", "relyOnReferentialIntegrity": false,
          "securityFilteringBehavior": "OneDirection"
        }],
        "expressions": [
          {"name": "NoKind", "expression": "1"},
          {"name": "Spaced", "kind": "m", "expression": "\"x\"\n  meta [IsParameterQuery=true]  "}
        ],
        "roles": []
      }
    }"#;
    let loaded = read_bim(text).unwrap();
    let m = &loaded.model;
    let t = &m.tables[0];
    assert!(!t.is_hidden);
    assert_eq!(t.columns[0].data_type, DataType::Automatic);
    assert_eq!(t.columns[1].data_type, DataType::Int64);
    assert!(t.columns[1].is_calculated());
    assert_eq!(t.partitions[0].mode, PartitionMode::Default);
    assert_eq!(t.partitions[1].mode, PartitionMode::Import);
    assert!(matches!(t.partitions[0].source, PartitionSource::M { .. }));
    assert!(matches!(
        t.partitions[1].source,
        PartitionSource::Query { .. }
    ));
    let r = &m.relationships[0];
    assert_eq!(
        (r.from_cardinality.clone(), r.to_cardinality.clone()),
        (Cardinality::Many, Cardinality::One)
    );
    assert_eq!(m.expressions[0].kind, ExpressionKind::M);
    assert!(m.expressions[1].is_parameter());
    assert_eq!(round_trip_value(text), value(text));

    // The defaults are ordinary typed values: a fresh model writes none of them.
    let mut fresh = m.clone();
    strip_extras(&mut fresh);
    let out = to_json(&fresh, &WriteOptions::default());
    let rel = out["model"]["relationships"][0].as_object().unwrap();
    assert_eq!(
        rel.keys().collect::<Vec<_>>(),
        ["name", "fromTable", "fromColumn", "toTable", "toColumn"]
    );
    assert_eq!(out["model"]["expressions"][0]["kind"], json!("m"));
    assert!(out["model"].get("name").is_none());
}

fn strip_extras(model: &mut Model) {
    model.extra.clear();
    for t in &mut model.tables {
        t.extra.clear();
        t.columns.iter_mut().for_each(|c| c.extra.clear());
        t.partitions.iter_mut().for_each(|p| p.extra.clear());
    }
    model.relationships.iter_mut().for_each(|r| r.extra.clear());
    model.expressions.iter_mut().for_each(|e| e.extra.clear());
}

// ---------------------------------------------------------------------------
// Writing models that did not come from TMSL

#[test]
fn writes_a_model_built_in_code() {
    let mut t = Table::new("Sales");
    t.description = Some("Line one\nLine two".into());
    t.columns.push(Column::data("Amount", DataType::Decimal));
    t.columns.push(Column::calculated(
        "Double",
        DataType::Decimal,
        "Sales[Amount] * 2",
    ));
    t.measures.push(Measure::new(
        "Total",
        "VAR s = SUM(Sales[Amount])\nRETURN s",
    ));
    t.partitions.push(pbi_model::Partition {
        name: "Sales".into(),
        mode: PartitionMode::Import,
        source: PartitionSource::M {
            expression: "let\n    Source = 1\nin\n    Source".into(),
        },
        ..Default::default()
    });
    let model = Model {
        name: "Model".into(),
        culture: Some("en-US".into()),
        tables: vec![t, Table::new("Other")],
        relationships: vec![Relationship::new(
            "r",
            ColumnRef::new("Sales", "Amount"),
            ColumnRef::new("Other", "Amount"),
        )],
        expressions: vec![NamedExpression::m(
            "Url",
            r#""https://x" meta [IsParameterQuery=true, Type="Text"]"#,
        )],
        ..Model::default()
    };
    let out = value(&write_bim(&model, Some("Db"), Some(1567)));
    assert_eq!(
        out,
        json!({
            "name": "Db",
            "compatibilityLevel": 1567,
            "model": {
                "culture": "en-US",
                "tables": [
                    {
                        "name": "Sales",
                        "description": ["Line one", "Line two"],
                        "columns": [
                            {"name": "Amount", "dataType": "decimal", "sourceColumn": "Amount"},
                            {"type": "calculated", "name": "Double", "dataType": "decimal", "expression": "Sales[Amount] * 2"}
                        ],
                        "partitions": [
                            {"name": "Sales", "mode": "import", "source": {"type": "m", "expression": ["let", "    Source = 1", "in", "    Source"]}}
                        ],
                        "measures": [
                            {"name": "Total", "expression": ["VAR s = SUM(Sales[Amount])", "RETURN s"]}
                        ]
                    },
                    {"name": "Other"}
                ],
                "relationships": [
                    {"name": "r", "fromTable": "Sales", "fromColumn": "Amount", "toTable": "Other", "toColumn": "Amount"}
                ],
                "expressions": [
                    {"name": "Url", "kind": "m", "expression": "\"https://x\" meta [IsParameterQuery=true, Type=\"Text\"]"}
                ]
            }
        })
    );
    let back = read_bim(&write_bim(&model, Some("Db"), Some(1567)))
        .unwrap()
        .model;
    assert_eq!(back.tables, model.tables);
    assert_eq!(back.relationships, model.relationships);
    assert_eq!(back.expressions, model.expressions);
}

// ---------------------------------------------------------------------------
// Layout detection, encodings and origins

#[test]
fn pbip_alphabetical_layout_round_trips_byte_identically() {
    let text = "{\r\n  \"compatibilityLevel\": 1550,\r\n  \"model\": {\r\n    \"annotations\": [\r\n      {\r\n        \"name\": \"PBI_QueryOrder\",\r\n        \"value\": \"[]\"\r\n      }\r\n    ],\r\n    \"culture\": \"en-US\",\r\n    \"tables\": [\r\n      {\r\n        \"name\": \"T\",\r\n        \"columns\": [\r\n          {\r\n            \"name\": \"X\",\r\n            \"dataType\": \"string\",\r\n            \"isNameInferred\": true,\r\n            \"sourceColumn\": \"[X]\",\r\n            \"summarizeBy\": \"none\",\r\n            \"type\": \"calculatedTableColumn\"\r\n          }\r\n        ],\r\n        \"lineageTag\": \"t\",\r\n        \"partitions\": [\r\n          {\r\n            \"name\": \"T\",\r\n            \"mode\": \"import\",\r\n            \"source\": {\r\n              \"expression\": \"ROW(\\\"X\\\", 1)\",\r\n              \"type\": \"calculated\"\r\n            }\r\n          }\r\n        ]\r\n      }\r\n    ]\r\n  }\r\n}";
    let loaded = read_bim(text).unwrap();
    assert_eq!(loaded.format.key_order, KeyOrder::Alphabetical);
    assert_eq!(loaded.format.line_ending, LineEnding::CrLf);
    assert!(!loaded.format.trailing_newline);
    assert_eq!(loaded.database_name, None);
    assert_eq!(write_text(&loaded.model, &loaded.write_options()), text);
}

#[test]
fn data_model_schema_utf16_is_decoded_and_rewritten() {
    let text = data("acme.bim");
    let expected = read_bim(&text).unwrap();
    for (encoding, bom) in [
        (TextEncoding::Utf16Le, false),
        (TextEncoding::Utf16Le, true),
        (TextEncoding::Utf16Be, true),
        (TextEncoding::Utf16Be, false),
        (TextEncoding::Utf8, true),
    ] {
        let bytes = encode_text(&text, encoding, bom);
        let loaded = read_data_model_schema(&bytes).unwrap();
        assert_eq!(loaded.model, expected.model, "{encoding:?} bom={bom}");
        assert_eq!((loaded.format.encoding, loaded.format.bom), (encoding, bom));
        assert_eq!(write_bytes(&loaded.model, &loaded.write_options()), bytes);
    }

    // PBIT default: UTF-16LE without BOM.
    let bytes = write_data_model_schema(&expected.model, Some("0a1b2c3d-guid"), Some(1601));
    assert_eq!(&bytes[..4], b"{\0\n\0");
    let back = read_data_model_schema(&bytes).unwrap();
    assert_eq!(back.database_name.as_deref(), Some("0a1b2c3d-guid"));
    assert_eq!(back.format, TmslFormat::data_model_schema());
    assert_eq!(back.model.tables, expected.model.tables);

    // `read_bim` tolerates a BOM character.
    let with_bom = format!("\u{FEFF}{text}");
    assert!(read_bim(&with_bom).unwrap().format.bom);
}

#[test]
fn origins_point_into_the_file_when_a_path_is_given() {
    let options = ReadOptions {
        source_path: Some("Model.SemanticModel/model.bim".into()),
    };
    let model = read_text(&data("acme.bim"), &options).unwrap().model;
    let origin = model.tables[1].columns[2].origin.as_ref().unwrap();
    assert_eq!(origin.path, "Model.SemanticModel/model.bim");
    assert_eq!(origin.pointer.as_deref(), Some("/model/tables/1/columns/2"));
    assert_eq!(
        model.expressions[0].origin.as_ref().unwrap().to_string(),
        "Model.SemanticModel/model.bim#/model/expressions/0"
    );
    assert!(
        read_bim(&data("acme.bim")).unwrap().model.tables[0]
            .origin
            .is_none()
    );
}

// ---------------------------------------------------------------------------
// Errors and hostile input

#[test]
fn malformed_input_is_rejected_with_errors() {
    let cases: &[(&str, &str)] = &[
        ("", "invalid JSON"),
        (r#"{"model": {"tables": [}"#, "invalid JSON"),
        (r#"{"model": {}} trailing"#, "invalid JSON"),
        ("[1, 2]", "expected an object at ``, found an array"),
        (r#"{"compatibilityLevel": 1567}"#, "no `model`"),
        (r#"{"model": []}"#, "expected an object at `/model`"),
        (
            r#"{"model": {"tables": {}}}"#,
            "expected an array at `/model/tables`",
        ),
        (
            r#"{"model": {"tables": [7]}}"#,
            "expected an object at `/model/tables/0`",
        ),
        (
            r#"{"model": {"tables": [{"name": "T", "columns": [{"name": 5}]}]}}"#,
            "at `/model/tables/0/columns/0/name`, found a number",
        ),
        (
            r#"{"model": {"tables": [{"name": "T", "isHidden": "yes"}]}}"#,
            "expected a boolean at `/model/tables/0/isHidden`",
        ),
        (
            r#"{"model": {"tables": [{"name": "T", "measures": [{"name": "M", "expression": ["a", 1]}]}]}}"#,
            "expected a string at `/model/tables/0/measures/0/expression/1`",
        ),
        (
            r#"{"compatibilityLevel": "1567", "model": {}}"#,
            "expected an integer",
        ),
        (
            r#"{"model": {"relationships": [{"crossFilteringBehavior": 2}]}}"#,
            "expected a string",
        ),
        (
            r#"{"model": {"tables": [{"partitions": [{"source": "m"}]}]}}"#,
            "expected an object at `/model/tables/0/partitions/0/source`",
        ),
    ];
    for (input, message) in cases {
        let err = read_bim(input).unwrap_err();
        assert!(
            err.to_string().contains(message),
            "{input}: {err} (expected {message:?})"
        );
    }
    assert!(matches!(read_bim("{"), Err(TmslError::Json(_))));
}

#[test]
fn deep_nesting_is_an_error_not_a_stack_overflow() {
    let depth = 100_000;
    let text = format!(
        r#"{{"model": {{"x": {}{}}}}}"#,
        "[".repeat(depth),
        "]".repeat(depth)
    );
    assert!(matches!(read_bim(&text), Err(TmslError::Json(_))));
    let objects = format!("{}{}", r#"{"a":"#.repeat(depth), "}".repeat(depth));
    assert!(read_bim(&objects).is_err());
}

#[test]
fn invalid_encodings_are_errors() {
    assert!(matches!(
        read_data_model_schema(b"{\"model\": \xFF}"),
        Err(TmslError::Encoding { .. })
    ));
    let mut odd = encode_text(r#"{"model": {}}"#, TextEncoding::Utf16Le, false);
    odd.push(b'x');
    assert!(matches!(
        read_data_model_schema(&odd),
        Err(TmslError::Encoding { .. })
    ));
    let lone_surrogate = [b'{', 0, 0x00, 0xD8, b'}', 0];
    assert!(read_data_model_schema(&lone_surrogate).is_err());
}

#[test]
fn duplicate_keys_are_reported() {
    let loaded =
        read_bim(r#"{"model": {"culture": "en-US", "culture": "de-DE", "tables": []}}"#).unwrap();
    assert_eq!(loaded.model.culture.as_deref(), Some("de-DE"));
    assert_eq!(loaded.diagnostics.len(), 1);
    assert_eq!(loaded.diagnostics[0].code, "TMSL001");
    assert_eq!(
        loaded.diagnostics[0]
            .origin
            .as_ref()
            .unwrap()
            .pointer
            .as_deref(),
        Some("/model/culture")
    );
}

#[test]
fn mutated_inputs_never_panic() {
    let base = encode_text(&data("features.bim"), TextEncoding::Utf16Le, false);
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for i in 0..400 {
        let mut bytes = base.clone();
        let cut = (next() as usize) % bytes.len();
        match i % 4 {
            0 => bytes.truncate(cut),
            1 => bytes[cut] = next() as u8,
            2 => {
                let len = (next() as usize % 64).min(bytes.len() - cut);
                bytes.drain(cut..cut + len);
            }
            _ => {
                let at = cut & !1;
                let chunk = bytes[at..(at + 32).min(bytes.len())].to_vec();
                bytes.splice(at..at, chunk);
            }
        }
        if let Ok(loaded) = read_bytes(&bytes, &ReadOptions::default()) {
            let written = write_bytes(&loaded.model, &loaded.write_options());
            let again = read_data_model_schema(&written).unwrap();
            assert_eq!(
                to_json(&again.model, &again.write_options()),
                to_json(&loaded.model, &loaded.write_options())
            );
        }
    }
}

/// `object.key` of every `extra` entry in `acme.bim`.
const EXPECTED_ACME_EXTRAS: &[&str] = &[
    "model.dataAccessOptions",
    "Orders[Sales (Scaled)].formatStringDefinition",
];
