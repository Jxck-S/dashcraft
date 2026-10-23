//! Conditional formatting and dynamic format strings end to end on a
//! synthetic model (`cf_support`): semantic query (hidden rule inputs,
//! per-cell effective format strings) → DataView → tables, matrices,
//! cards and charts → scene / SVG.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod cf_support;

use cf_support::*;
use pbi_engine::Value;
use pbi_model::{Page, Visual};
use pbi_render::conditional::icons;
use pbi_render::dataview::{CellValue, DataView};
use pbi_render::query::{InteractionState, QueryEnv, QueryLimits};
use pbi_render::scene::{Color, Group, HitId, Node};
use pbi_render::svg::{SvgOptions, to_svg};
use pbi_render::theme::ThemeResolver;
use pbi_render::visuals::page::{
    PageViewState, RenderedPage, query_page_data, render_interactive_page_cached,
};
use pbi_render::visuals::{PageRenderOptions, VisualCache, render_report_page};
use pretty_assertions::assert_eq;
use serde_json::json;

fn query(env: &QueryEnv, v: &Visual) -> DataView {
    let page = page("p", vec![v.clone()]);
    env.query_visual(
        &report(&page),
        &page,
        v,
        &InteractionState::new(),
        &QueryLimits::default(),
    )
    .unwrap_or_else(|e| panic!("{}: {e:?}", v.name))
}

fn render(env: &QueryEnv, page: &Page) -> RenderedPage {
    render_report_page(
        env,
        &report(page),
        page,
        &ThemeResolver::default(),
        &InteractionState::new(),
        &PageRenderOptions::default(),
    )
}

fn find_group<'a>(g: &'a Group, id: &HitId) -> Option<&'a Group> {
    if g.id.as_ref() == Some(id) {
        return Some(g);
    }
    g.children.iter().find_map(|n| match n {
        Node::Group(c) => find_group(c, id),
        _ => None,
    })
}

fn walk<'a>(g: &'a Group, out: &mut Vec<&'a Node>) {
    for n in &g.children {
        out.push(n);
        if let Node::Group(c) = n {
            walk(c, out);
        }
    }
}

fn nodes(g: &Group) -> Vec<&Node> {
    let mut out = Vec::new();
    walk(g, &mut out);
    out
}

fn texts(g: &Group) -> Vec<String> {
    nodes(g)
        .into_iter()
        .filter_map(|n| match n {
            Node::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect()
}

/// The data-point group `(series, index)` of `visual`.
fn point<'a>(r: &'a RenderedPage, visual: &str, series: usize, index: usize) -> &'a Group {
    find_group(&r.scene.root, &HitId::data_point(visual, series, index))
        .unwrap_or_else(|| panic!("{visual} ({series}, {index}) not drawn"))
}

/// Fill colors of the rectangles and paths in a group, in order.
fn fills(g: &Group) -> Vec<String> {
    nodes(g)
        .into_iter()
        .filter_map(|n| match n {
            Node::Rect(r) => r.fill.filter(|p| p.opacity > 0.0).map(|p| p.color.to_hex()),
            Node::Path(p) => p.fill.filter(|p| p.opacity > 0.0).map(|p| p.color.to_hex()),
            _ => None,
        })
        .collect()
}

fn text_colors(g: &Group) -> Vec<(String, String)> {
    nodes(g)
        .into_iter()
        .filter_map(|n| match n {
            Node::Text(t) => Some((t.text.clone(), t.style.color.to_hex())),
            _ => None,
        })
        .collect()
}

fn cell_texts(cells: &[CellValue]) -> Vec<&str> {
    cells.iter().map(|c| c.formatted.as_str()).collect()
}

// ---- dynamic format strings --------------------------------------------------

#[test]
fn calculation_items_format_cells_with_their_own_format_strings() {
    let env = env();
    // Matrix: Region × item.
    let m = visual(
        "m",
        "pivotTable",
        (0.0, 0.0, 400.0, 200.0),
        vec![
            ("Rows", col("Region")),
            ("Columns", item()),
            ("Values", measure("Total Sales")),
        ],
        json!({}),
    );
    let dv = query(&env, &m);
    let matrix = dv.matrix.as_ref().unwrap();
    let north = &matrix.rows.children[1];
    assert_eq!(north.value.as_ref().unwrap().formatted, "North");
    // Current, YOY %, total column (no item: the measure's own format).
    assert_eq!(cell_texts(&north.cells), ["$300", "100.0%", "$300"]);
    assert_eq!(north.cells[0].format, None);
    assert_eq!(north.cells[1].format.as_deref(), Some("0.0%"));
    let east = &matrix.rows.children[0];
    assert_eq!(cell_texts(&east.cells), ["$120", "", "$120"]);
    assert_eq!(cell_texts(&matrix.rows.cells), ["$960", "82.4%", "$960"]);
    assert_eq!(dv.metadata[2].format_string.as_deref(), Some("\\$#,0"));

    // Table: Region, item, Total Sales; the total row has no item.
    let t = visual(
        "t",
        "tableEx",
        (0.0, 0.0, 400.0, 200.0),
        vec![
            ("Values", col("Region")),
            ("Values", item()),
            ("Values", measure("Total Sales")),
        ],
        json!({}),
    );
    let dv = query(&env, &t);
    let table = dv.table.as_ref().unwrap();
    let rows: Vec<Vec<&str>> = table.rows.iter().map(|r| cell_texts(r)).collect();
    assert_eq!(
        rows,
        [
            vec!["East", "Current", "$120"],
            vec!["North", "Current", "$300"],
            vec!["North", "YOY %", "100.0%"],
            vec!["South", "Current", "$500"],
            vec!["South", "YOY %", "50.0%"],
            vec!["West", "Current", "$40"],
            vec!["West", "YOY %", "-100.0%"],
        ]
    );
    assert_eq!(cell_texts(table.totals.as_ref().unwrap())[2], "$960");

    // Cards: one under the YOY % item (visual filter), one without.
    let mut yoy = visual(
        "yoy",
        "cardVisual",
        (0.0, 0.0, 200.0, 100.0),
        vec![("Data", measure("Total Sales"))],
        json!({}),
    );
    yoy.filters = vec![item_filter("YOY %")];
    let single = query(&env, &yoy).single.unwrap();
    assert_eq!(single.formatted, "82.4%");
    assert_eq!(single.format.as_deref(), Some("0.0%"));
    let plain = visual(
        "plain",
        "cardVisual",
        (0.0, 120.0, 200.0, 100.0),
        vec![("Data", measure("Total Sales"))],
        json!({}),
    );
    let single = query(&env, &plain).single.unwrap();
    assert_eq!((single.formatted.as_str(), single.format), ("$960", None));

    // Rendered: the card shows the percent, the matrix both formats.
    let r = render(&env, &page("p", vec![yoy, m]));
    let all = texts(&r.scene.root);
    assert!(all.contains(&"82.4%".to_owned()), "{all:?}");
    assert!(all.contains(&"100.0%".to_owned()) && all.contains(&"$300".to_owned()));
}

#[test]
fn measure_format_string_definitions_format_per_row() {
    let env = env();
    let t = visual(
        "t",
        "tableEx",
        (0.0, 0.0, 300.0, 200.0),
        vec![("Values", col("Region")), ("Values", measure("Scaled"))],
        json!({}),
    );
    let dv = query(&env, &t);
    let table = dv.table.as_ref().unwrap();
    let scaled: Vec<(&str, Option<&str>)> = table
        .rows
        .iter()
        .map(|r| (r[1].formatted.as_str(), r[1].format.as_deref()))
        .collect();
    assert_eq!(
        scaled,
        [
            ("120", Some("0")),
            ("300.0", Some("#,0.0")),
            ("500.0", Some("#,0.0")),
            ("40", Some("0"))
        ]
    );
    assert_eq!(table.totals.as_ref().unwrap()[1].formatted, "960.0");
}

#[test]
fn chart_labels_use_each_cells_format_string() {
    let env = env();
    // Display units in thousands: the currency value scales, the YOY %
    // value keeps its percent format.
    let chart = visual(
        "c",
        "clusteredColumnChart",
        (0.0, 0.0, 500.0, 300.0),
        vec![("Category", item()), ("Y", measure("Total Sales"))],
        json!({"labels": [{"properties": {"show": prop("true"), "labelDisplayUnits": prop("1000D")}}]}),
    );
    let dv = query(&env, &chart);
    let values = &dv.categorical.as_ref().unwrap().values[0].values;
    assert_eq!(cell_texts(values), ["$960", "82.4%"]);
    let r = render(&env, &page("p", vec![chart]));
    let labels = texts(&r.scene.root);
    assert!(labels.contains(&"82.4%".to_owned()), "{labels:?}");
    assert!(labels.contains(&"$0.96K".to_owned()), "{labels:?}");
}

// ---- tables and matrices -------------------------------------------------------

#[test]
fn table_gradients_field_values_data_bars_and_icons() {
    let env = env();
    let t = showcase_table("t", (0.0, 0.0, 420.0, 220.0));
    let dv = query(&env, &t);
    // The field-value measures are hidden inputs, not columns.
    let hidden: Vec<&str> = dv
        .metadata
        .iter()
        .filter(|m| m.hidden)
        .map(|m| m.query_ref.as_str())
        .collect();
    assert_eq!(hidden, ["Sales.Status Color", "Sales.Status Icon"]);
    let table = dv.table.as_ref().unwrap();
    assert_eq!(table.columns, [0, 1, 2]);
    assert_eq!(table.hidden.len(), 2);
    assert_eq!(
        cell_texts(&table.hidden[0].values),
        ["#00AA00", "#00AA00", "#00AA00", "Red"]
    );
    assert!(dv.warnings.is_empty(), "{:?}", dv.warnings);

    let r = render(&env, &page("p", vec![t]));
    // Rows: East 120, North 300, South 500, West 40. Background gradient
    // over 40..500 from white to #118DFF.
    let bg: Vec<String> = (0..4)
        .map(|i| fills(point(&r, "t", 1, i))[0].clone())
        .collect();
    assert_eq!(bg, ["#D6EBFF", "#78BFFF", "#118DFF", "#FFFFFF"]);
    // Region font colors from [Status Color].
    let regions: Vec<(String, String)> = (0..4)
        .map(|i| text_colors(point(&r, "t", 0, i))[0].clone())
        .collect();
    assert_eq!(
        regions,
        [
            ("East".to_owned(), "#00AA00".to_owned()),
            ("North".to_owned(), "#00AA00".to_owned()),
            ("South".to_owned(), "#00AA00".to_owned()),
            ("West".to_owned(), "#FF0000".to_owned()),
        ]
    );
    // Icons from [Status Icon]: up arrows (green) except West (red).
    let icon_color = |i: usize| {
        let cell = point(&r, "t", 1, i);
        let icon = nodes(cell)
            .into_iter()
            .find_map(|n| match n {
                Node::Group(g) if g.class.as_deref() == Some("pbi-cf-icon") => Some(g),
                _ => None,
            })
            .expect("icon");
        fills(icon)[0].clone()
    };
    assert_eq!(icon_color(0), icons::GREEN.to_hex());
    assert_eq!(icon_color(3), icons::RED.to_hex());
    // Data bars on Margin (20, 60, 220, -20): the axis sits at 20/240 of
    // the bar area; West's bar is red and left of it.
    let bar_rects = |i: usize| -> Vec<(String, f64, f64)> {
        nodes(point(&r, "t", 2, i))
            .into_iter()
            .filter_map(|n| match n {
                Node::Rect(rect) => rect
                    .fill
                    .filter(|p| p.opacity > 0.0)
                    .map(|p| (p.color.to_hex(), rect.rect.x, rect.rect.width)),
                _ => None,
            })
            .collect()
    };
    let south = bar_rects(2);
    let west = bar_rects(3);
    assert_eq!(south[0].0, "#5FA35F");
    assert_eq!(west[0].0, "#D64554");
    let axis = south[1].clone();
    assert_eq!(axis.0, "#252423");
    let area = south[0].2 / (1.0 - 20.0 / 240.0);
    assert!((west[0].2 - area * 20.0 / 240.0).abs() < 0.01, "{west:?}");
    assert!((west[0].1 + west[0].2 - south[0].1).abs() < 0.01);
    // The values are still drawn over the bars.
    let margin_texts = texts(point(&r, "t", 2, 3));
    assert_eq!(margin_texts, ["-20"]);
}

#[test]
fn table_rules_and_font_gradients_with_blanks() {
    let env = env();
    // Sales 2026: East 120, North 200, South 300, West BLANK (its row
    // stays because the Region font color comes from a measure that is
    // never BLANK). A three-color font gradient (BLANK: no color) and
    // rules by range.
    let sales = measure_expr("Sales 2026");
    let t = visual(
        "t",
        "tableEx",
        (0.0, 0.0, 400.0, 200.0),
        vec![("Values", col("Region")), ("Values", measure("Sales 2026"))],
        json!({"values": [
            values_entry("Sales.Region", 1, json!({"fontColor": cf(measure_expr("Status Color"))})),
            values_entry("Sales.Sales 2026", 1, json!({
                "fontColor": cf(gradient3(sales.clone(), "#FF0000", "#FFFFFF", "#0000FF")),
                "backColor": cf(rules(vec![
                    (cmp(&sales, 0, "null"), "'#C8C6C4'"),
                    (cmp(&sales, 3, "150D"), "'#F4B6BB'"),
                    (and(cmp(&sales, 2, "150D"), cmp(&sales, 3, "250D")), "'#FCE6A4'"),
                ]))
            }))
        ]}),
    );
    let r = render(&env, &page("p", vec![t]));
    let back: Vec<Vec<String>> = (0..4).map(|i| fills(point(&r, "t", 1, i))).collect();
    assert_eq!(
        back,
        [
            vec!["#F4B6BB".to_owned()],
            vec!["#FCE6A4".to_owned()],
            vec![],
            vec!["#C8C6C4".to_owned()],
        ]
    );
    // Font over 120..300 (middle 210): red → white → blue.
    let font: Vec<(String, String)> = (0..3)
        .map(|i| text_colors(point(&r, "t", 1, i))[0].clone())
        .collect();
    assert_eq!(
        font,
        [
            ("$120".to_owned(), "#FF0000".to_owned()),
            ("$200".to_owned(), "#FFE3E3".to_owned()),
            ("$300".to_owned(), "#0000FF".to_owned()),
        ]
    );
    assert!(texts(point(&r, "t", 1, 3)).is_empty());
}

#[test]
fn matrix_rules_apply_to_values_not_totals() {
    let env = env();
    let m = showcase_matrix("m", (0.0, 0.0, 420.0, 220.0));
    let r = render(&env, &page("p", vec![m]));
    // Grid columns: Region, Current, YOY %, Total. Rows East, North,
    // South, West.
    let colors = |row: usize| -> Vec<Option<String>> {
        (1..4)
            .map(|c| fills(point(&r, "m", c, row)).first().cloned())
            .collect()
    };
    let green = Some("#B7E1B4".to_owned());
    assert_eq!(colors(0), [green.clone(), Some("#C8C6C4".into()), None]);
    assert_eq!(colors(1), [green.clone(), green.clone(), None]);
    assert_eq!(colors(2), [green.clone(), Some("#FCE6A4".into()), None]);
    assert_eq!(colors(3), [green, Some("#F4B6BB".into()), None]);
}

// ---- charts -------------------------------------------------------------------

#[test]
fn chart_conditional_fills() {
    let env = env();
    let margin = measure_expr("Margin");
    let column = visual(
        "column",
        "clusteredColumnChart",
        (0.0, 0.0, 400.0, 300.0),
        vec![("Category", col("Region")), ("Y", measure("Margin"))],
        data_point_fill(gradient2(margin.clone(), "#FFFFFF", "#000000")),
    );
    let pie = visual(
        "pie",
        "pieChart",
        (400.0, 0.0, 400.0, 300.0),
        vec![("Category", col("Region")), ("Y", measure("Total Sales"))],
        data_point_fill(measure_expr("Status Color")),
    );
    let funnel = visual(
        "funnel",
        "funnel",
        (800.0, 0.0, 400.0, 300.0),
        vec![("Category", col("Region")), ("Y", measure("Total Sales"))],
        data_point_fill(rules(vec![(cmp(&margin, 3, "50D"), "'#FF0000'")])),
    );
    let treemap = visual(
        "treemap",
        "treemap",
        (0.0, 300.0, 400.0, 300.0),
        vec![("Group", col("Region")), ("Values", measure("Total Sales"))],
        data_point_fill(measure_expr("Status Color")),
    );
    let scatter = visual(
        "scatter",
        "scatterChart",
        (400.0, 300.0, 400.0, 300.0),
        vec![
            ("Details", col("Region")),
            ("X", measure("Margin")),
            ("Y", measure("Total Sales")),
        ],
        data_point_fill(measure_expr("Status Color")),
    );
    let r = render(
        &env,
        &page("p", vec![column, pie, funnel, treemap, scatter]),
    );
    // Column: sorted by Margin descending (South 220, North 60, East 20,
    // West -20) on a white → black scale over -20..220.
    let col_fill: Vec<String> = (0..4)
        .map(|i| fills(point(&r, "column", 0, i))[0].clone())
        .collect();
    assert_eq!(col_fill, ["#000000", "#AAAAAA", "#D5D5D5", "#FFFFFF"]);
    // Pie, treemap and scatter: West (last by Total Sales) is red, the
    // others green.
    let pie_fill: Vec<String> = (0..4)
        .map(|i| fills(point(&r, "pie", 0, i))[0].clone())
        .collect();
    assert_eq!(pie_fill, ["#00AA00", "#00AA00", "#00AA00", "#FF0000"]);
    assert_eq!(fills(point(&r, "treemap", 0, 3))[0], "#FF0000");
    assert_eq!(fills(point(&r, "treemap", 0, 0))[0], "#00AA00");
    // Funnel: Margin < 50 in red, the others keep the default color.
    let funnel_fill: Vec<String> = (0..4)
        .map(|i| fills(point(&r, "funnel", 0, i))[0].clone())
        .collect();
    let base = ThemeResolver::default().data_color(0).to_hex();
    assert_eq!(
        funnel_fill,
        [base.clone(), base, "#FF0000".into(), "#FF0000".into()]
    );
    let scatter_fills: Vec<String> = nodes(&r.scene.root)
        .into_iter()
        .filter_map(|n| match n {
            Node::Group(g)
                if matches!(&g.id, Some(id) if id.visual == "scatter" && id != &HitId::visual("scatter")) =>
            {
                fills(g).first().cloned()
            }
            _ => None,
        })
        .collect();
    assert!(
        scatter_fills.contains(&"#FF0000".to_owned()),
        "{scatter_fills:?}"
    );
    assert!(
        scatter_fills.contains(&"#00AA00".to_owned()),
        "{scatter_fills:?}"
    );
}

// ---- determinism, cache, diagnostics ---------------------------------------------

fn showcase_page() -> Page {
    page(
        "showcase",
        vec![
            showcase_table("t", (0.0, 0.0, 420.0, 220.0)),
            showcase_matrix("m", (440.0, 0.0, 420.0, 220.0)),
        ],
    )
}

#[test]
fn renders_are_deterministic() {
    let env = env();
    let page = showcase_page();
    let a = to_svg(&render(&env, &page).scene, &SvgOptions::default());
    let b = to_svg(
        &render(&cf_support::env(), &page).scene,
        &SvgOptions::default(),
    );
    assert_eq!(a, b);
}

#[test]
fn the_render_cache_follows_rule_changes() {
    let env = env();
    let theme = ThemeResolver::default();
    let state = InteractionState::new();
    let opts = PageRenderOptions::default();
    let view = PageViewState::new();
    let cache = VisualCache::new();
    let draw = |page: &Page, cache: Option<&VisualCache>| {
        let report = report(page);
        let data = query_page_data(&env, &report, page, &state, &opts.limits);
        let r = render_interactive_page_cached(
            &report, page, &theme, &state, &opts, &view, &data, cache,
        );
        to_svg(&r.page.scene, &SvgOptions::default())
    };
    let first = showcase_page();
    let svg1 = draw(&first, Some(&cache));
    assert_eq!(svg1, draw(&first, Some(&cache)));
    let hits = cache.stats().hits;
    // A new maximum color, and font colors from a gradient on Margin
    // instead of [Status Color] (a different hidden input).
    let mut second = first.clone();
    let table = &mut second.visuals[0];
    table.objects["values"][1]["properties"]["backColor"] =
        cf(gradient2(measure_expr("Total Sales"), "#FFFFFF", "#E81123"));
    table.objects["values"][0]["properties"]["fontColor"] =
        cf(gradient2(measure_expr("Margin"), "#000000", "#00FF00"));
    let svg2 = draw(&second, Some(&cache));
    assert_ne!(svg1, svg2);
    assert!(svg2.contains("#E81123") && !svg1.contains("#E81123"));
    assert_eq!(svg2, draw(&second, None));
    // The unchanged matrix was reused.
    assert!(cache.stats().hits > hits);
}

#[test]
fn unsupported_and_broken_rules_render_without_formatting() {
    let env = env();
    let t = visual(
        "t",
        "tableEx",
        (0.0, 0.0, 400.0, 200.0),
        vec![
            ("Values", col("Region")),
            ("Values", measure("Total Sales")),
        ],
        json!({
            "values": [
                values_entry("Sales.Total Sales", 1, json!({"backColor": cf(json!({"FillRule": {
                    "Input": measure_expr("Total Sales"),
                    "FillRule": {"stepped": {}}
                }}))})),
                values_entry("Sales.Region", 1, json!({"fontColor": cf(measure_expr("Broken"))})),
                values_entry("Sales.Region", 1, json!({"backColor": cf(measure_expr("Missing"))}))
            ],
            "columnHeaders": [{"properties": {"fontColor": cf(measure_expr("Status Color"))}}]
        }),
    );
    let dv = query(&env, &t);
    let warnings: Vec<&str> = dv.warnings.iter().map(|w| w.message.as_str()).collect();
    assert_eq!(warnings.len(), 4, "{warnings:?}");
    assert!(
        warnings
            .iter()
            .all(|w| w.starts_with("conditional formatting"))
    );
    assert!(dv.warnings.iter().all(|w| w.not_applied));
    assert!(warnings.iter().any(|w| w.contains("stepped")));
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("columnHeaders.fontColor"))
    );
    assert!(warnings.iter().any(|w| w.contains("Missing")));
    assert!(warnings.iter().any(|w| w.contains("Broken")));
    // No hidden input survived; the table renders plainly.
    assert!(dv.metadata.iter().all(|m| !m.hidden));
    let r = render(&env, &page("p", vec![t]));
    assert!(fills(point(&r, "t", 1, 0)).is_empty());
    assert_eq!(text_colors(point(&r, "t", 0, 0))[0].1, "#252423");

    // A chart with a legend keeps its series colors (with a diagnostic).
    let legend = visual(
        "legend",
        "clusteredColumnChart",
        (0.0, 0.0, 400.0, 300.0),
        vec![
            ("Category", col("Region")),
            ("Series", col("Product")),
            ("Y", measure("Total Sales")),
        ],
        data_point_fill(measure_expr("Status Color")),
    );
    let dv = query(&env, &legend);
    assert_eq!(dv.warnings.len(), 1);
    assert!(dv.warnings[0].message.contains("legend"));
    assert!(dv.categorical.unwrap().hidden.is_empty());
}

#[test]
fn hidden_inputs_keep_rows_power_bi_would_show() {
    // A field-value measure that is never BLANK keeps rows whose visible
    // values are all BLANK (as in Power BI, where the formatting measure
    // is part of the visual's query).
    let env = env();
    let mut t = visual(
        "t",
        "tableEx",
        (0.0, 0.0, 400.0, 200.0),
        vec![("Values", col("Region")), ("Values", measure("Sales 2026"))],
        json!({}),
    );
    let plain = query(&env, &t);
    assert_eq!(plain.table.as_ref().unwrap().rows.len(), 3);
    t.objects = json!({"values": [values_entry(
        "Sales.Sales 2026",
        1,
        json!({"fontColor": cf(measure_expr("Status Color"))})
    )]});
    let formatted = query(&env, &t);
    let table = formatted.table.unwrap();
    assert_eq!(table.rows.len(), 4);
    assert_eq!(table.rows[3][0].raw, Value::from("West"));
    assert!(table.rows[3][1].is_blank());
    assert_eq!(table.hidden[0].values[3].formatted, "Red");
    assert_eq!(table.hidden[0].total.as_ref().unwrap().formatted, "#00AA00");
    assert_eq!(Color::parse("#00AA00"), Some(Color::rgb(0, 0xAA, 0)));
}
