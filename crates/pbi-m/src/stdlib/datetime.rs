//! Date/time intrinsics (`#date`, `#datetime`, `#datetimezone`, `#time`, `#duration`) and the
//! `Date.*`, `DateTime.*`, `DateTimeZone.*`, `Time.*` and `Duration.*` functions.

use chrono::{
    DateTime, Datelike, FixedOffset, Months, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta,
    TimeZone, Timelike,
};

use super::Builder;
use super::args::{arg, int_arg, num_arg, option, req_num, text_arg};
use super::convert::{
    parse_date, parse_datetime, parse_datetimezone, parse_duration, parse_time, to_date,
    to_datetime, to_datetimezone, to_duration, to_time,
};
use crate::eval::interp::{Interp, convert_error};
use crate::merror::MError;
use crate::value::Value;
use crate::value::format::value_to_text;

pub(super) fn register(b: &mut Builder) {
    b.func("#date", &["year", "month", "day"], hash_date);
    b.func(
        "#datetime",
        &["year", "month", "day", "hour", "minute", "second"],
        hash_datetime,
    );
    b.func(
        "#datetimezone",
        &[
            "year",
            "month",
            "day",
            "hour",
            "minute",
            "second",
            "offsetHours",
            "offsetMinutes",
        ],
        hash_datetimezone,
    );
    b.func("#time", &["hour", "minute", "second"], hash_time);
    b.func(
        "#duration",
        &["days", "hours", "minutes", "seconds"],
        hash_duration,
    );

    b.func("DateTimeZone.UtcNow", &[], utc_now);
    b.func("DateTimeZone.FixedUtcNow", &[], utc_now);
    b.func("DateTimeZone.LocalNow", &[], local_now_zone);
    b.func("DateTimeZone.FixedLocalNow", &[], local_now_zone);
    b.func("DateTime.LocalNow", &[], local_now);
    b.func("DateTime.FixedLocalNow", &[], local_now);
    b.func("DateTimeZone.RemoveZone", &["dateTimeZone"], remove_zone);
    b.func(
        "DateTimeZone.SwitchZone",
        &["dateTimeZone", "timezoneHours", "?timezoneMinutes"],
        switch_zone,
    );
    b.func("DateTimeZone.ToUtc", &["dateTimeZone"], to_utc);
    b.func("DateTimeZone.ToLocal", &["dateTimeZone"], to_local);
    b.func("DateTimeZone.ZoneHours", &["dateTimeZone"], zone_hours);
    b.func("DateTimeZone.ZoneMinutes", &["dateTimeZone"], zone_minutes);
    b.func(
        "DateTime.AddZone",
        &["dateTime", "timezoneHours", "?timezoneMinutes"],
        add_zone,
    );
    b.func("Date.From", &["value", "?culture"], date_from);
    b.func("DateTime.From", &["value", "?culture"], datetime_from);
    b.func(
        "DateTimeZone.From",
        &["value", "?culture"],
        datetimezone_from,
    );
    b.func("Time.From", &["value", "?culture"], time_from);
    b.func("Duration.From", &["value"], duration_from);
    b.func("Date.FromText", &["text", "?options"], date_from_text);
    b.func(
        "DateTime.FromText",
        &["text", "?options"],
        datetime_from_text,
    );
    b.func(
        "DateTimeZone.FromText",
        &["text", "?options"],
        datetimezone_from_text,
    );
    b.func("Time.FromText", &["text", "?options"], time_from_text);
    b.func("Duration.FromText", &["text"], duration_from_text);
    b.func("DateTime.Date", &["dateTime"], datetime_date);
    b.func("DateTime.Time", &["dateTime"], datetime_time);
    b.func("Date.Year", &["dateTime"], year);
    b.func("Date.Month", &["dateTime"], month);
    b.func("Date.Day", &["dateTime"], day);
    b.func(
        "Date.DayOfWeek",
        &["dateTime", "?firstDayOfWeek"],
        day_of_week,
    );
    b.func("Date.DayOfYear", &["dateTime"], day_of_year);
    b.func("Date.DaysInMonth", &["dateTime"], days_in_month);
    b.func("Date.IsLeapYear", &["dateTime"], is_leap_year);
    b.func("Date.AddDays", &["dateTime", "numberOfDays"], add_days);
    b.func(
        "Date.AddMonths",
        &["dateTime", "numberOfMonths"],
        add_months,
    );
    b.func("Date.AddYears", &["dateTime", "numberOfYears"], add_years);
    b.func("Date.StartOfMonth", &["dateTime"], start_of_month);
    b.func("Date.EndOfMonth", &["dateTime"], end_of_month);
    b.func("Date.StartOfYear", &["dateTime"], start_of_year);
    b.func("Date.EndOfYear", &["dateTime"], end_of_year);
    b.func("Time.Hour", &["dateTime"], hour);
    b.func("Time.Minute", &["dateTime"], minute);
    b.func("Time.Second", &["dateTime"], second);
    b.func("Duration.Days", &["duration"], duration_days);
    b.func("Duration.Hours", &["duration"], duration_hours);
    b.func("Duration.Minutes", &["duration"], duration_minutes);
    b.func("Duration.Seconds", &["duration"], duration_seconds);
    b.func("Duration.TotalDays", &["duration"], total_days);
    b.func("Duration.TotalHours", &["duration"], total_hours);
    b.func("Duration.TotalMinutes", &["duration"], total_minutes);
    b.func("Duration.TotalSeconds", &["duration"], total_seconds);
    b.func("Date.ToText", &["date", "?options", "?culture"], to_text);
    b.func(
        "DateTime.ToText",
        &["dateTime", "?options", "?culture"],
        to_text,
    );
    b.func(
        "DateTimeZone.ToText",
        &["dateTimeZone", "?options", "?culture"],
        to_text,
    );
    b.func("Time.ToText", &["time", "?options", "?culture"], to_text);
    b.func("Duration.ToText", &["duration", "?format"], to_text);
    b.consts(&[
        ("Day.Sunday", 0.0),
        ("Day.Monday", 1.0),
        ("Day.Tuesday", 2.0),
        ("Day.Wednesday", 3.0),
        ("Day.Thursday", 4.0),
        ("Day.Friday", 5.0),
        ("Day.Saturday", 6.0),
    ]);
}

fn invalid_date() -> MError {
    MError::expression("The Date operation failed because the values are out of range.")
}

fn int_part(args: &[Value], i: usize) -> Result<i64, MError> {
    int_arg(args, i)?.ok_or_else(|| convert_error(&Value::Null, "Number"))
}

fn small(n: i64) -> Result<u32, MError> {
    u32::try_from(n).map_err(|_| invalid_date())
}

fn make_date(args: &[Value]) -> Result<NaiveDate, MError> {
    let y = i32::try_from(int_part(args, 0)?).map_err(|_| invalid_date())?;
    NaiveDate::from_ymd_opt(y, small(int_part(args, 1)?)?, small(int_part(args, 2)?)?)
        .ok_or_else(invalid_date)
}

fn make_time(h: i64, m: i64, s: f64) -> Result<NaiveTime, MError> {
    if !(0.0..60.0).contains(&s) {
        return Err(invalid_date());
    }
    let whole = s.trunc();
    let nanos = ((s - whole) * 1e9).round().min(999_999_999.0) as u32;
    NaiveTime::from_hms_nano_opt(small(h)?, small(m)?, whole as u32, nanos).ok_or_else(invalid_date)
}

fn hash_date(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::Date(make_date(args)?))
}

fn make_datetime(args: &[Value]) -> Result<NaiveDateTime, MError> {
    let t = make_time(int_part(args, 3)?, int_part(args, 4)?, req_num(args, 5)?)?;
    Ok(NaiveDateTime::new(make_date(args)?, t))
}

fn hash_datetime(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::DateTime(make_datetime(args)?))
}

fn offset(hours: i64, minutes: i64) -> Result<FixedOffset, MError> {
    let secs = hours
        .checked_mul(3600)
        .and_then(|h| h.checked_add(minutes.checked_mul(if hours < 0 { -60 } else { 60 })?))
        .and_then(|s| i32::try_from(s).ok())
        .ok_or_else(invalid_date)?;
    FixedOffset::east_opt(secs).ok_or_else(invalid_date)
}

fn hash_datetimezone(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let dt = make_datetime(args)?;
    let off = offset(int_part(args, 6)?, int_part(args, 7)?)?;
    off.from_local_datetime(&dt)
        .single()
        .map(Value::DateTimeZone)
        .ok_or_else(invalid_date)
}

fn hash_time(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::Time(make_time(
        int_part(args, 0)?,
        int_part(args, 1)?,
        req_num(args, 2)?,
    )?))
}

fn hash_duration(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let secs = req_num(args, 0)? * 86_400.0
        + req_num(args, 1)? * 3_600.0
        + req_num(args, 2)? * 60.0
        + req_num(args, 3)?;
    let micros = (secs * 1e6).round();
    if !micros.is_finite() || micros.abs() > 9.0e18 {
        return Err(MError::expression("The duration is out of range."));
    }
    Ok(Value::Duration(TimeDelta::microseconds(micros as i64)))
}

fn utc_now(it: &Interp, _: &[Value]) -> Result<Value, MError> {
    Ok(Value::DateTimeZone(it.now.fixed_offset()))
}

fn local_now_zone(it: &Interp, _: &[Value]) -> Result<Value, MError> {
    Ok(Value::DateTimeZone(it.now.with_timezone(&it.local_offset)))
}

fn local_now(it: &Interp, _: &[Value]) -> Result<Value, MError> {
    Ok(Value::DateTime(
        it.now.with_timezone(&it.local_offset).naive_local(),
    ))
}

fn dtz_arg(args: &[Value]) -> Result<Option<DateTime<FixedOffset>>, MError> {
    match arg(args, 0) {
        Value::Null => Ok(None),
        Value::DateTimeZone(d) => Ok(Some(*d)),
        other => Err(convert_error(other, "DateTimeZone")),
    }
}

fn remove_zone(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(dtz_arg(args)?.map_or(Value::Null, |d| Value::DateTime(d.naive_local())))
}

fn switch_zone(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let off = offset(int_part(args, 1)?, int_arg(args, 2)?.unwrap_or(0))?;
    Ok(dtz_arg(args)?.map_or(Value::Null, |d| Value::DateTimeZone(d.with_timezone(&off))))
}

fn to_utc(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(dtz_arg(args)?.map_or(Value::Null, |d| {
        Value::DateTimeZone(d.to_utc().fixed_offset())
    }))
}

fn to_local(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(dtz_arg(args)?.map_or(Value::Null, |d| {
        Value::DateTimeZone(d.with_timezone(&it.local_offset))
    }))
}

fn zone_hours(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(dtz_arg(args)?.map_or(Value::Null, |d| {
        Value::Number(f64::from(d.offset().local_minus_utc() / 3600))
    }))
}

fn zone_minutes(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(dtz_arg(args)?.map_or(Value::Null, |d| {
        Value::Number(f64::from((d.offset().local_minus_utc() % 3600) / 60))
    }))
}

fn add_zone(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let off = offset(int_part(args, 1)?, int_arg(args, 2)?.unwrap_or(0))?;
    match arg(args, 0) {
        Value::Null => Ok(Value::Null),
        Value::DateTime(dt) => off
            .from_local_datetime(dt)
            .single()
            .map(Value::DateTimeZone)
            .ok_or_else(invalid_date),
        other => Err(convert_error(other, "DateTime")),
    }
}

fn date_from(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    to_date(arg(args, 0))
}

fn datetime_from(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    to_datetime(arg(args, 0), it.local_offset)
}

fn datetimezone_from(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    to_datetimezone(arg(args, 0), it.local_offset)
}

fn time_from(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    to_time(arg(args, 0))
}

fn duration_from(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    to_duration(arg(args, 0))
}

fn from_text_with(
    args: &[Value],
    target: &str,
    parse: impl FnOnce(&str) -> Option<Value>,
) -> Result<Value, MError> {
    match text_arg(args, 0, target)? {
        None => Ok(Value::Null),
        Some(t) => parse(t).ok_or_else(|| MError::Evaluation {
            reason: "DataFormat.Error".into(),
            message: format!("We couldn't parse the input provided as a {target} value."),
            detail: Some(Box::new(Value::text(t))),
            location: None,
        }),
    }
}

fn date_from_text(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    from_text_with(args, "Date", |t| parse_date(t).map(Value::Date))
}

fn datetime_from_text(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    from_text_with(args, "DateTime", |t| parse_datetime(t).map(Value::DateTime))
}

fn datetimezone_from_text(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    from_text_with(args, "DateTimeZone", |t| {
        parse_datetimezone(t, it.local_offset).map(Value::DateTimeZone)
    })
}

fn time_from_text(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    from_text_with(args, "Time", |t| parse_time(t).map(Value::Time))
}

fn duration_from_text(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    from_text_with(args, "Duration", |t| parse_duration(t).map(Value::Duration))
}

type Parts = (Option<NaiveDate>, Option<NaiveTime>);

/// The date and time parts of a date/datetime/datetimezone/time argument.
fn parts(v: &Value) -> Result<Option<Parts>, MError> {
    Ok(Some(match v.plain() {
        Value::Null => return Ok(None),
        Value::Date(d) => (Some(*d), None),
        Value::DateTime(dt) => (Some(dt.date()), Some(dt.time())),
        Value::DateTimeZone(dt) => (Some(dt.date_naive()), Some(dt.time())),
        Value::Time(t) => (None, Some(*t)),
        other => return Err(convert_error(other, "Date")),
    }))
}

fn date_part(args: &[Value], f: impl FnOnce(NaiveDate) -> f64) -> Result<Value, MError> {
    match parts(arg(args, 0))? {
        None => Ok(Value::Null),
        Some((Some(d), _)) => Ok(Value::Number(f(d))),
        Some((None, _)) => Err(convert_error(arg(args, 0), "Date")),
    }
}

fn time_part(args: &[Value], f: impl FnOnce(NaiveTime) -> f64) -> Result<Value, MError> {
    match parts(arg(args, 0))? {
        None => Ok(Value::Null),
        Some((_, Some(t))) => Ok(Value::Number(f(t))),
        Some((_, None)) => Err(convert_error(arg(args, 0), "Time")),
    }
}

fn datetime_date(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    match parts(arg(args, 0))? {
        Some((Some(d), _)) => Ok(Value::Date(d)),
        Some((None, _)) => Err(convert_error(arg(args, 0), "DateTime")),
        None => Ok(Value::Null),
    }
}

fn datetime_time(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    match parts(arg(args, 0))? {
        Some((_, t)) => Ok(Value::Time(t.unwrap_or(NaiveTime::MIN))),
        None => Ok(Value::Null),
    }
}

fn year(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    date_part(args, |d| f64::from(d.year()))
}

fn month(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    date_part(args, |d| f64::from(d.month()))
}

fn day(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    date_part(args, |d| f64::from(d.day()))
}

fn day_of_week(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let first = num_arg(args, 1)?.unwrap_or(0.0) as u32 % 7;
    date_part(args, |d| {
        f64::from((d.weekday().num_days_from_sunday() + 7 - first) % 7)
    })
}

fn day_of_year(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    date_part(args, |d| f64::from(d.ordinal()))
}

fn month_length(d: NaiveDate) -> u32 {
    let first = NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap_or(d);
    first.checked_add_months(Months::new(1)).map_or(31, |next| {
        next.signed_duration_since(first).num_days() as u32
    })
}

fn days_in_month(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    date_part(args, |d| f64::from(month_length(d)))
}

fn is_leap_year(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    match parts(arg(args, 0))? {
        Some((Some(d), _)) => Ok(Value::Logical(d.leap_year())),
        Some(_) => Err(convert_error(arg(args, 0), "Date")),
        None => Ok(Value::Null),
    }
}

/// Applies a date transformation to a date/datetime/datetimezone, keeping its kind.
fn map_date(v: &Value, f: impl FnOnce(NaiveDate) -> Option<NaiveDate>) -> Result<Value, MError> {
    Ok(match v.plain() {
        Value::Null => Value::Null,
        Value::Date(d) => Value::Date(f(*d).ok_or_else(invalid_date)?),
        Value::DateTime(dt) => Value::DateTime(NaiveDateTime::new(
            f(dt.date()).ok_or_else(invalid_date)?,
            dt.time(),
        )),
        Value::DateTimeZone(dt) => {
            let local = NaiveDateTime::new(f(dt.date_naive()).ok_or_else(invalid_date)?, dt.time());
            Value::DateTimeZone(
                dt.offset()
                    .from_local_datetime(&local)
                    .single()
                    .ok_or_else(invalid_date)?,
            )
        }
        other => return Err(convert_error(other, "Date")),
    })
}

fn add_days(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let n = int_part(args, 1)?;
    map_date(arg(args, 0), |d| {
        d.checked_add_signed(TimeDelta::try_days(n)?)
    })
}

fn add_months_to(d: NaiveDate, n: i64) -> Option<NaiveDate> {
    let m = u32::try_from(n.unsigned_abs()).ok()?;
    if n >= 0 {
        d.checked_add_months(Months::new(m))
    } else {
        d.checked_sub_months(Months::new(m))
    }
}

fn add_months(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let n = int_part(args, 1)?;
    map_date(arg(args, 0), |d| add_months_to(d, n))
}

fn add_years(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let n = int_part(args, 1)?;
    map_date(arg(args, 0), |d| add_months_to(d, n.checked_mul(12)?))
}

fn start_of_day(v: Value) -> Value {
    match v {
        Value::DateTime(dt) => Value::DateTime(NaiveDateTime::new(dt.date(), NaiveTime::MIN)),
        Value::DateTimeZone(dt) => dt
            .offset()
            .from_local_datetime(&NaiveDateTime::new(dt.date_naive(), NaiveTime::MIN))
            .single()
            .map_or(Value::DateTimeZone(dt), Value::DateTimeZone),
        other => other,
    }
}

fn end_of_day(v: Value) -> Value {
    let end = NaiveTime::from_hms_micro_opt(23, 59, 59, 999_999).unwrap_or(NaiveTime::MIN);
    match v {
        Value::DateTime(dt) => Value::DateTime(NaiveDateTime::new(dt.date(), end)),
        Value::DateTimeZone(dt) => dt
            .offset()
            .from_local_datetime(&NaiveDateTime::new(dt.date_naive(), end))
            .single()
            .map_or(Value::DateTimeZone(dt), Value::DateTimeZone),
        other => other,
    }
}

fn start_of_month(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    map_date(arg(args, 0), |d| {
        NaiveDate::from_ymd_opt(d.year(), d.month(), 1)
    })
    .map(start_of_day)
}

fn end_of_month(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    map_date(arg(args, 0), |d| {
        NaiveDate::from_ymd_opt(d.year(), d.month(), month_length(d))
    })
    .map(end_of_day)
}

fn start_of_year(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    map_date(arg(args, 0), |d| NaiveDate::from_ymd_opt(d.year(), 1, 1)).map(start_of_day)
}

fn end_of_year(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    map_date(arg(args, 0), |d| NaiveDate::from_ymd_opt(d.year(), 12, 31)).map(end_of_day)
}

fn hour(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    time_part(args, |t| f64::from(t.hour()))
}

fn minute(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    time_part(args, |t| f64::from(t.minute()))
}

fn second(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    time_part(args, |t| {
        f64::from(t.second()) + f64::from(t.nanosecond() % 1_000_000_000) / 1e9
    })
}

fn duration_arg(args: &[Value]) -> Result<Option<TimeDelta>, MError> {
    match arg(args, 0) {
        Value::Null => Ok(None),
        Value::Duration(d) => Ok(Some(*d)),
        other => Err(convert_error(other, "Duration")),
    }
}

fn duration_part(args: &[Value], f: impl FnOnce(TimeDelta) -> f64) -> Result<Value, MError> {
    Ok(duration_arg(args)?.map_or(Value::Null, |d| Value::Number(f(d))))
}

fn micros(d: TimeDelta) -> f64 {
    d.num_microseconds()
        .map_or(d.num_milliseconds() as f64 * 1e3, |m| m as f64)
}

fn duration_days(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    duration_part(args, |d| d.num_days() as f64)
}

fn duration_hours(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    duration_part(args, |d| (d.num_hours() % 24) as f64)
}

fn duration_minutes(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    duration_part(args, |d| (d.num_minutes() % 60) as f64)
}

fn duration_seconds(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    duration_part(args, |d| (micros(d) / 1e6) % 60.0)
}

fn total_days(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    duration_part(args, |d| micros(d) / 86_400e6)
}

fn total_hours(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    duration_part(args, |d| micros(d) / 3_600e6)
}

fn total_minutes(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    duration_part(args, |d| micros(d) / 60e6)
}

fn total_seconds(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    duration_part(args, |d| micros(d) / 1e6)
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// Formats date/time parts with a .NET-style format string (en-US).
pub(crate) fn format_datetime(
    date: Option<NaiveDate>,
    time: Option<NaiveTime>,
    offset: Option<FixedOffset>,
    fmt: &str,
) -> String {
    let expanded = match fmt {
        "d" => "M/d/yyyy",
        "D" => "dddd, MMMM d, yyyy",
        "t" => "h:mm tt",
        "T" => "h:mm:ss tt",
        "g" => "M/d/yyyy h:mm tt",
        "G" => "M/d/yyyy h:mm:ss tt",
        "s" => "yyyy-MM-ddTHH:mm:ss",
        "o" | "O" => "yyyy-MM-ddTHH:mm:ss.fffffffzzz",
        "u" => "yyyy-MM-dd HH:mm:ssZ",
        other => other,
    };
    let d = date.unwrap_or(NaiveDate::MIN);
    let t = time.unwrap_or(NaiveTime::MIN);
    let chars: Vec<char> = expanded.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let mut n = 1;
        while i + n < chars.len() && chars[i + n] == c {
            n += 1;
        }
        match c {
            'y' => {
                if n <= 2 {
                    out.push_str(&format!("{:0n$}", d.year().rem_euclid(100)));
                } else {
                    out.push_str(&format!("{:0n$}", d.year()));
                }
            }
            'M' => match n {
                1 => out.push_str(&d.month().to_string()),
                2 => out.push_str(&format!("{:02}", d.month())),
                3 => out.push_str(&MONTHS[d.month0() as usize][..3]),
                _ => out.push_str(MONTHS[d.month0() as usize]),
            },
            'd' => match n {
                1 => out.push_str(&d.day().to_string()),
                2 => out.push_str(&format!("{:02}", d.day())),
                3 => out.push_str(&DAYS[d.weekday().num_days_from_sunday() as usize][..3]),
                _ => out.push_str(DAYS[d.weekday().num_days_from_sunday() as usize]),
            },
            'H' => out.push_str(&format!("{:0w$}", t.hour(), w = n.min(2))),
            'h' => out.push_str(&format!("{:0w$}", t.hour12().1, w = n.min(2))),
            'm' => out.push_str(&format!("{:0w$}", t.minute(), w = n.min(2))),
            's' => out.push_str(&format!("{:0w$}", t.second(), w = n.min(2))),
            'f' | 'F' => {
                let frac = format!("{:09}", t.nanosecond() % 1_000_000_000);
                let mut digits = frac[..n.min(9)].to_owned();
                if c == 'F' {
                    while digits.ends_with('0') {
                        digits.pop();
                    }
                }
                out.push_str(&digits);
            }
            't' => {
                let ampm = if t.hour12().0 { "PM" } else { "AM" };
                out.push_str(if n == 1 { &ampm[..1] } else { ampm });
            }
            'z' => {
                let off = offset.map_or(0, |o| o.local_minus_utc());
                let sign = if off < 0 { '-' } else { '+' };
                let (h, m) = (off.abs() / 3600, (off.abs() % 3600) / 60);
                out.push_str(&match n {
                    1 => format!("{sign}{h}"),
                    2 => format!("{sign}{h:02}"),
                    _ => format!("{sign}{h:02}:{m:02}"),
                });
            }
            '\'' | '"' => {
                let mut j = i + 1;
                while j < chars.len() && chars[j] != c {
                    out.push(chars[j]);
                    j += 1;
                }
                i = j + 1;
                continue;
            }
            '\\' => {
                if let Some(next) = chars.get(i + 1) {
                    out.push(*next);
                }
                i += 2;
                continue;
            }
            other => {
                for _ in 0..n {
                    out.push(other);
                }
            }
        }
        i += n;
    }
    out
}

fn to_text(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let v = arg(args, 0);
    if v.is_null() {
        return Ok(Value::Null);
    }
    let fmt = match arg(args, 1) {
        Value::Null => None,
        Value::Text(t) => Some(t.to_string()),
        Value::Record(r) => {
            option(it, Some(r), "Format")?.and_then(|f| f.as_text().map(str::to_owned))
        }
        other => return Err(convert_error(other, "Text")),
    };
    let Some(fmt) = fmt else {
        return Ok(value_to_text(v).map_or(Value::Null, Value::text));
    };
    if let Value::Duration(_) = v.plain() {
        return Ok(value_to_text(v).map_or(Value::Null, Value::text));
    }
    let offset = match v.plain() {
        Value::DateTimeZone(d) => Some(*d.offset()),
        _ => None,
    };
    let (date, time) = parts(v)?.unwrap_or((None, None));
    Ok(Value::text(format_datetime(date, time, offset, &fmt)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_dates_like_dotnet() {
        let d = NaiveDate::from_ymd_opt(2024, 3, 5);
        let t = NaiveTime::from_hms_opt(14, 7, 9);
        assert_eq!(
            format_datetime(d, t, None, "yyyy-MM-dd HH:mm:ss"),
            "2024-03-05 14:07:09"
        );
        assert_eq!(format_datetime(d, t, None, "d"), "3/5/2024");
        assert_eq!(format_datetime(d, t, None, "dddd, MMM d"), "Tuesday, Mar 5");
        assert_eq!(format_datetime(d, t, None, "h:mm tt"), "2:07 PM");
        assert_eq!(format_datetime(d, t, None, "'Q'yy"), "Q24");
    }
}
