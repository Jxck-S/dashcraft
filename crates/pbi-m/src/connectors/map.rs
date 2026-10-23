//! [`MapBlobResolver`]: offline blob access through `blob-map.json`.

use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Utc};
use indexmap::IndexMap;

use super::{BlobEntry, BlobResolver, normalize_container_url};
use crate::merror::{LimitKind, MError, ResolverError, ResolverErrorKind};

/// Maximum size of a `blob-map.json` file.
const MAX_MAP_BYTES: u64 = 16 * 1024 * 1024;

/// Serves `AzureStorage.Blobs` from local files.
///
/// The map file has the form
/// `{ "containers": { "<container url>": { "<blob name>": "<relative path>" } } }`; paths are
/// relative to the map file's directory. Absolute paths, `..` components and (at open time)
/// symlinks resolving outside that directory are rejected.
#[derive(Debug, Clone)]
pub struct MapBlobResolver {
    base: PathBuf,
    containers: IndexMap<String, IndexMap<String, PathBuf>>,
}

fn invalid(message: impl Into<String>) -> ResolverError {
    ResolverError::new(ResolverErrorKind::Invalid, message)
}

/// Validates a mapped path: relative, no `..`, no drive or root.
fn checked_relative(rel: &str) -> Result<PathBuf, ResolverError> {
    let normalized = rel.replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.as_bytes().get(1) == Some(&b':')
    {
        return Err(invalid(format!(
            "blob-map path '{rel}' must be a relative path inside the map directory"
        )));
    }
    let path = PathBuf::from(&normalized);
    for c in path.components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            _ => {
                return Err(invalid(format!(
                    "blob-map path '{rel}' must not contain '..' or absolute components"
                )));
            }
        }
    }
    Ok(path)
}

impl MapBlobResolver {
    /// An empty map whose relative paths resolve against `base_dir`.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base: base_dir.into(),
            containers: IndexMap::new(),
        }
    }

    /// Reads a `blob-map.json` file.
    pub fn from_map_file(path: impl AsRef<Path>) -> Result<Self, ResolverError> {
        let path = path.as_ref();
        let io = |e: std::io::Error| {
            ResolverError::new(
                ResolverErrorKind::Io,
                format!("cannot read blob map {}: {e}", path.display()),
            )
        };
        let file = fs::File::open(path).map_err(io)?;
        let mut json = String::new();
        file.take(MAX_MAP_BYTES + 1)
            .read_to_string(&mut json)
            .map_err(io)?;
        if json.len() as u64 > MAX_MAP_BYTES {
            return Err(invalid(format!(
                "blob map {} is larger than {MAX_MAP_BYTES} bytes",
                path.display()
            )));
        }
        let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
        Self::from_json(&json, base)
    }

    /// Parses blob-map JSON with paths relative to `base_dir`.
    pub fn from_json(json: &str, base_dir: impl Into<PathBuf>) -> Result<Self, ResolverError> {
        let root: serde_json::Value = serde_json::from_str(json)
            .map_err(|e| invalid(format!("blob map is not valid JSON: {e}")))?;
        let containers = root
            .get("containers")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| invalid("blob map must have a \"containers\" object"))?;
        let mut map = Self::new(base_dir);
        for (url, blobs) in containers {
            let blobs = blobs
                .as_object()
                .ok_or_else(|| invalid(format!("blob map entry for '{url}' must be an object")))?;
            for (name, rel) in blobs {
                let rel = rel.as_str().ok_or_else(|| {
                    invalid(format!("blob map path for '{name}' must be a string"))
                })?;
                map.insert(url, name, rel)?;
            }
        }
        Ok(map)
    }

    /// Maps blob `name` of `container_url` to `relative_path`.
    pub fn insert(
        &mut self,
        container_url: &str,
        name: &str,
        relative_path: &str,
    ) -> Result<(), ResolverError> {
        let rel = checked_relative(relative_path)?;
        self.containers
            .entry(normalize_container_url(container_url))
            .or_default()
            .insert(name.to_owned(), rel);
        Ok(())
    }

    /// The mapped container URLs.
    pub fn container_urls(&self) -> impl Iterator<Item = &str> {
        self.containers.keys().map(String::as_str)
    }

    fn container(&self, url: &str) -> Result<&IndexMap<String, PathBuf>, ResolverError> {
        self.containers
            .get(&normalize_container_url(url))
            .ok_or_else(|| {
                let known: Vec<&str> = self.container_urls().collect();
                ResolverError::new(
                    ResolverErrorKind::NotFound,
                    format!(
                        "container '{url}' is not mapped in blob-map.json (mapped containers: {})",
                        if known.is_empty() {
                            "none".to_owned()
                        } else {
                            known.join(", ")
                        }
                    ),
                )
            })
    }

    /// The local file for a blob, verified to be inside the map directory.
    fn file(&self, url: &str, name: &str) -> Result<PathBuf, ResolverError> {
        let rel = self.container(url)?.get(name).ok_or_else(|| {
            ResolverError::new(
                ResolverErrorKind::NotFound,
                format!("blob '{name}' is not mapped for container '{url}' in blob-map.json"),
            )
        })?;
        let path = self.base.join(rel);
        let not_found = |e: std::io::Error| {
            ResolverError::new(
                if e.kind() == std::io::ErrorKind::NotFound {
                    ResolverErrorKind::NotFound
                } else {
                    ResolverErrorKind::Io
                },
                format!(
                    "mapped file '{}' for blob '{name}' cannot be opened: {e}",
                    rel.display()
                ),
            )
        };
        let canonical = fs::canonicalize(&path).map_err(not_found)?;
        let base = fs::canonicalize(if self.base.as_os_str().is_empty() {
            Path::new(".")
        } else {
            &self.base
        })
        .map_err(not_found)?;
        if !canonical.starts_with(&base) {
            return Err(invalid(format!(
                "mapped file '{}' resolves outside the blob-map directory",
                rel.display()
            )));
        }
        if !canonical.is_file() {
            return Err(invalid(format!(
                "mapped path '{}' is not a file",
                rel.display()
            )));
        }
        Ok(canonical)
    }
}

impl BlobResolver for MapBlobResolver {
    fn list(&self, container_url: &str) -> Result<Vec<BlobEntry>, ResolverError> {
        let blobs = self.container(container_url)?;
        Ok(blobs
            .iter()
            .map(|(name, rel)| {
                let meta = fs::metadata(self.base.join(rel)).ok();
                BlobEntry {
                    name: name.clone(),
                    size: meta.as_ref().map(fs::Metadata::len),
                    modified: meta
                        .and_then(|m| m.modified().ok())
                        .map(DateTime::<Utc>::from),
                }
            })
            .collect())
    }

    fn open(&self, container_url: &str, name: &str) -> Result<Vec<u8>, ResolverError> {
        let path = self.file(container_url, name)?;
        fs::read(&path).map_err(|e| {
            ResolverError::new(
                ResolverErrorKind::Io,
                format!("cannot read blob '{name}': {e}"),
            )
        })
    }

    fn open_limited(
        &self,
        container_url: &str,
        name: &str,
        max_bytes: u64,
    ) -> Result<Vec<u8>, MError> {
        let path = self.file(container_url, name)?;
        let io = |e: std::io::Error| {
            MError::Resolver(ResolverError::new(
                ResolverErrorKind::Io,
                format!("cannot read blob '{name}': {e}"),
            ))
        };
        let file = fs::File::open(&path).map_err(io)?;
        if file.metadata().map_err(io)?.len() > max_bytes {
            return Err(MError::limit(LimitKind::Bytes, max_bytes));
        }
        let mut bytes = Vec::new();
        file.take(max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(io)?;
        if bytes.len() as u64 > max_bytes {
            return Err(MError::limit(LimitKind::Bytes, max_bytes));
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_paths() {
        for bad in [
            "/etc/passwd",
            "../secret.csv",
            "csv/../../x",
            "C:\\x.csv",
            "a\\..\\..\\b",
            "",
        ] {
            assert!(checked_relative(bad).is_err(), "{bad}");
        }
        assert!(checked_relative("csv/a.csv").is_ok());
        assert!(checked_relative("./csv\\a.csv").is_ok());
    }

    #[test]
    fn parses_map_json() {
        let json = r#"{"$comment": "x", "containers": {"HTTPS://Acct.blob.core.windows.net/c/": {"a/b.csv": "csv/b.csv"}}}"#;
        let r = MapBlobResolver::from_json(json, "base").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            r.container_urls().collect::<Vec<_>>(),
            vec!["https://acct.blob.core.windows.net/c"]
        );
        let entries = r
            .list("https://acct.blob.core.windows.net/c")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "a/b.csv");
        assert!(r.list("https://other/c").is_err());
        assert!(
            MapBlobResolver::from_json(r#"{"containers": {"u": {"x": "../y"}}}"#, ".").is_err()
        );
        assert!(MapBlobResolver::from_json("[]", ".").is_err());
    }
}
