//! `canvas.*`: the interactive canvas on the registry (per-page interaction
//! and view state kept in the workspace, ADR-0006).

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{BASE, loaded_doc};
use crate::canvas::{
    self, CanvasAction, CanvasPageOut, Changed, MAX_SEARCH_CHARS, Modifiers, Target, TooltipLineOut,
};
use crate::command::{Category, Command, CommandMeta};
use crate::ctx::Ctx;
use crate::error::OpError;
use crate::workspace::{DocEntry, Workspace};

async fn target(
    ws: &Workspace,
    doc_id: &str,
    report: Option<&str>,
    page: Option<&str>,
    ctx: &Ctx,
) -> Result<Target, OpError> {
    let entry: Arc<DocEntry> = loaded_doc(ws, doc_id, ctx).await?;
    Target::resolve(entry, report, page)
}

/// The page of a canvas command.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasPageParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Report (default: the first).
    #[serde(default)]
    pub report: Option<String>,
    /// Page (default: the active page).
    #[serde(default)]
    pub page: Option<String>,
}

/// A viewport in CSS pixels.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewportParam {
    pub width: f64,
    pub height: f64,
}

/// Result of a state change.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CanvasChangeOut {
    /// The interaction or view state changed.
    pub changed: bool,
    /// The re-rendered page (also when unchanged, unless
    /// `renderUnchanged` is `false`).
    pub page: Option<CanvasPageOut>,
    /// What a clicked button asks the caller to do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<CanvasAction>,
}

impl From<Changed> for CanvasChangeOut {
    fn from(c: Changed) -> Self {
        Self {
            changed: c.changed,
            page: c.page,
            action: c.action,
        }
    }
}

fn yes() -> bool {
    true
}

const CANVAS: CommandMeta = CommandMeta {
    category: Category::Canvas,
    mutates_document: true,
    ..BASE
};

// ---------------------------------------------------------------------------

/// `canvas.render`.
#[derive(Debug)]
pub struct CanvasRender;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasRenderParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report.
    #[serde(default)]
    pub report: Option<String>,
    /// Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page.
    #[serde(default)]
    pub page: Option<String>,
    /// Fit the page into this viewport (kept for later renders).
    #[serde(default)]
    pub viewport: Option<ViewportParam>,
}

impl Command for CanvasRender {
    type Params = CanvasRenderParams;
    type Output = CanvasPageOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.render",
            title: "Render the canvas",
            summary: "Renders a page with its current interaction state: SVG, hit regions, visuals, slicer/cross-filter selections and effective filters.",
            long_running: true,
            examples: &[
                ("Active page", r#"{"docId": "d1"}"#),
                (
                    "Fit a viewport",
                    r#"{"docId": "d1", "report": "Stores", "viewport": {"width": 1280, "height": 720}}"#,
                ),
            ],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let t = target(ws, &p.doc_id, p.report.as_deref(), p.page.as_deref(), ctx).await?;
        canvas::render_page(&t, p.viewport.map(|v| (v.width, v.height))).await
    }
}

// ---------------------------------------------------------------------------

/// `canvas.click`.
#[derive(Debug)]
pub struct CanvasClick;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasClickParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report.
    #[serde(default)]
    pub report: Option<String>,
    /// Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page.
    #[serde(default)]
    pub page: Option<String>,
    /// A hit id from the last render (`hitRegions[].id`); `null` clicks the
    /// empty canvas (clears the cross-filter, closes dropdowns).
    #[serde(default)]
    pub hit_id: Option<String>,
    /// Ctrl (⌘) held: add to / remove from the selection.
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub shift: bool,
    /// Return the page even when nothing changed (default `true`).
    #[serde(default = "yes")]
    pub render_unchanged: bool,
}

impl Command for CanvasClick {
    type Params = CanvasClickParams;
    type Output = CanvasChangeOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.click",
            title: "Click on the canvas",
            summary: "Applies a click on a hit region (slicer item, data point, button, dropdown) and returns the re-rendered page and any button action.",
            long_running: true,
            examples: &[
                (
                    "Select a slicer item",
                    r#"{"docId": "d1", "hitId": "9a5c3f0e1b|slicerItem|1"}"#,
                ),
                (
                    "Ctrl+click a bar",
                    r#"{"docId": "d1", "page": "1", "hitId": "1f2e3d|dataPoint|0|2", "ctrl": true}"#,
                ),
            ],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let t = target(ws, &p.doc_id, p.report.as_deref(), p.page.as_deref(), ctx).await?;
        let mods = Modifiers {
            ctrl: p.ctrl,
            shift: p.shift,
        };
        Ok(
            canvas::click(&t, p.hit_id.as_deref(), mods, p.render_unchanged)
                .await?
                .into(),
        )
    }
}

// ---------------------------------------------------------------------------

/// `canvas.scroll`.
#[derive(Debug)]
pub struct CanvasScroll;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasScrollParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report.
    #[serde(default)]
    pub report: Option<String>,
    /// Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page.
    #[serde(default)]
    pub page: Option<String>,
    /// The table, matrix or slicer to scroll.
    pub visual: String,
    /// Horizontal delta in page units.
    #[serde(default)]
    pub dx: f64,
    /// Vertical delta in page units (positive scrolls down).
    #[serde(default)]
    pub dy: f64,
    /// Return the page even when nothing changed (default `true`).
    #[serde(default = "yes")]
    pub render_unchanged: bool,
}

impl Command for CanvasScroll {
    type Params = CanvasScrollParams;
    type Output = CanvasChangeOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.scroll",
            title: "Scroll a visual",
            summary: "Scrolls a table, matrix or slicer list by a delta (clamped to its range) and returns the re-rendered page.",
            examples: &[(
                "Scroll a table down",
                r#"{"docId": "d1", "page": "1", "visual": "a1b2c3", "dy": 200}"#,
            )],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let t = target(ws, &p.doc_id, p.report.as_deref(), p.page.as_deref(), ctx).await?;
        Ok(
            canvas::scroll(&t, &p.visual, p.dx, p.dy, p.render_unchanged)
                .await?
                .into(),
        )
    }
}

// ---------------------------------------------------------------------------

/// `canvas.setSlicer`.
#[derive(Debug)]
pub struct CanvasSetSlicer;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasSetSlicerParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report.
    #[serde(default)]
    pub report: Option<String>,
    /// Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page.
    #[serde(default)]
    pub page: Option<String>,
    /// The slicer's visual name…
    #[serde(default)]
    pub visual: Option<String>,
    /// …or its field (display name, column, `Table[Column]`).
    #[serde(default)]
    pub field: Option<String>,
    /// Values to select (`null` = BLANK); empty without `invert` selects
    /// all.
    #[serde(default)]
    pub values: Vec<Value>,
    /// Select everything except `values`.
    #[serde(default)]
    pub invert: bool,
}

impl Command for CanvasSetSlicer {
    type Params = CanvasSetSlicerParams;
    type Output = CanvasPageOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.setSlicer",
            title: "Set a slicer",
            summary: "Replaces a slicer's selection by value (no hit ids needed) and returns the re-rendered page.",
            long_running: true,
            examples: &[
                (
                    "Only the West region",
                    r#"{"docId": "d1", "field": "Region", "values": ["West"]}"#,
                ),
                (
                    "Clear a slicer",
                    r#"{"docId": "d1", "field": "Region", "values": []}"#,
                ),
            ],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let t = target(ws, &p.doc_id, p.report.as_deref(), p.page.as_deref(), ctx).await?;
        canvas::set_slicer(
            &t,
            p.visual.as_deref(),
            p.field.as_deref(),
            p.values,
            p.invert,
        )
        .await
    }
}

// ---------------------------------------------------------------------------

/// `canvas.clearSelections`.
#[derive(Debug)]
pub struct CanvasClearSelections;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasClearParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report.
    #[serde(default)]
    pub report: Option<String>,
    /// Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page.
    #[serde(default)]
    pub page: Option<String>,
    /// A slicer (clears its selection) or the cross-filter source; absent:
    /// clears the cross-filter and closes the dropdowns.
    #[serde(default)]
    pub visual: Option<String>,
}

impl Command for CanvasClearSelections {
    type Params = CanvasClearParams;
    type Output = CanvasPageOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.clearSelections",
            title: "Clear selections",
            summary: "Clears a slicer's selection (\"Clear selections\") or the page's cross-filter, and returns the re-rendered page.",
            long_running: true,
            examples: &[
                ("Clear the cross-filter", r#"{"docId": "d1"}"#),
                (
                    "Clear one slicer",
                    r#"{"docId": "d1", "visual": "9a5c3f0e1b"}"#,
                ),
            ],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let t = target(ws, &p.doc_id, p.report.as_deref(), p.page.as_deref(), ctx).await?;
        canvas::clear_selections(&t, p.visual.as_deref()).await
    }
}

// ---------------------------------------------------------------------------

/// `canvas.setSlicerSearch`.
#[derive(Debug)]
pub struct CanvasSetSlicerSearch;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasSearchParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report.
    #[serde(default)]
    pub report: Option<String>,
    /// Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page.
    #[serde(default)]
    pub page: Option<String>,
    /// The slicer.
    pub visual: String,
    /// Search text (at most 256 characters; empty clears it).
    pub text: String,
}

impl Command for CanvasSetSlicerSearch {
    type Params = CanvasSearchParams;
    type Output = CanvasPageOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.setSlicerSearch",
            title: "Search a slicer",
            summary: "Filters a slicer's list (or open dropdown) by a search text and returns the re-rendered page.",
            long_running: true,
            examples: &[(
                "Search",
                r#"{"docId": "d1", "visual": "9a5c3f0e1b", "text": "crit"}"#,
            )],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        if p.text.chars().count() > MAX_SEARCH_CHARS {
            return Err(OpError::invalid_params(format!(
                "the search text is longer than {MAX_SEARCH_CHARS} characters"
            )));
        }
        let t = target(ws, &p.doc_id, p.report.as_deref(), p.page.as_deref(), ctx).await?;
        canvas::set_slicer_search(&t, &p.visual, &p.text).await
    }
}

// ---------------------------------------------------------------------------

/// `canvas.toggleMatrixRow`.
#[derive(Debug)]
pub struct CanvasToggleMatrixRow;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasHitParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report.
    #[serde(default)]
    pub report: Option<String>,
    /// Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page.
    #[serde(default)]
    pub page: Option<String>,
    /// A hit id from the last render.
    pub hit_id: String,
}

impl Command for CanvasToggleMatrixRow {
    type Params = CanvasHitParams;
    type Output = CanvasPageOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.toggleMatrixRow",
            title: "Expand/collapse a matrix row",
            summary: "Expands or collapses the matrix row of a cell hit and returns the re-rendered page.",
            long_running: true,
            examples: &[(
                "Toggle a row",
                r#"{"docId": "d1", "hitId": "4e5f6a|dataPoint|0|3"}"#,
            )],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let t = target(ws, &p.doc_id, p.report.as_deref(), p.page.as_deref(), ctx).await?;
        canvas::toggle_matrix_row(&t, &p.hit_id).await
    }
}

// ---------------------------------------------------------------------------

/// `canvas.closePopups`.
#[derive(Debug)]
pub struct CanvasClosePopups;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasClosePopupsParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report.
    #[serde(default)]
    pub report: Option<String>,
    /// Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page.
    #[serde(default)]
    pub page: Option<String>,
    /// Return the page even when nothing changed (default `true`).
    #[serde(default = "yes")]
    pub render_unchanged: bool,
}

impl Command for CanvasClosePopups {
    type Params = CanvasClosePopupsParams;
    type Output = CanvasChangeOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.closePopups",
            title: "Close dropdowns",
            summary: "Closes the open slicer dropdowns of a page.",
            examples: &[("Close", r#"{"docId": "d1", "renderUnchanged": false}"#)],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let t = target(ws, &p.doc_id, p.report.as_deref(), p.page.as_deref(), ctx).await?;
        Ok(canvas::close_popups(&t, p.render_unchanged).await?.into())
    }
}

// ---------------------------------------------------------------------------

/// `canvas.reset`.
#[derive(Debug)]
pub struct CanvasReset;

impl Command for CanvasReset {
    type Params = CanvasPageParams;
    type Output = CanvasPageOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.reset",
            title: "Reset to default",
            summary: "Back to the report's saved slicer selections, no cross-filter, scroll and dropdowns reset; returns the re-rendered page.",
            long_running: true,
            examples: &[("Reset the active page", r#"{"docId": "d1"}"#)],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let t = target(ws, &p.doc_id, p.report.as_deref(), p.page.as_deref(), ctx).await?;
        canvas::reset_page(&t).await
    }
}

// ---------------------------------------------------------------------------

/// `canvas.tooltip`.
#[derive(Debug)]
pub struct CanvasTooltip;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TooltipOut {
    /// Label/value lines; empty when the hit has no tooltip.
    pub lines: Vec<TooltipLineOut>,
}

impl Command for CanvasTooltip {
    type Params = CanvasHitParams;
    type Output = TooltipOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.tooltip",
            title: "Data point tooltip",
            summary: "The hover tooltip (category, series, values) of a data point hit, from the last render.",
            mutates_document: false,
            examples: &[(
                "A bar's tooltip",
                r#"{"docId": "d1", "hitId": "1f2e3d|dataPoint|0|2"}"#,
            )],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let t = target(ws, &p.doc_id, p.report.as_deref(), p.page.as_deref(), ctx).await?;
        Ok(TooltipOut {
            lines: canvas::tooltip(&t, &p.hit_id)?,
        })
    }
}

// ---------------------------------------------------------------------------

/// `canvas.state`.
#[derive(Debug)]
pub struct CanvasState;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CanvasStateOut {
    /// Slicer selections and selected data points (`InteractionState`).
    pub interaction: Value,
    /// Scroll, expansion and dropdown state per visual (`PageViewState`).
    pub view: Value,
    /// The interaction state equals the report's saved state.
    pub is_default: bool,
}

impl Command for CanvasState {
    type Params = CanvasPageParams;
    type Output = CanvasStateOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.state",
            title: "Canvas state",
            summary: "The page's current interaction state (slicer selections, cross-filter) and view state, as JSON.",
            mutates_document: false,
            examples: &[("State of the active page", r#"{"docId": "d1"}"#)],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = super::doc(ws, &p.doc_id)?;
        let t = Target::resolve(entry, p.report.as_deref(), p.page.as_deref())?;
        let (interaction, view, is_default) = canvas::state(&t)?;
        Ok(CanvasStateOut {
            interaction,
            view,
            is_default,
        })
    }
}
