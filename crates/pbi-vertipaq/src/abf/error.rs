use thiserror::Error;

use crate::xpress8::Xpress8Error;
use crate::xpress9::Xpress9Error;

/// Errors produced while opening or reading an Analysis Services backup (ABF).
///
/// Every malformed input yields one of these; the parser never panics. Messages never
/// include file contents or the absolute server paths stored in the backup log.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AbfError {
    /// The bytes start with neither an XPress9 nor an uncompressed ABF signature.
    #[error("unrecognised DataModel signature (not an XPress9 stream or an ABF backup)")]
    UnknownSignature,
    /// The bytes are not an uncompressed ABF backup (`STREAM_STORAGE_SIGNATURE` missing).
    #[error("not an uncompressed Analysis Services backup (missing STREAM_STORAGE signature)")]
    NotAbf,
    /// The XPress9 layer of a PBIX `DataModel` could not be decoded.
    #[error("XPress9 decompression failed: {0}")]
    Xpress9(#[from] Xpress9Error),
    /// A structure extends past the end of the backup.
    #[error("{what} at offset {offset} with size {size} lies outside the backup ({len} bytes)")]
    OutOfBounds {
        /// The structure that was being located.
        what: &'static str,
        /// Declared start offset.
        offset: u64,
        /// Declared size.
        size: u64,
        /// Total size of the backup.
        len: u64,
    },
    /// An XML document of the backup is malformed.
    #[error("malformed {what} XML: {message}")]
    Xml {
        /// The document (`backup header`, `virtual directory`, `backup log`, ...).
        what: &'static str,
        /// Parser message (positions only, no content).
        message: String,
    },
    /// Text that must be UTF-16LE or UTF-8 is not valid.
    #[error("invalid text encoding in {0}")]
    Encoding(&'static str),
    /// A required XML element is missing.
    #[error("missing element <{element}> in {what}")]
    MissingField {
        /// The document.
        what: &'static str,
        /// The element name.
        element: &'static str,
    },
    /// An XML element has a value of the wrong type.
    #[error("invalid value {value:?} for <{element}> in {what}")]
    InvalidField {
        /// The document.
        what: &'static str,
        /// The element name.
        element: &'static str,
        /// The offending value (truncated to 40 characters).
        value: String,
    },
    /// The backup is encrypted (`EncryptionFlag`); decryption is not supported.
    #[error("encrypted Analysis Services backups are not supported")]
    Encrypted,
    /// The file is compressed inside the backup (`ApplyCompression`, typical for Power
    /// Pivot workbooks), so it has no zero-copy view: use
    /// [`AbfArchive::read_file`](super::AbfArchive::read_file), which decompresses it.
    #[error(
        "{path:?} is stored compressed inside the backup (ApplyCompression); read it with read_file"
    )]
    CompressedFile {
        /// Logical path of the file.
        path: String,
    },
    /// A file stored compressed inside the backup (`ApplyCompression`) could not be
    /// decompressed.
    #[error("cannot decompress {path:?}: {source}")]
    Decompress {
        /// Logical path of the file.
        path: String,
        /// The decoder error.
        #[source]
        source: Xpress8Error,
    },
    /// No file with this logical path (or storage name) exists in the backup.
    #[error("file not found in backup: {0:?}")]
    FileNotFound(String),
    /// A structural limit was exceeded.
    #[error("limit exceeded: {what} is {value}, the limit is {limit}")]
    LimitExceeded {
        /// The quantity that is limited.
        what: &'static str,
        /// The value found in the input.
        value: u64,
        /// The limit.
        limit: u64,
    },
}

/// Truncates a field value for inclusion in an error message.
pub(crate) fn excerpt(value: &str) -> String {
    const MAX: usize = 40;
    match value.char_indices().nth(MAX) {
        Some((cut, _)) => format!("{}…", value.get(..cut).unwrap_or_default()),
        None => value.to_owned(),
    }
}
