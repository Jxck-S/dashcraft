//! The DAX formula engine (evaluator).
//!
//! # Formula engine vs storage engine
//!
//! As in Analysis Services, evaluation is split in two. This module is the
//! **formula engine (FE)**: it walks the [`crate::syntax`] AST, owns row
//! context, context transition, `CALCULATE` filter semantics, measure
//! resolution, variables, type coercion and BLANK arithmetic. The **storage
//! engine (SE)**, [`pbi_engine`], answers "which rows of table `T` are
//! visible in filter context `F`" (relationship propagation, blank rows,
//! expanded-table filters) and runs scans, distinct/group-by and
//! aggregates. The FE never touches Arrow arrays directly except to build
//! results.
//!
//! # Semantics implemented
//!
//! - **Filter context** ([`pbi_engine::FilterContext`]) flows through every
//!   evaluation; table references (`'Web Sessions'`) are the visible rows,
//!   without the blank row. `VALUES(T)`/`ALL(T)`/`VALUES(T[c])`/
//!   `ALL(T[c])`/`DISTINCTCOUNT` include the blank row (or its BLANK value)
//!   when visible; `DISTINCT`, `ALLNOBLANKROW` and plain table references
//!   do not.
//! - **Row context**: iterators (`FILTER`, `SUMX`, `ADDCOLUMNS`, …) and
//!   calculated columns push row frames; `T[c]` reads the innermost frame
//!   that has the column (by name, then lineage); `EARLIER` reads outer
//!   frames.
//! - **Context transition**: a measure reference (`[Measure]`),
//!   `CALCULATE`, `CALCULATETABLE` and `RELATEDTABLE` turn every row frame
//!   into filters (a stored-table row filters its expanded table to that
//!   row; a materialized row filters its lineage columns). The `CALCULATE`
//!   expression and measure bodies then run without row context.
//! - **`CALCULATE` filter arguments** are evaluated in the original context;
//!   boolean predicates become `FILTER(ALL(col), pred)` → `IN` filters
//!   replacing filters on that column; table arguments (`FILTER(…)`,
//!   `VALUES(…)`, `ALL(…)`, variables) become table filters (stored rows →
//!   expanded-table filter; materialized columns → lineage column/tuple
//!   filters); modifiers `ALL`, `REMOVEFILTERS`, `ALLEXCEPT`,
//!   `ALLNOBLANKROW`; `KEEPFILTERS` intersects. Several filters on one
//!   column intersect.
//! - **BLANK** per Microsoft's table (see the private `ops` module): `BLANK +
//!   1 = 1`, `BLANK * 1 = BLANK`, `BLANK / BLANK = BLANK`, `5 / BLANK =
//!   ∞`, `BLANK = 0` is true, `BLANK == 0` is false, `BLANK && TRUE` is
//!   false, `BLANK & "a" = "a"`; `DIVIDE` returns the alternate result (or
//!   BLANK) for a zero/BLANK denominator; `IN` uses strict equality.
//! - **Relationship modifiers**: `USERELATIONSHIP` and `CROSSFILTER` in
//!   `CALCULATE` set [`pbi_engine::RelationshipOverride`]s on the new filter
//!   context; the storage engine propagates filters on the graph they
//!   describe. A filter on a date column keying a date table removes the
//!   other filters on that table (time intelligence in a month row).
//! - **Visual calculations**: every evaluation carries a *shadow* filter
//!   context (the query's outer filters, or a visual's filters before its
//!   group-by values: [`EvalContext::with_allselected`],
//!   [`EvalContext::with_data_point`]) that `ALLSELECTED` restores, and the
//!   group-by columns of the current level (`ISINSCOPE`).
//!   `SUMMARIZECOLUMNS` sets both for every row, including
//!   `ROLLUPADDISSUBTOTAL` subtotal rows.
//! - **Names** are case-insensitive (tables, columns, measures, variables,
//!   functions); text comparison is case-insensitive.
//! - **Measures**: bare `[X]` is a measure first, then a row-context
//!   column. Measure → measure cycles fail with
//!   [`DaxError::CircularDependency`]; nesting is capped at
//!   [`MAX_EVAL_DEPTH`] ([`DaxError::DepthExceeded`]). Every public entry
//!   point runs on a dedicated thread with a large stack, never panics, and
//!   reports unsupported functions as [`DaxError::UnsupportedFunction`].
//!
//! - **Calculation groups**: a measure reference evaluated in a filter
//!   context that selects one calculation item evaluates the item with
//!   `SELECTEDMEASURE()` bound to the measure (highest precedence
//!   outermost; selection expressions for multiple, empty and no
//!   selections; an applied item is never re-applied below itself). See
//!   the private `calcgroup` module; dynamic format strings come from
//!   [`EvalContext::evaluate_measure_formatted`].
//! - **Row-level security** is the storage engine's job (a restricted
//!   snapshot, see [`pbi_engine::security`]); `USERPRINCIPALNAME()` & co.
//!   read the snapshot's effective user, and
//!   [`EvalContext::evaluate_row_filter`] evaluates role filters.
//!
//! # Entry points
//!
//! [`evaluate_measure`], [`evaluate_scalar`], [`evaluate_table`],
//! [`evaluate_query`] (`DEFINE`/`EVALUATE`/`ORDER BY`/`START AT`),
//! [`compute_calculated_column`], [`compute_calculated_table`],
//! [`dependencies`] / [`transitive_dependencies`] (refresh planning) and
//! [`to_record_batch`]. [`EvalContext`] bundles a snapshot, model, filter
//! context, row context, variables and a cancellation signal for the same
//! operations. The function library is listed by [`supported_functions`];
//! `crates/pbi-dax/FUNCTIONS.md` has the per-function status.
//!
//! # Known gaps
//!
//! DirectQuery, `NONVISUAL`, `DEFINE COLUMN`,
//! `SUMMARIZE` rollups, `FORMAT` cultures other than `en-US`, `EXPR` (lazy)
//! parameters of user-defined functions (all parameters are bound by
//! value). `ALLSELECTED` knows one shadow context per query / data point
//! (iterations do not create new ones). `RELATED` ignores relationship
//! overrides and cannot follow many-to-many (limited) relationships, as in
//! Power BI. Context transition filters a stored row by row number, so
//! duplicate rows of a table without a key count once. Tuple filters on
//! columns of several tables are applied per table (correlation across
//! tables is lost).

mod calc;
mod calcgroup;
mod context;
mod deps;
mod error;
mod fast;
mod filters;
mod functions;
mod interp;
mod ops;
mod query;
mod table;

use std::sync::Mutex;

use pbi_engine::{EngineSnapshot, FilterContext, RowSet, Value};
use pbi_model::Model;

pub use calc::{compute_calculated_column, compute_calculated_table};
pub use context::{Cancel, EvalContext, RowContext, RowFrame, VarScope, VarValue};
pub use deps::{Dependency, dependencies, dependencies_in, transitive_dependencies};
pub use error::{DaxError, MAX_EVAL_DEPTH, Result};
pub use functions::{is_supported_function, supported_functions};
pub use query::evaluate_query;
pub use table::{VColumn, VirtualTable, to_record_batch};

use context::Env;
use interp::{Interp, parse};

/// Stack size of the evaluation thread.
pub const EVAL_STACK_SIZE: usize = 64 * 1024 * 1024;

/// Context options shared by the public entry points.
#[derive(Debug, Clone, Default)]
pub(crate) struct EvalOptions {
    pub filter: FilterContext,
    pub row: RowContext,
    pub vars: VarScope,
    pub depth: usize,
    pub cancel: Option<Cancel>,
    pub allselected: Option<FilterContext>,
    pub group_by: Vec<pbi_model::ColumnRef>,
    /// Skip the fast paths (tests compare them with the general path).
    pub general_only: bool,
}

impl EvalOptions {
    fn with_filter(filter: &FilterContext) -> Self {
        Self {
            filter: filter.clone(),
            ..Self::default()
        }
    }

    fn env(&self) -> Env {
        let mut env = Env::new(self.filter.clone());
        env.rows = self.row.clone();
        env.vars = self.vars.clone();
        if let Some(shadow) = &self.allselected {
            env.shadow = std::sync::Arc::new(shadow.clone());
        }
        env.scope = self.group_by.clone().into();
        env
    }
}

/// Runs `f` on a thread with an [`EVAL_STACK_SIZE`] stack (falls back to
/// the current thread if one cannot be spawned). A panic inside `f` is
/// reported as an error instead of unwinding into the caller.
pub(crate) fn on_eval_thread<T: Send>(f: impl FnOnce() -> Result<T> + Send) -> Result<T> {
    let slot = Mutex::new(Some(f));
    let take = || slot.lock().ok().and_then(|mut g| g.take());
    let outcome = std::thread::scope(|scope| {
        let spawned = std::thread::Builder::new()
            .name("pbi-dax-eval".into())
            .stack_size(EVAL_STACK_SIZE)
            .spawn_scoped(scope, || take().map(|f| f()));
        spawned.ok().map(|handle| handle.join())
    });
    match outcome {
        Some(Ok(Some(result))) => result,
        Some(Err(_)) => Err(DaxError::evaluation(
            "internal error: the DAX evaluator failed unexpectedly",
        )),
        Some(Ok(None)) | None => match take() {
            Some(f) => f(),
            None => Err(DaxError::evaluation(
                "internal error: the DAX evaluation could not be started",
            )),
        },
    }
}

/// Evaluates a model measure in `filter`. `measure` is `Name`, `[Name]`,
/// `Table[Name]` or `'Table'[Name]`.
pub fn evaluate_measure(
    snapshot: &EngineSnapshot,
    model: &Model,
    measure: &str,
    filter: &FilterContext,
) -> Result<Value> {
    evaluate_measure_with(snapshot, model, measure, &EvalOptions::with_filter(filter))
}

/// Splits a measure spec into `(table, name)`.
fn measure_spec(measure: &str) -> Result<(Option<String>, String)> {
    let m = measure.trim();
    if !m.contains('[') {
        return Ok((None, m.to_owned()));
    }
    match parse(m)?.kind {
        crate::syntax::ExprKind::ColumnRef { table, column } => Ok((table, column)),
        _ => Err(DaxError::UnknownMeasure { name: m.to_owned() }),
    }
}

pub(crate) fn evaluate_measure_with(
    snapshot: &EngineSnapshot,
    model: &Model,
    measure: &str,
    opts: &EvalOptions,
) -> Result<Value> {
    let (table, name) = measure_spec(measure)?;
    on_eval_thread(|| {
        let interp = Interp::with_options(snapshot, model, opts);
        let m = interp
            .find_measure(table.as_deref(), &name)?
            .ok_or_else(|| DaxError::UnknownMeasure { name: name.clone() })?;
        interp.eval_measure(&opts.env(), &m)
    })
}

/// Evaluates a model measure and its effective format string: the
/// `formatStringDefinition` of the highest-precedence calculation item that
/// applies, else the measure's own dynamic format string, else its static
/// `formatString` (`None`: no format string). A failing format string
/// expression falls back to the static format string.
pub(crate) fn evaluate_measure_formatted_with(
    snapshot: &EngineSnapshot,
    model: &Model,
    measure: &str,
    opts: &EvalOptions,
) -> Result<(Value, Option<String>)> {
    let (table, name) = measure_spec(measure)?;
    on_eval_thread(|| {
        let interp = Interp::with_options(snapshot, model, opts);
        let m = interp
            .find_measure(table.as_deref(), &name)?
            .ok_or_else(|| DaxError::UnknownMeasure { name: name.clone() })?;
        let env = opts.env();
        let value = interp.eval_measure(&env, &m)?;
        let format = match interp.measure_format_string(&env, &m) {
            Ok(format) => format,
            Err(e) if e.is_fatal() => return Err(e),
            Err(_) => model
                .measure(&m.table, &m.name)
                .and_then(|x| x.format_string.clone()),
        };
        Ok((value, format))
    })
}

/// The effective format string of a model measure (as
/// [`evaluate_measure_formatted_with`], without evaluating the value) at
/// each data point of a visual: `points[i]` holds one value per column of
/// `columns`, which replace the filters on those columns (as
/// [`EvalContext::with_data_point`]); `opts`' filter context is the
/// `ALLSELECTED` shadow context unless one is set. All points run on one
/// evaluation thread and share the interpreter's caches.
pub(crate) fn evaluate_measure_format_strings_with(
    snapshot: &EngineSnapshot,
    model: &Model,
    measure: &str,
    opts: &EvalOptions,
    columns: &[pbi_model::ColumnRef],
    points: &[Vec<Value>],
) -> Result<Vec<Option<String>>> {
    let (table, name) = measure_spec(measure)?;
    on_eval_thread(|| {
        let interp = Interp::with_options(snapshot, model, opts);
        let m = interp
            .find_measure(table.as_deref(), &name)?
            .ok_or_else(|| DaxError::UnknownMeasure { name: name.clone() })?;
        let fallback = model
            .measure(&m.table, &m.name)
            .and_then(|x| x.format_string.clone());
        let base = opts.env();
        let shadow = std::sync::Arc::new(
            opts.allselected
                .clone()
                .unwrap_or_else(|| opts.filter.clone()),
        );
        let scope: std::sync::Arc<[pbi_model::ColumnRef]> = columns.to_vec().into();
        let mut out = Vec::with_capacity(points.len());
        for point in points {
            interp.check_cancel()?;
            let mut filter = opts.filter.clone();
            for (col, value) in columns.iter().zip(point) {
                filter.replace_column(pbi_engine::ColumnFilter::in_values(
                    col.table.as_str(),
                    col.column.as_str(),
                    [value.clone()],
                ));
            }
            let mut env = base.with_filter(std::sync::Arc::new(filter));
            env.rows = base.rows.clone();
            env.shadow = std::sync::Arc::clone(&shadow);
            env.scope = std::sync::Arc::clone(&scope);
            out.push(match interp.measure_format_string(&env, &m) {
                Ok(format) => format,
                Err(e) if e.is_fatal() => return Err(e),
                Err(_) => fallback.clone(),
            });
        }
        Ok(out)
    })
}

impl EvalContext {
    /// The effective format string of a model measure (see
    /// [`EvalContext::evaluate_measure_formatted`]) at each data point of a
    /// visual, without evaluating its value: `points[i]` holds one value
    /// per column of `columns` (the data point's group-by values, as in
    /// [`EvalContext::with_data_point`]; an empty point is the grand
    /// total). Meant for visual queries whose measures have dynamic format
    /// strings (calculation items or `formatStringDefinition`): one call
    /// per measure and result set instead of one evaluation per cell.
    pub fn evaluate_measure_format_strings(
        &self,
        measure: &str,
        columns: &[pbi_model::ColumnRef],
        points: &[Vec<Value>],
    ) -> Result<Vec<Option<String>>> {
        evaluate_measure_format_strings_with(
            &self.snapshot,
            &self.model,
            measure,
            &self.options(),
            columns,
            points,
        )
    }
}

/// Rows of stored table `table` for which the DAX condition `filter` (e.g. a
/// role's row filter) is true, evaluated in a row context on each row with
/// `opts`' filter context: `(rows, blank row accepted)`. The blank row is
/// tested (all columns BLANK) only when the table has one.
pub(crate) fn evaluate_row_filter_with(
    snapshot: &EngineSnapshot,
    model: &Model,
    table: &str,
    filter: &str,
    opts: &EvalOptions,
) -> Result<(RowSet, bool)> {
    let expr = parse(filter)?;
    on_eval_thread(|| {
        let interp = Interp::with_options(snapshot, model, opts);
        let data = snapshot.table(table).map_err(|_| DaxError::UnknownTable {
            name: table.to_owned(),
        })?;
        let n = data.row_count();
        let has_blank = snapshot.graph().has_blank_row(data.name());
        let rows = std::sync::Arc::new(VirtualTable::from_base(
            std::sync::Arc::clone(data),
            RowSet::all(n),
            has_blank,
        ));
        let mut env = opts.env();
        env.home = Some(std::sync::Arc::from(data.name()));
        let mut accepted = RowSet::empty(n);
        let mut blank = false;
        interp.iterate(&env, &rows, |row_env, pos| {
            if interp.condition(row_env, &expr)? {
                match rows.base_row(pos) {
                    table::BLANK_ROW => blank = true,
                    row => {
                        accepted.insert(row);
                    }
                }
            }
            Ok(true)
        })?;
        Ok((accepted, blank))
    })
}

/// Evaluates a scalar DAX expression in `filter` (no row context).
pub fn evaluate_scalar(
    snapshot: &EngineSnapshot,
    model: &Model,
    dax_src: &str,
    filter: &FilterContext,
) -> Result<Value> {
    evaluate_scalar_with(snapshot, model, dax_src, &EvalOptions::with_filter(filter))
}

pub(crate) fn evaluate_scalar_with(
    snapshot: &EngineSnapshot,
    model: &Model,
    dax_src: &str,
    opts: &EvalOptions,
) -> Result<Value> {
    let expr = parse(dax_src)?;
    on_eval_thread(|| {
        let interp = Interp::with_options(snapshot, model, opts);
        interp.scalar(&opts.env(), &expr)
    })
}

/// Evaluates a table DAX expression in `filter`.
pub fn evaluate_table(
    snapshot: &EngineSnapshot,
    model: &Model,
    dax_src: &str,
    filter: &FilterContext,
) -> Result<VirtualTable> {
    evaluate_table_with(snapshot, model, dax_src, &EvalOptions::with_filter(filter))
}

pub(crate) fn evaluate_table_with(
    snapshot: &EngineSnapshot,
    model: &Model,
    dax_src: &str,
    opts: &EvalOptions,
) -> Result<VirtualTable> {
    let expr = parse(dax_src)?;
    on_eval_thread(|| {
        let interp = Interp::with_options(snapshot, model, opts);
        Ok((*interp.table(&opts.env(), &expr)?).clone())
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::Arc;

    use arrow::array::{ArrayRef, Int64Array, StringArray};
    use arrow::record_batch::RecordBatch;
    use pbi_engine::{ColumnFilter, Engine, FilterContext, Value};
    use pbi_model::{
        CalculationGroup, CalculationItem, Column, ColumnKind, ColumnRef, DataType, Measure, Model,
        PropValue, Table,
    };

    use super::EvalContext;

    /// `Sales` (`Region`, `Amount`) with `[Total]` (`$#,0`), `[Dyn]` (a
    /// value-dependent `formatStringDefinition`) and a `Calc` group whose
    /// `Pct` item has its own format string.
    fn ctx() -> EvalContext {
        let mut sales = Table::new("Sales");
        sales.columns = vec![
            Column::data("Region", DataType::String),
            Column::data("Amount", DataType::Int64),
        ];
        let mut total = Measure::new("Total", "SUM(Sales[Amount])");
        total.format_string = Some("$#,0".into());
        let mut dynamic = Measure::new("Dyn", "[Total]");
        dynamic.extra.insert(
            "formatStringDefinition".into(),
            PropValue::Expression("IF(SELECTEDMEASURE() > 100, \"#,0.0\", \"0\")".into()),
        );
        sales.measures = vec![total, dynamic];
        let mut calc = Table::new("Calc");
        let mut name = Column::data("Name", DataType::String);
        name.kind = ColumnKind::Data {
            source_column: "Name".into(),
        };
        calc.columns = vec![name, Column::data("Ordinal", DataType::Int64)];
        calc.calculation_group = Some(Box::new(CalculationGroup {
            calculation_items: vec![
                CalculationItem::new("Base", "SELECTEDMEASURE()"),
                CalculationItem {
                    format_string_definition: Some("\"0.0%\"".into()),
                    ..CalculationItem::new("Pct", "SELECTEDMEASURE() / 1000")
                },
            ],
            ..CalculationGroup::default()
        }));
        let model = Model {
            tables: vec![sales, calc],
            ..Model::default()
        };
        let mut engine = Engine::new(&model);
        let sales_rows = RecordBatch::try_from_iter([
            (
                "Region",
                Arc::new(StringArray::from(vec!["N", "S"])) as ArrayRef,
            ),
            (
                "Amount",
                Arc::new(Int64Array::from(vec![50, 500])) as ArrayRef,
            ),
        ])
        .unwrap();
        let calc_rows = RecordBatch::try_from_iter([
            (
                "Name",
                Arc::new(StringArray::from(vec!["Base", "Pct"])) as ArrayRef,
            ),
            (
                "Ordinal",
                Arc::new(Int64Array::from(vec![0, 1])) as ArrayRef,
            ),
        ])
        .unwrap();
        engine
            .load_tables([
                ("Sales".to_owned(), sales_rows),
                ("Calc".to_owned(), calc_rows),
            ])
            .unwrap();
        EvalContext::new(engine.snapshot(), Arc::new(model))
    }

    #[test]
    fn format_strings_per_data_point() {
        let ctx = ctx();
        let item = ColumnRef::new("Calc", "Name");
        let points = vec![
            vec![Value::from("Base")],
            vec![Value::from("Pct")],
            vec![Value::from("Nope")],
        ];
        let formats = ctx
            .evaluate_measure_format_strings("[Total]", std::slice::from_ref(&item), &points)
            .unwrap();
        assert_eq!(
            formats,
            vec![
                Some("$#,0".into()),
                Some("0.0%".into()),
                Some("$#,0".into())
            ]
        );
        // The grand total and a point agree with evaluate_measure_formatted.
        let region = ColumnRef::new("Sales", "Region");
        let formats = ctx
            .evaluate_measure_format_strings(
                "Sales[Dyn]",
                std::slice::from_ref(&region),
                &[vec![Value::from("N")], vec![Value::from("S")], vec![]],
            )
            .unwrap();
        assert_eq!(
            formats,
            vec![Some("0".into()), Some("#,0.0".into()), Some("#,0.0".into())]
        );
        let north = ctx
            .clone()
            .with_filter(FilterContext::new().with(ColumnFilter::in_values(
                "Sales",
                "Region",
                [Value::from("N")],
            )));
        assert_eq!(
            north.evaluate_measure_formatted("Sales[Dyn]").unwrap().1,
            Some("0".into())
        );
        // The data point replaces an outer filter on its column.
        let formats = north
            .evaluate_measure_format_strings(
                "Sales[Dyn]",
                std::slice::from_ref(&region),
                &[vec![Value::from("S")]],
            )
            .unwrap();
        assert_eq!(formats, vec![Some("#,0.0".into())]);
        assert!(
            ctx.evaluate_measure_format_strings("[Missing]", &[], &[vec![]])
                .is_err()
        );
    }
}
