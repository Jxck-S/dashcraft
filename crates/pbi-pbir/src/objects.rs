//! Read helpers for formatting `objects` / `visualContainerObjects` (kept
//! as raw JSON in the typed structs and the report AST).
//!
//! Shape: `{"<object>": [{"properties": {"<prop>": <value>}, "selector": {…}}]}`
//! where a value is usually `{"expr": {"Literal": {"Value": "…"}}}` or a
//! color (`{"solid": {"color": …}}`, see [`crate::theme::resolve_color`]).

use pbi_model::{Literal, LiteralValue};
use serde_json::Value;

/// Entries of `object` (`objects[object][]`): `(properties, selector)`.
pub fn entries<'a>(
    objects: &'a Value,
    object: &str,
) -> impl Iterator<Item = (&'a Value, Option<&'a Value>)> + 'a {
    objects
        .get(object)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| Some((entry.get("properties")?, entry.get("selector"))))
}

/// `true` for the default selector (no selector, or `{"id": "default"}`).
pub fn is_default_selector(selector: Option<&Value>) -> bool {
    match selector {
        None => true,
        Some(s) => {
            s.as_object().is_some_and(serde_json::Map::is_empty)
                || s.get("id").and_then(Value::as_str) == Some("default")
        }
    }
}

/// Raw value of `objects[object][*].properties[property]` for the default
/// selector (the first such entry that has the property).
pub fn property<'a>(objects: &'a Value, object: &str, property: &str) -> Option<&'a Value> {
    entries(objects, object)
        .filter(|(_, selector)| is_default_selector(*selector))
        .find_map(|(props, _)| props.get(property))
}

/// Literal of a property value (`{"expr": {"Literal": {"Value": "…"}}}`).
pub fn literal_of(value: &Value) -> Option<Literal> {
    let raw = value
        .get("expr")
        .unwrap_or(value)
        .get("Literal")?
        .get("Value")?
        .as_str()?;
    Some(Literal::new(raw))
}

/// Typed literal value of a default-selector property.
pub fn literal_property(objects: &Value, object: &str, prop: &str) -> Option<LiteralValue> {
    literal_of(property(objects, object, prop)?)?.value().ok()
}

/// Boolean property (`show`, `wordWrap`, …).
pub fn bool_property(objects: &Value, object: &str, prop: &str) -> Option<bool> {
    match literal_property(objects, object, prop)? {
        LiteralValue::Boolean(b) => Some(b),
        _ => None,
    }
}

/// Numeric property (`fontSize`, `transparency`, …).
pub fn number_property(objects: &Value, object: &str, prop: &str) -> Option<f64> {
    literal_property(objects, object, prop)?.as_f64()
}

/// Text property (`fontFamily`, `tileShape`, …), without quotes.
pub fn text_property(objects: &Value, object: &str, prop: &str) -> Option<String> {
    match literal_property(objects, object, prop)? {
        LiteralValue::Text(s) => Some(s),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_default_selector_literals() {
        let objects = json!({
            "border": [{"properties": {"show": {"expr": {"Literal": {"Value": "false"}}}}, "selector": {"id": "default"}}],
            "value": [
                {"properties": {"fontSize": {"expr": {"Literal": {"Value": "99D"}}}}, "selector": {"metadata": "x"}},
                {"properties": {"fontSize": {"expr": {"Literal": {"Value": "24D"}}},
                                "fontFamily": {"expr": {"Literal": {"Value": "'Calibri'"}}}}}
            ],
            "bad": "not an array"
        });
        assert_eq!(bool_property(&objects, "border", "show"), Some(false));
        assert_eq!(number_property(&objects, "value", "fontSize"), Some(24.0));
        assert_eq!(
            text_property(&objects, "value", "fontFamily").as_deref(),
            Some("Calibri")
        );
        assert_eq!(bool_property(&objects, "bad", "show"), None);
        assert_eq!(bool_property(&Value::Null, "border", "show"), None);
    }
}
