//! `Csv.Document` and text encodings.

use std::sync::Arc;

use encoding_rs::Encoding;

use super::Builder;
use super::args::{arg, items, option};
use crate::eval::interp::{Interp, convert_error};
use crate::merror::MError;
use crate::value::{Lazy, RecordValue, TableValue, Value};

/// Maximum number of fields in one CSV row.
const MAX_COLUMNS: usize = 16_384;

pub(super) fn register(b: &mut Builder) {
    b.func(
        "Csv.Document",
        &[
            "source",
            "?columns",
            "?delimiter",
            "?extraValues",
            "?encoding",
        ],
        csv_document,
    );
}

fn encoding_for(codepage: u32) -> Option<&'static Encoding> {
    use encoding_rs as e;
    Some(match codepage {
        65001 => e::UTF_8,
        1200 => e::UTF_16LE,
        1201 => e::UTF_16BE,
        1250 => e::WINDOWS_1250,
        1251 => e::WINDOWS_1251,
        1252 | 28591 | 20127 => e::WINDOWS_1252,
        1253 => e::WINDOWS_1253,
        1254 => e::WINDOWS_1254,
        1255 => e::WINDOWS_1255,
        1256 => e::WINDOWS_1256,
        1257 => e::WINDOWS_1257,
        1258 => e::WINDOWS_1258,
        874 => e::WINDOWS_874,
        866 => e::IBM866,
        28592 => e::ISO_8859_2,
        28593 => e::ISO_8859_3,
        28594 => e::ISO_8859_4,
        28595 => e::ISO_8859_5,
        28596 => e::ISO_8859_6,
        28597 => e::ISO_8859_7,
        28598 => e::ISO_8859_8,
        28603 => e::ISO_8859_13,
        28605 => e::ISO_8859_15,
        932 => e::SHIFT_JIS,
        936 => e::GBK,
        54936 => e::GB18030,
        949 => e::EUC_KR,
        950 => e::BIG5,
        51932 => e::EUC_JP,
        50220 => e::ISO_2022_JP,
        20866 => e::KOI8_R,
        21866 => e::KOI8_U,
        10000 => e::MACINTOSH,
        _ => return None,
    })
}

fn unsupported_encoding(codepage: u32) -> MError {
    MError::expression(format!("The text encoding {codepage} is not supported."))
}

/// Decodes bytes with a Windows code page (`None`: UTF-8, or the encoding of a byte order
/// mark). Byte order marks are removed; invalid sequences become U+FFFD.
pub(crate) fn decode_bytes(bytes: &[u8], codepage: Option<u32>) -> Result<String, MError> {
    let encoding = match codepage {
        None => encoding_rs::UTF_8,
        Some(cp) => encoding_for(cp).ok_or_else(|| unsupported_encoding(cp))?,
    };
    let text = if codepage.is_none() {
        // Sniffs a byte order mark, else UTF-8.
        encoding.decode(bytes).0
    } else {
        encoding.decode_with_bom_removal(bytes).0
    };
    Ok(text.into_owned())
}

/// Encodes text with a Windows code page (default UTF-8).
pub(crate) fn encode_text(text: &str, codepage: Option<u32>) -> Result<Vec<u8>, MError> {
    match codepage.unwrap_or(65001) {
        65001 => Ok(text.as_bytes().to_vec()),
        1200 => Ok(text.encode_utf16().flat_map(u16::to_le_bytes).collect()),
        1201 => Ok(text.encode_utf16().flat_map(u16::to_be_bytes).collect()),
        cp => {
            let enc = encoding_for(cp).ok_or_else(|| unsupported_encoding(cp))?;
            Ok(enc.encode(text).0.into_owned())
        }
    }
}

struct CsvOptions<'a> {
    delimiter: &'a str,
    /// `QuoteStyle.Csv`: line breaks inside quotes are data.
    quoted_newlines: bool,
    /// `CsvStyle.QuoteAlways`: quotes are significant anywhere in a field.
    quote_always: bool,
}

/// Parses delimited text into rows of fields.
fn parse(
    text: &str,
    opts: &CsvOptions<'_>,
    mut on_row: impl FnMut(usize) -> Result<(), MError>,
) -> Result<Vec<Vec<String>>, MError> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut field_start = true;
    let mut quote_line = 0usize;
    let mut line = 1usize;
    // Whether anything of the current row has been read.
    let mut dirty = false;
    let delim = opts.delimiter;
    let mut rest = text;
    let end_field = |row: &mut Vec<String>, field: &mut String| -> Result<(), MError> {
        if row.len() >= MAX_COLUMNS {
            return Err(MError::data_format(format!(
                "A CSV row has more than {MAX_COLUMNS} fields."
            )));
        }
        row.push(std::mem::take(field));
        Ok(())
    };
    while let Some(c) = rest.chars().next() {
        dirty = true;
        if in_quotes {
            if c == '"' {
                if rest[1..].starts_with('"') {
                    field.push('"');
                    rest = &rest[2..];
                } else {
                    in_quotes = false;
                    rest = &rest[1..];
                }
                continue;
            }
            if (c == '\r' || c == '\n') && !opts.quoted_newlines {
                in_quotes = false;
                // fall through to the line-break handling below
            } else {
                if c == '\n' || (c == '\r' && !rest[1..].starts_with('\n')) {
                    line += 1;
                }
                field.push(c);
                rest = &rest[c.len_utf8()..];
                continue;
            }
        }
        if !delim.is_empty() && rest.starts_with(delim) {
            end_field(&mut row, &mut field)?;
            field_start = true;
            rest = &rest[delim.len()..];
            continue;
        }
        if c == '\r' || c == '\n' {
            end_field(&mut row, &mut field)?;
            rows.push(std::mem::take(&mut row));
            on_row(rows.len())?;
            field_start = true;
            dirty = false;
            line += 1;
            rest = rest
                .strip_prefix("\r\n")
                .unwrap_or_else(|| &rest[c.len_utf8()..]);
            continue;
        }
        if c == '"' && (field_start || opts.quote_always) {
            in_quotes = true;
            quote_line = line;
            field_start = false;
            rest = &rest[1..];
            continue;
        }
        field.push(c);
        field_start = false;
        rest = &rest[c.len_utf8()..];
    }
    if in_quotes {
        return Err(MError::data_format(format!(
            "The CSV data is malformed: a quoted field starting on line {quote_line} is never closed."
        )));
    }
    if dirty {
        end_field(&mut row, &mut field)?;
        rows.push(row);
        on_row(rows.len())?;
    }
    Ok(rows)
}

/// Splits one line with CSV quoting (`Splitter.SplitTextByDelimiter(d, QuoteStyle.Csv)`).
pub(crate) fn split_csv_line(line: &str, delimiter: &str) -> Vec<String> {
    let opts = CsvOptions {
        delimiter,
        quoted_newlines: true,
        quote_always: false,
    };
    match parse(line, &opts, |_| Ok(())) {
        Ok(rows) => rows.into_iter().flatten().collect(),
        Err(_) => vec![line.to_owned()],
    }
}

fn csv_document(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let source = arg(args, 0);
    let options: Option<&RecordValue> = arg(args, 1).as_record();
    let (columns, delimiter, extra, encoding, quote_style, csv_style) = match options {
        Some(o) => (
            option(it, Some(o), "Columns")?,
            option(it, Some(o), "Delimiter")?,
            option(it, Some(o), "ExtraValues")?,
            option(it, Some(o), "Encoding")?,
            option(it, Some(o), "QuoteStyle")?,
            option(it, Some(o), "CsvStyle")?,
        ),
        None => {
            let opt = |i| {
                let v = arg(args, i);
                if v.is_null() { None } else { Some(v.clone()) }
            };
            (opt(1), opt(2), opt(3), opt(4), None, None)
        }
    };
    let delimiter = match &delimiter {
        None => ",".to_owned(),
        Some(Value::Text(t)) => t.to_string(),
        Some(other) => return Err(convert_error(other, "Text")),
    };
    let codepage = match &encoding {
        None => None,
        Some(Value::Number(n)) if n.fract() == 0.0 && *n > 0.0 => u32::try_from(*n as u64).ok(),
        Some(other) => return Err(convert_error(other, "Number")),
    };
    let text = match source.plain() {
        Value::Binary(b) => decode_bytes(&b.bytes(it.opts.limits.max_binary_bytes)?, codepage)?,
        Value::Text(t) => t.to_string(),
        Value::List(l) => {
            let mut s = String::new();
            for line in items(it, l)? {
                s.push_str(line.as_text().ok_or_else(|| convert_error(&line, "Text"))?);
                s.push('\n');
            }
            s
        }
        other => return Err(convert_error(other, "Binary")),
    };
    let opts = CsvOptions {
        delimiter: &delimiter,
        quoted_newlines: quote_style.as_ref().and_then(Value::as_number) == Some(1.0),
        quote_always: csv_style.as_ref().and_then(Value::as_number) == Some(1.0),
    };
    let max_rows = it.opts.limits.max_rows;
    let rows = parse(&text, &opts, |n| {
        it.tick()?;
        if n > max_rows {
            it.check_rows(n)?;
        }
        Ok(())
    })?;
    drop(text);

    let names: Vec<Arc<str>> = match &columns {
        None => {
            let width = rows.iter().map(Vec::len).max().unwrap_or(0);
            (1..=width)
                .map(|i| Arc::from(format!("Column{i}")))
                .collect()
        }
        Some(Value::Number(n)) if n.fract() == 0.0 && *n >= 0.0 && *n <= MAX_COLUMNS as f64 => (1
            ..=(*n as usize))
            .map(|i| Arc::from(format!("Column{i}")))
            .collect(),
        Some(v @ Value::List(_)) => super::args::names_arg(it, v)?
            .into_iter()
            .map(Arc::from)
            .collect(),
        Some(other) => return Err(convert_error(other, "Number")),
    };
    let width = names.len();
    // ExtraValues.List = 0, .Error = 1, .Ignore = 2 (default: ignore).
    let extra_mode = extra.as_ref().and_then(Value::as_number);
    let mut columns: Vec<Vec<Lazy>> = (0..width).map(|_| Vec::with_capacity(rows.len())).collect();
    for (r, row) in rows.into_iter().enumerate() {
        if row.len() > width && width > 0 {
            match extra_mode {
                Some(1.0) => {
                    return Err(MError::data_format(format!(
                        "Row {} of the CSV data has {} fields but {width} columns were expected.",
                        r + 1,
                        row.len()
                    )));
                }
                Some(0.0) => {
                    let mut fields = row.into_iter();
                    for col in columns.iter_mut().take(width - 1) {
                        col.push(Lazy::Value(fields.next().map_or(Value::Null, Value::from)));
                    }
                    let rest: Vec<Value> = fields.map(Value::from).collect();
                    if let Some(last) = columns.last_mut() {
                        last.push(Lazy::Value(Value::list(rest)));
                    }
                    continue;
                }
                _ => {}
            }
        }
        let mut fields = row.into_iter();
        for col in &mut columns {
            col.push(Lazy::Value(fields.next().map_or(Value::Null, Value::from)));
        }
    }
    let table = TableValue::new(
        names,
        vec![crate::value::TypeValue::any(); width],
        columns.into_iter().map(Arc::new).collect(),
    )?;
    Ok(Value::Table(table))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(text: &str, quoted_newlines: bool) -> Result<Vec<Vec<String>>, MError> {
        parse(
            text,
            &CsvOptions {
                delimiter: ",",
                quoted_newlines,
                quote_always: false,
            },
            |_| Ok(()),
        )
    }

    #[test]
    fn parses_quotes_and_newlines() {
        let r = rows("a,b\r\n\"x, \"\"y\"\"\",\"multi\nline\"\n", true).unwrap_or_default();
        assert_eq!(r, vec![vec!["a", "b"], vec!["x, \"y\"", "multi\nline"]]);
        let r = rows("a,\"b\nc\"\n", false).unwrap_or_default();
        assert_eq!(r, vec![vec!["a", "b"], vec!["c\""]]);
        assert_eq!(
            rows("", true).unwrap_or_default(),
            Vec::<Vec<String>>::new()
        );
        assert_eq!(
            rows("a\n\nb", true).unwrap_or_default(),
            vec![vec!["a"], vec![""], vec!["b"]]
        );
        assert_eq!(rows("a,", true).unwrap_or_default(), vec![vec!["a", ""]]);
    }

    #[test]
    fn unterminated_quote_is_an_error() {
        assert!(rows("a,\"b\nc", true).is_err());
    }

    #[test]
    fn decodes_code_pages() {
        assert_eq!(
            decode_bytes(b"\xEF\xBB\xBFabc", Some(65001)).unwrap_or_default(),
            "abc"
        );
        assert_eq!(decode_bytes(b"\xE9", Some(1252)).unwrap_or_default(), "é");
        assert_eq!(
            decode_bytes(b"a\0b\0", Some(1200)).unwrap_or_default(),
            "ab"
        );
        assert!(decode_bytes(b"x", Some(1)).is_err());
        assert_eq!(encode_text("é", Some(1252)).unwrap_or_default(), vec![0xE9]);
    }
}
