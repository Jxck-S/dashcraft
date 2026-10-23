//! Bar (horizontal) and column (vertical) charts, clustered, stacked and
//! 100% stacked, plus the column part of line + column combos.
//!
//! # Horizontal bars
//!
//! Matched to the service's renders: the category axis title is rotated at
//! the left, category labels are right-aligned 9px left of the plot and
//! take at most `maxMarginFactor` (25%) of the width, the value axis
//! (ticks every [`scale::nice_step`], dotted gridlines) is at the bottom
//! with its title under it. Bands are at least
//! `max(preferredCategoryWidth, 1.4 × label line height)` tall (22.4px at
//! 9pt); bars fill 70% of the band (`innerPadding` 30%). When not all
//! categories fit, the first ones that fit are drawn (the DataView order:
//! value descending by default) and a vertical scrollbar is drawn at the
//! right.
//!
//! The value axis starts at 0 (or the most negative value / stack) and ends
//! at the largest value or stack, so the longest bar reaches the plot edge.

use crate::scene::{Color, Group, Paint, Rect, RectNode, TextAnchor};
use crate::text::TextStyle;

use super::axis::{AxisSettings, SCROLLBAR_SIZE, scrollbar};
use super::cartesian::{self, Spec};
use super::colors::series_colors;
use super::format::{NumberStyle, data_label, tick_label};
use super::labels::{
    DataLabels, baseline_centered, prefers_white_text, rotated_text, tailor, width,
};
use super::legend::{Legend, LegendItem};
use super::scale::{self, LinearScale};
use super::{ChartCtx, ChartData, Stacking, line};

/// Gap between category labels and the plot.
const CATEGORY_GAP: f64 = 9.0;
/// Most category labels measured for the label column width.
const MAX_MEASURED: usize = 2_000;

/// Series drawn as bars (primary) and as lines (secondary, combos only).
pub fn split_series(data: &ChartData, combo: bool) -> (Vec<usize>, Vec<usize>) {
    let mut bars = Vec::new();
    let mut lines = Vec::new();
    for (i, s) in data.series.iter().enumerate() {
        if combo && s.secondary {
            lines.push(i);
        } else if !s.secondary {
            bars.push(i);
        }
    }
    (bars, lines)
}

/// One bar segment: value range `[from, to]` in axis units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub series: usize,
    pub category: usize,
    pub from: f64,
    pub to: f64,
    /// The data value (labels).
    pub value: f64,
    /// Slot within the band: `(index, count)` for clustered bars.
    pub slot: (usize, usize),
}

/// Bar segments of `categories` for `series` (clustered: side by side
/// from 0; stacked: positives up from 0 and negatives down; percent:
/// stacked shares of the category's absolute total).
pub fn segments(
    data: &ChartData,
    series: &[usize],
    categories: std::ops::Range<usize>,
    stacking: Stacking,
) -> Vec<Segment> {
    let mut out = Vec::new();
    let k = series.len();
    for i in categories {
        let total: f64 = series
            .iter()
            .filter_map(|s| data.series.get(*s)?.value(i))
            .map(f64::abs)
            .sum();
        let (mut pos, mut neg) = (0.0, 0.0);
        for (slot, s) in series.iter().enumerate() {
            let Some(v) = data.series.get(*s).and_then(|x| x.value(i)) else {
                continue;
            };
            match stacking {
                Stacking::Clustered => out.push(Segment {
                    series: *s,
                    category: i,
                    from: 0.0,
                    to: v,
                    value: v,
                    slot: (slot, k),
                }),
                Stacking::Stacked | Stacking::Percent => {
                    let v2 = if stacking == Stacking::Percent {
                        if total > 0.0 { v / total } else { 0.0 }
                    } else {
                        v
                    };
                    let (from, to) = if v2 >= 0.0 {
                        let r = (pos, pos + v2);
                        pos += v2;
                        r
                    } else {
                        let r = (neg, neg + v2);
                        neg += v2;
                        r
                    };
                    out.push(Segment {
                        series: *s,
                        category: i,
                        from,
                        to,
                        value: v,
                        slot: (0, 1),
                    });
                }
            }
        }
    }
    out
}

/// Value-axis domain of the bars (0 included).
pub fn bar_domain(data: &ChartData, series: &[usize], stacking: Stacking) -> (f64, f64) {
    let segs = segments(data, series, 0..data.len(), stacking);
    let ends = segs.iter().flat_map(|s| [s.from, s.to]);
    let (lo, hi) = scale::value_domain(ends, true);
    if stacking == Stacking::Percent {
        return (lo.clamp(-1.0, 0.0), if hi > 0.0 { 1.0 } else { hi });
    }
    (lo, hi)
}

fn number_style(data: &ChartData, series: &[usize], stacking: Stacking) -> NumberStyle {
    if stacking == Stacking::Percent {
        return NumberStyle {
            percent: true,
            prefix: String::new(),
        };
    }
    let fmt = series
        .first()
        .and_then(|s| data.series.get(*s))
        .and_then(|s| s.format_string.as_deref());
    NumberStyle::from_format(fmt)
}

/// Legend of a bar/column/line chart: shown when there is a series field
/// or more than one series (default position `Top`, title = series field).
pub fn series_legend(cc: &ChartCtx<'_>, colors: &[Color]) -> Option<Legend> {
    let data = cc.data;
    if data.series_meta.is_none() && data.series.len() < 2 {
        return None;
    }
    let items = data
        .series
        .iter()
        .zip(colors)
        .map(|(s, c)| LegendItem {
            label: s.name.clone(),
            color: *c,
        })
        .collect();
    let title = data.series_meta.as_ref().map(|m| m.display_name.clone());
    Legend::read(&cc.props, true, title, items)
}

/// Fill of bar `(series, category)`: a per-category `dataPoint` color for
/// single-series charts, else the series color.
fn bar_color(cc: &ChartCtx<'_>, colors: &[Color], series: usize, category: usize) -> Color {
    if let Some(c) = cc
        .data
        .series
        .get(series)
        .and_then(|s| s.conditional_fill(category))
    {
        return c;
    }
    let base = colors
        .get(series)
        .copied()
        .unwrap_or_else(|| cc.resolver().data_color(series));
    if cc.data.series.len() == 1 {
        cc.data_point_colors()
            .scoped_color(&cc.data.category_fields(category))
            .unwrap_or(base)
    } else {
        base
    }
}

fn push_bar(
    g: &mut Group,
    cc: &ChartCtx<'_>,
    rect: Rect,
    color: Color,
    seg: &Segment,
    opacity: &Option<Vec<f64>>,
) {
    let rect = Rect::new(rect.x, rect.y, rect.width.max(0.0), rect.height.max(0.0));
    let mut bg = Group::new().with_id(cc.hit(seg.series, seg.category));
    if let Some(o) = opacity
        .as_ref()
        .and_then(|o| o.get(seg.category))
        .filter(|o| **o < 1.0)
    {
        bg = bg.with_opacity(*o);
    }
    bg.push(RectNode::new(rect).fill(Paint::solid(color)));
    g.push(bg);
}

/// Renders a horizontal bar chart.
pub fn render_bar(cc: &ChartCtx<'_>, stacking: Stacking) -> Group {
    let mut g = Group::new();
    let data = cc.data;
    let fonts = cc.fonts();
    let resolver = cc.resolver();
    let colors = series_colors(&cc.data_point_colors(), data, resolver);
    let mut area = cc.area;
    if let Some(legend) = series_legend(cc, &colors) {
        area = legend.draw(&mut g, area, fonts).plot;
    }
    if area.is_empty() {
        return g;
    }
    let (series, _) = split_series(data, false);
    let cat = AxisSettings::read(
        &cc.props,
        resolver,
        "categoryAxis",
        data.category_title(),
        false,
    );
    let val = AxisSettings::read(
        &cc.props,
        resolver,
        "valueAxis",
        data.value_title(false),
        true,
    );
    let style = number_style(data, &series, stacking);
    let domain = val.apply_range(bar_domain(data, &series, stacking));
    let n = data.len();

    let title_w = if cat.show && cat.title.is_some() {
        cat.title_style.line_height()
    } else {
        0.0
    };
    let max_label = (area.width * cat.max_margin).max(0.0);
    // The service sizes the label column ~10% wider than the widest label.
    let labels_w = if cat.show {
        (data
            .categories
            .iter()
            .take(MAX_MEASURED)
            .map(|c| width(fonts, c, &cat.label_style))
            .fold(0.0, f64::max)
            * 1.1)
            .min(max_label)
    } else {
        0.0
    };
    let left = title_w + labels_w + if cat.show { CATEGORY_GAP } else { 2.0 };
    let lh_tick = val.label_style.line_height();
    let val_title_h = if val.show && val.title.is_some() {
        val.title_style.line_height()
    } else {
        0.0
    };
    let ticks_h = if val.show { lh_tick + 2.0 } else { 0.0 };
    let top = if val.show { lh_tick * 0.8 } else { 4.0 };
    let plot_y = area.y + top;
    let plot_bottom = (area.bottom() - val_title_h - ticks_h).max(plot_y + 1.0);
    let plot_h = plot_bottom - plot_y;

    // Bands and scrolling.
    let inner = cat.inner_padding;
    let min_step = cat.min_category.max(cat.label_style.line_height() * 1.4);
    let n_f = n.max(1) as f64;
    let scroll = n > 1 && (n_f + inner) * min_step > plot_h + 0.01;
    let (count, step) = if scroll {
        let fit = ((plot_h / min_step) - inner).floor().max(1.0) as usize;
        (fit.min(n), min_step)
    } else {
        (n, plot_h / (n_f + inner))
    };
    let y0 = plot_y + step * inner / 2.0;

    // Right margin: scrollbar, or half of the last tick label.
    let right = if scroll {
        SCROLLBAR_SIZE + 5.0
    } else {
        let last = tick_label(domain.1, 1.0, domain.1.abs(), &style, val.unit);
        (width(fonts, &last, &val.label_style) / 2.0).max(4.0)
    };
    let plot_x = area.x + left;
    let plot_w = (area.right() - right - plot_x).max(1.0);
    let plot = Rect::new(plot_x, plot_y, plot_w, plot_h);
    let x_scale = LinearScale::new(domain, (plot.x, plot.right()));
    let ticks = scale::ticks(domain.0, domain.1, scale::tick_count(plot.width, true));
    let step_v = match (ticks.first(), ticks.get(1)) {
        (Some(a), Some(b)) => b - a,
        _ => domain.1 - domain.0,
    };
    let max_abs = ticks.iter().map(|t| t.abs()).fold(0.0, f64::max);

    // Gridlines, ticks and titles.
    let mut ax = Group::new().with_class("pbi-axes");
    if let Some(stroke) = &val.gridlines {
        for t in &ticks {
            let x = x_scale.map(*t);
            ax.line(x, plot.y, x, plot.bottom(), stroke.clone());
        }
    }
    if val.show {
        let mut last_right = f64::NEG_INFINITY;
        for t in &ticks {
            let text = tick_label(*t, step_v, max_abs, &style, val.unit);
            let x = x_scale.map(*t);
            let w = width(fonts, &text, &val.label_style);
            if x - w / 2.0 < last_right + 2.0 {
                continue;
            }
            last_right = x + w / 2.0;
            ax.text(
                x,
                plot.bottom() + 2.0 + val.label_style.ascent(),
                text,
                val.label_style.clone(),
                TextAnchor::Middle,
            );
        }
        if let Some(title) = &val.title {
            let text = tailor(fonts, title, &val.title_style, plot.width);
            ax.text(
                plot.x + plot.width / 2.0,
                baseline_centered(&val.title_style, area.bottom() - val_title_h / 2.0),
                text,
                val.title_style.clone(),
                TextAnchor::Middle,
            );
        }
    }
    if cat.show {
        if let Some(title) = &cat.title {
            let text = tailor(fonts, title, &cat.title_style, plot.height);
            ax.push(rotated_text(
                &text,
                &cat.title_style,
                area.x + title_w / 2.0,
                plot.y + plot.height / 2.0,
                -90.0,
            ));
        }
        for i in 0..count {
            let Some(label) = data.categories.get(i) else {
                continue;
            };
            let cy = y0 + (i as f64 + 0.5) * step;
            let text = tailor(fonts, label, &cat.label_style, labels_w);
            ax.text(
                plot.x - CATEGORY_GAP,
                baseline_centered(&cat.label_style, cy),
                text,
                cat.label_style.clone(),
                TextAnchor::End,
            );
        }
    }
    if scroll {
        let track = Rect::new(
            area.right() - SCROLLBAR_SIZE,
            plot.y,
            SCROLLBAR_SIZE,
            plot.height,
        );
        scrollbar(&mut ax, track, count as f64 / n.max(1) as f64, 0.0, true);
    }
    g.push(ax);

    // Bars.
    let opacity = cc.selection_opacity();
    let segs = segments(data, &series, 0..count, stacking);
    let mut bars = Group::new().with_class("pbi-bars");
    for seg in &segs {
        let band = y0 + seg.category as f64 * step + step * inner / 2.0;
        let thick = step * (1.0 - inner) / seg.slot.1.max(1) as f64;
        let y = band + thick * seg.slot.0 as f64;
        let (xa, xb) = (x_scale.map_clamped(seg.from), x_scale.map_clamped(seg.to));
        let rect = Rect::new(xa.min(xb), y, (xb - xa).abs(), thick);
        push_bar(
            &mut bars,
            cc,
            rect,
            bar_color(cc, &colors, seg.series, seg.category),
            seg,
            &opacity,
        );
    }
    g.push(bars);

    let labels = DataLabels::read(&cc.props, false);
    if labels.show {
        let max_abs = segs.iter().map(|s| s.value.abs()).fold(0.0, f64::max);
        let mut lg = Group::new().with_class("pbi-data-labels");
        for seg in &segs {
            let Some(s) = data.series.get(seg.series) else {
                continue;
            };
            let text = data_label(
                seg.value,
                s.formatted(seg.category),
                max_abs,
                &NumberStyle::from_format(s.format_at(seg.category)),
                labels.unit,
                labels.precision,
            );
            let band = y0 + seg.category as f64 * step + step * inner / 2.0;
            let thick = step * (1.0 - inner) / seg.slot.1.max(1) as f64;
            let cy = band + thick * (seg.slot.0 as f64 + 0.5);
            let (xa, xb) = (x_scale.map_clamped(seg.from), x_scale.map_clamped(seg.to));
            let color = bar_color(cc, &colors, seg.series, seg.category);
            place_label_h(
                &mut lg,
                fonts,
                &labels,
                &text,
                (xa, xb),
                cy,
                thick,
                plot,
                stacking == Stacking::Clustered,
                color,
            );
        }
        g.push(lg);
    }
    g
}

/// Places a horizontal bar's data label: after the bar end when there is
/// room (clustered), else inside the end (or centred for stacks) with a
/// contrasting color; omitted when it does not fit.
#[allow(clippy::too_many_arguments)]
fn place_label_h(
    g: &mut Group,
    fonts: &crate::text::FontBook,
    labels: &DataLabels,
    text: &str,
    (xa, xb): (f64, f64),
    cy: f64,
    thick: f64,
    plot: Rect,
    outside_first: bool,
    bar: Color,
) {
    let style = &labels.style;
    let w = width(fonts, text, style);
    if style.line_height() > thick * 1.6 + 4.0 {
        return;
    }
    let forward = xb >= xa;
    let len = (xb - xa).abs();
    let baseline = baseline_centered(style, cy);
    let pos = labels.position.as_str();
    let outside_ok = if forward {
        xb + 4.0 + w <= plot.right() + 0.5
    } else {
        xb - 4.0 - w >= plot.x - 0.5
    };
    if outside_first
        && pos != "insideend"
        && pos != "insidecenter"
        && pos != "insidebase"
        && outside_ok
    {
        let (x, anchor) = if forward {
            (xb + 4.0, TextAnchor::Start)
        } else {
            (xb - 4.0, TextAnchor::End)
        };
        g.text(x, baseline, text.to_owned(), style.clone(), anchor);
        return;
    }
    if w + 8.0 > len {
        return;
    }
    let st = inside_style(style, bar);
    let (x, anchor) = if !outside_first || pos == "insidecenter" {
        ((xa + xb) / 2.0, TextAnchor::Middle)
    } else if pos == "insidebase" {
        if forward {
            (xa + 4.0, TextAnchor::Start)
        } else {
            (xa - 4.0, TextAnchor::End)
        }
    } else if forward {
        (xb - 4.0, TextAnchor::End)
    } else {
        (xb + 4.0, TextAnchor::Start)
    };
    g.text(x, baseline, text.to_owned(), st, anchor);
}

fn inside_style(style: &TextStyle, bar: Color) -> TextStyle {
    let mut st = style.clone();
    st.color = if prefers_white_text(bar) {
        Color::WHITE
    } else {
        Color::rgb(0x25, 0x24, 0x23)
    };
    st
}

/// Renders a column chart (vertical bars); with `combo`, `Y2` series are
/// drawn as lines on a secondary axis.
pub fn render_column(cc: &ChartCtx<'_>, stacking: Stacking, combo: bool) -> Group {
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
    let (bars_idx, lines_idx) = split_series(data, combo);
    let domain = bar_domain(data, &bars_idx, stacking);
    let domain2 = (!lines_idx.is_empty()).then(|| {
        let values = lines_idx
            .iter()
            .filter_map(|s| data.series.get(*s))
            .flat_map(|s| s.values.iter().flatten().copied());
        scale::value_domain(values, true)
    });
    let spec = Spec {
        domain,
        domain2,
        style: number_style(data, &bars_idx, stacking),
        style2: number_style(data, &lines_idx, Stacking::Clustered),
        allow_continuous: false,
        bars: true,
    };
    let layout = cartesian::layout(cc, area, &spec);
    layout.draw_axes(cc, &mut g);
    let visible = layout.x.visible(data.len());
    let segs = segments(data, &bars_idx, visible.clone(), stacking);
    let opacity = cc.selection_opacity();
    let mut bars = Group::new().with_class("pbi-bars");
    for seg in &segs {
        let Some((x, w)) = layout.x.bar_slot(seg.category) else {
            continue;
        };
        let thick = w / seg.slot.1.max(1) as f64;
        let x = x + thick * seg.slot.0 as f64;
        let (ya, yb) = (
            layout.y.scale.map_clamped(seg.from),
            layout.y.scale.map_clamped(seg.to),
        );
        let rect = Rect::new(x, ya.min(yb), thick, (yb - ya).abs());
        push_bar(
            &mut bars,
            cc,
            rect,
            bar_color(cc, &colors, seg.series, seg.category),
            seg,
            &opacity,
        );
    }
    g.push(bars);
    let labels = DataLabels::read(&cc.props, false);
    if labels.show {
        let max_abs = segs.iter().map(|s| s.value.abs()).fold(0.0, f64::max);
        let mut lg = Group::new().with_class("pbi-data-labels");
        for seg in &segs {
            let (Some(s), Some((x, w))) =
                (data.series.get(seg.series), layout.x.bar_slot(seg.category))
            else {
                continue;
            };
            let thick = w / seg.slot.1.max(1) as f64;
            let cx = x + thick * (seg.slot.0 as f64 + 0.5);
            let text = data_label(
                seg.value,
                s.formatted(seg.category),
                max_abs,
                &NumberStyle::from_format(s.format_at(seg.category)),
                labels.unit,
                labels.precision,
            );
            let tw = width(fonts, &text, &labels.style);
            if tw > thick + 6.0 {
                continue;
            }
            let (ya, yb) = (
                layout.y.scale.map_clamped(seg.from),
                layout.y.scale.map_clamped(seg.to),
            );
            let lh = labels.style.line_height();
            let up = yb <= ya;
            let clustered = stacking == Stacking::Clustered;
            let outside_y = if up {
                yb - 2.0 - lh / 2.0
            } else {
                yb + 2.0 + lh / 2.0
            };
            let fits_outside = outside_y - lh / 2.0 >= layout.plot.y - 0.5
                && outside_y + lh / 2.0 <= layout.plot.bottom() + 0.5;
            let (cy, st) = if clustered && fits_outside && !labels.position.starts_with("inside") {
                (outside_y, labels.style.clone())
            } else if (ya - yb).abs() >= lh + 2.0 {
                let color = bar_color(cc, &colors, seg.series, seg.category);
                let cy = if clustered {
                    if up {
                        yb + 2.0 + lh / 2.0
                    } else {
                        yb - 2.0 - lh / 2.0
                    }
                } else {
                    (ya + yb) / 2.0
                };
                (cy, inside_style(&labels.style, color))
            } else {
                continue;
            };
            lg.text(cx, baseline_centered(&st, cy), text, st, TextAnchor::Middle);
        }
        g.push(lg);
    }
    if !lines_idx.is_empty() {
        let scale = layout.y2.as_ref().map_or(layout.y.scale, |a| a.scale);
        g.push(line::draw_series(
            cc, &layout, &lines_idx, &colors, scale, false, false,
        ));
    }
    g
}
