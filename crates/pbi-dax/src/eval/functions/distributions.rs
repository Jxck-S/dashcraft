//! Probability distributions: `NORM.*`, `EXPON.DIST`, `POISSON.DIST`,
//! `BETA.*`, `CHISQ.*`, `T.*` and `CONFIDENCE.*`.
//!
//! Argument rules follow the reference (and Excel, which these functions
//! emulate): degrees of freedom and Poisson counts are truncated to
//! integers, probabilities outside the documented range, non-positive
//! scales and similar are "invalid value" errors, BLANK counts as 0 (FALSE
//! for `cumulative`). Shape parameters (degrees of freedom, `alpha`/`beta`,
//! Poisson `x`/`mean`) are limited to 10¹⁰ so every evaluation stays
//! bounded; results are always finite `Double`s.
//!
//! The numerics live in [`super::special`]: regularized incomplete gamma
//! and beta functions (series and Lentz continued fractions) and
//! Newton/Halley inverses kept inside a bisection bracket, with a relative
//! accuracy of about 1e-13 (tested against closed forms to 1e-12).

use super::math::{R, flag, invalid, num, num_or, real, too_large};
use super::special::{
    beta_reg, expand_upper, gamma_pq, ln_beta, ln_beta_prefix, ln_gamma, ln_gamma_prefix,
    ln_gamma_ratio, norm_cdf, norm_inv, norm_pdf, solve,
};
use super::{Builtin, arity, scalar_fn};
use crate::eval::context::Env;
use crate::eval::error::{DaxError, Result};
use crate::eval::interp::Interp;
use crate::syntax::Expr;

pub(super) const BUILTINS: &[Builtin] = &[
    scalar_fn("NORM.DIST", norm_dist),
    scalar_fn("NORM.INV", norm_inv_fn),
    scalar_fn("NORM.S.DIST", norm_s_dist),
    scalar_fn("NORM.S.INV", norm_s_inv),
    scalar_fn("EXPON.DIST", expon_dist),
    scalar_fn("POISSON.DIST", poisson_dist),
    scalar_fn("BETA.DIST", beta_dist),
    scalar_fn("BETA.INV", beta_inv),
    scalar_fn("CHISQ.DIST", chisq_dist),
    scalar_fn("CHISQ.DIST.RT", chisq_dist_rt),
    scalar_fn("CHISQ.INV", chisq_inv),
    scalar_fn("CHISQ.INV.RT", chisq_inv_rt),
    scalar_fn("T.DIST", t_dist),
    scalar_fn("T.DIST.2T", t_dist_2t),
    scalar_fn("T.DIST.RT", t_dist_rt),
    scalar_fn("T.INV", t_inv),
    scalar_fn("T.INV.2T", t_inv_2t),
    scalar_fn("CONFIDENCE.NORM", confidence_norm),
    scalar_fn("CONFIDENCE.T", confidence_t),
];

/// Largest shape parameter accepted (degrees of freedom, alpha/beta,
/// Poisson x/mean).
const MAX_SHAPE: f64 = 1.0e10;

fn not_converged(f: &str) -> DaxError {
    DaxError::argument(f, "the calculation did not converge")
}

/// Unwraps a numeric result that is `None` when the series did not converge.
fn conv(f: &str, x: Option<f64>) -> R {
    match x {
        Some(x) => real(f, x),
        None => Err(not_converged(f)),
    }
}

/// Degrees of freedom: truncated, 1 to 10¹⁰.
fn dof(i: &Interp<'_>, env: &Env, f: &str, args: &[Expr], n: usize) -> Result<f64> {
    let df = num(i, env, f, args, n)?.trunc();
    if (1.0..=MAX_SHAPE).contains(&df) {
        Ok(df)
    } else {
        Err(invalid(
            f,
            "the degrees of freedom must be between 1 and 10^10",
        ))
    }
}

/// A probability in `(0, 1)` (`one_ok`: `(0, 1]`).
fn prob(i: &Interp<'_>, env: &Env, f: &str, args: &[Expr], n: usize, one_ok: bool) -> Result<f64> {
    let p = num(i, env, f, args, n)?;
    let valid = p > 0.0 && (p < 1.0 || (one_ok && p == 1.0));
    if valid {
        Ok(p)
    } else if one_ok {
        Err(invalid(
            f,
            "the probability must be greater than 0 and at most 1",
        ))
    } else {
        Err(invalid(
            f,
            "the probability must be between 0 and 1 (exclusive)",
        ))
    }
}

/// A strictly positive argument (standard deviations, rates).
fn positive_arg(
    i: &Interp<'_>,
    env: &Env,
    f: &str,
    args: &[Expr],
    n: usize,
    what: &str,
) -> Result<f64> {
    let x = num(i, env, f, args, n)?;
    if x > 0.0 && x.is_finite() {
        Ok(x)
    } else {
        Err(invalid(f, &format!("{what} must be greater than 0")))
    }
}

// ---- normal -----------------------------------------------------------------------

fn norm_dist(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    const F: &str = "NORM.DIST";
    arity(F, args, 4, 4)?;
    let x = num(i, env, F, args, 0)?;
    let mean = num(i, env, F, args, 1)?;
    let sd = positive_arg(i, env, F, args, 2, "the standard deviation")?;
    let z = (x - mean) / sd;
    if flag(i, env, F, args, 3)? {
        real(F, norm_cdf(z))
    } else {
        real(F, norm_pdf(z) / sd)
    }
}

fn norm_inv_fn(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    const F: &str = "NORM.INV";
    arity(F, args, 3, 3)?;
    let p = prob(i, env, F, args, 0, false)?;
    let mean = num(i, env, F, args, 1)?;
    let sd = positive_arg(i, env, F, args, 2, "the standard deviation")?;
    conv(F, norm_inv(p).map(|z| mean + sd * z))
}

fn norm_s_dist(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    const F: &str = "NORM.S.DIST";
    arity(F, args, 2, 2)?;
    let z = num(i, env, F, args, 0)?;
    if flag(i, env, F, args, 1)? {
        real(F, norm_cdf(z))
    } else {
        real(F, norm_pdf(z))
    }
}

fn norm_s_inv(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    const F: &str = "NORM.S.INV";
    arity(F, args, 1, 1)?;
    let p = prob(i, env, F, args, 0, false)?;
    conv(F, norm_inv(p))
}

// ---- exponential and Poisson -----------------------------------------------------

fn expon_dist(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    const F: &str = "EXPON.DIST";
    arity(F, args, 3, 3)?;
    let x = num(i, env, F, args, 0)?;
    let lambda = positive_arg(i, env, F, args, 1, "lambda")?;
    if x.is_nan() || x < 0.0 {
        return Err(invalid(F, "x cannot be negative"));
    }
    if flag(i, env, F, args, 2)? {
        real(F, -(-lambda * x).exp_m1())
    } else {
        real(F, lambda * (-lambda * x).exp())
    }
}

fn poisson_dist(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    const F: &str = "POISSON.DIST";
    arity(F, args, 3, 3)?;
    let x = num(i, env, F, args, 0)?.trunc();
    let mean = num(i, env, F, args, 1)?;
    if x.is_nan() || mean.is_nan() || x < 0.0 || mean < 0.0 {
        return Err(invalid(F, "x and mean cannot be negative"));
    }
    if x > MAX_SHAPE || mean > MAX_SHAPE {
        return Err(invalid(F, "x and mean must be at most 10^10"));
    }
    let cumulative = flag(i, env, F, args, 2)?;
    if mean == 0.0 {
        let v = if cumulative || x == 0.0 { 1.0 } else { 0.0 };
        return real(F, v);
    }
    if cumulative {
        // P(X ≤ x) = Q(x + 1, mean)
        conv(F, gamma_pq(x + 1.0, mean).map(|(_, q)| q))
    } else {
        // e^-m m^x / x! = prefix(x + 1, m) / m, without large cancelling logs.
        real(F, (ln_gamma_prefix(x + 1.0, mean) - mean.ln()).exp())
    }
}

// ---- beta ---------------------------------------------------------------------------

/// `alpha`, `beta` (positive, at most 10¹⁰) and the interval `[A, B]`
/// (arguments `n`, `n + 1`; default `[0, 1]`).
fn beta_params(
    i: &Interp<'_>,
    env: &Env,
    f: &str,
    args: &[Expr],
    n: usize,
) -> Result<(f64, f64, f64, f64)> {
    let a = num(i, env, f, args, 1)?;
    let b = num(i, env, f, args, 2)?;
    if !(a > 0.0 && b > 0.0) {
        return Err(invalid(f, "alpha and beta must be greater than 0"));
    }
    if a > MAX_SHAPE || b > MAX_SHAPE {
        return Err(invalid(f, "alpha and beta must be at most 10^10"));
    }
    let lo = num_or(i, env, args, n, 0.0)?;
    let hi = num_or(i, env, args, n + 1, 1.0)?;
    if lo.is_nan() || hi.is_nan() || lo >= hi || !(hi - lo).is_finite() {
        return Err(invalid(
            f,
            "the lower bound must be less than the upper bound",
        ));
    }
    Ok((a, b, lo, hi))
}

/// `k · ln(x)` with `0 · ln(0) = 0`.
fn xlogy(k: f64, x: f64) -> f64 {
    if k == 0.0 { 0.0 } else { k * x.ln() }
}

fn beta_dist(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    const F: &str = "BETA.DIST";
    arity(F, args, 4, 6)?;
    let x = num(i, env, F, args, 0)?;
    let (a, b, lo, hi) = beta_params(i, env, F, args, 4)?;
    if x.is_nan() || x < lo || x > hi {
        return Err(invalid(F, "x must be between the lower and upper bounds"));
    }
    let width = hi - lo;
    let z = (x - lo) / width;
    let y = (hi - x) / width;
    if flag(i, env, F, args, 3)? {
        conv(F, beta_reg(a, b, z, y).map(|(p, _)| p))
    } else {
        real(F, beta_density(a, b, z, y) / width)
    }
}

/// Density of the standard beta distribution at `z` (`y = 1 - z`).
fn beta_density(a: f64, b: f64, z: f64, y: f64) -> f64 {
    if z <= 0.0 || y <= 0.0 {
        return (xlogy(a - 1.0, z) + xlogy(b - 1.0, y) - ln_beta(a, b)).exp();
    }
    let lz = if z <= 0.5 { z.ln() } else { (-y).ln_1p() };
    let ly = if y <= 0.5 { y.ln() } else { (-z).ln_1p() };
    (ln_beta_prefix(a, b, z, y) - lz - ly).exp()
}

fn beta_inv(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    const F: &str = "BETA.INV";
    arity(F, args, 3, 5)?;
    let p = prob(i, env, F, args, 0, true)?;
    let (a, b, lo, hi) = beta_params(i, env, F, args, 3)?;
    if p == 1.0 {
        return real(F, hi);
    }
    // Solve on the tail that holds the probability, for accuracy.
    let lower = p <= 0.5;
    let target = if lower { p } else { 1.0 - p };
    let z = solve(
        |z| {
            let (cdf, sf) = beta_reg(a, b, z, 1.0 - z)?;
            let v = if lower { cdf - target } else { sf - target };
            let d = beta_density(a, b, z, 1.0 - z);
            Some((v, if lower { d } else { -d }))
        },
        0.0,
        1.0,
        a / (a + b),
        lower,
    );
    conv(F, z.map(|z| lo + (hi - lo) * z))
}

// ---- chi-squared -------------------------------------------------------------------

fn chisq_x(i: &Interp<'_>, env: &Env, f: &str, args: &[Expr]) -> Result<f64> {
    let x = num(i, env, f, args, 0)?;
    if x.is_nan() || x < 0.0 {
        Err(invalid(f, "x cannot be negative"))
    } else {
        Ok(x)
    }
}

/// `x^(k-1) e^(-x/2) / (2^k Γ(k))` with `k = df/2`, as
/// `prefix(k, x/2) / x`.
fn chisq_density(x: f64, df: f64) -> f64 {
    if x == 0.0 {
        return if df < 2.0 {
            f64::INFINITY
        } else if df == 2.0 {
            0.5
        } else {
            0.0
        };
    }
    ln_gamma_prefix(df / 2.0, x / 2.0).exp() / x
}

fn chisq_dist(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    const F: &str = "CHISQ.DIST";
    arity(F, args, 3, 3)?;
    let x = chisq_x(i, env, F, args)?;
    let df = dof(i, env, F, args, 1)?;
    if flag(i, env, F, args, 2)? {
        conv(F, gamma_pq(df / 2.0, x / 2.0).map(|(p, _)| p))
    } else {
        real(F, chisq_density(x, df))
    }
}

fn chisq_dist_rt(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    const F: &str = "CHISQ.DIST.RT";
    arity(F, args, 2, 2)?;
    let x = chisq_x(i, env, F, args)?;
    let df = dof(i, env, F, args, 1)?;
    conv(F, gamma_pq(df / 2.0, x / 2.0).map(|(_, q)| q))
}

/// The chi-squared quantile with lower tail `p` (upper tail `q = 1 - p`,
/// passed separately so either can be tiny).
fn chisq_quantile(p: f64, q: f64, df: f64) -> Option<f64> {
    let k = df / 2.0;
    let lower = p <= 0.5;
    // Wilson–Hilferty start, or the small-x form P ≈ (x/2)^k / Γ(k+1).
    let z = if lower {
        norm_inv(p)
    } else {
        norm_inv(q).map(|z| -z)
    }?;
    let c = 2.0 / (9.0 * df);
    let wh = df * (1.0 - c + z * c.sqrt()).powi(3);
    let x0 = if wh.is_finite() && wh > 0.0 {
        wh
    } else {
        2.0 * ((p.ln() + ln_gamma(k + 1.0)) / k).exp()
    };
    let tail = move |x: f64| -> Option<f64> {
        let (pl, pu) = gamma_pq(k, x / 2.0)?;
        Some(if lower { pl - p } else { pu - q })
    };
    let hi = expand_upper(2.0 * x0.max(df), |x| {
        Some(tail(x)? * if lower { 1.0 } else { -1.0 } > 0.0)
    })?;
    solve(
        |x| {
            let d = chisq_density(x, df);
            Some((tail(x)?, if lower { d } else { -d }))
        },
        0.0,
        hi,
        x0,
        lower,
    )
}

fn chisq_inv(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    const F: &str = "CHISQ.INV";
    arity(F, args, 2, 2)?;
    let p = num(i, env, F, args, 0)?;
    let df = dof(i, env, F, args, 1)?;
    if p.is_nan() || !(0.0..1.0).contains(&p) {
        return Err(invalid(
            F,
            "the probability must be at least 0 and less than 1",
        ));
    }
    if p == 0.0 {
        return real(F, 0.0);
    }
    conv(F, chisq_quantile(p, 1.0 - p, df))
}

fn chisq_inv_rt(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    const F: &str = "CHISQ.INV.RT";
    arity(F, args, 2, 2)?;
    let q = prob(i, env, F, args, 0, true)?;
    let df = dof(i, env, F, args, 1)?;
    if q == 1.0 {
        return real(F, 0.0);
    }
    conv(F, chisq_quantile(1.0 - q, q, df))
}

// ---- Student's t ---------------------------------------------------------------------

/// `ln(1 + r²)` without overflow.
fn ln1p_sq(r: f64) -> f64 {
    if r.abs() > 1.0e150 {
        2.0 * r.abs().ln()
    } else {
        (r * r).ln_1p()
    }
}

/// `P(T > t)` for `t ≥ 0`: `½ I_{ν/(ν+t²)}(ν/2, ½)`.
fn t_upper(t: f64, df: f64) -> Option<f64> {
    let r = t / df.sqrt();
    let r2 = r * r;
    let (x, y) = if r2.is_infinite() {
        (0.0, 1.0)
    } else {
        (1.0 / (1.0 + r2), r2 / (1.0 + r2))
    };
    beta_reg(df / 2.0, 0.5, x, y).map(|(i, _)| 0.5 * i)
}

fn t_cdf(t: f64, df: f64) -> Option<f64> {
    if t < 0.0 {
        t_upper(-t, df)
    } else {
        t_upper(t, df).map(|u| 1.0 - u)
    }
}

fn t_density(t: f64, df: f64) -> f64 {
    let ln_norm = -ln_gamma_ratio(df / 2.0, 0.5) - 0.5 * (df * std::f64::consts::PI).ln();
    (ln_norm - (df + 1.0) / 2.0 * ln1p_sq(t / df.sqrt())).exp()
}

/// The `y > 0` with `P(T > y) = q`, `0 < q < ½`.
fn t_upper_inv(q: f64, df: f64) -> Option<f64> {
    let z = -norm_inv(q)?;
    // Normal quantile with the first Cornish–Fisher correction.
    let guess = z + (z * z * z + z) / (4.0 * df);
    let hi = expand_upper(2.0 * guess.max(1.0), |y| Some(t_upper(y, df)? < q))?;
    solve(
        |y| Some((t_upper(y, df)? - q, -t_density(y, df))),
        0.0,
        hi,
        guess,
        false,
    )
}

fn t_dist(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    const F: &str = "T.DIST";
    arity(F, args, 3, 3)?;
    let x = num(i, env, F, args, 0)?;
    let df = dof(i, env, F, args, 1)?;
    if x.is_nan() {
        return Err(too_large(F));
    }
    if flag(i, env, F, args, 2)? {
        conv(F, t_cdf(x, df))
    } else {
        real(F, t_density(x, df))
    }
}

fn t_dist_2t(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    const F: &str = "T.DIST.2T";
    arity(F, args, 2, 2)?;
    let x = num(i, env, F, args, 0)?;
    let df = dof(i, env, F, args, 1)?;
    if x.is_nan() || x < 0.0 {
        return Err(invalid(F, "x cannot be negative"));
    }
    conv(F, t_upper(x, df).map(|u| 2.0 * u))
}

fn t_dist_rt(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    const F: &str = "T.DIST.RT";
    arity(F, args, 2, 2)?;
    let x = num(i, env, F, args, 0)?;
    let df = dof(i, env, F, args, 1)?;
    if x.is_nan() {
        return Err(too_large(F));
    }
    conv(F, t_cdf(-x, df))
}

fn t_inv(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    const F: &str = "T.INV";
    arity(F, args, 2, 2)?;
    let p = prob(i, env, F, args, 0, false)?;
    let df = dof(i, env, F, args, 1)?;
    if p == 0.5 {
        return real(F, 0.0);
    }
    if p < 0.5 {
        conv(F, t_upper_inv(p, df).map(|y| -y))
    } else {
        conv(F, t_upper_inv(1.0 - p, df))
    }
}

/// Two-tailed quantile: `y` with `P(|T| > y) = p`.
fn t_two_tailed(p: f64, df: f64) -> Option<f64> {
    if p == 1.0 {
        return Some(0.0);
    }
    t_upper_inv(p / 2.0, df)
}

fn t_inv_2t(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    const F: &str = "T.INV.2T";
    arity(F, args, 2, 2)?;
    let p = prob(i, env, F, args, 0, true)?;
    let df = dof(i, env, F, args, 1)?;
    conv(F, t_two_tailed(p, df))
}

// ---- confidence intervals -----------------------------------------------------------

/// `(alpha, standard_dev, size)` of `CONFIDENCE.*` (`size` truncated).
fn confidence_args(
    i: &Interp<'_>,
    env: &Env,
    f: &str,
    args: &[Expr],
    min_size: f64,
) -> Result<(f64, f64, f64)> {
    arity(f, args, 3, 3)?;
    let alpha = prob(i, env, f, args, 0, false)?;
    let sd = positive_arg(i, env, f, args, 1, "the standard deviation")?;
    let n = num(i, env, f, args, 2)?.trunc();
    if n.is_nan() || n < min_size || n > MAX_SHAPE {
        return Err(invalid(
            f,
            &format!("the size must be between {min_size} and 10^10"),
        ));
    }
    Ok((alpha, sd, n))
}

fn confidence_norm(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    const F: &str = "CONFIDENCE.NORM";
    let (alpha, sd, n) = confidence_args(i, env, F, args, 1.0)?;
    conv(F, norm_inv(alpha / 2.0).map(|z| -z * sd / n.sqrt()))
}

fn confidence_t(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    const F: &str = "CONFIDENCE.T";
    let (alpha, sd, n) = confidence_args(i, env, F, args, 2.0)?;
    conv(F, t_two_tailed(alpha, n - 1.0).map(|t| t * sd / n.sqrt()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn close(a: f64, b: f64, rel: f64) -> bool {
        (a - b).abs() <= rel * b.abs()
    }

    #[test]
    fn t_matches_closed_forms() {
        let pi = std::f64::consts::PI;
        for &t in &[-30.0, -2.5, -0.3, 0.0, 0.1, 1.0, 7.0, 1e6] {
            // ν = 1 (Cauchy) and ν = 2.
            let c1 = 0.5 + f64::atan(t) / pi;
            assert!(close(t_cdf(t, 1.0).unwrap(), c1, 1e-13), "ν=1 t={t}");
            let c2 = 0.5 + t / (2.0 * (2.0 + t * t).sqrt());
            assert!(close(t_cdf(t, 2.0).unwrap(), c2, 1e-13), "ν=2 t={t}");
            let d1 = 1.0 / (pi * (1.0 + t * t));
            assert!(close(t_density(t, 1.0), d1, 1e-13));
        }
        // ν = 10 is even: P(T > t) = (1 - √(1-x) Σ_{k<5} C(2k,k)/4^k xᵏ) / 2
        // with x = ν/(ν+t²); its 2.5% point is 2.228138851986274 (an
        // often-quoted older approximation, 2.2281388519649385, is 1e-11 off).
        let y = 2.228_138_851_986_274f64;
        assert!(close(t_two_tailed(0.05, 10.0).unwrap(), y, 1e-13));
        assert!(close(
            t_two_tailed(0.05, 10.0).unwrap(),
            2.228_138_851_964_938_5,
            1e-10
        ));
        let x = 10.0 / (10.0 + y * y);
        let (mut c, mut sum) = (1.0, 0.0);
        for k in 0..5 {
            if k > 0 {
                c *= (2.0 * f64::from(k) - 1.0) / (2.0 * f64::from(k));
            }
            sum += c * x.powi(k);
        }
        let upper = (1.0 - (1.0 - x).sqrt() * sum) / 2.0;
        assert!(close(t_upper(y, 10.0).unwrap(), upper, 1e-12));
        assert!(close(
            t_upper_inv(0.25, 2.0).unwrap(),
            (2.0f64 / 3.0).sqrt(),
            1e-13
        ));
        // Huge degrees of freedom approach the normal.
        let t = t_upper_inv(0.025, 1e9).unwrap();
        assert!(close(t, 1.959_963_984_540_054, 1e-8), "{t}");
    }

    #[test]
    fn chisq_matches_closed_forms() {
        // ν = 2: P = 1 - e^{-x/2}; quantile -2 ln(1 - p).
        for &x in &[0.01, 0.5, 3.0, 40.0] {
            let (p, q) = gamma_pq(1.0, x / 2.0).unwrap();
            assert!(close(p, -(-x / 2.0f64).exp_m1(), 1e-14));
            assert!(close(q, (-x / 2.0f64).exp(), 1e-14));
        }
        for &p in &[1e-12, 0.05, 0.6, 0.99] {
            let x = chisq_quantile(p, 1.0 - p, 2.0).unwrap();
            assert!(close(x, -2.0 * (-p).ln_1p(), 1e-12), "p={p}: {x}");
        }
        let x = chisq_quantile(1.0 - 1e-15, 1e-15, 2.0).unwrap();
        assert!(close(x, -2.0 * 1e-15f64.ln(), 1e-12), "{x}");
        assert!(chisq_quantile(0.5, 0.5, 1e10).is_some());
    }
}
