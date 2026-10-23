//! Visual queryState + report/page/visual filters + slicer selections +
//! cross-filter → DAX evaluation → [`DataView`]. (owner: m4-semantic-query)
//!
//! # API
//!
//! - [`query_visual`] / [`query_page`]: one visual / every visual of a page
//!   → [`DataView`] or a per-visual [`VisualQueryError`] (a user-facing
//!   message and, for a failing measure, its DAX text: render it as ⚠).
//!   Nothing panics; filters that cannot be applied are reported in
//!   [`DataView::warnings`], never dropped silently.
//! - [`page_filter_context`]: the engine [`FilterContext`] a visual is
//!   evaluated in (for cross-checks with `pbi_dax::evaluate_measure`).
//! - [`InteractionState`]: slicer selections + selected data points;
//!   [`InteractionState::from_page`] reads the selections persisted in PBIR
//!   (see [`interaction`] for where they live).
//! - [`FilterBuilder`]: report-AST conditions → engine filters.
//! - [`QueryEnv`]: the same operations on a fixed engine snapshot + model
//!   (one consistent snapshot per page; usable without a `Session`).
//!
//! The functions are synchronous and CPU-bound: call them from a blocking
//! thread (`tokio::task::spawn_blocking`) in async code, and cancel through
//! [`QueryLimits::cancel`].
//!
//! # How a visual is evaluated
//!
//! 1. **Filter context** ([`page_filter_context`]), intersected:
//!    report filters, page filters, the visual's own filters, the
//!    selections of the *other* slicers on the page, and the selected data
//!    points of another visual — the last two honouring the page's
//!    `visualInteractions` (`NoFilter` → skipped; `HighlightFilter` →
//!    applied as a filter, with a warning; default `DataFilter`). Top N
//!    filters are evaluated last, in the context built so far.
//!    `isHiddenInViewMode` / `isLockedInViewMode` do not change evaluation.
//!    Filters on tables without a relationship path to a measure's tables
//!    have no effect on it (engine propagation), so a measure over a table
//!    without relationships ignores a filter on any other table.
//! 2. **Query**: `EVALUATE SUMMARIZECOLUMNS(<group columns>, "v0", <value>,
//!    …)` (or `ROW(…)` without group columns) with the filter context as the
//!    outer filter. Values are measures (`'Orders'[Total Sales]`) or
//!    aggregations (`SUM(Orders[Quantity])`). Rows whose values
//!    are all BLANK are dropped (SUMMARIZECOLUMNS semantics).
//! 3. **Post-processing**: visual-level measure filters (rows), sort, row /
//!    category / series limits (`truncated`), totals by re-evaluating the
//!    values with fewer group columns, formatting with the measure / column
//!    format string in the model culture.
//!
//! **Dynamic format strings.** A measure that may have one (its own
//! `formatStringDefinition`, or a calculation group with format strings
//! that is filtered or grouped by the visual, or formats its
//! `noSelectionExpression`) gets its effective format string per result
//! row, like the format-string columns of Power BI's visual queries:
//! `pbi_dax::EvalContext::evaluate_measure_format_strings` evaluates them in
//! one batch per measure and result set, and the cell is formatted with it
//! (kept in [`crate::dataview::CellValue::format`] when it differs from the
//! column's). Other measures cost nothing extra.
//!
//! **Conditional-formatting inputs.** Fields that the visual's conditional
//! formats ([`crate::conditional`]) are based on and that are not values of
//! the visual are added to the same queries as hidden values (so they take
//! part in dropping all-BLANK rows, as in Power BI). An input that is not
//! in the model or cannot be evaluated is dropped with a warning; the
//! visual still renders.
//!
//! # Role mapping
//!
//! | Visual type | Roles → query | DataView |
//! |---|---|---|
//! | `textbox`, `shape`, `basicShape`, `actionButton`, `image`, `pageNavigator`, `bookmarkNavigator`, groups | no query | `Empty` |
//! | `card`, `cardVisual` | every role (`Values`, `Data`, …) → values | `Single` (one field) or a one-row `Table` |
//! | `slicer` (+ `advancedSlicerVisual`, `listSlicer`, `textSlicer`) | `Values` → group columns (active projections), evaluated without its own selection | `Categorical` (categories only) + `slicer` selection |
//! | `tableEx`, `table`, `multiRowCard` | `Values`: columns → group, measures / aggregations → values | `Table` (+ totals row) |
//! | `pivotTable`, `matrix` | `Rows`, `Columns` → row / column levels; `Values` → values | `Matrix` (subtotals at every level) |
//! | `pieChart`, `donutChart`, `funnel`, `treemap` | `Category` → categories; `Series` → series; `Y` (and any other role) → values | `Categorical` |
//! | `*BarChart`, `*ColumnChart`, `lineChart`, `areaChart`, other `*Chart`, `gauge`, `kpi`, `map`, `filledMap` | `Category`/`Axis`/`Details`/`Group`/`Rows`/`Location`/`Play`/`TrendLine` (kpi) → categories; `Series`/`Legend`/`ColorBy` → series; `Y`, `Y2`, `Tooltips`, `Size`, `X`, … → values | `Categorical` |
//! | anything else | columns → group, measures → values | `Table` (placeholder data) |
//!
//! Only `active` projections are queried when a role marks some active
//! (drill level). A field bound to several roles is one metadata column
//! with several roles.
//!
//! **Implicit measures.** A plain column in a value role is summarized
//! by default: the model's `summarizeBy` when it is an aggregation (`sum`,
//! `min`, `max`, `count` → `COUNTA`, `average`, `distinctCount`); otherwise
//! `SUM` for numeric columns and `COUNTA` ("Count") for text, dates,
//! booleans and `summarizeBy: none`; on cards a text column shows its first
//! value (`MIN`). Explicit `Aggregation`s map to `SUM`, `AVERAGE`, `MIN`,
//! `MAX`, `COUNTA` (CountNonNull), `DISTINCTCOUNT`, `MEDIAN`, `STDEV.P`,
//! `VAR.P` (the last three need DAX support; otherwise the visual reports
//! the unsupported function).
//!
//! **Sorting.** `sortDefinition` (fields of the visual; a column's model
//! `sortByColumn` is honoured). Defaults: bar/column/pie/donut/funnel/
//! treemap/waterfall/ribbon charts by the first value descending; other
//! charts by category ascending; tables by the first column (ascending, or
//! descending for a value); slicers and matrix row and column levels
//! ascending (by the sort-by column when the model sets one). Series
//! are ordered ascending. Text sorts case-insensitively, BLANK first.
//!
//! # Filter support
//!
//! | Filter | Report / page | Visual | Slicer selection / cross-filter |
//! |---|---|---|---|
//! | Categorical `In` / `Not In` (one column) | ✅ | ✅ | ✅ |
//! | Tuple `In` (several columns, one or related tables) | ✅ | ✅ | ✅ |
//! | Advanced `=` `<>` `<` `>` `<=` `>=`, `Between`, `And` / `Or` / `Not` | ✅ | ✅ | ✅ (persisted range) |
//! | Contains / StartsWith (and negations), is (not) blank / empty | ✅ | ✅ | — |
//! | Top N (by measure or aggregation) | ✅ (evaluated in the visual's context) | ✅ | — |
//! | Measure / aggregation conditions (`[Total Sales] > 5`) | ⚠ not applied | ✅ on the visual's rows (not matrices) | — |
//! | Relative date / relative time | ⚠ not applied | ⚠ not applied | ⚠ not applied |
//! | Auto date/time hierarchy levels, `Exists`, `In` over tables, comparisons between fields | ⚠ not applied | ⚠ not applied | ⚠ not applied |
//!
//! ⚠ = reported as a [`QueryWarning`] with `not_applied: true`.
//!
//! # Known gaps
//!
//! - Highlight interactions are applied as filters (`ValueColumn::highlights`
//!   stays `None`); a visual's own selection does not highlight it.
//! - Synced slicers (`syncGroup`) and the `selfFilter` of newer slicers are
//!   not read; drill-down / drill-through state is not modelled beyond
//!   `active` projections.
//! - Totals and subtotals are re-evaluations with fewer group columns
//!   (right for non-additive measures too). `ALLSELECTED` and `ISINSCOPE`
//!   work through `SUMMARIZECOLUMNS` / `ROW`: the shadow context is the
//!   visual's filter context (filters, slicers, cross-filter), the scope
//!   the group columns of the row being evaluated.
//! - Group columns from unrelated tables cross-join (as in
//!   SUMMARIZECOLUMNS); combinations whose measures are all BLANK are
//!   dropped.
//! - Data reduction is a plain cap (`QueryLimits`), not Power BI's
//!   sampling algorithms.

mod context;
mod exec;
pub mod filter;
pub mod interaction;
mod plan;
mod semantic;

use std::sync::Arc;

use pbi_dax::{DaxError, EvalContext};
use pbi_engine::{CancellationToken, EngineSnapshot, FilterContext, Value};
use pbi_model::{Model, Page, Report, Visual};
use pbi_session::Session;
use serde::Serialize;

pub use crate::dataview::{DataView, QueryWarning, WarningScope};
pub use filter::{
    ColumnRef, FilterBuilder, FilterPlan, coerce_literal, coerce_value, parse_datetime,
};
pub use interaction::{
    InteractionState, PointField, PointSelection, SlicerSelection, is_slicer_type,
    persisted_selection, slicer_fields,
};
pub use semantic::TopNSpec;

use filter::ColumnRef as Col;

/// Query limits and cancellation.
#[derive(Debug, Clone)]
pub struct QueryLimits {
    /// Rows of a table / matrix, values of a slicer (default 30 000).
    pub max_rows: usize,
    /// Categories of a chart, columns of a matrix (default 1 000).
    pub max_categories: usize,
    /// Series of a chart (default 60).
    pub max_series: usize,
    /// Cancels the evaluation (the visual then fails with `cancelled`).
    pub cancel: CancellationToken,
}

impl Default for QueryLimits {
    fn default() -> Self {
        Self {
            max_rows: 30_000,
            max_categories: 1_000,
            max_series: 60,
            cancel: CancellationToken::new(),
        }
    }
}

/// A visual that could not be queried. Render it as a placeholder with ⚠,
/// the message, and the DAX text when present. The details are boxed (the
/// error travels through `Result`s); fields are reachable through `Deref`
/// ([`VisualErrorInfo`]).
#[derive(Debug, Clone, PartialEq, Serialize, thiserror::Error)]
#[serde(transparent)]
#[error("{0}")]
pub struct VisualQueryError(Box<VisualErrorInfo>);

/// Details of a [`VisualQueryError`].
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualErrorInfo {
    /// Visual name.
    pub visual: String,
    pub visual_type: String,
    /// User-facing message.
    pub message: String,
    /// Label of the failing field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// DAX of the failing measure (its expression) or of the query.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dax: Option<String>,
    /// The failure is an unsupported DAX feature.
    pub unsupported: bool,
    /// The query was cancelled.
    pub cancelled: bool,
    /// Filter warnings collected before the failure.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<QueryWarning>,
}

impl std::fmt::Display for VisualErrorInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}: {}", self.visual_type, self.visual, self.message)
    }
}

impl std::ops::Deref for VisualQueryError {
    type Target = VisualErrorInfo;
    fn deref(&self) -> &VisualErrorInfo {
        &self.0
    }
}

impl std::ops::DerefMut for VisualQueryError {
    fn deref_mut(&mut self) -> &mut VisualErrorInfo {
        &mut self.0
    }
}

impl VisualQueryError {
    fn new(visual: &Visual, message: impl Into<String>) -> Self {
        Self(Box::new(VisualErrorInfo {
            visual: visual.name.clone(),
            visual_type: visual.visual_type.clone(),
            message: message.into(),
            field: None,
            dax: None,
            unsupported: false,
            cancelled: false,
            warnings: Vec::new(),
        }))
    }

    fn with_field(mut self, field: impl Into<String>) -> Self {
        self.0.field = Some(field.into());
        self
    }

    fn with_dax(mut self, dax: impl Into<String>) -> Self {
        self.0.dax = Some(dax.into());
        self
    }

    fn cancelled(mut self) -> Self {
        self.0.cancelled = true;
        self
    }

    /// The details.
    pub fn info(&self) -> &VisualErrorInfo {
        &self.0
    }

    /// `⚠ <field>: <message>` for display.
    pub fn display(&self) -> String {
        match &self.field {
            Some(f) => format!("⚠ {f}: {}", self.message),
            None => format!("⚠ {}", self.message),
        }
    }
}

/// An engine snapshot + model + culture to query against.
#[derive(Debug, Clone)]
pub struct QueryEnv {
    pub snapshot: Arc<EngineSnapshot>,
    pub model: Arc<Model>,
    /// Culture used to format values (`en-US`, …).
    pub culture: String,
}

impl QueryEnv {
    pub fn new(
        snapshot: Arc<EngineSnapshot>,
        model: Arc<Model>,
        culture: impl Into<String>,
    ) -> Self {
        Self {
            snapshot,
            model,
            culture: culture.into(),
        }
    }

    /// The session's current snapshot, model and culture.
    pub fn from_session(session: &Session) -> Self {
        Self::new(session.snapshot(), session.model(), session.culture())
    }

    /// A [`FilterBuilder`] on this model and snapshot.
    pub fn builder(&self) -> FilterBuilder<'_> {
        FilterBuilder::new(&self.model, &self.snapshot)
    }

    /// See [`query_visual`].
    ///
    /// # Errors
    /// [`VisualQueryError`] when the visual cannot be evaluated.
    pub fn query_visual(
        &self,
        report: &Report,
        page: &Page,
        visual: &Visual,
        state: &InteractionState,
        limits: &QueryLimits,
    ) -> Result<DataView, VisualQueryError> {
        exec::run_visual(self, report, page, visual, state, limits)
    }

    /// See [`query_page`].
    pub fn query_page(
        &self,
        report: &Report,
        page: &Page,
        state: &InteractionState,
        limits: &QueryLimits,
    ) -> Vec<(String, Result<DataView, VisualQueryError>)> {
        page.visuals
            .iter()
            .map(|v| {
                (
                    v.name.clone(),
                    self.query_visual(report, page, v, state, limits),
                )
            })
            .collect()
    }

    /// See [`page_filter_context`].
    pub fn page_filter_context(
        &self,
        report: &Report,
        page: &Page,
        visual: &Visual,
        state: &InteractionState,
    ) -> (FilterContext, Vec<QueryWarning>) {
        let f = context::visual_filters(self, report, page, visual, state, &QueryLimits::default());
        let mut warnings = f.warnings;
        if !f.value_conditions.is_empty() {
            warnings.push(QueryWarning::new(
                WarningScope::Visual,
                "measure filters apply to the visual's rows and are not part of the filter context",
            ));
        }
        (f.context, warnings)
    }

    /// An evaluation context with `filter` as the filter context.
    pub fn eval_context(&self, filter: &FilterContext, limits: &QueryLimits) -> EvalContext {
        EvalContext::new(Arc::clone(&self.snapshot), Arc::clone(&self.model))
            .with_filter(filter.clone())
            .with_cancellation_token(limits.cancel.clone())
    }

    /// Rows of `SUMMARIZECOLUMNS(group…, "v0", values[0], …)` (or `ROW`
    /// without groups) in `filter`: group values, then values.
    pub(crate) fn summarize(
        &self,
        filter: &FilterContext,
        group: &[&Col],
        values: &[&str],
        limits: &QueryLimits,
    ) -> Result<Vec<Vec<Value>>, DaxError> {
        let query = exec::summarize_dax(group, values);
        let tables = self.eval_context(filter, limits).evaluate_query(&query)?;
        Ok(tables
            .first()
            .map(pbi_dax::VirtualTable::rows)
            .unwrap_or_default())
    }
}

/// Evaluates one visual: its projections under the report, page and visual
/// filters, the other slicers' selections and the cross-filter from the
/// selected visual (see the module docs for the role mapping).
///
/// # Errors
/// [`VisualQueryError`] when a field is missing from the model or a
/// measure cannot be evaluated (with its DAX text), or on cancellation.
pub fn query_visual(
    session: &Session,
    report: &Report,
    page: &Page,
    visual: &Visual,
    state: &InteractionState,
    limits: &QueryLimits,
) -> Result<DataView, VisualQueryError> {
    QueryEnv::from_session(session).query_visual(report, page, visual, state, limits)
}

/// Evaluates every visual of `page` on one engine snapshot, in page order.
pub fn query_page(
    session: &Session,
    report: &Report,
    page: &Page,
    state: &InteractionState,
    limits: &QueryLimits,
) -> Vec<(String, Result<DataView, VisualQueryError>)> {
    QueryEnv::from_session(session).query_page(report, page, state, limits)
}

/// The filter context `visual` is evaluated in: report + page + visual
/// filters, other slicers' selections and the cross-filter, per
/// `visualInteractions`, with Top N filters resolved. Warnings list what
/// was not applied (visual-level measure filters apply to rows and are not
/// part of the context).
pub fn page_filter_context(
    report: &Report,
    page: &Page,
    visual: &Visual,
    state: &InteractionState,
    session: &Session,
) -> (FilterContext, Vec<QueryWarning>) {
    QueryEnv::from_session(session).page_filter_context(report, page, visual, state)
}
