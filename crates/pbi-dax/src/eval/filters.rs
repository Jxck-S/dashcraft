//! `CALCULATE` / `CALCULATETABLE` filter arguments and conversion of table
//! values into filters.
//!
//! Steps (as in DAX):
//! 1. every filter argument is evaluated in the *original* context (filter
//!    and row context);
//! 2. context transition turns the row context into filters;
//! 3. modifiers (`ALL`, `REMOVEFILTERS`, `ALLEXCEPT`, `ALLNOBLANKROW`)
//!    remove filters;
//! 4. the new filters replace existing filters on the same columns
//!    (`KEEPFILTERS` intersects instead) and are intersected with each other.
//!
//! Boolean predicates (`T[c] = "x"`) become `FILTER(ALL(T[c]), pred)`: the
//! predicate is evaluated once per distinct value (BLANK included when the
//! table has a blank row) and the accepted values form an `IN` filter.
//! Predicates on several columns of one table become tuple filters.
//!
//! Other modifiers:
//! - `ALLSELECTED()`, `ALLSELECTED(T)`, `ALLSELECTED(T[c], …)` remove the
//!   filters on everything / `T`'s expanded table / those columns, then
//!   restore the filters of the *shadow* context ([`Env::shadow`]: the
//!   query's or visual's outer filters) on the same scope;
//! - `USERELATIONSHIP(a, b)` and `CROSSFILTER(a, b, dir)` set relationship
//!   overrides on the new context
//!   ([`pbi_engine::FilterContext::set_relationship_override`]).
//!
//! **Date tables.** A (non-`KEEPFILTERS`) filter on a date/time column
//! that is the one-side key of a relationship, or a date/time column of a
//! table marked as a date table (`dataCategory: Time`), also removes every
//! other filter on that table, as DAX does: `CALCULATE([Sales],
//! DATESYTD('Date'[Date]))` in a month row keeps the whole year to date.

use std::collections::HashSet;
use std::sync::Arc;

use pbi_engine::{
    ColumnFilter, CrossFilterDirection, Filter, FilterContext, RelationshipGraph,
    RelationshipOverride, RowSet, TableFilter, ValueKey,
};
use pbi_model::names::names_eq;

use super::context::{Env, RowFrame};
use super::error::{DaxError, Result};
use super::functions;
use super::interp::Interp;
use super::table::{RowPos, VirtualTable};
use crate::syntax::{Expr, ExprKind, for_each_expr};

/// A filter-removing `CALCULATE` modifier.
#[derive(Debug, Clone)]
pub(crate) enum Modifier {
    /// `ALL()` / `REMOVEFILTERS()`.
    All,
    /// `ALL(T)`: the expanded table.
    Table(String),
    /// `ALL(T[c], …)`.
    Columns(Vec<(String, String)>),
    /// `ALLEXCEPT(T, T[c], …)`.
    AllExcept {
        table: String,
        keep: Vec<(String, String)>,
    },
    /// `ALLSELECTED(…)`: restore the shadow filters on `target`.
    AllSelected {
        target: Selected,
        shadow: Arc<FilterContext>,
    },
    /// `USERELATIONSHIP` / `CROSSFILTER`.
    Relationship(RelationshipOverride),
}

/// What `ALLSELECTED` applies to.
#[derive(Debug, Clone)]
pub(crate) enum Selected {
    /// `ALLSELECTED()`.
    All,
    /// `ALLSELECTED(T)`: `T`'s expanded table.
    Table(String),
    /// `ALLSELECTED(T[c], …)`.
    Columns(Vec<(String, String)>),
}

/// Applies `ALLSELECTED(target)` to `f`: the filters on `target` are
/// replaced by those of `shadow` on the same scope.
pub(crate) fn apply_allselected(
    f: &mut FilterContext,
    target: &Selected,
    shadow: &FilterContext,
    graph: &RelationshipGraph,
) {
    match target {
        Selected::All => {
            f.clear();
            for flt in shadow.filters() {
                f.add(flt.clone());
            }
        }
        Selected::Table(t) => {
            let mut tables = vec![t.clone()];
            tables.extend(graph.expanded_tables(t).iter().map(|(x, _)| x.clone()));
            f.remove_expanded_table(graph, t);
            for flt in shadow.filters() {
                if tables.iter().any(|x| names_eq(x, flt.table())) {
                    f.add(flt.clone());
                }
            }
        }
        Selected::Columns(cols) => {
            for (t, c) in cols {
                f.remove_column(t, c);
            }
            for flt in shadow.filters() {
                if let Filter::Column(cf) = flt
                    && cols.iter().any(|(t, c)| cf.is_on(t, c))
                {
                    f.add(flt.clone());
                }
            }
        }
    }
}

/// A filter produced by a filter argument.
#[derive(Debug, Clone)]
pub(crate) enum FilterSpec {
    Column(ColumnFilter),
    /// Expanded-table filter (rows of a stored table).
    Table(TableFilter),
    /// Multi-column filter on one table (base-only table filter) and the
    /// columns it overrides.
    Tuple {
        filter: TableFilter,
        columns: Vec<(String, String)>,
    },
}

/// Evaluated filter arguments.
#[derive(Debug, Default)]
pub(crate) struct FilterArgs {
    pub modifiers: Vec<Modifier>,
    /// `(filter, keep)`: `keep` = `KEEPFILTERS` (intersect).
    pub filters: Vec<(FilterSpec, bool)>,
}

impl Interp<'_> {
    /// The filter context of `CALCULATE(…, args)` evaluated in `env`.
    pub fn calculate_context(
        &self,
        env: &Env,
        args: &[Expr],
        function: &str,
    ) -> Result<Arc<FilterContext>> {
        let fa = self.collect_filter_args(env, args, function)?;
        let base = self.transition(env)?;
        Ok(Arc::new(self.apply_filter_args(&base, fa)))
    }

    /// Evaluates filter arguments in `env`.
    pub fn collect_filter_args(
        &self,
        env: &Env,
        args: &[Expr],
        function: &str,
    ) -> Result<FilterArgs> {
        let mut out = FilterArgs::default();
        for arg in args {
            let a = arg.unparen();
            if matches!(a.kind, ExprKind::Omitted) {
                continue;
            }
            if let Some((name, fargs)) = a.as_call() {
                match name {
                    "ALL" | "REMOVEFILTERS" | "ALLNOBLANKROW" => {
                        out.modifiers.extend(self.modifiers(env, name, fargs)?);
                        continue;
                    }
                    "ALLEXCEPT" => {
                        out.modifiers.push(self.all_except_modifier(env, fargs)?);
                        continue;
                    }
                    "KEEPFILTERS" => {
                        let [inner] = fargs else {
                            return Err(DaxError::argument(
                                "KEEPFILTERS",
                                "expects exactly one argument",
                            ));
                        };
                        for spec in self.filter_specs(env, inner, function)? {
                            out.filters.push((spec, true));
                        }
                        continue;
                    }
                    "ALLSELECTED" => {
                        out.modifiers.push(Modifier::AllSelected {
                            target: self.selected_target(env, fargs)?,
                            shadow: Arc::clone(&env.shadow),
                        });
                        continue;
                    }
                    "USERELATIONSHIP" | "CROSSFILTER" => {
                        out.modifiers.push(Modifier::Relationship(
                            self.relationship_override(env, name, fargs)?,
                        ));
                        continue;
                    }
                    _ => {}
                }
            }
            for spec in self.filter_specs(env, a, function)? {
                out.filters.push((spec, false));
            }
        }
        Ok(out)
    }

    /// Applies evaluated filter arguments to `base`.
    pub fn apply_filter_args(&self, base: &FilterContext, fa: FilterArgs) -> FilterContext {
        let mut f = base.clone();
        for m in &fa.modifiers {
            if let Modifier::Relationship(o) = m {
                f.set_relationship_override(*o);
            }
        }
        let graph = self.snapshot.graph_for(&f);
        let graph = graph.as_ref();
        for m in fa.modifiers {
            match m {
                Modifier::All => f.clear(),
                Modifier::Table(t) => f.remove_expanded_table(graph, &t),
                Modifier::Columns(cols) => {
                    for (t, c) in cols {
                        f.remove_column(&t, &c);
                    }
                }
                Modifier::AllExcept { table, keep } => f = self.all_except(&f, &table, &keep),
                Modifier::AllSelected { target, shadow } => {
                    apply_allselected(&mut f, &target, &shadow, graph);
                }
                Modifier::Relationship(_) => {}
            }
        }
        for (spec, keep) in &fa.filters {
            if *keep {
                continue;
            }
            match spec {
                FilterSpec::Column(c) => {
                    if self.is_date_key(&c.table, &c.column) {
                        f.remove_table(&c.table);
                    } else {
                        f.remove_column(&c.table, &c.column);
                    }
                }
                FilterSpec::Table(t) => f.remove_expanded_table(graph, &t.table),
                FilterSpec::Tuple { columns, .. } => {
                    for (t, c) in columns {
                        f.remove_column(t, c);
                    }
                }
            }
        }
        for (spec, _) in fa.filters {
            match spec {
                FilterSpec::Column(c) => f.add(c),
                FilterSpec::Table(t) => f.add(t),
                FilterSpec::Tuple { filter, .. } => f.add(filter),
            }
        }
        f
    }

    fn all_except(
        &self,
        f: &FilterContext,
        table: &str,
        keep: &[(String, String)],
    ) -> FilterContext {
        let mut expanded = vec![table.to_owned()];
        expanded.extend(
            self.snapshot
                .graph()
                .expanded_tables(table)
                .iter()
                .map(|(t, _)| t.clone()),
        );
        let mut out = f.clone();
        out.clear();
        for flt in f.filters() {
            let on_expanded = expanded.iter().any(|t| names_eq(t, flt.table()));
            let kept =
                matches!(flt, Filter::Column(c) if keep.iter().any(|(t, col)| c.is_on(t, col)));
            if !on_expanded || kept {
                out.add(flt.clone());
            }
        }
        out
    }

    /// `true` for a date/time column that keys a date table: the one side
    /// of a relationship, or any date/time column of a table marked as a
    /// date table (`dataCategory: Time`).
    pub fn is_date_key(&self, table: &str, column: &str) -> bool {
        let is_datetime = self
            .snapshot
            .table(table)
            .ok()
            .and_then(|d| d.column_type(column).ok().cloned())
            .is_some_and(|t| matches!(t, arrow::datatypes::DataType::Timestamp(_, _)));
        if !is_datetime {
            return false;
        }
        let marked = self
            .model
            .table(table)
            .and_then(|t| t.data_category.as_deref())
            .is_some_and(|c| c.eq_ignore_ascii_case("Time"));
        marked
            || self
                .snapshot
                .graph()
                .definitions()
                .iter()
                .any(|d| d.one.matches(table, column))
    }

    /// The scope of an `ALLSELECTED(…)` call.
    pub fn selected_target(&self, env: &Env, args: &[Expr]) -> Result<Selected> {
        let Some(first) = args.first() else {
            return Ok(Selected::All);
        };
        match &first.unparen().kind {
            ExprKind::TableRef { name } | ExprKind::Keyword(name) => {
                if args.len() > 1 {
                    return Err(DaxError::argument(
                        "ALLSELECTED",
                        "expects a single table reference",
                    ));
                }
                let data = self
                    .snapshot
                    .table(name)
                    .map_err(|_| DaxError::UnknownTable { name: name.clone() })?;
                Ok(Selected::Table(data.name().to_owned()))
            }
            _ => Ok(Selected::Columns(
                args.iter()
                    .map(|a| self.column_arg(env, a, "ALLSELECTED"))
                    .collect::<Result<Vec<_>>>()?,
            )),
        }
    }

    /// `USERELATIONSHIP(c1, c2)` / `CROSSFILTER(c1, c2, direction)`.
    fn relationship_override(
        &self,
        env: &Env,
        name: &str,
        args: &[Expr],
    ) -> Result<RelationshipOverride> {
        let expected = if name == "CROSSFILTER" { 3 } else { 2 };
        super::functions::arity(name, args, expected, expected)?;
        let (t1, c1) = self.column_arg(env, super::functions::arg(name, args, 0)?, name)?;
        let (t2, c2) = self.column_arg(env, super::functions::arg(name, args, 1)?, name)?;
        let graph = self.snapshot.graph();
        let index = graph.find_relationship(&t1, &c1, &t2, &c2).ok_or_else(|| {
            DaxError::argument(
                name,
                format!(
                    "there is no relationship between {} and {}",
                    pbi_model::dax_column_ref(&t1, &c1),
                    pbi_model::dax_column_ref(&t2, &c2)
                ),
            )
        })?;
        if name == "USERELATIONSHIP" {
            return Ok(RelationshipOverride::Activate(index));
        }
        let left_is_many = graph
            .definitions()
            .get(index)
            .is_some_and(|d| d.many.matches(&t1, &c1));
        let dir = super::functions::arg(name, args, 2)?;
        let word = match &dir.unparen().kind {
            ExprKind::Keyword(k) | ExprKind::TableRef { name: k } => k.to_ascii_uppercase(),
            ExprKind::Number(_) => match super::ops::to_i64(&self.scalar(env, dir)?)? {
                Some(0) => "NONE".to_owned(),
                Some(1) => "ONEWAY".to_owned(),
                Some(2) => "BOTH".to_owned(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        let direction = match word.as_str() {
            "NONE" => CrossFilterDirection::None,
            "ONEWAY" => CrossFilterDirection::OneWay,
            "BOTH" => CrossFilterDirection::Both,
            // The named side filters the other one.
            "ONEWAY_LEFTFILTERSRIGHT" if left_is_many => CrossFilterDirection::ManyToOne,
            "ONEWAY_RIGHTFILTERSLEFT" if !left_is_many => CrossFilterDirection::ManyToOne,
            "ONEWAY_LEFTFILTERSRIGHT" | "ONEWAY_RIGHTFILTERSLEFT" => CrossFilterDirection::OneWay,
            _ => {
                return Err(DaxError::argument(
                    "CROSSFILTER",
                    "the direction must be None, OneWay, Both, OneWay_LeftFiltersRight or OneWay_RightFiltersLeft",
                ));
            }
        };
        Ok(RelationshipOverride::CrossFilter(index, direction))
    }

    fn modifiers(&self, env: &Env, name: &str, args: &[Expr]) -> Result<Vec<Modifier>> {
        if args.is_empty() {
            return Ok(vec![Modifier::All]);
        }
        let mut out = Vec::new();
        let mut cols = Vec::new();
        for a in args {
            match &a.unparen().kind {
                ExprKind::TableRef { name: t } | ExprKind::Keyword(t) => {
                    if let Ok(data) = self.snapshot.table(t) {
                        out.push(Modifier::Table(data.name().to_owned()));
                    } else if !self.table_exists(t) {
                        return Err(DaxError::UnknownTable { name: t.clone() });
                    }
                }
                ExprKind::ColumnRef { table, column } => {
                    cols.push(self.resolve_column(env, table.as_deref(), column)?);
                }
                ExprKind::VarRef(_) => {}
                _ => {
                    return Err(DaxError::argument(
                        name,
                        "expects table or column references when used as a CALCULATE filter",
                    ));
                }
            }
        }
        if !cols.is_empty() {
            out.push(Modifier::Columns(cols));
        }
        Ok(out)
    }

    fn all_except_modifier(&self, env: &Env, args: &[Expr]) -> Result<Modifier> {
        let Some((first, rest)) = args.split_first() else {
            return Err(DaxError::argument(
                "ALLEXCEPT",
                "expects a table and at least one column",
            ));
        };
        let table = match &first.unparen().kind {
            ExprKind::TableRef { name } | ExprKind::Keyword(name) => self
                .snapshot
                .table(name)
                .map_err(|_| DaxError::UnknownTable { name: name.clone() })?
                .name()
                .to_owned(),
            _ => {
                return Err(DaxError::argument(
                    "ALLEXCEPT",
                    "the first argument must be a table reference",
                ));
            }
        };
        let keep = rest
            .iter()
            .map(|c| self.column_arg(env, c, "ALLEXCEPT"))
            .collect::<Result<Vec<_>>>()?;
        Ok(Modifier::AllExcept { table, keep })
    }

    /// Filters for one (non-modifier) filter argument.
    pub fn filter_specs(&self, env: &Env, e: &Expr, function: &str) -> Result<Vec<FilterSpec>> {
        if self.is_table_expr(env, e) {
            let t = self.table(env, e)?;
            return self.table_filters(&t);
        }
        Ok(vec![self.predicate_filter(env, e, function)?])
    }

    /// Static check: does `e` produce a table?
    pub fn is_table_expr(&self, env: &Env, e: &Expr) -> bool {
        match &e.unparen().kind {
            ExprKind::TableRef { .. } | ExprKind::TableConstructor { .. } => true,
            ExprKind::Keyword(k) => self.table_exists(k),
            ExprKind::VarRef(n) => match env.vars.get(n) {
                Some(super::context::VarValue::Table(_)) => true,
                Some(super::context::VarValue::Scalar(_)) => false,
                None => self.table_exists(n),
            },
            ExprKind::Var { bindings, body } => match &body.unparen().kind {
                ExprKind::VarRef(n) => bindings
                    .iter()
                    .rev()
                    .find(|b| names_eq(&b.name, n))
                    .map_or_else(
                        || self.is_table_expr(env, body),
                        |b| self.is_table_expr(env, &b.expr),
                    ),
                _ => self.is_table_expr(env, body),
            },
            ExprKind::Call { name, .. } => functions::returns_table(name),
            _ => false,
        }
    }

    /// Filters equivalent to a table value: a stored table's rows filter
    /// its expanded table; otherwise every column with lineage filters its
    /// model column (several columns of one table → a tuple filter).
    pub fn table_filters(&self, t: &VirtualTable) -> Result<Vec<FilterSpec>> {
        if let (Some(data), Some((rows, blank))) = (&t.base, t.base_rowset()) {
            return Ok(vec![FilterSpec::Table(
                TableFilter::new(data.name(), rows).with_blank_row(blank),
            )]);
        }
        let mut groups: Vec<(String, Vec<(usize, String)>)> = Vec::new();
        for (i, col) in t.columns.iter().enumerate() {
            let Some(lin) = &col.lineage else { continue };
            if self.snapshot.table(&lin.table).is_err() {
                continue;
            }
            match groups.iter_mut().find(|(tb, _)| names_eq(tb, &lin.table)) {
                Some((_, cols)) => {
                    if !cols.iter().any(|(_, c)| names_eq(c, &lin.column)) {
                        cols.push((i, lin.column.clone()));
                    }
                }
                None => groups.push((lin.table.clone(), vec![(i, lin.column.clone())])),
            }
        }
        let positions = t.positions();
        let mut out = Vec::new();
        for (table, cols) in groups {
            if let [(i, col)] = cols.as_slice() {
                let keys: HashSet<ValueKey> = positions
                    .iter()
                    .map(|p| ValueKey::from(t.value(*p, *i)))
                    .collect();
                out.push(FilterSpec::Column(ColumnFilter::in_keys(table, col, keys)));
            } else {
                let tuples: HashSet<Vec<ValueKey>> = positions
                    .iter()
                    .map(|p| {
                        cols.iter()
                            .map(|(i, _)| ValueKey::from(t.value(*p, *i)))
                            .collect()
                    })
                    .collect();
                let names: Vec<String> = cols.into_iter().map(|(_, c)| c).collect();
                out.push(self.tuple_filter(&table, &names, &tuples)?);
            }
        }
        Ok(out)
    }

    /// Base-only table filter keeping the rows of `table` whose values in
    /// `cols` form one of `tuples`.
    fn tuple_filter(
        &self,
        table: &str,
        cols: &[String],
        tuples: &HashSet<Vec<ValueKey>>,
    ) -> Result<FilterSpec> {
        let data = self.snapshot.table(table)?;
        let encs = cols
            .iter()
            .map(|c| data.encoded(c))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let n = data.row_count();
        let mut rows = RowSet::empty(n);
        for r in 0..n {
            let key: Vec<ValueKey> = encs
                .iter()
                .map(|e| e.value_at(r).map_or(ValueKey::Blank, ValueKey::from))
                .collect();
            if tuples.contains(&key) {
                rows.insert(r);
            }
        }
        let blank = tuples.contains(&vec![ValueKey::Blank; cols.len()]);
        Ok(FilterSpec::Tuple {
            filter: TableFilter::new(data.name(), rows)
                .base_only()
                .with_blank_row(blank),
            columns: cols
                .iter()
                .map(|c| (data.name().to_owned(), c.clone()))
                .collect(),
        })
    }

    /// The first function called in `e` that is neither built in nor
    /// user-defined.
    fn unsupported_call(&self, e: &Expr) -> Option<String> {
        let mut found = None;
        for_each_expr(e, |node| {
            if found.is_none()
                && let ExprKind::Call { name, .. } = &node.kind
                && !functions::is_supported_function(name)
                && !self
                    .functions
                    .contains_key(&pbi_model::names::name_key(name))
            {
                found = Some(name.clone());
            }
        });
        found
    }

    /// `FILTER(ALL(columns of pred), pred)` as a filter.
    fn predicate_filter(&self, env: &Env, pred: &Expr, function: &str) -> Result<FilterSpec> {
        if let Some(name) = self.unsupported_call(pred) {
            return Err(DaxError::UnsupportedFunction { name });
        }
        let cols = self.predicate_columns(env, pred);
        let Some((table, _)) = cols.first().cloned() else {
            return Err(DaxError::argument(
                function,
                format!(
                    "the filter expression {} does not reference any column",
                    super::interp::snippet(pred)
                ),
            ));
        };
        if cols.iter().any(|(t, _)| !names_eq(t, &table)) {
            return Err(DaxError::argument(
                function,
                format!(
                    "the filter expression {} references columns of more than one table; use FILTER instead",
                    super::interp::snippet(pred)
                ),
            ));
        }
        let names: Vec<String> = cols.into_iter().map(|(_, c)| c).collect();
        let all = self.all_columns(&table, &names, true)?;
        let mut keep: Vec<RowPos> = Vec::new();
        for pos in all.positions() {
            self.check_cancel()?;
            let row_env = env.with_row(RowFrame::new(Arc::clone(&all), pos));
            if self.condition(&row_env, pred)? {
                keep.push(pos);
            }
        }
        if let [col] = names.as_slice() {
            let values = keep.iter().map(|p| all.value(*p, 0));
            return Ok(FilterSpec::Column(ColumnFilter::in_values(
                table.as_str(),
                col.as_str(),
                values,
            )));
        }
        let tuples: HashSet<Vec<ValueKey>> = keep.iter().map(|p| all.row_key(*p)).collect();
        self.tuple_filter(&table, &names, &tuples)
    }

    /// Stored columns referenced by a predicate. Columns that resolve in
    /// the current row context are treated as constants (`T[k] =
    /// Other[k]` inside an iterator over `Other`) unless nothing else is
    /// referenced.
    fn predicate_columns(&self, env: &Env, pred: &Expr) -> Vec<(String, String)> {
        let mut refs: Vec<(Option<String>, String)> = Vec::new();
        for_each_expr(pred, |e| {
            if let ExprKind::ColumnRef { table, column } = &e.kind {
                refs.push((table.clone(), column.clone()));
            }
        });
        let mut free: Vec<(String, String)> = Vec::new();
        let mut bound: Vec<(String, String)> = Vec::new();
        for (table, column) in refs {
            let is_measure = self
                .find_measure(table.as_deref(), &column)
                .ok()
                .flatten()
                .is_some();
            let is_column = match table.as_deref() {
                Some(t) => self.snapshot.table(t).is_ok_and(|d| d.has_column(&column)),
                None => self.resolve_column(env, None, &column).is_ok(),
            };
            if is_measure && !is_column {
                continue;
            }
            let Ok(resolved) = self.resolve_column(env, table.as_deref(), &column) else {
                continue;
            };
            let target = if self.frame_value(env, table.as_deref(), &column).is_some() {
                &mut bound
            } else {
                &mut free
            };
            if !target
                .iter()
                .any(|(t, c)| names_eq(t, &resolved.0) && names_eq(c, &resolved.1))
            {
                target.push(resolved);
            }
        }
        if free.is_empty() { bound } else { free }
    }
}
