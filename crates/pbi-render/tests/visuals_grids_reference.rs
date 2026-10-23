//! Slicers, tables and matrices of the Acme Outfitters pages, rendered from
//! the example refreshed offline: saved slicer selections drawn, the Top N
//! table in its expected order, the calculation-group matrix with its
//! columns in sort-by-column order, table rows as hit regions. Nothing
//! rendered here is written to disk.
//!
//! The fixtures are committed; without them the tests print `SKIP: … missing`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod acme_support;

use std::collections::BTreeSet;

use acme_support::{
    acme, acme_expected, expected_page, page, render, report_index, visual_id, visual_texts,
};
use pbi_render::scene::HitKind;
use serde_json::Value as Json;

fn expected_visual<'a>(expected: &'a Json, short: &str, page: &str, key: &str) -> &'a Json {
    expected_page(expected, short, page)["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["visualKey"] == key)
        .unwrap_or_else(|| panic!("{short} / {page}: no visual {key}"))
}

#[test]
fn slicers_show_their_values_and_saved_selections() {
    let (Some(session), Some(expected)) = (acme(), acme_expected()) else {
        return;
    };
    let overview = report_index(session, "Overview");
    let report = &session.document().reports[overview];
    let rendered = render(session, overview, page(report, "Executive Summary"));
    let year = visual_id(expected, "Overview", "Executive Summary", "year");
    assert_eq!(
        visual_texts(&rendered.scene, &year),
        ["Year", "2024", "2025"]
    );
    // A dropdown without a selection shows "All".
    assert_eq!(
        visual_texts(&rendered.scene, "578558f36aded7690edd"),
        ["Region", "All"]
    );
    // A dropdown with one saved value shows it.
    let rendered = render(session, overview, page(report, "Time Intelligence"));
    let month = visual_id(expected, "Overview", "Time Intelligence", "month");
    assert_eq!(visual_texts(&rendered.scene, &month), ["Month", "2025-06"]);
    // A multi-select list with "Select all"; a numeric range.
    let rendered = render(session, overview, page(report, "Product Detail"));
    let category = visual_id(expected, "Overview", "Product Detail", "category");
    assert_eq!(
        visual_texts(&rendered.scene, &category),
        [
            "Category",
            "Select all",
            "Apparel",
            "Camping",
            "Climbing",
            "Hiking",
            "Paddling"
        ]
    );
    let price = visual_id(expected, "Overview", "Product Detail", "price");
    assert_eq!(
        visual_texts(&rendered.scene, &price),
        ["List price", "50", "400"]
    );

    let stores = report_index(session, "Stores");
    let report = &session.document().reports[stores];
    let rendered = render(session, stores, page(report, "Store Performance"));
    let region = visual_id(expected, "Stores", "Store Performance", "region");
    assert_eq!(visual_texts(&rendered.scene, &region), ["Region", "West"]);
}

#[test]
fn top_n_table_rows_follow_the_expected_order() {
    let (Some(session), Some(expected)) = (acme(), acme_expected()) else {
        return;
    };
    let index = report_index(session, "Overview");
    let report = &session.document().reports[index];
    let rendered = render(session, index, page(report, "Product Detail"));
    let want = expected_visual(expected, "Overview", "Product Detail", "top10");
    let table = want["visual"].as_str().unwrap();
    let texts = visual_texts(&rendered.scene, table);
    for header in ["Product Name", "Total Sales", "Units Sold", "Margin %"] {
        assert!(texts.iter().any(|t| t == header), "{header}: {texts:?}");
    }
    // Each row: name, then its formatted values, in the expected order.
    let mut expected_texts = Vec::new();
    for row in want["rows"].as_array().unwrap() {
        expected_texts.push(row["Product Name"].as_str().unwrap().to_owned());
        for field in ["Total Sales", "Units Sold", "Margin %"] {
            expected_texts.push(row[field]["formatted"].as_str().unwrap().to_owned());
        }
    }
    let start = texts
        .iter()
        .position(|t| *t == expected_texts[0])
        .expect("first row");
    assert_eq!(
        texts[start..start + expected_texts.len()],
        expected_texts[..],
        "{texts:?}"
    );
    // The totals row.
    let total = &want["total"];
    let at = texts.iter().rposition(|t| t == "Total").expect("total row");
    let totals: Vec<&str> = ["Total Sales", "Units Sold", "Margin %"]
        .iter()
        .map(|f| total[f]["formatted"].as_str().unwrap())
        .collect();
    assert_eq!(texts[at + 1..at + 4], totals[..]);
    // One hit region per row, starting at row 0.
    let rows: BTreeSet<usize> = rendered
        .scene
        .hit_regions()
        .into_iter()
        .filter(|h| h.id.visual == table)
        .filter_map(|h| match h.id.kind {
            HitKind::DataPoint { index, .. } => Some(index),
            _ => None,
        })
        .collect();
    assert_eq!(rows, (0..10).collect());
}

#[test]
fn calculation_group_columns_follow_their_ordinal() {
    let (Some(session), Some(expected)) = (acme(), acme_expected()) else {
        return;
    };
    let index = report_index(session, "Overview");
    let report = &session.document().reports[index];
    let rendered = render(session, index, page(report, "Time Intelligence"));
    let want = expected_visual(expected, "Overview", "Time Intelligence", "calc_matrix");
    let texts = visual_texts(&rendered.scene, want["visual"].as_str().unwrap());
    // The calculation items sort by `Ordinal` (Current, YTD, PY, YoY %),
    // then the row-total column.
    let at = texts.iter().position(|t| t == "Year").expect("row header");
    assert_eq!(
        texts[at + 1..at + 6],
        ["Current", "YTD", "PY", "YoY %", "Total"]
    );
    // Year 2025: the values under those headers.
    let row = &want["rows"][1];
    assert_eq!(row["Year"], 2025);
    let y = texts.iter().position(|t| t == "2025").expect("2025 row");
    let values: Vec<&str> = ["Current", "YTD", "PY", "YoY %"]
        .iter()
        .map(|c| row[c]["formatted"].as_str().unwrap())
        .collect();
    assert_eq!(texts[y + 1..y + 5], values[..]);
}

#[test]
fn store_table_and_detail_page_render() {
    let (Some(session), Some(expected)) = (acme(), acme_expected()) else {
        return;
    };
    let index = report_index(session, "Stores");
    let report = &session.document().reports[index];
    let rendered = render(session, index, page(report, "Store Performance"));
    let want = expected_visual(expected, "Stores", "Store Performance", "stores");
    let texts = visual_texts(&rendered.scene, want["visual"].as_str().unwrap());
    let names: Vec<&str> = want["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["Store Name"].as_str().unwrap())
        .collect();
    let shown: Vec<&str> = texts
        .iter()
        .map(String::as_str)
        .filter(|t| names.contains(t))
        .collect();
    assert_eq!(shown, names);

    // The hidden detail page renders too (pages are reachable by
    // drill-through and bookmarks).
    let detail = page(report, "Store Detail");
    let rendered = render(session, index, detail);
    let want = expected_visual(expected, "Stores", "Store Detail", "stores");
    let texts = visual_texts(&rendered.scene, want["visual"].as_str().unwrap());
    for row in want["rows"].as_array().unwrap() {
        for field in ["Store Name", "Store Manager", "City"] {
            let v = row[field].as_str().unwrap();
            assert!(texts.iter().any(|t| t == v), "{v}: {texts:?}");
        }
    }
}
