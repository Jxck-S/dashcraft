//! Errors of the session API.

use std::path::PathBuf;

use pbi_dax::DaxError;
use pbi_engine::EngineError;

/// Error returned by [`Session`](crate::Session) operations.
///
/// Per-table refresh failures and per-measure evaluation errors are *not*
/// errors of the call: they are reported in
/// [`RefreshReport`](crate::RefreshReport) and
/// [`MeasureValue`](crate::MeasureValue) entries.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SessionError {
    /// The input could not be opened (I/O, unsafe archive, unsupported).
    #[error(transparent)]
    Open(#[from] pbi_core::CoreError),
    /// The blob map (`blob-map.json`) could not be read.
    #[error("cannot read the blob map {path:?}: {message}")]
    BlobMap {
        /// The blob map path.
        path: PathBuf,
        /// What went wrong.
        message: String,
    },
    /// A table name that is not in the model.
    #[error("table '{0}' is not in the model")]
    UnknownTable(String),
    /// A column name that is not in the model.
    #[error("column '{table}'[{column}] is not in the model")]
    UnknownColumn {
        /// Table name.
        table: String,
        /// Column name.
        column: String,
    },
    /// A measure name that is not in the model.
    #[error("measure [{0}] is not in the model")]
    UnknownMeasure(String),
    /// A [`FilterSpec`](crate::FilterSpec) that cannot be applied.
    #[error("invalid filter: {0}")]
    InvalidFilter(String),
    /// A manual CSV load was refused or the CSV could not be read.
    #[error("cannot load '{table}' from CSV: {reason}")]
    Csv {
        /// Target table.
        table: String,
        /// Why.
        reason: String,
    },
    /// A derived column could not be added or removed (name collision, a
    /// source column that is missing or not text, a DAX error, an array of
    /// the wrong length, not a derived column).
    #[error("derived column '{table}'[{column}]: {reason}")]
    Derived {
        /// Table name.
        table: String,
        /// Derived column name.
        column: String,
        /// Why.
        reason: String,
    },
    /// The document has no local data cache (PBIP `.pbi/cache.abf`).
    #[error(
        "the document has no local data cache (.pbi/cache.abf); refresh from the source instead"
    )]
    NoCache,
    /// The local data cache exists but cannot be read.
    #[error("cannot read the local data cache {entry}: {reason}")]
    Cache {
        /// The cache entry, e.g. `X.SemanticModel/.pbi/cache.abf`.
        entry: String,
        /// Why.
        reason: String,
    },
    /// A file could not be read.
    #[error("I/O error on {path:?}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// Storage engine / DataFusion failure (grid, SQL, stats).
    #[error(transparent)]
    Engine(EngineError),
    /// DAX failure outside per-measure results (e.g. a DAX query).
    #[error(transparent)]
    Dax(DaxError),
    /// A role name that is not in the model ("View as").
    #[error("role '{0}' is not in the model")]
    UnknownRole(String),
    /// A role's row filter expression could not be evaluated ("View as").
    #[error("role '{role}': the filter on '{table}' cannot be evaluated: {message}")]
    RoleFilter {
        /// Role name.
        role: String,
        /// Filtered table.
        table: String,
        /// Why.
        message: String,
    },
    /// The operation was cancelled through its `CancellationToken`.
    #[error("the operation was cancelled")]
    Cancelled,
    /// A background task failed unexpectedly.
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<EngineError> for SessionError {
    fn from(e: EngineError) -> Self {
        match e {
            EngineError::Cancelled => SessionError::Cancelled,
            other => SessionError::Engine(other),
        }
    }
}

impl From<DaxError> for SessionError {
    fn from(e: DaxError) -> Self {
        match e {
            DaxError::Cancelled => SessionError::Cancelled,
            other => SessionError::Dax(other),
        }
    }
}

impl From<tokio::task::JoinError> for SessionError {
    fn from(e: tokio::task::JoinError) -> Self {
        if e.is_cancelled() {
            SessionError::Cancelled
        } else {
            SessionError::Internal(format!("background task failed: {e}"))
        }
    }
}
