//! Fast paths: work the general evaluator does once per group or once per
//! row, done in one pass by the storage engine when the result is provably
//! the same. Every entry point returns `None` (or `Ok(None)`) when a guard
//! fails, and the caller then runs the general path, which stays the
//! reference semantics (the differential tests at the bottom compare both on
//! randomized models).
//!
//! # Grouped aggregation ([`summarize_columns`])
//!
//! `SUMMARIZECOLUMNS(groups…, "v", e…)` evaluates every `e` once per value
//! combination, in a filter context where each group column is filtered to
//! its value. When every `e` is a plain storage-engine aggregation over one
//! table `F` — `SUM`/`MIN`/`MAX`/`AVERAGE`/`COUNT`/`COUNTA`/`COUNTBLANK`/
//! `DISTINCTCOUNT`/`DISTINCTCOUNTNOBLANK` of a column of `F`, `COUNTROWS(F)`,
//! or a measure whose body is one of those (through other such measures) —
//! the rows a combination `c` sees are exactly the rows of `F` visible in
//! the outer context whose (`RELATED`) group values are `c`, so one
//! row → group index over the visible rows of `F` gives every combination's
//! aggregates in a single scan ([`pbi_engine::EngineSnapshot::group_index`],
//! [`pbi_engine::EngineSnapshot::aggregate_groups`]). Guards:
//!
//! - no `ROLLUPADDISSUBTOTAL`/`ROLLUPGROUP` and no relationship overrides
//!   (`USERELATIONSHIP`/`CROSSFILTER`) in the context;
//! - no calculation item could apply to a measure: no group column of a
//!   calculation group, no filter on a calculation-group table, no
//!   calculation group with a `noSelectionExpression` (calculation items
//!   apply only to measure references, so plain aggregations are not
//!   affected);
//! - every group column belongs to `F` or to a table `G` of `F`'s expanded
//!   table whose filters reach `F` exactly along that many → one path
//!   ([`pbi_engine::RelationshipGraph::filters_follow_expanded_path`]): a
//!   filter on `G` then keeps the rows of `F` whose related row (or blank
//!   row) passes it, which is what the group index groups by;
//! - every combination found among the visible rows of `F` is also a
//!   combination `SUMMARIZECOLUMNS` would build from the visible values of
//!   each group table (checked; it also gives the output order);
//! - rows of `F` that relate to a blank row on the way to a group table
//!   with no blank row of its own are left out: propagation drops the
//!   blank row of a table that has none, so every filter on that group
//!   table hides them
//!   ([`pbi_engine::RelationshipGraph::rows_reachable_from`]);
//! - `COUNTBLANK`/`DISTINCTCOUNT` only when `F` has no blank row (they
//!   count it).
//!
//! Combinations without visible rows of `F` aggregate to BLANK on every
//! accepted aggregation and are dropped by `SUMMARIZECOLUMNS` anyway. Any
//! storage-engine error falls back to the general path, so errors are
//! reported exactly as before.
//!
//! # Columnar iterators ([`Columnar`])
//!
//! `SUMX`/`AVERAGEX`/`MINX`/`MAXX`/`COUNTX`/`COUNTAX` over the rows of a
//! stored table (a table reference, `FILTER(…)`, `ALL(…)`, … without added
//! columns) whose row expression is built from columns of that table,
//! number literals, unary `-`/`+` and `+ - * / ^`: the expression is
//! compiled once and evaluated row by row on the column dictionaries with
//! the same operators ([`ops::arith`]), without a row context per row.
//! Bare `[Name]` must not name a measure (that would be a context
//! transition).

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use pbi_engine::{Agg, EncodedColumn, FilterContext, RowSet, Value, ValueKey};
use pbi_model::ColumnRef;
use pbi_model::names::{name_key, names_eq};

use super::context::{Env, VarScope};
use super::error::{MAX_EVAL_DEPTH, Result};
use super::interp::Interp;
use super::ops;
use super::table::{Rows, VirtualTable};
use crate::syntax::{BinaryOp, Expr, ExprKind, Number, UnaryOp};

/// Longest measure → measure chain followed when planning an aggregation.
const MAX_MEASURE_CHAIN: usize = 8;
/// Deepest row expression compiled by [`Columnar`].
const MAX_ROW_EXPR_DEPTH: usize = 64;
/// Groups / rows between cancellation checks.
const CANCEL_CHECK_EVERY: usize = 4096;

/// A plain storage-engine aggregation: `SUM(T[c])`, …, `COUNTROWS(T)`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SimpleAgg {
    /// Aggregated table (stored name).
    pub table: String,
    /// Aggregated column (empty for `COUNTROWS`).
    pub column: String,
    pub agg: Agg,
}

/// `e` as a [`SimpleAgg`] when evaluating it in `env` (no row context) is
/// exactly that aggregation over the rows of its table visible in
/// `env.filter`.
pub(crate) fn simple_agg(i: &Interp<'_>, env: &Env, e: &Expr) -> Option<SimpleAgg> {
    simple_agg_at(i, env, e, 0)
}

fn simple_agg_at(i: &Interp<'_>, env: &Env, e: &Expr, chain: usize) -> Option<SimpleAgg> {
    if chain > MAX_MEASURE_CHAIN || !env.rows.is_empty() {
        return None;
    }
    match &e.unparen().kind {
        ExprKind::Call { name, args, .. } => {
            if i.functions.contains_key(&name_key(name)) {
                return None;
            }
            let agg = match name.as_str() {
                "SUM" => Agg::Sum,
                "MIN" => Agg::Min,
                "MAX" => Agg::Max,
                "AVERAGE" => Agg::Average,
                "COUNT" => Agg::Count,
                "COUNTA" => Agg::CountNonBlank,
                "COUNTBLANK" => Agg::CountBlank,
                "DISTINCTCOUNT" => Agg::DistinctCount,
                "DISTINCTCOUNTNOBLANK" => Agg::DistinctCountNoBlank,
                "COUNTROWS" => return countrows_agg(i, env, args),
                _ => return None,
            };
            let [arg] = args.as_slice() else {
                return None;
            };
            let ExprKind::ColumnRef { table, column } = &arg.unparen().kind else {
                return None;
            };
            let (table, column) = i.resolve_column(env, table.as_deref(), column).ok()?;
            Some(SimpleAgg { table, column, agg })
        }
        ExprKind::ColumnRef { table, column } => {
            let m = i.find_measure(table.as_deref(), column).ok()??;
            if i.measure_on_stack(&m.name) || i.calc_groups_may_apply(&env.filter, []) {
                return None;
            }
            let menv = Env {
                filter: Arc::clone(&env.filter),
                rows: Default::default(),
                vars: VarScope::new(),
                home: Some(Arc::from(m.table.as_str())),
                shadow: Arc::clone(&env.shadow),
                scope: Arc::clone(&env.scope),
                applied: env.applied.clone(),
                selected: None,
            };
            simple_agg_at(i, &menv, &m.expr, chain + 1)
        }
        _ => None,
    }
}

/// `COUNTROWS(T)` (stored table reference) or `COUNTROWS()` in a measure of
/// a stored table.
fn countrows_agg(i: &Interp<'_>, env: &Env, args: &[Expr]) -> Option<SimpleAgg> {
    let name = match args {
        [] => env.home.as_deref()?,
        [a] => match &a.unparen().kind {
            ExprKind::TableRef { name } if !i.query_tables.contains_key(&name_key(name)) => {
                name.as_str()
            }
            _ => return None,
        },
        _ => return None,
    };
    let data = i.snapshot.table(name).ok()?;
    Some(SimpleAgg {
        table: data.name().to_owned(),
        column: String::new(),
        agg: Agg::CountRows,
    })
}

/// One-pass `SUMMARIZECOLUMNS` without rollups (see the module docs).
/// `group` are the resolved group-by columns, `by_table` the same columns
/// per table (tables in order of first appearance, with output positions),
/// `exprs` the extension expressions with their `IGNORE` flag, `fctx` the
/// filter context after the filter arguments. Returns the output rows
/// (group values, then expression values; rows whose counted values are all
/// BLANK dropped) in the order of the general path, or `None` to fall back.
pub(crate) fn summarize_columns(
    i: &Interp<'_>,
    env: &Env,
    fctx: &Arc<FilterContext>,
    group: &[(String, String)],
    by_table: &[(String, Vec<(usize, String)>)],
    exprs: &[(&Expr, bool)],
) -> Result<Option<Vec<Vec<Value>>>> {
    if !i.fast_paths || group.is_empty() || exprs.is_empty() || fctx.has_relationship_overrides() {
        return Ok(None);
    }
    // A calculation item applied per row (group-by column of a calculation
    // group) or to every row changes the measures: general path.
    if i.calc_groups_may_apply(fctx, group.iter().map(|(t, _)| t.as_str())) {
        return Ok(None);
    }
    let cenv = env.with_filter(Arc::clone(fctx));
    let Some(aggs) = exprs
        .iter()
        .map(|(e, _)| simple_agg(i, &cenv, e))
        .collect::<Option<Vec<_>>>()
    else {
        return Ok(None);
    };
    let Some(base) = aggs.first().map(|a| a.table.clone()) else {
        return Ok(None);
    };
    let graph = i.snapshot.graph();
    if aggs.iter().any(|a| !names_eq(&a.table, &base))
        || (graph.has_blank_row(&base)
            && aggs
                .iter()
                .any(|a| matches!(a.agg, Agg::CountBlank | Agg::DistinctCount)))
        || !by_table
            .iter()
            .all(|(t, _)| graph.filters_follow_expanded_path(&base, t))
    {
        return Ok(None);
    }
    let Some(index) = engine(group_index(i, fctx, &base, group, by_table)) else {
        return Ok(None);
    };
    i.check_cancel()?;
    let order: Vec<usize> = if let [(t, _)] = by_table
        && names_eq(t, &base)
    {
        // `SUMMARIZECOLUMNS` lists the combinations of one table in order of
        // first appearance among its visible rows: the index's order.
        (0..index.len()).collect()
    } else {
        match combination_order(i, fctx, by_table, &index)? {
            Some(order) => order,
            None => return Ok(None),
        }
    };
    let mut columns = Vec::with_capacity(aggs.len());
    for a in &aggs {
        match i.snapshot.aggregate_groups(&index, &a.column, a.agg) {
            Ok(values) => columns.push(values),
            Err(_) => return Ok(None),
        }
    }
    let mut out = Vec::with_capacity(order.len());
    for (n, g) in order.into_iter().enumerate() {
        if n % CANCEL_CHECK_EVERY == 0 {
            i.check_cancel()?;
        }
        let values: Vec<Value> = columns
            .iter()
            .map(|c| c.get(g).cloned().unwrap_or_default())
            .collect();
        let mut counted = values
            .iter()
            .zip(exprs)
            .filter(|(_, (_, ignored))| !ignored)
            .map(|(v, _)| v)
            .peekable();
        let empty = if counted.peek().is_none() {
            values.iter().all(Value::is_blank)
        } else {
            counted.all(Value::is_blank)
        };
        if empty {
            continue;
        }
        let Some(combo) = index.values().get(g) else {
            continue;
        };
        let mut row = combo.clone();
        row.extend(values);
        out.push(row);
    }
    i.fast_path_hits.set(i.fast_path_hits.get() + 1);
    Ok(Some(out))
}

/// `Some` for an engine result, `None` for an engine error (the general
/// path reports it).
fn engine<T>(r: pbi_engine::Result<T>) -> Option<T> {
    r.ok()
}

/// Groups the rows of `base` visible in `fctx` by the group columns. Rows
/// that the filter on some group table hides whatever its value (see
/// [`pbi_engine::RelationshipGraph::rows_reachable_from`]) belong to no
/// combination and are left out.
fn group_index(
    i: &Interp<'_>,
    fctx: &FilterContext,
    base: &str,
    group: &[(String, String)],
    by_table: &[(String, Vec<(usize, String)>)],
) -> pbi_engine::Result<pbi_engine::GroupIndex> {
    let mut rows = i.snapshot.visible(fctx, base)?.rows;
    for (t, _) in by_table {
        if !names_eq(t, base) {
            rows = i.snapshot.graph().rows_reachable_from(base, t, &rows);
        }
    }
    let refs: Vec<ColumnRef> = group
        .iter()
        .map(|(t, c)| ColumnRef::new(t.as_str(), c.as_str()))
        .collect();
    i.snapshot.group_index(base, &refs, &rows)
}

/// Positions of the groups of `index` in the order the general path lists
/// their combinations (cross product of every group table's visible
/// combinations, see `visible_combos`), or `None` when a group's values are
/// not among them.
fn combination_order(
    i: &Interp<'_>,
    fctx: &FilterContext,
    by_table: &[(String, Vec<(usize, String)>)],
    index: &pbi_engine::GroupIndex,
) -> Result<Option<Vec<usize>>> {
    let mut ranks: Vec<Vec<usize>> = vec![Vec::with_capacity(by_table.len()); index.len()];
    for (table, cols) in by_table {
        let names: Vec<String> = cols.iter().map(|(_, c)| c.clone()).collect();
        let Some(combos) = engine(table_combos(i, fctx, table, &names)) else {
            return Ok(None);
        };
        i.check_cancel()?;
        let rank: HashMap<Vec<ValueKey>, usize> = combos
            .iter()
            .enumerate()
            .map(|(r, c)| (c.iter().map(ValueKey::from).collect(), r))
            .collect();
        for (values, out) in index.values().iter().zip(&mut ranks) {
            let key: Vec<ValueKey> = cols
                .iter()
                .map(|(pos, _)| ValueKey::from(values.get(*pos).unwrap_or(&Value::Blank)))
                .collect();
            match rank.get(&key) {
                Some(r) => out.push(*r),
                None => return Ok(None),
            }
        }
    }
    let mut order: Vec<usize> = (0..index.len()).collect();
    order.sort_by(|a, b| ranks.get(*a).cmp(&ranks.get(*b)));
    Ok(Some(order))
}

/// The value combinations of `cols` of `table` visible in `filter`, as
/// `SUMMARIZECOLUMNS` lists them (blank-row combination last).
pub(crate) fn table_combos(
    i: &Interp<'_>,
    filter: &FilterContext,
    table: &str,
    cols: &[String],
) -> pbi_engine::Result<Vec<Vec<Value>>> {
    let vis = i.snapshot.visible(filter, table)?;
    let refs: Vec<ColumnRef> = cols.iter().map(|c| ColumnRef::new(table, c)).collect();
    let mut combos = i
        .snapshot
        .group_index(table, &refs, &vis.rows)?
        .into_values();
    if vis.blank_row && !combos.iter().any(|c| c.iter().all(Value::is_blank)) {
        combos.push(vec![Value::Blank; cols.len()]);
    }
    Ok(combos)
}

// ---- columnar iterators --------------------------------------------------------

/// A row expression compiled against the columns of a stored table.
#[derive(Debug)]
enum RowExpr {
    Const(Value),
    /// Index into [`Columnar::columns`].
    Column(usize),
    Unary(UnaryOp, Box<RowExpr>),
    Binary(BinaryOp, Box<RowExpr>, Box<RowExpr>),
}

/// An iterator's row expression over a stored table's rows, evaluated
/// column-wise (see the module docs).
#[derive(Debug)]
pub(crate) struct Columnar {
    rows: RowSet,
    blank_row: bool,
    /// Encodings of the referenced columns (`None`: the column cannot be
    /// encoded, which the general path reads as BLANK).
    columns: Vec<Option<Arc<EncodedColumn>>>,
    expr: RowExpr,
}

impl Columnar {
    /// Compiles `expr` for the rows of `table` (the innermost row context);
    /// `None` when the table or the expression is not supported.
    pub(crate) fn compile(i: &Interp<'_>, table: &VirtualTable, expr: &Expr) -> Option<Self> {
        if !i.fast_paths {
            return None;
        }
        let (Some(data), Rows::Set { rows, blank_row }) = (&table.base, &table.rows) else {
            return None;
        };
        if table.columns.len() != data.batch().num_columns() {
            return None;
        }
        let mut refs: Vec<usize> = Vec::new();
        let budget = MAX_EVAL_DEPTH.saturating_sub(i.depth() + 8);
        let expr = compile(i, table, expr, 0, budget.min(MAX_ROW_EXPR_DEPTH), &mut refs)?;
        let columns = refs
            .iter()
            .map(|c| data.encoded_at(*c).ok())
            .collect::<Vec<_>>();
        i.fast_path_hits.set(i.fast_path_hits.get() + 1);
        Some(Self {
            rows: rows.clone(),
            blank_row: *blank_row,
            columns,
            expr,
        })
    }

    /// Calls `f` with the value of every row, in iteration order (the blank
    /// row last). Stops at the first error, like the general path.
    pub(crate) fn for_each(
        &self,
        i: &Interp<'_>,
        mut f: impl FnMut(Cow<'_, Value>) -> Result<()>,
    ) -> Result<()> {
        for (n, row) in self.rows.iter().enumerate() {
            if n % CANCEL_CHECK_EVERY == 0 {
                i.check_cancel()?;
            }
            f(self.eval(&self.expr, Some(row))?)?;
        }
        if self.blank_row {
            f(self.eval(&self.expr, None)?)?;
        }
        Ok(())
    }

    /// Value of `e` at `row` (`None`: the blank row).
    fn eval<'s>(&'s self, e: &'s RowExpr, row: Option<usize>) -> Result<Cow<'s, Value>> {
        Ok(match e {
            RowExpr::Const(v) => Cow::Borrowed(v),
            RowExpr::Column(c) => match (row, self.columns.get(*c).and_then(Option::as_ref)) {
                (Some(r), Some(enc)) => enc
                    .value_at(r)
                    .map_or(Cow::Owned(Value::Blank), Cow::Borrowed),
                _ => Cow::Owned(Value::Blank),
            },
            RowExpr::Unary(op, inner) => {
                let v = self.eval(inner, row)?;
                Cow::Owned(match op {
                    UnaryOp::Plus => ops::to_numeric(&v)?,
                    _ => ops::negate(&v)?,
                })
            }
            RowExpr::Binary(op, lhs, rhs) => {
                let l = self.eval(lhs, row)?;
                let r = self.eval(rhs, row)?;
                Cow::Owned(ops::arith(*op, &l, &r)?)
            }
        })
    }
}

/// Compiles `e` (see [`Columnar`]); `refs` collects the referenced column
/// positions, [`RowExpr::Column`] indexes it.
fn compile(
    i: &Interp<'_>,
    table: &VirtualTable,
    e: &Expr,
    depth: usize,
    max_depth: usize,
    refs: &mut Vec<usize>,
) -> Option<RowExpr> {
    if depth >= max_depth {
        return None;
    }
    let sub = |e: &Expr, refs: &mut Vec<usize>| compile(i, table, e, depth + 1, max_depth, refs);
    Some(match &e.kind {
        ExprKind::Paren(inner) => sub(inner, refs)?,
        ExprKind::Number(Number::Integer(n)) => RowExpr::Const(Value::Integer(*n)),
        ExprKind::Number(Number::Decimal(d)) => RowExpr::Const(Value::Double(*d)),
        ExprKind::Unary { op, expr } if matches!(op, UnaryOp::Plus | UnaryOp::Minus) => {
            RowExpr::Unary(*op, Box::new(sub(expr, refs)?))
        }
        ExprKind::Binary { op, lhs, rhs }
            if matches!(
                op,
                BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Pow
            ) =>
        {
            let l = sub(lhs, refs)?;
            let r = sub(rhs, refs)?;
            RowExpr::Binary(*op, Box::new(l), Box::new(r))
        }
        ExprKind::ColumnRef {
            table: qualifier,
            column,
        } => {
            // Bare `[Name]` is a measure first (a context transition).
            if qualifier.is_none() && !matches!(i.find_measure(None, column), Ok(None)) {
                return None;
            }
            // The iterated table is the innermost row context, so it wins
            // over outer frames when it has the column.
            let pos = table.find_exact(qualifier.as_deref(), column)?;
            let slot = match refs.iter().position(|r| *r == pos) {
                Some(s) => s,
                None => {
                    refs.push(pos);
                    refs.len() - 1
                }
            };
            RowExpr::Column(slot)
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
