//! Scalar values shared by the storage engine and the DAX formula engine
//! (`pbi-dax`), their DAX comparison semantics, and Arrow conversions.
//!
//! # Types
//!
//! | DAX type | [`Value`] | Arrow storage type ([`arrow_type`]) |
//! |---|---|---|
//! | Text | [`Value::String`] | `Utf8` |
//! | Whole number | [`Value::Integer`] | `Int64` |
//! | Decimal number | [`Value::Double`] | `Float64` |
//! | Fixed decimal (currency) | [`Value::Decimal`] (`i128` scaled by 10⁴) | `Decimal128(19, 4)` |
//! | Date/time | [`Value::DateTime`] (naive µs since 1970-01-01) | `Timestamp(Microsecond, None)` |
//! | True/false | [`Value::Boolean`] | `Boolean` |
//! | BLANK | [`Value::Blank`] | null |
//!
//! Fixed decimals use `Decimal128(19, 4)` rather than `Float64` so currency
//! arithmetic stays exact, like VertiPaq's 64-bit scaled integers (19 digits
//! cover the full `i64` range).
//!
//! # Comparison semantics
//!
//! - [`Value::dax_eq`] / [`Value::dax_cmp`] implement DAX `=`, `<`, `>`…:
//!   BLANK is coerced to the other operand's "zero" (`0`, `""`, `FALSE`), so
//!   `BLANK = 0`, `BLANK = ""` and `BLANK = FALSE` are all true. Numbers of
//!   different types compare by value; dates compare with numbers as DAX
//!   serial dates. Text comparison is **case-insensitive**. Comparing
//!   incompatible types (text with a number, boolean with a number) is an
//!   error, as in DAX.
//! - [`Value::dax_strict_eq`] implements DAX `==`: like `=`, except BLANK only
//!   equals BLANK.
//! - [`ValueKey`] is the hashing key used for grouping, `IN` filters and
//!   relationship joins. It follows `==` (BLANK is distinct from `0`/`""`) and
//!   folds text case, so `"abc"` and `"ABC"` are the same key — matching
//!   VertiPaq, whose dictionaries are case-insensitive.
//! - The derived `PartialEq` on [`Value`] is *structural* (`Integer(1) !=
//!   Double(1.0)`, `"a" != "A"`); use it for tests, not for DAX semantics.

use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use arrow::array::{
    Array, ArrayRef, AsArray, BooleanBuilder, Decimal128Builder, Float64Builder, Int64Builder,
    StringBuilder, TimestampMicrosecondBuilder, new_null_array,
};
use arrow::compute::{CastOptions, cast_with_options};
use arrow::datatypes::{
    DataType as ArrowType, Decimal128Type, Float64Type, Int64Type, TimeUnit,
    TimestampMicrosecondType,
};
use pbi_model::DataType;

use crate::error::{EngineError, Result};

/// Scale of [`Value::Decimal`] (number of fractional digits).
pub const DECIMAL_SCALE: i8 = 4;
/// Precision of the Arrow `Decimal128` type used for fixed decimals.
pub const DECIMAL_PRECISION: u8 = 19;
/// `10^DECIMAL_SCALE`.
pub const DECIMAL_FACTOR: i128 = 10_000;
/// Microseconds per day.
pub const MICROS_PER_DAY: i64 = 86_400_000_000;
/// Days between the DAX (OLE Automation) date epoch 1899-12-30 and 1970-01-01.
pub const OADATE_UNIX_EPOCH_DAYS: f64 = 25_569.0;

/// A DAX scalar value.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Value {
    #[default]
    Blank,
    Boolean(bool),
    Integer(i64),
    Double(f64),
    /// Fixed decimal (DAX currency), scaled by 10⁴: `Decimal(12_345)` is `1.2345`.
    Decimal(i128),
    String(Arc<str>),
    /// Naive date/time: microseconds since 1970-01-01T00:00:00 (no time zone).
    DateTime(i64),
}

/// The type of a [`Value`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueType {
    Blank,
    Boolean,
    Integer,
    Double,
    Decimal,
    String,
    DateTime,
}

impl ValueType {
    /// Display name, e.g. `Integer`.
    pub fn name(self) -> &'static str {
        match self {
            ValueType::Blank => "Blank",
            ValueType::Boolean => "Boolean",
            ValueType::Integer => "Integer",
            ValueType::Double => "Double",
            ValueType::Decimal => "Decimal",
            ValueType::String => "String",
            ValueType::DateTime => "DateTime",
        }
    }

    /// Arrow storage type for values of this type (`Utf8` for `Blank`).
    pub fn arrow_type(self) -> ArrowType {
        match self {
            ValueType::Blank | ValueType::String => ArrowType::Utf8,
            ValueType::Boolean => ArrowType::Boolean,
            ValueType::Integer => ArrowType::Int64,
            ValueType::Double => ArrowType::Float64,
            ValueType::Decimal => ArrowType::Decimal128(DECIMAL_PRECISION, DECIMAL_SCALE),
            ValueType::DateTime => ArrowType::Timestamp(TimeUnit::Microsecond, None),
        }
    }
}

impl fmt::Display for ValueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl Value {
    /// A text value.
    pub fn string(text: impl Into<Arc<str>>) -> Value {
        Value::String(text.into())
    }

    /// A fixed decimal from a float, rounded half away from zero to 4
    /// decimals. `None` if not finite or out of range.
    pub fn decimal_from_f64(value: f64) -> Option<Value> {
        let scaled = (value * DECIMAL_FACTOR as f64).round();
        if !scaled.is_finite() || scaled.abs() >= 1.0e38 {
            return None;
        }
        Some(Value::Decimal(scaled as i128))
    }

    /// A date (midnight). `None` for invalid dates.
    pub fn date(year: i64, month: u32, day: u32) -> Option<Value> {
        Self::datetime(year, month, day, 0, 0, 0)
    }

    /// A naive date/time. `None` for invalid components.
    pub fn datetime(
        year: i64,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> Option<Value> {
        if !(1..=12).contains(&month)
            || day == 0
            || day > days_in_month(year, month)
            || hour > 23
            || minute > 59
            || second > 59
        {
            return None;
        }
        let days = days_from_civil(year, month, day)?;
        let secs = i64::from(hour * 3600 + minute * 60 + second);
        days.checked_mul(MICROS_PER_DAY)?
            .checked_add(secs * 1_000_000)
            .map(Value::DateTime)
    }

    /// A date/time from a DAX serial number (days since 1899-12-30, fraction
    /// = time of day). `None` if not finite or out of range.
    pub fn from_oadate(serial: f64) -> Option<Value> {
        let micros = ((serial - OADATE_UNIX_EPOCH_DAYS) * MICROS_PER_DAY as f64).round();
        if !micros.is_finite() || micros.abs() >= 9.0e18 {
            return None;
        }
        Some(Value::DateTime(micros as i64))
    }

    /// `true` for BLANK.
    pub fn is_blank(&self) -> bool {
        matches!(self, Value::Blank)
    }

    /// The value's type.
    pub fn value_type(&self) -> ValueType {
        match self {
            Value::Blank => ValueType::Blank,
            Value::Boolean(_) => ValueType::Boolean,
            Value::Integer(_) => ValueType::Integer,
            Value::Double(_) => ValueType::Double,
            Value::Decimal(_) => ValueType::Decimal,
            Value::String(_) => ValueType::String,
            Value::DateTime(_) => ValueType::DateTime,
        }
    }

    /// Type name, e.g. `Integer`.
    pub fn type_name(&self) -> &'static str {
        self.value_type().name()
    }

    /// `true` for `Integer`, `Double` and `Decimal`.
    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            Value::Integer(_) | Value::Double(_) | Value::Decimal(_)
        )
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// `Integer` values, and `Double`/`Decimal` values that are exact integers.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Integer(i) => Some(*i),
            Value::Double(d) if d.fract() == 0.0 && in_i64_range(*d) => Some(*d as i64),
            Value::Decimal(d) if d % DECIMAL_FACTOR == 0 => i64::try_from(d / DECIMAL_FACTOR).ok(),
            _ => None,
        }
    }

    /// Numeric value as `f64`; dates are DAX serial numbers. `None` for
    /// BLANK, text and booleans.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Integer(i) => Some(*i as f64),
            Value::Double(d) => Some(*d),
            Value::Decimal(d) => Some(decimal_to_f64(*d)),
            Value::DateTime(us) => Some(micros_to_oadate(*us)),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// Microseconds since 1970-01-01 for `DateTime` values.
    pub fn as_datetime_micros(&self) -> Option<i64> {
        match self {
            Value::DateTime(us) => Some(*us),
            _ => None,
        }
    }

    /// The hashing key (see [`ValueKey`]).
    pub fn key(&self) -> ValueKey {
        ValueKey::from(self)
    }

    /// DAX text conversion (`BLANK` → `""`, `TRUE` → `"True"`); dates are
    /// formatted as `yyyy-mm-dd hh:mm:ss`. `FORMAT` belongs to `pbi-dax`.
    pub fn to_text(&self) -> String {
        self.to_string()
    }

    /// DAX `=` (see the module docs).
    pub fn dax_eq(&self, other: &Value) -> Result<bool> {
        Ok(self.dax_cmp(other)? == Ordering::Equal)
    }

    /// DAX `==`: like [`Value::dax_eq`] but BLANK only equals BLANK.
    pub fn dax_strict_eq(&self, other: &Value) -> Result<bool> {
        match (self, other) {
            (Value::Blank, Value::Blank) => Ok(true),
            (Value::Blank, _) | (_, Value::Blank) => Ok(false),
            _ => self.dax_eq(other),
        }
    }

    /// DAX relational comparison (`<`, `<=`, `>`, `>=`, `=`): BLANK is coerced
    /// to the other operand's zero, text compares case-insensitively.
    pub fn dax_cmp(&self, other: &Value) -> Result<Ordering> {
        match (self, other) {
            (Value::Blank, Value::Blank) => Ok(Ordering::Equal),
            (Value::Blank, v) => blank_like(v).dax_cmp(v),
            (v, Value::Blank) => v.dax_cmp(&blank_like(v)),
            (Value::String(a), Value::String(b)) => Ok(cmp_text_ci(a, b)),
            (Value::Boolean(a), Value::Boolean(b)) => Ok(a.cmp(b)),
            (a, b) => numeric_cmp(a, b).ok_or(EngineError::IncomparableTypes {
                left: a.type_name(),
                right: b.type_name(),
            }),
        }
    }

    /// Total order for sorting: BLANK first, then booleans, numbers, dates and
    /// text (case-insensitive, ties broken by exact text). Never fails.
    pub fn total_cmp(&self, other: &Value) -> Ordering {
        fn rank(v: &Value) -> u8 {
            match v {
                Value::Blank => 0,
                Value::Boolean(_) => 1,
                Value::Integer(_) | Value::Double(_) | Value::Decimal(_) => 2,
                Value::DateTime(_) => 3,
                Value::String(_) => 4,
            }
        }
        match (self, other) {
            (Value::String(a), Value::String(b)) => {
                cmp_text_ci(a, b).then_with(|| a.as_ref().cmp(b.as_ref()))
            }
            (Value::Boolean(a), Value::Boolean(b)) => a.cmp(b),
            (Value::DateTime(a), Value::DateTime(b)) => a.cmp(b),
            (a, b) if rank(a) == 2 && rank(b) == 2 => numeric_cmp(a, b).unwrap_or(Ordering::Equal),
            (a, b) => rank(a).cmp(&rank(b)),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Blank => Ok(()),
            Value::Boolean(true) => f.write_str("True"),
            Value::Boolean(false) => f.write_str("False"),
            Value::Integer(i) => write!(f, "{i}"),
            Value::Double(d) => fmt_double(*d, f),
            Value::Decimal(d) => fmt_decimal(*d, f),
            Value::String(s) => f.write_str(s),
            Value::DateTime(us) => fmt_datetime(*us, f),
        }
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Boolean(v)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Integer(v)
    }
}

impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::Integer(i64::from(v))
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Double(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::String(Arc::from(v))
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::String(Arc::from(v))
    }
}

impl From<Arc<str>> for Value {
    fn from(v: Arc<str>) -> Self {
        Value::String(v)
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        v.map_or(Value::Blank, Into::into)
    }
}

/// Hashing key for grouping, `IN` filters and relationship joins.
///
/// Semantics follow DAX `==` with VertiPaq's case-insensitive dictionaries:
/// BLANK is its own key, numbers of any type with the same value share a key
/// (`1`, `1.0` and `1.0000` currency), and text is case-folded.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ValueKey {
    Blank,
    Boolean(bool),
    /// Integral numbers (from any numeric type).
    Integer(i64),
    /// Non-integral numbers: the `f64` bit pattern (NaN canonicalised).
    Number(u64),
    /// Case-folded text.
    Text(Box<str>),
    DateTime(i64),
}

impl ValueKey {
    /// Key for a text value (case-folded).
    pub fn text(text: &str) -> ValueKey {
        ValueKey::Text(fold_text(text).into_boxed_str())
    }

    fn number(value: f64) -> ValueKey {
        if value.fract() == 0.0 && in_i64_range(value) {
            ValueKey::Integer(value as i64)
        } else if value.is_nan() {
            ValueKey::Number(f64::NAN.to_bits())
        } else {
            ValueKey::Number(value.to_bits())
        }
    }
}

impl From<&Value> for ValueKey {
    fn from(value: &Value) -> Self {
        match value {
            Value::Blank => ValueKey::Blank,
            Value::Boolean(b) => ValueKey::Boolean(*b),
            Value::Integer(i) => ValueKey::Integer(*i),
            Value::Double(d) => ValueKey::number(*d),
            Value::Decimal(d) => {
                if d % DECIMAL_FACTOR == 0
                    && let Ok(i) = i64::try_from(d / DECIMAL_FACTOR)
                {
                    ValueKey::Integer(i)
                } else {
                    ValueKey::number(decimal_to_f64(*d))
                }
            }
            Value::String(s) => ValueKey::text(s),
            Value::DateTime(us) => ValueKey::DateTime(*us),
        }
    }
}

impl From<Value> for ValueKey {
    fn from(value: Value) -> Self {
        ValueKey::from(&value)
    }
}

/// Case folding used for text keys (Unicode lowercase).
pub fn fold_text(text: &str) -> String {
    if text
        .bytes()
        .all(|b| b.is_ascii() && !b.is_ascii_uppercase())
    {
        text.to_owned()
    } else {
        text.chars().flat_map(char::to_lowercase).collect()
    }
}

/// Case-insensitive text ordering.
pub fn cmp_text_ci(a: &str, b: &str) -> Ordering {
    if a.is_ascii() && b.is_ascii() {
        let (x, y) = (a.as_bytes(), b.as_bytes());
        return x
            .iter()
            .map(u8::to_ascii_lowercase)
            .cmp(y.iter().map(u8::to_ascii_lowercase));
    }
    a.chars()
        .flat_map(char::to_lowercase)
        .cmp(b.chars().flat_map(char::to_lowercase))
}

fn blank_like(v: &Value) -> Value {
    match v {
        Value::Boolean(_) => Value::Boolean(false),
        Value::String(_) => Value::String(Arc::from("")),
        Value::Double(_) => Value::Double(0.0),
        Value::Decimal(_) => Value::Decimal(0),
        _ => Value::Integer(0),
    }
}

/// Numeric comparison across `Integer`, `Double`, `Decimal` and `DateTime`.
fn numeric_cmp(a: &Value, b: &Value) -> Option<Ordering> {
    match (a, b) {
        (Value::Integer(x), Value::Integer(y)) => Some(x.cmp(y)),
        (Value::Decimal(x), Value::Decimal(y)) => Some(x.cmp(y)),
        (Value::DateTime(x), Value::DateTime(y)) => Some(x.cmp(y)),
        (Value::Integer(x), Value::Decimal(y)) => Some((i128::from(*x) * DECIMAL_FACTOR).cmp(y)),
        (Value::Decimal(x), Value::Integer(y)) => Some(x.cmp(&(i128::from(*y) * DECIMAL_FACTOR))),
        _ if is_number_or_date(a) && is_number_or_date(b) => {
            let (x, y) = (a.as_f64()?, b.as_f64()?);
            Some(x.partial_cmp(&y).unwrap_or_else(|| x.total_cmp(&y)))
        }
        _ => None,
    }
}

fn is_number_or_date(v: &Value) -> bool {
    v.is_numeric() || matches!(v, Value::DateTime(_))
}

fn in_i64_range(d: f64) -> bool {
    (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&d)
}

/// `Decimal` raw value → `f64`.
pub fn decimal_to_f64(raw: i128) -> f64 {
    raw as f64 / DECIMAL_FACTOR as f64
}

/// Microseconds since 1970-01-01 → DAX serial date.
pub fn micros_to_oadate(micros: i64) -> f64 {
    micros as f64 / MICROS_PER_DAY as f64 + OADATE_UNIX_EPOCH_DAYS
}

/// `v / d` rounded half away from zero (`d > 0`).
pub(crate) fn round_div(v: i128, d: i128) -> i128 {
    let q = v / d;
    let r = v % d;
    if r.unsigned_abs() * 2 >= d.unsigned_abs() {
        q + v.signum()
    } else {
        q
    }
}

/// Rescales a decimal with `scale` fractional digits to [`DECIMAL_SCALE`].
fn rescale_decimal(raw: i128, scale: i8) -> Option<i128> {
    let diff = i32::from(DECIMAL_SCALE) - i32::from(scale);
    match diff.cmp(&0) {
        Ordering::Equal => Some(raw),
        Ordering::Greater => raw.checked_mul(10i128.checked_pow(diff.unsigned_abs())?),
        Ordering::Less => {
            let factor = 10i128.checked_pow(diff.unsigned_abs())?;
            Some(round_div(raw, factor))
        }
    }
}

fn fmt_double(d: f64, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    if d.is_nan() {
        f.write_str("NaN")
    } else if d.is_infinite() {
        f.write_str(if d > 0.0 { "Infinity" } else { "-Infinity" })
    } else if d.fract() == 0.0 && d.abs() < 1.0e15 {
        write!(f, "{d:.0}")
    } else {
        write!(f, "{d}")
    }
}

fn fmt_decimal(raw: i128, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let sign = if raw < 0 { "-" } else { "" };
    let abs = raw.unsigned_abs();
    let factor = DECIMAL_FACTOR.unsigned_abs();
    let (int, frac) = (abs / factor, abs % factor);
    if frac == 0 {
        write!(f, "{sign}{int}")
    } else {
        let digits = format!("{frac:04}");
        write!(f, "{sign}{int}.{}", digits.trim_end_matches('0'))
    }
}

fn fmt_datetime(micros: i64, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let days = micros.div_euclid(MICROS_PER_DAY);
    let rem = micros.rem_euclid(MICROS_PER_DAY);
    let (y, m, d) = civil_from_days(days);
    let secs = rem / 1_000_000;
    let frac = rem % 1_000_000;
    write!(
        f,
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60
    )?;
    if frac != 0 {
        write!(f, ".{frac:06}")?;
    }
    Ok(())
}

fn is_leap_year(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(y) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant's
/// algorithm). `None` if the result does not fit in an `i64`.
pub fn days_from_civil(year: i64, month: u32, day: u32) -> Option<i64> {
    let y = i128::from(year) - i128::from(month <= 2);
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = (i128::from(month) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i128::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    i64::try_from(era * 146_097 + doe - 719_468).ok()
}

/// Proleptic Gregorian `(year, month, day)` of a day count since 1970-01-01.
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ---------------------------------------------------------------------------
// Arrow interop
// ---------------------------------------------------------------------------

/// Arrow storage type of a model column type.
///
/// `Binary` maps to Arrow `Binary` (readable by DataFusion, but not as a
/// [`Value`]); `Variant`, `Automatic` and unknown types map to `Utf8`.
pub fn arrow_type(data_type: &DataType) -> ArrowType {
    match data_type {
        DataType::String => ArrowType::Utf8,
        DataType::Int64 => ArrowType::Int64,
        DataType::Double => ArrowType::Float64,
        DataType::Decimal => ArrowType::Decimal128(DECIMAL_PRECISION, DECIMAL_SCALE),
        DataType::DateTime => ArrowType::Timestamp(TimeUnit::Microsecond, None),
        DataType::Boolean => ArrowType::Boolean,
        DataType::Binary => ArrowType::Binary,
        DataType::Variant | DataType::Automatic | DataType::Unknown(_) => ArrowType::Utf8,
    }
}

/// The canonical type a column of Arrow type `t` is read as, or `None` if it
/// cannot be represented as [`Value`]s.
///
/// Canonical types: `Null`, `Boolean`, `Int64`, `Float64`, `Utf8`,
/// `Decimal128(_, _)` and `Timestamp(Microsecond, None)`. Other integer and
/// float widths, large/view strings, dates, timestamps (any unit; time zones
/// are dropped, keeping wall-clock time), other decimal widths and
/// dictionary-encoded columns are converted.
pub fn canonical_type(t: &ArrowType) -> Option<ArrowType> {
    use ArrowType as A;
    Some(match t {
        A::Null
        | A::Boolean
        | A::Int64
        | A::Float64
        | A::Utf8
        | A::Decimal128(_, _)
        | A::Timestamp(TimeUnit::Microsecond, None) => t.clone(),
        A::Int8 | A::Int16 | A::Int32 | A::UInt8 | A::UInt16 | A::UInt32 | A::UInt64 => A::Int64,
        A::Float16 | A::Float32 => A::Float64,
        A::LargeUtf8 | A::Utf8View => A::Utf8,
        A::Date32 | A::Date64 | A::Timestamp(_, _) => A::Timestamp(TimeUnit::Microsecond, None),
        A::Decimal32(_, _) | A::Decimal64(_, _) | A::Decimal256(_, _) => {
            A::Decimal128(38, DECIMAL_SCALE)
        }
        A::Dictionary(_, values) => canonical_type(values)?,
        _ => return None,
    })
}

/// Converts `array` to its canonical type (see [`canonical_type`]); a no-op
/// (cheap `Arc` clone) when it already is.
pub fn to_canonical(array: &ArrayRef) -> Result<ArrayRef> {
    let target = canonical_type(array.data_type()).ok_or_else(|| EngineError::UnsupportedType {
        data_type: array.data_type().to_string(),
        context: "column cannot be read as DAX values".into(),
    })?;
    if &target == array.data_type() {
        return Ok(Arc::clone(array));
    }
    let options = CastOptions {
        safe: false,
        ..CastOptions::default()
    };
    Ok(cast_with_options(array, &target, &options)?)
}

/// Reads row `row` of an array of a canonical type.
pub(crate) fn read_canonical(array: &dyn Array, row: usize) -> Result<Value> {
    if row >= array.len() {
        return Err(EngineError::InvalidArgument(format!(
            "row {row} out of range for an array of length {}",
            array.len()
        )));
    }
    if array.is_null(row) {
        return Ok(Value::Blank);
    }
    let wrong = || EngineError::UnsupportedType {
        data_type: array.data_type().to_string(),
        context: "not a canonical engine type".into(),
    };
    Ok(match array.data_type() {
        ArrowType::Null => Value::Blank,
        ArrowType::Boolean => Value::Boolean(array.as_boolean_opt().ok_or_else(wrong)?.value(row)),
        ArrowType::Int64 => Value::Integer(
            array
                .as_primitive_opt::<Int64Type>()
                .ok_or_else(wrong)?
                .value(row),
        ),
        ArrowType::Float64 => Value::Double(
            array
                .as_primitive_opt::<Float64Type>()
                .ok_or_else(wrong)?
                .value(row),
        ),
        ArrowType::Utf8 => Value::String(Arc::from(
            array.as_string_opt::<i32>().ok_or_else(wrong)?.value(row),
        )),
        ArrowType::Decimal128(_, scale) => {
            let raw = array
                .as_primitive_opt::<Decimal128Type>()
                .ok_or_else(wrong)?
                .value(row);
            Value::Decimal(
                rescale_decimal(raw, *scale)
                    .ok_or_else(|| EngineError::Overflow("decimal rescale".into()))?,
            )
        }
        ArrowType::Timestamp(TimeUnit::Microsecond, None) => Value::DateTime(
            array
                .as_primitive_opt::<TimestampMicrosecondType>()
                .ok_or_else(wrong)?
                .value(row),
        ),
        _ => return Err(wrong()),
    })
}

/// Reads one element of any supported Arrow array as a [`Value`] (nulls are
/// BLANK). For many reads of the same column prefer
/// [`crate::storage::Storage::value_at`], which uses the column's cached
/// dictionary encoding.
pub fn value_from_array(array: &ArrayRef, row: usize) -> Result<Value> {
    if canonical_type(array.data_type()).as_ref() == Some(array.data_type()) {
        return read_canonical(array.as_ref(), row);
    }
    if row >= array.len() {
        return Err(EngineError::InvalidArgument(format!(
            "row {row} out of range for an array of length {}",
            array.len()
        )));
    }
    let one = to_canonical(&array.slice(row, 1))?;
    read_canonical(one.as_ref(), 0)
}

/// Reads a whole array as [`Value`]s.
pub fn values_from_array(array: &ArrayRef) -> Result<Vec<Value>> {
    let canonical = to_canonical(array)?;
    (0..canonical.len())
        .map(|row| read_canonical(canonical.as_ref(), row))
        .collect()
}

/// The Arrow type that can hold all of `values` (DAX-like type
/// unification): integers widen to decimals or doubles, any mix with text
/// or of otherwise incompatible types becomes `Utf8`. All-BLANK → `Utf8`.
pub fn infer_arrow_type(values: &[Value]) -> ArrowType {
    let mut acc: Option<ValueType> = None;
    for v in values {
        let t = v.value_type();
        if t == ValueType::Blank {
            continue;
        }
        acc = Some(match (acc, t) {
            (None, t) => t,
            (Some(a), t) if a == t => a,
            (Some(ValueType::Integer), ValueType::Decimal)
            | (Some(ValueType::Decimal), ValueType::Integer) => ValueType::Decimal,
            (Some(a), t)
                if matches!(
                    a,
                    ValueType::Integer | ValueType::Decimal | ValueType::Double
                ) && matches!(
                    t,
                    ValueType::Integer | ValueType::Decimal | ValueType::Double
                ) =>
            {
                ValueType::Double
            }
            _ => ValueType::String,
        });
    }
    acc.unwrap_or(ValueType::String).arrow_type()
}

fn conversion_error(v: &Value, target: &ArrowType) -> EngineError {
    EngineError::Conversion {
        value: format!("{} {v:?}", v.type_name()),
        target: target.to_string(),
    }
}

fn to_i64(v: &Value, target: &ArrowType) -> Result<Option<i64>> {
    Ok(match v {
        Value::Blank => None,
        Value::Integer(i) => Some(*i),
        Value::Boolean(b) => Some(i64::from(*b)),
        Value::Double(d) => {
            let r = d.round();
            if !in_i64_range(r) {
                return Err(conversion_error(v, target));
            }
            Some(r as i64)
        }
        Value::Decimal(d) => Some(
            i64::try_from(round_div(*d, DECIMAL_FACTOR))
                .map_err(|_| conversion_error(v, target))?,
        ),
        Value::String(s) => Some(
            s.trim()
                .parse::<i64>()
                .map_err(|_| conversion_error(v, target))?,
        ),
        Value::DateTime(_) => return Err(conversion_error(v, target)),
    })
}

fn to_f64(v: &Value, target: &ArrowType) -> Result<Option<f64>> {
    Ok(match v {
        Value::Blank => None,
        Value::Boolean(b) => Some(f64::from(u8::from(*b))),
        Value::String(s) => Some(
            s.trim()
                .parse::<f64>()
                .map_err(|_| conversion_error(v, target))?,
        ),
        other => other.as_f64(),
    })
}

fn to_decimal(v: &Value, target: &ArrowType) -> Result<Option<i128>> {
    Ok(match v {
        Value::Blank => None,
        Value::Decimal(d) => Some(*d),
        Value::Integer(i) => Some(i128::from(*i) * DECIMAL_FACTOR),
        Value::Boolean(b) => Some(i128::from(*b) * DECIMAL_FACTOR),
        Value::Double(_) | Value::String(_) => {
            let f = to_f64(v, target)?.ok_or_else(|| conversion_error(v, target))?;
            match Value::decimal_from_f64(f) {
                Some(Value::Decimal(d)) => Some(d),
                _ => return Err(conversion_error(v, target)),
            }
        }
        Value::DateTime(_) => return Err(conversion_error(v, target)),
    })
}

fn to_datetime(v: &Value, target: &ArrowType) -> Result<Option<i64>> {
    Ok(match v {
        Value::Blank => None,
        Value::DateTime(us) => Some(*us),
        Value::Integer(_) | Value::Double(_) | Value::Decimal(_) => {
            match v.as_f64().and_then(Value::from_oadate) {
                Some(Value::DateTime(us)) => Some(us),
                _ => return Err(conversion_error(v, target)),
            }
        }
        _ => return Err(conversion_error(v, target)),
    })
}

fn to_bool(v: &Value, target: &ArrowType) -> Result<Option<bool>> {
    Ok(match v {
        Value::Blank => None,
        Value::Boolean(b) => Some(*b),
        Value::String(s) if s.trim().eq_ignore_ascii_case("true") => Some(true),
        Value::String(s) if s.trim().eq_ignore_ascii_case("false") => Some(false),
        Value::Integer(_) | Value::Double(_) | Value::Decimal(_) => v.as_f64().map(|f| f != 0.0),
        _ => return Err(conversion_error(v, target)),
    })
}

/// Builds an Arrow array of type `target` from `values` (BLANK → null),
/// converting like DAX does when a calculated column has a declared type
/// (numbers round to whole numbers half away from zero, booleans become
/// 0/1, text parses). Non-canonical targets (e.g. `Int32`) are built in the
/// canonical type and cast. Unconvertible values are an error.
pub fn values_to_array(values: &[Value], target: &ArrowType) -> Result<ArrayRef> {
    let canonical = canonical_type(target).ok_or_else(|| EngineError::UnsupportedType {
        data_type: target.to_string(),
        context: "cannot build a column of this type from DAX values".into(),
    })?;
    let built: ArrayRef = match &canonical {
        ArrowType::Null => {
            if let Some(v) = values.iter().find(|v| !v.is_blank()) {
                return Err(conversion_error(v, target));
            }
            new_null_array(&ArrowType::Null, values.len())
        }
        ArrowType::Utf8 => {
            let mut b = StringBuilder::with_capacity(values.len(), values.len() * 8);
            for v in values {
                match v {
                    Value::Blank => b.append_null(),
                    Value::String(s) => b.append_value(s),
                    other => b.append_value(other.to_text()),
                }
            }
            Arc::new(b.finish())
        }
        ArrowType::Int64 => {
            let mut b = Int64Builder::with_capacity(values.len());
            for v in values {
                b.append_option(to_i64(v, target)?);
            }
            Arc::new(b.finish())
        }
        ArrowType::Float64 => {
            let mut b = Float64Builder::with_capacity(values.len());
            for v in values {
                b.append_option(to_f64(v, target)?);
            }
            Arc::new(b.finish())
        }
        ArrowType::Boolean => {
            let mut b = BooleanBuilder::with_capacity(values.len());
            for v in values {
                b.append_option(to_bool(v, target)?);
            }
            Arc::new(b.finish())
        }
        ArrowType::Decimal128(_, _) => {
            let mut b = Decimal128Builder::with_capacity(values.len())
                .with_precision_and_scale(DECIMAL_PRECISION, DECIMAL_SCALE)?;
            for v in values {
                b.append_option(to_decimal(v, target)?);
            }
            Arc::new(b.finish())
        }
        ArrowType::Timestamp(TimeUnit::Microsecond, None) => {
            let mut b = TimestampMicrosecondBuilder::with_capacity(values.len());
            for v in values {
                b.append_option(to_datetime(v, target)?);
            }
            Arc::new(b.finish())
        }
        other => {
            return Err(EngineError::UnsupportedType {
                data_type: other.to_string(),
                context: "cannot build a column of this type from DAX values".into(),
            });
        }
    };
    if built.data_type() == target {
        Ok(built)
    } else {
        let options = CastOptions {
            safe: false,
            ..CastOptions::default()
        };
        Ok(cast_with_options(&built, target, &options)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{
        Date32Array, Decimal128Array, DictionaryArray, Int32Array, StringArray,
        TimestampNanosecondArray,
    };
    use arrow::datatypes::Int8Type;

    fn s(text: &str) -> Value {
        Value::from(text)
    }

    #[test]
    fn dax_equality_coerces_blank_but_strict_equality_does_not() {
        assert!(Value::Blank.dax_eq(&Value::Integer(0)).unwrap());
        assert!(Value::Blank.dax_eq(&s("")).unwrap());
        assert!(Value::Blank.dax_eq(&Value::Boolean(false)).unwrap());
        assert!(Value::Double(0.0).dax_eq(&Value::Blank).unwrap());
        assert!(!Value::Blank.dax_eq(&Value::Integer(1)).unwrap());

        assert!(!Value::Blank.dax_strict_eq(&Value::Integer(0)).unwrap());
        assert!(!Value::Blank.dax_strict_eq(&s("")).unwrap());
        assert!(Value::Blank.dax_strict_eq(&Value::Blank).unwrap());
        assert!(s("abc").dax_strict_eq(&s("ABC")).unwrap());
    }

    #[test]
    fn text_comparison_is_case_insensitive() {
        assert!(s("Camping").dax_eq(&s("CAMPING")).unwrap());
        assert!(s("Äpfel").dax_eq(&s("äPFEL")).unwrap());
        assert_eq!(s("apple").dax_cmp(&s("Banana")).unwrap(), Ordering::Less);
        assert_eq!(s("b").dax_cmp(&Value::Blank).unwrap(), Ordering::Greater);
        assert_eq!(s("Camping").key(), s("cAmPiNg").key());
        assert_ne!(s("").key(), Value::Blank.key());
    }

    #[test]
    fn numbers_compare_across_types_and_incompatible_types_error() {
        assert!(Value::Integer(1).dax_eq(&Value::Double(1.0)).unwrap());
        assert!(Value::Integer(2).dax_eq(&Value::Decimal(20_000)).unwrap());
        assert_eq!(
            Value::Decimal(15_000).dax_cmp(&Value::Double(1.4)).unwrap(),
            Ordering::Greater
        );
        assert_eq!(Value::Integer(1).key(), Value::Double(1.0).key());
        assert_eq!(Value::Decimal(10_000).key(), Value::Integer(1).key());
        assert_eq!(Value::Double(0.5).key(), Value::Decimal(5_000).key());
        assert!(matches!(
            s("1").dax_eq(&Value::Integer(1)),
            Err(EngineError::IncomparableTypes { .. })
        ));
        assert!(Value::Boolean(true).dax_cmp(&Value::Integer(1)).is_err());
    }

    #[test]
    fn dates_are_serial_numbers() {
        let d = Value::date(2026, 10, 21).unwrap();
        assert_eq!(d.to_text(), "2026-10-21 00:00:00");
        let serial = d.as_f64().unwrap();
        assert_eq!(serial, 46_316.0);
        assert_eq!(Value::from_oadate(serial), Some(d.clone()));
        assert!(d.dax_eq(&Value::Integer(46_316)).unwrap());
        assert_eq!(Value::date(1899, 12, 30).unwrap().as_f64(), Some(0.0));
        assert!(Value::date(2026, 2, 29).is_none());
        assert!(Value::date(i64::MAX, 1, 1).is_none());
        assert_eq!(
            Value::datetime(2024, 2, 29, 13, 5, 9).unwrap().to_text(),
            "2024-02-29 13:05:09"
        );
        for days in [-800_000, -1, 0, 1, 19_000, 2_932_896] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), Some(days));
        }
    }

    #[test]
    fn text_conversion_follows_dax() {
        assert_eq!(Value::Blank.to_text(), "");
        assert_eq!(Value::Boolean(true).to_text(), "True");
        assert_eq!(Value::Double(2.0).to_text(), "2");
        assert_eq!(Value::Double(0.25).to_text(), "0.25");
        assert_eq!(Value::Decimal(12_500).to_text(), "1.25");
        assert_eq!(Value::Decimal(-5).to_text(), "-0.0005");
    }

    #[test]
    fn total_order_sorts_blank_first_and_text_case_insensitively() {
        let mut v = vec![s("b"), Value::Integer(3), s("A"), Value::Blank, s("a")];
        v.sort_by(Value::total_cmp);
        assert_eq!(
            v,
            vec![Value::Blank, Value::Integer(3), s("A"), s("a"), s("b")]
        );
    }

    #[test]
    fn arrow_type_mapping() {
        assert_eq!(arrow_type(&DataType::String), ArrowType::Utf8);
        assert_eq!(arrow_type(&DataType::Int64), ArrowType::Int64);
        assert_eq!(arrow_type(&DataType::Decimal), ArrowType::Decimal128(19, 4));
        assert_eq!(
            arrow_type(&DataType::DateTime),
            ArrowType::Timestamp(TimeUnit::Microsecond, None)
        );
        assert_eq!(
            arrow_type(&DataType::Unknown("geo".into())),
            ArrowType::Utf8
        );
    }

    #[test]
    fn reads_non_canonical_arrow_arrays() {
        let ints: ArrayRef = Arc::new(Int32Array::from(vec![Some(7), None]));
        assert_eq!(value_from_array(&ints, 0).unwrap(), Value::Integer(7));
        assert_eq!(value_from_array(&ints, 1).unwrap(), Value::Blank);
        assert!(value_from_array(&ints, 2).is_err());

        let dates: ArrayRef = Arc::new(Date32Array::from(vec![20_382]));
        assert_eq!(
            value_from_array(&dates, 0).unwrap(),
            Value::date(2025, 10, 21).unwrap()
        );
        let ns: ArrayRef = Arc::new(TimestampNanosecondArray::from(vec![1_500]));
        assert_eq!(value_from_array(&ns, 0).unwrap(), Value::DateTime(1));

        let dec: ArrayRef = Arc::new(
            Decimal128Array::from(vec![12_345_678])
                .with_precision_and_scale(10, 6)
                .unwrap(),
        );
        assert_eq!(value_from_array(&dec, 0).unwrap(), Value::Decimal(123_457));

        let dict: DictionaryArray<Int8Type> = vec!["x", "y", "x"].into_iter().collect();
        let dict: ArrayRef = Arc::new(dict);
        assert_eq!(
            values_from_array(&dict).unwrap(),
            vec![s("x"), s("y"), s("x")]
        );

        let list: ArrayRef = Arc::new(arrow::array::ListArray::new_null(
            Arc::new(arrow::datatypes::Field::new_list_field(
                ArrowType::Int64,
                true,
            )),
            1,
        ));
        assert!(matches!(
            value_from_array(&list, 0),
            Err(EngineError::UnsupportedType { .. })
        ));
    }

    #[test]
    fn builds_arrow_arrays_from_values() {
        let values = vec![Value::Integer(1), Value::Blank, Value::Double(2.5)];
        let arr = values_to_array(&values, &ArrowType::Int64).unwrap();
        assert_eq!(
            values_from_array(&arr).unwrap(),
            vec![Value::Integer(1), Value::Blank, Value::Integer(3)]
        );
        let arr = values_to_array(&values, &ArrowType::Utf8).unwrap();
        let strings = arr.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(strings.value(2), "2.5");
        let arr = values_to_array(&values, &arrow_type(&DataType::Decimal)).unwrap();
        assert_eq!(value_from_array(&arr, 2).unwrap(), Value::Decimal(25_000));
        let arr = values_to_array(&[Value::Integer(5)], &ArrowType::Int32).unwrap();
        assert_eq!(arr.data_type(), &ArrowType::Int32);
        assert!(values_to_array(&[s("x")], &ArrowType::Int64).is_err());
        assert_eq!(infer_arrow_type(&values), ArrowType::Float64);
        assert_eq!(infer_arrow_type(&[Value::Blank, s("a")]), ArrowType::Utf8);
        assert_eq!(
            infer_arrow_type(&[Value::Integer(1), Value::Decimal(1)]),
            arrow_type(&DataType::Decimal)
        );
    }
}
