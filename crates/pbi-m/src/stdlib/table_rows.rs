//! `Table.*` functions that select, find, rank and reorganise rows, and table metadata
//! (`Table.Schema`, `Table.ColumnsOfType`).

use std::sync::Arc;

use super::args::{
    arg, catch, count_arg, func_arg, int_arg, items, num_arg, option, record_arg, req_func,
    req_list, req_table, req_text,
};
use super::table::{
    RowOrder, build, columns_of, condition_holds, distinct_criteria, is_ignore_case, leading,
    record_row, table_value,
};
use super::table_ops::{Column, NOT_ENOUGH, TOO_MANY, enum_arg, null_cell, row_key};
use super::value_fns::type_compatible;
use super::{Builder, missing_column};
use crate::eval::interp::{Interp, convert_error};
use crate::merror::MError;
use crate::value::ops::equals_with;
use crate::value::types::Facet;
use crate::value::{Lazy, ListValue, RecordValue, TableValue, TypeValue, Value};

pub(super) fn register(b: &mut Builder) {
    b.consts(&[
        ("RankKind.Competition", 0.0),
        ("RankKind.Dense", 1.0),
        ("RankKind.Ordinal", 2.0),
    ]);
    b.value("RankKind.Type", Value::Type(TypeValue::number()));
    b.func(
        "Table.Max",
        &["table", "comparisonCriteria", "?default"],
        |it, a| extreme(it, a, true),
    );
    b.func(
        "Table.Min",
        &["table", "comparisonCriteria", "?default"],
        |it, a| extreme(it, a, false),
    );
    b.func(
        "Table.MaxN",
        &["table", "comparisonCriteria", "countOrCondition"],
        |it, a| extreme_n(it, a, true),
    );
    b.func(
        "Table.MinN",
        &["table", "comparisonCriteria", "countOrCondition"],
        |it, a| extreme_n(it, a, false),
    );
    b.func(
        "Table.AddRankColumn",
        &["table", "newColumnName", "comparisonCriteria", "?options"],
        add_rank_column,
    );
    b.func("Table.Schema", &["table"], schema);
    b.func(
        "Table.ColumnsOfType",
        &["table", "listOfTypes"],
        columns_of_type,
    );
    b.func(
        "Table.ApproximateRowCount",
        &["table"],
        approximate_row_count,
    );
    b.func("Table.ToList", &["table", "?combiner"], to_list);
    b.func("Table.FirstValue", &["table", "?default"], first_value);
    b.func("Table.FromValue", &["value", "?options"], from_value);
    b.func("Table.PrefixColumns", &["table", "prefix"], prefix_columns);
    b.func(
        "Table.Partition",
        &["table", "column", "groups", "hash"],
        partition,
    );
    b.func("Table.Repeat", &["table", "count"], repeat);
    b.func("Table.ReverseRows", &["table"], reverse_rows);
    b.func(
        "Table.AlternateRows",
        &["table", "offset", "skip", "take"],
        alternate_rows,
    );
    b.func(
        "Table.InsertRows",
        &["table", "offset", "rows"],
        insert_rows,
    );
    b.func(
        "Table.RemoveRows",
        &["table", "offset", "?count"],
        remove_rows,
    );
    b.func(
        "Table.ReplaceRows",
        &["table", "offset", "count", "rows"],
        replace_rows,
    );
    b.func(
        "Table.RemoveLastN",
        &["table", "?countOrCondition"],
        remove_last_n,
    );
    b.func("Table.SingleRow", &["table"], single_row);
    b.func("Table.Split", &["table", "pageSize"], split);
    b.func("Table.SplitAt", &["table", "count"], split_at);
    b.func(
        "Table.IsDistinct",
        &["table", "?comparisonCriteria"],
        is_distinct,
    );
    b.func("Table.MatchesAllRows", &["table", "condition"], |it, a| {
        matches_rows(it, a, true)
    });
    b.func("Table.MatchesAnyRows", &["table", "condition"], |it, a| {
        matches_rows(it, a, false)
    });
    b.func("Table.FindText", &["table", "text"], find_text);
    b.func(
        "Table.Contains",
        &["table", "row", "?equationCriteria"],
        contains,
    );
    b.func(
        "Table.ContainsAll",
        &["table", "rows", "?equationCriteria"],
        |it, a| contains_rows(it, a, true),
    );
    b.func(
        "Table.ContainsAny",
        &["table", "rows", "?equationCriteria"],
        |it, a| contains_rows(it, a, false),
    );
    b.func(
        "Table.PositionOf",
        &["table", "row", "?occurrence", "?equationCriteria"],
        position_of,
    );
    b.func(
        "Table.PositionOfAny",
        &["table", "rows", "?occurrence", "?equationCriteria"],
        position_of_any,
    );
    b.func(
        "Table.RemoveMatchingRows",
        &["table", "rows", "?equationCriteria"],
        remove_matching_rows,
    );
    b.func(
        "Table.ReplaceMatchingRows",
        &["table", "replacements", "?equationCriteria"],
        replace_matching_rows,
    );
}

fn all_rows(t: &TableValue) -> Vec<usize> {
    (0..t.row_count()).collect()
}

// ---------------------------------------------------------------------------------------------
// Ordering: Max, Min, MaxN, MinN, AddRankColumn
// ---------------------------------------------------------------------------------------------

fn extreme(it: &Interp, args: &[Value], max: bool) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let order = RowOrder::new(it, t, arg(args, 1))?;
    let mut best: Option<usize> = None;
    for r in 0..t.row_count() {
        it.tick()?;
        // Ties keep the last row, as Desktop does.
        let take = match best {
            None => true,
            Some(b) => {
                let o = order.compare(r, b)?;
                if max { o.is_ge() } else { o.is_le() }
            }
        };
        if take {
            best = Some(r);
        }
    }
    Ok(best
        .and_then(|r| t.row(r))
        .map_or_else(|| arg(args, 2).clone(), Value::Record))
}

fn extreme_n(it: &Interp, args: &[Value], max: bool) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let order = RowOrder::new(it, t, arg(args, 1))?.sorted(max)?;
    let sorted = t.take_rows(&order);
    let n = leading(it, &sorted, arg(args, 2), 0)?.min(sorted.row_count());
    table_value(sorted.take_rows(&(0..n).collect::<Vec<_>>()))
}

fn add_rank_column(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let name = req_text(args, 1)?;
    super::table::check_new_name(t, name)?;
    let kind = match option(it, record_arg(args, 3)?, "RankKind")? {
        Some(v) => enum_arg(&v, "RankKind", 3)?.unwrap_or(0),
        None => 0,
    };
    let order = RowOrder::new(it, t, arg(args, 2))?;
    let sorted = order.sorted(false)?;
    let mut ranks = Vec::with_capacity(sorted.len());
    let (mut competition, mut dense) = (0usize, 0usize);
    let mut prev: Option<usize> = None;
    for (i, &r) in sorted.iter().enumerate() {
        it.tick()?;
        let tie = match prev {
            Some(p) => order.compare(p, r)?.is_eq(),
            None => false,
        };
        if !tie {
            competition = i + 1;
            dense += 1;
        }
        let rank = match kind {
            1 => dense,
            2 => i + 1,
            _ => competition,
        };
        ranks.push(Lazy::Value(Value::Number(rank as f64)));
        prev = Some(r);
    }
    let mut cols = columns_of(&t.take_rows(&sorted));
    cols.push((
        Arc::from(name),
        TypeValue::facet(Facet::Int64).nullable(),
        Arc::new(ranks),
    ));
    table_value(build(it, cols)?)
}

// ---------------------------------------------------------------------------------------------
// Metadata
// ---------------------------------------------------------------------------------------------

const SCHEMA_COLUMNS: [&str; 19] = [
    "Name",
    "Position",
    "TypeName",
    "Kind",
    "IsNullable",
    "NumericPrecisionBase",
    "NumericPrecision",
    "NumericScale",
    "IsSigned",
    "DateTimePrecision",
    "IsUtc",
    "MaxLength",
    "IsVariableLength",
    "NativeTypeName",
    "NativeDefaultExpression",
    "NativeExpression",
    "Description",
    "IsWritable",
    "FieldCaption",
];

fn type_name(ty: &TypeValue) -> String {
    match ty.facet_kind() {
        Some(f) => f.type_name().to_owned(),
        None => format!("{}.Type", ty.message_name()),
    }
}

fn schema(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let rows = t
        .column_names()
        .iter()
        .zip(t.column_types())
        .enumerate()
        .map(|(i, (n, ty))| {
            let mut row = vec![
                Value::Text(n.clone()),
                Value::Number(i as f64),
                Value::text(type_name(ty)),
                Value::text(ty.base().name()),
                Value::Logical(ty.is_nullable()),
            ];
            row.resize(SCHEMA_COLUMNS.len(), Value::Null);
            row.into_iter().map(Lazy::Value).collect()
        })
        .collect();
    let any = TypeValue::any();
    let mut types = vec![any.clone(); SCHEMA_COLUMNS.len()];
    let fixed = [
        TypeValue::text(),
        TypeValue::number(),
        TypeValue::text(),
        TypeValue::text(),
        TypeValue::primitive(crate::ast::PrimitiveType::Logical),
    ];
    for (slot, ty) in types.iter_mut().zip(fixed) {
        *slot = ty;
    }
    let names = SCHEMA_COLUMNS.iter().map(|n| Arc::from(*n)).collect();
    table_value(TableValue::from_rows(names, rows)?.with_types(types))
}

fn columns_of_type(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let mut wanted = Vec::new();
    for v in items(it, req_list(args, 1)?)? {
        match v {
            Value::Type(ty) => wanted.push(ty),
            other => return Err(convert_error(&other, "Type")),
        }
    }
    Ok(Value::list(
        t.column_names()
            .iter()
            .zip(t.column_types())
            .filter(|(_, ty)| wanted.iter().any(|w| type_compatible(ty, w)))
            .map(|(n, _)| Value::Text(n.clone())),
    ))
}

fn approximate_row_count(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::Number(req_table(args, 0)?.row_count() as f64))
}

// ---------------------------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------------------------

fn to_list(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let combiner = func_arg(args, 1)?;
    let mut out = Vec::with_capacity(t.row_count());
    for r in 0..t.row_count() {
        it.tick()?;
        let cells: Vec<Lazy> = (0..t.column_count())
            .map(|c| t.cell(r, c).cloned().unwrap_or_else(null_cell))
            .collect();
        let v = match combiner {
            Some(f) => it.call(f, &[Value::List(ListValue::new(cells))]),
            None => join_texts(it, &cells),
        };
        out.push(catch(v)?);
    }
    Ok(Value::List(ListValue::new(out)))
}

/// The default `Table.ToList` combiner: texts joined with commas (null as empty text).
fn join_texts(it: &Interp, cells: &[Lazy]) -> Result<Value, MError> {
    let mut s = String::new();
    for (i, c) in cells.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        match c.force(it)?.plain() {
            Value::Null => {}
            Value::Text(t) => s.push_str(t),
            other => return Err(convert_error(other, "Text")),
        }
    }
    Ok(Value::text(s))
}

fn first_value(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    match t.cell(0, 0) {
        Some(c) => c.force(it),
        None => Ok(arg(args, 1).clone()),
    }
}

fn from_value(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let value_name: Arc<str> = match option(it, record_arg(args, 1)?, "DefaultColumnName")? {
        Some(Value::Text(n)) => n,
        Some(other) => return Err(convert_error(&other, "Text")),
        None => Arc::from("Value"),
    };
    let v = arg(args, 0);
    let (names, rows): (Vec<Arc<str>>, Vec<Vec<Lazy>>) = match v.plain() {
        Value::Table(t) => return table_value(t.clone()),
        // `DefaultColumnName` names the column of a list or a single value, not a record's.
        Value::Record(r) => (
            vec![Arc::from("Name"), Arc::from("Value")],
            r.iter()
                .map(|(n, c)| vec![Lazy::Value(Value::Text(n.clone())), c.clone()])
                .collect(),
        ),
        Value::List(l) => (
            vec![value_name],
            l.items().iter().map(|c| vec![c.clone()]).collect(),
        ),
        other => (vec![value_name], vec![vec![Lazy::Value(other.clone())]]),
    };
    it.check_rows(rows.len())?;
    table_value(TableValue::from_rows(names, rows)?)
}

fn prefix_columns(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let prefix = req_text(args, 1)?;
    let names = t
        .column_names()
        .iter()
        .map(|n| Arc::from(format!("{prefix}.{n}")))
        .collect();
    table_value(TableValue::new(
        names,
        t.column_types().to_vec(),
        t.columns().to_vec(),
    )?)
}

// ---------------------------------------------------------------------------------------------
// Reorganising rows
// ---------------------------------------------------------------------------------------------

fn partition(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let col = req_text(args, 1)?;
    let ci = t.column_index(col).ok_or_else(|| missing_column(col))?;
    let groups = count_arg(args, 2)?.unwrap_or(0);
    let hash = req_func(args, 3)?;
    it.check_rows(groups)?;
    let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); groups];
    if groups > 0 {
        for r in 0..t.row_count() {
            it.tick()?;
            let v = row_key(it, t, r, &[ci])?.pop().unwrap_or(Value::Null);
            let h = match it.call(hash, &[v])?.into_plain() {
                Value::Number(n) => n,
                other => return Err(convert_error(&other, "Number")),
            };
            // `Number.Mod`: a row whose hash modulo `groups` is negative or fractional is in no
            // partition (Desktop drops it).
            let m = h % groups as f64;
            if m >= 0.0
                && m.fract() == 0.0
                && let Some(bucket) = buckets.get_mut(m as usize)
            {
                bucket.push(r);
            }
        }
    }
    Ok(Value::list(
        buckets.iter().map(|rows| Value::Table(t.take_rows(rows))),
    ))
}

/// A non-negative integer argument named `name` in Desktop's out-of-range error.
fn range_arg(args: &[Value], i: usize, name: &str) -> Result<Option<usize>, MError> {
    match int_arg(args, i)? {
        None => Ok(None),
        Some(n) => usize::try_from(n).map(Some).map_err(|_| out_of_range(name)),
    }
}

fn out_of_range(name: &str) -> MError {
    MError::expression(format!("The '{name}' argument is out of range."))
}

fn repeat(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let count = range_arg(args, 1, "count")?.unwrap_or(0);
    if count == 0 {
        return table_value(TableValue::empty(Vec::new())?);
    }
    let total = t.row_count().saturating_mul(count);
    it.check_rows(total)?;
    let rows: Vec<usize> = (0..count).flat_map(|_| 0..t.row_count()).collect();
    table_value(t.take_rows(&rows))
}

fn reverse_rows(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    table_value(t.take_rows(&(0..t.row_count()).rev().collect::<Vec<_>>()))
}

fn alternate_rows(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let n = t.row_count();
    let offset = range_arg(args, 1, "offset")?.unwrap_or(0).min(n);
    let skip = range_arg(args, 2, "skip")?.unwrap_or(0);
    let take = range_arg(args, 3, "take")?.unwrap_or(0);
    let mut rows: Vec<usize> = (0..offset).collect();
    if skip.saturating_add(take) > 0 {
        let mut i = offset;
        while i < n {
            it.tick()?;
            i = i.saturating_add(skip);
            let end = i.saturating_add(take).min(n);
            rows.extend(i.min(n)..end);
            i = end;
        }
    }
    table_value(t.take_rows(&rows))
}

fn missing_field(n: &str) -> MError {
    MError::expression(format!("The field '{n}' of the record wasn't found."))
}

/// The cells of `records` laid out as rows of `t` (fields matched by column name).
fn record_rows(it: &Interp, t: &TableValue, v: &Value) -> Result<Vec<Vec<Lazy>>, MError> {
    let Value::List(l) = v.plain() else {
        return Err(convert_error(v, "List"));
    };
    let mut out = Vec::with_capacity(l.len());
    for item in items(it, l)? {
        let Value::Record(rec) = item else {
            return Err(convert_error(&item, "Record"));
        };
        out.push(record_row(&rec, t.column_names()));
    }
    Ok(out)
}

/// `t` with rows `offset..offset + remove` replaced by `insert`.
fn splice_rows(
    it: &Interp,
    t: &TableValue,
    offset: usize,
    remove: usize,
    insert: Vec<Vec<Lazy>>,
) -> Result<Value, MError> {
    let n = t.row_count();
    if offset > n || offset.saturating_add(remove) > n {
        return Err(MError::expression(NOT_ENOUGH));
    }
    let total = n - remove + insert.len();
    it.check_rows(total)?;
    let mut cols: Vec<Column> = columns_of(t);
    for (c, col) in cols.iter_mut().enumerate() {
        let mut cells = Vec::with_capacity(total);
        cells.extend(col.2.iter().take(offset).cloned());
        cells.extend(
            insert
                .iter()
                .map(|row| row.get(c).cloned().unwrap_or_else(null_cell)),
        );
        cells.extend(col.2.iter().skip(offset + remove).cloned());
        col.2 = Arc::new(cells);
    }
    if cols.is_empty() {
        return table_value(t.clone());
    }
    table_value(build(it, cols)?)
}

fn insert_rows(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let offset = count_arg(args, 1)?.unwrap_or(0);
    let rows = record_rows(it, t, arg(args, 2))?;
    splice_rows(it, t, offset, 0, rows)
}

fn remove_rows(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let offset = count_arg(args, 1)?.unwrap_or(0);
    let count = count_arg(args, 2)?.unwrap_or(1);
    splice_rows(it, t, offset, count, Vec::new())
}

fn replace_rows(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let offset = count_arg(args, 1)?.unwrap_or(0);
    let count = count_arg(args, 2)?.unwrap_or(0);
    let rows = record_rows(it, t, arg(args, 3))?;
    splice_rows(it, t, offset, count, rows)
}

fn remove_last_n(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let rows = t.row_count();
    let n = match arg(args, 1).plain() {
        Value::Function(f) => {
            let mut n = 0;
            for i in (0..rows).rev() {
                it.tick()?;
                let Some(row) = t.row(i) else { break };
                if !condition_holds(it, f, Value::Record(row))? {
                    break;
                }
                n += 1;
            }
            n
        }
        _ => count_arg(args, 1)?.unwrap_or(1),
    }
    .min(rows);
    table_value(t.take_rows(&(0..rows - n).collect::<Vec<_>>()))
}

fn single_row(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    match t.row_count() {
        0 => Err(MError::expression(NOT_ENOUGH)),
        1 => t
            .row(0)
            .map(Value::Record)
            .ok_or_else(|| MError::expression(NOT_ENOUGH)),
        _ => Err(MError::expression(TOO_MANY)),
    }
}

fn split(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let size = range_arg(args, 1, "pageSize")?.unwrap_or(0);
    if size == 0 {
        return Err(out_of_range("pageSize"));
    }
    let all = all_rows(t);
    Ok(Value::list(
        all.chunks(size).map(|rows| Value::Table(t.take_rows(rows))),
    ))
}

fn split_at(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let n = count_arg(args, 1)?.unwrap_or(0).min(t.row_count());
    let all = all_rows(t);
    let (a, b) = all.split_at(n);
    Ok(Value::list([
        Value::Table(t.take_rows(a)),
        Value::Table(t.take_rows(b)),
    ]))
}

// ---------------------------------------------------------------------------------------------
// Row predicates and lookups
// ---------------------------------------------------------------------------------------------

fn is_distinct(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let (cols, ignore_case) = distinct_criteria(it, t, arg(args, 1))?;
    let mut index = super::table_ops::KeyGroups::new(ignore_case);
    for r in 0..t.row_count() {
        it.tick()?;
        let key = row_key(it, t, r, &cols)?;
        if index.find(it, &key)?.is_some() {
            return Ok(Value::Logical(false));
        }
        index.insert(key);
    }
    Ok(Value::Logical(true))
}

fn matches_rows(it: &Interp, args: &[Value], all: bool) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let f = req_func(args, 1)?;
    for row in t.rows() {
        it.tick()?;
        if condition_holds(it, f, Value::Record(row))? != all {
            return Ok(Value::Logical(!all));
        }
    }
    Ok(Value::Logical(all))
}

fn find_text(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let needle = req_text(args, 1)?;
    let mut keep = Vec::new();
    for r in 0..t.row_count() {
        it.tick()?;
        for c in 0..t.column_count() {
            let v = match t.cell(r, c) {
                Some(cell) => cell.force(it)?,
                None => Value::Null,
            };
            if has_text(it, &v, needle)? {
                keep.push(r);
                break;
            }
        }
    }
    table_value(t.take_rows(&keep))
}

/// Whether `v` is a text containing `needle` or a list or record holding one (Desktop looks
/// inside lists and records, not tables, binaries or other values).
fn has_text(it: &Interp, v: &Value, needle: &str) -> Result<bool, MError> {
    let _depth = it.enter()?;
    match v.plain() {
        Value::Text(s) => Ok(s.contains(needle)),
        Value::List(l) => {
            for x in items(it, l)? {
                if has_text(it, &x, needle)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Value::Record(rec) => {
            for (_, c) in rec.iter() {
                it.tick()?;
                if has_text(it, &c.force(it)?, needle)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

/// `equationCriteria` of the row lookups: the columns to compare (default: the record's
/// fields) and whether texts compare ignoring case.
struct RowCriteria {
    columns: Option<Vec<Arc<str>>>,
    ignore_case: bool,
    /// Without columns, whether a row must equal the whole record (`Table.PositionOf`) rather
    /// than only agree on the record's fields (`Table.Contains`).
    whole_row: bool,
}

fn row_criteria(it: &Interp, v: &Value, whole_row: bool) -> Result<RowCriteria, MError> {
    let mut columns = Vec::new();
    let mut ignore_case = false;
    let mut add = |x: &Value| -> Result<(), MError> {
        match x.plain() {
            Value::Text(n) => columns.push(n.clone()),
            Value::Function(_) => ignore_case |= is_ignore_case(x),
            other => return Err(convert_error(other, "Text")),
        }
        Ok(())
    };
    match v.plain() {
        Value::Null => {}
        Value::List(l) => {
            for x in items(it, l)? {
                match &x {
                    Value::List(inner) => {
                        for y in items(it, inner)? {
                            add(&y)?;
                        }
                    }
                    other => add(other)?,
                }
            }
        }
        other => add(other)?,
    }
    Ok(RowCriteria {
        columns: (!columns.is_empty()).then_some(columns),
        ignore_case,
        whole_row,
    })
}

fn records_arg(it: &Interp, v: &Value) -> Result<Vec<RecordValue>, MError> {
    let Value::List(l) = v.plain() else {
        return Err(convert_error(v, "List"));
    };
    items(it, l)?
        .into_iter()
        .map(|x| match x {
            Value::Record(r) => Ok(r),
            other => Err(convert_error(&other, "Record")),
        })
        .collect()
}

fn record_of(v: &Value) -> Result<&RecordValue, MError> {
    match v.plain() {
        Value::Record(r) => Ok(r),
        other => Err(convert_error(other, "Record")),
    }
}

/// Whether row `r` of `t` equals `rec` under `crit`.
fn row_matches(
    it: &Interp,
    t: &TableValue,
    r: usize,
    rec: &RecordValue,
    crit: &RowCriteria,
) -> Result<bool, MError> {
    let cell = |c: usize| match t.cell(r, c) {
        Some(cell) => cell.force(it),
        None => Ok(Value::Null),
    };
    match &crit.columns {
        Some(cols) => {
            for n in cols {
                let want = rec.get(n).ok_or_else(|| missing_field(n))?;
                // The row is compared as a record, so a missing column is a missing field.
                let c = t.column_index(n).ok_or_else(|| missing_field(n))?;
                if !equals_with(it, &cell(c)?, &want.force(it)?, crit.ignore_case)? {
                    return Ok(false);
                }
            }
        }
        None => {
            if crit.whole_row && rec.len() != t.column_count() {
                return Ok(false);
            }
            for (n, want) in rec.iter() {
                let Some(c) = t.column_index(n) else {
                    return Ok(false);
                };
                if !equals_with(it, &cell(c)?, &want.force(it)?, crit.ignore_case)? {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

/// Whether row `r` matches any of `recs`.
fn row_matches_any(
    it: &Interp,
    t: &TableValue,
    r: usize,
    recs: &[RecordValue],
    crit: &RowCriteria,
) -> Result<bool, MError> {
    for rec in recs {
        if row_matches(it, t, r, rec, crit)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn contains(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let rec = record_of(arg(args, 1))?;
    let crit = row_criteria(it, arg(args, 2), false)?;
    for r in 0..t.row_count() {
        it.tick()?;
        if row_matches(it, t, r, rec, &crit)? {
            return Ok(Value::Logical(true));
        }
    }
    Ok(Value::Logical(false))
}

fn contains_rows(it: &Interp, args: &[Value], all: bool) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let recs = records_arg(it, arg(args, 1))?;
    let crit = row_criteria(it, arg(args, 2), false)?;
    for rec in &recs {
        let mut found = false;
        for r in 0..t.row_count() {
            it.tick()?;
            if row_matches(it, t, r, rec, &crit)? {
                found = true;
                break;
            }
        }
        if found != all {
            return Ok(Value::Logical(found));
        }
    }
    Ok(Value::Logical(all))
}

/// `Occurrence.First` (default), `Occurrence.Last` or `Occurrence.All` of the matching rows.
fn occurrences(args: &[Value], i: usize, positions: Vec<usize>) -> Result<Value, MError> {
    let pos = |p: Option<&usize>| Value::Number(p.map_or(-1.0, |&p| p as f64));
    Ok(match num_arg(args, i)? {
        Some(1.0) => pos(positions.last()),
        Some(2.0) => Value::list(positions.into_iter().map(|p| Value::Number(p as f64))),
        _ => pos(positions.first()),
    })
}

fn position_of(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let rec = record_of(arg(args, 1))?;
    let crit = row_criteria(it, arg(args, 3), true)?;
    let mut positions = Vec::new();
    for r in 0..t.row_count() {
        it.tick()?;
        if row_matches(it, t, r, rec, &crit)? {
            positions.push(r);
        }
    }
    occurrences(args, 2, positions)
}

fn position_of_any(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let recs = records_arg(it, arg(args, 1))?;
    let crit = row_criteria(it, arg(args, 3), true)?;
    let mut positions = Vec::new();
    for r in 0..t.row_count() {
        it.tick()?;
        if row_matches_any(it, t, r, &recs, &crit)? {
            positions.push(r);
        }
    }
    occurrences(args, 2, positions)
}

fn remove_matching_rows(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let recs = records_arg(it, arg(args, 1))?;
    let crit = row_criteria(it, arg(args, 2), true)?;
    let mut keep = Vec::new();
    for r in 0..t.row_count() {
        it.tick()?;
        if !row_matches_any(it, t, r, &recs, &crit)? {
            keep.push(r);
        }
    }
    table_value(t.take_rows(&keep))
}

fn replace_matching_rows(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let Value::List(l) = arg(args, 1).plain() else {
        return Err(convert_error(arg(args, 1), "List"));
    };
    let list = items(it, l)?;
    let pair_values: Vec<Value> = if matches!(list.first(), Some(Value::Record(_))) {
        vec![Value::list(list)]
    } else {
        list
    };
    let mut replacements: Vec<(RecordValue, RecordValue)> = Vec::new();
    for p in &pair_values {
        let Value::List(pl) = p else {
            return Err(convert_error(p, "List"));
        };
        match items(it, pl)?.as_slice() {
            [Value::Record(old), Value::Record(new)] => {
                replacements.push((old.clone(), new.clone()));
            }
            _ => {
                return Err(MError::expression(
                    "Table.ReplaceMatchingRows expects {oldRow, newRow} pairs of records.",
                ));
            }
        }
    }
    let crit = row_criteria(it, arg(args, 2), true)?;
    let names = t.column_names().to_vec();
    let mut rows = Vec::with_capacity(t.row_count());
    for r in 0..t.row_count() {
        it.tick()?;
        let mut replacement = None;
        for (old, new) in &replacements {
            if row_matches(it, t, r, old, &crit)? {
                replacement = Some(new);
                break;
            }
        }
        rows.push(match replacement {
            Some(new) => record_row(new, &names),
            None => (0..t.column_count())
                .map(|c| t.cell(r, c).cloned().unwrap_or_else(null_cell))
                .collect::<Vec<_>>(),
        });
    }
    table_value(TableValue::from_rows(names, rows)?.with_types(t.column_types().to_vec()))
}
