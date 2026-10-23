//! [`Workspace`]: the open documents and their per-document state, shared
//! by every command run.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};

use pbi_session::{
    RefreshEvent, RefreshOptions, RefreshReport, RefreshSource, Session, SessionOptions,
};

use crate::canvas::CanvasStore;
use crate::ctx::Ctx;
use crate::error::OpError;
use crate::path_policy::PathPolicy;

/// Default cap on open documents.
pub const DEFAULT_MAX_DOCUMENTS: usize = 32;

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Workspace-wide defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceSettings {
    /// Blob map used by `doc.open` when it gets none.
    pub blob_map: Option<PathBuf>,
}

/// How a document was opened; drives its refreshes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocOptions {
    /// Default data source of refreshes.
    pub source: RefreshSource,
    /// Data commands refresh the document once on first use when nothing
    /// was loaded yet.
    pub auto_refresh: bool,
    /// The blob map in use.
    pub blob_map: Option<PathBuf>,
    pub load_pbix_schema: bool,
    pub include_system_tables: bool,
    /// The pinned clock (RFC 3339) M sees as "now".
    pub now: Option<String>,
}

impl Default for DocOptions {
    fn default() -> Self {
        Self {
            source: RefreshSource::Auto,
            auto_refresh: true,
            blob_map: None,
            load_pbix_schema: true,
            include_system_tables: false,
            now: None,
        }
    }
}

/// One open document: its [`Session`] plus registry-side state (data
/// generation, canvas view state, ordering lock).
pub struct DocEntry {
    id: String,
    path: PathBuf,
    shown_path: String,
    session: Session,
    options: DocOptions,
    /// Bumped by every data change (refresh, CSV load, derived columns).
    generation: AtomicU64,
    /// Data was loaded (explicitly or by the automatic first refresh).
    loaded: tokio::sync::Mutex<bool>,
    /// Orders the mutating commands of this document.
    op_lock: Arc<tokio::sync::Mutex<()>>,
    pub(crate) canvas: CanvasStore,
}

impl fmt::Debug for DocEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DocEntry")
            .field("id", &self.id)
            .field("path", &self.path)
            .field("generation", &self.generation())
            .finish_non_exhaustive()
    }
}

impl DocEntry {
    /// The document id (`d1`, `d2`, …).
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The path the document was opened from (after the path policy).
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The path as the caller gave it.
    #[must_use]
    pub fn shown_path(&self) -> &str {
        &self.shown_path
    }

    /// The data session.
    #[must_use]
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// How it was opened.
    #[must_use]
    pub fn options(&self) -> &DocOptions {
        &self.options
    }

    /// Data generation: changes after every refresh, CSV load or derived
    /// column change.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    pub(crate) fn bump_generation(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// The lock that orders mutating commands of this document.
    #[must_use]
    pub fn op_lock(&self) -> Arc<tokio::sync::Mutex<()>> {
        Arc::clone(&self.op_lock)
    }

    /// `true` once data was loaded.
    pub async fn is_loaded(&self) -> bool {
        *self.loaded.lock().await
    }

    /// Marks the data as loaded (after a CSV load).
    pub(crate) async fn mark_loaded(&self) {
        *self.loaded.lock().await = true;
        self.bump_generation();
    }

    /// Refreshes the document once if it has a model, was never loaded and
    /// was opened with `autoRefresh`. Returns the refresh report when it
    /// refreshed.
    ///
    /// # Errors
    /// `cancelled`, or a refresh that fails as a whole.
    pub async fn ensure_loaded(&self, ctx: &Ctx) -> Result<Option<RefreshReport>, OpError> {
        if !self.session.has_model() || !self.options.auto_refresh {
            return Ok(None);
        }
        let mut loaded = tokio::select! {
            biased;
            () = ctx.cancel_token().cancelled() => return Err(OpError::cancelled()),
            guard = self.loaded.lock() => guard,
        };
        if *loaded {
            return Ok(None);
        }
        ctx.progress(
            format!("loading the data of {} (first use)", self.id),
            Some(0.0),
        );
        let report = self
            .refresh_unlocked(None, self.options.source, ctx)
            .await?;
        *loaded = true;
        Ok(Some(report))
    }

    /// Refreshes `tables` (all when `None`) from `source`, reporting
    /// progress to `ctx`.
    ///
    /// # Errors
    /// `cancelled`, `notFound` without a cache for `cache`, `internal`.
    pub async fn refresh(
        &self,
        tables: Option<Vec<String>>,
        source: RefreshSource,
        ctx: &Ctx,
    ) -> Result<RefreshReport, OpError> {
        let mut loaded = tokio::select! {
            biased;
            () = ctx.cancel_token().cancelled() => return Err(OpError::cancelled()),
            guard = self.loaded.lock() => guard,
        };
        let report = self.refresh_unlocked(tables, source, ctx).await?;
        *loaded = true;
        Ok(report)
    }

    async fn refresh_unlocked(
        &self,
        tables: Option<Vec<String>>,
        source: RefreshSource,
        ctx: &Ctx,
    ) -> Result<RefreshReport, OpError> {
        let total = Arc::new(AtomicUsize::new(0));
        let done = Arc::new(AtomicUsize::new(0));
        let progress_ctx = ctx.clone();
        let options = RefreshOptions {
            tables,
            cancel: ctx.cancel_token().child_token(),
            source,
            ..RefreshOptions::default()
        }
        .with_progress(move |event| {
            let (message, fraction) = match &event {
                RefreshEvent::Started { tables } => {
                    total.store(tables.len(), Ordering::Relaxed);
                    (format!("refreshing {} table(s)", tables.len()), Some(0.0))
                }
                RefreshEvent::TableStarted { name, .. } => (format!("{name}: loading"), None),
                RefreshEvent::TableFinished { table } => {
                    let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                    let t = total.load(Ordering::Relaxed).max(1);
                    #[allow(clippy::cast_precision_loss)]
                    let f = n as f64 / t as f64;
                    (
                        format!(
                            "{}: {} rows in {} ms",
                            table.name, table.rows, table.duration_ms
                        ),
                        Some(f.min(0.99)),
                    )
                }
                RefreshEvent::ColumnStarted { table, column } => {
                    (format!("{table}[{column}]: computing"), None)
                }
                RefreshEvent::ColumnFinished { table, column } => (
                    format!("{table}[{}]: {} ms", column.name, column.duration_ms),
                    None,
                ),
                RefreshEvent::Finished { duration_ms, .. } => {
                    (format!("refresh finished in {duration_ms} ms"), Some(1.0))
                }
            };
            progress_ctx.progress_with(message, fraction, serde_json::to_value(&event).ok());
        });
        let report = self.session.refresh(options).await?;
        self.bump_generation();
        Ok(report)
    }
}

/// Adjusts the [`SessionOptions`] of `doc.open` runs without an explicit
/// `blobMap` (see [`Workspace::set_open_hook`]).
pub type OpenHook = Arc<dyn Fn(&mut SessionOptions) + Send + Sync>;

/// The open documents and workspace settings. `Send + Sync`; share it in an
/// `Arc` between concurrent command runs.
pub struct Workspace {
    docs: Mutex<BTreeMap<u64, Arc<DocEntry>>>,
    next_id: AtomicU64,
    settings: RwLock<WorkspaceSettings>,
    policy: PathPolicy,
    max_documents: usize,
    open_hook: RwLock<Option<OpenHook>>,
}

impl fmt::Debug for Workspace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Workspace")
            .field("documents", &lock(&self.docs).len())
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

impl Workspace {
    /// An empty workspace with an unrestricted path policy (CLI default).
    #[must_use]
    pub fn new() -> Self {
        Self::with_policy(PathPolicy::unrestricted())
    }

    /// An empty workspace with `policy`.
    #[must_use]
    pub fn with_policy(policy: PathPolicy) -> Self {
        Self {
            docs: Mutex::new(BTreeMap::new()),
            next_id: AtomicU64::new(1),
            settings: RwLock::new(WorkspaceSettings::default()),
            policy,
            max_documents: DEFAULT_MAX_DOCUMENTS,
            open_hook: RwLock::new(None),
        }
    }

    /// Caps the number of open documents.
    #[must_use]
    pub fn with_max_documents(mut self, max: usize) -> Self {
        self.max_documents = max.max(1);
        self
    }

    /// The path policy every command applies.
    #[must_use]
    pub fn policy(&self) -> &PathPolicy {
        &self.policy
    }

    /// A copy of the settings.
    #[must_use]
    pub fn settings(&self) -> WorkspaceSettings {
        self.settings
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Sets the default blob map (checked by the path policy).
    ///
    /// # Errors
    /// `pathDenied` outside the read roots.
    pub fn set_default_blob_map(&self, path: Option<&Path>) -> Result<(), OpError> {
        let checked = path.map(|p| self.policy.check_blob_map(p)).transpose()?;
        self.settings
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .blob_map = checked;
        Ok(())
    }

    /// Lets the host adjust the session options of every `doc.open` that
    /// gets no `blobMap`, after the command set its own: the desktop app
    /// installs its switchable blob resolver this way, so changing the blob
    /// map applies to the next refresh without reopening the document. The
    /// host is trusted: what the hook installs is not checked by the
    /// [`PathPolicy`] (restricted hosts such as agents should not set one).
    pub fn set_open_hook(&self, hook: Option<OpenHook>) {
        *self
            .open_hook
            .write()
            .unwrap_or_else(PoisonError::into_inner) = hook;
    }

    /// Applies the [open hook](Self::set_open_hook), if any.
    pub(crate) fn apply_open_hook(&self, options: &mut SessionOptions) {
        let hook = self
            .open_hook
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(hook) = hook {
            hook(options);
        }
    }

    /// Adds an opened session as a new document.
    ///
    /// # Errors
    /// `busy` when the workspace is full.
    pub fn insert(
        &self,
        session: Session,
        path: PathBuf,
        shown_path: String,
        options: DocOptions,
    ) -> Result<Arc<DocEntry>, OpError> {
        let mut docs = lock(&self.docs);
        if docs.len() >= self.max_documents {
            return Err(OpError::busy(format!(
                "{} documents are open (the maximum); close one with doc.close",
                docs.len()
            )));
        }
        let n = self.next_id.fetch_add(1, Ordering::Relaxed);
        let entry = Arc::new(DocEntry {
            id: format!("d{n}"),
            path,
            shown_path,
            session,
            options,
            generation: AtomicU64::new(0),
            loaded: tokio::sync::Mutex::new(false),
            op_lock: Arc::new(tokio::sync::Mutex::new(())),
            canvas: CanvasStore::default(),
        });
        docs.insert(n, Arc::clone(&entry));
        Ok(entry)
    }

    fn key(id: &str) -> Option<u64> {
        id.trim().strip_prefix('d')?.parse().ok()
    }

    /// The document `id`, if open.
    #[must_use]
    pub fn find(&self, id: &str) -> Option<Arc<DocEntry>> {
        let key = Self::key(id)?;
        lock(&self.docs).get(&key).cloned()
    }

    /// The document `id`.
    ///
    /// # Errors
    /// `noDocument` (listing the open documents).
    pub fn get(&self, id: &str) -> Result<Arc<DocEntry>, OpError> {
        self.find(id).ok_or_else(|| {
            let open: Vec<String> = lock(&self.docs).values().map(|d| d.id.clone()).collect();
            OpError::no_document(format!(
                "no open document '{id}'{}",
                if open.is_empty() {
                    "; open one with doc.open".to_owned()
                } else {
                    format!("; open documents: {}", open.join(", "))
                }
            ))
        })
    }

    /// Removes the document `id`; its running canvas renders are cancelled.
    #[must_use]
    pub fn remove(&self, id: &str) -> Option<Arc<DocEntry>> {
        let key = Self::key(id)?;
        let entry = lock(&self.docs).remove(&key);
        if let Some(e) = &entry {
            e.canvas.cancel_all();
        }
        entry
    }

    /// The open documents, in opening order.
    #[must_use]
    pub fn documents(&self) -> Vec<Arc<DocEntry>> {
        lock(&self.docs).values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Workspace>();
        assert_send_sync::<DocEntry>();
    }

    #[test]
    fn doc_ids_parse() {
        assert_eq!(Workspace::key("d12"), Some(12));
        assert_eq!(Workspace::key("x1"), None);
        assert_eq!(Workspace::key("d"), None);
        let ws = Workspace::new();
        let err = ws.get("d1").err().map(|e| e.code);
        assert_eq!(err, Some(crate::OpErrorCode::NoDocument));
    }
}
