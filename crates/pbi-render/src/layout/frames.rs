//! Container ("frame") rendering: page canvas, visual chrome and
//! placeholders. This is the base every visual renderer plugs into via
//! [`render_page_with`] / [`frame_group`].
//!
//! Paint order per visual: drop shadow → background (or an invisible,
//! hit-testable rectangle) → title → content
//! (clipped to [`VisualFrame::content`], or to the rounded inner rect) →
//! border. Visual groups only paint their background. Hidden visuals are
//! skipped. The visual header is never drawn (it only appears on hover in
//! the service).

use pbi_model::{Page, Report, Visual};

use super::{LayoutOptions, PageLayout, TextAlign, VisualFrame};
use crate::scene::{
    Clip, Color, Group, HitId, Paint, PathBuilder, PathData, PathNode, Rect, RectNode, Scene,
    TextAnchor,
};
use crate::text::{FontBook, MeasureOptions, TextLayout};
use crate::theme::ThemeResolver;

/// CSS class of every visual container group.
pub const VISUAL_CLASS: &str = "pbi-visual";
/// CSS class of the page group.
pub const PAGE_CLASS: &str = "pbi-page";
/// Color of placeholder labels.
pub const PLACEHOLDER_COLOR: Color = Color::rgb(0xB3, 0xB0, 0xAD);

/// Options for [`render_page_frames`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameOptions {
    /// Viewport in CSS pixels (`None`: the page at scale 1).
    pub viewport: Option<(f64, f64)>,
    /// Draw a light label with the visual type in each content area.
    pub placeholders: bool,
}

impl Default for FrameOptions {
    fn default() -> Self {
        Self {
            viewport: None,
            placeholders: true,
        }
    }
}

/// Renders the page canvas and every visual's container (background,
/// border, rounded corners, drop shadow, title) with a placeholder label
/// showing the visual type.
pub fn render_page_frames(
    report: &Report,
    page: &Page,
    theme: &ThemeResolver,
    opts: &FrameOptions,
) -> Scene {
    let placeholders = opts.placeholders;
    render_page_with(report, page, theme, opts, |frame, _visual| {
        placeholders.then(|| placeholder(frame, theme))
    })
}

/// Renders a page, asking `content` for each visual's content group (in
/// page units; it is clipped to the frame's content box). `None` leaves the
/// content area empty. Groups are never passed to `content`.
pub fn render_page_with<F>(
    report: &Report,
    page: &Page,
    theme: &ThemeResolver,
    opts: &FrameOptions,
    mut content: F,
) -> Scene
where
    F: FnMut(&VisualFrame, &Visual) -> Option<Group>,
{
    let layout = PageLayout::compute_with(
        page,
        theme,
        &LayoutOptions::for_report(report, opts.viewport),
    );
    let mut scene = Scene::new(layout.canvas_size.0, layout.canvas_size.1);
    let page_props = theme.page(page);
    let objects = page_props.objects();

    let outspace = objects.color("outspace", "color").unwrap_or(Color::WHITE);
    let outspace_t = objects
        .number("outspace", "transparency")
        .unwrap_or(0.0)
        .clamp(0.0, 100.0);
    scene.root.push(
        RectNode::new(Rect::new(0.0, 0.0, scene.width, scene.height))
            .fill(Paint::with_transparency(outspace, outspace_t)),
    );

    let mut page_group = Group::new()
        .with_transform(layout.page_transform())
        .with_clip(Clip::rect(layout.page_rect()))
        .with_class(PAGE_CLASS);
    let bg = objects.color("background", "color").unwrap_or(Color::WHITE);
    let bg_t = objects
        .number("background", "transparency")
        .unwrap_or(100.0)
        .clamp(0.0, 100.0);
    let bg_paint = Paint::with_transparency(bg, bg_t);
    if bg_paint.is_visible() {
        page_group.push(RectNode::new(layout.page_rect()).fill(bg_paint));
    }

    for frame in layout.visible() {
        let Some(visual) = page.visual(&frame.name) else {
            continue;
        };
        let inner = if frame.is_group {
            None
        } else {
            content(frame, visual)
        };
        page_group.push(frame_group(frame, inner));
    }
    scene.root.push(page_group);
    scene
}

/// A visual's container group (hit id `HitId::visual(name)`, class
/// [`VISUAL_CLASS`]) with its chrome around `content`.
pub fn frame_group(frame: &VisualFrame, content: Option<Group>) -> Group {
    let mut g = Group::new()
        .with_id(HitId::visual(frame.name.clone()))
        .with_class(VISUAL_CLASS);
    if let Some(shadow) = &frame.drop_shadow
        && !shadow.inner
    {
        push_shadow(&mut g, frame, shadow);
    }
    match frame.background {
        Some(bg) => {
            g.push(RectNode::new(frame.outer).radius(frame.radius).fill(bg));
        }
        // Invisible but hit-testable, so every visual has a hit region.
        None if !frame.is_group => {
            g.push(
                RectNode::new(frame.outer)
                    .radius(frame.radius)
                    .fill(Paint::new(Color::BLACK, 0.0)),
            );
        }
        None => {}
    }
    if let Some(title) = &frame.title {
        if let Some(bg) = title.background {
            g.push(RectNode::new(title.rect).fill(bg));
        }
        let (x, anchor) = match title.alignment {
            TextAlign::Left => (title.rect.x, TextAnchor::Start),
            TextAlign::Center => (title.rect.x + title.rect.width / 2.0, TextAnchor::Middle),
            TextAlign::Right => (title.rect.right(), TextAnchor::End),
        };
        let lines = TextLayout {
            lines: title.lines.clone(),
            ..TextLayout::default()
        };
        for node in lines.to_nodes(x, title.rect.y, anchor, &title.style) {
            g.push(node);
        }
    }
    if let Some(mut content) = content
        && !content.is_empty()
    {
        if content.clip.is_none() {
            content.clip = Some(Clip::rect(frame.content));
        }
        let inner_radius = (frame.radius - frame.border.as_ref().map_or(0.0, |b| b.width)).max(0.0);
        if inner_radius > 0.0 {
            let mut rounded = Group::new().with_clip(Clip::rounded(frame.inner, inner_radius));
            rounded.push(content);
            g.push(rounded);
        } else {
            g.push(content);
        }
    }
    if let Some((rect, stroke)) = frame.border_stroke() {
        let radius = (frame.radius - stroke.width / 2.0).max(0.0);
        g.push(RectNode::new(rect).radius(radius).stroke(stroke));
    }
    g
}

/// Approximates a blurred box shadow with four stacked rings that exclude
/// the visual itself (like CSS `box-shadow`).
fn push_shadow(g: &mut Group, frame: &VisualFrame, shadow: &super::DropShadow) {
    const LAYERS: u32 = 4;
    let base = frame.outer.translate(shadow.offset.0, shadow.offset.1);
    let per_layer = shadow.opacity / f64::from(LAYERS);
    for i in 0..LAYERS {
        let t = f64::from(i + 1) / f64::from(LAYERS);
        let grow = shadow.spread + shadow.blur * (t - 0.5);
        let rect = base.inset_uniform(-grow);
        if rect.is_empty() {
            continue;
        }
        // The hole must lie inside the outer rect for the nonzero rule.
        let hole = frame.outer.intersect(&rect);
        let d = ring(rect, (frame.radius + grow).max(0.0), hole);
        g.push(PathNode::new(d).fill(Paint::new(shadow.color, per_layer)));
    }
}

/// `outer` (rounded) minus `hole`, using opposite windings (nonzero rule).
fn ring(outer: Rect, radius: f64, hole: Option<Rect>) -> PathData {
    let mut d = PathData::rounded_rect(outer, radius);
    let Some(hole) = hole else {
        return d;
    };
    let mut b = PathBuilder::new();
    b.move_to(hole.x, hole.y)
        .line_to(hole.x, hole.bottom())
        .line_to(hole.right(), hole.bottom())
        .line_to(hole.right(), hole.y)
        .close();
    d.segs.extend(b.build().segs);
    d
}

/// A centred, light label with the visual type (ellipsized; omitted when
/// the content box is too small).
pub fn placeholder(frame: &VisualFrame, theme: &ThemeResolver) -> Group {
    let mut g = Group::new();
    let style = theme.text_class("smallLabel").with_color(PLACEHOLDER_COLOR);
    let c = frame.content;
    if c.height < style.line_height() || c.width <= 0.0 {
        return g;
    }
    let layout = FontBook::shared().measure_with(
        &frame.visual_type,
        &style,
        &MeasureOptions {
            max_width: Some(c.width),
            wrap: false,
            max_lines: Some(1),
            ellipsis: true,
        },
    );
    let (cx, cy) = c.center();
    for node in layout.to_nodes(cx, cy - layout.height / 2.0, TextAnchor::Middle, &style) {
        g.push(node);
    }
    g
}
