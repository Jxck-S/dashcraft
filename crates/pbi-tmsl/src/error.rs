//! Error type of the TMSL reader.

use serde_json::Value;
use thiserror::Error;

/// Why a TMSL document could not be read. Writers never fail.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TmslError {
    /// Malformed JSON, trailing characters, or nesting deeper than the
    /// parser's recursion limit (128 levels).
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),

    /// The bytes are not valid text in the detected encoding (invalid UTF-8,
    /// odd-length UTF-16, unpaired surrogates). Text is never decoded lossily.
    #[error("invalid {encoding} text: {message}")]
    Encoding {
        encoding: &'static str,
        message: String,
    },

    /// A modelled property has the wrong JSON type, e.g. `"isHidden": "yes"`
    /// or `"tables": {}`. `pointer` is a JSON pointer (RFC 6901).
    #[error("expected {expected} at `{pointer}`, found {found}")]
    UnexpectedType {
        pointer: String,
        expected: &'static str,
        found: &'static str,
    },

    /// The root object has no `model` (e.g. a TMSL command script such as
    /// `createOrReplace`, which is not a database definition).
    #[error("the TMSL root object has no `model` object")]
    MissingModel,
}

impl TmslError {
    pub(crate) fn unexpected(pointer: &str, expected: &'static str, found: &Value) -> Self {
        TmslError::UnexpectedType {
            pointer: pointer.to_owned(),
            expected,
            found: json_kind(found),
        }
    }
}

/// Human-readable name of a JSON value's type.
pub(crate) fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}
