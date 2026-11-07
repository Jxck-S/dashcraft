//! `Binary.*` functions, `BinaryEncoding.*` and `Json.FromValue`.

use std::fmt::Write as _;

use chrono::{NaiveDate, NaiveTime, TimeDelta, Timelike};

use super::Builder;
use super::args::{arg, int_arg, items, req_list};
use super::csv::encode_text;
use super::value_fns::base64_encode;
use crate::eval::interp::{Interp, convert_error};
use crate::merror::MError;
use crate::value::format::number_to_text;
use crate::value::{BinaryValue, ListValue, Value};

pub(super) fn register(b: &mut Builder) {
    b.func("Binary.ToText", &["binary", "?encoding"], to_text);
    b.func("Binary.FromText", &["text", "?encoding"], from_text);
    b.func("Binary.From", &["value", "?encoding"], from);
    b.func("Binary.ToList", &["binary"], to_list);
    b.func("Binary.FromList", &["list"], from_list);
    b.func("Binary.Combine", &["binaries"], combine);
    b.func("Binary.Range", &["binary", "offset", "?count"], range);
    b.func("Binary.Split", &["binary", "pageSize"], split);
    b.func("Binary.ApproximateLength", &["binary"], approximate_length);
    b.func("Binary.Buffer", &["binary"], buffer);
    b.func("Json.FromValue", &["value", "?encoding"], json_from_value);
    b.consts(&[("BinaryEncoding.Base64", 0.0), ("BinaryEncoding.Hex", 1.0)]);
}

fn bytes_of(it: &Interp, b: &BinaryValue) -> Result<std::sync::Arc<[u8]>, MError> {
    b.bytes(it.opts.limits.max_binary_bytes)
}

fn binary(it: &Interp, bytes: Vec<u8>) -> Result<Value, MError> {
    it.check_bytes(bytes.len())?;
    Ok(Value::Binary(BinaryValue::from_bytes(bytes)))
}

/// A required binary argument (`null` is an error).
fn req_binary(it: &Interp, args: &[Value], i: usize) -> Result<std::sync::Arc<[u8]>, MError> {
    match arg(args, i) {
        Value::Binary(b) => bytes_of(it, b),
        other => Err(convert_error(other, "Binary")),
    }
}

fn invalid_encoding() -> MError {
    MError::expression("Invalid binary encoding.")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Base64,
    Hex,
}

fn encoding_arg(args: &[Value], i: usize) -> Result<Encoding, MError> {
    match int_arg(args, i).map_err(|_| invalid_encoding())? {
        None | Some(0) => Ok(Encoding::Base64),
        Some(1) => Ok(Encoding::Hex),
        Some(_) => Err(invalid_encoding()),
    }
}

/// An integral number argument (Desktop's `Int64` arguments).
fn int64_arg(args: &[Value], i: usize) -> Result<Option<i64>, MError> {
    int_arg(args, i)
        .map_err(|_| MError::expression("The number is out of range of a 64 bit integer value."))
}

fn out_of_range(name: &str) -> MError {
    MError::expression(format!("The '{name}' argument is out of range."))
}

fn to_text(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let bytes = match arg(args, 0) {
        Value::Null => return Ok(Value::Null),
        Value::Binary(b) => bytes_of(it, b)?,
        other => return Err(convert_error(other, "Binary")),
    };
    let text = match encoding_arg(args, 1)? {
        Encoding::Base64 => base64_encode(&bytes),
        Encoding::Hex => bytes.iter().fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        }),
    };
    it.check_bytes(text.len())?;
    Ok(Value::text(text))
}

/// Strict base64 (whitespace ignored, length a multiple of four, at most two `=` at the end).
fn base64_strict(text: &str) -> Option<Vec<u8>> {
    let chars: Vec<u8> = text.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    if !chars.len().is_multiple_of(4) {
        return None;
    }
    let padding = chars.iter().rev().take_while(|c| **c == b'=').count();
    if padding > 2 {
        return None;
    }
    let body = chars.get(..chars.len() - padding)?;
    let mut out = Vec::with_capacity(chars.len() / 4 * 3);
    let (mut buf, mut bits) = (0u32, 0u32);
    for &c in body {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        buf = (buf << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn hex_decode(text: &str) -> Result<Vec<u8>, MError> {
    if !text.len().is_multiple_of(2) {
        return Err(MError::data_format(
            "Hex string must contain an even number of characters.",
        ));
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| {
            let digit = |c: u8| {
                char::from(c)
                    .to_digit(16)
                    .ok_or_else(|| MError::data_format("Invalid hex digit."))
            };
            match pair {
                [hi, lo] => Ok((digit(*hi)? * 16 + digit(*lo)?) as u8),
                _ => Err(MError::data_format("Invalid hex digit.")),
            }
        })
        .collect()
}

fn decode(it: &Interp, text: &str, encoding: Encoding) -> Result<Value, MError> {
    it.check_bytes(text.len())?;
    let bytes = match encoding {
        Encoding::Base64 => base64_strict(text).ok_or_else(invalid_encoding)?,
        Encoding::Hex => hex_decode(text)?,
    };
    binary(it, bytes)
}

fn from_text(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    match arg(args, 0) {
        Value::Null => Ok(Value::Null),
        Value::Text(t) => decode(it, t, encoding_arg(args, 1)?),
        other => Err(convert_error(other, "Text")),
    }
}

fn from(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    match arg(args, 0) {
        Value::Null => Ok(Value::Null),
        Value::Text(t) => decode(it, t, encoding_arg(args, 1)?),
        Value::Binary(b) => Ok(Value::Binary(b.clone())),
        _ => Err(MError::data_format("We couldn't convert to Binary.")),
    }
}

fn to_list(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let bytes = req_binary(it, args, 0)?;
    it.check_rows(bytes.len())?;
    Ok(Value::list(
        bytes.iter().map(|b| Value::Number(f64::from(*b))),
    ))
}

fn from_list(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let list = req_list(args, 0)?;
    it.check_bytes(list.len())?;
    let bytes = items(it, list)?
        .iter()
        .map(|v| match v {
            Value::Number(n) if n.fract() != 0.0 || !n.is_finite() => Err(MError::expression(
                "The number is out of range of a 64 bit integer value.",
            )),
            Value::Number(n) if (0.0..=255.0).contains(n) => Ok(*n as u8),
            Value::Number(_) => Err(MError::expression(
                "The number is out of range of a byte value.",
            )),
            other => Err(convert_error(other, "Number")),
        })
        .collect::<Result<Vec<u8>, _>>()?;
    binary(it, bytes)
}

fn combine(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let mut out = Vec::new();
    for v in items(it, req_list(args, 0)?)? {
        match v {
            Value::Binary(b) => {
                out.extend_from_slice(&bytes_of(it, &b)?);
                it.check_bytes(out.len())?;
            }
            other => return Err(convert_error(&other, "Binary")),
        }
    }
    binary(it, out)
}

fn range(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let bytes = req_binary(it, args, 0)?;
    let offset = int64_arg(args, 1)?.ok_or_else(|| convert_error(&Value::Null, "Number"))?;
    let offset = usize::try_from(offset).map_err(|_| out_of_range("offset"))?;
    let count = match int64_arg(args, 2)? {
        None => None,
        Some(n) => Some(usize::try_from(n).map_err(|_| out_of_range("count"))?),
    };
    let start = offset.min(bytes.len());
    let end = count.map_or(bytes.len(), |c| start.saturating_add(c).min(bytes.len()));
    binary(it, bytes.get(start..end).unwrap_or_default().to_vec())
}

fn split(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let bytes = req_binary(it, args, 0)?;
    let page = int64_arg(args, 1)?.ok_or_else(|| convert_error(&Value::Null, "Number"))?;
    let page = usize::try_from(page)
        .ok()
        .filter(|p| *p > 0)
        .ok_or_else(|| out_of_range("pageSize"))?;
    it.check_rows(bytes.len().div_ceil(page))?;
    Ok(Value::list(bytes.chunks(page).map(|c| {
        Value::Binary(BinaryValue::from_bytes(c.to_vec()))
    })))
}

fn approximate_length(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    match arg(args, 0) {
        Value::Null => Ok(Value::Null),
        Value::Binary(b) => Ok(Value::Number(bytes_of(it, b)?.len() as f64)),
        other => Err(convert_error(other, "Binary")),
    }
}

fn buffer(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    match arg(args, 0) {
        Value::Null => Ok(Value::Null),
        Value::Binary(b) => Ok(Value::Binary(BinaryValue::from_bytes(bytes_of(it, b)?))),
        other => Err(convert_error(other, "Binary")),
    }
}

fn json_from_value(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let codepage = int_arg(args, 1)?.and_then(|n| u32::try_from(n).ok());
    let mut out = String::new();
    write_json(it, arg(args, 0), &mut out)?;
    binary(it, encode_text(&out, codepage)?)
}

fn write_json(it: &Interp, v: &Value, out: &mut String) -> Result<(), MError> {
    it.tick()?;
    it.check_bytes(out.len())?;
    match v.plain() {
        Value::Null => out.push_str("null"),
        Value::Logical(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) if !n.is_finite() => out.push_str("null"),
        Value::Number(n) => out.push_str(&number_to_text(if *n == 0.0 { 0.0 } else { *n })),
        Value::Text(t) => write_json_string(t, out),
        Value::Date(d) => write_json_string(&json_date(*d), out),
        Value::Time(t) => write_json_string(&json_time(*t), out),
        Value::DateTime(dt) => write_json_string(
            &format!("{}T{}", json_date(dt.date()), json_time(dt.time())),
            out,
        ),
        Value::DateTimeZone(dt) => write_json_string(
            &format!(
                "{}T{}{}",
                json_date(dt.date_naive()),
                json_time(dt.time()),
                dt.format("%:z")
            ),
            out,
        ),
        Value::Duration(d) => write_json_string(&xml_duration(*d), out),
        Value::Binary(b) => write_json_string(&base64_encode(&bytes_of(it, b)?), out),
        Value::List(l) => write_array(it, l, out)?,
        Value::Record(r) => {
            out.push('{');
            for (i, (name, value)) in r.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_string(name, out);
                out.push(':');
                write_json(it, &value.force(it)?, out)?;
            }
            out.push('}');
        }
        Value::Table(t) => {
            out.push('[');
            for (i, row) in t.rows().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json(it, &Value::Record(row), out)?;
            }
            out.push(']');
        }
        Value::Function(_) | Value::Type(_) | Value::Meta(_) => {
            return Err(MError::expression(
                "We can't convert values of this type to JSON.",
            ));
        }
    }
    Ok(())
}

fn write_array(it: &Interp, l: &ListValue, out: &mut String) -> Result<(), MError> {
    out.push('[');
    for (i, item) in l.items().iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_json(it, &item.force(it)?, out)?;
    }
    out.push(']');
    Ok(())
}

/// A JSON string as Power Query writes it: `"`, `\` and `/` escaped, everything outside
/// printable ASCII as `\uXXXX` (lowercase hex, UTF-16 code units).
fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '/' => out.push_str("\\/"),
            ' '..='~' => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
        }
    }
    out.push('"');
}

fn json_date(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// `HH:mm:ss` with the fraction of a second (in 100-nanosecond ticks) when not zero.
fn json_time(t: NaiveTime) -> String {
    let mut s = t.format("%H:%M:%S").to_string();
    let ticks = (t.nanosecond() % 1_000_000_000) / 100;
    if ticks != 0 {
        let digits = format!("{ticks:07}");
        s.push('.');
        s.push_str(digits.trim_end_matches('0'));
    }
    s
}

/// An ISO 8601 duration as .NET's `XmlConvert` writes it (`P1DT2H3M4S`, `-PT0.5S`, `PT0S`).
fn xml_duration(d: TimeDelta) -> String {
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
    s.push('P');
    if days != 0 {
        let _ = write!(s, "{days}D");
    }
    if hours != 0 || minutes != 0 || seconds != 0 || ticks != 0 || days == 0 {
        s.push('T');
        if hours != 0 {
            let _ = write!(s, "{hours}H");
        }
        if minutes != 0 {
            let _ = write!(s, "{minutes}M");
        }
        if seconds != 0 || ticks != 0 || (days == 0 && hours == 0 && minutes == 0) {
            let _ = write!(s, "{seconds}");
            if ticks != 0 {
                let digits = format!("{ticks:07}");
                s.push('.');
                s.push_str(digits.trim_end_matches('0'));
            }
            s.push('S');
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_base64() {
        assert_eq!(base64_strict(" AQ I= "), Some(vec![1, 2]));
        assert_eq!(base64_strict("AQ=="), Some(vec![1]));
        assert_eq!(base64_strict("AQI"), None);
        assert_eq!(base64_strict("AQI====="), None);
        assert_eq!(base64_strict("A=QI"), None);
    }

    #[test]
    fn xml_durations() {
        let d = |s: i64| TimeDelta::try_seconds(s).unwrap_or_default();
        assert_eq!(xml_duration(d(93_784)), "P1DT2H3M4S");
        assert_eq!(xml_duration(d(86_400)), "P1D");
        assert_eq!(xml_duration(d(0)), "PT0S");
        assert_eq!(
            xml_duration(-(d(86_399) + TimeDelta::try_milliseconds(500).unwrap_or_default())),
            "-PT23H59M59.5S"
        );
    }
}
