//! The TMDL language service: the keyword catalog recorded from Microsoft's TMDL reader
//! ([`catalog`]), diagnostics over a set of TMDL files ([`diagnostics`]), completion and
//! hover ([`complete`], [`hover`]), and the formatter ([`format`]).

mod catalog;
mod complete;
mod diagnostics;
mod format;
mod outline;

pub use catalog::{Catalog, Context, Form, Keyword, KeywordKind, Support, ValueType, catalog};
pub use complete::{
    CompletionItem, Completions, Declaration, FileText, Hover, ItemKind, complete, hover,
};
pub use diagnostics::{
    MAX_DIAGNOSTICS, Severity, Source, TmdlDiagnostic, codes, diagnostics, node_form,
};
pub use format::{FormatError, FormatOptions, MAX_INDENT_WIDTH, format};

pub use crate::lexer::{IndentStyle, LineEnding};
