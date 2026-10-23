//! Dev benchmark: interactive re-render cost when scrolling a table visual
//! and after a slicer selection, with and without the visual cache.
//!
//! ```text
//! cargo run -p pbi-render --example bench_scroll -- <file.pbip> --blob-map <map.json> \
//!     [--report <substring>] [--page <display name>] [--steps 20]
//!
//! E=reference/examples/acme-outfitters
//! cargo run --release -p pbi-render --example bench_scroll -- $E/AcmeOutfitters.pbip \
//!     --blob-map $E/blob-map.json --report Online --page "Customer Detail"
//! ```
//! Prints per-step render and SVG times for the first `tableEx` on the page
//! (default: the first page; uncached, then through a fresh
//! [`VisualCache`]), and the render time after selecting the first value of
//! the first slicer (the visuals whose data did not change are reused). No
//! data is written anywhere.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::time::Instant;

use pbi_core::ReportSource;
use pbi_render::layout::FrameOptions;
use pbi_render::query::{InteractionState, QueryEnv, QueryLimits, SlicerSelection};
use pbi_render::svg::{SvgOptions, to_svg};
use pbi_render::theme::ThemeResolver;
use pbi_render::visuals::page::{PageViewState, query_page_data};
use pbi_render::visuals::{PageRenderOptions, VisualCache, render_interactive_page_cached};
use pbi_session::{RefreshOptions, Session, SessionOptions};

fn main() {
    let mut args = std::env::args().skip(1);
    let mut path: Option<PathBuf> = None;
    let mut blob_map: Option<PathBuf> = None;
    let mut report_filter: Option<String> = None;
    let mut page_name: Option<String> = None;
    let mut steps = 20usize;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--blob-map" => blob_map = args.next().map(PathBuf::from),
            "--report" => report_filter = args.next(),
            "--page" => page_name = args.next(),
            "--steps" => steps = args.next().and_then(|s| s.parse().ok()).unwrap_or(20),
            _ => path = Some(PathBuf::from(a)),
        }
    }
    let session = Session::open(
        path.expect("usage: bench_scroll <file> --blob-map <map>"),
        SessionOptions {
            blob_map,
            ..SessionOptions::default()
        },
    )
    .unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(session.refresh(RefreshOptions::all())).unwrap();
    let env = QueryEnv::from_session(&session);
    let doc = session.document();
    let (ri, report) = doc
        .reports
        .iter()
        .enumerate()
        .find(|(_, r)| {
            report_filter
                .as_ref()
                .is_none_or(|f| r.name.to_lowercase().contains(&f.to_lowercase()))
        })
        .unwrap();
    let theme = match session.opened().report_sources.get(ri) {
        Some(ReportSource::Pbir { report: pbir, .. }) => ThemeResolver::from_pbir(pbir),
        _ => ThemeResolver::default(),
    };
    let page = match &page_name {
        Some(name) => report
            .pages
            .iter()
            .find(|p| &p.display_name == name)
            .expect("no page with that display name"),
        None => &report.pages[0],
    };
    let table = page
        .visuals
        .iter()
        .find(|v| v.visual_type == "tableEx")
        .expect("no tableEx on the page");
    let (state, _) = InteractionState::from_page(page);
    let t = Instant::now();
    let data = query_page_data(&env, report, page, &state, &QueryLimits::default());
    println!("query page: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    let opts = PageRenderOptions {
        frames: FrameOptions::default(),
        limits: QueryLimits::default(),
    };
    let ms = |t: Instant| t.elapsed().as_secs_f64() * 1e3;
    let scroll = |cache: Option<&VisualCache>, label: &str| {
        let mut view = PageViewState::new();
        let (mut total_render, mut total_svg, mut svg_len) = (0.0, 0.0, 0);
        for i in 0..steps {
            view.visual_mut(&table.name).table.scroll.y = (i + 1) as f64 * 40.0;
            let t = Instant::now();
            let out = render_interactive_page_cached(
                report, page, &theme, &state, &opts, &view, &data, cache,
            );
            let r = ms(t);
            let t = Instant::now();
            let svg = to_svg(&out.page.scene, &SvgOptions::default());
            let s = ms(t);
            svg_len = svg.len();
            total_render += r;
            total_svg += s;
            if i < 2 {
                println!(
                    "{label} step {i}: render {r:.2} ms ({} reused), svg {s:.2} ms",
                    out.reused.len()
                );
            }
        }
        println!(
            "{label} avg over {steps}: render {:.2} ms, svg {:.2} ms, svg {} KB",
            total_render / steps as f64,
            total_svg / steps as f64,
            svg_len / 1024
        );
    };
    scroll(None, "uncached");
    let cache = VisualCache::new();
    let view = PageViewState::new();
    let t = Instant::now();
    render_interactive_page_cached(
        report,
        page,
        &theme,
        &state,
        &opts,
        &view,
        &data,
        Some(&cache),
    );
    println!("cached cold page render: {:.2} ms", ms(t));
    let t = Instant::now();
    let out = render_interactive_page_cached(
        report,
        page,
        &theme,
        &state,
        &opts,
        &view,
        &data,
        Some(&cache),
    );
    println!(
        "cached unchanged re-render: {:.2} ms ({} reused)",
        ms(t),
        out.reused.len()
    );
    scroll(Some(&cache), "cached");

    // A slicer selection: re-query, then re-render with and without the
    // (warm) cache.
    let Some((slicer, value)) = page
        .visuals
        .iter()
        .filter(|v| v.visual_type == "slicer")
        .find_map(|v| {
            let dv = data.get(&v.name)?.as_ref().ok()?;
            let first = dv
                .categorical
                .as_ref()?
                .categories
                .first()?
                .values
                .first()?;
            Some((v.name.clone(), first.raw.clone()))
        })
    else {
        println!("no slicer with values on the page");
        return;
    };
    let mut selected = state.clone();
    selected.select(
        slicer.clone(),
        SlicerSelection {
            values: vec![vec![value]],
            ..SlicerSelection::default()
        },
    );
    let t = Instant::now();
    let filtered = query_page_data(&env, report, page, &selected, &QueryLimits::default());
    println!("slicer {slicer}: query {:.1} ms", ms(t));
    let t = Instant::now();
    render_interactive_page_cached(
        report, page, &theme, &selected, &opts, &view, &filtered, None,
    );
    println!("slicer change uncached: render {:.2} ms", ms(t));
    let t = Instant::now();
    let out = render_interactive_page_cached(
        report,
        page,
        &theme,
        &selected,
        &opts,
        &view,
        &filtered,
        Some(&cache),
    );
    println!(
        "slicer change cached: render {:.2} ms ({} of {} visuals reused)",
        ms(t),
        out.reused.len(),
        out.page.visuals.iter().filter(|v| !v.hidden).count()
    );
    println!("cache: {:?}", cache.stats());
}
