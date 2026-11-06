//! Data of the calculated tables and columns an edit creates.
//!
//! A created calculated table is computed when the edit is published, in a
//! document whose data is loaded (otherwise the next refresh computes it).
//! A created calculated column is computed when the edit is published, over
//! the loaded data of its table (a table that is not loaded gets it at the
//! next refresh). Undo and redo re-attach the kept data instead
//! ([`crate::removed`]); only tables and columns without it are computed.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use pbi_engine::{Engine, FilterContext};
use pbi_model::names::{dax_column_ref, name_key};
use pbi_model::{Column, CommandError, DataType, Model, ObjectId, TableKind};

use crate::derived::data_type_of;
use crate::report::{RefreshKind, TableRefresh, TableStatus};
use crate::session::Session;

/// Computes the calculated tables of `new` that `old` did not have (by
/// object id) and that have no refresh record in `tables` (a restored table
/// gets its record and data back), in model order, and records them. Their
/// calculated columns follow in [`compute_created_columns`]. Nothing is
/// computed while no table has been loaded: the next refresh does it.
/// `eval_model` is `new` with the session's derived columns.
pub(crate) fn compute_created_tables(
    engine: &mut Engine,
    old: &Model,
    new: &Model,
    eval_model: &Model,
    tables: &mut HashMap<String, TableRefresh>,
) -> Result<(), CommandError> {
    if tables.is_empty() {
        return Ok(());
    }
    let existing: HashSet<ObjectId> = old.tables.iter().map(|t| t.id).collect();
    for table in new.tables.iter().filter(|t| {
        t.id.is_assigned() && !existing.contains(&t.id) && t.kind() == TableKind::CalculatedTable
    }) {
        let key = name_key(&table.name);
        if tables.contains_key(&key) {
            continue;
        }
        let start = Instant::now();
        let batch = pbi_dax::compute_calculated_table(&engine.snapshot(), eval_model, &table.name)
            .map_err(|e| {
                CommandError::Rejected(format!("table '{}' cannot be calculated: {e}", table.name))
            })?;
        let rows = batch.num_rows();
        engine.load_table(&table.name, batch).map_err(|e| {
            CommandError::Rejected(format!("table '{}' cannot be stored: {e}", table.name))
        })?;
        tables.insert(
            key,
            TableRefresh {
                name: table.name.clone(),
                kind: RefreshKind::Calculated,
                rows,
                duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
                status: TableStatus::Ok,
                columns: Vec::new(),
            },
        );
    }
    Ok(())
}
/// Computes the calculated columns of `new` that `old` did not have (by
/// object id) and that have no stored data, in model order, so a column can
/// use one created before it in the same edit. `eval_model` is `new` with
/// the session's derived columns.
pub(crate) fn compute_created_columns(
    engine: &mut Engine,
    old: &Model,
    new: &Model,
    eval_model: &Model,
) -> Result<(), CommandError> {
    let existing: HashSet<ObjectId> = old
        .tables
        .iter()
        .flat_map(|t| t.columns.iter().map(|c| c.id))
        .collect();
    for table in &new.tables {
        for column in table
            .columns
            .iter()
            .filter(|c| c.is_calculated() && c.id.is_assigned() && !existing.contains(&c.id))
        {
            let Ok(data) = engine.table(&table.name) else {
                continue;
            };
            if data.has_column(&column.name) {
                continue;
            }
            let what = dax_column_ref(&table.name, &column.name);
            let array = pbi_dax::compute_calculated_column(
                &engine.snapshot(),
                eval_model,
                &table.name,
                &column.name,
            )
            .map_err(|e| CommandError::Rejected(format!("{what} cannot be calculated: {e}")))?;
            engine
                .add_column(&table.name, &column.name, array)
                .map_err(|e| CommandError::Rejected(format!("{what} cannot be stored: {e}")))?;
        }
    }
    Ok(())
}

impl Session {
    /// The data type a calculated column `name` with DAX `expression` would
    /// have on `table`, from its values over the loaded data (as Power BI
    /// Desktop infers it). `None` when it cannot be inferred: the table is
    /// unknown, not loaded or empty, or every value is blank.
    ///
    /// # Errors
    /// [`CommandError::NameConflict`] when the table has a column or measure
    /// named `name`; [`CommandError::Rejected`] with the DAX error when the
    /// expression does not parse or fails on a row.
    pub fn infer_column_type(
        &self,
        table: &str,
        name: &str,
        expression: &str,
    ) -> Result<Option<DataType>, CommandError> {
        let (snapshot, model) = self.base_snapshot_and_model();
        let mut model = model.as_ref().clone();
        let Some(t) = model.table_mut(table) else {
            return Ok(None);
        };
        let taken = t
            .column(name)
            .map(|c| format!("column '{}'[{}]", t.name, c.name))
            .or_else(|| {
                t.measure(name)
                    .map(|m| format!("measure '{}'[{}]", t.name, m.name))
            });
        if let Some(existing) = taken {
            return Err(CommandError::NameConflict {
                name: name.to_owned(),
                existing,
            });
        }
        if snapshot.table(&t.name).map_or(true, |d| d.row_count() == 0) {
            return Ok(None);
        }
        let table_name = t.name.clone();
        t.columns
            .push(Column::calculated(name, DataType::Automatic, expression));
        let array = pbi_dax::compute_calculated_column(&snapshot, &model, &table_name, name)
            .map_err(|e| {
                CommandError::Rejected(format!(
                    "{} cannot be calculated: {e}",
                    dax_column_ref(&table_name, name)
                ))
            })?;
        if array.null_count() == array.len() {
            return Ok(None);
        }
        Ok(match data_type_of(array.data_type()) {
            DataType::Variant => None,
            other => Some(other),
        })
    }
}

/// A column of a new calculated table, from its expression's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InferredColumn {
    /// The column's name: `Category` for `Products[Category]`.
    pub name: String,
    /// Where the values come from, as Power BI Desktop writes a calculated
    /// table column's `sourceColumn`: `Products[Category]` or `[Total]`.
    pub source_column: String,
    pub data_type: DataType,
}

impl Session {
    /// The columns a calculated table `name` with DAX `expression` would
    /// have, in order, with data types from the values over the loaded data
    /// (else from the model column they come from), as Power BI Desktop
    /// infers them.
    ///
    /// # Errors
    /// [`CommandError::NameConflict`] when the model has a table named
    /// `name`; [`CommandError::Rejected`] when the expression does not parse
    /// or fails, or a column's type cannot be inferred (no rows or only
    /// blanks, and not a model column).
    pub fn infer_table_columns(
        &self,
        name: &str,
        expression: &str,
    ) -> Result<Vec<InferredColumn>, CommandError> {
        let (snapshot, model) = self.base_snapshot_and_model();
        if let Some(other) = model.table(name) {
            return Err(CommandError::NameConflict {
                name: name.to_owned(),
                existing: format!("table '{}'", other.name),
            });
        }
        let table = pbi_dax::evaluate_table(&snapshot, &model, expression, &FilterContext::new())
            .map_err(|e| {
            CommandError::Rejected(format!("table '{name}' cannot be calculated: {e}"))
        })?;
        let batch = pbi_dax::to_record_batch(&table);
        let mut out = Vec::with_capacity(table.columns().len());
        for (column, array) in table.columns().iter().zip(batch.columns()) {
            let from_values = (array.null_count() < array.len())
                .then(|| data_type_of(array.data_type()))
                .filter(|t| *t != DataType::Variant);
            let from_model = || {
                let lineage = column.lineage.as_ref()?;
                let data_type = &model
                    .table(&lineage.table)?
                    .column(&lineage.column)?
                    .data_type;
                (!matches!(data_type, DataType::Variant | DataType::Automatic))
                    .then(|| data_type.clone())
            };
            let Some(data_type) = from_values.or_else(from_model) else {
                return Err(CommandError::Rejected(format!(
                    "table '{name}': the type of column [{}] cannot be inferred: the expression returns no rows or only blanks (refresh the data first when it is not loaded)",
                    column.name
                )));
            };
            out.push(InferredColumn {
                name: column.name.clone(),
                source_column: match &column.table {
                    Some(t) => dax_column_ref(t, &column.name),
                    None => format!("[{}]", column.name.replace(']', "]]")),
                },
                data_type,
            });
        }
        Ok(out)
    }
}
