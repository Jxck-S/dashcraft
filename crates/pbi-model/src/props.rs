//! Lossless property storage shared by all objects: [`PropertyBag`] for
//! properties the model does not type, [`Annotation`]s, and [`Origin`]s that
//! point back at the source file.

use std::fmt;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::names::names_eq;

/// Insertion-ordered map of properties the typed model does not cover.
///
/// Readers put every property they do not map to a typed field here, so
/// writers can emit it again. Keys use the source spelling (TMDL/TMSL/PBIR
/// property names, e.g. `isAvailableInMdx`).
pub type PropertyBag = IndexMap<String, PropValue>;

/// A value in a [`PropertyBag`].
///
/// The variants keep enough syntax to write the value back in its source
/// format:
/// - `Bool`: a bare TMDL flag (`isHidden`) or a JSON boolean.
/// - `Text`: a TMDL `name: value` scalar (raw text after the colon) or a
///   JSON string.
/// - `Expression`: a TMDL `name = expression` value (DAX/M, verbatim).
/// - `Json`: any other JSON value (TMSL/PBIR numbers, objects, arrays).
/// - `Object`: an unknown TMDL child object (`keyword name = value` plus
///   nested properties).
/// - `List`: several values under one key (e.g. repeated TMDL child objects
///   with the same keyword).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PropValue {
    Bool(bool),
    Text(String),
    Expression(String),
    Json(serde_json::Value),
    Object(Box<RawObject>),
    List(Vec<PropValue>),
}

impl PropValue {
    /// Text of a `Text` or `Expression` value, or the string in a JSON string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            PropValue::Text(s) | PropValue::Expression(s) => Some(s),
            PropValue::Json(serde_json::Value::String(s)) => Some(s),
            _ => None,
        }
    }

    /// Boolean value of a `Bool`, JSON boolean, or `true`/`false` text.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            PropValue::Bool(b) => Some(*b),
            PropValue::Json(serde_json::Value::Bool(b)) => Some(*b),
            PropValue::Text(s) if s.trim().eq_ignore_ascii_case("true") => Some(true),
            PropValue::Text(s) if s.trim().eq_ignore_ascii_case("false") => Some(false),
            _ => None,
        }
    }

    /// Best-effort conversion to JSON (for `inspect --json` style output).
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            PropValue::Bool(b) => serde_json::Value::Bool(*b),
            PropValue::Text(s) | PropValue::Expression(s) => serde_json::Value::String(s.clone()),
            PropValue::Json(v) => v.clone(),
            PropValue::Object(o) => {
                let mut map = serde_json::Map::new();
                if let Some(name) = &o.name {
                    map.insert("name".into(), serde_json::Value::String(name.clone()));
                }
                if let Some(expr) = &o.expression {
                    map.insert("expression".into(), serde_json::Value::String(expr.clone()));
                }
                for (k, v) in &o.properties {
                    map.insert(k.clone(), v.to_json());
                }
                serde_json::Value::Object(map)
            }
            PropValue::List(items) => {
                serde_json::Value::Array(items.iter().map(PropValue::to_json).collect())
            }
        }
    }
}

impl From<bool> for PropValue {
    fn from(value: bool) -> Self {
        PropValue::Bool(value)
    }
}

impl From<&str> for PropValue {
    fn from(value: &str) -> Self {
        PropValue::Text(value.to_owned())
    }
}

impl From<String> for PropValue {
    fn from(value: String) -> Self {
        PropValue::Text(value)
    }
}

impl From<serde_json::Value> for PropValue {
    fn from(value: serde_json::Value) -> Self {
        PropValue::Json(value)
    }
}

/// An object the model does not understand, kept for round-tripping
/// (e.g. a TMDL `extendedProperty`, `changedProperty` or a future keyword).
/// The keyword is the key under which it is stored in the parent bag.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RawObject {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Default value after `=` in TMDL (`keyword name = value`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub properties: PropertyBag,
}

/// A name/value annotation (TMDL `annotation X = …`, TMSL/PBIR
/// `annotations: [{name, value}]`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Annotation {
    pub name: String,
    pub value: String,
}

impl Annotation {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// Looks up an annotation by name (case-insensitive).
pub fn find_annotation<'a>(annotations: &'a [Annotation], name: &str) -> Option<&'a str> {
    annotations
        .iter()
        .find(|a| names_eq(&a.name, name))
        .map(|a| a.value.as_str())
}

/// Where an object came from, for diagnostics and "go to source".
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Origin {
    /// Path of the source file, relative to the opened container/folder
    /// (always `/`-separated).
    pub path: String,
    /// JSON pointer inside the file (PBIR/TMSL), e.g. `/filterConfig/filters/0`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
    /// 1-based line number (TMDL, M, DAX).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

impl Origin {
    pub fn file(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            pointer: None,
            line: None,
        }
    }

    pub fn with_pointer(mut self, pointer: impl Into<String>) -> Self {
        self.pointer = Some(pointer.into());
        self
    }

    pub fn with_line(mut self, line: u32) -> Self {
        self.line = Some(line);
        self
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.path)?;
        if let Some(line) = self.line {
            write!(f, ":{line}")?;
        }
        if let Some(pointer) = &self.pointer {
            write!(f, "#{pointer}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prop_value_accessors_and_json_projection() {
        let mut nested = RawObject {
            name: Some("PBI_ProTooling".into()),
            expression: Some("[\"DevMode\"]".into()),
            ..RawObject::default()
        };
        nested.properties.insert("isHidden".into(), true.into());
        let value = PropValue::Object(Box::new(nested));
        assert_eq!(
            value.to_json(),
            serde_json::json!({"name": "PBI_ProTooling", "expression": "[\"DevMode\"]", "isHidden": true})
        );
        assert_eq!(PropValue::from(" TRUE ").as_bool(), Some(true));
        assert_eq!(PropValue::Expression("1+1".into()).as_str(), Some("1+1"));
        assert_eq!(PropValue::Json(serde_json::json!(3)).as_str(), None);
    }

    #[test]
    fn annotations_are_found_case_insensitively() {
        let anns = vec![Annotation::new("PBI_ResultType", "Table")];
        assert_eq!(find_annotation(&anns, "pbi_resulttype"), Some("Table"));
        assert_eq!(find_annotation(&anns, "missing"), None);
    }

    #[test]
    fn origin_display_includes_line_and_pointer() {
        let o = Origin::file("definition/report.json").with_pointer("/filterConfig/filters/0");
        assert_eq!(
            o.to_string(),
            "definition/report.json#/filterConfig/filters/0"
        );
        assert_eq!(
            Origin::file("tables/A.tmdl").with_line(7).to_string(),
            "tables/A.tmdl:7"
        );
    }
}
