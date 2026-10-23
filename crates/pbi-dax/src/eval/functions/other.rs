//! Small logical, information and "other" functions: `IF.EAGER`, `ERROR`,
//! `EVALUATEANDLOG`, `USERCULTURE`, `ISAFTER`, `ISONORAFTER`.

use std::cmp::Ordering;

use pbi_engine::Value;

use super::{Builtin, arg, arity, opt_arg, scalar_fn};
use crate::eval::context::Env;
use crate::eval::error::{DaxError, Result};
use crate::eval::interp::{Interp, Val};
use crate::eval::ops;
use crate::syntax::{Expr, ExprKind};

pub(super) const BUILTINS: &[Builtin] = &[
    scalar_fn("IF.EAGER", if_eager),
    scalar_fn("ERROR", error),
    scalar_fn("EVALUATEANDLOG", evaluateandlog),
    scalar_fn("USERCULTURE", userculture),
    scalar_fn("ISAFTER", isafter),
    scalar_fn("ISONORAFTER", isonorafter),
];

type R = Result<Val>;

/// `IF.EAGER(condition, then[, else])`: both branches are evaluated (so an
/// error in either is raised), then the result is chosen like `IF`.
fn if_eager(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("IF.EAGER", args, 2, 3)?;
    let cond = i.condition(env, arg("IF.EAGER", args, 0)?)?;
    let then = i.eval(env, arg("IF.EAGER", args, 1)?)?;
    let other = match opt_arg(args, 2) {
        Some(e) => i.eval(env, e)?,
        None => Val::Scalar(Value::Blank),
    };
    Ok(if cond { then } else { other })
}

/// `ERROR(text)`: raises an error with the given message.
fn error(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("ERROR", args, 1, 1)?;
    let text = ops::to_text(&i.scalar(env, arg("ERROR", args, 0)?)?);
    Err(DaxError::evaluation(text))
}

/// `EVALUATEANDLOG(value[, label[, max_rows]])`: returns `value`; the
/// label and a summary go to the `tracing` log (debug level) instead of a
/// Power BI trace event.
fn evaluateandlog(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("EVALUATEANDLOG", args, 1, 3)?;
    let value = i.eval(env, arg("EVALUATEANDLOG", args, 0)?)?;
    let label = match opt_arg(args, 1) {
        Some(e) => ops::to_text(&i.scalar(env, e)?),
        None => String::new(),
    };
    match &value {
        Val::Scalar(v) => {
            tracing::debug!(target: "dax::evaluateandlog", %label, value = %ops::to_text(v))
        }
        Val::Table(t) => {
            tracing::debug!(target: "dax::evaluateandlog", %label, rows = t.row_count())
        }
    }
    Ok(value)
}

/// `USERCULTURE()`: the culture used for formatting, `en-US` (the only
/// culture `FORMAT` implements).
fn userculture(_: &Interp<'_>, _: &Env, args: &[Expr]) -> R {
    arity("USERCULTURE", args, 0, 0)?;
    Ok(Val::Scalar(Value::string("en-US")))
}

/// Compares `(a₁, b₁[, order₁], a₂, b₂[, order₂], …)` pair by pair: the
/// first pair that differs decides (`DESC` reverses it).
fn compare_tuples(i: &Interp<'_>, env: &Env, args: &[Expr], f: &str) -> Result<Ordering> {
    arity(f, args, 2, usize::MAX)?;
    // Group the arguments first so a malformed list is an error even when
    // an earlier pair already decides.
    let mut pairs: Vec<(&Expr, &Expr, bool)> = Vec::new();
    let mut k = 0;
    while k < args.len() {
        let (a, b) = (arg(f, args, k)?, arg(f, args, k + 1)?);
        k += 2;
        let mut desc = false;
        if let Some(order) = args.get(k) {
            match &order.unparen().kind {
                ExprKind::Keyword(w) if w == "ASC" => k += 1,
                ExprKind::Keyword(w) if w == "DESC" => {
                    desc = true;
                    k += 1;
                }
                ExprKind::Omitted => k += 1,
                _ => {}
            }
        }
        pairs.push((a, b, desc));
    }
    for (a, b, desc) in pairs {
        let o = i.scalar(env, a)?.dax_cmp(&i.scalar(env, b)?)?;
        let o = if desc { o.reverse() } else { o };
        if o != Ordering::Equal {
            return Ok(o);
        }
    }
    Ok(Ordering::Equal)
}

/// `ISAFTER(value, start[, ASC|DESC], …)`: `TRUE` when the values come
/// strictly after the start values in the given order.
fn isafter(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    let o = compare_tuples(i, env, args, "ISAFTER")?;
    Ok(Val::Scalar(Value::Boolean(o == Ordering::Greater)))
}

/// `ISONORAFTER(value, start[, ASC|DESC], …)`: as `ISAFTER`, equal counts.
fn isonorafter(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    let o = compare_tuples(i, env, args, "ISONORAFTER")?;
    Ok(Val::Scalar(Value::Boolean(o != Ordering::Less)))
}
