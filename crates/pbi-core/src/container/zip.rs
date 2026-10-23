//! ZIP container (PBIX, PBIT, XLSX, VPAX, PBIVIZ, PQT, ...).
//!
//! Defenses, all applied in [`ZipContainer::open`] before any entry is
//! decompressed:
//!
//! - the declared entry count is read from the end-of-central-directory
//!   record and checked *before* the central directory is parsed;
//! - every name is validated (no absolute paths, `..`, drive letters, NUL;
//!   `\` is normalized to `/` first); one bad name refuses the archive;
//! - raw duplicate names refuse the archive; names that only collide after
//!   normalization keep the first entry and are reported in
//!   [`Container::skipped`];
//! - declared sizes are checked against the per-entry, total and ratio caps
//!   (per entry and archive-wide), and overlapping entries (shared
//!   compressed data, a zip-bomb technique) are refused.
//!
//! Each [`Container::read`] decompresses through a reader bounded to the
//! declared size + 1 byte, so an entry that lies about its size fails with
//! [`LimitKind::DeclaredSize`] instead of exhausting memory. Only stored and
//! deflate entries are decoded; encrypted entries and other methods yield
//! [`ContainerError::Unsupported`].

use std::collections::HashSet;
use std::fmt;
use std::fs::File;
use std::io::{self, BufReader, Cursor, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use ::zip::result::ZipError;
use ::zip::{CompressionMethod, ZipArchive};

use super::path::{normalize_entry_name, normalize_lookup};
use super::{
    Container, ContainerError, ContainerKind, EntryInfo, LimitKind, Limits, SkipReason,
    SkippedEntry, UnsafePathReason,
};

/// Upper bound for the up-front allocation of a read buffer.
const PREALLOC_MAX: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Method {
    Stored,
    Deflated,
    Other(String),
}

#[derive(Debug, Clone)]
struct Slot {
    zip_index: usize,
    encrypted: bool,
    method: Method,
}

enum Backing {
    /// Cloned per read (cheap: `Arc` buffer + `Arc` metadata), so reads run
    /// in parallel.
    Bytes(ZipArchive<Cursor<Arc<[u8]>>>),
    /// One file handle; reads are serialized.
    File(Mutex<ZipArchive<BufReader<File>>>),
}

/// A validated, read-only ZIP archive. See the [module docs](self).
pub struct ZipContainer {
    label: String,
    limits: Limits,
    entries: Vec<EntryInfo>,
    slots: Vec<Slot>,
    skipped: Vec<SkippedEntry>,
    backing: Backing,
}

impl fmt::Debug for ZipContainer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ZipContainer")
            .field("label", &self.label)
            .field("limits", &self.limits)
            .field("entries", &self.entries.len())
            .field("skipped", &self.skipped)
            .finish_non_exhaustive()
    }
}

struct Listing {
    entries: Vec<EntryInfo>,
    slots: Vec<Slot>,
    skipped: Vec<SkippedEntry>,
}

impl ZipContainer {
    /// Opens and validates a ZIP file. The file stays open for later reads.
    ///
    /// # Errors
    /// [`ContainerError::UnsafePath`], [`ContainerError::LimitExceeded`],
    /// [`ContainerError::Corrupt`] (not a ZIP, bad headers) or I/O errors.
    pub fn open(path: impl AsRef<Path>, limits: Limits) -> Result<Self, ContainerError> {
        let path = path.as_ref();
        let label = path.display().to_string();
        // Refuse FIFOs/devices up front: opening a FIFO blocks.
        let meta = std::fs::metadata(path).map_err(|e| ContainerError::io(&label, e))?;
        if !meta.is_file() {
            return Err(ContainerError::InvalidRoot {
                path: label,
                detail: "not a regular file".into(),
            });
        }
        let file = File::open(path).map_err(|e| ContainerError::io(&label, e))?;
        let (archive, listing) = scan(BufReader::new(file), &label, &limits)?;
        Ok(Self::assemble(
            label,
            limits,
            listing,
            Backing::File(Mutex::new(archive)),
        ))
    }

    /// Opens and validates a ZIP archive held in memory.
    ///
    /// # Errors
    /// As [`open`](Self::open).
    pub fn from_bytes(bytes: Vec<u8>, limits: Limits) -> Result<Self, ContainerError> {
        Self::from_bytes_labeled(bytes, "<memory>", limits)
    }

    /// Like [`from_bytes`](Self::from_bytes) with a label for messages.
    ///
    /// # Errors
    /// As [`open`](Self::open).
    pub fn from_bytes_labeled(
        bytes: Vec<u8>,
        label: impl Into<String>,
        limits: Limits,
    ) -> Result<Self, ContainerError> {
        let label = label.into();
        let data: Arc<[u8]> = Arc::from(bytes);
        let (archive, listing) = scan(Cursor::new(data), &label, &limits)?;
        Ok(Self::assemble(
            label,
            limits,
            listing,
            Backing::Bytes(archive),
        ))
    }

    /// The limits this container enforces.
    #[must_use]
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    fn assemble(label: String, limits: Limits, listing: Listing, backing: Backing) -> Self {
        Self {
            label,
            limits,
            entries: listing.entries,
            slots: listing.slots,
            skipped: listing.skipped,
            backing,
        }
    }
}

impl Container for ZipContainer {
    fn kind(&self) -> ContainerKind {
        ContainerKind::Zip
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
        let not_found = || ContainerError::NotFound(path.to_owned());
        let key = normalize_lookup(path).ok_or_else(not_found)?;
        let i = self
            .entries
            .binary_search_by(|e| e.path.as_str().cmp(key.as_str()))
            .map_err(|_| not_found())?;
        let (Some(entry), Some(slot)) = (self.entries.get(i), self.slots.get(i)) else {
            return Err(not_found());
        };
        if entry.is_dir {
            return Err(ContainerError::IsDirectory(entry.path.clone()));
        }
        if slot.encrypted {
            return Err(ContainerError::Unsupported(format!(
                "{}: encrypted ZIP entry",
                entry.path
            )));
        }
        if let Method::Other(m) = &slot.method {
            return Err(ContainerError::Unsupported(format!(
                "{}: compression method {m} (only stored and deflate are supported)",
                entry.path
            )));
        }
        match &self.backing {
            Backing::Bytes(archive) => {
                let mut archive = archive.clone();
                read_entry(&mut archive, slot.zip_index, entry, &self.limits)
            }
            Backing::File(archive) => {
                let mut guard = archive.lock().unwrap_or_else(PoisonError::into_inner);
                read_entry(&mut guard, slot.zip_index, entry, &self.limits)
            }
        }
    }

    fn as_dyn(&self) -> &dyn Container {
        self
    }
}

fn scan<R: Read + Seek>(
    mut reader: R,
    label: &str,
    limits: &Limits,
) -> Result<(ZipArchive<R>, Listing), ContainerError> {
    let io_err = |e| ContainerError::io(label, e);
    let file_len = reader.seek(SeekFrom::End(0)).map_err(io_err)?;
    let declared = declared_entry_count(&mut reader, file_len).map_err(io_err)?;
    if let Some(n) = declared
        && n > limits.max_entries as u64
    {
        return Err(ContainerError::limit(
            LimitKind::Entries,
            format!("archive declares {n} entries (max {})", limits.max_entries),
        ));
    }
    reader.seek(SeekFrom::Start(0)).map_err(io_err)?;
    let mut archive = ZipArchive::new(reader).map_err(|e| zip_error(label, e))?;
    let count = archive.len();
    if count > limits.max_entries {
        return Err(ContainerError::limit(
            LimitKind::Entries,
            format!("archive has {count} entries (max {})", limits.max_entries),
        ));
    }
    // The zip crate reads exactly the declared number of central-directory
    // records and keys them by raw name, so fewer distinct names than
    // records means raw duplicates.
    if declared.is_some_and(|n| n > count as u64) {
        return Err(ContainerError::unsafe_path(
            label,
            UnsafePathReason::DuplicateName,
        ));
    }

    let mut rows: Vec<(EntryInfo, Slot)> = Vec::with_capacity(count);
    let mut seen: HashSet<String> = HashSet::with_capacity(count);
    let mut skipped = Vec::new();
    let mut total: u64 = 0;
    for zip_index in 0..count {
        let e = archive
            .by_index_data(zip_index)
            .map_err(|e| zip_error(label, e))?;
        if e.name_raw().contains(&0) {
            return Err(ContainerError::unsafe_path(
                String::from_utf8_lossy(e.name_raw()).replace('\0', "\\0"),
                UnsafePathReason::Nul,
            ));
        }
        let raw = e.name().map_err(|err| zip_error(label, err))?;
        let Some(name) = normalize_entry_name(&raw, limits)? else {
            skipped.push(SkippedEntry {
                path: raw.into_owned(),
                reason: SkipReason::RootEntry,
            });
            continue;
        };
        if e.is_symlink() {
            skipped.push(SkippedEntry {
                path: name.path,
                reason: SkipReason::Symlink,
            });
            continue;
        }
        let is_dir = name.dir_hint || e.is_dir();
        let compressed = e.compressed_size();
        let size = if is_dir { 0 } else { e.size() };
        if !is_dir {
            if size > limits.max_entry_size {
                return Err(ContainerError::limit(
                    LimitKind::EntrySize,
                    format!(
                        "{} declares {size} bytes (max {})",
                        name.path, limits.max_entry_size
                    ),
                ));
            }
            if limits.ratio_exceeded(size, compressed) {
                return Err(ContainerError::limit(
                    LimitKind::CompressionRatio,
                    format!(
                        "{} expands {compressed} -> {size} bytes (max {}:1)",
                        name.path, limits.max_compression_ratio
                    ),
                ));
            }
            total = total.saturating_add(size);
            if total > limits.max_total_size {
                return Err(ContainerError::limit(
                    LimitKind::TotalSize,
                    format!(
                        "declared uncompressed total exceeds {} bytes",
                        limits.max_total_size
                    ),
                ));
            }
        }
        if !seen.insert(name.path.clone()) {
            skipped.push(SkippedEntry {
                path: name.path,
                reason: SkipReason::Duplicate,
            });
            continue;
        }
        let method = match e.compression() {
            CompressionMethod::Stored => Method::Stored,
            CompressionMethod::Deflated => Method::Deflated,
            other => Method::Other(other.to_string()),
        };
        rows.push((
            EntryInfo {
                path: name.path,
                size,
                compressed_size: Some(compressed),
                is_dir,
            },
            Slot {
                zip_index,
                encrypted: e.encrypted(),
                method,
            },
        ));
    }
    if limits.ratio_exceeded(total, file_len) {
        return Err(ContainerError::limit(
            LimitKind::CompressionRatio,
            format!(
                "archive of {file_len} bytes declares {total} uncompressed bytes (max {}:1)",
                limits.max_compression_ratio
            ),
        ));
    }
    if archive
        .has_overlapping_files()
        .map_err(|e| zip_error(label, e))?
    {
        return Err(ContainerError::limit(
            LimitKind::OverlappingEntries,
            "entries share compressed data (overlapping-file zip bomb)",
        ));
    }
    rows.sort_by(|a, b| a.0.path.cmp(&b.0.path));
    let (entries, slots) = rows.into_iter().unzip();
    Ok((
        archive,
        Listing {
            entries,
            slots,
            skipped,
        },
    ))
}

fn read_entry<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    index: usize,
    entry: &EntryInfo,
    limits: &Limits,
) -> Result<Vec<u8>, ContainerError> {
    let declared = entry.size;
    if declared > limits.max_entry_size {
        return Err(ContainerError::limit(
            LimitKind::EntrySize,
            format!("{} declares {declared} bytes", entry.path),
        ));
    }
    let mut file = archive
        .by_index(index)
        .map_err(|e| zip_error(&entry.path, e))?;
    let mut out = Vec::new();
    let prealloc = usize::try_from(declared.min(PREALLOC_MAX)).unwrap_or(0);
    out.try_reserve_exact(prealloc)
        .map_err(|e| ContainerError::io(&entry.path, io::Error::other(e)))?;
    let mut bounded = (&mut file).take(declared.saturating_add(1));
    if let Err(e) = bounded.read_to_end(&mut out) {
        return Err(read_error(&entry.path, e));
    }
    let got = out.len() as u64;
    if got > declared {
        return Err(ContainerError::limit(
            LimitKind::DeclaredSize,
            format!(
                "{} produced more than its declared {declared} bytes",
                entry.path
            ),
        ));
    }
    if got < declared {
        return Err(ContainerError::Corrupt(format!(
            "{}: {got} bytes instead of the declared {declared}",
            entry.path
        )));
    }
    let compressed = entry.compressed_size.unwrap_or(0);
    if limits.ratio_exceeded(got, compressed) {
        return Err(ContainerError::limit(
            LimitKind::CompressionRatio,
            format!("{} expanded {compressed} -> {got} bytes", entry.path),
        ));
    }
    Ok(out)
}

fn read_error(path: &str, e: io::Error) -> ContainerError {
    let msg = e.to_string();
    // The zip crate itself refuses output beyond the declared size.
    if msg.contains("declared") {
        return ContainerError::limit(
            LimitKind::DeclaredSize,
            format!("{path} produced more than its declared size"),
        );
    }
    match e.kind() {
        io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof | io::ErrorKind::InvalidInput => {
            ContainerError::Corrupt(format!("{path}: {msg}"))
        }
        _ => ContainerError::io(path, e),
    }
}

fn zip_error(context: &str, e: ZipError) -> ContainerError {
    match e {
        ZipError::Io(err) => match err.kind() {
            io::ErrorKind::UnexpectedEof | io::ErrorKind::InvalidData => {
                ContainerError::Corrupt(format!("{context}: {err}"))
            }
            _ => ContainerError::io(context, err),
        },
        ZipError::UnsupportedArchive(m) if m == ZipError::PASSWORD_REQUIRED => {
            ContainerError::Unsupported(format!("{context}: encrypted ZIP entry"))
        }
        ZipError::UnsupportedArchive(m) => ContainerError::Unsupported(format!("{context}: {m}")),
        ZipError::CompressionMethodNotSupported(m) => {
            ContainerError::Unsupported(format!("{context}: compression method {m}"))
        }
        ZipError::FileNotFound => ContainerError::NotFound(context.to_owned()),
        other => ContainerError::Corrupt(format!("{context}: {other}")),
    }
}

const EOCD_SIG: [u8; 4] = *b"PK\x05\x06";
const ZIP64_LOCATOR_SIG: [u8; 4] = *b"PK\x06\x07";
const ZIP64_EOCD_SIG: [u8; 4] = *b"PK\x06\x06";
const EOCD_LEN: usize = 22;
const ZIP64_LOCATOR_LEN: usize = 20;
const ZIP64_EOCD_LEN: u64 = 56;

/// Total entry count declared by the last end-of-central-directory record
/// (following the ZIP64 locator when needed); `None` if it cannot be found
/// (the zip crate then reports the archive as corrupt, or the post-parse
/// count check applies).
fn declared_entry_count<R: Read + Seek>(r: &mut R, file_len: u64) -> io::Result<Option<u64>> {
    let max_tail = (EOCD_LEN + 0xFFFF + ZIP64_LOCATOR_LEN) as u64;
    let tail_len = file_len.min(max_tail);
    let Ok(tail_usize) = usize::try_from(tail_len) else {
        return Ok(None);
    };
    if tail_usize < EOCD_LEN {
        return Ok(None);
    }
    let tail_start = file_len - tail_len;
    r.seek(SeekFrom::Start(tail_start))?;
    let mut tail = vec![0u8; tail_usize];
    r.read_exact(&mut tail)?;
    let Some(pos) = (0..=tail.len() - EOCD_LEN)
        .rev()
        .find(|&i| tail[i..i + 4] == EOCD_SIG)
    else {
        return Ok(None);
    };
    let Some(total) = le_u16(&tail, pos + 10) else {
        return Ok(None);
    };
    if total != 0xFFFF {
        return Ok(Some(u64::from(total)));
    }
    let Some(loc) = pos.checked_sub(ZIP64_LOCATOR_LEN) else {
        return Ok(None);
    };
    if tail[loc..loc + 4] != ZIP64_LOCATOR_SIG {
        return Ok(None);
    }
    let loc_abs = tail_start + loc as u64;
    let candidates = [le_u64(&tail, loc + 8), loc_abs.checked_sub(ZIP64_EOCD_LEN)];
    for at in candidates.into_iter().flatten() {
        if at
            .checked_add(ZIP64_EOCD_LEN)
            .is_none_or(|end| end > file_len)
        {
            continue;
        }
        r.seek(SeekFrom::Start(at))?;
        let mut rec = [0u8; ZIP64_EOCD_LEN as usize];
        r.read_exact(&mut rec)?;
        if rec[..4] == ZIP64_EOCD_SIG {
            return Ok(le_u64(&rec, 32));
        }
    }
    Ok(None)
}

fn le_u16(b: &[u8], at: usize) -> Option<u16> {
    let bytes: [u8; 2] = b.get(at..at.checked_add(2)?)?.try_into().ok()?;
    Some(u16::from_le_bytes(bytes))
}

fn le_u64(b: &[u8], at: usize) -> Option<u64> {
    let bytes: [u8; 8] = b.get(at..at.checked_add(8)?)?.try_into().ok()?;
    Some(u64::from_le_bytes(bytes))
}
