//! TMSL JSON → [`Model`].

use pbi_model::{
    CalculationGroup, CalculationGroupExpression, CalculationItem, Cardinality, Column, ColumnKind,
    CrossFilter, Culture, DataType, Database, ExpressionKind, Hierarchy, Level, Measure, Model,
    NamedExpression, Partition, PartitionMode, PartitionSource, Perspective, PropValue,
    PropertyBag, QueryGroup, Relationship, Role, SecurityFilteringBehavior, Severity, SummarizeBy,
    Table, TablePermission,
};
use serde_json::Value;

use crate::codec::{
    Annotations, ColumnType, ColumnTypeCodec, EnumCodec, FLAG, Flag, FormatStringDefinition, Int,
    IntDefault, LinguisticMetadata, ModelName, NamedExpressionText, OPT_MULTI, OPT_SINGLE, OptEnum,
    REQ_MULTI, REQ_SINGLE, SourceType, SourceTypeCodec,
};
use crate::error::TmslError;
use crate::obj::{ObjReader, ReadCx, absent_marker};

const ANNOTATIONS: (Severity, &str, &str) = (Severity::Warning, "TMSL002", "annotations");
const FORMAT_STRING_DEFINITION: (Severity, &str, &str) =
    (Severity::Info, "TMSL006", "format string definition");

/// Reads the TMSL root object. Returns the model with `database` set.
pub(crate) fn read_root(root: &Value, cx: &mut ReadCx) -> Result<Model, TmslError> {
    let mut r = ObjReader::new(root, String::new(), cx)?;
    let name = r.take("name", &OPT_SINGLE)?;
    let description = r.take("description", &OPT_MULTI)?;
    let compatibility_level = r.take("compatibilityLevel", &Int)?;
    let compatibility_mode = r.take("compatibilityMode", &OPT_SINGLE)?;
    let annotations = r.take_lenient("annotations", &Annotations, cx, ANNOTATIONS)?;
    let model_pointer = r.child("model");
    let model_value = r.raw("model").ok_or(TmslError::MissingModel)?;
    let mut model = read_model(model_value, model_pointer, cx)?;
    model.database = Some(Database {
        name,
        compatibility_level,
        compatibility_mode,
        description,
        annotations,
        origin: cx.origin(r.pointer()),
        extra: r.finish(),
        ..Database::default()
    });
    Ok(model)
}

fn read_list<T>(
    r: &mut ObjReader<'_>,
    key: &'static str,
    cx: &mut ReadCx,
    read: fn(&Value, String, &mut ReadCx) -> Result<T, TmslError>,
) -> Result<Vec<T>, TmslError> {
    let base = r.child(key);
    let items = r.array(key)?;
    items
        .iter()
        .enumerate()
        .map(|(i, item)| read(item, format!("{base}/{i}"), cx))
        .collect()
}

fn require_name(name: &str, what: &str, pointer: &str, cx: &mut ReadCx) {
    if name.is_empty() {
        cx.report(
            Severity::Warning,
            "TMSL004",
            format!("{what} has no name"),
            pointer,
        );
    }
}

fn read_model(value: &Value, pointer: String, cx: &mut ReadCx) -> Result<Model, TmslError> {
    let mut r = ObjReader::new(value, pointer, cx)?;
    let name = r.take("name", &ModelName)?;
    let description = r.take("description", &OPT_MULTI)?;
    let culture = r.take("culture", &OPT_SINGLE)?;
    let default_power_bi_data_source_version =
        r.take("defaultPowerBIDataSourceVersion", &OPT_SINGLE)?;
    let source_query_culture = r.take("sourceQueryCulture", &OPT_SINGLE)?;
    let discourage_implicit_measures = r.take("discourageImplicitMeasures", &FLAG)?;
    let tables = read_list(&mut r, "tables", cx, read_table)?;
    let relationships = read_list(&mut r, "relationships", cx, read_relationship)?;
    let perspectives = read_list(&mut r, "perspectives", cx, read_perspective)?;
    let cultures = read_list(&mut r, "cultures", cx, read_culture)?;
    let roles = read_list(&mut r, "roles", cx, read_role)?;
    let expressions = read_list(&mut r, "expressions", cx, read_expression)?;
    let query_groups = read_list(&mut r, "queryGroups", cx, read_query_group)?;
    let annotations = r.take_lenient("annotations", &Annotations, cx, ANNOTATIONS)?;
    Ok(Model {
        name,
        description,
        culture,
        default_power_bi_data_source_version,
        source_query_culture,
        discourage_implicit_measures,
        tables,
        relationships,
        expressions,
        query_groups,
        roles,
        cultures,
        perspectives,
        annotations,
        origin: cx.origin(r.pointer()),
        extra: r.finish(),
        ..Model::default()
    })
}

fn read_table(value: &Value, pointer: String, cx: &mut ReadCx) -> Result<Table, TmslError> {
    let mut r = ObjReader::new(value, pointer, cx)?;
    let name = r.take("name", &REQ_SINGLE)?;
    require_name(&name, "table", r.pointer(), cx);
    let description = r.take("description", &OPT_MULTI)?;
    let is_hidden = r.take("isHidden", &FLAG)?;
    let is_private = r.take("isPrivate", &FLAG)?;
    let data_category = r.take("dataCategory", &OPT_SINGLE)?;
    let lineage_tag = r.take("lineageTag", &OPT_SINGLE)?;
    let source_lineage_tag = r.take("sourceLineageTag", &OPT_SINGLE)?;
    let columns = read_list(&mut r, "columns", cx, read_column)?;
    let partitions = read_list(&mut r, "partitions", cx, read_partition)?;
    let measures = read_list(&mut r, "measures", cx, read_measure)?;
    let hierarchies = read_list(&mut r, "hierarchies", cx, read_hierarchy)?;
    let group_pointer = r.child("calculationGroup");
    let calculation_group = match r.raw("calculationGroup") {
        None => None,
        Some(value) => match read_calculation_group(value, group_pointer.clone(), cx) {
            Ok(group) => Some(Box::new(group)),
            Err(err) => {
                cx.report(
                    Severity::Warning,
                    "TMSL007",
                    format!("calculation group kept verbatim: {err}"),
                    &group_pointer,
                );
                r.keep("calculationGroup", PropValue::Json(value.clone()));
                None
            }
        },
    };
    let annotations = r.take_lenient("annotations", &Annotations, cx, ANNOTATIONS)?;
    Ok(Table {
        name,
        description,
        lineage_tag,
        source_lineage_tag,
        is_hidden,
        is_private,
        data_category,
        columns,
        measures,
        partitions,
        hierarchies,
        calculation_group,
        annotations,
        origin: cx.origin(r.pointer()),
        extra: r.finish(),
        ..Table::default()
    })
}

fn read_calculation_group(
    value: &Value,
    pointer: String,
    cx: &mut ReadCx,
) -> Result<CalculationGroup, TmslError> {
    let mut r = ObjReader::new(value, pointer, cx)?;
    let description = r.take("description", &OPT_MULTI)?;
    let precedence = r.take("precedence", &IntDefault { default: 0 })?;
    let calculation_items = read_list(&mut r, "calculationItems", cx, read_calculation_item)?;
    let multiple_or_empty_selection_expression =
        read_selection_expression(&mut r, "multipleOrEmptySelectionExpression", cx)?;
    let no_selection_expression = read_selection_expression(&mut r, "noSelectionExpression", cx)?;
    let annotations = r.take_lenient("annotations", &Annotations, cx, ANNOTATIONS)?;
    Ok(CalculationGroup {
        description,
        precedence,
        calculation_items,
        multiple_or_empty_selection_expression,
        no_selection_expression,
        annotations,
        extra: r.finish(),
    })
}

fn read_calculation_item(
    value: &Value,
    pointer: String,
    cx: &mut ReadCx,
) -> Result<CalculationItem, TmslError> {
    let mut r = ObjReader::new(value, pointer, cx)?;
    let name = r.take("name", &REQ_SINGLE)?;
    require_name(&name, "calculation item", r.pointer(), cx);
    let description = r.take("description", &OPT_MULTI)?;
    let expression = r.take("expression", &REQ_MULTI)?;
    let ordinal = r.take("ordinal", &Int)?;
    let format_string_definition = r.take_lenient(
        "formatStringDefinition",
        &FormatStringDefinition,
        cx,
        FORMAT_STRING_DEFINITION,
    )?;
    let annotations = r.take_lenient("annotations", &Annotations, cx, ANNOTATIONS)?;
    Ok(CalculationItem {
        name,
        description,
        expression,
        ordinal,
        format_string_definition,
        annotations,
        origin: cx.origin(r.pointer()),
        extra: r.finish(),
    })
}

/// A selection expression object; another JSON value is kept verbatim in
/// the group's `extra`.
fn read_selection_expression(
    r: &mut ObjReader<'_>,
    key: &'static str,
    cx: &mut ReadCx,
) -> Result<Option<CalculationGroupExpression>, TmslError> {
    let pointer = r.child(key);
    let Some(value) = r.raw(key) else {
        return Ok(None);
    };
    if !value.is_object() {
        r.keep(key, PropValue::Json(value.clone()));
        return Ok(None);
    }
    let mut s = ObjReader::new(value, pointer, cx)?;
    let description = s.take("description", &OPT_MULTI)?;
    let expression = s.take("expression", &REQ_MULTI)?;
    let format_string_definition = s.take_lenient(
        "formatStringDefinition",
        &FormatStringDefinition,
        cx,
        FORMAT_STRING_DEFINITION,
    )?;
    let annotations = s.take_lenient("annotations", &Annotations, cx, ANNOTATIONS)?;
    Ok(Some(CalculationGroupExpression {
        description,
        expression,
        format_string_definition,
        annotations,
        extra: s.finish(),
    }))
}

fn read_column(value: &Value, pointer: String, cx: &mut ReadCx) -> Result<Column, TmslError> {
    let mut r = ObjReader::new(value, pointer, cx)?;
    let column_type = r.take_lenient(
        "type",
        &ColumnTypeCodec,
        cx,
        (
            Severity::Warning,
            "TMSL003",
            "column type (read as a data column)",
        ),
    )?;
    let name = r.take("name", &REQ_SINGLE)?;
    require_name(&name, "column", r.pointer(), cx);
    let description = r.take("description", &OPT_MULTI)?;
    let data_type = r.take("dataType", &EnumCodec::new(DataType::Automatic))?;
    let is_name_inferred = r.take("isNameInferred", &FLAG)?;
    let is_data_type_inferred = r.take("isDataTypeInferred", &FLAG)?;
    let is_hidden = r.take("isHidden", &FLAG)?;
    let is_key = r.take("isKey", &FLAG)?;
    let kind = match column_type {
        ColumnType::Data => ColumnKind::Data {
            source_column: r.take("sourceColumn", &REQ_SINGLE)?,
        },
        ColumnType::CalculatedTableColumn => ColumnKind::CalculatedTableColumn {
            source_column: r.take("sourceColumn", &REQ_SINGLE)?,
        },
        ColumnType::Calculated => ColumnKind::Calculated {
            expression: r.take("expression", &REQ_MULTI)?,
        },
        ColumnType::RowNumber => ColumnKind::RowNumber,
    };
    let format_string = r.take("formatString", &OPT_SINGLE)?;
    let display_folder = r.take("displayFolder", &OPT_SINGLE)?;
    let sort_by_column = r.take("sortByColumn", &OPT_SINGLE)?;
    let lineage_tag = r.take("lineageTag", &OPT_SINGLE)?;
    let source_lineage_tag = r.take("sourceLineageTag", &OPT_SINGLE)?;
    let data_category = r.take("dataCategory", &OPT_SINGLE)?;
    let summarize_by = r.take("summarizeBy", &OptEnum::<SummarizeBy>::new())?;
    let annotations = r.take_lenient("annotations", &Annotations, cx, ANNOTATIONS)?;
    Ok(Column {
        name,
        description,
        lineage_tag,
        source_lineage_tag,
        data_type,
        kind,
        is_hidden,
        is_key,
        is_name_inferred,
        is_data_type_inferred,
        format_string,
        summarize_by,
        sort_by_column,
        display_folder,
        data_category,
        annotations,
        origin: cx.origin(r.pointer()),
        extra: r.finish(),
        ..Column::default()
    })
}

fn read_measure(value: &Value, pointer: String, cx: &mut ReadCx) -> Result<Measure, TmslError> {
    let mut r = ObjReader::new(value, pointer, cx)?;
    let name = r.take("name", &REQ_SINGLE)?;
    require_name(&name, "measure", r.pointer(), cx);
    let description = r.take("description", &OPT_MULTI)?;
    let expression = r.take("expression", &REQ_MULTI)?;
    let format_string = r.take("formatString", &OPT_SINGLE)?;
    let is_hidden = r.take("isHidden", &FLAG)?;
    let display_folder = r.take("displayFolder", &OPT_SINGLE)?;
    let lineage_tag = r.take("lineageTag", &OPT_SINGLE)?;
    let data_category = r.take("dataCategory", &OPT_SINGLE)?;
    let annotations = r.take_lenient("annotations", &Annotations, cx, ANNOTATIONS)?;
    Ok(Measure {
        name,
        expression,
        format_string,
        display_folder,
        is_hidden,
        description,
        lineage_tag,
        data_category,
        annotations,
        origin: cx.origin(r.pointer()),
        extra: r.finish(),
        ..Measure::default()
    })
}

fn read_partition(value: &Value, pointer: String, cx: &mut ReadCx) -> Result<Partition, TmslError> {
    let mut r = ObjReader::new(value, pointer, cx)?;
    let name = r.take("name", &REQ_SINGLE)?;
    let description = r.take("description", &OPT_MULTI)?;
    let mode = r.take("mode", &EnumCodec::new(PartitionMode::Default))?;
    let query_group = r.take("queryGroup", &OPT_SINGLE)?;
    let source_pointer = r.child("source");
    let source = match r.raw("source") {
        None => {
            r.keep("source", absent_marker());
            PartitionSource::default()
        }
        Some(value) => {
            let (source, rest) = read_source(value, source_pointer, cx)?;
            if !rest.is_empty() {
                let object = rest.into_iter().map(|(k, v)| (k, v.to_json())).collect();
                r.keep("source", PropValue::Json(Value::Object(object)));
            }
            source
        }
    };
    let annotations = r.take_lenient("annotations", &Annotations, cx, ANNOTATIONS)?;
    Ok(Partition {
        name,
        mode,
        source,
        query_group,
        description,
        annotations,
        origin: cx.origin(r.pointer()),
        extra: r.finish(),
        ..Partition::default()
    })
}

/// Reads a partition source. Returns the typed source and the properties
/// it does not type (stored as an object under `extra["source"]` of the
/// partition; empty for unknown source types, which keep everything in
/// [`PartitionSource::Unknown`]).
fn read_source(
    value: &Value,
    pointer: String,
    cx: &mut ReadCx,
) -> Result<(PartitionSource, PropertyBag), TmslError> {
    let mut r = ObjReader::new(value, pointer, cx)?;
    let source = match r.take("type", &SourceTypeCodec)? {
        SourceType::M => PartitionSource::M {
            expression: r.take("expression", &REQ_MULTI)?,
        },
        SourceType::Calculated => PartitionSource::Calculated {
            expression: r.take("expression", &REQ_MULTI)?,
        },
        SourceType::Query => PartitionSource::Query {
            query: r.take("query", &REQ_MULTI)?,
            data_source: r.take("dataSource", &OPT_SINGLE)?,
        },
        SourceType::Entity => PartitionSource::Entity {
            entity_name: r.take("entityName", &REQ_SINGLE)?,
            expression_source: r.take("expressionSource", &OPT_SINGLE)?,
            schema_name: r.take("schemaName", &OPT_SINGLE)?,
        },
        SourceType::Other(kind) => {
            let props = r.finish();
            return Ok((PartitionSource::Unknown { kind, props }, PropertyBag::new()));
        }
    };
    Ok((source, r.finish()))
}

fn read_hierarchy(value: &Value, pointer: String, cx: &mut ReadCx) -> Result<Hierarchy, TmslError> {
    let mut r = ObjReader::new(value, pointer, cx)?;
    let name = r.take("name", &REQ_SINGLE)?;
    let description = r.take("description", &OPT_MULTI)?;
    let is_hidden = r.take("isHidden", &FLAG)?;
    let display_folder = r.take("displayFolder", &OPT_SINGLE)?;
    let lineage_tag = r.take("lineageTag", &OPT_SINGLE)?;
    let levels = read_list(&mut r, "levels", cx, read_level)?;
    let annotations = r.take_lenient("annotations", &Annotations, cx, ANNOTATIONS)?;
    Ok(Hierarchy {
        name,
        description,
        lineage_tag,
        is_hidden,
        display_folder,
        levels,
        annotations,
        origin: cx.origin(r.pointer()),
        extra: r.finish(),
        ..Hierarchy::default()
    })
}

fn read_level(value: &Value, pointer: String, cx: &mut ReadCx) -> Result<Level, TmslError> {
    let mut r = ObjReader::new(value, pointer, cx)?;
    let name = r.take("name", &REQ_SINGLE)?;
    let description = r.take("description", &OPT_MULTI)?;
    let ordinal = r.take("ordinal", &Int)?;
    let column = r.take("column", &REQ_SINGLE)?;
    let lineage_tag = r.take("lineageTag", &OPT_SINGLE)?;
    let annotations = r.take_lenient("annotations", &Annotations, cx, ANNOTATIONS)?;
    Ok(Level {
        name,
        ordinal,
        column,
        description,
        lineage_tag,
        annotations,
        extra: r.finish(),
        ..Level::default()
    })
}

fn read_relationship(
    value: &Value,
    pointer: String,
    cx: &mut ReadCx,
) -> Result<Relationship, TmslError> {
    let mut r = ObjReader::new(value, pointer, cx)?;
    let name = r.take("name", &REQ_SINGLE)?;
    let from_cardinality = r.take("fromCardinality", &EnumCodec::new(Cardinality::Many))?;
    let from_table = r.take("fromTable", &REQ_SINGLE)?;
    let from_column = r.take("fromColumn", &REQ_SINGLE)?;
    let to_table = r.take("toTable", &REQ_SINGLE)?;
    let to_column = r.take("toColumn", &REQ_SINGLE)?;
    let is_active = r.take("isActive", &Flag { default: true })?;
    let cross_filtering_behavior = r.take(
        "crossFilteringBehavior",
        &EnumCodec::new(CrossFilter::OneDirection),
    )?;
    let to_cardinality = r.take("toCardinality", &EnumCodec::new(Cardinality::One))?;
    let rely_on_referential_integrity = r.take("relyOnReferentialIntegrity", &FLAG)?;
    let security_filtering_behavior = r.take(
        "securityFilteringBehavior",
        &EnumCodec::new(SecurityFilteringBehavior::OneDirection),
    )?;
    let annotations = r.take_lenient("annotations", &Annotations, cx, ANNOTATIONS)?;
    Ok(Relationship {
        name,
        from: pbi_model::ColumnRef::new(from_table, from_column),
        to: pbi_model::ColumnRef::new(to_table, to_column),
        from_cardinality,
        to_cardinality,
        cross_filtering_behavior,
        is_active,
        security_filtering_behavior,
        rely_on_referential_integrity,
        annotations,
        origin: cx.origin(r.pointer()),
        extra: r.finish(),
        ..Relationship::default()
    })
}

fn read_expression(
    value: &Value,
    pointer: String,
    cx: &mut ReadCx,
) -> Result<NamedExpression, TmslError> {
    let mut r = ObjReader::new(value, pointer, cx)?;
    let name = r.take("name", &REQ_SINGLE)?;
    let description = r.take("description", &OPT_MULTI)?;
    let kind = r.take("kind", &EnumCodec::always(ExpressionKind::M))?;
    let text = NamedExpressionText {
        split_meta: kind == ExpressionKind::M,
    };
    let (expression, meta) = r.take("expression", &text)?;
    let lineage_tag = r.take("lineageTag", &OPT_SINGLE)?;
    let query_group = r.take("queryGroup", &OPT_SINGLE)?;
    let annotations = r.take_lenient("annotations", &Annotations, cx, ANNOTATIONS)?;
    Ok(NamedExpression {
        name,
        kind,
        expression,
        meta,
        description,
        query_group,
        lineage_tag,
        annotations,
        origin: cx.origin(r.pointer()),
        extra: r.finish(),
        ..NamedExpression::default()
    })
}

fn read_role(value: &Value, pointer: String, cx: &mut ReadCx) -> Result<Role, TmslError> {
    let mut r = ObjReader::new(value, pointer, cx)?;
    let name = r.take("name", &REQ_SINGLE)?;
    let description = r.take("description", &OPT_MULTI)?;
    let model_permission = r.take("modelPermission", &OPT_SINGLE)?;
    let table_permissions = read_list(&mut r, "tablePermissions", cx, read_table_permission)?;
    let annotations = r.take_lenient("annotations", &Annotations, cx, ANNOTATIONS)?;
    Ok(Role {
        name,
        description,
        model_permission,
        table_permissions,
        annotations,
        origin: cx.origin(r.pointer()),
        extra: r.finish(),
        ..Role::default()
    })
}

fn read_table_permission(
    value: &Value,
    pointer: String,
    cx: &mut ReadCx,
) -> Result<TablePermission, TmslError> {
    let mut r = ObjReader::new(value, pointer, cx)?;
    let table = r.take("name", &REQ_SINGLE)?;
    let filter_expression = r.take("filterExpression", &OPT_MULTI)?;
    let annotations = r.take_lenient("annotations", &Annotations, cx, ANNOTATIONS)?;
    Ok(TablePermission {
        table,
        filter_expression,
        annotations,
        extra: r.finish(),
        ..TablePermission::default()
    })
}

fn read_culture(value: &Value, pointer: String, cx: &mut ReadCx) -> Result<Culture, TmslError> {
    let mut r = ObjReader::new(value, pointer, cx)?;
    let name = r.take("name", &REQ_SINGLE)?;
    let linguistic_metadata = r.take_lenient(
        "linguisticMetadata",
        &LinguisticMetadata,
        cx,
        (Severity::Info, "TMSL005", "linguistic metadata"),
    )?;
    let annotations = r.take_lenient("annotations", &Annotations, cx, ANNOTATIONS)?;
    Ok(Culture {
        name,
        linguistic_metadata,
        annotations,
        extra: r.finish(),
        ..Culture::default()
    })
}

fn read_perspective(
    value: &Value,
    pointer: String,
    cx: &mut ReadCx,
) -> Result<Perspective, TmslError> {
    let mut r = ObjReader::new(value, pointer, cx)?;
    let name = r.take("name", &REQ_SINGLE)?;
    let description = r.take("description", &OPT_MULTI)?;
    let annotations = r.take_lenient("annotations", &Annotations, cx, ANNOTATIONS)?;
    Ok(Perspective {
        name,
        description,
        annotations,
        extra: r.finish(),
        ..Perspective::default()
    })
}

fn read_query_group(
    value: &Value,
    pointer: String,
    cx: &mut ReadCx,
) -> Result<QueryGroup, TmslError> {
    let mut r = ObjReader::new(value, pointer, cx)?;
    let folder = r.take("folder", &REQ_SINGLE)?;
    let description = r.take("description", &OPT_MULTI)?;
    let annotations = r.take_lenient("annotations", &Annotations, cx, ANNOTATIONS)?;
    Ok(QueryGroup {
        folder,
        description,
        annotations,
        extra: r.finish(),
        ..QueryGroup::default()
    })
}
