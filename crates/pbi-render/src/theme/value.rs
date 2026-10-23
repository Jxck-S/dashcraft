//! Formatting property values in their two JSON shapes.
//!
//! - Report `objects`/`visualContainerObjects`: `{"expr": {"Literal":
//!   {"Value": "12D"}}}` (PBIR literal text: `12D`, `221L`, `'Segoe UI'`,
//!   `true`, `null`, `datetime'…'`).
//! - Theme `visualStyles`: bare JSON (`12`, `true`, `"Segoe UI"`; strings
//!   that are complete PBIR literals such as `"'Segoe UI'"` or `"12D"` are
//!   decoded as literals).
//!
//! Anything unexpected yields `None`; nothing here panics.

use pbi_model::LiteralValue;
use serde_json::Value;

/// Decodes a property value to a typed literal.
pub fn literal(value: &Value) -> Option<LiteralValue> {
    literal_depth(value, 0)
}

fn literal_depth(value: &Value, depth: u8) -> Option<LiteralValue> {
    if depth > 4 {
        return None;
    }
    match value {
        Value::Bool(b) => Some(LiteralValue::Boolean(*b)),
        Value::Number(n) => n
            .as_i64()
            .map(LiteralValue::Integer)
            .or_else(|| n.as_f64().map(LiteralValue::Double)),
        Value::String(s) => {
            Some(LiteralValue::parse(s).unwrap_or_else(|_| LiteralValue::Text(s.clone())))
        }
        Value::Null => Some(LiteralValue::Null),
        Value::Object(map) => {
            if let Some(inner) = map.get("expr") {
                return literal_depth(inner, depth + 1);
            }
            let raw = map.get("Literal")?.get("Value")?.as_str()?;
            LiteralValue::parse(raw).ok()
        }
        Value::Array(_) => None,
    }
}

/// Boolean value (`true`, `"true"`, `{"expr":{"Literal":{"Value":"true"}}}`).
pub fn as_bool(value: &Value) -> Option<bool> {
    match literal(value)? {
        LiteralValue::Boolean(b) => Some(b),
        _ => None,
    }
}

/// Numeric value (`12`, `"12D"`, `"221L"`, `"2.5M"`, numeric text). Non-finite
/// numbers yield `None`.
pub fn as_number(value: &Value) -> Option<f64> {
    let n = match literal(value)? {
        LiteralValue::Text(s) => s.trim().parse::<f64>().ok()?,
        other => other.as_f64()?,
    };
    n.is_finite().then_some(n)
}

/// Text value without quotes (`'Segoe UI'` → `Segoe UI`).
pub fn as_text(value: &Value) -> Option<String> {
    match literal(value)? {
        LiteralValue::Text(s) => Some(s),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lit(raw: &str) -> Value {
        json!({"expr": {"Literal": {"Value": raw}}})
    }

    #[test]
    fn decodes_report_and_theme_literals() {
        assert_eq!(as_number(&lit("12D")), Some(12.0));
        assert_eq!(as_number(&lit("221L")), Some(221.0));
        assert_eq!(as_number(&lit("2.5M")), Some(2.5));
        assert_eq!(as_number(&json!(12)), Some(12.0));
        assert_eq!(as_number(&json!(1.5)), Some(1.5));
        assert_eq!(as_number(&json!("12D")), Some(12.0));
        assert_eq!(as_number(&json!("12")), Some(12.0));
        assert_eq!(as_text(&lit("'Segoe UI'")).as_deref(), Some("Segoe UI"));
        assert_eq!(as_text(&json!("'Segoe UI'")).as_deref(), Some("Segoe UI"));
        assert_eq!(
            as_text(&json!("RightCenter")).as_deref(),
            Some("RightCenter")
        );
        assert_eq!(
            as_text(&json!(
                "'Segoe UI Semibold', wf_segoe-ui_semibold, helvetica, arial, sans-serif"
            ))
            .as_deref(),
            Some("'Segoe UI Semibold', wf_segoe-ui_semibold, helvetica, arial, sans-serif")
        );
        assert_eq!(as_bool(&lit("true")), Some(true));
        assert_eq!(as_bool(&json!(false)), Some(false));
        assert_eq!(as_bool(&json!("false")), Some(false));
        assert_eq!(as_bool(&lit("12D")), None);
        assert_eq!(as_number(&lit("'x'")), None);
        assert_eq!(as_number(&json!({"expr": {"Measure": {}}})), None);
        assert_eq!(as_number(&json!([1])), None);
        assert_eq!(as_number(&lit("1e999D")), None);
        assert_eq!(literal(&lit("garbage")), None);
    }
}
