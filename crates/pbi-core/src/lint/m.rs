//! M reference checks (LINT013): identifiers in partition sources and
//! shared expressions that are not let steps, function parameters, record
//! fields, shared expressions/parameters, other queries (tables) or library
//! names.
//!
//! Heuristic: dotted names (`Table.SelectRows`, `Int64.Type`) are library
//! members and never flagged.

use std::collections::HashSet;

use pbi_m::ast::{
    Expr, ExprKind, ListItem, RecordType, TableRowType, TryHandler, TypeExpr, TypeKind,
};
use pbi_m::{Span, library_names, line_col, parse_expression};
use pbi_model::{Model, Severity, names_eq};

use super::{ExprSite, Finding, Sink, rules, suggest};

/// Walk depth cap (the parser already bounds trees to `MAX_AST_DEPTH`).
const MAX_DEPTH: usize = 512;

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

/// Free-identifier collector with lexical scopes.
#[derive(Default)]
struct Walker<'e> {
    scopes: Vec<Vec<&'e str>>,
    /// Every locally bound name (for suggestions).
    locals: Vec<&'e str>,
    free: Vec<(&'e str, Span)>,
    depth: usize,
}

impl<'e> Walker<'e> {
    fn bound(&self, name: &str) -> bool {
        self.scopes.iter().any(|s| s.contains(&name))
    }

    fn scoped(&mut self, names: Vec<&'e str>, f: impl FnOnce(&mut Self)) {
        self.locals.extend(names.iter().copied());
        self.scopes.push(names);
        f(self);
        self.scopes.pop();
    }

    fn expr(&mut self, e: &'e Expr) {
        if self.depth >= MAX_DEPTH {
            return;
        }
        self.depth += 1;
        self.expr_inner(e);
        self.depth -= 1;
    }

    fn expr_inner(&mut self, e: &'e Expr) {
        match &e.kind {
            ExprKind::Identifier { name, .. } => {
                if !self.bound(&name.text) {
                    self.free.push((&name.text, name.span));
                }
            }
            ExprKind::Literal(_)
            | ExprKind::Intrinsic(_)
            | ExprKind::SectionAccess { .. }
            | ExprKind::NotImplemented => {}
            ExprKind::Parenthesized(x)
            | ExprKind::Unary { operand: x, .. }
            | ExprKind::As { expr: x, .. }
            | ExprKind::Is { expr: x, .. }
            | ExprKind::Error(x) => self.expr(x),
            ExprKind::Let { bindings, body } => {
                let names = bindings.iter().map(|b| b.name.text.as_str()).collect();
                self.scoped(names, |w| {
                    for b in bindings {
                        w.expr(&b.value);
                    }
                    w.expr(body);
                });
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.expr(condition);
                self.expr(then_branch);
                self.expr(else_branch);
            }
            ExprKind::Each(body) => self.scoped(vec!["_"], |w| w.expr(body)),
            ExprKind::Function { params, body, .. } => {
                let names = params.iter().map(|p| p.name.text.as_str()).collect();
                self.scoped(names, |w| w.expr(body));
            }
            ExprKind::Invoke { function, args } => {
                self.expr(function);
                for a in args {
                    self.expr(a);
                }
            }
            ExprKind::Record(fields) => {
                let names = fields.iter().map(|f| f.name.text.as_str()).collect();
                self.scoped(names, |w| {
                    for f in fields {
                        w.expr(&f.value);
                    }
                });
            }
            ExprKind::List(items) => {
                for item in items {
                    match item {
                        ListItem::Single(x) => self.expr(x),
                        ListItem::Range { start, end } => {
                            self.expr(start);
                            self.expr(end);
                        }
                    }
                }
            }
            ExprKind::FieldAccess { target, .. } | ExprKind::FieldProjection { target, .. } => {
                if let Some(t) = target {
                    self.expr(t);
                }
            }
            ExprKind::ItemAccess {
                target, selector, ..
            } => {
                self.expr(target);
                self.expr(selector);
            }
            ExprKind::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            ExprKind::Type(t) => self.ty(t),
            ExprKind::Try { expr, handler } => {
                self.expr(expr);
                match handler {
                    Some(TryHandler::Otherwise(x)) => self.expr(x),
                    Some(TryHandler::Catch { param, body }) => {
                        let names = param.iter().map(|p| p.text.as_str()).collect();
                        self.scoped(names, |w| w.expr(body));
                    }
                    None => {}
                }
            }
        }
    }

    fn ty(&mut self, t: &'e TypeExpr) {
        if self.depth >= MAX_DEPTH {
            return;
        }
        self.depth += 1;
        match &t.kind {
            TypeKind::Primitive(_) => {}
            TypeKind::Nullable(x) | TypeKind::List(x) => self.ty(x),
            TypeKind::Record(r) | TypeKind::Table(TableRowType::Fields(r)) => self.record_type(r),
            TypeKind::Function(f) => {
                for p in &f.params {
                    if let Some(x) = &p.ty {
                        self.ty(x);
                    }
                }
                self.ty(&f.return_type);
            }
            TypeKind::Table(TableRowType::Expr(x)) | TypeKind::Expr(x) => self.expr(x),
        }
        self.depth -= 1;
    }

    fn record_type(&mut self, r: &'e RecordType) {
        for f in &r.fields {
            if let Some(x) = &f.ty {
                self.ty(x);
            }
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
    let mut w = Walker::default();
    w.expr(&expr);
    let mut reported: HashSet<&str> = HashSet::new();
    for (name, span) in &w.free {
        if name.contains('.')
            || globals.queries.contains(name)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn free(src: &str) -> Vec<String> {
        let expr = parse_expression(src).unwrap();
        let mut w = Walker::default();
        w.expr(&expr);
        w.free.iter().map(|(n, _)| (*n).to_owned()).collect()
    }

    #[test]
    fn let_steps_params_and_fields_are_bound() {
        let src = r#"let
            Source = AzureStorage.Blobs(StorageContainerUrl),
            F = (x as text) => Text.Upper(x) & Helper,
            R = [a = 1, b = a + 1],
            S = Table.SelectRows(Source, each [x] = _[y] and Missing),
            T = try Source otherwise Other,
            U = try Source catch (e) => e[Message]
        in
            Renamd"#;
        assert_eq!(
            free(src),
            [
                "AzureStorage.Blobs",
                "StorageContainerUrl",
                "Text.Upper",
                "Helper",
                "Table.SelectRows",
                "Missing",
                "Other",
                "Renamd"
            ]
        );
        assert_eq!(
            free("#table(type table [a = Int64.Type], {})"),
            ["Int64.Type"]
        );
    }
}
