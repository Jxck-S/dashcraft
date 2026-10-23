//! Action buttons (`actionButton`) and navigators (`pageNavigator`,
//! `bookmarkNavigator`): shape + vector icon + text, with a
//! [`HitKind::Button`](crate::scene::HitKind::Button) hit region. (owner:
//! visuals-basics)
//!
//! # Formatting objects (default state)
//!
//! | Object.property | Meaning | Default |
//! |---|---|---|
//! | `shape.tileShape`, `shape.roundEdge` | button geometry ([`super::shape::ShapeKind`]) | `rectangle`, `0` |
//! | `fill.show` / `fillColor` / `transparency` | button fill | `false` / theme background / `0` |
//! | `outline.show` / `lineColor` / `weight` / `transparency` / `roundEdge` | button outline | `false` / theme foreground / `1` / `0` |
//! | `icon.show` / `icon.shapeType` | the icon ([`IconKind::parse`]) | `true` / from the action (`Back` → `back`), else `blank` |
//! | `icon.lineColor` / `lineWeight` / `lineTransparency` | icon strokes | `#605E5C` (theme `foregroundNeutralSecondary`) / 10% of the icon size / `0` |
//! | `icon.fillColor` / `icon.transparency` | icon fill | none |
//! | `icon.padding` | icon inset (px) | `0` |
//! | `text.show` / `text.text` / font properties / alignment | button text (icon on its left) | `false` / – / `label` class / center |
//!
//! The icon is a square of the content box's shorter side, centred (with
//! text: on the text's left). The Back icon (an arrow in a circle, 10%
//! strokes) matches Power BI's 40×40 Back button: a 30px circle in
//! `#605E5C`.
//!
//! # Actions
//!
//! [`action`] reads `visualLink` (`visualContainerObjects`, else `objects`):
//! `type` = `Back`, `Bookmark` (`bookmark`), `PageNavigation`
//! (`navigationSection`), `Drillthrough` (`drillthroughSection`), `WebUrl`
//! (`webUrl`), `QnA`, `ApplyAllSlicers`, `ClearAllSlicers`; `show = false`
//! disables it. The canvas resolves a click on a `button` hit region through
//! [`action`] (or [`super::page::VisualRenderInfo::action`]).
//!
//! Navigators render as one labelled tab button: the page / bookmark list
//! is not available to visual renderers yet.

use pbi_model::Visual;
use serde::{Deserialize, Serialize};

use crate::layout::TextAlign;
use crate::scene::{
    Clip, Color, Group, HitId, Paint, PathBuilder, PathData, PathNode, Rect, RectNode, Stroke,
    TextAnchor,
};
use crate::theme::{PropLookup, ThemeResolver};

use super::VisualCtx;
use super::common::{ellipsize, family_metrics, text_node};
use super::shape::{ShapeKind, ellipse, shape_path};

/// What clicking a button does.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ButtonAction {
    /// Back to the previous page.
    Back,
    /// Apply a bookmark (its internal name).
    Bookmark {
        bookmark: String,
    },
    /// Go to a page (its internal name).
    PageNavigation {
        page: String,
    },
    /// Drill through to a page.
    Drillthrough {
        page: String,
    },
    /// Open a URL (never fetched by the renderer).
    WebUrl {
        url: String,
    },
    /// Q&A.
    QnA,
    ApplyAllSlicers,
    ClearAllSlicers,
    /// An action type this renderer does not know.
    Other {
        kind: String,
    },
}

/// Icon of an action button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconKind {
    Blank,
    /// Arrow pointing left in a circle.
    Back,
    /// Arrow pointing right in a circle.
    Forward,
    LeftArrow,
    RightArrow,
    Information,
    Help,
    QnA,
    Bookmark,
    Reset,
    Filter,
    Clear,
    Apply,
}

impl IconKind {
    /// Parses `icon.shapeType` (case and punctuation ignored); unknown
    /// names are `None`.
    pub fn parse(name: &str) -> Option<Self> {
        let n: String = name
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
            .collect();
        Some(match n.as_str() {
            "blank" | "none" | "" => IconKind::Blank,
            "back" | "backarrow" => IconKind::Back,
            "forward" | "next" | "nextarrow" => IconKind::Forward,
            "leftarrow" | "arrowleft" | "left" | "previous" => IconKind::LeftArrow,
            "rightarrow" | "arrowright" | "right" => IconKind::RightArrow,
            "information" | "info" => IconKind::Information,
            "help" | "question" => IconKind::Help,
            "qna" | "qanda" | "qa" => IconKind::QnA,
            "bookmark" => IconKind::Bookmark,
            "reset" | "undo" => IconKind::Reset,
            "filter" => IconKind::Filter,
            "clear" | "clearallslicers" | "close" => IconKind::Clear,
            "apply" | "applyallslicers" | "check" => IconKind::Apply,
            _ => return None,
        })
    }
}

fn link_text(visual: &Visual, theme: &ThemeResolver, prop: &str) -> Option<String> {
    let props = theme.visual(visual);
    props
        .container()
        .text("visualLink", prop)
        .or_else(|| props.objects().text("visualLink", prop))
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// The button's action (`visualLink`), if it has an enabled one.
pub fn action(visual: &Visual, theme: &ThemeResolver) -> Option<ButtonAction> {
    let props = theme.visual(visual);
    let show = props
        .container()
        .bool("visualLink", "show")
        .or_else(|| props.objects().bool("visualLink", "show"))
        .unwrap_or(true);
    if !show {
        return None;
    }
    let kind = link_text(visual, theme, "type")?;
    let norm: String = kind
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let target = |p: &str| link_text(visual, theme, p).unwrap_or_default();
    Some(match norm.as_str() {
        "back" => ButtonAction::Back,
        "bookmark" => ButtonAction::Bookmark {
            bookmark: target("bookmark"),
        },
        "pagenavigation" | "page" => ButtonAction::PageNavigation {
            page: target("navigationSection"),
        },
        "drillthrough" => ButtonAction::Drillthrough {
            page: target("drillthroughSection"),
        },
        "weburl" | "url" => ButtonAction::WebUrl {
            url: target("webUrl"),
        },
        "qna" | "qanda" => ButtonAction::QnA,
        "applyallslicers" => ButtonAction::ApplyAllSlicers,
        "clearallslicers" => ButtonAction::ClearAllSlicers,
        _ => ButtonAction::Other { kind },
    })
}

/// The button's `visualLink.tooltip`, if any.
pub fn tooltip(visual: &Visual, theme: &ThemeResolver) -> Option<String> {
    link_text(visual, theme, "tooltip")
}

/// Renders the button (see the module docs).
pub fn render(ctx: &VisualCtx<'_>) -> Option<Group> {
    let area = ctx.frame.inner;
    if area.is_empty() {
        return None;
    }
    let props = ctx.theme.visual(ctx.visual);
    let o = props.objects();
    let mut g = Group::new()
        .with_id(HitId::button(ctx.visual.name.clone()))
        .with_class("pbi-button")
        .with_clip(Clip::rect(area));
    // Invisible, so the hit region covers the whole button.
    g.push(RectNode::new(area).fill(Paint::new(Color::BLACK, 0.0)));
    let navigator = match ctx.visual.visual_type.as_str() {
        "pageNavigator" => Some("Page navigator"),
        "bookmarkNavigator" => Some("Bookmark navigator"),
        _ => None,
    };
    button_shape(ctx, &o, area, navigator.is_some(), &mut g);

    let c = ctx.frame.content;
    let text = match navigator {
        Some(label) => Some(label.to_owned()),
        None => o
            .bool("text", "show")
            .unwrap_or(false)
            .then(|| o.text("text", "text"))
            .flatten()
            .filter(|t| !t.trim().is_empty()),
    };
    let icon = if navigator.is_some() || !o.bool("icon", "show").unwrap_or(true) {
        IconKind::Blank
    } else {
        o.text("icon", "shapeType")
            .and_then(|n| IconKind::parse(&n))
            .unwrap_or_else(|| match action(ctx.visual, ctx.theme) {
                Some(ButtonAction::Back) => IconKind::Back,
                Some(ButtonAction::Bookmark { .. }) => IconKind::Bookmark,
                Some(ButtonAction::QnA) => IconKind::QnA,
                _ => IconKind::Blank,
            })
    };
    let pad = o.number("icon", "padding").unwrap_or(0.0).max(0.0);
    let mut text_box = c;
    if icon != IconKind::Blank && c.width > 0.0 && c.height > 0.0 {
        let side = if text.is_some() {
            c.height.min(c.width / 3.0)
        } else {
            c.width.min(c.height)
        };
        let side = (side - 2.0 * pad).max(0.0);
        let bx = if text.is_some() {
            c.x + pad
        } else {
            c.x + (c.width - side) / 2.0
        };
        let icon_box = Rect::new(bx, c.y + (c.height - side) / 2.0, side, side);
        if text.is_some() {
            text_box = Rect::new(
                icon_box.right() + pad + 4.0,
                c.y,
                (c.right() - icon_box.right() - pad - 4.0).max(0.0),
                c.height,
            );
        }
        let color = o
            .color("icon", "lineColor")
            .or_else(|| ctx.theme.named_color("foregroundNeutralSecondary"))
            .unwrap_or(Color::rgb(0x60, 0x5E, 0x5C));
        let weight = o
            .number("icon", "lineWeight")
            .filter(|w| *w >= 0.0)
            .unwrap_or_else(|| (side * 0.1).max(1.0))
            .min(side / 2.0);
        let t = o
            .number("icon", "lineTransparency")
            .unwrap_or(0.0)
            .clamp(0.0, 100.0);
        let stroke = Stroke::new(color, weight).with_opacity(1.0 - t / 100.0);
        let fill = o
            .color("icon", "fillColor")
            .map(|c| {
                let t = o
                    .number("icon", "transparency")
                    .unwrap_or(0.0)
                    .clamp(0.0, 100.0);
                Paint::with_transparency(c, t)
            })
            .filter(Paint::is_visible);
        for node in icon_nodes(icon, icon_box, &stroke, fill) {
            g.push(node);
        }
    }
    if let Some(text) = text {
        let style = o.text_style("text", "label");
        let m = family_metrics(&style);
        if text_box.width > 0.0 && text_box.height >= m.line_height_px(&style) * 0.5 {
            let align = o
                .text("text", "horizontalAlignment")
                .map_or(TextAlign::Center, |a| TextAlign::parse(&a));
            let t = ellipsize(ctx.fonts, &text, &style, text_box.width);
            let (x, anchor) = match align {
                TextAlign::Left => (text_box.x, TextAnchor::Start),
                TextAlign::Center => (text_box.x + text_box.width / 2.0, TextAnchor::Middle),
                TextAlign::Right => (text_box.right(), TextAnchor::End),
            };
            let top = text_box.y + (text_box.height - m.line_height_px(&style)) / 2.0;
            g.push(text_node(
                x,
                top + m.ascent_px(&style),
                t,
                &style,
                anchor,
                Some(text_box),
            ));
        }
    }
    Some(g)
}

/// The button's fill and outline.
fn button_shape(
    ctx: &VisualCtx<'_>,
    o: &PropLookup<'_>,
    area: Rect,
    navigator: bool,
    g: &mut Group,
) {
    let kind = o
        .text("shape", "tileShape")
        .and_then(|n| ShapeKind::parse(&n))
        .filter(|k| !k.is_line())
        .unwrap_or(ShapeKind::Rectangle);
    let fill = o
        .bool("fill", "show")
        .unwrap_or(navigator)
        .then(|| {
            let color = o.color("fill", "fillColor").unwrap_or_else(|| {
                if navigator {
                    ctx.theme
                        .named_color("backgroundLight")
                        .unwrap_or(Color::rgb(0xF3, 0xF2, 0xF1))
                } else {
                    ctx.theme.background()
                }
            });
            let t = o
                .number("fill", "transparency")
                .unwrap_or(0.0)
                .clamp(0.0, 100.0);
            Paint::with_transparency(color, t)
        })
        .filter(Paint::is_visible);
    let stroke = o
        .bool("outline", "show")
        .unwrap_or(navigator)
        .then(|| {
            let color = o.color("outline", "lineColor").unwrap_or_else(|| {
                if navigator {
                    Color::rgb(0xC8, 0xC6, 0xC4)
                } else {
                    ctx.theme.foreground()
                }
            });
            let w = o
                .number("outline", "weight")
                .unwrap_or(1.0)
                .clamp(0.0, area.width.min(area.height) / 2.0);
            let t = o
                .number("outline", "transparency")
                .unwrap_or(0.0)
                .clamp(0.0, 100.0);
            Stroke::new(color, w).with_opacity(1.0 - t / 100.0)
        })
        .filter(Stroke::is_visible);
    if fill.is_none() && stroke.is_none() {
        return;
    }
    let radius = o
        .number("shape", "roundEdge")
        .or_else(|| o.number("outline", "roundEdge"))
        .unwrap_or(if kind == ShapeKind::RoundedRectangle {
            10.0
        } else {
            0.0
        })
        .max(0.0);
    let inset = stroke.as_ref().map_or(0.0, |s| s.width / 2.0);
    if let Some(d) = shape_path(kind, area.inset_uniform(inset), (radius - inset).max(0.0)) {
        let mut node = PathNode::new(d);
        node.fill = fill;
        node.stroke = stroke;
        g.push(node);
    }
}

fn circle(cx: f64, cy: f64, r: f64) -> PathData {
    ellipse(Rect::new(cx - r, cy - r, 2.0 * r, 2.0 * r))
}

/// Builds a path from points relative to the icon centre, in units of the
/// icon size.
struct IconPath {
    cx: f64,
    cy: f64,
    s: f64,
    b: PathBuilder,
}

impl IconPath {
    fn new(r: Rect) -> Self {
        let (cx, cy) = r.center();
        Self {
            cx,
            cy,
            s: r.width.min(r.height),
            b: PathBuilder::new(),
        }
    }

    fn p(&self, u: f64, v: f64) -> (f64, f64) {
        (self.cx + u * self.s, self.cy + v * self.s)
    }

    fn m(&mut self, u: f64, v: f64) -> &mut Self {
        let (x, y) = self.p(u, v);
        self.b.move_to(x, y);
        self
    }

    fn l(&mut self, u: f64, v: f64) -> &mut Self {
        let (x, y) = self.p(u, v);
        self.b.line_to(x, y);
        self
    }

    fn c(&mut self, u1: f64, v1: f64, u2: f64, v2: f64, u: f64, v: f64) -> &mut Self {
        let (x1, y1) = self.p(u1, v1);
        let (x2, y2) = self.p(u2, v2);
        let (x, y) = self.p(u, v);
        self.b.cubic_to(x1, y1, x2, y2, x, y);
        self
    }

    fn arc(&mut self, r: f64, large: bool, sweep: bool, u: f64, v: f64) -> &mut Self {
        let (x, y) = self.p(u, v);
        let r = r * self.s;
        self.b.arc_to(r, r, 0.0, large, sweep, x, y);
        self
    }

    fn close(&mut self) -> &mut Self {
        self.b.close();
        self
    }

    fn build(&self) -> PathData {
        self.b.build()
    }
}

/// The icon's paths in the square `r`: outlines stroked with `stroke`
/// (closed outlines also filled with `fill`), dots filled with the stroke
/// color.
pub fn icon_nodes(kind: IconKind, r: Rect, stroke: &Stroke, fill: Option<Paint>) -> Vec<PathNode> {
    let mut out = Vec::new();
    if r.is_empty() || kind == IconKind::Blank {
        return out;
    }
    let (cx, cy) = r.center();
    let s = r.width.min(r.height);
    let ring = || {
        let mut n = PathNode::new(circle(cx, cy, 0.45 * s)).stroke(stroke.clone());
        n.fill = fill;
        n
    };
    let lines = |p: &IconPath| PathNode::new(p.build()).stroke(stroke.clone());
    let dot = |u: f64, v: f64| {
        let rad = (stroke.width * 0.75).max(0.5);
        PathNode::new(circle(cx + u * s, cy + v * s, rad))
            .fill(Paint::new(stroke.color, stroke.opacity))
    };
    let mut p = IconPath::new(r);
    match kind {
        IconKind::Blank => {}
        IconKind::Back | IconKind::Forward => {
            let k = if kind == IconKind::Back { 1.0 } else { -1.0 };
            out.push(ring());
            p.m(0.0, -0.25)
                .l(-0.27 * k, 0.0)
                .l(0.0, 0.25)
                .m(-0.27 * k, 0.0)
                .l(0.28 * k, 0.0);
            out.push(lines(&p));
        }
        IconKind::LeftArrow | IconKind::RightArrow => {
            let k = if kind == IconKind::LeftArrow {
                1.0
            } else {
                -1.0
            };
            p.m(-0.05 * k, -0.33)
                .l(-0.4 * k, 0.0)
                .l(-0.05 * k, 0.33)
                .m(-0.4 * k, 0.0)
                .l(0.4 * k, 0.0);
            out.push(lines(&p));
        }
        IconKind::Information => {
            out.push(ring());
            p.m(0.0, -0.04).l(0.0, 0.25);
            out.push(lines(&p));
            out.push(dot(0.0, -0.2));
        }
        IconKind::Help => {
            out.push(ring());
            p.m(-0.12, -0.1)
                .c(-0.12, -0.3, 0.14, -0.3, 0.14, -0.12)
                .c(0.14, -0.02, 0.0, 0.0, 0.0, 0.1);
            out.push(lines(&p));
            out.push(dot(0.0, 0.24));
        }
        IconKind::QnA => {
            p.m(-0.4, -0.35)
                .l(0.4, -0.35)
                .l(0.4, 0.2)
                .l(-0.05, 0.2)
                .l(-0.25, 0.4)
                .l(-0.25, 0.2)
                .l(-0.4, 0.2)
                .close();
            let mut bubble = lines(&p);
            bubble.fill = fill;
            out.push(bubble);
            let mut q = IconPath::new(r);
            q.m(-0.08, -0.17)
                .c(-0.08, -0.29, 0.1, -0.29, 0.1, -0.17)
                .c(0.1, -0.1, 0.0, -0.09, 0.0, -0.02);
            out.push(lines(&q));
            out.push(dot(0.0, 0.08));
        }
        IconKind::Bookmark => {
            p.m(-0.25, -0.4)
                .l(0.25, -0.4)
                .l(0.25, 0.4)
                .l(0.0, 0.18)
                .l(-0.25, 0.4)
                .close();
            let mut n = lines(&p);
            n.fill = fill;
            out.push(n);
        }
        IconKind::Reset => {
            // Three quarters of a circle (clockwise from the top to the
            // left) with an arrowhead at the top.
            p.m(0.0, -0.32)
                .arc(0.32, true, true, -0.32, 0.0)
                .m(0.1, -0.42)
                .l(0.0, -0.32)
                .l(0.1, -0.22);
            out.push(lines(&p));
        }
        IconKind::Filter => {
            p.m(-0.4, -0.35)
                .l(0.4, -0.35)
                .l(0.08, 0.02)
                .l(0.08, 0.35)
                .l(-0.08, 0.28)
                .l(-0.08, 0.02)
                .close();
            let mut n = lines(&p);
            n.fill = fill;
            out.push(n);
        }
        IconKind::Clear => {
            out.push(ring());
            p.m(-0.17, -0.17)
                .l(0.17, 0.17)
                .m(0.17, -0.17)
                .l(-0.17, 0.17);
            out.push(lines(&p));
        }
        IconKind::Apply => {
            out.push(ring());
            p.m(-0.2, 0.0).l(-0.05, 0.15).l(0.22, -0.13);
            out.push(lines(&p));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_names() {
        assert_eq!(IconKind::parse("back"), Some(IconKind::Back));
        assert_eq!(IconKind::parse("Q&A"), Some(IconKind::QnA));
        assert_eq!(IconKind::parse("information"), Some(IconKind::Information));
        assert_eq!(IconKind::parse("custom"), None);
    }

    #[test]
    fn every_icon_draws_inside_its_box() {
        let r = Rect::new(10.0, 10.0, 30.0, 30.0);
        let stroke = Stroke::new(Color::BLACK, 3.0);
        for k in [
            IconKind::Back,
            IconKind::Forward,
            IconKind::LeftArrow,
            IconKind::RightArrow,
            IconKind::Information,
            IconKind::Help,
            IconKind::QnA,
            IconKind::Bookmark,
            IconKind::Reset,
            IconKind::Filter,
            IconKind::Clear,
            IconKind::Apply,
        ] {
            let nodes = icon_nodes(k, r, &stroke, None);
            assert!(!nodes.is_empty(), "{k:?}");
            for n in nodes {
                let b = n.d.bounds().unwrap();
                assert!(r.contains_rect(&b, 0.5), "{k:?} {b:?}");
            }
        }
        assert!(icon_nodes(IconKind::Blank, r, &stroke, None).is_empty());
    }
}
