//! Directory container (PBIP projects, report/model folders, TMDL folders).
//!
//! The tree is walked once at open with `follow_links(false)`: symbolic
//! links are never followed and are reported in [`Container::skipped`],
//! together with `.git`, `node_modules` and `.DS_Store`. Only files found
//! by that walk can be read, and every read re-canonicalizes the path and
//! refuses anything that now resolves outside the canonical root (defense
//! against symlinks swapped in after the walk).

use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use walkdir::WalkDir;

use super::{
    Container, ContainerError, ContainerKind, EntryInfo, LimitKind, Limits, SkipReason,
    SkippedEntry, UnsafePathReason,
};

/// Names skipped while walking (version control and tooling noise).
const IGNORED_NAMES: &[&str] = &[".git", "node_modules", ".DS_Store"];

/// Upper bound for the up-front allocation of a read buffer.
const PREALLOC_MAX: u64 = 64 * 1024 * 1024;

/// A validated, read-only directory tree. See the [module docs](self).
#[derive(Debug, Clone)]
pub struct DirContainer {
    root: PathBuf,
    label: String,
    limits: Limits,
    entries: Vec<EntryInfo>,
    skipped: Vec<SkippedEntry>,
}

impl DirContainer {
    /// Walks `root` and lists its files and folders.
    ///
    /// # Errors
    /// [`ContainerError::InvalidRoot`] if `root` is not a directory,
    /// [`ContainerError::LimitExceeded`] for `max_entries`, `max_depth` or
    /// `max_path_len`, and I/O errors on the root itself. Unreadable
    /// sub-entries are skipped and reported instead.
    pub fn open(root: impl AsRef<Path>, limits: Limits) -> Result<Self, ContainerError> {
        let given = root.as_ref();
        let label = given.display().to_string();
        let canonical = fs::canonicalize(given).map_err(|e| ContainerError::io(&label, e))?;
        let meta = fs::metadata(&canonical).map_err(|e| ContainerError::io(&label, e))?;
        if !meta.is_dir() {
            return Err(ContainerError::InvalidRoot {
                path: label,
                detail: "not a directory".into(),
            });
        }

        let mut entries = Vec::new();
        let mut skipped = Vec::new();
        let mut walker = WalkDir::new(&canonical)
            .follow_links(false)
            .min_depth(1)
            .max_depth(limits.max_depth.saturating_add(1))
            .into_iter();
        while let Some(next) = walker.next() {
            let dent = match next {
                Ok(d) => d,
                Err(err) => {
                    let path = err
                        .path()
                        .and_then(|p| relative(&canonical, p))
                        .unwrap_or_default();
                    skipped.push(SkippedEntry {
                        path,
                        reason: SkipReason::Unreadable(err.to_string()),
                    });
                    continue;
                }
            };
            let ft = dent.file_type();
            let Some(rel) = relative(&canonical, dent.path()) else {
                skipped.push(SkippedEntry {
                    path: dent
                        .path()
                        .strip_prefix(&canonical)
                        .unwrap_or(dent.path())
                        .to_string_lossy()
                        .replace('\\', "/"),
                    reason: SkipReason::UnsupportedName,
                });
                if ft.is_dir() {
                    walker.skip_current_dir();
                }
                continue;
            };
            if IGNORED_NAMES.iter().any(|n| dent.file_name() == *n) {
                skipped.push(SkippedEntry {
                    path: rel,
                    reason: SkipReason::Ignored,
                });
                if ft.is_dir() {
                    walker.skip_current_dir();
                }
                continue;
            }
            if ft.is_symlink() {
                skipped.push(SkippedEntry {
                    path: rel,
                    reason: SkipReason::Symlink,
                });
                continue;
            }
            if dent.depth() > limits.max_depth {
                return Err(ContainerError::limit(
                    LimitKind::Depth,
                    format!("{rel:?} is deeper than {} levels", limits.max_depth),
                ));
            }
            if rel.len() > limits.max_path_len {
                return Err(ContainerError::limit(
                    LimitKind::PathLength,
                    format!(
                        "entry path is {} bytes (max {})",
                        rel.len(),
                        limits.max_path_len
                    ),
                ));
            }
            if ft.is_dir() {
                entries.push(EntryInfo {
                    path: rel,
                    size: 0,
                    compressed_size: None,
                    is_dir: true,
                });
            } else if ft.is_file() {
                match dent.metadata() {
                    Ok(m) => entries.push(EntryInfo {
                        path: rel,
                        size: m.len(),
                        compressed_size: None,
                        is_dir: false,
                    }),
                    Err(err) => skipped.push(SkippedEntry {
                        path: rel,
                        reason: SkipReason::Unreadable(err.to_string()),
                    }),
                }
            } else {
                skipped.push(SkippedEntry {
                    path: rel,
                    reason: SkipReason::SpecialFile,
                });
            }
            if entries.len() > limits.max_entries {
                return Err(ContainerError::limit(
                    LimitKind::Entries,
                    format!("more than {} entries", limits.max_entries),
                ));
            }
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        skipped.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Self {
            root: canonical,
            label,
            limits,
            entries,
            skipped,
        })
    }

    /// The canonical root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The limits this container enforces.
    #[must_use]
    pub fn limits(&self) -> &Limits {
        &self.limits
    }
}

impl Container for DirContainer {
    fn kind(&self) -> ContainerKind {
        ContainerKind::Directory
    }

    fn root_label(&self) -> &str {
        &self.label
    }

    fn entries(&self) -> &[EntryInfo] {
        &self.entries
    }

    fn skipped(&self) -> &[SkippedEntry] {
        &self.skipped
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, ContainerError> {
        let entry = self
            .entry(path)
            .ok_or_else(|| ContainerError::NotFound(path.to_owned()))?;
        if entry.is_dir {
            return Err(ContainerError::IsDirectory(entry.path.clone()));
        }
        let mut full = self.root.clone();
        full.extend(entry.path.split('/'));
        let real = fs::canonicalize(&full).map_err(|e| ContainerError::io(&entry.path, e))?;
        if !real.starts_with(&self.root) {
            return Err(ContainerError::unsafe_path(
                &entry.path,
                UnsafePathReason::EscapesRoot,
            ));
        }
        // Check before opening: opening a FIFO would block.
        let meta = fs::metadata(&real).map_err(|e| ContainerError::io(&entry.path, e))?;
        if !meta.is_file() {
            return Err(ContainerError::Unsupported(format!(
                "{}: no longer a regular file",
                entry.path
            )));
        }
        let max = self.limits.max_entry_size;
        if meta.len() > max {
            return Err(ContainerError::limit(
                LimitKind::EntrySize,
                format!("{} is {} bytes (max {max})", entry.path, meta.len()),
            ));
        }
        let file = File::open(&real).map_err(|e| ContainerError::io(&entry.path, e))?;
        let mut out = Vec::new();
        let prealloc = usize::try_from(meta.len().min(PREALLOC_MAX)).unwrap_or(0);
        out.try_reserve_exact(prealloc)
            .map_err(|e| ContainerError::io(&entry.path, std::io::Error::other(e)))?;
        file.take(max.saturating_add(1))
            .read_to_end(&mut out)
            .map_err(|e| ContainerError::io(&entry.path, e))?;
        if out.len() as u64 > max {
            return Err(ContainerError::limit(
                LimitKind::EntrySize,
                format!("{} grew beyond {max} bytes", entry.path),
            ));
        }
        Ok(out)
    }

    fn as_dyn(&self) -> &dyn Container {
        self
    }
}

/// `path` relative to `root` as a normalized `/`-separated string; `None`
/// if it is not under `root` or a component is not valid UTF-8.
fn relative(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for c in rel.components() {
        match c {
            Component::Normal(s) => {
                let s = s.to_str()?;
                if s.contains('\\') {
                    return None;
                }
                parts.push(s);
            }
            _ => return None,
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}
