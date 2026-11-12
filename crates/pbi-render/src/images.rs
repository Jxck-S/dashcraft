//! Report images: `RegisteredResources` items (and `data:` URIs from
//! themes) referenced by formatting objects — image visuals
//! (`general.imageUrl`), page backgrounds (`background.image`), wallpapers
//! (`outspace.image`) and button images (`icon.image`, `fill.image`).
//!
//! The renderer never reads files: an [`ImageSource`] attached to the
//! [`ThemeResolver`](crate::theme::ThemeResolver) hands out the bytes of a
//! package item and [`decode_image`] turns them into a `data:` URI:
//!
//! - PNG and JPEG are decoded in full (so a broken or hostile file is
//!   refused) and embedded as they are;
//! - GIF and BMP become a PNG of their first frame;
//! - SVG is parsed by usvg (no scripts, no external references, bundled
//!   fonts) and written back out;
//! - files over [`MAX_IMAGE_BYTES`], or images over [`MAX_IMAGE_SIDE`]
//!   pixels a side or [`MAX_IMAGE_PIXELS`] in total, are refused.
//!
//! Web URLs (`image.sourceUrl`) are never fetched.

use std::fmt;
use std::io::Cursor;
use std::sync::Arc;

use base64::Engine as _;
use serde_json::Value;

use crate::scene::{ImageFit, ImageNode, Rect};
use crate::theme::value::as_text;

/// Largest image file embedded (bytes).
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
/// Largest width or height (pixels).
pub const MAX_IMAGE_SIDE: u32 = 16_384;
/// Largest pixel count (Desktop-made page backgrounds reach 8K × 4.5K).
pub const MAX_IMAGE_PIXELS: u64 = 48_000_000;
/// Most tiles drawn for `Tile` scaling (more falls back to `Normal`).
const MAX_TILES: usize = 400;

/// Supplies the bytes of report resource-package items.
pub trait ImageSource: Send + Sync + fmt::Debug {
    /// The decoded image of item `item` of package `package`
    /// (`RegisteredResources`, …), `None` when missing or refused.
    fn image(&self, package: &str, item: &str) -> Option<Arc<ImageResource>>;
}

/// A decoded, embeddable image.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageResource {
    /// `data:image/…;base64,…`.
    pub href: String,
    /// Natural size in CSS pixels.
    pub width: f64,
    pub height: f64,
}

/// Why [`decode_image`] refused bytes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImageError {
    #[error("the image is larger than {MAX_IMAGE_BYTES} bytes")]
    TooLarge,
    #[error("the image is larger than {MAX_IMAGE_SIDE} pixels a side or {MAX_IMAGE_PIXELS} pixels")]
    TooManyPixels,
    #[error("unsupported image format")]
    Unsupported,
    #[error("invalid image: {0}")]
    Invalid(String),
}

/// Validates and embeds image bytes (see the module docs).
///
/// # Errors
/// [`ImageError`] when the bytes are too large, not a supported format or
/// do not decode.
pub fn decode_image(bytes: &[u8]) -> Result<ImageResource, ImageError> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(ImageError::TooLarge);
    }
    if looks_like_svg(bytes) {
        return decode_svg(bytes);
    }
    let format = image::guess_format(bytes).map_err(|_| ImageError::Unsupported)?;
    let (mime, reencode) = match format {
        image::ImageFormat::Png => ("image/png", false),
        image::ImageFormat::Jpeg => ("image/jpeg", false),
        image::ImageFormat::Gif | image::ImageFormat::Bmp => ("image/png", true),
        _ => return Err(ImageError::Unsupported),
    };
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits_for(MAX_IMAGE_SIDE, MAX_IMAGE_SIDE));
    let (w, h) = reader
        .into_dimensions()
        .map_err(|e| ImageError::Invalid(e.to_string()))?;
    if w == 0 || h == 0 {
        return Err(ImageError::Invalid("empty image".into()));
    }
    if w > MAX_IMAGE_SIDE || h > MAX_IMAGE_SIDE || u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS {
        return Err(ImageError::TooManyPixels);
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits_for(w, h));
    let decoded = reader.decode().map_err(|e| match e {
        image::ImageError::Limits(_) => ImageError::TooManyPixels,
        e => ImageError::Invalid(e.to_string()),
    })?;
    let data = if reencode {
        let mut out = Cursor::new(Vec::new());
        decoded
            .write_to(&mut out, image::ImageFormat::Png)
            .map_err(|e| ImageError::Invalid(e.to_string()))?;
        out.into_inner()
    } else {
        bytes.to_vec()
    };
    Ok(ImageResource {
        href: data_uri(mime, &data),
        width: f64::from(w),
        height: f64::from(h),
    })
}

fn limits_for(w: u32, h: u32) -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(u64::from(w) * u64::from(h) * 8 + 1024 * 1024);
    limits
}

fn looks_like_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(1024)];
    let text = String::from_utf8_lossy(head);
    let t = text.trim_start_matches('\u{feff}').trim_start();
    (t.starts_with("<?xml") || t.starts_with("<svg") || t.starts_with("<!--"))
        && text.contains("<svg")
}

fn decode_svg(bytes: &[u8]) -> Result<ImageResource, ImageError> {
    let tree = usvg::Tree::from_data(bytes, &crate::png::usvg_options())
        .map_err(|e| ImageError::Invalid(e.to_string()))?;
    let size = tree.size();
    let (w, h) = (f64::from(size.width()), f64::from(size.height()));
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return Err(ImageError::Invalid("empty image".into()));
    }
    let svg = tree.to_string(&usvg::WriteOptions::default());
    Ok(ImageResource {
        href: data_uri("image/svg+xml", svg.as_bytes()),
        width: w,
        height: h,
    })
}

fn data_uri(mime: &str, data: &[u8]) -> String {
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(data)
    )
}

/// Decodes a `data:image/…;base64,…` URI (a theme's embedded image).
#[must_use]
pub fn decode_data_uri(uri: &str) -> Option<ImageResource> {
    let rest = uri.trim().strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    if !meta.to_ascii_lowercase().starts_with("image/") || !meta.ends_with(";base64") {
        return None;
    }
    if payload.len() > MAX_IMAGE_BYTES / 3 * 4 + 4 {
        return None;
    }
    let payload: String = payload.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .ok()?;
    decode_image(&bytes).ok()
}

/// How an image fills its box (`scaling` / `imageScalingType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageScaling {
    /// Natural size, centred, clipped.
    #[default]
    Normal,
    /// Scaled to fit, keeping its aspect ratio.
    Fit,
    /// Scaled to cover, keeping its aspect ratio (clipped).
    Fill,
    /// Repeated at natural size from the top-left corner.
    Tile,
}

impl ImageScaling {
    /// Parses a scaling name (case-insensitive); unknown names are `None`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "normal" => Self::Normal,
            "fit" => Self::Fit,
            "fill" => Self::Fill,
            "tile" => Self::Tile,
            _ => return None,
        })
    }
}

/// Where an image comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageLocation {
    /// A resource-package item (`ResourcePackageItem`).
    Package { package: String, item: String },
    /// An embedded `data:image/…` URI (themes).
    Data(String),
    /// A web URL: never fetched.
    Web(String),
}

/// A parsed image property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    pub location: ImageLocation,
    /// `None` when the property does not say.
    pub scaling: Option<ImageScaling>,
}

/// Parses an image property value: `{"expr": {"ResourcePackageItem":
/// {…}}}` (image visuals), the nested `{"image": {"url": …, "scaling":
/// …}}` (backgrounds, wallpapers, button images, themes) or a URL literal.
#[must_use]
pub fn image_ref(value: &Value) -> Option<ImageRef> {
    let inner = value.get("image").unwrap_or(value);
    let scaling = inner
        .get("scaling")
        .and_then(as_text)
        .and_then(|s| ImageScaling::parse(&s));
    let url = inner.get("url").unwrap_or(inner);
    let location = location(url)?;
    Some(ImageRef { location, scaling })
}

fn location(url: &Value) -> Option<ImageLocation> {
    let expr = url.get("expr").unwrap_or(url);
    if let Some(item) = expr.get("ResourcePackageItem") {
        let text = |k: &str| item.get(k).and_then(Value::as_str).map(str::to_owned);
        return Some(ImageLocation::Package {
            package: text("PackageName").unwrap_or_else(|| "RegisteredResources".into()),
            item: text("ItemName")?,
        });
    }
    let text = as_text(url)?;
    let text = text.trim();
    if text.is_empty() {
        None
    } else if text
        .get(..5)
        .is_some_and(|p| p.eq_ignore_ascii_case("data:"))
    {
        Some(ImageLocation::Data(text.to_owned()))
    } else {
        Some(ImageLocation::Web(text.to_owned()))
    }
}

/// The image nodes drawing `image` in `area` with `scaling`. `Normal` and
/// `Tile` can reach past `area`: the caller clips.
#[must_use]
pub fn place(image: &ImageResource, area: Rect, scaling: ImageScaling) -> Vec<ImageNode> {
    if area.is_empty() || image.width <= 0.0 || image.height <= 0.0 {
        return Vec::new();
    }
    let node = |rect: Rect, fit: ImageFit| ImageNode {
        rect,
        href: image.href.clone(),
        fit,
    };
    let normal = || {
        let rect = Rect::new(
            area.x + (area.width - image.width) / 2.0,
            area.y + (area.height - image.height) / 2.0,
            image.width,
            image.height,
        );
        vec![node(rect, ImageFit::Fill)]
    };
    match scaling {
        ImageScaling::Normal => normal(),
        ImageScaling::Fit => vec![node(area, ImageFit::Contain)],
        ImageScaling::Fill => vec![node(area, ImageFit::Cover)],
        ImageScaling::Tile => {
            let cols = (area.width / image.width).ceil();
            let rows = (area.height / image.height).ceil();
            if !(cols.is_finite() && rows.is_finite()) || cols * rows > MAX_TILES as f64 {
                return normal();
            }
            let mut out = Vec::new();
            let mut y = area.y;
            while y < area.bottom() {
                let mut x = area.x;
                while x < area.right() {
                    out.push(node(
                        Rect::new(x, y, image.width, image.height),
                        ImageFit::Fill,
                    ));
                    x += image.width;
                }
                y += image.height;
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn decodes_png_and_refuses_junk_and_oversized_images() {
        let ok = decode_image(&png(3, 2)).unwrap();
        assert!(ok.href.starts_with("data:image/png;base64,"));
        assert_eq!((ok.width, ok.height), (3.0, 2.0));
        assert_eq!(decode_image(b"hello"), Err(ImageError::Unsupported));
        let mut truncated = png(3, 2);
        truncated.truncate(40);
        assert!(matches!(
            decode_image(&truncated),
            Err(ImageError::Invalid(_))
        ));
        // A header claiming a huge image is refused before decoding.
        let mut huge = png(1, 1);
        huge[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        assert!(decode_image(&huge).is_err());
        assert_eq!(
            decode_image(&vec![0u8; MAX_IMAGE_BYTES + 1]),
            Err(ImageError::TooLarge)
        );
    }

    #[test]
    fn gif_becomes_a_png_of_its_first_frame() {
        let mut out = Vec::new();
        {
            let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
            for color in [[255, 0, 0, 255], [0, 0, 255, 255]] {
                let frame =
                    image::Frame::new(image::RgbaImage::from_pixel(2, 2, image::Rgba(color)));
                enc.encode_frame(frame).unwrap();
            }
        }
        let gif = decode_image(&out).unwrap();
        let b64 = gif.href.strip_prefix("data:image/png;base64,").unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap();
        let first = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(first.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn svg_is_rewritten_without_scripts_or_external_references() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="20" height="10">
            <script>alert(1)</script>
            <rect width="20" height="10" fill="red" onclick="alert(2)"/>
            <image href="https://example.com/x.png" width="5" height="5"/>
            <image xlink:href="file:///etc/passwd" width="5" height="5"/>
        </svg>"#;
        let out = decode_image(svg).unwrap();
        assert_eq!((out.width, out.height), (20.0, 10.0));
        let b64 = out.href.strip_prefix("data:image/svg+xml;base64,").unwrap();
        let text = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(b64)
                .unwrap(),
        )
        .unwrap();
        for bad in ["script", "alert", "onclick", "example.com", "passwd"] {
            assert!(!text.contains(bad), "{bad} in {text}");
        }
        assert!(text.contains("<path") || text.contains("<rect"), "{text}");
        assert!(decode_image(b"<svg><unclosed").is_err());
    }

    #[test]
    fn parses_image_properties() {
        let item = json!({"expr": {"ResourcePackageItem": {
            "PackageName": "RegisteredResources", "PackageType": 1, "ItemName": "logo123.png"
        }}});
        let package = ImageLocation::Package {
            package: "RegisteredResources".into(),
            item: "logo123.png".into(),
        };
        assert_eq!(
            image_ref(&item),
            Some(ImageRef {
                location: package.clone(),
                scaling: None
            })
        );
        let nested = json!({"image": {
            "name": {"expr": {"Literal": {"Value": "'logo.png'"}}},
            "url": item,
            "scaling": {"expr": {"Literal": {"Value": "'Fill'"}}}
        }});
        assert_eq!(
            image_ref(&nested),
            Some(ImageRef {
                location: package,
                scaling: Some(ImageScaling::Fill)
            })
        );
        let theme = json!({"image": {"url": "data:image/png;base64,AAAA", "scaling": "Tile"}});
        assert_eq!(
            image_ref(&theme).map(|r| (r.location, r.scaling)),
            Some((
                ImageLocation::Data("data:image/png;base64,AAAA".into()),
                Some(ImageScaling::Tile)
            ))
        );
        let web = json!({"expr": {"Literal": {"Value": "'https://example.com/a.png'"}}});
        assert!(matches!(
            image_ref(&web).map(|r| r.location),
            Some(ImageLocation::Web(_))
        ));
        assert_eq!(image_ref(&json!({"image": {}})), None);
    }

    #[test]
    fn data_uris_are_validated() {
        let uri = data_uri("image/png", &png(4, 4));
        assert_eq!(decode_data_uri(&uri).map(|i| i.width), Some(4.0));
        assert!(decode_data_uri("data:image/png;base64,AAAA").is_none());
        assert!(decode_data_uri("data:text/html;base64,PHA+").is_none());
    }

    #[test]
    fn placement_by_scaling() {
        let img = ImageResource {
            href: "data:image/png;base64,".into(),
            width: 40.0,
            height: 20.0,
        };
        let area = Rect::new(10.0, 10.0, 100.0, 50.0);
        let normal = place(&img, area, ImageScaling::Normal);
        assert_eq!(normal[0].rect, Rect::new(40.0, 25.0, 40.0, 20.0));
        assert_eq!(normal[0].fit, ImageFit::Fill);
        assert_eq!(
            place(&img, area, ImageScaling::Fit)[0].fit,
            ImageFit::Contain
        );
        assert_eq!(
            place(&img, area, ImageScaling::Fill)[0].fit,
            ImageFit::Cover
        );
        let tiles = place(&img, area, ImageScaling::Tile);
        assert_eq!(tiles.len(), 3 * 3);
        assert_eq!(tiles[1].rect.x, 50.0);
        let tiny = ImageResource {
            width: 1.0,
            height: 1.0,
            ..img.clone()
        };
        assert_eq!(place(&tiny, area, ImageScaling::Tile).len(), 1);
        let small = Rect::new(0.0, 0.0, 10.0, 10.0);
        let cropped = place(&img, small, ImageScaling::Normal);
        assert_eq!(cropped[0].rect, Rect::new(-15.0, -5.0, 40.0, 20.0));
    }
}
