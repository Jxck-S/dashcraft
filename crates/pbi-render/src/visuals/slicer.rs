//! Slicer: dropdown (collapsed header) and list/tile styles, selection
//! state, search box placeholder; hit ids SlicerItem. (owner:
//! visuals-grids)
//!
//! # Modes
//!
//! [`SlicerMode`] comes from `objects.data.mode`: `Dropdown`; `Basic`
//! (vertical list, or tiles when `general.orientation` is horizontal);
//! `Between`/`Before`/`After`/`Relative` (range: value boxes + slider). Without
//! a mode, numeric and date fields default to a range and other fields to
//! a list; `advancedSlicerVisual` (button slicer) renders tiles.
//!
//! # Layout (page units, measured on the service renders)
//!
//! - Header (`objects.header`: `show`, `text` — default the field's display
//!   name —, `fontColor`, `fontSize`, `fontFamily`, `bold`, `italic`,
//!   `underline`, `background`, `outline`): DIN 10pt `#252423` (the
//!   `label` text class with the service's DIN header face), one line
//!   starting 8px inside the content box,
//!   ellipsized with `…` before a 26px reserve for the eraser icon (so a
//!   ~164px bold header truncates below ~210px of visual width). Controls
//!   start 23px below the content top (10pt).
//! - Dropdown: a box inset 5px left and right, `line height + 10` tall,
//!   `#EAEAEA` border; the summary ("All", the single selected value, or
//!   "Multiple selections") in the `items` style (default `#605E5C`) 6px
//!   inside, and a chevron 6.5px from the right edge.
//! - List: optional search box (`general.selfFilterEnabled`), an optional
//!   "Select all" row (`selection.selectAllCheckboxEnabled`, multi-select
//!   only), then one row per value with a checkbox (multi-select) or radio
//!   button (`selection.singleSelect`, the default) — rows are
//!   `line height + items.padding + 3` tall and scroll with a scrollbar.
//! - Tiles: equal-width buttons; selected tiles are filled.
//! - Range: low / high value boxes and a slider below them.
//!
//! # Selection state
//!
//! From the DataView's [`SlicerData`]: with nothing filtered the dropdown
//! shows "All"; list rows are all checked when the slicer is in inverted
//! ("select all") mode (`isInvertedSelectionMode` or an inverted
//! selection) and unchecked otherwise. Range bounds come from the
//! selection's condition in the [`crate::query::InteractionState`]
//! (`Between` or `>=`/`<=` comparisons), else the data's min and max.
//!
//! # Interactive canvas
//!
//! - List rows, popup rows and tiles are groups with
//!   `HitId::slicer_item(visual, row)` (`row` = category row of the
//!   DataView); the "Select all" row uses [`SELECT_ALL_INDEX`].
//! - The collapsed dropdown box carries `HitId::button(visual)` (toggle the
//!   popup). Render the open state with [`SlicerOptions::dropdown_open`]
//!   and draw [`render_dropdown_popup`] on top of the page (it is not
//!   clipped to the visual); [`layout`] gives the rectangles (dropdown box,
//!   popup, search box, list viewport) and the list's scroll range.
//! - [`SlicerOptions::search`] filters rows (case-insensitive substring)
//!   and [`SlicerOptions::scroll`] scrolls the list / popup.

use pbi_model::{ComparisonKind, Condition, FieldExpr};
use serde::{Deserialize, Serialize};

use crate::dataview::{BLANK_LABEL, CellValue, DataView, SlicerData};
use crate::query::coerce_literal;
use crate::scene::{
    Clip, Color, Group, HitId, Paint, PathBuilder, PathNode, Rect, RectNode, Stroke, TextAnchor,
};
use crate::text::TextStyle;
use crate::theme::{PropLookup, as_bool, as_number, as_text};

use super::VisualCtx;
use super::table::grid::{
    SCROLLBAR_BAND, SCROLLBAR_THUMB, ellipsize, line_metrics, scrollbar_thumb, text_line, thumb,
    width_factor,
};

/// Hit index of the "Select all" row (`u32::MAX`, safe in JSON).
pub const SELECT_ALL_INDEX: usize = 4_294_967_295;
/// Dropdown summary with nothing filtered.
pub const ALL_LABEL: &str = "All";
/// Dropdown summary with several values selected.
pub const MULTIPLE_LABEL: &str = "Multiple selections";
/// Label of the "Select all" row.
pub const SELECT_ALL_LABEL: &str = "Select all";
/// Search box placeholder.
pub const SEARCH_PLACEHOLDER: &str = "Search";
/// Header font family when neither the visual nor the theme sets one.
pub const DEFAULT_HEADER_FAMILY: &str = "DIN";

/// Horizontal inset of controls inside the content box.
const INSET: f64 = 5.0;
/// Header text inset from the content box's left edge.
const HEADER_TEXT_INSET: f64 = 8.0;
/// Header line top relative to the content top.
const HEADER_TOP: f64 = -1.0;
/// Gap between the header line and the controls.
const HEADER_GAP: f64 = 7.0;
/// Space kept free right of the header text (eraser icon).
const ERASER_RESERVE: f64 = 26.0;
/// Vertical padding inside a box (dropdown, search, range values).
const BOX_PAD_Y: f64 = 5.0;
/// Text inset inside a box (border included).
const BOX_TEXT_INSET: f64 = 6.0;
/// Chevron half width / half height.
const CHEVRON_HALF_W: f64 = 6.0;
const CHEVRON_HALF_H: f64 = 3.0;
/// Chevron centre from the box's right edge.
const CHEVRON_RIGHT: f64 = 12.75;
/// Dropdown / search box border.
const BOX_BORDER: Color = Color::rgb(0xEA, 0xEA, 0xEA);
/// Popup border.
const POPUP_BORDER: Color = Color::rgb(0xC8, 0xC6, 0xC4);
/// Placeholder text color.
const PLACEHOLDER_COLOR: Color = Color::rgb(0xA1, 0x9F, 0x9D);
/// Slider track color.
const TRACK_COLOR: Color = Color::rgb(0xC8, 0xC6, 0xC4);
/// Checkbox / radio size.
const CHECK_SIZE: f64 = 12.0;
/// Gap between the checkbox and the label.
const CHECK_GAP: f64 = 8.0;
/// Rows shown in the dropdown popup before it scrolls.
const POPUP_MAX_ROWS: usize = 10;
/// Gap between tiles.
const TILE_GAP: f64 = 8.0;

/// How the slicer presents its values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SlicerMode {
    /// Collapsed box + popup list.
    Dropdown,
    /// Vertical list with checkboxes / radio buttons.
    List,
    /// Horizontal tiles (buttons).
    Tile,
    /// Numeric / date range: value boxes and a slider.
    Range,
}

/// Interactive state of a slicer.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SlicerOptions {
    /// The dropdown popup is open (chevron points up; draw the popup with
    /// [`render_dropdown_popup`]).
    pub dropdown_open: bool,
    /// Search text (filters rows; shown in the search box).
    pub search: String,
    /// Scroll offset of the list / popup rows (page units; clamped).
    pub scroll: f64,
}

/// Rectangles of a slicer (page units) for the interactive canvas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlicerLayout {
    pub mode: SlicerMode,
    /// Header band, when shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<Rect>,
    /// The dropdown box (dropdown mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropdown: Option<Rect>,
    /// The popup below the dropdown box (dropdown mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub popup: Option<Rect>,
    /// Search box (list or popup), when search is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<Rect>,
    /// Scrolling row viewport (list or popup).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<Rect>,
    pub row_height: f64,
    /// Height of all rows (including "Select all").
    pub rows_height: f64,
    pub max_scroll: f64,
    pub single_select: bool,
}

/// One value row.
#[derive(Debug, Clone, PartialEq)]
struct Item {
    /// Hit index (category row, or [`SELECT_ALL_INDEX`]).
    index: usize,
    label: String,
    checked: bool,
}

/// Resolved slicer state.
#[derive(Debug, Clone)]
struct Slicer {
    content: Rect,
    visual: String,
    mode: SlicerMode,
    header: Option<HeaderSpec>,
    items_style: TextStyle,
    items_bg: Option<Color>,
    single_select: bool,
    select_all: bool,
    search_enabled: bool,
    item_padding: f64,
    rounded: f64,
    tile_columns: Option<usize>,
    /// Value rows (unfiltered).
    items: Vec<Item>,
    all_checked: bool,
    summary: String,
    range: Option<RangeSpec>,
}

#[derive(Debug, Clone)]
struct HeaderSpec {
    text: String,
    style: TextStyle,
    background: Option<Color>,
    /// Bottom line.
    outline: Option<(Color, f64)>,
}

#[derive(Debug, Clone)]
struct RangeSpec {
    low_text: String,
    high_text: String,
    /// Handle positions in `[0, 1]`.
    low: f64,
    high: f64,
    show_low: bool,
    show_high: bool,
}

/// Renders the visual content (see module docs).
pub fn render(ctx: &VisualCtx<'_>) -> Option<Group> {
    render_with(ctx, &SlicerOptions::default())
}

/// Renders the slicer with an interactive state.
pub fn render_with(ctx: &VisualCtx<'_>, opts: &SlicerOptions) -> Option<Group> {
    let c = ctx.frame.content;
    if c.width <= 1.0 || c.height <= 1.0 {
        return None;
    }
    let s = Slicer::resolve(ctx);
    let mut g = Group::new().with_class("pbi-slicer");
    let control_top = s.draw_header(ctx, &mut g);
    match s.mode {
        SlicerMode::Dropdown => s.draw_dropdown(ctx, &mut g, control_top, opts.dropdown_open),
        SlicerMode::List => {
            let area = Rect::new(
                c.x + INSET,
                control_top,
                (c.width - 2.0 * INSET).max(0.0),
                (c.bottom() - control_top).max(0.0),
            );
            g.push(s.list(ctx, area, opts));
        }
        SlicerMode::Tile => s.draw_tiles(ctx, &mut g, control_top),
        SlicerMode::Range => s.draw_range(ctx, &mut g, control_top),
    }
    Some(g)
}

/// The dropdown popup (page units, below the dropdown box; not clipped to
/// the visual). Empty for other modes.
pub fn render_dropdown_popup(ctx: &VisualCtx<'_>, opts: &SlicerOptions) -> Group {
    let s = Slicer::resolve(ctx);
    let mut g = Group::new().with_class("pbi-slicer-popup");
    if s.mode != SlicerMode::Dropdown {
        return g;
    }
    let control_top = s.control_top(ctx);
    let popup = s.popup_rect(ctx, control_top, opts);
    g.push(RectNode::new(popup.translate(1.0, 2.0)).fill(Paint::new(Color::BLACK, 0.08)));
    g.push(
        RectNode::new(popup)
            .fill(Paint::solid(s.items_bg.unwrap_or(Color::WHITE)))
            .stroke(Stroke::new(POPUP_BORDER, 1.0)),
    );
    g.push(s.list(ctx, popup.inset_uniform(1.0), opts));
    g
}

/// Rectangles and scroll range of the slicer (see module docs).
pub fn layout(ctx: &VisualCtx<'_>, opts: &SlicerOptions) -> SlicerLayout {
    let s = Slicer::resolve(ctx);
    let c = ctx.frame.content;
    let control_top = s.control_top(ctx);
    let header = s.header.as_ref().map(|_| s.header_rect(ctx));
    let (dropdown, popup, list_area) = match s.mode {
        SlicerMode::Dropdown => {
            let popup = s.popup_rect(ctx, control_top, opts);
            (
                Some(s.box_rect(ctx, control_top)),
                Some(popup),
                Some(popup.inset_uniform(1.0)),
            )
        }
        SlicerMode::List => (
            None,
            None,
            Some(Rect::new(
                c.x + INSET,
                control_top,
                (c.width - 2.0 * INSET).max(0.0),
                (c.bottom() - control_top).max(0.0),
            )),
        ),
        SlicerMode::Tile | SlicerMode::Range => (None, None, None),
    };
    let geometry = list_area.map(|a| s.list_geometry(ctx, a, opts));
    SlicerLayout {
        mode: s.mode,
        header,
        dropdown,
        popup,
        search: geometry.as_ref().and_then(|g| g.search),
        rows: geometry.as_ref().map(|g| g.rows),
        row_height: s.row_height(),
        rows_height: geometry.as_ref().map_or(0.0, |g| g.rows_height),
        max_scroll: geometry.as_ref().map_or(0.0, |g| g.max_scroll),
        single_select: s.single_select,
    }
}

/// Geometry of a list (in an area).
#[derive(Debug, Clone)]
struct ListGeometry {
    search: Option<Rect>,
    rows: Rect,
    rows_height: f64,
    max_scroll: f64,
    scroll: f64,
    scrollbar: bool,
    items: Vec<Item>,
}

fn dataview<'a>(ctx: &VisualCtx<'a>) -> Option<&'a DataView> {
    match ctx.data {
        Some(Ok(dv)) => Some(dv),
        _ => None,
    }
}

/// Literal / theme value of `objects.<object>.<prop>` that is either a
/// number or text (e.g. `orientation`).
fn number_or_text(o: &PropLookup<'_>, object: &str, prop: &str) -> Option<String> {
    let v = o.value(object, prop)?;
    as_number(&v)
        .map(|n| n.to_string())
        .or_else(|| as_text(&v))
        .or_else(|| as_bool(&v).map(|b| b.to_string()))
}

impl Slicer {
    fn resolve(ctx: &VisualCtx<'_>) -> Self {
        let props = ctx.theme.visual(ctx.visual);
        let o = props.objects();
        let dv = dataview(ctx);
        let categorical = dv.and_then(|d| d.categorical.as_ref());
        let first_meta = categorical
            .and_then(|c| c.categories.first())
            .and_then(|col| dv.and_then(|d| d.meta(col.meta_idx)));
        let field_name = first_meta
            .map(|m| m.display_name.clone())
            .or_else(|| {
                ctx.visual
                    .projections_for("Values")
                    .next()
                    .map(|p| p.label().to_owned())
            })
            .unwrap_or_default();
        let range_type = first_meta
            .and_then(|m| m.data_type.as_ref())
            .is_some_and(|t| t.is_numeric() || *t == pbi_model::DataType::DateTime);
        let horizontal = number_or_text(&o, "general", "orientation")
            .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("horizontal"));
        let mode_text = o.text("data", "mode").map(|m| m.to_ascii_lowercase());
        let mode = match mode_text.as_deref() {
            Some("dropdown") => SlicerMode::Dropdown,
            Some("between" | "before" | "after" | "relative") => SlicerMode::Range,
            Some(_) if horizontal => SlicerMode::Tile,
            Some(_) => SlicerMode::List,
            None if ctx.visual.visual_type == "advancedSlicerVisual" || horizontal => {
                SlicerMode::Tile
            }
            None if range_type => SlicerMode::Range,
            None => SlicerMode::List,
        };

        let header = o.bool("header", "show").unwrap_or(true).then(|| {
            let mut style = o.text_style("header", "label");
            if o.text("header", "fontFamily").is_none() {
                style.family = DEFAULT_HEADER_FAMILY.to_owned();
            }
            if ctx.visual.visual_type == "advancedSlicerVisual"
                && o.bool("header", "bold").is_none()
            {
                style.weight = 600;
            }
            let outline_on = o
                .text("header", "outline")
                .map(|s| !s.eq_ignore_ascii_case("none"))
                .or_else(|| o.number("header", "outlineStyle").map(|n| n != 0.0))
                .unwrap_or(false);
            let outline = outline_on.then(|| {
                (
                    o.color("header", "outlineColor")
                        .or_else(|| o.color("general", "outlineColor"))
                        .unwrap_or(Color::rgb(0xB3, 0xB0, 0xAD)),
                    o.number("general", "outlineWeight")
                        .filter(|w| *w > 0.0)
                        .unwrap_or(1.0)
                        .min(10.0),
                )
            });
            HeaderSpec {
                text: o
                    .text("header", "text")
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| field_name.clone()),
                style,
                background: o.color("header", "background"),
                outline,
            }
        });

        let mut items_style = o.text_style("items", "label");
        if o.color("items", "fontColor").is_none() {
            items_style.color = ctx
                .theme
                .named_color("foregroundNeutralSecondary")
                .unwrap_or(Color::rgb(0x60, 0x5E, 0x5C));
        }
        if ctx.visual.visual_type == "advancedSlicerVisual" {
            let value = o.text_style("value", "label");
            items_style.family = value.family;
            if o.number("value", "fontSize").is_some() {
                items_style.size_pt = value.size_pt;
            }
        }

        let single_select = o.bool("selection", "singleSelect").unwrap_or(true);
        let inverted_mode = o
            .bool("data", "isInvertedSelectionMode")
            .or_else(|| o.bool("general", "isInvertedSelectionMode"))
            .unwrap_or(false);
        let slicer = dv.and_then(|d| d.slicer.clone()).unwrap_or(SlicerData {
            all_selected: true,
            ..SlicerData::default()
        });
        let all_checked = slicer.all_selected && (inverted_mode || slicer.inverted);
        let items: Vec<Item> = (0..categorical.map_or(0, |c| c.len()))
            .map(|i| Item {
                index: i,
                label: categorical.map(|c| row_label(c, i)).unwrap_or_default(),
                checked: if slicer.all_selected {
                    all_checked
                } else if slicer.inverted {
                    !slicer.selected.contains(&i)
                } else {
                    slicer.selected.contains(&i)
                },
            })
            .collect();
        let summary = if slicer.all_selected {
            ALL_LABEL.to_owned()
        } else if !slicer.inverted && slicer.values.len() == 1 {
            slicer
                .values
                .first()
                .map(|t| tuple_label(t))
                .unwrap_or_default()
        } else {
            MULTIPLE_LABEL.to_owned()
        };
        let range = (mode == SlicerMode::Range).then(|| {
            range_spec(
                ctx,
                dv,
                mode_text.as_deref().unwrap_or("between"),
                first_meta.and_then(|m| m.data_type.as_ref()),
            )
        });
        Self {
            content: ctx.frame.content,
            visual: ctx.visual.name.clone(),
            mode,
            header,
            items_bg: o.color("items", "background"),
            items_style,
            single_select,
            select_all: o
                .bool("selection", "selectAllCheckboxEnabled")
                .unwrap_or(false),
            search_enabled: o.bool("general", "selfFilterEnabled").unwrap_or(false),
            item_padding: o
                .number("items", "padding")
                .filter(|p| *p >= 0.0)
                .unwrap_or(4.0)
                .min(40.0),
            rounded: if ctx.visual.visual_type == "advancedSlicerVisual" {
                o.number("shapeCustomRectangle", "rectangleRoundedCurve")
                    .filter(|r| *r >= 0.0)
                    .unwrap_or(4.0)
                    .min(40.0)
            } else {
                0.0
            },
            tile_columns: (ctx.visual.visual_type == "advancedSlicerVisual")
                .then(|| o.number("layout", "maxTiles"))
                .flatten()
                .filter(|n| *n >= 1.0)
                .map(|n| n.min(100.0) as usize),
            items,
            all_checked,
            summary,
            range,
        }
    }

    fn header_line_height(&self) -> f64 {
        self.header
            .as_ref()
            .map_or(0.0, |h| line_metrics(&h.style).0)
    }

    fn header_rect(&self, _ctx: &VisualCtx<'_>) -> Rect {
        let c = self.content;
        Rect::new(
            c.x,
            c.y,
            c.width,
            (self.header_line_height() + 4.0).min(c.height),
        )
    }

    /// Top of the controls below the header.
    fn control_top(&self, _ctx: &VisualCtx<'_>) -> f64 {
        match &self.header {
            Some(_) => self.content.y + HEADER_TOP + self.header_line_height() + HEADER_GAP,
            None => self.content.y,
        }
    }

    fn box_height(&self) -> f64 {
        line_metrics(&self.items_style).0 + 2.0 * BOX_PAD_Y
    }

    fn row_height(&self) -> f64 {
        line_metrics(&self.items_style).0 + self.item_padding + 3.0
    }

    fn box_rect(&self, _ctx: &VisualCtx<'_>, control_top: f64) -> Rect {
        let c = self.content;
        Rect::new(
            c.x + INSET,
            control_top,
            (c.width - 2.0 * INSET).max(0.0),
            self.box_height(),
        )
    }

    /// Draws the header and returns the top of the controls.
    fn draw_header(&self, ctx: &VisualCtx<'_>, g: &mut Group) -> f64 {
        let Some(h) = &self.header else {
            return self.content.y;
        };
        let c = self.content;
        let rect = self.header_rect(ctx);
        if let Some(bg) = h.background {
            g.push(RectNode::new(rect).fill(Paint::solid(bg)));
        }
        if let Some((color, w)) = h.outline {
            g.push(
                RectNode::new(Rect::new(rect.x, rect.bottom() - w, rect.width, w))
                    .fill(Paint::solid(color)),
            );
        }
        let (_, ascent) = line_metrics(&h.style);
        let max_w = (c.width - HEADER_TEXT_INSET - ERASER_RESERVE).max(0.0);
        let text = ellipsize(ctx.fonts, &h.text, &h.style, max_w);
        if !text.is_empty() {
            g.push(text_line(
                c.x + HEADER_TEXT_INSET,
                c.y + HEADER_TOP + ascent,
                text,
                &h.style,
                TextAnchor::Start,
            ));
        }
        self.control_top(ctx)
    }

    fn draw_dropdown(&self, ctx: &VisualCtx<'_>, g: &mut Group, control_top: f64, open: bool) {
        let b = self.box_rect(ctx, control_top);
        if b.width <= 0.0 {
            return;
        }
        let mut bg = Group::new()
            .with_id(HitId::button(&self.visual))
            .with_class("pbi-slicer-dropdown");
        bg.push(
            RectNode::new(b.inset_uniform(0.5))
                .fill(Paint::solid(self.items_bg.unwrap_or(Color::WHITE)))
                .stroke(Stroke::new(BOX_BORDER, 1.0)),
        );
        let (lh, ascent) = line_metrics(&self.items_style);
        let text_w = (b.width - 2.0 * BOX_TEXT_INSET - CHEVRON_RIGHT - CHEVRON_HALF_W).max(0.0);
        let text = ellipsize(ctx.fonts, &self.summary, &self.items_style, text_w);
        if !text.is_empty() {
            bg.push(text_line(
                b.x + BOX_TEXT_INSET,
                b.y + (b.height - lh) / 2.0 + ascent,
                text,
                &self.items_style,
                TextAnchor::Start,
            ));
        }
        bg.push(chevron(
            b.right() - CHEVRON_RIGHT,
            b.y + b.height / 2.0,
            open,
            self.items_style.color,
        ));
        g.push(bg);
    }

    fn popup_rect(&self, ctx: &VisualCtx<'_>, control_top: f64, opts: &SlicerOptions) -> Rect {
        let b = self.box_rect(ctx, control_top);
        let rows = self.visible_items(opts).len() + usize::from(self.shows_select_all(opts));
        let rows_h = self.row_height() * rows.clamp(1, POPUP_MAX_ROWS) as f64;
        let search_h = if self.search_enabled {
            self.box_height() + 4.0
        } else {
            0.0
        };
        Rect::new(
            b.x,
            b.bottom() + 2.0,
            b.width,
            rows_h + search_h + 2.0 + 8.0,
        )
    }

    fn shows_select_all(&self, opts: &SlicerOptions) -> bool {
        self.select_all && !self.single_select && opts.search.trim().is_empty()
    }

    /// Rows matching the search text.
    fn visible_items(&self, opts: &SlicerOptions) -> Vec<Item> {
        let needle = opts.search.trim().to_lowercase();
        self.items
            .iter()
            .filter(|i| needle.is_empty() || i.label.to_lowercase().contains(&needle))
            .cloned()
            .collect()
    }

    fn list_geometry(&self, ctx: &VisualCtx<'_>, area: Rect, opts: &SlicerOptions) -> ListGeometry {
        let _ = ctx;
        let mut top = area.y
            + if self.mode == SlicerMode::Dropdown {
                4.0
            } else {
                0.0
            };
        let search = self.search_enabled.then(|| {
            let r = Rect::new(
                area.x + 4.0,
                top,
                (area.width - 8.0).max(0.0),
                self.box_height(),
            );
            top += r.height + 4.0;
            r
        });
        let mut items = Vec::new();
        if self.shows_select_all(opts) {
            items.push(Item {
                index: SELECT_ALL_INDEX,
                label: SELECT_ALL_LABEL.to_owned(),
                checked: self.all_checked,
            });
        }
        items.extend(self.visible_items(opts));
        let rows = Rect::new(
            area.x,
            top,
            area.width,
            (area.bottom()
                - top
                - if self.mode == SlicerMode::Dropdown {
                    4.0
                } else {
                    0.0
                })
            .max(0.0),
        );
        let rows_height = items.len() as f64 * self.row_height();
        let max_scroll = (rows_height - rows.height).max(0.0);
        let scroll = if opts.scroll.is_finite() {
            opts.scroll.clamp(0.0, max_scroll)
        } else {
            0.0
        };
        ListGeometry {
            search,
            rows,
            rows_height,
            max_scroll,
            scroll,
            scrollbar: max_scroll > 0.0,
            items,
        }
    }

    /// A list of checkbox / radio rows in `area` (list mode and popup).
    fn list(&self, ctx: &VisualCtx<'_>, area: Rect, opts: &SlicerOptions) -> Group {
        let geo = self.list_geometry(ctx, area, opts);
        let mut g = Group::new().with_class("pbi-slicer-list");
        if let Some(sr) = geo.search {
            g.push(self.search_box(ctx, sr, &opts.search));
        }
        let row_h = self.row_height();
        let (lh, ascent) = line_metrics(&self.items_style);
        let mut rows = Group::new().with_clip(Clip::rect(geo.rows));
        let text_right = geo.rows.right() - if geo.scrollbar { SCROLLBAR_BAND } else { 0.0 };
        let first = (geo.scroll / row_h).floor().max(0.0) as usize;
        for (k, item) in geo.items.iter().enumerate().skip(first) {
            let y = geo.rows.y + k as f64 * row_h - geo.scroll;
            if y > geo.rows.bottom() {
                break;
            }
            let mut ig = Group::new().with_id(HitId::slicer_item(&self.visual, item.index));
            let row_rect = Rect::new(geo.rows.x, y, (text_right - geo.rows.x).max(0.0), row_h);
            ig.push(RectNode::new(row_rect).fill(Paint::new(Color::WHITE, 0.0)));
            let cx = geo.rows.x + 2.0;
            let cy = y + (row_h - CHECK_SIZE) / 2.0;
            let color = self.items_style.color;
            if self.single_select && item.index != SELECT_ALL_INDEX {
                ig.push(radio(cx, cy, item.checked, color));
            } else {
                ig.push(checkbox(cx, cy, item.checked, color));
            }
            let tx = cx + CHECK_SIZE + CHECK_GAP;
            let text = ellipsize(
                ctx.fonts,
                &item.label,
                &self.items_style,
                (text_right - tx - 2.0).max(0.0),
            );
            if !text.is_empty() {
                ig.push(text_line(
                    tx,
                    y + (row_h - lh) / 2.0 + ascent,
                    text,
                    &self.items_style,
                    TextAnchor::Start,
                ));
            }
            rows.push(ig);
        }
        g.push(rows);
        if geo.scrollbar {
            let inset = (SCROLLBAR_BAND - SCROLLBAR_THUMB) / 2.0;
            let track = (geo.rows.height - 2.0).max(0.0);
            let ratio = if geo.rows_height > 0.0 {
                geo.rows.height / geo.rows_height
            } else {
                1.0
            };
            let (pos, len) = thumb(track, ratio, geo.scroll, geo.max_scroll);
            g.push(scrollbar_thumb(Rect::new(
                geo.rows.right() - SCROLLBAR_BAND + inset,
                geo.rows.y + 1.0 + pos,
                SCROLLBAR_THUMB,
                len,
            )));
        }
        g
    }

    fn search_box(&self, ctx: &VisualCtx<'_>, r: Rect, search: &str) -> Group {
        let mut g = Group::new().with_class("pbi-slicer-search");
        g.push(
            RectNode::new(r.inset_uniform(0.5))
                .fill(Paint::solid(Color::WHITE))
                .stroke(Stroke::new(BOX_BORDER, 1.0)),
        );
        // Magnifier: a circle and a handle.
        let (cx, cy) = (r.x + BOX_TEXT_INSET + 4.0, r.y + r.height / 2.0 - 1.0);
        let stroke = Stroke::new(self.items_style.color, 1.0);
        let mut p = PathBuilder::new();
        p.move_to(cx + 4.0, cy)
            .arc_to(4.0, 4.0, 0.0, true, true, cx - 4.0, cy)
            .arc_to(4.0, 4.0, 0.0, true, true, cx + 4.0, cy)
            .close();
        g.push(PathNode::new(p.build()).stroke(stroke.clone()));
        g.line(cx + 2.9, cy + 2.9, cx + 6.0, cy + 6.0, stroke);
        let (lh, ascent) = line_metrics(&self.items_style);
        let (text, color) = if search.is_empty() {
            (SEARCH_PLACEHOLDER, PLACEHOLDER_COLOR)
        } else {
            (search, self.items_style.color)
        };
        let tx = cx + 12.0;
        let shown = ellipsize(
            ctx.fonts,
            text,
            &self.items_style,
            (r.right() - tx - BOX_TEXT_INSET).max(0.0),
        );
        if !shown.is_empty() {
            g.push(text_line(
                tx,
                r.y + (r.height - lh) / 2.0 + ascent,
                shown,
                &self.items_style.clone().with_color(color),
                TextAnchor::Start,
            ));
        }
        g
    }

    fn draw_tiles(&self, ctx: &VisualCtx<'_>, g: &mut Group, control_top: f64) {
        let c = self.content;
        let area = Rect::new(
            c.x + INSET,
            control_top,
            (c.width - 2.0 * INSET).max(0.0),
            (c.bottom() - control_top).max(0.0),
        );
        let n = self.items.len();
        if n == 0 || area.width <= 0.0 || area.height <= 0.0 {
            return;
        }
        let (lh, ascent) = line_metrics(&self.items_style);
        let factor = width_factor(&self.items_style);
        let min_w = self
            .items
            .iter()
            .map(|i| ctx.fonts.text_width(&i.label, &self.items_style) * factor)
            .fold(0.0, f64::max)
            .min(area.width)
            + 2.0 * BOX_TEXT_INSET;
        let fit = ((area.width + TILE_GAP) / (min_w + TILE_GAP))
            .floor()
            .max(1.0) as usize;
        let cols = self.tile_columns.unwrap_or(n).min(fit.max(1)).min(n).max(1);
        let tile_w = ((area.width - TILE_GAP * (cols as f64 - 1.0)) / cols as f64).max(1.0);
        let rows = n.div_ceil(cols);
        let tile_h = if self.tile_columns.is_some() {
            lh + 2.0 * (BOX_PAD_Y + 2.0)
        } else {
            ((area.height - TILE_GAP * (rows as f64 - 1.0)) / rows as f64)
                .clamp(lh + 2.0 * BOX_PAD_Y, 3.0 * lh)
        };
        let mut tiles = Group::new().with_clip(Clip::rect(area));
        for (k, item) in self.items.iter().enumerate() {
            let (col, row) = (k % cols, k / cols);
            let x = area.x + col as f64 * (tile_w + TILE_GAP);
            let y = area.y + row as f64 * (tile_h + TILE_GAP);
            if y > area.bottom() {
                break;
            }
            let rect = Rect::new(x, y, tile_w, tile_h);
            let mut tg = Group::new().with_id(HitId::slicer_item(&self.visual, item.index));
            let (fill, text_color) = if item.checked {
                (self.items_style.color, Color::WHITE)
            } else {
                (
                    self.items_bg.unwrap_or(Color::WHITE),
                    self.items_style.color,
                )
            };
            tg.push(
                RectNode::new(rect.inset_uniform(0.5))
                    .radius(self.rounded)
                    .fill(Paint::solid(fill))
                    .stroke(Stroke::new(BOX_BORDER, 1.0)),
            );
            let text = ellipsize(
                ctx.fonts,
                &item.label,
                &self.items_style,
                (tile_w - 2.0 * BOX_TEXT_INSET).max(0.0),
            );
            if !text.is_empty() {
                tg.push(text_line(
                    x + tile_w / 2.0,
                    y + (tile_h - lh) / 2.0 + ascent,
                    text,
                    &self.items_style.clone().with_color(text_color),
                    TextAnchor::Middle,
                ));
            }
            tiles.push(tg);
        }
        g.push(tiles);
    }

    fn draw_range(&self, ctx: &VisualCtx<'_>, g: &mut Group, control_top: f64) {
        let Some(r) = &self.range else {
            return;
        };
        let c = self.content;
        let width = (c.width - 2.0 * INSET).max(0.0);
        let x = c.x + INSET;
        let h = self.box_height();
        let (lh, ascent) = line_metrics(&self.items_style);
        let boxes: Vec<(&str, Rect)> = match (r.show_low, r.show_high) {
            (true, true) => {
                let w = ((width - TILE_GAP) / 2.0).max(0.0);
                vec![
                    (r.low_text.as_str(), Rect::new(x, control_top, w, h)),
                    (
                        r.high_text.as_str(),
                        Rect::new(x + w + TILE_GAP, control_top, w, h),
                    ),
                ]
            }
            (true, false) => vec![(r.low_text.as_str(), Rect::new(x, control_top, width, h))],
            _ => vec![(r.high_text.as_str(), Rect::new(x, control_top, width, h))],
        };
        for (text, b) in boxes {
            g.push(
                RectNode::new(b.inset_uniform(0.5))
                    .fill(Paint::solid(self.items_bg.unwrap_or(Color::WHITE)))
                    .stroke(Stroke::new(BOX_BORDER, 1.0)),
            );
            let shown = ellipsize(
                ctx.fonts,
                text,
                &self.items_style,
                (b.width - 2.0 * BOX_TEXT_INSET).max(0.0),
            );
            if !shown.is_empty() {
                g.push(text_line(
                    b.x + BOX_TEXT_INSET,
                    b.y + (b.height - lh) / 2.0 + ascent,
                    shown,
                    &self.items_style,
                    TextAnchor::Start,
                ));
            }
        }
        // Slider.
        let radius = 6.0;
        let ty = control_top + h + 14.0;
        let (x0, x1) = (x + radius, x + width - radius);
        if x1 <= x0 {
            return;
        }
        let color = self.items_style.color;
        g.push(
            RectNode::new(Rect::new(x0, ty - 1.0, x1 - x0, 2.0)).fill(Paint::solid(TRACK_COLOR)),
        );
        let lx = x0 + (x1 - x0) * r.low;
        let hx = x0 + (x1 - x0) * r.high;
        g.push(
            RectNode::new(Rect::new(lx, ty - 1.0, (hx - lx).max(0.0), 2.0))
                .fill(Paint::solid(color)),
        );
        let handles: Vec<f64> = [(r.show_low, lx), (r.show_high, hx)]
            .into_iter()
            .filter(|(on, _)| *on)
            .map(|(_, x)| x)
            .collect();
        for hx in handles {
            g.push(
                PathNode::new(circle(hx, ty, radius))
                    .fill(Paint::solid(Color::WHITE))
                    .stroke(Stroke::new(color, 1.0)),
            );
        }
    }
}

/// Label of category row `i` (several fields joined with ` / `).
fn row_label(c: &crate::dataview::Categorical, i: usize) -> String {
    let parts: Vec<String> = c
        .categories
        .iter()
        .filter_map(|col| col.values.get(i))
        .map(cell_label)
        .collect();
    parts.join(" / ")
}

fn cell_label(cell: &CellValue) -> String {
    if cell.formatted.is_empty() && cell.is_blank() {
        BLANK_LABEL.to_owned()
    } else {
        cell.formatted.clone()
    }
}

fn tuple_label(t: &[CellValue]) -> String {
    t.iter().map(cell_label).collect::<Vec<_>>().join(" / ")
}

/// Range bounds and handle positions.
fn range_spec(
    ctx: &VisualCtx<'_>,
    dv: Option<&DataView>,
    mode: &str,
    data_type: Option<&pbi_model::DataType>,
) -> RangeSpec {
    let cells: Vec<&CellValue> = dv
        .and_then(|d| d.categorical.as_ref())
        .and_then(|c| c.categories.first())
        .map(|col| col.values.iter().filter(|v| !v.is_blank()).collect())
        .unwrap_or_default();
    let numeric: Vec<(f64, &CellValue)> = cells
        .iter()
        .filter_map(|c| c.raw.as_f64().map(|n| (n, *c)))
        .collect();
    let min = numeric
        .iter()
        .map(|(n, _)| *n)
        .fold(f64::INFINITY, f64::min);
    let max = numeric
        .iter()
        .map(|(n, _)| *n)
        .fold(f64::NEG_INFINITY, f64::max);
    let condition = ctx
        .state
        .slicer_selections
        .get(&ctx.visual.name)
        .and_then(|s| s.condition.as_ref());
    let (mut lo, mut hi) = condition.map_or((None, None), |c| condition_bounds(c, data_type));
    if !min.is_finite() || !max.is_finite() {
        return RangeSpec {
            low_text: String::new(),
            high_text: String::new(),
            low: 0.0,
            high: 1.0,
            show_low: mode != "before",
            show_high: mode != "after",
        };
    }
    lo = lo.map(|v| v.clamp(min, max));
    hi = hi.map(|v| v.clamp(min, max));
    let lo_v = lo.unwrap_or(min);
    let hi_v = hi.unwrap_or(max);
    let text_of = |v: f64| {
        numeric
            .iter()
            .min_by(|a, b| (a.0 - v).abs().total_cmp(&(b.0 - v).abs()))
            .filter(|(n, _)| (n - v).abs() < 1e-9)
            .map(|(_, c)| c.formatted.clone())
            .unwrap_or_else(|| plain_number(v))
    };
    let span = max - min;
    let pos = |v: f64| if span > 0.0 { (v - min) / span } else { 0.0 };
    RangeSpec {
        low_text: text_of(lo_v),
        high_text: text_of(hi_v),
        low: pos(lo_v),
        high: if span > 0.0 { pos(hi_v) } else { 1.0 },
        show_low: mode != "before",
        show_high: mode != "after",
    }
}

/// `1234.5` → `1234.5`, `3.0` → `3`.
fn plain_number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{v:.0}")
    } else {
        let s = format!("{v:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

/// Lower / upper bounds of a range condition (`Between`, `>=`/`>`,
/// `<=`/`<`, combined with `And`).
fn condition_bounds(
    c: &Condition,
    data_type: Option<&pbi_model::DataType>,
) -> (Option<f64>, Option<f64>) {
    let number = |e: &FieldExpr| match e {
        FieldExpr::Literal(lit) => lit
            .value()
            .ok()
            .and_then(|lv| coerce_literal(&lv, data_type).as_f64().or(lv.as_f64())),
        _ => None,
    };
    match c {
        Condition::Between { lower, upper, .. } => (number(lower), number(upper)),
        Condition::Comparison { kind, right, .. } => match kind {
            ComparisonKind::GreaterThan | ComparisonKind::GreaterThanOrEqual => {
                (number(right), None)
            }
            ComparisonKind::LessThan | ComparisonKind::LessThanOrEqual => (None, number(right)),
            ComparisonKind::Equal => (number(right), number(right)),
        },
        Condition::And(a, b) => {
            let (a_lo, a_hi) = condition_bounds(a, data_type);
            let (b_lo, b_hi) = condition_bounds(b, data_type);
            (a_lo.or(b_lo), a_hi.or(b_hi))
        }
        _ => (None, None),
    }
}

/// The dropdown chevron centred at `(cx, cy)` (`∨`, or `∧` when open).
fn chevron(cx: f64, cy: f64, open: bool, color: Color) -> PathNode {
    let (w, h) = (
        CHEVRON_HALF_W,
        if open {
            -CHEVRON_HALF_H
        } else {
            CHEVRON_HALF_H
        },
    );
    let mut p = PathBuilder::new();
    p.move_to(cx - w, cy - h)
        .line_to(cx, cy + h)
        .line_to(cx + w, cy - h);
    PathNode::new(p.build()).stroke(Stroke::new(color, 1.0))
}

fn circle(cx: f64, cy: f64, r: f64) -> crate::scene::PathData {
    let mut p = PathBuilder::new();
    p.move_to(cx + r, cy)
        .arc_to(r, r, 0.0, true, true, cx - r, cy)
        .arc_to(r, r, 0.0, true, true, cx + r, cy)
        .close();
    p.build()
}

/// A checkbox with its top-left at `(x, y)`.
fn checkbox(x: f64, y: f64, checked: bool, color: Color) -> Group {
    let mut g = Group::new().with_class("pbi-checkbox");
    let r = Rect::new(x + 0.5, y + 0.5, CHECK_SIZE - 1.0, CHECK_SIZE - 1.0);
    if checked {
        g.push(
            RectNode::new(r)
                .fill(Paint::solid(color))
                .stroke(Stroke::new(color, 1.0)),
        );
        let mut p = PathBuilder::new();
        p.move_to(x + 2.5, y + 6.0)
            .line_to(x + 5.0, y + 8.5)
            .line_to(x + 9.5, y + 3.5);
        g.push(PathNode::new(p.build()).stroke(Stroke::new(Color::WHITE, 1.5)));
    } else {
        g.push(
            RectNode::new(r)
                .fill(Paint::solid(Color::WHITE))
                .stroke(Stroke::new(color, 1.0)),
        );
    }
    g
}

/// A radio button with its top-left at `(x, y)`.
fn radio(x: f64, y: f64, checked: bool, color: Color) -> Group {
    let mut g = Group::new().with_class("pbi-radio");
    let r = CHECK_SIZE / 2.0;
    let (cx, cy) = (x + r, y + r);
    g.push(
        PathNode::new(circle(cx, cy, r - 0.5))
            .fill(Paint::solid(Color::WHITE))
            .stroke(Stroke::new(color, 1.0)),
    );
    if checked {
        g.push(PathNode::new(circle(cx, cy, r - 3.0)).fill(Paint::solid(color)));
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_bounds_from_conditions() {
        use pbi_model::Literal;
        let lit = |v: &str| FieldExpr::Literal(Literal::new(v));
        let col = FieldExpr::Literal(Literal::null());
        let c = Condition::And(
            Box::new(Condition::Comparison {
                kind: ComparisonKind::GreaterThanOrEqual,
                left: col.clone(),
                right: lit("10L"),
            }),
            Box::new(Condition::Comparison {
                kind: ComparisonKind::LessThan,
                left: col.clone(),
                right: lit("20L"),
            }),
        );
        assert_eq!(condition_bounds(&c, None), (Some(10.0), Some(20.0)));
        let b = Condition::Between {
            expr: col,
            lower: lit("1.5D"),
            upper: lit("2.5D"),
        };
        assert_eq!(condition_bounds(&b, None), (Some(1.5), Some(2.5)));
        assert_eq!(plain_number(3.0), "3");
        assert_eq!(plain_number(2.25), "2.25");
    }
}
