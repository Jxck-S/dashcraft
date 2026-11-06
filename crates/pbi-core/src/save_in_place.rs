//! In-place save of a PBIP project (M6; ADR-0013, ADR-0014).
//!
//! [`save_in_place`] writes the edits of a document back into the PBIP
//! project folder it was opened from:
//!
//! - **Only what changed.** The project is serialized twice with
//!   [`pbip_files`](crate::pbip_files): as it was opened or last saved (the
//!   baseline) and as it is now. Only files whose bytes differ are written,
//!   and files the baseline produced that the current state no longer does
//!   (the `.tmdl` file of a renamed table) are deleted. Files DashCraft does
//!   not produce (`.pbi/` settings and caches, unknown files) are never
//!   touched.
//! - **Report references follow renames.** Net table, column and measure
//!   renames (matched by object id) are applied to the PBIR files
//!   (`report.json`, `page.json`, `visual.json` and other JSON files under
//!   `definition/`, such as bookmarks), and moved or resized visuals get
//!   their `position` patched. Report changes that cannot be written yet
//!   (a legacy `report.json` layout) are refused, never dropped.
//! - **External changes are conflicts.** A [`ProjectFingerprint`] of the
//!   project files is taken at open and updated by every save; if the files
//!   changed on disk since, the save is refused with the changed paths
//!   ([`CoreError::Conflict`]) unless `overwrite` is set. With `overwrite`,
//!   files DashCraft writes get the document's version; files it does not
//!   know are left alone.
//! - **Atomic files.** Each file is written to a temporary file in its
//!   folder and renamed into place. A failure part-way returns
//!   [`CoreError::PartialSave`] with the files already written; the next
//!   save writes the rest.
//!
//! Known gaps: `reportExtensions.json` (report-level measures) keeps the
//! old names, and DAX in it is not rewritten.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::hash::{DefaultHasher, Hasher};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use pbi_model::{Document, Model, ObjectId, Position, Rename, Report, SourceFormat, names_eq};
use pbi_pbir::{JsonStyle, PbirReport};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Number, Value};
use walkdir::WalkDir;

use crate::error::CoreError;
use crate::reader::{ModelSource, OpenedDocument, ReportSource};
use crate::save::{PbipOutput, pbip_output};

/// Most files a fingerprint tracks.
const MAX_FILES: usize = 100_000;
/// Most bytes a fingerprint reads.
const MAX_BYTES: u64 = 4 << 30;
/// Folders never tracked or written: Desktop's local settings and caches,
/// version control.
const IGNORED_DIRS: &[&str] = &[".pbi", ".git", "node_modules"];
/// Project structure files an in-place save never creates (they are
/// generated with random ids when missing).
const STRUCTURE_FILES: &[&str] = &[
    ".platform",
    "definition.pbism",
    "definition.pbir",
    "diagramLayout.json",
];

/// How a project file changed on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeKind {
    Added,
    Modified,
    Removed,
}

/// A project file that changed on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    /// Path relative to the project folder (`/`-separated).
    pub path: String,
    pub change: ChangeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    hash: u64,
}

/// Length and content hash of every file of a PBIP project (the `.pbip`
/// file, `.gitignore` and the model and report folders, without `.pbi/`),
/// to detect changes made by other programs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectFingerprint {
    /// Comparison key (lowercase on case-insensitive file systems) →
    /// (path as found, stamp).
    files: BTreeMap<String, (String, FileStamp)>,
}

impl ProjectFingerprint {
    /// Scans the project `doc` was opened from.
    ///
    /// # Errors
    /// [`CoreError::Save`] if `doc` is not a PBIP project opened from a
    /// folder, or has too many files; [`CoreError::Io`] on read failures.
    pub fn scan(doc: &OpenedDocument) -> Result<Self, CoreError> {
        Project::of(doc)?.scan()
    }

    /// Number of files tracked.
    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// `true` if no file is tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// The files that differ between `self` and `newer`, by path.
    #[must_use]
    pub fn changes(&self, newer: &Self) -> Vec<FileChange> {
        let mut out = Vec::new();
        for (key, (path, stamp)) in &self.files {
            match newer.files.get(key) {
                None => out.push(FileChange {
                    path: path.clone(),
                    change: ChangeKind::Removed,
                }),
                Some((_, now)) if now != stamp => out.push(FileChange {
                    path: path.clone(),
                    change: ChangeKind::Modified,
                }),
                Some(_) => {}
            }
        }
        for (key, (path, _)) in &newer.files {
            if !self.files.contains_key(key) {
                out.push(FileChange {
                    path: path.clone(),
                    change: ChangeKind::Added,
                });
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    fn record(&mut self, path: &str, bytes: &[u8]) {
        if let Ok(stamp) = stamp(bytes) {
            self.files.insert(key(path), (path.to_owned(), stamp));
        }
    }

    fn forget(&mut self, path: &str) {
        self.files.remove(&key(path));
    }
}

fn key(path: &str) -> String {
    if cfg!(any(windows, target_os = "macos")) {
        path.to_lowercase()
    } else {
        path.to_owned()
    }
}

fn stamp(mut reader: impl Read) -> io::Result<FileStamp> {
    let mut hasher = DefaultHasher::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut len = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let chunk = buf.get(..n).unwrap_or_default();
        hasher.write(chunk);
        len = len.saturating_add(n as u64);
    }
    hasher.write_u64(len);
    Ok(FileStamp {
        len,
        hash: hasher.finish(),
    })
}

/// The result of [`save_in_place`].
#[derive(Debug)]
pub struct SaveOutcome {
    /// Files written, relative to the project folder.
    pub written: Vec<String>,
    /// Files deleted (e.g. the `.tmdl` file of a renamed table).
    pub deleted: Vec<String>,
    /// External changes found before the save (and overwritten or kept,
    /// since the save had `overwrite`).
    pub overwritten: Vec<FileChange>,
    /// The saved state: the baseline of the next save.
    pub saved: OpenedDocument,
}

/// Saves `current` (an edited state of `baseline.document`) into the PBIP
/// project `baseline` was opened from. `fingerprint` is the project's
/// fingerprint at open or after the last save; it is updated to the state
/// after this save (also when the save fails part-way).
///
/// # Errors
/// - [`CoreError::Conflict`] when project files changed on disk since the
///   fingerprint and `overwrite` is not set (nothing is written);
/// - [`CoreError::Save`] when `baseline` is not a PBIP project opened from
///   a folder, or the edits contain report changes that cannot be written
///   in place yet (nothing is written);
/// - [`CoreError::PartialSave`] when a write or delete failed part-way;
/// - [`CoreError::Io`] when the project cannot be scanned.
pub fn save_in_place(
    baseline: &OpenedDocument,
    fingerprint: &mut ProjectFingerprint,
    current: &Document,
    overwrite: bool,
) -> Result<SaveOutcome, CoreError> {
    let project = Project::of(baseline)?;
    let now = project.scan()?;
    let changes = fingerprint.changes(&now);
    if !changes.is_empty() && !overwrite {
        return Err(CoreError::Conflict { changes });
    }

    let renames = match (&baseline.document.model, &current.model) {
        (Some(old), Some(new)) => net_renames(old, new),
        _ => Vec::new(),
    };
    let expected = expected_reports(&baseline.document, current, &renames);
    check_reports(&expected, &current.reports)?;
    let mut synced = baseline.clone();
    synced.report_sources = sync_reports(baseline, &expected, &renames)?;
    synced.document = current.clone();

    let before = pbip_output(baseline)?;
    let after = pbip_output(&synced)?;
    check_layout(&before)?;
    check_layout(&after)?;
    let plan = Plan::new(baseline, &before, &after, &changes, overwrite);

    let mut next = now;
    let mut written = Vec::new();
    let mut deleted = Vec::new();
    let result = (|| {
        for path in &plan.early_deletes {
            project.delete(path)?;
            next.forget(path);
            deleted.push(path.clone());
        }
        for (path, bytes) in &plan.writes {
            let abs = project.path(path)?;
            let same = fs::read(&abs).is_ok_and(|disk| disk == *bytes);
            if !same {
                write_atomic(&abs, bytes).map_err(|e| (path.clone(), e.to_string()))?;
                written.push(path.clone());
            }
            next.record(path, bytes);
        }
        for path in &plan.late_deletes {
            if project.delete(path)? {
                deleted.push(path.clone());
            }
            next.forget(path);
        }
        Ok::<(), (String, String)>(())
    })();
    *fingerprint = next;
    if let Err((failed, message)) = result {
        return Err(CoreError::PartialSave {
            written,
            deleted,
            failed,
            message,
        });
    }
    refresh_sources(&mut synced, &after);
    Ok(SaveOutcome {
        written,
        deleted,
        overwritten: changes,
        saved: synced,
    })
}

// ---------------------------------------------------------------------------
// The project folder
// ---------------------------------------------------------------------------

struct Project {
    root: PathBuf,
    /// Tracked files and folders, relative to `root`.
    scopes: Vec<String>,
}

impl Project {
    fn of(doc: &OpenedDocument) -> Result<Self, CoreError> {
        let refuse = |why: &str| {
            Err(CoreError::Save(format!(
                "{why}; save it as a new PBIP project instead"
            )))
        };
        if doc.document.source.format != SourceFormat::Pbip {
            return refuse("only a PBIP project can be saved in place");
        }
        let Some(root) = doc.container.root_dir() else {
            return refuse("the project was not opened from a folder");
        };
        let Some(pbip) = doc.detection.parts.pbip_file.clone() else {
            return refuse("the project has no .pbip file");
        };
        let mut scopes = vec![pbip, ".gitignore".to_owned()];
        let models = doc
            .model_source
            .iter()
            .chain(doc.extra_models.iter().map(|m| &m.source));
        for source in models {
            if !same_container(source.container(), &doc.container) {
                return refuse("a semantic model is outside the project folder");
            }
            scopes.push(source.folder().to_owned());
        }
        scopes.extend(doc.report_sources.iter().map(|s| s.folder().to_owned()));
        if scopes.iter().any(String::is_empty) {
            return refuse("an item of the project is not in its own folder");
        }
        Ok(Self {
            root: root.to_path_buf(),
            scopes,
        })
    }

    fn scan(&self) -> Result<ProjectFingerprint, CoreError> {
        let mut out = ProjectFingerprint::default();
        let mut total = 0u64;
        let mut add = |abs: &Path, out: &mut ProjectFingerprint| -> Result<(), CoreError> {
            let Some(rel) = relative(&self.root, abs) else {
                return Ok(());
            };
            if out.files.len() >= MAX_FILES {
                return Err(CoreError::Save(format!(
                    "the project has more than {MAX_FILES} files to track"
                )));
            }
            let file = File::open(abs).map_err(|e| CoreError::io(abs, e))?;
            let stamp = stamp(file).map_err(|e| CoreError::io(abs, e))?;
            total = total.saturating_add(stamp.len);
            if total > MAX_BYTES {
                return Err(CoreError::Save(format!(
                    "the project has more than {MAX_BYTES} bytes to track"
                )));
            }
            out.files.insert(key(&rel), (rel, stamp));
            Ok(())
        };
        for scope in &self.scopes {
            let start = self.root.join(scope);
            let meta = match fs::symlink_metadata(&start) {
                Ok(m) => m,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(CoreError::io(&start, e)),
            };
            if meta.is_file() {
                add(&start, &mut out)?;
                continue;
            }
            if !meta.is_dir() {
                continue;
            }
            let walk = WalkDir::new(&start)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| {
                    !(e.depth() > 0
                        && e.file_type().is_dir()
                        && e.file_name()
                            .to_str()
                            .is_some_and(|n| IGNORED_DIRS.contains(&n)))
                });
            for entry in walk {
                let entry = entry.map_err(|e| {
                    CoreError::Save(format!("cannot scan {}: {e}", start.display()))
                })?;
                if entry.file_type().is_file() {
                    add(entry.path(), &mut out)?;
                }
            }
        }
        Ok(out)
    }

    /// The absolute path of `rel`, refusing unsafe paths and symbolic
    /// links anywhere under the project folder.
    fn path(&self, rel: &str) -> Result<PathBuf, (String, String)> {
        let refuse = |why: &str| (rel.to_owned(), why.to_owned());
        let rel_path = Path::new(rel);
        if rel.is_empty()
            || !rel_path
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
        {
            return Err(refuse("refusing to write an unsafe path"));
        }
        let mut abs = self.root.clone();
        for c in rel_path.components() {
            abs.push(c);
            if fs::symlink_metadata(&abs).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(refuse("refusing to write through a symbolic link"));
            }
        }
        Ok(abs)
    }

    /// Deletes `rel` if it exists. Returns whether it did.
    fn delete(&self, rel: &str) -> Result<bool, (String, String)> {
        let abs = self.path(rel)?;
        match fs::remove_file(&abs) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err((rel.to_owned(), e.to_string())),
        }
    }
}

fn same_container(
    a: &Arc<dyn crate::container::Container>,
    b: &Arc<dyn crate::container::Container>,
) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
}

/// `abs` relative to `root`, `/`-separated (`None` outside `root` or for
/// non-UTF-8 names).
fn relative(root: &Path, abs: &Path) -> Option<String> {
    let rel = abs.strip_prefix(root).ok()?;
    let parts: Option<Vec<&str>> = rel
        .components()
        .map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    let parts = parts?;
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Writes `bytes` to a temporary file next to `path`, then renames it
/// into place.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("the path has no folder"))?;
    fs::create_dir_all(dir)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let mut attempt = 0u32;
    let (temp, mut file) = loop {
        let temp = dir.join(format!(
            ".{name}.dashcraft-{}-{attempt}.tmp",
            std::process::id()
        ));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
        {
            Ok(file) => break (temp, file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && attempt < 100 => attempt += 1,
            Err(e) => return Err(e),
        }
    };
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(e) = written.and_then(|()| fs::rename(&temp, path)) {
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// What to write
// ---------------------------------------------------------------------------

struct Plan {
    /// Old paths of files whose new path differs only in case: deleted
    /// before the writes so the new name sticks on any file system.
    early_deletes: Vec<String>,
    writes: Vec<(String, Vec<u8>)>,
    late_deletes: Vec<String>,
}

impl Plan {
    fn new(
        baseline: &OpenedDocument,
        before: &PbipOutput,
        after: &PbipOutput,
        changes: &[FileChange],
        overwrite: bool,
    ) -> Self {
        let before_bytes: HashMap<&str, &[u8]> = before
            .files
            .iter()
            .map(|(p, b)| (p.as_str(), b.as_slice()))
            .collect();
        let after_exact: HashSet<&str> = after.files.iter().map(|(p, _)| p.as_str()).collect();
        let after_folded: HashSet<String> =
            after.files.iter().map(|(p, _)| p.to_lowercase()).collect();
        let changed_on_disk: HashSet<String> = changes.iter().map(|c| key(&c.path)).collect();

        let mut early_deletes = Vec::new();
        let mut late_deletes = Vec::new();
        for (path, _) in &before.files {
            if after_exact.contains(path.as_str()) {
                continue;
            }
            if after_folded.contains(&path.to_lowercase()) {
                early_deletes.push(path.clone());
            } else {
                late_deletes.push(path.clone());
            }
        }
        let mut writes = Vec::new();
        for (path, bytes) in &after.files {
            let edited = before_bytes.get(path.as_str()) != Some(&bytes.as_slice());
            let restore = overwrite && changed_on_disk.contains(&key(path));
            if !(edited || restore) {
                continue;
            }
            let name = path.rsplit('/').next().unwrap_or(path);
            let structure = STRUCTURE_FILES.iter().any(|f| f.eq_ignore_ascii_case(name))
                || name.to_ascii_lowercase().ends_with(".pbip");
            if structure && !baseline.container.is_file(path) {
                continue;
            }
            writes.push((path.clone(), bytes.clone()));
        }
        Self {
            early_deletes,
            writes,
            late_deletes,
        }
    }
}

/// Refuses an output whose item folders differ from the source folders
/// (it would be written somewhere else).
fn check_layout(out: &PbipOutput) -> Result<(), CoreError> {
    for (source, output) in &out.folders {
        if source != output {
            return Err(CoreError::Save(format!(
                "the item folder `{source}` would be written as `{output}`; save it as a new PBIP project instead"
            )));
        }
    }
    Ok(())
}

/// After a save: the model CSTs and PBIR file bytes of `synced` become
/// the written files, so the next save preserves them.
fn refresh_sources(synced: &mut OpenedDocument, after: &PbipOutput) {
    if let Some(ModelSource::Tmdl {
        definition_dir,
        files,
        ..
    }) = &mut synced.model_source
    {
        let prefix = format!("{definition_dir}/");
        let parsed: Option<Vec<pbi_tmdl::TmdlFile>> = after
            .files
            .iter()
            .filter(|(p, _)| p.starts_with(&prefix) && p.to_ascii_lowercase().ends_with(".tmdl"))
            .map(|(p, b)| {
                std::str::from_utf8(b)
                    .ok()
                    .and_then(|text| pbi_tmdl::parse_file(p, text).ok())
            })
            .collect();
        match parsed {
            Some(parsed) => *files = parsed,
            None => tracing::warn!("a saved TMDL file did not parse back; keeping the old CSTs"),
        }
    }
    for source in &mut synced.report_sources {
        let ReportSource::Pbir { folder, report } = source else {
            continue;
        };
        let prefix = format!("{folder}/");
        for (path, bytes) in &after.files {
            if let Some(rel) = path.strip_prefix(&prefix)
                && let Some(file) = report.files.get_mut(rel)
            {
                bytes.clone_into(&mut file.bytes);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Report sync
// ---------------------------------------------------------------------------

/// The renames from `old` to `new` (tables, columns and measures matched
/// by id), as a sequence that turns `old` references into `new` ones when
/// applied in order: every name goes through a unique placeholder first,
/// so swaps and chains work.
fn net_renames(old: &Model, new: &Model) -> Vec<Rename> {
    let old_tables: HashMap<ObjectId, &pbi_model::Table> = old
        .tables
        .iter()
        .filter(|t| t.id.is_assigned())
        .map(|t| (t.id, t))
        .collect();
    let mut tables: Vec<(String, String)> = Vec::new();
    // (old table name, old name, new name)
    let mut columns: Vec<(String, String, String)> = Vec::new();
    let mut measures: Vec<(String, String, String)> = Vec::new();
    for table in &new.tables {
        let Some(before) = old_tables.get(&table.id) else {
            continue;
        };
        if before.name != table.name {
            tables.push((before.name.clone(), table.name.clone()));
        }
        for c in table.columns.iter().filter(|c| c.id.is_assigned()) {
            if let Some(o) = before.columns.iter().find(|o| o.id == c.id)
                && o.name != c.name
            {
                columns.push((before.name.clone(), o.name.clone(), c.name.clone()));
            }
        }
        for m in table.measures.iter().filter(|m| m.id.is_assigned()) {
            if let Some(o) = before.measures.iter().find(|o| o.id == m.id)
                && o.name != m.name
            {
                measures.push((before.name.clone(), o.name.clone(), m.name.clone()));
            }
        }
    }
    let mut n = 0usize;
    let mut placeholder = || {
        n += 1;
        format!("\u{1}dashcraft-rename-{n}\u{1}")
    };
    let column_slots: Vec<String> = columns.iter().map(|_| placeholder()).collect();
    let measure_slots: Vec<String> = measures.iter().map(|_| placeholder()).collect();
    let table_slots: Vec<String> = tables.iter().map(|_| placeholder()).collect();
    let mut out = Vec::new();
    for ((table, old, _), slot) in columns.iter().zip(&column_slots) {
        out.push(Rename::Column {
            table: table.clone(),
            old: old.clone(),
            new: slot.clone(),
        });
    }
    for ((table, old, _), slot) in measures.iter().zip(&measure_slots) {
        out.push(Rename::Measure {
            table: table.clone(),
            old: old.clone(),
            new: slot.clone(),
        });
    }
    for ((old, _), slot) in tables.iter().zip(&table_slots) {
        out.push(Rename::Table {
            old: old.clone(),
            new: slot.clone(),
        });
    }
    let table_now = |name: &str| {
        tables
            .iter()
            .zip(&table_slots)
            .find(|((old, _), _)| old == name)
            .map_or_else(|| name.to_owned(), |(_, slot)| slot.clone())
    };
    for ((table, _, new), slot) in columns.iter().zip(&column_slots) {
        out.push(Rename::Column {
            table: table_now(table),
            old: slot.clone(),
            new: new.clone(),
        });
    }
    for ((table, _, new), slot) in measures.iter().zip(&measure_slots) {
        out.push(Rename::Measure {
            table: table_now(table),
            old: slot.clone(),
            new: new.clone(),
        });
    }
    for ((_, new), slot) in tables.iter().zip(&table_slots) {
        out.push(Rename::Table {
            old: slot.clone(),
            new: new.clone(),
        });
    }
    out
}

/// The baseline reports with the renames applied and the current visual
/// positions: what the current reports must be for the save to write all
/// of their changes.
fn expected_reports(baseline: &Document, current: &Document, renames: &[Rename]) -> Vec<Report> {
    let positions: HashMap<ObjectId, &Position> = current
        .reports
        .iter()
        .flat_map(|r| &r.pages)
        .flat_map(|p| &p.visuals)
        .filter(|v| v.id.is_assigned())
        .map(|v| (v.id, &v.position))
        .collect();
    let mut reports = baseline.reports.clone();
    for report in &mut reports {
        for rename in renames {
            rename.apply_to_filters(&mut report.filters);
            for page in &mut report.pages {
                rename.apply_to_filters(&mut page.filters);
                for visual in &mut page.visuals {
                    rename.apply_to_visual(visual);
                }
            }
        }
        for visual in report.pages.iter_mut().flat_map(|p| &mut p.visuals) {
            if let Some(position) = positions.get(&visual.id) {
                (*position).clone_into(&mut visual.position);
            }
        }
    }
    reports
}

/// Refuses report changes the save cannot write.
fn check_reports(expected: &[Report], current: &[Report]) -> Result<(), CoreError> {
    if expected == current {
        return Ok(());
    }
    let refuse = |what: String| {
        Err(CoreError::Save(format!(
            "{what} has changes that cannot be saved in place yet"
        )))
    };
    if expected.len() != current.len() {
        return refuse("the list of reports".into());
    }
    for (e, c) in expected.iter().zip(current) {
        if e == c {
            continue;
        }
        if e.pages.len() == c.pages.len() {
            for (ep, cp) in e.pages.iter().zip(&c.pages) {
                if ep == cp {
                    continue;
                }
                if ep.visuals.len() == cp.visuals.len()
                    && let Some((_, cv)) = ep.visuals.iter().zip(&cp.visuals).find(|(a, b)| a != b)
                {
                    return refuse(format!(
                        "visual `{}` on page `{}` of report `{}`",
                        cv.name, cp.display_name, c.name
                    ));
                }
                return refuse(format!("page `{}` of report `{}`", cp.display_name, c.name));
            }
        }
        return refuse(format!("report `{}`", c.name));
    }
    Ok(())
}

/// The baseline report sources with `renames` applied and the visual
/// positions of `expected`.
fn sync_reports(
    baseline: &OpenedDocument,
    expected: &[Report],
    renames: &[Rename],
) -> Result<Vec<ReportSource>, CoreError> {
    let mut out = baseline.report_sources.clone();
    for (i, source) in out.iter_mut().enumerate() {
        let (Some(before), Some(after)) = (baseline.document.reports.get(i), expected.get(i))
        else {
            continue;
        };
        match source {
            ReportSource::Pbir { report, .. } => {
                if !renames.is_empty() {
                    rename_pbir(report, renames)?;
                }
                for (page, visual, old, new) in moved_visuals(before, after) {
                    patch_position(report, page, visual, old, new)?;
                }
            }
            ReportSource::Legacy { .. } => {
                if before != after {
                    return Err(CoreError::Save(format!(
                        "report `{}` uses the legacy layout (report.json), which cannot be updated in place yet; save it as a new PBIP project instead",
                        after.name
                    )));
                }
            }
        }
    }
    Ok(out)
}

/// `(page name, visual name, old position, new position)` of every visual
/// whose position differs between two states of a report.
fn moved_visuals<'a>(
    before: &'a Report,
    after: &'a Report,
) -> Vec<(&'a str, &'a str, &'a Position, &'a Position)> {
    let old: HashMap<ObjectId, &Position> = before
        .pages
        .iter()
        .flat_map(|p| &p.visuals)
        .map(|v| (v.id, &v.position))
        .collect();
    let mut out = Vec::new();
    for page in &after.pages {
        for visual in &page.visuals {
            if let Some(old) = old.get(&visual.id)
                && **old != visual.position
            {
                out.push((
                    page.name.as_str(),
                    visual.name.as_str(),
                    *old,
                    &visual.position,
                ));
            }
        }
    }
    out
}

fn rename_pbir(report: &mut PbirReport, renames: &[Rename]) -> Result<(), CoreError> {
    if let Some(json) = &report.report
        && let Some(renamed) = renamed(json, renames, "report.json")?
    {
        report.report = Some(renamed);
    }
    for page in &mut report.pages {
        let path = page.path();
        if let Some(renamed) = renamed(&page.page, renames, &path)? {
            page.page = renamed;
        }
        for i in 0..page.visuals.len() {
            let Some(visual) = page.visuals.get(i) else {
                continue;
            };
            let path = page.visual_path(visual);
            if let Some(renamed) = renamed(&visual.visual, renames, &path)?
                && let Some(slot) = page.visuals.get_mut(i)
            {
                slot.visual = renamed;
            }
        }
    }
    // Other JSON of the definition (bookmarks, …), kept verbatim otherwise.
    for (path, file) in &mut report.files {
        if file.parsed
            || !path.starts_with("definition/")
            || !path.to_ascii_lowercase().ends_with(".json")
        {
            continue;
        }
        let body = file
            .bytes
            .strip_prefix(b"\xEF\xBB\xBF")
            .unwrap_or(&file.bytes);
        let Ok(mut value) = serde_json::from_slice::<Value>(body) else {
            continue;
        };
        if rename_json(&mut value, renames)
            && let Ok(bytes) = JsonStyle::detect(&file.bytes).to_vec(&value)
        {
            file.bytes = bytes;
        }
    }
    Ok(())
}

/// `value` with `renames` applied, or `None` if nothing changed.
fn renamed<T: Serialize + DeserializeOwned>(
    value: &T,
    renames: &[Rename],
    path: &str,
) -> Result<Option<T>, CoreError> {
    let fail = |e: serde_json::Error| CoreError::Save(format!("{path}: {e}"));
    let mut json = serde_json::to_value(value).map_err(fail)?;
    if !rename_json(&mut json, renames) {
        return Ok(None);
    }
    serde_json::from_value(json).map(Some).map_err(fail)
}

/// Applies `renames` to PBIR JSON. Each filter of `filterConfig.filters`
/// is rewritten on its own (its `From` aliases are its own).
fn rename_json(value: &mut Value, renames: &[Rename]) -> bool {
    let mut filters = value
        .get_mut("filterConfig")
        .and_then(|c| c.get_mut("filters"))
        .map(Value::take);
    let mut changed = false;
    for rename in renames {
        changed |= rename_native_query_refs(value, rename);
        changed |= rename.apply_to_json(value);
        if let Some(Value::Array(items)) = &mut filters {
            for filter in items {
                changed |= rename.apply_to_json(filter);
            }
        }
    }
    if let Some(filters) = filters
        && let Some(slot) = value
            .get_mut("filterConfig")
            .and_then(|c| c.get_mut("filters"))
    {
        *slot = filters;
    }
    changed
}

/// Renames the `nativeQueryRef` (the bare field name) of projections of a
/// renamed column or measure. Runs before the field itself is renamed.
fn rename_native_query_refs(value: &mut Value, rename: &Rename) -> bool {
    let (table, old, new, kind) = match rename {
        Rename::Column { table, old, new } => (table, old, new, "Column"),
        Rename::Measure { table, old, new } => (table, old, new, "Measure"),
        Rename::Table { .. } => return false,
    };
    walk_native(value, table, old, new, kind)
}

fn walk_native(value: &mut Value, table: &str, old: &str, new: &str, kind: &str) -> bool {
    match value {
        Value::Object(map) => {
            let mut changed = false;
            let names_field = |f: &Value| {
                let entity = f
                    .get("Expression")
                    .and_then(|e| e.get("SourceRef"))
                    .and_then(|s| s.get("Entity"))
                    .and_then(Value::as_str);
                let property = f.get("Property").and_then(Value::as_str);
                entity.is_some_and(|e| names_eq(e, table))
                    && property.is_some_and(|p| names_eq(p, old))
            };
            let field = map.get("field");
            let plain = field.and_then(|f| f.get(kind)).is_some_and(names_field);
            let aggregated = kind == "Column"
                && field
                    .and_then(|f| f.get("Aggregation"))
                    .and_then(|a| a.get("Expression"))
                    .and_then(|e| e.get("Column"))
                    .is_some_and(names_field);
            if (plain || aggregated)
                && let Some(Value::String(native)) = map.get_mut("nativeQueryRef")
                && let Some(renamed) =
                    pbi_model::rewrite::renamed_native_query_ref(native, old, new, aggregated)
            {
                *native = renamed;
                changed = true;
            }
            for child in map.values_mut() {
                changed |= walk_native(child, table, old, new, kind);
            }
            changed
        }
        Value::Array(items) => items
            .iter_mut()
            .fold(false, |acc, v| walk_native(v, table, old, new, kind) | acc),
        _ => false,
    }
}

/// Patches the `position` of one visual: only the properties that changed.
fn patch_position(
    report: &mut PbirReport,
    page: &str,
    visual: &str,
    old: &Position,
    new: &Position,
) -> Result<(), CoreError> {
    let missing = || {
        CoreError::Save(format!(
            "visual `{visual}` on page `{page}` is not in the report files"
        ))
    };
    let pbir_page = report
        .pages
        .iter_mut()
        .find(|p| p.page.name.as_deref().unwrap_or(&p.dir) == page)
        .ok_or_else(missing)?;
    let pbir_visual = pbir_page
        .visuals
        .iter_mut()
        .find(|v| v.visual.name.as_deref().unwrap_or(&v.dir) == visual)
        .ok_or_else(missing)?;
    let pos = pbir_visual
        .visual
        .position
        .get_or_insert_with(Default::default);
    for (slot, a, b) in [
        (&mut pos.x, old.x, new.x),
        (&mut pos.y, old.y, new.y),
        (&mut pos.z, old.z, new.z),
        (&mut pos.width, old.width, new.width),
        (&mut pos.height, old.height, new.height),
    ] {
        if a != b {
            *slot = Some(number(b)?);
        }
    }
    if old.tab_order != new.tab_order {
        pos.tab_order = new.tab_order.map(Number::from);
    }
    if old.angle != new.angle {
        pos.angle = new.angle.map(number).transpose()?;
    }
    Ok(())
}

/// A JSON number with the shortest text (`100`, not `100.0`).
fn number(v: f64) -> Result<Number, CoreError> {
    if !v.is_finite() {
        return Err(CoreError::Save(format!(
            "the visual position {v} is not a finite number"
        )));
    }
    serde_json::from_str::<Number>(&v.to_string())
        .map_err(|e| CoreError::Save(format!("the visual position {v}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pbi_model::{IdGen, Measure, Table};
    use serde_json::json;

    fn measure_field(table: &str, name: &str) -> Value {
        json!({
            "field": {"Measure": {"Expression": {"SourceRef": {"Entity": table}}, "Property": name}},
            "queryRef": format!("{table}.{name}"),
            "nativeQueryRef": name
        })
    }

    #[test]
    fn swapped_names_and_a_renamed_table_rewrite_report_json() {
        let mut ids = IdGen::new();
        let measure = |ids: &mut IdGen, name: &str| Measure {
            id: ids.next_id(),
            name: name.into(),
            ..Measure::default()
        };
        let old = Model {
            tables: vec![Table {
                id: ids.next_id(),
                name: "Sales".into(),
                measures: vec![measure(&mut ids, "A"), measure(&mut ids, "B")],
                ..Table::default()
            }],
            ..Model::default()
        };
        let mut new = old.clone();
        let table = new.tables.first_mut().unwrap();
        table.name = "Revenue".into();
        table.measures.first_mut().unwrap().name = "B".into();
        table.measures.last_mut().unwrap().name = "A".into();

        let renames = net_renames(&old, &new);
        let mut value = json!({
            "projections": [measure_field("Sales", "A"), measure_field("Sales", "B")],
            "filterConfig": {"filters": [{
                "field": {"Measure": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "A"}},
                "filter": {"From": [{"Name": "s", "Entity": "Sales"}]}
            }]}
        });
        assert!(rename_json(&mut value, &renames));
        assert_eq!(
            value,
            json!({
                "projections": [measure_field("Revenue", "B"), measure_field("Revenue", "A")],
                "filterConfig": {"filters": [{
                    "field": {"Measure": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "B"}},
                    "filter": {"From": [{"Name": "s", "Entity": "Revenue"}]}
                }]}
            })
        );
        assert!(net_renames(&old, &old).is_empty());
    }

    #[test]
    fn positions_are_shortest_json_numbers() {
        assert_eq!(number(100.0).unwrap().to_string(), "100");
        assert_eq!(number(12.25).unwrap().to_string(), "12.25");
        assert!(number(f64::NAN).is_err());
    }
}
