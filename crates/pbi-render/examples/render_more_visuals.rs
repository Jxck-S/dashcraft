//! Dev tool: renders a synthetic dashboard with the scatter, funnel, gauge,
//! treemap and waterfall visuals, end to end (in-memory model → semantic
//! query → DataView → visuals → PNG/SVG), for eyeballing the renderers.
//!
//! ```text
//! cargo run -p pbi-render --example render_more_visuals -- [--out target/tmp/more-visuals]
//! ```
//! Writes `more.png` (defaults) and `options.png` (formatting options on)
//! at 2×, with their SVGs. Output stays under `target/` (never commit
//! renders).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, Int64Array, StringArray};
use arrow::record_batch::RecordBatch;
use pbi_engine::Engine;
use pbi_model::{
    Column, DataType, FieldExpr, Measure, Model, Page, Position, Projection, Report, SortDirection,
    SortItem, Table, Visual,
};
use pbi_render::png::{PngOptions, render_png};
use pbi_render::query::{InteractionState, QueryEnv, QueryLimits};
use pbi_render::scene::Color;
use pbi_render::svg::{SvgOptions, to_svg};
use pbi_render::theme::ThemeResolver;
use pbi_render::visuals::PageRenderOptions;
use pbi_render::visuals::page::{PageViewState, query_page_data, render_interactive_page_cached};
use serde_json::json;

const ROWS: usize = 6_000;
const REGIONS: [&str; 6] = ["North", "South", "East", "West", "Central", "Online"];
const STAGES: [&str; 5] = ["Lead", "Qualify", "Solution", "Proposal", "Finalize"];

struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n.max(1)
    }
}

fn model() -> Model {
    let mut sales = Table::new("Sales");
    for (name, dt) in [
        ("Category", DataType::String),
        ("Region", DataType::String),
        ("Quarter", DataType::String),
        ("Stage", DataType::String),
        ("Quantity", DataType::Int64),
        ("Amount", DataType::Double),
    ] {
        sales.columns.push(Column::data(name, dt));
    }
    for (name, expr, format) in [
        ("Total Amount", "SUM(Sales[Amount])", "#,0.00"),
        ("Orders", "COUNTROWS(Sales)", "#,0"),
        ("Avg Amount", "AVERAGE(Sales[Amount])", "#,0.00"),
        ("Units", "SUM(Sales[Quantity])", "#,0"),
        ("Target Amount", "4500", "#,0.00"),
        ("Variance", "SUM(Sales[Amount]) - 5000000", "#,0"),
    ] {
        let mut m = Measure::new(name, expr);
        m.format_string = Some(format.to_owned());
        sales.measures.push(m);
    }
    Model {
        tables: vec![sales],
        ..Model::default()
    }
}

fn batch() -> RecordBatch {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (mut cat, mut reg, mut qtr, mut stage, mut qty, mut amt) =
        (vec![], vec![], vec![], vec![], vec![], vec![]);
    for _ in 0..ROWS {
        let c = rng.below(9);
        let r = rng.below(6) as usize;
        let q = rng.below(4);
        cat.push(format!("Category {c:02}"));
        reg.push(REGIONS[r]);
        qtr.push(format!("2025 Q{}", q + 1));
        // A funnel: each later stage is rarer.
        let s = (0..4).take_while(|_| rng.below(100) < 55).count();
        stage.push(STAGES[s]);
        qty.push(rng.below(20) as i64 + 1 + (c as i64) * 3);
        let bias =
            1.0 + r as f64 * 0.15 + q as f64 * 0.1 * if r.is_multiple_of(2) { 1.0 } else { -1.0 };
        amt.push(rng.below(800_000) as f64 / 100.0 * bias + c as f64 * 150.0);
    }
    let columns: [(&str, ArrayRef); 6] = [
        ("Category", Arc::new(StringArray::from(cat))),
        ("Region", Arc::new(StringArray::from(reg))),
        ("Quarter", Arc::new(StringArray::from(qtr))),
        ("Stage", Arc::new(StringArray::from(stage))),
        ("Quantity", Arc::new(Int64Array::from(qty))),
        ("Amount", Arc::new(Float64Array::from(amt))),
    ];
    RecordBatch::try_from_iter(columns).unwrap()
}

fn col(name: &str) -> FieldExpr {
    FieldExpr::column("Sales", name)
}

fn measure(name: &str) -> FieldExpr {
    FieldExpr::measure("Sales", name)
}

fn visual(
    name: &str,
    visual_type: &str,
    (x, y, width, height): (f64, f64, f64, f64),
    fields: Vec<(&str, FieldExpr)>,
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
        ..Visual::default()
    }
}

fn page() -> Page {
    let mut stages = visual(
        "funnel",
        "funnel",
        (440.0, 10.0, 400.0, 340.0),
        vec![("Category", col("Stage")), ("Y", measure("Orders"))],
    );
    stages.sort = vec![SortItem {
        field: measure("Orders"),
        direction: SortDirection::Descending,
    }];
    let mut gauge = visual(
        "gauge",
        "gauge",
        (850.0, 10.0, 420.0, 340.0),
        vec![
            ("Y", measure("Avg Amount")),
            ("TargetValue", measure("Target Amount")),
        ],
    );
    gauge.objects =
        json!({"axis": [{"properties": {"min": {"expr": {"Literal": {"Value": "0D"}}}}}]});
    let mut waterfall_breakdown = visual(
        "waterfallBreakdown",
        "waterfallChart",
        (850.0, 360.0, 420.0, 350.0),
        vec![
            ("Category", col("Quarter")),
            ("Breakdown", col("Region")),
            ("Y", measure("Total Amount")),
        ],
    );
    waterfall_breakdown.objects = json!({"breakdown": [{"properties": {"maxBreakdowns": {"expr": {"Literal": {"Value": "3L"}}}}}]});
    let mut scatter = visual(
        "scatter",
        "scatterChart",
        (10.0, 10.0, 420.0, 340.0),
        vec![
            ("Category", col("Category")),
            ("Series", col("Region")),
            ("X", measure("Units")),
            ("Y", measure("Avg Amount")),
            ("Size", measure("Orders")),
        ],
    );
    scatter.objects = json!({"categoryLabels": [{"properties": {"show": {"expr": {"Literal": {"Value": "false"}}}}}]});
    Page {
        name: "more".into(),
        display_name: "More visuals".into(),
        width: 1280.0,
        height: 720.0,
        visuals: vec![
            scatter,
            stages,
            gauge,
            visual(
                "treemap",
                "treemap",
                (10.0, 360.0, 420.0, 350.0),
                vec![
                    ("Group", col("Category")),
                    ("Details", col("Region")),
                    ("Values", measure("Total Amount")),
                ],
            ),
            visual(
                "waterfall",
                "waterfallChart",
                (440.0, 360.0, 400.0, 350.0),
                vec![("Category", col("Region")), ("Y", measure("Variance"))],
            ),
            waterfall_breakdown,
        ],
        ..Page::default()
    }
}

fn lit(raw: &str) -> serde_json::Value {
    json!({"expr": {"Literal": {"Value": raw}}})
}

/// The same visuals with formatting options on: labels, shapes, colors.
fn options_page() -> Page {
    let mut page = page();
    page.name = "options".into();
    for v in &mut page.visuals {
        v.objects = match v.name.as_str() {
            "scatter" => json!({
                "categoryLabels": [{"properties": {"show": lit("true")}}],
                "bubbles": [{"properties": {"markerShape": lit("'diamond'"), "bubbleSize": lit("-5L")}}],
                "legend": [{"properties": {"position": lit("'Bottom'")}}]
            }),
            "funnel" => json!({
                "labels": [{"properties": {"funnelLabelStyle": lit("'Data value, percent of previous'")}}],
                "dataPoint": [{"properties": {"defaultColor": {"solid": {"color": lit("'#01B8AA'")}}}}]
            }),
            "gauge" => json!({
                "axis": [{"properties": {"min": lit("3000D"), "max": lit("9000D")}}],
                "dataPoint": [{"properties": {
                    "fill": {"solid": {"color": lit("'#365FDC'")}},
                    "target": {"solid": {"color": lit("'#333333'")}}
                }}]
            }),
            "treemap" => json!({"labels": [{"properties": {"show": lit("true")}}]}),
            "waterfall" => json!({
                "labels": [{"properties": {"show": lit("true")}}],
                "sentimentColors": [{"properties": {
                    "increaseFill": {"solid": {"color": lit("'#3BB44A'")}},
                    "decreaseFill": {"solid": {"color": lit("'#E81123'")}},
                    "totalFill": {"solid": {"color": lit("'#01B8AA'")}}
                }}]
            }),
            _ => v.objects.clone(),
        };
    }
    page
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut out = PathBuf::from("target/tmp/more-visuals");
    while let Some(a) = args.next() {
        if a == "--out" {
            out = args.next().map(PathBuf::from).expect("--out needs a value");
        }
    }
    std::fs::create_dir_all(&out).unwrap();
    let model = model();
    let mut engine = Engine::new(&model);
    engine.load_tables([("Sales".to_owned(), batch())]).unwrap();
    let env = QueryEnv::new(engine.snapshot(), Arc::new(model), "en-US");
    for page in [page(), options_page()] {
        render_page(&env, &page, &out);
    }
}

fn render_page(env: &QueryEnv, page: &Page, out: &std::path::Path) {
    let report = Report {
        name: "synthetic".into(),
        pages: vec![page.clone()],
        ..Report::default()
    };
    let state = InteractionState::new();
    let data = query_page_data(env, &report, page, &state, &QueryLimits::default());
    for (name, result) in &data.data {
        if let Err(e) = result {
            eprintln!("{name}: {e:?}");
        }
    }
    let theme = ThemeResolver::default();
    let rendered = render_interactive_page_cached(
        &report,
        page,
        &theme,
        &state,
        &PageRenderOptions::default(),
        &PageViewState::new(),
        &data,
        None,
    );
    let scene = &rendered.page.scene;
    let svg_path = out.join(format!("{}.svg", page.name));
    let png_path = out.join(format!("{}.png", page.name));
    std::fs::write(&svg_path, to_svg(scene, &SvgOptions::default())).unwrap();
    let png = render_png(
        scene,
        &PngOptions {
            scale: 2.0,
            background: Some(Color::WHITE),
        },
    )
    .unwrap();
    std::fs::write(&png_path, png).unwrap();
    for v in &rendered.page.visuals {
        println!(
            "{:<20} {:<16} query {:>6} µs  render {:>6} µs",
            v.name, v.visual_type, v.query_micros, v.render_micros
        );
    }
    println!(
        "wrote {} ({} hit regions)",
        png_path.display(),
        scene.hit_regions().len()
    );
}
