//! Textbox: `objects.general[0].properties.paragraphs[]` → rich text.
//! (owner: visuals-basics)
//!
//! # Model
//!
//! A paragraph has `textRuns[]` (`value`, optional `url`, `textStyle` with
//! `fontFamily`, `fontSize` (`"12pt"`, `"16px"` or a number of points),
//! `fontWeight` (`bold`, `normal`, `600`, …), `fontStyle` (`italic`),
//! `textDecoration` (`underline`, `line-through`) and `color` (CSS)),
//! `horizontalTextAlignment` (`left`/`center`/`right`; `justify` renders
//! left), `listType` (`bullet`, `number`/`ordered`) and `indentLevel`.
//! Runs without a style use Segoe UI 10pt in the theme foreground color;
//! runs with a `url` use the theme `hyperlink` color, underlined, unless
//! they set a color. Text is top-aligned (textboxes have no vertical
//! alignment).
//!
//! # Layout (matched to Power BI renders)
//!
//! - Lines wrap greedily at word boundaries across runs (long words break
//!   between glyphs); `\n` in a run is a hard line break. Paragraphs have no
//!   extra spacing.
//! - A line box is `line-height: normal` of every run on it **plus a strut**
//!   of Segoe UI 12px (the editor's paragraph font): ascent = max(run
//!   ascents, 12.95px), descent likewise, with per-family metrics
//!   ([`super::common::family_metrics`]). This reproduces the service's
//!   baselines to within a pixel (e.g. 9pt Calibri bullets on a 16px pitch,
//!   a 22pt spacer paragraph 39px tall).
//! - Lists follow the editor's CSS: text indented 1.5em (+3em per indent
//!   level), the marker (`•`, or `1.`, `2.`, …) right-aligned in a 1.2em box
//!   ending 0.3em before the text.
//! - When the text is taller than the content box, the service shows a
//!   scrollbar: an 8px `#C8C8C8` thumb at the right edge of the content box
//!   (at least 20px long, clamped to the box).

use std::collections::HashMap;

use pbi_model::Visual;
use serde_json::Value;

use crate::layout::TextAlign;
use crate::scene::{Color, Group, Paint, Rect, Stroke, TextAnchor};
use crate::text::{ASCENT_EM, DESCENT_EM, FontBook, TextStyle};
use crate::theme::ThemeResolver;

use super::VisualCtx;
use super::common::{adjusted_style, family_metrics, text_node, text_width};

/// Size of runs without a `fontSize`, in points.
pub const DEFAULT_FONT_SIZE_PT: f64 = 10.0;
/// Font size of the paragraph strut (Segoe UI), in pixels.
pub const STRUT_PX: f64 = 12.0;
/// Width of the overflow scrollbar thumb.
pub const SCROLLBAR_WIDTH: f64 = 8.0;
/// Color of the overflow scrollbar thumb.
pub const SCROLLBAR_COLOR: Color = Color::rgb(0xC8, 0xC8, 0xC8);
/// Paragraphs read at most (untrusted-input guard).
const MAX_PARAGRAPHS: usize = 2_000;
/// Runs read per paragraph at most.
const MAX_RUNS: usize = 2_000;
/// Lines laid out at most.
const MAX_LINES: usize = 5_000;
/// Characters read per textbox at most (longer text is cut).
const MAX_TEXT_CHARS: usize = crate::text::MAX_TEXT_CHARS;

/// List style of a paragraph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListKind {
    Bullet,
    Ordered,
}

/// One styled run of text.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub text: String,
    pub style: TextStyle,
    /// Hyperlink target (shown as styled text only).
    pub url: Option<String>,
    pub strikethrough: bool,
}

/// One paragraph of a textbox.
#[derive(Debug, Clone, PartialEq)]
pub struct Paragraph {
    pub runs: Vec<TextRun>,
    pub align: TextAlign,
    pub list: Option<ListKind>,
    pub indent: u32,
}

/// The style of runs without a `textStyle`.
pub fn default_style(theme: &ThemeResolver) -> TextStyle {
    TextStyle::new("Segoe UI", DEFAULT_FONT_SIZE_PT, theme.foreground())
}

/// Parses the visual's paragraphs (empty when there are none or the JSON is
/// malformed).
pub fn paragraphs(theme: &ThemeResolver, visual: &Visual) -> Vec<Paragraph> {
    let Some(list) = pbi_pbir::objects::property(&visual.objects, "general", "paragraphs")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let base = default_style(theme);
    let link_color = theme
        .named_color("hyperlink")
        .unwrap_or(Color::rgb(0x00, 0x78, 0xD4));
    let mut budget = MAX_TEXT_CHARS;
    let mut out: Vec<Paragraph> = Vec::new();
    for p in list.iter().take(MAX_PARAGRAPHS) {
        let Some(mut para) = paragraph(p, &base, link_color) else {
            continue;
        };
        for run in &mut para.runs {
            let n = run.text.chars().count();
            if n > budget {
                run.text = run.text.chars().take(budget).collect();
            }
            budget = budget.saturating_sub(n);
        }
        out.push(para);
        if budget == 0 {
            break;
        }
    }
    out
}

fn paragraph(p: &Value, base: &TextStyle, link_color: Color) -> Option<Paragraph> {
    let obj = p.as_object()?;
    let runs = obj
        .get("textRuns")
        .and_then(Value::as_array)
        .map(|runs| {
            runs.iter()
                .take(MAX_RUNS)
                .filter_map(|r| text_run(r, base, link_color))
                .collect()
        })
        .unwrap_or_default();
    let align = obj
        .get("horizontalTextAlignment")
        .and_then(Value::as_str)
        .map_or(TextAlign::Left, TextAlign::parse);
    let list = obj.get("listType").and_then(Value::as_str).and_then(|t| {
        match t.trim().to_ascii_lowercase().as_str() {
            "bullet" | "bullets" | "unordered" | "disc" => Some(ListKind::Bullet),
            "number" | "numbered" | "ordered" | "decimal" => Some(ListKind::Ordered),
            _ => None,
        }
    });
    let indent = ["indentLevel", "indent"]
        .iter()
        .find_map(|k| obj.get(*k).and_then(Value::as_f64))
        .filter(|n| n.is_finite())
        .map_or(0, |n| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let n = n.clamp(0.0, 8.0) as u32;
            n
        });
    Some(Paragraph {
        runs,
        align,
        list,
        indent,
    })
}

fn text_run(r: &Value, base: &TextStyle, link_color: Color) -> Option<TextRun> {
    let obj = r.as_object()?;
    let text = match obj.get("value") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => return None,
    };
    let url = obj
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(str::to_owned);
    let mut style = base.clone();
    let mut strikethrough = false;
    let mut color_set = false;
    if let Some(ts) = obj.get("textStyle").and_then(Value::as_object) {
        let text_of = |k: &str| ts.get(k).and_then(Value::as_str).map(str::trim);
        if let Some(f) = text_of("fontFamily").filter(|f| !f.is_empty()) {
            f.clone_into(&mut style.family);
        }
        if let Some(size) = ts.get("fontSize").and_then(parse_font_size) {
            style.size_pt = size;
        }
        if let Some(w) = text_of("fontWeight").and_then(parse_font_weight) {
            style.weight = w;
        }
        if let Some(s) = text_of("fontStyle") {
            let s = s.to_ascii_lowercase();
            style.italic = s.contains("italic") || s.contains("oblique");
        }
        if let Some(d) = text_of("textDecoration") {
            let d = d.to_ascii_lowercase();
            style.underline = d.contains("underline");
            strikethrough = d.contains("line-through") || d.contains("strikethrough");
        }
        if let Some(c) = text_of("color").and_then(super::common::parse_css_color) {
            style.color = c;
            color_set = true;
        }
    }
    if url.is_some() {
        style.underline = true;
        if !color_set {
            style.color = link_color;
        }
    }
    Some(TextRun {
        text,
        style,
        url,
        strikethrough,
    })
}

/// `"12pt"` → 12, `"16px"` → 12, `"9"`/`9` → 9 (points), clamped to
/// 1–400pt.
pub fn parse_font_size(v: &Value) -> Option<f64> {
    let pt = match v {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => {
            let s = s.trim().to_ascii_lowercase();
            if let Some(px) = s.strip_suffix("px") {
                px.trim().parse::<f64>().ok()? * 0.75
            } else {
                s.strip_suffix("pt")
                    .unwrap_or(&s)
                    .trim()
                    .parse::<f64>()
                    .ok()?
            }
        }
        _ => return None,
    };
    (pt.is_finite() && pt > 0.0).then(|| pt.clamp(1.0, 400.0))
}

/// CSS `font-weight` → numeric weight.
pub fn parse_font_weight(w: &str) -> Option<u16> {
    match w.trim().to_ascii_lowercase().as_str() {
        "bold" | "bolder" => Some(700),
        "normal" | "regular" => Some(400),
        "lighter" | "light" => Some(300),
        "semibold" => Some(600),
        n => n.parse::<f64>().ok().filter(|n| n.is_finite()).map(|n| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let w = n.clamp(100.0, 900.0) as u16;
            w
        }),
    }
}

/// One positioned piece of a line (a single run).
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// Index into the paragraph's runs.
    pub run: usize,
    pub text: String,
    /// Offset from the line's start.
    pub x: f64,
    pub width: f64,
}

/// One laid-out line.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLine {
    pub paragraph: usize,
    /// First line of its paragraph (carries the list marker).
    pub first: bool,
    pub segments: Vec<Segment>,
    /// Width of the visible text.
    pub width: f64,
    /// Line top relative to the layout top.
    pub top: f64,
    pub ascent: f64,
    pub descent: f64,
    /// Left edge of the text area of the line (list / indent offset).
    pub indent: f64,
    /// Available width after the indent.
    pub avail: f64,
}

impl TextLine {
    pub fn height(&self) -> f64 {
        self.ascent + self.descent
    }

    pub fn baseline(&self) -> f64 {
        self.top + self.ascent
    }
}

/// Laid-out paragraphs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextboxLayout {
    pub lines: Vec<TextLine>,
    pub height: f64,
}

/// A word with the whitespace after it.
struct Token {
    run: usize,
    word: String,
    space: String,
    word_w: f64,
    space_w: f64,
    hard_break: bool,
}

struct SpaceCache<'a> {
    fonts: &'a FontBook,
    widths: HashMap<(String, u64, u16, bool), f64>,
}

impl SpaceCache<'_> {
    fn space(&mut self, style: &TextStyle) -> f64 {
        let key = (
            style.family.clone(),
            style.size_pt.to_bits(),
            style.weight,
            style.italic,
        );
        let fonts = self.fonts;
        *self.widths.entry(key).or_insert_with(|| {
            (text_width(fonts, "a a", style) - text_width(fonts, "aa", style)).max(0.0)
        })
    }
}

fn tokenize(paragraph: &Paragraph, spaces: &mut SpaceCache<'_>) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::new();
    for (ri, run) in paragraph.runs.iter().enumerate() {
        let text = run.text.replace("\r\n", "\n").replace('\t', "    ");
        let space_w = spaces.space(&run.style);
        let mut parts = text.split('\n').peekable();
        while let Some(part) = parts.next() {
            let mut word = String::new();
            let mut space = String::new();
            for ch in part.chars() {
                if ch.is_whitespace() {
                    space.push(ch);
                } else {
                    if !space.is_empty() {
                        out.push(token(spaces.fonts, ri, run, &word, &space, space_w));
                        word.clear();
                        space.clear();
                    }
                    word.push(ch);
                }
            }
            if !word.is_empty() || !space.is_empty() {
                out.push(token(spaces.fonts, ri, run, &word, &space, space_w));
            }
            if parts.peek().is_some() {
                if let Some(last) = out.last_mut().filter(|t| t.run == ri) {
                    last.hard_break = true;
                } else {
                    out.push(token(spaces.fonts, ri, run, "", "", space_w));
                    if let Some(last) = out.last_mut() {
                        last.hard_break = true;
                    }
                }
            }
        }
    }
    out
}

fn token(fonts: &FontBook, run: usize, r: &TextRun, word: &str, space: &str, sw: f64) -> Token {
    Token {
        run,
        word: word.to_owned(),
        space: space.to_owned(),
        word_w: if word.is_empty() {
            0.0
        } else {
            text_width(fonts, word, &r.style)
        },
        space_w: sw * space.chars().count() as f64,
        hard_break: false,
    }
}

struct LineBuilder {
    segments: Vec<Segment>,
    runs: Vec<usize>,
    x: f64,
    visible_w: f64,
}

impl LineBuilder {
    fn new() -> Self {
        Self {
            segments: Vec::new(),
            runs: Vec::new(),
            x: 0.0,
            visible_w: 0.0,
        }
    }

    fn is_empty(&self) -> bool {
        self.segments.is_empty() && self.runs.is_empty()
    }

    fn push(&mut self, run: usize, text: &str, width: f64, space: &str, space_w: f64) {
        if !self.runs.contains(&run) {
            self.runs.push(run);
        }
        let joined = format!("{text}{space}");
        match self.segments.last_mut() {
            Some(seg) if seg.run == run => {
                seg.text.push_str(&joined);
                seg.width = self.x + width - seg.x;
            }
            _ => self.segments.push(Segment {
                run,
                text: joined,
                x: self.x,
                width,
            }),
        }
        if !text.is_empty() {
            self.visible_w = self.x + width;
        }
        self.x += width + space_w;
    }
}

/// Lays out `paragraphs` in a box `width` px wide (top-aligned).
pub fn layout(fonts: &FontBook, paragraphs: &[Paragraph], width: f64) -> TextboxLayout {
    let strut = TextStyle::new("Segoe UI", STRUT_PX * 0.75, Color::BLACK);
    let strut_ascent = strut.size_px() * ASCENT_EM;
    let strut_descent = strut.size_px() * DESCENT_EM;
    let default = TextStyle::new("Segoe UI", DEFAULT_FONT_SIZE_PT, Color::BLACK);
    let mut spaces = SpaceCache {
        fonts,
        widths: HashMap::new(),
    };
    let mut out = TextboxLayout::default();
    let mut top = 0.0;
    for (pi, p) in paragraphs.iter().enumerate() {
        let first_style = p
            .runs
            .iter()
            .find(|r| !r.text.trim().is_empty())
            .or(p.runs.first())
            .map_or(&default, |r| &r.style);
        let em = first_style.size_px();
        let indent_em = match p.list {
            Some(_) => 1.5 + 3.0 * f64::from(p.indent),
            None => 3.0 * f64::from(p.indent),
        };
        let indent = (indent_em * em).min(width * 0.9).max(0.0);
        let avail = (width - indent).max(1.0);
        let tokens = tokenize(p, &mut spaces);
        let mut lines: Vec<LineBuilder> = Vec::new();
        let mut cur = LineBuilder::new();
        for t in tokens {
            let style = p.runs.get(t.run).map_or(&default, |r| &r.style);
            if !cur.is_empty() && !t.word.is_empty() && cur.x + t.word_w > avail + 0.01 {
                lines.push(std::mem::replace(&mut cur, LineBuilder::new()));
            }
            if t.word_w > avail + 0.01 {
                // A word wider than the line: break it between glyphs.
                let f = family_metrics(style).width_factor;
                let chunks: Vec<String> = fonts
                    .measure(&t.word, &adjusted_style(style), Some(avail / f))
                    .lines
                    .into_iter()
                    .map(|l| l.text)
                    .collect();
                let n = chunks.len();
                for (i, chunk) in chunks.into_iter().enumerate() {
                    let w = text_width(fonts, &chunk, style);
                    if i + 1 < n {
                        cur.push(t.run, &chunk, w, "", 0.0);
                        lines.push(std::mem::replace(&mut cur, LineBuilder::new()));
                    } else {
                        cur.push(t.run, &chunk, w, &t.space, t.space_w);
                    }
                }
            } else {
                cur.push(t.run, &t.word, t.word_w, &t.space, t.space_w);
            }
            if t.hard_break {
                lines.push(std::mem::replace(&mut cur, LineBuilder::new()));
            }
            if lines.len() + out.lines.len() >= MAX_LINES {
                break;
            }
        }
        if !cur.is_empty() || lines.is_empty() {
            lines.push(cur);
        }
        for (li, l) in lines.into_iter().enumerate() {
            let mut ascent = strut_ascent;
            let mut descent = strut_descent;
            let styles: Vec<&TextStyle> = if l.runs.is_empty() {
                vec![first_style]
            } else {
                l.runs
                    .iter()
                    .filter_map(|r| p.runs.get(*r).map(|r| &r.style))
                    .collect()
            };
            for s in styles {
                let m = family_metrics(s);
                ascent = ascent.max(m.ascent_px(s));
                descent = descent.max(m.descent_px(s));
            }
            let mut segments = l.segments;
            for s in &mut segments {
                let trimmed = s.text.trim_end().len();
                s.text.truncate(trimmed);
            }
            segments.retain(|s| !s.text.is_empty());
            out.lines.push(TextLine {
                paragraph: pi,
                first: li == 0,
                segments,
                width: l.visible_w,
                top,
                ascent,
                descent,
                indent,
                avail,
            });
            top += ascent + descent;
        }
        if out.lines.len() >= MAX_LINES {
            break;
        }
    }
    out.height = top;
    out
}

/// Renders the textbox content (see the module docs).
pub fn render(ctx: &VisualCtx<'_>) -> Option<Group> {
    let paras = paragraphs(ctx.theme, ctx.visual);
    let c = ctx.frame.content;
    if paras.is_empty() || c.width <= 0.0 || c.height <= 0.0 {
        return None;
    }
    let laid = layout(ctx.fonts, &paras, c.width);
    let mut g = Group::new();
    let mut counters: HashMap<u32, usize> = HashMap::new();
    for line in &laid.lines {
        if line.top > c.height {
            break;
        }
        let Some(p) = paras.get(line.paragraph) else {
            continue;
        };
        let baseline = c.y + line.baseline();
        if line.first {
            list_marker(&mut g, p, line, c, baseline, &mut counters);
        }
        let x0 = c.x
            + line.indent
            + match p.align {
                TextAlign::Left => 0.0,
                TextAlign::Center => (line.avail - line.width) / 2.0,
                TextAlign::Right => line.avail - line.width,
            };
        for seg in &line.segments {
            let Some(run) = p.runs.get(seg.run) else {
                continue;
            };
            let x = x0 + seg.x;
            g.push(text_node(
                x,
                baseline,
                seg.text.clone(),
                &run.style,
                TextAnchor::Start,
                None,
            ));
            if run.strikethrough {
                let y = baseline - run.style.size_px() * 0.3;
                let w = (run.style.size_px() / 14.0).max(1.0);
                g.line(x, y, x + seg.width, y, Stroke::new(run.style.color, w));
            }
        }
    }
    if laid.height > c.height + 0.5 {
        let track = c.height;
        let thumb = (track * c.height / laid.height).max(track.min(20.0));
        let w = SCROLLBAR_WIDTH.min(c.width);
        g.rect(
            Rect::new(c.right() - w, c.y, w, thumb),
            Some(Paint::solid(SCROLLBAR_COLOR)),
            None,
        );
    }
    (!g.is_empty()).then_some(g)
}

fn list_marker(
    g: &mut Group,
    p: &Paragraph,
    line: &TextLine,
    c: Rect,
    baseline: f64,
    counters: &mut HashMap<u32, usize>,
) {
    let Some(kind) = p.list else {
        counters.clear();
        return;
    };
    let style = p
        .runs
        .iter()
        .find(|r| !r.text.trim().is_empty())
        .or(p.runs.first())
        .map(|r| {
            let mut s = r.style.clone();
            s.underline = false;
            s
        });
    let Some(style) = style else {
        return;
    };
    let text = match kind {
        ListKind::Bullet => {
            counters.remove(&p.indent);
            "•".to_owned()
        }
        ListKind::Ordered => {
            counters.retain(|level, _| *level <= p.indent);
            let n = counters.entry(p.indent).or_insert(0);
            *n += 1;
            format!("{n}.")
        }
    };
    let x = c.x + line.indent - 0.3 * style.size_px();
    g.push(text_node(x, baseline, text, &style, TextAnchor::End, None));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn font_sizes_and_weights() {
        assert_eq!(parse_font_size(&json!("22pt")), Some(22.0));
        assert_eq!(parse_font_size(&json!("16px")), Some(12.0));
        assert_eq!(parse_font_size(&json!(9)), Some(9.0));
        assert_eq!(parse_font_size(&json!("big")), None);
        assert_eq!(parse_font_size(&json!("-3pt")), None);
        assert_eq!(parse_font_size(&json!("1e9pt")), Some(400.0));
        assert_eq!(parse_font_weight("bold"), Some(700));
        assert_eq!(parse_font_weight("600"), Some(600));
        assert_eq!(parse_font_weight("2000"), Some(900));
        assert_eq!(parse_font_weight("heavy-ish"), None);
    }
}
