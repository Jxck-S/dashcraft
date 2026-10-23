//! [`PathPolicy`]: which files a command may read and write.
//!
//! Every path a command receives (documents, blob maps, CSVs, output files
//! and folders) goes through [`PathPolicy::check_read`] or
//! [`PathPolicy::check_write`] before it is used.
//!
//! - **Unrestricted** (the default, used by the CLI): paths are used as
//!   given, relative to the process's working directory.
//! - **Restricted** (agents, automation hosts): reads must resolve inside a
//!   read root (or a write root), writes inside a write root. Relative paths
//!   are resolved against the base directory (default: the first read root,
//!   else the first write root). The existing part of a path is
//!   canonicalized (symlinks resolved, `..` applied) and must stay inside a
//!   root; the part that does not exist yet may only contain plain names (no
//!   `..`), and a dangling symlink at its start is refused, so neither `..`
//!   nor a symlink can escape the roots. Writing to an existing symlink is
//!   refused.
//!
//! **Limits (TOCTOU).** The check happens before the file is opened: a
//! process that can write inside a root could swap a checked folder for a
//! symlink between the check and the use. The roots are meant to be folders
//! only the user controls; capability handles (`cap-std` style `openat`
//! confinement) would close this gap and can replace this check later
//! without changing the command API.

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::error::OpError;

/// Read and write roots; see the [module docs](self).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PathPolicy {
    /// `None`: unrestricted.
    read_roots: Option<Vec<PathBuf>>,
    /// `None`: unrestricted.
    write_roots: Option<Vec<PathBuf>>,
    base_dir: Option<PathBuf>,
}

/// Which roots a path is checked against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    Read,
    Write,
}

impl Access {
    fn verb(self) -> &'static str {
        match self {
            Access::Read => "read",
            Access::Write => "write",
        }
    }
}

impl PathPolicy {
    /// No restriction: every path is used as given.
    #[must_use]
    pub fn unrestricted() -> Self {
        Self::default()
    }

    /// Reads confined to `read_roots` (and `write_roots`), writes to
    /// `write_roots`. Each root must be an existing folder; it is
    /// canonicalized. Empty lists allow nothing.
    ///
    /// # Errors
    /// [`OpErrorCode::NotFound`](crate::OpErrorCode::NotFound) or
    /// [`OpErrorCode::InvalidParams`](crate::OpErrorCode::InvalidParams) for
    /// a root that is missing or not a folder.
    pub fn restricted<R, W>(read_roots: R, write_roots: W) -> Result<Self, OpError>
    where
        R: IntoIterator,
        R::Item: AsRef<Path>,
        W: IntoIterator,
        W::Item: AsRef<Path>,
    {
        let canonical = |roots: Vec<PathBuf>| -> Result<Vec<PathBuf>, OpError> {
            roots
                .iter()
                .map(|r| {
                    let c = fs::canonicalize(r)
                        .map_err(|e| OpError::not_found(format!("root {}: {e}", r.display())))?;
                    if c.is_dir() {
                        Ok(c)
                    } else {
                        Err(OpError::invalid_params(format!(
                            "root {} is not a folder",
                            r.display()
                        )))
                    }
                })
                .collect()
        };
        let read = canonical(
            read_roots
                .into_iter()
                .map(|p| p.as_ref().to_path_buf())
                .collect(),
        )?;
        let write = canonical(
            write_roots
                .into_iter()
                .map(|p| p.as_ref().to_path_buf())
                .collect(),
        )?;
        let base_dir = read.first().or_else(|| write.first()).cloned();
        Ok(Self {
            read_roots: Some(read),
            write_roots: Some(write),
            base_dir,
        })
    }

    /// Resolves relative paths against `dir` (restricted policies only).
    #[must_use]
    pub fn with_base_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.base_dir = Some(dir.into());
        self
    }

    /// `true` unless the policy is [`unrestricted`](Self::unrestricted).
    #[must_use]
    pub fn is_restricted(&self) -> bool {
        self.read_roots.is_some() || self.write_roots.is_some()
    }

    /// The canonical read roots (`None`: unrestricted).
    #[must_use]
    pub fn read_roots(&self) -> Option<&[PathBuf]> {
        self.read_roots.as_deref()
    }

    /// The canonical write roots (`None`: unrestricted).
    #[must_use]
    pub fn write_roots(&self) -> Option<&[PathBuf]> {
        self.write_roots.as_deref()
    }

    /// The path to read `path` from.
    ///
    /// # Errors
    /// [`OpErrorCode::PathDenied`](crate::OpErrorCode::PathDenied) outside
    /// the roots, [`OpErrorCode::InvalidParams`](crate::OpErrorCode::InvalidParams)
    /// for an empty path.
    pub fn check_read(&self, path: &Path) -> Result<PathBuf, OpError> {
        self.check(path, Access::Read)
    }

    /// The path to write `path` to (it may not exist yet).
    ///
    /// # Errors
    /// As [`check_read`](Self::check_read); writing to a symlink is refused.
    pub fn check_write(&self, path: &Path) -> Result<PathBuf, OpError> {
        self.check(path, Access::Write)
    }

    /// Checks a `blob-map.json` and, under a restricted policy, every local
    /// file it maps (relative to the map's folder) for reading.
    ///
    /// # Errors
    /// As [`check_read`](Self::check_read); an unreadable or malformed map
    /// under a restricted policy.
    pub fn check_blob_map(&self, path: &Path) -> Result<PathBuf, OpError> {
        let map = self.check_read(path)?;
        if !self.is_restricted() {
            return Ok(map);
        }
        let text = fs::read_to_string(&map)
            .map_err(|e| OpError::io(format!("cannot read blob map {}: {e}", path.display())))?;
        let json: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
            OpError::invalid_params(format!("blob map {} is not JSON: {e}", path.display()))
        })?;
        let base = map.parent().map(Path::to_path_buf).unwrap_or_default();
        let targets = json
            .get("containers")
            .and_then(serde_json::Value::as_object)
            .into_iter()
            .flat_map(|c| c.values())
            .filter_map(serde_json::Value::as_object)
            .flat_map(|blobs| blobs.values())
            .filter_map(serde_json::Value::as_str);
        for rel in targets {
            self.check_read(&base.join(rel))
                .map_err(|e| e.context(format!("blob map {}", path.display())))?;
        }
        Ok(map)
    }

    fn check(&self, path: &Path, access: Access) -> Result<PathBuf, OpError> {
        if path.as_os_str().is_empty() {
            return Err(OpError::invalid_params("the path is empty"));
        }
        let roots: Vec<&PathBuf> = match access {
            Access::Read => match (&self.read_roots, &self.write_roots) {
                (None, None) => return Ok(path.to_path_buf()),
                (r, w) => r.iter().chain(w.iter()).flatten().collect(),
            },
            Access::Write => match &self.write_roots {
                None if self.read_roots.is_none() => return Ok(path.to_path_buf()),
                None => Vec::new(),
                Some(w) => w.iter().collect(),
            },
        };
        let denied = |why: &str| {
            OpError::path_denied(format!("{} {}: {why}", access.verb(), path.display()))
        };
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            match &self.base_dir {
                Some(base) => base.join(path),
                None => return Err(denied("relative paths need a base folder")),
            }
        };
        if access == Access::Write
            && fs::symlink_metadata(&absolute).is_ok_and(|m| m.file_type().is_symlink())
        {
            return Err(denied("refusing to write through a symlink"));
        }
        let resolved = resolve(&absolute).map_err(|why| denied(&why))?;
        if roots.iter().any(|root| resolved.starts_with(root)) {
            Ok(resolved)
        } else {
            Err(denied(&format!(
                "outside the allowed {} roots",
                access.verb()
            )))
        }
    }
}

/// Canonicalizes the longest existing prefix of `path` and appends the rest,
/// which may only contain plain names.
fn resolve(path: &Path) -> Result<PathBuf, String> {
    let mut existing = path;
    let mut rest: Vec<Component<'_>> = Vec::new();
    loop {
        if existing.exists() {
            break;
        }
        let Some(name) = existing.components().next_back() else {
            return Err("no part of the path exists".to_owned());
        };
        rest.push(name);
        existing = match existing.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => return Err("no part of the path exists".to_owned()),
        };
    }
    let mut resolved = fs::canonicalize(existing).map_err(|e| format!("cannot resolve: {e}"))?;
    for (i, c) in rest.iter().rev().enumerate() {
        match c {
            Component::Normal(name) => {
                resolved.push(name);
                if i == 0 && fs::symlink_metadata(&resolved).is_ok() {
                    // Exists without `exists()`: a dangling symlink.
                    return Err("refusing a dangling symlink".to_owned());
                }
            }
            Component::CurDir => {}
            _ => return Err("`..` is not allowed after a missing folder".to_owned()),
        }
    }
    Ok(resolved)
}
