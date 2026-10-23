//! Refresh orchestration: data sources (M partitions, embedded VertiPaq
//! data, the PBIP local data cache, manual CSV loads), then calculated
//! columns and calculated tables in dependency order, then the session's
//! derived columns.
//!
//! A refresh works on a private copy of the engine (the previous tables are
//! shared, not copied); readers keep using the old snapshot until the
//! refresh commits. Cancelling leaves the session unchanged.

pub(crate) mod mquery;
pub(crate) mod plan;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use arrow::record_batch::RecordBatch;
use pbi_dax::{DaxError, EvalContext};
use pbi_engine::{Engine, Value};
use pbi_m::{EvalOptions, MError, ResolverErrorKind};
use pbi_model::{Diagnostic, Model, names_eq};
use tokio::runtime::Handle;
use tokio::task::JoinSet;
use tokio_util::sync::{CancellationToken, DropGuard};

use self::plan::{Node, Plan, Source, node_label};
use crate::cache::CacheData;
use crate::derived::{Applied, reapply};
use crate::error::SessionError;
use crate::options::{ProgressFn, RefreshOptions, RefreshSource};
use crate::report::{
    ColumnRefresh, DAX_UNSUPPORTED_HINT, LOAD_MANUALLY_HINT, OFFLINE_HINT, RefreshEvent,
    RefreshKind, RefreshReport, TableRefresh, TableStatus,
};
use crate::session::Session;

/// Diagnostic codes emitted by refreshes.
pub mod codes {
    /// A calculated column or table is part of a dependency cycle.
    pub const CYCLE: &str = "SESSION001";
    /// Values of a manually loaded CSV could not be converted.
    pub const CSV_CONVERSION: &str = "SESSION002";
    /// A model column was not found in a manually loaded CSV.
    pub const CSV_MISSING_COLUMN: &str = "SESSION003";
    /// The document has no semantic model (e.g. a report bound to a
    /// remote model).
    pub const NO_MODEL: &str = "SESSION004";
    /// A model table or column is not in the local data cache (or the cache
    /// has tables the model does not).
    pub const CACHE_MISSING: &str = "SESSION005";
    /// Cached values could not be converted to the model data type.
    pub const CACHE_CONVERSION: &str = "SESSION006";
}

/// The current Tokio runtime, or an error instead of a panic.
pub(crate) fn runtime() -> Result<Handle, SessionError> {
    Handle::try_current()
        .map_err(|_| SessionError::Internal("pbi-session must run on a Tokio runtime".into()))
}

pub(crate) fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// Sets the M evaluator's cancellation flag when the token is cancelled,
/// and when dropped (all evaluations have finished by then, unless the
/// refresh future itself was dropped: then they stop too).
struct CancelBridge {
    flag: Arc<AtomicBool>,
    watcher: Option<tokio::task::JoinHandle<()>>,
}

impl CancelBridge {
    fn new(handle: &Handle, token: &CancellationToken, flag: &Arc<AtomicBool>) -> Self {
        let (token, watch) = (token.clone(), Arc::clone(flag));
        Self {
            flag: Arc::clone(flag),
            watcher: Some(handle.spawn(async move {
                token.cancelled().await;
                watch.store(true, Ordering::SeqCst);
            })),
        }
    }
}

impl Drop for CancelBridge {
    fn drop(&mut self) {
        self.flag.store(true, Ordering::SeqCst);
        if let Some(h) = self.watcher.take() {
            h.abort();
        }
    }
}

/// Status of a failed M evaluation.
fn m_status(e: &MError) -> TableStatus {
    let (unsupported, hint) = match e {
        MError::Unsupported { hint, .. } => (
            true,
            Some(if hint.is_empty() {
                LOAD_MANUALLY_HINT.to_owned()
            } else {
                hint.clone()
            }),
        ),
        MError::Resolver(r) if r.kind == ResolverErrorKind::Offline => {
            (false, Some(OFFLINE_HINT.to_owned()))
        }
        MError::Resolver(_) => (
            false,
            Some("Check the blob map entry, or Load CSV manually…".to_owned()),
        ),
        _ => (false, None),
    };
    TableStatus::Error {
        message: e.to_string(),
        unsupported,
        hint,
        csv_fallback: true,
    }
}

/// Status of a failed DAX computation.
fn dax_status(e: &DaxError) -> TableStatus {
    TableStatus::Error {
        message: e.to_string(),
        unsupported: e.is_unsupported(),
        hint: e.is_unsupported().then(|| DAX_UNSUPPORTED_HINT.to_owned()),
        csv_fallback: false,
    }
}

fn plain_error(message: impl Into<String>, csv_fallback: bool) -> TableStatus {
    TableStatus::Error {
        message: message.into(),
        unsupported: false,
        hint: None,
        csv_fallback,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeState {
    /// Not part of this run (keeps its previous data).
    Unchanged,
    Ok,
    Failed,
    Skipped,
}

/// One refresh (or manual load) in progress.
pub(crate) struct Run {
    handle: Handle,
    model: Arc<Model>,
    plan: Plan,
    engine: Engine,
    affected: Vec<bool>,
    states: Vec<NodeState>,
    /// Data / calculated-table step result per model table.
    tables: Vec<Option<TableRefresh>>,
    /// Calculated column results per model table.
    columns: Vec<Vec<ColumnRefresh>>,
    diagnostics: Vec<Diagnostic>,
    /// Decoded local data cache (refreshes from the cache).
    cache: Option<CacheData>,
    /// The derived columns re-applied by [`Run::apply_derived`].
    derived: Option<Applied>,
    /// Child of the caller's token, cancelled when the run is dropped (so
    /// blocking M / DAX work stops if the refresh future is dropped).
    cancel: CancellationToken,
    _cancel_on_drop: DropGuard,
    progress: Option<ProgressFn>,
    start: Instant,
}

impl Run {
    /// Starts a run over `seeds` (node indices) on a copy of the session's
    /// current data.
    pub fn new(
        session: &Session,
        plan: Plan,
        seeds: &[usize],
        cancel: CancellationToken,
        progress: Option<ProgressFn>,
    ) -> Result<Run, SessionError> {
        let handle = runtime()?;
        let model = session.base_model();
        let (snapshot, _) = session.base_snapshot_and_model();
        let mut engine = Engine::new(&model);
        let previous: Vec<(String, RecordBatch)> = snapshot
            .storage()
            .tables()
            .filter(|t| t.is_loaded())
            .map(|t| (t.name().to_owned(), t.batch().clone()))
            .collect();
        if !previous.is_empty() {
            engine.load_tables(previous)?;
        }
        let affected = plan.affected(seeds);
        let n = plan.nodes.len();
        let tables = model.tables.len();
        let cancel = cancel.child_token();
        let mut diagnostics = Vec::new();
        if !session.has_model() {
            diagnostics.push(Diagnostic::info(
                codes::NO_MODEL,
                "the document has no semantic model; there is no data to refresh",
            ));
        }
        Ok(Run {
            handle,
            model,
            affected,
            states: vec![NodeState::Unchanged; n],
            plan,
            engine,
            tables: vec![None; tables],
            columns: vec![Vec::new(); tables],
            diagnostics,
            cache: None,
            derived: None,
            _cancel_on_drop: cancel.clone().drop_guard(),
            cancel,
            progress,
            start: Instant::now(),
        })
    }

    fn emit(&self, event: RefreshEvent) {
        if let Some(p) = &self.progress {
            p(event);
        }
    }

    fn check_cancel(&self) -> Result<(), SessionError> {
        if self.cancel.is_cancelled() {
            Err(SessionError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn table_name(&self, i: usize) -> String {
        self.model
            .tables
            .get(i)
            .map(|t| t.name.clone())
            .unwrap_or_default()
    }

    fn set_state(&mut self, node: Node, state: NodeState) {
        if let Some(s) = self.plan.index(node).and_then(|n| self.states.get_mut(n)) {
            *s = state;
        }
    }

    fn is_affected(&self, node: Node) -> bool {
        self.plan
            .index(node)
            .and_then(|n| self.affected.get(n))
            .copied()
            .unwrap_or(false)
    }

    /// Records the data step of table `i` and emits `TableFinished`.
    fn finish_table(
        &mut self,
        i: usize,
        kind: RefreshKind,
        elapsed: Duration,
        status: TableStatus,
    ) {
        let state = match &status {
            TableStatus::Ok => NodeState::Ok,
            TableStatus::Error { .. } => NodeState::Failed,
            TableStatus::Skipped { .. } => NodeState::Skipped,
        };
        self.set_state(Node::Table(i), state);
        let name = self.table_name(i);
        let entry = TableRefresh {
            rows: self.engine.row_count(&name).unwrap_or(0),
            name,
            kind,
            duration_ms: millis(elapsed),
            status,
            columns: Vec::new(),
        };
        self.emit(RefreshEvent::TableFinished {
            table: entry.clone(),
        });
        if let Some(slot) = self.tables.get_mut(i) {
            *slot = Some(entry);
        }
    }

    /// Stores a loaded batch, turning a storage error into a step error.
    fn store(&mut self, i: usize, batch: RecordBatch) -> TableStatus {
        let name = self.table_name(i);
        match self.engine.load_table(&name, batch) {
            Ok(()) => TableStatus::Ok,
            Err(e) => plain_error(format!("cannot store the table: {e}"), true),
        }
    }

    /// Table names in scope, in model order.
    fn scope(&self) -> Vec<usize> {
        (0..self.model.tables.len())
            .filter(|&i| {
                self.plan
                    .nodes
                    .iter()
                    .zip(&self.affected)
                    .any(|(n, a)| *a && matches!(n, Node::Table(t) | Node::Column(t, _) if *t == i))
            })
            .collect()
    }

    pub fn emit_started(&self) {
        let tables = self
            .scope()
            .into_iter()
            .map(|i| self.table_name(i))
            .collect();
        self.emit(RefreshEvent::Started { tables });
    }

    /// Loads every affected table that has a data source (phase 1).
    pub async fn load_sources(&mut self, session: &Session) -> Result<(), SessionError> {
        let jobs: Vec<(usize, Source)> = self
            .plan
            .sources
            .iter()
            .enumerate()
            .filter(|(i, s)| **s != Source::Calculated && self.is_affected(Node::Table(*i)))
            .map(|(i, s)| (i, s.clone()))
            .collect();
        let mut m_jobs = Vec::new();
        for (i, source) in jobs {
            match source {
                Source::Skip(reason) => {
                    self.emit(RefreshEvent::TableStarted {
                        name: self.table_name(i),
                        kind: RefreshKind::Skipped,
                    });
                    self.finish_table(
                        i,
                        RefreshKind::Skipped,
                        Duration::ZERO,
                        TableStatus::Skipped { reason },
                    );
                }
                Source::Pbix => self.load_pbix(session, i).await?,
                Source::Cache => self.load_cached(i),
                Source::NotCached(message) => {
                    self.emit(RefreshEvent::TableStarted {
                        name: self.table_name(i),
                        kind: RefreshKind::Cache,
                    });
                    self.finish_table(
                        i,
                        RefreshKind::Cache,
                        Duration::ZERO,
                        TableStatus::Error {
                            message,
                            unsupported: false,
                            hint: Some(
                                "Refresh from the source (M), or Load CSV manually…".to_owned(),
                            ),
                            csv_fallback: true,
                        },
                    );
                }
                Source::M(exprs) => m_jobs.push((i, exprs)),
                Source::CalculationGroup => self.load_calculation_group(i),
                Source::Calculated => {}
            }
        }
        if !m_jobs.is_empty() {
            self.load_m(session, m_jobs).await?;
        }
        self.check_cancel()
    }

    async fn load_pbix(&mut self, session: &Session, i: usize) -> Result<(), SessionError> {
        self.check_cancel()?;
        let name = self.table_name(i);
        self.emit(RefreshEvent::TableStarted {
            name: name.clone(),
            kind: RefreshKind::Pbix,
        });
        let start = Instant::now();
        let session = session.clone();
        let (result, diagnostics) = self
            .handle
            .spawn_blocking(move || session.decode_table(&name))
            .await?;
        self.diagnostics.extend(diagnostics);
        let status = match result {
            Ok(batch) => self.store(i, batch),
            Err(message) => plain_error(message, true),
        };
        self.finish_table(i, RefreshKind::Pbix, start.elapsed(), status);
        Ok(())
    }

    /// The rows of a calculation-group table: one per calculation item, in
    /// model order (`sourceColumn: Name` → item name, `Ordinal` → ordinal,
    /// -1 when unset; other columns BLANK).
    fn load_calculation_group(&mut self, i: usize) {
        let name = self.table_name(i);
        self.emit(RefreshEvent::TableStarted {
            name,
            kind: RefreshKind::Calculated,
        });
        let start = Instant::now();
        let status = match self.model.tables.get(i).map(calculation_group_batch) {
            Some(Ok(batch)) => self.store(i, batch),
            Some(Err(message)) => plain_error(message, false),
            None => plain_error("the table is not in the model", false),
        };
        self.finish_table(i, RefreshKind::Calculated, start.elapsed(), status);
    }

    /// Uses the local data cache (refreshes from the cache).
    pub fn set_cache(&mut self, mut cache: CacheData) {
        self.diagnostics.append(&mut cache.diagnostics);
        self.cache = Some(cache);
    }

    fn load_cached(&mut self, i: usize) {
        let name = self.table_name(i);
        self.emit(RefreshEvent::TableStarted {
            name,
            kind: RefreshKind::Cache,
        });
        let start = Instant::now();
        let status = match self.cache.as_mut().and_then(|c| c.take(i)) {
            Some(cached) => {
                if let Some(list) = self.columns.get_mut(i) {
                    list.extend(cached.missing.iter().map(|c| ColumnRefresh {
                        name: c.clone(),
                        duration_ms: 0,
                        status: TableStatus::Skipped {
                            reason: "not in the local data cache; loaded as blank".to_owned(),
                        },
                    }));
                }
                self.store(i, cached.batch)
            }
            None => plain_error("the table is not in the local data cache", true),
        };
        self.finish_table(i, RefreshKind::Cache, start.elapsed(), status);
    }

    async fn load_m(
        &mut self,
        session: &Session,
        jobs: Vec<(usize, Vec<String>)>,
    ) -> Result<(), SessionError> {
        let flag = Arc::new(AtomicBool::new(self.cancel.is_cancelled()));
        let _bridge = CancelBridge::new(&self.handle, &self.cancel, &flag);
        let engine = pbi_m::Engine::new(EvalOptions {
            resolver: session.resolver(),
            clock: session.clock(),
            limits: session.limits().m,
            cancel: Arc::clone(&flag),
        });
        let shared = Arc::new(mquery::shared_queries(&self.model));
        let slots = Arc::new(tokio::sync::Semaphore::new(
            session.limits().max_parallel_queries.max(1),
        ));
        let mut set = JoinSet::new();
        let mut cancelled = false;
        for (i, exprs) in jobs {
            let permit = Arc::clone(&slots)
                .acquire_owned()
                .await
                .map_err(|e| SessionError::Internal(e.to_string()))?;
            if self.cancel.is_cancelled() {
                cancelled = true;
                break;
            }
            let Some(table) = self.model.tables.get(i) else {
                continue;
            };
            let columns = mquery::data_columns(table);
            self.emit(RefreshEvent::TableStarted {
                name: table.name.clone(),
                kind: RefreshKind::M,
            });
            let (engine, shared) = (engine.clone(), Arc::clone(&shared));
            set.spawn_blocking_on(
                move || {
                    let _permit = permit;
                    let start = Instant::now();
                    let result = mquery::evaluate_table(&engine, &shared, &exprs, &columns);
                    (i, result, start.elapsed())
                },
                &self.handle,
            );
        }
        while let Some(joined) = set.join_next().await {
            let (i, result, elapsed) = joined?;
            let status = match result {
                Ok(batch) => self.store(i, batch),
                Err(MError::Cancelled) => {
                    cancelled = true;
                    continue;
                }
                Err(e) => m_status(&e),
            };
            if self.cancel.is_cancelled() {
                cancelled = true;
                continue;
            }
            self.finish_table(i, RefreshKind::M, elapsed, status);
        }
        if cancelled {
            return Err(SessionError::Cancelled);
        }
        Ok(())
    }

    /// Installs a manually loaded table (instead of phase 1).
    pub fn set_manual(&mut self, i: usize, batch: RecordBatch, elapsed: Duration) {
        self.emit(RefreshEvent::TableStarted {
            name: self.table_name(i),
            kind: RefreshKind::Manual,
        });
        let status = self.store(i, batch);
        self.finish_table(i, RefreshKind::Manual, elapsed, status);
    }

    pub fn add_diagnostics(&mut self, diagnostics: impl IntoIterator<Item = Diagnostic>) {
        self.diagnostics.extend(diagnostics);
    }

    /// Why node `n` cannot run (a dependency failed or was skipped).
    fn blocked_by(&self, n: usize) -> Option<String> {
        for &d in self.plan.deps.get(n).map(Vec::as_slice).unwrap_or_default() {
            let label = || {
                self.plan
                    .nodes
                    .get(d)
                    .map(|node| node_label(&self.model, *node))
                    .unwrap_or_default()
            };
            match self.states.get(d) {
                Some(NodeState::Failed) => {
                    return Some(format!("depends on {}, which failed to refresh", label()));
                }
                Some(NodeState::Skipped) => {
                    return Some(format!("depends on {}, which was not refreshed", label()));
                }
                _ => {}
            }
        }
        None
    }

    /// Computes the affected calculated columns and tables (phase 2).
    pub async fn calculate(&mut self) -> Result<(), SessionError> {
        for c in self.plan.cyclic.clone() {
            if !self.affected.get(c).copied().unwrap_or(false) {
                continue;
            }
            let Some(&node) = self.plan.nodes.get(c) else {
                continue;
            };
            let label = node_label(&self.model, node);
            let message = format!(
                "A circular dependency was detected: {label} depends (directly or indirectly) on itself."
            );
            self.diagnostics
                .push(Diagnostic::error(codes::CYCLE, message.clone()));
            self.record_calc(node, Duration::ZERO, plain_error(message, false));
        }
        for n in self.plan.order.clone() {
            if !self.affected.get(n).copied().unwrap_or(false) {
                continue;
            }
            let Some(&node) = self.plan.nodes.get(n) else {
                continue;
            };
            let calc_table = matches!(node, Node::Table(i)
                if self.plan.sources.get(i) == Some(&Source::Calculated));
            if matches!(node, Node::Table(_)) && !calc_table {
                continue;
            }
            self.check_cancel()?;
            if let Some(reason) = self.blocked_by(n) {
                if let Node::Column(i, j) = node {
                    let column = self.column_name(i, j);
                    self.emit(RefreshEvent::ColumnStarted {
                        table: self.table_name(i),
                        column,
                    });
                }
                self.record_calc(node, Duration::ZERO, TableStatus::Skipped { reason });
                continue;
            }
            let start = Instant::now();
            let status = match node {
                Node::Table(i) => self.compute_table(i).await?,
                Node::Column(i, j) => self.compute_column(i, j).await?,
            };
            self.record_calc(node, start.elapsed(), status);
        }
        Ok(())
    }

    fn column_name(&self, i: usize, j: usize) -> String {
        self.model
            .tables
            .get(i)
            .and_then(|t| t.columns.get(j))
            .map(|c| c.name.clone())
            .unwrap_or_default()
    }

    fn context(&self) -> EvalContext {
        EvalContext::new(self.engine.snapshot(), Arc::clone(&self.model))
            .with_cancellation_token(self.cancel.clone())
    }

    async fn compute_table(&mut self, i: usize) -> Result<TableStatus, SessionError> {
        let name = self.table_name(i);
        self.emit(RefreshEvent::TableStarted {
            name: name.clone(),
            kind: RefreshKind::Calculated,
        });
        let ctx = self.context();
        let result = self
            .handle
            .spawn_blocking(move || ctx.compute_calculated_table(&name))
            .await?;
        Ok(match result {
            Ok(batch) => self.store(i, batch),
            Err(DaxError::Cancelled) => return Err(SessionError::Cancelled),
            Err(e) => dax_status(&e),
        })
    }

    async fn compute_column(&mut self, i: usize, j: usize) -> Result<TableStatus, SessionError> {
        let (table, column) = (self.table_name(i), self.column_name(i, j));
        self.emit(RefreshEvent::ColumnStarted {
            table: table.clone(),
            column: column.clone(),
        });
        let ctx = self.context();
        let (t, c) = (table.clone(), column.clone());
        let result = self
            .handle
            .spawn_blocking(move || ctx.compute_calculated_column(&t, &c))
            .await?;
        Ok(match result {
            Ok(array) => match self.engine.add_column(&table, &column, array) {
                Ok(()) => TableStatus::Ok,
                Err(e) => plain_error(format!("cannot store the column: {e}"), false),
            },
            Err(DaxError::Cancelled) => return Err(SessionError::Cancelled),
            Err(e) => dax_status(&e),
        })
    }

    /// Records the result of a calculated table or column step.
    fn record_calc(&mut self, node: Node, elapsed: Duration, status: TableStatus) {
        match node {
            Node::Table(i) => {
                let kind = if status.is_skipped() {
                    RefreshKind::Skipped
                } else {
                    RefreshKind::Calculated
                };
                self.finish_table(i, kind, elapsed, status);
            }
            Node::Column(i, j) => {
                let state = match &status {
                    TableStatus::Ok => NodeState::Ok,
                    TableStatus::Error { .. } => NodeState::Failed,
                    TableStatus::Skipped { .. } => NodeState::Skipped,
                };
                self.set_state(node, state);
                let entry = ColumnRefresh {
                    name: self.column_name(i, j),
                    duration_ms: millis(elapsed),
                    status,
                };
                self.emit(RefreshEvent::ColumnFinished {
                    table: self.table_name(i),
                    column: entry.clone(),
                });
                if let Some(list) = self.columns.get_mut(i) {
                    list.push(entry);
                }
            }
        }
    }

    /// Re-applies the session's derived columns (after the data and DAX
    /// steps; see [`crate::derived`]).
    pub async fn apply_derived(&mut self, session: &Session) -> Result<(), SessionError> {
        self.check_cancel()?;
        let applied = reapply(
            &self.handle,
            &mut self.engine,
            &self.model,
            session.derived_entries(),
            &self.cancel,
        )
        .await?;
        self.derived = Some(applied);
        Ok(())
    }

    /// Puts the columns of refreshed tables back in model order.
    fn reorder_columns(&mut self, scope: &[usize]) {
        for &i in scope {
            let Some(table) = self.model.tables.get(i) else {
                continue;
            };
            let Ok(data) = self.engine.table(&table.name) else {
                continue;
            };
            let batch = data.batch();
            let schema = batch.schema();
            let current: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
            let mut order: Vec<usize> = table
                .columns
                .iter()
                .filter_map(|c| current.iter().position(|n| names_eq(n, &c.name)))
                .collect();
            for k in 0..current.len() {
                if !order.contains(&k) {
                    order.push(k);
                }
            }
            if order.iter().copied().eq(0..current.len()) {
                continue;
            }
            if let Ok(projected) = batch.project(&order) {
                let name = table.name.clone();
                if let Err(e) = self.engine.replace_table(&name, projected) {
                    tracing::warn!(table = %name, error = %e, "cannot reorder columns");
                }
            }
        }
    }

    /// Builds the report and commits the new data to the session.
    pub fn finish(mut self, session: &Session) -> RefreshReport {
        let scope = self.scope();
        self.reorder_columns(&scope);
        let mut entries = Vec::with_capacity(scope.len());
        for i in scope {
            let name = self.table_name(i);
            let rows = self.engine.row_count(&name).unwrap_or(0);
            let columns = self
                .columns
                .get_mut(i)
                .map(std::mem::take)
                .unwrap_or_default();
            let entry = match self.tables.get_mut(i).and_then(Option::take) {
                Some(mut t) => {
                    t.rows = rows;
                    t.columns = columns;
                    t
                }
                None => TableRefresh {
                    name,
                    kind: RefreshKind::Calculated,
                    rows,
                    duration_ms: 0,
                    status: TableStatus::Ok,
                    columns,
                },
            };
            entries.push(entry);
        }
        let mut diagnostics = std::mem::take(&mut self.diagnostics);
        diagnostics.extend(self.engine.diagnostics());
        let derived = self.derived.take().unwrap_or_else(|| Applied {
            model: session.model(),
            entries: session.derived_entries(),
        });
        let report = RefreshReport {
            duration_ms: millis(self.start.elapsed()),
            tables: entries,
            diagnostics,
            derived: derived.entries.iter().map(|e| e.info.clone()).collect(),
        };
        let (ok, failed, skipped) = report.tables.iter().fold((0, 0, 0), |(o, f, s), t| {
            if t.status.is_skipped() {
                (o, f, s + 1)
            } else if t.is_ok() {
                (o + 1, f, s)
            } else {
                (o, f + 1, s)
            }
        });
        let progress = self.progress.take();
        session.commit(self.engine, &report.tables, derived);
        if let Some(p) = progress {
            p(RefreshEvent::Finished {
                duration_ms: report.duration_ms,
                ok,
                failed,
                skipped,
            });
        }
        report
    }
}

/// Plan nodes to run for `tables` (`None`: everything): each table and its
/// own calculated columns.
/// One row per calculation item of a calculation-group table.
fn calculation_group_batch(table: &pbi_model::Table) -> Result<RecordBatch, String> {
    let group = table
        .effective_calculation_group()
        .ok_or_else(|| "the calculation group has no definition".to_owned())?;
    let items = &group.calculation_items;
    let mut fields = Vec::new();
    let mut arrays = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for column in &table.columns {
        if matches!(column.kind, pbi_model::ColumnKind::RowNumber)
            || !seen.insert(pbi_model::names::name_key(&column.name))
        {
            continue;
        }
        let source = match &column.kind {
            pbi_model::ColumnKind::Data { source_column } => source_column.as_str(),
            _ => "",
        };
        let values: Vec<Value> = if pbi_model::names_eq(source, "Name") {
            items
                .iter()
                .map(|i| Value::string(i.name.as_str()))
                .collect()
        } else if pbi_model::names_eq(source, "Ordinal") {
            items
                .iter()
                .map(|i| Value::Integer(i.ordinal.unwrap_or(-1)))
                .collect()
        } else {
            vec![Value::Blank; items.len()]
        };
        let data_type = match &column.data_type {
            pbi_model::DataType::Automatic | pbi_model::DataType::Unknown(_) => {
                pbi_engine::value::infer_arrow_type(&values)
            }
            other => pbi_engine::arrow_type(other),
        };
        let array = pbi_engine::value::values_to_array(&values, &data_type)
            .map_err(|e| format!("column [{}]: {e}", column.name))?;
        fields.push(arrow::datatypes::Field::new(
            column.name.as_str(),
            array.data_type().clone(),
            true,
        ));
        arrays.push(array);
    }
    let options = arrow::record_batch::RecordBatchOptions::new().with_row_count(Some(items.len()));
    RecordBatch::try_new_with_options(
        std::sync::Arc::new(arrow::datatypes::Schema::new(fields)),
        arrays,
        &options,
    )
    .map_err(|e| e.to_string())
}

fn seeds(
    plan: &Plan,
    model: &Model,
    tables: Option<&[String]>,
) -> Result<Vec<usize>, SessionError> {
    let Some(names) = tables else {
        return Ok((0..plan.nodes.len()).collect());
    };
    let mut seeds = Vec::with_capacity(names.len());
    for name in names {
        let i = model
            .tables
            .iter()
            .position(|t| names_eq(&t.name, name))
            .ok_or_else(|| SessionError::UnknownTable(name.clone()))?;
        seeds.extend(plan.index(Node::Table(i)));
        // Recompute the table's own calculated columns as well.
        seeds.extend(
            plan.nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| matches!(n, Node::Column(t, _) if *t == i))
                .map(|(k, _)| k),
        );
    }
    Ok(seeds)
}

impl Session {
    /// Refreshes the model's data, then calculated columns and calculated
    /// tables in dependency order, then the session's derived columns.
    ///
    /// The data comes from [`RefreshOptions::source`]:
    /// [`RefreshSource::Query`] (the default) evaluates the M partitions
    /// (PBIP, PBIT, `.bim`, TMDL) or decodes the embedded VertiPaq data
    /// (PBIX, ABF); [`RefreshSource::Cache`] loads the PBIP local data cache
    /// (see [`load_cached_data`](Self::load_cached_data));
    /// [`RefreshSource::Auto`] uses the cache when there is one
    /// ([`has_cached_data`](Self::has_cached_data)), else the query.
    ///
    /// A table that fails keeps its previous data and is reported with
    /// [`TableStatus::Error`]; calculated objects that depend on it are
    /// [`TableStatus::Skipped`]. The call itself fails only for unknown
    /// table names, a missing or unreadable cache, cancellation (the session
    /// then keeps its previous data) and internal errors. Refreshes are
    /// serialized.
    ///
    /// # Errors
    /// [`SessionError::UnknownTable`], [`SessionError::NoCache`],
    /// [`SessionError::Cache`], [`SessionError::Cancelled`],
    /// [`SessionError::Internal`].
    pub async fn refresh(&self, opts: RefreshOptions) -> Result<RefreshReport, SessionError> {
        let _guard = self.refresh_lock().lock().await;
        let start = Instant::now();
        let model = self.base_model();
        let from_cache = !self.is_pbix()
            && match opts.source {
                RefreshSource::Query => false,
                RefreshSource::Cache => true,
                RefreshSource::Auto => self.has_cached_data(),
            };
        let (plan, cache) = if from_cache {
            if opts.cancel.is_cancelled() {
                return Err(SessionError::Cancelled);
            }
            let handle = runtime()?;
            let session = self.clone();
            let cache = handle
                .spawn_blocking(move || session.read_cache())
                .await??;
            (cache.plan(&model), Some(cache))
        } else {
            (Plan::build(&model, self.is_pbix()), None)
        };
        let seeds = seeds(&plan, &model, opts.tables.as_deref())?;
        let mut run = Run::new(
            self,
            plan,
            &seeds,
            opts.cancel.clone(),
            opts.progress.clone(),
        )?;
        run.start = start;
        if let Some(cache) = cache {
            run.set_cache(cache);
        }
        run.check_cancel()?;
        run.emit_started();
        run.load_sources(self).await?;
        run.calculate().await?;
        run.apply_derived(self).await?;
        Ok(run.finish(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m_errors_offer_the_csv_fallback() {
        let offline = MError::Resolver(pbi_m::ResolverError::new(
            ResolverErrorKind::Offline,
            "offline: no blob mapping",
        ));
        match m_status(&offline) {
            TableStatus::Error {
                hint,
                csv_fallback,
                unsupported,
                ..
            } => {
                assert_eq!(hint.as_deref(), Some(OFFLINE_HINT));
                assert!(csv_fallback);
                assert!(!unsupported);
            }
            other => panic!("{other:?}"),
        }
        let unsupported = MError::unsupported("Sql.Database", "");
        assert!(matches!(
            m_status(&unsupported),
            TableStatus::Error { unsupported: true, hint: Some(h), .. } if h == LOAD_MANUALLY_HINT
        ));
    }

    #[test]
    fn unsupported_dax_hint_names_the_product() {
        let unsupported = DaxError::UnsupportedFunction {
            name: "PATHCONTAINS".into(),
        };
        match dax_status(&unsupported) {
            TableStatus::Error {
                hint: Some(hint),
                unsupported: true,
                ..
            } => {
                assert_eq!(
                    hint,
                    "This DAX feature is not implemented in DashCraft yet."
                );
            }
            other => panic!("{other:?}"),
        }
    }
}
