//! query_visual / query_page on an in-memory miniature of the
//! Acme-style store model: role mapping per visual type, slicers, interactions,
//! sorting, totals, matrices, errors and warnings (no fixture files).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod query_support;

use pbi_engine::Value;
use pbi_model::{
    AggregationFunction, ComparisonKind, Condition, FieldExpr, Filter, FilterKind, InteractionKind,
    Literal, Page, Report, SortDirection, SortItem, Visual, VisualInteraction,
};
use pbi_render::dataview::{DataView, DataViewKind};
use pbi_render::query::{
    InteractionState, PointSelection, QueryEnv, QueryLimits, SlicerSelection, VisualQueryError,
};
use pretty_assertions::assert_eq;
use query_support::{col, env, lit, measure, projection};

fn visual(name: &str, visual_type: &str, projections: Vec<pbi_model::Projection>) -> Visual {
    Visual {
        name: name.into(),
        visual_type: visual_type.into(),
        projections,
        ..Visual::default()
    }
}

fn card(name: &str, m: &str) -> Visual {
    visual(name, "cardVisual", vec![projection("Data", measure(m))])
}

/// A page with cards, two slicers, a bar chart, a pie, a table, a matrix,
/// a series chart and a textbox.
fn page() -> Page {
    let mut table = visual(
        "table",
        "tableEx",
        vec![
            projection("Values", col("Stores", "Store Name")),
            projection("Values", col("Stores", "Format")),
            projection(
                "Values",
                FieldExpr::aggregation(col("Stores", "Staff"), AggregationFunction::Sum),
            ),
        ],
    );
    table.projections[1].display_name = Some("Tier".into());
    table.sort = vec![SortItem {
        field: FieldExpr::aggregation(col("Stores", "Staff"), AggregationFunction::Sum),
        direction: SortDirection::Descending,
    }];
    let mut bar = visual(
        "bar",
        "clusteredBarChart",
        vec![
            projection("Category", col("Stores", "Store Name")),
            projection("Y", measure("Order Count")),
        ],
    );
    bar.sort = vec![SortItem {
        field: measure("Order Count"),
        direction: SortDirection::Descending,
    }];
    Page {
        name: "p1".into(),
        display_name: "Page 1".into(),
        visuals: vec![
            card("total", "Store Count"),
            card("orders", "Order Count"),
            card("sessions", "Sessions"),
            visual(
                "format",
                "slicer",
                vec![projection("Values", col("Stores", "Format"))],
            ),
            visual(
                "payment",
                "slicer",
                vec![projection("Values", col("Orders", "Payment"))],
            ),
            bar,
            visual(
                "pie",
                "pieChart",
                vec![
                    projection("Category", col("Stores", "Format")),
                    projection(
                        "Y",
                        FieldExpr::aggregation(
                            col("Stores", "Store Name"),
                            AggregationFunction::CountNonNull,
                        ),
                    ),
                ],
            ),
            table,
            visual(
                "matrix",
                "pivotTable",
                vec![
                    projection("Rows", col("Stores", "Format")),
                    projection("Rows", col("Stores", "Store Name")),
                    projection("Columns", col("Orders", "Payment")),
                    projection("Values", measure("Order Count")),
                ],
            ),
            visual(
                "stacked",
                "stackedColumnChart",
                vec![
                    projection("Category", col("Stores", "Region")),
                    projection("Series", col("Orders", "Payment")),
                    projection("Y", measure("Order Count")),
                ],
            ),
            visual("text", "textbox", vec![]),
        ],
        ..Page::default()
    }
}

fn report(page: Page) -> Report {
    Report {
        name: "r".into(),
        pages: vec![page],
        ..Report::default()
    }
}

fn query(env: &QueryEnv, report: &Report, name: &str, state: &InteractionState) -> DataView {
    try_query(env, report, name, state).unwrap_or_else(|e| panic!("{name}: {e:?}"))
}

fn try_query(
    env: &QueryEnv,
    report: &Report,
    name: &str,
    state: &InteractionState,
) -> Result<DataView, VisualQueryError> {
    let page = &report.pages[0];
    let v = page.visual(name).unwrap();
    env.query_visual(report, page, v, state, &QueryLimits::default())
}

fn single(dv: &DataView) -> String {
    assert_eq!(dv.kind, DataViewKind::Single, "{dv:?}");
    dv.single.as_ref().unwrap().formatted.clone()
}

fn categories(dv: &DataView) -> Vec<String> {
    dv.categorical.as_ref().unwrap().categories[0]
        .values
        .iter()
        .map(|c| c.formatted.clone())
        .collect()
}

fn values(dv: &DataView, i: usize) -> Vec<String> {
    dv.categorical.as_ref().unwrap().values[i]
        .values
        .iter()
        .map(|c| c.formatted.clone())
        .collect()
}

#[test]
fn cards_are_single_values_with_metadata() {
    let env = env();
    let report = report(page());
    let none = InteractionState::new();
    let dv = query(&env, &report, "total", &none);
    assert_eq!(single(&dv), "5");
    let meta = &dv.metadata[0];
    assert_eq!(meta.roles, vec!["Data"]);
    assert_eq!(meta.display_name, "Store Count");
    assert_eq!(meta.source.source_name, "Stores[Store Count]");
    assert!(meta.is_measure);
    assert_eq!(meta.format_string.as_deref(), Some("0"));
    assert_eq!(meta.expression.as_deref(), Some("COUNTROWS(Stores)"));
    assert_eq!(single(&query(&env, &report, "orders", &none)), "5");
    assert_eq!(single(&query(&env, &report, "sessions", &none)), "2");
    assert_eq!(query(&env, &report, "text", &none), DataView::empty());
}

#[test]
fn slicers_list_values_under_other_filters_and_show_their_selection() {
    let env = env();
    let report = report(page());
    let none = InteractionState::new();
    let format = query(&env, &report, "format", &none);
    assert_eq!(format.kind, DataViewKind::Categorical);
    assert_eq!(
        categories(&format),
        ["(Blank)", "Flagship", "Outlet", "Pop-up"]
    );
    assert!(format.slicer.as_ref().unwrap().all_selected);

    let mut state = InteractionState::new();
    state.select("format", SlicerSelection::values(["Flagship"]));
    // Its own selection does not filter the slicer.
    let format = query(&env, &report, "format", &state);
    assert_eq!(categories(&format).len(), 4);
    let sel = format.slicer.unwrap();
    assert_eq!(sel.selected, vec![1]);
    assert!(!sel.all_selected);
    // It filters the other slicer (through the bidirectional relationship).
    assert_eq!(
        categories(&query(&env, &report, "payment", &state)),
        ["Card", "Cash"]
    );
    state.select("payment", SlicerSelection::values(["Cash"]));
    assert_eq!(
        categories(&query(&env, &report, "format", &state)),
        ["Flagship", "Outlet"]
    );
    // Inverted selection.
    let mut state = InteractionState::new();
    state.select("format", SlicerSelection::excluding(["Flagship", "Pop-up"]));
    assert_eq!(single(&query(&env, &report, "total", &state)), "2");
}

#[test]
fn slicer_selections_filter_cards_except_unrelated_tables() {
    let env = env();
    let report = report(page());
    let mut state = InteractionState::new();
    state.select("format", SlicerSelection::values(["Flagship"]));
    assert_eq!(single(&query(&env, &report, "total", &state)), "2");
    assert_eq!(single(&query(&env, &report, "orders", &state)), "2");
    assert_eq!(single(&query(&env, &report, "sessions", &state)), "2");
    // Cross-check with the DAX engine in the same filter context.
    let page = &report.pages[0];
    let v = page.visual("total").unwrap();
    let (ctx, warnings) = env.page_filter_context(&report, page, v, &state);
    assert!(warnings.is_empty());
    assert_eq!(
        pbi_dax::evaluate_measure(&env.snapshot, &env.model, "Store Count", &ctx).unwrap(),
        Value::Integer(2)
    );
}

#[test]
fn visual_interactions_are_honoured() {
    let env = env();
    let mut page = page();
    page.visual_interactions = vec![
        VisualInteraction {
            source: "format".into(),
            target: "total".into(),
            kind: InteractionKind::NoFilter,
        },
        VisualInteraction {
            source: "format".into(),
            target: "orders".into(),
            kind: InteractionKind::HighlightFilter,
        },
    ];
    let report = report(page);
    let mut state = InteractionState::new();
    state.select("format", SlicerSelection::values(["Flagship"]));
    assert_eq!(single(&query(&env, &report, "total", &state)), "5");
    let orders = query(&env, &report, "orders", &state);
    assert_eq!(single(&orders), "2");
    assert!(
        orders
            .warnings
            .iter()
            .any(|w| w.message.contains("highlight")),
        "{:?}",
        orders.warnings
    );
}

#[test]
fn selected_points_cross_filter_other_visuals() {
    let env = env();
    let report = report(page());
    let mut state = InteractionState::new();
    state.selected_points = Some(PointSelection::point(
        "pie",
        vec![(col("Stores", "Format"), Value::from("Pop-up"))],
    ));
    assert_eq!(single(&query(&env, &report, "total", &state)), "1");
    // The source visual is not filtered by its own selection.
    let pie = query(&env, &report, "pie", &state);
    assert_eq!(categories(&pie).len(), 4);
    // Two points (OR).
    state
        .selected_points
        .as_mut()
        .unwrap()
        .points
        .push(vec![pbi_render::query::PointField {
            field: col("Stores", "Format"),
            value: Value::from("Outlet"),
        }]);
    assert_eq!(single(&query(&env, &report, "total", &state)), "2");
}

#[test]
fn bar_charts_sort_by_value_and_pies_sum_to_the_total() {
    let env = env();
    let report = report(page());
    let none = InteractionState::new();
    let bar = query(&env, &report, "bar", &none);
    assert_eq!(bar.kind, DataViewKind::Categorical);
    // Store 1 has 2 orders; the unmatched order shows as (Blank); `+ 0`
    // keeps stores without orders; ties are ordered by category.
    assert_eq!(
        categories(&bar),
        [
            "Store 1", "(Blank)", "Store 3", "Store 4", "Store 2", "Store 5"
        ]
    );
    assert_eq!(values(&bar, 0), ["2", "1", "1", "1", "0", "0"]);
    let meta = &bar.metadata[1];
    assert_eq!(meta.sort, Some(SortDirection::Descending));

    let pie = query(&env, &report, "pie", &none);
    let total: i64 = pie.categorical.as_ref().unwrap().values[0]
        .values
        .iter()
        .map(|c| c.raw.as_i64().unwrap_or(0))
        .sum();
    assert_eq!(total, 5);
    // Default sort of a pie: by value, descending.
    assert_eq!(categories(&pie)[0], "Flagship");
    assert_eq!(pie.metadata[1].display_name, "Store Name");
    assert_eq!(pie.metadata[1].query_ref, "CountNonNull(Stores.Store Name)");
}

#[test]
fn series_charts_pivot_values_per_series() {
    let env = env();
    let report = report(page());
    let dv = query(&env, &report, "stacked", &InteractionState::new());
    let cat = dv.categorical.as_ref().unwrap();
    assert_eq!(cat.series_meta_idx, Some(1));
    // Category order: by value desc, ties by category.
    assert_eq!(categories(&dv), ["East", "(Blank)", "Central", "West"]);
    let series: Vec<String> = cat
        .values
        .iter()
        .map(|v| v.series.as_ref().unwrap().formatted.clone())
        .collect();
    assert_eq!(series, ["Card", "Cash"]);
    let cats = categories(&dv);
    let idx = |c: &str| cats.iter().position(|x| x == c).unwrap();
    assert_eq!(cat.values[0].values[idx("East")].formatted, "1");
    assert_eq!(cat.values[1].values[idx("East")].formatted, "1");
    assert_eq!(cat.values[0].values[idx("West")].formatted, "1");
    // `+ 0` measures are never BLANK: every combination has a value.
    assert_eq!(cat.values[1].values[idx("West")].formatted, "0");
}

#[test]
fn tables_group_sort_and_total() {
    let env = env();
    let report = report(page());
    let dv = query(&env, &report, "table", &InteractionState::new());
    assert_eq!(dv.kind, DataViewKind::Table);
    let t = dv.table.as_ref().unwrap();
    assert_eq!(t.columns, vec![0, 1, 2]);
    let names: Vec<&str> = t.rows.iter().map(|r| r[0].formatted.as_str()).collect();
    // Store 5 has a BLANK staff sum: the row is dropped.
    assert_eq!(names, ["Store 4", "Store 3", "Store 2", "Store 1"]);
    assert_eq!(t.rows[0][2].formatted, "40");
    let totals = t.totals.as_ref().unwrap();
    assert_eq!(totals[2].formatted, "100");
    assert!(totals[0].is_blank());
    assert_eq!(dv.metadata[1].display_name, "Tier");
    assert_eq!(dv.metadata[1].source.property.as_deref(), Some("Format"));
    assert_eq!(
        dv.metadata[2].expression.as_deref(),
        Some("SUM(Stores[Staff])")
    );
}

#[test]
fn row_limits_truncate() {
    let env = env();
    let report = report(page());
    let page = &report.pages[0];
    let limits = QueryLimits {
        max_rows: 2,
        ..QueryLimits::default()
    };
    let dv = env
        .query_visual(
            &report,
            page,
            page.visual("table").unwrap(),
            &InteractionState::new(),
            &limits,
        )
        .unwrap();
    assert!(dv.truncated);
    assert_eq!(dv.table.unwrap().rows.len(), 2);
}

#[test]
fn matrices_have_subtotals_at_every_level() {
    let env = env();
    let report = report(page());
    let dv = query(&env, &report, "matrix", &InteractionState::new());
    assert_eq!(dv.kind, DataViewKind::Matrix);
    let m = dv.matrix.as_ref().unwrap();
    let headers: Vec<String> = m
        .columns
        .iter()
        .map(|c| {
            c.path
                .iter()
                .map(|p| p.formatted.clone())
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect();
    assert_eq!(headers, ["Card", "Cash", ""]);
    assert!(m.columns[2].is_total);
    // Grand total row: Card 3, Cash 2, total 5.
    let cells: Vec<&str> = m.rows.cells.iter().map(|c| c.formatted.as_str()).collect();
    assert_eq!(cells, ["3", "2", "5"]);
    let level0: Vec<&str> = m
        .rows
        .children
        .iter()
        .map(|n| n.value.as_ref().unwrap().formatted.as_str())
        .collect();
    assert_eq!(level0, ["(Blank)", "Flagship", "Outlet", "Pop-up"]);
    let flagship = &m.rows.children[1];
    let sub: Vec<&str> = flagship
        .cells
        .iter()
        .map(|c| c.formatted.as_str())
        .collect();
    assert_eq!(sub, ["1", "1", "2"]);
    let stores: Vec<&str> = flagship
        .children
        .iter()
        .map(|n| n.value.as_ref().unwrap().formatted.as_str())
        .collect();
    assert_eq!(stores, ["Store 1", "Store 2"]);
    assert_eq!(flagship.children[0].level, Some(1));
}

#[test]
fn matrix_levels_follow_the_sort_by_column() {
    let env = env();
    let mut page = page();
    let mut by_size = visual(
        "by_size",
        "pivotTable",
        vec![
            projection("Rows", col("Stores", "Size")),
            projection("Values", measure("Store Count")),
        ],
    );
    page.visuals.push(by_size.clone());
    by_size.name = "by_size_desc".into();
    by_size.sort = vec![SortItem {
        field: col("Stores", "Size"),
        direction: SortDirection::Descending,
    }];
    page.visuals.push(by_size);
    page.visuals.push(visual(
        "size_columns",
        "pivotTable",
        vec![
            projection("Rows", col("Stores", "Format")),
            projection("Columns", col("Stores", "Size")),
            projection("Values", measure("Store Count")),
        ],
    ));
    let report = report(page);
    let none = InteractionState::new();
    let rows = |name: &str| -> Vec<String> {
        let dv = query(&env, &report, name, &none);
        dv.matrix
            .unwrap()
            .rows
            .children
            .iter()
            .map(|n| n.value.as_ref().unwrap().formatted.clone())
            .collect()
    };
    // `Size` is sorted by `Size Order`, not alphabetically.
    assert_eq!(rows("by_size"), ["Small", "Medium", "Large"]);
    assert_eq!(rows("by_size_desc"), ["Large", "Medium", "Small"]);
    let dv = query(&env, &report, "size_columns", &none);
    let m = dv.matrix.unwrap();
    let headers: Vec<String> = m
        .columns
        .iter()
        .map(|c| c.path.iter().map(|p| p.formatted.clone()).collect())
        .collect();
    assert_eq!(headers, ["Small", "Medium", "Large", ""]);
    // Store Count per size in the grand total row: 2 small, 1 medium, 2 large.
    let cells: Vec<&str> = m.rows.cells.iter().map(|c| c.formatted.as_str()).collect();
    assert_eq!(cells, ["2", "1", "2", "5"]);
}

#[test]
fn failing_measures_report_their_dax() {
    let env = env();
    let mut page = page();
    page.visuals.push(card("broken", "Broken"));
    page.visuals.push(card("missing", "Nope"));
    let report = report(page);
    let err = try_query(&env, &report, "broken", &InteractionState::new()).unwrap_err();
    assert_eq!(err.dax.as_deref(), Some("NOSUCHFUNCTION(Stores[Staff])"));
    assert_eq!(err.field.as_deref(), Some("Broken"));
    assert!(err.display().starts_with("⚠ Broken: "));
    let err = try_query(&env, &report, "missing", &InteractionState::new()).unwrap_err();
    assert!(err.message.contains("Nope"), "{err:?}");
    // query_page returns every visual, failures included.
    let page = &report.pages[0];
    let results = env.query_page(
        &report,
        page,
        &InteractionState::new(),
        &QueryLimits::default(),
    );
    assert_eq!(results.len(), page.visuals.len());
    assert_eq!(results.iter().filter(|(_, r)| r.is_err()).count(), 2);
}

#[test]
fn report_page_and_visual_filters_with_warnings() {
    let env = env();
    let mut page = page();
    page.filters = vec![Filter {
        name: "pf".into(),
        kind: FilterKind::RelativeDate,
        field: Some(col("Stores", "Opened")),
        condition: Some(Condition::Comparison {
            kind: ComparisonKind::GreaterThan,
            left: col("Stores", "Opened"),
            right: lit("datetime'2026-01-01T00:00:00'"),
        }),
        ..Filter::default()
    }];
    let mut report = report(page);
    report.filters = vec![Filter {
        name: "rf".into(),
        kind: FilterKind::Categorical,
        condition: Some(Condition::In {
            exprs: vec![col("Stores", "Format")],
            values: vec![
                vec![Literal::text("Flagship")],
                vec![Literal::text("Pop-up")],
            ],
        }),
        is_hidden_in_view_mode: true,
        ..Filter::default()
    }];
    let dv = query(&env, &report, "total", &InteractionState::new());
    assert_eq!(single(&dv), "3");
    assert_eq!(dv.warnings.len(), 1, "{:?}", dv.warnings);
    assert!(dv.warnings[0].not_applied);
    assert_eq!(dv.warnings[0].source.as_deref(), Some("pf"));
    assert_eq!(
        single(&query(&env, &report, "sessions", &InteractionState::new())),
        "2"
    );
}

#[test]
fn visual_measure_filters_and_top_n() {
    let env = env();
    let mut page = page();
    let bar = page.visuals.iter_mut().find(|v| v.name == "bar").unwrap();
    bar.filters = vec![Filter {
        name: "vf".into(),
        kind: FilterKind::Advanced,
        condition: Some(Condition::Comparison {
            kind: ComparisonKind::GreaterThanOrEqual,
            left: measure("Order Count"),
            right: lit("2L"),
        }),
        ..Filter::default()
    }];
    let table = page.visuals.iter_mut().find(|v| v.name == "table").unwrap();
    let top = serde_json::json!({
        "name": "top",
        "type": "TopN",
        "filter": {
            "Version": 2,
            "From": [
                {"Name": "subquery", "Expression": {"Subquery": {"Query": {
                    "Version": 2,
                    "From": [{"Name": "a", "Entity": "Stores", "Type": 0}],
                    "Select": [{"Column": {"Expression": {"SourceRef": {"Source": "a"}}, "Property": "Store Name"}}],
                    "OrderBy": [{"Direction": 2, "Expression": {"Measure": {"Expression": {"SourceRef": {"Source": "a"}}, "Property": "Staff Sum"}}}],
                    "Top": 2
                }}}, "Type": 2},
                {"Name": "a", "Entity": "Stores", "Type": 0}
            ],
            "Where": [{"Condition": {"In": {
                "Expressions": [{"Column": {"Expression": {"SourceRef": {"Source": "a"}}, "Property": "Store Name"}}],
                "Table": {"SourceRef": {"Source": "subquery"}}
            }}}]
        }
    });
    table.filters = vec![Filter {
        name: "top".into(),
        kind: FilterKind::TopN,
        condition: Some(Condition::Unknown(serde_json::Value::Null)),
        raw: top,
        ..Filter::default()
    }];
    let report = report(page);
    let bar = query(&env, &report, "bar", &InteractionState::new());
    assert_eq!(categories(&bar), ["Store 1"]);
    let table = query(&env, &report, "table", &InteractionState::new());
    let names: Vec<&str> = table
        .table
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .map(|r| r[0].formatted.as_str())
        .collect();
    assert_eq!(names, ["Store 4", "Store 3"]);
    assert!(table.warnings.is_empty(), "{:?}", table.warnings);
}

#[test]
fn dataviews_serialize_for_the_app() {
    let env = env();
    let report = report(page());
    let dv = query(&env, &report, "bar", &InteractionState::new());
    let json = serde_json::to_value(&dv).unwrap();
    assert_eq!(json["kind"], "categorical");
    assert_eq!(
        json["categorical"]["categories"][0]["values"][0]["raw"],
        "Store 1"
    );
    assert_eq!(json["categorical"]["values"][0]["values"][0]["raw"], 2);
    assert_eq!(json["metadata"][1]["queryRef"], "Stores.Order Count");
}

#[test]
fn allselected_sees_the_visual_filter_context() {
    let env = env();
    let mut page = page();
    page.visuals.push(visual(
        "share",
        "columnChart",
        vec![
            projection("Category", col("Stores", "Format")),
            projection("Y", measure("Share")),
        ],
    ));
    let report = report(page);
    let mut state = InteractionState::new();
    state.select("format", SlicerSelection::excluding(["Pop-up"]));
    let dv = query(&env, &report, "share", &state);
    // 4 selected stores: Flagship 2 → 50%, Outlet 1 and (Blank) 1 → 25%.
    assert_eq!(categories(&dv), ["Flagship", "(Blank)", "Outlet"]);
    assert_eq!(values(&dv, 0), ["50.0%", "25.0%", "25.0%"]);
}

#[test]
fn table_columns_summarized_through_their_query_ref() {
    let env = env();
    let mut page = page();
    let mut staff = projection("Values", col("Stores", "Staff"));
    staff.query_ref = "Sum(Stores.Staff)".into();
    staff.native_query_ref = Some("Staff".into());
    page.visuals.push(visual(
        "plain",
        "tableEx",
        vec![projection("Values", col("Stores", "Store Name")), staff],
    ));
    let report = report(page);
    let dv = query(&env, &report, "plain", &InteractionState::new());
    assert!(dv.metadata[1].is_measure);
    assert_eq!(
        dv.metadata[1].expression.as_deref(),
        Some("SUM(Stores[Staff])")
    );
    let t = dv.table.unwrap();
    assert_eq!(t.rows.len(), 4);
    assert_eq!(t.totals.unwrap()[1].formatted, "100");
}

#[test]
fn scatter_gauge_treemap_and_waterfall_query_their_roles() {
    use pbi_render::visuals::charts::{ChartData, waterfall};
    let env = env();
    let page = Page {
        name: "p1".into(),
        visuals: vec![
            visual(
                "waterfall",
                "waterfallChart",
                vec![
                    projection("Category", col("Stores", "Format")),
                    projection("Breakdown", col("Orders", "Payment")),
                    projection("Y", measure("Order Count")),
                ],
            ),
            visual(
                "scatter",
                "scatterChart",
                vec![
                    projection("Category", col("Stores", "Store Name")),
                    projection("Series", col("Stores", "Format")),
                    projection("X", measure("Staff Sum")),
                    projection("Y", measure("Order Count")),
                    projection("Size", measure("Store Count")),
                ],
            ),
            visual(
                "gauge",
                "gauge",
                vec![
                    projection("Y", measure("Order Count")),
                    projection("MaxValue", measure("Store Count")),
                    projection("TargetValue", measure("Staff Sum")),
                ],
            ),
            visual(
                "treemap",
                "treemap",
                vec![
                    projection("Group", col("Stores", "Format")),
                    projection("Details", col("Stores", "Store Name")),
                    projection("Values", measure("Order Count")),
                ],
            ),
        ],
        ..Page::default()
    };
    let report = report(page);
    let none = InteractionState::new();

    // Waterfall: Breakdown is the series field; categories ascending (the
    // service's default for waterfalls), not by value.
    let wf = query(&env, &report, "waterfall", &none);
    let cat = wf.categorical.as_ref().unwrap();
    let breakdown = &wf.metadata[cat.series_meta_idx.unwrap()];
    assert_eq!(breakdown.roles, ["Breakdown"]);
    assert!(!breakdown.is_measure);
    let cats = categories(&wf);
    assert_eq!(cats, ["(Blank)", "Flagship", "Outlet", "Pop-up"]);
    let series: Vec<&str> = cat
        .values
        .iter()
        .map(|v| v.series.as_ref().unwrap().formatted.as_str())
        .collect();
    assert_eq!(series, ["Card", "Cash"]);
    let steps = waterfall::steps(&ChartData::from_dataview(&wf), 5);
    assert_eq!(steps[0].label, "(Blank)");
    assert_eq!(steps.last().unwrap().label, "Pop-up");

    // Scatter: X, Y and Size per legend value, categories ascending.
    let sc = query(&env, &report, "scatter", &none);
    let cat = sc.categorical.as_ref().unwrap();
    assert!(cat.series_meta_idx.is_some());
    let roles: Vec<&str> = cat
        .values
        .iter()
        .map(|v| sc.metadata[v.meta_idx].roles[0].as_str())
        .collect();
    assert_eq!(roles.len() % 3, 0);
    for chunk in roles.chunks(3) {
        assert_eq!(chunk, ["X", "Y", "Size"]);
    }
    let names = categories(&sc);
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);

    // Gauge: one row, one cell per role.
    let gauge = query(&env, &report, "gauge", &none);
    assert_eq!(gauge.kind, DataViewKind::Categorical);
    assert_eq!(values(&gauge, 0), ["5"]);
    assert_eq!(values(&gauge, 1), ["5"]);
    assert_eq!(values(&gauge, 2), ["100"]);

    // Treemap: Group × Details rows, by value descending.
    let tm = query(&env, &report, "treemap", &none);
    let cat = tm.categorical.as_ref().unwrap();
    assert_eq!(cat.categories.len(), 2);
    let v: Vec<i64> = cat.values[0]
        .values
        .iter()
        .map(|c| c.raw.as_i64().unwrap_or(0))
        .collect();
    assert!(v.windows(2).all(|w| w[0] >= w[1]), "{v:?}");
    assert_eq!(v.iter().sum::<i64>(), 5);
}
