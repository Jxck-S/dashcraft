//! `dax.query`, `dax.evaluateMeasure`, `measures.values`, `dax.diagnostics`,
//! `dax.signatureHelp`, `dax.hover`, `dax.complete`, `dax.tokens`,
//! `dax.references`, `dax.format`.

use std::sync::Arc;
use std::time::Instant;

use pbi_model::{Model, Severity, names_eq};
use pbi_session::dax_service::{
    self, Casing, CompletionKind, Diagnostic, FormatError, FormatOptions, HoverKind,
    MAX_INDENT_WIDTH, NameDetails, Schema, SemanticKind, SourceKind, Symbol,
};
use pbi_session::{FilterSpec, filters_from_page, filters_from_report};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::data::{CellFormat, result_columns, rows};
use super::{BASE, bounded, doc, loaded_doc, millis};
use crate::command::{Category, Command, CommandMeta};
use crate::ctx::Ctx;
use crate::dto::{
    ColumnFilterParam, FilterSpecOut, MeasureValueOut, RefreshCounts, ResultColumnOut,
};
use crate::error::OpError;
use crate::resolve;
use crate::workspace::{DocEntry, Workspace};

/// Largest `maxRows` of `dax.query`.
pub const MAX_QUERY_ROWS: usize = 5000;

/// Largest `maxItems` of `dax.complete`.
pub const MAX_COMPLETION_ITEMS: usize = 1000;

/// Largest `maxWidth` of `dax.format`.
pub const MAX_FORMAT_WIDTH: usize = 10_000;

fn default_max_items() -> usize {
    100
}

fn default_max_rows() -> usize {
    1000
}

/// Report (and page) filters plus explicit column filters.
fn filter_spec(
    entry: &DocEntry,
    report: Option<&str>,
    page: Option<&str>,
    filters: Vec<ColumnFilterParam>,
) -> Result<(FilterSpec, Option<String>, Option<String>), OpError> {
    let session = entry.session();
    let mut spec = FilterSpec::new();
    let mut report_name = None;
    let mut page_name = None;
    if report.is_some() || page.is_some() {
        let ri = resolve::report_index(session, report)?;
        let r = resolve::report_at(session, ri)?;
        spec.extend(filters_from_report(&r));
        report_name = Some(r.name.clone());
        if page.is_some() {
            let pi = resolve::page_index(&r, page)?;
            if let Some(pg) = r.pages.get(pi) {
                spec.extend(filters_from_page(pg));
                page_name = Some(pg.display_name.clone());
            }
        }
    }
    for f in filters {
        spec.filters.push(f.into());
    }
    Ok((spec, report_name, page_name))
}

// ---------------------------------------------------------------------------

/// `dax.query`.
#[derive(Debug)]
pub struct DaxQuery;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DaxQueryParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// A DAX query: `EVALUATE …`, with `DEFINE` / `ORDER BY`.
    pub query: String,
    /// Rows returned per result table (default 1000, at most 5000).
    #[serde(default = "default_max_rows")]
    pub max_rows: usize,
    /// Outer filter context (`table[column] IN values`, combined with AND).
    #[serde(default)]
    pub filters: Vec<ColumnFilterParam>,
    /// Cells as formatted text instead of raw values.
    #[serde(default)]
    pub formatted: bool,
    /// With `formatted`, BLANK cells are `null` instead of `""`.
    #[serde(default)]
    pub blank_as_null: bool,
}

/// One result table of a DAX query.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DaxTableOut {
    pub columns: Vec<ResultColumnOut>,
    /// Row-major cells (raw values, `null` for BLANK; or formatted text).
    pub rows: Vec<Vec<Value>>,
    pub total_rows: usize,
    /// More rows than `maxRows`.
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DaxQueryOut {
    /// Semantic document revision captured with the query engine snapshot.
    pub document_revision: u64,
    /// One table per `EVALUATE`.
    pub tables: Vec<DaxTableOut>,
    pub duration_ms: u64,
}

impl Command for DaxQuery {
    type Params = DaxQueryParams;
    type Output = DaxQueryOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "dax.query",
            title: "Run a DAX query",
            summary: "Evaluates a DAX query (EVALUATE …) and returns columns and rows per result table; loads the data on first use.",
            category: Category::Dax,
            long_running: true,
            examples: &[
                (
                    "One value",
                    r#"{"docId": "d1", "query": "EVALUATE ROW(\"n\", [Total Sales])"}"#,
                ),
                (
                    "Grouped, under a filter",
                    r#"{"docId": "d1", "query": "EVALUATE SUMMARIZECOLUMNS(Stores[Region], \"Sales\", [Total Sales])", "filters": [{"table": "Stores", "column": "Channel", "values": ["Retail"]}]}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let max_rows = bounded("maxRows", p.max_rows, MAX_QUERY_ROWS)?;
        if p.query.trim().is_empty() {
            return Err(OpError::invalid_params("the query is empty"));
        }
        let entry = loaded_doc(ws, &p.doc_id, ctx).await?;
        let (spec, _, _) = filter_spec(&entry, None, None, p.filters)?;
        let started = Instant::now();
        let session = entry.session();
        let (document_revision, model, batches) = session
            .dax_query_with_revision(&p.query, &spec, ctx.cancel_token().child_token())
            .await?;
        let format = CellFormat {
            formatted: p.formatted,
            blank_as_null: p.blank_as_null,
        };
        let tables = batches
            .iter()
            .map(|batch| {
                let total = batch.num_rows();
                let shown = batch.slice(0, total.min(max_rows));
                let columns = result_columns(&model, None, &shown);
                DaxTableOut {
                    rows: rows(session, &shown, &columns, format),
                    columns,
                    total_rows: total,
                    truncated: total > max_rows,
                }
            })
            .collect();
        Ok(DaxQueryOut {
            document_revision,
            tables,
            duration_ms: millis(started),
        })
    }
}

// ---------------------------------------------------------------------------

/// `dax.evaluateMeasure`.
#[derive(Debug)]
pub struct DaxEvaluateMeasure;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvaluateMeasureParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// `Name`, `[Name]` or `Table[Name]`.
    pub measure: String,
    /// Apply this report's report-level filters.
    #[serde(default)]
    pub report: Option<String>,
    /// Also apply this page's filters (of `report`, default the first).
    #[serde(default)]
    pub page: Option<String>,
    /// Extra column filters (AND).
    #[serde(default)]
    pub filters: Vec<ColumnFilterParam>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EvaluateMeasureOut {
    /// Semantic document revision captured with the query engine snapshot.
    pub document_revision: u64,
    pub measure: MeasureValueOut,
    /// The filters applied, and those that could not be.
    pub filters: FilterSpecOut,
}

impl Command for DaxEvaluateMeasure {
    type Params = EvaluateMeasureParams;
    type Output = EvaluateMeasureOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "dax.evaluateMeasure",
            title: "Evaluate a measure",
            summary: "One measure's value (raw and formatted, or ⚠ error with its DAX) under report/page and column filters.",
            category: Category::Dax,
            examples: &[
                ("Unfiltered", r#"{"docId": "d1", "measure": "Total Sales"}"#),
                (
                    "Under a report's filters",
                    r#"{"docId": "d1", "measure": "[Margin %]", "report": "Stores"}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = loaded_doc(ws, &p.doc_id, ctx).await?;
        resolve::require_model(&entry)?;
        let (spec, _, _) = filter_spec(&entry, p.report.as_deref(), p.page.as_deref(), p.filters)?;
        let (document_revision, value) = entry
            .session()
            .evaluate_measure_with_revision(&p.measure, &spec, ctx.cancel_token().child_token())
            .await?;
        Ok(EvaluateMeasureOut {
            document_revision,
            measure: MeasureValueOut::from(&value),
            filters: FilterSpecOut::from(&spec),
        })
    }
}

// ---------------------------------------------------------------------------

/// `measures.values`.
#[derive(Debug)]
pub struct MeasuresValues;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeasuresValuesParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Evaluate under this report's report-level filters.
    #[serde(default)]
    pub report: Option<String>,
    /// Also apply this page's filters.
    #[serde(default)]
    pub page: Option<String>,
    /// Extra column filters (AND).
    #[serde(default)]
    pub filters: Vec<ColumnFilterParam>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeasuresValuesOut {
    /// Semantic document revision captured with the query engine snapshot.
    pub document_revision: u64,
    /// Every measure evaluated without error.
    pub ok: bool,
    /// Report name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<String>,
    /// Page display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    /// Filters applied, and those that could not be (Top N, relative date…).
    pub filters: FilterSpecOut,
    /// What the values were computed on (tables loaded, failed, never
    /// loaded).
    pub data: RefreshCounts,
    /// Every measure, in model order; failures carry `error` and `⚠` in
    /// `display`, with their DAX.
    pub measures: Vec<MeasureValueOut>,
}

impl Command for MeasuresValues {
    type Params = MeasuresValuesParams;
    type Output = MeasuresValuesOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "measures.values",
            title: "Measure values",
            summary: "Every measure's formatted value under optional report/page filters; failed measures show ⚠ and their DAX.",
            category: Category::Dax,
            long_running: true,
            examples: &[
                ("Unfiltered", r#"{"docId": "d1"}"#),
                (
                    "Under the Stores report's filters",
                    r#"{"docId": "d1", "report": "Stores"}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = loaded_doc(ws, &p.doc_id, ctx).await?;
        resolve::require_model(&entry)?;
        let (spec, report, page) =
            filter_spec(&entry, p.report.as_deref(), p.page.as_deref(), p.filters)?;
        let (document_revision, values) = entry
            .session()
            .measures_with_revision(&spec, ctx.cancel_token().child_token())
            .await?;
        let measures: Vec<MeasureValueOut> = values.iter().map(MeasureValueOut::from).collect();
        Ok(MeasuresValuesOut {
            document_revision,
            ok: measures.iter().all(|m| m.error.is_none()),
            report,
            page,
            filters: FilterSpecOut::from(&spec),
            data: RefreshCounts::of_session(entry.session()),
            measures,
        })
    }
}

// ---------------------------------------------------------------------------

/// `dax.diagnostics`.
#[derive(Debug)]
pub struct DaxDiagnostics;

/// What a DAX text is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum DaxTextKind {
    /// A query when the text starts with `DEFINE` or `EVALUATE`, else an
    /// expression.
    #[default]
    Auto,
    /// A single expression: a measure, calculated column or table,
    /// calculation item, RLS filter, format string or detail rows.
    Expression,
    /// `[DEFINE …] EVALUATE …`.
    Query,
}

/// How a DAX text was checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum DaxCheckedKind {
    Expression,
    Query,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DaxDiagnosticsParams {
    /// Check names against this document's model and report-level measures
    /// (`d1`, …). Without it only syntax and functions are checked.
    #[serde(default)]
    pub doc_id: Option<String>,
    /// The DAX text.
    pub text: String,
    /// `auto` (default), `expression` or `query`.
    #[serde(default)]
    pub kind: DaxTextKind,
}

/// A position in the checked text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TextPositionOut {
    /// UTF-8 byte offset.
    pub offset: usize,
    /// UTF-16 code-unit offset (JavaScript string index).
    pub utf16_offset: usize,
    /// 1-based line (lines end at `\n`).
    pub line: usize,
    /// 1-based column, in characters.
    pub column: usize,
}

impl TextPositionOut {
    pub(crate) fn at(src: &str, offset: usize) -> Self {
        let (line, column) = dax_service::line_col(src, offset);
        let mut offset = offset.min(src.len());
        while !src.is_char_boundary(offset) {
            offset = offset.saturating_sub(1);
        }
        let utf16_offset = src.get(..offset).map_or(0, |s| s.encode_utf16().count());
        Self {
            offset,
            utf16_offset,
            line,
            column,
        }
    }
}

/// One problem in the text.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DaxDiagnosticOut {
    /// `error` or `warning` (a function DashCraft cannot evaluate yet).
    pub severity: String,
    /// `syntax`, `unknownTable`, `unknownColumn`, `unknownMeasure`,
    /// `argumentCount` or `unsupportedFunction`.
    pub code: String,
    pub message: String,
    /// Start of the text the problem covers.
    pub start: TextPositionOut,
    /// End (exclusive).
    pub end: TextPositionOut,
    /// A "did you mean …" correction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DaxDiagnosticsOut {
    /// The document revision whose model names were used (with `docId`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_revision: Option<u64>,
    /// How the text was checked.
    pub kind: DaxCheckedKind,
    /// No errors (warnings allowed).
    pub ok: bool,
    /// In source order, at most 1000.
    pub diagnostics: Vec<DaxDiagnosticOut>,
}

impl Command for DaxDiagnostics {
    type Params = DaxDiagnosticsParams;
    type Output = DaxDiagnosticsOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "dax.diagnostics",
            title: "Check DAX",
            summary: "Syntax errors, unknown tables, columns and measures (with a did-you-mean suggestion), wrong argument counts and functions DashCraft cannot evaluate, for a DAX expression or query; no data is loaded.",
            category: Category::Dax,
            examples: &[
                (
                    "A measure against the model",
                    r#"{"docId": "d1", "text": "CALCULATE([Total Sales], Stores[Regoin] = \"West\")"}"#,
                ),
                (
                    "Syntax only, without a document",
                    r#"{"text": "EVALUATE ROW(\"n\", 1 +)"}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let (document_revision, schema) = language_schema(ws, p.doc_id.as_deref())?;
        let kind = p.kind.resolve(&p.text);
        let src = p.text.as_str();
        let diagnostics: Vec<DaxDiagnosticOut> = dax_service::diagnostics(src, kind, &schema)
            .into_iter()
            .map(|d| DaxDiagnosticOut::new(src, d))
            .collect();
        Ok(DaxDiagnosticsOut {
            document_revision,
            kind: DaxCheckedKind::of(kind),
            ok: diagnostics.iter().all(|d| d.severity != "error"),
            diagnostics,
        })
    }
}

impl DaxTextKind {
    fn resolve(self, text: &str) -> SourceKind {
        match self {
            DaxTextKind::Auto => SourceKind::detect(text),
            DaxTextKind::Expression => SourceKind::Expression,
            DaxTextKind::Query => SourceKind::Query,
        }
    }
}

impl DaxCheckedKind {
    fn of(kind: SourceKind) -> Self {
        match kind {
            SourceKind::Expression => DaxCheckedKind::Expression,
            SourceKind::Query => DaxCheckedKind::Query,
        }
    }
}

impl DaxDiagnosticOut {
    fn new(src: &str, d: Diagnostic) -> Self {
        Self {
            severity: match d.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
                Severity::Info => "info",
            }
            .to_owned(),
            code: d.code.as_str().to_owned(),
            message: d.message,
            start: TextPositionOut::at(src, d.span.start),
            end: TextPositionOut::at(src, d.span.end),
            suggestion: d.suggestion,
        }
    }
}

/// The names a DAX text can use: the model of `doc_id` (at its current
/// revision) plus the report-level measures (`reportExtensions.json`) of
/// any of its reports; without a document, an unknown schema.
fn language_schema(ws: &Workspace, doc_id: Option<&str>) -> Result<(Option<u64>, Schema), OpError> {
    language_context(ws, doc_id).map(|c| (c.revision, c.schema))
}

/// [`language_schema`] and the model it came from.
struct LanguageContext {
    revision: Option<u64>,
    schema: Schema,
    model: Option<Arc<Model>>,
}

fn language_context(ws: &Workspace, doc_id: Option<&str>) -> Result<LanguageContext, OpError> {
    let Some(id) = doc_id else {
        return Ok(LanguageContext {
            revision: None,
            schema: Schema::unknown(),
            model: None,
        });
    };
    let entry = doc(ws, id)?;
    let capture = entry.session().revision_snapshot();
    let mut schema = Schema::from_model(&capture.model);
    for m in capture.document.reports.iter().flat_map(|r| &r.measures) {
        schema.add_measure_with(
            &m.table,
            &m.name,
            NameDetails {
                data_type: m.data_type.clone(),
                expression: Some(m.expression.clone()),
                format_string: m.format_string.clone(),
                description: m.description.clone(),
                hidden: m.hidden,
            },
        );
    }
    Ok(LanguageContext {
        revision: Some(capture.revision),
        schema,
        model: Some(capture.model),
    })
}

/// A cursor in a DAX text: `offset`, `utf16Offset`, or `line` with
/// `column`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextPositionIn {
    /// UTF-8 byte offset (rounded down to a character boundary).
    #[serde(default)]
    pub offset: Option<usize>,
    /// UTF-16 code-unit offset (JavaScript string index).
    #[serde(default)]
    pub utf16_offset: Option<usize>,
    /// 1-based line (lines end at `\n`); needs `column`.
    #[serde(default)]
    pub line: Option<usize>,
    /// 1-based column in characters; past the end of the line means its
    /// end.
    #[serde(default)]
    pub column: Option<usize>,
}

impl TextPositionIn {
    /// The UTF-8 byte offset in `src`.
    fn resolve(self, src: &str) -> Result<usize, OpError> {
        let bad = |m: &str| Err(OpError::invalid_params(format!("position: {m}")));
        match (self.offset, self.utf16_offset, self.line, self.column) {
            (Some(offset), None, None, None) => {
                if offset > src.len() {
                    return bad(&format!("offset {offset} is past the end ({})", src.len()));
                }
                let mut offset = offset;
                while !src.is_char_boundary(offset) {
                    offset = offset.saturating_sub(1);
                }
                Ok(offset)
            }
            (None, Some(units), None, None) => {
                let mut seen = 0;
                for (i, c) in src.char_indices() {
                    if seen >= units {
                        return Ok(i);
                    }
                    seen += c.len_utf16();
                    if seen > units {
                        // Inside a surrogate pair: the character's start.
                        return Ok(i);
                    }
                }
                if seen == units {
                    Ok(src.len())
                } else {
                    bad(&format!("utf16Offset {units} is past the end ({seen})"))
                }
            }
            (None, None, Some(line), Some(column)) => {
                if line == 0 || column == 0 {
                    return bad("line and column start at 1");
                }
                let mut start = 0;
                for _ in 1..line {
                    match src.get(start..).and_then(|rest| rest.find('\n')) {
                        Some(i) => start += i + 1,
                        None => return bad(&format!("line {line} is past the last line")),
                    }
                }
                let rest = src.get(start..).unwrap_or("");
                let text = rest.split('\n').next().unwrap_or("");
                let within = text
                    .char_indices()
                    .nth(column - 1)
                    .map_or(text.len(), |(i, _)| i);
                Ok(start + within)
            }
            _ => bad("give exactly one of `offset`, `utf16Offset` or `line` with `column`"),
        }
    }
}

// ---------------------------------------------------------------------------

/// `dax.signatureHelp`.
#[derive(Debug)]
pub struct DaxSignatureHelp;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DaxSignatureHelpParams {
    /// The DAX text.
    pub text: String,
    /// The cursor.
    pub position: TextPositionIn,
}

/// One parameter of a signature.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DaxParameterOut {
    pub name: String,
    /// May be left out.
    pub optional: bool,
    /// Part of a group that may repeat.
    pub repeating: bool,
    /// Start of the name in `signature` (the signature is ASCII, so bytes,
    /// characters and UTF-16 units agree).
    pub label_start: usize,
    /// End of the name in `signature` (exclusive).
    pub label_end: usize,
}

/// The function call around the cursor.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DaxSignatureOut {
    /// Upper-case name (`SUM`).
    pub function: String,
    /// `IF(logicalTest, valueIfTrue, [valueIfFalse])`.
    pub signature: String,
    /// The signature with its return type, `SUM(column) → number`.
    pub label: String,
    /// `any`, `boolean`, `currency`, `datetime`, `integer`, `number`,
    /// `table` or `text`.
    pub returns: String,
    /// The function family (`Text`, `Time intelligence`, …).
    pub category: String,
    /// One sentence.
    pub description: String,
    /// Whether DashCraft can evaluate it.
    pub supported: bool,
    pub min_args: usize,
    /// Absent when unlimited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_args: Option<usize>,
    pub parameters: Vec<DaxParameterOut>,
    /// 0-based index of the argument the cursor is in.
    pub argument: usize,
    /// The parameter that argument fills (repeating groups cycle); absent
    /// past the last parameter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_parameter: Option<usize>,
    /// Start of the function name in the text.
    pub name_start: TextPositionOut,
    /// End of the function name (exclusive).
    pub name_end: TextPositionOut,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DaxSignatureHelpOut {
    /// Absent outside a call of a known DAX function (or in a comment).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<DaxSignatureOut>,
}

impl Command for DaxSignatureHelp {
    type Params = DaxSignatureHelpParams;
    type Output = DaxSignatureHelpOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "dax.signatureHelp",
            title: "DAX signature help",
            summary: "The DAX function call around a cursor: its signature, parameters, return type, description and the parameter the cursor is in; works on text that does not parse yet.",
            category: Category::Dax,
            examples: &[(
                "The cursor after the first comma of IF",
                r#"{"text": "IF([Total Sales] > 0, ", "position": {"offset": 22}}"#,
            )],
            ..BASE
        }
    }

    async fn run(_ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let src = p.text.as_str();
        let offset = p.position.resolve(src)?;
        let signature = dax_service::signature_help(src, offset).map(|h| {
            let f = h.function;
            DaxSignatureOut {
                function: f.name.to_owned(),
                signature: f.signature.to_owned(),
                label: f.label(),
                returns: f.returns.as_str().to_owned(),
                category: f.category.to_owned(),
                description: f.description.to_owned(),
                supported: dax_service::is_supported_function(f.name),
                min_args: f.min_args,
                max_args: f.max_args,
                parameters: f
                    .params
                    .iter()
                    .map(|p| DaxParameterOut {
                        name: p.name.to_owned(),
                        optional: p.optional,
                        repeating: p.repeating,
                        label_start: p.label.0,
                        label_end: p.label.1,
                    })
                    .collect(),
                argument: h.argument,
                active_parameter: h.active_parameter,
                name_start: TextPositionOut::at(src, h.name_span.start),
                name_end: TextPositionOut::at(src, h.name_span.end),
            }
        });
        Ok(DaxSignatureHelpOut { signature })
    }
}

// ---------------------------------------------------------------------------

/// `dax.hover`.
#[derive(Debug)]
pub struct DaxHover;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DaxHoverParams {
    /// Describe tables, columns and measures of this document's model and
    /// report-level measures (`d1`, …). Without it only functions,
    /// keywords, variables and the query's own definitions are described.
    #[serde(default)]
    pub doc_id: Option<String>,
    /// The DAX text.
    pub text: String,
    /// The cursor.
    pub position: TextPositionIn,
}

/// What a hover describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum DaxHoverKind {
    Function,
    /// A user-defined function of the model or query.
    UserFunction,
    Table,
    Column,
    Measure,
    Variable,
    Keyword,
}

/// The name under the cursor.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DaxHoverInfo {
    pub kind: DaxHoverKind,
    /// The name as DAX: `SUM`, `Sales`, `'Product Category'[Category]`,
    /// `[Total Sales]`, a variable, `VAR`.
    pub name: String,
    /// The home table of a column or measure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    /// Start of the hovered name in the text.
    pub start: TextPositionOut,
    /// End (exclusive).
    pub end: TextPositionOut,
    /// Everything below as Markdown (code in `dax` fences).
    pub markdown: String,
    /// A function's signature, `SUM(column)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// A function's return type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returns: Option<String>,
    /// A function's or keyword's description, or the model description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// For a function: whether DashCraft can evaluate it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supported: Option<bool>,
    /// A column's data type (`string`, `int64`, `dateTime`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_type: Option<String>,
    /// The DAX of a measure, calculated column or variable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_string: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// A table's column count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column_count: Option<usize>,
    /// A table's measure count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measure_count: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DaxHoverOut {
    /// The document revision whose model was used (with `docId`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_revision: Option<u64>,
    /// Absent when there is nothing to describe at the cursor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hover: Option<DaxHoverInfo>,
}

impl Command for DaxHover {
    type Params = DaxHoverParams;
    type Output = DaxHoverOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "dax.hover",
            title: "DAX hover",
            summary: "Describes the DAX function, table, column, measure, variable or keyword at a cursor: signature and description, data type, expression, format string; no data is loaded.",
            category: Category::Dax,
            examples: &[
                (
                    "A measure of the model",
                    r#"{"docId": "d1", "text": "[Total Sales] * 2", "position": {"offset": 3}}"#,
                ),
                (
                    "A function, without a document",
                    r#"{"text": "SUMX(Sales, 1)", "position": {"line": 1, "column": 2}}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let (document_revision, schema) = language_schema(ws, p.doc_id.as_deref())?;
        let src = p.text.as_str();
        let offset = p.position.resolve(src)?;
        let hover = dax_service::hover(src, offset, &schema).map(|h| {
            let markdown = h.markdown();
            let is_function = h.kind == HoverKind::Function;
            let description = h.description.or(h.details.description);
            DaxHoverInfo {
                kind: match h.kind {
                    HoverKind::Function => DaxHoverKind::Function,
                    HoverKind::UserFunction => DaxHoverKind::UserFunction,
                    HoverKind::Table => DaxHoverKind::Table,
                    HoverKind::Column => DaxHoverKind::Column,
                    HoverKind::Measure => DaxHoverKind::Measure,
                    HoverKind::Variable => DaxHoverKind::Variable,
                    HoverKind::Keyword => DaxHoverKind::Keyword,
                },
                name: h.name,
                table: h.table,
                start: TextPositionOut::at(src, h.span.start),
                end: TextPositionOut::at(src, h.span.end),
                markdown,
                signature: h.signature,
                returns: h.returns.map(str::to_owned),
                description,
                supported: is_function.then_some(h.supported),
                data_type: h.details.data_type,
                expression: h.details.expression,
                format_string: h.details.format_string,
                hidden: h.details.hidden,
                column_count: h.counts.map(|(c, _)| c),
                measure_count: h.counts.map(|(_, m)| m),
            }
        });
        Ok(DaxHoverOut {
            document_revision,
            hover,
        })
    }
}

// ---------------------------------------------------------------------------

/// `dax.complete`.
#[derive(Debug)]
pub struct DaxComplete;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DaxCompleteParams {
    /// Suggest the tables, columns and measures of this document's model and
    /// its report-level measures (`d1`, …). Without it only functions,
    /// keywords, variables and the query's own definitions are suggested.
    #[serde(default)]
    pub doc_id: Option<String>,
    /// The DAX text.
    pub text: String,
    /// The cursor.
    pub position: TextPositionIn,
    /// Most items returned, best first (default 100, at most 1000).
    #[serde(default = "default_max_items")]
    pub max_items: usize,
}

/// What a completion item inserts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum DaxCompletionKind {
    Function,
    /// A user-defined function of the model or query.
    UserFunction,
    Table,
    Column,
    Measure,
    Variable,
    Keyword,
}

/// One suggestion.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DaxCompletionItemOut {
    /// What a list shows: `SUM`, `Sales`, `'Product Category'[Category]`,
    /// `[Total Sales]`, a variable, `ORDER BY`.
    pub label: String,
    pub kind: DaxCompletionKind,
    /// The text that replaces `start`..`end`: `SUM(`, `PI()`, `SUM` when a
    /// `(` already follows, the label for anything else.
    pub insert_text: String,
    /// `SUM(column) → number`, `column (string)`, `measure`, `table`,
    /// `variable`, `parameter` (of a user-defined function), `keyword`,
    /// `user-defined function`.
    pub detail: String,
    /// The home table of a column or measure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    /// A function's or keyword's description, or the model description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DaxCompleteOut {
    /// The document revision whose model was used (with `docId`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_revision: Option<u64>,
    /// Start of the text a chosen item replaces (the name being typed; the
    /// cursor when nothing is typed yet).
    pub start: TextPositionOut,
    /// End (exclusive) of the replaced text.
    pub end: TextPositionOut,
    /// The typed text the items match (`Tot` in `[Tot`).
    pub prefix: String,
    /// Best match first: the same name, then names that start with the
    /// prefix, then a word that does, then names that contain it. Empty
    /// inside a comment, string or number, and after `VAR`.
    pub items: Vec<DaxCompletionItemOut>,
    /// More items matched than `maxItems`.
    pub incomplete: bool,
}

impl Command for DaxComplete {
    type Params = DaxCompleteParams;
    type Output = DaxCompleteOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "dax.complete",
            title: "DAX completion",
            summary: "The DAX functions, tables, columns, measures, variables and keywords that fit at a cursor, best match first, with the text to insert; works on text that does not parse yet.",
            category: Category::Dax,
            examples: &[
                (
                    "A measure of the model, inside brackets",
                    r#"{"docId": "d1", "text": "[Tot", "position": {"offset": 4}}"#,
                ),
                (
                    "Functions, without a document",
                    r#"{"text": "SUMX(Sales, SU", "position": {"line": 1, "column": 15}, "maxItems": 10}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let max_items = bounded("maxItems", p.max_items, MAX_COMPLETION_ITEMS)?;
        if max_items == 0 {
            return Err(OpError::invalid_params("maxItems must be at least 1"));
        }
        let (document_revision, schema) = language_schema(ws, p.doc_id.as_deref())?;
        let src = p.text.as_str();
        let offset = p.position.resolve(src)?;
        let c = dax_service::complete(src, offset, &schema, max_items);
        let items = c
            .items
            .into_iter()
            .map(|i| DaxCompletionItemOut {
                label: i.label,
                kind: match i.kind {
                    CompletionKind::Function => DaxCompletionKind::Function,
                    CompletionKind::UserFunction => DaxCompletionKind::UserFunction,
                    CompletionKind::Table => DaxCompletionKind::Table,
                    CompletionKind::Column => DaxCompletionKind::Column,
                    CompletionKind::Measure => DaxCompletionKind::Measure,
                    CompletionKind::Variable => DaxCompletionKind::Variable,
                    CompletionKind::Keyword => DaxCompletionKind::Keyword,
                },
                insert_text: i.insert_text,
                detail: i.detail,
                table: i.table,
                description: i.description,
            })
            .collect();
        Ok(DaxCompleteOut {
            document_revision,
            start: TextPositionOut::at(src, c.span.start),
            end: TextPositionOut::at(src, c.span.end),
            prefix: c.prefix,
            items,
            incomplete: c.incomplete,
        })
    }
}

// ---------------------------------------------------------------------------

/// Text positions of non-decreasing byte offsets, in one pass over the
/// text (each [`TextPositionOut::at`] rescans from the start).
struct Positions<'a> {
    src: &'a str,
    at: TextPositionOut,
}

impl<'a> Positions<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            at: TextPositionOut {
                offset: 0,
                utf16_offset: 0,
                line: 1,
                column: 1,
            },
        }
    }

    fn at(&mut self, offset: usize) -> TextPositionOut {
        let mut offset = offset.min(self.src.len());
        while !self.src.is_char_boundary(offset) {
            offset = offset.saturating_sub(1);
        }
        let Some(gap) = self.src.get(self.at.offset..offset) else {
            return TextPositionOut::at(self.src, offset);
        };
        for c in gap.chars() {
            self.at.utf16_offset += c.len_utf16();
            if c == '\n' {
                self.at.line += 1;
                self.at.column = 1;
            } else {
                self.at.column += 1;
            }
        }
        self.at.offset = offset;
        self.at
    }
}

/// `dax.tokens`.
#[derive(Debug)]
pub struct DaxTokens;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DaxTokensParams {
    /// Resolve names against this document's model and report-level
    /// measures (`d1`, …). Without it names keep their syntactic kind and
    /// none is unresolved.
    #[serde(default)]
    pub doc_id: Option<String>,
    /// The DAX text.
    pub text: String,
}

/// What a token is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum DaxTokenKind {
    /// `VAR`, `RETURN`, `EVALUATE`, `TRUE`, `DESC`, a parameter type, …
    Keyword,
    Function,
    /// A user-defined function of the model or query.
    UserFunction,
    Table,
    Column,
    Measure,
    Variable,
    /// A parameter of a user-defined function.
    Parameter,
    /// A query parameter, `@name`.
    QueryParameter,
    String,
    /// `dt"2024-01-31"`.
    DateTime,
    Number,
    Comment,
    Operator,
}

/// One token.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DaxTokenOut {
    pub start: TextPositionOut,
    /// End (exclusive).
    pub end: TextPositionOut,
    pub kind: DaxTokenKind,
    /// The text declares the name here: a `VAR`, a function parameter, a
    /// query `DEFINE`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub declaration: bool,
    /// A name the document's model and the text do not have.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unresolved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DaxTokensOut {
    /// The document revision whose model was used (with `docId`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_revision: Option<u64>,
    /// In source order, without whitespace and punctuation.
    pub tokens: Vec<DaxTokenOut>,
}

impl Command for DaxTokens {
    type Params = DaxTokensParams;
    type Output = DaxTokensOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "dax.tokens",
            title: "DAX semantic tokens",
            summary: "What every token of a DAX text is (keyword, function, table, column, measure, variable, parameter, literal, comment) for highlighting, with declarations and unknown names marked.",
            category: Category::Dax,
            examples: &[
                (
                    "Names resolved against the model",
                    r#"{"docId": "d1", "text": "VAR x = [Total Sales] RETURN x * 2"}"#,
                ),
                (
                    "Without a document",
                    r#"{"text": "EVALUATE TOPN(5, Sales) ORDER BY Sales[Amount] DESC"}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let (document_revision, schema) = language_schema(ws, p.doc_id.as_deref())?;
        let src = p.text.as_str();
        let mut positions = Positions::new(src);
        let tokens = dax_service::semantic_tokens(src, &schema)
            .into_iter()
            .map(|t| DaxTokenOut {
                start: positions.at(t.span.start),
                end: positions.at(t.span.end),
                kind: match t.kind {
                    SemanticKind::Keyword => DaxTokenKind::Keyword,
                    SemanticKind::Function => DaxTokenKind::Function,
                    SemanticKind::UserFunction => DaxTokenKind::UserFunction,
                    SemanticKind::Table => DaxTokenKind::Table,
                    SemanticKind::Column => DaxTokenKind::Column,
                    SemanticKind::Measure => DaxTokenKind::Measure,
                    SemanticKind::Variable => DaxTokenKind::Variable,
                    SemanticKind::Parameter => DaxTokenKind::Parameter,
                    SemanticKind::QueryParameter => DaxTokenKind::QueryParameter,
                    SemanticKind::String => DaxTokenKind::String,
                    SemanticKind::DateTime => DaxTokenKind::DateTime,
                    SemanticKind::Number => DaxTokenKind::Number,
                    SemanticKind::Comment => DaxTokenKind::Comment,
                    SemanticKind::Operator => DaxTokenKind::Operator,
                },
                declaration: t.declaration,
                unresolved: t.unresolved,
            })
            .collect();
        Ok(DaxTokensOut {
            document_revision,
            tokens,
        })
    }
}

// ---------------------------------------------------------------------------

/// `dax.references`.
#[derive(Debug)]
pub struct DaxReferences;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DaxReferencesParams {
    /// Resolve names against this document's model and report-level
    /// measures (`d1`, …), and name model objects by `objectId`.
    #[serde(default)]
    pub doc_id: Option<String>,
    /// The DAX text.
    pub text: String,
    /// The cursor.
    pub position: TextPositionIn,
}

/// What a symbol is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum DaxSymbolKind {
    Function,
    /// A user-defined function of the model or query.
    UserFunction,
    Table,
    Column,
    Measure,
    Variable,
    /// A parameter of a user-defined function.
    Parameter,
}

/// The symbol at the cursor.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DaxSymbolOut {
    pub kind: DaxSymbolKind,
    /// The name as DAX, spelled as in the model, else as declared, else as
    /// first written: `SUM`, `Sales`, `'Product Category'[Category]`,
    /// `[Total Sales]`, a variable.
    pub name: String,
    /// The home table of a column or measure, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    /// The model's table, column or measure of that name (with `docId`;
    /// the `objectId` of `model.*` edit commands). A query `DEFINE` of the
    /// same name overrides it within the text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_id: Option<u64>,
}

/// A span of the text.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DaxRangeOut {
    pub start: TextPositionOut,
    /// End (exclusive).
    pub end: TextPositionOut,
}

/// One occurrence of the symbol.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DaxReferenceOut {
    pub start: TextPositionOut,
    /// End (exclusive).
    pub end: TextPositionOut,
    /// The text declares the symbol here.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub declaration: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DaxReferencesOut {
    /// The document revision whose model was used (with `docId`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_revision: Option<u64>,
    /// Absent when the cursor is not on a name (a keyword, literal,
    /// comment or operator).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<DaxSymbolOut>,
    /// Where the text declares the symbol (a `VAR`, a function parameter,
    /// a query `DEFINE`); absent for model objects and DAX functions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<DaxRangeOut>,
    /// Every occurrence in source order, the declaration included.
    pub references: Vec<DaxReferenceOut>,
}

/// The model object a symbol names, matching names case-insensitively.
fn object_id(model: &Model, symbol: &Symbol) -> Option<u64> {
    let table = |name: &str| model.tables.iter().find(|t| names_eq(&t.name, name));
    match symbol {
        Symbol::Table { name } => table(name).map(|t| t.id.get()),
        Symbol::Column {
            table: Some(home),
            name,
        } => table(home)?
            .columns
            .iter()
            .find(|c| names_eq(&c.name, name))
            .map(|c| c.id.get()),
        Symbol::Measure { table: home, name } => match home {
            Some(home) => table(home)?
                .measures
                .iter()
                .find(|m| names_eq(&m.name, name)),
            None => model
                .tables
                .iter()
                .flat_map(|t| &t.measures)
                .find(|m| names_eq(&m.name, name)),
        }
        .map(|m| m.id.get()),
        _ => None,
    }
}

impl Command for DaxReferences {
    type Params = DaxReferencesParams;
    type Output = DaxReferencesOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "dax.references",
            title: "DAX references",
            summary: "The symbol at a cursor in a DAX text, where the text declares it and every place it occurs (go to definition, find references); model objects carry their objectId.",
            category: Category::Dax,
            examples: &[
                (
                    "A measure of the model",
                    r#"{"docId": "d1", "text": "[Total Sales] / CALCULATE([Total Sales], ALL(Stores))", "position": {"offset": 3}}"#,
                ),
                (
                    "A variable, without a document",
                    r#"{"text": "VAR x = 1 RETURN x + x", "position": {"line": 1, "column": 5}}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let LanguageContext {
            revision: document_revision,
            schema,
            model,
        } = language_context(ws, p.doc_id.as_deref())?;
        let src = p.text.as_str();
        let offset = p.position.resolve(src)?;
        let Some(r) = dax_service::references(src, offset, &schema) else {
            return Ok(DaxReferencesOut {
                document_revision,
                symbol: None,
                definition: None,
                references: Vec::new(),
            });
        };
        let mut positions = Positions::new(src);
        let references = r
            .sites
            .iter()
            .map(|site| DaxReferenceOut {
                start: positions.at(site.span.start),
                end: positions.at(site.span.end),
                declaration: site.declaration,
            })
            .collect();
        let definition = r.definition.map(|d| DaxRangeOut {
            start: TextPositionOut::at(src, d.start),
            end: TextPositionOut::at(src, d.end),
        });
        let kind = match &r.symbol {
            Symbol::Function { .. } => DaxSymbolKind::Function,
            Symbol::UserFunction { .. } => DaxSymbolKind::UserFunction,
            Symbol::Table { .. } => DaxSymbolKind::Table,
            Symbol::Column { .. } => DaxSymbolKind::Column,
            Symbol::Measure { .. } => DaxSymbolKind::Measure,
            Symbol::Variable { .. } => DaxSymbolKind::Variable,
            Symbol::Parameter { .. } => DaxSymbolKind::Parameter,
        };
        let symbol = DaxSymbolOut {
            kind,
            name: r.symbol.display(),
            table: r.symbol.table().map(str::to_owned),
            object_id: model.as_deref().and_then(|m| object_id(m, &r.symbol)),
        };
        Ok(DaxReferencesOut {
            document_revision,
            symbol: Some(symbol),
            definition,
            references,
        })
    }
}

// ---------------------------------------------------------------------------

/// `dax.format`.
#[derive(Debug)]
pub struct DaxFormat;

/// Letter case `dax.format` gives keywords or function names.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum DaxCasing {
    /// `CALCULATE`, `VAR`.
    #[default]
    Upper,
    /// `calculate`, `var`.
    Lower,
    /// As written.
    Keep,
}

impl From<DaxCasing> for Casing {
    fn from(c: DaxCasing) -> Self {
        match c {
            DaxCasing::Upper => Casing::Upper,
            DaxCasing::Lower => Casing::Lower,
            DaxCasing::Keep => Casing::Keep,
        }
    }
}

fn default_format_width() -> usize {
    80
}

fn default_indent_width() -> usize {
    4
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DaxFormatParams {
    /// The DAX text.
    pub text: String,
    /// `auto` (default), `expression` or `query`.
    #[serde(default)]
    pub kind: DaxTextKind,
    /// Preferred maximum line length in characters (default 80, at most
    /// 10000). Only a single long token, comment or unbreakable run makes a
    /// line longer.
    #[serde(default = "default_format_width")]
    pub max_width: usize,
    /// Spaces per indentation level (default 4, at most 16), or the width a
    /// tab counts for with `useTabs`.
    #[serde(default = "default_indent_width")]
    pub indent_width: usize,
    /// Indent with tabs instead of spaces.
    #[serde(default)]
    pub use_tabs: bool,
    /// Case of keywords (`VAR`, `RETURN`, `IN`, `TRUE`, `EVALUATE`, `DESC`,
    /// parameter types, …): `upper` (default), `lower` or `keep`.
    #[serde(default)]
    pub keyword_case: DaxCasing,
    /// Case of built-in function names: `upper` (default), `lower` or
    /// `keep`. User-defined functions, tables, columns, measures and
    /// variables always keep their spelling.
    #[serde(default)]
    pub function_case: DaxCasing,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DaxFormatOut {
    /// How the text was parsed.
    pub kind: DaxCheckedKind,
    /// The text parses (else `text` is the input unchanged and `diagnostic`
    /// says why).
    pub ok: bool,
    /// The formatted text: `\n` line breaks, no trailing line break.
    pub text: String,
    /// `text` differs from the input.
    pub changed: bool,
    /// The syntax error when `ok` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<DaxDiagnosticOut>,
}

impl Command for DaxFormat {
    type Params = DaxFormatParams;
    type Output = DaxFormatOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "dax.format",
            title: "Format DAX",
            summary: "Lays out a DAX expression or query within a line width, keeping comments and meaning (only whitespace and keyword/function case change); idempotent, no data is loaded.",
            category: Category::Dax,
            examples: &[
                (
                    "A measure",
                    r#"{"text": "calculate(sum(Sales[Amount]),filter(all(Stores),Stores[Region]=\"West\"))"}"#,
                ),
                (
                    "A query, narrow, lower-case keywords",
                    r#"{"text": "EVALUATE SUMMARIZECOLUMNS(Stores[Region], \"Sales\", [Total Sales]) ORDER BY Stores[Region]", "maxWidth": 40, "keywordCase": "lower"}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(_ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let opts = FormatOptions {
            max_width: bounded("maxWidth", p.max_width, MAX_FORMAT_WIDTH)?,
            indent_width: bounded("indentWidth", p.indent_width, MAX_INDENT_WIDTH)?,
            use_tabs: p.use_tabs,
            keyword_case: p.keyword_case.into(),
            function_case: p.function_case.into(),
        };
        let kind = p.kind.resolve(&p.text);
        match dax_service::format(&p.text, kind, &opts) {
            Ok(text) => Ok(DaxFormatOut {
                kind: DaxCheckedKind::of(kind),
                ok: true,
                changed: text != p.text,
                text,
                diagnostic: None,
            }),
            Err(FormatError::Syntax(d)) => Ok(DaxFormatOut {
                kind: DaxCheckedKind::of(kind),
                ok: false,
                changed: false,
                diagnostic: Some(DaxDiagnosticOut::new(&p.text, d)),
                text: p.text,
            }),
            Err(e @ FormatError::Unstable) => Err(OpError::internal(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Positions, TextPositionOut};

    #[test]
    fn positions_match_a_rescan_at_every_offset() {
        let src = "a\u{e9}\n\u{1f600}x\r\n\u{4e2d}\n";
        let mut forward = Positions::new(src);
        for offset in 0..=src.len() + 1 {
            assert_eq!(
                forward.at(offset),
                TextPositionOut::at(src, offset),
                "{offset}"
            );
        }
        // Going back rescans.
        assert_eq!(forward.at(2), TextPositionOut::at(src, 2));
    }
}
