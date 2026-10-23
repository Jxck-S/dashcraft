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
//! 1899-12-30) and dates formatted with a numeric format as serial numbers,
//! as in Power BI. Text is returned unchanged; BLANK formats as the empty
//! string. Rounding is half away from zero on the value's 15 significant
//! digits (as Excel and Power BI display numbers), so `0.125` with `0.00`
//! is `0.13`.
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

/// Formats `value` with a Power BI format string (`None` or empty means
/// `General`). `culture` is reserved (only `en-US` is implemented). Never
/// fails or panics: characters of an unknown custom format that are not
/// placeholders are shown literally around the value, as in Power BI.
#[must_use]
pub fn format_value(value: &Value, format_string: Option<&str>, culture: &str) -> String {
    let _ = culture;
    if value.is_blank() {
        return String::new();
    }
    let Some(fmt) = format_string.map(str::trim).filter(|f| !f.is_empty()) else {
        return general(value);
    };
    if let Some(named) = Named::parse(fmt) {
        return named.apply(value);
    }
    match value {
        Value::Blank => String::new(),
        Value::String(s) => s.to_string(),
        Value::Boolean(b) => bool_text(*b).to_owned(),
        Value::DateTime(us) => {
            if number::is_numeric_format(fmt) {
                number::format_number(&serial_digits(value), fmt)
            } else {
                datetime::format_datetime(*us, fmt)
            }
        }
        Value::Integer(_) | Value::Double(_) | Value::Decimal(_) => {
            if !number::is_numeric_format(fmt) && datetime::is_date_format(fmt) {
                as_date(value, fmt)
            } else {
                as_number(value, fmt)
            }
        }
    }
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

/// A number read as a serial date and formatted with a date pattern.
fn as_date(value: &Value, pattern: &str) -> String {
    match value.as_f64().and_then(Value::from_oadate) {
        Some(Value::DateTime(us)) => datetime::format_datetime(us, pattern),
        _ => general(value),
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
            ("medium date", Named::LongDate),
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
            Named::Percent => "#,0.00 %",
            Named::Scientific => "0.00E+00",
            _ => return None,
        })
    }

    /// Custom equivalent of a date/time named format (en-US).
    fn date_pattern(self) -> Option<&'static str> {
        Some(match self {
            Named::LongDate => "dddd, mmmm d, yyyy",
            Named::ShortDate => "m/d/yyyy",
            Named::LongTime => "h:nn:ss AM/PM",
            Named::MediumTime => "hh:nn AM/PM",
            Named::ShortTime => "hh:nn",
            _ => return None,
        })
    }

    fn apply(self, value: &Value) -> String {
        let truthy = |yes: &str, no: &str| -> String {
            let is_true = match value {
                Value::Boolean(b) => *b,
                Value::String(s) => !s.is_empty(),
                other => other.as_f64().is_some_and(|f| f != 0.0),
            };
            if is_true { yes } else { no }.to_owned()
        };
        let is_number = matches!(
            value,
            Value::Integer(_) | Value::Double(_) | Value::Decimal(_)
        );
        match self {
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
                _ if is_number => match value.as_f64().and_then(Value::from_oadate) {
                    Some(Value::DateTime(us)) => datetime::general_date(us),
                    _ => general(value),
                },
                other => general(other),
            },
            _ => {
                if let Some(pattern) = self.numeric_pattern() {
                    return match value {
                        Value::DateTime(_) => number::format_number(&serial_digits(value), pattern),
                        _ if is_number => as_number(value, pattern),
                        other => general(other),
                    };
                }
                let pattern = self.date_pattern().unwrap_or("c");
                match value {
                    Value::DateTime(us) => datetime::format_datetime(*us, pattern),
                    _ if is_number => as_date(value, pattern),
                    other => general(other),
                }
            }
        }
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
        assert_eq!(f(v.clone(), "Percent"), "1,234,567.00 %");
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
        assert_eq!(f(v.clone(), "Medium Date"), "Wednesday, March 12, 2008");
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
