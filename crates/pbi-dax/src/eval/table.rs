//! Virtual tables: the values of DAX table expressions.
//!
//! A [`VirtualTable`] has named columns, each optionally carrying the
//! *lineage* of a model column (so `FILTER`/`VALUES`/`SUMMARIZE` results can
//! be applied as `CALCULATE` filters), and rows that are either
//!
//! - **base-backed**: rows of a stored table referenced by row number (plus
//!   the table's virtual blank row), optionally with extra computed columns
//!   (`ADDCOLUMNS`). Row identity is kept, so `RELATED`, context transition
//!   and table filters work on them; or
//! - **materialized**: rows of [`Value`]s (`SUMMARIZE`, `SELECTCOLUMNS`,
//!   table constructors, `VALUES(column)` …).

use std::collections::HashMap;
use std::sync::Arc;

use arrow::array::ArrayRef;
use arrow::datatypes::{DataType as ArrowType, Field, Schema};
use arrow::record_batch::{RecordBatch, RecordBatchOptions};
use pbi_engine::value::{infer_arrow_type, values_to_array};
use pbi_engine::{RowSet, TableData, Value, ValueKey};
use pbi_model::ColumnRef;
use pbi_model::names::names_eq;

/// Row number of a table's virtual blank row in a base-backed table.
pub(crate) const BLANK_ROW: usize = usize::MAX;

/// A column of a [`VirtualTable`].
#[derive(Debug, Clone, PartialEq)]
pub struct VColumn {
    /// Qualifying table (`'Web Sessions'` in `'Web Sessions'[Device]`); `None` for
    /// columns created by an expression (`"Total", [Total]`).
    pub table: Option<String>,
    pub name: String,
    /// The model column the values come from, if any.
    pub lineage: Option<ColumnRef>,
    /// Storage type when known (model columns); otherwise inferred.
    pub data_type: Option<ArrowType>,
}

impl VColumn {
    /// A computed column without lineage.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            table: None,
            name: name.into(),
            lineage: None,
            data_type: None,
        }
    }

    /// A model column `table[name]` (with lineage).
    pub fn model(table: &str, name: &str, data_type: Option<ArrowType>) -> Self {
        Self {
            table: Some(table.to_owned()),
            name: name.to_owned(),
            lineage: Some(ColumnRef::new(table, name)),
            data_type,
        }
    }

    /// `Table[Column]` for qualified columns, `[Column]` otherwise (the
    /// naming of DAX query results).
    pub fn qualified_name(&self) -> String {
        match &self.table {
            Some(t) => format!("{t}[{}]", self.name),
            None => format!("[{}]", self.name),
        }
    }

    fn is_exactly(&self, table: Option<&str>, name: &str) -> bool {
        if !names_eq(&self.name, name) {
            return false;
        }
        match (table, &self.table) {
            (None, _) => true,
            (Some(t), Some(own)) => names_eq(t, own),
            (Some(_), None) => false,
        }
    }

    fn has_lineage(&self, table: &str, name: &str) -> bool {
        self.lineage
            .as_ref()
            .is_some_and(|l| l.matches(table, name))
    }
}

/// Position of a row in a [`VirtualTable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RowPos {
    /// Row of the base table ([`BLANK_ROW`] for the blank row), for
    /// set-backed tables.
    Base(usize),
    /// Index into the row list.
    Item(usize),
}

/// One row of a list-backed table.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VRow {
    /// Base row (only meaningful for base-backed tables).
    pub base: usize,
    /// Values of the non-base columns.
    pub values: Vec<Value>,
}

#[derive(Debug, Clone)]
pub(crate) enum Rows {
    /// Base-backed rows without extra columns.
    Set { rows: RowSet, blank_row: bool },
    /// Explicit rows (ordered).
    List(Vec<VRow>),
}

/// The value of a DAX table expression. Cheap to clone (row storage is
/// shared through `Arc`s where it matters).
#[derive(Debug, Clone)]
pub struct VirtualTable {
    pub(crate) columns: Arc<[VColumn]>,
    pub(crate) base: Option<Arc<TableData>>,
    pub(crate) rows: Rows,
}

impl VirtualTable {
    /// Rows of a stored table (all its columns).
    pub(crate) fn from_base(data: Arc<TableData>, rows: RowSet, blank_row: bool) -> Self {
        let columns = base_columns(&data);
        let rows = rows.resized(data.row_count());
        Self {
            columns,
            base: Some(data),
            rows: Rows::Set { rows, blank_row },
        }
    }

    /// A materialized table. Every row must have one value per column.
    pub(crate) fn materialized(columns: Vec<VColumn>, rows: Vec<Vec<Value>>) -> Self {
        Self {
            columns: columns.into(),
            base: None,
            rows: Rows::List(
                rows.into_iter()
                    .map(|values| VRow { base: 0, values })
                    .collect(),
            ),
        }
    }

    /// An empty table with the given columns.
    pub(crate) fn empty(columns: Vec<VColumn>) -> Self {
        Self::materialized(columns, Vec::new())
    }

    pub fn columns(&self) -> &[VColumn] {
        &self.columns
    }

    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    /// Number of rows (the blank row counts when present).
    pub fn row_count(&self) -> usize {
        match &self.rows {
            Rows::Set { rows, blank_row } => rows.count() + usize::from(*blank_row),
            Rows::List(list) => list.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.row_count() == 0
    }

    /// Name of the stored table the rows belong to, for base-backed tables.
    pub fn base_table(&self) -> Option<&str> {
        self.base.as_deref().map(TableData::name)
    }

    /// All rows as values (column order of [`VirtualTable::columns`]).
    pub fn rows(&self) -> Vec<Vec<Value>> {
        self.positions()
            .into_iter()
            .map(|p| self.row_values(p))
            .collect()
    }

    /// Index of a column: `table[name]` matches the qualified name first,
    /// then the lineage; `[name]` (no table) matches by name.
    pub fn find_column(&self, table: Option<&str>, name: &str) -> Option<usize> {
        self.find_exact(table, name)
            .or_else(|| table.and_then(|t| self.find_by_lineage(t, name)))
    }

    pub(crate) fn find_exact(&self, table: Option<&str>, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.is_exactly(table, name))
    }

    pub(crate) fn find_by_lineage(&self, table: &str, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.has_lineage(table, name))
    }

    /// Row positions in table order.
    pub(crate) fn positions(&self) -> Vec<RowPos> {
        match &self.rows {
            Rows::Set { rows, blank_row } => {
                let mut out: Vec<RowPos> = rows.iter().map(RowPos::Base).collect();
                if *blank_row {
                    out.push(RowPos::Base(BLANK_ROW));
                }
                out
            }
            Rows::List(list) => (0..list.len()).map(RowPos::Item).collect(),
        }
    }

    /// Base row of a position ([`BLANK_ROW`] for the blank row).
    pub(crate) fn base_row(&self, pos: RowPos) -> usize {
        match (pos, &self.rows) {
            (RowPos::Base(r), _) => r,
            (RowPos::Item(i), Rows::List(list)) => list.get(i).map_or(BLANK_ROW, |r| r.base),
            (RowPos::Item(_), Rows::Set { .. }) => BLANK_ROW,
        }
    }

    fn extras(&self, pos: RowPos) -> &[Value] {
        match (pos, &self.rows) {
            (RowPos::Item(i), Rows::List(list)) => {
                list.get(i).map_or(&[][..], |r| r.values.as_slice())
            }
            _ => &[],
        }
    }

    /// Value at a row position and column index (BLANK when out of range).
    pub(crate) fn value(&self, pos: RowPos, col: usize) -> Value {
        if let Some(data) = &self.base {
            let width = data.batch().num_columns();
            if col < width {
                let row = self.base_row(pos);
                if row == BLANK_ROW {
                    return Value::Blank;
                }
                return data
                    .encoded_at(col)
                    .ok()
                    .and_then(|e| e.value_at(row).cloned())
                    .unwrap_or_default();
            }
            return self
                .extras(pos)
                .get(col - width)
                .cloned()
                .unwrap_or_default();
        }
        self.extras(pos).get(col).cloned().unwrap_or_default()
    }

    pub(crate) fn row_values(&self, pos: RowPos) -> Vec<Value> {
        (0..self.columns.len())
            .map(|c| self.value(pos, c))
            .collect()
    }

    /// Strict (`==`) equality key of a row.
    pub(crate) fn row_key(&self, pos: RowPos) -> Vec<ValueKey> {
        (0..self.columns.len())
            .map(|c| ValueKey::from(self.value(pos, c)))
            .collect()
    }

    /// The rows at `keep` (same structure: a set-backed table stays one).
    pub(crate) fn subset(&self, keep: &[RowPos]) -> VirtualTable {
        let rows = match &self.rows {
            Rows::Set { rows, .. } => {
                let mut set = RowSet::empty(rows.universe());
                let mut blank = false;
                for p in keep {
                    match p {
                        RowPos::Base(BLANK_ROW) => blank = true,
                        RowPos::Base(r) => {
                            set.insert(*r);
                        }
                        RowPos::Item(_) => {}
                    }
                }
                Rows::Set {
                    rows: set,
                    blank_row: blank,
                }
            }
            Rows::List(list) => Rows::List(
                keep.iter()
                    .filter_map(|p| match p {
                        RowPos::Item(i) => list.get(*i).cloned(),
                        RowPos::Base(_) => None,
                    })
                    .collect(),
            ),
        };
        VirtualTable {
            columns: Arc::clone(&self.columns),
            base: self.base.clone(),
            rows,
        }
    }

    /// The rows at `keep`, in that order (always list-backed).
    pub(crate) fn subset_ordered(&self, keep: &[RowPos]) -> VirtualTable {
        let list = keep
            .iter()
            .map(|p| VRow {
                base: self.base_row(*p),
                values: self.extras(*p).to_vec(),
            })
            .collect();
        VirtualTable {
            columns: Arc::clone(&self.columns),
            base: self.base.clone(),
            rows: Rows::List(list),
        }
    }

    /// Appends computed columns (`values[i]` for row `positions()[i]`).
    pub(crate) fn with_columns(
        &self,
        new_columns: Vec<VColumn>,
        values: Vec<Vec<Value>>,
    ) -> VirtualTable {
        let mut columns: Vec<VColumn> = self.columns.to_vec();
        columns.extend(new_columns);
        let list = self
            .positions()
            .into_iter()
            .zip(values)
            .map(|(p, extra)| {
                let mut row = self.extras(p).to_vec();
                row.extend(extra);
                VRow {
                    base: self.base_row(p),
                    values: row,
                }
            })
            .collect();
        VirtualTable {
            columns: columns.into(),
            base: self.base.clone(),
            rows: Rows::List(list),
        }
    }

    /// The base rows (real rows and blank-row flag) of a base-backed table.
    pub(crate) fn base_rowset(&self) -> Option<(RowSet, bool)> {
        let data = self.base.as_ref()?;
        Some(match &self.rows {
            Rows::Set { rows, blank_row } => (rows.clone(), *blank_row),
            Rows::List(list) => {
                let mut set = RowSet::empty(data.row_count());
                let mut blank = false;
                for r in list {
                    if r.base == BLANK_ROW {
                        blank = true;
                    } else {
                        set.insert(r.base);
                    }
                }
                (set, blank)
            }
        })
    }

    /// Same rows, with every column qualified by `table` (query tables:
    /// `DEFINE TABLE Busy = …` exposes `Busy[Column]`; lineage is kept).
    pub(crate) fn requalified(&self, table: &str) -> VirtualTable {
        let columns: Vec<VColumn> = self
            .columns
            .iter()
            .map(|c| VColumn {
                table: Some(table.to_owned()),
                ..c.clone()
            })
            .collect();
        VirtualTable {
            columns: columns.into(),
            base: self.base.clone(),
            rows: self.rows.clone(),
        }
    }

    /// Materialized copy with new column metadata (same width).
    pub(crate) fn relabeled(&self, columns: Vec<VColumn>) -> VirtualTable {
        VirtualTable::materialized(columns, self.rows())
    }

    /// Distinct rows (first occurrence kept, order preserved).
    pub(crate) fn distinct(&self) -> VirtualTable {
        let mut seen = std::collections::HashSet::new();
        let keep: Vec<RowPos> = self
            .positions()
            .into_iter()
            .filter(|p| seen.insert(self.row_key(*p)))
            .collect();
        if keep.len() == self.row_count() {
            return self.clone();
        }
        self.subset(&keep)
    }
}

fn base_columns(data: &TableData) -> Arc<[VColumn]> {
    data.schema()
        .fields()
        .iter()
        .map(|f| VColumn::model(data.name(), f.name(), Some(f.data_type().clone())))
        .collect()
}

/// Converts a virtual table to an Arrow record batch. Columns are named like
/// DAX query results (`Table[Column]` for model columns, `[Name]` for
/// computed ones); types come from the model column when known, otherwise
/// they are inferred from the values (mixed types become text).
pub fn to_record_batch(table: &VirtualTable) -> RecordBatch {
    let names: Vec<String> = table.columns.iter().map(VColumn::qualified_name).collect();
    record_batch_with_names(table, &names, &[])
}

/// Builds a record batch with explicit column names and optional target
/// types (`types[i]`, falling back to the column's own/inferred type).
pub(crate) fn record_batch_with_names(
    table: &VirtualTable,
    names: &[String],
    types: &[Option<ArrowType>],
) -> RecordBatch {
    let positions = table.positions();
    let mut fields = Vec::with_capacity(table.columns.len());
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(table.columns.len());
    for (i, col) in table.columns.iter().enumerate() {
        let values: Vec<Value> = positions.iter().map(|p| table.value(*p, i)).collect();
        let requested = types.get(i).cloned().flatten();
        let array = column_array(&values, requested.as_ref().or(col.data_type.as_ref()));
        let name = names.get(i).cloned().unwrap_or_else(|| col.name.clone());
        fields.push(Field::new(name, array.data_type().clone(), true));
        arrays.push(array);
    }
    let schema = Arc::new(Schema::new(fields));
    let options = RecordBatchOptions::new().with_row_count(Some(positions.len()));
    RecordBatch::try_new_with_options(Arc::clone(&schema), arrays, &options)
        .unwrap_or_else(|_| RecordBatch::new_empty(schema))
}

/// Array for `values`, preferring `preferred` and falling back to the
/// inferred type, then text. Never fails.
pub(crate) fn column_array(values: &[Value], preferred: Option<&ArrowType>) -> ArrayRef {
    if let Some(t) = preferred
        && let Ok(a) = values_to_array(values, t)
    {
        return a;
    }
    let inferred = infer_arrow_type(values);
    values_to_array(values, &inferred)
        .or_else(|_| values_to_array(values, &ArrowType::Utf8))
        .unwrap_or_else(|_| arrow::array::new_null_array(&ArrowType::Utf8, values.len()))
}

/// Groups row positions by key, preserving first-appearance order.
pub(crate) fn group_positions(
    table: &VirtualTable,
    cols: &[usize],
) -> Vec<(Vec<Value>, Vec<RowPos>)> {
    let mut index: HashMap<Vec<ValueKey>, usize> = HashMap::new();
    let mut groups: Vec<(Vec<Value>, Vec<RowPos>)> = Vec::new();
    for p in table.positions() {
        let values: Vec<Value> = cols.iter().map(|c| table.value(p, *c)).collect();
        let key: Vec<ValueKey> = values.iter().map(ValueKey::from).collect();
        match index.get(&key) {
            Some(&i) => {
                if let Some(g) = groups.get_mut(i) {
                    g.1.push(p);
                }
            }
            None => {
                index.insert(key, groups.len());
                groups.push((values, vec![p]));
            }
        }
    }
    groups
}
