//! M language service (M7.3): what editors need about M text without evaluating it.
//!
//! * [`catalog`] / [`catalog_entry`]: Power BI Desktop's standard library (every function with
//!   its parameters and types, type and value constants), recorded from Desktop and marked
//!   [`LibraryEntry::implemented`] where DashCraft evaluates the name.
//! * [`diagnostics`]: syntax errors, unknown names (with a suggestion), wrong argument counts of
//!   library functions, and names DashCraft cannot evaluate, with Power BI Desktop's messages.

mod diagnostics;
mod library;

pub use diagnostics::{
    Diagnostic, DiagnosticCode, Environment, MAX_DIAGNOSTICS, Severity, argument_count_message,
    diagnostics,
};

pub use library::{
    LibraryEntry, LibraryFunction, LibraryKind, LibraryParam, catalog, catalog_desktop_version,
    catalog_entry,
};
