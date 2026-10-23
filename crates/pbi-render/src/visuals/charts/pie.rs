//! Pie and donut charts.
//!
//! Slices follow the DataView order (the query sorts pies by value,
//! descending), starting at 12 o'clock and going clockwise; sizes use the
//! absolute values (zero, BLANK and non-finite values get no slice). The
//! radius is 80% of the half-size of the plot when labels sit outside
//! (95% otherwise); a donut's hole is `slices.innerRadiusRatio` (default
//! 50%) of the radius.
//!
//! # Detail labels
//!
//! Outside labels (`labels.labelPosition` other than `inside`) hang from a
//! leader line: radial from the slice edge to 90% of the half-size, then
//! 10px horizontally, the text right of it on the right half and
//! right-aligned left of it on the left half. As Power BI places them, a
//! label is tried at ¾ of its slice's arc, then the middle, then ¼, on one
//! line (`300 (75.00%)`), then split in two lines (`300` / `(75.00%)`) at
//! ¾ or ¼. It must stay off the outer ~27%
//! (one line) / ~20% (two lines) of the plot's half width, within the
//! pie's bounding square vertically, and must not overlap the pie or
//! earlier labels. Otherwise the label goes where it has the most room,
//! truncated inside the parentheses (`100 (25.0...)`).

use crate::scene::{
    Color, Group, Paint, PathBuilder, PathData, PathNode, Rect, Stroke, TextAnchor,
};
use crate::text::{FontBook, TextStyle};

use super::colors::{category_colors, series_colors};
use super::format::{NumberStyle, data_label, percent};
use super::labels::{
    DataLabels, baseline_centered, prefers_white_text, tailor, tailor_parenthesized, width,
};
use super::legend::{Legend, LegendItem};
use super::{ChartCtx, ChartKind};

/// Most labels placed (untrusted-input guard).
const MAX_LABELS: usize = 300;
/// Slices narrower than this (degrees) get no label.
const MIN_LABEL_SWEEP: f64 = 0.5;
/// Horizontal leader segment.
const LEADER_HORIZONTAL: f64 = 10.0;
/// Gap between the leader end and the text.
const LEADER_GAP: f64 = 4.0;

/// One slice to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct Slice {
    pub label: String,
    /// Absolute value (slice size).
    pub size: f64,
    pub value: f64,
    pub formatted: String,
    pub format_string: Option<String>,
    pub color: Color,
    pub series: usize,
    pub index: usize,
    pub opacity: f64,
}

/// Start and end angles (degrees clockwise from 12 o'clock) of slices with
/// the given sizes; non-positive or non-finite sizes get an empty range.
pub fn slice_angles(sizes: &[f64]) -> Vec<(f64, f64)> {
    let total: f64 = sizes.iter().filter(|v| v.is_finite() && **v > 0.0).sum();
    let mut start = 0.0;
    sizes
        .iter()
        .map(|v| {
            let sweep = if total > 0.0 && v.is_finite() && *v > 0.0 {
                v / total * 360.0
            } else {
                0.0
            };
            let range = (start, (start + sweep).min(360.0));
            start += sweep;
            range
        })
        .collect()
}

/// The slices of a pie: one per category (first series), one per measure
/// without categories, or one per category × series with both.
fn slices(cc: &ChartCtx<'_>) -> Vec<Slice> {
    let data = cc.data;
    let colors = cc.data_point_colors();
    let opacity = cc.selection_opacity();
    let op = |i: usize| {
        opacity
            .as_ref()
            .and_then(|o| o.get(i))
            .copied()
            .unwrap_or(1.0)
    };
    let mut out = Vec::new();
    if data.category_metas.is_empty() && data.series.len() > 1 {
        let palette = series_colors(&colors, data, cc.resolver());
        for (s, series) in data.series.iter().enumerate() {
            let Some(v) = series.value(0) else { continue };
            out.push(Slice {
                label: series.name.clone(),
                size: v.abs(),
                value: v,
                formatted: series.formatted(0).to_owned(),
                format_string: series.format_at(0).map(str::to_owned),
                color: series.conditional_fill(0).unwrap_or_else(|| {
                    palette
                        .get(s)
                        .copied()
                        .unwrap_or_else(|| cc.resolver().data_color(s))
                }),
                series: 0,
                index: s,
                opacity: 1.0,
            });
        }
        return out;
    }
    if data.series.len() <= 1 {
        let Some(series) = data.series.first() else {
            return out;
        };
        let palette = category_colors(&colors, data, cc.resolver());
        for (i, label) in data.categories.iter().enumerate() {
            let Some(v) = series.value(i) else { continue };
            out.push(Slice {
                label: label.clone(),
                size: v.abs(),
                value: v,
                formatted: series.formatted(i).to_owned(),
                format_string: series.format_at(i).map(str::to_owned),
                color: series.conditional_fill(i).unwrap_or_else(|| {
                    palette
                        .get(i)
                        .copied()
                        .unwrap_or_else(|| cc.resolver().data_color(i))
                }),
                series: 0,
                index: i,
                opacity: op(i),
            });
        }
        return out;
    }
    // Category × series ("Details"): explicit colors, else the palette.
    let mut next = 0usize;
    for (i, label) in data.categories.iter().enumerate() {
        for (s, series) in data.series.iter().enumerate() {
            let Some(v) = series.value(i) else { continue };
            let mut fields = data.category_fields(i);
            if let (Some(meta), Some(sv)) = (&data.series_meta, &series.series_value) {
                fields.push((meta, sv));
            }
            let color = colors.scoped_color(&fields).unwrap_or_else(|| {
                let c = cc.resolver().data_color(next);
                next += 1;
                c
            });
            out.push(Slice {
                label: format!("{label} {}", series.name),
                size: v.abs(),
                value: v,
                formatted: series.formatted(i).to_owned(),
                format_string: series.format_at(i).map(str::to_owned),
                color,
                series: s,
                index: i,
                opacity: op(i),
            });
        }
    }
    out
}

/// Renders a pie or donut chart.
pub fn render(cc: &ChartCtx<'_>) -> Group {
    let mut g = Group::new();
    let slices = slices(cc);
    let fonts = cc.fonts();
    let labels = DataLabels::read(&cc.props, true);
    let mut area = cc.area;
    let legend_title = if cc.data.category_metas.is_empty() {
        None
    } else {
        cc.data.category_title()
    };
    let items = slices
        .iter()
        .map(|s| LegendItem {
            label: s.label.clone(),
            color: s.color,
        })
        .collect();
    if let Some(legend) = Legend::read(&cc.props, true, legend_title, items) {
        area = legend.draw(&mut g, area, fonts).plot;
    }
    let sizes: Vec<f64> = slices.iter().map(|s| s.size).collect();
    let angles = slice_angles(&sizes);
    if area.is_empty() || angles.iter().all(|(a, b)| b <= a) {
        return g;
    }
    let outside = labels.show && labels.position != "inside";
    // Outside labels need room beside the pie: at most 75% of the half
    // width.
    let half = if outside {
        (area.height / 2.0 - 5.0).min(area.width / 2.0 * 0.75)
    } else {
        area.width.min(area.height) / 2.0 - 5.0
    }
    .max(1.0);
    let r = if outside { half * 0.8 } else { half * 0.95 };
    let (cx, cy) = area.center();
    let inner = if cc.kind == ChartKind::Donut {
        let ratio = cc
            .objects()
            .number("slices", "innerRadiusRatio")
            .map_or(0.5, |p| (p / 100.0).clamp(0.0, 0.95));
        r * ratio
    } else {
        0.0
    };
    let mut pie = Group::new().with_class("pbi-slices");
    for (s, (a0, a1)) in slices.iter().zip(&angles) {
        if a1 <= a0 {
            continue;
        }
        let mut sg = Group::new().with_id(cc.hit(s.series, s.index));
        if s.opacity < 1.0 {
            sg = sg.with_opacity(s.opacity);
        }
        sg.push(
            PathNode::new(PathData::pie_slice(cx, cy, r, inner, *a0, *a1))
                .fill(Paint::solid(s.color)),
        );
        pie.push(sg);
    }
    g.push(pie);
    if labels.show {
        let total: f64 = sizes.iter().filter(|v| v.is_finite() && **v > 0.0).sum();
        let max_abs = slices.iter().map(|s| s.size).fold(0.0, f64::max);
        let texts: Vec<Vec<String>> = slices
            .iter()
            .map(|s| label_parts(s, &labels, total, max_abs))
            .collect();
        let geo = PieGeometry {
            cx,
            cy,
            half,
            r,
            inner,
            elbow: (half * 0.9).max(r + 4.0),
            area,
        };
        let lg = if outside {
            outside_labels(fonts, &labels.style, &geo, &angles, &texts)
        } else {
            inside_labels(fonts, &labels.style, &geo, &angles, &texts, &slices)
        };
        g.push(lg);
    }
    g
}

/// The parts of a slice's detail label (category, value, `(percent)`).
fn label_parts(s: &Slice, labels: &DataLabels, total: f64, max_abs: f64) -> Vec<String> {
    let mut parts = Vec::new();
    if labels.parts.category {
        parts.push(s.label.clone());
    }
    let style = NumberStyle::from_format(s.format_string.as_deref());
    if labels.parts.value {
        parts.push(data_label(
            s.value,
            &s.formatted,
            max_abs,
            &style,
            labels.unit,
            labels.precision,
        ));
    }
    if labels.parts.percent && total > 0.0 {
        let p = percent(s.size / total, labels.percent_decimals);
        parts.push(if labels.parts.value {
            format!("({p})")
        } else {
            p
        });
    }
    parts.retain(|p| !p.is_empty());
    parts
}

#[derive(Debug, Clone, Copy)]
struct PieGeometry {
    cx: f64,
    cy: f64,
    /// Half-size of the pie's square (the radius is 80% of it).
    half: f64,
    r: f64,
    inner: f64,
    elbow: f64,
    area: Rect,
}

impl PieGeometry {
    fn at(&self, radius: f64, deg: f64) -> (f64, f64) {
        let (s, c) = deg.to_radians().sin_cos();
        (self.cx + radius * s, self.cy - radius * c)
    }
}

/// A placed outside label.
struct Placement {
    angle: f64,
    lines: Vec<String>,
    rect: Rect,
    right: bool,
    anchor_x: f64,
    center_y: f64,
}

fn outside_labels(
    fonts: &FontBook,
    style: &TextStyle,
    geo: &PieGeometry,
    angles: &[(f64, f64)],
    texts: &[Vec<String>],
) -> Group {
    let mut g = Group::new().with_class("pbi-data-labels");
    let lh = style.line_height();
    let half_w = geo.area.width / 2.0;
    // Horizontal margins inside the plot: one-line labels keep off the
    // outer ~27% of the half width (35px for a 131px half), two-line
    // labels off ~20% (26px); a label that would become unreadable may use
    // the full width.
    let inset_one = (half_w * 0.269).min(35.3);
    let inset_two = (half_w * 0.2).min(26.0);
    // Vertically, labels stay within the pie's square (and the plot).
    let top = geo.area.y.max(geo.cy - geo.half) - 1.0;
    let bottom = geo.area.bottom().min(geo.cy + geo.half) + 1.0;
    let mut placed: Vec<Rect> = Vec::new();
    let mut count = 0usize;
    for ((a0, a1), parts) in angles.iter().zip(texts) {
        let sweep = a1 - a0;
        if sweep < MIN_LABEL_SWEEP || parts.is_empty() || count >= MAX_LABELS {
            continue;
        }
        let one = vec![parts.join(" ")];
        let two = (parts.len() > 1).then(|| {
            let (last, first) = parts
                .split_last()
                .map_or((String::new(), &[][..]), |(l, f)| (l.clone(), f));
            vec![first.join(" "), last]
        });
        let one_w = width(fonts, &one[0], style);
        // A label anchored at `deg` and the width available to it.
        let place = |deg: f64, lines: &[String], inset: f64| -> (Placement, f64) {
            let (ex, ey) = geo.at(geo.elbow, deg);
            let right = deg.rem_euclid(360.0) < 180.0;
            let (anchor_x, avail) = if right {
                let x = ex + LEADER_HORIZONTAL + LEADER_GAP;
                (x, geo.area.right() - inset - x)
            } else {
                let x = ex - LEADER_HORIZONTAL - LEADER_GAP;
                (x, x - geo.area.x - inset)
            };
            let w = lines
                .iter()
                .map(|l| width(fonts, l, style))
                .fold(0.0, f64::max);
            let h = lh * lines.len() as f64;
            let x = if right { anchor_x } else { anchor_x - w };
            (
                Placement {
                    angle: deg,
                    lines: lines.to_vec(),
                    rect: Rect::new(x, ey - h / 2.0, w, h),
                    right,
                    anchor_x,
                    center_y: ey,
                },
                avail,
            )
        };
        let ok = |p: &Placement, avail: f64| {
            p.rect.width <= avail + 0.01
                && p.rect.y >= top
                && p.rect.bottom() <= bottom
                && !hits_disc(&p.rect, geo.cx, geo.cy, geo.r)
                && !placed.iter().any(|q| overlaps(q, &p.rect))
        };
        // Like Power BI: the label hangs at ¾ of the slice's arc when it
        // fits there (else the middle, then ¼) on one line, else split in
        // two lines (`300` / `(75.00%)`) at ¾ or ¼.
        let mut tries: Vec<(&Vec<String>, f64, f64)> = [0.75, 0.5, 0.25]
            .iter()
            .map(|f| (&one, *f, inset_one))
            .collect();
        if let Some(two) = &two {
            tries.extend([0.75, 0.25].iter().map(|f| (two, *f, inset_two)));
        }
        let mut chosen = tries.iter().find_map(|(lines, f, inset)| {
            let (p, avail) = place(a0 + sweep * f, lines, *inset);
            ok(&p, avail).then_some(p)
        });
        // Otherwise truncate where there is the most room, unless that
        // leaves too little of the label (then use the full width).
        for (round, inset) in [inset_one, 2.0].into_iter().enumerate() {
            if chosen.is_some() {
                break;
            }
            let best = [0.75, 0.5, 0.25]
                .iter()
                .map(|f| place(a0 + sweep * f, &one, inset))
                .max_by(|a, b| a.1.total_cmp(&b.1));
            let Some((mut p, avail)) = best else { break };
            if round == 0 && avail < one_w * 0.6 {
                continue;
            }
            let text = truncate_label(fonts, parts, style, avail);
            if text.is_empty() {
                continue;
            }
            let w = width(fonts, &text, style);
            let lo = geo.area.y + lh / 2.0;
            let cy = p.center_y.clamp(lo, (geo.area.bottom() - lh / 2.0).max(lo));
            let x = if p.right { p.anchor_x } else { p.anchor_x - w };
            p.rect = Rect::new(x, cy - lh / 2.0, w, lh);
            p.lines = vec![text];
            if !placed.iter().any(|q| overlaps(q, &p.rect)) {
                chosen = Some(p);
            }
        }
        let Some(p) = chosen else { continue };
        count += 1;
        placed.push(p.rect);
        draw_leader(&mut g, geo, &p, style.color);
        let anchor = if p.right {
            TextAnchor::Start
        } else {
            TextAnchor::End
        };
        for (k, line) in p.lines.iter().enumerate() {
            let cy = p.rect.y + lh * (k as f64 + 0.5);
            g.text(
                p.anchor_x,
                baseline_centered(style, cy),
                line.clone(),
                style.clone(),
                anchor,
            );
        }
    }
    g
}

/// One-line label truncated to `avail`: inside the trailing `(percent)`
/// when there is one, else at the end.
fn truncate_label(fonts: &FontBook, parts: &[String], style: &TextStyle, avail: f64) -> String {
    if let Some((last, head)) = parts.split_last()
        && !head.is_empty()
        && let Some(inner) = last.strip_prefix('(').and_then(|l| l.strip_suffix(')'))
    {
        return tailor_parenthesized(fonts, &head.join(" "), inner, style, avail);
    }
    tailor(fonts, &parts.join(" "), style, avail)
}

fn draw_leader(g: &mut Group, geo: &PieGeometry, p: &Placement, color: Color) {
    let (x0, y0) = geo.at(geo.r, p.angle);
    let (ex, ey) = geo.at(geo.elbow, p.angle);
    let hx = if p.right {
        ex + LEADER_HORIZONTAL
    } else {
        ex - LEADER_HORIZONTAL
    };
    // The text block is centred on the elbow unless it had to be clamped.
    let y = if (p.center_y - ey).abs() < 0.01 {
        ey
    } else {
        p.rect.y + p.rect.height / 2.0
    };
    let mut b = PathBuilder::new();
    b.move_to(x0, y0).line_to(ex, ey).line_to(hx, y);
    g.push(PathNode::new(b.build()).stroke(Stroke::new(color, 1.0)));
}

fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.x < b.right() - 0.5
        && b.x < a.right() - 0.5
        && a.y < b.bottom() - 0.5
        && b.y < a.bottom() - 0.5
}

/// `true` if `rect` intersects the disc of radius `r` at `(cx, cy)`.
fn hits_disc(rect: &Rect, cx: f64, cy: f64, r: f64) -> bool {
    let nx = cx.clamp(rect.x, rect.right());
    let ny = cy.clamp(rect.y, rect.bottom());
    (nx - cx).hypot(ny - cy) < r - 0.5
}

fn inside_labels(
    fonts: &FontBook,
    style: &TextStyle,
    geo: &PieGeometry,
    angles: &[(f64, f64)],
    texts: &[Vec<String>],
    slices: &[Slice],
) -> Group {
    let mut g = Group::new().with_class("pbi-data-labels");
    let lh = style.line_height();
    let rc = if geo.inner > 0.0 {
        (geo.r + geo.inner) / 2.0
    } else {
        geo.r * 0.6
    };
    let thickness = geo.r - geo.inner;
    for (((a0, a1), parts), s) in angles.iter().zip(texts).zip(slices).take(MAX_LABELS) {
        let sweep = a1 - a0;
        if sweep < MIN_LABEL_SWEEP || parts.is_empty() || thickness < lh {
            continue;
        }
        let chord = if sweep >= 180.0 {
            2.0 * rc
        } else {
            2.0 * rc * (sweep.to_radians() / 2.0).sin()
        };
        let text = tailor(fonts, &parts.join(" "), style, chord.min(thickness * 1.6));
        if text.is_empty() {
            continue;
        }
        let (x, y) = geo.at(rc, a0 + sweep / 2.0);
        let mut st = style.clone();
        st.color = if prefers_white_text(s.color) {
            Color::WHITE
        } else {
            Color::rgb(0x25, 0x24, 0x23)
        };
        g.text(x, baseline_centered(&st, y), text, st, TextAnchor::Middle);
    }
    g
}
