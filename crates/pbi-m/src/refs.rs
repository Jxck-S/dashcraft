//! Free references of M expressions and renaming them.
//!
//! [`free_names`] lists the identifiers of an expression that are not bound
//! inside it (by let steps, function parameters, `each`, record fields or a
//! `catch` parameter): the queries, parameters, functions and library
//! members it uses. [`rename_query`] rewrites the references to one query
//! at token level, keeping everything else (formatting, comments, other
//! names) byte for byte.
//!
//! Scoping follows the M language: a let step or record field is not in
//! scope inside its own initializer (`let a = a + 1 in a` refers to an outer
//! `a`), except through an inclusive reference `@a`; siblings and nested
//! scopes see it.

use crate::ast::{
    Expr, ExprKind, ListItem, Name, RecordType, Section, TableRowType, TryHandler, TypeExpr,
    TypeKind,
};
use crate::error::ParseError;
use crate::parser::parse_expression;
use crate::printer::print_expr;
use crate::span::Span;

/// Walk depth cap (the parser already bounds trees to `MAX_AST_DEPTH`).
const MAX_DEPTH: usize = 512;

/// What [`free_names`] finds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FreeNames<'e> {
    /// Identifiers not bound inside the expression, in source order
    /// (including dotted library names such as `Table.SelectRows`).
    pub free: Vec<&'e Name>,
    /// Section accesses `Section!Member`, as `(section, member)`.
    pub section_members: Vec<(&'e Name, &'e Name)>,
    /// Every name bound inside the expression (let steps, parameters,
    /// record fields, `_`), e.g. for suggestions.
    pub locals: Vec<&'e str>,
    /// Invocations whose callee is a free identifier (`Text.Upper(x)`), in
    /// source order, e.g. to check argument counts of library functions.
    pub calls: Vec<FreeCall<'e>>,
}

/// An invocation of a free identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreeCall<'e> {
    /// The callee.
    pub name: &'e Name,
    /// The number of arguments passed.
    pub args: usize,
    /// The whole invocation, callee to closing parenthesis.
    pub span: Span,
}

/// The free identifiers of `expr`.
pub fn free_names(expr: &Expr) -> FreeNames<'_> {
    let mut w = Walker::new(None);
    w.expr(expr);
    w.found
}

/// The free identifiers of the members of a section document: every member
/// is in scope in every member (references between queries; a member that
/// refers to itself is a cycle, not an unknown name).
pub fn section_free_names(section: &Section) -> FreeNames<'_> {
    let mut w = Walker::new(None);
    let names = section
        .members
        .iter()
        .map(|m| m.name.text.as_str())
        .collect();
    w.scoped(names, |w| {
        for m in &section.members {
            w.expr(&m.value);
        }
    });
    w.found
}

/// Why [`rename_query`] cannot rewrite an expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameError {
    /// The expression does not parse.
    Parse(ParseError),
    /// A reference to the old name is inside a scope where the new name is
    /// a let step, parameter or record field, which would capture it.
    Captured {
        /// The reference.
        span: Span,
    },
}

/// Rewrites the references to the query, parameter or function `old` in M
/// expression `src` so they name `new` (M names are case-sensitive): free
/// identifiers (`old`, `#"old"`, `@old`) and section members
/// (`Section1!old`). The new name is written bare when it is a regular
/// identifier, else as `#"new"`; nothing else changes. Returns `None` when
/// `src` does not refer to `old`.
///
/// # Errors
/// [`RenameError::Parse`] when `src` does not parse;
/// [`RenameError::Captured`] when a reference sits where `new` is bound
/// locally (a step named `new`), so the rewritten text would mean something
/// else.
pub fn rename_query(src: &str, old: &str, new: &str) -> Result<Option<String>, RenameError> {
    let expr = parse_expression(src).map_err(RenameError::Parse)?;
    let mut w = Walker::new(Some((old, new)));
    w.expr(&expr);
    if let Some(span) = w.captured.first() {
        return Err(RenameError::Captured { span: *span });
    }
    let mut spans: Vec<Span> = w
        .found
        .free
        .iter()
        .filter(|n| n.text == old)
        .map(|n| n.span)
        .chain(
            w.found
                .section_members
                .iter()
                .filter(|(_, m)| m.text == old)
                .map(|(_, m)| m.span),
        )
        .collect();
    if spans.is_empty() {
        return Ok(None);
    }
    spans.sort_by_key(|s| s.start);
    spans.dedup();
    let replacement = identifier(new);
    let mut out = String::with_capacity(src.len() + spans.len() * replacement.len());
    let mut at = 0;
    for span in spans {
        let (Some(before), true) = (src.get(at..span.start), span.end <= src.len()) else {
            continue;
        };
        out.push_str(before);
        out.push_str(&replacement);
        at = span.end;
    }
    out.push_str(src.get(at..).unwrap_or_default());
    Ok(Some(out))
}

/// `name` as an M identifier: bare when it is a regular identifier, else
/// `#"name"` with escapes.
pub fn identifier(name: &str) -> String {
    let expr = Expr::new(
        ExprKind::Identifier {
            name: Name::new(name),
            inclusive: false,
        },
        Span::default(),
    );
    print_expr(&expr)
}

/// The names one let, record, function, `each` or `catch` binds.
struct Scope<'e> {
    names: Vec<&'e str>,
    /// The let step or record field whose initializer is being walked: not
    /// in scope for references other than `@name`.
    excluded: Option<&'e str>,
}

/// Free-identifier collector with lexical scopes.
struct Walker<'e, 'r> {
    scopes: Vec<Scope<'e>>,
    found: FreeNames<'e>,
    depth: usize,
    /// `(old, new)`: report references to `old` where `new` is bound.
    rename: Option<(&'r str, &'r str)>,
    captured: Vec<Span>,
}

impl<'e, 'r> Walker<'e, 'r> {
    fn new(rename: Option<(&'r str, &'r str)>) -> Self {
        Self {
            scopes: Vec::new(),
            found: FreeNames::default(),
            depth: 0,
            rename,
            captured: Vec::new(),
        }
    }

    fn bound(&self, name: &str, inclusive: bool) -> bool {
        self.scopes
            .iter()
            .any(|s| s.names.contains(&name) && (inclusive || s.excluded != Some(name)))
    }

    fn scoped(&mut self, names: Vec<&'e str>, f: impl FnOnce(&mut Self)) {
        self.found.locals.extend(names.iter().copied());
        self.scopes.push(Scope {
            names,
            excluded: None,
        });
        f(self);
        self.scopes.pop();
    }

    /// Walks the initializer of the innermost scope's member `name`.
    fn initializer(&mut self, name: &'e str, value: &'e Expr) {
        if let Some(s) = self.scopes.last_mut() {
            s.excluded = Some(name);
        }
        self.expr(value);
        if let Some(s) = self.scopes.last_mut() {
            s.excluded = None;
        }
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
            ExprKind::Identifier { name, inclusive } => {
                if !self.bound(&name.text, *inclusive) {
                    if let Some((old, new)) = self.rename
                        && name.text == old
                        && self.bound(new, *inclusive)
                    {
                        self.captured.push(name.span);
                    }
                    self.found.free.push(name);
                }
            }
            ExprKind::SectionAccess { section, member } => {
                self.found.section_members.push((section, member));
            }
            ExprKind::Literal(_) | ExprKind::Intrinsic(_) | ExprKind::NotImplemented => {}
            ExprKind::Parenthesized(x)
            | ExprKind::Unary { operand: x, .. }
            | ExprKind::As { expr: x, .. }
            | ExprKind::Is { expr: x, .. }
            | ExprKind::Error(x) => self.expr(x),
            ExprKind::Let { bindings, body } => {
                let names = bindings.iter().map(|b| b.name.text.as_str()).collect();
                self.scoped(names, |w| {
                    for b in bindings {
                        w.initializer(&b.name.text, &b.value);
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
                if let ExprKind::Identifier { name, inclusive } = &function.kind
                    && !self.bound(&name.text, *inclusive)
                {
                    self.found.calls.push(FreeCall {
                        name,
                        args: args.len(),
                        span: e.span,
                    });
                }
                self.expr(function);
                for a in args {
                    self.expr(a);
                }
            }
            ExprKind::Record(fields) => {
                let names = fields.iter().map(|f| f.name.text.as_str()).collect();
                self.scoped(names, |w| {
                    for f in fields {
                        w.initializer(&f.name.text, &f.value);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn free(src: &str) -> Vec<String> {
        let expr = parse_expression(src).unwrap();
        free_names(&expr)
            .free
            .iter()
            .map(|n| n.text.clone())
            .collect()
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

    #[test]
    fn scoping_is_exclusive_in_initializers() {
        // A step or field does not see itself (Desktop: `let x = x in x`
        // names an unknown `x`; recursion needs `@`), siblings do.
        assert_eq!(free("let x = x in x"), ["x"]);
        assert_eq!(
            free("let f = (n) => if n = 0 then 1 else n * f(n - 1) in f(3)"),
            ["f"]
        );
        assert!(free("let f = (n) => if n = 0 then 1 else n * @f(n - 1) in f(3)").is_empty());
        assert!(free("let a = b, b = a in a").is_empty());
        assert!(free("[a = 1, b = a + 1][b]").is_empty());
        assert!(free("let a = 1, r = let a = a + 1 in a in r").is_empty());
        assert_eq!(free("[a = a + 1]"), ["a"]);
        // Nested scopes inside the initializer keep the exclusion; the body
        // and later siblings do not.
        assert_eq!(free("let a = (x) => a + x, b = a in b"), ["a"]);
    }

    #[test]
    fn calls_of_free_names_are_listed() {
        let expr =
            parse_expression("let f = (x) => x, t = Text.Upper(\"a\", 1) in f(List.Sum({}))")
                .unwrap();
        let calls: Vec<_> = free_names(&expr)
            .calls
            .iter()
            .map(|c| (c.name.text.clone(), c.args, c.span))
            .collect();
        assert_eq!(
            calls,
            [
                ("Text.Upper".to_owned(), 2, Span::new(22, 40)),
                ("List.Sum".to_owned(), 1, Span::new(46, 58)),
            ]
        );
        // A step shadowing a library name is local.
        let expr = parse_expression("let Text.Upper = (x) => x in Text.Upper(1, 2)").unwrap();
        assert!(free_names(&expr).calls.is_empty());
    }

    #[test]
    fn section_members_see_each_other() {
        let crate::Document::Section(s) =
            crate::parse_document("section S; A = B + C; B = A; shared C = Missing;").unwrap()
        else {
            panic!("not a section");
        };
        let found = section_free_names(&s);
        let names: Vec<_> = found.free.iter().map(|n| n.text.as_str()).collect();
        assert_eq!(names, ["Missing"]);
    }

    #[test]
    fn rename_rewrites_only_free_references() {
        let src = "let\n    // Orders, the query\n    Source = Table.Combine({Orders, #\"Orders\", @Orders, Section1!Orders}),\n    Kept = Table.SelectRows(Source, each [Orders] <> \"Orders\"),\n    F = (Orders) => Orders + 1,\n    R = [Orders = 1, x = Orders]\nin\n    Kept";
        let out = rename_query(src, "Orders", "Sales Orders")
            .unwrap()
            .unwrap();
        assert_eq!(
            out,
            "let\n    // Orders, the query\n    Source = Table.Combine({#\"Sales Orders\", #\"Sales Orders\", @#\"Sales Orders\", Section1!#\"Sales Orders\"}),\n    Kept = Table.SelectRows(Source, each [Orders] <> \"Orders\"),\n    F = (Orders) => Orders + 1,\n    R = [Orders = 1, x = Orders]\nin\n    Kept"
        );
        // A quoted reference becomes bare when the new name allows it; names
        // are case-sensitive.
        assert_eq!(
            rename_query("#\"Old Url\" & old_url", "Old Url", "Url")
                .unwrap()
                .as_deref(),
            Some("Url & old_url")
        );
        assert_eq!(
            rename_query("x", "x", "each").unwrap().as_deref(),
            Some("#\"each\"")
        );
        assert_eq!(rename_query("ORDERS + 1", "Orders", "Sales"), Ok(None));
        assert_eq!(
            rename_query("let Orders = 1 in Orders", "Orders", "S"),
            Ok(None)
        );
        // The initializer of a step named like the query refers to the query.
        assert_eq!(
            rename_query("let Orders = Orders in Orders", "Orders", "Sales")
                .unwrap()
                .as_deref(),
            Some("let Orders = Sales in Orders")
        );
    }

    #[test]
    fn rename_refuses_captures_and_unparsable_text() {
        let src = "let Source = AzureStorage.Blobs(Url), Next = Url in Next";
        assert_eq!(
            rename_query(src, "Url", "Source"),
            Err(RenameError::Captured {
                span: Span::new(45, 48)
            })
        );
        // Inside its own initializer the step is not in scope, so the new
        // name still refers to the query.
        assert_eq!(
            rename_query(
                "let Source = AzureStorage.Blobs(Url) in Source",
                "Url",
                "Source"
            )
            .unwrap()
            .as_deref(),
            Some("let Source = AzureStorage.Blobs(Source) in Source")
        );
        assert!(rename_query("Url & (each Url)(1)", "Url", "_").is_err());
        // Not captured where the new name is not in scope.
        assert_eq!(
            rename_query("let f = (Source) => Source in f(Url)", "Url", "Source")
                .unwrap()
                .as_deref(),
            Some("let f = (Source) => Source in f(Source)")
        );
        assert!(matches!(
            rename_query("let x = in x", "x", "y"),
            Err(RenameError::Parse(_))
        ));
    }
}
