//! Scatter/bubble, funnel, gauge, treemap and waterfall charts on synthetic
//! DataViews: marks, order, labels, theme colors, axis ticks, hit geometry,
//! edge cases (empty, BLANK, negative, single point, long labels, huge
//! ranges), deterministic SVG and the per-visual render cache.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use pbi_engine::Value;
use pbi_model::{Page, Position, Report, Visual};
use pbi_render::dataview::{
    Categorical, CategoryColumn, CellValue, ColumnMeta, DataView, DataViewKind, FieldRef,
    FieldRefKind, ValueColumn,
};
use pbi_render::layout::{VisualFrame, VisualHeader};
use pbi_render::query::{InteractionState, PointSelection};
use pbi_render::scene::{Color, Group, HitKind, HitRegion, Insets, Node, PathSeg, Rect, Scene};
use pbi_render::svg::{SvgOptions, to_svg};
use pbi_render::text::FontBook;
use pbi_render::theme::{Theme, ThemeResolver};
use pbi_render::visuals::charts::{self, waterfall};
use pbi_render::visuals::{VisualCtx, render_visual};
use serde_json::{Value as Json, json};

// ---------------------------------------------------------------------------
// DataView builder
// ---------------------------------------------------------------------------

fn meta(name: &str, role: &str, measure: bool, format: Option<&str>) -> ColumnMeta {
    ColumnMeta {
        roles: vec![role.to_owned()],
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
        format_string: format.map(str::to_owned),
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

fn text(s: &str) -> CellValue {
    if s.is_empty() {
        CellValue::new(Value::Blank, "(Blank)")
    } else {
        CellValue::new(Value::String(s.into()), s)
    }
}

/// Builds categorical DataViews column by column.
#[derive(Default)]
struct Dv {
    dv: DataView,
}

impl Dv {
    fn new() -> Self {
        Self {
            dv: DataView {
                kind: DataViewKind::Categorical,
                categorical: Some(Categorical::default()),
                ..DataView::default()
            },
        }
    }

    fn cat(&mut self) -> &mut Categorical {
        self.dv.categorical.as_mut().unwrap()
    }

    /// A text category column bound to `role`.
    fn category(mut self, name: &str, role: &str, values: &[&str]) -> Self {
        self.dv.metadata.push(meta(name, role, false, None));
        let meta_idx = self.dv.metadata.len() - 1;
        self.cat().categories.push(CategoryColumn {
            meta_idx,
            values: values.iter().map(|v| text(v)).collect(),
        });
        self
    }

    /// The series (legend / breakdown) field.
    fn series_field(mut self, name: &str, role: &str) -> Self {
        self.dv.metadata.push(meta(name, role, false, None));
        let idx = self.dv.metadata.len() - 1;
        self.cat().series_meta_idx = Some(idx);
        self
    }

    /// A measure bound to `role`; returns the builder and its meta index.
    fn measure(mut self, name: &str, role: &str, format: Option<&str>) -> Self {
        self.dv.metadata.push(meta(name, role, true, format));
        self
    }

    /// Values of the measure named `name` (for series value `series`).
    fn values(mut self, name: &str, series: Option<&str>, values: &[Option<f64>]) -> Self {
        let meta_idx = self
            .dv
            .metadata
            .iter()
            .position(|m| m.display_name == name)
            .unwrap();
        self.cat().values.push(ValueColumn {
            meta_idx,
            series: series.map(text),
            values: values.iter().map(|v| num(*v)).collect(),
            highlights: None,
        });
        self
    }

    fn build(self) -> DataView {
        self.dv
    }
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

fn frame(visual_type: &str, w: f64, h: f64) -> VisualFrame {
    let r = Rect::new(0.0, 0.0, w, h);
    VisualFrame {
        name: "v1".into(),
        visual_type: visual_type.into(),
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

fn solid(hex: &str) -> Json {
    json!({"solid": {"color": lit(&format!("'{hex}'"))}})
}

struct Opts {
    objects: Json,
    theme: ThemeResolver,
    state: InteractionState,
    size: (f64, f64),
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            objects: Json::Null,
            theme: ThemeResolver::default(),
            state: InteractionState::new(),
            size: (400.0, 250.0),
        }
    }
}

fn render_opts(visual_type: &str, dv: &DataView, o: &Opts) -> Group {
    let f = frame(visual_type, o.size.0, o.size.1);
    let visual = Visual {
        name: "v1".into(),
        visual_type: visual_type.into(),
        objects: o.objects.clone(),
        ..Visual::default()
    };
    let data = Ok(dv.clone());
    let ctx = VisualCtx {
        frame: &f,
        visual: &visual,
        theme: &o.theme,
        fonts: FontBook::shared(),
        data: Some(&data),
        state: &o.state,
    };
    render_visual(&ctx).unwrap_or_default()
}

fn render_with(visual_type: &str, dv: &DataView, objs: Json, w: f64, h: f64) -> Group {
    render_opts(
        visual_type,
        dv,
        &Opts {
            objects: objs,
            size: (w, h),
            ..Opts::default()
        },
    )
}

fn render(visual_type: &str, dv: &DataView) -> Group {
    render_with(visual_type, dv, Json::Null, 400.0, 250.0)
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

fn class_group<'a>(g: &'a Group, class: &str) -> Option<&'a Group> {
    if g.class.as_deref() == Some(class) {
        return Some(g);
    }
    g.children.iter().find_map(|c| match c {
        Node::Group(child) => class_group(child, class),
        _ => None,
    })
}

/// A data point mark: hit id, bounds and fill color of its first child.
#[derive(Debug, Clone, PartialEq)]
struct Mark {
    series: usize,
    index: usize,
    bounds: Rect,
    color: Option<Color>,
    opacity: f64,
}

fn marks(g: &Group) -> Vec<Mark> {
    let mut out = Vec::new();
    walk(g, &mut |g| {
        if let Some(HitKind::DataPoint { series, index }) = g.id.as_ref().map(|i| &i.kind)
            && let Some(child) = g.children.first()
        {
            let (bounds, color) = match child {
                Node::Rect(r) => (r.rect, r.fill.map(|p| p.color)),
                Node::Path(p) => (
                    p.d.bounds().unwrap_or_default(),
                    p.fill.filter(|f| f.opacity > 0.0).map(|f| f.color),
                ),
                _ => (Rect::default(), None),
            };
            out.push(Mark {
                series: *series,
                index: *index,
                bounds,
                color,
                opacity: g.opacity,
            });
        }
    });
    out
}

fn scene_of(g: Group, w: f64, h: f64) -> Scene {
    let mut scene = Scene::new(w, h);
    scene.root.push(g);
    scene
}

fn svg_of(g: Group) -> String {
    to_svg(&scene_of(g, 400.0, 250.0), &SvgOptions::default())
}

fn hits(g: Group) -> Vec<HitRegion> {
    scene_of(g, 400.0, 250.0)
        .hit_regions()
        .into_iter()
        .filter(|h| matches!(h.id.kind, HitKind::DataPoint { .. }))
        .collect()
}

fn hit_key(h: &HitRegion) -> (usize, usize) {
    match h.id.kind {
        HitKind::DataPoint { series, index } => (series, index),
        _ => (usize::MAX, usize::MAX),
    }
}

fn rgb(hex: &str) -> Color {
    Color::parse(hex).unwrap()
}

const BLUE: &str = "#118DFF";
const NAVY: &str = "#12239E";
const ORANGE: &str = "#E66C37";

/// The value of a tick label like `2.5K` or `-20`.
fn tick_value(label: &str) -> Option<f64> {
    let (body, mult) = match label.chars().last()? {
        'K' => (&label[..label.len() - 1], 1e3),
        'M' => (&label[..label.len() - 1], 1e6),
        _ => (label, 1.0),
    };
    body.replace(',', "").parse::<f64>().ok().map(|v| v * mult)
}

/// `true` if `step` is 1, 2, 2.5 or 5 × 10ⁿ.
fn is_nice(step: f64) -> bool {
    let e = step.abs().log10().floor();
    let m = step.abs() / 10f64.powf(e);
    [1.0, 2.0, 2.5, 5.0, 10.0]
        .iter()
        .any(|n| (m - n).abs() < 1e-6)
}

const TYPES: [&str; 5] = [
    "scatterChart",
    "funnel",
    "gauge",
    "treemap",
    "waterfallChart",
];

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

#[test]
fn new_types_render_as_charts_not_placeholders() {
    let dv = Dv::new()
        .category("Cat", "Category", &["A", "B"])
        .measure("M", "Y", Some("0"))
        .values("M", None, &[Some(3.0), Some(1.0)])
        .build();
    for t in TYPES {
        assert!(charts::handles(t), "{t}");
        let g = render(t, &dv);
        assert_eq!(g.class.as_deref(), Some("pbi-chart"), "{t}");
        assert!(!texts(&g).iter().any(|s| s.contains(t)), "{t} placeholder");
    }
}

// ---------------------------------------------------------------------------
// Scatter
// ---------------------------------------------------------------------------

/// Details A–D × legend North/South with X, Y and Size; South has no data
/// for D and a BLANK Y for C.
fn scatter_dv(with_size: bool) -> DataView {
    let mut b = Dv::new()
        .category("Product", "Category", &["A", "B", "C", "D"])
        .series_field("Region", "Series")
        .measure("Spend", "X", Some("#,0"))
        .measure("Revenue", "Y", Some("#,0"));
    if with_size {
        b = b.measure("Units", "Size", Some("0"));
    }
    let north = (
        [Some(-10.0), Some(5.0), Some(20.0), Some(40.0)],
        [Some(100.0), Some(-50.0), Some(30.0), Some(0.0)],
        [Some(400.0), Some(100.0), Some(25.0), Some(0.0)],
    );
    let south = (
        [Some(15.0), Some(25.0), Some(30.0), None],
        [Some(80.0), Some(60.0), None, None],
        [Some(100.0), Some(100.0), Some(100.0), None],
    );
    for (name, (x, y, s)) in [("North", north), ("South", south)] {
        b = b
            .values("Spend", Some(name), &x)
            .values("Revenue", Some(name), &y);
        if with_size {
            b = b.values("Units", Some(name), &s);
        }
    }
    b.build()
}

#[test]
fn scatter_plots_points_per_details_and_legend_with_nice_axes() {
    let dv = scatter_dv(false);
    let g = render("scatterChart", &dv);
    let pts = marks(&g);
    // 4 North points + 2 South points (C has a BLANK Y, D no data).
    let mut keys: Vec<(usize, usize)> = pts.iter().map(|m| (m.series, m.index)).collect();
    keys.sort_unstable();
    assert_eq!(keys, vec![(0, 0), (0, 1), (0, 2), (0, 3), (1, 0), (1, 1)]);
    // Legend colors from the theme palette, markers 8 px circles.
    for m in &pts {
        let expected = if m.series == 0 { BLUE } else { NAVY };
        assert_eq!(m.color, Some(rgb(expected)), "{m:?}");
        assert!((m.bounds.width - 8.0).abs() < 0.2, "{m:?}");
    }
    // Positions follow X (right) and Y (up), negative values included.
    let at = |s: usize, i: usize| {
        pts.iter()
            .find(|m| (m.series, m.index) == (s, i))
            .unwrap()
            .bounds
            .center()
    };
    assert!(at(0, 0).0 < at(0, 1).0 && at(0, 1).0 < at(0, 2).0 && at(0, 2).0 < at(0, 3).0);
    assert!(
        at(0, 0).1 < at(0, 2).1 && at(0, 2).1 < at(0, 1).1,
        "Y 100 above 30 above -50"
    );
    let t = texts(&g);
    for s in ["Region", "North", "South", "Spend", "Revenue"] {
        assert!(t.contains(&s.to_owned()), "{s} in {t:?}");
    }
    // Nice, evenly spaced ticks covering the data on both axes.
    let axes = class_group(&g, "pbi-axes").unwrap();
    let mut x_ticks = Vec::new();
    let mut y_ticks = Vec::new();
    for c in &axes.children {
        if let Node::Text(tn) = c
            && let Some(v) = tick_value(&tn.text)
        {
            match tn.anchor {
                pbi_render::scene::TextAnchor::Middle => x_ticks.push((tn.x, v)),
                pbi_render::scene::TextAnchor::End => y_ticks.push((tn.y, v)),
                pbi_render::scene::TextAnchor::Start => {}
            }
        }
    }
    for (ticks, lo, hi) in [(&x_ticks, -10.0, 40.0), (&y_ticks, -50.0, 100.0)] {
        assert!(ticks.len() >= 2, "{ticks:?}");
        let vals: Vec<f64> = ticks.iter().map(|t| t.1).collect();
        let step = vals[1] - vals[0];
        assert!(is_nice(step), "{vals:?}");
        assert!(
            vals.windows(2).all(|w| (w[1] - w[0] - step).abs() < 1e-9),
            "{vals:?}"
        );
        assert!(
            vals[0] >= lo - step && *vals.last().unwrap() <= hi + step,
            "{vals:?}"
        );
    }
    // Ticks go left to right and bottom to top.
    assert!(x_ticks.windows(2).all(|w| w[0].0 < w[1].0));
    assert!(y_ticks.windows(2).all(|w| w[0].0 > w[1].0));
    // Points stay inside the plot (between the extreme ticks' gridlines).
    let lines: Vec<f64> = axes
        .children
        .iter()
        .filter_map(|c| match c {
            Node::Line(l) if (l.x1 - l.x2).abs() < 1e-9 => Some(l.x1),
            _ => None,
        })
        .collect();
    assert!(!lines.is_empty(), "vertical gridlines on the X axis");
}

#[test]
fn bubbles_scale_by_area_and_draw_large_first() {
    let dv = scatter_dv(true);
    let g = render("scatterChart", &dv);
    let pts = marks(&g);
    assert_eq!(pts.len(), 6);
    let r = |s: usize, i: usize| {
        pts.iter()
            .find(|m| (m.series, m.index) == (s, i))
            .unwrap()
            .bounds
            .width
            / 2.0
    };
    // Single-sign sizes: magnitude scaling, area ∝ size (400 → rmax, 100 →
    // half the radius, 25 → a quarter, 0 → the 2 px minimum).
    assert!(
        (r(0, 1) / r(0, 0) - 0.5).abs() < 0.01,
        "{} {}",
        r(0, 1),
        r(0, 0)
    );
    assert!((r(0, 2) / r(0, 0) - 0.25).abs() < 0.01);
    assert!((r(0, 3) - 2.0).abs() < 0.05);
    assert!(r(0, 0) >= 6.0);
    // Paint order: radius descending.
    let radii: Vec<f64> = pts.iter().map(|m| m.bounds.width).collect();
    assert!(radii.windows(2).all(|w| w[0] >= w[1] - 1e-3), "{radii:?}");
    // Hit regions: one per bubble, at the bubbles.
    let regions = hits(g.clone());
    assert_eq!(regions.len(), 6);
    for h in &regions {
        let m = pts
            .iter()
            .find(|m| (m.series, m.index) == hit_key(h))
            .unwrap();
        assert!((h.rect.center().0 - m.bounds.center().0).abs() < 0.5);
    }
    // bubbleSize grows every bubble; markerRangeType 'DataRange' maps the
    // smallest size to the smallest bubble.
    let big = render_with(
        "scatterChart",
        &dv,
        objects(&[("bubbles", "bubbleSize", "10L")]),
        400.0,
        250.0,
    );
    let big_r = marks(&big)
        .iter()
        .map(|m| m.bounds.width)
        .fold(0.0, f64::max);
    assert!(big_r > radii[0] * 1.5, "{big_r} vs {}", radii[0]);
    let ranged = render_with(
        "scatterChart",
        &dv,
        objects(&[("bubbles", "markerRangeType", "'DataRange'")]),
        400.0,
        250.0,
    );
    let rp = marks(&ranged);
    let w = |s: usize, i: usize| {
        rp.iter()
            .find(|m| (m.series, m.index) == (s, i))
            .unwrap()
            .bounds
            .width
    };
    assert!(
        (w(0, 3) - 4.0).abs() < 0.1,
        "0 is the smallest: {}",
        w(0, 3)
    );
}

#[test]
fn scatter_labels_shapes_and_hollow_markers() {
    let dv = scatter_dv(false);
    assert!(!texts(&render("scatterChart", &dv)).contains(&"A".to_owned()));
    let objs = objects(&[
        ("categoryLabels", "show", "true"),
        ("labels", "show", "true"),
        ("legend", "show", "false"),
        ("bubbles", "markerShape", "'square'"),
        ("fillPoint", "show", "false"),
    ]);
    let g = render_with("scatterChart", &dv, objs, 600.0, 400.0);
    let t = texts(&g);
    assert!(
        t.contains(&"A".to_owned()) && t.contains(&"D".to_owned()),
        "{t:?}"
    );
    // Y values as data labels (labels that would overlap are skipped).
    let labels = class_group(&g, "pbi-data-labels").unwrap();
    let shown = texts(labels);
    for s in ["30", "80", "60"] {
        assert!(shown.contains(&s.to_owned()), "{s} in {shown:?}");
    }
    assert!(!t.contains(&"Region".to_owned()), "legend hidden");
    let points = class_group(&g, "pbi-points").unwrap();
    let mut shapes = 0;
    walk(points, &mut |g| {
        for c in &g.children {
            if let Node::Path(p) = c {
                assert!(p.stroke.is_some(), "hollow markers are stroked");
                assert!(!p.d.segs.iter().any(|s| matches!(s, PathSeg::ArcTo { .. })));
                shapes += 1;
            }
        }
    });
    assert_eq!(shapes, 6);
}

#[test]
fn scatter_draws_the_first_play_frame_only() {
    let dv = Dv::new()
        .category("Product", "Category", &["A", "A", "B", "B"])
        .category("Year", "Play", &["2021", "2020", "2021", "2020"])
        .measure("X", "X", None)
        .measure("Y", "Y", None)
        .values("X", None, &[Some(1.0), Some(2.0), Some(3.0), Some(4.0)])
        .values("Y", None, &[Some(1.0), Some(2.0), Some(3.0), Some(4.0)])
        .build();
    let objs = objects(&[("categoryLabels", "show", "true")]);
    let g = render_with("scatterChart", &dv, objs, 400.0, 250.0);
    let mut idx: Vec<usize> = marks(&g).iter().map(|m| m.index).collect();
    idx.sort_unstable();
    assert_eq!(idx, vec![1, 3], "rows of 2020");
    let t = texts(&g);
    assert!(
        t.contains(&"A".to_owned()) && !t.iter().any(|s| s.contains("2020")),
        "{t:?}"
    );
}

// ---------------------------------------------------------------------------
// Funnel
// ---------------------------------------------------------------------------

fn funnel_dv() -> DataView {
    Dv::new()
        .category(
            "Stage",
            "Category",
            &["Lead", "Qualify", "Solution", "Proposal", "Finalize"],
        )
        .measure("Count", "Y", Some("0"))
        .values(
            "Count",
            None,
            &[Some(268.0), Some(94.0), Some(74.0), Some(37.0), Some(14.0)],
        )
        .build()
}

#[test]
fn funnel_centres_bars_by_value_with_conversion_labels() {
    let g = render("funnel", &funnel_dv());
    let bars = marks(&g);
    assert_eq!(
        bars.iter().map(|m| m.index).collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4]
    );
    let first = bars[0].bounds;
    for (m, v) in bars.iter().zip([268.0, 94.0, 74.0, 37.0, 14.0]) {
        // Centred, top to bottom in data order, width ∝ value.
        assert!((m.bounds.center().0 - first.center().0).abs() < 1e-6);
        assert!((m.bounds.width / first.width - v / 268.0).abs() < 1e-9);
        assert_eq!(m.color, Some(rgb(BLUE)));
    }
    assert!(bars.windows(2).all(|w| w[0].bounds.y < w[1].bounds.y));
    let t = texts(&g);
    for s in ["Lead", "Finalize", "268", "94", "14", "100%", "5.2%"] {
        assert!(t.contains(&s.to_owned()), "{s} in {t:?}");
    }
    // Labels sit left of the widest bar.
    let regions = hits(g);
    assert_eq!(
        regions.iter().map(hit_key).collect::<Vec<_>>(),
        (0..5).map(|i| (0, i)).collect::<Vec<_>>()
    );
}

#[test]
fn funnel_options_colors_and_edge_cases() {
    let dv = funnel_dv();
    let objs = json!({
        "labels": [{"properties": {"funnelLabelStyle": lit("'Percent of first'")}}],
        "percentBarLabel": [{"properties": {"show": lit("false")}}],
        "dataPoint": [
            {"properties": {"defaultColor": solid("#30A440")}},
            {"properties": {"fill": solid("#C00000")}, "selector": {"data": [{"scopeId": {"Comparison": {
                "ComparisonKind": 0,
                "Left": {"Column": {"Expression": {"SourceRef": {"Entity": "T"}}, "Property": "Stage"}},
                "Right": {"Literal": {"Value": "'Proposal'"}}
            }}}]}}
        ]
    });
    let g = render_with("funnel", &dv, objs, 400.0, 250.0);
    let t = texts(&g);
    for s in ["100%", "35.1%", "5.2%"] {
        assert!(t.contains(&s.to_owned()), "{s} of the first: {t:?}");
    }
    assert!(!t.contains(&"268".to_owned()), "{t:?}");
    assert!(class_group(&g, "pbi-conversion-rate").is_none());
    let colors: Vec<Color> = marks(&g).iter().filter_map(|m| m.color).collect();
    assert_eq!(colors[0], rgb("#30A440"));
    assert_eq!(colors[3], rgb("#C00000"));
    // BLANK and zero stages keep their label but draw no bar; a long label
    // is truncated with an ellipsis.
    let long = "A very long stage name that cannot possibly fit in the label column";
    let dv = Dv::new()
        .category("Stage", "Category", &[long, "Empty", "Zero", "Last"])
        .measure("Count", "Y", None)
        .values("Count", None, &[Some(10.0), None, Some(0.0), Some(-4.0)])
        .build();
    let g = render("funnel", &dv);
    assert_eq!(
        marks(&g).iter().map(|m| m.index).collect::<Vec<_>>(),
        vec![0, 3]
    );
    let t = texts(&g);
    assert!(
        t.iter()
            .any(|s| s.starts_with("A very") && s.ends_with("...")),
        "{t:?}"
    );
    assert!(t.contains(&"Empty".to_owned()) && t.contains(&"Zero".to_owned()));
    // A funnel of measures (no category): one stage per measure.
    let dv = Dv::new()
        .measure("Leads", "Y", None)
        .measure("Wins", "Y", None)
        .values("Leads", None, &[Some(50.0)])
        .values("Wins", None, &[Some(10.0)])
        .build();
    let g = render("funnel", &dv);
    assert_eq!(
        marks(&g)
            .iter()
            .map(|m| (m.series, m.index))
            .collect::<Vec<_>>(),
        vec![(0, 0), (1, 0)]
    );
    assert!(texts(&g).contains(&"20%".to_owned()));
}

// ---------------------------------------------------------------------------
// Gauge
// ---------------------------------------------------------------------------

fn gauge_dv(roles: &[(&str, &str, Option<f64>)]) -> DataView {
    let mut b = Dv::new();
    for (name, role, _) in roles {
        b = b.measure(name, role, Some("#,0.00"));
    }
    for (name, _, v) in roles {
        b = b.values(name, None, &[*v]);
    }
    b.build()
}

/// The arc group: track path and fill path (if any).
fn gauge_arcs(g: &Group) -> (Rect, Option<(Rect, Color)>, Color) {
    let arc = class_group(g, "pbi-gauge-arc").unwrap();
    let path = |n: &Node| match n {
        Node::Path(p) => (p.d.bounds().unwrap(), p.fill.unwrap().color),
        _ => panic!("{n:?}"),
    };
    let (track, track_color) = path(&arc.children[0]);
    (track, arc.children.get(1).map(path), track_color)
}

#[test]
fn gauge_defaults_to_twice_the_value_and_formats_like_the_service() {
    let dv = gauge_dv(&[("Sales", "Y", Some(184_207.43))]);
    let g = render("gauge", &dv);
    let t = texts(&g);
    for s in ["184.21K", "0.00K", "368.41K"] {
        assert!(t.contains(&s.to_owned()), "{s} in {t:?}");
    }
    let (track, fill, track_color) = gauge_arcs(&g);
    assert_eq!(track_color, rgb("#E9E9E9"));
    let (fill, fill_color) = fill.unwrap();
    assert_eq!(fill_color, rgb(BLUE));
    // Half-way: the fill covers the left half of the semicircle.
    assert!(
        (fill.right() - track.center().0).abs() < 0.5,
        "{fill:?} {track:?}"
    );
    assert!(
        (track.width / track.height - 2.0).abs() < 0.05,
        "a semicircle"
    );
    // One hit region (tooltip), no target needle.
    let regions = hits(g.clone());
    assert_eq!(
        regions.iter().map(hit_key).collect::<Vec<_>>(),
        vec![(0, 0)]
    );
    assert!(class_group(&g, "pbi-gauge-target").is_none());
}

#[test]
fn gauge_bounds_target_and_colors() {
    let dv = gauge_dv(&[
        ("Sales", "Y", Some(184_207.43)),
        ("Low", "MinValue", Some(100_000.0)),
        ("High", "MaxValue", Some(250_000.0)),
        ("Goal", "TargetValue", Some(200_000.0)),
    ]);
    let objs = json!({"dataPoint": [{"properties": {
        "fill": solid("#365FDC"),
        "target": solid("#333333")
    }}]});
    let g = render_with("gauge", &dv, objs, 400.0, 250.0);
    let t = texts(&g);
    for s in ["184.21K", "100.00K", "250.00K", "200.00K"] {
        assert!(t.contains(&s.to_owned()), "{s} in {t:?}");
    }
    let (track, fill, _) = gauge_arcs(&g);
    let (fill, color) = fill.unwrap();
    assert_eq!(color, rgb("#365FDC"));
    // 56% of the way: past the top of the arc.
    assert!(fill.right() > track.center().0 + 5.0);
    let needle = class_group(&g, "pbi-gauge-target").unwrap();
    let Node::Path(p) = &needle.children[0] else {
        panic!()
    };
    assert_eq!(p.stroke.as_ref().unwrap().color, rgb("#333333"));
    let b = p.d.bounds().unwrap();
    assert!(
        b.center().0 > track.center().0,
        "target 200K right of the middle"
    );
    // Formatting options: axis max without a MaxValue field, callout
    // display units and precision, labels hidden.
    let dv = gauge_dv(&[("Sales", "Y", Some(50.0))]);
    let objs = objects(&[
        ("axis", "max", "200D"),
        ("calloutValue", "labelDisplayUnits", "1D"),
        ("calloutValue", "labelPrecision", "1L"),
        ("labels", "show", "false"),
    ]);
    let t = texts(&render_with("gauge", &dv, objs, 400.0, 250.0));
    assert_eq!(t, vec!["50.0"], "{t:?}");
}

#[test]
fn gauge_edge_cases() {
    // Negative value: the range is [2v, 0] and the arc is half full.
    let dv = gauge_dv(&[("Delta", "Y", Some(-50.0))]);
    let g = render("gauge", &dv);
    let (track, fill, _) = gauge_arcs(&g);
    assert!((fill.unwrap().0.right() - track.center().0).abs() < 0.5);
    assert!(texts(&g).contains(&"-100.00".to_owned()), "{:?}", texts(&g));
    // BLANK value: an empty track and `(Blank)`.
    let dv = gauge_dv(&[("Sales", "Y", None)]);
    let g = render("gauge", &dv);
    assert!(gauge_arcs(&g).1.is_none());
    assert!(texts(&g).contains(&"(Blank)".to_owned()));
    // Value above the maximum: the arc is full, not overflowing.
    let dv = gauge_dv(&[
        ("Sales", "Y", Some(500.0)),
        ("Max", "MaxValue", Some(100.0)),
    ]);
    let (track, fill, _) = gauge_arcs(&render("gauge", &dv));
    assert!((fill.unwrap().0.right() - track.right()).abs() < 0.5);
}

// ---------------------------------------------------------------------------
// Treemap
// ---------------------------------------------------------------------------

#[test]
fn treemap_squarifies_by_size_with_palette_colors() {
    let dv = Dv::new()
        .category("Category", "Group", &["Small", "Big", "Mid", "Tiny", "Neg"])
        .measure("Sales", "Values", Some("0"))
        .values(
            "Sales",
            None,
            &[Some(12.0), Some(50.0), Some(25.0), Some(5.0), Some(-3.0)],
        )
        .build();
    let g = render("treemap", &dv);
    let tiles = marks(&g);
    // Largest first: Big (row 1) at the top left; negative values skipped.
    assert_eq!(
        tiles.iter().map(|m| m.index).collect::<Vec<_>>(),
        vec![1, 2, 0, 3]
    );
    assert!(tiles[0].bounds.x < 2.0 && tiles[0].bounds.y < 2.0);
    // Palette colors in size order.
    assert_eq!(
        tiles.iter().filter_map(|m| m.color).collect::<Vec<_>>(),
        vec![rgb(BLUE), rgb(NAVY), rgb(ORANGE), rgb("#6B007B")]
    );
    // Areas ∝ values (2 px gaps between branches aside) and no overlap.
    let total: f64 = 50.0 + 25.0 + 12.0 + 5.0;
    for (m, v) in tiles.iter().zip([50.0, 25.0, 12.0, 5.0]) {
        let outer = (m.bounds.width + 2.0) * (m.bounds.height + 2.0);
        assert!((outer / (400.0 * 250.0) - v / total).abs() < 1e-6, "{m:?}");
    }
    for (a, ma) in tiles.iter().enumerate() {
        for mb in tiles.iter().skip(a + 1) {
            assert!(ma.bounds.intersect(&mb.bounds).is_none_or(|r| r.is_empty()));
        }
    }
    let t = texts(&g);
    assert_eq!(t, vec!["Big", "Mid", "Small", "Tiny"]);
    assert_eq!(
        hits(g).iter().map(hit_key).collect::<Vec<_>>(),
        vec![(0, 1), (0, 2), (0, 0), (0, 3)]
    );
}

#[test]
fn treemap_nests_details_in_groups() {
    let long = "An extremely long store chain name for the treemap leaf";
    let dv = Dv::new()
        .category(
            "Category",
            "Group",
            &["Mens", "Mens", "Shoes", "Shoes", "Kids"],
        )
        .category(
            "Chain",
            "Details",
            &["Fashions", long, "Fashions", "Lindseys", "Lindseys"],
        )
        .measure("Sales", "Values", None)
        .values(
            "Sales",
            None,
            &[Some(30.0), Some(20.0), Some(15.0), Some(5.0), Some(10.0)],
        )
        .build();
    let objs = objects(&[("labels", "show", "true")]);
    let g = render_with("treemap", &dv, objs, 500.0, 300.0);
    let tiles = marks(&g);
    assert_eq!(tiles.len(), 5);
    // Leaves share their branch color; branches take the palette by size.
    let color_of = |i: usize| tiles.iter().find(|m| m.index == i).unwrap().color;
    assert_eq!(color_of(0), Some(rgb(BLUE)));
    assert_eq!(color_of(1), Some(rgb(BLUE)));
    assert_eq!(color_of(2), Some(rgb(NAVY)));
    assert_eq!(color_of(3), Some(rgb(NAVY)));
    assert_eq!(color_of(4), Some(rgb(ORANGE)));
    // Mens' leaves are inside the Mens branch (the top-left 50% of the area).
    let mens = tiles[0].bounds.union(&tiles[1].bounds);
    let share = (mens.width + 2.0) * (mens.height + 2.0) / (500.0 * 300.0);
    assert!((share - 0.625).abs() < 0.02, "{share}");
    let t = texts(&g);
    for s in ["Mens", "Shoes", "Kids", "Fashions", "Lindseys", "30"] {
        assert!(t.contains(&s.to_owned()), "{s} in {t:?}");
    }
    assert!(
        t.iter()
            .any(|s| s.starts_with("An ext") && s.ends_with("...")),
        "{t:?}"
    );
    // Labels are white by default.
    let texts_group = class_group(&g, "pbi-data-labels").unwrap();
    assert!(texts_group.children.iter().all(|c| match c {
        Node::Text(t) => t.style.color == Color::WHITE,
        _ => true,
    }));
}

// ---------------------------------------------------------------------------
// Waterfall
// ---------------------------------------------------------------------------

fn waterfall_dv() -> DataView {
    Dv::new()
        .category("Month", "Category", &["Jan", "Feb", "Mar", "Apr", "May"])
        .measure("Variance", "Y", Some("0"))
        .values(
            "Variance",
            None,
            &[Some(10.0), Some(-4.0), Some(6.0), None, Some(-15.0)],
        )
        .build()
}

#[test]
fn waterfall_runs_increases_and_decreases_to_a_total() {
    let g = render("waterfallChart", &waterfall_dv());
    let cols = marks(&g);
    // Jan, Feb, Mar, May (Apr is BLANK) carry hit ids; the total has none.
    assert_eq!(
        cols.iter().map(|m| m.index).collect::<Vec<_>>(),
        vec![0, 1, 2, 4]
    );
    let colors: Vec<Color> = cols.iter().filter_map(|m| m.color).collect();
    assert_eq!(
        colors,
        vec![
            rgb("#1AAB40"),
            rgb("#D64554"),
            rgb("#1AAB40"),
            rgb("#D64554")
        ]
    );
    // Each column starts where the previous one ended.
    let (jan, feb, mar, may) = (
        cols[0].bounds,
        cols[1].bounds,
        cols[2].bounds,
        cols[3].bounds,
    );
    assert!((jan.y - feb.y).abs() < 1e-6, "Feb hangs from Jan's top");
    assert!(
        (feb.bottom() - mar.bottom()).abs() < 1e-6,
        "Mar rises from Feb's end"
    );
    assert!(
        (mar.y - may.y).abs() < 1e-6,
        "May falls from Mar's top (Apr BLANK)"
    );
    assert!((feb.height / jan.height - 0.4).abs() < 1e-6);
    // The total: 10 - 4 + 6 - 15 = -3, below the zero line (Jan's bottom).
    let bars = class_group(&g, "pbi-bars").unwrap();
    let Some(Node::Group(total)) = bars.children.last() else {
        panic!()
    };
    assert!(total.id.is_none());
    let Node::Rect(total) = &total.children[0] else {
        panic!()
    };
    assert_eq!(total.fill.unwrap().color, rgb(BLUE));
    assert!((total.rect.y - jan.bottom()).abs() < 1e-6);
    assert!((total.rect.height / jan.height - 0.3).abs() < 1e-6);
    // Connectors between neighbours, legend, category labels.
    assert_eq!(class_group(&g, "pbi-connectors").unwrap().children.len(), 5);
    let t = texts(&g);
    for s in [
        "Increase", "Decrease", "Total", "Jan", "Apr", "May", "Month", "Variance",
    ] {
        assert!(t.contains(&s.to_owned()), "{s} in {t:?}");
    }
    assert!(!t.contains(&"Other".to_owned()));
    // Data labels (off by default) show the changes and the total.
    let objs = objects(&[("labels", "show", "true")]);
    let t = texts(&render_with(
        "waterfallChart",
        &waterfall_dv(),
        objs,
        400.0,
        250.0,
    ));
    for s in ["10", "-4", "6", "-15", "-3"] {
        assert!(t.contains(&s.to_owned()), "{s} in {t:?}");
    }
}

#[test]
fn waterfall_colors_follow_sentiment_objects_and_theme() {
    let objs = json!({"sentimentColors": [{"properties": {
        "increaseFill": solid("#00FF00"),
        "decreaseFill": solid("#FF0000"),
        "totalFill": solid("#0000FF")
    }}]});
    let g = render_with("waterfallChart", &waterfall_dv(), objs, 400.0, 250.0);
    let colors: Vec<Color> = marks(&g).iter().filter_map(|m| m.color).collect();
    assert_eq!(colors[0], rgb("#00FF00"));
    assert_eq!(colors[1], rgb("#FF0000"));
    // Theme sentiment colors and data colors.
    let custom = json!({
        "good": "#008000", "bad": "#800000",
        "dataColors": ["#000080"],
        "visualStyles": {"waterfallChart": {"*": {"legend": [{"show": false}]}}}
    });
    let theme = ThemeResolver::new(Theme::from_values(None, Some(&custom)));
    let g = render_opts(
        "waterfallChart",
        &waterfall_dv(),
        &Opts {
            theme,
            ..Opts::default()
        },
    );
    let colors: Vec<Color> = marks(&g).iter().filter_map(|m| m.color).collect();
    assert_eq!(colors[0], rgb("#008000"));
    assert_eq!(colors[1], rgb("#800000"));
    assert!(
        class_group(&g, "pbi-legend").is_none(),
        "visualStyles hide the legend"
    );
}

/// Q1–Q3 by seven regions: Q1 → Q2 changes A +5, B −4, C +3, D +2, E −1,
/// F +1, G −1 (two smallest go to "Other").
fn breakdown_dv() -> DataView {
    let regions = ["A", "B", "C", "D", "E", "F", "G"];
    let q1 = [10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0];
    let q2 = [15.0, 6.0, 13.0, 12.0, 9.0, 11.0, 9.0];
    let q3 = [15.0, 6.0, 13.0, 12.0, 9.0, 11.0, 30.0];
    let mut b = Dv::new()
        .category("Quarter", "Category", &["Q1", "Q2", "Q3"])
        .series_field("Region", "Breakdown")
        .measure("Sales", "Y", Some("0"));
    for (k, r) in regions.iter().enumerate() {
        b = b.values("Sales", Some(r), &[Some(q1[k]), Some(q2[k]), Some(q3[k])]);
    }
    b.build()
}

#[test]
fn waterfall_breakdown_shows_top_contributors_and_other() {
    let dv = breakdown_dv();
    let data = charts::ChartData::from_dataview(&dv);
    let steps = waterfall::steps(&data, 5);
    let labels: Vec<&str> = steps.iter().map(|s| s.label.as_str()).collect();
    // Q1 total; Q1→Q2: the five largest changes A +5, B −4, C +3, D +2,
    // E −1 from the largest increase to the largest decrease (F +1 and
    // G −1 cancel out: no "Other"); Q2 total; Q2→Q3: G +21; Q3 total.
    assert_eq!(
        labels,
        vec!["Q1", "A", "C", "D", "E", "B", "Q2", "G", "Q3"],
        "{steps:#?}"
    );
    let kinds: Vec<waterfall::StepKind> = steps.iter().map(|s| s.kind).collect();
    use waterfall::StepKind::{Decrease as D, Increase as I, Total as T};
    assert_eq!(kinds, vec![T, I, I, I, D, D, T, I, T]);
    // Changes run from one category total to the next.
    assert_eq!((steps[0].from, steps[0].to), (0.0, 70.0));
    assert_eq!(steps[1].from, 70.0);
    assert_eq!(steps[5].to, 75.0);
    assert_eq!((steps[6].from, steps[6].to), (0.0, 75.0));
    assert_eq!(steps[8].to, 96.0);
    // Hit ids: changes → (breakdown series, category), totals → the
    // category only.
    assert_eq!(steps[1].hit, Some((0, 1)));
    assert_eq!(steps[0].hit, Some((7, 0)));
    // Two breakdowns: A +5 and B −4, the rest (C +3, D +2, E −1, F +1,
    // G −1) is "Other" +4.
    let two = waterfall::steps(&data, 2);
    let labels: Vec<&str> = two.iter().map(|s| s.label.as_str()).collect();
    assert_eq!(labels, vec!["Q1", "A", "Other", "B", "Q2", "G", "Q3"]);
    assert_eq!(two[2].kind, waterfall::StepKind::Other);
    assert!((two[2].value - 4.0).abs() < 1e-9);
    assert_eq!(two[2].hit, Some((7, 1)));
    // Rendered: legend with Other, other in the theme's neutral color.
    let objs = objects(&[("breakdown", "maxBreakdowns", "2L")]);
    let g = render_with("waterfallChart", &dv, objs, 600.0, 300.0);
    let t = texts(&g);
    assert!(
        t.contains(&"Other".to_owned()) && t.contains(&"Q2".to_owned()),
        "{t:?}"
    );
    let cols = marks(&g);
    assert_eq!(cols.len(), 7);
    let other = cols
        .iter()
        .find(|m| m.color == Some(rgb("#D9B300")))
        .unwrap();
    assert_eq!((other.series, other.index), (7, 1));
}

// ---------------------------------------------------------------------------
// All types
// ---------------------------------------------------------------------------

#[test]
fn robust_against_empty_blank_nan_and_huge_inputs() {
    let many: Vec<String> = (0..10_000).map(|i| format!("Category {i}")).collect();
    let many_refs: Vec<&str> = many.iter().map(String::as_str).collect();
    let many_vals: Vec<Option<f64>> = (0..10_000)
        .map(|i| Some(f64::from(i % 97) - 40.0))
        .collect();
    let simple = |cats: &[&str], vals: &[Option<f64>]| {
        Dv::new()
            .category("Cat", "Category", cats)
            .measure("M", "Y", None)
            .measure("X", "X", None)
            .measure("S", "Size", None)
            .values("M", None, vals)
            .values("X", None, vals)
            .values("S", None, vals)
            .build()
    };
    let cases = vec![
        DataView::default(),
        Dv::new().build(),
        simple(&[], &[]),
        simple(&["", "B"], &[None, None]),
        simple(&["A"], &[Some(f64::NAN)]),
        simple(&["A", "B"], &[Some(f64::INFINITY), Some(f64::NEG_INFINITY)]),
        simple(&["Only"], &[Some(0.0)]),
        simple(&["Only"], &[Some(42.0)]),
        simple(&["A", "B"], &[Some(-1.0), Some(-2.0)]),
        simple(&["", "B", "C"], &[Some(0.0), Some(1e300), Some(-1e300)]),
        simple(&["A", "B"], &[Some(f64::MAX), Some(f64::MIN)]),
        simple(&many_refs, &many_vals),
        breakdown_dv(),
        scatter_dv(true),
    ];
    for dv in &cases {
        for t in TYPES {
            for (w, h) in [(400.0, 250.0), (30.0, 20.0), (3.0, 3.0)] {
                let svg = svg_of(render_with(t, dv, Json::Null, w, h));
                assert!(!svg.contains("NaN") && !svg.contains("inf"), "{t} {w}x{h}");
            }
        }
    }
    // A single scatter point sits inside the plot.
    let one = simple(&["Only"], &[Some(42.0)]);
    let pts = marks(&render("scatterChart", &one));
    assert_eq!(pts.len(), 1);
    let (x, y) = pts[0].bounds.center();
    assert!(x > 20.0 && x < 400.0 && y > 0.0 && y < 230.0, "{x} {y}");
}

#[test]
fn renders_are_deterministic() {
    let dvs = [
        ("scatterChart", scatter_dv(true)),
        ("funnel", funnel_dv()),
        (
            "gauge",
            gauge_dv(&[("S", "Y", Some(3.0)), ("T", "TargetValue", Some(4.0))]),
        ),
        ("treemap", funnel_dv()),
        ("waterfallChart", breakdown_dv()),
    ];
    for (t, dv) in &dvs {
        let a = svg_of(render(t, dv));
        let b = svg_of(render(t, dv));
        assert_eq!(a, b, "{t}");
        assert!(a.len() > 500, "{t} drew something");
    }
}

#[test]
fn selection_dims_other_points_in_the_source_visual() {
    let mut state = InteractionState::new();
    state.selected_points = Some(PointSelection::point(
        "v1",
        vec![(
            pbi_model::FieldExpr::Column {
                entity: "T".into(),
                property: "Stage".into(),
            },
            Value::String("Solution".into()),
        )],
    ));
    for t in ["funnel", "treemap"] {
        let g = render_opts(
            t,
            &funnel_dv(),
            &Opts {
                state: state.clone(),
                ..Opts::default()
            },
        );
        let ops: Vec<(usize, f64)> = marks(&g).iter().map(|m| (m.index, m.opacity)).collect();
        assert!(
            ops.iter().all(|(i, o)| (*o == 1.0) == (*i == 2)),
            "{t}: {ops:?}"
        );
    }
}

#[test]
fn visual_cache_reuses_unchanged_new_visuals() {
    use pbi_render::visuals::{
        InteractivePage, PageData, PageRenderOptions, PageViewState, VisualCache,
        render_interactive_page_cached,
    };
    let placed = |name: &str, visual_type: &str, x: f64, y: f64| Visual {
        name: name.into(),
        visual_type: visual_type.into(),
        position: Position {
            x,
            y,
            width: 400.0,
            height: 300.0,
            ..Position::default()
        },
        ..Visual::default()
    };
    let page = Page {
        name: "p".into(),
        width: 1280.0,
        height: 720.0,
        visuals: vec![
            placed("scatter", "scatterChart", 0.0, 0.0),
            placed("funnel", "funnel", 420.0, 0.0),
            placed("gauge", "gauge", 840.0, 0.0),
            placed("treemap", "treemap", 0.0, 400.0),
            placed("waterfall", "waterfallChart", 420.0, 400.0),
        ],
        ..Page::default()
    };
    let all = ["scatter", "funnel", "gauge", "treemap", "waterfall"];
    let data_with = |gauge_value: f64| PageData {
        data: vec![
            ("scatter".into(), Ok(scatter_dv(true))),
            ("funnel".into(), Ok(funnel_dv())),
            (
                "gauge".into(),
                Ok(gauge_dv(&[("S", "Y", Some(gauge_value))])),
            ),
            ("treemap".into(), Ok(funnel_dv())),
            ("waterfall".into(), Ok(breakdown_dv())),
        ],
        ..PageData::default()
    };
    let report = Report::default();
    let opts = PageRenderOptions::default();
    let theme = ThemeResolver::default();
    let view = PageViewState::new();
    let cache = VisualCache::new();
    let svg = |p: &InteractivePage| to_svg(&p.page.scene, &SvgOptions::default());
    let render = |state: &InteractionState, data: &PageData| -> Vec<String> {
        let cached = render_interactive_page_cached(
            &report,
            &page,
            &theme,
            state,
            &opts,
            &view,
            data,
            Some(&cache),
        );
        let fresh =
            render_interactive_page_cached(&report, &page, &theme, state, &opts, &view, data, None);
        assert_eq!(svg(&cached), svg(&fresh));
        assert_eq!(
            cached.page.scene.hit_regions(),
            fresh.page.scene.hit_regions()
        );
        cached.reused
    };
    let state = InteractionState::new();
    let data = data_with(3.0);
    assert!(render(&state, &data).is_empty(), "cold");
    assert_eq!(render(&state, &data), all, "warm");
    // New data for the gauge: the gauge only.
    let but = |skip: &str| -> Vec<&str> { all.into_iter().filter(|n| *n != skip).collect() };
    assert_eq!(render(&state, &data_with(4.0)), but("gauge"));
    // A point selected in the funnel: the funnel (its source) re-renders,
    // the others read selections only through their (unchanged) data.
    let mut selected = InteractionState::new();
    selected.selected_points = Some(PointSelection::point(
        "funnel",
        vec![(
            pbi_model::FieldExpr::Column {
                entity: "T".into(),
                property: "Stage".into(),
            },
            Value::String("Lead".into()),
        )],
    ));
    assert_eq!(render(&selected, &data), but("funnel"));
    // Every new visual exposes hit regions on the page.
    let page_hits =
        render_interactive_page_cached(&report, &page, &theme, &state, &opts, &view, &data, None)
            .page
            .scene
            .hit_regions();
    for name in all {
        assert!(
            page_hits
                .iter()
                .any(|h| h.id.visual == name && matches!(h.id.kind, HitKind::DataPoint { .. })),
            "{name}"
        );
    }
}
