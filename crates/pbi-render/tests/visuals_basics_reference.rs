//! Every KPI card of every Acme Outfitters page, rendered from the example
//! refreshed offline (its CSVs through `blob-map.json`), with the page's
//! saved slicer selections: each card value of `expected.json` is drawn,
//! followed by its label. Also: no visual fails, and the buttons carry
//! their actions.
//!
//! The fixtures are committed; without them the test prints `SKIP: … missing`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod acme_support;

use acme_support::{
    ACME_REPORTS, acme, acme_expected, expected_report, page, render, report_index, visual_texts,
};
use pbi_render::dataview::DataView;
use pbi_render::svg::{SvgOptions, to_svg};
use pbi_render::visuals::button::ButtonAction;

/// The label a card draws for `measure`: the column's display name.
fn label(dv: &DataView, measure: &str) -> String {
    dv.metadata
        .iter()
        .find(|m| m.source.property.as_deref() == Some(measure))
        .map_or_else(|| measure.to_owned(), |m| m.display_name.clone())
}

#[test]
fn every_card_shows_its_expected_value_on_the_rendered_page() {
    let (Some(session), Some(expected)) = (acme(), acme_expected()) else {
        return;
    };
    let mut rows = Vec::new();
    let mut failures = Vec::new();
    for short in ACME_REPORTS {
        let index = report_index(session, short);
        let report = &session.document().reports[index];
        for entry in expected_report(expected, short)["pages"]
            .as_array()
            .unwrap()
        {
            let name = entry["page"].as_str().unwrap();
            let page = page(report, name);
            let rendered = render(session, index, page);
            let errors: Vec<String> = rendered
                .visuals
                .iter()
                .filter_map(|v| v.error.as_ref().map(|e| format!("{}: {e}", v.name)))
                .collect();
            assert!(errors.is_empty(), "{short} / {name}: {errors:#?}");
            let svg = to_svg(&rendered.scene, &SvgOptions::default());
            for card in entry["cards"].as_array().into_iter().flatten() {
                let visual = card["visual"].as_str().unwrap();
                let measure = card["measure"].as_str().unwrap();
                let want = card["formatted"].as_str().unwrap();
                let dv = rendered
                    .data
                    .iter()
                    .find(|(n, _)| n == visual)
                    .and_then(|(_, r)| r.as_ref().ok())
                    .unwrap_or_else(|| panic!("{visual}: no data"));
                let label = label(dv, measure);
                let texts = visual_texts(&rendered.scene, visual);
                let shown = texts.windows(2).any(|w| w[0] == want && w[1] == label);
                rows.push(format!(
                    "{short:<9} {name:<18} {:<12} {label:<26} expected {want:>12}  {}",
                    card["visualType"].as_str().unwrap_or_default(),
                    if shown { "drawn" } else { "MISSING" }
                ));
                if !shown {
                    failures.push(format!(
                        "{short} / {name} / {label}: {want} not drawn above its label in {texts:?}"
                    ));
                }
                if !svg.contains(&format!(">{want}</text>")) {
                    failures.push(format!(
                        "{short} / {name} / {label}: {want} missing from the SVG"
                    ));
                }
            }
        }
    }
    println!("{}", rows.join("\n"));
    assert_eq!(rows.len(), 39, "every card value of expected.json");
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn buttons_carry_their_actions() {
    let Some(session) = acme() else { return };
    let index = report_index(session, "Overview");
    let report = &session.document().reports[index];
    let summary = page(report, "Executive Summary");
    let detail = page(report, "Product Detail");
    let rendered = render(session, index, summary);
    let button = rendered.visual("5cd0757711b84d2d8214").expect("button");
    assert_eq!(
        button.action,
        Some(ButtonAction::PageNavigation {
            page: detail.name.clone()
        })
    );
    let rendered = render(session, index, detail);
    let back = rendered.visual("2bc4995228830afb03fb").expect("back");
    assert_eq!(back.action, Some(ButtonAction::Back));
}
