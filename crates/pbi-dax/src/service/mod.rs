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
//! - [`signature_help`], [`hover`] and [`complete`] at a cursor offset; they
//!   work on tokens, so text that does not parse yet still gets answers.
//! - [`semantic_tokens`]: what every token is (keyword, function, table,
//!   column, measure, variable, parameter, literal, …) for highlighting, and
//!   [`references`]: the symbol at a cursor, where the text declares it and
//!   every place it occurs (go to definition, find references).
//! - [`format`]: a deterministic, idempotent layout of an expression or
//!   query that keeps comments, with options for line width, indentation
//!   and the case of keywords and function names.
//!
//! The registry exposes it as `dax.diagnostics`, `dax.signatureHelp`,
//! `dax.hover`, `dax.complete`, `dax.tokens`, `dax.references` and
//! `dax.format` (`pbi-ops`, through `pbi-session`).

pub mod catalog;
mod complete;
mod diagnostics;
mod formatter;
mod help;
mod schema;
mod scope;
mod symbols;

pub use catalog::{FunctionInfo, FunctionParam, ReturnType};
pub use complete::{CompletionItem, CompletionKind, Completions, complete};
pub use diagnostics::{Diagnostic, DiagnosticCode, MAX_DIAGNOSTICS, SourceKind, diagnostics};
pub use formatter::{Casing, FormatError, FormatOptions, MAX_INDENT_WIDTH, format};
pub use help::{Hover, HoverKind, SignatureHelp, hover, signature_help};
pub use schema::{NameDetails, Schema, SchemaTable};
pub use symbols::{
    ReferenceSite, References, SemanticKind, SemanticToken, Symbol, references, semantic_tokens,
};

pub use crate::eval::is_supported_function;
pub use crate::syntax::{Span, line_col};
