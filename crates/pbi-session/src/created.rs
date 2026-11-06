//! Data of the calculated columns an edit creates.
//!
//! A created calculated column is computed when the edit is published, over
//! the loaded data of its table (a table that is not loaded gets it at the
//! next refresh). Undo and redo re-attach the kept data instead
//! ([`crate::removed`]); only columns without stored data are computed.

use std::collections::HashSet;

use pbi_engine::Engine;
use pbi_model::names::dax_column_ref;
use pbi_model::{Column, CommandError, DataType, Model, ObjectId};

use crate::derived::data_type_of;
use crate::session::Session;

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
