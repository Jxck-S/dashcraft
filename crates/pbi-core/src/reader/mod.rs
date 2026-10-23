//! Format readers: any supported input (path or bytes) → a unified
//! [`pbi_model::Document`] plus what is needed to load data and save later.
//!
//! [`open_path`] and [`open_bytes`] detect the format by content
//! ([`crate::detect`]), open it through a safe [`Container`], and dispatch
//! to a reader:
//!
//! | Input | Reader | Model | Reports |
//! |---|---|---|---|
//! | PBIP project (`.pbip` file or folder, Fabric Git layout) | `project` | every `*.SemanticModel` (TMDL `definition/` or `model.bim`) | every `*.Report` (PBIR `definition/` or PBIR-Legacy `report.json`) |
//! | `*.SemanticModel` folder, loose TMDL folder | `project` | TMDL / `model.bim` | none |
//! | `*.Report` folder | `project` | sibling `byPath` model if it exists on disk | PBIR / PBIR-Legacy |
//! | PBIX (ZIP or unzipped folder) | `package` | `DataModel` schema (VertiPaq metadata) | `Report/definition/` or `Report/Layout` |
//! | PBIT | `package` | `DataModelSchema` (TMSL) | as PBIX |
//! | `.bim`, `.tmdl`, `.abf`, `.pbids`, legacy layout JSON | `single` | TMSL / TMDL / VertiPaq metadata | legacy layout |
//! | `.vpax` | `vpax` | `Model.bim`, else `DaxModel.json` (+ [`VpaxStats`]) | none |
//! | Excel workbook with Power Pivot (`xl/model/item.data`) | `workbook` | VertiPaq metadata (legacy XML) + storage map | none |
//! | `.rdl` | `rdl` | none ([`PaginatedReport`] outline) | none |
//! | `.pq`/`.m`, `.pqt`, Dataflow Gen2 folder, CDM `model.json` | `mashup` | M queries as named expressions; CDM entities as tables | none |
//! | `.pbiviz`, `.mez`/`.pqx` | `extension` | none (metadata outline; never executed) | none |
//!
//! Format-specific structure that is neither model nor report goes to
//! [`OpenedDocument::outline`] ([`FormatOutline`]). Formats that are detected
//! but have no reader open as a document with only the entry listing and an
//! Info diagnostic ([`codes::NOT_IMPLEMENTED`]). Unrecognised input is
//! [`CoreError::Unsupported`].
//!
//! Every reader sets [`SourceInfo`] (format, path, entries), assigns object
//! IDs, sets each report's `dataset_reference` and the document's
//! [`ModelStatus`], and turns every recoverable problem into a
//! [`Diagnostic`] (codes in [`codes`], plus the `TMDL*`, `TMSL*` and
//! `PBIR*` codes of the format crates). Origins are relative to the opened
//! container (e.g. `Acme Outfitters - Overview.Report/definition/report.json`).
//!
//! Metadata parts (`.pbip`, `definition.pbism`, `.platform`,
//! `.pbi/localSettings.json`, `diagramLayout.json`, PBIX `Version`,
//! `Settings`, `Metadata`, `DiagramLayout`, `Connections`, `.pbids`) are
//! parsed leniently into [`OpenedDocument::parts`] after
//! [redaction](crate::redact); `SecurityBindings` is never decoded.

mod extension;
mod files;
mod mashup;
mod model;
pub mod outline;
mod package;
mod project;
mod rdl;
mod report;
mod single;
mod vpax;
mod workbook;

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pbi_model::{
    Diagnostic, Document, Model, ModelStatus, Origin, PartInfo, SourceFormat, SourceInfo,
};
use pbi_pbir::{LegacyReport, PbirReport};
use serde::Serialize;
use serde_json::Value;

use crate::container::{
    Container, DirContainer, Limits, SkippedEntry, TextEncoding, ZipContainer, decode_text,
};
use crate::detect::{DetectedFormat, Detection, detect_bytes, detect_container, detect_path};
use crate::error::CoreError;
use crate::redact::redact_json;

pub(crate) use files::{BytesContainer, file_name, file_stem, item_name, join};
pub(crate) use model::{strip_tmsl_shadows, without_storage_facts as model_for_writing};
pub use outline::{Connector, CustomVisual, FormatOutline, Mashup, PaginatedReport, VpaxStats};

/// `ModelStatus::Remote` description for `byConnection` / live reports.
pub const REMOTE_DESCRIPTION: &str = "remote, not available offline";

/// Diagnostic codes emitted by the readers of this crate.
pub mod codes {
    /// A part could not be read (I/O error, size limit, invalid encoding).
    pub const READ_FAILED: &str = "CORE001";
    /// The format was detected, but it has no reader yet.
    pub const NOT_IMPLEMENTED: &str = "CORE002";
    /// The semantic model could not be loaded (TMDL syntax, invalid TMSL,
    /// unreadable `DataModel`).
    pub const MODEL_FAILED: &str = "CORE003";
    /// A report's `byPath` model was not found.
    pub const MODEL_NOT_FOUND: &str = "CORE004";
    /// The model is remote (`byConnection`, live connection): not
    /// available offline.
    pub const REMOTE_MODEL: &str = "CORE005";
    /// A report could not be loaded.
    pub const REPORT_FAILED: &str = "CORE006";
    /// A metadata part (`.platform`, `Settings`, …) is invalid or too large.
    pub const METADATA: &str = "CORE007";
    /// Secrets were redacted from a metadata part.
    pub const REDACTED: &str = "CORE008";
    /// An opaque part (`SecurityBindings`, `DataMashup`) was listed but not
    /// decoded.
    pub const OPAQUE: &str = "CORE009";
    /// Notes about the PBIP `.pbi/cache.abf` cross-check.
    pub const CACHE_ABF: &str = "CORE010";
    /// The input holds several semantic models; one is the document model,
    /// the others are in [`OpenedDocument::extra_models`](super::OpenedDocument::extra_models).
    pub const MULTIPLE_MODELS: &str = "CORE011";
    /// No semantic model was found (or its schema was not loaded).
    pub const NO_MODEL: &str = "CORE012";
    /// A `.pbids` connection (protocol and address).
    pub const PBIDS: &str = "CORE013";
    /// Entries skipped while listing a folder (symlinks, VCS folders).
    pub const SKIPPED: &str = "CORE014";
    /// No report found.
    pub const NO_REPORT: &str = "CORE015";
    /// A report's `byPath` model was loaded from a folder next to the
    /// opened one.
    pub const EXTERNAL_MODEL: &str = "CORE016";
    /// VertiPaq Analyzer export notes (statistics source, unreadable parts).
    pub const VPAX: &str = "CORE017";
    /// Excel Power Pivot workbook notes.
    pub const POWER_PIVOT: &str = "CORE018";
    /// Paginated report (RDL) outline notes; malformed or refused XML.
    pub const PAGINATED: &str = "CORE019";
    /// An M script, template or dataflow query has a syntax error.
    pub const M_SYNTAX: &str = "CORE020";
    /// Power Query template / dataflow metadata notes.
    pub const MASHUP: &str = "CORE021";
    /// Custom visual or connector package notes (never executed).
    pub const EXTENSION: &str = "CORE022";
}

/// Options for [`open_path`] and [`open_bytes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenOptions {
    /// Caps applied to folders, archives and single files.
    pub limits: Limits,
    /// Keep internal VertiPaq tables (`H$`, `R$`, `U$`) when reading a
    /// model from a `DataModel` / ABF. Default `false`.
    pub include_system_tables: bool,
    /// Decompress the PBIX `DataModel` (and a PBIP `.pbi/cache.abf`) to read
    /// the model schema. Default `true`; when `false` the PBIX model is
    /// reported as not loaded.
    pub load_pbix_schema: bool,
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            include_system_tables: false,
            load_pbix_schema: true,
        }
    }
}

/// Where the document's semantic model came from, with what is needed to
/// write it back losslessly or to load its data later.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ModelSource {
    /// TMDL files.
    Tmdl {
        /// Semantic model folder in `container` (`""` = its root).
        folder: String,
        /// Container holding the model (normally the opened container).
        container: Arc<dyn Container>,
        /// Folder of the `.tmdl` files (`<folder>/definition`, or the folder
        /// itself for a loose TMDL folder).
        definition_dir: String,
        /// Lossless CSTs for [`pbi_tmdl::write_model_preserving`].
        files: Vec<pbi_tmdl::TmdlFile>,
    },
    /// TMSL JSON (`model.bim`, PBIT `DataModelSchema`).
    Tmsl {
        /// Semantic model folder in `container` (`""` = its root).
        folder: String,
        /// Container holding the model.
        container: Arc<dyn Container>,
        /// The TMSL entry, e.g. `X.SemanticModel/model.bim`, `DataModelSchema`.
        entry: String,
        /// Options that reproduce the source layout and encoding.
        write_options: pbi_tmsl::WriteOptions,
    },
    /// An Analysis Services backup (PBIX `DataModel`, `.abf`): schema from
    /// `metadata.sqlitedb`; the archive is kept for VertiPaq data loading.
    Abf {
        /// Folder of the entry in `container`.
        folder: String,
        /// Container holding the backup.
        container: Arc<dyn Container>,
        /// The backup entry, e.g. `DataModel`.
        entry: String,
        /// The entry was XPress9-compressed.
        compressed: bool,
        /// The parsed (decompressed) backup.
        archive: pbi_vertipaq::AbfArchive,
        /// Column → storage files map for the VertiPaq decoder.
        storage: pbi_vertipaq::StorageMap,
        /// `metadata.sqlitedb` schema version.
        schema_version: Option<i64>,
    },
}

impl ModelSource {
    /// Semantic model folder (or the folder of the model entry).
    #[must_use]
    pub fn folder(&self) -> &str {
        match self {
            Self::Tmdl { folder, .. } | Self::Tmsl { folder, .. } | Self::Abf { folder, .. } => {
                folder
            }
        }
    }

    /// Container holding the model files.
    #[must_use]
    pub fn container(&self) -> &Arc<dyn Container> {
        match self {
            Self::Tmdl { container, .. }
            | Self::Tmsl { container, .. }
            | Self::Abf { container, .. } => container,
        }
    }

    /// `tmdl`, `tmsl` or `abf`.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Tmdl { .. } => "tmdl",
            Self::Tmsl { .. } => "tmsl",
            Self::Abf { .. } => "abf",
        }
    }
}

/// The parsed source of one report, aligned with `Document::reports`, for
/// lossless re-saving.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ReportSource {
    /// A PBIR folder (`*.Report/` or PBIX `Report/`); every file verbatim.
    Pbir {
        /// Report folder in the container (`Report` in a PBIX, `""` for an
        /// opened report folder).
        folder: String,
        /// The typed files.
        report: Box<PbirReport>,
    },
    /// A legacy layout (PBIX `Report/Layout`, PBIR-Legacy `report.json`).
    Legacy {
        /// Report folder in the container.
        folder: String,
        /// The layout entry, e.g. `Report/Layout`.
        entry: String,
        /// The parsed layout.
        report: Box<LegacyReport>,
    },
}

impl ReportSource {
    /// Report folder in the container.
    #[must_use]
    pub fn folder(&self) -> &str {
        match self {
            Self::Pbir { folder, .. } | Self::Legacy { folder, .. } => folder,
        }
    }
}

/// An additional semantic model of a multi-model PBIP project (the
/// document holds one model; reports bound to these keep their `byPath`).
#[derive(Debug, Clone)]
pub struct ExtraModel {
    /// The model (IDs assigned, unique across the document).
    pub model: Model,
    /// Where it came from.
    pub source: ModelSource,
}

/// A metadata part parsed leniently (secrets redacted).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataPart {
    /// Path in the opened container.
    pub path: String,
    /// `pbip`, `pbism`, `platform`, `localSettings`, `diagramLayout`,
    /// `version`, `settings`, `metadata`, `connections`, `pbids`, …
    pub kind: String,
    /// Detected text encoding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<TextEncoding>,
    /// Plain-text value (e.g. PBIX `Version`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Parsed JSON, after redaction.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json: Option<Value>,
    /// JSON pointers of redacted values.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub redacted: Vec<String>,
}

/// An opened input: the document plus everything needed to load data and
/// save it again.
#[derive(Debug, Clone)]
pub struct OpenedDocument {
    /// Model, reports, diagnostics, source info.
    pub document: Document,
    /// How the input was recognised.
    pub detection: Detection,
    /// The opened folder, archive or file (kept for data loading/saving).
    pub container: Arc<dyn Container>,
    /// Source of `document.model`.
    pub model_source: Option<ModelSource>,
    /// Other semantic models of a multi-model project.
    pub extra_models: Vec<ExtraModel>,
    /// Sources of `document.reports`, same order.
    pub report_sources: Vec<ReportSource>,
    /// Parsed metadata parts (redacted).
    pub parts: Vec<MetadataPart>,
    /// Format-specific structure (VPAX statistics, RDL outline, custom
    /// visual / connector metadata, mashup details).
    pub outline: Option<FormatOutline>,
}

impl OpenedDocument {
    /// VertiPaq Analyzer statistics of a `.vpax`.
    #[must_use]
    pub fn vpax_stats(&self) -> Option<&VpaxStats> {
        match &self.outline {
            Some(FormatOutline::Vpax(s)) => Some(s),
            _ => None,
        }
    }

    /// Outline of a paginated report (`.rdl`).
    #[must_use]
    pub fn paginated_report(&self) -> Option<&PaginatedReport> {
        match &self.outline {
            Some(FormatOutline::PaginatedReport(r)) => Some(r),
            _ => None,
        }
    }

    /// Mashup details of an M script, template or dataflow.
    #[must_use]
    pub fn mashup(&self) -> Option<&Mashup> {
        match &self.outline {
            Some(FormatOutline::Mashup(m)) => Some(m),
            _ => None,
        }
    }

    /// Custom visual package metadata (`.pbiviz`).
    #[must_use]
    pub fn custom_visual(&self) -> Option<&CustomVisual> {
        match &self.outline {
            Some(FormatOutline::CustomVisual(v)) => Some(v),
            _ => None,
        }
    }

    /// Custom connector listing (`.mez`, `.pqx`).
    #[must_use]
    pub fn connector(&self) -> Option<&Connector> {
        match &self.outline {
            Some(FormatOutline::Connector(c)) => Some(c),
            _ => None,
        }
    }

    /// First metadata part of `kind`.
    #[must_use]
    pub fn part(&self, kind: &str) -> Option<&MetadataPart> {
        self.parts.iter().find(|p| p.kind == kind)
    }

    /// PBIX/PBIT `Version` text, e.g. `1.32`.
    #[must_use]
    pub fn package_version(&self) -> Option<&str> {
        self.part("version").and_then(|p| p.text.as_deref())
    }

    /// JSON for `inspect --json`: `{detection, document, parts,
    /// modelSource, extraModels, outline}` (no file contents, secrets
    /// redacted).
    #[must_use]
    pub fn inspect_json(&self) -> Value {
        let model_source = self.model_source.as_ref().map(|s| {
            serde_json::json!({
                "kind": s.kind(),
                "folder": s.folder(),
            })
        });
        let extra_models: Vec<Value> = self
            .extra_models
            .iter()
            .map(|m| {
                serde_json::json!({
                    "folder": m.source.folder(),
                    "kind": m.source.kind(),
                    "model": m.model,
                })
            })
            .collect();
        serde_json::json!({
            "detection": self.detection,
            "document": self.document,
            "parts": self.parts,
            "modelSource": model_source,
            "extraModels": extra_models,
            "outline": self.outline,
        })
    }
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Opens a file or folder.
///
/// Folders are opened as a [`DirContainer`], ZIP files as a
/// [`ZipContainer`], other files are read whole (up to
/// `limits.max_entry_size`). A `.pbip` file opens its project folder;
/// `definition.pbir`, `definition.pbism`, `.platform`, `model.tmdl` and
/// `database.tmdl` open their folder.
///
/// # Errors
/// [`CoreError::Io`] if the path cannot be read, [`CoreError::Container`]
/// if the folder or archive is refused (unsafe names, limits, corruption),
/// and [`CoreError::Unsupported`] if the format is not recognised.
pub fn open_path(path: &Path, opts: &OpenOptions) -> Result<OpenedDocument, CoreError> {
    open_path_inner(path, opts, true)
}

fn open_path_inner(
    path: &Path,
    opts: &OpenOptions,
    redirect: bool,
) -> Result<OpenedDocument, CoreError> {
    let meta = fs::metadata(path).map_err(|e| CoreError::io(path, e))?;
    let name = path.file_name().and_then(|n| n.to_str()).map(str::to_owned);
    let label = path.display().to_string();
    if meta.is_dir() {
        let dir = DirContainer::open(path, opts.limits)?;
        let mut detection = detect_container(&dir, name.as_deref());
        detection.parts.base_dir = Some(path.to_path_buf());
        let skipped = dir.skipped().to_vec();
        let container: Arc<dyn Container> = Arc::new(dir);
        let mut b = Builder::new(container, detection, label, name.as_deref(), opts);
        b.base_dir = Some(path.to_path_buf());
        b.note_skipped(&skipped);
        return b.read();
    }
    if !meta.is_file() {
        return Err(CoreError::Unsupported {
            format: "unknown".into(),
            hint: "not a regular file or directory".into(),
        });
    }
    if is_zip_file(path)? {
        let zip = ZipContainer::open(path, opts.limits)?;
        let mut detection = detect_container(&zip, name.as_deref());
        detection.signals.insert(0, "ZIP archive signature".into());
        let container: Arc<dyn Container> = Arc::new(zip);
        let b = Builder::new(container, detection, label, name.as_deref(), opts);
        return b.read();
    }

    let detection = detect_path(path, &opts.limits)?;
    let parent = path
        .parent()
        .map(|p| {
            if p.as_os_str().is_empty() {
                Path::new(".")
            } else {
                p
            }
        })
        .unwrap_or(Path::new("."));
    if detection.format == DetectedFormat::Pbip {
        let dir = DirContainer::open(parent, opts.limits)?;
        let skipped = dir.skipped().to_vec();
        let container: Arc<dyn Container> = Arc::new(dir);
        let mut b = Builder::new(container, detection, label, name.as_deref(), opts);
        b.base_dir = Some(parent.to_path_buf());
        b.note_skipped(&skipped);
        return b.read();
    }
    if redirect && name.as_deref().is_some_and(opens_parent_folder) {
        return open_path_inner(parent, opts, false);
    }

    if meta.len() > opts.limits.max_entry_size {
        return Err(crate::container::ContainerError::LimitExceeded {
            limit: crate::container::LimitKind::EntrySize,
            detail: format!("{label} is {} bytes", meta.len()),
        }
        .into());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|f| f.take(opts.limits.max_entry_size).read_to_end(&mut bytes))
        .map_err(|e| CoreError::io(path, e))?;
    let data: Arc<[u8]> = Arc::from(bytes);
    let container = BytesContainer::new(label.clone(), name.as_deref().unwrap_or("input"), data);
    let mut b = Builder::new(
        Arc::new(container.clone()),
        detection,
        label,
        name.as_deref(),
        opts,
    );
    b.base_dir = Some(parent.to_path_buf());
    single::read(b, &container)
}

/// Opens in-memory bytes. `name_hint` (a file name) is used as a detection
/// tie-breaker and for naming (report name, labels).
///
/// # Errors
/// [`CoreError::Container`] if a ZIP is refused or the bytes exceed
/// `limits.max_entry_size`, and [`CoreError::Unsupported`] if the format is
/// not recognised.
pub fn open_bytes(
    name_hint: &str,
    bytes: Vec<u8>,
    opts: &OpenOptions,
) -> Result<OpenedDocument, CoreError> {
    let name = file_name(&name_hint.replace('\\', "/")).to_owned();
    let label = if name_hint.is_empty() {
        "<bytes>".to_owned()
    } else {
        name_hint.to_owned()
    };
    let hint = (!name.is_empty()).then_some(name.as_str());
    if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") {
        let zip = ZipContainer::from_bytes_labeled(bytes, label.clone(), opts.limits)?;
        let mut detection = detect_container(&zip, hint);
        detection.signals.insert(0, "ZIP archive signature".into());
        let b = Builder::new(Arc::new(zip), detection, label, hint, opts);
        return b.read();
    }
    if bytes.len() as u64 > opts.limits.max_entry_size {
        return Err(crate::container::ContainerError::LimitExceeded {
            limit: crate::container::LimitKind::EntrySize,
            detail: format!("{label} is {} bytes", bytes.len()),
        }
        .into());
    }
    let detection = detect_bytes(hint, &bytes);
    let data: Arc<[u8]> = Arc::from(bytes);
    let container = BytesContainer::new(label.clone(), hint.unwrap_or("input"), data);
    let b = Builder::new(Arc::new(container.clone()), detection, label, hint, opts);
    single::read(b, &container)
}

fn is_zip_file(path: &Path) -> Result<bool, CoreError> {
    let mut head = Vec::with_capacity(4);
    File::open(path)
        .and_then(|f| f.take(4).read_to_end(&mut head))
        .map_err(|e| CoreError::io(path, e))?;
    Ok(head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06"))
}

/// Files that are only meaningful as part of their folder.
fn opens_parent_folder(name: &str) -> bool {
    [
        "definition.pbir",
        "definition.pbism",
        "definition.pbidataset",
        ".platform",
        "model.tmdl",
        "database.tmdl",
    ]
    .iter()
    .any(|n| name.eq_ignore_ascii_case(n))
}

// ---------------------------------------------------------------------------
// Document builder shared by the readers
// ---------------------------------------------------------------------------

pub(crate) struct Builder<'o> {
    pub opts: &'o OpenOptions,
    pub container: Arc<dyn Container>,
    pub detection: Detection,
    pub doc: Document,
    pub model_source: Option<ModelSource>,
    pub extra_models: Vec<ExtraModel>,
    pub report_sources: Vec<ReportSource>,
    pub parts: Vec<MetadataPart>,
    /// Entries interpreted as model/report definitions (not listed in
    /// `extra_parts`).
    pub consumed: HashSet<String>,
    /// Folder on disk the container paths resolve against (folders,
    /// `.pbip` files), for resolving `byPath` outside the container.
    pub base_dir: Option<PathBuf>,
    /// File name of the input (naming fallback).
    pub name: String,
    /// The model is remote (live-connected PBIX).
    pub remote: bool,
    /// Format-specific outline.
    pub outline: Option<FormatOutline>,
}

impl<'o> Builder<'o> {
    fn new(
        container: Arc<dyn Container>,
        detection: Detection,
        label: String,
        name: Option<&str>,
        opts: &'o OpenOptions,
    ) -> Self {
        let entries = container
            .entries()
            .iter()
            .filter(|e| !e.is_dir)
            .map(|e| e.path.clone())
            .collect();
        Self {
            opts,
            container,
            detection,
            doc: Document::new(SourceInfo {
                format: SourceFormat::default(),
                path: label,
                entries,
            }),
            model_source: None,
            extra_models: Vec::new(),
            report_sources: Vec::new(),
            parts: Vec::new(),
            consumed: HashSet::new(),
            base_dir: None,
            name: name.unwrap_or_default().to_owned(),
            remote: false,
            outline: None,
        }
    }

    pub(crate) fn diag(&mut self, d: Diagnostic) {
        self.doc.diagnostics.push(d);
    }

    fn note_skipped(&mut self, skipped: &[SkippedEntry]) {
        if skipped.is_empty() {
            return;
        }
        let sample: Vec<String> = skipped
            .iter()
            .take(5)
            .map(|s| format!("{} ({})", s.path, s.reason))
            .collect();
        self.diag(Diagnostic::info(
            codes::SKIPPED,
            format!(
                "{} entries skipped while listing: {}{}",
                skipped.len(),
                sample.join(", "),
                if skipped.len() > sample.len() {
                    ", …"
                } else {
                    ""
                }
            ),
        ));
    }

    /// Dispatches on the detected format.
    fn read(mut self) -> Result<OpenedDocument, CoreError> {
        use DetectedFormat as F;
        match self.detection.format.clone() {
            F::Pbip | F::PbipSemanticModelFolder | F::PbipReportFolder | F::TmdlFolder => {
                project::read(&mut self);
            }
            F::Pbix { .. } => package::read(&mut self, false),
            F::Pbit => package::read(&mut self, true),
            F::Vpax => vpax::read(&mut self),
            F::ExcelPowerPivot | F::ExcelNoModel => workbook::read(&mut self),
            F::PowerQueryTemplate => mashup::read_template(&mut self),
            F::DataflowFolder => mashup::read_dataflow_folder(&mut self),
            F::Pbiviz => extension::read_pbiviz(&mut self),
            F::CustomConnector => extension::read_connector(&mut self),
            F::Unknown => return Err(unsupported(&self.detection)),
            other => self.not_implemented(&other),
        }
        Ok(self.finish())
    }

    pub(crate) fn not_implemented(&mut self, format: &DetectedFormat) {
        self.doc.source.format = source_format_of(format);
        self.diag(Diagnostic::info(
            codes::NOT_IMPLEMENTED,
            format!(
                "detected {}; reader not implemented yet (see FORMATS.md)",
                format.label()
            ),
        ));
    }

    /// Parses a JSON metadata part (if present) into `parts`; failures are
    /// Info diagnostics.
    pub(crate) fn json_part(&mut self, path: &str, kind: &str) -> Option<usize> {
        if !self.container.is_file(path) {
            return None;
        }
        match files::read_json(self.container.as_ref(), path) {
            Ok((mut json, encoding)) => {
                let redacted = redact_json(&mut json);
                if !redacted.is_empty() {
                    self.diag(
                        Diagnostic::info(
                            codes::REDACTED,
                            format!(
                                "{} secret-looking value(s) redacted from {path}",
                                redacted.len()
                            ),
                        )
                        .with_origin(Origin::file(path)),
                    );
                }
                self.parts.push(MetadataPart {
                    path: path.to_owned(),
                    kind: kind.to_owned(),
                    encoding: Some(encoding),
                    text: None,
                    json: Some(json),
                    redacted,
                });
                Some(self.parts.len() - 1)
            }
            Err(e) => {
                self.diag(
                    Diagnostic::info(codes::METADATA, format!("{path}: {e}"))
                        .with_origin(Origin::file(path)),
                );
                None
            }
        }
    }

    /// Reads a short text part (e.g. PBIX `Version`).
    pub(crate) fn text_part(&mut self, path: &str, kind: &str) -> Option<String> {
        let entry = self.container.entry(path)?;
        if entry.is_dir || entry.size > 4096 {
            return None;
        }
        match self.container.read(path) {
            Ok(bytes) => {
                let decoded = decode_text(&bytes);
                let text = decoded
                    .text
                    .trim_start_matches('\u{feff}')
                    .trim()
                    .to_owned();
                self.parts.push(MetadataPart {
                    path: path.to_owned(),
                    kind: kind.to_owned(),
                    encoding: Some(decoded.encoding),
                    text: Some(text.clone()),
                    json: None,
                    redacted: Vec::new(),
                });
                Some(text)
            }
            Err(e) => {
                self.diag(
                    Diagnostic::warning(codes::READ_FAILED, format!("{path}: {e}"))
                        .with_origin(Origin::file(path)),
                );
                None
            }
        }
    }

    fn finish(mut self) -> OpenedDocument {
        self.doc.model_status = if self.doc.model.is_some() {
            ModelStatus::Local
        } else if self.remote
            || self
                .doc
                .reports
                .iter()
                .any(|r| r.dataset_reference.is_remote())
        {
            ModelStatus::Remote {
                description: REMOTE_DESCRIPTION.to_owned(),
            }
        } else {
            ModelStatus::Missing
        };
        self.doc.extra_parts = self
            .container
            .entries()
            .iter()
            .filter(|e| !e.is_dir && !self.consumed.contains(&e.path))
            .map(|e| PartInfo {
                path: e.path.clone(),
                kind: part_kind(&e.path).to_owned(),
                size: Some(e.size),
            })
            .collect();
        self.doc.assign_ids();
        for extra in &mut self.extra_models {
            let mut tmp = Document {
                ids: self.doc.ids.clone(),
                model: Some(std::mem::take(&mut extra.model)),
                ..Document::default()
            };
            tmp.assign_ids();
            self.doc.ids = tmp.ids;
            extra.model = tmp.model.unwrap_or_default();
        }
        OpenedDocument {
            document: self.doc,
            detection: self.detection,
            container: self.container,
            model_source: self.model_source,
            extra_models: self.extra_models,
            report_sources: self.report_sources,
            parts: self.parts,
            outline: self.outline,
        }
    }
}

fn unsupported(detection: &Detection) -> CoreError {
    CoreError::Unsupported {
        format: detection.format.label(),
        hint: if detection.signals.is_empty() {
            "no known format signature".into()
        } else {
            detection.signals.join("; ")
        },
    }
}

/// The [`SourceFormat`] of a detected format.
#[must_use]
pub fn source_format_of(format: &DetectedFormat) -> SourceFormat {
    use DetectedFormat as F;
    match format {
        F::Pbip => SourceFormat::Pbip,
        F::PbipSemanticModelFolder => SourceFormat::SemanticModelFolder,
        F::PbipReportFolder => SourceFormat::ReportFolder,
        F::Pbix { .. } => SourceFormat::Pbix,
        F::Pbit => SourceFormat::Pbit,
        F::Bim => SourceFormat::Bim,
        F::TmdlFolder | F::TmdlFile => SourceFormat::TmdlFolder,
        F::Pbids => SourceFormat::Pbids,
        F::Rdl => SourceFormat::Rdl,
        F::Abf { .. } => SourceFormat::Abf,
        F::Vpax => SourceFormat::Vpax,
        F::ExcelPowerPivot => SourceFormat::PowerPivot,
        F::PowerQueryTemplate => SourceFormat::PowerQueryTemplate,
        F::PowerQueryScript => SourceFormat::PowerQueryScript,
        F::DataflowModelJson => SourceFormat::DataflowModel,
        F::Pbiviz => SourceFormat::Pbiviz,
        F::CustomConnector => SourceFormat::Connector,
        F::LegacyLayoutJson => SourceFormat::PbirLegacy,
        other => SourceFormat::Unknown(other.label()),
    }
}

/// Kind of a container entry for [`Document::extra_parts`].
#[must_use]
pub fn part_kind(path: &str) -> &'static str {
    let name = file_name(path);
    let lower = name.to_ascii_lowercase();
    let in_static = path.contains("StaticResources/");
    match name {
        "DataModel" => return "dataModel",
        "DataModelSchema" => return "dataModelSchema",
        "SecurityBindings" => return "securityBindings",
        "DataMashup" => return "dataMashup",
        "Version" => return "version",
        "[Content_Types].xml" => return "contentTypes",
        "Settings" => return "settings",
        "Metadata" => return "metadata",
        "Connections" => return "connections",
        "DiagramLayout" | "diagramLayout.json" => return "diagramLayout",
        "LinguisticSchema" => return "linguisticSchema",
        "Layout" => return "layout",
        ".platform" => return "platform",
        "definition.pbism" | "definition.pbidataset" => return "pbism",
        "definition.pbir" => return "pbir",
        "localSettings.json" => return "localSettings",
        "cache.abf" => return "cacheAbf",
        _ => {}
    }
    if lower.ends_with(".pbip") {
        "pbip"
    } else if lower.ends_with(".tmdl") {
        "tmdl"
    } else if lower.ends_with(".bim") {
        "bim"
    } else if in_static && (path.contains("/BaseThemes/") || lower.ends_with(".json")) {
        "theme"
    } else if [
        ".png", ".jpg", ".jpeg", ".gif", ".svg", ".bmp", ".webp", ".ico",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
    {
        "image"
    } else if in_static {
        "staticResource"
    } else if path.contains("/.pbi/") || path.starts_with(".pbi/") {
        "pbiSettings"
    } else if path.contains("definition/") && lower.ends_with(".json") {
        "reportDefinition"
    } else {
        "other"
    }
}
