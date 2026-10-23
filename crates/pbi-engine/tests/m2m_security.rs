//! Many-to-many (limited) relationships and row-level security masks.
//!
//! Model for the many-to-many cases, two fact-like tables joined on a
//! non-unique key (no bridge table):
//!
//! ```text
//! Sales (from)               Targets (to)
//! row Customer Amount        row Customer Region Target
//! 0   C1       10            0   C1       East   50
//! 1   C1       20            1   C1       West   60
//! 2   C2        5            2   C2       East   30
//! 3   C3        7            3   C4       West   40
//! 4   C9      100
//! ```
//!
//! `Sales[Customer] (*) – (*) Targets[Customer]`: C3 and C9 have no target,
//! C4 has no sale.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use arrow::array::{ArrayRef, Int64Array, StringArray};
use arrow::record_batch::RecordBatch;
use pbi_engine::{
    Agg, ColumnFilter, Engine, FilterContext, RoleFilter, RowSet, SecurityContext, SecurityRole,
    TableFilter, UserIdentity, Value,
};
use pbi_model::{
    Cardinality, Column, ColumnRef, CrossFilter, DataType, Model, Relationship,
    SecurityFilteringBehavior, Severity, Table,
};
use pretty_assertions::assert_eq;

fn table(name: &str, columns: &[(&str, DataType)]) -> Table {
    let mut t = Table::new(name);
    for (c, dt) in columns {
        t.columns.push(Column::data(*c, dt.clone()));
    }
    t
}

fn text(values: &[&str]) -> ArrayRef {
    Arc::new(StringArray::from(values.to_vec()))
}

fn int(values: &[i64]) -> ArrayRef {
    Arc::new(Int64Array::from(values.to_vec()))
}

fn batch(columns: Vec<(&str, ArrayRef)>) -> RecordBatch {
    RecordBatch::try_from_iter(columns).unwrap()
}

fn m2m_engine(cross_filter: CrossFilter) -> Engine {
    let mut rel = Relationship::new(
        "m2m",
        ColumnRef::new("Sales", "Customer"),
        ColumnRef::new("Targets", "Customer"),
    );
    rel.from_cardinality = Cardinality::Many;
    rel.to_cardinality = Cardinality::Many;
    rel.cross_filtering_behavior = cross_filter;
    let model = Model {
        tables: vec![
            table(
                "Sales",
                &[("Customer", DataType::String), ("Amount", DataType::Int64)],
            ),
            table(
                "Targets",
                &[
                    ("Customer", DataType::String),
                    ("Region", DataType::String),
                    ("Target", DataType::Int64),
                ],
            ),
        ],
        relationships: vec![rel],
        ..Model::default()
    };
    let mut engine = Engine::new(&model);
    engine
        .load_table(
            "Sales",
            batch(vec![
                ("Customer", text(&["C1", "C1", "C2", "C3", "C9"])),
                ("Amount", int(&[10, 20, 5, 7, 100])),
            ]),
        )
        .unwrap();
    engine
        .load_table(
            "Targets",
            batch(vec![
                ("Customer", text(&["C1", "C1", "C2", "C4"])),
                ("Region", text(&["East", "West", "East", "West"])),
                ("Target", int(&[50, 60, 30, 40])),
            ]),
        )
        .unwrap();
    engine
}

fn sum(engine: &Engine, ctx: &FilterContext, table: &str, column: &str) -> Value {
    let rows = engine.visible_rows(ctx, table).unwrap();
    engine.aggregate(table, column, &rows, Agg::Sum).unwrap()
}

fn region(name: &str) -> FilterContext {
    FilterContext::new().with(ColumnFilter::equals("Targets", "Region", name))
}

#[test]
fn many_to_many_filters_cross_as_key_values() {
    let engine = m2m_engine(CrossFilter::OneDirection);
    // East targets: C1, C2 → sales of C1 and C2 = 10 + 20 + 5.
    assert_eq!(
        sum(&engine, &region("East"), "Sales", "Amount"),
        Value::Integer(35)
    );
    // West targets: C1, C4 → sales of C1 = 30 (C4 has none).
    assert_eq!(
        sum(&engine, &region("West"), "Sales", "Amount"),
        Value::Integer(30)
    );
    // The total is not the sum of the regions (C1 is in both) and includes
    // the sales without a target (C3, C9): an unfiltered side does not
    // restrict the other.
    assert_eq!(
        sum(&engine, &FilterContext::new(), "Sales", "Amount"),
        Value::Integer(142)
    );
    // Any filter on Targets hides the sales whose key has no target, even
    // one that keeps every Targets row.
    let all_targets = FilterContext::new().with(TableFilter::new("Targets", RowSet::all(4)));
    assert_eq!(
        sum(&engine, &all_targets, "Sales", "Amount"),
        Value::Integer(35)
    );
    // Single direction: Sales does not filter Targets.
    let big = FilterContext::new().with(ColumnFilter::predicate("Sales", "Amount", |v| {
        v.as_f64().is_some_and(|x| x >= 20.0)
    }));
    assert_eq!(sum(&engine, &big, "Targets", "Target"), Value::Integer(180));
    // Limited relationship: no blank row on either side.
    assert!(
        !engine
            .blank_row_visible(&region("East"), "Targets")
            .unwrap()
    );
    assert!(!engine.graph().has_blank_row("Sales"));
    assert!(!engine.graph().has_blank_row("Targets"));
    let diags = engine.diagnostics();
    assert_eq!(diags.len(), 1);
    assert_eq!(
        (diags[0].code.as_str(), diags[0].severity),
        ("ENG003", Severity::Info)
    );
}

#[test]
fn bidirectional_many_to_many_filters_both_ways() {
    let engine = m2m_engine(CrossFilter::BothDirections);
    // Sales ≥ 20: rows of C1 (20) and C9 (100) → targets of C1 = 50 + 60.
    let big = FilterContext::new().with(ColumnFilter::predicate("Sales", "Amount", |v| {
        v.as_f64().is_some_and(|x| x >= 20.0)
    }));
    assert_eq!(sum(&engine, &big, "Targets", "Target"), Value::Integer(110));
    // The other direction still works.
    assert_eq!(
        sum(&engine, &region("East"), "Sales", "Amount"),
        Value::Integer(35)
    );
    assert_eq!(
        engine.values(&big, "Targets", "Region").unwrap(),
        [Value::string("East"), Value::string("West")]
    );
}

/// `Region (1) → (*) Sales (*) → (1) Customer`; Sales → Customer is
/// bidirectional.
///
/// ```text
/// Sales row Region Customer Amount   Customer Segment
/// 0         East   C1       10       C1       Retail
/// 1         West   C1       20       C2       Corporate
/// 2         East   C2        5       C3       Retail
/// 3         North  C3        7
/// 4         West   C3        3
/// ```
fn star(security: SecurityFilteringBehavior) -> Engine {
    let to_region = Relationship::new(
        "r1",
        ColumnRef::new("Sales", "Region"),
        ColumnRef::new("Region", "Region"),
    );
    let mut to_customer = Relationship::new(
        "r2",
        ColumnRef::new("Sales", "Customer"),
        ColumnRef::new("Customer", "Customer"),
    );
    to_customer.cross_filtering_behavior = CrossFilter::BothDirections;
    to_customer.security_filtering_behavior = security;
    let model = Model {
        tables: vec![
            table("Region", &[("Region", DataType::String)]),
            table(
                "Customer",
                &[
                    ("Customer", DataType::String),
                    ("Segment", DataType::String),
                ],
            ),
            table(
                "Sales",
                &[
                    ("Region", DataType::String),
                    ("Customer", DataType::String),
                    ("Amount", DataType::Int64),
                ],
            ),
        ],
        relationships: vec![to_region, to_customer],
        ..Model::default()
    };
    let mut engine = Engine::new(&model);
    engine
        .load_table(
            "Region",
            batch(vec![("Region", text(&["East", "West", "North"]))]),
        )
        .unwrap();
    engine
        .load_table(
            "Customer",
            batch(vec![
                ("Customer", text(&["C1", "C2", "C3"])),
                ("Segment", text(&["Retail", "Corporate", "Retail"])),
            ]),
        )
        .unwrap();
    engine
        .load_table(
            "Sales",
            batch(vec![
                ("Region", text(&["East", "West", "East", "North", "West"])),
                ("Customer", text(&["C1", "C1", "C2", "C3", "C3"])),
                ("Amount", int(&[10, 20, 5, 7, 3])),
            ]),
        )
        .unwrap();
    engine
}

fn role(name: &str, filters: &[(&str, &[usize], usize)]) -> SecurityRole {
    SecurityRole {
        name: name.into(),
        filters: filters
            .iter()
            .map(|(t, rows, n)| RoleFilter {
                table: (*t).into(),
                rows: RowSet::from_rows(*n, rows.iter().copied()),
                blank_row: false,
            })
            .collect(),
    }
}

fn rows(masks: &std::collections::HashMap<String, pbi_engine::VisibleRows>, t: &str) -> Vec<usize> {
    masks[&t.to_lowercase()].rows.iter().collect()
}

#[test]
fn security_filters_propagate_in_their_direction_and_roles_union() {
    let east = role("East", &[("Region", &[0], 3)]);
    let retail = role("Retail", &[("Customer", &[0, 2], 3)]);

    // One direction (default): Region → Sales; Sales does not restrict
    // Customer although cross-filtering is bidirectional.
    let engine = star(SecurityFilteringBehavior::OneDirection);
    let masks = engine.security_masks(std::slice::from_ref(&east)).unwrap();
    assert_eq!(rows(&masks, "Region"), [0]);
    assert_eq!(rows(&masks, "Sales"), [0, 2]);
    assert_eq!(rows(&masks, "Customer"), [0, 1, 2]);

    // Both directions: Sales' East rows (C1, C2) restrict Customer.
    let engine = star(SecurityFilteringBehavior::BothDirections);
    let masks = engine.security_masks(std::slice::from_ref(&east)).unwrap();
    assert_eq!(rows(&masks, "Customer"), [0, 1]);

    // Union of two roles: East allows Sales {0, 2}, Retail allows Sales of
    // C1/C3 {0, 1, 3, 4} → every sale; Region: East's {East} ∪ Retail's
    // (unrestricted) → all.
    let engine = star(SecurityFilteringBehavior::OneDirection);
    let masks = engine.security_masks(&[east.clone(), retail]).unwrap();
    assert_eq!(rows(&masks, "Sales"), [0, 1, 2, 3, 4]);
    assert_eq!(rows(&masks, "Region"), [0, 1, 2]);

    // No role: nothing visible.
    let masks = engine.security_masks(&[]).unwrap();
    assert!(rows(&masks, "Sales").is_empty());
}

#[test]
fn restricted_engines_hide_rows_from_every_query() {
    let mut engine = star(SecurityFilteringBehavior::OneDirection);
    engine.set_security_context(Some(SecurityContext {
        roles: vec!["East".into()],
        identity: UserIdentity {
            user_name: Some("ann@contoso.com".into()),
            ..UserIdentity::default()
        },
    }));
    let east = role("East", &[("Region", &[0], 3)]);
    let secured = engine.with_row_security(&[east]).unwrap();
    assert_eq!(secured.row_count("Sales").unwrap(), 2);
    assert_eq!(secured.row_count("Region").unwrap(), 1);
    assert_eq!(secured.row_count("Customer").unwrap(), 3);
    // Filters cannot bring rows back (security is not a filter).
    assert_eq!(
        sum(&secured, &FilterContext::new(), "Sales", "Amount"),
        Value::Integer(15)
    );
    let west = FilterContext::new().with(ColumnFilter::equals("Region", "Region", "West"));
    assert_eq!(sum(&secured, &west, "Sales", "Amount"), Value::Blank);
    // Relationships still work on the restricted data, without blank rows.
    let c1 = FilterContext::new().with(ColumnFilter::equals("Customer", "Customer", "C1"));
    assert_eq!(sum(&secured, &c1, "Sales", "Amount"), Value::Integer(10));
    assert!(!secured.graph().has_blank_row("Region"));
    assert_eq!(
        secured.security().unwrap().identity.user_name.as_deref(),
        Some("ann@contoso.com")
    );
    // The source engine is unchanged.
    assert_eq!(engine.row_count("Sales").unwrap(), 5);
}

#[test]
fn one_to_one_relationships_filter_both_ways() {
    // Person (from) 1 – 1 Badge (to), single direction in the model: a
    // one-to-one relationship still cross-filters both ways and expands
    // both ways. Person 3 has no badge; badge 9 has no person.
    let mut rel = Relationship::new(
        "one",
        ColumnRef::new("Person", "Id"),
        ColumnRef::new("Badge", "Id"),
    );
    rel.from_cardinality = Cardinality::One;
    rel.to_cardinality = Cardinality::One;
    let model = Model {
        tables: vec![
            table(
                "Person",
                &[("Id", DataType::Int64), ("Name", DataType::String)],
            ),
            table(
                "Badge",
                &[("Id", DataType::Int64), ("Color", DataType::String)],
            ),
        ],
        relationships: vec![rel],
        ..Model::default()
    };
    let mut engine = Engine::new(&model);
    engine
        .load_table(
            "Person",
            batch(vec![
                ("Id", int(&[1, 2, 3])),
                ("Name", text(&["A", "B", "C"])),
            ]),
        )
        .unwrap();
    engine
        .load_table(
            "Badge",
            batch(vec![
                ("Id", int(&[1, 2, 9])),
                ("Color", text(&["Red", "Blue", "Red"])),
            ]),
        )
        .unwrap();
    let red = FilterContext::new().with(ColumnFilter::equals("Badge", "Color", "Red"));
    // Red badge 9 has no person: it relates to Person's blank row.
    assert_eq!(
        engine.values(&red, "Person", "Name").unwrap(),
        [Value::string("A"), Value::Blank]
    );
    let b = FilterContext::new().with(ColumnFilter::equals("Person", "Name", "B"));
    assert_eq!(
        engine.values(&b, "Badge", "Color").unwrap(),
        [Value::string("Blue")]
    );
    assert!(engine.graph().expanded_path("Badge", "Person").is_some());
    assert!(engine.graph().expanded_path("Person", "Badge").is_some());

    // A table filter filters its expanded table, the other side of a
    // one-to-one relationship included. Badge 9 has no person and relates
    // to Person's blank row, which keeps badge 9 visible when Person
    // cross-filters Badge back; person 3 (C) likewise.
    let ids = |ctx: &FilterContext| engine.values(ctx, "Badge", "Id").unwrap();
    let badges = |rows: &[usize]| {
        FilterContext::new().with(TableFilter::new(
            "Badge",
            RowSet::from_rows(3, rows.iter().copied()),
        ))
    };
    assert_eq!(
        ids(&badges(&[0, 2])),
        [Value::Integer(1), Value::Integer(9)]
    );
    assert_eq!(ids(&badges(&[2])), [Value::Integer(9)]);
    assert_eq!(
        engine.values(&badges(&[0, 2]), "Person", "Name").unwrap(),
        [Value::string("A"), Value::Blank]
    );
    let people =
        FilterContext::new().with(TableFilter::new("Person", RowSet::from_rows(3, [0, 2])));
    assert_eq!(
        engine.values(&people, "Person", "Name").unwrap(),
        [Value::string("A"), Value::string("C")]
    );
    let visible = engine.visible(&people, "Badge").unwrap();
    assert_eq!(visible.rows.iter().collect::<Vec<_>>(), [0]);
    assert!(visible.blank_row);
}
