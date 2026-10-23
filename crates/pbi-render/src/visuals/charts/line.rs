//! Line, area and stacked area charts, and the line part of combos.
//!
//! One polyline per series through the category centres (or the
//! continuous x positions of dates/numbers), broken at gaps (BLANK).
//! Areas fill down to 0 (stacked areas: down to the series below, with
//! cumulative positive and negative stacks). `lineStyles.strokeWidth`
//! (theme: 3), `lineStyle`, `showMarker`/`markerSize` apply; every point
//! gets an invisible hit circle ([`HitId`](crate::scene::HitId)
//! `DataPoint{series, index}`), up to [`MAX_HIT_POINTS`] per series.

use crate::scene::{
    Color, Group, Paint, PathBuilder, PathData, PathNode, Rect, Stroke, TextAnchor,
};

use super::bar::{series_legend, split_series};
use super::cartesian::{self, Layout, Spec};
use super::colors::series_colors;
use super::format::{NumberStyle, data_label};
use super::labels::{DataLabels, baseline_centered, width};
use super::scale::{self, LinearScale};
use super::{ChartCtx, ChartKind};

/// Most hit circles emitted per series.
pub const MAX_HIT_POINTS: usize = 2_000;
/// Fill opacity of areas.
const AREA_OPACITY: f64 = 0.4;

/// Renders a line, area or stacked area chart.
pub fn render(cc: &ChartCtx<'_>) -> Group {
    let mut g = Group::new();
    let data = cc.data;
    let fonts = cc.fonts();
    let colors = series_colors(&cc.data_point_colors(), data, cc.resolver());
    let mut area = cc.area;
    if let Some(legend) = series_legend(cc, &colors) {
        area = legend.draw(&mut g, area, fonts).plot;
    }
    if area.is_empty() {
        return g;
    }
    let (primary, _) = split_series(data, false);
    let secondary: Vec<usize> = data
        .series
        .iter()
        .enumerate()
        .filter(|(_, s)| s.secondary)
        .map(|(i, _)| i)
        .collect();
    let stacked = cc.kind == ChartKind::StackedArea;
    let fill = cc.kind != ChartKind::Line;
    let domain = line_domain(cc, &primary, stacked, fill);
    let domain2 = (!secondary.is_empty()).then(|| line_domain(cc, &secondary, false, false));
    let fmt = |idx: &[usize]| {
        NumberStyle::from_format(
            idx.first()
                .and_then(|s| data.series.get(*s))
                .and_then(|s| s.format_string.as_deref()),
        )
    };
    let spec = Spec {
        domain,
        domain2,
        style: fmt(&primary),
        style2: fmt(&secondary),
        allow_continuous: true,
        bars: false,
    };
    let layout = cartesian::layout(cc, area, &spec);
    layout.draw_axes(cc, &mut g);
    g.push(draw_series(
        cc,
        &layout,
        &primary,
        &colors,
        layout.y.scale,
        fill,
        stacked,
    ));
    if !secondary.is_empty() {
        let scale = layout.y2.as_ref().map_or(layout.y.scale, |a| a.scale);
        g.push(draw_series(
            cc, &layout, &secondary, &colors, scale, false, false,
        ));
    }
    g
}

/// Value domain of lines: stacks for stacked areas; 0 included for areas
/// and for positive lines whose minimum is below 60% of the maximum,
/// otherwise the nice extent of the data.
fn line_domain(cc: &ChartCtx<'_>, idx: &[usize], stacked: bool, fill: bool) -> (f64, f64) {
    let data = cc.data;
    if stacked {
        let segs = super::bar::segments(data, idx, 0..data.len(), super::Stacking::Stacked);
        return scale::value_domain(segs.iter().flat_map(|s| [s.from, s.to]), true);
    }
    let values: Vec<f64> = idx
        .iter()
        .filter_map(|s| data.series.get(*s))
        .flat_map(|s| s.values.iter().flatten().copied())
        .collect();
    let (lo, hi) = scale::value_domain(values.iter().copied(), false);
    if fill || values.is_empty() || (lo >= 0.0 && lo < hi * 0.6) || (hi <= 0.0 && hi > lo * 0.6) {
        return scale::value_domain(values, true);
    }
    scale::nice_domain(lo, hi, 5)
}

/// `lineStyles` settings.
struct LineStyle {
    width: f64,
    dash: Vec<f64>,
    markers: bool,
    marker_r: f64,
}

fn line_style(cc: &ChartCtx<'_>) -> LineStyle {
    let o = cc.objects();
    let width = o
        .number("lineStyles", "strokeWidth")
        .filter(|w| *w > 0.0)
        .unwrap_or(3.0)
        .min(20.0);
    let dash = match o
        .text("lineStyles", "lineStyle")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "dashed" => vec![width * 3.0, width * 2.0],
        "dotted" => vec![width, width * 2.0],
        _ => Vec::new(),
    };
    let markers = o.bool("lineStyles", "showMarker").unwrap_or(false);
    let marker_r = o
        .number("lineStyles", "markerSize")
        .filter(|s| *s > 0.0)
        .unwrap_or(5.0)
        .min(30.0)
        / 2.0
        + 1.0;
    LineStyle {
        width,
        dash,
        markers,
        marker_r,
    }
}

fn circle(cx: f64, cy: f64, r: f64) -> PathData {
    PathData::pie_slice(cx, cy, r, 0.0, 0.0, 360.0)
}

/// Draws `idx` series as lines (and areas with `fill`) on `scale`.
pub fn draw_series(
    cc: &ChartCtx<'_>,
    layout: &Layout,
    idx: &[usize],
    colors: &[Color],
    scale: LinearScale,
    fill: bool,
    stacked: bool,
) -> Group {
    let data = cc.data;
    let ls = line_style(cc);
    let visible = layout.x.visible(data.len());
    let baseline = scale.map_clamped(0.0);
    let mut fills = Group::new().with_class("pbi-areas");
    let mut lines = Group::new().with_class("pbi-lines");
    let mut points = Group::new().with_class("pbi-points");
    let mut label_group = Group::new().with_class("pbi-data-labels");
    let labels = DataLabels::read(&cc.props, false);
    // Running stacks per category (positive, negative).
    let mut stack_pos = vec![0.0; data.len()];
    let mut stack_neg = vec![0.0; data.len()];
    let mut placed_labels: Vec<Rect> = Vec::new();
    let max_abs = idx
        .iter()
        .filter_map(|s| data.series.get(*s))
        .flat_map(|s| s.values.iter().flatten())
        .fold(0.0f64, |m, v| m.max(v.abs()));
    for s in idx {
        let Some(series) = data.series.get(*s) else {
            continue;
        };
        let color = colors
            .get(*s)
            .copied()
            .unwrap_or_else(|| cc.resolver().data_color(*s));
        // Runs of consecutive points: (x, y top, y base, category).
        let mut runs: Vec<Vec<(f64, f64, f64, usize)>> = vec![Vec::new()];
        for i in visible.clone() {
            let (Some(x), Some(v)) = (layout.x.center(cc, i), series.value(i)) else {
                if runs.last().is_some_and(|r| !r.is_empty()) {
                    runs.push(Vec::new());
                }
                continue;
            };
            let (top, base) = if stacked {
                let slot = if v >= 0.0 {
                    stack_pos.get_mut(i)
                } else {
                    stack_neg.get_mut(i)
                };
                match slot {
                    Some(acc) => {
                        let from = *acc;
                        *acc += v;
                        (scale.map_clamped(*acc), scale.map_clamped(from))
                    }
                    None => (scale.map_clamped(v), baseline),
                }
            } else {
                (scale.map_clamped(v), baseline)
            };
            if let Some(run) = runs.last_mut() {
                run.push((x, top, base, i));
            }
        }
        for run in runs.iter().filter(|r| !r.is_empty()) {
            if fill && run.len() > 1 {
                let mut b = PathBuilder::new();
                for (k, (x, y, _, _)) in run.iter().enumerate() {
                    if k == 0 {
                        b.move_to(*x, *y);
                    } else {
                        b.line_to(*x, *y);
                    }
                }
                for (x, _, base, _) in run.iter().rev() {
                    b.line_to(*x, *base);
                }
                b.close();
                fills.push(PathNode::new(b.build()).fill(Paint::new(color, AREA_OPACITY)));
            }
            if run.len() == 1 {
                if let Some((x, y, _, _)) = run.first() {
                    lines.push(
                        PathNode::new(circle(*x, *y, ls.width.max(2.0))).fill(Paint::solid(color)),
                    );
                }
                continue;
            }
            let mut b = PathBuilder::new();
            for (k, (x, y, _, _)) in run.iter().enumerate() {
                if k == 0 {
                    b.move_to(*x, *y);
                } else {
                    b.line_to(*x, *y);
                }
            }
            lines.push(
                PathNode::new(b.build())
                    .stroke(Stroke::new(color, ls.width).with_dash(ls.dash.clone())),
            );
        }
        for (n, (x, y, _, i)) in runs.iter().flatten().enumerate() {
            if ls.markers {
                points.push(PathNode::new(circle(*x, *y, ls.marker_r)).fill(Paint::solid(color)));
            }
            if n < MAX_HIT_POINTS {
                let mut hit = Group::new().with_id(cc.hit(*s, *i));
                hit.push(
                    PathNode::new(circle(*x, *y, ls.marker_r.max(4.0)))
                        .fill(Paint::new(color, 0.0)),
                );
                points.push(hit);
            }
            if labels.show
                && let Some(v) = series.value(*i)
            {
                let text = data_label(
                    v,
                    series.formatted(*i),
                    max_abs,
                    &NumberStyle::from_format(series.format_at(*i)),
                    labels.unit,
                    labels.precision,
                );
                let w = width(cc.fonts(), &text, &labels.style);
                let lh = labels.style.line_height();
                let above = y - lh / 2.0 - 4.0;
                let below = y + lh / 2.0 + 4.0;
                // Above the point, else below; never over another label.
                let free = |cy: f64| {
                    let r = Rect::new(x - w / 2.0, cy - lh / 2.0, w, lh);
                    let inside =
                        r.y >= layout.plot.y - 0.5 && r.bottom() <= layout.plot.bottom() + 0.5;
                    let clear = !placed_labels.iter().any(|q: &Rect| {
                        r.x < q.right() && q.x < r.right() && r.y < q.bottom() && q.y < r.bottom()
                    });
                    (inside && clear).then_some(r)
                };
                let Some(rect) = free(above).or_else(|| free(below)) else {
                    continue;
                };
                placed_labels.push(rect);
                let cy = rect.y + lh / 2.0;
                label_group.text(
                    *x,
                    baseline_centered(&labels.style, cy),
                    text,
                    labels.style.clone(),
                    TextAnchor::Middle,
                );
            }
        }
    }
    let mut g = Group::new();
    g.push(fills);
    g.push(lines);
    g.push(points);
    g.push(label_group);
    g
}
