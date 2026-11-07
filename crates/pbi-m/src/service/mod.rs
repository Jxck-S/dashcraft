//! M language service (M7.3): what editors need about M text without evaluating it.
//!
//! * [`catalog`] / [`catalog_entry`]: Power BI Desktop's standard library (every function with
//!   its parameters and types, type and value constants), recorded from Desktop and marked
//!   [`LibraryEntry::implemented`] where DashCraft evaluates the name.
//! * [`diagnostics`]: syntax errors, unknown names (with a suggestion), wrong argument counts of
//!   library functions, and names DashCraft cannot evaluate, with Power BI Desktop's messages.
//! * [`hover`], [`signature_help`] and [`complete`]: what a name is, the signature of the call
//!   at a cursor, and the names that fit there. They bind names as Desktop does (the text's
//!   steps, parameters and fields, then the model's queries in [`Environment`], then the
//!   library) and work on text that does not parse yet.
//! * [`format`]: lays an expression or section document out deterministically, keeping its
//!   tokens and comments, as Power BI Desktop writes queries (one `let` step per line).

mod complete;
mod diagnostics;
mod environment;
mod format;
mod help;
mod library;
mod names;
mod scan;

pub use complete::{CompletionItem, Completions, complete};
pub use diagnostics::{
    Diagnostic, DiagnosticCode, MAX_DIAGNOSTICS, Severity, argument_count_message, diagnostics,
};
pub use environment::{Environment, EnvironmentEntry, EnvironmentKind};
pub use format::{FormatError, FormatOptions, MAX_INDENT_WIDTH, format};
pub use help::{Hover, SignatureHelp, SignatureParameter, hover, signature_help};
pub use names::NameKind;

pub use library::{
    LibraryEntry, LibraryFunction, LibraryKind, LibraryParam, catalog, catalog_desktop_version,
    catalog_entry,
};
