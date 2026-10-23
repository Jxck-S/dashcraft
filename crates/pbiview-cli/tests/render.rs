//! `dashcraft render` end to end on a synthetic PBIP (written to a temporary
//! folder; inline M data, no fixture files needed).
//!
//! The project: a `Sales` table (`#table` M partition: East 10, West 20,
//! North 5), a `'Last Refreshed'` table stamped with `DateTimeZone.UtcNow()`,
//! and a report with two pages: "Overview" (640×360: textbox, two cards,
//! a Region slicer, a bar chart) and "Details" (a table and a card bound to
//! a measure that does not exist).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

fn pbiview(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dashcraft"))
        .args(args)
        .env_remove("RUST_LOG")
        .output()
        .expect("run pbiview")
}

fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).expect("UTF-8 stdout")
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn assert_exit(o: &Output, code: i32) {
    assert_eq!(
        o.status.code(),
        Some(code),
        "exit status; stdout:\n{}\nstderr:\n{}",
        stdout(o),
        stderr(o)
    );
}

fn tmp_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("pbiview-render-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap()
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn write_json(root: &Path, rel: &str, value: &Value) {
    write(root, rel, &serde_json::to_string_pretty(value).unwrap());
}

fn column(table: &str, column: &str) -> Value {
    json!({"Column": {"Expression": {"SourceRef": {"Entity": table}}, "Property": column}})
}

fn measure(table: &str, measure: &str) -> Value {
    json!({"Measure": {"Expression": {"SourceRef": {"Entity": table}}, "Property": measure}})
}

fn projection(field: Value, query_ref: &str, native: &str) -> Value {
    json!({"field": field, "queryRef": query_ref, "nativeQueryRef": native, "active": true})
}

fn visual(page: &str, name: &str, pos: (f64, f64, f64, f64), body: Value) -> (String, Value) {
    (
        format!("Mini.Report/definition/pages/{page}/visuals/{name}/visual.json"),
        json!({
            "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/visualContainer/2.4.0/schema.json",
            "name": name,
            "position": {"x": pos.0, "y": pos.1, "z": 0, "width": pos.2, "height": pos.3, "tabOrder": 0},
            "visual": body
        }),
    )
}

fn card(page: &str, name: &str, pos: (f64, f64, f64, f64), m: &str) -> (String, Value) {
    visual(
        page,
        name,
        pos,
        json!({"visualType": "cardVisual", "query": {"queryState": {"Data": {"projections": [
            projection(measure("Sales", m), &format!("Sales.{m}"), m)
        ]}}}}),
    )
}

/// Writes the synthetic project; returns the `.pbip` path.
fn mini_pbip(root: &Path) -> PathBuf {
    write_json(
        root,
        "Mini.pbip",
        &json!({"version": "1.0", "artifacts": [{"report": {"path": "Mini.Report"}}]}),
    );
    write_json(
        root,
        "Mini.SemanticModel/definition.pbism",
        &json!({"version": "4.2", "settings": {}}),
    );
    write(
        root,
        "Mini.SemanticModel/definition/database.tmdl",
        "database\n\tcompatibilityLevel: 1567\n",
    );
    write(
        root,
        "Mini.SemanticModel/definition/model.tmdl",
        "model Model\n\tculture: en-US\n\nref table Sales\nref table 'Last Refreshed'\n",
    );
    write(
        root,
        "Mini.SemanticModel/definition/tables/Sales.tmdl",
        "table Sales\n\
         \n\
         \tmeasure Total = SUM(Sales[Amount])\n\
         \t\tformatString: 0\n\
         \n\
         \tmeasure Refreshed = MAX('Last Refreshed'[At])\n\
         \t\tformatString: yyyy-mm-dd hh:nn\n\
         \n\
         \tcolumn Region\n\
         \t\tdataType: string\n\
         \t\tsummarizeBy: none\n\
         \t\tsourceColumn: Region\n\
         \n\
         \tcolumn Amount\n\
         \t\tdataType: int64\n\
         \t\tformatString: 0\n\
         \t\tsummarizeBy: sum\n\
         \t\tsourceColumn: Amount\n\
         \n\
         \tpartition Sales = m\n\
         \t\tmode: import\n\
         \t\tsource =\n\
         \t\t\t\tlet\n\
         \t\t\t\t    Source = #table(type table [Region = text, Amount = number], {{\"East\", 10}, {\"West\", 20}, {\"North\", 5}})\n\
         \t\t\t\tin\n\
         \t\t\t\t    Source\n",
    );
    write(
        root,
        "Mini.SemanticModel/definition/tables/Last Refreshed.tmdl",
        "table 'Last Refreshed'\n\
         \n\
         \tcolumn At\n\
         \t\tdataType: dateTime\n\
         \t\tformatString: yyyy-mm-dd hh:nn\n\
         \t\tsummarizeBy: none\n\
         \t\tsourceColumn: At\n\
         \n\
         \tpartition 'Last Refreshed' = m\n\
         \t\tmode: import\n\
         \t\tsource =\n\
         \t\t\t\tlet\n\
         \t\t\t\t    Source = #table(type table [At = datetime], {{DateTimeZone.RemoveZone(DateTimeZone.UtcNow())}})\n\
         \t\t\t\tin\n\
         \t\t\t\t    Source\n",
    );
    write_json(
        root,
        "Mini.Report/definition.pbir",
        &json!({"version": "4.0", "datasetReference": {"byPath": {"path": "../Mini.SemanticModel"}}}),
    );
    write_json(
        root,
        "Mini.Report/definition/version.json",
        &json!({"$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/versionMetadata/1.0.0/schema.json", "version": "2.0.0"}),
    );
    write_json(
        root,
        "Mini.Report/definition/report.json",
        &json!({"$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/report/3.0.0/schema.json"}),
    );
    write_json(
        root,
        "Mini.Report/definition/pages/pages.json",
        &json!({"pageOrder": ["overview", "details"], "activePageName": "overview"}),
    );
    for (name, display) in [("overview", "Overview"), ("details", "Details")] {
        write_json(
            root,
            &format!("Mini.Report/definition/pages/{name}/page.json"),
            &json!({"name": name, "displayName": display, "displayOption": "FitToPage", "width": 640, "height": 360}),
        );
    }
    let region = || projection(column("Sales", "Region"), "Sales.Region", "Region");
    let amount = || {
        projection(
            json!({"Aggregation": {"Expression": column("Sales", "Amount"), "Function": 0}}),
            "Sum(Sales.Amount)",
            "Sum of Amount",
        )
    };
    let visuals = [
        visual(
            "overview",
            "title",
            (10.0, 10.0, 300.0, 40.0),
            json!({"visualType": "textbox", "objects": {"general": [{"properties": {"paragraphs": [
                {"textRuns": [{"value": "Mini sales report", "textStyle": {"fontWeight": "bold", "fontSize": "14pt"}}]}
            ]}}]}}),
        ),
        card("overview", "total", (10.0, 60.0, 180.0, 90.0), "Total"),
        card(
            "overview",
            "refreshed",
            (200.0, 60.0, 220.0, 90.0),
            "Refreshed",
        ),
        visual(
            "overview",
            "region",
            (440.0, 60.0, 190.0, 90.0),
            json!({"visualType": "slicer", "query": {"queryState": {"Values": {"projections": [region()]}}}}),
        ),
        visual(
            "overview",
            "bars",
            (10.0, 160.0, 620.0, 190.0),
            json!({"visualType": "clusteredBarChart", "query": {"queryState": {
                "Category": {"projections": [region()]},
                "Y": {"projections": [amount()]}
            }}}),
        ),
        visual(
            "details",
            "table",
            (10.0, 10.0, 300.0, 200.0),
            json!({"visualType": "tableEx", "query": {"queryState": {"Values": {"projections": [region(), amount()]}}}}),
        ),
        card("details", "broken", (320.0, 10.0, 200.0, 90.0), "Missing"),
    ];
    for (rel, v) in &visuals {
        write_json(root, rel, v);
    }
    root.join("Mini.pbip")
}

/// `(width, height)` from a PNG's `IHDR`.
fn png_size(bytes: &[u8]) -> (u32, u32) {
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");
    let be = |r: std::ops::Range<usize>| u32::from_be_bytes(bytes[r].try_into().unwrap());
    (be(16..20), be(20..24))
}

const NOW: &str = "2026-10-21T21:35:00Z";

#[test]
fn png_is_deterministic_and_sized() {
    let tmp = tmp_dir();
    let pbip = mini_pbip(tmp.path());
    let (a, b, small) = (
        tmp.path().join("a.png"),
        tmp.path().join("b.png"),
        tmp.path().join("small.png"),
    );
    for out in [&a, &b] {
        let o = pbiview(&["render", s(&pbip), "--png", s(out), "--now", NOW]);
        assert_exit(&o, 0);
        assert!(stdout(&o).contains("Overview"), "{}", stdout(&o));
    }
    let (bytes_a, bytes_b) = (fs::read(&a).unwrap(), fs::read(&b).unwrap());
    assert_eq!(png_size(&bytes_a), (1280, 720), "640×360 page at --scale 2");
    assert!(bytes_a == bytes_b, "same inputs, different PNG bytes");

    let o = pbiview(&[
        "render",
        s(&pbip),
        "--png",
        s(&small),
        "--scale",
        "1",
        "--now",
        NOW,
    ]);
    assert_exit(&o, 0);
    assert_eq!(png_size(&fs::read(&small).unwrap()), (640, 360));
}

#[test]
fn svg_shows_data_the_clock_and_slicer_selections() {
    let tmp = tmp_dir();
    let pbip = mini_pbip(tmp.path());
    let svg = |name: &str, extra: &[&str]| -> String {
        let out = tmp.path().join(name);
        let mut args = vec!["render", s(&pbip), "--svg", s(&out)];
        args.extend_from_slice(extra);
        let o = pbiview(&args);
        assert_exit(&o, 0);
        fs::read_to_string(out).unwrap()
    };
    let all = svg("all.svg", &["--now", NOW]);
    assert!(all.contains(">35</text>"), "total of every region");
    assert!(
        all.contains(">2026-10-21 21:35</text>"),
        "--now feeds UtcNow()"
    );
    assert!(all.contains(">Mini sales report</text>"));

    let later = svg("later.svg", &["--now", "2027-01-02T03:04:00+00:00"]);
    assert!(later.contains(">2027-01-02 03:04</text>"));

    let east = svg("east.svg", &["--now", NOW, "--slicer", "Region=East"]);
    assert!(east.contains(">10</text>") && !east.contains(">35</text>"));
    let not_east = svg(
        "not-east.svg",
        &["--now", NOW, "--slicer", "sales[region]!=East"],
    );
    assert!(not_east.contains(">25</text>"));
    let by_name = svg(
        "by-name.svg",
        &["--now", NOW, "--slicer", "region=West,North"],
    );
    assert!(by_name.contains(">25</text>"));
}

#[test]
fn json_reports_every_visual_and_errors_exit_2() {
    let tmp = tmp_dir();
    let pbip = mini_pbip(tmp.path());
    let dir = tmp.path().join("pages");
    let o = pbiview(&[
        "render",
        s(&pbip),
        "--out-dir",
        s(&dir),
        "--all-pages",
        "--format",
        "both",
        "--slicer",
        "Region=East",
        "--now",
        NOW,
        "--json",
    ]);
    // The "Details" page has a card bound to a missing measure.
    assert_exit(&o, 2);
    let out: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(out["schemaVersion"], 1);
    assert_eq!(out["report"], "Mini");
    assert_eq!(out["ok"], false);
    assert_eq!(out["now"], "2026-10-21T21:35:00+00:00");
    assert_eq!(out["refresh"]["failed"], 0);
    let pages = out["pages"].as_array().unwrap();
    assert_eq!(pages.len(), 2);

    let overview = &pages[0];
    assert_eq!(
        (&overview["index"], &overview["displayName"]),
        (&json!(1), &json!("Overview"))
    );
    assert_eq!(overview["summary"]["visuals"], 5);
    assert_eq!(overview["summary"]["errors"], 0);
    assert_eq!(
        overview["slicers"],
        json!([{"visual": "region", "values": ["East"], "inverted": false, "condition": false, "origin": "cli"}])
    );
    let mut statuses: Vec<(&str, &str)> = overview["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v["name"].as_str().unwrap(), v["status"].as_str().unwrap()))
        .collect();
    statuses.sort_unstable();
    assert_eq!(
        statuses,
        [
            ("bars", "ok"),
            ("refreshed", "ok"),
            ("region", "ok"),
            ("title", "ok"),
            ("total", "ok")
        ]
    );
    assert!(overview["visuals"][0]["queryMicros"].is_u64());
    let outputs = overview["outputs"].as_array().unwrap();
    assert_eq!(outputs.len(), 2);
    assert_eq!(
        (
            &outputs[0]["format"],
            &outputs[0]["width"],
            &outputs[0]["height"]
        ),
        (&json!("png"), &json!(1280), &json!(720))
    );

    let details = &pages[1];
    assert_eq!(details["summary"]["errors"], 1);
    let broken = details["visuals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "broken")
        .unwrap();
    assert_eq!(broken["status"], "error");
    assert!(broken["error"].as_str().unwrap().contains("Missing"));

    let mut files: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(
        files,
        [
            "01-Overview.png",
            "01-Overview.svg",
            "02-Details.png",
            "02-Details.svg"
        ]
    );
}

#[test]
fn existing_files_need_force() {
    let tmp = tmp_dir();
    let pbip = mini_pbip(tmp.path());
    let out = tmp.path().join("page.png");
    fs::write(&out, b"keep me").unwrap();
    let o = pbiview(&["render", s(&pbip), "--png", s(&out)]);
    assert_exit(&o, 1);
    assert!(stderr(&o).contains("--force"), "{}", stderr(&o));
    assert_eq!(fs::read(&out).unwrap(), b"keep me");

    let o = pbiview(&["render", s(&pbip), "--png", s(&out), "--force"]);
    assert_exit(&o, 0);
    assert_eq!(png_size(&fs::read(&out).unwrap()), (1280, 720));
}

#[test]
fn usage_errors_are_fatal() {
    let tmp = tmp_dir();
    let pbip = mini_pbip(tmp.path());
    let png = tmp.path().join("x.png");
    let dir = tmp.path().join("dir");
    let cases: &[(&[&str], &str)] = &[
        (&[], "--png"),
        (
            &["--png", s(&png), "--out-dir", s(&dir)],
            "cannot be used with",
        ),
        (&["--png", s(&png), "--all-pages"], "--out-dir"),
        (&["--png", s(&png), "--format", "svg"], "--out-dir"),
        (&["--png", s(&png), "--page", "3"], "has 2 page(s)"),
        (&["--png", s(&png), "--page", "Nope"], "no page 'Nope'"),
        (
            &["--png", s(&png), "--report", "Nope"],
            "no report named 'Nope'",
        ),
        (&["--png", s(&png), "--scale", "0"], "--scale"),
        (&["--png", s(&png), "--now", "yesterday"], "RFC 3339"),
        (&["--png", s(&png), "--slicer", "Region"], "SLICER=VALUE"),
        (
            &["--png", s(&png), "--slicer", "Nope=A"],
            "no slicer 'Nope'",
        ),
    ];
    for (extra, message) in cases {
        let mut args = vec!["render", s(&pbip)];
        args.extend_from_slice(extra);
        let o = pbiview(&args);
        assert_exit(&o, 1);
        assert!(
            stderr(&o).contains(message),
            "{extra:?}: expected {message:?} in:\n{}",
            stderr(&o)
        );
    }
    assert!(
        !png.exists() && !dir.exists(),
        "nothing is written on errors"
    );

    let o = pbiview(&[
        "render",
        s(&tmp.path().join("missing.pbip")),
        "--png",
        s(&png),
    ]);
    assert_exit(&o, 1);
}

#[test]
fn pages_are_found_by_name_index_and_default() {
    let tmp = tmp_dir();
    let pbip = mini_pbip(tmp.path());
    for (page, expected) in [
        (None, "Overview"),
        (Some("2"), "Details"),
        (Some("details"), "Details"),
        (Some("overview"), "Overview"),
        (Some("Detai"), "Details"),
    ] {
        let out = tmp
            .path()
            .join(format!("{}.svg", page.unwrap_or("default")));
        let mut args = vec!["render", s(&pbip), "--svg", s(&out), "--json"];
        if let Some(p) = page {
            args.extend_from_slice(&["--page", p]);
        }
        let o = pbiview(&args);
        let json: Value = serde_json::from_str(&stdout(&o)).unwrap();
        assert_eq!(json["pages"][0]["displayName"], expected, "--page {page:?}");
    }
}
