//! Exports: table data to CSV / Parquet, and the semantic model to a JSON
//! "model doc" or a Markdown data dictionary.
//!
//! # Table data
//!
//! [`write_csv`] and [`write_parquet`] write one Arrow [`RecordBatch`]
//! (typically [`Session::table_batch`]). Model metadata travels in the
//! batch's schema: [`annotate_batch`] attaches each column's model data type,
//! kind, description, format string and source column as Arrow field
//! metadata, and the table name, description and model culture as schema
//! metadata (keys in [`meta`]). [`Session::export_batch`] returns an
//! annotated table batch; [`Session::export_csv`] /
//! [`Session::export_parquet`] do both steps.
//!
//! - CSV ([`CsvExportOptions`]): RFC 4180 quoting (fields with the
//!   delimiter, `"` or line breaks are quoted, `"` doubled), BLANK is an
//!   empty field, `\n` line ends, optional UTF-8 BOM for Excel. Raw values
//!   by default (`1.2500`, `0.5`, `2026-10-21T14:05:00`, `true`); with
//!   `formatted` every value is formatted with its column's model
//!   `formatString` as Power BI shows it ([`pbi_dax::format_value`]). The
//!   opt-in `escape_formulas` guard prefixes text cells that start with
//!   `=`, `+`, `-`, `@`, TAB or CR with `'` (spreadsheet formula
//!   injection).
//! - Parquet ([`ParquetExportOptions`]): Zstd (default), Snappy or no
//!   compression. The Arrow schema (with the field metadata) is embedded as
//!   usual, and the file's key-value metadata holds
//!   `dashcraft.schemaVersion`, `dashcraft.table`, `dashcraft.tableDescription`,
//!   `dashcraft.culture` and `dashcraft.columns` (a JSON array of
//!   `{ name, dataType?, kind?, description?, formatString?, sourceColumn?, derived? }`).
//!
//! # Model documentation
//!
//! [`model_doc`] builds a [`ModelDoc`], a documented, stable JSON schema
//! (`schemaVersion: 1`) of the model: tables, columns, measures, partitions
//! (M / DAX), hierarchies, relationships, parameters, shared expressions and
//! roles, with the TMDL `///` descriptions. [`model_to_json`] is its JSON
//! value with every expression and hidden object included.
//! [`model_to_markdown`] renders the same content as a readable,
//! deterministic Markdown data dictionary. Session derived columns
//! (annotation [`DERIVED_ANNOTATION`](crate::DERIVED_ANNOTATION)) are never
//! part of a model export, even when given [`Session::model`].
//!
//! Table data exports do include derived columns (they are table data);
//! their fields carry [`meta::DERIVED`]` = true`.

mod csv_writer;
mod markdown;
mod model_doc;
mod parquet_writer;

use std::io::Write;

use arrow::datatypes::{Field, Schema};
use arrow::record_batch::RecordBatch;
use pbi_model::{ColumnKind, Table, names_eq};

use crate::error::SessionError;
use crate::session::Session;

pub use self::csv_writer::{CsvExportOptions, write_csv};
pub use self::markdown::{MarkdownOptions, model_to_markdown};
pub use self::model_doc::{
    ColumnDoc, ColumnRefDoc, ExpressionDoc, HierarchyDoc, LevelDoc, MODEL_DOC_SCHEMA_VERSION,
    MeasureDoc, ModelDoc, ModelDocOptions, ModelDocSummary, ParameterDoc, PartitionDoc,
    RelationshipDoc, RoleDoc, TableDoc, TablePermissionDoc, model_doc, model_to_json,
};
pub use self::parquet_writer::{ParquetCompression, ParquetExportOptions, write_parquet};

/// Metadata keys written by [`annotate_batch`] (Arrow field and schema
/// metadata) and [`write_parquet`] (Parquet key-value metadata).
pub mod meta {
    /// File: version of the DashCraft export metadata (`1`).
    pub const SCHEMA_VERSION: &str = "dashcraft.schemaVersion";
    /// Schema / file: model table name.
    pub const TABLE: &str = "dashcraft.table";
    /// Schema / file: table description (TMDL `///`).
    pub const TABLE_DESCRIPTION: &str = "dashcraft.tableDescription";
    /// Schema / file: model culture used for formatting (`en-US`).
    pub const CULTURE: &str = "dashcraft.culture";
    /// File: JSON array describing every column.
    pub const COLUMNS: &str = "dashcraft.columns";
    /// Field: model data type (`string`, `int64`, `decimal`, `dateTime`, …).
    pub const DATA_TYPE: &str = "dashcraft.dataType";
    /// Field: `data`, `calculated` or `calculatedTableColumn`.
    pub const KIND: &str = "dashcraft.kind";
    /// Field: column description (TMDL `///`).
    pub const DESCRIPTION: &str = "dashcraft.description";
    /// Field: model format string.
    pub const FORMAT_STRING: &str = "dashcraft.formatString";
    /// Field: source column of a data column.
    pub const SOURCE_COLUMN: &str = "dashcraft.sourceColumn";
    /// Field: `true` for a session derived column (not part of the model
    /// files; see [`crate::derived`]).
    pub const DERIVED: &str = "dashcraft.derived";
}

/// Version of the export metadata (`dashcraft.schemaVersion`).
pub const EXPORT_METADATA_VERSION: u32 = 1;

/// Error of an export.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ExportError {
    /// The table could not be read from the session.
    #[error(transparent)]
    Session(#[from] SessionError),
    /// Writing the output failed.
    #[error("cannot write the export: {0}")]
    Io(#[from] std::io::Error),
    /// An option is invalid (e.g. a non-ASCII CSV delimiter).
    #[error("invalid export option: {0}")]
    InvalidOption(String),
    /// A column could not be converted.
    #[error("column '{column}' cannot be exported: {reason}")]
    Column {
        /// Column name.
        column: String,
        /// Why.
        reason: String,
    },
    /// The CSV writer failed.
    #[error("CSV error: {0}")]
    Csv(String),
    /// The Parquet writer failed.
    #[error("Parquet error: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),
    /// An Arrow operation failed.
    #[error("Arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),
}

impl From<csv::Error> for ExportError {
    fn from(e: csv::Error) -> Self {
        if !e.is_io_error() {
            return ExportError::Csv(e.to_string());
        }
        match e.into_kind() {
            csv::ErrorKind::Io(io) => ExportError::Io(io),
            other => ExportError::Csv(format!("{other:?}")),
        }
    }
}

impl From<ExportError> for std::io::Error {
    /// I/O errors are returned as they are (so a closed pipe stays a
    /// `BrokenPipe`); everything else becomes [`std::io::ErrorKind::Other`].
    fn from(e: ExportError) -> Self {
        match e {
            ExportError::Io(io) => io,
            other => std::io::Error::other(other),
        }
    }
}

/// What an export wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    /// Data rows (without the CSV header).
    pub rows: usize,
    /// Columns.
    pub columns: usize,
}

/// Returns `batch` with model metadata attached (see [`meta`]): each field
/// matching a column of `table` (case-insensitive) gets the column's data
/// type, kind, description, format string and source column; the schema gets
/// the table name and description and `culture`. Data is shared, not copied.
///
/// # Errors
/// [`ExportError::Arrow`] if the batch cannot be rebuilt (not expected).
pub fn annotate_batch(
    batch: &RecordBatch,
    table: &Table,
    culture: &str,
) -> Result<RecordBatch, ExportError> {
    let schema = batch.schema();
    let fields: Vec<Field> = schema
        .fields()
        .iter()
        .map(|f| {
            let mut metadata = f.metadata().clone();
            if let Some(c) = table.columns.iter().find(|c| names_eq(&c.name, f.name())) {
                metadata.insert(meta::DATA_TYPE.into(), c.data_type.as_str().to_owned());
                let kind = match &c.kind {
                    ColumnKind::Data { .. } => "data",
                    ColumnKind::Calculated { .. } => "calculated",
                    ColumnKind::CalculatedTableColumn { .. } => "calculatedTableColumn",
                    ColumnKind::RowNumber => "rowNumber",
                };
                metadata.insert(meta::KIND.into(), kind.to_owned());
                let mut put = |key: &str, value: Option<&str>| {
                    if let Some(v) = value.map(str::trim).filter(|v| !v.is_empty()) {
                        metadata.insert(key.to_owned(), v.to_owned());
                    }
                };
                put(meta::DESCRIPTION, c.description.as_deref());
                put(meta::FORMAT_STRING, c.format_string.as_deref());
                put(meta::SOURCE_COLUMN, c.source_column());
                if crate::derived::is_derived(c) {
                    metadata.insert(meta::DERIVED.into(), "true".to_owned());
                }
            }
            f.as_ref().clone().with_metadata(metadata)
        })
        .collect();
    let mut metadata = schema.metadata().clone();
    metadata.insert(meta::TABLE.into(), table.name.clone());
    if let Some(d) = table
        .description
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        metadata.insert(meta::TABLE_DESCRIPTION.into(), d.to_owned());
    }
    metadata.insert(meta::CULTURE.into(), culture.to_owned());
    let schema = Schema::new_with_metadata(fields, metadata);
    Ok(batch.clone().with_schema(schema.into())?)
}

impl Session {
    /// One table's data with model metadata attached ([`annotate_batch`]),
    /// ready for [`write_csv`] / [`write_parquet`]. Columns are in model
    /// order; before a refresh the table is empty.
    ///
    /// # Errors
    /// [`SessionError::UnknownTable`].
    pub fn export_batch(&self, table: &str) -> Result<RecordBatch, ExportError> {
        let model = self.model();
        let t = model
            .table(table)
            .ok_or_else(|| SessionError::UnknownTable(table.to_owned()))?;
        let batch = self.table_batch(&t.name)?;
        annotate_batch(&batch, t, &self.culture())
    }

    /// Writes one table as CSV (see [`write_csv`]).
    ///
    /// # Errors
    /// [`SessionError::UnknownTable`], invalid options, write failures.
    pub fn export_csv<W: Write>(
        &self,
        table: &str,
        writer: W,
        options: &CsvExportOptions,
    ) -> Result<ExportSummary, ExportError> {
        write_csv(&self.export_batch(table)?, writer, options)
    }

    /// Writes one table as Parquet (see [`write_parquet`]).
    ///
    /// # Errors
    /// [`SessionError::UnknownTable`], write failures.
    pub fn export_parquet<W: Write + Send>(
        &self,
        table: &str,
        writer: W,
        options: &ParquetExportOptions,
    ) -> Result<ExportSummary, ExportError> {
        write_parquet(&self.export_batch(table)?, writer, options)
    }
}
