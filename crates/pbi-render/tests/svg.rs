//! SVG output: determinism, escaping, hit attributes, clips, fonts and
//! well-formedness (parsed back with roxmltree and usvg).

mod layout_fixtures;

use layout_fixtures::{EXECUTIVE_SUMMARY, STORE_PERFORMANCE, overview_report, stores_report};
use pbi_render::layout::{FrameOptions, render_page_frames};
use pbi_render::scene::{
    Clip, Color, Group, HitId, ImageFit, ImageNode, Paint, PathData, PathNode, Rect, RectNode,
    Scene, Stroke, TextAnchor, TextNode, Transform,
};
use pbi_render::svg::{SvgOptions, to_svg};
use pbi_render::text::TextStyle;
use usvg::roxmltree;

fn sample_scene() -> Scene {
    let mut scene = Scene::new(200.5, 100.0);
    scene
        .root
        .push(RectNode::new(Rect::new(0.0, 0.0, 200.5, 100.0)).fill(Paint::solid(Color::WHITE)));
    let mut visual = Group::new()
        .with_transform(Transform::translate(10.0, 10.0))
        .with_clip(Clip::rounded(Rect::new(0.0, 0.0, 120.0, 60.0), 4.0))
        .with_id(HitId::visual("v|1"))
        .with_class("pbi-visual");
    visual.push(
        RectNode::new(Rect::new(0.5, 0.5, 119.0, 59.0))
            .radius(4.0)
            .fill(Paint::new(Color::rgb(0xA0, 0xD1, 0xFF), 0.5))
            .stroke(Stroke::new(Color::BLACK, 1.0).with_dash(vec![2.0, 1.0])),
    );
    let mut point = Group::new().with_id(HitId::data_point("v|1", 0, 2));
    point.push(
        PathNode::new(PathData::pie_slice(60.0, 30.0, 20.0, 10.0, 0.0, 120.0))
            .fill(Paint::solid(Color::rgb(0x11, 0x8D, 0xFF))),
    );
    visual.push(point);
    visual.push(
        TextNode::new(
            60.0,
            50.0,
            "A < B & \"C\" 'd'  x\u{7}",
            TextStyle::new("Segoe UI Semibold", 10.0, Color::rgb(0x25, 0x24, 0x23)),
        )
        .anchor(TextAnchor::Middle)
        .clip(Rect::new(0.0, 40.0, 120.0, 20.0)),
    );
    visual.push(ImageNode {
        rect: Rect::new(0.0, 0.0, 10.0, 10.0),
        href: "file:///etc/passwd".into(),
        fit: ImageFit::Contain,
    });
    visual.push(ImageNode {
        rect: Rect::new(0.0, 0.0, 10.0, 10.0),
        href: "data:image/png;base64,iVBORw0KGgo=".into(),
        fit: ImageFit::Cover,
    });
    visual.line(0.0, 0.0, 1.0 / 3.0, -0.0, Stroke::new(Color::BLACK, 0.5));
    scene.root.push(visual);
    scene
        .root
        .push(Group::new().with_id(HitId::button("empty")));
    scene
}

#[test]
fn deterministic_sorted_and_escaped() {
    let scene = sample_scene();
    let svg = to_svg(&scene, &SvgOptions::default());
    assert_eq!(svg, to_svg(&scene.clone(), &SvgOptions::default()));
    assert!(svg.starts_with(
        "<svg height=\"100\" viewBox=\"0 0 200.5 100\" width=\"200.5\" xmlns=\"http://www.w3.org/2000/svg\">"
    ));
    assert!(svg.contains(
        "<g class=\"pbi-visual\" clip-path=\"url(#c0)\" data-hit=\"v%7C1|visual\" transform=\"translate(10 10)\">"
    ));
    assert!(svg.contains("data-hit=\"v%7C1|dataPoint|0|2\""));
    assert!(svg.contains("<clipPath id=\"c0\"><rect height=\"60\" rx=\"4\" ry=\"4\" width=\"120\" x=\"0\" y=\"0\"/></clipPath>"));
    assert!(svg.contains("fill-opacity=\"0.5\""));
    assert!(svg.contains("stroke-dasharray=\"2 1\""));
    assert!(svg.contains("x2=\"0.333\" y1=\"0\" y2=\"0\""));
    assert!(svg.contains("font-family=\"Selawik, &apos;Segoe UI&apos;, sans-serif\""));
    assert!(svg.contains("font-weight=\"600\""));
    assert!(svg.contains("text-anchor=\"middle\""));
    assert!(svg.contains(">A &lt; B &amp; \"C\" 'd'  x</text>"));
    assert!(!svg.contains("passwd"));
    assert!(svg.contains("preserveAspectRatio=\"xMidYMid slice\""));
    // Empty groups are omitted.
    assert!(!svg.contains("empty"));
    // No @font-face unless asked for.
    assert!(!svg.contains("@font-face"));

    let doc = roxmltree::Document::parse(&svg).expect("well-formed XML");
    let text = doc
        .descendants()
        .find(|n| n.has_tag_name("text"))
        .and_then(|n| n.text())
        .expect("text");
    assert_eq!(text, "A < B & \"C\" 'd'  x");
    let hits: Vec<&str> = doc
        .descendants()
        .filter_map(|n| n.attribute("data-hit"))
        .collect();
    assert_eq!(hits, ["v%7C1|visual", "v%7C1|dataPoint|0|2"]);
    assert_eq!(HitId::decode(hits[1]), Some(HitId::data_point("v|1", 0, 2)));
    usvg::Tree::from_str(&svg, &usvg::Options::default()).expect("usvg parses");
}

#[test]
fn font_faces_are_declared_only_when_requested() {
    let scene = sample_scene();
    let local = to_svg(
        &scene,
        &SvgOptions {
            embed_fonts: false,
            font_face_css: true,
        },
    );
    assert_eq!(local.matches("@font-face").count(), 1);
    assert!(local.contains("src:local(\"Selawik Semibold\")"));
    let embedded = to_svg(
        &scene,
        &SvgOptions {
            embed_fonts: true,
            font_face_css: false,
        },
    );
    assert!(embedded.contains("font-weight:600;src:url(data:font/ttf;base64,AAEAAA"));
    roxmltree::Document::parse(&embedded).expect("well-formed XML");
    usvg::Tree::from_str(&embedded, &usvg::Options::default()).expect("usvg parses");
}

#[test]
fn page_frames_svg_is_well_formed_and_stable() {
    let pages = [
        (overview_report(), EXECUTIVE_SUMMARY, "Sales by channel"),
        (stores_report(), STORE_PERFORMANCE, "Sales against target"),
    ];
    for (fx, page, title) in pages {
        let Some(fx) = fx else { continue };
        let page = fx.page(page);
        let opts = FrameOptions {
            viewport: Some((1920.0, 1080.0)),
            ..FrameOptions::default()
        };
        let scene = render_page_frames(&fx.report, page, &fx.theme, &opts);
        let svg = to_svg(&scene, &SvgOptions::default());
        let again = to_svg(
            &render_page_frames(&fx.report, page, &fx.theme, &opts),
            &SvgOptions::default(),
        );
        assert_eq!(svg, again);
        let doc = roxmltree::Document::parse(&svg).expect("well-formed XML");
        let visuals = doc
            .descendants()
            .filter(|n| n.attribute("class") == Some("pbi-visual"))
            .count();
        assert_eq!(visuals, page.visuals.len());
        assert!(
            svg.contains("transform=\"scale(1.5 1.5)\""),
            "FitToPage at 1920×1080"
        );
        assert!(svg.contains(&format!(">{title}</text>")), "{title}");
        let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).expect("usvg parses");
        assert_eq!(tree.size().width(), 1920.0);
        assert_eq!(tree.size().height(), 1080.0);
    }
}
