//! `Table.*` functions that reshape tables: grouping, pivoting, joins, expanding nested
//! values, transposing, and splitting or combining columns.
//!
//! Join row order follows a fixed model: matched rows in left order (each with its matches in
//! right order), then unmatched right rows, then unmatched left rows. Desktop's local engine
//! does not always produce the same order (it depends on its join algorithm); see
//! `docs/GAPS.md`.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hasher};
use std::sync::Arc;

use super::args::{
    arg, catch, count_arg, func_arg, items, names_arg, req_func, req_list, req_table, req_text,
};
use super::table::{
    build, column_spec, columns_of, default_names, is_ignore_case, names, null_column, pairs,
    table_value, text_of,
};
use super::{Builder, missing_column};
use crate::ast::PrimitiveType;
use crate::eval::interp::{Interp, convert_error};
use crate::merror::MError;
use crate::service::{LibraryKind, catalog_entry};
use crate::value::ops::{equals_with, hash_value};
use crate::value::{
    FunctionValue, Keys, Lazy, ListValue, RecordValue, TableValue, TypeKind, TypeValue, Value,
};

/// A column under construction: name, type and cells.
pub(super) type Column = (Arc<str>, TypeValue, Arc<Vec<Lazy>>);

pub(super) const TOO_MANY: &str =
    "There were too many elements in the enumeration to complete the operation.";
pub(super) const NOT_ENOUGH: &str =
    "There weren't enough elements in the enumeration to complete the operation.";

pub(super) fn register(b: &mut Builder) {
    b.func(
        "Table.Group",
        &[
            "table",
            "key",
            "aggregatedColumns",
            "?groupKind",
            "?comparer",
        ],
        group,
    );
    b.func(
        "Table.Pivot",
        &[
            "table",
            "pivotValues",
            "attributeColumn",
            "valueColumn",
            "?aggregationFunction",
        ],
        pivot,
    );
    b.func(
        "Table.Unpivot",
        &["table", "pivotColumns", "attributeColumn", "valueColumn"],
        unpivot,
    );
    b.func(
        "Table.UnpivotOtherColumns",
        &["table", "pivotColumns", "attributeColumn", "valueColumn"],
        unpivot_other_columns,
    );
    b.func(
        "Table.Join",
        &[
            "table1",
            "key1",
            "table2",
            "key2",
            "?joinKind",
            "?joinAlgorithm",
            "?keyEqualityComparers",
        ],
        join,
    );
    b.func(
        "Table.NestedJoin",
        &[
            "table1",
            "key1",
            "table2",
            "key2",
            "newColumnName",
            "?joinKind",
            "?keyEqualityComparers",
        ],
        nested_join,
    );
    b.func(
        "Table.AddJoinColumn",
        &["table1", "key1", "table2", "key2", "newColumnName"],
        add_join_column,
    );
    b.func(
        "Table.ExpandTableColumn",
        &["table", "column", "columnNames", "?newColumnNames"],
        expand_table_column,
    );
    b.func(
        "Table.ExpandRecordColumn",
        &["table", "column", "fieldNames", "?newColumnNames"],
        expand_record_column,
    );
    b.func(
        "Table.ExpandListColumn",
        &["table", "column"],
        expand_list_column,
    );
    b.func(
        "Table.AggregateTableColumn",
        &["table", "column", "aggregations"],
        aggregate_table_column,
    );
    b.func("Table.Transpose", &["table", "?columns"], transpose);
    b.func(
        "Table.SplitColumn",
        &[
            "table",
            "sourceColumn",
            "splitter",
            "?columnNamesOrNumber",
            "?default",
            "?extraColumns",
        ],
        split_column,
    );
    b.func(
        "Table.CombineColumns",
        &["table", "sourceColumns", "combiner", "column"],
        combine_columns,
    );
    b.func(
        "Table.CombineColumnsToRecord",
        &["table", "newColumnName", "sourceColumns", "?options"],
        combine_columns_to_record,
    );
}

pub(super) fn null_cell() -> Lazy {
    Lazy::Value(Value::Null)
}

pub(super) fn invalid_enum(v: &Value, name: &str) -> MError {
    MError::expression(format!(
        "{} is not a valid {name} value.",
        crate::value::format::describe(v)
    ))
}

/// A small non-negative integral enumeration value (`JoinKind.LeftOuter`, `GroupKind.Local`).
pub(super) fn enum_arg(v: &Value, name: &str, count: usize) -> Result<Option<usize>, MError> {
    match v.plain() {
        Value::Null => Ok(None),
        Value::Number(n) if n.fract() == 0.0 && *n >= 0.0 && *n < count as f64 => {
            Ok(Some(*n as usize))
        }
        Value::Number(_) => Err(invalid_enum(v, name)),
        other => Err(convert_error(other, "Number")),
    }
}

/// Column indices of a key argument (a column name or a list of names).
pub(super) fn key_columns(it: &Interp, t: &TableValue, v: &Value) -> Result<Vec<usize>, MError> {
    names_arg(it, v)?
        .iter()
        .map(|n| t.column_index(n).ok_or_else(|| missing_column(n)))
        .collect()
}

/// The forced values of columns `cols` in row `r`.
pub(super) fn row_key(
    it: &Interp,
    t: &TableValue,
    r: usize,
    cols: &[usize],
) -> Result<Vec<Value>, MError> {
    cols.iter()
        .map(|&c| match t.cell(r, c) {
            Some(cell) => cell.force(it),
            None => Ok(Value::Null),
        })
        .collect()
}

fn key_hash(key: &[Value], ignore_case: bool) -> u64 {
    let mut h = DefaultHasher::new();
    for v in key {
        hash_value(v, ignore_case, &mut h);
    }
    h.finish()
}

fn keys_equal(it: &Interp, a: &[Value], b: &[Value], ignore_case: bool) -> Result<bool, MError> {
    if a.len() != b.len() {
        return Ok(false);
    }
    for (x, y) in a.iter().zip(b) {
        if !equals_with(it, x, y, ignore_case)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Distinct keys in first-seen order, found by hashing.
pub(super) struct KeyGroups {
    buckets: HashMap<u64, Vec<usize>>,
    keys: Vec<Vec<Value>>,
    ignore_case: bool,
}

impl KeyGroups {
    pub(super) fn new(ignore_case: bool) -> Self {
        Self {
            buckets: HashMap::new(),
            keys: Vec::new(),
            ignore_case,
        }
    }

    /// The index of the group whose key equals `key`.
    pub(super) fn find(&self, it: &Interp, key: &[Value]) -> Result<Option<usize>, MError> {
        let Some(bucket) = self.buckets.get(&key_hash(key, self.ignore_case)) else {
            return Ok(None);
        };
        for &g in bucket {
            if let Some(k) = self.keys.get(g)
                && keys_equal(it, k, key, self.ignore_case)?
            {
                return Ok(Some(g));
            }
        }
        Ok(None)
    }

    /// Adds a group for `key` and returns its index.
    pub(super) fn insert(&mut self, key: Vec<Value>) -> usize {
        let g = self.keys.len();
        self.buckets
            .entry(key_hash(&key, self.ignore_case))
            .or_default()
            .push(g);
        self.keys.push(key);
        g
    }
}

/// The columns of `t` with column `ci` replaced by `new` at its position.
fn replace_column(t: &TableValue, ci: usize, new: Vec<Column>) -> Vec<Column> {
    let mut cols = columns_of(t);
    let tail = cols.split_off(ci.saturating_add(1).min(cols.len()));
    cols.truncate(ci);
    cols.extend(new);
    cols.extend(tail);
    cols
}

/// The columns of `t` without `sources`, with `new` at the position of the first source.
/// `t` without the `sources` columns and with `new` where the last listed source column was.
fn merge_columns(t: &TableValue, sources: &[usize], new: Column) -> Vec<Column> {
    let at = sources.last().copied();
    let mut new = Some(new);
    let mut cols = Vec::with_capacity(t.column_count());
    for (c, col) in columns_of(t).into_iter().enumerate() {
        if Some(c) == at
            && let Some(n) = new.take()
        {
            cols.push(n);
        }
        if !sources.contains(&c) {
            cols.push(col);
        }
    }
    cols.extend(new);
    cols
}

fn column_name(t: &TableValue, c: usize) -> Arc<str> {
    t.column_names()
        .get(c)
        .cloned()
        .unwrap_or_else(|| Arc::from(""))
}

fn column_type(t: &TableValue, c: usize) -> TypeValue {
    t.column_type(c).cloned().unwrap_or_else(TypeValue::any)
}

// ---------------------------------------------------------------------------------------------
// Table.Group
// ---------------------------------------------------------------------------------------------

enum KeyComparer {
    Default,
    IgnoreCase,
    Custom(FunctionValue),
}

fn same_key(it: &Interp, cmp: &KeyComparer, a: &[Value], b: &[Value]) -> Result<bool, MError> {
    match cmp {
        KeyComparer::Default => keys_equal(it, a, b, false),
        KeyComparer::IgnoreCase => keys_equal(it, a, b, true),
        KeyComparer::Custom(f) => {
            for (x, y) in a.iter().zip(b) {
                match it.call(f, &[x.clone(), y.clone()])?.plain() {
                    Value::Number(n) if *n == 0.0 => {}
                    Value::Number(_) => return Ok(false),
                    other => return Err(convert_error(other, "Number")),
                }
            }
            Ok(true)
        }
    }
}

/// The declared return type of a library function (`Table.RowCount` → `number`); `any` for
/// closures and anything else.
fn declared_return_type(f: &FunctionValue) -> TypeValue {
    let library_fn = f
        .name()
        .filter(|n| matches!(super::library().get(n), Some(Value::Function(g)) if g.ptr_eq(f)));
    let returns = library_fn
        .and_then(catalog_entry)
        .and_then(|e| match &e.kind {
            LibraryKind::Function(lf) => Some(lf.returns),
            _ => None,
        });
    returns.map_or_else(TypeValue::any, primitive_type_named)
}

/// `number`, `nullable text`, ... as a type.
fn primitive_type_named(s: &str) -> TypeValue {
    let (nullable, base) = match s.strip_prefix("nullable ") {
        Some(b) => (true, b),
        None => (false, s),
    };
    let p = PrimitiveType::ALL
        .iter()
        .copied()
        .find(|p| p.name() == base)
        .unwrap_or(PrimitiveType::Any);
    let ty = TypeValue::primitive(p);
    if nullable { ty.nullable() } else { ty }
}

/// `{name, function[, type]}` aggregates (one unnested aggregate is allowed).
fn aggregates(it: &Interp, v: &Value) -> Result<Vec<(Arc<str>, FunctionValue, TypeValue)>, MError> {
    pairs(it, v)?
        .iter()
        .map(|a| {
            let (name, f, ty) = match a.as_slice() {
                [n, f] => (n, f, None),
                [n, f, ty] => (n, f, Some(ty)),
                _ => {
                    return Err(MError::expression(
                        "Table.Group expects {name, function[, type]} lists.",
                    ));
                }
            };
            let Value::Function(f) = f.plain() else {
                return Err(convert_error(f, "Function"));
            };
            let ty = match ty.map(Value::plain) {
                Some(Value::Type(t)) => t.clone(),
                Some(Value::Null) | None => declared_return_type(f),
                Some(other) => return Err(convert_error(other, "Type")),
            };
            Ok((text_of(name)?, f.clone(), ty))
        })
        .collect()
}

fn group(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let key_cols = key_columns(it, t, arg(args, 1))?;
    let aggregates = aggregates(it, arg(args, 2))?;
    let local = enum_arg(arg(args, 3), "GroupKind", 2)? == Some(0);
    let comparer = match arg(args, 4) {
        Value::Null => KeyComparer::Default,
        v if is_ignore_case(v) => KeyComparer::IgnoreCase,
        Value::Function(f) => KeyComparer::Custom(f.clone()),
        other => return Err(convert_error(other, "Function")),
    };
    let mut out_names: Vec<Arc<str>> = key_cols.iter().map(|&c| column_name(t, c)).collect();
    out_names.extend(aggregates.iter().map(|a| a.0.clone()));
    Keys::try_new(out_names)?;

    let mut index = KeyGroups::new(matches!(comparer, KeyComparer::IgnoreCase));
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for r in 0..t.row_count() {
        it.tick()?;
        let key = row_key(it, t, r, &key_cols)?;
        let found = if local {
            let last = index.keys.len().checked_sub(1);
            match last.and_then(|g| index.keys.get(g).map(|k| (g, k))) {
                Some((g, k)) if same_key(it, &comparer, k, &key)? => Some(g),
                _ => None,
            }
        } else if let KeyComparer::Custom(_) = comparer {
            let mut hit = None;
            for (g, k) in index.keys.iter().enumerate() {
                it.tick()?;
                if same_key(it, &comparer, k, &key)? {
                    hit = Some(g);
                    break;
                }
            }
            hit
        } else {
            index.find(it, &key)?
        };
        match found.and_then(|g| groups.get_mut(g)) {
            Some(rows) => rows.push(r),
            None => {
                index.insert(key);
                groups.push(vec![r]);
            }
        }
    }

    let mut cols: Vec<Column> = Vec::with_capacity(key_cols.len() + aggregates.len());
    for (k, &c) in key_cols.iter().enumerate() {
        let cells = index
            .keys
            .iter()
            .map(|key| Lazy::Value(key.get(k).cloned().unwrap_or(Value::Null)))
            .collect();
        cols.push((column_name(t, c), column_type(t, c), Arc::new(cells)));
    }
    for (name, f, ty) in &aggregates {
        let mut cells = Vec::with_capacity(groups.len());
        for rows in &groups {
            it.tick()?;
            cells.push(catch(it.call(f, &[Value::Table(t.take_rows(rows))]))?);
        }
        cols.push((name.clone(), ty.clone(), Arc::new(cells)));
    }
    table_value(build(it, cols)?)
}

// ---------------------------------------------------------------------------------------------
// Table.Pivot, Table.Unpivot
// ---------------------------------------------------------------------------------------------

fn pivot(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let mut values: Vec<Arc<str>> = Vec::new();
    for v in items(it, req_list(args, 1)?)? {
        match v {
            Value::Text(s) => values.push(s),
            other => return Err(convert_error(&other, "Text")),
        }
    }
    let attr = req_text(args, 2)?;
    let ai = t.column_index(attr).ok_or_else(|| missing_column(attr))?;
    let val = req_text(args, 3)?;
    let vi = t.column_index(val).ok_or_else(|| missing_column(val))?;
    let agg = func_arg(args, 4)?;
    let others: Vec<usize> = (0..t.column_count())
        .filter(|&c| c != ai && c != vi)
        .collect();
    let mut out_names: Vec<Arc<str>> = others.iter().map(|&c| column_name(t, c)).collect();
    out_names.extend(values.iter().cloned());
    Keys::try_new(out_names)?;

    let mut index = KeyGroups::new(false);
    let mut first_rows: Vec<usize> = Vec::new();
    // group → pivot value → value cells
    let mut found: Vec<Vec<Vec<Lazy>>> = Vec::new();
    for r in 0..t.row_count() {
        it.tick()?;
        let key = row_key(it, t, r, &others)?;
        let g = match index.find(it, &key)? {
            Some(g) => g,
            None => {
                first_rows.push(r);
                found.push(vec![Vec::new(); values.len()]);
                index.insert(key)
            }
        };
        let a = match t.cell(r, ai) {
            Some(c) => c.force(it)?,
            None => Value::Null,
        };
        let Value::Text(a) = a.plain() else {
            continue;
        };
        if let Some(p) = values.iter().position(|v| **v == **a)
            && let Some(slot) = found.get_mut(g).and_then(|g| g.get_mut(p))
        {
            slot.push(t.cell(r, vi).cloned().unwrap_or_else(null_cell));
        }
    }

    let base = t.take_rows(&first_rows);
    let mut cols: Vec<Column> = others
        .iter()
        .filter_map(|&c| {
            Some((
                column_name(t, c),
                column_type(t, c),
                base.columns().get(c)?.clone(),
            ))
        })
        .collect();
    // With or without an aggregation function, the pivoted columns keep the value column's type.
    let value_type = column_type(t, vi).nullable();
    for (p, name) in values.iter().enumerate() {
        let mut cells = Vec::with_capacity(found.len());
        for g in &mut found {
            it.tick()?;
            let mut vals = g.get_mut(p).map(std::mem::take).unwrap_or_default();
            cells.push(match agg {
                Some(f) => catch(it.call(f, &[Value::List(ListValue::new(vals))]))?,
                None if vals.len() > 1 => Lazy::Error(Arc::new(MError::expression(TOO_MANY))),
                None => vals.pop().unwrap_or_else(null_cell),
            });
        }
        cols.push((name.clone(), value_type.clone(), Arc::new(cells)));
    }
    table_value(build(it, cols)?)
}

fn unpivot(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let mut cols = key_columns(it, t, arg(args, 1))?;
    cols.sort_unstable();
    cols.dedup();
    unpivot_columns(it, t, &cols, req_text(args, 2)?, req_text(args, 3)?)
}

fn unpivot_other_columns(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let keep = key_columns(it, t, arg(args, 1))?;
    let cols: Vec<usize> = (0..t.column_count())
        .filter(|c| !keep.contains(c))
        .collect();
    unpivot_columns(it, t, &cols, req_text(args, 2)?, req_text(args, 3)?)
}

/// Unpivots `pivot` (column indices in table order), skipping null values. The attribute and
/// value columns follow the remaining columns.
fn unpivot_columns(
    it: &Interp,
    t: &TableValue,
    pivot: &[usize],
    attr: &str,
    val: &str,
) -> Result<Value, MError> {
    let others: Vec<usize> = (0..t.column_count())
        .filter(|c| !pivot.contains(c))
        .collect();
    let mut out_names: Vec<Arc<str>> = others.iter().map(|&c| column_name(t, c)).collect();
    out_names.extend([Arc::from(attr), Arc::from(val)]);
    Keys::try_new(out_names)?;

    let mut src_rows = Vec::new();
    let mut attrs = Vec::new();
    let mut vals = Vec::new();
    for r in 0..t.row_count() {
        it.tick()?;
        for &c in pivot {
            let Some(cell) = t.cell(r, c) else { continue };
            match cell.force(it) {
                Ok(v) if v.is_null() => continue,
                Ok(v) => vals.push(Lazy::Value(v)),
                Err(e) => vals.push(catch(Err(e))?),
            }
            src_rows.push(r);
            attrs.push(Lazy::Value(Value::Text(column_name(t, c))));
        }
        it.check_rows(src_rows.len())?;
    }
    let types: Vec<TypeValue> = pivot
        .iter()
        .map(|&c| column_type(t, c).non_nullable())
        .collect();
    let value_type = match types.split_first() {
        Some((first, rest))
            if first.base() != PrimitiveType::Any && rest.iter().all(|ty| ty.same_as(first)) =>
        {
            first.clone()
        }
        _ => TypeValue::any(),
    };
    let base = t.take_rows(&src_rows);
    let mut cols: Vec<Column> = others
        .iter()
        .filter_map(|&c| {
            Some((
                column_name(t, c),
                column_type(t, c),
                base.columns().get(c)?.clone(),
            ))
        })
        .collect();
    cols.push((Arc::from(attr), TypeValue::text(), Arc::new(attrs)));
    cols.push((Arc::from(val), value_type, Arc::new(vals)));
    table_value(build(it, cols)?)
}

// ---------------------------------------------------------------------------------------------
// Joins
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JoinKind {
    Inner,
    LeftOuter,
    RightOuter,
    FullOuter,
    LeftAnti,
    RightAnti,
    LeftSemi,
    RightSemi,
}

impl JoinKind {
    const ALL: [JoinKind; 8] = [
        JoinKind::Inner,
        JoinKind::LeftOuter,
        JoinKind::RightOuter,
        JoinKind::FullOuter,
        JoinKind::LeftAnti,
        JoinKind::RightAnti,
        JoinKind::LeftSemi,
        JoinKind::RightSemi,
    ];

    fn from_arg(v: &Value, default: JoinKind) -> Result<Self, MError> {
        Ok(enum_arg(v, "JoinKind", Self::ALL.len())?
            .and_then(|i| Self::ALL.get(i).copied())
            .unwrap_or(default))
    }

    /// Whether the output has rows without a left row.
    fn left_nullable(self) -> bool {
        matches!(
            self,
            JoinKind::RightOuter | JoinKind::FullOuter | JoinKind::RightAnti | JoinKind::RightSemi
        )
    }

    /// Whether the output has rows without a right row.
    fn right_nullable(self) -> bool {
        matches!(
            self,
            JoinKind::LeftOuter | JoinKind::FullOuter | JoinKind::LeftAnti | JoinKind::LeftSemi
        )
    }
}

fn check_no_comparers(v: &Value) -> Result<(), MError> {
    if v.is_null() {
        Ok(())
    } else {
        Err(MError::expression(
            "Local evaluation of Table.Join or Table.NestedJoin with key equality comparers is not supported.",
        ))
    }
}

/// The right side of a nested join: a table, or a function returning one.
fn join_table(it: &Interp, v: &Value) -> Result<TableValue, MError> {
    match v.plain() {
        Value::Table(t) => Ok(t.clone()),
        Value::Function(f) => match it.call(f, &[])?.into_plain() {
            Value::Table(t) => Ok(t),
            other => Err(convert_error(&other, "Table")),
        },
        other => Err(convert_error(other, "Table")),
    }
}

struct JoinMatches {
    /// For each left row, its matching right rows in right order.
    left: Vec<Vec<usize>>,
    right_matched: Vec<bool>,
}

impl JoinMatches {
    fn right_rows(&self, matched: bool) -> Vec<usize> {
        self.right_matched
            .iter()
            .enumerate()
            .filter(|(_, m)| **m == matched)
            .map(|(r, _)| r)
            .collect()
    }
}

fn match_rows(
    it: &Interp,
    l: &TableValue,
    lk: &[usize],
    r: &TableValue,
    rk: &[usize],
) -> Result<JoinMatches, MError> {
    if lk.len() != rk.len() {
        return Err(MError::expression(
            "The join keys of both tables must have the same number of columns.",
        ));
    }
    let mut index = KeyGroups::new(false);
    let mut rows_of: Vec<Vec<usize>> = Vec::new();
    for rr in 0..r.row_count() {
        it.tick()?;
        let key = row_key(it, r, rr, rk)?;
        match index.find(it, &key)?.and_then(|g| rows_of.get_mut(g)) {
            Some(rows) => rows.push(rr),
            None => {
                index.insert(key);
                rows_of.push(vec![rr]);
            }
        }
    }
    let mut right_matched = vec![false; r.row_count()];
    let mut left = Vec::with_capacity(l.row_count());
    for lr in 0..l.row_count() {
        it.tick()?;
        let key = row_key(it, l, lr, lk)?;
        let m = match index.find(it, &key)? {
            Some(g) => rows_of.get(g).cloned().unwrap_or_default(),
            None => Vec::new(),
        };
        for &rr in &m {
            if let Some(f) = right_matched.get_mut(rr) {
                *f = true;
            }
        }
        left.push(m);
    }
    Ok(JoinMatches {
        left,
        right_matched,
    })
}

/// The cells of `t`'s columns (except `skip`) for `rows` (`None` → null).
fn side_columns(
    t: &TableValue,
    rows: &[Option<usize>],
    nullable: bool,
    skip: &[bool],
) -> Vec<Column> {
    (0..t.column_count())
        .filter(|&c| !skip.get(c).copied().unwrap_or(false))
        .map(|c| {
            let ty = column_type(t, c);
            let ty = if nullable { ty.nullable() } else { ty };
            let cells = rows
                .iter()
                .map(|r| {
                    r.and_then(|r| t.cell(r, c).cloned())
                        .unwrap_or_else(null_cell)
                })
                .collect();
            (column_name(t, c), ty, Arc::new(cells))
        })
        .collect()
}

fn join(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let l = req_table(args, 0)?;
    let lk = key_columns(it, l, arg(args, 1))?;
    let r = req_table(args, 2)?.clone();
    let rk = key_columns(it, &r, arg(args, 3))?;
    let kind = JoinKind::from_arg(arg(args, 4), JoinKind::Inner)?;
    enum_arg(arg(args, 5), "JoinAlgorithm", 7)?;
    check_no_comparers(arg(args, 6))?;

    // Columns of both sides; an inner join merges same-named key columns.
    let shared: Vec<(usize, usize)> = r
        .column_names()
        .iter()
        .enumerate()
        .filter_map(|(rc, n)| l.column_index(n).map(|lc| (lc, rc)))
        .collect();
    let mut drop_right = vec![false; r.column_count()];
    if !shared.is_empty() {
        let merge = kind == JoinKind::Inner
            && shared.iter().all(|&(lc, rc)| {
                let p = lk.iter().position(|&c| c == lc);
                p.is_some() && p == rk.iter().position(|&c| c == rc)
            });
        if !merge {
            let list = shared
                .iter()
                .map(|&(lc, _)| format!("\"{}\"", column_name(l, lc)))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(MError::expression(format!(
                "A join operation cannot result in a table with duplicate column names ({list})."
            )));
        }
        for &(_, rc) in &shared {
            if let Some(d) = drop_right.get_mut(rc) {
                *d = true;
            }
        }
    }

    let m = match_rows(it, l, &lk, &r, &rk)?;
    let mut lrows: Vec<Option<usize>> = Vec::new();
    let mut rrows: Vec<Option<usize>> = Vec::new();
    let mut push = |a: Option<usize>, b: Option<usize>| {
        lrows.push(a);
        rrows.push(b);
    };
    let matched = |push: &mut dyn FnMut(Option<usize>, Option<usize>)| {
        for (lr, ms) in m.left.iter().enumerate() {
            for &rr in ms {
                push(Some(lr), Some(rr));
            }
        }
    };
    match kind {
        JoinKind::Inner => matched(&mut push),
        JoinKind::LeftOuter => {
            for (lr, ms) in m.left.iter().enumerate() {
                if ms.is_empty() {
                    push(Some(lr), None);
                }
                for &rr in ms {
                    push(Some(lr), Some(rr));
                }
            }
        }
        JoinKind::RightOuter | JoinKind::FullOuter => {
            matched(&mut push);
            for rr in m.right_rows(false) {
                push(None, Some(rr));
            }
            if kind == JoinKind::FullOuter {
                for (lr, ms) in m.left.iter().enumerate() {
                    if ms.is_empty() {
                        push(Some(lr), None);
                    }
                }
            }
        }
        JoinKind::LeftAnti | JoinKind::LeftSemi => {
            let want_match = kind == JoinKind::LeftSemi;
            for (lr, ms) in m.left.iter().enumerate() {
                if ms.is_empty() != want_match {
                    push(Some(lr), None);
                }
            }
        }
        JoinKind::RightAnti | JoinKind::RightSemi => {
            for rr in m.right_rows(kind == JoinKind::RightSemi) {
                push(None, Some(rr));
            }
        }
    }
    it.check_rows(lrows.len())?;
    let mut cols = side_columns(l, &lrows, kind.left_nullable(), &[]);
    cols.extend(side_columns(&r, &rrows, kind.right_nullable(), &drop_right));
    table_value(build(it, cols)?)
}

fn nested_join(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let kind = JoinKind::from_arg(arg(args, 5), JoinKind::LeftOuter)?;
    nested(it, args, kind, arg(args, 6))
}

fn add_join_column(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    nested(it, args, JoinKind::LeftOuter, &Value::Null)
}

/// A table with the columns of `t` and one row of nulls.
fn null_row_table(t: &TableValue) -> Result<TableValue, MError> {
    TableValue::new(
        t.column_names().to_vec(),
        t.column_types().to_vec(),
        (0..t.column_count()).map(|_| null_column(1)).collect(),
    )
}

fn nested(it: &Interp, args: &[Value], kind: JoinKind, comparers: &Value) -> Result<Value, MError> {
    let l = req_table(args, 0)?;
    let lk = key_columns(it, l, arg(args, 1))?;
    let r = join_table(it, arg(args, 2))?;
    let rk = key_columns(it, &r, arg(args, 3))?;
    let name = req_text(args, 4)?;
    check_no_comparers(comparers)?;
    if l.column_index(name).is_some() {
        return Err(MError::expression(format!(
            "The column '{name}' already exists in the table."
        )));
    }
    let m = match_rows(it, l, &lk, &r, &rk)?;
    let null_row = null_row_table(&r)?;
    let mut lrows: Vec<Option<usize>> = Vec::new();
    let mut nested: Vec<Lazy> = Vec::new();
    let mut push = |a: Option<usize>, t: TableValue| {
        lrows.push(a);
        nested.push(Lazy::Value(Value::Table(t)));
    };
    let left = |push: &mut dyn FnMut(Option<usize>, TableValue), matched: bool, empty: bool| {
        for (lr, ms) in m.left.iter().enumerate() {
            match (ms.is_empty(), matched) {
                (false, true) if empty => push(Some(lr), null_row.clone()),
                (false, true) => push(Some(lr), r.take_rows(ms)),
                (true, false) if empty => push(Some(lr), null_row.clone()),
                (true, false) => push(Some(lr), r.take_rows(ms)),
                _ => {}
            }
        }
    };
    let unmatched_right = m.right_rows(false);
    match kind {
        JoinKind::Inner => left(&mut push, true, false),
        JoinKind::LeftOuter => {
            for (lr, ms) in m.left.iter().enumerate() {
                push(Some(lr), r.take_rows(ms));
            }
        }
        JoinKind::LeftAnti => left(&mut push, false, true),
        JoinKind::LeftSemi => left(&mut push, true, true),
        JoinKind::RightOuter | JoinKind::FullOuter => {
            left(&mut push, true, false);
            if !unmatched_right.is_empty() {
                push(None, r.take_rows(&unmatched_right));
            }
            if kind == JoinKind::FullOuter {
                left(&mut push, false, true);
            }
        }
        JoinKind::RightAnti => {
            if !unmatched_right.is_empty() {
                push(None, r.take_rows(&unmatched_right));
            }
        }
        JoinKind::RightSemi => {
            let matched = m.right_rows(true);
            if !matched.is_empty() {
                push(None, r.take_rows(&matched));
            }
        }
    }
    let mut cols = side_columns(l, &lrows, kind.left_nullable(), &[]);
    cols.push((Arc::from(name), r.table_type(), Arc::new(nested)));
    table_value(build(it, cols)?)
}

// ---------------------------------------------------------------------------------------------
// Expanding nested values
// ---------------------------------------------------------------------------------------------

type Names = Vec<Arc<str>>;

/// `columnNames` and `newColumnNames` of an expand.
fn expand_names(
    it: &Interp,
    names_v: &Value,
    new_v: &Value,
    what: &str,
) -> Result<(Names, Names), MError> {
    let wanted = names(names_arg(it, names_v)?);
    let new = match new_v.plain() {
        Value::Null => wanted.clone(),
        v => names(names_arg(it, v)?),
    };
    if new.len() != wanted.len() {
        return Err(MError::expression(format!(
            "We expected newColumnNames to have the same number of items as {what}."
        )));
    }
    Ok((wanted, new))
}

/// Checks that replacing column `ci` with `new` gives unique column names.
fn check_expanded_names(t: &TableValue, ci: usize, new: &[Arc<str>]) -> Result<(), MError> {
    let mut all: Vec<Arc<str>> = t.column_names().to_vec();
    let tail = all.split_off(ci.saturating_add(1).min(all.len()));
    all.truncate(ci);
    all.extend(new.iter().cloned());
    all.extend(tail);
    Keys::try_new(all).map(|_| ())
}

fn expand_table_column(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let col = req_text(args, 1)?;
    let ci = t.column_index(col).ok_or_else(|| missing_column(col))?;
    let (wanted, new) = expand_names(it, arg(args, 2), arg(args, 3), "columnNames")?;
    check_expanded_names(t, ci, &new)?;
    let nested_types = column_type(t, ci);
    let types: Vec<TypeValue> = wanted
        .iter()
        .map(|n| {
            nested_types
                .table_columns()
                .and_then(|cols| cols.iter().find(|(cn, _)| cn == n))
                .map_or_else(TypeValue::any, |(_, ty)| ty.nullable())
        })
        .collect();

    let mut src_rows: Vec<usize> = Vec::new();
    let mut cells: Vec<Vec<Lazy>> = vec![Vec::new(); wanted.len()];
    for r in 0..t.row_count() {
        it.tick()?;
        let v = match t.cell(r, ci) {
            Some(c) => c.force(it)?,
            None => Value::Null,
        };
        match v.plain() {
            Value::Table(nt) if nt.row_count() > 0 => {
                let idx: Vec<Option<usize>> = wanted.iter().map(|n| nt.column_index(n)).collect();
                for nr in 0..nt.row_count() {
                    it.tick()?;
                    src_rows.push(r);
                    for (out, i) in cells.iter_mut().zip(&idx) {
                        out.push(
                            i.and_then(|i| nt.cell(nr, i).cloned())
                                .unwrap_or_else(null_cell),
                        );
                    }
                }
            }
            Value::Table(_) | Value::Null => {
                src_rows.push(r);
                for out in &mut cells {
                    out.push(null_cell());
                }
            }
            other => return Err(convert_error(other, "Table")),
        }
        it.check_rows(src_rows.len())?;
    }
    let base = t.take_rows(&src_rows);
    let new_cols = new
        .into_iter()
        .zip(types)
        .zip(cells)
        .map(|((n, ty), c)| (n, ty, Arc::new(c)))
        .collect();
    table_value(build(it, replace_column(&base, ci, new_cols))?)
}

fn expand_record_column(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let col = req_text(args, 1)?;
    let ci = t.column_index(col).ok_or_else(|| missing_column(col))?;
    let (wanted, new) = expand_names(it, arg(args, 2), arg(args, 3), "fieldNames")?;
    check_expanded_names(t, ci, &new)?;
    let record_type = column_type(t, ci);
    let types: Vec<TypeValue> = wanted
        .iter()
        .map(|n| match record_type.kind() {
            TypeKind::Record(rt) => rt
                .fields
                .iter()
                .find(|f| f.name == *n)
                .map_or_else(TypeValue::any, |f| f.ty.clone()),
            _ => TypeValue::any(),
        })
        .collect();
    let mut cells: Vec<Vec<Lazy>> = vec![Vec::with_capacity(t.row_count()); wanted.len()];
    for r in 0..t.row_count() {
        it.tick()?;
        let v = match t.cell(r, ci) {
            Some(c) => c.force(it)?,
            None => Value::Null,
        };
        match v.plain() {
            Value::Record(rec) => {
                for (out, n) in cells.iter_mut().zip(&wanted) {
                    out.push(rec.get(n).cloned().unwrap_or_else(null_cell));
                }
            }
            Value::Null => {
                for out in &mut cells {
                    out.push(null_cell());
                }
            }
            other => return Err(convert_error(other, "Record")),
        }
    }
    let new_cols = new
        .into_iter()
        .zip(types)
        .zip(cells)
        .map(|((n, ty), c)| (n, ty, Arc::new(c)))
        .collect();
    table_value(build(it, replace_column(t, ci, new_cols))?)
}

fn expand_list_column(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let col = req_text(args, 1)?;
    let ci = t.column_index(col).ok_or_else(|| missing_column(col))?;
    let mut src_rows: Vec<usize> = Vec::new();
    let mut cells: Vec<Lazy> = Vec::new();
    for r in 0..t.row_count() {
        it.tick()?;
        let v = match t.cell(r, ci) {
            Some(c) => c.force(it)?,
            None => Value::Null,
        };
        match v.plain() {
            Value::List(l) if !l.is_empty() => {
                for item in l.items() {
                    src_rows.push(r);
                    cells.push(item.clone());
                }
            }
            Value::List(_) | Value::Null => {
                src_rows.push(r);
                cells.push(null_cell());
            }
            other => return Err(convert_error(other, "Table")),
        }
        it.check_rows(src_rows.len())?;
    }
    let base = t.take_rows(&src_rows);
    let new_col = (column_name(t, ci), TypeValue::any(), Arc::new(cells));
    table_value(build(it, replace_column(&base, ci, vec![new_col]))?)
}

fn aggregate_table_column(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let col = req_text(args, 1)?;
    let ci = t.column_index(col).ok_or_else(|| missing_column(col))?;
    let specs = pairs(it, arg(args, 2))?
        .iter()
        .map(|s| match s.as_slice() {
            [c, f, n] => match f.plain() {
                Value::Function(f) => Ok((text_of(c)?, f.clone(), text_of(n)?)),
                other => Err(convert_error(other, "Function")),
            },
            _ => Err(MError::expression(
                "Table.AggregateTableColumn expects {column, function, name} lists.",
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let new_names: Vec<Arc<str>> = specs.iter().map(|s| s.2.clone()).collect();
    check_expanded_names(t, ci, &new_names)?;
    let mut cells: Vec<Vec<Lazy>> = vec![Vec::with_capacity(t.row_count()); specs.len()];
    for r in 0..t.row_count() {
        it.tick()?;
        let v = match t.cell(r, ci) {
            Some(c) => c.force(it)?,
            None => Value::Null,
        };
        match v.plain() {
            Value::Null => {
                for out in &mut cells {
                    out.push(null_cell());
                }
            }
            Value::Table(nt) => {
                for ((c, f, _), out) in specs.iter().zip(cells.iter_mut()) {
                    let values = if nt.row_count() == 0 {
                        vec![null_cell()]
                    } else {
                        let i = nt.column_index(c).ok_or_else(|| missing_column(c))?;
                        nt.column(i).map(<[Lazy]>::to_vec).unwrap_or_default()
                    };
                    out.push(catch(it.call(f, &[Value::List(ListValue::new(values))]))?);
                }
            }
            other => return Err(convert_error(other, "Table")),
        }
    }
    let new_cols = new_names
        .into_iter()
        .zip(cells)
        .map(|(n, c)| (n, TypeValue::any(), Arc::new(c)))
        .collect();
    table_value(build(it, replace_column(t, ci, new_cols))?)
}

// ---------------------------------------------------------------------------------------------
// Transpose, split and combine columns
// ---------------------------------------------------------------------------------------------

fn transpose(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let rows = t.row_count();
    let (names, types) = match arg(args, 1) {
        Value::Null => (default_names(rows), Vec::new()),
        v => column_spec(it, v, rows)?,
    };
    if names.len() != rows {
        return Err(MError::expression(format!(
            "The count of 'columns' ({rows}) doesn't match that of 'columnNames' ({}).",
            names.len()
        )));
    }
    it.check_rows(t.column_count())?;
    let mut columns = Vec::with_capacity(rows);
    for r in 0..rows {
        it.tick()?;
        columns.push(Arc::new(
            (0..t.column_count())
                .map(|c| t.cell(r, c).cloned().unwrap_or_else(null_cell))
                .collect::<Vec<_>>(),
        ));
    }
    table_value(TableValue::new(names, types, columns)?)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExtraValues {
    List,
    Error,
    Ignore,
}

/// A split cell: `None` for null, the parts, or the error of the cell or the splitter.
type SplitParts = Result<Option<Vec<Value>>, Arc<MError>>;

fn split_cell(
    it: &Interp,
    cell: Option<&Lazy>,
    splitter: &FunctionValue,
) -> Result<SplitParts, MError> {
    let soft = |e: MError| {
        if e.is_catchable() {
            Ok(Err(Arc::new(e)))
        } else {
            Err(e)
        }
    };
    let v = match cell.map_or(Ok(Value::Null), |c| c.force(it)) {
        Ok(v) if v.is_null() => return Ok(Ok(None)),
        Ok(v) => v,
        Err(e) => return soft(e),
    };
    match it.call(splitter, &[v]) {
        Ok(out) => match out.into_plain() {
            Value::List(l) => match items(it, &l) {
                Ok(parts) => Ok(Ok(Some(parts))),
                Err(e) => soft(e),
            },
            other => Ok(Err(Arc::new(convert_error(&other, "List")))),
        },
        Err(e) => soft(e),
    }
}

fn split_column(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let src = req_text(args, 1)?;
    let ci = t.column_index(src).ok_or_else(|| missing_column(src))?;
    let splitter = req_func(args, 2)?;
    let default = arg(args, 4).clone();
    let extra = match enum_arg(arg(args, 5), "ExtraValues", 3)? {
        Some(0) => ExtraValues::List,
        Some(1) => ExtraValues::Error,
        _ => ExtraValues::Ignore,
    };
    let mut parts: Vec<SplitParts> = Vec::with_capacity(t.row_count());
    for r in 0..t.row_count() {
        it.tick()?;
        parts.push(split_cell(it, t.cell(r, ci), splitter)?);
    }
    let numbered = |n: usize| -> Result<Vec<Arc<str>>, MError> {
        it.check_rows(n)?;
        Ok((1..=n).map(|i| Arc::from(format!("{src}.{i}"))).collect())
    };
    let new_names: Vec<Arc<str>> = match arg(args, 3).plain() {
        Value::Null => numbered(match parts.first() {
            Some(Ok(Some(p))) => p.len().max(1),
            _ => 1,
        })?,
        Value::Number(_) => numbered(count_arg(args, 3)?.unwrap_or(1))?,
        v => names(names_arg(it, v)?),
    };
    let width = new_names.len();
    let mut cells: Vec<Vec<Lazy>> = vec![Vec::with_capacity(parts.len()); width];
    for p in parts {
        // A null cell splits into no values.
        let row: Vec<Lazy> = match p.map(Option::unwrap_or_default) {
            // The last column holds the values from its position on, as a list (empty when
            // there are fewer values than columns).
            Ok(mut vals) if extra == ExtraValues::List && width > 0 => {
                let rest = if vals.len() >= width {
                    vals.split_off(width - 1)
                } else {
                    vals.resize(width - 1, default.clone());
                    Vec::new()
                };
                vals.push(Value::list(rest));
                vals.into_iter().map(Lazy::Value).collect()
            }
            Ok(mut vals) if vals.len() > width => match extra {
                ExtraValues::Error => vec![
                    Lazy::Error(Arc::new(MError::data_format(
                        "There were more columns in the result than expected."
                    )));
                    width
                ],
                _ => {
                    vals.truncate(width);
                    vals.into_iter().map(Lazy::Value).collect()
                }
            },
            Ok(mut vals) => {
                vals.resize(width, default.clone());
                vals.into_iter().map(Lazy::Value).collect()
            }
            Err(e) => vec![Lazy::Error(e); width],
        };
        for (out, cell) in cells.iter_mut().zip(row) {
            out.push(cell);
        }
    }
    let ty = TypeValue::text().nullable();
    let new_cols = new_names
        .into_iter()
        .zip(cells)
        .map(|(n, c)| (n, ty.clone(), Arc::new(c)))
        .collect();
    table_value(build(it, replace_column(t, ci, new_cols))?)
}

fn combine_columns(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let sources = key_columns(it, t, arg(args, 1))?;
    let combiner = req_func(args, 2)?;
    let name = req_text(args, 3)?;
    let mut cells = Vec::with_capacity(t.row_count());
    for r in 0..t.row_count() {
        it.tick()?;
        let values: Vec<Lazy> = sources
            .iter()
            .map(|&c| t.cell(r, c).cloned().unwrap_or_else(null_cell))
            .collect();
        cells.push(catch(
            it.call(combiner, &[Value::List(ListValue::new(values))]),
        )?);
    }
    let ty = if combiner.name().is_some_and(|n| n.starts_with("Combiner.")) {
        TypeValue::text()
    } else {
        TypeValue::any()
    };
    let new = (Arc::from(name), ty, Arc::new(cells));
    table_value(build(it, merge_columns(t, &sources, new))?)
}

fn combine_columns_to_record(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let name = req_text(args, 1)?;
    let sources = key_columns(it, t, arg(args, 2))?;
    let keys = Arc::new(Keys::try_new(
        sources.iter().map(|&c| column_name(t, c)).collect(),
    )?);
    let cells = (0..t.row_count())
        .map(|r| {
            let values = sources
                .iter()
                .map(|&c| t.cell(r, c).cloned().unwrap_or_else(null_cell))
                .collect();
            Lazy::Value(Value::Record(RecordValue::new(keys.clone(), values)))
        })
        .collect();
    let new = (
        Arc::from(name),
        TypeValue::primitive(PrimitiveType::Record),
        Arc::new(cells),
    );
    table_value(build(it, merge_columns(t, &sources, new))?)
}
