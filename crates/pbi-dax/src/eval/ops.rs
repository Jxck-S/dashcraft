//! Scalar operators and conversions with DAX semantics.
//!
//! BLANK handling follows Microsoft's "Blanks, empty strings, and zero
//! values" table:
//!
//! | Expression | Result |
//! |---|---|
//! | `BLANK + BLANK` | BLANK |
//! | `BLANK + 5`, `5 - BLANK` | 5 (BLANK acts as 0) |
//! | `BLANK - 5` | -5 |
//! | `BLANK * 5` | BLANK |
//! | `5 / BLANK`, `0 / BLANK` | Infinity, NaN (division by zero is not an error) |
//! | `BLANK / 5`, `BLANK / BLANK` | BLANK |
//! | `FALSE && BLANK`, `TRUE && BLANK` | FALSE |
//! | `TRUE \|\| BLANK` / `FALSE \|\| BLANK` | TRUE / FALSE |
//! | `BLANK && BLANK`, `BLANK \|\| BLANK` | BLANK |
//! | `BLANK & "a"` | `"a"` |
//!
//! Result types follow the implicit-conversion tables of the same article
//! (integer ∘ integer → integer, currency is kept where DAX keeps it, any
//! real → real, date ± number → date). Text operands of arithmetic are
//! parsed as numbers (`"22" + 22 = 44`); booleans count as 0/1.

use std::cmp::Ordering;
use std::sync::Arc;

use pbi_engine::value::{
    DECIMAL_FACTOR, MICROS_PER_DAY, OADATE_UNIX_EPOCH_DAYS, civil_from_days, days_from_civil,
};
use pbi_engine::{Value, ValueKey};

use super::error::{DaxError, Result, dax_type_name};
use crate::syntax::BinaryOp;

/// Numeric view of an arithmetic operand.
#[derive(Debug, Clone, Copy)]
enum Num {
    Int(i64),
    Dec(i128),
    Real(f64),
    Date(i64),
}

impl Num {
    fn f64(self) -> f64 {
        match self {
            Num::Int(i) => i as f64,
            Num::Dec(d) => d as f64 / DECIMAL_FACTOR as f64,
            Num::Real(r) => r,
            Num::Date(us) => serial_of_micros(us),
        }
    }

    fn dec(self) -> Option<i128> {
        match self {
            Num::Int(i) => i128::from(i).checked_mul(DECIMAL_FACTOR),
            Num::Dec(d) => Some(d),
            Num::Real(r) => real_to_dec(r),
            Num::Date(_) => None,
        }
    }

    fn is_zero(self) -> bool {
        self.f64() == 0.0
    }
}

fn real_to_dec(r: f64) -> Option<i128> {
    let scaled = (r * DECIMAL_FACTOR as f64).round();
    (scaled.is_finite() && scaled.abs() < 1.0e36).then_some(scaled as i128)
}

fn serial_of_micros(us: i64) -> f64 {
    us as f64 / MICROS_PER_DAY as f64 + OADATE_UNIX_EPOCH_DAYS
}

/// A date/time value from a DAX serial number (BLANK if out of range).
pub(crate) fn date_from_serial(serial: f64) -> Value {
    Value::from_oadate(serial).unwrap_or(Value::Blank)
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Blank => "Blank",
        Value::Boolean(_) => "True/False",
        Value::Integer(_) => "Integer",
        Value::Double(_) => "Number",
        Value::Decimal(_) => "Currency",
        Value::String(_) => "Text",
        Value::DateTime(_) => "Date",
    }
}

fn cannot_convert(v: &Value, target: &str) -> DaxError {
    DaxError::type_error(format!(
        "Cannot convert value '{}' of type {} to type {target}.",
        to_text(v),
        type_name(v)
    ))
}

/// Parses text as a number like DAX's implicit conversion (`"12"`,
/// `" 1.5 "`, `"1e3"`, `"-4"`). `None` if it is not a number.
pub(crate) fn parse_number(text: &str) -> Option<Value> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(i) = t.parse::<i64>() {
        return Some(Value::Integer(i));
    }
    let lower = t.to_ascii_lowercase();
    if lower.contains("inf") || lower.contains("nan") {
        return None;
    }
    t.parse::<f64>().ok().map(Value::Double)
}

fn num(v: &Value) -> Result<Option<Num>> {
    Ok(Some(match v {
        Value::Blank => return Ok(None),
        Value::Boolean(b) => Num::Int(i64::from(*b)),
        Value::Integer(i) => Num::Int(*i),
        Value::Double(d) => Num::Real(*d),
        Value::Decimal(d) => Num::Dec(*d),
        Value::DateTime(us) => Num::Date(*us),
        Value::String(s) => match parse_number(s) {
            Some(Value::Integer(i)) => Num::Int(i),
            Some(Value::Double(d)) => Num::Real(d),
            _ => return Err(cannot_convert(v, "Number")),
        },
    }))
}

/// A numeric value (BLANK stays BLANK; text parsed; booleans 0/1). Dates are
/// kept as dates.
pub(crate) fn to_numeric(v: &Value) -> Result<Value> {
    Ok(match num(v)? {
        None => Value::Blank,
        Some(n) => from_num(n),
    })
}

/// `f64` view of a numeric value (BLANK → `None`).
pub(crate) fn to_f64(v: &Value) -> Result<Option<f64>> {
    Ok(num(v)?.map(Num::f64))
}

/// Integer argument (rounded half away from zero); BLANK → `None`.
pub(crate) fn to_i64(v: &Value) -> Result<Option<i64>> {
    match to_f64(v)? {
        None => Ok(None),
        Some(f) if f.is_finite() && f.abs() < 9.0e18 => Ok(Some(f.round() as i64)),
        Some(_) => Err(cannot_convert(v, "Integer")),
    }
}

fn from_num(n: Num) -> Value {
    match n {
        Num::Int(i) => Value::Integer(i),
        Num::Dec(d) => Value::Decimal(d),
        Num::Real(r) => Value::Double(r),
        Num::Date(us) => Value::DateTime(us),
    }
}

fn zero_like(n: Num) -> Num {
    match n {
        Num::Int(_) | Num::Date(_) => Num::Int(0),
        Num::Dec(_) => Num::Dec(0),
        Num::Real(_) => Num::Real(0.0),
    }
}

fn date_plus_days(us: i64, days: f64) -> Value {
    let delta = (days * MICROS_PER_DAY as f64).round();
    if !delta.is_finite() || delta.abs() >= 9.0e18 {
        return Value::Double(serial_of_micros(us) + days);
    }
    match us.checked_add(delta as i64) {
        Some(v) => Value::DateTime(v),
        None => Value::Double(serial_of_micros(us) + days),
    }
}

/// `+`, `-`, `*`, `/`, `^` with DAX BLANK and type rules.
pub(crate) fn arith(op: BinaryOp, lhs: &Value, rhs: &Value) -> Result<Value> {
    let (l, r) = (num(lhs)?, num(rhs)?);
    match op {
        BinaryOp::Add | BinaryOp::Sub => {
            let (l, r) = match (l, r) {
                (None, None) => return Ok(Value::Blank),
                (Some(l), None) => (l, zero_like(l)),
                (None, Some(r)) => (zero_like(r), r),
                (Some(l), Some(r)) => (l, r),
            };
            Ok(add_sub(op == BinaryOp::Add, l, r))
        }
        BinaryOp::Mul => match (l, r) {
            (Some(l), Some(r)) => Ok(mul(l, r)),
            _ => Ok(Value::Blank),
        },
        BinaryOp::Div => match (l, r) {
            (None, _) => Ok(Value::Blank),
            (Some(l), None) => Ok(Value::Double(l.f64() / 0.0)),
            (Some(l), Some(r)) => Ok(div(l, r)),
        },
        BinaryOp::Pow => {
            let base = l.map_or(0.0, Num::f64);
            let exp = r.map_or(0.0, Num::f64);
            if l.is_none() && r.is_none() {
                return Ok(Value::Blank);
            }
            Ok(Value::Double(base.powf(exp)))
        }
        _ => Err(DaxError::evaluation(format!(
            "operator {} is not arithmetic",
            op.symbol()
        ))),
    }
}

fn add_sub(add: bool, l: Num, r: Num) -> Value {
    let sign = if add { 1.0 } else { -1.0 };
    match (l, r) {
        (Num::Date(a), Num::Date(b)) => {
            let serial = serial_of_micros(a) + sign * serial_of_micros(b);
            date_from_serial(serial)
        }
        (Num::Date(a), other) => date_plus_days(a, sign * other.f64()),
        (other, Num::Date(b)) => {
            if add {
                date_plus_days(b, other.f64())
            } else {
                Value::Double(other.f64() - serial_of_micros(b))
            }
        }
        (Num::Int(a), Num::Int(b)) => {
            let res = if add {
                a.checked_add(b)
            } else {
                a.checked_sub(b)
            };
            res.map_or_else(|| Value::Double(a as f64 + sign * b as f64), Value::Integer)
        }
        (Num::Real(_), _) | (_, Num::Real(_)) => Value::Double(l.f64() + sign * r.f64()),
        _ => match (l.dec(), r.dec()) {
            (Some(a), Some(b)) => {
                let res = if add {
                    a.checked_add(b)
                } else {
                    a.checked_sub(b)
                };
                res.map_or_else(|| Value::Double(l.f64() + sign * r.f64()), Value::Decimal)
            }
            _ => Value::Double(l.f64() + sign * r.f64()),
        },
    }
}

fn mul(l: Num, r: Num) -> Value {
    match (l, r) {
        (Num::Int(a), Num::Int(b)) => a
            .checked_mul(b)
            .map_or_else(|| Value::Double(a as f64 * b as f64), Value::Integer),
        (Num::Int(i), Num::Dec(d)) | (Num::Dec(d), Num::Int(i)) => d
            .checked_mul(i128::from(i))
            .map_or_else(|| Value::Double(l.f64() * r.f64()), Value::Decimal),
        (Num::Dec(_), Num::Real(_)) | (Num::Real(_), Num::Dec(_)) => {
            Value::decimal_from_f64(l.f64() * r.f64()).unwrap_or(Value::Double(l.f64() * r.f64()))
        }
        _ => Value::Double(l.f64() * r.f64()),
    }
}

fn div(l: Num, r: Num) -> Value {
    if r.is_zero() {
        return Value::Double(l.f64() / 0.0);
    }
    match (l, r) {
        (Num::Dec(_), Num::Int(_) | Num::Real(_)) | (Num::Int(_), Num::Dec(_)) => {
            let q = l.f64() / r.f64();
            Value::decimal_from_f64(q).unwrap_or(Value::Double(q))
        }
        _ => Value::Double(l.f64() / r.f64()),
    }
}

/// Unary minus (BLANK stays BLANK).
pub(crate) fn negate(v: &Value) -> Result<Value> {
    Ok(match num(v)? {
        None => Value::Blank,
        Some(Num::Int(i)) => i
            .checked_neg()
            .map_or(Value::Double(-(i as f64)), Value::Integer),
        Some(Num::Dec(d)) => d
            .checked_neg()
            .map_or(Value::Double(-(d as f64)), Value::Decimal),
        Some(Num::Real(r)) => Value::Double(-r),
        Some(Num::Date(us)) => date_from_serial(-serial_of_micros(us)),
    })
}

/// Comparison operators (`=`, `==`, `<>`, `<`, `>`, `<=`, `>=`).
pub(crate) fn compare(op: BinaryOp, lhs: &Value, rhs: &Value) -> Result<Value> {
    let b = match op {
        BinaryOp::Eq => lhs.dax_eq(rhs)?,
        BinaryOp::Neq => !lhs.dax_eq(rhs)?,
        BinaryOp::StrictEq => lhs.dax_strict_eq(rhs)?,
        BinaryOp::Lt => lhs.dax_cmp(rhs)? == Ordering::Less,
        BinaryOp::Gt => lhs.dax_cmp(rhs)? == Ordering::Greater,
        BinaryOp::Le => lhs.dax_cmp(rhs)? != Ordering::Greater,
        BinaryOp::Ge => lhs.dax_cmp(rhs)? != Ordering::Less,
        _ => {
            return Err(DaxError::evaluation(format!(
                "operator {} is not a comparison",
                op.symbol()
            )));
        }
    };
    Ok(Value::Boolean(b))
}

/// Three-valued truth: `None` for BLANK.
pub(crate) fn truth(v: &Value) -> Result<Option<bool>> {
    Ok(match v {
        Value::Blank => None,
        Value::Boolean(b) => Some(*b),
        Value::Integer(i) => Some(*i != 0),
        Value::Double(d) => Some(*d != 0.0),
        Value::Decimal(d) => Some(*d != 0),
        Value::DateTime(us) => Some(serial_of_micros(*us) != 0.0),
        Value::String(s) => {
            let t = s.trim();
            if t.eq_ignore_ascii_case("true") {
                Some(true)
            } else if t.eq_ignore_ascii_case("false") {
                Some(false)
            } else {
                return Err(cannot_convert(v, "True/False"));
            }
        }
    })
}

/// Condition truth (`IF`, `FILTER`): BLANK is false.
pub(crate) fn is_true(v: &Value) -> Result<bool> {
    Ok(truth(v)?.unwrap_or(false))
}

/// `&&` / `AND` over already-evaluated operands.
pub(crate) fn and(l: Option<bool>, r: Option<bool>) -> Value {
    match (l, r) {
        (None, None) => Value::Blank,
        (Some(true), Some(true)) => Value::Boolean(true),
        _ => Value::Boolean(false),
    }
}

/// `||` / `OR` over already-evaluated operands.
pub(crate) fn or(l: Option<bool>, r: Option<bool>) -> Value {
    match (l, r) {
        (None, None) => Value::Blank,
        (Some(true), _) | (_, Some(true)) => Value::Boolean(true),
        _ => Value::Boolean(false),
    }
}

/// `&` concatenation.
pub(crate) fn concat(l: &Value, r: &Value) -> Value {
    let mut s = to_text(l);
    s.push_str(&to_text(r));
    Value::String(Arc::from(s))
}

/// DAX implicit conversion to text: BLANK → `""`, numbers with up to 15
/// significant digits, dates in the invariant `M/D/YYYY[ h:mm:ss AM]` form,
/// booleans `TRUE`/`FALSE`.
pub(crate) fn to_text(v: &Value) -> String {
    match v {
        Value::Blank => String::new(),
        Value::Boolean(true) => "TRUE".to_owned(),
        Value::Boolean(false) => "FALSE".to_owned(),
        Value::Integer(i) => i.to_string(),
        Value::Double(d) => format_double(*d),
        Value::Decimal(_) => v.to_string(),
        Value::String(s) => s.to_string(),
        Value::DateTime(us) => format_datetime(*us),
    }
}

/// Text for a `Double` with at most 15 significant digits.
pub(crate) fn format_double(d: f64) -> String {
    if d.is_nan() {
        return "NaN".to_owned();
    }
    if d.is_infinite() {
        return if d > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    if d == 0.0 {
        return "0".to_owned();
    }
    let sci = format!("{d:.14e}");
    let Some((mantissa, exp)) = sci.split_once('e') else {
        return d.to_string();
    };
    let exp: i32 = exp.parse().unwrap_or(0);
    let negative = mantissa.starts_with('-');
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let sign = if negative { "-" } else { "" };
    if !(-5..15).contains(&exp) {
        let (first, rest) = digits.split_at(1);
        let rest = if rest.is_empty() {
            String::new()
        } else {
            format!(".{rest}")
        };
        let esign = if exp < 0 { "-" } else { "+" };
        return format!("{sign}{first}{rest}E{esign}{:02}", exp.abs());
    }
    if exp < 0 {
        let zeros = "0".repeat(exp.unsigned_abs() as usize - 1);
        return format!("{sign}0.{zeros}{digits}");
    }
    let int_len = exp as usize + 1;
    if digits.len() <= int_len {
        format!("{sign}{digits}{}", "0".repeat(int_len - digits.len()))
    } else {
        let (int, frac) = digits.split_at(int_len);
        format!("{sign}{int}.{frac}")
    }
}

/// `M/D/YYYY` (midnight) or `M/D/YYYY h:mm:ss AM`.
pub(crate) fn format_datetime(us: i64) -> String {
    let days = us.div_euclid(MICROS_PER_DAY);
    let secs = us.rem_euclid(MICROS_PER_DAY) / 1_000_000;
    let (y, m, d) = civil_from_days(days);
    if secs == 0 {
        return format!("{m}/{d}/{y}");
    }
    let (h, min, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    let (h12, ampm) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    format!("{m}/{d}/{y} {h12}:{min:02}:{s:02} {ampm}")
}

/// Parses date/time text: `YYYY-MM-DD`, `YYYY-MM-DD[T ]hh:mm[:ss]`,
/// `M/D/YYYY[ hh:mm[:ss]]`. `None` if not a date.
pub(crate) fn parse_datetime(text: &str) -> Option<Value> {
    let t = text.trim();
    let (date, time) = match t.find(['T', ' ']) {
        Some(i) => (t.get(..i)?, t.get(i + 1..)?.trim()),
        None => (t, ""),
    };
    let (y, m, d) = if date.contains('-') {
        let mut it = date.split('-');
        let y: i64 = it.next()?.parse().ok()?;
        let m: u32 = it.next()?.parse().ok()?;
        let d: u32 = it.next()?.parse().ok()?;
        if it.next().is_some() {
            return None;
        }
        (y, m, d)
    } else if date.contains('/') {
        let mut it = date.split('/');
        let m: u32 = it.next()?.parse().ok()?;
        let d: u32 = it.next()?.parse().ok()?;
        let y: i64 = it.next()?.parse().ok()?;
        if it.next().is_some() {
            return None;
        }
        (y, m, d)
    } else {
        return None;
    };
    let (mut hh, mut mm, mut ss) = (0u32, 0u32, 0u32);
    if !time.is_empty() {
        let upper = time.to_ascii_uppercase();
        let (clock, pm, am) = if let Some(c) = upper.strip_suffix("PM") {
            (c.trim().to_owned(), true, false)
        } else if let Some(c) = upper.strip_suffix("AM") {
            (c.trim().to_owned(), false, true)
        } else {
            (upper.clone(), false, false)
        };
        let clock = clock.trim_end_matches('Z');
        let mut it = clock.split(':');
        hh = it.next()?.trim().parse().ok()?;
        mm = it.next().map_or(Some(0), |s| s.trim().parse().ok())?;
        ss = it
            .next()
            .map_or(Some(0.0), |s| s.trim().parse::<f64>().ok())?
            .floor() as u32;
        if pm && hh < 12 {
            hh += 12;
        }
        if am && hh == 12 {
            hh = 0;
        }
    }
    Value::datetime(y, m, d, hh, mm, ss)
}

/// A date/time from a value: dates as is, numbers as serial dates, text
/// parsed. BLANK → `None`.
pub(crate) fn to_datetime(v: &Value) -> Result<Option<i64>> {
    Ok(match v {
        Value::Blank => None,
        Value::DateTime(us) => Some(*us),
        Value::String(s) => match parse_datetime(s) {
            Some(Value::DateTime(us)) => Some(us),
            _ => return Err(cannot_convert(v, "Date")),
        },
        Value::Boolean(_) => return Err(cannot_convert(v, "Date")),
        other => match to_f64(other)?.map(date_from_serial) {
            Some(Value::DateTime(us)) => Some(us),
            _ => return Err(cannot_convert(v, "Date")),
        },
    })
}

/// `DATE(year, month, day)` with DAX overflow rules (months and days roll
/// over, years 0–1899 are added to 1900).
pub(crate) fn make_date(year: i64, month: i64, day: i64) -> Result<Value> {
    let year = if (0..1900).contains(&year) {
        year + 1900
    } else {
        year
    };
    if !(0..=9999).contains(&year) {
        return Err(DaxError::argument(
            "DATE",
            format!("the year {year} is out of range"),
        ));
    }
    let months = year
        .checked_mul(12)
        .and_then(|m| m.checked_add(month - 1))
        .ok_or_else(|| DaxError::argument("DATE", "month out of range"))?;
    let (y, m) = (months.div_euclid(12), months.rem_euclid(12) + 1);
    let first = days_from_civil(y, m as u32, 1)
        .ok_or_else(|| DaxError::argument("DATE", "date out of range"))?;
    let days = first
        .checked_add(day - 1)
        .ok_or_else(|| DaxError::argument("DATE", "day out of range"))?;
    days.checked_mul(MICROS_PER_DAY)
        .map(Value::DateTime)
        .ok_or_else(|| DaxError::argument("DATE", "date out of range"))
}

/// `(year, month, day, seconds of day)` of a date/time.
pub(crate) fn civil(us: i64) -> (i64, u32, u32, i64) {
    let days = us.div_euclid(MICROS_PER_DAY);
    let (y, m, d) = civil_from_days(days);
    (y, m, d, us.rem_euclid(MICROS_PER_DAY) / 1_000_000)
}

/// Strict (`==`/`IN`) equality key: text case-insensitive, BLANK only
/// equals BLANK, numbers compare by value.
pub(crate) fn key(v: &Value) -> ValueKey {
    ValueKey::from(v)
}

/// Error for a value of the wrong type in a function.
pub(crate) fn wrong_type(function: &str, v: &Value) -> DaxError {
    DaxError::argument(
        function,
        format!(
            "cannot work with values of type {}",
            dax_type_name(v.type_name())
        ),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn a(op: BinaryOp, l: Value, r: Value) -> Value {
        arith(op, &l, &r).unwrap()
    }

    #[test]
    fn blank_arithmetic_follows_the_microsoft_table() {
        use BinaryOp::*;
        assert_eq!(a(Add, Value::Blank, Value::Blank), Value::Blank);
        assert_eq!(a(Add, Value::Blank, 5.into()), Value::Integer(5));
        assert_eq!(a(Sub, Value::Blank, 5.into()), Value::Integer(-5));
        assert_eq!(a(Sub, 5.into(), Value::Blank), Value::Integer(5));
        assert_eq!(a(Mul, Value::Blank, 5.into()), Value::Blank);
        assert_eq!(a(Div, 5.into(), Value::Blank), Value::Double(f64::INFINITY));
        let nan = a(Div, 0.into(), Value::Blank);
        assert!(matches!(nan, Value::Double(d) if d.is_nan()));
        assert_eq!(a(Div, Value::Blank, Value::Blank), Value::Blank);
        assert_eq!(a(Div, Value::Blank, 5.into()), Value::Blank);
        assert_eq!(and(Some(false), None), Value::Boolean(false));
        assert_eq!(and(Some(true), None), Value::Boolean(false));
        assert_eq!(or(Some(true), None), Value::Boolean(true));
        assert_eq!(or(Some(false), None), Value::Boolean(false));
        assert_eq!(and(None, None), Value::Blank);
        assert_eq!(or(None, None), Value::Blank);
        assert_eq!(concat(&Value::Blank, &"a".into()), Value::from("a"));
    }

    #[test]
    fn arithmetic_types_and_conversions() {
        use BinaryOp::*;
        assert_eq!(a(Add, 1.into(), 2.into()), Value::Integer(3));
        assert_eq!(a(Div, 1.into(), 2.into()), Value::Double(0.5));
        assert_eq!(a(Add, "22".into(), 22.into()), Value::Integer(44));
        assert_eq!(a(Add, true.into(), 1.into()), Value::Integer(2));
        assert_eq!(
            a(Add, Value::Decimal(15_000), 1.into()),
            Value::Decimal(25_000)
        );
        assert_eq!(a(Mul, 1.5.into(), 2.into()), Value::Double(3.0));
        assert_eq!(
            a(Add, Value::Integer(i64::MAX), 1.into()),
            Value::Double(i64::MAX as f64 + 1.0)
        );
        assert!(arith(Add, &"abc".into(), &1.into()).is_err());
        let d = Value::date(2024, 1, 31).unwrap();
        assert_eq!(a(Add, d, 1.into()), Value::date(2024, 2, 1).unwrap());
        assert_eq!(negate(&Value::Blank).unwrap(), Value::Blank);
        assert_eq!(negate(&3.into()).unwrap(), Value::Integer(-3));
    }

    #[test]
    fn comparisons_follow_dax() {
        let c = |op, l: Value, r: Value| compare(op, &l, &r).unwrap();
        assert_eq!(c(BinaryOp::Eq, Value::Blank, 0.into()), true.into());
        assert_eq!(c(BinaryOp::StrictEq, Value::Blank, 0.into()), false.into());
        assert_eq!(c(BinaryOp::Eq, "abc".into(), "ABC".into()), true.into());
        assert_eq!(c(BinaryOp::Neq, Value::Blank, "None".into()), true.into());
        assert_eq!(c(BinaryOp::Lt, 1.into(), 1.5.into()), true.into());
        let err = compare(BinaryOp::Eq, &"1".into(), &1.into()).unwrap_err();
        assert!(err.to_string().contains("Text"), "{err}");
    }

    #[test]
    fn text_conversion() {
        assert_eq!(to_text(&Value::Double(0.1 + 0.2)), "0.3");
        assert_eq!(to_text(&Value::Double(1.0 / 3.0)), "0.333333333333333");
        assert_eq!(to_text(&Value::Double(2.5)), "2.5");
        assert_eq!(to_text(&Value::Double(-1250.0)), "-1250");
        assert_eq!(to_text(&Value::Double(1e20)), "1E+20");
        assert_eq!(to_text(&Value::Double(0.00012)), "0.00012");
        assert_eq!(to_text(&Value::Boolean(true)), "TRUE");
        assert_eq!(to_text(&Value::date(2024, 1, 2).unwrap()), "1/2/2024");
        assert_eq!(
            to_text(&Value::datetime(2024, 1, 2, 13, 5, 0).unwrap()),
            "1/2/2024 1:05:00 PM"
        );
    }

    #[test]
    fn dates() {
        assert_eq!(
            make_date(2024, 13, 1).unwrap(),
            Value::date(2025, 1, 1).unwrap()
        );
        assert_eq!(
            make_date(2024, 3, 0).unwrap(),
            Value::date(2024, 2, 29).unwrap()
        );
        assert_eq!(
            make_date(24, 1, 1).unwrap(),
            Value::date(1924, 1, 1).unwrap()
        );
        assert_eq!(
            parse_datetime("2026-10-21 08:30:00"),
            Value::datetime(2026, 10, 21, 8, 30, 0)
        );
        assert_eq!(parse_datetime("1/2/2024"), Value::date(2024, 1, 2));
        assert_eq!(parse_datetime("nope"), None);
    }
}
