//! Data types of calculated tables and columns before they are created or
//! changed, inferred from their values over the loaded data (their values
//! are computed when the edit is published: [`crate::recalc`]).

use pbi_engine::{EngineSnapshot, FilterContext};
use pbi_model::names::{dax_column_ref, names_eq};
use pbi_model::{Column, ColumnKind, CommandError, DataType, Model, TableKind};

use crate::derived::data_type_of;
use crate::session::Session;

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
        let table_name = t.name.clone();
        t.columns
            .push(Column::calculated(name, DataType::Automatic, expression));
        infer_type(&snapshot, &model, &table_name, name)
    }

    /// The data type the calculated column `column` of `table` would have
    /// with DAX `expression` instead of its own, from its values over the
    /// loaded data (as Power BI Desktop infers it when a calculated column's
    /// formula changes). `None` when it cannot be inferred: the table or
    /// column is unknown, the table is not loaded or empty, or every value
    /// is blank.
    ///
    /// # Errors
    /// [`CommandError::Rejected`] when the column is not a calculated column,
    /// or with the DAX error when the expression fails on a row (a circular
    /// dependency included).
    pub fn infer_changed_column_type(
        &self,
        table: &str,
        column: &str,
        expression: &str,
    ) -> Result<Option<DataType>, CommandError> {
        let (snapshot, model) = self.base_snapshot_and_model();
        let mut model = model.as_ref().clone();
        let Some(t) = model.table_mut(table) else {
            return Ok(None);
        };
        let table_name = t.name.clone();
        let Some(c) = t.columns.iter_mut().find(|c| names_eq(&c.name, column)) else {
            return Ok(None);
        };
        let ColumnKind::Calculated { expression: slot } = &mut c.kind else {
            return Err(CommandError::Rejected(format!(
                "{} is not a calculated column: its values and data type come from its query",
                dax_column_ref(&table_name, &c.name)
            )));
        };
        expression.clone_into(slot);
        c.data_type = DataType::Automatic;
        let column_name = c.name.clone();
        infer_type(&snapshot, &model, &table_name, &column_name)
    }
}

/// The data type of the calculated column `name` of `table` in `model`, from
/// its values over `snapshot` (`None` without rows or with only blanks).
fn infer_type(
    snapshot: &EngineSnapshot,
    model: &Model,
    table: &str,
    name: &str,
) -> Result<Option<DataType>, CommandError> {
    if snapshot.table(table).map_or(true, |d| d.row_count() == 0) {
        return Ok(None);
    }
    let array = pbi_dax::compute_calculated_column(snapshot, model, table, name).map_err(|e| {
        CommandError::Rejected(format!(
            "{} cannot be calculated: {e}",
            dax_column_ref(table, name)
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
        infer_columns(&snapshot, &model, name, expression)
    }

    /// The columns the calculated table `table` would have with DAX
    /// `expression` instead of its own, as [`Session::infer_table_columns`]
    /// infers them. The expression runs on the current data, the table's
    /// included: an expression that uses the table is refused by the edit
    /// (a circular dependency).
    ///
    /// # Errors
    /// [`CommandError::Rejected`] when the model has no calculated table
    /// `table`, or as [`Session::infer_table_columns`].
    pub fn infer_changed_table_columns(
        &self,
        table: &str,
        expression: &str,
    ) -> Result<Vec<InferredColumn>, CommandError> {
        let (snapshot, model) = self.base_snapshot_and_model();
        let Some(t) = model.table(table) else {
            return Err(CommandError::Rejected(format!("no table '{table}'")));
        };
        if t.kind() != TableKind::CalculatedTable {
            return Err(CommandError::Rejected(format!(
                "'{}' is not a calculated table: its columns come from its query",
                t.name
            )));
        }
        infer_columns(&snapshot, &model, &t.name, expression)
    }
}

/// The columns of a calculated table `name` with DAX `expression` (see
/// [`Session::infer_table_columns`]).
fn infer_columns(
    snapshot: &EngineSnapshot,
    model: &Model,
    name: &str,
    expression: &str,
) -> Result<Vec<InferredColumn>, CommandError> {
    let table = pbi_dax::evaluate_table(snapshot, model, expression, &FilterContext::new())
        .map_err(|e| CommandError::Rejected(format!("table '{name}' cannot be calculated: {e}")))?;
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
            // Desktop's form: the result's column name, the table unquoted
            // (`Web Sessions[Device]`, `[Pages]`).
            source_column: match &column.table {
                Some(t) => format!("{t}[{}]", column.name.replace(']', "]]")),
                None => format!("[{}]", column.name.replace(']', "]]")),
            },
            data_type,
        });
    }
    Ok(out)
}
