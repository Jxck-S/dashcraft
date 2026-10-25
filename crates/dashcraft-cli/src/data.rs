//! Helpers of the commands that load data (`refresh`, `measures`,
//! `export --table`, `render`): opening a [`Session`] with a blob map (and
//! clock), the data source (`--source`), the Tokio runtime, refreshing,
//! finding a report, and reporting refresh problems.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, anyhow, bail};
use pbi_model::{Model, Severity, names_eq};
use pbi_session::{
    Clock, OFFLINE_HINT, RefreshEvent, RefreshKind, RefreshOptions, RefreshReport, RefreshSource,
    Session, SessionError, SessionOptions, TableRefresh, TableStatus,
};
use serde::Serialize;
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

use crate::common::{Context, ReadArgs, open_error};

/// Where the data comes from (`--source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum SourceArg {
    /// The PBIP local data cache (`.pbi/cache.abf`) when there is one, else
    /// the M queries.
    #[default]
    Auto,
    /// Evaluate the M queries (PBIX files: their embedded data).
    #[value(alias = "query")]
    M,
    /// The PBIP local data cache only (fails without one; PBIX files: their
    /// embedded data).
    Cache,
}

impl From<SourceArg> for RefreshSource {
    fn from(s: SourceArg) -> Self {
        match s {
            SourceArg::Auto => RefreshSource::Auto,
            SourceArg::M => RefreshSource::Query,
            SourceArg::Cache => RefreshSource::Cache,
        }
    }
}

/// Flags shared by the commands that refresh a document.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct DataArgs {
    /// Offline `AzureStorage.Blobs` mapping (`blob-map.json`: blob URLs →
    /// local files, relative to the map). Without it, tables read from Azure
    /// Blob Storage fail with an "offline" hint.
    #[arg(long, value_name = "FILE")]
    pub blob_map: Option<PathBuf>,
    /// Where the data comes from: `auto` (default) loads a PBIP's local data
    /// cache (`.pbi/cache.abf`, Power BI Desktop's last refresh; no M, no
    /// network) when there is one, else evaluates the M queries; `m`
    /// always evaluates the M queries; `cache` requires the cache.
    #[arg(long, value_enum, value_name = "SOURCE")]
    pub source: Option<SourceArg>,
    #[command(flatten)]
    pub read: ReadArgs,
}

impl DataArgs {
    /// The refresh source (`auto` unless `--source` says otherwise).
    pub fn refresh_source(&self) -> RefreshSource {
        self.source.unwrap_or_default().into()
    }
}

/// Opens `path` as a session (no data loaded yet).
pub fn open_session(path: &Path, args: &DataArgs) -> anyhow::Result<Session> {
    open_session_with_clock(path, args, None)
}

/// [`open_session`] with a clock for M's `DateTime.LocalNow()` &
/// co. (`None`: the system clock).
pub fn open_session_with_clock(
    path: &Path,
    args: &DataArgs,
    clock: Option<Arc<dyn Clock>>,
) -> anyhow::Result<Session> {
    tracing::info!(path = %path.display(), "opening");
    let options = SessionOptions {
        open: args.read.open_options(),
        blob_map: args.blob_map.clone(),
        clock,
        ..SessionOptions::default()
    };
    Session::open(path, options).map_err(|e| match e {
        SessionError::Open(core) => open_error(path, core),
        other => anyhow!(other),
    })
}

/// Index of the report named `name` in the document: exact
/// (case-insensitive) match, else the only report whose name contains it.
pub fn find_report(session: &Session, name: &str) -> anyhow::Result<usize> {
    let reports = &session.document().reports;
    let names = || {
        reports
            .iter()
            .map(|r| format!("'{}'", r.name))
            .collect::<Vec<_>>()
            .join(", ")
    };
    if reports.is_empty() {
        bail!("--report: the document has no reports");
    }
    if let Some(i) = reports.iter().position(|r| names_eq(&r.name, name)) {
        return Ok(i);
    }
    let needle = name.to_lowercase();
    let mut partial = reports
        .iter()
        .enumerate()
        .filter(|(_, r)| r.name.to_lowercase().contains(&needle));
    match (partial.next(), partial.next()) {
        (Some((i, _)), None) => Ok(i),
        (Some(_), Some(_)) => bail!("'{name}' matches several reports: {}", names()),
        _ => bail!("no report named '{name}'; reports: {}", names()),
    }
}

/// Fails unless the session has a local semantic model.
pub fn require_model(session: &Session, path: &Path) -> anyhow::Result<()> {
    if session.has_model() {
        Ok(())
    } else {
        bail!(
            "{} has no local semantic model (a report bound to a remote model?)",
            path.display()
        )
    }
}

/// A multi-threaded runtime for the session's async API.
pub fn runtime() -> anyhow::Result<Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("dashcraft-worker")
        .build()
        .context("cannot start the async runtime")
}

/// `'A', 'B', …`: the model's table names, for error messages.
pub fn table_list(model: &Model) -> String {
    model
        .tables
        .iter()
        .map(|t| format!("'{}'", t.name))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The model spelling of `name` (case-insensitive), or an error listing the
/// tables.
pub fn resolve_table(model: &Model, name: &str) -> anyhow::Result<String> {
    model
        .table(name)
        .map(|t| t.name.clone())
        .ok_or_else(|| anyhow!("no table named '{name}'; tables: {}", table_list(model)))
}

/// Refreshes `tables` (every table when `None`) from `args.source`;
/// progress is logged with `-v`. Per-table failures are in the report, not
/// errors.
pub fn refresh(
    rt: &Runtime,
    session: &Session,
    tables: Option<Vec<String>>,
    args: &DataArgs,
    ctx: &Context,
) -> anyhow::Result<RefreshReport> {
    let source = args.refresh_source();
    if source != RefreshSource::Query
        && !session.is_pbix()
        && let Some(cache) = session.cache_info()
    {
        let when = cache
            .modified
            .map(|m| format!(" (as of {})", m.format("%Y-%m-%d %H:%M UTC")))
            .unwrap_or_default();
        ctx.note(format!(
            "using the local data cache {}{when}; pass --source m to evaluate the M queries instead",
            cache.entry
        ));
    }
    let options = RefreshOptions {
        tables,
        cancel: CancellationToken::new(),
        source,
        ..RefreshOptions::default()
    }
    .with_progress(log_event);
    rt.block_on(session.refresh(options))
        .map_err(|e| anyhow!(e).context("refresh failed"))
}

fn log_event(event: RefreshEvent) {
    match event {
        RefreshEvent::Started { tables } => {
            tracing::info!("refreshing {} table(s)", tables.len());
        }
        RefreshEvent::TableStarted { name, kind } => {
            tracing::info!("{name}: loading ({})", kind_name(kind));
        }
        RefreshEvent::TableFinished { table } => {
            tracing::info!(
                "{}: {} rows in {} ms ({})",
                table.name,
                table.rows,
                table.duration_ms,
                state_name(&table.status)
            );
        }
        RefreshEvent::ColumnStarted { table, column } => {
            tracing::debug!("{table}[{column}]: computing");
        }
        RefreshEvent::ColumnFinished { table, column } => {
            tracing::debug!(
                "{table}[{}]: {} ms ({})",
                column.name,
                column.duration_ms,
                state_name(&column.status)
            );
        }
        RefreshEvent::Finished { duration_ms, .. } => {
            tracing::info!("refresh finished in {duration_ms} ms");
        }
    }
}

/// Short name of a refresh kind: `m`, `calculated`, `pbix`, `cache`,
/// `manual`, `skipped`.
pub fn kind_name(kind: RefreshKind) -> &'static str {
    match kind {
        RefreshKind::M => "m",
        RefreshKind::Calculated => "calculated",
        RefreshKind::Pbix => "pbix",
        RefreshKind::Cache => "cache",
        RefreshKind::Manual => "manual",
        RefreshKind::Skipped => "skipped",
    }
}

/// `ok`, `error` or `skipped`.
pub fn state_name(status: &TableStatus) -> &'static str {
    match status {
        TableStatus::Ok => "ok",
        TableStatus::Error { .. } => "error",
        TableStatus::Skipped { .. } => "skipped",
    }
}

/// A refresh hint worded for the command line (the session's offline hint
/// names the app's "Load CSV manually…" action).
pub fn cli_hint(hint: &str) -> String {
    if hint == OFFLINE_HINT {
        "offline: pass --blob-map FILE (blob URLs → local files), or load a CSV in the app"
            .to_owned()
    } else {
        hint.to_owned()
    }
}

/// Table counts of a refresh.
#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshCounts {
    /// Tables in scope.
    pub tables: usize,
    /// Tables whose steps all succeeded.
    pub ok: usize,
    /// Tables with a failed step (the table or a calculated column).
    pub failed: usize,
    /// Tables not attempted.
    pub skipped: usize,
}

impl RefreshCounts {
    pub fn of(report: &RefreshReport) -> Self {
        let mut c = RefreshCounts {
            tables: report.tables.len(),
            ..RefreshCounts::default()
        };
        for t in &report.tables {
            if has_error(t) {
                c.failed += 1;
            } else if t.status.is_skipped() {
                c.skipped += 1;
            } else {
                c.ok += 1;
            }
        }
        c
    }
}

/// `true` if the table or one of its calculated columns failed.
pub fn has_error(t: &TableRefresh) -> bool {
    t.status.is_error() || t.columns.iter().any(|c| c.status.is_error())
}

/// `true` if any step failed or the refresh reported an Error diagnostic.
pub fn report_has_errors(report: &RefreshReport) -> bool {
    report.tables.iter().any(has_error)
        || report
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
}

/// Prints the refresh's diagnostics on stderr.
pub fn print_diagnostics(ctx: &Context, report: &RefreshReport) {
    for d in &report.diagnostics {
        let origin = d
            .origin
            .as_ref()
            .map(|o| format!(" ({o})"))
            .unwrap_or_default();
        match d.severity {
            Severity::Error => ctx.note(format!("error: {} {}{origin}", d.code, d.message)),
            Severity::Warning => ctx.warn(format!("{} {}{origin}", d.code, d.message)),
            Severity::Info => {}
        }
    }
}

/// Prints failed and skipped steps (with hints) and the diagnostics on
/// stderr, for commands whose stdout is something else. Returns
/// [`report_has_errors`].
pub fn print_problems(ctx: &Context, report: &RefreshReport) -> bool {
    for t in &report.tables {
        match &t.status {
            TableStatus::Ok => {}
            TableStatus::Error { message, hint, .. } => {
                let hint = hint
                    .as_deref()
                    .map(|h| format!(" (hint: {})", cli_hint(h)))
                    .unwrap_or_default();
                ctx.warn(format!(
                    "table '{}' failed to refresh: {message}{hint}",
                    t.name
                ));
            }
            TableStatus::Skipped { reason } => {
                ctx.warn(format!("table '{}' was not refreshed: {reason}", t.name));
            }
        }
        for c in &t.columns {
            if let Some(message) = c.status.message() {
                ctx.warn(format!(
                    "calculated column '{}'[{}] {}: {message}",
                    t.name,
                    c.name,
                    if c.status.is_error() {
                        "failed"
                    } else {
                        "was skipped"
                    }
                ));
            }
        }
    }
    print_diagnostics(ctx, report);
    report_has_errors(report)
}
