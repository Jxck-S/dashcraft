//! Linear and time scales with Power BI-like "nice" ticks.
//!
//! - [`nice_step`]: tick step `1`, `2`, `2.5`, `5` × 10ⁿ for a target tick
//!   count (d3-style error thresholds, so a 0–7 axis with 3 ticks steps by
//!   2 and a 0–10 axis by 5, as the service draws them).
//! - [`ticks`]: the multiples of the step inside the domain.
//! - [`value_domain`]: data extent with zero included (bar/column/area
//!   charts start at 0); the end is the data maximum, not a rounded tick
//!   (Power BI lets the longest bar reach the plot edge).
//! - [`time_ticks`]: day/week/month/quarter/year ticks for a continuous
//!   date axis (OLE serial days).
//!
//! Every function tolerates NaN/∞ and empty or degenerate domains.

/// Largest number of ticks generated (untrusted-input guard).
pub const MAX_TICKS: usize = 200;

/// Maps a numeric domain to a pixel range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinearScale {
    pub d0: f64,
    pub d1: f64,
    pub r0: f64,
    pub r1: f64,
}

impl LinearScale {
    /// A scale from `domain` to `range`. A degenerate or non-finite domain
    /// becomes `[d0, d0 + 1]`.
    pub fn new(domain: (f64, f64), range: (f64, f64)) -> Self {
        let (mut d0, mut d1) = domain;
        if !d0.is_finite() {
            d0 = 0.0;
        }
        if !d1.is_finite() || (d1 - d0).abs() < f64::EPSILON * d0.abs().max(1.0) {
            d1 = d0 + 1.0;
        }
        let r0 = if range.0.is_finite() { range.0 } else { 0.0 };
        let r1 = if range.1.is_finite() { range.1 } else { r0 };
        Self { d0, d1, r0, r1 }
    }

    /// Pixel position of `v` (not clamped; non-finite input maps to `r0`).
    pub fn map(&self, v: f64) -> f64 {
        if !v.is_finite() {
            return self.r0;
        }
        let t = (v - self.d0) / (self.d1 - self.d0);
        self.r0 + t * (self.r1 - self.r0)
    }

    /// Pixel position of `v`, clamped to the range.
    pub fn map_clamped(&self, v: f64) -> f64 {
        let lo = self.r0.min(self.r1);
        let hi = self.r0.max(self.r1);
        self.map(v).clamp(lo, hi)
    }
}

/// A "nice" step (`1`, `2`, `2.5`, `5` × 10ⁿ) splitting `span` into about
/// `count` intervals. `0` for an empty or invalid span.
pub fn nice_step(span: f64, count: usize) -> f64 {
    let span = span.abs();
    if !span.is_finite() || span <= 0.0 {
        return 0.0;
    }
    let count = count.max(1) as f64;
    let raw = span / count;
    let mut step = 10f64.powf(raw.log10().floor());
    if !step.is_finite() || step <= 0.0 {
        return 0.0;
    }
    // Fraction of the target count the base step would give.
    let err = count / span * step;
    if err <= 0.15 {
        step *= 10.0;
    } else if err <= 0.35 {
        step *= 5.0;
    } else if err <= 0.4 {
        step *= 2.5;
    } else if err <= 0.75 {
        step *= 2.0;
    }
    step
}

/// Multiples of [`nice_step`] inside `[min(d0, d1), max(d0, d1)]`
/// (ascending, at most [`MAX_TICKS`]).
pub fn ticks(d0: f64, d1: f64, count: usize) -> Vec<f64> {
    if !d0.is_finite() || !d1.is_finite() {
        return Vec::new();
    }
    let (lo, hi) = (d0.min(d1), d0.max(d1));
    if hi - lo <= 0.0 {
        return vec![lo];
    }
    let step = nice_step(hi - lo, count);
    ticks_with_step(lo, hi, step)
}

/// Multiples of `step` inside `[lo, hi]`.
pub fn ticks_with_step(lo: f64, hi: f64, step: f64) -> Vec<f64> {
    if !(step.is_finite() && step > 0.0 && lo.is_finite() && hi.is_finite()) {
        return Vec::new();
    }
    let eps = step * 1e-9;
    let first = (lo / step - 1e-9).ceil();
    let last = (hi / step + 1e-9).floor();
    if !first.is_finite() || !last.is_finite() || last < first {
        return Vec::new();
    }
    let n = ((last - first).min(MAX_TICKS as f64) as usize + 1).min(MAX_TICKS);
    // Snap float noise (0.6000000000000001 → 0.6) to the step's decimals.
    let decimals = (0..=12)
        .find(|d| {
            let scaled = step * 10f64.powi(*d);
            (scaled - scaled.round()).abs() < 1e-6 * scaled.abs().max(1.0)
        })
        .unwrap_or(12);
    let factor = 10f64.powi(decimals);
    (0..n)
        .map(|i| {
            let v = (first + i as f64) * step;
            let snapped = (v * factor).round() / factor;
            if snapped.abs() < eps || !snapped.is_finite() {
                if snapped.is_finite() { 0.0 } else { v }
            } else {
                snapped
            }
        })
        .filter(|v| *v >= lo - eps && *v <= hi + eps)
        .collect()
}

/// Recommended tick count for an axis of `length` pixels: horizontal axes
/// get one tick per ~150px (the service draws 0/2/4/6 on a 470px axis),
/// vertical axes 3/5/8 by height.
pub fn tick_count(length: f64, horizontal: bool) -> usize {
    if !length.is_finite() || length <= 0.0 {
        return 2;
    }
    if horizontal {
        ((length / 150.0).floor() as usize).clamp(2, 10)
    } else if length < 150.0 {
        3
    } else if length < 300.0 {
        5
    } else {
        8
    }
}

/// Data extent of `values` (finite only), with 0 included when
/// `include_zero`. `(0, 1)` when there is no data; a single value `v`
/// gives `(min(0, v), max(0, v))` or `(v - 1, v + 1)`.
pub fn value_domain(values: impl IntoIterator<Item = f64>, include_zero: bool) -> (f64, f64) {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for v in values.into_iter().filter(|v| v.is_finite()) {
        lo = lo.min(v);
        hi = hi.max(v);
    }
    if !lo.is_finite() || !hi.is_finite() {
        return (0.0, 1.0);
    }
    if include_zero {
        lo = lo.min(0.0);
        hi = hi.max(0.0);
    }
    if hi - lo <= 0.0 {
        if lo == 0.0 {
            return (0.0, 1.0);
        }
        let pad = lo.abs() * 0.1;
        return (lo - pad, hi + pad);
    }
    (lo, hi)
}

/// Extends `(lo, hi)` outwards to multiples of the nice step (line charts
/// whose axis does not include zero).
pub fn nice_domain(lo: f64, hi: f64, count: usize) -> (f64, f64) {
    let step = nice_step(hi - lo, count);
    if step <= 0.0 {
        return (lo, hi);
    }
    ((lo / step).floor() * step, (hi / step).ceil() * step)
}

/// A tick on a continuous date axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeTick {
    /// OLE serial date (days since 1899-12-30).
    pub serial: f64,
    /// Tick spacing unit (for label formatting).
    pub unit: TimeUnit,
}

/// Spacing of [`time_ticks`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeUnit {
    Day,
    Month,
    Year,
}

impl TimeUnit {
    /// Power BI-like label format for ticks of this unit.
    pub fn format(self) -> &'static str {
        match self {
            TimeUnit::Day => "MMM dd",
            TimeUnit::Month => "MMM yyyy",
            TimeUnit::Year => "yyyy",
        }
    }
}

/// Day/week/month/quarter/year ticks in `[lo, hi]` (OLE serial days), at
/// most about `count` of them.
pub fn time_ticks(lo: f64, hi: f64, count: usize) -> Vec<TimeTick> {
    // Serial days beyond ±3M (year ~10,000) are not dates.
    let sane = |v: f64| v.is_finite() && v.abs() <= 3_000_000.0;
    if !(sane(lo) && sane(hi)) || hi < lo {
        return Vec::new();
    }
    let count = count.max(1) as f64;
    let span_days = (hi - lo).max(1.0);
    let day_steps = [1.0, 2.0, 7.0, 14.0];
    if let Some(step) = day_steps.iter().find(|s| span_days / **s <= count) {
        let first = (lo / step).ceil() * step;
        return (0..MAX_TICKS)
            .map(|i| first + i as f64 * step)
            .take_while(|s| *s <= hi + 1e-9)
            .map(|serial| TimeTick {
                serial,
                unit: TimeUnit::Day,
            })
            .collect();
    }
    let span_months = span_days / 30.44;
    let month_steps = [1i64, 2, 3, 6];
    if let Some(step) = month_steps
        .iter()
        .find(|s| span_months / (**s as f64) <= count)
    {
        return month_ticks(lo, hi, *step, TimeUnit::Month);
    }
    let years = span_days / 365.25;
    let year_step = (nice_step(years, count as usize).ceil() as i64).max(1);
    month_ticks(lo, hi, year_step * 12, TimeUnit::Year)
}

fn month_ticks(lo: f64, hi: f64, months: i64, unit: TimeUnit) -> Vec<TimeTick> {
    let (y, m, _) = civil_from_serial(lo.floor() as i64);
    let mut index = y * 12 + (m - 1);
    // Align to the step (calendar quarters, half years, whole years).
    index = index.div_euclid(months) * months;
    let mut out = Vec::new();
    while out.len() < MAX_TICKS {
        let serial = serial_from_civil(index.div_euclid(12), index.rem_euclid(12) + 1, 1) as f64;
        if serial > hi + 1e-9 {
            break;
        }
        if serial >= lo - 1e-9 {
            out.push(TimeTick { serial, unit });
        }
        index += months;
    }
    out
}

/// Days between 1899-12-30 (OLE serial 0) and 1970-01-01.
const OLE_UNIX_DAYS: i64 = 25_569;

/// `(year, month, day)` of an OLE serial day.
pub fn civil_from_serial(serial: i64) -> (i64, i64, i64) {
    // Howard Hinnant's days-from-civil inverse.
    let z = serial.saturating_sub(OLE_UNIX_DAYS).saturating_add(719_468);
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// OLE serial day of a calendar date.
pub fn serial_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468 + OLE_UNIX_DAYS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trip() {
        for serial in [0, 1, 25_569, 46_000, 60, 61, -700] {
            let (y, m, d) = civil_from_serial(serial);
            assert_eq!(serial_from_civil(y, m, d), serial, "{serial}");
        }
        assert_eq!(civil_from_serial(25_569), (1970, 1, 1));
        assert_eq!(civil_from_serial(46_023), (2026, 1, 1));
    }

    #[test]
    fn month_and_year_ticks() {
        let lo = serial_from_civil(2026, 1, 15) as f64;
        let hi = serial_from_civil(2026, 12, 1) as f64;
        let t = time_ticks(lo, hi, 6);
        assert!(t.iter().all(|t| t.unit == TimeUnit::Month), "{t:?}");
        assert_eq!(civil_from_serial(t[0].serial as i64), (2026, 3, 1));
        let decade = time_ticks(lo, lo + 3_650.0, 5);
        assert!(decade.iter().all(|t| t.unit == TimeUnit::Year));
        assert!(time_ticks(f64::NAN, 1.0, 3).is_empty());
    }
}
