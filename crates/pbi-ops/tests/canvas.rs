//! Data, render and canvas commands on a synthetic PBIP (inline M data, no
//! fixture files needed).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use base64::Engine as _;
use common::{MINI_TOTAL, Runner, mini_pbip, tmp_dir};
use pbi_ops::OpErrorCode;
use serde_json::{Value, json};

fn hit<'a>(page: &'a Value, prefix: &str) -> &'a str {
    page["hitRegions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|h| h["id"].as_str())
        .find(|id| id.starts_with(prefix))
        .unwrap_or_else(|| panic!("no hit region {prefix}…"))
}

fn total(r: &Value) -> &Value {
    r["measures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "Total")
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn data_and_dax_commands() {
    let dir = tmp_dir("pbi-ops-data-");
    let pbip = mini_pbip(dir.path());
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip})).await;

    // First use loads the data (autoRefresh).
    let values = r.run("measures.values", json!({"docId": doc})).await;
    assert_eq!(values["ok"], true);
    assert_eq!(total(&values)["value"], MINI_TOTAL);
    assert_eq!(values["data"]["ok"], 1);
    let east = r
        .run(
            "measures.values",
            json!({"docId": doc, "filters": [{"table": "Sales", "column": "Region", "values": ["East"]}]}),
        )
        .await;
    let east_total: i64 = (1..=60).filter(|i| i % 3 == 0).sum();
    assert_eq!(total(&east)["display"], east_total.to_string());

    let q = r
        .run(
            "dax.query",
            json!({"docId": doc, "query": "EVALUATE SUMMARIZECOLUMNS(Sales[Region], \"T\", [Total]) ORDER BY Sales[Region]", "maxRows": 2}),
        )
        .await;
    let t = &q["tables"][0];
    assert_eq!(t["totalRows"], 3);
    assert_eq!(t["truncated"], true);
    assert_eq!(t["rows"].as_array().unwrap().len(), 2);
    assert_eq!(t["columns"][0]["name"], "Sales[Region]");
    let e = r
        .try_run(
            "dax.query",
            json!({"docId": doc, "query": "EVALUATE NOPE("}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::Query);
    assert_eq!(e.exit_code(), 2);

    let page = r
        .run(
            "data.page",
            json!({"docId": doc, "table": "sales", "offset": 1, "limit": 3, "sort": {"column": "Amount", "ascending": false}}),
        )
        .await;
    assert_eq!(page["table"], "Sales");
    assert_eq!(page["totalRows"], 60);
    assert_eq!(page["rows"][0], json!(["Item 59", "North", 59]));
    let stats = r
        .run(
            "data.columnStats",
            json!({"docId": doc, "table": "Sales", "column": "amount"}),
        )
        .await;
    assert_eq!(
        (stats["min"].clone(), stats["max"].clone()),
        (json!(1), json!(60))
    );
    assert_eq!(stats["distinct"], 60);

    let added = r
        .run(
            "data.addDerivedColumn",
            json!({"docId": doc, "table": "Sales", "name": "Double", "kind": {"dax": {"expression": "Sales[Amount] * 2"}}}),
        )
        .await;
    assert_eq!(added["active"], true);
    let q = r
        .run(
            "dax.query",
            json!({"docId": doc, "query": "EVALUATE ROW(\"d\", SUM(Sales[Double]))"}),
        )
        .await;
    assert_eq!(
        q["tables"][0]["rows"][0][0].as_f64(),
        Some((2 * MINI_TOTAL) as f64)
    );
    let status = r.run("data.tables", json!({"docId": doc})).await;
    assert_eq!(status["derived"][0]["name"], "Double");
    assert_eq!(status["tables"][0]["rows"], 60);
    r.run(
        "data.removeDerivedColumn",
        json!({"docId": doc, "table": "Sales", "name": "Double"}),
    )
    .await;
    let e = r
        .try_run(
            "data.addDerivedColumn",
            json!({"docId": doc, "table": "Sales", "name": "Amount", "kind": {"people": {"sourceColumn": "Item"}}}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::InvalidParams, "name collision");

    let people = r
        .run(
            "text.parsePeople",
            json!({"text": "[{'firstName': 'Ada', 'lastName': 'Lovelace'}, {'firstName': 'Alan', 'lastName': 'Turing'}]"}),
        )
        .await;
    assert_eq!(people["names"], json!(["Ada Lovelace", "Alan Turing"]));
    assert_eq!(people["display"], "Ada Lovelace; Alan Turing");
}

#[tokio::test(flavor = "multi_thread")]
async fn render_pages_and_visuals() {
    let dir = tmp_dir("pbi-ops-render-");
    let pbip = mini_pbip(dir.path());
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip})).await;

    let svg = r.run("render.page", json!({"docId": doc})).await;
    assert_eq!(svg["ok"], true, "{}", svg["visuals"]);
    assert_eq!(svg["page"]["name"], "overview");
    assert!(
        svg["svg"]
            .as_str()
            .unwrap()
            .contains(&format!(">{MINI_TOTAL}<"))
    );

    let filtered = r
        .run(
            "render.page",
            json!({"docId": doc, "slicers": [{"field": "Sales[Region]", "values": ["West"]}]}),
        )
        .await;
    let west: i64 = (1..=60).filter(|i| i % 3 == 1).sum();
    assert!(
        filtered["svg"]
            .as_str()
            .unwrap()
            .contains(&format!(">{west}<"))
    );
    assert_eq!(filtered["slicers"][0]["origin"], "params");

    let png = r
        .run(
            "render.visual",
            json!({"docId": doc, "visual": "total", "format": "png", "scale": 2}),
        )
        .await;
    assert_eq!(
        (png["pixelWidth"].clone(), png["pixelHeight"].clone()),
        (json!(360), json!(180))
    );
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(png["pngBase64"].as_str().unwrap())
        .unwrap();
    assert_eq!(&bytes[..4], b"\x89PNG");
    assert_eq!(png["visuals"].as_array().unwrap().len(), 1);

    let out = dir.path().join("out/page.png");
    let written = r
        .run(
            "render.page",
            json!({"docId": doc, "page": "Details", "format": "png", "outputPath": out}),
        )
        .await;
    assert!(written.get("pngBase64").is_none());
    assert_eq!(
        std::fs::metadata(&out).unwrap().len(),
        written["bytes"].as_u64().unwrap()
    );

    for bad in [
        json!({"docId": doc, "format": "png", "scale": 0}),
        json!({"docId": doc, "slicers": [{"field": "Nope", "values": []}]}),
        json!({"docId": doc, "slicers": [{"visual": "region", "field": "Region"}]}),
        json!({"docId": doc, "page": "9"}),
    ] {
        assert!(
            r.try_run("render.page", bad.clone()).await.is_err(),
            "{bad}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn canvas_interactions() {
    let dir = tmp_dir("pbi-ops-canvas-");
    let pbip = mini_pbip(dir.path());
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip})).await;

    let page = r.run("canvas.render", json!({"docId": doc})).await;
    assert_eq!(page["isDefault"], true);
    assert_eq!(page["generation"], 1);
    assert!(
        page["svg"]
            .as_str()
            .unwrap()
            .contains(&format!(">{MINI_TOTAL}<"))
    );
    let slicer = page["interactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["visual"] == "region")
        .unwrap();
    assert_eq!(slicer["kind"], "slicer");

    // Click a slicer item: the slicer filters the page.
    let item = hit(&page, "region|slicerItem|").to_owned();
    let clicked = r
        .run("canvas.click", json!({"docId": doc, "hitId": item}))
        .await;
    assert_eq!(clicked["changed"], true);
    let p = &clicked["page"];
    assert_eq!(p["isDefault"], false);
    assert_eq!(p["selections"][0]["kind"], "slicer");
    assert_eq!(p["effectiveFilters"]["filters"][0]["column"], "Region");
    assert!(p["generation"].as_u64().unwrap() > 1);

    // Click a bar: a cross-filter.
    let bar = hit(p, "bars|dataPoint|").to_owned();
    let crossed = r
        .run("canvas.click", json!({"docId": doc, "hitId": bar}))
        .await;
    let kinds: Vec<&Value> = crossed["page"]["selections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| &s["kind"])
        .collect();
    assert!(kinds.contains(&&json!("crossFilter")), "{kinds:?}");
    let tip = r
        .run("canvas.tooltip", json!({"docId": doc, "hitId": bar}))
        .await;
    assert!(!tip["lines"].as_array().unwrap().is_empty());

    // Empty canvas click clears the cross-filter; reset restores the saved
    // state.
    let empty = r
        .run("canvas.click", json!({"docId": doc, "hitId": null}))
        .await;
    assert_eq!(empty["page"]["selections"].as_array().unwrap().len(), 1);
    let state = r.run("canvas.state", json!({"docId": doc})).await;
    assert_eq!(state["isDefault"], false);
    let reset = r.run("canvas.reset", json!({"docId": doc})).await;
    assert_eq!(reset["isDefault"], true);
    assert!(reset["selections"].as_array().unwrap().is_empty());

    // setSlicer by field, then clear it.
    let set = r
        .run(
            "canvas.setSlicer",
            json!({"docId": doc, "field": "Region", "values": ["North"], "invert": true}),
        )
        .await;
    assert!(
        set["selections"][0]["summary"]
            .as_str()
            .unwrap()
            .starts_with("is not")
    );
    let cleared = r
        .run(
            "canvas.clearSelections",
            json!({"docId": doc, "visual": "region"}),
        )
        .await;
    assert!(cleared["selections"].as_array().unwrap().is_empty());

    // Scroll the long table on the other page (rendered on first use).
    let scrolled = r
        .run(
            "canvas.scroll",
            json!({"docId": doc, "page": "Details", "visual": "table", "dy": 100}),
        )
        .await;
    assert_eq!(scrolled["changed"], true);
    let y = scrolled["page"]["interactions"][0]["scroll"]["y"]
        .as_f64()
        .unwrap();
    assert!(y > 0.0, "{}", scrolled["page"]["interactions"]);
    let back = r
        .run(
            "canvas.scroll",
            json!({"docId": doc, "page": "Details", "visual": "table", "dy": -1000, "renderUnchanged": false}),
        )
        .await;
    assert_eq!(back["changed"], true);
    let again = r
        .run(
            "canvas.scroll",
            json!({"docId": doc, "page": "Details", "visual": "table", "dy": -10, "renderUnchanged": false}),
        )
        .await;
    assert_eq!(again["changed"], false);
    assert!(again["page"].is_null());

    let e = r
        .try_run("canvas.click", json!({"docId": doc, "hitId": "nonsense"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::InvalidParams);
    let e = r
        .try_run(
            "canvas.click",
            json!({"docId": doc, "hitId": "ghost|visual"}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::NotFound);
}

/// A cross-filter survives renames of its column and table, and their undo.
#[tokio::test(flavor = "multi_thread")]
async fn canvas_selections_follow_renames() {
    let dir = tmp_dir("pbi-ops-canvas-rename-");
    let pbip = mini_pbip(dir.path());
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip})).await;
    let unfiltered = format!(">{MINI_TOTAL}<");

    let page = r.run("canvas.render", json!({"docId": doc})).await;
    let bar = hit(&page, "bars|dataPoint|").to_owned();
    let clicked = r
        .run("canvas.click", json!({"docId": doc, "hitId": bar}))
        .await;
    let filtered = clicked["page"]["svg"].as_str().unwrap().to_owned();
    assert!(!filtered.contains(&unfiltered));
    let field =
        |state: &Value| state["interaction"]["selectedPoints"]["points"][0][0]["field"].clone();
    let before = field(&r.run("canvas.state", json!({"docId": doc})).await);
    assert_eq!(before["column"]["property"], "Region", "{before}");

    let check = |page: &Value, step: &str| {
        let svg = page["svg"].as_str().unwrap();
        assert!(
            !svg.contains(&unfiltered),
            "{step}: the cross-filter is lost"
        );
        let kinds: Vec<&Value> = page["selections"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| &s["kind"])
            .collect();
        assert_eq!(kinds, [&json!("crossFilter")], "{step}");
    };
    r.run(
        "model.object.rename",
        json!({"docId": doc, "table": "Sales", "column": "Region", "newName": "Area"}),
    )
    .await;
    check(
        &r.run("canvas.render", json!({"docId": doc})).await,
        "column rename",
    );
    r.run(
        "model.object.rename",
        json!({"docId": doc, "table": "Sales", "newName": "Orders"}),
    )
    .await;
    let page = r.run("canvas.render", json!({"docId": doc})).await;
    check(&page, "table rename");
    assert_eq!(page["isDefault"], false);
    let renamed = field(&r.run("canvas.state", json!({"docId": doc})).await);
    assert_eq!(renamed["column"]["property"], "Area", "{renamed}");
    assert_eq!(renamed["column"]["entity"], "Orders", "{renamed}");
    // The bar's selection still highlights the same data point.
    let again = r
        .run("canvas.click", json!({"docId": doc, "hitId": bar}))
        .await;
    assert_eq!(
        again["changed"], true,
        "a second click on the bar clears it"
    );
    assert!(again["page"]["svg"].as_str().unwrap().contains(&unfiltered));
    r.run("canvas.click", json!({"docId": doc, "hitId": bar}))
        .await;

    r.run("doc.undo", json!({"docId": doc})).await;
    r.run("doc.undo", json!({"docId": doc})).await;
    check(&r.run("canvas.render", json!({"docId": doc})).await, "undo");
    assert_eq!(
        field(&r.run("canvas.state", json!({"docId": doc})).await),
        before
    );
    r.run("doc.redo", json!({"docId": doc})).await;
    check(&r.run("canvas.render", json!({"docId": doc})).await, "redo");

    // A click on the render from before a rename (nothing rendered since)
    // uses today's names: it toggles the selected bar off, and on again.
    r.run(
        "model.object.rename",
        json!({"docId": doc, "table": "Sales", "column": "Area", "newName": "Zone"}),
    )
    .await;
    r.run("canvas.click", json!({"docId": doc, "hitId": bar}))
        .await;
    let state = r.run("canvas.state", json!({"docId": doc})).await;
    assert!(state["interaction"]["selectedPoints"].is_null(), "{state}");
    let on = r
        .run("canvas.click", json!({"docId": doc, "hitId": bar}))
        .await;
    check(&on["page"], "click after a rename");
    let zone = field(&r.run("canvas.state", json!({"docId": doc})).await);
    assert_eq!(zone["column"]["property"], "Zone", "{zone}");
}

#[tokio::test(flavor = "multi_thread")]
async fn scatter_tooltips_show_details_legend_x_y_and_size() {
    let dir = tmp_dir("pbi-ops-scatter-");
    let pbip = mini_pbip(dir.path());
    let tmdl = dir
        .path()
        .join("Mini.SemanticModel/definition/tables/Sales.tmdl");
    let sales = std::fs::read_to_string(&tmdl).unwrap().replacen(
        "\tcolumn Item\n",
        "\tmeasure Double = [Total] * 2\n\t\tformatString: 0\n\n\
         \tmeasure Ten = [Total] * 10\n\t\tformatString: 0.0\n\n\tcolumn Item\n",
        1,
    );
    std::fs::write(&tmdl, sales).unwrap();
    let field = |kind: &str, table: &str, name: &str| json!({kind: {"Expression": {"SourceRef": {"Entity": table}}, "Property": name}});
    let projection = |field: Value, name: &str| json!({"field": field, "queryRef": format!("Sales.{name}"), "nativeQueryRef": name, "active": true});
    let visual = dir
        .path()
        .join("Mini.Report/definition/pages/details/visuals/scatter/visual.json");
    std::fs::create_dir_all(visual.parent().unwrap()).unwrap();
    let body = json!({
        "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/visualContainer/2.4.0/schema.json",
        "name": "scatter",
        "position": {"x": 320, "y": 10, "z": 0, "width": 310, "height": 340, "tabOrder": 1},
        "visual": {"visualType": "scatterChart", "query": {"queryState": {
            "Category": {"projections": [projection(field("Column", "Sales", "Item"), "Item")]},
            "Series": {"projections": [projection(field("Column", "Sales", "Region"), "Region")]},
            "X": {"projections": [projection(field("Measure", "Sales", "Total"), "Total")]},
            "Y": {"projections": [projection(field("Measure", "Sales", "Double"), "Double")]},
            "Size": {"projections": [projection(field("Measure", "Sales", "Ten"), "Ten")]}
        }}}
    });
    std::fs::write(&visual, body.to_string()).unwrap();

    let r = Runner::new();
    let doc = r.open(json!({"path": pbip})).await;
    let page = r
        .run("canvas.render", json!({"docId": doc, "page": "Details"}))
        .await;
    let point = hit(&page, "scatter|dataPoint|").to_owned();
    let tip = r
        .run(
            "canvas.tooltip",
            json!({"docId": doc, "page": "Details", "hitId": point}),
        )
        .await;
    let lines: Vec<(&str, &str)> = tip["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| (l["label"].as_str().unwrap(), l["value"].as_str().unwrap()))
        .collect();
    let labels: Vec<&str> = lines.iter().map(|l| l.0).collect();
    assert_eq!(
        labels,
        ["Item", "Region", "Total", "Double", "Ten"],
        "{lines:?}"
    );
    let n: i64 = lines[0].1.strip_prefix("Item ").unwrap().parse().unwrap();
    assert_eq!(lines[1].1, ["East", "West", "North"][(n % 3) as usize]);
    assert_eq!(lines[2].1, n.to_string());
    assert_eq!(lines[3].1, (2 * n).to_string());
    assert_eq!(lines[4].1, format!("{}.0", 10 * n));
}

#[tokio::test(flavor = "multi_thread")]
async fn app_facing_additions() {
    let dir = tmp_dir("pbi-ops-app-");
    let pbip = mini_pbip(dir.path());
    let r = Runner::new();
    // The host's open hook runs for opens without a blob map (the app installs its blob
    // resolver this way).
    let hooked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = std::sync::Arc::clone(&hooked);
    r.ws.set_open_hook(Some(std::sync::Arc::new(move |_options| {
        seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    })));
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    assert_eq!(hooked.load(std::sync::atomic::Ordering::Relaxed), 1);

    // Without autoRefresh nothing is loaded until data.refresh.
    let status = r.run("data.tables", json!({"docId": doc})).await;
    assert_eq!(status["tables"][0]["loaded"], false);
    assert_eq!(status["tables"][0]["canLoadCsv"], true);
    let refreshed = r
        .run("data.refresh", json!({"docId": doc, "source": "query"}))
        .await;
    assert_eq!(refreshed["ok"], true);

    // Formatted cells with BLANK as null; text cells stay text.
    let page = r
        .run(
            "data.page",
            json!({"docId": doc, "table": "Sales", "limit": 2, "formatted": true, "blankAsNull": true}),
        )
        .await;
    assert_eq!(page["rows"][0][0], "Item 01");
    assert!(page["rows"][0][2].is_string());
    let q = r
        .run(
            "dax.query",
            json!({"docId": doc, "query": "EVALUATE ROW(\"b\", BLANK(), \"n\", 1)", "formatted": true, "blankAsNull": true}),
        )
        .await;
    assert_eq!(q["tables"][0]["rows"][0], json!([null, "1"]));
    let q = r
        .run(
            "dax.query",
            json!({"docId": doc, "query": "EVALUATE ROW(\"b\", BLANK())", "formatted": true}),
        )
        .await;
    assert_eq!(
        q["tables"][0]["rows"][0][0], "",
        "BLANK formats as empty text"
    );

    // report.filters: the mini report has no filters; the page must exist.
    let filters = r
        .run("report.filters", json!({"docId": doc, "page": "Details"}))
        .await;
    assert_eq!(filters["page"], "details");
    assert_eq!(
        filters["filters"],
        json!({"filters": [], "unsupported": []})
    );
    let e = r
        .try_run("report.filters", json!({"docId": doc, "page": "9"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::NotFound);

    // text.previewPeople over a column: counts and distinct samples.
    let preview = r
        .run(
            "text.previewPeople",
            json!({"docId": doc, "table": "sales", "column": "item", "limit": 3}),
        )
        .await;
    assert_eq!(preview["table"], "Sales");
    assert_eq!(preview["column"], "Item");
    assert_eq!(preview["newColumn"], "Item (parsed)");
    assert_eq!(
        (preview["rows"].clone(), preview["unparsed"].clone()),
        (json!(60), json!(60))
    );
    assert_eq!(preview["samples"].as_array().unwrap().len(), 3);
    assert_eq!(preview["samples"][0]["outcome"], "unparsed");
    let e = r
        .try_run(
            "text.previewPeople",
            json!({"docId": doc, "table": "Sales", "column": "Amount"}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::Query);
    let e = r
        .try_run(
            "text.previewPeople",
            json!({"docId": doc, "table": "Sales", "column": "Item", "limit": 201}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::InvalidParams);
}

/// "Parse people field" on a synthetic model (moved from the app's IPC tests): preview,
/// derived column (grid, sort, filter, statistics), survival across a refresh, removal.
#[tokio::test(flavor = "multi_thread")]
async fn people_field_into_a_derived_column() {
    let m = r#"#table({"App", "Owner", "Size"}, {{"A", "[{'firstName': 'Ada', 'lastName': 'Lovelace'}]", 1}, {"B", "[{'firstName': 'Alan', 'lastName': 'Turing'}, {'firstName': 'Grace', 'lastName': 'Hopper'}]", 2}, {"C", "", 3}, {"D", "not a list", 4}})"#;
    let bim = json!({
        "compatibilityLevel": 1567,
        "model": { "culture": "en-US", "tables": [{
            "name": "Owners",
            "columns": [
                { "name": "App", "dataType": "string", "sourceColumn": "App" },
                { "name": "Owner", "dataType": "string", "sourceColumn": "Owner" },
                { "name": "Size", "dataType": "int64", "sourceColumn": "Size" }
            ],
            "partitions": [{ "name": "Owners", "mode": "import",
                             "source": { "type": "m", "expression": m } }]
        }]}
    });
    let dir = tmp_dir("pbi-ops-people-");
    let path = dir.path().join("people.bim");
    std::fs::write(&path, bim.to_string()).unwrap();
    let r = Runner::new();
    let doc = r.open(json!({"path": path, "autoRefresh": false})).await;
    let report = r.run("data.refresh", json!({"docId": doc})).await;
    assert_eq!(report["ok"], true, "{report}");
    let q = r
        .run(
            "dax.query",
            json!({"docId": doc, "query": "EVALUATE ROW(\"rows\", COUNTROWS(Owners), \"max\", MAX(Owners[Size]))", "formatted": true}),
        )
        .await;
    assert_eq!(q["tables"][0]["rows"][0], json!(["4", "4"]));

    let preview = r
        .run(
            "text.previewPeople",
            json!({"docId": doc, "table": "owners", "column": "owner", "limit": 10}),
        )
        .await;
    assert_eq!(preview["column"], "Owner");
    assert_eq!(preview["newColumn"], "Owner (parsed)");
    assert_eq!(
        (
            &preview["rows"],
            &preview["names"],
            &preview["unparsed"],
            &preview["empty"]
        ),
        (&json!(4), &json!(2), &json!(1), &json!(1))
    );
    let samples = preview["samples"].as_array().unwrap();
    assert_eq!(samples[0]["parsed"], "Ada Lovelace");
    assert_eq!(samples[1]["parsed"], "Alan Turing; Grace Hopper");
    assert_eq!(samples[2]["outcome"], "unparsed");

    let added = r
        .run(
            "data.addDerivedColumn",
            json!({"docId": doc, "table": "Owners", "name": "Owner (parsed)", "kind": {"people": {"sourceColumn": "Owner"}}}),
        )
        .await;
    assert_eq!(added["active"], true, "{added}");
    assert_eq!(added["sourceColumn"], "Owner");

    let texts = |page: &Value, name: &str| -> Vec<Value> {
        let i = page["columns"]
            .as_array()
            .unwrap()
            .iter()
            .position(|c| c["name"] == name)
            .unwrap_or_else(|| panic!("no column {name}"));
        page["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row[i].clone())
            .collect()
    };
    let check_grid = || async {
        let page = r
            .run(
                "data.page",
                json!({"docId": doc, "table": "Owners", "limit": 10, "formatted": true, "blankAsNull": true,
                       "sort": {"column": "Owner (parsed)", "ascending": true}}),
            )
            .await;
        assert_eq!(
            page["columns"].as_array().unwrap().last().unwrap()["name"],
            "Owner (parsed)"
        );
        assert_eq!(
            texts(&page, "Owner (parsed)"),
            [
                Value::Null,
                json!("Ada Lovelace"),
                json!("Alan Turing; Grace Hopper"),
                json!("not a list")
            ]
        );
        assert_eq!(
            texts(&page, "App"),
            [json!("C"), json!("A"), json!("B"), json!("D")]
        );
    };
    check_grid().await;
    let filtered = r
        .run(
            "data.page",
            json!({"docId": doc, "table": "Owners", "filter": "\"Owner (parsed)\" LIKE '%Hopper%'"}),
        )
        .await;
    assert_eq!(filtered["totalRows"], 1);
    let stats = r
        .run(
            "data.columnStats",
            json!({"docId": doc, "table": "Owners", "column": "Owner (parsed)"}),
        )
        .await;
    assert_eq!(
        (&stats["blanks"], &stats["distinct"]),
        (&json!(1), &json!(3))
    );

    // The derived column is recomputed from the refreshed data.
    r.run("data.refresh", json!({"docId": doc, "tables": ["Owners"]}))
        .await;
    check_grid().await;
    let status = r.run("data.tables", json!({"docId": doc})).await;
    assert_eq!(status["derived"][0]["name"], "Owner (parsed)");

    r.run(
        "data.removeDerivedColumn",
        json!({"docId": doc, "table": "Owners", "name": "Owner (parsed)"}),
    )
    .await;
    let status = r.run("data.tables", json!({"docId": doc})).await;
    assert!(status["derived"].as_array().unwrap().is_empty());
    let page = r
        .run("data.page", json!({"docId": doc, "table": "Owners"}))
        .await;
    assert_eq!(page["columns"].as_array().unwrap().len(), 3);
}

/// Rewrites `visual.json` of `page`/`name` in the [`mini_pbip`] project.
fn write_visual(dir: &std::path::Path, page: &str, name: &str, hidden: bool, visual: Value) {
    let path = dir.join(format!(
        "Mini.Report/definition/pages/{page}/visuals/{name}/visual.json"
    ));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let body = json!({
        "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/visualContainer/2.4.0/schema.json",
        "name": name,
        "position": {"x": 400, "y": 10, "z": 0, "width": 200, "height": 90, "tabOrder": 0},
        "visual": visual,
        "isHidden": hidden
    });
    std::fs::write(path, serde_json::to_string_pretty(&body).unwrap()).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn synced_slicers_share_their_selection_across_pages() {
    let dir = tmp_dir("pbi-ops-sync-");
    let pbip = mini_pbip(dir.path());
    let slicer = |filter_changes: bool| {
        json!({
            "visualType": "slicer",
            "query": {"queryState": {"Values": {"projections": [{
                "field": {"Column": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Region"}},
                "queryRef": "Sales.Region", "nativeQueryRef": "Region", "active": true
            }]}}},
            "syncGroup": {"groupName": "Region", "fieldChanges": true, "filterChanges": filter_changes}
        })
    };
    write_visual(dir.path(), "overview", "region", false, slicer(true));
    write_visual(dir.path(), "details", "synced", true, slicer(true));
    write_visual(dir.path(), "details", "unsynced", false, slicer(false));
    write_visual(
        dir.path(),
        "overview",
        "clear",
        false,
        json!({"visualType": "actionButton", "visualContainerObjects": {"visualLink": [{"properties": {
            "show": {"expr": {"Literal": {"Value": "true"}}},
            "type": {"expr": {"Literal": {"Value": "'ClearAllSlicers'"}}}
        }}]}}),
    );
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip})).await;
    let details = json!({"docId": doc, "page": "Details"});

    // A click on the Overview slicer selects the same region in the hidden
    // synced slicer of Details, which filters that page.
    let page = r.run("canvas.render", json!({"docId": doc})).await;
    let item = hit(&page, "region|slicerItem|").to_owned();
    let clicked = r
        .run("canvas.click", json!({"docId": doc, "hitId": item}))
        .await;
    assert_eq!(clicked["changed"], true);
    let state = r.run("canvas.state", details.clone()).await;
    let selections = &state["interaction"]["slicerSelections"];
    assert_eq!(
        selections["synced"],
        r.run("canvas.state", json!({"docId": doc})).await["interaction"]["slicerSelections"]["region"],
        "{state}"
    );
    assert!(selections.get("unsynced").is_none(), "{state}");
    let rendered = r
        .run("canvas.render", json!({"docId": doc, "page": "Details"}))
        .await;
    assert_eq!(
        rendered["effectiveFilters"]["filters"][0]["column"], "Region",
        "{rendered}"
    );

    // setSlicer on Details carries back to Overview; the unsynced slicer
    // keeps its own selection.
    r.run(
        "canvas.setSlicer",
        json!({"docId": doc, "page": "Details", "visual": "synced", "values": ["North"]}),
    )
    .await;
    r.run(
        "canvas.setSlicer",
        json!({"docId": doc, "page": "Details", "visual": "unsynced", "values": ["West"]}),
    )
    .await;
    let overview = r.run("canvas.state", json!({"docId": doc})).await;
    assert_eq!(
        overview["interaction"]["slicerSelections"]["region"]["values"],
        json!([["North"]]),
        "{overview}"
    );

    // Clearing one clears the others.
    r.run(
        "canvas.clearSelections",
        json!({"docId": doc, "visual": "region"}),
    )
    .await;
    let state = r.run("canvas.state", details.clone()).await;
    let selections = &state["interaction"]["slicerSelections"];
    assert!(selections.get("synced").is_none(), "{state}");
    assert_eq!(
        selections["unsynced"]["values"],
        json!([["West"]]),
        "{state}"
    );

    // So does the Clear all slicers button.
    r.run(
        "canvas.setSlicer",
        json!({"docId": doc, "visual": "region", "values": ["East"]}),
    )
    .await;
    let page = r.run("canvas.render", json!({"docId": doc})).await;
    let button = hit(&page, "clear|button").to_owned();
    r.run("canvas.click", json!({"docId": doc, "hitId": button}))
        .await;
    let state = r.run("canvas.state", details.clone()).await;
    assert!(
        state["interaction"]["slicerSelections"]
            .get("synced")
            .is_none(),
        "{state}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn drillthrough_filters_the_target_page() {
    let dir = tmp_dir("pbi-ops-drill-");
    let pbip = mini_pbip(dir.path());
    let region =
        json!({"Column": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Region"}});
    // Details drills on Region; Desktop saves the last drilled value (East).
    let page = json!({
        "name": "details", "displayName": "Details", "displayOption": "FitToPage",
        "width": 640, "height": 360,
        "filterConfig": {"filters": [{
            "name": "drill", "field": region, "type": "Categorical", "howCreated": "Drillthrough",
            "filter": {"Version": 2, "From": [{"Name": "s", "Entity": "Sales", "Type": 0}],
                "Where": [{"Condition": {"In": {
                    "Expressions": [{"Column": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "Region"}}],
                    "Values": [[{"Literal": {"Value": "'East'"}}]]
                }}}]}
        }]},
        "pageBinding": {"name": "binding", "type": "Drillthrough", "acceptsFilterContext": "Default",
            "parameters": [{"name": "p", "boundFilter": "drill", "fieldExpr": region}]}
    });
    std::fs::write(
        dir.path()
            .join("Mini.Report/definition/pages/details/page.json"),
        serde_json::to_string_pretty(&page).unwrap(),
    )
    .unwrap();
    write_visual(
        dir.path(),
        "details",
        "sum",
        false,
        json!({"visualType": "cardVisual", "query": {"queryState": {"Data": {"projections": [{
            "field": {"Measure": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Total"}},
            "queryRef": "Sales.Total", "nativeQueryRef": "Total", "active": true
        }]}}}}),
    );
    write_visual(
        dir.path(),
        "overview",
        "drill",
        false,
        json!({"visualType": "actionButton", "visualContainerObjects": {"visualLink": [{"properties": {
            "show": {"expr": {"Literal": {"Value": "true"}}},
            "type": {"expr": {"Literal": {"Value": "'Drillthrough'"}}},
            "drillthroughSection": {"expr": {"Literal": {"Value": "'details'"}}}
        }}]}}),
    );
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip})).await;
    let overview = r.run("canvas.render", json!({"docId": doc})).await;

    // The West bar (items 1, 4, …, 58: 590 in total).
    let mut west = None;
    for h in overview["hitRegions"].as_array().unwrap() {
        let id = h["id"].as_str().unwrap();
        if !id.starts_with("bars|dataPoint|") {
            continue;
        }
        let tip = r
            .run("canvas.tooltip", json!({"docId": doc, "hitId": id}))
            .await;
        if tip["lines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["value"] == "West")
        {
            west = Some(id.to_owned());
        }
    }
    let west = west.expect("a West bar");

    // The drillthrough button needs a selection.
    let button = hit(&overview, "drill|button").to_owned();
    let none = r
        .run("canvas.click", json!({"docId": doc, "hitId": button}))
        .await;
    assert_eq!(none["action"]["kind"], "notAvailable", "{}", none["action"]);

    // Targets of a bar: Details; none for the card.
    let targets = r
        .run(
            "canvas.drillthroughTargets",
            json!({"docId": doc, "hitId": west}),
        )
        .await;
    assert_eq!(targets["targets"][0]["page"], "details", "{targets}");
    assert_eq!(targets["targets"][0]["fields"], json!(["Sales.Region"]));
    assert_eq!(targets["targets"][0]["keepAllFilters"], true);
    let card = hit(&overview, "total|").to_owned();
    let none = r
        .run(
            "canvas.drillthroughTargets",
            json!({"docId": doc, "hitId": card}),
        )
        .await;
    assert_eq!(none["targets"], json!([]));

    // Drilling replaces the saved East value with West.
    let drilled = r
        .run(
            "canvas.drillthrough",
            json!({"docId": doc, "hitId": west, "target": "Details"}),
        )
        .await;
    assert_eq!(drilled["page"], "details");
    let rendered = &drilled["rendered"];
    assert!(
        rendered["svg"].as_str().unwrap().contains(">590<"),
        "{}",
        rendered["visuals"]
    );
    let kinds: Vec<&Value> = rendered["selections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| &s["kind"])
        .collect();
    assert!(kinds.contains(&&json!("drillthrough")), "{kinds:?}");

    // Overview is not a drillthrough page.
    let err = r
        .try_run(
            "canvas.drillthrough",
            json!({"docId": doc, "page": "Details", "target": "Overview"}),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, OpErrorCode::InvalidParams);

    // With a selected bar, the button drills through.
    r.run("canvas.click", json!({"docId": doc, "hitId": west}))
        .await;
    let clicked = r
        .run("canvas.click", json!({"docId": doc, "hitId": button}))
        .await;
    assert_eq!(
        clicked["action"]["kind"], "drillthrough",
        "{}",
        clicked["action"]
    );
    assert_eq!(clicked["action"]["displayName"], "Details");
    let state = r
        .run("canvas.state", json!({"docId": doc, "page": "Details"}))
        .await;
    assert_eq!(
        state["interaction"]["drillthrough"]["sourcePage"], "overview",
        "{state}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tooltips_of_highlighted_charts_add_the_highlighted_value() {
    let dir = tmp_dir("pbi-ops-highlight-");
    let pbip = mini_pbip(dir.path());
    let field = |property: &str| json!({"Column": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": property}});
    write_visual(
        dir.path(),
        "overview",
        "items",
        false,
        json!({"visualType": "clusteredColumnChart", "query": {"queryState": {
            "Category": {"projections": [{"field": field("Item"), "queryRef": "Sales.Item", "nativeQueryRef": "Item"}]},
            "Y": {"projections": [{
                "field": {"Aggregation": {"Expression": field("Amount"), "Function": 0}},
                "queryRef": "Sum(Sales.Amount)", "nativeQueryRef": "Sum of Amount"
            }]}
        }}}),
    );
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip})).await;
    let labels = |tip: &Value| -> Vec<String> {
        tip["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["label"].as_str().unwrap().to_owned())
            .collect()
    };
    let page = r.run("canvas.render", json!({"docId": doc})).await;
    let column = hit(&page, "items|dataPoint|").to_owned();
    let tip = r
        .run("canvas.tooltip", json!({"docId": doc, "hitId": column}))
        .await;
    assert!(!labels(&tip).contains(&"Highlighted".to_owned()), "{tip}");

    // A bar selected in the other chart highlights the columns (the default
    // interaction): their tooltips add the highlighted value.
    let bar = hit(&page, "bars|dataPoint|").to_owned();
    let crossed = r
        .run("canvas.click", json!({"docId": doc, "hitId": bar}))
        .await;
    let column = hit(&crossed["page"], "items|dataPoint|").to_owned();
    let tip = r
        .run("canvas.tooltip", json!({"docId": doc, "hitId": column}))
        .await;
    assert_eq!(
        labels(&tip),
        ["Item", "Sum of Amount", "Highlighted"],
        "{tip}"
    );
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cross_report_drillthrough_goes_to_another_report() {
    let dir = tmp_dir("pbi-ops-xdrill-");
    let pbip = mini_pbip(dir.path());
    write_visual(
        dir.path(),
        "details",
        "sum",
        false,
        json!({"visualType": "cardVisual", "query": {"queryState": {"Data": {"projections": [{
            "field": {"Measure": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Total"}},
            "queryRef": "Sales.Total", "nativeQueryRef": "Total", "active": true
        }]}}}}),
    );
    // A second report over the same model whose Details page accepts
    // drillthrough from other reports.
    let detail = dir.path().join("Detail.Report");
    copy_dir(&dir.path().join("Mini.Report"), &detail);
    std::fs::write(
        dir.path().join("Mini.pbip"),
        json!({"version": "1.0", "artifacts": [
            {"report": {"path": "Mini.Report"}}, {"report": {"path": "Detail.Report"}}
        ]})
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        detail.join("definition/report.json"),
        json!({
            "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/report/3.0.0/schema.json",
            "settings": {"useCrossReportDrillthrough": true}
        })
        .to_string(),
    )
    .unwrap();
    let region =
        json!({"Column": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Region"}});
    std::fs::write(
        detail.join("definition/pages/details/page.json"),
        json!({
            "name": "details", "displayName": "Details", "displayOption": "FitToPage",
            "width": 640, "height": 360,
            "filterConfig": {"filters": [{"name": "drill", "field": region, "type": "Categorical"}]},
            "pageBinding": {"name": "binding", "type": "Drillthrough", "referenceScope": "CrossReport",
                "parameters": [{"name": "p", "boundFilter": "drill", "fieldExpr": region}]}
        })
        .to_string(),
    )
    .unwrap();
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip})).await;
    let overview = r
        .run("canvas.render", json!({"docId": doc, "report": "Mini"}))
        .await;
    let mut west = None;
    for h in overview["hitRegions"].as_array().unwrap() {
        let id = h["id"].as_str().unwrap();
        if !id.starts_with("bars|dataPoint|") {
            continue;
        }
        let tip = r
            .run(
                "canvas.tooltip",
                json!({"docId": doc, "report": "Mini", "hitId": id}),
            )
            .await;
        if tip["lines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["value"] == "West")
        {
            west = Some(id.to_owned());
        }
    }
    let west = west.expect("a West bar");

    // Mini's own Details page is no drillthrough page; Detail's is offered.
    let targets = r
        .run(
            "canvas.drillthroughTargets",
            json!({"docId": doc, "report": "Mini", "hitId": west}),
        )
        .await;
    assert_eq!(
        targets["targets"],
        json!([{"page": "details", "displayName": "Details", "fields": ["Sales.Region"],
            "keepAllFilters": true, "report": "Detail"}]),
        "{targets}"
    );

    // Without `targetReport` the page is looked up in Mini.
    let err = r
        .try_run(
            "canvas.drillthrough",
            json!({"docId": doc, "report": "Mini", "hitId": west, "target": "Details"}),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, OpErrorCode::InvalidParams);
    // Detail's Overview is not a cross-report drillthrough page.
    let err = r
        .try_run(
            "canvas.drillthrough",
            json!({"docId": doc, "report": "Mini", "hitId": west, "target": "Overview",
                "targetReport": "Detail"}),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, OpErrorCode::InvalidParams);

    let drilled = r
        .run(
            "canvas.drillthrough",
            json!({"docId": doc, "report": "Mini", "hitId": west, "target": "Details",
                "targetReport": "Detail"}),
        )
        .await;
    assert_eq!(drilled["page"], "details");
    assert_eq!(drilled["report"], "Detail");
    let rendered = &drilled["rendered"];
    assert!(
        rendered["svg"].as_str().unwrap().contains(">590<"),
        "{}",
        rendered["visuals"]
    );
    // The drilled state belongs to Detail's page, not Mini's.
    let again = r
        .run(
            "canvas.render",
            json!({"docId": doc, "report": "Detail", "page": "Details"}),
        )
        .await;
    let drill = again["selections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["kind"] == "drillthrough")
        .expect("a drillthrough selection");
    assert_eq!(
        drill["label"], "Drillthrough from the Mini report",
        "{drill}"
    );
    let mini = r
        .run(
            "canvas.render",
            json!({"docId": doc, "report": "Mini", "page": "Details"}),
        )
        .await;
    assert!(
        mini["selections"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["kind"] != "drillthrough"),
        "{}",
        mini["selections"]
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn range_slicers_filter_between_bounds() {
    let dir = tmp_dir("pbi-ops-range-");
    let pbip = mini_pbip(dir.path());
    write_visual(
        dir.path(),
        "overview",
        "amount",
        false,
        json!({"visualType": "slicer", "query": {"queryState": {"Values": {"projections": [{
            "field": {"Column": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Amount"}},
            "queryRef": "Sales.Amount", "nativeQueryRef": "Amount", "active": true
        }]}}}}),
    );
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip})).await;

    // A numeric slicer without a mode is a range over the data.
    let page = r.run("canvas.render", json!({"docId": doc})).await;
    let slicer = page["interactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["visual"] == "amount")
        .unwrap();
    assert_eq!(slicer["slicerMode"], "range", "{slicer}");
    let range = &slicer["range"];
    assert_eq!(
        (range["min"].as_f64(), range["max"].as_f64()),
        (Some(1.0), Some(60.0))
    );
    assert!(
        range["track"].is_object() && range["lowBox"].is_object(),
        "{range}"
    );

    // 1 to 10: the total is 55.
    let set = r
        .run(
            "canvas.setRange",
            json!({"docId": doc, "visual": "amount", "low": 1, "high": "10"}),
        )
        .await;
    assert_eq!(
        set["selections"][0]["summary"], "is between 1 and 10",
        "{set}"
    );
    assert!(set["svg"].as_str().unwrap().contains(">55<"));
    let slicer = set["interactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["visual"] == "amount")
        .unwrap();
    assert_eq!(slicer["range"]["high"].as_f64(), Some(10.0), "{slicer}");

    // Open upper bound; bad bounds; clear.
    let open = r
        .run(
            "canvas.setRange",
            json!({"docId": doc, "field": "Amount", "low": 59}),
        )
        .await;
    assert_eq!(open["selections"][0]["summary"], "is at least 59", "{open}");
    assert!(open["svg"].as_str().unwrap().contains(">119<"));
    for bad in [json!({"low": 5, "high": 4}), json!({"low": "x"})] {
        let mut params = json!({"docId": doc, "visual": "amount"});
        params
            .as_object_mut()
            .unwrap()
            .extend(bad.as_object().unwrap().clone());
        let err = r.try_run("canvas.setRange", params).await.unwrap_err();
        assert_eq!(err.code, OpErrorCode::InvalidParams, "{err:?}");
    }
    let err = r
        .try_run(
            "canvas.setRelativeDate",
            json!({"docId": doc, "visual": "amount", "anchor": "last"}),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, OpErrorCode::InvalidParams, "{err:?}");
    let cleared = r
        .run("canvas.setRange", json!({"docId": doc, "visual": "amount"}))
        .await;
    assert!(cleared["selections"].as_array().unwrap().is_empty());
    assert!(
        cleared["svg"]
            .as_str()
            .unwrap()
            .contains(&format!(">{MINI_TOTAL}<"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn relative_date_slicers_follow_the_session_clock() {
    let Some(pbip) = common::fixture(common::PBIP) else {
        println!("SKIP: {} missing", common::PBIP);
        return;
    };
    let blob_map = common::fixture(common::BLOB_MAP).unwrap();
    let r = Runner::new();
    let doc = r
        .open(json!({"path": pbip, "blobMap": blob_map, "now": "2026-01-15T09:30:00Z"}))
        .await;
    let at = json!({"docId": doc, "report": "Overview", "page": "Time Intelligence"});
    let with = |extra: Value| {
        let mut p = at.clone();
        p.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        p
    };
    let relative = |page: &Value| {
        page["interactions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["visual"] == "398b24904a1feb7d0755")
            .unwrap()
            .clone()
    };

    // The saved "last 10 years" setting reads back into the controls and
    // applies (no "not supported" warning).
    let page = r.run("canvas.render", at.clone()).await;
    let slicer = relative(&page);
    assert_eq!(slicer["slicerMode"], "relative", "{slicer}");
    assert_eq!(slicer["relative"]["anchor"], "last");
    assert_eq!(slicer["relative"]["duration"], 10);
    assert_eq!(slicer["relative"]["period"], "years");
    assert!(
        !page["warnings"].to_string().contains("RelativeDate"),
        "{}",
        page["warnings"]
    );

    let set = r
        .run(
            "canvas.setRelativeDate",
            with(json!({"visual": "398b24904a1feb7d0755", "anchor": "last", "duration": 3, "period": "calendarMonths"})),
        )
        .await;
    let summaries: Vec<&str> = set["selections"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["summary"].as_str())
        .collect();
    assert!(
        summaries.contains(&"is in the last 3 months (calendar)"),
        "{summaries:?}"
    );
    let slicer = relative(&set);
    assert_eq!(slicer["relative"]["period"], "calendarMonths", "{slicer}");
    let this = r
        .run(
            "canvas.setRelativeDate",
            with(json!({"field": "Date", "anchor": "this", "period": "years"})),
        )
        .await;
    assert_eq!(relative(&this)["relative"]["anchor"], "this");
    let err = r
        .try_run(
            "canvas.setRelativeDate",
            with(json!({"field": "Date", "anchor": "next", "duration": 0})),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, OpErrorCode::InvalidParams, "{err:?}");
    let cleared = r
        .run("canvas.setRelativeDate", with(json!({"field": "Date"})))
        .await;
    assert!(
        cleared["selections"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["visual"] != "398b24904a1feb7d0755"),
        "{}",
        cleared["selections"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn visual_header_focus_sort_spotlight_copy_and_export() {
    let dir = tmp_dir("pbi-ops-header-");
    let pbip = mini_pbip(dir.path());
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip})).await;
    let visual = |page: &Value, name: &str| {
        page["visuals"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == name)
            .cloned()
            .unwrap_or_else(|| panic!("no visual {name} in {page}"))
    };

    // The bar chart sorts by its axis or its value; cards do not sort.
    let page = r.run("canvas.render", json!({"docId": doc})).await;
    let sort = &visual(&page, "bars")["sort"];
    let refs: Vec<&str> = sort["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["queryRef"].as_str().unwrap())
        .collect();
    assert_eq!(refs, ["Sales.Region", "Sum(Sales.Amount)"]);
    assert_eq!(sort["changed"], false);
    assert!(visual(&page, "total").get("sort").is_none());

    let sorted = r
        .run(
            "canvas.sortVisual",
            json!({"docId": doc, "visual": "bars", "field": "Region"}),
        )
        .await;
    let current = &visual(&sorted, "bars")["sort"]["current"];
    assert_eq!(current[0]["queryRef"], "Sales.Region", "{current}");
    assert_eq!(current[0]["direction"], "ascending");
    let flipped = r
        .run(
            "canvas.sortVisual",
            json!({"docId": doc, "visual": "bars", "direction": "descending"}),
        )
        .await;
    let sort = &visual(&flipped, "bars")["sort"];
    assert_eq!(sort["current"][0]["direction"], "descending");
    assert_eq!(sort["changed"], true);
    let by_value = r
        .run(
            "canvas.sortVisual",
            json!({"docId": doc, "visual": "bars", "field": "Sales[Amount]"}),
        )
        .await;
    assert_eq!(
        visual(&by_value, "bars")["sort"]["current"][0]["direction"],
        "descending"
    );
    let reset = r
        .run("canvas.sortVisual", json!({"docId": doc, "visual": "bars"}))
        .await;
    assert_eq!(visual(&reset, "bars")["sort"]["changed"], false);
    let err = r
        .try_run(
            "canvas.sortVisual",
            json!({"docId": doc, "visual": "bars", "field": "Nope"}),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, OpErrorCode::NotFound, "{err:?}");
    let err = r
        .try_run(
            "canvas.sortVisual",
            json!({"docId": doc, "visual": "total", "field": "Total"}),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, OpErrorCode::InvalidParams, "{err:?}");

    // Copy value and selection of a bar.
    let bar = hit(&page, "bars|dataPoint|").to_owned();
    let menu = r
        .run("canvas.contextMenu", json!({"docId": doc, "hitId": bar}))
        .await;
    assert_eq!(menu["visual"], "bars");
    let value = menu["copyValue"].as_str().unwrap();
    let selection = menu["copySelection"].as_str().unwrap();
    let (header, values) = selection.split_once('\n').unwrap();
    assert_eq!(header, "Region\tSum of Amount", "{menu}");
    assert!(values.ends_with(&format!("\t{value}")), "{menu}");
    assert!(["630", "590", "610"].contains(&value), "{menu}");

    // Spotlight.
    let lit = r
        .run(
            "canvas.setSpotlight",
            json!({"docId": doc, "visual": "bars"}),
        )
        .await;
    assert_eq!(lit["spotlight"], "bars");
    let off = r.run("canvas.setSpotlight", json!({"docId": doc})).await;
    assert!(off.get("spotlight").is_none());

    // Focus mode: the chart fills the page, the others are hidden.
    let focused = r
        .run("canvas.focus", json!({"docId": doc, "visual": "bars"}))
        .await;
    assert_eq!(focused["focus"]["visual"], "bars", "{focused}");
    assert_eq!(visual(&focused, "total")["hidden"], true);
    assert_eq!(visual(&focused, "region")["hidden"], true);
    let rect = &visual(&focused, "bars")["rect"];
    assert!(rect["width"].as_f64().unwrap() > 600.0, "{rect}");
    assert_eq!(focused["isDefault"], false);

    // Show as a table: a table of the chart's data below it, whose cells
    // copy like any table's.
    let table = r
        .run(
            "canvas.showAsTable",
            json!({"docId": doc, "visual": "bars"}),
        )
        .await;
    assert_eq!(table["focus"]["showAsTable"], true);
    let data = visual(&table, "bars~table");
    assert_eq!(data["visualType"], "tableEx");
    assert!(data["error"].is_null(), "{data}");
    let cell = hit(&table, "bars~table|dataPoint|").to_owned();
    let menu = r
        .run("canvas.contextMenu", json!({"docId": doc, "hitId": cell}))
        .await;
    assert_eq!(menu["visual"], "bars", "{menu}");
    assert!(menu["copyValue"].is_string(), "{menu}");
    let click = r
        .run("canvas.click", json!({"docId": doc, "hitId": cell}))
        .await;
    assert_eq!(click["changed"], false, "a data table cell does not select");
    let back = r.run("canvas.focus", json!({"docId": doc})).await;
    assert!(back.get("focus").is_none());
    assert_eq!(visual(&back, "total")["hidden"], false);
    let err = r
        .try_run("canvas.focus", json!({"docId": doc, "visual": "nope"}))
        .await
        .unwrap_err();
    assert_eq!(err.code, OpErrorCode::NotFound, "{err:?}");

    // Export: the summarized data inline, the underlying rows to a file,
    // both under the slicer selection.
    let summary = r
        .run("export.visualData", json!({"docId": doc, "visual": "bars"}))
        .await;
    assert_eq!(summary["columns"], json!(["Region", "Sum of Amount"]));
    let mut lines: Vec<&str> = summary["csv"].as_str().unwrap().lines().collect();
    assert_eq!(lines.remove(0), "Region,Sum of Amount");
    lines.sort_unstable();
    assert_eq!(lines, ["East,630", "North,610", "West,590"]);
    r.run(
        "canvas.setSlicer",
        json!({"docId": doc, "visual": "region", "values": ["North"]}),
    )
    .await;
    let out = dir.path().join("rows.csv");
    let rows = r
        .run(
            "export.visualData",
            json!({"docId": doc, "visual": "bars", "mode": "underlying", "path": out}),
        )
        .await;
    assert_eq!(rows["rows"], 20, "{rows}");
    assert_eq!(rows["columns"][0], "Region");
    let csv = std::fs::read_to_string(&out).unwrap();
    assert!(csv.starts_with("Region,Amount,Item\r\n"), "{csv}");
    assert_eq!(csv.lines().count(), 21);
    let err = r
        .try_run(
            "export.visualData",
            json!({"docId": doc, "visual": "bars", "path": out}),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, OpErrorCode::Io, "{err:?}");
}
