//! Evaluation errors.
//!
//! [`MError`] is the error type of the evaluator. Only [`MError::Evaluation`] is an M error
//! *value* in the language sense: it can be caught by `try`, stored in table cells, list items
//! and record fields, and inspected as an error record (`Reason`, `Message`, `Detail`). All other
//! variants describe conditions outside the program (unsupported features, missing blob
//! mappings, resource limits, cancellation) and abort the whole evaluation; `try` does not catch
//! them, so a fallback branch can never silently hide that a query could not be evaluated
//! faithfully.

use std::fmt;
use std::sync::Arc;

use crate::span::Span;
use crate::value::Value;

/// Where an error was raised: a byte span in the source of the main query (`query: None`) or
/// of a shared query / parameter (`query: Some(name)`), with its 1-based line and column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub query: Option<Arc<str>>,
    pub span: Span,
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.query {
            Some(q) => write!(f, "query `{q}`, line {}, column {}", self.line, self.column),
            None => write!(f, "line {}, column {}", self.line, self.column),
        }
    }
}

/// Which resource limit was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitKind {
    /// Rows in a table or items in a list ([`crate::Limits::max_rows`]).
    Rows,
    /// Evaluation steps ([`crate::Limits::max_steps`]).
    Steps,
    /// Nesting / recursion depth ([`crate::Limits::max_depth`]).
    Depth,
    /// Size of a binary or text value ([`crate::Limits::max_binary_bytes`]).
    Bytes,
}

impl LimitKind {
    pub const fn describe(self) -> &'static str {
        match self {
            LimitKind::Rows => "too many rows or list items",
            LimitKind::Steps => "too many evaluation steps",
            LimitKind::Depth => "recursion too deep",
            LimitKind::Bytes => "value too large",
        }
    }
}

/// Error raised by a [`crate::BlobResolver`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolverError {
    pub kind: ResolverErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolverErrorKind {
    /// No resolver/mapping is configured (offline mode).
    Offline,
    /// The container or blob is unknown to the resolver.
    NotFound,
    /// Reading the mapped data failed.
    Io,
    /// The mapping is invalid or rejected (e.g. path traversal).
    Invalid,
    /// Access was denied (credentials missing or rejected).
    Denied,
}

impl ResolverError {
    pub fn new(kind: ResolverErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for ResolverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ResolverError {}

/// An error produced while parsing or evaluating M.
#[derive(Debug, Clone, thiserror::Error)]
pub enum MError {
    /// The query (or a shared query it references) does not parse.
    #[error("{}", syntax_message(query.as_deref(), *line, *column, message))]
    Syntax {
        /// `None` for the main query, else the name of the shared query.
        query: Option<String>,
        message: String,
        span: Span,
        line: usize,
        column: usize,
    },
    /// An M error value (`error ...`, a failed conversion, a missing field, ...). Catchable with
    /// `try`.
    #[error("{}", evaluation_message(reason, message, location.as_deref()))]
    Evaluation {
        /// e.g. `Expression.Error`, `DataFormat.Error`.
        reason: String,
        message: String,
        /// The error record's `Detail` value, if any.
        detail: Option<Box<Value>>,
        location: Option<Box<Location>>,
    },
    /// The query uses a function or feature DashCraft does not evaluate.
    #[error("{}", unsupported_message(function, location.as_deref(), hint))]
    Unsupported {
        /// The M identifier, e.g. `Web.Contents`.
        function: String,
        location: Option<Box<Location>>,
        /// What the user can do instead (shown by the UI).
        hint: String,
    },
    /// The blob resolver failed (no mapping, missing file, ...).
    #[error("blob storage: {0}")]
    Resolver(#[from] ResolverError),
    /// A configured [`crate::Limits`] value was exceeded.
    #[error("evaluation stopped: {} (limit {limit})", kind.describe())]
    LimitExceeded { kind: LimitKind, limit: u64 },
    /// The cancellation flag was set.
    #[error("evaluation cancelled")]
    Cancelled,
    /// A bug or environment failure inside the evaluator (never caused by M semantics).
    #[error("internal evaluator error: {0}")]
    Internal(String),
}

fn syntax_message(query: Option<&str>, line: usize, column: usize, message: &str) -> String {
    match query {
        Some(q) => {
            format!("syntax error in query `{q}` at line {line}, column {column}: {message}")
        }
        None => format!("syntax error at line {line}, column {column}: {message}"),
    }
}

fn evaluation_message(reason: &str, message: &str, location: Option<&Location>) -> String {
    match location {
        Some(l) => format!("{reason}: {message} ({l})"),
        None => format!("{reason}: {message}"),
    }
}

fn unsupported_message(function: &str, location: Option<&Location>, hint: &str) -> String {
    let at = location.map(|l| format!(" ({l})")).unwrap_or_default();
    if hint.is_empty() {
        format!("unsupported M function `{function}`{at}")
    } else {
        format!("unsupported M function `{function}`{at}. {hint}")
    }
}

/// Default hint for unsupported features.
pub const LOAD_MANUALLY_HINT: &str = "DashCraft evaluates only a subset of M offline. Use \"Load CSV manually…\" to provide this table's data.";

/// Hint for data-access functions that would need files, databases or the network.
pub const DATA_ACCESS_HINT: &str = "DashCraft never accesses files, databases or the network from M. Map Azure blobs in blob-map.json, or use \"Load CSV manually…\" to provide this table's data.";

impl MError {
    /// An `Expression.Error` with the given message.
    pub fn expression(message: impl Into<String>) -> Self {
        Self::eval("Expression.Error", message)
    }

    /// A `DataFormat.Error` (failed parse/conversion of data).
    pub fn data_format(message: impl Into<String>) -> Self {
        Self::eval("DataFormat.Error", message)
    }

    /// An evaluation error with an explicit reason.
    pub fn eval(reason: impl Into<String>, message: impl Into<String>) -> Self {
        MError::Evaluation {
            reason: reason.into(),
            message: message.into(),
            detail: None,
            location: None,
        }
    }

    pub fn unsupported(function: impl Into<String>, hint: impl Into<String>) -> Self {
        MError::Unsupported {
            function: function.into(),
            location: None,
            hint: hint.into(),
        }
    }

    pub fn limit(kind: LimitKind, limit: u64) -> Self {
        MError::LimitExceeded { kind, limit }
    }

    /// Whether `try` catches this error (only M error values are catchable).
    pub fn is_catchable(&self) -> bool {
        matches!(self, MError::Evaluation { .. })
    }

    /// The source location, for variants that carry one.
    pub fn location(&self) -> Option<&Location> {
        match self {
            MError::Evaluation { location, .. } | MError::Unsupported { location, .. } => {
                location.as_deref()
            }
            _ => None,
        }
    }

    /// Attaches `loc` unless the error already has a location.
    #[must_use]
    pub fn or_location(mut self, loc: impl FnOnce() -> Location) -> Self {
        if let MError::Evaluation { location, .. } | MError::Unsupported { location, .. } =
            &mut self
            && location.is_none()
        {
            *location = Some(Box::new(loc()));
        }
        self
    }

    /// Prefixes the message of an evaluation error (e.g. with a row/column context).
    #[must_use]
    pub fn with_context(mut self, context: &str) -> Self {
        if let MError::Evaluation { message, .. } = &mut self {
            *message = format!("{context}: {message}");
        }
        self
    }
}
