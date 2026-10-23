//! Storage-engine behaviour on a miniature retail model:
//! `'Orders'[Store ID] (*) → (1) Stores[Store ID]`, cross-filtering in
//! both directions, plus the unrelated `'Unmatched Stores'` table.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use arrow::array::{ArrayRef, Decimal128Array, Int64Array, StringArray};
use arrow::record_batch::RecordBatch;
use pbi_engine::{
    Agg, CancellationToken, ColumnFilter, CrossFilterDirection, Engine, EngineError, FilterContext,
    PageRequest, RelationshipOverride, RowSet, TableFilter, Value,
};
use pbi_model::{Column, ColumnRef, CrossFilter, DataType, Model, Relationship, Table};
use pretty_assertions::assert_eq;

fn table(name: &str, columns: &[(&str, DataType)]) -> Table {
    let mut t = Table::new(name);
    for (c, dt) in columns {
        t.columns.push(Column::data(*c, dt.clone()));
    }
    t
}

fn model(cross_filter: CrossFilter) -> Model {
    let mut rel = Relationship::new(
        "orders-stores",
        ColumnRef::new("Orders", "Store ID"),
        ColumnRef::new("Stores", "Store ID"),
    );
    rel.cross_filtering_behavior = cross_filter;
    Model {
        tables: vec![
            table(
                "Stores",
                &[
                    ("Store ID", DataType::String),
                    ("Store Name", DataType::String),
                    ("Region", DataType::String),
                    ("Channel", DataType::String),
                    ("Score", DataType::Int64),
                    ("Cost", DataType::Decimal),
                ],
            ),
            table(
                "Orders",
                &[
                    ("Store ID", DataType::String),
                    ("Ship Mode", DataType::String),
                ],
            ),
            table("Unmatched Stores", &[("Store Name", DataType::String)]),
        ],
        relationships: vec![rel],
        ..Model::default()
    }
}

fn strings(values: &[Option<&str>]) -> ArrayRef {
    Arc::new(StringArray::from(values.to_vec()))
}

/// Stores rows: 0 A01 Central, 1 A02 Central, 2 A03 West,
/// 3 A04 North, 4 A05 (blank region).
fn stores() -> RecordBatch {
    RecordBatch::try_from_iter([
        (
            "Store ID",
            strings(&[
                Some("A01"),
                Some("A02"),
                Some("A03"),
                Some("A04"),
                Some("A05"),
            ]),
        ),
        (
            "Store Name",
            strings(&[
                Some("Store 1"),
                Some("Store 2"),
                Some("Store 3"),
                Some("Store 4"),
                Some("Store 5"),
            ]),
        ),
        (
            "Region",
            strings(&[
                Some("Central"),
                Some("Central"),
                Some("West"),
                Some("North"),
                None,
            ]),
        ),
        (
            "Channel",
            strings(&[
                Some("Online"),
                Some("Retail"),
                Some("Retail"),
                Some("Outlet"),
                Some("Retail"),
            ]),
        ),
        (
            "Score",
            Arc::new(Int64Array::from(vec![
                Some(10),
                Some(20),
                None,
                Some(5),
                Some(1),
            ])) as ArrayRef,
        ),
        (
            "Cost",
            Arc::new(
                Decimal128Array::from(vec![Some(12_500), Some(10_000), None, None, Some(1)])
                    .with_precision_and_scale(19, 4)
                    .unwrap(),
            ) as ArrayRef,
        ),
    ])
    .unwrap()
}

/// Orders rows: 0 A01 Ground, 1 A01 Express, 2 a02 Ground (lower
/// case key), 3 A03 Express, 4 warehouse Ground (unmatched), 5 ORPHAN
/// Express (unmatched), 6 blank key Express (unmatched).
fn orders() -> RecordBatch {
    RecordBatch::try_from_iter([
        (
            "Store ID",
            strings(&[
                Some("A01"),
                Some("A01"),
                Some("a02"),
                Some("A03"),
                Some("(WAREHOUSE)"),
                Some("ORPHAN"),
                None,
            ]),
        ),
        (
            "Ship Mode",
            strings(&[
                Some("Ground"),
                Some("Express"),
                Some("Ground"),
                Some("Express"),
                Some("Ground"),
                Some("Express"),
                Some("Express"),
            ]),
        ),
    ])
    .unwrap()
}

fn unmatched() -> RecordBatch {
    RecordBatch::try_from_iter([(
        "Store Name",
        strings(&[Some("(Warehouse)"), Some("Orphan")]),
    )])
    .unwrap()
}

fn engine_with(cross_filter: CrossFilter) -> Engine {
    let mut engine = Engine::new(&model(cross_filter));
    engine
        .load_tables([
            ("Stores".to_owned(), stores()),
            ("Orders".to_owned(), orders()),
            ("Unmatched Stores".to_owned(), unmatched()),
        ])
        .unwrap();
    engine
}

fn engine() -> Engine {
    engine_with(CrossFilter::BothDirections)
}

fn rows(set: &RowSet) -> Vec<usize> {
    set.iter().collect()
}

fn central() -> FilterContext {
    // Lower case on purpose: IN filters on text are case-insensitive.
    FilterContext::new().with(ColumnFilter::equals("Stores", "Region", "central"))
}

#[test]
fn region_filter_restricts_orders_and_hides_unmatched_orders() {
    let e = engine();
    assert!(e.diagnostics().is_empty(), "{:?}", e.diagnostics());
    let ctx = central();
    assert_eq!(rows(&e.visible_rows(&ctx, "Stores").unwrap()), [0, 1]);
    // `a02` matches `A02`: relationship keys compare case-insensitively.
    assert_eq!(rows(&e.visible_rows(&ctx, "orders").unwrap()), [0, 1, 2]);
    assert!(!e.blank_row_visible(&ctx, "Stores").unwrap());
    let order_rows = e.visible_rows(&ctx, "Orders").unwrap();
    assert_eq!(
        e.aggregate("Orders", "", &order_rows, Agg::CountRows)
            .unwrap(),
        Value::Integer(3)
    );
}

#[test]
fn unfiltered_context_keeps_unmatched_orders_through_the_blank_row() {
    let e = engine();
    let ctx = FilterContext::new();
    assert_eq!(e.visible_rows(&ctx, "Orders").unwrap().count(), 7);
    let visible = e.visible(&ctx, "Stores").unwrap();
    assert_eq!(visible.rows.count(), 5);
    assert!(visible.blank_row);
    assert!(!e.blank_row_visible(&ctx, "Orders").unwrap());

    let values = e.values(&ctx, "Stores", "Store ID").unwrap();
    assert_eq!(values.len(), 6);
    assert_eq!(values.last(), Some(&Value::Blank));
    let all = RowSet::all(5);
    assert_eq!(
        e.aggregate_with_blank_row("Stores", "Store ID", &all, true, Agg::DistinctCount)
            .unwrap(),
        Value::Integer(6)
    );
    assert_eq!(
        e.aggregate("Stores", "Store Name", &all, Agg::DistinctCountNoBlank)
            .unwrap(),
        Value::Integer(5)
    );
}

#[test]
fn many_side_filter_reaches_one_side_only_with_both_directions() {
    let express = FilterContext::new().with(ColumnFilter::equals("Orders", "Ship Mode", "Express"));

    let both = engine();
    assert_eq!(
        rows(&both.visible_rows(&express, "Orders").unwrap()),
        [1, 3, 5, 6]
    );
    let store_rows = both.visible(&express, "Stores").unwrap();
    assert_eq!(rows(&store_rows.rows), [0, 2]);
    // ORPHAN and the blank key are visible Express orders without a store.
    assert!(store_rows.blank_row);

    let matched_only = FilterContext::new().with(ColumnFilter::in_values(
        "Orders",
        "Store ID",
        [Value::from("A01")],
    ));
    assert!(!both.blank_row_visible(&matched_only, "Stores").unwrap());

    let single = engine_with(CrossFilter::OneDirection);
    let store_rows = single.visible(&express, "Stores").unwrap();
    assert_eq!(store_rows.rows.count(), 5);
    assert!(store_rows.blank_row);
}

#[test]
fn filters_do_not_affect_tables_without_a_relationship_path() {
    let e = engine();
    let ctx = central().with(ColumnFilter::equals("Orders", "Ship Mode", "Express"));
    let rows = e.visible_rows(&ctx, "Unmatched Stores").unwrap();
    assert_eq!(rows.count(), 2);
    assert_eq!(
        e.aggregate("Unmatched Stores", "", &rows, Agg::CountRows)
            .unwrap(),
        Value::Integer(2)
    );
}

#[test]
fn blank_row_is_visible_when_the_filter_accepts_blank() {
    let e = engine();
    let blank =
        FilterContext::new().with(ColumnFilter::in_values("Stores", "Region", [Value::Blank]));
    let store_rows = e.visible(&blank, "Stores").unwrap();
    assert_eq!(rows(&store_rows.rows), [4]);
    assert!(store_rows.blank_row);
    assert_eq!(rows(&e.visible_rows(&blank, "Orders").unwrap()), [4, 5, 6]);

    // DAX `=`: BLANK = "" is true, so `Region = ""` behaves the same.
    let empty = FilterContext::new().with(ColumnFilter::dax_equals("Stores", "Region", ""));
    assert_eq!(rows(&e.visible_rows(&empty, "Orders").unwrap()), [4, 5, 6]);
    // Strict `==` with "" matches nothing.
    let strict = FilterContext::new().with(ColumnFilter::equals("Stores", "Region", ""));
    assert!(e.visible_rows(&strict, "Orders").unwrap().is_empty());
}

#[test]
fn calculate_style_replace_keepfilters_and_all() {
    let e = engine();
    let mut ctx = central();
    ctx.replace_column(ColumnFilter::equals("Stores", "Region", "West"));
    assert_eq!(rows(&e.visible_rows(&ctx, "Orders").unwrap()), [3]);

    let mut keep = central();
    keep.add(ColumnFilter::equals("Stores", "Region", "West"));
    assert!(e.visible_rows(&keep, "Orders").unwrap().is_empty());

    let mut all = central();
    all.remove_table("Stores");
    assert_eq!(e.visible_rows(&all, "Orders").unwrap().count(), 7);
}

#[test]
fn table_filters_filter_the_expanded_table() {
    // Single direction: a column filter on Orders would not reach
    // Stores, but a table filter on Orders does (expanded table).
    let e = engine_with(CrossFilter::OneDirection);
    let order3 = || TableFilter::new("Orders", RowSet::from_rows(7, [3]));
    let ctx = FilterContext::new().with(order3());
    assert_eq!(rows(&e.visible_rows(&ctx, "Stores").unwrap()), [2]);
    assert!(!e.blank_row_visible(&ctx, "Stores").unwrap());
    assert_eq!(rows(&e.visible_rows(&ctx, "Orders").unwrap()), [3]);

    let mut removed = ctx.clone();
    removed.remove_table("Stores");
    assert_eq!(e.visible_rows(&removed, "Stores").unwrap().count(), 5);
    assert_eq!(rows(&e.visible_rows(&removed, "Orders").unwrap()), [3]);

    let base_only = FilterContext::new().with(order3().base_only());
    assert_eq!(e.visible_rows(&base_only, "Stores").unwrap().count(), 5);

    let mut all = ctx;
    all.remove_expanded_table(e.graph(), "Orders");
    assert!(all.is_empty());
}

#[test]
fn group_by_columns_of_related_one_side_tables() {
    let e = engine();
    let all = RowSet::all(7);
    let groups = e
        .group_by("Orders", &[ColumnRef::new("Stores", "Region")], &all)
        .unwrap();
    let summary: Vec<(Vec<Value>, Vec<usize>)> = groups
        .iter()
        .map(|g| (g.values.clone(), rows(&g.rows)))
        .collect();
    assert_eq!(
        summary,
        vec![
            (vec![Value::from("Central")], vec![0, 1, 2]),
            (vec![Value::from("West")], vec![3]),
            (vec![Value::Blank], vec![4, 5, 6]),
        ]
    );

    let groups = e
        .group_by(
            "Orders",
            &[
                ColumnRef::new("Orders", "Ship Mode"),
                ColumnRef::new("Stores", "Store Name"),
            ],
            &e.visible_rows(&central(), "Orders").unwrap(),
        )
        .unwrap();
    let keys: Vec<Vec<Value>> = groups.into_iter().map(|g| g.values).collect();
    assert_eq!(
        keys,
        vec![
            vec![Value::from("Ground"), Value::from("Store 1")],
            vec![Value::from("Express"), Value::from("Store 1")],
            vec![Value::from("Ground"), Value::from("Store 2")],
        ]
    );

    assert!(matches!(
        e.group_by(
            "Stores",
            &[ColumnRef::new("Orders", "Ship Mode")],
            &RowSet::all(5)
        ),
        Err(EngineError::NoRelationshipPath { .. })
    ));
    assert_eq!(e.group_by("Orders", &[], &all).unwrap().len(), 1);
    assert!(
        e.group_by("Orders", &[], &RowSet::empty(7))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn aggregates_follow_dax_blank_semantics() {
    let e = engine();
    let all = RowSet::all(5);
    let none = RowSet::empty(5);
    let agg = |col: &str, rows: &RowSet, agg| e.aggregate("Stores", col, rows, agg).unwrap();

    assert_eq!(agg("Score", &all, Agg::Sum), Value::Integer(36));
    assert_eq!(agg("Score", &none, Agg::Sum), Value::Blank);
    assert_eq!(
        agg("Score", &RowSet::from_rows(5, [2]), Agg::Sum),
        Value::Blank
    );
    assert_eq!(agg("Score", &none, Agg::CountRows), Value::Blank);
    assert_eq!(agg("Score", &all, Agg::CountRows), Value::Integer(5));
    assert_eq!(agg("Score", &all, Agg::Count), Value::Integer(4));
    assert_eq!(agg("Score", &all, Agg::CountNonBlank), Value::Integer(4));
    assert_eq!(agg("Score", &all, Agg::CountBlank), Value::Integer(1));
    assert_eq!(
        agg("Score", &RowSet::from_rows(5, [0]), Agg::CountBlank),
        Value::Blank
    );
    assert_eq!(agg("Score", &all, Agg::Average), Value::Double(9.0));
    assert_eq!(agg("Score", &all, Agg::Min), Value::Integer(1));
    assert_eq!(agg("Score", &all, Agg::Max), Value::Integer(20));
    assert_eq!(agg("Score", &none, Agg::Max), Value::Blank);
    assert_eq!(agg("Cost", &all, Agg::Sum), Value::Decimal(22_501));
    assert_eq!(agg("Cost", &all, Agg::Average), Value::Decimal(7_500));
    assert_eq!(agg("Region", &all, Agg::Min), Value::from("Central"));
    assert_eq!(agg("Region", &all, Agg::Max), Value::from("West"));
    assert_eq!(agg("Region", &all, Agg::DistinctCount), Value::Integer(4));
    assert_eq!(
        agg("Region", &all, Agg::DistinctCountNoBlank),
        Value::Integer(3)
    );
    assert!(matches!(
        e.aggregate("Stores", "Region", &all, Agg::Sum),
        Err(EngineError::UnsupportedAggregation { .. })
    ));
    assert_eq!(
        e.aggregate_with_blank_row("Stores", "Score", &none, true, Agg::CountRows)
            .unwrap(),
        Value::Integer(1)
    );
}

#[test]
fn text_values_compare_case_insensitively_everywhere() {
    let mut e = engine();
    e.add_column(
        "Unmatched Stores",
        "Kind",
        strings(&[Some("Warehouse"), Some("WAREHOUSE")]),
    )
    .unwrap();
    let all = RowSet::all(2);
    // One dictionary value (first spelling wins), as in VertiPaq.
    assert_eq!(
        e.distinct("Unmatched Stores", "Kind", &all).unwrap(),
        vec![Value::from("Warehouse")]
    );
    assert_eq!(
        e.value_at("Unmatched Stores", "kind", 1).unwrap(),
        Value::from("Warehouse")
    );
    let ctx = FilterContext::new().with(ColumnFilter::equals(
        "unmatched stores",
        "KIND",
        "warehouse",
    ));
    assert_eq!(e.visible_rows(&ctx, "Unmatched Stores").unwrap().count(), 2);
    assert!(
        Value::from("Central")
            .dax_eq(&Value::from("CENTRAL"))
            .unwrap()
    );
}

#[test]
fn calculated_columns_from_relatedtable_drive_measures() {
    let mut e = engine();
    // Sales Status = IF(ISEMPTY(RELATEDTABLE('Orders')), "Inactive", "Active")
    let mut status = Vec::new();
    let mut counts = Vec::new();
    for row in 0..e.row_count("Stores").unwrap() {
        let related = e.related_rows("Stores", row, "Orders", None).unwrap();
        status.push(Value::from(if related.is_empty() {
            "Inactive"
        } else {
            "Active"
        }));
        counts.push(Value::Integer(related.count() as i64));
    }
    let to_array = |values: &[Value], dt: &DataType| {
        pbi_engine::value::values_to_array(values, &pbi_engine::arrow_type(dt)).unwrap()
    };
    e.add_column(
        "Stores",
        "Sales Status",
        to_array(&status, &DataType::String),
    )
    .unwrap();
    e.add_column("Stores", "Order Count", to_array(&counts, &DataType::Int64))
        .unwrap();
    assert_eq!(
        e.value_at("Stores", "Order Count", 0).unwrap(),
        Value::Integer(2)
    );
    assert_eq!(
        e.value_at("Stores", "Order Count", 1).unwrap(),
        Value::Integer(1)
    );

    // Active Stores = CALCULATE(COUNTROWS(Stores), Stores[Sales Status] = "Active")
    let active = |ctx: &FilterContext| {
        let mut ctx = ctx.clone();
        ctx.replace_column(ColumnFilter::equals("Stores", "Sales Status", "Active"));
        let rows = e.visible_rows(&ctx, "Stores").unwrap();
        e.aggregate("Stores", "", &rows, Agg::CountRows).unwrap()
    };
    assert_eq!(active(&FilterContext::new()), Value::Integer(3));
    assert_eq!(active(&central()), Value::Integer(2));

    // RELATEDTABLE respecting an outer filter context.
    let express = FilterContext::new().with(ColumnFilter::equals("Orders", "Ship Mode", "Express"));
    let related = e
        .related_rows("Stores", 0, "Orders", Some(&express))
        .unwrap();
    assert_eq!(rows(&related), [1]);
}

#[test]
fn related_and_lookup_navigation() {
    let e = engine();
    assert_eq!(e.related_row("Orders", 2, "Stores").unwrap(), Some(1));
    assert_eq!(e.related_row("Orders", 4, "Stores").unwrap(), None);
    assert_eq!(
        e.related_value("Orders", 3, "Stores", "Region").unwrap(),
        Value::from("West")
    );
    assert_eq!(
        e.related_value("Orders", 6, "Stores", "Region").unwrap(),
        Value::Blank
    );
    assert!(matches!(
        e.related_row("Stores", 0, "Orders"),
        Err(EngineError::NoRelationshipPath { .. })
    ));
    assert!(matches!(
        e.related_row("Orders", 70, "Stores"),
        Err(EngineError::RowOutOfRange { .. })
    ));

    let lookup = |result: &str, column: &str, value: &str| {
        e.lookup("Stores", result, &[(column, Value::from(value))], None)
    };
    assert_eq!(
        lookup("Region", "Store ID", "a03").unwrap(),
        Value::from("West")
    );
    assert_eq!(lookup("Region", "Store ID", "nope").unwrap(), Value::Blank);
    assert!(matches!(
        lookup("Store ID", "Channel", "Retail"),
        Err(EngineError::AmbiguousLookup { .. })
    ));
}

#[test]
fn chains_propagate_transitively() {
    // Sales → Products → Brands (single direction): a filter on Brands reaches
    // Sales through Products, and RELATED walks the chain.
    let mut m = Model {
        tables: vec![
            table("Sales", &[("Product", DataType::String)]),
            table(
                "Products",
                &[("Product", DataType::String), ("Brand", DataType::String)],
            ),
            table(
                "Brands",
                &[("Brand", DataType::String), ("Team", DataType::String)],
            ),
        ],
        ..Model::default()
    };
    m.relationships.push(Relationship::new(
        "r1",
        ColumnRef::new("Sales", "Product"),
        ColumnRef::new("Products", "Product"),
    ));
    m.relationships.push(Relationship::new(
        "r2",
        ColumnRef::new("Products", "Brand"),
        ColumnRef::new("Brands", "Brand"),
    ));
    let mut e = Engine::new(&m);
    let batch = |cols: Vec<(&str, ArrayRef)>| RecordBatch::try_from_iter(cols).unwrap();
    e.load_table(
        "Sales",
        batch(vec![(
            "Product",
            strings(&[Some("a"), Some("b"), Some("c"), Some("a")]),
        )]),
    )
    .unwrap();
    e.load_table(
        "Products",
        batch(vec![
            ("Product", strings(&[Some("a"), Some("b"), Some("c")])),
            (
                "Brand",
                strings(&[Some("trek"), Some("peak"), Some("trek")]),
            ),
        ]),
    )
    .unwrap();
    e.load_table(
        "Brands",
        batch(vec![
            ("Brand", strings(&[Some("trek"), Some("peak")])),
            ("Team", strings(&[Some("red"), Some("blue")])),
        ]),
    )
    .unwrap();
    let red = FilterContext::new().with(ColumnFilter::equals("Brands", "Team", "red"));
    assert_eq!(rows(&e.visible_rows(&red, "Sales").unwrap()), [0, 2, 3]);
    assert_eq!(e.visible_rows(&red, "Brands").unwrap().count(), 1);
    // Single direction: a filter on Sales does not reach Brands.
    let sale = FilterContext::new().with(ColumnFilter::equals("Sales", "Product", "b"));
    assert_eq!(e.visible_rows(&sale, "Brands").unwrap().count(), 2);
    assert_eq!(e.related_row("Sales", 1, "Brands").unwrap(), Some(1));
    let groups = e
        .group_by(
            "Sales",
            &[ColumnRef::new("Brands", "Team")],
            &RowSet::all(4),
        )
        .unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(rows(&groups[0].rows), [0, 2, 3]);
}

#[test]
fn bidirectional_cross_filter_continues_to_sibling_tables() {
    // Returns → Stores (single) and Orders ↔ Stores (both):
    // an Orders filter narrows Stores, which then narrows Returns.
    let mut m = model(CrossFilter::BothDirections);
    m.tables
        .push(table("Returns", &[("Store ID", DataType::String)]));
    m.relationships.push(Relationship::new(
        "returns",
        ColumnRef::new("Returns", "Store ID"),
        ColumnRef::new("Stores", "Store ID"),
    ));
    let mut e = Engine::new(&m);
    e.load_table("Stores", stores()).unwrap();
    e.load_table("Orders", orders()).unwrap();
    e.load_table(
        "Returns",
        RecordBatch::try_from_iter([(
            "Store ID",
            strings(&[Some("A01"), Some("A02"), Some("A04"), Some("A03")]),
        )])
        .unwrap(),
    )
    .unwrap();
    let ctx = FilterContext::new().with(ColumnFilter::equals("Orders", "Ship Mode", "Express"));
    assert_eq!(rows(&e.visible_rows(&ctx, "Returns").unwrap()), [0, 3]);
    // Returns → Stores is single-direction: a Returns filter does not
    // reach Stores or Orders.
    let returns = FilterContext::new().with(ColumnFilter::equals("Returns", "Store ID", "A04"));
    assert_eq!(e.visible_rows(&returns, "Orders").unwrap().count(), 7);

    // Model edits: dropping the Returns relationship stops the propagation.
    m.relationships.pop();
    e.sync_model(&m);
    assert_eq!(e.visible_rows(&ctx, "Returns").unwrap().count(), 4);
    assert!(e.remove_table("Returns"));
    assert!(!e.remove_table("Returns"));
    e.rebuild_relationships();
    assert_eq!(rows(&e.visible_rows(&ctx, "Stores").unwrap()), [0, 2]);
}

#[test]
fn bad_input_returns_errors() {
    let mut e = engine();
    assert!(matches!(
        e.visible_rows(&FilterContext::new(), "Nope"),
        Err(EngineError::TableNotFound(_))
    ));
    let bad_column = FilterContext::new().with(ColumnFilter::equals("Stores", "Nope", "x"));
    assert!(matches!(
        e.visible_rows(&bad_column, "Orders"),
        Err(EngineError::ColumnNotFound { .. })
    ));
    let bad_table = FilterContext::new().with(ColumnFilter::equals("Nope", "X", "x"));
    assert!(e.visible_rows(&bad_table, "Orders").is_err());
    assert!(matches!(
        e.add_column("Stores", "X", Arc::new(Int64Array::from(vec![1]))),
        Err(EngineError::RowCountMismatch { .. })
    ));
    assert!(e.value_at("Stores", "Store ID", 99).is_err());
    // A RowSet over the wrong universe is tolerated.
    assert_eq!(
        e.aggregate("Stores", "Score", &RowSet::all(1000), Agg::Count)
            .unwrap(),
        Value::Integer(4)
    );
}

#[test]
fn relationship_problems_become_diagnostics() {
    let mut m = model(CrossFilter::BothDirections);
    m.relationships.push(Relationship::new(
        "dangling",
        ColumnRef::new("Orders", "Missing"),
        ColumnRef::new("Stores", "Store ID"),
    ));
    let mut e = Engine::new(&m);
    let dup = RecordBatch::try_from_iter([("Store ID", strings(&[Some("A"), Some("a")]))]).unwrap();
    e.load_table("Stores", dup).unwrap();
    e.load_table("Orders", orders()).unwrap();
    let codes: Vec<String> = e.diagnostics().into_iter().map(|d| d.code).collect();
    assert!(codes.contains(&"ENG001".to_owned()), "{codes:?}");
    assert!(codes.contains(&"ENG002".to_owned()), "{codes:?}");
    // Still usable.
    assert_eq!(
        e.visible_rows(&FilterContext::new(), "Orders")
            .unwrap()
            .count(),
        7
    );
}

/// `Orders[OrderDay] → Days[Day]` (active) and `Orders[ShipDay] →
/// Days[Day]` (inactive). Orders: 0 (d1 → d2), 1 (d2 → d3), 2 (d3 → d9,
/// unmatched ship day).
fn role_playing_engine() -> Engine {
    let mut ship = Relationship::new(
        "ship",
        ColumnRef::new("Orders", "ShipDay"),
        ColumnRef::new("Days", "Day"),
    );
    ship.is_active = false;
    let m = Model {
        tables: vec![
            table("Days", &[("Day", DataType::String)]),
            table(
                "Orders",
                &[
                    ("OrderDay", DataType::String),
                    ("ShipDay", DataType::String),
                ],
            ),
        ],
        relationships: vec![
            Relationship::new(
                "order",
                ColumnRef::new("Orders", "OrderDay"),
                ColumnRef::new("Days", "Day"),
            ),
            ship,
        ],
        ..Model::default()
    };
    let mut e = Engine::new(&m);
    let batch = |cols: Vec<(&str, ArrayRef)>| RecordBatch::try_from_iter(cols).unwrap();
    e.load_table(
        "Days",
        batch(vec![(
            "Day",
            strings(&[Some("d1"), Some("d2"), Some("d3")]),
        )]),
    )
    .unwrap();
    e.load_table(
        "Orders",
        batch(vec![
            ("OrderDay", strings(&[Some("d1"), Some("d2"), Some("d3")])),
            ("ShipDay", strings(&[Some("d2"), Some("d3"), Some("d9")])),
        ]),
    )
    .unwrap();
    e
}

#[test]
fn userelationship_override_activates_an_inactive_relationship() {
    let e = role_playing_engine();
    let g = e.graph();
    let order = g
        .find_relationship("days", "DAY", "Orders", "OrderDay")
        .unwrap();
    let ship = g
        .find_relationship("Orders", "ShipDay", "Days", "Day")
        .unwrap();
    assert_eq!((order, ship), (0, 1));
    assert!(
        g.find_relationship("Orders", "ShipDay", "Orders", "OrderDay")
            .is_none()
    );

    let d2 = FilterContext::new().with(ColumnFilter::equals("Days", "Day", "d2"));
    assert_eq!(rows(&e.visible_rows(&d2, "Orders").unwrap()), [1]);
    let by_ship = d2
        .clone()
        .with_relationship_override(RelationshipOverride::Activate(ship));
    assert_eq!(rows(&e.visible_rows(&by_ship, "Orders").unwrap()), [0]);
    // The same answer from the graph directly and from the cached graph.
    let direct = e
        .graph()
        .visible(e.storage(), &by_ship, "Orders")
        .unwrap()
        .rows;
    assert_eq!(rows(&direct), [0]);
    assert_eq!(e.graph_for(&by_ship).active_relationships(), [ship]);
    assert_eq!(e.graph_for(&d2).active_relationships(), [order]);
    // The ship relationship has an unmatched key: Days gets a blank row.
    let none =
        FilterContext::new().with_relationship_override(RelationshipOverride::Activate(ship));
    assert!(none.is_empty() && none.has_relationship_overrides());
    assert!(!e.visible(&FilterContext::new(), "Days").unwrap().blank_row);
    assert!(e.visible(&none, "Days").unwrap().blank_row);
    // ALL() keeps the override; contexts differ in equality and fingerprint.
    let mut cleared = by_ship.clone();
    cleared.clear();
    assert_eq!(
        cleared.relationship_overrides(),
        [RelationshipOverride::Activate(ship)]
    );
    assert_ne!(d2, by_ship);
    assert_ne!(d2.fingerprint(), by_ship.fingerprint());
    // Inner overrides of the same kind replace outer ones.
    let back = by_ship.with_relationship_override(RelationshipOverride::Activate(order));
    assert_eq!(rows(&e.visible_rows(&back, "Orders").unwrap()), [1]);
    back.relationship_overrides()
        .iter()
        .for_each(|o| assert!(o.relationship() < 2));
}

#[test]
fn crossfilter_override_changes_the_filter_direction() {
    let e = role_playing_engine();
    let cross = |dir| RelationshipOverride::CrossFilter(0, dir);
    let d2 = FilterContext::new().with(ColumnFilter::equals("Days", "Day", "d2"));
    let o3 = FilterContext::new().with(ColumnFilter::equals("Orders", "OrderDay", "d3"));
    // Single direction by default: Orders does not filter Days.
    assert_eq!(e.visible_rows(&o3, "Days").unwrap().count(), 3);
    let both = o3
        .clone()
        .with_relationship_override(cross(CrossFilterDirection::Both));
    assert_eq!(rows(&e.visible_rows(&both, "Days").unwrap()), [2]);
    // None disables the relationship.
    let off = d2
        .clone()
        .with_relationship_override(cross(CrossFilterDirection::None));
    assert_eq!(e.visible_rows(&off, "Orders").unwrap().count(), 3);
    // Many → one only: Orders filters Days, Days no longer filters Orders.
    let reverse = |ctx: &FilterContext| {
        ctx.clone()
            .with_relationship_override(cross(CrossFilterDirection::ManyToOne))
    };
    assert_eq!(rows(&e.visible_rows(&reverse(&o3), "Days").unwrap()), [2]);
    assert_eq!(e.visible_rows(&reverse(&d2), "Orders").unwrap().count(), 3);
    // OneWay restores the default; the later CROSSFILTER wins.
    let one_way = reverse(&o3).with_relationship_override(cross(CrossFilterDirection::OneWay));
    assert_eq!(one_way.relationship_overrides().len(), 1);
    assert_eq!(e.visible_rows(&one_way, "Days").unwrap().count(), 3);
    // Unknown relationship indexes are ignored.
    let bogus = d2
        .clone()
        .with_relationship_override(RelationshipOverride::Activate(99));
    assert_eq!(rows(&e.visible_rows(&bogus, "Orders").unwrap()), [1]);
}

#[test]
fn snapshots_are_isolated_from_later_loads() {
    let mut e = Engine::new(&model(CrossFilter::BothDirections));
    let before = e.snapshot();
    assert_eq!(before.row_count("Stores").unwrap(), 0);
    assert!(!before.table("Stores").unwrap().is_loaded());
    e.load_table("stores", stores()).unwrap();
    assert_eq!(before.row_count("Stores").unwrap(), 0);
    assert_eq!(e.row_count("Stores").unwrap(), 5);
    assert_eq!(e.table("Stores").unwrap().name(), "Stores");
    assert!(e.generation() > before.generation());
    let rows = std::thread::spawn(move || before.row_count("Orders").unwrap())
        .join()
        .unwrap();
    assert_eq!(rows, 0);
}

#[test]
fn column_stats_summarise_a_column() {
    let e = engine();
    let stats = e.column_stats("Stores", "Region").unwrap();
    assert_eq!(stats.rows, 5);
    assert_eq!(stats.count, 4);
    assert_eq!(stats.blanks, 1);
    assert_eq!(stats.distinct, 3);
    assert_eq!(stats.min, Value::from("Central"));
    assert_eq!(stats.max, Value::from("West"));
    let score = e.column_stats("Stores", "Score").unwrap();
    assert_eq!(
        (score.min, score.max),
        (Value::Integer(1), Value::Integer(20))
    );
}

fn int_column(batches: &[RecordBatch], column: usize) -> Vec<i64> {
    batches
        .iter()
        .flat_map(|b| {
            b.column(column)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .values()
                .to_vec()
        })
        .collect()
}

fn string_column(batch: &RecordBatch, column: usize) -> Vec<Option<String>> {
    batch
        .column(column)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap()
        .iter()
        .map(|v| v.map(str::to_owned))
        .collect()
}

fn some(values: &[&str]) -> Vec<Option<String>> {
    values.iter().map(|v| Some((*v).to_owned())).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn sql_queries_tables_by_case_insensitive_name() {
    let e = engine();
    let snap = e.snapshot();
    let out = snap
        .sql(
            r#"SELECT count(*) AS n FROM "orders" WHERE "Ship Mode" = 'Express'"#,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(int_column(&out, 0), [4]);
    let out = snap
        .sql(
            r#"SELECT count(*) FROM stores a JOIN "Orders" f ON a."Store ID" = f."Store ID""#,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    // SQL joins are case-sensitive (`a02` ≠ `A02`), unlike relationships.
    assert_eq!(int_column(&out, 0), [3]);
    // Read-only: DDL and DML are rejected.
    for statement in [
        "CREATE TABLE x AS SELECT 1",
        "DROP TABLE Stores",
        "INSERT INTO Stores (\"Store ID\") VALUES ('x')",
    ] {
        assert!(
            snap.sql(statement, CancellationToken::new()).await.is_err(),
            "{statement}"
        );
    }
    assert!(
        snap.sql("SELECT * FROM nope", CancellationToken::new())
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn page_sorts_filters_and_counts() {
    let e = engine();
    let request = PageRequest::new("Orders", 1, 2)
        .sort_by("store id", true)
        .filter(r#""Ship Mode" = 'Express'"#);
    let page = e.page(&request, CancellationToken::new()).await.unwrap();
    assert_eq!(page.num_columns(), 2);
    assert_eq!(page.schema().field(0).name(), "Store ID");
    // Express orders sorted by Store ID: blank, A01, A03, ORPHAN.
    assert_eq!(string_column(&page, 0), some(&["A01", "A03"]));

    // Unsorted pages keep storage order; text sorts case-insensitively.
    let first = e
        .page(&PageRequest::new("Orders", 0, 3), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(string_column(&first, 0), some(&["A01", "A01", "a02"]));
    let sorted = e
        .page(
            &PageRequest::new("Orders", 0, 7).sort_by("Store ID", false),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let keys = string_column(&sorted, 0);
    assert_eq!(keys[0].as_deref(), Some("ORPHAN"));
    assert_eq!(keys[2].as_deref(), Some("a02"));
    assert_eq!(keys[6], None);

    let descending = e
        .page(
            &PageRequest::new("Stores", 0, 1).sort_by("Score", false),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(string_column(&descending, 0), some(&["A02"]));

    let count =
        |filter: Option<&'static str>| e.page_row_count("Orders", filter, CancellationToken::new());
    assert_eq!(count(Some(r#""Ship Mode" = 'Express'"#)).await.unwrap(), 4);
    assert_eq!(count(None).await.unwrap(), 7);

    assert!(matches!(
        e.page(
            &PageRequest::new("Orders", 0, 1).sort_by("Nope", true),
            CancellationToken::new()
        )
        .await,
        Err(EngineError::ColumnNotFound { .. })
    ));
    assert!(
        e.page(
            &PageRequest::new("Orders", 0, 1).filter("Nope = 1"),
            CancellationToken::new()
        )
        .await
        .is_err()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn queries_can_be_cancelled() {
    let e = engine();
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        e.sql("SELECT 1", cancelled.clone()).await,
        Err(EngineError::Cancelled)
    ));
    assert!(matches!(
        e.page(&PageRequest::new("Orders", 0, 5), cancelled).await,
        Err(EngineError::Cancelled)
    ));

    let token = CancellationToken::new();
    let trigger = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        trigger.cancel();
    });
    let snap = e.snapshot();
    let slow = snap.sql(
        "SELECT count(*) FROM generate_series(1, 100000000000) AS t",
        token,
    );
    let result = tokio::time::timeout(Duration::from_secs(30), slow)
        .await
        .expect("cancellation did not stop the query");
    assert!(matches!(result, Err(EngineError::Cancelled)), "{result:?}");
}

// ---- grouped aggregation, inverted indexes, sparse propagation ----------------

/// Deterministic xorshift64* generator.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) % n.max(1) as u64) as usize
    }

    fn pick<T: Clone>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())].clone()
    }
}

/// `Fact[K] → Dim[K]`, `Dim[H] → Dim2[H]` (single direction) with random
/// data: case variants, BLANKs, unmatched keys, a duplicate one-side key.
fn random_star(seed: u64, dim2_blank_row: bool) -> Engine {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let m = Model {
        tables: vec![
            table(
                "Fact",
                &[
                    ("K", DataType::String),
                    ("V", DataType::Int64),
                    ("X", DataType::Double),
                    ("T", DataType::String),
                    ("C", DataType::Decimal),
                ],
            ),
            table(
                "Dim",
                &[
                    ("K", DataType::String),
                    ("G", DataType::String),
                    ("H", DataType::Int64),
                ],
            ),
            table("Dim2", &[("H", DataType::Int64), ("Z", DataType::String)]),
        ],
        relationships: vec![
            Relationship::new(
                "r1",
                ColumnRef::new("Fact", "K"),
                ColumnRef::new("Dim", "K"),
            ),
            Relationship::new(
                "r2",
                ColumnRef::new("Dim", "H"),
                ColumnRef::new("Dim2", "H"),
            ),
        ],
        ..Model::default()
    };
    let n = 20 + rng.below(300);
    let keys = [
        Some("k0"),
        Some("K1"),
        Some("k2"),
        Some("k3"),
        Some("K4"),
        Some("k5"),
        Some("nope"),
        None,
    ];
    let texts = [Some("a"), Some("A"), Some("b"), Some(""), None, Some("c")];
    let fact = RecordBatch::try_from_iter([
        (
            "K",
            strings(&(0..n).map(|_| rng.pick(&keys)).collect::<Vec<_>>()),
        ),
        (
            "V",
            Arc::new(Int64Array::from(
                (0..n)
                    .map(|_| rng.pick(&[Some(1), Some(-2), Some(7), Some(0), None]))
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
        ),
        (
            "X",
            Arc::new(arrow::array::Float64Array::from(
                (0..n)
                    .map(|_| rng.pick(&[Some(0.1), Some(0.7), Some(-3.5), Some(1e15), None]))
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
        ),
        (
            "T",
            strings(&(0..n).map(|_| rng.pick(&texts)).collect::<Vec<_>>()),
        ),
        (
            "C",
            Arc::new(
                Decimal128Array::from(
                    (0..n)
                        .map(|_| rng.pick(&[Some(10_000i128), Some(-1), Some(25_005), None]))
                        .collect::<Vec<_>>(),
                )
                .with_precision_and_scale(19, 4)
                .unwrap(),
            ) as ArrayRef,
        ),
    ])
    .unwrap();
    let dim = RecordBatch::try_from_iter([
        (
            "K",
            strings(&[
                Some("K0"),
                Some("k1"),
                Some("k2"),
                Some("K3"),
                Some("k4"),
                Some("K0"),
            ]),
        ),
        (
            "G",
            strings(&(0..6).map(|_| rng.pick(&texts)).collect::<Vec<_>>()),
        ),
        (
            "H",
            Arc::new(Int64Array::from(
                (0..6)
                    .map(|_| rng.pick(&[Some(1), Some(2), Some(9), None]))
                    .map(|h| if dim2_blank_row { h } else { h.or(Some(2)) })
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
        ),
    ])
    .unwrap();
    let (h, z): (Vec<i64>, Vec<Option<&str>>) = if dim2_blank_row {
        (vec![1], vec![Some("one")])
    } else {
        (vec![1, 2, 9], vec![Some("one"), None, Some("nine")])
    };
    let dim2 = RecordBatch::try_from_iter([
        ("H", Arc::new(Int64Array::from(h)) as ArrayRef),
        ("Z", strings(&z)),
    ])
    .unwrap();
    let mut e = Engine::new(&m);
    e.load_tables([
        ("Fact".to_owned(), fact),
        ("Dim".to_owned(), dim),
        ("Dim2".to_owned(), dim2),
    ])
    .unwrap();
    e
}

fn random_rows(rng: &mut Rng, n: usize, percent: usize) -> RowSet {
    RowSet::from_rows(n, (0..n).filter(|_| rng.below(100) < percent))
}

#[test]
fn group_index_and_aggregate_groups_match_group_by_and_aggregate() {
    let aggs = [
        Agg::Sum,
        Agg::Min,
        Agg::Max,
        Agg::Count,
        Agg::CountNonBlank,
        Agg::DistinctCount,
        Agg::DistinctCountNoBlank,
        Agg::Average,
        Agg::CountRows,
        Agg::CountBlank,
    ];
    let groupings: [&[(&str, &str)]; 5] = [
        &[("Fact", "T")],
        &[("Dim", "G"), ("Fact", "V")],
        &[("Dim2", "Z")],
        &[("Fact", "K"), ("Dim2", "Z"), ("Dim", "H")],
        &[],
    ];
    for seed in 0..40 {
        let e = random_star(seed, seed % 2 == 0);
        let n = e.row_count("Fact").unwrap();
        let mut rng = Rng(seed + 7);
        for cols in groupings {
            let refs: Vec<ColumnRef> = cols.iter().map(|(t, c)| ColumnRef::new(*t, *c)).collect();
            let percent = rng.pick(&[3, 50, 100]);
            let selected = random_rows(&mut rng, n, percent);
            let groups = e.group_by("Fact", &refs, &selected).unwrap();
            let index = e.group_index("Fact", &refs, &selected).unwrap();
            assert_eq!(index.len(), groups.len());
            for (g, group) in groups.iter().enumerate() {
                assert_eq!(index.values()[g], group.values);
                let members: Vec<usize> = index.group_rows(g).iter().map(|r| *r as usize).collect();
                assert_eq!(members, rows(&group.rows));
                assert!(members.iter().all(|r| index.group_of(*r) == Some(g)));
            }
            for column in ["V", "X", "T", "C", "K"] {
                for agg in aggs {
                    let grouped = e.aggregate_groups(&index, column, agg);
                    let single: Vec<_> = groups
                        .iter()
                        .map(|g| e.aggregate("Fact", column, &g.rows, agg))
                        .collect();
                    match grouped {
                        Ok(values) => {
                            let expected: Vec<Value> =
                                single.into_iter().map(Result::unwrap).collect();
                            assert_eq!(
                                format!("{values:?}"),
                                format!("{expected:?}"),
                                "seed {seed} {cols:?} {column} {agg:?}"
                            );
                        }
                        Err(_) => assert!(
                            single.iter().any(Result::is_err),
                            "seed {seed} {cols:?} {column} {agg:?}"
                        ),
                    }
                }
            }
        }
    }
}

/// Brute-force visible rows of `Fact` under a filter on `Dim[G]`: the
/// related `Dim` row's value (or the blank row's BLANK) passes `keep`.
fn expected_fact_rows(e: &Engine, keep: &dyn Fn(&Value) -> bool) -> Vec<usize> {
    (0..e.row_count("Fact").unwrap())
        .filter(|r| {
            let v = match e.related_row("Fact", *r, "Dim").unwrap() {
                Some(d) => e.value_at("Dim", "G", d).unwrap(),
                None => Value::Blank,
            };
            keep(&v)
        })
        .collect()
}

#[test]
fn in_filters_and_propagation_use_indexes_without_changing_results() {
    let pool = [
        Value::from("a"),
        Value::from("B"),
        Value::from(""),
        Value::Blank,
        Value::from("c"),
        Value::from("zzz"),
    ];
    for seed in 0..40 {
        let e = random_star(seed, true);
        let mut rng = Rng(seed + 99);
        for _ in 0..8 {
            let wanted: Vec<Value> = (0..1 + rng.below(3)).map(|_| rng.pick(&pool)).collect();
            let keys: Vec<_> = wanted.iter().map(pbi_engine::ValueKey::from).collect();
            let keep = |v: &Value| keys.contains(&pbi_engine::ValueKey::from(v));
            let in_ctx =
                FilterContext::new().with(ColumnFilter::in_values("Dim", "G", wanted.clone()));
            let keys2 = keys.clone();
            let pred_ctx =
                FilterContext::new().with(ColumnFilter::predicate("Dim", "G", move |v| {
                    keys2.contains(&pbi_engine::ValueKey::from(v))
                }));
            for t in ["Fact", "Dim"] {
                assert_eq!(
                    e.visible(&in_ctx, t).unwrap(),
                    e.visible(&pred_ctx, t).unwrap()
                );
            }
            assert_eq!(
                rows(&e.visible_rows(&in_ctx, "Fact").unwrap()),
                expected_fact_rows(&e, &keep),
                "seed {seed} {wanted:?}"
            );
            // Fact-column IN filters (inverted index on a high-cardinality column).
            let t_ctx =
                FilterContext::new().with(ColumnFilter::in_values("Fact", "T", wanted.clone()));
            let expected: Vec<usize> = (0..e.row_count("Fact").unwrap())
                .filter(|r| keep(&e.value_at("Fact", "T", *r).unwrap()))
                .collect();
            assert_eq!(rows(&e.visible_rows(&t_ctx, "Fact").unwrap()), expected);
        }
    }
}

#[test]
fn expanded_paths_and_reachable_rows() {
    let e = random_star(3, false);
    let g = e.graph();
    assert!(g.filters_follow_expanded_path("Fact", "Dim2"));
    assert!(g.filters_follow_expanded_path("Fact", "Dim"));
    assert!(g.filters_follow_expanded_path("Fact", "fact"));
    assert!(!g.filters_follow_expanded_path("Dim", "Fact"));
    // With both directions, Fact's filters reach Dim, but not along a
    // many → one path.
    let both = engine();
    assert!(
        both.graph()
            .filters_follow_expanded_path("Orders", "Stores")
    );
    assert!(
        !both
            .graph()
            .filters_follow_expanded_path("Stores", "Orders")
    );
    assert!(
        !both
            .graph()
            .filters_follow_expanded_path("Orders", "Unmatched Stores")
    );

    // Dim2 without a blank row: facts whose key has no Dim match relate to
    // Dim's blank row, which no filter on Dim2 can keep.
    for (blank_row, seed) in [(false, 1u64), (true, 2)] {
        let e = random_star(seed, blank_row);
        let n = e.row_count("Fact").unwrap();
        let all = RowSet::all(n);
        assert_eq!(e.graph().has_blank_row("Dim2"), blank_row);
        assert!(e.graph().has_blank_row("Dim"));
        let reachable = e.graph().rows_reachable_from("Fact", "Dim2", &all);
        let any_z = FilterContext::new().with(ColumnFilter::predicate("Dim2", "Z", |_| true));
        let visible = e.visible_rows(&any_z, "Fact").unwrap();
        assert_eq!(reachable, visible, "Dim2 blank row: {blank_row}");
        assert_eq!(reachable.is_all(), blank_row);
        assert_eq!(e.graph().rows_reachable_from("Fact", "Dim", &all), all);
    }
}
