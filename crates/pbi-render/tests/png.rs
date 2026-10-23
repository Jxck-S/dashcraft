//! Headless PNG rendering: size, content and byte-for-byte determinism.

mod layout_fixtures;

use layout_fixtures::{EXECUTIVE_SUMMARY, STORE_PERFORMANCE, overview_report, stores_report};
use pbi_render::layout::{FrameOptions, render_page_frames};
use pbi_render::png::{PngOptions, render_png, render_rgba, render_svg_png};
use pbi_render::scene::{Color, Paint, Rect, RectNode, Scene, TextAnchor};
use pbi_render::svg::{SvgOptions, to_svg};
use pbi_render::text::TextStyle;

fn pixel(img: &image::RgbaImage, x: u32, y: u32) -> [u8; 4] {
    img.get_pixel(x, y).0
}

fn is_dark(p: [u8; 4]) -> bool {
    u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]) < 300
}

#[test]
fn executive_summary_frames_at_2x() {
    let Some(fx) = overview_report() else { return };
    let scene = render_page_frames(
        &fx.report,
        fx.page(EXECUTIVE_SUMMARY),
        &fx.theme,
        &FrameOptions::default(),
    );
    assert_eq!((scene.width, scene.height), (1280.0, 720.0));
    let opts = PngOptions::with_scale(2.0);
    let png = render_png(&scene, &opts).expect("render");
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    let img = image::load_from_memory(&png).expect("decode").to_rgba8();
    assert_eq!(img.dimensions(), (2560, 1440));
    // Page background from the page's objects: #F7F5EF, opaque.
    let page_bg = [0xF7, 0xF5, 0xEF, 255];
    assert_eq!(pixel(&img, 2, 2), page_bg);
    assert_eq!(pixel(&img, 1400, 200), page_bg);
    // Donut (24, 496, 300×208): the theme's 1px #D9D4C7 border is 2 device
    // pixels, then the white background.
    let (bx, by) = (24 * 2, (496 + 104) * 2);
    assert_eq!(pixel(&img, bx - 1, by), page_bg);
    assert_eq!(pixel(&img, bx, by), [0xD9, 0xD4, 0xC7, 255]);
    assert_eq!(pixel(&img, bx + 1, by), [0xD9, 0xD4, 0xC7, 255]);
    assert_eq!(pixel(&img, bx + 3, by), [255, 255, 255, 255]);
    // The donut title is drawn with the bundled font.
    let title_dark = (2 * 30..2 * 318)
        .flat_map(|x| (2 * 497..2 * 516).map(move |y| (x, y)))
        .filter(|&(x, y)| is_dark(pixel(&img, x, y)))
        .count();
    assert!(title_dark > 200, "title pixels: {title_dark}");

    // Same input → identical bytes; scene and SVG paths agree.
    let again = render_png(&scene, &opts).expect("render");
    assert_eq!(png, again);
    let via_svg = render_svg_png(&to_svg(&scene, &SvgOptions::default()), &opts).expect("svg");
    assert_eq!(png, via_svg);
}

#[test]
fn store_performance_renders_with_fonts_embedded_svg_too() {
    let Some(fx) = stores_report() else { return };
    let scene = render_page_frames(
        &fx.report,
        fx.page(STORE_PERFORMANCE),
        &fx.theme,
        &FrameOptions::default(),
    );
    let opts = PngOptions::with_scale(1.0);
    let plain = render_png(&scene, &opts).expect("render");
    let svg = to_svg(
        &scene,
        &SvgOptions {
            embed_fonts: true,
            font_face_css: true,
        },
    );
    // usvg ignores @font-face; output must not change.
    assert_eq!(render_svg_png(&svg, &opts).expect("render"), plain);
}

#[test]
fn background_and_text_render_deterministically() {
    let mut scene = Scene::new(64.0, 24.0);
    scene.root.push(
        RectNode::new(Rect::new(0.0, 0.0, 32.0, 24.0))
            .fill(Paint::with_transparency(Color::rgb(0xC0, 0, 0), 50.0)),
    );
    scene.root.text(
        2.0,
        18.0,
        "Ag",
        TextStyle::new("Segoe UI Bold", 12.0, Color::BLACK),
        TextAnchor::Start,
    );
    let opts = PngOptions {
        scale: 1.0,
        background: Some(Color::WHITE),
    };
    let rgba = render_rgba(&scene, &opts).expect("render");
    assert_eq!((rgba.width, rgba.height), (64, 24));
    let at = |x: usize, y: usize| {
        let i = (y * 64 + x) * 4;
        [
            rgba.data[i],
            rgba.data[i + 1],
            rgba.data[i + 2],
            rgba.data[i + 3],
        ]
    };
    assert_eq!(at(60, 2), [255, 255, 255, 255]);
    let half = at(30, 2);
    assert!((i32::from(half[0]) - 224).abs() <= 1 && (i32::from(half[1]) - 128).abs() <= 1);
    let dark = rgba
        .data
        .chunks(4)
        .filter(|p| p[0] < 80 && p[1] < 80)
        .count();
    assert!(dark > 20, "glyph pixels: {dark}");
    let a = render_png(&scene, &opts).expect("render");
    let b = render_png(&scene.clone(), &opts).expect("render");
    assert_eq!(a, b);
}
