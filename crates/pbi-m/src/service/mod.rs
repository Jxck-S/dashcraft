//! M language service (M7.3): what editors need about M text without evaluating it.
//!
//! * [`catalog`] / [`catalog_entry`]: Power BI Desktop's standard library (every function with
//!   its parameters and types, type and value constants), recorded from Desktop and marked
//!   [`LibraryEntry::implemented`] where DashCraft evaluates the name.

mod library;

pub use library::{
    LibraryEntry, LibraryFunction, LibraryKind, LibraryParam, catalog, catalog_desktop_version,
    catalog_entry,
};
