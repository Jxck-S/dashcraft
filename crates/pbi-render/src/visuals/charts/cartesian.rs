//! Layout of charts with the category axis at the bottom (column, line,
//! area, combo): value axis on the left (and a secondary one on the right
//! for combos), category bands or a continuous x scale, category label
//! wrapping/rotation and horizontal scrolling.
//!
//! - Bands: `step = width / (n + innerPadding)` with the bars inset by
//!   half the padding on each side; when `n` bands of
//!   `preferredCategoryWidth` (20px) do not fit, only the first ones that
//!   fit are drawn and a scrollbar is shown.
//! - Labels: one line when they fit the band, else wrapped on two lines,
//!   else slanted −35° (−90° when scrolling) and truncated to
//!   `maxMarginFactor` (25%) of the visual height.
//! - A continuous axis (dates or numbers on a line/area chart unless
//!   `categoryAxis.axisType` is `Categorical`) uses [`scale::time_ticks`]
//!   or nice numeric ticks.

use pbi_engine::Value;

use crate::scene::{Group, Rect, TextAnchor};
use crate::text::{FontBook, MeasureOptions};

use super::axis::{AxisSettings, SCROLLBAR_SIZE, scrollbar};
use super::format::{NumberStyle, group_number, tick_label};
use super::labels::{baseline_centered, rotated_text, slanted_text, tailor, width};
use super::scale::{self, LinearScale};
use super::{ChartCtx, ContinuousKind};

/// Gap between value tick labels and the plot.
const TICK_GAP: f64 = 6.0;
/// Most category labels measured for layout decisions.
const MAX_MEASURED: usize = 2_000;

/// How category labels are drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LabelMode {
    Horizontal,
    /// Up to two lines.
    Wrapped,
    /// Slanted by this many degrees (negative: counter-clockwise).
    Rotated(f64),
}

/// The x axis.
#[derive(Debug, Clone, PartialEq)]
pub enum XAxis {
    /// Category bands: categories `first..first + count` are visible.
    Bands {
        first: usize,
        count: usize,
        /// Left edge of the first visible band.
        x0: f64,
        step: f64,
        /// Gap between bars as a fraction of the step.
        inner: f64,
        mode: LabelMode,
        /// Total number of categories (scrollbar).
        total: usize,
    },
    /// A continuous scale (category values as numbers / OLE dates).
    Continuous {
        scale: LinearScale,
        kind: ContinuousKind,
        ticks: Vec<(f64, String)>,
    },
}

impl XAxis {
    /// Centre x of category `i`, if visible.
    pub fn center(&self, cc: &ChartCtx<'_>, i: usize) -> Option<f64> {
        match self {
            XAxis::Bands {
                first,
                count,
                x0,
                step,
                ..
            } => (i >= *first && i < first + count).then(|| x0 + ((i - first) as f64 + 0.5) * step),
            XAxis::Continuous { scale, .. } => cc.data.category_number(i).map(|v| scale.map(v)),
        }
    }

    /// Visible category indices.
    pub fn visible(&self, n: usize) -> std::ops::Range<usize> {
        match self {
            XAxis::Bands { first, count, .. } => *first..(first + count).min(n),
            XAxis::Continuous { .. } => 0..n,
        }
    }

    /// Bar slot `(x, width)` of category `i` (bands only).
    pub fn bar_slot(&self, i: usize) -> Option<(f64, f64)> {
        match self {
            XAxis::Bands {
                first,
                count,
                x0,
                step,
                inner,
                ..
            } if i >= *first && i < first + count => {
                let band = x0 + (i - first) as f64 * step;
                Some((band + step * inner / 2.0, step * (1.0 - inner)))
            }
            _ => None,
        }
    }
}

/// A value axis: scale, ticks and their labels.
#[derive(Debug, Clone, PartialEq)]
pub struct ValueAxis {
    pub settings: AxisSettings,
    pub scale: LinearScale,
    pub ticks: Vec<f64>,
    pub labels: Vec<String>,
}

/// What [`layout`] needs from the chart.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    /// Primary data domain.
    pub domain: (f64, f64),
    /// Secondary (combo line) data domain.
    pub domain2: Option<(f64, f64)>,
    pub style: NumberStyle,
    pub style2: NumberStyle,
    /// Lines/areas may use a continuous axis.
    pub allow_continuous: bool,
    /// Bands have an inner padding (bars) or none (points).
    pub bars: bool,
}

/// The computed layout.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub area: Rect,
    pub plot: Rect,
    pub category: AxisSettings,
    pub x: XAxis,
    pub y: ValueAxis,
    pub y2: Option<ValueAxis>,
    /// Height of the category label row.
    label_row: f64,
    scroll: bool,
}

fn value_axis(
    settings: AxisSettings,
    domain: (f64, f64),
    style: &NumberStyle,
    h: f64,
) -> ValueAxis {
    let domain = settings.apply_range(domain);
    let ticks = scale::ticks(domain.0, domain.1, scale::tick_count(h, false));
    let step = match (ticks.first(), ticks.get(1)) {
        (Some(a), Some(b)) => b - a,
        _ => (domain.1 - domain.0).abs(),
    };
    let max_abs = ticks.iter().map(|t| t.abs()).fold(0.0, f64::max);
    let labels = ticks
        .iter()
        .map(|t| tick_label(*t, step, max_abs, style, settings.unit))
        .collect();
    ValueAxis {
        settings,
        scale: LinearScale::new(domain, (0.0, 1.0)),
        ticks,
        labels,
    }
}

fn labels_width(fonts: &FontBook, axis: &ValueAxis) -> f64 {
    if !axis.settings.show {
        return 0.0;
    }
    axis.labels
        .iter()
        .map(|l| width(fonts, l, &axis.settings.label_style))
        .fold(0.0, f64::max)
}

/// Lays out the axes inside `area`.
pub fn layout(cc: &ChartCtx<'_>, area: Rect, spec: &Spec) -> Layout {
    let fonts = cc.fonts();
    let data = cc.data;
    let resolver = cc.resolver();
    let category = AxisSettings::read(
        &cc.props,
        resolver,
        "categoryAxis",
        data.category_title(),
        false,
    );
    let val_settings = AxisSettings::read(
        &cc.props,
        resolver,
        "valueAxis",
        data.value_title(false),
        true,
    );
    let y2_settings = spec
        .domain2
        .map(|_| AxisSettings::read(&cc.props, resolver, "y2Axis", data.value_title(true), false));
    let lh_val = val_settings.label_style.line_height();
    let lh_cat = category.label_style.line_height();
    let title_h = |a: &AxisSettings| {
        if a.show && a.title.is_some() {
            a.title_style.line_height()
        } else {
            0.0
        }
    };
    let top = if val_settings.show {
        lh_val / 2.0 + 2.0
    } else {
        4.0
    };
    let bottom_title = title_h(&category);
    let left_title = title_h(&val_settings);
    let right_title = y2_settings.as_ref().map_or(0.0, title_h);
    // First pass: ticks for an estimated plot height.
    let est_h = (area.height - top - bottom_title - lh_cat - 4.0).max(1.0);
    let y_est = value_axis(val_settings.clone(), spec.domain, &spec.style, est_h);
    let left_w = left_title
        + if val_settings.show {
            labels_width(fonts, &y_est) + TICK_GAP
        } else {
            4.0
        };
    let right_w = match (&y2_settings, spec.domain2) {
        (Some(s), Some(d)) if s.show => {
            let y2_est = value_axis(s.clone(), d, &spec.style2, est_h);
            right_title + labels_width(fonts, &y2_est) + TICK_GAP
        }
        _ => 8.0,
    };
    let plot_x = area.x + left_w;
    let plot_w = (area.width - left_w - right_w).max(1.0);
    let n = data.len();
    let continuous = spec.allow_continuous && !category.categorical;
    let continuous_kind = if continuous {
        data.continuous_kind()
    } else {
        None
    };
    let (x, label_row, scroll) = match continuous_kind {
        Some(kind) => {
            let x = continuous_axis(cc, kind, plot_x, plot_w);
            (x, if category.show { lh_cat + 2.0 } else { 0.0 }, false)
        }
        None => bands(cc, &category, spec, plot_x, plot_w, area, n),
    };
    let scroll_h = if scroll { SCROLLBAR_SIZE + 2.0 } else { 0.0 };
    let plot_y = area.y + top;
    let plot_bottom = (area.bottom() - bottom_title - label_row - scroll_h).max(plot_y + 1.0);
    let plot = Rect::new(plot_x, plot_y, plot_w, plot_bottom - plot_y);
    let mut y = value_axis(val_settings, spec.domain, &spec.style, plot.height);
    y.scale = LinearScale::new((y.scale.d0, y.scale.d1), (plot.bottom(), plot.y));
    let y2 = match (y2_settings, spec.domain2) {
        (Some(s), Some(d)) => {
            let mut a = value_axis(s, d, &spec.style2, plot.height);
            a.scale = LinearScale::new((a.scale.d0, a.scale.d1), (plot.bottom(), plot.y));
            Some(a)
        }
        _ => None,
    };
    Layout {
        area,
        plot,
        category,
        x,
        y,
        y2,
        label_row,
        scroll,
    }
}

fn continuous_axis(cc: &ChartCtx<'_>, kind: ContinuousKind, plot_x: f64, plot_w: f64) -> XAxis {
    let nums: Vec<f64> = (0..cc.data.len())
        .filter_map(|i| cc.data.category_number(i))
        .collect();
    let mut lo = nums.iter().copied().fold(f64::INFINITY, f64::min);
    let mut hi = nums.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !lo.is_finite() || !hi.is_finite() {
        lo = 0.0;
        hi = 1.0;
    }
    if hi - lo <= 0.0 {
        lo -= 1.0;
        hi += 1.0;
    }
    let count = ((plot_w / 90.0).floor() as usize).clamp(2, 12);
    let ticks: Vec<(f64, String)> = match kind {
        ContinuousKind::Date => scale::time_ticks(lo, hi, count)
            .into_iter()
            .map(|t| {
                let v = Value::from_oadate(t.serial).unwrap_or(Value::Blank);
                (
                    t.serial,
                    pbi_dax::format_value(&v, Some(t.unit.format()), "en-US"),
                )
            })
            .collect(),
        ContinuousKind::Number => {
            let ticks = scale::ticks(lo, hi, count);
            let step = match (ticks.first(), ticks.get(1)) {
                (Some(a), Some(b)) => b - a,
                _ => 1.0,
            };
            let d = super::format::decimals_for_step(step);
            ticks.into_iter().map(|t| (t, group_number(t, d))).collect()
        }
    };
    XAxis::Continuous {
        scale: LinearScale::new((lo, hi), (plot_x, plot_x + plot_w)),
        kind,
        ticks,
    }
}

/// Category bands, label mode and label row height.
fn bands(
    cc: &ChartCtx<'_>,
    category: &AxisSettings,
    spec: &Spec,
    plot_x: f64,
    plot_w: f64,
    area: Rect,
    n: usize,
) -> (XAxis, f64, bool) {
    let fonts = cc.fonts();
    let style = &category.label_style;
    let lh = style.line_height();
    let inner = if spec.bars {
        category.inner_padding
    } else {
        0.0
    };
    let min_step = category.min_category;
    let n_f = n.max(1) as f64;
    let scroll = (n_f + inner) * min_step > plot_w + 0.01 && n > 1;
    let (count, step) = if scroll {
        let fit = ((plot_w / min_step) - inner).floor().max(1.0) as usize;
        (fit.min(n), min_step)
    } else {
        (n, plot_w / (n_f + inner))
    };
    let x0 = plot_x + step * inner / 2.0;
    let visible: Vec<&String> = cc
        .data
        .categories
        .iter()
        .take(count.min(MAX_MEASURED))
        .collect();
    let avail = (step - 2.0).max(1.0);
    let max_w = visible
        .iter()
        .map(|t| width(fonts, t, style))
        .fold(0.0, f64::max);
    let (mode, row) = if !category.show {
        (LabelMode::Horizontal, 0.0)
    } else if max_w <= avail {
        (LabelMode::Horizontal, lh + 2.0)
    } else if visible.iter().all(|t| {
        let opts = MeasureOptions {
            max_width: Some(avail),
            wrap: true,
            max_lines: None,
            ellipsis: false,
        };
        let m = fonts.measure_with(t, style, &opts);
        m.lines.len() <= 2 && m.width <= avail + 0.01
    }) {
        (LabelMode::Wrapped, 2.0 * lh + 2.0)
    } else {
        let deg: f64 = if scroll { -90.0 } else { -35.0 };
        let (s, c) = deg.abs().to_radians().sin_cos();
        let max_len = (area.height * category.max_margin / s.max(0.1)).max(lh);
        let len = max_w.min(max_len);
        (LabelMode::Rotated(deg), len * s + lh * c + 4.0)
    };
    (
        XAxis::Bands {
            first: 0,
            count,
            x0,
            step,
            inner,
            mode,
            total: n,
        },
        row,
        scroll,
    )
}

impl Layout {
    /// Draws gridlines, tick labels, category labels, titles and the
    /// scrollbar (call before the marks so gridlines stay behind).
    pub fn draw_axes(&self, cc: &ChartCtx<'_>, g: &mut Group) {
        let fonts = cc.fonts();
        let plot = self.plot;
        let mut ax = Group::new().with_class("pbi-axes");
        if let Some(stroke) = &self.y.settings.gridlines {
            for t in &self.y.ticks {
                let y = self.y.scale.map(*t);
                ax.line(plot.x, y, plot.right(), y, stroke.clone());
            }
        }
        if let Some(stroke) = &self.category.gridlines {
            for i in self.x.visible(cc.data.len()) {
                if let Some(x) = self.x.center(cc, i) {
                    ax.line(x, plot.y, x, plot.bottom(), stroke.clone());
                }
            }
        }
        draw_value_labels(&mut ax, &self.y, plot.x - TICK_GAP, TextAnchor::End);
        if let Some(y2) = &self.y2 {
            draw_value_labels(&mut ax, y2, plot.right() + TICK_GAP, TextAnchor::Start);
        }
        // Value axis titles (rotated).
        let s = &self.y.settings;
        if let (true, Some(title)) = (s.show, &s.title) {
            let lh = s.title_style.line_height();
            let text = tailor(fonts, title, &s.title_style, plot.height);
            ax.push(rotated_text(
                &text,
                &s.title_style,
                self.area.x + lh / 2.0,
                plot.y + plot.height / 2.0,
                -90.0,
            ));
        }
        if let Some(y2) = &self.y2 {
            let s = &y2.settings;
            if let (true, Some(title)) = (s.show, &s.title) {
                let lh = s.title_style.line_height();
                let text = tailor(fonts, title, &s.title_style, plot.height);
                ax.push(rotated_text(
                    &text,
                    &s.title_style,
                    self.area.right() - lh / 2.0,
                    plot.y + plot.height / 2.0,
                    90.0,
                ));
            }
        }
        self.draw_category_labels(cc, &mut ax);
        let c = &self.category;
        if let (true, Some(title)) = (c.show, &c.title) {
            let lh = c.title_style.line_height();
            let text = tailor(fonts, title, &c.title_style, plot.width);
            ax.text(
                plot.x + plot.width / 2.0,
                baseline_centered(&c.title_style, self.area.bottom() - lh / 2.0),
                text,
                c.title_style.clone(),
                TextAnchor::Middle,
            );
        }
        if self.scroll
            && let XAxis::Bands { count, total, .. } = &self.x
        {
            let y = plot.bottom() + self.label_row;
            let track = Rect::new(plot.x, y, plot.width, SCROLLBAR_SIZE);
            scrollbar(
                &mut ax,
                track,
                *count as f64 / (*total).max(1) as f64,
                0.0,
                false,
            );
        }
        g.push(ax);
    }

    fn draw_category_labels(&self, cc: &ChartCtx<'_>, ax: &mut Group) {
        let c = &self.category;
        if !c.show {
            return;
        }
        let fonts = cc.fonts();
        let style = &c.label_style;
        let lh = style.line_height();
        let top = self.plot.bottom() + 2.0;
        match &self.x {
            XAxis::Continuous { scale, ticks, .. } => {
                let mut last_right = f64::NEG_INFINITY;
                for (v, text) in ticks {
                    let x = scale.map(*v);
                    let w = width(fonts, text, style);
                    if x - w / 2.0 < last_right + 4.0 {
                        continue;
                    }
                    last_right = x + w / 2.0;
                    ax.text(
                        x,
                        top + style.ascent(),
                        text.clone(),
                        style.clone(),
                        TextAnchor::Middle,
                    );
                }
            }
            XAxis::Bands {
                first,
                count,
                step,
                mode,
                ..
            } => {
                let avail = (step - 2.0).max(1.0);
                let skip = match mode {
                    // Thin out labels that would overlap when slanted.
                    LabelMode::Rotated(_) => (lh * 0.9 / step).ceil().max(1.0) as usize,
                    _ => 1,
                };
                for i in (*first..(first + count).min(cc.data.len())).step_by(skip) {
                    let (Some(x), Some(text)) = (self.x.center(cc, i), cc.data.categories.get(i))
                    else {
                        continue;
                    };
                    match mode {
                        LabelMode::Horizontal => {
                            let t = tailor(fonts, text, style, avail);
                            ax.text(
                                x,
                                top + style.ascent(),
                                t,
                                style.clone(),
                                TextAnchor::Middle,
                            );
                        }
                        LabelMode::Wrapped => {
                            let opts = MeasureOptions {
                                max_width: Some(avail),
                                wrap: true,
                                max_lines: Some(2),
                                ellipsis: true,
                            };
                            let m = fonts.measure_with(text, style, &opts);
                            for node in m.to_nodes(x, top, TextAnchor::Middle, style) {
                                ax.push(node);
                            }
                        }
                        LabelMode::Rotated(deg) => {
                            let (s, _) = deg.abs().to_radians().sin_cos();
                            let max_len = (self.label_row - 4.0).max(lh) / s.max(0.1);
                            let t = tailor(fonts, text, style, max_len);
                            ax.push(slanted_text(&t, style, x + lh * 0.3, top + 2.0, *deg));
                        }
                    }
                }
            }
        }
    }
}

fn draw_value_labels(ax: &mut Group, axis: &ValueAxis, x: f64, anchor: TextAnchor) {
    if !axis.settings.show {
        return;
    }
    let style = &axis.settings.label_style;
    let lh = style.line_height();
    let mut last: Option<f64> = None;
    for (t, label) in axis.ticks.iter().zip(&axis.labels) {
        let y = axis.scale.map(*t);
        if last.is_some_and(|l| (l - y).abs() < lh * 0.9) {
            continue;
        }
        last = Some(y);
        ax.text(
            x,
            baseline_centered(style, y),
            label.clone(),
            style.clone(),
            anchor,
        );
    }
}
