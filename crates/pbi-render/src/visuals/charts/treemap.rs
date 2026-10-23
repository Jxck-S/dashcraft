//! Treemaps (`treemap`).
//!
//! Roles: `Group` (or `Category`) → branches, `Details` → leaves inside
//! them, `Values` → sizes, `Legend`/`Series` → leaves per series value
//! colored by series; `Tooltips` are ignored. The query groups by
//! Group × Details, so each DataView row is a leaf.
//!
//! As in the service:
//!
//! - rectangles are arranged by size, largest branch at the top left, with
//!   the squarified algorithm (Bruls, Huizing & van Wijk, 2000: rows laid
//!   along the shorter side of the free space while the worst aspect ratio
//!   improves); leaves inside a branch the same way, largest first. Only
//!   positive values get an area (zero, negative, BLANK, NaN are skipped);
//! - branches take palette colors in size order (`dataPoint.fill` per
//!   group value overrides), leaves share their branch's color (with a
//!   legend: the series colors), 1px gaps between leaves and 2px between
//!   branches;
//! - category labels on by default (`categoryLabels`: `show`, `color`,
//!   `fontSize`, `fontFamily`; the `label` class in white): the branch
//!   name at the top left, leaf names at the bottom left, truncated with
//!   `...`, dropped when the rectangle is too small;
//! - data labels off by default (`labels`: `show`, `color`, font, display
//!   units, precision): the value under the branch name, or above the leaf
//!   name;
//! - legend off by default (`legend.*`).
//!
//! Hit ids: every leaf is `DataPoint{series, row}`; a branch without
//! details is its own leaf.

use std::collections::HashMap;

use pbi_engine::ValueKey;

use crate::dataview::BLANK_LABEL;
use crate::scene::{Color, Group, Paint, Rect, RectNode, TextAnchor};
use crate::text::TextStyle;

use super::ChartCtx;
use super::colors::series_colors;
use super::format::{NumberStyle, data_label};
use super::labels::{DataLabels, baseline_centered, tailor};
use super::legend::{Legend, LegendItem};

/// Most labels drawn (untrusted-input guard).
const MAX_LABELS: usize = 2_000;
/// Text inset inside a rectangle.
const TEXT_INSET: f64 = 4.0;

/// One sized rectangle.
#[derive(Debug, Clone, PartialEq)]
pub struct Leaf {
    /// Hit id `(series, index)`.
    pub series: usize,
    pub index: usize,
    /// Details / series label (empty for a branch without details).
    pub label: String,
    pub value: f64,
    pub formatted: String,
    pub format_string: Option<String>,
    pub color: Color,
}

/// A top-level rectangle (one group value).
#[derive(Debug, Clone, PartialEq)]
pub struct Branch {
    pub label: String,
    pub color: Color,
    pub total: f64,
    /// The branch has nested leaves (details or series).
    pub nested: bool,
    pub leaves: Vec<Leaf>,
}

/// Squarified layout of `values` in `rect`: one rectangle per value, in
/// input order (give values sorted descending for the classic look).
/// Non-positive and non-finite values get an empty rectangle.
pub fn squarify(values: &[f64], rect: Rect) -> Vec<Rect> {
    let n = values.len();
    let mut out = vec![Rect::new(rect.x, rect.y, 0.0, 0.0); n];
    let valid = |v: f64| v.is_finite() && v > 0.0;
    let total: f64 = values.iter().copied().filter(|v| valid(*v)).sum();
    if !(total.is_finite() && total > 0.0 && rect.width > 0.0 && rect.height > 0.0) {
        return out;
    }
    let scale = rect.width * rect.height / total;
    let items: Vec<(usize, f64)> = values
        .iter()
        .enumerate()
        .filter(|(_, v)| valid(**v))
        .map(|(i, v)| (i, v * scale))
        .filter(|(_, a)| a.is_finite() && *a > 0.0)
        .collect();
    let mut free = rect;
    let mut i = 0;
    while let Some(&(_, first)) = items.get(i) {
        let side = free.width.min(free.height).max(f64::MIN_POSITIVE);
        let (mut sum, mut hi, mut lo) = (first, first, first);
        let mut worst = worst_ratio(sum, hi, lo, side);
        let mut j = i + 1;
        while let Some(&(_, a)) = items.get(j) {
            let (s2, h2, l2) = (sum + a, hi.max(a), lo.min(a));
            let w2 = worst_ratio(s2, h2, l2, side);
            if w2 > worst {
                break;
            }
            (sum, hi, lo, worst) = (s2, h2, l2, w2);
            j += 1;
        }
        let row = items.get(i..j).unwrap_or_default();
        let last = j == items.len();
        if free.width >= free.height {
            // A column along the left edge.
            let w = if last {
                free.width
            } else {
                (sum / free.height).min(free.width)
            };
            let mut y = free.y;
            for (k, (idx, a)) in row.iter().enumerate() {
                let h = if k + 1 == row.len() {
                    free.bottom() - y
                } else {
                    a / w
                };
                if let Some(slot) = out.get_mut(*idx) {
                    *slot = Rect::new(free.x, y, w, h.max(0.0));
                }
                y += h;
            }
            free = Rect::new(free.x + w, free.y, (free.width - w).max(0.0), free.height);
        } else {
            // A row along the top edge.
            let h = if last {
                free.height
            } else {
                (sum / free.width).min(free.height)
            };
            let mut x = free.x;
            for (k, (idx, a)) in row.iter().enumerate() {
                let w = if k + 1 == row.len() {
                    free.right() - x
                } else {
                    a / h
                };
                if let Some(slot) = out.get_mut(*idx) {
                    *slot = Rect::new(x, free.y, w.max(0.0), h);
                }
                x += w;
            }
            free = Rect::new(free.x, free.y + h, free.width, (free.height - h).max(0.0));
        }
        i = j;
    }
    out
}

/// Worst aspect ratio of a row of areas (sum, max, min) laid along `side`.
fn worst_ratio(sum: f64, hi: f64, lo: f64, side: f64) -> f64 {
    let s2 = sum * sum;
    let w2 = side * side;
    (w2 * hi / s2).max(s2 / (w2 * lo))
}

fn cell_text(c: Option<&crate::dataview::CellValue>) -> String {
    match c {
        Some(c) if !c.formatted.is_empty() => c.formatted.clone(),
        Some(c) if !c.is_blank() => c.raw.to_string(),
        _ => BLANK_LABEL.to_owned(),
    }
}

/// The branches of a treemap, sorted by size (see the module docs).
pub fn branches(cc: &ChartCtx<'_>) -> Vec<Branch> {
    let data = cc.data;
    let Some(cat) = cc.dataview().and_then(|dv| dv.categorical.as_ref()) else {
        return Vec::new();
    };
    if data.series.is_empty() {
        return Vec::new();
    }
    let dv = cc.dataview();
    let role_of = |meta_idx: usize, role: &str| {
        dv.and_then(|dv| dv.meta(meta_idx))
            .is_some_and(|m| m.roles.iter().any(|r| r == role))
    };
    let details: Option<usize> = cat
        .categories
        .iter()
        .position(|c| role_of(c.meta_idx, "Details"));
    let mut groups: Vec<usize> = (0..cat.categories.len())
        .filter(|c| Some(*c) != details)
        .collect();
    let details = if groups.is_empty() {
        groups.extend(details);
        None
    } else {
        details
    };
    let legend = data.series_meta.is_some();
    let colors = cc.data_point_colors();
    let resolver = cc.resolver();
    let series_palette = series_colors(&colors, data, resolver);
    let mut out: Vec<Branch> = Vec::new();
    let mut by_key: HashMap<Vec<ValueKey>, usize> = HashMap::new();
    let mut explicit: Vec<Option<Color>> = Vec::new();
    for i in 0..data.len() {
        let mut leaves = Vec::new();
        let series: Vec<usize> = if legend {
            (0..data.series.len()).collect()
        } else {
            vec![0]
        };
        for s in series {
            let Some(sd) = data.series.get(s) else {
                continue;
            };
            let Some(v) = sd.value(i).filter(|v| *v > 0.0) else {
                continue;
            };
            let mut label = details
                .and_then(|d| cat.categories.get(d))
                .map(|c| cell_text(c.values.get(i)))
                .unwrap_or_default();
            if legend {
                let name = sd.series_label.clone().unwrap_or_else(|| sd.name.clone());
                label = if label.is_empty() {
                    name
                } else {
                    format!("{label} {name}")
                };
            }
            leaves.push(Leaf {
                series: s,
                index: i,
                label,
                value: v,
                formatted: sd.formatted(i).to_owned(),
                format_string: sd.format_at(i).map(str::to_owned),
                color: sd.conditional_fill(i).unwrap_or_else(|| {
                    series_palette
                        .get(s)
                        .copied()
                        .unwrap_or_else(|| resolver.data_color(s))
                }),
            });
        }
        if leaves.is_empty() {
            continue;
        }
        let key: Vec<ValueKey> = groups
            .iter()
            .map(|g| {
                cat.categories
                    .get(*g)
                    .and_then(|c| c.values.get(i))
                    .map_or(ValueKey::Blank, |c| ValueKey::from(&c.raw))
            })
            .collect();
        let at = match by_key.get(&key) {
            Some(at) => *at,
            None => {
                let label = groups
                    .iter()
                    .filter_map(|g| cat.categories.get(*g))
                    .map(|c| cell_text(c.values.get(i)))
                    .collect::<Vec<_>>()
                    .join(" ");
                let fields: Vec<_> = data
                    .category_fields(i)
                    .into_iter()
                    .enumerate()
                    .filter(|(c, _)| groups.contains(c))
                    .map(|(_, f)| f)
                    .collect();
                explicit.push(colors.scoped_color(&fields));
                out.push(Branch {
                    label,
                    color: Color::BLACK,
                    total: 0.0,
                    nested: details.is_some() || legend,
                    leaves: Vec::new(),
                });
                by_key.insert(key, out.len() - 1);
                out.len() - 1
            }
        };
        if let Some(b) = out.get_mut(at) {
            b.total += leaves.iter().map(|l| l.value).sum::<f64>();
            b.leaves.extend(leaves);
        }
    }
    // Largest first (stable: ties keep the data order), then colors.
    let mut order: Vec<(Branch, Option<Color>)> = out.into_iter().zip(explicit).collect();
    order.sort_by(|a, b| b.0.total.total_cmp(&a.0.total));
    let default = colors.default_color().filter(|_| data.series.len() == 1);
    let mut next = 0usize;
    let mut sorted = Vec::with_capacity(order.len());
    for (mut b, explicit) in order {
        b.color = match (explicit, default) {
            (Some(c), _) => c,
            (None, Some(d)) => d,
            (None, None) => {
                let c = resolver.data_color(next);
                next += 1;
                c
            }
        };
        b.leaves.sort_by(|x, y| y.value.total_cmp(&x.value));
        if !legend {
            for l in &mut b.leaves {
                l.color = cc
                    .data
                    .series
                    .get(l.series)
                    .and_then(|sd| sd.conditional_fill(l.index))
                    .unwrap_or(b.color);
            }
        }
        sorted.push(b);
    }
    sorted
}

/// Renders a treemap.
pub fn render(cc: &ChartCtx<'_>) -> Group {
    let mut g = Group::new();
    let branches = branches(cc);
    let mut area = cc.area;
    let fonts = cc.fonts();
    let o = cc.objects();
    let legend_items: Vec<LegendItem> = if cc.data.series_meta.is_some() {
        let palette = series_colors(&cc.data_point_colors(), cc.data, cc.resolver());
        cc.data
            .series
            .iter()
            .zip(palette)
            .map(|(s, color)| LegendItem {
                label: s.series_label.clone().unwrap_or_else(|| s.name.clone()),
                color,
            })
            .collect()
    } else {
        branches
            .iter()
            .map(|b| LegendItem {
                label: b.label.clone(),
                color: b.color,
            })
            .collect()
    };
    let legend_title = cc
        .data
        .series_meta
        .as_ref()
        .map(|m| m.display_name.clone())
        .or_else(|| {
            cc.data
                .category_metas
                .first()
                .map(|m| m.display_name.clone())
        });
    if let Some(legend) = Legend::read(&cc.props, false, legend_title, legend_items) {
        area = legend.draw(&mut g, area, fonts).plot;
    }
    if branches.is_empty() || area.width < 2.0 || area.height < 2.0 {
        return g;
    }
    let white_unless = |object: &str, class: &str| -> TextStyle {
        let mut style = o.text_style(object, class);
        let explicit = o
            .color(object, "color")
            .or_else(|| o.color(object, "labelColor"));
        style.color = explicit.unwrap_or(Color::WHITE);
        style
    };
    let show_cat = o.bool("categoryLabels", "show").unwrap_or(true);
    let cat_style = white_unless("categoryLabels", "label");
    let labels = DataLabels::read(&cc.props, false);
    let value_style = white_unless("labels", "smallLightLabel");
    let opacity = cc.selection_opacity();
    let max_abs = branches
        .iter()
        .flat_map(|b| b.leaves.iter().map(|l| l.value.abs()))
        .fold(0.0, f64::max);

    let totals: Vec<f64> = branches.iter().map(|b| b.total).collect();
    let rects = squarify(&totals, area);
    let mut tiles = Group::new().with_class("pbi-treemap");
    let mut texts = Group::new().with_class("pbi-data-labels");
    let mut labels_left = MAX_LABELS;
    let lh = cat_style.line_height();
    let vlh = value_style.line_height();
    for (b, br) in branches.iter().zip(&rects) {
        let outer = br.inset_uniform(1.0);
        if outer.width <= 0.0 || outer.height <= 0.0 {
            continue;
        }
        let values: Vec<f64> = b.leaves.iter().map(|l| l.value).collect();
        let leaf_rects = if b.nested {
            squarify(&values, outer)
        } else {
            vec![outer]
        };
        for (l, lr) in b.leaves.iter().zip(&leaf_rects) {
            let r = if b.nested { lr.inset_uniform(0.5) } else { *lr };
            if r.width <= 0.0 || r.height <= 0.0 {
                continue;
            }
            let mut lg = Group::new().with_id(cc.hit(l.series, l.index));
            if let Some(op) = opacity
                .as_ref()
                .and_then(|o| o.get(l.index))
                .filter(|o| **o < 1.0)
            {
                lg = lg.with_opacity(*op);
            }
            lg.push(RectNode::new(r).fill(Paint::solid(l.color)));
            tiles.push(lg);
        }
        let room = outer.width - 2.0 * TEXT_INSET;
        // Branch label (and value) at the top left.
        let mut header_bottom = outer.y;
        if show_cat && labels_left > 0 && room > 4.0 && outer.height >= lh + TEXT_INSET {
            let text = tailor(fonts, &b.label, &cat_style, room);
            if !text.is_empty() {
                labels_left -= 1;
                texts.text(
                    outer.x + TEXT_INSET,
                    baseline_centered(&cat_style, outer.y + TEXT_INSET / 2.0 + lh / 2.0),
                    text,
                    cat_style.clone(),
                    TextAnchor::Start,
                );
                header_bottom = outer.y + TEXT_INSET / 2.0 + lh;
            }
        }
        if labels.show
            && !b.nested
            && labels_left > 0
            && let Some(l) = b.leaves.first()
        {
            let text = value_text(l, &labels, max_abs);
            let y = header_bottom + vlh / 2.0;
            if y + vlh / 2.0 <= outer.bottom() && room > 4.0 {
                let text = tailor(fonts, &text, &value_style, room);
                labels_left = labels_left.saturating_sub(1);
                texts.text(
                    outer.x + TEXT_INSET,
                    baseline_centered(&value_style, y),
                    text,
                    value_style.clone(),
                    TextAnchor::Start,
                );
            }
        }
        if !b.nested {
            continue;
        }
        // Leaf labels (and values) at the bottom left.
        for (l, lr) in b.leaves.iter().zip(&leaf_rects) {
            if labels_left == 0 {
                break;
            }
            let r = lr.inset_uniform(0.5);
            let room = r.width - 2.0 * TEXT_INSET;
            let mut bottom = r.bottom() - TEXT_INSET / 2.0;
            // Keep clear of the branch label.
            let top_limit = if (r.y - outer.y).abs() < 1.0 {
                header_bottom.max(r.y)
            } else {
                r.y
            };
            if room <= 4.0 {
                continue;
            }
            if show_cat && !l.label.is_empty() && bottom - lh >= top_limit {
                let text = tailor(fonts, &l.label, &cat_style, room);
                if !text.is_empty() {
                    labels_left -= 1;
                    texts.text(
                        r.x + TEXT_INSET,
                        baseline_centered(&cat_style, bottom - lh / 2.0),
                        text,
                        cat_style.clone(),
                        TextAnchor::Start,
                    );
                    bottom -= lh;
                }
            }
            if labels.show && labels_left > 0 && bottom - vlh >= top_limit {
                let text = tailor(fonts, &value_text(l, &labels, max_abs), &value_style, room);
                labels_left -= 1;
                texts.text(
                    r.x + TEXT_INSET,
                    baseline_centered(&value_style, bottom - vlh / 2.0),
                    text,
                    value_style.clone(),
                    TextAnchor::Start,
                );
            }
        }
    }
    g.push(tiles);
    g.push(texts);
    g
}

fn value_text(l: &Leaf, labels: &DataLabels, max_abs: f64) -> String {
    data_label(
        l.value,
        &l.formatted,
        max_abs,
        &NumberStyle::from_format(l.format_string.as_deref()),
        labels.unit,
        labels.precision,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(r: &Rect) -> f64 {
        r.width * r.height
    }

    #[test]
    fn squarified_rectangles_tile_the_area_proportionally() {
        let bounds = Rect::new(0.0, 0.0, 600.0, 400.0);
        let values = [6.0, 6.0, 4.0, 3.0, 2.0, 2.0, 1.0];
        let rects = squarify(&values, bounds);
        let total: f64 = values.iter().sum();
        for (v, r) in values.iter().zip(&rects) {
            let expected = v / total * 240_000.0;
            assert!((area(r) - expected).abs() < 1e-6, "{r:?}");
            assert!(bounds.contains_rect(r, 1e-9), "{r:?}");
        }
        // The worked example of the paper (6 × 4, scaled by 100): the two
        // 6s make a column filling the left half, then 4 and 3 a row on
        // top of the right half.
        assert_eq!((rects[0].x, rects[1].x), (0.0, 0.0));
        assert!((rects[0].width - 300.0).abs() < 1e-9);
        assert!((rects[1].y - 200.0).abs() < 1e-9);
        assert!((rects[2].x - 300.0).abs() < 1e-9 && rects[2].y == 0.0);
        assert!((rects[3].y - rects[2].y).abs() < 1e-9 && rects[3].x > rects[2].x);
        // No two rectangles overlap.
        for (a, ra) in rects.iter().enumerate() {
            for rb in rects.iter().skip(a + 1) {
                let overlap = ra.intersect(rb).map_or(0.0, |r| area(&r));
                assert!(overlap < 1e-6, "{ra:?} {rb:?}");
            }
        }
        let odd = squarify(&[1.0, f64::NAN, -2.0, 0.0, 3.0], bounds);
        assert_eq!(area(&odd[1]) + area(&odd[2]) + area(&odd[3]), 0.0);
        assert!((area(&odd[0]) + area(&odd[4]) - 240_000.0).abs() < 1e-6);
        assert!(squarify(&[1.0], Rect::new(0.0, 0.0, 0.0, 10.0))[0].is_empty());
        assert!(squarify(&[], bounds).is_empty());
    }
}
