//! Offline `AzureStorage.Blobs` access: a blob map (`blob-map.json`, local files only) that can be
//! swapped at runtime. Sessions are opened with [`SwitchableResolver`], so `set_blob_map` applies
//! to the next refresh without reopening the document or dropping loaded data.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};

use pbi_m::{BlobEntry, BlobResolver, MError, MapBlobResolver, NoBlobResolver, ResolverError};

use crate::error::AppError;

/// Environment variable naming a blob map to use from startup.
pub const BLOB_MAP_ENV: &str = "DASHCRAFT_BLOB_MAP";
/// Former name of [`BLOB_MAP_ENV`] (before the rename to DashCraft), honoured when the new one
/// is not set.
pub const BLOB_MAP_ENV_ALIAS: &str = "PBIVIEW_BLOB_MAP";

/// The blob map named by the environment: [`BLOB_MAP_ENV`], else [`BLOB_MAP_ENV_ALIAS`] (empty
/// values are ignored), with the name of the variable that named it.
fn env_blob_map(var: impl Fn(&str) -> Option<OsString>) -> Option<(&'static str, PathBuf)> {
    [BLOB_MAP_ENV, BLOB_MAP_ENV_ALIAS]
        .into_iter()
        .find_map(|name| {
            var(name)
                .filter(|v| !v.is_empty())
                .map(|v| (name, PathBuf::from(v)))
        })
}

/// A [`BlobResolver`] that delegates to a replaceable inner resolver (a [`MapBlobResolver`] or
/// the offline [`NoBlobResolver`]).
#[derive(Debug)]
pub struct SwitchableResolver {
    inner: RwLock<Arc<dyn BlobResolver>>,
}

impl Default for SwitchableResolver {
    fn default() -> Self {
        Self {
            inner: RwLock::new(Arc::new(NoBlobResolver)),
        }
    }
}

impl SwitchableResolver {
    fn current(&self) -> Arc<dyn BlobResolver> {
        Arc::clone(&self.inner.read().unwrap_or_else(PoisonError::into_inner))
    }

    fn set(&self, resolver: Arc<dyn BlobResolver>) {
        *self.inner.write().unwrap_or_else(PoisonError::into_inner) = resolver;
    }
}

impl BlobResolver for SwitchableResolver {
    fn list(&self, container_url: &str) -> Result<Vec<BlobEntry>, ResolverError> {
        self.current().list(container_url)
    }

    fn open(&self, container_url: &str, name: &str) -> Result<Vec<u8>, ResolverError> {
        self.current().open(container_url, name)
    }

    fn open_limited(
        &self,
        container_url: &str,
        name: &str,
        max_bytes: u64,
    ) -> Result<Vec<u8>, MError> {
        self.current().open_limited(container_url, name, max_bytes)
    }
}

/// The configured blob map.
#[derive(Debug, Clone, Default)]
pub struct BlobMapSetting {
    /// Absolute path of the active `blob-map.json`, if any.
    pub path: Option<PathBuf>,
    /// The environment variable the active map came from ([`BLOB_MAP_ENV`] or
    /// [`BLOB_MAP_ENV_ALIAS`]); `None` when it was chosen in the app.
    pub env_var: Option<&'static str>,
    /// Why the map named by the environment could not be used.
    pub error: Option<String>,
}

/// The app's blob map: the setting plus the resolver shared by every session.
#[derive(Debug, Default)]
pub struct BlobMap {
    resolver: Arc<SwitchableResolver>,
    setting: RwLock<BlobMapSetting>,
}

impl BlobMap {
    /// Starts with the map named by `DASHCRAFT_BLOB_MAP` (or the legacy `PBIVIEW_BLOB_MAP`), if
    /// set and readable (otherwise offline, with the reason kept in [`BlobMapSetting::error`]).
    pub fn from_env() -> Self {
        let map = Self::default();
        if let Some((name, path)) = env_blob_map(|n| std::env::var_os(n)) {
            match map.set(Some(&path)) {
                Ok(_) => {
                    map.write().env_var = Some(name);
                }
                Err(e) => {
                    map.write().error = Some(format!("{name}={}: {e}", path.display()));
                }
            }
        }
        map
    }

    /// The resolver to pass to `SessionOptions::resolver`.
    pub fn resolver(&self) -> Arc<dyn BlobResolver> {
        Arc::clone(&self.resolver) as Arc<dyn BlobResolver>
    }

    pub fn setting(&self) -> BlobMapSetting {
        self.setting
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, BlobMapSetting> {
        self.setting.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Reads and activates the map at `path` (relative entries resolve against the map's folder),
    /// or goes offline with `None`. An unreadable map leaves the current one active.
    pub fn set(&self, path: Option<&Path>) -> Result<BlobMapSetting, AppError> {
        let Some(path) = path else {
            self.resolver.set(Arc::new(NoBlobResolver));
            *self.write() = BlobMapSetting::default();
            return Ok(BlobMapSetting::default());
        };
        if path.as_os_str().is_empty() {
            return Err(AppError::InvalidArgument("path is empty".to_owned()));
        }
        let abs = std::path::absolute(path).map_err(|e| AppError::from_io(path, &e))?;
        let meta = std::fs::metadata(&abs).map_err(|e| AppError::from_io(&abs, &e))?;
        if !meta.is_file() {
            return Err(AppError::InvalidArgument(format!(
                "{} is not a file",
                abs.display()
            )));
        }
        let map = MapBlobResolver::from_map_file(&abs)
            .map_err(|e| AppError::InvalidFile(format!("blob map {}: {e}", abs.display())))?;
        self.resolver.set(Arc::new(map));
        let setting = BlobMapSetting {
            path: Some(abs),
            env_var: None,
            error: None,
        };
        *self.write() = setting.clone();
        Ok(setting)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dashcraft_variable_wins_over_the_legacy_one() {
        let both = |n: &str| match n {
            "DASHCRAFT_BLOB_MAP" => Some(OsString::from("new.json")),
            "PBIVIEW_BLOB_MAP" => Some(OsString::from("old.json")),
            _ => None,
        };
        assert_eq!(
            env_blob_map(both),
            Some(("DASHCRAFT_BLOB_MAP", PathBuf::from("new.json")))
        );
        let legacy = |n: &str| (n == "PBIVIEW_BLOB_MAP").then(|| OsString::from("old.json"));
        assert_eq!(
            env_blob_map(legacy),
            Some(("PBIVIEW_BLOB_MAP", PathBuf::from("old.json")))
        );
        let empty = |_: &str| Some(OsString::new());
        assert_eq!(env_blob_map(empty), None);
    }
}
