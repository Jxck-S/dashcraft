//! The formula-engine core: expression evaluation, reference resolution,
//! measures and context transition.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;

use pbi_engine::{EngineSnapshot, FilterContext, RowSet, TableFilter, Value};
use pbi_model::names::{name_key, names_eq};
use pbi_model::{ColumnRef, Model};

use super::calcgroup::{self, CalcGroupInfo};
use super::context::{Cancel, Env, RowFrame, VarScope, VarValue};
use super::error::{DaxError, Result};
use super::functions;
use super::ops;
use super::table::{BLANK_ROW, RowPos, VColumn, VirtualTable};
use crate::syntax::{
    BinaryOp, Expr, ExprKind, FunctionParam, Number, UnaryOp, VarBinding, parse_expression,
    print_expr,
};

/// Result of evaluating an expression: a scalar or a table.
#[derive(Debug, Clone)]
pub(crate) enum Val {
    Scalar(Value),
    Table(Arc<VirtualTable>),
}

/// A measure defined in a query (`DEFINE MEASURE`).
#[derive(Debug, Clone)]
pub(crate) struct QueryMeasure {
    pub table: String,
    pub name: String,
    pub expr: Arc<Expr>,
}

/// A user-defined function (`DEFINE FUNCTION`).
#[derive(Debug, Clone)]
pub(crate) struct Udf {
    pub params: Vec<FunctionParam>,
    pub body: Arc<Expr>,
}

/// A resolved measure reference.
#[derive(Debug, Clone)]
pub(crate) struct MeasureRef {
    pub table: String,
    pub name: String,
    pub expr: Arc<Expr>,
    pub query: bool,
}

/// Interpreter state shared by one evaluation (not `Send`; lives on the
/// evaluation thread).
pub(crate) struct Interp<'a> {
    pub snapshot: &'a EngineSnapshot,
    pub model: &'a Model,
    cancel: Option<Cancel>,
    depth: Cell<usize>,
    measures: RefCell<HashMap<String, Arc<Expr>>>,
    stack: RefCell<Vec<String>>,
    all_cache: RefCell<HashMap<String, Arc<VirtualTable>>>,
    pub query_measures: HashMap<String, QueryMeasure>,
    pub query_tables: HashMap<String, Arc<VirtualTable>>,
    pub query_vars: VarScope,
    pub functions: HashMap<String, Udf>,
    /// Use the one-pass evaluations of [`super::fast`] where they apply
    /// (`false` only to test them against the general path).
    pub fast_paths: bool,
    /// Evaluations that took a fast path.
    pub fast_path_hits: Cell<usize>,
    /// The model's calculation groups, highest precedence first.
    pub(crate) calc_groups: Vec<CalcGroupInfo<'a>>,
    /// Parsed calculation item and format string expressions.
    pub(crate) calc_exprs: RefCell<HashMap<String, Arc<Expr>>>,
    /// The model's `selectionExpressionBehavior` is `visual`.
    pub(crate) visual_selection: bool,
    /// State of the `RAND`/`RANDBETWEEN` generator (seeded per evaluation).
    pub(crate) rand_state: Cell<u64>,
}

/// Decrements the evaluation depth when dropped.
pub(crate) struct DepthGuard<'i> {
    depth: &'i Cell<usize>,
}

impl Drop for DepthGuard<'_> {
    fn drop(&mut self) {
        self.depth.set(self.depth.get().saturating_sub(1));
    }
}

/// Parses an expression, converting the error.
pub(crate) fn parse(src: &str) -> Result<Expr> {
    parse_expression(src).map_err(|e| DaxError::from_parse(&e, src))
}

/// Short source text of an expression for error messages.
pub(crate) fn snippet(e: &Expr) -> String {
    let text = print_expr(e);
    if text.chars().count() > 80 {
        let cut: String = text.chars().take(77).collect();
        format!("{cut}...")
    } else {
        text
    }
}

impl<'a> Interp<'a> {
    pub fn new(
        snapshot: &'a EngineSnapshot,
        model: &'a Model,
        cancel: Option<Cancel>,
        depth: usize,
    ) -> Self {
        Self {
            snapshot,
            model,
            cancel,
            depth: Cell::new(depth),
            measures: RefCell::new(HashMap::new()),
            stack: RefCell::new(Vec::new()),
            all_cache: RefCell::new(HashMap::new()),
            query_measures: HashMap::new(),
            query_tables: HashMap::new(),
            query_vars: VarScope::new(),
            functions: HashMap::new(),
            fast_paths: true,
            fast_path_hits: Cell::new(0),
            calc_groups: calcgroup::calc_groups(model),
            calc_exprs: RefCell::new(HashMap::new()),
            visual_selection: calcgroup::visual_selection_behavior(model),
            rand_state: Cell::new(functions::RAND_SEED),
        }
    }

    /// An interpreter for the options of a public entry point.
    pub fn with_options(
        snapshot: &'a EngineSnapshot,
        model: &'a Model,
        opts: &super::EvalOptions,
    ) -> Self {
        let mut interp = Self::new(snapshot, model, opts.cancel.clone(), opts.depth);
        interp.fast_paths = !opts.general_only;
        interp
    }

    /// Current evaluation depth.
    pub fn depth(&self) -> usize {
        self.depth.get()
    }

    /// `true` while measure `name` is being evaluated (a reference to it
    /// would be circular).
    pub fn measure_on_stack(&self, name: &str) -> bool {
        let key = name_key(name);
        self.stack.borrow().iter().any(|s| name_key(s) == key)
    }

    // ---- guards ----------------------------------------------------------

    pub fn enter(&self) -> Result<DepthGuard<'_>> {
        let d = self.depth.get() + 1;
        if d > super::error::MAX_EVAL_DEPTH {
            return Err(DaxError::DepthExceeded);
        }
        self.check_cancel()?;
        self.depth.set(d);
        Ok(DepthGuard { depth: &self.depth })
    }

    pub fn check_cancel(&self) -> Result<()> {
        match &self.cancel {
            Some(c) if c.is_cancelled() => Err(DaxError::Cancelled),
            _ => Ok(()),
        }
    }

    // ---- evaluation --------------------------------------------------------

    pub fn eval(&self, env: &Env, e: &Expr) -> Result<Val> {
        let _guard = self.enter()?;
        match &e.kind {
            ExprKind::Number(Number::Integer(i)) => Ok(Val::Scalar(Value::Integer(*i))),
            ExprKind::Number(Number::Decimal(d)) => Ok(Val::Scalar(Value::Double(*d))),
            ExprKind::String(s) => Ok(Val::Scalar(Value::string(s.as_str()))),
            ExprKind::DateTime(s) => ops::parse_datetime(s).map(Val::Scalar).ok_or_else(|| {
                DaxError::type_error(format!("Cannot convert dt\"{s}\" to a date/time."))
            }),
            ExprKind::Boolean(b) => Ok(Val::Scalar(Value::Boolean(*b))),
            ExprKind::TableRef { name } => self.table_ref(env, name).map(Val::Table),
            ExprKind::ColumnRef { table, column } => self
                .column_ref(env, table.as_deref(), column)
                .map(Val::Scalar),
            ExprKind::Call { name, args, .. } => self.call(env, name, args),
            ExprKind::Var { bindings, body } => self.var_block(env, bindings, body),
            ExprKind::VarRef(name) => self.var_ref(env, name),
            ExprKind::Parameter(name) => Err(DaxError::evaluation(format!(
                "The query parameter @{name} has no value."
            ))),
            ExprKind::Keyword(k) => self.keyword(env, k),
            ExprKind::Unary { op, expr } => self.unary(env, *op, expr).map(Val::Scalar),
            ExprKind::Binary { op, lhs, rhs } => self.binary(env, *op, lhs, rhs).map(Val::Scalar),
            ExprKind::TableConstructor { rows } => {
                self.table_constructor(env, rows).map(Val::Table)
            }
            ExprKind::Tuple(_) => Err(DaxError::type_error(
                "A list of values in parentheses can only be used with IN.",
            )),
            ExprKind::Paren(inner) => self.eval(env, inner),
            ExprKind::Omitted => Ok(Val::Scalar(Value::Blank)),
        }
    }

    /// Evaluates to a scalar; a one-column table with at most one row is
    /// converted (BLANK when empty).
    pub fn scalar(&self, env: &Env, e: &Expr) -> Result<Value> {
        match self.eval(env, e)? {
            Val::Scalar(v) => Ok(v),
            Val::Table(t) => table_to_scalar(&t, e),
        }
    }

    pub fn table(&self, env: &Env, e: &Expr) -> Result<Arc<VirtualTable>> {
        match self.eval(env, e)? {
            Val::Table(t) => Ok(t),
            Val::Scalar(_) => Err(DaxError::type_error(format!(
                "The expression {} returns a single value, but a table is required here.",
                snippet(e)
            ))),
        }
    }

    /// Evaluates a condition (BLANK → false).
    pub fn condition(&self, env: &Env, e: &Expr) -> Result<bool> {
        ops::is_true(&self.scalar(env, e)?)
    }

    fn var_block(&self, env: &Env, bindings: &[VarBinding], body: &Expr) -> Result<Val> {
        let mut env = env.clone();
        for b in bindings {
            let value = match self.eval(&env, &b.expr)? {
                Val::Scalar(v) => VarValue::Scalar(v),
                Val::Table(t) => VarValue::Table(t),
            };
            env = env.with_var(&b.name, value);
        }
        self.eval(&env, body)
    }

    fn var_ref(&self, env: &Env, name: &str) -> Result<Val> {
        match env.vars.get(name) {
            Some(VarValue::Scalar(v)) => Ok(Val::Scalar(v.clone())),
            Some(VarValue::Table(t)) => Ok(Val::Table(Arc::clone(t))),
            None => {
                if let Some(t) = self.query_tables.get(&name_key(name)) {
                    return Ok(Val::Table(Arc::clone(t)));
                }
                if self.snapshot.table(name).is_ok() {
                    return self.table_ref(env, name).map(Val::Table);
                }
                Err(DaxError::evaluation(format!(
                    "The variable '{name}' is not defined."
                )))
            }
        }
    }

    fn keyword(&self, env: &Env, k: &str) -> Result<Val> {
        if self.query_tables.contains_key(&name_key(k)) || self.snapshot.table(k).is_ok() {
            return self.table_ref(env, k).map(Val::Table);
        }
        Err(DaxError::evaluation(format!(
            "The keyword {k} is not valid here."
        )))
    }

    fn unary(&self, env: &Env, op: UnaryOp, e: &Expr) -> Result<Value> {
        let v = self.scalar(env, e)?;
        match op {
            UnaryOp::Plus => ops::to_numeric(&v),
            UnaryOp::Minus => ops::negate(&v),
            UnaryOp::Not => Ok(Value::Boolean(!ops::truth(&v)?.unwrap_or(false))),
        }
    }

    fn binary(&self, env: &Env, op: BinaryOp, lhs: &Expr, rhs: &Expr) -> Result<Value> {
        match op {
            BinaryOp::And => {
                let l = ops::truth(&self.scalar(env, lhs)?)?;
                if l == Some(false) {
                    return Ok(Value::Boolean(false));
                }
                let r = ops::truth(&self.scalar(env, rhs)?)?;
                Ok(ops::and(l, r))
            }
            BinaryOp::Or => {
                let l = ops::truth(&self.scalar(env, lhs)?)?;
                if l == Some(true) {
                    return Ok(Value::Boolean(true));
                }
                let r = ops::truth(&self.scalar(env, rhs)?)?;
                Ok(ops::or(l, r))
            }
            BinaryOp::In => self.in_op(env, lhs, rhs),
            BinaryOp::Concat => {
                let l = self.scalar(env, lhs)?;
                let r = self.scalar(env, rhs)?;
                Ok(ops::concat(&l, &r))
            }
            op if op.is_comparison() => {
                let l = self.scalar(env, lhs)?;
                let r = self.scalar(env, rhs)?;
                ops::compare(op, &l, &r)
            }
            op => {
                let l = self.scalar(env, lhs)?;
                let r = self.scalar(env, rhs)?;
                ops::arith(op, &l, &r)
            }
        }
    }

    /// `value IN table` / `(a, b) IN table` (strict equality, like `==`).
    fn in_op(&self, env: &Env, lhs: &Expr, rhs: &Expr) -> Result<Value> {
        let values = match &lhs.unparen().kind {
            ExprKind::Tuple(items) => items
                .iter()
                .map(|i| self.scalar(env, i))
                .collect::<Result<Vec<_>>>()?,
            _ => vec![self.scalar(env, lhs)?],
        };
        let table = self.table(env, rhs)?;
        if table.column_count() != values.len() {
            return Err(DaxError::type_error(format!(
                "IN: the table has {} column(s) but {} value(s) were given.",
                table.column_count(),
                values.len()
            )));
        }
        let wanted: Vec<_> = values.iter().map(ops::key).collect();
        Ok(Value::Boolean(
            table
                .positions()
                .into_iter()
                .any(|p| table.row_key(p) == wanted),
        ))
    }

    fn table_constructor(&self, env: &Env, rows: &[Vec<Expr>]) -> Result<Arc<VirtualTable>> {
        let width = rows.first().map_or(1, Vec::len);
        if rows.iter().any(|r| r.len() != width) {
            return Err(DaxError::type_error(
                "All rows of a table constructor must have the same number of values.",
            ));
        }
        let columns = constructor_columns(width);
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            self.check_cancel()?;
            out.push(
                row.iter()
                    .map(|e| self.scalar(env, e))
                    .collect::<Result<Vec<_>>>()?,
            );
        }
        Ok(Arc::new(VirtualTable::materialized(columns, out)))
    }

    fn call(&self, env: &Env, name: &str, args: &[Expr]) -> Result<Val> {
        if let Some(udf) = self.functions.get(&name_key(name)) {
            return self.call_udf(env, udf, args);
        }
        match functions::lookup(name) {
            Some(builtin) => (builtin.f)(self, env, args),
            None => Err(DaxError::UnsupportedFunction {
                name: name.to_owned(),
            }),
        }
    }

    fn call_udf(&self, env: &Env, udf: &Udf, args: &[Expr]) -> Result<Val> {
        if args.len() > udf.params.len() {
            return Err(DaxError::evaluation(format!(
                "The function expects {} argument(s) but {} were given.",
                udf.params.len(),
                args.len()
            )));
        }
        let mut vars = self.query_vars.clone();
        for (i, p) in udf.params.iter().enumerate() {
            let value = match args.get(i) {
                Some(a) => match self.eval(env, a)? {
                    Val::Scalar(v) => VarValue::Scalar(v),
                    Val::Table(t) => VarValue::Table(t),
                },
                None => VarValue::Scalar(Value::Blank),
            };
            vars = vars.bind(&p.name, value);
        }
        let body_env = Env {
            vars,
            ..env.clone()
        };
        self.eval(&body_env, &udf.body)
    }

    // ---- references ------------------------------------------------------

    /// A table reference: a query table, or the visible rows of a stored
    /// table (without the blank row, like DAX).
    pub fn table_ref(&self, env: &Env, name: &str) -> Result<Arc<VirtualTable>> {
        if let Some(t) = self.query_tables.get(&name_key(name)) {
            return Ok(Arc::clone(t));
        }
        let data = self
            .snapshot
            .table(name)
            .map_err(|_| DaxError::UnknownTable {
                name: name.to_owned(),
            })?;
        let visible = self.snapshot.visible(&env.filter, data.name())?;
        Ok(Arc::new(VirtualTable::from_base(
            Arc::clone(data),
            visible.rows,
            false,
        )))
    }

    /// Value of a column in the row context (exact names first, then
    /// lineage), innermost frame first.
    pub fn frame_value(&self, env: &Env, table: Option<&str>, column: &str) -> Option<Value> {
        for f in env.rows.frames() {
            if let Some(i) = f.table.find_exact(table, column) {
                return Some(f.table.value(f.pos, i));
            }
        }
        let t = table?;
        for f in env.rows.frames() {
            if let Some(i) = f.table.find_by_lineage(t, column) {
                return Some(f.table.value(f.pos, i));
            }
        }
        None
    }

    fn column_ref(&self, env: &Env, table: Option<&str>, column: &str) -> Result<Value> {
        match table {
            None => {
                if let Some(m) = self.find_measure(None, column)? {
                    return self.eval_measure(env, &m);
                }
                if let Some(v) = self.frame_value(env, None, column) {
                    return Ok(v);
                }
                if self.resolve_column(env, None, column).is_ok() {
                    return Err(no_row_context(None, column));
                }
                Err(DaxError::UnknownMeasure {
                    name: column.to_owned(),
                })
            }
            Some(t) => {
                if let Some(v) = self.frame_value(env, Some(t), column) {
                    return Ok(v);
                }
                if let Some(m) = self.find_measure(Some(t), column)? {
                    return self.eval_measure(env, &m);
                }
                match self.snapshot.table(t) {
                    Ok(data) if data.has_column(column) => Err(no_row_context(Some(t), column)),
                    Ok(_) => Err(DaxError::unknown_column(t, column)),
                    Err(_) if self.query_tables.contains_key(&name_key(t)) => {
                        Err(no_row_context(Some(t), column))
                    }
                    Err(_) => Err(DaxError::UnknownTable { name: t.to_owned() }),
                }
            }
        }
    }

    /// Resolves a column reference to a stored `(table, column)` (canonical
    /// spelling). Bare `[Column]` looks at the row context, then the home
    /// table, then a unique table with that column.
    pub fn resolve_column(
        &self,
        env: &Env,
        table: Option<&str>,
        column: &str,
    ) -> Result<(String, String)> {
        if let Some(t) = table {
            let data = self.snapshot.table(t).map_err(|_| {
                if self.query_tables.contains_key(&name_key(t)) {
                    DaxError::evaluation(format!(
                        "Columns of the query table '{t}' cannot be used here."
                    ))
                } else {
                    DaxError::UnknownTable { name: t.to_owned() }
                }
            })?;
            let col = data
                .column_name(column)
                .map_err(|_| DaxError::unknown_column(data.name(), column))?;
            return Ok((data.name().to_owned(), col.to_owned()));
        }
        for f in env.rows.frames() {
            if let Some(i) = f.table.find_exact(None, column)
                && let Some(lin) = f.table.columns.get(i).and_then(|c| c.lineage.as_ref())
            {
                return Ok((lin.table.clone(), lin.column.clone()));
            }
        }
        if let Some(home) = env.home.as_deref()
            && let Ok(data) = self.snapshot.table(home)
            && let Ok(col) = data.column_name(column)
        {
            return Ok((data.name().to_owned(), col.to_owned()));
        }
        let mut found = None;
        for name in self.snapshot.table_names() {
            if let Ok(data) = self.snapshot.table(&name)
                && let Ok(col) = data.column_name(column)
            {
                if found.is_some() {
                    return Err(DaxError::evaluation(format!(
                        "The column [{column}] exists in several tables; qualify it with a table name."
                    )));
                }
                found = Some((data.name().to_owned(), col.to_owned()));
            }
        }
        found.ok_or_else(|| DaxError::UnknownColumn {
            name: format!("[{column}]"),
        })
    }

    /// A column-reference argument resolved to a stored column.
    pub fn column_arg(&self, env: &Env, e: &Expr, function: &str) -> Result<(String, String)> {
        match &e.unparen().kind {
            ExprKind::ColumnRef { table, column } => {
                self.resolve_column(env, table.as_deref(), column)
            }
            _ => Err(DaxError::argument(
                function,
                format!("expects a column reference, got {}", snippet(e)),
            )),
        }
    }

    /// `true` if `name` is a stored or query table.
    pub fn table_exists(&self, name: &str) -> bool {
        self.query_tables.contains_key(&name_key(name)) || self.snapshot.table(name).is_ok()
    }

    // ---- measures --------------------------------------------------------

    pub fn find_measure(&self, table: Option<&str>, name: &str) -> Result<Option<MeasureRef>> {
        let key = name_key(name);
        if let Some(q) = self.query_measures.get(&key)
            && table.is_none_or(|t| names_eq(t, &q.table))
        {
            return Ok(Some(MeasureRef {
                table: q.table.clone(),
                name: q.name.clone(),
                expr: Arc::clone(&q.expr),
                query: true,
            }));
        }
        let found = match table {
            Some(t) => self
                .model
                .table(t)
                .and_then(|tb| tb.measure(name).map(|m| (tb, m))),
            None => self.model.find_measure(name),
        };
        let Some((tb, m)) = found else {
            return Ok(None);
        };
        let cached = self.measures.borrow().get(&key).cloned();
        let expr = match cached {
            Some(e) => e,
            None => {
                let parsed = parse_expression(&m.expression).map_err(|e| {
                    let (line, col) = e.line_col(&m.expression);
                    DaxError::Parse {
                        message: format!("in measure [{}]: {e}", m.name),
                        line,
                        col,
                    }
                })?;
                let parsed = Arc::new(parsed);
                self.measures.borrow_mut().insert(key, Arc::clone(&parsed));
                parsed
            }
        };
        Ok(Some(MeasureRef {
            table: tb.name.clone(),
            name: m.name.clone(),
            expr,
            query: false,
        }))
    }

    /// Evaluates a measure: context transition, then the calculation item
    /// that applies (see [`super::calcgroup`]), or the measure body in the
    /// resulting filter context with no row context.
    pub fn eval_measure(&self, env: &Env, m: &MeasureRef) -> Result<Value> {
        let key = name_key(&m.name);
        let cycle = {
            let stack = self.stack.borrow();
            stack.iter().position(|s| name_key(s) == key).map(|start| {
                let mut chain: Vec<String> =
                    stack.iter().skip(start).map(|s| format!("[{s}]")).collect();
                chain.push(format!("[{}]", m.name));
                chain
            })
        };
        if let Some(chain) = cycle {
            return Err(DaxError::CircularDependency { chain });
        }
        let filter = self.transition(env)?;
        if !self.calc_groups.is_empty()
            && let Some(app) = self.next_application(&filter, &env.applied)?
        {
            return self.apply_calc_item(env, filter, m, &app);
        }
        let menv = Env {
            filter,
            rows: Default::default(),
            vars: if m.query {
                self.query_vars.clone()
            } else {
                VarScope::new()
            },
            home: Some(Arc::from(m.table.as_str())),
            shadow: Arc::clone(&env.shadow),
            scope: Arc::clone(&env.scope),
            applied: env.applied.clone(),
            selected: None,
        };
        self.stack.borrow_mut().push(m.name.clone());
        let result = self.scalar(&menv, &m.expr);
        self.stack.borrow_mut().pop();
        result
    }

    /// Context transition: every row context becomes a filter (outer rows
    /// first, inner rows override). A row of a stored table filters its
    /// expanded table to that row; a row of a materialized table filters
    /// each column that has lineage to its value.
    pub fn transition(&self, env: &Env) -> Result<Arc<FilterContext>> {
        if env.rows.is_empty() {
            return Ok(Arc::clone(&env.filter));
        }
        let mut f: FilterContext = (*env.filter).clone();
        let frames: Vec<&RowFrame> = env.rows.frames().collect();
        for frame in frames.into_iter().rev() {
            self.apply_row_filter(&mut f, frame);
        }
        Ok(Arc::new(f))
    }

    fn apply_row_filter(&self, f: &mut FilterContext, frame: &RowFrame) {
        let t = &frame.table;
        if let Some(data) = &t.base {
            let n = data.row_count();
            let row = t.base_row(frame.pos);
            let filter = if row == BLANK_ROW {
                TableFilter::new(data.name(), RowSet::empty(n)).with_blank_row(true)
            } else {
                TableFilter::new(data.name(), RowSet::from_rows(n, [row]))
            };
            let graph = self.snapshot.graph_for(f);
            f.replace_table(filter, &graph);
            return;
        }
        for (i, col) in t.columns.iter().enumerate() {
            if let Some(lin) = &col.lineage
                && self
                    .snapshot
                    .table(&lin.table)
                    .is_ok_and(|d| d.has_column(&lin.column))
            {
                f.replace_column(pbi_engine::ColumnFilter::in_values(
                    &lin.table,
                    &lin.column,
                    [t.value(frame.pos, i)],
                ));
            }
        }
    }

    // ---- iteration -------------------------------------------------------

    /// Calls `f` for each row of `table` with a row context on it; stops
    /// when `f` returns `false`.
    pub fn iterate(
        &self,
        env: &Env,
        table: &Arc<VirtualTable>,
        mut f: impl FnMut(&Env, RowPos) -> Result<bool>,
    ) -> Result<()> {
        for pos in table.positions() {
            self.check_cancel()?;
            let row_env = env.with_row(RowFrame::new(Arc::clone(table), pos));
            if !f(&row_env, pos)? {
                break;
            }
        }
        Ok(())
    }

    /// `ALL(table[c1], table[c2], …)`: distinct value combinations over all
    /// rows (plus the blank-row combination when the table has one).
    /// Cached per interpreter.
    pub fn all_columns(
        &self,
        table: &str,
        cols: &[String],
        with_blank_row: bool,
    ) -> Result<Arc<VirtualTable>> {
        let cache_key = format!(
            "{}|{}|{with_blank_row}",
            name_key(table),
            cols.iter()
                .map(|c| name_key(c))
                .collect::<Vec<_>>()
                .join("|")
        );
        if let Some(t) = self.all_cache.borrow().get(&cache_key) {
            return Ok(Arc::clone(t));
        }
        let data = self.snapshot.table(table)?;
        let all = RowSet::all(data.row_count());
        let blank = with_blank_row && self.snapshot.graph().has_blank_row(data.name());
        let mut rows: Vec<Vec<Value>> = if let [col] = cols {
            self.snapshot
                .distinct(data.name(), col, &all)?
                .into_iter()
                .map(|v| vec![v])
                .collect()
        } else {
            let refs: Vec<ColumnRef> = cols
                .iter()
                .map(|c| ColumnRef::new(data.name(), c))
                .collect();
            self.snapshot
                .group_index(data.name(), &refs, &all)?
                .into_values()
        };
        if blank && !rows.iter().any(|r| r.iter().all(Value::is_blank)) {
            rows.push(vec![Value::Blank; cols.len()]);
        }
        let columns = cols
            .iter()
            .map(|c| VColumn::model(data.name(), c, data.column_type(c).ok().cloned()))
            .collect();
        let t = Arc::new(VirtualTable::materialized(columns, rows));
        self.all_cache
            .borrow_mut()
            .insert(cache_key, Arc::clone(&t));
        Ok(t)
    }
}

/// Column names of a table constructor: `Value`, or `Value1`, `Value2`, ….
pub(crate) fn constructor_columns(width: usize) -> Vec<VColumn> {
    if width == 1 {
        vec![VColumn::new("Value")]
    } else {
        (1..=width)
            .map(|i| VColumn::new(format!("Value{i}")))
            .collect()
    }
}

fn no_row_context(table: Option<&str>, column: &str) -> DaxError {
    let name = match table {
        Some(t) => format!("{}[{column}]", pbi_model::dax_table_ref(t)),
        None => format!("[{column}]"),
    };
    DaxError::evaluation(format!(
        "A single value for column {name} cannot be determined: there is no row context for it. Use an aggregation such as MAX or an iterator."
    ))
}

/// One-column, at-most-one-row table → scalar.
pub(crate) fn table_to_scalar(t: &VirtualTable, e: &Expr) -> Result<Value> {
    if t.column_count() != 1 {
        return Err(DaxError::type_error(format!(
            "The expression {} returns a table with {} columns where a single value is required.",
            snippet(e),
            t.column_count()
        )));
    }
    match t.positions().as_slice() {
        [] => Ok(Value::Blank),
        [p] => Ok(t.value(*p, 0)),
        _ => Err(DaxError::type_error(format!(
            "A table of multiple values was supplied where a single value was expected ({}).",
            snippet(e)
        ))),
    }
}
