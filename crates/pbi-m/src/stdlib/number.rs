//! `Number.*`, `Int64.From` & co., and `Logical.*` functions.

use super::Builder;
use super::args::{arg, int_arg, num_arg, req_num, text_arg};
use super::convert::{apply_facet, parse_number, round_half_even, to_logical, to_number};
use crate::eval::interp::{Interp, convert_error};
use crate::merror::MError;
use crate::value::format::number_to_text;
use crate::value::{Facet, Value};

pub(super) fn register(b: &mut Builder) {
    b.func("Number.From", &["value", "?culture"], from);
    b.func("Number.FromText", &["text", "?culture"], from_text);
    b.func("Number.ToText", &["number", "?format", "?culture"], to_text);
    b.func(
        "Number.Round",
        &["number", "?digits", "?roundingMode"],
        round,
    );
    b.func("Number.RoundUp", &["number", "?digits"], round_up);
    b.func("Number.RoundDown", &["number", "?digits"], round_down);
    b.func(
        "Number.RoundTowardZero",
        &["number", "?digits"],
        round_toward_zero,
    );
    b.func(
        "Number.RoundAwayFromZero",
        &["number", "?digits"],
        round_away_from_zero,
    );
    b.func("Number.Abs", &["number"], abs);
    b.func("Number.Sign", &["number"], sign);
    b.func("Number.Mod", &["number", "divisor", "?precision"], modulo);
    b.func(
        "Number.IntegerDivide",
        &["number1", "number2", "?precision"],
        integer_divide,
    );
    b.func("Number.Power", &["number", "power"], power);
    b.func("Number.Sqrt", &["number"], sqrt);
    b.func("Number.Exp", &["number"], exp);
    b.func("Number.Ln", &["number"], ln);
    b.func("Number.Log10", &["number"], log10);
    b.func("Number.IsNaN", &["number"], is_nan);
    b.func("Number.IsEven", &["number"], is_even);
    b.func("Number.IsOdd", &["number"], is_odd);
    b.value("Number.PI", Value::Number(std::f64::consts::PI));
    b.value("Number.E", Value::Number(std::f64::consts::E));
    b.value("Number.Epsilon", Value::Number(f64::from_bits(1)));
    b.value("Number.MaxValue", Value::Number(f64::MAX));
    b.value("Number.MinValue", Value::Number(f64::MIN));
    b.value("Number.PositiveInfinity", Value::Number(f64::INFINITY));
    b.value("Number.NegativeInfinity", Value::Number(f64::NEG_INFINITY));
    b.value("Number.NaN", Value::Number(f64::NAN));
    b.func(
        "Int64.From",
        &["value", "?culture", "?roundingMode"],
        int64_from,
    );
    b.func(
        "Int32.From",
        &["value", "?culture", "?roundingMode"],
        int32_from,
    );
    b.func(
        "Int16.From",
        &["value", "?culture", "?roundingMode"],
        int16_from,
    );
    b.func(
        "Int8.From",
        &["value", "?culture", "?roundingMode"],
        int8_from,
    );
    b.func(
        "Byte.From",
        &["value", "?culture", "?roundingMode"],
        byte_from,
    );
    b.func("Decimal.From", &["value", "?culture"], from);
    b.func("Double.From", &["value", "?culture"], from);
    b.func("Single.From", &["value", "?culture"], from);
    b.func(
        "Currency.From",
        &["value", "?culture", "?roundingMode"],
        currency_from,
    );
    b.func("Percentage.From", &["value", "?culture"], from);
    b.func("Logical.From", &["value"], logical_from);
    b.func("Logical.FromText", &["text"], logical_from_text);
    b.func("Logical.ToText", &["logical"], logical_to_text);
    b.consts(&[
        ("RoundingMode.Up", 0.0),
        ("RoundingMode.Down", 1.0),
        ("RoundingMode.AwayFromZero", 2.0),
        ("RoundingMode.TowardZero", 3.0),
        ("RoundingMode.ToEven", 4.0),
    ]);
}

fn number_or_null(
    args: &[Value],
    f: impl FnOnce(f64) -> Result<Value, MError>,
) -> Result<Value, MError> {
    match num_arg(args, 0)? {
        None => Ok(Value::Null),
        Some(n) => f(n),
    }
}

fn from(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    to_number(arg(args, 0))
}

fn from_text(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    match text_arg(args, 0, "Number.FromText")? {
        None => Ok(Value::Null),
        Some(t) => parse_number(t)
            .map(Value::Number)
            .ok_or_else(|| MError::Evaluation {
                reason: "DataFormat.Error".into(),
                message: "We couldn't convert to Number.".into(),
                detail: Some(Box::new(Value::text(t))),
                location: None,
            }),
    }
}

fn group_thousands(int_part: &str) -> String {
    let (sign, digits) = match int_part.strip_prefix('-') {
        Some(d) => ("-", d),
        None => ("", int_part),
    };
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("{sign}{out}")
}

fn fixed(n: f64, decimals: usize, grouping: bool) -> String {
    let s = format!("{:.*}", decimals, round_half_away(n, decimals as i32));
    if !grouping {
        return s;
    }
    match s.split_once('.') {
        Some((i, f)) => format!("{}.{f}", group_thousands(i)),
        None => group_thousands(&s),
    }
}

fn round_half_away(n: f64, digits: i32) -> f64 {
    let factor = 10f64.powi(digits);
    let r = (n * factor).round() / factor;
    if r.is_finite() { r } else { n }
}

/// .NET-style numeric formats: `G`, `D[n]`, `F[n]`, `N[n]`, `P[n]`, `X`, and simple custom
/// patterns (`0.00`, `#,##0.##`, `000`).
pub(crate) fn format_number(n: f64, fmt: &str) -> Result<String, MError> {
    let precision = |rest: &str, default: usize| -> Result<usize, MError> {
        if rest.is_empty() {
            Ok(default)
        } else {
            rest.parse::<usize>()
                .ok()
                .filter(|p| *p <= 99)
                .ok_or_else(|| MError::expression(format!("The format '{fmt}' is not supported.")))
        }
    };
    let mut chars = fmt.chars();
    let first = chars.next().map(|c| c.to_ascii_uppercase());
    let rest = chars.as_str();
    let standard = rest.chars().all(|c| c.is_ascii_digit());
    match first {
        None => Ok(number_to_text(n)),
        Some('G') if standard => Ok(number_to_text(n)),
        Some('D') if standard => {
            if n.fract() != 0.0 {
                return Err(MError::expression("The 'D' format requires an integer."));
            }
            let width = precision(rest, 0)?;
            let i = n as i64;
            Ok(if i < 0 {
                format!("-{:0width$}", i.unsigned_abs())
            } else {
                format!("{i:0width$}")
            })
        }
        Some('F') if standard => Ok(fixed(n, precision(rest, 2)?, false)),
        Some('N') if standard => Ok(fixed(n, precision(rest, 2)?, true)),
        Some('P') if standard => Ok(format!("{}%", fixed(n * 100.0, precision(rest, 2)?, true))),
        Some('X') if standard && n.fract() == 0.0 && n >= 0.0 => {
            let width = precision(rest, 0)?;
            let s = format!("{:0width$X}", n as u64);
            Ok(if fmt.starts_with('x') {
                s.to_lowercase()
            } else {
                s
            })
        }
        _ => custom_format(n, fmt),
    }
}

fn custom_format(n: f64, fmt: &str) -> Result<String, MError> {
    let start = fmt
        .find(['0', '#'])
        .ok_or_else(|| MError::expression(format!("The format '{fmt}' is not supported.")))?;
    let end = fmt.rfind(['0', '#']).map_or(start, |e| e + 1);
    let (prefix, pattern, suffix) = (&fmt[..start], &fmt[start..end], &fmt[end..]);
    if pattern.chars().any(|c| !matches!(c, '0' | '#' | ',' | '.')) {
        return Err(MError::expression(format!(
            "The format '{fmt}' is not supported."
        )));
    }
    let percent = suffix.contains('%') || prefix.contains('%');
    let n = if percent { n * 100.0 } else { n };
    let (int_pat, frac_pat) = pattern.split_once('.').unwrap_or((pattern, ""));
    let min_dec = frac_pat.chars().filter(|&c| c == '0').count();
    let max_dec = frac_pat.chars().filter(|&c| c == '0' || c == '#').count();
    let int_min = int_pat.chars().filter(|&c| c == '0').count();
    let grouping = int_pat.contains(',');
    let s = format!("{:.*}", max_dec, round_half_away(n, max_dec as i32));
    let (int_part, frac_part) = s.split_once('.').unwrap_or((&s, ""));
    let mut frac = frac_part.to_owned();
    while frac.len() > min_dec && frac.ends_with('0') {
        frac.pop();
    }
    let (sign, digits) = match int_part.strip_prefix('-') {
        Some(d) => ("-", d),
        None => ("", int_part),
    };
    let digits = digits.trim_start_matches('0');
    let mut int_digits = format!("{digits:0>int_min$}");
    if grouping {
        int_digits = group_thousands(&int_digits);
    }
    let mut out = format!("{prefix}{sign}{int_digits}");
    if !frac.is_empty() {
        out.push('.');
        out.push_str(&frac);
    }
    out.push_str(suffix);
    Ok(out)
}

fn to_text(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let fmt = text_arg(args, 1, "Number.ToText")?.unwrap_or("");
    number_or_null(args, |n| Ok(Value::text(format_number(n, fmt)?)))
}

/// Rounds with an M `RoundingMode` (default `ToEven`).
pub(crate) fn round_with_mode(n: f64, digits: i32, mode: Option<f64>) -> Result<f64, MError> {
    let factor = 10f64.powi(digits);
    let scaled = n * factor;
    let r = match mode.map(|m| m as i64) {
        None | Some(4) => return Ok(round_half_even(n, digits)),
        Some(0) => scaled.ceil(),
        Some(1) => scaled.floor(),
        Some(2) => scaled.round(),
        Some(3) => scaled.trunc(),
        Some(other) => {
            return Err(MError::expression(format!(
                "{other} is not a valid RoundingMode value."
            )));
        }
    };
    let r = r / factor;
    Ok(if r.is_finite() { r } else { n })
}

fn digits_arg(args: &[Value]) -> Result<i32, MError> {
    Ok(int_arg(args, 1)?.map_or(0, |d| d.clamp(-308, 308) as i32))
}

fn round(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let digits = digits_arg(args)?;
    let mode = num_arg(args, 2)?;
    number_or_null(args, |n| {
        Ok(Value::Number(round_with_mode(n, digits, mode)?))
    })
}

fn round_mode(args: &[Value], mode: f64) -> Result<Value, MError> {
    let digits = digits_arg(args)?;
    number_or_null(args, |n| {
        Ok(Value::Number(round_with_mode(n, digits, Some(mode))?))
    })
}

fn round_up(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    round_mode(args, 0.0)
}

fn round_down(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    round_mode(args, 1.0)
}

fn round_away_from_zero(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    round_mode(args, 2.0)
}

fn round_toward_zero(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    round_mode(args, 3.0)
}

fn unary(args: &[Value], f: impl FnOnce(f64) -> f64) -> Result<Value, MError> {
    number_or_null(args, |n| Ok(Value::Number(f(n))))
}

fn abs(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    unary(args, f64::abs)
}

fn sign(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    unary(args, |n| {
        if n > 0.0 {
            1.0
        } else if n < 0.0 {
            -1.0
        } else {
            0.0
        }
    })
}

fn binary(args: &[Value], f: impl FnOnce(f64, f64) -> f64) -> Result<Value, MError> {
    match (num_arg(args, 0)?, num_arg(args, 1)?) {
        (Some(a), Some(b)) => Ok(Value::Number(f(a, b))),
        _ => Ok(Value::Null),
    }
}

fn modulo(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    binary(args, |a, b| a % b)
}

fn integer_divide(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    if num_arg(args, 1)? == Some(0.0) && num_arg(args, 0)?.is_some() {
        return Err(MError::expression("Attempted to divide by zero."));
    }
    binary(args, |a, b| (a / b).trunc())
}

fn power(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    binary(args, f64::powf)
}

fn sqrt(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    unary(args, f64::sqrt)
}

fn exp(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    unary(args, f64::exp)
}

fn ln(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    unary(args, f64::ln)
}

fn log10(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    unary(args, f64::log10)
}

fn is_nan(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::Logical(req_num(args, 0)?.is_nan()))
}

fn is_even(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::Logical(req_num(args, 0)? % 2.0 == 0.0))
}

fn is_odd(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::Logical(req_num(args, 0)?.abs() % 2.0 == 1.0))
}

fn integer_from(args: &[Value], facet: Facet) -> Result<Value, MError> {
    match to_number(arg(args, 0))? {
        Value::Number(n) => {
            let mode = num_arg(args, 2)?;
            let r = round_with_mode(n, 0, mode)?;
            Ok(Value::Number(apply_facet(r, Some(facet))?))
        }
        other => Ok(other),
    }
}

fn int64_from(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    integer_from(args, Facet::Int64)
}

fn int32_from(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    integer_from(args, Facet::Int32)
}

fn int16_from(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    integer_from(args, Facet::Int16)
}

fn int8_from(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    integer_from(args, Facet::Int8)
}

fn byte_from(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    integer_from(args, Facet::Byte)
}

fn currency_from(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    match to_number(arg(args, 0))? {
        Value::Number(n) => Ok(Value::Number(round_with_mode(n, 4, num_arg(args, 2)?)?)),
        other => Ok(other),
    }
}

fn logical_from(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    to_logical(arg(args, 0))
}

fn logical_from_text(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    match arg(args, 0) {
        Value::Null => Ok(Value::Null),
        v @ Value::Text(_) => to_logical(v),
        other => Err(convert_error(other, "Text")),
    }
}

fn logical_to_text(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    match arg(args, 0) {
        Value::Null => Ok(Value::Null),
        Value::Logical(b) => Ok(Value::text(if *b { "true" } else { "false" })),
        other => Err(convert_error(other, "Logical")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_numbers() {
        let f = |n, s| format_number(n, s).unwrap_or_else(|e| e.to_string());
        assert_eq!(f(1234.5, "N"), "1,234.50");
        assert_eq!(f(1234.5, "F1"), "1234.5");
        assert_eq!(f(0.256, "P1"), "25.6%");
        assert_eq!(f(42.0, "D5"), "00042");
        assert_eq!(f(255.0, "X"), "FF");
        assert_eq!(f(1234.567, "#,##0.00"), "1,234.57");
        assert_eq!(f(3.0, "000"), "003");
        assert_eq!(f(2.5, "0.##"), "2.5");
        assert_eq!(f(1.5, ""), "1.5");
    }
}
