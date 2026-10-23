//! Columnar table values.
//!
//! A [`TableValue`] stores one `Arc`-shared vector of [`Lazy`] cells per column, so column
//! operations (select, rename, reorder, add) share untouched columns. Rows are materialized as
//! [`RecordValue`]s on demand and share the table's [`Keys`]. The abstraction (`row_count`,
//! `column`, `rows`) is what a later streaming / Arrow-backed / query-folding implementation
//! must provide.

use std::fmt;
use std::sync::Arc;

use crate::merror::MError;
use crate::value::{Keys, Lazy, RecordValue, TypeValue, Value};

struct TableData {
    keys: Arc<Keys>,
    types: Vec<TypeValue>,
    columns: Vec<Arc<Vec<Lazy>>>,
    rows: usize,
}

/// An M table.
#[derive(Clone)]
pub struct TableValue(Arc<TableData>);

impl TableValue {
    /// Builds a table from named columns of equal length. `types` may be shorter than `names`
    /// (missing column types are `any`).
    pub fn new(
        names: Vec<Arc<str>>,
        types: Vec<TypeValue>,
        columns: Vec<Arc<Vec<Lazy>>>,
    ) -> Result<Self, MError> {
        let keys = Arc::new(Keys::try_new(names)?);
        Self::from_parts(keys, types, columns)
    }

    pub fn from_parts(
        keys: Arc<Keys>,
        mut types: Vec<TypeValue>,
        columns: Vec<Arc<Vec<Lazy>>>,
    ) -> Result<Self, MError> {
        if columns.len() != keys.len() {
            return Err(MError::Internal(format!(
                "table has {} column names but {} columns",
                keys.len(),
                columns.len()
            )));
        }
        let rows = columns.first().map_or(0, |c| c.len());
        if columns.iter().any(|c| c.len() != rows) {
            return Err(MError::Internal("table columns differ in length".into()));
        }
        types.resize(keys.len(), TypeValue::any());
        Ok(Self(Arc::new(TableData {
            keys,
            types,
            columns,
            rows,
        })))
    }

    /// Builds a table from rows; every row must have one cell per column.
    pub fn from_rows(names: Vec<Arc<str>>, rows: Vec<Vec<Lazy>>) -> Result<Self, MError> {
        let width = names.len();
        let mut columns: Vec<Vec<Lazy>> =
            (0..width).map(|_| Vec::with_capacity(rows.len())).collect();
        for (i, row) in rows.into_iter().enumerate() {
            if row.len() != width {
                return Err(MError::expression(format!(
                    "Row {} has {} values but the table has {width} columns.",
                    i + 1,
                    row.len()
                )));
            }
            for (col, cell) in columns.iter_mut().zip(row) {
                col.push(cell);
            }
        }
        Self::new(
            names,
            Vec::new(),
            columns.into_iter().map(Arc::new).collect(),
        )
    }

    pub fn empty(names: Vec<Arc<str>>) -> Result<Self, MError> {
        let n = names.len();
        Self::new(
            names,
            Vec::new(),
            (0..n).map(|_| Arc::new(Vec::new())).collect(),
        )
    }

    pub fn row_count(&self) -> usize {
        self.0.rows
    }

    pub fn column_count(&self) -> usize {
        self.0.keys.len()
    }

    pub fn column_names(&self) -> &[Arc<str>] {
        self.0.keys.names()
    }

    pub fn keys(&self) -> &Arc<Keys> {
        &self.0.keys
    }

    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.0.keys.position(name)
    }

    /// The cells of column `i`.
    pub fn column(&self, i: usize) -> Option<&[Lazy]> {
        self.0.columns.get(i).map(|c| c.as_slice())
    }

    pub fn column_by_name(&self, name: &str) -> Option<&[Lazy]> {
        self.column_index(name).and_then(|i| self.column(i))
    }

    pub(crate) fn columns(&self) -> &[Arc<Vec<Lazy>>] {
        &self.0.columns
    }

    pub fn column_type(&self, i: usize) -> Option<&TypeValue> {
        self.0.types.get(i)
    }

    pub fn column_types(&self) -> &[TypeValue] {
        &self.0.types
    }

    /// The cell at `row`, `col`.
    pub fn cell(&self, row: usize, col: usize) -> Option<&Lazy> {
        self.0.columns.get(col).and_then(|c| c.get(row))
    }

    /// Row `i` as a record.
    pub fn row(&self, i: usize) -> Option<RecordValue> {
        if i >= self.0.rows {
            return None;
        }
        let values = self
            .0
            .columns
            .iter()
            .map(|c| c.get(i).cloned().unwrap_or(Lazy::Value(Value::Null)))
            .collect();
        Some(RecordValue::new(self.0.keys.clone(), values))
    }

    pub fn rows(&self) -> impl Iterator<Item = RecordValue> + '_ {
        (0..self.0.rows).filter_map(|i| self.row(i))
    }

    /// The table type (`type table [col = T, ...]`).
    pub fn table_type(&self) -> TypeValue {
        TypeValue::table(
            self.0
                .keys
                .names()
                .iter()
                .cloned()
                .zip(self.0.types.iter().cloned())
                .collect(),
        )
    }

    /// A table with the rows at `indices` (in that order).
    pub(crate) fn take_rows(&self, indices: &[usize]) -> TableValue {
        let columns = self
            .0
            .columns
            .iter()
            .map(|c| {
                Arc::new(
                    indices
                        .iter()
                        .map(|&i| c.get(i).cloned().unwrap_or(Lazy::Value(Value::Null)))
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        TableValue(Arc::new(TableData {
            keys: self.0.keys.clone(),
            types: self.0.types.clone(),
            columns,
            rows: indices.len(),
        }))
    }

    /// A table with the same columns and the given types.
    pub(crate) fn with_types(&self, types: Vec<TypeValue>) -> TableValue {
        let mut types = types;
        types.resize(self.0.keys.len(), TypeValue::any());
        TableValue(Arc::new(TableData {
            keys: self.0.keys.clone(),
            types,
            columns: self.0.columns.clone(),
            rows: self.0.rows,
        }))
    }

    pub fn ptr_eq(&self, other: &TableValue) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Drop for TableValue {
    fn drop(&mut self) {
        if let Some(data) = Arc::get_mut(&mut self.0)
            && !data.columns.is_empty()
        {
            crate::value::drop::defer(crate::value::drop::Garbage::Columns(std::mem::take(
                &mut data.columns,
            )));
        }
    }
}

impl fmt::Debug for TableValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&Value::Table(self.clone()), f)
    }
}
