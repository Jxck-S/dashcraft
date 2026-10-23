//! Charts on synthetic DataViews: nice ticks, pie/donut angles and labels,
//! bar clustering/stacking/100%, negative values, robustness (empty,
//! blank, NaN, a single category, 10k categories), legend positions, the
//! category scroll indicator, colors and deterministic SVG.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use pbi_engine::Value;
use pbi_model::Visual;
use pbi_render::dataview::{
    Categorical, CategoryColumn, CellValue, ColumnMeta, DataView, DataViewKind, FieldRef,
    FieldRefKind, ValueColumn,
};
use pbi_render::layout::{VisualFrame, VisualHeader};
use pbi_render::query::InteractionState;
use pbi_render::scene::{Color, Group, HitKind, Insets, Node, Paint, PathSeg, Rect, Scene};
use pbi_render::svg::{SvgOptions, to_svg};
use pbi_render::text::FontBook;
use pbi_render::theme::ThemeResolver;
use pbi_render::visuals::VisualCtx;
use pbi_render::visuals::charts::{self, ChartData, legend::LegendPosition, pie, scale};
use serde_json::{Value as Json, json};

fn meta(name: &str, roles: &[&str], measure: bool) -> ColumnMeta {
    ColumnMeta {
        roles: roles.iter().map(|r| (*r).to_owned()).collect(),
        query_ref: format!("T.{name}"),
        display_name: name.to_owned(),
        source: FieldRef {
            kind: if measure {
                FieldRefKind::Measure
            } else {
                FieldRefKind::Column
            },
            entity: Some("T".to_owned()),
            property: Some(name.to_owned()),
            aggregation: None,
            hierarchy: None,
            source_name: format!("T[{name}]"),
        },
        is_measure: measure,
        format_string: measure.then(|| "0".to_owned()),
        data_type: None,
        expression: None,
        sort: None,
        sort_order: None,
        hidden: false,
    }
}

fn num(v: Option<f64>) -> CellValue {
    match v {
        Some(x) if x.is_finite() => CellValue::new(Value::Double(x), format!("{x}")),
        Some(x) => CellValue::new(Value::Double(x), "NaN"),
        None => CellValue::blank(),
    }
}

fn cat_cell(s: &str) -> CellValue {
    if s.is_empty() {
        CellValue::new(Value::Blank, "(Blank)")
    } else {
        CellValue::new(Value::String(s.into()), s)
    }
}

/// A categorical DataView: one category column, one value column per
/// measure (`Y` role, or `Y2` when the name starts with `2:`).
fn dataview(categories: &[&str], measures: &[(&str, Vec<Option<f64>>)]) -> DataView {
    let mut metadata = vec![meta("Cat", &["Category"], false)];
    let mut values = Vec::new();
    for (name, vals) in measures {
        let (name, role) = match name.strip_prefix("2:") {
            Some(n) => (n, "Y2"),
            None => (*name, "Y"),
        };
        metadata.push(meta(name, &[role], true));
        values.push(ValueColumn {
            meta_idx: metadata.len() - 1,
            series: None,
            values: vals.iter().map(|v| num(*v)).collect(),
            highlights: None,
        });
    }
    DataView {
        kind: DataViewKind::Categorical,
        metadata,
        categorical: Some(Categorical {
            categories: vec![CategoryColumn {
                meta_idx: 0,
                values: categories.iter().map(|c| cat_cell(c)).collect(),
            }],
            values,
            series_meta_idx: None,
            hidden: Vec::new(),
        }),
        ..DataView::default()
    }
}

/// A DataView with a series (legend) field: one value column per series.
fn series_dataview(categories: &[&str], series: &[(&str, Vec<Option<f64>>)]) -> DataView {
    let metadata = vec![
        meta("Cat", &["Category"], false),
        meta("Ser", &["Series"], false),
        meta("M", &["Y"], true),
    ];
    let values = series
        .iter()
        .map(|(s, vals)| ValueColumn {
            meta_idx: 2,
            series: Some(cat_cell(s)),
            values: vals.iter().map(|v| num(*v)).collect(),
            highlights: None,
        })
        .collect();
    DataView {
        kind: DataViewKind::Categorical,
        metadata,
        categorical: Some(Categorical {
            categories: vec![CategoryColumn {
                meta_idx: 0,
                values: categories.iter().map(|c| cat_cell(c)).collect(),
            }],
            values,
            series_meta_idx: Some(1),
            hidden: Vec::new(),
        }),
        ..DataView::default()
    }
}

fn frame(w: f64, h: f64) -> VisualFrame {
    let r = Rect::new(0.0, 0.0, w, h);
    VisualFrame {
        name: "v1".into(),
        visual_type: String::new(),
        is_group: false,
        parent_group: None,
        z: 0.0,
        tab_order: None,
        outer: r,
        inner: r,
        content: r,
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

fn lit(raw: &str) -> Json {
    json!({"expr": {"Literal": {"Value": raw}}})
}

/// `{"<object>": [{"properties": {k: literal}}]}`.
fn objects(entries: &[(&str, &str, &str)]) -> Json {
    let mut out = serde_json::Map::new();
    for (object, prop, raw) in entries {
        let slot = out
            .entry((*object).to_owned())
            .or_insert_with(|| json!([{"properties": {}}]));
        slot[0]["properties"][*prop] = lit(raw);
    }
    Json::Object(out)
}

fn render_with(visual_type: &str, dv: &DataView, objs: Json, w: f64, h: f64) -> Group {
    let mut f = frame(w, h);
    f.visual_type = visual_type.into();
    let visual = Visual {
        name: "v1".into(),
        visual_type: visual_type.into(),
        objects: objs,
        ..Visual::default()
    };
    let theme = ThemeResolver::default();
    let data = Ok(dv.clone());
    let state = InteractionState::new();
    let ctx = VisualCtx {
        frame: &f,
        visual: &visual,
        theme: &theme,
        fonts: FontBook::shared(),
        data: Some(&data),
        state: &state,
    };
    charts::render(&ctx).unwrap_or_default()
}

fn render(visual_type: &str, dv: &DataView) -> Group {
    render_with(visual_type, dv, Json::Null, 400.0, 250.0)
}

/// Data-point groups: `(series, index, first child)`.
fn points(g: &Group) -> Vec<(usize, usize, Node)> {
    let mut out = Vec::new();
    walk(g, &mut |g| {
        if let Some(HitKind::DataPoint { series, index }) = g.id.as_ref().map(|i| &i.kind)
            && let Some(child) = g.children.first()
        {
            out.push((*series, *index, child.clone()));
        }
    });
    out
}

fn walk(g: &Group, f: &mut dyn FnMut(&Group)) {
    f(g);
    for c in &g.children {
        if let Node::Group(child) = c {
            walk(child, f);
        }
    }
}

fn texts(g: &Group) -> Vec<String> {
    let mut out = Vec::new();
    walk(g, &mut |g| {
        for c in &g.children {
            if let Node::Text(t) = c {
                out.push(t.text.clone());
            }
        }
    });
    out
}

fn bar_rects(g: &Group) -> Vec<(usize, usize, Rect, Color)> {
    points(g)
        .into_iter()
        .filter_map(|(s, i, n)| match n {
            Node::Rect(r) => Some((s, i, r.rect, r.fill.map_or(Color::BLACK, |p| p.color))),
            _ => None,
        })
        .collect()
}

fn class_group<'a>(g: &'a Group, class: &str) -> Option<&'a Group> {
    if g.class.as_deref() == Some(class) {
        return Some(g);
    }
    g.children.iter().find_map(|c| match c {
        Node::Group(child) => class_group(child, class),
        _ => None,
    })
}

fn svg_of(g: Group) -> String {
    let mut scene = Scene::new(400.0, 250.0);
    scene.root.push(g);
    to_svg(&scene, &SvgOptions::default())
}

#[test]
fn nice_ticks_match_the_service() {
    assert_eq!(scale::ticks(0.0, 7.0, 3), vec![0.0, 2.0, 4.0, 6.0]);
    assert_eq!(scale::ticks(0.0, 10.0, 3), vec![0.0, 5.0, 10.0]);
    assert_eq!(scale::ticks(0.0, 49.0, 3), vec![0.0, 20.0, 40.0]);
    assert_eq!(scale::ticks(0.0, 10.0, 4), vec![0.0, 2.5, 5.0, 7.5, 10.0]);
    assert_eq!(
        scale::ticks(0.0, 1.0, 5),
        vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0]
    );
    assert_eq!(
        scale::ticks(-30.0, 80.0, 5),
        vec![-20.0, 0.0, 20.0, 40.0, 60.0, 80.0]
    );
    assert_eq!(scale::nice_step(1_234_567.0, 5), 200_000.0);
    assert!(scale::ticks(f64::NAN, 1.0, 3).is_empty());
    assert_eq!(scale::ticks(3.0, 3.0, 3), vec![3.0]);
    assert!(scale::ticks(0.0, 1e300, 1_000_000).len() <= scale::MAX_TICKS);
    assert_eq!(scale::tick_count(470.0, true), 3);
    assert_eq!(scale::value_domain([5.0, 7.0], true), (0.0, 7.0));
    assert_eq!(
        scale::value_domain([-3.0, f64::NAN, 7.0], true),
        (-3.0, 7.0)
    );
    assert_eq!(scale::value_domain([], true), (0.0, 1.0));
    use charts::format::{DisplayUnit, NumberStyle, tick_label};
    let plain = NumberStyle::default();
    assert_eq!(
        tick_label(2_500_000.0, 500_000.0, 2_500_000.0, &plain, None),
        "2.5M"
    );
    assert_eq!(tick_label(1_000.0, 500.0, 1_500.0, &plain, None), "1.0K");
    assert_eq!(tick_label(6.0, 2.0, 6.0, &plain, None), "6");
    let pct = NumberStyle::from_format(Some("0.0%"));
    assert_eq!(tick_label(0.25, 0.25, 1.0, &pct, None), "25%");
    let none = Some(DisplayUnit::NONE);
    assert_eq!(
        tick_label(12_000.0, 4_000.0, 12_000.0, &plain, none),
        "12,000"
    );
}

#[test]
fn pie_slices_start_at_twelve_and_sum_to_360() {
    let angles = pie::slice_angles(&[300.0, 100.0]);
    assert_eq!(angles[0].0, 0.0);
    assert!((angles[0].1 - 270.0).abs() < 0.01, "{angles:?}");
    assert!((angles[1].1 - 360.0).abs() < 1e-9);
    let odd = pie::slice_angles(&[1.0, f64::NAN, -2.0, 0.0, 3.0]);
    assert_eq!(odd[1].0, odd[1].1);
    assert!((odd[4].1 - 360.0).abs() < 1e-9);
    let sum: f64 = pie::slice_angles(&[0.1, 0.2, 0.3, 0.4])
        .iter()
        .map(|(a, b)| b - a)
        .sum();
    assert!((sum - 360.0).abs() < 1e-9);

    let dv = dataview(
        &["Retail", "Online"],
        &[("Count", vec![Some(300.0), Some(100.0)])],
    );
    let g = render("pieChart", &dv);
    let pts = points(&g);
    assert_eq!(
        pts.iter().map(|(s, i, _)| (*s, *i)).collect::<Vec<_>>(),
        vec![(0, 0), (0, 1)]
    );
    // First slice starts at 12 o'clock: its outer arc begins straight above
    // the centre.
    let Node::Path(p) = &pts[0].2 else {
        panic!("slice is a path")
    };
    let (cx, cy) = match p.d.segs[0] {
        PathSeg::MoveTo(x, y) => (x, y),
        _ => panic!("{:?}", p.d.segs),
    };
    let PathSeg::LineTo(x1, y1) = p.d.segs[1] else {
        panic!("{:?}", p.d.segs)
    };
    assert!((x1 - cx).abs() < 1e-6 && y1 < cy);
    assert_eq!(p.fill.map(|f| f.color), Some(Color::rgb(0x11, 0x8D, 0xFF)));
    let t = texts(&g);
    assert!(t.iter().any(|s| s.contains("300")), "{t:?}");
    assert!(t.iter().any(|s| s.contains("75.00%")), "{t:?}");
    assert!(t.iter().any(|s| s.contains("100")), "{t:?}");
    // The legend (Top by default) lists the categories under the field name.
    assert!(t.contains(&"Cat".to_owned()) && t.contains(&"Online".to_owned()));
}

#[test]
fn pie_labels_follow_label_style_and_truncate() {
    let dv = dataview(&["A", "B"], &[("M", vec![Some(3.0), Some(1.0)])]);
    let objs = objects(&[
        ("labels", "labelStyle", "'Category'"),
        ("legend", "show", "false"),
    ]);
    let t = texts(&render_with("pieChart", &dv, objs, 400.0, 250.0));
    assert_eq!(t, vec!["A", "B"]);
    let objs = objects(&[
        ("labels", "labelStyle", "'Percent of total'"),
        ("legend", "show", "false"),
    ]);
    let t = texts(&render_with("pieChart", &dv, objs, 400.0, 250.0));
    assert_eq!(t, vec!["75.00%", "25.00%"]);
    let objs = objects(&[("labels", "show", "false"), ("legend", "show", "false")]);
    assert!(texts(&render_with("pieChart", &dv, objs, 400.0, 250.0)).is_empty());
    // A narrow pie truncates inside the parentheses: `1 (25...)`.
    let objs = objects(&[("legend", "show", "false")]);
    let t = texts(&render_with("pieChart", &dv, objs, 150.0, 220.0));
    assert!(
        t.iter().any(|s| s.ends_with("...)") || s.ends_with("...")),
        "{t:?}"
    );
}

#[test]
fn donut_has_a_hole_and_per_category_colors() {
    let dv = dataview(
        &["Pending", "Shipped", "Other"],
        &[("M", vec![Some(5.0), Some(3.0), Some(2.0)])],
    );
    let objs = json!({"dataPoint": [{
        "properties": {"fill": {"solid": {"color": lit("'#30A440'")}}},
        "selector": {"data": [{"scopeId": {"Comparison": {
            "ComparisonKind": 0,
            "Left": {"Column": {"Expression": {"SourceRef": {"Entity": "T"}}, "Property": "Cat"}},
            "Right": {"Literal": {"Value": "'Pending'"}}
        }}}]}
    }]});
    let g = render_with("donutChart", &dv, objs, 400.0, 250.0);
    let pts = points(&g);
    assert_eq!(pts.len(), 3);
    let colors: Vec<Color> = pts
        .iter()
        .filter_map(|(_, _, n)| match n {
            Node::Path(p) => p.fill.map(|f| f.color),
            _ => None,
        })
        .collect();
    // The pinned slice is green; the others start the palette at 0.
    assert_eq!(
        colors,
        vec![
            Color::rgb(0x30, 0xA4, 0x40),
            Color::rgb(0x11, 0x8D, 0xFF),
            Color::rgb(0x12, 0x23, 0x9E)
        ]
    );
    // Ring segments: outer arc, line in, inner arc back (no centre point).
    let Node::Path(p) = &pts[0].2 else { panic!() };
    let arcs =
        p.d.segs
            .iter()
            .filter(|s| matches!(s, PathSeg::ArcTo { .. }))
            .count();
    assert_eq!(arcs, 2, "{:?}", p.d.segs);
}

#[test]
fn clustered_stacked_and_percent_bars() {
    let dv = dataview(
        &["A", "B"],
        &[
            ("M1", vec![Some(4.0), Some(2.0)]),
            ("M2", vec![Some(1.0), Some(2.0)]),
        ],
    );
    // Clustered: two thinner bars per band, side by side, from the same base.
    let bars = bar_rects(&render("clusteredBarChart", &dv));
    assert_eq!(bars.len(), 4);
    let a1 = bars.iter().find(|b| (b.0, b.1) == (0, 0)).unwrap().2;
    let a2 = bars.iter().find(|b| (b.0, b.1) == (1, 0)).unwrap().2;
    assert!((a1.x - a2.x).abs() < 1e-6 && (a1.bottom() - a2.y).abs() < 1e-6);
    assert!((a1.width / a2.width - 4.0).abs() < 1e-6);
    assert_ne!(bars[0].3, bars[1].3, "series get palette colors");
    // Stacked: segments continue each other; A (5) is the longest stack.
    let bars = bar_rects(&render("barChart", &dv));
    let a1 = bars.iter().find(|b| (b.0, b.1) == (0, 0)).unwrap().2;
    let a2 = bars.iter().find(|b| (b.0, b.1) == (1, 0)).unwrap().2;
    assert!((a1.right() - a2.x).abs() < 1e-6 && (a1.y - a2.y).abs() < 1e-6);
    // 100%: every stack has the same length.
    let bars = bar_rects(&render("hundredPercentStackedColumnChart", &dv));
    let total = |i: usize| -> f64 { bars.iter().filter(|b| b.1 == i).map(|b| b.2.height).sum() };
    assert!((total(0) - total(1)).abs() < 1e-6 && total(0) > 10.0);
    let t = texts(&render("hundredPercentStackedColumnChart", &dv));
    assert!(t.contains(&"100%".to_owned()), "{t:?}");
}

#[test]
fn negative_values_extend_below_the_axis() {
    let dv = dataview(&["Up", "Down"], &[("M", vec![Some(10.0), Some(-5.0)])]);
    let cols = bar_rects(&render("clusteredColumnChart", &dv));
    let up = cols.iter().find(|b| b.1 == 0).unwrap().2;
    let down = cols.iter().find(|b| b.1 == 1).unwrap().2;
    // Both start at the zero line: the positive one ends there, the
    // negative one begins there.
    assert!((up.bottom() - down.y).abs() < 1e-6, "{up:?} {down:?}");
    assert!((up.height / down.height - 2.0).abs() < 1e-6);
    let bars = bar_rects(&render("clusteredBarChart", &dv));
    let up = bars.iter().find(|b| b.1 == 0).unwrap().2;
    let down = bars.iter().find(|b| b.1 == 1).unwrap().2;
    assert!((up.x - down.right()).abs() < 1e-6, "{up:?} {down:?}");
    // Pies size slices by magnitude.
    let pts = points(&render("pieChart", &dv));
    assert_eq!(pts.len(), 2);
}

#[test]
fn robust_against_empty_blank_nan_and_huge_inputs() {
    let kinds = [
        "pieChart",
        "donutChart",
        "clusteredBarChart",
        "barChart",
        "hundredPercentStackedBarChart",
        "clusteredColumnChart",
        "columnChart",
        "hundredPercentStackedColumnChart",
        "lineChart",
        "areaChart",
        "stackedAreaChart",
        "lineClusteredColumnComboChart",
        "lineStackedColumnComboChart",
    ];
    let many: Vec<String> = (0..10_000).map(|i| format!("Category {i}")).collect();
    let many_refs: Vec<&str> = many.iter().map(String::as_str).collect();
    let many_vals: Vec<Option<f64>> = (0..10_000).map(|i| Some(f64::from(10_000 - i))).collect();
    let cases = vec![
        DataView::default(),
        dataview(&[], &[("M", vec![])]),
        dataview(&["", "B"], &[("M", vec![None, None])]),
        dataview(&["A"], &[("M", vec![Some(f64::NAN)])]),
        dataview(
            &["A", "B"],
            &[("M", vec![Some(f64::INFINITY), Some(f64::NEG_INFINITY)])],
        ),
        dataview(&["Only"], &[("M", vec![Some(0.0)])]),
        dataview(
            &["Only"],
            &[("M", vec![Some(42.0)]), ("2:L", vec![Some(-1.0)])],
        ),
        dataview(&["A", "B"], &[("M", vec![Some(-1.0), Some(-2.0)])]),
        dataview(
            &["", "B", "C"],
            &[("M", vec![Some(0.0), Some(1e300), Some(-1e300)])],
        ),
        dataview(&many_refs, &[("M", many_vals.clone()), ("2:L", many_vals)]),
    ];
    for dv in &cases {
        for k in kinds {
            for (w, h) in [(400.0, 250.0), (30.0, 20.0), (3.0, 3.0)] {
                let g = render_with(k, dv, Json::Null, w, h);
                let svg = svg_of(g);
                assert!(!svg.contains("NaN") && !svg.contains("inf"), "{k}");
            }
        }
    }
    // Numeric categories with extreme values on a continuous axis.
    let mut extreme = dataview(
        &["a", "b", "c"],
        &[("M", vec![Some(1.0), Some(2.0), Some(3.0)])],
    );
    if let Some(cat) = extreme.categorical.as_mut() {
        cat.categories[0].values = [1e300, -1e300, 5.0]
            .iter()
            .map(|v| CellValue::new(Value::Double(*v), "x"))
            .collect();
    }
    for dt in [pbi_model::DataType::Double, pbi_model::DataType::DateTime] {
        extreme.metadata[0].data_type = Some(dt);
        for k in ["lineChart", "areaChart", "stackedAreaChart"] {
            let svg = svg_of(render(k, &extreme));
            assert!(!svg.contains("NaN"), "{k}");
        }
    }
    // Blank categories show as (Blank).
    let dv = dataview(&["", "B"], &[("M", vec![Some(2.0), Some(1.0)])]);
    assert!(texts(&render("clusteredBarChart", &dv)).contains(&"(Blank)".to_owned()));
    let data = ChartData::from_dataview(&dv);
    assert_eq!(data.categories, vec!["(Blank)", "B"]);
}

#[test]
fn overflowing_categories_scroll() {
    let names: Vec<String> = (0..50).map(|i| format!("App{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let vals: Vec<Option<f64>> = (0..50).map(|i| Some(f64::from(50 - i))).collect();
    let dv = dataview(&refs, &[("M", vals)]);
    let g = render_with("clusteredBarChart", &dv, Json::Null, 600.0, 176.0);
    let bars = bar_rects(&g);
    assert!(bars.len() >= 3 && bars.len() < 10, "{}", bars.len());
    // Only the first categories, top to bottom, longest first.
    assert!(bars.iter().enumerate().all(|(k, b)| b.1 == k));
    assert!(
        bars.windows(2)
            .all(|w| w[0].2.width > w[1].2.width && w[0].2.y < w[1].2.y)
    );
    let sb = class_group(&g, "pbi-scrollbar").expect("scrollbar");
    let Node::Rect(track) = &sb.children[0] else {
        panic!()
    };
    assert!(track.rect.height > track.rect.width, "vertical track");
    // Few categories: no scrollbar.
    let few = dataview(&["A", "B"], &[("M", vec![Some(2.0), Some(1.0)])]);
    assert!(class_group(&render("clusteredBarChart", &few), "pbi-scrollbar").is_none());
    // Columns scroll horizontally.
    let g = render_with("clusteredColumnChart", &dv, Json::Null, 300.0, 200.0);
    assert!(bar_rects(&g).len() < 50);
    let sb = class_group(&g, "pbi-scrollbar").expect("column scrollbar");
    let Node::Rect(track) = &sb.children[0] else {
        panic!()
    };
    assert!(track.rect.width > track.rect.height, "horizontal track");
}

#[test]
fn legend_positions_shrink_the_plot() {
    let dv = series_dataview(
        &["A", "B"],
        &[
            ("North", vec![Some(1.0), Some(2.0)]),
            ("South", vec![Some(3.0), Some(1.0)]),
        ],
    );
    let legend_rect = |pos: &str| {
        let objs = objects(&[("legend", "position", &format!("'{pos}'"))]);
        let g = render_with("clusteredColumnChart", &dv, objs, 400.0, 250.0);
        let lg = class_group(&g, "pbi-legend").expect("legend");
        let t = texts(lg);
        assert_eq!(t, vec!["Ser", "North", "South"], "{pos}");
        let bars = bar_rects(&g);
        let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for b in &bars {
            x0 = x0.min(b.2.x);
            x1 = x1.max(b.2.right());
            y0 = y0.min(b.2.y);
            y1 = y1.max(b.2.bottom());
        }
        let mut marker_x = f64::MAX;
        let mut marker_y = f64::MAX;
        walk(lg, &mut |g| {
            for c in &g.children {
                if let Node::Path(p) = c
                    && let Some(b) = p.d.bounds()
                {
                    marker_x = marker_x.min(b.x);
                    marker_y = marker_y.min(b.y);
                }
            }
        });
        (marker_x, marker_y, Rect::new(x0, y0, x1 - x0, y1 - y0))
    };
    let (_, top_y, top_bars) = legend_rect("Top");
    assert!(top_y < top_bars.y);
    let (_, bottom_y, bottom_bars) = legend_rect("Bottom");
    assert!(bottom_y > bottom_bars.bottom());
    let (right_x, _, right_bars) = legend_rect("Right");
    assert!(right_x > right_bars.right());
    let (left_x, _, left_bars) = legend_rect("LeftCenter");
    assert!(left_x < left_bars.x);
    let (center_x, _, _) = legend_rect("TopCenter");
    let (plain_x, _, _) = legend_rect("Top");
    assert!(center_x > plain_x);
    assert_eq!(
        LegendPosition::parse("rightcenter"),
        LegendPosition::RightCenter
    );
    // Single series without a series field: no legend.
    let single = dataview(&["A"], &[("M", vec![Some(1.0)])]);
    assert!(class_group(&render("clusteredColumnChart", &single), "pbi-legend").is_none());
}

#[test]
fn axes_titles_gridlines_and_labels_follow_objects() {
    let dv = dataview(&["A", "B"], &[("Order Count", vec![Some(7.0), Some(3.0)])]);
    let t = texts(&render("clusteredBarChart", &dv));
    assert!(
        t.contains(&"Cat".to_owned()) && t.contains(&"Order Count".to_owned()),
        "{t:?}"
    );
    assert!(
        t.contains(&"0".to_owned()) && t.contains(&"5".to_owned()),
        "{t:?}"
    );
    let t = texts(&render_with(
        "clusteredBarChart",
        &dv,
        Json::Null,
        604.0,
        176.0,
    ));
    assert!(
        t.contains(&"6".to_owned()) && t.contains(&"4".to_owned()),
        "{t:?}"
    );
    let objs = objects(&[
        ("categoryAxis", "showAxisTitle", "false"),
        ("valueAxis", "titleText", "'Feeds'"),
        ("valueAxis", "gridlineShow", "false"),
        ("labels", "show", "true"),
    ]);
    let g = render_with("clusteredBarChart", &dv, objs, 400.0, 250.0);
    let t = texts(&g);
    assert!(
        !t.contains(&"Cat".to_owned()) && t.contains(&"Feeds".to_owned()),
        "{t:?}"
    );
    assert!(
        t.contains(&"7".to_owned()) && t.contains(&"3".to_owned()),
        "data labels {t:?}"
    );
    let axes = class_group(&g, "pbi-axes").unwrap();
    assert!(!axes.children.iter().any(|c| matches!(c, Node::Line(_))));
    let g = render("clusteredBarChart", &dv);
    let axes = class_group(&g, "pbi-axes").unwrap();
    let dotted = axes.children.iter().any(|c| match c {
        Node::Line(l) => !l.stroke.dash.is_empty(),
        _ => false,
    });
    assert!(dotted, "dotted value gridlines by default");
    // Default bar color: dataColors[0]; dataPoint.defaultColor overrides.
    assert_eq!(bar_rects(&g)[0].3, Color::rgb(0x11, 0x8D, 0xFF));
    let objs = json!({"dataPoint": [{"properties": {"defaultColor": {"solid": {"color": lit("'#C00000'")}}}}]});
    let g = render_with("clusteredBarChart", &dv, objs, 400.0, 250.0);
    assert_eq!(bar_rects(&g)[0].3, Color::rgb(0xC0, 0, 0));
}

#[test]
fn lines_areas_and_combos() {
    let dv = dataview(
        &["Jan", "Feb", "Mar", "Apr"],
        &[("Sales", vec![Some(5.0), None, Some(7.0), Some(9.0)])],
    );
    let g = render("lineChart", &dv);
    let lines = class_group(&g, "pbi-lines").unwrap();
    // The gap after Jan breaks the line: a dot for Jan, a polyline for Mar–Apr.
    assert_eq!(lines.children.len(), 2);
    let pts = points(&g);
    assert_eq!(
        pts.iter().map(|p| p.1).collect::<Vec<_>>(),
        vec![0, 2, 3],
        "hit circles for the non-blank points"
    );
    let area = render("areaChart", &dv);
    let fills = class_group(&area, "pbi-areas").unwrap();
    assert_eq!(fills.children.len(), 1);
    let combo = dataview(
        &["A", "B", "C"],
        &[
            ("Count", vec![Some(5.0), Some(3.0), Some(1.0)]),
            ("2:Rate", vec![Some(0.5), Some(0.25), Some(0.75)]),
        ],
    );
    let g = render("lineClusteredColumnComboChart", &combo);
    assert_eq!(bar_rects(&g).len(), 3);
    assert!(class_group(&g, "pbi-lines").is_some_and(|l| !l.children.is_empty()));
    let t = texts(&g);
    assert!(t.contains(&"Rate".to_owned()), "secondary axis title {t:?}");
    let stacked = dataview(
        &["A", "B"],
        &[
            ("M1", vec![Some(1.0), Some(2.0)]),
            ("M2", vec![Some(3.0), Some(4.0)]),
        ],
    );
    let g = render("stackedAreaChart", &stacked);
    assert_eq!(class_group(&g, "pbi-areas").unwrap().children.len(), 2);
}

#[test]
fn continuous_date_axis() {
    let mut dv = dataview(
        &["a", "b", "c"],
        &[("M", vec![Some(1.0), Some(3.0), Some(2.0)])],
    );
    let dates = [46_023.0, 46_054.0, 46_235.0];
    if let Some(cat) = dv.categorical.as_mut() {
        cat.categories[0].values = dates
            .iter()
            .map(|d| CellValue::new(Value::from_oadate(*d).unwrap(), "date"))
            .collect();
    }
    dv.metadata[0].data_type = Some(pbi_model::DataType::DateTime);
    let g = render("lineChart", &dv);
    let t = texts(&g);
    assert!(t.iter().any(|s| s.contains("2026")), "{t:?}");
    // Points are spaced by date, not evenly.
    let xs: Vec<f64> = points(&g)
        .iter()
        .filter_map(|(_, _, n)| match n {
            Node::Path(p) => p.d.bounds().map(|b| b.center().0),
            _ => None,
        })
        .collect();
    assert_eq!(xs.len(), 3);
    assert!((xs[2] - xs[1]) > 3.0 * (xs[1] - xs[0]), "{xs:?}");
}

#[test]
fn selection_dims_other_points() {
    use pbi_render::query::PointSelection;
    let dv = dataview(&["A", "B"], &[("M", vec![Some(2.0), Some(1.0)])]);
    let mut f = frame(400.0, 250.0);
    f.visual_type = "clusteredColumnChart".into();
    let visual = Visual {
        name: "v1".into(),
        visual_type: "clusteredColumnChart".into(),
        ..Visual::default()
    };
    let theme = ThemeResolver::default();
    let data = Ok(dv);
    let mut state = InteractionState::new();
    state.selected_points = Some(PointSelection::point(
        "v1",
        vec![(
            pbi_model::FieldExpr::Column {
                entity: "T".into(),
                property: "Cat".into(),
            },
            Value::String("B".into()),
        )],
    ));
    let ctx = VisualCtx {
        frame: &f,
        visual: &visual,
        theme: &theme,
        fonts: FontBook::shared(),
        data: Some(&data),
        state: &state,
    };
    let g = charts::render(&ctx).unwrap();
    let mut opacities = Vec::new();
    walk(&g, &mut |g| {
        if matches!(
            g.id.as_ref().map(|i| &i.kind),
            Some(HitKind::DataPoint { .. })
        ) {
            opacities.push(g.opacity);
        }
    });
    assert_eq!(opacities, vec![charts::DIMMED_OPACITY, 1.0]);
}

#[test]
fn svg_output_is_deterministic() {
    let dv = series_dataview(
        &["A", "B", "C"],
        &[
            ("North", vec![Some(1.0), Some(2.0), Some(4.0)]),
            ("South", vec![Some(3.0), None, Some(1.0)]),
        ],
    );
    for k in [
        "pieChart",
        "clusteredBarChart",
        "columnChart",
        "lineChart",
        "lineStackedColumnComboChart",
    ] {
        let a = svg_of(render(k, &dv));
        let b = svg_of(render(k, &dv));
        assert_eq!(a, b, "{k}");
        assert!(a.contains("data-hit=\"v1|dataPoint|"), "{k}");
    }
    // Untyped paint stays opaque.
    assert_eq!(Paint::solid(Color::BLACK).opacity, 1.0);
}
