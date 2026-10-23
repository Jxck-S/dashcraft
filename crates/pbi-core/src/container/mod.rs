//! Safe, read-only views over untrusted containers: directories, ZIP
//! archives (PBIX, PBIT, XLSX, VPAX, ...) and in-memory file sets.
//!
//! Every container lists its entries up front with normalized paths
//! (relative, `/`-separated, no `.`/`..`/empty components) and validates
//! them against [`Limits`]. Readers only ever address entries by these
//! normalized paths, so nothing outside the container can be reached.
//!
//! Readers should take `&dyn Container`; a sub-folder is viewed with
//! [`Container::sub`], which returns a [`SubContainer`] that is itself a
//! `Container`:
//!
//! ```no_run
//! use pbi_core::container::{Container, Limits, ZipContainer};
//!
//! fn read_report(report: &dyn Container) -> usize {
//!     report.list_prefix("definition/pages").len()
//! }
//!
//! let pbix = ZipContainer::open("report.pbix", Limits::default())?;
//! let report = pbix.sub("Report");
//! read_report(&report);
//! # Ok::<(), pbi_core::container::ContainerError>(())
//! ```

mod dir;
mod error;
mod limits;
mod mem;
pub(crate) mod path;
mod text;
mod zip;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

pub use dir::DirContainer;
pub use error::{ContainerError, LimitKind, SkipReason, SkippedEntry, UnsafePathReason};
pub use limits::Limits;
pub use mem::MemContainer;
pub use text::{DecodedText, TextEncoding, decode_text};
pub use zip::ZipContainer;

use path::{is_under, join, normalize_lookup};

/// What backs a container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ContainerKind {
    /// A directory on disk.
    Directory,
    /// A ZIP archive (file or bytes).
    Zip,
    /// An in-memory file set.
    Memory,
}

/// One entry of a container listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryInfo {
    /// Normalized path relative to the container root: `/`-separated, no
    /// leading `./`, no trailing `/`. Comparisons are case-sensitive.
    pub path: String,
    /// Uncompressed size in bytes (declared size for ZIP entries; 0 for
    /// directories).
    pub size: u64,
    /// Compressed size for ZIP entries, `None` otherwise.
    pub compressed_size: Option<u64>,
    /// `true` for directory entries.
    pub is_dir: bool,
}

impl EntryInfo {
    /// The last path component.
    #[must_use]
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

/// An immediate child of a directory, as returned by
/// [`Container::list_dir`]. Directories implied by deeper entries are
/// included even when the container has no explicit entry for them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirChild {
    /// Child name (one component).
    pub name: String,
    /// Full normalized path relative to the container root.
    pub path: String,
    /// `true` if the child is a directory.
    pub is_dir: bool,
}

/// A read-only, thread-safe view over a set of named files.
///
/// Implementations must return [`entries`](Self::entries) sorted by `path`
/// (byte order) with unique paths; the provided lookup methods rely on it.
pub trait Container: Send + Sync + fmt::Debug {
    /// What backs this container.
    fn kind(&self) -> ContainerKind;

    /// Human-readable label for messages (file or folder path).
    fn root_label(&self) -> &str;

    /// All entries, sorted by path.
    fn entries(&self) -> &[EntryInfo];

    /// Reads a file entry fully, enforcing the container's [`Limits`].
    ///
    /// # Errors
    /// [`ContainerError::NotFound`] if no such file entry exists,
    /// [`ContainerError::IsDirectory`] for directories, and limit, safety,
    /// corruption or I/O errors as they occur.
    fn read(&self, path: &str) -> Result<Vec<u8>, ContainerError>;

    /// `self` as a trait object (lets provided methods build sub-views).
    fn as_dyn(&self) -> &dyn Container;

    /// Entries deliberately left out of the listing (symlinks, duplicates,
    /// ignored folders), for diagnostics.
    fn skipped(&self) -> &[SkippedEntry] {
        &[]
    }

    /// The explicit entry at `path` (case-sensitive).
    fn entry(&self, path: &str) -> Option<&EntryInfo> {
        let key = normalize_lookup(path)?;
        let entries = self.entries();
        entries
            .binary_search_by(|e| e.path.as_str().cmp(key.as_str()))
            .ok()
            .and_then(|i| entries.get(i))
    }

    /// `true` if `path` is a file, an explicit directory or a directory
    /// implied by deeper entries. `""` (the root) always exists.
    fn exists(&self, path: &str) -> bool {
        self.entry(path).is_some() || self.is_dir(path)
    }

    /// `true` if `path` is a file entry.
    fn is_file(&self, path: &str) -> bool {
        self.entry(path).is_some_and(|e| !e.is_dir)
    }

    /// `true` if `path` is an explicit or implied directory (or the root).
    fn is_dir(&self, path: &str) -> bool {
        let Some(key) = normalize_lookup(path) else {
            return false;
        };
        if key.is_empty() {
            return true;
        }
        self.entry(&key).is_some_and(|e| e.is_dir)
            || !entries_under(self.entries(), &key).is_empty()
    }

    /// All entries strictly under the folder `prefix` (recursively), in
    /// path order. `""` lists everything.
    fn list_prefix(&self, prefix: &str) -> Vec<&EntryInfo> {
        let Some(dir) = normalize_lookup(prefix) else {
            return Vec::new();
        };
        entries_under(self.entries(), &dir).iter().collect()
    }

    /// Immediate children of the folder `dir` (`""` for the root), sorted
    /// by name, including implied directories.
    fn list_dir(&self, dir: &str) -> Vec<DirChild> {
        let Some(dir) = normalize_lookup(dir) else {
            return Vec::new();
        };
        let mut children: BTreeMap<&str, bool> = BTreeMap::new();
        let skip = if dir.is_empty() { 0 } else { dir.len() + 1 };
        for e in self.list_prefix(&dir) {
            let rest = &e.path[skip..];
            let (name, deeper) = match rest.split_once('/') {
                Some((name, _)) => (name, true),
                None => (rest, e.is_dir),
            };
            *children.entry(name).or_insert(false) |= deeper;
        }
        children
            .into_iter()
            .map(|(name, is_dir)| DirChild {
                name: name.to_owned(),
                path: join(&dir, name),
                is_dir,
            })
            .collect()
    }

    /// Like [`entry`](Self::entry), but falls back to a case-insensitive
    /// match (first in path order) when there is no exact match.
    fn find_case_insensitive(&self, path: &str) -> Option<&EntryInfo> {
        if let Some(e) = self.entry(path) {
            return Some(e);
        }
        let key = normalize_lookup(path)?.to_lowercase();
        self.entries().iter().find(|e| e.path.to_lowercase() == key)
    }

    /// Reads a file entry and decodes it as text (UTF-8 or UTF-16, detected
    /// from the BOM or NUL-byte pattern).
    ///
    /// # Errors
    /// As [`read`](Self::read).
    fn read_text(&self, path: &str) -> Result<DecodedText, ContainerError> {
        self.read(path).map(|bytes| decode_text(&bytes))
    }

    /// A view of the folder `prefix` as its own container. An invalid
    /// prefix (e.g. containing `..`) yields an empty view.
    fn sub(&self, prefix: &str) -> SubContainer<'_> {
        SubContainer::new(self.as_dyn(), prefix)
    }
}

/// The contiguous run of sorted `entries` strictly under the normalized
/// folder `dir` (`""` = all).
fn entries_under<'a>(entries: &'a [EntryInfo], dir: &str) -> &'a [EntryInfo] {
    if dir.is_empty() {
        return entries;
    }
    let pfx = format!("{dir}/");
    let start = entries.partition_point(|e| e.path.as_str() < pfx.as_str());
    let len = entries[start..]
        .iter()
        .take_while(|e| e.path.starts_with(pfx.as_str()))
        .count();
    &entries[start..start + len]
}

impl<C: Container + ?Sized> Container for &C {
    fn kind(&self) -> ContainerKind {
        (**self).kind()
    }
    fn root_label(&self) -> &str {
        (**self).root_label()
    }
    fn entries(&self) -> &[EntryInfo] {
        (**self).entries()
    }
    fn read(&self, path: &str) -> Result<Vec<u8>, ContainerError> {
        (**self).read(path)
    }
    fn as_dyn(&self) -> &dyn Container {
        (**self).as_dyn()
    }
    fn skipped(&self) -> &[SkippedEntry] {
        (**self).skipped()
    }
}

impl<C: Container + ?Sized> Container for Box<C> {
    fn kind(&self) -> ContainerKind {
        (**self).kind()
    }
    fn root_label(&self) -> &str {
        (**self).root_label()
    }
    fn entries(&self) -> &[EntryInfo] {
        (**self).entries()
    }
    fn read(&self, path: &str) -> Result<Vec<u8>, ContainerError> {
        (**self).read(path)
    }
    fn as_dyn(&self) -> &dyn Container {
        (**self).as_dyn()
    }
    fn skipped(&self) -> &[SkippedEntry] {
        (**self).skipped()
    }
}

impl<C: Container + ?Sized> Container for Arc<C> {
    fn kind(&self) -> ContainerKind {
        (**self).kind()
    }
    fn root_label(&self) -> &str {
        (**self).root_label()
    }
    fn entries(&self) -> &[EntryInfo] {
        (**self).entries()
    }
    fn read(&self, path: &str) -> Result<Vec<u8>, ContainerError> {
        (**self).read(path)
    }
    fn as_dyn(&self) -> &dyn Container {
        (**self).as_dyn()
    }
    fn skipped(&self) -> &[SkippedEntry] {
        (**self).skipped()
    }
}

#[derive(Debug, Clone)]
enum Parent<'a> {
    Borrowed(&'a dyn Container),
    Shared(Arc<dyn Container>),
}

impl Parent<'_> {
    fn get(&self) -> &dyn Container {
        match self {
            Parent::Borrowed(c) => *c,
            Parent::Shared(c) => c.as_ref(),
        }
    }
}

/// A view of one folder of another container, re-rooted so that paths are
/// relative to that folder. Reads are delegated to the parent (with the
/// parent's limits). Nested `sub` calls work as expected.
#[derive(Debug, Clone)]
pub struct SubContainer<'a> {
    parent: Parent<'a>,
    /// Normalized prefix; `None` if the requested prefix was invalid.
    prefix: Option<String>,
    label: String,
    entries: Vec<EntryInfo>,
    skipped: Vec<SkippedEntry>,
}

impl<'a> SubContainer<'a> {
    /// A view of `prefix` within a borrowed container.
    #[must_use]
    pub fn new(parent: &'a dyn Container, prefix: &str) -> Self {
        Self::build(Parent::Borrowed(parent), prefix)
    }

    /// The normalized folder this view is rooted at (`""` for the parent
    /// root), or `None` if the requested prefix was invalid.
    #[must_use]
    pub fn prefix(&self) -> Option<&str> {
        self.prefix.as_deref()
    }

    fn build(parent: Parent<'a>, prefix: &str) -> Self {
        let p = parent.get();
        let norm = normalize_lookup(prefix);
        let (entries, skipped) = match norm.as_deref() {
            Some(dir) => {
                let cut = if dir.is_empty() { 0 } else { dir.len() + 1 };
                let entries = p
                    .list_prefix(dir)
                    .into_iter()
                    .map(|e| EntryInfo {
                        path: e.path[cut..].to_owned(),
                        ..e.clone()
                    })
                    .collect();
                let skipped = p
                    .skipped()
                    .iter()
                    .filter(|s| is_under(&s.path, dir))
                    .map(|s| SkippedEntry {
                        path: s.path[cut..].to_owned(),
                        reason: s.reason.clone(),
                    })
                    .collect();
                (entries, skipped)
            }
            None => (Vec::new(), Vec::new()),
        };
        let label = match norm.as_deref() {
            Some("") => p.root_label().to_owned(),
            Some(dir) => format!("{}/{dir}", p.root_label()),
            None => format!("{}/<invalid {prefix:?}>", p.root_label()),
        };
        Self {
            parent,
            prefix: norm,
            label,
            entries,
            skipped,
        }
    }
}

impl SubContainer<'static> {
    /// A view of `prefix` within a shared container; the view keeps the
    /// parent alive and can be stored freely.
    #[must_use]
    pub fn shared(parent: Arc<dyn Container>, prefix: &str) -> Self {
        Self::build(Parent::Shared(parent), prefix)
    }
}

impl Container for SubContainer<'_> {
    fn kind(&self) -> ContainerKind {
        self.parent.get().kind()
    }

    fn root_label(&self) -> &str {
        &self.label
    }

    fn entries(&self) -> &[EntryInfo] {
        &self.entries
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, ContainerError> {
        let (Some(prefix), Some(rel)) = (self.prefix.as_deref(), normalize_lookup(path)) else {
            return Err(ContainerError::NotFound(path.to_owned()));
        };
        if rel.is_empty() {
            return Err(ContainerError::IsDirectory(path.to_owned()));
        }
        self.parent.get().read(&join(prefix, &rel))
    }

    fn as_dyn(&self) -> &dyn Container {
        self
    }

    fn skipped(&self) -> &[SkippedEntry] {
        &self.skipped
    }
}
