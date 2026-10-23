//! Golden-image regression tests: renders of the Acme Outfitters pages
//! compared region by region (SSIM, [`pbi_render::golden`]) with PNG exports
//! of the same pages made by Power BI, wherever such an export exists; the
//! comparison pipeline on renders compared with themselves; and the
//! interactive canvas path (visual cache) on real data.
//!
//! Goldens are optional local files (`reference/golden/` is gitignored):
//! `golden/<example>/<report>/<page>.png`, e.g.
//! `golden/acme-outfitters/Overview/Executive Summary.png`: the page as
//! Power BI exports it (any scale; the page at the top-left of the image,
//! padding allowed). A page without one prints `SKIP: <file> missing`.
//! Every region of a page with a golden must reach
//! [`DEFAULT_THRESHOLD`] (no per-page exceptions; AGENTS.md rule 11 says
//! how a lower threshold may be justified), and so must the whole page.
//!
//! Run locally with the scores printed:
//!
//! ```text
//! cargo test -p pbi-render --test golden -- --nocapture
//! ```
//!
//! Each comparison writes the render, the aligned golden, a diff heatmap
//! and the scores (`*.json`) to `target/tmp/golden/` (never commit them).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod acme_support;

use std::path::{Path, PathBuf};

use acme_support::{ACME_REPORTS, acme, page, reference_root, render, report_index, theme};
use pbi_model::{Page, Report};
use pbi_render::golden::{
    self, CompareOptions, DEFAULT_THRESHOLD, GoldenReport, diff_heatmap, resample,
};
use pbi_render::query::{InteractionState, QueryEnv};
use pbi_render::svg::{SvgOptions, to_svg};
use pbi_render::theme::ThemeResolver;
use pbi_render::visuals::{
    PageData, PageRenderOptions, PageViewState, RenderedPage, VisualCache, query_page_data,
    render_interactive_page_cached,
};

/// Output scale of the renders.
const SCALE: f32 = 2.0;
/// The example's folder name under `golden/`.
const EXAMPLE: &str = "acme-outfitters";

fn out_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp/golden")
}

/// `golden/<example>/<report>/<page>.png`.
fn golden_path(report: &str, page: &str) -> PathBuf {
    reference_root()
        .join("golden")
        .join(EXAMPLE)
        .join(report)
        .join(format!("{page}.png"))
}

/// Compares a render with `golden`; writes the artifacts under
/// `target/tmp/golden/<stem>-*` and prints the scores.
fn compare(
    stem: &str,
    report: &Report,
    page: &Page,
    theme: &ThemeResolver,
    rendered: &RenderedPage,
    golden: &image::RgbaImage,
) -> GoldenReport {
    let render = golden::rasterize(&rendered.scene, SCALE).unwrap();
    let regions = golden::page_regions(report, page, theme, f64::from(SCALE));
    let scores =
        golden::compare_regions(&render, golden, &regions, &CompareOptions::default()).unwrap();
    let path = out_dir().join(stem);
    let dir = path.parent().unwrap();
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let aligned = resample(golden, &scores.alignment, render.width(), render.height());
    golden::save_png(&render, &dir.join(format!("{name}-render.png"))).unwrap();
    golden::save_png(&aligned, &dir.join(format!("{name}-golden.png"))).unwrap();
    golden::save_png(
        &diff_heatmap(&render, &aligned, Some(&scores), DEFAULT_THRESHOLD),
        &dir.join(format!("{name}-diff.png")),
    )
    .unwrap();
    std::fs::write(
        dir.join(format!("{name}.json")),
        serde_json::to_string_pretty(&scores).unwrap(),
    )
    .unwrap();
    println!(
        "{stem}: render {}x{}, golden {}x{}\n{}",
        render.width(),
        render.height(),
        golden.width(),
        golden.height(),
        scores.table()
    );
    scores
}

/// Every region and the whole page reach [`DEFAULT_THRESHOLD`]; lists all
/// failures.
fn assert_regions_pass(name: &str, report: &GoldenReport) {
    assert!(!report.regions.is_empty(), "{name}: no regions");
    let failures: Vec<String> = report
        .below(DEFAULT_THRESHOLD)
        .iter()
        .map(|r| format!("{} ssim {:.4} < {DEFAULT_THRESHOLD}", r.label(), r.ssim))
        .collect();
    assert!(
        failures.is_empty(),
        "{name}: regions below the SSIM threshold:\n{}",
        failures.join("\n")
    );
    assert!(
        report.overall >= DEFAULT_THRESHOLD,
        "{name}: page SSIM {:.4}",
        report.overall
    );
}

/// Every Acme Outfitters page against its golden, where one exists.
#[test]
fn acme_pages_match_their_goldens() {
    let Some(session) = acme() else { return };
    let (mut compared, mut skipped) = (0, 0);
    for short in ACME_REPORTS {
        let index = report_index(session, short);
        let report = &session.document().reports[index];
        let theme = theme(session, index);
        for page in &report.pages {
            let path = golden_path(short, &page.display_name);
            if !path.exists() {
                println!("SKIP: {} missing", path.display());
                skipped += 1;
                continue;
            }
            let golden = golden::load_png(&path).unwrap();
            let rendered = render(session, index, page);
            let failed: Vec<_> = rendered
                .visuals
                .iter()
                .filter_map(|v| v.error.as_ref().map(|e| format!("{}: {e}", v.name)))
                .collect();
            assert!(failed.is_empty(), "visual errors: {failed:#?}");
            let stem = format!("{EXAMPLE}/{short}/{}", page.display_name);
            let scores = compare(&stem, report, page, &theme, &rendered, &golden);
            assert_regions_pass(&stem, &scores);
            compared += 1;
        }
    }
    println!("goldens: {compared} compared, {skipped} missing");
}

/// The pipeline on a real page without goldens: a render compared with
/// itself (and with itself inside export padding, at another scale) scores
/// 1 everywhere, at the identity alignment.
#[test]
fn a_render_matches_itself_through_the_golden_pipeline() {
    let Some(session) = acme() else { return };
    let index = report_index(session, "Overview");
    let report = &session.document().reports[index];
    let theme = theme(session, index);
    let page = page(report, "Executive Summary");
    let rendered = render(session, index, page);
    let render = golden::rasterize(&rendered.scene, SCALE).unwrap();
    assert_eq!(render.dimensions(), (2560, 1440));
    let regions = golden::page_regions(report, page, &theme, f64::from(SCALE));
    assert_eq!(regions.len(), page.visuals.len());

    // Through a PNG file, as a golden would be read.
    let dir = out_dir().join("self");
    let path = dir.join("executive-summary.png");
    golden::save_png(&render, &path).unwrap();
    let golden = golden::load_png(&path).unwrap();
    let scores =
        golden::compare_regions(&render, &golden, &regions, &CompareOptions::default()).unwrap();
    assert_eq!(
        (
            scores.alignment.scale,
            scores.alignment.offset_x,
            scores.alignment.offset_y
        ),
        (1.0, 0.0, 0.0)
    );
    assert_eq!(scores.regions.len(), regions.len());
    assert!(
        scores.regions.iter().all(|r| r.ssim > 0.9999),
        "{}",
        scores.table()
    );
    assert_regions_pass("self", &scores);

    // A 1× render inside 20px of bottom padding (like an export): the
    // alignment finds scale 2 at the origin and the regions still match.
    let small = golden::rasterize(&rendered.scene, 1.0).unwrap();
    let mut padded = image::RgbaImage::from_pixel(1280, 740, image::Rgba([255, 255, 255, 255]));
    image::imageops::overlay(&mut padded, &small, 0, 0);
    let scores =
        golden::compare_regions(&render, &padded, &regions, &CompareOptions::default()).unwrap();
    assert_eq!(
        (
            scores.alignment.scale,
            scores.alignment.offset_x,
            scores.alignment.offset_y
        ),
        (0.5, 0.0, 0.0)
    );
    assert_regions_pass("self, padded 1x", &scores);
    let heatmap = diff_heatmap(&render, &render, Some(&scores), DEFAULT_THRESHOLD);
    assert_eq!(heatmap.dimensions(), render.dimensions());
}

/// The interactive canvas path on real data: renders through the visual
/// cache are byte-identical to uncached renders, scrolling the table
/// re-renders only the table, and a slicer selection re-renders exactly the
/// slicer and the visuals whose data changed.
#[test]
fn pages_render_identically_through_the_visual_cache() {
    let Some(session) = acme() else { return };
    let index = report_index(session, "Overview");
    let report = &session.document().reports[index];
    let theme = theme(session, index);
    let page = page(report, "Product Detail");
    let env = QueryEnv::from_session(session);
    let opts = PageRenderOptions::default();
    let cache = VisualCache::new();
    let render = |state: &InteractionState, view: &PageViewState, data: &PageData| {
        let cached = render_interactive_page_cached(
            report,
            page,
            &theme,
            state,
            &opts,
            view,
            data,
            Some(&cache),
        );
        let fresh =
            render_interactive_page_cached(report, page, &theme, state, &opts, view, data, None);
        assert_eq!(
            to_svg(&cached.page.scene, &SvgOptions::default()),
            to_svg(&fresh.page.scene, &SvgOptions::default()),
            "cached and uncached renders differ"
        );
        assert_eq!(cached.interactions, fresh.interactions);
        cached.reused
    };
    let (state, _) = InteractionState::from_page(page);
    let data = query_page_data(&env, report, page, &state, &opts.limits);
    let mut view = PageViewState::new();
    assert!(render(&state, &view, &data).is_empty(), "cold cache");
    let all = render(&state, &view, &data);
    assert!(all.len() > 10, "{all:?}");

    let table = page
        .visuals
        .iter()
        .find(|v| v.visual_type == "tableEx")
        .expect("a table");
    view.visual_mut(&table.name).table.scroll.y = 120.0;
    let reused = render(&state, &view, &data);
    let others: Vec<String> = all.iter().filter(|n| **n != table.name).cloned().collect();
    assert_eq!(reused, others, "a scroll re-renders the table only");

    // Select the first value of the first slicer with values.
    let (slicer, value) = page
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
        .expect("a slicer with values");
    let mut selected = state.clone();
    selected.select(
        slicer.clone(),
        pbi_render::query::SlicerSelection {
            values: vec![vec![value]],
            ..Default::default()
        },
    );
    let filtered = query_page_data(&env, report, page, &selected, &opts.limits);
    let changed: Vec<&String> = all
        .iter()
        .filter(|n| **n == slicer || data.get(n) != filtered.get(n))
        .collect();
    let reused = render(&selected, &view, &filtered);
    let expected: Vec<String> = all
        .iter()
        .filter(|n| !changed.contains(n))
        .cloned()
        .collect();
    println!(
        "slicer {slicer}: re-rendered {} of {} visuals: {changed:?}",
        changed.len(),
        all.len()
    );
    assert_eq!(reused, expected);
    assert!(changed.len() < all.len(), "unaffected visuals are reused");
}

#[test]
fn golden_paths_follow_the_documented_layout() {
    let path = golden_path("Overview", "Executive Summary");
    let rel = path.strip_prefix(reference_root()).unwrap();
    assert_eq!(
        rel,
        Path::new("golden/acme-outfitters/Overview/Executive Summary.png")
    );
}
