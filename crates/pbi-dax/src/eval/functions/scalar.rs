//! Logical, math, text and date/time functions.

use std::sync::Arc;

use pbi_engine::Value;
use pbi_engine::value::{DECIMAL_FACTOR, MICROS_PER_DAY};

use super::{Builtin, arg, arity, opt_arg, scalar_fn};
use crate::eval::context::Env;
use crate::eval::error::{DaxError, Result};
use crate::eval::interp::{Interp, Val};
use crate::eval::ops;
use crate::syntax::{BinaryOp, Expr};

pub(super) const BUILTINS: &[Builtin] = &[
    // logical
    scalar_fn("IF", if_),
    scalar_fn("SWITCH", switch),
    scalar_fn("COALESCE", coalesce),
    scalar_fn("ISBLANK", isblank),
    scalar_fn("BLANK", blank),
    scalar_fn("TRUE", true_),
    scalar_fn("FALSE", false_),
    scalar_fn("NOT", not),
    scalar_fn("AND", and),
    scalar_fn("OR", or),
    scalar_fn("IFERROR", iferror),
    scalar_fn("ISERROR", iserror),
    scalar_fn("ISNUMBER", isnumber),
    scalar_fn("ISTEXT", istext),
    scalar_fn("ISLOGICAL", islogical),
    scalar_fn("ISNONTEXT", isnontext),
    scalar_fn("ISEVEN", iseven),
    scalar_fn("ISODD", isodd),
    scalar_fn("CONVERT", convert),
    // math
    scalar_fn("DIVIDE", divide),
    scalar_fn("ABS", abs),
    scalar_fn("INT", int),
    scalar_fn("ROUND", round),
    scalar_fn("ROUNDUP", roundup),
    scalar_fn("ROUNDDOWN", rounddown),
    scalar_fn("TRUNC", trunc),
    scalar_fn("MOD", mod_),
    scalar_fn("POWER", power),
    scalar_fn("SQRT", sqrt),
    scalar_fn("SIGN", sign),
    scalar_fn("CURRENCY", currency),
    // text
    scalar_fn("UPPER", upper),
    scalar_fn("LOWER", lower),
    scalar_fn("LEN", len),
    scalar_fn("LEFT", left),
    scalar_fn("RIGHT", right),
    scalar_fn("MID", mid),
    scalar_fn("TRIM", trim),
    scalar_fn("CONCATENATE", concatenate),
    scalar_fn("SUBSTITUTE", substitute),
    scalar_fn("CONTAINSSTRING", containsstring),
    scalar_fn("CONTAINSSTRINGEXACT", containsstringexact),
    scalar_fn("EXACT", exact),
    scalar_fn("VALUE", value),
    scalar_fn("FORMAT", format),
    scalar_fn("FIXED", fixed),
    scalar_fn("UNICHAR", unichar),
    scalar_fn("UNICODE", unicode),
    scalar_fn("REPT", rept),
    scalar_fn("SEARCH", search),
    scalar_fn("FIND", find),
    scalar_fn("REPLACE", replace),
    scalar_fn("COMBINEVALUES", combinevalues),
    // date/time
    scalar_fn("NOW", now),
    scalar_fn("TODAY", today),
    scalar_fn("UTCNOW", utcnow),
    scalar_fn("UTCTODAY", utctoday),
    scalar_fn("DATE", date),
    scalar_fn("YEAR", year),
    scalar_fn("MONTH", month),
    scalar_fn("DAY", day),
    scalar_fn("HOUR", hour),
    scalar_fn("MINUTE", minute),
    scalar_fn("SECOND", second),
];

type R = Result<Val>;

fn ok(v: Value) -> R {
    Ok(Val::Scalar(v))
}

fn text(v: impl Into<String>) -> Value {
    Value::String(Arc::from(v.into()))
}

/// Scalar value of argument `i`.
fn sarg(i: &Interp<'_>, env: &Env, f: &str, args: &[Expr], n: usize) -> Result<Value> {
    i.scalar(env, arg(f, args, n)?)
}

/// Scalar value of optional argument `n` (BLANK when missing).
fn sopt(i: &Interp<'_>, env: &Env, args: &[Expr], n: usize) -> Result<Value> {
    match opt_arg(args, n) {
        Some(e) => i.scalar(env, e),
        None => Ok(Value::Blank),
    }
}

// ---- logical -----------------------------------------------------------------

fn if_(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("IF", args, 2, 3)?;
    if i.condition(env, arg("IF", args, 0)?)? {
        i.eval(env, arg("IF", args, 1)?)
    } else {
        match opt_arg(args, 2) {
            Some(e) => i.eval(env, e),
            None => ok(Value::Blank),
        }
    }
}

fn switch(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("SWITCH", args, 3, usize::MAX)?;
    let v = sarg(i, env, "SWITCH", args, 0)?;
    let rest = args.get(1..).unwrap_or_default();
    let (pairs, remainder) = rest.as_chunks::<2>();
    for [cand, result] in pairs {
        if v.dax_eq(&i.scalar(env, cand)?)? {
            return i.eval(env, result);
        }
    }
    match remainder {
        [otherwise] => i.eval(env, otherwise),
        _ => ok(Value::Blank),
    }
}

fn coalesce(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("COALESCE", args, 2, usize::MAX)?;
    for a in args {
        let v = i.scalar(env, a)?;
        if !v.is_blank() {
            return ok(v);
        }
    }
    ok(Value::Blank)
}

fn isblank(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("ISBLANK", args, 1, 1)?;
    ok(Value::Boolean(sarg(i, env, "ISBLANK", args, 0)?.is_blank()))
}

fn blank(_: &Interp<'_>, _: &Env, args: &[Expr]) -> R {
    arity("BLANK", args, 0, 0)?;
    ok(Value::Blank)
}

fn true_(_: &Interp<'_>, _: &Env, args: &[Expr]) -> R {
    arity("TRUE", args, 0, 0)?;
    ok(Value::Boolean(true))
}

fn false_(_: &Interp<'_>, _: &Env, args: &[Expr]) -> R {
    arity("FALSE", args, 0, 0)?;
    ok(Value::Boolean(false))
}

fn not(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("NOT", args, 1, 1)?;
    let v = sarg(i, env, "NOT", args, 0)?;
    ok(Value::Boolean(!ops::truth(&v)?.unwrap_or(false)))
}

fn and(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("AND", args, 2, 2)?;
    let l = ops::truth(&sarg(i, env, "AND", args, 0)?)?;
    if l == Some(false) {
        return ok(Value::Boolean(false));
    }
    let r = ops::truth(&sarg(i, env, "AND", args, 1)?)?;
    ok(ops::and(l, r))
}

fn or(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("OR", args, 2, 2)?;
    let l = ops::truth(&sarg(i, env, "OR", args, 0)?)?;
    if l == Some(true) {
        return ok(Value::Boolean(true));
    }
    let r = ops::truth(&sarg(i, env, "OR", args, 1)?)?;
    ok(ops::or(l, r))
}

fn iferror(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("IFERROR", args, 2, 2)?;
    match i.eval(env, arg("IFERROR", args, 0)?) {
        Ok(v) => Ok(v),
        Err(e) if e.is_fatal() => Err(e),
        Err(_) => i.eval(env, arg("IFERROR", args, 1)?),
    }
}

fn iserror(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("ISERROR", args, 1, 1)?;
    match i.eval(env, arg("ISERROR", args, 0)?) {
        Ok(_) => ok(Value::Boolean(false)),
        Err(e) if e.is_fatal() => Err(e),
        Err(_) => ok(Value::Boolean(true)),
    }
}

fn isnumber(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("ISNUMBER", args, 1, 1)?;
    ok(Value::Boolean(
        sarg(i, env, "ISNUMBER", args, 0)?.is_numeric(),
    ))
}

fn istext(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("ISTEXT", args, 1, 1)?;
    ok(Value::Boolean(matches!(
        sarg(i, env, "ISTEXT", args, 0)?,
        Value::String(_)
    )))
}

fn islogical(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("ISLOGICAL", args, 1, 1)?;
    ok(Value::Boolean(matches!(
        sarg(i, env, "ISLOGICAL", args, 0)?,
        Value::Boolean(_)
    )))
}

fn isnontext(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("ISNONTEXT", args, 1, 1)?;
    ok(Value::Boolean(!matches!(
        sarg(i, env, "ISNONTEXT", args, 0)?,
        Value::String(_)
    )))
}

/// Integer part (truncated) of a numeric argument for `ISEVEN`/`ISODD`.
fn parity(i: &Interp<'_>, env: &Env, f: &str, args: &[Expr]) -> Result<i64> {
    arity(f, args, 1, 1)?;
    let v = sarg(i, env, f, args, 0)?;
    if matches!(v, Value::Boolean(_)) {
        return Err(ops::wrong_type(f, &v));
    }
    let x = ops::to_f64(&v)?.unwrap_or(0.0).trunc();
    Ok(if x.is_finite() { (x % 2.0) as i64 } else { 0 })
}

fn iseven(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    ok(Value::Boolean(parity(i, env, "ISEVEN", args)? == 0))
}

fn isodd(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    ok(Value::Boolean(parity(i, env, "ISODD", args)? != 0))
}

/// `CONVERT(expr, INTEGER | DOUBLE | STRING | BOOLEAN | CURRENCY |
/// DATETIME)`.
fn convert(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("CONVERT", args, 2, 2)?;
    let v = sarg(i, env, "CONVERT", args, 0)?;
    let word = match &arg("CONVERT", args, 1)?.unparen().kind {
        crate::syntax::ExprKind::Keyword(k) | crate::syntax::ExprKind::TableRef { name: k } => {
            k.to_ascii_uppercase()
        }
        _ => String::new(),
    };
    if v.is_blank() {
        return ok(Value::Blank);
    }
    ok(match word.as_str() {
        "INTEGER" => match ops::to_i64(&v)? {
            Some(n) => Value::Integer(n),
            None => Value::Blank,
        },
        "DOUBLE" => ops::to_f64(&v)?.map_or(Value::Blank, Value::Double),
        "STRING" => text(ops::to_text(&v)),
        "BOOLEAN" => ops::truth(&v)?.map_or(Value::Blank, Value::Boolean),
        "CURRENCY" => to_currency(&v)?,
        "DATETIME" => ops::to_datetime(&v)?.map_or(Value::Blank, Value::DateTime),
        _ => {
            return Err(DaxError::argument(
                "CONVERT",
                "the type must be INTEGER, DOUBLE, STRING, BOOLEAN, CURRENCY or DATETIME",
            ));
        }
    })
}

// ---- math --------------------------------------------------------------------

fn to_currency(v: &Value) -> Result<Value> {
    match v {
        Value::Decimal(_) | Value::Blank => Ok(v.clone()),
        other => match ops::to_f64(other)? {
            Some(x) => Value::decimal_from_f64(x).ok_or_else(|| {
                DaxError::type_error(format!(
                    "Cannot convert value '{}' to type Currency.",
                    ops::to_text(other)
                ))
            }),
            None => Ok(Value::Blank),
        },
    }
}

fn currency(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("CURRENCY", args, 1, 1)?;
    ok(to_currency(&sarg(i, env, "CURRENCY", args, 0)?)?)
}

fn divide(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("DIVIDE", args, 2, 3)?;
    let n = sarg(i, env, "DIVIDE", args, 0)?;
    let d = sarg(i, env, "DIVIDE", args, 1)?;
    match ops::to_f64(&d)? {
        None => ok(sopt(i, env, args, 2)?),
        Some(0.0) => ok(sopt(i, env, args, 2)?),
        Some(_) => ok(ops::arith(BinaryOp::Div, &n, &d)?),
    }
}

fn abs(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("ABS", args, 1, 1)?;
    let v = ops::to_numeric(&sarg(i, env, "ABS", args, 0)?)?;
    ok(match v {
        Value::Integer(x) => x
            .checked_abs()
            .map_or(Value::Double((x as f64).abs()), Value::Integer),
        Value::Decimal(x) => x
            .checked_abs()
            .map_or(Value::Double((x as f64).abs()), Value::Decimal),
        Value::Double(x) => Value::Double(x.abs()),
        Value::DateTime(_) => Value::Double(ops::to_f64(&v)?.unwrap_or(0.0).abs()),
        other => other,
    })
}

fn int(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("INT", args, 1, 1)?;
    let v = ops::to_numeric(&sarg(i, env, "INT", args, 0)?)?;
    ok(match v {
        Value::Blank | Value::Integer(_) => v,
        Value::Decimal(d) => {
            Value::Integer(i64::try_from(d.div_euclid(DECIMAL_FACTOR)).unwrap_or(i64::MAX))
        }
        other => {
            let f = ops::to_f64(&other)?.unwrap_or(0.0).floor();
            if f.is_finite() && f.abs() < 9.0e18 {
                Value::Integer(f as i64)
            } else {
                Value::Double(f)
            }
        }
    })
}

#[derive(Clone, Copy, PartialEq)]
enum RoundMode {
    Nearest,
    Up,
    Down,
}

fn round_f64(x: f64, digits: i64, mode: RoundMode) -> f64 {
    let scale = 10f64.powi(digits.clamp(-300, 300) as i32);
    // Round to 15 significant digits first so binary artefacts such as
    // 1.005 * 100 = 100.49999999999999 round like DAX/Excel do.
    let scaled = x * scale;
    let scaled = format!("{scaled:.14e}").parse::<f64>().unwrap_or(scaled);
    let r = match mode {
        RoundMode::Nearest => scaled.round(),
        RoundMode::Up => scaled.abs().ceil().copysign(scaled),
        RoundMode::Down => scaled.trunc(),
    };
    r / scale
}

fn round_i128(v: i128, factor: i128, mode: RoundMode) -> Option<i128> {
    if factor <= 1 {
        return Some(v);
    }
    let q = v / factor;
    let r = v % factor;
    let adjust = match mode {
        RoundMode::Nearest => r.unsigned_abs() * 2 >= factor.unsigned_abs(),
        RoundMode::Up => r != 0,
        RoundMode::Down => false,
    };
    let q = if adjust { q + v.signum() } else { q };
    q.checked_mul(factor)
}

fn round_value(v: &Value, digits: i64, mode: RoundMode) -> Result<Value> {
    let v = ops::to_numeric(v)?;
    Ok(match v {
        Value::Blank => Value::Blank,
        Value::Integer(x) if digits >= 0 => Value::Integer(x),
        Value::Integer(x) => {
            let factor = 10i128.checked_pow(digits.unsigned_abs().min(30) as u32);
            match factor.and_then(|f| round_i128(i128::from(x), f, mode)) {
                Some(r) => i64::try_from(r).map_or(Value::Double(r as f64), Value::Integer),
                None => Value::Integer(0),
            }
        }
        Value::Decimal(x) if digits >= 4 => Value::Decimal(x),
        Value::Decimal(x) => {
            let factor = 10i128.checked_pow((4 - digits).clamp(0, 30) as u32);
            match factor.and_then(|f| round_i128(x, f, mode)) {
                Some(r) => Value::Decimal(r),
                None => Value::Decimal(0),
            }
        }
        other => Value::Double(round_f64(ops::to_f64(&other)?.unwrap_or(0.0), digits, mode)),
    })
}

fn rounding(name: &str, mode: RoundMode, min: usize) -> impl Fn(&Interp<'_>, &Env, &[Expr]) -> R {
    let name = name.to_owned();
    move |i, env, args| {
        arity(&name, args, min, 2)?;
        let v = sarg(i, env, &name, args, 0)?;
        let digits = ops::to_i64(&sopt(i, env, args, 1)?)?.unwrap_or(0);
        ok(round_value(&v, digits, mode)?)
    }
}

fn round(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    rounding("ROUND", RoundMode::Nearest, 1)(i, env, args)
}

fn roundup(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    rounding("ROUNDUP", RoundMode::Up, 1)(i, env, args)
}

fn rounddown(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    rounding("ROUNDDOWN", RoundMode::Down, 1)(i, env, args)
}

fn trunc(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    rounding("TRUNC", RoundMode::Down, 1)(i, env, args)
}

fn mod_(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("MOD", args, 2, 2)?;
    let n = ops::to_numeric(&sarg(i, env, "MOD", args, 0)?)?;
    let d = ops::to_numeric(&sarg(i, env, "MOD", args, 1)?)?;
    let nf = ops::to_f64(&n)?.unwrap_or(0.0);
    let df = ops::to_f64(&d)?.unwrap_or(0.0);
    if df == 0.0 {
        return Err(DaxError::argument("MOD", "division by zero"));
    }
    let int_n = match n {
        Value::Integer(a) => Some(a),
        Value::Blank => Some(0),
        _ => None,
    };
    if let (Some(a), Value::Integer(b)) = (int_n, &d) {
        let r = a.checked_rem(*b).unwrap_or(0);
        let r = if r != 0 && (r < 0) != (*b < 0) {
            r + b
        } else {
            r
        };
        return ok(Value::Integer(r));
    }
    ok(Value::Double(nf - df * (nf / df).floor()))
}

fn power(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("POWER", args, 2, 2)?;
    let b = sarg(i, env, "POWER", args, 0)?;
    let e = sarg(i, env, "POWER", args, 1)?;
    ok(ops::arith(BinaryOp::Pow, &b, &e)?)
}

fn sqrt(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("SQRT", args, 1, 1)?;
    match ops::to_f64(&sarg(i, env, "SQRT", args, 0)?)? {
        None => ok(Value::Double(0.0)),
        Some(x) if x < 0.0 => Err(DaxError::argument(
            "SQRT",
            "cannot take the square root of a negative number",
        )),
        Some(x) => ok(Value::Double(x.sqrt())),
    }
}

fn sign(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("SIGN", args, 1, 1)?;
    let x = ops::to_f64(&sarg(i, env, "SIGN", args, 0)?)?.unwrap_or(0.0);
    ok(Value::Integer(if x > 0.0 {
        1
    } else if x < 0.0 {
        -1
    } else {
        0
    }))
}

// ---- text --------------------------------------------------------------------

/// Text of an argument; `None` for BLANK.
fn text_arg(i: &Interp<'_>, env: &Env, f: &str, args: &[Expr], n: usize) -> Result<Option<String>> {
    let v = sarg(i, env, f, args, n)?;
    Ok((!v.is_blank()).then(|| ops::to_text(&v)))
}

fn count_arg(i: &Interp<'_>, env: &Env, f: &str, args: &[Expr], n: usize) -> Result<usize> {
    match opt_arg(args, n) {
        None => Ok(1),
        Some(e) => {
            let c = ops::to_i64(&i.scalar(env, e)?)?.unwrap_or(0);
            usize::try_from(c)
                .map_err(|_| DaxError::argument(f, "the number of characters cannot be negative"))
        }
    }
}

fn upper(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("UPPER", args, 1, 1)?;
    ok(text_arg(i, env, "UPPER", args, 0)?.map_or(Value::Blank, |t| text(t.to_uppercase())))
}

fn lower(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("LOWER", args, 1, 1)?;
    ok(text_arg(i, env, "LOWER", args, 0)?.map_or(Value::Blank, |t| text(t.to_lowercase())))
}

fn len(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("LEN", args, 1, 1)?;
    let t = text_arg(i, env, "LEN", args, 0)?.unwrap_or_default();
    ok(Value::Integer(
        i64::try_from(t.chars().count()).unwrap_or(i64::MAX),
    ))
}

fn left(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("LEFT", args, 1, 2)?;
    let n = count_arg(i, env, "LEFT", args, 1)?;
    ok(
        text_arg(i, env, "LEFT", args, 0)?.map_or(Value::Blank, |t| {
            text(t.chars().take(n).collect::<String>())
        }),
    )
}

fn right(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("RIGHT", args, 1, 2)?;
    let n = count_arg(i, env, "RIGHT", args, 1)?;
    ok(
        text_arg(i, env, "RIGHT", args, 0)?.map_or(Value::Blank, |t| {
            let len = t.chars().count();
            text(t.chars().skip(len.saturating_sub(n)).collect::<String>())
        }),
    )
}

fn mid(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("MID", args, 3, 3)?;
    let start = ops::to_i64(&sarg(i, env, "MID", args, 1)?)?.unwrap_or(0);
    if start < 1 {
        return Err(DaxError::argument(
            "MID",
            "the start position must be at least 1",
        ));
    }
    let n = count_arg(i, env, "MID", args, 2)?;
    let skip = usize::try_from(start - 1).unwrap_or(usize::MAX);
    ok(text_arg(i, env, "MID", args, 0)?.map_or(Value::Blank, |t| {
        text(t.chars().skip(skip).take(n).collect::<String>())
    }))
}

fn trim(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("TRIM", args, 1, 1)?;
    ok(
        text_arg(i, env, "TRIM", args, 0)?.map_or(Value::Blank, |t| {
            text(
                t.split(' ')
                    .filter(|w| !w.is_empty())
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        }),
    )
}

fn concatenate(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("CONCATENATE", args, 2, 2)?;
    let a = sarg(i, env, "CONCATENATE", args, 0)?;
    let b = sarg(i, env, "CONCATENATE", args, 1)?;
    ok(ops::concat(&a, &b))
}

fn substitute(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("SUBSTITUTE", args, 3, 4)?;
    let Some(t) = text_arg(i, env, "SUBSTITUTE", args, 0)? else {
        return ok(Value::Blank);
    };
    let old = ops::to_text(&sarg(i, env, "SUBSTITUTE", args, 1)?);
    let new = ops::to_text(&sarg(i, env, "SUBSTITUTE", args, 2)?);
    if old.is_empty() {
        return ok(text(t));
    }
    match ops::to_i64(&sopt(i, env, args, 3)?)? {
        None => ok(text(t.replace(&old, &new))),
        Some(n) if n < 1 => Err(DaxError::argument(
            "SUBSTITUTE",
            "the instance number must be at least 1",
        )),
        Some(n) => {
            let nth = usize::try_from(n - 1).unwrap_or(usize::MAX);
            match t.match_indices(&old).nth(nth) {
                Some((pos, _)) => {
                    let mut out = String::with_capacity(t.len());
                    out.push_str(t.get(..pos).unwrap_or_default());
                    out.push_str(&new);
                    out.push_str(t.get(pos + old.len()..).unwrap_or_default());
                    ok(text(out))
                }
                None => ok(text(t)),
            }
        }
    }
}

/// Case-insensitive wildcard match (`?` one char, `*` any run, `~` escapes)
/// of `pattern` anywhere in `text`.
pub(crate) fn wildcard_contains(text: &str, pattern: &str) -> bool {
    #[derive(Clone, Copy, PartialEq)]
    enum Tok {
        Char(char),
        One,
        Any,
    }
    let mut toks = vec![Tok::Any];
    let mut chars = pattern.chars().flat_map(char::to_lowercase).peekable();
    while let Some(c) = chars.next() {
        toks.push(match c {
            '~' => match chars.next() {
                Some(n) => Tok::Char(n),
                None => Tok::Char('~'),
            },
            '?' => Tok::One,
            '*' => Tok::Any,
            c => Tok::Char(c),
        });
    }
    toks.push(Tok::Any);
    let text: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    // dp[j]: pattern prefix of length j matches the text prefix so far.
    let mut dp = vec![false; toks.len() + 1];
    dp[0] = true;
    for j in 1..=toks.len() {
        dp[j] = dp[j - 1] && toks[j - 1] == Tok::Any;
    }
    for c in text {
        let mut next = vec![false; toks.len() + 1];
        for j in 1..=toks.len() {
            next[j] = match toks[j - 1] {
                Tok::Any => next[j - 1] || dp[j],
                Tok::One => dp[j - 1],
                Tok::Char(p) => dp[j - 1] && p == c,
            };
        }
        dp = next;
    }
    dp[toks.len()]
}

fn containsstring(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("CONTAINSSTRING", args, 2, 2)?;
    let within = ops::to_text(&sarg(i, env, "CONTAINSSTRING", args, 0)?);
    let find = ops::to_text(&sarg(i, env, "CONTAINSSTRING", args, 1)?);
    ok(Value::Boolean(wildcard_contains(&within, &find)))
}

fn containsstringexact(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("CONTAINSSTRINGEXACT", args, 2, 2)?;
    let within = ops::to_text(&sarg(i, env, "CONTAINSSTRINGEXACT", args, 0)?);
    let find = ops::to_text(&sarg(i, env, "CONTAINSSTRINGEXACT", args, 1)?);
    ok(Value::Boolean(within.contains(&find)))
}

fn exact(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("EXACT", args, 2, 2)?;
    let a = ops::to_text(&sarg(i, env, "EXACT", args, 0)?);
    let b = ops::to_text(&sarg(i, env, "EXACT", args, 1)?);
    ok(Value::Boolean(a == b))
}

fn value(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("VALUE", args, 1, 1)?;
    let v = sarg(i, env, "VALUE", args, 0)?;
    ok(match &v {
        Value::Blank => Value::Blank,
        Value::String(s) => match ops::parse_number(s) {
            Some(n) => Value::Double(ops::to_f64(&n)?.unwrap_or(0.0)),
            None => match ops::parse_datetime(s) {
                Some(d) => Value::Double(ops::to_f64(&d)?.unwrap_or(0.0)),
                None => {
                    return Err(DaxError::type_error(format!(
                        "Cannot convert value '{s}' of type Text to type Number."
                    )));
                }
            },
        },
        Value::Boolean(_) => return Err(ops::wrong_type("VALUE", &v)),
        Value::DateTime(_) => Value::Double(ops::to_f64(&v)?.unwrap_or(0.0)),
        other => other.clone(),
    })
}

/// `FORMAT(value, format_string[, locale])`: text per a Power BI format
/// string (see [`crate::format`](mod@crate::format)); BLANK formats as `""`, a BLANK format
/// string means `General`.
fn format(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("FORMAT", args, 2, 3)?;
    let v = sarg(i, env, "FORMAT", args, 0)?;
    let f = sarg(i, env, "FORMAT", args, 1)?;
    let locale = match opt_arg(args, 2) {
        Some(e) => ops::to_text(&i.scalar(env, e)?),
        None => "en-US".to_owned(),
    };
    let fmt = (!f.is_blank()).then(|| ops::to_text(&f));
    ok(text(crate::format::format_value(
        &v,
        fmt.as_deref(),
        &locale,
    )))
}

/// `FIXED(number[, decimals = 2[, no_commas = FALSE]])`.
fn fixed(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("FIXED", args, 1, 3)?;
    let v = sarg(i, env, "FIXED", args, 0)?;
    if v.is_blank() {
        return ok(Value::Blank);
    }
    let decimals = match opt_arg(args, 1) {
        Some(e) => ops::to_i64(&i.scalar(env, e)?)?.unwrap_or(0),
        None => 2,
    }
    .clamp(-30, 30);
    let no_commas = match opt_arg(args, 2) {
        Some(e) => ops::is_true(&i.scalar(env, e)?)?,
        None => false,
    };
    let rounded = round_value(&v, decimals, RoundMode::Nearest)?;
    let mut pattern = String::from(if no_commas { "0" } else { "#,0" });
    if decimals > 0 {
        pattern.push('.');
        pattern.push_str(&"0".repeat(decimals as usize));
    }
    ok(text(crate::format::format_value(
        &Value::Double(ops::to_f64(&rounded)?.unwrap_or(0.0)),
        Some(&pattern),
        "en-US",
    )))
}

fn unichar(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("UNICHAR", args, 1, 1)?;
    let Some(code) = ops::to_i64(&sarg(i, env, "UNICHAR", args, 0)?)? else {
        return ok(Value::Blank);
    };
    match u32::try_from(code).ok().and_then(char::from_u32) {
        Some(c) if code > 0 => ok(text(c.to_string())),
        _ => Err(DaxError::argument(
            "UNICHAR",
            format!("{code} is not a valid Unicode code point"),
        )),
    }
}

fn unicode(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("UNICODE", args, 1, 1)?;
    let t = ops::to_text(&sarg(i, env, "UNICODE", args, 0)?);
    match t.chars().next() {
        Some(c) => ok(Value::Integer(i64::from(u32::from(c)))),
        None => Err(DaxError::argument("UNICODE", "the text is empty")),
    }
}

/// Maximum length of a `REPT` result (characters).
const MAX_REPT_CHARS: usize = 1 << 20;

fn rept(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("REPT", args, 2, 2)?;
    let t = ops::to_text(&sarg(i, env, "REPT", args, 0)?);
    let n = ops::to_i64(&sarg(i, env, "REPT", args, 1)?)?.unwrap_or(0);
    let n = usize::try_from(n)
        .map_err(|_| DaxError::argument("REPT", "the number of times cannot be negative"))?;
    if t.chars().count().saturating_mul(n) > MAX_REPT_CHARS {
        return Err(DaxError::argument("REPT", "the result is too long"));
    }
    if t.is_empty() || n == 0 {
        return ok(Value::Blank);
    }
    ok(text(t.repeat(n)))
}

/// Pattern tokens for `SEARCH` wildcards.
#[derive(Clone, Copy, PartialEq)]
enum Wild {
    Char(char),
    One,
    Any,
}

fn wild_tokens(pattern: &str) -> Vec<Wild> {
    let mut toks = Vec::new();
    let mut chars = pattern.chars().flat_map(char::to_lowercase).peekable();
    while let Some(c) = chars.next() {
        toks.push(match c {
            '~' => Wild::Char(chars.next().unwrap_or('~')),
            '?' => Wild::One,
            '*' => Wild::Any,
            c => Wild::Char(c),
        });
    }
    toks
}

/// `true` if `toks` matches a prefix of `text`.
fn wild_prefix(text: &[char], toks: &[Wild]) -> bool {
    let mut dp = vec![false; toks.len() + 1];
    dp[0] = true;
    for j in 1..=toks.len() {
        dp[j] = dp[j - 1] && toks[j - 1] == Wild::Any;
    }
    if dp[toks.len()] {
        return true;
    }
    for c in text {
        let mut next = vec![false; toks.len() + 1];
        for j in 1..=toks.len() {
            next[j] = match toks[j - 1] {
                Wild::Any => next[j - 1] || dp[j],
                Wild::One => dp[j - 1],
                Wild::Char(p) => dp[j - 1] && p == *c,
            };
        }
        dp = next;
        if dp[toks.len()] {
            return true;
        }
    }
    false
}

/// `SEARCH`/`FIND(find, within[, start[, not_found]])`: 1-based position.
fn locate(i: &Interp<'_>, env: &Env, args: &[Expr], f: &str, wildcards: bool) -> R {
    arity(f, args, 2, 4)?;
    let needle = ops::to_text(&sarg(i, env, f, args, 0)?);
    let hay: Vec<char> = ops::to_text(&sarg(i, env, f, args, 1)?).chars().collect();
    let start = match opt_arg(args, 2) {
        Some(e) => ops::to_i64(&i.scalar(env, e)?)?.unwrap_or(1),
        None => 1,
    };
    let from = usize::try_from(start - 1).ok().filter(|s| *s <= hay.len());
    let found = from.and_then(|from| {
        if wildcards {
            let lower: Vec<char> = hay
                .iter()
                .map(|c| c.to_lowercase().next().unwrap_or(*c))
                .collect();
            let toks = wild_tokens(&needle);
            (from..=lower.len()).find(|s| wild_prefix(lower.get(*s..).unwrap_or_default(), &toks))
        } else {
            let n: Vec<char> = needle.chars().collect();
            (from..=hay.len()).find(|s| hay.get(*s..s + n.len()).is_some_and(|w| w == n.as_slice()))
        }
    });
    match (found, args.len() > 3) {
        (Some(p), _) => ok(Value::Integer(i64::try_from(p + 1).unwrap_or(i64::MAX))),
        (None, true) => ok(sopt(i, env, args, 3)?),
        (None, false) => Err(DaxError::evaluation(format!(
            "{f}: the text '{needle}' was not found."
        ))),
    }
}

fn search(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    locate(i, env, args, "SEARCH", true)
}

fn find(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    locate(i, env, args, "FIND", false)
}

/// `REPLACE(old_text, start, num_chars, new_text)`.
fn replace(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("REPLACE", args, 4, 4)?;
    let old: Vec<char> = ops::to_text(&sarg(i, env, "REPLACE", args, 0)?)
        .chars()
        .collect();
    let start = ops::to_i64(&sarg(i, env, "REPLACE", args, 1)?)?.unwrap_or(1);
    if start < 1 {
        return Err(DaxError::argument(
            "REPLACE",
            "the start position must be at least 1",
        ));
    }
    let count = count_arg(i, env, "REPLACE", args, 2)?;
    let new = ops::to_text(&sarg(i, env, "REPLACE", args, 3)?);
    let from = usize::try_from(start - 1)
        .unwrap_or(usize::MAX)
        .min(old.len());
    let to = from.saturating_add(count).min(old.len());
    let mut out: String = old.get(..from).unwrap_or_default().iter().collect();
    out.push_str(&new);
    out.extend(old.get(to..).unwrap_or_default());
    ok(text(out))
}

/// `COMBINEVALUES(delimiter, expr1, expr2, …)`.
fn combinevalues(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("COMBINEVALUES", args, 3, usize::MAX)?;
    let delimiter = ops::to_text(&sarg(i, env, "COMBINEVALUES", args, 0)?);
    let parts = args
        .get(1..)
        .unwrap_or_default()
        .iter()
        .map(|a| Ok(ops::to_text(&i.scalar(env, a)?)))
        .collect::<Result<Vec<_>>>()?;
    ok(text(parts.join(&delimiter)))
}

// ---- date/time -----------------------------------------------------------------

fn naive_micros(ndt: chrono::NaiveDateTime) -> Value {
    Value::DateTime(ndt.and_utc().timestamp_micros())
}

fn midnight(v: Value) -> Value {
    match v {
        Value::DateTime(us) => Value::DateTime(us - us.rem_euclid(MICROS_PER_DAY)),
        other => other,
    }
}

fn now(_: &Interp<'_>, _: &Env, args: &[Expr]) -> R {
    arity("NOW", args, 0, 0)?;
    ok(naive_micros(chrono::Local::now().naive_local()))
}

fn today(_: &Interp<'_>, _: &Env, args: &[Expr]) -> R {
    arity("TODAY", args, 0, 0)?;
    ok(midnight(naive_micros(chrono::Local::now().naive_local())))
}

fn utcnow(_: &Interp<'_>, _: &Env, args: &[Expr]) -> R {
    arity("UTCNOW", args, 0, 0)?;
    ok(naive_micros(chrono::Utc::now().naive_utc()))
}

fn utctoday(_: &Interp<'_>, _: &Env, args: &[Expr]) -> R {
    arity("UTCTODAY", args, 0, 0)?;
    ok(midnight(naive_micros(chrono::Utc::now().naive_utc())))
}

fn date(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("DATE", args, 3, 3)?;
    let mut parts = [0i64; 3];
    for (n, slot) in parts.iter_mut().enumerate() {
        *slot = ops::to_i64(&sarg(i, env, "DATE", args, n)?)?.unwrap_or(0);
    }
    let [y, m, d] = parts;
    ok(ops::make_date(y, m, d)?)
}

/// Date part of an argument (BLANK is the serial date 0, 1899-12-30).
fn date_part(i: &Interp<'_>, env: &Env, f: &str, args: &[Expr]) -> Result<(i64, u32, u32, i64)> {
    arity(f, args, 1, 1)?;
    let v = sarg(i, env, f, args, 0)?;
    let us = match ops::to_datetime(&v)? {
        Some(us) => us,
        None => match ops::date_from_serial(0.0) {
            Value::DateTime(us) => us,
            _ => 0,
        },
    };
    Ok(ops::civil(us))
}

fn year(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    ok(Value::Integer(date_part(i, env, "YEAR", args)?.0))
}

fn month(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    ok(Value::Integer(i64::from(
        date_part(i, env, "MONTH", args)?.1,
    )))
}

fn day(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    ok(Value::Integer(i64::from(date_part(i, env, "DAY", args)?.2)))
}

fn hour(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    ok(Value::Integer(date_part(i, env, "HOUR", args)?.3 / 3600))
}

fn minute(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    ok(Value::Integer(
        (date_part(i, env, "MINUTE", args)?.3 / 60) % 60,
    ))
}

fn second(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    ok(Value::Integer(date_part(i, env, "SECOND", args)?.3 % 60))
}
