//! Axis settings (`categoryAxis`, `valueAxis`, `y2Axis`) and drawing
//! helpers: tick labels, titles, gridlines, scrollbars.
//!
//! Defaults follow the service renders: axis labels in the
//! `smallLightLabel` class (9pt `#605E5C`), titles in the `title` class
//! family and color at the label size (9pt), dotted `#C8C6C4` value
//! gridlines, no category gridlines.

use crate::scene::{Color, Group, Paint, Rect, RectNode, Stroke};
use crate::text::TextStyle;
use crate::theme::{ThemeResolver, VisualProps};

use super::format::DisplayUnit;

/// Gridline color when the theme and the visual set none.
pub const GRIDLINE_COLOR: Color = Color::rgb(0xC8, 0xC6, 0xC4);
/// Scrollbar track color.
pub const SCROLL_TRACK: Color = Color::rgb(0xF0, 0xF0, 0xF0);
/// Scrollbar thumb color.
pub const SCROLL_THUMB: Color = Color::rgb(0xB7, 0xB7, 0xB7);
/// Scrollbar thickness.
pub const SCROLLBAR_SIZE: f64 = 8.0;

/// Resolved settings of one axis object.
#[derive(Debug, Clone, PartialEq)]
pub struct AxisSettings {
    pub show: bool,
    pub label_style: TextStyle,
    /// Title text when shown (`showAxisTitle` and a non-empty text).
    pub title: Option<String>,
    pub title_style: TextStyle,
    /// Gridline stroke when shown.
    pub gridlines: Option<Stroke>,
    /// `start` / `end` overrides.
    pub start: Option<f64>,
    pub end: Option<f64>,
    /// `labelDisplayUnits` (`None`: Auto).
    pub unit: Option<DisplayUnit>,
    pub precision: Option<usize>,
    /// `axisType` is `Categorical`.
    pub categorical: bool,
    /// Gap between bands as a fraction of the band (`innerPadding`, %).
    pub inner_padding: f64,
    /// Minimum band width/height (`preferredCategoryWidth`).
    pub min_category: f64,
    /// Largest share of the visual the labels may take (`maxMarginFactor`).
    pub max_margin: f64,
}

impl AxisSettings {
    /// Reads `object` (`categoryAxis`, `valueAxis`, `y2Axis`). The title
    /// defaults to `default_title`; gridlines default to `default_grid`.
    pub fn read(
        props: &VisualProps<'_>,
        resolver: &ThemeResolver,
        object: &str,
        default_title: Option<String>,
        default_grid: bool,
    ) -> Self {
        let o = props.objects();
        let mut label_style = o.text_style(object, "smallLightLabel");
        if let Some(c) = o.color(object, "labelColor") {
            label_style.color = c;
        }
        let title_class = resolver.text_class("title");
        let mut title_style = TextStyle {
            size_pt: label_style.size_pt,
            ..title_class
        };
        if let Some(c) = o.color(object, "titleColor") {
            title_style.color = c;
        }
        if let Some(s) = o.number(object, "titleFontSize").filter(|s| *s > 0.0) {
            title_style.size_pt = s;
        }
        if let Some(f) = o
            .text(object, "titleFontFamily")
            .filter(|f| !f.trim().is_empty())
        {
            title_style.family = f;
        }
        if o.bool(object, "titleBold") == Some(true) {
            title_style.weight = 700;
        }
        let title = if o.bool(object, "showAxisTitle").unwrap_or(true) {
            o.text(object, "titleText")
                .filter(|t| !t.trim().is_empty())
                .or(default_title)
        } else {
            None
        };
        let gridlines = o
            .bool(object, "gridlineShow")
            .unwrap_or(default_grid)
            .then(|| {
                let color = o
                    .color(object, "gridlineColor")
                    .or_else(|| resolver.named_color("backgroundNeutral"))
                    .unwrap_or(GRIDLINE_COLOR);
                let w = o
                    .number(object, "gridlineThickness")
                    .filter(|w| *w > 0.0)
                    .unwrap_or(1.0)
                    .min(20.0);
                let style = o
                    .text(object, "gridlineStyle")
                    .unwrap_or_else(|| "dotted".to_owned())
                    .to_ascii_lowercase();
                let dash = match style.as_str() {
                    "solid" => Vec::new(),
                    "dashed" => vec![4.0 * w, 3.0 * w],
                    _ => vec![w, 4.0 * w],
                };
                Stroke::new(color, w).with_dash(dash)
            });
        let pct = |p: &str, default: f64| {
            o.number(object, p)
                .filter(|v| v.is_finite())
                .map_or(default, |v| (v / 100.0).clamp(0.0, 0.9))
        };
        Self {
            show: o.bool(object, "show").unwrap_or(true),
            label_style,
            title,
            title_style,
            gridlines,
            start: o.number(object, "start"),
            end: o.number(object, "end"),
            unit: DisplayUnit::from_setting(o.number(object, "labelDisplayUnits")),
            precision: o
                .number(object, "labelPrecision")
                .filter(|p| *p >= 0.0)
                .map(|p| p.min(10.0) as usize),
            categorical: o
                .text(object, "axisType")
                .is_some_and(|t| t.eq_ignore_ascii_case("categorical")),
            inner_padding: pct("innerPadding", 0.3),
            min_category: o
                .number(object, "preferredCategoryWidth")
                .filter(|v| *v > 0.0)
                .unwrap_or(20.0)
                .clamp(2.0, 500.0),
            max_margin: pct("maxMarginFactor", 0.25).max(0.05),
        }
    }

    /// Applies `start`/`end` to a data domain (keeping it non-degenerate).
    pub fn apply_range(&self, (lo, hi): (f64, f64)) -> (f64, f64) {
        let lo2 = self.start.filter(|v| v.is_finite()).unwrap_or(lo);
        let hi2 = self.end.filter(|v| v.is_finite()).unwrap_or(hi);
        if hi2 > lo2 { (lo2, hi2) } else { (lo, hi) }
    }
}

/// A scrollbar: light track along `track` with a thumb at `offset`
/// (fraction of items before the first visible one) sized by `visible`
/// (fraction of items shown).
pub fn scrollbar(g: &mut Group, track: Rect, visible: f64, offset: f64, vertical: bool) {
    if track.is_empty() {
        return;
    }
    let mut sb = Group::new().with_class("pbi-scrollbar");
    let radius = SCROLLBAR_SIZE / 2.0;
    sb.push(
        RectNode::new(track)
            .radius(radius)
            .fill(Paint::solid(SCROLL_TRACK)),
    );
    let visible = if visible.is_finite() {
        visible.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let offset = if offset.is_finite() {
        offset.clamp(0.0, 1.0 - visible)
    } else {
        0.0
    };
    let thumb = if vertical {
        let len = (track.height * visible).max(2.0).min(track.height);
        Rect::new(track.x, track.y + track.height * offset, track.width, len)
    } else {
        let len = (track.width * visible).max(2.0).min(track.width);
        Rect::new(track.x + track.width * offset, track.y, len, track.height)
    };
    sb.push(
        RectNode::new(thumb)
            .radius(radius)
            .fill(Paint::solid(SCROLL_THUMB)),
    );
    g.push(sb);
}
