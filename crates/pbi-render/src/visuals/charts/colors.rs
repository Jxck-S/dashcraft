//! Data colors: theme `dataColors` and `dataPoint` formatting objects.
//!
//! Precedence (as Power BI's color helper): an explicit `dataPoint.fill`
//! whose selector matches the point (per category / per series value via
//! `selector.data[].scopeId` comparisons, per measure via
//! `selector.metadata`), then `dataPoint.defaultColor` (or a
//! default-selector / `dataViewWildcard` `fill`), then the next palette
//! color. A palette slot is consumed only by points without an explicit
//! color, so in a pie whose "Online" slice is pinned to green the other
//! slices still start at `dataColors[0]`.

use pbi_engine::Value;
use pbi_model::LiteralValue;
use serde_json::Value as Json;

use crate::dataview::ColumnMeta;
use crate::scene::Color;
use crate::theme::ThemeResolver;

use super::ChartData;

/// `dataPoint` entries of a visual, resolved against the theme.
#[derive(Debug, Clone, Copy)]
pub struct DataPointColors<'a> {
    objects: &'a Json,
    resolver: &'a ThemeResolver,
}

impl<'a> DataPointColors<'a> {
    pub fn new(objects: &'a Json, resolver: &'a ThemeResolver) -> Self {
        Self { objects, resolver }
    }

    fn entries(&self) -> impl Iterator<Item = (&'a Json, Option<&'a Json>)> + 'a {
        pbi_pbir::objects::entries(self.objects, "dataPoint")
    }

    fn fill_of(&self, props: &Json, prop: &str) -> Option<Color> {
        props.get(prop).and_then(|v| self.resolver.color(v))
    }

    /// `defaultColor` (or a `fill` with the default / wildcard selector).
    pub fn default_color(&self) -> Option<Color> {
        let mut fallback = None;
        for (props, selector) in self.entries() {
            if pbi_pbir::objects::is_default_selector(selector) || is_wildcard(selector) {
                if let Some(c) = self.fill_of(props, "defaultColor") {
                    return Some(c);
                }
                if fallback.is_none() {
                    fallback = self.fill_of(props, "fill");
                }
            }
        }
        fallback
    }

    /// Explicit color for a measure (`selector.metadata == query_ref`).
    pub fn measure_color(&self, query_ref: &str) -> Option<Color> {
        self.entries().find_map(|(props, selector)| {
            let s = selector?;
            (s.get("metadata").and_then(Json::as_str) == Some(query_ref) && s.get("data").is_none())
                .then(|| self.fill_of(props, "fill"))
                .flatten()
        })
    }

    /// Explicit color for the point identified by `fields` (column meta,
    /// value) pairs (a category row, a series value, or both).
    pub fn scoped_color(&self, fields: &[(&ColumnMeta, &Value)]) -> Option<Color> {
        if fields.is_empty() {
            return None;
        }
        self.entries().find_map(|(props, selector)| {
            let data = selector?.get("data")?.as_array()?;
            let scoped: Vec<&Json> = data.iter().filter_map(|d| d.get("scopeId")).collect();
            if scoped.is_empty() {
                return None;
            }
            scoped
                .iter()
                .all(|scope| scope_matches(scope, fields, 0))
                .then(|| self.fill_of(props, "fill"))
                .flatten()
        })
    }
}

fn is_wildcard(selector: Option<&Json>) -> bool {
    selector
        .and_then(|s| s.get("data"))
        .and_then(Json::as_array)
        .is_some_and(|d| !d.is_empty() && d.iter().all(|e| e.get("dataViewWildcard").is_some()))
}

/// `true` if a `scopeId` condition holds for `fields`.
fn scope_matches(scope: &Json, fields: &[(&ColumnMeta, &Value)], depth: u8) -> bool {
    if depth > 8 {
        return false;
    }
    if let Some(and) = scope.get("And") {
        return match (and.get("Left"), and.get("Right")) {
            (Some(l), Some(r)) => {
                scope_matches(l, fields, depth + 1) && scope_matches(r, fields, depth + 1)
            }
            _ => false,
        };
    }
    let Some(cmp) = scope.get("Comparison") else {
        return false;
    };
    if cmp
        .get("ComparisonKind")
        .and_then(Json::as_i64)
        .unwrap_or(0)
        != 0
    {
        return false;
    }
    let (Some(left), Some(raw)) = (
        cmp.get("Left"),
        cmp.get("Right")
            .and_then(|r| r.get("Literal"))
            .and_then(|l| l.get("Value"))
            .and_then(Json::as_str),
    ) else {
        return false;
    };
    let Some((entity, property)) = column_of(left) else {
        return false;
    };
    let Ok(literal) = LiteralValue::parse(raw) else {
        return false;
    };
    fields.iter().any(|(meta, value)| {
        let src = &meta.source;
        let prop_ok = src.property.as_deref() == Some(property);
        let entity_ok = match (entity, src.entity.as_deref()) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        };
        prop_ok && entity_ok && literal_eq(&literal, value)
    })
}

/// `(entity, property)` of a `Column` / `HierarchyLevel` expression.
fn column_of(expr: &Json) -> Option<(Option<&str>, &str)> {
    if let Some(col) = expr.get("Column") {
        let entity = col
            .get("Expression")
            .and_then(|e| e.get("SourceRef"))
            .and_then(|s| s.get("Entity"))
            .and_then(Json::as_str);
        return Some((entity, col.get("Property")?.as_str()?));
    }
    if let Some(level) = expr.get("HierarchyLevel") {
        return Some((None, level.get("Level")?.as_str()?));
    }
    None
}

/// Literal ↔ engine value equality (text exact, numbers numerically, dates
/// by their ISO text).
pub fn literal_eq(literal: &LiteralValue, value: &Value) -> bool {
    match (literal, value) {
        (LiteralValue::Null, Value::Blank) => true,
        (LiteralValue::Text(s), Value::String(v)) => s.as_str() == v.as_ref(),
        (LiteralValue::Boolean(a), Value::Boolean(b)) => a == b,
        (LiteralValue::Integer(i), v) if v.is_numeric() => v.as_f64() == Some(*i as f64),
        (LiteralValue::Double(d) | LiteralValue::Decimal(d), v) if v.is_numeric() => v
            .as_f64()
            .is_some_and(|x| (x - d).abs() <= 1e-9 * d.abs().max(1.0)),
        (LiteralValue::DateTime(iso), Value::DateTime(_)) => {
            let json = pbi_session::value_to_json(value);
            let text = json.as_str().unwrap_or("");
            let n = 19.min(iso.len()).min(text.len());
            n >= 10 && iso.get(..n) == text.get(..n)
        }
        _ => false,
    }
}

/// Assigns colors in order: explicit colors win, other items take
/// `default` when set, else the next palette color.
fn allocate(
    explicit: impl Iterator<Item = Option<Color>>,
    default: Option<Color>,
    resolver: &ThemeResolver,
) -> Vec<Color> {
    let mut next = 0usize;
    explicit
        .map(|c| match (c, default) {
            (Some(c), _) => c,
            (None, Some(d)) => d,
            (None, None) => {
                let c = resolver.data_color(next);
                next += 1;
                c
            }
        })
        .collect()
}

/// One color per category (pie/donut slices; bars with per-category
/// colors use [`DataPointColors::scoped_color`] directly).
pub fn category_colors(
    colors: &DataPointColors<'_>,
    data: &ChartData,
    resolver: &ThemeResolver,
) -> Vec<Color> {
    let explicit =
        (0..data.categories.len()).map(|i| colors.scoped_color(&data.category_fields(i)));
    allocate(explicit, None, resolver)
}

/// One color per series: series value / measure overrides, then
/// `defaultColor` (single-series charts), then the palette.
pub fn series_colors(
    colors: &DataPointColors<'_>,
    data: &ChartData,
    resolver: &ThemeResolver,
) -> Vec<Color> {
    // `defaultColor` only applies to a single series; several series keep
    // their palette colors.
    let default = if data.series.len() == 1 {
        colors.default_color()
    } else {
        None
    };
    let explicit = data.series.iter().map(|s| {
        let by_value = match (&data.series_meta, &s.series_value) {
            (Some(meta), Some(v)) => colors.scoped_color(&[(meta, v)]),
            _ => None,
        };
        by_value.or_else(|| colors.measure_color(&s.query_ref))
    });
    allocate(explicit, default, resolver)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn literals_compare_with_engine_values() {
        let text = LiteralValue::parse("'Online'").unwrap();
        assert!(literal_eq(&text, &Value::String("Online".into())));
        assert!(!literal_eq(&text, &Value::String("online".into())));
        let int = LiteralValue::parse("3L").unwrap();
        assert!(literal_eq(&int, &Value::Double(3.0)));
        assert!(literal_eq(&LiteralValue::Null, &Value::Blank));
        assert!(!literal_eq(&int, &Value::Blank));
    }

    #[test]
    fn default_and_measure_colors() {
        let objects = json!({"dataPoint": [
            {"properties": {"defaultColor": {"solid": {"color": "#112233"}}}},
            {"properties": {"fill": {"solid": {"color": "#445566"}}}, "selector": {"metadata": "T.M"}}
        ]});
        let r = ThemeResolver::default();
        let c = DataPointColors::new(&objects, &r);
        assert_eq!(c.default_color(), Some(Color::rgb(0x11, 0x22, 0x33)));
        assert_eq!(c.measure_color("T.M"), Some(Color::rgb(0x44, 0x55, 0x66)));
        assert_eq!(c.measure_color("T.X"), None);
    }
}
