//! Calculated columns and calculated tables.

use std::sync::Arc;

use arrow::array::ArrayRef;
use arrow::datatypes::DataType as ArrowType;
use arrow::record_batch::RecordBatch;
use pbi_engine::value::values_to_array;
use pbi_engine::{EngineSnapshot, FilterContext, RowSet, Value};
use pbi_model::{ColumnKind, DataType, Model};

use super::context::{Env, RowFrame};
use super::error::{DaxError, Result};
use super::interp::{Interp, parse};
use super::table::{RowPos, VirtualTable, column_array, record_batch_with_names};
use super::{EvalOptions, on_eval_thread};

/// Arrow type of a model column type, `None` when it must be inferred.
fn model_arrow_type(dt: &DataType) -> Option<ArrowType> {
    match dt {
        DataType::Automatic | DataType::Variant | DataType::Unknown(_) | DataType::Binary => None,
        other => Some(pbi_engine::arrow_type(other)),
    }
}

/// Computes calculated column `column` of `table`: the expression is
/// evaluated once per row with a row context on that row and an empty
/// filter context. The array is typed per the model `dataType` (inferred
/// for `automatic`). Errors in any row fail the column.
pub fn compute_calculated_column(
    snapshot: &EngineSnapshot,
    model: &Model,
    table: &str,
    column: &str,
) -> Result<ArrayRef> {
    compute_calculated_column_with(snapshot, model, table, column, &EvalOptions::default())
}

pub(crate) fn compute_calculated_column_with(
    snapshot: &EngineSnapshot,
    model: &Model,
    table: &str,
    column: &str,
    opts: &EvalOptions,
) -> Result<ArrayRef> {
    let mt = model.table(table).ok_or_else(|| DaxError::UnknownTable {
        name: table.to_owned(),
    })?;
    let col = mt
        .column(column)
        .ok_or_else(|| DaxError::unknown_column(&mt.name, column))?;
    let Some(src) = col.expression() else {
        return Err(DaxError::argument(
            "calculated column",
            format!(
                "{} is not a calculated column",
                pbi_model::dax_column_ref(&mt.name, &col.name)
            ),
        ));
    };
    let expr = parse(src)?;
    let data = Arc::clone(snapshot.table(&mt.name)?);
    let home = mt.name.clone();
    let values = on_eval_thread(|| {
        let interp = Interp::with_options(snapshot, model, opts);
        let n = data.row_count();
        let base = Arc::new(VirtualTable::from_base(
            Arc::clone(&data),
            RowSet::all(n),
            false,
        ));
        let mut env = Env::new(FilterContext::new());
        env.home = Some(Arc::from(home.as_str()));
        let mut values: Vec<Value> = Vec::with_capacity(n);
        for r in 0..n {
            interp.check_cancel()?;
            let row_env = env.with_row(RowFrame::new(Arc::clone(&base), RowPos::Base(r)));
            values.push(interp.scalar(&row_env, &expr)?);
        }
        Ok(values)
    })?;
    match model_arrow_type(&col.data_type) {
        Some(t) => values_to_array(&values, &t).map_err(|e| {
            DaxError::type_error(format!(
                "The values of calculated column {} cannot be stored as {}: {e}",
                pbi_model::dax_column_ref(&mt.name, &col.name),
                col.data_type.as_str()
            ))
        }),
        None => Ok(column_array(&values, None)),
    }
}

/// Splits a calculated-table `sourceColumn` (`[Name]`, `'T'[Name]`,
/// `T[Name]` or `Name`) into `(table, column)`.
fn parse_source_column(s: &str) -> (Option<String>, String) {
    let s = s.trim();
    let Some(body) = s.strip_suffix(']') else {
        return (None, s.to_owned());
    };
    if let Some(name) = body.strip_prefix('[') {
        return (None, name.replace("]]", "]"));
    }
    let (table, rest) = if let Some(quoted) = body.strip_prefix('\'') {
        let mut table = String::new();
        let mut chars = quoted.char_indices().peekable();
        let mut end = None;
        while let Some((i, c)) = chars.next() {
            if c == '\'' {
                if chars.peek().is_some_and(|(_, n)| *n == '\'') {
                    table.push('\'');
                    chars.next();
                    continue;
                }
                end = Some(i + 1);
                break;
            }
            table.push(c);
        }
        match end.and_then(|e| quoted.get(e..)) {
            Some(rest) => (table, rest),
            None => return (None, s.to_owned()),
        }
    } else {
        match body.find('[') {
            Some(i) => (
                body.get(..i).unwrap_or_default().trim().to_owned(),
                body.get(i..).unwrap_or_default(),
            ),
            None => return (None, s.to_owned()),
        }
    };
    match rest.strip_prefix('[') {
        Some(name) => (Some(table), name.replace("]]", "]")),
        None => (None, s.to_owned()),
    }
}

/// Computes calculated table `table_name`: its partition expression is
/// evaluated with an empty filter context, and the result columns are
/// mapped to the model columns (by `sourceColumn` such as `[App Short
/// Name]`, else by name) and typed per the model. A table without model
/// columns yields the expression's columns as they are.
pub fn compute_calculated_table(
    snapshot: &EngineSnapshot,
    model: &Model,
    table_name: &str,
) -> Result<RecordBatch> {
    compute_calculated_table_with(snapshot, model, table_name, &EvalOptions::default())
}

pub(crate) fn compute_calculated_table_with(
    snapshot: &EngineSnapshot,
    model: &Model,
    table_name: &str,
    opts: &EvalOptions,
) -> Result<RecordBatch> {
    let mt = model
        .table(table_name)
        .ok_or_else(|| DaxError::UnknownTable {
            name: table_name.to_owned(),
        })?;
    let Some(src) = mt.calculated_expression() else {
        return Err(DaxError::argument(
            "calculated table",
            format!("'{}' is not a calculated table", mt.name),
        ));
    };
    let expr = parse(src)?;
    let vt = on_eval_thread(|| {
        let interp = Interp::with_options(snapshot, model, opts);
        let env = Env::new(FilterContext::new());
        Ok((*interp.table(&env, &expr)?).clone())
    })?;
    let model_cols: Vec<_> = mt
        .columns
        .iter()
        .filter(|c| {
            !matches!(
                c.kind,
                ColumnKind::RowNumber | ColumnKind::Calculated { .. }
            )
        })
        .collect();
    if model_cols.is_empty() {
        let names: Vec<String> = vt.columns.iter().map(|c| c.name.clone()).collect();
        return Ok(record_batch_with_names(&vt, &names, &[]));
    }
    let positions = vt.positions();
    let mut fields = Vec::with_capacity(model_cols.len());
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(model_cols.len());
    for c in model_cols {
        let by_source = c.source_column().and_then(|s| {
            let (t, n) = parse_source_column(s);
            vt.find_column(t.as_deref(), &n)
        });
        let idx = by_source
            .or_else(|| vt.find_exact(None, &c.name))
            .ok_or_else(|| {
                DaxError::evaluation(format!(
                    "The expression of calculated table '{}' does not produce column [{}].",
                    mt.name, c.name
                ))
            })?;
        let values: Vec<Value> = positions.iter().map(|p| vt.value(*p, idx)).collect();
        let preferred = model_arrow_type(&c.data_type)
            .or_else(|| vt.columns.get(idx).and_then(|vc| vc.data_type.clone()));
        let array = column_array(&values, preferred.as_ref());
        fields.push(arrow::datatypes::Field::new(
            c.name.clone(),
            array.data_type().clone(),
            true,
        ));
        arrays.push(array);
    }
    let schema = Arc::new(arrow::datatypes::Schema::new(fields));
    let options =
        arrow::record_batch::RecordBatchOptions::new().with_row_count(Some(positions.len()));
    RecordBatch::try_new_with_options(schema, arrays, &options)
        .map_err(|e| DaxError::evaluation(format!("cannot build calculated table: {e}")))
}

#[cfg(test)]
mod tests {
    use super::parse_source_column;

    #[test]
    fn source_columns() {
        assert_eq!(
            parse_source_column("[Store Name]"),
            (None, "Store Name".into())
        );
        assert_eq!(
            parse_source_column("'Web Sessions'[Session ID]"),
            (Some("Web Sessions".into()), "Session ID".into())
        );
        assert_eq!(
            parse_source_column("T[a]]b]"),
            (Some("T".into()), "a]b".into())
        );
        assert_eq!(parse_source_column("Plain"), (None, "Plain".into()));
    }
}
