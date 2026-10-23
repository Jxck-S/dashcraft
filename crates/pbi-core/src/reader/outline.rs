//! Format-specific structure that does not fit the model/report document:
//! VertiPaq Analyzer statistics, paginated report outlines, custom visual and
//! connector metadata, and Power Query mashup details. Exposed as
//! [`OpenedDocument::outline`](super::OpenedDocument::outline).
//!
//! | Input | Outline | Document |
//! |---|---|---|
//! | `.vpax` | [`FormatOutline::Vpax`] ([`VpaxStats`]) | model from `Model.bim`, else from `DaxModel.json` |
//! | `.rdl` | [`FormatOutline::PaginatedReport`] | no model, no reports |
//! | `.pq`/`.m`, `.pqt`, Dataflow Gen2, CDM `model.json` | [`FormatOutline::Mashup`] | model with the M queries |
//! | `.pbiviz` | [`FormatOutline::CustomVisual`] | nothing (never executed) |
//! | `.mez`/`.pqx` | [`FormatOutline::Connector`] | nothing (never executed) |
//!
//! Secrets never reach these types: RDL connection strings are replaced by
//! [`REDACTED`](crate::redact::REDACTED), dataflow locations and connection
//! paths have secret parameters masked, and custom visual scripts are only
//! measured.

use serde::Serialize;

/// Format-specific structure of an opened input.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[non_exhaustive]
pub enum FormatOutline {
    /// VertiPaq Analyzer statistics (`.vpax`).
    Vpax(VpaxStats),
    /// Paginated report outline (`.rdl`).
    PaginatedReport(PaginatedReport),
    /// Custom visual package metadata (`.pbiviz`).
    CustomVisual(CustomVisual),
    /// Custom connector listing (`.mez`, `.pqx`).
    Connector(Connector),
    /// Power Query details (`.pq`, `.pqt`, dataflows).
    Mashup(Mashup),
}

// ---------------------------------------------------------------------------
// VPAX
// ---------------------------------------------------------------------------

/// Size and cardinality statistics of a VertiPaq Analyzer export.
///
/// Sizes are bytes as reported by the engine DMVs at extraction time:
/// column `dataSize` = sum of the segments' `UsedSize`, `hierarchiesSize` =
/// sum of the attribute hierarchies' `UsedSize`, `totalSize` = data +
/// dictionary + hierarchies; table `tableSize` = columns + user hierarchies
/// + relationships (as VertiPaq Analyzer computes it).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VpaxStats {
    /// The part the statistics were read from: `DaxModel.json` or
    /// `DaxVpaView.json`.
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility_level: Option<i64>,
    /// Extracting application and version, e.g. `DAX Studio 3.2.1`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extractor: Option<String>,
    /// `ExtractionDate` (UTC, as written).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extraction_date: Option<String>,
    /// `LastDataRefresh` (UTC, as written).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_data_refresh: Option<String>,
    /// Sum of the table sizes.
    pub total_size: u64,
    pub tables: Vec<VpaxTableStats>,
    pub relationships: Vec<VpaxRelationshipStats>,
}

/// Statistics of one table.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VpaxTableStats {
    pub name: String,
    pub rows_count: u64,
    /// Sum of the columns' total sizes.
    pub columns_size: u64,
    pub user_hierarchies_size: u64,
    pub relationships_size: u64,
    /// Columns + user hierarchies + relationships.
    pub table_size: u64,
    pub is_hidden: bool,
    pub columns: Vec<VpaxColumnStats>,
}

/// Statistics of one column.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VpaxColumnStats {
    pub name: String,
    /// `Int64`, `String`, `DateTime`, … as written by the extractor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_type: Option<String>,
    /// `HASH` or `VALUE`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    /// Distinct values.
    pub cardinality: u64,
    pub data_size: u64,
    pub dictionary_size: u64,
    pub hierarchies_size: u64,
    pub total_size: u64,
    /// Number of segments.
    pub segments: u64,
    pub is_hidden: bool,
}

/// Statistics of one relationship.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VpaxRelationshipStats {
    pub from_table: String,
    pub from_column: String,
    pub to_table: String,
    pub to_column: String,
    /// `UsedSizeFrom` + `UsedSizeTo`.
    pub used_size: u64,
    /// Keys of the many side missing on the one side (referential
    /// integrity violations).
    pub missing_keys: u64,
    pub invalid_rows: u64,
}

// ---------------------------------------------------------------------------
// RDL
// ---------------------------------------------------------------------------

/// Outline of a paginated report definition.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginatedReport {
    /// RDL schema from the namespace, e.g. `2016/01`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    pub data_sources: Vec<RdlDataSource>,
    pub datasets: Vec<RdlDataSet>,
    pub parameters: Vec<RdlParameter>,
    /// Report sections (one for pre-2010 RDL).
    pub sections: Vec<RdlSection>,
    /// Names of embedded images (contents are not kept).
    pub embedded_images: Vec<String>,
    /// Report items listed (all sections, all nesting levels).
    pub item_count: usize,
    /// `true` if items were left out because of the size limits.
    pub truncated: bool,
}

/// A report data source.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RdlDataSource {
    pub name: String,
    /// `DataProvider`, e.g. `SQL`, `PBIDATASET`, `OLEDB`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_provider: Option<String>,
    /// Always [`REDACTED`](crate::redact::REDACTED) when the source has an
    /// embedded connection string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_string: Option<String>,
    /// `DataSourceReference` of a shared data source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrated_security: Option<bool>,
}

/// A report dataset (query).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RdlDataSet {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_source: Option<String>,
    /// `Text`, `StoredProcedure` or `TableDirect` (absent = `Text`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_type: Option<String>,
    /// Query or command text (SQL, DAX, MDX, …), verbatim.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_text: Option<String>,
    /// `SharedDataSet/SharedDataSetReference`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_reference: Option<String>,
    pub fields: Vec<RdlField>,
    pub query_parameters: Vec<RdlQueryParameter>,
    /// Number of dataset filters.
    pub filters: usize,
}

/// A dataset field.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RdlField {
    pub name: String,
    /// Source column (`DataField`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_field: Option<String>,
    /// Expression of a calculated field (`Value`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// `rd:TypeName`, e.g. `System.String`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
}

/// A query parameter of a dataset.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RdlQueryParameter {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// A report parameter.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RdlParameter {
    pub name: String,
    /// `String`, `Integer`, `Float`, `DateTime`, `Boolean`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    pub nullable: bool,
    pub allow_blank: bool,
    pub multi_value: bool,
    pub hidden: bool,
    /// Literal or expression default values.
    pub default_values: Vec<String>,
    /// Dataset providing the default values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_dataset: Option<String>,
    /// Dataset providing the available values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_values_dataset: Option<String>,
    /// Number of literal available values.
    pub valid_values: usize,
}

/// A report section: body, page header and page footer items.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RdlSection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_width: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_height: Option<String>,
    pub body: Vec<RdlItem>,
    pub header: Vec<RdlItem>,
    pub footer: Vec<RdlItem>,
}

/// A report item and the items nested in it.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RdlItem {
    /// `tablix`, `textbox`, `chart`, `image`, `rectangle`, `subreport`,
    /// `line`, `map`, `gaugePanel` or `customReportItem`.
    pub kind: String,
    pub name: String,
    /// `DataSetName` of data regions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dataset: Option<String>,
    /// Text runs of a textbox (values or expressions), joined.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// `ReportName` of a subreport.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_name: Option<String>,
    pub children: Vec<RdlItem>,
}

// ---------------------------------------------------------------------------
// Custom visuals and connectors
// ---------------------------------------------------------------------------

/// Metadata of a custom visual package. The visual is never executed; its
/// script is only measured.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomVisual {
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
    /// Author name (e-mail addresses are not kept).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Visuals API version, e.g. `5.3.0`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub support_url: Option<String>,
    pub data_roles: Vec<VisualDataRole>,
    /// Format pane object names.
    pub objects: Vec<String>,
    /// Requested privileges, e.g. `WebAccess`, `LocalStorage`.
    pub privileges: Vec<String>,
    /// Size of the bundled script (never run).
    pub script_bytes: u64,
    /// The `resources/*.pbiviz.json` part read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
}

/// A data role of a custom visual.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualDataRole {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// `Grouping`, `Measure` or `GroupingOrMeasure`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// A custom connector package, listed only (never executed).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connector {
    /// Entries of the package.
    pub files: Vec<String>,
    /// The package carries a digital signature part (`.pqx`).
    pub signed: bool,
    /// M source files (`*.m`, `*.pq`) and what they declare.
    pub sources: Vec<ConnectorSource>,
}

/// One M source file of a connector.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorSource {
    pub path: String,
    /// Section name (`section HelloWorld;`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    pub members: Vec<ConnectorMember>,
    /// Syntax error, if the file did not parse.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A section member of a connector.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorMember {
    pub name: String,
    pub shared: bool,
    /// `DataSource.Kind` attribute: the member is a data source function.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_source_kind: Option<String>,
    /// `Publish` attribute (navigator/UI record).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publish: Option<String>,
}

// ---------------------------------------------------------------------------
// Mashups (M scripts, templates, dataflows)
// ---------------------------------------------------------------------------

/// Power Query details of an M script, template or dataflow. The queries
/// themselves are in the document model (named expressions, or tables with
/// M partitions for dataflow entities).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mashup {
    /// `script` (`.pq`/`.m`), `template` (`.pqt`), `dataflowGen2` or
    /// `dataflowCdm` (`model.json`).
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Document locale / culture.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub culture: Option<String>,
    /// Section name of the M document.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    pub queries: Vec<MashupQuery>,
    /// Dataflow entities (CDM `model.json`).
    pub entities: Vec<DataflowEntity>,
    /// Connections (secret parameters masked).
    pub connections: Vec<MashupConnection>,
}

/// One query and its metadata.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MashupQuery {
    pub name: String,
    /// Query group folder, e.g. `Sources\Sales`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    /// The query is a dataflow entity (a table in the model).
    pub entity: bool,
    /// The query text did not parse (see the diagnostics).
    pub syntax_error: bool,
}

/// A dataflow entity (CDM).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataflowEntity {
    pub name: String,
    /// `$type`: `LocalEntity` or `ReferenceEntity`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_type: Option<String>,
    pub attributes: usize,
    pub partitions: Vec<DataflowPartition>,
}

/// A data partition (file) of a dataflow entity; listed, never read.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataflowPartition {
    pub name: String,
    /// Data location (secret parameters such as `sig=` masked).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_time: Option<String>,
}

/// A connection used by a dataflow.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MashupConnection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Connection path (secret parameters masked).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}
