//! Serializable result and parameter types shared by several commands.
//! Every type derives `JsonSchema`: the catalog publishes their schemas.

use pbi_model::Severity;
use pbi_session::{
    ColumnFilterSpec, ColumnRefresh, DerivedColumnInfo, FilterSpec, MeasureValue, OFFLINE_HINT,
    RefreshKind, RefreshReport, Session, TableRefresh, TableStatus, UnsupportedFilter,
    value_to_json,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::inspect_model::{DiagnosticOut, diagnostic_out};

// ---------------------------------------------------------------------------
// Refresh
// ---------------------------------------------------------------------------

/// Table counts of a refresh (or of the data loaded so far).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RefreshCounts {
    /// Tables in scope.
    pub tables: usize,
    /// Tables whose steps all succeeded.
    pub ok: usize,
    /// Tables with a failed step (the table or a calculated column).
    pub failed: usize,
    /// Tables not attempted (or, for data status, not loaded yet).
    pub skipped: usize,
}

impl RefreshCounts {
    /// Counts of a refresh report.
    #[must_use]
    pub fn of(report: &RefreshReport) -> Self {
        Self::count(report.tables.iter().map(Some))
    }

    /// Counts of the session's last results per table (never-loaded tables
    /// count as skipped).
    #[must_use]
    pub fn of_session(session: &Session) -> Self {
        let tables = session.tables();
        Self::count(tables.iter().map(|t| t.last_refresh.as_ref()))
    }

    fn count<'a>(tables: impl Iterator<Item = Option<&'a TableRefresh>>) -> Self {
        let mut c = Self::default();
        for t in tables {
            c.tables += 1;
            match t {
                Some(t) if has_error(t) => c.failed += 1,
                Some(t) if !t.status.is_skipped() => c.ok += 1,
                _ => c.skipped += 1,
            }
        }
        c
    }
}

/// `true` if the table or one of its calculated columns failed.
#[must_use]
pub fn has_error(t: &TableRefresh) -> bool {
    t.status.is_error() || t.columns.iter().any(|c| c.status.is_error())
}

/// `true` if any step failed or the refresh reported an Error diagnostic.
#[must_use]
pub fn report_has_errors(report: &RefreshReport) -> bool {
    report.tables.iter().any(has_error)
        || report
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
}

/// Outcome of one refresh step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatusOut {
    /// `ok`, `error` or `skipped`.
    pub state: String,
    /// Error message or skip reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Errors: an M or DAX feature DashCraft does not implement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsupported: Option<bool>,
    /// Errors: what to do about it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Errors: the table can be loaded from a CSV instead (`data.loadCsv`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub csv_fallback: Option<bool>,
    /// Errors: blob access is offline (no blob map configured); `true` or
    /// absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offline: Option<bool>,
}

/// The session's offline hint, worded for the command API.
fn op_hint(hint: &str) -> String {
    if hint == OFFLINE_HINT {
        "offline: pass blobMap to doc.open (blob URLs → local files), or load a CSV with data.loadCsv"
            .to_owned()
    } else {
        hint.to_owned()
    }
}

impl StatusOut {
    #[must_use]
    pub fn of(status: &TableStatus) -> Self {
        match status {
            TableStatus::Ok => Self {
                state: "ok".into(),
                message: None,
                unsupported: None,
                hint: None,
                csv_fallback: None,
                offline: None,
            },
            TableStatus::Error {
                message,
                unsupported,
                hint,
                csv_fallback,
            } => Self {
                state: "error".into(),
                message: Some(message.clone()),
                unsupported: Some(*unsupported),
                hint: hint.as_deref().map(op_hint),
                csv_fallback: Some(*csv_fallback),
                offline: (hint.as_deref() == Some(OFFLINE_HINT)).then_some(true),
            },
            TableStatus::Skipped { reason } => Self {
                state: "skipped".into(),
                message: Some(reason.clone()),
                unsupported: None,
                hint: None,
                csv_fallback: None,
                offline: None,
            },
        }
    }
}

/// `m`, `calculated`, `pbix`, `cache`, `manual`, `skipped`.
#[must_use]
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

/// A recomputed calculated column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ColumnRefreshOut {
    pub name: String,
    pub duration_ms: u64,
    pub status: StatusOut,
}

impl ColumnRefreshOut {
    #[must_use]
    pub fn of(c: &ColumnRefresh) -> Self {
        Self {
            name: c.name.clone(),
            duration_ms: c.duration_ms,
            status: StatusOut::of(&c.status),
        }
    }
}

/// One table of a refresh.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TableRefreshOut {
    pub name: String,
    /// `m`, `calculated`, `pbix`, `cache`, `manual` or `skipped`.
    pub kind: String,
    /// Rows after the step (the previous data if it failed).
    pub rows: usize,
    pub duration_ms: u64,
    pub status: StatusOut,
    /// Calculated columns recomputed for this table.
    pub columns: Vec<ColumnRefreshOut>,
}

impl TableRefreshOut {
    #[must_use]
    pub fn of(t: &TableRefresh) -> Self {
        Self {
            name: t.name.clone(),
            kind: kind_name(t.kind).to_owned(),
            rows: t.rows,
            duration_ms: t.duration_ms,
            status: StatusOut::of(&t.status),
            columns: t.columns.iter().map(ColumnRefreshOut::of).collect(),
        }
    }
}

/// Result of `data.refresh` and `data.loadCsv`.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RefreshOut {
    /// Every table and calculated column in scope succeeded.
    pub ok: bool,
    /// The data source used: `auto`, `query` or `cache` (`manual` for CSV
    /// loads).
    pub source: String,
    pub duration_ms: u64,
    pub summary: RefreshCounts,
    /// Tables in scope, in model order.
    pub tables: Vec<TableRefreshOut>,
    /// Session derived columns after the run, with their status.
    pub derived: Vec<DerivedColumnOut>,
    /// Refresh diagnostics (cycles, decoder notes, CSV conversion warnings).
    pub diagnostics: Vec<DiagnosticOut>,
    /// The data generation after the run.
    pub generation: u64,
}

impl RefreshOut {
    #[must_use]
    pub fn of(report: &RefreshReport, source: &str, generation: u64) -> Self {
        Self {
            ok: !report_has_errors(report),
            source: source.to_owned(),
            duration_ms: report.duration_ms,
            summary: RefreshCounts::of(report),
            tables: report.tables.iter().map(TableRefreshOut::of).collect(),
            derived: report.derived.iter().map(DerivedColumnOut::of).collect(),
            diagnostics: report.diagnostics.iter().map(diagnostic_out).collect(),
            generation,
        }
    }
}

/// A session derived column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DerivedColumnOut {
    pub table: String,
    pub name: String,
    /// `people`, `expression` or `array`.
    pub kind: String,
    /// The parsed text column (`people`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_column: Option<String>,
    /// The DAX expression (`expression`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dax: Option<String>,
    /// Model data type of the values (`string`, `int64`, …).
    pub data_type: String,
    /// The column is in the table data now.
    pub active: bool,
    pub status: StatusOut,
}

impl DerivedColumnOut {
    #[must_use]
    pub fn of(d: &DerivedColumnInfo) -> Self {
        Self {
            table: d.table.clone(),
            name: d.name.clone(),
            kind: d.kind.as_str().to_owned(),
            source_column: d.source_column.clone(),
            dax: d.dax.clone(),
            data_type: d.data_type.as_str().to_owned(),
            active: d.active,
            status: StatusOut::of(&d.status),
        }
    }
}

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

/// `table[column] IN values` (or `NOT IN` with `exclude`). Parameter form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColumnFilterParam {
    /// Table name (case-insensitive).
    pub table: String,
    /// Column name (case-insensitive).
    pub column: String,
    /// Values: strings, numbers, booleans or `null` (BLANK); converted to
    /// the column type (`"42"` matches an integer column, ISO text a date).
    pub values: Vec<Value>,
    /// Keep the rows whose value is *not* in `values`.
    #[serde(default)]
    pub exclude: bool,
}

impl From<ColumnFilterParam> for ColumnFilterSpec {
    fn from(p: ColumnFilterParam) -> Self {
        ColumnFilterSpec {
            table: p.table,
            column: p.column,
            values: p.values,
            exclude: p.exclude,
        }
    }
}

/// An applied column filter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ColumnFilterOut {
    pub table: String,
    pub column: String,
    /// Values (`null` = BLANK).
    pub values: Vec<Value>,
    /// `NOT IN` instead of `IN`.
    pub exclude: bool,
}

/// A filter that could not be applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UnsupportedFilterOut {
    pub name: String,
    /// `Categorical`, `Advanced`, `TopN`, `Slicer`, `CrossFilter`, …
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// Why it was not applied.
    pub reason: String,
}

impl From<&UnsupportedFilter> for UnsupportedFilterOut {
    fn from(u: &UnsupportedFilter) -> Self {
        Self {
            name: u.name.clone(),
            kind: u.kind.clone(),
            field: u.field.clone(),
            reason: u.reason.clone(),
        }
    }
}

/// A filter specification: column filters (AND) plus what was not applied.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FilterSpecOut {
    pub filters: Vec<ColumnFilterOut>,
    pub unsupported: Vec<UnsupportedFilterOut>,
}

impl From<&FilterSpec> for FilterSpecOut {
    fn from(spec: &FilterSpec) -> Self {
        Self {
            filters: spec
                .filters
                .iter()
                .map(|f| ColumnFilterOut {
                    table: f.table.clone(),
                    column: f.column.clone(),
                    values: f.values.clone(),
                    exclude: f.exclude,
                })
                .collect(),
            unsupported: spec.unsupported.iter().map(Into::into).collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Measures
// ---------------------------------------------------------------------------

/// The value of one measure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeasureValueOut {
    /// Home table.
    pub table: String,
    pub name: String,
    /// Raw value: number, string, boolean, ISO date/time text, or `null`
    /// (BLANK, or an error).
    pub value: Value,
    /// The value formatted with the format string (`""` for BLANK); absent
    /// on error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formatted: Option<String>,
    /// What to show: the formatted value, or `⚠ <error>`.
    pub display: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_string: Option<String>,
    /// Why the measure could not be evaluated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The error is a DAX feature DashCraft does not implement yet.
    pub unsupported: bool,
    /// The measure's DAX expression.
    pub dax: String,
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_folder: Option<String>,
}

impl From<&MeasureValue> for MeasureValueOut {
    fn from(m: &MeasureValue) -> Self {
        Self {
            table: m.table.clone(),
            name: m.name.clone(),
            value: m.value.as_ref().map_or(Value::Null, value_to_json),
            formatted: m.formatted.clone().filter(|_| m.error.is_none()),
            display: m.display(),
            format_string: m.format_string.clone(),
            error: m.error.clone(),
            unsupported: m.unsupported,
            dax: m.dax.clone(),
            hidden: m.hidden,
            display_folder: m.display_folder.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tabular results
// ---------------------------------------------------------------------------

/// A column of a tabular result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResultColumnOut {
    pub name: String,
    /// Model data type (`string`, `int64`, `double`, `decimal`, `dateTime`,
    /// `boolean`) or the Arrow type for computed columns.
    pub data_type: String,
    /// The model format string, when the column is a model column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_string: Option<String>,
}
