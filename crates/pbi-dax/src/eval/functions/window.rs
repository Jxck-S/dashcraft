//! Window functions: `INDEX`, `OFFSET`, `WINDOW`, `RANK`, `ROWNUMBER` with
//! `ORDERBY`, `PARTITIONBY` and `MATCHBY`.
//!
//! Basic semantics: the *relation* (a table; when omitted, the
//! `ALLSELECTED` combinations of the `ORDERBY`/`PARTITIONBY` columns) is
//! split into partitions by the `PARTITIONBY` columns and sorted by the
//! `ORDERBY` expressions (evaluated in each row's context; ties keep the
//! relation's order). The *current row* is the relation row whose columns
//! equal their current values — from the row context, else the single
//! value in the filter context. The `MATCHBY` columns (default: the
//! `PARTITIONBY` columns and the `ORDERBY` column references) must be
//! known; without `MATCHBY` the other relation columns whose value is known
//! are matched too. When the current row is needed but cannot be
//! determined (e.g. a total row) the result is empty (BLANK for `RANK`
//! and `ROWNUMBER`). The `blanks` argument is accepted and ignored (BLANK
//! sorts first ascending, as in `ORDER BY`).

use std::sync::Arc;

use pbi_engine::{Value, ValueKey};

use super::aggregate::cmp_keys;
use super::{Builtin, arg, arity, scalar_fn, table_fn};
use crate::eval::context::Env;
use crate::eval::error::{DaxError, Result};
use crate::eval::filters::{Selected, apply_allselected};
use crate::eval::interp::{Interp, Val};
use crate::eval::ops;
use crate::eval::table::{RowPos, VColumn, VirtualTable};
use crate::syntax::{Expr, ExprKind};

pub(super) const BUILTINS: &[Builtin] = &[
    table_fn("INDEX", index),
    table_fn("OFFSET", offset),
    table_fn("WINDOW", window),
    scalar_fn("RANK", rank),
    scalar_fn("ROWNUMBER", rownumber),
    table_fn("ORDERBY", orderby),
    table_fn("PARTITIONBY", partitionby),
    table_fn("MATCHBY", matchby),
];

type R = Result<Val>;

/// The arguments after the position(s).
struct Spec<'e> {
    relation: Option<&'e Expr>,
    order: Vec<(&'e Expr, bool)>,
    partition: Vec<&'e Expr>,
    matchby: Vec<&'e Expr>,
}

fn parse_spec<'e>(f: &str, args: &'e [Expr]) -> Result<Spec<'e>> {
    let mut spec = Spec {
        relation: None,
        order: Vec::new(),
        partition: Vec::new(),
        matchby: Vec::new(),
    };
    for a in args {
        let u = a.unparen();
        match (&u.kind, u.as_call()) {
            (_, Some(("ORDERBY", items))) => {
                let mut k = 0;
                while let Some(e) = items.get(k) {
                    k += 1;
                    let desc = match items.get(k).map(|n| &n.unparen().kind) {
                        Some(ExprKind::Keyword(w)) if w == "DESC" => {
                            k += 1;
                            true
                        }
                        Some(ExprKind::Keyword(w)) if w == "ASC" => {
                            k += 1;
                            false
                        }
                        _ => false,
                    };
                    spec.order.push((e, desc));
                }
            }
            (_, Some(("PARTITIONBY", items))) => spec.partition.extend(items.iter()),
            (_, Some(("MATCHBY", items))) => spec.matchby.extend(items.iter()),
            (ExprKind::Omitted | ExprKind::Keyword(_), _) => {}
            _ if spec.relation.is_none() => spec.relation = Some(a),
            _ => {
                return Err(DaxError::argument(
                    f,
                    format!("unexpected argument {}", crate::eval::interp::snippet(a)),
                ));
            }
        }
    }
    Ok(spec)
}

/// The relation: the given table, or the `ALLSELECTED` value combinations
/// of the `ORDERBY`/`PARTITIONBY` columns (one table only).
fn relation(i: &Interp<'_>, env: &Env, f: &str, spec: &Spec<'_>) -> Result<Arc<VirtualTable>> {
    if let Some(r) = spec.relation {
        return i.table(env, r);
    }
    let mut cols: Vec<(String, String)> = Vec::new();
    for e in spec
        .partition
        .iter()
        .chain(spec.order.iter().map(|(e, _)| e))
    {
        if let ExprKind::ColumnRef { .. } = e.unparen().kind {
            let c = i.column_arg(env, e, f)?;
            if !cols.contains(&c) {
                cols.push(c);
            }
        }
    }
    let Some((table, _)) = cols.first().cloned() else {
        return Err(DaxError::argument(
            f,
            "expects a relation table or ORDERBY/PARTITIONBY columns",
        ));
    };
    if cols.iter().any(|(t, _)| !pbi_model::names_eq(t, &table)) {
        return Err(DaxError::Unsupported {
            feature: format!("{f} without a relation over columns of several tables"),
        });
    }
    let mut filter = (*env.filter).clone();
    let graph = i.snapshot.graph_for(&filter);
    apply_allselected(
        &mut filter,
        &Selected::Columns(cols.clone()),
        &env.shadow,
        &graph,
    );
    let names: Vec<String> = cols.iter().map(|(_, c)| c.clone()).collect();
    let combos = super::tables::visible_combos(i, &filter, &table, &names)?;
    let columns = cols
        .iter()
        .map(|(t, c)| {
            let dt = i
                .snapshot
                .table(t)
                .ok()
                .and_then(|d| d.column_type(c).ok().cloned());
            VColumn::model(t, c, dt)
        })
        .collect();
    Ok(Arc::new(VirtualTable::materialized(columns, combos)))
}

/// Index of a column-reference expression in `rel`.
fn column_index(i: &Interp<'_>, env: &Env, f: &str, rel: &VirtualTable, e: &Expr) -> Result<usize> {
    let ExprKind::ColumnRef { table, column } = &e.unparen().kind else {
        return Err(DaxError::argument(
            f,
            "PARTITIONBY and MATCHBY expect column references",
        ));
    };
    if let Some(ix) = rel.find_column(table.as_deref(), column) {
        return Ok(ix);
    }
    if let Ok((t, c)) = i.resolve_column(env, table.as_deref(), column)
        && let Some(ix) = rel.find_column(Some(&t), &c)
    {
        return Ok(ix);
    }
    Err(DaxError::argument(
        f,
        format!(
            "the column {} is not in the relation",
            crate::eval::interp::snippet(e)
        ),
    ))
}

/// Current value of relation column `ix`: row context first (by lineage or
/// name), then a single value in the filter context.
fn current_value(
    i: &Interp<'_>,
    env: &Env,
    rel: &VirtualTable,
    ix: usize,
) -> Result<Option<Value>> {
    let Some(col) = rel.columns().get(ix) else {
        return Ok(None);
    };
    if let Some(lin) = &col.lineage {
        if let Some(v) = i.frame_value(env, Some(&lin.table), &lin.column) {
            return Ok(Some(v));
        }
        if i.snapshot
            .table(&lin.table)
            .is_ok_and(|d| d.has_column(&lin.column))
        {
            let values = i.snapshot.values(&env.filter, &lin.table, &lin.column)?;
            return Ok(match values.as_slice() {
                [one] => Some(one.clone()),
                _ => None,
            });
        }
    }
    Ok(i.frame_value(env, col.table.as_deref(), &col.name))
}

/// Sorted partition and the position of the current row in it (`None`
/// when the current row is not needed or cannot be determined).
struct Prepared {
    rel: Arc<VirtualTable>,
    partition: Vec<RowPos>,
    /// `ORDERBY` keys of each partition row.
    keys: Vec<Vec<Value>>,
    desc: Vec<bool>,
    current: Option<usize>,
}

fn prepare(
    i: &Interp<'_>,
    env: &Env,
    f: &str,
    rest: &[Expr],
    need_current: bool,
) -> Result<Option<Prepared>> {
    let spec = parse_spec(f, rest)?;
    let rel = relation(i, env, f, &spec)?;
    let part_cols = spec
        .partition
        .iter()
        .map(|e| column_index(i, env, f, &rel, e))
        .collect::<Result<Vec<_>>>()?;
    // Columns that must identify the current row (`MATCHBY`, else the
    // `PARTITIONBY` and `ORDERBY` columns); without `MATCHBY` every other
    // relation column whose current value is known also takes part.
    let explicit = !spec.matchby.is_empty();
    let required = if explicit {
        spec.matchby
            .iter()
            .map(|e| column_index(i, env, f, &rel, e))
            .collect::<Result<Vec<_>>>()?
    } else {
        let mut cols = part_cols.clone();
        for (e, _) in &spec.order {
            if matches!(e.unparen().kind, ExprKind::ColumnRef { .. })
                && let Ok(ix) = column_index(i, env, f, &rel, e)
                && !cols.contains(&ix)
            {
                cols.push(ix);
            }
        }
        cols
    };
    let candidates: Vec<usize> = if explicit {
        required.clone()
    } else {
        (0..rel.column_count()).collect()
    };
    let need_current = need_current || !part_cols.is_empty();

    // Current row.
    let mut current_row: Option<RowPos> = None;
    if need_current {
        let mut wanted: Vec<(usize, ValueKey)> = Vec::with_capacity(candidates.len());
        for &ix in &candidates {
            match current_value(i, env, &rel, ix)? {
                Some(v) => wanted.push((ix, ValueKey::from(v))),
                None if required.contains(&ix) => return Ok(None),
                None => {}
            }
        }
        if wanted.is_empty() {
            return Ok(None);
        }
        current_row = rel.positions().into_iter().find(|p| {
            wanted
                .iter()
                .all(|(ix, w)| ValueKey::from(rel.value(*p, *ix)) == *w)
        });
        if current_row.is_none() {
            return Ok(None);
        }
    }

    // Partition of the current row (or the whole relation).
    let part_key = |p: RowPos| -> Vec<ValueKey> {
        part_cols
            .iter()
            .map(|ix| ValueKey::from(rel.value(p, *ix)))
            .collect()
    };
    let key = current_row.map(part_key);
    let desc: Vec<bool> = spec.order.iter().map(|(_, d)| *d).collect();
    let mut rows: Vec<(Vec<Value>, RowPos)> = Vec::new();
    i.iterate(env, &rel, |row_env, pos| {
        if key.as_ref().is_some_and(|k| part_key(pos) != *k) {
            return Ok(true);
        }
        let keys = spec
            .order
            .iter()
            .map(|(e, _)| i.scalar(row_env, e))
            .collect::<Result<Vec<_>>>()?;
        rows.push((keys, pos));
        Ok(true)
    })?;
    rows.sort_by(|a, b| cmp_keys(&a.0, &b.0, &desc));
    let (keys, partition): (Vec<Vec<Value>>, Vec<RowPos>) = rows.into_iter().unzip();
    let current = current_row.and_then(|c| partition.iter().position(|p| *p == c));
    Ok(Some(Prepared {
        rel,
        partition,
        keys,
        desc,
        current,
    }))
}

fn empty_like(i: &Interp<'_>, env: &Env, f: &str, rest: &[Expr]) -> R {
    let spec = parse_spec(f, rest)?;
    let rel = relation(i, env, f, &spec)?;
    Ok(Val::Table(Arc::new(rel.subset_ordered(&[]))))
}

fn pick(p: &Prepared, lo: i64, hi: i64) -> R {
    let len = i64::try_from(p.partition.len()).unwrap_or(i64::MAX);
    let lo = lo.max(0);
    let hi = hi.min(len - 1);
    let keep: Vec<RowPos> = if lo > hi {
        Vec::new()
    } else {
        p.partition
            .get(lo as usize..=hi as usize)
            .map(<[RowPos]>::to_vec)
            .unwrap_or_default()
    };
    Ok(Val::Table(Arc::new(p.rel.subset_ordered(&keep))))
}

fn int_arg(i: &Interp<'_>, env: &Env, f: &str, e: &Expr) -> Result<i64> {
    ops::to_i64(&i.scalar(env, e)?)?
        .ok_or_else(|| DaxError::argument(f, "the position cannot be blank"))
}

/// 0-based index of a 1-based absolute position (negative: from the end).
fn absolute(pos: i64, len: usize) -> i64 {
    let len = i64::try_from(len).unwrap_or(i64::MAX);
    if pos > 0 { pos - 1 } else { len + pos }
}

/// `INDEX(position[, relation][, ORDERBY(…)][, blanks][, PARTITIONBY(…)][, MATCHBY(…)])`.
fn index(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("INDEX", args, 1, usize::MAX)?;
    let pos = int_arg(i, env, "INDEX", arg("INDEX", args, 0)?)?;
    let rest = args.get(1..).unwrap_or_default();
    let Some(p) = prepare(i, env, "INDEX", rest, false)? else {
        return empty_like(i, env, "INDEX", rest);
    };
    if pos == 0 {
        return pick(&p, 1, 0);
    }
    let ix = absolute(pos, p.partition.len());
    pick(&p, ix, ix)
}

/// `OFFSET(delta[, relation][, ORDERBY(…)][, blanks][, PARTITIONBY(…)][, MATCHBY(…)])`.
fn offset(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("OFFSET", args, 1, usize::MAX)?;
    let delta = int_arg(i, env, "OFFSET", arg("OFFSET", args, 0)?)?;
    let rest = args.get(1..).unwrap_or_default();
    let Some(p) = prepare(i, env, "OFFSET", rest, true)? else {
        return empty_like(i, env, "OFFSET", rest);
    };
    let Some(cur) = p.current else {
        return pick(&p, 1, 0);
    };
    let ix = i64::try_from(cur).unwrap_or(0).saturating_add(delta);
    pick(&p, ix, ix)
}

fn is_rel_keyword(e: &Expr) -> Option<bool> {
    match &e.unparen().kind {
        ExprKind::Keyword(k) if k == "ABS" => Some(false),
        ExprKind::Keyword(k) if k == "REL" => Some(true),
        _ => None,
    }
}

/// `WINDOW(from[, from_type], to[, to_type][, relation][, ORDERBY(…)]…)`;
/// `from_type`/`to_type` are `ABS` or `REL` (default).
fn window(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("WINDOW", args, 2, usize::MAX)?;
    let mut k = 0;
    let from = int_arg(i, env, "WINDOW", arg("WINDOW", args, k)?)?;
    k += 1;
    let from_rel = match args.get(k).and_then(is_rel_keyword) {
        Some(r) => {
            k += 1;
            r
        }
        None => true,
    };
    let to = int_arg(i, env, "WINDOW", arg("WINDOW", args, k)?)?;
    k += 1;
    let to_rel = match args.get(k).and_then(is_rel_keyword) {
        Some(r) => {
            k += 1;
            r
        }
        None => true,
    };
    let rest = args.get(k..).unwrap_or_default();
    let Some(p) = prepare(i, env, "WINDOW", rest, from_rel || to_rel)? else {
        return empty_like(i, env, "WINDOW", rest);
    };
    let resolve = |v: i64, rel: bool| -> Option<i64> {
        if rel {
            p.current
                .map(|c| i64::try_from(c).unwrap_or(0).saturating_add(v))
        } else {
            Some(absolute(v, p.partition.len()))
        }
    };
    match (resolve(from, from_rel), resolve(to, to_rel)) {
        (Some(lo), Some(hi)) => pick(&p, lo, hi),
        _ => pick(&p, 1, 0),
    }
}

/// `ROWNUMBER([relation][, ORDERBY(…)]…)`: 1-based position of the
/// current row in its sorted partition (BLANK when undetermined).
fn rownumber(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    let Some(p) = prepare(i, env, "ROWNUMBER", args, true)? else {
        return Ok(Val::Scalar(Value::Blank));
    };
    Ok(Val::Scalar(p.current.map_or(Value::Blank, |c| {
        Value::Integer(i64::try_from(c).unwrap_or(i64::MAX).saturating_add(1))
    })))
}

/// `RANK([SKIP|DENSE][, relation][, ORDERBY(…)]…)`: rank of the current
/// row by its `ORDERBY` keys (ties share a rank).
fn rank(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    let dense = args
        .first()
        .is_some_and(|a| matches!(&a.unparen().kind, ExprKind::Keyword(k) if k == "DENSE"));
    let Some(p) = prepare(i, env, "RANK", args, true)? else {
        return Ok(Val::Scalar(Value::Blank));
    };
    let Some(current) = p.current.and_then(|c| p.keys.get(c)) else {
        return Ok(Val::Scalar(Value::Blank));
    };
    let mut ahead: Vec<&Vec<Value>> = p
        .keys
        .iter()
        .filter(|k| cmp_keys(k, current, &p.desc) == std::cmp::Ordering::Less)
        .collect();
    if dense {
        ahead.dedup_by(|a, b| cmp_keys(a, b, &p.desc) == std::cmp::Ordering::Equal);
    }
    let n = i64::try_from(ahead.len()).unwrap_or(i64::MAX);
    Ok(Val::Scalar(Value::Integer(n.saturating_add(1))))
}

fn helper_only(f: &str) -> R {
    Err(DaxError::argument(
        f,
        "can only be used as an argument of INDEX, OFFSET, WINDOW, RANK or ROWNUMBER",
    ))
}

fn orderby(_: &Interp<'_>, _: &Env, _: &[Expr]) -> R {
    helper_only("ORDERBY")
}

fn partitionby(_: &Interp<'_>, _: &Env, _: &[Expr]) -> R {
    helper_only("PARTITIONBY")
}

fn matchby(_: &Interp<'_>, _: &Env, _: &[Expr]) -> R {
    helper_only("MATCHBY")
}
