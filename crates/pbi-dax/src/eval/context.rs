//! Evaluation context: filter context, row context (stack of row frames),
//! variable scope.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pbi_engine::{CancellationToken, ColumnFilter, EngineSnapshot, FilterContext, RowSet, Value};
use pbi_model::names::name_key;
use pbi_model::{ColumnRef, Model};

use super::calcgroup::Applied;
use super::error::{DaxError, Result};
use super::interp::MeasureRef;
use super::table::{RowPos, VirtualTable};

/// One row context: a row of a (virtual) table being iterated.
#[derive(Debug, Clone)]
pub struct RowFrame {
    pub(crate) table: Arc<VirtualTable>,
    pub(crate) pos: RowPos,
}

impl RowFrame {
    pub(crate) fn new(table: Arc<VirtualTable>, pos: RowPos) -> Self {
        Self { table, pos }
    }

    /// The table being iterated.
    pub fn table(&self) -> &VirtualTable {
        &self.table
    }

    /// Values of the row.
    pub fn values(&self) -> Vec<Value> {
        self.table.row_values(self.pos)
    }
}

#[derive(Debug)]
struct RowNode {
    frame: RowFrame,
    parent: Option<Arc<RowNode>>,
}

/// The row context: a persistent stack of [`RowFrame`]s (innermost first
/// when iterated). Cloning is cheap.
#[derive(Debug, Clone, Default)]
pub struct RowContext {
    head: Option<Arc<RowNode>>,
}

impl RowContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.head.is_none()
    }

    /// A new context with `frame` as the innermost row.
    pub fn push(&self, frame: RowFrame) -> RowContext {
        RowContext {
            head: Some(Arc::new(RowNode {
                frame,
                parent: self.head.clone(),
            })),
        }
    }

    /// A new context with row `row` of stored table `table` as the
    /// innermost row (as in a calculated column).
    pub fn push_base_row(
        &self,
        snapshot: &EngineSnapshot,
        table: &str,
        row: usize,
    ) -> Result<RowContext> {
        let data = Arc::clone(snapshot.table(table)?);
        if row >= data.row_count() {
            return Err(DaxError::evaluation(format!(
                "row {row} is out of range for table '{}'",
                data.name()
            )));
        }
        let n = data.row_count();
        let vt = VirtualTable::from_base(data, RowSet::all(n), false);
        Ok(self.push(RowFrame::new(Arc::new(vt), RowPos::Base(row))))
    }

    /// Frames, innermost first.
    pub fn frames(&self) -> impl Iterator<Item = &RowFrame> {
        let mut node = self.head.as_deref();
        std::iter::from_fn(move || {
            let n = node?;
            node = n.parent.as_deref();
            Some(&n.frame)
        })
    }
}

/// Value of a variable.
#[derive(Debug, Clone)]
pub enum VarValue {
    Scalar(Value),
    Table(Arc<VirtualTable>),
}

#[derive(Debug)]
struct VarNode {
    key: String,
    value: VarValue,
    parent: Option<Arc<VarNode>>,
}

/// Variables in scope (persistent list, innermost first; names are
/// case-insensitive).
#[derive(Debug, Clone, Default)]
pub struct VarScope {
    head: Option<Arc<VarNode>>,
}

impl VarScope {
    pub fn new() -> Self {
        Self::default()
    }

    /// A new scope with `name` bound to `value` (shadowing outer bindings).
    pub fn bind(&self, name: &str, value: VarValue) -> VarScope {
        VarScope {
            head: Some(Arc::new(VarNode {
                key: name_key(name),
                value,
                parent: self.head.clone(),
            })),
        }
    }

    pub fn get(&self, name: &str) -> Option<&VarValue> {
        let key = name_key(name);
        let mut node = self.head.as_deref();
        while let Some(n) = node {
            if n.key == key {
                return Some(&n.value);
            }
            node = n.parent.as_deref();
        }
        None
    }
}

/// Cancellation signal checked during evaluation.
#[derive(Debug, Clone)]
pub enum Cancel {
    Flag(Arc<AtomicBool>),
    Token(CancellationToken),
}

impl Cancel {
    pub fn is_cancelled(&self) -> bool {
        match self {
            Cancel::Flag(f) => f.load(Ordering::Relaxed),
            Cancel::Token(t) => t.is_cancelled(),
        }
    }
}

/// Everything an evaluation needs. Build one with [`EvalContext::new`],
/// refine it with the `with_*` methods, then evaluate expressions:
///
/// - `filter`: the filter context (e.g. report/visual filters);
/// - `row`: the row context (empty for measures; one frame per row of a
///   calculated column);
/// - `vars`: variables in scope;
/// - `depth`: evaluation depth already used by the caller;
/// - `cancel`: optional cancellation flag, checked in every loop;
/// - `allselected`: the *shadow* filter context that `ALLSELECTED`
///   restores (`None`: `filter` itself);
/// - `group_by`: the columns grouped at the current level of a visual
///   (`ISINSCOPE`).
///
/// # Visual calculations (`ALLSELECTED`, `ISINSCOPE`)
///
/// A visual is evaluated once per data point. Its *outer* filter context
/// (report, page and visual filters, slicers, cross-highlights) is what
/// `ALLSELECTED` restores; the data point adds one filter per group-by
/// column. Either set both explicitly:
///
/// ```ignore
/// let ctx = EvalContext::new(snapshot, model)
///     .with_filter(outer.clone().with(ColumnFilter::in_values("Product", "Color", [color])))
///     .with_allselected(outer)
///     .with_group_by([ColumnRef::new("Product", "Color")]);
/// ```
///
/// or use [`EvalContext::with_data_point`], which does the same from the
/// current `filter` (a total row is simply a data point with fewer
/// group-by values). `SUMMARIZECOLUMNS` sets both internally for every
/// row it produces (its filter arguments are part of the shadow context;
/// `ROLLUPADDISSUBTOTAL` subtotal rows are not in scope for the rolled-up
/// columns).
#[derive(Debug, Clone)]
pub struct EvalContext {
    pub snapshot: Arc<EngineSnapshot>,
    pub model: Arc<Model>,
    pub filter: FilterContext,
    pub row: RowContext,
    pub vars: VarScope,
    pub depth: usize,
    pub cancel: Option<Cancel>,
    /// Shadow filter context restored by `ALLSELECTED` (`None`: `filter`).
    pub allselected: Option<FilterContext>,
    /// Columns grouped at the current level (`ISINSCOPE`).
    pub group_by: Vec<ColumnRef>,
}

impl EvalContext {
    pub fn new(snapshot: Arc<EngineSnapshot>, model: Arc<Model>) -> Self {
        Self {
            snapshot,
            model,
            filter: FilterContext::new(),
            row: RowContext::new(),
            vars: VarScope::new(),
            depth: 0,
            cancel: None,
            allselected: None,
            group_by: Vec::new(),
        }
    }

    pub fn with_filter(mut self, filter: FilterContext) -> Self {
        self.filter = filter;
        self
    }

    /// Sets the shadow filter context that `ALLSELECTED` restores: the
    /// visual's outer filters, before its group-by filters.
    pub fn with_allselected(mut self, filter: FilterContext) -> Self {
        self.allselected = Some(filter);
        self
    }

    /// Sets the columns grouped at the current level of a visual:
    /// `ISINSCOPE(c)` is true for them while they are filtered.
    pub fn with_group_by(mut self, columns: impl IntoIterator<Item = ColumnRef>) -> Self {
        self.group_by = columns.into_iter().collect();
        self
    }

    /// One data point of a visual: the current `filter` becomes the
    /// `ALLSELECTED` shadow context (unless one was set), then each
    /// `(column, value)` replaces the filters on its column (as
    /// `SUMMARIZECOLUMNS` does) and the columns become the group-by scope
    /// (`ISINSCOPE`). Pass no values for the grand total.
    pub fn with_data_point(mut self, values: &[(ColumnRef, Value)]) -> Self {
        if self.allselected.is_none() {
            self.allselected = Some(self.filter.clone());
        }
        for (col, value) in values {
            self.filter.replace_column(ColumnFilter::in_values(
                col.table.as_str(),
                col.column.as_str(),
                [value.clone()],
            ));
        }
        self.group_by = values.iter().map(|(c, _)| c.clone()).collect();
        self
    }

    pub fn with_cancel_flag(mut self, flag: Arc<AtomicBool>) -> Self {
        self.cancel = Some(Cancel::Flag(flag));
        self
    }

    pub fn with_cancellation_token(mut self, token: CancellationToken) -> Self {
        self.cancel = Some(Cancel::Token(token));
        self
    }

    /// Adds row `row` of stored table `table` as the innermost row context.
    pub fn with_base_row(mut self, table: &str, row: usize) -> Result<Self> {
        self.row = self.row.push_base_row(&self.snapshot, table, row)?;
        Ok(self)
    }

    pub fn with_var(mut self, name: &str, value: VarValue) -> Self {
        self.vars = self.vars.bind(name, value);
        self
    }

    pub(crate) fn options(&self) -> super::EvalOptions {
        super::EvalOptions {
            filter: self.filter.clone(),
            row: self.row.clone(),
            vars: self.vars.clone(),
            depth: self.depth,
            cancel: self.cancel.clone(),
            allselected: self.allselected.clone(),
            group_by: self.group_by.clone(),
            general_only: false,
        }
    }

    /// Evaluates a scalar DAX expression.
    pub fn evaluate_scalar(&self, src: &str) -> Result<Value> {
        super::evaluate_scalar_with(&self.snapshot, &self.model, src, &self.options())
    }

    /// Evaluates a table DAX expression.
    pub fn evaluate_table(&self, src: &str) -> Result<VirtualTable> {
        super::evaluate_table_with(&self.snapshot, &self.model, src, &self.options())
    }

    /// Evaluates a model measure (`Name`, `[Name]` or `Table[Name]`) in this
    /// context (with context transition when a row context is set).
    pub fn evaluate_measure(&self, measure: &str) -> Result<Value> {
        super::evaluate_measure_with(&self.snapshot, &self.model, measure, &self.options())
    }

    /// Evaluates a model measure (like [`EvalContext::evaluate_measure`])
    /// together with its effective format string: the dynamic format string
    /// of the highest-precedence calculation item that applies, else the
    /// measure's own `formatStringDefinition`, else its `formatString`.
    pub fn evaluate_measure_formatted(&self, measure: &str) -> Result<(Value, Option<String>)> {
        super::evaluate_measure_formatted_with(
            &self.snapshot,
            &self.model,
            measure,
            &self.options(),
        )
    }

    /// Rows of stored table `table` for which the DAX condition `filter` is
    /// true (a row-level security filter expression), evaluated row by row
    /// in this context's filter context: `(rows, blank row accepted)`.
    pub fn evaluate_row_filter(&self, table: &str, filter: &str) -> Result<(RowSet, bool)> {
        super::evaluate_row_filter_with(&self.snapshot, &self.model, table, filter, &self.options())
    }

    /// Evaluates a DAX query; `filter` acts as an outer filter context
    /// (e.g. report and page filters).
    pub fn evaluate_query(&self, query_src: &str) -> Result<Vec<VirtualTable>> {
        super::query::evaluate_query_with(&self.snapshot, &self.model, query_src, &self.options())
    }

    /// [`super::compute_calculated_column`] honouring `cancel`.
    pub fn compute_calculated_column(
        &self,
        table: &str,
        column: &str,
    ) -> Result<arrow::array::ArrayRef> {
        super::calc::compute_calculated_column_with(
            &self.snapshot,
            &self.model,
            table,
            column,
            &self.options(),
        )
    }

    /// [`super::compute_calculated_table`] honouring `cancel`.
    pub fn compute_calculated_table(
        &self,
        table: &str,
    ) -> Result<arrow::record_batch::RecordBatch> {
        super::calc::compute_calculated_table_with(
            &self.snapshot,
            &self.model,
            table,
            &self.options(),
        )
    }
}

/// Internal per-evaluation environment (cheap to clone).
#[derive(Debug, Clone)]
pub(crate) struct Env {
    pub filter: Arc<FilterContext>,
    pub rows: RowContext,
    pub vars: VarScope,
    /// Table owning the expression (measure or calculated column), used to
    /// resolve bare `[Column]` references.
    pub home: Option<Arc<str>>,
    /// Shadow filter context restored by `ALLSELECTED`.
    pub shadow: Arc<FilterContext>,
    /// Group-by columns of the current level (`ISINSCOPE`).
    pub scope: Arc<[ColumnRef]>,
    /// Calculation items applied along this evaluation.
    pub applied: Applied,
    /// The measure `SELECTEDMEASURE()` stands for (in calculation item and
    /// format string expressions).
    pub selected: Option<Arc<MeasureRef>>,
}

impl Env {
    /// An environment whose shadow context is `filter` itself.
    pub fn new(filter: FilterContext) -> Self {
        let filter = Arc::new(filter);
        Self {
            shadow: Arc::clone(&filter),
            filter,
            rows: RowContext::new(),
            vars: VarScope::new(),
            home: None,
            scope: Arc::from(Vec::new()),
            applied: Applied::default(),
            selected: None,
        }
    }

    pub fn with_row(&self, frame: RowFrame) -> Env {
        Env {
            rows: self.rows.push(frame),
            ..self.clone()
        }
    }

    /// Same variables, home table, shadow context and scope; new filter
    /// context, no row context (the expression of `CALCULATE`).
    pub fn with_filter(&self, filter: Arc<FilterContext>) -> Env {
        Env {
            filter,
            rows: RowContext::new(),
            ..self.clone()
        }
    }

    pub fn with_var(&self, name: &str, value: VarValue) -> Env {
        Env {
            vars: self.vars.bind(name, value),
            ..self.clone()
        }
    }

    /// `true` if `table[column]` is a group-by column of the current level.
    pub fn in_scope(&self, table: &str, column: &str) -> bool {
        self.scope.iter().any(|c| c.matches(table, column))
    }
}
