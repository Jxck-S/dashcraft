//! Schema-tolerant access to metadata tables: every table is read with `SELECT *` and
//! columns are looked up by name, so older or newer schema versions (missing or extra
//! columns) degrade to `None` instead of failing.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;
use rusqlite::types::ValueRef;

use super::MAX_CELLS;
use super::error::MetadataError;

/// One SQLite value (blobs are not needed and only their presence is kept).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Cell {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob,
}

impl Cell {
    fn from_ref(value: ValueRef<'_>) -> Self {
        match value {
            ValueRef::Null => Cell::Null,
            ValueRef::Integer(i) => Cell::Int(i),
            ValueRef::Real(f) => Cell::Real(f),
            ValueRef::Text(bytes) => Cell::Text(String::from_utf8_lossy(bytes).into_owned()),
            ValueRef::Blob(_) => Cell::Blob,
        }
    }
}

/// All rows of one metadata table, sorted by `ID` when the table has one.
#[derive(Debug, Default)]
pub(crate) struct Rows {
    columns: HashMap<String, usize>,
    rows: Vec<Vec<Cell>>,
    by_id: HashMap<i64, usize>,
}

/// Names of the real tables (not views) in the database, lower-cased.
pub(crate) fn table_names(conn: &Connection) -> Result<HashSet<String>, MetadataError> {
    let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?;
    let names = stmt
        .query_map([], |row| {
            Ok(match row.get_ref(0)? {
                ValueRef::Text(bytes) => String::from_utf8_lossy(bytes).to_lowercase(),
                _ => String::new(),
            })
        })?
        .collect::<Result<HashSet<_>, _>>()?;
    Ok(names)
}

impl Rows {
    /// Reads every row of `table`; a missing table yields no rows.
    pub(crate) fn load(
        conn: &Connection,
        existing: &HashSet<String>,
        table: &'static str,
    ) -> Result<Rows, MetadataError> {
        if !existing.contains(&table.to_lowercase()) {
            return Ok(Rows::default());
        }
        let quoted = table.replace('"', "\"\"");
        // Column names come from `PRAGMA table_info` as values: rusqlite's
        // `Statement::column_names` panics on names that are not valid UTF-8, which a
        // corrupted schema can contain.
        let mut info = conn.prepare(&format!("PRAGMA table_info(\"{quoted}\")"))?;
        let mut named: Vec<(i64, String)> = info
            .query_map([], |row| {
                let cid = match row.get_ref(0)? {
                    ValueRef::Integer(i) => i,
                    _ => i64::MAX,
                };
                let name = match row.get_ref(1)? {
                    ValueRef::Text(bytes) => String::from_utf8_lossy(bytes).to_lowercase(),
                    _ => String::new(),
                };
                Ok((cid, name))
            })?
            .collect::<Result<_, _>>()?;
        named.sort_by_key(|(cid, _)| *cid);
        let mut stmt = conn.prepare(&format!("SELECT * FROM \"{quoted}\""))?;
        let mut columns: HashMap<String, usize> = HashMap::new();
        for (index, (_, name)) in named.into_iter().enumerate() {
            columns.entry(name).or_insert(index);
        }
        let width = stmt.column_count();
        let mut rows = Vec::new();
        let mut cells: u64 = 0;
        let mut query = stmt.query([])?;
        while let Some(row) = query.next()? {
            cells = cells.saturating_add(width as u64);
            if cells > MAX_CELLS {
                return Err(MetadataError::TooManyRows { table, cells });
            }
            let mut values = Vec::with_capacity(width);
            for i in 0..width {
                values.push(Cell::from_ref(row.get_ref(i)?));
            }
            rows.push(values);
        }
        let mut out = Rows {
            columns,
            rows,
            by_id: HashMap::new(),
        };
        if let Some(&id_col) = out.columns.get("id") {
            out.rows
                .sort_by_key(|r| as_i64(r.get(id_col)).unwrap_or(i64::MAX));
            for (index, row) in out.rows.iter().enumerate() {
                if let Some(id) = as_i64(row.get(id_col)) {
                    out.by_id.entry(id).or_insert(index);
                }
            }
        }
        Ok(out)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = Row<'_>> {
        self.rows.iter().map(|cells| Row { rows: self, cells })
    }

    pub(crate) fn by_id(&self, id: i64) -> Option<Row<'_>> {
        let index = *self.by_id.get(&id)?;
        self.rows.get(index).map(|cells| Row { rows: self, cells })
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }
}

fn as_i64(cell: Option<&Cell>) -> Option<i64> {
    match cell? {
        Cell::Int(i) => Some(*i),
        Cell::Real(f) if f.fract() == 0.0 && f.is_finite() && f.abs() < 9.0e18 => Some(*f as i64),
        Cell::Text(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// One row of a [`Rows`] table.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Row<'a> {
    rows: &'a Rows,
    cells: &'a [Cell],
}

impl<'a> Row<'a> {
    fn cell(&self, column: &str) -> Option<&'a Cell> {
        let index = match self.rows.columns.get(column) {
            Some(&index) => index,
            None => *self.rows.columns.get(&column.to_lowercase())?,
        };
        self.cells.get(index)
    }

    /// Integer value (integral reals and numeric text are accepted).
    pub(crate) fn i64(&self, column: &str) -> Option<i64> {
        as_i64(self.cell(column))
    }

    /// Integer value that is a valid object reference (> 0).
    pub(crate) fn id_ref(&self, column: &str) -> Option<i64> {
        self.i64(column).filter(|&id| id > 0)
    }

    /// The `ID` column (0 if absent).
    pub(crate) fn id(&self) -> i64 {
        self.i64("ID").unwrap_or(0)
    }

    pub(crate) fn f64(&self, column: &str) -> Option<f64> {
        match self.cell(column)? {
            Cell::Int(i) => Some(*i as f64),
            Cell::Real(f) => Some(*f),
            Cell::Text(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    /// Text value, also when empty.
    pub(crate) fn raw_str(&self, column: &str) -> Option<&'a str> {
        match self.cell(column)? {
            Cell::Text(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Non-empty text value.
    pub(crate) fn str(&self, column: &str) -> Option<&'a str> {
        self.raw_str(column).filter(|s| !s.is_empty())
    }

    pub(crate) fn string(&self, column: &str) -> Option<String> {
        self.str(column).map(str::to_owned)
    }

    /// Boolean stored as an integer; `None` when the column is absent or NULL.
    pub(crate) fn opt_flag(&self, column: &str) -> Option<bool> {
        self.i64(column).map(|v| v != 0)
    }

    /// Boolean stored as an integer; absent or NULL is `false`.
    pub(crate) fn flag(&self, column: &str) -> bool {
        self.opt_flag(column).unwrap_or(false)
    }
}
