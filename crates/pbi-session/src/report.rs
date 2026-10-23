//! Refresh results and progress events.

use pbi_model::Diagnostic;
use serde::{Deserialize, Serialize};

use crate::derived::DerivedColumnInfo;

/// Hint shown when a table could not be loaded because blob access is
/// offline (no blob map).
pub const OFFLINE_HINT: &str = "offline: configure a blob map or Load CSV manually…";

/// Hint shown when an M partition uses something DashCraft does not evaluate.
pub const LOAD_MANUALLY_HINT: &str = pbi_m::merror::LOAD_MANUALLY_HINT;

/// Hint shown when a DAX calculation uses something DashCraft does not
/// implement.
pub(crate) const DAX_UNSUPPORTED_HINT: &str =
    "This DAX feature is not implemented in DashCraft yet.";

/// How a table's data was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RefreshKind {
    /// Evaluated M partition(s) (PBIP, PBIT, `.bim`, TMDL).
    M,
    /// Calculated table (DAX), or a table whose data was kept and only its
    /// calculated columns were recomputed.
    Calculated,
    /// Embedded VertiPaq data (PBIX, ABF).
    Pbix,
    /// The local data cache of a PBIP (`.pbi/cache.abf`, written by Power
    /// BI Desktop's last refresh), see
    /// [`Session::load_cached_data`](crate::Session::load_cached_data).
    Cache,
    /// Loaded manually from a CSV ([`Session::load_csv`](crate::Session::load_csv)).
    Manual,
    /// Not refreshed (unsupported source, or a dependency failed); see the
    /// status reason.
    Skipped,
}

/// Outcome of one refresh step (a table or a calculated column).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TableStatus {
    /// Loaded / computed.
    Ok,
    /// Failed; the previous data (if any) is kept.
    Error {
        /// User-facing message (M or DAX error text).
        message: String,
        /// The source uses an M or DAX feature DashCraft does not implement.
        unsupported: bool,
        /// What the user can do about it.
        #[serde(skip_serializing_if = "Option::is_none")]
        hint: Option<String>,
        /// The table can be loaded with "Load CSV manually…"
        /// ([`Session::load_csv`](crate::Session::load_csv)).
        csv_fallback: bool,
    },
    /// Not attempted.
    Skipped {
        /// Why (e.g. "depends on 'Orders', which failed to refresh").
        reason: String,
    },
}

impl TableStatus {
    /// `true` for [`TableStatus::Ok`].
    #[must_use]
    pub fn is_ok(&self) -> bool {
        matches!(self, TableStatus::Ok)
    }

    /// `true` for [`TableStatus::Error`].
    #[must_use]
    pub fn is_error(&self) -> bool {
        matches!(self, TableStatus::Error { .. })
    }

    /// `true` for [`TableStatus::Skipped`].
    #[must_use]
    pub fn is_skipped(&self) -> bool {
        matches!(self, TableStatus::Skipped { .. })
    }

    /// Error message or skip reason.
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        match self {
            TableStatus::Ok => None,
            TableStatus::Error { message, .. } => Some(message),
            TableStatus::Skipped { reason } => Some(reason),
        }
    }
}

/// Result of computing one calculated column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnRefresh {
    /// Column name.
    pub name: String,
    /// Time spent, in milliseconds.
    pub duration_ms: u64,
    /// Outcome.
    pub status: TableStatus,
}

/// Result of refreshing one table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableRefresh {
    /// Table name (model spelling).
    pub name: String,
    /// How the data was produced.
    pub kind: RefreshKind,
    /// Rows in the session after the refresh (previous data if the step
    /// failed).
    pub rows: usize,
    /// Time spent loading or computing the table itself (calculated columns
    /// are timed separately), in milliseconds.
    pub duration_ms: u64,
    /// Outcome of loading the table.
    pub status: TableStatus,
    /// Calculated columns recomputed for this table, in computation order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<ColumnRefresh>,
}

impl TableRefresh {
    /// `true` if the table and all its recomputed columns are OK.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.status.is_ok() && self.columns.iter().all(|c| c.status.is_ok())
    }

    /// A recomputed calculated column by name (case-insensitive).
    #[must_use]
    pub fn column(&self, name: &str) -> Option<&ColumnRefresh> {
        self.columns
            .iter()
            .find(|c| pbi_model::names_eq(&c.name, name))
    }
}

/// Progress of a refresh, in order. Table events of M partitions may
/// interleave (they are evaluated concurrently).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "event",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RefreshEvent {
    /// The refresh started; `tables` will be refreshed.
    Started {
        /// Table names in scope.
        tables: Vec<String>,
    },
    /// A table's data step started.
    TableStarted {
        /// Table name.
        name: String,
        /// Source kind.
        kind: RefreshKind,
    },
    /// A table's data step finished (`columns` is empty here; calculated
    /// columns are reported by [`RefreshEvent::ColumnFinished`]).
    TableFinished {
        /// The result.
        table: TableRefresh,
    },
    /// A calculated column started.
    ColumnStarted {
        /// Table name.
        table: String,
        /// Column name.
        column: String,
    },
    /// A calculated column finished.
    ColumnFinished {
        /// Table name.
        table: String,
        /// The result.
        column: ColumnRefresh,
    },
    /// The refresh finished (not sent when cancelled).
    Finished {
        /// Total time, in milliseconds.
        duration_ms: u64,
        /// Tables whose steps all succeeded.
        ok: usize,
        /// Tables with a failed step.
        failed: usize,
        /// Tables skipped.
        skipped: usize,
    },
}

/// Result of [`Session::refresh`](crate::Session::refresh),
/// [`Session::load_cached_data`](crate::Session::load_cached_data) or
/// [`Session::load_csv`](crate::Session::load_csv).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshReport {
    /// One entry per table in scope, in model order.
    pub tables: Vec<TableRefresh>,
    /// Warnings and errors that are not tied to one step result (decoder
    /// notes, CSV conversion warnings, relationship problems, cycles).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
    /// Total time, in milliseconds.
    pub duration_ms: u64,
    /// Every session derived column after the run (they are re-applied
    /// after each refresh and CSV load), with its status. Derived columns
    /// do not count in [`is_ok`](Self::is_ok).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub derived: Vec<DerivedColumnInfo>,
}

impl RefreshReport {
    /// The entry of `name` (case-insensitive).
    #[must_use]
    pub fn table(&self, name: &str) -> Option<&TableRefresh> {
        self.tables
            .iter()
            .find(|t| pbi_model::names_eq(&t.name, name))
    }

    /// `true` if every table and calculated column in scope is OK.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.tables.iter().all(TableRefresh::is_ok)
    }

    /// Tables with a failed or skipped step.
    pub fn problems(&self) -> impl Iterator<Item = &TableRefresh> {
        self.tables.iter().filter(|t| !t.is_ok())
    }
}
