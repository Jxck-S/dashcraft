use thiserror::Error;

use crate::abf::AbfError;
use crate::metadata::MetadataError;

/// Errors produced while decoding VertiPaq column data (`.idfmeta`, `.idf`,
/// `.dictionary`) into Arrow arrays.
///
/// Every malformed input yields one of these; the decoder never panics. Messages name
/// structures, offsets and counts, never data values.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum VertipaqError {
    /// The backup could not be read.
    #[error(transparent)]
    Abf(#[from] AbfError),
    /// The metadata could not be read.
    #[error(transparent)]
    Metadata(#[from] MetadataError),
    /// No table with this name exists in the storage map (or model).
    #[error("table {0:?} not found")]
    TableNotFound(String),
    /// No column with this name exists in the table.
    #[error("column {table:?}[{column:?}] not found")]
    ColumnNotFound {
        /// Table name.
        table: String,
        /// Column name.
        column: String,
    },
    /// A structure ends before its declared size.
    #[error("{what}: truncated at offset {offset} (need {needed} bytes, {available} available)")]
    Truncated {
        /// The structure being read.
        what: &'static str,
        /// Offset where the read started.
        offset: u64,
        /// Bytes needed.
        needed: u64,
        /// Bytes available from `offset`.
        available: u64,
    },
    /// A structure is internally inconsistent.
    #[error("{what}: {message}")]
    Corrupt {
        /// The structure.
        what: &'static str,
        /// What is wrong (counts and offsets only).
        message: String,
    },
    /// The input uses an encoding this decoder does not implement.
    #[error("unsupported {what}: {detail}")]
    Unsupported {
        /// The feature.
        what: &'static str,
        /// Details (class ids, codes).
        detail: String,
    },
    /// A resource limit from [`DecodeLimits`](super::DecodeLimits) was exceeded.
    #[error("limit exceeded: {what} is {value}, the limit is {limit}")]
    LimitExceeded {
        /// The limited quantity.
        what: &'static str,
        /// The value found or required.
        value: u64,
        /// The limit.
        limit: u64,
    },
    /// Data ids that map to no dictionary entry (or values that cannot be converted to
    /// the column type) while [`InvalidIdPolicy::Error`](super::InvalidIdPolicy) is in
    /// effect.
    #[error("{count} row(s) reference invalid data ids or unconvertible values (first: {first})")]
    InvalidDataIds {
        /// Number of affected rows.
        count: u64,
        /// Description of the first offending id.
        first: String,
    },
    /// The decoded column length differs from the table's row count.
    #[error("column has {actual} rows, the table has {expected}")]
    RowCountMismatch {
        /// Rows of the table (`SegmentMapStorage.RecordCount` summed over partitions).
        expected: u64,
        /// Rows decoded for the column.
        actual: u64,
    },
    /// Error while decoding one column (context wrapper).
    #[error("{table:?}[{column:?}]: {source}")]
    InColumn {
        /// Table name.
        table: String,
        /// Column name.
        column: String,
        /// The underlying error.
        #[source]
        source: Box<VertipaqError>,
    },
    /// Arrow rejected an array or record batch.
    #[error("arrow: {0}")]
    Arrow(#[from] arrow::error::ArrowError),
}

impl VertipaqError {
    /// The error without [`VertipaqError::InColumn`] context.
    pub fn innermost(&self) -> &VertipaqError {
        match self {
            VertipaqError::InColumn { source, .. } => source.innermost(),
            other => other,
        }
    }

    pub(crate) fn corrupt(what: &'static str, message: impl Into<String>) -> Self {
        VertipaqError::Corrupt {
            what,
            message: message.into(),
        }
    }

    pub(crate) fn unsupported(what: &'static str, detail: impl Into<String>) -> Self {
        VertipaqError::Unsupported {
            what,
            detail: detail.into(),
        }
    }

    pub(crate) fn in_column(self, table: &str, column: &str) -> Self {
        match self {
            already @ VertipaqError::InColumn { .. } => already,
            other => VertipaqError::InColumn {
                table: table.to_owned(),
                column: column.to_owned(),
                source: Box::new(other),
            },
        }
    }
}
