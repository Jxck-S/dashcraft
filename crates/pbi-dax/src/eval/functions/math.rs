//! Math and trigonometry (`LN`, `LOG`, `EXP`, `SIN` … `ACOTH`, `FACT`,
//! `COMBIN`, `GCD`, `MROUND`, `CEILING`, `RAND` …) and the bit functions
//! (`BITAND` … `BITRSHIFT`).
//!
//! Conventions (as in Power BI):
//!
//! - Arguments are converted to numbers like DAX's implicit conversion
//!   (booleans 0/1, numeric text parsed, other text is a type error);
//!   BLANK counts as 0, except for the rounding-to-a-multiple functions
//!   (`MROUND`, `CEILING`, `FLOOR`, `ISO.CEILING`) whose BLANK number gives
//!   BLANK, like `ROUND`.
//! - An argument outside the function's domain (`LN(0)`, `ASIN(2)`,
//!   `FACT(-1)` …) is an error, and so is a result that overflows or is not
//!   a number (`EXP(1000)`): "the result is too large or too small". The
//!   functions never return `NaN` or infinities (unlike `/`).
//! - Results are `Double` unless the reference documents a whole number
//!   (`COMBIN`, `COMBINA`, `PERMUT`, `GCD`, `LCM`, `QUOTIENT`, `EVEN`,
//!   `ODD`, `RANDBETWEEN`, the bit functions), which are `Integer`
//!   (`Double` past the 64-bit range). `CEILING` takes the type of its
//!   significance (currency numbers stay currency), `ISO.CEILING` the type
//!   of its number.
//! - `RAND`/`RANDBETWEEN` draw from a SplitMix64 generator seeded with a
//!   fixed value for every evaluation (one query, measure or calculated
//!   column): different calls and rows get different numbers, and the
//!   same evaluation repeated gives the same numbers (deterministic
//!   output, as the renderer requires).

use pbi_engine::Value;

use super::{Builtin, arg, arity, opt_arg, scalar_fn};
use crate::eval::context::Env;
use crate::eval::error::{DaxError, Result};
use crate::eval::interp::{Interp, Val};
use crate::eval::ops;
use crate::syntax::Expr;

pub(super) const BUILTINS: &[Builtin] = &[
    scalar_fn("PI", pi),
    scalar_fn("SQRTPI", sqrtpi),
    scalar_fn("EXP", exp),
    scalar_fn("LN", ln),
    scalar_fn("LOG", log),
    scalar_fn("LOG10", log10),
    scalar_fn("SIN", sin),
    scalar_fn("COS", cos),
    scalar_fn("TAN", tan),
    scalar_fn("COT", cot),
    scalar_fn("ASIN", asin),
    scalar_fn("ACOS", acos),
    scalar_fn("ATAN", atan),
    scalar_fn("ACOT", acot),
    scalar_fn("SINH", sinh),
    scalar_fn("COSH", cosh),
    scalar_fn("TANH", tanh),
    scalar_fn("COTH", coth),
    scalar_fn("ASINH", asinh),
    scalar_fn("ACOSH", acosh),
    scalar_fn("ATANH", atanh),
    scalar_fn("ACOTH", acoth),
    scalar_fn("DEGREES", degrees),
    scalar_fn("RADIANS", radians),
    scalar_fn("FACT", fact),
    scalar_fn("COMBIN", combin),
    scalar_fn("COMBINA", combina),
    scalar_fn("PERMUT", permut),
    scalar_fn("GCD", gcd),
    scalar_fn("LCM", lcm),
    scalar_fn("QUOTIENT", quotient),
    scalar_fn("EVEN", even),
    scalar_fn("ODD", odd),
    scalar_fn("MROUND", mround),
    scalar_fn("CEILING", ceiling),
    scalar_fn("ISO.CEILING", iso_ceiling),
    scalar_fn("FLOOR", floor),
    scalar_fn("RAND", rand),
    scalar_fn("RANDBETWEEN", randbetween),
    scalar_fn("BITAND", bitand),
    scalar_fn("BITOR", bitor),
    scalar_fn("BITXOR", bitxor),
    scalar_fn("BITLSHIFT", bitlshift),
    scalar_fn("BITRSHIFT", bitrshift),
];

/// Seed of the per-evaluation `RAND` generator.
pub(crate) const RAND_SEED: u64 = 0x0D45_C2AF_7A11_E5ED;

/// Largest integer a double holds exactly (2^53), the limit of `GCD`/`LCM`.
const MAX_EXACT: f64 = 9_007_199_254_740_992.0;

pub(super) type R = Result<Val>;

pub(super) fn ok(v: Value) -> R {
    Ok(Val::Scalar(v))
}

// ---- shared numeric helpers -------------------------------------------------

/// "An argument has an invalid value" error with a reason.
pub(super) fn invalid(f: &str, detail: &str) -> DaxError {
    DaxError::argument(f, format!("an argument has an invalid value ({detail})"))
}

/// Overflow / not-a-number result error.
pub(super) fn too_large(f: &str) -> DaxError {
    DaxError::argument(f, "the result is too large or too small")
}

/// A finite `Double` result, or the overflow error.
pub(super) fn real(f: &str, x: f64) -> R {
    if x.is_finite() {
        ok(Value::Double(if x == 0.0 { 0.0 } else { x }))
    } else {
        Err(too_large(f))
    }
}

/// Numeric value of argument `n` (BLANK → 0).
pub(super) fn num(i: &Interp<'_>, env: &Env, f: &str, args: &[Expr], n: usize) -> Result<f64> {
    Ok(ops::to_f64(&i.scalar(env, arg(f, args, n)?)?)?.unwrap_or(0.0))
}

/// Numeric value of optional argument `n`; `default` when omitted or BLANK.
pub(super) fn num_or(
    i: &Interp<'_>,
    env: &Env,
    args: &[Expr],
    n: usize,
    default: f64,
) -> Result<f64> {
    match opt_arg(args, n) {
        Some(e) => Ok(ops::to_f64(&i.scalar(env, e)?)?.unwrap_or(default)),
        None => Ok(default),
    }
}

/// Truth value of argument `n` (BLANK → FALSE).
pub(super) fn flag(i: &Interp<'_>, env: &Env, f: &str, args: &[Expr], n: usize) -> Result<bool> {
    ops::is_true(&i.scalar(env, arg(f, args, n)?)?)
}

/// Rounds to 15 significant digits, removing binary artefacts such as
/// `0.3 / 0.1 = 2.9999999999999996` before rounding to a multiple.
fn sig15(x: f64) -> f64 {
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    format!("{x:.14e}").parse::<f64>().unwrap_or(x)
}

/// An integer result: `Integer` when it fits, `Double` otherwise (error
/// when not finite).
fn whole(f: &str, x: f64) -> R {
    if !x.is_finite() {
        return Err(too_large(f));
    }
    if x.abs() < 9.0e18 {
        ok(Value::Integer(x as i64))
    } else {
        real(f, x)
    }
}

fn unary(
    i: &Interp<'_>,
    env: &Env,
    args: &[Expr],
    f: &str,
    op: fn(f64) -> std::result::Result<f64, &'static str>,
) -> R {
    arity(f, args, 1, 1)?;
    let x = num(i, env, f, args, 0)?;
    match op(x) {
        Ok(y) => real(f, y),
        Err(detail) => Err(invalid(f, detail)),
    }
}

// ---- constants, exponentials and logarithms ----------------------------------

fn pi(_: &Interp<'_>, _: &Env, args: &[Expr]) -> R {
    arity("PI", args, 0, 0)?;
    real("PI", std::f64::consts::PI)
}

fn sqrtpi(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    unary(i, env, args, "SQRTPI", |x| {
        if x < 0.0 {
            Err("the number cannot be negative")
        } else {
            Ok((x * std::f64::consts::PI).sqrt())
        }
    })
}

fn exp(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    unary(i, env, args, "EXP", |x| Ok(x.exp()))
}

fn positive(x: f64) -> std::result::Result<f64, &'static str> {
    if x > 0.0 {
        Ok(x)
    } else {
        Err("the number must be greater than 0")
    }
}

fn ln(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    unary(i, env, args, "LN", |x| positive(x).map(f64::ln))
}

fn log10(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    unary(i, env, args, "LOG10", |x| positive(x).map(f64::log10))
}

/// `LOG(number[, base = 10])`.
fn log(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("LOG", args, 1, 2)?;
    let x = num(i, env, "LOG", args, 0)?;
    let base = num_or(i, env, args, 1, 10.0)?;
    positive(x).map_err(|d| invalid("LOG", d))?;
    if base <= 0.0 || base == 1.0 || base.is_nan() {
        return Err(invalid(
            "LOG",
            "the base must be greater than 0 and different from 1",
        ));
    }
    let y = if base == 10.0 {
        x.log10()
    } else if base == 2.0 {
        x.log2()
    } else {
        x.ln() / base.ln()
    };
    real("LOG", y)
}

// ---- trigonometry ----------------------------------------------------------------

fn sin(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    unary(i, env, args, "SIN", |x| Ok(x.sin()))
}

fn cos(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    unary(i, env, args, "COS", |x| Ok(x.cos()))
}

fn tan(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    unary(i, env, args, "TAN", |x| Ok(x.tan()))
}

/// Limit of `|x|` for `COT` (as in the reference).
const COT_LIMIT: f64 = 134_217_728.0;

fn cot(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    unary(i, env, args, "COT", |x| {
        if x == 0.0 {
            Err("the number cannot be 0")
        } else if x.abs() >= COT_LIMIT {
            Err("the absolute value must be less than 2^27")
        } else {
            Ok(1.0 / x.tan())
        }
    })
}

fn unit_range(x: f64) -> std::result::Result<f64, &'static str> {
    if (-1.0..=1.0).contains(&x) {
        Ok(x)
    } else {
        Err("the number must be between -1 and 1")
    }
}

fn asin(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    unary(i, env, args, "ASIN", |x| unit_range(x).map(f64::asin))
}

fn acos(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    unary(i, env, args, "ACOS", |x| unit_range(x).map(f64::acos))
}

fn atan(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    unary(i, env, args, "ATAN", |x| Ok(x.atan()))
}

/// Arccotangent in `(0, π)`.
fn acot(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    unary(i, env, args, "ACOT", |x| {
        Ok(std::f64::consts::FRAC_PI_2 - x.atan())
    })
}

fn sinh(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    unary(i, env, args, "SINH", |x| Ok(x.sinh()))
}

fn cosh(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    unary(i, env, args, "COSH", |x| Ok(x.cosh()))
}

fn tanh(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    unary(i, env, args, "TANH", |x| Ok(x.tanh()))
}

fn coth(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    unary(i, env, args, "COTH", |x| {
        if x == 0.0 {
            Err("the number cannot be 0")
        } else {
            Ok(1.0 / x.tanh())
        }
    })
}

fn asinh(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    unary(i, env, args, "ASINH", |x| Ok(x.asinh()))
}

fn acosh(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    unary(i, env, args, "ACOSH", |x| {
        if x >= 1.0 {
            Ok(x.acosh())
        } else {
            Err("the number must be at least 1")
        }
    })
}

fn atanh(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    unary(i, env, args, "ATANH", |x| {
        if x > -1.0 && x < 1.0 {
            Ok(x.atanh())
        } else {
            Err("the number must be strictly between -1 and 1")
        }
    })
}

fn acoth(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    unary(i, env, args, "ACOTH", |x| {
        if x.abs() > 1.0 {
            Ok((1.0 / x).atanh())
        } else {
            Err("the absolute value must be greater than 1")
        }
    })
}

fn degrees(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    unary(i, env, args, "DEGREES", |x| Ok(x.to_degrees()))
}

fn radians(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    unary(i, env, args, "RADIANS", |x| Ok(x.to_radians()))
}

// ---- factorials, combinations and integers ----------------------------------------

/// Truncated integer argument; errors when not finite.
fn int_arg(i: &Interp<'_>, env: &Env, f: &str, args: &[Expr], n: usize) -> Result<f64> {
    let x = num(i, env, f, args, n)?.trunc();
    if x.is_finite() {
        Ok(x)
    } else {
        Err(invalid(f, "the number is not finite"))
    }
}

/// `FACT(number)`: `number` truncated, 0 to 170.
fn fact(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("FACT", args, 1, 1)?;
    let n = int_arg(i, env, "FACT", args, 0)?;
    if n < 0.0 {
        return Err(invalid("FACT", "the number cannot be negative"));
    }
    if n > 170.0 {
        return Err(too_large("FACT"));
    }
    let mut p = 1.0f64;
    let mut k = 2.0;
    while k <= n {
        p *= k;
        k += 1.0;
    }
    real("FACT", p)
}

/// `C(n, k)` as a double (`None` on overflow); `0 ≤ k ≤ n`.
fn choose(n: f64, k: f64) -> Option<f64> {
    let k = k.min(n - k);
    // C(2k, k) overflows a double well before k = 1100.
    if k > 1_100.0 {
        return None;
    }
    let mut c = 1.0f64;
    let mut i = 1.0;
    while i <= k {
        c = c * (n - k + i) / i;
        if !c.is_finite() {
            return None;
        }
        i += 1.0;
    }
    Some(c.round())
}

fn pair(i: &Interp<'_>, env: &Env, f: &str, args: &[Expr]) -> Result<(f64, f64)> {
    arity(f, args, 2, 2)?;
    Ok((int_arg(i, env, f, args, 0)?, int_arg(i, env, f, args, 1)?))
}

fn combin(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    let (n, k) = pair(i, env, "COMBIN", args)?;
    if n < 0.0 || k < 0.0 || n < k {
        return Err(invalid(
            "COMBIN",
            "the numbers must satisfy 0 <= number_chosen <= number",
        ));
    }
    match choose(n, k) {
        Some(c) => whole("COMBIN", c),
        None => Err(too_large("COMBIN")),
    }
}

/// Combinations with repetitions: `C(n + k - 1, k)`.
fn combina(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    let (n, k) = pair(i, env, "COMBINA", args)?;
    if n < 0.0 || k < 0.0 || n < k {
        return Err(invalid(
            "COMBINA",
            "the numbers must satisfy 0 <= number_chosen <= number",
        ));
    }
    if k == 0.0 {
        return whole("COMBINA", 1.0);
    }
    match choose(n + k - 1.0, k) {
        Some(c) => whole("COMBINA", c),
        None => Err(too_large("COMBINA")),
    }
}

/// `n! / (n - k)!`.
fn permut(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    let (n, k) = pair(i, env, "PERMUT", args)?;
    if n < 0.0 || k < 0.0 || n < k {
        return Err(invalid(
            "PERMUT",
            "the numbers must satisfy 0 <= number_chosen <= number",
        ));
    }
    let mut p = 1.0f64;
    let mut j = 0.0;
    while j < k {
        p *= n - j;
        if !p.is_finite() {
            return Err(too_large("PERMUT"));
        }
        j += 1.0;
    }
    whole("PERMUT", p)
}

/// Two non-negative integers below 2^53 (`GCD`/`LCM`).
fn gcd_args(i: &Interp<'_>, env: &Env, f: &str, args: &[Expr]) -> Result<(u64, u64)> {
    let (a, b) = pair(i, env, f, args)?;
    if a < 0.0 || b < 0.0 {
        return Err(invalid(f, "the numbers cannot be negative"));
    }
    if a >= MAX_EXACT || b >= MAX_EXACT {
        return Err(invalid(f, "the numbers must be less than 2^53"));
    }
    Ok((a as u64, b as u64))
}

fn euclid(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a
}

fn gcd(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    let (a, b) = gcd_args(i, env, "GCD", args)?;
    ok(Value::Integer(i64::try_from(euclid(a, b)).unwrap_or(0)))
}

fn lcm(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    let (a, b) = gcd_args(i, env, "LCM", args)?;
    if a == 0 || b == 0 {
        return ok(Value::Integer(0));
    }
    let g = euclid(a, b).max(1);
    let l = u128::from(a / g) * u128::from(b);
    if l >= MAX_EXACT as u128 {
        return Err(too_large("LCM"));
    }
    ok(Value::Integer(i64::try_from(l).unwrap_or(0)))
}

/// `QUOTIENT(numerator, denominator)`: the integer part of the division.
fn quotient(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("QUOTIENT", args, 2, 2)?;
    let n = ops::to_numeric(&i.scalar(env, arg("QUOTIENT", args, 0)?)?)?;
    let d = ops::to_numeric(&i.scalar(env, arg("QUOTIENT", args, 1)?)?)?;
    let df = ops::to_f64(&d)?.unwrap_or(0.0);
    if df == 0.0 {
        return Err(invalid("QUOTIENT", "division by zero"));
    }
    let int_n = match n {
        Value::Integer(a) => Some(a),
        Value::Blank => Some(0),
        _ => None,
    };
    if let (Some(a), Value::Integer(b)) = (int_n, &d)
        && let Some(q) = a.checked_div(*b)
    {
        return ok(Value::Integer(q));
    }
    let nf = ops::to_f64(&n)?.unwrap_or(0.0);
    whole("QUOTIENT", (nf / df).trunc())
}

/// Rounds away from zero to the next integer with the wanted parity.
fn parity_round(f: &str, x: f64, odd: bool) -> R {
    let m = x.abs().ceil();
    let is_odd = m % 2.0 == 1.0;
    let m = if is_odd == odd { m } else { m + 1.0 };
    whole(f, if x < 0.0 { -m } else { m })
}

fn even(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("EVEN", args, 1, 1)?;
    let x = num(i, env, "EVEN", args, 0)?;
    parity_round("EVEN", x, false)
}

fn odd(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("ODD", args, 1, 1)?;
    let x = num(i, env, "ODD", args, 0)?;
    parity_round("ODD", x, true)
}

// ---- rounding to a multiple ----------------------------------------------------

/// Result type of the multiple-rounding functions.
#[derive(Clone, Copy)]
enum Kind {
    Int,
    Dec,
    Real,
}

fn typed(f: &str, x: f64, kind: Kind) -> R {
    let x = sig15(x);
    match kind {
        Kind::Int => whole(f, x),
        Kind::Dec => match Value::decimal_from_f64(x) {
            Some(v) => ok(v),
            None => real(f, x),
        },
        Kind::Real => real(f, x),
    }
}

/// `number / multiple` cleaned of binary artefacts.
fn ratio(n: f64, m: f64) -> f64 {
    sig15(n / m)
}

/// `MROUND(number, multiple)`: nearest multiple, halves away from zero.
fn mround(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("MROUND", args, 2, 2)?;
    let v = i.scalar(env, arg("MROUND", args, 0)?)?;
    let Some(n) = ops::to_f64(&v)? else {
        return ok(Value::Blank);
    };
    let m = num(i, env, "MROUND", args, 1)?;
    if (n > 0.0 && m < 0.0) || (n < 0.0 && m > 0.0) {
        return Err(invalid(
            "MROUND",
            "the number and the multiple must have the same sign",
        ));
    }
    if m == 0.0 || n == 0.0 {
        return real("MROUND", 0.0);
    }
    typed("MROUND", ratio(n, m).round() * m, Kind::Real)
}

/// `CEILING(number, significance)`: the result has the type of the
/// significance (Integer for TRUE/FALSE), or Currency for a currency number.
fn ceiling(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("CEILING", args, 2, 2)?;
    let v = i.scalar(env, arg("CEILING", args, 0)?)?;
    let Some(n) = ops::to_f64(&v)? else {
        return ok(Value::Blank);
    };
    let sv = i.scalar(env, arg("CEILING", args, 1)?)?;
    let s = ops::to_f64(&sv)?.unwrap_or(0.0);
    let kind = match (&v, &sv) {
        (Value::Decimal(_), _) | (_, Value::Decimal(_)) => Kind::Dec,
        (_, Value::Integer(_) | Value::Boolean(_)) => Kind::Int,
        _ => Kind::Real,
    };
    if n > 0.0 && s < 0.0 {
        return Err(invalid(
            "CEILING",
            "a positive number cannot have a negative significance",
        ));
    }
    if s == 0.0 || n == 0.0 {
        return typed("CEILING", 0.0, kind);
    }
    typed("CEILING", ratio(n, s).ceil() * s, kind)
}

/// `ISO.CEILING(number[, significance = 1])`: always towards +∞, using the
/// absolute significance; the result has the type of the number.
fn iso_ceiling(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("ISO.CEILING", args, 1, 2)?;
    let v = i.scalar(env, arg("ISO.CEILING", args, 0)?)?;
    let Some(n) = ops::to_f64(&v)? else {
        return ok(Value::Blank);
    };
    let s = num_or(i, env, args, 1, 1.0)?.abs();
    let kind = match v {
        Value::Integer(_) | Value::Boolean(_) => Kind::Int,
        Value::Decimal(_) => Kind::Dec,
        _ => Kind::Real,
    };
    if s == 0.0 || n == 0.0 {
        return typed("ISO.CEILING", 0.0, kind);
    }
    typed("ISO.CEILING", ratio(n, s).ceil() * s, kind)
}

/// `FLOOR(number, significance)`: towards zero; both arguments must have
/// the same sign.
fn floor(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("FLOOR", args, 2, 2)?;
    let v = i.scalar(env, arg("FLOOR", args, 0)?)?;
    let Some(n) = ops::to_f64(&v)? else {
        return ok(Value::Blank);
    };
    let s = num(i, env, "FLOOR", args, 1)?;
    let kind = if matches!(v, Value::Decimal(_)) {
        Kind::Dec
    } else {
        Kind::Real
    };
    if n == 0.0 {
        return typed("FLOOR", 0.0, kind);
    }
    if s == 0.0 {
        return Err(invalid("FLOOR", "the significance cannot be 0"));
    }
    if (n > 0.0) != (s > 0.0) {
        return Err(invalid(
            "FLOOR",
            "the number and the significance must have the same sign",
        ));
    }
    typed("FLOOR", ratio(n, s).floor() * s, kind)
}

// ---- random numbers -------------------------------------------------------------

/// Next 64 random bits of the evaluation's generator (SplitMix64).
fn next_random(i: &Interp<'_>) -> u64 {
    let state = i.rand_state.get().wrapping_add(0x9E37_79B9_7F4A_7C15);
    i.rand_state.set(state);
    let mut z = state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// `RAND()`: uniform in `[0, 1)`.
fn rand(i: &Interp<'_>, _: &Env, args: &[Expr]) -> R {
    arity("RAND", args, 0, 0)?;
    let bits = next_random(i) >> 11;
    real("RAND", bits as f64 / (1u64 << 53) as f64)
}

/// `RANDBETWEEN(bottom, top)`: a whole number in `[⌈bottom⌉, ⌊top⌋]`.
fn randbetween(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("RANDBETWEEN", args, 2, 2)?;
    let lo = num(i, env, "RANDBETWEEN", args, 0)?.ceil();
    let hi = num(i, env, "RANDBETWEEN", args, 1)?.floor();
    if !(lo.abs() < 9.0e18 && hi.abs() < 9.0e18) {
        return Err(invalid("RANDBETWEEN", "the bounds are out of range"));
    }
    let (lo, hi) = (lo as i64, hi as i64);
    if lo > hi {
        return Err(invalid(
            "RANDBETWEEN",
            "bottom must not be greater than top",
        ));
    }
    let span = u128::try_from(i128::from(hi) - i128::from(lo) + 1).unwrap_or(1);
    let offset = (u128::from(next_random(i)) * span) >> 64;
    let value = i128::from(lo) + i128::try_from(offset).unwrap_or(0);
    ok(Value::Integer(i64::try_from(value).unwrap_or(lo)))
}

// ---- bit functions ----------------------------------------------------------------

/// A 64-bit integer argument (truncated; BLANK → 0).
fn bit_arg(i: &Interp<'_>, env: &Env, f: &str, args: &[Expr], n: usize) -> Result<i64> {
    let x = num(i, env, f, args, n)?.trunc();
    if x.abs() < 9.2e18 {
        Ok(x as i64)
    } else {
        Err(invalid(f, "the number must be a 64-bit integer"))
    }
}

fn bitwise(i: &Interp<'_>, env: &Env, args: &[Expr], f: &str, op: fn(i64, i64) -> i64) -> R {
    arity(f, args, 2, 2)?;
    let a = bit_arg(i, env, f, args, 0)?;
    let b = bit_arg(i, env, f, args, 1)?;
    ok(Value::Integer(op(a, b)))
}

fn bitand(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    bitwise(i, env, args, "BITAND", |a, b| a & b)
}

fn bitor(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    bitwise(i, env, args, "BITOR", |a, b| a | b)
}

fn bitxor(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    bitwise(i, env, args, "BITXOR", |a, b| a ^ b)
}

/// `n` shifted left by `s` bits (right when `s < 0`); bits shifted out are
/// lost, right shifts keep the sign.
fn shift_left(n: i64, s: i64) -> i64 {
    let amount = s.unsigned_abs();
    if s >= 0 {
        if amount >= 64 { 0 } else { n << amount }
    } else if amount >= 64 {
        if n < 0 { -1 } else { 0 }
    } else {
        n >> amount
    }
}

fn bitlshift(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    bitwise(i, env, args, "BITLSHIFT", shift_left)
}

fn bitrshift(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    bitwise(i, env, args, "BITRSHIFT", |n, s| {
        shift_left(n, s.checked_neg().unwrap_or(i64::MAX))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shifts_never_overflow() {
        assert_eq!(shift_left(2, 3), 16);
        assert_eq!(shift_left(128, -1), 64);
        assert_eq!(shift_left(1, 64), 0);
        assert_eq!(shift_left(-8, -100), -1);
        assert_eq!(shift_left(i64::MAX, i64::MIN), 0);
        assert_eq!(shift_left(1, 63), i64::MIN);
    }

    #[test]
    fn helpers() {
        assert_eq!(sig15(0.3 / 0.1), 3.0);
        assert_eq!(choose(5.0, 2.0), Some(10.0));
        assert_eq!(choose(1e9, 2_000.0), None);
        assert_eq!(euclid(24, 36), 12);
    }
}
