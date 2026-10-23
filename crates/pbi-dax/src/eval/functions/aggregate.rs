//! Aggregations over columns (`SUM`, `MAX`, …, evaluated by the storage
//! engine over the visible rows) and iterators (`SUMX`, `MAXX`, …,
//! evaluated row by row in a row context).

use std::cmp::Ordering;
use std::sync::Arc;

use pbi_engine::value::DECIMAL_FACTOR;
use pbi_engine::{Agg, Value};

use super::{Builtin, arg, arity, opt_arg, scalar_fn};
use crate::eval::context::Env;
use crate::eval::error::{DaxError, Result};
use crate::eval::fast::Columnar;
use crate::eval::interp::{Interp, Val};
use crate::eval::ops;
use crate::eval::table::RowPos;
use crate::syntax::{Expr, ExprKind};

pub(super) const BUILTINS: &[Builtin] = &[
    scalar_fn("SUM", sum),
    scalar_fn("MIN", min),
    scalar_fn("MAX", max),
    scalar_fn("AVERAGE", average),
    scalar_fn("COUNT", count),
    scalar_fn("COUNTA", counta),
    scalar_fn("COUNTBLANK", countblank),
    scalar_fn("COUNTROWS", countrows),
    scalar_fn("DISTINCTCOUNT", distinctcount),
    scalar_fn("DISTINCTCOUNTNOBLANK", distinctcountnoblank),
    scalar_fn("SUMX", sumx),
    scalar_fn("COUNTX", countx),
    scalar_fn("COUNTAX", countax),
    scalar_fn("MINX", minx),
    scalar_fn("MAXX", maxx),
    scalar_fn("AVERAGEX", averagex),
    scalar_fn("CONCATENATEX", concatenatex),
];

type R = Result<Val>;

fn ok(v: Value) -> R {
    Ok(Val::Scalar(v))
}

pub(crate) fn count_value(n: usize) -> Value {
    if n == 0 {
        Value::Blank
    } else {
        Value::Integer(i64::try_from(n).unwrap_or(i64::MAX))
    }
}

// ---- column aggregations -------------------------------------------------------

fn column_agg(i: &Interp<'_>, env: &Env, args: &[Expr], f: &str, agg: Agg) -> R {
    arity(f, args, 1, 1)?;
    let (t, c) = i.column_arg(env, arg(f, args, 0)?, f)?;
    let vis = i.snapshot.visible(&env.filter, &t)?;
    ok(i.snapshot
        .aggregate_with_blank_row(&t, &c, &vis.rows, vis.blank_row, agg)?)
}

fn sum(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    column_agg(i, env, args, "SUM", Agg::Sum)
}

fn average(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    column_agg(i, env, args, "AVERAGE", Agg::Average)
}

fn count(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    column_agg(i, env, args, "COUNT", Agg::Count)
}

fn counta(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    column_agg(i, env, args, "COUNTA", Agg::CountNonBlank)
}

fn countblank(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    column_agg(i, env, args, "COUNTBLANK", Agg::CountBlank)
}

fn distinctcount(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    column_agg(i, env, args, "DISTINCTCOUNT", Agg::DistinctCount)
}

fn distinctcountnoblank(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    column_agg(
        i,
        env,
        args,
        "DISTINCTCOUNTNOBLANK",
        Agg::DistinctCountNoBlank,
    )
}

/// BLANK as the "zero" of `other`'s type (two-argument `MIN`/`MAX`).
fn zero_like(other: &Value) -> Value {
    match other {
        Value::String(_) => Value::string(""),
        Value::Boolean(_) => Value::Boolean(false),
        Value::Double(_) => Value::Double(0.0),
        Value::Decimal(_) => Value::Decimal(0),
        Value::DateTime(_) => ops::date_from_serial(0.0),
        _ => Value::Integer(0),
    }
}

fn two_arg(i: &Interp<'_>, env: &Env, args: &[Expr], f: &str, want: Ordering) -> R {
    let a = i.scalar(env, arg(f, args, 0)?)?;
    let b = i.scalar(env, arg(f, args, 1)?)?;
    let (a, b) = match (a.is_blank(), b.is_blank()) {
        (true, true) => return ok(Value::Blank),
        (true, false) => (zero_like(&b), b),
        (false, true) => {
            let z = zero_like(&a);
            (a, z)
        }
        (false, false) => (a, b),
    };
    ok(if b.dax_cmp(&a)? == want { b } else { a })
}

fn min(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("MIN", args, 1, 2)?;
    if args.len() == 2 {
        return two_arg(i, env, args, "MIN", Ordering::Less);
    }
    column_agg(i, env, args, "MIN", Agg::Min)
}

fn max(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("MAX", args, 1, 2)?;
    if args.len() == 2 {
        return two_arg(i, env, args, "MAX", Ordering::Greater);
    }
    column_agg(i, env, args, "MAX", Agg::Max)
}

fn countrows(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("COUNTROWS", args, 0, 1)?;
    let Some(a) = opt_arg(args, 0) else {
        let home = env.home.as_deref().ok_or_else(|| {
            DaxError::argument("COUNTROWS", "expects a table when used outside a table")
        })?;
        let vis = i.snapshot.visible(&env.filter, home)?;
        return ok(count_value(vis.rows.count()));
    };
    if let ExprKind::TableRef { name } = &a.unparen().kind
        && !i
            .query_tables
            .contains_key(&pbi_model::names::name_key(name))
        && let Ok(data) = i.snapshot.table(name)
    {
        let vis = i.snapshot.visible(&env.filter, data.name())?;
        return ok(count_value(vis.rows.count()));
    }
    ok(count_value(i.table(env, a)?.row_count()))
}

// ---- iterators -----------------------------------------------------------------

/// Evaluates `args[1]` for every row of the table `args[0]`.
fn iterate_values(i: &Interp<'_>, env: &Env, args: &[Expr], f: &str) -> Result<Vec<Value>> {
    arity(f, args, 2, 2)?;
    let table = i.table(env, arg(f, args, 0)?)?;
    let expr = arg(f, args, 1)?;
    let mut out = Vec::with_capacity(table.row_count());
    if let Some(columnar) = Columnar::compile(i, &table, expr) {
        columnar.for_each(i, |v| {
            out.push(v.into_owned());
            Ok(())
        })?;
        return Ok(out);
    }
    i.iterate(env, &table, |row_env, _| {
        out.push(i.scalar(row_env, expr)?);
        Ok(true)
    })?;
    Ok(out)
}

/// `SUMX`/`AVERAGEX`: the accumulated values of `args[1]` over the rows of
/// `args[0]`. Like the general path, an evaluation error anywhere wins over
/// a value of the wrong type.
fn accumulate(i: &Interp<'_>, env: &Env, args: &[Expr], f: &str) -> Result<NumAcc> {
    let mut acc = NumAcc::default();
    arity(f, args, 2, 2)?;
    let table = i.table(env, arg(f, args, 0)?)?;
    let expr = arg(f, args, 1)?;
    if let Some(columnar) = Columnar::compile(i, &table, expr) {
        let mut wrong_type = None;
        columnar.for_each(i, |v| {
            if wrong_type.is_none()
                && let Err(e) = acc.add(f, &v)
            {
                wrong_type = Some(e);
            }
            Ok(())
        })?;
        return match wrong_type {
            Some(e) => Err(e),
            None => Ok(acc),
        };
    }
    for v in iterate_values(i, env, args, f)? {
        acc.add(f, &v)?;
    }
    Ok(acc)
}

/// Numeric accumulator with DAX result typing.
#[derive(Default)]
struct NumAcc {
    int: i128,
    dec: i128,
    real: f64,
    has_dec: bool,
    has_real: bool,
    count: u64,
}

impl NumAcc {
    fn add(&mut self, f: &str, v: &Value) -> Result<()> {
        match v {
            Value::Blank => return Ok(()),
            Value::Integer(x) => self.int = self.int.saturating_add(i128::from(*x)),
            Value::Decimal(d) => {
                self.has_dec = true;
                self.dec = self.dec.saturating_add(*d);
            }
            Value::Double(d) => {
                self.has_real = true;
                self.real += d;
            }
            Value::DateTime(_) => {
                self.has_real = true;
                self.real += ops::to_f64(v)?.unwrap_or(0.0);
            }
            Value::Boolean(_) | Value::String(_) => return Err(ops::wrong_type(f, v)),
        }
        self.count += 1;
        Ok(())
    }

    fn total_f64(&self) -> f64 {
        self.real + self.int as f64 + self.dec as f64 / DECIMAL_FACTOR as f64
    }

    fn sum(&self) -> Value {
        if self.count == 0 {
            Value::Blank
        } else if self.has_real {
            Value::Double(self.total_f64())
        } else if self.has_dec {
            match self
                .int
                .checked_mul(DECIMAL_FACTOR)
                .and_then(|x| x.checked_add(self.dec))
            {
                Some(d) => Value::Decimal(d),
                None => Value::Double(self.total_f64()),
            }
        } else {
            i64::try_from(self.int).map_or(Value::Double(self.int as f64), Value::Integer)
        }
    }

    fn average(&self) -> Value {
        if self.count == 0 {
            Value::Blank
        } else {
            Value::Double(self.total_f64() / self.count as f64)
        }
    }
}

fn sumx(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    ok(accumulate(i, env, args, "SUMX")?.sum())
}

fn averagex(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    ok(accumulate(i, env, args, "AVERAGEX")?.average())
}

fn countx(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    let values = iterate_values(i, env, args, "COUNTX")?;
    ok(count_value(values.iter().filter(|v| !v.is_blank()).count()))
}

fn countax(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    let values = iterate_values(i, env, args, "COUNTAX")?;
    ok(count_value(values.iter().filter(|v| !v.is_blank()).count()))
}

fn extreme(values: Vec<Value>, want: Ordering) -> Result<Value> {
    let mut best: Option<Value> = None;
    for v in values.into_iter().filter(|v| !v.is_blank()) {
        best = Some(match best {
            Some(b) if v.dax_cmp(&b)? != want => b,
            _ => v,
        });
    }
    Ok(best.unwrap_or_default())
}

fn minx(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    ok(extreme(
        iterate_values(i, env, args, "MINX")?,
        Ordering::Less,
    )?)
}

fn maxx(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    ok(extreme(
        iterate_values(i, env, args, "MAXX")?,
        Ordering::Greater,
    )?)
}

/// `(orderBy expression, descending)` pairs from `args`, where an order
/// flag (`ASC`/`DESC`, `0`/`1`, `TRUE`/`FALSE`) may follow each expression.
pub(crate) fn order_specs<'e>(
    i: &Interp<'_>,
    env: &Env,
    args: &'e [Expr],
    default_desc: bool,
) -> Result<Vec<(&'e Expr, bool)>> {
    let mut out = Vec::new();
    let mut k = 0;
    while let Some(e) = args.get(k) {
        k += 1;
        if matches!(e.kind, ExprKind::Omitted) {
            continue;
        }
        let mut desc = default_desc;
        if let Some(flag) = args.get(k) {
            let parsed = match &flag.unparen().kind {
                ExprKind::Keyword(kw) if kw == "ASC" => Some(false),
                ExprKind::Keyword(kw) if kw == "DESC" => Some(true),
                ExprKind::Number(_) | ExprKind::Boolean(_) => {
                    Some(!ops::is_true(&i.scalar(env, flag)?)?)
                }
                ExprKind::Omitted => Some(default_desc),
                _ => None,
            };
            if let Some(d) = parsed {
                desc = d;
                k += 1;
            }
        }
        out.push((e, desc));
    }
    Ok(out)
}

/// Compares sort keys (`total_cmp`, per-key direction).
pub(crate) fn cmp_keys(a: &[Value], b: &[Value], desc: &[bool]) -> Ordering {
    for ((x, y), d) in a.iter().zip(b).zip(desc) {
        let o = x.total_cmp(y);
        let o = if *d { o.reverse() } else { o };
        if o != Ordering::Equal {
            return o;
        }
    }
    Ordering::Equal
}

fn concatenatex(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("CONCATENATEX", args, 2, usize::MAX)?;
    let table = i.table(env, arg("CONCATENATEX", args, 0)?)?;
    let expr = arg("CONCATENATEX", args, 1)?;
    let delimiter = match opt_arg(args, 2) {
        Some(d) => ops::to_text(&i.scalar(env, d)?),
        None => String::new(),
    };
    let specs = order_specs(i, env, args.get(3..).unwrap_or_default(), false)?;
    let desc: Vec<bool> = specs.iter().map(|(_, d)| *d).collect();
    let mut rows: Vec<(Vec<Value>, String)> = Vec::new();
    i.iterate(env, &table, |row_env, _| {
        let text = ops::to_text(&i.scalar(row_env, expr)?);
        let keys = specs
            .iter()
            .map(|(e, _)| i.scalar(row_env, e))
            .collect::<Result<Vec<_>>>()?;
        rows.push((keys, text));
        Ok(true)
    })?;
    if rows.is_empty() {
        return ok(Value::Blank);
    }
    rows.sort_by(|a, b| cmp_keys(&a.0, &b.0, &desc));
    let parts: Vec<String> = rows.into_iter().map(|(_, t)| t).collect();
    ok(Value::String(Arc::from(parts.join(&delimiter))))
}

/// Sort keys of every row of `table` for `specs`.
pub(crate) fn sort_positions(
    i: &Interp<'_>,
    env: &Env,
    table: &Arc<crate::eval::table::VirtualTable>,
    specs: &[(&Expr, bool)],
) -> Result<Vec<(Vec<Value>, RowPos)>> {
    let desc: Vec<bool> = specs.iter().map(|(_, d)| *d).collect();
    let mut rows: Vec<(Vec<Value>, RowPos)> = Vec::with_capacity(table.row_count());
    i.iterate(env, table, |row_env, pos| {
        let keys = specs
            .iter()
            .map(|(e, _)| i.scalar(row_env, e))
            .collect::<Result<Vec<_>>>()?;
        rows.push((keys, pos));
        Ok(true)
    })?;
    rows.sort_by(|a, b| cmp_keys(&a.0, &b.0, &desc));
    Ok(rows)
}
