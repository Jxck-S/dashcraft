//! Interactive report canvas: the interaction and view state of every
//! (report, page) of an open document, page renders on a blocking worker,
//! and the click / scroll / tooltip logic behind the `canvas.*` commands.
//!
//! The desktop app, the CLI (`dashcraft run`/`serve`) and agents all use it
//! (ADR-0006, ADR-0007):
//!
//! - **State.** Per page: an `InteractionState` (slicer selections and
//!   selected data points) seeded from the report's saved selections, and a
//!   `PageViewState` (table / matrix scroll and expansion, open dropdowns
//!   with their search text). `canvas.reset` goes back to the saved state.
//! - **Renders.** Every render bumps the page's generation and runs on a
//!   blocking worker. A change of the interaction state cancels the renders
//!   of the previous state (their data is obsolete); renders of the same
//!   state (scrolling, dropdowns, search) run to completion. A finished
//!   render is kept and returned if it is newer than the newest completed
//!   one, even while a later render is still running ("newest completed
//!   wins"); an older one fails with `cancelled`, so out-of-order
//!   completions never move the page backwards (callers also drop responses
//!   older than what they show, by `generation`). Query results
//!   ([`PageData`], with its render-cache id) are reused while the
//!   interaction state and the data (engine snapshot, model) are unchanged,
//!   so scrolling and dropdowns only re-render, and `pbi-render`'s visual
//!   cache re-renders only the visuals whose inputs changed (one table on a
//!   scroll).
//! - **Themes.** PBIR reports use their theme files; PBIX layouts and
//!   PBIR-Legacy reports read their base and custom theme resources from
//!   the container.
//! - **Clicks** read the DataViews of the last completed render (what the
//!   caller saw); a page that was never rendered is rendered first.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use pbi_core::{OpenedDocument, ReportSource};
use pbi_engine::{CancellationToken, EngineSnapshot, Value};
use pbi_model::{FieldExpr, Literal, LiteralValue, Model, Page, Report, ResourceRef, Visual};
use pbi_render::dataview::{
    BLANK_LABEL, CellValue, ColumnMeta, DataView, FieldRefKind, ValueColumn,
};
use pbi_render::layout::{FrameOptions, auto_title};
use pbi_render::query::{
    InteractionState, PointField, PointSelection, QueryEnv, QueryLimits, SlicerSelection,
    is_slicer_type, slicer_fields,
};
use pbi_render::scene::{HitId, HitKind, HitRegion, Rect};
use pbi_render::svg::{SvgOptions, to_svg};
use pbi_render::theme::{Theme, ThemeResolver};
use pbi_render::visuals::button::{self, ButtonAction};
use pbi_render::visuals::charts::{self, ChartData};
use pbi_render::visuals::slicer::{SELECT_ALL_INDEX, SlicerMode};
use pbi_render::visuals::table::GridScroll;
use pbi_render::visuals::{
    InteractivePage, PageData, PageRenderOptions, PageViewState, VisualInteraction,
    query_page_data, render_interactive_page,
};
use pbi_session::{
    ColumnFilterSpec, FilterSpec, Session, UnsupportedFilter, filters_from_page,
    filters_from_report, value_to_json,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::dto::FilterSpecOut;
use crate::error::OpError;
use crate::resolve;
use crate::workspace::{DocEntry, lock};

/// Lines of a data point tooltip.
const MAX_TOOLTIP_LINES: usize = 12;
/// Values listed in a selection summary before "and n more".
const MAX_SUMMARY_VALUES: usize = 3;
/// Longest slicer search text (characters).
pub const MAX_SEARCH_CHARS: usize = 256;
/// Longest tooltip / summary value (characters).
const MAX_VALUE_CHARS: usize = 200;
/// Longest visual title (characters).
const MAX_TITLE_CHARS: usize = 120;
/// Hit regions returned per scene.
const MAX_HIT_REGIONS: usize = 100_000;
/// Renders of one page running at once; older ones are cancelled beyond
/// this.
const MAX_RUNNING: usize = 4;
/// Largest theme file read for a legacy report (bytes).
const MAX_THEME_BYTES: u64 = 8 * 1024 * 1024;

/// (report index, page index).
type PageKey = (usize, usize);

fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => format!("{}…", text.get(..cut).unwrap_or_default()),
        None => text.to_owned(),
    }
}

fn superseded() -> OpError {
    OpError::cancelled().context("a newer render of the page started")
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

/// A rectangle in page units (canvas pixels at 100% zoom without a
/// viewport).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RectOut {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A hit region: the `data-hit` id of an SVG group and its bounding box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HitRegionOut {
    /// Hit id, e.g. `<visual>|slicerItem|2`; pass it to `canvas.click`.
    pub id: String,
    pub rect: RectOut,
}

/// One visual of a rendered page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CanvasVisualOut {
    /// Internal name, the `visual` part of hit ids.
    pub name: String,
    pub visual_type: String,
    /// Title text, else the automatic title, else the visual type.
    pub title: Option<String>,
    /// Container rectangle (`null` for hidden visuals).
    pub rect: Option<RectOut>,
    pub hidden: bool,
    /// Query error (`⚠ field: message`).
    pub error: Option<String>,
    /// Hover text: the error with its DAX, or a button's tooltip.
    pub tooltip: Option<String>,
    /// Filters and interactions not applied (or only approximately).
    pub warnings: Vec<String>,
    /// The data was cut at the query limits.
    pub truncated: bool,
    /// A button whose click does something (`canvas.click` returns it).
    pub has_action: bool,
}

/// Scroll position and range of a table / matrix body or slicer list.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScrollOut {
    pub x: f64,
    pub y: f64,
    pub max_x: f64,
    pub max_y: f64,
    /// The scrolling area.
    pub viewport: RectOut,
}

/// Interactive geometry of a slicer, table or matrix.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CanvasInteractionOut {
    pub visual: String,
    /// `slicer`, `table` or `matrix`.
    pub kind: String,
    pub scroll: Option<ScrollOut>,
    /// Slicers: `dropdown`, `list`, `tile` or `range`.
    pub slicer_mode: Option<String>,
    /// Slicer header band.
    pub header: Option<RectOut>,
    /// Collapsed dropdown box.
    pub dropdown: Option<RectOut>,
    /// The open dropdown popup (in the overlay), `null` when closed.
    pub popup: Option<RectOut>,
    /// Search box, when the slicer has one.
    pub search: Option<RectOut>,
    pub dropdown_open: bool,
    pub search_text: String,
    /// The slicer filters something (not "All").
    pub has_selection: bool,
}

/// Kind of a Filters pane selection card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SelectionKind {
    Slicer,
    CrossFilter,
}

/// A slicer selection or the cross-filter, as a Filters pane card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SelectionOut {
    /// Slicer, or the cross-filter source visual.
    pub visual: String,
    pub kind: SelectionKind,
    pub label: String,
    /// `Table[Column]` of each field.
    pub fields: Vec<String>,
    /// `is West or East`, `is not Online`, …
    pub summary: String,
}

/// A rendered page of the interactive canvas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CanvasPageOut {
    pub doc_id: String,
    /// Report name.
    pub report: String,
    /// Page internal name.
    pub page: String,
    pub page_display_name: String,
    /// Render number of this page (increasing): drop older responses.
    pub generation: u32,
    /// The page as a standalone SVG document (deterministic; `data-hit`
    /// attributes on interactive groups).
    pub svg: String,
    /// Open slicer dropdown popups, drawn above the page; `null` when none
    /// is open.
    pub overlay_svg: Option<String>,
    pub width: f64,
    pub height: f64,
    /// Hit regions in paint order (hit-test from the end).
    pub hit_regions: Vec<HitRegionOut>,
    /// Hit regions of the overlay (above every page region).
    pub overlay_hit_regions: Vec<HitRegionOut>,
    /// Every visual of the page (hidden ones included), in page order.
    pub visuals: Vec<CanvasVisualOut>,
    pub interactions: Vec<CanvasInteractionOut>,
    /// Slicer selections and the cross-filter.
    pub selections: Vec<SelectionOut>,
    /// Report + page filters + selections as a filter spec (what cards are
    /// evaluated in, ignoring per-visual interactions).
    pub effective_filters: FilterSpecOut,
    /// The state equals the report's saved state.
    pub is_default: bool,
    /// Unreadable saved slicer selections.
    pub warnings: Vec<String>,
    /// Query + render wall time.
    pub duration_ms: u32,
}

/// What a click on an action button asks the caller to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CanvasAction {
    /// Back to the previous page of the navigation history.
    Back,
    /// Go to a page of the same report.
    Navigate { page: String, display_name: String },
    /// A web link: never opened automatically.
    WebUrl { url: String },
    /// An action DashCraft does not implement (bookmarks, drillthrough, Q&A).
    NotAvailable { message: String },
}

/// One line of a data point tooltip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TooltipLineOut {
    pub label: String,
    pub value: String,
}

/// Modifier keys of a click (`ctrl` is Ctrl, or ⌘ on macOS).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Canvas state of one open document.
#[derive(Default)]
pub struct CanvasStore {
    pages: Mutex<HashMap<PageKey, PageEntry>>,
    /// Theme of each report (by index), resolved once.
    themes: Mutex<HashMap<usize, Arc<ThemeResolver>>>,
}

impl fmt::Debug for CanvasStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CanvasStore")
            .field("pages", &lock(&self.pages).len())
            .finish_non_exhaustive()
    }
}

/// Interaction and view state of one page.
struct PageEntry {
    state: InteractionState,
    /// The report's saved state, for `canvas.reset`.
    initial: InteractionState,
    /// Saved slicer selections that could not be read.
    problems: Vec<String>,
    view: PageViewState,
    /// Viewport of the last `canvas.render` (later renders reuse it).
    viewport: Option<(f64, f64)>,
    /// Bumped whenever `state` changes: cached query results are stale.
    state_version: u64,
    /// Bumped by every render; a finished render is kept only if it is
    /// newer than `last`.
    generation: u32,
    /// Renders in progress, oldest first.
    running: Vec<Running>,
    /// The newest completed render.
    last: Option<Arc<LastRender>>,
}

/// A render in progress.
struct Running {
    generation: u32,
    state_version: u64,
    cancel: CancellationToken,
}

/// `a` is a later generation than `b` (wrapping counters).
fn is_newer(a: u32, b: u32) -> bool {
    a != b && a.wrapping_sub(b) < u32::MAX / 2
}

/// What the newest completed render queried and laid out.
struct LastRender {
    generation: u32,
    state_version: u64,
    snapshot: Arc<EngineSnapshot>,
    model: Arc<Model>,
    data: Arc<PageData>,
    interactions: Vec<VisualInteraction>,
}

impl LastRender {
    fn dataview(&self, visual: &str) -> Option<&DataView> {
        self.data.get(visual).and_then(|r| r.as_ref().ok())
    }

    fn interaction(&self, visual: &str) -> Option<&VisualInteraction> {
        self.interactions.iter().find(|i| i.name == visual)
    }
}

impl PageEntry {
    fn new(page: &Page) -> Self {
        let (state, problems) = InteractionState::from_page(page);
        Self {
            initial: state.clone(),
            state,
            problems,
            view: PageViewState::new(),
            viewport: None,
            state_version: 0,
            generation: 0,
            running: Vec::new(),
            last: None,
        }
    }

    /// Replaces the interaction state; `true` if it changed.
    fn set_state(&mut self, state: InteractionState) -> bool {
        if state == self.state {
            return false;
        }
        self.state = state;
        self.state_version += 1;
        true
    }

    fn last(&self) -> Result<Arc<LastRender>, OpError> {
        self.last.clone().ok_or_else(|| {
            OpError::invalid_params("the page has not been rendered yet: call canvas.render first")
        })
    }
}

/// A render prepared under the store lock.
struct Job {
    key: PageKey,
    generation: u32,
    state: InteractionState,
    is_default: bool,
    problems: Vec<String>,
    view: PageViewState,
    viewport: Option<(f64, f64)>,
    state_version: u64,
    cached: Option<Arc<PageData>>,
    cancel: CancellationToken,
}

impl CanvasStore {
    fn update<R>(&self, key: PageKey, page: &Page, f: impl FnOnce(&mut PageEntry) -> R) -> R {
        let mut pages = lock(&self.pages);
        f(pages.entry(key).or_insert_with(|| PageEntry::new(page)))
    }

    /// Starts a render: a newer generation; cancels the running renders of
    /// an older interaction state (and the oldest beyond [`MAX_RUNNING`]).
    fn begin(&self, key: PageKey, page: &Page, env: &QueryEnv) -> Job {
        self.update(key, page, |entry| {
            entry.generation = entry.generation.wrapping_add(1);
            let version = entry.state_version;
            entry.running.retain(|r| {
                let current = r.state_version == version;
                if !current {
                    r.cancel.cancel();
                }
                current
            });
            while entry.running.len() >= MAX_RUNNING {
                let oldest = entry.running.remove(0);
                oldest.cancel.cancel();
            }
            let cancel = CancellationToken::new();
            entry.running.push(Running {
                generation: entry.generation,
                state_version: version,
                cancel: cancel.clone(),
            });
            let cached = entry
                .last
                .as_ref()
                .filter(|l| {
                    l.state_version == entry.state_version
                        && Arc::ptr_eq(&l.snapshot, &env.snapshot)
                        && Arc::ptr_eq(&l.model, &env.model)
                })
                .map(|l| Arc::clone(&l.data));
            Job {
                key,
                generation: entry.generation,
                is_default: entry.state == entry.initial,
                state: entry.state.clone(),
                problems: entry.problems.clone(),
                view: entry.view.clone(),
                viewport: entry.viewport,
                state_version: entry.state_version,
                cached,
                cancel,
            }
        })
    }

    /// Keeps a finished render if it was not cancelled and is newer than
    /// the newest completed one (never moves the page backwards).
    fn finish(&self, job: &Job, last: LastRender) -> Result<(), OpError> {
        let mut pages = lock(&self.pages);
        let entry = pages.get_mut(&job.key).ok_or_else(superseded)?;
        entry.running.retain(|r| r.generation != job.generation);
        let older = entry
            .last
            .as_ref()
            .is_some_and(|l| !is_newer(job.generation, l.generation));
        if job.cancel.is_cancelled() || older {
            return Err(superseded());
        }
        entry.last = Some(Arc::new(last));
        Ok(())
    }

    fn theme(&self, session: &Session, index: usize) -> Arc<ThemeResolver> {
        let mut themes = lock(&self.themes);
        Arc::clone(
            themes
                .entry(index)
                .or_insert_with(|| Arc::new(report_theme(session.opened(), index))),
        )
    }

    fn has_render(&self, key: PageKey) -> bool {
        lock(&self.pages)
            .get(&key)
            .is_some_and(|e| e.last.is_some())
    }

    /// Cancels every running render (the document is closed).
    pub fn cancel_all(&self) {
        for entry in lock(&self.pages).values_mut() {
            for r in entry.running.drain(..) {
                r.cancel.cancel();
            }
        }
    }
}

/// The theme of report `index`: a PBIR report's theme files, else (PBIX
/// layouts, PBIR-Legacy) the base and custom theme resources read from the
/// container (the built-in default when they are missing or unreadable).
#[must_use]
pub fn report_theme(opened: &OpenedDocument, index: usize) -> ThemeResolver {
    match opened.report_sources.get(index) {
        Some(ReportSource::Pbir { report, .. }) => ThemeResolver::from_pbir(report),
        _ => {
            let (base, custom) = report_theme_values(opened, index);
            ThemeResolver::new(Theme::from_values(base.as_ref(), custom.as_ref()))
        }
    }
}

/// The base and custom theme JSON of report `index`, when readable.
#[must_use]
pub fn report_theme_values(
    opened: &OpenedDocument,
    index: usize,
) -> (Option<serde_json::Value>, Option<serde_json::Value>) {
    let Some(report) = opened.document.reports.get(index) else {
        return (None, None);
    };
    let folder = opened
        .report_sources
        .get(index)
        .map(ReportSource::folder)
        .unwrap_or_default();
    let read = |resource: Option<&ResourceRef>| {
        resource
            .and_then(|r| read_theme_resource(opened, folder, report, r))
            .map(serde_json::Value::Object)
    };
    (
        read(report.theme.base_theme.as_ref()),
        read(report.theme.custom_theme.as_ref()),
    )
}

fn read_theme_resource(
    opened: &OpenedDocument,
    folder: &str,
    report: &Report,
    resource: &ResourceRef,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    let relative = report.static_resource_path(resource)?;
    let path = if folder.is_empty() {
        relative
    } else {
        format!("{}/{relative}", folder.trim_end_matches('/'))
    };
    let container = opened.container.as_ref();
    if container
        .entry(&path)
        .is_none_or(|e| e.size > MAX_THEME_BYTES)
    {
        return None;
    }
    let text = container.read_text(&path).ok()?.text;
    match serde_json::from_str(&text).ok()? {
        serde_json::Value::Object(map) => Some(map),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Targets and renders
// ---------------------------------------------------------------------------

/// A page of an open document.
pub struct Target {
    entry: Arc<DocEntry>,
    report: usize,
    page: usize,
}

impl fmt::Debug for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Target")
            .field("doc", &self.entry.id())
            .field("report", &self.report)
            .field("page", &self.page)
            .finish()
    }
}

impl Target {
    /// Resolves `report` / `page` (see [`resolve::report_index`],
    /// [`resolve::page_index`]).
    ///
    /// # Errors
    /// `notFound`, `noDocument`, `invalidParams`.
    pub fn resolve(
        entry: Arc<DocEntry>,
        report: Option<&str>,
        page: Option<&str>,
    ) -> Result<Self, OpError> {
        let session = entry.session();
        let report = resolve::report_index(session, report)?;
        let page = resolve::page_index(resolve::report_at(session, report)?, page)?;
        Ok(Self {
            entry,
            report,
            page,
        })
    }

    fn key(&self) -> PageKey {
        (self.report, self.page)
    }

    fn report_and_page(&self) -> Result<(&Report, &Page), OpError> {
        locate(self.entry.session(), self.report, self.page)
    }

    fn theme(&self) -> Arc<ThemeResolver> {
        self.entry.canvas.theme(self.entry.session(), self.report)
    }

    fn update<R>(&self, f: impl FnOnce(&mut PageEntry, &Report, &Page) -> R) -> Result<R, OpError> {
        let (report, page) = self.report_and_page()?;
        Ok(self
            .entry
            .canvas
            .update(self.key(), page, |e| f(e, report, page)))
    }
}

fn locate(session: &Session, report: usize, page: usize) -> Result<(&Report, &Page), OpError> {
    let r = resolve::report_at(session, report)?;
    let p = r
        .pages
        .get(page)
        .ok_or_else(|| OpError::not_found("the page is gone"))?;
    Ok((r, p))
}

/// Renders the target page with its current state.
async fn render(target: &Target) -> Result<CanvasPageOut, OpError> {
    let session = target.entry.session().clone();
    let env = QueryEnv::from_session(&session);
    let job = {
        let (_, page) = target.report_and_page()?;
        target.entry.canvas.begin(target.key(), page, &env)
    };
    let theme = target.theme();
    let entry = Arc::clone(&target.entry);
    let (ri, pi) = (target.report, target.page);
    tokio::task::spawn_blocking(move || {
        let (dto, last) = run_job(&entry, env, ri, pi, &theme, &job)?;
        entry.canvas.finish(&job, last)?;
        Ok(dto)
    })
    .await?
}

/// Renders the page first if it was never rendered (clicks need the data
/// the caller saw).
async fn ensure_rendered(target: &Target) -> Result<(), OpError> {
    if !target.entry.canvas.has_render(target.key()) {
        render(target).await?;
    }
    Ok(())
}

fn run_job(
    entry: &DocEntry,
    env: QueryEnv,
    report_index: usize,
    page_index: usize,
    theme: &ThemeResolver,
    job: &Job,
) -> Result<(CanvasPageOut, LastRender), OpError> {
    let started = Instant::now();
    let (report, page) = locate(entry.session(), report_index, page_index)?;
    let limits = QueryLimits {
        cancel: job.cancel.clone(),
        ..QueryLimits::default()
    };
    let data = match &job.cached {
        Some(d) => Arc::clone(d),
        None => Arc::new(query_page_data(&env, report, page, &job.state, &limits)),
    };
    if job.cancel.is_cancelled() {
        return Err(superseded());
    }
    let opts = PageRenderOptions {
        frames: FrameOptions {
            viewport: job.viewport,
            ..FrameOptions::default()
        },
        limits,
    };
    let rendered =
        render_interactive_page(report, page, theme, &job.state, &opts, &job.view, &data);
    let dto = page_dto(entry, report, page, theme, job, &data, &rendered, started);
    let last = LastRender {
        generation: job.generation,
        state_version: job.state_version,
        snapshot: env.snapshot,
        model: env.model,
        data,
        interactions: rendered.interactions,
    };
    Ok((dto, last))
}

/// NaN / ±∞ → 0.
fn finite(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

fn rect_out(r: &Rect) -> RectOut {
    RectOut {
        x: finite(r.x),
        y: finite(r.y),
        width: finite(r.width),
        height: finite(r.height),
    }
}

fn hit_outs(regions: Vec<HitRegion>) -> Vec<HitRegionOut> {
    regions
        .into_iter()
        .take(MAX_HIT_REGIONS)
        .map(|r| HitRegionOut {
            id: r.id.encode(),
            rect: rect_out(&r.rect),
        })
        .collect()
}

fn ms(started: Instant) -> u32 {
    u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX)
}

#[allow(clippy::too_many_arguments)]
fn page_dto(
    entry: &DocEntry,
    report: &Report,
    page: &Page,
    theme: &ThemeResolver,
    job: &Job,
    data: &PageData,
    rendered: &InteractivePage,
    started: Instant,
) -> CanvasPageOut {
    let scene = &rendered.page.scene;
    let regions = scene.hit_regions();
    let visual_rects: HashMap<&str, Rect> = regions
        .iter()
        .filter(|r| r.id.kind == HitKind::Visual)
        .map(|r| (r.id.visual.as_str(), r.rect))
        .collect();
    let visuals = rendered
        .page
        .visuals
        .iter()
        .map(|v| CanvasVisualOut {
            name: v.name.clone(),
            visual_type: v.visual_type.clone(),
            title: page.visual(&v.name).map(visual_label),
            rect: visual_rects.get(v.name.as_str()).map(rect_out),
            hidden: v.hidden,
            error: v.error.clone(),
            tooltip: v.tooltip.clone(),
            warnings: v.warnings.clone(),
            truncated: v.truncated,
            has_action: v.action.is_some(),
        })
        .collect();
    let interactions = rendered
        .interactions
        .iter()
        .map(|i| interaction_out(i, job))
        .collect();
    let (selections, effective) = selections(report, page, theme, &job.state, data);
    let overlay = rendered.overlay.as_ref();
    CanvasPageOut {
        doc_id: entry.id().to_owned(),
        report: report.name.clone(),
        page: page.name.clone(),
        page_display_name: page.display_name.clone(),
        generation: job.generation,
        svg: to_svg(scene, &SvgOptions::default()),
        overlay_svg: overlay.map(|s| to_svg(s, &SvgOptions::default())),
        width: finite(scene.width),
        height: finite(scene.height),
        hit_regions: hit_outs(regions),
        overlay_hit_regions: overlay
            .map(|s| hit_outs(s.hit_regions()))
            .unwrap_or_default(),
        visuals,
        interactions,
        selections,
        effective_filters: FilterSpecOut::from(&effective),
        is_default: job.is_default,
        warnings: job.problems.clone(),
        duration_ms: ms(started),
    }
}

fn interaction_out(i: &VisualInteraction, job: &Job) -> CanvasInteractionOut {
    let kind = if is_slicer_type(&i.visual_type) {
        "slicer"
    } else if matches!(i.visual_type.as_str(), "pivotTable" | "matrix") {
        "matrix"
    } else {
        "table"
    };
    let view = job.view.get(&i.name);
    let open = view.is_some_and(|v| v.slicer.dropdown_open);
    let slicer = i.slicer.as_ref();
    CanvasInteractionOut {
        visual: i.name.clone(),
        kind: kind.to_owned(),
        scroll: i.scroll.map(|s| ScrollOut {
            x: finite(s.x),
            y: finite(s.y),
            max_x: finite(s.max_x),
            max_y: finite(s.max_y),
            viewport: rect_out(&s.viewport),
        }),
        slicer_mode: slicer.map(|s| {
            match s.mode {
                SlicerMode::Dropdown => "dropdown",
                SlicerMode::List => "list",
                SlicerMode::Tile => "tile",
                SlicerMode::Range => "range",
            }
            .to_owned()
        }),
        header: slicer.and_then(|s| s.header.as_ref()).map(rect_out),
        dropdown: slicer.and_then(|s| s.dropdown.as_ref()).map(rect_out),
        popup: slicer
            .and_then(|s| s.popup.as_ref())
            .filter(|_| open)
            .map(rect_out),
        search: slicer
            .filter(|s| s.mode != SlicerMode::Dropdown || open)
            .and_then(|s| s.search.as_ref())
            .map(rect_out),
        dropdown_open: open,
        search_text: view.map(|v| v.slicer.search.clone()).unwrap_or_default(),
        has_selection: job
            .state
            .slicer_selections
            .get(&i.name)
            .is_some_and(|s| !s.is_all()),
    }
}

// ---------------------------------------------------------------------------
// Operations (one per canvas command)
// ---------------------------------------------------------------------------

/// `canvas.render`: renders a page with its interaction state; `viewport`
/// fits the page into it (kept for later renders).
///
/// # Errors
/// `cancelled` when superseded, `notFound`.
pub async fn render_page(
    target: &Target,
    viewport: Option<(f64, f64)>,
) -> Result<CanvasPageOut, OpError> {
    let viewport = viewport.filter(|(w, h)| w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0);
    if viewport.is_some() {
        target.update(|entry, _, _| entry.viewport = viewport)?;
    }
    render(target).await
}

/// Result of a state change: whether it changed and the page.
#[derive(Debug)]
pub struct Changed {
    pub changed: bool,
    pub page: Option<CanvasPageOut>,
    pub action: Option<CanvasAction>,
}

async fn finish_change(
    target: &Target,
    changed: bool,
    render_unchanged: bool,
    action: Option<CanvasAction>,
) -> Result<Changed, OpError> {
    let page = if changed || render_unchanged {
        Some(render(target).await?)
    } else {
        None
    };
    Ok(Changed {
        changed,
        page,
        action,
    })
}

/// `canvas.click`: applies a click on `hit` (`None`: the empty canvas).
///
/// # Errors
/// `invalidParams` for a malformed hit id, `notFound`.
pub async fn click(
    target: &Target,
    hit: Option<&str>,
    mods: Modifiers,
    render_unchanged: bool,
) -> Result<Changed, OpError> {
    let hit = hit.map(decode_hit).transpose()?;
    ensure_rendered(target).await?;
    let theme = target.theme();
    let (changed, action) = target.update(|entry, report, page| {
        apply_click(entry, report, page, &theme, hit.as_ref(), mods)
    })??;
    finish_change(target, changed, render_unchanged, action).await
}

/// `canvas.scroll`: scrolls a table, matrix or slicer list by a delta in
/// page units.
///
/// # Errors
/// `notFound`.
pub async fn scroll(
    target: &Target,
    visual: &str,
    dx: f64,
    dy: f64,
    render_unchanged: bool,
) -> Result<Changed, OpError> {
    {
        let (_, page) = target.report_and_page()?;
        resolve::visual(page, visual)?;
    }
    ensure_rendered(target).await?;
    let changed = target.update(|entry, _, _| apply_scroll(entry, visual, dx, dy))??;
    finish_change(target, changed, render_unchanged, None).await
}

/// `canvas.toggleMatrixRow`: expands or collapses the matrix row of a cell
/// hit.
///
/// # Errors
/// `invalidParams` when the hit is not a matrix cell.
pub async fn toggle_matrix_row(target: &Target, hit: &str) -> Result<CanvasPageOut, OpError> {
    let hit = decode_hit(hit)?;
    ensure_rendered(target).await?;
    target.update(|entry, _, _| apply_matrix_toggle(entry, &hit))??;
    render(target).await
}

/// `canvas.clearSelections`: `visual` = a slicer clears its selection (or
/// the cross-filter source its points); `None` clears the cross-filter and
/// closes the dropdowns.
///
/// # Errors
/// `notFound` for an unknown visual.
pub async fn clear_selections(
    target: &Target,
    visual: Option<&str>,
) -> Result<CanvasPageOut, OpError> {
    target.update(|entry, _, page| {
        let mut state = entry.state.clone();
        match visual {
            Some(v) => {
                resolve::visual(page, v)?;
                state.clear_slicer(v);
                if state
                    .selected_points
                    .as_ref()
                    .is_some_and(|p| p.source == v)
                {
                    state.selected_points = None;
                }
            }
            None => {
                state.selected_points = None;
                entry.view.close_dropdowns();
            }
        }
        entry.set_state(state);
        Ok::<(), OpError>(())
    })??;
    render(target).await
}

/// `canvas.setSlicer`: replaces a slicer's selection (empty `values` and no
/// `invert` selects all).
///
/// # Errors
/// `invalidParams`, `notFound`.
pub async fn set_slicer(
    target: &Target,
    visual: Option<&str>,
    field: Option<&str>,
    values: Vec<serde_json::Value>,
    invert: bool,
) -> Result<CanvasPageOut, OpError> {
    target.update(|entry, _, page| {
        let slicer = resolve::slicer(page, visual, field)?;
        let mut state = entry.state.clone();
        if values.is_empty() && !invert {
            state.clear_slicer(&slicer.name);
        } else {
            state.select(slicer.name.clone(), selection_of(values, invert)?);
        }
        entry.set_state(state);
        Ok::<(), OpError>(())
    })??;
    render(target).await
}

/// A slicer selection of JSON values (`null` = BLANK).
///
/// # Errors
/// `invalidParams` for values that are not scalars.
pub fn selection_of(
    values: Vec<serde_json::Value>,
    invert: bool,
) -> Result<SlicerSelection, OpError> {
    serde_json::from_value(serde_json::json!({ "values": values, "inverted": invert }))
        .map_err(|e| OpError::invalid_params(format!("slicer values: {e}")))
}

/// `canvas.setSlicerSearch`: filters a slicer's list by a search text.
///
/// # Errors
/// `invalidParams` when the visual is not a slicer.
pub async fn set_slicer_search(
    target: &Target,
    visual: &str,
    text: &str,
) -> Result<CanvasPageOut, OpError> {
    target.update(|entry, _, page| {
        resolve::slicer(page, Some(visual), None)?;
        let slicer = &mut entry.view.visual_mut(visual).slicer;
        slicer.search = text.chars().take(MAX_SEARCH_CHARS).collect();
        slicer.scroll = 0.0;
        Ok::<(), OpError>(())
    })??;
    render(target).await
}

/// `canvas.closePopups`: closes the open slicer dropdowns.
///
/// # Errors
/// `notFound`.
pub async fn close_popups(target: &Target, render_unchanged: bool) -> Result<Changed, OpError> {
    let changed = target.update(|entry, _, _| entry.view.close_dropdowns())?;
    finish_change(target, changed, render_unchanged, None).await
}

/// `canvas.reset`: the report's saved slicer selections, no cross-filter,
/// scroll positions and dropdowns reset.
///
/// # Errors
/// `notFound`.
pub async fn reset_page(target: &Target) -> Result<CanvasPageOut, OpError> {
    target.update(|entry, _, _| {
        let initial = entry.initial.clone();
        entry.set_state(initial);
        entry.view = PageViewState::new();
    })?;
    render(target).await
}

/// `canvas.tooltip`: the hover tooltip of a data point from the last
/// render's data (empty when there is none).
///
/// # Errors
/// `invalidParams` for a malformed hit id or a page never rendered.
pub fn tooltip(target: &Target, hit: &str) -> Result<Vec<TooltipLineOut>, OpError> {
    let hit = decode_hit(hit)?;
    let theme = target.theme();
    target.update(|entry, _, page| {
        let last = entry.last()?;
        let Some(visual) = page.visual(&hit.visual) else {
            return Ok(Vec::new());
        };
        let (HitKind::DataPoint { series, index }, Some(dv)) =
            (&hit.kind, last.dataview(&hit.visual))
        else {
            return Ok(Vec::new());
        };
        let mut lines = tooltip_lines(
            visual,
            &theme,
            dv,
            last.interaction(&hit.visual),
            *series,
            *index,
        );
        lines.truncate(MAX_TOOLTIP_LINES);
        Ok(lines)
    })?
}

/// The page's interaction and view state, as JSON.
///
/// # Errors
/// `notFound`.
pub fn state(target: &Target) -> Result<(serde_json::Value, serde_json::Value, bool), OpError> {
    target.update(|entry, _, _| {
        (
            serde_json::to_value(&entry.state).unwrap_or_default(),
            serde_json::to_value(&entry.view).unwrap_or_default(),
            entry.state == entry.initial,
        )
    })
}

fn decode_hit(text: &str) -> Result<HitId, OpError> {
    HitId::decode(text).ok_or_else(|| {
        OpError::invalid_params(format!("malformed hit id `{}`", truncate(text, 80)))
    })
}

// ---------------------------------------------------------------------------
// Click logic
// ---------------------------------------------------------------------------

fn apply_click(
    entry: &mut PageEntry,
    report: &Report,
    page: &Page,
    theme: &ThemeResolver,
    hit: Option<&HitId>,
    mods: Modifiers,
) -> Result<(bool, Option<CanvasAction>), OpError> {
    let mut changed = false;
    let mut action = None;
    let Some(hit) = hit else {
        // The empty canvas: close the popups and clear the cross-filter.
        changed |= entry.view.close_dropdowns();
        let mut state = entry.state.clone();
        state.selected_points = None;
        changed |= entry.set_state(state);
        return Ok((changed, action));
    };
    let visual = resolve::visual(page, &hit.visual)?;
    let slicer = is_slicer_type(&visual.visual_type);
    // Any click outside a slicer closes its dropdown.
    let others: Vec<String> = entry
        .view
        .open_dropdowns()
        .filter(|n| *n != visual.name)
        .map(str::to_owned)
        .collect();
    for name in others {
        entry.view.visual_mut(&name).slicer.dropdown_open = false;
        changed = true;
    }
    match &hit.kind {
        HitKind::Button if slicer => {
            let s = &mut entry.view.visual_mut(&visual.name).slicer;
            s.dropdown_open = !s.dropdown_open;
            s.scroll = 0.0;
            changed = true;
        }
        HitKind::SlicerItem { index } if slicer => {
            let last = entry.last()?;
            let dv = last.dataview(&visual.name).ok_or_else(|| {
                OpError::invalid_params(format!("slicer {} has no data to click", visual.name))
            })?;
            let current = entry.state.slicer_selections.get(&visual.name);
            let (next, single) = slicer_click(current, visual, theme, dv, *index, mods.ctrl)?;
            let mut state = entry.state.clone();
            if next == SlicerSelection::default() {
                state.clear_slicer(&visual.name);
            } else {
                state.select(visual.name.clone(), next);
            }
            changed |= entry.set_state(state);
            if single {
                let s = &mut entry.view.visual_mut(&visual.name).slicer;
                changed |= std::mem::take(&mut s.dropdown_open);
            }
        }
        HitKind::Button => {
            if let Some(a) = button::action(visual, theme) {
                action = button_action(entry, report, a, &mut changed);
            }
        }
        HitKind::DataPoint { series, index } => {
            let last = entry.last()?;
            let Some(dv) = last.dataview(&visual.name) else {
                return Ok((changed, action));
            };
            let point = point_fields(visual, dv, last.interaction(&visual.name), *series, *index);
            let mut state = entry.state.clone();
            state.selected_points = match point {
                Some(point) => {
                    next_points(state.selected_points.take(), &visual.name, point, mods.ctrl)
                }
                // Totals and cells without a group clear the selection.
                None => state
                    .selected_points
                    .take()
                    .filter(|p| p.source != visual.name),
            };
            changed |= entry.set_state(state);
        }
        HitKind::Visual | HitKind::SlicerItem { .. } => {
            // A click on a visual's background clears its own selection.
            if entry
                .state
                .selected_points
                .as_ref()
                .is_some_and(|p| p.source == visual.name)
            {
                let mut state = entry.state.clone();
                state.selected_points = None;
                changed |= entry.set_state(state);
            }
        }
    }
    Ok((changed, action))
}

fn button_action(
    entry: &mut PageEntry,
    report: &Report,
    action: ButtonAction,
    changed: &mut bool,
) -> Option<CanvasAction> {
    let unavailable = |what: &str| {
        Some(CanvasAction::NotAvailable {
            message: format!("{what} is not available in DashCraft"),
        })
    };
    match action {
        ButtonAction::Back => Some(CanvasAction::Back),
        ButtonAction::PageNavigation { page } => match report.find_page(&page) {
            Some(p) => Some(CanvasAction::Navigate {
                page: p.name.clone(),
                display_name: p.display_name.clone(),
            }),
            None if page.is_empty() => unavailable("This page navigation (no destination)"),
            None => Some(CanvasAction::NotAvailable {
                message: format!(
                    "The destination page '{}' is not in this report",
                    truncate(&page, 80)
                ),
            }),
        },
        ButtonAction::Drillthrough { .. } => unavailable("Drillthrough"),
        ButtonAction::Bookmark { .. } => unavailable("Bookmarks"),
        ButtonAction::QnA => unavailable("Q&A"),
        ButtonAction::WebUrl { url } => Some(CanvasAction::WebUrl {
            url: truncate(&url, 2048),
        }),
        // Slicer selections apply immediately: nothing is pending.
        ButtonAction::ApplyAllSlicers => None,
        ButtonAction::ClearAllSlicers => {
            let mut state = entry.state.clone();
            state.slicer_selections.clear();
            *changed |= entry.set_state(state);
            None
        }
        ButtonAction::Other { kind } => {
            unavailable(&format!("The '{}' action", truncate(&kind, 40)))
        }
    }
}

fn values_match(a: &Value, b: &Value) -> bool {
    a == b || value_to_json(a) == value_to_json(b)
}

fn tuples_match(a: &[Value], b: &[Value]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| values_match(x, y))
}

/// The values of slicer row `index` (one per category column).
fn slicer_tuple(dv: &DataView, index: usize) -> Option<Vec<Value>> {
    let cat = dv.categorical.as_ref()?;
    let tuple: Option<Vec<Value>> = cat
        .categories
        .iter()
        .map(|c| c.values.get(index).map(|v| v.raw.clone()))
        .collect();
    tuple.filter(|t| !t.is_empty())
}

/// The selection after clicking slicer row `index`, and whether the slicer
/// behaves as single select (a dropdown then closes). Same rules as Power
/// BI: "Single select", "Multi-select with Ctrl" (default), plain toggles,
/// inverted "Select all" mode.
fn slicer_click(
    current: Option<&SlicerSelection>,
    visual: &Visual,
    theme: &ThemeResolver,
    dv: &DataView,
    index: usize,
    ctrl: bool,
) -> Result<(SlicerSelection, bool), OpError> {
    let props = theme.visual(visual);
    let o = props.objects();
    let strict = o.bool("selection", "strictSingleSelect").unwrap_or(false);
    let ctrl_mode = o.bool("selection", "singleSelect").unwrap_or(true);
    let inverted_mode = o
        .bool("data", "isInvertedSelectionMode")
        .or_else(|| o.bool("general", "isInvertedSelectionMode"))
        .unwrap_or(false);
    let current = current.cloned().unwrap_or_default();
    let all_checked = current.is_all() && (inverted_mode || current.inverted);
    if index == SELECT_ALL_INDEX {
        let next = if all_checked {
            SlicerSelection::default()
        } else {
            SlicerSelection {
                inverted: true,
                ..SlicerSelection::default()
            }
        };
        return Ok((next, false));
    }
    let tuple = slicer_tuple(dv, index)
        .ok_or_else(|| OpError::invalid_params(format!("the slicer has no row {index}")))?;
    let only = |values: Vec<Vec<Value>>, inverted: bool| SlicerSelection {
        values,
        inverted,
        condition: None,
    };
    let replace = strict || (ctrl_mode && !ctrl);
    if replace {
        let only_this = !current.inverted
            && current.condition.is_none()
            && current.values.len() == 1
            && current
                .values
                .first()
                .is_some_and(|t| tuples_match(t, &tuple));
        let next = if only_this && !strict {
            SlicerSelection::default()
        } else {
            only(vec![tuple], false)
        };
        return Ok((next, true));
    }
    let next = if current.condition.is_some() {
        only(vec![tuple], false)
    } else if all_checked {
        only(vec![tuple], true)
    } else {
        let mut values = current.values.clone();
        match values.iter().position(|t| tuples_match(t, &tuple)) {
            Some(i) => {
                values.remove(i);
            }
            None => values.push(tuple),
        }
        if values.is_empty() && !current.inverted {
            SlicerSelection::default()
        } else {
            only(values, current.inverted)
        }
    };
    Ok((next, false))
}

/// The model field of a DataView column (measures and aggregations have
/// none).
fn field_of(visual: &Visual, meta: &ColumnMeta) -> Option<FieldExpr> {
    if meta.is_measure {
        return None;
    }
    visual
        .projections
        .iter()
        .find(|p| p.query_ref == meta.query_ref)
        .map(|p| p.field.clone())
        .or_else(
            || match (meta.source.kind, &meta.source.entity, &meta.source.property) {
                (FieldRefKind::Column, Some(e), Some(p)) => Some(FieldExpr::column(e, p)),
                _ => None,
            },
        )
}

/// The identity of a clicked data point: chart category (+ series), table
/// row groups, matrix row path. `None` for totals.
fn point_fields(
    visual: &Visual,
    dv: &DataView,
    interaction: Option<&VisualInteraction>,
    series: usize,
    index: usize,
) -> Option<Vec<PointField>> {
    let field = |meta: Option<&ColumnMeta>, value: Value| {
        meta.and_then(|m| field_of(visual, m))
            .map(|field| PointField { field, value })
    };
    let fields: Vec<PointField> = match visual.visual_type.as_str() {
        "tableEx" | "table" => {
            let table = dv.table.as_ref()?;
            let row = table.rows.get(index)?;
            table
                .columns
                .iter()
                .zip(row)
                .filter_map(|(&mi, cell)| field(dv.meta(mi), cell.raw.clone()))
                .collect()
        }
        "pivotTable" | "matrix" => {
            let matrix = dv.matrix.as_ref()?;
            let row = interaction?.matrix_rows.get(index)?;
            matrix
                .row_levels
                .iter()
                .zip(&row.path)
                .filter_map(|(&mi, cell)| field(dv.meta(mi), cell.raw.clone()))
                .collect()
        }
        t if charts::handles(t) => {
            let chart = ChartData::from_dataview(dv);
            let mut fields: Vec<PointField> = chart
                .category_fields(index)
                .into_iter()
                .filter_map(|(m, v)| field(Some(m), v.clone()))
                .collect();
            if let (Some(meta), Some(value)) = (
                chart.series_meta.as_ref(),
                chart
                    .series
                    .get(series)
                    .and_then(|s| s.series_value.clone()),
            ) && let Some(f) = field(Some(meta), value)
            {
                fields.push(f);
            }
            fields
        }
        _ => return None,
    };
    (!fields.is_empty()).then_some(fields)
}

/// The point selection after clicking `point` in `source`.
fn next_points(
    current: Option<PointSelection>,
    source: &str,
    point: Vec<PointField>,
    ctrl: bool,
) -> Option<PointSelection> {
    let same = |a: &[PointField], b: &[PointField]| {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|(x, y)| x.field == y.field && values_match(&x.value, &y.value))
    };
    match current {
        Some(mut sel) if sel.source == source => {
            let pos = sel.points.iter().position(|p| same(p, &point));
            if ctrl {
                match pos {
                    Some(i) => {
                        sel.points.remove(i);
                    }
                    None => sel.points.push(point),
                }
                (!sel.points.is_empty()).then_some(sel)
            } else if sel.points.len() == 1 && pos == Some(0) {
                None
            } else {
                Some(PointSelection {
                    source: source.to_owned(),
                    points: vec![point],
                })
            }
        }
        _ => Some(PointSelection {
            source: source.to_owned(),
            points: vec![point],
        }),
    }
}

fn clamp(v: f64, max: f64) -> f64 {
    if v.is_finite() {
        v.clamp(0.0, max.max(0.0))
    } else {
        0.0
    }
}

fn apply_scroll(entry: &mut PageEntry, visual: &str, dx: f64, dy: f64) -> Result<bool, OpError> {
    let last = entry.last()?;
    let Some(info) = last.interaction(visual) else {
        return Ok(false);
    };
    let Some(range) = info.scroll else {
        return Ok(false);
    };
    let kind = info.visual_type.as_str();
    let view = entry.view.visual_mut(visual);
    let (x, y) = if is_slicer_type(kind) {
        (0.0, view.slicer.scroll)
    } else if matches!(kind, "pivotTable" | "matrix") {
        (view.matrix.scroll.x, view.matrix.scroll.y)
    } else {
        (view.table.scroll.x, view.table.scroll.y)
    };
    let (x0, y0) = (clamp(x, range.max_x), clamp(y, range.max_y));
    let (x1, y1) = (
        clamp(x0 + finite(dx), range.max_x),
        clamp(y0 + finite(dy), range.max_y),
    );
    if (x1 - x0).abs() < 0.01 && (y1 - y0).abs() < 0.01 {
        return Ok(false);
    }
    if is_slicer_type(kind) {
        view.slicer.scroll = y1;
    } else if matches!(kind, "pivotTable" | "matrix") {
        view.matrix.scroll = GridScroll { x: x1, y: y1 };
    } else {
        view.table.scroll = GridScroll { x: x1, y: y1 };
    }
    Ok(true)
}

/// Matrix row labels as the expansion state keys them.
fn row_label(cell: &CellValue) -> String {
    if cell.formatted.is_empty() && cell.is_blank() {
        BLANK_LABEL.to_owned()
    } else {
        cell.formatted.clone()
    }
}

fn apply_matrix_toggle(entry: &mut PageEntry, hit: &HitId) -> Result<(), OpError> {
    let HitKind::DataPoint { index, .. } = hit.kind else {
        return Err(OpError::invalid_params("not a matrix cell"));
    };
    let last = entry.last()?;
    let row = last
        .interaction(&hit.visual)
        .filter(|i| matches!(i.visual_type.as_str(), "pivotTable" | "matrix"))
        .ok_or_else(|| OpError::invalid_params(format!("{} is not a matrix", hit.visual)))?
        .matrix_rows
        .get(index)
        .ok_or_else(|| OpError::invalid_params(format!("the matrix has no row {index}")))?;
    if row.path.is_empty() {
        return Ok(());
    }
    let key: Vec<String> = row.path.iter().map(row_label).collect();
    let toggled = &mut entry.view.visual_mut(&hit.visual).matrix.toggled;
    match toggled.iter().position(|t| *t == key) {
        Some(i) => {
            toggled.remove(i);
        }
        None => toggled.push(key),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tooltips
// ---------------------------------------------------------------------------

fn cell_text(cell: Option<&CellValue>, category: bool) -> String {
    let text = match cell {
        Some(c) if !c.formatted.is_empty() => c.formatted.clone(),
        Some(c) if !c.is_blank() => c.raw.to_string(),
        _ if category => BLANK_LABEL.to_owned(),
        _ => String::new(),
    };
    truncate(&text, MAX_VALUE_CHARS)
}

fn line(label: &str, value: String) -> TooltipLineOut {
    TooltipLineOut {
        label: truncate(label, MAX_VALUE_CHARS),
        value,
    }
}

fn tooltip_lines(
    visual: &Visual,
    theme: &ThemeResolver,
    dv: &DataView,
    interaction: Option<&VisualInteraction>,
    series: usize,
    index: usize,
) -> Vec<TooltipLineOut> {
    let label = |mi: usize| {
        dv.meta(mi)
            .map(|m| m.display_name.clone())
            .unwrap_or_default()
    };
    match visual.visual_type.as_str() {
        "tableEx" | "table" => {
            let Some(table) = dv.table.as_ref() else {
                return Vec::new();
            };
            let Some(row) = table.rows.get(index) else {
                return Vec::new();
            };
            table
                .columns
                .iter()
                .zip(row)
                .map(|(&mi, cell)| line(&label(mi), cell_text(Some(cell), false)))
                .collect()
        }
        "pivotTable" | "matrix" => {
            let (Some(matrix), Some(row)) = (
                dv.matrix.as_ref(),
                interaction.and_then(|i| i.matrix_rows.get(index)),
            ) else {
                return Vec::new();
            };
            let mut lines: Vec<TooltipLineOut> = matrix
                .row_levels
                .iter()
                .zip(&row.path)
                .map(|(&mi, cell)| line(&label(mi), cell_text(Some(cell), true)))
                .collect();
            let stepped = theme
                .visual(visual)
                .objects()
                .bool("rowHeaders", "stepped")
                .unwrap_or(true);
            let header_cols = if stepped {
                1
            } else {
                matrix.row_levels.len().max(1)
            };
            if let (Some(k), Some(node)) = (
                series.checked_sub(header_cols),
                find_node(&matrix.rows, &row.path),
            ) && let Some(cell) = node.cells.get(k)
            {
                let values = matrix.values.len().max(1);
                let mut name = matrix
                    .values
                    .get(k % values)
                    .map(|&mi| label(mi))
                    .unwrap_or_default();
                if let Some(col) = matrix.columns.get(k / values) {
                    let path: Vec<String> = col.path.iter().map(row_label).collect();
                    if !path.is_empty() {
                        name = format!("{name} ({})", path.join(", "));
                    }
                }
                lines.push(line(&name, cell_text(Some(cell), false)));
            }
            lines
        }
        t if charts::handles(t) => {
            let chart = ChartData::from_dataview(dv);
            let Some(cat) = dv.categorical.as_ref() else {
                return Vec::new();
            };
            let mut lines: Vec<TooltipLineOut> = cat
                .categories
                .iter()
                .map(|c| line(&label(c.meta_idx), cell_text(c.values.get(index), true)))
                .collect();
            let s = chart.series.get(series);
            if let (Some(meta), Some(s)) = (chart.series_meta.as_ref(), s)
                && s.series_value.is_some()
            {
                let value = s
                    .name
                    .strip_suffix(&format!(" {}", s.measure_name))
                    .unwrap_or(&s.name);
                lines.push(line(&meta.display_name, truncate(value, MAX_VALUE_CHARS)));
            }
            let series_value = s.and_then(|s| s.series_value.as_ref());
            let has_role = |vc: &ValueColumn, role: &str| {
                dv.meta(vc.meta_idx)
                    .is_some_and(|m| m.roles.iter().any(|r| r == role))
            };
            let same_series = |vc: &ValueColumn| match (&vc.series, series_value) {
                (Some(a), Some(b)) => values_match(&a.raw, b),
                (None, _) => true,
                _ => false,
            };
            let value_line = |vc: &ValueColumn| {
                line(&label(vc.meta_idx), cell_text(vc.values.get(index), false))
            };
            // A scatter point (series = its Y column) also shows its X and
            // Size values, around Y as in the service.
            let scatter_field = |role: &str| {
                (visual.visual_type == "scatterChart")
                    .then(|| {
                        cat.values
                            .iter()
                            .find(|vc| has_role(vc, role) && same_series(vc))
                    })
                    .flatten()
            };
            lines.extend(scatter_field("X").map(value_line));
            if let Some(s) = s {
                lines.push(line(
                    &s.measure_name,
                    truncate(s.formatted(index), MAX_VALUE_CHARS),
                ));
            }
            lines.extend(scatter_field("Size").map(value_line));
            for vc in &cat.values {
                if has_role(vc, "Tooltips") && same_series(vc) {
                    lines.push(value_line(vc));
                }
            }
            lines
        }
        _ => Vec::new(),
    }
}

fn find_node<'a>(
    root: &'a pbi_render::dataview::MatrixNode,
    path: &[CellValue],
) -> Option<&'a pbi_render::dataview::MatrixNode> {
    let mut node = root;
    for cell in path {
        node = node.children.iter().find(|c| {
            c.value
                .as_ref()
                .is_some_and(|v| values_match(&v.raw, &cell.raw))
        })?;
    }
    Some(node)
}

// ---------------------------------------------------------------------------
// Selections and effective filters
// ---------------------------------------------------------------------------

/// `Table[Column]`-style name of a field.
fn field_name(field: &FieldExpr) -> String {
    match (field.entity(), field.property()) {
        (Some(t), Some(c)) => format!("{t}[{c}]"),
        _ => field.query_ref().unwrap_or_else(|| "(field)".into()),
    }
}

fn column_of(field: &FieldExpr) -> Option<(String, String)> {
    match field {
        FieldExpr::Column { entity, property } => Some((entity.clone(), property.clone())),
        _ => None,
    }
}

fn value_label(v: &Value) -> String {
    if v.is_blank() {
        BLANK_LABEL.to_owned()
    } else {
        truncate(&v.to_string(), MAX_VALUE_CHARS)
    }
}

/// `A`, `A or B`, `A, B, C and 2 more`.
fn list_summary(values: &[String]) -> String {
    match values {
        [] => "(none)".to_owned(),
        [a] => a.clone(),
        [a, b] => format!("{a} or {b}"),
        _ if values.len() <= MAX_SUMMARY_VALUES => values.join(", "),
        _ => format!(
            "{} and {} more",
            values
                .iter()
                .take(MAX_SUMMARY_VALUES)
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
            values.len() - MAX_SUMMARY_VALUES
        ),
    }
}

fn slicer_label(visual: &Visual, theme: &ThemeResolver) -> String {
    theme
        .visual(visual)
        .objects()
        .text("header", "text")
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| slicer_projection_labels(visual).join(", "))
}

fn slicer_projection_labels(visual: &Visual) -> Vec<String> {
    let all: Vec<_> = visual.projections_for("Values").collect();
    let any_active = all.iter().any(|p| p.active == Some(true));
    all.into_iter()
        .filter(|p| !any_active || p.active == Some(true))
        .map(|p| p.label().to_owned())
        .collect()
}

fn non_empty(text: Option<&str>) -> Option<String> {
    text.map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

/// A visual's explicit title: its display name, title text, or a textbox's
/// text.
fn visual_title(v: &Visual) -> Option<String> {
    if let Some(name) = non_empty(v.display_name.as_deref()) {
        return Some(truncate(&name, MAX_TITLE_CHARS));
    }
    for objects in [&v.container_objects, &v.objects] {
        let raw = objects
            .pointer("/title/0/properties/text/expr/Literal/Value")
            .and_then(serde_json::Value::as_str);
        if let Some(Ok(LiteralValue::Text(text))) = raw.map(|r| Literal::new(r).value())
            && !text.trim().is_empty()
        {
            return Some(truncate(text.trim(), MAX_TITLE_CHARS));
        }
    }
    let paragraphs = v
        .objects
        .pointer("/general/0/properties/paragraphs")
        .and_then(serde_json::Value::as_array)?;
    let mut text = String::new();
    for run in paragraphs
        .iter()
        .filter_map(|p| p.get("textRuns").and_then(serde_json::Value::as_array))
        .flatten()
    {
        if let Some(value) = run.get("value").and_then(serde_json::Value::as_str) {
            if !text.is_empty() && !text.ends_with(' ') {
                text.push(' ');
            }
            text.push_str(value.trim());
        }
        if text.chars().count() > MAX_TITLE_CHARS {
            break;
        }
    }
    non_empty(Some(&text)).map(|t| truncate(&t, MAX_TITLE_CHARS))
}

/// The visual's title (its title text, else the automatic "<values> by
/// <category>" title, else its type).
#[must_use]
pub fn visual_label(visual: &Visual) -> String {
    visual_title(visual)
        .or_else(|| Some(auto_title(visual)).filter(|t| !t.trim().is_empty()))
        .map_or_else(
            || visual.visual_type.clone(),
            |t| truncate(&t, MAX_VALUE_CHARS),
        )
}

/// The page's slicer selections and cross-filter as Filters pane entries,
/// and the effective filter spec (report + page filters + selections).
fn selections(
    report: &Report,
    page: &Page,
    theme: &ThemeResolver,
    state: &InteractionState,
    data: &PageData,
) -> (Vec<SelectionOut>, FilterSpec) {
    let mut spec = filters_from_report(report);
    spec.extend(filters_from_page(page));
    let mut out = Vec::new();
    for v in page
        .visuals
        .iter()
        .filter(|v| is_slicer_type(&v.visual_type))
    {
        let Some(sel) = state.slicer_selections.get(&v.name).filter(|s| !s.is_all()) else {
            continue;
        };
        let fields = slicer_fields(v);
        let names: Vec<String> = fields.iter().map(field_name).collect();
        let label = slicer_label(v, theme);
        let unsupported = |reason: &str| UnsupportedFilter {
            name: format!("Slicer: {label}"),
            kind: "Slicer".into(),
            field: fields.first().and_then(FieldExpr::query_ref),
            reason: reason.to_owned(),
        };
        let formatted = data
            .get(&v.name)
            .and_then(|r| r.as_ref().ok())
            .and_then(|dv| dv.slicer.as_ref())
            .filter(|s| s.values.len() == sel.values.len())
            .map(|s| {
                s.values
                    .iter()
                    .map(|t| {
                        t.iter()
                            .map(|c| cell_text(Some(c), true))
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .collect::<Vec<_>>()
            });
        let texts = formatted.unwrap_or_else(|| {
            sel.values
                .iter()
                .map(|t| t.iter().map(value_label).collect::<Vec<_>>().join(", "))
                .collect()
        });
        let mut summary = if sel.values.is_empty() {
            String::new()
        } else if sel.inverted {
            format!("is not {}", list_summary(&texts))
        } else {
            format!("is {}", list_summary(&texts))
        };
        if sel.condition.is_some() {
            if !summary.is_empty() {
                summary.push_str("; ");
            }
            summary.push_str("has a range or advanced condition");
            spec.unsupported.push(unsupported(
                "range and advanced slicer conditions are not translated",
            ));
        }
        if !sel.values.is_empty() {
            match (fields.as_slice(), fields.first().and_then(column_of)) {
                ([_], Some((table, column))) => spec.filters.push(ColumnFilterSpec {
                    table,
                    column,
                    values: sel
                        .values
                        .iter()
                        .filter_map(|t| t.first())
                        .map(value_to_json)
                        .collect(),
                    exclude: sel.inverted,
                }),
                _ => spec.unsupported.push(unsupported(
                    "selections over several fields are not translated",
                )),
            }
        }
        out.push(SelectionOut {
            visual: v.name.clone(),
            kind: SelectionKind::Slicer,
            label,
            fields: names,
            summary,
        });
    }
    if let Some(points) = state
        .selected_points
        .as_ref()
        .filter(|p| !p.points.is_empty())
    {
        let source = page.visual(&points.source);
        let label = source.map_or_else(|| points.source.clone(), visual_label);
        let first = points.points.first().map(Vec::as_slice).unwrap_or_default();
        let fields: Vec<FieldExpr> = first.iter().map(|p| p.field.clone()).collect();
        let texts: Vec<String> = points
            .points
            .iter()
            .map(|p| {
                p.iter()
                    .map(|f| value_label(&f.value))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .collect();
        let uniform = points
            .points
            .iter()
            .all(|p| p.len() == fields.len() && p.iter().zip(&fields).all(|(a, b)| a.field == *b));
        let columns: Option<Vec<(String, String)>> = fields.iter().map(column_of).collect();
        match columns {
            Some(columns) if uniform && (points.points.len() == 1 || fields.len() == 1) => {
                for (i, (table, column)) in columns.into_iter().enumerate() {
                    let mut values: Vec<serde_json::Value> = Vec::new();
                    for p in &points.points {
                        if let Some(f) = p.get(i) {
                            let json = value_to_json(&f.value);
                            if !values.contains(&json) {
                                values.push(json);
                            }
                        }
                    }
                    spec.filters.push(ColumnFilterSpec {
                        table,
                        column,
                        values,
                        exclude: false,
                    });
                }
            }
            _ => spec.unsupported.push(UnsupportedFilter {
                name: format!("Selection in {label}"),
                kind: "CrossFilter".into(),
                field: fields.first().and_then(FieldExpr::query_ref),
                reason: "this selection cannot be expressed as column filters".into(),
            }),
        }
        out.push(SelectionOut {
            visual: points.source.clone(),
            kind: SelectionKind::CrossFilter,
            label,
            fields: fields.iter().map(field_name).collect(),
            summary: format!("is {}", list_summary(&texts)),
        });
    }
    (out, spec)
}

#[cfg(test)]
mod tests {
    use pbi_engine::Engine;

    use super::*;

    fn env() -> QueryEnv {
        let model = Model::default();
        QueryEnv::new(Engine::new(&model).snapshot(), Arc::new(model), "en-US")
    }

    fn done(job: &Job, env: &QueryEnv) -> LastRender {
        LastRender {
            generation: job.generation,
            state_version: job.state_version,
            snapshot: Arc::clone(&env.snapshot),
            model: Arc::clone(&env.model),
            data: Arc::new(PageData::default()),
            interactions: Vec::new(),
        }
    }

    fn shown(store: &CanvasStore, key: PageKey) -> Option<u32> {
        lock(&store.pages)
            .get(&key)
            .and_then(|e| e.last.as_ref().map(|l| l.generation))
    }

    #[test]
    fn newest_completed_render_wins_and_never_regresses() {
        let (store, env, page, key) = (CanvasStore::default(), env(), Page::default(), (1, 1));
        // Two renders of the same state (a scroll during a scroll): neither is cancelled.
        let a = store.begin(key, &page, &env);
        let b = store.begin(key, &page, &env);
        assert!(!a.cancel.is_cancelled());
        // The newer one finishes first; the older one can no longer replace it.
        assert!(store.finish(&b, done(&b, &env)).is_ok());
        let err = store.finish(&a, done(&a, &env)).err().map(|e| e.code);
        assert_eq!(err, Some(crate::OpErrorCode::Cancelled));
        assert_eq!(shown(&store, key), Some(b.generation));

        // In order: the older render is shown while the newer one is still running.
        let c = store.begin(key, &page, &env);
        let d = store.begin(key, &page, &env);
        assert!(store.finish(&c, done(&c, &env)).is_ok());
        assert_eq!(shown(&store, key), Some(c.generation));
        assert!(store.finish(&d, done(&d, &env)).is_ok());
        assert_eq!(shown(&store, key), Some(d.generation));

        // A new interaction state cancels the renders of the old one.
        let e = store.begin(key, &page, &env);
        store.update(key, &page, |entry| {
            let mut state = entry.state.clone();
            state.selected_points = Some(PointSelection::default());
            assert!(entry.set_state(state));
        });
        let f = store.begin(key, &page, &env);
        assert!(e.cancel.is_cancelled() && !f.cancel.is_cancelled());
        assert!(store.finish(&e, done(&e, &env)).is_err());
        assert!(store.finish(&f, done(&f, &env)).is_ok());
        assert_eq!(shown(&store, key), Some(f.generation));

        // At most MAX_RUNNING renders of one state run at once: the oldest is cancelled.
        let running: Vec<Job> = (0..=MAX_RUNNING)
            .map(|_| store.begin(key, &page, &env))
            .collect();
        assert!(running.first().is_some_and(|j| j.cancel.is_cancelled()));
        assert!(running.iter().skip(1).all(|j| !j.cancel.is_cancelled()));

        // Closing the document cancels whatever still runs.
        let g = store.begin(key, &page, &env);
        store.cancel_all();
        assert!(g.cancel.is_cancelled());
        assert!(running.iter().all(|j| j.cancel.is_cancelled()));
        assert!(is_newer(0, u32::MAX) && !is_newer(u32::MAX, 0) && !is_newer(3, 3));
    }

    #[test]
    fn cached_query_results_are_reused_while_state_and_data_are_unchanged() {
        let (store, env, page, key) = (CanvasStore::default(), env(), Page::default(), (0, 0));
        let a = store.begin(key, &page, &env);
        assert!(a.cached.is_none());
        let data = Arc::new(PageData {
            id: PageData::next_id(),
            ..PageData::default()
        });
        let last = LastRender {
            data: Arc::clone(&data),
            ..done(&a, &env)
        };
        assert!(store.finish(&a, last).is_ok());
        // Same state and snapshot: the same results (same render-cache id) are reused.
        let b = store.begin(key, &page, &env);
        assert!(b.cached.as_ref().is_some_and(|c| Arc::ptr_eq(c, &data)));
        // A new snapshot (data changed) or a new interaction state invalidates them.
        let other = super::tests::env();
        assert!(store.begin(key, &page, &other).cached.is_none());
        store.update(key, &page, |entry| {
            let mut state = entry.state.clone();
            state.selected_points = Some(PointSelection::default());
            entry.set_state(state);
        });
        assert!(store.begin(key, &page, &env).cached.is_none());
    }

    #[test]
    fn summaries_list_a_few_values() {
        assert_eq!(list_summary(&[]), "(none)");
        assert_eq!(list_summary(&["A".into(), "B".into()]), "A or B");
        let many: Vec<String> = ["A", "B", "C", "D", "E"].map(String::from).to_vec();
        assert_eq!(list_summary(&many), "A, B, C and 2 more");
    }

    #[test]
    fn slicer_values_become_selections() {
        let sel = selection_of(
            vec![serde_json::json!("West"), serde_json::Value::Null],
            true,
        );
        let sel = sel.unwrap_or_default();
        assert!(sel.inverted);
        assert_eq!(sel.values.len(), 2);
        assert!(selection_of(vec![serde_json::json!({"a": 1})], false).is_err());
    }
}
