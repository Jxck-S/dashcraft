//! Built-in DAX functions. Every function is a plain `fn` registered by its
//! upper-case name; unknown names produce
//! [`DaxError::UnsupportedFunction`](super::DaxError::UnsupportedFunction).

mod aggregate;
mod distributions;
mod info;
mod math;
mod model;
mod other;
mod scalar;
mod special;
mod stats;
mod tables;
mod time;
mod window;

pub(crate) use aggregate::{cmp_keys, sort_positions};
pub(crate) use math::RAND_SEED;

use std::collections::HashMap;
use std::sync::OnceLock;

use super::context::Env;
use super::error::{DaxError, Result};
use super::interp::{Interp, Val};
use crate::syntax::{Expr, ExprKind};

/// Implementation of a built-in function.
pub(crate) type BuiltinFn =
    for<'a, 'b, 'c, 'd> fn(&'b Interp<'a>, &'c Env, &'d [Expr]) -> Result<Val>;

/// A registered built-in function.
#[derive(Clone, Copy)]
pub(crate) struct Builtin {
    pub name: &'static str,
    /// Returns a table (used to classify `CALCULATE` filter arguments).
    pub table: bool,
    pub f: BuiltinFn,
}

pub(crate) const fn scalar_fn(name: &'static str, f: BuiltinFn) -> Builtin {
    Builtin {
        name,
        table: false,
        f,
    }
}

pub(crate) const fn table_fn(name: &'static str, f: BuiltinFn) -> Builtin {
    Builtin {
        name,
        table: true,
        f,
    }
}

fn registry() -> &'static HashMap<&'static str, Builtin> {
    static REGISTRY: OnceLock<HashMap<&'static str, Builtin>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        scalar::BUILTINS
            .iter()
            .chain(info::BUILTINS)
            .chain(aggregate::BUILTINS)
            .chain(tables::BUILTINS)
            .chain(time::BUILTINS)
            .chain(stats::BUILTINS)
            .chain(window::BUILTINS)
            .chain(model::BUILTINS)
            .chain(math::BUILTINS)
            .chain(distributions::BUILTINS)
            .chain(other::BUILTINS)
            .map(|b| (b.name, *b))
            .collect()
    })
}

/// Looks up a built-in by (case-insensitive) name.
pub(crate) fn lookup(name: &str) -> Option<&'static Builtin> {
    let reg = registry();
    reg.get(name)
        .or_else(|| reg.get(name.to_ascii_uppercase().as_str()))
}

/// `true` if `name` is a built-in that returns a table.
pub(crate) fn returns_table(name: &str) -> bool {
    lookup(name).is_some_and(|b| b.table)
}

/// Names of every supported DAX function, sorted.
pub fn supported_functions() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = registry().keys().copied().collect();
    names.sort_unstable();
    names
}

/// `true` if the evaluator implements the function `name`.
pub fn is_supported_function(name: &str) -> bool {
    lookup(name).is_some()
}

// ---- argument helpers ------------------------------------------------------

/// Checks the argument count (`max = usize::MAX` for variadic functions).
pub(crate) fn arity(function: &str, args: &[Expr], min: usize, max: usize) -> Result<()> {
    let n = args.len();
    if n < min || n > max {
        let expected = match (min, max) {
            (a, b) if a == b => format!("{a}"),
            (a, usize::MAX) => format!("at least {a}"),
            (a, b) => format!("{a} to {b}"),
        };
        return Err(DaxError::argument(
            function,
            format!("expects {expected} argument(s) but {n} were given"),
        ));
    }
    Ok(())
}

/// Argument `i` (must exist).
pub(crate) fn arg<'e>(function: &str, args: &'e [Expr], i: usize) -> Result<&'e Expr> {
    args.get(i)
        .ok_or_else(|| DaxError::argument(function, format!("missing argument {}", i + 1)))
}

/// Argument `i` unless missing or omitted.
pub(crate) fn opt_arg(args: &[Expr], i: usize) -> Option<&Expr> {
    args.get(i).filter(|e| !matches!(e.kind, ExprKind::Omitted))
}

/// A string-literal argument (column names in `ADDCOLUMNS` …).
pub(crate) fn name_arg(function: &str, e: &Expr) -> Result<String> {
    match &e.unparen().kind {
        ExprKind::String(s) => Ok(s.clone()),
        _ => Err(DaxError::argument(
            function,
            "expects a column name in double quotes",
        )),
    }
}
