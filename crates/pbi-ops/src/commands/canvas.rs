//! `canvas.*`: the interactive canvas on the registry (per-page interaction
//! and view state kept in the workspace, ADR-0006).

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pbi_render::query::DrillAction;
use pbi_render::query::relative::RelativeDateSpec;

use super::{BASE, loaded_doc};
use crate::canvas::{
    self, CanvasAction, CanvasPageOut, Changed, ContextMenuOut, DrillthroughTargetOut,
    MAX_SEARCH_CHARS, Modifiers, SortOrder, Target, TooltipLineOut, TooltipPageOut,
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

/// `canvas.setRange`.
#[derive(Debug)]
pub struct CanvasSetRange;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasSetRangeParams {
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
    /// Lower bound (inclusive): a number, or for dates `YYYY-MM-DD` or an
    /// OADate day number. Absent: open.
    #[serde(default)]
    pub low: Option<canvas::RangeBound>,
    /// Upper bound (inclusive; a date includes the whole day). Absent:
    /// open. Both absent clear the slicer.
    #[serde(default)]
    pub high: Option<canvas::RangeBound>,
}

impl Command for CanvasSetRange {
    type Params = CanvasSetRangeParams;
    type Output = CanvasPageOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.setRange",
            title: "Set a range slicer",
            summary: "Filters a numeric or date range slicer to a lower and/or upper bound (a date includes the whole day) and returns the re-rendered page.",
            long_running: true,
            examples: &[
                (
                    "Orders in the first quarter of 2025",
                    r#"{"docId": "d1", "field": "Date", "low": "2025-01-01", "high": "2025-03-31"}"#,
                ),
                (
                    "Quantity of at least 5",
                    r#"{"docId": "d1", "field": "Quantity", "low": 5}"#,
                ),
            ],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let t = target(ws, &p.doc_id, p.report.as_deref(), p.page.as_deref(), ctx).await?;
        canvas::set_range(&t, p.visual.as_deref(), p.field.as_deref(), p.low, p.high).await
    }
}

// ---------------------------------------------------------------------------

/// `canvas.setRelativeDate`.
#[derive(Debug)]
pub struct CanvasSetRelativeDate;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasSetRelativeDateParams {
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
    /// `last`, `next` or `this`; absent clears the slicer.
    #[serde(default)]
    pub anchor: Option<canvas::RelativeAnchorArg>,
    /// Number of periods (not for `this`). Default 1.
    #[serde(default = "one")]
    pub duration: u32,
    /// `minutes`, `hours`, `days`, `weeks`, `calendarWeeks`, `months`,
    /// `calendarMonths`, `years` or `calendarYears`. Default `days`.
    #[serde(default)]
    pub period: canvas::RelativePeriodArg,
    /// Count today as one of the days (non-calendar date periods). Default
    /// `true`.
    #[serde(default = "yes")]
    pub include_today: bool,
}

fn one() -> u32 {
    1
}

impl Command for CanvasSetRelativeDate {
    type Params = CanvasSetRelativeDateParams;
    type Output = CanvasPageOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.setRelativeDate",
            title: "Set a relative date slicer",
            summary: "Filters a date slicer to a relative period (\"Last 3 months\", \"This year\", \"Next 2 hours\") of the session clock and returns the re-rendered page.",
            long_running: true,
            examples: &[
                (
                    "The last 3 calendar months",
                    r#"{"docId": "d1", "field": "Date", "anchor": "last", "duration": 3, "period": "calendarMonths"}"#,
                ),
                (
                    "Clear a relative slicer",
                    r#"{"docId": "d1", "field": "Date"}"#,
                ),
            ],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let t = target(ws, &p.doc_id, p.report.as_deref(), p.page.as_deref(), ctx).await?;
        let spec = p.anchor.map(|anchor| RelativeDateSpec {
            anchor: anchor.into(),
            duration: p.duration,
            period: p.period.into(),
            include_today: p.include_today,
        });
        canvas::set_relative_date(&t, p.visual.as_deref(), p.field.as_deref(), spec).await
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

/// `canvas.applySlicers`.
#[derive(Debug)]
pub struct CanvasApplySlicers;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasApplySlicersParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report.
    #[serde(default)]
    pub report: Option<String>,
    /// Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page.
    #[serde(default)]
    pub page: Option<String>,
    /// The slicer whose pending selection to apply (its Apply button);
    /// absent: every slicer of the page ("Apply all slicers").
    #[serde(default)]
    pub visual: Option<String>,
}

impl Command for CanvasApplySlicers {
    type Params = CanvasApplySlicersParams;
    type Output = CanvasPageOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.applySlicers",
            title: "Apply slicers",
            summary: "Applies pending slicer selections (made while the page has an \"Apply all slicers\" button or the report adds an Apply button to slicers), and returns the re-rendered page.",
            long_running: true,
            examples: &[
                ("Apply every pending slicer", r#"{"docId": "d1"}"#),
                (
                    "Apply one slicer",
                    r#"{"docId": "d1", "visual": "9a5c3f0e1b"}"#,
                ),
            ],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let t = target(ws, &p.doc_id, p.report.as_deref(), p.page.as_deref(), ctx).await?;
        canvas::apply_slicers(&t, p.visual.as_deref()).await
    }
}

// ---------------------------------------------------------------------------

/// `canvas.focus`.
#[derive(Debug)]
pub struct CanvasFocus;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasFocusParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report.
    #[serde(default)]
    pub report: Option<String>,
    /// Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page.
    #[serde(default)]
    pub page: Option<String>,
    /// The visual to show in focus mode; absent: back to the report.
    #[serde(default)]
    pub visual: Option<String>,
    /// Also show the visual's data as a table below it ("Show as a
    /// table").
    #[serde(default)]
    pub show_as_table: bool,
}

impl Command for CanvasFocus {
    type Params = CanvasFocusParams;
    type Output = CanvasPageOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.focus",
            title: "Focus mode",
            summary: "Shows one visual filling the page (focus mode), optionally with its data as a table below it; without a visual goes back to the report.",
            long_running: true,
            examples: &[
                (
                    "Focus on a visual",
                    r#"{"docId": "d1", "visual": "9a5c3f0e1b"}"#,
                ),
                ("Back to the report", r#"{"docId": "d1"}"#),
            ],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let t = target(ws, &p.doc_id, p.report.as_deref(), p.page.as_deref(), ctx).await?;
        canvas::focus(&t, p.visual.as_deref(), p.show_as_table).await
    }
}

// ---------------------------------------------------------------------------

/// `canvas.showAsTable`.
#[derive(Debug)]
pub struct CanvasShowAsTable;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasVisualParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report.
    #[serde(default)]
    pub report: Option<String>,
    /// Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page.
    #[serde(default)]
    pub page: Option<String>,
    /// The visual's name; absent: ends the mode.
    #[serde(default)]
    pub visual: Option<String>,
}

impl Command for CanvasShowAsTable {
    type Params = CanvasVisualParams;
    type Output = CanvasPageOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.showAsTable",
            title: "Show as a table",
            summary: "Shows a visual with its data as a table below it, filling the page (focus mode); without a visual goes back to the report.",
            long_running: true,
            examples: &[(
                "A chart's data as a table",
                r#"{"docId": "d1", "visual": "9a5c3f0e1b"}"#,
            )],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let t = target(ws, &p.doc_id, p.report.as_deref(), p.page.as_deref(), ctx).await?;
        canvas::focus(&t, p.visual.as_deref(), true).await
    }
}

// ---------------------------------------------------------------------------

/// `canvas.setSpotlight`.
#[derive(Debug)]
pub struct CanvasSetSpotlight;

impl Command for CanvasSetSpotlight {
    type Params = CanvasVisualParams;
    type Output = CanvasPageOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.setSpotlight",
            title: "Spotlight",
            summary: "Spotlights a visual (the other visuals fade); without a visual ends the spotlight.",
            long_running: true,
            examples: &[
                (
                    "Spotlight a visual",
                    r#"{"docId": "d1", "visual": "9a5c3f0e1b"}"#,
                ),
                ("End the spotlight", r#"{"docId": "d1"}"#),
            ],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let t = target(ws, &p.doc_id, p.report.as_deref(), p.page.as_deref(), ctx).await?;
        canvas::set_spotlight(&t, p.visual.as_deref()).await
    }
}

// ---------------------------------------------------------------------------

/// `canvas.sortVisual`.
#[derive(Debug)]
pub struct CanvasSortVisual;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasSortVisualParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report.
    #[serde(default)]
    pub report: Option<String>,
    /// Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page.
    #[serde(default)]
    pub page: Option<String>,
    /// The visual's name.
    pub visual: String,
    /// Field to sort by: `queryRef`, display name or `Table[Column]`;
    /// absent: `direction` applies to the current sort, or (without
    /// `direction`) the saved sort returns.
    #[serde(default)]
    pub field: Option<String>,
    /// Default: descending for values, ascending for categories.
    #[serde(default)]
    pub direction: Option<SortOrder>,
}

impl Command for CanvasSortVisual {
    type Params = CanvasSortVisualParams;
    type Output = CanvasPageOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.sortVisual",
            title: "Sort a visual",
            summary: "Sorts a visual by one of its fields (the header's \"Sort axis\"), changes the sort direction or restores the saved sort.",
            long_running: true,
            examples: &[
                (
                    "Sort by a measure",
                    r#"{"docId": "d1", "visual": "9a5c3f0e1b", "field": "Sales.Total Sales"}"#,
                ),
                (
                    "Ascending",
                    r#"{"docId": "d1", "visual": "9a5c3f0e1b", "direction": "ascending"}"#,
                ),
            ],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let t = target(ws, &p.doc_id, p.report.as_deref(), p.page.as_deref(), ctx).await?;
        canvas::sort_visual(&t, &p.visual, p.field.as_deref(), p.direction).await
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

/// `canvas.drill`.
#[derive(Debug)]
pub struct CanvasDrill;

/// What `canvas.drill` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum DrillActionParam {
    /// Drill down on the data point `hitId`: its values filter the visual
    /// and the next level is shown.
    Down,
    /// Back up one level (or collapse an expanded level).
    Up,
    /// Go to the next level (all values of the levels above).
    Next,
    /// Expand all down one level.
    Expand,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasDrillParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Report (default: the first).
    #[serde(default)]
    pub report: Option<String>,
    /// Page (default: the active page).
    #[serde(default)]
    pub page: Option<String>,
    /// Visual: internal name or title (default: the visual of `hitId`).
    #[serde(default)]
    pub visual: Option<String>,
    pub action: DrillActionParam,
    /// A data point hit id from the last render (required for `down`).
    #[serde(default)]
    pub hit_id: Option<String>,
}

impl Command for CanvasDrill {
    type Params = CanvasDrillParams;
    type Output = CanvasPageOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.drill",
            title: "Drill down or up",
            summary: "Drills a visual with several axis (or matrix row) fields: down on a data point, up, to the next level or expanded one level; returns the re-rendered page.",
            long_running: true,
            examples: &[
                (
                    "Drill down on a column",
                    r#"{"docId": "d1", "action": "down", "hitId": "1f2e3d|dataPoint|0|2"}"#,
                ),
                (
                    "Drill up",
                    r#"{"docId": "d1", "visual": "1f2e3d", "action": "up"}"#,
                ),
            ],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let t = target(ws, &p.doc_id, p.report.as_deref(), p.page.as_deref(), ctx).await?;
        let action = match p.action {
            DrillActionParam::Down => DrillAction::Down,
            DrillActionParam::Up => DrillAction::Up,
            DrillActionParam::Next => DrillAction::Next,
            DrillActionParam::Expand => DrillAction::Expand,
        };
        canvas::drill(&t, p.visual.as_deref(), action, p.hit_id.as_deref()).await
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

/// `canvas.applyBookmark`.
#[derive(Debug)]
pub struct CanvasApplyBookmark;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasBookmarkParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report.
    #[serde(default)]
    pub report: Option<String>,
    /// Bookmark: internal name, else display name (case-insensitive); see
    /// `report.bookmarks`.
    pub bookmark: String,
    /// The page shown now, rendered when the bookmark does not change the
    /// page (internal name, display name, 1-based index or a unique part).
    /// Default: the active page.
    #[serde(default)]
    pub page: Option<String>,
}

/// Result of `canvas.applyBookmark`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CanvasBookmarkOut {
    /// Internal name of the bookmark.
    pub bookmark: String,
    pub display_name: String,
    /// The bookmark shows another page than `page` (`rendered.page`).
    pub navigated: bool,
    /// Pages whose slicer or cross-filter state the bookmark changed
    /// (internal names).
    pub changed_pages: Vec<String>,
    /// Parts of the bookmark DashCraft does not apply yet (Top N filter
    /// changes, focus mode, matrix expansion, cross-highlights,
    /// formatting, …).
    pub not_applied: Vec<String>,
    /// The page the bookmark shows (else `page`), rendered.
    pub rendered: CanvasPageOut,
}

impl Command for CanvasApplyBookmark {
    type Params = CanvasBookmarkParams;
    type Output = CanvasBookmarkOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.applyBookmark",
            title: "Apply a bookmark",
            summary: "Applies a report bookmark to the canvas (page, slicers, filters, visibility, sort, drill and spotlight; all or only its target visuals) and renders the page it shows; lists the parts not applied yet.",
            long_running: true,
            examples: &[
                (
                    "By display name",
                    r#"{"docId": "d1", "report": "Stores", "bookmark": "West region"}"#,
                ),
                (
                    "Keep the current page",
                    r#"{"docId": "d1", "report": "Stores", "bookmark": "831d0f1adb1c9f2ed1d0", "page": "Store Performance"}"#,
                ),
            ],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let t = target(ws, &p.doc_id, p.report.as_deref(), p.page.as_deref(), ctx).await?;
        let (applied, rendered) = canvas::apply_bookmark_and_render(&t, &p.bookmark).await?;
        let pages = t.page_names()?;
        let CanvasAction::Bookmark {
            name,
            display_name,
            not_applied,
            ..
        } = applied.action
        else {
            return Err(OpError::internal("a bookmark gave another action"));
        };
        Ok(CanvasBookmarkOut {
            bookmark: name,
            display_name,
            navigated: applied.page.is_some_and(|i| i != t.page_index()),
            changed_pages: applied
                .changed
                .iter()
                .filter_map(|&i| pages.get(i).cloned())
                .collect(),
            not_applied,
            rendered,
        })
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

/// `canvas.contextMenu`.
#[derive(Debug)]
pub struct CanvasContextMenu;

impl Command for CanvasContextMenu {
    type Params = CanvasHitParams;
    type Output = ContextMenuOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.contextMenu",
            title: "Data point context menu",
            summary: "What a data point's context menu can copy (\"Copy value\", \"Copy selection\"), from the last render.",
            mutates_document: false,
            examples: &[(
                "A table cell",
                r#"{"docId": "d1", "hitId": "1f2e3d|dataPoint|1|4"}"#,
            )],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let t = target(ws, &p.doc_id, p.report.as_deref(), p.page.as_deref(), ctx).await?;
        canvas::context_menu(&t, &p.hit_id)
    }
}

// ---------------------------------------------------------------------------

/// `canvas.tooltipPage`.
#[derive(Debug)]
pub struct CanvasTooltipPage;

/// Result of `canvas.tooltipPage`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CanvasTooltipPageOut {
    /// The rendered report-page tooltip; absent when the visual shows none
    /// for this hit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<TooltipPageOut>,
}

impl Command for CanvasTooltipPage {
    type Params = CanvasHitParams;
    type Output = CanvasTooltipPageOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.tooltipPage",
            title: "Report-page tooltip",
            summary: "Renders the report-page tooltip of a data point hit, filtered by the point and the page's filters and slicers.",
            mutates_document: false,
            examples: &[(
                "Hover a column",
                r#"{"docId": "d1", "report": "Stores", "page": "Store Performance", "hitId": "1f2e3d|dataPoint|0|2"}"#,
            )],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let t = target(ws, &p.doc_id, p.report.as_deref(), p.page.as_deref(), ctx).await?;
        Ok(CanvasTooltipPageOut {
            tooltip: canvas::tooltip_page(&t, &p.hit_id).await?,
        })
    }
}

// ---------------------------------------------------------------------------

/// `canvas.drillthroughTargets`.
#[derive(Debug)]
pub struct CanvasDrillthroughTargets;

/// Result of `canvas.drillthroughTargets`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DrillthroughTargetsOut {
    /// Pages whose drillthrough fields the data point all has, in page
    /// order; empty for other hits.
    pub targets: Vec<DrillthroughTargetOut>,
}

impl Command for CanvasDrillthroughTargets {
    type Params = CanvasHitParams;
    type Output = DrillthroughTargetsOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.drillthroughTargets",
            title: "Drillthrough targets",
            summary: "The drillthrough pages a data point hit can drill through to (the \"Drill through\" menu).",
            mutates_document: false,
            examples: &[(
                "A bar's drillthrough pages",
                r#"{"docId": "d1", "hitId": "1f2e3d|dataPoint|0|2"}"#,
            )],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let t = target(ws, &p.doc_id, p.report.as_deref(), p.page.as_deref(), ctx).await?;
        Ok(DrillthroughTargetsOut {
            targets: canvas::drillthrough_targets(&t, &p.hit_id).await?,
        })
    }
}

// ---------------------------------------------------------------------------

/// `canvas.drillthrough`.
#[derive(Debug)]
pub struct CanvasDrillthrough;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanvasDrillthroughParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report.
    #[serde(default)]
    pub report: Option<String>,
    /// The source page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page.
    #[serde(default)]
    pub page: Option<String>,
    /// A data point hit id from the last render of the source page.
    /// Default: the page's selected data points.
    #[serde(default)]
    pub hit_id: Option<String>,
    /// The drillthrough page (see `canvas.drillthroughTargets`).
    pub target: String,
    /// The report of the drillthrough page, for a cross-report
    /// drillthrough (`report` of the target). Default: the source's report.
    #[serde(default)]
    pub target_report: Option<String>,
}

/// Result of `canvas.drillthrough`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CanvasDrillthroughOut {
    /// Internal name of the drillthrough page.
    pub page: String,
    pub display_name: String,
    /// The report of the drillthrough page, when it is another report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<String>,
    /// Parts of the source's filter context not kept (Top N filters, …).
    pub not_applied: Vec<String>,
    /// The drillthrough page, filtered and rendered.
    pub rendered: CanvasPageOut,
}

impl Command for CanvasDrillthrough {
    type Params = CanvasDrillthroughParams;
    type Output = CanvasDrillthroughOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "canvas.drillthrough",
            title: "Drill through",
            summary: "Drills through from a data point (or the page's selection) to a drillthrough page (also of another report): filters it on the point's values, keeps the source's filters if set and renders it.",
            long_running: true,
            examples: &[
                (
                    "From a bar",
                    r#"{"docId": "d1", "hitId": "1f2e3d|dataPoint|0|2", "target": "Store details"}"#,
                ),
                (
                    "From the selection",
                    r#"{"docId": "d1", "page": "Overview", "target": "Store details"}"#,
                ),
                (
                    "To another report",
                    r#"{"docId": "d1", "report": "Overview", "hitId": "1f2e3d|dataPoint|0|2", "target": "Store Detail", "targetReport": "Stores"}"#,
                ),
            ],
            ..CANVAS
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let t = target(ws, &p.doc_id, p.report.as_deref(), p.page.as_deref(), ctx).await?;
        let (action, rendered) = canvas::drillthrough(
            &t,
            p.hit_id.as_deref(),
            &p.target,
            p.target_report.as_deref(),
        )
        .await?;
        let CanvasAction::Drillthrough {
            page,
            display_name,
            report,
            not_applied,
        } = action
        else {
            return Err(OpError::internal("a drillthrough gave another action"));
        };
        Ok(CanvasDrillthroughOut {
            page,
            display_name,
            report,
            not_applied,
            rendered,
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
