//! The backup header: the UTF-16LE `<BackupLog>` document in the first page.

use super::error::AbfError;
use super::xml;
use crate::xpress9::STREAM_STORAGE_SIGNATURE_LEN;

/// ABF page size; the header, the files section and the virtual directory are page
/// aligned ([MS-XLDM] §2.1).
pub const PAGE_SIZE: usize = 4096;

const WHAT: &str = "backup header";

/// Fields of the backup header (`BackupLogHeaderType`, [MS-XLDM] §2.1.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct BackupHeader {
    /// `BackupRestoreSyncVersion` (140 in Power BI and Power Pivot files).
    pub backup_restore_sync_version: Option<i64>,
    /// `Fault`.
    pub fault: bool,
    /// `faultcode` (unused by the format).
    pub fault_code: Option<u64>,
    /// `ErrorCode`: every stored file (and the log) ends with a 4-byte CRC marker that is
    /// not part of the file content.
    pub error_code: bool,
    /// `EncryptionFlag`.
    pub encryption_flag: bool,
    /// `EncryptionKey` (unused by the format).
    pub encryption_key: Option<i64>,
    /// `ApplyCompression`: stored files are individually compressed.
    pub apply_compression: bool,
    /// `m_cbOffsetHeader`: absolute offset of the virtual directory.
    pub directory_offset: u64,
    /// `DataSize`: size of the virtual directory in bytes.
    pub directory_size: u64,
    /// `Files`: number of virtual-directory entries.
    pub file_count: Option<u64>,
    /// `ObjectID` (a GUID, unused by the format).
    pub object_id: Option<String>,
    /// `m_cbOffsetData`: start of the files section (normally 4096).
    pub data_offset: Option<u64>,
}

/// Parses the header document that follows the 72-byte signature in the first page.
pub(crate) fn parse_header(data: &[u8]) -> Result<BackupHeader, AbfError> {
    let end = data.len().min(PAGE_SIZE);
    let region = data
        .get(STREAM_STORAGE_SIGNATURE_LEN..end)
        .ok_or(AbfError::OutOfBounds {
            what: WHAT,
            offset: STREAM_STORAGE_SIGNATURE_LEN as u64,
            size: (PAGE_SIZE - STREAM_STORAGE_SIGNATURE_LEN) as u64,
            len: data.len() as u64,
        })?;
    let text = xml::decode_utf16le_until_nul(region, WHAT)?;
    let doc = xml::parse(&text, WHAT)?;
    let root = xml::expect_root(&doc, "BackupLog", WHAT)?;
    Ok(BackupHeader {
        backup_restore_sync_version: xml::opt_i64(root, "BackupRestoreSyncVersion", WHAT)?,
        fault: xml::opt_bool(root, "Fault", WHAT)?.unwrap_or(false),
        fault_code: xml::opt_u64(root, "faultcode", WHAT)?,
        error_code: xml::opt_bool(root, "ErrorCode", WHAT)?.unwrap_or(false),
        encryption_flag: xml::opt_bool(root, "EncryptionFlag", WHAT)?.unwrap_or(false),
        encryption_key: xml::opt_i64(root, "EncryptionKey", WHAT)?,
        apply_compression: xml::opt_bool(root, "ApplyCompression", WHAT)?.unwrap_or(false),
        directory_offset: xml::req_u64(root, "m_cbOffsetHeader", WHAT)?,
        directory_size: xml::req_u64(root, "DataSize", WHAT)?,
        file_count: xml::opt_u64(root, "Files", WHAT)?,
        object_id: xml::opt_text(root, "ObjectID"),
        data_offset: xml::opt_u64(root, "m_cbOffsetData", WHAT)?,
    })
}
