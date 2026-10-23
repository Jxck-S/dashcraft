//! Charts on the Acme Outfitters pages, rendered from the example refreshed
//! offline: the donut, bar, line and KPI of Executive Summary, the pie of
//! sessions under the report filter on a table without relationships, the
//! gauge, waterfall and map placeholder of Store Performance.
//!
//! The fixtures are committed; without them the tests print `SKIP: … missing`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod acme_support;

use acme_support::{
    acme, acme_expected, data_points, expected_page, find_visual, formatted, page, render,
    report_index, texts, visual_id,
};
use pbi_render::dataview::DataView;
use pbi_render::scene::{Group, HitKind, Node};
use pbi_render::visuals::RenderedPage;

const DONUT: &str = "c9f87905246642c35687";
const BAR: &str = "ed03ff7722a2eaf64065";
const LINE: &str = "569b9ae1b4cfad5aa48d";

fn data<'a>(rendered: &'a RenderedPage, visual: &str) -> &'a DataView {
    rendered
        .data
        .iter()
        .find(|(n, _)| n == visual)
        .and_then(|(_, r)| r.as_ref().ok())
        .unwrap_or_else(|| panic!("{visual}: no data"))
}

fn categories(dv: &DataView) -> Vec<String> {
    dv.categorical.as_ref().unwrap().categories[0]
        .values
        .iter()
        .map(|c| c.formatted.clone())
        .collect()
}

/// `(y, text)` of every text of `g`.
fn text_positions(g: &Group, out: &mut Vec<(f64, String)>) {
    for c in &g.children {
        match c {
            Node::Text(t) => out.push((t.y, t.text.clone())),
            Node::Group(child) => text_positions(child, out),
            _ => {}
        }
    }
}

/// Width of the first rect of every data point of series 0.
fn bar_widths(g: &Group, out: &mut Vec<f64>) {
    if let Some(HitKind::DataPoint { series: 0, .. }) = g.id.as_ref().map(|i| &i.kind)
        && let Some(w) = g.children.iter().find_map(|c| match c {
            Node::Rect(r) => Some(r.rect.width),
            _ => None,
        })
    {
        out.push(w);
    }
    for c in &g.children {
        if let Node::Group(child) = c {
            bar_widths(child, out);
        }
    }
}

#[test]
fn executive_summary_donut_bar_line_and_kpi() {
    let (Some(session), Some(expected)) = (acme(), acme_expected()) else {
        return;
    };
    let index = report_index(session, "Overview");
    let report = &session.document().reports[index];
    let rendered = render(session, index, page(report, "Executive Summary"));
    let scene = &rendered.scene;

    // Donut: one slice per channel, by value; the slices add up to the
    // page's Total Sales card.
    let donut = find_visual(&scene.root, DONUT).expect("donut");
    let dv = data(&rendered, DONUT);
    let channels = categories(dv);
    assert_eq!(channels, ["Retail", "Online", "Outlet"]);
    assert_eq!(data_points(donut).len(), 3);
    let t = texts(donut);
    assert!(t.contains(&"Channel".to_owned()), "legend title: {t:?}");
    for c in &channels {
        assert!(t.contains(c), "{c}: {t:?}");
    }
    let total: f64 = dv.categorical.as_ref().unwrap().values[0]
        .values
        .iter()
        .filter_map(|c| c.raw.as_f64())
        .sum();
    let card = &expected_page(expected, "Overview", "Executive Summary")["cards"][0];
    assert_eq!(card["measure"], "Total Sales");
    assert!(
        (total - card["value"].as_f64().unwrap()).abs() < 1e-6,
        "{total}"
    );

    // Bar: regions by Total Sales descending, two series (sales, target);
    // the top bar is the largest and its category label is the topmost.
    // The categories that do not fit the plot scroll (not drawn).
    let bar = find_visual(&scene.root, BAR).expect("bar");
    let dv = data(&rendered, BAR);
    let regions = categories(dv);
    assert_eq!(regions, ["Online", "West", "Central", "East", "South"]);
    let mut widths = Vec::new();
    bar_widths(bar, &mut widths);
    let drawn = widths.len();
    assert!(drawn >= 3, "{widths:?}");
    assert_eq!(data_points(bar).len(), 2 * drawn);
    assert!(widths.windows(2).all(|w| w[0] >= w[1]), "{widths:?}");
    let mut labels = Vec::new();
    text_positions(bar, &mut labels);
    let y_of = |s: &str| labels.iter().find(|(_, t)| t == s).map(|(y, _)| *y);
    let ys: Vec<f64> = regions.iter().map_while(|r| y_of(r)).collect();
    assert_eq!(ys.len(), drawn, "{labels:?}");
    assert!(ys.windows(2).all(|w| w[0] < w[1]), "{ys:?}");
    for legend in ["Total Sales", "Target"] {
        assert!(labels.iter().any(|(_, t)| t == legend), "{legend}");
    }

    // Line: 12 months × 2 series, months in calendar order (the model
    // sorts `Month` by `Month Number`).
    let line = find_visual(&scene.root, LINE).expect("line");
    assert_eq!(data_points(line).len(), 24);
    let months = categories(data(&rendered, LINE));
    assert_eq!(
        months,
        [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"
        ]
    );

    // KPI: the indicator and goal at the last trend point (expected.json).
    let kpi_id = visual_id(expected, "Overview", "Executive Summary", "kpi");
    let want = expected_page(expected, "Overview", "Executive Summary")["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["visualKey"] == "kpi")
        .unwrap();
    let kpi = find_visual(&scene.root, &kpi_id).expect("kpi");
    let t = texts(kpi);
    let indicator = formatted(want, "indicator");
    let goal = format!("Goal: {} (", formatted(want, "goal"));
    assert!(t.iter().any(|s| s == indicator), "{indicator}: {t:?}");
    assert!(t.iter().any(|s| s.starts_with(&goal)), "{goal}: {t:?}");
}

#[test]
fn online_pie_has_only_the_mobile_slice() {
    let (Some(session), Some(expected)) = (acme(), acme_expected()) else {
        return;
    };
    let index = report_index(session, "Online");
    let report = &session.document().reports[index];
    let rendered = render(session, index, page(report, "Online Overview"));
    let pie_id = visual_id(expected, "Online", "Online Overview", "devices");
    let pie = find_visual(&rendered.scene.root, &pie_id).expect("pie");
    assert_eq!(data_points(pie), [(0, 0)]);
    let t = texts(pie);
    for s in ["Device", "Mobile", "678", "(100.00%)"] {
        assert!(t.contains(&s.to_owned()), "{s}: {t:?}");
    }
}

#[test]
fn store_performance_gauge_waterfall_and_map_placeholder() {
    let Some(session) = acme() else { return };
    let index = report_index(session, "Stores");
    let report = &session.document().reports[index];
    let rendered = render(session, index, page(report, "Store Performance"));
    let scene = &rendered.scene;
    let gauge = find_visual(&scene.root, "6f647d31a8428ad62fb0").expect("gauge");
    assert_eq!(data_points(gauge).len(), 1);
    let waterfall = find_visual(&scene.root, "9b4e31f5f08b8acaa5ed").expect("waterfall");
    assert_eq!(data_points(waterfall).len(), 4);
    let t = texts(waterfall);
    for q in ["Q1", "Q2", "Q3", "Q4", "Total"] {
        assert!(t.contains(&q.to_owned()), "{q}: {t:?}");
    }
    // Maps need online tiles: a labelled placeholder with the bound fields.
    let map = find_visual(&scene.root, "a7199dc503d24da7b45e").expect("map");
    let t = texts(map);
    assert!(t.contains(&"Category: City".to_owned()), "{t:?}");
    assert!(rendered.visuals.iter().all(|v| v.error.is_none()));
}
