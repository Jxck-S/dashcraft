//! Render snapshots of every Acme Outfitters page (no Power BI goldens
//! needed): the example refreshed offline and rendered with each page's
//! saved slicer selections, summarised per visual (type, box, data points,
//! warnings, every drawn text and a hash of the visual's SVG) plus a hash
//! of the whole page's SVG. Any rendering change shows up as a snapshot
//! diff naming the visuals that changed; review it with `cargo insta
//! review` and accept it only when the change is intended.
//!
//! The hashes are FNV-1a over the SVG text, which is the same on every OS
//! (bundled fonts only, no wall clock: AGENTS.md rule 13).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod acme_support;

use std::fmt::Write as _;

use acme_support::{ACME_REPORTS, acme, data_points, render, report_index, texts};
use pbi_render::layout::{LayoutOptions, PageLayout};
use pbi_render::scene::{Group, HitKind, Node, Scene};
use pbi_render::svg::{SvgOptions, to_svg};

/// 64-bit FNV-1a.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// The visual container groups of `g`, in drawing order.
fn visual_groups<'a>(g: &'a Group, out: &mut Vec<&'a Group>) {
    if g.id.as_ref().is_some_and(|id| id.kind == HitKind::Visual) {
        out.push(g);
    }
    for c in &g.children {
        if let Node::Group(child) = c {
            visual_groups(child, out);
        }
    }
}

fn snapshot(short: &str, page_name: &str) -> Option<String> {
    let session = acme()?;
    let index = report_index(session, short);
    let report = &session.document().reports[index];
    let page = report
        .pages
        .iter()
        .find(|p| p.display_name == page_name)
        .unwrap_or_else(|| panic!("no page {page_name}"));
    let rendered = render(session, index, page);
    let svg = to_svg(&rendered.scene, &SvgOptions::default());
    // The same input renders the same bytes.
    let again = to_svg(&render(session, index, page).scene, &SvgOptions::default());
    assert!(svg == again, "{short} / {page_name}: non-deterministic SVG");

    let layout = PageLayout::compute_with(
        page,
        &acme_support::theme(session, index),
        &LayoutOptions::for_report(report, None),
    );
    let mut out = String::new();
    writeln!(
        out,
        "{} / {} ({:?}, {}×{}, {} visuals)",
        report.name,
        page.display_name,
        page.display_option,
        page.width,
        page.height,
        page.visuals.len()
    )
    .unwrap();
    writeln!(
        out,
        "page svg: {} bytes, fnv1a {:016x}",
        svg.len(),
        fnv1a(svg.as_bytes())
    )
    .unwrap();
    let mut groups = Vec::new();
    visual_groups(&rendered.scene.root, &mut groups);
    for g in groups {
        let name = &g.id.as_ref().unwrap().visual;
        let info = rendered
            .visual(name)
            .unwrap_or_else(|| panic!("{name}: no info"));
        let mut alone = Scene::new(rendered.scene.width, rendered.scene.height);
        alone.root.push(g.clone());
        let part = to_svg(&alone, &SvgOptions::default());
        let rect = layout
            .frame(name)
            .map(|f| {
                format!(
                    "{}×{} at ({}, {})",
                    f.outer.width, f.outer.height, f.outer.x, f.outer.y
                )
            })
            .unwrap_or_default();
        writeln!(
            out,
            "\n{name} {} {rect}: svg {:016x}, {} data points",
            info.visual_type,
            fnv1a(part.as_bytes()),
            data_points(g).len()
        )
        .unwrap();
        for w in &info.warnings {
            writeln!(out, "  warning: {w}").unwrap();
        }
        if let Some(e) = &info.error {
            writeln!(out, "  error: {e}").unwrap();
        }
        for t in texts(g) {
            writeln!(out, "  | {t}").unwrap();
        }
    }
    Some(out)
}

fn check(short: &str, page: &str, snapshot_name: &str) {
    let Some(text) = snapshot(short, page) else {
        return;
    };
    insta::assert_snapshot!(snapshot_name, text);
}

#[test]
fn overview_executive_summary() {
    check(
        "Overview",
        "Executive Summary",
        "overview_executive_summary",
    );
}

#[test]
fn overview_product_detail() {
    check("Overview", "Product Detail", "overview_product_detail");
}

#[test]
fn overview_time_intelligence() {
    check(
        "Overview",
        "Time Intelligence",
        "overview_time_intelligence",
    );
}

#[test]
fn stores_store_performance() {
    check("Stores", "Store Performance", "stores_store_performance");
}

#[test]
fn stores_store_detail() {
    check("Stores", "Store Detail", "stores_store_detail");
}

#[test]
fn stores_store_tooltip() {
    check("Stores", "Store Tooltip", "stores_store_tooltip");
}

#[test]
fn online_online_overview() {
    check("Online", "Online Overview", "online_online_overview");
}

#[test]
fn online_customer_detail() {
    check("Online", "Customer Detail", "online_customer_detail");
}

#[test]
fn every_page_has_a_snapshot() {
    let Some(session) = acme() else { return };
    let mut pages: Vec<String> = ACME_REPORTS
        .iter()
        .flat_map(|short| {
            let report = &session.document().reports[report_index(session, short)];
            report
                .pages
                .iter()
                .map(move |p| format!("{short} / {}", p.display_name))
        })
        .collect();
    pages.sort();
    assert_eq!(
        pages,
        [
            "Online / Customer Detail",
            "Online / Online Overview",
            "Overview / Executive Summary",
            "Overview / Product Detail",
            "Overview / Time Intelligence",
            "Stores / Store Detail",
            "Stores / Store Performance",
            "Stores / Store Tooltip"
        ]
    );
}
