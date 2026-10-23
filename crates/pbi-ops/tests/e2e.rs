//! End to end through `Registry::execute` on the synthetic Acme Outfitters
//! example (`reference/examples/acme-outfitters/`): open → inspect → lint →
//! pages → render, then refresh → measures → DAX → canvas with the values of
//! its `expected.json`; and the local data cache of a PBIP built from a
//! committed Microsoft sample.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::{Path, PathBuf};

use common::{
    ACME, BLOB_MAP, MS_PBIX, ORDERS_CSV, PBIP, Runner, copy_dir, expected, fixture, ms_expected,
    reference_root, tmp_dir,
};
use pbi_ops::OpErrorCode;
use serde_json::{Value, json};

fn measure<'a>(out: &'a Value, name: &str) -> &'a Value {
    out["measures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == name)
        .unwrap_or_else(|| panic!("no measure {name}"))
}

fn table<'a>(tables: &'a Value, name: &str) -> &'a Value {
    tables
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == name)
        .unwrap_or_else(|| panic!("no table {name} in {tables}"))
}

/// `visual.json` files under `dir` (counted on disk, independently of DashCraft).
fn visual_files(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .map(|p| {
            if p.is_dir() {
                visual_files(&p)
            } else {
                usize::from(p.file_name().unwrap() == "visual.json")
            }
        })
        .sum()
}

/// A copy of the example with one stale `queryRef` (a LINT003 warning) on the
/// Executive Summary KPI card.
fn acme_with_stale_query_ref(dir: &Path) -> PathBuf {
    let root = dir.join("acme");
    copy_dir(&reference_root().join(ACME), &root);
    let visual = root.join(
        "Acme Outfitters - Overview.Report/definition/pages/ef796b39c816a4afa4f1/visuals/f7a9c9413fef68532313/visual.json",
    );
    let text = std::fs::read_to_string(&visual).unwrap();
    let stale = text.replacen(
        "\"queryRef\": \"Orders.Total Sales\"",
        "\"queryRef\": \"Orders.Sales Total\"",
        1,
    );
    assert_ne!(stale, text, "the KPI card projects Orders.Total Sales");
    std::fs::write(&visual, stale).unwrap();
    root.join("AcmeOutfitters.pbip")
}

#[tokio::test(flavor = "multi_thread")]
async fn pbip_structure_lint_and_render() {
    let (Some(pbip), Some(exp)) = (fixture(PBIP), expected()) else {
        return;
    };
    let acme = reference_root().join(ACME);
    let reports = exp.0["reports"].as_array().unwrap();
    let pages: usize = reports
        .iter()
        .map(|r| r["pages"].as_array().unwrap().len())
        .sum();
    let relationships = std::fs::read_to_string(
        acme.join("AcmeOutfitters.SemanticModel/definition/relationships.tmdl"),
    )
    .unwrap()
    .lines()
    .filter(|l| l.starts_with("relationship "))
    .count();

    let r = Runner::new();
    let opened = r
        .run("doc.open", json!({"path": pbip, "autoRefresh": false}))
        .await;
    let doc = opened["docId"].as_str().unwrap().to_owned();
    assert_eq!(doc, "d1");
    assert_eq!(opened["format"], "pbip");
    assert_eq!(opened["tables"], exp.row_counts().len());
    assert_eq!(opened["measures"], exp.measures().len());
    assert_eq!(opened["reports"].as_array().unwrap().len(), reports.len());
    assert_eq!(opened["loaded"], false);

    let inspect = r.run("doc.inspect", json!({"docId": doc})).await;
    assert_eq!(inspect["schemaVersion"], 1);
    let counts = &inspect["counts"];
    assert_eq!(
        (
            &counts["tables"],
            &counts["measures"],
            &counts["relationships"],
            &counts["reports"],
            &counts["pages"],
            &counts["visuals"]
        ),
        (
            &json!(exp.row_counts().len()),
            &json!(exp.measures().len()),
            &json!(relationships),
            &json!(reports.len()),
            &json!(pages),
            &json!(visual_files(&acme))
        )
    );
    assert!(inspect.to_string().contains("DISTINCTCOUNTNOBLANK"));
    let outline = r
        .run(
            "doc.inspect",
            json!({"docId": doc, "includeExpressions": false}),
        )
        .await;
    assert!(!outline.to_string().contains("DISTINCTCOUNTNOBLANK"));

    let lint = r.run("doc.lint", json!({"docId": doc})).await;
    assert_eq!(
        (&lint["summary"]["errors"], &lint["summary"]["warnings"]),
        (&json!(0), &json!(0)),
        "{lint}"
    );
    assert_eq!(lint["ok"], true);

    let measures = r.run("model.measures", json!({"docId": doc})).await;
    let total = measure(&measures, "Total Sales");
    assert_eq!(total["table"], "Orders");
    assert_eq!(total["expression"], "SUM(Orders[Line Amount])");
    let rels = r.run("model.relationships", json!({"docId": doc})).await;
    assert_eq!(
        rels["relationships"].as_array().unwrap().len(),
        relationships
    );
    assert_eq!(
        rels["relationships"][0]["crossFilteringBehavior"],
        "bothDirections"
    );
    let tables = r.run("model.tables", json!({"docId": doc})).await;
    assert_eq!(
        tables["tables"].as_array().unwrap().len(),
        exp.row_counts().len()
    );

    let list = r.run("report.list", json!({"docId": doc})).await;
    assert_eq!(list["reports"].as_array().unwrap().len(), reports.len());
    let stores = exp.report("Stores");
    let first = &stores["pages"][0];
    let page_dir = acme.join(format!(
        "{}/definition/pages/{}",
        stores["folder"].as_str().unwrap(),
        first["pageId"].as_str().unwrap()
    ));
    let on_first_page = visual_files(&page_dir);
    let pages = r
        .run("report.pages", json!({"docId": doc, "report": "Stores"}))
        .await;
    assert_eq!(pages["report"], stores["report"]);
    assert_eq!(
        pages["pages"].as_array().unwrap().len(),
        stores["pages"].as_array().unwrap().len()
    );
    assert_eq!(pages["pages"][0]["visuals"], on_first_page);
    let visuals = r
        .run(
            "report.visuals",
            json!({"docId": doc, "report": "Stores", "page": "1"}),
        )
        .await;
    assert_eq!(visuals["visuals"].as_array().unwrap().len(), on_first_page);

    let render = r
        .run("render.page", json!({"docId": doc, "report": "Stores"}))
        .await;
    let svg = render["svg"].as_str().unwrap();
    assert!(svg.starts_with("<svg"));
    assert!(svg.contains("data-hit"), "interactive groups");
    assert_eq!(render["summary"]["visuals"], on_first_page);
    assert_eq!(render["visuals"].as_array().unwrap().len(), on_first_page);
    assert_eq!(render["page"]["displayName"], first["page"]);

    let e = r
        .try_run("report.pages", json!({"docId": doc, "report": "Nope"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::NotFound);
    assert!(e.data.unwrap()["reports"].as_array().unwrap().len() == reports.len());

    let closed = r.run("doc.close", json!({"docId": doc})).await;
    assert_eq!(closed["closed"], true);
    assert_eq!(
        r.try_run("doc.inspect", json!({"docId": doc}))
            .await
            .unwrap_err()
            .code,
        OpErrorCode::NoDocument
    );

    // Lint warnings do not fail by default; `deny` makes them fail.
    let dir = tmp_dir("pbi-ops-lint-");
    let stale = acme_with_stale_query_ref(dir.path());
    let doc = r.open(json!({"path": stale, "autoRefresh": false})).await;
    let lint = r.run("doc.lint", json!({"docId": doc})).await;
    assert_eq!(lint["summary"]["warnings"], 1, "{lint}");
    assert_eq!(lint["summary"]["byRule"]["LINT003"], 1);
    assert_eq!(lint["ok"], true, "warnings do not fail by default");
    let strict = r
        .run("doc.lint", json!({"docId": doc, "deny": "warning"}))
        .await;
    assert_eq!(strict["ok"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn expected_values_through_the_registry() {
    let (Some(pbip), Some(map), Some(csv), Some(exp)) = (
        fixture(PBIP),
        fixture(BLOB_MAP),
        fixture(ORDERS_CSV),
        expected(),
    ) else {
        return;
    };
    let r = Runner::new();
    let doc = r
        .open(json!({"path": pbip, "blobMap": map, "source": "query", "autoRefresh": false}))
        .await;
    let refresh = r.run("data.refresh", json!({"docId": doc})).await;
    assert_eq!(refresh["ok"], true, "{refresh}");
    for (name, rows) in exp.row_counts() {
        assert_eq!(table(&refresh["tables"], name)["rows"], rows, "{name}");
    }

    let all = r.run("measures.values", json!({"docId": doc})).await;
    assert_eq!(all["ok"], true);
    for (name, _, formatted) in exp.measures() {
        assert_eq!(measure(&all, name)["formatted"], formatted, "{name}");
    }
    let stores = r
        .run("measures.values", json!({"docId": doc, "report": "Stores"}))
        .await;
    for (name, _, formatted) in exp.report_measures("Stores") {
        assert_eq!(measure(&stores, name)["display"], formatted, "{name}");
    }

    let q = r
        .run(
            "dax.query",
            json!({"docId": doc, "query": "EVALUATE ROW(\"n\", [Order Count])"}),
        )
        .await;
    assert_eq!(
        q["tables"][0]["rows"][0][0],
        exp.0["measures"]["unfiltered"]["Order Count"]["value"]
    );
    let one = r
        .run(
            "dax.evaluateMeasure",
            json!({"docId": doc, "measure": "[Total Sales]", "report": "Online"}),
        )
        .await;
    assert_eq!(
        one["measure"]["formatted"],
        exp.measure(Some("Online"), "Total Sales")
    );

    // The West stores: the rows the "Region West" role sees.
    let west = exp.0["rls"][0]["visibleRows"]["Stores"].clone();
    assert_eq!(exp.0["rls"][0]["roles"], json!(["Region West"]));
    let page = r
        .run(
            "data.page",
            json!({"docId": doc, "table": "Stores", "limit": 2, "filter": "\"Region\" = 'West'"}),
        )
        .await;
    assert_eq!(page["totalRows"], west);
    assert_eq!(page["rows"].as_array().unwrap().len(), 2);
    let stats = r
        .run(
            "data.columnStats",
            json!({"docId": doc, "table": "Orders", "column": "Status"}),
        )
        .await;
    assert_eq!(stats["rows"], exp.rows("Orders"));

    let render = r
        .run("render.page", json!({"docId": doc, "report": "Stores"}))
        .await;
    assert_eq!(render["ok"], true, "{}", render["visuals"]);
    let card = exp.card("Stores", "Store Performance", "kpis", "Total Sales");
    assert!(
        render["svg"]
            .as_str()
            .unwrap()
            .contains(&format!(">{card}<")),
        "no {card} on the page"
    );

    // A CSV load replaces a table and recomputes what depends on it (do it
    // last: the raw CSV changes the values).
    let loaded = r
        .run(
            "data.loadCsv",
            json!({"docId": doc, "table": "Orders", "path": csv}),
        )
        .await;
    assert_eq!(loaded["source"], "manual");
    assert_eq!(loaded["tables"][0]["name"], "Orders");
    // The raw CSV, without the M steps (filters, de-duplication).
    assert_eq!(
        loaded["tables"][0]["rows"],
        exp.0["rawCsvRows"]["orders.csv"]
    );

    let dir = tmp_dir("pbi-ops-e2e-");
    let out = dir.path().join("orders.csv");
    let exported = r
        .run(
            "export.table",
            json!({"docId": doc, "table": "Orders", "format": "csv", "path": out}),
        )
        .await;
    assert_eq!(exported["rows"], loaded["tables"][0]["rows"]);
    assert!(exported["columns"].as_u64().unwrap() > 10);
    let e = r
        .try_run(
            "export.table",
            json!({"docId": doc, "table": "Orders", "format": "csv", "path": out}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::Io, "no overwrite without overwrite");
}

/// The tables whose M reads the example's blob container.
const BLOB_TABLES: [&str; 7] = [
    "Orders",
    "Customers",
    "Products",
    "Stores",
    "Targets",
    "Returns",
    "Web Sessions",
];

#[tokio::test(flavor = "multi_thread")]
async fn offline_refresh_reports_failures() {
    let Some(pbip) = fixture(PBIP) else { return };
    let r = Runner::new();
    let doc = r
        .open(json!({"path": pbip, "source": "query", "autoRefresh": false}))
        .await;
    let refresh = r.run("data.refresh", json!({"docId": doc})).await;
    assert_eq!(refresh["ok"], false);
    let orders = table(&refresh["tables"], "Orders");
    assert_eq!(orders["status"]["state"], "error");
    assert!(
        orders["status"]["hint"]
            .as_str()
            .unwrap()
            .contains("blobMap"),
        "{}",
        orders["status"]
    );
    for name in BLOB_TABLES {
        let status = &table(&refresh["tables"], name)["status"];
        assert_eq!(status["csvFallback"], true, "{name}: {status}");
        assert_eq!(status["offline"], true, "{name}: {status}");
    }
    let e = r
        .try_run(
            "data.refresh",
            json!({"docId": doc, "tables": ["No Such Table"]}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::NotFound);

    // "Load CSV manually…" stands in for a failed table (a synthetic CSV, `;`-separated).
    let status = r.run("data.tables", json!({"docId": doc})).await;
    assert_eq!(
        table(&status["tables"], "Products Never Sold")["canLoadCsv"],
        false
    );
    let dir = tmp_dir("pbi-ops-csv-");
    let csv = dir.path().join("stores.csv");
    std::fs::write(
        &csv,
        "Store ID;Store Name;Region\nS01;North Pier;West\nS02;Lakeside;East\n;(Online);Online\n",
    )
    .unwrap();
    let options = json!({"delimiter": ";", "hasHeader": true});
    let loaded = r
        .run(
            "data.loadCsv",
            json!({"docId": doc, "table": "Stores", "path": csv, "options": options}),
        )
        .await;
    let stores = table(&loaded["tables"], "Stores");
    assert_eq!(
        (&stores["rows"], &stores["kind"]),
        (&json!(3), &json!("manual"))
    );
    let status = r.run("data.tables", json!({"docId": doc})).await;
    let stores = table(&status["tables"], "Stores");
    assert_eq!(
        (&stores["rows"], &stores["loaded"]),
        (&json!(3), &json!(true))
    );
    assert_eq!(stores["lastRefresh"]["kind"], "manual");
    // The grid's page: formatted text, BLANK as null.
    let page = r
        .run(
            "data.page",
            json!({"docId": doc, "table": "Stores", "limit": 10, "formatted": true, "blankAsNull": true}),
        )
        .await;
    let column = |name: &str| {
        page["columns"]
            .as_array()
            .unwrap()
            .iter()
            .position(|c| c["name"] == name)
            .unwrap_or_else(|| panic!("no column {name}"))
    };
    let (names, city) = (column("Store Name"), column("City"));
    let texts: Vec<&Value> = page["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| &row[names])
        .collect();
    assert_eq!(
        texts,
        [&json!("North Pier"), &json!("Lakeside"), &json!("(Online)")]
    );
    assert!(page["rows"][0][city].is_null());

    for (table_name, options) in [
        ("Products Never Sold", options.clone()),
        ("Stores", json!({"delimiter": ";;"})),
    ] {
        let e = r
            .try_run(
                "data.loadCsv",
                json!({"docId": doc, "table": table_name, "path": csv, "options": options}),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(e.code, OpErrorCode::Query | OpErrorCode::InvalidParams),
            "{table_name}: {e:?}"
        );
    }
}

/// Texts of the `<text>` elements inside the SVG group with `data-hit="<hit>"`.
fn group_texts(svg: &str, hit: &str) -> Vec<String> {
    let attr = format!("data-hit=\"{hit}\"");
    let Some(at) = svg.find(&attr) else {
        return Vec::new();
    };
    let start = svg[..at].rfind("<g").unwrap();
    let (mut depth, mut i, mut out) = (0usize, start, Vec::new());
    while i < svg.len() {
        let rest = &svg[i..];
        if rest.starts_with("<g ") || rest.starts_with("<g>") {
            depth += 1;
        } else if rest.starts_with("</g>") {
            depth -= 1;
            if depth == 0 {
                break;
            }
        } else if rest.starts_with("<text") {
            let open = rest.find('>').unwrap();
            let close = rest.find("</text>").unwrap();
            out.push(rest[open + 1..close].replace("&amp;", "&"));
        }
        i += rest.chars().next().unwrap().len_utf8();
    }
    out
}

fn ids(page: &Value, key: &str) -> Vec<String> {
    page[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_owned())
        .collect()
}

/// The interactive canvas on the example's reports: SVG pages with the expected card values,
/// the Back button's action, slicer dropdown + item selection, chart cross-filtering (and
/// clearing it), the effective filters feeding `measures.values`, table scrolling, tooltips
/// and Reset to default.
#[tokio::test(flavor = "multi_thread")]
async fn canvas_on_the_example_reports() {
    let (Some(pbip), Some(map), Some(exp)) = (fixture(PBIP), fixture(BLOB_MAP), expected()) else {
        return;
    };
    let r = Runner::new();
    let doc = r
        .open(json!({"path": pbip, "blobMap": map, "source": "query", "autoRefresh": false}))
        .await;
    let refresh = r.run("data.refresh", json!({"docId": doc})).await;
    assert_eq!(refresh["ok"], true, "{refresh}");
    let page_of = |report: &str, page: &str, extra: Value| {
        let mut body = json!({"docId": doc, "report": exp.report_name(report), "page": page});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        body
    };

    // Overview, Product Detail: the KPI cards and the Back button.
    let detail = "Product Detail";
    let at = |extra: Value| page_of("Overview", detail, extra);
    let first = r.run("canvas.render", at(json!({}))).await;
    let svg = first["svg"].as_str().unwrap();
    assert!(svg.starts_with("<svg"));
    for measure in ["Total Sales", "Units Sold", "Margin %", "Revenue per Unit"] {
        let value = exp.card("Overview", detail, "kpis", measure);
        assert!(svg.contains(&format!(">{value}<")), "{measure}: no {value}");
    }
    assert!(!svg.contains("<script") && !svg.contains("@font-face"));
    assert_eq!(
        (first["width"].as_f64(), first["height"].as_f64()),
        (Some(1280.0), Some(1080.0))
    );
    assert!(first["overlaySvg"].is_null());
    assert_eq!(first["isDefault"], true);
    let again = r.run("canvas.render", at(json!({}))).await;
    assert_eq!(again["svg"], first["svg"], "renders are deterministic");
    assert!(again["generation"].as_u64() > first["generation"].as_u64());
    let fitted = r
        .run(
            "canvas.render",
            at(json!({"viewport": {"width": 640, "height": 360}})),
        )
        .await;
    assert!(
        fitted["width"].as_f64().unwrap() <= 640.0,
        "{}",
        fitted["width"]
    );
    let button = first["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["visualType"] == "actionButton")
        .unwrap();
    assert_eq!(button["hasAction"], true);
    let hit = format!("{}|button", button["name"].as_str().unwrap());
    assert!(ids(&first, "hitRegions").contains(&hit));
    let clicked = r
        .run(
            "canvas.click",
            at(json!({"hitId": hit, "renderUnchanged": false})),
        )
        .await;
    assert_eq!(clicked["action"]["kind"], "back", "{clicked}");
    assert!(clicked["page"].is_null());
    let e = r
        .try_run("canvas.click", at(json!({"hitId": "nope"})))
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::InvalidParams);
    let e = r
        .try_run("canvas.render", page_of("Overview", "999999", json!({})))
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::NotFound);

    // Stores, Store Performance: saved slicers Region = West and 2025.
    let perf = "Store Performance";
    let at = |extra: Value| page_of("Stores", perf, extra);
    let click = |hit: Value| {
        let body = at(json!({"hitId": hit}));
        let r = &r;
        async move { r.run("canvas.click", body).await }
    };
    let kpis = exp.visual_id("Stores", perf, "kpis");
    let total =
        |page: &Value| group_texts(page["svg"].as_str().unwrap(), &format!("{kpis}|visual"));
    let west = exp.card("Stores", perf, "kpis", "Total Sales").to_owned();
    // Without the Region selection: every Retail and Outlet store in 2025, which is
    // the year-to-date total at the end of the data (2025-12-31).
    let all_regions = exp.measure(Some("Stores"), "Sales YTD").to_owned();
    let page = r.run("canvas.render", at(json!({}))).await;
    assert!(total(&page).contains(&west), "{:?}", total(&page));

    let region = exp.visual_id("Stores", perf, "region");
    let cleared = r
        .run("canvas.clearSelections", at(json!({"visual": region})))
        .await;
    assert!(
        total(&cleared).contains(&all_regions),
        "{:?}",
        total(&cleared)
    );
    assert_eq!(cleared["isDefault"], false);

    // The Region dropdown opens into the overlay; "West" selects.
    let open = click(json!(format!("{region}|button"))).await["page"].clone();
    let overlay = open["overlaySvg"].as_str().unwrap();
    let item = ids(&open, "overlayHitRegions")
        .into_iter()
        .find(|id| group_texts(overlay, id) == ["West"])
        .unwrap_or_else(|| panic!("no West item in {overlay}"));
    let state = open["interactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["visual"] == region)
        .unwrap();
    assert_eq!(
        (&state["dropdownOpen"], &state["slicerMode"]),
        (&json!(true), &json!("dropdown"))
    );
    assert!(state["popup"]["height"].as_f64().unwrap() > 0.0);
    let selected = click(json!(item)).await["page"].clone();
    assert!(total(&selected).contains(&west), "{:?}", total(&selected));
    let selections = selected["selections"].as_array().unwrap();
    let slicer = selections
        .iter()
        .find(|s| s["visual"] == region)
        .unwrap_or_else(|| panic!("no Region selection in {selections:?}"));
    assert_eq!(slicer["kind"], "slicer");
    assert_eq!(slicer["summary"], "is West");
    // Clicking outside closes the dropdown; the selection stays.
    let closed = click(Value::Null).await["page"].clone();
    assert!(closed["overlaySvg"].is_null());
    assert!(total(&closed).contains(&west));

    // The table's "Seattle Flagship" row cross-filters the cards; clicking it again clears.
    let stores = exp.visual("Stores", perf, "stores");
    let table_id = stores["visual"].as_str().unwrap().to_owned();
    let row = &stores["rows"][0];
    let store = row["Store Name"].as_str().unwrap();
    let store_sales = row["Total Sales"]["formatted"].as_str().unwrap();
    let mut cell = None;
    for id in ids(&closed, "hitRegions") {
        if !id.starts_with(&format!("{table_id}|dataPoint|")) {
            continue;
        }
        let tip = r.run("canvas.tooltip", at(json!({"hitId": id}))).await;
        if tip["lines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["value"] == store)
        {
            cell = Some(id);
            break;
        }
    }
    let cell = cell.unwrap_or_else(|| panic!("no {store} row"));
    let filtered = click(json!(cell)).await["page"].clone();
    assert!(
        total(&filtered).contains(&store_sales.to_owned()),
        "{:?}",
        total(&filtered)
    );
    let cross = filtered["selections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["kind"] == "crossFilter")
        .unwrap_or_else(|| panic!("no cross-filter in {}", filtered["selections"]));
    assert_eq!(cross["visual"], table_id.as_str());
    let unfiltered = click(json!(cell)).await["page"].clone();
    assert!(total(&unfiltered).contains(&west));
    assert!(
        unfiltered["selections"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["kind"] != "crossFilter")
    );

    // Reset to default.
    let reset = r.run("canvas.reset", at(json!({}))).await;
    assert_eq!(reset["isDefault"], true);
    assert!(total(&reset).contains(&west));

    // Overview, Executive Summary: the Year list slicer (saved: 2025). The Measures panel
    // gets the page's effective filters (all categorical here): its values match the cards.
    let summary = "Executive Summary";
    let at = |extra: Value| page_of("Overview", summary, extra);
    let click = |hit: Value| {
        let body = at(json!({"hitId": hit}));
        let r = &r;
        async move { r.run("canvas.click", body).await }
    };
    let kpis = exp.visual_id("Overview", summary, "kpis");
    let total =
        |page: &Value| group_texts(page["svg"].as_str().unwrap(), &format!("{kpis}|visual"));
    let panel = |page: &Value| {
        let body = json!({"docId": doc, "filters": page["effectiveFilters"]["filters"]});
        let r = &r;
        async move {
            let out = r.run("measures.values", body).await;
            measure(&out, "Total Sales")["display"].clone()
        }
    };
    let in_2025 = exp
        .card("Overview", summary, "kpis", "Total Sales")
        .to_owned();
    let page = r.run("canvas.render", at(json!({}))).await;
    assert!(total(&page).contains(&in_2025), "{:?}", total(&page));
    assert!(
        page["effectiveFilters"]["unsupported"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(panel(&page).await, in_2025.as_str());
    // 2024: the Year 2024 row of the Time Intelligence matrix (same report filter).
    let in_2024 = exp.visual("Overview", "Time Intelligence", "calc_matrix")["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["Year"] == 2024 || row["Year"] == "2024")
        .unwrap()["Current"]["formatted"]
        .as_str()
        .unwrap()
        .to_owned();
    let year = exp.visual_id("Overview", summary, "year");
    let item = ids(&page, "hitRegions")
        .into_iter()
        .filter(|id| id.starts_with(&format!("{year}|")))
        .find(|id| group_texts(page["svg"].as_str().unwrap(), id) == ["2024"])
        .unwrap_or_else(|| panic!("no 2024 item in the Year slicer"));
    let selected = click(json!(item)).await["page"].clone();
    assert!(
        total(&selected).contains(&in_2024),
        "{:?}",
        total(&selected)
    );
    assert_eq!(panel(&selected).await, in_2024.as_str());
    // No year: the report filter only.
    let all_years = exp.measure(Some("Overview"), "Total Sales").to_owned();
    let cleared = r
        .run("canvas.clearSelections", at(json!({"visual": year})))
        .await;
    assert!(
        total(&cleared).contains(&all_years),
        "{:?}",
        total(&cleared)
    );
    assert_eq!(panel(&cleared).await, all_years.as_str());

    // Online, Online Overview: the device pie shows only the Mobile slice (report filter on
    // the unrelated Web Sessions table); cross-filtering by it changes no sales value.
    let online = "Online Overview";
    let at = |extra: Value| page_of("Online", online, extra);
    let pie = exp.visual_id("Online", online, "devices");
    let page = r.run("canvas.render", at(json!({}))).await;
    let slice = ids(&page, "hitRegions")
        .into_iter()
        .find(|id| id.starts_with(&format!("{pie}|dataPoint|")))
        .unwrap();
    let tip = r.run("canvas.tooltip", at(json!({"hitId": slice}))).await;
    let sessions = exp.card("Online", online, "sessions", "Sessions");
    let lines = tip["lines"].as_array().unwrap();
    assert!(lines.iter().any(|l| l["value"] == "Mobile"), "{tip}");
    assert!(lines.iter().any(|l| l["value"] == sessions), "{tip}");
    let filtered = r.run("canvas.click", at(json!({"hitId": slice}))).await["page"].clone();
    assert!(
        filtered["selections"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["kind"] == "crossFilter" && s["visual"] == pie.as_str()),
        "{}",
        filtered["selections"]
    );
    let values = r
        .run(
            "measures.values",
            json!({"docId": doc, "filters": filtered["effectiveFilters"]["filters"]}),
        )
        .await;
    for (key, name) in [
        ("kpis", "Total Sales"),
        ("kpis", "Order Count"),
        ("sessions", "Sessions"),
    ] {
        assert_eq!(
            measure(&values, name)["display"],
            exp.card("Online", online, key, name),
            "{name}"
        );
    }

    // Online, Customer Detail: the customer table scrolls; its first visible row moves
    // down, and stops at the top.
    let customers = "Customer Detail";
    let at = |extra: Value| page_of("Online", customers, extra);
    let table_id = exp.visual_id("Online", customers, "table");
    let first_row = |page: &Value| {
        ids(page, "hitRegions")
            .iter()
            .filter_map(|id| {
                id.strip_prefix(&format!("{table_id}|dataPoint|"))
                    .map(str::to_owned)
            })
            .map(|rest| rest.split('|').nth(1).unwrap().parse::<u64>().unwrap())
            .min()
            .unwrap()
    };
    let page = r.run("canvas.render", at(json!({}))).await;
    assert_eq!(first_row(&page), 0);
    let scroll = |dy: f64| {
        let body = at(json!({"visual": table_id, "dy": dy, "renderUnchanged": false}));
        let r = &r;
        async move { r.run("canvas.scroll", body).await }
    };
    let top = scroll(-50.0).await;
    assert_eq!(top["changed"], false, "already at the top");
    assert!(top["page"].is_null());
    let scrolled = scroll(200.0).await["page"].clone();
    assert!(first_row(&scrolled) > 0, "{}", first_row(&scrolled));
    let range = scrolled["interactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["visual"] == table_id)
        .unwrap()["scroll"]
        .clone();
    assert_eq!(range["y"].as_f64(), Some(200.0));
    assert!(range["maxY"].as_f64().unwrap() > 200.0);
    let reset = r.run("canvas.reset", at(json!({}))).await;
    assert_eq!(reset["isDefault"], true);
    assert_eq!(first_row(&reset), 0);
}

/// A PBIP whose `.pbi/cache.abf` holds data: a committed Microsoft sample PBIX saved as a
/// PBIP with its data model as the cache (as Power BI Desktop would leave it). `doc.open`
/// announces the cache, `data.refresh` with source `cache` loads it (no M), and `query`
/// evaluates the M queries again.
#[tokio::test(flavor = "multi_thread")]
async fn pbip_local_data_cache() {
    let (Some(pbix), Some(facts)) = (fixture(MS_PBIX), ms_expected(MS_PBIX)) else {
        return;
    };
    let dir = tmp_dir("pbi-ops-cache-");
    let opened = pbi_core::open_path(&pbix, &pbi_core::OpenOptions::default()).unwrap();
    pbi_core::save_as_pbip(&opened, dir.path(), &pbi_core::SaveOptions::default()).unwrap();
    let Some(pbi_core::ModelSource::Abf { archive, .. }) = &opened.model_source else {
        panic!("the PBIX model is not read from its DataModel");
    };
    let entries: Vec<PathBuf> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    let find = |suffix: &str| {
        entries
            .iter()
            .find(|p| p.to_string_lossy().ends_with(suffix))
            .cloned()
            .unwrap()
    };
    let cache = find(".SemanticModel").join(".pbi");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("cache.abf"), archive.as_bytes()).unwrap();

    let r = Runner::new();
    let summary = r
        .run(
            "doc.open",
            json!({"path": find(".pbip"), "autoRefresh": false}),
        )
        .await;
    assert_eq!(summary["hasCache"], true, "{summary}");
    let doc = summary["docId"].as_str().unwrap().to_owned();
    let status = r.run("data.tables", json!({"docId": doc})).await;
    let info = &status["cache"];
    assert!(
        info["entry"]
            .as_str()
            .unwrap()
            .ends_with(".SemanticModel/.pbi/cache.abf"),
        "{status}"
    );
    assert_eq!(info["size"], json!(archive.as_bytes().len()));

    let report = r
        .run("data.refresh", json!({"docId": doc, "source": "cache"}))
        .await;
    assert_eq!(report["ok"], true, "{report}");
    assert!(
        report["tables"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["kind"] == "cache"),
        "{report}"
    );
    let status = r.run("data.tables", json!({"docId": doc})).await;
    for t in facts["model"]["tables"].as_array().unwrap() {
        let name = t["name"].as_str().unwrap();
        let loaded = table(&status["tables"], name);
        assert_eq!(
            (&loaded["rows"], &loaded["loaded"]),
            (&t["rows"], &json!(true)),
            "{name}"
        );
    }
    let report = r
        .run(
            "data.refresh",
            json!({"docId": doc, "tables": ["Opportunity"], "source": "query"}),
        )
        .await;
    assert_eq!(report["tables"][0]["kind"], "m", "{report}");
}

#[tokio::test(flavor = "multi_thread")]
async fn exports_and_conversions_write_files() {
    let (Some(pbip), Some(exp)) = (fixture(PBIP), expected()) else {
        return;
    };
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    let dir = tmp_dir("pbi-ops-convert-");

    let md = r
        .run("export.model", json!({"docId": doc, "format": "markdown"}))
        .await;
    assert!(
        md["markdown"]
            .as_str()
            .unwrap()
            .starts_with("# AcmeOutfitters\n")
    );
    let json_doc = r
        .run(
            "export.model",
            json!({"docId": doc, "format": "json", "includeExpressions": false}),
        )
        .await;
    assert_eq!(json_doc["json"]["schemaVersion"], 1);

    let project = dir.path().join("project");
    let pbip_out = r
        .run("convert.pbip", json!({"docId": doc, "outDir": project}))
        .await;
    assert!(pbip_out["files"].as_array().unwrap().len() > 10);
    assert!(
        project.join("AcmeOutfitters.pbip").exists()
            || pbip_out["files"].to_string().contains(".pbip")
    );
    let e = r
        .try_run("convert.pbip", json!({"docId": doc, "outDir": project}))
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::Io, "non-empty folder needs overwrite");

    let stores = exp.report("Stores");
    let pbit = dir.path().join("stores.pbit");
    let out = r
        .run(
            "convert.pbit",
            json!({"docId": doc, "outPath": pbit, "report": stores["report"]}),
        )
        .await;
    assert_eq!(out["report"], stores["report"]);
    let reports = exp.0["reports"].as_array().unwrap().len();
    assert_eq!(out["omittedReports"].as_array().unwrap().len(), reports - 1);
    let reopened = r
        .run("doc.open", json!({"path": pbit, "autoRefresh": false}))
        .await;
    assert_eq!(reopened["format"], "pbit");
    assert_eq!(reopened["reports"].as_array().unwrap().len(), 1);
    assert_eq!(
        reopened["reports"][0]["pages"],
        stores["pages"].as_array().unwrap().len(),
        "the Stores report"
    );
}
