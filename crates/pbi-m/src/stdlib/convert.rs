//! Culture-invariant value conversions (`Number.From`, `Date.From`, ...) shared by the library,
//! `Table.TransformColumnTypes` and the Arrow conversion.
//!
//! Text parsing accepts invariant/ISO forms and en-US forms (`1,234.5`, `50%`, `2024-01-31`,
//! `1/31/2024`, `2024-01-31T13:45:00`, `1:45 PM`). Empty or blank text converts to `null` for
//! all non-text types (as Power Query does for blank CSV cells). Culture arguments are accepted
//! and ignored.

use chrono::{
    DateTime, Datelike, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, TimeZone,
    Timelike,
};

use crate::ast::PrimitiveType;
use crate::eval::interp::{Interp, convert_error};
use crate::merror::MError;
use crate::value::format::value_to_text;
use crate::value::{Facet, TypeValue, Value};

const OLE_EPOCH: (i32, u32, u32) = (1899, 12, 30);

fn ole_epoch() -> NaiveDate {
    NaiveDate::from_ymd_opt(OLE_EPOCH.0, OLE_EPOCH.1, OLE_EPOCH.2).unwrap_or(NaiveDate::MIN)
}

fn data_format(target: &str, text: &str) -> MError {
    MError::Evaluation {
        reason: "DataFormat.Error".into(),
        message: format!("We couldn't convert to {target}."),
        detail: Some(Box::new(Value::text(text))),
        location: None,
    }
}

/// Parses a number: optional sign, digits with optional `,` group separators, optional
/// fraction and exponent, optional trailing `%`.
pub(crate) fn parse_number(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (s, percent) = match s.strip_suffix('%') {
        Some(rest) => (rest.trim_end(), true),
        None => (s, false),
    };
    let (negative, body) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    if body.is_empty()
        || !body
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | 'e' | 'E' | '+' | '-'))
        || !body.starts_with(|c: char| c.is_ascii_digit() || c == '.')
    {
        return None;
    }
    let cleaned: String = body.chars().filter(|&c| c != ',').collect();
    let mut n: f64 = cleaned.parse().ok()?;
    if negative {
        n = -n;
    }
    if percent {
        n /= 100.0;
    }
    Some(n)
}

/// Rounds half to even at `digits` decimals (M's default `RoundingMode.ToEven`).
pub(crate) fn round_half_even(x: f64, digits: i32) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let factor = 10f64.powi(digits);
    let scaled = x * factor;
    if !scaled.is_finite() {
        return x;
    }
    let r = scaled.round();
    let result = if (scaled - scaled.trunc()).abs() == 0.5 {
        let t = scaled.trunc();
        if t % 2.0 == 0.0 { t } else { r }
    } else {
        r
    };
    result / factor
}

const DATE_FORMATS: &[&str] = &[
    "%Y-%m-%d",
    "%Y/%m/%d",
    "%m/%d/%Y",
    "%d-%b-%Y",
    "%d %b %Y",
    "%d %B %Y",
    "%b %d, %Y",
    "%B %d, %Y",
    "%Y.%m.%d",
    "%Y%m%d",
];

const TIME_FORMATS: &[&str] = &[
    "%H:%M:%S%.f",
    "%H:%M",
    "%I:%M:%S%.f %p",
    "%I:%M %p",
    "%I:%M:%S%.f%p",
    "%I:%M%p",
];

pub(crate) fn parse_date(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    for f in DATE_FORMATS {
        if let Ok(d) = NaiveDate::parse_from_str(s, f) {
            return Some(d);
        }
    }
    parse_datetime(s).map(|dt| dt.date())
}

pub(crate) fn parse_time(s: &str) -> Option<NaiveTime> {
    let s = s.trim();
    for f in TIME_FORMATS {
        if let Ok(t) = NaiveTime::parse_from_str(s, f) {
            return Some(t);
        }
    }
    None
}

/// Splits a trailing UTC offset (`Z`, `+01:00`, `-0500`) off a date-time string.
fn split_offset(s: &str) -> (&str, Option<FixedOffset>) {
    if let Some(rest) = s.strip_suffix('Z').or_else(|| s.strip_suffix('z')) {
        return (rest, FixedOffset::east_opt(0));
    }
    let bytes = s.as_bytes();
    // Look for [+-]hh:mm or [+-]hhmm at the end, after a time part.
    for len in [6usize, 5, 3] {
        if bytes.len() <= len + 8 {
            continue;
        }
        let start = bytes.len() - len;
        let Some(tail) = s.get(start..) else {
            continue;
        };
        let sign = match tail.as_bytes().first() {
            Some(b'+') => 1,
            Some(b'-') => -1,
            _ => continue,
        };
        let digits: String = tail[1..].chars().filter(char::is_ascii_digit).collect();
        if digits.len() + usize::from(tail.contains(':')) + 1 != len {
            continue;
        }
        let (h, m) = match digits.len() {
            4 => (
                digits[..2].parse::<i32>().ok(),
                digits[2..].parse::<i32>().ok(),
            ),
            2 => (digits.parse::<i32>().ok(), Some(0)),
            _ => continue,
        };
        if let (Some(h), Some(m)) = (h, m)
            && h <= 14
            && m < 60
        {
            return (
                s[..start].trim_end(),
                FixedOffset::east_opt(sign * (h * 3600 + m * 60)),
            );
        }
    }
    (s, None)
}

pub(crate) fn parse_datetime(s: &str) -> Option<NaiveDateTime> {
    parse_datetime_offset(s).map(|(dt, _)| dt)
}

fn parse_datetime_offset(s: &str) -> Option<(NaiveDateTime, Option<FixedOffset>)> {
    let s = s.trim();
    let (body, offset) = split_offset(s);
    let split = body
        .find('T')
        .or_else(|| body.find(' ').filter(|&i| body[i..].contains(':')));
    match split {
        Some(i) => {
            let date_part = body[..i].trim();
            let mut time_part = body[i + 1..].trim();
            // en-US with a date like "Jan 5, 2024 1:00 PM": find the date using the formats.
            let date = DATE_FORMATS
                .iter()
                .find_map(|f| NaiveDate::parse_from_str(date_part, f).ok());
            let date = match date {
                Some(d) => d,
                None => {
                    // The split may have happened inside the date ("Jan 5, 2024 13:00").
                    let j = body.rfind(' ')?;
                    time_part = body[j + 1..].trim();
                    let date_part = body[..j].trim();
                    let (date_part, tp) = match date_part.rsplit_once(' ') {
                        Some((d, t))
                            if time_part.eq_ignore_ascii_case("AM")
                                || time_part.eq_ignore_ascii_case("PM") =>
                        {
                            (d, format!("{t} {time_part}"))
                        }
                        _ => (date_part, time_part.to_owned()),
                    };
                    let d = DATE_FORMATS
                        .iter()
                        .find_map(|f| NaiveDate::parse_from_str(date_part, f).ok())?;
                    return Some((NaiveDateTime::new(d, parse_time(&tp)?), offset));
                }
            };
            Some((NaiveDateTime::new(date, parse_time(time_part)?), offset))
        }
        None => DATE_FORMATS
            .iter()
            .find_map(|f| NaiveDate::parse_from_str(body, f).ok())
            .map(|d| (NaiveDateTime::new(d, NaiveTime::MIN), offset)),
    }
}

pub(crate) fn parse_datetimezone(s: &str, local: FixedOffset) -> Option<DateTime<FixedOffset>> {
    let (dt, offset) = parse_datetime_offset(s)?;
    offset.unwrap_or(local).from_local_datetime(&dt).single()
}

/// Parses `[-][d.]hh:mm[:ss[.fffffff]]`.
pub(crate) fn parse_duration(s: &str) -> Option<TimeDelta> {
    let s = s.trim();
    let (negative, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let (days, rest) = match s.split_once(':') {
        Some((head, _)) if head.contains('.') => {
            let (d, _) = head.split_once('.')?;
            (d.parse::<i64>().ok()?, &s[d.len() + 1..])
        }
        Some(_) => (0, s),
        None => (s.parse::<i64>().ok()?, ""),
    };
    let mut total = TimeDelta::try_days(days)?;
    if !rest.is_empty() {
        let parts: Vec<&str> = rest.split(':').collect();
        let (h, m, sec) = match parts.as_slice() {
            [h, m] => (*h, *m, "0"),
            [h, m, s] => (*h, *m, *s),
            _ => return None,
        };
        let h: i64 = h.parse().ok()?;
        let m: i64 = m.parse().ok()?;
        let sec: f64 = sec.parse().ok()?;
        if !(0.0..60.0).contains(&sec) || !(0..60).contains(&m) {
            return None;
        }
        total = total
            .checked_add(&TimeDelta::try_hours(h)?)?
            .checked_add(&TimeDelta::try_minutes(m)?)?
            .checked_add(&TimeDelta::microseconds((sec * 1e6).round() as i64))?;
    }
    Some(if negative { -total } else { total })
}

fn days_to_delta(days: f64) -> Option<TimeDelta> {
    let micros = days * 86_400_000_000.0;
    if !micros.is_finite() || micros.abs() > 9.0e18 {
        return None;
    }
    Some(TimeDelta::microseconds(micros.round() as i64))
}

fn ole_to_datetime(days: f64) -> Option<NaiveDateTime> {
    NaiveDateTime::new(ole_epoch(), NaiveTime::MIN).checked_add_signed(days_to_delta(days)?)
}

fn datetime_to_ole(dt: NaiveDateTime) -> f64 {
    let delta = dt.signed_duration_since(NaiveDateTime::new(ole_epoch(), NaiveTime::MIN));
    delta
        .num_microseconds()
        .map_or(f64::NAN, |m| m as f64 / 86_400_000_000.0)
}

fn time_fraction(t: NaiveTime) -> f64 {
    (f64::from(t.num_seconds_from_midnight()) + f64::from(t.nanosecond() % 1_000_000_000) / 1e9)
        / 86_400.0
}

fn blank(v: &Value) -> bool {
    v.as_text().is_some_and(|t| t.trim().is_empty())
}

/// `Number.From`.
pub(crate) fn to_number(v: &Value) -> Result<Value, MError> {
    Ok(match v.plain() {
        Value::Null => Value::Null,
        Value::Number(n) => Value::Number(*n),
        Value::Logical(b) => Value::Number(if *b { 1.0 } else { 0.0 }),
        Value::Text(_) if blank(v) => Value::Null,
        Value::Text(t) => Value::Number(parse_number(t).ok_or_else(|| data_format("Number", t))?),
        Value::Date(d) => Value::Number(datetime_to_ole(NaiveDateTime::new(*d, NaiveTime::MIN))),
        Value::DateTime(dt) => Value::Number(datetime_to_ole(*dt)),
        Value::DateTimeZone(dt) => Value::Number(datetime_to_ole(dt.naive_utc())),
        Value::Time(t) => Value::Number(time_fraction(*t)),
        Value::Duration(d) => Value::Number(
            d.num_microseconds()
                .map_or(f64::NAN, |m| m as f64 / 86_400_000_000.0),
        ),
        other => return Err(convert_error(other, "Number")),
    })
}

/// `Text.From`.
pub(crate) fn to_text(v: &Value) -> Result<Value, MError> {
    match v.plain() {
        Value::Null => Ok(Value::Null),
        Value::Text(t) => Ok(Value::Text(t.clone())),
        other => match value_to_text(other) {
            Some(s) => Ok(Value::text(s)),
            None => Err(MError::expression(format!(
                "We cannot convert a value of type {} to type Text.",
                other.kind().name()
            ))),
        },
    }
}

/// `Logical.From`.
pub(crate) fn to_logical(v: &Value) -> Result<Value, MError> {
    Ok(match v.plain() {
        Value::Null => Value::Null,
        Value::Logical(b) => Value::Logical(*b),
        Value::Number(n) => Value::Logical(*n != 0.0),
        Value::Text(_) if blank(v) => Value::Null,
        Value::Text(t) => match t.trim().to_ascii_lowercase().as_str() {
            "true" => Value::Logical(true),
            "false" => Value::Logical(false),
            _ => return Err(data_format("Logical", t)),
        },
        other => return Err(convert_error(other, "Logical")),
    })
}

/// `Date.From`.
pub(crate) fn to_date(v: &Value) -> Result<Value, MError> {
    Ok(match v.plain() {
        Value::Null => Value::Null,
        Value::Date(d) => Value::Date(*d),
        Value::DateTime(dt) => Value::Date(dt.date()),
        Value::DateTimeZone(dt) => Value::Date(dt.date_naive()),
        Value::Number(n) => Value::Date(
            ole_to_datetime(*n)
                .ok_or_else(|| convert_error(v, "Date"))?
                .date(),
        ),
        Value::Text(_) if blank(v) => Value::Null,
        Value::Text(t) => Value::Date(parse_date(t).ok_or_else(|| data_format("Date", t))?),
        other => return Err(convert_error(other, "Date")),
    })
}

/// `DateTime.From` (`datetimezone` values are converted to local time).
pub(crate) fn to_datetime(v: &Value, local: FixedOffset) -> Result<Value, MError> {
    Ok(match v.plain() {
        Value::Null => Value::Null,
        Value::DateTime(dt) => Value::DateTime(*dt),
        Value::Date(d) => Value::DateTime(NaiveDateTime::new(*d, NaiveTime::MIN)),
        Value::DateTimeZone(dt) => Value::DateTime(dt.with_timezone(&local).naive_local()),
        Value::Time(t) => Value::DateTime(NaiveDateTime::new(ole_epoch(), *t)),
        Value::Number(n) => {
            Value::DateTime(ole_to_datetime(*n).ok_or_else(|| convert_error(v, "DateTime"))?)
        }
        Value::Text(_) if blank(v) => Value::Null,
        Value::Text(t) => {
            Value::DateTime(parse_datetime(t).ok_or_else(|| data_format("DateTime", t))?)
        }
        other => return Err(convert_error(other, "DateTime")),
    })
}

/// `DateTimeZone.From` (values without an offset get the local offset).
pub(crate) fn to_datetimezone(v: &Value, local: FixedOffset) -> Result<Value, MError> {
    let attach = |dt: NaiveDateTime| {
        local
            .from_local_datetime(&dt)
            .single()
            .map(Value::DateTimeZone)
            .ok_or_else(|| convert_error(v, "DateTimeZone"))
    };
    match v.plain() {
        Value::Null => Ok(Value::Null),
        Value::DateTimeZone(dt) => Ok(Value::DateTimeZone(*dt)),
        Value::DateTime(dt) => attach(*dt),
        Value::Date(d) => attach(NaiveDateTime::new(*d, NaiveTime::MIN)),
        Value::Number(n) => {
            attach(ole_to_datetime(*n).ok_or_else(|| convert_error(v, "DateTimeZone"))?)
        }
        Value::Text(_) if blank(v) => Ok(Value::Null),
        Value::Text(t) => parse_datetimezone(t, local)
            .map(Value::DateTimeZone)
            .ok_or_else(|| data_format("DateTimeZone", t)),
        other => Err(convert_error(other, "DateTimeZone")),
    }
}

/// `Time.From`.
pub(crate) fn to_time(v: &Value) -> Result<Value, MError> {
    Ok(match v.plain() {
        Value::Null => Value::Null,
        Value::Time(t) => Value::Time(*t),
        Value::DateTime(dt) => Value::Time(dt.time()),
        Value::DateTimeZone(dt) => Value::Time(dt.time()),
        Value::Number(n) if (0.0..1.0).contains(n) => Value::Time(
            NaiveTime::MIN
                .overflowing_add_signed(days_to_delta(*n).unwrap_or_default())
                .0,
        ),
        Value::Text(_) if blank(v) => Value::Null,
        Value::Text(t) => Value::Time(
            parse_time(t)
                .or_else(|| parse_datetime(t).map(|d| d.time()))
                .ok_or_else(|| data_format("Time", t))?,
        ),
        other => return Err(convert_error(other, "Time")),
    })
}

/// `Duration.From` (numbers are days).
pub(crate) fn to_duration(v: &Value) -> Result<Value, MError> {
    Ok(match v.plain() {
        Value::Null => Value::Null,
        Value::Duration(d) => Value::Duration(*d),
        Value::Number(n) => {
            Value::Duration(days_to_delta(*n).ok_or_else(|| convert_error(v, "Duration"))?)
        }
        Value::Text(_) if blank(v) => Value::Null,
        Value::Text(t) => {
            Value::Duration(parse_duration(t).ok_or_else(|| data_format("Duration", t))?)
        }
        other => return Err(convert_error(other, "Duration")),
    })
}

/// Applies a number facet: integer types round half to even, `Currency.Type` keeps 4 decimals.
pub(crate) fn apply_facet(n: f64, facet: Option<Facet>) -> Result<f64, MError> {
    Ok(match facet {
        Some(f) if f.is_integer() => {
            let r = round_half_even(n, 0);
            let (lo, hi) = match f {
                Facet::Int64 => (-9.223_372_036_854_776e18, 9.223_372_036_854_776e18),
                Facet::Int32 => (f64::from(i32::MIN), f64::from(i32::MAX)),
                Facet::Int16 => (f64::from(i16::MIN), f64::from(i16::MAX)),
                Facet::Int8 => (f64::from(i8::MIN), f64::from(i8::MAX)),
                _ => (0.0, 255.0),
            };
            if !(lo..=hi).contains(&r) {
                return Err(MError::expression(format!(
                    "The number is out of range of a {} value.",
                    f.type_name()
                )));
            }
            r
        }
        Some(Facet::Currency) => round_half_even(n, 4),
        _ => n,
    })
}

/// Converts a value to a type, as `Table.TransformColumnTypes` does.
pub(crate) fn convert_to_type(it: &Interp, v: &Value, ty: &TypeValue) -> Result<Value, MError> {
    if v.is_null() {
        return Ok(Value::Null);
    }
    let local = it.local_offset;
    match ty.base() {
        PrimitiveType::Any => Ok(v.clone()),
        PrimitiveType::AnyNonNull => Ok(v.clone()),
        PrimitiveType::Text => to_text(v),
        PrimitiveType::Number => match to_number(v)? {
            Value::Number(n) => Ok(Value::Number(apply_facet(n, ty.facet_kind())?)),
            other => Ok(other),
        },
        PrimitiveType::Logical => to_logical(v),
        PrimitiveType::Date => to_date(v),
        PrimitiveType::DateTime => to_datetime(v, local),
        PrimitiveType::DateTimeZone => to_datetimezone(v, local),
        PrimitiveType::Time => to_time(v),
        PrimitiveType::Duration => to_duration(v),
        _ if ty.conforms(v) => Ok(v.clone()),
        _ => Err(convert_error(v, &ty.message_name())),
    }
}

/// Days since 1970-01-01 (Arrow `Date32`).
pub(crate) fn date_to_epoch_days(d: NaiveDate) -> i32 {
    d.num_days_from_ce() - 719_163
}

/// Microseconds since the Unix epoch of a naive date-time taken as UTC.
pub(crate) fn datetime_to_epoch_micros(dt: NaiveDateTime) -> i64 {
    dt.and_utc().timestamp_micros()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_parse_invariant_and_en_us() {
        assert_eq!(parse_number(" 42 "), Some(42.0));
        assert_eq!(parse_number("-1,234.5"), Some(-1234.5));
        assert_eq!(parse_number("1e3"), Some(1000.0));
        assert_eq!(parse_number("50%"), Some(0.5));
        assert_eq!(parse_number(".5"), Some(0.5));
        assert_eq!(parse_number("abc"), None);
        assert_eq!(parse_number("inf"), None);
        assert_eq!(parse_number(""), None);
    }

    #[test]
    fn rounding_is_half_to_even() {
        assert_eq!(round_half_even(2.5, 0), 2.0);
        assert_eq!(round_half_even(3.5, 0), 4.0);
        assert_eq!(round_half_even(-2.5, 0), -2.0);
        assert_eq!(round_half_even(1.2345, 2), 1.23);
    }

    #[test]
    fn dates_and_times_parse() {
        let d = NaiveDate::from_ymd_opt(2024, 1, 31);
        assert_eq!(parse_date("2024-01-31"), d);
        assert_eq!(parse_date("1/31/2024"), d);
        assert_eq!(parse_date("31-Jan-2024"), d);
        assert_eq!(parse_date("2024-01-31T10:00:00"), d);
        let dt =
            parse_datetime("2024-01-31 13:45:10.5").map(|x| (x.hour(), x.minute(), x.second()));
        assert_eq!(dt, Some((13, 45, 10)));
        assert_eq!(
            parse_datetime("1/31/2024 1:45 PM").map(|x| x.hour()),
            Some(13)
        );
        let z = parse_datetimezone(
            "2024-01-31T13:45:00+02:00",
            FixedOffset::east_opt(0).unwrap_or_else(|| unreachable!()),
        );
        assert_eq!(z.map(|x| x.offset().local_minus_utc()), Some(7200));
        assert_eq!(parse_time("1:05 PM").map(|t| t.hour()), Some(13));
        assert_eq!(
            parse_duration("1.02:03:04"),
            Some(TimeDelta::seconds(86_400 + 2 * 3600 + 3 * 60 + 4))
        );
        assert_eq!(parse_duration("-00:00:30"), Some(TimeDelta::seconds(-30)));
    }
}
