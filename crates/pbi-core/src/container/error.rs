//! Errors and diagnostics produced by containers.

use std::fmt;

/// Error returned when opening or reading a [`Container`](super::Container).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ContainerError {
    /// An entry name is unsafe (zip-slip, absolute path, ...). Opening the
    /// container fails as a whole: a single hostile name taints the archive.
    #[error("unsafe entry path {path:?}: {reason}")]
    UnsafePath {
        /// The offending name as found in the container (lossy UTF-8).
        path: String,
        /// Why it was rejected.
        reason: UnsafePathReason,
    },
    /// A resource cap from [`Limits`](super::Limits) was exceeded.
    #[error("limit exceeded ({limit}): {detail}")]
    LimitExceeded {
        /// Which cap.
        limit: LimitKind,
        /// Human-readable detail (entry name, sizes).
        detail: String,
    },
    /// The entry uses a feature this reader does not support (encryption,
    /// a compression method other than stored/deflate, ...).
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// No file entry with that path.
    #[error("entry not found: {0:?}")]
    NotFound(String),
    /// The path names a directory, not a file.
    #[error("entry is a directory: {0:?}")]
    IsDirectory(String),
    /// The container is structurally invalid (bad ZIP headers, CRC mismatch,
    /// truncated data, ...).
    #[error("corrupt container: {0}")]
    Corrupt(String),
    /// The path given to `open` is not usable (missing, not a directory, ...).
    #[error("cannot open {path:?}: {detail}")]
    InvalidRoot {
        /// The path passed to `open`.
        path: String,
        /// Why it cannot be used.
        detail: String,
    },
    /// An I/O error from the operating system.
    #[error("I/O error on {path:?}: {source}")]
    Io {
        /// The file or entry involved.
        path: String,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
}

impl ContainerError {
    pub(crate) fn limit(limit: LimitKind, detail: impl Into<String>) -> Self {
        Self::LimitExceeded {
            limit,
            detail: detail.into(),
        }
    }

    pub(crate) fn unsafe_path(path: impl Into<String>, reason: UnsafePathReason) -> Self {
        Self::UnsafePath {
            path: path.into(),
            reason,
        }
    }

    pub(crate) fn io(path: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

/// Why an entry path was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UnsafePathReason {
    /// Starts with `/` (or `\`, after normalization), including UNC paths.
    Absolute,
    /// Has a `..` component.
    ParentTraversal,
    /// Starts with a Windows drive letter such as `C:`.
    DriveLetter,
    /// Contains a NUL byte.
    Nul,
    /// The same raw name occurs more than once in the ZIP central directory.
    /// ZIP tools disagree about which copy wins, so the archive is refused.
    DuplicateName,
    /// The resolved file lies outside the container root (symlink swap).
    EscapesRoot,
}

impl fmt::Display for UnsafePathReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Absolute => "absolute path",
            Self::ParentTraversal => "`..` component (path traversal)",
            Self::DriveLetter => "Windows drive letter",
            Self::Nul => "NUL byte in name",
            Self::DuplicateName => "duplicate raw name in the ZIP central directory",
            Self::EscapesRoot => "resolves outside the container root",
        })
    }
}

/// Which [`Limits`](super::Limits) cap was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LimitKind {
    /// `max_entries`.
    Entries,
    /// `max_entry_size`.
    EntrySize,
    /// `max_total_size`.
    TotalSize,
    /// `max_compression_ratio` (per entry or archive-wide).
    CompressionRatio,
    /// An entry produced more bytes than its declared size.
    DeclaredSize,
    /// ZIP entries share compressed data (overlapping-file zip bomb).
    OverlappingEntries,
    /// `max_path_len`.
    PathLength,
    /// `max_depth`.
    Depth,
}

impl fmt::Display for LimitKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Entries => "entry count",
            Self::EntrySize => "entry size",
            Self::TotalSize => "total size",
            Self::CompressionRatio => "compression ratio",
            Self::DeclaredSize => "declared size",
            Self::OverlappingEntries => "overlapping entries",
            Self::PathLength => "path length",
            Self::Depth => "path depth",
        })
    }
}

/// An entry that was deliberately left out of a container listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedEntry {
    /// Normalized (or best-effort) path relative to the container root.
    pub path: String,
    /// Why it was skipped.
    pub reason: SkipReason,
}

/// Why an entry was skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SkipReason {
    /// A symbolic link (never followed).
    Symlink,
    /// A well-known noise entry (`.git`, `node_modules`, `.DS_Store`).
    Ignored,
    /// A later entry whose normalized path duplicates an earlier one (the
    /// first is kept).
    Duplicate,
    /// Not a regular file or directory (FIFO, socket, device).
    SpecialFile,
    /// The file name is not valid UTF-8 or contains a backslash.
    UnsupportedName,
    /// The entry names the container root itself (e.g. `./`).
    RootEntry,
    /// The entry could not be inspected (permission denied, ...).
    Unreadable(String),
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Symlink => f.write_str("symbolic link (not followed)"),
            Self::Ignored => f.write_str("ignored (VCS/tooling noise)"),
            Self::Duplicate => f.write_str("duplicate path (first entry kept)"),
            Self::SpecialFile => f.write_str("not a regular file"),
            Self::UnsupportedName => f.write_str("file name is not valid UTF-8 or contains `\\`"),
            Self::RootEntry => f.write_str("refers to the container root"),
            Self::Unreadable(why) => write!(f, "unreadable: {why}"),
        }
    }
}
