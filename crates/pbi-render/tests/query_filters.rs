//! FilterBuilder: report-AST conditions → engine filters, on an in-memory
//! miniature of an Acme-style store model (no fixture files needed).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod query_support;

use pbi_engine::{FilterContext, Value};
use pbi_model::{ComparisonKind, Condition, FieldExpr, Filter, FilterKind, Literal};
use pretty_assertions::assert_eq;
use query_support::{col, env, lit, measure, text, visible};

fn in_values(table: &str, column: &str, values: &[&str]) -> Condition {
    Condition::In {
        exprs: vec![col(table, column)],
        values: values.iter().map(|v| vec![Literal::new(*v)]).collect(),
    }
}

fn cmp(kind: ComparisonKind, left: FieldExpr, right: FieldExpr) -> Condition {
    Condition::Comparison { kind, left, right }
}

fn not(c: Condition) -> Condition {
    Condition::Not(Box::new(c))
}

/// Visible (Stores, Orders) rows under `condition`.
fn counts(condition: &Condition) -> (usize, usize) {
    let env = env();
    let mut ctx = FilterContext::new();
    env.builder()
        .add_condition(condition, &mut ctx)
        .unwrap_or_else(|e| panic!("{condition:?}: {e}"));
    (visible(&env, &ctx, "Stores"), visible(&env, &ctx, "Orders"))
}

#[test]
fn categorical_in_and_not_in() {
    let flagship = in_values("Stores", "Format", &["'flagship'"]);
    // Text matches case-insensitively; the unmatched order (blank row) is hidden.
    assert_eq!(counts(&flagship), (2, 2));
    // NOT IN keeps BLANK format and the blank row's unmatched order.
    assert_eq!(counts(&not(flagship)), (3, 3));
    let blank = in_values("Stores", "Format", &["null"]);
    assert_eq!(counts(&blank), (1, 1));
}

#[test]
fn advanced_comparisons_follow_dax_semantics() {
    let staff = || col("Stores", "Staff");
    assert_eq!(
        counts(&cmp(ComparisonKind::GreaterThan, staff(), lit("15L"))).0,
        3
    );
    // BLANK compares as 0, so it is <= 20.
    assert_eq!(
        counts(&cmp(ComparisonKind::LessThanOrEqual, staff(), lit("20L"))).0,
        3
    );
    // `<>` is Not(=).
    assert_eq!(
        counts(&not(cmp(ComparisonKind::Equal, staff(), lit("10L")))).0,
        4
    );
    // Literal on the left is flipped.
    assert_eq!(
        counts(&cmp(ComparisonKind::LessThan, lit("35D"), staff())).0,
        1
    );
    // Text literals coerce to the integer column.
    assert_eq!(
        counts(&cmp(ComparisonKind::Equal, staff(), text("20"))).0,
        1
    );
    let between = Condition::Between {
        expr: staff(),
        lower: lit("15L"),
        upper: lit("35L"),
    };
    assert_eq!(counts(&between).0, 2);
}

#[test]
fn is_blank_and_is_not_blank() {
    let is_blank = cmp(ComparisonKind::Equal, col("Stores", "Staff"), lit("null"));
    assert_eq!(counts(&is_blank).0, 1);
    assert_eq!(counts(&not(is_blank)).0, 4);
}

#[test]
fn and_or_not_trees() {
    let flagship = cmp(
        ComparisonKind::Equal,
        col("Stores", "Format"),
        text("Flagship"),
    );
    let high = cmp(
        ComparisonKind::GreaterThanOrEqual,
        col("Stores", "Staff"),
        lit("20L"),
    );
    let and = Condition::And(Box::new(flagship.clone()), Box::new(high.clone()));
    assert_eq!(counts(&and).0, 1);
    // OR across two columns of one table: a scanned table filter.
    let east = cmp(ComparisonKind::Equal, col("Stores", "Region"), text("East"));
    let pop_up = cmp(
        ComparisonKind::Equal,
        col("Stores", "Format"),
        text("Pop-up"),
    );
    let or = Condition::Or(Box::new(east), Box::new(pop_up));
    assert_eq!(counts(&or), (2, 3));
    // OR on one column stays a column predicate.
    let either = Condition::Or(
        Box::new(flagship),
        Box::new(cmp(
            ComparisonKind::Equal,
            col("Stores", "Format"),
            text("Pop-up"),
        )),
    );
    assert_eq!(counts(&either).0, 3);
    assert_eq!(counts(&not(either)).0, 2);
}

#[test]
fn text_filters() {
    let format = || col("Stores", "Format");
    let contains = Condition::Contains {
        left: format(),
        right: text("SHIP"),
    };
    assert_eq!(counts(&contains).0, 2);
    let starts = Condition::StartsWith {
        left: format(),
        right: text("p"),
    };
    assert_eq!(counts(&starts).0, 1);
    // "does not contain" keeps BLANK.
    assert_eq!(counts(&not(contains)).0, 3);
    let empty = cmp(ComparisonKind::Equal, format(), text(""));
    // DAX `=`: BLANK equals "".
    assert_eq!(counts(&empty).0, 1);
}

#[test]
fn datetime_literals() {
    let when = cmp(
        ComparisonKind::Equal,
        col("Stores", "Opened"),
        lit("datetime'2026-02-01T00:00:00'"),
    );
    assert_eq!(counts(&when).0, 1);
    let after = cmp(
        ComparisonKind::GreaterThan,
        col("Stores", "Opened"),
        text("2026-02-15"),
    );
    assert_eq!(counts(&after).0, 2);
}

#[test]
fn multi_column_tuples() {
    let tuple = Condition::In {
        exprs: vec![col("Stores", "Format"), col("Stores", "Region")],
        values: vec![
            vec![Literal::text("Flagship"), Literal::text("East")],
            vec![Literal::text("Pop-up"), Literal::text("West")],
        ],
    };
    assert_eq!(counts(&tuple), (2, 3));
    // Columns of related tables: rows of 'Orders' (whose expanded
    // table holds Stores) are matched.
    let cross = Condition::In {
        exprs: vec![col("Stores", "Format"), col("Orders", "Payment")],
        values: vec![vec![Literal::text("Flagship"), Literal::text("Cash")]],
    };
    assert_eq!(counts(&cross), (1, 1));
}

#[test]
fn relationship_path_rule() {
    let env = env();
    let mut ctx = FilterContext::new();
    env.builder()
        .add_condition(&in_values("Stores", "Format", &["'Flagship'"]), &mut ctx)
        .unwrap();
    let eval = |m: &str| pbi_dax::evaluate_measure(&env.snapshot, &env.model, m, &ctx).unwrap();
    assert_eq!(eval("Store Count"), Value::Integer(2));
    assert_eq!(eval("Sessions"), Value::Integer(2));
}

#[test]
fn plans_separate_measure_topn_and_unsupported_filters() {
    let env = env();
    let b = env.builder();
    let filter = |kind: FilterKind, condition: Option<Condition>| Filter {
        name: "f".into(),
        kind,
        condition,
        ..Filter::default()
    };
    let measure_cond = cmp(
        ComparisonKind::GreaterThan,
        measure("Order Count"),
        lit("1L"),
    );
    let mixed = Condition::And(
        Box::new(in_values("Stores", "Format", &["'Pop-up'"])),
        Box::new(measure_cond.clone()),
    );
    let plan = b.plan(&filter(FilterKind::Advanced, Some(mixed)));
    assert_eq!(plan.filters.len(), 1);
    assert_eq!(plan.value_conditions, vec![measure_cond]);
    assert!(plan.skipped.is_empty());

    let relative = b.plan(&filter(
        FilterKind::RelativeDate,
        Some(in_values("Stores", "Opened", &["null"])),
    ));
    assert!(relative.is_empty());
    assert_eq!(relative.skipped.len(), 1);

    let unknown = b.plan(&filter(
        FilterKind::Advanced,
        Some(Condition::Unknown(serde_json::json!({"Exists": {}}))),
    ));
    assert_eq!(unknown.skipped, vec!["unsupported filter condition"]);

    let missing = b.plan(&filter(
        FilterKind::Categorical,
        Some(in_values("Stores", "Nope", &["'x'"])),
    ));
    assert_eq!(missing.skipped.len(), 1);
    assert!(missing.skipped[0].contains("Nope"), "{missing:?}");

    // No condition: the filter card does not filter.
    assert!(b.plan(&filter(FilterKind::Categorical, None)).is_empty());
}

#[test]
fn tuple_filters_from_ui_values_coerce_types() {
    let env = env();
    let b = env.builder();
    let (filters, approx) = b
        .tuple_filter(
            &[col("Stores", "Staff")],
            &[vec![Value::from("20")], vec![Value::Integer(30)]],
            false,
        )
        .unwrap();
    assert!(approx.is_empty());
    let mut ctx = FilterContext::new();
    for f in filters {
        ctx.add(f);
    }
    assert_eq!(visible(&env, &ctx, "Stores"), 2);
    assert!(
        b.tuple_filter(&[col("Stores", "Staff")], &[vec![]], false)
            .is_err()
    );
}
