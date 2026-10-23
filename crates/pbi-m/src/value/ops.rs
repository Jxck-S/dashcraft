//! Operators: equality, ordering, arithmetic and combination, with M null semantics.

use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use chrono::{NaiveDateTime, NaiveTime, TimeDelta};

use crate::ast::{BinaryOp, UnaryOp};
use crate::merror::{LimitKind, MError};
use crate::value::format::describe;
use crate::value::{Lazy, ListValue, TableValue, Value, ValueKind};

/// Evaluates [`Lazy`] slots on behalf of an operator.
pub(crate) trait Forcer {
    fn force_slot(&self, slot: &Lazy) -> Result<Value, MError>;
    fn binary_limit(&self) -> u64;
}

/// Forces nothing: reads already-resolved slots (for values returned by the engine).
pub(crate) struct Resolved;

impl Forcer for Resolved {
    fn force_slot(&self, slot: &Lazy) -> Result<Value, MError> {
        slot.resolved()
    }

    fn binary_limit(&self) -> u64 {
        u64::MAX
    }
}

/// Maximum nesting depth for structural equality.
pub(crate) const MAX_STRUCTURAL_DEPTH: usize = 4096;

/// M `=`: `null = null` is true, values of different kinds are unequal, lists/records/tables
/// compare structurally, functions by identity.
pub(crate) fn equals(f: &dyn Forcer, a: &Value, b: &Value, depth: usize) -> Result<bool, MError> {
    if depth > MAX_STRUCTURAL_DEPTH {
        return Err(MError::limit(LimitKind::Depth, MAX_STRUCTURAL_DEPTH as u64));
    }
    Ok(match (a.plain(), b.plain()) {
        (Value::Null, Value::Null) => true,
        (Value::Logical(x), Value::Logical(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => x == y,
        (Value::Text(x), Value::Text(y)) => x == y,
        (Value::Date(x), Value::Date(y)) => x == y,
        (Value::Time(x), Value::Time(y)) => x == y,
        (Value::DateTime(x), Value::DateTime(y)) => x == y,
        (Value::DateTimeZone(x), Value::DateTimeZone(y)) => x == y,
        (Value::Duration(x), Value::Duration(y)) => x == y,
        (Value::Binary(x), Value::Binary(y)) => {
            x.ptr_eq(y) || x.bytes(f.binary_limit())? == y.bytes(f.binary_limit())?
        }
        (Value::List(x), Value::List(y)) => {
            if x.ptr_eq(y) {
                return Ok(true);
            }
            if x.len() != y.len() {
                return Ok(false);
            }
            for (i, j) in x.items().iter().zip(y.items()) {
                if !equals(f, &f.force_slot(i)?, &f.force_slot(j)?, depth + 1)? {
                    return Ok(false);
                }
            }
            true
        }
        (Value::Record(x), Value::Record(y)) => {
            if x.len() != y.len() {
                return Ok(false);
            }
            for (name, slot) in x.iter() {
                let Some(other) = y.get(name) else {
                    return Ok(false);
                };
                if !equals(f, &f.force_slot(slot)?, &f.force_slot(other)?, depth + 1)? {
                    return Ok(false);
                }
            }
            true
        }
        (Value::Table(x), Value::Table(y)) => tables_equal(f, x, y, depth)?,
        (Value::Function(x), Value::Function(y)) => x.ptr_eq(y),
        (Value::Type(x), Value::Type(y)) => x.same_as(y),
        _ => false,
    })
}

fn tables_equal(
    f: &dyn Forcer,
    x: &TableValue,
    y: &TableValue,
    depth: usize,
) -> Result<bool, MError> {
    if x.ptr_eq(y) {
        return Ok(true);
    }
    if x.column_count() != y.column_count() || x.row_count() != y.row_count() {
        return Ok(false);
    }
    for (ci, name) in x.column_names().iter().enumerate() {
        let (Some(a), Some(b)) = (x.column(ci), y.column_index(name).and_then(|j| y.column(j)))
        else {
            return Ok(false);
        };
        for (p, q) in a.iter().zip(b) {
            if !equals(f, &f.force_slot(p)?, &f.force_slot(q)?, depth + 1)? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Ordering for `<`, `<=`, `>`, `>=`: `Ok(None)` when unordered (NaN), an error for kinds that
/// cannot be compared.
pub(crate) fn compare(a: &Value, b: &Value, op: &str) -> Result<Option<Ordering>, MError> {
    Ok(match (a.plain(), b.plain()) {
        (Value::Number(x), Value::Number(y)) => x.partial_cmp(y),
        (Value::Text(x), Value::Text(y)) => Some(x.cmp(y)),
        (Value::Logical(x), Value::Logical(y)) => Some(x.cmp(y)),
        (Value::Date(x), Value::Date(y)) => Some(x.cmp(y)),
        (Value::Time(x), Value::Time(y)) => Some(x.cmp(y)),
        (Value::DateTime(x), Value::DateTime(y)) => Some(x.cmp(y)),
        (Value::DateTimeZone(x), Value::DateTimeZone(y)) => Some(x.cmp(y)),
        (Value::Duration(x), Value::Duration(y)) => Some(x.cmp(y)),
        (x, y) => return Err(operator_error(op, x, y)),
    })
}

/// A total order for sorting (`Table.Sort`, `List.Sort`, `List.Max`): `null` first, then by
/// kind, then by value.
pub(crate) fn sort_compare(a: &Value, b: &Value) -> Ordering {
    let (ka, kb) = (a.kind(), b.kind());
    if ka != kb {
        if ka == ValueKind::Null {
            return Ordering::Less;
        }
        if kb == ValueKind::Null {
            return Ordering::Greater;
        }
        return (ka as u8).cmp(&(kb as u8));
    }
    match (a.plain(), b.plain()) {
        (Value::Number(x), Value::Number(y)) => x.partial_cmp(y).unwrap_or_else(|| {
            // NaN sorts before all numbers.
            x.is_nan().cmp(&y.is_nan()).reverse()
        }),
        _ => compare(a, b, "<").ok().flatten().unwrap_or(Ordering::Equal),
    }
}

pub(crate) fn relational(op: BinaryOp, a: &Value, b: &Value) -> Result<Value, MError> {
    if a.is_null() || b.is_null() {
        return Ok(Value::Null);
    }
    let ord = compare(a, b, op.symbol())?;
    let result = match ord {
        None => false,
        Some(o) => match op {
            BinaryOp::LessThan => o == Ordering::Less,
            BinaryOp::LessThanOrEqual => o != Ordering::Greater,
            BinaryOp::GreaterThan => o == Ordering::Greater,
            BinaryOp::GreaterThanOrEqual => o != Ordering::Less,
            _ => {
                return Err(MError::Internal(format!(
                    "not a relational operator: {op:?}"
                )));
            }
        },
    };
    Ok(Value::Logical(result))
}

pub(crate) fn operator_error(op: &str, a: &Value, b: &Value) -> MError {
    MError::expression(format!(
        "We cannot apply operator {op} to types {} and {}.",
        a.kind().name(),
        b.kind().name()
    ))
}

fn out_of_range() -> MError {
    MError::expression("The result of the operation is out of range.")
}

fn date_add(d: chrono::NaiveDate, delta: TimeDelta) -> Result<Value, MError> {
    let dt = NaiveDateTime::new(d, NaiveTime::MIN);
    dt.checked_add_signed(delta)
        .map(|r| Value::Date(r.date()))
        .ok_or_else(out_of_range)
}

/// `+ - * / &` on plain values (`and`/`or`/`??`/`meta` are handled by the evaluator).
pub(crate) fn arithmetic(op: BinaryOp, a: &Value, b: &Value) -> Result<Value, MError> {
    let (a, b) = (a.plain(), b.plain());
    if matches!(a, Value::Null) || matches!(b, Value::Null) {
        return Ok(Value::Null);
    }
    use Value as V;
    let r = match (op, a, b) {
        (BinaryOp::Add, V::Number(x), V::Number(y)) => V::Number(x + y),
        (BinaryOp::Subtract, V::Number(x), V::Number(y)) => V::Number(x - y),
        (BinaryOp::Multiply, V::Number(x), V::Number(y)) => V::Number(x * y),
        (BinaryOp::Divide, V::Number(x), V::Number(y)) => V::Number(x / y),

        (BinaryOp::Add, V::Date(d), V::Duration(t))
        | (BinaryOp::Add, V::Duration(t), V::Date(d)) => date_add(*d, *t)?,
        (BinaryOp::Subtract, V::Date(d), V::Duration(t)) => date_add(*d, -*t)?,
        (BinaryOp::Subtract, V::Date(x), V::Date(y)) => V::Duration(x.signed_duration_since(*y)),

        (BinaryOp::Add, V::DateTime(d), V::Duration(t))
        | (BinaryOp::Add, V::Duration(t), V::DateTime(d)) => {
            V::DateTime(d.checked_add_signed(*t).ok_or_else(out_of_range)?)
        }
        (BinaryOp::Subtract, V::DateTime(d), V::Duration(t)) => {
            V::DateTime(d.checked_sub_signed(*t).ok_or_else(out_of_range)?)
        }
        (BinaryOp::Subtract, V::DateTime(x), V::DateTime(y)) => {
            V::Duration(x.signed_duration_since(*y))
        }

        (BinaryOp::Add, V::DateTimeZone(d), V::Duration(t))
        | (BinaryOp::Add, V::Duration(t), V::DateTimeZone(d)) => {
            V::DateTimeZone(d.checked_add_signed(*t).ok_or_else(out_of_range)?)
        }
        (BinaryOp::Subtract, V::DateTimeZone(d), V::Duration(t)) => {
            V::DateTimeZone(d.checked_sub_signed(*t).ok_or_else(out_of_range)?)
        }
        (BinaryOp::Subtract, V::DateTimeZone(x), V::DateTimeZone(y)) => {
            V::Duration(x.signed_duration_since(*y))
        }

        (BinaryOp::Add, V::Time(d), V::Duration(t))
        | (BinaryOp::Add, V::Duration(t), V::Time(d)) => V::Time(d.overflowing_add_signed(*t).0),
        (BinaryOp::Subtract, V::Time(d), V::Duration(t)) => V::Time(d.overflowing_sub_signed(*t).0),
        (BinaryOp::Subtract, V::Time(x), V::Time(y)) => V::Duration(x.signed_duration_since(*y)),

        (BinaryOp::Add, V::Duration(x), V::Duration(y)) => {
            V::Duration(x.checked_add(y).ok_or_else(out_of_range)?)
        }
        (BinaryOp::Subtract, V::Duration(x), V::Duration(y)) => {
            V::Duration(x.checked_sub(y).ok_or_else(out_of_range)?)
        }
        (BinaryOp::Multiply, V::Duration(d), V::Number(n))
        | (BinaryOp::Multiply, V::Number(n), V::Duration(d)) => {
            V::Duration(scale_duration(*d, *n)?)
        }
        (BinaryOp::Divide, V::Duration(d), V::Number(n)) => {
            V::Duration(scale_duration(*d, 1.0 / *n)?)
        }

        (BinaryOp::Concatenate, V::Text(x), V::Text(y)) => {
            let mut s = String::with_capacity(x.len() + y.len());
            s.push_str(x);
            s.push_str(y);
            V::Text(s.into())
        }
        (BinaryOp::Concatenate, V::List(x), V::List(y)) => {
            let mut items = Vec::with_capacity(x.len() + y.len());
            items.extend_from_slice(x.items());
            items.extend_from_slice(y.items());
            V::List(ListValue::new(items))
        }
        (BinaryOp::Concatenate, V::Record(x), V::Record(y)) => V::Record(x.merge(y)),
        (BinaryOp::Concatenate, V::Date(d), V::Time(t)) => V::DateTime(NaiveDateTime::new(*d, *t)),
        (BinaryOp::Concatenate, V::Table(x), V::Table(y)) => {
            V::Table(combine_tables(&[x.clone(), y.clone()])?)
        }
        (op, x, y) => return Err(operator_error(op.symbol(), x, y)),
    };
    Ok(r)
}

fn scale_duration(d: TimeDelta, n: f64) -> Result<TimeDelta, MError> {
    let micros = (d.num_microseconds().ok_or_else(out_of_range)? as f64) * n;
    if !micros.is_finite() || micros.abs() > 9.0e18 {
        return Err(out_of_range());
    }
    Ok(TimeDelta::microseconds(micros.round() as i64))
}

pub(crate) fn unary(op: UnaryOp, v: &Value) -> Result<Value, MError> {
    match (op, v.plain()) {
        (_, Value::Null) => Ok(Value::Null),
        (UnaryOp::Plus, Value::Number(n)) => Ok(Value::Number(*n)),
        (UnaryOp::Plus, Value::Duration(d)) => Ok(Value::Duration(*d)),
        (UnaryOp::Minus, Value::Number(n)) => Ok(Value::Number(-n)),
        (UnaryOp::Minus, Value::Duration(d)) => Ok(Value::Duration(-*d)),
        (UnaryOp::Not, Value::Logical(b)) => Ok(Value::Logical(!b)),
        (UnaryOp::Not, other) => Err(MError::expression(format!(
            "We cannot convert the value {} to type Logical.",
            describe(other)
        ))),
        (op, other) => Err(MError::expression(format!(
            "We cannot apply operator {} to type {}.",
            op.symbol(),
            other.kind().name()
        ))),
    }
}

/// Appends tables, unioning their columns by name (missing cells are `null`).
pub(crate) fn combine_tables(tables: &[TableValue]) -> Result<TableValue, MError> {
    let mut names: Vec<Arc<str>> = Vec::new();
    let mut types = Vec::new();
    for t in tables {
        for (i, n) in t.column_names().iter().enumerate() {
            if !names.contains(n) {
                names.push(n.clone());
                types.push(
                    t.column_type(i)
                        .cloned()
                        .unwrap_or_else(super::TypeValue::any),
                );
            }
        }
    }
    let mut columns: Vec<Vec<Lazy>> = names.iter().map(|_| Vec::new()).collect();
    for t in tables {
        for (ci, name) in names.iter().enumerate() {
            let Some(col) = columns.get_mut(ci) else {
                continue;
            };
            match t.column_by_name(name) {
                Some(cells) => col.extend_from_slice(cells),
                None => col.extend((0..t.row_count()).map(|_| Lazy::Value(Value::Null))),
            }
        }
    }
    TableValue::new(names, types, columns.into_iter().map(Arc::new).collect())
}

/// Hashes a value consistently with [`equals`] for primitive kinds; compound values hash by
/// kind only (equal values still hash equally).
pub(crate) fn hash_value<H: Hasher>(v: &Value, ignore_case: bool, h: &mut H) {
    let v = v.plain();
    (v.kind() as u8).hash(h);
    match v {
        Value::Logical(b) => b.hash(h),
        Value::Number(n) => {
            let n = if *n == 0.0 { 0.0 } else { *n };
            n.to_bits().hash(h);
        }
        Value::Text(t) => {
            if ignore_case {
                for c in t.chars() {
                    for l in c.to_lowercase() {
                        l.hash(h);
                    }
                }
            } else {
                t.hash(h);
            }
        }
        Value::Date(d) => d.hash(h),
        Value::Time(t) => t.hash(h),
        Value::DateTime(d) => d.hash(h),
        Value::DateTimeZone(d) => d.naive_utc().hash(h),
        Value::Duration(d) => d.hash(h),
        _ => {}
    }
}

/// Equality with an optional ordinal-ignore-case comparison for text.
pub(crate) fn equals_with(
    f: &dyn Forcer,
    a: &Value,
    b: &Value,
    ignore_case: bool,
) -> Result<bool, MError> {
    if ignore_case && let (Some(x), Some(y)) = (a.as_text(), b.as_text()) {
        return Ok(x.to_lowercase() == y.to_lowercase());
    }
    equals(f, a, b, 0)
}
