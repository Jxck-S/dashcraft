//! The TMDL language service: the keyword catalog recorded from Microsoft's TMDL reader
//! ([`catalog`]) and diagnostics over a set of TMDL files ([`diagnostics`]).

mod catalog;
mod diagnostics;

pub use catalog::{Catalog, Context, Form, Keyword, KeywordKind, Support, ValueType, catalog};
pub use diagnostics::{
    MAX_DIAGNOSTICS, Severity, Source, TmdlDiagnostic, codes, diagnostics, node_form,
};
