//! Engine error type.

use datafusion::error::DataFusionError;

/// Errors returned by the storage engine. Bad input (missing tables, type
/// mismatches, out-of-range rows, cancelled queries) is always reported here;
/// the engine never panics on it.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("table '{0}' not found")]
    TableNotFound(String),

    #[error("column '{table}'[{column}] not found")]
    ColumnNotFound { table: String, column: String },

    #[error("duplicate column '{column}' in table '{table}' (names are case-insensitive)")]
    DuplicateColumn { table: String, column: String },

    #[error("column '{table}'[{column}] has {actual} rows, expected {expected}")]
    RowCountMismatch {
        table: String,
        column: String,
        expected: usize,
        actual: usize,
    },

    #[error("row {row} is out of range for table '{table}' ({rows} rows)")]
    RowOutOfRange {
        table: String,
        row: usize,
        rows: usize,
    },

    #[error("table '{table}' has too many rows ({rows}); the engine supports up to {max}")]
    TooManyRows {
        table: String,
        rows: usize,
        max: usize,
    },

    #[error("unsupported Arrow type {data_type} ({context})")]
    UnsupportedType { data_type: String, context: String },

    #[error("cannot compare a value of type {left} with a value of type {right}")]
    IncomparableTypes {
        left: &'static str,
        right: &'static str,
    },

    #[error("cannot convert {value} to {target}")]
    Conversion { value: String, target: String },

    #[error("{agg} is not supported for column '{table}'[{column}] of type {data_type}")]
    UnsupportedAggregation {
        agg: String,
        table: String,
        column: String,
        data_type: String,
    },

    #[error("numeric overflow in {0}")]
    Overflow(String),

    #[error("no active many-to-one relationship path from '{from}' to '{to}'")]
    NoRelationshipPath { from: String, to: String },

    #[error("lookup in '{table}'[{column}] found multiple different values")]
    AmbiguousLookup { table: String, column: String },

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("query cancelled")]
    Cancelled,

    #[error("Arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),

    /// Boxed to keep `Result<T, EngineError>` small.
    #[error("DataFusion error: {0}")]
    DataFusion(Box<DataFusionError>),
}

impl From<DataFusionError> for EngineError {
    fn from(err: DataFusionError) -> Self {
        EngineError::DataFusion(Box::new(err))
    }
}

/// Engine result type.
pub type Result<T, E = EngineError> = std::result::Result<T, E>;
