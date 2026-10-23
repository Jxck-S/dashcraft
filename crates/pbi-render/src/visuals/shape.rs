//! Shape (`shape`, legacy `basicShape`): rectangle, rounded rectangle,
//! oval, line, triangles, polygons, arrows, … with fill, outline, rotation
//! and optional text. (owner: visuals-basics)
//!
//! # Formatting objects
//!
//! | Object.property | Meaning | Default |
//! |---|---|---|
//! | `shape.tileShape` (also `shape.shapeType`, `general.shapeType`) | the geometry, see [`ShapeKind::parse`]; unknown names draw a rectangle | `rectangle` |
//! | `shape.roundEdge` (also `line.roundEdge`, `outline.roundEdge`) | corner radius (px) of rectangles and tabs | `0`, `10` for rounded rectangles |
//! | `fill.show` / `fill.fillColor` / `fill.transparency` | fill | `true` / theme data color 1 / `0` |
//! | `outline.show` / `lineColor` / `weight` / `transparency` (legacy `line.*`) | outline, drawn inside the bounds | `true` / data color 1 darkened 50% / `1` / `0` |
//! | `rotation.shapeAngle` (legacy `rotation.angle`) | clockwise rotation (degrees) about the centre | `0` |
//! | `text.show` / `text.text` / font properties / `horizontalAlignment` / `verticalAlignment` | text inside the shape | `false` / – / `label` class / center / middle |
//!
//! Shapes cover the whole container (inside its border; padding does not
//! apply) and are clipped to it. A page header band, for example, is a
//! `rectangle` shape with a solid fill and `outline.show = false`.

use crate::layout::TextAlign;
use crate::scene::{Clip, Group, Paint, PathBuilder, PathData, PathNode, Rect, Stroke, Transform};
use crate::theme::PropLookup;

use super::VisualCtx;
use super::common::{family_metrics, text_node, wrap};

/// Arrow direction of [`ShapeKind::Arrow`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Right,
    Left,
    Up,
    Down,
}

/// A shape geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShapeKind {
    Rectangle,
    RoundedRectangle,
    Oval,
    Line,
    Triangle,
    RightTriangle,
    Pentagon,
    Hexagon,
    Octagon,
    Parallelogram,
    Trapezoid,
    Diamond,
    Arrow(Direction),
    Chevron,
    PentagonArrow,
    Heart,
    Plus,
    Star,
    /// Rectangle with the two top corners rounded.
    TabRoundedTop,
    /// Rectangle with the two top corners cut.
    TabCutTop,
    /// Rectangle with the top-left and bottom-right corners rounded.
    TabRoundedCorners,
    /// Rectangle with the top-left and bottom-right corners cut.
    TabCutCorners,
}

impl ShapeKind {
    /// Parses a `tileShape`/`shapeType` name (case, spaces and punctuation
    /// ignored). Unknown names are `None` (callers fall back to
    /// [`ShapeKind::Rectangle`]).
    pub fn parse(name: &str) -> Option<Self> {
        let n: String = name
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
            .collect();
        let dir = || {
            if n.contains("left") {
                Direction::Left
            } else if n.contains("up") || n.contains("top") {
                Direction::Up
            } else if n.contains("down") || n.contains("bottom") {
                Direction::Down
            } else {
                Direction::Right
            }
        };
        let kind = match n.as_str() {
            "" | "rectangle" | "rect" | "square" => ShapeKind::Rectangle,
            "oval" | "ellipse" | "circle" => ShapeKind::Oval,
            "line" | "horizontalline" | "straightline" => ShapeKind::Line,
            "parallelogram" => ShapeKind::Parallelogram,
            "trapezoid" | "trapezium" => ShapeKind::Trapezoid,
            "diamond" | "rhombus" => ShapeKind::Diamond,
            "heart" => ShapeKind::Heart,
            "plus" | "cross" => ShapeKind::Plus,
            "hexagon" => ShapeKind::Hexagon,
            "octagon" => ShapeKind::Octagon,
            _ if n.starts_with("tab") => match (n.contains("cut"), n.contains("top")) {
                (true, true) => ShapeKind::TabCutTop,
                (true, false) => ShapeKind::TabCutCorners,
                (false, true) => ShapeKind::TabRoundedTop,
                (false, false) => ShapeKind::TabRoundedCorners,
            },
            _ if n.contains("rect") && n.contains("round") => ShapeKind::RoundedRectangle,
            _ if n.contains("chevron") => ShapeKind::Chevron,
            _ if n.contains("pentagon") && n.contains("arrow") => ShapeKind::PentagonArrow,
            _ if n.contains("pentagon") => ShapeKind::Pentagon,
            _ if n.contains("arrow") => ShapeKind::Arrow(dir()),
            _ if n.contains("righttriangle") => ShapeKind::RightTriangle,
            _ if n.contains("triangle") => ShapeKind::Triangle,
            _ if n.starts_with("star") => ShapeKind::Star,
            _ => return None,
        };
        Some(kind)
    }

    /// Whether the outline is the whole shape (no fill).
    pub fn is_line(self) -> bool {
        self == ShapeKind::Line
    }
}

/// A polygon from points in the unit square mapped onto `r`.
fn poly(r: Rect, pts: &[(f64, f64)]) -> PathData {
    let mut b = PathBuilder::new();
    for (i, (u, v)) in pts.iter().enumerate() {
        let (x, y) = (r.x + u * r.width, r.y + v * r.height);
        if i == 0 {
            b.move_to(x, y);
        } else {
            b.line_to(x, y);
        }
    }
    b.close();
    b.build()
}

/// An ellipse inscribed in `r`.
pub fn ellipse(r: Rect) -> PathData {
    let (rx, ry) = (r.width / 2.0, r.height / 2.0);
    let (cx, cy) = r.center();
    let mut b = PathBuilder::new();
    b.move_to(cx - rx, cy)
        .arc_to(rx, ry, 0.0, true, true, cx + rx, cy)
        .arc_to(rx, ry, 0.0, true, true, cx - rx, cy)
        .close();
    b.build()
}

/// A rectangle with selected corners rounded (`[tl, tr, br, bl]`).
fn rounded_corners(r: Rect, radius: f64, corners: [bool; 4]) -> PathData {
    let rad = radius.max(0.0).min(r.width / 2.0).min(r.height / 2.0);
    let k = |on: bool| if on { rad } else { 0.0 };
    let (tl, tr, br, bl) = (k(corners[0]), k(corners[1]), k(corners[2]), k(corners[3]));
    let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
    let mut b = PathBuilder::new();
    b.move_to(x0 + tl, y0).line_to(x1 - tr, y0);
    if tr > 0.0 {
        b.arc_to(tr, tr, 0.0, false, true, x1, y0 + tr);
    }
    b.line_to(x1, y1 - br);
    if br > 0.0 {
        b.arc_to(br, br, 0.0, false, true, x1 - br, y1);
    }
    b.line_to(x0 + bl, y1);
    if bl > 0.0 {
        b.arc_to(bl, bl, 0.0, false, true, x0, y1 - bl);
    }
    b.line_to(x0, y0 + tl);
    if tl > 0.0 {
        b.arc_to(tl, tl, 0.0, false, true, x0 + tl, y0);
    }
    b.close();
    b.build()
}

/// A rectangle with selected corners cut by `cut` px (`[tl, tr, br, bl]`).
fn cut_corners(r: Rect, cut: f64, corners: [bool; 4]) -> PathData {
    let c = cut.max(0.0).min(r.width / 2.0).min(r.height / 2.0);
    let k = |on: bool| if on { c } else { 0.0 };
    let (tl, tr, br, bl) = (k(corners[0]), k(corners[1]), k(corners[2]), k(corners[3]));
    let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
    let mut b = PathBuilder::new();
    b.move_to(x0 + tl, y0)
        .line_to(x1 - tr, y0)
        .line_to(x1, y0 + tr)
        .line_to(x1, y1 - br)
        .line_to(x1 - br, y1)
        .line_to(x0 + bl, y1)
        .line_to(x0, y1 - bl)
        .line_to(x0, y0 + tl)
        .close();
    b.build()
}

/// The closed outline of `kind` in `r` (`None` for [`ShapeKind::Line`],
/// which is a stroke through the centre). `radius` applies to rectangles
/// and tabs.
pub fn shape_path(kind: ShapeKind, r: Rect, radius: f64) -> Option<PathData> {
    if r.is_empty() {
        return None;
    }
    let tab = if radius > 0.0 {
        radius
    } else {
        r.width.min(r.height) * 0.2
    };
    let d = match kind {
        ShapeKind::Line => return None,
        ShapeKind::Rectangle => PathData::rounded_rect(r, radius),
        ShapeKind::RoundedRectangle => PathData::rounded_rect(r, radius),
        ShapeKind::Oval => ellipse(r),
        ShapeKind::Triangle => poly(r, &[(0.5, 0.0), (1.0, 1.0), (0.0, 1.0)]),
        ShapeKind::RightTriangle => poly(r, &[(0.0, 0.0), (1.0, 1.0), (0.0, 1.0)]),
        ShapeKind::Pentagon => poly(
            r,
            &[
                (0.5, 0.0),
                (1.0, 0.382),
                (0.809, 1.0),
                (0.191, 1.0),
                (0.0, 0.382),
            ],
        ),
        ShapeKind::Hexagon => poly(
            r,
            &[
                (0.25, 0.0),
                (0.75, 0.0),
                (1.0, 0.5),
                (0.75, 1.0),
                (0.25, 1.0),
                (0.0, 0.5),
            ],
        ),
        ShapeKind::Octagon => poly(
            r,
            &[
                (0.293, 0.0),
                (0.707, 0.0),
                (1.0, 0.293),
                (1.0, 0.707),
                (0.707, 1.0),
                (0.293, 1.0),
                (0.0, 0.707),
                (0.0, 0.293),
            ],
        ),
        ShapeKind::Parallelogram => poly(r, &[(0.25, 0.0), (1.0, 0.0), (0.75, 1.0), (0.0, 1.0)]),
        ShapeKind::Trapezoid => poly(r, &[(0.25, 0.0), (0.75, 0.0), (1.0, 1.0), (0.0, 1.0)]),
        ShapeKind::Diamond => poly(r, &[(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)]),
        ShapeKind::Chevron => poly(
            r,
            &[
                (0.0, 0.0),
                (0.75, 0.0),
                (1.0, 0.5),
                (0.75, 1.0),
                (0.0, 1.0),
                (0.25, 0.5),
            ],
        ),
        ShapeKind::PentagonArrow => poly(
            r,
            &[(0.0, 0.0), (0.75, 0.0), (1.0, 0.5), (0.75, 1.0), (0.0, 1.0)],
        ),
        ShapeKind::Arrow(dir) => {
            let right: [(f64, f64); 7] = [
                (0.0, 0.25),
                (0.6, 0.25),
                (0.6, 0.0),
                (1.0, 0.5),
                (0.6, 1.0),
                (0.6, 0.75),
                (0.0, 0.75),
            ];
            let pts: Vec<(f64, f64)> = right
                .iter()
                .map(|&(u, v)| match dir {
                    Direction::Right => (u, v),
                    Direction::Left => (1.0 - u, v),
                    Direction::Down => (v, u),
                    Direction::Up => (v, 1.0 - u),
                })
                .collect();
            poly(r, &pts)
        }
        ShapeKind::Plus => poly(
            r,
            &[
                (1.0 / 3.0, 0.0),
                (2.0 / 3.0, 0.0),
                (2.0 / 3.0, 1.0 / 3.0),
                (1.0, 1.0 / 3.0),
                (1.0, 2.0 / 3.0),
                (2.0 / 3.0, 2.0 / 3.0),
                (2.0 / 3.0, 1.0),
                (1.0 / 3.0, 1.0),
                (1.0 / 3.0, 2.0 / 3.0),
                (0.0, 2.0 / 3.0),
                (0.0, 1.0 / 3.0),
                (1.0 / 3.0, 1.0 / 3.0),
            ],
        ),
        ShapeKind::Star => {
            let pts: Vec<(f64, f64)> = (0..10)
                .map(|i| {
                    let rad = if i % 2 == 0 { 0.5 } else { 0.2 };
                    let a = (f64::from(i) * 36.0).to_radians();
                    (0.5 + rad * a.sin(), 0.5 - rad * a.cos())
                })
                .collect();
            poly(r, &pts)
        }
        ShapeKind::Heart => {
            let (x, y, w, h) = (r.x, r.y, r.width, r.height);
            let p = |u: f64, v: f64| (x + u * w, y + v * h);
            let mut b = PathBuilder::new();
            let (sx, sy) = p(0.5, 0.25);
            b.move_to(sx, sy);
            let (c1x, c1y) = p(0.5, 0.0);
            let (c2x, c2y) = p(0.0, 0.0);
            let (ex, ey) = p(0.0, 0.3);
            b.cubic_to(c1x, c1y, c2x, c2y, ex, ey);
            let (c1x, c1y) = p(0.0, 0.6);
            let (c2x, c2y) = p(0.35, 0.75);
            let (ex, ey) = p(0.5, 1.0);
            b.cubic_to(c1x, c1y, c2x, c2y, ex, ey);
            let (c1x, c1y) = p(0.65, 0.75);
            let (c2x, c2y) = p(1.0, 0.6);
            let (ex, ey) = p(1.0, 0.3);
            b.cubic_to(c1x, c1y, c2x, c2y, ex, ey);
            let (c1x, c1y) = p(1.0, 0.0);
            let (c2x, c2y) = p(0.5, 0.0);
            b.cubic_to(c1x, c1y, c2x, c2y, sx, sy);
            b.close();
            b.build()
        }
        ShapeKind::TabRoundedTop => rounded_corners(r, tab, [true, true, false, false]),
        ShapeKind::TabRoundedCorners => rounded_corners(r, tab, [true, false, true, false]),
        ShapeKind::TabCutTop => cut_corners(r, tab, [true, true, false, false]),
        ShapeKind::TabCutCorners => cut_corners(r, tab, [true, false, true, false]),
    };
    Some(d)
}

/// The shape geometry of a visual's objects.
fn kind_of(o: &PropLookup<'_>) -> ShapeKind {
    o.text("shape", "tileShape")
        .or_else(|| o.text("shape", "shapeType"))
        .or_else(|| o.text("general", "shapeType"))
        .and_then(|n| ShapeKind::parse(&n))
        .unwrap_or(ShapeKind::Rectangle)
}

/// First value of `prop` in the new `outline` object, else the legacy
/// `line` object.
fn outline_number(o: &PropLookup<'_>, prop: &str) -> Option<f64> {
    o.number("outline", prop).or_else(|| o.number("line", prop))
}

/// Renders the shape (see the module docs).
pub fn render(ctx: &VisualCtx<'_>) -> Option<Group> {
    let props = ctx.theme.visual(ctx.visual);
    let o = props.objects();
    let theme = ctx.theme;
    let area = ctx.frame.inner;
    if area.is_empty() {
        return None;
    }
    let kind = kind_of(&o);
    let accent = theme.data_color(0);

    let fill = o
        .bool("fill", "show")
        .unwrap_or(true)
        .then(|| {
            let color = o.color("fill", "fillColor").unwrap_or(accent);
            let t = o
                .number("fill", "transparency")
                .unwrap_or(0.0)
                .clamp(0.0, 100.0);
            Paint::with_transparency(color, t)
        })
        .filter(Paint::is_visible);

    let outline_shown = o
        .bool("outline", "show")
        .or_else(|| o.bool("line", "show"))
        .unwrap_or(true);
    let stroke = outline_shown
        .then(|| {
            let color = o
                .color("outline", "lineColor")
                .or_else(|| o.color("line", "lineColor"))
                .unwrap_or_else(|| accent.shade(-0.5));
            let weight = outline_number(&o, "weight")
                .unwrap_or(1.0)
                .clamp(0.0, area.width.min(area.height).max(0.0));
            let t = outline_number(&o, "transparency")
                .unwrap_or(0.0)
                .clamp(0.0, 100.0);
            Stroke::new(color, weight).with_opacity(1.0 - t / 100.0)
        })
        .filter(Stroke::is_visible);

    let radius = o
        .number("shape", "roundEdge")
        .or_else(|| outline_number(&o, "roundEdge"))
        .filter(|r| *r > 0.0)
        .unwrap_or(if kind == ShapeKind::RoundedRectangle {
            10.0
        } else {
            0.0
        });
    let angle = o
        .number("rotation", "shapeAngle")
        .or_else(|| o.number("rotation", "angle"))
        .unwrap_or(0.0);

    let mut shape = Group::new();
    let (cx, cy) = area.center();
    if angle.is_finite() && angle.rem_euclid(360.0) != 0.0 {
        shape = shape.with_transform(Transform::rotate_about(angle, cx, cy));
    }
    if kind.is_line() {
        let s = stroke.clone().or_else(|| {
            fill.map(|p| {
                Stroke::new(p.color, 1.0_f64.max(area.height.min(3.0))).with_opacity(p.opacity)
            })
        });
        if let Some(s) = s {
            shape.line(area.x, cy, area.right(), cy, s);
        }
    } else {
        let inset = stroke.as_ref().map_or(0.0, |s| s.width / 2.0);
        let r = area.inset_uniform(inset);
        if let Some(d) = shape_path(kind, r, (radius - inset).max(0.0)) {
            let mut node = PathNode::new(d);
            node.fill = fill;
            node.stroke = stroke;
            if node.fill.is_some() || node.stroke.is_some() {
                shape.push(node);
            }
        }
    }
    let mut g = Group::new().with_clip(Clip::rect(ctx.frame.outer));
    if !shape.is_empty() {
        g.push(shape);
    }
    if let Some(text) = shape_text(ctx, &o) {
        g.push(text);
    }
    (!g.is_empty()).then_some(g)
}

/// Optional text inside the shape (`text.show`, `text.text`).
fn shape_text(ctx: &VisualCtx<'_>, o: &PropLookup<'_>) -> Option<Group> {
    if !o.bool("text", "show").unwrap_or(false) {
        return None;
    }
    let text = o.text("text", "text").filter(|t| !t.trim().is_empty())?;
    let style = o.text_style("text", "label");
    let c = ctx.frame.content;
    let lh = family_metrics(&style).line_height_px(&style);
    if c.width <= 0.0 || lh <= 0.0 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let max_lines = ((c.height / lh).floor().max(1.0) as usize).min(500);
    let lines = wrap(ctx.fonts, &text, &style, c.width, Some(max_lines));
    let align = o
        .text("text", "horizontalAlignment")
        .map_or(TextAlign::Center, |a| TextAlign::parse(&a));
    let valign = o
        .text("text", "verticalAlignment")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let block = lh * lines.len() as f64;
    let top = match valign.as_str() {
        "top" => c.y,
        "bottom" => c.bottom() - block,
        _ => c.y + (c.height - block) / 2.0,
    };
    let (x, anchor) = match align {
        TextAlign::Left => (c.x, crate::scene::TextAnchor::Start),
        TextAlign::Center => (c.x + c.width / 2.0, crate::scene::TextAnchor::Middle),
        TextAlign::Right => (c.right(), crate::scene::TextAnchor::End),
    };
    let ascent = family_metrics(&style).ascent_px(&style);
    let mut g = Group::new();
    for (i, line) in lines.into_iter().enumerate() {
        g.push(text_node(
            x,
            top + lh * i as f64 + ascent,
            line,
            &style,
            anchor,
            None,
        ));
    }
    Some(g)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_names() {
        assert_eq!(ShapeKind::parse("rectangle"), Some(ShapeKind::Rectangle));
        assert_eq!(
            ShapeKind::parse("rectangleRoundedByPixel"),
            Some(ShapeKind::RoundedRectangle)
        );
        assert_eq!(ShapeKind::parse("Oval"), Some(ShapeKind::Oval));
        assert_eq!(
            ShapeKind::parse("arrowLeft"),
            Some(ShapeKind::Arrow(Direction::Left))
        );
        assert_eq!(
            ShapeKind::parse("pentagonArrow"),
            Some(ShapeKind::PentagonArrow)
        );
        assert_eq!(
            ShapeKind::parse("Right triangle"),
            Some(ShapeKind::RightTriangle)
        );
        assert_eq!(
            ShapeKind::parse("tabCutTopCorners"),
            Some(ShapeKind::TabCutTop)
        );
        assert_eq!(ShapeKind::parse("cloud"), None);
    }

    #[test]
    fn every_kind_has_a_path_in_bounds() {
        let r = Rect::new(10.0, 20.0, 100.0, 50.0);
        let kinds = [
            ShapeKind::Rectangle,
            ShapeKind::RoundedRectangle,
            ShapeKind::Oval,
            ShapeKind::Triangle,
            ShapeKind::RightTriangle,
            ShapeKind::Pentagon,
            ShapeKind::Hexagon,
            ShapeKind::Octagon,
            ShapeKind::Parallelogram,
            ShapeKind::Trapezoid,
            ShapeKind::Diamond,
            ShapeKind::Arrow(Direction::Up),
            ShapeKind::Arrow(Direction::Down),
            ShapeKind::Chevron,
            ShapeKind::PentagonArrow,
            ShapeKind::Heart,
            ShapeKind::Plus,
            ShapeKind::Star,
            ShapeKind::TabRoundedTop,
            ShapeKind::TabCutTop,
            ShapeKind::TabRoundedCorners,
            ShapeKind::TabCutCorners,
        ];
        for k in kinds {
            let d = shape_path(k, r, 5.0).unwrap();
            let b = d.bounds().unwrap();
            assert!(
                r.inset_uniform(-0.01).contains_rect(&b, 0.01),
                "{k:?} {b:?}"
            );
        }
        assert!(shape_path(ShapeKind::Line, r, 0.0).is_none());
        assert!(shape_path(ShapeKind::Oval, Rect::new(0.0, 0.0, 0.0, 5.0), 0.0).is_none());
    }
}
