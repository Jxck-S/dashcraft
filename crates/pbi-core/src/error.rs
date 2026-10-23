//! Errors of the readers ([`crate::open_path`], [`crate::open_bytes`]) and
//! writers ([`crate::save_as_pbip`], [`crate::export_pbit`]).
//!
//! Only unreadable, unsafe or unsupported input is an error; problems inside
//! an otherwise readable file become [`pbi_model::Diagnostic`]s.

use std::path::PathBuf;

use crate::container::ContainerError;
use crate::detect::DetectError;

/// Error returned by the readers and writers of this crate.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CoreError {
    /// The path could not be read or written.
    #[error("I/O error on {path:?}: {source}")]
    Io {
        /// The file or folder involved.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// The folder or archive was refused (unsafe entry names, limits
    /// exceeded, corrupt ZIP).
    #[error(transparent)]
    Container(#[from] ContainerError),
    /// The input is not a format this crate can open.
    #[error("unsupported input ({format}): {hint}")]
    Unsupported {
        /// Detected format label (`unknown` if nothing matched).
        format: String,
        /// Why, and what to try instead.
        hint: String,
    },
    /// The document cannot be written in the requested form.
    #[error("cannot save: {0}")]
    Save(String),
}

impl CoreError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

impl From<DetectError> for CoreError {
    fn from(e: DetectError) -> Self {
        match e {
            DetectError::Io { path, source } => Self::Io { path, source },
            DetectError::Container(c) => Self::Container(c),
        }
    }
}
