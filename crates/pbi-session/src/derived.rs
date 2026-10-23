//! Session derived columns: columns the explorer adds to a loaded table
//! ("Parse people field", a DAX expression, or an Arrow array) without
//! changing the document.
//!
//! A derived column is stored in the engine table like any other column, so
//! DAX (queries, other derived expressions), SQL, the grid, column
//! statistics and table exports all see it. The session model
//! ([`Session::model`]) gets a synthetic column entry for it (kind
//! `Calculated`, annotation [`DERIVED_ANNOTATION`]` = true`), so DAX can
//! resolve it by name. The document itself ([`Session::document`],
//! [`Session::base_model`]) never contains derived columns, and the model
//! exports of this crate ([`model_doc`](crate::model_doc),
//! [`model_to_markdown`](crate::model_to_markdown)) skip columns carrying the
//! annotation, so they are never written back to a model file.
//!
//! Derived columns are registered in the session, in creation order, and
//! re-applied after every refresh, cache load and CSV load (an expression
//! can use derived columns created before it). A derived column whose
//! source column is no longer loaded, whose DAX fails or whose array no
//! longer matches the table's row count is taken out of the table data and
//! reported with that status ([`DerivedColumnInfo::active`] is `false`); it
//! stays registered and comes back on the next run where it computes again.
//! [`Session::remove_derived_column`] removes it for good.

use std::sync::Arc;

use arrow::array::ArrayRef;
use arrow::datatypes::DataType as ArrowType;
use pbi_dax::{DaxError, EvalContext};
use pbi_engine::value::canonical_type;
use pbi_engine::{Engine, EngineError, EngineSnapshot};
use pbi_model::props::find_annotation;
use pbi_model::{Annotation, Column, DataType, Model, names_eq};
use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

use crate::error::SessionError;
use crate::refresh::runtime;
use crate::report::{DAX_UNSUPPORTED_HINT, TableStatus};
use crate::session::Session;
use crate::transforms::people::people_array;

/// Annotation (`"true"`) on the synthetic model column of a derived column.
pub const DERIVED_ANNOTATION: &str = "pbiview.derived";
/// Annotation with the [`DerivedKind`] of a derived column (`people`,
/// `expression`, `array`).
pub const DERIVED_KIND_ANNOTATION: &str = "pbiview.derivedKind";

/// `true` if `column` is the synthetic model entry of a session derived
/// column (annotation [`DERIVED_ANNOTATION`]` = true`). Such columns must not
/// be written to model files.
#[must_use]
pub fn is_derived(column: &Column) -> bool {
    find_annotation(&column.annotations, DERIVED_ANNOTATION)
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
}

/// How a derived column is computed.
#[derive(Debug, Clone)]
pub enum DerivedColumn {
    /// "Parse people field": `Name; …` from a text column of the same table
    /// holding Python-style lists of people (see [`crate::transforms::people`]).
    People {
        /// The text column to parse.
        source_column: String,
    },
    /// A DAX calculated-column expression, evaluated per row of the table
    /// (row context, no filter), like a model calculated column. The data
    /// type is inferred from the values.
    Expression {
        /// The DAX expression.
        dax: String,
    },
    /// Fixed values, matched to the table's rows by position. After a
    /// refresh that changes the row count it is reported as an error and
    /// left out of the table.
    Array(ArrayRef),
}

impl DerivedColumn {
    /// The kind.
    #[must_use]
    pub fn kind(&self) -> DerivedKind {
        match self {
            DerivedColumn::People { .. } => DerivedKind::People,
            DerivedColumn::Expression { .. } => DerivedKind::Expression,
            DerivedColumn::Array(_) => DerivedKind::Array,
        }
    }

    /// Text stored as the synthetic model column's expression.
    fn description(&self) -> String {
        match self {
            DerivedColumn::People { source_column } => {
                format!(
                    "Parse people field ([{}])",
                    source_column.replace(']', "]]")
                )
            }
            DerivedColumn::Expression { dax } => dax.clone(),
            DerivedColumn::Array(array) => format!("<array of {} values>", array.len()),
        }
    }
}

/// Kind of a [`DerivedColumn`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DerivedKind {
    /// [`DerivedColumn::People`].
    People,
    /// [`DerivedColumn::Expression`].
    Expression,
    /// [`DerivedColumn::Array`].
    Array,
}

impl DerivedKind {
    /// `people`, `expression` or `array`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            DerivedKind::People => "people",
            DerivedKind::Expression => "expression",
            DerivedKind::Array => "array",
        }
    }
}

/// A registered derived column and its current state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivedColumnInfo {
    /// Table (model spelling).
    pub table: String,
    /// Column name.
    pub name: String,
    /// How it is computed.
    pub kind: DerivedKind,
    /// The parsed column of a [`DerivedKind::People`] column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_column: Option<String>,
    /// The DAX of a [`DerivedKind::Expression`] column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dax: Option<String>,
    /// Data type of the stored values (inferred).
    pub data_type: DataType,
    /// The column is in the table data now.
    pub active: bool,
    /// Outcome of the last computation (on add, then after each refresh /
    /// cache load / CSV load).
    pub status: TableStatus,
}

/// A registry entry: the definition and its state.
#[derive(Debug, Clone)]
pub(crate) struct DerivedEntry {
    pub info: DerivedColumnInfo,
    pub column: DerivedColumn,
}

/// Why a derived column could not be computed.
pub(crate) enum Failure {
    Status(TableStatus),
    Cancelled,
}

fn error(message: impl Into<String>) -> Failure {
    Failure::Status(TableStatus::Error {
        message: message.into(),
        unsupported: false,
        hint: None,
        csv_fallback: false,
    })
}

/// The model data type of values stored as `t`.
pub(crate) fn data_type_of(t: &ArrowType) -> DataType {
    match canonical_type(t) {
        Some(ArrowType::Boolean) => DataType::Boolean,
        Some(ArrowType::Int64) => DataType::Int64,
        Some(ArrowType::Float64) => DataType::Double,
        Some(ArrowType::Decimal128(..)) => DataType::Decimal,
        Some(ArrowType::Timestamp(..)) => DataType::DateTime,
        Some(ArrowType::Utf8) => DataType::String,
        _ => DataType::Variant,
    }
}

/// The synthetic model column of a computed derived column.
pub(crate) fn synthetic_column(info: &DerivedColumnInfo, column: &DerivedColumn) -> Column {
    let mut c = Column::calculated(&info.name, info.data_type.clone(), column.description());
    c.description = Some("Session-only derived column (DashCraft); never saved.".to_owned());
    c.annotations = vec![
        Annotation::new(DERIVED_ANNOTATION, "true"),
        Annotation::new(DERIVED_KIND_ANNOTATION, info.kind.as_str()),
    ];
    c
}

/// Computes `table[name]` on `snapshot` (blocking). `model` is the session
/// model without this column.
pub(crate) fn compute(
    snapshot: Arc<EngineSnapshot>,
    model: &Model,
    table: &str,
    name: &str,
    column: &DerivedColumn,
    cancel: &CancellationToken,
) -> Result<ArrayRef, Failure> {
    if cancel.is_cancelled() {
        return Err(Failure::Cancelled);
    }
    let data = snapshot
        .table(table)
        .map_err(|e| error(format!("the table is not available: {e}")))?;
    match column {
        DerivedColumn::People { source_column } => {
            let Ok(source) = data.column(source_column) else {
                return Err(Failure::Status(TableStatus::Skipped {
                    reason: format!("its source column [{source_column}] is not loaded"),
                }));
            };
            people_array(source).ok_or_else(|| {
                error(format!(
                    "\"Parse people field\" needs a text column; [{source_column}] is {}",
                    source.data_type()
                ))
            })
        }
        DerivedColumn::Array(array) => {
            let rows = data.row_count();
            if array.len() == rows {
                Ok(Arc::clone(array))
            } else {
                Err(error(format!(
                    "the array has {} values but '{}' has {rows} rows",
                    array.len(),
                    data.name()
                )))
            }
        }
        DerivedColumn::Expression { dax } => {
            let mut temp = model.clone();
            let Some(t) = temp.table_mut(table) else {
                return Err(error(format!("table '{table}' is not in the model")));
            };
            t.columns
                .push(Column::calculated(name, DataType::Automatic, dax.clone()));
            let ctx =
                EvalContext::new(snapshot, Arc::new(temp)).with_cancellation_token(cancel.clone());
            match ctx.compute_calculated_column(table, name) {
                Ok(array) => Ok(array),
                Err(DaxError::Cancelled) => Err(Failure::Cancelled),
                Err(e) => Err(Failure::Status(TableStatus::Error {
                    message: e.to_string(),
                    unsupported: e.is_unsupported(),
                    hint: e.is_unsupported().then(|| DAX_UNSUPPORTED_HINT.to_owned()),
                    csv_fallback: false,
                })),
            }
        }
    }
}

/// Removes `table[column]` from the engine if it is stored. An unloaded
/// table stays unloaded (it is rebuilt from `base` plus its other extra
/// columns).
pub(crate) fn drop_column(
    engine: &mut Engine,
    base: &Model,
    table: &str,
    column: &str,
) -> Result<(), EngineError> {
    let data = Arc::clone(engine.table(table)?);
    let Some(index) = data.column_index(column) else {
        return Ok(());
    };
    let keep: Vec<usize> = (0..data.batch().num_columns())
        .filter(|&i| i != index)
        .collect();
    let projected = data.batch().project(&keep)?;
    if data.is_loaded() {
        return engine.replace_table(data.name(), projected);
    }
    engine.remove_table(data.name());
    engine.sync_model(base);
    let base_table = base.table(data.name());
    for (field, array) in projected.schema().fields().iter().zip(projected.columns()) {
        if base_table.is_none_or(|t| t.column(field.name()).is_none()) {
            engine.add_column(data.name(), field.name(), Arc::clone(array))?;
        }
    }
    Ok(())
}

/// The derived columns after re-applying `entries` (in order) to `engine`,
/// and the session model (`base` plus the active ones).
pub(crate) struct Applied {
    pub model: Arc<Model>,
    pub entries: Vec<DerivedEntry>,
}

/// Re-applies every derived column to `engine`: stale copies are dropped
/// first, then each column is computed in creation order (so expressions
/// see the derived columns created before them).
pub(crate) async fn reapply(
    handle: &Handle,
    engine: &mut Engine,
    base: &Arc<Model>,
    entries: Vec<DerivedEntry>,
    cancel: &CancellationToken,
) -> Result<Applied, SessionError> {
    if entries.is_empty() {
        return Ok(Applied {
            model: Arc::clone(base),
            entries,
        });
    }
    for e in &entries {
        if let Err(err) = drop_column(engine, base, &e.info.table, &e.info.name) {
            tracing::warn!(table = %e.info.table, column = %e.info.name, error = %err, "cannot drop a stale derived column");
        }
    }
    let mut model = (**base).clone();
    let mut out = Vec::with_capacity(entries.len());
    for mut entry in entries {
        let snapshot = engine.snapshot();
        let current = Arc::new(model.clone());
        let (table, name, column, token) = (
            entry.info.table.clone(),
            entry.info.name.clone(),
            entry.column.clone(),
            cancel.clone(),
        );
        let result = handle
            .spawn_blocking(move || compute(snapshot, &current, &table, &name, &column, &token))
            .await?;
        let status = match result {
            Ok(array) => {
                let data_type = data_type_of(array.data_type());
                match engine.add_column(&entry.info.table, &entry.info.name, array) {
                    Ok(()) => {
                        entry.info.data_type = data_type;
                        if let Some(t) = model.table_mut(&entry.info.table) {
                            t.columns.push(synthetic_column(&entry.info, &entry.column));
                        }
                        TableStatus::Ok
                    }
                    Err(e) => TableStatus::Error {
                        message: format!("cannot store the column: {e}"),
                        unsupported: false,
                        hint: None,
                        csv_fallback: false,
                    },
                }
            }
            Err(Failure::Cancelled) => return Err(SessionError::Cancelled),
            Err(Failure::Status(status)) => status,
        };
        entry.info.active = status.is_ok();
        entry.info.status = status;
        out.push(entry);
    }
    Ok(Applied {
        model: Arc::new(model),
        entries: out,
    })
}

impl Session {
    /// Adds the derived column `table[name]` (see the [module docs](self)):
    /// computes it on the current data, stores it in the table and registers
    /// it so it is re-applied after every refresh, cache load and CSV load.
    /// Waits for a running refresh to finish.
    ///
    /// # Errors
    /// [`SessionError::UnknownTable`]; [`SessionError::Derived`] for an empty
    /// name, a name already used by a column of the table or a measure, a
    /// missing or non-text people source column, a DAX error, or an array
    /// whose length is not the table's row count; [`SessionError::Cancelled`]
    /// if the future is dropped.
    pub async fn add_derived_column(
        &self,
        table: &str,
        name: &str,
        column: DerivedColumn,
    ) -> Result<DerivedColumnInfo, SessionError> {
        let handle = runtime()?;
        let base = self.base_model();
        let table_name = base
            .table(table)
            .map(|t| t.name.clone())
            .ok_or_else(|| SessionError::UnknownTable(table.to_owned()))?;
        let name = name.trim().to_owned();
        let fail = |reason: String| SessionError::Derived {
            table: table_name.clone(),
            column: name.clone(),
            reason,
        };
        if name.is_empty() {
            return Err(fail("the name is empty".to_owned()));
        }
        if name.chars().any(char::is_control) {
            return Err(fail("the name contains control characters".to_owned()));
        }
        let _guard = self.refresh_lock().lock().await;
        let (snapshot, model) = self.base_snapshot_and_model();
        let stored = snapshot
            .table(&table_name)
            .is_ok_and(|d| d.has_column(&name));
        if stored || model.column(&table_name, &name).is_some() {
            return Err(fail(format!(
                "the table already has a column named '{name}'"
            )));
        }
        if let Some((t, _)) = model.tables.iter().find_map(|t| {
            t.measures
                .iter()
                .find(|m| names_eq(&m.name, &name))
                .map(|m| (t, m))
        }) {
            return Err(fail(format!(
                "a measure named [{name}] exists in '{}'",
                t.name
            )));
        }
        let cancel = CancellationToken::new();
        let _stop_on_drop = cancel.clone().drop_guard();
        let (t, n, c, m, token) = (
            table_name.clone(),
            name.clone(),
            column.clone(),
            Arc::clone(&model),
            cancel.clone(),
        );
        let result = handle
            .spawn_blocking(move || compute(snapshot, &m, &t, &n, &c, &token))
            .await?;
        let array = match result {
            Ok(array) => array,
            Err(Failure::Cancelled) => return Err(SessionError::Cancelled),
            Err(Failure::Status(status)) => {
                return Err(fail(status.message().unwrap_or("failed").to_owned()));
            }
        };
        let info = DerivedColumnInfo {
            table: table_name.clone(),
            name: name.clone(),
            kind: column.kind(),
            source_column: match &column {
                DerivedColumn::People { source_column } => Some(source_column.clone()),
                _ => None,
            },
            dax: match &column {
                DerivedColumn::Expression { dax } => Some(dax.clone()),
                _ => None,
            },
            data_type: data_type_of(array.data_type()),
            active: true,
            status: TableStatus::Ok,
        };
        let synthetic = synthetic_column(&info, &column);
        self.update_derived(|engine, model, entries| {
            engine.add_column(&table_name, &name, array)?;
            let mut next = (**model).clone();
            if let Some(t) = next.table_mut(&table_name) {
                t.columns.push(synthetic);
            }
            *model = Arc::new(next);
            entries.push(DerivedEntry {
                info: info.clone(),
                column,
            });
            Ok::<_, SessionError>(())
        })?;
        Ok(info)
    }

    /// Removes the derived column `table[name]` (names case-insensitive)
    /// from the table data, the session model and the registry. Derived
    /// expressions that use it fail on the next refresh. Waits for a
    /// running refresh to finish.
    ///
    /// # Errors
    /// [`SessionError::Derived`] if `table[name]` is not a derived column.
    pub async fn remove_derived_column(
        &self,
        table: &str,
        name: &str,
    ) -> Result<DerivedColumnInfo, SessionError> {
        let _guard = self.refresh_lock().lock().await;
        let base = self.base_model();
        self.update_derived(|engine, model, entries| {
            let index = entries
                .iter()
                .position(|e| names_eq(&e.info.table, table) && names_eq(&e.info.name, name.trim()))
                .ok_or_else(|| SessionError::Derived {
                    table: table.to_owned(),
                    column: name.to_owned(),
                    reason: "not a derived column".to_owned(),
                })?;
            if let Some(e) = entries.get(index) {
                drop_column(engine, &base, &e.info.table, &e.info.name)?;
            }
            let entry = entries.remove(index);
            let mut next = (**model).clone();
            if let Some(t) = next.table_mut(&entry.info.table) {
                t.columns
                    .retain(|c| !(is_derived(c) && names_eq(&c.name, &entry.info.name)));
            }
            *model = Arc::new(next);
            Ok(entry.info)
        })
    }

    /// Every registered derived column, in creation order, with its state.
    #[must_use]
    pub fn derived_columns(&self) -> Vec<DerivedColumnInfo> {
        self.derived_entries().into_iter().map(|e| e.info).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_types_are_inferred_from_arrow() {
        assert_eq!(data_type_of(&ArrowType::Utf8), DataType::String);
        assert_eq!(data_type_of(&ArrowType::Int32), DataType::Int64);
        assert_eq!(data_type_of(&ArrowType::Float64), DataType::Double);
        assert_eq!(data_type_of(&ArrowType::Boolean), DataType::Boolean);
        assert_eq!(data_type_of(&ArrowType::Binary), DataType::Variant);
    }

    #[test]
    fn synthetic_columns_are_marked() {
        let info = DerivedColumnInfo {
            table: "T".into(),
            name: "Managers (parsed)".into(),
            kind: DerivedKind::People,
            source_column: Some("Managers".into()),
            dax: None,
            data_type: DataType::String,
            active: true,
            status: TableStatus::Ok,
        };
        let column = DerivedColumn::People {
            source_column: "Managers".into(),
        };
        let c = synthetic_column(&info, &column);
        assert!(is_derived(&c));
        assert!(c.is_calculated());
        assert_eq!(c.expression(), Some("Parse people field ([Managers])"));
        assert!(!is_derived(&Column::calculated("X", DataType::Int64, "1")));
    }
}
