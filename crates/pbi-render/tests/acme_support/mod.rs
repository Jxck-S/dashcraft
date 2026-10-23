//! Shared helpers for the tests on the committed fixtures: the synthetic
//! Acme Outfitters example (`examples/acme-outfitters/`, refreshed offline
//! through its `blob-map.json`) with its `expected.json`, and the Microsoft
//! samples (`samples/microsoft/`, MIT) with theirs.
//!
//! Paths are relative to the fixture root (`$PBIVIEW_REFERENCE`, else
//! `<workspace>/reference`). A missing file prints `SKIP: <file> missing`
//! and the test returns early.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use pbi_core::ReportSource;
use pbi_model::{Page, Report};
use pbi_render::query::{InteractionState, QueryEnv};
use pbi_render::scene::{Group, HitKind, Node, Scene};
use pbi_render::theme::ThemeResolver;
use pbi_render::visuals::{PageRenderOptions, RenderedPage, render_report_page};
use pbi_session::{FixedClock, RefreshOptions, Session, SessionOptions};
use serde_json::Value as Json;

/// The example folder, relative to the fixture root.
pub const ACME: &str = "examples/acme-outfitters";
/// Report short names (`--report`), in `expected.json` order.
pub const ACME_REPORTS: [&str; 3] = ["Overview", "Stores", "Online"];

/// `$PBIVIEW_REFERENCE`, else `<workspace>/reference`.
pub fn reference_root() -> PathBuf {
    std::env::var_os("PBIVIEW_REFERENCE").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference"),
        PathBuf::from,
    )
}

/// `<root>/<rel>`, or `None` after printing `SKIP: … missing`.
pub fn fixture(rel: &str) -> Option<PathBuf> {
    let path = reference_root().join(rel);
    if path.exists() {
        Some(path)
    } else {
        println!("SKIP: {} missing", path.display());
        None
    }
}

fn read_json(path: &Path) -> Json {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The fixed refresh time (the example's `Last Refreshed` table is the
/// only thing that reads the clock).
pub fn clock() -> FixedClock {
    use chrono::TimeZone;
    FixedClock::utc(
        chrono::Utc
            .with_ymd_and_hms(2026, 1, 15, 9, 30, 0)
            .single()
            .unwrap(),
    )
}

/// Opens `path` and refreshes every table, asserting the refresh succeeds.
pub fn open_refreshed(path: &Path, blob_map: Option<PathBuf>) -> Session {
    let session = Session::open(
        path,
        SessionOptions {
            blob_map,
            clock: Some(Arc::new(clock())),
            ..SessionOptions::default()
        },
    )
    .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let refresh = rt.block_on(session.refresh(RefreshOptions::all())).unwrap();
    assert!(refresh.is_ok(), "{}: {refresh:#?}", path.display());
    session
}

/// A fresh, refreshed Acme Outfitters session (or `None`: SKIP).
pub fn open_acme() -> Option<Session> {
    let pbip = fixture(&format!("{ACME}/AcmeOutfitters.pbip"))?;
    let blob_map = fixture(&format!("{ACME}/blob-map.json"))?;
    Some(open_refreshed(&pbip, Some(blob_map)))
}

/// The shared refreshed Acme Outfitters session of this test binary.
pub fn acme() -> Option<&'static Session> {
    static SESSION: OnceLock<Option<Session>> = OnceLock::new();
    SESSION.get_or_init(open_acme).as_ref()
}

/// `examples/acme-outfitters/expected.json`.
pub fn acme_expected() -> Option<&'static Json> {
    static EXPECTED: OnceLock<Option<Json>> = OnceLock::new();
    EXPECTED
        .get_or_init(|| fixture(&format!("{ACME}/expected.json")).map(|p| read_json(&p)))
        .as_ref()
}

/// Index of the report `Acme Outfitters - <short>` in the session.
pub fn report_index(session: &Session, short: &str) -> usize {
    let name = format!("Acme Outfitters - {short}");
    session
        .document()
        .reports
        .iter()
        .position(|r| r.name == name)
        .unwrap_or_else(|| panic!("no report {name}"))
}

pub fn report<'a>(session: &'a Session, short: &str) -> &'a Report {
    &session.document().reports[report_index(session, short)]
}

/// The page with display name `display`.
pub fn page<'a>(report: &'a Report, display: &str) -> &'a Page {
    report
        .pages
        .iter()
        .find(|p| p.display_name == display)
        .unwrap_or_else(|| panic!("{}: no page {display}", report.name))
}

/// `expected.json` report entry of `Acme Outfitters - <short>`.
pub fn expected_report<'a>(expected: &'a Json, short: &str) -> &'a Json {
    let name = format!("Acme Outfitters - {short}");
    expected["reports"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|r| r["report"] == name.as_str())
        .unwrap_or_else(|| panic!("expected.json: no report {name}"))
}

/// `expected.json` page entry.
pub fn expected_page<'a>(expected: &'a Json, short: &str, page: &str) -> &'a Json {
    expected_report(expected, short)["pages"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["page"] == page)
        .unwrap_or_else(|| panic!("expected.json: no page {short} / {page}"))
}

/// The id of the visual whose `expected.json` key (`visualKey`) is `key`.
pub fn visual_id(expected: &Json, short: &str, page: &str, key: &str) -> String {
    let entry = expected_page(expected, short, page);
    ["cards", "visuals", "slicers"]
        .iter()
        .filter_map(|list| entry[*list].as_array())
        .flatten()
        .find(|v| v["visualKey"] == key)
        .and_then(|v| v["visual"].as_str())
        .unwrap_or_else(|| panic!("{short} / {page}: no visual key {key}"))
        .to_owned()
}

/// The `formatted` text of `entry[field]` (`{"value", "formatted"}`).
pub fn formatted<'a>(entry: &'a Json, field: &str) -> &'a str {
    entry[field]["formatted"]
        .as_str()
        .unwrap_or_else(|| panic!("no formatted {field} in {entry}"))
}

/// The theme of report `index` (PBIR themes; legacy layouts use the
/// built-in defaults here).
pub fn theme(session: &Session, index: usize) -> ThemeResolver {
    match session.opened().report_sources.get(index) {
        Some(ReportSource::Pbir { report, .. }) => ThemeResolver::from_pbir(report),
        _ => ThemeResolver::default(),
    }
}

/// Renders `page` of report `index` with its saved slicer selections.
pub fn render(session: &Session, index: usize, page: &Page) -> RenderedPage {
    render_with(session, index, page, &InteractionState::from_page(page).0)
}

/// Renders `page` of report `index` in `state`.
pub fn render_with(
    session: &Session,
    index: usize,
    page: &Page,
    state: &InteractionState,
) -> RenderedPage {
    let report = &session.document().reports[index];
    let env = QueryEnv::from_session(session);
    render_report_page(
        &env,
        report,
        page,
        &theme(session, index),
        state,
        &PageRenderOptions::default(),
    )
}

/// The container group of visual `name`.
pub fn find_visual<'a>(g: &'a Group, name: &str) -> Option<&'a Group> {
    if g.id
        .as_ref()
        .is_some_and(|id| id.visual == name && id.kind == HitKind::Visual)
    {
        return Some(g);
    }
    g.children.iter().find_map(|c| match c {
        Node::Group(child) => find_visual(child, name),
        _ => None,
    })
}

/// Every text of `g`, in drawing order.
pub fn texts(g: &Group) -> Vec<String> {
    fn walk(g: &Group, out: &mut Vec<String>) {
        for c in &g.children {
            match c {
                Node::Text(t) => out.push(t.text.clone()),
                Node::Group(child) => walk(child, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(g, &mut out);
    out
}

/// The texts drawn by visual `name` (empty when it is not drawn).
pub fn visual_texts(scene: &Scene, name: &str) -> Vec<String> {
    find_visual(&scene.root, name)
        .map(texts)
        .unwrap_or_default()
}

/// `(series, index)` of every data-point group of `g`.
pub fn data_points(g: &Group) -> Vec<(usize, usize)> {
    fn walk(g: &Group, out: &mut Vec<(usize, usize)>) {
        if let Some(HitKind::DataPoint { series, index }) = g.id.as_ref().map(|i| &i.kind) {
            out.push((*series, *index));
        }
        for c in &g.children {
            if let Node::Group(child) = c {
                walk(child, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(g, &mut out);
    out
}

/// `samples/microsoft/expected.json`.
pub fn samples_expected() -> Option<&'static Json> {
    static EXPECTED: OnceLock<Option<Json>> = OnceLock::new();
    EXPECTED
        .get_or_init(|| fixture("samples/microsoft/expected.json").map(|p| read_json(&p)))
        .as_ref()
}

/// The `files[]` entries of the samples' `expected.json`.
pub fn sample_files() -> Vec<&'static Json> {
    samples_expected()
        .and_then(|e| e["files"].as_array())
        .map(|files| files.iter().collect())
        .unwrap_or_default()
}

/// Opens and refreshes a sample (`file` relative to the fixture root); a
/// missing fetched sample prints `SKIP` and gives `None`. A template
/// (PBIT) has no data and refreshes only partially, which is expected.
pub fn open_sample(file: &str) -> Option<Session> {
    let path = reference_root().join(file);
    if !path.exists() {
        println!(
            "SKIP: {} missing (run scripts/fetch-samples.sh)",
            path.display()
        );
        return None;
    }
    let session =
        Session::open(&path, SessionOptions::default()).unwrap_or_else(|e| panic!("{file}: {e}"));
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let _refresh = rt.block_on(session.refresh(RefreshOptions::all())).unwrap();
    Some(session)
}
