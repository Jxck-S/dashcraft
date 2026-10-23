//! Column, table and model decoding on top of the [`StorageMap`].

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use arrow::array::{ArrayRef, RecordBatch, RecordBatchOptions, new_null_array};
use arrow::datatypes::{Field, Schema};
use pbi_model::{ColumnKind, Diagnostic, Model};

use super::convert::{self, Source};
use super::dictionary::{Dictionary, parse_dictionary};
use super::error::VertipaqError;
use super::idf::decode_idf;
use super::idfmeta::parse_idfmeta;
use super::{DecodeLimits, InvalidIdPolicy, codes};
use crate::abf::{AbfArchive, open_pbix_datamodel};
use crate::metadata::{ColumnStorage, DictionaryEncoding, ModelOptions, StorageMap, load_model};

/// A decoded column plus non-fatal findings.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DecodedColumn {
    /// The values.
    pub array: ArrayRef,
    /// Rows that were set to BLANK because their data id (or value) was invalid
    /// (only with [`InvalidIdPolicy::Blank`]).
    pub invalid_rows: u64,
    /// Warnings (e.g. [`codes::INVALID_IDS`]).
    pub diagnostics: Vec<Diagnostic>,
}

/// Decoded tables plus diagnostics, from [`decode_all_with_diagnostics`].
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct DecodedTables {
    /// `(table name, batch)` in model order.
    pub tables: Vec<(String, RecordBatch)>,
    /// Findings; columns that failed are BLANK-filled and reported as errors.
    pub diagnostics: Vec<Diagnostic>,
}

/// Result of [`open_pbix_data`]: the model, its decoded tables and diagnostics.
pub type PbixData = (Model, Vec<(String, RecordBatch)>, Vec<Diagnostic>);

/// Decodes `table[column]` (names case-insensitive) into an Arrow array.
///
/// Partitions are concatenated in storage order. Invalid data ids are handled per
/// [`DecodeLimits::invalid_ids`] (an error by default).
pub fn decode_column(
    archive: &AbfArchive,
    storage: &StorageMap,
    table: &str,
    column: &str,
    limits: &DecodeLimits,
) -> Result<ArrayRef, VertipaqError> {
    decode_column_with_diagnostics(archive, storage, table, column, limits).map(|c| c.array)
}

/// Like [`decode_column`], also returning warnings.
pub fn decode_column_with_diagnostics(
    archive: &AbfArchive,
    storage: &StorageMap,
    table: &str,
    column: &str,
    limits: &DecodeLimits,
) -> Result<DecodedColumn, VertipaqError> {
    let table_storage = storage
        .table(table)
        .ok_or_else(|| VertipaqError::TableNotFound(table.to_owned()))?;
    let column_storage =
        table_storage
            .column(column)
            .ok_or_else(|| VertipaqError::ColumnNotFound {
                table: table.to_owned(),
                column: column.to_owned(),
            })?;
    decode_column_storage(archive, column_storage, limits)
        .map_err(|err| err.in_column(&table_storage.name, &column_storage.name))
}

/// Decodes one column given its [`ColumnStorage`].
pub fn decode_column_storage(
    archive: &AbfArchive,
    column: &ColumnStorage,
    limits: &DecodeLimits,
) -> Result<DecodedColumn, VertipaqError> {
    if !column.has_values() {
        return Err(VertipaqError::unsupported(
            "column kind",
            format!("{:?} columns hold no values", column.kind),
        ));
    }
    let dictionary_storage = column.dictionary.as_ref().ok_or_else(|| {
        VertipaqError::corrupt("column storage", "column has no DictionaryStorage row")
    })?;
    match dictionary_storage.encoding {
        DictionaryEncoding::Value | DictionaryEncoding::Hash => {}
        DictionaryEncoding::None => {
            return Err(VertipaqError::unsupported(
                "dictionary encoding",
                "type 0 (internal column without dictionary)",
            ));
        }
        DictionaryEncoding::Other(code) => {
            return Err(VertipaqError::unsupported(
                "dictionary encoding",
                format!("DictionaryStorage.Type {code}"),
            ));
        }
    }
    let ids = decode_ids(archive, column, limits)?;
    let dictionary: Option<Dictionary> = match (
        dictionary_storage.encoding,
        dictionary_storage.path.as_deref(),
    ) {
        (DictionaryEncoding::Hash, Some(path)) => {
            Some(parse_dictionary(&archive.read_file(path)?, limits)?)
        }
        _ => None,
    };
    let source = match dictionary_storage.encoding {
        DictionaryEncoding::Value => Source::Value {
            base_id: dictionary_storage.base_id,
            magnitude: dictionary_storage.magnitude,
        },
        // Hash encoding without a file: the only value is BLANK.
        _ => dictionary
            .as_ref()
            .map_or(Source::Blank, Source::Dictionary),
    };

    let converted = convert::convert(&ids, &source, &column.data_type, limits.max_column_bytes)?;
    let mut diagnostics = Vec::new();
    let invalid_rows = converted.invalid.count;
    if invalid_rows > 0 {
        let first = converted.invalid.first.unwrap_or_default();
        match limits.invalid_ids {
            InvalidIdPolicy::Error => {
                return Err(VertipaqError::InvalidDataIds {
                    count: invalid_rows,
                    first,
                });
            }
            InvalidIdPolicy::Blank => diagnostics.push(Diagnostic::warning(
                codes::INVALID_IDS,
                format!(
                    "column {:?}: {invalid_rows} row(s) with invalid data ids or values were set to BLANK (first: {first})",
                    column.name
                ),
            )),
        }
    }
    Ok(DecodedColumn {
        array: converted.array,
        invalid_rows,
        diagnostics,
    })
}

/// Data ids of every partition, in storage order.
fn decode_ids(
    archive: &AbfArchive,
    column: &ColumnStorage,
    limits: &DecodeLimits,
) -> Result<Vec<u32>, VertipaqError> {
    let mut ids = Vec::new();
    for partition in &column.partitions {
        let missing = |what: &str| {
            VertipaqError::corrupt(
                "column storage",
                format!(
                    "partition {} has no {what} file",
                    partition.partition_storage_id
                ),
            )
        };
        let idf_path = partition.idf.as_deref().ok_or_else(|| missing(".idf"))?;
        let parsed;
        let meta = match &partition.meta {
            Some(meta) => meta,
            None => {
                let meta_path = partition
                    .idfmeta
                    .as_deref()
                    .ok_or_else(|| missing(".idfmeta"))?;
                parsed = parse_idfmeta(&archive.read_file(meta_path)?)?;
                &parsed
            }
        };
        if let (Some(expected), Some(actual)) = (partition.record_count, meta.records())
            && expected != actual
        {
            return Err(VertipaqError::corrupt(
                "idfmeta",
                format!(
                    "segments hold {actual} rows but partition {} has {expected}",
                    partition.partition_storage_id
                ),
            ));
        }
        decode_idf(
            &archive.read_file(idf_path)?,
            meta,
            limits.max_rows,
            &mut ids,
        )?;
    }
    Ok(ids)
}

/// The Arrow type used for a BLANK-filled column of this logical type.
fn fallback_type(column: &ColumnStorage) -> arrow::datatypes::DataType {
    convert::arrow_type(&column.data_type, &Source::Blank)
}

/// Decodes a model table into a [`RecordBatch`]: model columns in model order, named
/// as in the model, without the row-number column; partitions concatenated.
pub fn decode_table(
    archive: &AbfArchive,
    storage: &StorageMap,
    model: &Model,
    table: &str,
    limits: &DecodeLimits,
) -> Result<RecordBatch, VertipaqError> {
    let mut diagnostics = Vec::new();
    decode_table_inner(
        archive,
        storage,
        model,
        table,
        limits,
        None,
        &mut diagnostics,
    )
}

/// Like [`decode_table`], but a column that fails is replaced by an all-BLANK column
/// and reported in `diagnostics` (only table-level problems are errors).
pub fn decode_table_with_diagnostics(
    archive: &AbfArchive,
    storage: &StorageMap,
    model: &Model,
    table: &str,
    limits: &DecodeLimits,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<RecordBatch, VertipaqError> {
    let mut lenient = Vec::new();
    let batch = decode_table_inner(
        archive,
        storage,
        model,
        table,
        limits,
        Some(&mut lenient),
        diagnostics,
    )?;
    diagnostics.append(&mut lenient);
    Ok(batch)
}

fn decode_table_inner(
    archive: &AbfArchive,
    storage: &StorageMap,
    model: &Model,
    table: &str,
    limits: &DecodeLimits,
    mut lenient: Option<&mut Vec<Diagnostic>>,
    warnings: &mut Vec<Diagnostic>,
) -> Result<RecordBatch, VertipaqError> {
    let model_table = model
        .table(table)
        .ok_or_else(|| VertipaqError::TableNotFound(table.to_owned()))?;
    let table_storage = storage
        .table(&model_table.name)
        .ok_or_else(|| VertipaqError::TableNotFound(model_table.name.clone()))?;
    let rows = table_storage.row_count;
    if rows > limits.max_rows {
        return Err(VertipaqError::LimitExceeded {
            what: "rows in a table",
            value: rows,
            limit: limits.max_rows,
        });
    }
    let rows_usize = usize::try_from(rows).map_err(|_| VertipaqError::LimitExceeded {
        what: "rows in a table",
        value: rows,
        limit: usize::MAX as u64,
    })?;

    let columns: Vec<_> = model_table
        .columns
        .iter()
        .filter(|c| !matches!(c.kind, ColumnKind::RowNumber))
        .collect();
    let jobs: Vec<Option<&ColumnStorage>> = columns
        .iter()
        .map(|c| table_storage.column(&c.name))
        .collect();
    let results = parallel_map(&jobs, rows, |job| match job {
        Some(column) => decode_column_storage(archive, column, limits).and_then(|decoded| {
            let actual = decoded.array.len() as u64;
            if actual == rows {
                Ok(decoded)
            } else {
                Err(VertipaqError::RowCountMismatch {
                    expected: rows,
                    actual,
                })
            }
        }),
        None => Err(VertipaqError::ColumnNotFound {
            table: model_table.name.clone(),
            column: String::new(),
        }),
    });

    // Without any decoded column the row count is backed by metadata only; do not
    // allocate BLANK columns for it.
    if lenient.is_some() && rows > 0 && !results.is_empty() && results.iter().all(Result::is_err) {
        let first = results.into_iter().find_map(Result::err);
        let first = first.map(|e| e.to_string()).unwrap_or_default();
        return Err(VertipaqError::corrupt(
            "table",
            format!(
                "no column of {:?} could be decoded (first error: {first})",
                model_table.name
            ),
        ));
    }

    let mut fields = Vec::with_capacity(columns.len());
    let mut arrays = Vec::with_capacity(columns.len());
    for ((column, job), result) in columns.iter().zip(&jobs).zip(results) {
        let array = match result {
            Ok(decoded) => {
                warnings.extend(
                    decoded
                        .diagnostics
                        .into_iter()
                        .map(|d| d.with_object(column.id)),
                );
                decoded.array
            }
            Err(err) => {
                let err = match err {
                    VertipaqError::ColumnNotFound { table, .. } => VertipaqError::ColumnNotFound {
                        table,
                        column: column.name.clone(),
                    },
                    other => other.in_column(&model_table.name, &column.name),
                };
                let Some(sink) = lenient.as_deref_mut() else {
                    return Err(err);
                };
                sink.push(
                    Diagnostic::error(
                        codes::COLUMN_FAILED,
                        format!("{err}; the column was filled with BLANK"),
                    )
                    .with_object(column.id),
                );
                let data_type = match job {
                    Some(storage) => fallback_type(storage),
                    None => convert::arrow_type(&column.data_type, &Source::Blank),
                };
                new_null_array(&data_type, rows_usize)
            }
        };
        fields.push(Field::new(
            column.name.clone(),
            array.data_type().clone(),
            true,
        ));
        arrays.push(array);
    }
    let schema = Arc::new(Schema::new(fields));
    let options = RecordBatchOptions::new().with_row_count(Some(rows_usize));
    Ok(RecordBatch::try_new_with_options(schema, arrays, &options)?)
}

/// Decodes every non-internal model table (strict: the first error aborts).
pub fn decode_all(
    archive: &AbfArchive,
    storage: &StorageMap,
    model: &Model,
    limits: &DecodeLimits,
) -> Result<Vec<(String, RecordBatch)>, VertipaqError> {
    let mut out = Vec::new();
    let mut total = 0u64;
    for table in model_tables(storage, model) {
        let batch = decode_table(archive, storage, model, table, limits)?;
        total = total.saturating_add(batch.get_array_memory_size() as u64);
        if total > limits.max_total_bytes {
            return Err(VertipaqError::LimitExceeded {
                what: "decoded bytes",
                value: total,
                limit: limits.max_total_bytes,
            });
        }
        out.push((table.to_owned(), batch));
    }
    Ok(out)
}

/// Decodes every non-internal model table, collecting problems instead of failing: a
/// broken column becomes an all-BLANK column ([`codes::COLUMN_FAILED`]), a broken table
/// is skipped ([`codes::TABLE_FAILED`]), and decoding stops once
/// [`DecodeLimits::max_total_bytes`] is exceeded ([`codes::BUDGET_EXHAUSTED`]).
pub fn decode_all_with_diagnostics(
    archive: &AbfArchive,
    storage: &StorageMap,
    model: &Model,
    limits: &DecodeLimits,
) -> DecodedTables {
    let mut out = DecodedTables::default();
    let mut total = 0u64;
    for table in model_tables(storage, model) {
        if total > limits.max_total_bytes {
            out.diagnostics.push(Diagnostic::error(
                codes::BUDGET_EXHAUSTED,
                format!(
                    "table {table:?} was not decoded: decoded data exceeds {} bytes",
                    limits.max_total_bytes
                ),
            ));
            continue;
        }
        match decode_table_with_diagnostics(
            archive,
            storage,
            model,
            table,
            limits,
            &mut out.diagnostics,
        ) {
            Ok(batch) => {
                total = total.saturating_add(batch.get_array_memory_size() as u64);
                out.tables.push((table.to_owned(), batch));
            }
            Err(err) => {
                let mut diagnostic = Diagnostic::error(
                    codes::TABLE_FAILED,
                    format!("table {table:?} could not be decoded: {err}"),
                );
                if let Some(t) = model.table(table) {
                    diagnostic = diagnostic.with_object(t.id);
                }
                out.diagnostics.push(diagnostic);
            }
        }
    }
    out
}

/// Names of the model tables that have stored data (internal tables excluded).
fn model_tables<'m>(storage: &StorageMap, model: &'m Model) -> Vec<&'m str> {
    model
        .tables
        .iter()
        .filter(|t| storage.table(&t.name).is_none_or(|s| !s.is_internal))
        .map(|t| t.name.as_str())
        .collect()
}

/// PBIX `DataModel` bytes (XPress9 or plain ABF) → model, decoded tables, diagnostics.
///
/// Container and metadata errors fail; column and table problems are reported as
/// diagnostics (see [`decode_all_with_diagnostics`]).
pub fn open_pbix_data(
    datamodel_bytes: &[u8],
    limits: &DecodeLimits,
) -> Result<PbixData, VertipaqError> {
    let archive = open_pbix_datamodel(datamodel_bytes, &limits.container)?;
    let loaded = load_model(&archive, &ModelOptions::default())?;
    let decoded = decode_all_with_diagnostics(&archive, &loaded.storage, &loaded.model, limits);
    Ok((loaded.model, decoded.tables, decoded.diagnostics))
}

/// Tables smaller than this are decoded on the calling thread.
const PARALLEL_MIN_ROWS: u64 = 64 * 1024;

/// `f` over `items`, on scoped worker threads for large tables; results in input order.
fn parallel_map<T: Sync, R: Send>(
    items: &[T],
    rows: u64,
    f: impl Fn(&T) -> Result<R, VertipaqError> + Sync,
) -> Vec<Result<R, VertipaqError>> {
    let workers = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(8)
        .min(items.len());
    if workers <= 1 || rows < PARALLEL_MIN_ROWS {
        return items.iter().map(&f).collect();
    }
    let next = AtomicUsize::new(0);
    let worker = || {
        let mut done = Vec::new();
        loop {
            let i = next.fetch_add(1, Ordering::Relaxed);
            let Some(item) = items.get(i) else {
                break;
            };
            done.push((i, f(item)));
        }
        done
    };
    let mut slots: Vec<Option<Result<R, VertipaqError>>> = Vec::new();
    slots.resize_with(items.len(), || None);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .filter_map(|_| std::thread::Builder::new().spawn_scoped(scope, worker).ok())
            .collect();
        for handle in handles {
            if let Ok(done) = handle.join() {
                for (i, result) in done {
                    if let Some(slot) = slots.get_mut(i) {
                        *slot = Some(result);
                    }
                }
            }
        }
    });
    // Items no worker reached (spawn failure) are decoded here.
    slots
        .into_iter()
        .zip(items)
        .map(|(slot, item)| slot.unwrap_or_else(|| f(item)))
        .collect()
}
