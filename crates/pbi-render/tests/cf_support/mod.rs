//! A small sales model for conditional-formatting and format-string tests
//! (no fixture files), shared with `examples/render_cond_format.rs`.
//!
//! ```text
//! Sales: Region  Product  Year  Amount  Cost
//!        North   A        2025  100     80
//!        North   A        2026  150     90
//!        North   B        2026  50      70
//!        South   A        2025  200     150
//!        South   B        2026  300     130
//!        East    A        2026  120     100
//!        West    B        2025  40      60
//! ```
//!
//! Per region: Total Sales East 120, North 300, South 500, West 40 (all
//! 960); Sales 2026 120, 200, 300, BLANK; Margin 20, 60, 220, -20 (all
//! 280); YOY % BLANK, 100%, 50%, -100% (all 82.4%). `'Time Calc'` is a
//! calculation group: `Current` and `YOY %` (format string `0.0%`).

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use arrow::array::{ArrayRef, Int64Array, StringArray};
use arrow::record_batch::RecordBatch;
use pbi_engine::Engine;
use pbi_model::{
    CalculationGroup, CalculationItem, Column, ColumnKind, Condition, DataType, FieldExpr, Filter,
    FilterKind, Literal, Measure, Model, Page, Position, Projection, PropValue, Report, Table,
    Visual,
};
use pbi_render::query::QueryEnv;
use serde_json::{Value as Json, json};

pub fn model() -> Model {
    let mut sales = Table::new("Sales");
    for (name, dt) in [
        ("Region", DataType::String),
        ("Product", DataType::String),
        ("Year", DataType::Int64),
        ("Amount", DataType::Int64),
        ("Cost", DataType::Int64),
    ] {
        sales.columns.push(Column::data(name, dt));
    }
    let mut add = |name: &str, expr: &str, format: Option<&str>| {
        let mut m = Measure::new(name, expr);
        m.format_string = format.map(str::to_owned);
        sales.measures.push(m);
    };
    add("Total Sales", "SUM(Sales[Amount])", Some("\\$#,0"));
    add(
        "Margin",
        "SUM(Sales[Amount]) - SUM(Sales[Cost])",
        Some("#,0"),
    );
    add(
        "Sales 2026",
        "CALCULATE([Total Sales], Sales[Year] = 2026)",
        Some("\\$#,0"),
    );
    add(
        "Status Color",
        "IF([Margin] < 0, \"Red\", \"#00AA00\")",
        None,
    );
    add(
        "Status Icon",
        "IF([Margin] < 0, \"arrow_down\", \"arrow_up\")",
        None,
    );
    add("Broken", "NOSUCHFUNCTION(1)", None);
    let mut scaled = Measure::new("Scaled", "[Total Sales]");
    scaled.extra.insert(
        "formatStringDefinition".into(),
        PropValue::Expression("IF(SELECTEDMEASURE() >= 200, \"#,0.0\", \"0\")".into()),
    );
    sales.measures.push(scaled);

    let mut calc = Table::new("Time Calc");
    let mut name = Column::data("Name", DataType::String);
    name.kind = ColumnKind::Data {
        source_column: "Name".into(),
    };
    calc.columns = vec![name, Column::data("Ordinal", DataType::Int64)];
    calc.calculation_group = Some(Box::new(CalculationGroup {
        calculation_items: vec![
            CalculationItem::new("Current", "SELECTEDMEASURE()"),
            CalculationItem {
                format_string_definition: Some("\"0.0%\"".into()),
                ..CalculationItem::new(
                    "YOY %",
                    "VAR cy = CALCULATE(SELECTEDMEASURE(), Sales[Year] = 2026) \
                     VAR py = CALCULATE(SELECTEDMEASURE(), Sales[Year] = 2025) \
                     RETURN DIVIDE(cy - py, py)",
                )
            },
        ],
        ..CalculationGroup::default()
    }));
    Model {
        tables: vec![sales, calc],
        ..Model::default()
    }
}

fn strings(values: &[&str]) -> ArrayRef {
    Arc::new(StringArray::from(values.to_vec()))
}

fn ints(values: &[i64]) -> ArrayRef {
    Arc::new(Int64Array::from(values.to_vec()))
}

pub fn env() -> QueryEnv {
    let model = model();
    let mut engine = Engine::new(&model);
    let sales = RecordBatch::try_from_iter([
        (
            "Region",
            strings(&["North", "North", "North", "South", "South", "East", "West"]),
        ),
        ("Product", strings(&["A", "A", "B", "A", "B", "A", "B"])),
        ("Year", ints(&[2025, 2026, 2026, 2025, 2026, 2026, 2025])),
        ("Amount", ints(&[100, 150, 50, 200, 300, 120, 40])),
        ("Cost", ints(&[80, 90, 70, 150, 130, 100, 60])),
    ])
    .unwrap();
    let calc = RecordBatch::try_from_iter([
        ("Name", strings(&["Current", "YOY %"])),
        ("Ordinal", ints(&[0, 1])),
    ])
    .unwrap();
    engine
        .load_tables([("Sales".to_owned(), sales), ("Time Calc".to_owned(), calc)])
        .unwrap();
    QueryEnv::new(engine.snapshot(), Arc::new(model), "en-US")
}

pub fn col(name: &str) -> FieldExpr {
    FieldExpr::column("Sales", name)
}

pub fn item() -> FieldExpr {
    FieldExpr::column("Time Calc", "Name")
}

pub fn measure(name: &str) -> FieldExpr {
    FieldExpr::measure("Sales", name)
}

pub fn visual(
    name: &str,
    visual_type: &str,
    (x, y, width, height): (f64, f64, f64, f64),
    fields: Vec<(&str, FieldExpr)>,
    objects: Json,
) -> Visual {
    Visual {
        name: name.into(),
        visual_type: visual_type.into(),
        position: Position {
            x,
            y,
            width,
            height,
            ..Position::default()
        },
        projections: fields
            .into_iter()
            .map(|(role, field)| Projection {
                role: role.into(),
                query_ref: field.query_ref().unwrap_or_default(),
                field,
                ..Projection::default()
            })
            .collect(),
        objects,
        ..Visual::default()
    }
}

/// A visual filter selecting calculation item `name`.
pub fn item_filter(name: &str) -> Filter {
    Filter {
        name: "item".into(),
        field: Some(item()),
        kind: FilterKind::Categorical,
        condition: Some(Condition::In {
            exprs: vec![item()],
            values: vec![vec![Literal::text(name)]],
        }),
        ..Filter::default()
    }
}

pub fn page(name: &str, visuals: Vec<Visual>) -> Page {
    Page {
        name: name.into(),
        display_name: name.into(),
        width: 1280.0,
        height: 720.0,
        visuals,
        ..Page::default()
    }
}

pub fn report(page: &Page) -> Report {
    Report {
        name: "cf".into(),
        pages: vec![page.clone()],
        ..Report::default()
    }
}

// ---- PBIR expressions ------------------------------------------------------

pub fn lit(raw: &str) -> Json {
    json!({"Literal": {"Value": raw}})
}

pub fn prop(raw: &str) -> Json {
    json!({"expr": lit(raw)})
}

pub fn color(hex: &str) -> Json {
    json!({"solid": {"color": {"expr": lit(&format!("'{hex}'"))}}})
}

pub fn measure_expr(name: &str) -> Json {
    json!({"Measure": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": name}})
}

/// A conditional color (`{"solid": {"color": {"expr": …}}}`).
pub fn cf(expr: Json) -> Json {
    json!({"solid": {"color": {"expr": expr}}})
}

/// `linearGradient2` over `input`, data-driven bounds.
pub fn gradient2(input: Json, min: &str, max: &str) -> Json {
    json!({"FillRule": {"Input": input, "FillRule": {"linearGradient2": {
        "min": {"color": lit(&format!("'{min}'"))},
        "max": {"color": lit(&format!("'{max}'"))},
        "nullColoringStrategy": {"strategy": lit("'asZero'")}
    }}}})
}

/// `linearGradient3` over `input`, data-driven bounds.
pub fn gradient3(input: Json, min: &str, mid: &str, max: &str) -> Json {
    json!({"FillRule": {"Input": input, "FillRule": {"linearGradient3": {
        "min": {"color": lit(&format!("'{min}'"))},
        "mid": {"color": lit(&format!("'{mid}'"))},
        "max": {"color": lit(&format!("'{max}'"))},
        "nullColoringStrategy": {"strategy": lit("'noColor'")}
    }}}})
}

/// `input <kind> value` (`ComparisonKind` 0 `=`, 1 `>`, 2 `>=`, 3 `<`, 4 `<=`).
pub fn cmp(input: &Json, kind: u8, value: &str) -> Json {
    json!({"Comparison": {"ComparisonKind": kind, "Left": input, "Right": lit(value)}})
}

pub fn and(a: Json, b: Json) -> Json {
    json!({"And": {"Left": a, "Right": b}})
}

/// Rules: `(condition, value literal)` cases.
pub fn rules(cases: Vec<(Json, &str)>) -> Json {
    let cases: Vec<Json> = cases
        .into_iter()
        .map(|(c, v)| json!({"Condition": c, "Value": lit(v)}))
        .collect();
    json!({"Conditional": {"Cases": cases}})
}

/// A `values` entry for column `query_ref`; `matching`: 0 values and
/// totals, 1 values, 2 totals.
pub fn values_entry(query_ref: &str, matching: u8, properties: Json) -> Json {
    json!({
        "properties": properties,
        "selector": {"data": [{"dataViewWildcard": {"matchingOption": matching}}], "metadata": query_ref}
    })
}

/// A `dataPoint` entry with a conditional fill for every value.
pub fn data_point_fill(expr: Json) -> Json {
    json!({"dataPoint": [{
        "properties": {"fill": cf(expr)},
        "selector": {"data": [{"dataViewWildcard": {"matchingOption": 1}}]}
    }]})
}

/// The showcase table: Region, Total Sales, Margin with a gradient
/// background (values and totals), field-value font colors and icons
/// (hidden measures) and data bars (negatives included).
pub fn showcase_table(name: &str, rect: (f64, f64, f64, f64)) -> Visual {
    let sales = measure_expr("Total Sales");
    visual(
        name,
        "tableEx",
        rect,
        vec![
            ("Values", col("Region")),
            ("Values", measure("Total Sales")),
            ("Values", measure("Margin")),
        ],
        json!({"values": [
            values_entry("Sales.Region", 1, json!({"fontColor": cf(measure_expr("Status Color"))})),
            values_entry("Sales.Total Sales", 0, json!({
                "backColor": cf(gradient2(sales, "#FFFFFF", "#118DFF")),
                "icon": {"kind": "Icon", "layout": lit("'Before'"), "verticalAlignment": lit("'Middle'"),
                         "value": {"expr": measure_expr("Status Icon")}}
            })),
            values_entry("Sales.Margin", 1, json!({"dataBars": {
                "positiveColor": color("#5FA35F"),
                "negativeColor": color("#D64554"),
                "axisColor": color("#252423")
            }}))
        ]}),
    )
}

/// The showcase matrix: Region × `'Time Calc'` items of Total Sales, with
/// rule colors (BLANK gray, negative red, below 1 yellow, else green).
pub fn showcase_matrix(name: &str, rect: (f64, f64, f64, f64)) -> Visual {
    let sales = measure_expr("Total Sales");
    let rule = rules(vec![
        (cmp(&sales, 0, "null"), "'#C8C6C4'"),
        (cmp(&sales, 3, "0D"), "'#F4B6BB'"),
        (and(cmp(&sales, 2, "0D"), cmp(&sales, 3, "1D")), "'#FCE6A4'"),
        (cmp(&sales, 2, "1D"), "'#B7E1B4'"),
    ]);
    visual(
        name,
        "pivotTable",
        rect,
        vec![
            ("Rows", col("Region")),
            ("Columns", item()),
            ("Values", measure("Total Sales")),
        ],
        json!({"values": [values_entry("Sales.Total Sales", 1, json!({"backColor": cf(rule)}))]}),
    )
}
