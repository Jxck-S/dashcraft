//! Built-in conditional-formatting icons, drawn as simple deterministic
//! vector shapes (no image assets).
//!
//! PBIR stores an icon as a name (`'circle_green'`, `'ArrowUp'`, …).
//! [`Icon::parse`] reads names tolerantly: it splits them into words
//! (separators and camelCase) and looks for a shape word (`circle`,
//! `light`, `square`, `diamond`, `triangle`, `arrow`, `dash`, `flag`,
//! `check`, `x`/`cross`, `exclamation`/`warning`, `star`, `bar`/`signal`)
//! plus optional direction (`up`, `down`, `right`, `left`, `upright`/
//! `diagonal`), fill (`half`, `quarter`, `empty`, digits) and color
//! (`red`, `yellow`/`amber`, `orange`, `green`, `gray`/`grey`/`silver`,
//! `black`, `blue`) words. Without a color word an icon takes its usual
//! status color (up arrows green, down arrows red, …). Unknown names give
//! no icon.

use crate::scene::{Clip, Color, Group, Paint, PathBuilder, PathData, PathNode, Rect, Stroke};

/// Status colors of the icons.
pub const GREEN: Color = Color::rgb(0x1A, 0xAB, 0x40);
pub const YELLOW: Color = Color::rgb(0xF2, 0xC8, 0x0F);
pub const ORANGE: Color = Color::rgb(0xF7, 0x8C, 0x2E);
pub const RED: Color = Color::rgb(0xD6, 0x45, 0x54);
pub const GRAY: Color = Color::rgb(0x9C, 0x9C, 0x9C);
pub const BLACK: Color = Color::rgb(0x25, 0x24, 0x23);
pub const BLUE: Color = Color::rgb(0x11, 0x8D, 0xFF);
/// Unfilled parts of stars and signal bars.
const TRACK: Color = Color::rgb(0xC8, 0xC6, 0xC4);

/// The shape of an icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconShape {
    Circle,
    /// A circle with a dark rim (traffic lights with rims).
    RimmedCircle,
    Square,
    Diamond,
    TriangleUp,
    TriangleDown,
    /// A short horizontal bar (no change).
    Dash,
    ArrowUp,
    ArrowDown,
    ArrowRight,
    ArrowLeft,
    ArrowUpRight,
    ArrowDownRight,
    Flag,
    Check,
    Cross,
    /// `!` in a circle.
    Exclamation,
    /// `!` in a triangle.
    Warning,
    /// A star filled `n` quarters (0 = outline only, 4 = full).
    Star(u8),
    /// Four signal bars, `n` of them filled.
    Bars(u8),
}

/// A shape in a color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Icon {
    pub shape: IconShape,
    pub color: Color,
}

/// Words of an icon name: split at non-alphanumerics, lowercase → uppercase
/// and letter ↔ digit boundaries, lowercased.
fn words(name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev: Option<char> = None;
    for c in name.chars().take(80) {
        if !c.is_ascii_alphanumeric() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev = None;
            continue;
        }
        let split = prev.is_some_and(|p| {
            (p.is_ascii_lowercase() && c.is_ascii_uppercase())
                || (p.is_ascii_digit() != c.is_ascii_digit())
        });
        if split && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        cur.push(c.to_ascii_lowercase());
        prev = Some(c);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

impl Icon {
    /// Reads an icon name (see the module docs); quotes are ignored.
    pub fn parse(name: &str) -> Option<Icon> {
        let w = words(name.trim().trim_matches('\''));
        let has = |k: &str| w.iter().any(|x| x == k);
        let any = |ks: &[&str]| ks.iter().any(|k| has(k));
        let digit = w.iter().find_map(|x| x.parse::<u8>().ok());
        let up = any(&["up", "upward", "increase", "rising"]);
        let down = any(&["down", "downward", "decrease", "falling"]);
        let right = any(&["right", "sideways", "side", "flat", "neutral", "same"]);
        let diagonal = any(&["diagonal", "upright", "downright"]);
        use IconShape as S;
        let shape = if any(&["exclamation", "warning", "alert", "caution"]) {
            if has("circle") {
                S::Exclamation
            } else {
                S::Warning
            }
        } else if any(&["check", "checkmark", "tick", "ok"]) {
            S::Check
        } else if any(&["x", "cross", "close", "xmark", "times"]) {
            S::Cross
        } else if has("flag") {
            S::Flag
        } else if has("star") || has("stars") {
            let fill = if any(&["empty", "outline", "hollow", "none", "zero"]) {
                0
            } else if has("quarter") {
                if has("three") { 3 } else { 1 }
            } else if has("half") {
                2
            } else {
                digit.map_or(4, |d| d.min(4))
            };
            S::Star(fill)
        } else if any(&["bar", "bars", "signal", "strength", "rating"]) {
            let n = if any(&["empty", "none", "zero"]) {
                0
            } else if has("low") {
                1
            } else if has("medium") {
                2
            } else if has("high") {
                3
            } else {
                digit.map_or(4, |d| d.min(4))
            };
            S::Bars(n)
        } else if has("arrow") || has("arrows") {
            match (up, down, right || diagonal, has("left")) {
                (true, false, true, _) => S::ArrowUpRight,
                (false, true, true, _) => S::ArrowDownRight,
                (true, false, _, _) => S::ArrowUp,
                (false, true, _, _) => S::ArrowDown,
                (_, _, _, true) => S::ArrowLeft,
                _ if diagonal && has("downright") => S::ArrowDownRight,
                _ if diagonal => S::ArrowUpRight,
                _ => S::ArrowRight,
            }
        } else if has("triangle") || has("triangles") {
            if up {
                S::TriangleUp
            } else if down {
                S::TriangleDown
            } else {
                S::Dash
            }
        } else if any(&["dash", "minus", "hyphen", "line"]) {
            S::Dash
        } else if has("square") {
            S::Square
        } else if has("diamond") {
            S::Diamond
        } else if any(&[
            "circle", "circles", "dot", "ball", "light", "lights", "traffic",
        ]) {
            if any(&["rim", "rimmed", "border"]) {
                S::RimmedCircle
            } else {
                S::Circle
            }
        } else if up {
            S::TriangleUp
        } else if down {
            S::TriangleDown
        } else {
            return None;
        };
        let color = if any(&["red", "bad", "negative"]) {
            RED
        } else if any(&["yellow", "amber", "gold"]) {
            YELLOW
        } else if has("orange") {
            ORANGE
        } else if any(&["green", "good", "positive"]) {
            GREEN
        } else if any(&["gray", "grey", "silver"]) {
            GRAY
        } else if has("black") {
            BLACK
        } else if has("blue") {
            BLUE
        } else {
            default_color(shape)
        };
        Some(Icon { shape, color })
    }

    /// The icon in the `size`×`size` box at `(x, y)`.
    pub fn draw(&self, x: f64, y: f64, size: f64) -> Group {
        let mut g = Group::new().with_class("pbi-cf-icon");
        let s = size.max(1.0);
        let fill = Paint::solid(self.color);
        let (cx, cy, r) = (x + s / 2.0, y + s / 2.0, s * 0.42);
        let poly = |pts: &[(f64, f64)]| {
            let mut b = PathBuilder::new();
            for (i, (px, py)) in pts.iter().enumerate() {
                let (px, py) = (x + px * s, y + py * s);
                if i == 0 {
                    b.move_to(px, py);
                } else {
                    b.line_to(px, py);
                }
            }
            b.close();
            b.build()
        };
        let disc = |radius: f64| PathData::pie_slice(cx, cy, radius, 0.0, 0.0, 360.0);
        let bang = |g: &mut Group, top: f64, bottom: f64| {
            let w = (s * 0.12).max(1.0);
            g.rect(
                Rect::new(cx - w / 2.0, y + top * s, w, (bottom - top) * s),
                Some(Paint::solid(Color::WHITE)),
                None,
            );
            g.rect(
                Rect::new(cx - w / 2.0, y + (bottom + 0.08) * s, w, w),
                Some(Paint::solid(Color::WHITE)),
                None,
            );
        };
        match self.shape {
            IconShape::Circle => {
                g.push(PathNode::new(disc(r)).fill(fill));
            }
            IconShape::RimmedCircle => {
                g.push(
                    PathNode::new(disc(r))
                        .fill(fill)
                        .stroke(Stroke::new(BLACK, (s * 0.08).max(1.0))),
                );
            }
            IconShape::Square => {
                let m = s * 0.12;
                g.rect(
                    Rect::new(x + m, y + m, s - 2.0 * m, s - 2.0 * m),
                    Some(fill),
                    None,
                );
            }
            IconShape::Diamond => {
                g.push(
                    PathNode::new(poly(&[(0.5, 0.05), (0.95, 0.5), (0.5, 0.95), (0.05, 0.5)]))
                        .fill(fill),
                );
            }
            IconShape::TriangleUp => {
                g.push(PathNode::new(poly(&[(0.5, 0.15), (0.92, 0.85), (0.08, 0.85)])).fill(fill));
            }
            IconShape::TriangleDown => {
                g.push(PathNode::new(poly(&[(0.08, 0.15), (0.92, 0.15), (0.5, 0.85)])).fill(fill));
            }
            IconShape::Dash => {
                g.rect(
                    Rect::new(x + s * 0.12, y + s * 0.38, s * 0.76, s * 0.24),
                    Some(fill),
                    None,
                );
            }
            IconShape::ArrowUp => {
                g.push(PathNode::new(poly(&ARROW_UP)).fill(fill));
            }
            IconShape::ArrowDown => {
                let pts: Vec<(f64, f64)> =
                    ARROW_UP.iter().map(|(px, py)| (*px, 1.0 - py)).collect();
                g.push(PathNode::new(poly(&pts)).fill(fill));
            }
            IconShape::ArrowRight | IconShape::ArrowLeft => {
                let flip = self.shape == IconShape::ArrowLeft;
                let pts: Vec<(f64, f64)> = ARROW_UP
                    .iter()
                    .map(|(px, py)| if flip { (*py, *px) } else { (1.0 - py, *px) })
                    .collect();
                g.push(PathNode::new(poly(&pts)).fill(fill));
            }
            IconShape::ArrowUpRight | IconShape::ArrowDownRight => {
                // The up arrow turned clockwise by 45° or 135° about the centre.
                let deg: f64 = if self.shape == IconShape::ArrowUpRight {
                    45.0
                } else {
                    135.0
                };
                let (sin, cos) = deg.to_radians().sin_cos();
                let pts: Vec<(f64, f64)> = ARROW_UP
                    .iter()
                    .map(|(px, py)| {
                        let (dx, dy) = (px - 0.5, py - 0.5);
                        (0.5 + dx * cos - dy * sin, 0.5 + dx * sin + dy * cos)
                    })
                    .collect();
                g.push(PathNode::new(poly(&pts)).fill(fill));
            }
            IconShape::Flag => {
                let w = (s * 0.1).max(1.0);
                g.rect(
                    Rect::new(x + s * 0.18, y + s * 0.08, w, s * 0.84),
                    Some(fill),
                    None,
                );
                g.push(PathNode::new(poly(&[(0.18, 0.08), (0.88, 0.3), (0.18, 0.52)])).fill(fill));
            }
            IconShape::Check => {
                let mut b = PathBuilder::new();
                b.move_to(x + s * 0.12, y + s * 0.52)
                    .line_to(x + s * 0.4, y + s * 0.8)
                    .line_to(x + s * 0.9, y + s * 0.2);
                g.push(
                    PathNode::new(b.build()).stroke(Stroke::new(self.color, (s * 0.16).max(1.0))),
                );
            }
            IconShape::Cross => {
                let stroke = Stroke::new(self.color, (s * 0.16).max(1.0));
                g.line(
                    x + s * 0.18,
                    y + s * 0.18,
                    x + s * 0.82,
                    y + s * 0.82,
                    stroke.clone(),
                );
                g.line(
                    x + s * 0.82,
                    y + s * 0.18,
                    x + s * 0.18,
                    y + s * 0.82,
                    stroke,
                );
            }
            IconShape::Exclamation => {
                g.push(PathNode::new(disc(r)).fill(fill));
                bang(&mut g, 0.22, 0.6);
            }
            IconShape::Warning => {
                g.push(PathNode::new(poly(&[(0.5, 0.06), (0.96, 0.9), (0.04, 0.9)])).fill(fill));
                bang(&mut g, 0.34, 0.66);
            }
            IconShape::Star(n) => {
                let star = star_path(cx, cy, r * 1.12);
                g.push(PathNode::new(star.clone()).fill(Paint::solid(TRACK)));
                let part = f64::from(n.min(4)) / 4.0;
                if part > 0.0 {
                    let mut filled =
                        Group::new().with_clip(Clip::rect(Rect::new(x, y, s * part, s)));
                    filled.push(PathNode::new(star).fill(fill));
                    g.push(filled);
                }
            }
            IconShape::Bars(n) => {
                let w = s * 0.18;
                for i in 0..4u8 {
                    let h = s * (0.3 + 0.2 * f64::from(i));
                    let color = if i < n { self.color } else { TRACK };
                    g.rect(
                        Rect::new(
                            x + s * 0.06 + f64::from(i) * s * 0.23,
                            y + s * 0.92 - h,
                            w,
                            h,
                        ),
                        Some(Paint::solid(color)),
                        None,
                    );
                }
            }
        }
        g
    }
}

/// An up arrow in the unit box (head and shaft).
const ARROW_UP: [(f64, f64); 7] = [
    (0.5, 0.06),
    (0.92, 0.5),
    (0.64, 0.5),
    (0.64, 0.94),
    (0.36, 0.94),
    (0.36, 0.5),
    (0.08, 0.5),
];

/// A five-pointed star of outer radius `r` centred on `(cx, cy)`.
fn star_path(cx: f64, cy: f64, r: f64) -> PathData {
    let mut b = PathBuilder::new();
    for k in 0..10 {
        let radius = if k % 2 == 0 { r } else { r * 0.45 };
        let a = (f64::from(k) * 36.0).to_radians();
        let (px, py) = (cx + radius * a.sin(), cy - radius * a.cos());
        if k == 0 {
            b.move_to(px, py);
        } else {
            b.line_to(px, py);
        }
    }
    b.close();
    b.build()
}

fn default_color(shape: IconShape) -> Color {
    match shape {
        IconShape::ArrowUp | IconShape::TriangleUp | IconShape::Check => GREEN,
        IconShape::ArrowDown | IconShape::TriangleDown | IconShape::Cross | IconShape::Flag => RED,
        IconShape::ArrowRight
        | IconShape::ArrowLeft
        | IconShape::ArrowUpRight
        | IconShape::ArrowDownRight
        | IconShape::Dash
        | IconShape::Exclamation
        | IconShape::Warning
        | IconShape::Star(_) => YELLOW,
        IconShape::Bars(_) => BLUE,
        IconShape::Circle | IconShape::RimmedCircle | IconShape::Square | IconShape::Diamond => {
            GRAY
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_names() {
        let p = |n: &str| Icon::parse(n).map(|i| (i.shape, i.color));
        assert_eq!(p("'circle_green'"), Some((IconShape::Circle, GREEN)));
        assert_eq!(p("CircleRed"), Some((IconShape::Circle, RED)));
        assert_eq!(
            p("trafficLightRimmedYellow"),
            Some((IconShape::RimmedCircle, YELLOW))
        );
        assert_eq!(p("arrow_up"), Some((IconShape::ArrowUp, GREEN)));
        assert_eq!(p("ArrowDownGray"), Some((IconShape::ArrowDown, GRAY)));
        assert_eq!(p("arrow_up_right"), Some((IconShape::ArrowUpRight, YELLOW)));
        assert_eq!(p("arrow-sideways"), Some((IconShape::ArrowRight, YELLOW)));
        assert_eq!(p("triangle_down_red"), Some((IconShape::TriangleDown, RED)));
        assert_eq!(p("triangle_dash"), Some((IconShape::Dash, YELLOW)));
        assert_eq!(
            p("exclamation_triangle"),
            Some((IconShape::Warning, YELLOW))
        );
        assert_eq!(p("x_mark"), Some((IconShape::Cross, RED)));
        assert_eq!(p("checkmark_green"), Some((IconShape::Check, GREEN)));
        assert_eq!(p("star_half"), Some((IconShape::Star(2), YELLOW)));
        assert_eq!(p("starEmpty"), Some((IconShape::Star(0), YELLOW)));
        assert_eq!(p("signal_3"), Some((IconShape::Bars(3), BLUE)));
        assert_eq!(p("flag_green"), Some((IconShape::Flag, GREEN)));
        assert_eq!(p("https://example.com/icon.png"), None);
        assert_eq!(p(""), None);
    }

    #[test]
    fn every_shape_draws_inside_its_box() {
        let shapes = [
            IconShape::Circle,
            IconShape::RimmedCircle,
            IconShape::Square,
            IconShape::Diamond,
            IconShape::TriangleUp,
            IconShape::TriangleDown,
            IconShape::Dash,
            IconShape::ArrowUp,
            IconShape::ArrowDown,
            IconShape::ArrowRight,
            IconShape::ArrowLeft,
            IconShape::ArrowUpRight,
            IconShape::ArrowDownRight,
            IconShape::Flag,
            IconShape::Check,
            IconShape::Cross,
            IconShape::Exclamation,
            IconShape::Warning,
            IconShape::Star(2),
            IconShape::Bars(1),
        ];
        for shape in shapes {
            let g = Icon { shape, color: RED }.draw(10.0, 20.0, 12.0);
            assert!(!g.is_empty(), "{shape:?}");
            for node in &g.children {
                if let crate::scene::Node::Path(p) = node {
                    let b = p.d.bounds().unwrap();
                    assert!(
                        b.x >= 9.0 && b.y >= 19.0 && b.right() <= 23.0 && b.bottom() <= 33.0,
                        "{shape:?} {b:?}"
                    );
                }
            }
        }
    }
}
