//! Custom numeric format strings (`0`, `#`, `.`, `,`, `%`, `E+`, literals,
//! up to three `;` sections).
//!
//! Numbers are formatted from their decimal digits ([`Digits`]): integers
//! and fixed decimals exactly, doubles from their 15 significant digits.
//! Rounding is half away from zero.

use pbi_engine::Value;

/// A finite decimal number `±0.d₁d₂…dₙ × 10^point` with no leading or
/// trailing zero digits (zero has no digits).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digits {
    neg: bool,
    digits: Vec<u8>,
    point: i64,
}

impl Digits {
    /// Zero.
    #[must_use]
    pub fn zero() -> Self {
        Self {
            neg: false,
            digits: Vec::new(),
            point: 0,
        }
    }

    /// Digits of a numeric value (`Integer`, `Double`, `Decimal`; dates as
    /// serial numbers). `None` for BLANK, text, booleans and non-finite
    /// doubles.
    #[must_use]
    pub fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Integer(i) => Some(Self::from_u128(*i < 0, u128::from(i.unsigned_abs()), 0)),
            Value::Decimal(d) => Some(Self::from_u128(*d < 0, d.unsigned_abs(), 4)),
            Value::Double(f) => Self::from_f64(*f),
            Value::DateTime(_) => value.as_f64().and_then(Self::from_f64),
            Value::Blank | Value::Boolean(_) | Value::String(_) => None,
        }
    }

    /// Digits of a double, from its 15 significant digits. `None` if not
    /// finite.
    #[must_use]
    pub fn from_f64(value: f64) -> Option<Self> {
        if !value.is_finite() {
            return None;
        }
        if value == 0.0 {
            return Some(Self::zero());
        }
        let sci = format!("{:.14e}", value.abs());
        let (mantissa, exp) = sci.split_once('e')?;
        let exp: i64 = exp.parse().ok()?;
        let mut d = Self {
            neg: value < 0.0,
            digits: mantissa
                .bytes()
                .filter(u8::is_ascii_digit)
                .map(|b| b - b'0')
                .collect(),
            point: exp + 1,
        };
        d.normalize();
        Some(d)
    }

    /// `abs / 10^scale`.
    fn from_u128(neg: bool, abs: u128, scale: i64) -> Self {
        let text = abs.to_string();
        let mut d = Self {
            neg,
            digits: text.bytes().map(|b| b - b'0').collect(),
            point: i64::try_from(text.len()).unwrap_or(0) - scale,
        };
        d.normalize();
        d
    }

    fn normalize(&mut self) {
        let lead = self.digits.iter().take_while(|&&d| d == 0).count();
        if lead > 0 {
            self.digits.drain(..lead);
            self.point -= i64::try_from(lead).unwrap_or(0);
        }
        while self.digits.last() == Some(&0) {
            self.digits.pop();
        }
        if self.digits.is_empty() {
            *self = Self::zero();
        }
    }

    /// `true` for zero.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.digits.is_empty()
    }

    /// `true` for values below zero.
    #[must_use]
    pub fn is_negative(&self) -> bool {
        self.neg && !self.is_zero()
    }

    fn abs(&self) -> Self {
        Self {
            neg: false,
            ..self.clone()
        }
    }

    /// Multiplies by `10^by`.
    fn shift(&mut self, by: i64) {
        if !self.is_zero() {
            self.point = self.point.saturating_add(by);
        }
    }

    /// Rounds half away from zero to `decimals` fractional digits.
    fn round(&mut self, decimals: i64) {
        if self.is_zero() {
            return;
        }
        let keep = self.point.saturating_add(decimals);
        if keep < 0 {
            *self = Self::zero();
            return;
        }
        let Ok(keep) = usize::try_from(keep) else {
            return;
        };
        if keep >= self.digits.len() {
            return;
        }
        let up = self.digits.get(keep).is_some_and(|&d| d >= 5);
        self.digits.truncate(keep);
        if up {
            let mut i = keep;
            loop {
                if i == 0 {
                    self.digits.insert(0, 1);
                    self.point += 1;
                    break;
                }
                i -= 1;
                match self.digits.get_mut(i) {
                    Some(d) if *d == 9 => *d = 0,
                    Some(d) => {
                        *d += 1;
                        break;
                    }
                    None => break,
                }
            }
        }
        self.normalize();
    }

    /// Digit at position `index` from the most significant one (0 past the
    /// end).
    fn digit(&self, index: i64) -> u8 {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.digits.get(i))
            .copied()
            .unwrap_or(0)
    }

    /// Integer part (`""` below 1).
    fn int_text(&self) -> String {
        (0..self.point.max(0))
            .map(|i| char::from(b'0' + self.digit(i)))
            .collect()
    }

    /// The first `n` fractional digits.
    fn frac_digits(&self, n: usize) -> Vec<u8> {
        (0..n)
            .map(|k| self.digit(self.point.saturating_add(i64::try_from(k).unwrap_or(0))))
            .collect()
    }
}

/// Splits a format string on `;` outside quotes and escapes.
pub(crate) fn split_sections(fmt: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (i, c) in fmt.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if !quoted => escaped = true,
            '"' => quoted = !quoted,
            ';' if !quoted => {
                out.push(fmt.get(start..i).unwrap_or_default());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(fmt.get(start..).unwrap_or_default());
    out
}

/// Unquoted, unescaped characters of a format string.
pub(crate) fn bare_chars(fmt: &str) -> impl Iterator<Item = char> + '_ {
    let mut quoted = false;
    let mut escaped = false;
    fmt.chars().filter(move |&c| {
        if escaped {
            escaped = false;
            return false;
        }
        match c {
            '\\' if !quoted => {
                escaped = true;
                false
            }
            '"' => {
                quoted = !quoted;
                false
            }
            _ => !quoted,
        }
    })
}

/// `true` if the format has digit placeholders (`0`, `#`).
#[must_use]
pub fn is_numeric_format(fmt: &str) -> bool {
    bare_chars(fmt).any(|c| matches!(c, '0' | '#'))
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    /// Digit placeholder; `true` for `0`.
    Digit(bool),
    Point,
    Comma,
    Percent,
    Exp {
        upper: bool,
        plus: bool,
    },
    Lit(String),
}

fn tokenize(section: &str) -> Vec<Tok> {
    let chars: Vec<char> = section.chars().collect();
    let mut toks = Vec::new();
    let (mut seen_point, mut seen_exp) = (false, false);
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        i += 1;
        match c {
            '0' => toks.push(Tok::Digit(true)),
            '#' | '?' => toks.push(Tok::Digit(false)),
            '.' if !seen_point && !seen_exp => {
                seen_point = true;
                toks.push(Tok::Point);
            }
            ',' if !seen_exp => toks.push(Tok::Comma),
            '%' => toks.push(Tok::Percent),
            'E' | 'e'
                if !seen_exp
                    && matches!(chars.get(i), Some('+' | '-'))
                    && matches!(chars.get(i + 1), Some('0' | '#')) =>
            {
                seen_exp = true;
                toks.push(Tok::Exp {
                    upper: c == 'E',
                    plus: chars.get(i) == Some(&'+'),
                });
                i += 1;
            }
            '\\' => {
                if let Some(&n) = chars.get(i) {
                    toks.push(Tok::Lit(n.to_string()));
                    i += 1;
                }
            }
            '"' => {
                let mut lit = String::new();
                while let Some(&n) = chars.get(i) {
                    i += 1;
                    if n == '"' {
                        break;
                    }
                    lit.push(n);
                }
                toks.push(Tok::Lit(lit));
            }
            // Excel: `_x` is a space as wide as `x`; `*x` repeats `x` to fill.
            '_' => {
                toks.push(Tok::Lit(" ".into()));
                i += 1;
            }
            '*' => i += 1,
            other => toks.push(Tok::Lit(other.to_string())),
        }
    }
    toks
}

/// Placeholder layout of one section.
#[derive(Debug, Default)]
struct Layout {
    int_count: usize,
    min_int: usize,
    frac_count: usize,
    min_frac: usize,
    exp_min: usize,
    grouping: bool,
    thousands: i64,
    percent: i64,
    scientific: bool,
}

fn layout(toks: &[Tok]) -> Layout {
    #[derive(PartialEq)]
    enum Phase {
        Int,
        Frac,
        Exp,
    }
    let mut l = Layout::default();
    let mut phase = Phase::Int;
    let mut first_zero: Option<usize> = None;
    for (i, t) in toks.iter().enumerate() {
        match t {
            Tok::Digit(zero) => match phase {
                Phase::Int => {
                    if *zero && first_zero.is_none() {
                        first_zero = Some(l.int_count);
                    }
                    l.int_count += 1;
                }
                Phase::Frac => {
                    l.frac_count += 1;
                    if *zero {
                        l.min_frac = l.frac_count;
                    }
                }
                Phase::Exp => {
                    if *zero {
                        l.exp_min += 1;
                    }
                }
            },
            Tok::Point => phase = Phase::Frac,
            Tok::Exp { .. } => {
                phase = Phase::Exp;
                l.scientific = true;
            }
            Tok::Percent => l.percent += 1,
            Tok::Comma if phase == Phase::Int => {
                let prev_digit = toks
                    .get(..i)
                    .is_some_and(|p| p.iter().any(|t| matches!(t, Tok::Digit(_))));
                if !prev_digit || matches!(toks.get(i.wrapping_sub(1)), Some(Tok::Comma)) {
                    continue;
                }
                let run = toks
                    .get(i..)
                    .map_or(0, |r| r.iter().take_while(|t| **t == Tok::Comma).count());
                let next = toks.get(i + run);
                let more_int_digits = toks.get(i + run..).is_some_and(|r| {
                    r.iter()
                        .take_while(|t| !matches!(t, Tok::Point | Tok::Exp { .. }))
                        .any(|t| matches!(t, Tok::Digit(_)))
                });
                if matches!(next, Some(Tok::Digit(_))) {
                    l.grouping = true;
                } else if matches!(next, Some(Tok::Point)) || !more_int_digits {
                    l.thousands += i64::try_from(run).unwrap_or(0);
                } else {
                    l.grouping = true;
                }
            }
            // Excel: commas after the last fractional placeholder scale too.
            Tok::Comma if phase == Phase::Frac => {
                if matches!(toks.get(i.wrapping_sub(1)), Some(Tok::Comma)) {
                    continue;
                }
                let more_frac_digits = toks.get(i..).is_some_and(|r| {
                    r.iter()
                        .take_while(|t| !matches!(t, Tok::Exp { .. }))
                        .any(|t| matches!(t, Tok::Digit(_)))
                });
                if !more_frac_digits {
                    let run = toks
                        .get(i..)
                        .map_or(0, |r| r.iter().take_while(|t| **t == Tok::Comma).count());
                    l.thousands += i64::try_from(run).unwrap_or(0);
                }
            }
            Tok::Comma | Tok::Lit(_) => {}
        }
    }
    l.min_int = first_zero.map_or(0, |z| l.int_count - z);
    if l.scientific && l.exp_min == 0 {
        l.exp_min = 1;
    }
    l
}

/// Renders `value` (non-negative) with one section; also returns whether
/// every displayed digit is zero.
fn render_section(section: &str, value: &Digits) -> (String, bool) {
    let toks = tokenize(section);
    let l = layout(&toks);
    let mut d = value.clone();
    d.shift(2 * l.percent - 3 * l.thousands);
    let mut exponent: i64 = 0;
    if l.scientific {
        if !d.is_zero() {
            let int_places = i64::try_from(l.int_count.max(1)).unwrap_or(1);
            exponent = d.point - int_places;
            d.point = int_places;
            d.round(i64::try_from(l.frac_count).unwrap_or(i64::MAX));
            if d.point > int_places {
                d.point = int_places;
                exponent += 1;
            }
        }
    } else {
        d.round(i64::try_from(l.frac_count).unwrap_or(i64::MAX));
    }
    let mut int_text = d.int_text();
    if int_text.len() < l.min_int {
        int_text = format!("{}{int_text}", "0".repeat(l.min_int - int_text.len()));
    }
    let mut frac = d.frac_digits(l.frac_count);
    while frac.len() > l.min_frac && frac.last() == Some(&0) {
        frac.pop();
    }
    let all_zero = d.is_zero();

    let int_bytes = int_text.as_bytes();
    let len = int_bytes.len();
    let mut out = String::new();
    let push_int_digit = |out: &mut String, j: usize| {
        if let Some(&b) = int_bytes.get(j) {
            out.push(char::from(b));
            if l.grouping && j + 1 < len && (len - 1 - j).is_multiple_of(3) {
                out.push(',');
            }
        }
    };
    let mut int_emitted = l.int_count > 0;
    let (mut int_idx, mut frac_idx) = (0usize, 0usize);
    let mut in_frac = false;
    let mut in_exp = false;
    for t in &toks {
        match t {
            Tok::Digit(_) if in_exp => {}
            Tok::Digit(_) if in_frac => {
                if let Some(&f) = frac.get(frac_idx) {
                    out.push(char::from(b'0' + f));
                }
                frac_idx += 1;
            }
            Tok::Digit(_) => {
                let k = int_idx;
                int_idx += 1;
                let rest = l.int_count - k;
                if k == 0 {
                    for j in 0..len.saturating_sub(rest - 1) {
                        push_int_digit(&mut out, j);
                    }
                } else if let Some(j) = len.checked_sub(rest) {
                    push_int_digit(&mut out, j);
                }
            }
            Tok::Point => {
                in_frac = true;
                if !int_emitted {
                    int_emitted = true;
                    if int_text != "0" {
                        out.push_str(&int_text);
                    }
                }
                if !frac.is_empty() {
                    out.push('.');
                }
            }
            Tok::Comma => {}
            Tok::Percent => out.push('%'),
            Tok::Exp { upper, plus } => {
                in_frac = false;
                in_exp = true;
                out.push(if *upper { 'E' } else { 'e' });
                if exponent < 0 {
                    out.push('-');
                } else if *plus {
                    out.push('+');
                }
                let digits = exponent.unsigned_abs().to_string();
                if digits.len() < l.exp_min {
                    out.push_str(&"0".repeat(l.exp_min - digits.len()));
                }
                out.push_str(&digits);
            }
            Tok::Lit(s) => out.push_str(s),
        }
    }
    (out, all_zero)
}

/// Formats a number with a custom numeric format string.
#[must_use]
pub fn format_number(value: &Digits, fmt: &str) -> String {
    let sections = split_sections(fmt);
    let section = |i: usize| sections.get(i).copied().filter(|s| !s.is_empty());
    let zero = section(2);
    if value.is_zero()
        && let Some(z) = zero
    {
        return render_section(z, value).0;
    }
    let (body, minus) = match (value.is_negative(), section(1)) {
        (true, Some(neg)) => (neg, false),
        (neg, _) => (sections.first().copied().unwrap_or_default(), neg),
    };
    let (text, rounded_to_zero) = render_section(body, &value.abs());
    if rounded_to_zero {
        if let Some(z) = zero {
            return render_section(z, &Digits::zero()).0;
        }
        return text;
    }
    if minus { format!("-{text}") } else { text }
}

/// `General Number` text of a double: up to 15 significant digits, no
/// grouping, scientific notation outside `1E-5 ≤ |x| < 1E+15`.
#[must_use]
pub fn general_double(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    let Some(d) = Digits::from_f64(value) else {
        return value.to_string();
    };
    if d.is_zero() {
        return "0".to_owned();
    }
    let sign = if d.is_negative() { "-" } else { "" };
    let exp = d.point - 1;
    let digits: String = d.digits.iter().map(|&b| char::from(b'0' + b)).collect();
    if !(-5..15).contains(&exp) {
        let (first, rest) = digits.split_at(1);
        let rest = if rest.is_empty() {
            String::new()
        } else {
            format!(".{rest}")
        };
        let esign = if exp < 0 { "-" } else { "+" };
        return format!("{sign}{first}{rest}E{esign}{:02}", exp.unsigned_abs());
    }
    let int = d.int_text();
    let int = if int.is_empty() { "0".to_owned() } else { int };
    let frac_len = usize::try_from(i64::try_from(digits.len()).unwrap_or(0) - d.point).unwrap_or(0);
    let frac: String = d
        .frac_digits(frac_len)
        .into_iter()
        .map(|b| char::from(b'0' + b))
        .collect();
    if frac.is_empty() {
        format!("{sign}{int}")
    } else {
        format!("{sign}{int}.{frac}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(v: f64, fmt: &str) -> String {
        format_number(&Digits::from_f64(v).unwrap_or_else(Digits::zero), fmt)
    }

    fn i(v: i64, fmt: &str) -> String {
        format_number(
            &Digits::from_value(&Value::Integer(v)).unwrap_or_else(Digits::zero),
            fmt,
        )
    }

    #[test]
    fn digit_placeholders_and_grouping() {
        assert_eq!(i(2024, "0"), "2024");
        assert_eq!(i(5, "000"), "005");
        assert_eq!(i(0, "#"), "");
        assert_eq!(i(0, "0"), "0");
        assert_eq!(i(1_234_567, "#,0"), "1,234,567");
        assert_eq!(i(1_234_567, "#,##0"), "1,234,567");
        assert_eq!(i(123, "#,0"), "123");
        assert_eq!(i(-1_234, "#,0"), "-1,234");
        assert_eq!(f(1234.5678, "#,0.00"), "1,234.57");
        assert_eq!(f(0.5, "#.##"), ".5");
        assert_eq!(f(0.5, "0.##"), "0.5");
        assert_eq!(f(2.0, "0.0#"), "2.0");
        assert_eq!(f(12.5, ".00"), "12.50");
        assert_eq!(f(1.0, "0."), "1");
        assert_eq!(f(0.125, "0.00"), "0.13");
        assert_eq!(f(2.675, "0.00"), "2.68");
        assert_eq!(f(-0.5, "0"), "-1");
        assert_eq!(f(9.999, "0.00"), "10.00");
        assert_eq!(i(123_456_789, "000-00-0000"), "123-45-6789");
    }

    #[test]
    fn sections_percent_scaling_and_literals() {
        assert_eq!(f(0.452_915, "0.0%;-0.0%;0.0%"), "45.3%");
        assert_eq!(f(-0.452_915, "0.0%;-0.0%;0.0%"), "-45.3%");
        assert_eq!(f(0.0, "0.0%;-0.0%;0.0%"), "0.0%");
        assert_eq!(f(0.000_01, "0.0%;-0.0%;\"zero\""), "zero");
        assert_eq!(f(-0.000_01, "0.0%"), "0.0%");
        assert_eq!(f(0.25, "0%"), "25%");
        assert_eq!(f(-5.0, "$#,##0;($#,##0)"), "($5)");
        assert_eq!(f(5.0, "$#,##0;($#,##0)"), "$5");
        assert_eq!(f(-5.0, "$#,##0"), "-$5");
        assert_eq!(f(0.0, "0;-0;\\Z\\e\\r\\o"), "Zero");
        assert_eq!(f(-3.0, "0;;0"), "-3");
        assert_eq!(f(1234.5, "\\$#,0.00"), "$1,234.50");
        assert_eq!(f(1234.5, "\"USD \"#,0"), "USD 1,235");
        assert_eq!(f(123_456_789.0, "##0,,"), "123");
        assert_eq!(f(1_500_000.0, "#,0.0,, \"M\""), "1.5 M");
        assert_eq!(f(1_234.0, "0.0,"), "1.2");
    }

    #[test]
    fn scientific() {
        assert_eq!(f(12345.67, "0.00E+00"), "1.23E+04");
        assert_eq!(f(0.000_123, "0.00E+00"), "1.23E-04");
        assert_eq!(f(0.000_123, "0.0e-0"), "1.2e-4");
        assert_eq!(f(12345.0, "0.0E-0"), "1.2E4");
        assert_eq!(f(9.999, "0.00E+00"), "1.00E+01");
        assert_eq!(f(0.0, "0.00E+00"), "0.00E+00");
        assert_eq!(f(-12345.67, "0.00E+00"), "-1.23E+04");
    }

    #[test]
    fn exact_digits_for_integers_and_decimals() {
        assert_eq!(i(i64::MAX, "#,0"), "9,223,372,036,854,775,807");
        assert_eq!(i(i64::MIN, "0"), "-9223372036854775808");
        let d = Digits::from_value(&Value::Decimal(12_345)).unwrap_or_else(Digits::zero);
        assert_eq!(format_number(&d, "0.0000"), "1.2345");
        assert_eq!(format_number(&d, "0.00"), "1.23");
    }

    #[test]
    fn general_numbers() {
        assert_eq!(general_double(12345.67), "12345.67");
        assert_eq!(general_double(0.1 + 0.2), "0.3");
        assert_eq!(general_double(5.0 / 11.0), "0.454545454545455");
        assert_eq!(general_double(-2.5), "-2.5");
        assert_eq!(general_double(1e20), "1E+20");
        assert_eq!(general_double(1.5e-7), "1.5E-07");
        assert_eq!(general_double(100.0), "100");
        assert_eq!(general_double(0.0), "0");
    }
}
