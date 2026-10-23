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
