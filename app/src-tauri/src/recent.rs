//! File → Recent: the most recently opened files and folders, newest first.
//!
//! The list lives in memory and, once [`RecentFiles::set_store`] names a file (the app config
//! folder's `recent-files.json`, set by `run()`), is loaded from and saved to it. Saving is
//! best-effort: a read-only config folder only loses the list. Tests never set a store, so they
//! never touch the user's config.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::dto::RecentFileDto;

/// Entries kept in the list.
pub const MAX_RECENT: usize = 20;
/// Largest `recent-files.json` read (bytes).
const MAX_STORE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    path: String,
    opened_at_ms: f64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    version: u32,
    #[serde(default)]
    files: Vec<Entry>,
}

#[derive(Debug, Default)]
pub struct RecentFiles {
    store: OnceLock<PathBuf>,
    entries: Mutex<Vec<Entry>>,
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

fn same_path(a: &str, b: &str) -> bool {
    if cfg!(any(windows, target_os = "macos")) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

impl RecentFiles {
    fn lock(&self) -> MutexGuard<'_, Vec<Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Persists the list to `path` (first call wins) and loads what it holds.
    pub fn set_store(&self, path: PathBuf) {
        let loaded = load(&path);
        if self.store.set(path).is_ok() {
            *self.lock() = loaded;
        }
    }

    /// Moves `path` to the top of the list.
    pub fn add(&self, path: &str) {
        let mut entries = self.lock();
        entries.retain(|e| !same_path(&e.path, path));
        entries.insert(
            0,
            Entry {
                path: path.to_owned(),
                opened_at_ms: now_ms(),
            },
        );
        entries.truncate(MAX_RECENT);
        self.save(&entries);
    }

    /// Removes `path`, or everything with `None`.
    pub fn remove(&self, path: Option<&str>) {
        let mut entries = self.lock();
        match path {
            Some(p) => entries.retain(|e| !same_path(&e.path, p)),
            None => entries.clear(),
        }
        self.save(&entries);
    }

    /// The list, newest first.
    pub fn list(&self) -> Vec<RecentFileDto> {
        self.lock()
            .iter()
            .map(|e| {
                let path = Path::new(&e.path);
                RecentFileDto {
                    file_name: path
                        .file_name()
                        .map_or_else(|| e.path.clone(), |n| n.to_string_lossy().into_owned()),
                    exists: path.exists(),
                    opened_at_ms: if e.opened_at_ms.is_finite() {
                        e.opened_at_ms
                    } else {
                        0.0
                    },
                    path: e.path.clone(),
                }
            })
            .collect()
    }

    fn save(&self, entries: &[Entry]) {
        let Some(path) = self.store.get() else {
            return;
        };
        let store = Store {
            version: 1,
            files: entries.to_vec(),
        };
        let Ok(json) = serde_json::to_vec_pretty(&store) else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Write then rename, so a crash never leaves a half-written list.
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

fn load(path: &Path) -> Vec<Entry> {
    let Ok(meta) = std::fs::metadata(path) else {
        return Vec::new();
    };
    if meta.len() > MAX_STORE_BYTES {
        return Vec::new();
    }
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Store>(&bytes).ok())
        .map(|s| {
            let mut files: Vec<Entry> = s
                .files
                .into_iter()
                .filter(|e| !e.path.trim().is_empty())
                .collect();
            files.truncate(MAX_RECENT);
            files
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_newest_first_without_duplicates_and_persists() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../target/pbiview-app-tests/recent-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store = dir.join("recent-files.json");

        let recent = RecentFiles::default();
        recent.set_store(store.clone());
        recent.add("/a/One.pbix");
        recent.add("/b/Two.pbip");
        recent.add("/a/One.pbix");
        let list = recent.list();
        assert_eq!(
            list.iter()
                .map(|r| r.file_name.as_str())
                .collect::<Vec<_>>(),
            ["One.pbix", "Two.pbip"]
        );
        assert!(!list[0].exists);
        assert!(list[0].opened_at_ms > 0.0);

        let reloaded = RecentFiles::default();
        reloaded.set_store(store.clone());
        assert_eq!(reloaded.list().len(), 2);
        reloaded.remove(Some("/b/Two.pbip"));
        assert_eq!(reloaded.list().len(), 1);
        reloaded.remove(None);
        assert!(reloaded.list().is_empty());

        for i in 0..(MAX_RECENT + 5) {
            recent.add(&format!("/f/{i}.pbix"));
        }
        assert_eq!(recent.list().len(), MAX_RECENT);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
