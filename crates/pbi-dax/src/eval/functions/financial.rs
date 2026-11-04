//! Financial functions: `PMT`, `PV`, `FV`, and `NPER`.
//!
//! Sign conventions follow standard financial mathematics and Excel/DAX:
//! cash paid out (such as loan payments or investments) is represented by
//! negative numbers, while cash received (such as loan proceeds or returns)
//! is represented by positive numbers.
//!
//! Fundamental annuity relationship:
//!   `pv * (1 + rate)^nper + pmt * (1 + rate * type) * ((1 + rate)^nper - 1) / rate + fv = 0`
//!
//! For `rate = 0`:
//!   `pv + pmt * nper + fv = 0`

use super::math::{R, invalid, num, num_or, real, too_large};
use super::{Builtin, arity, scalar_fn};
use crate::eval::context::Env;
use crate::eval::error::Result;
use crate::eval::interp::Interp;
use crate::syntax::Expr;

pub(super) const BUILTINS: &[Builtin] = &[
    scalar_fn("PMT", pmt),
    scalar_fn("PV", pv),
    scalar_fn("FV", fv),
    scalar_fn("NPER", nper),
];

/// Rounds, validates and extracts the `type` argument:
/// - 0 or omitted: payments due at the end of the period.
/// - 1: payments due at the beginning of the period.
fn payment_type(i: &Interp<'_>, env: &Env, f: &str, args: &[Expr], n: usize) -> Result<f64> {
    let t = num_or(i, env, args, n, 0.0)?.round();
    if t == 0.0 {
        Ok(0.0)
    } else if t == 1.0 {
        Ok(1.0)
    } else {
        Err(invalid(
            f,
            "type must be 0 (end of period) or 1 (beginning of period)",
        ))
    }
}

/// `(1 + rate)^nper` and `((1 + rate)^nper - 1) / rate` for a non-zero
/// `rate > -1`. Computed through `ln_1p`/`exp_m1` so that tiny rates (where
/// `1.0 + rate == 1.0`) still approach the zero-rate limits.
fn growth(rate: f64, nper: f64) -> (f64, f64) {
    let l = nper * rate.ln_1p();
    let annuity = if l == 0.0 { nper } else { l.exp_m1() / rate };
    (l.exp(), annuity)
}

/// `PMT(rate, nper, pv[, fv][, type])`: calculates payment for a loan
/// based on constant payments and a constant interest rate.
fn pmt(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("PMT", args, 3, 5)?;
    let rate = num(i, env, "PMT", args, 0)?;
    let nper = num(i, env, "PMT", args, 1)?;
    let pv = num(i, env, "PMT", args, 2)?;
    let fv = num_or(i, env, args, 3, 0.0)?;
    let typ = payment_type(i, env, "PMT", args, 4)?;

    if !nper.is_finite() || nper < 1.0 {
        return Err(invalid("PMT", "nper must be finite and at least 1"));
    }
    if rate <= -1.0 {
        return Err(invalid("PMT", "rate must be greater than -1"));
    }

    if rate == 0.0 {
        return real("PMT", -(pv + fv) / nper);
    }

    // Discount positive-rate cash flows to avoid overflowing the growth factor.
    let (numerator, annuity) = if rate > 0.0 {
        let (discount, annuity) = growth(rate, -nper);
        (pv + fv * discount, -annuity)
    } else {
        let (factor, annuity) = growth(rate, nper);
        (fv + pv * factor, annuity)
    };
    let denom = (1.0 + rate * typ) * annuity;
    if denom == 0.0 {
        return Err(invalid("PMT", "division by zero in annuity calculation"));
    }
    real("PMT", -numerator / denom)
}

/// `PV(rate, nper, pmt[, fv][, type])`: calculates present value of an annuity.
fn pv(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("PV", args, 3, 5)?;
    let rate = num(i, env, "PV", args, 0)?;
    let nper = num(i, env, "PV", args, 1)?;
    let pmt_val = num(i, env, "PV", args, 2)?;
    let fv_val = num_or(i, env, args, 3, 0.0)?;
    let typ = payment_type(i, env, "PV", args, 4)?;

    if !nper.is_finite() || nper < 1.0 {
        return Err(invalid("PV", "nper must be finite and at least 1"));
    }
    if rate <= -1.0 {
        return Err(invalid("PV", "rate must be greater than -1"));
    }

    if rate == 0.0 {
        return real("PV", -(fv_val + pmt_val * nper));
    }

    if rate > 0.0 {
        let (discount, annuity) = growth(rate, -nper);
        let term = pmt_val * (1.0 + rate * typ) * -annuity;
        return real("PV", -(fv_val * discount + term));
    }

    let (factor, annuity) = growth(rate, nper);
    if factor == 0.0 {
        return Err(too_large("PV"));
    }
    let term = pmt_val * (1.0 + rate * typ) * annuity;
    real("PV", -(fv_val + term) / factor)
}

/// `FV(rate, nper, pmt[, pv][, type])`: calculates future value of an investment.
fn fv(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("FV", args, 3, 5)?;
    let rate = num(i, env, "FV", args, 0)?;
    let nper = num(i, env, "FV", args, 1)?;
    let pmt_val = num(i, env, "FV", args, 2)?;
    let pv_val = num_or(i, env, args, 3, 0.0)?;
    let typ = payment_type(i, env, "FV", args, 4)?;

    if !nper.is_finite() || nper < 1.0 {
        return Err(invalid("FV", "nper must be finite and at least 1"));
    }
    if rate <= -1.0 {
        return Err(invalid("FV", "rate must be greater than -1"));
    }

    if rate == 0.0 {
        return real("FV", -(pv_val + pmt_val * nper));
    }

    let (factor, annuity) = growth(rate, nper);
    let term = pmt_val * (1.0 + rate * typ) * annuity;
    real("FV", -(pv_val * factor + term))
}

/// `NPER(rate, pmt, pv[, fv][, type])`: returns the number of periods for an investment.
fn nper(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("NPER", args, 3, 5)?;
    let rate = num(i, env, "NPER", args, 0)?;
    let pmt_val = num(i, env, "NPER", args, 1)?;
    let pv_val = num(i, env, "NPER", args, 2)?;
    let fv_val = num_or(i, env, args, 3, 0.0)?;
    let typ = payment_type(i, env, "NPER", args, 4)?;

    if rate <= -1.0 {
        return Err(invalid("NPER", "rate must be greater than -1"));
    }

    if rate == 0.0 {
        if pmt_val == 0.0 {
            if pv_val + fv_val == 0.0 {
                return real("NPER", 0.0);
            }
            return Err(invalid("NPER", "payment cannot be zero when rate is zero"));
        }
        return real("NPER", -(pv_val + fv_val) / pmt_val);
    }

    let p_star = pmt_val * (1.0 + rate * typ);
    let den = p_star + rate * pv_val;

    if den == 0.0 {
        return Err(invalid("NPER", "cannot divide by zero"));
    }

    let numerator = p_star - rate * fv_val;
    if !numerator.is_finite() || !den.is_finite() {
        return Err(too_large("NPER"));
    }
    if numerator == 0.0 || numerator.is_sign_positive() != den.is_sign_positive() {
        return Err(invalid("NPER", "cash flows cannot satisfy target"));
    }

    // ln_1p keeps ratios near 1 accurate; separate logs avoid cancellation near 0.
    let x = -rate * (pv_val + fv_val) / den;
    let log_ratio = if x.abs() < 0.5 {
        x.ln_1p()
    } else {
        numerator.abs().ln() - den.abs().ln()
    };
    real("NPER", log_ratio / rate.ln_1p())
}
