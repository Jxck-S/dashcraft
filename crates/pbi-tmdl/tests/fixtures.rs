//! Tests against the committed Acme Outfitters example
//! (`reference/examples/acme-outfitters/AcmeOutfitters.SemanticModel/definition`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pbi_model::{
    Cardinality, ColumnKind, ColumnRef, CrossFilter, DataType, Model, PartitionMode,
    PartitionSource, Severity, SummarizeBy, TableKind,
};
use pbi_tmdl::{
    LoadedModel, clear_origins, load_model, load_model_from_dir, parse_file, write_model,
};
use pretty_assertions::assert_eq;

fn reference_root() -> PathBuf {
    std::env::var_os("PBIVIEW_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
}

fn definition_dir() -> Option<PathBuf> {
    let dir =
        reference_root().join("examples/acme-outfitters/AcmeOutfitters.SemanticModel/definition");
    if dir.join("model.tmdl").is_file() {
        Some(dir)
    } else {
        println!("SKIP: {} missing", dir.display());
        None
    }
}

/// `(path relative to definition/, text)`, sorted by path.
fn fixture_files(dir: &Path) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "tmdl"))
        .map(|e| {
            let rel = e
                .path()
                .strip_prefix(dir)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            (rel, std::fs::read_to_string(e.path()).unwrap())
        })
        .collect();
    files.sort();
    files
}

fn load_fixture() -> Option<(Vec<(String, String)>, LoadedModel)> {
    let dir = definition_dir()?;
    let files = fixture_files(&dir);
    let loaded = load_model(&files).unwrap();
    Some((files, loaded))
}

fn without_origins(mut model: Model) -> Model {
    clear_origins(&mut model);
    model
}

#[test]
fn every_fixture_tmdl_file_round_trips_byte_for_byte() {
    let root = reference_root();
    if !root.is_dir() {
        println!("SKIP: {} missing", root.display());
        return;
    }
    let mut count = 0;
    for entry in walkdir::WalkDir::new(&root)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !path
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("tmdl"))
        {
            continue;
        }
        let text = std::fs::read_to_string(path).unwrap();
        let file = parse_file(&path.display().to_string(), &text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(file.to_string(), text, "{}", path.display());
        count += 1;
    }
    if count == 0 {
        println!("SKIP: no .tmdl files under {}", root.display());
    }
}

#[test]
fn fixture_maps_to_the_expected_model() {
    let Some((_, loaded)) = load_fixture() else {
        return;
    };
    let problems: Vec<_> = loaded
        .diagnostics
        .iter()
        .filter(|d| d.severity != Severity::Info)
        .collect();
    assert!(problems.is_empty(), "{problems:#?}");
    let m = &loaded.model;

    assert_eq!(
        loaded.database.as_ref().unwrap().compatibility_level,
        Some(1601)
    );
    assert_eq!(m.compatibility_level(), Some(1601));
    assert_eq!(m.name, "Model");
    assert_eq!(m.culture.as_deref(), Some("en-US"));
    assert_eq!(m.source_query_culture.as_deref(), Some("en-US"));
    assert_eq!(
        m.default_power_bi_data_source_version.as_deref(),
        Some("powerBI_V3")
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

    let orders = m.table("Orders").unwrap();
    assert!(
        orders
            .description
            .as_deref()
            .unwrap()
            .starts_with("One row per order line (synthetic).")
    );
    let origin = orders.origin.as_ref().unwrap();
    assert_eq!(
        (origin.path.as_str(), origin.line),
        ("tables/Orders.tmdl", Some(2))
    );
    let measures: Vec<_> = orders.measures.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(
        measures,
        [
            "Total Sales",
            "Total Cost",
            "Gross Margin",
            "Margin %",
            "Sales (Scaled)",
            "Completed Sales",
            "Shipped Sales",
            "Sales All Stores",
            "Store Share",
            "Order Count",
            "Units Sold",
            "Active Customers",
            "Avg Order Value",
            "Revenue per Unit",
            "Last Order Date",
            "Unknown Product Orders",
            "Sales YTD",
            "Sales PY",
            "Sales YoY %",
            "Sales Prev Month",
        ]
    );
    let measure = |name: &str| {
        let m = orders.measure(name).unwrap();
        (m.expression.as_str(), m.format_string.as_deref())
    };
    assert_eq!(
        measure("Total Sales"),
        ("SUM(Orders[Line Amount])", Some("\\$#,0.00"))
    );
    assert_eq!(
        measure("Margin %"),
        ("DIVIDE([Gross Margin], [Total Sales])", Some("0.0%"))
    );
    assert_eq!(
        measure("Shipped Sales"),
        (
            "CALCULATE([Total Sales], USERELATIONSHIP(Orders[Ship Date], 'Date'[Date]))",
            Some("\\$#,0.00")
        )
    );
    assert_eq!(
        measure("Avg Order Value"),
        (
            "AVERAGEX(\n    VALUES(Orders[Order ID]),\n    CALCULATE(SUM(Orders[Line Amount]))\n)",
            Some("\\$#,0.00")
        )
    );
    assert_eq!(
        measure("Sales YoY %"),
        (
            "DIVIDE([Total Sales] - [Sales PY], [Sales PY])",
            Some("+0.0%;-0.0%;0.0%")
        )
    );
    assert_eq!(
        measure("Last Order Date"),
        ("MAX(Orders[Order Date])", Some("yyyy-mm-dd"))
    );
    let total = orders.measure("Total Sales").unwrap();
    assert_eq!(
        total.description.as_deref(),
        Some("Sum of the order line amounts.")
    );
    assert_eq!(total.display_folder.as_deref(), Some("Sales"));
    assert!(orders.measure("Sales All Stores").unwrap().is_hidden);
    assert!(
        orders
            .measure("Sales (Scaled)")
            .unwrap()
            .format_string_definition()
            .is_some_and(|d| d.starts_with("IF(ABS(SELECTEDMEASURE()) >= 1000000"))
    );

    let returned = orders.column("Is Returned").unwrap();
    assert_eq!(
        returned.expression(),
        Some("IF(ISEMPTY(RELATEDTABLE(Returns)), \"No\", \"Yes\")")
    );
    assert_eq!(returned.data_type, DataType::String);
    let cost = orders.column("Line Cost").unwrap();
    assert_eq!(
        cost.expression(),
        Some("Orders[Quantity] * RELATED(Products[Unit Cost])")
    );
    assert_eq!(cost.data_type, DataType::Double);
    assert_eq!(cost.summarize_by, Some(SummarizeBy::Sum));
    assert_eq!(cost.format_string.as_deref(), Some("\\$#,0.00"));

    let key = orders.column("Customer ID").unwrap();
    assert!(key.is_hidden);
    assert_eq!(key.source_column(), Some("Customer ID"));
    assert_eq!(key.summarize_by, Some(SummarizeBy::None));
    assert!(m.column("Stores", "Manager Email").unwrap().is_hidden);
    assert!(!orders.column("Order ID").unwrap().is_hidden);
    assert_eq!(
        m.column("Last Refreshed", "Last Refreshed (UTC)")
            .unwrap()
            .source_column(),
        Some("Last Refreshed (UTC)")
    );
    assert_eq!(orders.columns.len(), 15);

    let never_sold = m.table("Products Never Sold").unwrap();
    assert_eq!(never_sold.kind(), TableKind::CalculatedTable);
    let col = &never_sold.columns[0];
    assert_eq!(
        col.kind,
        ColumnKind::CalculatedTableColumn {
            source_column: "[Product ID]".into()
        }
    );
    assert!(col.is_name_inferred);
    assert_eq!(never_sold.columns.len(), 4);
    assert!(never_sold.calculated_expression().unwrap().starts_with(
        "VAR SoldProducts =\n    SELECTCOLUMNS(SUMMARIZE(Orders, Orders[Product ID])"
    ));
    assert!(
        never_sold
            .calculated_expression()
            .unwrap()
            .ends_with("\"List Price\", Products[List Price]\n    )")
    );
    assert_eq!(m.table("Date").unwrap().kind(), TableKind::CalculatedTable);
    assert_eq!(
        m.table("Time Calc").unwrap().kind(),
        TableKind::CalculationGroup
    );

    for name in [
        "Orders",
        "Customers",
        "Products",
        "Stores",
        "Targets",
        "Returns",
        "Web Sessions",
        "Last Refreshed",
    ] {
        let t = m.table(name).unwrap();
        assert_eq!(t.kind(), TableKind::Regular, "{name}");
        let p = &t.partitions[0];
        assert_eq!(p.name, name);
        assert_eq!(p.mode, PartitionMode::Import);
        assert!(matches!(p.source, PartitionSource::M { .. }), "{name}");
        assert!(
            p.source
                .expression()
                .unwrap()
                .starts_with("let\n    Source = ")
        );
    }
    assert!(m.table("Last Refreshed").unwrap().is_hidden);
    assert!(!orders.is_hidden);

    // The pbi-m test scripts are copies of these partitions and expressions.
    let m_data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../pbi-m/tests/data");
    let partition = |table: &str| {
        m.table(table).unwrap().partitions[0]
            .source
            .expression()
            .unwrap()
            .to_owned()
    };
    let expression = |name: &str| m.expression(name).unwrap().full_expression();
    for (file, expr) in [
        ("orders.m", partition("Orders")),
        ("customers.m", partition("Customers")),
        ("stores.m", partition("Stores")),
        ("last_refreshed.m", partition("Last Refreshed")),
        ("storage_container_url.m", expression("StorageContainerUrl")),
        ("fn_clean_key.m", expression("fnCleanKey")),
        ("fn_parse_names.m", expression("fnParseNames")),
    ] {
        let Ok(script) = std::fs::read_to_string(m_data.join(file)) else {
            println!("SKIP: pbi-m/tests/data/{file} missing");
            continue;
        };
        assert_eq!(expr, script.trim_end(), "{file}");
    }

    assert_eq!(m.relationships.len(), 8);
    let r = &m.relationships[0];
    assert_eq!(r.name, "c79d4066-724d-5669-b57b-f6bf11da6f40");
    assert_eq!(r.from, ColumnRef::new("Orders", "Customer ID"));
    assert_eq!(r.to, ColumnRef::new("Customers", "Customer ID"));
    assert_eq!(r.cross_filtering_behavior, CrossFilter::BothDirections);
    assert!(r.is_active);
    assert_eq!(r.origin.as_ref().unwrap().line, Some(1));
    let ship = &m.relationships[4];
    assert_eq!(ship.from, ColumnRef::new("Orders", "Ship Date"));
    assert!(!ship.is_active);
    let one_to_one = &m.relationships[5];
    assert_eq!(
        (&one_to_one.from_cardinality, &one_to_one.to_cardinality),
        (&Cardinality::One, &Cardinality::One)
    );
    let many_to_many = &m.relationships[6];
    assert_eq!(many_to_many.to, ColumnRef::new("Stores", "Region"));
    assert_eq!(
        (&many_to_many.from_cardinality, &many_to_many.to_cardinality),
        (&Cardinality::Many, &Cardinality::Many)
    );

    let blob = m.expression("StorageContainerUrl").unwrap();
    assert!(blob.is_parameter());
    assert_eq!(
        blob.expression,
        "\"https://acmeoutfittersdemo.blob.core.windows.net/sample-data\""
    );
    assert_eq!(
        blob.meta.as_deref(),
        Some("[IsParameterQuery=true, Type=\"Text\", IsParameterQueryRequired=true]")
    );
    assert!(
        blob.description
            .as_deref()
            .unwrap()
            .starts_with("Fictional Azure Blob container")
    );
    assert_eq!(blob.origin.as_ref().unwrap().line, Some(3));

    let roles: Vec<_> = m.roles.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(roles, ["Region West", "Store Manager"]);
    assert_eq!(
        m.roles[1].table_permissions[0].filter_expression.as_deref(),
        Some("Stores[Manager Email] = USERPRINCIPALNAME()")
    );
}

/// The canonical writer reproduces every fixture file byte for byte except
/// one line: the example generator spells out the default `toCardinality: one`
/// of the one-to-one relationship, while this writer omits default values.
#[test]
fn canonical_writer_reproduces_the_fixture() {
    let Some((files, loaded)) = load_fixture() else {
        return;
    };
    let written: BTreeMap<String, String> = write_model(&loaded.model, loaded.database.as_ref())
        .into_iter()
        .collect();
    let original: BTreeMap<String, String> = files.into_iter().collect();
    assert_eq!(
        written.keys().collect::<Vec<_>>(),
        original.keys().collect::<Vec<_>>()
    );
    for (path, text) in &original {
        let expected = if path == "relationships.tmdl" {
            text.replace(
                "\tfromCardinality: one\n\ttoCardinality: one\n",
                "\tfromCardinality: one\n",
            )
        } else {
            text.clone()
        };
        assert_ne!(path == "relationships.tmdl", &expected == text, "{path}");
        assert_eq!(written[path], expected, "{path}");
    }
}

#[test]
fn semantic_round_trip_through_the_writer() {
    let Some((_, loaded)) = load_fixture() else {
        return;
    };
    let written = write_model(&loaded.model, loaded.database.as_ref());
    let reloaded = load_model(&written).unwrap();
    assert_eq!(
        without_origins(reloaded.model),
        without_origins(loaded.model.clone())
    );
    assert_eq!(
        reloaded.database.map(|mut d| {
            d.origin = None;
            d
        }),
        loaded.database.map(|mut d| {
            d.origin = None;
            d
        })
    );
}

#[test]
fn preserving_writer_is_byte_identical_and_rewrites_only_edited_files() {
    let Some((files, mut loaded)) = load_fixture() else {
        return;
    };
    let original: BTreeMap<String, String> = files.into_iter().collect();
    let written: BTreeMap<String, String> = loaded.write().into_iter().collect();
    assert_eq!(written, original);

    let orders = loaded.model.table_mut("Orders").unwrap();
    orders.measures[0].format_string = Some("#,0".into());
    let written: BTreeMap<String, String> = loaded.write().into_iter().collect();
    for (path, text) in &original {
        if path == "tables/Orders.tmdl" {
            assert_ne!(&written[path], text);
            assert!(written[path].contains("\t\tformatString: #,0\n"));
        } else {
            assert_eq!(&written[path], text, "{path}");
        }
    }
    let reloaded = load_model(&written.into_iter().collect::<Vec<_>>()).unwrap();
    assert_eq!(
        reloaded
            .model
            .measure("Orders", "Total Sales")
            .unwrap()
            .format_string
            .as_deref(),
        Some("#,0")
    );
}

#[test]
fn loads_from_definition_or_semantic_model_folder() {
    let Some(dir) = definition_dir() else {
        return;
    };
    let a = load_model_from_dir(&dir).unwrap();
    let b = load_model_from_dir(dir.parent().unwrap()).unwrap();
    assert_eq!(a.model, b.model);
    assert_eq!(a.files.len(), 17);
    assert!(a.files.iter().any(|f| f.path == "tables/Web Sessions.tmdl"));
    assert!(a.files.iter().any(|f| f.path == "roles/Store Manager.tmdl"));
}
