//! Table functions: `FILTER`, `ALL`, `ALLSELECTED`, `VALUES`, `FILTERS`,
//! `SUMMARIZE`, `SUMMARIZECOLUMNS` (with `ROLLUPADDISSUBTOTAL`/`ROLLUPGROUP`
//! and `IGNORE`), `GROUPBY`/`CURRENTGROUP`, `SELECTCOLUMNS`, `ADDCOLUMNS`,
//! `TOPN`, `GENERATE`, natural joins, set operations, constructors,
//! `CALCULATETABLE`/`RELATEDTABLE` …

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use arrow::datatypes::DataType as ArrowType;
use pbi_engine::value::{DECIMAL_PRECISION, DECIMAL_SCALE};
use pbi_engine::{ColumnFilter, FilterContext, RowSet, TableFilter, Value, ValueKey};
use pbi_model::ColumnRef;
use pbi_model::names::{name_key, names_eq};

use super::aggregate::{cmp_keys, order_specs, sort_positions};
use super::{Builtin, arg, arity, name_arg, opt_arg, scalar_fn, table_fn};
use crate::eval::context::{Env, RowFrame, VarValue};
use crate::eval::error::{DaxError, Result};
use crate::eval::fast;
use crate::eval::filters::apply_allselected;
use crate::eval::interp::{Interp, Val};
use crate::eval::ops;
use crate::eval::table::{RowPos, VColumn, VirtualTable, group_positions};
use crate::syntax::{Expr, ExprKind};

/// Upper bound on rows produced by `CROSSJOIN`/`GENERATESERIES`/
/// `SUMMARIZECOLUMNS` cross products.
const MAX_GENERATED_ROWS: usize = 5_000_000;

pub(super) const BUILTINS: &[Builtin] = &[
    table_fn("FILTER", filter),
    table_fn("ALL", all),
    table_fn("ALLNOBLANKROW", allnoblankrow),
    table_fn("REMOVEFILTERS", removefilters),
    table_fn("ALLEXCEPT", allexcept),
    table_fn("VALUES", values),
    table_fn("DISTINCT", distinct),
    table_fn("SUMMARIZE", summarize),
    table_fn("SUMMARIZECOLUMNS", summarizecolumns),
    table_fn("SELECTCOLUMNS", selectcolumns),
    table_fn("ADDCOLUMNS", addcolumns),
    table_fn("TOPN", topn),
    table_fn("UNION", union),
    table_fn("INTERSECT", intersect),
    table_fn("EXCEPT", except),
    table_fn("ROW", row),
    table_fn("DATATABLE", datatable),
    table_fn("RELATEDTABLE", relatedtable),
    table_fn("CALCULATETABLE", calculatetable),
    table_fn("CROSSJOIN", crossjoin),
    table_fn("TREATAS", treatas),
    table_fn("KEEPFILTERS", keepfilters),
    table_fn("GENERATESERIES", generateseries),
    table_fn("ALLSELECTED", allselected),
    table_fn("FILTERS", filters),
    table_fn("GENERATE", generate),
    table_fn("GENERATEALL", generateall),
    table_fn("NATURALINNERJOIN", naturalinnerjoin),
    table_fn("NATURALLEFTOUTERJOIN", naturalleftouterjoin),
    table_fn("GROUPBY", groupby),
    table_fn("CURRENTGROUP", currentgroup),
    table_fn("ROLLUPADDISSUBTOTAL", rollup_only),
    table_fn("ROLLUPGROUP", rollup_only),
    scalar_fn("IGNORE", ignore),
];

/// Variable holding the current group of `GROUPBY` (not a valid DAX name).
const CURRENTGROUP_VAR: &str = "<currentgroup>";

type R = Result<Val>;

/// Group values and, for stored tables, the group's rows (+ blank row).
type SummaryGroup = (Vec<Value>, Option<(RowSet, bool)>);

fn tbl(t: VirtualTable) -> R {
    Ok(Val::Table(Arc::new(t)))
}

fn filter(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("FILTER", args, 2, 2)?;
    let table = i.table(env, arg("FILTER", args, 0)?)?;
    let pred = arg("FILTER", args, 1)?;
    let mut keep = Vec::new();
    i.iterate(env, &table, |row_env, pos| {
        if i.condition(row_env, pred)? {
            keep.push(pos);
        }
        Ok(true)
    })?;
    tbl(table.subset(&keep))
}

/// Name of a table-reference argument, if it is one.
fn table_name(e: &Expr) -> Option<&str> {
    match &e.unparen().kind {
        ExprKind::TableRef { name } | ExprKind::Keyword(name) => Some(name),
        _ => None,
    }
}

fn all_impl(i: &Interp<'_>, env: &Env, args: &[Expr], f: &str, with_blank: bool) -> R {
    if args.is_empty() {
        return Err(DaxError::argument(
            f,
            "without arguments it can only be used as a CALCULATE filter",
        ));
    }
    let first = arg(f, args, 0)?;
    if let Some(name) = table_name(first) {
        if args.len() > 1 {
            return Err(DaxError::argument(f, "expects a single table reference"));
        }
        if let Some(t) = i.query_tables.get(&name_key(name)) {
            return Ok(Val::Table(Arc::clone(t)));
        }
        let data = i.snapshot.table(name).map_err(|_| DaxError::UnknownTable {
            name: name.to_owned(),
        })?;
        let blank = with_blank && i.snapshot.graph().has_blank_row(data.name());
        let n = data.row_count();
        return tbl(VirtualTable::from_base(
            Arc::clone(data),
            RowSet::all(n),
            blank,
        ));
    }
    let mut table: Option<String> = None;
    let mut cols = Vec::with_capacity(args.len());
    for a in args {
        let (t, c) = i.column_arg(env, a, f)?;
        match &table {
            Some(existing) if !names_eq(existing, &t) => {
                return Err(DaxError::argument(
                    f,
                    "all columns must belong to the same table",
                ));
            }
            Some(_) => {}
            None => table = Some(t),
        }
        cols.push(c);
    }
    let table = table.unwrap_or_default();
    Ok(Val::Table(i.all_columns(&table, &cols, with_blank)?))
}

fn all(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    all_impl(i, env, args, "ALL", true)
}

fn allnoblankrow(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    all_impl(i, env, args, "ALLNOBLANKROW", false)
}

fn removefilters(_: &Interp<'_>, _: &Env, _: &[Expr]) -> R {
    Err(DaxError::argument(
        "REMOVEFILTERS",
        "can only be used as a CALCULATE filter argument",
    ))
}

fn allexcept(_: &Interp<'_>, _: &Env, _: &[Expr]) -> R {
    Err(DaxError::Unsupported {
        feature: "ALLEXCEPT outside of CALCULATE filter arguments".into(),
    })
}

/// A one-column table of `values` with the lineage of `table[column]`.
pub(super) fn column_table(
    i: &Interp<'_>,
    table: &str,
    column: &str,
    values: Vec<Value>,
) -> VirtualTable {
    let data_type = i
        .snapshot
        .table(table)
        .ok()
        .and_then(|d| d.column_type(column).ok().cloned());
    VirtualTable::materialized(
        vec![VColumn::model(table, column, data_type)],
        values.into_iter().map(|v| vec![v]).collect(),
    )
}

fn values(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("VALUES", args, 1, 1)?;
    let a = arg("VALUES", args, 0)?;
    if let Some(name) = table_name(a)
        && !i.query_tables.contains_key(&name_key(name))
        && let Ok(data) = i.snapshot.table(name)
    {
        let vis = i.snapshot.visible(&env.filter, data.name())?;
        return tbl(VirtualTable::from_base(
            Arc::clone(data),
            vis.rows,
            vis.blank_row,
        ));
    }
    if let ExprKind::ColumnRef { .. } = a.unparen().kind {
        let (t, c) = i.column_arg(env, a, "VALUES")?;
        let vals = i.snapshot.values(&env.filter, &t, &c)?;
        return tbl(column_table(i, &t, &c, vals));
    }
    Ok(Val::Table(i.table(env, a)?))
}

fn distinct(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("DISTINCT", args, 1, 1)?;
    let a = arg("DISTINCT", args, 0)?;
    if let ExprKind::ColumnRef { .. } = a.unparen().kind {
        let (t, c) = i.column_arg(env, a, "DISTINCT")?;
        let vis = i.snapshot.visible(&env.filter, &t)?;
        let vals = i.snapshot.distinct(&t, &c, &vis.rows)?;
        return tbl(column_table(i, &t, &c, vals));
    }
    tbl(i.table(env, a)?.distinct())
}

/// `"name", expr` pairs starting at `args[from]`.
fn name_expr_pairs<'e>(f: &str, args: &'e [Expr], from: usize) -> Result<Vec<(String, &'e Expr)>> {
    let rest = args.get(from..).unwrap_or_default();
    if rest.len() % 2 != 0 {
        return Err(DaxError::argument(
            f,
            "expects pairs of a column name and an expression",
        ));
    }
    let (pairs, _) = rest.as_chunks::<2>();
    pairs
        .iter()
        .map(|[name, expr]| Ok((name_arg(f, name)?, expr)))
        .collect()
}

/// Column metadata for `name = expr` over rows of `source`: a plain column
/// reference keeps its lineage and type.
fn derived_column(
    i: &Interp<'_>,
    env: &Env,
    source: &VirtualTable,
    name: String,
    expr: &Expr,
    keep_table: bool,
) -> VColumn {
    if let ExprKind::ColumnRef { table, column } = &expr.unparen().kind {
        if let Some(idx) = source.find_column(table.as_deref(), column)
            && let Some(src) = source.columns.get(idx)
        {
            return VColumn {
                table: if keep_table { src.table.clone() } else { None },
                name,
                lineage: src.lineage.clone(),
                data_type: src.data_type.clone(),
            };
        }
        if let Ok((t, c)) = i.resolve_column(env, table.as_deref(), column)
            && i.find_measure(table.as_deref(), column)
                .ok()
                .flatten()
                .is_none()
        {
            let data_type = i
                .snapshot
                .table(&t)
                .ok()
                .and_then(|d| d.column_type(&c).ok().cloned());
            return VColumn {
                table: keep_table.then(|| t.clone()),
                name,
                lineage: Some(ColumnRef::new(t, c)),
                data_type,
            };
        }
    }
    VColumn::new(name)
}

/// Evaluates `exprs` for every row of `table`.
fn row_values(
    i: &Interp<'_>,
    env: &Env,
    table: &Arc<VirtualTable>,
    exprs: &[&Expr],
) -> Result<Vec<Vec<Value>>> {
    let mut out = Vec::with_capacity(table.row_count());
    i.iterate(env, table, |row_env, _| {
        out.push(
            exprs
                .iter()
                .map(|e| i.scalar(row_env, e))
                .collect::<Result<Vec<_>>>()?,
        );
        Ok(true)
    })?;
    Ok(out)
}

fn selectcolumns(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("SELECTCOLUMNS", args, 1, usize::MAX)?;
    let table = i.table(env, arg("SELECTCOLUMNS", args, 0)?)?;
    let mut columns = Vec::new();
    let mut exprs: Vec<&Expr> = Vec::new();
    let mut k = 1;
    while let Some(a) = args.get(k) {
        match &a.unparen().kind {
            ExprKind::String(name) => {
                let expr = arg("SELECTCOLUMNS", args, k + 1)?;
                columns.push(derived_column(i, env, &table, name.clone(), expr, false));
                exprs.push(expr);
                k += 2;
            }
            ExprKind::ColumnRef { column, .. } => {
                columns.push(derived_column(i, env, &table, column.clone(), a, true));
                exprs.push(a);
                k += 1;
            }
            _ => {
                return Err(DaxError::argument(
                    "SELECTCOLUMNS",
                    "expects column names in double quotes followed by expressions",
                ));
            }
        }
    }
    let rows = row_values(i, env, &table, &exprs)?;
    tbl(VirtualTable::materialized(columns, rows))
}

fn addcolumns(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("ADDCOLUMNS", args, 3, usize::MAX)?;
    let table = i.table(env, arg("ADDCOLUMNS", args, 0)?)?;
    let pairs = name_expr_pairs("ADDCOLUMNS", args, 1)?;
    let columns = pairs
        .iter()
        .map(|(name, e)| derived_column(i, env, &table, name.clone(), e, false))
        .collect();
    let exprs: Vec<&Expr> = pairs.iter().map(|(_, e)| *e).collect();
    let rows = row_values(i, env, &table, &exprs)?;
    tbl(table.with_columns(columns, rows))
}

fn summarize(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("SUMMARIZE", args, 1, usize::MAX)?;
    let table = i.table(env, arg("SUMMARIZE", args, 0)?)?;
    let mut k = 1;
    let mut group_refs: Vec<(Option<String>, String)> = Vec::new();
    while let Some(a) = args.get(k) {
        match &a.unparen().kind {
            ExprKind::ColumnRef { table, column } => {
                group_refs.push((table.clone(), column.clone()));
                k += 1;
            }
            ExprKind::Call { name, .. } if name.starts_with("ROLLUP") => {
                return Err(DaxError::UnsupportedFunction { name: name.clone() });
            }
            _ => break,
        }
    }
    let pairs = name_expr_pairs("SUMMARIZE", args, k)?;

    // Group values (and, for stored tables, the rows of each group).
    let mut columns: Vec<VColumn> = Vec::new();
    let mut groups: Vec<SummaryGroup> = Vec::new();
    if let (Some(data), Some((rows, blank))) = (&table.base, table.base_rowset()) {
        let mut refs = Vec::new();
        for (t, c) in &group_refs {
            let (t, c) = i.resolve_column(env, t.as_deref(), c)?;
            let data_type = i
                .snapshot
                .table(&t)
                .ok()
                .and_then(|d| d.column_type(&c).ok().cloned());
            columns.push(VColumn::model(&t, &c, data_type));
            refs.push(ColumnRef::new(t, c));
        }
        for g in i.snapshot.group_by(data.name(), &refs, &rows)? {
            groups.push((g.values, Some((g.rows, false))));
        }
        if blank {
            let blank_values = vec![Value::Blank; refs.len()];
            match groups
                .iter_mut()
                .find(|(v, _)| v.iter().all(Value::is_blank))
            {
                Some((_, Some(set))) => set.1 = true,
                _ => groups.push((blank_values, Some((RowSet::empty(data.row_count()), true)))),
            }
        }
    } else {
        let mut idx = Vec::new();
        for (t, c) in &group_refs {
            let col = table
                .find_column(t.as_deref(), c)
                .ok_or_else(|| DaxError::unknown_column(t.as_deref().unwrap_or_default(), c))?;
            columns.push(
                table
                    .columns
                    .get(col)
                    .cloned()
                    .unwrap_or_else(|| VColumn::new(c)),
            );
            idx.push(col);
        }
        for (values, _) in group_positions(&table, &idx) {
            groups.push((values, None));
        }
    }
    let group_table = Arc::new(VirtualTable::materialized(
        columns,
        groups.iter().map(|(v, _)| v.clone()).collect(),
    ));
    if pairs.is_empty() {
        return Ok(Val::Table(group_table));
    }
    let mut ext_values = Vec::with_capacity(groups.len());
    for (gi, (_, rows)) in groups.iter().enumerate() {
        i.check_cancel()?;
        let mut genv = env.clone();
        if let (Some(data), Some((set, blank))) = (&table.base, rows) {
            let mut f: FilterContext = (*env.filter).clone();
            f.add(TableFilter::new(data.name(), set.clone()).with_blank_row(*blank));
            genv.filter = Arc::new(f);
        }
        let genv = genv.with_row(RowFrame::new(Arc::clone(&group_table), RowPos::Item(gi)));
        ext_values.push(
            pairs
                .iter()
                .map(|(_, e)| i.scalar(&genv, e))
                .collect::<Result<Vec<_>>>()?,
        );
    }
    let ext_columns = pairs.iter().map(|(n, _)| VColumn::new(n.clone())).collect();
    tbl(group_table.with_columns(ext_columns, ext_values))
}

/// Value combinations of `cols` (all of `table`) visible in `filter`.
pub(super) fn visible_combos(
    i: &Interp<'_>,
    filter: &FilterContext,
    table: &str,
    cols: &[String],
) -> Result<Vec<Vec<Value>>> {
    Ok(fast::table_combos(i, filter, table, cols)?)
}

/// A `ROLLUPADDISSUBTOTAL` group: levels (indices into the group-by
/// columns; a `ROLLUPGROUP` level has several) and their subtotal flag
/// column names, outermost first.
struct Rollup {
    levels: Vec<Vec<usize>>,
    flags: Vec<String>,
}

/// Group-by columns of `ROLLUPADDISSUBTOTAL(col | ROLLUPGROUP(…), "flag", …)`.
fn rollup_spec(
    i: &Interp<'_>,
    env: &Env,
    args: &[Expr],
    group: &mut Vec<(String, String)>,
) -> Result<Rollup> {
    let mut out = Rollup {
        levels: Vec::new(),
        flags: Vec::new(),
    };
    let mut k = 0;
    while let Some(a) = args.get(k) {
        let cols: Vec<&Expr> = match (&a.unparen().kind, a.as_call()) {
            (ExprKind::ColumnRef { .. }, _) => vec![a],
            (_, Some(("ROLLUPGROUP", inner))) => inner.iter().collect(),
            _ => {
                return Err(DaxError::Unsupported {
                    feature: "ROLLUPADDISSUBTOTAL with grand-total or group-level filters".into(),
                });
            }
        };
        let mut level = Vec::with_capacity(cols.len());
        for c in cols {
            level.push(group.len());
            group.push(i.column_arg(env, c, "ROLLUPADDISSUBTOTAL")?);
        }
        let flag = args.get(k + 1).ok_or_else(|| {
            DaxError::argument(
                "ROLLUPADDISSUBTOTAL",
                "each column must be followed by the name of its subtotal column",
            )
        })?;
        out.flags.push(name_arg("ROLLUPADDISSUBTOTAL", flag)?);
        out.levels.push(level);
        k += 2;
    }
    if out.levels.is_empty() {
        return Err(DaxError::argument(
            "ROLLUPADDISSUBTOTAL",
            "expects at least one column",
        ));
    }
    Ok(out)
}

fn summarizecolumns(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    let mut k = 0;
    let mut group: Vec<(String, String)> = Vec::new();
    let mut rollups: Vec<Rollup> = Vec::new();
    while let Some(a) = args.get(k) {
        if let ExprKind::ColumnRef { .. } = a.unparen().kind {
            group.push(i.column_arg(env, a, "SUMMARIZECOLUMNS")?);
        } else if let Some(("ROLLUPADDISSUBTOTAL", rargs)) = a.as_call() {
            rollups.push(rollup_spec(i, env, rargs, &mut group)?);
        } else if let Some(("ROLLUPGROUP", cols)) = a.as_call() {
            for c in cols {
                group.push(i.column_arg(env, c, "SUMMARIZECOLUMNS")?);
            }
        } else {
            break;
        }
        k += 1;
    }
    let mut filter_args: Vec<Expr> = Vec::new();
    while let Some(a) = args.get(k) {
        if matches!(a.unparen().kind, ExprKind::String(_)) {
            break;
        }
        match a.as_call() {
            Some((name, _)) if name.starts_with("ROLLUP") => {
                return Err(DaxError::argument(
                    "SUMMARIZECOLUMNS",
                    format!("{name} must come before the filter arguments"),
                ));
            }
            Some(("NONVISUAL", _)) => {
                return Err(DaxError::UnsupportedFunction {
                    name: "NONVISUAL".to_owned(),
                });
            }
            _ => {}
        }
        filter_args.push(a.clone());
        k += 1;
    }
    let pairs: Vec<(String, &Expr, bool)> = name_expr_pairs("SUMMARIZECOLUMNS", args, k)?
        .into_iter()
        .map(|(n, e)| match e.unparen().as_call() {
            Some(("IGNORE", [inner])) => (n, inner, true),
            _ => (n, e, false),
        })
        .collect();
    let fa = i.collect_filter_args(env, &filter_args, "SUMMARIZECOLUMNS")?;
    let fctx = Arc::new(i.apply_filter_args(&env.filter, fa));

    // Group columns by table, remembering their output positions.
    let mut by_table: Vec<(String, Vec<(usize, String)>)> = Vec::new();
    for (pos, (t, c)) in group.iter().enumerate() {
        match by_table.iter_mut().find(|(bt, _)| names_eq(bt, t)) {
            Some((_, cols)) => cols.push((pos, c.clone())),
            None => by_table.push((t.clone(), vec![(pos, c.clone())])),
        }
    }
    let width = group.len();
    if rollups.is_empty() && !pairs.is_empty() {
        let exprs: Vec<(&Expr, bool)> = pairs.iter().map(|(_, e, ig)| (*e, *ig)).collect();
        if let Some(rows) = fast::summarize_columns(i, env, &fctx, &group, &by_table, &exprs)? {
            let columns = summarize_columns_header(i, &group, &rollups, &pairs);
            return tbl(VirtualTable::materialized(columns, rows));
        }
    }
    let mut combos: Vec<Vec<Value>> = vec![vec![Value::Blank; width]];
    let common_base = if by_table.len() > 1 && pairs.is_empty() {
        i.snapshot.table_names().into_iter().find(|b| {
            by_table
                .iter()
                .all(|(t, _)| i.snapshot.graph().expanded_path(b, t).is_some())
        })
    } else {
        None
    };
    if let Some(base) = common_base {
        let vis = i.snapshot.visible(&fctx, &base)?;
        let refs: Vec<ColumnRef> = group
            .iter()
            .map(|(t, c)| ColumnRef::new(t.as_str(), c.as_str()))
            .collect();
        combos = i
            .snapshot
            .group_index(&base, &refs, &vis.rows)?
            .into_values();
    } else {
        for (t, cols) in &by_table {
            let names: Vec<String> = cols.iter().map(|(_, c)| c.clone()).collect();
            let table_combos = visible_combos(i, &fctx, t, &names)?;
            if combos.len().saturating_mul(table_combos.len()) > MAX_GENERATED_ROWS {
                return Err(DaxError::evaluation(
                    "SUMMARIZECOLUMNS: the result is too large",
                ));
            }
            let mut next = Vec::with_capacity(combos.len() * table_combos.len());
            for base in &combos {
                for tc in &table_combos {
                    let mut row = base.clone();
                    for ((pos, _), v) in cols.iter().zip(tc) {
                        if let Some(slot) = row.get_mut(*pos) {
                            *slot = v.clone();
                        }
                    }
                    next.push(row);
                }
            }
            combos = next;
        }
    }

    // Detail rows first, then every subtotal level (rolled-up columns BLANK).
    let flag_count: usize = rollups.iter().map(|r| r.flags.len()).sum();
    let mut states: Vec<Vec<usize>> = vec![vec![0; rollups.len()]];
    for (ri, r) in rollups.iter().enumerate() {
        let mut next = Vec::new();
        for st in &states {
            for depth in 0..=r.levels.len() {
                let mut s2 = st.clone();
                if let Some(slot) = s2.get_mut(ri) {
                    *slot = depth;
                }
                next.push(s2);
            }
        }
        states = next;
    }
    let mut out_rows = Vec::new();
    for state in &states {
        let mut rolled = vec![false; width];
        let mut flags = Vec::with_capacity(flag_count);
        for (r, depth) in rollups.iter().zip(state) {
            let first_rolled = r.levels.len() - depth;
            for (li, level) in r.levels.iter().enumerate() {
                let is_rolled = li >= first_rolled;
                flags.push(Value::Boolean(is_rolled));
                for c in level {
                    if let Some(slot) = rolled.get_mut(*c) {
                        *slot = is_rolled;
                    }
                }
            }
        }
        let mut seen: HashSet<Vec<ValueKey>> = HashSet::new();
        let scope: Arc<[ColumnRef]> = group
            .iter()
            .zip(&rolled)
            .filter(|(_, r)| !**r)
            .map(|((t, c), _)| ColumnRef::new(t.as_str(), c.as_str()))
            .collect();
        for combo in &combos {
            i.check_cancel()?;
            let combo: Vec<Value> = combo
                .iter()
                .zip(&rolled)
                .map(|(v, r)| if *r { Value::Blank } else { v.clone() })
                .collect();
            if rolled.iter().any(|r| *r) && !seen.insert(combo.iter().map(ValueKey::from).collect())
            {
                continue;
            }
            let mut row = combo.clone();
            row.extend(flags.iter().cloned());
            if pairs.is_empty() {
                out_rows.push(row);
                continue;
            }
            let mut f = (*fctx).clone();
            for (((t, c), v), r) in group.iter().zip(&combo).zip(&rolled) {
                if !*r {
                    f.replace_column(ColumnFilter::in_values(t.as_str(), c.as_str(), [v.clone()]));
                }
            }
            let mut cenv = env.with_filter(Arc::new(f));
            cenv.shadow = Arc::clone(&fctx);
            cenv.scope = Arc::clone(&scope);
            let values = pairs
                .iter()
                .map(|(_, e, _)| i.scalar(&cenv, e))
                .collect::<Result<Vec<_>>>()?;
            let counted: Vec<&Value> = values
                .iter()
                .zip(&pairs)
                .filter(|(_, (_, _, ignored))| !ignored)
                .map(|(v, _)| v)
                .collect();
            let empty = if counted.is_empty() {
                values.iter().all(Value::is_blank)
            } else {
                counted.iter().all(|v| v.is_blank())
            };
            if empty {
                continue;
            }
            row.extend(values);
            out_rows.push(row);
        }
    }
    let columns = summarize_columns_header(i, &group, &rollups, &pairs);
    tbl(VirtualTable::materialized(columns, out_rows))
}

/// Result columns of `SUMMARIZECOLUMNS`: group columns, subtotal flags,
/// extension columns.
fn summarize_columns_header(
    i: &Interp<'_>,
    group: &[(String, String)],
    rollups: &[Rollup],
    pairs: &[(String, &Expr, bool)],
) -> Vec<VColumn> {
    let mut columns: Vec<VColumn> = group
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
    for r in rollups {
        for name in &r.flags {
            let mut col = VColumn::new(name.clone());
            col.data_type = Some(ArrowType::Boolean);
            columns.push(col);
        }
    }
    columns.extend(pairs.iter().map(|(n, _, _)| VColumn::new(n.clone())));
    columns
}

fn rollup_only(_: &Interp<'_>, _: &Env, _: &[Expr]) -> R {
    Err(DaxError::argument(
        "ROLLUPADDISSUBTOTAL",
        "ROLLUPADDISSUBTOTAL and ROLLUPGROUP can only be used as group-by arguments of SUMMARIZECOLUMNS",
    ))
}

/// `IGNORE(expr)`: in `SUMMARIZECOLUMNS` the expression does not decide
/// whether a row is kept; elsewhere it is `expr`.
fn ignore(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("IGNORE", args, 1, 1)?;
    i.eval(env, arg("IGNORE", args, 0)?)
}

/// `ALLSELECTED(T)` / `ALLSELECTED(T[c], …)` as a table: the rows (value
/// combinations) visible once the filters on that scope are replaced by
/// the shadow context's (see [`crate::EvalContext::with_allselected`]).
fn allselected(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    if args.is_empty() {
        return Err(DaxError::argument(
            "ALLSELECTED",
            "without arguments it can only be used as a CALCULATE filter",
        ));
    }
    let target = i.selected_target(env, args)?;
    let mut f = (*env.filter).clone();
    let graph = i.snapshot.graph_for(&f);
    apply_allselected(&mut f, &target, &env.shadow, &graph);
    match target {
        crate::eval::filters::Selected::Table(t) => {
            let data = i.snapshot.table(&t)?;
            let vis = i.snapshot.visible(&f, data.name())?;
            tbl(VirtualTable::from_base(
                Arc::clone(data),
                vis.rows,
                vis.blank_row,
            ))
        }
        crate::eval::filters::Selected::Columns(cols) => {
            let Some((table, _)) = cols.first().cloned() else {
                return tbl(VirtualTable::empty(Vec::new()));
            };
            if cols.iter().any(|(t, _)| !names_eq(t, &table)) {
                return Err(DaxError::argument(
                    "ALLSELECTED",
                    "all columns must belong to the same table",
                ));
            }
            let names: Vec<String> = cols.iter().map(|(_, c)| c.clone()).collect();
            let rows = visible_combos(i, &f, &table, &names)?;
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
            tbl(VirtualTable::materialized(columns, rows))
        }
        crate::eval::filters::Selected::All => Err(DaxError::argument(
            "ALLSELECTED",
            "without arguments it can only be used as a CALCULATE filter",
        )),
    }
}

/// `FILTERS(T[c])`: the values allowed by the filters applied directly to
/// the column (all values when it is not filtered).
fn filters(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("FILTERS", args, 1, 1)?;
    let (t, c) = i.column_arg(env, arg("FILTERS", args, 0)?, "FILTERS")?;
    let values = super::info::direct_filter_values(i, env, &t, &c)?;
    tbl(column_table(i, &t, &c, values))
}

/// `GENERATE(t1, t2)`: for each row of `t1`, the rows of `t2` evaluated in
/// that row context, side by side (`all`: keep `t1` rows with no match,
/// with BLANKs).
fn generate_impl(i: &Interp<'_>, env: &Env, args: &[Expr], f: &str, all: bool) -> R {
    arity(f, args, 2, 2)?;
    let left = i.table(env, arg(f, args, 0)?)?;
    let right_expr = arg(f, args, 1)?;
    let mut right_columns: Option<Vec<VColumn>> = None;
    let mut rows: Vec<(Vec<Value>, Vec<Vec<Value>>)> = Vec::new();
    i.iterate(env, &left, |row_env, pos| {
        let right = i.table(row_env, right_expr)?;
        if right_columns.is_none() {
            right_columns = Some(right.columns().to_vec());
        }
        rows.push((left.row_values(pos), right.rows()));
        Ok(true)
    })?;
    let right_columns = match right_columns {
        Some(c) => c,
        None => i
            .table(env, right_expr)
            .map_or_else(|_| Vec::new(), |t| t.columns().to_vec()),
    };
    let width = right_columns.len();
    let mut out = Vec::new();
    for (l, rs) in rows {
        if rs.is_empty() && all {
            let mut row = l.clone();
            row.extend(std::iter::repeat_n(Value::Blank, width));
            out.push(row);
        }
        for r in rs {
            if out.len() >= MAX_GENERATED_ROWS {
                return Err(DaxError::evaluation(format!(
                    "{f}: the result is too large"
                )));
            }
            let mut row = l.clone();
            row.extend(
                r.into_iter()
                    .chain(std::iter::repeat(Value::Blank))
                    .take(width),
            );
            out.push(row);
        }
    }
    let mut columns = left.columns().to_vec();
    columns.extend(right_columns);
    tbl(VirtualTable::materialized(columns, out))
}

fn generate(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    generate_impl(i, env, args, "GENERATE", false)
}

fn generateall(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    generate_impl(i, env, args, "GENERATEALL", true)
}

/// Natural join on the columns with the same name (case-insensitive)
/// whose lineages, when both have one, agree.
fn natural_join(i: &Interp<'_>, env: &Env, args: &[Expr], f: &str, outer: bool) -> R {
    arity(f, args, 2, 2)?;
    let left = i.table(env, arg(f, args, 0)?)?;
    let right = i.table(env, arg(f, args, 1)?)?;
    let mut keys: Vec<(usize, usize)> = Vec::new();
    for (li, lc) in left.columns().iter().enumerate() {
        let found = right.columns().iter().position(|rc| {
            names_eq(&rc.name, &lc.name)
                && match (&lc.lineage, &rc.lineage) {
                    (Some(a), Some(b)) => a.matches(&b.table, &b.column),
                    _ => true,
                }
        });
        if let Some(ri) = found {
            keys.push((li, ri));
        }
    }
    if keys.is_empty() {
        return Err(DaxError::argument(f, "no common join columns were found"));
    }
    let right_keep: Vec<usize> = (0..right.column_count())
        .filter(|c| !keys.iter().any(|(_, r)| r == c))
        .collect();
    let mut index: HashMap<Vec<ValueKey>, Vec<Vec<Value>>> = HashMap::new();
    for p in right.positions() {
        let key: Vec<ValueKey> = keys
            .iter()
            .map(|(_, r)| ValueKey::from(right.value(p, *r)))
            .collect();
        let extra = right_keep.iter().map(|c| right.value(p, *c)).collect();
        index.entry(key).or_default().push(extra);
    }
    let mut out = Vec::new();
    for p in left.positions() {
        i.check_cancel()?;
        let key: Vec<ValueKey> = keys
            .iter()
            .map(|(l, _)| ValueKey::from(left.value(p, *l)))
            .collect();
        let base = left.row_values(p);
        match index.get(&key) {
            Some(matches) => {
                for extra in matches {
                    if out.len() >= MAX_GENERATED_ROWS {
                        return Err(DaxError::evaluation(format!(
                            "{f}: the result is too large"
                        )));
                    }
                    let mut row = base.clone();
                    row.extend(extra.iter().cloned());
                    out.push(row);
                }
            }
            None if outer => {
                let mut row = base;
                row.extend(std::iter::repeat_n(Value::Blank, right_keep.len()));
                out.push(row);
            }
            None => {}
        }
    }
    let mut columns = left.columns().to_vec();
    columns.extend(
        right_keep
            .iter()
            .filter_map(|c| right.columns().get(*c).cloned()),
    );
    tbl(VirtualTable::materialized(columns, out))
}

fn naturalinnerjoin(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    natural_join(i, env, args, "NATURALINNERJOIN", false)
}

fn naturalleftouterjoin(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    natural_join(i, env, args, "NATURALLEFTOUTERJOIN", true)
}

/// How a `GROUPBY` column is read from the grouped table.
enum GroupKey {
    Column(usize),
    /// A column of a table related to the (stored) base table.
    Related(String, String),
}

/// `GROUPBY(table, col…, "name", expr…)`: one row per combination of the
/// group-by columns (columns of the table or of related one-side tables);
/// each expression is evaluated with `CURRENTGROUP()` bound to the group's
/// rows.
fn groupby(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("GROUPBY", args, 1, usize::MAX)?;
    let table = i.table(env, arg("GROUPBY", args, 0)?)?;
    let mut k = 1;
    let mut keys = Vec::new();
    let mut columns = Vec::new();
    while let Some(a) = args.get(k) {
        let ExprKind::ColumnRef { table: t, column } = &a.unparen().kind else {
            break;
        };
        if let Some(ix) = table.find_column(t.as_deref(), column) {
            columns.push(
                table
                    .columns()
                    .get(ix)
                    .cloned()
                    .unwrap_or_else(|| VColumn::new(column.clone())),
            );
            keys.push(GroupKey::Column(ix));
        } else {
            let (rt, rc) = i.column_arg(env, a, "GROUPBY")?;
            let reachable = table
                .base_table()
                .is_some_and(|b| i.snapshot.graph().expanded_path(b, &rt).is_some());
            if !reachable {
                return Err(DaxError::argument(
                    "GROUPBY",
                    format!(
                        "the column {} is not in the table or a related table",
                        crate::eval::interp::snippet(a)
                    ),
                ));
            }
            let dt = i
                .snapshot
                .table(&rt)
                .ok()
                .and_then(|d| d.column_type(&rc).ok().cloned());
            columns.push(VColumn::model(&rt, &rc, dt));
            keys.push(GroupKey::Related(rt, rc));
        }
        k += 1;
    }
    let pairs = name_expr_pairs("GROUPBY", args, k)?;
    let base = table.base_table().map(str::to_owned);
    let mut index: HashMap<Vec<ValueKey>, usize> = HashMap::new();
    let mut groups: Vec<(Vec<Value>, Vec<RowPos>)> = Vec::new();
    for p in table.positions() {
        i.check_cancel()?;
        let mut values = Vec::with_capacity(keys.len());
        for key in &keys {
            values.push(match key {
                GroupKey::Column(ix) => table.value(p, *ix),
                GroupKey::Related(t, c) => match &base {
                    Some(b) if table.base_row(p) != crate::eval::table::BLANK_ROW => {
                        i.snapshot.related_value(b, table.base_row(p), t, c)?
                    }
                    _ => Value::Blank,
                },
            });
        }
        let key: Vec<ValueKey> = values.iter().map(ValueKey::from).collect();
        match index.get(&key) {
            Some(&g) => {
                if let Some(group) = groups.get_mut(g) {
                    group.1.push(p);
                }
            }
            None => {
                index.insert(key, groups.len());
                groups.push((values, vec![p]));
            }
        }
    }
    let group_table = Arc::new(VirtualTable::materialized(
        columns,
        groups.iter().map(|(v, _)| v.clone()).collect(),
    ));
    if pairs.is_empty() {
        return Ok(Val::Table(group_table));
    }
    let mut ext_values = Vec::with_capacity(groups.len());
    for (gi, (_, positions)) in groups.iter().enumerate() {
        i.check_cancel()?;
        let current = Arc::new(table.subset(positions));
        let genv = env
            .with_var(CURRENTGROUP_VAR, VarValue::Table(current))
            .with_row(RowFrame::new(Arc::clone(&group_table), RowPos::Item(gi)));
        ext_values.push(
            pairs
                .iter()
                .map(|(_, e)| i.scalar(&genv, e))
                .collect::<Result<Vec<_>>>()?,
        );
    }
    let ext_columns = pairs.iter().map(|(n, _)| VColumn::new(n.clone())).collect();
    tbl(group_table.with_columns(ext_columns, ext_values))
}

fn currentgroup(_: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("CURRENTGROUP", args, 0, 0)?;
    match env.vars.get(CURRENTGROUP_VAR) {
        Some(VarValue::Table(t)) => Ok(Val::Table(Arc::clone(t))),
        _ => Err(DaxError::argument(
            "CURRENTGROUP",
            "can only be used in the expressions of GROUPBY",
        )),
    }
}

fn topn(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("TOPN", args, 2, usize::MAX)?;
    let n = ops::to_i64(&i.scalar(env, arg("TOPN", args, 0)?)?)?;
    let table = i.table(env, arg("TOPN", args, 1)?)?;
    let Some(n) = n else {
        return Ok(Val::Table(table));
    };
    let n = usize::try_from(n).unwrap_or(0);
    if n == 0 {
        return tbl(table.subset(&[]));
    }
    let specs = order_specs(i, env, args.get(2..).unwrap_or_default(), true)?;
    if specs.is_empty() {
        let keep: Vec<RowPos> = table.positions().into_iter().take(n).collect();
        return tbl(table.subset_ordered(&keep));
    }
    let desc: Vec<bool> = specs.iter().map(|(_, d)| *d).collect();
    let sorted = sort_positions(i, env, &table, &specs)?;
    let mut end = n.min(sorted.len());
    while end > 0 && end < sorted.len() {
        let (Some(prev), Some(next)) = (sorted.get(end - 1), sorted.get(end)) else {
            break;
        };
        if cmp_keys(&prev.0, &next.0, &desc) != std::cmp::Ordering::Equal {
            break;
        }
        end += 1;
    }
    let keep: Vec<RowPos> = sorted.iter().take(end).map(|(_, p)| *p).collect();
    tbl(table.subset_ordered(&keep))
}

fn same_width(f: &str, tables: &[Arc<VirtualTable>]) -> Result<usize> {
    let width = tables.first().map_or(0, |t| t.column_count());
    if tables.iter().any(|t| t.column_count() != width) {
        return Err(DaxError::argument(
            f,
            "all tables must have the same number of columns",
        ));
    }
    Ok(width)
}

fn union(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("UNION", args, 2, usize::MAX)?;
    let tables = args
        .iter()
        .map(|a| i.table(env, a))
        .collect::<Result<Vec<_>>>()?;
    let width = same_width("UNION", &tables)?;
    let Some(first) = tables.first() else {
        return tbl(VirtualTable::empty(Vec::new()));
    };
    let columns: Vec<VColumn> = (0..width)
        .map(|c| {
            let mut col = first
                .columns
                .get(c)
                .cloned()
                .unwrap_or_else(|| VColumn::new(""));
            let same = |f: &dyn Fn(&VColumn) -> bool| {
                tables.iter().all(|t| t.columns.get(c).is_some_and(f))
            };
            if let Some(lin) = col.lineage.clone()
                && !same(&|o| {
                    o.lineage
                        .as_ref()
                        .is_some_and(|l| l.matches(&lin.table, &lin.column))
                })
            {
                col.lineage = None;
                col.table = None;
            }
            if let Some(dt) = col.data_type.clone()
                && !same(&|o| o.data_type.as_ref() == Some(&dt))
            {
                col.data_type = None;
            }
            col
        })
        .collect();
    let rows = tables.iter().flat_map(|t| t.rows()).collect();
    tbl(VirtualTable::materialized(columns, rows))
}

fn set_op(i: &Interp<'_>, env: &Env, args: &[Expr], f: &str, keep_common: bool) -> R {
    arity(f, args, 2, 2)?;
    let left = i.table(env, arg(f, args, 0)?)?;
    let right = i.table(env, arg(f, args, 1)?)?;
    same_width(f, &[Arc::clone(&left), Arc::clone(&right)])?;
    let keys: HashSet<Vec<ValueKey>> = right
        .positions()
        .into_iter()
        .map(|p| right.row_key(p))
        .collect();
    let keep: Vec<RowPos> = left
        .positions()
        .into_iter()
        .filter(|p| keys.contains(&left.row_key(*p)) == keep_common)
        .collect();
    tbl(left.subset(&keep))
}

fn intersect(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    set_op(i, env, args, "INTERSECT", true)
}

fn except(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    set_op(i, env, args, "EXCEPT", false)
}

fn row(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("ROW", args, 2, usize::MAX)?;
    let pairs = name_expr_pairs("ROW", args, 0)?;
    let mut columns = Vec::with_capacity(pairs.len());
    let mut values = Vec::with_capacity(pairs.len());
    for (name, e) in pairs {
        columns.push(VColumn::new(name));
        values.push(i.scalar(env, e)?);
    }
    tbl(VirtualTable::materialized(columns, vec![values]))
}

#[derive(Clone, Copy)]
enum DtType {
    Integer,
    Double,
    String,
    Boolean,
    Currency,
    DateTime,
}

fn datatable_type(e: &Expr) -> Result<DtType> {
    let word = match &e.unparen().kind {
        ExprKind::Keyword(k) => k.to_ascii_uppercase(),
        ExprKind::TableRef { name } => name.to_ascii_uppercase(),
        _ => String::new(),
    };
    Ok(match word.as_str() {
        "INTEGER" => DtType::Integer,
        "DOUBLE" => DtType::Double,
        "STRING" => DtType::String,
        "BOOLEAN" => DtType::Boolean,
        "CURRENCY" => DtType::Currency,
        "DATETIME" => DtType::DateTime,
        _ => {
            return Err(DaxError::argument(
                "DATATABLE",
                "column types must be INTEGER, DOUBLE, STRING, BOOLEAN, CURRENCY or DATETIME",
            ));
        }
    })
}

fn datatable_value(t: DtType, v: &Value) -> Result<Value> {
    if v.is_blank() {
        return Ok(Value::Blank);
    }
    Ok(match t {
        DtType::Integer => ops::to_i64(v)?.map_or(Value::Blank, Value::Integer),
        DtType::Double => ops::to_f64(v)?.map_or(Value::Blank, Value::Double),
        DtType::String => Value::string(ops::to_text(v)),
        DtType::Boolean => ops::truth(v)?.map_or(Value::Blank, Value::Boolean),
        DtType::Currency => match ops::to_f64(v)?.and_then(Value::decimal_from_f64) {
            Some(d) => d,
            None => Value::Blank,
        },
        DtType::DateTime => ops::to_datetime(v)?.map_or(Value::Blank, Value::DateTime),
    })
}

fn arrow_of(t: DtType) -> ArrowType {
    match t {
        DtType::Integer => ArrowType::Int64,
        DtType::Double => ArrowType::Float64,
        DtType::String => ArrowType::Utf8,
        DtType::Boolean => ArrowType::Boolean,
        DtType::Currency => ArrowType::Decimal128(DECIMAL_PRECISION, DECIMAL_SCALE),
        DtType::DateTime => ArrowType::Timestamp(arrow::datatypes::TimeUnit::Microsecond, None),
    }
}

fn datatable(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("DATATABLE", args, 3, usize::MAX)?;
    let Some((data, defs)) = args.split_last() else {
        return Err(DaxError::argument("DATATABLE", "missing data"));
    };
    if defs.len() % 2 != 0 {
        return Err(DaxError::argument(
            "DATATABLE",
            "expects column name/type pairs followed by the data",
        ));
    }
    let mut columns = Vec::new();
    let mut types = Vec::new();
    let (defs, _) = defs.as_chunks::<2>();
    for [name, ty] in defs {
        let t = datatable_type(ty)?;
        let mut col = VColumn::new(name_arg("DATATABLE", name)?);
        col.data_type = Some(arrow_of(t));
        columns.push(col);
        types.push(t);
    }
    let ExprKind::TableConstructor { rows } = &data.unparen().kind else {
        return Err(DaxError::argument(
            "DATATABLE",
            "the data must be a table constructor such as {{\"a\", 1}, {\"b\", 2}}",
        ));
    };
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let cells: Vec<&Expr> = match row.as_slice() {
            [single] => match &single.unparen().kind {
                ExprKind::TableConstructor { rows: inner } => inner.iter().flatten().collect(),
                _ => vec![single],
            },
            many => many.iter().collect(),
        };
        if cells.len() != columns.len() {
            return Err(DaxError::argument(
                "DATATABLE",
                format!(
                    "each row must have {} values, found {}",
                    columns.len(),
                    cells.len()
                ),
            ));
        }
        let mut values = Vec::with_capacity(cells.len());
        for (cell, t) in cells.into_iter().zip(&types) {
            values.push(datatable_value(*t, &i.scalar(env, cell)?)?);
        }
        out.push(values);
    }
    tbl(VirtualTable::materialized(columns, out))
}

fn relatedtable(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("RELATEDTABLE", args, 1, 1)?;
    let a = arg("RELATEDTABLE", args, 0)?;
    let Some(name) = table_name(a) else {
        return Err(DaxError::argument(
            "RELATEDTABLE",
            "expects a table reference",
        ));
    };
    let filter = i.transition(env)?;
    Ok(Val::Table(i.table_ref(&env.with_filter(filter), name)?))
}

fn calculatetable(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("CALCULATETABLE", args, 1, usize::MAX)?;
    let filter = i.calculate_context(env, args.get(1..).unwrap_or_default(), "CALCULATETABLE")?;
    Ok(Val::Table(i.table(
        &env.with_filter(filter),
        arg("CALCULATETABLE", args, 0)?,
    )?))
}

fn crossjoin(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("CROSSJOIN", args, 2, usize::MAX)?;
    let mut columns: Vec<VColumn> = Vec::new();
    let mut rows: Vec<Vec<Value>> = vec![Vec::new()];
    for a in args {
        let t = i.table(env, a)?;
        if rows.len().saturating_mul(t.row_count()) > MAX_GENERATED_ROWS {
            return Err(DaxError::evaluation("CROSSJOIN: the result is too large"));
        }
        columns.extend(t.columns.iter().cloned());
        let right = t.rows();
        let mut next = Vec::with_capacity(rows.len() * right.len());
        for l in &rows {
            for r in &right {
                let mut row = l.clone();
                row.extend(r.iter().cloned());
                next.push(row);
            }
        }
        rows = next;
    }
    tbl(VirtualTable::materialized(columns, rows))
}

fn treatas(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("TREATAS", args, 2, usize::MAX)?;
    let table = i.table(env, arg("TREATAS", args, 0)?)?;
    let cols = args.get(1..).unwrap_or_default();
    if cols.len() != table.column_count() {
        return Err(DaxError::argument(
            "TREATAS",
            "the number of columns must match the table",
        ));
    }
    let columns = cols
        .iter()
        .map(|c| {
            let (t, c) = i.column_arg(env, c, "TREATAS")?;
            let dt = i
                .snapshot
                .table(&t)
                .ok()
                .and_then(|d| d.column_type(&c).ok().cloned());
            Ok(VColumn::model(&t, &c, dt))
        })
        .collect::<Result<Vec<_>>>()?;
    tbl(table.relabeled(columns))
}

fn keepfilters(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("KEEPFILTERS", args, 1, 1)?;
    Ok(Val::Table(i.table(env, arg("KEEPFILTERS", args, 0)?)?))
}

fn generateseries(i: &Interp<'_>, env: &Env, args: &[Expr]) -> R {
    arity("GENERATESERIES", args, 2, 3)?;
    let start = ops::to_numeric(&i.scalar(env, arg("GENERATESERIES", args, 0)?)?)?;
    let end = ops::to_numeric(&i.scalar(env, arg("GENERATESERIES", args, 1)?)?)?;
    let step = match opt_arg(args, 2) {
        Some(e) => ops::to_numeric(&i.scalar(env, e)?)?,
        None => Value::Integer(1),
    };
    let (Some(s), Some(e), Some(st)) = (
        ops::to_f64(&start)?,
        ops::to_f64(&end)?,
        ops::to_f64(&step)?,
    ) else {
        return Err(DaxError::argument(
            "GENERATESERIES",
            "arguments cannot be blank",
        ));
    };
    if st <= 0.0 || !st.is_finite() {
        return Err(DaxError::argument(
            "GENERATESERIES",
            "the increment must be positive",
        ));
    }
    let count = ((e - s) / st).floor() + 1.0;
    if count > MAX_GENERATED_ROWS as f64 {
        return Err(DaxError::evaluation(
            "GENERATESERIES: the result is too large",
        ));
    }
    let count = if count.is_finite() && count > 0.0 {
        count as usize
    } else {
        0
    };
    let ints = matches!((&start, &step), (Value::Integer(_), Value::Integer(_)));
    let mut rows = Vec::with_capacity(count);
    let mut current = start.clone();
    for _ in 0..count {
        rows.push(vec![current.clone()]);
        current = if ints {
            ops::arith(crate::syntax::BinaryOp::Add, &current, &step)?
        } else {
            Value::Double(ops::to_f64(&current)?.unwrap_or(0.0) + st)
        };
    }
    tbl(VirtualTable::materialized(
        vec![VColumn::new("Value")],
        rows,
    ))
}
