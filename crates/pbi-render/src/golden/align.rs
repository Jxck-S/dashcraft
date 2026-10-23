//! Page alignment: where the rendered page sits in a golden export.
//!
//! The service's `ExportTo` PNGs are not always the page alone: a 1280×720
//! page exported at 2560 px wide comes back as 2560×1460, the page scaled
//! by 2 at the top and 20 px of white padding below (`FitToWidth` and
//! `FitToPage` pages alike). The method:
//!
//! 1. **Scale from the page bounds.** The page fills the export along its
//!    limiting axis and keeps its aspect ratio, so the scale hypothesis is
//!    `min(golden.w / render.w, golden.h / render.h)` (plus `1.0` when the
//!    render fits unscaled, for same-resolution exports with margins).
//! 2. **Offset search in the padding.** For each scale, the page can only
//!    be shifted within the slack `golden - render·scale` on each axis.
//!    Offsets are scored by the mean absolute luminance difference between
//!    the render and the bilinearly sampled golden (every
//!    [`AlignOptions::sample_step`]-th render pixel), coarse to fine: a
//!    grid of at most [`AlignOptions::coarse_steps`] offsets per axis, then
//!    3×3 neighbourhoods at halving steps down to 1 px.
//! 3. The lowest residual wins (ties: the smaller offset). The golden is
//!    then resampled into the render's pixel grid ([`resample`]); padding
//!    outside the page is never compared.
//!
//! For a Power BI PNG export of a page at 2× (the page at the top-left of
//! the image) this finds scale 2 (render at scale 1) or 1 (render at scale
//! 2) and offset (0, 0).

use std::collections::HashMap;

use image::{Rgba, RgbaImage};
use serde::Serialize;

use super::ssim::{DYNAMIC_RANGE, Plane};

/// Maps the render onto the golden: a render point `(x, y)` (pixel-edge
/// coordinates) is at `(x·scale + offset_x, y·scale + offset_y)` in the
/// golden.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Alignment {
    pub scale: f64,
    pub offset_x: f64,
    pub offset_y: f64,
    /// Mean absolute luminance difference (0–1) of the sampled page area
    /// at this alignment (`NaN` when it was given, not searched).
    pub residual: f64,
}

impl Alignment {
    /// Same size, same origin.
    pub const IDENTITY: Alignment = Alignment {
        scale: 1.0,
        offset_x: 0.0,
        offset_y: 0.0,
        residual: f64::NAN,
    };

    /// A fixed alignment (not searched).
    pub fn new(scale: f64, offset_x: f64, offset_y: f64) -> Self {
        Self {
            scale,
            offset_x,
            offset_y,
            residual: f64::NAN,
        }
    }

    /// The golden position of a render point (pixel-edge coordinates).
    pub fn map(&self, x: f64, y: f64) -> (f64, f64) {
        (
            x * self.scale + self.offset_x,
            y * self.scale + self.offset_y,
        )
    }

    /// Golden pixel-centre coordinates of the centre of render pixel
    /// `(x, y)`.
    fn map_center(&self, x: usize, y: usize) -> (f64, f64) {
        let (gx, gy) = self.map(x as f64 + 0.5, y as f64 + 0.5);
        (gx - 0.5, gy - 0.5)
    }
}

/// Search parameters of [`align_with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlignOptions {
    /// Score candidates on every n-th render pixel (both axes).
    pub sample_step: usize,
    /// Most offsets per axis on the coarse grid.
    pub coarse_steps: usize,
}

impl Default for AlignOptions {
    fn default() -> Self {
        Self {
            sample_step: 4,
            coarse_steps: 16,
        }
    }
}

/// Finds the alignment of `render` in `golden` (see the module docs).
pub fn align(render: &Plane, golden: &Plane) -> Alignment {
    align_with(render, golden, &AlignOptions::default())
}

/// [`align`] with explicit search parameters.
pub fn align_with(render: &Plane, golden: &Plane, opts: &AlignOptions) -> Alignment {
    if render.is_empty() || golden.is_empty() {
        return Alignment {
            residual: 1.0,
            ..Alignment::IDENTITY
        };
    }
    let (rw, rh) = (render.width() as f64, render.height() as f64);
    let (gw, gh) = (golden.width() as f64, golden.height() as f64);
    let fit = (gw / rw).min(gh / rh);
    let mut scales = vec![fit];
    if (fit - 1.0).abs() > 1e-9 && rw <= gw && rh <= gh {
        scales.push(1.0);
    }
    let step = opts.sample_step.max(1);
    let coarse = opts.coarse_steps.max(1);
    let mut best: Option<Alignment> = None;
    for scale in scales {
        let slack = |g: f64, r: f64| -> i64 {
            // Bounded by the image size, so the cast is exact.
            #[allow(clippy::cast_possible_truncation)]
            let s = (g - r * scale).max(0.0).floor() as i64;
            s
        };
        let (slack_x, slack_y) = (slack(gw, rw), slack(gh, rh));
        let mut cache: HashMap<(i64, i64), f64> = HashMap::new();
        let mut score = |ox: i64, oy: i64| -> f64 {
            *cache.entry((ox, oy)).or_insert_with(|| {
                #[allow(clippy::cast_precision_loss)]
                let a = Alignment::new(scale, ox as f64, oy as f64);
                residual(render, golden, &a, step)
            })
        };
        let coarse_step = |slack: i64| -> i64 {
            #[allow(clippy::cast_possible_wrap)]
            let n = coarse as i64;
            ((slack + n - 1) / n).max(1)
        };
        let (mut sx, mut sy) = (coarse_step(slack_x), coarse_step(slack_y));
        let mut current = (0_i64, 0_i64, f64::INFINITY);
        let mut consider = |ox: i64, oy: i64, current: &mut (i64, i64, f64)| {
            let r = score(ox, oy);
            let better = r < current.2 - 1e-12
                || ((r - current.2).abs() <= 1e-12 && ox + oy < current.0 + current.1);
            if better {
                *current = (ox, oy, r);
            }
        };
        let mut oy = 0;
        while oy <= slack_y {
            let mut ox = 0;
            while ox <= slack_x {
                consider(ox, oy, &mut current);
                ox += sx;
            }
            oy += sy;
        }
        loop {
            sx = (sx + 1) / 2;
            sy = (sy + 1) / 2;
            let (cx, cy, _) = current;
            for dy in [-sy, 0, sy] {
                for dx in [-sx, 0, sx] {
                    let (ox, oy) = (cx + dx, cy + dy);
                    if (0..=slack_x).contains(&ox) && (0..=slack_y).contains(&oy) {
                        consider(ox, oy, &mut current);
                    }
                }
            }
            if sx <= 1 && sy <= 1 {
                break;
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let candidate = Alignment {
            scale,
            offset_x: current.0 as f64,
            offset_y: current.1 as f64,
            residual: current.2,
        };
        if best.is_none_or(|b| candidate.residual < b.residual - 1e-12) {
            best = Some(candidate);
        }
    }
    best.unwrap_or(Alignment::IDENTITY)
}

/// Mean absolute luminance difference (0–1) of the sampled render pixels
/// and the golden under `a` (white outside the golden).
fn residual(render: &Plane, golden: &Plane, a: &Alignment, step: usize) -> f64 {
    let mut sum = 0.0_f64;
    let mut n = 0_u64;
    for y in (0..render.height()).step_by(step) {
        for x in (0..render.width()).step_by(step) {
            let (gx, gy) = a.map_center(x, y);
            let g = golden.sample(gx, gy, 255.0);
            let r = render.get(x, y).unwrap_or(255.0);
            sum += f64::from((r - g).abs());
            n += 1;
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let n = n.max(1) as f64;
    sum / n / DYNAMIC_RANGE
}

/// The golden resampled (bilinear) into a `width`×`height` render grid
/// under `a`; white outside the golden.
pub fn resample(golden: &RgbaImage, a: &Alignment, width: u32, height: u32) -> RgbaImage {
    const WHITE: Rgba<u8> = Rgba([255, 255, 255, 255]);
    let integral = |v: f64| v.fract() == 0.0 && v.is_finite();
    if a.scale == 1.0 && integral(a.offset_x) && integral(a.offset_y) {
        // Bounded offsets (checked by the get_pixel_checked lookups).
        #[allow(clippy::cast_possible_truncation)]
        let (ox, oy) = (a.offset_x as i64, a.offset_y as i64);
        return RgbaImage::from_fn(width, height, |x, y| {
            let (gx, gy) = (i64::from(x) + ox, i64::from(y) + oy);
            match (u32::try_from(gx), u32::try_from(gy)) {
                (Ok(gx), Ok(gy)) => golden.get_pixel_checked(gx, gy).copied().unwrap_or(WHITE),
                _ => WHITE,
            }
        });
    }
    let (gw, gh) = (f64::from(golden.width()), f64::from(golden.height()));
    RgbaImage::from_fn(width, height, |x, y| {
        let (gx, gy) = a.map_center(x as usize, y as usize);
        if golden.width() == 0
            || golden.height() == 0
            || !(gx >= -0.5 && gy >= -0.5 && gx <= gw - 0.5 && gy <= gh - 0.5)
        {
            return WHITE;
        }
        let gx = gx.clamp(0.0, gw - 1.0);
        let gy = gy.clamp(0.0, gh - 1.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (x0, y0) = (gx.floor() as u32, gy.floor() as u32);
        let x1 = (x0 + 1).min(golden.width() - 1);
        let y1 = (y0 + 1).min(golden.height() - 1);
        let (fx, fy) = (gx - f64::from(x0), gy - f64::from(y0));
        let (p00, p10) = (golden.get_pixel(x0, y0), golden.get_pixel(x1, y0));
        let (p01, p11) = (golden.get_pixel(x0, y1), golden.get_pixel(x1, y1));
        let mut out = [0_u8; 4];
        for (c, o) in out.iter_mut().enumerate() {
            let top = f64::from(p00[c]) * (1.0 - fx) + f64::from(p10[c]) * fx;
            let bottom = f64::from(p01[c]) * (1.0 - fx) + f64::from(p11[c]) * fx;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let v = (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8;
            *o = v;
        }
        Rgba(out)
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// A page with a frame and a few blocks, `w`×`h`.
    fn page(w: u32, h: u32, unit: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            let frame = x < 3 * unit || y < 3 * unit || x >= w - 3 * unit || y >= h - 3 * unit;
            let block = (x / (10 * unit) + y / (7 * unit)).is_multiple_of(3)
                && !(y / unit).is_multiple_of(5);
            if frame {
                Rgba([192, 0, 0, 255])
            } else if block {
                Rgba([40, 90, 200, 255])
            } else {
                Rgba([255, 255, 255, 255])
            }
        })
    }

    /// `img` pasted at `(dx, dy)` on a white `w`×`h` canvas.
    fn pad(img: &RgbaImage, w: u32, h: u32, dx: u32, dy: u32) -> RgbaImage {
        let mut out = RgbaImage::from_pixel(w, h, Rgba([255, 255, 255, 255]));
        image::imageops::overlay(&mut out, img, i64::from(dx), i64::from(dy));
        out
    }

    #[test]
    fn export_padding_is_found() {
        let render = page(320, 180, 1);
        for (dx, dy) in [(0, 0), (0, 7), (0, 20)] {
            let golden = pad(&render, 320, 200, dx, dy);
            let a = align(&Plane::luma(&render), &Plane::luma(&golden));
            assert_eq!(
                (a.scale, a.offset_x, a.offset_y),
                (1.0, f64::from(dx), f64::from(dy))
            );
            assert!(a.residual < 1e-9);
            assert_eq!(resample(&golden, &a, 320, 180), render);
        }
    }

    #[test]
    fn scaled_exports_are_found() {
        let render = page(160, 90, 1);
        let golden = pad(&page(320, 180, 2), 320, 196, 0, 9);
        let a = align(&Plane::luma(&render), &Plane::luma(&golden));
        assert_eq!((a.scale, a.offset_x, a.offset_y), (2.0, 0.0, 9.0));
        let back = resample(&golden, &a, 160, 90);
        assert_eq!((back.width(), back.height()), (160, 90));
        assert_eq!(back.get_pixel(0, 0), &Rgba([192, 0, 0, 255]));
    }
}
