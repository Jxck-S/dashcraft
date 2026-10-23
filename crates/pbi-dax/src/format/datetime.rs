//! Custom date/time format strings (VB / OLE Automation dialect, letters
//! case-insensitive) for the `en-US` culture.

use chrono::{Datelike, NaiveDateTime, Timelike};
use pbi_engine::Value;
use pbi_engine::value::MICROS_PER_DAY;

use super::number::{bare_chars, split_sections};

const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

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

/// `true` if the format has date/time placeholders (`d m y h n s q w c t`).
#[must_use]
pub fn is_date_format(fmt: &str) -> bool {
    bare_chars(fmt).any(|c| {
        matches!(
            c.to_ascii_lowercase(),
            'd' | 'm' | 'y' | 'h' | 'n' | 's' | 'q' | 'w' | 'c' | 't'
        )
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AmPm {
    /// `AM/PM`, `AMPM`, `tt`.
    Upper,
    /// `am/pm`.
    Lower,
    /// `A/P`, `t`.
    UpperShort,
    /// `a/p`.
    LowerShort,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    /// `d` … `dddddd`.
    Day(usize),
    WeekDay,
    WeekOfYear,
    /// `m`/`mm`: month or minute, resolved after parsing.
    MonthOrMinute(usize),
    Month(usize),
    Minute(usize),
    Quarter,
    DayOfYear,
    Year2,
    Year4,
    Hour(usize),
    Second(usize),
    Fraction(usize),
    AmPm(AmPm),
    /// `c`.
    General,
    /// `ttttt`.
    LongTime,
    Lit(String),
}

fn starts_with_ci(chars: &[char], at: usize, pattern: &str) -> bool {
    pattern.chars().enumerate().all(|(k, p)| {
        chars
            .get(at + k)
            .is_some_and(|c| c.eq_ignore_ascii_case(&p))
    })
}

fn tokenize(section: &str) -> Vec<Tok> {
    let chars: Vec<char> = section.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        let lower = c.to_ascii_lowercase();
        let run = chars.get(i..).map_or(1, |r| {
            r.iter().take_while(|x| x.eq_ignore_ascii_case(&c)).count()
        });
        let mut consumed = run;
        let tok = match lower {
            'd' => Tok::Day(run.min(6)),
            'w' if run == 1 => Tok::WeekDay,
            'w' => Tok::WeekOfYear,
            'm' if run <= 2 => Tok::MonthOrMinute(run),
            'm' => Tok::Month(run.min(4)),
            'n' => Tok::Minute(run.min(2)),
            'q' => {
                consumed = 1;
                Tok::Quarter
            }
            'y' if run == 1 => Tok::DayOfYear,
            'y' if run == 2 => Tok::Year2,
            'y' => Tok::Year4,
            'h' => Tok::Hour(run.min(2)),
            's' => Tok::Second(run.min(2)),
            'f' => Tok::Fraction(run.min(7)),
            'c' => {
                consumed = 1;
                Tok::General
            }
            't' if run >= 5 => {
                consumed = 5;
                Tok::LongTime
            }
            't' if run == 1 => Tok::AmPm(AmPm::UpperShort),
            't' => Tok::AmPm(AmPm::Upper),
            'a' if starts_with_ci(&chars, i, "am/pm") => {
                consumed = 5;
                Tok::AmPm(if c == 'A' { AmPm::Upper } else { AmPm::Lower })
            }
            'a' if starts_with_ci(&chars, i, "a/p") => {
                consumed = 3;
                Tok::AmPm(if c == 'A' {
                    AmPm::UpperShort
                } else {
                    AmPm::LowerShort
                })
            }
            'a' if starts_with_ci(&chars, i, "ampm") => {
                consumed = 4;
                Tok::AmPm(AmPm::Upper)
            }
            '\\' => {
                consumed = 2;
                Tok::Lit(chars.get(i + 1).map(char::to_string).unwrap_or_default())
            }
            '"' => {
                let lit: String = chars
                    .get(i + 1..)
                    .map(|r| r.iter().take_while(|&&x| x != '"').collect())
                    .unwrap_or_default();
                consumed = lit.chars().count() + 2;
                Tok::Lit(lit)
            }
            _ => {
                consumed = 1;
                Tok::Lit(c.to_string())
            }
        };
        toks.push(tok);
        i += consumed.max(1);
    }
    resolve_minutes(&mut toks);
    toks
}

/// `m`/`mm` right after an hour or right before a second (ignoring literal
/// separators) is a minute; otherwise a month.
fn resolve_minutes(toks: &mut [Tok]) {
    let is_lit = |t: &Tok| matches!(t, Tok::Lit(_));
    for i in 0..toks.len() {
        let Some(Tok::MonthOrMinute(n)) = toks.get(i).cloned() else {
            continue;
        };
        let prev = toks
            .get(..i)
            .and_then(|p| p.iter().rev().find(|t| !is_lit(t)));
        let next = toks
            .get(i + 1..)
            .and_then(|r| r.iter().find(|t| !is_lit(t)));
        let minute = matches!(prev, Some(Tok::Hour(_))) || matches!(next, Some(Tok::Second(_)));
        if let Some(t) = toks.get_mut(i) {
            *t = if minute {
                Tok::Minute(n)
            } else {
                Tok::Month(n)
            };
        }
    }
}

fn naive(us: i64) -> Option<NaiveDateTime> {
    chrono::DateTime::from_timestamp_micros(us).map(|d| d.naive_utc())
}

/// Microseconds of 1899-12-30T00:00 (DAX serial date 0).
const SERIAL_ZERO_MICROS: i64 = -25_569 * MICROS_PER_DAY;

fn render(dt: &NaiveDateTime, us: i64, toks: &[Tok]) -> String {
    let twelve_hour = toks.iter().any(|t| matches!(t, Tok::AmPm(_)));
    let pm = dt.hour() >= 12;
    let mut out = String::new();
    for t in toks {
        match t {
            Tok::Day(1) => out.push_str(&dt.day().to_string()),
            Tok::Day(2) => out.push_str(&format!("{:02}", dt.day())),
            Tok::Day(3) => out.push_str(day_name(dt).get(..3).unwrap_or_default()),
            Tok::Day(4) => out.push_str(day_name(dt)),
            Tok::Day(5) => out.push_str(&short_date(dt)),
            Tok::Day(_) => out.push_str(&long_date(dt)),
            Tok::WeekDay => out.push_str(&(dt.weekday().num_days_from_sunday() + 1).to_string()),
            Tok::WeekOfYear => out.push_str(&week_of_year(dt).to_string()),
            Tok::Month(1) | Tok::MonthOrMinute(1) => out.push_str(&dt.month().to_string()),
            Tok::Month(2) | Tok::MonthOrMinute(_) => {
                out.push_str(&format!("{:02}", dt.month()));
            }
            Tok::Month(3) => out.push_str(month_name(dt).get(..3).unwrap_or_default()),
            Tok::Month(_) => out.push_str(month_name(dt)),
            Tok::Minute(1) => out.push_str(&dt.minute().to_string()),
            Tok::Minute(_) => out.push_str(&format!("{:02}", dt.minute())),
            Tok::Quarter => out.push_str(&(dt.month0() / 3 + 1).to_string()),
            Tok::DayOfYear => out.push_str(&dt.ordinal().to_string()),
            Tok::Year2 => out.push_str(&format!("{:02}", dt.year().rem_euclid(100))),
            Tok::Year4 => out.push_str(&format!("{:04}", dt.year())),
            Tok::Hour(n) => {
                let h = if twelve_hour {
                    match dt.hour() % 12 {
                        0 => 12,
                        h => h,
                    }
                } else {
                    dt.hour()
                };
                if *n == 1 {
                    out.push_str(&h.to_string());
                } else {
                    out.push_str(&format!("{h:02}"));
                }
            }
            Tok::Second(1) => out.push_str(&dt.second().to_string()),
            Tok::Second(_) => out.push_str(&format!("{:02}", dt.second())),
            Tok::Fraction(n) => {
                let micros = format!("{:06}", dt.nanosecond() / 1_000);
                let digits: String = micros
                    .chars()
                    .chain(std::iter::repeat('0'))
                    .take(*n)
                    .collect();
                out.push_str(&digits);
            }
            Tok::AmPm(style) => out.push_str(match (style, pm) {
                (AmPm::Upper, false) => "AM",
                (AmPm::Upper, true) => "PM",
                (AmPm::Lower, false) => "am",
                (AmPm::Lower, true) => "pm",
                (AmPm::UpperShort, false) => "A",
                (AmPm::UpperShort, true) => "P",
                (AmPm::LowerShort, false) => "a",
                (AmPm::LowerShort, true) => "p",
            }),
            Tok::General => {
                let time = us.rem_euclid(MICROS_PER_DAY) != 0;
                let date = us.div_euclid(MICROS_PER_DAY) != SERIAL_ZERO_MICROS / MICROS_PER_DAY;
                let mut parts = Vec::new();
                if date || !time {
                    parts.push(short_date(dt));
                }
                if time {
                    parts.push(long_time(dt));
                }
                out.push_str(&parts.join(" "));
            }
            Tok::LongTime => out.push_str(&long_time(dt)),
            Tok::Lit(s) => out.push_str(s),
        }
    }
    out
}

fn day_name(dt: &NaiveDateTime) -> &'static str {
    DAYS.get(dt.weekday().num_days_from_sunday() as usize)
        .copied()
        .unwrap_or_default()
}

fn month_name(dt: &NaiveDateTime) -> &'static str {
    MONTHS
        .get(dt.month0() as usize)
        .copied()
        .unwrap_or_default()
}

/// Week number with weeks starting on Sunday and January 1 in week 1.
fn week_of_year(dt: &NaiveDateTime) -> u32 {
    let jan1 = (dt.weekday().num_days_from_sunday() + 7 - dt.ordinal0() % 7) % 7;
    (dt.ordinal0() + jan1) / 7 + 1
}

/// `ddddd`: `MM/dd/yyyy`.
fn short_date(dt: &NaiveDateTime) -> String {
    format!("{:02}/{:02}/{:04}", dt.month(), dt.day(), dt.year())
}

/// `dddddd`: `dddd, mmmm d, yyyy`.
fn long_date(dt: &NaiveDateTime) -> String {
    format!(
        "{}, {} {}, {:04}",
        day_name(dt),
        month_name(dt),
        dt.day(),
        dt.year()
    )
}

/// `ttttt`: `hh:nn:ss` (24-hour).
fn long_time(dt: &NaiveDateTime) -> String {
    format!("{:02}:{:02}:{:02}", dt.hour(), dt.minute(), dt.second())
}

/// Formats a date/time (microseconds since 1970-01-01, no time zone) with a
/// custom date/time format. Only the first `;` section is used.
#[must_use]
pub fn format_datetime(us: i64, fmt: &str) -> String {
    let Some(dt) = naive(us) else {
        return Value::DateTime(us).to_string();
    };
    let section = split_sections(fmt).first().copied().unwrap_or_default();
    render(&dt, us, &tokenize(section))
}

/// `General Date`: `M/d/yyyy h:nn:ss AM/PM`, without the time at midnight
/// and without the date on serial day 0 (1899-12-30).
#[must_use]
pub fn general_date(us: i64) -> String {
    let time = us.rem_euclid(MICROS_PER_DAY) != 0;
    let date = us.div_euclid(MICROS_PER_DAY) != SERIAL_ZERO_MICROS / MICROS_PER_DAY;
    let pattern = match (date, time) {
        (_, false) => "m/d/yyyy",
        (false, true) => "h:nn:ss AM/PM",
        (true, true) => "m/d/yyyy h:nn:ss AM/PM",
    };
    format_datetime(us, pattern)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn us(y: i64, m: u32, d: u32, h: u32, n: u32, s: u32) -> i64 {
        Value::datetime(y, m, d, h, n, s)
            .and_then(|v| v.as_datetime_micros())
            .unwrap_or_default()
    }

    #[test]
    fn documented_examples() {
        // Thursday, June 25, 2020, 1:23:45 PM (DAX FORMAT docs, en-US).
        let t = us(2020, 6, 25, 13, 23, 45);
        let cases = [
            ("c", "06/25/2020 13:23:45"),
            ("d", "25"),
            ("dd", "25"),
            ("ddd", "Thu"),
            ("dddd", "Thursday"),
            ("ddddd", "06/25/2020"),
            ("dddddd", "Thursday, June 25, 2020"),
            ("w", "5"),
            ("ww", "26"),
            ("m", "6"),
            ("mm", "06"),
            ("mmm", "Jun"),
            ("mmmm", "June"),
            ("q", "2"),
            ("y", "177"),
            ("yy", "20"),
            ("yyyy", "2020"),
            ("\"Year\" yyyy", "Year 2020"),
            ("yyyy \\Qq", "2020 Q2"),
            ("dd/mm/yyyy", "25/06/2020"),
            ("mm/dd/yyyy", "06/25/2020"),
            ("h:nn:ss", "13:23:45"),
            ("h:nn:ss AMPM", "1:23:45 PM"),
            ("hh:nn:ss", "13:23:45"),
            ("hh:nn:ss AMPM", "01:23:45 PM"),
            ("ttttt", "13:23:45"),
            ("ttttt AMPM", "13:23:45 PM"),
            ("mm/dd/yyyy hh:nn:ss AMPM", "06/25/2020 01:23:45 PM"),
        ];
        for (fmt, expected) in cases {
            assert_eq!(format_datetime(t, fmt), expected, "{fmt}");
        }
    }

    #[test]
    fn dotnet_spellings_and_minutes() {
        let t = us(2026, 10, 21, 9, 5, 7);
        assert_eq!(format_datetime(t, "yyyy-mm-dd hh:nn"), "2026-10-21 09:05");
        assert_eq!(format_datetime(t, "yyyy-MM-dd HH:mm"), "2026-10-21 09:05");
        assert_eq!(format_datetime(t, "dd/MM/yyyy"), "21/10/2026");
        assert_eq!(format_datetime(t, "mm:ss"), "05:07");
        assert_eq!(format_datetime(t, "h:mm tt"), "9:05 AM");
        assert_eq!(format_datetime(t, "h:mm a/p"), "9:05 a");
        assert_eq!(format_datetime(t, "MMMM yyyy"), "October 2026");
        assert_eq!(format_datetime(t, "hh:nn:ss.fff"), "09:05:07.000");
        assert_eq!(format_datetime(us(2026, 1, 1, 0, 0, 0), "h AM/PM"), "12 AM");
        assert_eq!(
            format_datetime(us(2026, 1, 1, 12, 0, 0), "h am/pm"),
            "12 pm"
        );
    }

    #[test]
    fn general_date_variants() {
        assert_eq!(
            general_date(us(2008, 3, 12, 11, 7, 31)),
            "3/12/2008 11:07:31 AM"
        );
        assert_eq!(general_date(us(2008, 3, 12, 0, 0, 0)), "3/12/2008");
        assert_eq!(general_date(us(1899, 12, 30, 13, 0, 0)), "1:00:00 PM");
    }

    #[test]
    fn date_format_detection() {
        assert!(is_date_format("yyyy-mm-dd"));
        assert!(is_date_format("Short Date"));
        assert!(!is_date_format("0.0%"));
        assert!(!is_date_format("\\d\"ays\""));
    }
}
