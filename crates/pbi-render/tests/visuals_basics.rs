//! Basic visuals on synthetic visuals and DataViews (no fixture files):
//! textbox paragraphs, lists and alignment; shapes; buttons and their
//! actions; cards (single, multi, blank, error, label clipping); the
//! placeholder; page orchestration; SVG output.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod query_support;

use pbi_engine::Value;
use pbi_model::{Page, Position, Report, Visual};
use pbi_render::dataview::{
    CellValue, ColumnMeta, DataView, DataViewKind, FieldRef, FieldRefKind, TableData,
};
use pbi_render::layout::{FrameOptions, PageLayout, render_page_with};
use pbi_render::query::{InteractionState, VisualQueryError};
use pbi_render::scene::{
    Color, Group, HitId, HitKind, Node, PathNode, Rect, Scene, TextAnchor, TextNode,
};
use pbi_render::svg::{SvgOptions, to_svg};
use pbi_render::text::{BundledFace, FontBook};
use pbi_render::theme::ThemeResolver;
use pbi_render::visuals::button::{self, ButtonAction};
use pbi_render::visuals::{
    PageRenderOptions, VisualCtx, render_report_page, render_visual, textbox,
};
use query_support::{env, measure, projection};
use serde_json::{Value as Json, json};

fn lit(raw: &str) -> Json {
    json!({"expr": {"Literal": {"Value": raw}}})
}

fn color(hex: &str) -> Json {
    json!({"solid": {"color": lit(&format!("'{hex}'"))}})
}

fn visual(name: &str, visual_type: &str, rect: (f64, f64, f64, f64)) -> Visual {
    Visual {
        name: name.into(),
        visual_type: visual_type.into(),
        position: Position {
            x: rect.0,
            y: rect.1,
            width: rect.2,
            height: rect.3,
            ..Position::default()
        },
        ..Visual::default()
    }
}

fn page_of(visuals: Vec<Visual>) -> Page {
    Page {
        name: "p".into(),
        width: 1280.0,
        height: 720.0,
        visuals,
        ..Page::default()
    }
}

/// Renders one visual's content with an optional query result.
fn render_one(v: &Visual, data: Option<&Result<DataView, VisualQueryError>>) -> (Group, Rect) {
    let page = page_of(vec![v.clone()]);
    let theme = ThemeResolver::default();
    let layout = PageLayout::compute(&page, None);
    let frame = layout.frame(&v.name).unwrap();
    let state = InteractionState::default();
    let ctx = VisualCtx {
        frame,
        visual: v,
        theme: &theme,
        fonts: FontBook::shared(),
        data,
        state: &state,
    };
    (render_visual(&ctx).unwrap_or_default(), frame.content)
}

/// A scene with the visuals' frames and contents.
fn scene_of(visuals: Vec<Visual>, data: &[(&str, DataView)]) -> Scene {
    let page = page_of(visuals);
    let report = Report::default();
    let theme = ThemeResolver::default();
    let state = InteractionState::default();
    let results: Vec<(String, Result<DataView, VisualQueryError>)> = data
        .iter()
        .map(|(n, d)| ((*n).to_owned(), Ok(d.clone())))
        .collect();
    render_page_with(
        &report,
        &page,
        &theme,
        &FrameOptions::default(),
        |frame, visual| {
            let ctx = VisualCtx {
                frame,
                visual,
                theme: &theme,
                fonts: FontBook::shared(),
                data: results
                    .iter()
                    .find(|(n, _)| *n == visual.name)
                    .map(|(_, r)| r),
                state: &state,
            };
            render_visual(&ctx)
        },
    )
}

/// Text nodes with their absolute x (group transforms applied to x only:
/// translate + horizontal scale).
fn texts(g: &Group) -> Vec<(f64, TextNode)> {
    fn walk(g: &Group, a: f64, e: f64, out: &mut Vec<(f64, TextNode)>) {
        let (a, e) = match g.transform {
            Some(t) => (a * t.a, a * t.e + e),
            None => (a, e),
        };
        for c in &g.children {
            match c {
                Node::Text(t) => out.push((a * t.x + e, t.clone())),
                Node::Group(g) => walk(g, a, e, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(g, 1.0, 0.0, &mut out);
    out
}

fn text_of<'a>(all: &'a [(f64, TextNode)], s: &str) -> &'a (f64, TextNode) {
    all.iter().find(|(_, t)| t.text == s).unwrap_or_else(|| {
        panic!(
            "no text {s:?} in {:?}",
            all.iter().map(|t| &t.1.text).collect::<Vec<_>>()
        )
    })
}

/// Texts that are not clipped away by an ancestor group clip (groups with
/// clips here are untransformed).
fn visible_texts(g: &Group) -> Vec<String> {
    fn walk(g: &Group, clip: Option<Rect>, out: &mut Vec<String>) {
        let clip = match (g.clip.map(|c| c.rect), clip) {
            (Some(a), Some(b)) => Some(a.intersect(&b).unwrap_or(Rect::new(a.x, a.y, 0.0, 0.0))),
            (a, b) => a.or(b),
        };
        for c in &g.children {
            match c {
                Node::Text(t) => {
                    let top = t.y - t.style.size_px() * 0.7;
                    if clip.is_none_or(|c| top < c.bottom() && t.y > c.y) {
                        out.push(t.text.clone());
                    }
                }
                Node::Group(g) => walk(g, clip, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(g, None, &mut out);
    out
}

fn paths(g: &Group) -> Vec<PathNode> {
    let mut out = Vec::new();
    for c in &g.children {
        match c {
            Node::Path(p) => out.push(p.clone()),
            Node::Group(g) => out.extend(paths(g)),
            _ => {}
        }
    }
    out
}

fn find_group<'a>(g: &'a Group, id: &HitId) -> Option<&'a Group> {
    if g.id.as_ref() == Some(id) {
        return Some(g);
    }
    g.children.iter().find_map(|c| match c {
        Node::Group(g) => find_group(g, id),
        _ => None,
    })
}

fn meta(name: &str, role: &str, is_measure: bool) -> ColumnMeta {
    ColumnMeta {
        roles: vec![role.into()],
        query_ref: format!("Orders.{name}"),
        display_name: name.into(),
        source: FieldRef {
            kind: if is_measure {
                FieldRefKind::Measure
            } else {
                FieldRefKind::Column
            },
            entity: Some("Orders".into()),
            property: Some(name.into()),
            aggregation: None,
            hierarchy: None,
            source_name: format!("Orders[{name}]"),
        },
        is_measure,
        format_string: None,
        data_type: None,
        expression: None,
        sort: None,
        sort_order: None,
        hidden: false,
    }
}

fn single(name: &str, cell: CellValue) -> DataView {
    DataView {
        kind: DataViewKind::Single,
        metadata: vec![meta(name, "Data", true)],
        single: Some(cell),
        ..DataView::default()
    }
}

fn int(n: i64) -> CellValue {
    CellValue::new(Value::Integer(n), n.to_string())
}

// ---------------------------------------------------------------- textbox

fn textbox(name: &str, rect: (f64, f64, f64, f64), paragraphs: Json) -> Visual {
    let mut v = visual(name, "textbox", rect);
    v.objects = json!({"general": [{"properties": {"paragraphs": paragraphs}}]});
    v.container_objects = json!({"border": [{"properties": {"show": lit("true")}}]});
    v
}

#[test]
fn textbox_spacer_paragraph_and_centered_bold_title() {
    let v = textbox(
        "title",
        (40.0, 10.0, 452.0, 116.0),
        json!([
            {"textRuns": [{"value": " ", "textStyle": {"fontSize": "22pt"}}],
             "horizontalTextAlignment": "center"},
            {"textRuns": [{"value": "Acme Outfitters Sales Dashboard",
                           "textStyle": {"fontWeight": "bold", "fontFamily": "Calibri", "fontSize": "12pt"}}],
             "horizontalTextAlignment": "center"}
        ]),
    );
    let (g, content) = render_one(&v, None);
    let all = texts(&g);
    assert_eq!(all.len(), 1, "the spacer draws nothing");
    let (x, t) = text_of(&all, "Acme Outfitters Sales Dashboard");
    assert_eq!(t.style.weight, 700);
    assert_eq!(t.style.size_pt, 12.0, "Calibri is drawn at its size");
    assert_eq!(t.style.family, "Calibri");
    assert_eq!(t.style.face(), BundledFace::CarlitoBold);
    // As Power BI draws it: the baseline 60px below the top of a 116px
    // textbox at y=10 (22pt Segoe UI spacer line, then 12pt Calibri).
    assert!((t.y - 70.2).abs() < 0.5, "baseline {}", t.y);
    // Centred: the run starts half its width left of the centre.
    let width = pbi_render::visuals::common::text_width(FontBook::shared(), &t.text, &t.style);
    let center = content.x + content.width / 2.0;
    assert!(
        (x + width / 2.0 - center).abs() < 0.5,
        "{x} {width} {center}"
    );
}

#[test]
fn textbox_bullets_wrap_with_hanging_indent_and_scroll() {
    let run = |s: &str| json!({"textRuns": [{"value": s, "textStyle": {"fontFamily": "Calibri", "fontSize": "9pt"}}], "listType": "bullet"});
    let v = textbox(
        "bullets",
        (520.0, 60.0, 200.0, 84.0),
        json!([
            run("Pick a region or a year in the slicers on the left"),
            run("The cards and charts update with every selection you make"),
            run("Clear all slicers to see every store."),
        ]),
    );
    let (g, content) = render_one(&v, None);
    let all = texts(&g);
    let bullets: Vec<&(f64, TextNode)> = all.iter().filter(|(_, t)| t.text == "•").collect();
    assert_eq!(bullets.len(), 3);
    assert!(bullets.iter().all(|(_, t)| t.anchor == TextAnchor::End));
    // Text starts 1.5em (18px for 12px text) right of the content box,
    // the marker ends 0.3em before it.
    let lines: Vec<&(f64, TextNode)> = all.iter().filter(|(_, t)| t.text != "•").collect();
    assert!(lines.len() > 3, "long items wrap: {lines:?}");
    for (x, _) in &lines {
        assert!((x - (content.x + 18.0)).abs() < 1e-6, "hanging indent {x}");
    }
    assert!((bullets[0].0 - (content.x + 18.0 - 3.6)).abs() < 1e-6);
    // 9pt Calibri lines on the 12px Segoe UI strut: 16.17px pitch, first
    // baseline 12.95px below the top.
    assert!((lines[0].1.y - (content.y + 12.95)).abs() < 0.01);
    assert!((lines[1].1.y - lines[0].1.y - 16.17).abs() < 0.01);
    // Overflowing text shows the scrollbar thumb at the right edge.
    let thumb = g.children.iter().find_map(|c| match c {
        Node::Rect(r) if r.fill.is_some_and(|f| f.color == textbox::SCROLLBAR_COLOR) => {
            Some(r.rect)
        }
        _ => None,
    });
    let thumb = thumb.expect("scrollbar");
    assert!((thumb.right() - content.right()).abs() < 1e-6);
    assert_eq!(thumb.width, textbox::SCROLLBAR_WIDTH);
}

#[test]
fn textbox_ordered_lists_alignment_and_run_styles() {
    let v = textbox(
        "styled",
        (0.0, 0.0, 400.0, 200.0),
        json!([
            {"textRuns": [{"value": "first"}], "listType": "number"},
            {"textRuns": [{"value": "second"}], "listType": "number"},
            {"textRuns": [
                {"value": "red ", "textStyle": {"color": "#ff0000", "fontStyle": "italic"}},
                {"value": "link", "url": "https://example.com"},
                {"value": " gone", "textStyle": {"textDecoration": "line-through"}}
             ], "horizontalTextAlignment": "right"},
            {"textRuns": "garbage"},
            42,
            {"textRuns": [{"value": 7}, {"novalue": true}]}
        ]),
    );
    let (g, content) = render_one(&v, None);
    let all = texts(&g);
    text_of(&all, "1.");
    text_of(&all, "2.");
    let red = &text_of(&all, "red").1;
    assert_eq!(red.style.color, Color::rgb(255, 0, 0));
    assert!(red.style.italic);
    let link = &text_of(&all, "link").1;
    assert!(link.style.underline);
    assert_eq!(link.style.color, Color::rgb(0x00, 0x78, 0xD4));
    text_of(&all, "7");
    // Right-aligned: the last run ends at the content box's right edge.
    let (gx, gone) = text_of(&all, " gone");
    let w = pbi_render::visuals::common::text_width(FontBook::shared(), &gone.text, &gone.style);
    assert!((gx + w - content.right()).abs() < 0.5);
    // Strikethrough is a line under the run.
    assert!(g.children.iter().any(|c| matches!(c, Node::Line(_))));
}

#[test]
fn textbox_without_paragraphs_is_empty() {
    let mut v = visual("t", "textbox", (0.0, 0.0, 100.0, 50.0));
    v.objects = json!({"general": [{"properties": {"paragraphs": {"not": "a list"}}}]});
    assert!(render_one(&v, None).0.is_empty());
    // Huge text is cut to the measurement limit before layout.
    let long = "word ".repeat(10_000);
    let huge = textbox(
        "huge",
        (0.0, 0.0, 100.0, 50.0),
        json!([{"textRuns": [{"value": long}]}, {"textRuns": [{"value": long}]}]),
    );
    let paras = textbox::paragraphs(&ThemeResolver::default(), &huge);
    let chars: usize = paras
        .iter()
        .flat_map(|p| &p.runs)
        .map(|r| r.text.chars().count())
        .sum();
    assert_eq!(chars, pbi_render::text::MAX_TEXT_CHARS);
}

// ----------------------------------------------------------------- shapes

#[test]
fn header_shapes_fill_the_whole_container() {
    let mut v = visual("header", "shape", (0.0, 0.0, 1280.0, 64.0));
    v.objects = json!({
        "shape": [{"properties": {"tileShape": lit("'rectangle'")}}],
        "fill": [{"properties": {"fillColor": color("#2F6B4F"), "transparency": lit("0D")},
                  "selector": {"id": "default"}}],
        "outline": [{"properties": {"show": lit("false")}}]
    });
    v.container_objects = json!({"background": [{"properties": {"show": lit("false")}}]});
    let (g, _) = render_one(&v, None);
    let p = paths(&g);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].fill.unwrap().color, Color::rgb(0x2F, 0x6B, 0x4F));
    assert!(p[0].stroke.is_none());
    assert_eq!(p[0].d.bounds().unwrap(), Rect::new(0.0, 0.0, 1280.0, 64.0));
    let svg = to_svg(&scene_of(vec![v], &[]), &SvgOptions::default());
    assert!(svg.contains(r##"fill="#2F6B4F""##), "{svg}");
}

#[test]
fn shapes_outline_rotation_line_and_text() {
    let mut oval = visual("oval", "shape", (100.0, 100.0, 80.0, 40.0));
    oval.objects = json!({
        "shape": [{"properties": {"tileShape": lit("'oval'")}}],
        "fill": [{"properties": {"show": lit("false")}}],
        "outline": [{"properties": {"lineColor": color("#8E4A6B"), "weight": lit("4D")}}],
        "rotation": [{"properties": {"shapeAngle": lit("90L")}}],
        "text": [{"properties": {"show": lit("true"), "text": lit("'Hi'")}}]
    });
    let (g, _) = render_one(&oval, None);
    let p = paths(&g);
    assert_eq!(p.len(), 1);
    assert!(p[0].fill.is_none());
    let s = p[0].stroke.as_ref().unwrap();
    assert_eq!((s.color, s.width), (Color::rgb(0x8E, 0x4A, 0x6B), 4.0));
    // Stroke inset by half its width.
    assert_eq!(
        p[0].d.bounds().unwrap(),
        Rect::new(102.0, 102.0, 76.0, 36.0)
    );
    let rotated = g
        .children
        .iter()
        .any(|c| matches!(c, Node::Group(g) if g.transform.is_some()));
    assert!(rotated);
    text_of(&texts(&g), "Hi");

    let mut line = visual("line", "basicShape", (0.0, 0.0, 100.0, 20.0));
    line.objects = json!({"general": [{"properties": {"shapeType": lit("'line'")}}],
                          "line": [{"properties": {"lineColor": color("#123456"), "weight": lit("3D")}}]});
    let (g, _) = render_one(&line, None);
    let has_line = |g: &Group| {
        fn any(g: &Group) -> bool {
            g.children.iter().any(|c| match c {
                Node::Line(l) => l.stroke.color == Color::rgb(0x12, 0x34, 0x56) && l.y1 == 10.0,
                Node::Group(g) => any(g),
                _ => false,
            })
        }
        any(g)
    };
    assert!(has_line(&g));

    let mut unknown = visual("u", "shape", (0.0, 0.0, 50.0, 50.0));
    unknown.objects = json!({"shape": [{"properties": {"tileShape": lit("'cloud'")}}],
                             "fill": [{"properties": {"transparency": lit("'abc'")}}]});
    let (g, _) = render_one(&unknown, None);
    assert_eq!(paths(&g).len(), 1, "unknown shapes draw a rectangle");
}

// ---------------------------------------------------------------- buttons

fn back_button() -> Visual {
    let mut v = visual("btn", "actionButton", (22.0, 60.0, 40.0, 40.0));
    v.objects = json!({"icon": [{"properties": {"shapeType": lit("'back'")}, "selector": {"id": "default"}}]});
    v.container_objects =
        json!({"visualLink": [{"properties": {"show": lit("true"), "type": lit("'Back'")}}]});
    v
}

#[test]
fn back_button_draws_a_circled_arrow_with_a_button_hit_region() {
    let v = back_button();
    let theme = ThemeResolver::default();
    assert_eq!(button::action(&v, &theme), Some(ButtonAction::Back));
    let (g, content) = render_one(&v, None);
    assert_eq!(g.id, Some(HitId::button("btn")));
    let p = paths(&g);
    assert_eq!(p.len(), 2, "circle + arrow");
    for path in &p {
        let s = path.stroke.as_ref().unwrap();
        assert_eq!(s.color, Color::rgb(0x60, 0x5E, 0x5C));
        assert!((s.width - 3.0).abs() < 1e-9);
    }
    // A 30px circle (stroke included) centred in the 40px button.
    let circle = p[0].d.bounds().unwrap();
    assert!((circle.width - 27.0).abs() < 1e-6);
    assert_eq!(circle.center(), content.center());

    let scene = scene_of(vec![v], &[]);
    let hits = scene.hit_regions();
    let hit = hits
        .iter()
        .rev()
        .find(|h| h.id.kind == HitKind::Button)
        .unwrap();
    assert_eq!(hit.rect, Rect::new(22.0, 60.0, 40.0, 40.0));
    let svg = to_svg(&scene, &SvgOptions::default());
    assert!(svg.contains(r#"data-hit="btn|button""#), "{svg}");
}

#[test]
fn button_actions_text_and_disabled_links() {
    let theme = ThemeResolver::default();
    let link = |props: Json| {
        let mut v = visual("b", "actionButton", (0.0, 0.0, 120.0, 32.0));
        v.container_objects = json!({"visualLink": [{"properties": props}]});
        v
    };
    let bm = link(json!({"type": lit("'Bookmark'"), "bookmark": lit("'Bookmark1'")}));
    assert_eq!(
        button::action(&bm, &theme),
        Some(ButtonAction::Bookmark {
            bookmark: "Bookmark1".into()
        })
    );
    let nav = link(
        json!({"type": lit("'PageNavigation'"), "navigationSection": lit("'abc'"),
                          "tooltip": lit("'Go'")}),
    );
    assert_eq!(
        button::action(&nav, &theme),
        Some(ButtonAction::PageNavigation { page: "abc".into() })
    );
    assert_eq!(button::tooltip(&nav, &theme).as_deref(), Some("Go"));
    let url = link(json!({"type": lit("'WebUrl'"), "webUrl": lit("'https://x'")}));
    assert_eq!(
        button::action(&url, &theme),
        Some(ButtonAction::WebUrl {
            url: "https://x".into()
        })
    );
    let off = link(json!({"show": lit("false"), "type": lit("'Back'")}));
    assert_eq!(button::action(&off, &theme), None);
    assert_eq!(
        button::action(&visual("x", "actionButton", (0.0, 0.0, 1.0, 1.0)), &theme),
        None
    );

    let mut text = nav.clone();
    text.objects = json!({
        "text": [{"properties": {"show": lit("true"), "text": lit("'Details'")}}],
        "fill": [{"properties": {"show": lit("true"), "fillColor": color("#EEEEEE")}}],
        "icon": [{"properties": {"shapeType": lit("'rightArrow'")}}]
    });
    let (g, _) = render_one(&text, None);
    text_of(&texts(&g), "Details");
    let p = paths(&g);
    assert!(p.iter().any(|p| {
        p.fill
            .is_some_and(|f| f.color == Color::rgb(0xEE, 0xEE, 0xEE))
    }));
    assert!(p.iter().any(|p| p.stroke.is_some()), "icon");
}

// ------------------------------------------------------------------ cards

fn card_visual(name: &str, height: f64) -> Visual {
    let mut v = visual(name, "cardVisual", (24.0, 152.0, 153.07, height));
    v.objects = json!({
        "value": [{"properties": {"fontSize": lit("24D"), "fontFamily": lit("'Calibri'")}, "selector": {"id": "default"}}],
        "label": [{"properties": {"fontSize": lit("10D")}, "selector": {"id": "default"}}],
        "padding": [{"properties": {"paddingUniform": lit("6L")}, "selector": {"id": "default"}}],
        "outline": [{"properties": {"show": lit("false")}, "selector": {"id": "default"}}],
        "border": [{"properties": {"show": lit("false")}, "selector": {"id": "default"}}]
    });
    v.container_objects = json!({
        "border": [{"properties": {"show": lit("true")}}],
        "padding": [{"properties": {"top": lit("0D"), "bottom": lit("0D"), "left": lit("0D"), "right": lit("0D")}}]
    });
    v
}

#[test]
fn card_value_and_label_match_the_power_bi_layout() {
    let v = card_visual("card", 100.0);
    let data = Ok(single("Order Count", int(2024)));
    let (g, _) = render_one(&v, Some(&data));
    let all = texts(&g);
    let (vx, value) = text_of(&all, "2024");
    let (lx, label) = text_of(&all, "Order Count");
    // As Power BI lays out a 153×100 card: value baseline 51px and label
    // baseline 76px below the top, text 18px inside the 1px container
    // border.
    assert!((value.y - 203.07).abs() < 0.5, "{}", value.y);
    assert!((label.y - 228.05).abs() < 0.5, "{}", label.y);
    assert!((vx - 43.0).abs() < 1e-6 && (lx - 43.0).abs() < 1e-6);
    assert_eq!(label.style.color, Color::rgb(0x60, 0x5E, 0x5C));
    assert_eq!(value.style.size_pt, 24.0);
    assert_eq!(value.style.face(), BundledFace::CarlitoRegular);
    // Inner tile border off: only the container border is drawn.
    assert!(paths(&g).is_empty());
    assert!(
        !g.children
            .iter()
            .any(|c| matches!(c, Node::Rect(r) if r.stroke.is_some()))
    );
}

#[test]
fn short_cards_clip_the_label_and_default_cards_draw_a_tile_border() {
    let data = Ok(single("Order Count", int(2024)));
    let (g, _) = render_one(&card_visual("tall", 100.0), Some(&data));
    assert_eq!(visible_texts(&g), ["2024", "Order Count"]);
    let (g, _) = render_one(&card_visual("short", 80.0), Some(&data));
    assert_eq!(visible_texts(&g), ["2024"], "label clipped below ~100px");

    let mut plain = visual("plain", "cardVisual", (0.0, 0.0, 200.0, 120.0));
    plain.objects =
        json!({"border": [{"properties": {"show": lit("true"), "color": color("#000000")}}]});
    let (g, _) = render_one(&plain, Some(&data));
    let tile = g.children.iter().find_map(|c| match c {
        Node::Rect(r) if r.stroke.is_some() => Some(r.clone()),
        _ => None,
    });
    let tile = tile.expect("inner tile border");
    assert_eq!(tile.stroke.unwrap().color, Color::BLACK);
    // Narrower than the container: inset by the layout padding.
    assert!(tile.rect.x > 5.0 && tile.rect.right() < 195.0);
}

#[test]
fn cards_with_several_values_blank_and_errors() {
    let blank = Ok(single("Avg Order Value", CellValue::blank()));
    let (g, _) = render_one(&card_visual("b", 100.0), Some(&blank));
    text_of(&texts(&g), "(Blank)");

    let table = DataView {
        kind: DataViewKind::Table,
        metadata: vec![
            meta("Online Orders", "Data", true),
            meta("Store Orders", "Data", true),
        ],
        table: Some(TableData {
            columns: vec![0, 1],
            rows: vec![vec![int(678), CellValue::blank()]],
            totals: None,
            hidden: Vec::new(),
        }),
        ..DataView::default()
    };
    let mut wide = card_visual("multi", 100.0);
    wide.position.width = 400.0;
    let (g, _) = render_one(&wide, Some(&Ok(table)));
    let all = texts(&g);
    let x678 = text_of(&all, "678").0;
    let xb = text_of(&all, "(Blank)").0;
    assert!(xb > x678 + 150.0, "tiles side by side: {x678} {xb}");
    text_of(&all, "Store Orders");

    // A failing measure renders a warning, never a panic; the page info
    // carries the message and DAX for the tooltip.
    let env = env();
    let mut broken = card_visual("broken", 100.0);
    broken.projections = vec![projection("Data", measure("Broken"))];
    let mut hidden = card_visual("hidden", 100.0);
    hidden.is_hidden = true;
    hidden.projections = vec![projection("Data", measure("Store Count"))];
    let mut ok = card_visual("ok", 100.0);
    ok.position.x = 500.0;
    ok.projections = vec![projection("Data", measure("Store Count"))];
    let page = page_of(vec![broken, hidden, ok, back_button()]);
    let report = Report {
        pages: vec![page.clone()],
        ..Report::default()
    };
    let rendered = render_report_page(
        &env,
        &report,
        &page,
        &ThemeResolver::default(),
        &InteractionState::default(),
        &PageRenderOptions::default(),
    );
    let info = rendered.visual("broken").unwrap();
    assert!(info.error.as_deref().unwrap().starts_with("⚠ Broken"));
    assert!(info.tooltip.as_deref().unwrap().contains("NOSUCHFUNCTION"));
    assert!(rendered.visual("hidden").unwrap().hidden);
    assert!(rendered.data.iter().all(|(n, _)| n != "hidden"));
    assert_eq!(
        rendered.visual("btn").unwrap().action,
        Some(ButtonAction::Back)
    );
    let broken = find_group(&rendered.scene.root, &HitId::visual("broken")).unwrap();
    let all = texts(broken);
    text_of(&all, "Can't display this visual");
    assert!(!paths(broken).is_empty(), "warning icon");
    let ok = find_group(&rendered.scene.root, &HitId::visual("ok")).unwrap();
    text_of(&texts(ok), "5");
    assert!(find_group(&rendered.scene.root, &HitId::visual("hidden")).is_none());
}

#[test]
fn legacy_card_multi_row_card_and_kpi() {
    let data = Ok(single("Sessions", int(678)));
    let (g, _) = render_one(
        &visual("legacy", "card", (0.0, 0.0, 200.0, 120.0)),
        Some(&data),
    );
    let all = texts(&g);
    assert_eq!(text_of(&all, "678").1.anchor, TextAnchor::Middle);
    text_of(&all, "Sessions");

    let table = DataView {
        kind: DataViewKind::Table,
        metadata: vec![
            meta("Store", "Values", false),
            meta("Orders", "Values", true),
        ],
        table: Some(TableData {
            columns: vec![0, 1],
            rows: vec![
                vec![CellValue::new(Value::string("A"), "A"), int(3)],
                vec![CellValue::new(Value::string("B"), "B"), int(4)],
            ],
            totals: None,
            hidden: Vec::new(),
        }),
        ..DataView::default()
    };
    let (g, _) = render_one(
        &visual("mrc", "multiRowCard", (0.0, 0.0, 300.0, 300.0)),
        Some(&Ok(table)),
    );
    let all = texts(&g);
    for t in ["A", "B", "3", "4", "Orders"] {
        text_of(&all, t);
    }

    let mut m_ind = meta("Value", "Indicator", true);
    m_ind.roles = vec!["Indicator".into()];
    let kpi = DataView {
        kind: DataViewKind::Categorical,
        metadata: vec![
            meta("Month", "TrendLine", false),
            m_ind,
            meta("Target", "Goal", true),
        ],
        categorical: Some(pbi_render::dataview::Categorical {
            categories: vec![pbi_render::dataview::CategoryColumn {
                meta_idx: 0,
                values: vec![int(1), int(2), int(3)],
            }],
            values: vec![
                pbi_render::dataview::ValueColumn {
                    meta_idx: 1,
                    series: None,
                    values: vec![int(5), int(8), int(12)],
                    highlights: None,
                },
                pbi_render::dataview::ValueColumn {
                    meta_idx: 2,
                    series: None,
                    values: vec![int(10), int(10), int(10)],
                    highlights: None,
                },
            ],
            series_meta_idx: None,
            hidden: Vec::new(),
        }),
        ..DataView::default()
    };
    let (g, _) = render_one(
        &visual("kpi", "kpi", (0.0, 0.0, 200.0, 120.0)),
        Some(&Ok(kpi)),
    );
    let all = texts(&g);
    assert_eq!(
        text_of(&all, "12").1.style.color,
        Color::rgb(0x1A, 0xAB, 0x40)
    );
    text_of(&all, "Goal: 10 (+20.00%)");
    assert!(!paths(&g).is_empty(), "trend area");
}

// ------------------------------------------------------------ placeholder

#[test]
fn placeholder_lists_type_and_bound_fields() {
    let mut v = visual("x", "decompositionTreeVisual", (0.0, 0.0, 300.0, 120.0));
    v.projections = vec![
        projection("Analyze", measure("Store Count")),
        projection("ExplainBy", measure("Order Count")),
    ];
    v.projections[1].display_name = Some("Orders".into());
    let (g, _) = render_one(&v, Some(&Ok(DataView::empty())));
    let all: Vec<String> = texts(&g).into_iter().map(|t| t.1.text).collect();
    assert_eq!(
        all,
        [
            "decompositionTreeVisual",
            "Analyze: Store Count",
            "ExplainBy: Orders"
        ]
    );
    assert_eq!(g.class.as_deref(), Some("pbi-placeholder"));
}

#[test]
fn svg_of_basic_visuals_is_deterministic() {
    let visuals = vec![
        textbox(
            "t",
            (10.0, 10.0, 200.0, 40.0),
            json!([{"textRuns": [{"value": "Directions", "textStyle": {"fontWeight": "bold", "fontFamily": "Calibri"}}]}]),
        ),
        back_button(),
        card_visual("c", 100.0),
    ];
    let data = [("c", single("Order Count", int(2024)))];
    let a = to_svg(&scene_of(visuals.clone(), &data), &SvgOptions::default());
    let b = to_svg(&scene_of(visuals, &data), &SvgOptions::default());
    assert_eq!(a, b);
    assert!(a.contains(">Directions</text>"), "{a}");
    assert!(a.contains(">2024</text>"));
    assert!(a.contains(">Order Count</text>"));
    assert!(a.contains(r#"font-weight="700""#));
    assert!(
        a.contains(r#"font-family="Carlito, Calibri, sans-serif""#),
        "Calibri is drawn with Carlito"
    );
    assert!(!a.contains(" scale("), "no condensed text");
}
