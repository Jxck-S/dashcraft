//! Data-label settings (`labels` object) and text helpers shared by the
//! charts: Power BI-style truncation with `...`, vertical centring, rotated
//! text.

use crate::scene::{Group, TextAnchor, TextNode, Transform};
use crate::text::{FontBook, TextStyle};
use crate::theme::VisualProps;

use super::format::DisplayUnit;

/// The ellipsis Power BI appends to truncated chart text.
pub const DOTS: &str = "...";

/// Advance width of `text`.
pub fn width(fonts: &FontBook, text: &str, style: &TextStyle) -> f64 {
    fonts.text_width(text, style)
}

/// `text` if it fits in `max_width`, else its longest prefix that fits
/// followed by `...` (empty if not even `...` fits).
pub fn tailor(fonts: &FontBook, text: &str, style: &TextStyle, max_width: f64) -> String {
    if !max_width.is_finite() || max_width <= 0.0 {
        return String::new();
    }
    if width(fonts, text, style) <= max_width + 0.01 {
        return text.to_owned();
    }
    let bounds: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    let candidate = |k: usize| -> String {
        let end = bounds.get(k).copied().unwrap_or(text.len());
        format!("{}{DOTS}", text.get(..end).unwrap_or("").trim_end())
    };
    let fits = |k: usize| width(fonts, &candidate(k), style) <= max_width + 0.01;
    if !fits(0) {
        return String::new();
    }
    let (mut lo, mut hi) = (0usize, bounds.len().saturating_sub(1));
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    candidate(lo)
}

/// Truncates a `head (inner)` label inside the parentheses, as the service
/// does for pie labels (`100 (25.0...)`); falls back to [`tailor`] when even
/// `head (...)` does not fit.
pub fn tailor_parenthesized(
    fonts: &FontBook,
    head: &str,
    inner: &str,
    style: &TextStyle,
    max_width: f64,
) -> String {
    let full = format!("{head} ({inner})");
    let fits = |t: &str| width(fonts, t, style) <= max_width + 0.01;
    if fits(&full) {
        return full;
    }
    let bounds: Vec<usize> = inner.char_indices().map(|(i, _)| i).collect();
    let candidate = |k: usize| -> String {
        let end = bounds.get(k).copied().unwrap_or(inner.len());
        let kept = inner.get(..end).unwrap_or("").trim_end();
        format!("{head} ({}{DOTS})", kept.trim_end_matches(['.', ',']))
    };
    // Keep at least one character of the parenthesized part, else show
    // the head alone.
    if !max_width.is_finite() || bounds.len() < 2 || !fits(&candidate(1)) {
        return if fits(head) {
            head.to_owned()
        } else {
            tailor(fonts, head, style, max_width)
        };
    }
    let (mut lo, mut hi) = (1usize, bounds.len().saturating_sub(1));
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if fits(&candidate(mid)) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    candidate(lo)
}

/// Baseline that vertically centres one line of `style` on `center_y`.
pub fn baseline_centered(style: &TextStyle, center_y: f64) -> f64 {
    center_y - style.line_height() / 2.0 + style.ascent()
}

/// A text line centred on `(cx, cy)` and rotated by `degrees` (clockwise;
/// `-90` reads bottom-to-top) about that point.
pub fn rotated_text(text: &str, style: &TextStyle, cx: f64, cy: f64, degrees: f64) -> Group {
    let mut g = Group::new().with_transform(Transform::rotate_about(degrees, cx, cy));
    g.push(
        TextNode::new(cx, baseline_centered(style, cy), text, style.clone())
            .anchor(TextAnchor::Middle),
    );
    g
}

/// A text line rotated by `degrees` about its anchor point `(x, y)`, where
/// `(x, y)` is the end of the text (`anchor` End) on its vertical centre
/// line: used for slanted category labels hanging below an axis.
pub fn slanted_text(text: &str, style: &TextStyle, x: f64, y: f64, degrees: f64) -> Group {
    let mut g = Group::new().with_transform(Transform::rotate_about(degrees, x, y));
    g.push(
        TextNode::new(x, baseline_centered(style, y), text, style.clone()).anchor(TextAnchor::End),
    );
    g
}

/// Pie/donut detail label contents (`labels.labelStyle`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LabelParts {
    pub category: bool,
    pub value: bool,
    pub percent: bool,
}

impl LabelParts {
    /// Parses Power BI's `labelStyle` (`Category`, `Data value`,
    /// `Percent of total`, `Category, data value`, `Category, percent of
    /// total`, `Data value, percent of total`, `All detail labels`).
    pub fn parse(text: Option<&str>) -> Self {
        let t = text
            .unwrap_or("Data value, percent of total")
            .to_ascii_lowercase();
        if t.contains("all") {
            return Self {
                category: true,
                value: true,
                percent: true,
            };
        }
        let parts = Self {
            category: t.contains("category"),
            value: t.contains("value"),
            percent: t.contains("percent"),
        };
        if parts.category || parts.value || parts.percent {
            parts
        } else {
            Self {
                category: false,
                value: true,
                percent: true,
            }
        }
    }
}

/// `labels` object settings.
#[derive(Debug, Clone, PartialEq)]
pub struct DataLabels {
    pub show: bool,
    pub style: TextStyle,
    /// Display unit; `None` is `Auto`.
    pub unit: Option<DisplayUnit>,
    /// Decimals; `None` is `Auto`.
    pub precision: Option<usize>,
    /// `labelPosition`, lower case (`outside`, `insideend`, …; empty when
    /// unset).
    pub position: String,
    /// Pie/donut label contents.
    pub parts: LabelParts,
    /// Decimals of percentages (`percentageLabelPrecision`, default 2).
    pub percent_decimals: usize,
}

impl DataLabels {
    /// Reads `labels` (`default_show` when `labels.show` is unset). The
    /// text uses the `smallLightLabel` class (9pt `#605E5C`).
    pub fn read(props: &VisualProps<'_>, default_show: bool) -> Self {
        let o = props.objects();
        let mut style = o.text_style("labels", "smallLightLabel");
        if let Some(c) = o.color("labels", "labelColor") {
            style.color = c;
        }
        let precision = o
            .number("labels", "labelPrecision")
            .filter(|p| *p >= 0.0)
            .map(|p| p.min(10.0) as usize);
        Self {
            show: o.bool("labels", "show").unwrap_or(default_show),
            style,
            unit: DisplayUnit::from_setting(o.number("labels", "labelDisplayUnits")),
            precision,
            position: o
                .text("labels", "labelPosition")
                .unwrap_or_default()
                .to_ascii_lowercase(),
            parts: LabelParts::parse(o.text("labels", "labelStyle").as_deref()),
            percent_decimals: o
                .number("labels", "percentageLabelPrecision")
                .filter(|p| *p >= 0.0)
                .map_or(2, |p| p.min(10.0) as usize),
        }
    }
}

/// `true` if white text reads better than dark text on `bg`.
pub fn prefers_white_text(bg: crate::scene::Color) -> bool {
    let l = 0.299 * f64::from(bg.r) + 0.587 * f64::from(bg.g) + 0.114 * f64::from(bg.b);
    l < 150.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_parts() {
        let p = LabelParts::parse(Some("Data value, percent of total"));
        assert!(p.value && p.percent && !p.category);
        assert!(LabelParts::parse(Some("All detail labels")).category);
        let c = LabelParts::parse(Some("Category"));
        assert!(c.category && !c.value && !c.percent);
        assert!(LabelParts::parse(Some("???")).value);
    }

    #[test]
    fn tailoring() {
        let fonts = FontBook::shared();
        let style = TextStyle::default().with_size_pt(9.0);
        let full = "100 (25.04%)";
        let w = width(fonts, full, &style);
        assert_eq!(tailor(fonts, full, &style, w + 1.0), full);
        let cut = tailor(fonts, full, &style, w - 5.0);
        assert!(cut.ends_with(DOTS) && cut.len() < full.len() + 3, "{cut}");
        assert!(width(fonts, &cut, &style) <= w - 5.0 + 0.01);
        assert_eq!(tailor(fonts, full, &style, 1.0), "");
        assert_eq!(tailor(fonts, full, &style, f64::NAN), "");
    }
}
