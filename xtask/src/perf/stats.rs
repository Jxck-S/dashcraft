//! Summary statistics of timing samples.

use serde::{Deserialize, Serialize};

use super::budgets::Metric;

/// Statistics of one scenario's samples, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    pub samples: usize,
    pub min_ms: f64,
    pub median_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
    pub mean_ms: f64,
}

impl Stats {
    /// `None` for no (finite) samples.
    pub fn from_samples(samples: &[f64]) -> Option<Self> {
        let mut v: Vec<f64> = samples.iter().copied().filter(|x| x.is_finite()).collect();
        if v.is_empty() {
            return None;
        }
        v.sort_by(f64::total_cmp);
        let n = v.len();
        let at = |i: usize| v.get(i).copied().unwrap_or(f64::NAN);
        let median = if n % 2 == 1 {
            at(n / 2)
        } else {
            (at(n / 2 - 1) + at(n / 2)) / 2.0
        };
        // Nearest-rank percentile: the smallest sample with at least 95% of samples ≤ it.
        let rank = (n * 95).div_ceil(100).max(1);
        Some(Self {
            samples: n,
            min_ms: round_us(at(0)),
            median_ms: round_us(median),
            p95_ms: round_us(at(rank - 1)),
            max_ms: round_us(at(n - 1)),
            mean_ms: round_us(v.iter().sum::<f64>() / n as f64),
        })
    }

    pub fn metric(&self, metric: Metric) -> f64 {
        match metric {
            Metric::Median => self.median_ms,
            Metric::P95 => self.p95_ms,
        }
    }
}

/// Rounded to 0.1 µs (keeps reports and the baseline readable).
fn round_us(ms: f64) -> f64 {
    (ms * 1e4).round() / 1e4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_and_nearest_rank_p95() {
        let s = Stats::from_samples(&[5.0, 1.0, 3.0]).unwrap();
        assert_eq!(
            (s.min_ms, s.median_ms, s.p95_ms, s.max_ms),
            (1.0, 3.0, 5.0, 5.0)
        );
        assert_eq!(s.mean_ms, 3.0);

        let s = Stats::from_samples(&[4.0, 1.0, 3.0, 2.0]).unwrap();
        assert_eq!(s.median_ms, 2.5);

        let twenty: Vec<f64> = (1..=20).map(f64::from).collect();
        let s = Stats::from_samples(&twenty).unwrap();
        assert_eq!((s.p95_ms, s.metric(Metric::P95)), (19.0, 19.0));
        assert_eq!(s.metric(Metric::Median), 10.5);

        let hundred: Vec<f64> = (1..=100).rev().map(f64::from).collect();
        assert_eq!(Stats::from_samples(&hundred).unwrap().p95_ms, 95.0);

        let s = Stats::from_samples(&[1.512_345_67, 1.511_111_11]).unwrap();
        assert_eq!(s.median_ms, 1.5117);
    }

    #[test]
    fn ignores_non_finite_samples() {
        assert_eq!(Stats::from_samples(&[]), None);
        assert_eq!(Stats::from_samples(&[f64::NAN]), None);
        let s = Stats::from_samples(&[f64::INFINITY, 2.0]).unwrap();
        assert_eq!((s.samples, s.median_ms), (1, 2.0));
    }
}
