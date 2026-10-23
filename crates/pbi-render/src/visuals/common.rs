//! Shared helpers for visuals: the query-error, no-data and data-reduced
//! states, a vector warning icon, text drawing with per-family metrics and
//! value display. (owner: visuals-basics; other visual agents may call these
//! helpers and add their own in their files.)
//!
//! # Families other than Segoe UI
//!
//! Segoe UI (and DIN, Arial, unknown families) render with the bundled
//! Selawik, which has Segoe UI's metrics; Calibri renders with the bundled
//! Carlito, which has Calibri's advance widths and vertical metrics
//! ([`crate::text::fonts`]). [`family_metrics`] returns, per family, the
//! vertical metrics browsers use (`usWinAscent` / `usWinDescent`, which
//! define `line-height: normal`):
//!
//! | Family | Ascent / descent (em) | Face |
//! |---|---|---|
//! | Calibri | 1950/2048, 550/2048 (line 1.22 em) | Carlito |
//! | anything else | Segoe UI: 2210/2048, 514/2048 (line 1.33 em) | Selawik |
//!
//! [`FamilyMetrics`] can also carry a glyph size adjustment (like CSS
//! `size-adjust`) and an advance-width factor for a family drawn with a
//! stand-in face of different proportions: [`text_node`] then draws with the
//! adjusted size and applies the factor as a horizontal scale about the
//! anchor, and [`text_width`], [`ellipsize`] and [`wrap`] measure the same
//! way. Both are 1 for every family today (Calibri used to be emulated with
//! Selawik at 0.9 × size and a 0.96–1.02 width factor before Carlito was
//! bundled). Line boxes and baselines use the nominal size.

use crate::dataview::{BLANK_LABEL, CellValue, DataView};
use crate::layout::frames::PLACEHOLDER_COLOR;
use crate::query::VisualQueryError;
use crate::scene::{
    Color, Group, Node, Paint, PathBuilder, PathNode, Rect, TextAnchor, TextNode, Transform,
};
use crate::text::{ASCENT_EM, DESCENT_EM, FontBook, MeasureOptions, TextStyle, is_calibri};

use super::VisualCtx;

/// Vertical metrics (em) and advance-width factor of a family as the
/// service renders it (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FamilyMetrics {
    /// Baseline below the top of the line box.
    pub ascent: f64,
    /// Line box below the baseline.
    pub descent: f64,
    /// Glyph size relative to the nominal size (1 for every bundled
    /// family).
    pub size_adjust: f64,
    /// Horizontal scale applied to the size-adjusted advances of the
    /// bundled face (1 for every bundled family).
    pub width_factor: f64,
}

impl FamilyMetrics {
    /// Segoe UI (and Selawik).
    pub const SEGOE_UI: FamilyMetrics = FamilyMetrics {
        ascent: ASCENT_EM,
        descent: DESCENT_EM,
        size_adjust: 1.0,
        width_factor: 1.0,
    };

    /// Baseline offset in pixels for `style`'s size.
    pub fn ascent_px(&self, style: &TextStyle) -> f64 {
        style.size_px() * self.ascent
    }

    /// Descent in pixels for `style`'s size.
    pub fn descent_px(&self, style: &TextStyle) -> f64 {
        style.size_px() * self.descent
    }

    /// `line-height: normal` in pixels for `style`'s size.
    pub fn line_height_px(&self, style: &TextStyle) -> f64 {
        style.size_px() * (self.ascent + self.descent)
    }
}

impl FamilyMetrics {
    /// Calibri (and the bundled Carlito): `usWinAscent` 1950,
    /// `usWinDescent` 550 per 2048 units.
    pub const CALIBRI: FamilyMetrics = FamilyMetrics {
        ascent: 1950.0 / 2048.0,
        descent: 550.0 / 2048.0,
        size_adjust: 1.0,
        width_factor: 1.0,
    };
}

/// Metrics of the family `style` names (see the module docs).
pub fn family_metrics(style: &TextStyle) -> FamilyMetrics {
    if is_calibri(&style.family) {
        FamilyMetrics::CALIBRI
    } else {
        FamilyMetrics::SEGOE_UI
    }
}

/// `style` with the family's glyph size adjustment applied (the style
/// [`text_node`] draws with).
pub fn adjusted_style(style: &TextStyle) -> TextStyle {
    let m = family_metrics(style);
    if (m.size_adjust - 1.0).abs() < 1e-9 {
        style.clone()
    } else {
        style.clone().with_size_pt(style.size_pt * m.size_adjust)
    }
}

/// Advance width of one line of `text` as drawn by [`text_node`].
pub fn text_width(fonts: &FontBook, text: &str, style: &TextStyle) -> f64 {
    fonts.text_width(text, &adjusted_style(style)) * family_metrics(style).width_factor
}

/// `text`, or its longest prefix + `…` that fits `max_width` (as drawn by
/// [`text_node`]).
pub fn ellipsize(fonts: &FontBook, text: &str, style: &TextStyle, max_width: f64) -> String {
    let f = family_metrics(style).width_factor;
    fonts.ellipsize(text, &adjusted_style(style), max_width.max(0.0) / f)
}

/// Wraps `text` to `max_width` (as drawn by [`text_node`]), keeping at most
/// `max_lines` lines (the last one ellipsized when lines are dropped).
pub fn wrap(
    fonts: &FontBook,
    text: &str,
    style: &TextStyle,
    max_width: f64,
    max_lines: Option<usize>,
) -> Vec<String> {
    let f = family_metrics(style).width_factor;
    fonts
        .measure_with(
            text,
            &adjusted_style(style),
            &MeasureOptions {
                max_width: Some(max_width.max(0.0) / f),
                wrap: true,
                max_lines,
                ellipsis: true,
            },
        )
        .lines
        .into_iter()
        .map(|l| l.text)
        .collect()
}

/// One line of text at `(x, baseline)` drawn with the family's adjusted
/// size and horizontally scaled about `x` by its width factor (a plain
/// [`TextNode`] when the factor is 1). `clip` is in the caller's
/// coordinates.
pub fn text_node(
    x: f64,
    baseline: f64,
    text: impl Into<String>,
    style: &TextStyle,
    anchor: TextAnchor,
    clip: Option<Rect>,
) -> Node {
    let f = family_metrics(style).width_factor;
    let style = adjusted_style(style);
    let text = text.into();
    if (f - 1.0).abs() < 1e-9 {
        let mut node = TextNode::new(x, baseline, text, style).anchor(anchor);
        node.clip = clip;
        return node.into();
    }
    let mut node = TextNode::new(0.0, baseline, text, style).anchor(anchor);
    node.clip = clip.map(|c| Rect::new((c.x - x) / f, c.y, c.width / f, c.height));
    let mut g =
        Group::new().with_transform(Transform::translate(x, 0.0).then(&Transform::scale(f, 1.0)));
    g.push(node);
    g.into()
}

/// The text a card or header shows for a value: its formatted text, or
/// `(Blank)` for BLANK.
pub fn display_value(cell: &CellValue) -> String {
    if cell.formatted.is_empty() && cell.is_blank() {
        BLANK_LABEL.to_owned()
    } else {
        cell.formatted.clone()
    }
}

/// Parses a CSS color: `#RGB`, `#RRGGBB`, `#RRGGBBAA`, `rgb(r, g, b)` or
/// `rgba(r, g, b, a)` (alpha ignored).
pub fn parse_css_color(text: &str) -> Option<Color> {
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    let inner = lower
        .strip_prefix("rgba(")
        .or_else(|| lower.strip_prefix("rgb("))
        .and_then(|s| s.strip_suffix(')'));
    let Some(inner) = inner else {
        return Color::parse(t);
    };
    let mut parts = inner.split(',').map(|p| p.trim().parse::<f64>().ok());
    let mut channel = || -> Option<u8> {
        let v = parts.next()??;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let c = v.clamp(0.0, 255.0).round() as u8;
        v.is_finite().then_some(c)
    };
    Some(Color::rgb(channel()?, channel()?, channel()?))
}

/// Color of the warning icon.
pub const WARNING_COLOR: Color = Color::rgb(0xD6, 0x45, 0x54);

/// A warning sign (rounded triangle with `!`) of `size` px centred on
/// `(cx, cy)`, drawn as paths (no glyph needed).
pub fn warning_icon(cx: f64, cy: f64, size: f64, color: Color) -> Group {
    let mut g = Group::new();
    if !(size.is_finite() && size > 0.0) {
        return g;
    }
    let h = size * 0.9;
    let (top, bottom) = (cy - h / 2.0, cy + h / 2.0);
    let half = size / 2.0;
    let mut tri = PathBuilder::new();
    tri.move_to(cx, top)
        .line_to(cx + half, bottom)
        .line_to(cx - half, bottom)
        .close();
    g.push(PathNode::new(tri.build()).fill(Paint::solid(color)));
    let bar_w = (size * 0.12).max(0.5);
    let mut bang = PathBuilder::new();
    let bar_top = top + h * 0.34;
    let bar_bottom = top + h * 0.68;
    bang.move_to(cx - bar_w / 2.0, bar_top)
        .line_to(cx + bar_w / 2.0, bar_top)
        .line_to(cx + bar_w / 2.0, bar_bottom)
        .line_to(cx - bar_w / 2.0, bar_bottom)
        .close();
    let dot_top = top + h * 0.76;
    let dot_bottom = dot_top + bar_w;
    bang.move_to(cx - bar_w / 2.0, dot_top)
        .line_to(cx + bar_w / 2.0, dot_top)
        .line_to(cx + bar_w / 2.0, dot_bottom)
        .line_to(cx - bar_w / 2.0, dot_bottom)
        .close();
    g.push(PathNode::new(bang.build()).fill(Paint::solid(Color::WHITE)));
    g
}

/// Full text for a failed visual's tooltip: the message, the failing
/// field and the DAX text.
pub fn error_tooltip(err: &VisualQueryError) -> String {
    let mut out = err.display();
    if let Some(dax) = err.dax.as_deref().filter(|d| !d.trim().is_empty()) {
        out.push_str("\n\n");
        out.push_str(dax.trim());
    }
    out
}

/// Content shown when the visual's query failed: a warning icon, "Can't
/// display this visual" and the (wrapped) message, centred in the content
/// box. The full message and DAX text are in [`error_tooltip`] (exposed per
/// visual by [`super::page::VisualRenderInfo::tooltip`]).
pub fn error_content(ctx: &VisualCtx<'_>, err: &VisualQueryError) -> Group {
    let c = ctx.frame.content;
    let mut g = Group::new().with_class("pbi-visual-error");
    if c.width <= 2.0 || c.height <= 2.0 {
        return g;
    }
    let fonts = ctx.fonts;
    let title_style = ctx.theme.text_class("semiboldLabel");
    let msg_style = ctx.theme.text_class("smallLightLabel");
    let title = if err.cancelled {
        "Query cancelled"
    } else {
        "Can't display this visual"
    };
    let message = match &err.field {
        Some(f) => format!("{f}: {}", err.message),
        None => err.message.clone(),
    };
    let icon = 16.0_f64.min(c.height).min(c.width);
    let title_h = title_style.line_height();
    let msg_lh = msg_style.line_height();
    let room = (c.height - icon - 4.0 - title_h).max(0.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let max_lines = ((room / msg_lh).floor().max(0.0) as usize).min(4);
    let lines = if max_lines > 0 {
        wrap(fonts, &message, &msg_style, c.width - 8.0, Some(max_lines))
    } else {
        Vec::new()
    };
    let show_title = c.height >= icon + 4.0 + title_h;
    let block_h = icon + if show_title { 4.0 + title_h } else { 0.0 } + msg_lh * lines.len() as f64;
    let (cx, _) = c.center();
    let mut y = c.y + ((c.height - block_h) / 2.0).max(0.0);
    g.push(warning_icon(cx, y + icon / 2.0, icon, WARNING_COLOR));
    y += icon;
    if show_title {
        y += 4.0;
        let t = ellipsize(fonts, title, &title_style, c.width - 4.0);
        g.push(text_node(
            cx,
            y + title_style.ascent(),
            t,
            &title_style,
            TextAnchor::Middle,
            None,
        ));
        y += title_h;
    }
    for line in lines {
        g.push(text_node(
            cx,
            y + msg_style.ascent(),
            line,
            &msg_style,
            TextAnchor::Middle,
            None,
        ));
        y += msg_lh;
    }
    g
}

/// A light "No data" label centred in the content box (for visuals whose
/// query returned no rows).
pub fn no_data(ctx: &VisualCtx<'_>) -> Group {
    centered_label(ctx, "No data")
}

/// A light, centred, ellipsized one-line label in the content box (omitted
/// when the box is too small).
pub fn centered_label(ctx: &VisualCtx<'_>, text: &str) -> Group {
    let mut g = Group::new();
    let c = ctx.frame.content;
    let style = ctx
        .theme
        .text_class("smallLabel")
        .with_color(PLACEHOLDER_COLOR);
    if c.height < style.line_height() || c.width <= 2.0 {
        return g;
    }
    let t = ellipsize(ctx.fonts, text, &style, c.width - 2.0);
    let (cx, cy) = c.center();
    g.push(text_node(
        cx,
        cy - style.line_height() / 2.0 + style.ascent(),
        t,
        &style,
        TextAnchor::Middle,
        None,
    ));
    g
}

/// The successful DataView of the visual, if any.
pub fn data_view<'a>(ctx: &VisualCtx<'a>) -> Option<&'a DataView> {
    ctx.data.and_then(|r| r.as_ref().ok())
}

/// A small warning sign in the top-right corner of the visual when its
/// data was cut at the query limits (Power BI's "Not all data shown"
/// hint). Push it after the visual's content.
pub fn data_reduced_hint(ctx: &VisualCtx<'_>) -> Option<Group> {
    let dv = data_view(ctx)?;
    if !dv.truncated {
        return None;
    }
    let r = ctx.frame.inner;
    let size = 10.0_f64.min(r.width / 2.0).min(r.height / 2.0);
    if size < 4.0 {
        return None;
    }
    let mut g = warning_icon(
        r.right() - size / 2.0 - 3.0,
        r.y + size / 2.0 + 3.0,
        size,
        Color::rgb(0xD9, 0xB3, 0x00),
    );
    g.class = Some("pbi-data-reduced".to_owned());
    Some(g)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_colors() {
        assert_eq!(parse_css_color("#C00000"), Some(Color::rgb(0xC0, 0, 0)));
        assert_eq!(
            parse_css_color("rgb(1, 2, 300)"),
            Some(Color::rgb(1, 2, 255))
        );
        assert_eq!(
            parse_css_color("RGBA(10,20,30,0.5)"),
            Some(Color::rgb(10, 20, 30))
        );
        assert_eq!(parse_css_color("rgb(1,2)"), None);
        assert_eq!(parse_css_color("rgb(a,b,c)"), None);
        assert_eq!(parse_css_color("nope"), None);
    }

    #[test]
    fn calibri_metrics() {
        let calibri = TextStyle::new("Calibri", 12.0, Color::BLACK);
        let m = family_metrics(&calibri);
        assert!((m.line_height_px(&calibri) - 16.0 * 2500.0 / 2048.0).abs() < 1e-9);
        assert_eq!(
            family_metrics(&calibri.clone().bold()),
            FamilyMetrics::CALIBRI
        );
        // Carlito has Calibri's advances: drawn at the nominal size, unscaled.
        assert_eq!(adjusted_style(&calibri), calibri);
        let segoe = TextStyle::new("'Segoe UI', wf_segoe-ui_normal", 12.0, Color::BLACK);
        assert_eq!(family_metrics(&segoe), FamilyMetrics::SEGOE_UI);
        for style in [&calibri, &segoe] {
            assert!(matches!(
                text_node(5.0, 10.0, "x", style, TextAnchor::Start, None),
                Node::Text(_)
            ));
        }
        let fonts = FontBook::shared();
        let w = text_width(fonts, "Directions", &calibri);
        // Carlito Regular: 8442 units for "Directions" at 16 px / 2048 em.
        assert!((w - 8442.0 * 16.0 / 2048.0).abs() < 0.5, "{w}");
    }

    #[test]
    fn blank_values_display_as_blank_label() {
        assert_eq!(display_value(&CellValue::blank()), "(Blank)");
        assert_eq!(
            display_value(&CellValue::new(pbi_engine::Value::Integer(3), "3")),
            "3"
        );
    }
}
