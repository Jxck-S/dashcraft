//! A miniature store model in memory (no fixture files), in the style of
//! the Acme Outfitters example:
//! `'Orders'[StoreKey] (*) ↔ (1) Stores[StoreKey]` (both directions) plus
//! the unrelated `'Web Sessions'`.
//!
//! Stores (key, name, format, region, staff, opened, size): S1 Store 1
//! Flagship/East/10/2026-01-01/Large, S2 Store 2 Flagship/Central/20/
//! 2026-02-01/Small, S3 Store 3 Pop-up/West/30/2026-03-01/Medium, S4 Store 4
//! Outlet/Central/40/BLANK/Large, S5 Store 5 BLANK/Central/BLANK/2026-05-01/
//! Small. `Size` is sorted by `Size Order` (Small 1, Medium 2, Large 3).
//! Orders (store, payment): S1 Card, S1 Cash, S3 Card, S4 Cash, SX Card (no
//! store row → the Stores blank row).
//! Web Sessions (device): Desktop, Mobile.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use arrow::array::{ArrayRef, Int64Array, StringArray, TimestampMicrosecondArray};
use arrow::record_batch::RecordBatch;
use pbi_engine::{Engine, Value};
use pbi_model::{
    Column, ColumnRef, CrossFilter, DataType, FieldExpr, Literal, Measure, Model, Projection,
    Relationship, Table,
};
use pbi_render::query::QueryEnv;

fn table(name: &str, columns: &[(&str, DataType)]) -> Table {
    let mut t = Table::new(name);
    for (c, dt) in columns {
        t.columns.push(Column::data(*c, dt.clone()));
    }
    t
}

pub fn model() -> Model {
    let mut rel = Relationship::new(
        "r1",
        ColumnRef::new("Orders", "StoreKey"),
        ColumnRef::new("Stores", "StoreKey"),
    );
    rel.cross_filtering_behavior = CrossFilter::BothDirections;
    let mut stores = table(
        "Stores",
        &[
            ("StoreKey", DataType::String),
            ("Store Name", DataType::String),
            ("Format", DataType::String),
            ("Region", DataType::String),
            ("Staff", DataType::Int64),
            ("Opened", DataType::DateTime),
            ("Size", DataType::String),
            ("Size Order", DataType::Int64),
        ],
    );
    if let Some(size) = stores.columns.iter_mut().find(|c| c.name == "Size") {
        size.sort_by_column = Some("Size Order".into());
    }
    let mut add = |name: &str, expr: &str, format: Option<&str>| {
        let mut m = Measure::new(name, expr);
        m.format_string = format.map(str::to_owned);
        stores.measures.push(m);
    };
    add("Store Count", "COUNTROWS(Stores)", Some("0"));
    add("Order Count", "COUNTROWS('Orders') + 0", Some("0"));
    add("Sessions", "COUNTROWS('Web Sessions') + 0", Some("0"));
    add("Staff Sum", "SUM(Stores[Staff])", Some("#,0"));
    add(
        "Share",
        "DIVIDE([Store Count], CALCULATE([Store Count], ALLSELECTED(Stores)))",
        Some("0.0%;-0.0%;0.0%"),
    );
    add("Broken", "NOSUCHFUNCTION(Stores[Staff])", None);
    Model {
        tables: vec![
            stores,
            table(
                "Orders",
                &[
                    ("StoreKey", DataType::String),
                    ("Payment", DataType::String),
                ],
            ),
            table("Web Sessions", &[("Device", DataType::String)]),
        ],
        relationships: vec![rel],
        ..Model::default()
    }
}

fn strings(values: &[Option<&str>]) -> ArrayRef {
    Arc::new(StringArray::from(values.to_vec()))
}

fn ints(values: &[Option<i64>]) -> ArrayRef {
    Arc::new(Int64Array::from(values.to_vec()))
}

fn micros(y: i64, m: u32, d: u32) -> Option<i64> {
    match Value::date(y, m, d) {
        Some(Value::DateTime(us)) => Some(us),
        _ => None,
    }
}

pub fn env() -> QueryEnv {
    let model = model();
    let mut engine = Engine::new(&model);
    let stores = RecordBatch::try_from_iter([
        (
            "StoreKey",
            strings(&[Some("S1"), Some("S2"), Some("S3"), Some("S4"), Some("S5")]),
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
            "Format",
            strings(&[
                Some("Flagship"),
                Some("Flagship"),
                Some("Pop-up"),
                Some("Outlet"),
                None,
            ]),
        ),
        (
            "Region",
            strings(&[
                Some("East"),
                Some("Central"),
                Some("West"),
                Some("Central"),
                Some("Central"),
            ]),
        ),
        (
            "Staff",
            ints(&[Some(10), Some(20), Some(30), Some(40), None]),
        ),
        (
            "Opened",
            Arc::new(TimestampMicrosecondArray::from(vec![
                micros(2026, 1, 1),
                micros(2026, 2, 1),
                micros(2026, 3, 1),
                None,
                micros(2026, 5, 1),
            ])) as ArrayRef,
        ),
        (
            "Size",
            strings(&[
                Some("Large"),
                Some("Small"),
                Some("Medium"),
                Some("Large"),
                Some("Small"),
            ]),
        ),
        (
            "Size Order",
            ints(&[Some(3), Some(1), Some(2), Some(3), Some(1)]),
        ),
    ])
    .unwrap();
    let orders = RecordBatch::try_from_iter([
        (
            "StoreKey",
            strings(&[Some("S1"), Some("S1"), Some("S3"), Some("S4"), Some("SX")]),
        ),
        (
            "Payment",
            strings(&[
                Some("Card"),
                Some("Cash"),
                Some("Card"),
                Some("Cash"),
                Some("Card"),
            ]),
        ),
    ])
    .unwrap();
    let sessions =
        RecordBatch::try_from_iter([("Device", strings(&[Some("Desktop"), Some("Mobile")]))])
            .unwrap();
    engine
        .load_tables([
            ("Stores".to_owned(), stores),
            ("Orders".to_owned(), orders),
            ("Web Sessions".to_owned(), sessions),
        ])
        .unwrap();
    QueryEnv::new(engine.snapshot(), Arc::new(model), "en-US")
}

pub fn col(table: &str, column: &str) -> FieldExpr {
    FieldExpr::column(table, column)
}

pub fn measure(name: &str) -> FieldExpr {
    FieldExpr::measure("Stores", name)
}

pub fn text(s: &str) -> FieldExpr {
    FieldExpr::Literal(Literal::text(s))
}

pub fn lit(raw: &str) -> FieldExpr {
    FieldExpr::Literal(Literal::new(raw))
}

pub fn projection(role: &str, field: FieldExpr) -> Projection {
    Projection {
        role: role.into(),
        query_ref: field.query_ref().unwrap_or_default(),
        field,
        ..Projection::default()
    }
}

/// Visible rows of `table` in `ctx`.
pub fn visible(env: &QueryEnv, ctx: &pbi_engine::FilterContext, table: &str) -> usize {
    env.snapshot.visible_rows(ctx, table).unwrap().count()
}
