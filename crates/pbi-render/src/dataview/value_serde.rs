//! JSON (de)serialization of engine [`Value`]s.
//!
//! Serialization uses [`pbi_session::value_to_json`]: BLANK → `null`,
//! numbers as numbers, text as strings, dates as ISO text
//! `YYYY-MM-DDThh:mm:ss[.ffffff]`. Deserialization is the lenient inverse
//! for scalars coming from the UI: `null` → BLANK, booleans, integers →
//! `Integer`, other numbers → `Double`, strings → text. Dates therefore
//! come back as text; the query engine coerces filter values to the
//! column's data type, so ISO text selects a date.

use std::sync::Arc;

use pbi_engine::Value;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value as Json;

/// Serializes one value.
///
/// # Errors
/// Propagates the serializer's errors.
pub fn serialize<S: Serializer>(value: &Value, s: S) -> Result<S::Ok, S::Error> {
    pbi_session::value_to_json(value).serialize(s)
}

/// Deserializes one value from a JSON scalar.
///
/// # Errors
/// Arrays and objects are rejected.
pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Value, D::Error> {
    let json = Json::deserialize(d)?;
    from_json(&json).map_err(D::Error::custom)
}

/// The engine value of a JSON scalar (see the module docs).
///
/// # Errors
/// A message for arrays and objects.
pub fn from_json(json: &Json) -> Result<Value, String> {
    Ok(match json {
        Json::Null => Value::Blank,
        Json::Bool(b) => Value::Boolean(*b),
        Json::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => Value::Integer(i),
            (None, Some(f)) => Value::Double(f),
            _ => return Err(format!("unsupported number {n}")),
        },
        Json::String(s) => Value::String(Arc::from(s.as_str())),
        Json::Array(_) | Json::Object(_) => {
            return Err("expected a scalar (string, number, boolean or null)".into());
        }
    })
}

/// `Vec<Value>` helpers.
pub mod vec {
    use super::*;

    /// # Errors
    /// Propagates the serializer's errors.
    pub fn serialize<S: Serializer>(values: &[Value], s: S) -> Result<S::Ok, S::Error> {
        values
            .iter()
            .map(pbi_session::value_to_json)
            .collect::<Vec<_>>()
            .serialize(s)
    }

    /// # Errors
    /// Non-scalar items.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Value>, D::Error> {
        let items = Vec::<Json>::deserialize(d)?;
        items
            .iter()
            .map(|j| from_json(j).map_err(D::Error::custom))
            .collect()
    }
}

/// `Vec<Vec<Value>>` (tuples) helpers.
pub mod tuples {
    use super::*;

    /// # Errors
    /// Propagates the serializer's errors.
    pub fn serialize<S: Serializer>(values: &[Vec<Value>], s: S) -> Result<S::Ok, S::Error> {
        values
            .iter()
            .map(|t| t.iter().map(pbi_session::value_to_json).collect::<Vec<_>>())
            .collect::<Vec<_>>()
            .serialize(s)
    }

    /// Accepts tuples (`[["Camping"], ["Hiking"]]`) or bare scalars
    /// (`["Camping", "Hiking"]`, one-column selections).
    ///
    /// # Errors
    /// Nested arrays deeper than one level, objects.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Vec<Value>>, D::Error> {
        let items = Vec::<Json>::deserialize(d)?;
        items
            .iter()
            .map(|item| match item {
                Json::Array(parts) => parts
                    .iter()
                    .map(|j| from_json(j).map_err(D::Error::custom))
                    .collect(),
                scalar => from_json(scalar).map(|v| vec![v]).map_err(D::Error::custom),
            })
            .collect()
    }
}
