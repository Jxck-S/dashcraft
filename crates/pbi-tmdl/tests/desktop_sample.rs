//! A Desktop-style model exercising most TMDL features: lineage tags,
//! annotations, unmodeled properties and objects, query groups, roles,
//! cultures, perspectives, functions, calculated tables, calculation groups,
//! multi-line and fenced expressions. The sample is written in canonical
//! form, so the writer must reproduce it byte for byte.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use pbi_model::{
    ColumnKind, PartitionSource, PropValue, RawObject, Severity, TableKind, names::name_key,
};
use pbi_tmdl::{LoadedModel, clear_origins, load_model, parse_file, write_model};
use pretty_assertions::assert_eq;

mod common;

use common::DESKTOP_SAMPLE as FILES;

fn files() -> Vec<(String, String)> {
    FILES
        .iter()
        .map(|(p, t)| ((*p).to_owned(), (*t).to_owned()))
        .collect()
}

fn load() -> LoadedModel {
    load_model(&files()).unwrap()
}

fn obj(value: Option<&PropValue>) -> &RawObject {
    match value {
        Some(PropValue::Object(o)) => o,
        other => panic!("expected an object, got {other:?}"),
    }
}

#[test]
fn desktop_style_model_maps_typed_and_untyped_properties() {
    let loaded = load();
    for f in &loaded.files {
        let original = FILES.iter().find(|(p, _)| *p == f.path).unwrap().1;
        assert_eq!(f.to_string(), original, "{}", f.path);
    }
    let problems: Vec<_> = loaded
        .diagnostics
        .iter()
        .filter(|d| d.severity != Severity::Info)
        .collect();
    assert!(problems.is_empty(), "{problems:#?}");
    assert!(
        loaded
            .diagnostics
            .iter()
            .any(|d| d.message.contains("`isAvailableInMdx` on column"))
    );

    let m = &loaded.model;
    assert!(m.discourage_implicit_measures);
    let dao = obj(m.extra.get("dataAccessOptions"));
    assert_eq!(
        dao.properties.get("legacyRedirects"),
        Some(&PropValue::Bool(true))
    );
    assert_eq!(m.query_groups[0].folder, "Parameters");
    assert_eq!(m.query_groups[0].annotations[0].value, "0");
    assert_eq!(m.annotations[0].name, "PBI_QueryOrder");
    assert_eq!(m.annotations[0].value, "[\"Server\",\"Sales\"]");
    let function = obj(m.extra.get("function"));
    assert_eq!(function.name.as_deref(), Some("AddOne"));
    assert_eq!(function.expression.as_deref(), Some("(x: int64) => x + 1"));

    let server = m.expression("Server").unwrap();
    assert!(server.is_parameter());
    assert_eq!(server.query_group.as_deref(), Some("Parameters"));
    assert_eq!(
        m.expression("Sales").unwrap().expression,
        "let\n    Source = Sql.Database(Server, \"db\")\nin\n    Source"
    );

    let a1 = &m.relationships[0];
    assert!(!a1.is_active);
    assert_eq!(
        a1.extra.get("joinOnDateBehavior"),
        Some(&PropValue::Text("datePartOnly".into()))
    );
    assert_eq!(m.relationships[1].from.column, "Customer Key");

    let names: Vec<_> = m.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["Sales", "Calendar", "Time Intelligence"]);
    let sales = m.table("Sales").unwrap();
    assert_eq!(sales.lineage_tag.as_deref(), Some("t1"));
    let ytd = sales.measure("YTD").unwrap();
    assert_eq!(
        ytd.description.as_deref(),
        Some("Year to date.\nSecond line.")
    );
    assert_eq!(
        ytd.expression,
        "VAR d = MAX('Calendar'[Date])\n\nRETURN\n    TOTALYTD([Sales Amount], 'Calendar'[Date])"
    );
    assert_eq!(
        ytd.extra.get("formatStringDefinition"),
        Some(&PropValue::Expression(
            "SELECTEDMEASUREFORMATSTRING()".into()
        ))
    );
    assert_eq!(
        sales.measure("Padded").unwrap().expression,
        "  keep leading spaces  "
    );
    assert_eq!(
        sales
            .measure("Sales Amount")
            .unwrap()
            .format_string
            .as_deref(),
        Some("\\$#,0.00;(\\$#,0.00);\\$#,0.00")
    );
    let order_date = sales.column("OrderDate").unwrap();
    assert_eq!(
        order_date.extra.get("isAvailableInMdx"),
        Some(&PropValue::Text("false".into()))
    );
    assert_eq!(
        order_date.extra.get("changedProperty"),
        Some(&PropValue::Expression("IsHidden".into()))
    );
    let variation = obj(order_date.extra.get("variation"));
    assert_eq!(variation.name.as_deref(), Some("Variation"));
    assert_eq!(order_date.annotations[0].value, "Automatic");
    let key = sales.column("Customer Key").unwrap();
    assert!(key.is_key && key.is_hidden);
    assert_eq!(key.sort_by_column.as_deref(), Some("Order Date"));
    assert_eq!(sales.hierarchies[0].levels[0].column, "Year");
    assert_eq!(
        sales.partitions[0].query_group.as_deref(),
        Some("Parameters")
    );

    let calendar = m.table("Calendar").unwrap();
    assert_eq!(calendar.kind(), TableKind::CalculatedTable);
    assert_eq!(calendar.calculated_expression(), Some("CALENDARAUTO()"));
    assert!(matches!(
        calendar.column("Date").unwrap().kind,
        ColumnKind::CalculatedTableColumn { .. }
    ));
    assert_eq!(
        calendar.column("Year").unwrap().expression(),
        Some("YEAR('Calendar'[Date])")
    );

    let ti = m.table("Time Intelligence").unwrap();
    assert_eq!(ti.kind(), TableKind::CalculationGroup);
    assert!(matches!(
        &ti.partitions[0].source,
        PartitionSource::Unknown { kind, .. } if kind == "calculationGroup"
    ));
    assert!(!ti.extra.contains_key("calculationGroup"));
    let group = ti.calculation_group.as_ref().unwrap();
    assert_eq!(group.precedence, 1);
    let names: Vec<_> = group
        .calculation_items
        .iter()
        .map(|i| (i.name.as_str(), i.ordinal))
        .collect();
    assert_eq!(names, [("Current", None), ("YTD", Some(1))]);
    assert_eq!(group.calculation_items[0].expression, "SELECTEDMEASURE()");
    assert_eq!(
        group.calculation_items[1].expression,
        "CALCULATE(\n    SELECTEDMEASURE(),\n    DATESYTD('Calendar'[Date])\n)"
    );
    assert_eq!(
        ti.calculation_group_column().map(|c| c.name.as_str()),
        Some("Time Calculation")
    );

    let roles: Vec<_> = m.roles.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(roles, ["Readers", "Admins"]);
    let readers = m.role("Readers").unwrap();
    assert_eq!(readers.model_permission.as_deref(), Some("read"));
    assert_eq!(readers.table_permissions[0].table, "Sales");
    assert_eq!(
        readers.table_permissions[0].filter_expression.as_deref(),
        Some("Sales[Region] = \"West\"")
    );
    assert!(readers.extra.contains_key("member"));

    let culture = &m.cultures[0];
    assert_eq!(culture.name, "en-US");
    assert_eq!(
        culture.linguistic_metadata.as_deref(),
        Some("{\n  \"Version\": \"1.0.0\",\n  \"Language\": \"en-US\"\n}")
    );
    assert_eq!(m.perspectives[0].name, "Sales View");
    assert!(m.perspectives[0].extra.contains_key("perspectiveTable"));
}

#[test]
fn canonical_sample_is_a_writer_fixpoint() {
    let loaded = load();
    let written: BTreeMap<String, String> = write_model(&loaded.model, loaded.database.as_ref())
        .into_iter()
        .collect();
    let original: BTreeMap<String, String> = files().into_iter().collect();
    assert_eq!(
        written.keys().collect::<Vec<_>>(),
        original.keys().collect::<Vec<_>>()
    );
    for (path, text) in &original {
        assert_eq!(&written[path], text, "{path}");
    }
}

#[test]
fn crlf_and_space_indentation_load_the_same_model() {
    let converted: Vec<(String, String)> = files()
        .into_iter()
        .map(|(p, t)| (p, t.replace('\t', "    ").replace('\n', "\r\n")))
        .collect();
    let loaded = load_model(&converted).unwrap();
    for (f, (_, text)) in loaded.files.iter().zip(&converted) {
        assert_eq!(&f.to_string(), text, "{}", f.path);
    }
    let mut a = loaded.model;
    let mut b = load().model;
    clear_origins(&mut a);
    clear_origins(&mut b);
    assert_eq!(a, b);
}

#[test]
fn edits_round_trip_and_unknown_content_is_preserved() {
    let mut loaded = load();
    let sales = loaded.model.table_mut("Sales").unwrap();
    sales.measures[0].expression = "  SUM(Sales[Amount])\n  + 0".into();
    sales.measures[1].expression = "SUM(Sales[Amount])\n\t+ 1".into();
    sales.measures[0].description = Some("Edited.".into());
    sales.columns[0].name = "Order Date".into();
    let written = loaded.write();
    let changed: Vec<_> = written
        .iter()
        .filter(|(p, t)| {
            FILES
                .iter()
                .find(|(fp, _)| fp == p)
                .is_none_or(|(_, ft)| ft != t)
        })
        .map(|(p, _)| p.as_str())
        .collect();
    assert_eq!(changed, ["tables/Sales.tmdl"]);
    let reloaded = load_model(&written).unwrap();
    let sales = reloaded.model.table("Sales").unwrap();
    assert_eq!(sales.measures[0].expression, "  SUM(Sales[Amount])\n  + 0");
    assert_eq!(sales.measures[1].expression, "SUM(Sales[Amount])\n\t+ 1");
    assert_eq!(sales.measures[0].description.as_deref(), Some("Edited."));
    let col = sales.column("Order Date").unwrap();
    assert!(col.extra.contains_key("variation"));
    let text = &written
        .iter()
        .find(|(p, _)| p == "tables/Sales.tmdl")
        .unwrap()
        .1;
    assert!(text.contains(
        "\tmeasure 'Sales Amount' = ```\n\t\t\t  SUM(Sales[Amount])\n\t\t\t  + 0\n\t\t\t```\n"
    ));
    assert!(text.contains("\tmeasure YTD =\n\t\t\tSUM(Sales[Amount])\n\t\t\t\t+ 1\n"));
    assert!(text.contains("\tcolumn 'Order Date'\n"));
}

#[test]
fn spans_point_at_source_positions() {
    let file = parse_file("tables/Sales.tmdl", FILES[5].1).unwrap();
    let table = &file.nodes[0];
    assert_eq!((table.line_number(), table.column()), (2, 1));
    let ytd = &table.children[2];
    assert_eq!(ytd.name().as_deref(), Some("YTD"));
    assert_eq!(
        (ytd.line_number(), ytd.column(), ytd.last_line_number()),
        (12, 2, 18)
    );
    assert_eq!(ytd.leading.len(), 3);
    assert_eq!(name_key(ytd.keyword()), "measure");
}
