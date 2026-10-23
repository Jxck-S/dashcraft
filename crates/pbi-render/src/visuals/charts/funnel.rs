//! Funnel charts (`funnel`).
//!
//! One horizontal bar per category (`Category` role, value: the first `Y`
//! / `Values` measure), or one per measure when there is no category. The
//! stages keep the DataView order (the query sorts funnels by value,
//! descending, unless the visual has a sort), top to bottom; each bar is
//! centred and as wide as its value relative to the largest magnitude
//! (BLANK and zero stages keep their row and label but draw no bar).
//!
//! As in the service:
//!
//! - category labels on the left (`categoryAxis`: `show`, `labelColor`,
//!   `fontSize`, `fontFamily`; 9pt `#605E5C`), truncated with `...` to at
//!   most 30% of the width;
//! - bars fill 90% of their band (at least 1.4 label lines tall: stages
//!   that do not fit are cut and a scrollbar is drawn);
//! - one color for every bar (`dataPoint.defaultColor`, else
//!   `dataColors[0]`), per-stage `dataPoint.fill` overrides;
//! - data labels on by default (`labels`: `show`, `color`, `fontSize`,
//!   `fontFamily`, `labelDisplayUnits`, `labelPrecision`, `labelPosition`
//!   `insideCenter` (default) / `outside`, and the contents in
//!   `funnelLabelStyle` or `labelStyle`: data value (default), percent of
//!   first, percent of previous, combinations, all), centred inside the
//!   bar in a contrasting color, else right of it;
//! - the conversion-rate label (`percentBarLabel.show`, default on): `100%`
//!   over a bracket spanning the first bar, and the last stage as a percent
//!   of the first under a bracket spanning the last bar.
//!
//! Hit ids: `DataPoint{0, category}` (`DataPoint{measure, 0}` for a
//! funnel of measures).

use crate::scene::{
    Color, Group, Paint, PathBuilder, PathNode, Rect, RectNode, Stroke, TextAnchor,
};
use crate::text::TextStyle;

use super::ChartCtx;
use super::axis::{AxisSettings, SCROLLBAR_SIZE, scrollbar};
use super::format::{NumberStyle, data_label};
use super::labels::{DataLabels, baseline_centered, prefers_white_text, tailor, width};

/// Share of a band filled by its bar.
const BAR_FILL: f64 = 0.9;
/// Gap between category labels and the widest bar.
const LABEL_GAP: f64 = 8.0;
/// Height of a conversion-rate bracket.
const BRACKET: f64 = 6.0;
/// Bracket and percent-label row spacing.
const BRACKET_GAP: f64 = 3.0;

/// One funnel stage.
#[derive(Debug, Clone, PartialEq)]
pub struct Stage {
    pub label: String,
    pub value: Option<f64>,
    pub formatted: String,
    pub format_string: Option<String>,
    pub color: Color,
    /// Hit id `(series, index)`.
    pub series: usize,
    pub index: usize,
    pub opacity: f64,
}

/// What a data label shows (`funnelLabelStyle`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LabelContents {
    value: bool,
    of_first: bool,
    of_previous: bool,
}

impl LabelContents {
    fn parse(text: Option<&str>) -> Self {
        let t = text.unwrap_or("Data value").to_ascii_lowercase();
        if t.contains("all") {
            return Self {
                value: true,
                of_first: true,
                of_previous: true,
            };
        }
        let parts = Self {
            value: t.contains("value"),
            of_first: t.contains("first"),
            of_previous: t.contains("previous"),
        };
        if parts.value || parts.of_first || parts.of_previous {
            parts
        } else {
            Self {
                value: true,
                of_first: false,
                of_previous: false,
            }
        }
    }
}

/// `share` as a percentage with one decimal, `.0` dropped (`5.2%`,
/// `100%`), as the service's conversion-rate labels.
pub fn short_percent(share: f64) -> String {
    if !share.is_finite() {
        return String::new();
    }
    let text = super::format::group_number(share * 100.0, 1);
    format!("{}%", text.strip_suffix(".0").unwrap_or(&text))
}

/// The stages of a funnel (see the module docs).
pub fn stages(cc: &ChartCtx<'_>) -> Vec<Stage> {
    let data = cc.data;
    let colors = cc.data_point_colors();
    let resolver = cc.resolver();
    let default = colors.default_color();
    let opacity = cc.selection_opacity();
    if data.category_metas.is_empty() && data.series.len() > 1 {
        return data
            .series
            .iter()
            .enumerate()
            .map(|(s, series)| Stage {
                label: series.measure_name.clone(),
                value: series.value(0),
                formatted: series.formatted(0).to_owned(),
                format_string: series.format_at(0).map(str::to_owned),
                color: series
                    .conditional_fill(0)
                    .or_else(|| colors.measure_color(&series.query_ref))
                    .or(default)
                    .unwrap_or_else(|| resolver.data_color(0)),
                series: s,
                index: 0,
                opacity: 1.0,
            })
            .collect();
    }
    let Some(series) = data.series.first() else {
        return Vec::new();
    };
    let base = colors
        .measure_color(&series.query_ref)
        .or(default)
        .unwrap_or_else(|| resolver.data_color(0));
    data.categories
        .iter()
        .enumerate()
        .map(|(i, label)| Stage {
            label: label.clone(),
            value: series.value(i),
            formatted: series.formatted(i).to_owned(),
            format_string: series.format_at(i).map(str::to_owned),
            color: series
                .conditional_fill(i)
                .or_else(|| colors.scoped_color(&data.category_fields(i)))
                .unwrap_or(base),
            series: 0,
            index: i,
            opacity: opacity
                .as_ref()
                .and_then(|o| o.get(i))
                .copied()
                .unwrap_or(1.0),
        })
        .collect()
}

/// Renders a funnel chart.
pub fn render(cc: &ChartCtx<'_>) -> Group {
    let mut g = Group::new();
    let stages = stages(cc);
    let area = cc.area;
    if stages.is_empty() || area.is_empty() {
        return g;
    }
    let fonts = cc.fonts();
    let o = cc.objects();
    let cat = AxisSettings::read(&cc.props, cc.resolver(), "categoryAxis", None, false);
    let labels = DataLabels::read(&cc.props, true);
    let contents = LabelContents::parse(
        o.text("labels", "funnelLabelStyle")
            .or_else(|| o.text("labels", "labelStyle"))
            .as_deref(),
    );
    let explicit_label_color =
        o.color("labels", "color").is_some() || o.color("labels", "labelColor").is_some();
    let show_pct = o.bool("percentBarLabel", "show").unwrap_or(true);
    let mut pct_style = o.text_style("percentBarLabel", "smallLightLabel");
    if let Some(c) = o.color("percentBarLabel", "labelColor") {
        pct_style.color = c;
    }
    let first = stages.first().and_then(|s| s.value).filter(|v| *v != 0.0);
    let last = stages.iter().rev().find_map(|s| s.value);
    let pct_rows = show_pct && first.is_some();
    let pct_row_h = if pct_rows {
        pct_style.line_height() + BRACKET + BRACKET_GAP
    } else {
        0.0
    };
    let label_w = if cat.show {
        let widest = stages
            .iter()
            .take(2_000)
            .map(|s| width(fonts, &s.label, &cat.label_style))
            .fold(0.0, f64::max);
        (widest + LABEL_GAP).min(area.width * 0.3)
    } else {
        0.0
    };
    let top = area.y + pct_row_h;
    let bottom = area.bottom() - if stages.len() > 1 { pct_row_h } else { 0.0 };
    let plot_h = (bottom - top).max(1.0);
    let n = stages.len();
    let min_band = if cat.show || labels.show {
        cat.label_style
            .line_height()
            .max(labels.style.line_height())
            * 1.4
    } else {
        4.0
    };
    let scroll = n > 1 && n as f64 * min_band > plot_h + 0.01;
    let (count, band) = if scroll {
        (((plot_h / min_band).floor() as usize).clamp(1, n), min_band)
    } else {
        (n, plot_h / n as f64)
    };
    let right = if scroll { SCROLLBAR_SIZE + 4.0 } else { 0.0 };
    let plot = Rect::new(
        area.x + label_w,
        top,
        (area.width - label_w - right).max(1.0),
        plot_h,
    );
    let cx = plot.x + plot.width / 2.0;
    let max_abs = stages
        .iter()
        .filter_map(|s| s.value)
        .map(f64::abs)
        .fold(0.0, f64::max);
    let bar_w = |v: Option<f64>| match v {
        Some(v) if max_abs > 0.0 => (v.abs() / max_abs * plot.width).clamp(0.0, plot.width),
        _ => 0.0,
    };
    let bar_h = band * BAR_FILL;
    let bar_y = |k: usize| plot.y + k as f64 * band + (band - bar_h) / 2.0;

    let mut ax = Group::new().with_class("pbi-axes");
    if cat.show {
        for (k, s) in stages.iter().take(count).enumerate() {
            let text = tailor(fonts, &s.label, &cat.label_style, label_w - LABEL_GAP);
            ax.text(
                area.x,
                baseline_centered(&cat.label_style, bar_y(k) + bar_h / 2.0),
                text,
                cat.label_style.clone(),
                TextAnchor::Start,
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
        scrollbar(&mut ax, track, count as f64 / n as f64, 0.0, true);
    }
    g.push(ax);

    let mut bars = Group::new().with_class("pbi-bars");
    for (k, s) in stages.iter().take(count).enumerate() {
        let w = bar_w(s.value);
        if w <= 0.0 {
            continue;
        }
        let rect = Rect::new(cx - w / 2.0, bar_y(k), w, bar_h);
        let mut bg = Group::new().with_id(cc.hit(s.series, s.index));
        if s.opacity < 1.0 {
            bg = bg.with_opacity(s.opacity);
        }
        bg.push(RectNode::new(rect).fill(Paint::solid(s.color)));
        bars.push(bg);
    }
    g.push(bars);

    if labels.show {
        let mut lg = Group::new().with_class("pbi-data-labels");
        let mut previous: Option<f64> = None;
        for (k, s) in stages.iter().enumerate() {
            let prev = previous;
            previous = s.value.or(previous);
            if k >= count {
                break;
            }
            let Some(v) = s.value else { continue };
            let text = label_text(s, v, contents, &labels, max_abs, first, prev);
            if text.is_empty() {
                continue;
            }
            let w = bar_w(s.value);
            let tw = width(fonts, &text, &labels.style);
            let cy = bar_y(k) + bar_h / 2.0;
            if labels.style.line_height() > band + 2.0 {
                continue;
            }
            let outside = labels.position.starts_with("outside");
            if !outside && tw + 6.0 <= w {
                let st = if explicit_label_color {
                    labels.style.clone()
                } else {
                    inside_style(&labels.style, s.color)
                };
                lg.text(cx, baseline_centered(&st, cy), text, st, TextAnchor::Middle);
            } else if cx + w / 2.0 + 4.0 + tw <= plot.right() + 0.5 {
                lg.text(
                    cx + w / 2.0 + 4.0,
                    baseline_centered(&labels.style, cy),
                    text,
                    labels.style.clone(),
                    TextAnchor::Start,
                );
            }
        }
        g.push(lg);
    }

    if let (true, Some(first_v)) = (pct_rows, first) {
        let mut pg = Group::new().with_class("pbi-conversion-rate");
        let stroke = Stroke::new(
            cc.resolver()
                .named_color("foregroundNeutralTertiary")
                .unwrap_or(Color::rgb(0xB3, 0xB0, 0xAD)),
            1.0,
        );
        let lh = pct_style.line_height();
        // Top: 100% over the first bar.
        let w0 = bar_w(Some(first_v)).max(2.0);
        let y_top = area.y + lh + BRACKET_GAP;
        bracket(&mut pg, cx, w0, y_top, BRACKET, &stroke);
        pg.text(
            cx,
            baseline_centered(&pct_style, area.y + lh / 2.0),
            short_percent(1.0),
            pct_style.clone(),
            TextAnchor::Middle,
        );
        if n > 1
            && let Some(last_v) = last
        {
            let wl = bar_w(Some(last_v)).max(2.0);
            let y_b = area.bottom() - lh - BRACKET_GAP;
            bracket(&mut pg, cx, wl, y_b, -BRACKET, &stroke);
            pg.text(
                cx,
                baseline_centered(&pct_style, area.bottom() - lh / 2.0),
                short_percent(last_v / first_v),
                pct_style.clone(),
                TextAnchor::Middle,
            );
        }
        g.push(pg);
    }
    g
}

/// A horizontal line `w` wide centred on `cx` at `y`, with end ticks
/// `tick` long (down when positive).
fn bracket(g: &mut Group, cx: f64, w: f64, y: f64, tick: f64, stroke: &Stroke) {
    let (x0, x1) = (cx - w / 2.0, cx + w / 2.0);
    let mut b = PathBuilder::new();
    b.move_to(x0, y + tick)
        .line_to(x0, y)
        .line_to(x1, y)
        .line_to(x1, y + tick);
    g.push(PathNode::new(b.build()).stroke(stroke.clone()));
}

fn label_text(
    s: &Stage,
    v: f64,
    contents: LabelContents,
    labels: &DataLabels,
    max_abs: f64,
    first: Option<f64>,
    previous: Option<f64>,
) -> String {
    let mut parts = Vec::new();
    if contents.value {
        parts.push(data_label(
            v,
            &s.formatted,
            max_abs,
            &NumberStyle::from_format(s.format_string.as_deref()),
            labels.unit,
            labels.precision,
        ));
    }
    if contents.of_first
        && let Some(f) = first
    {
        parts.push(short_percent(v / f));
    }
    if contents.of_previous {
        match previous.filter(|p| *p != 0.0) {
            Some(p) => parts.push(short_percent(v / p)),
            None if previous.is_none() => parts.push(short_percent(1.0)),
            None => {}
        }
    }
    parts.retain(|p| !p.is_empty());
    parts.join(" ")
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
