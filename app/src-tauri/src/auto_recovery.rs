//! File → Options → Auto recovery: whether the unsaved edits of open documents are journaled
//! (`pbi_ops::recovery`, ADR-0013 §5) and where. On by default, as in Power BI Desktop; the
//! journals live in the per-user data folder (`RecoveryStore::default_location`), never in a
//! project. The choice persists in the app config folder's `auto-recovery.json` once `run()`
//! names it; tests never do, so they never touch the user's settings or journals.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use pbi_ops::Workspace;
use pbi_ops::recovery::RecoveryStore;
use serde::{Deserialize, Serialize};

use crate::dto::AutoRecoveryDto;
use crate::error::AppError;

/// Largest `auto-recovery.json` read (bytes).
const MAX_SETTING_BYTES: u64 = 64 * 1024;

#[derive(Debug, Serialize, Deserialize)]
struct Setting {
    version: u32,
    enabled: bool,
}

#[derive(Debug, Default)]
struct Paths {
    /// `auto-recovery.json`.
    setting: Option<PathBuf>,
    /// The journal folder.
    journals: Option<PathBuf>,
}

/// The Auto recovery setting.
#[derive(Debug, Default)]
pub struct AutoRecovery {
    paths: Mutex<Paths>,
}

impl AutoRecovery {
    fn lock(&self) -> MutexGuard<'_, Paths> {
        self.paths.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Journals to `journals` (when known) unless `setting` says it was turned off.
    pub fn install(&self, ws: &Workspace, setting: Option<PathBuf>, journals: Option<PathBuf>) {
        let enabled = setting.as_deref().and_then(load).unwrap_or(true);
        let mut paths = self.lock();
        *paths = Paths { setting, journals };
        ws.set_recovery(
            paths
                .journals
                .clone()
                .filter(|_| enabled)
                .map(RecoveryStore::new),
        );
    }

    /// The current setting.
    pub fn get(&self, ws: &Workspace) -> AutoRecoveryDto {
        let paths = self.lock();
        AutoRecoveryDto {
            enabled: ws.recovery().is_some(),
            available: paths.journals.is_some(),
            location: paths
                .journals
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
        }
    }

    /// Turns journaling on or off and remembers the choice. Turning it off deletes the
    /// journals of open documents.
    ///
    /// # Errors
    /// `unsupported` when there is no per-user data folder to journal to.
    pub fn set(&self, ws: &Workspace, enabled: bool) -> Result<AutoRecoveryDto, AppError> {
        {
            let paths = self.lock();
            let Some(journals) = &paths.journals else {
                return Err(AppError::Unsupported(
                    "there is no per-user data folder for recovery files".to_owned(),
                ));
            };
            ws.set_recovery(enabled.then(|| RecoveryStore::new(journals)));
            if let Some(path) = &paths.setting {
                save(path, enabled);
            }
        }
        Ok(self.get(ws))
    }
}

fn load(path: &Path) -> Option<bool> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_SETTING_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice::<Setting>(&bytes)
        .ok()
        .map(|s| s.enabled)
}

/// Best-effort: a read-only config folder only forgets the choice.
fn save(path: &Path, enabled: bool) {
    let Ok(json) = serde_json::to_vec_pretty(&Setting {
        version: 1,
        enabled,
    }) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Write then rename, so a crash never leaves a half-written setting.
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}
