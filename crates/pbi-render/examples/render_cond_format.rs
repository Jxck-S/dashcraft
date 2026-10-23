//! Dev tool: renders conditional formatting and dynamic format strings end
//! to end on the synthetic sales model of the tests (in-memory model →
//! semantic query → DataView → visuals → PNG/SVG), for eyeballing.
//!
//! ```text
//! cargo run -p pbi-render --example render_cond_format -- [--out target/tmp/cond-format]
//! ```
//! Writes `tables.png` (table and matrix backgrounds, font colors, rules,
//! data bars, icons), `formats.png` (calculation-item format strings on
//! cards, a matrix and chart labels) and `charts.png` (conditional data
//! colors) at 2×, with their SVGs. Output stays under `target/` (never
//! commit renders).

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "../tests/cf_support/mod.rs"]
mod cf_support;

use std::path::{Path, PathBuf};

use cf_support::*;
use pbi_model::Page;
use pbi_render::png::{PngOptions, render_png};
use pbi_render::query::{InteractionState, QueryEnv};
use pbi_render::scene::Color;
use pbi_render::svg::{SvgOptions, to_svg};
use pbi_render::theme::ThemeResolver;
use pbi_render::visuals::{PageRenderOptions, render_report_page};
use serde_json::json;

fn tables_page() -> Page {
    let sales = measure_expr("Sales 2026");
    let margin = measure_expr("Margin");
    let rules_table = visual(
        "rules",
        "tableEx",
        (10.0, 250.0, 420.0, 200.0),
        vec![
            ("Values", col("Region")),
            ("Values", measure("Sales 2026")),
            ("Values", measure("Margin")),
        ],
        json!({"values": [
            values_entry("Sales.Region", 1, json!({"fontColor": cf(measure_expr("Status Color"))})),
            values_entry("Sales.Sales 2026", 0, json!({
                "fontColor": cf(gradient3(sales.clone(), "#D64554", "#605E5C", "#118DFF")),
                "backColor": cf(rules(vec![
                    (cmp(&sales, 0, "null"), "'#E1DFDD'"),
                    (cmp(&sales, 3, "150D"), "'#F4B6BB'"),
                    (and(cmp(&sales, 2, "150D"), cmp(&sales, 3, "250D")), "'#FCE6A4'"),
                    (cmp(&sales, 2, "250D"), "'#B7E1B4'"),
                ]))
            })),
            values_entry("Sales.Margin", 1, json!({"icon": {
                "kind": "Icon",
                "layout": lit("'After'"),
                "value": {"expr": rules(vec![
                    (cmp(&margin, 3, "0D"), "'triangle_down_red'"),
                    (and(cmp(&margin, 2, "0D"), cmp(&margin, 3, "50D")), "'triangle_dash'"),
                    (cmp(&margin, 2, "50D"), "'triangle_up_green'"),
                ])}
            }}))
        ]}),
    );
    let icon_sets = visual(
        "iconSets",
        "tableEx",
        (440.0, 250.0, 420.0, 200.0),
        vec![
            ("Values", col("Region")),
            ("Values", measure("Total Sales")),
            ("Values", measure("Margin")),
        ],
        json!({"values": [
            values_entry("Sales.Total Sales", 1, json!({"icon": {
                "kind": "Icon",
                "layout": lit("'IconOnly'"),
                "value": {"expr": rules(vec![
                    (cmp(&measure_expr("Total Sales"), 3, "100D"), "'circle_red'"),
                    (cmp(&measure_expr("Total Sales"), 3, "400D"), "'circle_yellow'"),
                    (cmp(&measure_expr("Total Sales"), 2, "400D"), "'circle_green'"),
                ])}
            }})),
            values_entry("Sales.Margin", 1, json!({"dataBars": {
                "positiveColor": color("#118DFF"),
                "negativeColor": color("#D64554"),
                "axisColor": color("#252423"),
                "hideText": prop("true")
            }}))
        ]}),
    );
    page(
        "tables",
        vec![
            showcase_table("showcase", (10.0, 10.0, 420.0, 220.0)),
            showcase_matrix("matrix", (440.0, 10.0, 420.0, 220.0)),
            rules_table,
            icon_sets,
        ],
    )
}

fn formats_page() -> Page {
    let mut yoy = visual(
        "yoyCard",
        "cardVisual",
        (10.0, 10.0, 240.0, 110.0),
        vec![("Data", measure("Total Sales"))],
        json!({}),
    );
    yoy.filters = vec![item_filter("YOY %")];
    let current = visual(
        "currentCard",
        "cardVisual",
        (260.0, 10.0, 210.0, 110.0),
        vec![("Data", measure("Total Sales"))],
        json!({}),
    );
    let matrix = visual(
        "matrix",
        "pivotTable",
        (10.0, 140.0, 410.0, 200.0),
        vec![
            ("Rows", col("Region")),
            ("Columns", item()),
            ("Values", measure("Total Sales")),
        ],
        json!({}),
    );
    let chart = visual(
        "chart",
        "clusteredColumnChart",
        (490.0, 10.0, 420.0, 330.0),
        vec![
            ("Category", col("Region")),
            ("Series", item()),
            ("Y", measure("Total Sales")),
        ],
        json!({"labels": [{"properties": {"show": prop("true")}}]}),
    );
    let scaled = visual(
        "scaled",
        "tableEx",
        (10.0, 360.0, 410.0, 200.0),
        vec![("Values", col("Region")), ("Values", measure("Scaled"))],
        json!({}),
    );
    page("formats", vec![yoy, current, matrix, chart, scaled])
}

fn charts_page() -> Page {
    let margin = measure_expr("Margin");
    page(
        "charts",
        vec![
            visual(
                "column",
                "clusteredColumnChart",
                (10.0, 10.0, 410.0, 300.0),
                vec![("Category", col("Region")), ("Y", measure("Margin"))],
                data_point_fill(gradient3(margin.clone(), "#D64554", "#E1DFDD", "#1AAB40")),
            ),
            visual(
                "pie",
                "pieChart",
                (430.0, 10.0, 410.0, 300.0),
                vec![("Category", col("Region")), ("Y", measure("Total Sales"))],
                data_point_fill(measure_expr("Status Color")),
            ),
            visual(
                "bar",
                "clusteredBarChart",
                (850.0, 10.0, 410.0, 300.0),
                vec![("Category", col("Region")), ("Y", measure("Total Sales"))],
                data_point_fill(rules(vec![
                    (cmp(&margin, 3, "0D"), "'#D64554'"),
                    (cmp(&margin, 3, "100D"), "'#F2C80F'"),
                ])),
            ),
            visual(
                "funnel",
                "funnel",
                (10.0, 330.0, 410.0, 300.0),
                vec![("Category", col("Region")), ("Y", measure("Total Sales"))],
                data_point_fill(gradient2(measure_expr("Total Sales"), "#C6E2FF", "#0B5394")),
            ),
            visual(
                "treemap",
                "treemap",
                (430.0, 330.0, 410.0, 300.0),
                vec![("Group", col("Region")), ("Values", measure("Total Sales"))],
                data_point_fill(gradient2(margin.clone(), "#FDE2E4", "#118DFF")),
            ),
            visual(
                "scatter",
                "scatterChart",
                (850.0, 330.0, 410.0, 300.0),
                vec![
                    ("Details", col("Region")),
                    ("X", measure("Margin")),
                    ("Y", measure("Total Sales")),
                ],
                data_point_fill(measure_expr("Status Color")),
            ),
        ],
    )
}

fn render_page(env: &QueryEnv, page: &Page, out: &Path) {
    let report = report(page);
    let rendered = render_report_page(
        env,
        &report,
        page,
        &ThemeResolver::default(),
        &InteractionState::new(),
        &PageRenderOptions::default(),
    );
    for (name, result) in &rendered.data {
        match result {
            Ok(dv) => {
                for w in &dv.warnings {
                    eprintln!("{name}: {}", w.message);
                }
            }
            Err(e) => eprintln!("{name}: {e:?}"),
        }
    }
    let scene = &rendered.scene;
    std::fs::write(
        out.join(format!("{}.svg", page.name)),
        to_svg(scene, &SvgOptions::default()),
    )
    .unwrap();
    let png = render_png(
        scene,
        &PngOptions {
            scale: 2.0,
            background: Some(Color::WHITE),
        },
    )
    .unwrap();
    let path = out.join(format!("{}.png", page.name));
    std::fs::write(&path, png).unwrap();
    println!("wrote {}", path.display());
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut out = PathBuf::from("target/tmp/cond-format");
    while let Some(a) = args.next() {
        if a == "--out" {
            out = args.next().map(PathBuf::from).expect("--out needs a value");
        }
    }
    std::fs::create_dir_all(&out).unwrap();
    let env = env();
    for page in [tables_page(), formats_page(), charts_page()] {
        render_page(&env, &page, &out);
    }
}
