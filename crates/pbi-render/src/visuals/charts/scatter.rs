//! Scatter and bubble charts (`scatterChart`).
//!
//! Roles: `Category`/`Details` (one point per value), `Series`/`Legend`
//! (color per series value), `X`, `Y`, `Size` (bubbles), `Play` and
//! `Tooltips`. The query groups by Details × Legend (× Play), so every
//! (row, legend value) pair with a numeric X and Y is a point. A play axis
//! is not animated: only its first frame (the smallest play value) is
//! drawn.
//!
//! As in the service:
//!
//! - numeric X (`categoryAxis`) and Y (`valueAxis`) axes over the data
//!   extent (zero is not forced), padded so markers stay inside the plot,
//!   with "nice" ticks, gridlines on both axes, axis titles (the X and Y
//!   field names), `start`/`end`, display units and precision;
//! - markers are 8px circles in the series color (`dataPoint.fill` per
//!   legend value or point, `defaultColor`, else the palette);
//!   `bubbles.markerShape` (`circle`, `square`, `diamond`, `triangle`) and
//!   `fillPoint.show` (hollow markers when off) apply;
//! - with `Size`, bubble areas are proportional to the size: "magnitude"
//!   scaling (0 to the largest magnitude) for single-sign data, "data
//!   range" scaling (smallest to largest value) when signs are mixed, or as
//!   set by `bubbles.markerRangeType`; radii range from 2px to 7.5% of the
//!   plot's shorter side (6–60px), times `2^(bubbleSize / 10)`. Larger
//!   bubbles are drawn first so small ones stay visible;
//! - category labels (`categoryLabels.show`, off by default: the Details
//!   value above the marker) and data labels (`labels.show`, off by
//!   default: the Y value right of the marker); a label that would leave
//!   the plot (and the margins above and right of it) or overlap an
//!   earlier one tries the other sides, else is skipped;
//! - the legend (`legend.*`) lists the legend values when there is a legend
//!   field.
//!
//! Hit ids: every marker is `DataPoint{series, row}` (`series` indexes the
//! `Y` columns, one per legend value).

use pbi_engine::Value;

use crate::dataview::{BLANK_LABEL, CellValue, ColumnMeta, ValueColumn};
use crate::scene::{
    Clip, Color, Group, Paint, PathBuilder, PathData, PathNode, Rect, Stroke, TextAnchor,
};

use super::ChartCtx;
use super::axis::AxisSettings;
use super::colors::series_colors;
use super::format::{NumberStyle, data_label, tick_label};
use super::labels::{DataLabels, baseline_centered, rotated_text, tailor, width};
use super::legend::{Legend, LegendItem};
use super::scale::{self, LinearScale};

/// Most points drawn (Power BI's scatter limit).
pub const MAX_POINTS: usize = 10_000;
/// Marker radius without a `Size` field (8px circles).
pub const MARKER_RADIUS: f64 = 4.0;
/// Smallest bubble radius.
pub const MIN_BUBBLE_RADIUS: f64 = 2.0;
/// Most labels drawn.
const MAX_LABELS: usize = 2_000;
/// Gap between tick labels and the plot.
const TICK_GAP: f64 = 6.0;

/// Marker shape (`bubbles.markerShape`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerShape {
    Circle,
    Square,
    Diamond,
    Triangle,
}

impl MarkerShape {
    /// Parses a shape name (case-insensitive); unknown → circle.
    pub fn parse(text: Option<&str>) -> Self {
        match text.unwrap_or("").trim().to_ascii_lowercase().as_str() {
            "square" => Self::Square,
            "diamond" => Self::Diamond,
            "triangle" => Self::Triangle,
            _ => Self::Circle,
        }
    }

    fn path(self, cx: f64, cy: f64, r: f64) -> PathData {
        let mut b = PathBuilder::new();
        match self {
            Self::Circle => return PathData::pie_slice(cx, cy, r, 0.0, 0.0, 360.0),
            Self::Square => {
                let h = r * 0.89;
                b.move_to(cx - h, cy - h)
                    .line_to(cx + h, cy - h)
                    .line_to(cx + h, cy + h)
                    .line_to(cx - h, cy + h);
            }
            Self::Diamond => {
                b.move_to(cx, cy - r)
                    .line_to(cx + r, cy)
                    .line_to(cx, cy + r)
                    .line_to(cx - r, cy);
            }
            Self::Triangle => {
                b.move_to(cx, cy - r)
                    .line_to(cx + r * 0.87, cy + r * 0.5)
                    .line_to(cx - r * 0.87, cy + r * 0.5);
            }
        }
        b.close();
        b.build()
    }
}

/// One plotted point.
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    /// Hit id `(series, index)`.
    pub series: usize,
    pub index: usize,
    pub x: f64,
    pub y: f64,
    pub size: Option<f64>,
    /// Details label (category columns other than `Play`).
    pub label: String,
    pub y_formatted: String,
    pub color: Color,
}

/// Points, legend entries and the axis fields of a scatter chart.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScatterData {
    pub points: Vec<Point>,
    pub legend: Vec<LegendItem>,
    pub x_meta: Option<ColumnMeta>,
    pub y_meta: Option<ColumnMeta>,
    /// A `Size` field is bound.
    pub bubbles: bool,
}

fn number(c: Option<&CellValue>) -> Option<f64> {
    c.filter(|c| c.raw.is_numeric())
        .and_then(|c| c.raw.as_f64())
        .filter(|v| v.is_finite())
}

fn same_series(a: &ValueColumn, b: &ValueColumn) -> bool {
    match (&a.series, &b.series) {
        (None, None) => true,
        (Some(x), Some(y)) => {
            x.formatted == y.formatted && (x.raw.total_cmp(&y.raw) == std::cmp::Ordering::Equal)
        }
        _ => false,
    }
}

fn label_of(c: Option<&CellValue>) -> String {
    match c {
        Some(c) if !c.formatted.is_empty() => c.formatted.clone(),
        Some(c) if !c.is_blank() => c.raw.to_string(),
        _ => BLANK_LABEL.to_owned(),
    }
}

/// Reads the points of a scatter chart (see the module docs).
pub fn scatter_data(cc: &ChartCtx<'_>) -> ScatterData {
    let data = cc.data;
    let mut out = ScatterData::default();
    let Some(dv) = cc.dataview() else {
        return out;
    };
    let Some(cat) = dv.categorical.as_ref() else {
        return out;
    };
    let role = |meta_idx: usize, r: &str| {
        dv.meta(meta_idx)
            .is_some_and(|m| m.roles.iter().any(|x| x == r))
    };
    let (play, details): (Vec<usize>, Vec<usize>) = (0..cat.categories.len()).partition(|k| {
        cat.categories
            .get(*k)
            .is_some_and(|c| role(c.meta_idx, "Play"))
    });
    let n = data.len();
    let play_values = |i: usize| -> Vec<&Value> {
        play.iter()
            .filter_map(|k| cat.categories.get(*k))
            .map(|c| c.values.get(i).map_or(&Value::Blank, |v| &v.raw))
            .collect()
    };
    let lexi = |a: &[&Value], b: &[&Value]| {
        a.iter()
            .zip(b)
            .map(|(x, y)| x.total_cmp(y))
            .find(|o| o.is_ne())
            .unwrap_or(std::cmp::Ordering::Equal)
    };
    let first_frame: Option<Vec<&Value>> = if play.is_empty() {
        None
    } else {
        (0..n).map(play_values).min_by(|a, b| lexi(a, b))
    };
    let in_frame = |i: usize| {
        first_frame
            .as_ref()
            .is_none_or(|f| lexi(&play_values(i), f).is_eq())
    };
    let colors = cc.data_point_colors();
    let resolver = cc.resolver();
    let palette = series_colors(&colors, data, resolver);
    for (s, sd) in data.series.iter().enumerate() {
        let Some(ycol) = cat.values.get(sd.column) else {
            continue;
        };
        if !role(ycol.meta_idx, "Y") {
            continue;
        }
        let sibling = |r: &str| {
            cat.values
                .iter()
                .find(|vc| same_series(vc, ycol) && role(vc.meta_idx, r))
        };
        let Some(xcol) = sibling("X") else {
            continue;
        };
        let size = sibling("Size");
        if out.x_meta.is_none() {
            out.x_meta = dv.meta(xcol.meta_idx).cloned();
            out.y_meta = dv.meta(ycol.meta_idx).cloned();
        }
        out.bubbles |= size.is_some();
        let color = palette
            .get(s)
            .copied()
            .unwrap_or_else(|| resolver.data_color(s));
        out.legend.push(LegendItem {
            label: sd
                .series_label
                .clone()
                .unwrap_or_else(|| sd.measure_name.clone()),
            color,
        });
        for i in 0..n {
            if out.points.len() >= MAX_POINTS {
                break;
            }
            if !in_frame(i) {
                continue;
            }
            let (Some(x), Some(y)) = (number(xcol.values.get(i)), number(ycol.values.get(i)))
            else {
                continue;
            };
            let mut fields = data.category_fields(i);
            if let (Some(meta), Some(v)) = (&data.series_meta, &sd.series_value) {
                fields.push((meta, v));
            }
            let label = details
                .iter()
                .filter_map(|k| cat.categories.get(*k))
                .map(|c| label_of(c.values.get(i)))
                .collect::<Vec<_>>()
                .join(" ");
            out.points.push(Point {
                series: s,
                index: i,
                x,
                y,
                size: size.and_then(|c| number(c.values.get(i))),
                label,
                y_formatted: sd.formatted(i).to_owned(),
                color: sd
                    .conditional_fill(i)
                    .or_else(|| colors.scoped_color(&fields))
                    .unwrap_or(color),
            });
        }
    }
    out
}

/// Bubble radius per point (see the module docs); `MARKER_RADIUS` × the
/// size factor without a `Size` field.
pub fn radii(sd: &ScatterData, max_radius: f64, mode: Option<&str>, factor: f64) -> Vec<f64> {
    if !sd.bubbles {
        return vec![MARKER_RADIUS * factor; sd.points.len()];
    }
    let rmin = MIN_BUBBLE_RADIUS;
    let rmax = (max_radius * factor).max(rmin);
    let sizes: Vec<f64> = sd.points.iter().filter_map(|p| p.size).collect();
    let lo = sizes.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = sizes.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let mode = mode.unwrap_or("auto").to_ascii_lowercase();
    let data_range = if mode.contains("range") {
        true
    } else if mode.contains("magnitude") {
        false
    } else {
        lo < 0.0 && hi > 0.0
    };
    let magnitude = sizes.iter().map(|s| s.abs()).fold(0.0, f64::max);
    sd.points
        .iter()
        .map(|p| {
            let Some(s) = p.size else { return rmin };
            let r = if data_range {
                let span = hi - lo;
                let t = if span.is_finite() && span > 0.0 {
                    ((s - lo) / span).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                (rmin * rmin + t * (rmax * rmax - rmin * rmin)).sqrt()
            } else if magnitude > 0.0 && magnitude.is_finite() {
                rmax * (s.abs() / magnitude).clamp(0.0, 1.0).sqrt()
            } else {
                rmin
            };
            if r.is_finite() { r.max(rmin) } else { rmin }
        })
        .collect()
}

/// Data extent of `values`, widened when degenerate.
fn extent(values: impl Iterator<Item = f64>) -> (f64, f64) {
    let (lo, hi) = scale::value_domain(values, false);
    (lo, hi)
}

/// Pads `(lo, hi)` by `pad_px` at both ends of a `len_px` long axis.
fn pad_domain((lo, hi): (f64, f64), pad_px: f64, len_px: f64) -> (f64, f64) {
    let span = hi - lo;
    if !(span.is_finite() && span > 0.0 && len_px > 2.0 * pad_px) {
        return (lo, hi);
    }
    let d = span * pad_px / (len_px - 2.0 * pad_px);
    let (a, b) = (lo - d, hi + d);
    if a.is_finite() && b.is_finite() {
        (a, b)
    } else {
        (lo, hi)
    }
}

/// A numeric axis: settings, scale, ticks and labels.
struct NumAxis {
    settings: AxisSettings,
    scale: LinearScale,
    ticks: Vec<f64>,
    labels: Vec<String>,
}

fn tick_labels(ticks: &[f64], style: &NumberStyle, settings: &AxisSettings) -> Vec<String> {
    let step = match (ticks.first(), ticks.get(1)) {
        (Some(a), Some(b)) => b - a,
        _ => 1.0,
    };
    let max_abs = ticks.iter().map(|t| t.abs()).fold(0.0, f64::max);
    ticks
        .iter()
        .map(|t| tick_label(*t, step, max_abs, style, settings.unit))
        .collect()
}

/// Renders a scatter / bubble chart.
pub fn render(cc: &ChartCtx<'_>) -> Group {
    let mut g = Group::new();
    let sd = scatter_data(cc);
    let fonts = cc.fonts();
    let resolver = cc.resolver();
    let o = cc.objects();
    let mut area = cc.area;
    if cc.data.series_meta.is_some() {
        let title = cc.data.series_meta.as_ref().map(|m| m.display_name.clone());
        if let Some(legend) = Legend::read(&cc.props, true, title, sd.legend.clone()) {
            area = legend.draw(&mut g, area, fonts).plot;
        }
    }
    if sd.points.is_empty() || area.width < 8.0 || area.height < 8.0 {
        return g;
    }
    let name = |m: &Option<ColumnMeta>| m.as_ref().map(|m| m.display_name.clone());
    let xs = AxisSettings::read(&cc.props, resolver, "categoryAxis", name(&sd.x_meta), true);
    let ys = AxisSettings::read(&cc.props, resolver, "valueAxis", name(&sd.y_meta), true);
    let style = |m: &Option<ColumnMeta>| {
        NumberStyle::from_format(m.as_ref().and_then(|m| m.format_string.as_deref()))
    };
    let (x_style, y_style) = (style(&sd.x_meta), style(&sd.y_meta));
    let x_raw = extent(sd.points.iter().map(|p| p.x));
    let y_raw = extent(sd.points.iter().map(|p| p.y));

    // Margins from estimated ticks (as the column charts).
    let lh_x = xs.label_style.line_height();
    let lh_y = ys.label_style.line_height();
    let title_h = |a: &AxisSettings| {
        if a.show && a.title.is_some() {
            a.title_style.line_height()
        } else {
            0.0
        }
    };
    let top = if ys.show { lh_y / 2.0 + 2.0 } else { 4.0 };
    let bottom = title_h(&xs) + if xs.show { lh_x + 4.0 } else { 0.0 };
    let est_h = (area.height - top - bottom).max(1.0);
    let y_est = ys.apply_range(y_raw);
    let y_ticks_est = scale::ticks(y_est.0, y_est.1, scale::tick_count(est_h, false));
    let y_labels_est = tick_labels(&y_ticks_est, &y_style, &ys);
    let left = title_h(&ys)
        + if ys.show {
            y_labels_est
                .iter()
                .map(|l| width(fonts, l, &ys.label_style))
                .fold(0.0, f64::max)
                + TICK_GAP
        } else {
            4.0
        };
    let est_w = (area.width - left - 8.0).max(1.0);
    let x_est = xs.apply_range(x_raw);
    let x_ticks_est = scale::ticks(x_est.0, x_est.1, scale::tick_count(est_w, true));
    let right = if xs.show {
        tick_labels(&x_ticks_est, &x_style, &xs)
            .last()
            .map_or(4.0, |l| (width(fonts, l, &xs.label_style) / 2.0).max(4.0))
    } else {
        4.0
    };
    let plot = Rect::new(
        area.x + left,
        area.y + top,
        (area.width - left - right).max(1.0),
        est_h,
    );

    // Marker sizes, then the axis domains padded so markers fit.
    let max_radius = (plot.width.min(plot.height) * 0.075).clamp(6.0, 60.0);
    let factor = o
        .number("bubbles", "bubbleSize")
        .filter(|v| v.is_finite())
        .map_or(1.0, |v| 2f64.powf(v / 10.0).clamp(0.25, 4.0));
    let radii = radii(
        &sd,
        max_radius,
        o.text("bubbles", "markerRangeType").as_deref(),
        factor,
    );
    let pad = radii.iter().copied().fold(0.0, f64::max) + 2.0;
    let make_axis = |settings: AxisSettings,
                     raw: (f64, f64),
                     len: f64,
                     horizontal: bool,
                     style: &NumberStyle,
                     range: (f64, f64)| {
        let domain = settings.apply_range(pad_domain(raw, pad, len));
        let ticks = scale::ticks(domain.0, domain.1, scale::tick_count(len, horizontal));
        let labels = tick_labels(&ticks, style, &settings);
        NumAxis {
            scale: LinearScale::new(domain, range),
            ticks,
            labels,
            settings,
        }
    };
    let x = make_axis(
        xs,
        x_raw,
        plot.width,
        true,
        &x_style,
        (plot.x, plot.right()),
    );
    let y = make_axis(
        ys,
        y_raw,
        plot.height,
        false,
        &y_style,
        (plot.bottom(), plot.y),
    );
    draw_axes(&mut g, cc, area, plot, &x, &y);

    // Markers: larger first, so small bubbles stay on top.
    let shape = MarkerShape::parse(o.text("bubbles", "markerShape").as_deref());
    let filled = o.bool("fillPoint", "show").unwrap_or(true);
    let opacity = cc.selection_opacity();
    let mut order: Vec<usize> = (0..sd.points.len()).collect();
    let radius = |k: usize| radii.get(k).copied().unwrap_or(MARKER_RADIUS);
    order.sort_by(|a, b| radius(*b).total_cmp(&radius(*a)).then(a.cmp(b)));
    let mut marks = Group::new()
        .with_class("pbi-points")
        .with_clip(Clip::rect(plot));
    let mut placed: Vec<(usize, f64, f64, f64)> = Vec::new();
    for k in order {
        let Some(p) = sd.points.get(k) else { continue };
        let r = radius(k);
        let (cx, cy) = (x.scale.map(p.x), y.scale.map(p.y));
        if !(cx.is_finite() && cy.is_finite())
            || cx < plot.x - r
            || cx > plot.right() + r
            || cy < plot.y - r
            || cy > plot.bottom() + r
        {
            continue;
        }
        let mut pg = Group::new().with_id(cc.hit(p.series, p.index));
        if let Some(op) = opacity
            .as_ref()
            .and_then(|o| o.get(p.index))
            .filter(|o| **o < 1.0)
        {
            pg = pg.with_opacity(*op);
        }
        let node = PathNode::new(shape.path(cx, cy, r));
        pg.push(if filled {
            node.fill(Paint::solid(p.color))
        } else {
            node.fill(Paint::new(p.color, 0.0))
                .stroke(Stroke::new(p.color, 1.5))
        });
        marks.push(pg);
        placed.push((k, cx, cy, r));
    }
    g.push(marks);

    // Category labels above the markers, data labels right of them.
    let show_cat = o.bool("categoryLabels", "show").unwrap_or(false);
    let labels = DataLabels::read(&cc.props, false);
    if show_cat || labels.show {
        let mut cat_style = o.text_style("categoryLabels", "smallLightLabel");
        if let Some(c) = o.color("categoryLabels", "labelColor") {
            cat_style.color = c;
        }
        let max_abs = sd.points.iter().map(|p| p.y.abs()).fold(0.0, f64::max);
        let mut lg = Group::new().with_class("pbi-data-labels");
        let mut taken: Vec<Rect> = Vec::new();
        let mut count = 0usize;
        // Draw order: as the markers, smallest last; place labels of the
        // first points in data order.
        placed.sort_by_key(|(k, ..)| *k);
        for (k, cx, cy, r) in &placed {
            if count >= MAX_LABELS {
                break;
            }
            let Some(p) = sd.points.get(*k) else { continue };
            // Candidate boxes: above, right, below, left (category
            // labels), or right, left, above, below (data labels).
            let mut try_place = |text: String, style: &crate::text::TextStyle, above: bool| {
                if text.is_empty() {
                    return;
                }
                let text = tailor(fonts, &text, style, plot.width.min(160.0));
                let w = width(fonts, &text, style);
                let lh = style.line_height();
                let up = Rect::new(cx - w / 2.0, cy - r - 2.0 - lh, w, lh);
                let down = Rect::new(cx - w / 2.0, cy + r + 2.0, w, lh);
                let right = Rect::new(cx + r + 3.0, cy - lh / 2.0, w, lh);
                let left = Rect::new(cx - r - 3.0 - w, cy - lh / 2.0, w, lh);
                let candidates = if above {
                    [up, right, down, left]
                } else {
                    [right, left, up, down]
                };
                // Labels may use the margins above and right of the plot.
                let fits = |rect: &Rect| {
                    let inside = rect.x >= plot.x - 0.5
                        && rect.right() <= area.right() + 0.5
                        && rect.y >= area.y - 0.5
                        && rect.bottom() <= plot.bottom() + 0.5;
                    inside
                        && !taken.iter().any(|q| {
                            rect.x < q.right()
                                && q.x < rect.right()
                                && rect.y < q.bottom()
                                && q.y < rect.bottom()
                        })
                };
                let Some(rect) = candidates.into_iter().find(|r| fits(r)) else {
                    return;
                };
                if text.is_empty() {
                    return;
                }
                taken.push(rect);
                count += 1;
                lg.text(
                    rect.x,
                    baseline_centered(style, rect.y + lh / 2.0),
                    text,
                    style.clone(),
                    TextAnchor::Start,
                );
            };
            if show_cat {
                try_place(p.label.clone(), &cat_style, true);
            }
            if labels.show {
                let text = data_label(
                    p.y,
                    &p.y_formatted,
                    max_abs,
                    &style(&sd.y_meta),
                    labels.unit,
                    labels.precision,
                );
                try_place(text, &labels.style, false);
            }
        }
        g.push(lg);
    }
    g
}

fn draw_axes(g: &mut Group, cc: &ChartCtx<'_>, area: Rect, plot: Rect, x: &NumAxis, y: &NumAxis) {
    let fonts = cc.fonts();
    let mut ax = Group::new().with_class("pbi-axes");
    if let Some(stroke) = &y.settings.gridlines {
        for t in &y.ticks {
            let py = y.scale.map(*t);
            ax.line(plot.x, py, plot.right(), py, stroke.clone());
        }
    }
    if let Some(stroke) = &x.settings.gridlines {
        for t in &x.ticks {
            let px = x.scale.map(*t);
            ax.line(px, plot.y, px, plot.bottom(), stroke.clone());
        }
    }
    if y.settings.show {
        let style = &y.settings.label_style;
        let lh = style.line_height();
        let mut last: Option<f64> = None;
        for (t, label) in y.ticks.iter().zip(&y.labels) {
            let py = y.scale.map(*t);
            if last.is_some_and(|l| (l - py).abs() < lh * 0.9) {
                continue;
            }
            last = Some(py);
            ax.text(
                plot.x - TICK_GAP,
                baseline_centered(style, py),
                label.clone(),
                style.clone(),
                TextAnchor::End,
            );
        }
        if let Some(title) = &y.settings.title {
            let ts = &y.settings.title_style;
            let text = tailor(fonts, title, ts, plot.height);
            ax.push(rotated_text(
                &text,
                ts,
                area.x + ts.line_height() / 2.0,
                plot.y + plot.height / 2.0,
                -90.0,
            ));
        }
    }
    if x.settings.show {
        let style = &x.settings.label_style;
        let mut last_right = f64::NEG_INFINITY;
        for (t, label) in x.ticks.iter().zip(&x.labels) {
            let px = x.scale.map(*t);
            let w = width(fonts, label, style);
            if px - w / 2.0 < last_right + 4.0 {
                continue;
            }
            last_right = px + w / 2.0;
            ax.text(
                px,
                plot.bottom() + 2.0 + style.ascent(),
                label.clone(),
                style.clone(),
                TextAnchor::Middle,
            );
        }
        if let Some(title) = &x.settings.title {
            let ts = &x.settings.title_style;
            let text = tailor(fonts, title, ts, plot.width);
            ax.text(
                plot.x + plot.width / 2.0,
                baseline_centered(ts, area.bottom() - ts.line_height() / 2.0),
                text,
                ts.clone(),
                TextAnchor::Middle,
            );
        }
    }
    g.push(ax);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(sizes: &[Option<f64>]) -> ScatterData {
        ScatterData {
            points: sizes
                .iter()
                .enumerate()
                .map(|(i, s)| Point {
                    series: 0,
                    index: i,
                    x: i as f64,
                    y: i as f64,
                    size: *s,
                    label: String::new(),
                    y_formatted: String::new(),
                    color: Color::BLACK,
                })
                .collect(),
            bubbles: true,
            ..ScatterData::default()
        }
    }

    #[test]
    fn bubble_areas_follow_power_bi_range_scaling() {
        // Magnitude (single sign): area ∝ value, 0 → the minimum.
        let r = radii(
            &data(&[Some(100.0), Some(25.0), Some(0.0), None]),
            40.0,
            None,
            1.0,
        );
        assert!((r[0] - 40.0).abs() < 1e-9);
        assert!((r[1] - 20.0).abs() < 1e-9, "quarter area → half radius");
        assert_eq!(r[2], MIN_BUBBLE_RADIUS);
        assert_eq!(r[3], MIN_BUBBLE_RADIUS);
        // Mixed signs: data range, smallest value → smallest bubble.
        let r = radii(
            &data(&[Some(-10.0), Some(10.0), Some(0.0)]),
            40.0,
            None,
            1.0,
        );
        assert_eq!(r[0], MIN_BUBBLE_RADIUS);
        assert!((r[1] - 40.0).abs() < 1e-9);
        let mid = (MIN_BUBBLE_RADIUS.powi(2) + 0.5 * (1600.0 - MIN_BUBBLE_RADIUS.powi(2))).sqrt();
        assert!((r[2] - mid).abs() < 1e-9);
        // Forced magnitude: |-10| is as large as 10.
        let r = radii(
            &data(&[Some(-10.0), Some(10.0)]),
            40.0,
            Some("magnitude"),
            1.0,
        );
        assert_eq!(r[0], r[1]);
        // No size field: fixed markers, scaled by bubbleSize.
        let mut plain = data(&[None]);
        plain.bubbles = false;
        assert_eq!(radii(&plain, 40.0, None, 2.0), vec![MARKER_RADIUS * 2.0]);
    }
}
