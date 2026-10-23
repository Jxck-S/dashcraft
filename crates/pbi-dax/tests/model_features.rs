#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Calculation groups, many-to-many relationships and the effective user,
//! on small hand-built models. Expected values are derived from the
//! documented semantics (Microsoft Learn "Calculation groups", "Model
//! relationships"; SQLBI on sideways recursion) and spelled out per case.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::data::{batch, datetimes, ints, texts};
use pbi_dax::{DaxError, EvalContext, evaluate_measure, evaluate_scalar, format_value};
use pbi_engine::{ColumnFilter, Engine, FilterContext, SecurityContext, UserIdentity, Value};
use pbi_model::{
    CalculationGroup, CalculationGroupExpression, CalculationItem, Cardinality, Column, ColumnKind,
    ColumnRef, CrossFilter, DataType, Measure, Model, PropValue, Relationship, Table,
};
use pretty_assertions::assert_eq;

// ---- calculation groups --------------------------------------------------------

/// A calculation-group table: `Name` (`sourceColumn: Name`) and `Ordinal`.
fn calc_table(name: &str, precedence: i64, items: &[(&str, &str, Option<&str>)]) -> Table {
    let mut t = Table::new(name);
    let mut col = Column::data("Name", DataType::String);
    col.kind = ColumnKind::Data {
        source_column: "Name".into(),
    };
    t.columns = vec![col, Column::data("Ordinal", DataType::Int64)];
    t.calculation_group = Some(Box::new(CalculationGroup {
        precedence,
        calculation_items: items
            .iter()
            .enumerate()
            .map(|(n, (item, expr, format))| CalculationItem {
                ordinal: Some(n as i64),
                format_string_definition: format.map(str::to_owned),
                ..CalculationItem::new(*item, *expr)
            })
            .collect(),
        ..CalculationGroup::default()
    }));
    t
}

fn measure(name: &str, expr: &str, format: Option<&str>) -> Measure {
    Measure {
        format_string: format.map(str::to_owned),
        ..Measure::new(name, expr)
    }
}

/// `Date` (every day of 2023 and 2024) ← `Sales`, plus three calculation
/// groups: `Time Calc` (precedence 20), `Averages` (10) and `Meta` (5).
///
/// ```text
/// Sales: 2023-01-10 100   2023-02-15 50   2023-03-20 30
///        2024-01-05 200   2024-02-10 80   2024-03-15 40
/// ```
fn model(averages_precedence: i64) -> Model {
    let mut date = Table::new("Date");
    date.columns = vec![
        Column::data("Date", DataType::DateTime),
        Column::data("Year", DataType::Int64),
        Column::data("Month", DataType::Int64),
    ];
    let mut sales = Table::new("Sales");
    sales.columns = vec![
        Column::data("Date", DataType::DateTime),
        Column::data("Amount", DataType::Int64),
    ];
    let mut dynamic = measure("Dyn", "[Sales]", None);
    dynamic.extra.insert(
        "formatStringDefinition".into(),
        PropValue::Expression("IF(SELECTEDMEASURE() > 100, \"#,0.00\", \"0\")".into()),
    );
    sales.measures = vec![
        measure("Sales", "SUM(Sales[Amount])", Some("#,0")),
        measure("Units", "COUNTROWS(Sales)", Some("0")),
        measure("Sales x2", "[Sales] * 2", None),
        measure("Sum Monthly", "SUMX(VALUES('Date'[Month]), [Sales])", None),
        measure("Big", "[Sales] * 100", Some("#,0")),
        measure("Bad", "SELECTEDMEASURE()", None),
        dynamic,
    ];
    let time = calc_table(
        "Time Calc",
        20,
        &[
            ("Current", "SELECTEDMEASURE()", None),
            (
                "YTD",
                "CALCULATE(SELECTEDMEASURE(), DATESYTD('Date'[Date]))",
                None,
            ),
            (
                "PY",
                "CALCULATE(SELECTEDMEASURE(), SAMEPERIODLASTYEAR('Date'[Date]))",
                None,
            ),
            (
                "PY YTD",
                "CALCULATE(SELECTEDMEASURE(), SAMEPERIODLASTYEAR('Date'[Date]), 'Time Calc'[Name] = \"YTD\")",
                None,
            ),
            (
                "YOY %",
                "VAR py = CALCULATE(SELECTEDMEASURE(), 'Time Calc'[Name] = \"PY\") RETURN DIVIDE(SELECTEDMEASURE() - py, py)",
                Some("\"0.0%\""),
            ),
        ],
    );
    let averages = calc_table(
        "Averages",
        averages_precedence,
        &[
            ("No Average", "SELECTEDMEASURE()", None),
            (
                "Daily Average",
                "DIVIDE(SELECTEDMEASURE(), COUNTROWS('Date'))",
                None,
            ),
        ],
    );
    let mut meta = calc_table(
        "Meta",
        5,
        &[
            ("Name", "SELECTEDMEASURENAME()", None),
            (
                "Only Sales",
                "IF(ISSELECTEDMEASURE([Sales], [Big]), SELECTEDMEASURE())",
                None,
            ),
            ("Fmt", "SELECTEDMEASUREFORMATSTRING()", None),
            (
                "Text",
                "FORMAT(SELECTEDMEASURE(), SELECTEDMEASUREFORMATSTRING())",
                None,
            ),
        ],
    );
    if let Some(g) = &mut meta.calculation_group {
        g.multiple_or_empty_selection_expression =
            Some(CalculationGroupExpression::new("\"multi\""));
    }
    Model {
        tables: vec![date, sales, time, averages, meta],
        relationships: vec![Relationship::new(
            "r",
            ColumnRef::new("Sales", "Date"),
            ColumnRef::new("Date", "Date"),
        )],
        ..Model::default()
    }
}

fn days() -> Vec<(i64, u32, u32)> {
    let mut out = Vec::new();
    for year in [2023, 2024] {
        for month in 1..=12u32 {
            let n = match month {
                2 if year == 2024 => 29,
                2 => 28,
                4 | 6 | 9 | 11 => 30,
                _ => 31,
            };
            out.extend((1..=n).map(|d| (year, month, d)));
        }
    }
    out
}

/// Loads the rows of a calculation-group table: one per item.
fn load_calc_table(engine: &mut Engine, model: &Model, name: &str) {
    let g = model
        .table(name)
        .unwrap()
        .calculation_group
        .as_ref()
        .unwrap();
    let names: Vec<&str> = g
        .calculation_items
        .iter()
        .map(|i| i.name.as_str())
        .collect();
    let ordinals: Vec<Option<i64>> = g.calculation_items.iter().map(|i| i.ordinal).collect();
    engine
        .load_table(
            name,
            batch(vec![("Name", texts(&names)), ("Ordinal", ints(&ordinals))]),
        )
        .unwrap();
}

fn engine(model: &Model) -> Engine {
    let mut engine = Engine::new(model);
    let days = days();
    engine
        .load_table(
            "Date",
            batch(vec![
                (
                    "Date",
                    datetimes(
                        &days
                            .iter()
                            .map(|(y, m, d)| Some((*y, *m, *d, 0, 0, 0)))
                            .collect::<Vec<_>>(),
                    ),
                ),
                (
                    "Year",
                    ints(&days.iter().map(|(y, _, _)| Some(*y)).collect::<Vec<_>>()),
                ),
                (
                    "Month",
                    ints(
                        &days
                            .iter()
                            .map(|(_, m, _)| Some(i64::from(*m)))
                            .collect::<Vec<_>>(),
                    ),
                ),
            ]),
        )
        .unwrap();
    engine
        .load_table(
            "Sales",
            batch(vec![
                (
                    "Date",
                    datetimes(&[
                        Some((2023, 1, 10, 0, 0, 0)),
                        Some((2023, 2, 15, 0, 0, 0)),
                        Some((2023, 3, 20, 0, 0, 0)),
                        Some((2024, 1, 5, 0, 0, 0)),
                        Some((2024, 2, 10, 0, 0, 0)),
                        Some((2024, 3, 15, 0, 0, 0)),
                    ]),
                ),
                (
                    "Amount",
                    ints(&[Some(100), Some(50), Some(30), Some(200), Some(80), Some(40)]),
                ),
            ]),
        )
        .unwrap();
    for t in ["Time Calc", "Averages", "Meta"] {
        load_calc_table(&mut engine, model, t);
    }
    engine
}

struct Fx {
    engine: Engine,
    model: Model,
}

impl Fx {
    fn new(averages_precedence: i64) -> Self {
        let model = model(averages_precedence);
        Self {
            engine: engine(&model),
            model,
        }
    }

    fn measure(&self, name: &str, filter: &FilterContext) -> Value {
        evaluate_measure(&self.engine.snapshot(), &self.model, name, filter)
            .unwrap_or_else(|e| panic!("[{name}]: {e}"))
    }

    fn scalar(&self, src: &str, filter: &FilterContext) -> Value {
        evaluate_scalar(&self.engine.snapshot(), &self.model, src, filter)
            .unwrap_or_else(|e| panic!("{src}: {e}"))
    }

    fn ctx(&self, filter: &FilterContext) -> EvalContext {
        EvalContext::new(self.engine.snapshot(), Arc::new(self.model.clone()))
            .with_filter(filter.clone())
    }

    /// `(value, formatted with the effective format string)`.
    fn formatted(&self, name: &str, filter: &FilterContext) -> (Value, Option<String>, String) {
        let (v, f) = self.ctx(filter).evaluate_measure_formatted(name).unwrap();
        let text = format_value(&v, f.as_deref(), "en-US");
        (v, f, text)
    }

    fn query(&self, src: &str, filter: &FilterContext) -> Vec<Vec<Value>> {
        let tables = self.ctx(filter).evaluate_query(src).unwrap();
        tables[0].rows()
    }
}

/// February 2024 (`Date[Year] = 2024`, `Date[Month] = 2`) plus a selection
/// on each named calculation group.
fn feb_2024(selections: &[(&str, &str)]) -> FilterContext {
    let mut f = FilterContext::new()
        .with(ColumnFilter::equals("Date", "Year", Value::Integer(2024)))
        .with(ColumnFilter::equals("Date", "Month", Value::Integer(2)));
    for (group, item) in selections {
        f.add(ColumnFilter::equals(*group, "Name", *item));
    }
    f
}

fn i(v: i64) -> Value {
    Value::Integer(v)
}

#[test]
fn a_single_selected_item_replaces_the_measure() {
    let fx = Fx::new(10);
    // Feb 2024 sales: 80. YTD = Jan + Feb 2024 = 200 + 80. PY = Feb 2023 =
    // 50. PY YTD: SAMEPERIODLASTYEAR moves to Feb 2023, then (sideways
    // recursion) the YTD item applies there: Jan + Feb 2023 = 100 + 50.
    // YOY % = (80 - 50) / 50.
    let cases = [
        ("Current", i(80)),
        ("YTD", i(280)),
        ("PY", i(50)),
        ("PY YTD", i(150)),
        ("YOY %", Value::Double(0.6)),
    ];
    for (item, expected) in cases {
        assert_eq!(
            fx.measure("Sales", &feb_2024(&[("Time Calc", item)])),
            expected,
            "{item}"
        );
    }
    // Any measure: COUNTROWS through YTD = 2 sales rows.
    assert_eq!(
        fx.measure("Units", &feb_2024(&[("Time Calc", "YTD")])),
        i(2)
    );
    // No selection: the measure itself.
    assert_eq!(fx.measure("Sales", &feb_2024(&[])), i(80));
    // Several items or an unknown item, and no selection expression: the
    // group does not apply.
    let mut both = feb_2024(&[]);
    both.add(ColumnFilter::in_values(
        "Time Calc",
        "Name",
        [Value::string("YTD"), Value::string("PY")],
    ));
    assert_eq!(fx.measure("Sales", &both), i(80));
    assert_eq!(
        fx.measure("Sales", &feb_2024(&[("Time Calc", "Nope")])),
        i(80)
    );
}

#[test]
fn calculate_selects_an_item_for_explicit_measures_only() {
    let fx = Fx::new(10);
    let f = feb_2024(&[]);
    assert_eq!(
        fx.scalar("CALCULATE([Sales], 'Time Calc'[Name] = \"YTD\")", &f),
        i(280)
    );
    // An inline aggregation is not a measure reference: no item applies.
    assert_eq!(
        fx.scalar(
            "CALCULATE(SUM(Sales[Amount]), 'Time Calc'[Name] = \"YTD\")",
            &f
        ),
        i(80)
    );
    // Items apply to the measure reference, not again to the measures it
    // references: PY applied to [Sales x2] moves to Feb 2023 once (50 × 2);
    // re-applying it to the inner [Sales] would move to 2022 (BLANK).
    assert_eq!(
        fx.measure("Sales x2", &feb_2024(&[("Time Calc", "PY")])),
        i(100)
    );
    // [Sum Monthly] = SUMX(VALUES(Date[Month]), [Sales]) with YTD selected
    // over 2024: YTD applies to [Sum Monthly] (Jan 1 – Dec 31 2024), and
    // not again to the inner [Sales] of each month: 200 + 80 + 40.
    let year = FilterContext::new()
        .with(ColumnFilter::equals("Date", "Year", Value::Integer(2024)))
        .with(ColumnFilter::equals("Time Calc", "Name", "YTD"));
    assert_eq!(fx.measure("Sum Monthly", &year), i(320));
    // The same iterator inline: each [Sales] reference gets YTD applied
    // for its month: Jan 200, Feb 280, Mar–Dec 320 each → 200 + 280 + 10 ×
    // 320. The columnar SUMX fast path (no measure reference) is not
    // affected.
    assert_eq!(
        fx.scalar("SUMX(VALUES('Date'[Month]), [Sales])", &year),
        i(3680)
    );
    assert_eq!(fx.scalar("SUMX(Sales, Sales[Amount])", &year), i(320));
}

#[test]
fn precedence_orders_several_groups() {
    // YTD (precedence 20) wraps Daily Average (10):
    // CALCULATE(DIVIDE([Sales], COUNTROWS('Date')), DATESYTD(...)) =
    // 280 / (31 + 29 days).
    let fx = Fx::new(10);
    let f = feb_2024(&[("Time Calc", "YTD"), ("Averages", "Daily Average")]);
    assert_eq!(fx.measure("Sales", &f), Value::Double(280.0 / 60.0));
    // Daily Average alone: 80 / 29 days of February 2024.
    let f = feb_2024(&[("Averages", "Daily Average")]);
    assert_eq!(fx.measure("Sales", &f), Value::Double(80.0 / 29.0));
    // With Averages at precedence 30 it wraps YTD instead:
    // DIVIDE(CALCULATE([Sales], DATESYTD(...)), COUNTROWS('Date')) = 280 / 29.
    let fx = Fx::new(30);
    let f = feb_2024(&[("Time Calc", "YTD"), ("Averages", "Daily Average")]);
    assert_eq!(fx.measure("Sales", &f), Value::Double(280.0 / 29.0));
}

#[test]
fn selected_measure_functions() {
    let fx = Fx::new(10);
    let meta = |item| feb_2024(&[("Meta", item)]);
    assert_eq!(fx.measure("Sales", &meta("Name")), Value::string("Sales"));
    assert_eq!(fx.measure("Units", &meta("Name")), Value::string("Units"));
    assert_eq!(fx.measure("Sales", &meta("Only Sales")), i(80));
    assert_eq!(fx.measure("Units", &meta("Only Sales")), Value::Blank);
    assert_eq!(fx.measure("Big", &meta("Fmt")), Value::string("#,0"));
    // FORMAT with the measure's format string: 80 × 100 → "8,000".
    assert_eq!(fx.measure("Big", &meta("Text")), Value::string("8,000"));
    // Two Meta items selected: its multipleOrEmptySelectionExpression.
    let mut two = feb_2024(&[]);
    two.add(ColumnFilter::in_values(
        "Meta",
        "Name",
        [Value::string("Name"), Value::string("Fmt")],
    ));
    assert_eq!(fx.measure("Sales", &two), Value::string("multi"));
    // Time Calc (precedence 20) applies outside Meta (5): the measure name
    // is still the measure's.
    let both = feb_2024(&[("Time Calc", "YTD"), ("Meta", "Name")]);
    assert_eq!(fx.measure("Sales", &both), Value::string("Sales"));
    // Outside a calculation item SELECTEDMEASURE is an error.
    let err =
        evaluate_measure(&fx.engine.snapshot(), &fx.model, "Bad", &feb_2024(&[])).unwrap_err();
    assert!(
        matches!(&err, DaxError::Evaluation { message } if message.contains("calculation item")),
        "{err}"
    );
}

#[test]
fn dynamic_format_strings_follow_precedence() {
    let fx = Fx::new(10);
    // The item's format string wins over the measure's.
    let (v, f, text) = fx.formatted("Sales", &feb_2024(&[("Time Calc", "YOY %")]));
    assert_eq!(v, Value::Double(0.6));
    assert_eq!(f.as_deref(), Some("0.0%"));
    assert_eq!(text, "60.0%");
    // No item format string: the measure's static one.
    let (_, f, text) = fx.formatted("Big", &feb_2024(&[("Time Calc", "YTD")]));
    assert_eq!((f.as_deref(), text.as_str()), (Some("#,0"), "28,000"));
    // A measure's own dynamic format string sees the displayed value:
    // 80 → "0"; with YTD, 280 > 100 → "#,0.00".
    let (_, f, text) = fx.formatted("Dyn", &feb_2024(&[]));
    assert_eq!((f.as_deref(), text.as_str()), (Some("0"), "80"));
    let (_, f, text) = fx.formatted("Dyn", &feb_2024(&[("Time Calc", "YTD")]));
    assert_eq!((f.as_deref(), text.as_str()), (Some("#,0.00"), "280.00"));
    // No format string at all.
    let (_, f, _) = fx.formatted("Sales x2", &feb_2024(&[]));
    assert_eq!(f, None);
}

#[test]
fn summarizecolumns_applies_items_per_row_and_falls_back_from_fast_paths() {
    let fx = Fx::new(10);
    // Visual-query shape: one row per calculation item.
    let rows = fx.query(
        "EVALUATE SUMMARIZECOLUMNS('Time Calc'[Name], \"v0\", Sales[Sales])",
        &feb_2024(&[]),
    );
    let by_item: BTreeMap<String, Value> = rows
        .into_iter()
        .map(|r| (r[0].to_string(), r[1].clone()))
        .collect();
    let expected: BTreeMap<String, Value> = [
        ("Current", i(80)),
        ("PY", i(50)),
        ("PY YTD", i(150)),
        ("YOY %", Value::Double(0.6)),
        ("YTD", i(280)),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    assert_eq!(by_item, expected);
    // Grand total (no item selected): the plain measure.
    assert_eq!(
        fx.query("EVALUATE ROW(\"v0\", Sales[Sales])", &feb_2024(&[]))[0][0],
        i(80)
    );
    // A slicer on YTD and months on rows: the one-pass grouped aggregation
    // would give monthly sums; with the item applied, YTD runs on: Jan 200,
    // Feb 280, then 320 for March to December.
    let ytd_2024 = FilterContext::new()
        .with(ColumnFilter::equals("Date", "Year", Value::Integer(2024)))
        .with(ColumnFilter::equals("Time Calc", "Name", "YTD"));
    let rows = fx.query(
        "EVALUATE SUMMARIZECOLUMNS('Date'[Month], \"v0\", Sales[Sales])",
        &ytd_2024,
    );
    let mut expected = vec![vec![i(1), i(200)], vec![i(2), i(280)]];
    expected.extend((3..=12).map(|m| vec![i(m), i(320)]));
    assert_eq!(rows, expected);
    // Without a selection the fast path's monthly sums are right.
    let year =
        FilterContext::new().with(ColumnFilter::equals("Date", "Year", Value::Integer(2024)));
    assert_eq!(
        fx.query(
            "EVALUATE SUMMARIZECOLUMNS('Date'[Month], \"v0\", Sales[Sales])",
            &year
        ),
        vec![vec![i(1), i(200)], vec![i(2), i(80)], vec![i(3), i(40)]]
    );
}

#[test]
fn selection_expressions_apply_to_empty_multiple_and_no_selection() {
    let mut model = model(10);
    let scale = calc_table("Scale", 50, &[("Half", "SELECTEDMEASURE() / 2", None)]);
    model.tables.push(scale);
    if let Some(g) = &mut model.tables[5].calculation_group {
        g.no_selection_expression = Some(CalculationGroupExpression::new("SELECTEDMEASURE() * 10"));
        g.multiple_or_empty_selection_expression =
            Some(CalculationGroupExpression::new("-SELECTEDMEASURE()"));
    }
    let mut engine = engine(&model);
    load_calc_table(&mut engine, &model, "Scale");
    let fx = Fx { engine, model };
    // Not filtered: noSelectionExpression (80 × 10).
    assert_eq!(fx.measure("Sales", &feb_2024(&[])), i(800));
    // One item: the item (80 / 2).
    assert_eq!(
        fx.measure("Sales", &feb_2024(&[("Scale", "Half")])),
        Value::Double(40.0)
    );
    // An item that does not exist: multipleOrEmptySelectionExpression.
    assert_eq!(fx.measure("Sales", &feb_2024(&[("Scale", "Nope")])), i(-80));
    // The no-selection expression wraps the other groups' items too
    // (precedence 50 > 20): (YTD 280) × 10.
    assert_eq!(
        fx.measure("Sales", &feb_2024(&[("Time Calc", "YTD")])),
        i(2800)
    );
    // `selectionExpressionBehavior: visual`: a multiple selection without a
    // selection expression returns BLANK.
    let mut visual = fx.model.clone();
    visual.extra.insert(
        "selectionExpressionBehavior".into(),
        PropValue::Text("visual".into()),
    );
    let mut both = feb_2024(&[]);
    both.add(ColumnFilter::in_values(
        "Time Calc",
        "Name",
        [Value::string("YTD"), Value::string("PY")],
    ));
    let v = evaluate_measure(&fx.engine.snapshot(), &visual, "Units", &both).unwrap();
    assert_eq!(v, Value::Blank);
}

// ---- many-to-many relationships ------------------------------------------------

/// `Sales[Customer] (*) – (*) Targets[Customer]` (see
/// `pbi-engine/tests/m2m_security.rs` for the data):
///
/// ```text
/// Sales: C1 10, C1 20, C2 5, C3 7, C9 100
/// Targets: C1 East 50, C1 West 60, C2 East 30, C4 West 40
/// ```
fn m2m(cross_filter: CrossFilter) -> (Engine, Model) {
    let mut rel = Relationship::new(
        "m2m",
        ColumnRef::new("Sales", "Customer"),
        ColumnRef::new("Targets", "Customer"),
    );
    rel.from_cardinality = Cardinality::Many;
    rel.to_cardinality = Cardinality::Many;
    rel.cross_filtering_behavior = cross_filter;
    let mut sales = Table::new("Sales");
    sales.columns = vec![
        Column::data("Customer", DataType::String),
        Column::data("Amount", DataType::Int64),
    ];
    sales.measures = vec![Measure::new("Amount", "SUM(Sales[Amount])")];
    let mut targets = Table::new("Targets");
    targets.columns = vec![
        Column::data("Customer", DataType::String),
        Column::data("Region", DataType::String),
        Column::data("Target", DataType::Int64),
    ];
    targets.measures = vec![Measure::new("Target", "SUM(Targets[Target])")];
    let model = Model {
        tables: vec![sales, targets],
        relationships: vec![rel],
        ..Model::default()
    };
    let mut engine = Engine::new(&model);
    engine
        .load_table(
            "Sales",
            batch(vec![
                ("Customer", texts(&["C1", "C1", "C2", "C3", "C9"])),
                (
                    "Amount",
                    ints(&[Some(10), Some(20), Some(5), Some(7), Some(100)]),
                ),
            ]),
        )
        .unwrap();
    engine
        .load_table(
            "Targets",
            batch(vec![
                ("Customer", texts(&["C1", "C1", "C2", "C4"])),
                ("Region", texts(&["East", "West", "East", "West"])),
                ("Target", ints(&[Some(50), Some(60), Some(30), Some(40)])),
            ]),
        )
        .unwrap();
    (engine, model)
}

#[test]
fn many_to_many_measures_by_the_other_side_with_totals() {
    let (engine, model) = m2m(CrossFilter::OneDirection);
    let ctx = EvalContext::new(engine.snapshot(), Arc::new(model.clone()));
    let rows = ctx
        .evaluate_query(
            "EVALUATE SUMMARIZECOLUMNS(Targets[Region], \"Amount\", [Amount], \"Target\", [Target])",
        )
        .unwrap()[0]
        .rows();
    // East: customers C1, C2 → 10 + 20 + 5; West: C1, C4 → 30. Targets per
    // region are local sums. No blank row for the sales without a target.
    assert_eq!(
        rows,
        vec![
            vec![Value::string("East"), i(35), i(80)],
            vec![Value::string("West"), i(30), i(100)],
        ]
    );
    // The total is not the sum of the rows: C1 counts once and the sales
    // without a target (C3 7, C9 100) are included.
    let total = evaluate_measure(&engine.snapshot(), &model, "Amount", &FilterContext::new());
    assert_eq!(total.unwrap(), i(142));
    // Single direction: Sales does not filter Targets.
    let rows = ctx
        .evaluate_query("EVALUATE SUMMARIZECOLUMNS(Sales[Customer], \"Target\", [Target])")
        .unwrap()[0]
        .rows();
    assert!(rows.iter().all(|r| r[1] == i(180)), "{rows:?}");
}

#[test]
fn many_to_many_both_directions_and_related_functions() {
    let (engine, model) = m2m(CrossFilter::BothDirections);
    let snapshot = engine.snapshot();
    let f = FilterContext::new();
    let ctx = EvalContext::new(engine.snapshot(), Arc::new(model.clone()));
    // Targets by sale customer: C1 → 50 + 60; C2 → 30; C3 and C9 have no
    // target (dropped as BLANK).
    let rows = ctx
        .evaluate_query("EVALUATE SUMMARIZECOLUMNS(Sales[Customer], \"Target\", [Target])")
        .unwrap()[0]
        .rows();
    assert_eq!(
        rows,
        vec![
            vec![Value::string("C1"), i(110)],
            vec![Value::string("C2"), i(30)]
        ]
    );
    // RELATEDTABLE follows the relationship through filter propagation: the
    // sales of the first target's customer (C1: two rows).
    assert_eq!(
        evaluate_scalar(
            &snapshot,
            &model,
            "MAXX(FILTER(Targets, Targets[Region] = \"East\" && Targets[Customer] = \"C1\"), COUNTROWS(RELATEDTABLE(Sales)))",
            &f
        )
        .unwrap(),
        i(2)
    );
    // RELATED cannot follow a many-to-many relationship (as in Power BI).
    let err = evaluate_scalar(
        &snapshot,
        &model,
        "MAXX(Sales, RELATED(Targets[Region]))",
        &f,
    )
    .unwrap_err();
    assert!(err.to_string().contains("many-to-many"), "{err}");
}

// ---- effective user ------------------------------------------------------------

#[test]
fn identity_functions_return_the_effective_user() {
    let (mut engine, model) = m2m(CrossFilter::OneDirection);
    let f = FilterContext::new();
    let eval =
        |engine: &Engine, src: &str| evaluate_scalar(&engine.snapshot(), &model, src, &f).unwrap();
    assert_eq!(eval(&engine, "USERPRINCIPALNAME()"), Value::string(""));
    assert_eq!(eval(&engine, "CUSTOMDATA()"), Value::Blank);
    engine.set_security_context(Some(SecurityContext {
        roles: Vec::new(),
        identity: UserIdentity {
            user_name: Some("ann@contoso.com".into()),
            user_object_id: Some("0f1e".into()),
            custom_data: Some("tenant=7".into()),
        },
    }));
    assert_eq!(
        eval(&engine, "USERPRINCIPALNAME() & \"|\" & USERNAME()"),
        Value::string("ann@contoso.com|ann@contoso.com")
    );
    assert_eq!(eval(&engine, "USEROBJECTID()"), Value::string("0f1e"));
    assert_eq!(eval(&engine, "CUSTOMDATA()"), Value::string("tenant=7"));
}
