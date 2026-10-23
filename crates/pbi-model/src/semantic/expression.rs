//! Named (shared) expressions: M queries, parameters, DAX expressions.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::types::ExpressionKind;
use crate::ids::ObjectId;
use crate::names::names_eq;
use crate::props::{Annotation, Origin, PropertyBag};

/// A model-level named expression (TMDL `expression X = …`, TOM
/// `NamedExpression`). M parameters such as `StorageContainerUrl` are named
/// expressions whose metadata record contains `IsParameterQuery=true`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NamedExpression {
    pub id: ObjectId,
    pub name: String,
    pub kind: ExpressionKind,
    /// Expression text without the trailing `meta [...]` record (see
    /// [`split_m_meta`]). Readers that do not split keep the full text here;
    /// all helpers handle both.
    pub expression: String,
    /// The metadata record text, starting at `[` (without the `meta`
    /// keyword), e.g. `[IsParameterQuery=true, Type="Text"]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lineage_tag: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

impl NamedExpression {
    /// An M expression; a trailing `meta [...]` record is split off into
    /// [`NamedExpression::meta`].
    pub fn m(name: impl Into<String>, full_text: &str) -> Self {
        let (expression, meta) = split_m_meta(full_text);
        Self {
            name: name.into(),
            kind: ExpressionKind::M,
            expression: expression.to_owned(),
            meta: meta.map(str::to_owned),
            ..Self::default()
        }
    }

    /// Expression text including the metadata record (`value meta [...]`),
    /// as it appears in TMDL/TMSL.
    pub fn full_expression(&self) -> String {
        match &self.meta {
            Some(meta) => format!("{} meta {}", self.expression, meta),
            None => self.expression.clone(),
        }
    }

    /// Fields of the metadata record (`IsParameterQuery` → `true`, `Type` →
    /// `Text`, …). String values are unquoted. Uses [`Self::meta`] or, if
    /// that is empty, a `meta [...]` suffix of [`Self::expression`].
    pub fn meta_record(&self) -> IndexMap<String, String> {
        if let Some(meta) = &self.meta {
            return parse_m_record(meta);
        }
        match split_m_meta(&self.expression) {
            (_, Some(meta)) => parse_m_record(meta),
            _ => IndexMap::new(),
        }
    }

    /// `true` for Power Query parameters (`IsParameterQuery=true`, any case
    /// or spacing).
    pub fn is_parameter(&self) -> bool {
        self.meta_value("IsParameterQuery")
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
    }

    /// Parameter type from the metadata record (`Text`, `Number`, …).
    pub fn parameter_type(&self) -> Option<String> {
        self.meta_value("Type")
    }

    /// Value of one metadata field (key compared case-insensitively).
    pub fn meta_value(&self, key: &str) -> Option<String> {
        self.meta_record()
            .into_iter()
            .find(|(k, _)| names_eq(k, key))
            .map(|(_, v)| v)
    }
}

/// Calls `f(index, byte, depth_before)` for every byte of M `text` that is
/// outside string literals, quoted identifiers (`#"…"`) and comments.
fn for_each_m_code_byte(text: &str, mut f: impl FnMut(usize, u8, usize)) {
    fn skip_string(b: &[u8], mut i: usize) -> usize {
        while let Some(&c) = b.get(i) {
            if c == b'"' {
                if b.get(i + 1) == Some(&b'"') {
                    i += 2;
                    continue;
                }
                return i + 1;
            }
            i += 1;
        }
        b.len()
    }

    let b = text.as_bytes();
    let mut i = 0;
    let mut depth = 0usize;
    while let Some(&c) = b.get(i) {
        match (c, b.get(i + 1)) {
            (b'"', _) => {
                i = skip_string(b, i + 1);
                continue;
            }
            (b'#', Some(b'"')) => {
                i = skip_string(b, i + 2);
                continue;
            }
            (b'/', Some(b'/')) => {
                while b.get(i).is_some_and(|&c| c != b'\n') {
                    i += 1;
                }
                continue;
            }
            (b'/', Some(b'*')) => {
                i += 2;
                while let Some(&c) = b.get(i) {
                    if c == b'*' && b.get(i + 1) == Some(&b'/') {
                        break;
                    }
                    i += 1;
                }
                i = (i + 2).min(b.len());
                continue;
            }
            _ => {}
        }
        f(i, c, depth);
        match c {
            b'[' | b'(' | b'{' => depth += 1,
            b']' | b')' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
}

fn is_ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'.' || c >= 0x80
}

/// Splits a trailing top-level `meta [ … ]` record off an M expression.
///
/// Returns `(value, Some(record))` where `record` starts at `[`, e.g.
/// `"https://x" meta [IsParameterQuery=true]` →
/// `("\"https://x\"", Some("[IsParameterQuery=true]"))`. Strings, quoted
/// identifiers, comments and nested brackets are respected; text with no
/// trailing record is returned unchanged with `None`.
pub fn split_m_meta(text: &str) -> (&str, Option<&str>) {
    let bytes = text.as_bytes();
    let end = text.trim_end().len();
    let mut metas = Vec::new();
    let mut top_open: Option<(usize, u8)> = None;
    let mut record_at_end: Option<usize> = None;
    for_each_m_code_byte(text, |i, c, depth| {
        if depth == 0 {
            if c == b'm'
                && bytes.get(i..i + 4) == Some(b"meta".as_slice())
                && (i == 0 || bytes.get(i - 1).is_some_and(|&p| !is_ident_byte(p)))
                && bytes.get(i + 4).is_none_or(|&n| !is_ident_byte(n))
            {
                metas.push(i);
            }
            if matches!(c, b'[' | b'(' | b'{') {
                top_open = Some((i, c));
            }
        }
        if c == b']' && depth == 1 && i + 1 == end {
            record_at_end = match top_open {
                Some((open, b'[')) => Some(open),
                _ => None,
            };
        }
    });
    let Some(open) = record_at_end else {
        return (text, None);
    };
    for m in metas {
        let between = text.get(m + 4..open);
        if between.is_some_and(|s| s.trim().is_empty())
            && let (Some(value), Some(record)) = (text.get(..m), text.get(open..end))
        {
            return (value.trim_end(), Some(record));
        }
    }
    (text, None)
}

/// Parses a simple M record literal (`[A=1, B="x"]`) into field → raw value.
/// String values are unquoted (`""` → `"`), `#"quoted"` keys are unquoted;
/// other values (numbers, nested records, lists) are kept as source text.
/// Malformed input yields the fields that could be read.
pub fn parse_m_record(text: &str) -> IndexMap<String, String> {
    let mut out = IndexMap::new();
    let t = text.trim();
    let Some(inner) = t.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
        return out;
    };
    let mut commas = Vec::new();
    for_each_m_code_byte(inner, |i, c, depth| {
        if depth == 0 && c == b',' {
            commas.push(i);
        }
    });
    let mut start = 0;
    let bounds = commas.into_iter().chain(std::iter::once(inner.len()));
    for stop in bounds {
        let field = inner.get(start..stop).unwrap_or_default();
        start = stop + 1;
        let mut eq = None;
        for_each_m_code_byte(field, |i, c, depth| {
            if depth == 0 && c == b'=' && eq.is_none() {
                eq = Some(i);
            }
        });
        let Some(eq) = eq else { continue };
        let (Some(key), Some(value)) = (field.get(..eq), field.get(eq + 1..)) else {
            continue;
        };
        let key = unquote_m(key.trim(), true);
        if key.is_empty() {
            continue;
        }
        out.insert(key, unquote_m(value.trim(), false));
    }
    out
}

fn unquote_m(text: &str, allow_hash: bool) -> String {
    let body = if allow_hash {
        text.strip_prefix('#')
            .filter(|s| s.starts_with('"'))
            .unwrap_or(text)
    } else {
        text
    };
    if body.len() >= 2 && body.starts_with('"') && body.ends_with('"') {
        body[1..body.len() - 1].replace("\"\"", "\"")
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOB: &str = r#""https://acmeoutfittersdemo.blob.core.windows.net/sample-data" meta [IsParameterQuery=true, Type="Text", IsParameterQueryRequired=true]"#;

    #[test]
    fn splits_parameter_meta_record() {
        let (value, meta) = split_m_meta(BLOB);
        assert_eq!(
            value,
            r#""https://acmeoutfittersdemo.blob.core.windows.net/sample-data""#
        );
        assert_eq!(
            meta,
            Some(r#"[IsParameterQuery=true, Type="Text", IsParameterQueryRequired=true]"#)
        );

        let e = NamedExpression::m("StorageContainerUrl", BLOB);
        assert!(e.is_parameter());
        assert_eq!(e.parameter_type().as_deref(), Some("Text"));
        assert_eq!(e.full_expression(), BLOB);
    }

    #[test]
    fn meta_inside_strings_comments_or_nested_code_is_ignored() {
        for text in [
            r#""a meta [b=1]""#,
            "let x = 1 meta [A=1] in x + 1",
            "// meta [IsParameterQuery=true]\n1",
            r##"#"meta [x]""##,
            "f(1 meta [A=1])",
            "1 meta [A=1] // trailing comment",
        ] {
            assert_eq!(split_m_meta(text), (text, None), "text {text:?}");
        }
        let (value, meta) = split_m_meta("\"x\" meta [a=1] meta [b=\"]\"]\n");
        assert_eq!(value, "\"x\" meta [a=1]");
        assert_eq!(meta, Some("[b=\"]\"]"));
    }

    #[test]
    fn is_parameter_tolerates_spacing_case_and_unsplit_text() {
        let spaced = NamedExpression {
            name: "P".into(),
            expression: "42".into(),
            meta: Some("[ isparameterquery = TRUE , Type = \"Number\" ]".into()),
            ..NamedExpression::default()
        };
        assert!(spaced.is_parameter());
        assert_eq!(spaced.parameter_type().as_deref(), Some("Number"));

        // A reader that did not split keeps everything in `expression`.
        let unsplit = NamedExpression {
            expression: BLOB.into(),
            ..NamedExpression::default()
        };
        assert!(unsplit.is_parameter());

        let not_param = NamedExpression::m("Q", "let Source = 1 in Source");
        assert!(!not_param.is_parameter());
        let explicit_false = NamedExpression::m("Q", "1 meta [IsParameterQuery=false]");
        assert!(!explicit_false.is_parameter());
    }

    #[test]
    fn record_parser_handles_nested_values_and_quoted_keys() {
        let rec = parse_m_record(
            r##"[#"Odd, Key"="a ""b"", c", List={1, 2}, Nested=[X=1, Y=2], Flag=true]"##,
        );
        assert_eq!(rec.get("Odd, Key").map(String::as_str), Some("a \"b\", c"));
        assert_eq!(rec.get("List").map(String::as_str), Some("{1, 2}"));
        assert_eq!(rec.get("Nested").map(String::as_str), Some("[X=1, Y=2]"));
        assert_eq!(rec.get("Flag").map(String::as_str), Some("true"));
        assert!(parse_m_record("not a record").is_empty());
    }
}
