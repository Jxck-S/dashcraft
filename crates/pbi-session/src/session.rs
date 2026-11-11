//! The [`Session`] handle: an opened document, its engine state and the
//! last refresh results.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use arrow::record_batch::RecordBatch;
use pbi_core::{FileChange, ModelSource, OpenedDocument, ProjectFingerprint};
use pbi_engine::{Engine, EngineSnapshot};
use pbi_m::{BlobResolver, Clock, MapBlobResolver, NoBlobResolver, SystemClock};
use pbi_model::names::name_key;
use pbi_model::{
    Command, CommandError, Diagnostic, Document, History, HistoryResult, Model, ObjectId, TableKind,
};
use serde::{Deserialize, Serialize};

use crate::derived::{Applied, DerivedEntry, synthetic_column};
use crate::edit::{
    ModelRenames, RenameObject, RenamePreview, preview_rename, rename_results, structure_changed,
};
use crate::error::SessionError;
use crate::identifiers::{IdentifierGenerator, RandomIdentifiers, lineage_tags};
use crate::options::{SessionLimits, SessionOptions};
use crate::recalc::{Changes, recalculate, recompute_derived};
use crate::removed::{DataPlan, RemovedData};
use crate::report::{Recalculated, TableRefresh};
use crate::security::{ActiveView, ViewAs};

/// An edit ready to publish (see [`Session::execute_document_command`]).
struct PreparedEdit {
    /// `None` when the semantic model did not change (report-only edits
    /// keep the engine, the session model and the query caches).
    model: Option<ModelEdit>,
}

/// The session state that follows a changed semantic model.
struct ModelEdit {
    /// The re-synced engine, if tables, columns, relationships or roles
    /// changed.
    engine: Option<Engine>,
    /// The new session model (document model plus active derived columns).
    model: Arc<Model>,
    derived: Vec<DerivedEntry>,
    /// The per-table refresh results after the edit (renamed, without the
    /// deleted tables and columns).
    tables: HashMap<String, TableRefresh>,
    removed: RemovedData,
    /// The calculated tables and columns the edit recomputed.
    recalculated: Vec<Recalculated>,
    /// The active "View as" rebuilt for the edited engine and model,
    /// published with them so no query reads the new model over the
    /// previous restricted engine.
    view: Option<ActiveView>,
}

/// Mutable state: the published engine, the session model, the derived
/// columns and the last result per table.
#[derive(Debug)]
struct State {
    engine: Engine,
    document_revision: u64,
    /// The document model plus the synthetic entries of the active derived
    /// columns.
    model: Arc<Model>,
    /// Registered derived columns, in creation order.
    derived: Vec<DerivedEntry>,
    /// Last refresh / load result per table (by case-folded name).
    tables: HashMap<String, TableRefresh>,
    /// Data of deleted tables and columns, to undo the deletes.
    removed: RemovedData,
    /// "View as": the restricted engine queries read instead of `engine`.
    view: Option<ActiveView>,
    /// What the last published edit recalculated, with its revision.
    recalculated: (u64, Vec<Recalculated>),
}

impl State {
    /// The state of a freshly opened `model`: nothing loaded.
    fn fresh(model: Arc<Model>, revision: u64) -> Self {
        Self {
            engine: Engine::new(&model),
            document_revision: revision,
            model,
            derived: Vec::new(),
            tables: HashMap::new(),
            removed: RemovedData::default(),
            view: None,
            recalculated: (revision, Vec::new()),
        }
    }

    /// The engine queries read: the "View as" engine when one is active.
    fn query_engine(&self) -> &Engine {
        self.view.as_ref().map_or(&self.engine, |v| &v.engine)
    }
}

struct Inner {
    opened: RwLock<Arc<OpenedState>>,
    history: Mutex<History>,
    resolver: Arc<dyn BlobResolver>,
    blob_map: bool,
    clock: Arc<dyn Clock>,
    identifiers: Arc<dyn IdentifierGenerator>,
    limits: SessionLimits,
    state: RwLock<State>,
    refresh_lock: tokio::sync::Mutex<()>,
    saved: Mutex<SavedState>,
    /// Reader options of the open, reused to read back a Save As.
    open_options: pbi_core::OpenOptions,
}

/// The project as last opened or saved: the baseline of the next in-place
/// save ([`Session::save`]).
struct SavedState {
    opened: Arc<OpenedDocument>,
    /// Fingerprint of the project files (PBIP only).
    fingerprint: Option<ProjectFingerprint>,
}

struct OpenedState {
    opened: Arc<OpenedDocument>,
    document: Arc<Document>,
}

/// The fingerprint of a PBIP project's files, the baseline of save
/// conflicts and external changes (`None` for other formats, or when the
/// project cannot be scanned).
fn project_fingerprint(opened: &OpenedDocument) -> Option<ProjectFingerprint> {
    if opened.document.source.format != pbi_model::SourceFormat::Pbip {
        return None;
    }
    ProjectFingerprint::scan(opened)
        .map_err(|e| tracing::warn!("cannot fingerprint the project: {e}"))
        .ok()
}

/// What a query reads, captured at one semantic revision
/// ([`Session::revision_snapshot`]).
#[derive(Debug, Clone)]
pub struct RevisionSnapshot {
    /// The semantic document revision.
    pub revision: u64,
    /// The document (model, reports) of `revision`.
    pub document: Arc<Document>,
    /// The session model of `revision`: the document model plus the active
    /// derived columns.
    pub model: Arc<Model>,
    /// The engine snapshot queries read ("View as" applied).
    pub snapshot: Arc<EngineSnapshot>,
}

impl RevisionSnapshot {
    /// The model culture (`en-US` when unset).
    #[must_use]
    pub fn culture(&self) -> String {
        self.model
            .culture
            .clone()
            .filter(|c| !c.trim().is_empty())
            .unwrap_or_else(|| "en-US".to_owned())
    }
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
            .field("source", &self.opened().document.source.path)
            .field("model", &self.base_model().name)
            .field("tables", &self.base_model().tables.len())
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

/// One consistent snapshot of the document's revision and edit history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentHistory {
    pub revision: u64,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
}

/// What [`Session::save`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveReport {
    /// Files written, relative to the project folder.
    pub written: Vec<String>,
    /// Files deleted (e.g. the `.tmdl` file of a renamed table).
    pub deleted: Vec<String>,
    /// Files changed on disk by another program that the save overwrote
    /// or kept (only with `overwrite`).
    pub overwritten: Vec<FileChange>,
    /// The revision saved.
    pub revision: u64,
}

/// What [`Session::save_as`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveAsReport {
    /// The project folder written.
    pub project_dir: PathBuf,
    /// The `.pbip` file, relative to `project_dir`.
    pub pbip_file: String,
    /// Files written, relative to `project_dir`.
    pub written: Vec<String>,
    /// Bytes written.
    pub bytes: u64,
    /// Format of the document before the Save As.
    pub previous_format: pbi_model::SourceFormat,
    /// The source had imported data (PBIX VertiPaq storage) that the project
    /// does not carry: refresh it from its sources.
    pub data_not_carried: bool,
    /// The source project had a local data cache (`.pbi/cache.abf`) that was
    /// not copied.
    pub cache_not_copied: bool,
    /// The revision saved.
    pub revision: u64,
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
        let document = Arc::new(opened.document.clone());
        let fingerprint = project_fingerprint(&opened);
        let opened = Arc::new(opened);
        Ok(Session {
            inner: Arc::new(Inner {
                saved: Mutex::new(SavedState {
                    opened: Arc::clone(&opened),
                    fingerprint,
                }),
                opened: RwLock::new(Arc::new(OpenedState { opened, document })),
                history: Mutex::new(History::new()),
                resolver,
                blob_map,
                clock: options.clock.unwrap_or_else(|| Arc::new(SystemClock)),
                identifiers: options
                    .identifiers
                    .unwrap_or_else(|| Arc::new(RandomIdentifiers)),
                limits: options.limits,
                state: RwLock::new(State::fresh(model, 0)),
                refresh_lock: tokio::sync::Mutex::new(()),
                open_options: options.open,
            }),
        })
    }

    /// The opened document (model, reports, diagnostics, source info).
    #[must_use]
    pub fn document(&self) -> Arc<Document> {
        Arc::clone(
            &self
                .inner
                .opened
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .document,
        )
    }

    /// Everything the reader returned (detection, container, sources).
    #[must_use]
    pub fn opened(&self) -> Arc<OpenedDocument> {
        Arc::clone(
            &self
                .inner
                .opened
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .opened,
        )
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

    /// The document, the session model and the engine snapshot queries read,
    /// all of one revision (ADR-0012): read a page's report and query its
    /// data from one capture, so an edit committing meanwhile cannot mix
    /// two revisions.
    #[must_use]
    pub fn revision_snapshot(&self) -> RevisionSnapshot {
        // Edits publish with `opened` locked, then `state`: take them in
        // the same order.
        let opened = self
            .inner
            .opened
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        let state = self.read();
        RevisionSnapshot {
            revision: state.document_revision,
            document: Arc::clone(&opened.document),
            model: Arc::clone(&state.model),
            snapshot: state.query_engine().snapshot(),
        }
    }

    /// The semantic model as read from the document (empty if it has none),
    /// without session derived columns.
    #[must_use]
    pub fn base_model(&self) -> Arc<Model> {
        Arc::new(self.opened().document.model.clone().unwrap_or_default())
    }

    /// A lineage tag for an object about to be created, from the session's
    /// [`IdentifierGenerator`] and not in use in the model. `None` when the
    /// model has none at all (older projects): no tag is introduced there
    /// (ADR-0014). Put it in the object before building the create command,
    /// so undo, redo and later saves keep it.
    #[must_use]
    pub fn new_lineage_tag(&self) -> Option<String> {
        let document = self.document();
        let used = lineage_tags(document.model.as_ref()?);
        if used.is_empty() {
            return None;
        }
        (0..8)
            .map(|_| self.inner.identifiers.lineage_tag())
            .find(|tag| !used.contains(tag.as_str()))
    }

    /// A name for a relationship about to be created: a GUID from the
    /// session's [`IdentifierGenerator`], as Power BI Desktop names them,
    /// not in use by another relationship.
    #[must_use]
    pub fn new_relationship_name(&self) -> String {
        let document = self.document();
        let taken = |name: &str| {
            document.model.as_ref().is_some_and(|m| {
                m.relationships
                    .iter()
                    .any(|r| pbi_model::names_eq(&r.name, name))
            })
        };
        let mut name = self.inner.identifiers.lineage_tag();
        for _ in 0..8 {
            if !taken(&name) {
                break;
            }
            name = self.inner.identifiers.lineage_tag();
        }
        name
    }

    /// `true` if the document has a semantic model.
    #[must_use]
    pub fn has_model(&self) -> bool {
        self.opened().document.model.is_some()
    }

    /// `true` if the data comes from embedded VertiPaq storage (PBIX, ABF)
    /// rather than M partitions.
    #[must_use]
    pub fn is_pbix(&self) -> bool {
        matches!(self.opened().model_source, Some(ModelSource::Abf { .. }))
    }

    /// `true` if a blob map or custom resolver is configured.
    #[must_use]
    pub fn has_blob_map(&self) -> bool {
        self.inner.blob_map
    }

    /// The model culture (`en-US` when unset); used to format values.
    #[must_use]
    pub fn culture(&self) -> String {
        self.base_model()
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
        let base_model = self.base_model();
        let state = self.read();
        let snapshot = state.query_engine().snapshot();
        base_model
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
        let mut out = self.opened().document.diagnostics.clone();
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

    fn write_opened(&self) -> std::sync::RwLockWriteGuard<'_, Arc<OpenedState>> {
        self.inner
            .opened
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Applies a document command transactionally to the current revision.
    ///
    /// The edit is prepared on copies first (renamed tables and columns keep
    /// their loaded data, derived columns follow, the engine re-syncs only
    /// when tables, columns, relationships or roles changed) and published
    /// only if that succeeds.
    ///
    /// # Errors
    /// The command's validation error, or
    /// [`CommandError::Rejected`](pbi_model::CommandError::Rejected) when the
    /// loaded data cannot follow the edit (e.g. a rename onto the name of a
    /// derived column); the open document, data, history and revision are
    /// then unchanged.
    pub async fn execute_document_command(
        &self,
        command: impl Command + 'static,
    ) -> Result<HistoryResult, CommandError> {
        let _refresh = self.inner.refresh_lock.lock().await;
        self.edit_locked(true, |history, document, check| {
            history.execute_boxed_checked(document, Box::new(command), check)
        })
    }

    /// Renames a table, column or measure and rewrites the DAX that refers
    /// to it ([`RenameObject`]) as one undo step. Also returns what was
    /// rewritten and what was left (see [`preview_rename`]).
    ///
    /// # Errors
    /// See [`execute_document_command`](Self::execute_document_command).
    pub async fn rename_object(
        &self,
        object: ObjectId,
        new_name: &str,
    ) -> Result<(HistoryResult, RenamePreview), CommandError> {
        let _refresh = self.inner.refresh_lock.lock().await;
        let preview = preview_rename(&self.document(), object, new_name)?;
        let command = RenameObject {
            object,
            new_name: new_name.to_owned(),
        };
        let result = self.edit_locked(true, |history, document, check| {
            history.execute_boxed_checked(document, Box::new(command), check)
        })?;
        Ok((result, preview))
    }

    /// Undoes the most recent document edit.
    ///
    /// # Errors
    /// [`CommandError::NothingToUndo`](pbi_model::CommandError::NothingToUndo),
    /// or see [`execute_document_command`](Self::execute_document_command).
    pub async fn undo_document_command(&self) -> Result<HistoryResult, CommandError> {
        let _refresh = self.inner.refresh_lock.lock().await;
        self.edit_locked(false, |history, document, check| {
            history.undo_checked(document, check)
        })
    }

    /// Redoes the most recently undone document edit.
    ///
    /// # Errors
    /// [`CommandError::NothingToRedo`](pbi_model::CommandError::NothingToRedo),
    /// or see [`execute_document_command`](Self::execute_document_command).
    pub async fn redo_document_command(&self) -> Result<HistoryResult, CommandError> {
        let _refresh = self.inner.refresh_lock.lock().await;
        self.edit_locked(false, |history, document, check| {
            history.redo_checked(document, check)
        })
    }

    /// Runs one history step (with the refresh lock held) and publishes its
    /// result: `step` gets the history, a copy of the current document and
    /// the check that prepares the publication. With `strict`, a calculated
    /// table or column the step defines that cannot be calculated refuses
    /// it (undo and redo are lenient: see [`crate::recalc`]).
    fn edit_locked(
        &self,
        strict: bool,
        step: impl FnOnce(
            &mut History,
            &mut Document,
            &dyn Fn(&Document) -> Result<PreparedEdit, CommandError>,
        ) -> Result<(HistoryResult, Option<PreparedEdit>), CommandError>,
    ) -> Result<HistoryResult, CommandError> {
        let (result, rebuild_view) = {
            let mut opened = self.write_opened();
            let current = Arc::clone(&opened.document);
            let mut document = current.as_ref().clone();
            let mut history = self
                .inner
                .history
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let check = |next: &Document| self.prepare_edit(&current, next, strict);
            let (result, prepared) = step(&mut history, &mut document, &check)?;
            let rebuild_view =
                prepared.map(|p| self.publish_edit(&mut opened, document, result.revision, p));
            (result, rebuild_view)
        };
        if let Some(force) = rebuild_view {
            self.rebuild_view(force);
        }
        Ok(result)
    }

    /// Prepares publishing the edited document `next` in place of
    /// `current`, without changing anything.
    fn prepare_edit(
        &self,
        current: &Document,
        next: &Document,
        strict: bool,
    ) -> Result<PreparedEdit, CommandError> {
        let empty = Model::default();
        let old = current.model.as_ref().unwrap_or(&empty);
        let new = next.model.as_ref().unwrap_or(&empty);
        if old == new {
            return Ok(PreparedEdit { model: None });
        }
        let renames = ModelRenames::between(old, new);
        let changes = Changes::between(old, new, &renames);
        let state = self.read();
        renames.check_derived(&state.derived)?;
        let mut tables = state.tables.clone();
        let mut removed = state.removed.clone();
        let synced = structure_changed(old, new);
        let mut engine = if synced {
            let plan = DataPlan::between(old, new, &state.removed, state.engine.storage());
            let mut engine = state.engine.clone();
            let detached = engine
                .sync_model_with_edits(new, plan.edits(&state.removed, renames.storage()))
                .map_err(|e| {
                    CommandError::Rejected(format!("the loaded data cannot follow this edit: {e}"))
                })?;
            removed = plan.apply(&state.removed, detached, &mut tables, &renames);
            Some(engine)
        } else {
            rename_results(&mut tables, &renames);
            (!changes.is_empty()).then(|| state.engine.clone())
        };
        let mut derived = state.derived.clone();
        let view = state.view.as_ref().map(|v| v.view.clone());
        let base = state.engine.clone();
        drop(state);
        renames.apply_to_derived(&mut derived, new);
        let mut model = new.clone();
        for entry in derived.iter().filter(|e| e.info.active) {
            if let Some(t) = model.table_mut(&entry.info.table) {
                t.columns.push(synthetic_column(&entry.info, &entry.column));
            }
        }
        let mut recalculated = Vec::new();
        if let Some(data) = engine.as_mut() {
            recalculated = recalculate(data, old, new, &model, &changes, &mut tables, strict)?;
            let rederived =
                recompute_derived(data, new, &mut model, &mut derived, &recalculated, &changes);
            if !synced && recalculated.is_empty() && !rederived {
                engine = None;
            }
        }
        let view =
            view.map(|v| ActiveView::build_or_deny(engine.as_ref().unwrap_or(&base), &model, v));
        Ok(PreparedEdit {
            model: Some(ModelEdit {
                engine,
                model: Arc::new(model),
                derived,
                tables,
                removed,
                recalculated,
                view,
            }),
        })
    }

    /// Publishes a prepared edit with its active view. Returns whether the
    /// active view must still be rebuilt even if the engine did not change
    /// (the model did and the view changed meanwhile: role filters may use
    /// any of the model).
    fn publish_edit(
        &self,
        opened: &mut Arc<OpenedState>,
        document: Document,
        revision: u64,
        prepared: PreparedEdit,
    ) -> bool {
        let mut next_opened = (*opened.opened).clone();
        next_opened.document = document;
        let next_opened = Arc::new(next_opened);
        let document = Arc::new(next_opened.document.clone());
        let mut state = self.write();
        let mut rebuild = false;
        state.recalculated = (revision, Vec::new());
        if let Some(edit) = prepared.model {
            state.recalculated.1 = edit.recalculated;
            if let Some(engine) = edit.engine {
                state.engine = engine;
            }
            state.model = edit.model;
            state.derived = edit.derived;
            state.tables = edit.tables;
            state.removed = edit.removed;
            match edit.view {
                Some(view)
                    if view.base_generation == state.engine.generation()
                        && state.view.as_ref().is_some_and(|v| v.view == view.view) =>
                {
                    state.view = Some(view);
                }
                _ => rebuild = true,
            }
        }
        state.document_revision = revision;
        *opened = Arc::new(OpenedState {
            opened: next_opened,
            document,
        });
        rebuild
    }

    /// The calculated tables and columns the edit that produced `revision`
    /// recomputed over the loaded data, in computation order (empty when it
    /// recomputed nothing, or when `revision` is not the latest edit). A
    /// failed one has an error status and no values until an edit or a
    /// refresh computes it again.
    #[must_use]
    pub fn recalculated(&self, revision: u64) -> Vec<Recalculated> {
        let state = self.read();
        if state.recalculated.0 == revision {
            state.recalculated.1.clone()
        } else {
            Vec::new()
        }
    }

    /// Current semantic revision.
    #[must_use]
    pub fn document_revision(&self) -> u64 {
        self.inner
            .history
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .revision()
    }

    /// Returns revision, dirty state and undo/redo availability under one
    /// history lock.
    #[must_use]
    pub fn document_history(&self) -> DocumentHistory {
        let history = self
            .inner
            .history
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        DocumentHistory {
            revision: history.revision(),
            dirty: history.is_dirty(),
            can_undo: history.can_undo(),
            can_redo: history.can_redo(),
            undo_label: history.undo_label().map(str::to_owned),
            redo_label: history.redo_label().map(str::to_owned),
        }
    }

    /// Whether a document command can currently be undone.
    #[must_use]
    pub fn can_undo_document_command(&self) -> bool {
        self.inner
            .history
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .can_undo()
    }

    /// Whether a document command can currently be redone.
    #[must_use]
    pub fn can_redo_document_command(&self) -> bool {
        self.inner
            .history
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .can_redo()
    }

    /// Label of the next undo operation, if any.
    #[must_use]
    pub fn undo_document_label(&self) -> Option<String> {
        self.inner
            .history
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .undo_label()
            .map(str::to_owned)
    }

    /// Label of the next redo operation, if any.
    #[must_use]
    pub fn redo_document_label(&self) -> Option<String> {
        self.inner
            .history
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .redo_label()
            .map(str::to_owned)
    }

    /// Whether the current semantic document differs from its last save.
    #[must_use]
    pub fn is_document_dirty(&self) -> bool {
        self.inner
            .history
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_dirty()
    }

    /// Marks the current semantic revision as saved.
    pub fn mark_document_saved(&self) {
        self.inner
            .history
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .mark_saved();
    }

    /// Saves the current document into the PBIP project it was opened from
    /// ([`pbi_core::save_in_place`]): only the files that changed are
    /// written, report references follow renames, and files changed on disk
    /// by another program since the open or last save are a conflict unless
    /// `overwrite` is set. On success the revision is marked saved.
    ///
    /// # Errors
    /// [`SessionError::Open`] with [`pbi_core::CoreError::Conflict`] (nothing
    /// written), [`pbi_core::CoreError::Save`] (not a PBIP project, or
    /// changes that cannot be saved in place; nothing written),
    /// [`pbi_core::CoreError::PartialSave`] or an I/O error.
    pub async fn save(&self, overwrite: bool) -> Result<SaveReport, SessionError> {
        let handle = crate::refresh::runtime()?;
        // Edits take the refresh lock: the document cannot change meanwhile.
        let _refresh = self.inner.refresh_lock.lock().await;
        let document = self.document();
        let (baseline, mut fingerprint) = {
            let saved = self
                .inner
                .saved
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            (
                Arc::clone(&saved.opened),
                saved.fingerprint.clone().unwrap_or_default(),
            )
        };
        let current = Arc::clone(&document);
        let (result, fingerprint) = handle
            .spawn_blocking(move || {
                let result =
                    pbi_core::save_in_place(&baseline, &mut fingerprint, &current, overwrite);
                (result, fingerprint)
            })
            .await?;
        let mut opened = self.write_opened();
        let mut saved = self
            .inner
            .saved
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // Also after a failed save: it may have written part of the files.
        saved.fingerprint = Some(fingerprint);
        let outcome = result?;
        let mut history = self
            .inner
            .history
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        history.mark_saved();
        let report = SaveReport {
            written: outcome.written,
            deleted: outcome.deleted,
            overwritten: outcome.overwritten,
            revision: history.revision(),
        };
        let mut next = (*opened.opened).clone();
        next.model_source.clone_from(&outcome.saved.model_source);
        next.report_sources
            .clone_from(&outcome.saved.report_sources);
        saved.opened = Arc::new(outcome.saved);
        *opened = Arc::new(OpenedState {
            opened: Arc::new(next),
            document,
        });
        Ok(report)
    }

    /// The opened document with the current edits: what a save writes
    /// ([`pbi_core::edited_document`]). Exports use it to include unsaved
    /// edits.
    ///
    /// # Errors
    /// [`SessionError::Open`] with [`pbi_core::CoreError::Save`] when the
    /// reports have changes that cannot be written yet.
    pub fn edited_opened(&self) -> Result<OpenedDocument, SessionError> {
        let baseline = Arc::clone(
            &self
                .inner
                .saved
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .opened,
        );
        Ok(pbi_core::edited_document(&baseline, &self.document())?)
    }

    /// Saves the current document, edits included, as a new PBIP project
    /// in `out_dir` ([`pbi_core::save_as_project`]) and continues editing
    /// it there: later [`save`](Self::save)s write to the new project, the
    /// revision is marked saved and the undo history is kept. Works from
    /// any source format; a PBIX's imported data and a PBIP's local data
    /// cache are not carried over (the loaded data stays in memory).
    ///
    /// # Errors
    /// [`SessionError::Open`] with [`pbi_core::CoreError::Save`] (`out_dir`
    /// not empty without `overwrite`, or report changes that cannot be
    /// written yet; nothing written), [`pbi_core::CoreError::PartialSave`]
    /// or an I/O or reader error.
    pub async fn save_as(
        &self,
        out_dir: impl AsRef<Path>,
        overwrite: bool,
    ) -> Result<SaveAsReport, SessionError> {
        let handle = crate::refresh::runtime()?;
        let _refresh = self.inner.refresh_lock.lock().await;
        let baseline = Arc::clone(
            &self
                .inner
                .saved
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .opened,
        );
        let current = self.document();
        let previous_format = current.source.format.clone();
        let data_not_carried = self.is_pbix();
        let cache_not_copied = self.cache_info().is_some();
        let out_dir = out_dir.as_ref().to_path_buf();
        let options = self.inner.open_options;
        let dir = out_dir.clone();
        let outcome = handle
            .spawn_blocking(move || {
                pbi_core::save_as_project(&baseline, &current, &dir, overwrite, &options)
            })
            .await??;
        let mut opened = self.write_opened();
        let mut saved = self
            .inner
            .saved
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut history = self
            .inner
            .history
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        history.mark_saved();
        let saved_doc = Arc::new(outcome.saved);
        saved.opened = Arc::clone(&saved_doc);
        saved.fingerprint = Some(outcome.fingerprint);
        *opened = Arc::new(OpenedState {
            document: Arc::new(saved_doc.document.clone()),
            opened: saved_doc,
        });
        Ok(SaveAsReport {
            project_dir: out_dir,
            pbip_file: outcome.pbip_file,
            written: outcome.written,
            bytes: outcome.bytes,
            previous_format,
            data_not_carried,
            cache_not_copied,
            revision: history.revision(),
        })
    }

    /// The project files changed on disk by another program since the open
    /// or the last save: what the next [`save`](Self::save) would refuse as
    /// a conflict. `None` when nothing is tracked (not a PBIP project
    /// opened from a folder).
    ///
    /// # Errors
    /// [`SessionError::Open`] when the project cannot be scanned.
    pub async fn external_changes(&self) -> Result<Option<Vec<FileChange>>, SessionError> {
        let handle = crate::refresh::runtime()?;
        let (baseline, fingerprint) = {
            let saved = self
                .inner
                .saved
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            match &saved.fingerprint {
                Some(f) => (Arc::clone(&saved.opened), f.clone()),
                None => return Ok(None),
            }
        };
        let now = handle
            .spawn_blocking(move || ProjectFingerprint::scan(&baseline))
            .await??;
        Ok(Some(fingerprint.changes(&now)))
    }

    /// Rereads the document from `path` (where it was opened from or last
    /// saved as) in place of the open one, with the same blob resolver,
    /// clock and reader options. The edits and their undo history are
    /// dropped, and so are the loaded data, the derived columns and "View
    /// as" (refresh again); the revision advances to a new saved state, it
    /// never rewinds. If the document cannot be read nothing changes
    /// (ADR-0013). Returns the new revision.
    ///
    /// # Errors
    /// [`SessionError::Open`] when `path` cannot be read.
    pub async fn reload(&self, path: impl AsRef<Path>) -> Result<u64, SessionError> {
        let handle = crate::refresh::runtime()?;
        let _refresh = self.inner.refresh_lock.lock().await;
        let path = path.as_ref().to_path_buf();
        let options = self.inner.open_options;
        let (opened, fingerprint) = handle
            .spawn_blocking(move || {
                pbi_core::open_path(&path, &options).map(|opened| {
                    let fingerprint = project_fingerprint(&opened);
                    (opened, fingerprint)
                })
            })
            .await??;
        let model = Arc::new(opened.document.model.clone().unwrap_or_default());
        let document = Arc::new(opened.document.clone());
        let opened = Arc::new(opened);
        let mut current = self.write_opened();
        let mut saved = self
            .inner
            .saved
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut history = self
            .inner
            .history
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        history.reset();
        let revision = history.revision();
        *self.write() = State::fresh(model, revision);
        *saved = SavedState {
            opened: Arc::clone(&opened),
            fingerprint,
        };
        *current = Arc::new(OpenedState { opened, document });
        Ok(revision)
    }

    /// Reads the document at `path` with this session's reader options,
    /// without opening it: e.g. the files on disk to compare with the
    /// current revision ([`pbi_core::diff_documents`]).
    ///
    /// # Errors
    /// [`SessionError::Open`] when `path` cannot be read.
    pub async fn read_path(&self, path: impl AsRef<Path>) -> Result<Document, SessionError> {
        let handle = crate::refresh::runtime()?;
        let path = path.as_ref().to_path_buf();
        let options = self.inner.open_options;
        let opened = handle
            .spawn_blocking(move || pbi_core::open_path(&path, &options))
            .await??;
        Ok(opened.document)
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

    pub(crate) fn snapshot_model_and_revision(&self) -> (Arc<EngineSnapshot>, Arc<Model>, u64) {
        let state = self.read();
        (
            state.query_engine().snapshot(),
            Arc::clone(&state.model),
            state.document_revision,
        )
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
        self.rebuild_view(false);
    }

    /// [`resecure`](Self::resecure); with `force` also when the engine did
    /// not change (after a model edit: role filters may use any of it).
    fn rebuild_view(&self, force: bool) {
        let (base, model, view) = {
            let state = self.read();
            match &state.view {
                Some(v) if force || v.base_generation != state.engine.generation() => (
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
        let opened = self.opened();
        let model = self.base_model();
        let result = match &opened.model_source {
            Some(ModelSource::Abf {
                archive, storage, ..
            }) => pbi_vertipaq::decode_table_with_diagnostics(
                archive,
                storage,
                &model,
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
