//! DAX language service: what editors need besides evaluation.
//!
//! - [`Schema`]: the tables, columns, measures and user-defined functions an
//!   expression can refer to, usually [`Schema::from_model`].
//! - [`diagnostics`]: syntax errors, unknown tables, columns, measures and
//!   variables, and functions DashCraft cannot evaluate, with byte spans
//!   ([`line_col`] turns an offset into a line and column).
//!
//! The registry exposes it as `dax.diagnostics` (`pbi-ops`, through
//! `pbi-session`).

mod diagnostics;
mod schema;

pub use diagnostics::{Diagnostic, DiagnosticCode, MAX_DIAGNOSTICS, SourceKind, diagnostics};
pub use schema::{Schema, SchemaTable};

pub use crate::syntax::{Span, line_col};
