//! `export.table`, `export.model`, `convert.pbip`, `convert.pbit`.

use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use pbi_core::{pbip_files, pbit_bytes};
use pbi_model::{Model, names_eq};
use pbi_session::{
    CsvExportOptions, MarkdownOptions, ModelDocOptions, ParquetCompression, ParquetExportOptions,
    TableStatus, model_doc, model_to_markdown,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{BASE, doc, loaded_doc};
use crate::command::{Category, Command, CommandMeta};
use crate::ctx::Ctx;
use crate::error::OpError;
use crate::files::{check_output_file, write_bytes, write_file_atomic};
use crate::resolve;
use crate::workspace::Workspace;

/// Largest inline model documentation (bytes); pass `path` beyond.
pub const MAX_INLINE_DOC_BYTES: usize = 16 * 1024 * 1024;

/// Table export format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum TableFormat {
    Csv,
    Parquet,
}

/// Parquet compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum CompressionParam {
    #[default]
    Zstd,
    Snappy,
    None,
}

/// `export.table`.
#[derive(Debug)]
pub struct ExportTable;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportTableParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Table name (case-insensitive).
    pub table: String,
    pub format: TableFormat,
    /// The output file.
    pub path: PathBuf,
    /// CSV: values formatted with each column's format string (as Power BI
    /// shows them) instead of raw values.
    #[serde(default)]
    pub formatted: bool,
    /// CSV: prefix text cells starting with `=`, `+`, `-`, `@`, TAB or CR
    /// with `'` (spreadsheet formula-injection guard).
    #[serde(default)]
    pub excel_safe: bool,
    /// CSV: start with a UTF-8 BOM.
    #[serde(default)]
    pub bom: bool,
    /// CSV: field delimiter (default `,`).
    #[serde(default)]
    pub delimiter: Option<char>,
    /// Parquet: compression (default `zstd`).
    #[serde(default)]
    pub compression: CompressionParam,
    /// Replace an existing file.
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExportTableOut {
    /// Table name (model spelling).
    pub table: String,
    pub format: TableFormat,
    pub path: String,
    /// Data rows (without a CSV header).
    pub rows: usize,
    pub columns: usize,
    /// File size.
    pub bytes: u64,
}

impl Command for ExportTable {
    type Params = ExportTableParams;
    type Output = ExportTableOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "export.table",
            title: "Export a table",
            summary: "Writes one table's data as CSV or Parquet (with model metadata); loads the data on first use.",
            category: Category::Export,
            long_running: true,
            writes_files: true,
            examples: &[
                (
                    "CSV for Excel",
                    r#"{"docId": "d1", "table": "Orders", "format": "csv", "path": "target/tmp/orders.csv", "formatted": true, "excelSafe": true, "bom": true}"#,
                ),
                (
                    "Parquet",
                    r#"{"docId": "d1", "table": "Returns", "format": "parquet", "path": "target/tmp/returns.parquet", "overwrite": true}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let path = check_output_file(ws.policy(), &p.path, p.overwrite)?;
        let entry = loaded_doc(ws, &p.doc_id, ctx).await?;
        resolve::require_model(&entry)?;
        let session = entry.session().clone();
        let table = resolve::table_name(&session.model(), &p.table)?;
        if let Some(TableStatus::Error { message, .. }) =
            session.table_status(&table).map(|t| t.status)
        {
            return Err(OpError::query(format!(
                "table '{table}' failed to load, nothing was written: {message}"
            )));
        }
        let format = p.format;
        let target = path.clone();
        let t = table.clone();
        let summary = tokio::task::spawn_blocking(move || {
            write_file_atomic(&target, p.overwrite, |w| match format {
                TableFormat::Csv => {
                    let options = CsvExportOptions {
                        delimiter: p.delimiter.unwrap_or(','),
                        formatted: p.formatted,
                        bom: p.bom,
                        escape_formulas: p.excel_safe,
                        ..CsvExportOptions::default()
                    };
                    Ok(session.export_csv(&t, w, &options)?)
                }
                TableFormat::Parquet => {
                    let options = ParquetExportOptions {
                        compression: match p.compression {
                            CompressionParam::Zstd => ParquetCompression::Zstd,
                            CompressionParam::Snappy => ParquetCompression::Snappy,
                            CompressionParam::None => ParquetCompression::None,
                        },
                    };
                    Ok(session.export_parquet(&t, w, &options)?)
                }
            })
        })
        .await??;
        let bytes = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        Ok(ExportTableOut {
            table,
            format,
            path: path.to_string_lossy().into_owned(),
            rows: summary.rows,
            columns: summary.columns,
            bytes,
        })
    }
}

// ---------------------------------------------------------------------------

/// `export.model`.
#[derive(Debug)]
pub struct ExportModel;

/// Model documentation format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ModelFormat {
    /// The model doc JSON (`schemaVersion` 1).
    Json,
    /// A Markdown data dictionary.
    #[serde(alias = "md")]
    Markdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportModelParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    pub format: ModelFormat,
    /// Write to this file; without it the documentation is returned inline.
    #[serde(default)]
    pub path: Option<PathBuf>,
    /// Include DAX and M (default `true`).
    #[serde(default)]
    pub include_expressions: Option<bool>,
    /// Include hidden objects (default `true`).
    #[serde(default)]
    pub include_hidden: Option<bool>,
    /// Markdown title (default: the file or project name).
    #[serde(default)]
    pub title: Option<String>,
    /// Replace an existing file.
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExportModelOut {
    pub format: ModelFormat,
    /// The written file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Size of the documentation.
    pub bytes: usize,
    /// The model doc (inline JSON; shape in docs/CLI.md, `export --model`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json: Option<Value>,
    /// The data dictionary (inline Markdown).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub markdown: Option<String>,
}

/// The file or project name, else the model name.
fn default_title(path: &Path, model: &Model) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(str::trim)
        .filter(|s| {
            !s.is_empty()
                && !s.starts_with('.')
                && !["model", "definition", "datamodelschema"]
                    .iter()
                    .any(|generic| s.eq_ignore_ascii_case(generic))
        })
        .map_or_else(|| model.name.clone(), str::to_owned)
}

impl Command for ExportModel {
    type Params = ExportModelParams;
    type Output = ExportModelOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "export.model",
            title: "Document the model",
            summary: "The model documentation as JSON (model doc) or a Markdown data dictionary, inline or to a file; no data needed.",
            category: Category::Export,
            writes_files: true,
            examples: &[
                (
                    "Inline Markdown",
                    r#"{"docId": "d1", "format": "markdown"}"#,
                ),
                (
                    "JSON to a file, no DAX/M",
                    r#"{"docId": "d1", "format": "json", "path": "target/tmp/model.json", "includeExpressions": false, "overwrite": true}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let target = p
            .path
            .as_deref()
            .map(|path| check_output_file(ws.policy(), path, p.overwrite))
            .transpose()?;
        let entry = doc(ws, &p.doc_id)?;
        resolve::require_model(&entry)?;
        let model = entry.session().base_model();
        let include_expressions = p.include_expressions.unwrap_or(true);
        let include_hidden = p.include_hidden.unwrap_or(true);
        let (text, json, markdown) = match p.format {
            ModelFormat::Json => {
                let doc = model_doc(
                    &model,
                    &ModelDocOptions {
                        include_expressions,
                        include_hidden,
                    },
                );
                let value = serde_json::to_value(&doc).map_err(|e| {
                    OpError::internal(format!("cannot serialize the model doc: {e}"))
                })?;
                let mut text = serde_json::to_string_pretty(&value).map_err(|e| {
                    OpError::internal(format!("cannot serialize the model doc: {e}"))
                })?;
                text.push('\n');
                (text, Some(value), None)
            }
            ModelFormat::Markdown => {
                let title = p
                    .title
                    .clone()
                    .unwrap_or_else(|| default_title(entry.path(), &model));
                let text = model_to_markdown(
                    &model,
                    &MarkdownOptions {
                        include_expressions,
                        include_hidden,
                        title: Some(title),
                    },
                );
                (text.clone(), None, Some(text))
            }
        };
        let bytes = text.len();
        match target {
            Some(path) => {
                let overwrite = p.overwrite;
                let out = path.clone();
                tokio::task::spawn_blocking(move || write_bytes(&out, overwrite, text.as_bytes()))
                    .await??;
                Ok(ExportModelOut {
                    format: p.format,
                    path: Some(path.to_string_lossy().into_owned()),
                    bytes,
                    json: None,
                    markdown: None,
                })
            }
            None if bytes > MAX_INLINE_DOC_BYTES => Err(OpError::invalid_params(format!(
                "the documentation is {bytes} bytes; pass path to write it to a file"
            ))),
            None => Ok(ExportModelOut {
                format: p.format,
                path: None,
                bytes,
                json,
                markdown,
            }),
        }
    }
}

// ---------------------------------------------------------------------------

/// `convert.pbip`.
#[derive(Debug)]
pub struct ConvertPbip;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConvertPbipParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Output folder (created if missing; must be empty unless
    /// `overwrite`).
    pub out_dir: PathBuf,
    /// Write into a non-empty folder (files with the same paths are
    /// replaced).
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConvertPbipOut {
    pub out_dir: String,
    /// Files written, relative to `outDir`.
    pub files: Vec<String>,
    pub bytes: u64,
}

impl Command for ConvertPbip {
    type Params = ConvertPbipParams;
    type Output = ConvertPbipOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "convert.pbip",
            title: "Save as PBIP",
            summary: "Writes the document as a PBIP project (TMDL + PBIR) into a folder.",
            category: Category::Convert,
            long_running: true,
            writes_files: true,
            examples: &[(
                "To a new folder",
                r#"{"docId": "d1", "outDir": "target/tmp/project"}"#,
            )],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let out_dir = ws.policy().check_write(&p.out_dir)?;
        if let Ok(meta) = fs::metadata(&out_dir) {
            if !meta.is_dir() {
                return Err(OpError::invalid_params(format!(
                    "{} exists and is not a folder",
                    p.out_dir.display()
                )));
            }
            let mut entries = fs::read_dir(&out_dir)
                .map_err(|e| OpError::io(format!("cannot read {}: {e}", p.out_dir.display())))?;
            if entries.next().is_some() && !p.overwrite {
                return Err(OpError::io(format!(
                    "{} is not empty; pass \"overwrite\": true to write into it",
                    p.out_dir.display()
                )));
            }
        }
        let entry = doc(ws, &p.doc_id)?;
        let opened = entry.session().opened().clone();
        let policy = ws.policy().clone();
        let dir = out_dir.clone();
        let (files, bytes) = tokio::task::spawn_blocking(move || {
            let files = pbip_files(&opened)?;
            let mut written = Vec::with_capacity(files.len());
            let mut bytes = 0u64;
            for (rel, data) in files {
                let rel_path = Path::new(&rel);
                if rel.is_empty()
                    || !rel_path
                        .components()
                        .all(|c| matches!(c, Component::Normal(_)))
                {
                    return Err(OpError::unsupported(format!(
                        "refusing to write unsafe path {rel:?}"
                    )));
                }
                let path = policy.check_write(&dir.join(rel_path))?;
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(|e| {
                        OpError::io(format!("cannot create {}: {e}", parent.display()))
                    })?;
                }
                write_bytes(&path, true, &data)?;
                bytes += data.len() as u64;
                written.push(rel);
            }
            Ok::<_, OpError>((written, bytes))
        })
        .await??;
        Ok(ConvertPbipOut {
            out_dir: out_dir.to_string_lossy().into_owned(),
            files,
            bytes,
        })
    }
}

// ---------------------------------------------------------------------------

/// `convert.pbit`.
#[derive(Debug)]
pub struct ConvertPbit;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConvertPbitParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Output `.pbit` file.
    pub out_path: PathBuf,
    /// The report to include (a PBIT holds one; exact name, default the
    /// first).
    #[serde(default)]
    pub report: Option<String>,
    /// Replace an existing file.
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConvertPbitOut {
    pub out_path: String,
    /// The report written.
    pub report: String,
    /// Reports left out (a PBIT holds one).
    pub omitted_reports: Vec<String>,
    pub bytes: u64,
}

impl Command for ConvertPbit {
    type Params = ConvertPbitParams;
    type Output = ConvertPbitOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "convert.pbit",
            title: "Export a PBIT template",
            summary: "Writes a PBIT template (model schema and one report, no data).",
            category: Category::Convert,
            long_running: true,
            writes_files: true,
            examples: &[(
                "One report",
                r#"{"docId": "d1", "outPath": "target/tmp/stores.pbit", "report": "Acme Outfitters - Stores", "overwrite": true}"#,
            )],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let out = check_output_file(ws.policy(), &p.out_path, p.overwrite)?;
        let entry = doc(ws, &p.doc_id)?;
        let mut opened = entry.session().opened().clone();
        let names: Vec<String> = opened
            .document
            .reports
            .iter()
            .map(|r| r.name.clone())
            .collect();
        if names.is_empty() {
            return Err(OpError::no_document("the document has no report to export"));
        }
        if opened.document.reports.len() != opened.report_sources.len() {
            return Err(OpError::internal(
                "reports and report sources are not aligned",
            ));
        }
        let index = match &p.report {
            Some(name) => names
                .iter()
                .position(|r| names_eq(r, name))
                .ok_or_else(|| {
                    OpError::not_found(format!("no report named '{name}'"))
                        .with_data(serde_json::json!({ "reports": names }))
                })?,
            None => 0,
        };
        if index > 0 {
            let report = opened.document.reports.remove(index);
            opened.document.reports.insert(0, report);
            let source = opened.report_sources.remove(index);
            opened.report_sources.insert(0, source);
        }
        let overwrite = p.overwrite;
        let target = out.clone();
        let bytes = tokio::task::spawn_blocking(move || {
            let bytes = pbit_bytes(&opened)?;
            write_file_atomic(&target, overwrite, |w| {
                w.write_all(&bytes)
                    .map_err(|e| OpError::io(format!("cannot write {}: {e}", target.display())))
            })?;
            Ok::<_, OpError>(bytes.len() as u64)
        })
        .await??;
        let report = names.get(index).cloned().unwrap_or_default();
        Ok(ConvertPbitOut {
            out_path: out.to_string_lossy().into_owned(),
            omitted_reports: names.into_iter().filter(|n| *n != report).collect(),
            report,
            bytes,
        })
    }
}
