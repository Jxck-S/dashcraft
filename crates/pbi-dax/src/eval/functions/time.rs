//! Date/time functions (`EOMONTH`, `DATEDIFF`, `WEEKNUM`, `YEARFRAC`,
//! `NETWORKDAYS`, `CALENDAR`, …)
//! and time intelligence (`DATESYTD`, `TOTALYTD`, `DATEADD`,
//! `SAMEPERIODLASTYEAR`, …).
//!
//! Time-intelligence functions take a `dates` argument — a date column
//! (`'Date'[Date]`, evaluated like `CALCULATETABLE(DISTINCT(col))`, so a
//! row context becomes a filter) or a one-column table with the lineage of
//! a date column — and return a one-column table of dates with that
//! lineage, so the result works as a `CALCULATE` filter. The candidate
//! dates are the column's values over the whole table (`ALL`), as in DAX;
//! periods are computed on whole days. When the column keys a date table,
//! applying the result also removes the other filters on that table (see
//! the `filters` module).

use std::collections::BTreeSet;
use std::sync::Arc;

use arrow::datatypes::{DataType as ArrowType, TimeUnit};
use pbi_engine::value::{MICROS_PER_DAY, civil_from_days, days_from_civil};
use pbi_engine::{Agg, RowSet, Value};
use pbi_model::TableKind;

use super::{Builtin, arg, arity, opt_arg, scalar_fn, table_fn};
use crate::eval::context::Env;
use crate::eval::error::{DaxError, Result};
use crate::eval::interp::{Interp, Val};
use crate::eval::ops;
use crate::eval::table::{VColumn, VirtualTable};
use crate::syntax::{Expr, ExprKind};

pub(super) const BUILTINS: &[Builtin] = &[
    // date/time
    scalar_fn("EOMONTH", eomonth),
    scalar_fn("EDATE", edate),
    scalar_fn("DATEDIFF", datediff),
    scalar_fn("WEEKDAY", weekday),
    scalar_fn("WEEKNUM", weeknum),
    scalar_fn("QUARTER", quarter),
    scalar_fn("YEARFRAC", yearfrac),
    scalar_fn("NETWORKDAYS", networkdays),
    scalar_fn("DATEVALUE", datevalue),
    scalar_fn("TIMEVALUE", timevalue),
    scalar_fn("TIME", time),
    table_fn("CALENDAR", calendar),
    table_fn("CALENDARAUTO", calendarauto),
    // time intelligence
    table_fn("DATESYTD", datesytd),
    table_fn("DATESQTD", datesqtd),
    table_fn("DATESMTD", datesmtd),
    scalar_fn("TOTALYTD", totalytd),
    scalar_fn("TOTALQTD", totalqtd),
    scalar_fn("TOTALMTD", totalmtd),
    table_fn("SAMEPERIODLASTYEAR", sameperiodlastyear),
    table_fn("DATEADD", dateadd),
    table_fn("PARALLELPERIOD", parallelperiod),
    table_fn("PREVIOUSDAY", previousday),
    table_fn("PREVIOUSMONTH", previousmonth),
    table_fn("PREVIOUSQUARTER", previousquarter),
    table_fn("PREVIOUSYEAR", previousyear),
    table_fn("NEXTDAY", nextday),
    table_fn("NEXTMONTH", nextmonth),
    table_fn("NEXTQUARTER", nextquarter),
    table_fn("NEXTYEAR", nextyear),
    table_fn("STARTOFMONTH", startofmonth),
    table_fn("STARTOFQUARTER", startofquarter),
    table_fn("STARTOFYEAR", startofyear),
    table_fn("ENDOFMONTH", endofmonth),
    table_fn("ENDOFQUARTER", endofquarter),
    table_fn("ENDOFYEAR", endofyear),
    table_fn("FIRSTDATE", firstdate),
    table_fn("LASTDATE", lastdate),
    table_fn("DATESBETWEEN", datesbetween),
    table_fn("DATESINPERIOD", datesinperiod),
    table_fn("FIRSTNONBLANK", firstnonblank),
    table_fn("LASTNONBLANK", lastnonblank),
    scalar_fn("FIRSTNONBLANKVALUE", firstnonblankvalue),
    scalar_fn("LASTNONBLANKVALUE", lastnonblankvalue),
    scalar_fn("OPENINGBALANCEMONTH", openingbalancemonth),
    scalar_fn("OPENINGBALANCEQUARTER", openingbalancequarter),
    scalar_fn("OPENINGBALANCEYEAR", openingbalanceyear),
    scalar_fn("CLOSINGBALANCEMONTH", closingbalancemonth),
    scalar_fn("CLOSINGBALANCEQUARTER", closingbalancequarter),
    scalar_fn("CLOSINGBALANCEYEAR", closingbalanceyear),
];

/// Upper bound on the rows of `CALENDAR`/`CALENDARAUTO`.
const MAX_CALENDAR_DAYS: i64 = 1_000_000;

type R = Result<Val>;

fn ok(v: Value) -> R {
    Ok(Val::Scalar(v))
}

// ---- calendar arithmetic (day numbers = days since 1970-01-01) ------------

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Day number of `y`-`m`-`d`; `m` may be out of 1..=12 (normalised) and
/// `d` is clamped to the month's length.
fn day_number(y: i64, m: i64, d: u32) -> Option<i64> {
    let months = y.checked_mul(12)?.checked_add(m - 1)?;
    let (y, m) = (months.div_euclid(12), (months.rem_euclid(12) + 1) as u32);
    days_from_civil(y, m, d.clamp(1, days_in_month(y, m)))
}

fn ymd(day: i64) -> (i64, u32, u32) {
    civil_from_days(day)
}

fn add_months(day: i64, n: i64) -> Option<i64> {
    let (y, m, d) = ymd(day);
    day_number(y, i64::from(m).checked_add(n)?, d)
}

fn month_start(day: i64) -> i64 {
    let (y, m, _) = ymd(day);
    day_number(y, i64::from(m), 1).unwrap_or(day)
}

fn month_end(day: i64) -> i64 {
    let (y, m, _) = ymd(day);
    day_number(y, i64::from(m), 31).unwrap_or(day)
}

fn quarter_start(day: i64) -> i64 {
    let (y, m, _) = ymd(day);
    day_number(y, i64::from((m - 1) / 3 * 3 + 1), 1).unwrap_or(day)
}

fn quarter_end(day: i64) -> i64 {
    add_months(quarter_start(day), 3).map_or(day, |d| d - 1)
}

/// Fiscal year end as `(month, day)`; `(12, 31)` is the calendar year.
type YearEnd = (u32, u32);

const CALENDAR_YEAR: YearEnd = (12, 31);

fn year_start(day: i64, ye: YearEnd) -> i64 {
    let (y, _, _) = ymd(day);
    let end_this = day_number(y, i64::from(ye.0), ye.1).unwrap_or(day);
    if day <= end_this {
        day_number(y - 1, i64::from(ye.0), ye.1).map_or(day, |d| d + 1)
    } else {
        end_this + 1
    }
}

fn year_end(day: i64, ye: YearEnd) -> i64 {
    add_months(year_start(day, ye), 12).map_or(day, |d| d - 1)
}

/// Day of the week, Sunday = 0.
fn dow(day: i64) -> i64 {
    (day + 4).rem_euclid(7)
}

fn day_of(us: i64) -> i64 {
    us.div_euclid(MICROS_PER_DAY)
}

fn date_value(day: i64) -> Value {
    day.checked_mul(MICROS_PER_DAY)
        .map_or(Value::Blank, Value::DateTime)
}

// ---- argument helpers ------------------------------------------------------

/// Date/time value of a scalar argument (`None` for BLANK).
fn datetime_arg(
    i: &Interp<'_>,
    env: &Env,
    f: &str,
    args: &[Expr],
    n: usize,
) -> Result<Option<i64>> {
    ops::to_datetime(&i.scalar(env, arg(f, args, n)?)?)
}

/// Integer argument, rounded (BLANK → 0).
fn int_arg(i: &Interp<'_>, env: &Env, f: &str, args: &[Expr], n: usize) -> Result<i64> {
    let v = i.scalar(env, arg(f, args, n)?)?;
    Ok(ops::to_f64(&v)?.map_or(0, |x| x.round().clamp(-1.0e15, 1.0e15) as i64))
}

/// Integer argument, truncated (BLANK → 0).
fn trunc_arg(i: &Interp<'_>, env: &Env, f: &str, args: &[Expr], n: usize) -> Result<i64> {
    let v = i.scalar(env, arg(f, args, n)?)?;
    Ok(ops::to_f64(&v)?.map_or(0, |x| x.trunc().clamp(-1.0e15, 1.0e15) as i64))
}

/// Day number of the DAX zero date, 1899-12-30 (what BLANK dates mean).
fn zero_day() -> i64 {
    match ops::date_from_serial(0.0) {
        Value::DateTime(us) => day_of(us),
        _ => 0,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Interval {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Quarter,
    Year,
}

fn interval_arg(f: &str, e: &Expr) -> Result<Interval> {
    let word = match &e.unparen().kind {
        ExprKind::Keyword(k) | ExprKind::TableRef { name: k } | ExprKind::String(k) => {
            k.to_ascii_uppercase()
        }
        _ => String::new(),
    };
    Ok(match word.as_str() {
        "SECOND" => Interval::Second,
        "MINUTE" => Interval::Minute,
        "HOUR" => Interval::Hour,
        "DAY" => Interval::Day,
        "WEEK" => Interval::Week,
        "MONTH" => Interval::Month,
        "QUARTER" => Interval::Quarter,
        "YEAR" => Interval::Year,
        _ => {
            return Err(DaxError::argument(
                f,
                "the interval must be SECOND, MINUTE, HOUR, DAY, WEEK, MONTH, QUARTER or YEAR",
            ));
        }
    })
}

/// `day` shifted by `n` days, months, quarters or years (month arithmetic
/// clamps the day to the target month).
fn shift(day: i64, n: i64, interval: Interval) -> Option<i64> {
    match interval {
        Interval::Day => day.checked_add(n),
        Interval::Week => day.checked_add(n.checked_mul(7)?),
        Interval::Month => add_months(day, n),
        Interval::Quarter => add_months(day, n.checked_mul(3)?),
        Interval::Year => add_months(day, n.checked_mul(12)?),
        Interval::Second | Interval::Minute | Interval::Hour => Some(day),
    }
}

/// Fiscal year end of an optional `"6/30"`-style argument.
fn year_end_arg(i: &Interp<'_>, env: &Env, e: Option<&Expr>) -> Result<YearEnd> {
    let Some(e) = e else {
        return Ok(CALENDAR_YEAR);
    };
    let v = i.scalar(env, e)?;
    if v.is_blank() {
        return Ok(CALENDAR_YEAR);
    }
    if let Value::DateTime(us) = v {
        let (_, m, d) = ymd(day_of(us));
        return Ok((m, d));
    }
    let text = ops::to_text(&v);
    let parts: Vec<&str> = text.trim().split(['/', '-']).collect();
    let parsed = match parts.as_slice() {
        [m, d] => m
            .trim()
            .parse::<u32>()
            .ok()
            .zip(d.trim().parse::<u32>().ok()),
        _ => match ops::parse_datetime(&text) {
            Some(Value::DateTime(us)) => {
                let (_, m, d) = ymd(day_of(us));
                Some((m, d))
            }
            _ => None,
        },
    };
    match parsed {
        Some((m, d)) if (1..=12).contains(&m) && (1..=31).contains(&d) => Ok((m, d)),
        _ => Err(DaxError::argument(
            "year_end_date",
            format!("cannot read '{text}' as a month/day such as \"6/30\""),
        )),
    }
}

/// A `dates` argument: the date column it refers to and its dates in the
/// current context.
struct Dates {
    table: String,
    column: String,
    data_type: Option<ArrowType>,
    /// Visible dates (µs), ascending, distinct.
    visible: Vec<i64>,
}

impl Dates {
    fn first_day(&self) -> Option<i64> {
        self.visible.first().map(|us| day_of(*us))
    }

    fn last_day(&self) -> Option<i64> {
        self.visible.last().map(|us| day_of(*us))
    }
}

fn sorted_micros(values: impl IntoIterator<Item = Value>) -> Vec<i64> {
    let set: BTreeSet<i64> = values
        .into_iter()
        .filter_map(|v| match v {
            Value::DateTime(us) => Some(us),
            _ => None,
        })
        .collect();
    set.into_iter().collect()
}

fn dates_arg(i: &Interp<'_>, env: &Env, f: &str, e: &Expr) -> Result<Dates> {
    if let ExprKind::ColumnRef { .. } = e.unparen().kind {
        let (table, column) = i.column_arg(env, e, f)?;
        let filter = i.transition(env)?;
        let values = i.snapshot.values(&filter, &table, &column)?;
        return Ok(Dates {
            data_type: column_type(i, &table, &column),
            visible: sorted_micros(values),
            table,
            column,
        });
    }
    let t = i.table(env, e)?;
    let lineage = match t.columns() {
        [col] => col.lineage.clone(),
        _ => None,
    };
    let Some(lin) = lineage else {
        return Err(DaxError::argument(
            f,
            "the dates argument must be a date column or a one-column table of dates from a column",
        ));
    };
    let values = t.positions().into_iter().map(|p| t.value(p, 0));
    Ok(Dates {
        data_type: column_type(i, &lin.table, &lin.column),
        visible: sorted_micros(values),
        table: lin.table,
        column: lin.column,
    })
}

fn column_type(i: &Interp<'_>, table: &str, column: &str) -> Option<ArrowType> {
    i.snapshot
        .table(table)
        .ok()
        .and_then(|d| d.column_type(column).ok().cloned())
}

/// Every date of the column (all rows), ascending, distinct.
fn domain(i: &Interp<'_>, d: &Dates) -> Result<Vec<i64>> {
    let n = i.snapshot.row_count(&d.table)?;
    Ok(sorted_micros(i.snapshot.distinct(
        &d.table,
        &d.column,
        &RowSet::all(n),
    )?))
}

/// The dates of the column whose day is in `lo..=hi`.
fn between(i: &Interp<'_>, d: &Dates, lo: i64, hi: i64) -> Result<Vec<i64>> {
    Ok(domain(i, d)?
        .into_iter()
        .filter(|us| (lo..=hi).contains(&day_of(*us)))
        .collect())
}

/// A one-column table of dates with the lineage of the `dates` column.
fn dates_table(d: &Dates, micros: Vec<i64>) -> VirtualTable {
    VirtualTable::materialized(
        vec![VColumn::model(&d.table, &d.column, d.data_type.clone())],
        micros
            .into_iter()
            .map(|us| vec![Value::DateTime(us)])
            .collect(),
    )
}

fn table_of(d: &Dates, micros: Vec<i64>) -> R {
    Ok(Val::Table(Arc::new(dates_table(d, micros))))
}

/// The dates of `d`'s column in `range` (computed from the visible dates);
/// an empty table when nothing is visible.
fn period(
    i: &Interp<'_>,
    d: &Dates,
    range: impl FnOnce(i64, i64) -> Option<(i64, i64)>,
) -> Result<Vec<i64>> {
    let (Some(first), Some(last)) = (d.first_day(), d.last_day()) else {
        return Ok(Vec::new());
    };
    match range(first, last) {
        Some((lo, hi)) => between(i, d, lo, hi),
        None => Ok(Vec::new()),
    }
}

// ---- date/time scalars -------------------------------------------------------

fn eomonth(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("EOMONTH", args, 2, 2)?;
    let Some(us) = datetime_arg(i, env, "EOMONTH", args, 0)? else {
        return ok(Value::Blank);
    };
    let months = int_arg(i, env, "EOMONTH", args, 1)?;
    let day = add_months(day_of(us), months)
        .ok_or_else(|| DaxError::argument("EOMONTH", "date out of range"))?;
    ok(date_value(month_end(day)))
}

/// `EDATE(start_date, months)`: `months` is truncated (`EOMONTH` rounds).
fn edate(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("EDATE", args, 2, 2)?;
    let Some(us) = datetime_arg(i, env, "EDATE", args, 0)? else {
        return ok(Value::Blank);
    };
    let months = trunc_arg(i, env, "EDATE", args, 1)?;
    let day = add_months(day_of(us), months)
        .ok_or_else(|| DaxError::argument("EDATE", "date out of range"))?;
    ok(date_value(day))
}

fn datediff(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("DATEDIFF", args, 3, 3)?;
    let (Some(a), Some(b)) = (
        datetime_arg(i, env, "DATEDIFF", args, 0)?,
        datetime_arg(i, env, "DATEDIFF", args, 1)?,
    ) else {
        return ok(Value::Blank);
    };
    let interval = interval_arg("DATEDIFF", arg("DATEDIFF", args, 2)?)?;
    let units = |us: i64, per: i64| us.div_euclid(per);
    let (da, db) = (day_of(a), day_of(b));
    let ((ya, ma, _), (yb, mb, _)) = (ymd(da), ymd(db));
    let months = |y: i64, m: u32| y * 12 + i64::from(m);
    let diff = match interval {
        Interval::Second => units(b, 1_000_000) - units(a, 1_000_000),
        Interval::Minute => units(b, 60_000_000) - units(a, 60_000_000),
        Interval::Hour => units(b, 3_600_000_000) - units(a, 3_600_000_000),
        Interval::Day => db - da,
        Interval::Week => ((db - dow(db)) - (da - dow(da))) / 7,
        Interval::Month => months(yb, mb) - months(ya, ma),
        Interval::Quarter => {
            (yb * 4 + i64::from((mb - 1) / 3)) - (ya * 4 + i64::from((ma - 1) / 3))
        }
        Interval::Year => yb - ya,
    };
    ok(Value::Integer(diff))
}

/// Date argument for date-part functions (BLANK is serial 0, 1899-12-30).
fn date_part_day(i: &Interp<'_>, env: &Env, f: &str, args: &[Expr]) -> Result<i64> {
    let us = match datetime_arg(i, env, f, args, 0)? {
        Some(us) => us,
        None => match ops::date_from_serial(0.0) {
            Value::DateTime(us) => us,
            _ => 0,
        },
    };
    Ok(day_of(us))
}

fn weekday(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("WEEKDAY", args, 1, 2)?;
    let day = date_part_day(i, env, "WEEKDAY", args)?;
    let kind = match opt_arg(args, 1) {
        Some(e) => ops::to_i64(&i.scalar(env, e)?)?.unwrap_or(1),
        None => 1,
    };
    let sun0 = dow(day);
    let mon0 = (sun0 + 6) % 7;
    ok(Value::Integer(match kind {
        1 => sun0 + 1,
        2 => mon0 + 1,
        3 => mon0,
        _ => {
            return Err(DaxError::argument(
                "WEEKDAY",
                "the return type must be 1, 2 or 3",
            ));
        }
    }))
}

/// ISO 8601 week number.
fn iso_week(day: i64) -> i64 {
    let (y, _, _) = ymd(day);
    let jan1 = day_number(y, 1, 1).unwrap_or(day);
    let doy = day - jan1 + 1;
    let wd = (dow(day) + 6) % 7 + 1;
    let week = (doy - wd + 10) / 7;
    let weeks_in = |year: i64| {
        let j1 = day_number(year, 1, 1).map_or(0, dow);
        if j1 == 4 || (j1 == 3 && is_leap(year)) {
            53
        } else {
            52
        }
    };
    if week < 1 {
        weeks_in(y - 1)
    } else if week > weeks_in(y) {
        1
    } else {
        week
    }
}

fn weeknum(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("WEEKNUM", args, 1, 2)?;
    let day = date_part_day(i, env, "WEEKNUM", args)?;
    let kind = match opt_arg(args, 1) {
        Some(e) => ops::to_i64(&i.scalar(env, e)?)?.unwrap_or(1),
        None => 1,
    };
    let start = match kind {
        1 | 17 => 0,
        2 | 11 => 1,
        12..=16 => kind - 10,
        21 => return ok(Value::Integer(iso_week(day))),
        _ => {
            return Err(DaxError::argument(
                "WEEKNUM",
                "the return type must be 1, 2, 11 to 17 or 21",
            ));
        }
    };
    let (y, _, _) = ymd(day);
    let jan1 = day_number(y, 1, 1).unwrap_or(day);
    let offset = (dow(jan1) - start).rem_euclid(7);
    ok(Value::Integer((day - jan1 + offset) / 7 + 1))
}

/// `QUARTER(date)`: 1–4; BLANK for a BLANK date.
fn quarter(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("QUARTER", args, 1, 1)?;
    let Some(us) = datetime_arg(i, env, "QUARTER", args, 0)? else {
        return ok(Value::Blank);
    };
    let (_, m, _) = ymd(day_of(us));
    ok(Value::Integer(i64::from((m - 1) / 3 + 1)))
}

/// Day number of a date argument; BLANK is the zero date.
fn day_arg(i: &Interp<'_>, env: &Env, f: &str, args: &[Expr], n: usize) -> Result<i64> {
    Ok(datetime_arg(i, env, f, args, n)?.map_or_else(zero_day, day_of))
}

/// 30/360 day count: US (NASD) rules as Excel's `YEARFRAC` basis 0
/// applies them, or European (basis 4).
fn days_360(start: i64, end: i64, european: bool) -> i64 {
    let (y1, m1, d1) = ymd(start);
    let (y2, m2, d2) = ymd(end);
    let (mut d1, mut d2) = (i64::from(d1), i64::from(d2));
    if european {
        d1 = d1.min(30);
        d2 = d2.min(30);
    } else {
        let last_of_feb = |y: i64, m: u32, d: i64| m == 2 && d == i64::from(days_in_month(y, 2));
        let (feb1, feb2) = (last_of_feb(y1, m1, d1), last_of_feb(y2, m2, d2));
        if d1 == 31 && d2 == 31 {
            d1 = 30;
            d2 = 30;
        } else if d1 == 31 {
            d1 = 30;
        } else if d1 == 30 && d2 == 31 {
            d2 = 30;
        } else if feb1 && feb2 {
            d1 = 30;
            d2 = 30;
        } else if feb1 {
            d1 = 30;
        }
    }
    (y2 - y1) * 360 + (i64::from(m2) - i64::from(m1)) * 30 + (d2 - d1)
}

/// Actual/actual year fraction (basis 1): the length of the year (365 or
/// 366) when the dates are at most a year apart, else the average length
/// of the years spanned.
fn actual_actual(start: i64, end: i64) -> f64 {
    let days = (end - start) as f64;
    let (y1, m1, d1) = ymd(start);
    let (y2, m2, d2) = ymd(end);
    let year_len = |y: i64| if is_leap(y) { 366.0 } else { 365.0 };
    let within_a_year = y1 == y2 || (y2 == y1 + 1 && (m1 > m2 || (m1 == m2 && d1 >= d2)));
    if !within_a_year {
        let years = (y2 - y1 + 1) as f64;
        let total: f64 = (y1..=y2).map(year_len).sum();
        return days / (total / years);
    }
    let march_1 = |y: i64| day_number(y, 3, 1).unwrap_or(i64::MIN);
    let spans_feb_29 = (is_leap(y1) && start < march_1(y1) && end >= march_1(y1))
        || (is_leap(y2) && end >= march_1(y2) && start < march_1(y2))
        || (m2 == 2 && d2 == 29);
    let len = if (y1 == y2 && is_leap(y1)) || spans_feb_29 {
        366.0
    } else {
        365.0
    };
    days / len
}

/// `YEARFRAC(start_date, end_date[, basis = 0])`: 0 US 30/360, 1
/// actual/actual, 2 actual/360, 3 actual/365, 4 European 30/360. Whole
/// days; the dates may come in either order.
fn yearfrac(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("YEARFRAC", args, 2, 3)?;
    let a = day_arg(i, env, "YEARFRAC", args, 0)?;
    let b = day_arg(i, env, "YEARFRAC", args, 1)?;
    let basis = match opt_arg(args, 2) {
        Some(e) => ops::to_f64(&i.scalar(env, e)?)?.unwrap_or(0.0).trunc(),
        None => 0.0,
    };
    if !(0.0..=4.0).contains(&basis) {
        return Err(DaxError::argument(
            "YEARFRAC",
            "the basis must be 0, 1, 2, 3 or 4",
        ));
    }
    let (start, end) = if a <= b { (a, b) } else { (b, a) };
    let days = (end - start) as f64;
    let frac = match basis as i64 {
        1 => actual_actual(start, end),
        2 => days / 360.0,
        3 => days / 365.0,
        4 => days_360(start, end, true) as f64 / 360.0,
        _ => days_360(start, end, false) as f64 / 360.0,
    };
    ok(Value::Double(frac))
}

/// Weekend days (index: Sunday = 0) of a `NETWORKDAYS` weekend argument:
/// codes 1–7 (two days: 1 = Saturday/Sunday … 7 = Friday/Saturday), 11–17
/// (one day: 11 = Sunday … 17 = Saturday), or a 7-character `0`/`1` mask
/// starting on Monday.
fn weekend_arg(v: &Value) -> Result<[bool; 7]> {
    let bad = || {
        DaxError::argument(
            "NETWORKDAYS",
            "the weekend must be 1 to 7, 11 to 17 or a 7-character mask of 0 and 1",
        )
    };
    let mut days = [false; 7];
    if let Value::String(mask) = v
        && mask.chars().count() == 7
        && mask.chars().all(|c| c == '0' || c == '1')
    {
        for (j, c) in mask.chars().enumerate() {
            days[(j + 1) % 7] = c == '1';
        }
        if days.iter().all(|d| *d) {
            return Err(bad());
        }
        return Ok(days);
    }
    let code = ops::to_f64(v)?.unwrap_or(1.0);
    match code {
        c if (1.0..=7.0).contains(&c) && c.fract() == 0.0 => {
            let k = c as usize;
            days[(k + 5) % 7] = true;
            days[(k + 6) % 7] = true;
        }
        c if (11.0..=17.0).contains(&c) && c.fract() == 0.0 => days[c as usize - 11] = true,
        _ => return Err(bad()),
    }
    Ok(days)
}

/// Working days in `lo..=hi` (`lo ≤ hi`) for the weekend pattern.
fn workdays(lo: i64, hi: i64, weekend: &[bool; 7]) -> i64 {
    let total = hi - lo + 1;
    let per_week = weekend.iter().filter(|w| !**w).count() as i64;
    let (weeks, rest) = (total / 7, total % 7);
    let tail_start = lo + weeks * 7;
    let tail = (0..rest)
        .filter(|j| !weekend[dow(tail_start + j) as usize])
        .count() as i64;
    weeks * per_week + tail
}

/// `NETWORKDAYS(start_date, end_date[, weekend[, holidays]])`: working days
/// between the dates, both included (negative when the end is earlier).
/// BLANK when both dates are BLANK; one BLANK date is 1899-12-30.
fn networkdays(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    const F: &str = "NETWORKDAYS";
    arity(F, args, 2, 4)?;
    let a = datetime_arg(i, env, F, args, 0)?;
    let b = datetime_arg(i, env, F, args, 1)?;
    if a.is_none() && b.is_none() {
        return ok(Value::Blank);
    }
    let start = a.map_or_else(zero_day, day_of);
    let end = b.map_or_else(zero_day, day_of);
    let weekend = match opt_arg(args, 2) {
        Some(e) => weekend_arg(&i.scalar(env, e)?)?,
        None => weekend_arg(&Value::Integer(1))?,
    };
    let (lo, hi) = (start.min(end), start.max(end));
    let mut holidays = BTreeSet::new();
    if let Some(e) = opt_arg(args, 3) {
        let t = i.table(env, e)?;
        if t.column_count() != 1 {
            return Err(DaxError::argument(
                F,
                "the holidays must be a one-column table of dates",
            ));
        }
        for p in t.positions() {
            if let Some(us) = ops::to_datetime(&t.value(p, 0))? {
                let d = day_of(us);
                if (lo..=hi).contains(&d) && !weekend[dow(d) as usize] {
                    holidays.insert(d);
                }
            }
        }
    }
    let count = workdays(lo, hi, &weekend) - holidays.len() as i64;
    ok(Value::Integer(if start <= end { count } else { -count }))
}

fn datevalue(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("DATEVALUE", args, 1, 1)?;
    let v = i.scalar(env, arg("DATEVALUE", args, 0)?)?;
    ok(match ops::to_datetime(&v)? {
        Some(us) => date_value(day_of(us)),
        None => Value::Blank,
    })
}

/// Time of day (µs) of a `hh:mm[:ss] [AM|PM]` text or a date/time text.
fn parse_time(text: &str) -> Option<i64> {
    let parsed =
        ops::parse_datetime(text).or_else(|| ops::parse_datetime(&format!("1899-12-30 {text}")));
    match parsed {
        Some(Value::DateTime(us)) => Some(us.rem_euclid(MICROS_PER_DAY)),
        _ => None,
    }
}

/// A time of day as a date/time on the DAX zero date (1899-12-30).
fn time_value(micros_of_day: i64) -> Value {
    let base = match ops::date_from_serial(0.0) {
        Value::DateTime(us) => us,
        _ => 0,
    };
    Value::DateTime(base + micros_of_day)
}

fn timevalue(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("TIMEVALUE", args, 1, 1)?;
    let v = i.scalar(env, arg("TIMEVALUE", args, 0)?)?;
    ok(match &v {
        Value::Blank => Value::Blank,
        Value::DateTime(us) => time_value(us.rem_euclid(MICROS_PER_DAY)),
        Value::String(s) => match parse_time(s) {
            Some(t) => time_value(t),
            None => {
                return Err(DaxError::type_error(format!(
                    "Cannot convert value '{s}' of type Text to type Date."
                )));
            }
        },
        other => match ops::to_f64(other)? {
            Some(x) => time_value(((x.fract() * 86_400.0).round() as i64) * 1_000_000),
            None => Value::Blank,
        },
    })
}

fn time(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("TIME", args, 3, 3)?;
    let h = int_arg(i, env, "TIME", args, 0)?;
    let m = int_arg(i, env, "TIME", args, 1)?;
    let s = int_arg(i, env, "TIME", args, 2)?;
    let secs = (h * 3600 + m * 60 + s).rem_euclid(86_400);
    ok(time_value(secs * 1_000_000))
}

fn calendar_table(start: i64, end: i64) -> Result<VirtualTable> {
    if end < start {
        return Err(DaxError::argument(
            "CALENDAR",
            "the end date must not be before the start date",
        ));
    }
    if end - start >= MAX_CALENDAR_DAYS {
        return Err(DaxError::evaluation("CALENDAR: the result is too large"));
    }
    let mut col = VColumn::new("Date");
    col.data_type = Some(ArrowType::Timestamp(TimeUnit::Microsecond, None));
    Ok(VirtualTable::materialized(
        vec![col],
        (start..=end).map(|d| vec![date_value(d)]).collect(),
    ))
}

fn calendar(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("CALENDAR", args, 2, 2)?;
    let (Some(a), Some(b)) = (
        datetime_arg(i, env, "CALENDAR", args, 0)?,
        datetime_arg(i, env, "CALENDAR", args, 1)?,
    ) else {
        return Err(DaxError::argument("CALENDAR", "the dates cannot be blank"));
    };
    Ok(Val::Table(Arc::new(calendar_table(day_of(a), day_of(b))?)))
}

/// `CALENDARAUTO([fiscal_year_end_month])`: whole (fiscal) years covering
/// every date/time column of the model, except calculated columns and
/// calculated tables.
fn calendarauto(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("CALENDARAUTO", args, 0, 1)?;
    let end_month = match opt_arg(args, 0) {
        Some(e) => ops::to_i64(&i.scalar(env, e)?)?.unwrap_or(12),
        None => 12,
    };
    let end_month = u32::try_from(end_month)
        .ok()
        .filter(|m| (1..=12).contains(m))
        .ok_or_else(|| DaxError::argument("CALENDARAUTO", "the month must be 1 to 12"))?;
    let (mut lo, mut hi): (Option<i64>, Option<i64>) = (None, None);
    for name in i.snapshot.table_names() {
        let model_table = i.model.table(&name);
        if model_table.is_some_and(|t| t.kind() == TableKind::CalculatedTable) {
            continue;
        }
        let data = i.snapshot.table(&name)?;
        let all = RowSet::all(data.row_count());
        let columns: Vec<String> = data.column_names().map(str::to_owned).collect();
        for c in columns {
            if !matches!(data.column_type(&c), Ok(ArrowType::Timestamp(_, _))) {
                continue;
            }
            if model_table
                .and_then(|t| t.column(&c))
                .is_some_and(|mc| mc.is_calculated())
            {
                continue;
            }
            for (agg, slot, pick) in [
                (Agg::Min, &mut lo, i64::min as fn(i64, i64) -> i64),
                (Agg::Max, &mut hi, i64::max),
            ] {
                if let Value::DateTime(us) = i.snapshot.aggregate(data.name(), &c, &all, agg)? {
                    let d = day_of(us);
                    *slot = Some(slot.map_or(d, |cur| pick(cur, d)));
                }
            }
        }
    }
    let (Some(lo), Some(hi)) = (lo, hi) else {
        return Err(DaxError::evaluation(
            "CALENDARAUTO: the model has no date columns with values",
        ));
    };
    let ye = (end_month, 31);
    let table = calendar_table(year_start(lo, ye), year_end(hi, ye))?;
    Ok(Val::Table(Arc::new(table)))
}

// ---- time intelligence -------------------------------------------------------

#[derive(Clone, Copy)]
enum Grain {
    Month,
    Quarter,
    Year(YearEnd),
}

fn grain_start(day: i64, g: Grain) -> i64 {
    match g {
        Grain::Month => month_start(day),
        Grain::Quarter => quarter_start(day),
        Grain::Year(ye) => year_start(day, ye),
    }
}

fn grain_end(day: i64, g: Grain) -> i64 {
    match g {
        Grain::Month => month_end(day),
        Grain::Quarter => quarter_end(day),
        Grain::Year(ye) => year_end(day, ye),
    }
}

fn grain_months(g: Grain) -> i64 {
    match g {
        Grain::Month => 1,
        Grain::Quarter => 3,
        Grain::Year(_) => 12,
    }
}

/// `DATESMTD`/`DATESQTD`/`DATESYTD`: from the start of the period of the
/// last visible date up to that date.
fn to_date(
    i: &Interp<'_>,
    env: &Env,
    f: &str,
    dates: &Expr,
    g: Grain,
) -> Result<(Dates, Vec<i64>)> {
    let d = dates_arg(i, env, f, dates)?;
    let days = period(i, &d, |_, last| Some((grain_start(last, g), last)))?;
    Ok((d, days))
}

fn datesytd(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("DATESYTD", args, 1, 2)?;
    let ye = year_end_arg(i, env, opt_arg(args, 1))?;
    let (d, days) = to_date(
        i,
        env,
        "DATESYTD",
        arg("DATESYTD", args, 0)?,
        Grain::Year(ye),
    )?;
    table_of(&d, days)
}

fn datesqtd(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("DATESQTD", args, 1, 1)?;
    let (d, days) = to_date(
        i,
        env,
        "DATESQTD",
        arg("DATESQTD", args, 0)?,
        Grain::Quarter,
    )?;
    table_of(&d, days)
}

fn datesmtd(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("DATESMTD", args, 1, 1)?;
    let (d, days) = to_date(i, env, "DATESMTD", arg("DATESMTD", args, 0)?, Grain::Month)?;
    table_of(&d, days)
}

/// `CALCULATE(expr, <tables as filters>, extra filter arguments…)`.
fn calculate_with(
    i: &Interp<'_>,
    env: &Env,
    f: &str,
    expr: &Expr,
    tables: &[VirtualTable],
    extra: &[Expr],
) -> R {
    let mut fa = i.collect_filter_args(env, extra, f)?;
    for t in tables {
        for spec in i.table_filters(t)? {
            fa.filters.push((spec, false));
        }
    }
    let base = i.transition(env)?;
    let filter = Arc::new(i.apply_filter_args(&base, fa));
    ok(i.scalar(&env.with_filter(filter), expr)?)
}

/// `TOTALxTD(expr, dates[, filter][, year_end])`.
fn total_to_date(i: &Interp<'_>, env: &Env, args: &[Expr], f: &str, g: Grain) -> R {
    let max = if matches!(g, Grain::Year(_)) { 4 } else { 3 };
    arity(f, args, 2, max)?;
    let (filter, ye) = match (opt_arg(args, 2), opt_arg(args, 3)) {
        (Some(e), None)
            if matches!(g, Grain::Year(_)) && matches!(e.unparen().kind, ExprKind::String(_)) =>
        {
            (None, Some(e))
        }
        (a, b) => (a, b),
    };
    let g = match g {
        Grain::Year(_) => Grain::Year(year_end_arg(i, env, ye)?),
        other => other,
    };
    let (d, days) = to_date(i, env, f, arg(f, args, 1)?, g)?;
    let extra: Vec<Expr> = filter.into_iter().cloned().collect();
    calculate_with(
        i,
        env,
        f,
        arg(f, args, 0)?,
        &[dates_table(&d, days)],
        &extra,
    )
}

fn totalytd(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    total_to_date(i, env, args, "TOTALYTD", Grain::Year(CALENDAR_YEAR))
}

fn totalqtd(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    total_to_date(i, env, args, "TOTALQTD", Grain::Quarter)
}

fn totalmtd(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    total_to_date(i, env, args, "TOTALMTD", Grain::Month)
}

/// `DATEADD`: every visible date shifted by `n` intervals. Month-based
/// shifts of a month's last day extend to the end of the target month, so
/// a whole month maps to the whole target month.
fn shifted(i: &Interp<'_>, d: &Dates, n: i64, interval: Interval) -> Result<Vec<i64>> {
    let mut days = BTreeSet::new();
    for us in &d.visible {
        let day = day_of(*us);
        let Some(s) = shift(day, n, interval) else {
            continue;
        };
        days.insert(s);
        if matches!(
            interval,
            Interval::Month | Interval::Quarter | Interval::Year
        ) && day == month_end(day)
        {
            days.extend(s..=month_end(s));
        }
    }
    Ok(domain(i, d)?
        .into_iter()
        .filter(|us| days.contains(&day_of(*us)))
        .collect())
}

fn dateadd(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("DATEADD", args, 3, 3)?;
    let d = dates_arg(i, env, "DATEADD", arg("DATEADD", args, 0)?)?;
    let n = int_arg(i, env, "DATEADD", args, 1)?;
    let interval = interval_arg("DATEADD", arg("DATEADD", args, 2)?)?;
    if !matches!(
        interval,
        Interval::Day | Interval::Month | Interval::Quarter | Interval::Year
    ) {
        return Err(DaxError::argument(
            "DATEADD",
            "the interval must be DAY, MONTH, QUARTER or YEAR",
        ));
    }
    let days = shifted(i, &d, n, interval)?;
    table_of(&d, days)
}

fn sameperiodlastyear(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("SAMEPERIODLASTYEAR", args, 1, 1)?;
    let d = dates_arg(
        i,
        env,
        "SAMEPERIODLASTYEAR",
        arg("SAMEPERIODLASTYEAR", args, 0)?,
    )?;
    let days = shifted(i, &d, -1, Interval::Year)?;
    table_of(&d, days)
}

fn parallelperiod(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("PARALLELPERIOD", args, 3, 3)?;
    let d = dates_arg(i, env, "PARALLELPERIOD", arg("PARALLELPERIOD", args, 0)?)?;
    let n = int_arg(i, env, "PARALLELPERIOD", args, 1)?;
    let g = match interval_arg("PARALLELPERIOD", arg("PARALLELPERIOD", args, 2)?)? {
        Interval::Month => Grain::Month,
        Interval::Quarter => Grain::Quarter,
        Interval::Year => Grain::Year(CALENDAR_YEAR),
        _ => {
            return Err(DaxError::argument(
                "PARALLELPERIOD",
                "the interval must be MONTH, QUARTER or YEAR",
            ));
        }
    };
    let months = n.saturating_mul(grain_months(g));
    let days = period(i, &d, |first, last| {
        Some((
            grain_start(add_months(first, months)?, g),
            grain_end(add_months(last, months)?, g),
        ))
    })?;
    table_of(&d, days)
}

/// `PREVIOUS*`/`NEXT*`: the whole period before the first (after the
/// last) visible date.
fn adjacent(i: &Interp<'_>, env: &Env, args: &[Expr], f: &str, g: Option<Grain>, next: bool) -> R {
    let max = if matches!(g, Some(Grain::Year(_))) {
        2
    } else {
        1
    };
    arity(f, args, 1, max)?;
    let g = match g {
        Some(Grain::Year(_)) => Some(Grain::Year(year_end_arg(i, env, opt_arg(args, 1))?)),
        other => other,
    };
    let d = dates_arg(i, env, f, arg(f, args, 0)?)?;
    let days = period(i, &d, |first, last| match g {
        None if next => Some((last + 1, last + 1)),
        None => Some((first - 1, first - 1)),
        Some(g) => {
            let anchor = if next {
                grain_end(last, g) + 1
            } else {
                grain_start(first, g) - 1
            };
            Some((grain_start(anchor, g), grain_end(anchor, g)))
        }
    })?;
    table_of(&d, days)
}

fn previousday(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    adjacent(i, env, args, "PREVIOUSDAY", None, false)
}

fn previousmonth(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    adjacent(i, env, args, "PREVIOUSMONTH", Some(Grain::Month), false)
}

fn previousquarter(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    adjacent(i, env, args, "PREVIOUSQUARTER", Some(Grain::Quarter), false)
}

fn previousyear(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    adjacent(
        i,
        env,
        args,
        "PREVIOUSYEAR",
        Some(Grain::Year(CALENDAR_YEAR)),
        false,
    )
}

fn nextday(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    adjacent(i, env, args, "NEXTDAY", None, true)
}

fn nextmonth(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    adjacent(i, env, args, "NEXTMONTH", Some(Grain::Month), true)
}

fn nextquarter(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    adjacent(i, env, args, "NEXTQUARTER", Some(Grain::Quarter), true)
}

fn nextyear(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    adjacent(
        i,
        env,
        args,
        "NEXTYEAR",
        Some(Grain::Year(CALENDAR_YEAR)),
        true,
    )
}

/// `STARTOF*` (first date of the period of the first visible date) and
/// `ENDOF*` (last date of the period of the last visible date).
fn boundary_dates(i: &Interp<'_>, d: &Dates, g: Grain, end: bool) -> Result<Vec<i64>> {
    let days = period(i, d, |first, last| {
        if end {
            Some((grain_start(last, g), grain_end(last, g)))
        } else {
            Some((grain_start(first, g), grain_end(first, g)))
        }
    })?;
    let pick = if end { days.last() } else { days.first() };
    Ok(pick.copied().into_iter().collect())
}

fn boundary(i: &Interp<'_>, env: &Env, args: &[Expr], f: &str, g: Grain, end: bool) -> R {
    let max = if matches!(g, Grain::Year(_)) { 2 } else { 1 };
    arity(f, args, 1, max)?;
    let g = match g {
        Grain::Year(_) => Grain::Year(year_end_arg(i, env, opt_arg(args, 1))?),
        other => other,
    };
    let d = dates_arg(i, env, f, arg(f, args, 0)?)?;
    let days = boundary_dates(i, &d, g, end)?;
    table_of(&d, days)
}

fn startofmonth(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    boundary(i, env, args, "STARTOFMONTH", Grain::Month, false)
}

fn startofquarter(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    boundary(i, env, args, "STARTOFQUARTER", Grain::Quarter, false)
}

fn startofyear(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    boundary(
        i,
        env,
        args,
        "STARTOFYEAR",
        Grain::Year(CALENDAR_YEAR),
        false,
    )
}

fn endofmonth(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    boundary(i, env, args, "ENDOFMONTH", Grain::Month, true)
}

fn endofquarter(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    boundary(i, env, args, "ENDOFQUARTER", Grain::Quarter, true)
}

fn endofyear(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    boundary(i, env, args, "ENDOFYEAR", Grain::Year(CALENDAR_YEAR), true)
}

fn firstdate(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("FIRSTDATE", args, 1, 1)?;
    let d = dates_arg(i, env, "FIRSTDATE", arg("FIRSTDATE", args, 0)?)?;
    let first = d.visible.first().copied().into_iter().collect();
    table_of(&d, first)
}

fn lastdate(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("LASTDATE", args, 1, 1)?;
    let d = dates_arg(i, env, "LASTDATE", arg("LASTDATE", args, 0)?)?;
    let last = d.visible.last().copied().into_iter().collect();
    table_of(&d, last)
}

/// The column of a `dates` argument without computing visible dates.
fn dates_column(i: &Interp<'_>, env: &Env, f: &str, e: &Expr) -> Result<Dates> {
    if let ExprKind::ColumnRef { .. } = e.unparen().kind {
        let (table, column) = i.column_arg(env, e, f)?;
        return Ok(Dates {
            data_type: column_type(i, &table, &column),
            visible: Vec::new(),
            table,
            column,
        });
    }
    dates_arg(i, env, f, e)
}

fn datesbetween(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("DATESBETWEEN", args, 3, 3)?;
    let d = dates_column(i, env, "DATESBETWEEN", arg("DATESBETWEEN", args, 0)?)?;
    let lo = datetime_arg(i, env, "DATESBETWEEN", args, 1)?.map_or(i64::MIN, day_of);
    let hi = datetime_arg(i, env, "DATESBETWEEN", args, 2)?.map_or(i64::MAX, day_of);
    let days = between(i, &d, lo, hi)?;
    table_of(&d, days)
}

fn datesinperiod(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("DATESINPERIOD", args, 4, 4)?;
    let d = dates_column(i, env, "DATESINPERIOD", arg("DATESINPERIOD", args, 0)?)?;
    let Some(start) = datetime_arg(i, env, "DATESINPERIOD", args, 1)?.map(day_of) else {
        return table_of(&d, Vec::new());
    };
    let n = int_arg(i, env, "DATESINPERIOD", args, 2)?;
    let interval = interval_arg("DATESINPERIOD", arg("DATESINPERIOD", args, 3)?)?;
    let end = shift(start, n, interval)
        .ok_or_else(|| DaxError::argument("DATESINPERIOD", "date out of range"))?;
    let days = match n.signum() {
        1 => between(i, &d, start, end - 1)?,
        -1 => between(i, &d, end + 1, start)?,
        _ => Vec::new(),
    };
    table_of(&d, days)
}

/// `FIRSTNONBLANK`/`LASTNONBLANK(column, expression)`: the first/last
/// visible value of the column (ascending) for which the expression,
/// evaluated with that value in row context, is not BLANK. Returns the
/// value and the expression's result.
fn nonblank(
    i: &Interp<'_>,
    env: &Env,
    args: &[Expr],
    f: &str,
    last: bool,
) -> Result<Option<(VirtualTable, Value)>> {
    arity(f, args, 2, 2)?;
    let (t, c) = i.column_arg(env, arg(f, args, 0)?, f)?;
    let expr = arg(f, args, 1)?;
    let filter = i.transition(env)?;
    let mut values: Vec<Value> = i
        .snapshot
        .values(&filter, &t, &c)?
        .into_iter()
        .filter(|v| !v.is_blank())
        .collect();
    values.sort_by(Value::total_cmp);
    if last {
        values.reverse();
    }
    let dt = column_type(i, &t, &c);
    let column = VColumn::model(&t, &c, dt);
    let table = Arc::new(VirtualTable::materialized(
        vec![column.clone()],
        values.into_iter().map(|v| vec![v]).collect(),
    ));
    let mut found = None;
    i.iterate(env, &table, |row_env, pos| {
        let v = i.scalar(row_env, expr)?;
        if v.is_blank() {
            return Ok(true);
        }
        found = Some((
            VirtualTable::materialized(vec![column.clone()], vec![vec![table.value(pos, 0)]]),
            v,
        ));
        Ok(false)
    })?;
    Ok(found)
}

fn nonblank_table(i: &Interp<'_>, env: &Env, args: &[Expr], f: &str, last: bool) -> R {
    match nonblank(i, env, args, f, last)? {
        Some((t, _)) => Ok(Val::Table(Arc::new(t))),
        None => {
            let (t, c) = i.column_arg(env, arg(f, args, 0)?, f)?;
            let dt = column_type(i, &t, &c);
            Ok(Val::Table(Arc::new(VirtualTable::empty(vec![
                VColumn::model(&t, &c, dt),
            ]))))
        }
    }
}

fn firstnonblank(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    nonblank_table(i, env, args, "FIRSTNONBLANK", false)
}

fn lastnonblank(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    nonblank_table(i, env, args, "LASTNONBLANK", true)
}

fn firstnonblankvalue(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    ok(nonblank(i, env, args, "FIRSTNONBLANKVALUE", false)?.map_or(Value::Blank, |(_, v)| v))
}

fn lastnonblankvalue(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    ok(nonblank(i, env, args, "LASTNONBLANKVALUE", true)?.map_or(Value::Blank, |(_, v)| v))
}

/// `OPENINGBALANCE*` (the day before the period of the first visible
/// date) and `CLOSINGBALANCE*` (the last date of the period of the last
/// visible date): `CALCULATE(expr, <that date>, filter)`.
fn balance(i: &Interp<'_>, env: &Env, args: &[Expr], f: &str, g: Grain, closing: bool) -> R {
    let max = if matches!(g, Grain::Year(_)) { 4 } else { 3 };
    arity(f, args, 2, max)?;
    let (filter, ye) = match (opt_arg(args, 2), opt_arg(args, 3)) {
        (Some(e), None)
            if matches!(g, Grain::Year(_)) && matches!(e.unparen().kind, ExprKind::String(_)) =>
        {
            (None, Some(e))
        }
        (a, b) => (a, b),
    };
    let g = match g {
        Grain::Year(_) => Grain::Year(year_end_arg(i, env, ye)?),
        other => other,
    };
    let d = dates_arg(i, env, f, arg(f, args, 1)?)?;
    let days = if closing {
        boundary_dates(i, &d, g, true)?
    } else {
        period(i, &d, |first, _| {
            let day = grain_start(first, g) - 1;
            Some((day, day))
        })?
    };
    let extra: Vec<Expr> = filter.into_iter().cloned().collect();
    calculate_with(
        i,
        env,
        f,
        arg(f, args, 0)?,
        &[dates_table(&d, days)],
        &extra,
    )
}

fn openingbalancemonth(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    balance(i, env, args, "OPENINGBALANCEMONTH", Grain::Month, false)
}

fn openingbalancequarter(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    balance(i, env, args, "OPENINGBALANCEQUARTER", Grain::Quarter, false)
}

fn openingbalanceyear(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    balance(
        i,
        env,
        args,
        "OPENINGBALANCEYEAR",
        Grain::Year(CALENDAR_YEAR),
        false,
    )
}

fn closingbalancemonth(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    balance(i, env, args, "CLOSINGBALANCEMONTH", Grain::Month, true)
}

fn closingbalancequarter(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    balance(i, env, args, "CLOSINGBALANCEQUARTER", Grain::Quarter, true)
}

fn closingbalanceyear(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    balance(
        i,
        env,
        args,
        "CLOSINGBALANCEYEAR",
        Grain::Year(CALENDAR_YEAR),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i64, m: u32, d: u32) -> i64 {
        days_from_civil(y, m, d).unwrap()
    }

    #[test]
    fn calendar_arithmetic() {
        assert_eq!(add_months(day(2024, 1, 31), 1), Some(day(2024, 2, 29)));
        assert_eq!(add_months(day(2024, 3, 31), -13), Some(day(2023, 2, 28)));
        assert_eq!(month_end(day(2023, 2, 3)), day(2023, 2, 28));
        assert_eq!(quarter_start(day(2024, 5, 9)), day(2024, 4, 1));
        assert_eq!(quarter_end(day(2024, 5, 9)), day(2024, 6, 30));
        assert_eq!(year_start(day(2024, 5, 9), CALENDAR_YEAR), day(2024, 1, 1));
        assert_eq!(year_start(day(2024, 5, 9), (6, 30)), day(2023, 7, 1));
        assert_eq!(year_start(day(2024, 7, 1), (6, 30)), day(2024, 7, 1));
        assert_eq!(year_end(day(2024, 7, 1), (6, 30)), day(2025, 6, 30));
        assert_eq!(dow(day(2024, 1, 7)), 0);
        assert_eq!(iso_week(day(2021, 1, 3)), 53);
        assert_eq!(iso_week(day(2024, 12, 30)), 1);
        assert_eq!(iso_week(day(2024, 6, 15)), 24);
    }
}
