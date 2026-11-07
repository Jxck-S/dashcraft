//! `Value.*`, `Type.*`, `Error.Record`, `Function.Invoke`, `Json.Document`, `#binary`, type
//! constants (`Int64.Type`, `Text.Type`, ...) and enumeration constants.

use std::sync::Arc;

use super::Builder;
use super::args::{arg, int_arg, items, list_arg, req_func, req_list, req_text, text_arg};
use super::csv::decode_bytes;
use crate::ast::PrimitiveType;
use crate::eval::interp::{Interp, convert_error};
use crate::merror::{ErrorFields, MError};
use crate::value::format::{duration_to_text, number_to_text};
use crate::value::ops::{equals, sort_compare};
use crate::value::types::type_of;
use crate::value::{
    BinaryValue, Facet, ListValue, RecordValue, TableValue, TypeKind, TypeValue, Value,
};

pub(super) fn register(b: &mut Builder) {
    b.func("#binary", &["value"], hash_binary);
    b.func("Binary.Length", &["binary"], binary_length);
    b.func("Value.Is", &["value", "type"], is);
    b.func("Value.As", &["value", "type"], value_as);
    b.func("Value.Type", &["value"], value_type);
    b.raw_func("Value.Metadata", &["value"], metadata);
    b.raw_func(
        "Value.RemoveMetadata",
        &["value", "?metaValue"],
        remove_metadata,
    );
    b.raw_func(
        "Value.ReplaceMetadata",
        &["value", "metaValue"],
        replace_metadata,
    );
    b.func("Value.ReplaceType", &["value", "type"], replace_type);
    b.func(
        "Value.Equals",
        &["value1", "value2", "?precision"],
        value_equals,
    );
    b.func(
        "Value.NullableEquals",
        &["value1", "value2", "?precision"],
        nullable_equals,
    );
    b.func(
        "Value.Compare",
        &["value1", "value2", "?precision"],
        compare,
    );
    b.func("Type.Is", &["type1", "type2"], type_is);
    b.func("Type.IsNullable", &["type"], type_is_nullable);
    b.func("Type.NonNullable", &["type"], type_non_nullable);
    b.func(
        "Type.TableColumn",
        &["tableType", "column"],
        type_table_column,
    );
    b.func(
        "Error.Record",
        &["reason", "?message", "?detail", "?parameters", "?errorCode"],
        error_record,
    );
    b.func(
        "Diagnostics.Trace",
        &["traceLevel", "message", "value", "?delayed"],
        trace,
    );
    b.func("Function.Invoke", &["function", "args"], function_invoke);
    b.func("Json.Document", &["jsonText", "?encoding"], json_document);

    for (name, p) in [
        ("Any.Type", PrimitiveType::Any),
        ("None.Type", PrimitiveType::None),
        ("Null.Type", PrimitiveType::Null),
        ("Logical.Type", PrimitiveType::Logical),
        ("Number.Type", PrimitiveType::Number),
        ("Text.Type", PrimitiveType::Text),
        ("Date.Type", PrimitiveType::Date),
        ("DateTime.Type", PrimitiveType::DateTime),
        ("DateTimeZone.Type", PrimitiveType::DateTimeZone),
        ("Duration.Type", PrimitiveType::Duration),
        ("Time.Type", PrimitiveType::Time),
        ("Binary.Type", PrimitiveType::Binary),
        ("List.Type", PrimitiveType::List),
        ("Record.Type", PrimitiveType::Record),
        ("Table.Type", PrimitiveType::Table),
        ("Function.Type", PrimitiveType::Function),
        ("Type.Type", PrimitiveType::Type),
    ] {
        b.value(name, Value::Type(TypeValue::primitive(p)));
    }
    for f in Facet::ALL {
        b.value(f.type_name(), Value::Type(TypeValue::facet(f)));
    }
    // The enumerations' types are `type number`.
    for name in [
        "BinaryEncoding.Type",
        "BinaryOccurrence.Type",
        "CsvStyle.Type",
        "Day.Type",
        "ExtraValues.Type",
        "GroupKind.Type",
        "JoinAlgorithm.Type",
        "JoinKind.Type",
        "JoinSide.Type",
        "MissingField.Type",
        "Occurrence.Type",
        "Order.Type",
        "PercentileMode.Type",
        "QuoteStyle.Type",
        "RelativePosition.Type",
        "RoundingMode.Type",
        "TextEncoding.Type",
        "TraceLevel.Type",
    ] {
        b.value(
            name,
            Value::Type(TypeValue::primitive(PrimitiveType::Number)),
        );
    }
    b.consts(&[
        ("QuoteStyle.None", 0.0),
        ("QuoteStyle.Csv", 1.0),
        ("CsvStyle.QuoteAfterDelimiter", 0.0),
        ("CsvStyle.QuoteAlways", 1.0),
        ("MissingField.Error", 0.0),
        ("MissingField.Ignore", 1.0),
        ("MissingField.UseNull", 2.0),
        ("ExtraValues.List", 0.0),
        ("ExtraValues.Error", 1.0),
        ("ExtraValues.Ignore", 2.0),
        ("RelativePosition.FromStart", 0.0),
        ("RelativePosition.FromEnd", 1.0),
        ("Order.Ascending", 0.0),
        ("Order.Descending", 1.0),
        ("Occurrence.First", 0.0),
        ("Occurrence.Last", 1.0),
        ("Occurrence.All", 2.0),
        ("TextEncoding.Utf8", 65001.0),
        ("TextEncoding.Utf16", 1200.0),
        ("TextEncoding.Unicode", 1200.0),
        ("TextEncoding.BigEndianUnicode", 1201.0),
        ("TextEncoding.Ascii", 20127.0),
        ("TextEncoding.Windows", 1252.0),
        ("JoinKind.Inner", 0.0),
        ("JoinKind.LeftOuter", 1.0),
        ("JoinKind.RightOuter", 2.0),
        ("JoinKind.FullOuter", 3.0),
        ("JoinKind.LeftAnti", 4.0),
        ("JoinKind.RightAnti", 5.0),
        ("JoinKind.LeftSemi", 6.0),
        ("JoinKind.RightSemi", 7.0),
        ("JoinAlgorithm.Dynamic", 0.0),
        ("JoinAlgorithm.PairwiseHash", 1.0),
        ("JoinAlgorithm.SortMerge", 2.0),
        ("JoinAlgorithm.LeftHash", 3.0),
        ("JoinAlgorithm.RightHash", 4.0),
        ("JoinAlgorithm.LeftIndex", 5.0),
        ("JoinAlgorithm.RightIndex", 6.0),
        ("JoinSide.Left", 0.0),
        ("JoinSide.Right", 1.0),
        ("PercentileMode.ExcelInc", 1.0),
        ("PercentileMode.ExcelExc", 2.0),
        ("PercentileMode.SqlDisc", 3.0),
        ("PercentileMode.SqlCont", 4.0),
        ("Occurrence.Optional", 0.0),
        ("Occurrence.Required", 1.0),
        ("Occurrence.Repeating", 2.0),
        ("BinaryOccurrence.Optional", 0.0),
        ("BinaryOccurrence.Required", 1.0),
        ("BinaryOccurrence.Repeating", 2.0),
        ("GroupKind.Local", 0.0),
        ("GroupKind.Global", 1.0),
        ("TraceLevel.Critical", 1.0),
        ("TraceLevel.Error", 2.0),
        ("TraceLevel.Warning", 4.0),
        ("TraceLevel.Information", 8.0),
        ("TraceLevel.Verbose", 16.0),
    ]);
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace()) {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
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

fn hash_binary(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    match arg(args, 0) {
        Value::List(l) => {
            it.check_bytes(l.len())?;
            let bytes = items(it, l)?
                .iter()
                .map(|v| match v {
                    Value::Number(n) if n.fract() == 0.0 && (0.0..=255.0).contains(n) => {
                        Ok(*n as u8)
                    }
                    other => Err(convert_error(other, "Byte")),
                })
                .collect::<Result<Vec<u8>, _>>()?;
            Ok(Value::Binary(BinaryValue::from_bytes(bytes)))
        }
        Value::Text(t) => {
            it.check_bytes(t.len())?;
            base64_decode(t)
                .map(|b| Value::Binary(BinaryValue::from_bytes(b)))
                .ok_or_else(|| MError::data_format("The text is not valid base64."))
        }
        other => Err(convert_error(other, "List")),
    }
}

fn binary_length(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    match arg(args, 0) {
        Value::Null => Ok(Value::Null),
        Value::Binary(b) => Ok(Value::Number(
            b.bytes(it.opts.limits.max_binary_bytes)?.len() as f64,
        )),
        other => Err(convert_error(other, "Binary")),
    }
}

fn type_arg(args: &[Value], i: usize) -> Result<&TypeValue, MError> {
    match arg(args, i) {
        Value::Type(t) => Ok(t),
        other => Err(convert_error(other, "Type")),
    }
}

fn is(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::Logical(type_arg(args, 1)?.conforms(arg(args, 0))))
}

fn value_as(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = type_arg(args, 1)?;
    let v = arg(args, 0);
    if t.conforms(v) {
        Ok(v.clone())
    } else {
        Err(convert_error(v, &t.message_name()))
    }
}

fn value_type(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::Type(type_of(arg(args, 0))))
}

fn metadata(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::Record(arg(args, 0).metadata()))
}

fn remove_metadata(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let v = arg(args, 0);
    match arg(args, 1).plain() {
        Value::Text(field) => {
            let meta = v.metadata();
            let kept: Vec<(Arc<str>, crate::value::Lazy)> = meta
                .iter()
                .filter(|(n, _)| n.as_ref() != field.as_ref())
                .map(|(n, l)| (n.clone(), l.clone()))
                .collect();
            let rec = RecordValue::from_lazy_pairs(kept)?;
            Ok(if rec.is_empty() {
                v.plain().clone()
            } else {
                v.plain().clone().with_meta(rec)
            })
        }
        _ => Ok(v.plain().clone()),
    }
}

fn replace_metadata(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    match arg(args, 1).plain() {
        Value::Record(r) => Ok(arg(args, 0).plain().clone().with_meta(r.clone())),
        Value::Null => Ok(arg(args, 0).plain().clone()),
        other => Err(convert_error(other, "Record")),
    }
}

fn replace_type(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = type_arg(args, 1)?;
    let v = arg(args, 0).clone().into_plain();
    match (&v, t.table_columns()) {
        (Value::Table(table), Some(cols)) => {
            if cols.len() != table.column_count() {
                return Err(MError::expression(format!(
                    "The table type has {} columns but the table has {}.",
                    cols.len(),
                    table.column_count()
                )));
            }
            Ok(Value::Table(TableValue::new(
                cols.iter().map(|(n, _)| n.clone()).collect(),
                cols.iter().map(|(_, t)| t.clone()).collect(),
                table.columns().to_vec(),
            )?))
        }
        _ if t.conforms(&v) => Ok(v),
        _ => Err(convert_error(&v, &t.message_name())),
    }
}

fn value_equals(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::Logical(equals(it, arg(args, 0), arg(args, 1), 0)?))
}

fn nullable_equals(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    if arg(args, 0).is_null() || arg(args, 1).is_null() {
        return Ok(Value::Null);
    }
    value_equals(it, args)
}

fn compare(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::Number(
        match sort_compare(arg(args, 0), arg(args, 1)) {
            std::cmp::Ordering::Less => -1.0,
            std::cmp::Ordering::Equal => 0.0,
            std::cmp::Ordering::Greater => 1.0,
        },
    ))
}

fn type_is(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let (a, b) = (type_arg(args, 0)?, type_arg(args, 1)?);
    let compatible = match b.base() {
        PrimitiveType::Any => true,
        PrimitiveType::AnyNonNull => !a.is_nullable(),
        p => a.base() == p && (b.is_nullable() || !a.is_nullable()),
    };
    Ok(Value::Logical(compatible))
}

fn type_is_nullable(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::Logical(type_arg(args, 0)?.is_nullable()))
}

fn type_non_nullable(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::Type(type_arg(args, 0)?.non_nullable()))
}

fn type_table_column(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = type_arg(args, 0)?;
    let name = req_text(args, 1)?;
    match t.kind() {
        TypeKind::Table(tt) => tt
            .columns
            .iter()
            .find(|(n, _)| &**n == name)
            .map(|(_, ty)| Value::Type(ty.clone()))
            .ok_or_else(|| super::missing_column(name)),
        _ => Err(convert_error(arg(args, 0), "table type")),
    }
}

/// `Error.Record`: without parameters `Message` is the message (default empty) and
/// `Message.Format` is `null`; with parameters `Message.Format` is the message and `Message` the
/// format with its `#{n}` placeholders filled.
fn error_record(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let reason = req_text(args, 0)?;
    let message = text_arg(args, 1, "Error.Record")?.unwrap_or_default();
    let parameters = list_arg(args, 3)?;
    let error_code = text_arg(args, 4, "Error.Record")?.map_or(Value::Null, Value::text);
    let (message, format) = match parameters {
        None => (Value::text(message), Value::Null),
        Some(p) => (
            Value::text(format_error_message(it, message, Some(p))?),
            Value::text(message),
        ),
    };
    Ok(Value::Record(RecordValue::from_pairs([
        ("Reason", Value::text(reason)),
        ("Message", message),
        ("Detail", arg(args, 2).clone()),
        ("Message.Format", format),
        ("Message.Parameters", arg(args, 3).clone()),
        ("ErrorCode", error_code),
    ])?))
}

const INVALID_PLACEHOLDER: &str =
    "Invalid placeholder or value in format string. Underlying error message: ";

fn invalid_placeholder(underlying: &str) -> MError {
    MError::Evaluation {
        reason: "Expression.Error".into(),
        message: format!("{INVALID_PLACEHOLDER}{underlying}"),
        detail: None,
        location: None,
        fields: Some(Box::new(ErrorFields {
            null_message: false,
            message_format: Value::text(format!("{INVALID_PLACEHOLDER}#{{0}}")),
            message_parameters: Value::list([Value::text(underlying)]),
            error_code: Value::text("10582"),
        })),
    }
}

/// The index of a `#{n}` placeholder: an `Int32` with optional sign and surrounding spaces.
fn placeholder_index(inner: &str) -> Result<usize, MError> {
    match inner.trim().parse::<i32>() {
        Ok(n) => {
            usize::try_from(n).map_err(|_| invalid_placeholder("The index cannot be negative."))
        }
        Err(_) => Err(invalid_placeholder("We expected an integer value.")),
    }
}

/// Fills the `#{n}` placeholders of an error message format with `parameters` (`None`: no
/// parameters), as Desktop does for `Error.Record` and `error [Message.Format = ...]`. Only the
/// parameters a placeholder refers to are evaluated.
pub(crate) fn format_error_message(
    it: &Interp,
    format: &str,
    parameters: Option<&ListValue>,
) -> Result<String, MError> {
    let mut out = String::new();
    let mut rest = format;
    while let Some(start) = rest.find("#{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            return Err(MError::data_format(
                "Text format placeholders must be closed.",
            ));
        };
        let index = placeholder_index(&after[..end])?;
        let slot = parameters
            .and_then(|p| p.items().get(index))
            .ok_or_else(|| {
                invalid_placeholder(
                    "There weren't enough elements in the enumeration to complete the operation.",
                )
            })?;
        let value = match slot.force(it) {
            Ok(v) => v,
            Err(MError::Evaluation { message, .. }) => return Err(invalid_placeholder(&message)),
            Err(e) => return Err(e),
        };
        out.push_str(&placeholder_text(it, &value)?);
        it.check_bytes(out.len())?;
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// A value as an error message parameter (en-US): structured values give their kind.
fn placeholder_text(it: &Interp, v: &Value) -> Result<String, MError> {
    Ok(match v.plain() {
        Value::Null => "null".into(),
        Value::Logical(b) => b.to_string(),
        Value::Number(n) => number_to_text(*n),
        Value::Text(t) => t.to_string(),
        Value::Date(d) => d.format("%m/%d/%Y").to_string(),
        Value::DateTime(dt) => dt.format("%m/%d/%Y %H:%M:%S").to_string(),
        Value::DateTimeZone(dt) => dt.format("%m/%d/%Y %H:%M:%S %:z").to_string(),
        Value::Time(t) => t.format("%H:%M").to_string(),
        Value::Duration(d) => duration_to_text(*d),
        Value::Binary(b) => base64_encode(&b.bytes(it.opts.limits.max_binary_bytes)?),
        Value::List(_) => "list".into(),
        Value::Record(_) => "record".into(),
        Value::Table(_) => "table".into(),
        Value::Function(_) => "function".into(),
        Value::Type(_) => "type".into(),
        Value::Meta(m) => return placeholder_text(it, &m.value),
    })
}

pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[((n >> shift) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn trace(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let delayed = arg(args, 3).as_logical() == Some(true);
    match (arg(args, 2), delayed) {
        (Value::Function(f), true) => it.call(f, &[]),
        (v, _) => Ok(v.clone()),
    }
}

fn function_invoke(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let f = req_func(args, 0)?;
    let a = items(it, req_list(args, 1)?)?;
    it.call(f, &a)
}

fn json_to_value(it: &Interp, j: &serde_json::Value) -> Result<Value, MError> {
    it.tick()?;
    Ok(match j {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Logical(*b),
        serde_json::Value::Number(n) => Value::Number(
            n.as_f64()
                .or_else(|| n.to_string().parse().ok())
                .unwrap_or(f64::NAN),
        ),
        serde_json::Value::String(s) => Value::text(s.as_str()),
        serde_json::Value::Array(a) => {
            it.check_rows(a.len())?;
            Value::List(ListValue::from_values(
                a.iter()
                    .map(|x| json_to_value(it, x))
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        serde_json::Value::Object(o) => Value::Record(RecordValue::from_pairs(
            o.iter()
                .map(|(k, v)| Ok((Arc::<str>::from(k.as_str()), json_to_value(it, v)?)))
                .collect::<Result<Vec<_>, MError>>()?,
        )?),
    })
}

fn json_document(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let codepage = int_arg(args, 1)?.and_then(|n| u32::try_from(n).ok());
    let text = match arg(args, 0) {
        Value::Text(t) => t.to_string(),
        Value::Binary(b) => decode_bytes(&b.bytes(it.opts.limits.max_binary_bytes)?, codepage)?,
        other => return Err(convert_error(other, "Text")),
    };
    let j: serde_json::Value = serde_json::from_str(&text).map_err(|e| MError::Evaluation {
        reason: "DataFormat.Error".into(),
        message: format!("We found an invalid JSON value: {e}"),
        detail: None,
        location: None,
        fields: None,
    })?;
    json_to_value(it, &j)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_base64() {
        assert_eq!(base64_decode("AQID"), Some(vec![1, 2, 3]));
        assert_eq!(base64_decode("aGk="), Some(b"hi".to_vec()));
        assert_eq!(base64_decode("a$"), None);
    }
}
