//! Ranking and statistics: `RANKX`, `RANK.EQ`, `MEDIAN`/`MEDIANX`,
//! `PERCENTILE(X).INC/EXC`, `STDEV(X).S/P`, `VAR(X).S/P`,
//! `PRODUCT`/`PRODUCTX`, `GEOMEAN`/`GEOMEANX`, and the `A` aggregations
//! `MAXA`/`MINA`/`AVERAGEA`.
//!
//! Column forms aggregate the column over the visible rows (duplicates
//! included); `X` forms evaluate the expression for every row of the table.
//! BLANKs are ignored; text and booleans are errors. An empty input (or
//! fewer than two values for the sample statistics) gives BLANK. The `A`
//! forms instead count TRUE as 1 and FALSE as 0 (`AVERAGEA` also counts
//! text: numeric text as its number, other text as 0; `MAXA`/`MINA` skip
//! text) and give 0 when there are rows but no usable value.

use std::cmp::Ordering;

use pbi_engine::Value;

use super::{Builtin, arg, arity, opt_arg, scalar_fn};
use crate::eval::context::Env;
use crate::eval::error::{DaxError, Result};
use crate::eval::interp::{Interp, Val};
use crate::eval::ops;
use crate::syntax::{Expr, ExprKind};

pub(super) const BUILTINS: &[Builtin] = &[
    scalar_fn("RANKX", rankx),
    scalar_fn("RANK.EQ", rank_eq),
    scalar_fn("MEDIAN", median),
    scalar_fn("MEDIANX", medianx),
    scalar_fn("PERCENTILE.INC", percentile_inc),
    scalar_fn("PERCENTILE.EXC", percentile_exc),
    scalar_fn("PERCENTILEX.INC", percentilex_inc),
    scalar_fn("PERCENTILEX.EXC", percentilex_exc),
    scalar_fn("STDEV.S", stdev_s),
    scalar_fn("STDEV.P", stdev_p),
    scalar_fn("STDEVX.S", stdevx_s),
    scalar_fn("STDEVX.P", stdevx_p),
    scalar_fn("VAR.S", var_s),
    scalar_fn("VAR.P", var_p),
    scalar_fn("VARX.S", varx_s),
    scalar_fn("VARX.P", varx_p),
    scalar_fn("PRODUCT", product),
    scalar_fn("PRODUCTX", productx),
    scalar_fn("GEOMEAN", geomean),
    scalar_fn("GEOMEANX", geomeanx),
    scalar_fn("MAXA", maxa),
    scalar_fn("MINA", mina),
    scalar_fn("AVERAGEA", averagea),
];

type R = Result<Val>;

fn ok(v: Value) -> R {
    Ok(Val::Scalar(v))
}

/// DAX comparison for ranking: BLANK compares as the other value's zero;
/// incomparable values fall back to the total order.
fn rank_cmp(a: &Value, b: &Value) -> Ordering {
    a.dax_cmp(b).unwrap_or_else(|_| a.total_cmp(b))
}

/// `ASC`/`DESC`/0/1/TRUE/FALSE order argument; `true` = descending.
fn order_arg(i: &Interp<'_>, env: &Env, f: &str, e: Option<&Expr>) -> Result<bool> {
    let Some(e) = e else { return Ok(true) };
    match &e.unparen().kind {
        ExprKind::Keyword(k) if k == "DESC" => Ok(true),
        ExprKind::Keyword(k) if k == "ASC" => Ok(false),
        _ => {
            let v = i.scalar(env, e)?;
            match ops::truth(&v) {
                Ok(Some(asc)) => Ok(!asc),
                Ok(None) => Ok(true),
                Err(_) => Err(DaxError::argument(f, "the order must be ASC or DESC")),
            }
        }
    }
}

/// 1 + the number of values (distinct values when `dense`) ranked strictly
/// before `value`.
fn rank_of(values: &[Value], value: &Value, desc: bool, dense: bool) -> i64 {
    let before = |v: &&Value| {
        let o = rank_cmp(v, value);
        if desc {
            o == Ordering::Greater
        } else {
            o == Ordering::Less
        }
    };
    let ahead: Vec<&Value> = values.iter().filter(before).collect();
    let count = if dense {
        let mut sorted = ahead;
        sorted.sort_by(|a, b| rank_cmp(a, b));
        sorted.dedup_by(|a, b| rank_cmp(a, b) == Ordering::Equal);
        sorted.len()
    } else {
        ahead.len()
    };
    i64::try_from(count).unwrap_or(i64::MAX).saturating_add(1)
}

/// `RANKX(table, expression[, value[, order[, ties]]])`.
fn rankx(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("RANKX", args, 2, 5)?;
    let table = i.table(env, arg("RANKX", args, 0)?)?;
    let expr = arg("RANKX", args, 1)?;
    let mut values = Vec::with_capacity(table.row_count());
    i.iterate(env, &table, |row_env, _| {
        values.push(i.scalar(row_env, expr)?);
        Ok(true)
    })?;
    let value = match opt_arg(args, 2) {
        Some(e) => i.scalar(env, e)?,
        None => i.scalar(env, expr)?,
    };
    let desc = order_arg(i, env, "RANKX", opt_arg(args, 3))?;
    let dense = match opt_arg(args, 4).map(|e| &e.unparen().kind) {
        None => false,
        Some(ExprKind::Keyword(k)) if k == "DENSE" => true,
        Some(ExprKind::Keyword(k)) if k == "SKIP" => false,
        Some(_) => {
            return Err(DaxError::argument(
                "RANKX",
                "the ties argument must be SKIP or DENSE",
            ));
        }
    };
    ok(Value::Integer(rank_of(&values, &value, desc, dense)))
}

/// Values of a column over the visible rows (duplicates and BLANKs kept).
fn column_values(i: &Interp<'_>, env: &Env, f: &str, e: &Expr) -> Result<Vec<Value>> {
    let (t, c) = i.column_arg(env, e, f)?;
    let vis = i.snapshot.visible(&env.filter, &t)?;
    let mut values = i.snapshot.column_values(&t, &c, &vis.rows)?;
    if vis.blank_row {
        values.push(Value::Blank);
    }
    Ok(values)
}

/// `RANK.EQ(value, column[, order])`: rank of `value` among the column's
/// visible values (ties share the rank, the next rank is skipped); BLANK
/// when `value` is BLANK or not in the column.
fn rank_eq(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("RANK.EQ", args, 2, 3)?;
    let value = i.scalar(env, arg("RANK.EQ", args, 0)?)?;
    if value.is_blank() {
        return ok(Value::Blank);
    }
    let values: Vec<Value> = column_values(i, env, "RANK.EQ", arg("RANK.EQ", args, 1)?)?
        .into_iter()
        .filter(|v| !v.is_blank())
        .collect();
    if !values
        .iter()
        .any(|v| rank_cmp(v, &value) == Ordering::Equal)
    {
        return ok(Value::Blank);
    }
    let desc = order_arg(i, env, "RANK.EQ", opt_arg(args, 2))?;
    ok(Value::Integer(rank_of(&values, &value, desc, false)))
}

/// Numeric inputs of a statistic, and whether they were all dates.
struct Numbers {
    values: Vec<f64>,
    dates: bool,
}

fn numbers(f: &str, values: Vec<Value>) -> Result<Numbers> {
    let mut out = Vec::with_capacity(values.len());
    let mut dates = true;
    for v in values {
        match &v {
            Value::Blank => continue,
            Value::Boolean(_) | Value::String(_) => return Err(ops::wrong_type(f, &v)),
            Value::DateTime(_) => {}
            _ => dates = false,
        }
        if let Some(x) = ops::to_f64(&v)? {
            out.push(x);
        }
    }
    Ok(Numbers {
        dates: dates && !out.is_empty(),
        values: out,
    })
}

/// Column values (column form) or row-by-row expression values (`X` form:
/// `args[0]` table, `args[1]` expression).
fn inputs(i: &Interp<'_>, env: &Env, f: &str, args: &[Expr], x: bool) -> Result<Numbers> {
    let values = if x {
        let table = i.table(env, arg(f, args, 0)?)?;
        let expr = arg(f, args, 1)?;
        let mut out = Vec::with_capacity(table.row_count());
        i.iterate(env, &table, |row_env, _| {
            out.push(i.scalar(row_env, expr)?);
            Ok(true)
        })?;
        out
    } else {
        column_values(i, env, f, arg(f, args, 0)?)?
    };
    numbers(f, values)
}

fn result(n: &Numbers, x: f64) -> Value {
    if n.dates {
        ops::date_from_serial(x)
    } else {
        Value::Double(x)
    }
}

/// Percentile of sorted values, `k` in `0..=1`: inclusive (`(n-1)k`) or
/// exclusive (`(n+1)k - 1`) rank with linear interpolation.
fn percentile(f: &str, mut n: Numbers, k: f64, exclusive: bool) -> Result<Value> {
    if n.values.is_empty() {
        return Ok(Value::Blank);
    }
    n.values.sort_by(f64::total_cmp);
    let count = n.values.len() as f64;
    let rank = if exclusive {
        let r = k * (count + 1.0) - 1.0;
        if !(0.0..=count - 1.0).contains(&r) {
            return Err(DaxError::argument(
                f,
                "k is outside the range the data allows",
            ));
        }
        r
    } else {
        if !(0.0..=1.0).contains(&k) {
            return Err(DaxError::argument(f, "k must be between 0 and 1"));
        }
        k * (count - 1.0)
    };
    let lo = rank.floor();
    let frac = rank - lo;
    let lo = lo as usize;
    let a = n.values.get(lo).copied().unwrap_or(0.0);
    let b = n.values.get(lo + 1).copied().unwrap_or(a);
    Ok(result(&n, a + (b - a) * frac))
}

fn k_arg(i: &Interp<'_>, env: &Env, f: &str, args: &[Expr], n: usize) -> Result<f64> {
    ops::to_f64(&i.scalar(env, arg(f, args, n)?)?)?
        .ok_or_else(|| DaxError::argument(f, "k cannot be blank"))
}

fn median(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("MEDIAN", args, 1, 1)?;
    ok(percentile(
        "MEDIAN",
        inputs(i, env, "MEDIAN", args, false)?,
        0.5,
        false,
    )?)
}

fn medianx(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("MEDIANX", args, 2, 2)?;
    ok(percentile(
        "MEDIANX",
        inputs(i, env, "MEDIANX", args, true)?,
        0.5,
        false,
    )?)
}

fn percentile_impl(i: &Interp<'_>, env: &Env, args: &[Expr], f: &str, x: bool, exc: bool) -> R {
    let k_pos = if x { 2 } else { 1 };
    arity(f, args, k_pos + 1, k_pos + 1)?;
    let k = k_arg(i, env, f, args, k_pos)?;
    ok(percentile(f, inputs(i, env, f, args, x)?, k, exc)?)
}

fn percentile_inc(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    percentile_impl(i, env, args, "PERCENTILE.INC", false, false)
}

fn percentile_exc(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    percentile_impl(i, env, args, "PERCENTILE.EXC", false, true)
}

fn percentilex_inc(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    percentile_impl(i, env, args, "PERCENTILEX.INC", true, false)
}

fn percentilex_exc(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    percentile_impl(i, env, args, "PERCENTILEX.EXC", true, true)
}

/// Variance (`sample`: n - 1 denominator); BLANK for too few values.
fn variance(n: &Numbers, sample: bool) -> Option<f64> {
    let count = n.values.len();
    if count == 0 || (sample && count < 2) {
        return None;
    }
    let mean = n.values.iter().sum::<f64>() / count as f64;
    let ss: f64 = n.values.iter().map(|x| (x - mean).powi(2)).sum();
    Some(ss / (count - usize::from(sample)) as f64)
}

fn spread(i: &Interp<'_>, env: &Env, args: &[Expr], f: &str, x: bool, sample: bool, sd: bool) -> R {
    let n = if x { 2 } else { 1 };
    arity(f, args, n, n)?;
    let values = inputs(i, env, f, args, x)?;
    ok(match variance(&values, sample) {
        Some(v) if sd => Value::Double(v.sqrt()),
        Some(v) => Value::Double(v),
        None => Value::Blank,
    })
}

fn stdev_s(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    spread(i, env, args, "STDEV.S", false, true, true)
}

fn stdev_p(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    spread(i, env, args, "STDEV.P", false, false, true)
}

fn stdevx_s(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    spread(i, env, args, "STDEVX.S", true, true, true)
}

fn stdevx_p(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    spread(i, env, args, "STDEVX.P", true, false, true)
}

fn var_s(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    spread(i, env, args, "VAR.S", false, true, false)
}

fn var_p(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    spread(i, env, args, "VAR.P", false, false, false)
}

fn varx_s(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    spread(i, env, args, "VARX.S", true, true, false)
}

fn varx_p(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    spread(i, env, args, "VARX.P", true, false, false)
}

/// Product of the non-blank values: an integer when every value is one and
/// the product fits, otherwise a double.
fn product_of(f: &str, values: Vec<Value>) -> Result<Value> {
    let mut int: Option<i64> = Some(1);
    let mut real = 1.0f64;
    let mut any = false;
    for v in values {
        match &v {
            Value::Blank => continue,
            Value::Boolean(_) | Value::String(_) => return Err(ops::wrong_type(f, &v)),
            Value::Integer(x) => int = int.and_then(|p| p.checked_mul(*x)),
            _ => int = None,
        }
        real *= ops::to_f64(&v)?.unwrap_or(1.0);
        any = true;
    }
    Ok(match (any, int) {
        (false, _) => Value::Blank,
        (true, Some(p)) => Value::Integer(p),
        (true, None) => Value::Double(real),
    })
}

fn product(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("PRODUCT", args, 1, 1)?;
    let values = column_values(i, env, "PRODUCT", arg("PRODUCT", args, 0)?)?;
    ok(product_of("PRODUCT", values)?)
}

fn productx(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("PRODUCTX", args, 2, 2)?;
    let table = i.table(env, arg("PRODUCTX", args, 0)?)?;
    let expr = arg("PRODUCTX", args, 1)?;
    let mut values = Vec::with_capacity(table.row_count());
    i.iterate(env, &table, |row_env, _| {
        values.push(i.scalar(row_env, expr)?);
        Ok(true)
    })?;
    ok(product_of("PRODUCTX", values)?)
}

/// Geometric mean `exp(mean(ln x))` of the non-blank values; every value
/// must be positive.
fn geometric_mean(f: &str, n: &Numbers) -> Result<Value> {
    if n.values.is_empty() {
        return Ok(Value::Blank);
    }
    if n.values.iter().any(|x| x.is_nan() || *x <= 0.0) {
        return Err(DaxError::argument(
            f,
            "an argument has an invalid value (every value must be greater than 0)",
        ));
    }
    let mean_ln = n.values.iter().map(|x| x.ln()).sum::<f64>() / n.values.len() as f64;
    let g = mean_ln.exp();
    if g.is_finite() {
        Ok(Value::Double(g))
    } else {
        Err(DaxError::argument(
            f,
            "the result is too large or too small",
        ))
    }
}

fn geomean(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("GEOMEAN", args, 1, 1)?;
    let n = inputs(i, env, "GEOMEAN", args, false)?;
    ok(geometric_mean("GEOMEAN", &n)?)
}

fn geomeanx(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("GEOMEANX", args, 2, 2)?;
    let n = inputs(i, env, "GEOMEANX", args, true)?;
    ok(geometric_mean("GEOMEANX", &n)?)
}

/// `MAXA`/`MINA`: numbers, dates and booleans (as 0/1) of the column; BLANK
/// without rows, 0 when no value is usable.
fn extreme_a(i: &Interp<'_>, env: &Env, args: &[Expr], f: &str, want: Ordering) -> R {
    arity(f, args, 1, 1)?;
    let values = column_values(i, env, f, arg(f, args, 0)?)?;
    if values.is_empty() {
        return ok(Value::Blank);
    }
    let mut best: Option<Value> = None;
    for v in values {
        let v = match v {
            Value::Blank | Value::String(_) => continue,
            Value::Boolean(b) => Value::Integer(i64::from(b)),
            other => other,
        };
        best = Some(match best {
            Some(b) if v.dax_cmp(&b)? != want => b,
            _ => v,
        });
    }
    ok(best.unwrap_or(Value::Integer(0)))
}

fn maxa(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    extreme_a(i, env, args, "MAXA", Ordering::Greater)
}

fn mina(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    extreme_a(i, env, args, "MINA", Ordering::Less)
}

/// `AVERAGEA(column)`: TRUE = 1, FALSE = 0, numeric text as its number,
/// other text (and `""`) as 0; BLANKs are skipped.
fn averagea(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("AVERAGEA", args, 1, 1)?;
    let values = column_values(i, env, "AVERAGEA", arg("AVERAGEA", args, 0)?)?;
    if values.is_empty() {
        return ok(Value::Blank);
    }
    let mut sum = 0.0;
    let mut count = 0usize;
    for v in &values {
        let x = match v {
            Value::Blank => continue,
            Value::String(s) => match ops::parse_number(s) {
                Some(n) => ops::to_f64(&n)?.unwrap_or(0.0),
                None => 0.0,
            },
            other => ops::to_f64(other)?.unwrap_or(0.0),
        };
        sum += x;
        count += 1;
    }
    if count == 0 {
        return ok(Value::Double(0.0));
    }
    ok(Value::Double(sum / count as f64))
}
