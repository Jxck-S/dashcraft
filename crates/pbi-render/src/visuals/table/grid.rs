//! Grid engine shared by `tableEx` ([`super`]) and `pivotTable`
//! ([`crate::visuals::matrix`]): a [`GridModel`] (columns, header rows,
//! body rows, pinned total row) plus a [`GridStyle`] → column widths, row
//! heights, scrolling, clipping, scrollbars and the scene.
//!
//! # Geometry (page units, measured on the service renders)
//!
//! - Text lines use browser metrics: ascent and descent are rounded
//!   separately (`line height = round(ascent) + round(descent)`, 17px for
//!   10pt Segoe UI or Calibri, with a 14px or 13px ascent), as Chromium
//!   does for `line-height: normal`.
//! - Cells: 6px left/right padding, `grid.rowPadding / 2` (default 1px)
//!   above and below the text. A 1px horizontal grid line separates body
//!   rows (none after the last row).
//! - The last header row reserves an 8px strip under its text for the
//!   sort arrow (always, as the service does); a 1px outline (the theme's
//!   `tableAccent`) follows the header. The total row has the same line on
//!   top and follows the last body row (it scrolls with the rows: the
//!   service renders do not pin it).
//! - Lines may use [`wrap_slack`] (2%, at least 1px) of the right padding
//!   before wrapping, to absorb Selawik being slightly wider than Segoe UI.
//! - Calibri is measured and drawn with the bundled Carlito (Calibri's
//!   advance widths). Fonts we do not bundle but whose advance widths
//!   differ a lot from Selawik/Segoe UI (Arial Narrow; see
//!   [`width_factor`]) are measured with a width factor and drawn
//!   horizontally condensed by the same factor, so wrapping and auto-sized
//!   widths match the service.
//! - Column widths: an explicit width (`columnWidth`) is used as is;
//!   other columns auto-size to their widest header / value (first
//!   [`AUTO_SIZE_ROWS`] rows) plus padding, capped at
//!   [`AUTO_SIZE_MAX_FRACTION`] of the viewport when values wrap (their
//!   text then wraps). Tables narrower than the visual stay left-aligned
//!   (no stretching); wider ones scroll horizontally.
//! - Overflow: a 9px band on the right (vertical) and/or bottom
//!   (horizontal) holds a 7px rounded thumb (`#AEADAC`, at least 8px
//!   long). The header stays fixed when the body scrolls vertically; both
//!   scroll horizontally.
//! - Conditional formatting ([`GridCell::cf`]): the background replaces
//!   the band / column color, the font color the text color; a data bar
//!   fills [`BAR_INSET`] inside the cell under the text (an axis line where
//!   the range crosses zero); an icon ([`icon_size`] square, [`ICON_GAP`]
//!   from the text) sits before or after the value, or alone at the
//!   value's alignment, and counts in the column's auto width.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use pbi_model::SortDirection;
use serde::{Deserialize, Serialize};

use crate::conditional::{CellStyle, IconAlign, IconLayout};
use crate::layout::TextAlign;
use crate::scene::{
    Clip, Color, Group, HitId, Node, Paint, PathBuilder, PathNode, Rect, RectNode, Stroke,
    TextAnchor, TextNode, Transform,
};
use crate::text::{FontBook, MeasureOptions, TextStyle};
use crate::theme::VisualProps;
use crate::visuals::common::family_metrics;

/// Horizontal cell padding (left).
pub const CELL_PAD_LEFT: f64 = 6.0;
/// Horizontal cell padding (right).
pub const CELL_PAD_RIGHT: f64 = 6.0;
/// Height of the sort-arrow strip under the last header row's text.
pub const SORT_STRIP: f64 = 8.0;
/// Width of a scrollbar band.
pub const SCROLLBAR_BAND: f64 = 9.0;
/// Thickness of a scrollbar thumb.
pub const SCROLLBAR_THUMB: f64 = 7.0;
/// Shortest scrollbar thumb.
pub const SCROLLBAR_MIN_THUMB: f64 = 8.0;
/// Scrollbar thumb color.
pub const SCROLLBAR_COLOR: Color = Color::rgb(0xAE, 0xAD, 0xAC);
/// Default horizontal grid line color.
pub const GRID_LINE_COLOR: Color = Color::rgb(0xE5, 0xE5, 0xE5);
/// Default background of every second body row ("Default" style preset).
pub const BAND_COLOR: Color = Color::rgb(0xEE, 0xED, 0xED);
/// Rows sampled when auto-sizing a column.
pub const AUTO_SIZE_ROWS: usize = 500;
/// Largest auto-sized column, as a fraction of the viewport width, when
/// values wrap.
pub const AUTO_SIZE_MAX_FRACTION: f64 = 0.6;
/// Rows measured exactly; later rows get the average height.
pub const MAX_MEASURED_ROWS: usize = 5_000;
/// Most lines kept in one cell.
const MAX_CELL_LINES: usize = 40;
/// Narrowest column.
const MIN_COLUMN_WIDTH: f64 = 12.0;
/// Widest column (untrusted `columnWidth` values).
const MAX_COLUMN_WIDTH: f64 = 10_000.0;
/// Side of the matrix expand/collapse box.
const EXPANDER_SIZE: f64 = 9.0;
/// Gap between the expander and the row-header text.
const EXPANDER_GAP: f64 = 4.0;
/// Horizontal space an expander takes before the row-header text.
pub(crate) const EXPANDER_LEAD: f64 = EXPANDER_SIZE + EXPANDER_GAP;
/// Width of the sort arrow.
const SORT_ARROW_WIDTH: f64 = 8.5;
/// Space between a data bar and the cell edges.
pub const BAR_INSET: f64 = 2.0;
/// Space between a conditional icon and the value.
pub const ICON_GAP: f64 = 4.0;

/// Side of a conditional-formatting icon for text in `style`: three
/// quarters of the line height (13px for 10pt Segoe UI).
pub fn icon_size(style: &TextStyle) -> f64 {
    (line_metrics(style).0 * 0.75).round().clamp(8.0, 32.0)
}

/// Scroll position of a grid, in page units (clamped when rendering).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GridScroll {
    pub x: f64,
    pub y: f64,
}

/// Scroll geometry of a rendered grid (for the interactive canvas).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GridMetrics {
    /// The visual's content box.
    pub viewport: Rect,
    /// The vertically scrolling body (below the header, above the
    /// horizontal scrollbar).
    pub body: Rect,
    /// Sum of the column widths.
    pub content_width: f64,
    /// Height of all body rows and the total row (estimated beyond
    /// [`MAX_MEASURED_ROWS`]).
    pub rows_height: f64,
    /// Header height including its outline.
    pub header_height: f64,
    pub column_widths: Vec<f64>,
    /// The applied (clamped) scroll position.
    pub scroll: GridScroll,
    pub max_scroll_x: f64,
    pub max_scroll_y: f64,
    pub vertical_scrollbar: bool,
    pub horizontal_scrollbar: bool,
    /// Body rows drawn: `[first, last)`.
    pub visible_rows: (usize, usize),
}

/// Line height and (rounded) ascent for `style`, as a browser lays out
/// `line-height: normal` with the family's metrics (Segoe UI's, or
/// Calibri's for Calibri; see [`family_metrics`]).
pub fn line_metrics(style: &TextStyle) -> (f64, f64) {
    let m = family_metrics(style);
    let ascent = m.ascent_px(style).round();
    let descent = m.descent_px(style).round();
    (ascent + descent, ascent)
}

/// Extra width a line may use before it wraps: Selawik's advances run
/// 1–3% wider than Segoe UI's, so without slack lines that fit in the
/// service would wrap here. The slack comes out of the right padding.
pub fn wrap_slack(avail: f64) -> f64 {
    (avail * 0.02).max(1.0)
}

/// Advance-width ratio of `style`'s family to the bundled face it is drawn
/// with (1 for Segoe UI-like families and for Calibri, which is drawn with
/// the metric-compatible Carlito).
pub fn width_factor(style: &TextStyle) -> f64 {
    let first = style
        .family
        .split(',')
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches(|c| c == '\'' || c == '"')
        .to_ascii_lowercase();
    match first.as_str() {
        "arial narrow" => 0.82,
        _ => 1.0,
    }
}

/// A text line at `(x, baseline)`, condensed horizontally around `x` by
/// [`width_factor`] when the family is narrower than the bundled face.
pub(crate) fn text_line(
    x: f64,
    baseline: f64,
    text: String,
    style: &TextStyle,
    anchor: TextAnchor,
) -> Node {
    let node = TextNode::new(x, baseline, text, style.clone()).anchor(anchor);
    let f = width_factor(style);
    if (f - 1.0).abs() < 1e-9 {
        return node.into();
    }
    let t = Transform::translate(x, 0.0)
        .then(&Transform::scale(f, 1.0))
        .then(&Transform::translate(-x, 0.0));
    let mut g = Group::new().with_transform(t);
    g.push(node);
    g.into()
}

/// Ellipsizes one line to `max_width`, honouring [`width_factor`].
pub(crate) fn ellipsize(fonts: &FontBook, text: &str, style: &TextStyle, max_width: f64) -> String {
    fonts.ellipsize(text, style, max_width / width_factor(style))
}

/// Which text style of a [`GridStyle`] a piece of text uses (measurement
/// cache key).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum StyleId {
    Header,
    Values,
    RowHeader,
    Subtotal,
    Total,
}

/// Cached text measurement.
pub(crate) struct Measurer<'a> {
    fonts: &'a FontBook,
    widths: HashMap<(StyleId, String), f64>,
    wrapped: HashMap<(StyleId, String, u64, bool), Vec<String>>,
}

impl std::fmt::Debug for Measurer<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Measurer")
            .field("widths", &self.widths.len())
            .finish_non_exhaustive()
    }
}

impl<'a> Measurer<'a> {
    pub(crate) fn new(fonts: &'a FontBook) -> Self {
        Self {
            fonts,
            widths: HashMap::new(),
            wrapped: HashMap::new(),
        }
    }

    /// One-line advance width.
    pub(crate) fn width(&mut self, id: StyleId, style: &TextStyle, text: &str) -> f64 {
        if text.is_empty() {
            return 0.0;
        }
        let key = (id, text.to_owned());
        if let Some(w) = self.widths.get(&key) {
            return *w;
        }
        let w = self.fonts.text_width(text, style) * width_factor(style);
        self.widths.insert(key, w);
        w
    }

    /// Lines of `text` in `avail` pixels: wrapped (word, then glyph), or
    /// one ellipsized line per hard line without `wrap`.
    pub(crate) fn lines(
        &mut self,
        id: StyleId,
        style: &TextStyle,
        text: &str,
        avail: f64,
        wrap: bool,
    ) -> Vec<String> {
        if text.is_empty() {
            return Vec::new();
        }
        let avail = if avail.is_finite() {
            avail.max(1.0)
        } else {
            1.0
        };
        let avail = avail + wrap_slack(avail);
        if !text.contains('\n') && self.width(id, style, text) <= avail + 0.01 {
            return vec![text.to_owned()];
        }
        let key = (id, text.to_owned(), avail.to_bits(), wrap);
        if let Some(lines) = self.wrapped.get(&key) {
            return lines.clone();
        }
        let layout = self.fonts.measure_with(
            text,
            style,
            &MeasureOptions {
                max_width: Some(avail / width_factor(style)),
                wrap,
                max_lines: Some(MAX_CELL_LINES),
                ellipsis: true,
            },
        );
        let lines: Vec<String> = layout.lines.into_iter().map(|l| l.text).collect();
        self.wrapped.insert(key, lines.clone());
        lines
    }
}

/// A grid column.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GridColumn {
    /// Explicit width (`columnWidth`); `None` auto-sizes.
    pub width: Option<f64>,
    /// Value alignment.
    pub align: TextAlign,
    pub header_align: TextAlign,
    /// Sort arrow in the header.
    pub sort: Option<SortDirection>,
    /// Per-column overrides (`columnFormatting`).
    pub font_color: Option<Color>,
    pub back_color: Option<Color>,
}

impl Default for GridColumn {
    fn default() -> Self {
        Self {
            width: None,
            align: TextAlign::Left,
            header_align: TextAlign::Left,
            sort: None,
            font_color: None,
            back_color: None,
        }
    }
}

/// A header cell spanning `span` columns from `col`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HeaderCell {
    pub col: usize,
    pub span: usize,
    pub text: String,
    /// `None`: the first spanned column's header alignment.
    pub align: Option<TextAlign>,
}

/// Body row kind (styling).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RowKind {
    #[default]
    Data,
    /// A matrix parent / subtotal row.
    Subtotal,
    /// The (grand) total row.
    Total,
}

/// A body cell.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct GridCell {
    pub text: String,
    /// `None`: the column's alignment.
    pub align: Option<TextAlign>,
    /// Drawn with the row-header style (matrix row headers).
    pub row_header: bool,
    /// Left indentation (stepped layout).
    pub indent: f64,
    /// Expand/collapse box: `Some(expanded)`.
    pub expander: Option<bool>,
    /// `(series, index)` of the cell's data-point hit id.
    pub hit: Option<(usize, usize)>,
    /// Conditional formatting (background, font color, data bar, icon).
    pub cf: Option<Box<CellStyle>>,
}

impl GridCell {
    pub(crate) fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }
}

/// A body row.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct GridRow {
    pub cells: Vec<GridCell>,
    pub kind: RowKind,
}

/// Everything a grid shows.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct GridModel {
    pub columns: Vec<GridColumn>,
    /// Header rows, top to bottom.
    pub header_rows: Vec<Vec<HeaderCell>>,
    pub rows: Vec<GridRow>,
    /// Total row (after the body rows).
    pub total: Option<GridRow>,
    /// Data was cut at the query limits ("data reduced" hint).
    pub truncated: bool,
}

/// Background and font color of a body row band.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Band {
    pub bg: Option<Color>,
    pub font: Color,
}

/// A line (color, weight).
pub(crate) type Line = (Color, f64);

/// Resolved grid formatting.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GridStyle {
    pub header: TextStyle,
    pub header_bg: Option<Color>,
    pub header_wrap: bool,
    /// Header bottom / total top outline.
    pub outline: Option<Line>,
    pub values: TextStyle,
    pub values_wrap: bool,
    /// Odd (first) and even body rows.
    pub bands: [Band; 2],
    pub row_header: TextStyle,
    pub row_header_bg: Option<Color>,
    pub subtotal: TextStyle,
    pub subtotal_bg: Option<Color>,
    pub total: TextStyle,
    pub total_bg: Option<Color>,
    pub grid_h: Option<Line>,
    pub grid_v: Option<Line>,
    pub pad_v: f64,
}

impl GridStyle {
    fn text(&self, id: StyleId) -> &TextStyle {
        match id {
            StyleId::Header => &self.header,
            StyleId::Values => &self.values,
            StyleId::RowHeader => &self.row_header,
            StyleId::Subtotal => &self.subtotal,
            StyleId::Total => &self.total,
        }
    }

    /// Reads the formatting shared by tables and matrices. `total_object`
    /// is `total` (table) or `rowTotal` (matrix grand total).
    pub(crate) fn resolve(props: &VisualProps<'_>, total_object: &str) -> Self {
        let o = props.objects();
        let resolver = props.resolver();
        let preset = props
            .container()
            .text("stylePreset", "name")
            .or_else(|| o.text("stylePreset", "name"))
            .unwrap_or_default()
            .to_ascii_lowercase();
        let (bands_on, outline_on, grid_on) = match preset.as_str() {
            "none" => (false, false, false),
            "minimal" | "sparse" | "boldheader" | "condensed" => (false, true, true),
            _ => (true, true, true),
        };
        let text_size = o.number("grid", "textSize").filter(|s| *s > 0.0);
        let style = |object: &str| {
            let mut s = o.text_style(object, "label");
            if o.number(object, "fontSize").is_none()
                && let Some(size) = text_size
            {
                s.size_pt = size;
            }
            s
        };
        let bold_default = |object: &str| {
            let mut s = style(object);
            if o.bool(object, "bold").is_none() {
                s.weight = 700;
            }
            s
        };
        let values = style("values");
        let font0 = o
            .color("values", "fontColorPrimary")
            .or_else(|| o.color("values", "fontColor"))
            .unwrap_or(values.color);
        let font1 = o.color("values", "fontColorSecondary").unwrap_or(font0);
        let back = o.color("values", "backColor");
        let bg0 = o.color("values", "backColorPrimary").or(back);
        let bg1 = o
            .color("values", "backColorSecondary")
            .or(back)
            .or(bands_on.then_some(BAND_COLOR));
        let accent = resolver
            .named_color("tableAccent")
            .unwrap_or(Color::rgb(0x11, 0x8D, 0xFF));
        let outline_color = o.color("grid", "outlineColor").unwrap_or(accent);
        let outline_weight = o
            .number("grid", "outlineWeight")
            .filter(|w| *w >= 0.0)
            .unwrap_or(1.0)
            .min(20.0);
        let header_outline = o
            .text("columnHeaders", "outline")
            .map(|s| !s.eq_ignore_ascii_case("none"))
            .or_else(|| o.number("columnHeaders", "outlineStyle").map(|n| n != 0.0))
            .unwrap_or(outline_on);
        let line = |show: Option<bool>, default_on: bool, color: &str, weight: &str| {
            show.unwrap_or(default_on).then(|| {
                (
                    o.color("grid", color).unwrap_or(GRID_LINE_COLOR),
                    o.number("grid", weight)
                        .filter(|w| *w >= 0.0)
                        .unwrap_or(1.0)
                        .min(20.0),
                )
            })
        };
        let grid_h = line(
            o.bool("grid", "gridHorizontal"),
            grid_on,
            "gridHorizontalColor",
            "gridHorizontalWeight",
        );
        let grid_v = line(
            o.bool("grid", "gridVertical"),
            false,
            "gridVerticalColor",
            "gridVerticalWeight",
        );
        let pad_v = o
            .number("grid", "rowPadding")
            .filter(|p| *p >= 0.0)
            .map_or(1.0, |p| (p / 2.0).min(50.0));
        let mut total = bold_default(total_object);
        if o.color(total_object, "fontColor").is_none() {
            total.color = font0;
        }
        let mut row_header = style("rowHeaders");
        if o.color("rowHeaders", "fontColor").is_none() {
            row_header.color = font0;
        }
        let mut subtotal = bold_default("subTotals");
        if o.color("subTotals", "fontColor").is_none() {
            subtotal.color = font0;
        }
        Self {
            header: style("columnHeaders"),
            header_bg: o.color("columnHeaders", "backColor"),
            header_wrap: o.bool("columnHeaders", "wordWrap").unwrap_or(true),
            outline: (header_outline && outline_weight > 0.0)
                .then_some((outline_color, outline_weight)),
            values_wrap: o.bool("values", "wordWrap").unwrap_or(true),
            values,
            bands: [
                Band {
                    bg: bg0,
                    font: font0,
                },
                Band {
                    bg: bg1,
                    font: font1,
                },
            ],
            row_header_bg: o.color("rowHeaders", "backColor"),
            row_header,
            subtotal_bg: o.color("subTotals", "backColor"),
            subtotal,
            total_bg: o.color(total_object, "backColor"),
            total,
            grid_h,
            grid_v,
            pad_v,
        }
    }
}

/// Scroll-independent measurements of a grid: column widths, header and
/// row heights. Measuring every row's wrapped text is the expensive part of
/// a table render, and scrolling only changes which rows are drawn, so the
/// result is memoized by a fingerprint of everything it depends on (model
/// text and structure, style, viewport width). The per-row vectors are
/// shared (`Arc`), so a grid built from a layout copies nothing
/// proportional to the rows.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GridLayout {
    widths: Vec<f64>,
    col_x: Vec<f64>,
    header_heights: Vec<f64>,
    header_total: f64,
    row_tops: Arc<[f64]>,
    row_heights: Arc<[f64]>,
    rows_height: f64,
    total_height: f64,
}

/// A grid's model, style and layout, built once and drawn at any scroll
/// position ([`Grid::with_layout`]); matrices also keep their body rows.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PreparedGrid {
    pub model: GridModel,
    pub style: GridStyle,
    pub layout: Arc<GridLayout>,
    /// Matrix body rows in display order (empty for tables).
    pub rows: Vec<crate::visuals::matrix::MatrixRowRef>,
}

impl PreparedGrid {
    /// Lays out `model` (memoized; see [`Grid::layout`]).
    pub(crate) fn new(env: GridEnv<'_>, model: GridModel, style: GridStyle) -> Self {
        let layout = Grid::layout(env, &model, &style);
        Self {
            model,
            style,
            layout,
            rows: Vec::new(),
        }
    }

    /// Cells of the model (a size measure for caches).
    pub(crate) fn cells(&self) -> usize {
        self.model
            .rows
            .iter()
            .chain(self.model.total.iter())
            .map(|r| r.cells.len())
            .sum::<usize>()
            + self.rows.len()
    }

    /// The grid at `scroll`: its content group and scroll geometry.
    pub(crate) fn render(&self, env: GridEnv<'_>, scroll: GridScroll) -> (Group, GridMetrics) {
        let mut grid = Grid::with_layout(env, &self.model, &self.style, &self.layout, scroll);
        let metrics = grid.metrics().clone();
        (grid.render(), metrics)
    }
}

/// Layouts kept before the memo is cleared (a page has a few grids; this
/// covers several pages, zoom levels and selections).
const LAYOUT_CACHE_MAX: usize = 64;

/// Layouts by (fingerprint, row count, column count).
type LayoutCache = HashMap<(u64, usize, usize), Arc<GridLayout>>;

fn layout_cache() -> &'static Mutex<LayoutCache> {
    static CACHE: OnceLock<Mutex<LayoutCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn hash_text_style(h: &mut impl Hasher, s: &TextStyle) {
    s.family.hash(h);
    s.size_pt.to_bits().hash(h);
    s.weight.hash(h);
    s.italic.hash(h);
}

/// Fingerprint of everything [`GridLayout`] depends on (colors and other
/// paint-only properties excluded).
fn layout_fingerprint(model: &GridModel, style: &GridStyle, viewport_width: f64) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    viewport_width.to_bits().hash(&mut h);
    for c in &model.columns {
        c.width.map(f64::to_bits).hash(&mut h);
    }
    for row in &model.header_rows {
        row.len().hash(&mut h);
        for c in row {
            (c.col, c.span).hash(&mut h);
            c.text.hash(&mut h);
        }
    }
    let hash_row = |h: &mut std::collections::hash_map::DefaultHasher, row: &GridRow| {
        (row.kind as u8, row.cells.len()).hash(h);
        for c in &row.cells {
            c.text.hash(h);
            c.indent.to_bits().hash(h);
            c.expander.hash(h);
            c.row_header.hash(h);
        }
    };
    for row in &model.rows {
        hash_row(&mut h, row);
    }
    if let Some(total) = &model.total {
        1u8.hash(&mut h);
        hash_row(&mut h, total);
    }
    for s in [
        &style.header,
        &style.values,
        &style.row_header,
        &style.subtotal,
        &style.total,
    ] {
        hash_text_style(&mut h, s);
    }
    (style.header_wrap, style.values_wrap).hash(&mut h);
    style.pad_v.to_bits().hash(&mut h);
    style.grid_h.map(|g| g.1.to_bits()).hash(&mut h);
    style.outline.map(|o| o.1.to_bits()).hash(&mut h);
    // Conditional icons take room: where they are, then how many (nothing
    // is hashed per cell without them).
    let mut icons = 0usize;
    for (r, row) in model.rows.iter().chain(model.total.iter()).enumerate() {
        for (c, cell) in row.cells.iter().enumerate() {
            if let Some((_, layout, _)) = cell.cf.as_ref().and_then(|s| s.icon) {
                (r, c, layout as u8).hash(&mut h);
                icons += 1;
            }
        }
    }
    icons.hash(&mut h);
    h.finish()
}

/// Where and what to draw.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GridEnv<'a> {
    pub fonts: &'a FontBook,
    /// The visual's content box.
    pub viewport: Rect,
    /// Visual name for hit ids.
    pub visual: &'a str,
}

/// A laid-out grid.
#[derive(Debug)]
pub(crate) struct Grid<'m, 'f> {
    env: GridEnv<'f>,
    model: &'m GridModel,
    style: &'m GridStyle,
    m: Measurer<'f>,
    widths: Vec<f64>,
    /// Left edge of each column relative to the grid origin.
    col_x: Vec<f64>,
    header_heights: Vec<f64>,
    /// Header rows + outline.
    header_total: f64,
    /// Top of each body row relative to the body origin (unscrolled).
    row_tops: Arc<[f64]>,
    row_heights: Arc<[f64]>,
    rows_height: f64,
    total_height: f64,
    metrics: GridMetrics,
}

impl<'m, 'f> Grid<'m, 'f> {
    fn blank(env: GridEnv<'f>, model: &'m GridModel, style: &'m GridStyle) -> Self {
        Self {
            env,
            model,
            style,
            m: Measurer::new(env.fonts),
            widths: Vec::new(),
            col_x: Vec::new(),
            header_heights: Vec::new(),
            header_total: 0.0,
            row_tops: Arc::from([]),
            row_heights: Arc::from([]),
            rows_height: 0.0,
            total_height: 0.0,
            metrics: GridMetrics::default(),
        }
    }

    /// Lays out `model` in the viewport at `scroll`.
    pub(crate) fn new(
        env: GridEnv<'f>,
        model: &'m GridModel,
        style: &'m GridStyle,
        scroll: GridScroll,
    ) -> Self {
        let layout = Self::layout(env, model, style);
        Self::with_layout(env, model, style, &layout, scroll)
    }

    /// `model` at `scroll` with a layout from [`Grid::layout`] (for the
    /// same model, style and viewport width).
    pub(crate) fn with_layout(
        env: GridEnv<'f>,
        model: &'m GridModel,
        style: &'m GridStyle,
        layout: &GridLayout,
        scroll: GridScroll,
    ) -> Self {
        let mut grid = Self::blank(env, model, style);
        grid.apply_layout(layout);
        grid.scroll_geometry(scroll);
        grid
    }

    /// The scroll-independent layout of `model` (memoized by content).
    pub(crate) fn layout(
        env: GridEnv<'f>,
        model: &'m GridModel,
        style: &'m GridStyle,
    ) -> Arc<GridLayout> {
        let key = (
            layout_fingerprint(model, style, env.viewport.width),
            model.rows.len(),
            model.columns.len(),
        );
        let cached = layout_cache()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
            .cloned();
        if let Some(layout) = cached {
            return layout;
        }
        let layout = Arc::new(Self::blank(env, model, style).measure());
        let mut cache = layout_cache()
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if cache.len() >= LAYOUT_CACHE_MAX {
            cache.clear();
        }
        cache.insert(key, Arc::clone(&layout));
        layout
    }

    /// Measures columns, header and rows (scroll-independent).
    fn measure(&mut self) -> GridLayout {
        self.widths = self.column_widths();
        let mut x = 0.0;
        self.col_x.clear();
        for w in &self.widths {
            self.col_x.push(x);
            x += w;
        }
        self.measure_header();
        self.measure_rows();
        let model = self.model;
        self.total_height = match &model.total {
            Some(row) => self.row_height(row) + self.outline_weight(),
            None => 0.0,
        };
        GridLayout {
            widths: self.widths.clone(),
            col_x: self.col_x.clone(),
            header_heights: self.header_heights.clone(),
            header_total: self.header_total,
            row_tops: Arc::clone(&self.row_tops),
            row_heights: Arc::clone(&self.row_heights),
            rows_height: self.rows_height,
            total_height: self.total_height,
        }
    }

    fn apply_layout(&mut self, layout: &GridLayout) {
        self.widths.clone_from(&layout.widths);
        self.col_x.clone_from(&layout.col_x);
        self.header_heights.clone_from(&layout.header_heights);
        self.header_total = layout.header_total;
        self.row_tops = Arc::clone(&layout.row_tops);
        self.row_heights = Arc::clone(&layout.row_heights);
        self.rows_height = layout.rows_height;
        self.total_height = layout.total_height;
    }

    pub(crate) fn metrics(&self) -> &GridMetrics {
        &self.metrics
    }

    fn outline_weight(&self) -> f64 {
        self.style.outline.map_or(0.0, |o| o.1)
    }

    fn separator(&self) -> f64 {
        self.style.grid_h.map_or(0.0, |g| g.1)
    }

    fn content_width(&self) -> f64 {
        self.widths.iter().sum()
    }

    fn cell_style(kind: RowKind, cell: &GridCell) -> StyleId {
        match kind {
            RowKind::Total => StyleId::Total,
            RowKind::Subtotal => StyleId::Subtotal,
            RowKind::Data if cell.row_header => StyleId::RowHeader,
            RowKind::Data => StyleId::Values,
        }
    }

    /// Space taken by indentation and the expander before a cell's text.
    fn cell_lead(cell: &GridCell) -> f64 {
        cell.indent.max(0.0)
            + if cell.expander.is_some() {
                EXPANDER_LEAD
            } else {
                0.0
            }
    }

    /// Space a conditional icon takes `(before, after)` the text.
    fn icon_space(&self, kind: RowKind, cell: &GridCell) -> (f64, f64) {
        let Some((_, layout, _)) = cell.cf.as_ref().and_then(|s| s.icon) else {
            return (0.0, 0.0);
        };
        let size = icon_size(self.style.text(Self::cell_style(kind, cell)));
        match layout {
            IconLayout::Before => (size + ICON_GAP, 0.0),
            IconLayout::After => (0.0, size + ICON_GAP),
            IconLayout::IconOnly => (size, 0.0),
        }
    }

    fn column_widths(&mut self) -> Vec<f64> {
        let model = self.model;
        let style = self.style;
        let pads = CELL_PAD_LEFT + CELL_PAD_RIGHT;
        let cap = if style.values_wrap {
            (self.env.viewport.width * AUTO_SIZE_MAX_FRACTION).max(pads + 48.0)
        } else {
            MAX_COLUMN_WIDTH
        };
        let mut widths = Vec::with_capacity(model.columns.len());
        for (i, col) in model.columns.iter().enumerate() {
            if let Some(w) = col.width.filter(|w| w.is_finite()) {
                widths.push(w.clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH));
                continue;
            }
            let mut natural: f64 = 0.0;
            for hc in model
                .header_rows
                .iter()
                .flatten()
                .filter(|h| h.col == i && h.span <= 1)
            {
                natural = natural.max(self.m.width(StyleId::Header, &style.header, &hc.text));
            }
            let body = model
                .rows
                .iter()
                .take(AUTO_SIZE_ROWS)
                .chain(model.total.iter());
            for row in body {
                if let Some(cell) = row.cells.get(i) {
                    let id = Self::cell_style(row.kind, cell);
                    let (before, after) = self.icon_space(row.kind, cell);
                    let w = self.m.width(id, style.text(id), &cell.text)
                        + Self::cell_lead(cell)
                        + before
                        + after;
                    natural = natural.max(w);
                }
            }
            let w = (natural + pads).min(cap);
            widths.push(w.clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH));
        }
        // Spanning header cells widen the last spanned auto-sized column.
        for hc in model.header_rows.iter().flatten().filter(|h| h.span > 1) {
            let end = (hc.col + hc.span).min(widths.len());
            let Some(spanned) = widths.get(hc.col..end) else {
                continue;
            };
            let have: f64 = spanned.iter().sum();
            let need = self.m.width(StyleId::Header, &style.header, &hc.text) + pads;
            if need > have
                && let Some(last) = (hc.col..end)
                    .rev()
                    .find(|&c| model.columns.get(c).is_some_and(|c| c.width.is_none()))
                && let Some(w) = widths.get_mut(last)
            {
                *w = (*w + need - have).min(cap.max(*w));
            }
        }
        widths
    }

    fn span_width(&self, col: usize, span: usize) -> f64 {
        let end = (col + span.max(1)).min(self.widths.len());
        self.widths.get(col..end).map_or(0.0, |w| w.iter().sum())
    }

    fn header_lines(&mut self, cell: &HeaderCell) -> Vec<String> {
        let avail = self.span_width(cell.col, cell.span) - CELL_PAD_LEFT - CELL_PAD_RIGHT;
        let style = self.style;
        self.m.lines(
            StyleId::Header,
            &style.header,
            &cell.text,
            avail,
            style.header_wrap,
        )
    }

    fn measure_header(&mut self) {
        let (lh, _) = line_metrics(&self.style.header);
        let model = self.model;
        let rows = model.header_rows.len();
        let mut heights = Vec::with_capacity(rows);
        for (r, cells) in model.header_rows.iter().enumerate() {
            let lines = cells
                .iter()
                .map(|c| self.header_lines(c).len())
                .max()
                .unwrap_or(0)
                .max(1);
            let mut h = lines as f64 * lh + 2.0 * self.style.pad_v;
            if r + 1 == rows {
                h += SORT_STRIP;
            }
            heights.push(h);
        }
        self.header_total =
            heights.iter().sum::<f64>() + if rows > 0 { self.outline_weight() } else { 0.0 };
        self.header_heights = heights;
    }

    fn cell_lines(&mut self, kind: RowKind, col: usize, cell: &GridCell) -> Vec<String> {
        let id = Self::cell_style(kind, cell);
        let style = self.style;
        let (before, after) = self.icon_space(kind, cell);
        let avail = self.widths.get(col).copied().unwrap_or(0.0)
            - CELL_PAD_LEFT
            - CELL_PAD_RIGHT
            - Self::cell_lead(cell)
            - before
            - after;
        self.m
            .lines(id, style.text(id), &cell.text, avail, style.values_wrap)
    }

    fn row_height(&mut self, row: &GridRow) -> f64 {
        let mut lines = 1usize;
        let mut lh: f64 = 0.0;
        for (c, cell) in row.cells.iter().enumerate() {
            let id = Self::cell_style(row.kind, cell);
            lh = lh.max(line_metrics(self.style.text(id)).0);
            lines = lines.max(self.cell_lines(row.kind, c, cell).len());
        }
        if lh <= 0.0 {
            lh = line_metrics(&self.style.values).0;
        }
        lines as f64 * lh + 2.0 * self.style.pad_v
    }

    fn measure_rows(&mut self) {
        let sep = self.separator();
        let mut top = 0.0;
        let model = self.model;
        let n = model.rows.len();
        let measured = n.min(MAX_MEASURED_ROWS);
        let mut row_tops = Vec::with_capacity(n);
        let mut row_heights = Vec::with_capacity(n);
        for row in model.rows.iter().take(measured) {
            let h = self.row_height(row);
            row_tops.push(top);
            row_heights.push(h);
            top += h + sep;
        }
        if n > measured {
            let avg = if measured > 0 {
                row_heights.iter().sum::<f64>() / measured as f64
            } else {
                line_metrics(&self.style.values).0 + 2.0 * self.style.pad_v
            };
            for _ in measured..n {
                row_tops.push(top);
                row_heights.push(avg);
                top += avg + sep;
            }
        }
        self.row_tops = Arc::from(row_tops);
        self.row_heights = Arc::from(row_heights);
        self.rows_height = if n > 0 { top - sep } else { 0.0 };
    }

    fn scroll_geometry(&mut self, scroll: GridScroll) {
        let vp = self.env.viewport;
        let content_w = self.content_width();
        let scroll_h = self.rows_height + self.total_height;
        let content_h = self.header_total + scroll_h;
        let band = |on: bool| if on { SCROLLBAR_BAND } else { 0.0 };
        let mut need_v = content_h > vp.height + 0.5;
        let need_h = content_w > vp.width - band(need_v) + 0.5;
        if need_h && !need_v {
            need_v = content_h > vp.height - SCROLLBAR_BAND + 0.5;
        }
        let view_w = (vp.width - band(need_v)).max(0.0);
        let view_h = (vp.height - band(need_h)).max(0.0);
        let body_y = vp.y + self.header_total;
        let body_h = scroll_h.min((view_h - self.header_total).max(0.0));
        let max_x = (content_w - view_w).max(0.0);
        let max_y = (scroll_h - body_h).max(0.0);
        let clamp = |v: f64, max: f64| {
            if v.is_finite() {
                v.clamp(0.0, max)
            } else {
                0.0
            }
        };
        let scroll = GridScroll {
            x: clamp(scroll.x, max_x),
            y: clamp(scroll.y, max_y),
        };
        let first = self
            .row_tops
            .partition_point(|t| *t <= scroll.y)
            .saturating_sub(1);
        let last = self
            .row_tops
            .partition_point(|t| *t < scroll.y + body_h)
            .max(first);
        self.metrics = GridMetrics {
            viewport: vp,
            body: Rect::new(vp.x, body_y, view_w, body_h),
            content_width: content_w,
            rows_height: scroll_h,
            header_height: self.header_total,
            column_widths: self.widths.clone(),
            scroll,
            max_scroll_x: max_x,
            max_scroll_y: max_y,
            vertical_scrollbar: need_v,
            horizontal_scrollbar: need_h,
            visible_rows: (first.min(self.model.rows.len()), last),
        };
    }

    /// The x and anchor of a cell's text for `align` within `[x, x + w]`.
    fn text_x(align: TextAlign, x: f64, w: f64, lead: f64) -> (f64, TextAnchor) {
        Self::text_x_in(align, x, w, lead, 0.0)
    }

    /// [`Grid::text_x`] with `trail` reserved after the text.
    fn text_x_in(align: TextAlign, x: f64, w: f64, lead: f64, trail: f64) -> (f64, TextAnchor) {
        match align {
            TextAlign::Left => (x + CELL_PAD_LEFT + lead, TextAnchor::Start),
            TextAlign::Center => (x + lead + (w - lead - trail) / 2.0, TextAnchor::Middle),
            TextAlign::Right => (x + w - CELL_PAD_RIGHT - trail, TextAnchor::End),
        }
    }

    /// Renders the grid.
    pub(crate) fn render(&mut self) -> Group {
        let mut g = Group::new().with_class("pbi-grid");
        let vp = self.env.viewport;
        let mt = self.metrics.clone();
        let x0 = vp.x - mt.scroll.x;
        let content_w = self.content_width();

        g.push(self.render_body(x0, content_w));
        if !self.model.header_rows.is_empty() {
            g.push(self.render_header(x0, content_w));
        }
        let model = self.model;
        if let Some(total) = &model.total {
            let ty = mt.body.y + self.rows_height - mt.scroll.y;
            if ty < mt.body.bottom() {
                let mut tg = Group::new().with_clip(Clip::rect(mt.body));
                let ow = self.outline_weight();
                if let Some(bg) = self.style.total_bg.or(self.style.bands[0].bg) {
                    tg.push(
                        RectNode::new(Rect::new(x0, ty, content_w, self.total_height))
                            .fill(Paint::solid(bg)),
                    );
                }
                if let Some((color, w)) = self.style.outline {
                    tg.push(
                        RectNode::new(Rect::new(x0, ty, content_w, w)).fill(Paint::solid(color)),
                    );
                }
                let h = self.total_height - ow;
                let font = self.style.total.color;
                self.draw_row(&mut tg, total, (x0, ty + ow, h), font);
                self.vertical_lines(&mut tg, x0, ty, self.total_height);
                g.push(tg);
            }
        }
        self.scrollbars(&mut g);
        if self.model.truncated {
            g.push(data_reduced_badge(vp));
        }
        g
    }

    fn render_body(&mut self, x0: f64, content_w: f64) -> Group {
        let mt = self.metrics.clone();
        let mut body = Group::new().with_clip(Clip::rect(mt.body));
        let (first, last) = mt.visible_rows;
        let sep = self.separator();
        let model = self.model;
        for i in first..last {
            let (Some(row), Some(&top), Some(&h)) = (
                model.rows.get(i),
                self.row_tops.get(i),
                self.row_heights.get(i),
            ) else {
                continue;
            };
            let y = mt.body.y + top - mt.scroll.y;
            let band = self.style.bands[i % 2];
            let bg = match row.kind {
                RowKind::Subtotal => self.style.subtotal_bg.or(band.bg),
                _ => band.bg,
            };
            if let Some(bg) = bg {
                body.push(RectNode::new(Rect::new(x0, y, content_w, h)).fill(Paint::solid(bg)));
            }
            if i + 1 < model.rows.len()
                && sep > 0.0
                && let Some((color, _)) = self.style.grid_h
            {
                body.push(
                    RectNode::new(Rect::new(x0, y + h, content_w, sep)).fill(Paint::solid(color)),
                );
            }
            self.draw_row(&mut body, row, (x0, y, h), band.font);
        }
        self.vertical_lines(&mut body, x0, mt.body.y, mt.body.height);
        body
    }

    fn render_header(&mut self, x0: f64, content_w: f64) -> Group {
        let vp = self.env.viewport;
        let view_w = self.metrics.body.width;
        let mut header =
            Group::new().with_clip(Clip::rect(Rect::new(vp.x, vp.y, view_w, self.header_total)));
        let header_h: f64 = self.header_heights.iter().sum();
        if let Some(bg) = self.style.header_bg {
            header.push(
                RectNode::new(Rect::new(x0, vp.y, content_w, header_h)).fill(Paint::solid(bg)),
            );
        }
        let style = self.style;
        let (lh, ascent) = line_metrics(&style.header);
        let mut y = vp.y;
        let model = self.model;
        let rows = model.header_rows.len();
        let view_right = vp.x + view_w;
        for (r, cells) in model.header_rows.iter().enumerate() {
            let h = self.header_heights.get(r).copied().unwrap_or(0.0);
            for cell in cells {
                let x = x0 + self.col_x.get(cell.col).copied().unwrap_or(0.0);
                let w = self.span_width(cell.col, cell.span);
                if x > view_right || x + w < vp.x {
                    continue;
                }
                let align = cell.align.unwrap_or_else(|| {
                    model
                        .columns
                        .get(cell.col)
                        .map_or(TextAlign::Left, |c| c.header_align)
                });
                let (tx, anchor) = Self::text_x(align, x, w, 0.0);
                for (k, line) in self.header_lines(cell).into_iter().enumerate() {
                    let baseline = y + style.pad_v + ascent + k as f64 * lh;
                    header.push(text_line(tx, baseline, line, &style.header, anchor));
                }
                if r + 1 == rows
                    && cell.span <= 1
                    && let Some(dir) = model.columns.get(cell.col).and_then(|c| c.sort)
                {
                    let ax = match align {
                        TextAlign::Left => x + CELL_PAD_LEFT,
                        TextAlign::Center => x + (w - SORT_ARROW_WIDTH) / 2.0,
                        TextAlign::Right => x + w - CELL_PAD_RIGHT - SORT_ARROW_WIDTH,
                    };
                    let strip_top = y + h - style.pad_v - SORT_STRIP;
                    header.push(sort_arrow(ax, strip_top + 1.0, dir, style.header.color));
                }
            }
            y += h;
        }
        if let Some((color, w)) = style.outline {
            header.push(
                RectNode::new(Rect::new(x0, vp.y + header_h, content_w, w))
                    .fill(Paint::solid(color)),
            );
        }
        self.vertical_lines(&mut header, x0, vp.y, header_h);
        header
    }

    fn vertical_lines(&self, g: &mut Group, x0: f64, y: f64, h: f64) {
        let Some((color, w)) = self.style.grid_v else {
            return;
        };
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        for cx in self.col_x.iter().skip(1) {
            g.push(RectNode::new(Rect::new(x0 + cx - w / 2.0, y, w, h)).fill(Paint::solid(color)));
        }
    }

    /// Draws a row's cells in the band `(x0, y, height)`.
    fn draw_row(&mut self, g: &mut Group, row: &GridRow, band: (f64, f64, f64), font: Color) {
        let (x0, y, h) = band;
        let view = self.metrics.body;
        let pad_v = self.style.pad_v;
        let model = self.model;
        for (c, cell) in row.cells.iter().enumerate() {
            let (Some(&cx), Some(&w)) = (self.col_x.get(c), self.widths.get(c)) else {
                continue;
            };
            let x = x0 + cx;
            if x > view.right() || x + w < view.x {
                continue;
            }
            let column = model.columns.get(c);
            let id = Self::cell_style(row.kind, cell);
            let mut text_style = self.style.text(id).clone();
            if row.kind == RowKind::Data && !cell.row_header {
                text_style.color = font;
            }
            if let Some(fc) = column.and_then(|c| c.font_color) {
                text_style.color = fc;
            }
            let cf = cell.cf.as_deref();
            if let Some(fc) = cf.and_then(|s| s.font) {
                text_style.color = fc;
            }
            let mut cg = Group::new();
            let cell_rect = Rect::new(x, y, w, h);
            let back = column
                .and_then(|c| c.back_color)
                .or(if cell.row_header && row.kind == RowKind::Data {
                    self.style.row_header_bg
                } else {
                    None
                })
                .map(Paint::solid);
            match cf.and_then(|s| s.background).or(back) {
                Some(bg) => {
                    cg.push(RectNode::new(cell_rect).fill(bg));
                }
                None if cell.hit.is_some() => {
                    cg.push(RectNode::new(cell_rect).fill(Paint::new(Color::WHITE, 0.0)));
                }
                None => {}
            }
            if let Some(bar) = cf.and_then(|s| s.bar) {
                data_bar(&mut cg, cell_rect, &bar);
            }
            let (lh, ascent) = line_metrics(&text_style);
            let max_lines = if lh > 0.0 {
                ((h - 2.0 * pad_v) / lh).round().max(1.0) as usize
            } else {
                1
            };
            let mut lead = Self::cell_lead(cell);
            let mut trail = 0.0;
            if let Some(expanded) = cell.expander {
                let ex = x + CELL_PAD_LEFT + cell.indent.max(0.0);
                let ey = y + pad_v + ((lh - EXPANDER_SIZE) / 2.0).max(0.0);
                cg.push(expander(ex, ey, expanded, text_style.color));
            }
            let align = cell
                .align
                .unwrap_or_else(|| column.map_or(TextAlign::Left, |c| c.align));
            if let Some((icon, layout, valign)) = cf.and_then(|s| s.icon) {
                let size = icon_size(&text_style);
                let iy = match valign {
                    IconAlign::Top => y + pad_v + (lh - size) / 2.0,
                    IconAlign::Middle => y + (h - size) / 2.0,
                    IconAlign::Bottom => y + h - pad_v - lh + (lh - size) / 2.0,
                };
                let left = x + CELL_PAD_LEFT + lead;
                let ix = match (layout, align) {
                    (IconLayout::Before, _) | (IconLayout::IconOnly, TextAlign::Left) => left,
                    (IconLayout::After, _) | (IconLayout::IconOnly, TextAlign::Right) => {
                        x + w - CELL_PAD_RIGHT - size
                    }
                    (IconLayout::IconOnly, TextAlign::Center) => left + (w - lead - size) / 2.0,
                };
                match layout {
                    IconLayout::Before => lead += size + ICON_GAP,
                    IconLayout::After => trail = size + ICON_GAP,
                    IconLayout::IconOnly => {}
                }
                cg.push(icon.draw(ix, iy, size));
            }
            let (tx, anchor) = Self::text_x_in(align, x, w, lead, trail);
            let lines = self.cell_lines(row.kind, c, cell);
            for (k, line) in lines.into_iter().take(max_lines).enumerate() {
                let baseline = y + pad_v + ascent + k as f64 * lh;
                cg.push(text_line(tx, baseline, line, &text_style, anchor));
            }
            if let Some((series, index)) = cell.hit {
                cg = cg.with_id(HitId::data_point(self.env.visual, series, index));
            }
            if !cg.is_empty() {
                g.push(cg);
            }
        }
    }

    fn scrollbars(&self, g: &mut Group) {
        let vp = self.env.viewport;
        let mt = &self.metrics;
        let inset = (SCROLLBAR_BAND - SCROLLBAR_THUMB) / 2.0;
        if mt.vertical_scrollbar {
            let view_h = vp.height
                - if mt.horizontal_scrollbar {
                    SCROLLBAR_BAND
                } else {
                    0.0
                };
            let track = (view_h - 2.0).max(0.0);
            let ratio = if mt.rows_height > 0.0 {
                mt.body.height / mt.rows_height
            } else {
                1.0
            };
            let (pos, len) = thumb(track, ratio, mt.scroll.y, mt.max_scroll_y);
            let rect = Rect::new(
                vp.right() - SCROLLBAR_BAND + inset,
                vp.y + 1.0 + pos,
                SCROLLBAR_THUMB,
                len,
            );
            g.push(scrollbar_thumb(rect));
        }
        if mt.horizontal_scrollbar {
            let track = (mt.body.width - 4.0).max(0.0);
            let ratio = if mt.content_width > 0.0 {
                mt.body.width / mt.content_width
            } else {
                1.0
            };
            let (pos, len) = thumb(track, ratio, mt.scroll.x, mt.max_scroll_x);
            let rect = Rect::new(
                vp.x + 2.0 + pos,
                vp.bottom() - SCROLLBAR_BAND + inset,
                len,
                SCROLLBAR_THUMB,
            );
            g.push(scrollbar_thumb(rect));
        }
    }
}

/// A conditional-formatting data bar in `cell` (see [`BAR_INSET`]).
fn data_bar(g: &mut Group, cell: Rect, bar: &crate::conditional::Bar) {
    let area = Rect::new(
        cell.x + BAR_INSET,
        cell.y + BAR_INSET,
        (cell.width - 2.0 * BAR_INSET).max(0.0),
        (cell.height - 2.0 * BAR_INSET).max(0.0),
    );
    if area.is_empty() {
        return;
    }
    let (from, to) = (area.x + area.width * bar.from, area.x + area.width * bar.to);
    if to - from > 0.0 {
        g.push(RectNode::new(Rect::new(from, area.y, to - from, area.height)).fill(bar.paint));
    }
    if let Some((at, color)) = bar.axis {
        let ax = (area.x + area.width * at).round();
        g.push(
            RectNode::new(Rect::new(ax - 0.5, area.y, 1.0, area.height)).fill(Paint::solid(color)),
        );
    }
}

/// Thumb `(offset, length)` on a track for a visible `ratio` and a scroll
/// position.
pub(crate) fn thumb(track: f64, ratio: f64, scroll: f64, max_scroll: f64) -> (f64, f64) {
    let ratio = if ratio.is_finite() {
        ratio.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let len = (track * ratio).max(SCROLLBAR_MIN_THUMB).min(track);
    let pos = if max_scroll > 0.0 {
        (track - len) * (scroll / max_scroll).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (pos, len)
}

/// A rounded scrollbar thumb.
pub(crate) fn scrollbar_thumb(rect: Rect) -> RectNode {
    RectNode::new(rect)
        .radius(rect.width.min(rect.height) / 2.0)
        .fill(Paint::solid(SCROLLBAR_COLOR))
}

/// A filled sort triangle (`▲` ascending, `▼` descending) with its top at
/// `y`.
fn sort_arrow(x: f64, y: f64, dir: SortDirection, color: Color) -> PathNode {
    let w = SORT_ARROW_WIDTH;
    let h = w / 2.0;
    let mut p = PathBuilder::new();
    match dir {
        SortDirection::Ascending => {
            p.move_to(x, y + h)
                .line_to(x + w / 2.0, y)
                .line_to(x + w, y + h);
        }
        SortDirection::Descending => {
            p.move_to(x, y)
                .line_to(x + w / 2.0, y + h)
                .line_to(x + w, y);
        }
    }
    p.close();
    PathNode::new(p.build()).fill(Paint::solid(color))
}

/// A matrix expand (`+`) / collapse (`−`) box.
fn expander(x: f64, y: f64, expanded: bool, color: Color) -> Group {
    let mut g = Group::new().with_class("pbi-expander");
    let s = EXPANDER_SIZE;
    g.push(
        RectNode::new(Rect::new(x + 0.5, y + 0.5, s - 1.0, s - 1.0))
            .fill(Paint::solid(Color::WHITE))
            .stroke(Stroke::new(color, 1.0)),
    );
    let stroke = Stroke::new(color, 1.0);
    let cy = y + s / 2.0;
    let cx = x + s / 2.0;
    g.line(x + 2.0, cy, x + s - 2.0, cy, stroke.clone());
    if !expanded {
        g.line(cx, y + 2.0, cx, y + s - 2.0, stroke);
    }
    g
}

/// Small warning badge in the top-right corner: the query was cut at the
/// row limits ("data reduced").
pub(crate) fn data_reduced_badge(vp: Rect) -> Group {
    let mut g = Group::new().with_class("pbi-data-reduced");
    let (w, h) = (10.0, 9.0);
    let x = vp.right() - w - 2.0;
    let y = vp.y + 2.0;
    let mut p = PathBuilder::new();
    p.move_to(x, y + h)
        .line_to(x + w / 2.0, y)
        .line_to(x + w, y + h)
        .close();
    g.push(PathNode::new(p.build()).fill(Paint::solid(Color::rgb(0xD9, 0xB3, 0x00))));
    g.rect(
        Rect::new(x + w / 2.0 - 0.5, y + 3.0, 1.0, 3.0),
        Some(Paint::solid(Color::WHITE)),
        None,
    );
    g.rect(
        Rect::new(x + w / 2.0 - 0.5, y + 7.0, 1.0, 1.0),
        Some(Paint::solid(Color::WHITE)),
        None,
    );
    g
}

/// The property of the `object` entry whose selector is
/// `{"metadata": query_ref}` (per-column formatting).
pub(crate) fn column_property<'v>(
    objects: &'v serde_json::Value,
    object: &str,
    prop: &str,
    query_ref: &str,
) -> Option<&'v serde_json::Value> {
    pbi_pbir::objects::entries(objects, object)
        .filter(|(_, sel)| {
            sel.and_then(|s| s.get("metadata"))
                .and_then(serde_json::Value::as_str)
                == Some(query_ref)
        })
        .find_map(|(props, _)| props.get(prop))
}
