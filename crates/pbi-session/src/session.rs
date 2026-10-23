//! The [`Session`] handle: an opened document, its engine state and the
//! last refresh results.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use arrow::record_batch::RecordBatch;
use pbi_core::{ModelSource, OpenedDocument};
use pbi_engine::{Engine, EngineSnapshot};
use pbi_m::{BlobResolver, Clock, MapBlobResolver, NoBlobResolver, SystemClock};
use pbi_model::names::name_key;
use pbi_model::{Diagnostic, Document, Model, TableKind};
use serde::{Deserialize, Serialize};

use crate::derived::{Applied, DerivedEntry};
use crate::error::SessionError;
use crate::options::{SessionLimits, SessionOptions};
use crate::report::TableRefresh;
use crate::security::{ActiveView, ViewAs};

/// Mutable state: the published engine, the session model, the derived
/// columns and the last result per table.
#[derive(Debug)]
struct State {
    engine: Engine,
    /// The document model plus the synthetic entries of the active derived
    /// columns.
    model: Arc<Model>,
    /// Registered derived columns, in creation order.
    derived: Vec<DerivedEntry>,
    /// Last refresh / load result per table (by case-folded name).
    tables: HashMap<String, TableRefresh>,
    /// "View as": the restricted engine queries read instead of `engine`.
    view: Option<ActiveView>,
}

impl State {
    /// The engine queries read: the "View as" engine when one is active.
    fn query_engine(&self) -> &Engine {
        self.view.as_ref().map_or(&self.engine, |v| &v.engine)
    }
}

struct Inner {
    opened: OpenedDocument,
    /// The document model (never contains derived columns).
    base_model: Arc<Model>,
    resolver: Arc<dyn BlobResolver>,
    blob_map: bool,
    clock: Arc<dyn Clock>,
    limits: SessionLimits,
    state: RwLock<State>,
    refresh_lock: tokio::sync::Mutex<()>,
}

/// An opened Power BI document with its data engine: the façade used by
/// the CLI and the app (like Power BI Desktop's local engine session).
///
/// Cheap to clone (`Arc`), `Send + Sync`. Queries run on immutable engine
/// snapshots, so they never block a refresh and always see a consistent
/// state. Async methods need a Tokio runtime.
#[derive(Clone)]
pub struct Session {
    inner: Arc<Inner>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("source", &self.inner.opened.document.source.path)
            .field("model", &self.inner.base_model.name)
            .field("tables", &self.inner.base_model.tables.len())
            .field("blob_map", &self.inner.blob_map)
            .finish_non_exhaustive()
    }
}

/// Summary of one model table and its data in the session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    /// Table name.
    pub name: String,
    /// Model kind: `regular`, `calculatedTable`, `calculationGroup`, `remote`.
    pub kind: TableKind,
    /// Hidden in report view.
    pub hidden: bool,
    /// Rows currently loaded (0 before the first refresh).
    pub rows: usize,
    /// Columns currently stored (data, computed calculated columns and
    /// active derived columns).
    pub columns: usize,
    /// Data has been loaded (refreshed, decoded or loaded from CSV).
    pub loaded: bool,
    /// Result of the last refresh or manual load of this table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_refresh: Option<TableRefresh>,
}

impl Session {
    /// Opens a file or folder (PBIP, PBIX, PBIT, `.bim`, TMDL, ABF, …) with
    /// [`pbi_core::open_path`]. No data is loaded until
    /// [`refresh`](Self::refresh). A document without a semantic model (a
    /// report bound to a remote model) opens with an empty model.
    ///
    /// # Errors
    /// [`SessionError::Open`] if the input cannot be opened,
    /// [`SessionError::BlobMap`] if `options.blob_map` cannot be read.
    pub fn open(path: impl AsRef<Path>, options: SessionOptions) -> Result<Session, SessionError> {
        let opened = pbi_core::open_path(path.as_ref(), &options.open)?;
        Self::from_opened(opened, options)
    }

    /// Wraps an already opened document.
    ///
    /// # Errors
    /// [`SessionError::BlobMap`] if `options.blob_map` cannot be read.
    pub fn from_opened(
        opened: OpenedDocument,
        options: SessionOptions,
    ) -> Result<Session, SessionError> {
        let (resolver, blob_map): (Arc<dyn BlobResolver>, bool) =
            match (options.resolver, &options.blob_map) {
                (Some(r), _) => (r, true),
                (None, Some(path)) => {
                    let map = MapBlobResolver::from_map_file(path).map_err(|e| {
                        SessionError::BlobMap {
                            path: path.clone(),
                            message: e.to_string(),
                        }
                    })?;
                    (Arc::new(map), true)
                }
                (None, None) => (Arc::new(NoBlobResolver), false),
            };
        let model = Arc::new(opened.document.model.clone().unwrap_or_default());
        let engine = Engine::new(&model);
        Ok(Session {
            inner: Arc::new(Inner {
                opened,
                base_model: Arc::clone(&model),
                resolver,
                blob_map,
                clock: options.clock.unwrap_or_else(|| Arc::new(SystemClock)),
                limits: options.limits,
                state: RwLock::new(State {
                    engine,
                    model,
                    derived: Vec::new(),
                    tables: HashMap::new(),
                    view: None,
                }),
                refresh_lock: tokio::sync::Mutex::new(()),
            }),
        })
    }

    /// The opened document (model, reports, diagnostics, source info).
    #[must_use]
    pub fn document(&self) -> &Document {
        &self.inner.opened.document
    }

    /// Everything the reader returned (detection, container, sources).
    #[must_use]
    pub fn opened(&self) -> &OpenedDocument {
        &self.inner.opened
    }

    /// The session's semantic model (empty if the document has none): the
    /// document model plus a synthetic column entry for each active derived
    /// column (annotation [`DERIVED_ANNOTATION`](crate::DERIVED_ANNOTATION);
    /// see [`add_derived_column`](Self::add_derived_column)). Use
    /// [`base_model`](Self::base_model) for the model as read from the
    /// document, e.g. to save or convert it.
    #[must_use]
    pub fn model(&self) -> Arc<Model> {
        Arc::clone(&self.read().model)
    }

    /// The semantic model as read from the document (empty if it has none),
    /// without session derived columns.
    #[must_use]
    pub fn base_model(&self) -> Arc<Model> {
        Arc::clone(&self.inner.base_model)
    }

    /// `true` if the document has a semantic model.
    #[must_use]
    pub fn has_model(&self) -> bool {
        self.inner.opened.document.model.is_some()
    }

    /// `true` if the data comes from embedded VertiPaq storage (PBIX, ABF)
    /// rather than M partitions.
    #[must_use]
    pub fn is_pbix(&self) -> bool {
        matches!(
            self.inner.opened.model_source,
            Some(ModelSource::Abf { .. })
        )
    }

    /// `true` if a blob map or custom resolver is configured.
    #[must_use]
    pub fn has_blob_map(&self) -> bool {
        self.inner.blob_map
    }

    /// The model culture (`en-US` when unset); used to format values.
    #[must_use]
    pub fn culture(&self) -> String {
        self.inner
            .base_model
            .culture
            .clone()
            .filter(|c| !c.trim().is_empty())
            .unwrap_or_else(|| "en-US".to_owned())
    }

    /// The current engine snapshot (immutable; cheap to clone). Pass it to
    /// `pbi-dax` / `pbi-engine` for advanced queries. With "View as"
    /// ([`set_view_as`](Self::set_view_as)) it holds only the rows the roles
    /// allow and carries the effective user.
    #[must_use]
    pub fn snapshot(&self) -> Arc<EngineSnapshot> {
        self.read().query_engine().snapshot()
    }

    /// One table's data as a single batch (columns in model order).
    ///
    /// # Errors
    /// [`SessionError::UnknownTable`].
    pub fn table_batch(&self, table: &str) -> Result<RecordBatch, SessionError> {
        let snapshot = self.snapshot();
        let data = snapshot
            .table(table)
            .map_err(|_| SessionError::UnknownTable(table.to_owned()))?;
        Ok(data.batch().clone())
    }

    /// Every model table with its loaded row count and last refresh result.
    #[must_use]
    pub fn tables(&self) -> Vec<TableInfo> {
        let state = self.read();
        let snapshot = state.query_engine().snapshot();
        self.inner
            .base_model
            .tables
            .iter()
            .map(|t| {
                let data = snapshot.table(&t.name).ok();
                TableInfo {
                    name: t.name.clone(),
                    kind: t.kind(),
                    hidden: t.is_hidden,
                    rows: data.map_or(0, |d| d.row_count()),
                    columns: data.map_or(0, |d| d.column_names().count()),
                    loaded: data.is_some_and(|d| d.is_loaded()),
                    last_refresh: state.tables.get(&name_key(&t.name)).cloned(),
                }
            })
            .collect()
    }

    /// Result of the last refresh or manual load of `table`.
    #[must_use]
    pub fn table_status(&self, table: &str) -> Option<TableRefresh> {
        self.read().tables.get(&name_key(table)).cloned()
    }

    /// Document diagnostics plus the engine's relationship diagnostics (and
    /// a failed "View as").
    #[must_use]
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        let mut out = self.inner.opened.document.diagnostics.clone();
        let state = self.read();
        out.extend(state.engine.snapshot().diagnostics());
        out.extend(state.view.as_ref().and_then(ActiveView::diagnostic));
        out
    }

    // ---- crate-internal ------------------------------------------------------

    fn read(&self) -> RwLockReadGuard<'_, State> {
        self.inner
            .state
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, State> {
        self.inner
            .state
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn refresh_lock(&self) -> &tokio::sync::Mutex<()> {
        &self.inner.refresh_lock
    }

    pub(crate) fn resolver(&self) -> Arc<dyn BlobResolver> {
        Arc::clone(&self.inner.resolver)
    }

    pub(crate) fn clock(&self) -> Arc<dyn Clock> {
        Arc::clone(&self.inner.clock)
    }

    pub(crate) fn limits(&self) -> SessionLimits {
        self.inner.limits
    }

    /// The engine snapshot queries read ("View as" applied) and the session
    /// model, read together.
    pub(crate) fn snapshot_and_model(&self) -> (Arc<EngineSnapshot>, Arc<Model>) {
        let state = self.read();
        (state.query_engine().snapshot(), Arc::clone(&state.model))
    }

    /// The unrestricted snapshot and the session model (refresh, derived
    /// columns: they work on all rows).
    pub(crate) fn base_snapshot_and_model(&self) -> (Arc<EngineSnapshot>, Arc<Model>) {
        let state = self.read();
        (state.engine.snapshot(), Arc::clone(&state.model))
    }

    /// The unrestricted engine and the session model.
    pub(crate) fn base_engine_and_model(&self) -> (Engine, Arc<Model>) {
        let state = self.read();
        (state.engine.clone(), Arc::clone(&state.model))
    }

    pub(crate) fn set_active_view(&self, view: Option<ActiveView>) {
        self.write().view = view;
    }

    pub(crate) fn active_view_settings(&self) -> Option<ViewAs> {
        self.read().view.as_ref().map(|v| v.view.clone())
    }

    pub(crate) fn active_view_error(&self) -> Option<String> {
        self.read().view.as_ref().and_then(|v| v.error.clone())
    }

    /// Rebuilds the active view for the current unrestricted engine (after
    /// it changed). Until then queries keep reading the previous restricted
    /// engine, never the unrestricted one.
    fn resecure(&self) {
        let (base, model, view) = {
            let state = self.read();
            match &state.view {
                Some(v) if v.base_generation != state.engine.generation() => (
                    state.engine.clone(),
                    Arc::clone(&state.model),
                    v.view.clone(),
                ),
                _ => return,
            }
        };
        let rebuilt = ActiveView::build_or_deny(&base, &model, view);
        let mut state = self.write();
        let current = state.engine.generation() == rebuilt.base_generation
            && state.view.as_ref().is_some_and(|v| v.view == rebuilt.view);
        if current {
            state.view = Some(rebuilt);
        }
    }

    /// The registered derived columns.
    pub(crate) fn derived_entries(&self) -> Vec<DerivedEntry> {
        self.read().derived.clone()
    }

    /// Changes the engine, the session model and the derived columns
    /// together (under the state lock; keep `f` cheap).
    pub(crate) fn update_derived<R, E>(
        &self,
        f: impl FnOnce(&mut Engine, &mut Arc<Model>, &mut Vec<DerivedEntry>) -> Result<R, E>,
    ) -> Result<R, E> {
        let result = {
            let mut state = self.write();
            let State {
                engine,
                model,
                derived,
                ..
            } = &mut *state;
            f(engine, model, derived)
        };
        self.resecure();
        result
    }

    /// Publishes a refreshed engine, the step results and the re-applied
    /// derived columns.
    pub(crate) fn commit(&self, engine: Engine, results: &[TableRefresh], derived: Applied) {
        let view = self.active_view_settings();
        let rebuilt = view.map(|v| ActiveView::build_or_deny(&engine, &derived.model, v));
        let mut state = self.write();
        state.engine = engine;
        state.model = derived.model;
        state.derived = derived.entries;
        for r in results {
            state.tables.insert(name_key(&r.name), r.clone());
        }
        if let (Some(rebuilt), Some(current)) = (rebuilt, &state.view)
            && current.view == rebuilt.view
        {
            state.view = Some(rebuilt);
        }
    }

    /// Decodes one table of the embedded VertiPaq data (blocking).
    pub(crate) fn decode_table(
        &self,
        table: &str,
    ) -> (Result<RecordBatch, String>, Vec<Diagnostic>) {
        let mut diagnostics = Vec::new();
        let result = match &self.inner.opened.model_source {
            Some(ModelSource::Abf {
                archive, storage, ..
            }) => pbi_vertipaq::decode_table_with_diagnostics(
                archive,
                storage,
                &self.inner.base_model,
                table,
                &self.inner.limits.decode,
                &mut diagnostics,
            )
            .map_err(|e| e.to_string()),
            _ => Err("the document has no embedded data".to_owned()),
        };
        (result, diagnostics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_is_send_sync_and_cheap_to_clone() {
        fn assert_send_sync<T: Send + Sync + Clone>() {}
        assert_send_sync::<Session>();
    }
}
