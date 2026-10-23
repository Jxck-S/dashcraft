//! Path geometry: [`PathData`] (serialized as an SVG `d` string) and a
//! [`PathBuilder`] with helpers for rounded rectangles and pie/donut slices.

use std::fmt::Write as _;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::geom::Rect;

/// One path command, in absolute coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathSeg {
    MoveTo(f64, f64),
    LineTo(f64, f64),
    /// Quadratic Bézier: control point, end point.
    QuadTo(f64, f64, f64, f64),
    /// Cubic Bézier: two control points, end point.
    CubicTo(f64, f64, f64, f64, f64, f64),
    /// SVG elliptical arc to `(x, y)`.
    ArcTo {
        rx: f64,
        ry: f64,
        x_rotation: f64,
        large_arc: bool,
        sweep: bool,
        x: f64,
        y: f64,
    },
    Close,
}

/// Path geometry. Serializes as a deterministic SVG path `d` string
/// (absolute commands, numbers rounded to 3 decimals).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PathData {
    pub segs: Vec<PathSeg>,
}

/// Formats a coordinate for SVG/JSON output: rounded to 3 decimals, no
/// trailing zeros, no `-0`, non-finite values become `0`.
pub fn format_number(v: f64) -> String {
    if !v.is_finite() {
        return "0".to_owned();
    }
    let r = (v * 1000.0).round() / 1000.0;
    if r == 0.0 {
        return "0".to_owned();
    }
    let mut s = format!("{r:.3}");
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    s
}

impl PathData {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.segs.is_empty()
    }

    /// The SVG `d` attribute value.
    pub fn to_d(&self) -> String {
        let mut out = String::new();
        let n = format_number;
        for seg in &self.segs {
            if !out.is_empty() {
                out.push(' ');
            }
            // Writing to a String cannot fail.
            let _ = match *seg {
                PathSeg::MoveTo(x, y) => write!(out, "M{} {}", n(x), n(y)),
                PathSeg::LineTo(x, y) => write!(out, "L{} {}", n(x), n(y)),
                PathSeg::QuadTo(x1, y1, x, y) => {
                    write!(out, "Q{} {} {} {}", n(x1), n(y1), n(x), n(y))
                }
                PathSeg::CubicTo(x1, y1, x2, y2, x, y) => write!(
                    out,
                    "C{} {} {} {} {} {}",
                    n(x1),
                    n(y1),
                    n(x2),
                    n(y2),
                    n(x),
                    n(y)
                ),
                PathSeg::ArcTo {
                    rx,
                    ry,
                    x_rotation,
                    large_arc,
                    sweep,
                    x,
                    y,
                } => write!(
                    out,
                    "A{} {} {} {} {} {} {}",
                    n(rx),
                    n(ry),
                    n(x_rotation),
                    u8::from(large_arc),
                    u8::from(sweep),
                    n(x),
                    n(y)
                ),
                PathSeg::Close => write!(out, "Z"),
            };
        }
        out
    }

    /// Parses an SVG path `d` string (absolute and relative `M L H V C Q A
    /// Z`). Returns `None` on malformed input; never panics.
    pub fn parse(d: &str) -> Option<Self> {
        PathParser::new(d).parse()
    }

    /// Conservative bounding box (Bézier control points included; arcs are
    /// sampled), or `None` for an empty path.
    pub fn bounds(&self) -> Option<Rect> {
        let mut acc = BoundsAcc::default();
        let (mut cx, mut cy) = (0.0, 0.0);
        let (mut sx, mut sy) = (0.0, 0.0);
        for seg in &self.segs {
            match *seg {
                PathSeg::MoveTo(x, y) => {
                    acc.add(x, y);
                    (cx, cy, sx, sy) = (x, y, x, y);
                }
                PathSeg::LineTo(x, y) => {
                    acc.add(x, y);
                    (cx, cy) = (x, y);
                }
                PathSeg::QuadTo(x1, y1, x, y) => {
                    acc.add(x1, y1);
                    acc.add(x, y);
                    (cx, cy) = (x, y);
                }
                PathSeg::CubicTo(x1, y1, x2, y2, x, y) => {
                    acc.add(x1, y1);
                    acc.add(x2, y2);
                    acc.add(x, y);
                    (cx, cy) = (x, y);
                }
                PathSeg::ArcTo {
                    rx,
                    ry,
                    x_rotation,
                    large_arc,
                    sweep,
                    x,
                    y,
                } => {
                    for (px, py) in
                        sample_arc((cx, cy), rx, ry, x_rotation, large_arc, sweep, (x, y))
                    {
                        acc.add(px, py);
                    }
                    acc.add(x, y);
                    (cx, cy) = (x, y);
                }
                PathSeg::Close => (cx, cy) = (sx, sy),
            }
        }
        acc.rect()
    }

    /// A rectangle with corner radius `r` (clamped to half the shorter side).
    pub fn rounded_rect(rect: Rect, r: f64) -> Self {
        let r = r.max(0.0).min(rect.width / 2.0).min(rect.height / 2.0);
        let mut b = PathBuilder::new();
        if r <= 0.0 || !r.is_finite() {
            b.move_to(rect.x, rect.y)
                .line_to(rect.right(), rect.y)
                .line_to(rect.right(), rect.bottom())
                .line_to(rect.x, rect.bottom())
                .close();
            return b.build();
        }
        let (x0, y0, x1, y1) = (rect.x, rect.y, rect.right(), rect.bottom());
        b.move_to(x0 + r, y0)
            .line_to(x1 - r, y0)
            .arc_to(r, r, 0.0, false, true, x1, y0 + r)
            .line_to(x1, y1 - r)
            .arc_to(r, r, 0.0, false, true, x1 - r, y1)
            .line_to(x0 + r, y1)
            .arc_to(r, r, 0.0, false, true, x0, y1 - r)
            .line_to(x0, y0 + r)
            .arc_to(r, r, 0.0, false, true, x0 + r, y0)
            .close();
        b.build()
    }

    /// A pie slice (`inner_r == 0`) or donut segment centred on `(cx, cy)`.
    ///
    /// Angles are in degrees, clockwise from 12 o'clock (Power BI's pie
    /// orientation). A sweep of 360° or more draws a full disc/ring.
    pub fn pie_slice(
        cx: f64,
        cy: f64,
        outer_r: f64,
        inner_r: f64,
        start_deg: f64,
        end_deg: f64,
    ) -> Self {
        let outer_r = outer_r.max(0.0);
        let inner_r = inner_r.clamp(0.0, outer_r);
        let sweep = (end_deg - start_deg).clamp(0.0, 360.0);
        let at = |r: f64, deg: f64| {
            let (s, c) = deg.to_radians().sin_cos();
            (cx + r * s, cy - r * c)
        };
        let mut b = PathBuilder::new();
        if sweep <= 0.0 || outer_r <= 0.0 || !sweep.is_finite() {
            return b.build();
        }
        if sweep >= 360.0 - 1e-9 {
            b.move_to(cx, cy - outer_r)
                .arc_to(outer_r, outer_r, 0.0, true, true, cx, cy + outer_r)
                .arc_to(outer_r, outer_r, 0.0, true, true, cx, cy - outer_r)
                .close();
            if inner_r > 0.0 {
                b.move_to(cx, cy - inner_r)
                    .arc_to(inner_r, inner_r, 0.0, true, false, cx, cy + inner_r)
                    .arc_to(inner_r, inner_r, 0.0, true, false, cx, cy - inner_r)
                    .close();
            }
            return b.build();
        }
        let large = sweep > 180.0;
        let end = start_deg + sweep;
        let (ox0, oy0) = at(outer_r, start_deg);
        let (ox1, oy1) = at(outer_r, end);
        if inner_r > 0.0 {
            let (ix0, iy0) = at(inner_r, start_deg);
            let (ix1, iy1) = at(inner_r, end);
            b.move_to(ox0, oy0)
                .arc_to(outer_r, outer_r, 0.0, large, true, ox1, oy1)
                .line_to(ix1, iy1)
                .arc_to(inner_r, inner_r, 0.0, large, false, ix0, iy0)
                .close();
        } else {
            b.move_to(cx, cy)
                .line_to(ox0, oy0)
                .arc_to(outer_r, outer_r, 0.0, large, true, ox1, oy1)
                .close();
        }
        b.build()
    }
}

impl Serialize for PathData {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_d())
    }
}

impl<'de> Deserialize<'de> for PathData {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let d = String::deserialize(deserializer)?;
        PathData::parse(&d).ok_or_else(|| serde::de::Error::custom("invalid SVG path data"))
    }
}

/// Fluent builder for [`PathData`].
#[derive(Debug, Clone, Default)]
pub struct PathBuilder {
    segs: Vec<PathSeg>,
}

impl PathBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn move_to(&mut self, x: f64, y: f64) -> &mut Self {
        self.segs.push(PathSeg::MoveTo(x, y));
        self
    }

    pub fn line_to(&mut self, x: f64, y: f64) -> &mut Self {
        self.segs.push(PathSeg::LineTo(x, y));
        self
    }

    pub fn quad_to(&mut self, x1: f64, y1: f64, x: f64, y: f64) -> &mut Self {
        self.segs.push(PathSeg::QuadTo(x1, y1, x, y));
        self
    }

    pub fn cubic_to(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, x: f64, y: f64) -> &mut Self {
        self.segs.push(PathSeg::CubicTo(x1, y1, x2, y2, x, y));
        self
    }

    #[allow(clippy::too_many_arguments)]
    pub fn arc_to(
        &mut self,
        rx: f64,
        ry: f64,
        x_rotation: f64,
        large_arc: bool,
        sweep: bool,
        x: f64,
        y: f64,
    ) -> &mut Self {
        self.segs.push(PathSeg::ArcTo {
            rx,
            ry,
            x_rotation,
            large_arc,
            sweep,
            x,
            y,
        });
        self
    }

    pub fn close(&mut self) -> &mut Self {
        self.segs.push(PathSeg::Close);
        self
    }

    pub fn build(&self) -> PathData {
        PathData {
            segs: self.segs.clone(),
        }
    }
}

#[derive(Default)]
struct BoundsAcc {
    min: Option<(f64, f64)>,
    max: (f64, f64),
}

impl BoundsAcc {
    fn add(&mut self, x: f64, y: f64) {
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        match &mut self.min {
            None => {
                self.min = Some((x, y));
                self.max = (x, y);
            }
            Some(min) => {
                min.0 = min.0.min(x);
                min.1 = min.1.min(y);
                self.max.0 = self.max.0.max(x);
                self.max.1 = self.max.1.max(y);
            }
        }
    }

    fn rect(&self) -> Option<Rect> {
        let (x0, y0) = self.min?;
        Some(Rect::from_points(x0, y0, self.max.0, self.max.1))
    }
}

/// Points along an SVG arc (endpoint parameterization, SVG 1.1 §F.6.5).
fn sample_arc(
    from: (f64, f64),
    rx: f64,
    ry: f64,
    x_rotation: f64,
    large_arc: bool,
    sweep: bool,
    to: (f64, f64),
) -> Vec<(f64, f64)> {
    let (x1, y1) = from;
    let (x2, y2) = to;
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx == 0.0 || ry == 0.0 || !rx.is_finite() || !ry.is_finite() || from == to {
        return Vec::new();
    }
    let (sin_phi, cos_phi) = x_rotation.to_radians().sin_cos();
    let dx = (x1 - x2) / 2.0;
    let dy = (y1 - y2) / 2.0;
    let x1p = cos_phi * dx + sin_phi * dy;
    let y1p = -sin_phi * dx + cos_phi * dy;
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut coef = if den > 0.0 {
        (num / den).max(0.0).sqrt()
    } else {
        0.0
    };
    if large_arc == sweep {
        coef = -coef;
    }
    let cxp = coef * rx * y1p / ry;
    let cyp = -coef * ry * x1p / rx;
    let cx = cos_phi * cxp - sin_phi * cyp + (x1 + x2) / 2.0;
    let cy = sin_phi * cxp + cos_phi * cyp + (y1 + y2) / 2.0;
    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| {
        let dot = ux * vx + uy * vy;
        let len = (ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt();
        let mut a = if len > 0.0 {
            (dot / len).clamp(-1.0, 1.0).acos()
        } else {
            0.0
        };
        if ux * vy - uy * vx < 0.0 {
            a = -a;
        }
        a
    };
    let theta1 = angle(1.0, 0.0, (x1p - cxp) / rx, (y1p - cyp) / ry);
    let mut delta = angle(
        (x1p - cxp) / rx,
        (y1p - cyp) / ry,
        (-x1p - cxp) / rx,
        (-y1p - cyp) / ry,
    );
    if !sweep && delta > 0.0 {
        delta -= std::f64::consts::TAU;
    } else if sweep && delta < 0.0 {
        delta += std::f64::consts::TAU;
    }
    const STEPS: u32 = 16;
    (0..=STEPS)
        .map(|i| {
            let t = theta1 + delta * f64::from(i) / f64::from(STEPS);
            let (s, c) = t.sin_cos();
            (
                cx + rx * c * cos_phi - ry * s * sin_phi,
                cy + rx * c * sin_phi + ry * s * cos_phi,
            )
        })
        .collect()
}

/// Minimal SVG path-data parser (used to deserialize [`PathData`]).
struct PathParser<'a> {
    s: &'a [u8],
    pos: usize,
}

impl<'a> PathParser<'a> {
    fn new(d: &'a str) -> Self {
        Self {
            s: d.as_bytes(),
            pos: 0,
        }
    }

    fn skip_ws(&mut self) {
        while let Some(&c) = self.s.get(self.pos) {
            if c.is_ascii_whitespace() || c == b',' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn peek_number_start(&mut self) -> bool {
        self.skip_ws();
        matches!(self.s.get(self.pos), Some(c) if c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.'))
    }

    fn number(&mut self) -> Option<f64> {
        self.skip_ws();
        let start = self.pos;
        let at = |p: usize| self.s.get(p).copied();
        let mut p = self.pos;
        if matches!(at(p), Some(b'+' | b'-')) {
            p += 1;
        }
        let mut digits = false;
        while matches!(at(p), Some(c) if c.is_ascii_digit()) {
            p += 1;
            digits = true;
        }
        if at(p) == Some(b'.') {
            p += 1;
            while matches!(at(p), Some(c) if c.is_ascii_digit()) {
                p += 1;
                digits = true;
            }
        }
        if !digits {
            return None;
        }
        if matches!(at(p), Some(b'e' | b'E')) {
            let mut q = p + 1;
            if matches!(at(q), Some(b'+' | b'-')) {
                q += 1;
            }
            if matches!(at(q), Some(c) if c.is_ascii_digit()) {
                while matches!(at(q), Some(c) if c.is_ascii_digit()) {
                    q += 1;
                }
                p = q;
            }
        }
        let text = std::str::from_utf8(self.s.get(start..p)?).ok()?;
        let v: f64 = text.parse().ok()?;
        self.pos = p;
        v.is_finite().then_some(v)
    }

    fn flag(&mut self) -> Option<bool> {
        self.skip_ws();
        let c = *self.s.get(self.pos)?;
        self.pos += 1;
        match c {
            b'0' => Some(false),
            b'1' => Some(true),
            _ => None,
        }
    }

    fn parse(mut self) -> Option<PathData> {
        let mut segs = Vec::new();
        let (mut cx, mut cy) = (0.0, 0.0);
        let (mut sx, mut sy) = (0.0, 0.0);
        let mut cmd: Option<u8> = None;
        loop {
            self.skip_ws();
            let Some(&c) = self.s.get(self.pos) else {
                break;
            };
            if c.is_ascii_alphabetic() {
                self.pos += 1;
                cmd = Some(c);
            } else if cmd.is_none() || matches!(cmd, Some(b'Z' | b'z')) {
                return None;
            }
            let c = cmd?;
            let rel = c.is_ascii_lowercase();
            let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
            match c.to_ascii_uppercase() {
                b'M' => {
                    let (x, y) = (self.number()? + ox, self.number()? + oy);
                    segs.push(PathSeg::MoveTo(x, y));
                    (cx, cy, sx, sy) = (x, y, x, y);
                    // Further coordinate pairs are implicit line-tos.
                    cmd = Some(if rel { b'l' } else { b'L' });
                }
                b'L' => {
                    let (x, y) = (self.number()? + ox, self.number()? + oy);
                    segs.push(PathSeg::LineTo(x, y));
                    (cx, cy) = (x, y);
                }
                b'H' => {
                    let x = self.number()? + ox;
                    segs.push(PathSeg::LineTo(x, cy));
                    cx = x;
                }
                b'V' => {
                    let y = self.number()? + oy;
                    segs.push(PathSeg::LineTo(cx, y));
                    cy = y;
                }
                b'Q' => {
                    let (x1, y1) = (self.number()? + ox, self.number()? + oy);
                    let (x, y) = (self.number()? + ox, self.number()? + oy);
                    segs.push(PathSeg::QuadTo(x1, y1, x, y));
                    (cx, cy) = (x, y);
                }
                b'C' => {
                    let (x1, y1) = (self.number()? + ox, self.number()? + oy);
                    let (x2, y2) = (self.number()? + ox, self.number()? + oy);
                    let (x, y) = (self.number()? + ox, self.number()? + oy);
                    segs.push(PathSeg::CubicTo(x1, y1, x2, y2, x, y));
                    (cx, cy) = (x, y);
                }
                b'A' => {
                    let rx = self.number()?;
                    let ry = self.number()?;
                    let x_rotation = self.number()?;
                    let large_arc = self.flag()?;
                    let sweep = self.flag()?;
                    let (x, y) = (self.number()? + ox, self.number()? + oy);
                    segs.push(PathSeg::ArcTo {
                        rx,
                        ry,
                        x_rotation,
                        large_arc,
                        sweep,
                        x,
                        y,
                    });
                    (cx, cy) = (x, y);
                }
                b'Z' => {
                    segs.push(PathSeg::Close);
                    (cx, cy) = (sx, sy);
                    continue;
                }
                _ => return None,
            }
            if !self.peek_number_start() && self.pos < self.s.len() {
                let next = self.s.get(self.pos).copied();
                if !matches!(next, Some(c) if c.is_ascii_alphabetic()) {
                    return None;
                }
            }
        }
        Some(PathData { segs })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_format_stably() {
        assert_eq!(format_number(1.0), "1");
        assert_eq!(format_number(1.23456), "1.235");
        assert_eq!(format_number(-0.0001), "0");
        assert_eq!(format_number(f64::NAN), "0");
        assert_eq!(format_number(-12.5), "-12.5");
        assert_eq!(format_number(1e21), "1000000000000000000000");
    }

    #[test]
    fn d_round_trips() {
        let mut b = PathBuilder::new();
        b.move_to(0.0, 0.0)
            .line_to(10.0, 0.5)
            .quad_to(1.0, 2.0, 3.0, 4.0)
            .cubic_to(1.0, 2.0, 3.0, 4.0, 5.0, 6.0)
            .arc_to(5.0, 5.0, 0.0, true, false, 10.0, 10.0)
            .close();
        let p = b.build();
        let d = p.to_d();
        assert_eq!(d, "M0 0 L10 0.5 Q1 2 3 4 C1 2 3 4 5 6 A5 5 0 1 0 10 10 Z");
        assert_eq!(PathData::parse(&d), Some(p));
        let rel = PathData::parse("m1 1 10 0 v5 h-10z").unwrap();
        assert_eq!(rel.to_d(), "M1 1 L11 1 L11 6 L1 6 Z");
        for bad in ["L", "M1", "1 2", "M1 2 X", "A1 1 0 2 0 1 1", "M1e999 0"] {
            assert_eq!(PathData::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn pie_slices_and_bounds() {
        let quarter = PathData::pie_slice(50.0, 50.0, 10.0, 0.0, 0.0, 90.0);
        assert_eq!(quarter.to_d(), "M50 50 L50 40 A10 10 0 0 1 60 50 Z");
        let b = quarter.bounds().unwrap();
        assert!((b.x - 50.0).abs() < 1e-6 && (b.right() - 60.0).abs() < 1e-6);
        assert!((b.y - 40.0).abs() < 1e-6 && (b.bottom() - 50.0).abs() < 1e-6);
        let ring = PathData::pie_slice(0.0, 0.0, 10.0, 5.0, 0.0, 360.0);
        let rb = ring.bounds().unwrap();
        assert!((rb.width - 20.0).abs() < 1e-6 && (rb.height - 20.0).abs() < 1e-6);
        assert!(PathData::pie_slice(0.0, 0.0, 10.0, 0.0, 10.0, 10.0).is_empty());
        let rr = PathData::rounded_rect(Rect::new(0.0, 0.0, 20.0, 10.0), 50.0);
        assert!(rr.to_d().contains("A5 5"));
    }
}
