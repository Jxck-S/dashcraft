//! Storage-engine primitives used by the DAX formula engine: scans,
//! distinct values, group-by (also over columns of related one-side tables,
//! and as a row → group [`GroupIndex`] with one-pass per-group aggregates),
//! aggregates with DAX BLANK semantics, `RELATED`/`RELATEDTABLE` navigation
//! and `LOOKUPVALUE`.
//!
//! Row sets passed in are real rows only; the virtual blank row is handled by
//! the `*_with_blank_row` variants and [`EngineSnapshot::values`].

use std::cmp::Ordering;
use std::sync::Arc;

use std::collections::HashMap;

use arrow::compute::filter_record_batch;
use arrow::record_batch::RecordBatch;
use pbi_model::ColumnRef;
use pbi_model::names::names_eq;

use crate::engine::EngineSnapshot;
use crate::error::{EngineError, Result};
use crate::filter::FilterContext;
use crate::relationships::Hop;
use crate::rowset::RowSet;
use crate::storage::{EncodedColumn, TableData};
use crate::value::{DECIMAL_FACTOR, Value, ValueKey, round_div};

/// Storage-engine aggregations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Agg {
    /// `SUM`: numeric columns; BLANK when no non-blank value.
    Sum,
    /// `MIN`: numbers, dates, text (case-insensitive) and booleans; BLANKs
    /// ignored.
    Min,
    /// `MAX`.
    Max,
    /// `COUNT`/`COUNTA`: non-blank values.
    Count,
    /// Same as [`Agg::Count`] (kept for API clarity).
    CountNonBlank,
    /// `DISTINCTCOUNT`: distinct values, BLANK included.
    DistinctCount,
    /// `DISTINCTCOUNTNOBLANK`.
    DistinctCountNoBlank,
    /// `AVERAGE`: `Double` (or `Decimal` for fixed-decimal columns).
    Average,
    /// `COUNTROWS` (the column is ignored).
    CountRows,
    /// `COUNTBLANK`: BLANK values (empty strings are not BLANK).
    CountBlank,
}

/// One group of [`EngineSnapshot::group_by`].
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    /// Group-by column values, in the order of the requested columns.
    pub values: Vec<Value>,
    /// Rows of the grouped table in this group.
    pub rows: RowSet,
}

/// Summary statistics of a column (explorer grid header).
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnStats {
    /// Total rows.
    pub rows: usize,
    /// Non-blank values.
    pub count: usize,
    /// BLANK (null) values.
    pub blanks: usize,
    /// Distinct non-blank values (text compared case-insensitively).
    pub distinct: usize,
    /// Smallest non-blank value ([`Value::total_cmp`]); BLANK if none.
    pub min: Value,
    /// Largest non-blank value; BLANK if none.
    pub max: Value,
}

/// The rows of one table assigned to groups of equal values
/// ([`EngineSnapshot::group_index`]). Unlike [`EngineSnapshot::group_by`]
/// it keeps no row set per group, so per-group work
/// ([`EngineSnapshot::aggregate_groups`], [`GroupIndex::group_rows`]) is
/// proportional to the group's size.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupIndex {
    table: String,
    universe: usize,
    values: Vec<Vec<Value>>,
    /// Group of every table row (`NO_GROUP`: not grouped).
    row_groups: Vec<u32>,
    /// Rows of group `g`: `members[offsets[g]..offsets[g + 1]]`, ascending.
    offsets: Vec<u32>,
    members: Vec<u32>,
}

const NO_GROUP: u32 = u32::MAX;

impl GroupIndex {
    /// The grouped table (stored name).
    pub fn table(&self) -> &str {
        &self.table
    }

    /// Number of rows of the grouped table.
    pub fn universe(&self) -> usize {
        self.universe
    }

    /// Number of groups.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Group-by values of every group (in the order of the requested
    /// columns), groups in order of first appearance.
    pub fn values(&self) -> &[Vec<Value>] {
        &self.values
    }

    /// The group-by values, consuming the index.
    pub fn into_values(self) -> Vec<Vec<Value>> {
        self.values
    }

    /// Group of `row`; `None` if the row was not grouped.
    pub fn group_of(&self, row: usize) -> Option<usize> {
        match self.row_groups.get(row) {
            Some(&g) if g != NO_GROUP => Some(g as usize),
            _ => None,
        }
    }

    /// Rows of group `group`, ascending.
    pub fn group_rows(&self, group: usize) -> &[u32] {
        let (Some(&start), Some(&end)) = (self.offsets.get(group), self.offsets.get(group + 1))
        else {
            return &[];
        };
        self.members
            .get(start as usize..end as usize)
            .unwrap_or_default()
    }
}

/// A column reached from a base table, possibly through relationships.
struct Accessor {
    enc: Arc<EncodedColumn>,
    path: Vec<Hop>,
}

const BLANK_ID: u32 = u32::MAX;

/// Combined dictionary ids of one row's group-by columns.
enum GroupKeys {
    /// Mixed-radix code of the ids (`radix[i]` = cardinality + 1, the last
    /// digit standing for "no value").
    Packed { radix: Vec<u64> },
    /// The ids themselves (when the code would not fit in 64 bits).
    Wide,
}

impl EngineSnapshot {
    fn data(&self, table: &str) -> Result<&Arc<TableData>> {
        self.storage().table(table)
    }

    fn accessor(&self, table: &str, column: &ColumnRef) -> Result<Accessor> {
        if names_eq(&column.table, table) {
            return Ok(Accessor {
                enc: self.data(table)?.encoded(&column.column)?,
                path: Vec::new(),
            });
        }
        let base = self.data(table)?;
        let path = self
            .graph()
            .expanded_path(base.name(), &column.table)
            .ok_or_else(|| EngineError::NoRelationshipPath {
                from: table.to_owned(),
                to: column.table.clone(),
            })?
            .to_vec();
        Ok(Accessor {
            enc: self.storage().encoded(&column.table, &column.column)?,
            path,
        })
    }

    fn accessor_id(&self, a: &Accessor, row: usize) -> u32 {
        let target = if a.path.is_empty() {
            Some(row)
        } else {
            self.graph().follow(&a.path, row)
        };
        match target.and_then(|r| a.enc.id_at(r)) {
            Some(id) => id,
            None => a.enc.blank_id().unwrap_or(BLANK_ID),
        }
    }

    /// Distinct values of `table[column]` over `rows`, in order of first
    /// appearance in the table. BLANK is included if a row has it.
    pub fn distinct(&self, table: &str, column: &str, rows: &RowSet) -> Result<Vec<Value>> {
        let enc = self.data(table)?.encoded(column)?;
        let counts = enc.counts(Some(rows));
        Ok(enc
            .values()
            .iter()
            .zip(&counts)
            .filter(|(_, c)| **c > 0)
            .map(|(v, _)| v.clone())
            .collect())
    }

    /// DAX `VALUES(table[column])` in `ctx`: distinct visible values plus
    /// BLANK when the table's blank row is visible.
    pub fn values(&self, ctx: &FilterContext, table: &str, column: &str) -> Result<Vec<Value>> {
        let visible = self.visible(ctx, table)?;
        let mut values = self.distinct(table, column, &visible.rows)?;
        if visible.blank_row && !values.iter().any(Value::is_blank) {
            values.push(Value::Blank);
        }
        Ok(values)
    }

    /// Value of `column` for each row of `rows` (ascending row order).
    pub fn column_values(&self, table: &str, column: &str, rows: &RowSet) -> Result<Vec<Value>> {
        let enc = self.data(table)?.encoded(column)?;
        Ok(rows
            .iter()
            .filter_map(|r| enc.value_at(r).cloned())
            .collect())
    }

    /// Rows of `rows` whose `column` value passes `predicate` (evaluated once
    /// per distinct value).
    pub fn filter_rows(
        &self,
        table: &str,
        column: &str,
        rows: &RowSet,
        predicate: impl Fn(&Value) -> bool,
    ) -> Result<RowSet> {
        let enc = self.data(table)?.encoded(column)?;
        let pass: Vec<bool> = enc.values().iter().map(predicate).collect();
        let mut out = RowSet::empty(rows.universe());
        for row in rows.iter() {
            if enc
                .id_at(row)
                .and_then(|id| pass.get(id as usize).copied())
                .unwrap_or(false)
            {
                out.insert(row);
            }
        }
        Ok(out)
    }

    /// Materialises `columns` (all when empty) of `rows` as a record batch.
    pub fn scan(&self, table: &str, columns: &[&str], rows: &RowSet) -> Result<RecordBatch> {
        let data = self.data(table)?;
        let batch = if columns.is_empty() {
            data.batch().clone()
        } else {
            let indices = columns
                .iter()
                .map(|c| {
                    data.column_index(c)
                        .ok_or_else(|| EngineError::ColumnNotFound {
                            table: table.to_owned(),
                            column: (*c).to_owned(),
                        })
                })
                .collect::<Result<Vec<_>>>()?;
            data.batch().project(&indices)?
        };
        if rows.universe() == batch.num_rows() && rows.is_all() {
            return Ok(batch);
        }
        let mask = rows.resized(batch.num_rows()).to_boolean_array();
        Ok(filter_record_batch(&batch, &mask)?)
    }

    /// Groups `rows` of `table` by `columns`. Columns may belong to `table`
    /// or to any table of its expanded table (related one-side tables, via
    /// `RELATED` paths); rows without a related row group under BLANK.
    /// Groups are in order of first appearance. With no columns, all rows
    /// form one group (none if `rows` is empty).
    ///
    /// Each group carries a full-size [`RowSet`]; for high-cardinality
    /// groupings prefer [`EngineSnapshot::group_index`].
    pub fn group_by(
        &self,
        table: &str,
        columns: &[ColumnRef],
        rows: &RowSet,
    ) -> Result<Vec<Group>> {
        let index = self.group_index(table, columns, rows)?;
        let n = index.universe;
        let sets: Vec<RowSet> = (0..index.len())
            .map(|g| RowSet::from_rows(n, index.group_rows(g).iter().map(|r| *r as usize)))
            .collect();
        Ok(index
            .values
            .into_iter()
            .zip(sets)
            .map(|(values, rows)| Group { values, rows })
            .collect())
    }

    /// Groups `rows` of `table` by `columns` like [`EngineSnapshot::group_by`]
    /// (same groups, same order), as a row → group index.
    pub fn group_index(
        &self,
        table: &str,
        columns: &[ColumnRef],
        rows: &RowSet,
    ) -> Result<GroupIndex> {
        let data = self.data(table)?;
        let n = data.row_count();
        let accessors = columns
            .iter()
            .map(|c| self.accessor(table, c))
            .collect::<Result<Vec<_>>>()?;
        let mut radix = Vec::with_capacity(accessors.len());
        let mut span: Option<u64> = Some(1);
        for a in &accessors {
            let r = u64::try_from(a.enc.cardinality())
                .unwrap_or(u64::MAX)
                .saturating_add(1);
            span = span.and_then(|s| s.checked_mul(r));
            radix.push(r);
        }
        let keys = if span.is_some() {
            GroupKeys::Packed { radix }
        } else {
            GroupKeys::Wide
        };
        let mut row_groups = vec![NO_GROUP; n];
        let mut sizes: Vec<u32> = Vec::new();
        let mut first_keys: Vec<Vec<u32>> = Vec::new();
        let mut packed: HashMap<u64, u32> = HashMap::new();
        let mut wide: HashMap<Vec<u32>, u32> = HashMap::new();
        let mut key = vec![0u32; accessors.len()];
        for row in rows.iter().take_while(|r| *r < n) {
            for (slot, a) in key.iter_mut().zip(&accessors) {
                *slot = self.accessor_id(a, row);
            }
            let next = u32::try_from(first_keys.len()).unwrap_or(NO_GROUP);
            let group = match &keys {
                GroupKeys::Packed { radix } => {
                    let code = key.iter().zip(radix).fold(0u64, |acc, (id, r)| {
                        let digit = if *id == BLANK_ID {
                            r - 1
                        } else {
                            u64::from(*id)
                        };
                        acc.wrapping_mul(*r).wrapping_add(digit)
                    });
                    *packed.entry(code).or_insert(next)
                }
                GroupKeys::Wide => match wide.get(key.as_slice()) {
                    Some(g) => *g,
                    None => {
                        wide.insert(key.clone(), next);
                        next
                    }
                },
            };
            if group == next {
                first_keys.push(key.clone());
                sizes.push(0);
            }
            if let (Some(slot), Some(size)) =
                (row_groups.get_mut(row), sizes.get_mut(group as usize))
            {
                *slot = group;
                *size += 1;
            }
        }
        let mut offsets = Vec::with_capacity(sizes.len() + 1);
        let mut running = 0u32;
        offsets.push(0);
        for size in &sizes {
            running = running.saturating_add(*size);
            offsets.push(running);
        }
        let mut cursor = offsets.clone();
        let mut members = vec![0u32; running as usize];
        for (row, g) in row_groups.iter().enumerate() {
            if let Some(pos) = cursor.get_mut(*g as usize) {
                if let Some(slot) = members.get_mut(*pos as usize) {
                    *slot = u32::try_from(row).unwrap_or(u32::MAX);
                }
                *pos += 1;
            }
        }
        let values = first_keys
            .iter()
            .map(|key| {
                key.iter()
                    .zip(&accessors)
                    .map(|(id, a)| a.enc.value_of(*id).cloned().unwrap_or_default())
                    .collect()
            })
            .collect();
        Ok(GroupIndex {
            table: data.name().to_owned(),
            universe: n,
            values,
            row_groups,
            offsets,
            members,
        })
    }

    /// Aggregates `column` of the grouped table over the rows of every group
    /// of `index`: element `g` equals [`EngineSnapshot::aggregate`] over
    /// group `g`'s rows. One pass over the grouped rows (a counting sort by
    /// value), whatever the number of groups.
    pub fn aggregate_groups(
        &self,
        index: &GroupIndex,
        column: &str,
        agg: Agg,
    ) -> Result<Vec<Value>> {
        let data = self.data(&index.table)?;
        if agg == Agg::CountRows {
            return Ok((0..index.len())
                .map(|g| count_value(index.group_rows(g).len()))
                .collect());
        }
        let enc = data.encoded(column)?;
        let ids = enc.ids();
        // Grouped rows ordered by value id (stable counting sort) ...
        let mut by_id = vec![0u32; enc.cardinality() + 1];
        for (row, g) in index.row_groups.iter().enumerate() {
            if *g != NO_GROUP
                && let Some(c) = ids.get(row).and_then(|id| by_id.get_mut(*id as usize + 1))
            {
                *c += 1;
            }
        }
        let mut running = 0u32;
        for c in &mut by_id {
            running = running.saturating_add(*c);
            *c = running;
        }
        let mut rows_by_id = vec![0u32; running as usize];
        for (row, g) in index.row_groups.iter().enumerate() {
            if *g == NO_GROUP {
                continue;
            }
            let Some(id) = ids.get(row) else { continue };
            if let Some(pos) = by_id.get_mut(*id as usize) {
                if let Some(slot) = rows_by_id.get_mut(*pos as usize) {
                    *slot = u32::try_from(row).unwrap_or(u32::MAX);
                }
                *pos += 1;
            }
        }
        // ... then distributed to their groups, so each group's ids ascend.
        let mut cursor = index.offsets.clone();
        let mut group_ids = vec![0u32; index.members.len()];
        for row in &rows_by_id {
            let r = *row as usize;
            let (Some(&g), Some(&id)) = (index.row_groups.get(r), ids.get(r)) else {
                continue;
            };
            if let Some(pos) = cursor.get_mut(g as usize) {
                if let Some(slot) = group_ids.get_mut(*pos as usize) {
                    *slot = id;
                }
                *pos += 1;
            }
        }
        let mut present: Vec<(u32, u64)> = Vec::new();
        let mut out = Vec::with_capacity(index.len());
        for g in 0..index.len() {
            let (Some(&start), Some(&end)) = (index.offsets.get(g), cursor.get(g)) else {
                out.push(Value::Blank);
                continue;
            };
            present.clear();
            for id in group_ids
                .get(start as usize..end as usize)
                .unwrap_or_default()
            {
                match present.last_mut() {
                    Some((last, count)) if last == id => *count += 1,
                    _ => present.push((*id, 1)),
                }
            }
            out.push(aggregate_present(data, column, &enc, &present, false, agg)?);
        }
        Ok(out)
    }

    /// Aggregates `table[column]` over `rows` with DAX BLANK semantics:
    /// aggregates over no (non-blank) values return BLANK, counts of zero
    /// return BLANK.
    pub fn aggregate(&self, table: &str, column: &str, rows: &RowSet, agg: Agg) -> Result<Value> {
        self.aggregate_with_blank_row(table, column, rows, false, agg)
    }

    /// Like [`EngineSnapshot::aggregate`], with the table's blank row (all
    /// columns BLANK) included when `blank_row` is true. It changes
    /// `CountRows`, `CountBlank` and `DistinctCount` only.
    pub fn aggregate_with_blank_row(
        &self,
        table: &str,
        column: &str,
        rows: &RowSet,
        blank_row: bool,
        agg: Agg,
    ) -> Result<Value> {
        let data = self.data(table)?;
        let extra = usize::from(blank_row);
        if agg == Agg::CountRows {
            let n = rows.resized(data.row_count()).count() + extra;
            return Ok(count_value(n));
        }
        let enc = data.encoded(column)?;
        let present = present_ids(&enc, rows);
        aggregate_present(data, column, &enc, &present, blank_row, agg)
    }

    /// `RELATED`: the row of `to_table` related to `row` of `from_table` over
    /// the many → one path; `None` for the blank row.
    pub fn related_row(
        &self,
        from_table: &str,
        row: usize,
        to_table: &str,
    ) -> Result<Option<usize>> {
        let data = self.data(from_table)?;
        check_row(data, row)?;
        let path = self
            .graph()
            .expanded_path(from_table, to_table)
            .ok_or_else(|| EngineError::NoRelationshipPath {
                from: from_table.to_owned(),
                to: to_table.to_owned(),
            })?;
        Ok(self.graph().follow(path, row))
    }

    /// `RELATED(to_table[column])` for `row` of `from_table` (BLANK for the
    /// blank row).
    pub fn related_value(
        &self,
        from_table: &str,
        row: usize,
        to_table: &str,
        column: &str,
    ) -> Result<Value> {
        match self.related_row(from_table, row, to_table)? {
            Some(r) => self.storage().value_at(to_table, column, r),
            None => {
                self.data(to_table)?.column(column)?;
                Ok(Value::Blank)
            }
        }
    }

    /// `RELATEDTABLE`: rows of `many_table` related to `row` of `one_table`
    /// (over the many → one path from `many_table`), intersected with the
    /// rows visible in `ctx` when given. Context transition is the caller's
    /// job: pass the context it produces.
    pub fn related_rows(
        &self,
        one_table: &str,
        row: usize,
        many_table: &str,
        ctx: Option<&FilterContext>,
    ) -> Result<RowSet> {
        check_row(self.data(one_table)?, row)?;
        let many = self.data(many_table)?;
        let n = many.row_count();
        let path = self
            .graph()
            .expanded_path(many_table, one_table)
            .ok_or_else(|| EngineError::NoRelationshipPath {
                from: many_table.to_owned(),
                to: one_table.to_owned(),
            })?;
        let mut rows = match path {
            [hop] if hop.to_one => match self.graph().index(hop.relationship) {
                Some(index) => RowSet::from_rows(n, index.sources(row).iter().map(|r| *r as usize)),
                None => RowSet::empty(n),
            },
            _ => RowSet::from_rows(
                n,
                (0..n).filter(|r| self.graph().follow(path, *r) == Some(row)),
            ),
        };
        if let Some(ctx) = ctx {
            rows.intersect_with(&self.visible_rows(ctx, many_table)?);
        }
        Ok(rows)
    }

    /// `LOOKUPVALUE`: the value of `result_column` in the rows (of `rows`, or
    /// all rows) where every `(column, value)` criterion matches (strict,
    /// case-insensitive [`ValueKey`] equality). BLANK if no row matches;
    /// error if matching rows disagree.
    pub fn lookup(
        &self,
        table: &str,
        result_column: &str,
        criteria: &[(&str, Value)],
        rows: Option<&RowSet>,
    ) -> Result<Value> {
        let data = self.data(table)?;
        let result = data.encoded(result_column)?;
        let mut wanted = Vec::with_capacity(criteria.len());
        for (column, value) in criteria {
            let enc = data.encoded(column)?;
            match enc.id_of(&ValueKey::from(value)) {
                Some(id) => wanted.push((enc, id)),
                None => return Ok(Value::Blank),
            }
        }
        let all;
        let rows = match rows {
            Some(r) => r,
            None => {
                all = RowSet::all(data.row_count());
                &all
            }
        };
        let mut found: Option<u32> = None;
        for row in rows.iter() {
            if wanted.iter().all(|(enc, id)| enc.id_at(row) == Some(*id)) {
                let Some(id) = result.id_at(row) else {
                    continue;
                };
                match found {
                    Some(f) if f != id => {
                        return Err(EngineError::AmbiguousLookup {
                            table: data.name().to_owned(),
                            column: result_column.to_owned(),
                        });
                    }
                    _ => found = Some(id),
                }
            }
        }
        Ok(found
            .and_then(|id| result.value_of(id).cloned())
            .unwrap_or_default())
    }

    /// Count, blanks, distinct count, min and max of a column.
    pub fn column_stats(&self, table: &str, column: &str) -> Result<ColumnStats> {
        let data = self.data(table)?;
        let enc = data.encoded(column)?;
        let counts = enc.counts(None);
        let blanks = enc
            .blank_id()
            .and_then(|id| counts.get(id as usize).copied())
            .unwrap_or(0) as usize;
        let non_blank: Vec<&Value> = enc.values().iter().filter(|v| !v.is_blank()).collect();
        let min = non_blank.iter().copied().min_by(|a, b| a.total_cmp(b));
        let max = non_blank.iter().copied().max_by(|a, b| a.total_cmp(b));
        Ok(ColumnStats {
            rows: data.row_count(),
            count: data.row_count().saturating_sub(blanks),
            blanks,
            distinct: non_blank.len(),
            min: min.cloned().unwrap_or_default(),
            max: max.cloned().unwrap_or_default(),
        })
    }
}

/// `(dictionary id, rows)` of the values of `rows`, ascending by id.
fn present_ids(enc: &EncodedColumn, rows: &RowSet) -> Vec<(u32, u64)> {
    let ids = enc.ids();
    let selected = rows.count();
    if selected.saturating_mul(SPARSE_RATIO) < enc.cardinality() {
        // Few rows of a high-cardinality column: sort their ids instead of
        // counting over the whole dictionary.
        let mut found: Vec<u32> = rows.iter().filter_map(|r| ids.get(r).copied()).collect();
        found.sort_unstable();
        let mut out: Vec<(u32, u64)> = Vec::new();
        for id in found {
            match out.last_mut() {
                Some((last, count)) if *last == id => *count += 1,
                _ => out.push((id, 1)),
            }
        }
        return out;
    }
    enc.counts(Some(rows))
        .into_iter()
        .enumerate()
        .filter(|(_, c)| *c > 0)
        .map(|(id, c)| (u32::try_from(id).unwrap_or(u32::MAX), c))
        .collect()
}

/// [`present_ids`] switches to sorting when the rows are fewer than
/// 1/`SPARSE_RATIO` of the column's distinct values.
const SPARSE_RATIO: usize = 4;

/// Aggregates the values `present` (`(dictionary id, rows)`, ascending by
/// id, every count > 0) of `column` with DAX BLANK semantics (see
/// [`EngineSnapshot::aggregate_with_blank_row`]). `CountRows` is the
/// caller's job.
fn aggregate_present(
    data: &TableData,
    column: &str,
    enc: &EncodedColumn,
    present: &[(u32, u64)],
    blank_row: bool,
    agg: Agg,
) -> Result<Value> {
    let extra = usize::from(blank_row);
    let blank_count = enc
        .blank_id()
        .and_then(|blank| present.iter().find(|(id, _)| *id == blank))
        .map_or(0, |(_, c)| *c);
    let present = || {
        present
            .iter()
            .filter_map(|(id, c)| enc.value_of(*id).map(|v| (v, c)))
    };
    let non_blank: u64 = present()
        .filter(|(v, _)| !v.is_blank())
        .map(|(_, c)| *c)
        .sum();
    let unsupported = || EngineError::UnsupportedAggregation {
        agg: format!("{agg:?}"),
        table: data.name().to_owned(),
        column: column.to_owned(),
        data_type: data
            .column_type(column)
            .map(ToString::to_string)
            .unwrap_or_default(),
    };
    Ok(match agg {
        Agg::CountRows => count_value(present().map(|(_, c)| *c as usize).sum::<usize>() + extra),
        Agg::Count | Agg::CountNonBlank => count_value(non_blank as usize),
        Agg::CountBlank => count_value(blank_count as usize + extra),
        Agg::DistinctCount => {
            let distinct = present().count();
            let blank_extra = usize::from(blank_row && blank_count == 0);
            count_value(distinct + blank_extra)
        }
        Agg::DistinctCountNoBlank => count_value(present().filter(|(v, _)| !v.is_blank()).count()),
        Agg::Min | Agg::Max => {
            let wanted = if agg == Agg::Min {
                Ordering::Less
            } else {
                Ordering::Greater
            };
            present()
                .map(|(v, _)| v)
                .filter(|v| !v.is_blank())
                .fold(None::<&Value>, |best, v| match best {
                    Some(b) if v.total_cmp(b) != wanted => Some(b),
                    _ => Some(v),
                })
                .cloned()
                .unwrap_or_default()
        }
        Agg::Sum | Agg::Average => {
            let sum = numeric_sum(present()).ok_or_else(unsupported)??;
            if agg == Agg::Sum {
                sum
            } else {
                average(sum, non_blank)
            }
        }
    })
}

fn check_row(data: &TableData, row: usize) -> Result<()> {
    if row >= data.row_count() {
        return Err(EngineError::RowOutOfRange {
            table: data.name().to_owned(),
            row,
            rows: data.row_count(),
        });
    }
    Ok(())
}

fn count_value(n: usize) -> Value {
    if n == 0 {
        Value::Blank
    } else {
        Value::Integer(i64::try_from(n).unwrap_or(i64::MAX))
    }
}

/// Sum of the present values: `None` if a value is not numeric, `Some(Err)`
/// on decimal overflow, BLANK when there are no non-blank values.
fn numeric_sum<'a>(values: impl Iterator<Item = (&'a Value, &'a u64)>) -> Option<Result<Value>> {
    let mut int_sum: i128 = 0;
    let mut dec_sum: i128 = 0;
    let mut float_sum = 0.0f64;
    let (mut ints, mut decs, mut floats) = (false, false, false);
    for (value, count) in values {
        let count = i128::from(*count);
        match value {
            Value::Blank => {}
            Value::Integer(i) => {
                ints = true;
                int_sum = int_sum.saturating_add(i128::from(*i) * count);
            }
            Value::Decimal(d) => {
                decs = true;
                match d.checked_mul(count).and_then(|x| dec_sum.checked_add(x)) {
                    Some(s) => dec_sum = s,
                    None => return Some(Err(EngineError::Overflow("decimal SUM".into()))),
                }
            }
            Value::Double(f) => {
                floats = true;
                float_sum += f * count as f64;
            }
            _ => return None,
        }
    }
    Some(Ok(if floats {
        Value::Double(float_sum + int_sum as f64 + dec_sum as f64 / DECIMAL_FACTOR as f64)
    } else if decs {
        match int_sum
            .checked_mul(DECIMAL_FACTOR)
            .and_then(|x| x.checked_add(dec_sum))
        {
            Some(s) => Value::Decimal(s),
            None => return Some(Err(EngineError::Overflow("decimal SUM".into()))),
        }
    } else if ints {
        match i64::try_from(int_sum) {
            Ok(i) => Value::Integer(i),
            Err(_) => Value::Double(int_sum as f64),
        }
    } else {
        Value::Blank
    }))
}

fn average(sum: Value, count: u64) -> Value {
    if count == 0 {
        return Value::Blank;
    }
    match sum {
        Value::Decimal(d) => Value::Decimal(round_div(d, i128::from(count))),
        Value::Blank => Value::Blank,
        other => other
            .as_f64()
            .map_or(Value::Blank, |s| Value::Double(s / count as f64)),
    }
}
