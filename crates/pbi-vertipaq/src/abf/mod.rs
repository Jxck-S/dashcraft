//! Analysis Services backup (ABF) container: header, virtual directory, backup log and
//! file access.
//!
//! An ABF backup — the decompressed PBIX `DataModel`, a standalone `.abf`, PBIP
//! `.pbi/cache.abf`, or Power Pivot `xl/model/item.data` — is laid out in 4 KiB pages:
//!
//! ```text
//! 0      FF FE + UTF-16LE "STREAM_STORAGE_SIGNATURE_)!@#$%^&*("        (72 bytes)
//! 72     UTF-16LE <BackupLog> header, NUL-padded to 4096   -> BackupHeader
//! 4096   stored files back to back (ADDITIONAL_LOG, PARTITIONS, model files, LOG)
//! m_cbOffsetHeader  UTF-8 <VirtualDirectory>: storage name -> offset, size
//! ```
//!
//! The `LOG` entry is a UTF-16 `<BackupLog>` document that maps each storage name to the
//! original server path of the file. [`AbfArchive`] joins both and exposes every model
//! file under a model-relative, `/`-separated **logical path** such as
//! `metadata.sqlitedb` or `Sales (12).tbl/15.prt/80.Sales (12).Amount (20).0.idf`.
//! Absolute server paths (which contain the workstation user profile) are never kept.
//!
//! All input is untrusted: offsets and sizes are bounds-checked, XML is parsed without
//! DTDs and with a node limit, and every malformed input yields an [`AbfError`].
//!
//! References (see `crates/pbi-vertipaq/REFERENCES.md`): [MS-XLDM] §2.1 (#4, page layout,
//! header fields, CRC end markers), pbixray `abf/*.py` (#3, MIT; structure of the header,
//! virtual directory and log and the join on `StoragePath`), and our own fixture
//! observations (`FORMAT-NOTES.md` §4).

mod directory;
mod error;
mod header;
mod log;
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod test_support;
#[cfg(test)]
mod tests;
pub(crate) mod xml;

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

pub use directory::{
    ADDITIONAL_LOG_STORAGE_NAME, DirectoryEntry, LOG_STORAGE_NAME, PARTITIONS_STORAGE_NAME,
};
pub use error::AbfError;
pub use header::{BackupHeader, PAGE_SIZE};
pub use log::{BackupLogInfo, FileGroupInfo};

use crate::xpress9::{DataModelFormat, Limits, decompress_datamodel, detect_format};

/// Maximum number of virtual-directory entries (and backup-log files) accepted.
pub const MAX_ENTRIES: usize = 1 << 20;

/// Largest decompressed size accepted for one file of a backup with `ApplyCompression`
/// (the size recorded in the backup log must not exceed it).
pub const MAX_DECOMPRESSED_FILE_SIZE: u64 = 2 << 30;

/// Size of the CRC end marker that follows each stored file when the header's
/// `ErrorCode` is set ([MS-XLDM] §2.1.2.2.1).
pub const CRC_MARKER_LEN: u64 = 4;

/// File name of the tabular metadata database inside a backup.
pub const METADATA_FILE_NAME: &str = "metadata.sqlitedb";

/// A model file inside an ABF backup.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AbfFile {
    /// Model-relative, `/`-separated path, e.g. `metadata.sqlitedb` or
    /// `T (12).tbl/15.prt/80.T (12).C (20).0.idf`.
    pub logical_path: String,
    /// Virtual-directory storage name (20 hex digits).
    pub storage_path: String,
    /// Absolute offset of the content in the backup.
    pub offset: u64,
    /// Content size in bytes (stored size minus the CRC marker, if any).
    pub size: u64,
    /// Stored size from the virtual directory.
    pub stored_size: u64,
    /// Size recorded in the backup log (the uncompressed size when `ApplyCompression`
    /// is set).
    pub log_size: Option<u64>,
    /// Index into [`BackupLogInfo::file_groups`].
    pub file_group: usize,
    /// `LastWriteTime` from the backup log (FILETIME ticks).
    pub last_write_time: Option<i64>,
}

impl AbfFile {
    /// Last path component.
    pub fn file_name(&self) -> &str {
        self.logical_path
            .rsplit('/')
            .next()
            .unwrap_or(&self.logical_path)
    }

    /// Extension of the file name without the dot (`idf`, `idfmeta`, `dictionary`,
    /// `hidx`, `sqlitedb`, ...).
    pub fn extension(&self) -> Option<&str> {
        self.file_name().rsplit_once('.').map(|(_, ext)| ext)
    }
}

/// A file selector for [`AbfArchive::read`]: a logical path or an [`AbfFile`].
#[derive(Debug, Clone, Copy)]
pub enum FileRef<'a> {
    /// Logical path (`/` or `\` separators, case-insensitive).
    Path(&'a str),
    /// A file entry of this archive.
    File(&'a AbfFile),
}

impl FileRef<'_> {
    /// The logical path this selector refers to.
    pub fn path(&self) -> &str {
        match self {
            FileRef::Path(path) => path,
            FileRef::File(file) => &file.logical_path,
        }
    }
}

impl<'a> From<&'a str> for FileRef<'a> {
    fn from(path: &'a str) -> Self {
        FileRef::Path(path)
    }
}

impl<'a> From<&'a String> for FileRef<'a> {
    fn from(path: &'a String) -> Self {
        FileRef::Path(path)
    }
}

impl<'a> From<&'a AbfFile> for FileRef<'a> {
    fn from(file: &'a AbfFile) -> Self {
        FileRef::File(file)
    }
}

type Backing = Arc<dyn AsRef<[u8]> + Send + Sync>;

struct Inner {
    data: Backing,
    header: BackupHeader,
    directory: Vec<DirectoryEntry>,
    log: BackupLogInfo,
    files: Vec<AbfFile>,
    /// Lower-cased logical paths, parallel to `files`.
    lower: Vec<String>,
    index: HashMap<String, usize>,
}

/// A parsed ABF backup with zero-copy access to its files.
///
/// Cheap to clone (shared, immutable) and `Send + Sync`, so column decoders can read
/// files from several threads.
#[derive(Clone)]
pub struct AbfArchive {
    inner: Arc<Inner>,
}

impl fmt::Debug for AbfArchive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AbfArchive")
            .field("size", &self.size())
            .field("header", &self.inner.header)
            .field("directory_entries", &self.inner.directory.len())
            .field("files", &self.inner.files.len())
            .finish_non_exhaustive()
    }
}

fn range(
    data: &[u8],
    offset: u64,
    size: u64,
    what: &'static str,
) -> Result<std::ops::Range<usize>, AbfError> {
    let out_of_bounds = || AbfError::OutOfBounds {
        what,
        offset,
        size,
        len: data.len() as u64,
    };
    let end = offset.checked_add(size).ok_or_else(out_of_bounds)?;
    if end > data.len() as u64 {
        return Err(out_of_bounds());
    }
    let start = usize::try_from(offset).map_err(|_| out_of_bounds())?;
    let end = usize::try_from(end).map_err(|_| out_of_bounds())?;
    Ok(start..end)
}

fn slice<'d>(
    data: &'d [u8],
    offset: u64,
    size: u64,
    what: &'static str,
) -> Result<&'d [u8], AbfError> {
    let r = range(data, offset, size, what)?;
    data.get(r).ok_or(AbfError::OutOfBounds {
        what,
        offset,
        size,
        len: data.len() as u64,
    })
}

fn content_size(
    stored: u64,
    error_code: bool,
    what: &'static str,
    offset: u64,
) -> Result<u64, AbfError> {
    if !error_code {
        return Ok(stored);
    }
    stored
        .checked_sub(CRC_MARKER_LEN)
        .ok_or(AbfError::OutOfBounds {
            what,
            offset,
            size: CRC_MARKER_LEN,
            len: stored,
        })
}

fn lookup_key(path: &str) -> String {
    log::sanitize(path).to_lowercase()
}

/// Normalises a relative path to the logical form used by [`AbfArchive`]: `/`
/// separators, no empty, `.` or `..` segments.
pub fn sanitize_path(path: &str) -> String {
    log::sanitize(path)
}

impl AbfArchive {
    /// Parses an **uncompressed** ABF backup. Takes ownership of any byte container
    /// (`Vec<u8>`, `Arc<[u8]>`, `Box<[u8]>`, a memory map, ...) without copying.
    ///
    /// Use [`open_pbix_datamodel`] or [`AbfArchive::open`] for input that may still be
    /// XPress9-compressed.
    pub fn parse<B>(bytes: B) -> Result<Self, AbfError>
    where
        B: AsRef<[u8]> + Send + Sync + 'static,
    {
        Self::from_backing(Arc::new(bytes))
    }

    /// Opens a PBIX `DataModel` stream, a standalone `.abf` or a PBIP `cache.abf`: the
    /// bytes are decompressed first if they carry an XPress9 signature, otherwise parsed
    /// in place (no copy).
    pub fn open(bytes: Vec<u8>, limits: &Limits) -> Result<Self, AbfError> {
        match detect_format(&bytes) {
            Some(DataModelFormat::Uncompressed) => Self::parse(bytes),
            Some(_) => Self::parse(decompress_datamodel(&bytes, limits)?),
            None => Err(AbfError::UnknownSignature),
        }
    }

    fn from_backing(data: Backing) -> Result<Self, AbfError> {
        let bytes = (*data).as_ref();
        if detect_format(bytes) != Some(DataModelFormat::Uncompressed) {
            return Err(AbfError::NotAbf);
        }
        let header = header::parse_header(bytes)?;
        if header.encryption_flag {
            return Err(AbfError::Encrypted);
        }
        let directory_bytes = slice(
            bytes,
            header.directory_offset,
            header.directory_size,
            "virtual directory",
        )?;
        let directory = directory::parse_directory(directory_bytes, MAX_ENTRIES)?;
        if let Some(expected) = header.file_count
            && expected != directory.len() as u64
        {
            tracing::warn!(
                expected,
                found = directory.len(),
                "ABF header file count differs from the virtual directory"
            );
        }
        for entry in &directory {
            range(bytes, entry.offset, entry.size, "stored file")?;
        }

        let log_entry = directory
            .iter()
            .find(|e| e.storage_path == LOG_STORAGE_NAME)
            .or_else(|| directory.last())
            .ok_or(AbfError::MissingField {
                what: "virtual directory",
                element: "BackupFile",
            })?;
        let log_size = content_size(
            log_entry.size,
            header.error_code,
            "backup log",
            log_entry.offset,
        )?;
        let log_bytes = slice(bytes, log_entry.offset, log_size, "backup log")?;
        let (log_info, log_files) = log::parse_log(log_bytes, MAX_ENTRIES)?;

        let by_storage: HashMap<&str, &DirectoryEntry> = directory
            .iter()
            .map(|e| (e.storage_path.as_str(), e))
            .collect();
        let mut files = Vec::with_capacity(log_files.len());
        let mut lower = Vec::with_capacity(log_files.len());
        let mut index = HashMap::with_capacity(log_files.len());
        for file in log_files {
            let Some(entry) = by_storage.get(file.storage_path.as_str()) else {
                tracing::warn!(
                    storage = %file.storage_path,
                    "backup log entry has no virtual-directory entry; skipped"
                );
                continue;
            };
            let size = content_size(entry.size, header.error_code, "stored file", entry.offset)?;
            let key = lookup_key(&file.logical_path);
            index.entry(key.clone()).or_insert(files.len());
            lower.push(key);
            files.push(AbfFile {
                logical_path: file.logical_path,
                storage_path: file.storage_path,
                offset: entry.offset,
                size,
                stored_size: entry.size,
                log_size: file.size,
                file_group: file.group,
                last_write_time: file.last_write_time,
            });
        }

        Ok(Self {
            inner: Arc::new(Inner {
                data,
                header,
                directory,
                log: log_info,
                files,
                lower,
                index,
            }),
        })
    }

    /// The whole backup.
    pub fn as_bytes(&self) -> &[u8] {
        (*self.inner.data).as_ref()
    }

    /// Size of the backup in bytes.
    pub fn size(&self) -> u64 {
        self.as_bytes().len() as u64
    }

    /// The backup header.
    pub fn header(&self) -> &BackupHeader {
        &self.inner.header
    }

    /// Summary of the backup log (without absolute paths).
    pub fn log_info(&self) -> &BackupLogInfo {
        &self.inner.log
    }

    /// Raw virtual-directory entries, including `ADDITIONAL_LOG`, `PARTITIONS` and `LOG`.
    pub fn directory(&self) -> &[DirectoryEntry] {
        &self.inner.directory
    }

    /// Model files (backup-log entries joined with the virtual directory), in log order.
    pub fn files(&self) -> &[AbfFile] {
        &self.inner.files
    }

    /// File by logical path (`/` or `\` separators, case-insensitive).
    pub fn find(&self, path: &str) -> Option<&AbfFile> {
        let idx = *self.inner.index.get(&lookup_key(path))?;
        self.inner.files.get(idx)
    }

    /// First file whose logical path ends with `suffix` (case-insensitive; `\` and `/`
    /// are equivalent), e.g. `find_suffix("metadata.sqlitedb")`.
    pub fn find_suffix(&self, suffix: &str) -> Option<&AbfFile> {
        self.files_with_suffix(suffix).next()
    }

    /// All files whose logical path ends with `suffix` (case-insensitive), e.g.
    /// `files_with_suffix(".dictionary")`.
    pub fn files_with_suffix<'a>(&'a self, suffix: &str) -> impl Iterator<Item = &'a AbfFile> + 'a {
        let suffix = suffix.replace('\\', "/").to_lowercase();
        self.inner
            .files
            .iter()
            .zip(&self.inner.lower)
            .filter(move |(_, lower)| lower.ends_with(&suffix))
            .map(|(file, _)| file)
    }

    /// First file whose last path component equals `name` (case-insensitive).
    pub fn find_file_name(&self, name: &str) -> Option<&AbfFile> {
        let name = name.to_lowercase();
        self.inner
            .files
            .iter()
            .zip(&self.inner.lower)
            .find(|(_, lower)| lower.rsplit('/').next() == Some(name.as_str()))
            .map(|(file, _)| file)
    }

    /// The tabular metadata database (`metadata.sqlitedb`), preferring a top-level one.
    pub fn metadata_file(&self) -> Option<&AbfFile> {
        self.find(METADATA_FILE_NAME)
            .or_else(|| self.find_file_name(METADATA_FILE_NAME))
    }

    /// `(offset, size)` of a file's content.
    fn locate(&self, file: FileRef<'_>) -> Result<(u64, u64), AbfError> {
        match file {
            FileRef::File(file) => Ok((file.offset, file.size)),
            FileRef::Path(path) => self
                .find(path)
                .map(|f| (f.offset, f.size))
                .ok_or_else(|| AbfError::FileNotFound(path.to_owned())),
        }
    }

    /// `true` when the files of this backup are stored compressed (`ApplyCompression`,
    /// as in Power Pivot workbooks); [`AbfArchive::read_file`] decompresses them.
    pub fn is_compressed(&self) -> bool {
        self.inner.header.apply_compression
    }

    /// Content of a file: borrowed (zero-copy) from an uncompressed backup, or
    /// decompressed ([`crate::xpress8`]) when the backup sets `ApplyCompression`. The
    /// decompressed size must equal the size in the backup log and is capped at
    /// [`MAX_DECOMPRESSED_FILE_SIZE`].
    pub fn read_file<'p>(&self, file: impl Into<FileRef<'p>>) -> Result<Cow<'_, [u8]>, AbfError> {
        let file = file.into();
        let (offset, size) = self.locate(file)?;
        let stored = slice(self.as_bytes(), offset, size, "file")?;
        if !self.inner.header.apply_compression {
            return Ok(Cow::Borrowed(stored));
        }
        let expected = match file {
            FileRef::File(f) => f.log_size,
            FileRef::Path(path) => self.find(path).and_then(|f| f.log_size),
        };
        crate::xpress8::decompress_chunked(stored, expected, MAX_DECOMPRESSED_FILE_SIZE)
            .map(Cow::Owned)
            .map_err(|source| AbfError::Decompress {
                path: file.path().to_owned(),
                source,
            })
    }

    /// Content of a file (zero-copy).
    ///
    /// Fails with [`AbfError::CompressedFile`] when the backup stores files compressed
    /// (`ApplyCompression`); use [`AbfArchive::read_file`] to decompress them, or
    /// [`AbfArchive::read_stored`] for the raw bytes.
    pub fn read<'p>(&self, file: impl Into<FileRef<'p>>) -> Result<&[u8], AbfError> {
        let file = file.into();
        let (offset, size) = self.locate(file)?;
        if self.inner.header.apply_compression {
            return Err(AbfError::CompressedFile {
                path: file.path().to_owned(),
            });
        }
        slice(self.as_bytes(), offset, size, "file")
    }

    /// Stored bytes of a file without the CRC marker, even when they are compressed.
    pub fn read_stored<'p>(&self, file: impl Into<FileRef<'p>>) -> Result<&[u8], AbfError> {
        let (offset, size) = self.locate(file.into())?;
        slice(self.as_bytes(), offset, size, "file")
    }

    /// Stored bytes (without the CRC marker) of a virtual-directory entry by storage
    /// name, e.g. `LOG` or `PARTITIONS`.
    pub fn read_storage(&self, storage_path: &str) -> Result<&[u8], AbfError> {
        let entry = self
            .inner
            .directory
            .iter()
            .find(|e| e.storage_path == storage_path)
            .ok_or_else(|| AbfError::FileNotFound(storage_path.to_owned()))?;
        let size = content_size(
            entry.size,
            self.inner.header.error_code,
            "stored file",
            entry.offset,
        )?;
        slice(self.as_bytes(), entry.offset, size, "stored file")
    }
}

/// Identifies the container format of `bytes` (first 102 bytes suffice): an
/// uncompressed ABF backup (`.abf`, PBIP `cache.abf`) or an XPress9-compressed PBIX
/// `DataModel`.
#[must_use]
pub fn sniff(bytes: &[u8]) -> Option<DataModelFormat> {
    detect_format(bytes)
}

/// Opens a PBIX `DataModel` entry (XPress9 single- or multi-threaded) or an uncompressed
/// ABF backup and parses the backup.
///
/// Uncompressed input is copied once; use [`AbfArchive::open`] with an owned `Vec` to
/// avoid the copy.
pub fn open_pbix_datamodel(
    datamodel_bytes: &[u8],
    limits: &Limits,
) -> Result<AbfArchive, AbfError> {
    match detect_format(datamodel_bytes) {
        Some(DataModelFormat::Uncompressed) => AbfArchive::parse(datamodel_bytes.to_vec()),
        Some(_) => AbfArchive::parse(decompress_datamodel(datamodel_bytes, limits)?),
        None => Err(AbfError::UnknownSignature),
    }
}

/// Fuzzing entry point: opens `data` as a `DataModel`/ABF with [`Limits::SMALL`], reads
/// every file and, if present, the tabular metadata. Never panics.
pub fn fuzz_abf(data: &[u8]) {
    let archive = match open_pbix_datamodel(data, &Limits::SMALL) {
        Ok(archive) => archive,
        Err(_) => return,
    };
    let _ = format!("{archive:?}");
    for file in archive.files() {
        let _ = archive.read(file);
        let _ = archive.read_file(file);
        let _ = archive.find(&file.logical_path);
    }
    for entry in archive.directory() {
        let _ = archive.read_storage(&entry.storage_path);
    }
    let _ = archive.find_suffix(".idf").map(AbfFile::extension);
    if let Ok(db) = crate::metadata::MetadataDb::from_archive(&archive) {
        let _ = db.load(&crate::metadata::ModelOptions::default());
        let _ = crate::metadata::load_model(&archive, &crate::metadata::ModelOptions::default());
    }
}
