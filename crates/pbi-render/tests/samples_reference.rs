//! The Microsoft Power BI Desktop samples (MIT, `samples/microsoft/`, and
//! the fetched `samples/downloads/` ones when present): the report
//! structure matches their `expected.json` (pages, sizes, visibility, visual
//! counts by type, groups included), and every page of every report
//! renders without panicking, with one render entry per visual.
//!
//! Committed samples are always present; a fetched one that is missing
//! prints `SKIP: … missing (run scripts/fetch-samples.sh)`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod acme_support;

use std::collections::BTreeMap;

use acme_support::{open_sample, sample_files, samples_expected, theme};
use pbi_render::query::{InteractionState, QueryEnv};
use pbi_render::svg::{SvgOptions, to_svg};
use pbi_render::visuals::{PageRenderOptions, render_report_page};

/// Checks and renders the sample with `expected.json` id `id`.
fn check_sample(id: &str) {
    if samples_expected().is_none() {
        return;
    }
    let files = sample_files();
    let file = files
        .iter()
        .find(|f| f["id"] == id)
        .unwrap_or_else(|| panic!("expected.json: no sample {id}"));
    let path = file["file"].as_str().unwrap();
    let Some(session) = open_sample(path) else {
        assert_eq!(file["committed"], false, "{path} is committed");
        return;
    };
    let want = &file["report"];
    let doc = session.document();
    assert_eq!(doc.reports.len(), 1, "{path}");
    let report = &doc.reports[0];
    let pages = want["pages"].as_array().unwrap();
    assert_eq!(report.pages.len(), pages.len(), "{path}: pages");
    let env = QueryEnv::from_session(&session);
    let theme = theme(&session, 0);
    for (page, w) in report.pages.iter().zip(pages) {
        let what = format!("{path} / {}", page.display_name);
        assert_eq!(page.name, w["name"].as_str().unwrap(), "{what}");
        assert_eq!(
            page.display_name,
            w["displayName"].as_str().unwrap(),
            "{what}"
        );
        assert_eq!(
            (page.width, page.height),
            (w["width"].as_f64().unwrap(), w["height"].as_f64().unwrap()),
            "{what}: size"
        );
        assert_eq!(
            page.is_hidden,
            w["hidden"].as_bool().unwrap(),
            "{what}: hidden"
        );
        let mut by_type: BTreeMap<String, u64> = BTreeMap::new();
        for v in &page.visuals {
            let t = if v.is_group() {
                "group"
            } else {
                v.visual_type.as_str()
            };
            *by_type.entry(t.to_owned()).or_default() += 1;
        }
        let want_types: BTreeMap<String, u64> = w["visualsByType"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_u64().unwrap()))
            .collect();
        assert_eq!(by_type, want_types, "{what}: visuals by type");
        assert_eq!(
            page.visuals.len() as u64,
            w["visuals"].as_u64().unwrap(),
            "{what}"
        );

        let (state, _) = InteractionState::from_page(page);
        let rendered = render_report_page(
            &env,
            report,
            page,
            &theme,
            &state,
            &PageRenderOptions::default(),
        );
        assert_eq!(
            rendered.visuals.len(),
            page.visuals.len(),
            "{what}: render entries"
        );
        assert!(rendered.scene.width > 0.0 && rendered.scene.height > 0.0);
        let svg = to_svg(&rendered.scene, &SvgOptions::default());
        usvg::Tree::from_str(&svg, &usvg::Options::default())
            .unwrap_or_else(|e| panic!("{what}: SVG does not parse: {e}"));
        let failed = rendered
            .visuals
            .iter()
            .filter(|v| v.error.is_some())
            .count();
        println!(
            "{what}: {} visuals rendered, {failed} drawn as ⚠ errors",
            rendered.visuals.len()
        );
    }
}

#[test]
fn revenue_opportunities() {
    check_sample("revenue-opportunities");
}

#[test]
fn corporate_spend() {
    check_sample("corporate-spend");
}

#[test]
fn regional_sales_legacy_layout() {
    check_sample("regional-sales");
}

#[test]
fn covid_tracking_template() {
    check_sample("covid-19-us-tracking");
}

#[test]
fn adventureworks_sales_if_fetched() {
    check_sample("adventureworks-sales");
}

#[test]
fn store_sales_if_fetched() {
    check_sample("store-sales");
}

#[test]
fn every_sample_has_a_test() {
    let ids: Vec<&str> = sample_files()
        .iter()
        .map(|f| f["id"].as_str().unwrap())
        .collect();
    if ids.is_empty() {
        return;
    }
    assert_eq!(
        ids,
        [
            "revenue-opportunities",
            "corporate-spend",
            "regional-sales",
            "covid-19-us-tracking",
            "adventureworks-sales",
            "store-sales"
        ]
    );
}
