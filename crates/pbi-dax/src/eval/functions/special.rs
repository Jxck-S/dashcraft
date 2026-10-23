//! Special functions behind the statistical distributions: log-gamma,
//! regularized incomplete gamma and beta functions, the normal CDF and its
//! inverse, and a safeguarded Newton root finder for the other inverses.
//!
//! Written from the mathematical definitions (no third-party code or
//! coefficient tables):
//!
//! - `ln Γ(x)`: Stirling's series with the Bernoulli-number terms up to
//!   `x⁻¹³`, after shifting `x` to at least 10 with `Γ(x+1) = xΓ(x)`;
//!   absolute error below `1e-15`.
//! - `P(a, x)` / `Q(a, x)`: the power series for `x < a + 1`, otherwise
//!   Legendre's continued fraction evaluated with the modified Lentz
//!   method. The prefactor `xᵃe⁻ˣ/Γ(a)` uses `ln(1+t) - t` and the
//!   Stirling remainder for large `a`, so no large logarithms cancel.
//! - `I_x(a, b)`: the classic continued fraction (Lentz), on the side of
//!   `(a+1)/(a+b+2)` where it converges fast, with `x` and `1 - x` passed
//!   separately so neither tail loses digits; the prefactor
//!   `xᵃ(1-x)ᵇ/B(a,b)` is computed in the same cancellation-free form for
//!   large parameters.
//! - Inverses: Halley's method for the normal quantile, Newton steps kept
//!   inside a shrinking bracket (bisection otherwise) for the others.
//!
//! Relative accuracy is about `1e-13` or better for the parameters seen in
//! practice (degrees of freedom up to about `1e6`); it degrades slowly
//! (roughly `ε·√a`) for very large shape parameters. Iterations are capped,
//! so a calculation that does not converge returns `None` instead of
//! looping.

/// `0.5 · ln(2π)`.
const HALF_LN_2PI: f64 = 0.918_938_533_204_672_8;
/// `√(2π)`.
const SQRT_2PI: f64 = 2.506_628_274_631_000_2;
/// Lentz's guard against division by zero.
const TINY: f64 = 1.0e-300;
const EPS: f64 = f64::EPSILON;

/// Iteration cap for the series and continued fractions: they need
/// `O(√a)` terms when `x` is close to the distribution's centre.
fn iteration_cap(a: f64) -> usize {
    (1_000.0 + 60.0 * a.max(0.0).sqrt()).min(8_000_000.0) as usize
}

/// Stirling remainder `δ(x) = ln Γ(x) - [(x - ½)ln x - x + ½ln 2π]` for
/// `x ≥ 10` (the asymptotic series `Σ B₂ₖ / (2k(2k-1)x²ᵏ⁻¹)`).
fn stirling_tail(x: f64) -> f64 {
    let r = 1.0 / x;
    let r2 = r * r;
    // B2/2, B4/12, B6/30, B8/56, B10/90, B12/132, B14/182 (Bernoulli numbers).
    let terms = [
        1.0 / 12.0,
        -1.0 / 360.0,
        1.0 / 1260.0,
        -1.0 / 1680.0,
        1.0 / 1188.0,
        -691.0 / 360_360.0,
        1.0 / 156.0,
    ];
    let mut sum = 0.0;
    for c in terms.iter().rev() {
        sum = sum * r2 + c;
    }
    sum * r
}

/// `ln Γ(x)` for `x > 0` (`NaN` otherwise).
pub(crate) fn ln_gamma(x: f64) -> f64 {
    if x.is_nan() || x <= 0.0 {
        return f64::NAN;
    }
    if x.is_infinite() {
        return f64::INFINITY;
    }
    let mut y = x;
    let mut prod = 1.0;
    while y < 10.0 {
        prod *= y;
        y += 1.0;
    }
    let shift = if prod == 1.0 { 0.0 } else { prod.ln() };
    (y - 0.5) * y.ln() - y + HALF_LN_2PI + stirling_tail(y) - shift
}

/// `ln(1 + t) - t`, accurate for small `t` (no cancellation).
pub(crate) fn log1pmx(t: f64) -> f64 {
    if t.abs() > 0.25 {
        return t.ln_1p() - t;
    }
    // -t²/2 + t³/3 - t⁴/4 + …
    let mut power = t * t;
    let mut sum = 0.0;
    let mut k = 2.0;
    let mut sign = -1.0;
    while k < 200.0 {
        let term = sign * power / k;
        sum += term;
        if term.abs() <= EPS * sum.abs() {
            break;
        }
        power *= t;
        sign = -sign;
        k += 1.0;
    }
    sum
}

/// `ln Γ(a) - ln Γ(a + b)` for `a, b > 0`; for `a ≥ 10` without the
/// cancellation of two large logarithms.
pub(crate) fn ln_gamma_ratio(a: f64, b: f64) -> f64 {
    if a < 10.0 {
        return ln_gamma(a) - ln_gamma(a + b);
    }
    -(a - 0.5) * (b / a).ln_1p() - b * (a + b).ln() + b + stirling_tail(a) - stirling_tail(a + b)
}

/// `ln B(a, b)` for `a, b > 0`.
pub(crate) fn ln_beta(a: f64, b: f64) -> f64 {
    let (small, big) = if a < b { (a, b) } else { (b, a) };
    if big < 10.0 {
        return ln_gamma(a) + ln_gamma(b) - ln_gamma(a + b);
    }
    if small < 10.0 {
        return ln_gamma(small) + ln_gamma_ratio(big, small);
    }
    // Both large: ln B = a ln x₀ + b ln y₀ - ½ ln(ab/(a+b)) + ½ ln 2π + Δ.
    let s = a + b;
    a * (-(b / a).ln_1p()) + b * (-(a / b).ln_1p()) - 0.5 * (a * b / s).ln()
        + HALF_LN_2PI
        + stirling_tail(a)
        + stirling_tail(b)
        - stirling_tail(s)
}

/// `ln(xᵃ e⁻ˣ / Γ(a))`.
pub(crate) fn ln_gamma_prefix(a: f64, x: f64) -> f64 {
    if a < 10.0 {
        return a * x.ln() - x - ln_gamma(a);
    }
    let t = (x - a) / a;
    a * log1pmx(t) + 0.5 * a.ln() - HALF_LN_2PI - stirling_tail(a)
}

/// Regularized incomplete gamma functions `(P(a, x), Q(a, x))`, `a > 0`,
/// `x ≥ 0`. `None` for invalid input or when the iteration cap is hit.
pub(crate) fn gamma_pq(a: f64, x: f64) -> Option<(f64, f64)> {
    if a.is_nan() || a <= 0.0 || a.is_infinite() || x.is_nan() || x < 0.0 {
        return None;
    }
    if x == 0.0 {
        return Some((0.0, 1.0));
    }
    if x.is_infinite() {
        return Some((1.0, 0.0));
    }
    let front = ln_gamma_prefix(a, x).exp();
    let cap = iteration_cap(a.max(x));
    if x < a + 1.0 {
        // P = front · Σₙ xⁿ / (a(a+1)…(a+n))
        let mut ap = a;
        let mut term = 1.0 / a;
        let mut sum = term;
        let mut converged = false;
        for _ in 0..cap {
            ap += 1.0;
            term *= x / ap;
            sum += term;
            if term.abs() <= sum.abs() * EPS {
                converged = true;
                break;
            }
        }
        if !converged {
            return None;
        }
        let p = (front * sum).clamp(0.0, 1.0);
        return Some((p, 1.0 - p));
    }
    // Q = front · 1/(x+1-a - 1(1-a)/(x+3-a - 2(2-a)/(x+5-a - …)))
    let mut b = x + 1.0 - a;
    let mut c = 1.0 / TINY;
    let mut d = 1.0 / b;
    let mut h = d;
    let mut converged = false;
    for n in 1..cap {
        let n = n as f64;
        let an = -n * (n - a);
        b += 2.0;
        d = an * d + b;
        if d.abs() < TINY {
            d = TINY;
        }
        c = b + an / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let delta = d * c;
        h *= delta;
        if (delta - 1.0).abs() <= EPS {
            converged = true;
            break;
        }
    }
    if !converged {
        return None;
    }
    let q = (front * h).clamp(0.0, 1.0);
    Some((1.0 - q, q))
}

/// `ln(xᵃ yᵇ / B(a, b))` with `y = 1 - x` given separately.
pub(crate) fn ln_beta_prefix(a: f64, b: f64, x: f64, y: f64) -> f64 {
    if a < 10.0 || b < 10.0 {
        let lx = if x <= 0.5 { x.ln() } else { (-y).ln_1p() };
        let ly = if y <= 0.5 { y.ln() } else { (-x).ln_1p() };
        return a * lx + b * ly - ln_beta(a, b);
    }
    // a ln(x/x₀) + b ln(y/y₀) with x₀ = a/(a+b): the linear terms cancel
    // exactly, leaving a·(ln(1+u) - u) + b·(ln(1+v) - v).
    let s = a + b;
    let d = b * x - a * y;
    a * log1pmx(d / a) + b * log1pmx(-d / b) + 0.5 * (a * b / s).ln()
        - HALF_LN_2PI
        - stirling_tail(a)
        - stirling_tail(b)
        + stirling_tail(s)
}

/// Continued fraction of `I_x(a, b)` (Lentz); `None` if it does not
/// converge within the cap.
fn beta_cf(a: f64, b: f64, x: f64) -> Option<f64> {
    let s = a + b;
    let mut c = 1.0;
    let mut d = 1.0 - s * x / (a + 1.0);
    if d.abs() < TINY {
        d = TINY;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..iteration_cap(a.max(b)) {
        let m = m as f64;
        let m2 = 2.0 * m;
        // even step: m(b-m)x / ((a+2m-1)(a+2m))
        let num = m * (b - m) * x / ((a + m2 - 1.0) * (a + m2));
        d = 1.0 + num * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + num / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        h *= d * c;
        // odd step: -(a+m)(a+b+m)x / ((a+2m)(a+2m+1))
        let num = -(a + m) * (s + m) * x / ((a + m2) * (a + m2 + 1.0));
        d = 1.0 + num * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + num / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let delta = d * c;
        h *= delta;
        if (delta - 1.0).abs() <= EPS {
            return Some(h);
        }
    }
    None
}

/// Regularized incomplete beta `(I_x(a, b), 1 - I_x(a, b))` for `a, b > 0`
/// and `x ∈ [0, 1]`, with `y = 1 - x` supplied by the caller (computed
/// without cancellation where possible).
pub(crate) fn beta_reg(a: f64, b: f64, x: f64, y: f64) -> Option<(f64, f64)> {
    if !(a > 0.0 && b > 0.0) || !a.is_finite() || !b.is_finite() || x.is_nan() || y.is_nan() {
        return None;
    }
    if x <= 0.0 {
        return Some((0.0, 1.0));
    }
    if y <= 0.0 {
        return Some((1.0, 0.0));
    }
    let swap = x > (a + 1.0) / (a + b + 2.0);
    let (a, b, x, y) = if swap { (b, a, y, x) } else { (a, b, x, y) };
    let front = (ln_beta_prefix(a, b, x, y) - a.ln()).exp();
    let v = (front * beta_cf(a, b, x)?).clamp(0.0, 1.0);
    Some(if swap { (1.0 - v, v) } else { (v, 1.0 - v) })
}

/// Standard normal density.
pub(crate) fn norm_pdf(z: f64) -> f64 {
    (-0.5 * z * z).exp() / SQRT_2PI
}

/// Standard normal CDF `Φ(z)` through `P(½, z²/2)`/`Q(½, z²/2)`.
pub(crate) fn norm_cdf(z: f64) -> f64 {
    if z.is_nan() {
        return f64::NAN;
    }
    let x = 0.5 * z * z;
    match gamma_pq(0.5, x) {
        Some((p, _)) if x < 1.5 => {
            if z < 0.0 {
                0.5 - 0.5 * p
            } else {
                0.5 + 0.5 * p
            }
        }
        Some((_, q)) => {
            if z < 0.0 {
                0.5 * q
            } else {
                1.0 - 0.5 * q
            }
        }
        None => {
            if z < 0.0 {
                0.0
            } else {
                1.0
            }
        }
    }
}

/// Lower-tail standard normal quantile for `0 < p ≤ 0.5`.
fn norm_inv_lower(p: f64) -> Option<f64> {
    // Starting point: the tail asymptote x² ≈ t² - ln(2π) - 2 ln t with
    // t² = -2 ln p, or the linear term of Φ near 0.
    let t2 = -2.0 * p.ln();
    let tail = t2 - (2.0 * std::f64::consts::PI).ln() - t2.sqrt().ln() * 2.0;
    let mut x = if p < 0.1 && tail > 0.0 {
        -tail.sqrt()
    } else {
        (p - 0.5) * SQRT_2PI
    };
    for _ in 0..100 {
        let pdf = norm_pdf(x);
        if pdf.is_nan() || pdf <= 0.0 {
            break;
        }
        let u = (norm_cdf(x) - p) / pdf;
        let step = u / (1.0 + 0.5 * x * u);
        if !step.is_finite() {
            break;
        }
        x -= step;
        if step.abs() <= 1.0e-15 * x.abs().max(1.0) {
            return Some(x);
        }
    }
    // Fallback: bisection on the increasing CDF.
    solve(
        |x| Some((norm_cdf(x) - p, norm_pdf(x))),
        -40.0,
        0.0,
        x.clamp(-40.0, 0.0),
        true,
    )
}

/// Standard normal quantile `Φ⁻¹(p)` for `0 < p < 1`.
pub(crate) fn norm_inv(p: f64) -> Option<f64> {
    if !(p > 0.0 && p < 1.0) {
        return None;
    }
    if p <= 0.5 {
        norm_inv_lower(p)
    } else {
        norm_inv_lower(1.0 - p).map(|x| -x)
    }
}

/// Midpoint for bisection: geometric on wide positive brackets so that
/// small roots are reached in few steps.
fn midpoint(lo: f64, hi: f64) -> f64 {
    if lo > 0.0 && hi > 4.0 * lo {
        (lo * hi).sqrt()
    } else if lo == 0.0 && hi > 1.0e-300 {
        hi / 16.0
    } else {
        lo + 0.5 * (hi - lo)
    }
}

/// Root of a monotone function on `[lo, hi]` (`increasing` tells its
/// direction); `f` returns the value and the derivative. Newton steps from
/// `x0` are taken while they stay inside the bracket and shrink fast
/// enough, bisection steps otherwise. `None` if `f` fails or the root is
/// not reached within the iteration limit.
pub(crate) fn solve(
    f: impl Fn(f64) -> Option<(f64, f64)>,
    mut lo: f64,
    mut hi: f64,
    x0: f64,
    increasing: bool,
) -> Option<f64> {
    if !(lo.is_finite() && hi.is_finite() && lo <= hi) {
        return None;
    }
    let mut x = if x0 > lo && x0 < hi {
        x0
    } else {
        midpoint(lo, hi)
    };
    let mut step = f64::INFINITY;
    let mut prev_step = f64::INFINITY;
    for _ in 0..500 {
        let (fx, dfx) = f(x)?;
        if fx.is_nan() {
            return None;
        }
        if fx == 0.0 {
            return Some(x);
        }
        if (fx < 0.0) == increasing {
            lo = x;
        } else {
            hi = x;
        }
        let newton = x - fx / dfx;
        // Bisect when Newton leaves the bracket or does not at least halve
        // the step before last (slow, e.g. doubling in a 1/x tail).
        let use_newton = dfx != 0.0
            && newton.is_finite()
            && newton > lo
            && newton < hi
            && (newton - x).abs() * 2.0 <= prev_step;
        let next = if use_newton { newton } else { midpoint(lo, hi) };
        prev_step = step;
        step = (next - x).abs();
        let tol = 4.0 * EPS * next.abs().max(f64::MIN_POSITIVE);
        if step <= tol || hi - lo <= tol {
            return Some(next);
        }
        x = next;
    }
    None
}

/// Doubles `start` until `beyond(x)` holds (at most 2,100 times); the upper
/// end of a bracket on `[0, ∞)`.
pub(crate) fn expand_upper(start: f64, beyond: impl Fn(f64) -> Option<bool>) -> Option<f64> {
    let mut x = start.max(1.0);
    for _ in 0..2_100 {
        if beyond(x)? {
            return Some(x);
        }
        x *= 2.0;
        if !x.is_finite() {
            return None;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn close(a: f64, b: f64, rel: f64) -> bool {
        (a - b).abs() <= rel * b.abs().max(1e-300)
    }

    #[test]
    fn ln_gamma_matches_factorials_and_half_integers() {
        let mut fact = 1.0f64;
        for n in 1..60 {
            // Γ(n + 1) = n!
            fact *= f64::from(n);
            let got = ln_gamma(f64::from(n) + 1.0);
            assert!((got - fact.ln()).abs() <= 1e-13 * fact.ln().max(1.0), "{n}");
        }
        let sqrt_pi_ln = 0.5 * std::f64::consts::PI.ln();
        assert!((ln_gamma(0.5) - sqrt_pi_ln).abs() < 4e-15);
        assert!(ln_gamma(1.0).abs() < 4e-15 && ln_gamma(2.0).abs() < 4e-15);
        assert!(ln_gamma(0.0).is_nan() && ln_gamma(-1.0).is_nan());
        // Large argument: compare the ratio form with the direct one.
        let r = ln_gamma_ratio(1.0e6, 0.5);
        assert!(close(r, ln_gamma(1.0e6) - ln_gamma(1.0e6 + 0.5), 1e-9));
    }

    #[test]
    fn incomplete_gamma_matches_closed_forms() {
        // Integer a: Q(k, x) = e^-x Σ_{j<k} x^j / j!, P(k, x) = e^-x Σ_{j≥k} x^j / j!
        for &(k, x) in &[
            (1u32, 0.3),
            (2, 1.0),
            (3, 2.5),
            (5, 4.0),
            (8, 20.0),
            (12, 3.0),
        ] {
            let mut term = 1.0;
            let (mut below, mut above) = (0.0, 0.0);
            for j in 0..400u32 {
                if j > 0 {
                    term *= x / f64::from(j);
                }
                if j < k {
                    below += term;
                } else {
                    above += term;
                }
            }
            let (q, p) = ((-x).exp() * below, (-x).exp() * above);
            let (p_got, q_got) = gamma_pq(f64::from(k), x).unwrap();
            assert!(close(q_got, q, 1e-13), "Q({k},{x}) {q_got} vs {q}");
            assert!(close(p_got, p, 1e-13), "P({k},{x}) {p_got} vs {p}");
        }
        assert_eq!(gamma_pq(2.0, 0.0), Some((0.0, 1.0)));
        assert!(gamma_pq(0.0, 1.0).is_none() && gamma_pq(1.0, -1.0).is_none());
        // Large a near the centre converges.
        let (p, q) = gamma_pq(5.0e5, 5.0e5).unwrap();
        assert!(p > 0.49 && p < 0.51 && close(p + q, 1.0, 1e-15));
    }

    #[test]
    fn incomplete_beta_matches_binomial_sums() {
        // I_x(a, b) = P(Binomial(a + b - 1, x) ≥ a) for integer a, b.
        let binom_tail = |a: u32, b: u32, x: f64| {
            let n = a + b - 1;
            let mut total = 0.0;
            for j in a..=n {
                let mut c = 1.0;
                for i in 0..j {
                    c = c * f64::from(n - i) / f64::from(i + 1);
                }
                total += c * x.powi(j as i32) * (1.0 - x).powi((n - j) as i32);
            }
            total
        };
        for &(a, b, x) in &[
            (2u32, 3u32, 0.5),
            (8, 10, 0.5),
            (1, 1, 0.3),
            (5, 2, 0.9),
            (12, 15, 0.2),
        ] {
            let (i, c) = beta_reg(f64::from(a), f64::from(b), x, 1.0 - x).unwrap();
            let want = binom_tail(a, b, x);
            assert!(close(i, want, 1e-13), "I_{x}({a},{b}) = {i} vs {want}");
            assert!(close(c, 1.0 - want, 1e-12));
        }
        assert_eq!(beta_reg(2.0, 3.0, 0.0, 1.0), Some((0.0, 1.0)));
        assert!(beta_reg(0.0, 1.0, 0.5, 0.5).is_none());
        // Both parameters large: symmetric case is exactly 1/2.
        let (i, _) = beta_reg(2.0e5, 2.0e5, 0.5, 0.5).unwrap();
        assert!(close(i, 0.5, 1e-12), "{i}");
    }

    #[test]
    fn normal_cdf_and_inverse() {
        assert!(close(norm_cdf(1.96), 0.975_002_104_851_779_5, 1e-15));
        assert!(close(norm_cdf(0.0), 0.5, 1e-16));
        assert!(close(norm_cdf(-1.0), 0.158_655_253_931_457_05, 1e-14));
        for &p in &[
            1e-300,
            1e-20,
            1e-5,
            0.025,
            0.3,
            0.5,
            0.7,
            0.975,
            1.0 - 1e-12,
        ] {
            let x = norm_inv(p).unwrap();
            let back = norm_cdf(x);
            let rel = if p < 0.5 { 1e-13 } else { 1e-15 };
            assert!(close(back, p, rel), "p = {p}: x = {x}, Φ(x) = {back}");
        }
        assert!(close(
            norm_inv(0.975).unwrap(),
            1.959_963_984_540_054,
            1e-14
        ));
        assert!(norm_inv(0.0).is_none() && norm_inv(1.0).is_none());
    }

    #[test]
    fn solver_handles_tiny_roots_and_brackets() {
        // x³ = 1e-30 → x = 1e-10, starting far away.
        let r = solve(
            |x| Some((x * x * x - 1e-30, 3.0 * x * x)),
            0.0,
            10.0,
            5.0,
            true,
        )
        .unwrap();
        assert!(close(r, 1e-10, 1e-12), "{r}");
        let up = expand_upper(1.0, |x| Some(x > 1000.0)).unwrap();
        assert!(up > 1000.0);
        assert!(expand_upper(1.0, |_| Some(false)).is_none());
    }
}
