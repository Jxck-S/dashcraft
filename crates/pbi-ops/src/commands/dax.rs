//! `dax.query`, `dax.evaluateMeasure`, `measures.values`.

use std::time::Instant;

use pbi_session::{FilterSpec, filters_from_page, filters_from_report};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::data::{CellFormat, result_columns, rows};
use super::{BASE, bounded, loaded_doc, millis};
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
        spec.extend(filters_from_report(r));
        report_name = Some(r.name.clone());
        if page.is_some() {
            let pi = resolve::page_index(r, page)?;
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
        let batches = session
            .dax_query(&p.query, &spec, ctx.cancel_token().child_token())
            .await?;
        let model = session.model();
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
        let value = entry
            .session()
            .evaluate_measure(&p.measure, &spec, ctx.cancel_token().child_token())
            .await?;
        Ok(EvaluateMeasureOut {
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
        let values = entry
            .session()
            .measures(&spec, ctx.cancel_token().child_token())
            .await?;
        let measures: Vec<MeasureValueOut> = values.iter().map(MeasureValueOut::from).collect();
        Ok(MeasuresValuesOut {
            ok: measures.iter().all(|m| m.error.is_none()),
            report,
            page,
            filters: FilterSpecOut::from(&spec),
            data: RefreshCounts::of_session(entry.session()),
            measures,
        })
    }
}
