//! Decimal digit buffers for Power Query-compatible number text and rounding.
//!
//! Power Query runs on .NET Framework: standard and custom numeric formats start from the
//! number's 15 significant digits and round them half away from zero; round-trip text uses 15
//! digits when they parse back to the same double and 17 otherwise; `Number.Round` and friends
//! convert the double to `System.Decimal` (15 significant digits, at most 28 decimals) and round
//! that. Working on these digits instead of scaled doubles gives the same answers as Desktop for
//! values such as `2.675` or `0.29` whose binary form sits just below the decimal one.

/// The largest `System.Decimal` magnitude; larger numbers stay doubles in Power Query.
pub(crate) const DECIMAL_MAX: f64 = 7.922_816_251_426_434e28;

/// How to drop digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rounding {
    /// Nearest, ties to the even digit (`RoundingMode.ToEven`, `Number.Round`'s default).
    HalfEven,
    /// Nearest, ties away from zero (`RoundingMode.AwayFromZero`, .NET number formats).
    HalfAwayFromZero,
    /// Nearest, ties toward zero (`RoundingMode.TowardZero`).
    HalfTowardZero,
    /// Nearest, ties toward positive infinity (`RoundingMode.Up`).
    HalfUp,
    /// Nearest, ties toward negative infinity (`RoundingMode.Down`).
    HalfDown,
    /// Toward positive infinity (`Number.RoundUp`).
    Ceiling,
    /// Toward negative infinity (`Number.RoundDown`).
    Floor,
    /// Away from zero (`Number.RoundAwayFromZero`).
    AwayFromZero,
    /// Toward zero (`Number.RoundTowardZero`).
    TowardZero,
}

impl Rounding {
    /// The tie-breaking rule of an M `RoundingMode` value; `None` for an unknown value.
    pub(crate) fn from_mode(mode: i64) -> Option<Rounding> {
        Some(match mode {
            0 => Rounding::HalfUp,
            1 => Rounding::HalfDown,
            2 => Rounding::HalfAwayFromZero,
            3 => Rounding::HalfTowardZero,
            4 => Rounding::HalfEven,
            _ => return None,
        })
    }
}

/// A finite number as `±0.d₁d₂…dₙ × 10^point`, without trailing zero digits; zero has no digits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Digits {
    pub(crate) negative: bool,
    digits: Vec<u8>,
    /// The number of digits before the decimal point (negative for leading fraction zeros).
    point: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Rest {
    Zero,
    Below,
    Half,
    Above,
}

impl Digits {
    fn zero() -> Digits {
        Digits {
            negative: false,
            digits: Vec::new(),
            point: 0,
        }
    }

    /// `n` correctly rounded to `significant` (1–17) digits; `None` for NaN and infinities.
    pub(crate) fn of(n: f64, significant: usize) -> Option<Digits> {
        if !n.is_finite() {
            return None;
        }
        let significant = significant.clamp(1, 17);
        let text = format!("{:.*e}", significant - 1, n.abs());
        let (mantissa, exp) = text.split_once('e')?;
        let exp: i32 = exp.parse().ok()?;
        let mut digits: Vec<u8> = mantissa
            .bytes()
            .filter(u8::is_ascii_digit)
            .map(|b| b - b'0')
            .collect();
        while digits.last() == Some(&0) {
            digits.pop();
        }
        if digits.is_empty() {
            return Some(Digits::zero());
        }
        Some(Digits {
            negative: n < 0.0,
            digits,
            point: exp.saturating_add(1),
        })
    }

    /// .NET Framework's round-trip digits: 15 when they parse back to `n`, else 17. Returns the
    /// digits and the precision used.
    pub(crate) fn round_trip(n: f64) -> Option<(Digits, usize)> {
        let d15 = Digits::of(n, 15)?;
        if d15.to_f64() == n {
            return Some((d15, 15));
        }
        Some((Digits::of(n, 17)?, 17))
    }

    pub(crate) fn is_zero(&self) -> bool {
        self.digits.is_empty()
    }

    /// The decimal exponent of the first digit (`1.5E+03` → 3); 0 for zero.
    pub(crate) fn exponent(&self) -> i32 {
        if self.is_zero() { 0 } else { self.point - 1 }
    }

    /// Multiplies by `10^places`.
    pub(crate) fn shift(&mut self, places: i32) {
        if !self.is_zero() {
            self.point = self.point.saturating_add(places);
        }
    }

    pub(crate) fn to_f64(&self) -> f64 {
        if self.is_zero() {
            return 0.0;
        }
        let mut s = String::with_capacity(self.digits.len() + 8);
        if self.negative {
            s.push('-');
        }
        s.extend(self.digits.iter().map(|d| char::from(b'0' + d)));
        let len = i64::try_from(self.digits.len()).unwrap_or(i64::MAX);
        s.push('e');
        s.push_str(&(i64::from(self.point) - len).to_string());
        s.parse().unwrap_or(0.0)
    }

    /// Rounds to `decimals` digits after the decimal point (before it when negative).
    pub(crate) fn round(&mut self, decimals: i32, mode: Rounding) {
        if self.is_zero() {
            return;
        }
        let keep = i64::from(self.point) + i64::from(decimals);
        let len = i64::try_from(self.digits.len()).unwrap_or(i64::MAX);
        if keep >= len {
            return;
        }
        let (mut kept, rest) = match usize::try_from(keep) {
            Ok(k) => match self.digits.split_at_checked(k) {
                Some((kept, rest)) => (kept.to_vec(), classify(rest)),
                None => return,
            },
            Err(_) => (Vec::new(), Rest::Below),
        };
        let positive = !self.negative;
        let last_odd = kept.last().is_some_and(|d| d % 2 == 1);
        let up = match (rest, mode) {
            (Rest::Zero, _) => false,
            (_, Rounding::Ceiling) => positive,
            (_, Rounding::Floor) => !positive,
            (_, Rounding::AwayFromZero) => true,
            (_, Rounding::TowardZero) => false,
            (Rest::Above, _) => true,
            (Rest::Below, _) => false,
            (Rest::Half, Rounding::HalfEven) => last_odd,
            (Rest::Half, Rounding::HalfAwayFromZero) => true,
            (Rest::Half, Rounding::HalfTowardZero) => false,
            (Rest::Half, Rounding::HalfUp) => positive,
            (Rest::Half, Rounding::HalfDown) => !positive,
        };
        let mut point = self.point;
        if up {
            if kept.is_empty() {
                kept.push(1);
                point = 1_i32.saturating_sub(decimals);
            } else if let Some(i) = kept.iter().rposition(|&d| d != 9) {
                for d in kept.iter_mut().skip(i + 1) {
                    *d = 0;
                }
                if let Some(d) = kept.get_mut(i) {
                    *d += 1;
                }
            } else {
                kept.clear();
                kept.push(1);
                point = point.saturating_add(1);
            }
        }
        while kept.last() == Some(&0) {
            kept.pop();
        }
        if kept.is_empty() {
            *self = Digits::zero();
        } else {
            self.digits = kept;
            self.point = point;
        }
    }

    fn digit(&self, index: i64) -> char {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.digits.get(i))
            .map_or('0', |d| char::from(b'0' + d))
    }

    /// The integer digits (at least `0`) and exactly `decimals` fraction digits, unsigned.
    pub(crate) fn fixed_parts(&self, decimals: usize) -> (String, String) {
        let point = i64::from(self.point);
        let mut int: String = (0..point.max(0)).map(|i| self.digit(i)).collect();
        if int.is_empty() {
            int.push('0');
        }
        let frac = (0..i64::try_from(decimals).unwrap_or(0))
            .map(|j| self.digit(point + j))
            .collect();
        (int, frac)
    }

    fn sign(&self) -> &'static str {
        if self.negative && !self.is_zero() {
            "-"
        } else {
            ""
        }
    }

    /// Positional text with every digit: `123.45`, `0.001`, `1000`.
    pub(crate) fn positional(&self) -> String {
        let len = i64::try_from(self.digits.len()).unwrap_or(0);
        let decimals = usize::try_from(len - i64::from(self.point)).unwrap_or(0);
        let (int, frac) = self.fixed_parts(decimals);
        if frac.is_empty() {
            format!("{}{int}", self.sign())
        } else {
            format!("{}{int}.{frac}", self.sign())
        }
    }

    /// Scientific text `d.ddd` + `e` + signed exponent of at least `exp_digits` digits. The
    /// mantissa has `decimals` fraction digits, or every remaining digit when `None`.
    pub(crate) fn scientific(&self, decimals: Option<usize>, e: char, exp_digits: usize) -> String {
        let all = self.digits.len().saturating_sub(1);
        let decimals = decimals.unwrap_or(all);
        let mut out = String::from(self.sign());
        out.push(self.digit(0));
        if decimals > 0 {
            out.push('.');
            out.extend((1..=i64::try_from(decimals).unwrap_or(0)).map(|i| self.digit(i)));
        }
        let exp = self.exponent();
        out.push(e);
        out.push(if exp < 0 { '-' } else { '+' });
        out.push_str(&format!("{:0exp_digits$}", exp.unsigned_abs()));
        out
    }

    /// .NET's `G` layout for these digits: positional when the exponent is above -5 and below
    /// `precision`, else scientific with 2 exponent digits.
    pub(crate) fn general(&self, precision: usize, e: char) -> String {
        if self.is_zero() {
            return "0".into();
        }
        let exp = i64::from(self.exponent());
        let precision = i64::try_from(precision).unwrap_or(i64::MAX);
        if exp > -5 && exp < precision {
            self.positional()
        } else {
            self.scientific(None, e, 2)
        }
    }
}

fn classify(rest: &[u8]) -> Rest {
    let Some((&first, tail)) = rest.split_first() else {
        return Rest::Zero;
    };
    let tail_nonzero = tail.iter().any(|&d| d != 0);
    match first.cmp(&5) {
        std::cmp::Ordering::Less if first == 0 && !tail_nonzero => Rest::Zero,
        std::cmp::Ordering::Less => Rest::Below,
        std::cmp::Ordering::Equal if tail_nonzero => Rest::Above,
        std::cmp::Ordering::Equal => Rest::Half,
        std::cmp::Ordering::Greater => Rest::Above,
    }
}

/// Power Query's `G`/round-trip text of `n` from its `precision`-digit `digits`: .NET's `G`
/// layout, except that a number in the `System.Decimal` range is written out in full from its
/// 15 significant digits (`1E+21` → `1000000000000000000000`) instead of in exponent notation.
pub(crate) fn general_number(n: f64, digits: &Digits, precision: usize, e: char) -> String {
    let exp = i64::from(digits.exponent());
    if exp >= i64::try_from(precision).unwrap_or(i64::MAX)
        && n.abs() < DECIMAL_MAX
        && let Some(d) = Digits::of(n, 15)
    {
        return d.positional();
    }
    digits.general(precision, e)
}

/// Rounds like Power Query's `Number.Round` family: in `System.Decimal` (the double's 15
/// significant digits, at most 28 decimals) when the magnitude fits, else on the round-trip digits.
pub(crate) fn round_number(n: f64, decimals: i32, mode: Rounding) -> f64 {
    if !n.is_finite() || n == 0.0 {
        return n;
    }
    if decimals == 0 && n.fract() == 0.0 {
        return n;
    }
    if decimals == 0 && n.abs() < 1e9 && is_tie_mode(mode) && (n.fract().abs() - 0.5).abs() > 1e-6 {
        return n.round();
    }
    let digits = if n.abs() < DECIMAL_MAX {
        Digits::of(n, 15).map(|mut d| {
            d.round(28, Rounding::HalfEven);
            d
        })
    } else {
        Digits::round_trip(n).map(|(d, _)| d)
    };
    let Some(mut d) = digits else {
        return n;
    };
    d.round(decimals, mode);
    d.to_f64()
}

fn is_tie_mode(mode: Rounding) -> bool {
    matches!(
        mode,
        Rounding::HalfEven
            | Rounding::HalfAwayFromZero
            | Rounding::HalfTowardZero
            | Rounding::HalfUp
            | Rounding::HalfDown
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_like_power_query() {
        let r = round_number;
        assert_eq!(r(2.345, 2, Rounding::HalfEven), 2.34);
        assert_eq!(r(2.675, 2, Rounding::HalfEven), 2.68);
        assert_eq!(r(1.005, 2, Rounding::HalfEven), 1.0);
        assert_eq!(r(125.0, -1, Rounding::HalfEven), 120.0);
        assert_eq!(r(-0.5, 0, Rounding::HalfEven), 0.0);
        assert_eq!(r(0.1 + 0.2, 17, Rounding::HalfEven), 0.3);
        assert_eq!(r(-1.2, 0, Rounding::AwayFromZero), -2.0);
        assert_eq!(r(-1.25, 1, Rounding::AwayFromZero), -1.3);
        assert_eq!(r(0.29, 2, Rounding::Floor), 0.29);
        assert_eq!(r(1e-30, 2, Rounding::Ceiling), 0.0);
        assert_eq!(r(2.4, 0, Rounding::HalfUp), 2.0);
        assert_eq!(r(-2.5, 0, Rounding::HalfUp), -2.0);
        assert_eq!(r(-2.5, 0, Rounding::HalfDown), -3.0);
        assert_eq!(r(9.99, 1, Rounding::HalfAwayFromZero), 10.0);
        assert_eq!(r(0.006, 1, Rounding::Ceiling), 0.1);
        assert_eq!(r(9e28, 0, Rounding::HalfEven), 9e28);
    }

    #[test]
    fn lays_out_digits() {
        let (d, p) = Digits::round_trip(0.1 + 0.2).unwrap_or_else(|| unreachable!());
        assert_eq!((d.general(p, 'E'), p), ("0.30000000000000004".into(), 17));
        let d = Digits::of(1e21, 15).unwrap_or_else(|| unreachable!());
        assert_eq!(general_number(1e21, &d, 15, 'E'), "1000000000000000000000");
        assert_eq!(d.general(15, 'E'), "1E+21");
        let x = 1.2345678901234568e23;
        let (d, p) = Digits::round_trip(x).unwrap_or_else(|| unreachable!());
        assert_eq!(d.general(p, 'E'), "1.2345678901234569E+23");
        assert_eq!(general_number(x, &d, p, 'E'), "123456789012346000000000");
        let d = Digits::of(-1.5e-5, 15).unwrap_or_else(|| unreachable!());
        assert_eq!(general_number(-1.5e-5, &d, 15, 'E'), "-1.5E-05");
        assert_eq!(d.scientific(Some(3), 'e', 3), "-1.500e-005");
        let d = Digits::of(-0.001, 15).unwrap_or_else(|| unreachable!());
        let mut r = d.clone();
        r.round(2, Rounding::HalfAwayFromZero);
        assert_eq!(r.fixed_parts(2), ("0".into(), "00".into()));
        assert_eq!(d.positional(), "-0.001");
    }
}
