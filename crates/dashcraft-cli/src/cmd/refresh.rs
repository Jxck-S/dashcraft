//! `dashcraft refresh`: loads a document's data headlessly (M partitions,
//! embedded PBIX data, calculated columns and tables) and prints the result
//! per table.

use std::io::{self, Write};
use std::path::PathBuf;

use pbi_session::{ColumnRefresh, RefreshReport, TableRefresh, TableStatus};
use serde::Serialize;

use crate::common::{Context, Status, one_line, print_json, with_stdout};
use crate::data::{
    DataArgs, RefreshCounts, cli_hint, kind_name, open_session, print_diagnostics, refresh,
    report_has_errors, require_model, resolve_table, runtime, state_name,
};
use crate::inspect_model::{DiagnosticOut, diagnostic_out};

/// Version of the `refresh --json` schema.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, clap::Args)]
pub struct RefreshArgs {
    /// File or folder to refresh: .pbip, .pbix, .pbit, *.SemanticModel,
    /// model.bim, a TMDL folder, …
    pub path: PathBuf,
    /// Refresh only these tables (comma-separated or repeated; what depends
    /// on them is recomputed too).
    #[arg(long, value_name = "TABLES", value_delimiter = ',')]
    pub tables: Vec<String>,
    /// Print JSON (`schemaVersion` 1) instead of a table.
    #[arg(long)]
    pub json: bool,
    /// Pretty-print the JSON (implies --json).
    #[arg(long)]
    pub pretty: bool,
    #[command(flatten)]
    pub data: DataArgs,
}

/// Root of `refresh --json`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshOut {
    pub schema_version: u32,
    /// The path as given on the command line.
    pub source: String,
    /// Every table and calculated column in scope succeeded.
    pub ok: bool,
    pub duration_ms: u64,
    pub summary: RefreshCounts,
    /// Tables in scope, in model order.
    pub tables: Vec<TableOut>,
    /// Refresh diagnostics (cycles, decoder notes, relationship problems).
    pub diagnostics: Vec<DiagnosticOut>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableOut {
    pub name: String,
    /// `m`, `calculated`, `pbix`, `cache`, `manual` or `skipped`.
    pub kind: &'static str,
    /// Rows after the refresh (the previous data if the table failed).
    pub rows: usize,
    pub duration_ms: u64,
    pub status: StatusOut,
    /// Calculated columns recomputed for this table.
    pub columns: Vec<ColumnOut>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnOut {
    pub name: String,
    pub duration_ms: u64,
    pub status: StatusOut,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusOut {
    /// `ok`, `error` or `skipped`.
    pub state: &'static str,
    /// Error message or skip reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Errors: an M or DAX feature DashCraft does not implement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unsupported: Option<bool>,
    /// Errors: what to do about it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Errors: the table can be loaded from a CSV instead (in the app).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csv_fallback: Option<bool>,
}

pub fn run(args: &RefreshArgs, ctx: &Context) -> anyhow::Result<Status> {
    let session = open_session(&args.path, &args.data)?;
    require_model(&session, &args.path)?;
    let model = session.model();
    let tables = if args.tables.is_empty() {
        None
    } else {
        Some(
            args.tables
                .iter()
                .map(|t| resolve_table(&model, t.trim()))
                .collect::<anyhow::Result<Vec<_>>>()?,
        )
    };
    let rt = runtime()?;
    let report = refresh(&rt, &session, tables, &args.data, ctx)?;
    let shown = args.path.to_string_lossy();
    if args.json || args.pretty {
        print_json(&refresh_out(&shown, &report), args.pretty)?;
    } else {
        with_stdout(|w| write_report(w, &report))?;
    }
    print_diagnostics(ctx, &report);
    Ok(if report_has_errors(&report) {
        Status::ErrorDiagnostics
    } else {
        Status::Ok
    })
}

pub fn refresh_out(source: &str, report: &RefreshReport) -> RefreshOut {
    RefreshOut {
        schema_version: SCHEMA_VERSION,
        source: source.to_owned(),
        ok: !report_has_errors(report),
        duration_ms: report.duration_ms,
        summary: RefreshCounts::of(report),
        tables: report
            .tables
            .iter()
            .map(|t| TableOut {
                name: t.name.clone(),
                kind: kind_name(t.kind),
                rows: t.rows,
                duration_ms: t.duration_ms,
                status: status_out(&t.status),
                columns: t
                    .columns
                    .iter()
                    .map(|c| ColumnOut {
                        name: c.name.clone(),
                        duration_ms: c.duration_ms,
                        status: status_out(&c.status),
                    })
                    .collect(),
            })
            .collect(),
        diagnostics: report.diagnostics.iter().map(diagnostic_out).collect(),
    }
}

fn status_out(status: &TableStatus) -> StatusOut {
    let mut out = StatusOut {
        state: state_name(status),
        message: status.message().map(str::to_owned),
        unsupported: None,
        hint: None,
        csv_fallback: None,
    };
    if let TableStatus::Error {
        unsupported,
        hint,
        csv_fallback,
        ..
    } = status
    {
        out.unsupported = Some(*unsupported);
        out.hint.clone_from(hint);
        out.csv_fallback = Some(*csv_fallback);
    }
    out
}

/// Longest message shown on one line of the table.
const MESSAGE_WIDTH: usize = 160;

fn write_report(w: &mut dyn Write, report: &RefreshReport) -> io::Result<()> {
    let width = report
        .tables
        .iter()
        .flat_map(|t| {
            std::iter::once(t.name.chars().count())
                .chain(t.columns.iter().map(|c| c.name.chars().count() + 2))
        })
        .max()
        .unwrap_or(5)
        .clamp(5, 40);
    writeln!(
        w,
        "{:<width$}  {:<10}  {:>10}  {:>9}  STATUS",
        "TABLE", "SOURCE", "ROWS", "TIME"
    )?;
    for t in &report.tables {
        write_table(w, t, width)?;
    }
    let c = RefreshCounts::of(report);
    writeln!(w)?;
    writeln!(
        w,
        "{} table{} in {} ms: {} ok, {} failed, {} skipped",
        c.tables,
        if c.tables == 1 { "" } else { "s" },
        report.duration_ms,
        c.ok,
        c.failed,
        c.skipped
    )
}

fn write_table(w: &mut dyn Write, t: &TableRefresh, width: usize) -> io::Result<()> {
    writeln!(
        w,
        "{:<width$}  {:<10}  {:>10}  {:>9}  {}",
        t.name,
        kind_name(t.kind),
        t.rows,
        format!("{} ms", t.duration_ms),
        status_text(&t.status)
    )?;
    write_hint(w, &t.status)?;
    for c in &t.columns {
        write_column(w, c, width)?;
    }
    Ok(())
}

fn write_column(w: &mut dyn Write, c: &ColumnRefresh, width: usize) -> io::Result<()> {
    writeln!(
        w,
        "{:<width$}  {:<10}  {:>10}  {:>9}  {}",
        format!("  {}", c.name),
        "column",
        "",
        format!("{} ms", c.duration_ms),
        status_text(&c.status)
    )?;
    write_hint(w, &c.status)
}

fn status_text(status: &TableStatus) -> String {
    match status {
        TableStatus::Ok => "ok".to_owned(),
        TableStatus::Error {
            message,
            unsupported,
            ..
        } => format!(
            "ERROR{}: {}",
            if *unsupported { " (unsupported)" } else { "" },
            one_line(message, MESSAGE_WIDTH)
        ),
        TableStatus::Skipped { reason } => {
            format!("skipped: {}", one_line(reason, MESSAGE_WIDTH))
        }
    }
}

fn write_hint(w: &mut dyn Write, status: &TableStatus) -> io::Result<()> {
    if let TableStatus::Error {
        hint, csv_fallback, ..
    } = status
    {
        if let Some(hint) = hint {
            writeln!(w, "    hint: {}", cli_hint(hint))?;
        }
        if *csv_fallback && hint.as_deref().is_none_or(|h| !h.contains("CSV")) {
            writeln!(
                w,
                "    hint: the table can be loaded from a CSV file in the app"
            )?;
        }
    }
    Ok(())
}
