//! Arrow/engine helpers for evaluator tests.

use std::path::PathBuf;
use std::sync::Arc;

use arrow::array::{
    ArrayRef, BooleanArray, Decimal128Array, Float64Array, Int64Array, StringArray,
    TimestampMicrosecondArray,
};
use arrow::datatypes::{Field, Schema};
use arrow::record_batch::RecordBatch;
use pbi_engine::Value;

pub fn strings(values: &[Option<&str>]) -> ArrayRef {
    Arc::new(StringArray::from(values.to_vec()))
}

pub fn texts(values: &[&str]) -> ArrayRef {
    Arc::new(StringArray::from(values.to_vec()))
}

pub fn ints(values: &[Option<i64>]) -> ArrayRef {
    Arc::new(Int64Array::from(values.to_vec()))
}

pub fn floats(values: &[Option<f64>]) -> ArrayRef {
    Arc::new(Float64Array::from(values.to_vec()))
}

pub fn bools(values: &[Option<bool>]) -> ArrayRef {
    Arc::new(BooleanArray::from(values.to_vec()))
}

/// Fixed decimals (`Decimal128(19, 4)`, the engine's `decimal` type).
pub fn decimals(values: &[Option<f64>]) -> ArrayRef {
    let raw: Vec<Option<i128>> = values
        .iter()
        .map(|v| v.map(|f| (f * 10_000.0).round() as i128))
        .collect();
    Arc::new(
        Decimal128Array::from(raw)
            .with_precision_and_scale(19, 4)
            .unwrap(),
    )
}

/// Midnight timestamps of `(year, month, day)` dates.
pub fn dates(values: &[Option<(i64, u32, u32)>]) -> ArrayRef {
    let parts: Vec<Option<DateParts>> = values
        .iter()
        .map(|v| v.map(|(y, m, d)| (y, m, d, 0, 0, 0)))
        .collect();
    datetimes(&parts)
}

/// `(year, month, day, hour, minute, second)`.
pub type DateParts = (i64, u32, u32, u32, u32, u32);

/// Timestamps (µs) from `Value::datetime` components.
pub fn datetimes(values: &[Option<DateParts>]) -> ArrayRef {
    let micros: Vec<Option<i64>> = values
        .iter()
        .map(|v| {
            v.and_then(
                |(y, mo, d, h, mi, s)| match Value::datetime(y, mo, d, h, mi, s) {
                    Some(Value::DateTime(us)) => Some(us),
                    _ => None,
                },
            )
        })
        .collect();
    Arc::new(TimestampMicrosecondArray::from(micros))
}

pub fn batch(columns: Vec<(&str, ArrayRef)>) -> RecordBatch {
    let fields: Vec<Field> = columns
        .iter()
        .map(|(n, a)| Field::new(*n, a.data_type().clone(), true))
        .collect();
    let arrays = columns.into_iter().map(|(_, a)| a).collect();
    RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays).unwrap()
}

/// Fixture root (`$DASHCRAFT_REFERENCE` or `<workspace>/reference`).
pub fn reference_root() -> PathBuf {
    std::env::var_os("DASHCRAFT_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
}
