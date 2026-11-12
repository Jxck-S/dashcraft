//! Relative date and time conditions: `RelativeDate` / `RelativeTime`
//! filter cards and relative slicers ("Last 3 months", "This year",
//! "Next 2 hours").
//!
//! # Semantic-query encoding
//!
//! A relative condition is a `Between` (or comparison) whose bounds are
//! date expressions instead of literals:
//!
//! - `Now: {}`: the current local date and time (the session clock,
//!   `--now` in the CLI);
//! - `DateAdd: {Expression, Amount, TimeUnit}`: a moment plus `Amount`
//!   units;
//! - `DateSpan: {Expression, TimeUnit}`: the whole unit (day, Sunday-based
//!   week, month, year, …) containing a moment.
//!
//! `TimeUnit` codes: 0 day, 1 week, 2 month, 3 year, 4 decade, 5 second,
//! 6 minute, 7 hour. A span used as a lower bound starts at the span's
//! start and as an upper bound ends (exclusive) at its end, so
//! `Between(Date, DateSpan(Day, Now - 9 days), DateSpan(Day, Now))` keeps
//! the last ten days including all of today. [`resolve`] rewrites such
//! bounds into `datetime'…'` literals for a given "now";
//! [`RelativeDateSpec::condition`] builds the condition Desktop writes for
//! a relative slicer setting and [`RelativeDateSpec::from_condition`] reads
//! it back.

use chrono::{Datelike, Months, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Timelike};
use pbi_model::{ComparisonKind, Condition, FieldExpr, Literal, LiteralValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

/// Semantic-query `TimeUnit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimeUnit {
    Day,
    Week,
    Month,
    Year,
    Decade,
    Second,
    Minute,
    Hour,
}

impl TimeUnit {
    /// The unit of a `TimeUnit` code.
    pub fn from_code(code: i64) -> Option<Self> {
        Some(match code {
            0 => Self::Day,
            1 => Self::Week,
            2 => Self::Month,
            3 => Self::Year,
            4 => Self::Decade,
            5 => Self::Second,
            6 => Self::Minute,
            7 => Self::Hour,
            _ => return None,
        })
    }

    /// The `TimeUnit` code.
    pub fn code(self) -> i64 {
        match self {
            Self::Day => 0,
            Self::Week => 1,
            Self::Month => 2,
            Self::Year => 3,
            Self::Decade => 4,
            Self::Second => 5,
            Self::Minute => 6,
            Self::Hour => 7,
        }
    }
}

/// Last, next or this period (`data.relativeRange`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RelativeAnchor {
    #[default]
    Last,
    Next,
    This,
}

impl RelativeAnchor {
    /// From the slicer's `relativeRange` text (`Last`, `Next`, `This`).
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "last" => Some(Self::Last),
            "next" => Some(Self::Next),
            "this" => Some(Self::This),
            _ => None,
        }
    }

    /// `Last`, `Next`, `This`.
    pub fn label(self) -> &'static str {
        match self {
            Self::Last => "Last",
            Self::Next => "Next",
            Self::This => "This",
        }
    }
}

/// The period of a relative setting (`data.relativePeriod`, or
/// `relativeTimePeriod` for minutes and hours).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RelativePeriod {
    Minutes,
    Hours,
    #[default]
    Days,
    Weeks,
    CalendarWeeks,
    Months,
    CalendarMonths,
    Years,
    CalendarYears,
}

impl RelativePeriod {
    /// Every period, in the order of Desktop's list (time periods first).
    pub const ALL: [RelativePeriod; 9] = [
        Self::Minutes,
        Self::Hours,
        Self::Days,
        Self::Weeks,
        Self::CalendarWeeks,
        Self::Months,
        Self::CalendarMonths,
        Self::Years,
        Self::CalendarYears,
    ];

    /// From the slicer's `relativePeriod` text (`Days`, `CalendarWeeks`, …;
    /// `Weeks (Calendar)` is accepted too).
    pub fn parse(text: &str) -> Option<Self> {
        let key: String = text
            .chars()
            .filter(char::is_ascii_alphabetic)
            .collect::<String>()
            .to_ascii_lowercase();
        Some(match key.as_str() {
            "minutes" | "minute" => Self::Minutes,
            "hours" | "hour" => Self::Hours,
            "days" | "day" => Self::Days,
            "weeks" | "week" => Self::Weeks,
            "calendarweeks" | "weekscalendar" => Self::CalendarWeeks,
            "months" | "month" => Self::Months,
            "calendarmonths" | "monthscalendar" => Self::CalendarMonths,
            "years" | "year" => Self::Years,
            "calendaryears" | "yearscalendar" => Self::CalendarYears,
            _ => return None,
        })
    }

    /// The label in Desktop's list (`Weeks (Calendar)`, …).
    pub fn label(self) -> &'static str {
        match self {
            Self::Minutes => "Minutes",
            Self::Hours => "Hours",
            Self::Days => "Days",
            Self::Weeks => "Weeks",
            Self::CalendarWeeks => "Weeks (Calendar)",
            Self::Months => "Months",
            Self::CalendarMonths => "Months (Calendar)",
            Self::Years => "Years",
            Self::CalendarYears => "Years (Calendar)",
        }
    }

    /// The unit and whether whole calendar units are meant.
    pub fn unit(self) -> (TimeUnit, bool) {
        match self {
            Self::Minutes => (TimeUnit::Minute, false),
            Self::Hours => (TimeUnit::Hour, false),
            Self::Days => (TimeUnit::Day, false),
            Self::Weeks => (TimeUnit::Week, false),
            Self::CalendarWeeks => (TimeUnit::Week, true),
            Self::Months => (TimeUnit::Month, false),
            Self::CalendarMonths => (TimeUnit::Month, true),
            Self::Years => (TimeUnit::Year, false),
            Self::CalendarYears => (TimeUnit::Year, true),
        }
    }

    /// Minutes and hours (relative time, not date).
    pub fn is_time(self) -> bool {
        matches!(self, Self::Minutes | Self::Hours)
    }

    fn of(unit: TimeUnit, calendar: bool) -> Option<Self> {
        Some(match (unit, calendar) {
            (TimeUnit::Minute, _) => Self::Minutes,
            (TimeUnit::Hour, _) => Self::Hours,
            (TimeUnit::Day, _) => Self::Days,
            (TimeUnit::Week, false) => Self::Weeks,
            (TimeUnit::Week, true) => Self::CalendarWeeks,
            (TimeUnit::Month, false) => Self::Months,
            (TimeUnit::Month, true) => Self::CalendarMonths,
            (TimeUnit::Year, false) => Self::Years,
            (TimeUnit::Year, true) => Self::CalendarYears,
            _ => return None,
        })
    }
}

/// A relative slicer setting: "Last 3 Months", "This Year", …
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelativeDateSpec {
    pub anchor: RelativeAnchor,
    /// Number of periods (ignored for `This`).
    pub duration: u32,
    pub period: RelativePeriod,
    /// Count today as one of the days (non-calendar periods only).
    pub include_today: bool,
}

impl Default for RelativeDateSpec {
    fn default() -> Self {
        Self {
            anchor: RelativeAnchor::Last,
            duration: 1,
            period: RelativePeriod::Days,
            include_today: true,
        }
    }
}

fn now_expr() -> Json {
    json!({ "Now": {} })
}

fn add(expr: Json, amount: i64, unit: TimeUnit) -> Json {
    json!({ "DateAdd": { "Expression": expr, "Amount": amount, "TimeUnit": unit.code() } })
}

fn span(expr: Json, unit: TimeUnit) -> Json {
    json!({ "DateSpan": { "Expression": expr, "TimeUnit": unit.code() } })
}

impl RelativeDateSpec {
    /// The same setting with the fields it ignores reset: `include_today`
    /// only matters for non-calendar date periods; `This` has no duration
    /// and no calendar variant.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        let (_, calendar) = self.period.unit();
        if calendar || self.period.is_time() || self.anchor == RelativeAnchor::This {
            self.include_today = true;
        }
        if self.anchor == RelativeAnchor::This {
            self.duration = 1;
            let (unit, _) = self.period.unit();
            self.period = RelativePeriod::of(unit, false).unwrap_or(self.period);
        }
        self.duration = self.duration.max(1);
        self
    }

    /// "Last 3 months", "This year", "Next 2 hours", "Last 2 weeks
    /// (calendar)".
    pub fn describe(&self) -> String {
        let period = self.period.label().to_ascii_lowercase();
        match self.anchor {
            RelativeAnchor::This => {
                let (unit, _) = self.period.unit();
                let name = match unit {
                    TimeUnit::Minute => "minute",
                    TimeUnit::Hour => "hour",
                    TimeUnit::Day => "day",
                    TimeUnit::Week => "week",
                    TimeUnit::Month => "month",
                    _ => "year",
                };
                format!("this {name}")
            }
            anchor => format!(
                "{} {} {period}",
                anchor.label().to_ascii_lowercase(),
                self.duration
            ),
        }
    }

    /// The `Between` condition Desktop writes for this setting on `field`.
    pub fn condition(&self, field: &FieldExpr) -> Condition {
        let n = i64::from(self.duration.max(1));
        let (unit, calendar) = self.period.unit();
        let day = TimeUnit::Day;
        let now = now_expr;
        let (lower, upper) = if self.period.is_time() {
            match self.anchor {
                RelativeAnchor::This => (span(now(), unit), span(now(), unit)),
                RelativeAnchor::Last => (add(now(), -n, unit), now()),
                RelativeAnchor::Next => (now(), add(now(), n, unit)),
            }
        } else {
            match (self.anchor, calendar, self.include_today) {
                (RelativeAnchor::This, ..) => (span(now(), unit), span(now(), unit)),
                (RelativeAnchor::Last, true, _) => (
                    span(add(now(), -n, unit), unit),
                    span(add(now(), -1, unit), unit),
                ),
                (RelativeAnchor::Last, false, true) => (
                    span(add(add(now(), 1, day), -n, unit), day),
                    span(now(), day),
                ),
                (RelativeAnchor::Last, false, false) => (
                    span(add(now(), -n, unit), day),
                    span(add(now(), -1, day), day),
                ),
                (RelativeAnchor::Next, true, _) => (
                    span(add(now(), 1, unit), unit),
                    span(add(now(), n, unit), unit),
                ),
                (RelativeAnchor::Next, false, true) => (
                    span(now(), day),
                    span(add(add(now(), -1, day), n, unit), day),
                ),
                (RelativeAnchor::Next, false, false) => (
                    span(add(now(), 1, day), day),
                    span(add(now(), n, unit), day),
                ),
            }
        };
        Condition::Between {
            expr: field.clone(),
            lower: FieldExpr::Unknown(lower),
            upper: FieldExpr::Unknown(upper),
        }
    }

    /// Reads back a condition written by [`RelativeDateSpec::condition`]
    /// (the forms Desktop writes); `None` for anything else.
    pub fn from_condition(condition: &Condition) -> Option<Self> {
        let Condition::Between { lower, upper, .. } = condition else {
            return None;
        };
        let (FieldExpr::Unknown(lower), FieldExpr::Unknown(upper)) = (lower, upper) else {
            return None;
        };
        let (lower, upper) = (shape(lower)?, shape(upper)?);
        let day = TimeUnit::Day;
        let spec = |anchor, duration: i64, unit, calendar, include_today| {
            Some(Self {
                anchor,
                duration: u32::try_from(duration).ok().filter(|d| *d > 0)?,
                period: RelativePeriod::of(unit, calendar)?,
                include_today,
            })
        };
        use RelativeAnchor::{Last, Next, This};
        match (&lower, &upper) {
            (Shape::Span(lu, lo), Shape::Span(uu, uo))
                if lu == uu && lo.is_empty() && uo.is_empty() =>
            {
                spec(This, 1, *lu, false, true)
            }
            (Shape::Span(lu, lo), Shape::Span(uu, uo)) => match (lo.as_slice(), uo.as_slice()) {
                // Last n days without today, or n calendar units.
                ([(n, a)], [(-1, b)]) if a == lu && b == uu && lu == uu && *n < 0 => {
                    spec(Last, -n, *lu, *lu != day, *lu != day)
                }
                ([(1, a), (n, u)], []) if *lu == day && *uu == day && *a == day && *n < 0 => {
                    spec(Last, -n, *u, false, true)
                }
                ([(n, u)], [(-1, b)]) if *lu == day && *uu == day && *b == day && *n < 0 => {
                    spec(Last, -n, *u, false, false)
                }
                ([(1, a)], [(n, b)]) if a == lu && b == uu && lu == uu && *n > 0 => {
                    spec(Next, *n, *lu, *lu != day, *lu != day)
                }
                ([], [(-1, a), (n, u)]) if *lu == day && *uu == day && *a == day && *n > 0 => {
                    spec(Next, *n, *u, false, true)
                }
                ([(1, a)], [(n, u)]) if *lu == day && *uu == day && *a == day && *n > 0 => {
                    spec(Next, *n, *u, false, false)
                }
                _ => None,
            },
            (Shape::Point(lo), Shape::Point(uo)) => match (lo.as_slice(), uo.as_slice()) {
                ([(n, u)], []) if *n < 0 => spec(Last, -n, *u, false, true),
                ([], [(n, u)]) if *n > 0 => spec(Next, *n, *u, false, true),
                _ => None,
            },
            _ => None,
        }
    }
}

/// A relative bound: offsets from `Now` (innermost first), optionally
/// wrapped in a span.
enum Shape {
    Point(Vec<(i64, TimeUnit)>),
    Span(TimeUnit, Vec<(i64, TimeUnit)>),
}

fn shape(j: &Json) -> Option<Shape> {
    fn offsets(j: &Json, depth: usize) -> Option<Vec<(i64, TimeUnit)>> {
        if depth > MAX_DEPTH {
            return None;
        }
        if j.get("Now").is_some() {
            return Some(Vec::new());
        }
        let a = j.get("DateAdd")?;
        let mut inner = offsets(a.get("Expression")?, depth + 1)?;
        inner.push((
            amount(a)?,
            TimeUnit::from_code(a.get("TimeUnit")?.as_i64()?)?,
        ));
        Some(inner)
    }
    match j.get("DateSpan") {
        Some(s) => Some(Shape::Span(
            TimeUnit::from_code(s.get("TimeUnit")?.as_i64()?)?,
            offsets(s.get("Expression")?, 0)?,
        )),
        None => offsets(j, 0).map(Shape::Point),
    }
}

/// Nesting cap for date expressions read from files.
const MAX_DEPTH: usize = 32;

fn amount(add: &Json) -> Option<i64> {
    let a = add.get("Amount")?;
    a.as_i64().or_else(|| {
        a.as_f64()
            .filter(|f| f.is_finite() && f.abs() < 1e12)
            .map(|f| f.trunc() as i64)
    })
}

/// `true` if the expression is a relative date expression (`Now`,
/// `DateAdd`, `DateSpan`).
pub fn is_relative_expr(f: &FieldExpr) -> bool {
    matches!(f, FieldExpr::Unknown(j) if ["Now", "DateAdd", "DateSpan"].iter().any(|k| j.get(k).is_some()))
}

/// `true` if the condition has a relative bound.
pub fn is_relative(condition: &Condition) -> bool {
    let mut found = false;
    condition.visit_fields(&mut |f| found |= is_relative_expr(f));
    found
}

/// A moment or a whole unit of time.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Moment {
    Point(NaiveDateTime),
    /// `[start, end)`.
    Span(NaiveDateTime, NaiveDateTime),
}

impl Moment {
    fn start(self) -> NaiveDateTime {
        match self {
            Moment::Point(t) | Moment::Span(t, _) => t,
        }
    }
}

const OUT_OF_RANGE: &str = "the relative date is out of range";

fn eval(j: &Json, now: NaiveDateTime, depth: usize) -> Result<Moment, String> {
    if depth > MAX_DEPTH {
        return Err("the relative date expression is nested too deeply".into());
    }
    if j.get("Now").is_some() {
        return Ok(Moment::Point(now));
    }
    if let Some(a) = j.get("DateAdd") {
        let inner = eval(a.get("Expression").unwrap_or(&Json::Null), now, depth + 1)?;
        let n = amount(a).ok_or("DateAdd has no amount")?;
        let unit = unit_of(a)?;
        return add_units(inner.start(), n, unit)
            .map(Moment::Point)
            .ok_or_else(|| OUT_OF_RANGE.into());
    }
    if let Some(s) = j.get("DateSpan") {
        let inner = eval(s.get("Expression").unwrap_or(&Json::Null), now, depth + 1)?;
        let unit = unit_of(s)?;
        let start = truncate(inner.start(), unit).ok_or(OUT_OF_RANGE)?;
        let end = add_units(start, 1, unit).ok_or(OUT_OF_RANGE)?;
        return Ok(Moment::Span(start, end));
    }
    if let Some(value) = j
        .get("Literal")
        .and_then(|l| l.get("Value"))
        .and_then(Json::as_str)
        && let Ok(LiteralValue::DateTime(iso)) = LiteralValue::parse(value)
        && let Some(t) = parse_iso(&iso)
    {
        return Ok(Moment::Point(t));
    }
    Err("unsupported date expression in a relative filter".into())
}

fn unit_of(j: &Json) -> Result<TimeUnit, String> {
    j.get("TimeUnit")
        .and_then(Json::as_i64)
        .and_then(TimeUnit::from_code)
        .ok_or_else(|| "unknown TimeUnit in a relative filter".into())
}

fn parse_iso(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim().trim_end_matches(['Z', 'z']);
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S"))
        .ok()
        .or_else(|| {
            NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .ok()
                .map(|d| d.and_time(NaiveTime::MIN))
        })
}

fn add_months(t: NaiveDateTime, months: i64) -> Option<NaiveDateTime> {
    let m = Months::new(u32::try_from(months.unsigned_abs()).ok()?);
    if months >= 0 {
        t.checked_add_months(m)
    } else {
        t.checked_sub_months(m)
    }
}

fn add_units(t: NaiveDateTime, n: i64, unit: TimeUnit) -> Option<NaiveDateTime> {
    let delta = |d: Option<TimeDelta>| d.and_then(|d| t.checked_add_signed(d));
    match unit {
        TimeUnit::Day => delta(TimeDelta::try_days(n)),
        TimeUnit::Week => delta(TimeDelta::try_weeks(n)),
        TimeUnit::Month => add_months(t, n),
        TimeUnit::Year => add_months(t, n.checked_mul(12)?),
        TimeUnit::Decade => add_months(t, n.checked_mul(120)?),
        TimeUnit::Second => delta(TimeDelta::try_seconds(n)),
        TimeUnit::Minute => delta(TimeDelta::try_minutes(n)),
        TimeUnit::Hour => delta(TimeDelta::try_hours(n)),
    }
}

/// The start of the unit containing `t` (weeks start on Sunday).
fn truncate(t: NaiveDateTime, unit: TimeUnit) -> Option<NaiveDateTime> {
    let d = t.date();
    let midnight = |d: NaiveDate| d.and_time(NaiveTime::MIN);
    Some(match unit {
        TimeUnit::Day => midnight(d),
        TimeUnit::Week => midnight(d.checked_sub_signed(TimeDelta::try_days(i64::from(
            d.weekday().num_days_from_sunday(),
        ))?)?),
        TimeUnit::Month => midnight(d.with_day(1)?),
        TimeUnit::Year => midnight(NaiveDate::from_ymd_opt(d.year(), 1, 1)?),
        TimeUnit::Decade => midnight(NaiveDate::from_ymd_opt(
            d.year() - d.year().rem_euclid(10),
            1,
            1,
        )?),
        TimeUnit::Hour => d.and_hms_opt(t.hour(), 0, 0)?,
        TimeUnit::Minute => d.and_hms_opt(t.hour(), t.minute(), 0)?,
        TimeUnit::Second => d.and_hms_opt(t.hour(), t.minute(), t.second())?,
    })
}

fn literal(t: NaiveDateTime) -> FieldExpr {
    FieldExpr::Literal(Literal::datetime(
        &t.format("%Y-%m-%dT%H:%M:%S").to_string(),
    ))
}

fn cmp(kind: ComparisonKind, left: &FieldExpr, t: NaiveDateTime) -> Condition {
    Condition::Comparison {
        kind,
        left: left.clone(),
        right: literal(t),
    }
}

fn and(a: Condition, b: Condition) -> Condition {
    Condition::And(Box::new(a), Box::new(b))
}

fn bound(f: &FieldExpr, now: NaiveDateTime) -> Result<Option<Moment>, String> {
    match f {
        FieldExpr::Unknown(j) if is_relative_expr(f) => eval(j, now, 0).map(Some),
        _ => Ok(None),
    }
}

/// Rewrites the relative bounds of `condition` into literals for `now`
/// (see the module docs): a span lower bound becomes `>= start`, a span
/// upper bound `< end`.
///
/// # Errors
/// A user-facing reason for an unsupported or out-of-range expression.
pub fn resolve(condition: &Condition, now: NaiveDateTime) -> Result<Condition, String> {
    use ComparisonKind::{
        Equal, GreaterThan, GreaterThanOrEqual as Ge, LessThan as Lt, LessThanOrEqual,
    };
    Ok(match condition {
        Condition::And(a, b) => and(resolve(a, now)?, resolve(b, now)?),
        Condition::Or(a, b) => {
            Condition::Or(Box::new(resolve(a, now)?), Box::new(resolve(b, now)?))
        }
        Condition::Not(c) => Condition::Not(Box::new(resolve(c, now)?)),
        Condition::Between { expr, lower, upper } => {
            match (bound(lower, now)?, bound(upper, now)?) {
                (None, None) => condition.clone(),
                (lo, hi) => {
                    let lower = match lo {
                        Some(m) => cmp(Ge, expr, m.start()),
                        None => Condition::Comparison {
                            kind: Ge,
                            left: expr.clone(),
                            right: lower.clone(),
                        },
                    };
                    let upper = match hi {
                        Some(Moment::Span(_, end)) => cmp(Lt, expr, end),
                        Some(Moment::Point(t)) => cmp(LessThanOrEqual, expr, t),
                        None => Condition::Comparison {
                            kind: LessThanOrEqual,
                            left: expr.clone(),
                            right: upper.clone(),
                        },
                    };
                    and(lower, upper)
                }
            }
        }
        Condition::Comparison { kind, left, right } => match bound(right, now)? {
            None => condition.clone(),
            Some(Moment::Point(t)) => cmp(*kind, left, t),
            Some(Moment::Span(start, end)) => match kind {
                GreaterThan => cmp(Ge, left, end),
                Ge => cmp(Ge, left, start),
                Lt => cmp(Lt, left, start),
                LessThanOrEqual => cmp(Lt, left, end),
                Equal => and(cmp(Ge, left, start), cmp(Lt, left, end)),
            },
        },
        other => other.clone(),
    })
}

/// The `[start, end)` range a relative condition keeps for `now` (both
/// bounds needed); `None` when it is not a relative `Between`.
pub fn range_of(
    condition: &Condition,
    now: NaiveDateTime,
) -> Option<(NaiveDateTime, NaiveDateTime)> {
    let Condition::Between { lower, upper, .. } = condition else {
        return None;
    };
    let lo = bound(lower, now).ok()??;
    let hi = bound(upper, now).ok()??;
    let end = match hi {
        Moment::Span(_, end) => end,
        Moment::Point(t) => t,
    };
    Some((lo.start(), end))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, m: u32, d: u32, h: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .and_then(|d| d.and_hms_opt(h, 30, 0))
            .unwrap()
    }

    fn bounds(spec: RelativeDateSpec, now: NaiveDateTime) -> (String, String) {
        let c = spec.condition(&FieldExpr::column("Date", "Date"));
        let (lo, hi) = range_of(&c, now).unwrap();
        (lo.to_string(), hi.to_string())
    }

    fn spec(
        anchor: RelativeAnchor,
        n: u32,
        period: RelativePeriod,
        today: bool,
    ) -> RelativeDateSpec {
        RelativeDateSpec {
            anchor,
            duration: n,
            period,
            include_today: today,
        }
    }

    #[test]
    fn relative_settings_cover_the_documented_ranges() {
        use RelativeAnchor::{Last, Next, This};
        use RelativePeriod::*;
        // Thursday 2026-01-15 09:30.
        let now = at(2026, 1, 15, 9);
        let cases = [
            (
                spec(Last, 3, Days, true),
                "2026-01-13 00:00:00",
                "2026-01-16 00:00:00",
            ),
            (
                spec(Last, 3, Days, false),
                "2026-01-12 00:00:00",
                "2026-01-15 00:00:00",
            ),
            (
                spec(Last, 10, Years, true),
                "2016-01-16 00:00:00",
                "2026-01-16 00:00:00",
            ),
            (
                spec(Last, 2, CalendarMonths, true),
                "2025-11-01 00:00:00",
                "2026-01-01 00:00:00",
            ),
            (
                spec(Last, 1, CalendarWeeks, true),
                "2026-01-04 00:00:00",
                "2026-01-11 00:00:00",
            ),
            (
                spec(Next, 2, Weeks, true),
                "2026-01-15 00:00:00",
                "2026-01-29 00:00:00",
            ),
            (
                spec(Next, 1, CalendarYears, true),
                "2027-01-01 00:00:00",
                "2028-01-01 00:00:00",
            ),
            (
                spec(This, 1, Months, true),
                "2026-01-01 00:00:00",
                "2026-02-01 00:00:00",
            ),
            (
                spec(This, 1, Weeks, true),
                "2026-01-11 00:00:00",
                "2026-01-18 00:00:00",
            ),
            (
                spec(Last, 2, Hours, true),
                "2026-01-15 07:30:00",
                "2026-01-15 09:30:00",
            ),
            (
                spec(This, 1, Hours, true),
                "2026-01-15 09:00:00",
                "2026-01-15 10:00:00",
            ),
        ];
        for (s, lo, hi) in cases {
            assert_eq!(bounds(s, now), (lo.to_owned(), hi.to_owned()), "{s:?}");
            let c = s.condition(&FieldExpr::column("Date", "Date"));
            assert_eq!(
                RelativeDateSpec::from_condition(&c),
                Some(s.normalized()),
                "{c:?}"
            );
        }
    }

    #[test]
    fn resolved_conditions_compare_with_literals() {
        let c = spec(RelativeAnchor::Last, 10, RelativePeriod::Years, true)
            .condition(&FieldExpr::column("Date", "Date"));
        assert!(is_relative(&c));
        let r = resolve(&c, at(2026, 1, 15, 9)).unwrap();
        assert!(!is_relative(&r));
        let Condition::And(lo, hi) = r else {
            panic!("{r:?}")
        };
        assert!(
            matches!(*lo, Condition::Comparison { kind: ComparisonKind::GreaterThanOrEqual, right: FieldExpr::Literal(ref l), .. } if l.raw == "datetime'2016-01-16T00:00:00'")
        );
        assert!(
            matches!(*hi, Condition::Comparison { kind: ComparisonKind::LessThan, right: FieldExpr::Literal(ref l), .. } if l.raw == "datetime'2026-01-16T00:00:00'")
        );
    }

    #[test]
    fn hostile_amounts_are_errors_not_panics() {
        let field = FieldExpr::column("Date", "Date");
        for amount in [json!(i64::MAX), json!(i64::MIN), json!(1e300), json!("x")] {
            let c = Condition::Between {
                expr: field.clone(),
                lower: FieldExpr::Unknown(
                    json!({"DateAdd": {"Expression": {"Now": {}}, "Amount": amount, "TimeUnit": 3}}),
                ),
                upper: FieldExpr::Unknown(json!({"Now": {}})),
            };
            assert!(resolve(&c, at(2026, 1, 15, 9)).is_err(), "{amount}");
        }
        let mut deep = json!({"Now": {}});
        for _ in 0..100 {
            deep = json!({"DateAdd": {"Expression": deep, "Amount": 1, "TimeUnit": 0}});
        }
        let c = Condition::Comparison {
            kind: ComparisonKind::GreaterThan,
            left: field,
            right: FieldExpr::Unknown(deep),
        };
        assert!(resolve(&c, at(2026, 1, 15, 9)).is_err());
    }
}
