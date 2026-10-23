//! Cards: `cardVisual` (new card), legacy `card`, `multiRowCard` and a
//! basic `kpi`. (owner: visuals-basics)
//!
//! Values are the DataView's formatted text ([`CellValue::formatted`]);
//! BLANK shows `(Blank)`. Query errors never reach these renderers (see
//! [`super::common::error_content`]).
//!
//! # `cardVisual` (new card)
//!
//! Each value field is a *tile* (several values, or a category field, make
//! a grid of up to `layout.maxTiles` (theme: 3) tiles per row, `cellPadding`
//! apart; with several rows a tile is at least as tall as its content and
//! the rows that do not fit are clipped). Measured against Power BI renders
//! (a 153×100 card: value baseline 51px and label baseline 76px below the
//! top):
//!
//! - tiles sit inside `layout.paddingUniform` (theme 12) of the content box;
//!   text sits inside the tile's `padding.paddingUniform` (theme 12, often
//!   set to 6);
//! - value (`value.*` over the `callout` text class; theme: Segoe UI
//!   Semibold) and label (`label.*` over `label`; theme color
//!   `foregroundNeutralSecondary`, `position` `belowValue`/`aboveValue`)
//!   form a block `spacing.verticalSpacing` (theme 2) apart, using the
//!   families' own line boxes (Calibri 1.22em), centred vertically and
//!   aligned per `layout.alignment` (default left);
//! - **font sizes are honoured** (`value.fontSize`, e.g. 24 or 16);
//! - **label clipping**: when the block is taller than the tile it is
//!   top-aligned and clipped to the tile, so the label disappears first
//!   (when not even the value fits, the value line is centred) —
//!   with a tile padding of 6 that happens below ~97px of card height
//!   (the "labels clip below ~100px" quirk); values and labels clip at the
//!   tile edge without an ellipsis;
//! - **inner tile border**: the tile has its own border, drawn when
//!   `border.show` (color `border.color`, default foreground) or
//!   `outline.show` (default **on**, color `outline.lineColor`, theme
//!   `backgroundLight`) — reports often turn both off and use the container
//!   border instead (`visualContainerObjects.border`, padding 0); tile shape
//!   `shapeCustomRectangle.tileShape` (`rectangleRoundedByPixel` →
//!   `rectangleRoundedCurve` px corners);
//! - tile fill `fillCustom` (when shown), else `cardCalloutArea`
//!   `backgroundFillColor` (when shown); accent bar `accentBar` (when
//!   shown);
//! - reference labels (fields in a `*reference*` role) list below the
//!   callout as `title  value` lines after an optional `divider`.
//!
//! # Legacy `card`
//!
//! Value (`labels.*` over `callout`) centred, shrunk to fit the width,
//! with the category label (`categoryLabels.*` over `largeLightLabel`: 12pt
//! `#605E5C`) below when `categoryLabels.show` (default true).
//!
//! # `multiRowCard`
//!
//! One block per row: a leading group column as the card title
//! (`cardTitle.*` over `largeLabel`), then each field as value
//! (`dataLabels.*` over `largeLabel`) over its name (`categoryLabels.*` over
//! `lightLabel`), flowing left to right; an accent bar (`card.barShow`,
//! `barWeight`, `barColor`, default data color 1) and a separator line
//! (`card.outlineColor`, `outlineWeight`).
//!
//! # `kpi`
//!
//! The last non-blank `Indicator` value (`indicator.*` over `callout`),
//! colored by `Goal` (`status.direction`: high is good → theme `good` /
//! `bad`), `Goal: … (±x%)` below, and the trend (`TrendLine` axis) as a
//! translucent area across the bottom.

use pbi_engine::Value;

use crate::dataview::{CellValue, DataView, DataViewKind};
use crate::layout::TextAlign;
use crate::scene::{
    Clip, Color, Group, Insets, Paint, PathBuilder, PathData, PathNode, Rect, RectNode, Stroke,
    TextAnchor,
};
use crate::text::TextStyle;
use crate::theme::PropLookup;

use super::VisualCtx;
use super::common::{data_view, display_value, ellipsize, family_metrics, text_node, text_width};

/// Tiles shown at most (untrusted-input guard).
const MAX_TILES: usize = 200;
/// Multi-row card rows shown at most.
const MAX_ROWS: usize = 500;

/// One value of a card.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CardTile {
    pub value: String,
    pub label: String,
    /// Small-multiple header (category values), if any.
    pub header: Option<String>,
    /// Reference labels: `(title, value)`.
    pub references: Vec<(String, String)>,
}

/// Renders a card (dispatching on `visualType`).
pub fn render(ctx: &VisualCtx<'_>) -> Option<Group> {
    match ctx.visual.visual_type.as_str() {
        "card" => legacy_card(ctx),
        "multiRowCard" => multi_row_card(ctx),
        "kpi" => kpi(ctx),
        _ => new_card(ctx),
    }
}

fn is_reference_role(role: &str) -> bool {
    role.to_ascii_lowercase().contains("reference")
}

fn label_of(dv: &DataView, idx: usize) -> String {
    dv.meta(idx)
        .map(|m| m.display_name.clone())
        .unwrap_or_default()
}

/// The tiles of a card's DataView (single value, one-row table, table with
/// category columns, or categorical data).
pub fn card_tiles(dv: &DataView) -> Vec<CardTile> {
    let mut out = Vec::new();
    match dv.kind {
        DataViewKind::Single => {
            let idx = dv.metadata.iter().position(|m| m.is_measure).unwrap_or(0);
            let value = dv
                .single
                .as_ref()
                .map_or_else(|| crate::dataview::BLANK_LABEL.to_owned(), display_value);
            out.push(CardTile {
                value,
                label: label_of(dv, idx),
                ..CardTile::default()
            });
        }
        DataViewKind::Table => {
            let Some(t) = dv.table.as_ref() else {
                return out;
            };
            let is_ref = |m: usize| {
                dv.meta(m)
                    .is_some_and(|c| c.roles.iter().any(|r| is_reference_role(r)))
            };
            let is_cat = |m: usize| dv.meta(m).is_some_and(|c| !c.is_measure) && !is_ref(m);
            let cols: Vec<(usize, usize)> = t.columns.iter().copied().enumerate().collect();
            let cats: Vec<usize> = cols
                .iter()
                .filter(|(_, m)| is_cat(*m))
                .map(|c| c.0)
                .collect();
            let refs: Vec<(usize, usize)> =
                cols.iter().copied().filter(|(_, m)| is_ref(*m)).collect();
            let mut vals: Vec<(usize, usize)> = cols
                .iter()
                .copied()
                .filter(|(_, m)| !is_cat(*m) && !is_ref(*m))
                .collect();
            if vals.is_empty() && cats.is_empty() {
                vals = refs.clone();
            }
            let rows: Vec<&Vec<CellValue>> = if t.rows.is_empty() && cats.is_empty() {
                Vec::new()
            } else {
                t.rows.iter().collect()
            };
            if rows.is_empty() && cats.is_empty() {
                for (_, m) in &vals {
                    out.push(CardTile {
                        value: crate::dataview::BLANK_LABEL.to_owned(),
                        label: label_of(dv, *m),
                        ..CardTile::default()
                    });
                }
                return out;
            }
            for row in rows {
                let header = (!cats.is_empty()).then(|| {
                    cats.iter()
                        .filter_map(|i| row.get(*i).map(display_value))
                        .collect::<Vec<_>>()
                        .join(", ")
                });
                let references: Vec<(String, String)> = refs
                    .iter()
                    .filter_map(|(i, m)| row.get(*i).map(|c| (label_of(dv, *m), display_value(c))))
                    .collect();
                let fields = if vals.is_empty() { &cats } else { &Vec::new() };
                for i in fields {
                    if let Some(c) = row.get(*i) {
                        out.push(CardTile {
                            value: display_value(c),
                            label: t
                                .columns
                                .get(*i)
                                .map(|m| label_of(dv, *m))
                                .unwrap_or_default(),
                            header: None,
                            references: references.clone(),
                        });
                    }
                }
                for (i, m) in &vals {
                    if let Some(c) = row.get(*i) {
                        out.push(CardTile {
                            value: display_value(c),
                            label: label_of(dv, *m),
                            header: header.clone(),
                            references: references.clone(),
                        });
                    }
                }
                if out.len() >= MAX_TILES {
                    break;
                }
            }
        }
        DataViewKind::Categorical => {
            let Some(cat) = dv.categorical.as_ref() else {
                return out;
            };
            for row in 0..cat.len().min(MAX_TILES) {
                let header = (!cat.categories.is_empty()).then(|| {
                    cat.categories
                        .iter()
                        .filter_map(|c| c.values.get(row).map(display_value))
                        .collect::<Vec<_>>()
                        .join(", ")
                });
                for v in &cat.values {
                    if let Some(c) = v.values.get(row) {
                        out.push(CardTile {
                            value: display_value(c),
                            label: label_of(dv, v.meta_idx),
                            header: header.clone(),
                            references: Vec::new(),
                        });
                    }
                }
            }
        }
        DataViewKind::Empty | DataViewKind::Matrix => {}
    }
    out.truncate(MAX_TILES);
    out
}

/// Number of an object, trying several property names.
fn num(o: &PropLookup<'_>, object: &str, props: &[&str]) -> Option<f64> {
    props.iter().find_map(|p| o.number(object, p))
}

/// `paddingUniform`, or the individual sides when `paddingIndividual`.
fn padding(o: &PropLookup<'_>, object: &str, default: f64) -> Insets {
    let uniform = o
        .number(object, "paddingUniform")
        .unwrap_or(default)
        .max(0.0);
    if o.bool(object, "paddingIndividual").unwrap_or(false) {
        let side = |names: &[&str]| num(o, object, names).unwrap_or(uniform).max(0.0);
        Insets {
            top: side(&["paddingTop", "top"]),
            right: side(&["paddingRight", "right"]),
            bottom: side(&["paddingBottom", "bottom"]),
            left: side(&["paddingLeft", "left"]),
        }
    } else {
        Insets::uniform(uniform)
    }
}

fn align_of(o: &PropLookup<'_>, candidates: &[(&str, &str)], default: TextAlign) -> TextAlign {
    candidates
        .iter()
        .find_map(|(obj, p)| o.text(obj, p))
        .map_or(default, |a| TextAlign::parse(&a))
}

/// A one-line text at `top` (line top), aligned in the horizontal span
/// `(x, width)`, clipped to `clip` (no ellipsis: the service clips card
/// text).
fn line_at(
    g: &mut Group,
    text: &str,
    style: &TextStyle,
    (x, w): (f64, f64),
    top: f64,
    align: TextAlign,
    clip: Rect,
) {
    if text.is_empty() {
        return;
    }
    let ascent = family_metrics(style).ascent_px(style);
    let (ax, anchor) = match align {
        TextAlign::Left => (x, TextAnchor::Start),
        TextAlign::Center => (x + w / 2.0, TextAnchor::Middle),
        TextAlign::Right => (x + w, TextAnchor::End),
    };
    g.push(text_node(ax, top + ascent, text, style, anchor, Some(clip)));
}

fn new_card(ctx: &VisualCtx<'_>) -> Option<Group> {
    let dv = data_view(ctx)?;
    let tiles = card_tiles(dv);
    let area = ctx.frame.content;
    if tiles.is_empty() || area.width <= 0.0 || area.height <= 0.0 {
        return None;
    }
    let o = ctx.theme.visual(ctx.visual).objects();
    let outer = padding(&o, "layout", 12.0);
    let gap = o.number("layout", "cellPadding").unwrap_or(12.0).max(0.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let max_tiles = o
        .number("layout", "maxTiles")
        .filter(|n| n.is_finite())
        .map_or(3, |n| n.clamp(1.0, 100.0) as usize);
    let n = tiles.len();
    let cols = n.min(max_tiles).max(1);
    let rows = n.div_ceil(cols);
    let inner = area.inset(outer);
    let tw = ((inner.width - gap * (cols - 1) as f64) / cols as f64).max(0.0);
    let style = TileStyle::new(ctx, &o);
    // Several rows of tiles never shrink below their content (the rest
    // overflows the visual and is clipped); a single row fills the visual.
    let fit = ((inner.height - gap * (rows - 1) as f64) / rows as f64).max(0.0);
    let th = if rows > 1 {
        fit.max(style.min_tile_height())
    } else {
        fit
    };
    let mut g = Group::new();
    for (i, tile) in tiles.iter().enumerate() {
        let (r, c) = (i / cols, i % cols);
        let rect = Rect::new(
            inner.x + c as f64 * (tw + gap),
            inner.y + r as f64 * (th + gap),
            tw,
            th,
        );
        if rect.y > area.bottom() {
            break;
        }
        draw_tile(ctx, &mut g, tile, rect, &style);
    }
    (!g.is_empty()).then_some(g)
}

/// Resolved formatting shared by every tile of a new card.
struct TileStyle {
    value: TextStyle,
    label: TextStyle,
    show_label: bool,
    label_above: bool,
    spacing: f64,
    padding: Insets,
    align: TextAlign,
    valign: String,
    radius: f64,
    fill: Option<Paint>,
    border: Option<Stroke>,
    accent: Option<(Color, f64)>,
    ref_title: TextStyle,
    ref_value: TextStyle,
    divider: Option<Stroke>,
}

impl TileStyle {
    fn new(ctx: &VisualCtx<'_>, o: &PropLookup<'_>) -> Self {
        let theme = ctx.theme;
        // Fallbacks for themes without `visualStyles.cardVisual` (the
        // service's defaults: Segoe UI Semibold callout, secondary label).
        let mut value = o.text_style("value", "callout");
        if o.color("value", "fontColor").is_none() && o.color("value", "color").is_none() {
            value.color = theme.foreground();
        }
        if o.text("value", "fontFamily").is_none() {
            value.family = "Segoe UI Semibold".to_owned();
        }
        let mut label = o.text_style("label", "label");
        if o.color("label", "fontColor").is_none() && o.color("label", "color").is_none() {
            label.color = theme
                .named_color("foregroundNeutralSecondary")
                .unwrap_or(Color::rgb(0x60, 0x5E, 0x5C));
        }
        let tile_shape = o
            .text("shapeCustomRectangle", "tileShape")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let radius = if tile_shape.contains("round") {
            o.number("shapeCustomRectangle", "rectangleRoundedCurve")
                .unwrap_or(4.0)
                .max(0.0)
        } else {
            0.0
        };
        let fill = if o.bool("fillCustom", "show") == Some(true) {
            let color = o
                .color("fillCustom", "fillColor")
                .or_else(|| o.color("fillCustom", "color"))
                .unwrap_or_else(|| theme.background());
            let t = o
                .number("fillCustom", "transparency")
                .unwrap_or(0.0)
                .clamp(0.0, 100.0);
            Some(Paint::with_transparency(color, t))
        } else if o.bool("cardCalloutArea", "show") == Some(true) {
            o.color("cardCalloutArea", "backgroundFillColor")
                .map(Paint::solid)
        } else {
            None
        }
        .filter(Paint::is_visible);
        let border = if o.bool("border", "show").unwrap_or(false) {
            let color = o
                .color("border", "color")
                .or_else(|| o.color("border", "lineColor"))
                .unwrap_or_else(|| theme.foreground());
            let w = num(o, "border", &["width", "weight"]).unwrap_or(1.0);
            Some(Stroke::new(color, w))
        } else if o.bool("outline", "show").unwrap_or(true) {
            let color = o
                .color("outline", "lineColor")
                .or_else(|| theme.named_color("backgroundLight"))
                .unwrap_or(Color::rgb(0xF3, 0xF2, 0xF1));
            let w = num(o, "outline", &["weight", "width"]).unwrap_or(1.0);
            let t = o
                .number("outline", "transparency")
                .unwrap_or(0.0)
                .clamp(0.0, 100.0);
            Some(Stroke::new(color, w).with_opacity(1.0 - t / 100.0))
        } else {
            None
        }
        .filter(Stroke::is_visible);
        let accent = (o.bool("accentBar", "show") == Some(true)).then(|| {
            (
                o.color("accentBar", "color")
                    .unwrap_or_else(|| theme.data_color(0)),
                num(o, "accentBar", &["width", "weight"])
                    .unwrap_or(6.0)
                    .max(0.0),
            )
        });
        let mut ref_title = o.text_style("referenceLabelTitle", "label");
        if let Some(c) = o.color("referenceLabelTitle", "titleFontColor") {
            ref_title.color = c;
        }
        let mut ref_value = o.text_style("referenceLabelValue", "label");
        if let Some(f) = o.text("referenceLabelValue", "valueFontFamily") {
            ref_value.family = f;
        }
        if let Some(c) = o.color("referenceLabelValue", "valueFontColor") {
            ref_value.color = c;
        }
        let divider = (o.bool("divider", "show").unwrap_or(true)).then(|| {
            Stroke::new(
                o.color("divider", "dividerColor")
                    .or_else(|| theme.named_color("backgroundLight"))
                    .unwrap_or(Color::rgb(0xF3, 0xF2, 0xF1)),
                num(o, "divider", &["dividerWeight", "weight"]).unwrap_or(1.0),
            )
        });
        let position = o.text("label", "position").unwrap_or_default();
        Self {
            value,
            label,
            show_label: o.bool("label", "show").unwrap_or(true),
            label_above: position.eq_ignore_ascii_case("aboveValue"),
            spacing: o
                .number("spacing", "verticalSpacing")
                .unwrap_or(2.0)
                .clamp(0.0, 1_000.0),
            padding: padding(o, "padding", 12.0),
            align: align_of(
                o,
                &[
                    ("layout", "alignment"),
                    ("layout", "horizontalAlignment"),
                    ("value", "horizontalAlignment"),
                    ("cardCalloutArea", "horizontalAlignment"),
                ],
                TextAlign::Left,
            ),
            valign: o
                .text("layout", "verticalAlignment")
                .unwrap_or_default()
                .to_ascii_lowercase(),
            radius,
            fill,
            border,
            accent,
            ref_title,
            ref_value,
            divider,
        }
    }
}

impl TileStyle {
    /// Padding + value + spacing + label: the height a tile needs.
    fn min_tile_height(&self) -> f64 {
        let label = if self.show_label {
            self.spacing + family_metrics(&self.label).line_height_px(&self.label)
        } else {
            0.0
        };
        self.padding.top
            + self.padding.bottom
            + family_metrics(&self.value).line_height_px(&self.value)
            + label
    }
}

fn draw_tile(ctx: &VisualCtx<'_>, g: &mut Group, tile: &CardTile, rect: Rect, s: &TileStyle) {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    if let Some(fill) = s.fill {
        g.push(RectNode::new(rect).radius(s.radius).fill(fill));
    }
    let mut body = rect;
    if let Some((color, w)) = s.accent {
        let w = w.min(rect.width);
        g.rect(
            Rect::new(rect.x, rect.y, w, rect.height),
            Some(Paint::solid(color)),
            None,
        );
        body = Rect::new(rect.x + w, rect.y, rect.width - w, rect.height);
    }
    let inner = body.inset(s.padding);
    if inner.width > 0.0 && inner.height > 0.0 {
        let mut text = Group::new().with_clip(Clip::rect(inner));
        tile_text(ctx, &mut text, tile, inner, s);
        if !text.is_empty() {
            g.push(text);
        }
    }
    if let Some(stroke) = &s.border {
        let r = rect.inset_uniform(stroke.width / 2.0);
        g.push(
            RectNode::new(r)
                .radius((s.radius - stroke.width / 2.0).max(0.0))
                .stroke(stroke.clone()),
        );
    }
}

fn tile_text(ctx: &VisualCtx<'_>, g: &mut Group, tile: &CardTile, inner: Rect, s: &TileStyle) {
    let vm = family_metrics(&s.value);
    let lm = family_metrics(&s.label);
    let value_h = vm.line_height_px(&s.value);
    let label_h = if s.show_label && !tile.label.is_empty() {
        lm.line_height_px(&s.label)
    } else {
        0.0
    };
    let header_style = s.label.clone();
    let header_h = tile
        .header
        .as_ref()
        .map_or(0.0, |_| lm.line_height_px(&header_style) + s.spacing);
    let rm = family_metrics(&s.ref_title);
    let ref_line = rm
        .line_height_px(&s.ref_title)
        .max(family_metrics(&s.ref_value).line_height_px(&s.ref_value));
    let refs_h = if tile.references.is_empty() {
        0.0
    } else {
        8.0 + ref_line * tile.references.len() as f64
    };
    let block = header_h
        + value_h
        + if label_h > 0.0 {
            s.spacing + label_h
        } else {
            0.0
        }
        + refs_h;
    let mut top = if header_h + value_h > inner.height {
        // Not even the value fits: centre its line box so the glyphs
        // (which sit inside it) stay visible.
        inner.y + (inner.height - value_h) / 2.0 - header_h
    } else if block > inner.height {
        // Top-aligned: the label (and reference labels) clip first.
        inner.y
    } else {
        match s.valign.as_str() {
            "top" => inner.y,
            "bottom" => inner.bottom() - block,
            _ => inner.y + (inner.height - block) / 2.0,
        }
    };
    let (x, w) = (inner.x, inner.width);
    if let Some(h) = &tile.header {
        let t = ellipsize(ctx.fonts, h, &header_style, w);
        line_at(g, &t, &header_style, (x, w), top, s.align, inner);
        top += header_h;
    }
    let draw_label = |g: &mut Group, top: f64| {
        line_at(g, &tile.label, &s.label, (x, w), top, s.align, inner);
    };
    if label_h > 0.0 && s.label_above {
        draw_label(g, top);
        top += label_h + s.spacing;
        line_at(g, &tile.value, &s.value, (x, w), top, s.align, inner);
        top += value_h;
    } else {
        line_at(g, &tile.value, &s.value, (x, w), top, s.align, inner);
        top += value_h;
        if label_h > 0.0 {
            top += s.spacing;
            draw_label(g, top);
            top += label_h;
        }
    }
    if !tile.references.is_empty() {
        top += 4.0;
        if let Some(d) = &s.divider {
            g.line(x, top, x + w, top, d.clone());
        }
        top += 4.0;
        for (title, value) in &tile.references {
            let tm = family_metrics(&s.ref_title);
            let vm = family_metrics(&s.ref_value);
            let baseline = top + tm.ascent_px(&s.ref_title).max(vm.ascent_px(&s.ref_value));
            let title_w = text_width(ctx.fonts, title, &s.ref_title);
            g.push(text_node(
                x,
                baseline,
                title.clone(),
                &s.ref_title,
                TextAnchor::Start,
                Some(inner),
            ));
            let value_x = x + title_w + 8.0;
            if value_x < x + w {
                g.push(text_node(
                    x + w,
                    baseline,
                    ellipsize(ctx.fonts, value, &s.ref_value, x + w - value_x),
                    &s.ref_value,
                    TextAnchor::End,
                    Some(inner),
                ));
            }
            top += ref_line;
        }
    }
}

fn legacy_card(ctx: &VisualCtx<'_>) -> Option<Group> {
    let dv = data_view(ctx)?;
    let tile = card_tiles(dv).into_iter().next()?;
    let c = ctx.frame.content;
    if c.width <= 0.0 || c.height <= 0.0 {
        return None;
    }
    let o = ctx.theme.visual(ctx.visual).objects();
    let mut value = o.text_style("labels", "callout");
    let label = o.text_style("categoryLabels", "largeLightLabel");
    let show_label = o.bool("categoryLabels", "show").unwrap_or(true) && !tile.label.is_empty();
    // The service shrinks the callout until it fits the width.
    let fonts = ctx.fonts;
    while value.size_pt > 8.0 && text_width(fonts, &tile.value, &value) > c.width {
        value.size_pt = (value.size_pt * 0.9).max(8.0);
    }
    let vm = family_metrics(&value);
    let lm = family_metrics(&label);
    let value_h = vm.line_height_px(&value);
    let label_h = if show_label {
        lm.line_height_px(&label)
    } else {
        0.0
    };
    let block = value_h + label_h;
    let top = c.y + ((c.height - block) / 2.0).max(0.0);
    let mut g = Group::new();
    let v = ellipsize(fonts, &tile.value, &value, c.width);
    line_at(
        &mut g,
        &v,
        &value,
        (c.x, c.width),
        top,
        TextAlign::Center,
        c,
    );
    if show_label {
        let l = ellipsize(fonts, &tile.label, &label, c.width);
        line_at(
            &mut g,
            &l,
            &label,
            (c.x, c.width),
            top + value_h,
            TextAlign::Center,
            c,
        );
    }
    (!g.is_empty()).then_some(g)
}

fn multi_row_card(ctx: &VisualCtx<'_>) -> Option<Group> {
    let dv = data_view(ctx)?;
    let t = dv.table.as_ref()?;
    let c = ctx.frame.content;
    if c.width <= 0.0 || c.height <= 0.0 || t.columns.is_empty() {
        return None;
    }
    let o = ctx.theme.visual(ctx.visual).objects();
    let theme = ctx.theme;
    let value_style = o.text_style("dataLabels", "largeLabel");
    let label_style = o.text_style("categoryLabels", "lightLabel");
    let title_style = o.text_style("cardTitle", "largeLabel");
    let show_labels = o.bool("categoryLabels", "show").unwrap_or(true);
    let bar = o.bool("card", "barShow").unwrap_or(true).then(|| {
        (
            o.color("card", "barColor")
                .unwrap_or_else(|| theme.data_color(0)),
            o.number("card", "barWeight")
                .unwrap_or(2.0)
                .clamp(0.0, 50.0),
        )
    });
    let separator = Stroke::new(
        o.color("card", "outlineColor")
            .unwrap_or(Color::rgb(0xC8, 0xC6, 0xC4)),
        o.number("card", "outlineWeight")
            .unwrap_or(1.0)
            .clamp(0.0, 50.0),
    );
    let title_col = t
        .columns
        .first()
        .filter(|m| dv.meta(**m).is_some_and(|c| !c.is_measure) && t.columns.len() > 1)
        .map(|_| 0usize);
    let vm = family_metrics(&value_style);
    let lm = family_metrics(&label_style);
    let tm = family_metrics(&title_style);
    let field_h = vm.line_height_px(&value_style)
        + if show_labels {
            lm.line_height_px(&label_style)
        } else {
            0.0
        };
    let fonts = ctx.fonts;
    let mut g = Group::new();
    let mut y = c.y;
    for row in t.rows.iter().take(MAX_ROWS) {
        if y >= c.bottom() {
            break;
        }
        let bar_w = bar.map_or(0.0, |b| b.1);
        let x0 = c.x + bar_w + if bar_w > 0.0 { 10.0 } else { 0.0 };
        let avail = (c.right() - x0).max(0.0);
        let start = y + 5.0;
        let mut cy = start;
        if let Some(cell) = title_col.and_then(|i| row.get(i)) {
            let text = ellipsize(fonts, &display_value(cell), &title_style, avail);
            g.push(text_node(
                x0,
                cy + tm.ascent_px(&title_style),
                text,
                &title_style,
                TextAnchor::Start,
                Some(c),
            ));
            cy += tm.line_height_px(&title_style) + 2.0;
        }
        let mut x = x0;
        for (ci, m) in t.columns.iter().enumerate() {
            if Some(ci) == title_col {
                continue;
            }
            let Some(cell) = row.get(ci) else { continue };
            let value = display_value(cell);
            let name = label_of(dv, *m);
            let w = text_width(fonts, &value, &value_style)
                .max(if show_labels {
                    text_width(fonts, &name, &label_style)
                } else {
                    0.0
                })
                .min(avail);
            if x > x0 && x + w > x0 + avail {
                x = x0;
                cy += field_h + 4.0;
            }
            let clip = Rect::new(x, cy, w.max(1.0), field_h);
            g.push(text_node(
                x,
                cy + vm.ascent_px(&value_style),
                ellipsize(fonts, &value, &value_style, avail),
                &value_style,
                TextAnchor::Start,
                Some(clip.intersect(&c).unwrap_or(clip)),
            ));
            if show_labels {
                g.push(text_node(
                    x,
                    cy + vm.line_height_px(&value_style) + lm.ascent_px(&label_style),
                    ellipsize(fonts, &name, &label_style, avail),
                    &label_style,
                    TextAnchor::Start,
                    Some(clip.intersect(&c).unwrap_or(clip)),
                ));
            }
            x += w + 20.0;
        }
        let end = cy + field_h + 5.0;
        if let Some((color, w)) = bar.filter(|b| b.1 > 0.0) {
            g.rect(
                Rect::new(c.x, y, w, end - y),
                Some(Paint::solid(color)),
                None,
            );
        }
        if separator.is_visible() {
            g.line(c.x, end + 5.0, c.right(), end + 5.0, separator.clone());
        }
        y = end + 10.0;
    }
    (!g.is_empty()).then_some(g)
}

fn kpi(ctx: &VisualCtx<'_>) -> Option<Group> {
    let dv = data_view(ctx)?;
    let cat = dv.categorical.as_ref()?;
    let c = ctx.frame.content;
    if c.width <= 0.0 || c.height <= 0.0 {
        return None;
    }
    let role_col = |role: &str| {
        cat.values.iter().find(|v| {
            dv.meta(v.meta_idx)
                .is_some_and(|m| m.roles.iter().any(|r| r.eq_ignore_ascii_case(role)))
        })
    };
    let indicator = role_col("Indicator").or(cat.values.first())?;
    let goal = role_col("Goal").or_else(|| role_col("TargetValue"));
    let last = indicator.values.iter().rposition(|v| !v.is_blank());
    let value_cell = last.and_then(|i| indicator.values.get(i));
    let goal_cell = last.and_then(|i| goal.and_then(|g| g.values.get(i)));
    let o = ctx.theme.visual(ctx.visual).objects();
    let theme = ctx.theme;
    let as_f64 = |cell: Option<&CellValue>| -> Option<f64> {
        match &cell?.raw {
            Value::Integer(n) => Some(*n as f64),
            Value::Double(d) if d.is_finite() => Some(*d),
            other => other.as_f64().filter(|v| v.is_finite()),
        }
    };
    let (v, gl) = (as_f64(value_cell), as_f64(goal_cell));
    let high_is_good = !o
        .text("status", "direction")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .contains("low");
    let status = match (v, gl) {
        (Some(v), Some(goal)) => {
            let good = if high_is_good { v >= goal } else { v <= goal };
            Some(if good {
                o.color("status", "goodColor")
                    .or_else(|| theme.named_color("good"))
                    .unwrap_or(Color::rgb(0x1A, 0xAB, 0x40))
            } else {
                o.color("status", "badColor")
                    .or_else(|| theme.named_color("bad"))
                    .unwrap_or(Color::rgb(0xD6, 0x45, 0x54))
            })
        }
        _ => None,
    };
    let mut g = Group::new();
    let trend_color = status.unwrap_or_else(|| theme.data_color(0));
    let trend_h = c.height * 0.4;
    let points: Vec<f64> = indicator
        .values
        .iter()
        .map(|cell| as_f64(Some(cell)).unwrap_or(f64::NAN))
        .collect();
    if o.bool("trendline", "show").unwrap_or(true)
        && let Some(d) = trend_area(
            &points,
            Rect::new(c.x, c.bottom() - trend_h, c.width, trend_h),
        )
    {
        let t = o
            .number("trendline", "transparency")
            .unwrap_or(20.0)
            .clamp(0.0, 100.0);
        // A light area so the indicator stays readable over it.
        g.push(PathNode::new(d).fill(Paint::new(trend_color, (1.0 - t / 100.0) * 0.3)));
    }
    let mut value_style = o.text_style("indicator", "callout");
    if let Some(color) = status {
        value_style.color = color;
    }
    let fonts = ctx.fonts;
    let value_text =
        value_cell.map_or_else(|| crate::dataview::BLANK_LABEL.to_owned(), display_value);
    while value_style.size_pt > 8.0 && text_width(fonts, &value_text, &value_style) > c.width {
        value_style.size_pt = (value_style.size_pt * 0.9).max(8.0);
    }
    let vm = family_metrics(&value_style);
    let goal_style = o.text_style("goals", "lightLabel");
    let gm = family_metrics(&goal_style);
    let top_area = c.height - trend_h;
    let block = vm.line_height_px(&value_style) + gm.line_height_px(&goal_style);
    let top = c.y + ((top_area - block) / 2.0).max(0.0);
    line_at(
        &mut g,
        &ellipsize(fonts, &value_text, &value_style, c.width),
        &value_style,
        (c.x, c.width),
        top,
        TextAlign::Center,
        c,
    );
    if let Some(goal_cell) = goal_cell {
        let mut text = format!("Goal: {}", display_value(goal_cell));
        if let (Some(v), Some(goal)) = (v, gl)
            && goal != 0.0
        {
            let pct = (v - goal) / goal.abs() * 100.0;
            text.push_str(&format!(
                " ({}{:.2}%)",
                if pct >= 0.0 { "+" } else { "" },
                pct
            ));
        }
        line_at(
            &mut g,
            &ellipsize(fonts, &text, &goal_style, c.width),
            &goal_style,
            (c.x, c.width),
            top + vm.line_height_px(&value_style),
            TextAlign::Center,
            c,
        );
    }
    (!g.is_empty()).then_some(g)
}

/// A closed area under `values` (NaN = gap, drawn at the baseline) spanning
/// `r`; `None` with fewer than two finite values.
fn trend_area(values: &[f64], r: Rect) -> Option<PathData> {
    let finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.len() < 2 || r.is_empty() {
        return None;
    }
    let min = finite
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min)
        .min(0.0);
    let max = finite.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = if max - min > 0.0 { max - min } else { 1.0 };
    let n = values.len();
    let step = r.width / (n - 1).max(1) as f64;
    let mut b = PathBuilder::new();
    b.move_to(r.x, r.bottom());
    for (i, v) in values.iter().enumerate() {
        let y = if v.is_finite() {
            r.bottom() - (v - min) / span * r.height
        } else {
            r.bottom()
        };
        b.line_to(r.x + i as f64 * step, y);
    }
    b.line_to(r.x + (n - 1) as f64 * step, r.bottom()).close();
    Some(b.build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trend_area_needs_two_points() {
        let r = Rect::new(0.0, 0.0, 100.0, 10.0);
        assert!(trend_area(&[1.0], r).is_none());
        assert!(trend_area(&[f64::NAN, 1.0], r).is_none());
        let d = trend_area(&[1.0, f64::NAN, 3.0], r).unwrap();
        let b = d.bounds().unwrap();
        assert!(r.contains_rect(&b, 1e-6));
    }
}
