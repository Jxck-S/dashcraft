//! Property codecs: how a typed model field is decoded from, and encoded
//! to, a TMSL JSON value.
//!
//! The reader and the writer share these, which is what makes the "shadow"
//! mechanism exact: the reader stores the original JSON in `extra` whenever
//! `encode(decode(original))` differs from the original (explicit default,
//! single-line text written as an array, multi-line text written as a
//! string, non-canonical enum spelling, …), and the writer re-emits that
//! original only while it still decodes to the current typed value.

use pbi_model::semantic::split_m_meta;
use pbi_model::{
    Annotation, Cardinality, CrossFilter, DataType, ExpressionKind, PartitionMode,
    SecurityFilteringBehavior, SummarizeBy,
};
use serde_json::{Map, Value};

use crate::error::TmslError;

pub(crate) trait Codec {
    type T;
    /// Decodes a property value; `None` means the key is absent.
    fn decode(&self, value: Option<&Value>, pointer: &str) -> Result<Self::T, TmslError>;
    /// Encodes a typed value; `None` means the key is omitted.
    fn encode(&self, value: &Self::T) -> Option<Value>;
}

/// Decodes a TMSL text value: a string, or an array of strings (lines)
/// joined with `\n`.
pub(crate) fn decode_lines(value: &Value, pointer: &str) -> Result<String, TmslError> {
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Array(items) => {
            let mut out = String::new();
            for (i, item) in items.iter().enumerate() {
                let Value::String(line) = item else {
                    return Err(TmslError::unexpected(
                        &format!("{pointer}/{i}"),
                        "a string",
                        item,
                    ));
                };
                if i > 0 {
                    out.push('\n');
                }
                out.push_str(line);
            }
            Ok(out)
        }
        other => Err(TmslError::unexpected(
            pointer,
            "a string or an array of strings",
            other,
        )),
    }
}

/// Encodes text the way the TOM serializer does with
/// `SplitMultilineStrings`: an array of lines when it contains `\n` (and
/// `multiline` is set), otherwise a string.
pub(crate) fn encode_lines(text: &str, multiline: bool) -> Value {
    if multiline && text.contains('\n') {
        Value::Array(
            text.split('\n')
                .map(|line| Value::String(line.to_owned()))
                .collect(),
        )
    } else {
        Value::String(text.to_owned())
    }
}

/// Optional text (`description`, `formatString`, …).
#[derive(Debug, Clone, Copy)]
pub(crate) struct OptText {
    pub multiline: bool,
}

pub(crate) const OPT_SINGLE: OptText = OptText { multiline: false };
pub(crate) const OPT_MULTI: OptText = OptText { multiline: true };

impl Codec for OptText {
    type T = Option<String>;
    fn decode(&self, value: Option<&Value>, pointer: &str) -> Result<Self::T, TmslError> {
        value.map(|v| decode_lines(v, pointer)).transpose()
    }
    fn encode(&self, value: &Self::T) -> Option<Value> {
        value.as_deref().map(|s| encode_lines(s, self.multiline))
    }
}

/// Text stored as a plain `String` in the model; empty means absent.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ReqText {
    pub multiline: bool,
}

pub(crate) const REQ_SINGLE: ReqText = ReqText { multiline: false };
pub(crate) const REQ_MULTI: ReqText = ReqText { multiline: true };

impl Codec for ReqText {
    type T = String;
    fn decode(&self, value: Option<&Value>, pointer: &str) -> Result<Self::T, TmslError> {
        Ok(value
            .map(|v| decode_lines(v, pointer))
            .transpose()?
            .unwrap_or_default())
    }
    fn encode(&self, value: &Self::T) -> Option<Value> {
        (!value.is_empty()).then(|| encode_lines(value, self.multiline))
    }
}

/// Model name: TOM default `Model` is omitted.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ModelName;

impl Codec for ModelName {
    type T = String;
    fn decode(&self, value: Option<&Value>, pointer: &str) -> Result<Self::T, TmslError> {
        Ok(OPT_SINGLE
            .decode(value, pointer)?
            .unwrap_or_else(|| "Model".to_owned()))
    }
    fn encode(&self, value: &Self::T) -> Option<Value> {
        (value != "Model").then(|| Value::String(value.clone()))
    }
}

/// Boolean with a TOM default that is omitted.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Flag {
    pub default: bool,
}

pub(crate) const FLAG: Flag = Flag { default: false };

impl Codec for Flag {
    type T = bool;
    fn decode(&self, value: Option<&Value>, pointer: &str) -> Result<Self::T, TmslError> {
        match value {
            None => Ok(self.default),
            Some(Value::Bool(b)) => Ok(*b),
            Some(other) => Err(TmslError::unexpected(pointer, "a boolean", other)),
        }
    }
    fn encode(&self, value: &Self::T) -> Option<Value> {
        (*value != self.default).then_some(Value::Bool(*value))
    }
}

/// Optional integer (`compatibilityLevel`, level `ordinal`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Int;

impl Codec for Int {
    type T = Option<i64>;
    fn decode(&self, value: Option<&Value>, pointer: &str) -> Result<Self::T, TmslError> {
        match value {
            None => Ok(None),
            Some(v) => v
                .as_i64()
                .map(Some)
                .ok_or_else(|| TmslError::unexpected(pointer, "an integer", v)),
        }
    }
    fn encode(&self, value: &Self::T) -> Option<Value> {
        value.map(Value::from)
    }
}

/// Integer with a TOM default that is omitted (calculation group
/// `precedence`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct IntDefault {
    pub default: i64,
}

impl Codec for IntDefault {
    type T = i64;
    fn decode(&self, value: Option<&Value>, pointer: &str) -> Result<Self::T, TmslError> {
        match value {
            None => Ok(self.default),
            Some(v) => v
                .as_i64()
                .ok_or_else(|| TmslError::unexpected(pointer, "an integer", v)),
        }
    }
    fn encode(&self, value: &Self::T) -> Option<Value> {
        (*value != self.default).then(|| Value::from(*value))
    }
}

/// `formatStringDefinition: {expression}` (calculation items and selection
/// expressions). Other shapes fail so the reader keeps them verbatim.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FormatStringDefinition;

impl Codec for FormatStringDefinition {
    type T = Option<String>;
    fn decode(&self, value: Option<&Value>, pointer: &str) -> Result<Self::T, TmslError> {
        let Some(value) = value else {
            return Ok(None);
        };
        let Value::Object(map) = value else {
            return Err(TmslError::unexpected(pointer, "an object", value));
        };
        match (map.len(), map.get("expression")) {
            (1, Some(expression)) => {
                decode_lines(expression, &format!("{pointer}/expression")).map(Some)
            }
            _ => Err(TmslError::UnexpectedType {
                pointer: pointer.to_owned(),
                expected: "an object with only `expression`",
                found: "another shape",
            }),
        }
    }
    fn encode(&self, value: &Self::T) -> Option<Value> {
        let text = value.as_ref()?;
        let mut map = Map::new();
        map.insert("expression".into(), encode_lines(text, true));
        Some(Value::Object(map))
    }
}

/// Model enums that parse leniently and print canonically.
pub(crate) trait StrEnum: Clone + PartialEq {
    fn parse_str(s: &str) -> Self;
    fn canonical(&self) -> &str;
}

macro_rules! str_enums {
    ($($t:ty),* $(,)?) => {
        $(impl StrEnum for $t {
            fn parse_str(s: &str) -> Self {
                <$t>::parse(s)
            }
            fn canonical(&self) -> &str {
                self.as_str()
            }
        })*
    };
}

str_enums!(
    DataType,
    SummarizeBy,
    PartitionMode,
    Cardinality,
    CrossFilter,
    SecurityFilteringBehavior,
    ExpressionKind,
);

fn decode_enum<E: StrEnum>(value: &Value, pointer: &str) -> Result<E, TmslError> {
    match value {
        Value::String(s) => Ok(E::parse_str(s)),
        other => Err(TmslError::unexpected(pointer, "a string", other)),
    }
}

/// Enum with a default. The default is omitted unless `always` is set.
#[derive(Debug, Clone)]
pub(crate) struct EnumCodec<E> {
    pub default: E,
    pub always: bool,
}

impl<E: StrEnum> EnumCodec<E> {
    pub fn new(default: E) -> Self {
        Self {
            default,
            always: false,
        }
    }

    pub fn always(default: E) -> Self {
        Self {
            default,
            always: true,
        }
    }
}

impl<E: StrEnum> Codec for EnumCodec<E> {
    type T = E;
    fn decode(&self, value: Option<&Value>, pointer: &str) -> Result<Self::T, TmslError> {
        value.map_or_else(|| Ok(self.default.clone()), |v| decode_enum(v, pointer))
    }
    fn encode(&self, value: &Self::T) -> Option<Value> {
        (self.always || *value != self.default).then(|| Value::String(value.canonical().into()))
    }
}

/// Optional enum (no default; absent is `None`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct OptEnum<E>(std::marker::PhantomData<E>);

impl<E> OptEnum<E> {
    pub fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<E: StrEnum> Codec for OptEnum<E> {
    type T = Option<E>;
    fn decode(&self, value: Option<&Value>, pointer: &str) -> Result<Self::T, TmslError> {
        value.map(|v| decode_enum(v, pointer)).transpose()
    }
    fn encode(&self, value: &Self::T) -> Option<Value> {
        value
            .as_ref()
            .map(|e| Value::String(e.canonical().to_owned()))
    }
}

/// `annotations: [{name, value}]`. Values may be strings or line arrays.
/// Entries with other keys or non-text values cannot be represented by
/// [`Annotation`]; decoding fails and the reader keeps the array verbatim.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Annotations;

impl Codec for Annotations {
    type T = Vec<Annotation>;
    fn decode(&self, value: Option<&Value>, pointer: &str) -> Result<Self::T, TmslError> {
        let items = match value {
            None => return Ok(Vec::new()),
            Some(Value::Array(items)) => items,
            Some(other) => return Err(TmslError::unexpected(pointer, "an array", other)),
        };
        let mut out = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let item_ptr = format!("{pointer}/{i}");
            let Value::Object(map) = item else {
                return Err(TmslError::unexpected(&item_ptr, "an object", item));
            };
            if let Some(key) = map.keys().find(|k| *k != "name" && *k != "value") {
                return Err(TmslError::UnexpectedType {
                    pointer: format!("{item_ptr}/{key}"),
                    expected: "only `name` and `value` in an annotation",
                    found: "another property",
                });
            }
            let name = match map.get("name") {
                Some(Value::String(s)) => s.clone(),
                Some(other) => {
                    return Err(TmslError::unexpected(
                        &format!("{item_ptr}/name"),
                        "a string",
                        other,
                    ));
                }
                None => {
                    return Err(TmslError::UnexpectedType {
                        pointer: item_ptr,
                        expected: "an annotation name",
                        found: "none",
                    });
                }
            };
            let value = map
                .get("value")
                .map(|v| decode_lines(v, &format!("{item_ptr}/value")))
                .transpose()?
                .unwrap_or_default();
            out.push(Annotation { name, value });
        }
        Ok(out)
    }
    fn encode(&self, value: &Self::T) -> Option<Value> {
        (!value.is_empty()).then(|| {
            Value::Array(
                value
                    .iter()
                    .map(|a| {
                        let mut map = Map::new();
                        map.insert("name".into(), Value::String(a.name.clone()));
                        map.insert("value".into(), encode_lines(&a.value, true));
                        Value::Object(map)
                    })
                    .collect(),
            )
        })
    }
}

/// Culture `linguisticMetadata: {content, contentType}`. The model keeps the
/// content as text: pretty JSON for `json`, verbatim for `xml`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LinguisticMetadata;

impl Codec for LinguisticMetadata {
    type T = Option<String>;
    fn decode(&self, value: Option<&Value>, pointer: &str) -> Result<Self::T, TmslError> {
        let Some(value) = value else {
            return Ok(None);
        };
        let Value::Object(map) = value else {
            return Err(TmslError::unexpected(pointer, "an object", value));
        };
        let shape_error = || TmslError::UnexpectedType {
            pointer: pointer.to_owned(),
            expected: "`content` with `contentType` json or xml",
            found: "another shape",
        };
        if map.len() != 2 {
            return Err(shape_error());
        }
        match (
            map.get("content"),
            map.get("contentType").and_then(Value::as_str),
        ) {
            (Some(content @ Value::Object(_)), Some("json")) => Ok(Some(
                serde_json::to_string_pretty(content).unwrap_or_default(),
            )),
            (Some(Value::String(content)), Some("xml")) => Ok(Some(content.clone())),
            _ => Err(shape_error()),
        }
    }
    fn encode(&self, value: &Self::T) -> Option<Value> {
        let text = value.as_ref()?;
        let mut map = Map::new();
        match serde_json::from_str::<Value>(text) {
            Ok(content @ Value::Object(_)) => {
                map.insert("content".into(), content);
                map.insert("contentType".into(), Value::String("json".into()));
            }
            _ => {
                map.insert("content".into(), Value::String(text.clone()));
                map.insert("contentType".into(), Value::String("xml".into()));
            }
        }
        Some(Value::Object(map))
    }
}

/// TOM column `type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColumnType {
    Data,
    Calculated,
    CalculatedTableColumn,
    RowNumber,
}

impl ColumnType {
    fn as_str(self) -> &'static str {
        match self {
            ColumnType::Data => "data",
            ColumnType::Calculated => "calculated",
            ColumnType::CalculatedTableColumn => "calculatedTableColumn",
            ColumnType::RowNumber => "rowNumber",
        }
    }
}

/// Column `type`; unknown values fail so the reader keeps them verbatim.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ColumnTypeCodec;

impl Codec for ColumnTypeCodec {
    type T = ColumnType;
    fn decode(&self, value: Option<&Value>, pointer: &str) -> Result<Self::T, TmslError> {
        let Some(value) = value else {
            return Ok(ColumnType::Data);
        };
        let Value::String(s) = value else {
            return Err(TmslError::unexpected(pointer, "a string", value));
        };
        [
            ColumnType::Data,
            ColumnType::Calculated,
            ColumnType::CalculatedTableColumn,
            ColumnType::RowNumber,
        ]
        .into_iter()
        .find(|t| t.as_str().eq_ignore_ascii_case(s))
        .ok_or_else(|| TmslError::UnexpectedType {
            pointer: pointer.to_owned(),
            expected: "a known column type",
            found: "an unknown column type",
        })
    }
    fn encode(&self, value: &Self::T) -> Option<Value> {
        (*value != ColumnType::Data).then(|| Value::String(value.as_str().to_owned()))
    }
}

/// Partition source `type`. Absent means `query` (legacy TMSL).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SourceType {
    M,
    Calculated,
    Query,
    Entity,
    Other(String),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SourceTypeCodec;

impl Codec for SourceTypeCodec {
    type T = SourceType;
    fn decode(&self, value: Option<&Value>, pointer: &str) -> Result<Self::T, TmslError> {
        let Some(value) = value else {
            return Ok(SourceType::Query);
        };
        let Value::String(s) = value else {
            return Err(TmslError::unexpected(pointer, "a string", value));
        };
        Ok(match s.to_ascii_lowercase().as_str() {
            "m" => SourceType::M,
            "calculated" => SourceType::Calculated,
            "query" => SourceType::Query,
            "entity" => SourceType::Entity,
            _ => SourceType::Other(s.clone()),
        })
    }
    fn encode(&self, value: &Self::T) -> Option<Value> {
        let s = match value {
            SourceType::M => "m",
            SourceType::Calculated => "calculated",
            SourceType::Query => return None,
            SourceType::Entity => "entity",
            SourceType::Other(s) => s,
        };
        Some(Value::String(s.to_owned()))
    }
}

/// Named expression text, split into `(expression, meta)` for M.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NamedExpressionText {
    pub split_meta: bool,
}

impl Codec for NamedExpressionText {
    type T = (String, Option<String>);
    fn decode(&self, value: Option<&Value>, pointer: &str) -> Result<Self::T, TmslError> {
        let text = REQ_MULTI.decode(value, pointer)?;
        if !self.split_meta {
            return Ok((text, None));
        }
        let (expression, meta) = split_m_meta(&text);
        Ok((expression.to_owned(), meta.map(str::to_owned)))
    }
    fn encode(&self, (expression, meta): &Self::T) -> Option<Value> {
        let full = match meta {
            Some(meta) => format!("{expression} meta {meta}"),
            None => expression.clone(),
        };
        REQ_MULTI.encode(&full)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn round<C: Codec>(codec: &C, value: &Value) -> Option<Value> {
        codec.encode(&codec.decode(Some(value), "").unwrap())
    }

    #[test]
    fn text_forms_follow_split_multiline_strings() {
        assert_eq!(
            round(&OPT_MULTI, &json!("one line")),
            Some(json!("one line"))
        );
        assert_eq!(
            round(&OPT_MULTI, &json!(["a", "", "b"])),
            Some(json!(["a", "", "b"]))
        );
        // Deviations from the default form are detected by the reader.
        assert_eq!(round(&OPT_MULTI, &json!("a\nb")), Some(json!(["a", "b"])));
        assert_eq!(round(&OPT_MULTI, &json!(["single"])), Some(json!("single")));
        assert_eq!(round(&OPT_SINGLE, &json!("a\nb")), Some(json!("a\nb")));
        assert!(OPT_MULTI.decode(Some(&json!(["a", 1])), "/x").is_err());
        assert!(OPT_MULTI.decode(Some(&json!(true)), "/x").is_err());
    }

    #[test]
    fn defaults_are_omitted() {
        assert_eq!(FLAG.encode(&false), None);
        assert_eq!(Flag { default: true }.encode(&true), None);
        assert_eq!(ModelName.encode(&"Model".to_owned()), None);
        assert_eq!(REQ_SINGLE.encode(&String::new()), None);
        let cf = EnumCodec::new(CrossFilter::OneDirection);
        assert_eq!(cf.encode(&CrossFilter::OneDirection), None);
        assert_eq!(
            cf.encode(&cf.decode(Some(&json!("BothDirections")), "").unwrap()),
            Some(json!("bothDirections"))
        );
        let kind = EnumCodec::always(ExpressionKind::M);
        assert_eq!(kind.encode(&ExpressionKind::M), Some(json!("m")));
    }

    #[test]
    fn annotations_and_linguistic_metadata() {
        let anns = json!([{"name": "A", "value": ["x", "y"]}, {"name": "B", "value": "1"}]);
        let decoded = Annotations.decode(Some(&anns), "").unwrap();
        assert_eq!(decoded[0].value, "x\ny");
        assert_eq!(Annotations.encode(&decoded), Some(anns));
        assert!(
            Annotations
                .decode(Some(&json!([{"name": "A", "extra": 1}])), "")
                .is_err()
        );
        assert!(
            Annotations
                .decode(Some(&json!([{"value": "v"}])), "")
                .is_err()
        );

        let lm = json!({"content": {"Version": "1.0.0", "n": 1.50}, "contentType": "json"});
        assert_eq!(round(&LinguisticMetadata, &lm), Some(lm));
        let xml = json!({"content": "<Linguistic/>", "contentType": "xml"});
        assert_eq!(round(&LinguisticMetadata, &xml), Some(xml));
        assert!(
            LinguisticMetadata
                .decode(Some(&json!({"content": 1})), "")
                .is_err()
        );
    }

    #[test]
    fn named_expression_meta_is_split_and_rejoined() {
        let codec = NamedExpressionText { split_meta: true };
        let text = json!("\"x\" meta [IsParameterQuery=true]");
        let (expr, meta) = codec.decode(Some(&text), "").unwrap();
        assert_eq!(expr, "\"x\"");
        assert_eq!(meta.as_deref(), Some("[IsParameterQuery=true]"));
        assert_eq!(codec.encode(&(expr, meta)), Some(text));
        let odd = json!("\"x\"  meta [A=1]\n");
        assert_eq!(round(&codec, &odd), Some(json!("\"x\" meta [A=1]")));
    }
}
