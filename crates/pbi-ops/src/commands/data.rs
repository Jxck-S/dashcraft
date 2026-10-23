//! `data.refresh`, `data.tables`, `data.page`, `data.columnStats`,
//! `data.loadCsv`, `data.addDerivedColumn`, `data.removeDerivedColumn`.

use std::path::PathBuf;

use arrow::array::Array;
use arrow::record_batch::RecordBatch;
use arrow::util::display::{ArrayFormatter, FormatOptions};
use pbi_engine::PageRequest;
use pbi_model::{Model, TableKind};
use pbi_session::{CsvOptions, CsvSource, DerivedColumn, Session, value_to_json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::workspace::{SourceParam, source_name};
use super::{BASE, DocParams, bounded, doc, loaded_doc};
use crate::command::{Category, Command, CommandMeta};
use crate::ctx::Ctx;
use crate::dto::{DerivedColumnOut, RefreshOut, ResultColumnOut, TableRefreshOut};
use crate::error::OpError;
use crate::resolve;
use crate::workspace::Workspace;

/// Largest `limit` of `data.page`.
pub const MAX_PAGE_ROWS: usize = 5000;
/// Longest formatted cell (characters).
const MAX_CELL_CHARS: usize = 4000;

// ---------------------------------------------------------------------------
// Tabular helpers (also used by `dax.query`)
// ---------------------------------------------------------------------------

fn truncate(text: String, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", text.get(..cut).unwrap_or_default()),
        None => text,
    }
}

/// `Table[Column]` → (`Table`, `Column`), as DAX query results name columns.
fn split_qualified(name: &str) -> Option<(&str, &str)> {
    let open = name.find('[')?;
    let column = name.get(open + 1..)?.strip_suffix(']')?;
    let table = name.get(..open)?.trim();
    (!table.is_empty()).then_some((table.trim_matches('\''), column))
}

/// Columns of `batch`: model columns (of `table`, or `Table[Column]`
/// names) keep their data type and format string.
pub(crate) fn result_columns(
    model: &Model,
    table: Option<&str>,
    batch: &RecordBatch,
) -> Vec<ResultColumnOut> {
    batch
        .schema()
        .fields()
        .iter()
        .map(|f| {
            let column = match table {
                Some(t) => model.column(t, f.name()),
                None => split_qualified(f.name())
                    .and_then(|(t, c)| model.table(t).and_then(|t| t.column(c))),
            };
            ResultColumnOut {
                name: f.name().clone(),
                data_type: column.map_or_else(
                    || f.data_type().to_string(),
                    |c| c.data_type.as_str().to_owned(),
                ),
                format_string: column.and_then(|c| c.format_string.clone()),
            }
        })
        .collect()
}

/// How [`rows`] writes cells.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CellFormat {
    /// Text formatted with the column format string instead of raw values.
    pub formatted: bool,
    /// With `formatted`, BLANK is `null` instead of `""`.
    pub blank_as_null: bool,
}

/// Row-major cells of `batch`: raw JSON values (BLANK → `null`), or
/// formatted text with `formatted`.
pub(crate) fn rows(
    session: &Session,
    batch: &RecordBatch,
    columns: &[ResultColumnOut],
    format: CellFormat,
) -> Vec<Vec<Value>> {
    let mut rows: Vec<Vec<Value>> = (0..batch.num_rows())
        .map(|_| Vec::with_capacity(columns.len()))
        .collect();
    for (array, column) in batch.columns().iter().zip(columns) {
        match pbi_engine::value::values_from_array(array) {
            Ok(values) => {
                for (row, value) in rows.iter_mut().zip(&values) {
                    row.push(if format.blank_as_null && value.is_blank() {
                        Value::Null
                    } else if format.formatted {
                        Value::String(truncate(
                            session.format_value(value, column.format_string.as_deref()),
                            MAX_CELL_CHARS,
                        ))
                    } else {
                        value_to_json(value)
                    });
                }
            }
            Err(_) => {
                let formatter = ArrayFormatter::try_new(array.as_ref(), &FormatOptions::default());
                for (i, row) in rows.iter_mut().enumerate() {
                    row.push(match &formatter {
                        Ok(f) if !array.is_null(i) => {
                            Value::String(truncate(f.value(i).to_string(), MAX_CELL_CHARS))
                        }
                        _ => Value::Null,
                    });
                }
            }
        }
    }
    rows
}

// ---------------------------------------------------------------------------

/// `data.refresh`.
#[derive(Debug)]
pub struct DataRefresh;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataRefreshParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Tables to refresh (what depends on them is recomputed too). Default:
    /// every table.
    #[serde(default)]
    pub tables: Option<Vec<String>>,
    /// Data source (default: the one given to `doc.open`).
    #[serde(default)]
    pub source: Option<SourceParam>,
}

impl Command for DataRefresh {
    type Params = DataRefreshParams;
    type Output = RefreshOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "data.refresh",
            title: "Refresh data",
            summary: "Loads the data offline (PBIP cache, M partitions with the blob map, embedded PBIX data, calculated objects); per-table rows, time and status.",
            category: Category::Data,
            mutates_document: true,
            long_running: true,
            examples: &[
                ("Every table", r#"{"docId": "d1"}"#),
                (
                    "Evaluate the M queries of one table",
                    r#"{"docId": "d1", "tables": ["Orders"], "source": "query"}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &p.doc_id)?;
        resolve::require_model(&entry)?;
        let model = entry.session().base_model();
        let tables = p
            .tables
            .map(|ts| {
                ts.iter()
                    .map(|t| resolve::table_name(&model, t))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?;
        let source = p.source.map_or(entry.options().source, Into::into);
        let report = entry.refresh(tables, source, ctx).await?;
        Ok(RefreshOut::of(
            &report,
            source_name(source),
            entry.generation(),
        ))
    }
}

// ---------------------------------------------------------------------------

/// `data.tables`.
#[derive(Debug)]
pub struct DataTables;

/// Data status of one table.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DataTableOut {
    pub name: String,
    /// `regular`, `calculatedTable`, `calculationGroup`, `remote`.
    pub kind: String,
    pub hidden: bool,
    pub rows: usize,
    /// Columns stored (data, calculated and derived).
    pub columns: usize,
    pub loaded: bool,
    /// `data.loadCsv` can replace its data (every table of a PBIX; tables
    /// other than calculated tables otherwise).
    pub can_load_csv: bool,
    /// Result of the last refresh or CSV load of this table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_refresh: Option<TableRefreshOut>,
}

/// The PBIP local data cache.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CacheOut {
    /// Entry in the opened folder, e.g. `X.SemanticModel/.pbi/cache.abf`.
    pub entry: String,
    pub size: u64,
    /// Last modification (Power BI Desktop's last refresh), RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DataTablesOut {
    pub tables: Vec<DataTableOut>,
    /// Session derived columns.
    pub derived: Vec<DerivedColumnOut>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache: Option<CacheOut>,
    /// Data generation.
    pub generation: u64,
}

impl Command for DataTables {
    type Params = DocParams;
    type Output = DataTablesOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "data.tables",
            title: "Data status",
            summary: "Loaded rows, columns and last refresh status of every table, the derived columns and the local data cache.",
            category: Category::Data,
            examples: &[("Status of d1", r#"{"docId": "d1"}"#)],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &p.doc_id)?;
        let session = entry.session();
        let pbix = session.is_pbix();
        let tables = session
            .tables()
            .into_iter()
            .map(|t| DataTableOut {
                can_load_csv: pbix || t.kind != TableKind::CalculatedTable,
                kind: serde_json::to_value(t.kind)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default(),
                hidden: t.hidden,
                rows: t.rows,
                columns: t.columns,
                loaded: t.loaded,
                last_refresh: t.last_refresh.as_ref().map(TableRefreshOut::of),
                name: t.name,
            })
            .collect();
        Ok(DataTablesOut {
            tables,
            derived: session
                .derived_columns()
                .iter()
                .map(DerivedColumnOut::of)
                .collect(),
            cache: session.cache_info().map(|c| CacheOut {
                entry: c.entry,
                size: c.size,
                modified: c.modified.map(|m| m.to_rfc3339()),
            }),
            generation: entry.generation(),
        })
    }
}

// ---------------------------------------------------------------------------

/// `data.page`.
#[derive(Debug)]
pub struct DataPage;

/// Sort order of `data.page`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SortParam {
    /// Column name (case-insensitive).
    pub column: String,
    /// Default `true` (BLANKs first).
    #[serde(default = "yes")]
    pub ascending: bool,
}

fn yes() -> bool {
    true
}

fn default_limit() -> usize {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataPageParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Table name (case-insensitive).
    pub table: String,
    /// First row (0-based, after sorting and filtering).
    #[serde(default)]
    pub offset: usize,
    /// Rows to return (default 100, at most 5000).
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub sort: Option<SortParam>,
    /// SQL boolean expression over the table's columns, e.g.
    /// `"Status" = 'Completed'` (quote names with spaces).
    #[serde(default)]
    pub filter: Option<String>,
    /// Cells as text formatted with each column's format string instead of
    /// raw values.
    #[serde(default)]
    pub formatted: bool,
    /// With `formatted`, BLANK cells are `null` instead of `""` (tells an
    /// empty text from BLANK).
    #[serde(default)]
    pub blank_as_null: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DataPageOut {
    /// Table name (model spelling).
    pub table: String,
    pub offset: usize,
    /// Rows passing the filter.
    pub total_rows: usize,
    pub columns: Vec<ResultColumnOut>,
    /// Row-major cells: raw values (numbers, strings, booleans, ISO dates,
    /// `null` for BLANK) or formatted text.
    pub rows: Vec<Vec<Value>>,
}

impl Command for DataPage {
    type Params = DataPageParams;
    type Output = DataPageOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "data.page",
            title: "Table rows",
            summary: "One page of a table's rows (sorted, SQL-filtered), raw or formatted; loads the data on first use.",
            category: Category::Data,
            examples: &[
                (
                    "First 20 rows",
                    r#"{"docId": "d1", "table": "Orders", "limit": 20}"#,
                ),
                (
                    "Filtered and sorted",
                    r#"{"docId": "d1", "table": "Stores", "filter": "\"Region\" = 'West'", "sort": {"column": "Store Name"}, "formatted": true}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let limit = bounded("limit", p.limit, MAX_PAGE_ROWS)?;
        let entry = loaded_doc(ws, &p.doc_id, ctx).await?;
        let session = entry.session();
        let model = session.model();
        let table = resolve::table_name(&model, &p.table)?;
        let filter = p
            .filter
            .map(|f| f.trim().to_owned())
            .filter(|f| !f.is_empty());
        let mut request = PageRequest::new(table.clone(), p.offset, limit);
        if let Some(s) = &p.sort {
            request = request.sort_by(s.column.clone(), s.ascending);
        }
        if let Some(f) = &filter {
            request = request.filter(f.clone());
        }
        let cancel = ctx.cancel_token().child_token();
        let total_rows = session
            .row_count(&table, filter.as_deref(), cancel.clone())
            .await?;
        let batch = session.page(&request, cancel).await?;
        let columns = result_columns(&model, Some(&table), &batch);
        let format = CellFormat {
            formatted: p.formatted,
            blank_as_null: p.blank_as_null,
        };
        let rows = rows(session, &batch, &columns, format);
        Ok(DataPageOut {
            table,
            offset: p.offset,
            total_rows,
            columns,
            rows,
        })
    }
}

// ---------------------------------------------------------------------------

/// `data.columnStats`.
#[derive(Debug)]
pub struct DataColumnStats;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColumnParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Table name (case-insensitive).
    pub table: String,
    /// Column name (case-insensitive).
    pub column: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ColumnStatsOut {
    /// Table name (model spelling).
    pub table: String,
    /// Column name (model spelling).
    pub column: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_type: Option<String>,
    pub rows: usize,
    /// Non-blank values.
    pub count: usize,
    pub blanks: usize,
    /// Distinct non-blank values (text compared case-insensitively).
    pub distinct: usize,
    /// Smallest non-blank value (`null` if none).
    pub min: Value,
    pub max: Value,
    pub min_formatted: String,
    pub max_formatted: String,
}

impl Command for DataColumnStats {
    type Params = ColumnParams;
    type Output = ColumnStatsOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "data.columnStats",
            title: "Column statistics",
            summary: "Rows, blanks, distinct count, min and max of one column; loads the data on first use.",
            category: Category::Data,
            examples: &[(
                "Order status",
                r#"{"docId": "d1", "table": "Orders", "column": "Status"}"#,
            )],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = loaded_doc(ws, &p.doc_id, ctx).await?;
        let session = entry.session();
        let model = session.model();
        let table = resolve::table_name(&model, &p.table)?;
        let column = model.column(&table, &p.column);
        let column_name = column.map_or_else(|| p.column.clone(), |c| c.name.clone());
        let format = column.and_then(|c| c.format_string.clone());
        let stats = session.column_stats(&table, &column_name).await?;
        Ok(ColumnStatsOut {
            data_type: column.map(|c| c.data_type.as_str().to_owned()),
            rows: stats.rows,
            count: stats.count,
            blanks: stats.blanks,
            distinct: stats.distinct,
            min: value_to_json(&stats.min),
            max: value_to_json(&stats.max),
            min_formatted: session.format_value(&stats.min, format.as_deref()),
            max_formatted: session.format_value(&stats.max, format.as_deref()),
            table,
            column: column_name,
        })
    }
}

// ---------------------------------------------------------------------------

/// `data.loadCsv`.
#[derive(Debug)]
pub struct DataLoadCsv;

/// CSV reading options.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CsvParams {
    /// Field delimiter, one ASCII character (default `,`).
    #[serde(default)]
    pub delimiter: Option<char>,
    /// The first record holds column names (default `true`).
    #[serde(default)]
    pub has_header: Option<bool>,
    /// Encoding: WHATWG label (`utf-8`, `windows-1252`, …) or code page
    /// (`65001`, `1252`). Default: BOM, else UTF-8, else Windows-1252.
    #[serde(default)]
    pub encoding: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataLoadCsvParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Table name (case-insensitive).
    pub table: String,
    /// The CSV file.
    pub path: PathBuf,
    #[serde(default)]
    pub options: CsvParams,
}

impl Command for DataLoadCsv {
    type Params = DataLoadCsvParams;
    type Output = RefreshOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "data.loadCsv",
            title: "Load a CSV",
            summary: "Replaces a table's data with a CSV (\"Load CSV manually…\") and recomputes what depends on it.",
            category: Category::Data,
            mutates_document: true,
            long_running: true,
            examples: &[(
                "Orders from a CSV",
                r#"{"docId": "d1", "table": "Orders", "path": "examples/acme-outfitters/data/orders.csv"}"#,
            )],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &p.doc_id)?;
        resolve::require_model(&entry)?;
        let path = ws.policy().check_read(&p.path)?;
        let table = resolve::table_name(&entry.session().base_model(), &p.table)?;
        let defaults = CsvOptions::default();
        let options = CsvOptions {
            delimiter: p.options.delimiter.unwrap_or(defaults.delimiter),
            has_header: p.options.has_header.unwrap_or(defaults.has_header),
            encoding: p.options.encoding.clone(),
        };
        // Load what can be loaded first, so the CSV completes the data.
        entry.ensure_loaded(ctx).await?;
        ctx.progress(format!("{table}: loading {}", p.path.display()), None);
        let report = entry
            .session()
            .load_csv(&table, CsvSource::Path(path), options)
            .await?;
        entry.mark_loaded().await;
        Ok(RefreshOut::of(&report, "manual", entry.generation()))
    }
}

// ---------------------------------------------------------------------------

/// `data.addDerivedColumn`.
#[derive(Debug)]
pub struct DataAddDerivedColumn;

/// How a derived column is computed.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum DerivedKindParam {
    /// "Parse people field": `First Last; …` from a people-picker
    /// text column of the same table.
    People { source_column: String },
    /// A DAX calculated-column expression, evaluated per row.
    Dax { expression: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataAddDerivedColumnParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Table name (case-insensitive).
    pub table: String,
    /// New column name (unique in the table).
    pub name: String,
    pub kind: DerivedKindParam,
}

impl Command for DataAddDerivedColumn {
    type Params = DataAddDerivedColumnParams;
    type Output = DerivedColumnOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "data.addDerivedColumn",
            title: "Add a derived column",
            summary: "Adds a session-only column (people parsing or a DAX expression), re-applied after every refresh; never saved to the model.",
            category: Category::Data,
            mutates_document: true,
            examples: &[
                (
                    "Parse a people field",
                    r#"{"docId": "d1", "table": "Customers", "name": "Account Managers (parsed)", "kind": {"people": {"sourceColumn": "Account Managers"}}}"#,
                ),
                (
                    "A DAX column",
                    r#"{"docId": "d1", "table": "Orders", "name": "Is Completed", "kind": {"dax": {"expression": "Orders[Status] = \"Completed\""}}}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = loaded_doc(ws, &p.doc_id, ctx).await?;
        resolve::require_model(&entry)?;
        let column = match p.kind {
            DerivedKindParam::People { source_column } => DerivedColumn::People { source_column },
            DerivedKindParam::Dax { expression } => DerivedColumn::Expression { dax: expression },
        };
        let info = entry
            .session()
            .add_derived_column(&p.table, &p.name, column)
            .await?;
        entry.bump_generation();
        Ok(DerivedColumnOut::of(&info))
    }
}

// ---------------------------------------------------------------------------

/// `data.removeDerivedColumn`.
#[derive(Debug)]
pub struct DataRemoveDerivedColumn;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DerivedRefParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Table name (case-insensitive).
    pub table: String,
    pub name: String,
}

impl Command for DataRemoveDerivedColumn {
    type Params = DerivedRefParams;
    type Output = DerivedColumnOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "data.removeDerivedColumn",
            title: "Remove a derived column",
            summary: "Removes a session derived column from the table data and the session model.",
            category: Category::Data,
            mutates_document: true,
            examples: &[(
                "Remove a parsed column",
                r#"{"docId": "d1", "table": "Customers", "name": "Account Managers (parsed)"}"#,
            )],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &p.doc_id)?;
        let info = entry
            .session()
            .remove_derived_column(&p.table, &p.name)
            .await?;
        entry.bump_generation();
        Ok(DerivedColumnOut::of(&info))
    }
}
