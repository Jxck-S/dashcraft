//! Recovery journals (M8.1, [ADR-0013] §5): the unsaved edits of open
//! documents, kept in the per-user data folder (never in the project) so
//! that they survive a crash.
//!
//! Recovery is opt-in: a host turns it on with
//! [`Workspace::set_recovery`](crate::Workspace::set_recovery). Each
//! document gets a journal on its first edit: a JSON-lines file whose first
//! line says how to reopen the document (the `doc.open` parameters) and
//! whose next lines are the registry commands that changed its revision, in
//! order. `doc.recovery.restore` reopens the document and replays them
//! through the registry (object ids are assigned deterministically, so the
//! ids the commands name match). A save, Save As, reload or explicit close
//! deletes the journal; a crash leaves it for `doc.recovery.list`.
//!
//! [ADR-0013]: ../../../docs/adr/0013-save-conflicts-and-recovery.md

use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::OpError;

/// The `format` of a journal's first line.
pub const JOURNAL_FORMAT: &str = "dashcraft.recovery";
/// The journal format version this build writes and reads.
pub const JOURNAL_VERSION: u32 = 1;
/// Journals larger than this are not read.
const MAX_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;
const EXTENSION: &str = "jsonl";

/// The first line of a journal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalHeader {
    /// [`JOURNAL_FORMAT`].
    pub format: String,
    /// [`JOURNAL_VERSION`].
    pub format_version: u32,
    /// The document's path (absolute).
    pub path: String,
    /// The `doc.open` parameters that reopen the document.
    pub open: Value,
    /// When the journal was started (RFC 3339, UTC).
    pub created: String,
    /// The DashCraft version that wrote it.
    pub dashcraft_version: String,
}

/// One journaled command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEntry {
    /// The command id (`model.measure.create`, `doc.undo`, …).
    pub command: String,
    /// Its parameters as run (`docId` is replaced on replay).
    pub params: Value,
    /// The document revision after the command.
    pub revision: u64,
    /// The undo label after the command (what the next undo would undo).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// A journal read back.
#[derive(Debug, Clone, PartialEq)]
pub struct Journal {
    pub header: JournalHeader,
    pub entries: Vec<JournalEntry>,
}

/// A journal found by [`RecoveryStore::list`].
#[derive(Debug, Clone, PartialEq)]
pub struct JournalInfo {
    /// The recovery id (the file name without `.jsonl`).
    pub id: String,
    /// The header, when the journal can be read.
    pub header: Option<JournalHeader>,
    /// Journaled commands.
    pub edits: usize,
    /// The undo label after the last journaled command.
    pub last_edit: Option<String>,
    /// Last write (RFC 3339, UTC), when known.
    pub updated: Option<String>,
    /// Why the journal cannot be restored (unreadable, newer format, …).
    pub problem: Option<String>,
}

/// The folder of the recovery journals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryStore {
    dir: PathBuf,
}

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

impl RecoveryStore {
    /// Journals in `dir` (created on the first journal).
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The default folder: `$DASHCRAFT_DATA_DIR/recovery`, else
    /// `dashcraft/recovery` under the platform's per-user data folder
    /// (`%LOCALAPPDATA%` on Windows, `~/Library/Application Support` on
    /// macOS, `$XDG_DATA_HOME` or `~/.local/share` elsewhere). `None` when
    /// no folder can be found.
    #[must_use]
    pub fn default_location() -> Option<PathBuf> {
        Self::default_location_from(|name| std::env::var_os(name), std::env::consts::OS)
    }

    /// [`default_location`](Self::default_location) with an explicit
    /// environment and OS (testable).
    #[must_use]
    pub fn default_location_from(
        env: impl Fn(&str) -> Option<std::ffi::OsString>,
        os: &str,
    ) -> Option<PathBuf> {
        let var = |name: &str| env(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        if let Some(dir) = var("DASHCRAFT_DATA_DIR") {
            return Some(dir.join("recovery"));
        }
        let base = match os {
            "windows" => var("LOCALAPPDATA"),
            "macos" => var("HOME").map(|h| h.join("Library").join("Application Support")),
            _ => {
                var("XDG_DATA_HOME").or_else(|| var("HOME").map(|h| h.join(".local").join("share")))
            }
        }?;
        Some(base.join("dashcraft").join("recovery"))
    }

    /// The journal folder.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn file(&self, id: &str) -> Result<PathBuf, OpError> {
        if !valid_id(id) {
            return Err(OpError::invalid_params(format!(
                "'{id}' is not a recovery id (list them with doc.recovery.list)"
            )));
        }
        Ok(self.dir.join(format!("{id}.{EXTENSION}")))
    }

    /// Starts a journal with `header`; returns its id.
    pub(crate) fn create(&self, header: &JournalHeader) -> io::Result<String> {
        fs::create_dir_all(&self.dir)?;
        let n = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let id = format!(
            "{:016x}-{}-{}-{n}",
            fnv1a(header.path.as_bytes()),
            std::process::id(),
            chrono::Utc::now().timestamp_millis()
        );
        let path = self.dir.join(format!("{id}.{EXTENSION}"));
        let mut file = new_file(&path)?;
        file.write_all(&line(header)?)?;
        Ok(id)
    }

    /// Appends `entry` to the journal `id`.
    pub(crate) fn append(&self, id: &str, entry: &JournalEntry) -> io::Result<()> {
        let path = self
            .file(id)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.message))?;
        let mut file = fs::OpenOptions::new().append(true).open(path)?;
        file.write_all(&line(entry)?)
    }

    /// Reads the journal `id`. A last line cut short (a crash while it was
    /// written) is ignored.
    ///
    /// # Errors
    /// `invalidParams` for a malformed id, `notFound` without such a
    /// journal, `unsupported` for a journal of another format or a newer
    /// version, `io` when it cannot be read.
    pub fn read(&self, id: &str) -> Result<Journal, OpError> {
        let path = self.file(id)?;
        let file = fs::File::open(&path).map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                OpError::not_found(format!("no recovery journal '{id}'"))
            } else {
                OpError::io(format!("cannot read {}: {e}", path.display()))
            }
        })?;
        read_journal(file).map_err(|e| match e {
            ReadError::Io(e) => OpError::io(format!("cannot read {}: {e}", path.display())),
            ReadError::Invalid(message) => {
                OpError::unsupported(format!("recovery journal '{id}': {message}"))
            }
        })
    }

    /// Deletes the journal `id`; `false` when there was none.
    ///
    /// # Errors
    /// `invalidParams` for a malformed id, `io` when it cannot be deleted.
    pub fn discard(&self, id: &str) -> Result<bool, OpError> {
        let path = self.file(id)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(OpError::io(format!(
                "cannot delete {}: {e}",
                path.display()
            ))),
        }
    }

    /// The journals in the folder (none when it does not exist), oldest
    /// first.
    ///
    /// # Errors
    /// `io` when the folder cannot be read.
    pub fn list(&self) -> Result<Vec<JournalInfo>, OpError> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => {
                return Err(OpError::io(format!(
                    "cannot read {}: {e}",
                    self.dir.display()
                )));
            }
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some(EXTENSION) {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if !valid_id(id) {
                continue;
            }
            let updated = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .map(|t| rfc3339(chrono::DateTime::<chrono::Utc>::from(t)));
            let info = match self.read(id) {
                Ok(journal) => JournalInfo {
                    id: id.to_owned(),
                    edits: journal.entries.len(),
                    last_edit: journal.entries.last().and_then(|e| e.label.clone()),
                    header: Some(journal.header),
                    updated,
                    problem: None,
                },
                Err(e) => JournalInfo {
                    id: id.to_owned(),
                    header: None,
                    edits: 0,
                    last_edit: None,
                    updated,
                    problem: Some(e.message),
                },
            };
            out.push(info);
        }
        out.sort_by(|a, b| {
            let created = |i: &JournalInfo| i.header.as_ref().map(|h| h.created.clone());
            created(a).cmp(&created(b)).then_with(|| a.id.cmp(&b.id))
        });
        Ok(out)
    }
}

/// The time as RFC 3339 (UTC, seconds).
/// The undo and redo steps a journal holds: an undo past its first edit, or
/// a redo of a step undone before it, cannot be replayed from the saved
/// document.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct JournalDepth {
    undo: u32,
    redo: u32,
}

impl JournalDepth {
    /// The depth after `command`; `None` when it reaches before the journal.
    pub(crate) fn after(self, command: &str) -> Option<Self> {
        match command {
            "doc.undo" => Some(Self {
                undo: self.undo.checked_sub(1)?,
                redo: self.redo.saturating_add(1),
            }),
            "doc.redo" => Some(Self {
                undo: self.undo.saturating_add(1),
                redo: self.redo.checked_sub(1)?,
            }),
            _ => Some(Self {
                undo: self.undo.saturating_add(1),
                redo: 0,
            }),
        }
    }

    /// The depth after replaying `entries` from the saved document.
    pub(crate) fn of(entries: &[JournalEntry]) -> Option<Self> {
        entries
            .iter()
            .try_fold(Self::default(), |depth, e| depth.after(&e.command))
    }
}

pub(crate) fn rfc3339(t: chrono::DateTime<chrono::Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 80 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn line(value: &impl Serialize) -> io::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[cfg(unix)]
fn new_file(path: &Path) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn new_file(path: &Path) -> io::Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

enum ReadError {
    Io(io::Error),
    Invalid(String),
}

fn read_journal(file: fs::File) -> Result<Journal, ReadError> {
    let size = file.metadata().map_err(ReadError::Io)?.len();
    if size > MAX_JOURNAL_BYTES {
        return Err(ReadError::Invalid(format!(
            "{size} bytes is more than the {MAX_JOURNAL_BYTES} read"
        )));
    }
    let mut lines = io::BufReader::new(file).lines();
    let first = lines
        .next()
        .transpose()
        .map_err(ReadError::Io)?
        .ok_or_else(|| ReadError::Invalid("empty".into()))?;
    let header: JournalHeader = serde_json::from_str(&first)
        .map_err(|e| ReadError::Invalid(format!("not a recovery journal ({e})")))?;
    if header.format != JOURNAL_FORMAT {
        return Err(ReadError::Invalid(format!(
            "format '{}' is not {JOURNAL_FORMAT}",
            header.format
        )));
    }
    if header.format_version > JOURNAL_VERSION {
        return Err(ReadError::Invalid(format!(
            "written by a newer DashCraft ({}, format version {})",
            header.dashcraft_version, header.format_version
        )));
    }
    let lines: Vec<String> = lines.collect::<Result<_, _>>().map_err(ReadError::Io)?;
    let mut entries = Vec::with_capacity(lines.len());
    let last = lines.len().saturating_sub(1);
    for (i, text) in lines.iter().enumerate() {
        if text.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<JournalEntry>(text) {
            Ok(entry) => entries.push(entry),
            Err(_) if i == last => break,
            Err(e) => {
                return Err(ReadError::Invalid(format!("entry {} ({e})", i + 1)));
            }
        }
    }
    Ok(Journal { header, entries })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn header(path: &str) -> JournalHeader {
        JournalHeader {
            format: JOURNAL_FORMAT.into(),
            format_version: JOURNAL_VERSION,
            path: path.into(),
            open: json!({"path": path}),
            created: "2026-11-11T00:00:00Z".into(),
            dashcraft_version: "0.0.0".into(),
        }
    }

    fn entry(n: u64) -> JournalEntry {
        JournalEntry {
            command: "doc.undo".into(),
            params: json!({"docId": "d1"}),
            revision: n,
            label: Some(format!("edit {n}")),
        }
    }

    fn tmp() -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/tmp/recovery-unit")
            .join(format!(
                "{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn journals_round_trip_and_tolerate_a_cut_last_line() {
        let dir = tmp();
        let store = RecoveryStore::new(&dir);
        assert_eq!(store.list().map(|l| l.len()).ok(), Some(0));
        let id = store.create(&header("/p/A.pbip")).unwrap();
        store.append(&id, &entry(1)).unwrap();
        store.append(&id, &entry(2)).unwrap();
        let journal = store.read(&id).unwrap();
        assert_eq!(journal.entries, vec![entry(1), entry(2)]);

        let path = dir.join(format!("{id}.jsonl"));
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(br#"{"command": "doc.re"#).unwrap();
        drop(file);
        assert_eq!(store.read(&id).unwrap().entries.len(), 2);
        let list = store.list().unwrap();
        assert_eq!(
            (list.len(), list[0].edits, list[0].last_edit.as_deref()),
            (1, 2, Some("edit 2"))
        );

        assert!(store.discard(&id).unwrap());
        assert!(!store.discard(&id).unwrap());
        assert_eq!(
            store.read(&id).unwrap_err().code,
            crate::OpErrorCode::NotFound
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn foreign_newer_and_malformed_journals_are_refused() {
        let dir = tmp();
        let store = RecoveryStore::new(&dir);
        assert_eq!(
            store.read("../x").unwrap_err().code,
            crate::OpErrorCode::InvalidParams
        );
        fs::create_dir_all(&dir).unwrap();
        let mut newer = header("/p/B.pbip");
        newer.format_version = JOURNAL_VERSION + 1;
        fs::write(dir.join("newer.jsonl"), line(&newer).unwrap()).unwrap();
        fs::write(dir.join("junk.jsonl"), b"not json\n").unwrap();
        fs::write(dir.join("ignored.txt"), b"x").unwrap();
        let list = store.list().unwrap();
        let problems: Vec<_> = list
            .iter()
            .map(|i| (i.id.as_str(), i.problem.is_some()))
            .collect();
        assert_eq!(problems, [("junk", true), ("newer", true)]);
        assert_eq!(
            store.read("newer").unwrap_err().code,
            crate::OpErrorCode::Unsupported
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_location_follows_the_platform() {
        let p = |vars: &[(&str, &str)], os: &str| {
            let vars: Vec<(String, String)> = vars
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect();
            RecoveryStore::default_location_from(
                |name| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.into()),
                os,
            )
        };
        assert_eq!(
            p(&[("DASHCRAFT_DATA_DIR", "/d"), ("HOME", "/h")], "linux"),
            Some(PathBuf::from("/d").join("recovery"))
        );
        assert_eq!(
            p(&[("LOCALAPPDATA", "C:\\L")], "windows"),
            Some(PathBuf::from("C:\\L").join("dashcraft").join("recovery"))
        );
        assert_eq!(
            p(&[("HOME", "/h")], "macos"),
            Some(PathBuf::from(
                "/h/Library/Application Support/dashcraft/recovery"
            ))
        );
        assert_eq!(
            p(&[("HOME", "/h")], "linux"),
            Some(PathBuf::from("/h/.local/share/dashcraft/recovery"))
        );
        assert_eq!(p(&[], "linux"), None);
    }
}
