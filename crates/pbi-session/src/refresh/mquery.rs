//! Evaluating a table's M partitions into a record batch with the model's
//! column names and engine types.

use std::sync::Arc;

use arrow::array::ArrayRef;
use arrow::compute::{CastOptions, cast_with_options, concat_batches};
use arrow::datatypes::{DataType as ArrowType, Field, Schema, TimeUnit};
use arrow::record_batch::{RecordBatch, RecordBatchOptions};
use indexmap::IndexMap;
use pbi_m::{MError, TableValue, TypeValue};
use pbi_model::{Column, ColumnKind, DataType, Model, NamedExpression, Table};

/// A model data column and where it comes from.
#[derive(Debug, Clone)]
pub(crate) struct ColumnSpec {
    /// Model (display) name.
    pub name: String,
    /// Source column name in the query result.
    pub source: String,
    pub data_type: DataType,
}

/// The data columns of `table` (not calculated, not row-number), in model
/// order. An empty `sourceColumn` falls back to the column name.
pub(crate) fn data_columns(table: &Table) -> Vec<ColumnSpec> {
    table
        .columns
        .iter()
        .filter_map(|c: &Column| match &c.kind {
            ColumnKind::Data { source_column } => Some(ColumnSpec {
                name: c.name.clone(),
                source: if source_column.trim().is_empty() {
                    c.name.clone()
                } else {
                    source_column.clone()
                },
                data_type: c.data_type.clone(),
            }),
            _ => None,
        })
        .collect()
}

/// Shared queries visible to every partition: the model's M expressions
/// (parameters such as `StorageContainerUrl`, helper queries) and, so that one
/// table's query can reference another by name, every table with a single
/// M partition.
pub(crate) fn shared_queries(model: &Model) -> IndexMap<String, String> {
    let mut shared = IndexMap::new();
    for e in &model.expressions {
        if e.kind == pbi_model::ExpressionKind::M {
            shared.insert(e.name.clone(), NamedExpression::full_expression(e));
        }
    }
    for t in &model.tables {
        let mut m = t.partitions.iter().filter_map(|p| match &p.source {
            pbi_model::PartitionSource::M { expression } => Some(expression),
            _ => None,
        });
        if let (Some(expr), None) = (m.next(), m.next())
            && !shared.keys().any(|k| pbi_model::names_eq(k, &t.name))
        {
            shared.insert(t.name.clone(), expr.clone());
        }
    }
    shared
}

/// Arrow type requested from M for a model data type (`None`: use the
/// query's column type).
fn m_arrow_type(dt: &DataType) -> Option<ArrowType> {
    Some(match dt {
        DataType::String => ArrowType::Utf8,
        DataType::Int64 => ArrowType::Int64,
        DataType::Double | DataType::Decimal => ArrowType::Float64,
        DataType::Boolean => ArrowType::Boolean,
        DataType::DateTime => ArrowType::Timestamp(TimeUnit::Microsecond, None),
        DataType::Binary | DataType::Variant | DataType::Automatic | DataType::Unknown(_) => {
            return None;
        }
    })
}

/// Evaluates every partition expression and returns one batch with the
/// model's data columns (model names, engine types). Extra query columns
/// are ignored; a missing source column is an error, as in Power BI.
pub(crate) fn evaluate_table(
    engine: &pbi_m::Engine,
    shared: &IndexMap<String, String>,
    exprs: &[String],
    columns: &[ColumnSpec],
) -> Result<RecordBatch, MError> {
    let mut batches = Vec::with_capacity(exprs.len());
    for src in exprs {
        let value = engine.evaluate_query(src, shared)?;
        let Some(table) = value.as_table() else {
            return Err(MError::expression(format!(
                "The query returned a value of type {} instead of a table.",
                value.kind().name()
            )));
        };
        batches.push(to_model_batch(table, columns)?);
    }
    match batches.len() {
        1 => batches
            .pop()
            .ok_or_else(|| MError::Internal("no partition result".into())),
        _ => {
            let schema = batches
                .first()
                .map_or_else(|| Arc::new(model_schema(columns)), RecordBatch::schema);
            concat_batches(&schema, &batches)
                .map_err(|e| MError::Internal(format!("cannot combine partitions: {e}")))
        }
    }
}

/// Empty-table schema of the data columns.
fn model_schema(columns: &[ColumnSpec]) -> Schema {
    Schema::new(
        columns
            .iter()
            .map(|c| Field::new(&c.name, pbi_engine::arrow_type(&c.data_type), true))
            .collect::<Vec<_>>(),
    )
}

/// Projects an M table onto the model's source columns and converts it.
fn to_model_batch(table: &TableValue, columns: &[ColumnSpec]) -> Result<RecordBatch, MError> {
    let missing: Vec<&str> = columns
        .iter()
        .filter(|c| table.column_index(&c.source).is_none())
        .map(|c| c.source.as_str())
        .collect();
    if !missing.is_empty() {
        let list = missing
            .iter()
            .map(|m| format!("'{m}'"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(MError::expression(format!(
            "The column {list} of the table wasn't found."
        )));
    }
    let mut sources: Vec<(String, usize, ArrowType)> = Vec::new();
    for c in columns {
        if sources.iter().any(|(s, _, _)| *s == c.source) {
            continue;
        }
        let Some(index) = table.column_index(&c.source) else {
            continue;
        };
        let m_type = table
            .column_type(index)
            .cloned()
            .unwrap_or_else(TypeValue::any);
        let arrow = m_arrow_type(&c.data_type)
            .unwrap_or_else(|| pbi_m::eval::arrow::arrow_type_for(&m_type));
        sources.push((c.source.clone(), index, arrow));
    }
    let projected = TableValue::new(
        sources
            .iter()
            .map(|(s, _, _)| Arc::from(s.as_str()))
            .collect(),
        sources
            .iter()
            .map(|(_, i, _)| {
                table
                    .column_type(*i)
                    .cloned()
                    .unwrap_or_else(TypeValue::any)
            })
            .collect(),
        sources
            .iter()
            .map(|(_, i, _)| Arc::new(table.column(*i).map(<[_]>::to_vec).unwrap_or_default()))
            .collect(),
    )?;
    let expected: Vec<(String, ArrowType)> = sources
        .iter()
        .map(|(s, _, t)| (s.clone(), t.clone()))
        .collect();
    let batch = pbi_m::table_to_record_batch(&projected, Some(&expected))?;
    let mut fields = Vec::with_capacity(columns.len());
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(columns.len());
    for c in columns {
        let Some(array) = batch.column_by_name(&c.source) else {
            continue;
        };
        let array = to_engine_type(array, &c.data_type).map_err(|e| {
            MError::data_format(format!(
                "Column '{}' cannot be stored as {}: {e}",
                c.name,
                c.data_type.as_str()
            ))
        })?;
        fields.push(Field::new(&c.name, array.data_type().clone(), true));
        arrays.push(array);
    }
    let options = RecordBatchOptions::new().with_row_count(Some(batch.num_rows()));
    RecordBatch::try_new_with_options(Arc::new(Schema::new(fields)), arrays, &options)
        .map_err(|e| MError::Internal(format!("building the table failed: {e}")))
}

/// Casts an array to the engine's storage type for a model data type
/// (fixed decimals become `Decimal128(19, 4)`). Columns without a declared
/// type keep their type.
pub(crate) fn to_engine_type(
    array: &ArrayRef,
    data_type: &DataType,
) -> Result<ArrayRef, arrow::error::ArrowError> {
    if m_arrow_type(data_type).is_none() {
        return Ok(Arc::clone(array));
    }
    let target = pbi_engine::arrow_type(data_type);
    if array.data_type() == &target {
        return Ok(Arc::clone(array));
    }
    let options = CastOptions {
        safe: false,
        ..CastOptions::default()
    };
    cast_with_options(array, &target, &options)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use arrow::array::{Array, AsArray};
    use arrow::datatypes::Decimal128Type;

    fn spec(name: &str, source: &str, dt: DataType) -> ColumnSpec {
        ColumnSpec {
            name: name.into(),
            source: source.into(),
            data_type: dt,
        }
    }

    #[test]
    fn renames_projects_and_types_columns() {
        let engine = pbi_m::Engine::default();
        let src = r#"#table(type table [a = text, b = number, extra = text], {{"x", 1.5, "e"}, {null, 2, "f"}})"#;
        let cols = [
            spec("Amount", "b", DataType::Decimal),
            spec("Name", "a", DataType::String),
            spec("Name again", "a", DataType::String),
        ];
        let batch =
            evaluate_table(&engine, &IndexMap::new(), &[src.into(), src.into()], &cols).unwrap();
        assert_eq!(batch.num_rows(), 4);
        let names: Vec<&str> = batch
            .schema_ref()
            .fields()
            .iter()
            .map(|f| f.name().as_str())
            .collect();
        assert_eq!(names, ["Amount", "Name", "Name again"]);
        let amount = batch.column(0).as_primitive::<Decimal128Type>();
        assert_eq!(amount.value(0), 15_000);
        assert!(batch.column(1).is_null(1));
    }

    #[test]
    fn missing_columns_are_named() {
        let engine = pbi_m::Engine::default();
        let err = evaluate_table(
            &engine,
            &IndexMap::new(),
            &["#table({\"a\"}, {{1}})".into()],
            &[spec("B", "b", DataType::Int64)],
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "Expression.Error: The column 'b' of the table wasn't found."
        );
    }

    #[test]
    fn non_table_results_are_errors() {
        let engine = pbi_m::Engine::default();
        let err = evaluate_table(&engine, &IndexMap::new(), &["1 + 1".into()], &[]).unwrap_err();
        assert!(err.to_string().contains("instead of a table"), "{err}");
    }
}
