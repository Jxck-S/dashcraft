//! Number formatting for axes and data labels: display units (`Auto`, K,
//! M, bn, T), tick precision from the tick step, the measure's format
//! string (percent, currency prefix) through [`pbi_dax::format_value`].

use pbi_engine::Value;

/// A display unit (`labelDisplayUnits`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplayUnit {
    pub divisor: f64,
    pub suffix: &'static str,
}

impl DisplayUnit {
    pub const NONE: DisplayUnit = DisplayUnit {
        divisor: 1.0,
        suffix: "",
    };

    /// The unit Power BI's `Auto` picks for values up to `max_abs`.
    pub fn auto(max_abs: f64) -> Self {
        let m = if max_abs.is_finite() {
            max_abs.abs()
        } else {
            0.0
        };
        let (divisor, suffix) = if m >= 1e12 {
            (1e12, "T")
        } else if m >= 1e9 {
            (1e9, "bn")
        } else if m >= 1e6 {
            (1e6, "M")
        } else if m >= 1e3 {
            (1e3, "K")
        } else {
            (1.0, "")
        };
        Self { divisor, suffix }
    }

    /// From a `labelDisplayUnits` value: `0` (or missing) is `Auto`
    /// (`None` here), `1` none, `1000` K, `1000000` M, … .
    pub fn from_setting(v: Option<f64>) -> Option<Self> {
        let v = v.filter(|v| v.is_finite() && *v > 0.0)?;
        let unit = |divisor, suffix| Some(Self { divisor, suffix });
        if v >= 1e12 {
            unit(1e12, "T")
        } else if v >= 1e9 {
            unit(1e9, "bn")
        } else if v >= 1e6 {
            unit(1e6, "M")
        } else if v >= 1e3 {
            unit(1e3, "K")
        } else {
            Some(Self::NONE)
        }
    }

    pub fn is_none(&self) -> bool {
        self.divisor == 1.0
    }
}

/// What a format string implies for axis/label numbers.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NumberStyle {
    /// Values are fractions shown as percentages.
    pub percent: bool,
    /// Literal prefix (currency symbol).
    pub prefix: String,
}

impl NumberStyle {
    /// Reads the first section of a Power BI format string.
    pub fn from_format(format: Option<&str>) -> Self {
        let Some(fmt) = format.map(str::trim).filter(|f| !f.is_empty()) else {
            return Self::default();
        };
        let lower = fmt.to_ascii_lowercase();
        if lower == "percent" {
            return Self {
                percent: true,
                prefix: String::new(),
            };
        }
        if lower == "currency" {
            return Self {
                percent: false,
                prefix: "$".to_owned(),
            };
        }
        let section = first_section(fmt);
        let mut percent = false;
        let mut prefix = String::new();
        let mut seen_digit = false;
        let mut chars = section.chars().peekable();
        let mut in_quotes = false;
        while let Some(c) = chars.next() {
            match c {
                '"' => in_quotes = !in_quotes,
                _ if in_quotes => {
                    if !seen_digit {
                        prefix.push(c);
                    }
                }
                '\\' => {
                    if let Some(n) = chars.next()
                        && !seen_digit
                    {
                        prefix.push(n);
                    }
                }
                '%' => percent = true,
                '0' | '#' => seen_digit = true,
                '$' | '€' | '£' | '¥' if !seen_digit => prefix.push(c),
                _ => {}
            }
        }
        let prefix = if prefix.chars().count() <= 4 {
            prefix.trim().to_owned()
        } else {
            String::new()
        };
        Self { percent, prefix }
    }
}

/// First `;` section of a format string (quotes and escapes respected).
fn first_section(fmt: &str) -> &str {
    let mut in_quotes = false;
    let mut escaped = false;
    for (i, c) in fmt.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '"' => in_quotes = !in_quotes,
            ';' if !in_quotes => return fmt.get(..i).unwrap_or(fmt),
            _ => {}
        }
    }
    fmt
}

/// Smallest number of decimals that shows `step` exactly (capped at 6).
pub fn decimals_for_step(step: f64) -> usize {
    if !step.is_finite() || step <= 0.0 {
        return 0;
    }
    for d in 0..=6 {
        let scaled = step * 10f64.powi(d);
        if (scaled - scaled.round()).abs() < 1e-6 * scaled.abs().max(1.0) {
            return d as usize;
        }
    }
    6
}

/// `v` with `decimals` decimals and thousands separators (`-` sign).
pub fn group_number(v: f64, decimals: usize) -> String {
    if !v.is_finite() {
        return String::new();
    }
    let fmt = if decimals == 0 {
        "#,0".to_owned()
    } else {
        format!("#,0.{}", "0".repeat(decimals.min(10)))
    };
    pbi_dax::format_value(&Value::Double(v), Some(&fmt), "en-US")
}

/// A value-axis tick label: display unit (auto from the largest tick when
/// `unit` is `None`), precision from the step, percent and currency from
/// the measure's format string.
pub fn tick_label(
    v: f64,
    step: f64,
    max_abs: f64,
    style: &NumberStyle,
    unit: Option<DisplayUnit>,
) -> String {
    if !v.is_finite() {
        return String::new();
    }
    if style.percent {
        let decimals = decimals_for_step(step * 100.0);
        return format!("{}%", group_number(v * 100.0, decimals));
    }
    let unit = unit.unwrap_or_else(|| DisplayUnit::auto(max_abs));
    let decimals = decimals_for_step(step / unit.divisor);
    let body = group_number(v / unit.divisor, decimals);
    with_prefix(&style.prefix, &body, unit.suffix)
}

fn with_prefix(prefix: &str, body: &str, suffix: &str) -> String {
    match body.strip_prefix('-') {
        Some(rest) if !prefix.is_empty() => format!("-{prefix}{rest}{suffix}"),
        _ => format!("{prefix}{body}{suffix}"),
    }
}

/// A data label: the DataView's formatted text unless a display unit
/// applies (`unit`, or `Auto` from `max_abs` ≥ 1000), then the scaled value
/// with `precision` decimals (default: 3 significant digits, trailing
/// zeros removed).
pub fn data_label(
    v: f64,
    formatted: &str,
    max_abs: f64,
    style: &NumberStyle,
    unit: Option<DisplayUnit>,
    precision: Option<usize>,
) -> String {
    if !v.is_finite() {
        return formatted.to_owned();
    }
    let unit = unit.unwrap_or_else(|| DisplayUnit::auto(max_abs));
    if style.percent || (unit.is_none() && precision.is_none()) {
        if !formatted.is_empty() {
            return formatted.to_owned();
        }
        if style.percent {
            return format!("{}%", group_number(v * 100.0, precision.unwrap_or(0)));
        }
    }
    let scaled = v / unit.divisor;
    let (decimals, trim) = match precision {
        Some(p) => (p.min(10), false),
        None => {
            let a = scaled.abs();
            let d = if (unit.is_none() && a.fract() == 0.0) || a >= 100.0 {
                0
            } else if a >= 10.0 {
                1
            } else {
                2
            };
            (d, true)
        }
    };
    let mut body = group_number(scaled, decimals);
    if trim && body.contains('.') {
        while body.ends_with('0') {
            body.pop();
        }
        if body.ends_with('.') {
            body.pop();
        }
    }
    with_prefix(&style.prefix, &body, unit.suffix)
}

/// `share` (0–1) as a percentage with `decimals` decimals (`75.20%`).
pub fn percent(share: f64, decimals: usize) -> String {
    if !share.is_finite() {
        return String::new();
    }
    format!("{}%", group_number(share * 100.0, decimals))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_from_format_strings() {
        assert!(NumberStyle::from_format(Some("0.0%;-0.0%;0.0%")).percent);
        assert!(!NumberStyle::from_format(Some("#,0")).percent);
        assert_eq!(NumberStyle::from_format(Some("\\$#,0.00")).prefix, "$");
        assert_eq!(NumberStyle::from_format(Some("$#,0")).prefix, "$");
        assert_eq!(NumberStyle::from_format(Some("Currency")).prefix, "$");
        assert_eq!(NumberStyle::from_format(None), NumberStyle::default());
    }

    #[test]
    fn data_labels() {
        let s = NumberStyle::default();
        assert_eq!(data_label(678.0, "678", 678.0, &s, None, None), "678");
        assert_eq!(data_label(1234.0, "1234", 5000.0, &s, None, None), "1.23K");
        assert_eq!(data_label(-25_000.0, "", 25_000.0, &s, None, None), "-25K");
        let usd = NumberStyle::from_format(Some("$#,0"));
        assert_eq!(data_label(-1500.0, "", 1500.0, &usd, None, None), "-$1.5K");
        assert_eq!(percent(0.752_04, 2), "75.20%");
    }
}
