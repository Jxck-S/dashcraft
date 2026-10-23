//! Error types. Syntax errors carry the file path and a 1-based line/column.

use thiserror::Error;

/// What is wrong with a TMDL line.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SyntaxError {
    /// A line is indented deeper than its position allows (e.g. a property
    /// two levels below its object, or an indented first line).
    #[error("unexpected indentation: expected level {expected} or less, found level {found}")]
    UnexpectedIndentation { expected: usize, found: usize },
    /// Space indentation that is not a whole number of indentation units.
    #[error("indentation of {spaces} space(s) is not a whole number of {unit}-space units")]
    PartialIndentation { spaces: usize, unit: usize },
    /// The line does not start with an object type or property name.
    #[error("expected an object type or property name")]
    ExpectedKeyword,
    /// A `'quoted name` without its closing quote.
    #[error("unterminated quoted name (missing closing `'`)")]
    UnterminatedName,
    /// Text after an object name that is neither `=` nor the end of the line.
    #[error(
        "unexpected text after the object name (names containing spaces, `.`, `=`, `:` or `'` must be quoted)"
    )]
    UnexpectedText,
    /// A ```` ``` ```` expression without its closing delimiter line.
    #[error("unterminated ``` expression (missing closing ``` line)")]
    UnterminatedFence,
    /// Indentation nesting beyond the parser limit.
    #[error("nesting deeper than {limit} levels")]
    TooDeep { limit: usize },
}

/// Errors returned by the TMDL reader.
#[derive(Debug, Error)]
pub enum TmdlError {
    /// Malformed TMDL.
    #[error("{path}:{line}:{column}: {kind}")]
    Syntax {
        path: String,
        /// 1-based line number.
        line: u32,
        /// 1-based column (in characters).
        column: u32,
        kind: SyntaxError,
    },
    /// File system error (only from [`crate::load_model_from_dir`]).
    #[error("{path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    /// The file is not UTF-8.
    #[error("{path}: not valid UTF-8 text")]
    Encoding { path: String },
    /// A size or count limit was exceeded.
    #[error("{path}: {what} exceeds the limit of {limit}")]
    Limit {
        path: String,
        what: &'static str,
        limit: u64,
    },
}

impl TmdlError {
    /// The syntax error kind, if this is a syntax error.
    pub fn syntax_kind(&self) -> Option<&SyntaxError> {
        match self {
            TmdlError::Syntax { kind, .. } => Some(kind),
            _ => None,
        }
    }
}
