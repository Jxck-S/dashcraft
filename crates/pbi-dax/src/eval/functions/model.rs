//! Calculation-group functions (`SELECTEDMEASURE`, `SELECTEDMEASURENAME`,
//! `SELECTEDMEASUREFORMATSTRING`, `ISSELECTEDMEASURE`) and the effective
//! user (`USERPRINCIPALNAME`, `USERNAME`, `USEROBJECTID`, `CUSTOMDATA`).

use std::sync::Arc;

use pbi_engine::{UserIdentity, Value};
use pbi_model::names::names_eq;

use super::{Builtin, arity, scalar_fn};
use crate::eval::context::Env;
use crate::eval::error::{DaxError, Result};
use crate::eval::interp::{Interp, MeasureRef, Val, snippet};
use crate::syntax::{Expr, ExprKind};

pub(super) const BUILTINS: &[Builtin] = &[
    scalar_fn("SELECTEDMEASURE", selectedmeasure),
    scalar_fn("SELECTEDMEASURENAME", selectedmeasurename),
    scalar_fn("SELECTEDMEASUREFORMATSTRING", selectedmeasureformatstring),
    scalar_fn("ISSELECTEDMEASURE", isselectedmeasure),
    scalar_fn("USERPRINCIPALNAME", userprincipalname),
    scalar_fn("USERNAME", username),
    scalar_fn("USEROBJECTID", userobjectid),
    scalar_fn("CUSTOMDATA", customdata),
];

type R = Result<Val>;

fn ok(v: Value) -> R {
    Ok(Val::Scalar(v))
}

/// The measure a calculation item (or dynamic format string) applies to.
fn selected<'e>(env: &'e Env, function: &str) -> Result<&'e Arc<MeasureRef>> {
    env.selected.as_ref().ok_or_else(|| {
        DaxError::evaluation(format!(
            "{function} can only be used in the expression of a calculation item or a dynamic format string."
        ))
    })
}

fn selectedmeasure(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("SELECTEDMEASURE", args, 0, 0)?;
    let m = selected(env, "SELECTEDMEASURE")?;
    ok(i.eval_measure(env, m)?)
}

fn selectedmeasurename(_: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("SELECTEDMEASURENAME", args, 0, 0)?;
    let m = selected(env, "SELECTEDMEASURENAME")?;
    ok(Value::string(m.name.as_str()))
}

fn selectedmeasureformatstring(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("SELECTEDMEASUREFORMATSTRING", args, 0, 0)?;
    let m = selected(env, "SELECTEDMEASUREFORMATSTRING")?;
    ok(i.measure_format_string(env, m)?
        .map_or(Value::Blank, Value::string))
}

fn isselectedmeasure(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("ISSELECTEDMEASURE", args, 1, usize::MAX)?;
    let m = selected(env, "ISSELECTEDMEASURE")?;
    let mut found = false;
    for a in args {
        let ExprKind::ColumnRef { table, column } = &a.unparen().kind else {
            return Err(DaxError::argument(
                "ISSELECTEDMEASURE",
                format!("expects measure references, got {}", snippet(a)),
            ));
        };
        let Some(listed) = i.find_measure(table.as_deref(), column)? else {
            return Err(DaxError::UnknownMeasure {
                name: column.clone(),
            });
        };
        found |= names_eq(&listed.name, &m.name);
    }
    ok(Value::Boolean(found))
}

fn identity<'s>(i: &'s Interp<'_>) -> Option<&'s UserIdentity> {
    i.snapshot.security().map(|s| &s.identity)
}

/// The effective user name (empty text when none is set).
fn user_name(i: &Interp<'_>) -> Value {
    Value::string(
        identity(i)
            .and_then(|u| u.user_name.as_deref())
            .unwrap_or_default(),
    )
}

fn userprincipalname(i: &Interp<'_>, _: &Env, args: &[Expr]) -> R {
    arity("USERPRINCIPALNAME", args, 0, 0)?;
    ok(user_name(i))
}

fn username(i: &Interp<'_>, _: &Env, args: &[Expr]) -> R {
    arity("USERNAME", args, 0, 0)?;
    ok(user_name(i))
}

fn userobjectid(i: &Interp<'_>, _: &Env, args: &[Expr]) -> R {
    arity("USEROBJECTID", args, 0, 0)?;
    ok(Value::string(
        identity(i)
            .and_then(|u| u.user_object_id.as_deref())
            .unwrap_or_default(),
    ))
}

fn customdata(i: &Interp<'_>, _: &Env, args: &[Expr]) -> R {
    arity("CUSTOMDATA", args, 0, 0)?;
    ok(identity(i)
        .and_then(|u| u.custom_data.as_deref())
        .map_or(Value::Blank, Value::string))
}
