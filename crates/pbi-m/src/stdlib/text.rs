//! `Text.*`, `Character.*`, `Lines.*`, `Splitter.*`, `Combiner.*`, `Replacer.*` and
//! `Comparer.*` functions.
//!
//! Positions and lengths count Unicode scalar values (M counts UTF-16 code units; the two only
//! differ for characters outside the Basic Multilingual Plane).

use std::cmp::Ordering;
use std::sync::Arc;

use super::Builder;
use super::args::{
    arg, count_arg, func_arg, int_arg, items, list_arg, req_list, req_text, text_arg,
};
use super::csv::{decode_bytes, encode_text, split_csv_line};
use super::table::is_ignore_case;
use crate::eval::interp::{Interp, convert_error};
use crate::merror::MError;
use crate::value::format::value_to_text;
use crate::value::ops::equals;
use crate::value::{BinaryValue, FunctionValue, NativeImpl, Value};

pub(super) fn register(b: &mut Builder) {
    b.func("Text.Trim", &["text", "?trim"], trim);
    b.func("Text.TrimStart", &["text", "?trim"], trim_start);
    b.func("Text.TrimEnd", &["text", "?trim"], trim_end);
    b.func("Text.Upper", &["text", "?culture"], upper);
    b.func("Text.Lower", &["text", "?culture"], lower);
    b.func("Text.Proper", &["text", "?culture"], proper);
    b.func("Text.Length", &["text"], length);
    b.func("Text.Start", &["text", "count"], start);
    b.func("Text.End", &["text", "count"], end);
    b.func("Text.Range", &["text", "offset", "?count"], range);
    b.func("Text.Middle", &["text", "start", "?count"], middle);
    b.func("Text.At", &["text", "index"], at);
    b.func(
        "Text.AfterDelimiter",
        &["text", "delimiter", "?index"],
        after_delimiter,
    );
    b.func(
        "Text.BeforeDelimiter",
        &["text", "delimiter", "?index"],
        before_delimiter,
    );
    b.func(
        "Text.BetweenDelimiters",
        &[
            "text",
            "startDelimiter",
            "endDelimiter",
            "?startIndex",
            "?endIndex",
        ],
        between_delimiters,
    );
    b.func("Text.Split", &["text", "separator"], split);
    b.func("Text.SplitAny", &["text", "separators"], split_any);
    b.func(
        "Text.Contains",
        &["text", "substring", "?comparer"],
        contains,
    );
    b.func(
        "Text.StartsWith",
        &["text", "substring", "?comparer"],
        starts_with,
    );
    b.func(
        "Text.EndsWith",
        &["text", "substring", "?comparer"],
        ends_with,
    );
    b.func(
        "Text.PositionOf",
        &["text", "substring", "?occurrence", "?comparer"],
        position_of,
    );
    b.func("Text.Replace", &["text", "old", "new"], replace);
    b.func("Text.Combine", &["texts", "?separator"], combine);
    b.func("Text.From", &["value", "?culture"], from);
    b.func("Text.PadStart", &["text", "count", "?character"], pad_start);
    b.func("Text.PadEnd", &["text", "count", "?character"], pad_end);
    b.func("Text.Remove", &["text", "removeChars"], remove);
    b.func("Text.Select", &["text", "selectChars"], select);
    b.func("Text.Insert", &["text", "offset", "newText"], insert);
    b.func("Text.Repeat", &["text", "count"], repeat);
    b.func("Text.Reverse", &["text"], reverse);
    b.func("Text.ToList", &["text"], to_list);
    b.func("Text.Clean", &["text"], clean);
    b.func(
        "Text.Format",
        &["formatString", "arguments", "?culture"],
        format,
    );
    b.func(
        "Text.ToBinary",
        &["text", "?encoding", "?includeByteOrderMark"],
        to_binary,
    );
    b.func("Text.FromBinary", &["binary", "?encoding"], from_binary);
    b.func("Character.FromNumber", &["number"], char_from_number);
    b.func("Character.ToNumber", &["character"], char_to_number);
    b.func(
        "Lines.FromText",
        &["text", "?quoteStyle", "?includeLineSeparators"],
        lines_from_text,
    );
    b.func(
        "Lines.FromBinary",
        &[
            "binary",
            "?quoteStyle",
            "?includeLineSeparators",
            "?encoding",
        ],
        lines_from_binary,
    );
    b.func("Lines.ToText", &["lines", "?lineSeparator"], lines_to_text);
    b.func(
        "Splitter.SplitTextByDelimiter",
        &["delimiter", "?quoteStyle"],
        splitter_by_delimiter,
    );
    b.func(
        "Splitter.SplitTextByAnyDelimiter",
        &["delimiters", "?quoteStyle"],
        splitter_by_any_delimiter,
    );
    b.func("Splitter.SplitByNothing", &[], splitter_by_nothing);
    b.func(
        "Combiner.CombineTextByDelimiter",
        &["delimiter", "?quoteStyle"],
        combiner_by_delimiter,
    );
    b.func(
        "Replacer.ReplaceText",
        &["text", "old", "new"],
        replacer_text,
    );
    b.func(
        "Replacer.ReplaceValue",
        &["value", "old", "new"],
        replacer_value,
    );
    b.func("Comparer.Ordinal", &["x", "y"], comparer_ordinal);
    b.func(
        "Comparer.OrdinalIgnoreCase",
        &["x", "y"],
        comparer_ordinal_ignore_case,
    );
    b.func(
        "Comparer.FromCulture",
        &["culture", "?ignoreCase"],
        comparer_from_culture,
    );
    b.func("Comparer.Equals", &["comparer", "x", "y"], comparer_equals);
}

/// Applies `f` to the first (text) argument; `null` text gives `null`.
fn with_text(
    args: &[Value],
    f: impl FnOnce(&str) -> Result<Value, MError>,
) -> Result<Value, MError> {
    match text_arg(args, 0, "")? {
        None => Ok(Value::Null),
        Some(t) => f(t),
    }
}

fn text(s: impl Into<Arc<str>>) -> Result<Value, MError> {
    Ok(Value::Text(s.into()))
}

/// Characters selected by a `trim`/`removeChars` argument: a text (each character) or a list.
fn char_set(it: &Interp, v: &Value) -> Result<Option<Vec<char>>, MError> {
    match v.plain() {
        Value::Null => Ok(None),
        Value::Text(t) => Ok(Some(t.chars().collect())),
        Value::List(l) => {
            let mut set = Vec::new();
            for item in items(it, l)? {
                match item {
                    Value::Text(t) => set.extend(t.chars()),
                    other => return Err(convert_error(&other, "Text")),
                }
            }
            Ok(Some(set))
        }
        other => Err(convert_error(other, "Text")),
    }
}

fn trim_impl(it: &Interp, args: &[Value], start: bool, end: bool) -> Result<Value, MError> {
    let set = char_set(it, arg(args, 1))?;
    with_text(args, |t| {
        let pred = |c: char| match &set {
            Some(s) => s.contains(&c),
            None => c.is_whitespace(),
        };
        let mut s = t;
        if start {
            s = s.trim_start_matches(pred);
        }
        if end {
            s = s.trim_end_matches(pred);
        }
        text(s)
    })
}

fn trim(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    trim_impl(it, args, true, true)
}

fn trim_start(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    trim_impl(it, args, true, false)
}

fn trim_end(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    trim_impl(it, args, false, true)
}

fn upper(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    with_text(args, |t| text(t.to_uppercase()))
}

fn lower(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    with_text(args, |t| text(t.to_lowercase()))
}

fn proper(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    with_text(args, |t| {
        let mut out = String::with_capacity(t.len());
        let mut prev_letter = false;
        for c in t.chars() {
            if prev_letter {
                out.extend(c.to_lowercase());
            } else {
                out.extend(c.to_uppercase());
            }
            prev_letter = c.is_alphabetic();
        }
        text(out)
    })
}

fn length(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    with_text(args, |t| Ok(Value::Number(t.chars().count() as f64)))
}

fn byte_offset(t: &str, chars: usize) -> Option<usize> {
    if chars == 0 {
        return Some(0);
    }
    t.char_indices()
        .nth(chars)
        .map(|(i, _)| i)
        .or_else(|| (t.chars().count() == chars).then_some(t.len()))
}

fn start(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let n = count_arg(args, 1)?.unwrap_or(0);
    with_text(args, |t| text(&t[..byte_offset(t, n).unwrap_or(t.len())]))
}

fn end(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let n = count_arg(args, 1)?.unwrap_or(0);
    with_text(args, |t| {
        let len = t.chars().count();
        let skip = len.saturating_sub(n);
        text(&t[byte_offset(t, skip).unwrap_or(t.len())..])
    })
}

fn range(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let offset = count_arg(args, 1)?.unwrap_or(0);
    let count = count_arg(args, 2)?;
    with_text(args, |t| {
        let len = t.chars().count();
        if offset > len {
            return Err(MError::expression("The 'offset' argument is out of range."));
        }
        let end = match count {
            Some(c) if offset.saturating_add(c) > len => {
                return Err(MError::expression("The 'count' argument is out of range."));
            }
            Some(c) => offset + c,
            None => len,
        };
        let (a, b) = (
            byte_offset(t, offset).unwrap_or(t.len()),
            byte_offset(t, end).unwrap_or(t.len()),
        );
        text(&t[a..b])
    })
}

fn middle(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let offset = count_arg(args, 1)?.unwrap_or(0);
    let count = count_arg(args, 2)?;
    with_text(args, |t| {
        let a = byte_offset(t, offset).unwrap_or(t.len());
        let rest = &t[a..];
        match count {
            Some(c) => text(&rest[..byte_offset(rest, c).unwrap_or(rest.len())]),
            None => text(rest),
        }
    })
}

fn at(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let i = count_arg(args, 1)?.unwrap_or(0);
    with_text(args, |t| match t.chars().nth(i) {
        Some(c) => text(c.to_string()),
        None => Err(MError::expression("The 'index' argument is out of range.")),
    })
}

/// Byte positions of non-overlapping occurrences of `needle`.
fn occurrences(t: &str, needle: &str) -> Vec<usize> {
    if needle.is_empty() {
        return Vec::new();
    }
    t.match_indices(needle).map(|(i, _)| i).collect()
}

/// `index` argument: a number or `{number, RelativePosition}`; returns (index, from_end).
fn delimiter_index(it: &Interp, v: &Value) -> Result<(usize, bool), MError> {
    match v.plain() {
        Value::Null => Ok((0, false)),
        Value::Number(_) => Ok((count_arg(std::slice::from_ref(v), 0)?.unwrap_or(0), false)),
        Value::List(l) => {
            let parts = items(it, l)?;
            let idx = count_arg(&parts, 0)?.unwrap_or(0);
            let from_end = parts.get(1).and_then(Value::as_number) == Some(1.0);
            Ok((idx, from_end))
        }
        other => Err(convert_error(other, "Number")),
    }
}

fn find_delimiter(t: &str, delim: &str, index: (usize, bool)) -> Option<usize> {
    let occ = occurrences(t, delim);
    if index.1 {
        occ.len()
            .checked_sub(index.0 + 1)
            .and_then(|i| occ.get(i).copied())
    } else {
        occ.get(index.0).copied()
    }
}

fn after_delimiter(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let delim = req_text(args, 1)?;
    let index = delimiter_index(it, arg(args, 2))?;
    with_text(args, |t| match find_delimiter(t, delim, index) {
        Some(p) => text(&t[p + delim.len()..]),
        None => text(""),
    })
}

fn before_delimiter(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let delim = req_text(args, 1)?;
    let index = delimiter_index(it, arg(args, 2))?;
    with_text(args, |t| match find_delimiter(t, delim, index) {
        Some(p) => text(&t[..p]),
        None => text(t),
    })
}

fn between_delimiters(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let start_delim = req_text(args, 1)?;
    let end_delim = req_text(args, 2)?;
    let start_index = delimiter_index(it, arg(args, 3))?;
    let end_index = delimiter_index(it, arg(args, 4))?;
    with_text(args, |t| {
        let Some(s) = find_delimiter(t, start_delim, start_index) else {
            return text("");
        };
        let rest = &t[s + start_delim.len()..];
        match find_delimiter(rest, end_delim, end_index) {
            Some(e) => text(&rest[..e]),
            None => text(rest),
        }
    })
}

fn split(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let sep = req_text(args, 1)?;
    with_text(args, |t| {
        if sep.is_empty() {
            return Ok(Value::list([Value::text(t)]));
        }
        Ok(Value::list(t.split(sep).map(Value::text)))
    })
}

fn split_any(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let seps: Vec<char> = req_text(args, 1)?.chars().collect();
    with_text(args, |t| {
        Ok(Value::list(t.split(|c| seps.contains(&c)).map(Value::text)))
    })
}

fn compare_args(args: &[Value], i: usize) -> bool {
    is_ignore_case(arg(args, i))
}

fn text_predicate(args: &[Value], f: impl Fn(&str, &str) -> bool) -> Result<Value, MError> {
    let ignore_case = compare_args(args, 2);
    let sub = req_text(args, 1)?;
    with_text(args, |t| {
        Ok(Value::Logical(if ignore_case {
            f(&t.to_lowercase(), &sub.to_lowercase())
        } else {
            f(t, sub)
        }))
    })
}

fn contains(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    text_predicate(args, |t, s| t.contains(s))
}

fn starts_with(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    text_predicate(args, |t, s| t.starts_with(s))
}

fn ends_with(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    text_predicate(args, |t, s| t.ends_with(s))
}

fn position_of(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let sub = req_text(args, 1)?;
    let occurrence = int_arg(args, 2)?.unwrap_or(0);
    let ignore_case = compare_args(args, 3);
    with_text(args, |t| {
        let (hay, needle) = if ignore_case {
            (t.to_lowercase(), sub.to_lowercase())
        } else {
            (t.to_owned(), sub.to_owned())
        };
        let positions: Vec<f64> = occurrences(&hay, &needle)
            .into_iter()
            .map(|b| hay[..b].chars().count() as f64)
            .collect();
        Ok(match occurrence {
            1 => Value::Number(positions.last().copied().unwrap_or(-1.0)),
            2 => Value::list(positions.into_iter().map(Value::Number)),
            _ => Value::Number(positions.first().copied().unwrap_or(-1.0)),
        })
    })
}

fn replace(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let old = req_text(args, 1)?;
    let new = req_text(args, 2)?;
    with_text(args, |t| {
        if old.is_empty() {
            return Err(MError::expression("The 'old' argument cannot be empty."));
        }
        text(t.replace(old, new))
    })
}

fn combine(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let list = req_list(args, 0)?;
    let sep = text_arg(args, 1, "Text.Combine")?.unwrap_or("");
    let mut out = String::new();
    let mut first = true;
    for v in items(it, list)? {
        let s = match &v {
            Value::Null => continue,
            Value::Text(t) => t.to_string(),
            other => return Err(convert_error(other, "Text")),
        };
        if !first {
            out.push_str(sep);
        }
        first = false;
        out.push_str(&s);
        it.check_bytes(out.len())?;
    }
    text(out)
}

fn from(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    super::convert::to_text(arg(args, 0))
}

fn pad(it: &Interp, args: &[Value], at_start: bool) -> Result<Value, MError> {
    let count = count_arg(args, 1)?.unwrap_or(0);
    it.check_bytes(count)?;
    let pad_char = match text_arg(args, 2, "")? {
        None => ' ',
        Some(p) => {
            let mut chars = p.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => c,
                _ => {
                    return Err(MError::expression(
                        "The padding character must be a single character.",
                    ));
                }
            }
        }
    };
    with_text(args, |t| {
        let len = t.chars().count();
        if len >= count {
            return text(t);
        }
        let padding: String = std::iter::repeat_n(pad_char, count - len).collect();
        text(if at_start {
            format!("{padding}{t}")
        } else {
            format!("{t}{padding}")
        })
    })
}

fn pad_start(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    pad(it, args, true)
}

fn pad_end(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    pad(it, args, false)
}

fn remove(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let set = char_set(it, arg(args, 1))?.unwrap_or_default();
    with_text(args, |t| {
        text(t.chars().filter(|c| !set.contains(c)).collect::<String>())
    })
}

fn select(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let set = char_set(it, arg(args, 1))?.unwrap_or_default();
    with_text(args, |t| {
        text(t.chars().filter(|c| set.contains(c)).collect::<String>())
    })
}

fn insert(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let offset = count_arg(args, 1)?.unwrap_or(0);
    let new = req_text(args, 2)?;
    with_text(args, |t| {
        let Some(b) = byte_offset(t, offset) else {
            return Err(MError::expression("The 'offset' argument is out of range."));
        };
        text(format!("{}{new}{}", &t[..b], &t[b..]))
    })
}

fn repeat(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let count = count_arg(args, 1)?.unwrap_or(0);
    with_text(args, |t| {
        it.check_bytes(t.len().saturating_mul(count))?;
        text(t.repeat(count))
    })
}

fn reverse(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    with_text(args, |t| text(t.chars().rev().collect::<String>()))
}

fn to_list(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    with_text(args, |t| {
        Ok(Value::list(t.chars().map(|c| Value::text(c.to_string()))))
    })
}

fn clean(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    with_text(args, |t| {
        text(t.chars().filter(|c| !c.is_control()).collect::<String>())
    })
}

fn format(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let fmt = req_text(args, 0)?;
    let arguments = arg(args, 1);
    let lookup = |key: &str| -> Result<Value, MError> {
        match arguments.plain() {
            Value::List(l) => {
                let i: usize = key.parse().map_err(|_| {
                    MError::expression(format!("Invalid format placeholder '{key}'."))
                })?;
                l.items()
                    .get(i)
                    .ok_or_else(|| MError::expression(format!("Format argument {i} is missing.")))?
                    .force(it)
            }
            Value::Record(r) => r
                .get(key)
                .ok_or_else(|| MError::expression(format!("Format argument '{key}' is missing.")))?
                .force(it),
            other => Err(convert_error(other, "List")),
        }
    };
    let mut out = String::new();
    let mut rest = fmt;
    while let Some(i) = rest.find('#') {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        let close = match tail.chars().next() {
            Some('{') => '}',
            Some('[') => ']',
            _ => {
                out.push('#');
                rest = tail;
                continue;
            }
        };
        let Some(j) = tail.find(close) else {
            out.push('#');
            rest = tail;
            continue;
        };
        let v = lookup(&tail[1..j])?;
        out.push_str(&value_to_text(&v).unwrap_or_default());
        rest = &tail[j + 1..];
    }
    out.push_str(rest);
    text(out)
}

fn encoding_arg(args: &[Value], i: usize) -> Result<Option<u32>, MError> {
    Ok(int_arg(args, i)?.and_then(|n| u32::try_from(n).ok()))
}

fn to_binary(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let enc = encoding_arg(args, 1)?;
    with_text(args, |t| {
        Ok(Value::Binary(BinaryValue::from_bytes(encode_text(t, enc)?)))
    })
}

fn from_binary(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let enc = encoding_arg(args, 1)?;
    match arg(args, 0) {
        Value::Null => Ok(Value::Null),
        Value::Binary(b) => text(decode_bytes(
            &b.bytes(it.opts.limits.max_binary_bytes)?,
            enc,
        )?),
        other => Err(convert_error(other, "Binary")),
    }
}

fn char_from_number(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    match int_arg(args, 0)? {
        None => Ok(Value::Null),
        Some(n) => u32::try_from(n)
            .ok()
            .and_then(char::from_u32)
            .map(|c| Value::text(c.to_string()))
            .ok_or_else(|| MError::expression("The number is not a valid character code.")),
    }
}

fn char_to_number(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    with_text(args, |t| {
        let mut chars = t.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Ok(Value::Number(f64::from(u32::from(c)))),
            _ => Err(MError::expression(
                "Character.ToNumber expects a single character.",
            )),
        }
    })
}

fn split_lines(t: &str, keep_separators: bool) -> Vec<Value> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = t.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let sep_len = match bytes[i] {
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => 2,
            b'\r' | b'\n' => 1,
            _ => {
                i += 1;
                continue;
            }
        };
        let end = if keep_separators { i + sep_len } else { i };
        out.push(Value::text(&t[start..end]));
        i += sep_len;
        start = i;
    }
    if start < t.len() {
        out.push(Value::text(&t[start..]));
    }
    out
}

fn lines_from_text(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let keep = super::args::bool_arg(args, 2)?.unwrap_or(false);
    with_text(args, |t| Ok(Value::list(split_lines(t, keep))))
}

fn lines_from_binary(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let keep = super::args::bool_arg(args, 2)?.unwrap_or(false);
    let enc = encoding_arg(args, 3)?;
    match arg(args, 0) {
        Value::Null => Ok(Value::Null),
        Value::Binary(b) => {
            let t = decode_bytes(&b.bytes(it.opts.limits.max_binary_bytes)?, enc)?;
            let lines = split_lines(&t, keep);
            it.check_rows(lines.len())?;
            Ok(Value::list(lines))
        }
        other => Err(convert_error(other, "Binary")),
    }
}

fn lines_to_text(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let sep = text_arg(args, 1, "")?.unwrap_or("\r\n");
    let mut out = String::new();
    for v in items(it, req_list(args, 0)?)? {
        out.push_str(v.as_text().ok_or_else(|| convert_error(&v, "Text"))?);
        out.push_str(sep);
        it.check_bytes(out.len())?;
    }
    text(out)
}

fn dyn_fn(
    name: &str,
    params: &[(&str, bool)],
    f: impl Fn(&Interp, &[Value]) -> Result<Value, MError> + Send + Sync + 'static,
) -> Value {
    Value::Function(FunctionValue::native(
        name,
        params,
        NativeImpl::Dyn(Arc::new(f)),
    ))
}

fn splitter_by_delimiter(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let delim = req_text(args, 0)?.to_owned();
    let csv = super::args::num_arg(args, 1)? == Some(1.0);
    Ok(dyn_fn(
        "Splitter.SplitTextByDelimiter",
        &[("line", false)],
        move |_, a| {
            with_text(a, |t| {
                let parts: Vec<String> = if csv {
                    split_csv_line(t, &delim)
                } else if delim.is_empty() {
                    vec![t.to_owned()]
                } else {
                    t.split(delim.as_str()).map(str::to_owned).collect()
                };
                Ok(Value::list(parts.into_iter().map(Value::text)))
            })
        },
    ))
}

fn splitter_by_any_delimiter(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let delims: Vec<String> = items(it, req_list(args, 0)?)?
        .into_iter()
        .map(|v| {
            v.as_text()
                .map(str::to_owned)
                .ok_or_else(|| convert_error(&v, "Text"))
        })
        .collect::<Result<_, _>>()?;
    Ok(dyn_fn(
        "Splitter.SplitTextByAnyDelimiter",
        &[("line", false)],
        move |_, a| {
            with_text(a, |t| {
                let mut parts = Vec::new();
                let mut rest = t;
                loop {
                    let next = delims
                        .iter()
                        .filter(|d| !d.is_empty())
                        .filter_map(|d| rest.find(d.as_str()).map(|i| (i, d.len())))
                        .min();
                    match next {
                        Some((i, len)) => {
                            parts.push(Value::text(&rest[..i]));
                            rest = &rest[i + len..];
                        }
                        None => {
                            parts.push(Value::text(rest));
                            break;
                        }
                    }
                }
                Ok(Value::list(parts))
            })
        },
    ))
}

fn splitter_by_nothing(_: &Interp, _: &[Value]) -> Result<Value, MError> {
    Ok(dyn_fn(
        "Splitter.SplitByNothing",
        &[("value", false)],
        |_, a| Ok(Value::list([arg(a, 0).clone()])),
    ))
}

fn combiner_by_delimiter(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let delim = req_text(args, 0)?.to_owned();
    Ok(dyn_fn(
        "Combiner.CombineTextByDelimiter",
        &[("texts", false)],
        move |it, a| {
            let parts: Vec<String> = match list_arg(a, 0)? {
                Some(l) => items(it, l)?
                    .iter()
                    .map(|v| value_to_text(v).unwrap_or_default())
                    .collect(),
                None => Vec::new(),
            };
            text(parts.join(&delim))
        },
    ))
}

fn replacer_text(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    match (arg(args, 0), arg(args, 1), arg(args, 2)) {
        (Value::Text(t), Value::Text(old), Value::Text(new)) if !old.is_empty() => {
            text(t.replace(&**old, new))
        }
        (v, _, _) => Ok(v.clone()),
    }
}

fn replacer_value(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let (v, old, new) = (arg(args, 0), arg(args, 1), arg(args, 2));
    Ok(if equals(it, v, old, 0)? {
        new.clone()
    } else {
        v.clone()
    })
}

fn ordering_value(o: Ordering) -> Value {
    Value::Number(match o {
        Ordering::Less => -1.0,
        Ordering::Equal => 0.0,
        Ordering::Greater => 1.0,
    })
}

fn comparer_ordinal(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(ordering_value(crate::value::ops::sort_compare(
        arg(args, 0),
        arg(args, 1),
    )))
}

fn comparer_ordinal_ignore_case(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    match (arg(args, 0), arg(args, 1)) {
        (Value::Text(a), Value::Text(b)) => {
            Ok(ordering_value(a.to_lowercase().cmp(&b.to_lowercase())))
        }
        (a, b) => Ok(ordering_value(crate::value::ops::sort_compare(a, b))),
    }
}

fn comparer_from_culture(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let ignore_case = super::args::bool_arg(args, 1)?.unwrap_or(false);
    let name = if ignore_case {
        "Comparer.OrdinalIgnoreCase"
    } else {
        "Comparer.Ordinal"
    };
    super::library()
        .get(name)
        .cloned()
        .ok_or_else(|| MError::Internal(format!("{name} missing")))
}

fn comparer_equals(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let f = func_arg(args, 0)?.ok_or_else(|| convert_error(&Value::Null, "Function"))?;
    let r = it.call(f, &[arg(args, 1).clone(), arg(args, 2).clone()])?;
    Ok(Value::Logical(r.as_number() == Some(0.0)))
}
