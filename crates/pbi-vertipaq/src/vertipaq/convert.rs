//! Data ids → Arrow arrays: dictionary lookup, value encoding, BLANK handling and the
//! logical type conversions.
//!
//! | Logical type | Arrow | Stored as |
//! |---|---|---|
//! | String | `Utf8` | string dictionary |
//! | Int64 | `Int64` | long dictionary or value encoding |
//! | Double | `Float64` | real dictionary or value encoding (`/ magnitude`) |
//! | Decimal | `Decimal128(19, 4)` | long dictionary / value encoding of value × 10⁴ |
//! | DateTime | `Timestamp(Microsecond, None)` | real (or integer) days since 1899-12-30 |
//! | Boolean | `Boolean` | integer, non-zero = true |
//! | Binary | `Binary` | base64 text in a string dictionary |
//! | Variant / Automatic / unknown | by storage: `Int64`, `Float64` or `Utf8` | |
//!
//! Data id [`NULL_DATA_ID`] is BLANK (Arrow null); dictionary entry `i` has data id
//! [`FIRST_DATA_ID`]` + i`. A string dictionary behind a non-string, non-binary logical
//! type is returned as `Utf8` (no parsing).

use std::sync::Arc;

use arrow::array::{
    ArrayRef, BinaryArray, BooleanArray, Decimal128Array, Float64Array, Int64Array, StringArray,
    StringBuilder, TimestampMicrosecondArray, new_null_array,
};
use arrow::datatypes::{DataType as ArrowType, TimeUnit};
use pbi_model::DataType;

use super::dictionary::Dictionary;
use super::error::VertipaqError;
use super::idfmeta::{FIRST_DATA_ID, NULL_DATA_ID};

/// Precision of the Arrow `Decimal128` used for fixed decimals (as `pbi-engine`).
pub const DECIMAL_PRECISION: u8 = 19;
/// Scale of the Arrow `Decimal128` used for fixed decimals (as `pbi-engine`).
pub const DECIMAL_SCALE: i8 = 4;

/// Microseconds from 1899-12-30 (OLE automation date 0) to 1970-01-01.
const OLE_EPOCH_OFFSET_MICROS: i64 = 25_569 * MICROS_PER_DAY;
const MICROS_PER_DAY: i64 = 86_400_000_000;

/// Where a column's values come from.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Source<'a> {
    /// Hash encoding without a dictionary file: every row is BLANK.
    Blank,
    /// Value encoding: `(data_id + base_id) / magnitude`.
    Value { base_id: i64, magnitude: f64 },
    /// Hash encoding with a dictionary.
    Dictionary(&'a Dictionary),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Physical {
    Blank,
    Integer,
    Real,
    Text,
}

impl Source<'_> {
    fn physical(&self) -> Physical {
        match self {
            Source::Blank => Physical::Blank,
            Source::Value { magnitude, .. } if *magnitude == 1.0 => Physical::Integer,
            Source::Value { .. } => Physical::Real,
            Source::Dictionary(Dictionary::Long(_)) => Physical::Integer,
            Source::Dictionary(Dictionary::Real(_)) => Physical::Real,
            Source::Dictionary(Dictionary::String(_)) => Physical::Text,
        }
    }
}

/// The Arrow type of a column with this logical type and storage.
pub(crate) fn arrow_type(logical: &DataType, source: &Source<'_>) -> ArrowType {
    let physical = source.physical();
    let by_storage = || match physical {
        Physical::Integer => ArrowType::Int64,
        Physical::Real => ArrowType::Float64,
        Physical::Text | Physical::Blank => ArrowType::Utf8,
    };
    match logical {
        DataType::String => ArrowType::Utf8,
        DataType::Binary if matches!(physical, Physical::Text | Physical::Blank) => {
            ArrowType::Binary
        }
        _ if physical == Physical::Text => ArrowType::Utf8,
        DataType::Int64 => ArrowType::Int64,
        DataType::Double => ArrowType::Float64,
        DataType::Decimal => ArrowType::Decimal128(DECIMAL_PRECISION, DECIMAL_SCALE),
        DataType::DateTime => ArrowType::Timestamp(TimeUnit::Microsecond, None),
        DataType::Boolean => ArrowType::Boolean,
        _ => by_storage(),
    }
}

/// The value of one row before conversion.
#[derive(Debug, Clone, Copy)]
enum Cell<'a> {
    Null,
    Invalid(u32),
    Int(i64),
    Real(f64),
    Text(&'a str),
}

/// Rows that could not be mapped to a value (invalid ids or failed conversions).
#[derive(Debug, Default)]
pub(crate) struct Invalid {
    pub count: u64,
    pub first: Option<String>,
}

impl Invalid {
    fn note(&mut self, describe: impl FnOnce() -> String) {
        if self.first.is_none() {
            self.first = Some(describe());
        }
        self.count += 1;
    }
}

/// A converted column.
#[derive(Debug)]
pub(crate) struct Converted {
    pub array: ArrayRef,
    pub invalid: Invalid,
}

fn validate_source(source: &Source<'_>) -> Result<(), VertipaqError> {
    if let Source::Value { magnitude, .. } = source
        && (!magnitude.is_finite() || *magnitude == 0.0)
    {
        return Err(VertipaqError::corrupt(
            "value encoding",
            format!("magnitude {magnitude} is not a finite non-zero number"),
        ));
    }
    Ok(())
}

fn cell<'a>(source: &Source<'a>, id: u32) -> Cell<'a> {
    if id == NULL_DATA_ID {
        return Cell::Null;
    }
    match source {
        Source::Blank => Cell::Null,
        Source::Value { base_id, magnitude } => {
            if id < NULL_DATA_ID {
                return Cell::Invalid(id);
            }
            match base_id.checked_add(i64::from(id)) {
                Some(v) if *magnitude == 1.0 => Cell::Int(v),
                Some(v) => Cell::Real(v as f64 / magnitude),
                None => Cell::Invalid(id),
            }
        }
        Source::Dictionary(dictionary) => {
            let Some(index) = id.checked_sub(FIRST_DATA_ID).map(|i| i as usize) else {
                return Cell::Invalid(id);
            };
            match dictionary {
                Dictionary::Long(values) => values
                    .get(index)
                    .map_or(Cell::Invalid(id), |&v| Cell::Int(v)),
                Dictionary::Real(values) => values
                    .get(index)
                    .map_or(Cell::Invalid(id), |&v| Cell::Real(v)),
                Dictionary::String(values) => values
                    .get(index)
                    .map_or(Cell::Invalid(id), |v| Cell::Text(v.as_str())),
            }
        }
    }
}

fn describe_id(id: u32) -> String {
    format!("data id {id}")
}

/// Converts data ids to an Arrow array of [`arrow_type`].
pub(crate) fn convert(
    ids: &[u32],
    source: &Source<'_>,
    logical: &DataType,
    max_string_bytes: u64,
) -> Result<Converted, VertipaqError> {
    validate_source(source)?;
    let target = arrow_type(logical, source);
    let mut invalid = Invalid::default();
    if matches!(source, Source::Blank) {
        for &id in ids {
            if id != NULL_DATA_ID {
                invalid.note(|| format!("data id {id} in a column without dictionary"));
            }
        }
        return Ok(Converted {
            array: new_null_array(&target, ids.len()),
            invalid,
        });
    }
    let cells = ids.iter().map(|&id| cell(source, id));

    let array: ArrayRef = match target {
        ArrowType::Utf8 => {
            let limit = max_string_bytes.min(i32::MAX as u64);
            let too_large = |value: u64| VertipaqError::LimitExceeded {
                what: "string bytes in a column",
                value,
                limit,
            };
            if let Source::Dictionary(Dictionary::String(values)) = source {
                let total = dictionary_text_bytes(ids, values);
                if total > limit {
                    return Err(too_large(total));
                }
                let mut out = StringBuilder::with_capacity(ids.len(), total as usize);
                for c in cells {
                    match c {
                        Cell::Text(s) => out.append_value(s),
                        Cell::Null => out.append_null(),
                        other => {
                            invalid.note(|| invalid_cell(other, "String"));
                            out.append_null();
                        }
                    }
                }
                Arc::new(out.finish()) as ArrayRef
            } else {
                let mut total = 0u64;
                let mut rows: Vec<Option<String>> = Vec::with_capacity(ids.len());
                for c in cells {
                    let text = match c {
                        Cell::Int(v) => Some(v.to_string()),
                        Cell::Real(v) => Some(v.to_string()),
                        Cell::Text(s) => Some(s.to_owned()),
                        Cell::Null => None,
                        Cell::Invalid(id) => {
                            invalid.note(|| describe_id(id));
                            None
                        }
                    };
                    total = total.saturating_add(text.as_ref().map_or(0, |t| t.len() as u64));
                    if total > limit {
                        return Err(too_large(total));
                    }
                    rows.push(text);
                }
                Arc::new(StringArray::from_iter(rows)) as ArrayRef
            }
        }
        ArrowType::Binary => {
            let mut total = 0u64;
            let mut rows: Vec<Option<Vec<u8>>> = Vec::with_capacity(ids.len());
            for c in cells {
                rows.push(match c {
                    Cell::Text(s) => match base64_decode(s) {
                        Some(bytes) => {
                            total = total.saturating_add(bytes.len() as u64);
                            Some(bytes)
                        }
                        None => {
                            invalid.note(|| "invalid base64 binary value".to_owned());
                            None
                        }
                    },
                    Cell::Null => None,
                    other => {
                        invalid.note(|| invalid_cell(other, "binary"));
                        None
                    }
                });
                if total > max_string_bytes || total > i32::MAX as u64 {
                    return Err(VertipaqError::LimitExceeded {
                        what: "binary bytes in a column",
                        value: total,
                        limit: max_string_bytes.min(i32::MAX as u64),
                    });
                }
            }
            Arc::new(BinaryArray::from_iter(rows))
        }
        ArrowType::Int64 => Arc::new(Int64Array::from_iter(
            cells.map(|c| numeric(c, &mut invalid, "Int64", to_i64)),
        )),
        ArrowType::Float64 => Arc::new(Float64Array::from_iter(
            cells.map(|c| numeric(c, &mut invalid, "Double", to_f64)),
        )),
        ArrowType::Decimal128(precision, scale) => Arc::new(
            Decimal128Array::from_iter(
                cells.map(|c| numeric(c, &mut invalid, "Decimal", to_decimal_raw)),
            )
            .with_precision_and_scale(precision, scale)?,
        ),
        ArrowType::Timestamp(TimeUnit::Microsecond, None) => {
            Arc::new(TimestampMicrosecondArray::from_iter(
                cells.map(|c| numeric(c, &mut invalid, "DateTime", to_micros)),
            ))
        }
        ArrowType::Boolean => Arc::new(BooleanArray::from_iter(
            cells.map(|c| numeric(c, &mut invalid, "Boolean", to_bool)),
        )),
        other => {
            return Err(VertipaqError::unsupported(
                "Arrow target type",
                other.to_string(),
            ));
        }
    };
    Ok(Converted { array, invalid })
}

fn invalid_cell(c: Cell<'_>, target: &str) -> String {
    match c {
        Cell::Invalid(id) => describe_id(id),
        Cell::Int(_) => format!("integer value not convertible to {target}"),
        Cell::Real(_) => format!("real value not convertible to {target}"),
        Cell::Text(_) => format!("text value not convertible to {target}"),
        Cell::Null => "null".to_owned(),
    }
}

/// Numeric view of a cell for the non-text targets.
#[derive(Debug, Clone, Copy)]
enum Num {
    Int(i64),
    Real(f64),
}

fn numeric<T>(
    c: Cell<'_>,
    invalid: &mut Invalid,
    target: &str,
    convert: impl Fn(Num) -> Option<T>,
) -> Option<T> {
    let converted = match c {
        Cell::Null => return None,
        Cell::Int(v) => convert(Num::Int(v)),
        Cell::Real(v) => convert(Num::Real(v)),
        Cell::Invalid(_) | Cell::Text(_) => None,
    };
    if converted.is_none() {
        invalid.note(|| invalid_cell(c, target));
    }
    converted
}

fn real_to_i64(v: f64) -> Option<i64> {
    let r = v.round();
    // `i64::MAX as f64` rounds up to 2^63, which is out of range.
    (r.is_finite() && r >= i64::MIN as f64 && r < i64::MAX as f64).then_some(r as i64)
}

fn to_i64(n: Num) -> Option<i64> {
    match n {
        Num::Int(v) => Some(v),
        Num::Real(v) => real_to_i64(v),
    }
}

fn to_f64(n: Num) -> Option<f64> {
    match n {
        Num::Int(v) => Some(v as f64),
        Num::Real(v) => Some(v),
    }
}

/// Fixed decimals are stored as value × 10⁴, which is the `Decimal128(19, 4)` raw value.
fn to_decimal_raw(n: Num) -> Option<i128> {
    to_i64(n).map(i128::from)
}

fn to_bool(n: Num) -> Option<bool> {
    match n {
        Num::Int(v) => Some(v != 0),
        Num::Real(v) => (!v.is_nan()).then_some(v != 0.0),
    }
}

/// OLE automation date (days since 1899-12-30) → microseconds since 1970-01-01.
fn to_micros(n: Num) -> Option<i64> {
    match n {
        Num::Int(days) => days
            .checked_mul(MICROS_PER_DAY)?
            .checked_sub(OLE_EPOCH_OFFSET_MICROS),
        Num::Real(days) => days_to_micros(days),
    }
}

/// Real days → microseconds, rounding the fractional day to the nearest microsecond.
pub(crate) fn days_to_micros(days: f64) -> Option<i64> {
    if !days.is_finite() || days.abs() > 1e8 {
        return None;
    }
    let whole = days.trunc();
    let frac = days - whole;
    let whole_micros = (whole as i64).checked_mul(MICROS_PER_DAY)?;
    let frac_micros = (frac * MICROS_PER_DAY as f64).round() as i64;
    whole_micros
        .checked_add(frac_micros)?
        .checked_sub(OLE_EPOCH_OFFSET_MICROS)
}

/// Total UTF-8 bytes of the rows of a string-dictionary column.
fn dictionary_text_bytes(ids: &[u32], values: &[String]) -> u64 {
    ids.iter()
        .filter_map(|&id| {
            id.checked_sub(FIRST_DATA_ID)
                .and_then(|i| values.get(i as usize))
        })
        .fold(0u64, |acc, s| acc.saturating_add(s.len() as u64))
}

/// Standard base64 (RFC 4648, `+/`, optional `=` padding, whitespace ignored).
pub(crate) fn base64_decode(text: &str) -> Option<Vec<u8>> {
    fn sextet(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    }
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    let mut padding = 0usize;
    for &c in text.as_bytes() {
        if c.is_ascii_whitespace() {
            continue;
        }
        if c == b'=' {
            padding += 1;
            continue;
        }
        if padding > 0 {
            return None;
        }
        acc = (acc << 6) | sextet(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    (padding <= 2 && bits < 6).then_some(out)
}
