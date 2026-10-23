//! Waterfall charts (`waterfallChart`).
//!
//! Roles: `Category`, `Breakdown` (optional, a series field), `Y` (the
//! first value measure; `Tooltips` are ignored). Categories keep the
//! DataView order, which is ascending by category by default (as the
//! service).
//!
//! - **Without a breakdown** each category is a floating column from the
//!   running total before it to the running total after it (increase or
//!   decrease), and a final "Total" column goes from 0 to the sum.
//! - **With a breakdown** (as the service): the first category is a total
//!   column (its value: the sum over the breakdown values), then each
//!   following category shows the change from the previous one split by
//!   breakdown value — the `breakdown.maxBreakdowns` (default 5) largest
//!   changes by magnitude plus one "Other" column for the rest, ordered
//!   from the largest increase to the largest decrease — followed by the
//!   category's own total column. There is no extra grand total: the last
//!   category's column ends the chart.
//!
//! Colors come from `sentimentColors` (`increaseFill`, `decreaseFill`,
//! `totalFill`, `otherFill`; theme `visualStyles` apply) or the theme:
//! increase `good`, decrease `bad`, other `neutral`, total `dataColors[0]`
//! (green, red, yellow and blue with the default theme). Thin connector
//! lines join each column's end to the next column. The legend (Increase,
//! Decrease, Total, Other) is on by default; axes, gridlines, category
//! label wrapping/slanting, scrolling and data labels (off by default) are
//! the column chart's ([`super::cartesian`]).
//!
//! Hit ids: a category column is `DataPoint{0, category}` without a
//! breakdown; with one, a breakdown change is `DataPoint{breakdown series,
//! category}` (the category it leads to) and total / "Other" columns use
//! `series = ChartData::series.len()` (the category only). The grand total
//! has none.

use crate::scene::{Color, Group, Paint, Rect, RectNode, Stroke, TextAnchor};

use super::cartesian::{self, Spec};
use super::format::{NumberStyle, data_label};
use super::labels::{DataLabels, baseline_centered, prefers_white_text, width};
use super::legend::{Legend, LegendItem};
use super::scale;
use super::{ChartCtx, ChartData};

/// Default `breakdown.maxBreakdowns`.
pub const DEFAULT_MAX_BREAKDOWNS: usize = 5;
/// Label of the grand total column.
pub const TOTAL_LABEL: &str = "Total";
/// Label of the column grouping the smaller breakdown changes.
pub const OTHER_LABEL: &str = "Other";

/// What a column shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepKind {
    Increase,
    Decrease,
    Total,
    Other,
}

/// One waterfall column, in axis units.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub label: String,
    pub kind: StepKind,
    pub from: f64,
    pub to: f64,
    /// The change (or the total) the column stands for.
    pub value: f64,
    /// The DataView's text for the value (empty when computed).
    pub formatted: String,
    /// No value (BLANK): the slot stays empty.
    pub blank: bool,
    /// Hit id `(series, index)`; `None` for the grand total.
    pub hit: Option<(usize, usize)>,
    /// Category row (selection dimming).
    pub category: Option<usize>,
}

fn step(label: String, kind: StepKind, from: f64, to: f64) -> Step {
    Step {
        label,
        kind,
        from,
        to,
        value: to - from,
        formatted: String::new(),
        blank: false,
        hit: None,
        category: None,
    }
}

fn change_kind(v: f64) -> StepKind {
    if v < 0.0 {
        StepKind::Decrease
    } else {
        StepKind::Increase
    }
}

/// The columns of a waterfall (see the module docs).
pub fn steps(data: &ChartData, max_breakdowns: usize) -> Vec<Step> {
    let n = data.len();
    let Some(first) = data.series.first() else {
        return Vec::new();
    };
    if n == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    if data.series_meta.is_none() {
        let mut running = 0.0;
        for (i, label) in data.categories.iter().enumerate() {
            match first.value(i) {
                Some(v) if (running + v).is_finite() => {
                    out.push(Step {
                        formatted: first.formatted(i).to_owned(),
                        hit: Some((0, i)),
                        category: Some(i),
                        value: v,
                        ..step(label.clone(), change_kind(v), running, running + v)
                    });
                    running += v;
                }
                _ => out.push(Step {
                    blank: true,
                    hit: Some((0, i)),
                    category: Some(i),
                    ..step(label.clone(), StepKind::Increase, running, running)
                }),
            }
        }
        out.push(step(TOTAL_LABEL.to_owned(), StepKind::Total, 0.0, running));
        return out;
    }
    // Breakdown: category totals joined by per-breakdown changes.
    let pillar = data.series.len();
    let level = |k: usize| -> f64 {
        let sum: f64 = data.series.iter().filter_map(|s| s.value(k)).sum();
        if sum.is_finite() { sum } else { 0.0 }
    };
    let total_step = |k: usize, label: &String| Step {
        hit: Some((pillar, k)),
        category: Some(k),
        ..step(label.clone(), StepKind::Total, 0.0, level(k))
    };
    for (k, label) in data.categories.iter().enumerate() {
        if k > 0 {
            let mut changes: Vec<(usize, f64)> = data
                .series
                .iter()
                .enumerate()
                .map(|(s, sd)| {
                    let d = sd.value(k).unwrap_or(0.0) - sd.value(k - 1).unwrap_or(0.0);
                    (s, d)
                })
                .filter(|(_, d)| d.is_finite() && *d != 0.0)
                .collect();
            changes.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()).then(a.0.cmp(&b.0)));
            let rest: f64 = changes.iter().skip(max_breakdowns).map(|c| c.1).sum();
            let has_rest = changes.len() > max_breakdowns;
            changes.truncate(max_breakdowns);
            // `None`: the "Other" column.
            let mut items: Vec<(Option<usize>, f64)> =
                changes.into_iter().map(|(s, d)| (Some(s), d)).collect();
            if has_rest && rest.is_finite() && rest != 0.0 {
                items.push((None, rest));
            }
            items.sort_by(|a, b| b.1.total_cmp(&a.1));
            let mut running = level(k - 1);
            for (s, d) in items {
                let (label, kind, hit) = match s {
                    Some(s) => {
                        let sd = data.series.get(s);
                        let label = sd
                            .and_then(|sd| {
                                sd.series_label.clone().or_else(|| Some(sd.name.clone()))
                            })
                            .unwrap_or_default();
                        (label, change_kind(d), (s, k))
                    }
                    None => (OTHER_LABEL.to_owned(), StepKind::Other, (pillar, k)),
                };
                out.push(Step {
                    hit: Some(hit),
                    category: Some(k),
                    ..step(label, kind, running, running + d)
                });
                running += d;
            }
        }
        out.push(total_step(k, label));
    }
    out
}

/// Column colors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SentimentColors {
    pub increase: Color,
    pub decrease: Color,
    pub total: Color,
    pub other: Color,
}

impl SentimentColors {
    /// `sentimentColors.*Fill`, else the theme's `good`, `bad`,
    /// `dataColors[0]` and `neutral`.
    pub fn read(cc: &ChartCtx<'_>) -> Self {
        let o = cc.objects();
        let r = cc.resolver();
        let named = |name: &str, fallback: Color| r.named_color(name).unwrap_or(fallback);
        Self {
            increase: o
                .color("sentimentColors", "increaseFill")
                .unwrap_or_else(|| named("good", Color::rgb(0x1A, 0xAB, 0x40))),
            decrease: o
                .color("sentimentColors", "decreaseFill")
                .unwrap_or_else(|| named("bad", Color::rgb(0xD6, 0x45, 0x54))),
            total: o
                .color("sentimentColors", "totalFill")
                .unwrap_or_else(|| r.data_color(0)),
            other: o
                .color("sentimentColors", "otherFill")
                .unwrap_or_else(|| named("neutral", Color::rgb(0xD9, 0xB3, 0x00))),
        }
    }

    pub fn of(&self, kind: StepKind) -> Color {
        match kind {
            StepKind::Increase => self.increase,
            StepKind::Decrease => self.decrease,
            StepKind::Total => self.total,
            StepKind::Other => self.other,
        }
    }
}

/// Renders a waterfall chart.
pub fn render(cc: &ChartCtx<'_>) -> Group {
    let mut g = Group::new();
    let data = cc.data;
    let max_breakdowns = cc
        .objects()
        .number("breakdown", "maxBreakdowns")
        .filter(|v| v.is_finite() && *v >= 1.0)
        .map_or(DEFAULT_MAX_BREAKDOWNS, |v| v.min(1_000.0) as usize);
    let steps = steps(data, max_breakdowns);
    if steps.is_empty() {
        return g;
    }
    let colors = SentimentColors::read(cc);
    let fonts = cc.fonts();
    let mut area = cc.area;
    let mut items = vec![
        LegendItem {
            label: "Increase".to_owned(),
            color: colors.increase,
        },
        LegendItem {
            label: "Decrease".to_owned(),
            color: colors.decrease,
        },
        LegendItem {
            label: TOTAL_LABEL.to_owned(),
            color: colors.total,
        },
    ];
    if steps.iter().any(|s| s.kind == StepKind::Other) {
        items.push(LegendItem {
            label: OTHER_LABEL.to_owned(),
            color: colors.other,
        });
    }
    if let Some(legend) = Legend::read(&cc.props, true, None, items) {
        area = legend.draw(&mut g, area, fonts).plot;
    }
    if area.is_empty() {
        return g;
    }
    // The columns as a category axis for the shared cartesian layout.
    let columns = ChartData {
        categories: steps.iter().map(|s| s.label.clone()).collect(),
        category_values: vec![Vec::new(); steps.len()],
        category_metas: data.category_metas.clone(),
        series: data.series.iter().take(1).cloned().collect(),
        series_meta: None,
        truncated: data.truncated,
    };
    let wcc = ChartCtx {
        data: &columns,
        ..*cc
    };
    let style =
        NumberStyle::from_format(data.series.first().and_then(|s| s.format_string.as_deref()));
    let spec = Spec {
        domain: scale::value_domain(steps.iter().flat_map(|s| [s.from, s.to]), true),
        domain2: None,
        style: style.clone(),
        style2: NumberStyle::default(),
        allow_continuous: false,
        bars: true,
    };
    let layout = cartesian::layout(&wcc, area, &spec);
    layout.draw_axes(&wcc, &mut g);
    let y = |v: f64| layout.y.scale.map_clamped(v);
    let opacity = cc.selection_opacity();
    let connector = Stroke::new(
        cc.resolver()
            .named_color("foregroundNeutralSecondary")
            .unwrap_or(Color::rgb(0x60, 0x5E, 0x5C)),
        1.0,
    );
    let mut connectors = Group::new().with_class("pbi-connectors");
    let mut bars = Group::new().with_class("pbi-bars");
    let visible = layout.x.visible(steps.len());
    let mut previous: Option<(f64, f64)> = None;
    for j in visible.clone() {
        let (Some(s), Some((x, w))) = (steps.get(j), layout.x.bar_slot(j)) else {
            continue;
        };
        if let Some((px, level)) = previous {
            let ly = y(level);
            connectors.line(px, ly, x, ly, connector.clone());
        }
        previous = Some((x + w, s.to));
        if s.blank {
            continue;
        }
        let (ya, yb) = (y(s.from), y(s.to));
        let rect = Rect::new(x, ya.min(yb), w.max(0.0), (yb - ya).abs());
        let mut col = Group::new();
        if let Some((series, index)) = s.hit {
            col = col.with_id(cc.hit(series, index));
        }
        if let Some(op) = s
            .category
            .and_then(|c| opacity.as_ref().and_then(|o| o.get(c)))
            .filter(|o| **o < 1.0)
        {
            col = col.with_opacity(*op);
        }
        col.push(RectNode::new(rect).fill(Paint::solid(colors.of(s.kind))));
        bars.push(col);
    }
    g.push(connectors);
    g.push(bars);

    let labels = DataLabels::read(&cc.props, false);
    if labels.show {
        let max_abs = steps.iter().map(|s| s.value.abs()).fold(0.0, f64::max);
        let lh = labels.style.line_height();
        let mut lg = Group::new().with_class("pbi-data-labels");
        for j in visible {
            let (Some(s), Some((x, w))) = (steps.get(j), layout.x.bar_slot(j)) else {
                continue;
            };
            if s.blank {
                continue;
            }
            let text = data_label(
                s.value,
                &s.formatted,
                max_abs,
                &style,
                labels.unit,
                labels.precision,
            );
            if text.is_empty() || width(fonts, &text, &labels.style) > w + 6.0 {
                continue;
            }
            let (top, bottom) = (y(s.from.max(s.to)), y(s.from.min(s.to)));
            let up = s.value >= 0.0;
            let outside = if up {
                top - 2.0 - lh / 2.0
            } else {
                bottom + 2.0 + lh / 2.0
            };
            let fits = outside - lh / 2.0 >= layout.plot.y - 0.5
                && outside + lh / 2.0 <= layout.plot.bottom() + 0.5;
            let (cy, st) = if fits {
                (outside, labels.style.clone())
            } else if bottom - top >= lh + 2.0 {
                let mut st = labels.style.clone();
                st.color = if prefers_white_text(colors.of(s.kind)) {
                    Color::WHITE
                } else {
                    Color::rgb(0x25, 0x24, 0x23)
                };
                ((top + bottom) / 2.0, st)
            } else {
                continue;
            };
            lg.text(
                x + w / 2.0,
                baseline_centered(&st, cy),
                text,
                st,
                TextAnchor::Middle,
            );
        }
        g.push(lg);
    }
    g
}
