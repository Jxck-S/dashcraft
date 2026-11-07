//! Compile-time name binding.
//!
//! Power BI Desktop's mashup engine binds names when it compiles a query: a query that names
//! anything that is neither in scope nor in the standard library fails as a whole, even where
//! the name would never be evaluated (an unused step, inside `try`, an untaken branch, a
//! function body). Queries report `The name 'X' wasn't recognized.  Make sure it's spelled
//! correctly.`; `Expression.Evaluate` reports `[1,9-1,19] The name 'X' doesn't exist in the
//! current context.` with the name's range. Both were checked against Desktop
//! (`reference/groundtruth`, suite `m-functions`, cases `names/*`).

use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use crate::ast::Name;
use crate::merror::{Location, MError};
use crate::refs::FreeNames;
use crate::service::{catalog, catalog_entry};
use crate::span::{Span, line_col};

/// Whether `name` is in Desktop's standard library or implemented by DashCraft.
pub(crate) fn in_library(name: &str) -> bool {
    catalog_entry(name).is_some() || crate::stdlib::library().get(name).is_some()
}

/// The namespaces of the standard library: the text before the last `.` of its dotted names
/// (`Table`, `Text`, `Int64`, `AzureStorage`).
fn namespaces() -> &'static HashSet<&'static str> {
    static NAMESPACES: OnceLock<HashSet<&'static str>> = OnceLock::new();
    NAMESPACES.get_or_init(|| {
        catalog()
            .iter()
            .filter_map(|e| e.name.rsplit_once('.').map(|(ns, _)| ns))
            .collect()
    })
}

/// A dotted name outside the library's namespaces, e.g. a custom connector's
/// `MyConnector.Contents`. DashCraft cannot evaluate those and reports them as unsupported where
/// they are evaluated, with a hint to load the data manually, rather than as unknown names. A
/// misspelled library member (`Table.RowCnt`) is in a library namespace and stays unknown.
pub(crate) fn is_extension_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
        && name
            .rsplit_once('.')
            .is_some_and(|(ns, _)| !namespaces().contains(ns))
}

/// The first free name in source order that is not `in_scope`. In a query (`query`), the library
/// and [extension names](is_extension_name) are in scope too; `Expression.Evaluate` sees only
/// its environment.
pub(crate) fn first_unbound<'e>(
    free: &FreeNames<'e>,
    in_scope: impl Fn(&str) -> bool,
    query: bool,
) -> Option<&'e Name> {
    free.free
        .iter()
        .copied()
        .filter(|n| {
            !in_scope(&n.text) && !(query && (in_library(&n.text) || is_extension_name(&n.text)))
        })
        .min_by_key(|n| n.span.start)
}

/// The message of an unknown name in a query.
pub(crate) fn unrecognized_message(name: &str) -> String {
    format!("The name '{name}' wasn't recognized.  Make sure it's spelled correctly.")
}

/// The error of an unknown name in a query (`query` is `None` for the main query).
pub(crate) fn unrecognized(name: &Name, query: Option<Arc<str>>, src: &str) -> MError {
    let (line, column) = line_col(src, name.span.start);
    MError::Evaluation {
        reason: "Expression.Error".into(),
        message: unrecognized_message(&name.text),
        detail: None,
        location: Some(Box::new(Location {
            query,
            span: name.span,
            line,
            column,
        })),
        fields: None,
    }
}

/// Desktop's range prefix `[l,c-l,c]` (1-based, the end column exclusive).
pub(crate) fn range_prefix(src: &str, span: Span) -> String {
    let (l1, c1) = line_col(src, span.start);
    let (l2, c2) = line_col(src, span.end);
    format!("[{l1},{c1}-{l2},{c2}]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_names_and_namespaces() {
        assert!(in_library("Table.RowCount"));
        // In Desktop's catalog, not evaluated by DashCraft.
        assert!(in_library("Web.Contents"));
        assert!(!in_library("Table.RowCnt"));
        assert!(!in_library("text.upper"));
        assert!(!is_extension_name("Table.RowCnt"));
        assert!(!is_extension_name("Undefined"));
        assert!(is_extension_name("MyConnector.Contents"));
        assert_eq!(
            range_prefix("let a = Undefined2 in 2", Span::new(8, 18)),
            "[1,9-1,19]"
        );
    }
}
