#![allow(clippy::unwrap_used, clippy::expect_used)]

use pbi_engine::Value;
use pbi_model::{AggregationFunction, FieldExpr, Projection, Visual};
use serde_json::{Value as Json, json};

use super::*;

pub(crate) fn lit(raw: &str) -> Json {
    json!({"Literal": {"Value": raw}})
}

pub(crate) fn sum(table: &str, column: &str) -> Json {
    json!({"Aggregation": {
        "Expression": {"Column": {"Expression": {"SourceRef": {"Entity": table}}, "Property": column}},
        "Function": 0
    }})
}

pub(crate) fn measure(table: &str, name: &str) -> Json {
    json!({"Measure": {"Expression": {"SourceRef": {"Entity": table}}, "Property": name}})
}

fn color_expr(expr: Json) -> Json {
    json!({"solid": {"color": {"expr": expr}}})
}

fn visual(visual_type: &str, objects: Json) -> Visual {
    Visual {
        name: "v".into(),
        visual_type: visual_type.into(),
        objects,
        ..Visual::default()
    }
}

/// A column of cells: `values[input][cell]`, with total flags.
struct Cells {
    values: Vec<Vec<Value>>,
    totals: Vec<bool>,
}

impl CellSource for Cells {
    fn cell_count(&self) -> usize {
        self.totals.len()
    }
    fn is_total(&self, cell: usize) -> bool {
        self.totals[cell]
    }
    fn bind(&self, input: &Input) -> Option<usize> {
        match input {
            Input::Own => Some(0),
            Input::Field(f) if f.property() == Some("Sales") => Some(0),
            Input::Field(f) if f.property() == Some("Color") => Some(1),
            Input::Field(_) => None,
        }
    }
    fn value(&self, column: usize, cell: usize) -> Value {
        self.values[column][cell].clone()
    }
}

fn cells(sales: &[Option<f64>], colors: &[&str]) -> Cells {
    let mut totals = vec![false; sales.len()];
    if let Some(last) = totals.last_mut() {
        *last = true;
    }
    Cells {
        values: vec![
            sales
                .iter()
                .map(|v| v.map_or(Value::Blank, Value::Double))
                .collect(),
            colors.iter().map(|c| Value::from(*c)).collect(),
        ],
        totals,
    }
}

fn hex(p: Option<Paint>) -> Option<String> {
    p.map(|p| p.color.to_hex())
}

#[test]
fn gradients_scale_over_the_data_and_handle_blanks() {
    let gradient = json!({"FillRule": {
        "Input": sum("T", "Sales"),
        "FillRule": {"linearGradient2": {
            "min": {"color": lit("'#FFFFFF'")},
            "max": {"color": lit("'#000000'")},
            "nullColoringStrategy": {"strategy": lit("'specificColor'"), "color": lit("'#FF0000'")}
        }}
    }});
    let v = visual(
        "tableEx",
        json!({"values": [{
            "properties": {"backColor": color_expr(gradient)},
            "selector": {"data": [{"dataViewWildcard": {"matchingOption": 0}}], "metadata": "Sum(T.Sales)"}
        }]}),
    );
    let spec = CfSpec::parse(&v);
    assert!(spec.diagnostics.is_empty(), "{:?}", spec.diagnostics);
    assert_eq!(spec.rules.len(), 1);
    let rule = &spec.rules[0];
    assert_eq!(rule.target.as_deref(), Some("Sum(T.Sales)"));
    assert_eq!(rule.apply, ApplyTo::Both);
    assert_eq!(
        spec.field_inputs(),
        vec![&FieldExpr::aggregation(
            FieldExpr::column("T", "Sales"),
            AggregationFunction::Sum
        )]
    );
    // 0, 50, 100, BLANK and a total of 400 (outside the data range).
    let src = cells(
        &[Some(0.0), Some(50.0), Some(100.0), None, Some(400.0)],
        &["", "", "", "", ""],
    );
    let theme = ThemeResolver::default();
    let styles = evaluate_cells(&[rule], &src, &theme);
    let bg: Vec<Option<String>> = styles.iter().map(|s| hex(s.background)).collect();
    assert_eq!(
        bg,
        [
            Some("#FFFFFF".into()),
            Some("#808080".into()),
            Some("#000000".into()),
            Some("#FF0000".into()),
            Some("#000000".into()),
        ]
    );

    // Three colors with explicit bounds; BLANK as zero.
    let gradient3 = json!({"FillRule": {
        "Input": sum("T", "Sales"),
        "FillRule": {"linearGradient3": {
            "min": {"color": lit("'#FF0000'"), "value": lit("-100D")},
            "mid": {"color": lit("'#FFFFFF'"), "value": lit("0D")},
            "max": {"color": lit("'#0000FF'"), "value": lit("100D")}
        }}
    }});
    let v = visual(
        "pivotTable",
        json!({"values": [{"properties": {"fontColor": color_expr(gradient3)}, "selector": {"metadata": "Sum(T.Sales)"}}]}),
    );
    let spec = CfSpec::parse(&v);
    let src = cells(&[Some(-50.0), None, Some(200.0), Some(1.0)], &[""; 4]);
    let font: Vec<Option<String>> = evaluate_cells(&[&spec.rules[0]], &src, &theme)
        .iter()
        .map(|s| s.font.map(Color::to_hex))
        .collect();
    // The total (last cell) is not formatted: values only by default.
    assert_eq!(
        font,
        [
            Some("#FF8080".into()),
            Some("#FFFFFF".into()),
            Some("#0000FF".into()),
            None
        ]
    );
}

#[test]
fn rules_with_ranges_blanks_and_percent_of_range() {
    let s = sum("T", "Sales");
    let cmp = |kind: u8, right: Json| json!({"Comparison": {"ComparisonKind": kind, "Left": s, "Right": right}});
    // Percent of the range: Min + (Max - Min) * 0.5.
    let half = json!({"Arithmetic": {
        "Left": {"ScopedEval": {"Expression": {"Min": {"Expression": s}}, "Scope": []}},
        "Right": {"Arithmetic": {
            "Left": {"Arithmetic": {
                "Left": {"ScopedEval": {"Expression": {"Max": {"Expression": s}}, "Scope": []}},
                "Right": {"ScopedEval": {"Expression": {"Min": {"Expression": s}}, "Scope": []}},
                "Operator": 1
            }},
            "Right": lit("0.5D"),
            "Operator": 2
        }},
        "Operator": 0
    }});
    let rules = json!({"Conditional": {
        "Cases": [
            {"Condition": cmp(0, lit("null")), "Value": lit("'#999999'")},
            {"Condition": {"And": {"Left": cmp(2, lit("0D")), "Right": cmp(3, half.clone())}}, "Value": lit("'#FF0000'")},
            {"Condition": cmp(2, half), "Value": lit("'#00FF00'")}
        ]
    }});
    let v = visual(
        "tableEx",
        json!({"columnFormatting": [{"properties": {"backColor": color_expr(rules)}, "selector": {"metadata": "Sum(T.Sales)"}}]}),
    );
    let spec = CfSpec::parse(&v);
    assert!(spec.diagnostics.is_empty(), "{:?}", spec.diagnostics);
    let src = cells(
        &[
            Some(10.0),
            Some(49.0),
            Some(60.0),
            None,
            Some(-5.0),
            Some(0.0),
        ],
        &[""; 6],
    );
    let bg: Vec<Option<String>> =
        evaluate_cells(&[&spec.rules[0]], &src, &ThemeResolver::default())
            .iter()
            .map(|s| hex(s.background))
            .collect();
    // Range -5..60 (the total is not data): half = 27.5; -5 matches nothing.
    assert_eq!(
        bg,
        [
            Some("#FF0000".into()),
            Some("#00FF00".into()),
            Some("#00FF00".into()),
            Some("#999999".into()),
            None,
            None
        ]
    );
}

#[test]
fn field_values_data_bars_and_icons() {
    let theme = ThemeResolver::default();
    let v = visual(
        "tableEx",
        json!({"values": [
            {"properties": {
                "fontColor": color_expr(json!({"Aggregation": {
                    "Expression": {"Column": {"Expression": {"SourceRef": {"Entity": "T"}}, "Property": "Color"}},
                    "Function": 3
                }})),
                "dataBars": {
                    "positiveColor": {"solid": {"color": lit("'#00AA00'")}},
                    "negativeColor": {"solid": {"color": lit("'#AA0000'")}},
                    "axisColor": {"solid": {"color": lit("'#000000'")}},
                    "hideText": lit("true")
                },
                "icon": {
                    "kind": "Icon",
                    "layout": lit("'After'"),
                    "verticalAlignment": lit("'Middle'"),
                    "value": {"expr": {"Conditional": {"Cases": [
                        {"Condition": {"Comparison": {"ComparisonKind": 3, "Left": sum("T", "Sales"), "Right": lit("0D")}}, "Value": lit("'arrow_down'")},
                        {"Condition": {"Comparison": {"ComparisonKind": 2, "Left": sum("T", "Sales"), "Right": lit("0D")}}, "Value": lit("'arrow_up'")}
                    ]}}}
                }
            },
            "selector": {"metadata": "Sum(T.Sales)"}}
        ]}),
    );
    let spec = CfSpec::parse(&v);
    assert!(spec.diagnostics.is_empty(), "{:?}", spec.diagnostics);
    assert_eq!(spec.rules.len(), 3);
    let rules: Vec<&CfRule> = spec.rules.iter().collect();
    let src = cells(
        &[Some(-50.0), Some(150.0), None, Some(100.0)],
        &["Red", "#00ff00", "nonsense", "Blue"],
    );
    let styles = evaluate_cells(&rules, &src, &theme);
    assert_eq!(styles[0].font, Some(Color::rgb(255, 0, 0)));
    assert_eq!(styles[1].font, Some(Color::rgb(0, 255, 0)));
    assert_eq!(styles[2].font, None);
    // Range -50..150: the axis sits at 25%.
    let bar = styles[0].bar.unwrap();
    assert_eq!((bar.from, bar.to), (0.0, 0.25));
    assert_eq!(bar.paint.color, Color::rgb(0xAA, 0, 0));
    assert_eq!(bar.axis, Some((0.25, Color::BLACK)));
    let bar = styles[1].bar.unwrap();
    assert_eq!((bar.from, bar.to), (0.25, 1.0));
    assert!(styles[0].hide_text && styles[1].hide_text);
    assert_eq!(styles[2].bar, None);
    // No bars in totals.
    assert_eq!(styles[3].bar, None);
    let icon = styles[0].icon.unwrap();
    assert_eq!(icon.0.shape, IconShape::ArrowDown);
    assert_eq!((icon.1, icon.2), (IconLayout::After, IconAlign::Middle));
    assert_eq!(styles[1].icon.unwrap().0.shape, IconShape::ArrowUp);
    assert!(styles[2].icon.is_none());
}

#[test]
fn reversed_bars_and_positive_ranges() {
    let bars = DataBars {
        reverse: true,
        ..DataBars::default()
    };
    let rule = CfRule {
        target: Some("x".into()),
        apply: ApplyTo::Values,
        inputs: vec![Input::Own],
        action: Action::DataBars(bars),
        origin: "values.dataBars".into(),
    };
    let src = cells(&[Some(25.0), Some(100.0), Some(0.0)], &[""; 3]);
    let styles = evaluate_cells(&[&rule], &src, &ThemeResolver::default());
    // All positive: 0..100, bars grow from the right.
    let b = styles[0].bar.unwrap();
    assert_eq!((b.from, b.to, b.axis), (0.75, 1.0, None));
}

#[test]
fn unsupported_variants_are_diagnosed() {
    let v = visual(
        "tableEx",
        json!({
            "values": [
                {"properties": {"backColor": color_expr(json!({"FillRule": {"Input": sum("T", "Sales"), "FillRule": {"linearGradient9": {}}}}))},
                 "selector": {"metadata": "Sum(T.Sales)"}},
                {"properties": {"fontColor": color_expr(measure("T", "Color"))}},
                {"properties": {"backColor": color_expr(measure("T", "Color"))},
                 "selector": {"data": [{"scopeId": {"Comparison": {}}}], "metadata": "T.Region"}},
                {"properties": {"backColor": {"solid": {"color": lit("'#FF0000'")}}},
                 "selector": {"metadata": "T.Region"}}
            ],
            "columnHeaders": [{"properties": {"fontColor": color_expr(measure("T", "Color"))}}]
        }),
    );
    let spec = CfSpec::parse(&v);
    assert!(spec.rules.is_empty());
    let d = spec.diagnostics.join("\n");
    assert!(d.contains("linearGradient9"), "{d}");
    assert!(d.contains("no column selected"), "{d}");
    assert!(d.contains("single data points"), "{d}");
    assert!(d.contains("columnHeaders.fontColor"), "{d}");
    assert_eq!(spec.diagnostics.len(), 4, "{d}");

    // Cards do not take conditional colors; charts with a legend keep
    // their series colors.
    let card = visual(
        "cardVisual",
        json!({"value": [{"properties": {"fontColor": color_expr(measure("T", "Color"))}}]}),
    );
    assert_eq!(CfSpec::parse(&card).diagnostics.len(), 1);
    let mut bar = visual(
        "clusteredColumnChart",
        json!({"dataPoint": [{"properties": {"fill": color_expr(measure("T", "Color"))},
               "selector": {"data": [{"dataViewWildcard": {"matchingOption": 1}}]}}]}),
    );
    let spec = CfSpec::parse(&bar);
    assert_eq!(spec.rules.len(), 1);
    assert!(spec.rules[0].target.is_none());
    bar.projections.push(Projection {
        role: "Series".into(),
        ..Projection::default()
    });
    let spec = CfSpec::parse(&bar);
    assert!(spec.rules.is_empty());
    assert!(spec.diagnostics[0].contains("legend"));
    // Static colors are not conditional formatting.
    assert!(CfSpec::parse(&visual("tableEx", json!({"values": [{"properties": {"backColor": {"solid": {"color": lit("'#EEEEEE'")}}}}]}))).diagnostics.is_empty());
}

#[test]
fn inputs_match_visible_value_columns() {
    use crate::dataview::{ColumnMeta, FieldRef, FieldRefKind};
    let meta = |kind: FieldRefKind, prop: &str, agg: Option<AggregationFunction>, hidden: bool| {
        ColumnMeta {
            roles: Vec::new(),
            query_ref: if hidden {
                "Sum(T.Margin)".into()
            } else {
                format!("T.{prop}")
            },
            display_name: prop.into(),
            source: FieldRef {
                kind,
                entity: Some("T".into()),
                property: Some(prop.into()),
                aggregation: agg,
                hierarchy: None,
                source_name: String::new(),
            },
            is_measure: kind != FieldRefKind::Column,
            format_string: None,
            data_type: None,
            expression: None,
            sort: None,
            sort_order: None,
            hidden,
        }
    };
    let metas = vec![
        meta(FieldRefKind::Column, "Region", None, false),
        meta(FieldRefKind::Measure, "Sales", None, false),
        meta(
            FieldRefKind::Aggregation,
            "Units",
            Some(AggregationFunction::Sum),
            false,
        ),
        meta(
            FieldRefKind::Aggregation,
            "Margin",
            Some(AggregationFunction::Sum),
            true,
        ),
    ];
    let col = |c: &str| FieldExpr::column("T", c);
    let agg = |c: &str, f| FieldExpr::aggregation(col(c), f);
    assert_eq!(
        find_input(&metas, &FieldExpr::measure("t", "SALES")),
        Some(1)
    );
    assert_eq!(
        find_input(&metas, &agg("Units", AggregationFunction::Sum)),
        Some(2)
    );
    assert_eq!(
        find_input(&metas, &agg("Units", AggregationFunction::Max)),
        None
    );
    assert_eq!(
        find_input(&metas, &agg("Margin", AggregationFunction::Sum)),
        Some(3)
    );
    assert_eq!(find_input(&metas, &col("Region")), None);
}
