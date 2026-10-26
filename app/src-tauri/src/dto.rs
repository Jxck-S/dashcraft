//! IPC data-transfer objects of the app-only views: what the Svelte UI receives besides the
//! registry commands (`ops_execute`, typed by `app/src/lib/ops.gen.ts`).
//!
//! These are UI projections of an open document of the registry workspace: the Data pane tree,
//! the report outline, the Properties and Format panes, the TMDL and Model views. pbi-model
//! types are not sent over IPC directly (their lenient string enums have hand-written serde, so
//! `specta::Type` cannot be derived on them). These DTOs are shaped for the UI instead: enums
//! with an `Unknown(raw)` fallback become plain strings, expressions are pre-formatted
//! (`'Web Sessions'[Device]`), and every object carries its [`ObjectIdDto`] so the UI can ask for
//! [`ObjectDetailsDto`]. Mapping lives in [`crate::mapping`].
//!
//! Every DTO is `camelCase` on the wire. 64-bit integers are avoided (specta forbids them):
//! IDs and counts are `u32`, sizes are exported as TS `number`. Page sizes and positions are
//! `f64` in Rust but exported as a plain TS `number` (`#[specta(type = i32)]`; TS has a single
//! number type) instead of specta's `number | null`: the mapping replaces NaN/±∞, the only
//! values serde_json would turn into `null`, with 0.

use serde::Serialize;
use specta::Type;

/// Document-unique object handle (`pbi_model::ObjectId`). Valid while the document
/// ([`DocumentSummary::doc_id`]) is open.
pub type ObjectIdDto = u32;

/// What kind of filesystem entry was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    /// A regular file (`.pbix`, `.pbit`, `.pbip`, `.bim`, ...).
    File,
    /// A directory (PBIP `*.SemanticModel` / `*.Report` folder, TMDL folder, ...).
    Directory,
}

/// Availability of the semantic model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ModelAvailability {
    /// The model was loaded from the opened files.
    Local,
    /// `byConnection` / live connection: the model lives in the service, not available offline.
    Remote,
    /// No model found (or its schema could not be read).
    Missing,
}

/// [`ModelAvailability`] plus the reader's explanation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatusDto {
    pub availability: ModelAvailability,
    /// E.g. `remote, not available offline`.
    pub description: Option<String>,
}

/// Object counts for the status bar.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DocumentCounts {
    pub tables: u32,
    pub columns: u32,
    pub measures: u32,
    pub relationships: u32,
    /// Named expressions (shared M queries and parameters).
    pub expressions: u32,
    pub parameters: u32,
    pub reports: u32,
    pub pages: u32,
    pub visuals: u32,
}

/// Diagnostics by severity.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticCounts {
    pub errors: u32,
    pub warnings: u32,
    pub infos: u32,
}

/// What was opened, at a glance (part of [`DocumentViewsDto`]).
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSummary {
    /// The registry document id (`d1`, `d2`, …; from `doc.open`): every registry command on
    /// this document takes it, and the UI drops responses for another one.
    pub doc_id: String,
    /// Absolute path of the opened entry.
    pub path: String,
    /// Last path component, e.g. `AcmeOutfitters.pbip`.
    pub file_name: String,
    pub entry_kind: EntryKind,
    /// Size in bytes for files, `null` for directories.
    /// Exported as a TS `number` (exact up to 2^53 bytes, far beyond any real file).
    #[specta(type = Option<f64>)]
    pub size_bytes: Option<u64>,
    /// Document format (`pbip`, `pbix`, `pbit`, `bim`, `reportFolder`, ...).
    pub format: String,
    /// Human-readable detection result, e.g. `PBIP project`.
    pub format_label: String,
    pub model_status: ModelStatusDto,
    /// Semantic model name (usually `Model`), if a model was loaded.
    pub model_name: Option<String>,
    pub counts: DocumentCounts,
    pub diagnostics: DiagnosticCounts,
    /// The PBIP local data cache (`<Model>.SemanticModel/.pbi/cache.abf`), if there is one.
    pub cache: Option<CacheInfoDto>,
    /// What the UI loads right after opening, as Power BI Desktop does.
    pub on_open: OpenLoad,
}

/// What the UI loads when it opens a document, as Power BI Desktop does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum OpenLoad {
    /// Nothing: no local model, or no saved data (a PBIP without `.pbi/cache.abf`); Home →
    /// Refresh loads it.
    None,
    /// The data saved in the file (PBIX, ABF, Power Pivot): `data.refresh` with source `cache`.
    Embedded,
    /// The PBIP local data cache (`cache`): `data.refresh` with source `cache`.
    Cache,
    /// A template (PBIT) carries no data: `data.refresh` with source `query`, using the
    /// parameter values saved in the template.
    Query,
}

/// The local data cache of a PBIP: Power BI Desktop's last refreshed data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CacheInfoDto {
    /// Entry in the opened folder, e.g. `Sales.SemanticModel/.pbi/cache.abf`.
    pub entry: String,
    /// Size in bytes (exported as a TS `number`).
    #[specta(type = f64)]
    pub size_bytes: u64,
    /// Last modification time of the file (RFC 3339, UTC): when Power BI Desktop last refreshed.
    pub modified: Option<String>,
}

// ---------------------------------------------------------------------------
// Model tree
// ---------------------------------------------------------------------------

/// The semantic model for the Data pane, Model and Queries views.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelTreeDto {
    /// Model object ID, if a model was loaded.
    pub id: Option<ObjectIdDto>,
    pub name: Option<String>,
    pub status: ModelStatusDto,
    pub description: Option<String>,
    pub culture: Option<String>,
    pub compatibility_level: Option<u32>,
    /// `tmdl`, `tmsl` or `abf`: how the model was stored.
    pub source_kind: Option<String>,
    /// Folder of the semantic model in the opened container.
    pub source_folder: Option<String>,
    pub tables: Vec<TableDto>,
    pub relationships: Vec<RelationshipDto>,
    /// Named expressions: shared M queries and parameters (`isParameter`).
    pub expressions: Vec<ExpressionDto>,
}

/// How a table gets its rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TableKindDto {
    /// M (Power Query) or native query.
    Regular,
    /// DAX calculated table.
    CalculatedTable,
    CalculationGroup,
    /// Entity / DirectQuery / Direct Lake partitions: data not available offline.
    Remote,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TableDto {
    pub id: ObjectIdDto,
    pub name: String,
    pub description: Option<String>,
    pub hidden: bool,
    pub kind: TableKindDto,
    pub data_category: Option<String>,
    /// DAX of a calculated table.
    pub expression: Option<String>,
    pub columns: Vec<ColumnDto>,
    pub measures: Vec<MeasureDto>,
    pub partitions: Vec<PartitionDto>,
    pub hierarchies: Vec<HierarchyDto>,
}

/// How a column gets its values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ColumnKindDto {
    /// Loaded from the partition's source column.
    Data,
    /// DAX calculated column.
    Calculated,
    /// Column of a calculated table.
    CalculatedTableColumn,
    /// Internal row-number column.
    RowNumber,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ColumnDto {
    pub id: ObjectIdDto,
    /// Display name.
    pub name: String,
    /// TMDL spelling: `string`, `int64`, `double`, `decimal`, `dateTime`, `boolean`, ...
    pub data_type: String,
    pub kind: ColumnKindDto,
    pub hidden: bool,
    pub is_key: bool,
    pub description: Option<String>,
    /// Name in the source query (shown as a tooltip when it differs from `name`).
    pub source_column: Option<String>,
    /// DAX of a calculated column.
    pub expression: Option<String>,
    pub format_string: Option<String>,
    pub display_folder: Option<String>,
    pub sort_by_column: Option<String>,
    pub summarize_by: Option<String>,
    pub data_category: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MeasureDto {
    pub id: ObjectIdDto,
    pub name: String,
    /// DAX text, verbatim.
    pub expression: String,
    pub format_string: Option<String>,
    pub display_folder: Option<String>,
    pub description: Option<String>,
    pub hidden: bool,
}

/// Partition source type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PartitionKind {
    /// Power Query M.
    M,
    /// DAX calculated table.
    Calculated,
    /// Entity of a remote model / Direct Lake.
    Entity,
    /// Legacy provider / native query.
    Query,
    /// Anything else (`calculationGroup`, `policyRange`, ...); see `sourceType`.
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PartitionDto {
    pub id: ObjectIdDto,
    pub name: String,
    pub kind: PartitionKind,
    /// TMDL source keyword (`m`, `calculated`, `entity`, ...).
    pub source_type: String,
    /// Storage mode: `import`, `directQuery`, `dual`, `default`, `directLake`.
    pub mode: String,
    /// M, DAX or native query text.
    pub expression: Option<String>,
    pub query_group: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HierarchyDto {
    pub id: ObjectIdDto,
    pub name: String,
    pub hidden: bool,
    /// Level names, top to bottom.
    pub levels: Vec<String>,
}

/// Relationship cardinality, `from` side first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RelationshipCardinality {
    ManyToOne,
    OneToMany,
    OneToOne,
    ManyToMany,
    Unknown,
}

/// Cross-filter direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CrossFilterDirection {
    /// The one side filters the many side.
    Single,
    /// Both directions.
    Both,
    Automatic,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RelationshipDto {
    pub id: ObjectIdDto,
    /// Usually a GUID.
    pub name: String,
    pub from_table: String,
    pub from_column: String,
    pub to_table: String,
    pub to_column: String,
    pub cardinality: RelationshipCardinality,
    pub cross_filter: CrossFilterDirection,
    pub active: bool,
}

/// Language of an expression or code block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CodeLanguage {
    Dax,
    M,
    Json,
    Text,
}

/// A model-level named expression: a shared M query or a parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExpressionDto {
    pub id: ObjectIdDto,
    pub name: String,
    pub language: CodeLanguage,
    /// Expression text including a trailing `meta [...]` record, as in the source.
    pub expression: String,
    /// Power Query parameter (`IsParameterQuery=true`).
    pub is_parameter: bool,
    /// Parameter type from the metadata record (`Text`, `Number`, ...).
    pub parameter_type: Option<String>,
    pub query_group: Option<String>,
    pub description: Option<String>,
}

// ---------------------------------------------------------------------------
// Report outline
// ---------------------------------------------------------------------------

/// Where a report's semantic model lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum DatasetKind {
    /// Local model folder (`definition.pbir` `byPath`).
    ByPath,
    /// Service-hosted model: remote, not available offline.
    ByConnection,
    /// Model embedded in the same file (PBIX, PBIT).
    Embedded,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DatasetReferenceDto {
    pub kind: DatasetKind,
    /// `byPath` path, or the connection's model / database name.
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReportDto {
    pub id: ObjectIdDto,
    pub name: String,
    pub dataset: DatasetReferenceDto,
    /// Internal name of the page shown on open.
    pub start_page: Option<String>,
    /// Report-level filters.
    pub filters: Vec<FilterDto>,
    pub pages: Vec<PageDto>,
    /// `report.json` (or `Layout`) path in the opened container.
    pub origin: Option<String>,
    /// The report theme (Design ribbon → Themes gallery, read-only).
    pub theme: ReportThemeDto,
}

/// Base and custom theme of a report, with the colors of the effective theme file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReportThemeDto {
    /// Base theme resource name, e.g. `CY24SU10`.
    pub base_theme: Option<String>,
    /// Custom theme resource name, if one is applied.
    pub custom_theme: Option<String>,
    /// `name` of the effective theme JSON (custom theme first, else the base theme).
    pub name: Option<String>,
    /// `dataColors` of the effective theme (`#RRGGBB`, at most 16).
    pub data_colors: Vec<String>,
    /// `background` / `foreground` / `tableAccent` of the effective theme, if set.
    pub background: Option<String>,
    pub foreground: Option<String>,
    pub table_accent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PageDto {
    pub id: ObjectIdDto,
    /// Internal name (PBIR folder name).
    pub name: String,
    pub display_name: String,
    #[specta(type = i32)]
    pub width: f64,
    #[specta(type = i32)]
    pub height: f64,
    pub hidden: bool,
    /// `FitToPage`, `FitToWidth`, `ActualSize`.
    pub display_option: String,
    /// Page-level filters.
    pub filters: Vec<FilterDto>,
    /// Visuals in source order.
    pub visuals: Vec<VisualDto>,
}

/// Visual container position in page units.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PositionDto {
    #[specta(type = i32)]
    pub x: f64,
    #[specta(type = i32)]
    pub y: f64,
    #[specta(type = i32)]
    pub z: f64,
    #[specta(type = i32)]
    pub width: f64,
    #[specta(type = i32)]
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VisualDto {
    pub id: ObjectIdDto,
    /// Internal name (PBIR folder name).
    pub name: String,
    /// E.g. `cardVisual`, `slicer`, `tableEx`; `group` for visual groups.
    pub visual_type: String,
    pub is_group: bool,
    /// Title text, group name or textbox text (best effort), for labels.
    pub title: Option<String>,
    pub position: PositionDto,
    pub hidden: bool,
    pub parent_group: Option<String>,
    /// Projections (data roles → fields).
    pub fields: Vec<FieldDto>,
    /// Visual-level filters.
    pub filters: Vec<FilterDto>,
}

/// What a projected field refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum FieldKindDto {
    Column,
    Measure,
    /// Implicit aggregation of a column (`Sum(T[C])`).
    Aggregation,
    HierarchyLevel,
    Literal,
    Unknown,
}

/// A field bound to a visual role.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FieldDto {
    /// Data role, e.g. `Values`, `Category`, `Y`.
    pub role: String,
    pub kind: FieldKindDto,
    /// Label shown in the UI: the projection's `displayName` if renamed, else the field name.
    pub display_name: String,
    /// Model reference, e.g. `'Web Sessions'[Device]` or `Sum(Orders[Quantity])`; tooltip.
    pub source_name: String,
    /// The projection was renamed (`displayName` differs from the field name).
    pub renamed: bool,
    pub table: Option<String>,
    /// Column / measure / level name in the model.
    pub field: Option<String>,
    /// `sum`, `avg`, `distinctCount`, ... for implicit aggregations.
    pub aggregation: Option<String>,
    pub query_ref: String,
}

/// Where a filter applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum FilterScope {
    Report,
    Page,
    Visual,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FilterDto {
    pub id: ObjectIdDto,
    pub name: String,
    pub scope: FilterScope,
    /// User-facing filter card title, if renamed.
    pub display_name: Option<String>,
    /// Filtered field, e.g. `Stores[Channel]`.
    pub field: Option<String>,
    /// `Categorical`, `Advanced`, `TopN`, `RelativeDate`, ...
    pub filter_type: String,
    /// DAX-like condition, e.g. `Stores[Channel] IN {"Retail"}`.
    pub summary: String,
    pub hidden: bool,
    pub locked: bool,
}

// ---------------------------------------------------------------------------
// Diagnostics, detection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SeverityDto {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticDto {
    pub severity: SeverityDto,
    /// Stable code, e.g. `CORE005`, `TMDL012`.
    pub code: String,
    pub message: String,
    /// Source location, e.g. `X.SemanticModel/definition/tables/A.tmdl:12`.
    pub origin: Option<String>,
    /// Object the message is about (for navigation).
    pub object_id: Option<ObjectIdDto>,
}

/// How sure the format detection is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ConfidenceDto {
    NoSignal,
    ExtensionOnly,
    Likely,
    Certain,
}

/// A semantic model or report folder found during detection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DetectedItemDto {
    /// Folder path in the opened container (`""` = the opened folder itself).
    pub path: String,
    /// `tmdl`, `bim`, `pbir`, `pbirLegacy`, `legacyLayout`, `unknown`.
    pub flavour: String,
    /// E.g. `byPath ../X.SemanticModel`, `byConnection`, `cache.abf`.
    pub detail: Option<String>,
}

/// A container part listed for the explorer (metadata, theme, image, `DataModel`, ...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PartDto {
    pub path: String,
    pub kind: String,
    #[specta(type = Option<f64>)]
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DetectionDto {
    /// Detected format kind, e.g. `pbip`, `pbix`, `pbipReportFolder`.
    pub format: String,
    /// Human-readable label, e.g. `PBIP project`.
    pub label: String,
    pub confidence: ConfidenceDto,
    /// Why, in the order the signals were found.
    pub signals: Vec<String>,
    /// The `.pbip` project file, if any.
    pub pbip_file: Option<String>,
    pub semantic_models: Vec<DetectedItemDto>,
    pub reports: Vec<DetectedItemDto>,
    /// Number of entries in the opened folder / archive.
    pub entry_count: u32,
    /// Metadata and uninterpreted parts.
    pub parts: Vec<PartDto>,
}

// ---------------------------------------------------------------------------
// Object details (Properties pane)
// ---------------------------------------------------------------------------

/// Kind of a model or report object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ObjectKindDto {
    Model,
    Database,
    Table,
    Column,
    Measure,
    Partition,
    Hierarchy,
    Level,
    Relationship,
    Expression,
    QueryGroup,
    Role,
    TablePermission,
    Culture,
    Perspective,
    Report,
    Page,
    Visual,
    Filter,
}

/// One row of the property sheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PropertyDto {
    pub label: String,
    pub value: String,
    /// Extra information (e.g. the source name of a renamed field).
    pub tooltip: Option<String>,
    /// Render in a monospace font (names, refs, format strings).
    pub mono: bool,
}

/// A read-only code block (DAX, M, JSON).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CodeBlockDto {
    pub label: String,
    pub language: CodeLanguage,
    pub text: String,
}

/// Everything the Properties pane shows about one object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ObjectDetailsDto {
    pub id: ObjectIdDto,
    pub kind: ObjectKindDto,
    /// Display name.
    pub name: String,
    /// Containing object, e.g. the table of a column or `Report › Page` of a visual.
    pub parent: Option<String>,
    pub description: Option<String>,
    pub properties: Vec<PropertyDto>,
    /// Full DAX / M / JSON text.
    pub code: Vec<CodeBlockDto>,
    /// Annotations (`name = value`), verbatim.
    pub annotations: Vec<PropertyDto>,
    /// Properties DashCraft does not model yet (kept for round-tripping), as JSON text.
    pub extra: Vec<PropertyDto>,
    /// Source file (and line or JSON pointer) in the opened container.
    pub origin: Option<String>,
}

// ---------------------------------------------------------------------------
// App settings: blob map
// ---------------------------------------------------------------------------

/// The offline blob map used by `AzureStorage.Blobs` sources (local files only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BlobMapDto {
    /// Absolute path of the active `blob-map.json`; `null` when offline.
    pub path: Option<String>,
    /// The map was named by the environment variable `DASHCRAFT_BLOB_MAP`.
    pub from_environment: bool,
    /// The environment variable that named the map; `null` when it was chosen in the app.
    pub environment_variable: Option<String>,
    /// Why the map named by the environment could not be used.
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Desktop views: TMDL view, Model view diagram, Format pane, recent files
// ---------------------------------------------------------------------------

/// One TMDL document of the TMDL view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TmdlFileDto {
    /// Path relative to the `definition/` folder, e.g. `tables/Orders.tmdl`.
    pub path: String,
    pub text: String,
    /// Model table described by this file, if any (to open it from the Data pane).
    pub table: Option<String>,
}

/// Where the TMDL view's text comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TmdlSourceDto {
    /// The opened `.tmdl` files, verbatim.
    Files,
    /// Generated from the model (TMSL, PBIX / ABF): canonical TMDL as Power BI Desktop writes it.
    Generated,
}

/// The model as TMDL scripts (TMDL view, read-only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TmdlViewDto {
    pub source: TmdlSourceDto,
    pub files: Vec<TmdlFileDto>,
}

/// A table card position in a Model view layout (`diagramLayout.json` units = pixels at 100%).
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiagramNodeDto {
    /// Table name (`nodeIndex`).
    pub table: String,
    #[specta(type = i32)]
    pub x: f64,
    #[specta(type = i32)]
    pub y: f64,
    #[specta(type = i32)]
    pub width: f64,
    #[specta(type = i32)]
    pub height: f64,
    /// The card is collapsed to its header.
    pub collapsed: bool,
}

/// One Model view layout tab (`All tables`, custom layouts).
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiagramDto {
    pub name: String,
    pub nodes: Vec<DiagramNodeDto>,
    /// Zoom in percent (100 = 100%).
    #[specta(type = i32)]
    pub zoom: f64,
}

/// `diagramLayout.json` of the semantic model, if the document has one.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelDiagramDto {
    pub diagrams: Vec<DiagramDto>,
    /// Name of the layout shown first.
    pub default_diagram: Option<String>,
}

/// One instance of a formatting object (`objects.title[0]`), with its selector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FormatEntryDto {
    /// What the instance applies to (`series: Sum(Sales[Amount])`, `id: default`), if scoped.
    pub selector: Option<String>,
    /// Property name (display label in `label`, raw name in `tooltip`) → readable value.
    pub properties: Vec<PropertyDto>,
}

/// A Format pane card: one formatting object (`title`, `labels`, `categoryAxis`, …).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FormatCardDto {
    /// Raw object name, e.g. `categoryAxis`.
    pub name: String,
    /// Display label, e.g. `Category axis`.
    pub label: String,
    pub entries: Vec<FormatEntryDto>,
}

/// The Format pane of a visual (read-only): `Visual` cards from `visual.objects`, `General`
/// cards from `visual.visualContainerObjects`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VisualFormatDto {
    pub visual: Vec<FormatCardDto>,
    pub general: Vec<FormatCardDto>,
    /// Properties of the container that are not formatting objects (position, size, layer).
    pub layout: Vec<PropertyDto>,
}

/// A recently opened file or folder (File → Recent).
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RecentFileDto {
    /// Absolute path.
    pub path: String,
    /// Last path component.
    pub file_name: String,
    /// When it was last opened, in milliseconds since the Unix epoch (a plain TS `number`).
    #[specta(type = i32)]
    pub opened_at_ms: f64,
    /// The path still exists.
    pub exists: bool,
}

/// Everything the panes need about an open document (`document_views`).
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DocumentViewsDto {
    pub summary: DocumentSummary,
    pub tree: ModelTreeDto,
    pub reports: Vec<ReportDto>,
    pub diagnostics: Vec<DiagnosticDto>,
    pub detection: DetectionDto,
}

/// A command of the registry catalog (`ops_catalog`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OpsCommandDto {
    /// Stable dotted id, e.g. `doc.open`.
    pub id: String,
    pub title: String,
    pub summary: String,
    /// `workspace`, `data`, `canvas`, …
    pub category: String,
    pub mutates_document: bool,
    pub long_running: bool,
    pub agent_safe: bool,
    pub writes_files: bool,
}
