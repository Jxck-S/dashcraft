//! DAX lexer, parser and evaluator.
//!
//! - [`syntax`]: lexer, spanned AST, Pratt parser, pretty-printer and
//!   reference visitor. Independent of the model and engine crates.
//! - [`format`](mod@format): Power BI / Excel-style format strings ([`format_value`]),
//!   used by the DAX `FORMAT` function and to display measure and column
//!   values with their model `formatString`.
//! - [`eval`]: the formula engine — filter context, row context, context
//!   transition, BLANK semantics and the function library — on top of the
//!   `pbi-engine` storage engine. Its public API (measures, scalar/table
//!   expressions, queries, calculated columns/tables, dependencies) is
//!   re-exported at the crate root. `FUNCTIONS.md` (next to `Cargo.toml`)
//!   lists every supported function with its status and the known semantic
//!   gaps.
//!
//! Visuals evaluate measures per data point with
//! [`EvalContext::with_data_point`] (or [`EvalContext::with_allselected`] +
//! [`EvalContext::with_group_by`]) so that `ALLSELECTED` and `ISINSCOPE`
//! work; `USERELATIONSHIP`/`CROSSFILTER` become
//! [`pbi_engine::RelationshipOverride`]s on the filter context.

pub mod eval;
pub mod format;
pub mod syntax;

pub use eval::{
    Cancel, DaxError, Dependency, EVAL_STACK_SIZE, EvalContext, MAX_EVAL_DEPTH, RowContext,
    RowFrame, VColumn, VarScope, VarValue, VirtualTable, compute_calculated_column,
    compute_calculated_table, dependencies, dependencies_in, evaluate_measure, evaluate_query,
    evaluate_scalar, evaluate_table, is_supported_function, supported_functions, to_record_batch,
    transitive_dependencies,
};
pub use format::format_value;
pub use syntax::*;
