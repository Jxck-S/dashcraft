//! DAX reference checks: LINT007 (unknown table/column/measure) and LINT008
//! (syntax errors), using `pbi_dax::syntax`.

use std::collections::HashSet;

use pbi_dax::syntax::{
    ExprKind, Reference, for_each_expr, line_col, parse_expression, reference_sites,
};
use pbi_model::{Model, Severity, dax_column_ref, dax_measure_ref, dax_table_ref};

use super::{ExprSite, Finding, Sink, rules, suggest};

/// Checks one DAX expression against `model`.
pub(crate) fn check(sink: &mut Sink<'_>, model: &Model, site: &ExprSite<'_>) {
    if site.expression.trim().is_empty() {
        return;
    }
    let expr = match parse_expression(site.expression) {
        Ok(e) => e,
        Err(e) => {
            let (origin, position) = site.locate(line_col(site.expression, e.span.start));
            sink.push(
                Finding::new(
                    rules::DAX_SYNTAX,
                    Severity::Error,
                    format!("{}: DAX syntax error at {position}: {e}", site.what),
                )
                .at(origin)
                .on(site.object),
            );
            return;
        }
    };
    if !sink.enabled(rules::DAX_REF) {
        return;
    }

    let mut vars: HashSet<String> = HashSet::new();
    let mut strings: HashSet<String> = HashSet::new();
    for_each_expr(&expr, |e| match &e.kind {
        ExprKind::Var { bindings, .. } => {
            vars.extend(bindings.iter().map(|b| b.name.to_lowercase()));
        }
        ExprKind::String(s) => {
            strings.insert(s.to_lowercase());
        }
        _ => {}
    });

    let mut reported: HashSet<String> = HashSet::new();
    for (reference, span) in reference_sites(&expr) {
        let problem = match &reference {
            Reference::Table(t) if !vars.contains(&t.to_lowercase()) => missing_table(model, t),
            Reference::Column { table, column } if !vars.contains(&table.to_lowercase()) => {
                match model.table(table) {
                    None => missing_table(model, table),
                    Some(t)
                        if t.column(column).is_some()
                            || t.measure(column).is_some()
                            || strings.contains(&column.to_lowercase()) =>
                    {
                        None
                    }
                    Some(t) => Some((
                        format!("unknown column `{}`", dax_column_ref(&t.name, column)),
                        suggest::field(model, &t.name, column),
                    )),
                }
            }
            Reference::Measure(name) => {
                let known = model.find_measure(name).is_some()
                    || model.tables.iter().any(|t| t.column(name).is_some())
                    || strings.contains(&name.to_lowercase());
                (!known).then(|| {
                    let candidates = model
                        .tables
                        .iter()
                        .flat_map(|t| t.measures.iter().map(|m| m.name.as_str()));
                    (
                        format!("unknown measure or column `{}`", dax_measure_ref(name)),
                        suggest::closest(name, candidates)
                            .map(|m| format!("did you mean `{}`?", dax_measure_ref(m))),
                    )
                })
            }
            _ => None,
        };
        let Some((message, suggestion)) = problem else {
            continue;
        };
        if !reported.insert(message.to_lowercase()) {
            continue;
        }
        let (origin, position) = site.locate(line_col(site.expression, span.start));
        sink.push(
            Finding::new(
                rules::DAX_REF,
                Severity::Error,
                format!("{}: {message} ({position})", site.what),
            )
            .at(origin)
            .on(site.object)
            .suggest(suggestion),
        );
    }
}

fn missing_table(model: &Model, name: &str) -> Option<(String, Option<String>)> {
    model.table(name).is_none().then(|| {
        (
            format!("unknown table `{}`", dax_table_ref(name)),
            suggest::table(model, name),
        )
    })
}
