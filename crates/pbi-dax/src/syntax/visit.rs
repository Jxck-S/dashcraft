//! AST traversal and reference extraction.
//!
//! [`references`] / [`referenced_tables`] feed the broken-reference linter and
//! the refresh dependency planner (which calculated columns / tables depend on
//! which tables).

use std::collections::HashSet;

use super::ast::{Expr, ExprKind, Span};

/// Recursive visitor. Override [`Visitor::visit_expr`] and call
/// [`walk_expr`] to continue into children. Recursion depth is bounded by
/// [`MAX_DEPTH`](super::parser::MAX_DEPTH) for parsed trees; prefer
/// [`for_each_expr`] when a pre-order callback suffices.
pub trait Visitor {
    fn visit_expr(&mut self, expr: &Expr) {
        walk_expr(self, expr);
    }
}

/// Visits each direct child of `expr` in source order.
pub fn walk_expr<V: Visitor + ?Sized>(visitor: &mut V, expr: &Expr) {
    for child in expr.children() {
        visitor.visit_expr(child);
    }
}

/// Calls `f` on every node in pre-order (parents before children, children
/// left to right). Iterative: safe on arbitrarily deep trees.
pub fn for_each_expr<'a>(root: &'a Expr, mut f: impl FnMut(&'a Expr)) {
    let mut stack = vec![root];
    while let Some(e) = stack.pop() {
        f(e);
        let children = e.children();
        stack.extend(children.into_iter().rev());
    }
}

/// A model object (or function) referenced by an expression.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Reference {
    /// Explicit table reference: `Orders`, `'Web Sessions'`. Tables that
    /// only appear as column qualifiers are reported as [`Reference::Column`]
    /// (see [`referenced_tables`] for the union).
    Table(String),
    /// Qualified column reference `Table[Column]`.
    Column { table: String, column: String },
    /// Unqualified `[Name]`: a measure, or a column of the row-context table
    /// (e.g. in a calculated column or iterator) — resolve against the model.
    Measure(String),
    /// Function call, upper-cased name (`CALCULATE`, `PERCENTILE.INC`, or a
    /// user-defined function).
    Function(String),
}

impl Reference {
    fn key(&self) -> (u8, String, String) {
        match self {
            Reference::Table(t) => (0, t.to_lowercase(), String::new()),
            Reference::Column { table, column } => (1, table.to_lowercase(), column.to_lowercase()),
            Reference::Measure(m) => (2, m.to_lowercase(), String::new()),
            Reference::Function(f) => (3, f.to_uppercase(), String::new()),
        }
    }
}

/// Every reference occurrence with its span, in source order (not deduplicated).
/// Variable references, query parameters and enum keywords are excluded.
pub fn reference_sites(expr: &Expr) -> Vec<(Reference, Span)> {
    let mut out = Vec::new();
    for_each_expr(expr, |e| {
        let reference = match &e.kind {
            ExprKind::TableRef { name } => Reference::Table(name.clone()),
            ExprKind::ColumnRef {
                table: Some(table),
                column,
            } => Reference::Column {
                table: table.clone(),
                column: column.clone(),
            },
            ExprKind::ColumnRef {
                table: None,
                column,
            } => Reference::Measure(column.clone()),
            ExprKind::Call { name, .. } => Reference::Function(name.clone()),
            _ => return,
        };
        out.push((reference, e.span));
    });
    out
}

/// Distinct references in order of first appearance. DAX names are
/// case-insensitive, so duplicates differing only in case are merged (the
/// first spelling wins).
pub fn references(expr: &Expr) -> Vec<Reference> {
    let mut seen = HashSet::new();
    reference_sites(expr)
        .into_iter()
        .map(|(r, _)| r)
        .filter(|r| seen.insert(r.key()))
        .collect()
}

/// Distinct table names the expression depends on — explicit table
/// references plus the tables qualifying column references — in order of
/// first appearance (case-insensitive dedup).
pub fn referenced_tables(expr: &Expr) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (r, _) in reference_sites(expr) {
        let table = match r {
            Reference::Table(t) | Reference::Column { table: t, .. } => t,
            Reference::Measure(_) | Reference::Function(_) => continue,
        };
        if seen.insert(table.to_lowercase()) {
            out.push(table);
        }
    }
    out
}
