//! Text rendering of values: `Text.From` semantics (en-US, culture-invariant numbers), short
//! descriptions for error messages, and M-literal debug rendering.

use std::fmt::Write as _;

use chrono::{Datelike, NaiveDate, NaiveTime, TimeDelta, Timelike};

use crate::value::types::quote_name;
use crate::value::{Lazy, Value};

/// Formats a number like M's `Number.ToText`/`Text.From` with the default format: integers
/// without a decimal point, otherwise up to 15 significant digits, switching to exponent
/// notation (`1E+15`, `1E-05`) for very large or small magnitudes.
pub fn number_to_text(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if n == 0.0 {
        return "0".into();
    }
    if n.fract() == 0.0 && n.abs() < 1e15 {
        return format!("{}", n as i64);
    }
    let sci = format!("{:.14e}", n);
    let Some((mantissa, exp)) = sci.split_once('e') else {
        return sci;
    };
    let Ok(exp) = exp.parse::<i32>() else {
        return sci;
    };
    let negative = mantissa.starts_with('-');
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    if !(-4..15).contains(&exp) {
        out.push_str(&digits[..1]);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        let _ = write!(out, "E{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs());
        return out;
    }
    if exp >= 0 {
        let int_len = usize::try_from(exp).unwrap_or(0) + 1;
        if digits.len() <= int_len {
            out.push_str(digits);
            out.extend(std::iter::repeat_n('0', int_len - digits.len()));
        } else {
            out.push_str(&digits[..int_len]);
            out.push('.');
            out.push_str(&digits[int_len..]);
        }
    } else {
        out.push_str("0.");
        out.extend(std::iter::repeat_n(
            '0',
            usize::try_from(-exp - 1).unwrap_or(0),
        ));
        out.push_str(digits);
    }
    out
}

pub(crate) fn date_to_text(d: NaiveDate) -> String {
    format!("{}/{}/{}", d.month(), d.day(), d.year())
}

pub(crate) fn time_to_text(t: NaiveTime) -> String {
    let (pm, h12) = t.hour12();
    format!(
        "{}:{:02}:{:02} {}",
        h12,
        t.minute(),
        t.second(),
        if pm { "PM" } else { "AM" }
    )
}

pub(crate) fn duration_to_text(d: TimeDelta) -> String {
    let negative = d < TimeDelta::zero();
    let d = d.abs();
    let days = d.num_days();
    let hours = d.num_hours() % 24;
    let minutes = d.num_minutes() % 60;
    let seconds = d.num_seconds() % 60;
    let ticks = (d.subsec_nanos() / 100).unsigned_abs();
    let mut s = String::new();
    if negative {
        s.push('-');
    }
    if days != 0 {
        let _ = write!(s, "{days}.");
    }
    let _ = write!(s, "{hours:02}:{minutes:02}:{seconds:02}");
    if ticks != 0 {
        let _ = write!(s, ".{ticks:07}");
    }
    s
}

/// `Text.From(value)` for scalar values; `None` for `null` and for values without a text form
/// (lists, records, tables, functions, types, binaries).
pub fn value_to_text(v: &Value) -> Option<String> {
    Some(match v.plain() {
        Value::Text(t) => t.to_string(),
        Value::Number(n) => number_to_text(*n),
        Value::Logical(b) => b.to_string(),
        Value::Date(d) => date_to_text(*d),
        Value::Time(t) => time_to_text(*t),
        Value::DateTime(dt) => format!("{} {}", date_to_text(dt.date()), time_to_text(dt.time())),
        Value::DateTimeZone(dt) => {
            let off = dt.offset().local_minus_utc();
            let sign = if off < 0 { '-' } else { '+' };
            let off = off.abs();
            format!(
                "{} {} {sign}{:02}:{:02}",
                date_to_text(dt.date_naive()),
                time_to_text(dt.time()),
                off / 3600,
                (off % 3600) / 60
            )
        }
        Value::Duration(d) => duration_to_text(*d),
        _ => return None,
    })
}

/// Short description of a value for error messages (`"abc"`, `1`, `null`, `[List]`).
pub fn describe(v: &Value) -> String {
    match v.plain() {
        Value::Text(t) => {
            let shown: String = t.chars().take(60).collect();
            if shown.len() < t.len() {
                format!("\"{shown}…\"")
            } else {
                format!("\"{shown}\"")
            }
        }
        Value::Null => "null".into(),
        Value::List(_) => "[List]".into(),
        Value::Record(_) => "[Record]".into(),
        Value::Table(_) => "[Table]".into(),
        Value::Function(_) => "[Function]".into(),
        Value::Type(_) => "[Type]".into(),
        Value::Binary(_) => "[Binary]".into(),
        other => render(other, 0, 0),
    }
}

fn seconds_literal(t: NaiveTime) -> String {
    let nanos = t.nanosecond() % 1_000_000_000;
    if nanos == 0 {
        t.second().to_string()
    } else {
        number_to_text(f64::from(t.second()) + f64::from(nanos) / 1e9)
    }
}

fn number_literal(n: f64) -> String {
    if n.is_nan() {
        "#nan".into()
    } else if n.is_infinite() {
        if n > 0.0 { "#infinity" } else { "-#infinity" }.into()
    } else {
        number_to_text(n)
    }
}

fn render_slot(slot: &Lazy, depth: usize, max_items: usize, out: &mut String) {
    match slot.resolved() {
        Ok(v) => render_into(&v, depth, max_items, out),
        Err(crate::merror::MError::Internal(_)) => out.push_str("..."),
        Err(e) => {
            let _ = write!(out, "error {:?}", e.to_string());
        }
    }
}

/// Renders a value as (approximately) M source. Nesting below `depth` and collections beyond
/// `max_items` are abbreviated.
pub fn render(v: &Value, depth: usize, max_items: usize) -> String {
    let mut s = String::new();
    render_into(v, depth, max_items, &mut s);
    s
}

fn render_into(v: &Value, depth: usize, max_items: usize, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Logical(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&number_literal(*n)),
        Value::Text(t) => {
            out.push('"');
            out.push_str(&t.replace('"', "\"\""));
            out.push('"');
        }
        Value::Date(d) => {
            let _ = write!(out, "#date({}, {}, {})", d.year(), d.month(), d.day());
        }
        Value::Time(t) => {
            let _ = write!(
                out,
                "#time({}, {}, {})",
                t.hour(),
                t.minute(),
                seconds_literal(*t)
            );
        }
        Value::DateTime(dt) => {
            let _ = write!(
                out,
                "#datetime({}, {}, {}, {}, {}, {})",
                dt.year(),
                dt.month(),
                dt.day(),
                dt.hour(),
                dt.minute(),
                seconds_literal(dt.time())
            );
        }
        Value::DateTimeZone(dt) => {
            let off = dt.offset().local_minus_utc();
            let _ = write!(
                out,
                "#datetimezone({}, {}, {}, {}, {}, {}, {}, {})",
                dt.year(),
                dt.month(),
                dt.day(),
                dt.hour(),
                dt.minute(),
                seconds_literal(dt.time()),
                off / 3600,
                (off % 3600) / 60
            );
        }
        Value::Duration(d) => {
            let total = d.num_microseconds().map(|m| m as f64 / 1e6);
            let secs = d.num_seconds();
            let frac = total.map_or(0.0, |t| t - secs as f64);
            let _ = write!(
                out,
                "#duration({}, {}, {}, {})",
                d.num_days(),
                d.num_hours() % 24,
                d.num_minutes() % 60,
                number_to_text((secs % 60) as f64 + frac)
            );
        }
        Value::Binary(b) => match b.size_hint() {
            Some(n) => {
                let _ = write!(out, "#binary(<{n} bytes>)");
            }
            None => out.push_str("#binary(<not loaded>)"),
        },
        Value::Function(f) => match f.name() {
            Some(n) => out.push_str(n),
            None => out.push_str("<function>"),
        },
        Value::Type(t) => {
            let _ = write!(out, "type {t}");
        }
        Value::Meta(m) => {
            render_into(&m.value, depth, max_items, out);
            out.push_str(" meta ");
            render_into(&Value::Record(m.meta.clone()), depth, max_items, out);
        }
        Value::List(l) => {
            if depth == 0 {
                out.push_str("{...}");
                return;
            }
            out.push('{');
            for (i, item) in l.items().iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                if i >= max_items {
                    let _ = write!(out, "... ({} items)", l.len());
                    break;
                }
                render_slot(item, depth - 1, max_items, out);
            }
            out.push('}');
        }
        Value::Record(r) => {
            if depth == 0 {
                out.push_str("[...]");
                return;
            }
            out.push('[');
            for (i, (name, slot)) in r.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                if i >= max_items {
                    let _ = write!(out, "... ({} fields)", r.len());
                    break;
                }
                out.push_str(&quote_name(name));
                out.push_str(" = ");
                render_slot(slot, depth - 1, max_items, out);
            }
            out.push(']');
        }
        Value::Table(t) => {
            if depth == 0 {
                let _ = write!(out, "#table(<{} rows>)", t.row_count());
                return;
            }
            out.push_str("#table({");
            for (i, n) in t.column_names().iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                let _ = write!(out, "\"{}\"", n.replace('"', "\"\""));
            }
            out.push_str("}, {");
            for r in 0..t.row_count() {
                if r > 0 {
                    out.push_str(", ");
                }
                if r >= max_items {
                    let _ = write!(out, "... ({} rows)", t.row_count());
                    break;
                }
                out.push('{');
                for c in 0..t.column_count() {
                    if c > 0 {
                        out.push_str(", ");
                    }
                    if let Some(cell) = t.cell(r, c) {
                        render_slot(cell, depth - 1, max_items, out);
                    }
                }
                out.push('}');
            }
            out.push_str("})");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_format_like_m() {
        assert_eq!(number_to_text(1.0), "1");
        assert_eq!(number_to_text(-42.0), "-42");
        assert_eq!(number_to_text(1.5), "1.5");
        assert_eq!(number_to_text(0.1 + 0.2), "0.3");
        assert_eq!(number_to_text(1e15), "1E+15");
        assert_eq!(number_to_text(123456.789), "123456.789");
        assert_eq!(number_to_text(0.0001), "0.0001");
        assert_eq!(number_to_text(0.00001), "1E-05");
        assert_eq!(number_to_text(-2.5e-7), "-2.5E-07");
        assert_eq!(number_to_text(f64::NAN), "NaN");
        assert_eq!(number_to_text(f64::NEG_INFINITY), "-Infinity");
    }

    #[test]
    fn durations_format_like_timespan() {
        assert_eq!(
            duration_to_text(TimeDelta::seconds(86_400 + 3_723)),
            "1.01:02:03"
        );
        assert_eq!(duration_to_text(TimeDelta::seconds(-61)), "-00:01:01");
    }
}
