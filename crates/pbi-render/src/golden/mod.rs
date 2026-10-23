//! Golden-image comparison: renders vs Power BI PNG exports.
//!
//! Used by the golden regression tests (`tests/golden.rs`) and available to
//! tools. The pipeline:
//!
//! 1. **Load** both images as RGBA ([`load_png`], [`from_render`] /
//!    [`rasterize`] for a scene).
//! 2. **Align** the render with the golden ([`align()`], see the
//!    [`align` module](mod@align) for the method): the page scale comes from the
//!    page bounds (the page fills the export along its limiting axis), the
//!    offset from a coarse-to-fine search inside the export padding (a
//!    2560×1460 export of a 1280×720 page puts the page at the top). The
//!    golden is resampled into the render's pixel grid ([`resample`]).
//! 3. **Score** per region ([`compare_regions`]): each visual's layout box
//!    ([`page_regions`] / [`regions_from_layout`], render pixels) is
//!    cropped from both images' luminance and compared with **SSIM**
//!    ([`ssim()`]: 11×11 Gaussian window, σ = 1.5, `K1 = 0.01`,
//!    `K2 = 0.03`, implemented from Wang et al. 2004) and the mean absolute
//!    luminance difference. By default the comparison runs at half the
//!    render resolution ([`CompareOptions::downsample`] = 2, box filter),
//!    i.e. one sample per page unit (CSS pixel) for a scale-2 render, the
//!    unit layouts are authored in; this separates a matching layout from
//!    one shifted by a few page units better than full resolution does.
//!    The whole page is scored the same way ([`GoldenReport::overall`]).
//! 4. **Inspect** ([`diff_heatmap`] + [`save_png`]): a heatmap of the
//!    luminance difference with each region outlined green (passes) or
//!    red (below the threshold).
//!
//! Golden PNGs are optional local files (`reference/golden/`, gitignored):
//! write renders and heatmaps made from them under `target/` (the tests use
//! `target/tmp/golden/`) and never commit them.

pub mod align;
pub mod ssim;

use std::path::Path;

use image::{ImageFormat, Rgba, RgbaImage};
use pbi_model::{Page, Report};
use serde::Serialize;

use crate::layout::{LayoutOptions, PageLayout};
use crate::png::{PngError, PngOptions, render_rgba};
use crate::scene::{Color, Rect, Scene};
use crate::theme::ThemeResolver;

pub use align::{AlignOptions, Alignment, align, align_with, resample};
pub use ssim::{Plane, SsimWindow, mean_abs_diff, ssim, ssim_map};

/// Largest golden side accepted by [`load_png`].
pub const MAX_SIDE: u32 = 16_384;
/// Largest decoded golden accepted by [`load_png`], in bytes.
pub const MAX_BYTES: u64 = 512 * 1024 * 1024;
/// The per-region acceptance threshold of the golden tests (`AGENTS.md` §7).
pub const DEFAULT_THRESHOLD: f64 = 0.85;

/// Golden comparison errors.
#[derive(Debug, thiserror::Error)]
pub enum GoldenError {
    #[error("cannot read {path}: {message}")]
    Read { path: String, message: String },
    #[error("cannot write {path}: {message}")]
    Write { path: String, message: String },
    #[error("empty image")]
    Empty,
    #[error("invalid image buffer ({width}x{height}, {len} bytes)")]
    Buffer { width: u32, height: u32, len: usize },
    #[error(transparent)]
    Render(#[from] PngError),
}

/// A region to score: a visual's layout box in render pixels.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Region {
    /// Visual name.
    pub name: String,
    pub visual_type: String,
    /// The visual's title, if it shows one (for reports).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Box in render pixels.
    pub rect: Rect,
}

impl Region {
    pub fn new(name: impl Into<String>, visual_type: impl Into<String>, rect: Rect) -> Self {
        Self {
            name: name.into(),
            visual_type: visual_type.into(),
            title: None,
            rect,
        }
    }
}

/// The score of one region.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionScore {
    pub name: String,
    pub visual_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Box in render pixels (clipped to the render).
    pub rect: Rect,
    /// Mean SSIM (−1–1, 1 = identical).
    pub ssim: f64,
    /// Mean absolute luminance difference (0–1).
    pub mean_abs_diff: f64,
}

impl RegionScore {
    /// `name` plus the title or type, for messages.
    pub fn label(&self) -> String {
        match &self.title {
            Some(t) => format!("{} ({}, \"{t}\")", self.name, self.visual_type),
            None => format!("{} ({})", self.name, self.visual_type),
        }
    }
}

/// Result of [`compare_regions`].
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoldenReport {
    /// Where the render sits in the golden.
    pub alignment: Alignment,
    /// Resolution divisor the scores were computed at.
    pub downsample: u32,
    /// SSIM of the whole render area.
    pub overall: f64,
    /// Mean absolute luminance difference of the whole render area.
    pub overall_mean_abs_diff: f64,
    /// Regions in input order (empty regions are left out).
    pub regions: Vec<RegionScore>,
}

impl GoldenReport {
    /// The region with the lowest SSIM.
    pub fn worst(&self) -> Option<&RegionScore> {
        self.regions.iter().min_by(|a, b| a.ssim.total_cmp(&b.ssim))
    }

    /// Regions whose SSIM is below `threshold`.
    pub fn below(&self, threshold: f64) -> Vec<&RegionScore> {
        self.regions.iter().filter(|r| r.ssim < threshold).collect()
    }

    /// A plain-text table of the scores (for test output).
    pub fn table(&self) -> String {
        let mut out = format!(
            "alignment: scale {:.3}, offset ({}, {}), residual {:.4}; downsample {}\n",
            self.alignment.scale,
            self.alignment.offset_x,
            self.alignment.offset_y,
            self.alignment.residual,
            self.downsample
        );
        out.push_str(&format!(
            "{:<22} {:<20} {:>7} {:>7}  {:<24} title\n",
            "visual", "type", "ssim", "mad", "rect (px)"
        ));
        for r in &self.regions {
            let rect = format!(
                "{:.0},{:.0} {:.0}x{:.0}",
                r.rect.x, r.rect.y, r.rect.width, r.rect.height
            );
            out.push_str(&format!(
                "{:<22} {:<20} {:>7.4} {:>7.4}  {:<24} {}\n",
                r.name,
                r.visual_type,
                r.ssim,
                r.mean_abs_diff,
                rect,
                r.title.as_deref().unwrap_or("")
            ));
        }
        out.push_str(&format!(
            "{:<22} {:<20} {:>7.4} {:>7.4}\n",
            "(page)", "", self.overall, self.overall_mean_abs_diff
        ));
        out
    }
}

/// Options of [`compare_regions`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompareOptions {
    pub window: SsimWindow,
    /// Compare at `1/downsample` of the render resolution (box filter; 1 =
    /// full resolution).
    pub downsample: u32,
    /// Use this alignment instead of searching for one.
    pub alignment: Option<Alignment>,
    /// Search parameters when `alignment` is `None`.
    pub align: AlignOptions,
}

impl Default for CompareOptions {
    fn default() -> Self {
        Self {
            window: SsimWindow::default(),
            downsample: 2,
            alignment: None,
            align: AlignOptions::default(),
        }
    }
}

/// Aligns `golden` with `render` and scores every region (render pixels).
///
/// # Errors
/// [`GoldenError::Empty`] if either image is empty.
pub fn compare_regions(
    render: &RgbaImage,
    golden: &RgbaImage,
    regions: &[Region],
    opts: &CompareOptions,
) -> Result<GoldenReport, GoldenError> {
    if render.width() == 0 || render.height() == 0 || golden.width() == 0 || golden.height() == 0 {
        return Err(GoldenError::Empty);
    }
    let render_luma = Plane::luma(render);
    let alignment = match opts.alignment {
        Some(a) => a,
        None => align_with(&render_luma, &Plane::luma(golden), &opts.align),
    };
    let aligned = resample(golden, &alignment, render.width(), render.height());
    let f = opts.downsample.max(1);
    let fs = f as usize;
    let a = render_luma.downsample(fs);
    let b = Plane::luma(&aligned).downsample(fs);
    let overall = ssim(&a, &b, opts.window).unwrap_or(0.0);
    let overall_mean_abs_diff = mean_abs_diff(&a, &b).unwrap_or(1.0);

    let bounds = Rect::new(
        0.0,
        0.0,
        f64::from(render.width()),
        f64::from(render.height()),
    );
    let scores = regions
        .iter()
        .filter_map(|region| {
            let rect = region.rect.intersect(&bounds)?;
            let (x0, y0, x1, y1) = grid_box(&rect, f64::from(f), a.width(), a.height())?;
            let (ca, cb) = (
                a.crop(x0, y0, x1 - x0, y1 - y0),
                b.crop(x0, y0, x1 - x0, y1 - y0),
            );
            Some(RegionScore {
                name: region.name.clone(),
                visual_type: region.visual_type.clone(),
                title: region.title.clone(),
                rect,
                ssim: ssim(&ca, &cb, opts.window)?,
                mean_abs_diff: mean_abs_diff(&ca, &cb)?,
            })
        })
        .collect();
    Ok(GoldenReport {
        alignment,
        downsample: f,
        overall,
        overall_mean_abs_diff,
        regions: scores,
    })
}

/// `rect` (render pixels) as a `[x0, x1) × [y0, y1)` box on a grid
/// downsampled by `f`, clamped to `w`×`h`; `None` when empty.
fn grid_box(rect: &Rect, f: f64, w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    let cell = |v: f64, max: usize| -> usize {
        // Clamped to [0, max], so the cast is exact.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let c = v.clamp(0.0, max as f64) as usize;
        c
    };
    let x0 = cell((rect.x / f).floor(), w);
    let y0 = cell((rect.y / f).floor(), h);
    let x1 = cell((rect.right() / f).ceil(), w);
    let y1 = cell((rect.bottom() / f).ceil(), h);
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
}

/// Regions of the visible, non-group visuals of a laid-out page, in render
/// pixels for a render at `pixel_scale` (canvas pixels × scale).
pub fn regions_from_layout(layout: &PageLayout, pixel_scale: f64) -> Vec<Region> {
    layout
        .visible()
        .filter(|f| !f.is_group && !f.outer.is_empty())
        .map(|f| {
            let r = layout.to_canvas(&f.outer);
            Region {
                name: f.name.clone(),
                visual_type: f.visual_type.clone(),
                title: f.title.as_ref().map(|t| t.text.clone()),
                rect: Rect::new(
                    r.x * pixel_scale,
                    r.y * pixel_scale,
                    r.width * pixel_scale,
                    r.height * pixel_scale,
                ),
            }
        })
        .collect()
}

/// [`regions_from_layout`] for a page laid out the way
/// [`render_report_page`](crate::visuals::render_report_page) does with
/// the default options (no viewport: the page at its own size).
pub fn page_regions(
    report: &Report,
    page: &Page,
    theme: &ThemeResolver,
    pixel_scale: f64,
) -> Vec<Region> {
    let layout = PageLayout::compute_with(page, theme, &LayoutOptions::for_report(report, None));
    regions_from_layout(&layout, pixel_scale)
}

/// Rasterizes a scene at `scale` on white (as Power BI exports pages).
///
/// # Errors
/// Rendering failures ([`PngError`]).
pub fn rasterize(scene: &Scene, scale: f32) -> Result<RgbaImage, GoldenError> {
    let img = render_rgba(
        scene,
        &PngOptions {
            scale,
            background: Some(Color::WHITE),
        },
    )?;
    from_render(img)
}

/// Converts a [`crate::png::RgbaImage`] to an [`image::RgbaImage`].
///
/// # Errors
/// [`GoldenError::Buffer`] if the buffer does not match the size.
pub fn from_render(img: crate::png::RgbaImage) -> Result<RgbaImage, GoldenError> {
    let (width, height, len) = (img.width, img.height, img.data.len());
    RgbaImage::from_raw(width, height, img.data).ok_or(GoldenError::Buffer { width, height, len })
}

/// Decodes a PNG file as RGBA (at most [`MAX_SIDE`] px per side and
/// [`MAX_BYTES`] decoded).
///
/// # Errors
/// [`GoldenError::Read`] if the file cannot be read or decoded.
pub fn load_png(path: &Path) -> Result<RgbaImage, GoldenError> {
    let err = |message: String| GoldenError::Read {
        path: path.display().to_string(),
        message,
    };
    let mut reader = image::ImageReader::open(path).map_err(|e| err(e.to_string()))?;
    reader.set_format(ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_BYTES);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| err(e.to_string()))?;
    Ok(img.to_rgba8())
}

/// Writes `img` as a PNG, creating the parent folders.
///
/// # Errors
/// [`GoldenError::Write`] on I/O or encoding failures.
pub fn save_png(img: &RgbaImage, path: &Path) -> Result<(), GoldenError> {
    let err = |message: String| GoldenError::Write {
        path: path.display().to_string(),
        message,
    };
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| err(e.to_string()))?;
    }
    img.save_with_format(path, ImageFormat::Png)
        .map_err(|e| err(e.to_string()))
}

/// A heatmap of the luminance difference of `render` and the aligned
/// golden (same grid, e.g. from [`resample`]): the render faded to light
/// grey, red where they differ (opaque at ≥ 1/3 of the range). With a
/// report, every region is outlined: green when its SSIM ≥ `threshold`,
/// red otherwise.
pub fn diff_heatmap(
    render: &RgbaImage,
    aligned_golden: &RgbaImage,
    report: Option<&GoldenReport>,
    threshold: f64,
) -> RgbaImage {
    let (w, h) = (
        render.width().min(aligned_golden.width()),
        render.height().min(aligned_golden.height()),
    );
    let luma = |p: &Rgba<u8>| {
        let a = f64::from(p[3]) / 255.0;
        let c = |v: u8| f64::from(v) * a + 255.0 * (1.0 - a);
        0.299 * c(p[0]) + 0.587 * c(p[1]) + 0.114 * c(p[2])
    };
    let mut out = RgbaImage::from_fn(w, h, |x, y| {
        let r = luma(render.get_pixel(x, y));
        let g = luma(aligned_golden.get_pixel(x, y));
        let t = ((r - g).abs() / 255.0 * 3.0).min(1.0);
        let base = 170.0 + r / 255.0 * 75.0;
        let mix = |from: f64, to: f64| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let v = (from + (to - from) * t).round().clamp(0.0, 255.0) as u8;
            v
        };
        Rgba([mix(base, 220.0), mix(base, 0.0), mix(base, 0.0), 255])
    });
    if let Some(report) = report {
        for r in &report.regions {
            let color = if r.ssim >= threshold {
                Rgba([0, 160, 60, 255])
            } else {
                Rgba([200, 0, 0, 255])
            };
            outline(&mut out, &r.rect, color, 3);
        }
    }
    out
}

/// Draws a `width`-px rectangle outline (inside `rect`).
fn outline(img: &mut RgbaImage, rect: &Rect, color: Rgba<u8>, width: u32) {
    let px = |v: f64, max: u32| -> u32 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let p = v.round().clamp(0.0, f64::from(max)) as u32;
        p
    };
    let (iw, ih) = (img.width(), img.height());
    let (x0, y0) = (px(rect.x, iw), px(rect.y, ih));
    let (x1, y1) = (px(rect.right(), iw), px(rect.bottom(), ih));
    for y in y0..y1 {
        for x in x0..x1 {
            let edge = x < x0 + width || x + width >= x1 || y < y0 + width || y + width >= y1;
            if edge {
                img.put_pixel(x, y, color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn white(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([255, 255, 255, 255]))
    }

    fn fill(img: &mut RgbaImage, rect: (u32, u32, u32, u32), c: [u8; 4]) {
        for y in rect.1..rect.1 + rect.3 {
            for x in rect.0..rect.0 + rect.2 {
                img.put_pixel(x, y, Rgba(c));
            }
        }
    }

    /// A 200×100 "page": a striped card at the left, text-like bars at the
    /// right.
    fn page() -> RgbaImage {
        let mut img = white(200, 100);
        fill(&mut img, (10, 10, 80, 80), [30, 30, 30, 255]);
        fill(&mut img, (20, 20, 60, 10), [255, 255, 255, 255]);
        fill(&mut img, (20, 50, 30, 10), [255, 255, 255, 255]);
        for i in 0..6 {
            fill(
                &mut img,
                (110, 10 + i * 14, 70 - i * 8, 6),
                [20, 90, 200, 255],
            );
        }
        img
    }

    #[test]
    fn regions_are_scored_after_alignment() {
        let render = page();
        // The export: the page 12 px down on a taller white canvas, and the
        // right half changed.
        let mut golden = white(200, 120);
        image::imageops::overlay(&mut golden, &render, 0, 12);
        fill(&mut golden, (110, 22, 80, 80), [255, 255, 255, 255]);
        fill(&mut golden, (120, 60, 40, 30), [200, 30, 30, 255]);
        let regions = [
            Region::new("card", "cardVisual", Rect::new(10.0, 10.0, 80.0, 80.0)),
            Region::new("bars", "barChart", Rect::new(105.0, 5.0, 90.0, 90.0)),
            Region::new("outside", "shape", Rect::new(300.0, 0.0, 10.0, 10.0)),
        ];
        let report =
            compare_regions(&render, &golden, &regions, &CompareOptions::default()).unwrap();
        assert_eq!(report.alignment.offset_y, 12.0);
        assert_eq!(report.regions.len(), 2, "empty regions are left out");
        let card = &report.regions[0];
        assert!(card.ssim > 0.999, "{card:?}");
        assert!(card.mean_abs_diff < 1e-6);
        let bars = &report.regions[1];
        assert!(bars.ssim < 0.85, "{bars:?}");
        assert_eq!(report.below(0.85).len(), 1);
        assert_eq!(report.worst().unwrap().name, "bars");
        assert!(report.overall < 1.0 && report.overall > bars.ssim);
        assert!(report.table().contains("barChart"));

        let aligned = resample(&golden, &report.alignment, 200, 100);
        let heat = diff_heatmap(&render, &aligned, Some(&report), 0.85);
        assert_eq!((heat.width(), heat.height()), (200, 100));
        assert_eq!(heat.get_pixel(10, 10), &Rgba([0, 160, 60, 255]));
        assert_eq!(heat.get_pixel(105, 50), &Rgba([200, 0, 0, 255]));
        // Changed pixels are red, identical ones grey.
        assert_eq!(heat.get_pixel(155, 75)[1], 0);
        let same = heat.get_pixel(50, 95);
        assert_eq!(same[0], same[1]);
    }

    #[test]
    fn empty_images_are_rejected_and_buffers_checked() {
        let e = compare_regions(&white(0, 0), &white(4, 4), &[], &CompareOptions::default());
        assert!(matches!(e, Err(GoldenError::Empty)));
        let bad = crate::png::RgbaImage {
            width: 2,
            height: 2,
            data: vec![0; 3],
        };
        assert!(matches!(from_render(bad), Err(GoldenError::Buffer { .. })));
    }

    #[test]
    fn png_files_round_trip_and_bad_files_are_errors() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp/golden-unit");
        let file = dir.join("round-trip.png");
        let img = page();
        save_png(&img, &file).unwrap();
        assert_eq!(load_png(&file).unwrap(), img);
        let not_png = dir.join("not.png");
        std::fs::write(&not_png, b"nope").unwrap();
        assert!(matches!(load_png(&not_png), Err(GoldenError::Read { .. })));
        assert!(matches!(
            load_png(&dir.join("missing.png")),
            Err(GoldenError::Read { .. })
        ));
    }
}
