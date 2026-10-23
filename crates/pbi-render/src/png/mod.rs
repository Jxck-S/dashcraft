//! Headless rasterization: scene or SVG → PNG through `usvg` + `resvg` +
//! `tiny-skia`.
//!
//! Deterministic by construction: the font database holds **only** the
//! bundled Selawik and Carlito faces
//! ([`crate::text::fonts::usvg_database`]; system fonts are never loaded),
//! `<image>` hrefs resolve only `data:` URIs (no file or network access),
//! and PNG encoding has no timestamps. The same input gives the same bytes
//! on every OS.

use std::sync::{Arc, OnceLock};

use usvg::ImageHrefResolver;

use crate::scene::{Color, Scene};
use crate::svg::{SvgOptions, to_svg};
use crate::text::FAMILY;
use crate::text::fonts::usvg_database;

/// Largest output side in pixels.
pub const MAX_SIDE: u32 = 16_384;
/// Largest output area in pixels (256 MiB of RGBA).
pub const MAX_PIXELS: u64 = 64 * 1024 * 1024;

/// PNG rendering options.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PngOptions {
    /// Output pixels per scene pixel (2.0 for a 2560×1440 render of a
    /// 1280×720 page).
    pub scale: f32,
    /// Opaque background under the scene (`None`: transparent).
    pub background: Option<Color>,
}

impl Default for PngOptions {
    fn default() -> Self {
        Self {
            scale: 1.0,
            background: None,
        }
    }
}

impl PngOptions {
    pub fn with_scale(scale: f32) -> Self {
        Self {
            scale,
            ..Self::default()
        }
    }
}

/// Rendering errors.
#[derive(Debug, thiserror::Error)]
pub enum PngError {
    #[error("invalid SVG: {0}")]
    Svg(#[from] usvg::Error),
    #[error("invalid output size {width}x{height} (scale {scale})")]
    Size { width: f32, height: f32, scale: f32 },
    #[error("PNG encoding failed: {0}")]
    Encode(String),
}

/// A demultiplied RGBA8 image (row-major, 4 bytes per pixel).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Renders a scene to PNG bytes.
pub fn render_png(scene: &Scene, opts: &PngOptions) -> Result<Vec<u8>, PngError> {
    render_svg_png(&to_svg(scene, &SvgOptions::default()), opts)
}

/// Renders an SVG document to PNG bytes.
pub fn render_svg_png(svg: &str, opts: &PngOptions) -> Result<Vec<u8>, PngError> {
    rasterize(svg, opts)?
        .encode_png()
        .map_err(|e| PngError::Encode(e.to_string()))
}

/// Renders a scene to raw RGBA (e.g. for image comparisons).
pub fn render_rgba(scene: &Scene, opts: &PngOptions) -> Result<RgbaImage, PngError> {
    let pixmap = rasterize(&to_svg(scene, &SvgOptions::default()), opts)?;
    let (width, height) = (pixmap.width(), pixmap.height());
    let data = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    Ok(RgbaImage {
        width,
        height,
        data,
    })
}

/// The bundled-only font database, shared across renders.
fn fontdb() -> Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| Arc::new(usvg_database())).clone()
}

/// usvg options: bundled fonts only, `data:` images only.
fn usvg_options() -> usvg::Options<'static> {
    usvg::Options {
        font_family: FAMILY.to_owned(),
        fontdb: fontdb(),
        image_href_resolver: ImageHrefResolver {
            resolve_data: ImageHrefResolver::default_data_resolver(),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    }
}

fn rasterize(svg: &str, opts: &PngOptions) -> Result<tiny_skia::Pixmap, PngError> {
    let tree = usvg::Tree::from_str(svg, &usvg_options())?;
    let size = tree.size();
    let scale = opts.scale;
    let bad = || PngError::Size {
        width: size.width(),
        height: size.height(),
        scale,
    };
    if !(scale.is_finite() && scale > 0.0) {
        return Err(bad());
    }
    let px = |v: f32| -> Option<u32> {
        let p = (v * scale - 1e-3).ceil();
        // Only used when range-checked, so the (saturating) cast is exact.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        (p.is_finite() && p >= 1.0 && p <= MAX_SIDE as f32).then_some(p as u32)
    };
    let (width, height) = (
        px(size.width()).ok_or_else(bad)?,
        px(size.height()).ok_or_else(bad)?,
    );
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(bad());
    }
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or_else(bad)?;
    if let Some(bg) = opts.background {
        pixmap.fill(tiny_skia::Color::from_rgba8(bg.r, bg.g, bg.b, 255));
    }
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_sizes_and_svg() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"/>"#;
        assert!(matches!(
            render_svg_png(svg, &PngOptions::with_scale(0.0)),
            Err(PngError::Size { .. })
        ));
        assert!(matches!(
            render_svg_png(svg, &PngOptions::with_scale(f32::NAN)),
            Err(PngError::Size { .. })
        ));
        assert!(matches!(
            render_svg_png(svg, &PngOptions::with_scale(10_000.0)),
            Err(PngError::Size { .. })
        ));
        assert!(matches!(
            render_svg_png("not svg", &PngOptions::default()),
            Err(PngError::Svg(_))
        ));
        assert!(render_svg_png(svg, &PngOptions::default()).is_ok());
    }

    #[test]
    fn file_hrefs_are_not_loaded() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><image href="/etc/hosts" width="4" height="4"/></svg>"#;
        let img = render_rgba(
            &Scene::new(1.0, 1.0),
            &PngOptions {
                scale: 1.0,
                background: Some(Color::WHITE),
            },
        )
        .unwrap();
        assert_eq!(img.data, [255, 255, 255, 255]);
        assert!(render_svg_png(svg, &PngOptions::default()).is_ok());
    }
}
