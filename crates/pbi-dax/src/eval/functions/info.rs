//! Context-aware functions: `CALCULATE`, `ISEMPTY`, `HASONEVALUE`,
//! `SELECTEDVALUE`, `ISFILTERED`, `ISCROSSFILTERED`, `HASONEFILTER`,
//! `ISINSCOPE`, `RELATED`, `EARLIER`, `LOOKUPVALUE`, `CONTAINS`,
//! `CONTAINSROW`, parent-child `PATH` functions, and the `CALCULATE`-only
//! modifiers `USERELATIONSHIP`/`CROSSFILTER`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use pbi_engine::{EngineError, Filter, FilterContext, RowSet, Value, ValueKey};
use pbi_model::names::names_eq;

use super::{Builtin, arg, arity, opt_arg, scalar_fn};
use crate::eval::context::Env;
use crate::eval::error::{DaxError, Result};
use crate::eval::interp::{Interp, Val};
use crate::eval::table::BLANK_ROW;
use crate::syntax::{Expr, ExprKind};

pub(super) const BUILTINS: &[Builtin] = &[
    scalar_fn("CALCULATE", calculate),
    scalar_fn("ISEMPTY", isempty),
    scalar_fn("HASONEVALUE", hasonevalue),
    scalar_fn("SELECTEDVALUE", selectedvalue),
    scalar_fn("ISFILTERED", isfiltered),
    scalar_fn("ISCROSSFILTERED", iscrossfiltered),
    scalar_fn("HASONEFILTER", hasonefilter),
    scalar_fn("ISINSCOPE", isinscope),
    scalar_fn("CONTAINS", contains),
    scalar_fn("CONTAINSROW", containsrow),
    scalar_fn("PATH", path),
    scalar_fn("PATHITEM", pathitem),
    scalar_fn("PATHITEMREVERSE", pathitemreverse),
    scalar_fn("PATHLENGTH", pathlength),
    scalar_fn("PATHCONTAINS", pathcontains),
    scalar_fn("USERELATIONSHIP", userelationship),
    scalar_fn("CROSSFILTER", crossfilter),
    scalar_fn("RELATED", related),
    scalar_fn("EARLIER", earlier),
    scalar_fn("EARLIEST", earliest),
    scalar_fn("LOOKUPVALUE", lookupvalue),
];

type R = Result<Val>;

fn ok(v: Value) -> R {
    Ok(Val::Scalar(v))
}

fn calculate(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("CALCULATE", args, 1, usize::MAX)?;
    let filter = i.calculate_context(env, args.get(1..).unwrap_or_default(), "CALCULATE")?;
    let inner = env.with_filter(filter);
    ok(i.scalar(&inner, arg("CALCULATE", args, 0)?)?)
}

fn isempty(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("ISEMPTY", args, 1, 1)?;
    let a = arg("ISEMPTY", args, 0)?.unparen();
    // Short-circuit ISEMPTY(FILTER(t, pred)): stop at the first match.
    if let Some(("FILTER", [t, pred])) = a.as_call() {
        let table = i.table(env, t)?;
        let mut any = false;
        i.iterate(env, &table, |row_env, _| {
            if i.condition(row_env, pred)? {
                any = true;
                return Ok(false);
            }
            Ok(true)
        })?;
        return ok(Value::Boolean(!any));
    }
    ok(Value::Boolean(i.table(env, a)?.is_empty()))
}

fn hasonevalue(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("HASONEVALUE", args, 1, 1)?;
    let (t, c) = i.column_arg(env, arg("HASONEVALUE", args, 0)?, "HASONEVALUE")?;
    let values = i.snapshot.values(&env.filter, &t, &c)?;
    ok(Value::Boolean(values.len() == 1))
}

fn selectedvalue(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("SELECTEDVALUE", args, 1, 2)?;
    let (t, c) = i.column_arg(env, arg("SELECTEDVALUE", args, 0)?, "SELECTEDVALUE")?;
    let values = i.snapshot.values(&env.filter, &t, &c)?;
    match values.as_slice() {
        [one] => ok(one.clone()),
        _ => match opt_arg(args, 1) {
            Some(alt) => ok(i.scalar(env, alt)?),
            None => ok(Value::Blank),
        },
    }
}

fn isfiltered(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("ISFILTERED", args, 1, 1)?;
    let a = arg("ISFILTERED", args, 0)?;
    match &a.unparen().kind {
        ExprKind::TableRef { name } | ExprKind::Keyword(name) => {
            let data = i
                .snapshot
                .table(name)
                .map_err(|_| DaxError::UnknownTable { name: name.clone() })?;
            ok(Value::Boolean(env.filter.is_table_filtered(data.name())))
        }
        _ => {
            let (t, c) = i.column_arg(env, a, "ISFILTERED")?;
            ok(Value::Boolean(env.filter.is_column_filtered(&t, &c)))
        }
    }
}

/// `ISCROSSFILTERED(T)` / `ISCROSSFILTERED(T[c])`: a filter applies to
/// the table, directly or through relationships (or an expanded-table
/// filter).
fn iscrossfiltered(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("ISCROSSFILTERED", args, 1, 1)?;
    let a = arg("ISCROSSFILTERED", args, 0)?;
    let table = match &a.unparen().kind {
        ExprKind::TableRef { name } | ExprKind::Keyword(name) => i
            .snapshot
            .table(name)
            .map_err(|_| DaxError::UnknownTable { name: name.clone() })?
            .name()
            .to_owned(),
        _ => i.column_arg(env, a, "ISCROSSFILTERED")?.0,
    };
    let graph = i.snapshot.graph_for(&env.filter);
    let sources: Vec<String> = graph
        .tree(&table)
        .map(|t| t.nodes.iter().map(|n| n.table.clone()).collect())
        .unwrap_or_else(|| vec![table.clone()]);
    let crossed = env.filter.filters().iter().any(|f| {
        sources.iter().any(|s| names_eq(s, f.table()))
            || matches!(f, Filter::Table(t) if t.expanded && graph.expanded_path(&t.table, &table).is_some())
    });
    ok(Value::Boolean(crossed))
}

/// Values of `T[c]` allowed by the filters applied directly to that column
/// (`FILTERS`); all values when it has none.
pub(crate) fn direct_filter_values(
    i: &Interp<'_>,
    env: &Env,
    t: &str,
    c: &str,
) -> Result<Vec<Value>> {
    let mut own = FilterContext::new();
    for f in env.filter.column_filters(t, c) {
        own.add(f.clone());
    }
    Ok(i.snapshot.values(&own, t, c)?)
}

fn hasonefilter(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("HASONEFILTER", args, 1, 1)?;
    let (t, c) = i.column_arg(env, arg("HASONEFILTER", args, 0)?, "HASONEFILTER")?;
    if env.filter.column_filters(&t, &c).next().is_none() {
        return ok(Value::Boolean(false));
    }
    ok(Value::Boolean(
        direct_filter_values(i, env, &t, &c)?.len() == 1,
    ))
}

/// `ISINSCOPE(T[c])`: the column is a group-by column of the current level
/// (set by `SUMMARIZECOLUMNS` or the caller, see
/// [`crate::EvalContext::with_group_by`]) and still filtered.
fn isinscope(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("ISINSCOPE", args, 1, 1)?;
    let (t, c) = i.column_arg(env, arg("ISINSCOPE", args, 0)?, "ISINSCOPE")?;
    ok(Value::Boolean(
        env.in_scope(&t, &c) && env.filter.is_column_filtered(&t, &c),
    ))
}

/// `CONTAINS(table, column, value, …)`: some row has every column equal
/// (DAX `=`) to its value.
fn contains(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("CONTAINS", args, 3, usize::MAX)?;
    let table = i.table(env, arg("CONTAINS", args, 0)?)?;
    let rest = args.get(1..).unwrap_or_default();
    let (pairs, extra) = rest.as_chunks::<2>();
    if !extra.is_empty() {
        return Err(DaxError::argument(
            "CONTAINS",
            "expects pairs of a column and a value",
        ));
    }
    let mut criteria = Vec::with_capacity(pairs.len());
    for [col, value] in pairs {
        let ExprKind::ColumnRef { table: t, column } = &col.unparen().kind else {
            return Err(DaxError::argument("CONTAINS", "expects a column reference"));
        };
        let ix = table
            .find_column(t.as_deref(), column)
            .or_else(|| {
                i.resolve_column(env, t.as_deref(), column)
                    .ok()
                    .and_then(|(t, c)| table.find_column(Some(&t), &c))
            })
            .ok_or_else(|| {
                DaxError::argument(
                    "CONTAINS",
                    format!(
                        "the column {} is not in the table",
                        crate::eval::interp::snippet(col)
                    ),
                )
            })?;
        criteria.push((ix, i.scalar(env, value)?));
    }
    let found = table.positions().into_iter().any(|p| {
        criteria
            .iter()
            .all(|(ix, v)| table.value(p, *ix).dax_eq(v).unwrap_or(false))
    });
    ok(Value::Boolean(found))
}

/// `CONTAINSROW(table, value, …)`: like `IN` (strict equality).
fn containsrow(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("CONTAINSROW", args, 2, usize::MAX)?;
    let table = i.table(env, arg("CONTAINSROW", args, 0)?)?;
    let values = args
        .get(1..)
        .unwrap_or_default()
        .iter()
        .map(|a| i.scalar(env, a))
        .collect::<Result<Vec<_>>>()?;
    if values.len() != table.column_count() {
        return Err(DaxError::argument(
            "CONTAINSROW",
            format!(
                "the table has {} column(s) but {} value(s) were given",
                table.column_count(),
                values.len()
            ),
        ));
    }
    let wanted: Vec<ValueKey> = values.iter().map(ValueKey::from).collect();
    ok(Value::Boolean(
        table
            .positions()
            .into_iter()
            .any(|p| table.row_key(p) == wanted),
    ))
}

/// Maximum depth of a parent-child path.
const MAX_PATH_DEPTH: usize = 10_000;

/// `PATH(id_column, parent_column)`: `root|…|parent|id` for the current
/// row (row context required).
fn path(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("PATH", args, 2, 2)?;
    let (t, id_col) = i.column_arg(env, arg("PATH", args, 0)?, "PATH")?;
    let (t2, parent_col) = i.column_arg(env, arg("PATH", args, 1)?, "PATH")?;
    if !names_eq(&t, &t2) {
        return Err(DaxError::argument(
            "PATH",
            "both columns must belong to the same table",
        ));
    }
    let Some(start) = i.frame_value(env, Some(&t), &id_col) else {
        return Err(DaxError::argument(
            "PATH",
            "needs a row context on the table (use it in a calculated column or an iterator)",
        ));
    };
    if start.is_blank() {
        return ok(Value::Blank);
    }
    let n = i.snapshot.row_count(&t)?;
    let all = RowSet::all(n);
    let ids = i.snapshot.column_values(&t, &id_col, &all)?;
    let parents = i.snapshot.column_values(&t, &parent_col, &all)?;
    let parent_of: HashMap<ValueKey, Value> = ids
        .iter()
        .zip(parents)
        .map(|(id, p)| (ValueKey::from(id), p))
        .collect();
    let mut chain = vec![crate::eval::ops::to_text(&start)];
    let mut seen: HashSet<ValueKey> = HashSet::from([ValueKey::from(&start)]);
    let mut current = parent_of.get(&ValueKey::from(&start)).cloned();
    while let Some(p) = current.filter(|p| !p.is_blank()) {
        i.check_cancel()?;
        if !seen.insert(ValueKey::from(&p)) || chain.len() > MAX_PATH_DEPTH {
            return Err(DaxError::evaluation(format!(
                "PATH: the parent chain of '{}' is circular",
                crate::eval::ops::to_text(&start)
            )));
        }
        if !parent_of.contains_key(&ValueKey::from(&p)) {
            return Err(DaxError::evaluation(format!(
                "PATH: the parent '{}' is not a value of the id column",
                crate::eval::ops::to_text(&p)
            )));
        }
        chain.push(crate::eval::ops::to_text(&p));
        current = parent_of.get(&ValueKey::from(&p)).cloned();
    }
    chain.reverse();
    ok(Value::String(Arc::from(chain.join("|"))))
}

fn path_items(i: &Interp<'_>, env: &Env, f: &str, args: &[Expr]) -> Result<Vec<String>> {
    let v = i.scalar(env, arg(f, args, 0)?)?;
    let text = crate::eval::ops::to_text(&v);
    if text.is_empty() {
        return Ok(Vec::new());
    }
    Ok(text.split('|').map(str::to_owned).collect())
}

fn path_item(i: &Interp<'_>, env: &Env, args: &[Expr], f: &str, reverse: bool) -> R {
    arity(f, args, 2, 3)?;
    let mut items = path_items(i, env, f, args)?;
    if reverse {
        items.reverse();
    }
    let pos = crate::eval::ops::to_i64(&i.scalar(env, arg(f, args, 1)?)?)?.unwrap_or(0);
    let Some(item) = usize::try_from(pos - 1).ok().and_then(|p| items.get(p)) else {
        return ok(Value::Blank);
    };
    let as_int = match opt_arg(args, 2) {
        Some(e) => crate::eval::ops::to_i64(&i.scalar(env, e)?)?.unwrap_or(0) == 1,
        None => false,
    };
    if as_int {
        return match item.trim().parse::<i64>() {
            Ok(n) => ok(Value::Integer(n)),
            Err(_) => Err(DaxError::type_error(format!(
                "Cannot convert value '{item}' of type Text to type Integer."
            ))),
        };
    }
    ok(Value::string(item.as_str()))
}

fn pathitem(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    path_item(i, env, args, "PATHITEM", false)
}

fn pathitemreverse(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    path_item(i, env, args, "PATHITEMREVERSE", true)
}

fn pathlength(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("PATHLENGTH", args, 1, 1)?;
    let n = path_items(i, env, "PATHLENGTH", args)?.len();
    ok(Value::Integer(i64::try_from(n).unwrap_or(i64::MAX)))
}

fn pathcontains(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("PATHCONTAINS", args, 2, 2)?;
    let items = path_items(i, env, "PATHCONTAINS", args)?;
    let item = crate::eval::ops::to_text(&i.scalar(env, arg("PATHCONTAINS", args, 1)?)?);
    ok(Value::Boolean(
        items.iter().any(|x| x.eq_ignore_ascii_case(&item)),
    ))
}

fn calculate_only(f: &str) -> R {
    Err(DaxError::argument(
        f,
        "can only be used as a filter argument of CALCULATE or CALCULATETABLE",
    ))
}

fn userelationship(_: &Interp<'_>, _: &Env, _: &[Expr]) -> R {
    calculate_only("USERELATIONSHIP")
}

fn crossfilter(_: &Interp<'_>, _: &Env, _: &[Expr]) -> R {
    calculate_only("CROSSFILTER")
}

fn related(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("RELATED", args, 1, 1)?;
    let (t, c) = i.column_arg(env, arg("RELATED", args, 0)?, "RELATED")?;
    for f in env.rows.frames() {
        let Some(data) = &f.table.base else { continue };
        if i.snapshot.graph().expanded_path(data.name(), &t).is_none() {
            continue;
        }
        let row = f.table.base_row(f.pos);
        if row == BLANK_ROW {
            return ok(Value::Blank);
        }
        return ok(i.snapshot.related_value(data.name(), row, &t, &c)?);
    }
    if let Some(m2m) = env.rows.frames().find_map(|f| {
        let data = f.table.base.as_ref()?;
        i.snapshot
            .graph()
            .many_to_many_between(data.name(), &t)
            .then(|| data.name().to_owned())
    }) {
        return Err(DaxError::argument(
            "RELATED",
            format!(
                "the column '{t}'[{c}] either doesn't exist or doesn't have a relationship to any table available in the current context: '{m2m}' and '{t}' are joined by a many-to-many (limited) relationship, which RELATED cannot follow; use RELATEDTABLE or a filter"
            ),
        ));
    }
    Err(DaxError::argument(
        "RELATED",
        format!(
            "there is no row context on a table related to '{t}' (RELATED follows many-to-one relationships)"
        ),
    ))
}

fn earlier_impl(i: &Interp<'_>, env: &Env, args: &[Expr], f: &str, last: bool) -> R {
    let a = arg(f, args, 0)?;
    let ExprKind::ColumnRef { table, column } = &a.unparen().kind else {
        return Err(DaxError::argument(f, "expects a column reference"));
    };
    let n = match opt_arg(args, 1) {
        Some(e) => crate::eval::ops::to_i64(&i.scalar(env, e)?)?.unwrap_or(1),
        None => 1,
    };
    let frames: Vec<_> = env
        .rows
        .frames()
        .filter_map(|fr| {
            fr.table
                .find_column(table.as_deref(), column)
                .map(|c| fr.table.value(fr.pos, c))
        })
        .collect();
    let picked = if last {
        frames.last().cloned()
    } else {
        usize::try_from(n).ok().and_then(|n| frames.get(n).cloned())
    };
    picked.map(Val::Scalar).ok_or_else(|| {
        DaxError::argument(f, "refers to an earlier row context which doesn't exist")
    })
}

fn earlier(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("EARLIER", args, 1, 2)?;
    earlier_impl(i, env, args, "EARLIER", false)
}

fn earliest(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("EARLIEST", args, 1, 1)?;
    earlier_impl(i, env, args, "EARLIEST", true)
}

fn lookupvalue(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("LOOKUPVALUE", args, 3, usize::MAX)?;
    let (table, result_col) = i.column_arg(env, arg("LOOKUPVALUE", args, 0)?, "LOOKUPVALUE")?;
    let rest = args.get(1..).unwrap_or_default();
    let (pairs, remainder) = rest.as_chunks::<2>();
    let mut criteria = Vec::new();
    for [col, value] in pairs {
        let (t, c) = i.column_arg(env, col, "LOOKUPVALUE")?;
        if !pbi_model::names_eq(&t, &table) {
            return Err(DaxError::Unsupported {
                feature: "LOOKUPVALUE with search columns from another table".into(),
            });
        }
        criteria.push((c, i.scalar(env, value)?));
    }
    let alternate = remainder.first();
    let refs: Vec<(&str, Value)> = criteria
        .iter()
        .map(|(c, v)| (c.as_str(), v.clone()))
        .collect();
    match i.snapshot.lookup(&table, &result_col, &refs, None) {
        Ok(Value::Blank) => match alternate {
            Some(alt) => ok(i.scalar(env, alt)?),
            None => ok(Value::Blank),
        },
        Ok(v) => ok(v),
        Err(EngineError::AmbiguousLookup { .. }) if alternate.is_some() => match alternate {
            Some(alt) => ok(i.scalar(env, alt)?),
            None => ok(Value::Blank),
        },
        Err(EngineError::AmbiguousLookup { .. }) => Err(DaxError::evaluation(
            "LOOKUPVALUE: a table of multiple values was supplied where a single value was expected.",
        )),
        Err(e) => Err(e.into()),
    }
}
