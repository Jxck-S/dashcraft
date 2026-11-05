//! Power BI / Excel-style value formatting: model `formatString`s and the
//! DAX `FORMAT` function.
//!
//! Format strings follow the Visual Basic (OLE Automation) dialect used by
//! Analysis Services, with the common .NET spellings accepted as well:
//!
//! - **Predefined formats** (case-insensitive): `General`, `General Number`,
//!   `Currency`, `Fixed`, `Standard`, `Percent`, `Scientific`, `Yes/No`,
//!   `True/False`, `On/Off`, `General Date`, `Long Date`, `Medium Date`,
//!   `Short Date`, `Long Time`, `Medium Time`, `Short Time`.
//! - **Custom numeric formats** ([`number`]): up to three `;`-separated
//!   sections (positive; negative; zero), digit placeholders `0` and `#`,
//!   `.` decimal point, `,` thousands separator or ÷1000 scaling, `%`,
//!   `E+`/`E-` scientific notation, `\x` and `"text"` literals. A value that
//!   rounds to zero uses the zero section; a negative value formatted with
//!   the first section gets a leading `-`.
//! - **Custom date/time formats** ([`datetime`]): `d`…`dddddd`, `w`, `ww`,
//!   `m`…`mmmm` (minutes when `m`/`mm` follows an hour or precedes a
//!   second), `q`, `y`, `yy`, `yyyy`, `h`, `hh` (24-hour unless an AM/PM
//!   designator is present), `n`, `nn`, `s`, `ss`, `f…`, `c`, `ttttt`,
//!   `AM/PM`, `am/pm`, `A/P`, `a/p`, `AMPM`, `tt`. Letters are
//!   case-insensitive, so .NET-style `MM/dd/yyyy HH:mm` works too.
//!
//! Numbers formatted with a date format are read as serial dates (days since
//! 1899-12-30, a negative serial's fraction being the time of day as in OLE
//! Automation) and dates formatted with a numeric format as serial numbers,
//! as in Power BI. Booleans are `-1`/`0` under numeric and date formats.
//! Text is returned unchanged; BLANK formats as the empty string; a format
//! of only text placeholders (`@`, `>`, …) shows the `General` text. The
//! rules were checked against Power BI Desktop (`reference/groundtruth`,
//! suite `format`). Rounding is half away from zero on the value's 15
//! significant digits (as Excel and Power BI display numbers), so `0.125`
//! with `0.00` is `0.13`.
//!
//! Only the `en-US` culture is implemented; the `culture` argument is
//! accepted for forward compatibility and currently ignored.
//!
//! ```
//! use pbi_dax::format::format_value;
//! use pbi_engine::Value;
//!
//! let pct = Value::Double(5.0 / 11.0);
//! assert_eq!(format_value(&pct, Some("0.0%;-0.0%;0.0%"), "en-US"), "45.5%");
//! assert_eq!(format_value(&Value::Integer(1234567), Some("#,0"), "en-US"), "1,234,567");
//! assert_eq!(format_value(&Value::Blank, Some("0"), "en-US"), "");
//! ```

pub mod datetime;
pub mod number;

use pbi_engine::Value;

use self::number::Digits;

/// Why [`try_format_value`] (the DAX `FORMAT` function) fails.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FormatError {
    /// A number formatted with a date format lies outside the dates Power BI
    /// can show (100-01-01 to 9999-12-31).
    #[error("Input number '{number}' is out of range for format string '{format}'.")]
    OutOfDateRange {
        /// The number, as `General` text.
        number: String,
        /// The format string.
        format: String,
    },
}

/// Formats `value` with a Power BI format string (`None` or empty means
/// `General`) for display (model format strings in visuals). `culture` is
/// reserved (only `en-US` is implemented). Never fails or panics: a number
/// out of the date range shows as `General`, and characters of an unknown
/// custom format that are not placeholders are shown literally around the
/// value, as in Power BI. Thousands-scaling commas after the decimal
/// placeholders (`#,0.0,,"M"`) scale, as in Excel.
#[must_use]
pub fn format_value(value: &Value, format_string: Option<&str>, culture: &str) -> String {
    let _ = culture;
    render(value, format_string, Dialect::Display).unwrap_or_else(|_| general(value))
}

/// Formats `value` like the DAX `FORMAT` function in Power BI Desktop's
/// engine (recorded in `reference/groundtruth`): as [`format_value`], except
/// that a number out of the date range formatted with a date format is an
/// error and commas after the decimal placeholders do not scale
/// (`FORMAT(1234, "#,0.0,,""M""")` is `1,234.0M`).
///
/// # Errors
///
/// [`FormatError::OutOfDateRange`].
pub fn try_format_value(
    value: &Value,
    format_string: Option<&str>,
    culture: &str,
) -> Result<String, FormatError> {
    let _ = culture;
    render(value, format_string, Dialect::Engine)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dialect {
    Display,
    Engine,
}

/// Shared by both dialects. Text is returned unchanged under every format;
/// booleans are `-1`/`0` under numeric and date formats (`True`/`False`
/// under `General`, the Yes/No-style formats and text formats); a format of
/// only text placeholders (`@`, `&`, `<`, `>`, `!`) shows the `General` text,
/// in upper case with `>` and lower case with `<`.
fn render(
    value: &Value,
    format_string: Option<&str>,
    dialect: Dialect,
) -> Result<String, FormatError> {
    match value {
        Value::Blank => return Ok(String::new()),
        Value::String(s) => return Ok(s.to_string()),
        _ => {}
    }
    let Some(fmt) = format_string.map(str::trim).filter(|f| !f.is_empty()) else {
        return Ok(general(value));
    };
    if fmt
        .chars()
        .all(|c| matches!(c, '@' | '&' | '<' | '>' | '!'))
    {
        let text = general(value);
        return Ok(if fmt.contains('>') {
            text.to_uppercase()
        } else if fmt.contains('<') {
            text.to_lowercase()
        } else {
            text
        });
    }
    let named = Named::parse(fmt);
    if let Value::Boolean(b) = value {
        match named {
            Some(Named::General | Named::YesNo | Named::TrueFalse | Named::OnOff) => {}
            _ => return render(&Value::Integer(-i64::from(*b)), Some(fmt), dialect),
        }
    }
    if let Some(named) = named {
        return named.apply(value, fmt);
    }
    let engine_pattern;
    let pattern = if dialect == Dialect::Engine {
        engine_pattern = number::without_fraction_scaling(fmt);
        engine_pattern.as_str()
    } else {
        fmt
    };
    Ok(match value {
        Value::DateTime(us) => {
            if number::is_numeric_format(pattern) {
                number::format_number(&serial_digits(value), pattern)
            } else {
                datetime::format_datetime(*us, pattern)
            }
        }
        Value::Integer(_) | Value::Double(_) | Value::Decimal(_) => {
            if !number::is_numeric_format(pattern) && datetime::is_date_format(pattern) {
                as_date(value, pattern, fmt)?
            } else {
                as_number(value, pattern)
            }
        }
        other => general(other),
    })
}

fn bool_text(b: bool) -> &'static str {
    if b { "True" } else { "False" }
}

/// A number formatted with a numeric pattern (non-finite values as text).
fn as_number(value: &Value, pattern: &str) -> String {
    match Digits::from_value(value) {
        Some(d) => number::format_number(&d, pattern),
        None => general(value),
    }
}

/// A number read as a serial date and formatted with a date pattern
/// (`fmt` is the format string as written, for the error).
fn as_date(value: &Value, pattern: &str, fmt: &str) -> Result<String, FormatError> {
    serial_micros(value)
        .map(|us| datetime::format_datetime(us, pattern))
        .ok_or_else(|| FormatError::OutOfDateRange {
            number: general(value),
            format: fmt.to_owned(),
        })
}

/// The date/time of a serial number (days since 1899-12-30), as OLE
/// Automation reads it: a negative serial's fraction is still the time of
/// day (`-1.25` is 1899-12-29 06:00), rounded to the second. `None` outside
/// 100-01-01 to 9999-12-31.
fn serial_micros(value: &Value) -> Option<i64> {
    const MIN_SERIAL: f64 = -657_434.0;
    const END_SERIAL: f64 = 2_958_466.0;
    const SECONDS_PER_DAY: f64 = 86_400.0;
    let serial = value
        .as_f64()
        .filter(|s| (MIN_SERIAL..END_SERIAL).contains(s))?;
    let linear = if serial < 0.0 {
        let days = serial.trunc();
        days - (serial - days)
    } else {
        serial
    };
    let linear = (linear * SECONDS_PER_DAY).round() / SECONDS_PER_DAY;
    match Value::from_oadate(linear)? {
        Value::DateTime(us) => Some(us),
        _ => None,
    }
}

/// The serial number of a date (days since 1899-12-30).
fn serial_digits(value: &Value) -> Digits {
    value
        .as_f64()
        .and_then(Digits::from_f64)
        .unwrap_or_else(Digits::zero)
}

/// The `General` format: numbers with up to 15 significant digits and no
/// grouping, dates as `General Date`, booleans `True`/`False`.
fn general(value: &Value) -> String {
    match value {
        Value::Blank => String::new(),
        Value::Boolean(b) => bool_text(*b).to_owned(),
        Value::Integer(i) => i.to_string(),
        Value::Decimal(_) => value.to_string(),
        Value::Double(d) => number::general_double(*d),
        Value::String(s) => s.to_string(),
        Value::DateTime(us) => datetime::general_date(*us),
    }
}

/// Predefined (named) formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Named {
    General,
    GeneralNumber,
    Currency,
    Fixed,
    Standard,
    Percent,
    Scientific,
    YesNo,
    TrueFalse,
    OnOff,
    GeneralDate,
    LongDate,
    MediumDate,
    ShortDate,
    LongTime,
    MediumTime,
    ShortTime,
}

impl Named {
    fn parse(fmt: &str) -> Option<Named> {
        const NAMES: [(&str, Named); 17] = [
            ("general", Named::General),
            ("general number", Named::GeneralNumber),
            ("currency", Named::Currency),
            ("fixed", Named::Fixed),
            ("standard", Named::Standard),
            ("percent", Named::Percent),
            ("scientific", Named::Scientific),
            ("yes/no", Named::YesNo),
            ("true/false", Named::TrueFalse),
            ("on/off", Named::OnOff),
            ("general date", Named::GeneralDate),
            ("long date", Named::LongDate),
            ("medium date", Named::MediumDate),
            ("short date", Named::ShortDate),
            ("long time", Named::LongTime),
            ("medium time", Named::MediumTime),
            ("short time", Named::ShortTime),
        ];
        NAMES
            .iter()
            .find(|(name, _)| fmt.eq_ignore_ascii_case(name))
            .map(|(_, n)| *n)
    }

    /// Custom equivalent of a numeric named format (en-US).
    fn numeric_pattern(self) -> Option<&'static str> {
        Some(match self {
            Named::Currency => "\\$#,0.00;(\\$#,0.00)",
            Named::Fixed => "0.00",
            Named::Standard => "#,0.00",
            Named::Percent => "0.00%",
            Named::Scientific => "0.00E+00",
            _ => return None,
        })
    }

    /// Custom equivalent of a date/time named format (en-US).
    fn date_pattern(self) -> Option<&'static str> {
        Some(match self {
            Named::LongDate => "dddd, mmmm d, yyyy",
            Named::MediumDate => "dd-mmm-yy",
            Named::ShortDate => "m/d/yyyy",
            Named::LongTime => "h:nn:ss AM/PM",
            Named::MediumTime => "hh:nn AM/PM",
            Named::ShortTime => "hh:nn",
            _ => return None,
        })
    }

    /// Text and BLANK never get here; booleans only for `General` and the
    /// Yes/No-style formats.
    fn apply(self, value: &Value, fmt: &str) -> Result<String, FormatError> {
        let truthy = |yes: &str, no: &str| -> String {
            let is_true = match value {
                Value::Boolean(b) => *b,
                other => other.as_f64().is_some_and(|f| f != 0.0),
            };
            if is_true { yes } else { no }.to_owned()
        };
        let is_number = matches!(
            value,
            Value::Integer(_) | Value::Double(_) | Value::Decimal(_)
        );
        Ok(match self {
            Named::General => general(value),
            Named::GeneralNumber => match value {
                Value::DateTime(_) => number::general_double(value.as_f64().unwrap_or(0.0)),
                other => general(other),
            },
            Named::YesNo => truthy("Yes", "No"),
            Named::TrueFalse => truthy("True", "False"),
            Named::OnOff => truthy("On", "Off"),
            Named::GeneralDate => match value {
                Value::DateTime(us) => datetime::general_date(*us),
                _ if is_number => match serial_micros(value) {
                    Some(us) => datetime::general_date(us),
                    None => as_date(value, "c", fmt)?,
                },
                other => general(other),
            },
            _ => {
                if let Some(pattern) = self.numeric_pattern() {
                    return Ok(match value {
                        Value::DateTime(_) => number::format_number(&serial_digits(value), pattern),
                        _ if is_number => as_number(value, pattern),
                        other => general(other),
                    });
                }
                let pattern = self.date_pattern().unwrap_or("c");
                match value {
                    Value::DateTime(us) => datetime::format_datetime(*us, pattern),
                    _ if is_number => as_date(value, pattern, fmt)?,
                    other => general(other),
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(value: Value, fmt: &str) -> String {
        format_value(&value, Some(fmt), "en-US")
    }

    fn dt(y: i64, m: u32, d: u32, h: u32, n: u32, s: u32) -> Value {
        Value::datetime(y, m, d, h, n, s).unwrap_or_default()
    }

    #[test]
    fn blank_is_empty_and_missing_format_is_general() {
        assert_eq!(format_value(&Value::Blank, Some("0"), "en-US"), "");
        assert_eq!(format_value(&Value::Blank, None, "en-US"), "");
        assert_eq!(format_value(&Value::Integer(42), None, "en-US"), "42");
        assert_eq!(format_value(&Value::Double(0.5), Some(""), "en-US"), "0.5");
        assert_eq!(format_value(&Value::from("x"), Some("0"), "en-US"), "x");
        assert_eq!(format_value(&Value::Boolean(true), None, "en-US"), "True");
        assert_eq!(format_value(&Value::Decimal(12_500), None, "en-US"), "1.25");
    }

    #[test]
    fn kpi_formats() {
        assert_eq!(f(Value::Integer(2024), "#,0"), "2,024");
        assert_eq!(f(Value::Double(815.0 / 1930.0), "0.0%;-0.0%;0.0%"), "42.2%");
        assert_eq!(f(Value::Double(2.0 / 7.0), "0.0%;-0.0%;0.0%"), "28.6%");
        assert_eq!(f(Value::Double(4.0 / 3.0), "+0.0%;-0.0%;0.0%"), "+133.3%");
        assert_eq!(f(Value::Integer(0), "0.0%;-0.0%;0.0%"), "0.0%");
        assert_eq!(f(Value::Double(-0.25), "0.0%;-0.0%;0.0%"), "-25.0%");
        assert_eq!(
            f(dt(2026, 10, 21, 14, 5, 59), "yyyy-mm-dd hh:nn"),
            "2026-10-21 14:05"
        );
    }

    #[test]
    fn named_numeric_formats() {
        let v = Value::Double(12345.67);
        assert_eq!(f(v.clone(), "General Number"), "12345.67");
        assert_eq!(f(v.clone(), "General"), "12345.67");
        assert_eq!(f(v.clone(), "Currency"), "$12,345.67");
        assert_eq!(f(Value::Double(-12345.67), "Currency"), "($12,345.67)");
        assert_eq!(f(v.clone(), "Fixed"), "12345.67");
        assert_eq!(f(v.clone(), "Standard"), "12,345.67");
        assert_eq!(f(v.clone(), "Percent"), "1234567.00%");
        assert_eq!(f(v, "Scientific"), "1.23E+04");
        assert_eq!(f(Value::Integer(0), "Yes/No"), "No");
        assert_eq!(f(Value::Integer(3), "yes/no"), "Yes");
        assert_eq!(f(Value::Boolean(false), "True/False"), "False");
        assert_eq!(f(Value::Integer(1), "On/Off"), "On");
    }

    #[test]
    fn named_date_formats() {
        let v = dt(2008, 3, 12, 11, 7, 31);
        assert_eq!(f(v.clone(), "General Date"), "3/12/2008 11:07:31 AM");
        assert_eq!(f(v.clone(), "Long Date"), "Wednesday, March 12, 2008");
        assert_eq!(f(v.clone(), "Medium Date"), "12-Mar-08");
        assert_eq!(f(v.clone(), "Short Date"), "3/12/2008");
        assert_eq!(f(v.clone(), "Long Time"), "11:07:31 AM");
        assert_eq!(f(v.clone(), "Medium Time"), "11:07 AM");
        assert_eq!(f(v, "Short Time"), "11:07");
        assert_eq!(f(dt(2008, 3, 12, 0, 0, 0), "General Date"), "3/12/2008");
        assert_eq!(
            format_value(&dt(2020, 12, 15, 12, 30, 59), None, "en-US"),
            "12/15/2020 12:30:59 PM"
        );
    }

    #[test]
    fn desktop_format_rules() {
        let e = |v: Value, fmt: &str| try_format_value(&v, Some(fmt), "en-US");
        // Booleans are -1/0 under numeric and date formats.
        assert_eq!(f(Value::Boolean(true), "General Number"), "-1");
        assert_eq!(f(Value::Boolean(true), "Percent"), "-100.00%");
        assert_eq!(f(Value::Boolean(true), "Short Date"), "12/29/1899");
        assert_eq!(f(Value::Boolean(true), "0.0"), "-1.0");
        assert_eq!(f(Value::Boolean(true), "Yes/No"), "Yes");
        // Text is unchanged, also under Yes/No.
        assert_eq!(f(Value::from("abc"), "Yes/No"), "abc");
        // Text-only formats show the General text.
        assert_eq!(f(Value::Integer(1234), "@"), "1234");
        assert_eq!(f(Value::Boolean(true), ">"), "TRUE");
        assert_eq!(f(dt(2025, 3, 7, 0, 0, 0), "@"), "3/7/2025");
        // Negative serials keep a positive time of day (OLE Automation).
        assert_eq!(
            f(Value::Double(-1234.5678), "General Date"),
            "8/13/1896 1:37:38 PM"
        );
        assert_eq!(f(Value::Integer(0), "General Date"), "12:00:00 AM");
        // Out of the date range: an error in FORMAT, General for display.
        assert_eq!(
            e(Value::Double(1_234_567_890.5), "yyyy")
                .unwrap_err()
                .to_string(),
            "Input number '1234567890.5' is out of range for format string 'yyyy'."
        );
        assert_eq!(f(Value::Double(1_234_567_890.5), "yyyy"), "1234567890.5");
        // Commas after the decimal point scale only for display.
        assert_eq!(
            e(Value::Integer(1234), "#,0.0,,\"M\"").as_deref(),
            Ok("1,234.0M")
        );
        assert_eq!(f(Value::Integer(1_500_000), "#,0.0,,\"M\""), "1.5M");
    }

    #[test]
    fn numbers_as_dates_and_dates_as_numbers() {
        assert_eq!(f(Value::Integer(45292), "yyyy-mm-dd"), "2024-01-01");
        assert_eq!(f(Value::Double(45292.5), "Short Date"), "1/1/2024");
        assert_eq!(f(dt(2024, 1, 1, 12, 0, 0), "0.00"), "45292.50");
        assert_eq!(f(dt(2024, 1, 1, 0, 0, 0), "General Number"), "45292");
    }

    #[test]
    fn unknown_formats_never_panic() {
        for fmt in [
            "\\",
            "\"",
            "E+",
            "0E+",
            "%%%",
            ";;;;",
            "#,##0,,,,,,,,",
            "dddddddd",
            "a/",
            "AM/",
            "\u{1F600}",
            "0.0.0",
            ",",
            "00000000000000000000000000.000000000000000000000",
            "0E+00000000000000000000000",
        ] {
            for v in [
                Value::Integer(i64::MIN),
                Value::Double(f64::NAN),
                Value::Double(f64::INFINITY),
                Value::Double(-1e300),
                Value::Double(5e-324),
                Value::Decimal(i128::MAX),
                Value::Decimal(i128::MIN),
                Value::DateTime(i64::MAX),
                Value::DateTime(i64::MIN),
            ] {
                let _ = format_value(&v, Some(fmt), "en-US");
            }
        }
    }
}
