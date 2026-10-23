//! Page layout of the Acme Outfitters report pages: size, scaling, z-order,
//! container chrome from the custom theme and `visualContainerObjects`.

mod layout_fixtures;

use layout_fixtures::{
    EXECUTIVE_SUMMARY, PRODUCT_DETAIL, STORE_PERFORMANCE, STORE_TOOLTIP, overview_report,
    stores_report,
};
use pbi_model::DisplayOption;
use pbi_render::layout::{LayoutOptions, PageLayout, TextAlign, VerticalAlignment};
use pbi_render::scene::{Color, Rect};

const HEADER_SHAPE: &str = "923adeb43119cf2eccfc";
const KPI_CARDS: &str = "f7a9c9413fef68532313";
const SCALED_CARD: &str = "d1534ca5259c4de6151d";
const DONUT: &str = "c9f87905246642c35687";
const NAV_BUTTON: &str = "5cd0757711b84d2d8214";
const NOTES_TEXTBOX: &str = "b8b18fd95a6ed3ba97a1";

#[test]
fn executive_summary_size_order_and_count() {
    let Some(fx) = overview_report() else { return };
    let page = fx.page(EXECUTIVE_SUMMARY);
    let layout = PageLayout::compute_with(page, &fx.theme, &LayoutOptions::default());
    assert_eq!(layout.page_size, (1280.0, 720.0));
    assert_eq!(layout.canvas_size, (1280.0, 720.0));
    assert_eq!((layout.scale, layout.offset), (1.0, (0.0, 0.0)));
    assert_eq!(layout.display_option, DisplayOption::FitToPage);
    assert_eq!(layout.visuals.len(), 14);
    assert!(layout.visuals.iter().all(|f| !f.hidden && !f.is_group));
    let z: Vec<f64> = layout.visuals.iter().map(|f| f.z).collect();
    assert!(z.windows(2).all(|w| w[0] <= w[1]), "{z:?}");
    let first = layout.visuals.first().expect("first");
    let last = layout.visuals.last().expect("last");
    assert_eq!(
        (first.name.as_str(), first.visual_type.as_str(), first.z),
        (HEADER_SHAPE, "shape", 0.0)
    );
    assert_eq!((last.name.as_str(), last.z), (NOTES_TEXTBOX, 13000.0));
    let donut = layout.frame(DONUT).expect("donut");
    assert_eq!(donut.outer, Rect::new(24.0, 496.0, 300.0, 208.0));
    // Every visual lies on the page.
    let page_rect = Rect::new(0.0, 0.0, 1280.0, 720.0);
    for f in &layout.visuals {
        assert!(
            page_rect.contains_rect(&f.outer, 0.01),
            "{} {:?}",
            f.name,
            f.outer
        );
    }
}

#[test]
fn pages_scale_to_viewports() {
    let Some(fx) = overview_report() else { return };
    let opts = |viewport| LayoutOptions::for_report(&fx.report, viewport);
    // Product Detail is FitToWidth (1280×1080): fills the width, scrolls.
    let detail = fx.page(PRODUCT_DETAIL);
    let wide = PageLayout::compute_with(detail, &fx.theme, &opts(Some((2560.0, 1000.0))));
    assert_eq!(wide.display_option, DisplayOption::FitToWidth);
    assert_eq!((wide.scale, wide.canvas_size), (2.0, (2560.0, 2160.0)));
    assert_eq!(
        wide.to_canvas(&Rect::new(10.0, 10.0, 100.0, 50.0)),
        Rect::new(20.0, 20.0, 200.0, 100.0)
    );
    // The report sets no vertical alignment: the default centres a page
    // shorter than the viewport ((1000 - 540) / 2).
    assert_eq!(
        LayoutOptions::for_report(&fx.report, None).vertical_alignment,
        VerticalAlignment::Middle
    );
    let tall = PageLayout::compute_with(detail, &fx.theme, &opts(Some((640.0, 1000.0))));
    assert_eq!((tall.scale, tall.offset), (0.5, (0.0, 230.0)));
    let top = LayoutOptions {
        vertical_alignment: VerticalAlignment::Top,
        ..opts(Some((640.0, 1000.0)))
    };
    let tall = PageLayout::compute_with(detail, &fx.theme, &top);
    assert_eq!((tall.scale, tall.offset), (0.5, (0.0, 0.0)));
    // Executive Summary is FitToPage: letterboxed and centred horizontally.
    let summary = fx.page(EXECUTIVE_SUMMARY);
    let fit = PageLayout::compute_with(summary, &fx.theme, &opts(Some((1600.0, 720.0))));
    assert_eq!((fit.scale, fit.offset), (1.0, (160.0, 0.0)));

    // The 320×240 tooltip page is ActualSize: never scaled, centred.
    let Some(stores) = stores_report() else {
        return;
    };
    let tooltip = stores.page(STORE_TOOLTIP);
    let actual = PageLayout::compute_with(
        tooltip,
        &stores.theme,
        &LayoutOptions::for_report(&stores.report, Some((1280.0, 720.0))),
    );
    assert_eq!(actual.display_option, DisplayOption::ActualSize);
    assert_eq!(actual.page_size, (320.0, 240.0));
    assert_eq!((actual.scale, actual.offset), (1.0, (480.0, 240.0)));
}

#[test]
fn header_shapes_and_buttons_have_no_chrome() {
    let Some(fx) = overview_report() else { return };
    let layout = PageLayout::compute_with(
        fx.page(EXECUTIVE_SUMMARY),
        &fx.theme,
        &LayoutOptions::default(),
    );
    // The header band switches the theme's background, border and title off.
    let header = layout.frame(HEADER_SHAPE).expect("header");
    assert_eq!(header.outer, Rect::new(0.0, 0.0, 1280.0, 64.0));
    assert!(header.background.is_none() && header.border.is_none() && header.title.is_none());
    assert!(!header.header.show);
    // No border: content is the frame minus the default 5px padding.
    assert_eq!(header.inner, header.outer);
    assert_eq!(header.content, Rect::new(5.0, 5.0, 1270.0, 54.0));
    let button = layout.frame(NAV_BUTTON).expect("button");
    assert!(button.background.is_none() && button.border.is_none());
    assert!(!button.header.show);
}

#[test]
fn container_chrome_from_the_theme_and_visual_container_objects() {
    let Some(fx) = overview_report() else { return };
    let page = fx.page(EXECUTIVE_SUMMARY);
    let layout = PageLayout::compute_with(page, &fx.theme, &LayoutOptions::default());

    // Donut: the theme's 1px #D9D4C7 border with 8px corners, a white
    // background, an explicit bold 11pt title.
    let donut = layout.frame(DONUT).expect("donut");
    let border = donut.border.as_ref().expect("border");
    assert_eq!(
        (border.color, border.width),
        (Color::rgb(0xD9, 0xD4, 0xC7), 1.0)
    );
    assert_eq!(donut.radius, 8.0);
    assert_eq!(donut.inner, Rect::new(25.0, 497.0, 298.0, 206.0));
    assert_eq!(
        donut.background.map(|p| (p.color, p.opacity)),
        Some((Color::WHITE, 1.0))
    );
    let title = donut.title.as_ref().expect("title");
    assert_eq!(title.text, "Sales by channel");
    assert_eq!(title.alignment, TextAlign::Left);
    assert_eq!(
        (
            title.style.family.as_str(),
            title.style.size_pt,
            title.style.weight
        ),
        ("Segoe UI Semibold", 11.0, 700)
    );
    assert_eq!(title.style.color, Color::rgb(0x1F, 0x3A, 0x2E));
    assert_eq!(title.lines.len(), 1);
    // The title band starts at the inner rect, inset by the 5px padding.
    assert_eq!((title.rect.x, title.rect.y), (30.0, 497.0));
    assert_eq!(donut.content.y, title.rect.bottom());
    assert_eq!(donut.content.x, 30.0);
    assert!((donut.content.bottom() - 698.0).abs() < 1e-9);

    // Card without a title: content starts 6px inside (border + padding).
    let kpis = layout.frame(KPI_CARDS).expect("cards");
    assert!(kpis.title.is_none());
    assert_eq!(kpis.content, Rect::new(30.0, 158.0, 988.0, 112.0));
    assert!(kpis.header.show);

    // Legacy card with a title.
    let scaled = layout.frame(SCALED_CARD).expect("card");
    assert_eq!(
        scaled.title.as_ref().map(|t| t.text.as_str()),
        Some("Sales (dynamic format)")
    );
}

#[test]
fn compute_without_theme_uses_builtin_defaults() {
    let Some(fx) = stores_report() else { return };
    let page = fx.page(STORE_PERFORMANCE);
    let layout = PageLayout::compute(page, Some((1280.0, 720.0)));
    assert_eq!(layout.visuals.len(), 14);
    let gauge = layout.frame("6f647d31a8428ad62fb0").expect("gauge");
    // The visual's own border object still applies; its title keeps the
    // explicit text with the built-in title class (DIN 12pt) for the rest.
    assert_eq!(gauge.border.as_ref().map(|b| b.width), Some(1.0));
    let title = gauge.title.as_ref().expect("title");
    assert_eq!(title.text, "Sales against target");
    assert_eq!(title.style.family, "DIN");
}

fn lit(raw: &str) -> serde_json::Value {
    serde_json::json!({"expr": {"Literal": {"Value": raw}}})
}

#[test]
fn shadow_radius_title_alignment_and_odd_values() {
    use pbi_model::{Page, Position, Report, Visual};
    use pbi_render::layout::{FrameOptions, frame_group, render_page_frames};
    use pbi_render::scene::{HitId, Node, Paint};
    use pbi_render::theme::ThemeResolver;
    use serde_json::json;

    let visual = Visual {
        name: "v1".into(),
        visual_type: "lineChart".into(),
        position: Position {
            x: 100.0,
            y: 50.0,
            width: 300.0,
            height: 200.0,
            ..Position::default()
        },
        container_objects: json!({
            "dropShadow": [{"properties": {"show": lit("true"), "preset": lit("'Bottom'"),
                "shadowDistance": lit("8L"), "transparency": lit("40L")}}],
            "border": [{"properties": {"show": lit("true"), "radius": lit("10D"),
                "width": lit("2D"), "color": {"solid": {"color": lit("'#123456'")}}}}],
            "background": [{"properties": {"transparency": lit("50D")}}],
            "title": [{"properties": {"text": lit("'Centered'"), "alignment": lit("'center'"),
                "fontSize": lit("'not a number'"), "background": {"solid": {"color": lit("'#EEEEEE'")}}}}],
            "padding": [{"properties": {"left": lit("-7D"), "top": lit("bogus")}}],
            "visualHeader": "not an array"
        }),
        ..Visual::default()
    };
    let page = Page {
        visuals: vec![visual],
        ..Page::default()
    };
    let theme = ThemeResolver::default();
    let layout = PageLayout::compute_with(&page, &theme, &LayoutOptions::default());
    let f = layout.frame("v1").expect("frame");
    let shadow = f.drop_shadow.as_ref().expect("shadow");
    assert!((shadow.offset.0).abs() < 1e-9 && (shadow.offset.1 - 8.0).abs() < 1e-9);
    assert!((shadow.opacity - 0.6).abs() < 1e-9);
    assert_eq!(f.radius, 10.0);
    let border = f.border.as_ref().expect("border");
    assert_eq!(
        (border.color, border.width),
        (Color::rgb(0x12, 0x34, 0x56), 2.0)
    );
    assert_eq!(f.inner, Rect::new(102.0, 52.0, 296.0, 196.0));
    assert_eq!(f.background, Some(Paint::new(Color::WHITE, 0.5)));
    // Invalid values fall back to defaults; negative padding clamps to 0.
    assert_eq!(f.padding.left, 0.0);
    assert_eq!(f.padding.top, 5.0);
    let title = f.title.as_ref().expect("title");
    assert_eq!(title.alignment, TextAlign::Center);
    assert_eq!(title.style.size_pt, 12.0);
    assert_eq!(
        title.background.map(|p| p.color),
        Some(Color::rgb(0xEE, 0xEE, 0xEE))
    );

    let group = frame_group(f, None);
    assert_eq!(group.id, Some(HitId::visual("v1")));
    let shadows = group
        .children
        .iter()
        .filter(|n| matches!(n, Node::Path(_)))
        .count();
    assert_eq!(shadows, 4);
    let texts: Vec<_> = group
        .children
        .iter()
        .filter_map(|n| match n {
            Node::Text(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(texts.len(), 1);
    assert_eq!(texts[0].x, title.rect.x + title.rect.width / 2.0);
    let scene = render_page_frames(&Report::default(), &page, &theme, &FrameOptions::default());
    let svg = pbi_render::svg::to_svg(&scene, &Default::default());
    assert!(
        svg.contains("rx=\"9\""),
        "border radius minus half the stroke"
    );
}
