//! Visual queries (DataViews) on the Acme Outfitters example, refreshed
//! offline from its CSVs through `blob-map.json`, checked against the
//! independent values in `expected.json`: every KPI card of every page,
//! table, matrix, gauge, KPI and pie values, saved slicer selections,
//! slicer and cross-filter interactions (`visualInteractions`), the report
//! filter on a table without relationships; plus every visual of the
//! Microsoft samples queries without panicking.
//!
//! The fixtures are committed; without them the tests print `SKIP: … missing`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod acme_support;

use acme_support::{
    ACME_REPORTS, acme, acme_expected, expected_page, expected_report, open_sample, page, report,
    sample_files, visual_id,
};
use pbi_engine::Value;
use pbi_model::{FieldExpr, Page, Report};
use pbi_render::dataview::{CellValue, DataView, DataViewKind, Matrix, MatrixNode};
use pbi_render::query::{InteractionState, PointSelection, QueryEnv, QueryLimits, SlicerSelection};
use pbi_session::Session;
use pretty_assertions::assert_eq;
use serde_json::Value as Json;

fn query(
    session: &Session,
    report: &Report,
    page: &Page,
    visual: &str,
    state: &InteractionState,
) -> DataView {
    let v = page
        .visual(visual)
        .unwrap_or_else(|| panic!("no visual {visual}"));
    QueryEnv::from_session(session)
        .query_visual(report, page, v, state, &QueryLimits::default())
        .unwrap_or_else(|e| panic!("{visual}: {e:?}"))
}

/// The page's saved slicer selections.
fn saved(page: &Page) -> InteractionState {
    let (state, problems) = InteractionState::from_page(page);
    assert!(problems.is_empty(), "{}: {problems:?}", page.display_name);
    state
}

/// Asserts `cell` shows `want` (`{"value", "formatted"}`): the same text and,
/// for numbers, the same raw value.
fn assert_cell(what: &str, cell: &CellValue, want: &Json) {
    let formatted = want["formatted"].as_str().unwrap_or_default();
    match &want["value"] {
        Json::Null => assert!(cell.is_blank(), "{what}: {cell:?} is not BLANK"),
        Json::Number(n) => {
            let (got, want_n) = (cell.raw.as_f64(), n.as_f64().unwrap_or(f64::NAN));
            assert!(
                got.is_some_and(|g| (g - want_n).abs() <= 1e-6 * want_n.abs().max(1.0)),
                "{what}: raw {:?}, expected {want_n}",
                cell.raw
            );
            assert_eq!(cell.formatted, formatted, "{what}");
        }
        other => panic!("{what}: unexpected value {other}"),
    }
}

/// The text of a category in `expected.json` (strings or numbers).
fn category_text(v: &Json) -> String {
    match v {
        Json::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Index of the metadata column for field `name` (model name or display
/// name).
fn column(dv: &DataView, name: &str) -> usize {
    dv.metadata
        .iter()
        .position(|m| m.source.property.as_deref() == Some(name) || m.display_name == name)
        .unwrap_or_else(|| panic!("no column {name} in {:?}", dv.metadata))
}

/// The cell of value field `name` in a single-value or one-row card.
fn card_cell<'a>(dv: &'a DataView, name: &str) -> &'a CellValue {
    match dv.kind {
        DataViewKind::Single => dv.single.as_ref().expect("single"),
        DataViewKind::Table => {
            let t = dv.table.as_ref().expect("table");
            let m = column(dv, name);
            let pos = t.columns.iter().position(|&c| c == m).expect("column");
            &t.rows[0][pos]
        }
        other => panic!("card data view {other:?}"),
    }
}

#[test]
fn kpi_cards_match_expected_on_every_page() {
    let (Some(session), Some(expected)) = (acme(), acme_expected()) else {
        return;
    };
    let mut checked = 0;
    for short in ACME_REPORTS {
        let report = report(session, short);
        for entry in expected_report(expected, short)["pages"]
            .as_array()
            .unwrap()
        {
            let name = entry["page"].as_str().unwrap();
            let page = page(report, name);
            let state = saved(page);
            for card in entry["cards"].as_array().into_iter().flatten() {
                let visual = card["visual"].as_str().unwrap();
                let measure = card["measure"].as_str().unwrap();
                let dv = query(session, report, page, visual, &state);
                let cell = card_cell(&dv, measure);
                println!(
                    "{short:<9} {name:<18} {measure:<26} expected {:>12}  actual {:>12}",
                    card["formatted"].as_str().unwrap_or_default(),
                    cell.formatted
                );
                assert_cell(&format!("{short} / {name} / {measure}"), cell, card);
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 39, "every card value of expected.json");
}

fn table_row<'a>(dv: &'a DataView, key: &str, value: &str) -> &'a [CellValue] {
    let t = dv.table.as_ref().expect("table");
    let m = column(dv, key);
    let pos = t.columns.iter().position(|&c| c == m).expect("key column");
    t.rows
        .iter()
        .find(|r| r[pos].formatted == value)
        .unwrap_or_else(|| panic!("no row {key} = {value}"))
}

fn table_cell<'a>(dv: &DataView, row: &'a [CellValue], name: &str) -> &'a CellValue {
    let t = dv.table.as_ref().expect("table");
    let m = column(dv, name);
    &row[t.columns.iter().position(|&c| c == m).expect("column")]
}

/// Checks `rows` (objects: one text key, the rest `{value, formatted}`)
/// and `total` of a table visual.
fn check_table(what: &str, dv: &DataView, want: &Json) {
    let t = dv.table.as_ref().expect("table");
    if let Some(rows) = want["rows"].as_array() {
        assert_eq!(t.rows.len(), rows.len(), "{what}: rows");
        for row in rows {
            let obj = row.as_object().unwrap();
            let (key, label) = obj
                .iter()
                .find(|(_, v)| !v.is_object())
                .expect("key column");
            let label = category_text(label);
            let got = table_row(dv, key, &label);
            for (field, v) in obj {
                let cell = table_cell(dv, got, field);
                let at = format!("{what} / {label} / {field}");
                if v.is_object() {
                    assert_cell(&at, cell, v);
                } else {
                    assert_eq!(cell.formatted, category_text(v), "{at}");
                }
            }
        }
    }
    if let Some(total) = want["total"].as_object() {
        let totals = t
            .totals
            .as_ref()
            .unwrap_or_else(|| panic!("{what}: no totals"));
        for (field, v) in total {
            assert_cell(
                &format!("{what} / total / {field}"),
                table_cell(dv, totals, field),
                v,
            );
        }
    }
}

/// The header texts of a matrix's columns (`""` for the total column).
fn matrix_headers(m: &Matrix) -> Vec<String> {
    m.columns
        .iter()
        .map(|c| {
            c.path
                .iter()
                .map(|p| p.formatted.as_str())
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect()
}

fn matrix_child<'a>(m: &'a Matrix, label: &str) -> &'a MatrixNode {
    m.rows
        .children
        .iter()
        .find(|n| n.value.as_ref().is_some_and(|v| v.formatted == label))
        .unwrap_or_else(|| panic!("no matrix row {label}"))
}

#[test]
fn tables_matrices_and_charts_match_expected() {
    let (Some(session), Some(expected)) = (acme(), acme_expected()) else {
        return;
    };
    let mut checked = 0;
    for short in ACME_REPORTS {
        let report = report(session, short);
        for entry in expected_report(expected, short)["pages"]
            .as_array()
            .unwrap()
        {
            let name = entry["page"].as_str().unwrap();
            let page = page(report, name);
            let state = saved(page);
            for want in entry["visuals"].as_array().into_iter().flatten() {
                let visual = want["visual"].as_str().unwrap();
                let key = want["visualKey"].as_str().unwrap();
                let what = format!("{short} / {name} / {key}");
                let dv = query(session, report, page, visual, &state);
                match want["visualType"].as_str().unwrap() {
                    "tableEx" | "multiRowCard" => {
                        check_table(&what, &dv, want);
                        if let Some(n) = want["rowCount"].as_u64() {
                            let rows = dv.table.as_ref().unwrap().rows.len();
                            assert_eq!(rows as u64, n, "{what}: row count");
                        }
                    }
                    "pivotTable" => check_matrix(&what, &dv, want),
                    "kpi" => {
                        let cat = dv.categorical.as_ref().expect("categorical");
                        let axis = &cat.categories[0].values;
                        let last = axis.len().checked_sub(1).expect("trend points");
                        assert_eq!(axis[last].formatted, want["trendPoint"].as_str().unwrap());
                        assert_cell(
                            &format!("{what} / indicator"),
                            &cat.values[0].values[last],
                            &want["indicator"],
                        );
                        assert_cell(
                            &format!("{what} / goal"),
                            &cat.values[1].values[last],
                            &want["goal"],
                        );
                    }
                    "gauge" => {
                        let cat = dv.categorical.as_ref().expect("categorical");
                        for (i, role) in ["value", "target", "max"].into_iter().enumerate() {
                            assert_cell(
                                &format!("{what} / {role}"),
                                &cat.values[i].values[0],
                                &want[role],
                            );
                        }
                    }
                    "pieChart" => {
                        let cat = dv.categorical.as_ref().expect("categorical");
                        let wanted = want["categories"].as_array().unwrap();
                        assert_eq!(cat.categories[0].values.len(), wanted.len(), "{what}");
                        for (i, c) in wanted.iter().enumerate() {
                            let obj = c.as_object().unwrap();
                            let (_, label) = obj.iter().find(|(_, v)| !v.is_object()).unwrap();
                            assert_eq!(cat.categories[0].values[i].formatted, category_text(label));
                            let (_, v) = obj.iter().find(|(_, v)| v.is_object()).unwrap();
                            assert_cell(&what, &cat.values[0].values[i], v);
                        }
                    }
                    other => panic!("{what}: no check for {other}"),
                }
                println!("{what}: ok");
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 11, "every visual entry of expected.json");
}

fn check_matrix(what: &str, dv: &DataView, want: &Json) {
    let m = dv.matrix.as_ref().expect("matrix");
    let headers = matrix_headers(m);
    let values: Vec<usize> = m.values.clone();
    // A cell of `node` for value field `field` under column header `header`.
    let cell = |node: &MatrixNode, header: &str, field: &str| -> CellValue {
        let col = headers
            .iter()
            .position(|h| h == header)
            .unwrap_or_else(|| panic!("{what}: no column {header:?} in {headers:?}"));
        let q = values
            .iter()
            .position(|&v| {
                let meta = &dv.metadata[v];
                meta.source.property.as_deref() == Some(field) || meta.display_name == field
            })
            .unwrap_or(0);
        node.cells[col * values.len() + q].clone()
    };
    if m.column_levels.is_empty() {
        // Rows × value fields; one (empty) column.
        for (field, v) in want["total"].as_object().into_iter().flatten() {
            assert_cell(
                &format!("{what} / total / {field}"),
                &cell(&m.rows, "", field),
                v,
            );
        }
        for row in want["rows"].as_array().into_iter().flatten() {
            let obj = row.as_object().unwrap();
            let (_, label) = obj.iter().find(|(_, v)| !v.is_object()).unwrap();
            let node = matrix_child(m, &category_text(label));
            for (field, v) in obj.iter().filter(|(_, v)| v.is_object()) {
                assert_cell(
                    &format!("{what} / {label} / {field}"),
                    &cell(node, "", field),
                    v,
                );
            }
        }
        return;
    }
    let field = dv.metadata[values[0]]
        .source
        .property
        .clone()
        .unwrap_or_default();
    if let Some(columns) = want["columns"].as_array() {
        // One value field per column header, at the grand-total row.
        for c in columns {
            let obj = c.as_object().unwrap();
            let (_, label) = obj.iter().find(|(_, v)| !v.is_object()).unwrap();
            let (_, v) = obj.iter().find(|(_, v)| v.is_object()).unwrap();
            assert_cell(
                &format!("{what} / {label}"),
                &cell(&m.rows, &category_text(label), &field),
                v,
            );
        }
        let mut order: Vec<String> = columns
            .iter()
            .filter_map(|c| {
                c.as_object()?
                    .values()
                    .find(|v| !v.is_object())
                    .map(category_text)
            })
            .collect();
        order.push(String::new());
        assert_eq!(headers, order, "{what}: column order");
    } else {
        // Columns are the keys of the row objects (calculation items in
        // their sort order).
        let first = want["rows"][0].as_object().expect("rows");
        let mut order: Vec<String> = first
            .iter()
            .filter(|(_, v)| v.is_object())
            .map(|(k, _)| k.clone())
            .collect();
        order.push(String::new());
        assert_eq!(headers, order, "{what}: column order");
        for row in want["rows"].as_array().into_iter().flatten() {
            let obj = row.as_object().unwrap();
            let (_, label) = obj.iter().find(|(_, v)| !v.is_object()).unwrap();
            let node = matrix_child(m, &category_text(label));
            for (header, v) in obj.iter().filter(|(_, v)| v.is_object()) {
                assert_cell(
                    &format!("{what} / {label} / {header}"),
                    &cell(node, header, &field),
                    v,
                );
            }
        }
    }
    for (header, v) in want["total"].as_object().into_iter().flatten() {
        let header = if header == &field {
            ""
        } else {
            header.as_str()
        };
        assert_cell(
            &format!("{what} / total / {header}"),
            &cell(&m.rows, header, &field),
            v,
        );
    }
}

#[test]
fn saved_slicer_selections_are_read() {
    let (Some(session), Some(expected)) = (acme(), acme_expected()) else {
        return;
    };
    let overview = report(session, "Overview");
    let summary = page(overview, "Executive Summary");
    let year = visual_id(expected, "Overview", "Executive Summary", "year");
    let state = saved(summary);
    assert_eq!(
        state.slicer_selections.get(&year),
        Some(&SlicerSelection::values([Value::Integer(2025)]))
    );
    // A list slicer marks its saved selection.
    let dv = query(session, overview, summary, &year, &state);
    let slicer = dv.slicer.as_ref().expect("slicer");
    assert!(!slicer.all_selected);
    let values = &dv.categorical.as_ref().unwrap().categories[0].values;
    let selected: Vec<&str> = slicer
        .selected
        .iter()
        .map(|&i| values[i].formatted.as_str())
        .collect();
    assert_eq!(selected, ["2025"]);

    // Online Overview: an inverted selection ("all but Unassigned").
    let online = report(session, "Online");
    let overview_page = page(online, "Online Overview");
    let tier = visual_id(expected, "Online", "Online Overview", "tier");
    let sel = saved(overview_page)
        .slicer_selections
        .get(&tier)
        .cloned()
        .expect("tier");
    assert!(sel.inverted);
    assert_eq!(sel.values, vec![vec![Value::from("Unassigned")]]);

    // Product Detail: a numeric range is kept as a condition.
    let detail = page(overview, "Product Detail");
    let price = visual_id(expected, "Overview", "Product Detail", "price");
    let sel = saved(detail)
        .slicer_selections
        .get(&price)
        .cloned()
        .expect("price");
    assert!(sel.condition.is_some() && sel.values.is_empty(), "{sel:?}");
}

#[test]
fn slicer_and_cross_filter_interactions() {
    let (Some(session), Some(expected)) = (acme(), acme_expected()) else {
        return;
    };
    let id = |key: &str| visual_id(expected, "Overview", "Executive Summary", key);
    let (kpis, scaled, region) = (id("kpis"), id("scaled"), "578558f36aded7690edd");
    let report = report(session, "Overview");
    let summary = page(report, "Executive Summary");
    let base = saved(summary);
    let env = QueryEnv::from_session(session);
    let total = |dv: &DataView| card_cell(dv, "Total Sales").raw.as_f64().unwrap();
    let all = total(&query(session, report, summary, &kpis, &base));
    assert!((all - 457_432.15).abs() < 1e-6);

    // Selecting West in the Region slicer filters the cards; the DAX engine
    // agrees in the same filter context.
    let mut west = base.clone();
    west.select(region, SlicerSelection::values(["West"]));
    let west_sales = total(&query(session, report, summary, &kpis, &west));
    assert!(west_sales > 0.0 && west_sales < all, "{west_sales}");
    let v = summary.visual(&kpis).unwrap();
    let (ctx, warnings) = env.page_filter_context(report, summary, v, &west);
    assert!(warnings.is_empty(), "{warnings:?}");
    let dax = pbi_dax::evaluate_measure(&env.snapshot, &env.model, "Total Sales", &ctx).unwrap();
    assert!((dax.as_f64().unwrap() - west_sales).abs() < 1e-6);
    // The slicer lists every region and marks West.
    let dv = query(session, report, summary, region, &west);
    let slicer = dv.slicer.unwrap();
    assert_eq!(slicer.selected.len(), 1);

    // Cross-filter from the bar chart (Region = West): the page sets the
    // bar → KPI cards interaction to NoFilter, so they keep the page
    // total; the Sales (Scaled) card (default DataFilter) follows.
    let bar = id_bar(summary);
    let mut point = base.clone();
    point.selected_points = Some(PointSelection::point(
        bar.clone(),
        vec![(FieldExpr::column("Stores", "Region"), Value::from("West"))],
    ));
    assert_eq!(total(&query(session, report, summary, &kpis, &point)), all);
    let scaled_west = card_cell(
        &query(session, report, summary, &scaled, &point),
        "Sales (Scaled)",
    )
    .raw
    .as_f64()
    .unwrap();
    assert!(
        (scaled_west - west_sales).abs() < 1e-6,
        "{scaled_west} vs {west_sales}"
    );

    // Donut → bar is DataFilter: Online orders only.
    let donut = "c9f87905246642c35687";
    let mut online = base.clone();
    online.selected_points = Some(PointSelection::point(
        donut,
        vec![(
            FieldExpr::column("Stores", "Channel"),
            Value::from("Online"),
        )],
    ));
    let sum = |dv: &DataView| -> f64 {
        dv.categorical.as_ref().unwrap().values[0]
            .values
            .iter()
            .filter_map(|c| c.raw.as_f64())
            .sum()
    };
    let bar_all = sum(&query(session, report, summary, &bar, &base));
    let bar_online = sum(&query(session, report, summary, &bar, &online));
    assert!((bar_all - all).abs() < 1e-6, "{bar_all}");
    assert!(bar_online > 0.0 && bar_online < bar_all, "{bar_online}");
    // Column chart → donut is HighlightFilter: applied as a filter, with a
    // warning.
    let mut camping = base.clone();
    camping.selected_points = Some(PointSelection::point(
        "bdf9d92d87bec8350cc4",
        vec![(
            FieldExpr::column("Products", "Category"),
            Value::from("Camping"),
        )],
    ));
    let dv = query(session, report, summary, donut, &camping);
    assert!(
        dv.warnings.iter().any(|w| w.message.contains("highlight")),
        "{:?}",
        dv.warnings
    );
}

/// The clustered bar chart of Executive Summary ("Sales and target by region").
fn id_bar(page: &Page) -> String {
    page.visuals
        .iter()
        .find(|v| v.visual_type == "clusteredBarChart")
        .map(|v| v.name.clone())
        .expect("bar chart")
}

#[test]
fn report_filter_on_an_unrelated_table_only_filters_its_table() {
    let (Some(session), Some(expected)) = (acme(), acme_expected()) else {
        return;
    };
    let want = &expected["unrelatedReportFilter"];
    let online = report(session, "Online");
    assert_eq!(online.name, want["report"].as_str().unwrap());
    let overview = page(online, "Online Overview");
    let env = QueryEnv::from_session(session);
    // No slicer selections: the report filters only.
    let none = InteractionState::new();
    let card = overview
        .visuals
        .iter()
        .find(|v| v.visual_type == "card")
        .expect("Sessions card");
    let eval = |report: &Report, measure: &str| {
        let (ctx, warnings) = env.page_filter_context(report, overview, card, &none);
        assert!(warnings.iter().all(|w| !w.not_applied), "{warnings:?}");
        let v = pbi_dax::evaluate_measure(&env.snapshot, &env.model, measure, &ctx).unwrap();
        v.as_f64().unwrap_or(f64::NAN)
    };
    let num = |v: &Json| v["value"].as_f64().unwrap();
    assert_eq!(
        eval(online, "Sessions"),
        num(&want["Sessions"]["withReportFilters"])
    );
    assert_eq!(
        eval(online, "Sessions (All Devices)"),
        num(&want["Sessions (All Devices)"])
    );
    let sales = eval(online, "Total Sales");
    assert!((sales - num(&want["Total Sales"]["withReportFilters"])).abs() < 1e-6);

    // Without the Device filter: Sessions grows, Total Sales does not move.
    let mut without = online.clone();
    without.filters.retain(|f| {
        !f.field
            .as_ref()
            .is_some_and(|x| x.entity() == Some("Web Sessions"))
    });
    assert_eq!(without.filters.len() + 1, online.filters.len());
    assert_eq!(
        eval(&without, "Sessions"),
        num(&want["Sessions"]["withoutDeviceFilter"])
    );
    let sales = eval(&without, "Total Sales");
    assert!((sales - num(&want["Total Sales"]["withoutDeviceFilter"])).abs() < 1e-6);

    // On the page, the pie of sessions by device has only the Mobile slice.
    let entry = expected_page(expected, "Online", "Online Overview");
    assert!(entry["visuals"].as_array().is_some_and(|v| !v.is_empty()));
}

#[test]
fn every_visual_of_every_report_queries() {
    let Some(session) = acme() else { return };
    let mut count = 0;
    for report in &session.document().reports {
        for page in &report.pages {
            let state = saved(page);
            for (name, result) in pbi_render::query::query_page(
                session,
                report,
                page,
                &state,
                &QueryLimits::default(),
            ) {
                let v = page.visual(&name).unwrap();
                let dv = result.unwrap_or_else(|e| {
                    panic!(
                        "{} / {} / {name} ({}): {e:?}",
                        report.name, page.display_name, v.visual_type
                    )
                });
                // Relative-date filters are the only filters not applied
                // yet (they keep every row here: the data ends 2025-12-31).
                for w in dv.warnings.iter().filter(|w| w.not_applied) {
                    assert!(
                        w.message.contains("RelativeDate"),
                        "{} / {name}: {w:?}",
                        page.display_name
                    );
                }
                println!(
                    "{:<32} {:<18} {:<34} {:?}{}",
                    report.name,
                    page.display_name,
                    v.visual_type,
                    dv.kind,
                    if dv.truncated { " (truncated)" } else { "" }
                );
                count += 1;
            }
        }
    }
    assert_eq!(count, 80, "the example's visuals");
}

/// The committed samples (`tests/samples_reference.rs` renders the fetched
/// ones too).
#[test]
fn sample_visuals_never_panic() {
    for file in sample_files()
        .into_iter()
        .filter(|f| f["committed"] == true)
    {
        let path = file["file"].as_str().unwrap();
        let Some(session) = open_sample(path) else {
            continue;
        };
        let (mut ok, mut failed) = (0, 0);
        for report in &session.document().reports {
            for page in &report.pages {
                let (state, _) = InteractionState::from_page(page);
                for (name, result) in pbi_render::query::query_page(
                    &session,
                    report,
                    page,
                    &state,
                    &QueryLimits::default(),
                ) {
                    match result {
                        Ok(_) => ok += 1,
                        Err(e) => {
                            failed += 1;
                            println!(
                                "  ⚠ {path} / {} / {name}: {}",
                                page.display_name,
                                e.display()
                            );
                        }
                    }
                }
            }
        }
        println!("{path}: {ok} visuals ok, {failed} failed");
        assert!(ok > 0, "{path}");
    }
}
