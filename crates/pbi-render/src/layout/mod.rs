//! Page layout: page size and scaling (`FitToPage`, `FitToWidth`,
//! `ActualSize`), paint order (z, then tab order), visual groups and each
//! visual's container chrome from `visualContainerObjects` (background,
//! border, rounded corners, drop shadow, padding, title, visual header).
//!
//! # Coordinates
//!
//! [`VisualFrame`] rectangles are in **page units** (the PBIR `position`
//! space, e.g. 1280×720). [`PageLayout::page_transform`] maps page units to
//! the canvas (`translate(offset) scale(scale)`); scenes apply it on a root
//! group, so visuals always draw in page units.
//!
//! # Container model
//!
//! - `outer` is the visual position (group offsets applied, see below).
//! - The border (if shown) is drawn inside `outer`; `inner` is `outer` inset
//!   by the border width.
//! - The title band starts at the top of `inner`, inset horizontally by the
//!   padding; its height is the laid-out title lines (wrapped when
//!   `titleWrap`, else one ellipsized line). Titles use the `title` text
//!   class (12pt `DIN` in Power BI's default theme) unless the theme
//!   defines `largeTitle`; the auto title is
//!   [`auto_title`].
//! - `content` is `inner` minus the padding, starting below the title when
//!   there is one. This matches the service: with a 1px border and the
//!   default 5px padding, a table's header starts 6px inside the visual and
//!   a chart title's cap height sits ~5px below the top edge.
//!
//! # Groups
//!
//! Visuals with a `parentGroupName` are positioned absolutely when they fit
//! inside the (resolved) group rectangle, otherwise relative to it (both
//! conventions occur in PBIR exports). Hidden groups hide their children.
//! A group paints before its first child.

pub mod frames;

use std::collections::HashMap;

use pbi_model::{DisplayOption, Page, Report, Visual};
use serde::{Deserialize, Serialize};

use crate::scene::{Color, Insets, Paint, Rect, Stroke, Transform};
use crate::text::{FontBook, Line, MeasureOptions, TextStyle};
use crate::theme::{ThemeResolver, as_text};

pub use frames::{FrameOptions, frame_group, render_page_frames, render_page_with};

/// Default page size when a page has none (or an invalid one).
pub const DEFAULT_PAGE_SIZE: (f64, f64) = (1280.0, 720.0);
/// Largest page dimension honored (untrusted-input guard).
pub const MAX_PAGE_DIMENSION: f64 = 100_000.0;
/// Deepest group nesting followed.
const MAX_GROUP_DEPTH: usize = 32;

/// Vertical placement of a page shorter than the viewport (report
/// `section.verticalAlignment`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VerticalAlignment {
    Top,
    #[default]
    Middle,
}

/// Options for [`PageLayout::compute_with`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LayoutOptions {
    /// Viewport `(width, height)` in CSS pixels; `None` renders the page at
    /// its own size (scale 1, no offset).
    pub viewport: Option<(f64, f64)>,
    pub vertical_alignment: VerticalAlignment,
    /// Keep hidden visuals (with [`VisualFrame::hidden`] set) instead of
    /// skipping them.
    pub include_hidden: bool,
}

impl LayoutOptions {
    /// Options with the report's canvas vertical alignment.
    pub fn for_report(report: &Report, viewport: Option<(f64, f64)>) -> Self {
        Self {
            viewport,
            vertical_alignment: report_vertical_alignment(report),
            include_hidden: false,
        }
    }
}

/// Report `objects.section[0].verticalAlignment` (`'Top'`/`'Middle'`).
pub fn report_vertical_alignment(report: &Report) -> VerticalAlignment {
    let text = pbi_pbir::objects::property(&report.objects, "section", "verticalAlignment")
        .and_then(as_text);
    match text.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("top") => VerticalAlignment::Top,
        _ => VerticalAlignment::Middle,
    }
}

/// Title alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

impl TextAlign {
    /// Parses `left`/`center`/`centre`/`right` (case-insensitive); anything
    /// else is `Left`.
    pub fn parse(text: &str) -> Self {
        match text.trim().to_ascii_lowercase().as_str() {
            "center" | "centre" | "middle" => TextAlign::Center,
            "right" | "end" => TextAlign::Right,
            _ => TextAlign::Left,
        }
    }
}

/// A laid-out container title.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleBox {
    /// The title band (page units).
    pub rect: Rect,
    pub text: String,
    pub style: TextStyle,
    pub alignment: TextAlign,
    pub wrap: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<Paint>,
    /// Lines relative to `rect` (top-left), already wrapped/ellipsized.
    pub lines: Vec<Line>,
}

/// A container border.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Border {
    pub color: Color,
    pub width: f64,
}

/// A drop shadow (`dropShadow`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DropShadow {
    pub color: Color,
    /// 0–1 (from `transparency`).
    pub opacity: f64,
    /// Offset `(dx, dy)` from `preset`/`angle` and `shadowDistance`.
    pub offset: (f64, f64),
    pub blur: f64,
    pub spread: f64,
    /// `position: 'Inner'` (not drawn by the frame renderer).
    pub inner: bool,
}

/// The visual header (hover icons; not drawn in static renders).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualHeader {
    pub show: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<Color>,
}

/// One visual container, laid out in page units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualFrame {
    pub name: String,
    /// `visualType`, or `group` for visual groups.
    pub visual_type: String,
    pub is_group: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_group: Option<String>,
    pub z: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_order: Option<i64>,
    /// Container rectangle (group offsets applied).
    pub outer: Rect,
    /// `outer` inside the border.
    pub inner: Rect,
    /// Drawing area for the visual (after border, title and padding).
    pub content: Rect,
    pub padding: Insets,
    /// Corner radius of background, border and clip (`border.radius`).
    pub radius: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<TitleBox>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<Paint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<Border>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drop_shadow: Option<DropShadow>,
    pub header: VisualHeader,
    pub hidden: bool,
}

impl VisualFrame {
    /// The border as a stroke centred on the inside of `outer`.
    pub fn border_stroke(&self) -> Option<(Rect, Stroke)> {
        let b = self.border.as_ref()?;
        let rect = self.outer.inset_uniform(b.width / 2.0);
        Some((rect, Stroke::new(b.color, b.width)))
    }
}

/// A page laid out for a viewport.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageLayout {
    pub page_name: String,
    /// Page `(width, height)` in page units.
    pub page_size: (f64, f64),
    /// Canvas `(width, height)` in CSS pixels (the scene size).
    pub canvas_size: (f64, f64),
    /// Page units → canvas pixels.
    pub scale: f64,
    /// Canvas position of the page's top-left corner.
    pub offset: (f64, f64),
    pub display_option: DisplayOption,
    /// Frames in paint order (back to front).
    pub visuals: Vec<VisualFrame>,
}

impl PageLayout {
    /// Lays out `page` with the built-in theme defaults (no report theme).
    pub fn compute(page: &Page, viewport: Option<(f64, f64)>) -> Self {
        Self::compute_with(
            page,
            &ThemeResolver::default(),
            &LayoutOptions {
                viewport,
                ..LayoutOptions::default()
            },
        )
    }

    /// Lays out `page` with a theme and options.
    pub fn compute_with(page: &Page, theme: &ThemeResolver, opts: &LayoutOptions) -> Self {
        let page_size = sanitize_page_size(page.width, page.height);
        let (scale, offset, canvas_size) = scaling(
            &page.display_option,
            page_size,
            opts.viewport,
            opts.vertical_alignment,
        );
        let positions = resolve_positions(page);
        let hidden = resolve_hidden(page);
        let mut order: Vec<(usize, &Visual)> = page.visuals.iter().enumerate().collect();
        let keys: HashMap<&str, f64> = paint_z(page);
        order.sort_by(|(ia, a), (ib, b)| {
            let za = keys.get(a.name.as_str()).copied().unwrap_or(a.position.z);
            let zb = keys.get(b.name.as_str()).copied().unwrap_or(b.position.z);
            za.total_cmp(&zb)
                .then_with(|| b.is_group().cmp(&a.is_group()))
                .then_with(|| tab_key(a).cmp(&tab_key(b)))
                .then_with(|| ia.cmp(ib))
        });
        let visuals = order
            .into_iter()
            .filter_map(|(_, v)| {
                let is_hidden = hidden.get(v.name.as_str()).copied().unwrap_or(v.is_hidden);
                if is_hidden && !opts.include_hidden {
                    return None;
                }
                let outer = positions
                    .get(v.name.as_str())
                    .copied()
                    .unwrap_or_else(|| position_rect(v));
                Some(frame(theme, v, outer, is_hidden))
            })
            .collect();
        Self {
            page_name: page.name.clone(),
            page_size,
            canvas_size,
            scale,
            offset,
            display_option: page.display_option.clone(),
            visuals,
        }
    }

    /// Page units → canvas transform.
    pub fn page_transform(&self) -> Transform {
        Transform::translate(self.offset.0, self.offset.1)
            .then(&Transform::scale(self.scale, self.scale))
    }

    /// A rectangle in canvas pixels.
    pub fn to_canvas(&self, r: &Rect) -> Rect {
        self.page_transform().apply_rect(r)
    }

    /// The page rectangle in page units.
    pub fn page_rect(&self) -> Rect {
        Rect::new(0.0, 0.0, self.page_size.0, self.page_size.1)
    }

    /// Frame by visual name.
    pub fn frame(&self, name: &str) -> Option<&VisualFrame> {
        self.visuals.iter().find(|f| f.name == name)
    }

    /// Frames that are not hidden.
    pub fn visible(&self) -> impl Iterator<Item = &VisualFrame> {
        self.visuals.iter().filter(|f| !f.hidden)
    }
}

fn sanitize_dim(v: f64, default: f64) -> f64 {
    if v.is_finite() && v > 0.0 {
        v.min(MAX_PAGE_DIMENSION)
    } else {
        default
    }
}

fn sanitize_page_size(w: f64, h: f64) -> (f64, f64) {
    (
        sanitize_dim(w, DEFAULT_PAGE_SIZE.0),
        sanitize_dim(h, DEFAULT_PAGE_SIZE.1),
    )
}

/// `(scale, offset, canvas size)` for a display option and viewport.
fn scaling(
    display: &DisplayOption,
    (pw, ph): (f64, f64),
    viewport: Option<(f64, f64)>,
    valign: VerticalAlignment,
) -> (f64, (f64, f64), (f64, f64)) {
    let viewport = viewport.filter(|(w, h)| w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0);
    let Some((vw, vh)) =
        viewport.map(|(w, h)| (w.min(MAX_PAGE_DIMENSION), h.min(MAX_PAGE_DIMENSION)))
    else {
        return (1.0, (0.0, 0.0), (pw, ph));
    };
    let center_y = |content_h: f64| match valign {
        VerticalAlignment::Middle if content_h < vh => (vh - content_h) / 2.0,
        _ => 0.0,
    };
    match display {
        DisplayOption::FitToWidth => {
            let s = vw / pw;
            (s, (0.0, center_y(ph * s)), (vw, vh.max(ph * s)))
        }
        DisplayOption::ActualSize => (
            1.0,
            (((vw - pw) / 2.0).max(0.0), center_y(ph)),
            (vw.max(pw), vh.max(ph)),
        ),
        _ => {
            let s = (vw / pw).min(vh / ph);
            (s, ((vw - pw * s) / 2.0, center_y(ph * s)), (vw, vh))
        }
    }
}

fn tab_key(v: &Visual) -> i64 {
    v.position.tab_order.unwrap_or(i64::MAX)
}

fn position_rect(v: &Visual) -> Rect {
    let p = &v.position;
    let f = |x: f64| if x.is_finite() { x } else { 0.0 };
    Rect::new(f(p.x), f(p.y), f(p.width).max(0.0), f(p.height).max(0.0))
}

/// Absolute rectangles of every visual (group offsets applied).
fn resolve_positions(page: &Page) -> HashMap<&str, Rect> {
    let by_name: HashMap<&str, &Visual> =
        page.visuals.iter().map(|v| (v.name.as_str(), v)).collect();
    let mut out: HashMap<&str, Rect> = HashMap::new();
    for v in &page.visuals {
        let rect = absolute_rect(v, &by_name, 0);
        out.insert(v.name.as_str(), rect);
    }
    out
}

fn absolute_rect(v: &Visual, by_name: &HashMap<&str, &Visual>, depth: usize) -> Rect {
    let own = position_rect(v);
    let parent = v
        .parent_group_name
        .as_deref()
        .filter(|p| *p != v.name)
        .and_then(|p| by_name.get(p));
    let Some(parent) = parent.filter(|_| depth < MAX_GROUP_DEPTH) else {
        return own;
    };
    let group = absolute_rect(parent, by_name, depth + 1);
    let relative = own.translate(group.x, group.y);
    let eps = 0.5;
    if group.contains_rect(&own, eps) || !group.contains_rect(&relative, eps) {
        own
    } else {
        relative
    }
}

/// Effective hidden flag (a hidden group hides its descendants).
fn resolve_hidden(page: &Page) -> HashMap<&str, bool> {
    let by_name: HashMap<&str, &Visual> =
        page.visuals.iter().map(|v| (v.name.as_str(), v)).collect();
    page.visuals
        .iter()
        .map(|v| {
            let mut hidden = v.is_hidden;
            let mut cur = v;
            for _ in 0..MAX_GROUP_DEPTH {
                let Some(p) = cur
                    .parent_group_name
                    .as_deref()
                    .and_then(|n| by_name.get(n))
                else {
                    break;
                };
                hidden |= p.is_hidden;
                cur = p;
            }
            (v.name.as_str(), hidden)
        })
        .collect()
}

/// Paint key per visual: a group sorts at the smallest z of itself and its
/// descendants, so its background paints before its children.
fn paint_z(page: &Page) -> HashMap<&str, f64> {
    let by_name: HashMap<&str, &Visual> =
        page.visuals.iter().map(|v| (v.name.as_str(), v)).collect();
    let mut keys: HashMap<&str, f64> = page
        .visuals
        .iter()
        .map(|v| (v.name.as_str(), v.position.z))
        .collect();
    for v in &page.visuals {
        let mut cur = v;
        for _ in 0..MAX_GROUP_DEPTH {
            let Some(parent) = cur
                .parent_group_name
                .as_deref()
                .and_then(|n| by_name.get(n))
            else {
                break;
            };
            if let Some(k) = keys.get_mut(parent.name.as_str()) {
                *k = k.min(v.position.z);
            }
            cur = parent;
        }
    }
    keys
}

/// Power BI's automatic title: value fields `by` category fields (`Total
/// Sales by Category`, `Count of Order ID by Region`).
pub fn auto_title(visual: &Visual) -> String {
    const VALUE_ROLES: &[&str] = &[
        "Y",
        "Y2",
        "Values",
        "Value",
        "Data",
        "X",
        "Size",
        "Indicator",
        "Goal",
        "TargetValue",
    ];
    const CATEGORY_ROLES: &[&str] = &[
        "Category", "Axis", "Rows", "Columns", "Series", "Legend", "Details",
    ];
    let join = |roles: &[&str]| -> String {
        let mut labels: Vec<&str> = Vec::new();
        for role in roles {
            for p in visual.projections_for(role) {
                let l = p.label();
                if !l.is_empty() && !labels.contains(&l) {
                    labels.push(l);
                }
            }
        }
        labels.join(" and ")
    };
    let values = join(VALUE_ROLES);
    let categories = join(CATEGORY_ROLES);
    match (values.is_empty(), categories.is_empty()) {
        (false, false) => format!("{values} by {categories}"),
        (false, true) => values,
        (true, false) => categories,
        (true, true) => String::new(),
    }
}

/// Builds one frame from the visual's container objects.
fn frame(theme: &ThemeResolver, v: &Visual, outer: Rect, hidden: bool) -> VisualFrame {
    let props = theme.visual(v);
    let c = props.container();
    let visual_type = props.visual_type().to_owned();
    let bag = if v.is_group() { props.objects() } else { c };

    let background = bag
        .bool("background", "show")
        .unwrap_or(true)
        .then(|| {
            let color = bag
                .color("background", "color")
                .unwrap_or_else(|| theme.background());
            let transparency = bag
                .number("background", "transparency")
                .unwrap_or(0.0)
                .clamp(0.0, 100.0);
            Paint::with_transparency(color, transparency)
        })
        .filter(Paint::is_visible);

    let radius = c
        .number("border", "radius")
        .unwrap_or(0.0)
        .clamp(0.0, 10_000.0);
    let border = (!v.is_group() && c.bool("border", "show") == Some(true))
        .then(|| Border {
            color: c.color("border", "color").unwrap_or(Color::BLACK),
            width: c
                .number("border", "width")
                .unwrap_or(1.0)
                .clamp(0.0, 1_000.0),
        })
        .filter(|b| b.width > 0.0);

    let drop_shadow = (!v.is_group() && c.bool("dropShadow", "show") == Some(true))
        .then(|| drop_shadow(&c))
        .filter(|s| s.opacity > 0.0);

    let header = VisualHeader {
        show: !v.is_group() && c.bool("visualHeader", "show").unwrap_or(false),
        background: c.color("visualHeader", "background"),
        foreground: c.color("visualHeader", "foreground"),
        border: c.color("visualHeader", "border"),
    };

    let border_w = border.as_ref().map_or(0.0, |b| b.width);
    let inner = outer.inset_uniform(border_w);
    let padding = if v.is_group() {
        Insets::ZERO
    } else {
        c.insets("padding")
    };

    let title = if v.is_group() {
        None
    } else {
        title_box(theme, v, &c, inner, padding)
    };
    let top_offset = title.as_ref().map_or(padding.top, |t| t.rect.height);
    let content = Rect::new(
        inner.x + padding.left,
        inner.y + top_offset,
        (inner.width - padding.left - padding.right).max(0.0),
        (inner.height - top_offset - padding.bottom).max(0.0),
    );

    VisualFrame {
        name: v.name.clone(),
        visual_type,
        is_group: v.is_group(),
        parent_group: v.parent_group_name.clone(),
        z: if v.position.z.is_finite() {
            v.position.z
        } else {
            0.0
        },
        tab_order: v.position.tab_order,
        outer,
        inner,
        content,
        padding,
        radius,
        title,
        background,
        border,
        drop_shadow,
        header,
        hidden,
    }
}

fn drop_shadow(c: &crate::theme::PropLookup<'_>) -> DropShadow {
    let color = c.color("dropShadow", "color").unwrap_or(Color::BLACK);
    let transparency = c
        .number("dropShadow", "transparency")
        .unwrap_or(70.0)
        .clamp(0.0, 100.0);
    let distance = c
        .number("dropShadow", "shadowDistance")
        .unwrap_or(10.0)
        .clamp(0.0, 1_000.0);
    let preset = c.text("dropShadow", "preset").unwrap_or_default();
    let angle = match preset.to_ascii_lowercase().as_str() {
        "bottomright" => Some(45.0),
        "bottom" => Some(90.0),
        "bottomleft" => Some(135.0),
        "left" => Some(180.0),
        "topleft" => Some(225.0),
        "top" => Some(270.0),
        "topright" => Some(315.0),
        "right" => Some(0.0),
        "center" => None,
        _ => Some(c.number("dropShadow", "angle").unwrap_or(45.0)),
    };
    let offset = angle.map_or((0.0, 0.0), |a: f64| {
        let (s, co) = a.to_radians().sin_cos();
        (distance * co, distance * s)
    });
    let position = c.text("dropShadow", "position").unwrap_or_default();
    DropShadow {
        color,
        opacity: 1.0 - transparency / 100.0,
        offset,
        blur: c
            .number("dropShadow", "shadowBlur")
            .unwrap_or(10.0)
            .clamp(0.0, 1_000.0),
        spread: c
            .number("dropShadow", "shadowSpread")
            .unwrap_or(3.0)
            .clamp(-1_000.0, 1_000.0),
        inner: position.eq_ignore_ascii_case("inner"),
    }
}

fn title_box(
    theme: &ThemeResolver,
    v: &Visual,
    c: &crate::theme::PropLookup<'_>,
    inner: Rect,
    padding: Insets,
) -> Option<TitleBox> {
    if !c.bool("title", "show").unwrap_or(false) {
        return None;
    }
    let text = c
        .text("title", "text")
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| auto_title(v));
    if text.trim().is_empty() {
        return None;
    }
    let class = if theme.theme().json().text_class("largeTitle").is_some() {
        "largeTitle"
    } else {
        "title"
    };
    let style = c.text_style("title", class);
    let alignment = c
        .text("title", "alignment")
        .map_or(TextAlign::Left, |a| TextAlign::parse(&a));
    let wrap = c.bool("title", "titleWrap").unwrap_or(false);
    let width = (inner.width - padding.left - padding.right).max(0.0);
    let line_height = style.line_height();
    let max_lines = if wrap && line_height > 0.0 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let n = ((inner.height / 2.0) / line_height).floor().max(1.0) as usize;
        n
    } else {
        1
    };
    let layout = FontBook::shared().measure_with(
        &text,
        &style,
        &MeasureOptions {
            max_width: Some(width),
            wrap,
            max_lines: Some(max_lines),
            ellipsis: true,
        },
    );
    let height = layout.height.max(line_height).min(inner.height.max(0.0));
    let background = c
        .color("title", "background")
        .map(|color| {
            let t = c
                .number("title", "transparency")
                .unwrap_or(0.0)
                .clamp(0.0, 100.0);
            Paint::with_transparency(color, t)
        })
        .filter(Paint::is_visible);
    Some(TitleBox {
        rect: Rect::new(inner.x + padding.left, inner.y, width, height),
        text,
        style,
        alignment,
        wrap,
        background,
        lines: layout.lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pbi_model::Position;

    fn visual(name: &str, x: f64, y: f64, w: f64, h: f64, z: f64) -> Visual {
        Visual {
            name: name.into(),
            visual_type: "shape".into(),
            position: Position {
                x,
                y,
                z,
                width: w,
                height: h,
                ..Position::default()
            },
            ..Visual::default()
        }
    }

    #[test]
    fn scaling_modes() {
        let page = |d: DisplayOption| Page {
            display_option: d,
            ..Page::default()
        };
        let fit = PageLayout::compute(&page(DisplayOption::FitToPage), Some((640.0, 720.0)));
        assert_eq!(fit.scale, 0.5);
        assert_eq!(fit.offset, (0.0, 180.0));
        assert_eq!(fit.canvas_size, (640.0, 720.0));
        let width = PageLayout::compute(&page(DisplayOption::FitToWidth), Some((2560.0, 1000.0)));
        assert_eq!((width.scale, width.canvas_size), (2.0, (2560.0, 1440.0)));
        let actual = PageLayout::compute(&page(DisplayOption::ActualSize), Some((1480.0, 720.0)));
        assert_eq!((actual.scale, actual.offset), (1.0, (100.0, 0.0)));
        let none = PageLayout::compute(&page(DisplayOption::FitToPage), None);
        assert_eq!((none.scale, none.canvas_size), (1.0, (1280.0, 720.0)));
        let bad = PageLayout::compute(
            &Page {
                width: f64::NAN,
                height: -5.0,
                ..Page::default()
            },
            Some((0.0, f64::INFINITY)),
        );
        assert_eq!(bad.page_size, DEFAULT_PAGE_SIZE);
        assert_eq!(bad.scale, 1.0);
    }

    #[test]
    fn z_order_tab_order_groups_and_hidden() {
        let mut a = visual("a", 0.0, 0.0, 10.0, 10.0, 5.0);
        a.position.tab_order = Some(2);
        let mut b = visual("b", 0.0, 0.0, 10.0, 10.0, 5.0);
        b.position.tab_order = Some(1);
        let c = visual("c", 0.0, 0.0, 10.0, 10.0, 1.0);
        let mut g = visual("g", 100.0, 100.0, 50.0, 50.0, 9.0);
        g.kind = pbi_model::VisualKind::Group;
        g.visual_type = String::new();
        let mut rel = visual("rel", 5.0, 5.0, 10.0, 10.0, 7.0);
        rel.parent_group_name = Some("g".into());
        let mut abs = visual("abs", 110.0, 110.0, 10.0, 10.0, 8.0);
        abs.parent_group_name = Some("g".into());
        let mut hidden = visual("h", 0.0, 0.0, 1.0, 1.0, 0.0);
        hidden.is_hidden = true;
        let page = Page {
            visuals: vec![a, b, c, g, rel, abs, hidden],
            ..Page::default()
        };
        let layout = PageLayout::compute(&page, None);
        let names: Vec<&str> = layout.visuals.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["c", "b", "a", "g", "rel", "abs"]);
        assert_eq!(
            layout.frame("rel").unwrap().outer,
            Rect::new(105.0, 105.0, 10.0, 10.0)
        );
        assert_eq!(
            layout.frame("abs").unwrap().outer,
            Rect::new(110.0, 110.0, 10.0, 10.0)
        );
        let all = PageLayout::compute_with(
            &page,
            &ThemeResolver::default(),
            &LayoutOptions {
                include_hidden: true,
                ..LayoutOptions::default()
            },
        );
        assert!(all.frame("h").is_some_and(|f| f.hidden));
    }

    #[test]
    fn auto_titles() {
        let mut v = Visual::default();
        assert_eq!(auto_title(&v), "");
        v.projections = vec![
            pbi_model::Projection {
                role: "Y".into(),
                native_query_ref: Some("Count of Order ID".into()),
                ..Default::default()
            },
            pbi_model::Projection {
                role: "Category".into(),
                native_query_ref: Some("Region".into()),
                ..Default::default()
            },
        ];
        assert_eq!(auto_title(&v), "Count of Order ID by Region");
    }

    #[test]
    fn alignment_parsing() {
        assert_eq!(TextAlign::parse("Center"), TextAlign::Center);
        assert_eq!(TextAlign::parse("right"), TextAlign::Right);
        assert_eq!(TextAlign::parse("???"), TextAlign::Left);
    }
}
