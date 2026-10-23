//! Built-in visuals: DataView + formatting objects → scene graph.
//!
//! Every visual renders into the content box of its [`VisualFrame`] (page
//! units); container chrome (background, title, border, shadow, hit area) is
//! drawn by [`crate::layout::frame_group`]. [`render_visual`] dispatches on
//! `visualType`; unknown types fall back to [`placeholder::render`] (type
//! label + bound fields). A failed query renders as `⚠` + message (with the
//! DAX text in a tooltip), never as a panic.
//!
//! Ownership (parallel development):
//! - `common`, `textbox`, `shape`, `button`, `card`, `placeholder`, `page`
//!   — basics + page orchestration
//! - `slicer`, `table`, `matrix` — slicers and grids
//! - `charts/**` — pie/donut, bar/column, line/area, scatter, funnel,
//!   gauge, treemap, waterfall, axes, legends, labels

pub mod button;
pub mod cache;
pub mod card;
pub mod charts;
pub mod common;
pub mod matrix;
pub mod page;
pub mod placeholder;
pub mod shape;
pub mod slicer;
pub mod table;
pub mod textbox;

use pbi_model::Visual;

use crate::dataview::DataView;
use crate::layout::VisualFrame;
use crate::query::{InteractionState, VisualQueryError};
use crate::scene::Group;
use crate::text::FontBook;
use crate::theme::ThemeResolver;

pub use cache::{CacheLimits, CacheStats, VisualCache};
pub use page::{
    InteractivePage, PageData, PageRenderOptions, PageViewState, RenderedPage, ScrollInfo,
    VisualInteraction, VisualViewState, query_page_data, render_interactive_page,
    render_interactive_page_cached, render_report_page,
};

/// Everything a visual renderer needs. Interactive renders are cached by a
/// fingerprint of these inputs ([`cache`]): a renderer must not read
/// anything else, and from `state` only its own slicer selection and the
/// point selection whose source it is.
#[derive(Debug, Clone, Copy)]
pub struct VisualCtx<'a> {
    /// Layout of the visual (draw inside `frame.content`).
    pub frame: &'a VisualFrame,
    /// The visual definition (type, projections, `objects`, …).
    pub visual: &'a Visual,
    /// Theme + formatting-object resolution (`theme.visual(visual)`).
    pub theme: &'a ThemeResolver,
    /// Bundled fonts for measurement.
    pub fonts: &'a FontBook,
    /// Query result; `None` for visuals without a query (textbox, shape, …).
    pub data: Option<&'a Result<DataView, VisualQueryError>>,
    /// Current slicer/point selections (for selected-state styling).
    pub state: &'a InteractionState,
}

/// Renders one visual's content group, dispatching on `visualType`.
pub fn render_visual(ctx: &VisualCtx<'_>) -> Option<Group> {
    if let Some(Err(err)) = ctx.data {
        return Some(common::error_content(ctx, err));
    }
    match ctx.visual.visual_type.as_str() {
        "textbox" => textbox::render(ctx),
        "shape" | "basicShape" => shape::render(ctx),
        "actionButton" | "pageNavigator" | "bookmarkNavigator" => button::render(ctx),
        "cardVisual" | "card" | "multiRowCard" | "kpi" => card::render(ctx),
        "slicer" | "advancedSlicerVisual" => slicer::render(ctx),
        "tableEx" | "table" => table::render(ctx),
        "pivotTable" | "matrix" => matrix::render(ctx),
        t if charts::handles(t) => charts::render(ctx),
        _ => placeholder::render(ctx),
    }
}

/// [`render_visual`] with an interactive view state: slicers, tables and
/// matrices render with their options (open dropdown, search, scroll,
/// expansion); other visuals ignore it. `None` is [`render_visual`].
pub fn render_visual_with(ctx: &VisualCtx<'_>, view: Option<&VisualViewState>) -> Option<Group> {
    let Some(view) = view else {
        return render_visual(ctx);
    };
    if let Some(Err(err)) = ctx.data {
        return Some(common::error_content(ctx, err));
    }
    match ctx.visual.visual_type.as_str() {
        "slicer" | "advancedSlicerVisual" => slicer::render_with(ctx, &view.slicer),
        "tableEx" | "table" => table::render_with(ctx, &view.table),
        "pivotTable" | "matrix" => matrix::render_with(ctx, &view.matrix),
        _ => render_visual(ctx),
    }
}
