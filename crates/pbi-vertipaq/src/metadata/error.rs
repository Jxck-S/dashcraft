use thiserror::Error;

use crate::abf::AbfError;

/// Errors produced while reading the tabular metadata (`metadata.sqlitedb`).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum MetadataError {
    /// The backup could not be read.
    #[error(transparent)]
    Abf(#[from] AbfError),
    /// The backup contains no `metadata.sqlitedb` (e.g. a multidimensional or pre-1200
    /// model, whose metadata is XML).
    #[error("the backup contains no metadata.sqlitedb")]
    MissingMetadata,
    /// The bytes are not an SQLite database.
    #[error("metadata.sqlitedb is not an SQLite database")]
    NotSqlite,
    /// The database is larger than [`MetadataDb::MAX_SIZE`](super::MetadataDb::MAX_SIZE).
    #[error("metadata.sqlitedb is too large ({size} bytes, limit {limit})")]
    TooLarge {
        /// Size of the database.
        size: u64,
        /// Limit.
        limit: u64,
    },
    /// A table the reader needs is missing (not a tabular metadata database).
    #[error("metadata.sqlitedb has no {0} table (not a tabular model database)")]
    MissingTable(&'static str),
    /// A metadata table has more cells than [`MAX_CELLS`](super::MAX_CELLS).
    #[error("metadata table {table} is too large ({cells} cells)")]
    TooManyRows {
        /// The metadata table.
        table: &'static str,
        /// Number of cells read before giving up.
        cells: u64,
    },
    /// SQLite reported an error (corrupt database, I/O, ...).
    #[error("SQLite error reading metadata.sqlitedb: {0}")]
    Sqlite(#[from] rusqlite::Error),
}
