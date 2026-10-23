//! Luminance planes and the structural similarity index (SSIM).
//!
//! SSIM follows Wang, Bovik, Sheikh & Simoncelli, "Image quality
//! assessment: from error visibility to structural similarity" (IEEE TIP,
//! 2004): local means, variances and covariance under a normalized window
//! `w` (11×11 Gaussian, σ = 1.5 by default), then
//!
//! ```text
//! SSIM(x, y) = (2·μx·μy + C1)(2·σxy + C2) / ((μx² + μy² + C1)(σx² + σy² + C2))
//! C1 = (0.01·255)², C2 = (0.03·255)²
//! ```
//!
//! averaged over every window position that lies entirely inside the
//! image ("valid" convolution, as in the reference implementation). Inputs
//! smaller than the window get one global window with uniform weights.
//! Implemented from the paper; no third-party code.

use image::RgbaImage;

/// `K1` of the SSIM stabilizing constant `C1 = (K1·L)²`.
pub const K1: f64 = 0.01;
/// `K2` of `C2 = (K2·L)²`.
pub const K2: f64 = 0.03;
/// Dynamic range of the luminance values.
pub const DYNAMIC_RANGE: f64 = 255.0;

/// A single-channel image of luminance values (0–255), row-major.
#[derive(Debug, Clone, PartialEq)]
pub struct Plane {
    width: usize,
    height: usize,
    data: Vec<f32>,
}

impl Plane {
    /// A plane from row-major values; `None` if `data` has the wrong length.
    pub fn new(width: usize, height: usize, data: Vec<f32>) -> Option<Self> {
        (width.checked_mul(height) == Some(data.len())).then_some(Self {
            width,
            height,
            data,
        })
    }

    /// A plane of one value.
    pub fn filled(width: usize, height: usize, value: f32) -> Self {
        Self {
            width,
            height,
            data: vec![value; width.saturating_mul(height)],
        }
    }

    /// BT.601 luma (`0.299 R + 0.587 G + 0.114 B`) of an RGBA image
    /// composited over white (transparent pixels count as page
    /// background).
    pub fn luma(img: &RgbaImage) -> Self {
        let data = img
            .pixels()
            .map(|p| {
                let a = f32::from(p[3]) / 255.0;
                let c = |v: u8| f32::from(v) * a + 255.0 * (1.0 - a);
                0.299 * c(p[0]) + 0.587 * c(p[1]) + 0.114 * c(p[2])
            })
            .collect();
        Self {
            width: img.width() as usize,
            height: img.height() as usize,
            data,
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn data(&self) -> &[f32] {
        &self.data
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// The value at `(x, y)`; `None` outside the plane.
    pub fn get(&self, x: usize, y: usize) -> Option<f32> {
        (x < self.width && y < self.height).then(|| self.data[y * self.width + x])
    }

    /// Bilinear sample at `(x, y)` in pixel-centre coordinates (`(0, 0)` is
    /// the centre of the top-left pixel); `outside` beyond the edges.
    pub fn sample(&self, x: f64, y: f64, outside: f32) -> f32 {
        if self.is_empty() || !x.is_finite() || !y.is_finite() {
            return outside;
        }
        let (w, h) = (self.width as f64, self.height as f64);
        if x < -0.5 || y < -0.5 || x > w - 0.5 || y > h - 0.5 {
            return outside;
        }
        let x = x.clamp(0.0, w - 1.0);
        let y = y.clamp(0.0, h - 1.0);
        // In range (clamped above), so the casts are exact.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        #[allow(clippy::cast_possible_truncation)]
        let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
        let at = |x: usize, y: usize| self.data[y * self.width + x];
        let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
        let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
        top * (1.0 - fy) + bottom * fy
    }

    /// Box-filtered `1/factor` copy (partial blocks at the right and bottom
    /// edges average the pixels they have).
    pub fn downsample(&self, factor: usize) -> Self {
        if factor <= 1 || self.is_empty() {
            return self.clone();
        }
        let ow = self.width.div_ceil(factor);
        let oh = self.height.div_ceil(factor);
        let mut sums = vec![0.0_f64; ow * oh];
        let mut counts = vec![0_u32; ow * oh];
        for y in 0..self.height {
            let row = &self.data[y * self.width..(y + 1) * self.width];
            let oy = y / factor;
            for (x, v) in row.iter().enumerate() {
                let i = oy * ow + x / factor;
                sums[i] += f64::from(*v);
                counts[i] += 1;
            }
        }
        let data = sums
            .iter()
            .zip(&counts)
            .map(|(s, c)| {
                #[allow(clippy::cast_possible_truncation)]
                let v = (s / f64::from((*c).max(1))) as f32;
                v
            })
            .collect();
        Self {
            width: ow,
            height: oh,
            data,
        }
    }

    /// The `w`×`h` sub-plane at `(x, y)`, clamped to the plane.
    pub fn crop(&self, x: usize, y: usize, w: usize, h: usize) -> Self {
        let x0 = x.min(self.width);
        let y0 = y.min(self.height);
        let x1 = x0.saturating_add(w).min(self.width);
        let y1 = y0.saturating_add(h).min(self.height);
        let mut data = Vec::with_capacity((x1 - x0) * (y1 - y0));
        for row in y0..y1 {
            data.extend_from_slice(&self.data[row * self.width + x0..row * self.width + x1]);
        }
        Self {
            width: x1 - x0,
            height: y1 - y0,
            data,
        }
    }
}

/// The SSIM weighting window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SsimWindow {
    /// Circular-symmetric Gaussian (the paper uses 11×11, σ = 1.5).
    Gaussian { size: usize, sigma: f64 },
    /// Uniform `size`×`size` box (e.g. the 8×8 block variant).
    Uniform { size: usize },
}

impl Default for SsimWindow {
    fn default() -> Self {
        SsimWindow::Gaussian {
            size: 11,
            sigma: 1.5,
        }
    }
}

impl SsimWindow {
    /// Window side in pixels (at least 1).
    pub fn size(&self) -> usize {
        match *self {
            SsimWindow::Gaussian { size, .. } | SsimWindow::Uniform { size } => size.max(1),
        }
    }

    /// The normalized 1-D kernel (the 2-D window is its outer product).
    pub fn kernel(&self) -> Vec<f64> {
        let n = self.size();
        let raw: Vec<f64> = match *self {
            SsimWindow::Gaussian { sigma, .. } if sigma.is_finite() && sigma > 0.0 => {
                let c = (n as f64 - 1.0) / 2.0;
                (0..n)
                    .map(|i| {
                        let d = i as f64 - c;
                        (-(d * d) / (2.0 * sigma * sigma)).exp()
                    })
                    .collect()
            }
            _ => vec![1.0; n],
        };
        let sum: f64 = raw.iter().sum();
        raw.into_iter().map(|v| v / sum).collect()
    }
}

/// SSIM of one window from its statistics.
fn ssim_value(mu_a: f64, mu_b: f64, var_a: f64, var_b: f64, cov: f64) -> f64 {
    let c1 = (K1 * DYNAMIC_RANGE).powi(2);
    let c2 = (K2 * DYNAMIC_RANGE).powi(2);
    ((2.0 * mu_a * mu_b + c1) * (2.0 * cov + c2))
        / ((mu_a * mu_a + mu_b * mu_b + c1) * (var_a + var_b + c2))
}

/// Mean SSIM of two planes of the same size; `None` if the sizes differ or
/// the planes are empty.
pub fn ssim(a: &Plane, b: &Plane, window: SsimWindow) -> Option<f64> {
    let mut sum = 0.0;
    let mut count = 0_u64;
    ssim_windows(a, b, window, |_, _, v| {
        sum += v;
        count += 1;
    })?;
    #[allow(clippy::cast_precision_loss)]
    let mean = sum / count.max(1) as f64;
    Some(mean)
}

/// The SSIM map: one value per valid window position (the plane is
/// `window - 1` pixels smaller than the inputs on each axis; 1×1 for
/// inputs smaller than the window).
pub fn ssim_map(a: &Plane, b: &Plane, window: SsimWindow) -> Option<Plane> {
    let n = window.size();
    let (w, h) = if a.width >= n && a.height >= n {
        (a.width - n + 1, a.height - n + 1)
    } else {
        (1, 1)
    };
    let mut map = Plane::filled(w, h, 0.0);
    ssim_windows(a, b, window, |x, y, v| {
        #[allow(clippy::cast_possible_truncation)]
        let v = v as f32;
        map.data[y * w + x] = v;
    })?;
    Some(map)
}

/// Calls `f(x, y, ssim)` for every valid window position.
fn ssim_windows(
    a: &Plane,
    b: &Plane,
    window: SsimWindow,
    mut f: impl FnMut(usize, usize, f64),
) -> Option<()> {
    if a.width != b.width || a.height != b.height || a.is_empty() {
        return None;
    }
    let (w, h) = (a.width, a.height);
    let kernel = window.kernel();
    let n = kernel.len();
    if w < n || h < n {
        f(0, 0, global_ssim(a, b));
        return Some(());
    }
    let ow = w - n + 1;
    // Horizontal pass into a ring of `n` rows; each output row is the
    // vertical pass over the ring. Sums: a, b, a², b², ab.
    let mut ring = vec![[0.0_f64; 5]; n * ow];
    for y in 0..h {
        let slot = (y % n) * ow;
        let ra = &a.data[y * w..(y + 1) * w];
        let rb = &b.data[y * w..(y + 1) * w];
        for x in 0..ow {
            let mut s = [0.0_f64; 5];
            for (i, k) in kernel.iter().enumerate() {
                let va = f64::from(ra[x + i]);
                let vb = f64::from(rb[x + i]);
                s[0] += k * va;
                s[1] += k * vb;
                s[2] += k * va * va;
                s[3] += k * vb * vb;
                s[4] += k * va * vb;
            }
            ring[slot + x] = s;
        }
        if y + 1 < n {
            continue;
        }
        let top = y + 1 - n;
        for x in 0..ow {
            let mut s = [0.0_f64; 5];
            for (j, k) in kernel.iter().enumerate() {
                let r = &ring[((top + j) % n) * ow + x];
                for (acc, v) in s.iter_mut().zip(r) {
                    *acc += k * v;
                }
            }
            let (mu_a, mu_b) = (s[0], s[1]);
            f(
                x,
                top,
                ssim_value(
                    mu_a,
                    mu_b,
                    s[2] - mu_a * mu_a,
                    s[3] - mu_b * mu_b,
                    s[4] - mu_a * mu_b,
                ),
            );
        }
    }
    Some(())
}

/// SSIM with one window covering the whole planes (uniform weights).
fn global_ssim(a: &Plane, b: &Plane) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let n = a.data.len().max(1) as f64;
    let mean = |p: &Plane| p.data.iter().map(|v| f64::from(*v)).sum::<f64>() / n;
    let (mu_a, mu_b) = (mean(a), mean(b));
    let (mut var_a, mut var_b, mut cov) = (0.0, 0.0, 0.0);
    for (va, vb) in a.data.iter().zip(&b.data) {
        let (da, db) = (f64::from(*va) - mu_a, f64::from(*vb) - mu_b);
        var_a += da * da;
        var_b += db * db;
        cov += da * db;
    }
    ssim_value(mu_a, mu_b, var_a / n, var_b / n, cov / n)
}

/// Mean absolute difference of two planes of the same size, as a fraction
/// of the dynamic range (0 = identical, 1 = black vs white everywhere).
pub fn mean_abs_diff(a: &Plane, b: &Plane) -> Option<f64> {
    if a.width != b.width || a.height != b.height || a.is_empty() {
        return None;
    }
    let sum: f64 = a
        .data
        .iter()
        .zip(&b.data)
        .map(|(x, y)| f64::from((x - y).abs()))
        .sum();
    #[allow(clippy::cast_precision_loss)]
    let n = a.data.len() as f64;
    Some(sum / n / DYNAMIC_RANGE)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn pattern(w: usize, h: usize, shift: usize) -> Plane {
        let data = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w + shift, i / w);
                if (x / 4 + y / 4) % 2 == 0 {
                    30.0
                } else {
                    220.0
                }
            })
            .collect();
        Plane::new(w, h, data).unwrap()
    }

    #[test]
    fn identical_planes_score_one() {
        let p = pattern(40, 30, 0);
        assert!((ssim(&p, &p, SsimWindow::default()).unwrap() - 1.0).abs() < 1e-9);
        assert_eq!(mean_abs_diff(&p, &p), Some(0.0));
        let flat = Plane::filled(20, 20, 255.0);
        assert!((ssim(&flat, &flat, SsimWindow::Uniform { size: 8 }).unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn structure_changes_lower_the_score() {
        let a = pattern(64, 48, 0);
        let shifted = pattern(64, 48, 2);
        let inverted = Plane::new(64, 48, a.data().iter().map(|v| 250.0 - v).collect()).unwrap();
        let s_shift = ssim(&a, &shifted, SsimWindow::default()).unwrap();
        let s_inv = ssim(&a, &inverted, SsimWindow::default()).unwrap();
        assert!(s_shift < 0.9, "{s_shift}");
        assert!(s_inv < 0.0, "{s_inv}");
        // A small uniform brightness change keeps the structure.
        let brighter = Plane::new(64, 48, a.data().iter().map(|v| v + 5.0).collect()).unwrap();
        assert!(ssim(&a, &brighter, SsimWindow::default()).unwrap() > 0.99);
        assert!(ssim(&a, &Plane::filled(10, 10, 0.0), SsimWindow::default()).is_none());
    }

    #[test]
    fn small_planes_use_one_global_window() {
        let a = pattern(6, 5, 0);
        let map = ssim_map(&a, &a, SsimWindow::default()).unwrap();
        assert_eq!((map.width(), map.height()), (1, 1));
        assert!((map.data()[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn downsample_crop_and_sample() {
        let p = Plane::new(3, 2, vec![0.0, 10.0, 20.0, 30.0, 40.0, 50.0]).unwrap();
        let d = p.downsample(2);
        assert_eq!((d.width(), d.height()), (2, 1));
        assert_eq!(d.data(), &[20.0, 35.0]);
        let c = p.crop(1, 1, 5, 5);
        assert_eq!(c.data(), &[40.0, 50.0]);
        assert_eq!(p.sample(0.5, 0.0, 255.0), 5.0);
        assert_eq!(p.sample(-2.0, 0.0, 255.0), 255.0);
        assert_eq!(p.get(2, 1), Some(50.0));
    }
}
