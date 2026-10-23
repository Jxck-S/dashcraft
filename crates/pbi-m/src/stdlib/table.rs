//! `Table.*` functions and `#table`.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hasher};
use std::sync::Arc;

use super::args::{
    arg, catch, count_arg, func_arg, items, names_arg, option, record_arg, req_func, req_list,
    req_table, req_text,
};
use super::convert::convert_to_type;
use super::{Builder, MissingField, missing_column};
use crate::eval::interp::{Interp, convert_error};
use crate::merror::MError;
use crate::value::format::value_to_text;
use crate::value::ops::{combine_tables, equals_with, hash_value, sort_compare};
use crate::value::{
    FunctionKind, FunctionValue, Lazy, ListValue, RecordValue, TableValue, TypeValue, Value,
};

pub(super) fn register(b: &mut Builder) {
    b.func("#table", &["columns", "rows"], hash_table);
    b.func(
        "Table.PromoteHeaders",
        &["table", "?options"],
        promote_headers,
    );
    b.func("Table.DemoteHeaders", &["table"], demote_headers);
    b.func("Table.SelectRows", &["table", "condition"], select_rows);
    b.func(
        "Table.RenameColumns",
        &["table", "renames", "?missingField"],
        rename_columns,
    );
    b.func(
        "Table.TransformColumns",
        &[
            "table",
            "transformOperations",
            "?defaultTransformation",
            "?missingField",
        ],
        transform_columns,
    );
    b.func(
        "Table.TransformColumnTypes",
        &["table", "typeTransformations", "?culture"],
        transform_column_types,
    );
    b.func(
        "Table.AddColumn",
        &["table", "newColumnName", "columnGenerator", "?columnType"],
        add_column,
    );
    b.func(
        "Table.AddIndexColumn",
        &[
            "table",
            "newColumnName",
            "?initialValue",
            "?increment",
            "?columnType",
        ],
        add_index_column,
    );
    b.func(
        "Table.DuplicateColumn",
        &["table", "columnName", "newColumnName", "?columnType"],
        duplicate_column,
    );
    b.func("Table.Distinct", &["table", "?equationCriteria"], distinct);
    b.func(
        "Table.SelectColumns",
        &["table", "columns", "?missingField"],
        select_columns_fn,
    );
    b.func(
        "Table.RemoveColumns",
        &["table", "columns", "?missingField"],
        remove_columns,
    );
    b.func(
        "Table.ReorderColumns",
        &["table", "columnOrder", "?missingField"],
        reorder_columns,
    );
    b.func(
        "Table.TransformColumnNames",
        &["table", "nameGenerator", "?options"],
        transform_column_names,
    );
    b.func("Table.RowCount", &["table"], row_count);
    b.func("Table.ColumnCount", &["table"], column_count);
    b.func("Table.ColumnNames", &["table"], column_names);
    b.func("Table.HasColumns", &["table", "columns"], has_columns);
    b.func("Table.Column", &["table", "column"], column);
    b.func("Table.IsEmpty", &["table"], is_empty);
    b.func("Table.FromRows", &["rows", "?columns"], from_rows);
    b.func(
        "Table.FromRecords",
        &["records", "?columns", "?missingField"],
        from_records,
    );
    b.func("Table.FromColumns", &["lists", "?columns"], from_columns);
    b.func(
        "Table.FromList",
        &["list", "?splitter", "?columns", "?default", "?extraValues"],
        from_list,
    );
    b.func("Table.ToRecords", &["table"], to_records);
    b.func("Table.ToRows", &["table"], to_rows);
    b.func("Table.ToColumns", &["table"], to_columns);
    b.func(
        "Table.TransformRows",
        &["table", "transform"],
        transform_rows,
    );
    b.func("Table.FirstN", &["table", "countOrCondition"], first_n);
    b.func("Table.LastN", &["table", "countOrCondition"], last_n);
    b.func("Table.Skip", &["table", "?countOrCondition"], skip);
    b.func("Table.RemoveFirstN", &["table", "?countOrCondition"], skip);
    b.func("Table.Range", &["table", "offset", "?count"], range);
    b.func("Table.First", &["table", "?default"], first);
    b.func("Table.Last", &["table", "?default"], last);
    b.func("Table.Sort", &["table", "comparisonCriteria"], sort);
    b.func(
        "Table.ReplaceValue",
        &[
            "table",
            "oldValue",
            "newValue",
            "replacer",
            "columnsToSearch",
        ],
        replace_value,
    );
    b.func("Table.Combine", &["tables", "?columns"], combine);
    b.func("Table.FillDown", &["table", "columns"], fill_down);
    b.func("Table.FillUp", &["table", "columns"], fill_up);
    b.func("Table.Buffer", &["table", "?options"], identity);
    b.func("Table.StopFolding", &["table"], identity);
    b.func(
        "Table.ReplaceErrorValues",
        &["table", "errorReplacement"],
        replace_error_values,
    );
    b.func(
        "Table.RemoveRowsWithErrors",
        &["table", "?columns"],
        remove_rows_with_errors,
    );
    b.func(
        "Table.SelectRowsWithErrors",
        &["table", "?columns"],
        select_rows_with_errors,
    );
}

fn identity(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(arg(args, 0).clone())
}

fn names(v: Vec<String>) -> Vec<Arc<str>> {
    v.into_iter().map(Arc::from).collect()
}

fn missing_arg(args: &[Value], i: usize) -> Result<MissingField, MError> {
    MissingField::from_arg(arg(args, i))
}

fn table_value(t: TableValue) -> Result<Value, MError> {
    Ok(Value::Table(t))
}

/// Builds a table whose columns are (name, type, cells).
fn build(
    it: &Interp,
    cols: Vec<(Arc<str>, TypeValue, Arc<Vec<Lazy>>)>,
) -> Result<TableValue, MError> {
    let rows = cols.first().map_or(0, |c| c.2.len());
    it.check_rows(rows)?;
    let mut names = Vec::with_capacity(cols.len());
    let mut types = Vec::with_capacity(cols.len());
    let mut cells = Vec::with_capacity(cols.len());
    for (n, t, c) in cols {
        names.push(n);
        types.push(t);
        cells.push(c);
    }
    TableValue::new(names, types, cells)
}

fn columns_of(t: &TableValue) -> Vec<(Arc<str>, TypeValue, Arc<Vec<Lazy>>)> {
    t.column_names()
        .iter()
        .zip(t.column_types())
        .zip(t.columns())
        .map(|((n, ty), c)| (n.clone(), ty.clone(), c.clone()))
        .collect()
}

fn null_column(rows: usize) -> Arc<Vec<Lazy>> {
    Arc::new(vec![Lazy::Value(Value::Null); rows])
}

/// `Table.SelectColumns` semantics, also used by `table[[a], [b]]`.
pub(crate) fn select_columns(
    t: &TableValue,
    names: &[Arc<str>],
    missing: MissingField,
) -> Result<TableValue, MError> {
    let mut cols = Vec::with_capacity(names.len());
    for n in names {
        match t.column_index(n) {
            Some(i) => cols.push((
                n.clone(),
                t.column_type(i).cloned().unwrap_or_else(TypeValue::any),
                t.columns()[i].clone(),
            )),
            None => match missing {
                MissingField::Error => return Err(missing_column(n)),
                MissingField::Ignore => {}
                MissingField::UseNull => {
                    cols.push((n.clone(), TypeValue::any(), null_column(t.row_count())));
                }
            },
        }
    }
    let mut names = Vec::with_capacity(cols.len());
    let mut types = Vec::with_capacity(cols.len());
    let mut cells = Vec::with_capacity(cols.len());
    for (n, ty, c) in cols {
        names.push(n);
        types.push(ty);
        cells.push(c);
    }
    TableValue::new(names, types, cells)
}

/// Forces a list of rows (lists) into cells.
fn row_cells(it: &Interp, row: &Value, width: usize, index: usize) -> Result<Vec<Lazy>, MError> {
    let Value::List(l) = row.plain() else {
        return Err(convert_error(row, "List"));
    };
    if l.len() != width {
        return Err(MError::expression(format!(
            "Row {} has {} values but the table has {width} columns.",
            index + 1,
            l.len()
        )));
    }
    l.items().iter().map(|c| catch(c.force(it))).collect()
}

/// Column names and types from a `columns` argument: list of names, count or table type.
fn column_spec(
    it: &Interp,
    v: &Value,
    default_width: usize,
) -> Result<(Vec<Arc<str>>, Vec<TypeValue>), MError> {
    match v.plain() {
        Value::Null => Ok((default_names(default_width), Vec::new())),
        Value::Number(n) if n.fract() == 0.0 && *n >= 0.0 => {
            let n = *n as usize;
            it.check_rows(n)?;
            Ok((default_names(n), Vec::new()))
        }
        Value::List(_) => Ok((names(names_arg(it, v)?), Vec::new())),
        Value::Type(t) => match t.table_columns() {
            Some(cols) => Ok((
                cols.iter().map(|(n, _)| n.clone()).collect(),
                cols.iter().map(|(_, t)| t.clone()).collect(),
            )),
            None => match t.kind() {
                crate::value::TypeKind::Record(r) => Ok((
                    r.fields.iter().map(|f| f.name.clone()).collect(),
                    r.fields.iter().map(|f| f.ty.clone()).collect(),
                )),
                _ => Err(convert_error(v, "table type")),
            },
        },
        other => Err(convert_error(other, "List")),
    }
}

fn default_names(n: usize) -> Vec<Arc<str>> {
    (1..=n).map(|i| Arc::from(format!("Column{i}"))).collect()
}

fn hash_table(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let rows = items(it, req_list(args, 1)?)?;
    let first_width = rows
        .first()
        .and_then(|r| r.as_list().map(ListValue::len))
        .unwrap_or(0);
    let (names, types) = column_spec(it, arg(args, 0), first_width)?;
    it.check_rows(rows.len())?;
    let mut out = Vec::with_capacity(rows.len());
    for (i, r) in rows.iter().enumerate() {
        it.tick()?;
        out.push(row_cells(it, r, names.len(), i)?);
    }
    let t = TableValue::from_rows(names, out)?;
    table_value(t.with_types(types))
}

fn promote_headers(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let all_scalars = option(it, record_arg(args, 1)?, "PromoteAllScalars")?
        .and_then(|v| v.as_logical())
        .unwrap_or(false);
    if t.row_count() == 0 {
        return Ok(Value::Table(t.clone()));
    }
    let mut new_names: Vec<Arc<str>> = Vec::with_capacity(t.column_count());
    for (i, old) in t.column_names().iter().enumerate() {
        let header = match t.cell(0, i) {
            Some(c) => c.force(it)?,
            None => Value::Null,
        };
        let name = match header.plain() {
            Value::Text(s) if !s.is_empty() => Some(s.to_string()),
            Value::Text(_) | Value::Null => None,
            other if all_scalars => value_to_text(other),
            _ => None,
        };
        let base: Arc<str> = name.map_or_else(|| old.clone(), Arc::from);
        let mut candidate = base.clone();
        let mut k = 1;
        while new_names.contains(&candidate) {
            candidate = Arc::from(format!("{base}_{k}"));
            k += 1;
        }
        new_names.push(candidate);
    }
    let columns = t
        .columns()
        .iter()
        .map(|c| Arc::new(c.get(1..).map(<[Lazy]>::to_vec).unwrap_or_default()))
        .collect();
    table_value(TableValue::new(new_names, Vec::new(), columns)?)
}

fn demote_headers(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let width = t.column_count();
    let columns = t
        .columns()
        .iter()
        .zip(t.column_names())
        .map(|(c, n)| {
            let mut v = Vec::with_capacity(c.len() + 1);
            v.push(Lazy::Value(Value::Text(n.clone())));
            v.extend_from_slice(c);
            Arc::new(v)
        })
        .collect();
    table_value(TableValue::new(default_names(width), Vec::new(), columns)?)
}

fn condition_holds(it: &Interp, f: &FunctionValue, row: Value) -> Result<bool, MError> {
    match it.call(f, &[row])?.plain() {
        Value::Logical(b) => Ok(*b),
        other => Err(convert_error(other, "Logical")),
    }
}

fn select_rows(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let f = req_func(args, 1)?;
    let mut keep = Vec::new();
    for (i, row) in t.rows().enumerate() {
        it.tick()?;
        if condition_holds(it, f, Value::Record(row))? {
            keep.push(i);
        }
    }
    table_value(t.take_rows(&keep))
}

/// Parses `{old, new}` or `{{old, new}, ...}` (also used for `{col, fn[, type]}` lists).
fn pairs(it: &Interp, v: &Value) -> Result<Vec<Vec<Value>>, MError> {
    let Value::List(l) = v.plain() else {
        return Err(convert_error(v, "List"));
    };
    let list = items(it, l)?;
    if list.first().is_some_and(|x| !matches!(x, Value::List(_))) {
        return Ok(vec![list]);
    }
    list.iter()
        .map(|x| match x {
            Value::List(inner) => items(it, inner),
            other => Err(convert_error(other, "List")),
        })
        .collect()
}

fn text_of(v: &Value) -> Result<Arc<str>, MError> {
    match v.plain() {
        Value::Text(t) => Ok(t.clone()),
        other => Err(convert_error(other, "Text")),
    }
}

fn rename_columns(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let missing = missing_arg(args, 2)?;
    let mut cols = columns_of(t);
    for pair in pairs(it, arg(args, 1))? {
        let [old, new] = pair.as_slice() else {
            return Err(MError::expression(
                "Table.RenameColumns expects {old, new} pairs.",
            ));
        };
        let (old, new) = (text_of(old)?, text_of(new)?);
        match t.column_index(&old) {
            Some(i) => {
                if let Some(c) = cols.get_mut(i) {
                    c.0 = new;
                }
            }
            None => match missing {
                MissingField::Error => return Err(missing_column(&old)),
                MissingField::Ignore => {}
                MissingField::UseNull => {
                    cols.push((new, TypeValue::any(), null_column(t.row_count())));
                }
            },
        }
    }
    table_value(build(it, cols)?)
}

/// Maps a column's cells through `f` (cell errors are kept; callback errors become cell
/// errors).
fn map_cells(
    it: &Interp,
    cells: &[Lazy],
    mut f: impl FnMut(Value) -> Result<Value, MError>,
) -> Result<Arc<Vec<Lazy>>, MError> {
    let mut out = Vec::with_capacity(cells.len());
    for c in cells {
        it.tick()?;
        out.push(match c.force(it) {
            Ok(v) => catch(f(v))?,
            Err(e) => catch(Err(e))?,
        });
    }
    Ok(Arc::new(out))
}

fn transform_columns(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let default = func_arg(args, 2)?;
    let missing = missing_arg(args, 3)?;
    let mut cols = columns_of(t);
    let mut transformed = vec![false; cols.len()];
    for op in pairs(it, arg(args, 1))? {
        let (name, f, ty) = match op.as_slice() {
            [n, f] => (n, f, None),
            [n, f, ty] => (n, f, Some(ty)),
            _ => {
                return Err(MError::expression(
                    "Table.TransformColumns expects {column, transform[, type]} lists.",
                ));
            }
        };
        let name = text_of(name)?;
        let Value::Function(f) = f.plain() else {
            return Err(convert_error(f, "Function"));
        };
        let Some(i) = t.column_index(&name) else {
            match missing {
                MissingField::Error => return Err(missing_column(&name)),
                _ => continue,
            }
        };
        let ty = match ty.map(Value::plain) {
            Some(Value::Type(ty)) => ty.clone(),
            Some(Value::Null) | None => TypeValue::any(),
            Some(other) => return Err(convert_error(other, "Type")),
        };
        let cells = map_cells(it, &t.columns()[i], |v| it.call(f, &[v]))?;
        if let Some(c) = cols.get_mut(i) {
            c.1 = ty;
            c.2 = cells;
        }
        transformed[i] = true;
    }
    if let Some(f) = default {
        for (i, c) in cols.iter_mut().enumerate() {
            if !transformed[i] {
                c.2 = map_cells(it, &t.columns()[i], |v| it.call(f, &[v]))?;
                c.1 = TypeValue::any();
            }
        }
    }
    table_value(build(it, cols)?)
}

fn transform_column_types(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let missing = match arg(args, 2).plain() {
        Value::Record(r) => {
            MissingField::from_arg(&option(it, Some(r), "MissingField")?.unwrap_or(Value::Null))?
        }
        _ => MissingField::Error,
    };
    let mut cols = columns_of(t);
    for op in pairs(it, arg(args, 1))? {
        let [name, ty] = op.as_slice() else {
            return Err(MError::expression(
                "Table.TransformColumnTypes expects {column, type} pairs.",
            ));
        };
        let name = text_of(name)?;
        let Value::Type(ty) = ty.plain() else {
            return Err(convert_error(ty, "Type"));
        };
        let Some(i) = t.column_index(&name) else {
            match missing {
                MissingField::Error => return Err(missing_column(&name)),
                _ => continue,
            }
        };
        let cells = map_cells(it, &cols[i].2.clone(), |v| convert_to_type(it, &v, ty))?;
        if let Some(c) = cols.get_mut(i) {
            c.1 = ty.clone();
            c.2 = cells;
        }
    }
    table_value(build(it, cols)?)
}

fn type_arg(args: &[Value], i: usize) -> Result<TypeValue, MError> {
    match arg(args, i) {
        Value::Null => Ok(TypeValue::any()),
        Value::Type(t) => Ok(t.clone()),
        other => Err(convert_error(other, "Type")),
    }
}

fn check_new_name(t: &TableValue, name: &str) -> Result<(), MError> {
    if t.column_index(name).is_some() {
        return Err(MError::expression(format!(
            "The field '{name}' already exists in the record."
        )));
    }
    Ok(())
}

fn add_column(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let name = req_text(args, 1)?;
    let f = req_func(args, 2)?;
    let ty = type_arg(args, 3)?;
    check_new_name(t, name)?;
    let mut cells = Vec::with_capacity(t.row_count());
    for row in t.rows() {
        it.tick()?;
        cells.push(catch(it.call(f, &[Value::Record(row)]))?);
    }
    let mut cols = columns_of(t);
    cols.push((Arc::from(name), ty, Arc::new(cells)));
    table_value(build(it, cols)?)
}

fn add_index_column(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let name = req_text(args, 1)?;
    check_new_name(t, name)?;
    let start = super::args::num_arg(args, 2)?.unwrap_or(0.0);
    let step = super::args::num_arg(args, 3)?.unwrap_or(1.0);
    let ty = match arg(args, 4) {
        Value::Null => TypeValue::facet(crate::value::Facet::Int64),
        _ => type_arg(args, 4)?,
    };
    let cells = (0..t.row_count())
        .map(|i| Lazy::Value(Value::Number(start + step * i as f64)))
        .collect();
    let mut cols = columns_of(t);
    cols.push((Arc::from(name), ty, Arc::new(cells)));
    table_value(build(it, cols)?)
}

fn duplicate_column(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let src = req_text(args, 1)?;
    let name = req_text(args, 2)?;
    check_new_name(t, name)?;
    let i = t.column_index(src).ok_or_else(|| missing_column(src))?;
    let ty = match arg(args, 3) {
        Value::Null => t.column_type(i).cloned().unwrap_or_else(TypeValue::any),
        _ => type_arg(args, 3)?,
    };
    let mut cols = columns_of(t);
    cols.push((Arc::from(name), ty, t.columns()[i].clone()));
    table_value(build(it, cols)?)
}

/// Whether a function value is the library's `Comparer.OrdinalIgnoreCase`.
pub(crate) fn is_ignore_case(v: &Value) -> bool {
    matches!(v.plain(), Value::Function(f) if matches!(&*f.0, FunctionKind::Native(n) if &*n.name == "Comparer.OrdinalIgnoreCase"))
}

/// Parses `equationCriteria`: column name(s), a comparer, or `{column, comparer}`.
fn distinct_criteria(it: &Interp, t: &TableValue, v: &Value) -> Result<(Vec<usize>, bool), MError> {
    let all = || (0..t.column_count()).collect::<Vec<_>>();
    match v.plain() {
        Value::Null => Ok((all(), false)),
        Value::Function(_) => Ok((all(), is_ignore_case(v))),
        Value::Text(n) => Ok((
            vec![t.column_index(n).ok_or_else(|| missing_column(n))?],
            false,
        )),
        Value::List(l) => {
            let list = items(it, l)?;
            let mut cols = Vec::new();
            let mut ignore_case = false;
            for x in &list {
                match x {
                    Value::Text(n) => {
                        cols.push(t.column_index(n).ok_or_else(|| missing_column(n))?)
                    }
                    Value::Function(_) => ignore_case |= is_ignore_case(x),
                    Value::List(inner) => {
                        for y in items(it, inner)? {
                            match &y {
                                Value::Text(n) => {
                                    cols.push(t.column_index(n).ok_or_else(|| missing_column(n))?)
                                }
                                Value::Function(_) => ignore_case |= is_ignore_case(&y),
                                other => return Err(convert_error(other, "Text")),
                            }
                        }
                    }
                    other => return Err(convert_error(other, "Text")),
                }
            }
            if cols.is_empty() {
                cols = all();
            }
            Ok((cols, ignore_case))
        }
        other => Err(convert_error(other, "List")),
    }
}

fn distinct(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let (cols, ignore_case) = distinct_criteria(it, t, arg(args, 1))?;
    let mut buckets: HashMap<u64, Vec<(usize, Vec<Value>)>> = HashMap::new();
    let mut keep = Vec::new();
    for row in 0..t.row_count() {
        it.tick()?;
        let mut key = Vec::with_capacity(cols.len());
        let mut h = DefaultHasher::new();
        for &c in &cols {
            let v = match t.cell(row, c) {
                Some(cell) => cell.force(it)?,
                None => Value::Null,
            };
            hash_value(&v, ignore_case, &mut h);
            key.push(v);
        }
        let bucket = buckets.entry(h.finish()).or_default();
        let mut duplicate = false;
        for (_, other) in bucket.iter() {
            let mut same = true;
            for (a, b) in key.iter().zip(other) {
                if !equals_with(it, a, b, ignore_case)? {
                    same = false;
                    break;
                }
            }
            if same {
                duplicate = true;
                break;
            }
        }
        if !duplicate {
            bucket.push((row, key));
            keep.push(row);
        }
    }
    table_value(t.take_rows(&keep))
}

fn select_columns_fn(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let cols = names(names_arg(it, arg(args, 1))?);
    table_value(select_columns(t, &cols, missing_arg(args, 2)?)?)
}

fn remove_columns(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let remove = names_arg(it, arg(args, 1))?;
    let missing = missing_arg(args, 2)?;
    if missing == MissingField::Error
        && let Some(m) = remove.iter().find(|n| t.column_index(n).is_none())
    {
        return Err(missing_column(m));
    }
    let keep: Vec<Arc<str>> = t
        .column_names()
        .iter()
        .filter(|n| !remove.iter().any(|r| r.as_str() == &***n))
        .cloned()
        .collect();
    table_value(select_columns(t, &keep, MissingField::Error)?)
}

fn reorder_columns(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let order = names(names_arg(it, arg(args, 1))?);
    let missing = missing_arg(args, 2)?;
    let mut present: Vec<Arc<str>> = Vec::new();
    for n in &order {
        if t.column_index(n).is_some() {
            present.push(n.clone());
        } else if missing == MissingField::Error {
            return Err(missing_column(n));
        }
    }
    // Listed columns take the positions of the listed columns, in the given order.
    let mut queue = present.iter();
    let names: Vec<Arc<str>> = t
        .column_names()
        .iter()
        .map(|n| {
            if present.contains(n) {
                queue.next().cloned().unwrap_or_else(|| n.clone())
            } else {
                n.clone()
            }
        })
        .collect();
    table_value(select_columns(t, &names, MissingField::Error)?)
}

fn transform_column_names(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let f = req_func(args, 1)?;
    let mut cols = columns_of(t);
    for c in &mut cols {
        c.0 = text_of(&it.call(f, &[Value::Text(c.0.clone())])?)?;
    }
    table_value(build(it, cols)?)
}

fn row_count(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::Number(req_table(args, 0)?.row_count() as f64))
}

fn column_count(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::Number(req_table(args, 0)?.column_count() as f64))
}

fn column_names(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::list(
        req_table(args, 0)?
            .column_names()
            .iter()
            .map(|n| Value::Text(n.clone())),
    ))
}

fn has_columns(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let cols = names_arg(it, arg(args, 1))?;
    Ok(Value::Logical(
        cols.iter().all(|c| t.column_index(c).is_some()),
    ))
}

fn column(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let name = req_text(args, 1)?;
    let cells = t.column_by_name(name).ok_or_else(|| missing_column(name))?;
    Ok(Value::List(ListValue::new(cells.to_vec())))
}

fn is_empty(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::Logical(req_table(args, 0)?.row_count() == 0))
}

fn from_rows(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let rows = items(it, req_list(args, 0)?)?;
    let width = rows
        .first()
        .and_then(|r| r.as_list().map(ListValue::len))
        .unwrap_or(0);
    let (names, types) = column_spec(it, arg(args, 1), width)?;
    it.check_rows(rows.len())?;
    let mut out = Vec::with_capacity(rows.len());
    for (i, r) in rows.iter().enumerate() {
        it.tick()?;
        out.push(row_cells(it, r, names.len(), i)?);
    }
    table_value(TableValue::from_rows(names, out)?.with_types(types))
}

fn from_records(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let records = items(it, req_list(args, 0)?)?;
    let missing = missing_arg(args, 2)?;
    it.check_rows(records.len())?;
    let (names, types) = match arg(args, 1) {
        Value::Null => match records.first() {
            Some(Value::Record(r)) => (r.names().to_vec(), Vec::new()),
            Some(other) => return Err(convert_error(other, "Record")),
            None => (Vec::new(), Vec::new()),
        },
        other => column_spec(it, other, 0)?,
    };
    let mut rows = Vec::with_capacity(records.len());
    for rec in &records {
        it.tick()?;
        let Value::Record(r) = rec else {
            return Err(convert_error(rec, "Record"));
        };
        let mut row = Vec::with_capacity(names.len());
        for n in &names {
            match r.get(n) {
                Some(slot) => row.push(catch(slot.force(it))?),
                None if missing != MissingField::Error => row.push(Lazy::Value(Value::Null)),
                None => {
                    return Err(MError::expression(format!(
                        "The field '{n}' of the record wasn't found."
                    )));
                }
            }
        }
        rows.push(row);
    }
    table_value(TableValue::from_rows(names, rows)?.with_types(types))
}

fn from_columns(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let lists = items(it, req_list(args, 0)?)?;
    let (names, types) = column_spec(it, arg(args, 1), lists.len())?;
    if names.len() != lists.len() {
        return Err(MError::expression(
            "The number of column names doesn't match the number of columns.",
        ));
    }
    let rows = lists
        .iter()
        .map(|l| l.as_list().map_or(0, ListValue::len))
        .max()
        .unwrap_or(0);
    it.check_rows(rows)?;
    let mut columns = Vec::with_capacity(lists.len());
    for l in &lists {
        let Value::List(l) = l else {
            return Err(convert_error(l, "List"));
        };
        let mut cells: Vec<Lazy> = l
            .items()
            .iter()
            .map(|c| catch(c.force(it)))
            .collect::<Result<_, _>>()?;
        cells.resize(rows, Lazy::Value(Value::Null));
        columns.push(Arc::new(cells));
    }
    table_value(TableValue::new(names, types, columns)?)
}

fn from_list(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let list = items(it, req_list(args, 0)?)?;
    let splitter = func_arg(args, 1)?;
    let mut rows: Vec<Vec<Value>> = Vec::with_capacity(list.len());
    for v in list {
        it.tick()?;
        let row = match splitter {
            Some(f) => match it.call(f, &[v])?.into_plain() {
                Value::List(l) => items(it, &l)?,
                other => vec![other],
            },
            None => match v {
                Value::List(l) => items(it, &l)?,
                Value::Text(t) => t.split(',').map(Value::text).collect(),
                other => vec![other],
            },
        };
        rows.push(row);
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(1);
    let (names, types) = column_spec(it, arg(args, 2), width)?;
    let default = arg(args, 3).clone();
    let extra_error = arg(args, 4).as_number() == Some(1.0);
    let mut out = Vec::with_capacity(rows.len());
    for (i, mut row) in rows.into_iter().enumerate() {
        if row.len() > names.len() && extra_error {
            return Err(MError::expression(format!(
                "Row {} has {} values but the table has {} columns.",
                i + 1,
                row.len(),
                names.len()
            )));
        }
        row.resize(names.len(), default.clone());
        out.push(row.into_iter().map(Lazy::Value).collect());
    }
    table_value(TableValue::from_rows(names, out)?.with_types(types))
}

fn to_records(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::list(req_table(args, 0)?.rows().map(Value::Record)))
}

fn to_rows(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    Ok(Value::list((0..t.row_count()).map(|r| {
        Value::List(ListValue::new(
            (0..t.column_count())
                .map(|c| t.cell(r, c).cloned().unwrap_or(Lazy::Value(Value::Null)))
                .collect(),
        ))
    })))
}

fn to_columns(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    Ok(Value::list(
        t.columns()
            .iter()
            .map(|c| Value::List(ListValue::new(c.to_vec()))),
    ))
}

fn transform_rows(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let f = req_func(args, 1)?;
    let mut out = Vec::with_capacity(t.row_count());
    for row in t.rows() {
        it.tick()?;
        out.push(catch(it.call(f, &[Value::Record(row)]))?);
    }
    Ok(Value::List(ListValue::new(out)))
}

/// Number of leading rows selected by a count or a condition.
fn leading(it: &Interp, t: &TableValue, v: &Value, default: usize) -> Result<usize, MError> {
    match v.plain() {
        Value::Null => Ok(default),
        Value::Number(_) => Ok(count_arg(std::slice::from_ref(v), 0)?.unwrap_or(default)),
        Value::Function(f) => {
            let mut n = 0;
            for row in t.rows() {
                it.tick()?;
                if !condition_holds(it, f, Value::Record(row))? {
                    break;
                }
                n += 1;
            }
            Ok(n)
        }
        other => Err(convert_error(other, "Number")),
    }
}

fn first_n(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let n = leading(it, t, arg(args, 1), 0)?.min(t.row_count());
    table_value(t.take_rows(&(0..n).collect::<Vec<_>>()))
}

fn last_n(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let rows = t.row_count();
    let n = match arg(args, 1) {
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
    table_value(t.take_rows(&(rows - n..rows).collect::<Vec<_>>()))
}

fn skip(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let n = leading(it, t, arg(args, 1), 1)?.min(t.row_count());
    table_value(t.take_rows(&(n..t.row_count()).collect::<Vec<_>>()))
}

fn range(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let offset = count_arg(args, 1)?.unwrap_or(0).min(t.row_count());
    let count = count_arg(args, 2)?.unwrap_or(usize::MAX);
    let end = offset.saturating_add(count).min(t.row_count());
    table_value(t.take_rows(&(offset..end).collect::<Vec<_>>()))
}

fn first(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    Ok(t.row(0).map_or_else(|| arg(args, 1).clone(), Value::Record))
}

fn last(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    Ok(t.row_count()
        .checked_sub(1)
        .and_then(|i| t.row(i))
        .map_or_else(|| arg(args, 1).clone(), Value::Record))
}

enum SortKey {
    Column(usize, bool),
    Key(FunctionValue, bool),
    Comparer(FunctionValue),
}

fn sort_keys(it: &Interp, t: &TableValue, v: &Value) -> Result<Vec<SortKey>, MError> {
    let one = |x: &Value| -> Result<SortKey, MError> {
        match x.plain() {
            Value::Text(n) => Ok(SortKey::Column(
                t.column_index(n).ok_or_else(|| missing_column(n))?,
                false,
            )),
            Value::Function(f) if f.arity().1 >= 2 => Ok(SortKey::Comparer(f.clone())),
            Value::Function(f) => Ok(SortKey::Key(f.clone(), false)),
            Value::List(l) => {
                let parts = items(it, l)?;
                let descending = parts.get(1).and_then(Value::as_number) == Some(1.0);
                match parts.first() {
                    Some(Value::Text(n)) => Ok(SortKey::Column(
                        t.column_index(n).ok_or_else(|| missing_column(n))?,
                        descending,
                    )),
                    Some(Value::Function(f)) => Ok(SortKey::Key(f.clone(), descending)),
                    _ => Err(MError::expression("Invalid sort criteria.")),
                }
            }
            other => Err(convert_error(other, "Text")),
        }
    };
    match v.plain() {
        Value::List(l) => {
            let parts = items(it, l)?;
            // `{"Col", Order.Descending}` is a single criterion.
            if let [Value::Text(_), Value::Number(_)] = parts.as_slice() {
                return Ok(vec![one(v)?]);
            }
            parts.iter().map(one).collect()
        }
        _ => Ok(vec![one(v)?]),
    }
}

fn sort(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let keys = sort_keys(it, t, arg(args, 1))?;
    let rows: Vec<RecordValue> = t.rows().collect();
    // Precompute key values.
    let mut key_values: Vec<Vec<Value>> = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        it.tick()?;
        let mut kv = Vec::new();
        for k in &keys {
            match k {
                SortKey::Column(c, _) => kv.push(match t.cell(i, *c) {
                    Some(cell) => cell.force(it)?,
                    None => Value::Null,
                }),
                SortKey::Key(f, _) => kv.push(it.call(f, &[Value::Record(row.clone())])?),
                SortKey::Comparer(_) => kv.push(Value::Null),
            }
        }
        key_values.push(kv);
    }
    let error: RefCell<Option<MError>> = RefCell::new(None);
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by(|&a, &b| {
        for (ki, k) in keys.iter().enumerate() {
            let o = match k {
                SortKey::Column(_, desc) | SortKey::Key(_, desc) => {
                    let o = sort_compare(&key_values[a][ki], &key_values[b][ki]);
                    if *desc { o.reverse() } else { o }
                }
                SortKey::Comparer(f) => {
                    if error.borrow().is_some() {
                        return Ordering::Equal;
                    }
                    match it.call(
                        f,
                        &[
                            Value::Record(rows[a].clone()),
                            Value::Record(rows[b].clone()),
                        ],
                    ) {
                        Ok(v) => v.as_number().map_or(Ordering::Equal, |n| {
                            n.partial_cmp(&0.0).unwrap_or(Ordering::Equal)
                        }),
                        Err(e) => {
                            *error.borrow_mut() = Some(e);
                            Ordering::Equal
                        }
                    }
                }
            };
            if o != Ordering::Equal {
                return o;
            }
        }
        Ordering::Equal
    });
    if let Some(e) = error.into_inner() {
        return Err(e);
    }
    table_value(t.take_rows(&order))
}

fn replace_value(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let old = arg(args, 1);
    let new = arg(args, 2);
    let replacer = req_func(args, 3)?;
    let cols = names_arg(it, arg(args, 4))?;
    let row_dependent = matches!(old, Value::Function(_)) || matches!(new, Value::Function(_));
    let mut out = columns_of(t);
    for name in &cols {
        let ci = t.column_index(name).ok_or_else(|| missing_column(name))?;
        let mut cells = Vec::with_capacity(t.row_count());
        for (r, cell) in t.columns()[ci].iter().enumerate() {
            it.tick()?;
            let v = match cell.force(it) {
                Ok(v) => v,
                Err(e) => {
                    cells.push(catch(Err(e))?);
                    continue;
                }
            };
            let (o, n) = if row_dependent {
                let row = Value::Record(t.row(r).unwrap_or_else(RecordValue::empty));
                let eval = |x: &Value| match x {
                    Value::Function(f) => it.call(f, std::slice::from_ref(&row)),
                    other => Ok(other.clone()),
                };
                (eval(old)?, eval(new)?)
            } else {
                (old.clone(), new.clone())
            };
            cells.push(catch(it.call(replacer, &[v, o, n]))?);
        }
        if let Some(c) = out.get_mut(ci) {
            c.2 = Arc::new(cells);
        }
    }
    table_value(build(it, out)?)
}

fn combine(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let list = items(it, req_list(args, 0)?)?;
    let mut tables = Vec::with_capacity(list.len());
    let mut rows = 0usize;
    for v in &list {
        let Value::Table(t) = v else {
            return Err(convert_error(v, "Table"));
        };
        rows = rows.saturating_add(t.row_count());
        tables.push(t.clone());
    }
    it.check_rows(rows)?;
    let combined = combine_tables(&tables)?;
    match arg(args, 1) {
        Value::Null => table_value(combined),
        cols => {
            let (names, _) = column_spec(it, cols, 0)?;
            table_value(select_columns(&combined, &names, MissingField::UseNull)?)
        }
    }
}

fn fill(it: &Interp, args: &[Value], down: bool) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let cols = names_arg(it, arg(args, 1))?;
    let mut out = columns_of(t);
    for name in &cols {
        let ci = t.column_index(name).ok_or_else(|| missing_column(name))?;
        let mut cells = t.columns()[ci].to_vec();
        let mut last: Option<Lazy> = None;
        let indices: Vec<usize> = if down {
            (0..cells.len()).collect()
        } else {
            (0..cells.len()).rev().collect()
        };
        for i in indices {
            let is_null = matches!(&cells[i], Lazy::Value(v) if v.is_null());
            if is_null {
                if let Some(l) = &last {
                    cells[i] = l.clone();
                }
            } else {
                last = Some(cells[i].clone());
            }
        }
        if let Some(c) = out.get_mut(ci) {
            c.2 = Arc::new(cells);
        }
    }
    table_value(build(it, out)?)
}

fn fill_down(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    fill(it, args, true)
}

fn fill_up(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    fill(it, args, false)
}

fn replace_error_values(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let mut out = columns_of(t);
    for pair in pairs(it, arg(args, 1))? {
        let [name, replacement] = pair.as_slice() else {
            return Err(MError::expression(
                "Table.ReplaceErrorValues expects {column, value} pairs.",
            ));
        };
        let name = text_of(name)?;
        let ci = t.column_index(&name).ok_or_else(|| missing_column(&name))?;
        let mut cells = Vec::with_capacity(t.row_count());
        for c in t.columns()[ci].iter() {
            cells.push(match c.force(it) {
                Ok(v) => Lazy::Value(v),
                Err(e) if e.is_catchable() => Lazy::Value(replacement.clone()),
                Err(e) => return Err(e),
            });
        }
        if let Some(c) = out.get_mut(ci) {
            c.2 = Arc::new(cells);
        }
    }
    table_value(build(it, out)?)
}

fn rows_with_errors(it: &Interp, args: &[Value], keep_errors: bool) -> Result<Value, MError> {
    let t = req_table(args, 0)?;
    let cols: Vec<usize> = match arg(args, 1) {
        Value::Null => (0..t.column_count()).collect(),
        v => names_arg(it, v)?
            .iter()
            .map(|n| t.column_index(n).ok_or_else(|| missing_column(n)))
            .collect::<Result<_, _>>()?,
    };
    let mut keep = Vec::new();
    for r in 0..t.row_count() {
        it.tick()?;
        let mut has_error = false;
        for &c in &cols {
            if let Some(cell) = t.cell(r, c) {
                match cell.force(it) {
                    Ok(_) => {}
                    Err(e) if e.is_catchable() => {
                        has_error = true;
                        break;
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        if has_error == keep_errors {
            keep.push(r);
        }
    }
    table_value(t.take_rows(&keep))
}

fn remove_rows_with_errors(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    rows_with_errors(it, args, false)
}

fn select_rows_with_errors(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    rows_with_errors(it, args, true)
}
