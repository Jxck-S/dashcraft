//! Conversion of M tables to Arrow record batches.
//!
//! | Arrow type | Accepted cell values |
//! |---|---|
//! | `Utf8` / `LargeUtf8` | any scalar (`Text.From` formatting) |
//! | `Int64` | numbers (rounded half to even), numeric text, logicals |
//! | `Float64` | numbers, numeric text, logicals |
//! | `Boolean` | logicals, `"true"`/`"false"`, numbers |
//! | `Timestamp(Microsecond, tz)` | datetime (as UTC when `tz` is set), date, datetimezone (UTC instant), text |
//! | `Date32` | date, datetime, datetimezone (local date), text |
//! | `Time64(Microsecond)` | time, datetime, text |
//! | `Duration(Microsecond)` | duration, text |
//!
//! `null` cells become Arrow nulls. A cell holding an M error, or a value that cannot be
//! coerced, fails the conversion with an error naming the row and column (Power BI Desktop
//! instead loads such cells as blanks and reports an error count; pbiview surfaces the error).

use std::sync::Arc;

use arrow::array::{
    ArrayRef, BooleanBuilder, Date32Builder, DurationMicrosecondBuilder, Float64Builder,
    Int64Builder, LargeStringBuilder, StringBuilder, Time64MicrosecondBuilder,
    TimestampMicrosecondBuilder,
};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use arrow::record_batch::{RecordBatch, RecordBatchOptions};
use chrono::{Offset, Timelike, Utc};

use crate::ast::PrimitiveType;
use crate::merror::MError;
use crate::stdlib::convert::{
    self, date_to_epoch_days, datetime_to_epoch_micros, parse_number, round_half_even,
};
use crate::value::{Lazy, TableValue, TypeValue, Value};

/// The Arrow type pbiview uses for an M column type.
pub fn arrow_type_for(ty: &TypeValue) -> DataType {
    match ty.base() {
        PrimitiveType::Text => DataType::Utf8,
        PrimitiveType::Number => match ty.facet_kind() {
            Some(f) if f.is_integer() => DataType::Int64,
            _ => DataType::Float64,
        },
        PrimitiveType::Logical => DataType::Boolean,
        PrimitiveType::Date => DataType::Date32,
        PrimitiveType::DateTime => DataType::Timestamp(TimeUnit::Microsecond, None),
        PrimitiveType::DateTimeZone => {
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
        }
        PrimitiveType::Time => DataType::Time64(TimeUnit::Microsecond),
        PrimitiveType::Duration => DataType::Duration(TimeUnit::Microsecond),
        _ => DataType::Utf8,
    }
}

fn infer_column_type(ty: &TypeValue, cells: &[Lazy]) -> DataType {
    if !matches!(ty.base(), PrimitiveType::Any | PrimitiveType::AnyNonNull) {
        return arrow_type_for(ty);
    }
    let mut kind = None;
    for c in cells {
        if let Lazy::Value(v) = c {
            if v.is_null() {
                continue;
            }
            let k = v.kind();
            match kind {
                None => kind = Some(k),
                Some(prev) if prev == k => {}
                Some(_) => return DataType::Utf8,
            }
        }
    }
    match kind {
        Some(k) => arrow_type_for(&TypeValue::primitive(k.primitive())),
        None => DataType::Utf8,
    }
}

/// Converts a resolved table to a record batch. With `expected`, the batch has exactly those
/// columns (by name, in that order) coerced to those types; missing and extra columns are
/// reported together in one error. Without it, types are derived from the column types (or
/// from the values for `any` columns).
pub fn table_to_record_batch(
    table: &TableValue,
    expected: Option<&[(String, DataType)]>,
) -> Result<RecordBatch, MError> {
    let columns: Vec<(String, usize, DataType)> = match expected {
        Some(expected) => {
            let missing: Vec<&str> = expected
                .iter()
                .filter(|(n, _)| table.column_index(n).is_none())
                .map(|(n, _)| n.as_str())
                .collect();
            let extra: Vec<&str> = table
                .column_names()
                .iter()
                .filter(|n| !expected.iter().any(|(e, _)| e.as_str() == &***n))
                .map(|n| &**n)
                .collect();
            if !missing.is_empty() || !extra.is_empty() {
                let mut parts = Vec::new();
                if !missing.is_empty() {
                    parts.push(format!("missing column(s): {}", quote_list(&missing)));
                }
                if !extra.is_empty() {
                    parts.push(format!("unexpected column(s): {}", quote_list(&extra)));
                }
                return Err(MError::expression(format!(
                    "The query result does not match the table's columns: {}.",
                    parts.join("; ")
                )));
            }
            expected
                .iter()
                .filter_map(|(n, t)| table.column_index(n).map(|i| (n.clone(), i, t.clone())))
                .collect()
        }
        None => (0..table.column_count())
            .map(|i| {
                let ty = table.column_type(i).cloned().unwrap_or_else(TypeValue::any);
                let cells = table.column(i).unwrap_or(&[]);
                (
                    table.column_names()[i].to_string(),
                    i,
                    infer_column_type(&ty, cells),
                )
            })
            .collect(),
    };
    let mut fields = Vec::with_capacity(columns.len());
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(columns.len());
    for (name, index, dt) in columns {
        let cells = table.column(index).unwrap_or(&[]);
        arrays.push(build_array(&name, cells, &dt)?);
        fields.push(Field::new(name, dt, true));
    }
    let options = RecordBatchOptions::new().with_row_count(Some(table.row_count()));
    RecordBatch::try_new_with_options(Arc::new(Schema::new(fields)), arrays, &options)
        .map_err(|e| MError::Internal(format!("building the Arrow batch failed: {e}")))
}

fn quote_list(names: &[&str]) -> String {
    names
        .iter()
        .map(|n| format!("'{n}'"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn cell_value(column: &str, row: usize, cell: &Lazy) -> Result<Value, MError> {
    cell.resolved()
        .map(Value::into_plain)
        .map_err(|e| e.with_context(&format!("Row {}, column '{column}'", row + 1)))
}

fn coerce_error(column: &str, row: usize, v: &Value, target: &str) -> MError {
    MError::data_format(format!(
        "Row {}, column '{column}': cannot convert {} to {target}.",
        row + 1,
        crate::value::format::describe(v)
    ))
}

fn build_array(column: &str, cells: &[Lazy], dt: &DataType) -> Result<ArrayRef, MError> {
    let utc = Utc.fix();
    macro_rules! fill {
        ($builder:expr, $target:expr, |$v:ident| $conv:expr) => {{
            let mut b = $builder;
            for (row, cell) in cells.iter().enumerate() {
                let $v = cell_value(column, row, cell)?;
                if $v.is_null() {
                    b.append_null();
                    continue;
                }
                let converted: Option<_> = $conv;
                match converted {
                    Some(x) => b.append_value(x),
                    None => return Err(coerce_error(column, row, &$v, $target)),
                }
            }
            Arc::new(b.finish()) as ArrayRef
        }};
    }
    Ok(match dt {
        DataType::Utf8 => fill!(StringBuilder::new(), "text", |v| text_of(&v)),
        DataType::LargeUtf8 => fill!(LargeStringBuilder::new(), "text", |v| text_of(&v)),
        DataType::Int64 => fill!(Int64Builder::new(), "Int64", |v| number_of(&v)
            .map(|n| round_half_even(n, 0))
            .filter(|n| n.is_finite() && n.abs() < 9.2e18)
            .map(|n| n as i64)),
        DataType::Float64 => fill!(Float64Builder::new(), "Float64", |v| number_of(&v)),
        DataType::Boolean => fill!(BooleanBuilder::new(), "Boolean", |v| match &v {
            Value::Logical(b) => Some(*b),
            Value::Number(n) => Some(*n != 0.0),
            Value::Text(_) => convert::to_logical(&v).ok().and_then(|x| x.as_logical()),
            _ => None,
        }),
        DataType::Timestamp(TimeUnit::Microsecond, tz) => {
            let builder = match tz {
                Some(tz) => TimestampMicrosecondBuilder::new().with_timezone(tz.clone()),
                None => TimestampMicrosecondBuilder::new(),
            };
            fill!(builder, "datetime", |v| match &v {
                Value::DateTimeZone(d) => Some(d.timestamp_micros()),
                _ => match convert::to_datetime(&v, utc) {
                    Ok(Value::DateTime(d)) => Some(datetime_to_epoch_micros(d)),
                    _ => None,
                },
            })
        }
        DataType::Date32 => fill!(
            Date32Builder::new(),
            "date",
            |v| match convert::to_date(&v) {
                Ok(Value::Date(d)) => Some(date_to_epoch_days(d)),
                _ => None,
            }
        ),
        DataType::Time64(TimeUnit::Microsecond) => {
            fill!(
                Time64MicrosecondBuilder::new(),
                "time",
                |v| match convert::to_time(&v) {
                    Ok(Value::Time(t)) => Some(
                        i64::from(t.num_seconds_from_midnight()) * 1_000_000
                            + i64::from(t.nanosecond() % 1_000_000_000) / 1_000,
                    ),
                    _ => None,
                }
            )
        }
        DataType::Duration(TimeUnit::Microsecond) => {
            fill!(
                DurationMicrosecondBuilder::new(),
                "duration",
                |v| match convert::to_duration(&v) {
                    Ok(Value::Duration(d)) => d.num_microseconds(),
                    _ => None,
                }
            )
        }
        other => {
            return Err(MError::expression(format!(
                "Column '{column}': Arrow type {other} is not supported (use Utf8, Int64, Float64, Boolean, Timestamp(µs), Date32, Time64(µs) or Duration(µs))."
            )));
        }
    })
}

fn text_of(v: &Value) -> Option<String> {
    crate::value::format::value_to_text(v)
}

fn number_of(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(*n),
        Value::Logical(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::Text(t) => parse_number(t),
        _ => None,
    }
}
