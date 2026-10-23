//! Managed state: the command registry (the core catalog plus the opt-in `ai.*` commands) and
//! the workspace (shared with the CLI and agents through `pbi-ops`), plus app-only settings
//! (blob map, File → Recent) and the running `ops_execute` requests.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use pbi_ops::{OpError, Registry, Workspace};
use pbi_session::{CancellationToken, SessionOptions};

use crate::blob::BlobMap;
use crate::events::OpsProgress;
use crate::recent::RecentFiles;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A running `ops_execute` request.
#[derive(Debug)]
struct Request {
    /// The command id (`data.refresh`, …).
    command: String,
    cancel: CancellationToken,
}

/// Delivers `ops://progress` events to the webview (installed once the app is built).
#[derive(Clone)]
pub struct ProgressSink(Arc<dyn Fn(OpsProgress) + Send + Sync>);

impl ProgressSink {
    pub fn new(f: impl Fn(OpsProgress) + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }

    pub fn send(&self, progress: OpsProgress) {
        (self.0)(progress);
    }
}

impl fmt::Debug for ProgressSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProgressSink")
    }
}

/// The app's command catalog: `Registry::default_catalog()` plus the opt-in AI commands
/// (`pbi_ai::catalog()`, off until enabled in Options). `app/src/lib/ops.gen.ts` is generated
/// from exactly this catalog (`npm run gen:ops`).
pub fn app_registry() -> Registry {
    pbi_ai::catalog()
}

/// The app's state. Documents live in [`AppState::workspace`] (keyed by `docId`); the UI shows
/// one at a time and closes the previous one after opening another.
#[derive(Debug)]
pub struct AppState {
    /// The command catalog ([`app_registry`]: `docs/COMMANDS.md` plus the `ai.*` commands of
    /// `docs/AI.md`).
    pub registry: Registry,
    /// Open documents. Unrestricted path policy, as in the CLI: the user picks files through
    /// the OS dialog, drag and drop, the command line or Finder.
    pub workspace: Arc<Workspace>,
    /// Blob map shared by every document (set with `set_blob_map` or `DASHCRAFT_BLOB_MAP`).
    pub blob_map: BlobMap,
    /// File → Recent (persisted once `run()` names the store).
    pub recent: RecentFiles,
    requests: Mutex<HashMap<String, Request>>,
    progress: OnceLock<ProgressSink>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new(BlobMap::default())
    }
}

impl AppState {
    /// State whose blob map comes from `DASHCRAFT_BLOB_MAP` (or `PBIVIEW_BLOB_MAP`), if set.
    pub fn from_env() -> Self {
        Self::new(BlobMap::from_env())
    }

    fn new(blob_map: BlobMap) -> Self {
        let workspace = Workspace::new();
        // Documents resolve blobs through the app's switchable resolver, so `set_blob_map`
        // applies to the next refresh without reopening.
        let resolver = blob_map.resolver();
        workspace.set_open_hook(Some(Arc::new(move |options: &mut SessionOptions| {
            options.resolver = Some(Arc::clone(&resolver));
        })));
        Self {
            registry: app_registry(),
            workspace: Arc::new(workspace),
            blob_map,
            recent: RecentFiles::default(),
            requests: Mutex::new(HashMap::new()),
            progress: OnceLock::new(),
        }
    }

    /// Installs the progress sink (first call wins).
    pub fn set_progress_sink(&self, sink: ProgressSink) {
        let _ = self.progress.set(sink);
    }

    pub fn progress_sink(&self) -> Option<ProgressSink> {
        self.progress.get().cloned()
    }

    /// Registers request `id` running `command`; its cancellation token.
    ///
    /// # Errors
    /// `invalidParams` when the id is empty or already running.
    pub fn begin_request(&self, id: &str, command: &str) -> Result<CancellationToken, OpError> {
        if id.is_empty() {
            return Err(OpError::invalid_params("the request id is empty"));
        }
        let mut requests = lock(&self.requests);
        if requests.contains_key(id) {
            return Err(OpError::invalid_params(format!(
                "request {id} is already running"
            )));
        }
        let cancel = CancellationToken::new();
        requests.insert(
            id.to_owned(),
            Request {
                command: command.to_owned(),
                cancel: cancel.clone(),
            },
        );
        Ok(cancel)
    }

    pub fn end_request(&self, id: &str) {
        lock(&self.requests).remove(id);
    }

    /// Cancels request `id`; `false` if it is not running.
    pub fn cancel_request(&self, id: &str) -> bool {
        lock(&self.requests)
            .get(id)
            .map(|r| r.cancel.cancel())
            .is_some()
    }

    /// A request running `command` (e.g. `data.refresh`) exists.
    pub fn is_running(&self, command: &str) -> bool {
        lock(&self.requests).values().any(|r| r.command == command)
    }
}
