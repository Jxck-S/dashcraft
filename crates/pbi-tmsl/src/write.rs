//! [`Model`] → TMSL JSON.

use std::borrow::Cow;

use pbi_model::{
    Annotation, CalculationGroup, CalculationGroupExpression, CalculationItem, Cardinality, Column,
    ColumnKind, CrossFilter, Culture, DataType, ExpressionKind, Hierarchy, Level, Measure, Model,
    NamedExpression, Partition, PartitionMode, PartitionSource, Perspective, PropValue,
    PropertyBag, QueryGroup, Relationship, Role, SecurityFilteringBehavior, SummarizeBy, Table,
    TablePermission,
};
use serde_json::Value;

use crate::WriteOptions;
use crate::codec::{
    Annotations, Codec, ColumnType, ColumnTypeCodec, EnumCodec, FLAG, Flag, FormatStringDefinition,
    Int, IntDefault, LinguisticMetadata, ModelName, NamedExpressionText, OPT_MULTI, OPT_SINGLE,
    OptEnum, REQ_MULTI, REQ_SINGLE, SourceType, SourceTypeCodec,
};
use crate::format::KeyOrder;
use crate::obj::{ObjWriter, is_absent_marker};
use crate::order;

pub(crate) fn write_root(model: &Model, options: &WriteOptions) -> Value {
    let style = options.format.key_order;
    let empty = PropertyBag::new();
    let database = model.database.as_ref();
    let mut w = ObjWriter::new(database.map_or(&empty, |d| &d.extra));
    let name = options
        .database_name
        .clone()
        .or_else(|| database.and_then(|d| d.name.clone()));
    w.put("name", &OPT_SINGLE, &name);
    w.put(
        "description",
        &OPT_MULTI,
        &database.and_then(|d| d.description.clone()),
    );
    w.put(
        "compatibilityLevel",
        &Int,
        &options
            .compatibility_level
            .or_else(|| database.and_then(|d| d.compatibility_level)),
    );
    w.put(
        "compatibilityMode",
        &OPT_SINGLE,
        &database.and_then(|d| d.compatibility_mode.clone()),
    );
    w.put_value("model", write_model(model, style));
    let annotations: &[Annotation] = database.map_or(&[], |d| &d.annotations);
    w.put("annotations", &Annotations, &annotations.to_vec());
    w.finish(order::ROOT, style)
}

fn list<T>(items: &[T], style: KeyOrder, write: fn(&T, KeyOrder) -> Value) -> Vec<Value> {
    items.iter().map(|item| write(item, style)).collect()
}

fn write_model(m: &Model, style: KeyOrder) -> Value {
    let mut w = ObjWriter::new(&m.extra);
    w.put("name", &ModelName, &m.name);
    w.put("description", &OPT_MULTI, &m.description);
    w.put("culture", &OPT_SINGLE, &m.culture);
    w.put(
        "defaultPowerBIDataSourceVersion",
        &OPT_SINGLE,
        &m.default_power_bi_data_source_version,
    );
    w.put("sourceQueryCulture", &OPT_SINGLE, &m.source_query_culture);
    w.put(
        "discourageImplicitMeasures",
        &FLAG,
        &m.discourage_implicit_measures,
    );
    w.put_collection("tables", list(&m.tables, style, write_table));
    w.put_collection(
        "relationships",
        list(&m.relationships, style, write_relationship),
    );
    w.put_collection(
        "perspectives",
        list(&m.perspectives, style, write_perspective),
    );
    w.put_collection("cultures", list(&m.cultures, style, write_culture));
    w.put_collection("roles", list(&m.roles, style, write_role));
    w.put_collection("expressions", list(&m.expressions, style, write_expression));
    w.put_collection(
        "queryGroups",
        list(&m.query_groups, style, write_query_group),
    );
    w.put("annotations", &Annotations, &m.annotations);
    w.finish(order::MODEL, style)
}

fn write_table(t: &Table, style: KeyOrder) -> Value {
    let mut w = ObjWriter::new(&t.extra);
    w.put("name", &REQ_SINGLE, &t.name);
    w.put("description", &OPT_MULTI, &t.description);
    w.put("isHidden", &FLAG, &t.is_hidden);
    w.put("isPrivate", &FLAG, &t.is_private);
    w.put("dataCategory", &OPT_SINGLE, &t.data_category);
    w.put("lineageTag", &OPT_SINGLE, &t.lineage_tag);
    w.put("sourceLineageTag", &OPT_SINGLE, &t.source_lineage_tag);
    w.put_collection("columns", list(&t.columns, style, write_column));
    w.put_collection("partitions", list(&t.partitions, style, write_partition));
    w.put_collection("measures", list(&t.measures, style, write_measure));
    w.put_collection("hierarchies", list(&t.hierarchies, style, write_hierarchy));
    if let Some(group) = &t.calculation_group {
        w.put_value("calculationGroup", write_calculation_group(group, style));
    }
    w.put("annotations", &Annotations, &t.annotations);
    w.finish(order::TABLE, style)
}

fn write_calculation_group(g: &CalculationGroup, style: KeyOrder) -> Value {
    let mut w = ObjWriter::new(&g.extra);
    w.put("description", &OPT_MULTI, &g.description);
    w.put("precedence", &IntDefault { default: 0 }, &g.precedence);
    w.put_collection(
        "calculationItems",
        list(&g.calculation_items, style, write_calculation_item),
    );
    for (key, selection) in [
        (
            "multipleOrEmptySelectionExpression",
            &g.multiple_or_empty_selection_expression,
        ),
        ("noSelectionExpression", &g.no_selection_expression),
    ] {
        if let Some(e) = selection {
            w.put_value(key, write_selection_expression(e, style));
        }
    }
    w.put("annotations", &Annotations, &g.annotations);
    w.finish(order::CALCULATION_GROUP, style)
}

fn write_calculation_item(i: &CalculationItem, style: KeyOrder) -> Value {
    let mut w = ObjWriter::new(&i.extra);
    w.put("name", &REQ_SINGLE, &i.name);
    w.put("description", &OPT_MULTI, &i.description);
    w.put("expression", &REQ_MULTI, &i.expression);
    w.put("ordinal", &Int, &i.ordinal);
    w.put(
        "formatStringDefinition",
        &FormatStringDefinition,
        &i.format_string_definition,
    );
    w.put("annotations", &Annotations, &i.annotations);
    w.finish(order::CALCULATION_ITEM, style)
}

fn write_selection_expression(e: &CalculationGroupExpression, style: KeyOrder) -> Value {
    let mut w = ObjWriter::new(&e.extra);
    w.put("description", &OPT_MULTI, &e.description);
    w.put("expression", &REQ_MULTI, &e.expression);
    w.put(
        "formatStringDefinition",
        &FormatStringDefinition,
        &e.format_string_definition,
    );
    w.put("annotations", &Annotations, &e.annotations);
    w.finish(order::SELECTION_EXPRESSION, style)
}

fn write_column(c: &Column, style: KeyOrder) -> Value {
    let mut w = ObjWriter::new(&c.extra);
    let column_type = match c.kind {
        ColumnKind::Data { .. } => ColumnType::Data,
        ColumnKind::Calculated { .. } => ColumnType::Calculated,
        ColumnKind::CalculatedTableColumn { .. } => ColumnType::CalculatedTableColumn,
        ColumnKind::RowNumber => ColumnType::RowNumber,
    };
    w.put("type", &ColumnTypeCodec, &column_type);
    w.put("name", &REQ_SINGLE, &c.name);
    w.put("description", &OPT_MULTI, &c.description);
    w.put(
        "dataType",
        &EnumCodec::new(DataType::Automatic),
        &c.data_type,
    );
    w.put("isNameInferred", &FLAG, &c.is_name_inferred);
    w.put("isDataTypeInferred", &FLAG, &c.is_data_type_inferred);
    w.put("isHidden", &FLAG, &c.is_hidden);
    w.put("isKey", &FLAG, &c.is_key);
    match &c.kind {
        ColumnKind::Data { source_column }
        | ColumnKind::CalculatedTableColumn { source_column } => {
            w.put("sourceColumn", &REQ_SINGLE, source_column);
        }
        ColumnKind::Calculated { expression } => w.put("expression", &REQ_MULTI, expression),
        ColumnKind::RowNumber => {}
    }
    w.put("formatString", &OPT_SINGLE, &c.format_string);
    w.put("displayFolder", &OPT_SINGLE, &c.display_folder);
    w.put("sortByColumn", &OPT_SINGLE, &c.sort_by_column);
    w.put("lineageTag", &OPT_SINGLE, &c.lineage_tag);
    w.put("sourceLineageTag", &OPT_SINGLE, &c.source_lineage_tag);
    w.put("dataCategory", &OPT_SINGLE, &c.data_category);
    w.put(
        "summarizeBy",
        &OptEnum::<SummarizeBy>::new(),
        &c.summarize_by,
    );
    w.put("annotations", &Annotations, &c.annotations);
    w.finish(order::COLUMN, style)
}

fn write_measure(m: &Measure, style: KeyOrder) -> Value {
    let extra = tom_format_string_definition(&m.extra);
    let mut w = ObjWriter::new(&extra);
    w.put("name", &REQ_SINGLE, &m.name);
    w.put("description", &OPT_MULTI, &m.description);
    w.put("expression", &REQ_MULTI, &m.expression);
    w.put("formatString", &OPT_SINGLE, &m.format_string);
    w.put("isHidden", &FLAG, &m.is_hidden);
    w.put("displayFolder", &OPT_SINGLE, &m.display_folder);
    w.put("lineageTag", &OPT_SINGLE, &m.lineage_tag);
    w.put("dataCategory", &OPT_SINGLE, &m.data_category);
    w.put("annotations", &Annotations, &m.annotations);
    w.finish(order::MEASURE, style)
}

/// A measure's dynamic format string read from TMDL (or set in code) is an
/// expression in `extra`; TOM JSON wraps it as `{"expression": …}`. JSON
/// read from TMSL is written as it was.
fn tom_format_string_definition(extra: &PropertyBag) -> Cow<'_, PropertyBag> {
    let text = match extra.get("formatStringDefinition") {
        Some(PropValue::Expression(text) | PropValue::Text(text)) => text,
        _ => return Cow::Borrowed(extra),
    };
    let Some(value) = FormatStringDefinition.encode(&Some(text.clone())) else {
        return Cow::Borrowed(extra);
    };
    let mut bag = extra.clone();
    bag.insert("formatStringDefinition".into(), PropValue::Json(value));
    Cow::Owned(bag)
}

fn write_partition(p: &Partition, style: KeyOrder) -> Value {
    let mut w = ObjWriter::new(&p.extra);
    w.put("name", &REQ_SINGLE, &p.name);
    w.put("description", &OPT_MULTI, &p.description);
    w.put("mode", &EnumCodec::new(PartitionMode::Default), &p.mode);
    w.put("queryGroup", &OPT_SINGLE, &p.query_group);
    let source_extra = p.extra.get("source");
    if source_extra.is_some_and(is_absent_marker) && p.source == PartitionSource::default() {
        w.claim("source");
    } else {
        let rest: PropertyBag = match source_extra.map(PropValue::to_json) {
            Some(Value::Object(map)) => map
                .into_iter()
                .map(|(k, v)| (k, PropValue::Json(v)))
                .collect(),
            _ => PropertyBag::new(),
        };
        w.put_value("source", write_source(&p.source, &rest, style));
    }
    w.put("annotations", &Annotations, &p.annotations);
    w.finish(order::PARTITION, style)
}

fn write_source(source: &PartitionSource, rest: &PropertyBag, style: KeyOrder) -> Value {
    if let PartitionSource::Unknown { kind, props } = source {
        let mut w = ObjWriter::new(props);
        w.put("type", &SourceTypeCodec, &SourceType::Other(kind.clone()));
        return w.finish(order::SOURCE, style);
    }
    let mut w = ObjWriter::new(rest);
    match source {
        PartitionSource::M { expression } => {
            w.put("type", &SourceTypeCodec, &SourceType::M);
            w.put("expression", &REQ_MULTI, expression);
        }
        PartitionSource::Calculated { expression } => {
            w.put("type", &SourceTypeCodec, &SourceType::Calculated);
            w.put("expression", &REQ_MULTI, expression);
        }
        PartitionSource::Query { query, data_source } => {
            w.put("type", &SourceTypeCodec, &SourceType::Query);
            w.put("query", &REQ_MULTI, query);
            w.put("dataSource", &OPT_SINGLE, data_source);
        }
        PartitionSource::Entity {
            entity_name,
            expression_source,
            schema_name,
        } => {
            w.put("type", &SourceTypeCodec, &SourceType::Entity);
            w.put("entityName", &REQ_SINGLE, entity_name);
            w.put("expressionSource", &OPT_SINGLE, expression_source);
            w.put("schemaName", &OPT_SINGLE, schema_name);
        }
        PartitionSource::Unknown { .. } => {}
    }
    w.finish(order::SOURCE, style)
}

fn write_hierarchy(h: &Hierarchy, style: KeyOrder) -> Value {
    let mut w = ObjWriter::new(&h.extra);
    w.put("name", &REQ_SINGLE, &h.name);
    w.put("description", &OPT_MULTI, &h.description);
    w.put("isHidden", &FLAG, &h.is_hidden);
    w.put("displayFolder", &OPT_SINGLE, &h.display_folder);
    w.put("lineageTag", &OPT_SINGLE, &h.lineage_tag);
    w.put_collection("levels", list(&h.levels, style, write_level));
    w.put("annotations", &Annotations, &h.annotations);
    w.finish(order::HIERARCHY, style)
}

fn write_level(l: &Level, style: KeyOrder) -> Value {
    let mut w = ObjWriter::new(&l.extra);
    w.put("name", &REQ_SINGLE, &l.name);
    w.put("description", &OPT_MULTI, &l.description);
    w.put("ordinal", &Int, &l.ordinal);
    w.put("column", &REQ_SINGLE, &l.column);
    w.put("lineageTag", &OPT_SINGLE, &l.lineage_tag);
    w.put("annotations", &Annotations, &l.annotations);
    w.finish(order::LEVEL, style)
}

fn write_relationship(r: &Relationship, style: KeyOrder) -> Value {
    let mut w = ObjWriter::new(&r.extra);
    w.put("name", &REQ_SINGLE, &r.name);
    w.put(
        "fromCardinality",
        &EnumCodec::new(Cardinality::Many),
        &r.from_cardinality,
    );
    w.put("fromTable", &REQ_SINGLE, &r.from.table);
    w.put("fromColumn", &REQ_SINGLE, &r.from.column);
    w.put("toTable", &REQ_SINGLE, &r.to.table);
    w.put("toColumn", &REQ_SINGLE, &r.to.column);
    w.put("isActive", &Flag { default: true }, &r.is_active);
    w.put(
        "crossFilteringBehavior",
        &EnumCodec::new(CrossFilter::OneDirection),
        &r.cross_filtering_behavior,
    );
    w.put(
        "toCardinality",
        &EnumCodec::new(Cardinality::One),
        &r.to_cardinality,
    );
    w.put(
        "relyOnReferentialIntegrity",
        &FLAG,
        &r.rely_on_referential_integrity,
    );
    w.put(
        "securityFilteringBehavior",
        &EnumCodec::new(SecurityFilteringBehavior::OneDirection),
        &r.security_filtering_behavior,
    );
    w.put("annotations", &Annotations, &r.annotations);
    w.finish(order::RELATIONSHIP, style)
}

fn write_expression(e: &NamedExpression, style: KeyOrder) -> Value {
    let mut w = ObjWriter::new(&e.extra);
    w.put("name", &REQ_SINGLE, &e.name);
    w.put("description", &OPT_MULTI, &e.description);
    w.put("kind", &EnumCodec::always(ExpressionKind::M), &e.kind);
    let text = NamedExpressionText {
        split_meta: e.kind == ExpressionKind::M,
    };
    w.put("expression", &text, &(e.expression.clone(), e.meta.clone()));
    w.put("lineageTag", &OPT_SINGLE, &e.lineage_tag);
    w.put("queryGroup", &OPT_SINGLE, &e.query_group);
    w.put("annotations", &Annotations, &e.annotations);
    w.finish(order::EXPRESSION, style)
}

fn write_role(r: &Role, style: KeyOrder) -> Value {
    let mut w = ObjWriter::new(&r.extra);
    w.put("name", &REQ_SINGLE, &r.name);
    w.put("description", &OPT_MULTI, &r.description);
    w.put("modelPermission", &OPT_SINGLE, &r.model_permission);
    w.put_collection(
        "tablePermissions",
        list(&r.table_permissions, style, write_table_permission),
    );
    w.put("annotations", &Annotations, &r.annotations);
    w.finish(order::ROLE, style)
}

fn write_table_permission(p: &TablePermission, style: KeyOrder) -> Value {
    let mut w = ObjWriter::new(&p.extra);
    w.put("name", &REQ_SINGLE, &p.table);
    w.put("filterExpression", &OPT_MULTI, &p.filter_expression);
    w.put("annotations", &Annotations, &p.annotations);
    w.finish(order::TABLE_PERMISSION, style)
}

fn write_culture(c: &Culture, style: KeyOrder) -> Value {
    let mut w = ObjWriter::new(&c.extra);
    w.put("name", &REQ_SINGLE, &c.name);
    w.put(
        "linguisticMetadata",
        &LinguisticMetadata,
        &c.linguistic_metadata,
    );
    w.put("annotations", &Annotations, &c.annotations);
    w.finish(order::CULTURE, style)
}

fn write_perspective(p: &Perspective, style: KeyOrder) -> Value {
    let mut w = ObjWriter::new(&p.extra);
    w.put("name", &REQ_SINGLE, &p.name);
    w.put("description", &OPT_MULTI, &p.description);
    w.put("annotations", &Annotations, &p.annotations);
    w.finish(order::PERSPECTIVE, style)
}

fn write_query_group(g: &QueryGroup, style: KeyOrder) -> Value {
    let mut w = ObjWriter::new(&g.extra);
    w.put("folder", &REQ_SINGLE, &g.folder);
    w.put("description", &OPT_MULTI, &g.description);
    w.put("annotations", &Annotations, &g.annotations);
    w.finish(order::QUERY_GROUP, style)
}
