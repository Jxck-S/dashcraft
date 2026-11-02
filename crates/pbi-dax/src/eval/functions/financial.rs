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

/// Validates and extracts the `type` argument:
/// - 0 or omitted: payments due at the end of the period.
/// - 1: payments due at the beginning of the period.
fn payment_type(i: &Interp<'_>, env: &Env, f: &str, args: &[Expr], n: usize) -> Result<f64> {
    let t = num_or(i, env, args, n, 0.0)?;
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

/// `PMT(rate, nper, pv[, fv][, type])`: calculates payment for a loan
/// based on constant payments and a constant interest rate.
fn pmt(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("PMT", args, 3, 5)?;
    let rate = num(i, env, "PMT", args, 0)?;
    let nper = num(i, env, "PMT", args, 1)?;
    let pv = num(i, env, "PMT", args, 2)?;
    let fv = num_or(i, env, args, 3, 0.0)?;
    let typ = payment_type(i, env, "PMT", args, 4)?;

    if nper == 0.0 {
        return Err(invalid("PMT", "nper cannot be zero"));
    }
    if rate <= -1.0 {
        return Err(invalid("PMT", "rate must be greater than -1"));
    }

    if rate == 0.0 {
        return real("PMT", -(pv + fv) / nper);
    }

    let factor = (1.0 + rate).powf(nper);
    let denom = (1.0 + rate * typ) * (factor - 1.0);
    if denom == 0.0 {
        return Err(invalid("PMT", "division by zero in annuity calculation"));
    }
    real("PMT", -(fv + pv * factor) * rate / denom)
}

/// `PV(rate, nper, pmt[, fv][, type])`: calculates present value of an annuity.
fn pv(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("PV", args, 3, 5)?;
    let rate = num(i, env, "PV", args, 0)?;
    let nper = num(i, env, "PV", args, 1)?;
    let pmt_val = num(i, env, "PV", args, 2)?;
    let fv_val = num_or(i, env, args, 3, 0.0)?;
    let typ = payment_type(i, env, "PV", args, 4)?;

    if rate <= -1.0 {
        return Err(invalid("PV", "rate must be greater than -1"));
    }

    if rate == 0.0 {
        return real("PV", -(fv_val + pmt_val * nper));
    }

    let factor = (1.0 + rate).powf(nper);
    if factor == 0.0 {
        return Err(too_large("PV"));
    }
    let term = pmt_val * (1.0 + rate * typ) * (factor - 1.0) / rate;
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

    if rate <= -1.0 {
        return Err(invalid("FV", "rate must be greater than -1"));
    }

    if rate == 0.0 {
        return real("FV", -(pv_val + pmt_val * nper));
    }

    let factor = (1.0 + rate).powf(nper);
    let term = pmt_val * (1.0 + rate * typ) * (factor - 1.0) / rate;
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
    let num = p_star - rate * fv_val;
    let den = p_star + rate * pv_val;

    if den == 0.0 {
        return Err(invalid("NPER", "cannot divide by zero"));
    }

    let ratio = num / den;
    if ratio <= 0.0 || !ratio.is_finite() {
        return Err(invalid("NPER", "cash flows cannot satisfy target"));
    }

    let n = ratio.ln() / rate.ln_1p();
    real("NPER", n)
}
