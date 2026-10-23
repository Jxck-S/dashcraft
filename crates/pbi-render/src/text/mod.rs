//! Text styles, shaping and measurement with the bundled fonts (Selawik for
//! Segoe UI and most families, Carlito for Calibri).
//!
//! - [`TextStyle`]: Power BI font family (name or CSS list), size in points,
//!   color, CSS weight, italic/underline.
//! - [`FontBook`]: a `cosmic-text` font system loaded **only** with the
//!   bundled faces ([`fonts`]), so measurement is deterministic across OSes.
//!   Each style is shaped with its [`BundledFace`] ([`resolve_face_with`]).
//!   [`FontBook::measure`] wraps (word, then glyph) to a width;
//!   [`FontBook::measure_with`] adds a line limit and an ellipsis;
//!   [`FontBook::ellipsize`] truncates one line with `…`.
//!
//! # Units and vertical metrics
//!
//! Power BI font sizes are points: `px = pt · 96/72` ([`PX_PER_PT`]). Line
//! height and baseline follow Segoe UI's (and Selawik's) OS/2 Windows
//! metrics, which browsers use for `line-height: normal`: line height
//! `= 2724/2048 em` ([`LINE_HEIGHT_EM`]), baseline `2210/2048 em` below the
//! line top ([`ASCENT_EM`]). [`Line::baseline`] is relative to the layout's
//! top edge. Visuals that lay out Calibri text use Carlito's (Calibri's)
//! own metrics through [`crate::visuals::common::family_metrics`].

pub mod fonts;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style, Weight, Wrap};
use serde::{Deserialize, Serialize};

use crate::scene::{Color, TextAnchor, TextNode};

pub use fonts::{BundledFace, CARLITO_FAMILY, FAMILY, is_calibri, resolve_face, resolve_face_with};

/// CSS pixels per typographic point.
pub const PX_PER_PT: f64 = 96.0 / 72.0;
/// Line height in em (Segoe UI / Selawik `usWinAscent + usWinDescent`).
pub const LINE_HEIGHT_EM: f64 = (2210.0 + 514.0) / 2048.0;
/// Baseline offset from the line top, in em (`usWinAscent`).
pub const ASCENT_EM: f64 = 2210.0 / 2048.0;
/// Descent below the baseline, in em (`usWinDescent`).
pub const DESCENT_EM: f64 = 514.0 / 2048.0;
/// The ellipsis appended by [`FontBook::ellipsize`].
pub const ELLIPSIS: &str = "…";
/// Longest text measured; longer input is cut (untrusted-input guard).
pub const MAX_TEXT_CHARS: usize = 20_000;

fn normal_weight() -> u16 {
    400
}

fn is_normal_weight(w: &u16) -> bool {
    *w == 400
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A resolved text style.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextStyle {
    /// Power BI family name or CSS family list (`Segoe UI`, `DIN`,
    /// `'Segoe UI Semibold', wf_segoe-ui_semibold, helvetica`). Rendered with
    /// the bundled face from [`resolve_face`].
    pub family: String,
    /// Size in points.
    pub size_pt: f64,
    pub color: Color,
    /// CSS weight (400 normal, 700 bold).
    #[serde(default = "normal_weight", skip_serializing_if = "is_normal_weight")]
    pub weight: u16,
    #[serde(default, skip_serializing_if = "is_false")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub underline: bool,
}

impl Default for TextStyle {
    /// Power BI's `label` text class: Segoe UI 10pt `#252423`.
    fn default() -> Self {
        Self {
            family: "Segoe UI".to_owned(),
            size_pt: 10.0,
            color: Color::rgb(0x25, 0x24, 0x23),
            weight: 400,
            italic: false,
            underline: false,
        }
    }
}

impl TextStyle {
    pub fn new(family: impl Into<String>, size_pt: f64, color: Color) -> Self {
        Self {
            family: family.into(),
            size_pt,
            color,
            ..Self::default()
        }
    }

    pub fn with_weight(mut self, weight: u16) -> Self {
        self.weight = weight;
        self
    }

    pub fn bold(self) -> Self {
        self.with_weight(700)
    }

    pub fn with_size_pt(mut self, size_pt: f64) -> Self {
        self.size_pt = size_pt;
        self
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn with_family(mut self, family: impl Into<String>) -> Self {
        self.family = family.into();
        self
    }

    /// Font size in CSS pixels (non-finite or negative sizes become 0).
    pub fn size_px(&self) -> f64 {
        let px = self.size_pt * PX_PER_PT;
        if px.is_finite() { px.max(0.0) } else { 0.0 }
    }

    /// Line height in pixels.
    pub fn line_height(&self) -> f64 {
        self.size_px() * LINE_HEIGHT_EM
    }

    /// Baseline offset from the top of a line, in pixels.
    pub fn ascent(&self) -> f64 {
        self.size_px() * ASCENT_EM
    }

    /// The bundled face this style renders with.
    pub fn face(&self) -> BundledFace {
        resolve_face_with(&self.family, self.weight, self.italic)
    }
}

/// One laid-out line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    /// Visible text (trailing whitespace removed; may end with `…`).
    pub text: String,
    /// Advance width of the visible text, in pixels.
    pub width: f64,
    /// Top of the line box, relative to the layout top.
    pub top: f64,
    /// Baseline, relative to the layout top.
    pub baseline: f64,
}

/// Result of [`FontBook::measure`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextLayout {
    pub lines: Vec<Line>,
    /// Widest line.
    pub width: f64,
    /// `lines.len() × line_height`.
    pub height: f64,
    pub line_height: f64,
    /// `true` if lines were dropped or ellipsized.
    pub truncated: bool,
}

impl TextLayout {
    /// One [`TextNode`] per non-empty line, with the layout's top at `top`.
    /// `x` is the left edge for [`TextAnchor::Start`], the centre for
    /// `Middle` and the right edge for `End`.
    pub fn to_nodes(
        &self,
        x: f64,
        top: f64,
        anchor: TextAnchor,
        style: &TextStyle,
    ) -> Vec<TextNode> {
        self.lines
            .iter()
            .filter(|l| !l.text.is_empty())
            .map(|l| {
                TextNode::new(x, top + l.baseline, l.text.clone(), style.clone()).anchor(anchor)
            })
            .collect()
    }
}

/// Options for [`FontBook::measure_with`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MeasureOptions {
    /// Available width in pixels (`None`: unlimited).
    pub max_width: Option<f64>,
    /// Wrap at `max_width` (word, then glyph). Without wrapping each hard
    /// line is one line.
    pub wrap: bool,
    /// Keep at most this many lines.
    pub max_lines: Option<usize>,
    /// End truncated lines (too wide, or the last kept line when lines were
    /// dropped) with `…`.
    pub ellipsis: bool,
}

/// Shapes and measures text with the bundled fonts. Use
/// [`FontBook::shared`] rather than building one per call.
pub struct FontBook {
    system: Mutex<FontSystem>,
    /// Memoized [`FontBook::shape_lines`] results. Shaping is the dominant
    /// cost of re-rendering a page (scrolling a table re-lays out every
    /// row), and its result depends only on the key, so cached and fresh
    /// results are identical. Cleared when it reaches [`SHAPE_CACHE_MAX`].
    shaped: Mutex<ShapeCache>,
}

/// Shaped lines (text, advance width) by [`ShapeKey`].
type ShapeCache = HashMap<ShapeKey, Arc<[(String, f64)]>>;

/// Entries kept in the shaping cache before it is cleared.
const SHAPE_CACHE_MAX: usize = 200_000;

/// Everything [`FontBook::shape_lines`] depends on.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ShapeKey {
    text: Box<str>,
    px_bits: u64,
    /// Family and weight determine the bundled face; keyed directly so a
    /// cache hit skips resolving the face (CSS family-list parsing).
    family: Box<str>,
    weight: u16,
    italic: bool,
    wrap_bits: Option<u64>,
}

impl std::fmt::Debug for FontBook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontBook")
            .field("family", &FAMILY)
            .finish_non_exhaustive()
    }
}

impl Default for FontBook {
    fn default() -> Self {
        Self::new()
    }
}

impl FontBook {
    /// A font book with only the bundled faces.
    pub fn new() -> Self {
        let system =
            FontSystem::new_with_locale_and_db("en-US".to_owned(), fonts::cosmic_database());
        Self {
            system: Mutex::new(system),
            shaped: Mutex::new(HashMap::new()),
        }
    }

    /// Process-wide instance.
    pub fn shared() -> &'static FontBook {
        static BOOK: OnceLock<FontBook> = OnceLock::new();
        BOOK.get_or_init(FontBook::new)
    }

    /// Lays out `text`, wrapping at `max_width` when given (no line limit,
    /// no ellipsis).
    pub fn measure(&self, text: &str, style: &TextStyle, max_width: Option<f64>) -> TextLayout {
        self.measure_with(
            text,
            style,
            &MeasureOptions {
                max_width,
                wrap: max_width.is_some(),
                ..MeasureOptions::default()
            },
        )
    }

    /// Lays out `text` with wrapping, a line limit and ellipsis.
    pub fn measure_with(&self, text: &str, style: &TextStyle, opts: &MeasureOptions) -> TextLayout {
        let line_height = style.line_height();
        let ascent = style.ascent();
        let max_width = opts.max_width.filter(|w| w.is_finite()).map(|w| w.max(0.0));
        let wrap_width = max_width.filter(|_| opts.wrap);
        let mut raw = self.shape_lines(text, style, wrap_width);
        let mut truncated = false;
        if let Some(max) = opts.max_lines
            && raw.len() > max
        {
            truncated = true;
            raw.truncate(max);
            if opts.ellipsis
                && let (Some(last), Some(mw)) = (raw.last_mut(), max_width)
            {
                let text = self.ellipsize_forced(&last.0, style, mw);
                let width = self.text_width(&text, style);
                *last = (text, width);
            }
        }
        if opts.ellipsis
            && let Some(mw) = max_width
        {
            for line in &mut raw {
                if line.1 > mw + WIDTH_EPSILON {
                    truncated = true;
                    let text = self.ellipsize(&line.0, style, mw);
                    let width = self.text_width(&text, style);
                    *line = (text, width);
                }
            }
        }
        let mut top = 0.0;
        let lines: Vec<Line> = raw
            .into_iter()
            .map(|(text, width)| {
                let line = Line {
                    text,
                    width,
                    top,
                    baseline: top + ascent,
                };
                top += line_height;
                line
            })
            .collect();
        TextLayout {
            width: lines.iter().map(|l| l.width).fold(0.0, f64::max),
            height: top,
            line_height,
            truncated,
            lines,
        }
    }

    /// Advance width of `text` on one line (hard line breaks: widest line).
    pub fn text_width(&self, text: &str, style: &TextStyle) -> f64 {
        self.shape_lines(text, style, None)
            .iter()
            .map(|l| l.1)
            .fold(0.0, f64::max)
    }

    /// `text` if it fits in `max_width`, else its longest prefix that fits
    /// with `…` appended (just `…`, or empty if even that does not fit).
    pub fn ellipsize(&self, text: &str, style: &TextStyle, max_width: f64) -> String {
        if self.text_width(text, style) <= max_width + WIDTH_EPSILON {
            return text.to_owned();
        }
        self.ellipsize_forced(text, style, max_width)
    }

    /// Longest prefix of `text` followed by `…` that fits `max_width`.
    fn ellipsize_forced(&self, text: &str, style: &TextStyle, max_width: f64) -> String {
        let text = text.trim_end_matches(ELLIPSIS);
        let bounds: Vec<usize> = text
            .char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(text.len()))
            .collect();
        let candidate = |k: usize| -> String {
            let end = bounds.get(k).copied().unwrap_or(text.len());
            let prefix = text.get(..end).unwrap_or("").trim_end();
            format!("{prefix}{ELLIPSIS}")
        };
        let fits = |k: usize| self.text_width(&candidate(k), style) <= max_width + WIDTH_EPSILON;
        if !fits(0) {
            return String::new();
        }
        // Largest prefix length that fits (widths grow with the prefix).
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

    /// Shapes `text` into `(visible text, width)` lines.
    fn shape_lines(
        &self,
        text: &str,
        style: &TextStyle,
        wrap_width: Option<f64>,
    ) -> Vec<(String, f64)> {
        let px = style.size_px();
        if text.is_empty() || px <= 0.0 {
            return Vec::new();
        }
        let text = match text.char_indices().nth(MAX_TEXT_CHARS) {
            Some((i, _)) => text.get(..i).unwrap_or(text),
            None => text,
        };
        let key = ShapeKey {
            text: text.into(),
            px_bits: px.to_bits(),
            family: style.family.as_str().into(),
            weight: style.weight,
            italic: style.italic,
            wrap_bits: wrap_width.map(f64::to_bits),
        };
        if let Some(hit) = self
            .shaped
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
        {
            return hit.to_vec();
        }
        let lines = self.shape_uncached(text, style, px, wrap_width);
        let mut cache = self.shaped.lock().unwrap_or_else(PoisonError::into_inner);
        if cache.len() >= SHAPE_CACHE_MAX {
            cache.clear();
        }
        cache.insert(key, Arc::from(lines.as_slice()));
        lines
    }

    /// Shapes `text` (non-empty, already capped) with cosmic-text.
    fn shape_uncached(
        &self,
        text: &str,
        style: &TextStyle,
        px: f64,
        wrap_width: Option<f64>,
    ) -> Vec<(String, f64)> {
        let mut system = self.system.lock().unwrap_or_else(PoisonError::into_inner);
        #[allow(clippy::cast_possible_truncation)]
        let metrics = Metrics::new(px as f32, (px * LINE_HEIGHT_EM) as f32);
        let mut buffer = Buffer::new(&mut system, metrics);
        let mut b = buffer.borrow_with(&mut system);
        b.set_wrap(if wrap_width.is_some() {
            Wrap::WordOrGlyph
        } else {
            Wrap::None
        });
        #[allow(clippy::cast_possible_truncation)]
        b.set_size(wrap_width.map(|w| w as f32), None);
        let face = style.face();
        let attrs = Attrs::new()
            .family(Family::Name(face.family()))
            .weight(Weight(face.weight()))
            .style(if style.italic {
                Style::Italic
            } else {
                Style::Normal
            });
        b.set_text(text, &attrs, Shaping::Advanced, None);
        b.shape_until_scroll(false);
        b.layout_runs()
            .map(|run| {
                let mut start = usize::MAX;
                let mut end = 0usize;
                let mut right = 0f32;
                for g in run.glyphs {
                    let visible = run
                        .text
                        .get(g.start..g.end)
                        .is_some_and(|s| !s.chars().all(char::is_whitespace));
                    if visible {
                        right = right.max(g.x + g.w);
                    }
                    start = start.min(g.start);
                    end = end.max(g.end);
                }
                let line = if start < end {
                    run.text.get(start..end).unwrap_or("")
                } else {
                    ""
                };
                (line.trim_end().to_owned(), f64::from(right))
            })
            .collect()
    }
}

/// Tolerance when comparing measured widths with an available width.
const WIDTH_EPSILON: f64 = 0.01;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units() {
        let s = TextStyle::default().with_size_pt(12.0);
        assert!((s.size_px() - 16.0).abs() < 1e-9);
        assert!((s.line_height() - 16.0 * LINE_HEIGHT_EM).abs() < 1e-9);
        assert_eq!(TextStyle::default().with_size_pt(f64::NAN).size_px(), 0.0);
    }

    #[test]
    fn empty_and_zero_size_text_has_no_lines() {
        let book = FontBook::shared();
        assert!(
            book.measure("", &TextStyle::default(), None)
                .lines
                .is_empty()
        );
        let zero = TextStyle::default().with_size_pt(0.0);
        assert!(book.measure("abc", &zero, None).lines.is_empty());
    }
}
