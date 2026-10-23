//! The capability contract: stable ids, the data each one may expose, and
//! typed inputs (what is sent) and outputs (what the model must answer).
//!
//! Every capability declares a [`DataExposure`] class. v1 capabilities send
//! [`DataExposure::Metadata`] only (plus [`DataExposure::Sample`] rows when
//! a caller asks for them and the settings allow it); nothing sends
//! [`DataExposure::Full`].

use std::fmt;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// An AI capability. The id (`dax.explain`, …) is stable.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub enum Capability {
    /// Explain a measure: its DAX, dependencies and filter-context behaviour.
    #[serde(rename = "dax.explain")]
    DaxExplain,
    /// Natural language → a DAX measure suggestion.
    #[serde(rename = "dax.generate")]
    DaxGenerate,
    /// Natural language → a Power Query (M) query suggestion.
    #[serde(rename = "m.generate")]
    MGenerate,
    /// Explain and suggest fixes for report lint findings.
    #[serde(rename = "report.lint")]
    ReportLint,
    /// Suggest a visual for a question.
    #[serde(rename = "chart.suggest")]
    ChartSuggest,
}

impl Capability {
    /// Every capability, in contract order.
    pub const ALL: [Capability; 5] = [
        Capability::DaxExplain,
        Capability::DaxGenerate,
        Capability::MGenerate,
        Capability::ReportLint,
        Capability::ChartSuggest,
    ];

    /// The stable id (`dax.explain`, …).
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Capability::DaxExplain => "dax.explain",
            Capability::DaxGenerate => "dax.generate",
            Capability::MGenerate => "m.generate",
            Capability::ReportLint => "report.lint",
            Capability::ChartSuggest => "chart.suggest",
        }
    }

    /// Short title.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Capability::DaxExplain => "Explain a DAX measure",
            Capability::DaxGenerate => "Suggest a DAX measure",
            Capability::MGenerate => "Suggest a Power Query (M) query",
            Capability::ReportLint => "Explain report lint findings",
            Capability::ChartSuggest => "Suggest a visual",
        }
    }

    /// The exposure class of a request without sample rows.
    #[must_use]
    pub fn exposure(self) -> DataExposure {
        DataExposure::Metadata
    }

    /// The most a request of this capability may expose.
    #[must_use]
    pub fn max_exposure(self) -> DataExposure {
        match self {
            Capability::DaxGenerate | Capability::ChartSuggest => DataExposure::Sample,
            _ => DataExposure::Metadata,
        }
    }

    /// Implemented in this version (the others answer
    /// [`AiError::Unsupported`](crate::AiError::Unsupported)).
    #[must_use]
    pub fn implemented(self) -> bool {
        matches!(self, Capability::DaxExplain | Capability::DaxGenerate)
    }

    /// Parses an id.
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id() == id.trim())
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// What kind of data a request contains. Ordered: a consent for a class
/// covers the classes below it.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Default,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum DataExposure {
    /// Names, data types, descriptions, format strings, relationships and
    /// DAX/M expressions. No row values.
    #[default]
    Metadata,
    /// Metadata plus at most `sampleRows` rows of a table (off by default).
    Sample,
    /// Whole tables. No v1 capability sends this, and it cannot be granted.
    Full,
}

impl DataExposure {
    /// `metadata`, `sample`, `full`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            DataExposure::Metadata => "metadata",
            DataExposure::Sample => "sample",
            DataExposure::Full => "full",
        }
    }

    /// For people (consent prompts and errors).
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            DataExposure::Metadata => {
                "model metadata (names, data types, descriptions, relationships and DAX/M \
                 expressions; no row values)"
            }
            DataExposure::Sample => "model metadata plus sample rows of table data",
            DataExposure::Full => "full table data",
        }
    }

    /// A consent for `self` allows a request of class `request`.
    #[must_use]
    pub fn covers(self, request: DataExposure) -> bool {
        self >= request
    }
}

impl fmt::Display for DataExposure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The typed contract of one capability.
pub trait CapabilityContract {
    const CAPABILITY: Capability;
    /// What is sent to the provider (inside the prompt's data block).
    type Input: Serialize + DeserializeOwned + JsonSchema;
    /// What the model must answer (JSON, validated on receipt).
    type Output: Serialize + DeserializeOwned + JsonSchema;
}

/// JSON Schema (draft 2020-12) of `T` as the model must produce it: no
/// `$schema` key (some providers reject it).
#[must_use]
pub fn model_output_schema<T: JsonSchema>() -> Value {
    let mut schema = pbi_ops::output_schema::<T>();
    if let Value::Object(map) = &mut schema {
        map.remove("$schema");
    }
    schema
}

/// Description of a capability (`ai.status`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityInfo {
    pub id: Capability,
    pub title: String,
    /// Exposure class of a request without sample rows.
    pub exposure: DataExposure,
    /// The most a request may expose.
    pub max_exposure: DataExposure,
    /// Implemented in this version.
    pub implemented: bool,
    /// The registry command that runs it, when implemented.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

impl CapabilityInfo {
    #[must_use]
    pub fn of(c: Capability) -> Self {
        Self {
            id: c,
            title: c.title().to_owned(),
            exposure: c.exposure(),
            max_exposure: c.max_exposure(),
            implemented: c.implemented(),
            command: match c {
                Capability::DaxExplain => Some("ai.explainMeasure".to_owned()),
                Capability::DaxGenerate => Some("ai.generateMeasure".to_owned()),
                _ => None,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Shared metadata

/// A table (metadata only).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TableMeta {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `regular`, `calculatedTable`, `calculationGroup`, `remote`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// DAX of a calculated table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
}

/// A column (metadata only).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ColumnMeta {
    pub table: String,
    pub name: String,
    /// `string`, `int64`, `double`, `decimal`, `dateTime`, `boolean`, …
    pub data_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_string: Option<String>,
    /// DAX of a calculated column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_by_column: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

/// A measure (metadata only).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeasureMeta {
    /// Home table.
    pub table: String,
    pub name: String,
    /// DAX (omitted in large schemas).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_string: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_folder: Option<String>,
}

/// A relationship (metadata only).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelationshipMeta {
    /// `Table[Column]` on the `from` side (usually many).
    pub from: String,
    /// `Table[Column]` on the `to` side (usually one).
    pub to: String,
    /// `many-to-one`, `one-to-one`, `many-to-many`, `one-to-many`.
    pub cardinality: String,
    /// `single` (to → from), `both` or `automatic`.
    pub cross_filter: String,
    pub active: bool,
}

/// Every table, column, measure and relationship (bounded).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelSchemaMeta {
    pub tables: Vec<TableMeta>,
    pub columns: Vec<ColumnMeta>,
    pub measures: Vec<MeasureMeta>,
    pub relationships: Vec<RelationshipMeta>,
    /// Some objects or expressions were left out to stay within limits.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// Up to `sampleRows` rows of one table ([`DataExposure::Sample`]).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SampleRows {
    pub table: String,
    pub columns: Vec<String>,
    /// Row-major values (`null` for BLANK).
    pub rows: Vec<Vec<Value>>,
}

// ---------------------------------------------------------------------------
// dax.explain

/// `dax.explain`.
#[derive(Debug)]
pub struct DaxExplain;

/// Input of `dax.explain`: the measure and everything it reaches.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DaxExplainInput {
    /// The measure to explain.
    pub measure: MeasureMeta,
    /// Measures it references, transitively.
    pub referenced_measures: Vec<MeasureMeta>,
    /// Columns referenced by it or by the referenced measures.
    pub columns: Vec<ColumnMeta>,
    /// Tables referenced (directly, or through columns and measures).
    pub tables: Vec<TableMeta>,
    /// Relationships touching those tables.
    pub relationships: Vec<RelationshipMeta>,
    /// Model culture (formatting).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub culture: Option<String>,
    /// Local parse problems of the measure's DAX, if any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// A note about one dependency.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct DependencyNote {
    /// `[Measure]`, `Table[Column]` or `Table`.
    pub name: String,
    /// Its role in the calculation.
    pub note: String,
}

/// Output of `dax.explain`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct DaxExplainOutput {
    /// One or two sentences.
    pub summary: String,
    /// The full explanation (Markdown).
    pub explanation: String,
    /// Filter context, context transition and relationship behaviour.
    pub filter_context: Vec<String>,
    /// What each dependency contributes.
    pub dependencies: Vec<DependencyNote>,
    /// Pitfalls: BLANK handling, division by zero, ambiguous paths, …
    pub caveats: Vec<String>,
}

impl CapabilityContract for DaxExplain {
    const CAPABILITY: Capability = Capability::DaxExplain;
    type Input = DaxExplainInput;
    type Output = DaxExplainOutput;
}

// ---------------------------------------------------------------------------
// dax.generate

/// `dax.generate`.
#[derive(Debug)]
pub struct DaxGenerate;

/// Input of `dax.generate`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DaxGenerateInput {
    /// What the measure should compute, in the user's words.
    pub request: String,
    /// Home table for the new measure, if the user chose one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_table: Option<String>,
    pub schema: ModelSchemaMeta,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub culture: Option<String>,
    /// Sample rows (only with a [`DataExposure::Sample`] consent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample: Option<SampleRows>,
}

/// Output of `dax.generate`: a measure suggestion (never applied
/// automatically).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct DaxGenerateOutput {
    /// Suggested measure name.
    pub name: String,
    /// The DAX expression (without `Name =`).
    pub expression: String,
    /// Suggested format string.
    pub format_string: Option<String>,
    /// How it works (Markdown).
    pub explanation: String,
    /// Assumptions about the request or the model.
    pub assumptions: Vec<String>,
}

impl CapabilityContract for DaxGenerate {
    const CAPABILITY: Capability = Capability::DaxGenerate;
    type Input = DaxGenerateInput;
    type Output = DaxGenerateOutput;
}

// ---------------------------------------------------------------------------
// m.generate (contract only)

/// `m.generate` (not implemented yet).
#[derive(Debug)]
pub struct MGenerate;

/// A named M query.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MQueryMeta {
    pub name: String,
    pub expression: String,
}

/// Input of `m.generate`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MGenerateInput {
    pub request: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_table: Option<String>,
    /// Existing partitions and shared expressions.
    pub queries: Vec<MQueryMeta>,
    pub schema: ModelSchemaMeta,
}

/// Output of `m.generate`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct MGenerateOutput {
    pub name: String,
    pub expression: String,
    pub explanation: String,
    pub assumptions: Vec<String>,
}

impl CapabilityContract for MGenerate {
    const CAPABILITY: Capability = Capability::MGenerate;
    type Input = MGenerateInput;
    type Output = MGenerateOutput;
}

// ---------------------------------------------------------------------------
// report.lint (contract only)

/// `report.lint` (not implemented yet).
#[derive(Debug)]
pub struct ReportLint;

/// A finding of the local linter.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LintFindingMeta {
    pub rule: String,
    pub severity: String,
    pub message: String,
    /// Report / page / visual / field.
    pub location: String,
}

/// Input of `report.lint`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportLintInput {
    pub findings: Vec<LintFindingMeta>,
    pub schema: ModelSchemaMeta,
}

/// Advice for one finding.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct LintAdvice {
    /// Index into the input's `findings`.
    pub finding: usize,
    pub explanation: String,
    pub suggestion: String,
}

/// Output of `report.lint`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct ReportLintOutput {
    pub summary: String,
    pub advice: Vec<LintAdvice>,
}

impl CapabilityContract for ReportLint {
    const CAPABILITY: Capability = Capability::ReportLint;
    type Input = ReportLintInput;
    type Output = ReportLintOutput;
}

// ---------------------------------------------------------------------------
// chart.suggest (contract only)

/// `chart.suggest` (not implemented yet).
#[derive(Debug)]
pub struct ChartSuggest;

/// Input of `chart.suggest`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChartSuggestInput {
    pub question: String,
    pub schema: ModelSchemaMeta,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample: Option<SampleRows>,
}

/// A field bound to a visual role.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct ChartField {
    /// `Category`, `Y`, `Values`, `Legend`, …
    pub role: String,
    /// `Table[Column]` or `[Measure]`.
    pub field: String,
}

/// Output of `chart.suggest`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct ChartSuggestOutput {
    /// Power BI `visualType` (`clusteredBarChart`, `card`, …).
    pub visual_type: String,
    pub title: String,
    pub fields: Vec<ChartField>,
    pub explanation: String,
}

impl CapabilityContract for ChartSuggest {
    const CAPABILITY: Capability = Capability::ChartSuggest;
    type Input = ChartSuggestInput;
    type Output = ChartSuggestOutput;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_and_round_trip() {
        for c in Capability::ALL {
            let json = serde_json::to_value(c).unwrap_or_default();
            assert_eq!(json, Value::String(c.id().to_owned()));
            assert_eq!(Capability::from_id(c.id()), Some(c));
        }
        assert_eq!(
            Capability::ALL.map(Capability::id),
            [
                "dax.explain",
                "dax.generate",
                "m.generate",
                "report.lint",
                "chart.suggest"
            ]
        );
        let implemented: Vec<_> = Capability::ALL
            .into_iter()
            .filter(|c| c.implemented())
            .collect();
        assert_eq!(
            implemented,
            vec![Capability::DaxExplain, Capability::DaxGenerate]
        );
    }

    #[test]
    fn exposure_classes_are_ordered() {
        assert!(DataExposure::Sample.covers(DataExposure::Metadata));
        assert!(!DataExposure::Metadata.covers(DataExposure::Sample));
        assert!(DataExposure::Metadata.covers(DataExposure::Metadata));
        for c in Capability::ALL {
            assert!(
                c.max_exposure() < DataExposure::Full,
                "{c} never sends Full"
            );
            assert!(c.max_exposure().covers(c.exposure()));
        }
    }

    #[test]
    fn output_schemas_have_no_schema_key() {
        let s = model_output_schema::<DaxExplainOutput>();
        assert!(s.get("$schema").is_none());
        assert_eq!(s["type"], "object");
        assert!(s["properties"]["summary"].is_object());
    }
}
