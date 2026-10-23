//! Geometry primitives: points, rectangles, insets and affine transforms.

use serde::{Deserialize, Serialize};

/// An axis-aligned rectangle (`x`, `y` is the top-left corner).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// A rectangle from two corners (in any order).
    pub fn from_points(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Self::new(x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs())
    }

    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    /// `true` if the rectangle has no area (or non-finite coordinates).
    pub fn is_empty(&self) -> bool {
        let finite = self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite();
        !(finite && self.width > 0.0 && self.height > 0.0)
    }

    /// Shrinks the rectangle by `insets`; width/height never go negative.
    pub fn inset(&self, insets: Insets) -> Self {
        let width = (self.width - insets.left - insets.right).max(0.0);
        let height = (self.height - insets.top - insets.bottom).max(0.0);
        Self::new(self.x + insets.left, self.y + insets.top, width, height)
    }

    /// Shrinks the rectangle by `d` on every side.
    pub fn inset_uniform(&self, d: f64) -> Self {
        self.inset(Insets::uniform(d))
    }

    /// Moves the rectangle by `(dx, dy)`.
    pub fn translate(&self, dx: f64, dy: f64) -> Self {
        Self::new(self.x + dx, self.y + dy, self.width, self.height)
    }

    /// `true` if `(x, y)` lies inside (edges inclusive).
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.right() && y >= self.y && y <= self.bottom()
    }

    /// `true` if `other` lies entirely inside this rectangle, with `eps`
    /// tolerance.
    pub fn contains_rect(&self, other: &Rect, eps: f64) -> bool {
        other.x >= self.x - eps
            && other.y >= self.y - eps
            && other.right() <= self.right() + eps
            && other.bottom() <= self.bottom() + eps
    }

    /// Smallest rectangle containing both.
    pub fn union(&self, other: &Rect) -> Self {
        let x0 = self.x.min(other.x);
        let y0 = self.y.min(other.y);
        let x1 = self.right().max(other.right());
        let y1 = self.bottom().max(other.bottom());
        Self::new(x0, y0, x1 - x0, y1 - y0)
    }

    /// Overlap of both, or `None` if they do not overlap.
    pub fn intersect(&self, other: &Rect) -> Option<Self> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = self.right().min(other.right());
        let y1 = self.bottom().min(other.bottom());
        (x1 > x0 && y1 > y0).then(|| Self::new(x0, y0, x1 - x0, y1 - y0))
    }
}

/// Distances from each edge (padding, border widths).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Insets {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

impl Insets {
    pub const ZERO: Insets = Insets {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    };

    pub const fn uniform(d: f64) -> Self {
        Self {
            top: d,
            right: d,
            bottom: d,
            left: d,
        }
    }
}

/// A 2D affine transform `[a c e; b d f; 0 0 1]` (SVG `matrix(a b c d e f)`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Transform = Transform {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub const fn translate(tx: f64, ty: f64) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: tx,
            f: ty,
        }
    }

    pub const fn scale(sx: f64, sy: f64) -> Self {
        Self {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: sy,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Clockwise rotation by `degrees` (SVG `rotate`, y axis pointing down)
    /// about the origin.
    pub fn rotate(degrees: f64) -> Self {
        let (s, c) = degrees.to_radians().sin_cos();
        Self {
            a: c,
            b: s,
            c: -s,
            d: c,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Rotation by `degrees` about `(cx, cy)`.
    pub fn rotate_about(degrees: f64, cx: f64, cy: f64) -> Self {
        Self::translate(cx, cy)
            .then(&Self::rotate(degrees))
            .then(&Self::translate(-cx, -cy))
    }

    /// `self ∘ other`: applies `other` first, then `self` (SVG nesting order:
    /// `transform="A B"` is `A.then(B)`).
    pub fn then(&self, other: &Transform) -> Self {
        Self {
            a: self.a * other.a + self.c * other.b,
            b: self.b * other.a + self.d * other.b,
            c: self.a * other.c + self.c * other.d,
            d: self.b * other.c + self.d * other.d,
            e: self.a * other.e + self.c * other.f + self.e,
            f: self.b * other.e + self.d * other.f + self.f,
        }
    }

    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }

    /// `true` for a translation only.
    pub fn is_translate(&self) -> bool {
        self.a == 1.0 && self.b == 0.0 && self.c == 0.0 && self.d == 1.0
    }

    /// Maps a point.
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    /// Bounding box of a transformed rectangle.
    pub fn apply_rect(&self, r: &Rect) -> Rect {
        let corners = [
            self.apply(r.x, r.y),
            self.apply(r.right(), r.y),
            self.apply(r.x, r.bottom()),
            self.apply(r.right(), r.bottom()),
        ];
        let (mut x0, mut y0) = corners[0];
        let (mut x1, mut y1) = corners[0];
        for (x, y) in corners {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        Rect::from_points(x0, y0, x1, y1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_helpers() {
        let r = Rect::new(10.0, 20.0, 100.0, 50.0);
        assert_eq!(r.right(), 110.0);
        assert_eq!(r.inset_uniform(5.0), Rect::new(15.0, 25.0, 90.0, 40.0));
        assert_eq!(r.inset_uniform(80.0).width, 0.0);
        assert!(Rect::new(0.0, 0.0, 0.0, 5.0).is_empty());
        assert!(Rect::new(f64::NAN, 0.0, 5.0, 5.0).is_empty());
        assert_eq!(
            r.intersect(&Rect::new(100.0, 60.0, 50.0, 50.0)),
            Some(Rect::new(100.0, 60.0, 10.0, 10.0))
        );
        assert_eq!(r.intersect(&Rect::new(200.0, 0.0, 1.0, 1.0)), None);
    }

    #[test]
    fn transforms_compose_like_svg() {
        let t = Transform::translate(10.0, 5.0).then(&Transform::scale(2.0, 2.0));
        assert_eq!(t.apply(1.0, 1.0), (12.0, 7.0));
        let r = Transform::rotate_about(90.0, 10.0, 10.0);
        let (x, y) = r.apply(20.0, 10.0);
        assert!((x - 10.0).abs() < 1e-9 && (y - 20.0).abs() < 1e-9);
    }
}
