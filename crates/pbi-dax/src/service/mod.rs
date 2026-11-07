//! DAX language service: what editors need besides evaluation.
//!
//! - [`Schema`]: the tables, columns, measures and user-defined functions an
//!   expression can refer to, usually [`Schema::from_model`].
//! - [`diagnostics`]: syntax errors, unknown tables, columns, measures and
//!   variables, wrong argument counts, and functions DashCraft cannot
//!   evaluate, with byte spans ([`line_col`] turns an offset into a line and
//!   column).
//! - [`catalog`]: signature, parameters, argument count, return type and a
//!   one-line description (DashCraft's words) of every supported function.
//! - [`signature_help`] and [`hover`] at a cursor offset; both work on
//!   tokens, so text that does not parse yet still gets answers.
//!
//! The registry exposes it as `dax.diagnostics`, `dax.signatureHelp` and
//! `dax.hover` (`pbi-ops`, through `pbi-session`).

pub mod catalog;
mod diagnostics;
mod help;
mod schema;

pub use catalog::{FunctionInfo, FunctionParam, ReturnType};
pub use diagnostics::{Diagnostic, DiagnosticCode, MAX_DIAGNOSTICS, SourceKind, diagnostics};
pub use help::{Hover, HoverKind, SignatureHelp, hover, signature_help};
pub use schema::{NameDetails, Schema, SchemaTable};

pub use crate::eval::is_supported_function;
pub use crate::syntax::{Span, line_col};
