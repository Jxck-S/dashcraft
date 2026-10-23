//! Errors of the app-only Tauri commands (the registry commands run through `ops_execute`
//! fail with `pbi_ops::OpError` codes instead).

use std::io;
use std::path::Path;

use serde::Serialize;

/// Error type for every IPC command.
///
/// Serialized as `{ "kind": "...", "message": "..." }` so the frontend can branch on `kind`
/// (see the generated `AppError` type in `app/src/lib/bindings.ts`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, specta::Type)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
pub enum AppError {
    /// The request itself was malformed (e.g. an empty path).
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    /// The path does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// The OS refused access to the path.
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    /// The path exists but is not something DashCraft can open (unknown format, socket, ...).
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// The folder or archive was refused: unsafe entry names, size limits, corrupt ZIP.
    #[error("invalid file: {0}")]
    InvalidFile(String),
    /// Any other I/O failure.
    #[error("I/O error: {0}")]
    Io(String),
    /// The command needs an open document, but none is open.
    #[error("no document: {0}")]
    NoDocument(String),
    /// No object with the requested ID in the open document (stale selection).
    #[error("unknown object: {0}")]
    UnknownObject(String),
    /// Another operation holds the resource (e.g. a refresh is already running).
    #[error("busy: {0}")]
    Busy(String),
    /// A bug or an unexpected failure in the backend (e.g. a crashed worker).
    #[error("internal error: {0}")]
    Internal(String),
}

impl AppError {
    /// Maps an [`io::Error`] for `path` to the closest variant.
    pub(crate) fn from_io(path: &Path, err: &io::Error) -> Self {
        let shown = path.display();
        match err.kind() {
            io::ErrorKind::NotFound => Self::NotFound(shown.to_string()),
            io::ErrorKind::PermissionDenied => Self::PermissionDenied(shown.to_string()),
            _ => Self::Io(format!("{shown}: {err}")),
        }
    }
}
