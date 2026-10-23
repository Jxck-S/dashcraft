//! Slicers, tables and matrices rendered from synthetic DataViews (no
//! fixture files): dropdown / list / tiles / search / ellipsis, table
//! auto-size, wrapping, totals, bands, overflow + scrollbars, alignment,
//! hit ids, and the stepped matrix with subtotals.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use pbi_engine::Value;
use pbi_model::{DataType, FieldExpr, SortDirection, SortItem, Visual};
use pbi_render::dataview::{
    Categorical, CategoryColumn, CellValue, ColumnMeta, DataView, DataViewKind, FieldRef,
    FieldRefKind, Matrix, MatrixColumn, MatrixNode, SlicerData, TableData,
};
use pbi_render::layout::{VisualFrame, VisualHeader};
use pbi_render::query::{InteractionState, VisualQueryError};
use pbi_render::scene::{
    Color, Group, HitKind, Insets, Node, Paint, Rect, Scene, TextAnchor, TextNode,
};
use pbi_render::text::FontBook;
use pbi_render::theme::ThemeResolver;
use pbi_render::visuals::slicer::{self, SELECT_ALL_INDEX, SlicerMode, SlicerOptions};
use pbi_render::visuals::table::{self, GridScroll, TableOptions};
use pbi_render::visuals::{VisualCtx, matrix, render_visual};
use serde_json::{Value as Json, json};

fn lit(raw: &str) -> Json {
    json!({"expr": {"Literal": {"Value": raw}}})
}

fn meta(name: &str, data_type: DataType, is_measure: bool) -> ColumnMeta {
    ColumnMeta {
        roles: vec!["Values".into()],
        query_ref: format!("T.{name}"),
        display_name: name.into(),
        source: FieldRef {
            kind: if is_measure {
                FieldRefKind::Measure
            } else {
                FieldRefKind::Column
            },
            entity: Some("T".into()),
            property: Some(name.into()),
            aggregation: None,
            hierarchy: None,
            source_name: format!("T[{name}]"),
        },
        is_measure,
        format_string: None,
        data_type: Some(data_type),
        expression: None,
        sort: None,
        sort_order: None,
        hidden: false,
    }
}

fn text(s: &str) -> CellValue {
    CellValue::new(Value::string(s), s)
}

fn int(n: i64) -> CellValue {
    CellValue::new(Value::Integer(n), n.to_string())
}

fn visual(name: &str, visual_type: &str, objects: Json) -> Visual {
    Visual {
        name: name.into(),
        visual_type: visual_type.into(),
        objects,
        ..Visual::default()
    }
}

fn frame(v: &Visual, content: Rect) -> VisualFrame {
    VisualFrame {
        name: v.name.clone(),
        visual_type: v.visual_type.clone(),
        is_group: false,
        parent_group: None,
        z: 0.0,
        tab_order: None,
        outer: content,
        inner: content,
        content,
        padding: Insets::ZERO,
        radius: 0.0,
        title: None,
        background: None,
        border: None,
        drop_shadow: None,
        header: VisualHeader::default(),
        hidden: false,
    }
}

/// Everything a `VisualCtx` borrows.
struct Fixture {
    visual: Visual,
    frame: VisualFrame,
    theme: ThemeResolver,
    data: Option<Result<DataView, VisualQueryError>>,
    state: InteractionState,
}

impl Fixture {
    fn new(visual: Visual, content: Rect, dv: DataView) -> Self {
        Self {
            frame: frame(&visual, content),
            visual,
            theme: ThemeResolver::default(),
            data: Some(Ok(dv)),
            state: InteractionState::new(),
        }
    }

    fn ctx(&self) -> VisualCtx<'_> {
        VisualCtx {
            frame: &self.frame,
            visual: &self.visual,
            theme: &self.theme,
            fonts: FontBook::shared(),
            data: self.data.as_ref(),
            state: &self.state,
        }
    }
}

fn walk<'a>(g: &'a Group, out: &mut Vec<&'a Node>) {
    for n in &g.children {
        out.push(n);
        if let Node::Group(child) = n {
            walk(child, out);
        }
    }
}

fn nodes(g: &Group) -> Vec<&Node> {
    let mut out = Vec::new();
    walk(g, &mut out);
    out
}

fn text_nodes(g: &Group) -> Vec<&TextNode> {
    nodes(g)
        .into_iter()
        .filter_map(|n| match n {
            Node::Text(t) => Some(t),
            _ => None,
        })
        .collect()
}

fn texts(g: &Group) -> Vec<String> {
    text_nodes(g).iter().map(|t| t.text.clone()).collect()
}

fn hit_kinds(g: &Group) -> Vec<HitKind> {
    let mut scene = Scene::new(2000.0, 2000.0);
    scene.root.push(g.clone());
    scene.hit_regions().into_iter().map(|h| h.id.kind).collect()
}

fn groups_with_class<'a>(g: &'a Group, class: &str) -> Vec<&'a Group> {
    nodes(g)
        .into_iter()
        .filter_map(|n| match n {
            Node::Group(c) if c.class.as_deref() == Some(class) => Some(c),
            _ => None,
        })
        .collect()
}

fn has_fill(g: &Group, color: Color) -> bool {
    nodes(g).iter().any(|n| match n {
        Node::Rect(r) => r.fill == Some(Paint::solid(color)),
        _ => false,
    })
}

// ---------------------------------------------------------------- slicers

fn slicer_view(values: &[&str], selection: SlicerData) -> DataView {
    DataView {
        kind: DataViewKind::Categorical,
        metadata: vec![meta("Loyalty Tier", DataType::String, false)],
        categorical: Some(Categorical {
            categories: vec![CategoryColumn {
                meta_idx: 0,
                values: values.iter().map(|v| text(v)).collect(),
            }],
            values: Vec::new(),
            series_meta_idx: None,
            hidden: Vec::new(),
        }),
        slicer: Some(selection),
        ..DataView::default()
    }
}

fn all_selected() -> SlicerData {
    SlicerData {
        all_selected: true,
        ..SlicerData::default()
    }
}

fn dropdown_objects() -> Json {
    json!({
        "data": [{"properties": {"mode": lit("'Dropdown'")}}],
        "header": [{"properties": {"bold": lit("true")}}]
    })
}

/// A 216×72 slicer (1px border + 5px padding → 204×60 content).
fn slicer_fixture(objects: Json, dv: DataView) -> Fixture {
    Fixture::new(
        visual("s1", "slicer", objects),
        Rect::new(28.0, 152.0, 204.0, 60.0),
        dv,
    )
}

#[test]
fn dropdown_slicer_shows_bold_header_all_and_chevron() {
    let f = slicer_fixture(
        dropdown_objects(),
        slicer_view(&["Bronze", "Gold"], all_selected()),
    );
    let g = render_visual(&f.ctx()).unwrap();
    assert_eq!(texts(&g), ["Loyalty Tier", "All"]);
    let t = text_nodes(&g);
    assert_eq!(t[0].style.weight, 700);
    assert_eq!(
        (t[0].style.family.as_str(), t[0].style.size_pt),
        ("DIN", 10.0)
    );
    // Header 8px inside the content box, baseline 13px below its top.
    assert_eq!((t[0].x, t[0].y), (36.0, 165.0));
    // "All" in the secondary text color, inside the box.
    assert_eq!(t[1].style.color, Color::rgb(0x60, 0x5E, 0x5C));
    let layout = slicer::layout(&f.ctx(), &SlicerOptions::default());
    assert_eq!(layout.mode, SlicerMode::Dropdown);
    assert_eq!(layout.dropdown, Some(Rect::new(33.0, 175.0, 194.0, 27.0)));
    assert_eq!((t[1].x, t[1].y), (39.0, 194.0));
    // The box toggles the popup; no item hit ids while collapsed.
    assert_eq!(hit_kinds(&g), [HitKind::Button]);
    let chevron = nodes(&g)
        .into_iter()
        .any(|n| matches!(n, Node::Path(p) if p.stroke.is_some() && p.fill.is_none()));
    assert!(chevron);
}

#[test]
fn dropdown_summary_follows_the_selection() {
    let one = SlicerData {
        selected: vec![1],
        values: vec![vec![text("Gold")]],
        ..SlicerData::default()
    };
    let f = slicer_fixture(dropdown_objects(), slicer_view(&["A", "Gold"], one));
    assert_eq!(texts(&render_visual(&f.ctx()).unwrap())[1], "Gold");

    let two = SlicerData {
        selected: vec![0, 1],
        values: vec![vec![text("A")], vec![text("Gold")]],
        ..SlicerData::default()
    };
    let f = slicer_fixture(dropdown_objects(), slicer_view(&["A", "Gold"], two));
    assert_eq!(
        texts(&render_visual(&f.ctx()).unwrap())[1],
        "Multiple selections"
    );

    let inverted = SlicerData {
        selected: vec![0],
        values: vec![vec![text("A")]],
        inverted: true,
        ..SlicerData::default()
    };
    let f = slicer_fixture(dropdown_objects(), slicer_view(&["A", "B"], inverted));
    assert_eq!(
        texts(&render_visual(&f.ctx()).unwrap())[1],
        "Multiple selections"
    );
}

#[test]
fn slicer_header_ellipsizes_below_about_210px() {
    let mut dv = slicer_view(&["Retail"], all_selected());
    dv.metadata[0].display_name = "Customer Segment Group".into();
    let header = |width: f64| {
        let f = Fixture::new(
            visual("s", "slicer", dropdown_objects()),
            Rect::new(0.0, 0.0, width - 12.0, 60.0),
            dv.clone(),
        );
        texts(&render_visual(&f.ctx()).unwrap())[0].clone()
    };
    assert_eq!(header(216.0), "Customer Segment Group");
    assert_eq!(header(212.0), "Customer Segment Group");
    let short = header(205.0);
    assert!(short.ends_with('…'), "{short}");
    assert!(header(150.0).len() < short.len());
}

#[test]
fn list_slicer_draws_checkboxes_select_all_and_item_hits() {
    let objects = json!({
        "data": [{"properties": {"mode": lit("'Basic'")}}],
        "selection": [{"properties": {
            "singleSelect": lit("false"),
            "selectAllCheckboxEnabled": lit("true")
        }}]
    });
    let sel = SlicerData {
        selected: vec![0, 2],
        values: vec![vec![text("A")], vec![text("C")]],
        ..SlicerData::default()
    };
    let f = Fixture::new(
        visual("s", "slicer", objects),
        Rect::new(0.0, 0.0, 200.0, 200.0),
        slicer_view(&["A", "B", "C"], sel),
    );
    let g = render_visual(&f.ctx()).unwrap();
    assert_eq!(texts(&g), ["Loyalty Tier", "Select all", "A", "B", "C"]);
    let boxes = groups_with_class(&g, "pbi-checkbox");
    let checked: Vec<bool> = boxes.iter().map(|b| b.children.len() == 2).collect();
    assert_eq!(checked, [false, true, false, true]);
    assert_eq!(
        hit_kinds(&g),
        [
            HitKind::SlicerItem {
                index: SELECT_ALL_INDEX
            },
            HitKind::SlicerItem { index: 0 },
            HitKind::SlicerItem { index: 1 },
            HitKind::SlicerItem { index: 2 },
        ]
    );
    // Rows are evenly spaced below the header.
    let t = text_nodes(&g);
    let pitch = t[2].y - t[1].y;
    assert!(pitch > 17.0 && (t[3].y - t[2].y - pitch).abs() < 1e-9);
}

#[test]
fn inverted_mode_checks_everything_and_single_select_uses_radios() {
    let objects = json!({
        "data": [{"properties": {"mode": lit("'Basic'"), "isInvertedSelectionMode": lit("true")}}],
        "selection": [{"properties": {"singleSelect": lit("false")}}]
    });
    let f = Fixture::new(
        visual("s", "slicer", objects),
        Rect::new(0.0, 0.0, 200.0, 200.0),
        slicer_view(&["A", "B"], all_selected()),
    );
    let g = render_visual(&f.ctx()).unwrap();
    let boxes = groups_with_class(&g, "pbi-checkbox");
    assert!(boxes.len() == 2 && boxes.iter().all(|b| b.children.len() == 2));

    let objects = json!({"data": [{"properties": {"mode": lit("'Basic'")}}]});
    let sel = SlicerData {
        selected: vec![1],
        values: vec![vec![text("B")]],
        ..SlicerData::default()
    };
    let f = Fixture::new(
        visual("s", "slicer", objects),
        Rect::new(0.0, 0.0, 200.0, 200.0),
        slicer_view(&["A", "B"], sel),
    );
    let g = render_visual(&f.ctx()).unwrap();
    let radios = groups_with_class(&g, "pbi-radio");
    let on: Vec<bool> = radios.iter().map(|r| r.children.len() == 2).collect();
    assert_eq!(on, [false, true]);
    assert!(groups_with_class(&g, "pbi-checkbox").is_empty());
}

#[test]
fn search_box_placeholder_and_filtering() {
    let objects = json!({
        "data": [{"properties": {"mode": lit("'Basic'")}}],
        "general": [{"properties": {"selfFilterEnabled": lit("true")}}],
        "header": [{"properties": {"show": lit("false")}}]
    });
    let f = Fixture::new(
        visual("s", "slicer", objects),
        Rect::new(0.0, 0.0, 200.0, 200.0),
        slicer_view(&["Basecamp", "Juniper", "Timber"], all_selected()),
    );
    let g = render_visual(&f.ctx()).unwrap();
    assert_eq!(texts(&g), ["Search", "Basecamp", "Juniper", "Timber"]);
    let opts = SlicerOptions {
        search: "er".into(),
        ..SlicerOptions::default()
    };
    let g = slicer::render_with(&f.ctx(), &opts).unwrap();
    assert_eq!(texts(&g), ["er", "Juniper", "Timber"]);
    assert_eq!(
        hit_kinds(&g),
        [
            HitKind::SlicerItem { index: 1 },
            HitKind::SlicerItem { index: 2 }
        ]
    );
    let layout = slicer::layout(&f.ctx(), &opts);
    assert!(layout.search.is_some() && layout.header.is_none());
}

#[test]
fn long_lists_scroll_with_a_scrollbar() {
    let values: Vec<String> = (0..40).map(|i| format!("Item {i:02}")).collect();
    let refs: Vec<&str> = values.iter().map(String::as_str).collect();
    let objects = json!({"data": [{"properties": {"mode": lit("'Basic'")}}]});
    let f = Fixture::new(
        visual("s", "slicer", objects),
        Rect::new(0.0, 0.0, 200.0, 150.0),
        slicer_view(&refs, all_selected()),
    );
    let layout = slicer::layout(&f.ctx(), &SlicerOptions::default());
    assert!(layout.max_scroll > 0.0);
    let g = render_visual(&f.ctx()).unwrap();
    assert!(has_fill(&g, Color::rgb(0xAE, 0xAD, 0xAC)));
    let shown = texts(&g);
    assert!(shown.contains(&"Item 00".to_owned()) && !shown.contains(&"Item 39".to_owned()));
    let end = SlicerOptions {
        scroll: 1e9,
        ..SlicerOptions::default()
    };
    let shown = texts(&slicer::render_with(&f.ctx(), &end).unwrap());
    assert!(shown.contains(&"Item 39".to_owned()) && !shown.contains(&"Item 00".to_owned()));
}

#[test]
fn dropdown_popup_lists_items_below_the_box() {
    let mut objects = dropdown_objects();
    objects["selection"] = json!([{"properties": {
        "singleSelect": lit("false"),
        "selectAllCheckboxEnabled": lit("true")
    }}]);
    let f = slicer_fixture(objects, slicer_view(&["Bronze", "Gold"], all_selected()));
    let opts = SlicerOptions {
        dropdown_open: true,
        ..SlicerOptions::default()
    };
    let layout = slicer::layout(&f.ctx(), &opts);
    let (dropdown, popup) = (layout.dropdown.unwrap(), layout.popup.unwrap());
    assert!(popup.y >= dropdown.bottom() && popup.x == dropdown.x);
    let g = slicer::render_dropdown_popup(&f.ctx(), &opts);
    assert_eq!(texts(&g), ["Select all", "Bronze", "Gold"]);
    assert_eq!(hit_kinds(&g).len(), 3);
    assert!(text_nodes(&g).iter().all(|t| t.y > dropdown.bottom()));
    // The open box flips its chevron.
    let closed = render_visual(&f.ctx()).unwrap();
    let open = slicer::render_with(&f.ctx(), &opts).unwrap();
    assert_ne!(closed, open);
}

#[test]
fn tiles_and_ranges() {
    let objects = json!({"general": [{"properties": {"orientation": lit("1D")}}],
        "data": [{"properties": {"mode": lit("'Basic'")}}]});
    let sel = SlicerData {
        selected: vec![0],
        values: vec![vec![text("Camping")]],
        ..SlicerData::default()
    };
    let f = Fixture::new(
        visual("s", "slicer", objects),
        Rect::new(0.0, 0.0, 300.0, 60.0),
        slicer_view(&["Camping", "Hiking"], sel),
    );
    assert_eq!(
        slicer::layout(&f.ctx(), &SlicerOptions::default()).mode,
        SlicerMode::Tile
    );
    let g = render_visual(&f.ctx()).unwrap();
    let tiles: Vec<&TextNode> = text_nodes(&g)
        .into_iter()
        .filter(|t| t.anchor == TextAnchor::Middle)
        .collect();
    assert_eq!(tiles.len(), 2);
    assert_eq!(tiles[0].style.color, Color::WHITE);
    assert_eq!(hit_kinds(&g).len(), 2);

    let mut dv = slicer_view(&[], all_selected());
    dv.metadata[0] = meta("Score", DataType::Int64, false);
    dv.categorical.as_mut().unwrap().categories[0].values = vec![int(5), int(10), int(40)];
    let f = Fixture::new(
        visual("s", "slicer", json!({})),
        Rect::new(0.0, 0.0, 200.0, 80.0),
        dv,
    );
    assert_eq!(
        slicer::layout(&f.ctx(), &SlicerOptions::default()).mode,
        SlicerMode::Range
    );
    assert_eq!(
        texts(&render_visual(&f.ctx()).unwrap()),
        ["Score", "5", "40"]
    );
}

// ----------------------------------------------------------------- tables

fn table_view(columns: Vec<ColumnMeta>, rows: Vec<Vec<CellValue>>) -> DataView {
    DataView {
        kind: DataViewKind::Table,
        table: Some(TableData {
            columns: (0..columns.len()).collect(),
            rows,
            totals: None,
            hidden: Vec::new(),
        }),
        metadata: columns,
        ..DataView::default()
    }
}

fn products_table() -> DataView {
    table_view(
        vec![
            meta("Product Code", DataType::String, false),
            meta("Product Description", DataType::String, false),
            meta("Return Count", DataType::Int64, true),
        ],
        vec![
            vec![text("JNP-HYD"), text("Juniper Hydration"), int(0)],
            vec![text("TRL-TNT"), text("Trail Tent"), int(1)],
            vec![
                text("ALP-BTS"),
                text("Alpine Waterproof Insulated Hiking Boots"),
                int(0),
            ],
        ],
    )
}

fn table_fixture(objects: Json, dv: DataView, content: Rect) -> Fixture {
    Fixture::new(visual("t1", "tableEx", objects), content, dv)
}

fn width(objects: &mut Json, query_ref: &str, w: f64) {
    let entry = json!({"properties": {"value": lit(&format!("{w}D"))},
        "selector": {"metadata": query_ref}});
    match objects.get_mut("columnWidth").and_then(Json::as_array_mut) {
        Some(list) => list.push(entry),
        None => objects["columnWidth"] = json!([entry]),
    }
}

#[test]
fn columns_auto_size_to_content_unless_a_width_is_set() {
    let f = table_fixture(
        json!({}),
        products_table(),
        Rect::new(0.0, 0.0, 1000.0, 400.0),
    );
    let m = table::metrics(&f.ctx(), &TableOptions::default()).unwrap();
    let fonts = FontBook::shared();
    let style = pbi_render::text::TextStyle::default();
    let natural = fonts.text_width("Alpine Waterproof Insulated Hiking Boots", &style);
    assert_eq!(m.column_widths[1], natural + 12.0);
    let header = fonts.text_width("Return Count", &style);
    assert_eq!(m.column_widths[2], header + 12.0);
    // Narrower than the visual: no stretching, no scrollbars.
    assert!(m.content_width < 1000.0);
    assert!(!m.vertical_scrollbar && !m.horizontal_scrollbar);

    let mut objects = json!({});
    width(&mut objects, "T.Product Description", 103.9);
    let f = table_fixture(
        objects,
        products_table(),
        Rect::new(0.0, 0.0, 1000.0, 400.0),
    );
    let m = table::metrics(&f.ctx(), &TableOptions::default()).unwrap();
    assert_eq!(m.column_widths[1], 103.9);
}

#[test]
fn wrapped_cells_make_rows_taller_and_headers_wrap() {
    let mut objects = json!({});
    width(&mut objects, "T.Product Description", 104.0);
    width(&mut objects, "T.Return Count", 70.0);
    let f = table_fixture(
        objects,
        products_table(),
        Rect::new(0.0, 0.0, 1000.0, 400.0),
    );
    let g = render_visual(&f.ctx()).unwrap();
    let all = texts(&g);
    for line in [
        "Juniper",
        "Hydration",
        "Alpine",
        "Waterproof",
        "Hiking Boots",
    ] {
        assert!(all.contains(&line.to_owned()), "{line}: {all:?}");
    }
    // "Product Description" and "Return Count" wrap in their headers.
    assert!(all.contains(&"Product".to_owned()) && all.contains(&"Description".to_owned()));
    let y = |s: &str| text_nodes(&g).iter().find(|t| t.text == s).unwrap().y;
    // Header: 2 lines + sort strip + paddings + outline; rows: 17px lines
    // + 2px padding + 1px grid line.
    let m = table::metrics(&f.ctx(), &TableOptions::default()).unwrap();
    assert_eq!(m.header_height, 1.0 + 34.0 + 8.0 + 1.0 + 1.0);
    let row1 = y("JNP-HYD");
    assert_eq!(row1, m.header_height + 1.0 + 14.0);
    assert_eq!(y("TRL-TNT"), row1 + 36.0 + 1.0);
    assert_eq!(y("ALP-BTS"), row1 + 37.0 + 19.0 + 1.0);
    assert_eq!(y("Hiking Boots") - y("Alpine"), 3.0 * 17.0);
}

#[test]
fn numbers_align_right_and_totals_are_bold() {
    let mut dv = products_table();
    dv.table.as_mut().unwrap().totals = Some(vec![CellValue::blank(), CellValue::blank(), int(1)]);
    let f = table_fixture(json!({}), dv, Rect::new(0.0, 0.0, 1000.0, 400.0));
    let g = render_visual(&f.ctx()).unwrap();
    let m = table::metrics(&f.ctx(), &TableOptions::default()).unwrap();
    let t = text_nodes(&g);
    let zero = t.iter().find(|t| t.text == "0").unwrap();
    assert_eq!(zero.anchor, TextAnchor::End);
    let right = m.column_widths.iter().sum::<f64>() - 6.0;
    assert_eq!(zero.x, right);
    let total = t.iter().find(|t| t.text == "Total").unwrap();
    assert_eq!(total.style.weight, 700);
    let one: Vec<_> = t.iter().filter(|t| t.text == "1").collect();
    assert!(one.iter().any(|t| t.style.weight == 700));
    // Header text stays left-aligned for numeric columns.
    let header = t.iter().find(|t| t.text == "Return Count").unwrap();
    assert_eq!(header.anchor, TextAnchor::Start);
}

#[test]
fn rows_alternate_and_headers_use_column_header_formatting() {
    let objects = json!({"columnHeaders": [{"properties": {
        "backColor": {"solid": {"color": lit("'#A0D1FF'")}},
        "bold": lit("true")
    }}]});
    let f = table_fixture(
        objects,
        products_table(),
        Rect::new(0.0, 0.0, 1000.0, 400.0),
    );
    let g = render_visual(&f.ctx()).unwrap();
    assert!(has_fill(&g, Color::rgb(0xEE, 0xED, 0xED)));
    assert!(has_fill(&g, Color::rgb(0xA0, 0xD1, 0xFF)));
    assert!(has_fill(&g, Color::rgb(0xE5, 0xE5, 0xE5)));
    assert!(has_fill(&g, Color::rgb(0x11, 0x8D, 0xFF)));
    let header = text_nodes(&g)
        .into_iter()
        .find(|t| t.text == "Product Code")
        .unwrap();
    assert_eq!(header.style.weight, 700);

    let none = json!({"stylePreset": [{"properties": {"name": lit("'None'")}}]});
    let f = table_fixture(none, products_table(), Rect::new(0.0, 0.0, 1000.0, 400.0));
    let g = render_visual(&f.ctx()).unwrap();
    assert!(!has_fill(&g, Color::rgb(0xEE, 0xED, 0xED)));
}

#[test]
fn overflow_clips_rows_and_shows_both_scrollbars() {
    let rows: Vec<Vec<CellValue>> = (0..60)
        .map(|i| vec![text(&format!("SKU {i:02}")), text("Name"), int(i)])
        .collect();
    let dv = table_view(products_table().metadata, rows);
    let mut objects = json!({});
    width(&mut objects, "T.Product Code", 300.0);
    width(&mut objects, "T.Product Description", 300.0);
    let f = table_fixture(objects, dv, Rect::new(10.0, 10.0, 400.0, 200.0));
    let m = table::metrics(&f.ctx(), &TableOptions::default()).unwrap();
    assert!(m.vertical_scrollbar && m.horizontal_scrollbar);
    assert_eq!(m.body.width, 400.0 - 9.0);
    assert!(m.body.bottom() <= 210.0 - 9.0 + 1e-9);
    let g = render_visual(&f.ctx()).unwrap();
    let thumbs = nodes(&g)
        .into_iter()
        .filter(|n| matches!(n, Node::Rect(r) if r.fill == Some(Paint::solid(Color::rgb(0xAE, 0xAD, 0xAC)))))
        .count();
    assert_eq!(thumbs, 2);
    // Only the visible rows are drawn, inside a clipped body.
    let shown = texts(&g);
    assert!(shown.contains(&"SKU 00".to_owned()) && !shown.contains(&"SKU 59".to_owned()));
    let (first, last) = m.visible_rows;
    assert!(first == 0 && last < 60);

    let scrolled = TableOptions {
        scroll: GridScroll { x: 50.0, y: 1e9 },
    };
    let ms = table::metrics(&f.ctx(), &scrolled).unwrap();
    assert_eq!(ms.scroll.y, ms.max_scroll_y);
    assert_eq!(ms.scroll.x, 50.0);
    let g = table::render_with(&f.ctx(), &scrolled).unwrap();
    let shown = texts(&g);
    assert!(shown.contains(&"SKU 59".to_owned()) && !shown.contains(&"SKU 00".to_owned()));
    let header = text_nodes(&g)
        .into_iter()
        .find(|t| t.text == "Product Code")
        .unwrap();
    assert_eq!(header.x, 10.0 - 50.0 + 6.0);
}

#[test]
fn sort_arrow_and_cell_hit_ids() {
    let mut dv = products_table();
    dv.metadata[0].sort = Some(SortDirection::Ascending);
    dv.metadata[0].sort_order = Some(0);
    let mut f = table_fixture(json!({}), dv.clone(), Rect::new(0.0, 0.0, 1000.0, 400.0));
    let arrows = |g: &Group| {
        nodes(g)
            .into_iter()
            .filter(|n| matches!(n, Node::Path(p) if p.fill.is_some()))
            .count()
    };
    // Default sort (no sortDefinition): no arrow.
    assert_eq!(arrows(&render_visual(&f.ctx()).unwrap()), 0);
    f.visual.sort = vec![SortItem {
        field: FieldExpr::column("T", "Product Code"),
        direction: SortDirection::Ascending,
    }];
    let g = render_visual(&f.ctx()).unwrap();
    assert_eq!(arrows(&g), 1);
    let hits = hit_kinds(&g);
    assert_eq!(hits.len(), 9);
    assert!(hits.contains(&HitKind::DataPoint {
        series: 2,
        index: 1
    }));
}

#[test]
fn tables_without_data_fall_back_to_the_placeholder() {
    let mut f = table_fixture(
        json!({}),
        DataView::empty(),
        Rect::new(0.0, 0.0, 300.0, 100.0),
    );
    assert_eq!(texts(&render_visual(&f.ctx()).unwrap()), ["tableEx"]);
    f.data = None;
    assert!(table::metrics(&f.ctx(), &TableOptions::default()).is_none());
}

// --------------------------------------------------------------- matrices

fn node(level: usize, value: &str, cells: &[i64], children: Vec<MatrixNode>) -> MatrixNode {
    MatrixNode {
        level: Some(level),
        value: Some(text(value)),
        cells: cells.iter().map(|&n| int(n)).collect(),
        children,
        hidden: Vec::new(),
    }
}

fn matrix_view() -> DataView {
    let rows = MatrixNode {
        level: None,
        value: None,
        cells: vec![int(3), int(2), int(5)],
        children: vec![
            node(
                0,
                "Camping",
                &[2, 1, 3],
                vec![
                    node(1, "Tent", &[2, 0, 2], vec![]),
                    node(1, "Stove", &[0, 1, 1], vec![]),
                ],
            ),
            node(
                0,
                "Hiking",
                &[1, 1, 2],
                vec![node(1, "Boots", &[1, 1, 2], vec![])],
            ),
        ],
        hidden: Vec::new(),
    };
    DataView {
        kind: DataViewKind::Matrix,
        metadata: vec![
            meta("Category", DataType::String, false),
            meta("Product", DataType::String, false),
            meta("Channel", DataType::String, false),
            meta("Orders", DataType::Int64, true),
        ],
        matrix: Some(Matrix {
            row_levels: vec![0, 1],
            column_levels: vec![2],
            values: vec![3],
            columns: vec![
                MatrixColumn {
                    path: vec![text("Online")],
                    is_total: false,
                },
                MatrixColumn {
                    path: vec![text("Retail")],
                    is_total: false,
                },
                MatrixColumn {
                    path: vec![],
                    is_total: true,
                },
            ],
            rows,
            row_subtotals: true,
            column_subtotals: true,
            hidden: Vec::new(),
        }),
        ..DataView::default()
    }
}

fn matrix_fixture(objects: Json) -> Fixture {
    Fixture::new(
        visual("m1", "pivotTable", objects),
        Rect::new(0.0, 0.0, 600.0, 400.0),
        matrix_view(),
    )
}

#[test]
fn matrix_shows_the_first_level_until_expanded() {
    let f = matrix_fixture(json!({}));
    let g = render_visual(&f.ctx()).unwrap();
    let mut shown = texts(&g);
    shown.sort();
    let mut want = [
        "Camping", "2", "1", "3", "Hiking", "1", "1", "2", "Total", "3", "2", "5", "Category",
        "Online", "Retail", "Total",
    ]
    .map(String::from)
    .to_vec();
    want.sort();
    assert_eq!(shown, want);
    // Collapsed parents are regular rows (no subtotal styling).
    let camping = text_nodes(&g)
        .into_iter()
        .find(|t| t.text == "Camping")
        .unwrap();
    assert_eq!(camping.style.weight, 400);
}

#[test]
fn stepped_matrix_indents_levels_with_bold_subtotals_and_grand_total() {
    let f = matrix_fixture(json!({"rowHeaders": [{"properties": {
        "showExpandCollapseButtons": lit("true")
    }}]}));
    let opts = matrix::MatrixOptions {
        expand_depth: Some(usize::MAX),
        ..matrix::MatrixOptions::default()
    };
    let g = matrix::render_with(&f.ctx(), &opts).unwrap();
    let t = text_nodes(&g);
    let find = |s: &str| *t.iter().find(|n| n.text == s).unwrap();
    let (camping, tent, hiking) = (find("Camping"), find("Tent"), find("Hiking"));
    // Children are indented below their parent; parents carry expanders.
    assert_eq!(tent.x - camping.x, 10.0);
    assert!(tent.y > camping.y && hiking.y > tent.y);
    assert_eq!(groups_with_class(&g, "pbi-expander").len(), 2);
    // Parent rows show bold subtotals; leaf rows are regular.
    assert_eq!(camping.style.weight, 700);
    assert_eq!(tent.style.weight, 400);
    let on_row = |row: &TextNode| -> Vec<String> {
        t.iter()
            .filter(|n| n.y == row.y && n.anchor == TextAnchor::End)
            .map(|n| n.text.clone())
            .collect()
    };
    assert_eq!(on_row(camping), ["2", "1", "3"]);
    assert_eq!(on_row(tent), ["2", "0", "2"]);
    let total = t
        .iter()
        .filter(|n| n.text == "Total")
        .max_by(|a, b| a.y.total_cmp(&b.y))
        .unwrap();
    assert_eq!(total.style.weight, 700);
    // Header: the column level plus the grand-total column.
    for h in ["Category", "Online", "Retail"] {
        assert!(t.iter().any(|n| n.text == h), "{h}");
    }
    let refs = matrix::visible_rows(&f.ctx(), &opts);
    assert_eq!(refs.len(), 5);
    assert!(refs[0].subtotal && refs[0].level == Some(0));
    assert_eq!(refs[1].path.len(), 2);
    let hits = hit_kinds(&g);
    assert!(hits.contains(&HitKind::DataPoint {
        series: 3,
        index: 4
    }));

    // Toggling one group expands only that group.
    let one = matrix::MatrixOptions {
        expand_depth: Some(0),
        toggled: vec![vec!["Hiking".into()]],
        ..matrix::MatrixOptions::default()
    };
    let shown = texts(&matrix::render_with(&f.ctx(), &one).unwrap());
    assert!(shown.contains(&"Boots".to_owned()) && !shown.contains(&"Tent".to_owned()));
}

#[test]
fn tabular_matrix_adds_total_rows_per_group() {
    let f = matrix_fixture(json!({"rowHeaders": [{"properties": {"stepped": lit("false")}}]}));
    let opts = matrix::MatrixOptions {
        expand_depth: Some(1),
        ..matrix::MatrixOptions::default()
    };
    let refs = matrix::visible_rows(&f.ctx(), &opts);
    // Camping, Tent, Stove, Total, Hiking, Boots, Total.
    assert_eq!(refs.len(), 7);
    assert!(refs[3].subtotal && refs[3].level.is_none());
    let m = matrix::metrics(&f.ctx(), &opts).unwrap();
    assert_eq!(m.column_widths.len(), 2 + 3);
}

#[test]
fn interactive_page_overlays_open_dropdowns_and_scrolls_tables() {
    use pbi_model::{Page, Position, Report};
    use pbi_render::visuals::{
        PageData, PageRenderOptions, PageViewState, render_interactive_page,
    };
    let placed = |mut v: Visual, x: f64, y: f64, width: f64, height: f64| {
        v.position = Position {
            x,
            y,
            width,
            height,
            ..Position::default()
        };
        v
    };
    let slicer_objects = json!({
        "data": [{"properties": {"mode": lit("'Dropdown'")}}],
        "selection": [{"properties": {"singleSelect": lit("false"), "selectAllCheckboxEnabled": lit("true")}}]
    });
    let rows: Vec<Vec<CellValue>> = (0..60)
        .map(|i| vec![text(&format!("SKU {i:02}")), text("Name"), int(i)])
        .collect();
    let page = Page {
        name: "p".into(),
        width: 1280.0,
        height: 720.0,
        visuals: vec![
            placed(
                visual("s1", "slicer", slicer_objects),
                10.0,
                10.0,
                200.0,
                60.0,
            ),
            placed(
                visual("t1", "tableEx", json!({})),
                300.0,
                10.0,
                400.0,
                200.0,
            ),
        ],
        ..Page::default()
    };
    let data = PageData {
        data: vec![
            (
                "s1".into(),
                Ok(slicer_view(&["Bronze", "Gold"], all_selected())),
            ),
            ("t1".into(), Ok(table_view(products_table().metadata, rows))),
        ],
        ..PageData::default()
    };
    let (report, theme, state) = (
        Report::default(),
        ThemeResolver::default(),
        InteractionState::new(),
    );
    let opts = PageRenderOptions::default();
    let render = |view: &PageViewState| {
        render_interactive_page(&report, &page, &theme, &state, &opts, view, &data)
    };
    let first_row = |scene: &Scene| {
        scene
            .hit_regions()
            .iter()
            .filter(|r| r.id.visual == "t1")
            .filter_map(|r| match r.id.kind {
                HitKind::DataPoint { index, .. } => Some(index),
                _ => None,
            })
            .min()
    };

    // Nothing open: no overlay; the table starts at its first row.
    let closed = render(&PageViewState::new());
    assert!(closed.overlay.is_none());
    let table = closed.interaction("t1").unwrap().scroll.unwrap();
    assert!(table.max_y > 0.0 && table.y == 0.0);
    assert_eq!(first_row(&closed.page.scene), Some(0));
    let dropdown = closed.interaction("s1").unwrap().slicer.clone().unwrap();
    assert_eq!(dropdown.mode, SlicerMode::Dropdown);

    // An open dropdown: the popup items are hit regions of the overlay only.
    let mut view = PageViewState::new();
    view.visual_mut("s1").slicer.dropdown_open = true;
    view.visual_mut("t1").table.scroll.y = 1e9;
    let open = render(&view);
    let overlay = open.overlay.as_ref().unwrap();
    assert_eq!(
        (overlay.width, overlay.height),
        (open.page.scene.width, open.page.scene.height)
    );
    let items: Vec<HitKind> = overlay
        .hit_regions()
        .into_iter()
        .map(|r| r.id.kind)
        .collect();
    assert_eq!(
        items,
        [
            HitKind::SlicerItem {
                index: SELECT_ALL_INDEX
            },
            HitKind::SlicerItem { index: 0 },
            HitKind::SlicerItem { index: 1 }
        ]
    );
    assert!(
        !open
            .page
            .scene
            .hit_regions()
            .iter()
            .any(|r| matches!(r.id.kind, HitKind::SlicerItem { .. }))
    );
    // The scrolled table is clamped to its range and starts further down.
    let scrolled = open.interaction("t1").unwrap().scroll.unwrap();
    assert_eq!(scrolled.y, scrolled.max_y);
    assert!(first_row(&open.page.scene).unwrap() > 0);
    assert!(view.close_dropdowns() && !view.close_dropdowns());
}

#[test]
fn visual_cache_rerenders_only_changed_visuals_and_matches_uncached_output() {
    use pbi_model::{Page, Position, Report};
    use pbi_render::query::{PointField, PointSelection, SlicerSelection};
    use pbi_render::svg::{SvgOptions, to_svg};
    use pbi_render::theme::Theme;
    use pbi_render::visuals::{
        InteractivePage, PageData, PageRenderOptions, PageViewState, VisualCache,
        render_interactive_page_cached,
    };
    let placed = |mut v: Visual, x: f64, y: f64, width: f64, height: f64| {
        v.position = Position {
            x,
            y,
            width,
            height,
            ..Position::default()
        };
        v
    };
    let page = Page {
        name: "p".into(),
        width: 1280.0,
        height: 720.0,
        visuals: vec![
            placed(visual("s1", "slicer", json!({})), 10.0, 10.0, 200.0, 200.0),
            placed(
                visual("t1", "tableEx", json!({})),
                300.0,
                10.0,
                400.0,
                200.0,
            ),
            placed(
                visual(
                    "x1",
                    "textbox",
                    json!({"general": [{"properties": {"paragraphs": [
                        {"textRuns": [{"value": "Directions", "textStyle": {"fontFamily": "Calibri"}}]}
                    ]}}]}),
                ),
                10.0,
                300.0,
                300.0,
                40.0,
            ),
            placed(
                visual("m1", "pivotTable", json!({})),
                400.0,
                300.0,
                300.0,
                60.0,
            ),
        ],
        ..Page::default()
    };
    let rows = |renamed: Option<&str>| -> Vec<Vec<CellValue>> {
        (0..60)
            .map(|i| {
                let name = match (i, renamed) {
                    (0, Some(r)) => r.to_owned(),
                    _ => format!("SKU {i:02}"),
                };
                vec![text(&name), text("Name"), int(i)]
            })
            .collect()
    };
    let data_with = |rows: Vec<Vec<CellValue>>| PageData {
        data: vec![
            (
                "s1".into(),
                Ok(slicer_view(&["Bronze", "Gold"], all_selected())),
            ),
            ("t1".into(), Ok(table_view(products_table().metadata, rows))),
            ("m1".into(), Ok(matrix_view())),
        ],
        ..PageData::default()
    };
    let report = Report::default();
    let opts = PageRenderOptions::default();
    let cache = VisualCache::new();
    let svgs = |p: &InteractivePage| {
        (
            to_svg(&p.page.scene, &SvgOptions::default()),
            p.overlay
                .as_ref()
                .map(|o| to_svg(o, &SvgOptions::default())),
        )
    };
    // Renders through the cache and without it: identical bytes and
    // geometry. Returns the visuals that were reused (not re-rendered).
    let render = |theme: &ThemeResolver,
                  state: &InteractionState,
                  view: &PageViewState,
                  data: &PageData|
     -> Vec<String> {
        let cached = render_interactive_page_cached(
            &report,
            &page,
            theme,
            state,
            &opts,
            view,
            data,
            Some(&cache),
        );
        let fresh =
            render_interactive_page_cached(&report, &page, theme, state, &opts, view, data, None);
        assert!(fresh.reused.is_empty());
        assert_eq!(svgs(&cached), svgs(&fresh));
        assert_eq!(cached.interactions, fresh.interactions);
        cached.reused
    };
    let theme = ThemeResolver::default();
    let state = InteractionState::new();
    let data = data_with(rows(None));
    let mut view = PageViewState::new();
    let all = ["s1", "t1", "x1", "m1"];
    let but = |skip: &str| -> Vec<&str> { all.into_iter().filter(|n| *n != skip).collect() };

    assert!(render(&theme, &state, &view, &data).is_empty(), "cold");
    assert_eq!(render(&theme, &state, &view, &data), all, "warm");

    // Scrolling the table re-renders the table only (drawn from its
    // prepared grid); scrolling back is a hit (keys are content hashes, not
    // revisions).
    let grids = cache.stats();
    view.visual_mut("t1").table.scroll.y = 40.0;
    assert_eq!(render(&theme, &state, &view, &data), but("t1"));
    view.visual_mut("t1").table.scroll.y = 80.0;
    assert_eq!(render(&theme, &state, &view, &data), but("t1"));
    assert_eq!(cache.stats().grid_hits, grids.grid_hits + 2);
    view.visual_mut("t1").table.scroll.y = 0.0;
    assert_eq!(render(&theme, &state, &view, &data), all);

    // The matrix: scrolling and expanding re-render it only.
    view.visual_mut("m1").matrix.scroll.y = 20.0;
    assert_eq!(render(&theme, &state, &view, &data), but("m1"));
    view.visual_mut("m1").matrix.expand_depth = Some(1);
    assert_eq!(render(&theme, &state, &view, &data), but("m1"));
    view.visual_mut("m1").matrix.toggled = vec![vec!["Camping".into()]];
    assert_eq!(render(&theme, &state, &view, &data), but("m1"));
    view.visual_mut("m1").matrix = Default::default();

    // New data for the table (one cell): the table only.
    let renamed = data_with(rows(Some("Renamed")));
    assert_eq!(render(&theme, &state, &view, &renamed), but("t1"));

    // Query-result identity: the data hash is remembered per id, and a
    // result replaced in place under the same id is still detected.
    let mut identified = data_with(rows(None));
    identified.id = PageData::next_id();
    assert_eq!(render(&theme, &state, &view, &identified), all);
    assert_eq!(render(&theme, &state, &view, &identified), all);
    identified.data[1].1 = Ok(table_view(
        products_table().metadata,
        rows(Some("Replaced")),
    ));
    assert_eq!(render(&theme, &state, &view, &identified), but("t1"));

    // The slicer's own selection and its search box: the slicer only.
    let mut selected = InteractionState::new();
    selected.select("s1", SlicerSelection::values(["Gold"]));
    assert_eq!(render(&theme, &selected, &view, &data), but("s1"));
    view.visual_mut("s1").slicer.search = "On".into();
    assert_eq!(render(&theme, &state, &view, &data), but("s1"));
    view.visual_mut("s1").slicer.search.clear();

    // A cross-filter selection changes the source's styling only (the
    // other visuals see it through their re-queried data).
    let mut crossed = InteractionState::new();
    crossed.selected_points = Some(PointSelection {
        source: "t1".into(),
        points: vec![vec![PointField {
            field: FieldExpr::column("T", "Product Code"),
            value: Value::string("SKU 01"),
        }]],
    });
    assert_eq!(render(&theme, &crossed, &view, &data), but("t1"));

    // Another theme re-renders everything.
    let red = ThemeResolver::new(Theme::from_values(
        Some(&json!({"name": "Red", "dataColors": ["#FF0000"]})),
        None,
    ));
    assert!(render(&red, &state, &view, &data).is_empty());
    assert_eq!(render(&red, &state, &view, &data), all);

    let stats = cache.stats();
    assert!(stats.hits >= 10 && stats.misses >= 10, "{stats:?}");
    assert_eq!(stats.uncacheable, 0);
}

#[test]
fn memoized_grid_layout_is_identical_and_never_stale() {
    let rect = Rect::new(0.0, 0.0, 240.0, 60.0);
    let f = table_fixture(json!({}), products_table(), rect);
    let first = table::render_with(&f.ctx(), &TableOptions::default()).unwrap();
    let again = table::render_with(&f.ctx(), &TableOptions::default()).unwrap();
    assert_eq!(
        first, again,
        "a cached layout renders exactly like a fresh one"
    );

    // Scrolling reuses the layout: same column widths, other visible rows.
    let m0 = table::metrics(&f.ctx(), &TableOptions::default()).unwrap();
    let scrolled = TableOptions {
        scroll: GridScroll { x: 0.0, y: 30.0 },
    };
    let m1 = table::metrics(&f.ctx(), &scrolled).unwrap();
    assert_eq!(m0.column_widths, m1.column_widths);
    assert_eq!(m0.rows_height, m1.rows_height);
    assert!(m1.scroll.y > 0.0);

    // Different text → different layout (the memo is keyed by content).
    let mut longer = products_table();
    if let Some(t) = longer.table.as_mut() {
        t.rows[0][1] = text("A much longer product description that has to wrap onto more lines");
    }
    let g = table_fixture(json!({}), longer, rect);
    let m2 = table::metrics(&g.ctx(), &TableOptions::default()).unwrap();
    // The capped column keeps its width; the longer text wraps onto more lines.
    assert!(
        m2.rows_height > m0.rows_height,
        "{} vs {}",
        m2.rows_height,
        m0.rows_height
    );

    // A different width → a different layout too.
    let h = table_fixture(
        json!({}),
        products_table(),
        Rect::new(0.0, 0.0, 120.0, 60.0),
    );
    let m3 = table::metrics(&h.ctx(), &TableOptions::default()).unwrap();
    assert!(m3.rows_height >= m0.rows_height);
    assert_eq!(m3.viewport.width, 120.0);
}
