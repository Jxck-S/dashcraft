//! Lossless JSON plumbing shared by every PBIR type.
//!
//! - [`json_object!`] declares a typed JSON object whose known properties are
//!   `Option<T>` fields and whose unknown properties (and explicit `null`s)
//!   are kept in an `extra` map. The source key order is recorded in a
//!   [`KeyOrder`] so serialization writes keys back exactly as read.
//!   Numbers are kept as [`serde_json::Number`], which (with the workspace's
//!   `arbitrary_precision` feature) preserves the source digits.
//! - [`JsonStyle`] detects and reproduces a file's formatting (indent,
//!   compact vs pretty, line endings, trailing newline, BOM, `\u` escaping).
//! - [`json_identical`] compares two values *including* object key order
//!   (`serde_json::Value`'s own `==` ignores key order).

use std::fmt;

use serde::Serialize;
use serde_json::Value;

/// Insertion-ordered JSON object (`preserve_order` is enabled workspace-wide).
pub type JsonMap = serde_json::Map<String, Value>;

/// Source order of an object's keys.
///
/// Equality always holds: key order is presentation, not content, so two
/// objects with the same properties compare equal regardless of order.
#[derive(Clone, Default)]
pub struct KeyOrder(Vec<String>);

impl KeyOrder {
    pub fn with_capacity(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    pub fn push(&mut self, key: String) {
        self.0.push(key);
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Forgets the recorded order (keys are then written in declaration
    /// order followed by `extra`).
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

impl PartialEq for KeyOrder {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl fmt::Debug for KeyOrder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(&self.0).finish()
    }
}

/// Deserializes one property value, prefixing errors with the key so nested
/// failures read like a path (`visual: query: queryState: …`).
pub(crate) fn from_field<T: serde::de::DeserializeOwned>(
    key: &str,
    value: Value,
) -> Result<T, serde_json::Error> {
    serde_json::from_value(value)
        .map_err(|e| <serde_json::Error as serde::de::Error>::custom(format!("{key}: {e}")))
}

/// Declares a lossless JSON object type. See the [module docs](self).
///
/// ```ignore
/// json_object! {
///     /// Docs.
///     pub struct Foo {
///         /// Field docs.
///         "jsonKey" => rust_field: Type,
///     }
/// }
/// ```
macro_rules! json_object {
    (
        $(#[$meta:meta])*
        pub struct $name:ident {
            $(
                $(#[$fmeta:meta])*
                $key:literal => $field:ident : $ty:ty
            ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Default)]
        pub struct $name {
            $( $(#[$fmeta])* pub $field: Option<$ty>, )*
            /// Properties not modelled above, plus explicit `null`s, in
            /// source order.
            pub extra: $crate::json::JsonMap,
            /// Source key order, used to write keys back in the same order.
            pub key_order: $crate::json::KeyOrder,
        }

        impl $name {
            /// JSON property names modelled as typed fields.
            #[allow(dead_code)]
            pub const KEYS: &'static [&'static str] = &[$($key),*];

            /// Builds the value from a JSON object, recording key order.
            pub fn from_map(map: $crate::json::JsonMap) -> Result<Self, serde_json::Error> {
                let mut out = Self::default();
                out.key_order = $crate::json::KeyOrder::with_capacity(map.len());
                for (key, value) in map {
                    out.key_order.push(key.clone());
                    if value.is_null() {
                        out.extra.insert(key, value);
                        continue;
                    }
                    match key.as_str() {
                        $( $key => out.$field = Some($crate::json::from_field($key, value)?), )*
                        _ => {
                            out.extra.insert(key, value);
                        }
                    }
                }
                Ok(out)
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(None)?;
                let mut done: std::collections::HashSet<&str> = std::collections::HashSet::new();
                for key in self.key_order.iter() {
                    if done.contains(key) {
                        continue;
                    }
                    #[allow(clippy::match_single_binding)]
                    let written = match key {
                        $( $key => match &self.$field {
                            Some(v) => {
                                map.serialize_entry($key, v)?;
                                true
                            }
                            None => false,
                        }, )*
                        _ => false,
                    };
                    if written {
                        done.insert(key);
                    } else if let Some((k, v)) = self.extra.get_key_value(key) {
                        map.serialize_entry(k, v)?;
                        done.insert(k.as_str());
                    }
                }
                $(
                    if !done.contains($key) {
                        if let Some(v) = &self.$field {
                            map.serialize_entry($key, v)?;
                            done.insert($key);
                        }
                    }
                )*
                for (k, v) in &self.extra {
                    if !done.contains(k.as_str()) {
                        map.serialize_entry(k, v)?;
                    }
                }
                map.end()
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let map = <$crate::json::JsonMap as serde::Deserialize>::deserialize(deserializer)?;
                Self::from_map(map).map_err(<D::Error as serde::de::Error>::custom)
            }
        }
    };
}

pub(crate) use json_object;

/// `true` if `a` and `b` are equal **including** the key order of every
/// object (and the exact digits of every number).
pub fn json_identical(a: &Value, b: &Value) -> bool {
    first_difference(a, b).is_none()
}

/// JSON pointer of the first difference between `a` and `b` (key order
/// counts), or `None` if they are identical.
pub fn first_difference(a: &Value, b: &Value) -> Option<String> {
    fn walk(a: &Value, b: &Value, path: &mut String) -> bool {
        match (a, b) {
            (Value::Object(x), Value::Object(y)) => {
                if x.len() != y.len() || !x.keys().eq(y.keys()) {
                    return false;
                }
                for ((k, va), vb) in x.iter().zip(y.values()) {
                    let len = path.len();
                    path.push('/');
                    path.push_str(&k.replace('~', "~0").replace('/', "~1"));
                    if !walk(va, vb, path) {
                        return false;
                    }
                    path.truncate(len);
                }
                true
            }
            (Value::Array(x), Value::Array(y)) => {
                if x.len() != y.len() {
                    return false;
                }
                for (i, (va, vb)) in x.iter().zip(y).enumerate() {
                    let len = path.len();
                    path.push('/');
                    path.push_str(&i.to_string());
                    if !walk(va, vb, path) {
                        return false;
                    }
                    path.truncate(len);
                }
                true
            }
            // `Number` equality compares the stored text under
            // `arbitrary_precision`, so `1` and `1.0` differ.
            _ => a == b,
        }
    }
    let mut path = String::new();
    if walk(a, b, &mut path) {
        None
    } else {
        Some(path)
    }
}

/// Formatting of a JSON file, detected from its bytes so the writer can
/// reproduce it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonStyle {
    /// Indent unit for pretty output; `None` for compact (single-line) JSON.
    pub indent: Option<String>,
    /// Use `\r\n` line endings.
    pub crlf: bool,
    /// End the file with a line break.
    pub trailing_newline: bool,
    /// Start with a UTF-8 byte-order mark.
    pub bom: bool,
    /// Escape non-ASCII characters as `\uXXXX`.
    pub ascii_only: bool,
    /// Use uppercase hex digits in those escapes (`\u00E9`).
    pub upper_hex_escapes: bool,
}

impl Default for JsonStyle {
    /// Power BI's PBIR style: 2-space indent, LF, no trailing newline.
    fn default() -> Self {
        Self {
            indent: Some("  ".to_owned()),
            crlf: false,
            trailing_newline: false,
            bom: false,
            ascii_only: false,
            upper_hex_escapes: false,
        }
    }
}

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

impl JsonStyle {
    /// Compact style (as used for PBIR files inside a PBIX and for themes).
    pub fn compact() -> Self {
        Self {
            indent: None,
            ..Self::default()
        }
    }

    /// Detects the style of `bytes` (a UTF-8 JSON document).
    pub fn detect(bytes: &[u8]) -> Self {
        let bom = bytes.starts_with(UTF8_BOM);
        let body = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
        let trimmed_end = body
            .iter()
            .rposition(|b| !matches!(b, b'\n' | b'\r'))
            .map_or(0, |i| i + 1);
        let trailing_newline = trimmed_end < body.len();
        let content = &body[..trimmed_end];
        let crlf = body.windows(2).any(|w| w == b"\r\n");
        let indent = content.iter().position(|&b| b == b'\n').map(|nl| {
            let rest = &content[nl + 1..];
            let n = rest
                .iter()
                .take_while(|b| matches!(b, b' ' | b'\t'))
                .count();
            String::from_utf8_lossy(&rest[..n]).into_owned()
        });
        let escapes = non_ascii_escapes(content);
        let ascii_only = body.is_ascii() && escapes.is_some();
        Self {
            indent,
            crlf,
            trailing_newline,
            bom,
            ascii_only,
            upper_hex_escapes: ascii_only && escapes == Some(true),
        }
    }

    /// Serializes `value` in this style.
    pub fn to_vec<T: Serialize + ?Sized>(&self, value: &T) -> Result<Vec<u8>, serde_json::Error> {
        let mut out = Vec::new();
        if self.bom {
            out.extend_from_slice(UTF8_BOM);
        }
        let body = match &self.indent {
            Some(indent) => {
                let mut buf = Vec::new();
                let formatter = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
                let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
                value.serialize(&mut ser)?;
                buf
            }
            None => serde_json::to_vec(value)?,
        };
        let body = if self.ascii_only {
            escape_non_ascii(&body, self.upper_hex_escapes)
        } else {
            body
        };
        let newline: &[u8] = if self.crlf { b"\r\n" } else { b"\n" };
        if self.crlf {
            // Raw line breaks only occur between tokens (strings escape them).
            for &b in &body {
                if b == b'\n' {
                    out.extend_from_slice(newline);
                } else {
                    out.push(b);
                }
            }
        } else {
            out.extend_from_slice(&body);
        }
        if self.trailing_newline {
            out.extend_from_slice(newline);
        }
        Ok(out)
    }
}

/// If the text contains `\uXXXX` escapes of non-ASCII code units:
/// `Some(true)` when their hex letters are uppercase, else `Some(false)`.
fn non_ascii_escapes(content: &[u8]) -> Option<bool> {
    let mut found = None;
    let mut i = 0;
    while i + 6 <= content.len() {
        if content[i] == b'\\' {
            let hex = &content[i + 2..i + 6];
            if content[i + 1] == b'u'
                && let Ok(text) = std::str::from_utf8(hex)
                && u16::from_str_radix(text, 16).is_ok_and(|c| c >= 0x80)
            {
                if hex.iter().any(u8::is_ascii_uppercase) {
                    return Some(true);
                }
                if hex.iter().any(u8::is_ascii_lowercase) {
                    return Some(false);
                }
                found = Some(false);
            }
            // Skip the escaped character so `\\u` is not misread.
            i += 2;
        } else {
            i += 1;
        }
    }
    found
}

/// Replaces every non-ASCII character of serialized JSON with `\uXXXX`
/// (surrogate pairs above the BMP). Non-ASCII only occurs inside strings.
fn escape_non_ascii(json: &[u8], upper: bool) -> Vec<u8> {
    let text = String::from_utf8_lossy(json);
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units) {
                if upper {
                    out.push_str(&format!("\\u{unit:04X}"));
                } else {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    out.into_bytes()
}

/// Strips a UTF-8 BOM.
pub(crate) fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes)
}

/// Converts a JSON number to `f64` (`None` if it is not finite).
pub fn num_f64(n: &serde_json::Number) -> Option<f64> {
    n.as_f64().filter(|f| f.is_finite())
}

/// Converts a JSON number to `i64`, accepting integral floats (`3.0`).
pub fn num_i64(n: &serde_json::Number) -> Option<i64> {
    n.as_i64().or_else(|| {
        let f = num_f64(n)?;
        #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
        let i = f as i64;
        #[allow(clippy::cast_precision_loss, clippy::float_cmp)]
        (i as f64 == f).then_some(i)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    json_object! {
        /// Test object.
        pub struct Probe {
            "a" => a: serde_json::Number,
            "b" => b: String,
            "nested" => nested: Box<Probe>,
        }
    }

    #[test]
    fn objects_round_trip_with_order_unknowns_and_nulls() {
        let src = r#"{"z":1,"b":"x","nested":{"b":"y","a":1.50},"a":null,"q":[1,2]}"#;
        let v: Value = serde_json::from_str(src).unwrap();
        let p: Probe = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(p.b.as_deref(), Some("x"));
        assert!(p.a.is_none());
        assert_eq!(
            p.nested.as_ref().unwrap().a.as_ref().unwrap().to_string(),
            "1.50"
        );
        let back = serde_json::to_value(&p).unwrap();
        assert!(json_identical(&back, &v), "{back}");
        assert_eq!(serde_json::to_string(&p).unwrap(), src);
    }

    #[test]
    fn new_fields_and_edits_serialize_deterministically() {
        let mut p: Probe = serde_json::from_str(r#"{"q":0,"b":"x","a":null}"#).unwrap();
        p.a = Some(serde_json::Number::from(7));
        p.nested = Some(Box::default());
        assert_eq!(
            serde_json::to_string(&p).unwrap(),
            r#"{"q":0,"b":"x","a":7,"nested":{}}"#
        );
        let fresh = Probe {
            b: Some("v".into()),
            ..Probe::default()
        };
        assert_eq!(serde_json::to_string(&fresh).unwrap(), r#"{"b":"v"}"#);
    }

    #[test]
    fn wrong_types_report_a_path() {
        let err = serde_json::from_str::<Probe>(r#"{"nested":{"b":5}}"#).unwrap_err();
        assert!(err.to_string().contains("nested: b: invalid type"), "{err}");
        assert!(serde_json::from_str::<Probe>("[1]").is_err());
    }

    #[test]
    fn key_order_matters_for_identity_only() {
        let a: Value = serde_json::from_str(r#"{"x":1,"y":[{"p":1,"q":2}]}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"x":1,"y":[{"q":2,"p":1}]}"#).unwrap();
        assert_eq!(a, b);
        assert!(!json_identical(&a, &b));
        assert_eq!(first_difference(&a, &b).as_deref(), Some("/y/0"));
        let c: Value = serde_json::from_str("1.0").unwrap();
        let d: Value = serde_json::from_str("1").unwrap();
        assert!(!json_identical(&c, &d));
    }

    #[test]
    fn styles_are_detected_and_reproduced() {
        let cases: [&[u8]; 7] = [
            b"{\n  \"a\": [\n    1\n  ],\n  \"b\": {}\n}",
            b"{\"a\":[1],\"b\":{}}",
            b"{\r\n    \"a\": 1\r\n}\r\n",
            b"\xEF\xBB\xBF{\n\t\"a\": \"\xC3\xA9\"\n}\n",
            b"{\n  \"a\": \"\\u00e9\\ud83d\\ude00\"\n}",
            b"{\n  \"a\": []\n}",
            b"{\"a\":\"\\u00E9\\u0080\"}",
        ];
        for bytes in cases {
            let style = JsonStyle::detect(bytes);
            let v: Value = serde_json::from_slice(strip_bom(bytes)).unwrap();
            let out = style.to_vec(&v).unwrap();
            assert_eq!(
                String::from_utf8_lossy(&out),
                String::from_utf8_lossy(bytes),
                "{style:?}"
            );
        }
        assert_eq!(JsonStyle::detect(b"{}"), JsonStyle::compact());
    }

    #[test]
    fn number_helpers() {
        let n = |s: &str| serde_json::from_str::<serde_json::Number>(s).unwrap();
        assert_eq!(num_i64(&n("3")), Some(3));
        assert_eq!(num_i64(&n("3.0")), Some(3));
        assert_eq!(num_i64(&n("3.5")), None);
        assert_eq!(num_f64(&n("1e400")), None);
        assert_eq!(num_f64(&n("-0.6")), Some(-0.6));
    }
}
