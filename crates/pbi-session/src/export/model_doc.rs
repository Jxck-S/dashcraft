//! The "model doc": a documented, stable JSON description of a semantic
//! model ([`ModelDoc`], `schemaVersion: 1`), built by [`model_doc`].
//! The schema is documented on [`ModelDoc`].

use pbi_model::semantic::split_m_meta;
use pbi_model::{
    Column, ColumnKind, Measure, Model, NamedExpression, Partition, PartitionSource, Relationship,
    Role, Table, TableKind,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// Current [`ModelDoc::schema_version`].
pub const MODEL_DOC_SCHEMA_VERSION: u32 = 1;

/// What [`model_doc`] includes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelDocOptions {
    /// Include DAX, M and query text (default `true`). Turn it off to share
    /// a model outline without its logic.
    pub include_expressions: bool,
    /// Include hidden tables, columns, measures and hierarchies (default
    /// `true`).
    pub include_hidden: bool,
}

impl Default for ModelDocOptions {
    fn default() -> Self {
        Self {
            include_expressions: true,
            include_hidden: true,
        }
    }
}

/// Root of the model doc, a documented and stable JSON description of a
/// semantic model (`schemaVersion` 1), built by [`model_doc`].
///
/// Compatibility rules for `schemaVersion` 1:
/// - fields are only ever **added**; existing fields keep their name, type
///   and meaning (a breaking change bumps the version);
/// - arrays and booleans are always present (arrays may be empty);
/// - optional strings and numbers are omitted when absent or empty;
/// - enumerations use the TMDL/TMSL spellings (`string`, `dateTime`,
///   `bothDirections`, `import`, …) and keep unknown values verbatim;
/// - order is the model's order (deterministic output).
///
/// Shape:
///
/// ```text
/// schemaVersion: 1, name, description?, culture?, compatibilityLevel?,
/// includesExpressions, includesHidden,
/// summary { tables, columns, calculatedColumns, calculatedTables, measures,
///           relationships, parameters, expressions, roles }
/// tables[] { name, description?, kind: regular|calculatedTable|calculationGroup|remote,
///            hidden, dataCategory?,
///            columns[] { name, dataType, kind: data|calculated|calculatedTableColumn,
///                        hidden, key, description?, sourceColumn?, expression?,
///                        formatString?, displayFolder?, sortByColumn?, summarizeBy?,
///                        dataCategory? },
///            measures[] { name, expression?, formatString?, description?,
///                         displayFolder?, hidden },
///            partitions[] { name, kind: m|calculated|entity|query|…, language?: m|dax|sql,
///                           mode, expression?, queryGroup?, entityName?,
///                           expressionSource?, description? },
///            hierarchies[] { name, description?, hidden, displayFolder?,
///                            levels[] { name, column } } }
/// relationships[] { name, from { table, column }, to { table, column },
///                   fromCardinality, toCardinality, crossFilteringBehavior, active,
///                   securityFilteringBehavior }
/// parameters[] { name, type?, required, description?, queryGroup?, value? }
/// expressions[] { name, kind: m|dax, description?, queryGroup?, expression? }
/// roles[] { name, description?, modelPermission?,
///           tablePermissions[] { table, filterExpression? } }
/// ```
///
/// `expression`, `value` and `filterExpression` (DAX, M and query text) are
/// left out when [`ModelDocOptions::include_expressions`] is `false`; hidden
/// tables, columns, measures and hierarchies (and relationships to hidden
/// tables) when [`ModelDocOptions::include_hidden`] is `false`. Internal
/// `rowNumber` columns are never listed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDoc {
    /// [`MODEL_DOC_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Model name (often `Model`).
    pub name: String,
    /// Model description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Model culture, e.g. `en-US`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub culture: Option<String>,
    /// Database compatibility level, e.g. `1567`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility_level: Option<i64>,
    /// DAX/M text is included.
    pub includes_expressions: bool,
    /// Hidden objects are included.
    pub includes_hidden: bool,
    /// Counts of what is listed.
    pub summary: ModelDocSummary,
    /// Tables in model order.
    pub tables: Vec<TableDoc>,
    /// Relationships in model order.
    pub relationships: Vec<RelationshipDoc>,
    /// Power Query parameters (`IsParameterQuery=true`).
    pub parameters: Vec<ParameterDoc>,
    /// Other shared (named) expressions: M queries and DAX expressions.
    pub expressions: Vec<ExpressionDoc>,
    /// Security roles.
    pub roles: Vec<RoleDoc>,
}

/// Counts of the objects listed in a [`ModelDoc`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDocSummary {
    /// Tables.
    pub tables: usize,
    /// Columns of all tables (data and calculated).
    pub columns: usize,
    /// Calculated (DAX) columns.
    pub calculated_columns: usize,
    /// Calculated (DAX) tables.
    pub calculated_tables: usize,
    /// Measures.
    pub measures: usize,
    /// Relationships.
    pub relationships: usize,
    /// Parameters.
    pub parameters: usize,
    /// Shared expressions that are not parameters.
    pub expressions: usize,
    /// Roles.
    pub roles: usize,
}

/// One table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableDoc {
    /// Table name.
    pub name: String,
    /// Description (TMDL `///`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `regular`, `calculatedTable`, `calculationGroup` or `remote`.
    pub kind: String,
    /// Hidden in report view.
    pub hidden: bool,
    /// Data category (e.g. `Time`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_category: Option<String>,
    /// Columns in model order.
    pub columns: Vec<ColumnDoc>,
    /// Measures in model order.
    pub measures: Vec<MeasureDoc>,
    /// Partitions (data sources).
    pub partitions: Vec<PartitionDoc>,
    /// Hierarchies.
    pub hierarchies: Vec<HierarchyDoc>,
}

/// One column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnDoc {
    /// Column name.
    pub name: String,
    /// `string`, `int64`, `double`, `decimal`, `dateTime`, `boolean`, …
    pub data_type: String,
    /// `data`, `calculated` or `calculatedTableColumn`.
    pub kind: String,
    /// Hidden in report view.
    pub hidden: bool,
    /// Key column of its table.
    pub key: bool,
    /// Description (TMDL `///`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Source column (data columns; calculated table columns).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_column: Option<String>,
    /// DAX of a calculated column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    /// Format string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_string: Option<String>,
    /// Display folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_folder: Option<String>,
    /// Sort-by column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_by_column: Option<String>,
    /// Default aggregation (`none`, `sum`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summarize_by: Option<String>,
    /// Data category.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_category: Option<String>,
}

/// One measure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureDoc {
    /// Measure name.
    pub name: String,
    /// DAX expression, verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    /// Format string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_string: Option<String>,
    /// Description (TMDL `///`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Display folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_folder: Option<String>,
    /// Hidden in report view.
    pub hidden: bool,
}

/// One partition (the data source of a table).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartitionDoc {
    /// Partition name.
    pub name: String,
    /// `m`, `calculated`, `entity`, `query`, or another TMDL source kind.
    pub kind: String,
    /// Language of `expression`: `m` (Power Query), `dax` or `sql`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// `import`, `directQuery`, `dual`, `directLake` or `default`.
    pub mode: String,
    /// M, DAX or native query text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    /// Power Query group (folder).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_group: Option<String>,
    /// Entity partitions: the remote entity name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_name: Option<String>,
    /// Entity partitions: the named expression holding the source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression_source: Option<String>,
    /// Description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One hierarchy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HierarchyDoc {
    /// Hierarchy name.
    pub name: String,
    /// Description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Hidden in report view.
    pub hidden: bool,
    /// Display folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_folder: Option<String>,
    /// Levels, top-down.
    pub levels: Vec<LevelDoc>,
}

/// One hierarchy level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelDoc {
    /// Level name.
    pub name: String,
    /// Column of the level.
    pub column: String,
}

/// One relationship.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationshipDoc {
    /// Relationship name (often a GUID).
    pub name: String,
    /// The "from" end (TOM `fromColumn`, usually the many side).
    pub from: ColumnRefDoc,
    /// The "to" end (TOM `toColumn`, usually the one side).
    pub to: ColumnRefDoc,
    /// `many` or `one`.
    pub from_cardinality: String,
    /// `many` or `one`.
    pub to_cardinality: String,
    /// `oneDirection`, `bothDirections` or `automatic`.
    pub cross_filtering_behavior: String,
    /// Active relationship.
    pub active: bool,
    /// `oneDirection`, `bothDirections` or `none`.
    pub security_filtering_behavior: String,
}

/// A `table[column]` reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnRefDoc {
    /// Table name.
    pub table: String,
    /// Column name.
    pub column: String,
}

/// A Power Query parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterDoc {
    /// Parameter name.
    pub name: String,
    /// Parameter type (`Text`, `Number`, …).
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub parameter_type: Option<String>,
    /// `IsParameterQueryRequired=true`.
    pub required: bool,
    /// Description (TMDL `///`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Power Query group (folder).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_group: Option<String>,
    /// Current value, as M text (e.g. `"https://…"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// A shared expression that is not a parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpressionDoc {
    /// Expression name.
    pub name: String,
    /// `m` or `dax`.
    pub kind: String,
    /// Description (TMDL `///`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Power Query group (folder).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_group: Option<String>,
    /// Expression text (without a `meta [...]` record).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
}

/// A security role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleDoc {
    /// Role name.
    pub name: String,
    /// Description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `read`, `readRefresh`, …
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_permission: Option<String>,
    /// Row filters per table.
    pub table_permissions: Vec<TablePermissionDoc>,
}

/// A role's row filter on one table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TablePermissionDoc {
    /// Table name.
    pub table: String,
    /// DAX row filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_expression: Option<String>,
}

/// Builds the model doc of `model`. Session derived columns (see
/// [`crate::derived`]) are left out.
#[must_use]
pub fn model_doc(model: &Model, options: &ModelDocOptions) -> ModelDoc {
    let hidden_ok = |hidden: bool| options.include_hidden || !hidden;
    let tables: Vec<TableDoc> = model
        .tables
        .iter()
        .filter(|t| hidden_ok(t.is_hidden))
        .map(|t| table_doc(t, options))
        .collect();
    let visible_table =
        |name: &str| options.include_hidden || model.table(name).is_none_or(|t| !t.is_hidden);
    let relationships: Vec<RelationshipDoc> = model
        .relationships
        .iter()
        .filter(|r| visible_table(&r.from.table) && visible_table(&r.to.table))
        .map(relationship_doc)
        .collect();
    let parameters: Vec<ParameterDoc> = model
        .expressions
        .iter()
        .filter(|e| e.is_parameter())
        .map(|e| parameter_doc(e, options))
        .collect();
    let expressions: Vec<ExpressionDoc> = model
        .expressions
        .iter()
        .filter(|e| !e.is_parameter())
        .map(|e| ExpressionDoc {
            name: e.name.clone(),
            kind: e.kind.as_str().to_owned(),
            description: text(e.description.as_deref()),
            query_group: text(e.query_group.as_deref()),
            expression: expression(&e.expression, options),
        })
        .collect();
    let roles: Vec<RoleDoc> = model.roles.iter().map(|r| role_doc(r, options)).collect();
    let summary = ModelDocSummary {
        tables: tables.len(),
        columns: tables.iter().map(|t| t.columns.len()).sum(),
        calculated_columns: tables
            .iter()
            .flat_map(|t| &t.columns)
            .filter(|c| c.kind == "calculated")
            .count(),
        calculated_tables: tables
            .iter()
            .filter(|t| t.kind == "calculatedTable")
            .count(),
        measures: tables.iter().map(|t| t.measures.len()).sum(),
        relationships: relationships.len(),
        parameters: parameters.len(),
        expressions: expressions.len(),
        roles: roles.len(),
    };
    ModelDoc {
        schema_version: MODEL_DOC_SCHEMA_VERSION,
        name: model.name.clone(),
        description: text(model.description.as_deref()),
        culture: text(model.culture.as_deref()),
        compatibility_level: model.compatibility_level(),
        includes_expressions: options.include_expressions,
        includes_hidden: options.include_hidden,
        summary,
        tables,
        relationships,
        parameters,
        expressions,
        roles,
    }
}

/// The model doc of `model` as JSON, with every expression and hidden
/// object (see [`model_doc`] for other options).
#[must_use]
pub fn model_to_json(model: &Model) -> Json {
    serde_json::to_value(model_doc(model, &ModelDocOptions::default())).unwrap_or(Json::Null)
}

/// Trimmed text, `None` when empty.
pub(super) fn text(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn expression(s: &str, options: &ModelDocOptions) -> Option<String> {
    if options.include_expressions {
        text(Some(s))
    } else {
        None
    }
}

/// Stable kind name of a table.
pub(super) fn table_kind(kind: TableKind) -> &'static str {
    match kind {
        TableKind::Regular => "regular",
        TableKind::CalculatedTable => "calculatedTable",
        TableKind::CalculationGroup => "calculationGroup",
        TableKind::Remote => "remote",
    }
}

fn table_doc(t: &Table, options: &ModelDocOptions) -> TableDoc {
    let hidden_ok = |hidden: bool| options.include_hidden || !hidden;
    TableDoc {
        name: t.name.clone(),
        description: text(t.description.as_deref()),
        kind: table_kind(t.kind()).to_owned(),
        hidden: t.is_hidden,
        data_category: text(t.data_category.as_deref()),
        columns: t
            .columns
            .iter()
            .filter(|c| {
                !matches!(c.kind, ColumnKind::RowNumber)
                    && !crate::derived::is_derived(c)
                    && hidden_ok(c.is_hidden)
            })
            .map(|c| column_doc(c, options))
            .collect(),
        measures: t
            .measures
            .iter()
            .filter(|m| hidden_ok(m.is_hidden))
            .map(|m| measure_doc(m, options))
            .collect(),
        partitions: t
            .partitions
            .iter()
            .map(|p| partition_doc(p, options))
            .collect(),
        hierarchies: t
            .hierarchies
            .iter()
            .filter(|h| hidden_ok(h.is_hidden))
            .map(|h| HierarchyDoc {
                name: h.name.clone(),
                description: text(h.description.as_deref()),
                hidden: h.is_hidden,
                display_folder: text(h.display_folder.as_deref()),
                levels: h
                    .levels
                    .iter()
                    .map(|l| LevelDoc {
                        name: l.name.clone(),
                        column: l.column.clone(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn column_doc(c: &Column, options: &ModelDocOptions) -> ColumnDoc {
    let kind = match &c.kind {
        ColumnKind::Data { .. } => "data",
        ColumnKind::Calculated { .. } => "calculated",
        ColumnKind::CalculatedTableColumn { .. } => "calculatedTableColumn",
        ColumnKind::RowNumber => "rowNumber",
    };
    ColumnDoc {
        name: c.name.clone(),
        data_type: c.data_type.as_str().to_owned(),
        kind: kind.to_owned(),
        hidden: c.is_hidden,
        key: c.is_key,
        description: text(c.description.as_deref()),
        source_column: text(c.source_column()),
        expression: c.expression().and_then(|e| expression(e, options)),
        format_string: text(c.format_string.as_deref()),
        display_folder: text(c.display_folder.as_deref()),
        sort_by_column: text(c.sort_by_column.as_deref()),
        summarize_by: c.summarize_by.as_ref().map(|s| s.as_str().to_owned()),
        data_category: text(c.data_category.as_deref()),
    }
}

fn measure_doc(m: &Measure, options: &ModelDocOptions) -> MeasureDoc {
    MeasureDoc {
        name: m.name.clone(),
        expression: expression(&m.expression, options),
        format_string: text(m.format_string.as_deref()),
        description: text(m.description.as_deref()),
        display_folder: text(m.display_folder.as_deref()),
        hidden: m.is_hidden,
    }
}

fn partition_doc(p: &Partition, options: &ModelDocOptions) -> PartitionDoc {
    let (language, entity_name, expression_source) = match &p.source {
        PartitionSource::M { .. } => (Some("m"), None, None),
        PartitionSource::Calculated { .. } => (Some("dax"), None, None),
        PartitionSource::Query { .. } => (Some("sql"), None, None),
        PartitionSource::Entity {
            entity_name,
            expression_source,
            ..
        } => (
            None,
            text(Some(entity_name)),
            text(expression_source.as_deref()),
        ),
        PartitionSource::Unknown { .. } => (None, None, None),
    };
    let source_text = match &p.source {
        PartitionSource::Entity { .. } => None,
        other => other.expression(),
    };
    PartitionDoc {
        name: p.name.clone(),
        kind: p.source.kind_name().to_owned(),
        language: language.map(str::to_owned),
        mode: p.mode.as_str().to_owned(),
        expression: source_text.and_then(|e| expression(e, options)),
        query_group: text(p.query_group.as_deref()),
        entity_name,
        expression_source,
        description: text(p.description.as_deref()),
    }
}

fn relationship_doc(r: &Relationship) -> RelationshipDoc {
    RelationshipDoc {
        name: r.name.clone(),
        from: ColumnRefDoc {
            table: r.from.table.clone(),
            column: r.from.column.clone(),
        },
        to: ColumnRefDoc {
            table: r.to.table.clone(),
            column: r.to.column.clone(),
        },
        from_cardinality: r.from_cardinality.as_str().to_owned(),
        to_cardinality: r.to_cardinality.as_str().to_owned(),
        cross_filtering_behavior: r.cross_filtering_behavior.as_str().to_owned(),
        active: r.is_active,
        security_filtering_behavior: r.security_filtering_behavior.as_str().to_owned(),
    }
}

fn parameter_doc(e: &NamedExpression, options: &ModelDocOptions) -> ParameterDoc {
    ParameterDoc {
        name: e.name.clone(),
        parameter_type: e.parameter_type().and_then(|t| text(Some(&t))),
        required: e
            .meta_value("IsParameterQueryRequired")
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("true")),
        description: text(e.description.as_deref()),
        query_group: text(e.query_group.as_deref()),
        value: expression(split_m_meta(&e.expression).0, options),
    }
}

fn role_doc(r: &Role, options: &ModelDocOptions) -> RoleDoc {
    RoleDoc {
        name: r.name.clone(),
        description: text(r.description.as_deref()),
        model_permission: text(r.model_permission.as_deref()),
        table_permissions: r
            .table_permissions
            .iter()
            .map(|p| TablePermissionDoc {
                table: p.table.clone(),
                filter_expression: p
                    .filter_expression
                    .as_deref()
                    .and_then(|f| expression(f, options)),
            })
            .collect(),
    }
}
