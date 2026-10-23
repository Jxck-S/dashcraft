//! Performance scenarios of the renderer (`cargo xtask perf`; budgets in `perf/budgets.toml`,
//! docs in `docs/perf.md`):
//!
//! * P01 table scroll step: re-render + SVG of a page with a 50 000-row × 8-column synthetic
//!   table, scrolled 40 px per step (warm caches, as in the app).
//! * P02 first render of that page in a fresh process: query + render + SVG (cold caches).
//! * P03 render of the first page of the committed Acme Outfitters example (report
//!   Overview, page Executive Summary) without data (layout, empty visuals, placeholders):
//!   query + render + SVG.
//! * P07 SVG serialization of a busy synthetic page (cards, slicer, charts, matrix, table).
//! * P08 scroll step of the table on the example's Online / Customer Detail page, with the
//!   example refreshed offline from its CSVs (`blob-map.json`).
//!
//! ```text
//! cargo run --release -p pbi-render --example perf_render -- --scenario P01 --iterations 40
//! ```

#[path = "../../../perf/harness.rs"]
mod harness;

use std::hint::black_box;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use arrow::array::{ArrayRef, Float64Array, Int64Array, StringArray, TimestampMicrosecondArray};
use arrow::record_batch::RecordBatch;
use harness::{Args, Outcome, ms_since, sample};
use pbi_core::ReportSource;
use pbi_engine::{Engine, Value};
use pbi_model::{
    AggregationFunction, Column, DataType, FieldExpr, Measure, Model, Page, Position, Projection,
    Report, Table, Visual,
};
use pbi_render::query::{InteractionState, QueryEnv, QueryLimits};
use pbi_render::svg::{SvgOptions, to_svg};
use pbi_render::text::FontBook;
use pbi_render::theme::ThemeResolver;
use pbi_render::visuals::PageRenderOptions;
use pbi_render::visuals::page::{
    PageData, PageViewState, query_page_data, render_interactive_page,
};
use pbi_session::{RefreshOptions, Session, SessionOptions};

fn main() -> ExitCode {
    harness::main(
        env!("CARGO_MANIFEST_DIR"),
        &[
            ("P01", table_scroll_step),
            ("P02", table_page_cold),
            ("P03", acme_page_without_data),
            ("P07", busy_page_svg),
            ("P08", acme_table_scroll_step),
        ],
    )
}

/// Rows of the synthetic `Sales` table.
const ROWS: usize = 50_000;
/// The synthetic table visual.
const TABLE: &str = "salesTable";
/// Scroll distance per step (about two rows, like a wheel notch).
const SCROLL_STEP_PX: f64 = 40.0;

fn table_scroll_step(args: &Args) -> Result<Outcome, String> {
    let fx = Synthetic::new()?;
    let page = fx.page("table")?;
    let data = fx.query(page);
    let out = scroll_steps(&fx.report, page, &fx.theme, TABLE, &data, args)?;
    Ok(out.with("rows", ROWS).with("columns", 8usize))
}

fn table_page_cold(_args: &Args) -> Result<Outcome, String> {
    let fx = Synthetic::new()?;
    let page = fx.page("table")?;
    // Font loading is a once-per-process cost outside the page render.
    let _ = FontBook::shared();
    let opts = PageRenderOptions::default();
    let state = InteractionState::from_page(page).0;
    let view = PageViewState::new();
    let t = Instant::now();
    let data = query_page_data(&fx.env, &fx.report, page, &state, &opts.limits);
    let query_ms = ms_since(t);
    let t2 = Instant::now();
    let out = render_interactive_page(&fx.report, page, &fx.theme, &state, &opts, &view, &data);
    let render_ms = ms_since(t2);
    let t3 = Instant::now();
    let svg = to_svg(&out.page.scene, &SvgOptions::default());
    let svg_ms = ms_since(t3);
    let total = ms_since(t);
    let truncated = out.page.visual(TABLE).is_some_and(|v| v.truncated);
    Ok(Outcome::ok(vec![total])
        .with("rows", ROWS)
        .with("query_ms", query_ms)
        .with("render_ms", render_ms)
        .with("svg_ms", svg_ms)
        .with("svg_kb", black_box(svg.len()) / 1024)
        .with("truncated_at_query_limit", usize::from(truncated)))
}

/// The example, relative to the fixture root.
const ACME: &str = "examples/acme-outfitters";

fn acme_page_without_data(args: &Args) -> Result<Outcome, String> {
    let pbip = args.reference.join(ACME).join("AcmeOutfitters.pbip");
    if !pbip.exists() {
        return Ok(Outcome::missing(&pbip));
    }
    let session = Session::open(&pbip, SessionOptions::default()).map_err(|e| e.to_string())?;
    let env = QueryEnv::from_session(&session);
    let (report, theme) = report_and_theme(&session, "Acme Outfitters - Overview")?;
    let page = report.pages.first().ok_or("the report has no pages")?;
    let state = InteractionState::from_page(page).0;
    let opts = PageRenderOptions::default();
    let view = PageViewState::new();
    let mut svg_len = 0;
    let samples = sample(args.warmup, args.iterations, |_| {
        let data = query_page_data(&env, report, page, &state, &opts.limits);
        let out = render_interactive_page(report, page, &theme, &state, &opts, &view, &data);
        svg_len = black_box(to_svg(&out.page.scene, &SvgOptions::default())).len();
        Ok(())
    })?;
    Ok(Outcome::ok(samples)
        .with("visuals", page.visuals.len())
        .with("svg_kb", svg_len / 1024))
}

fn busy_page_svg(args: &Args) -> Result<Outcome, String> {
    let fx = Synthetic::new()?;
    let page = fx.page("overview")?;
    let data = fx.query(page);
    let state = InteractionState::from_page(page).0;
    let out = render_interactive_page(
        &fx.report,
        page,
        &fx.theme,
        &state,
        &PageRenderOptions::default(),
        &PageViewState::new(),
        &data,
    );
    if let Some(v) = out.page.visuals.iter().find(|v| v.error.is_some()) {
        return Err(format!("visual `{}` failed: {:?}", v.name, v.error));
    }
    let scene = &out.page.scene;
    let mut svg_len = 0;
    let samples = sample(args.warmup, args.iterations, |_| {
        svg_len = black_box(to_svg(scene, &SvgOptions::default())).len();
        Ok(())
    })?;
    Ok(Outcome::ok(samples)
        .with("visuals", page.visuals.len())
        .with("hit_regions", scene.hit_regions().len())
        .with("svg_kb", svg_len / 1024))
}

fn acme_table_scroll_step(args: &Args) -> Result<Outcome, String> {
    let dir = args.reference.join(ACME);
    let (pbip, blob_map) = (dir.join("AcmeOutfitters.pbip"), dir.join("blob-map.json"));
    for path in [&pbip, &blob_map] {
        if !path.exists() {
            return Ok(Outcome::missing(path));
        }
    }
    let session = Session::open(
        &pbip,
        SessionOptions {
            blob_map: Some(blob_map),
            ..SessionOptions::default()
        },
    )
    .map_err(|e| e.to_string())?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(session.refresh(RefreshOptions::all()))
        .map_err(|e| e.to_string())?;
    let env = QueryEnv::from_session(&session);
    let (report, theme) = report_and_theme(&session, "Acme Outfitters - Online")?;
    let page = report
        .pages
        .iter()
        .find(|p| p.display_name == "Customer Detail")
        .ok_or("no page Customer Detail")?;
    let table = page
        .visuals
        .iter()
        .find(|v| v.visual_type == "tableEx")
        .ok_or("no tableEx on Customer Detail")?;
    let state = InteractionState::from_page(page).0;
    let data = query_page_data(&env, report, page, &state, &QueryLimits::default());
    let rows = data
        .get(&table.name)
        .and_then(|r| r.as_ref().ok())
        .and_then(|dv| dv.table.as_ref())
        .map_or(0, |t| t.rows.len());
    let out = scroll_steps(report, page, &theme, &table.name, &data, args)?;
    Ok(out.with("rows", rows).with("visuals", page.visuals.len()))
}

/// The report named `name` and its theme.
fn report_and_theme<'a>(
    session: &'a Session,
    name: &str,
) -> Result<(&'a Report, ThemeResolver), String> {
    let doc = session.document();
    let index = doc
        .reports
        .iter()
        .position(|r| r.name == name)
        .ok_or_else(|| format!("no report `{name}`"))?;
    let report = &doc.reports[index];
    let theme = match session.opened().report_sources.get(index) {
        Some(ReportSource::Pbir { report: pbir, .. }) => ThemeResolver::from_pbir(pbir),
        _ => ThemeResolver::default(),
    };
    Ok((report, theme))
}

/// Re-renders `page` (render + SVG, as the app does per scroll event) with the table `table`
/// scrolled one more step each iteration.
fn scroll_steps(
    report: &Report,
    page: &Page,
    theme: &ThemeResolver,
    table: &str,
    data: &PageData,
    args: &Args,
) -> Result<Outcome, String> {
    let state = InteractionState::from_page(page).0;
    let opts = PageRenderOptions::default();
    let mut view = PageViewState::new();
    let first = render_interactive_page(report, page, theme, &state, &opts, &view, data);
    let max_y = first
        .interaction(table)
        .and_then(|i| i.scroll.as_ref())
        .map(|s| s.max_y)
        .ok_or("the table does not scroll")?;
    let (mut render_ms, mut svg_ms, mut svg_len) = (Vec::new(), Vec::new(), 0);
    let samples = sample(args.warmup, args.iterations, |i| {
        view.visual_mut(table).table.scroll.y = ((i + 1) as f64 * SCROLL_STEP_PX).min(max_y);
        let t = Instant::now();
        let out = render_interactive_page(report, page, theme, &state, &opts, &view, data);
        render_ms.push(ms_since(t));
        let t = Instant::now();
        svg_len = black_box(to_svg(&out.page.scene, &SvgOptions::default())).len();
        svg_ms.push(ms_since(t));
        Ok(())
    })?;
    Ok(Outcome::ok(samples)
        .with("render_median_ms", median(&mut render_ms))
        .with("svg_median_ms", median(&mut svg_ms))
        .with("svg_kb", svg_len / 1024))
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(f64::NAN)
}

/// The synthetic model, data and report: `Sales` (50 000 rows × 8 columns, 4 measures);
/// page `table` holds one full-page table of every column, page `overview` a dashboard.
struct Synthetic {
    env: QueryEnv,
    report: Report,
    theme: ThemeResolver,
}

impl Synthetic {
    fn new() -> Result<Self, String> {
        let model = sales_model();
        let mut engine = Engine::new(&model);
        engine
            .load_tables([("Sales".to_owned(), sales_batch(ROWS)?)])
            .map_err(|e| e.to_string())?;
        Ok(Self {
            env: QueryEnv::new(engine.snapshot(), Arc::new(model), "en-US"),
            report: sales_report(),
            theme: ThemeResolver::default(),
        })
    }

    fn page(&self, name: &str) -> Result<&Page, String> {
        self.report
            .pages
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| format!("no page `{name}`"))
    }

    fn query(&self, page: &Page) -> PageData {
        let state = InteractionState::from_page(page).0;
        query_page_data(
            &self.env,
            &self.report,
            page,
            &state,
            &QueryLimits::default(),
        )
    }
}

fn sales_model() -> Model {
    let mut sales = Table::new("Sales");
    for (name, dt) in [
        ("Order ID", DataType::Int64),
        ("Customer", DataType::String),
        ("Product", DataType::String),
        ("Category", DataType::String),
        ("Region", DataType::String),
        ("Order Date", DataType::DateTime),
        ("Quantity", DataType::Int64),
        ("Amount", DataType::Double),
    ] {
        sales.columns.push(Column::data(name, dt));
    }
    for (name, expr, format) in [
        ("Total Amount", "SUM(Sales[Amount])", "#,0.00"),
        ("Orders", "COUNTROWS(Sales)", "#,0"),
        ("Avg Amount", "AVERAGE(Sales[Amount])", "#,0.00"),
        ("Customers", "DISTINCTCOUNT(Sales[Customer])", "#,0"),
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

/// Deterministic xorshift64 generator.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n.max(1)
    }
}

const REGIONS: [&str; 6] = ["North", "South", "East", "West", "Central", "Online"];

fn sales_batch(rows: usize) -> Result<RecordBatch, String> {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let day0 = match Value::date(2024, 1, 1) {
        Some(Value::DateTime(us)) => us,
        _ => return Err("cannot build the start date".to_owned()),
    };
    const DAY_US: i64 = 86_400_000_000;
    let mut ids = Vec::with_capacity(rows);
    let mut customers = Vec::with_capacity(rows);
    let mut products = Vec::with_capacity(rows);
    let mut categories = Vec::with_capacity(rows);
    let mut regions = Vec::with_capacity(rows);
    let mut dates = Vec::with_capacity(rows);
    let mut quantities = Vec::with_capacity(rows);
    let mut amounts = Vec::with_capacity(rows);
    for i in 0..rows {
        let product = rng.below(200);
        ids.push(i64::try_from(i).unwrap_or(i64::MAX) + 1);
        customers.push(format!("Customer {:05}", rng.below(5_000)));
        products.push(format!("Product {product:03}"));
        categories.push(format!("Category {:02}", product % 12));
        regions.push(REGIONS[usize::try_from(rng.below(6)).unwrap_or(0) % REGIONS.len()]);
        dates.push(day0 + i64::try_from(rng.below(730)).unwrap_or(0) * DAY_US);
        quantities.push(i64::try_from(rng.below(100)).unwrap_or(0) + 1);
        amounts.push(rng.below(1_000_000) as f64 / 100.0);
    }
    let columns: [(&str, ArrayRef); 8] = [
        ("Order ID", Arc::new(Int64Array::from(ids))),
        ("Customer", Arc::new(StringArray::from(customers))),
        ("Product", Arc::new(StringArray::from(products))),
        ("Category", Arc::new(StringArray::from(categories))),
        ("Region", Arc::new(StringArray::from(regions))),
        (
            "Order Date",
            Arc::new(TimestampMicrosecondArray::from(dates)),
        ),
        ("Quantity", Arc::new(Int64Array::from(quantities))),
        ("Amount", Arc::new(Float64Array::from(amounts))),
    ];
    RecordBatch::try_from_iter(columns).map_err(|e| e.to_string())
}

fn col(name: &str) -> FieldExpr {
    FieldExpr::column("Sales", name)
}

fn sum(name: &str) -> FieldExpr {
    FieldExpr::aggregation(col(name), AggregationFunction::Sum)
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

fn sales_report() -> Report {
    let table = visual(
        TABLE,
        "tableEx",
        (10.0, 10.0, 1260.0, 700.0),
        vec![
            ("Values", col("Order ID")),
            ("Values", col("Customer")),
            ("Values", col("Product")),
            ("Values", col("Category")),
            ("Values", col("Region")),
            ("Values", col("Order Date")),
            ("Values", sum("Quantity")),
            ("Values", sum("Amount")),
        ],
    );
    let card = |name: &str, x: f64, m: &str| {
        visual(
            name,
            "card",
            (x, 10.0, 305.0, 90.0),
            vec![("Values", measure(m))],
        )
    };
    let overview = vec![
        card("cardAmount", 10.0, "Total Amount"),
        card("cardOrders", 325.0, "Orders"),
        card("cardAvg", 640.0, "Avg Amount"),
        card("cardCustomers", 955.0, "Customers"),
        visual(
            "regionSlicer",
            "slicer",
            (10.0, 110.0, 200.0, 290.0),
            vec![("Values", col("Region"))],
        ),
        visual(
            "categoryColumns",
            "clusteredColumnChart",
            (220.0, 110.0, 520.0, 290.0),
            vec![
                ("Category", col("Category")),
                ("Y", measure("Total Amount")),
            ],
        ),
        visual(
            "dailyLine",
            "lineChart",
            (750.0, 110.0, 520.0, 290.0),
            vec![
                ("Category", col("Order Date")),
                ("Y", measure("Total Amount")),
            ],
        ),
        visual(
            "regionPie",
            "pieChart",
            (10.0, 410.0, 300.0, 300.0),
            vec![("Category", col("Region")), ("Y", measure("Total Amount"))],
        ),
        visual(
            "categoryMatrix",
            "pivotTable",
            (320.0, 410.0, 470.0, 300.0),
            vec![
                ("Rows", col("Category")),
                ("Rows", col("Product")),
                ("Columns", col("Region")),
                ("Values", measure("Total Amount")),
            ],
        ),
        visual(
            "customerTable",
            "tableEx",
            (800.0, 410.0, 470.0, 300.0),
            vec![
                ("Values", col("Customer")),
                ("Values", measure("Orders")),
                ("Values", measure("Total Amount")),
            ],
        ),
    ];
    Report {
        name: "Synthetic sales".into(),
        pages: vec![
            Page {
                name: "table".into(),
                display_name: "Table".into(),
                visuals: vec![table],
                ..Page::default()
            },
            Page {
                name: "overview".into(),
                display_name: "Overview".into(),
                visuals: overview,
                ..Page::default()
            },
        ],
        ..Report::default()
    }
}
