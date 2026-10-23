//! Content-based format detection.
//!
//! Formats are recognised from what the bytes contain (ZIP entry names,
//! JSON keys and `$schema` URLs, XML namespaces, ABF signatures, leading
//! keywords) and only fall back to the file extension when the content is
//! inconclusive ([`Confidence::ExtensionOnly`]). When the extension
//! disagrees with the content, the content wins and a signal says so.
//!
//! Detection is cheap and bounded: ZIP archives and folders are classified
//! from their entry listing (only small metadata files such as `.pbip`,
//! `.platform` and `definition.pbir` are read, up to 1 MiB each); single
//! files are sniffed from at most 32 MiB. JSON is scanned with a bounded,
//! shallow parser that never builds a full document tree.
//!
//! | Input | Signals |
//! |---|---|
//! | ZIP | `DataModel` → PBIX; `DataModelSchema` → PBIT; `Report/` + `Connections` → thin PBIX; `Report/Layout` vs `Report/definition/` → report flavour; `xl/workbook.xml` (+ `xl/model/item.data`) → Excel (Power Pivot); `DaxModel.json`/`DaxVpaView.json` → VPAX; `package.json` + `resources/*.pbiviz.json` → PBIVIZ; `MashupDocument.pq` (or `Formulas/Section1.m`) → PQT; root `*.m`/`*.pqm`/`*.pq` + `[Content_Types].xml`/`*.resx` → connector (a root `.pq` without `.resx` → PQT) |
//! | Folder | `*.pbip` → PBIP; `definition.pbism` → semantic model folder; `definition.pbir` → report folder; `.platform` `metadata.type`; `model.tmdl`/`database.tmdl` → TMDL folder; item sub-folders without `.pbip` → PBIP (Fabric Git layout); otherwise the ZIP rules (unzipped PBIX) |
//! | JSON | `$schema` URL under `/fabric/…`; `compatibilityLevel` + `model` → BIM; `connections[].details.protocol` → PBIDS; `version` + `artifacts` → PBIP; `entities` + `pbi:mashup` → dataflow `model.json`; `datasetReference` → `definition.pbir`; `sections` → legacy layout |
//! | Other | UTF-16LE `This backup was created using …XPress9/XPrs9` → compressed ABF; `FF FE` + UTF-16LE `STREAM_STORAGE_SIGNATURE_)!@#$%^&*(` → ABF; XML root `<Report>` in a `reportdefinition` namespace → RDL; `section`/`let` → M; TMDL top-level keyword → TMDL file |
//!
//! PQT layout (Power Query template, e.g. exported from Power Query Online
//! or Dataflow Gen2): an OPC ZIP with `MashupDocument.pq` (the M section
//! document), `MashupMetadata.json` (query metadata), `Metadata.json`
//! (template metadata) and `[Content_Types].xml`. Packages laid out like
//! the PBIX `DataMashup` part (`Formulas/Section1.m`, `Config/Package.xml`)
//! are reported as PQT with [`Confidence::Likely`].

use std::cell::Cell;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};

use crate::container::path::{join, normalize_lookup};
use crate::container::{
    Container, ContainerError, ContainerKind, DecodedText, DirContainer, Limits, TextEncoding,
    ZipContainer,
};

pub use crate::container::decode_text;

/// Bytes read from a file before deciding how to sniff it.
const HEAD_LEN: u64 = 64 * 1024;
/// Maximum bytes of a single file sniffed as text/JSON.
const TEXT_SNIFF_MAX: u64 = 32 * 1024 * 1024;
/// Maximum size of a metadata file (`.pbip`, `.platform`, ...) read while
/// classifying a folder or archive.
const SMALL_JSON_MAX: u64 = 1024 * 1024;

/// Result of format detection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detection {
    /// The detected format.
    pub format: DetectedFormat,
    /// How sure the detection is.
    pub confidence: Confidence,
    /// Human-readable reasons, in the order they were found (for
    /// `inspect` output).
    pub signals: Vec<String>,
    /// Structure discovered along the way (PBIP items, report flavours).
    pub parts: DetectedParts,
}

/// How sure a [`Detection`] is. Ordered from weakest to strongest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Confidence {
    /// Nothing matched (the format is [`DetectedFormat::Unknown`]).
    NoSignal,
    /// Only the file extension matched; payload fields of the format (e.g.
    /// `Pbix::has_data_model`) are defaults.
    ExtensionOnly,
    /// Heuristic content match (keywords, partial or truncated JSON).
    Likely,
    /// Structural content match (signature, required entries or keys).
    Certain,
}

/// A detected format.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum DetectedFormat {
    /// PBIP project: a folder with a `*.pbip` file (or a Fabric Git /
    /// deployment export with item folders), or the `.pbip` file itself.
    /// Items are listed in [`DetectedParts`].
    Pbip,
    /// A `*.SemanticModel` folder (`definition.pbism`).
    PbipSemanticModelFolder,
    /// A `*.Report` folder (`definition.pbir`).
    PbipReportFolder,
    /// PBIX (ZIP, or an unzipped PBIX folder).
    Pbix {
        /// Has a `DataModel` entry (imported data). `false` for thin /
        /// live-connected reports.
        has_data_model: bool,
        /// Which report format the `Report/` folder holds.
        report: PbixReport,
    },
    /// PBIT template (ZIP with `DataModelSchema`).
    Pbit,
    /// TMSL JSON (`model.bim`, or a PBIT `DataModelSchema` on its own).
    Bim,
    /// A standalone TMDL folder (`model.tmdl` / `database.tmdl`).
    TmdlFolder,
    /// A single TMDL file.
    TmdlFile,
    /// `.pbids` data-source connection file.
    Pbids,
    /// Paginated report definition (`.rdl`).
    Rdl,
    /// Analysis Services backup.
    Abf {
        /// XPress9-compressed (as in a PBIX `DataModel`).
        compressed: bool,
    },
    /// VertiPaq Analyzer export.
    Vpax,
    /// Excel workbook with a Power Pivot model (`xl/model/item.data`).
    ExcelPowerPivot,
    /// Excel workbook without a data model.
    ExcelNoModel,
    /// Power Query template (`.pqt`).
    PowerQueryTemplate,
    /// Power Query M script (`.pq`, `.m`).
    PowerQueryScript,
    /// Dataflow (CDM) `model.json`.
    DataflowModelJson,
    /// Fabric Dataflow Gen2 item folder (`.platform` type `Dataflow`,
    /// `mashup.pq`).
    DataflowFolder,
    /// Another Fabric Git item folder, identified by `.platform`.
    FabricItemFolder {
        /// `.platform` `metadata.type`.
        item_type: String,
    },
    /// Custom visual package (never executed).
    Pbiviz,
    /// Power Query custom connector (`.mez`/`.pqx`; never executed).
    CustomConnector,
    /// A single PBIR JSON file identified by its `$schema` (or by its keys
    /// for `definition.pbir`).
    PbirJsonFile {
        /// Schema kind from the URL, e.g. `visualContainer`, `page`,
        /// `report`, `pagesMetadata`, `versionMetadata`,
        /// `definitionProperties`.
        schema_kind: String,
    },
    /// A PBIX `Report/Layout` or PBIR-Legacy `report.json` (`sections[]`).
    LegacyLayoutJson,
    /// Another Fabric JSON file (`.platform`, `definition.pbism`, ...).
    FabricJsonFile {
        /// Area from the schema URL, e.g. `semanticModel`, `gitIntegration`.
        item: String,
        /// Schema kind, e.g. `definitionProperties`, `platformProperties`.
        schema_kind: String,
    },
    /// Not recognised.
    Unknown,
}

impl DetectedFormat {
    /// Short human-readable name.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Pbip => "PBIP project".into(),
            Self::PbipSemanticModelFolder => "PBIP semantic model folder".into(),
            Self::PbipReportFolder => "PBIP report folder".into(),
            Self::Pbix {
                has_data_model,
                report,
            } => format!(
                "PBIX ({}, report: {report})",
                if *has_data_model {
                    "with data model"
                } else {
                    "no data model"
                }
            ),
            Self::Pbit => "PBIT template".into(),
            Self::Bim => "TMSL model (.bim)".into(),
            Self::TmdlFolder => "TMDL folder".into(),
            Self::TmdlFile => "TMDL file".into(),
            Self::Pbids => "PBIDS connection file".into(),
            Self::Rdl => "RDL paginated report".into(),
            Self::Abf { compressed: true } => "ABF backup (XPress9-compressed)".into(),
            Self::Abf { compressed: false } => "ABF backup".into(),
            Self::Vpax => "VertiPaq Analyzer export (.vpax)".into(),
            Self::ExcelPowerPivot => "Excel workbook with Power Pivot model".into(),
            Self::ExcelNoModel => "Excel workbook (no data model)".into(),
            Self::PowerQueryTemplate => "Power Query template (.pqt)".into(),
            Self::PowerQueryScript => "Power Query M script".into(),
            Self::DataflowModelJson => "Dataflow model.json (CDM)".into(),
            Self::DataflowFolder => "Dataflow Gen2 item folder".into(),
            Self::FabricItemFolder { item_type } => format!("Fabric item folder ({item_type})"),
            Self::Pbiviz => "Custom visual package (.pbiviz)".into(),
            Self::CustomConnector => "Power Query custom connector".into(),
            Self::PbirJsonFile { schema_kind } => format!("PBIR JSON file ({schema_kind})"),
            Self::LegacyLayoutJson => "Legacy report layout JSON".into(),
            Self::FabricJsonFile { item, schema_kind } => {
                format!("Fabric JSON file ({item}/{schema_kind})")
            }
            Self::Unknown => "unknown".into(),
        }
    }

    /// Coarse family used to compare content with extension.
    fn family(&self) -> &'static str {
        match self {
            Self::Pbip => "pbip",
            Self::PbipSemanticModelFolder => "semantic-model",
            Self::PbipReportFolder => "report",
            Self::Pbix { .. } => "pbix",
            Self::Pbit => "pbit",
            Self::Bim => "bim",
            Self::TmdlFolder | Self::TmdlFile => "tmdl",
            Self::Pbids => "pbids",
            Self::Rdl => "rdl",
            Self::Abf { .. } => "abf",
            Self::Vpax => "vpax",
            Self::ExcelPowerPivot | Self::ExcelNoModel => "excel",
            Self::PowerQueryTemplate => "pqt",
            Self::PowerQueryScript => "pq",
            Self::DataflowModelJson => "dataflow-json",
            Self::DataflowFolder => "dataflow",
            Self::FabricItemFolder { .. } => "fabric-item",
            Self::Pbiviz => "pbiviz",
            Self::CustomConnector => "connector",
            Self::PbirJsonFile { .. } => "pbir-json",
            Self::LegacyLayoutJson => "legacy-layout",
            Self::FabricJsonFile { .. } => "fabric-json",
            Self::Unknown => "unknown",
        }
    }
}

/// Report format inside a PBIX/PBIT `Report/` folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PbixReport {
    /// `Report/definition/` (PBIR).
    Pbir,
    /// Legacy single `Report/Layout` file.
    LegacyLayout,
    /// No report found.
    None,
}

impl fmt::Display for PbixReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pbir => "PBIR",
            Self::LegacyLayout => "legacy Layout",
            Self::None => "none",
        })
    }
}

/// Structure discovered during detection. Paths are normalized and
/// relative to [`base_dir`](Self::base_dir) (folders), the archive root
/// (ZIPs) or the detected folder itself (`""`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedParts {
    /// Directory the relative paths resolve against (set by
    /// [`detect_path`] for folders and `.pbip` files).
    pub base_dir: Option<PathBuf>,
    /// The `.pbip` project file.
    pub pbip_file: Option<String>,
    /// `artifacts[].*.path` values from the `.pbip` file, verbatim.
    pub pbip_artifacts: Vec<String>,
    /// Semantic model folders (or the folder itself).
    pub semantic_models: Vec<SemanticModelPart>,
    /// Report folders (or the PBIX/PBIT `Report/` folder).
    pub reports: Vec<ReportPart>,
    /// Other Fabric item folders (`.platform` with another type).
    pub other_items: Vec<FabricItemPart>,
    /// Text encoding of a sniffed text file.
    pub text_encoding: Option<TextEncoding>,
}

/// A semantic model folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticModelPart {
    /// Folder path (`""` for the detected folder itself).
    pub path: String,
    /// How the model is stored.
    pub flavour: ModelFlavour,
    /// `.pbi/cache.abf` (local VertiPaq cache) is present.
    pub has_cache_abf: bool,
    /// Parsed `.platform`, if present.
    pub platform: Option<PlatformInfo>,
}

/// How a semantic model is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelFlavour {
    /// `definition/*.tmdl`.
    Tmdl,
    /// TMSL `model.bim`.
    Bim,
    /// Neither found.
    Unknown,
}

/// A report folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportPart {
    /// Folder path (`""` for the detected folder itself, `Report` in a
    /// PBIX/PBIT).
    pub path: String,
    /// Report format.
    pub flavour: ReportFlavour,
    /// The `definition.pbir` dataset reference.
    pub dataset: DatasetReference,
    /// Parsed `.platform`, if present.
    pub platform: Option<PlatformInfo>,
}

/// Report format of a report folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReportFlavour {
    /// `definition/` folder (PBIR).
    Pbir,
    /// Single `report.json` at the folder root (PBIR-Legacy).
    PbirLegacy,
    /// PBIX/PBIT `Report/Layout`.
    LegacyLayout,
    /// Neither found.
    Unknown,
}

/// Where a report's semantic model lives (`definition.pbir`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum DatasetReference {
    /// `datasetReference.byPath`: a local model folder.
    ByPath {
        /// The path as written (relative to the report folder).
        path: String,
        /// Resolved against the detection root; `None` if it points
        /// outside it.
        resolved: Option<String>,
    },
    /// `datasetReference.byConnection`: a service-hosted model (remote,
    /// not available offline).
    ByConnection,
    /// No or unreadable `definition.pbir` (or not applicable).
    Unknown,
}

/// Parsed Fabric Git `.platform` metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformInfo {
    /// `metadata.type`, e.g. `SemanticModel`, `Report`, `Dataflow`.
    pub item_type: Option<String>,
    /// `metadata.displayName`.
    pub display_name: Option<String>,
    /// `config.logicalId`.
    pub logical_id: Option<String>,
}

/// A Fabric item folder that is neither a model nor a report.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FabricItemPart {
    /// Folder path.
    pub path: String,
    /// `.platform` `metadata.type`.
    pub item_type: String,
}

/// Error from [`detect_path`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DetectError {
    /// The path could not be read.
    #[error("I/O error on {path:?}: {source}")]
    Io {
        /// The path.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: io::Error,
    },
    /// The folder or archive was refused by the container layer (unsafe
    /// names, limits, corruption).
    #[error(transparent)]
    Container(#[from] ContainerError),
}

impl Detection {
    fn new(format: DetectedFormat, confidence: Confidence) -> Self {
        Self {
            format,
            confidence,
            signals: Vec::new(),
            parts: DetectedParts::default(),
        }
    }

    fn unknown() -> Self {
        Self::new(DetectedFormat::Unknown, Confidence::NoSignal)
    }

    fn with(mut self, signal: impl Into<String>) -> Self {
        self.signals.push(signal.into());
        self
    }

    fn signal(&mut self, signal: impl Into<String>) {
        self.signals.push(signal.into());
    }
}

/// What kind of content the extension is compared against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentKind {
    Zip,
    Dir,
    Other,
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Detects the format of a file or folder on disk.
///
/// Folders are opened as a [`DirContainer`] and ZIP files as a
/// [`ZipContainer`] with `limits`; other files are sniffed from at most
/// 32 MiB. For a `.pbip` file, the project folder next to it is scanned
/// for its items.
///
/// # Errors
/// [`DetectError::Io`] if the path cannot be read, and
/// [`DetectError::Container`] if the folder or archive is refused (unsafe
/// names, limits exceeded, corrupt ZIP).
pub fn detect_path(path: &Path, limits: &Limits) -> Result<Detection, DetectError> {
    let io_err = |source| DetectError::Io {
        path: path.to_path_buf(),
        source,
    };
    let meta = fs::metadata(path).map_err(io_err)?;
    let name = path.file_name().and_then(|n| n.to_str());
    if meta.is_dir() {
        let dir = DirContainer::open(path, *limits)?;
        let mut d = detect_container(&dir, name);
        d.parts.base_dir = Some(path.to_path_buf());
        if !dir.skipped().is_empty() {
            d.signal(format!(
                "{} entries skipped while listing (symlinks, VCS/tool folders, ...)",
                dir.skipped().len()
            ));
        }
        return Ok(d);
    }
    if !meta.is_file() {
        return Ok(Detection::unknown().with("not a regular file or directory"));
    }
    let mut file = File::open(path).map_err(io_err)?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(HEAD_LEN)
        .read_to_end(&mut bytes)
        .map_err(io_err)?;
    if is_zip_magic(&bytes) {
        drop(file);
        let zip = ZipContainer::open(path, *limits)?;
        let mut d = detect_container(&zip, name);
        d.signals.insert(0, "ZIP archive signature".into());
        return Ok(d);
    }
    let len = meta.len();
    if len > bytes.len() as u64 && sniff_abf(&bytes).is_none() && head_is_textual(&bytes) {
        let cap = TEXT_SNIFF_MAX.min(limits.max_entry_size);
        let more = cap.saturating_sub(bytes.len() as u64);
        file.take(more).read_to_end(&mut bytes).map_err(io_err)?;
    }
    let truncated = len > bytes.len() as u64;
    let mut d = sniff(name, &bytes, truncated);
    if d.format == DetectedFormat::Pbip
        && let (Some(parent), Some(file_name)) = (path.parent(), name)
    {
        let parent = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        match DirContainer::open(parent, *limits) {
            Ok(dir) => {
                d.parts.base_dir = Some(parent.to_path_buf());
                d.parts.pbip_file = Some(file_name.to_owned());
                let artifacts = d.parts.pbip_artifacts.clone();
                scan_items(&dir, &mut d, &artifacts, false);
            }
            Err(e) => d.signal(format!("could not scan the project folder: {e}")),
        }
    }
    Ok(d)
}

/// Detects the format of in-memory bytes. `name_hint` (a file name) is
/// only used as a tie-breaker and to report extension mismatches. ZIP
/// input is opened with [`Limits::default`]; a refused archive yields
/// [`DetectedFormat::Unknown`] (or the extension's format) with a signal.
#[must_use]
pub fn detect_bytes(name_hint: Option<&str>, bytes: &[u8]) -> Detection {
    sniff(name_hint, bytes, false)
}

/// Detects the format of an already-opened container (folder or ZIP) from
/// its entry listing and small metadata files.
#[must_use]
pub fn detect_container(container: &dyn Container, name_hint: Option<&str>) -> Detection {
    let (mut d, kind) = if container.kind() == ContainerKind::Zip {
        (classify_container(container, true), ContentKind::Zip)
    } else {
        (classify_container(container, false), ContentKind::Dir)
    };
    apply_extension(&mut d, name_hint, kind);
    d
}

fn sniff(name_hint: Option<&str>, bytes: &[u8], truncated: bool) -> Detection {
    let (mut d, kind) = sniff_content(name_hint, bytes, truncated);
    apply_extension(&mut d, name_hint, kind);
    d
}

fn sniff_content(
    name_hint: Option<&str>,
    bytes: &[u8],
    truncated: bool,
) -> (Detection, ContentKind) {
    if bytes.is_empty() {
        return (Detection::unknown().with("empty input"), ContentKind::Other);
    }
    if is_zip_magic(bytes) {
        let label = name_hint.unwrap_or("<bytes>");
        let d = match ZipContainer::from_bytes_labeled(bytes.to_vec(), label, Limits::default()) {
            Ok(zip) => {
                let mut d = classify_container(&zip, true);
                d.signals.insert(0, "ZIP archive signature".into());
                d
            }
            Err(e) => Detection::unknown().with(format!(
                "ZIP archive signature, but the archive was refused: {e}"
            )),
        };
        return (d, ContentKind::Zip);
    }
    if let Some(d) = sniff_abf(bytes) {
        return (d, ContentKind::Other);
    }
    if !head_is_textual(bytes) {
        return (
            Detection::unknown().with("binary data without a known signature"),
            ContentKind::Other,
        );
    }
    let decoded = decode_text(bytes);
    (
        sniff_text(name_hint, &decoded, truncated),
        ContentKind::Other,
    )
}

// ---------------------------------------------------------------------------
// Extension handling
// ---------------------------------------------------------------------------

fn extension_of(name: &str) -> Option<String> {
    let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    if name.eq_ignore_ascii_case(".platform") {
        return Some("platform".into());
    }
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

fn format_for_extension(ext: &str, kind: ContentKind) -> Option<DetectedFormat> {
    use DetectedFormat as F;
    let f = match ext {
        "pbix" => F::Pbix {
            has_data_model: false,
            report: PbixReport::None,
        },
        "pbit" => F::Pbit,
        "pbip" => F::Pbip,
        "bim" => F::Bim,
        "tmdl" => F::TmdlFile,
        "pbids" => F::Pbids,
        "rdl" => F::Rdl,
        "abf" => F::Abf { compressed: false },
        "vpax" => F::Vpax,
        "xlsx" | "xlsm" | "xlsb" => F::ExcelNoModel,
        "pqt" => F::PowerQueryTemplate,
        "pq" | "m" | "pqm" => F::PowerQueryScript,
        "pbiviz" => F::Pbiviz,
        "mez" | "pqx" => F::CustomConnector,
        "pbism" | "pbidataset" => F::FabricJsonFile {
            item: "semanticModel".into(),
            schema_kind: "definitionProperties".into(),
        },
        "pbir" => F::PbirJsonFile {
            schema_kind: "definitionProperties".into(),
        },
        "platform" => F::FabricJsonFile {
            item: "gitIntegration".into(),
            schema_kind: "platformProperties".into(),
        },
        "semanticmodel" | "dataset" if kind == ContentKind::Dir => F::PbipSemanticModelFolder,
        "report" if kind == ContentKind::Dir => F::PbipReportFolder,
        _ => return None,
    };
    Some(f)
}

fn is_zip_based(f: &DetectedFormat) -> bool {
    matches!(
        f,
        DetectedFormat::Pbix { .. }
            | DetectedFormat::Pbit
            | DetectedFormat::Vpax
            | DetectedFormat::ExcelNoModel
            | DetectedFormat::ExcelPowerPivot
            | DetectedFormat::PowerQueryTemplate
            | DetectedFormat::Pbiviz
            | DetectedFormat::CustomConnector
    )
}

fn apply_extension(d: &mut Detection, name_hint: Option<&str>, kind: ContentKind) {
    let Some(ext) = name_hint.and_then(extension_of) else {
        return;
    };
    let Some(by_ext) = format_for_extension(&ext, kind) else {
        return;
    };
    if d.format == DetectedFormat::Unknown {
        if kind == ContentKind::Zip
            && is_zip_based(&by_ext)
            && d.signals.iter().all(|s| !s.contains("refused"))
        {
            d.confidence = Confidence::Likely;
            d.signal(format!(
                "ZIP layout not recognised; `.{ext}` extension suggests {}",
                by_ext.label()
            ));
        } else {
            d.confidence = Confidence::ExtensionOnly;
            d.signal(format!(
                "no content signal matched; using the `.{ext}` extension ({})",
                by_ext.label()
            ));
        }
        d.format = by_ext;
    } else if by_ext.family() == d.format.family() {
        if d.confidence == Confidence::Likely {
            d.confidence = Confidence::Certain;
            d.signal(format!("`.{ext}` extension agrees"));
        }
    } else {
        d.signal(format!(
            "extension `.{ext}` suggests {}, but the content is {}; using the content",
            by_ext.label(),
            d.format.label()
        ));
    }
}

// ---------------------------------------------------------------------------
// Containers (ZIP archives and folders)
// ---------------------------------------------------------------------------

fn classify_container(c: &dyn Container, zip_first: bool) -> Detection {
    if zip_first {
        if let Some(d) = detect_package(c) {
            return d;
        }
        if let Some(d) = detect_tree(c) {
            return d;
        }
    } else {
        if let Some(d) = detect_tree(c) {
            return d;
        }
        if let Some(mut d) = detect_package(c) {
            d.signals.insert(
                0,
                "folder laid out like a package (e.g. an unzipped PBIX)".into(),
            );
            return d;
        }
    }
    let top: Vec<String> = c
        .list_dir("")
        .into_iter()
        .take(8)
        .map(|ch| {
            if ch.is_dir {
                format!("{}/", ch.name)
            } else {
                ch.name
            }
        })
        .collect();
    Detection::unknown().with(format!(
        "{} entries, no known layout (top level: {})",
        c.entries().len(),
        top.join(", ")
    ))
}

fn pbix_report(c: &dyn Container, d: &mut Detection) -> PbixReport {
    let layout = c.is_file("Report/Layout");
    let pbir = c.is_dir("Report/definition");
    let (report, flavour) = match (pbir, layout) {
        (true, also_layout) => {
            d.signal("`Report/definition/` (PBIR report)");
            if also_layout {
                d.signal("also has a legacy `Report/Layout` (PBIR preferred)");
            }
            (PbixReport::Pbir, ReportFlavour::Pbir)
        }
        (false, true) => {
            d.signal("`Report/Layout` (legacy report layout)");
            (PbixReport::LegacyLayout, ReportFlavour::LegacyLayout)
        }
        (false, false) => {
            d.signal("no `Report/Layout` or `Report/definition/`");
            (PbixReport::None, ReportFlavour::Unknown)
        }
    };
    if report != PbixReport::None {
        d.parts.reports.push(ReportPart {
            path: "Report".into(),
            flavour,
            dataset: DatasetReference::Unknown,
            platform: None,
        });
    }
    report
}

/// ZIP-style packages, classified by entry names.
fn detect_package(c: &dyn Container) -> Option<Detection> {
    use DetectedFormat as F;
    let has = |p: &str| c.is_file(p);
    let has_dir = |p: &str| c.is_dir(p);

    if has("DataModel") {
        let mut d = Detection::new(F::Unknown, Confidence::Certain)
            .with("entry `DataModel` (VertiPaq model: XPress9-compressed ABF)");
        if has("DataModelSchema") {
            d.signal("also has `DataModelSchema`; `DataModel` wins");
        }
        let report = pbix_report(c, &mut d);
        d.format = F::Pbix {
            has_data_model: true,
            report,
        };
        return Some(d);
    }
    if has("DataModelSchema") {
        let mut d = Detection::new(F::Pbit, Confidence::Certain)
            .with("entry `DataModelSchema` without `DataModel` (TMSL model, no data)");
        pbix_report(c, &mut d);
        return Some(d);
    }
    if has_dir("Report") && (has("Connections") || (has("Version") && has("[Content_Types].xml"))) {
        let certain = has("Connections");
        let mut d = Detection::new(
            F::Unknown,
            if certain {
                Confidence::Certain
            } else {
                Confidence::Likely
            },
        );
        if certain {
            d.signal(
                "`Report/` + `Connections` without `DataModel`: thin report (live connection)",
            );
        } else {
            d.signal("`Report/` + `Version` + `[Content_Types].xml` without `DataModel`");
        }
        let report = pbix_report(c, &mut d);
        d.format = F::Pbix {
            has_data_model: false,
            report,
        };
        return Some(d);
    }
    if has("xl/workbook.xml") || has("xl/workbook.bin") {
        return Some(if has("xl/model/item.data") {
            Detection::new(F::ExcelPowerPivot, Confidence::Certain)
                .with("`xl/workbook.*` + `xl/model/item.data` (Power Pivot ABF)")
        } else {
            Detection::new(F::ExcelNoModel, Confidence::Certain)
                .with("`xl/workbook.*` without `xl/model/item.data`")
        });
    }
    if has("DaxVpaView.json") || has("DaxModel.json") {
        let mut d = Detection::new(F::Vpax, Confidence::Certain);
        for name in ["DaxModel.json", "DaxVpaView.json", "Model.bim"] {
            if has(name) {
                d.signal(format!("entry `{name}`"));
            }
        }
        return Some(d);
    }
    if has("package.json") {
        if c.list_prefix("resources")
            .iter()
            .any(|e| !e.is_dir && e.path.ends_with(".pbiviz.json"))
        {
            return Some(Detection::new(F::Pbiviz, Confidence::Certain).with(
                "`package.json` + `resources/*.pbiviz.json` (custom visual; never executed)",
            ));
        }
        if has_dir("resources") {
            return Some(
                Detection::new(F::Pbiviz, Confidence::Likely)
                    .with("`package.json` + `resources/` (custom visual?; never executed)"),
            );
        }
    }
    if has("MashupDocument.pq") {
        let mut d = Detection::new(F::PowerQueryTemplate, Confidence::Certain)
            .with("entry `MashupDocument.pq` (Power Query template)");
        if has("MashupMetadata.json") {
            d.signal("entry `MashupMetadata.json`");
        }
        return Some(d);
    }
    if has("Formulas/Section1.m") {
        return Some(
            Detection::new(F::PowerQueryTemplate, Confidence::Likely)
                .with("`Formulas/Section1.m` (DataMashup-style M package)"),
        );
    }
    let root = c.list_dir("");
    let root_file_ends = |suffix: &str| {
        root.iter()
            .any(|ch| !ch.is_dir && ends_with_ci(&ch.name, suffix))
    };
    // Connectors ship a `resources.resx` (and icons) next to their M source.
    if has("[Content_Types].xml") && root_file_ends(".pq") && !root_file_ends(".resx") {
        return Some(
            Detection::new(F::PowerQueryTemplate, Confidence::Likely)
                .with("`[Content_Types].xml` + a root `.pq` file"),
        );
    }
    if (root_file_ends(".m") || root_file_ends(".pqm") || root_file_ends(".pq"))
        && (has("[Content_Types].xml") || root_file_ends(".resx"))
    {
        return Some(Detection::new(F::CustomConnector, Confidence::Likely).with(
            "root `.m`/`.pqm` sources in an OPC package (custom connector; never executed)",
        ));
    }
    None
}

/// Folder layouts (PBIP and friends), classified by entry names and small
/// metadata files.
fn detect_tree(c: &dyn Container) -> Option<Detection> {
    use DetectedFormat as F;
    let root = c.list_dir("");
    let root_file = |name: &str| root.iter().any(|ch| !ch.is_dir && ch.name == name);

    let pbip_files: Vec<_> = root
        .iter()
        .filter(|ch| !ch.is_dir && ends_with_ci(&ch.name, ".pbip"))
        .collect();
    if let Some(first) = pbip_files.first() {
        let mut d = Detection::new(F::Pbip, Confidence::Certain)
            .with(format!("project file `{}`", first.path));
        if pbip_files.len() > 1 {
            d.signal(format!(
                "{} `.pbip` files; using the first",
                pbip_files.len()
            ));
        }
        d.parts.pbip_file = Some(first.path.clone());
        match read_small_json(c, &first.path) {
            Ok(json) => {
                d.parts.pbip_artifacts = pbip_artifacts(&json);
                if json.get("artifacts").is_none() {
                    d.signal("`.pbip` file has no `artifacts` array");
                }
            }
            Err(e) => d.signal(format!("`.pbip` file unreadable: {e}")),
        }
        let artifacts = d.parts.pbip_artifacts.clone();
        scan_items(c, &mut d, &artifacts, false);
        return Some(d);
    }

    let platform = read_platform(c, ".platform");
    let platform_type = platform.as_ref().and_then(|p| p.item_type.clone());

    if root_file("definition.pbism") || root_file("definition.pbidataset") {
        let mut d = Detection::new(F::PbipSemanticModelFolder, Confidence::Certain)
            .with("`definition.pbism` (semantic model definition)");
        let part = analyze_model(c, "");
        d.signal(describe_model(&part));
        d.parts.semantic_models.push(part);
        return Some(d);
    }
    if root_file("definition.pbir") {
        let mut d = Detection::new(F::PbipReportFolder, Confidence::Certain)
            .with("`definition.pbir` (report definition)");
        let part = analyze_report(c, "");
        d.signal(describe_report(&part));
        d.parts.reports.push(part);
        return Some(d);
    }
    if let Some(item_type) = platform_type {
        let mut d = match item_type.as_str() {
            "SemanticModel" => {
                let mut d = Detection::new(F::PbipSemanticModelFolder, Confidence::Likely);
                let part = analyze_model(c, "");
                d.signal(describe_model(&part));
                d.parts.semantic_models.push(part);
                d
            }
            "Report" => {
                let mut d = Detection::new(F::PbipReportFolder, Confidence::Likely);
                let part = analyze_report(c, "");
                d.signal(describe_report(&part));
                d.parts.reports.push(part);
                d
            }
            "Dataflow" => {
                let certain = root_file("mashup.pq");
                let mut d = Detection::new(
                    F::DataflowFolder,
                    if certain {
                        Confidence::Certain
                    } else {
                        Confidence::Likely
                    },
                );
                if certain {
                    d.signal("`mashup.pq` (Dataflow Gen2 M document)");
                }
                d
            }
            other => Detection::new(
                F::FabricItemFolder {
                    item_type: other.to_owned(),
                },
                Confidence::Certain,
            ),
        };
        d.signals
            .insert(0, format!("`.platform` with metadata.type `{item_type}`"));
        return Some(d);
    }
    if root_file("model.tmdl") || root_file("database.tmdl") {
        let tables = c
            .list_prefix("tables")
            .iter()
            .filter(|e| e.path.ends_with(".tmdl"))
            .count();
        return Some(
            Detection::new(F::TmdlFolder, Confidence::Certain).with(format!(
                "`model.tmdl`/`database.tmdl` at the root ({tables} files under `tables/`)"
            )),
        );
    }
    if c.is_file("definition/model.tmdl") || c.is_file("definition/database.tmdl") {
        let mut d = Detection::new(F::PbipSemanticModelFolder, Confidence::Likely)
            .with("`definition/` TMDL files without `definition.pbism`");
        d.parts.semantic_models.push(analyze_model(c, ""));
        return Some(d);
    }

    let mut d = Detection::new(F::Pbip, Confidence::Likely);
    scan_items(c, &mut d, &[], true);
    if !d.parts.semantic_models.is_empty() || !d.parts.reports.is_empty() {
        d.signals.insert(
            0,
            "no `.pbip` file, but item folders found (Fabric Git / deployment export layout)"
                .into(),
        );
        return Some(d);
    }
    if root_file("mashup.pq") && root_file("queryMetadata.json") {
        return Some(
            Detection::new(F::DataflowFolder, Confidence::Likely)
                .with("`mashup.pq` + `queryMetadata.json` without `.platform`"),
        );
    }
    None
}

/// Finds model and report item folders among the root's children (and the
/// `.pbip` artifact paths) and records them in `d.parts`. With
/// `conventional_only` (no `.pbip` to vouch for the folder), only folders
/// named `<name>.<ItemType>` count, as in Fabric Git and deployment exports.
fn scan_items(c: &dyn Container, d: &mut Detection, artifacts: &[String], conventional_only: bool) {
    let mut candidates: Vec<String> = c
        .list_dir("")
        .into_iter()
        .filter(|ch| ch.is_dir)
        .map(|ch| ch.path)
        .collect();
    for a in artifacts {
        match normalize_lookup(a) {
            Some(p) if !p.is_empty() && c.is_dir(&p) => {
                if !candidates.contains(&p) {
                    candidates.push(p);
                }
            }
            _ => d.signal(format!(
                "`.pbip` artifact `{a}` not found in the project folder"
            )),
        }
    }
    for dir in candidates {
        let name = dir.rsplit('/').next().unwrap_or(&dir);
        let platform = read_platform(c, &join(&dir, ".platform"));
        let ptype = platform.as_ref().and_then(|p| p.item_type.as_deref());
        let named_model = ends_with_ci(name, ".SemanticModel") || ends_with_ci(name, ".Dataset");
        let named_report = ends_with_ci(name, ".Report");
        let is_model = named_model
            || (!conventional_only
                && (c.is_file(&join(&dir, "definition.pbism"))
                    || c.is_file(&join(&dir, "definition.pbidataset"))
                    || ptype == Some("SemanticModel")));
        let is_report = named_report
            || (!conventional_only
                && (c.is_file(&join(&dir, "definition.pbir")) || ptype == Some("Report")));
        if is_model {
            let part = analyze_model(c, &dir);
            d.signal(describe_model(&part));
            d.parts.semantic_models.push(part);
        } else if is_report {
            let part = analyze_report(c, &dir);
            d.signal(describe_report(&part));
            d.parts.reports.push(part);
        } else if let Some(t) = ptype
            && (!conventional_only || ends_with_ci(name, &format!(".{t}")))
        {
            d.signal(format!("Fabric item `{dir}` ({t})"));
            d.parts.other_items.push(FabricItemPart {
                path: dir.clone(),
                item_type: t.to_owned(),
            });
        }
    }
    let models: Vec<String> = d
        .parts
        .semantic_models
        .iter()
        .map(|m| m.path.clone())
        .collect();
    for r in &d.parts.reports {
        if let DatasetReference::ByPath { path, resolved } = &r.dataset {
            match resolved {
                Some(m) if models.contains(m) => {}
                Some(m) => d.signals.push(format!(
                    "report `{}` references model `{m}`, which was not found",
                    r.path
                )),
                None => d.signals.push(format!(
                    "report `{}` references `{path}` outside the project folder",
                    r.path
                )),
            }
        }
    }
}

fn analyze_model(c: &dyn Container, dir: &str) -> SemanticModelPart {
    let tmdl = c
        .list_prefix(&join(dir, "definition"))
        .iter()
        .any(|e| !e.is_dir && e.path.ends_with(".tmdl"));
    let bim = c.is_file(&join(dir, "model.bim"));
    SemanticModelPart {
        path: dir.to_owned(),
        flavour: if tmdl {
            ModelFlavour::Tmdl
        } else if bim {
            ModelFlavour::Bim
        } else {
            ModelFlavour::Unknown
        },
        has_cache_abf: c.is_file(&join(dir, ".pbi/cache.abf")),
        platform: read_platform(c, &join(dir, ".platform")),
    }
}

fn analyze_report(c: &dyn Container, dir: &str) -> ReportPart {
    let pbir =
        c.is_file(&join(dir, "definition/report.json")) || c.is_dir(&join(dir, "definition/pages"));
    let legacy = c.is_file(&join(dir, "report.json"));
    let dataset = read_small_json(c, &join(dir, "definition.pbir"))
        .map(|j| dataset_reference(&j, dir))
        .unwrap_or(DatasetReference::Unknown);
    ReportPart {
        path: dir.to_owned(),
        flavour: if pbir {
            ReportFlavour::Pbir
        } else if legacy {
            ReportFlavour::PbirLegacy
        } else {
            ReportFlavour::Unknown
        },
        dataset,
        platform: read_platform(c, &join(dir, ".platform")),
    }
}

fn display_dir(path: &str) -> &str {
    if path.is_empty() { "." } else { path }
}

fn describe_model(m: &SemanticModelPart) -> String {
    let flavour = match m.flavour {
        ModelFlavour::Tmdl => "TMDL",
        ModelFlavour::Bim => "TMSL model.bim",
        ModelFlavour::Unknown => "no TMDL or model.bim found",
    };
    let cache = if m.has_cache_abf {
        ", has .pbi/cache.abf"
    } else {
        ""
    };
    format!(
        "semantic model `{}` ({flavour}{cache})",
        display_dir(&m.path)
    )
}

fn describe_report(r: &ReportPart) -> String {
    let flavour = match r.flavour {
        ReportFlavour::Pbir => "PBIR",
        ReportFlavour::PbirLegacy => "PBIR-Legacy report.json",
        ReportFlavour::LegacyLayout => "legacy Layout",
        ReportFlavour::Unknown => "no report definition found",
    };
    let dataset = match &r.dataset {
        DatasetReference::ByPath { path, .. } => format!("byPath `{path}`"),
        DatasetReference::ByConnection => {
            "byConnection: remote model, not available offline".into()
        }
        DatasetReference::Unknown => "no dataset reference".into(),
    };
    format!("report `{}` ({flavour}; {dataset})", display_dir(&r.path))
}

fn dataset_reference(json: &JVal, report_dir: &str) -> DatasetReference {
    let Some(reference) = json.get("datasetReference") else {
        return DatasetReference::Unknown;
    };
    if let Some(path) = reference
        .get("byPath")
        .and_then(|p| p.get("path"))
        .and_then(JVal::as_str)
    {
        return DatasetReference::ByPath {
            path: path.to_owned(),
            resolved: resolve_relative(report_dir, path),
        };
    }
    match reference.get("byConnection") {
        Some(JVal::Null) | None => DatasetReference::Unknown,
        Some(_) => DatasetReference::ByConnection,
    }
}

/// Resolves `rel` (may contain `..`) against `base_dir`; `None` if it
/// escapes the root.
fn resolve_relative(base_dir: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = base_dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in rel.split(['/', '\\']) {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

fn pbip_artifacts(json: &JVal) -> Vec<String> {
    let mut out = Vec::new();
    for item in json.get("artifacts").map(JVal::items).unwrap_or_default() {
        if let JVal::Obj(fields) = item {
            for (_, v) in fields {
                if let Some(p) = v.get("path").and_then(JVal::as_str) {
                    out.push(p.to_owned());
                }
            }
        }
    }
    out
}

fn read_small_json(c: &dyn Container, path: &str) -> Result<JVal, String> {
    let entry = c.entry(path).ok_or_else(|| "missing".to_owned())?;
    if entry.is_dir {
        return Err("is a directory".into());
    }
    if entry.size > SMALL_JSON_MAX {
        return Err(format!("{} bytes is too large to inspect", entry.size));
    }
    let bytes = c.read(path).map_err(|e| e.to_string())?;
    let decoded = decode_text(&bytes);
    parse_shallow(decoded.text.trim_start()).map_err(|e| e.to_string())
}

fn read_platform(c: &dyn Container, path: &str) -> Option<PlatformInfo> {
    let json = read_small_json(c, path).ok()?;
    let s = |v: Option<&JVal>| v.and_then(JVal::as_str).map(str::to_owned);
    let metadata = json.get("metadata");
    Some(PlatformInfo {
        item_type: s(metadata.and_then(|m| m.get("type"))),
        display_name: s(metadata.and_then(|m| m.get("displayName"))),
        logical_id: s(json.get("config").and_then(|c| c.get("logicalId"))),
    })
}

// ---------------------------------------------------------------------------
// Single files
// ---------------------------------------------------------------------------

fn is_zip_magic(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06")
}

const XPRESS9_SINGLE: &str = "This backup was created using XPress9 compression.";
const XPRESS9_MULTI: &str = "This backup was created using multithreaded XPrs9.";
const STREAM_STORAGE: &str = "STREAM_STORAGE_SIGNATURE_)!@#$%^&*(";

fn starts_with_utf16le(bytes: &[u8], text: &str) -> bool {
    let mut i = 0usize;
    for unit in text.encode_utf16() {
        let Some(pair) = bytes.get(i..i + 2) else {
            return false;
        };
        if pair != unit.to_le_bytes() {
            return false;
        }
        i += 2;
    }
    true
}

fn sniff_abf(bytes: &[u8]) -> Option<Detection> {
    let compressed = |what: &str| {
        Detection::new(DetectedFormat::Abf { compressed: true }, Confidence::Certain).with(format!(
            "UTF-16LE header \"This backup was created using …\" ({what} XPress9 ABF, as in a PBIX `DataModel`)"
        ))
    };
    if starts_with_utf16le(bytes, XPRESS9_SINGLE) {
        return Some(compressed("single-threaded"));
    }
    if starts_with_utf16le(bytes, XPRESS9_MULTI) {
        return Some(compressed("multi-threaded"));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE])
        && starts_with_utf16le(rest, STREAM_STORAGE)
    {
        return Some(
            Detection::new(
                DetectedFormat::Abf { compressed: false },
                Confidence::Certain,
            )
            .with("`FF FE` + UTF-16LE `STREAM_STORAGE_SIGNATURE_)!@#$%^&*(` (uncompressed ABF)"),
        );
    }
    None
}

/// `true` if the first 4 KiB decode to text with few control characters.
fn head_is_textual(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(4096)];
    let decoded = decode_text(head);
    let mut total = 0usize;
    let mut bad = 0usize;
    for ch in decoded.text.chars() {
        total += 1;
        if ch == '\0' {
            return false;
        }
        if (ch.is_control() && !matches!(ch, '\t' | '\n' | '\r' | '\u{c}')) || ch == '\u{fffd}' {
            bad += 1;
        }
    }
    total > 0 && bad * 20 <= total
}

fn sniff_text(name_hint: Option<&str>, decoded: &DecodedText, truncated: bool) -> Detection {
    let text = decoded.text.trim_start_matches('\u{feff}').trim_start();
    let mut d = if text.starts_with('{') || text.starts_with('[') {
        sniff_json(name_hint, text, truncated)
    } else if text.starts_with('<') {
        sniff_xml(text)
    } else {
        sniff_code(text)
    };
    if decoded.encoding != TextEncoding::Utf8 {
        d.signals
            .insert(0, format!("text encoding {}", decoded.encoding.label()));
    }
    d.parts.text_encoding = Some(decoded.encoding);
    d
}

fn sniff_json(name_hint: Option<&str>, text: &str, truncated: bool) -> Detection {
    match parse_shallow(text) {
        Ok(root @ JVal::Obj(_)) => classify_json(name_hint, &root),
        Ok(_) => Detection::unknown().with("JSON document whose root is not an object"),
        Err(e) => {
            let mut d = heuristic_json(text);
            d.signals.insert(
                0,
                if truncated {
                    format!("JSON read only partially (file larger than the sniff limit): {e}")
                } else {
                    format!("JSON did not parse: {e}")
                },
            );
            d
        }
    }
}

fn classify_schema(url: &str) -> Option<Detection> {
    use DetectedFormat as F;
    let i = url.find("/fabric/")?;
    let segs: Vec<&str> = url[i + "/fabric/".len()..]
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let format = match segs.as_slice() {
        ["item", "report", "definition", kind, ..] => F::PbirJsonFile {
            schema_kind: (*kind).to_owned(),
        },
        ["item", "report", "definitionProperties", ..] => F::PbirJsonFile {
            schema_kind: "definitionProperties".into(),
        },
        ["pbip", ..] => F::Pbip,
        ["item", item, kind, ..] | [item, kind, ..] => F::FabricJsonFile {
            item: (*item).to_owned(),
            schema_kind: (*kind).to_owned(),
        },
        _ => return None,
    };
    Some(Detection::new(format, Confidence::Certain).with(format!("`$schema` {url}")))
}

fn classify_json(name_hint: Option<&str>, root: &JVal) -> Detection {
    use DetectedFormat as F;
    if let Some(mut d) = root
        .get("$schema")
        .and_then(JVal::as_str)
        .and_then(classify_schema)
    {
        if d.format == F::Pbip {
            d.parts.pbip_artifacts = pbip_artifacts(root);
        }
        return d;
    }
    let has = |k: &str| root.get(k).is_some();
    if has("compatibilityLevel") && root.get("model").is_some_and(JVal::is_obj) {
        return Detection::new(F::Bim, Confidence::Certain)
            .with("JSON keys `compatibilityLevel` + `model` (TMSL database)");
    }
    if root
        .get("createOrReplace")
        .and_then(|c| c.get("database"))
        .is_some_and(|db| db.get("model").is_some())
    {
        return Detection::new(F::Bim, Confidence::Likely)
            .with("TMSL `createOrReplace` script with a database model");
    }
    if root.get("connections").is_some_and(|conns| {
        conns
            .items()
            .iter()
            .any(|c| c.get("details").and_then(|d| d.get("protocol")).is_some())
    }) {
        return Detection::new(F::Pbids, Confidence::Certain)
            .with("JSON `connections[].details.protocol` (Power BI data source file)");
    }
    if has("version") && root.get("artifacts").is_some_and(JVal::is_arr) {
        let mut d = Detection::new(F::Pbip, Confidence::Certain)
            .with("JSON keys `version` + `artifacts` (PBIP project file)");
        d.parts.pbip_artifacts = pbip_artifacts(root);
        return d;
    }
    if root.get("entities").is_some_and(JVal::is_arr) {
        return if has("pbi:mashup") {
            Detection::new(F::DataflowModelJson, Confidence::Certain)
                .with("JSON `entities[]` + `pbi:mashup` (dataflow CDM model.json)")
        } else {
            Detection::new(F::DataflowModelJson, Confidence::Likely)
                .with("JSON `entities[]` without `pbi:mashup` (CDM model.json?)")
        };
    }
    if has("version") && root.get("datasetReference").is_some_and(JVal::is_obj) {
        return Detection::new(
            F::PbirJsonFile {
                schema_kind: "definitionProperties".into(),
            },
            Confidence::Certain,
        )
        .with("JSON keys `version` + `datasetReference` (definition.pbir)");
    }
    if root
        .get("metadata")
        .is_some_and(|m| m.get("type").is_some())
        && has("config")
    {
        return Detection::new(
            F::FabricJsonFile {
                item: "gitIntegration".into(),
                schema_kind: "platformProperties".into(),
            },
            Confidence::Likely,
        )
        .with("JSON `metadata.type` + `config` (Fabric `.platform`)");
    }
    if let Some(sections) = root.get("sections").filter(|s| s.is_arr()) {
        let containers = sections
            .items()
            .iter()
            .any(|s| s.get("visualContainers").is_some());
        return Detection::new(
            F::LegacyLayoutJson,
            if containers {
                Confidence::Certain
            } else {
                Confidence::Likely
            },
        )
        .with("JSON `sections[]` (legacy report layout / PBIR-Legacy report.json)");
    }
    if has("version") && has("settings") {
        let pbism = name_hint
            .and_then(extension_of)
            .is_some_and(|e| e == "pbism" || e == "pbidataset");
        return Detection::new(
            F::FabricJsonFile {
                item: "semanticModel".into(),
                schema_kind: "definitionProperties".into(),
            },
            if pbism {
                Confidence::Certain
            } else {
                Confidence::Likely
            },
        )
        .with("JSON keys `version` + `settings` (definition.pbism)");
    }
    let keys: Vec<&str> = match root {
        JVal::Obj(fields) => fields.iter().take(8).map(|(k, _)| k.as_str()).collect(),
        _ => Vec::new(),
    };
    Detection::unknown().with(format!("JSON object with keys: {}", keys.join(", ")))
}

/// Key-presence heuristics for JSON that did not parse (truncated input).
fn heuristic_json(text: &str) -> Detection {
    use DetectedFormat as F;
    let head = head_str(text, 1024 * 1024);
    let has = |k: &str| head.contains(&format!("\"{k}\""));
    let likely = |f: F, why: &str| Detection::new(f, Confidence::Likely).with(why.to_owned());
    if let Some(mut d) = find_schema_url(head).and_then(classify_schema) {
        d.confidence = Confidence::Likely;
        return d;
    }
    if has("compatibilityLevel") && has("model") {
        return likely(F::Bim, "keys `compatibilityLevel` and `model` present");
    }
    if has("connections") && has("protocol") {
        return likely(F::Pbids, "keys `connections` and `protocol` present");
    }
    if has("version") && has("artifacts") {
        return likely(F::Pbip, "keys `version` and `artifacts` present");
    }
    if has("entities") && has("pbi:mashup") {
        return likely(
            F::DataflowModelJson,
            "keys `entities` and `pbi:mashup` present",
        );
    }
    if has("sections") && has("visualContainers") {
        return likely(
            F::LegacyLayoutJson,
            "keys `sections` and `visualContainers` present",
        );
    }
    if has("datasetReference") {
        return likely(
            F::PbirJsonFile {
                schema_kind: "definitionProperties".into(),
            },
            "key `datasetReference` present",
        );
    }
    Detection::unknown()
}

fn find_schema_url(text: &str) -> Option<&str> {
    let i = text.find("\"$schema\"")?;
    let rest = text[i + "\"$schema\"".len()..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}

fn sniff_xml(text: &str) -> Detection {
    let mut rest = head_str(text, 64 * 1024);
    loop {
        rest = rest.trim_start();
        if let Some(r) = rest.strip_prefix("<?") {
            let Some(end) = r.find("?>") else { break };
            rest = &r[end + 2..];
        } else if let Some(r) = rest.strip_prefix("<!--") {
            let Some(end) = r.find("-->") else { break };
            rest = &r[end + 3..];
        } else if let Some(r) = rest.strip_prefix("<!") {
            let Some(end) = r.find('>') else { break };
            rest = &r[end + 1..];
        } else {
            break;
        }
    }
    let Some(tag) = rest.strip_prefix('<') else {
        return Detection::unknown().with("XML-like text without a root element");
    };
    let tag = tag.split('>').next().unwrap_or(tag);
    let name = tag
        .split(|c: char| c.is_whitespace() || c == '/')
        .next()
        .unwrap_or_default();
    let local = name.rsplit(':').next().unwrap_or(name);
    if local == "Report" {
        let lower = tag.to_ascii_lowercase();
        return if lower.contains("reportdefinition") {
            Detection::new(DetectedFormat::Rdl, Confidence::Certain)
                .with("XML root `<Report>` in a `…/reporting/…/reportdefinition` namespace")
        } else {
            Detection::new(DetectedFormat::Rdl, Confidence::Likely)
                .with("XML root `<Report>` without the reportdefinition namespace")
        };
    }
    Detection::unknown().with(format!("XML document with root element `<{name}>`"))
}

const TMDL_KEYWORDS: &[&str] = &[
    "model",
    "database",
    "table",
    "relationship",
    "expression",
    "role",
    "culture",
    "perspective",
    "dataSource",
    "queryGroup",
    "createOrReplace",
];

/// M or TMDL, from the first keyword after comments.
fn sniff_code(text: &str) -> Detection {
    use DetectedFormat as F;
    let rest = skip_comments(text);
    let word_len = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    let (word, after) = rest.split_at(word_len);
    let followed_by_space = after.starts_with([' ', '\t']);
    match word {
        "section" if followed_by_space => {
            let line = after.lines().next().unwrap_or_default();
            if line.trim_end().ends_with(';') {
                Detection::new(F::PowerQueryScript, Confidence::Certain)
                    .with("M section document (`section …;`)")
            } else {
                Detection::new(F::PowerQueryScript, Confidence::Likely)
                    .with("starts with the M keyword `section`")
            }
        }
        "let" if after.starts_with(char::is_whitespace) => {
            Detection::new(F::PowerQueryScript, Confidence::Likely)
                .with("starts with the M keyword `let`")
        }
        "shared" if followed_by_space => Detection::new(F::PowerQueryScript, Confidence::Likely)
            .with("starts with the M keyword `shared`"),
        w if followed_by_space && TMDL_KEYWORDS.contains(&w) => {
            Detection::new(F::TmdlFile, Confidence::Likely)
                .with(format!("starts with the TMDL keyword `{w}`"))
        }
        _ => {
            if text.lines().take(200).any(|l| {
                let l = l.trim();
                l.starts_with("section ") && l.ends_with(';')
            }) {
                Detection::new(F::PowerQueryScript, Confidence::Likely)
                    .with("contains an M `section …;` header")
            } else {
                Detection::unknown().with("text that matches no known format")
            }
        }
    }
}

/// Skips whitespace, `//` line comments (incl. TMDL `///` descriptions)
/// and `/* */` block comments.
fn skip_comments(mut s: &str) -> &str {
    loop {
        s = s.trim_start();
        if let Some(r) = s.strip_prefix("//") {
            s = r.split_once('\n').map_or("", |(_, rest)| rest);
        } else if let Some(r) = s.strip_prefix("/*") {
            s = r.split_once("*/").map_or("", |(_, rest)| rest);
        } else {
            return s;
        }
    }
}

fn ends_with_ci(s: &str, suffix: &str) -> bool {
    s.len() >= suffix.len()
        && s.is_char_boundary(s.len() - suffix.len())
        && s[s.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

fn head_str(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut i = max;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    &s[..i]
}

// ---------------------------------------------------------------------------
// Bounded, shallow JSON scanning
// ---------------------------------------------------------------------------

/// A shallow JSON value: containers deeper than the depth budget, arrays
/// past [`MAX_ITEMS`], objects past [`MAX_KEYS`] and anything past the
/// node budget are skipped (kept as empty containers), and long strings
/// are truncated, so memory stays bounded whatever the input.
#[derive(Debug, Clone, PartialEq)]
enum JVal {
    Null,
    Bool,
    Num,
    Str(String),
    Arr(Vec<JVal>),
    Obj(Vec<(String, JVal)>),
}

impl JVal {
    fn get(&self, key: &str) -> Option<&JVal> {
        match self {
            JVal::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            JVal::Str(s) => Some(s),
            _ => None,
        }
    }

    fn is_obj(&self) -> bool {
        matches!(self, JVal::Obj(_))
    }

    fn is_arr(&self) -> bool {
        matches!(self, JVal::Arr(_))
    }

    fn items(&self) -> &[JVal] {
        match self {
            JVal::Arr(items) => items,
            _ => &[],
        }
    }
}

const MAX_DEPTH: u8 = 6;
const MAX_ITEMS: usize = 32;
const MAX_KEYS: usize = 256;
const MAX_STR: usize = 4096;
const NODE_BUDGET: usize = 20_000;
/// Map key serde_json uses for numbers with its `arbitrary_precision`
/// feature (enabled in this workspace).
const NUMBER_TOKEN: &str = "$serde_json::private::Number";

fn parse_shallow(text: &str) -> Result<JVal, serde_json::Error> {
    let budget = Cell::new(NODE_BUDGET);
    let mut de = serde_json::Deserializer::from_str(text);
    let value = Seed {
        depth: MAX_DEPTH,
        budget: &budget,
    }
    .deserialize(&mut de)?;
    de.end()?;
    Ok(value)
}

#[derive(Clone, Copy)]
struct Seed<'b> {
    depth: u8,
    budget: &'b Cell<usize>,
}

impl Seed<'_> {
    fn take(&self) -> bool {
        let left = self.budget.get();
        if self.depth == 0 || left == 0 {
            return false;
        }
        self.budget.set(left - 1);
        true
    }

    fn child(&self) -> Self {
        Seed {
            depth: self.depth.saturating_sub(1),
            budget: self.budget,
        }
    }
}

impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = JVal;

    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<JVal, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Seed<'_> {
    type Value = JVal;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<JVal, E> {
        Ok(JVal::Bool)
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> Result<JVal, E> {
        Ok(JVal::Num)
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> Result<JVal, E> {
        Ok(JVal::Num)
    }

    fn visit_i128<E: de::Error>(self, _: i128) -> Result<JVal, E> {
        Ok(JVal::Num)
    }

    fn visit_u128<E: de::Error>(self, _: u128) -> Result<JVal, E> {
        Ok(JVal::Num)
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> Result<JVal, E> {
        Ok(JVal::Num)
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<JVal, E> {
        Ok(JVal::Str(head_str(v, MAX_STR).to_owned()))
    }

    fn visit_unit<E: de::Error>(self) -> Result<JVal, E> {
        Ok(JVal::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<JVal, E> {
        Ok(JVal::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<JVal, A::Error> {
        let mut items = Vec::new();
        loop {
            if items.len() < MAX_ITEMS && self.take() {
                match seq.next_element_seed(self.child())? {
                    Some(v) => items.push(v),
                    None => break,
                }
            } else if seq.next_element::<IgnoredAny>()?.is_none() {
                break;
            }
        }
        Ok(JVal::Arr(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<JVal, A::Error> {
        let mut fields = Vec::new();
        let mut first = true;
        loop {
            let keep = fields.len() < MAX_KEYS && self.take();
            if keep || first {
                let Some(key) = map.next_key::<String>()? else {
                    break;
                };
                if first && key == NUMBER_TOKEN {
                    map.next_value::<IgnoredAny>()?;
                    return Ok(JVal::Num);
                }
                first = false;
                if keep {
                    let value = map.next_value_seed(self.child())?;
                    fields.push((key, value));
                } else {
                    map.next_value::<IgnoredAny>()?;
                }
            } else {
                if map.next_key::<IgnoredAny>()?.is_none() {
                    break;
                }
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(JVal::Obj(fields))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shallow_parser_handles_numbers_depth_and_budget() {
        let v = parse_shallow(
            r#"{"a": 1.5, "b": [1, 2, {"c": "x"}], "d": {"e": {"f": {"g": {"h": {"i": {"j": 1}}}}}}}"#,
        )
        .unwrap();
        assert_eq!(v.get("a"), Some(&JVal::Num));
        assert_eq!(v.get("b").unwrap().items().len(), 3);
        assert_eq!(
            v.get("b").unwrap().items()[2]
                .get("c")
                .and_then(JVal::as_str),
            Some("x")
        );
        // Below the depth budget containers are kept but emptied.
        let mut node = v.get("d").unwrap();
        for key in ["e", "f", "g", "h"] {
            node = node.get(key).unwrap();
        }
        assert_eq!(node.get("i"), Some(&JVal::Obj(Vec::new())));

        let big = format!("[{}]", vec!["{\"k\":[1,2,3]}"; 50_000].join(","));
        assert_eq!(parse_shallow(&big).unwrap().items().len(), MAX_ITEMS);
        assert!(parse_shallow("{\"a\": [1, 2").is_err());
        // Deep nesting must neither overflow the stack nor allocate a tree.
        let nested = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
        let _ = parse_shallow(&nested);
        let nested = format!("{}1{}", "{\"a\":".repeat(100_000), "}".repeat(100_000));
        let _ = parse_shallow(&nested);
    }

    #[test]
    fn relative_paths_resolve_within_root() {
        assert_eq!(
            resolve_relative(
                "Acme Outfitters - Stores.Report",
                "../AcmeOutfitters.SemanticModel"
            )
            .as_deref(),
            Some("AcmeOutfitters.SemanticModel")
        );
        assert_eq!(resolve_relative("", "../x"), None);
        assert_eq!(extension_of("a/b.PBIX").as_deref(), Some("pbix"));
        assert_eq!(extension_of(".platform").as_deref(), Some("platform"));
        assert_eq!(extension_of(".gitignore"), None);
    }
}
