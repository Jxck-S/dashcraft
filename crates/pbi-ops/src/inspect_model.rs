//! Stable JSON schema of `dashcraft inspect --json` and the `doc.inspect`
//! command (`schemaVersion: 1`).
//!
//! These types are a public contract for CI, scripts and agents (moved here
//! from `dashcraft-cli` so the CLI and the command registry share them; the
//! `JsonSchema` derives feed the command catalog). They are built from the
//! internal `pbi_core`/`pbi_model` types, which are free to change, and are
//! never those types serialized directly.
//!
//! Compatibility rules for `schemaVersion` 1:
//! - fields are only ever **added**; existing fields keep their name, type
//!   and meaning (a breaking change bumps `schemaVersion`);
//! - arrays and booleans are always present (arrays may be empty);
//! - optional strings/numbers (marked `?` in the README) are omitted when
//!   absent; `model` is `null` when no model is available locally;
//! - enumerations use the canonical TMDL/TMSL/PBIR spellings (camelCase),
//!   and keep unknown source values verbatim;
//! - paths are as given on the command line (`source.path`) or relative to
//!   the opened folder/archive (origins, folders), always `/`-separated.
//!
//! Format-specific sections (added in 0.2, all optional and omitted when
//! absent): `vpaxStats` (`.vpax` size/cardinality statistics),
//! `paginatedReport` (`.rdl` outline: data sources with connection strings
//! redacted, datasets, parameters, report items), `mashup` (`.pq`, `.pqt`,
//! dataflows: query metadata, entities, connections), `customVisual`
//! (`.pbiviz` metadata) and `connector` (`.mez`/`.pqx` listing); and
//! `model.cacheAbf` (PBIP `.pbi/cache.abf` schema summary).

use pbi_core::OpenedDocument;
use pbi_core::detect::{Confidence, DetectedFormat, Detection};
use pbi_core::reader::outline as ol;
use pbi_model::{
    Column, ColumnKind, Condition, DatasetReference, Diagnostic, FieldExpr, Filter, Literal,
    LiteralValue, Model, ModelStatus, Page, PartitionSource, Projection, Report, Severity, Table,
    TableKind, Visual, dax_column_ref, dax_table_ref,
};
use schemars::JsonSchema;
use serde::Serialize;

/// Current schema version.
pub const SCHEMA_VERSION: u32 = 1;

/// What to include in the output.
#[derive(Debug, Clone, Copy)]
pub struct BuildOptions {
    /// Include DAX, M and query text (measures, calculated columns and
    /// tables, partitions, named expressions, role filters).
    pub expressions: bool,
}

/// Root object of `inspect --json`.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Inspect {
    pub schema_version: u32,
    pub source: SourceOut,
    pub model_status: ModelStatusOut,
    /// The document's semantic model; `null` if none is available locally.
    pub model: Option<ModelOut>,
    /// Other semantic models of a multi-model project.
    pub extra_models: Vec<ModelOut>,
    pub reports: Vec<ReportOut>,
    pub diagnostics: Vec<DiagnosticOut>,
    pub counts: Counts,
    /// VertiPaq Analyzer statistics (`.vpax`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpax_stats: Option<VpaxStatsOut>,
    /// Paginated report outline (`.rdl`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paginated_report: Option<PaginatedReportOut>,
    /// Power Query details (`.pq`, `.pqt`, dataflows).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mashup: Option<MashupOut>,
    /// Custom visual metadata (`.pbiviz`; never executed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_visual: Option<CustomVisualOut>,
    /// Custom connector listing (`.mez`/`.pqx`; never executed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connector: Option<ConnectorOut>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceOut {
    /// The path as given on the command line.
    pub path: String,
    /// Document format: `pbip`, `pbix`, `pbit`, `semanticModelFolder`,
    /// `reportFolder`, `bim`, `tmdlFolder`, …
    pub format: String,
    pub detection: DetectionOut,
    /// PBIX/PBIT `Version`, e.g. `1.32`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DetectionOut {
    /// Detected format kind: `pbip`, `pbipSemanticModelFolder`,
    /// `pbipReportFolder`, `pbix`, `pbit`, `bim`, `tmdlFolder`, …,
    /// `unknown`.
    pub format: String,
    /// Human-readable label, e.g. `PBIP project`.
    pub label: String,
    /// `noSignal`, `extensionOnly`, `likely` or `certain`.
    pub confidence: String,
    /// Why the format was recognised, in detection order.
    pub signals: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatusOut {
    /// `local`, `remote` (e.g. `byConnection`) or `missing`.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelOut {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub culture: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility_level: Option<i64>,
    /// Where the model definition was read from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<ModelSourceOut>,
    pub tables: Vec<TableOut>,
    pub relationships: Vec<RelationshipOut>,
    /// Named expressions: M queries and parameters (`isParameter`).
    pub expressions: Vec<ExpressionOut>,
    pub roles: Vec<RoleOut>,
    /// PBIP `.pbi/cache.abf` of this model folder (schema summary).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_abf: Option<CacheAbfOut>,
}

/// Schema summary of a PBIP local data cache (`.pbi/cache.abf`).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CacheAbfOut {
    /// Path relative to the opened folder.
    pub path: String,
    pub size: u64,
    /// Visible tables in the cache.
    pub table_count: u64,
    pub tables: Vec<String>,
    /// `true`/`false` when compared with the model definition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matches_definition: Option<bool>,
    /// `sqlite` or `xml`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility_level: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelSourceOut {
    /// `tmdl`, `tmsl` or `abf`.
    pub kind: String,
    /// Model folder relative to the opened folder/archive (`""` = root).
    pub folder: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TableOut {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub is_hidden: bool,
    /// `regular`, `calculatedTable`, `calculationGroup` or `remote`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_category: Option<String>,
    pub columns: Vec<ColumnOut>,
    pub measures: Vec<MeasureOut>,
    pub partitions: Vec<PartitionOut>,
    pub hierarchies: Vec<HierarchyOut>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ColumnOut {
    pub name: String,
    /// `string`, `int64`, `double`, `decimal`, `dateTime`, `boolean`, …
    pub data_type: String,
    /// `data`, `calculated`, `calculatedTableColumn` or `rowNumber`.
    pub kind: String,
    pub is_hidden: bool,
    pub is_key: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_column: Option<String>,
    /// DAX of a calculated column (omitted with `--no-expressions`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_string: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summarize_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_by_column: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_folder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_category: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeasureOut {
    pub name: String,
    /// DAX text, verbatim (omitted with `--no-expressions`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_string: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub is_hidden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_folder: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PartitionOut {
    pub name: String,
    /// Source kind: `m`, `calculated`, `entity`, `query`, or another TMDL
    /// source keyword.
    pub kind: String,
    /// `import`, `directQuery`, `dual`, `directLake` or `default`.
    pub mode: String,
    /// M, DAX or native query text (omitted with `--no-expressions`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_group: Option<String>,
    /// Entity partitions: the remote entity name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_name: Option<String>,
    /// Entity partitions: the named expression holding the source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression_source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HierarchyOut {
    pub name: String,
    pub is_hidden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub levels: Vec<LevelOut>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LevelOut {
    pub name: String,
    pub column: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelationshipOut {
    pub name: String,
    /// The many side (TOM `fromColumn`).
    pub from: ColumnRefOut,
    /// The one side (TOM `toColumn`).
    pub to: ColumnRefOut,
    /// `many` or `one`.
    pub from_cardinality: String,
    /// `many` or `one`.
    pub to_cardinality: String,
    /// `oneDirection`, `bothDirections` or `automatic`.
    pub cross_filtering_behavior: String,
    pub is_active: bool,
    /// `oneDirection`, `bothDirections` or `none`.
    pub security_filtering_behavior: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ColumnRefOut {
    pub table: String,
    pub column: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExpressionOut {
    pub name: String,
    /// `m` or `dax`.
    pub kind: String,
    /// Power Query parameter (`IsParameterQuery=true`).
    pub is_parameter: bool,
    /// Parameter type from the metadata record (`Text`, `Number`, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_group: Option<String>,
    /// Expression text without the `meta [...]` record; for parameters the
    /// value (omitted with `--no-expressions`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoleOut {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_permission: Option<String>,
    pub table_permissions: Vec<TablePermissionOut>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TablePermissionOut {
    pub table: String,
    /// DAX row filter (omitted with `--no-expressions`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_expression: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportOut {
    pub name: String,
    pub dataset_reference: DatasetReferenceOut,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_theme: Option<String>,
    /// Internal name of the page shown on open.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_page: Option<String>,
    /// Report-level filters (apply to every page).
    pub filters: Vec<FilterOut>,
    /// Pages in display order.
    pub pages: Vec<PageOut>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DatasetReferenceOut {
    /// `byPath`, `byConnection` (remote), `embedded` (PBIX/PBIT) or `none`.
    pub kind: String,
    /// `byPath`: model folder relative to the report folder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// `byConnection`: connection string (secrets masked).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_string: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PageOut {
    /// Internal name (PBIR folder name / ID).
    pub name: String,
    pub display_name: String,
    pub width: f64,
    pub height: f64,
    /// `FitToPage`, `FitToWidth` or `ActualSize`.
    pub display_option: String,
    pub is_hidden: bool,
    pub filters: Vec<FilterOut>,
    pub visuals: Vec<VisualOut>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VisualOut {
    /// Internal name (PBIR folder name / ID).
    pub name: String,
    /// E.g. `cardVisual`, `slicer`, `tableEx`; `group` for visual groups.
    pub visual_type: String,
    pub is_group: bool,
    /// Group display name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub is_hidden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_group_name: Option<String>,
    pub position: PositionOut,
    /// Bound fields (`queryState` projections), in role order.
    pub fields: Vec<FieldOut>,
    /// Visual-level filters.
    pub filters: Vec<FilterOut>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PositionOut {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub width: f64,
    pub height: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_order: Option<i64>,
}

/// A field reference (column, measure, aggregation, hierarchy level, …).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FieldRefOut {
    /// `column`, `measure`, `aggregation`, `hierarchyLevel`, `literal` or
    /// `unknown`.
    pub kind: String,
    /// Table (for aggregations, of the aggregated field).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    /// Column, measure or hierarchy-level name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `aggregation`: `sum`, `avg`, `distinctCount`, `min`, `max`,
    /// `countNonNull`, … (or the PBIR code as text).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggregation: Option<String>,
    /// `hierarchyLevel`: the hierarchy name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hierarchy: Option<String>,
    /// DAX-style reference, e.g. `'Web Sessions'[Device]`,
    /// `Sum(Orders[Quantity])`.
    #[serde(rename = "ref")]
    pub reference: String,
}

/// A field bound to a visual data role.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FieldOut {
    /// Data role, e.g. `Values`, `Category`, `Y`.
    pub role: String,
    #[serde(flatten)]
    pub field: FieldRefOut,
    pub query_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_query_ref: Option<String>,
    /// User-renamed label shown instead of the field name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FilterOut {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Filter type: `Categorical`, `Advanced`, `TopN`, `RelativeDate`, …
    pub kind: String,
    /// The field shown in the filter pane.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<FieldRefOut>,
    /// Condition summary, e.g. `Stores[Channel] IN ('Retail')`;
    /// omitted when the filter card is not filtering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    pub is_hidden_in_view_mode: bool,
    pub is_locked_in_view_mode: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub how_created: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticOut {
    /// `error`, `warning` or `info`.
    pub severity: String,
    /// Stable code, e.g. `CORE004`, `TMDL001`.
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<OriginOut>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OriginOut {
    /// File relative to the opened folder/archive.
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// JSON pointer inside the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
}

/// Totals over `model`, `extraModels` and `reports`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub tables: usize,
    pub columns: usize,
    pub calculated_columns: usize,
    pub calculated_tables: usize,
    pub measures: usize,
    pub relationships: usize,
    pub expressions: usize,
    pub parameters: usize,
    pub reports: usize,
    pub pages: usize,
    pub visuals: usize,
    /// Report-, page- and visual-level filters.
    pub filters: usize,
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
}

// ---------------------------------------------------------------------------
// Format-specific sections
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VpaxStatsOut {
    /// `DaxModel.json` or `DaxVpaView.json`.
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility_level: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extractor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extraction_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_data_refresh: Option<String>,
    /// Bytes.
    pub total_size: u64,
    pub tables: Vec<VpaxTableOut>,
    pub relationships: Vec<VpaxRelationshipOut>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VpaxTableOut {
    pub name: String,
    pub rows_count: u64,
    pub columns_size: u64,
    pub user_hierarchies_size: u64,
    pub relationships_size: u64,
    pub table_size: u64,
    pub is_hidden: bool,
    pub columns: Vec<VpaxColumnOut>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VpaxColumnOut {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_type: Option<String>,
    /// `HASH` or `VALUE`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    pub cardinality: u64,
    pub data_size: u64,
    pub dictionary_size: u64,
    pub hierarchies_size: u64,
    pub total_size: u64,
    pub segments: u64,
    pub is_hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VpaxRelationshipOut {
    pub from: ColumnRefOut,
    pub to: ColumnRefOut,
    pub used_size: u64,
    pub missing_keys: u64,
    pub invalid_rows: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PaginatedReportOut {
    /// RDL schema, e.g. `2016/01`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    pub data_sources: Vec<RdlDataSourceOut>,
    pub datasets: Vec<RdlDataSetOut>,
    pub parameters: Vec<RdlParameterOut>,
    pub sections: Vec<RdlSectionOut>,
    pub embedded_images: Vec<String>,
    pub item_count: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RdlDataSourceOut {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_provider: Option<String>,
    /// Always `<redacted>` when the source embeds one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_string: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrated_security: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RdlDataSetOut {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_type: Option<String>,
    /// Query text (omitted with `--no-expressions`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_reference: Option<String>,
    pub fields: Vec<RdlFieldOut>,
    pub query_parameters: Vec<RdlQueryParameterOut>,
    pub filters: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RdlFieldOut {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_field: Option<String>,
    /// Calculated field expression (omitted with `--no-expressions`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RdlQueryParameterOut {
    pub name: String,
    /// Omitted with `--no-expressions`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RdlParameterOut {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    pub nullable: bool,
    pub allow_blank: bool,
    pub multi_value: bool,
    pub hidden: bool,
    pub default_values: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_dataset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_values_dataset: Option<String>,
    pub valid_values: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RdlSectionOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_width: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_height: Option<String>,
    pub body: Vec<RdlItemOut>,
    pub header: Vec<RdlItemOut>,
    pub footer: Vec<RdlItemOut>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RdlItemOut {
    /// `tablix`, `textbox`, `chart`, `image`, `rectangle`, `subreport`, …
    pub kind: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dataset: Option<String>,
    /// Textbox text (omitted with `--no-expressions`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_name: Option<String>,
    pub children: Vec<RdlItemOut>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MashupOut {
    /// `script`, `template`, `dataflowGen2` or `dataflowCdm`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub culture: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    pub queries: Vec<MashupQueryOut>,
    pub entities: Vec<DataflowEntityOut>,
    pub connections: Vec<MashupConnectionOut>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MashupQueryOut {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    pub entity: bool,
    pub syntax_error: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DataflowEntityOut {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_type: Option<String>,
    pub attributes: usize,
    pub partitions: Vec<DataflowPartitionOut>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DataflowPartitionOut {
    pub name: String,
    /// Secret parameters masked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_time: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MashupConnectionOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CustomVisualOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub support_url: Option<String>,
    pub data_roles: Vec<VisualDataRoleOut>,
    pub objects: Vec<String>,
    pub privileges: Vec<String>,
    pub script_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VisualDataRoleOut {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorOut {
    pub files: Vec<String>,
    pub signed: bool,
    pub sources: Vec<ConnectorSourceOut>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorSourceOut {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    pub members: Vec<ConnectorMemberOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorMemberOut {
    pub name: String,
    pub shared: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_source_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publish: Option<String>,
}

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

/// Builds the output for an opened document. `path` is the path as given
/// on the command line.
pub fn build(path: &str, opened: &OpenedDocument, opts: BuildOptions) -> Inspect {
    let doc = &opened.document;
    let model = doc.model.as_ref().map(|m| {
        let source = opened.model_source.as_ref().map(|s| ModelSourceOut {
            kind: s.kind().to_owned(),
            folder: s.folder().to_owned(),
        });
        let mut out = model_out(m, source, opts);
        out.cache_abf = cache_abf_out(opened, opened.model_source.as_ref().map(|s| s.folder()));
        out
    });
    let extra_models: Vec<ModelOut> = opened
        .extra_models
        .iter()
        .map(|e| {
            let source = ModelSourceOut {
                kind: e.source.kind().to_owned(),
                folder: e.source.folder().to_owned(),
            };
            let mut out = model_out(&e.model, Some(source), opts);
            out.cache_abf = cache_abf_out(opened, Some(e.source.folder()));
            out
        })
        .collect();
    let reports: Vec<ReportOut> = doc.reports.iter().map(report_out).collect();
    let diagnostics: Vec<DiagnosticOut> = doc.diagnostics.iter().map(diagnostic_out).collect();
    let counts = counts(model.iter().chain(&extra_models), &reports, &diagnostics);
    Inspect {
        schema_version: SCHEMA_VERSION,
        source: SourceOut {
            path: path.to_owned(),
            format: doc.source.format.as_str().to_owned(),
            detection: detection_out(&opened.detection),
            package_version: opened.package_version().map(str::to_owned),
        },
        model_status: model_status_out(&doc.model_status),
        model,
        extra_models,
        reports,
        diagnostics,
        counts,
        vpax_stats: opened.vpax_stats().map(vpax_out),
        paginated_report: opened.paginated_report().map(|r| paginated_out(r, opts)),
        mashup: opened.mashup().map(mashup_out),
        custom_visual: opened.custom_visual().map(custom_visual_out),
        connector: opened.connector().map(connector_out),
    }
}

/// The `cacheAbf` part of the model folder `folder`.
fn cache_abf_out(opened: &OpenedDocument, folder: Option<&str>) -> Option<CacheAbfOut> {
    let folder = folder?;
    let expected = if folder.is_empty() {
        ".pbi/cache.abf".to_owned()
    } else {
        format!("{folder}/.pbi/cache.abf")
    };
    let part = opened
        .parts
        .iter()
        .find(|p| p.kind == "cacheAbf" && p.path == expected)?;
    let json = part.json.as_ref()?;
    Some(CacheAbfOut {
        path: part.path.clone(),
        size: json
            .get("size")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        table_count: json
            .get("tableCount")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        tables: json
            .get("tables")
            .and_then(serde_json::Value::as_array)
            .map(|t| {
                t.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        matches_definition: json
            .get("matchesDefinition")
            .and_then(serde_json::Value::as_bool),
        metadata_format: json
            .get("metadataFormat")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        compatibility_level: json
            .get("compatibilityLevel")
            .and_then(serde_json::Value::as_i64),
    })
}

fn vpax_out(s: &ol::VpaxStats) -> VpaxStatsOut {
    VpaxStatsOut {
        source: s.source.clone(),
        model_name: s.model_name.clone(),
        compatibility_level: s.compatibility_level,
        extractor: s.extractor.clone(),
        extraction_date: s.extraction_date.clone(),
        last_data_refresh: s.last_data_refresh.clone(),
        total_size: s.total_size,
        tables: s
            .tables
            .iter()
            .map(|t| VpaxTableOut {
                name: t.name.clone(),
                rows_count: t.rows_count,
                columns_size: t.columns_size,
                user_hierarchies_size: t.user_hierarchies_size,
                relationships_size: t.relationships_size,
                table_size: t.table_size,
                is_hidden: t.is_hidden,
                columns: t
                    .columns
                    .iter()
                    .map(|c| VpaxColumnOut {
                        name: c.name.clone(),
                        data_type: c.data_type.clone(),
                        encoding: c.encoding.clone(),
                        cardinality: c.cardinality,
                        data_size: c.data_size,
                        dictionary_size: c.dictionary_size,
                        hierarchies_size: c.hierarchies_size,
                        total_size: c.total_size,
                        segments: c.segments,
                        is_hidden: c.is_hidden,
                    })
                    .collect(),
            })
            .collect(),
        relationships: s
            .relationships
            .iter()
            .map(|r| VpaxRelationshipOut {
                from: ColumnRefOut {
                    table: r.from_table.clone(),
                    column: r.from_column.clone(),
                },
                to: ColumnRefOut {
                    table: r.to_table.clone(),
                    column: r.to_column.clone(),
                },
                used_size: r.used_size,
                missing_keys: r.missing_keys,
                invalid_rows: r.invalid_rows,
            })
            .collect(),
    }
}

fn rdl_items_out(items: &[ol::RdlItem], opts: BuildOptions) -> Vec<RdlItemOut> {
    items
        .iter()
        .map(|i| RdlItemOut {
            kind: i.kind.clone(),
            name: i.name.clone(),
            dataset: i.dataset.clone(),
            value: i.value.clone().filter(|_| opts.expressions),
            report_name: i.report_name.clone(),
            children: rdl_items_out(&i.children, opts),
        })
        .collect()
}

fn paginated_out(r: &ol::PaginatedReport, opts: BuildOptions) -> PaginatedReportOut {
    PaginatedReportOut {
        schema: r.schema.clone(),
        description: r.description.clone(),
        author: r.author.clone(),
        data_sources: r
            .data_sources
            .iter()
            .map(|d| RdlDataSourceOut {
                name: d.name.clone(),
                data_provider: d.data_provider.clone(),
                connection_string: d.connection_string.clone(),
                shared_reference: d.shared_reference.clone(),
                integrated_security: d.integrated_security,
            })
            .collect(),
        datasets: r
            .datasets
            .iter()
            .map(|d| RdlDataSetOut {
                name: d.name.clone(),
                data_source: d.data_source.clone(),
                command_type: d.command_type.clone(),
                command_text: d.command_text.clone().filter(|_| opts.expressions),
                shared_reference: d.shared_reference.clone(),
                fields: d
                    .fields
                    .iter()
                    .map(|f| RdlFieldOut {
                        name: f.name.clone(),
                        data_field: f.data_field.clone(),
                        value: f.value.clone().filter(|_| opts.expressions),
                        type_name: f.type_name.clone(),
                    })
                    .collect(),
                query_parameters: d
                    .query_parameters
                    .iter()
                    .map(|q| RdlQueryParameterOut {
                        name: q.name.clone(),
                        value: q.value.clone().filter(|_| opts.expressions),
                    })
                    .collect(),
                filters: d.filters,
            })
            .collect(),
        parameters: r
            .parameters
            .iter()
            .map(|p| RdlParameterOut {
                name: p.name.clone(),
                data_type: p.data_type.clone(),
                prompt: p.prompt.clone(),
                nullable: p.nullable,
                allow_blank: p.allow_blank,
                multi_value: p.multi_value,
                hidden: p.hidden,
                default_values: p.default_values.clone(),
                default_dataset: p.default_dataset.clone(),
                valid_values_dataset: p.valid_values_dataset.clone(),
                valid_values: p.valid_values,
            })
            .collect(),
        sections: r
            .sections
            .iter()
            .map(|s| RdlSectionOut {
                page_width: s.page_width.clone(),
                page_height: s.page_height.clone(),
                body: rdl_items_out(&s.body, opts),
                header: rdl_items_out(&s.header, opts),
                footer: rdl_items_out(&s.footer, opts),
            })
            .collect(),
        embedded_images: r.embedded_images.clone(),
        item_count: r.item_count,
        truncated: r.truncated,
    }
}

fn mashup_out(m: &ol::Mashup) -> MashupOut {
    MashupOut {
        kind: m.kind.clone(),
        name: m.name.clone(),
        description: m.description.clone(),
        culture: m.culture.clone(),
        section: m.section.clone(),
        queries: m
            .queries
            .iter()
            .map(|q| MashupQueryOut {
                name: q.name.clone(),
                group: q.group.clone(),
                load_enabled: q.load_enabled,
                hidden: q.hidden,
                entity: q.entity,
                syntax_error: q.syntax_error,
            })
            .collect(),
        entities: m
            .entities
            .iter()
            .map(|e| DataflowEntityOut {
                name: e.name.clone(),
                entity_type: e.entity_type.clone(),
                attributes: e.attributes,
                partitions: e
                    .partitions
                    .iter()
                    .map(|p| DataflowPartitionOut {
                        name: p.name.clone(),
                        location: p.location.clone(),
                        refresh_time: p.refresh_time.clone(),
                    })
                    .collect(),
            })
            .collect(),
        connections: m
            .connections
            .iter()
            .map(|c| MashupConnectionOut {
                kind: c.kind.clone(),
                path: c.path.clone(),
            })
            .collect(),
    }
}

fn custom_visual_out(v: &ol::CustomVisual) -> CustomVisualOut {
    CustomVisualOut {
        name: v.name.clone(),
        display_name: v.display_name.clone(),
        guid: v.guid.clone(),
        version: v.version.clone(),
        description: v.description.clone(),
        author: v.author.clone(),
        api_version: v.api_version.clone(),
        support_url: v.support_url.clone(),
        data_roles: v
            .data_roles
            .iter()
            .map(|r| VisualDataRoleOut {
                name: r.name.clone(),
                display_name: r.display_name.clone(),
                kind: r.kind.clone(),
            })
            .collect(),
        objects: v.objects.clone(),
        privileges: v.privileges.clone(),
        script_bytes: v.script_bytes,
    }
}

fn connector_out(c: &ol::Connector) -> ConnectorOut {
    ConnectorOut {
        files: c.files.clone(),
        signed: c.signed,
        sources: c
            .sources
            .iter()
            .map(|s| ConnectorSourceOut {
                path: s.path.clone(),
                section: s.section.clone(),
                members: s
                    .members
                    .iter()
                    .map(|m| ConnectorMemberOut {
                        name: m.name.clone(),
                        shared: m.shared,
                        data_source_kind: m.data_source_kind.clone(),
                        publish: m.publish.clone(),
                    })
                    .collect(),
                error: s.error.clone(),
            })
            .collect(),
    }
}

pub fn detection_out(d: &Detection) -> DetectionOut {
    DetectionOut {
        format: detected_format_kind(&d.format).to_owned(),
        label: d.format.label(),
        confidence: confidence_name(d.confidence).to_owned(),
        signals: d.signals.clone(),
    }
}

/// Stable kind name of a detected format.
pub fn detected_format_kind(format: &DetectedFormat) -> &'static str {
    use DetectedFormat as F;
    match format {
        F::Pbip => "pbip",
        F::PbipSemanticModelFolder => "pbipSemanticModelFolder",
        F::PbipReportFolder => "pbipReportFolder",
        F::Pbix { .. } => "pbix",
        F::Pbit => "pbit",
        F::Bim => "bim",
        F::TmdlFolder => "tmdlFolder",
        F::TmdlFile => "tmdlFile",
        F::Pbids => "pbids",
        F::Rdl => "rdl",
        F::Abf { .. } => "abf",
        F::Vpax => "vpax",
        F::ExcelPowerPivot => "excelPowerPivot",
        F::ExcelNoModel => "excelNoModel",
        F::PowerQueryTemplate => "powerQueryTemplate",
        F::PowerQueryScript => "powerQueryScript",
        F::DataflowModelJson => "dataflowModelJson",
        F::DataflowFolder => "dataflowFolder",
        F::FabricItemFolder { .. } => "fabricItemFolder",
        F::Pbiviz => "pbiviz",
        F::CustomConnector => "customConnector",
        F::PbirJsonFile { .. } => "pbirJsonFile",
        F::LegacyLayoutJson => "legacyLayoutJson",
        F::FabricJsonFile { .. } => "fabricJsonFile",
        F::Unknown => "unknown",
        _ => "other",
    }
}

fn confidence_name(c: Confidence) -> &'static str {
    match c {
        Confidence::NoSignal => "noSignal",
        Confidence::ExtensionOnly => "extensionOnly",
        Confidence::Likely => "likely",
        Confidence::Certain => "certain",
    }
}

pub fn model_status_out(s: &ModelStatus) -> ModelStatusOut {
    match s {
        ModelStatus::Local => ModelStatusOut {
            status: "local".into(),
            description: None,
        },
        ModelStatus::Remote { description } => ModelStatusOut {
            status: "remote".into(),
            description: non_empty(description),
        },
        ModelStatus::Missing => ModelStatusOut {
            status: "missing".into(),
            description: None,
        },
    }
}

pub fn model_out(m: &Model, source: Option<ModelSourceOut>, opts: BuildOptions) -> ModelOut {
    ModelOut {
        name: m.name.clone(),
        description: m.description.clone(),
        culture: m.culture.clone(),
        compatibility_level: m.compatibility_level(),
        source,
        tables: m.tables.iter().map(|t| table_out(t, opts)).collect(),
        relationships: m
            .relationships
            .iter()
            .map(|r| RelationshipOut {
                name: r.name.clone(),
                from: ColumnRefOut {
                    table: r.from.table.clone(),
                    column: r.from.column.clone(),
                },
                to: ColumnRefOut {
                    table: r.to.table.clone(),
                    column: r.to.column.clone(),
                },
                from_cardinality: r.from_cardinality.as_str().to_owned(),
                to_cardinality: r.to_cardinality.as_str().to_owned(),
                cross_filtering_behavior: r.cross_filtering_behavior.as_str().to_owned(),
                is_active: r.is_active,
                security_filtering_behavior: r.security_filtering_behavior.as_str().to_owned(),
            })
            .collect(),
        expressions: m
            .expressions
            .iter()
            .map(|e| ExpressionOut {
                name: e.name.clone(),
                kind: e.kind.as_str().to_owned(),
                is_parameter: e.is_parameter(),
                parameter_type: e.parameter_type(),
                description: e.description.clone(),
                query_group: e.query_group.clone(),
                expression: opts.expressions.then(|| e.expression.clone()),
            })
            .collect(),
        roles: m
            .roles
            .iter()
            .map(|r| RoleOut {
                name: r.name.clone(),
                description: r.description.clone(),
                model_permission: r.model_permission.clone(),
                table_permissions: r
                    .table_permissions
                    .iter()
                    .map(|p| TablePermissionOut {
                        table: p.table.clone(),
                        filter_expression: p.filter_expression.clone().filter(|_| opts.expressions),
                    })
                    .collect(),
            })
            .collect(),
        cache_abf: None,
    }
}

pub fn table_out(t: &Table, opts: BuildOptions) -> TableOut {
    TableOut {
        name: t.name.clone(),
        description: t.description.clone(),
        is_hidden: t.is_hidden,
        kind: match t.kind() {
            TableKind::Regular => "regular",
            TableKind::CalculatedTable => "calculatedTable",
            TableKind::CalculationGroup => "calculationGroup",
            TableKind::Remote => "remote",
        }
        .to_owned(),
        data_category: t.data_category.clone(),
        columns: t.columns.iter().map(|c| column_out(c, opts)).collect(),
        measures: t
            .measures
            .iter()
            .map(|m| MeasureOut {
                name: m.name.clone(),
                expression: opts.expressions.then(|| m.expression.clone()),
                format_string: m.format_string.clone(),
                description: m.description.clone(),
                is_hidden: m.is_hidden,
                display_folder: m.display_folder.clone(),
            })
            .collect(),
        partitions: t
            .partitions
            .iter()
            .map(|p| {
                let (entity_name, expression_source) = match &p.source {
                    PartitionSource::Entity {
                        entity_name,
                        expression_source,
                        ..
                    } => (non_empty(entity_name), expression_source.clone()),
                    _ => (None, None),
                };
                let expression = match &p.source {
                    PartitionSource::Entity { .. } => None,
                    other => other.expression().map(str::to_owned),
                };
                PartitionOut {
                    name: p.name.clone(),
                    kind: p.source.kind_name().to_owned(),
                    mode: p.mode.as_str().to_owned(),
                    expression: expression.filter(|_| opts.expressions),
                    query_group: p.query_group.clone(),
                    entity_name,
                    expression_source,
                }
            })
            .collect(),
        hierarchies: t
            .hierarchies
            .iter()
            .map(|h| HierarchyOut {
                name: h.name.clone(),
                is_hidden: h.is_hidden,
                description: h.description.clone(),
                levels: h
                    .levels
                    .iter()
                    .map(|l| LevelOut {
                        name: l.name.clone(),
                        column: l.column.clone(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

pub fn column_out(c: &Column, opts: BuildOptions) -> ColumnOut {
    let kind = match &c.kind {
        ColumnKind::Data { .. } => "data",
        ColumnKind::Calculated { .. } => "calculated",
        ColumnKind::CalculatedTableColumn { .. } => "calculatedTableColumn",
        ColumnKind::RowNumber => "rowNumber",
    };
    ColumnOut {
        name: c.name.clone(),
        data_type: c.data_type.as_str().to_owned(),
        kind: kind.to_owned(),
        is_hidden: c.is_hidden,
        is_key: c.is_key,
        description: c.description.clone(),
        source_column: c.source_column().and_then(non_empty),
        expression: c
            .expression()
            .map(str::to_owned)
            .filter(|_| opts.expressions),
        format_string: c.format_string.clone(),
        summarize_by: c.summarize_by.as_ref().map(|s| s.as_str().to_owned()),
        sort_by_column: c.sort_by_column.clone(),
        display_folder: c.display_folder.clone(),
        data_category: c.data_category.clone(),
    }
}

pub fn report_out(r: &Report) -> ReportOut {
    ReportOut {
        name: r.name.clone(),
        dataset_reference: dataset_reference_out(&r.dataset_reference),
        base_theme: r.theme.base_theme.as_ref().map(|t| t.name.clone()),
        custom_theme: r.theme.custom_theme.as_ref().map(|t| t.name.clone()),
        active_page: r.active_page.clone(),
        filters: r.filters.iter().map(filter_out).collect(),
        pages: r.pages.iter().map(page_out).collect(),
    }
}

pub fn dataset_reference_out(d: &DatasetReference) -> DatasetReferenceOut {
    let empty = |kind: &str| DatasetReferenceOut {
        kind: kind.to_owned(),
        path: None,
        connection_string: None,
        model_id: None,
        database_name: None,
    };
    match d {
        DatasetReference::ByPath { path } => DatasetReferenceOut {
            path: Some(path.clone()),
            ..empty("byPath")
        },
        DatasetReference::ByConnection {
            connection_string,
            model_id,
            database_name,
            ..
        } => DatasetReferenceOut {
            connection_string: connection_string.clone(),
            model_id: model_id.clone(),
            database_name: database_name.clone(),
            ..empty("byConnection")
        },
        DatasetReference::Embedded => empty("embedded"),
        DatasetReference::None => empty("none"),
    }
}

pub fn page_out(p: &Page) -> PageOut {
    PageOut {
        name: p.name.clone(),
        display_name: p.display_name.clone(),
        width: p.width,
        height: p.height,
        display_option: p.display_option.as_str().to_owned(),
        is_hidden: p.is_hidden,
        filters: p.filters.iter().map(filter_out).collect(),
        visuals: p.visuals.iter().map(visual_out).collect(),
    }
}

pub fn visual_out(v: &Visual) -> VisualOut {
    let visual_type = if v.visual_type.is_empty() && v.is_group() {
        "group".to_owned()
    } else {
        v.visual_type.clone()
    };
    VisualOut {
        name: v.name.clone(),
        visual_type,
        is_group: v.is_group(),
        display_name: v.display_name.clone(),
        is_hidden: v.is_hidden,
        parent_group_name: v.parent_group_name.clone(),
        position: PositionOut {
            x: v.position.x,
            y: v.position.y,
            z: v.position.z,
            width: v.position.width,
            height: v.position.height,
            tab_order: v.position.tab_order,
        },
        fields: v.projections.iter().map(field_out).collect(),
        filters: v.filters.iter().map(filter_out).collect(),
    }
}

fn field_out(p: &Projection) -> FieldOut {
    FieldOut {
        role: p.role.clone(),
        field: field_ref_out(&p.field),
        query_ref: p.query_ref.clone(),
        native_query_ref: p.native_query_ref.clone(),
        display_name: p.display_name.clone(),
    }
}

pub fn field_ref_out(e: &FieldExpr) -> FieldRefOut {
    let (kind, aggregation, hierarchy) = match e {
        FieldExpr::Column { .. } => ("column", None, None),
        FieldExpr::Measure { .. } => ("measure", None, None),
        FieldExpr::Aggregation { function, .. } => (
            "aggregation",
            Some(
                function
                    .name()
                    .map_or_else(|| function.code().to_string(), str::to_owned),
            ),
            None,
        ),
        FieldExpr::HierarchyLevel { hierarchy, .. } => {
            ("hierarchyLevel", None, Some(hierarchy.clone()))
        }
        FieldExpr::Literal(_) => ("literal", None, None),
        FieldExpr::Unknown(_) => ("unknown", None, None),
    };
    FieldRefOut {
        kind: kind.to_owned(),
        table: e.entity().map(str::to_owned),
        name: e.property().map(str::to_owned),
        aggregation,
        hierarchy,
        reference: field_display(e),
    }
}

pub fn filter_out(f: &Filter) -> FilterOut {
    FilterOut {
        name: f.name.clone(),
        display_name: f.display_name.clone(),
        kind: f.kind.as_str().to_owned(),
        field: f.field.as_ref().map(field_ref_out),
        condition: f.condition.as_ref().map(condition_summary),
        is_hidden_in_view_mode: f.is_hidden_in_view_mode,
        is_locked_in_view_mode: f.is_locked_in_view_mode,
        how_created: f.how_created.clone(),
    }
}

pub fn diagnostic_out(d: &Diagnostic) -> DiagnosticOut {
    DiagnosticOut {
        severity: severity_name(d.severity).to_owned(),
        code: d.code.clone(),
        message: d.message.clone(),
        origin: d.origin.as_ref().map(|o| OriginOut {
            path: o.path.clone(),
            line: o.line,
            pointer: o.pointer.clone(),
        }),
    }
}

pub fn severity_name(s: Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

fn counts<'a>(
    models: impl Iterator<Item = &'a ModelOut>,
    reports: &[ReportOut],
    diagnostics: &[DiagnosticOut],
) -> Counts {
    let mut c = Counts::default();
    for m in models {
        c.tables += m.tables.len();
        c.relationships += m.relationships.len();
        c.expressions += m.expressions.len();
        c.parameters += m.expressions.iter().filter(|e| e.is_parameter).count();
        for t in &m.tables {
            c.columns += t.columns.len();
            c.calculated_columns += t.columns.iter().filter(|c| c.kind == "calculated").count();
            c.calculated_tables += usize::from(t.kind == "calculatedTable");
            c.measures += t.measures.len();
        }
    }
    c.reports = reports.len();
    for r in reports {
        c.pages += r.pages.len();
        c.filters += r.filters.len();
        for p in &r.pages {
            c.visuals += p.visuals.len();
            c.filters += p.filters.len();
            c.filters += p.visuals.iter().map(|v| v.filters.len()).sum::<usize>();
        }
    }
    for d in diagnostics {
        match d.severity.as_str() {
            "error" => c.errors += 1,
            "warning" => c.warnings += 1,
            _ => c.infos += 1,
        }
    }
    c
}

fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_owned())
}

// ---------------------------------------------------------------------------
// Display helpers (shared with the human-readable output)
// ---------------------------------------------------------------------------

/// DAX-style display of a field: `Stores[Channel]`,
/// `'Web Sessions'[Device]`, `Sum(Orders[Quantity])`,
/// `Calendar[Date Hierarchy].[Year]`, a literal, or `?` for unknown
/// expressions.
pub fn field_display(e: &FieldExpr) -> String {
    match e {
        FieldExpr::Column { entity, property } | FieldExpr::Measure { entity, property } => {
            dax_column_ref(entity, property)
        }
        FieldExpr::Aggregation { expr, function } => {
            let name = function
                .query_ref_name()
                .map_or_else(|| format!("Aggregation{}", function.code()), str::to_owned);
            format!("{name}({})", field_display(expr))
        }
        FieldExpr::HierarchyLevel {
            entity,
            hierarchy,
            level,
            ..
        } => format!(
            "{}[{}].[{}]",
            dax_table_ref(entity),
            hierarchy.replace(']', "]]"),
            level.replace(']', "]]")
        ),
        FieldExpr::Literal(l) => literal_display(l),
        FieldExpr::Unknown(_) => "?".to_owned(),
    }
}

fn literal_display(l: &Literal) -> String {
    match l.value() {
        Ok(LiteralValue::Text(s)) => format!("'{}'", s.replace('\'', "''")),
        Ok(LiteralValue::Integer(n)) => n.to_string(),
        Ok(LiteralValue::Double(f) | LiteralValue::Decimal(f)) => f.to_string(),
        Ok(LiteralValue::Boolean(b)) => b.to_string(),
        Ok(LiteralValue::DateTime(s)) => format!("datetime'{s}'"),
        Ok(LiteralValue::Null) => "null".to_owned(),
        Err(_) => l.raw.clone(),
    }
}

/// One-line summary of a filter condition, e.g.
/// `Stores[Channel] IN ('Retail', 'Outlet')`.
pub fn condition_summary(c: &Condition) -> String {
    match c {
        Condition::In { exprs, values } => {
            let tuple = |items: Vec<String>| {
                if items.len() == 1 {
                    items.concat()
                } else {
                    format!("({})", items.join(", "))
                }
            };
            let left = tuple(exprs.iter().map(field_display).collect());
            let rows: Vec<String> = values
                .iter()
                .map(|row| tuple(row.iter().map(literal_display).collect()))
                .collect();
            format!("{left} IN ({})", rows.join(", "))
        }
        Condition::Comparison { kind, left, right } => {
            let op = match kind {
                pbi_model::ComparisonKind::Equal => "=",
                pbi_model::ComparisonKind::GreaterThan => ">",
                pbi_model::ComparisonKind::GreaterThanOrEqual => ">=",
                pbi_model::ComparisonKind::LessThan => "<",
                pbi_model::ComparisonKind::LessThanOrEqual => "<=",
            };
            format!("{} {op} {}", field_display(left), field_display(right))
        }
        Condition::And(a, b) => format!("{} AND {}", nested(a), nested(b)),
        Condition::Or(a, b) => format!("{} OR {}", nested(a), nested(b)),
        Condition::Not(inner) => format!("NOT {}", nested(inner)),
        Condition::Contains { left, right } => {
            format!("{} CONTAINS {}", field_display(left), field_display(right))
        }
        Condition::StartsWith { left, right } => {
            format!(
                "{} STARTSWITH {}",
                field_display(left),
                field_display(right)
            )
        }
        Condition::Between { expr, lower, upper } => format!(
            "{} BETWEEN {} AND {}",
            field_display(expr),
            field_display(lower),
            field_display(upper)
        ),
        Condition::Unknown(_) => "(unsupported condition)".to_owned(),
    }
}

/// Parenthesizes compound sub-conditions.
fn nested(c: &Condition) -> String {
    let s = condition_summary(c);
    match c {
        Condition::And(..) | Condition::Or(..) => format!("({s})"),
        _ => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pbi_model::{AggregationFunction, ComparisonKind};

    #[test]
    fn conditions_are_summarized_on_one_line() {
        let channel = FieldExpr::column("Stores", "Channel");
        let cond = Condition::And(
            Box::new(Condition::In {
                exprs: vec![channel.clone()],
                values: vec![vec![Literal::text("Retail")], vec![Literal::text("It's")]],
            }),
            Box::new(Condition::Not(Box::new(Condition::Or(
                Box::new(Condition::Comparison {
                    kind: ComparisonKind::GreaterThanOrEqual,
                    left: FieldExpr::aggregation(
                        FieldExpr::column("Web Sessions", "Pages"),
                        AggregationFunction::Sum,
                    ),
                    right: FieldExpr::Literal(Literal::integer(5)),
                }),
                Box::new(Condition::Unknown(serde_json::Value::Null)),
            )))),
        );
        assert_eq!(
            condition_summary(&cond),
            "Stores[Channel] IN ('Retail', 'It''s') AND \
             NOT (Sum('Web Sessions'[Pages]) >= 5 OR (unsupported condition))"
        );
        let tuple = Condition::In {
            exprs: vec![channel, FieldExpr::column("Stores", "Is Open")],
            values: vec![vec![Literal::text("Outlet"), Literal::boolean(true)]],
        };
        assert_eq!(
            condition_summary(&tuple),
            "(Stores[Channel], Stores[Is Open]) IN (('Outlet', true))"
        );
    }

    #[test]
    fn field_refs_report_kind_table_and_name() {
        let agg = field_ref_out(&FieldExpr::aggregation(
            FieldExpr::column("Web Sessions", "Session ID"),
            AggregationFunction::CountNonNull,
        ));
        assert_eq!(agg.kind, "aggregation");
        assert_eq!(agg.table.as_deref(), Some("Web Sessions"));
        assert_eq!(agg.name.as_deref(), Some("Session ID"));
        assert_eq!(agg.aggregation.as_deref(), Some("countNonNull"));
        assert_eq!(agg.reference, "CountNonNull('Web Sessions'[Session ID])");
        let unknown = field_ref_out(&FieldExpr::Unknown(serde_json::json!({"x": 1})));
        assert_eq!(unknown.kind, "unknown");
        assert_eq!(unknown.table, None);
        assert_eq!(unknown.reference, "?");
    }
}
