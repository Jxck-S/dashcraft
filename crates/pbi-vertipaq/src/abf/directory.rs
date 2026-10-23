//! The virtual directory: the UTF-8 `<VirtualDirectory>` document that maps storage
//! names to byte ranges of the backup.

use super::error::AbfError;
use super::xml;

const WHAT: &str = "virtual directory";

/// Storage name of the backup log entry.
pub const LOG_STORAGE_NAME: &str = "LOG";
/// Storage name of the additional log entry.
pub const ADDITIONAL_LOG_STORAGE_NAME: &str = "ADDITIONAL_LOG";
/// Storage name of the partitions marker entry.
pub const PARTITIONS_STORAGE_NAME: &str = "PARTITIONS";

/// One `<BackupFile>` of the virtual directory.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DirectoryEntry {
    /// `Path`: the storage name (20 hex digits for model files, or `LOG`,
    /// `ADDITIONAL_LOG`, `PARTITIONS`).
    pub storage_path: String,
    /// `m_cbOffsetHeader`: absolute offset of the stored bytes.
    pub offset: u64,
    /// `Size`: stored size in bytes (includes the CRC marker when the header's
    /// `ErrorCode` is set).
    pub size: u64,
    /// `Delete`.
    pub delete: bool,
    /// `CreatedTimestamp` (Windows FILETIME ticks).
    pub created_timestamp: Option<i64>,
    /// `Access` (Windows FILETIME ticks).
    pub access: Option<i64>,
    /// `LastWriteTime` (Windows FILETIME ticks).
    pub last_write_time: Option<i64>,
}

/// Parses the virtual directory document. `max_entries` bounds the entry count.
pub(crate) fn parse_directory(
    bytes: &[u8],
    max_entries: usize,
) -> Result<Vec<DirectoryEntry>, AbfError> {
    let text = xml::decode_document(bytes, WHAT)?;
    let doc = xml::parse(&text, WHAT)?;
    let root = xml::expect_root(&doc, "VirtualDirectory", WHAT)?;
    let mut entries = Vec::new();
    for node in xml::children(root, "BackupFile") {
        if entries.len() >= max_entries {
            return Err(AbfError::LimitExceeded {
                what: "virtual directory entries",
                value: entries.len() as u64 + 1,
                limit: max_entries as u64,
            });
        }
        let storage_path = xml::opt_text(node, "Path").ok_or(AbfError::MissingField {
            what: WHAT,
            element: "Path",
        })?;
        entries.push(DirectoryEntry {
            storage_path,
            offset: xml::req_u64(node, "m_cbOffsetHeader", WHAT)?,
            size: xml::req_u64(node, "Size", WHAT)?,
            delete: xml::opt_bool(node, "Delete", WHAT)?.unwrap_or(false),
            created_timestamp: xml::opt_i64(node, "CreatedTimestamp", WHAT)?,
            access: xml::opt_i64(node, "Access", WHAT)?,
            last_write_time: xml::opt_i64(node, "LastWriteTime", WHAT)?,
        });
    }
    Ok(entries)
}
