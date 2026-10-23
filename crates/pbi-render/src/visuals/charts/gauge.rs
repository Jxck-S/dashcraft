//! Radial gauges (`gauge`).
//!
//! Roles: `Y` (the value), `MinValue`, `MaxValue`, `TargetValue`
//! (`Tooltips` are ignored); the query has no category, so each role is
//! one cell. Missing bounds come from `axis.min` / `axis.max` /
//! `axis.target`, else Power BI's defaults: the minimum is 0 and the
//! maximum is double the value (the value sits half-way; for a negative
//! value the range is `[2 × value, 0]`).
//!
//! Drawing, as in the service:
//!
//! - a 180° arc (outer radius as large as the box allows, inner radius 70%
//!   of it) in a light grey track, filled from the minimum to the value in
//!   `dataPoint.fill` (else `defaultColor`, else `dataColors[0]`);
//! - the target as a needle across the arc (`dataPoint.target`, default the
//!   theme foreground) labelled outside the arc (`targetLabel`: `show`,
//!   `color`, font, `labelDisplayUnits`, `labelPrecision`);
//! - the minimum and maximum under the arc ends (`labels`: `show`, `color`,
//!   font, display units, precision);
//! - the value in the middle (`calloutValue`: `show`, `color`, `fontSize`,
//!   `fontFamily`, `labelDisplayUnits`, `labelPrecision`; the `callout`
//!   class), sized to the inner radius unless a size is set, then shrunk
//!   to fit.
//!
//! Numbers share one display unit (Auto: from the largest of the value,
//! bounds and target) and keep the decimals of the value's format string
//! (`184.21K`, `0.00K`, `368.42K` for a `#,0.00` measure).
//!
//! Hit id: the arc is `DataPoint{series of Y, 0}` (tooltip only: a gauge
//! has no category to cross-filter by).

use crate::dataview::DataView;
use crate::scene::{Color, Group, Paint, PathBuilder, PathData, PathNode, Stroke, TextAnchor};
use crate::text::TextStyle;

use super::ChartCtx;
use super::format::{DisplayUnit, NumberStyle, data_label, percent};
use super::labels::{baseline_centered, tailor, width};

/// Inner radius as a share of the outer radius.
pub const INNER_RATIO: f64 = 0.7;
/// Color of the unfilled part of the arc.
pub const TRACK_COLOR: Color = Color::rgb(0xE9, 0xE9, 0xE9);
/// Arc start and sweep (degrees clockwise from 12 o'clock).
const START_DEG: f64 = 270.0;
const SWEEP_DEG: f64 = 180.0;

/// A number with its DataView text (empty for formatting constants).
#[derive(Debug, Clone, PartialEq)]
pub struct GaugeNumber {
    pub value: f64,
    pub formatted: String,
}

/// The resolved inputs of a gauge.
#[derive(Debug, Clone, PartialEq)]
pub struct GaugeData {
    pub value: Option<GaugeNumber>,
    pub min: GaugeNumber,
    pub max: GaugeNumber,
    pub target: Option<GaugeNumber>,
    pub format_string: Option<String>,
    /// Hit id series (the `Y` column among [`super::ChartData::series`]).
    pub series: usize,
}

impl GaugeData {
    /// Fill fraction of the arc (0–1).
    pub fn fraction(&self, v: f64) -> f64 {
        let span = self.max.value - self.min.value;
        if !(span.is_finite() && span > 0.0) || !v.is_finite() {
            return 0.0;
        }
        ((v - self.min.value) / span).clamp(0.0, 1.0)
    }
}

fn has_role(dv: &DataView, meta_idx: usize, roles: &[&str]) -> bool {
    dv.meta(meta_idx)
        .is_some_and(|m| m.roles.iter().any(|r| roles.contains(&r.as_str())))
}

/// Row-0 cell of the first value column bound to one of `roles`.
fn role_number(dv: &DataView, roles: &[&str]) -> Option<(GaugeNumber, usize)> {
    let cat = dv.categorical.as_ref()?;
    let (k, vc) = cat
        .values
        .iter()
        .enumerate()
        .find(|(_, vc)| has_role(dv, vc.meta_idx, roles))?;
    let cell = vc.values.first()?;
    let v = cell
        .raw
        .is_numeric()
        .then(|| cell.raw.as_f64())
        .flatten()
        .filter(|v| v.is_finite())?;
    Some((
        GaugeNumber {
            value: v,
            formatted: cell.formatted.clone(),
        },
        k,
    ))
}

fn constant(v: f64) -> GaugeNumber {
    GaugeNumber {
        value: v,
        formatted: String::new(),
    }
}

/// Reads the gauge's value, bounds and target (see the module docs).
pub fn gauge_data(cc: &ChartCtx<'_>) -> Option<GaugeData> {
    let dv = cc.dataview()?;
    let cat = dv.categorical.as_ref()?;
    let bounds = ["MinValue", "MaxValue", "TargetValue", "Tooltips"];
    let y_column = cat
        .values
        .iter()
        .position(|vc| has_role(dv, vc.meta_idx, &["Y", "Values"]))
        .or_else(|| {
            cat.values
                .iter()
                .position(|vc| !has_role(dv, vc.meta_idx, &bounds))
        })?;
    let y = cat.values.get(y_column)?;
    let value = y.values.first().and_then(|cell| {
        let v = cell.raw.as_f64().filter(|v| v.is_finite())?;
        cell.raw.is_numeric().then(|| GaugeNumber {
            value: v,
            formatted: cell.formatted.clone(),
        })
    });
    let format_string = dv.meta(y.meta_idx).and_then(|m| m.format_string.clone());
    let o = cc.objects();
    let axis = |prop: &str| o.number("axis", prop).filter(|v| v.is_finite());
    let v = value.as_ref().map_or(0.0, |n| n.value);
    let twice = if (2.0 * v).is_finite() { 2.0 * v } else { v };
    let min = role_number(dv, &["MinValue"])
        .map(|(n, _)| n)
        .or_else(|| axis("min").map(constant))
        .unwrap_or_else(|| constant(if v < 0.0 { twice } else { 0.0 }));
    let mut max = role_number(dv, &["MaxValue"])
        .map(|(n, _)| n)
        .or_else(|| axis("max").map(constant))
        .unwrap_or_else(|| constant(if v > 0.0 { twice } else { 0.0 }));
    let ordered = max.value.is_finite() && max.value > min.value;
    if !ordered {
        let span = if v > min.value {
            2.0 * (v - min.value)
        } else {
            0.0
        };
        let span = if span.is_finite() && span > 0.0 {
            span
        } else {
            min.value.abs().max(1.0)
        };
        max = constant(min.value + span);
    }
    let target = role_number(dv, &["TargetValue"])
        .map(|(n, _)| n)
        .or_else(|| axis("target").map(constant));
    let series = cc
        .data
        .series
        .iter()
        .position(|s| s.column == y_column)
        .unwrap_or(cc.data.series.len());
    Some(GaugeData {
        value,
        min,
        max,
        target,
        format_string,
        series,
    })
}

/// Decimals of the first section of a format string (`#,0.00` → 2),
/// `None` without one.
pub fn format_decimals(format: Option<&str>) -> Option<usize> {
    let fmt = format.map(str::trim).filter(|f| !f.is_empty())?;
    match fmt.to_ascii_lowercase().as_str() {
        "percent" | "currency" | "fixed" | "standard" => return Some(2),
        "general number" | "g" => return None,
        _ => {}
    }
    let mut in_quotes = false;
    let mut escaped = false;
    let mut seen_digit = false;
    let mut decimals: Option<usize> = None;
    for c in fmt.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '"' => in_quotes = !in_quotes,
            _ if in_quotes => {}
            ';' => break,
            '0' | '#' => match decimals.as_mut() {
                Some(d) => *d += 1,
                None => seen_digit = true,
            },
            '.' if decimals.is_none() => decimals = Some(0),
            _ if decimals.is_some_and(|d| d > 0) => break,
            _ => {}
        }
    }
    (seen_digit || decimals.is_some()).then(|| decimals.unwrap_or(0))
}

/// Number formatting shared by the callout, bounds and target labels.
struct Formatter {
    style: NumberStyle,
    decimals: Option<usize>,
    max_abs: f64,
}

impl Formatter {
    fn text(&self, n: &GaugeNumber, unit: Option<DisplayUnit>, precision: Option<usize>) -> String {
        let precision = precision.or(self.decimals);
        if self.style.percent {
            return if n.formatted.is_empty() {
                percent(n.value, precision.unwrap_or(0))
            } else {
                n.formatted.clone()
            };
        }
        let unit = unit.unwrap_or_else(|| DisplayUnit::auto(self.max_abs));
        data_label(
            n.value,
            &n.formatted,
            self.max_abs,
            &self.style,
            Some(unit),
            precision,
        )
    }
}

/// `labelDisplayUnits` / `labelPrecision` of an object.
fn units(cc: &ChartCtx<'_>, object: &str) -> (Option<DisplayUnit>, Option<usize>) {
    let o = cc.objects();
    (
        DisplayUnit::from_setting(o.number(object, "labelDisplayUnits")),
        o.number(object, "labelPrecision")
            .filter(|p| *p >= 0.0)
            .map(|p| p.min(10.0) as usize),
    )
}

fn label_style(cc: &ChartCtx<'_>, object: &str) -> TextStyle {
    let o = cc.objects();
    let mut style = o.text_style(object, "smallLightLabel");
    if let Some(c) = o.color(object, "labelColor") {
        style.color = c;
    }
    style
}

/// Renders a gauge.
pub fn render(cc: &ChartCtx<'_>) -> Group {
    let mut g = Group::new();
    let area = cc.area;
    let Some(gd) = gauge_data(cc) else {
        return g;
    };
    if area.is_empty() {
        return g;
    }
    let fonts = cc.fonts();
    let o = cc.objects();
    let resolver = cc.resolver();
    let max_abs = [
        Some(gd.min.value),
        Some(gd.max.value),
        gd.value.as_ref().map(|n| n.value),
        gd.target.as_ref().map(|n| n.value),
    ]
    .into_iter()
    .flatten()
    .map(f64::abs)
    .fold(0.0, f64::max);
    let fmt = Formatter {
        style: NumberStyle::from_format(gd.format_string.as_deref()),
        decimals: format_decimals(gd.format_string.as_deref()),
        max_abs,
    };
    let show_bounds = o.bool("labels", "show").unwrap_or(true);
    let bounds_style = label_style(cc, "labels");
    let show_target_label = o.bool("targetLabel", "show").unwrap_or(true);
    let target_style = label_style(cc, "targetLabel");
    let (b_unit, b_prec) = units(cc, "labels");
    let (t_unit, t_prec) = units(cc, "targetLabel");
    let target_text = gd
        .target
        .as_ref()
        .filter(|_| show_target_label)
        .map(|t| fmt.text(t, t_unit, t_prec));

    // Geometry: the arc's bounding box is 2R × R; the bounds labels hang
    // under it and the target label may stick out above and beside it.
    let bounds_h = if show_bounds {
        bounds_style.line_height() + 2.0
    } else {
        0.0
    };
    let (side, top) = match &target_text {
        Some(t) => (
            (width(fonts, t, &target_style) + 6.0).min(area.width * 0.2),
            target_style.line_height() + 2.0,
        ),
        None => (4.0, 4.0),
    };
    let r = ((area.width - 2.0 * side) / 2.0)
        .min(area.height - top - bounds_h)
        .max(0.0);
    if r < 4.0 {
        return g;
    }
    let inner = r * INNER_RATIO;
    let cx = area.x + area.width / 2.0;
    let slack = (area.height - top - r - bounds_h).max(0.0);
    let cy = area.y + top + slack / 2.0 + r;
    let at = |radius: f64, deg: f64| {
        let (s, c) = deg.to_radians().sin_cos();
        (cx + radius * s, cy - radius * c)
    };

    let fill = cc
        .data_point_colors()
        .default_color()
        .or_else(|| o.color("dataPoint", "fill"))
        .unwrap_or_else(|| resolver.data_color(0));
    let mut arc = Group::new().with_id(cc.hit(gd.series, 0));
    if let Some(op) = cc
        .selection_opacity()
        .and_then(|o| o.first().copied())
        .filter(|o| *o < 1.0)
    {
        arc = arc.with_opacity(op);
    }
    arc.push(
        PathNode::new(PathData::pie_slice(
            cx,
            cy,
            r,
            inner,
            START_DEG,
            START_DEG + SWEEP_DEG,
        ))
        .fill(Paint::solid(TRACK_COLOR)),
    );
    if let Some(v) = &gd.value {
        let sweep = SWEEP_DEG * gd.fraction(v.value);
        if sweep > 0.0 {
            arc.push(
                PathNode::new(PathData::pie_slice(
                    cx,
                    cy,
                    r,
                    inner,
                    START_DEG,
                    START_DEG + sweep,
                ))
                .fill(Paint::solid(fill)),
            );
        }
    }
    g.push(arc.with_class("pbi-gauge-arc"));

    let mut lg = Group::new().with_class("pbi-data-labels");
    if let Some(t) = &gd.target {
        let deg = START_DEG + SWEEP_DEG * gd.fraction(t.value);
        let color = o
            .color("dataPoint", "target")
            .unwrap_or_else(|| resolver.foreground());
        let (x0, y0) = at(inner, deg);
        let (x1, y1) = at(r, deg);
        let mut b = PathBuilder::new();
        b.move_to(x0, y0).line_to(x1, y1);
        let mut needle = Group::new().with_class("pbi-gauge-target");
        needle.push(PathNode::new(b.build()).stroke(Stroke::new(color, 2.0)));
        g.push(needle);
        if let Some(text) = &target_text {
            let (x, y) = at(r + 4.0, deg);
            let sin = deg.to_radians().sin();
            let lh = target_style.line_height();
            let (anchor, cy_text) = if sin.abs() < 0.15 {
                (TextAnchor::Middle, y - lh / 2.0)
            } else if sin > 0.0 {
                (TextAnchor::Start, y - lh / 2.0)
            } else {
                (TextAnchor::End, y - lh / 2.0)
            };
            let room = match anchor {
                TextAnchor::Start => area.right() - x,
                TextAnchor::End => x - area.x,
                TextAnchor::Middle => area.width,
            };
            let text = tailor(fonts, text, &target_style, room);
            lg.text(
                x,
                baseline_centered(&target_style, cy_text.max(area.y + lh / 2.0)),
                text,
                target_style.clone(),
                anchor,
            );
        }
    }
    if show_bounds {
        let band = (r + inner) / 2.0;
        let lh = bounds_style.line_height();
        let room = (r - inner + 2.0 * side).max(r - inner);
        for (n, x) in [(&gd.min, cx - band), (&gd.max, cx + band)] {
            let text = tailor(fonts, &fmt.text(n, b_unit, b_prec), &bounds_style, room);
            lg.text(
                x,
                baseline_centered(&bounds_style, cy + 2.0 + lh / 2.0),
                text,
                bounds_style.clone(),
                TextAnchor::Middle,
            );
        }
    }
    if o.bool("calloutValue", "show").unwrap_or(true) {
        let text = match &gd.value {
            Some(v) => {
                let (unit, prec) = units(cc, "calloutValue");
                fmt.text(v, unit, prec)
            }
            None => crate::dataview::BLANK_LABEL.to_owned(),
        };
        let mut style = o.text_style("calloutValue", "callout");
        if o.number("calloutValue", "fontSize").is_none() {
            // Sized to the hole (about 45% of the inner radius in px).
            style.size_pt = (inner * 0.45 * 0.75).clamp(6.0, 200.0);
        }
        let max_w = inner * 1.7;
        while style.size_pt > 6.0
            && (width(fonts, &text, &style) > max_w || style.ascent() > inner * 0.85)
        {
            style.size_pt = (style.size_pt * 0.9).max(6.0);
        }
        let text = tailor(fonts, &text, &style, max_w);
        lg.text(cx, cy - inner * 0.12, text, style, TextAnchor::Middle);
    }
    g.push(lg);
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_of_format_strings() {
        assert_eq!(format_decimals(Some("#,0.00")), Some(2));
        assert_eq!(format_decimals(Some("0")), Some(0));
        assert_eq!(format_decimals(Some("0.0%;-0.0%;0.0%")), Some(1));
        assert_eq!(format_decimals(Some("\\$#,0.000 \"units\"")), Some(3));
        assert_eq!(format_decimals(Some("Percent")), Some(2));
        assert_eq!(format_decimals(None), None);
        assert_eq!(format_decimals(Some("General Number")), None);
    }
}
