//! M reference checks (LINT013): identifiers in partition sources and
//! shared expressions that are not let steps, function parameters, record
//! fields, shared expressions/parameters, other queries (tables) or library
//! names.
//!
//! Heuristic: dotted names (`Table.SelectRows`, `Int64.Type`) are library
//! members and never flagged.

use std::collections::HashSet;

use pbi_m::{free_names, library_names, line_col, parse_expression};
use pbi_model::{Model, Severity, names_eq};

use super::{ExprSite, Finding, Sink, rules, suggest};

/// Names visible to every query of a model.
pub(crate) struct Globals<'a> {
    /// Shared expressions/parameters and table (query) names.
    queries: Vec<&'a str>,
    library: HashSet<&'static str>,
}

impl<'a> Globals<'a> {
    pub(crate) fn new(model: &'a Model) -> Self {
        let queries = model
            .expressions
            .iter()
            .map(|e| e.name.as_str())
            .chain(model.tables.iter().map(|t| t.name.as_str()))
            .collect();
        Self {
            queries,
            library: library_names().into_iter().collect(),
        }
    }
}

/// Checks one M expression for undefined references.
pub(crate) fn check(sink: &mut Sink<'_>, globals: &Globals<'_>, site: &ExprSite<'_>) {
    if site.expression.trim().is_empty() || !sink.enabled(rules::M_REF) {
        return;
    }
    let expr = match parse_expression(site.expression) {
        Ok(e) => e,
        Err(e) => {
            let (origin, position) = site.locate(line_col(site.expression, e.span.start));
            sink.push(
                Finding::new(
                    rules::M_REF,
                    Severity::Info,
                    format!(
                        "{}: M could not be parsed at {position} ({}); references not checked",
                        site.what, e.message
                    ),
                )
                .at(origin)
                .on(site.object),
            );
            return;
        }
    };
    let w = free_names(&expr);
    let mut reported: HashSet<&str> = HashSet::new();
    for free in &w.free {
        let (name, span) = (free.text.as_str(), free.span);
        if name.contains('.')
            || globals.queries.contains(&name)
            || globals.library.contains(name)
            || !reported.insert(name)
        {
            continue;
        }
        let (origin, position) = site.locate(line_col(site.expression, span.start));
        let case_match = globals.queries.iter().find(|q| names_eq(q, name));
        let (message, suggestion) = match case_match {
            Some(q) => (
                format!(
                    "{}: `{name}` does not match `{q}` (M names are case-sensitive) ({position})",
                    site.what
                ),
                Some(format!("use `{q}`")),
            ),
            None => (
                format!(
                    "{}: `{name}` is not a step, parameter, shared expression or query ({position})",
                    site.what
                ),
                suggest::closest(name, w.locals.iter().chain(globals.queries.iter()).copied())
                    .map(|c| format!("did you mean `{c}`?")),
            ),
        };
        sink.push(
            Finding::new(rules::M_REF, Severity::Warning, message)
                .at(origin)
                .on(site.object)
                .suggest(suggestion),
        );
    }
}
