//! Argument helpers for library functions.

use crate::eval::interp::{Interp, convert_error};
use crate::merror::MError;
use crate::value::{FunctionValue, Lazy, ListValue, RecordValue, TableValue, Value};

static NULL: Value = Value::Null;

pub(crate) fn arg(args: &[Value], i: usize) -> &Value {
    args.get(i).unwrap_or(&NULL)
}

fn expected(v: &Value, target: &str) -> MError {
    convert_error(v, target)
}

/// A text argument; `None` for `null`.
pub(crate) fn text_arg<'a>(
    args: &'a [Value],
    i: usize,
    _function: &str,
) -> Result<Option<&'a str>, MError> {
    match arg(args, i) {
        Value::Null => Ok(None),
        Value::Text(t) => Ok(Some(t)),
        other => Err(expected(other, "Text")),
    }
}

/// A required (non-null) text argument.
pub(crate) fn req_text(args: &[Value], i: usize) -> Result<&str, MError> {
    match arg(args, i) {
        Value::Text(t) => Ok(t),
        other => Err(expected(other, "Text")),
    }
}

pub(crate) fn num_arg(args: &[Value], i: usize) -> Result<Option<f64>, MError> {
    match arg(args, i) {
        Value::Null => Ok(None),
        Value::Number(n) => Ok(Some(*n)),
        other => Err(expected(other, "Number")),
    }
}

pub(crate) fn req_num(args: &[Value], i: usize) -> Result<f64, MError> {
    num_arg(args, i)?.ok_or_else(|| expected(&Value::Null, "Number"))
}

/// An integral number argument.
pub(crate) fn int_arg(args: &[Value], i: usize) -> Result<Option<i64>, MError> {
    match num_arg(args, i)? {
        None => Ok(None),
        Some(n) if n.fract() == 0.0 && n.abs() <= 9.007_199_254_740_992e15 => Ok(Some(n as i64)),
        Some(n) => Err(MError::expression(format!(
            "The value {} is not an integer.",
            crate::value::format::number_to_text(n)
        ))),
    }
}

/// A non-negative integral number argument (counts, offsets).
pub(crate) fn count_arg(args: &[Value], i: usize) -> Result<Option<usize>, MError> {
    match int_arg(args, i)? {
        None => Ok(None),
        Some(n) if n >= 0 => Ok(Some(usize::try_from(n).unwrap_or(usize::MAX))),
        Some(_) => Err(MError::expression(format!(
            "The argument {} must not be negative.",
            i + 1
        ))),
    }
}

pub(crate) fn bool_arg(args: &[Value], i: usize) -> Result<Option<bool>, MError> {
    match arg(args, i) {
        Value::Null => Ok(None),
        Value::Logical(b) => Ok(Some(*b)),
        other => Err(expected(other, "Logical")),
    }
}

pub(crate) fn list_arg(args: &[Value], i: usize) -> Result<Option<&ListValue>, MError> {
    match arg(args, i) {
        Value::Null => Ok(None),
        Value::List(l) => Ok(Some(l)),
        other => Err(expected(other, "List")),
    }
}

pub(crate) fn req_list(args: &[Value], i: usize) -> Result<&ListValue, MError> {
    match arg(args, i) {
        Value::List(l) => Ok(l),
        other => Err(expected(other, "List")),
    }
}

pub(crate) fn record_arg(args: &[Value], i: usize) -> Result<Option<&RecordValue>, MError> {
    match arg(args, i) {
        Value::Null => Ok(None),
        Value::Record(r) => Ok(Some(r)),
        other => Err(expected(other, "Record")),
    }
}

pub(crate) fn req_record(args: &[Value], i: usize) -> Result<&RecordValue, MError> {
    match arg(args, i) {
        Value::Record(r) => Ok(r),
        other => Err(expected(other, "Record")),
    }
}

pub(crate) fn req_table(args: &[Value], i: usize) -> Result<&TableValue, MError> {
    match arg(args, i) {
        Value::Table(t) => Ok(t),
        other => Err(expected(other, "Table")),
    }
}

pub(crate) fn func_arg(args: &[Value], i: usize) -> Result<Option<&FunctionValue>, MError> {
    match arg(args, i) {
        Value::Null => Ok(None),
        Value::Function(f) => Ok(Some(f)),
        other => Err(expected(other, "Function")),
    }
}

pub(crate) fn req_func(args: &[Value], i: usize) -> Result<&FunctionValue, MError> {
    match arg(args, i) {
        Value::Function(f) => Ok(f),
        other => Err(expected(other, "Function")),
    }
}

/// Forces every item of a list (errors propagate).
pub(crate) fn items(it: &Interp, l: &ListValue) -> Result<Vec<Value>, MError> {
    let mut out = Vec::with_capacity(l.len());
    for item in l.items() {
        it.tick()?;
        out.push(item.force(it)?.into_plain());
    }
    Ok(out)
}

/// A text or a list of texts (column names, field names).
pub(crate) fn names_arg(it: &Interp, v: &Value) -> Result<Vec<String>, MError> {
    match v.plain() {
        Value::Text(t) => Ok(vec![t.to_string()]),
        Value::List(l) => items(it, l)?
            .into_iter()
            .map(|v| match v {
                Value::Text(t) => Ok(t.to_string()),
                other => Err(expected(&other, "Text")),
            })
            .collect(),
        other => Err(expected(other, "List")),
    }
}

/// Wraps a callback result as a cell: catchable errors become cell errors, others propagate.
pub(crate) fn catch(r: Result<Value, MError>) -> Result<Lazy, MError> {
    match r {
        Ok(v) => Ok(Lazy::Value(v)),
        Err(e) if e.is_catchable() => Ok(Lazy::Error(std::sync::Arc::new(e))),
        Err(e) => Err(e),
    }
}

/// Reads an option from an options record.
pub(crate) fn option(
    it: &Interp,
    options: Option<&RecordValue>,
    name: &str,
) -> Result<Option<Value>, MError> {
    match options.and_then(|r| r.get(name)) {
        Some(slot) => {
            let v = slot.force(it)?.into_plain();
            Ok(if v.is_null() { None } else { Some(v) })
        }
        None => Ok(None),
    }
}
