//! Page orchestration: query every visible visual, lay out frames, render
//! visuals into one scene, and collect per-visual information (timings,
//! warnings, errors, button actions). (owner: visuals-basics)
//!
//! Hidden visuals (`isHidden`, or inside a hidden group) are neither
//! queried nor drawn; groups only paint their container (see
//! [`crate::layout`]). Timings are wall-clock and only reported in
//! [`VisualRenderInfo`]; the scene itself is deterministic.
//!
//! # Interactive canvas
//!
//! [`render_interactive_page`] renders a page with a per-visual view state
//! ([`PageViewState`]: table / matrix scroll offsets and row expansion, open
//! slicer dropdowns with their search text and list scroll) from query
//! results computed separately by [`query_page_data`], so view-only changes
//! (scrolling, opening a dropdown) re-render without re-querying. It also
//! returns the open dropdown popups as an overlay scene (drawn above the
//! page and not clipped to it) and the interactive geometry of each slicer,
//! table and matrix ([`VisualInteraction`]: scroll range, slicer rectangles,
//! matrix rows). [`render_report_page`] is the same pipeline with the
//! default view state and no overlay.
//!
//! Interactive renders reuse unchanged visuals: each visual's content,
//! geometry and popup are cached by a content fingerprint of its inputs
//! ([`super::cache`]), so scrolling one table or changing one slicer only
//! re-renders the visuals whose definition, frame, data, view state or
//! selection changed. The output is identical with or without the cache
//! ([`render_interactive_page_cached`] with `None` renders everything).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use indexmap::IndexMap;
use pbi_model::{Page, Report, Visual};
use serde::{Deserialize, Serialize};

use crate::dataview::DataView;
use crate::fingerprint::Fingerprint;
use crate::layout::{FrameOptions, LayoutOptions, PageLayout, render_page_with};
use crate::query::{InteractionState, QueryEnv, QueryLimits, VisualQueryError};
use crate::scene::{Group, Rect, Scene};
use crate::text::FontBook;
use crate::theme::ThemeResolver;

use super::button::{self, ButtonAction};
use super::cache::{CachedVisual, GridKey, VisualCache, VisualKey, data_fingerprint};
use super::common::error_tooltip;
use super::matrix::{self, MatrixOptions, MatrixRowRef};
use super::slicer::{self, SlicerLayout, SlicerMode, SlicerOptions};
use super::table::{self, TableOptions};
use super::{VisualCtx, render_visual, render_visual_with};

/// Deepest group nesting followed when resolving hidden groups.
const MAX_GROUP_DEPTH: usize = 32;

/// Options for [`render_report_page`].
#[derive(Debug, Clone, Default)]
pub struct PageRenderOptions {
    /// Frame options (viewport, placeholders).
    pub frames: FrameOptions,
    /// Query limits (row caps, cancellation).
    pub limits: QueryLimits,
}

/// What happened to one visual while rendering a page.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualRenderInfo {
    pub name: String,
    pub visual_type: String,
    /// Hidden (not queried, not drawn).
    pub hidden: bool,
    /// Query wall time in microseconds.
    pub query_micros: u64,
    /// Render wall time in microseconds (0 when not drawn).
    pub render_micros: u64,
    /// Filters not applied (or applied approximately), as messages.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// The query error (`⚠ field: message`), if the visual failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Hover text: the error with its DAX, or a button's tooltip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    /// What a click on the visual's `button` hit region does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<ButtonAction>,
    /// The data was cut at the query limits.
    pub truncated: bool,
}

/// A rendered page plus the per-visual query results (for tooltips, the
/// Visualizations pane and debugging).
#[derive(Debug)]
pub struct RenderedPage {
    pub scene: Scene,
    /// Query results of the visible visuals, in page order
    /// ([`render_report_page`]). Empty for [`render_interactive_page`],
    /// whose caller already holds them (its [`PageData`]): copying every
    /// DataView on each scroll step would cost time proportional to the
    /// data.
    pub data: Vec<(String, Result<DataView, VisualQueryError>)>,
    /// One entry per visual of the page (hidden ones included), in page
    /// order.
    pub visuals: Vec<VisualRenderInfo>,
}

impl RenderedPage {
    /// Info of the visual named `name`.
    pub fn visual(&self, name: &str) -> Option<&VisualRenderInfo> {
        self.visuals.iter().find(|v| v.name == name)
    }
}

/// Names of hidden visuals (hidden themselves or inside a hidden group).
pub fn hidden_visuals(page: &Page) -> HashSet<&str> {
    let by_name: HashMap<&str, &Visual> =
        page.visuals.iter().map(|v| (v.name.as_str(), v)).collect();
    page.visuals
        .iter()
        .filter(|v| {
            let mut cur = *v;
            for _ in 0..MAX_GROUP_DEPTH {
                if cur.is_hidden {
                    return true;
                }
                match cur
                    .parent_group_name
                    .as_deref()
                    .filter(|p| *p != cur.name)
                    .and_then(|p| by_name.get(p))
                {
                    Some(parent) => cur = parent,
                    None => break,
                }
            }
            false
        })
        .map(|v| v.name.as_str())
        .collect()
}

fn micros(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX)
}

/// Queries and renders every visible visual of `page`.
pub fn render_report_page(
    env: &QueryEnv,
    report: &Report,
    page: &Page,
    theme: &ThemeResolver,
    state: &InteractionState,
    opts: &PageRenderOptions,
) -> RenderedPage {
    let data = query_page_data(env, report, page, state, &opts.limits);
    let mut rendered = render_page_data(report, page, theme, state, opts, None, &data, None).page;
    rendered.data = data.data;
    rendered
}

/// Interactive state of one visual; only the part matching its type is
/// used (slicer, table or matrix options).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VisualViewState {
    /// Slicers: open dropdown, search text, list scroll.
    pub slicer: SlicerOptions,
    /// Tables: body scroll.
    pub table: TableOptions,
    /// Matrices: body scroll and row expansion.
    pub matrix: MatrixOptions,
}

/// Interactive state of a page's visuals by visual name. Visuals not listed
/// use the defaults (unscrolled, dropdowns closed, persisted expansion).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PageViewState {
    pub visuals: IndexMap<String, VisualViewState>,
}

impl PageViewState {
    pub fn new() -> Self {
        Self::default()
    }

    /// The state of `visual`, if it has one.
    pub fn get(&self, visual: &str) -> Option<&VisualViewState> {
        self.visuals.get(visual)
    }

    /// The state of `visual`, created with the defaults.
    pub fn visual_mut(&mut self, visual: &str) -> &mut VisualViewState {
        self.visuals.entry(visual.to_owned()).or_default()
    }

    /// Closes every open slicer dropdown; `true` if one was open.
    pub fn close_dropdowns(&mut self) -> bool {
        let mut closed = false;
        for v in self.visuals.values_mut() {
            closed |= std::mem::take(&mut v.slicer.dropdown_open);
        }
        closed
    }

    /// Names of the slicers whose dropdown is open.
    pub fn open_dropdowns(&self) -> impl Iterator<Item = &str> {
        self.visuals
            .iter()
            .filter(|(_, v)| v.slicer.dropdown_open)
            .map(|(name, _)| name.as_str())
    }
}

/// Query results of a page's visible visuals ([`query_page_data`]).
#[derive(Debug, Clone, Default)]
pub struct PageData {
    /// Visual name → query result, in page order (hidden visuals skipped).
    pub data: Vec<(String, Result<DataView, VisualQueryError>)>,
    /// Query wall time per visual, in microseconds.
    pub query_micros: HashMap<String, u64>,
    /// Identity of these results for render caches: [`query_page_data`]
    /// sets a process-unique id ([`PageData::next_id`]), so interactive
    /// renders remember each result's content hash instead of re-hashing
    /// it (a scroll step does no work proportional to the data). `0` (the
    /// default) means unknown: the results are hashed on every render.
    /// Results with a non-zero id must not be modified in place: set `id`
    /// to 0 or a fresh [`PageData::next_id`] after changing `data`.
    pub id: u64,
}

impl PageData {
    /// A new process-unique, non-zero [`PageData::id`].
    pub fn next_id() -> u64 {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        NEXT.fetch_add(1, Ordering::Relaxed)
    }

    /// The query result of `visual`.
    pub fn get(&self, visual: &str) -> Option<&Result<DataView, VisualQueryError>> {
        self.data.iter().find(|(n, _)| n == visual).map(|(_, r)| r)
    }
}

/// Queries every visible visual of `page` (page order) on one snapshot.
pub fn query_page_data(
    env: &QueryEnv,
    report: &Report,
    page: &Page,
    state: &InteractionState,
    limits: &QueryLimits,
) -> PageData {
    let hidden = hidden_visuals(page);
    let mut out = PageData {
        data: Vec::with_capacity(page.visuals.len()),
        query_micros: HashMap::new(),
        id: PageData::next_id(),
    };
    for v in &page.visuals {
        if hidden.contains(v.name.as_str()) {
            continue;
        }
        let start = Instant::now();
        let result = env.query_visual(report, page, v, state, limits);
        out.query_micros.insert(v.name.clone(), micros(start));
        out.data.push((v.name.clone(), result));
    }
    out
}

/// Scroll position and range of a visual's scrolling area (page units).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrollInfo {
    /// Current offsets (clamped).
    pub x: f64,
    pub y: f64,
    pub max_x: f64,
    pub max_y: f64,
    /// The scrolling viewport: a table / matrix body, a slicer list or the
    /// rows of a dropdown popup.
    pub viewport: Rect,
}

/// Interactive geometry of a slicer, table or matrix after rendering.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualInteraction {
    pub name: String,
    pub visual_type: String,
    /// Scroll state (`None` when the visual has no data to scroll).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scroll: Option<ScrollInfo>,
    /// Slicer rectangles (dropdown box, popup, search box, rows).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slicer: Option<SlicerLayout>,
    /// Matrix body rows in display order (data point `index` → row).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub matrix_rows: Vec<MatrixRowRef>,
}

/// A page rendered for the interactive canvas ([`render_interactive_page`]).
#[derive(Debug)]
pub struct InteractivePage {
    pub page: RenderedPage,
    /// Open slicer dropdown popups: a scene of the same size and page
    /// transform as `page.scene`, drawn above it and not clipped to the
    /// page; `None` when no dropdown is open.
    pub overlay: Option<Scene>,
    /// Geometry of the page's visible slicers, tables and matrices, in
    /// paint order.
    pub interactions: Vec<VisualInteraction>,
    /// Visuals whose render came from the visual cache (not re-rendered),
    /// in paint order. Empty without a cache.
    pub reused: Vec<String>,
}

impl InteractivePage {
    /// Interactive geometry of the visual named `name`.
    pub fn interaction(&self, name: &str) -> Option<&VisualInteraction> {
        self.interactions.iter().find(|v| v.name == name)
    }
}

/// Renders `page` from query results ([`query_page_data`]) with a view
/// state: scroll offsets, matrix expansion and open slicer dropdowns (see
/// the module docs). `state` must be the one `data` was queried with (it
/// drives the selected-state styling). Unchanged visuals are reused from
/// the process-wide [`VisualCache::shared`].
pub fn render_interactive_page(
    report: &Report,
    page: &Page,
    theme: &ThemeResolver,
    state: &InteractionState,
    opts: &PageRenderOptions,
    view: &PageViewState,
    data: &PageData,
) -> InteractivePage {
    render_interactive_page_cached(
        report,
        page,
        theme,
        state,
        opts,
        view,
        data,
        Some(VisualCache::shared()),
    )
}

/// [`render_interactive_page`] with an explicit visual cache; `None`
/// renders every visual. The page is identical either way.
#[allow(clippy::too_many_arguments)]
pub fn render_interactive_page_cached(
    report: &Report,
    page: &Page,
    theme: &ThemeResolver,
    state: &InteractionState,
    opts: &PageRenderOptions,
    view: &PageViewState,
    data: &PageData,
    cache: Option<&VisualCache>,
) -> InteractivePage {
    render_page_data(report, page, theme, state, opts, Some(view), data, cache)
}

/// Renders one visual for the interactive canvas: content, geometry and
/// open popup. With `grids` (a cache and the visual's input fingerprint
/// from [`VisualKey::new`]), tables and matrices draw from a cached
/// prepared grid.
fn render_interactive_visual(
    ctx: &VisualCtx<'_>,
    vs: Option<&VisualViewState>,
    grids: Option<(&VisualCache, u64)>,
) -> CachedVisual {
    if let Some((cache, base)) = grids
        && let Some(out) = render_prepared_grid(ctx, vs, cache, base)
    {
        return out;
    }
    CachedVisual {
        interaction: interaction(ctx, vs),
        popup: vs
            .filter(|s| s.slicer.dropdown_open)
            .and_then(|s| dropdown_popup(ctx, &s.slicer)),
        content: render_visual_with(ctx, vs),
    }
}

/// A table or matrix drawn from its cached [`PreparedGrid`] (model, style
/// and layout are built once per data, definition, frame and expansion), so
/// a scroll step only lays out and draws the visible rows. Same output as
/// the uncached path; `None` for other visuals and without grid data.
fn render_prepared_grid(
    ctx: &VisualCtx<'_>,
    vs: Option<&VisualViewState>,
    cache: &VisualCache,
    base: u64,
) -> Option<CachedVisual> {
    if !matches!(ctx.data, Some(Ok(_))) {
        return None;
    }
    let default = VisualViewState::default();
    let v = vs.unwrap_or(&default);
    let matrix = match ctx.visual.visual_type.as_str() {
        "tableEx" | "table" => false,
        "pivotTable" | "matrix" => true,
        _ => return None,
    };
    let (variant, scroll) = if matrix {
        let expansion = Fingerprint::new()
            .add(&v.matrix.expand_depth)
            .add(&v.matrix.toggled)
            .finish()?;
        (expansion, v.matrix.scroll)
    } else {
        (0, v.table.scroll)
    };
    let key = GridKey {
        visual: ctx.visual.name.clone(),
        base,
        variant,
    };
    let grid = match cache.grid(&key) {
        Some(grid) => grid,
        None => {
            let prepared = if matrix {
                matrix::prepare(ctx, &v.matrix)?
            } else {
                table::prepare(ctx)?
            };
            cache.insert_grid(key, prepared)
        }
    };
    let (content, metrics) = grid.render(table::env(ctx), scroll);
    Some(CachedVisual {
        content: Some(content),
        interaction: Some(VisualInteraction {
            name: ctx.visual.name.clone(),
            visual_type: ctx.visual.visual_type.clone(),
            scroll: Some(grid_scroll(&metrics)),
            slicer: None,
            matrix_rows: grid.rows.clone(),
        }),
        popup: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn render_page_data(
    report: &Report,
    page: &Page,
    theme: &ThemeResolver,
    state: &InteractionState,
    opts: &PageRenderOptions,
    view: Option<&PageViewState>,
    data: &PageData,
    cache: Option<&VisualCache>,
) -> InteractivePage {
    let hidden = hidden_visuals(page);
    let fonts = FontBook::shared();
    let mut render_times: HashMap<String, u64> = HashMap::new();
    let mut popups: Vec<Group> = Vec::new();
    let mut interactions: Vec<VisualInteraction> = Vec::new();
    let mut reused: Vec<String> = Vec::new();
    let scene = render_page_with(report, page, theme, &opts.frames, |frame, visual| {
        let start = Instant::now();
        let ctx = VisualCtx {
            frame,
            visual,
            theme,
            fonts,
            data: data.get(&visual.name),
            state,
        };
        let group = match view {
            Some(view) => {
                let vs = view.get(&visual.name);
                let rendered = match cache {
                    Some(cache) => match data_fingerprint(data.id, &visual.name, ctx.data)
                        .and_then(|d| VisualKey::new(&ctx, vs, d))
                    {
                        Some((key, base)) => match cache.get(&key) {
                            Some(hit) => {
                                reused.push(visual.name.clone());
                                hit
                            }
                            None => cache.insert(
                                key,
                                render_interactive_visual(&ctx, vs, Some((cache, base))),
                            ),
                        },
                        None => {
                            cache.note_uncacheable();
                            Arc::new(render_interactive_visual(&ctx, vs, None))
                        }
                    },
                    None => Arc::new(render_interactive_visual(&ctx, vs, None)),
                };
                if let Some(info) = &rendered.interaction {
                    interactions.push(info.clone());
                }
                if let Some(popup) = &rendered.popup {
                    popups.push(popup.clone());
                }
                rendered.content.clone()
            }
            None => render_visual(&ctx),
        };
        render_times.insert(visual.name.clone(), micros(start));
        group
    });
    let overlay = (!popups.is_empty()).then(|| {
        let layout = PageLayout::compute_with(
            page,
            theme,
            &LayoutOptions::for_report(report, opts.frames.viewport),
        );
        let mut overlay = Scene::new(scene.width, scene.height);
        let mut root = Group::new()
            .with_transform(layout.page_transform())
            .with_class("pbi-overlay");
        for popup in popups {
            root.push(popup);
        }
        overlay.root.push(root);
        overlay
    });
    let visuals = page
        .visuals
        .iter()
        .map(|v| {
            let result = data.get(&v.name);
            let (warnings, error, mut tooltip, truncated) = match result {
                Some(Ok(dv)) => (
                    dv.warnings.iter().map(|w| w.message.clone()).collect(),
                    None,
                    None,
                    dv.truncated,
                ),
                Some(Err(e)) => (
                    e.warnings.iter().map(|w| w.message.clone()).collect(),
                    Some(e.display()),
                    Some(error_tooltip(e)),
                    false,
                ),
                None => (Vec::new(), None, None, false),
            };
            let is_button = matches!(
                v.visual_type.as_str(),
                "actionButton" | "pageNavigator" | "bookmarkNavigator"
            );
            let action = is_button.then(|| button::action(v, theme)).flatten();
            if tooltip.is_none() && is_button {
                tooltip = button::tooltip(v, theme);
            }
            VisualRenderInfo {
                name: v.name.clone(),
                visual_type: v.visual_type.clone(),
                hidden: hidden.contains(v.name.as_str()),
                query_micros: data.query_micros.get(v.name.as_str()).copied().unwrap_or(0),
                render_micros: render_times.get(&v.name).copied().unwrap_or(0),
                warnings,
                error,
                tooltip,
                action,
                truncated,
            }
        })
        .collect();
    InteractivePage {
        page: RenderedPage {
            scene,
            data: Vec::new(),
            visuals,
        },
        overlay,
        interactions,
        reused,
    }
}

/// The open dropdown popup of a slicer in dropdown mode (`None` otherwise
/// or without data).
fn dropdown_popup(ctx: &VisualCtx<'_>, opts: &SlicerOptions) -> Option<Group> {
    if !matches!(ctx.data, Some(Ok(_)))
        || !matches!(
            ctx.visual.visual_type.as_str(),
            "slicer" | "advancedSlicerVisual"
        )
    {
        return None;
    }
    let popup = slicer::render_dropdown_popup(ctx, opts);
    (!popup.is_empty()).then_some(popup)
}

fn clamp_scroll(v: f64, max: f64) -> f64 {
    if v.is_finite() {
        v.clamp(0.0, max.max(0.0))
    } else {
        0.0
    }
}

/// Interactive geometry of a slicer, table or matrix (`None` for other
/// visual types and failed queries).
fn interaction(ctx: &VisualCtx<'_>, view: Option<&VisualViewState>) -> Option<VisualInteraction> {
    if !matches!(ctx.data, Some(Ok(_))) {
        return None;
    }
    let default = VisualViewState::default();
    let vs = view.unwrap_or(&default);
    let mut info = VisualInteraction {
        name: ctx.visual.name.clone(),
        visual_type: ctx.visual.visual_type.clone(),
        scroll: None,
        slicer: None,
        matrix_rows: Vec::new(),
    };
    match ctx.visual.visual_type.as_str() {
        "slicer" | "advancedSlicerVisual" => {
            let layout = slicer::layout(ctx, &vs.slicer);
            info.scroll = layout
                .rows
                .filter(|_| matches!(layout.mode, SlicerMode::List | SlicerMode::Dropdown))
                .map(|rows| ScrollInfo {
                    x: 0.0,
                    y: clamp_scroll(vs.slicer.scroll, layout.max_scroll),
                    max_x: 0.0,
                    max_y: layout.max_scroll.max(0.0),
                    viewport: rows,
                });
            info.slicer = Some(layout);
        }
        "tableEx" | "table" => {
            let m = table::metrics(ctx, &vs.table)?;
            info.scroll = Some(grid_scroll(&m));
        }
        "pivotTable" | "matrix" => {
            let m = matrix::metrics(ctx, &vs.matrix)?;
            info.scroll = Some(grid_scroll(&m));
            info.matrix_rows = matrix::visible_rows(ctx, &vs.matrix);
        }
        _ => return None,
    }
    Some(info)
}

fn grid_scroll(m: &table::GridMetrics) -> ScrollInfo {
    ScrollInfo {
        x: m.scroll.x,
        y: m.scroll.y,
        max_x: m.max_scroll_x.max(0.0),
        max_y: m.max_scroll_y.max(0.0),
        viewport: m.body,
    }
}
