//! `dashcraft measures`: refreshes a document and prints every measure's
//! value, formatted with its format string, optionally under a report's
//! filters.

use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::anyhow;
use pbi_model::dax_column_ref;
use pbi_session::{ColumnFilterSpec, FilterSpec, MeasureValue, filters_from_report, value_to_json};
use serde::Serialize;
use serde_json::Value as Json;
use tokio_util::sync::CancellationToken;

use crate::common::{Context, Status, one_line, print_json, with_stdout};
use crate::data::{
    DataArgs, RefreshCounts, find_report, open_session, print_problems, refresh, require_model,
    runtime,
};

/// Version of the `measures --json` schema.
pub const SCHEMA_VERSION: u32 = 1;

/// Longest DAX snippet shown under a failed measure.
const DAX_WIDTH: usize = 160;

#[derive(Debug, clap::Args)]
pub struct MeasuresArgs {
    /// File or folder: .pbip, .pbix, .pbit, *.SemanticModel, model.bim, …
    pub path: PathBuf,
    /// Evaluate under the report-level filters of this report (name as in
    /// `inspect`, case-insensitive).
    #[arg(long, value_name = "NAME")]
    pub report: Option<String>,
    /// Print JSON (`schemaVersion` 1) instead of a list.
    #[arg(long)]
    pub json: bool,
    /// Pretty-print the JSON (implies --json).
    #[arg(long)]
    pub pretty: bool,
    #[command(flatten)]
    pub data: DataArgs,
}

/// Root of `measures --json`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasuresOut {
    pub schema_version: u32,
    /// The path as given on the command line.
    pub source: String,
    /// The report whose filters were applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<String>,
    /// Filters applied (combined with AND).
    pub filters: Vec<FilterOut>,
    /// Report filters that could not be applied.
    pub unsupported_filters: Vec<UnsupportedFilterOut>,
    /// Refresh outcome (the values are computed on what loaded).
    pub refresh: RefreshCounts,
    /// Every measure, in model order.
    pub measures: Vec<MeasureOut>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterOut {
    pub table: String,
    pub column: String,
    /// Selected values (`null` = BLANK).
    pub values: Vec<Json>,
    /// `NOT IN` instead of `IN`.
    pub exclude: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnsupportedFilterOut {
    pub name: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    pub reason: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureOut {
    /// Home table.
    pub table: String,
    pub name: String,
    /// Raw value: number, string, boolean, ISO date/time text, or `null`
    /// (BLANK, or an error).
    pub value: Json,
    /// The value formatted with the format string (`""` for BLANK); omitted
    /// on error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formatted: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_string: Option<String>,
    /// Why the measure could not be evaluated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The error is a DAX feature DashCraft does not implement yet.
    pub unsupported: bool,
    /// The measure's DAX expression.
    pub dax: String,
    pub hidden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_folder: Option<String>,
}

pub fn run(args: &MeasuresArgs, ctx: &Context) -> anyhow::Result<Status> {
    let session = open_session(&args.path, &args.data)?;
    require_model(&session, &args.path)?;
    let (report_name, filter) = match &args.report {
        Some(name) => {
            let report = &session.document().reports[find_report(&session, name)?];
            (Some(report.name.clone()), filters_from_report(report))
        }
        None => (None, FilterSpec::new()),
    };
    for u in &filter.unsupported {
        ctx.warn(format!(
            "report filter '{}' ({}{}) is not applied: {}",
            u.name,
            u.kind,
            u.field
                .as_deref()
                .map(|f| format!(" on {f}"))
                .unwrap_or_default(),
            u.reason
        ));
    }

    let rt = runtime()?;
    let report = refresh(&rt, &session, None, &args.data, ctx)?;
    let refresh_failed = print_problems(ctx, &report);
    let measures = rt
        .block_on(session.measures(&filter, CancellationToken::new()))
        .map_err(|e| anyhow!(e).context("cannot evaluate the measures"))?;
    if measures.is_empty() {
        ctx.note("the model has no measures");
    }

    if args.json || args.pretty {
        let out = MeasuresOut {
            schema_version: SCHEMA_VERSION,
            source: args.path.to_string_lossy().into_owned(),
            report: report_name,
            filters: filter
                .filters
                .iter()
                .map(|f| FilterOut {
                    table: f.table.clone(),
                    column: f.column.clone(),
                    values: f.values.clone(),
                    exclude: f.exclude,
                })
                .collect(),
            unsupported_filters: filter
                .unsupported
                .iter()
                .map(|u| UnsupportedFilterOut {
                    name: u.name.clone(),
                    kind: u.kind.clone(),
                    field: u.field.clone(),
                    reason: u.reason.clone(),
                })
                .collect(),
            refresh: RefreshCounts::of(&report),
            measures: measures.iter().map(measure_out).collect(),
        };
        print_json(&out, args.pretty)?;
    } else {
        with_stdout(|w| write_list(w, report_name.as_deref(), &filter, &measures))?;
    }

    let measure_failed = measures.iter().any(MeasureValue::is_error);
    Ok(if refresh_failed || measure_failed {
        Status::ErrorDiagnostics
    } else {
        Status::Ok
    })
}

fn measure_out(m: &MeasureValue) -> MeasureOut {
    MeasureOut {
        table: m.table.clone(),
        name: m.name.clone(),
        value: m.value.as_ref().map_or(Json::Null, value_to_json),
        formatted: m.formatted.clone().filter(|_| m.error.is_none()),
        format_string: m.format_string.clone(),
        error: m.error.clone(),
        unsupported: m.unsupported,
        dax: m.dax.clone(),
        hidden: m.hidden,
        display_folder: m.display_folder.clone(),
    }
}

/// `Stores[Channel] IN ("Retail", "Outlet")`.
pub fn filter_text(f: &ColumnFilterSpec) -> String {
    let values: Vec<String> = f
        .values
        .iter()
        .map(|v| match v {
            Json::Null => "BLANK()".to_owned(),
            other => other.to_string(),
        })
        .collect();
    format!(
        "{} {}IN ({})",
        dax_column_ref(&f.table, &f.column),
        if f.exclude { "NOT " } else { "" },
        values.join(", ")
    )
}

fn write_list(
    w: &mut dyn Write,
    report: Option<&str>,
    filter: &FilterSpec,
    measures: &[MeasureValue],
) -> io::Result<()> {
    if let Some(report) = report {
        writeln!(w, "Report: {report}")?;
    }
    for f in &filter.filters {
        writeln!(w, "Filter: {}", filter_text(f))?;
    }
    if report.is_some() || !filter.filters.is_empty() {
        writeln!(w)?;
    }
    let width = measures
        .iter()
        .map(|m| m.name.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(10, 48);
    let mut table: Option<&str> = None;
    for m in measures {
        if table != Some(m.table.as_str()) {
            if table.is_some() {
                writeln!(w)?;
            }
            writeln!(w, "{}", m.table)?;
            table = Some(&m.table);
        }
        let value = match (&m.error, m.formatted.as_deref()) {
            (Some(e), _) => format!("⚠ {}", one_line(e, DAX_WIDTH)),
            (None, Some("") | None) => "(blank)".to_owned(),
            (None, Some(f)) => f.to_owned(),
        };
        writeln!(w, "  {:<width$}  {value}", m.name)?;
        if m.is_error() {
            writeln!(
                w,
                "  {:<width$}    DAX: {}",
                "",
                one_line(&m.dax, DAX_WIDTH)
            )?;
        }
    }
    Ok(())
}
