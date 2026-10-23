//! Theme resolution against the Acme Outfitters custom theme
//! (`RegisteredResources/AcmeOutfitters.json`, no base theme) and the
//! formatting objects of the example's visuals.

mod layout_fixtures;

use layout_fixtures::{EXECUTIVE_SUMMARY, online_report, overview_report, stores_report};
use pbi_render::scene::Color;
use pbi_render::text::BundledFace;
use pbi_render::theme::{PropSource, Theme, ThemeResolver, as_bool, as_number};
use serde_json::{Value as Json, json};

const KPI_CARDS: &str = "f7a9c9413fef68532313";
const DONUT: &str = "c9f87905246642c35687";

fn hex(c: Color) -> String {
    c.to_hex()
}

#[test]
fn custom_theme_palette_and_text_classes() {
    let Some(fx) = overview_report() else { return };
    assert!(
        fx.pbir.base_theme().is_none(),
        "the example has no base theme"
    );
    let custom = fx.pbir.custom_theme().expect("custom theme");
    assert_eq!(custom.name.as_deref(), Some("Acme Outfitters"));
    let t = &fx.theme;
    assert_eq!(hex(t.data_color(0)), "#2F6B4F");
    assert_eq!(hex(t.data_color(1)), "#D9822B");
    // The palette cycles.
    assert_eq!(hex(t.data_color(10)), "#2F6B4F");
    assert_eq!(hex(t.foreground()), "#252423");
    assert_eq!(hex(t.background()), "#FFFFFF");
    assert_eq!(
        t.named_color("backgroundLight").map(hex).as_deref(),
        Some("#F3EFE4")
    );
    assert_eq!(t.named_color("bad").map(hex).as_deref(), Some("#B5452E"));

    let title = t.text_class("title");
    assert_eq!(
        (title.family.as_str(), title.size_pt),
        ("Segoe UI Semibold", 12.0)
    );
    assert_eq!(hex(title.color), "#1F3A2E");
    assert_eq!(title.face(), BundledFace::Semibold);
    let callout = t.text_class("callout");
    assert_eq!(
        (callout.family.as_str(), callout.size_pt),
        ("Segoe UI Semibold", 24.0)
    );
    let label = t.text_class("label");
    assert_eq!((label.family.as_str(), label.size_pt), ("Segoe UI", 10.0));
    // Classes the theme does not define derive from the ones it does:
    // `largeTitle` from `title` (14pt), the light labels in
    // `foregroundNeutralSecondary`.
    let large = t.text_class("largeTitle");
    assert_eq!(
        (large.family.as_str(), large.size_pt),
        ("Segoe UI Semibold", 14.0)
    );
    assert_eq!(hex(t.text_class("lightLabel").color), "#5B6770");
}

#[test]
fn theme_data_colors_resolve_against_the_custom_theme() {
    let Some(fx) = overview_report() else { return };
    let t = &fx.theme;
    let tdc = |id: i64, p: f64| json!({"solid": {"color": {"expr": {"ThemeDataColor": {"ColorId": id, "Percent": p}}}}});
    assert_eq!(t.color(&tdc(0, 0.0)).map(hex).as_deref(), Some("#FFFFFF"));
    assert_eq!(t.color(&tdc(0, -0.6)).map(hex).as_deref(), Some("#666666"));
    assert_eq!(t.color(&tdc(1, 0.0)).map(hex).as_deref(), Some("#000000"));
    // ColorId 2 is the first data color; +60% lightens it towards white.
    assert_eq!(t.color(&tdc(2, 0.0)).map(hex).as_deref(), Some("#2F6B4F"));
    assert_eq!(t.color(&tdc(2, 0.6)).map(hex).as_deref(), Some("#ACC4B9"));
    assert_eq!(t.color(&tdc(3, 0.0)).map(hex).as_deref(), Some("#D9822B"));
    assert_eq!(t.color(&tdc(42, 0.0)), None);
}

#[test]
fn header_shapes_fill_dark_green_and_hide_chrome() {
    let mut seen = 0;
    for fx in [overview_report(), stores_report(), online_report()]
        .into_iter()
        .flatten()
    {
        for page in fx.report.pages.iter().filter(|p| p.width >= 1280.0) {
            let header = page
                .visuals
                .iter()
                .find(|v| v.visual_type == "shape" && v.position.z == 0.0)
                .unwrap_or_else(|| panic!("{}: no header shape", page.display_name));
            let props = fx.theme.visual(header);
            assert_eq!(props.visual_type(), "shape");
            assert_eq!(
                props
                    .objects()
                    .color("fill", "fillColor")
                    .map(hex)
                    .as_deref(),
                Some("#1F3A2E"),
                "{}",
                page.display_name
            );
            assert_eq!(props.objects().number("fill", "transparency"), Some(0.0));
            assert_eq!(
                props.objects().text("shape", "tileShape").as_deref(),
                Some("rectangle")
            );
            assert_eq!(props.objects().bool("outline", "show"), Some(false));
            let c = props.container();
            assert_eq!(c.bool("background", "show"), Some(false));
            assert_eq!(c.bool("border", "show"), Some(false));
            assert_eq!(c.bool("visualHeader", "show"), Some(false));
            assert_eq!(c.bool("title", "show"), Some(false));
            seen += 1;
        }
    }
    assert!(seen == 0 || seen == 7, "{seen} header shapes");
}

#[test]
fn card_fonts_and_colors_follow_objects_then_theme() {
    let Some(fx) = overview_report() else { return };
    let page = fx.page(EXECUTIVE_SUMMARY);
    let card = fx.theme.visual(page.visual(KPI_CARDS).expect("card"));

    // Value: size and color from the objects, family from the callout class.
    let value = card.objects().text_style("value", "callout");
    assert_eq!(
        (value.family.as_str(), value.size_pt),
        ("Segoe UI Semibold", 20.0)
    );
    assert_eq!(hex(value.color), "#1F3A2E");
    assert_eq!(
        card.objects().get("value", "fontColor").map(|p| p.source),
        Some(PropSource::Visual)
    );
    assert_eq!(card.objects().get("value", "fontFamily"), None);
    let label = card.objects().text_style("label", "label");
    assert_eq!((label.family.as_str(), label.size_pt), ("Segoe UI", 10.0));
    assert_eq!(
        card.objects().text("label", "position").as_deref(),
        Some("belowValue")
    );
    assert_eq!(card.objects().number("layout", "maxTiles"), Some(5.0));
    assert_eq!(
        card.objects()
            .color("accentBar", "color")
            .map(hex)
            .as_deref(),
        Some("#2F6B4F")
    );
    assert_eq!(card.objects().bool("outline", "show"), Some(false));

    // Container: explicit 1px border with 12px corners, a drop shadow, no
    // title.
    let c = card.container();
    assert_eq!(c.bool("border", "show"), Some(true));
    let width = c.get("border", "width").expect("width");
    assert_eq!(width.source, PropSource::Visual);
    assert_eq!(as_number(&width.value), Some(1.0));
    assert_eq!(c.number("border", "radius"), Some(12.0));
    assert_eq!(
        c.color("border", "color").map(hex).as_deref(),
        Some("#D9D4C7")
    );
    assert_eq!(c.bool("dropShadow", "show"), Some(true));
    assert_eq!(c.bool("title", "show"), Some(false));
}

#[test]
fn theme_wildcard_styles_apply_to_visuals_without_objects() {
    let Some(fx) = overview_report() else { return };
    // `visualStyles["*"]["*"]`: a rounded #D9D4C7 border and a white
    // background on every visual that does not set its own.
    let table = fx.theme.props("tableEx", &Json::Null, &Json::Null);
    let c = table.container();
    let show = c.get("border", "show").expect("border.show");
    assert_eq!(
        (as_bool(&show.value), show.source),
        (Some(true), PropSource::ThemeWildcard)
    );
    assert_eq!(
        c.color("border", "color").map(hex).as_deref(),
        Some("#D9D4C7")
    );
    assert_eq!(c.number("border", "radius"), Some(8.0));
    assert_eq!(c.bool("background", "show"), Some(true));
    // Not in the theme: the built-in defaults (cards have no title).
    let card = fx.theme.props("cardVisual", &Json::Null, &Json::Null);
    let title = card.container().get("title", "show").expect("title.show");
    assert_eq!(
        (as_bool(&title.value), title.source),
        (Some(false), PropSource::Builtin)
    );
}

#[test]
fn chart_titles_come_from_container_objects_and_the_title_class() {
    let Some(fx) = overview_report() else { return };
    let page = fx.page(EXECUTIVE_SUMMARY);
    let donut = fx.theme.visual(page.visual(DONUT).expect("donut"));
    let c = donut.container();
    assert_eq!(c.bool("title", "show"), Some(true));
    assert_eq!(c.text("title", "text").as_deref(), Some("Sales by channel"));
    let style = c.text_style("title", "title");
    assert_eq!(
        (style.family.as_str(), style.size_pt, style.weight),
        ("Segoe UI Semibold", 11.0, 700)
    );
    assert_eq!(hex(style.color), "#1F3A2E");
    let legend = donut.objects().text("legend", "position");
    assert_eq!(legend.as_deref(), Some("Top"));
}

#[test]
fn a_custom_theme_layers_over_the_example_theme() {
    let Some(fx) = overview_report() else { return };
    let custom = json!({
        "name": "Custom",
        "dataColors": ["#C00000"],
        "textClasses": {"title": {"fontFace": "Segoe UI Light"}},
        "visualStyles": {"shape": {"*": {"background": [{"show": false}]}}}
    });
    let base = serde_json::to_value(fx.pbir.custom_theme().expect("theme")).expect("json");
    let r = ThemeResolver::new(Theme::from_values(Some(&base), Some(&custom)));
    assert_eq!(hex(r.data_color(0)), "#C00000");
    assert_eq!(
        hex(r.named_color("tableAccent").expect("accent")),
        "#2F6B4F"
    );
    // The title class keeps the base size and color, with the new face.
    let title = r.text_class("title");
    assert_eq!(
        (title.family.as_str(), title.size_pt),
        ("Segoe UI Light", 12.0)
    );
    assert_eq!(hex(title.color), "#1F3A2E");
    let shape = r.props("shape", &Json::Null, &Json::Null);
    assert_eq!(shape.container().bool("background", "show"), Some(false));
    // Base-theme entries next to the overridden one survive the merge.
    assert_eq!(shape.container().bool("border", "show"), Some(true));
    assert_eq!(
        r.named_color("background").map(hex).as_deref(),
        Some("#FFFFFF")
    );
}
