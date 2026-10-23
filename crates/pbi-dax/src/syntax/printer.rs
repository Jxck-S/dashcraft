//! DAX pretty-printer.
//!
//! [`print_expr`] renders a canonical single-line form; [`pretty_expr`]
//! breaks long calls, VAR blocks, table constructors and operator chains
//! over multiple lines (DAX Formatter style). Both produce valid DAX such
//! that re-parsing yields the same AST (modulo spans) for any tree produced
//! by the parser. Parentheses are only added where precedence requires them
//! (explicit [`ExprKind::Paren`] nodes are always kept). Comments are not
//! preserved.

use super::ast::{
    BinaryOp, Define, Expr, ExprKind, FunctionParam, Number, Query, SortDir, UnaryOp, VarBinding,
};
use super::lexer::{Keyword, is_enum_keyword, is_ident_continue, is_ident_start};

/// Layout options for [`pretty_expr`] / [`pretty_query`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintOptions {
    /// One indentation level.
    pub indent: String,
    /// Preferred maximum line width (in chars) before breaking.
    pub max_width: usize,
}

impl Default for PrintOptions {
    fn default() -> Self {
        Self {
            indent: "    ".to_string(),
            max_width: 80,
        }
    }
}

/// Single-line canonical rendering of an expression.
pub fn print_expr(expr: &Expr) -> String {
    let mut out = String::new();
    Printer::new(None).flat(expr, &mut out);
    out
}

/// Multi-line rendering of an expression.
pub fn pretty_expr(expr: &Expr, opts: &PrintOptions) -> String {
    let mut out = String::new();
    Printer::new(Some(opts)).pretty(expr, 0, &mut out);
    out
}

/// Renders a query with one definition / clause per line and single-line
/// expressions.
pub fn print_query(query: &Query) -> String {
    Printer::new(None).query(query)
}

/// Renders a query with [`pretty_expr`]-style expressions.
pub fn pretty_query(query: &Query, opts: &PrintOptions) -> String {
    Printer::new(Some(opts)).query(query)
}

const DEFAULT_INDENT: &str = "    ";

/// All output is appended to a shared buffer and the recursive functions
/// keep few locals, so printing a tree at the parser's depth limit stays
/// well within a 2 MiB stack even in unoptimised builds.
struct Printer<'o> {
    opts: Option<&'o PrintOptions>,
    /// Lower-cased variable names in scope (a bare table name that collides
    /// with one must be quoted to stay a table reference).
    scopes: Vec<Vec<String>>,
}

impl<'o> Printer<'o> {
    fn new(opts: Option<&'o PrintOptions>) -> Self {
        Self {
            opts,
            scopes: Vec::new(),
        }
    }

    fn declare(&mut self, name: &str) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.push(name.to_lowercase());
        }
    }

    fn is_var(&self, name: &str) -> bool {
        let lower = name.to_lowercase();
        self.scopes.iter().any(|s| s.contains(&lower))
    }

    fn max_width(&self) -> usize {
        self.opts.map_or(usize::MAX, |o| o.max_width)
    }

    fn newline(&self, level: usize, out: &mut String) {
        let unit = self.opts.map_or(DEFAULT_INDENT, |o| o.indent.as_str());
        out.push('\n');
        for _ in 0..level {
            out.push_str(unit);
        }
    }

    fn table_name(&self, name: &str, standalone: bool) -> String {
        if needs_quote(name) || (standalone && self.is_var(name)) {
            quote_name(name)
        } else {
            name.to_string()
        }
    }

    // ---- flat -------------------------------------------------------------

    fn flat(&mut self, e: &Expr, out: &mut String) {
        match &e.kind {
            ExprKind::Call {
                name,
                raw_name,
                args,
            } => {
                out.push_str(call_name(name, raw_name));
                self.flat_list(args, out);
            }
            ExprKind::Var { bindings, body } => self.flat_var(bindings, body, out),
            ExprKind::Unary { op, expr } => self.flat_unary(*op, expr, out),
            ExprKind::Binary { .. } => self.flat_binary(e, out),
            ExprKind::TableConstructor { rows } => self.flat_table(rows, out),
            ExprKind::Tuple(items) => self.flat_list(items, out),
            ExprKind::Paren(inner) => self.flat_child(inner, true, out),
            _ => self.leaf(e, out),
        }
    }

    /// Non-recursive nodes.
    #[inline(never)]
    fn leaf(&self, e: &Expr, out: &mut String) {
        match &e.kind {
            ExprKind::Number(n) => out.push_str(&format_number(*n)),
            ExprKind::String(s) => out.push_str(&quote_string(s)),
            ExprKind::DateTime(s) => {
                out.push_str("dt");
                out.push_str(&quote_string(s));
            }
            ExprKind::Boolean(b) => out.push_str(if *b { "TRUE" } else { "FALSE" }),
            ExprKind::TableRef { name } => out.push_str(&self.table_name(name, true)),
            ExprKind::ColumnRef { table, column } => {
                if let Some(table) = table {
                    out.push_str(&self.table_name(table, false));
                }
                out.push_str(&bracket(column));
            }
            ExprKind::VarRef(name) | ExprKind::Keyword(name) => out.push_str(name),
            ExprKind::Parameter(name) => {
                out.push('@');
                out.push_str(name);
            }
            ExprKind::Omitted
            | ExprKind::Call { .. }
            | ExprKind::Var { .. }
            | ExprKind::Unary { .. }
            | ExprKind::Binary { .. }
            | ExprKind::TableConstructor { .. }
            | ExprKind::Tuple(_)
            | ExprKind::Paren(_) => {}
        }
    }

    /// `(a, b, c)`
    fn flat_list(&mut self, items: &[Expr], out: &mut String) {
        out.push('(');
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            self.flat(item, out);
        }
        out.push(')');
    }

    fn flat_var(&mut self, bindings: &[VarBinding], body: &Expr, out: &mut String) {
        self.scopes.push(Vec::new());
        for b in bindings {
            out.push_str("VAR ");
            out.push_str(&b.name);
            out.push_str(" = ");
            self.flat(&b.expr, out);
            out.push(' ');
            self.declare(&b.name);
        }
        out.push_str("RETURN ");
        self.flat(body, out);
        self.scopes.pop();
    }

    fn flat_unary(&mut self, op: UnaryOp, operand: &Expr, out: &mut String) {
        let paren = operand_needs_parens(op.operand_precedence(), operand);
        let mark = push_unary_op(op, out);
        self.flat_child(operand, paren, out);
        fix_double_minus(op, mark, out);
    }

    fn flat_binary(&mut self, e: &Expr, out: &mut String) {
        let (p, first, rest) = left_chain(e);
        self.flat_child(first, lhs_needs_parens(p, first), out);
        for (op, rhs) in rest {
            out.push(' ');
            out.push_str(op.symbol());
            out.push(' ');
            self.flat_child(rhs, rhs_needs_parens(p, rhs), out);
        }
    }

    fn flat_child(&mut self, e: &Expr, paren: bool, out: &mut String) {
        if paren {
            out.push('(');
        }
        self.flat(e, out);
        if paren {
            out.push(')');
        }
    }

    fn flat_table(&mut self, rows: &[Vec<Expr>], out: &mut String) {
        out.push('{');
        for (i, row) in rows.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            match row.as_slice() {
                [single] => self.flat(single, out),
                _ => self.flat_list(row, out),
            }
        }
        out.push('}');
    }

    /// Flat rendering of `e` (optionally parenthesised) if it has no VAR
    /// block and fits on the current line after `extra` more chars.
    #[inline(never)]
    fn flat_if_fits(&mut self, e: &Expr, paren: bool, extra: usize, out: &str) -> Option<String> {
        if contains_var(e) {
            return None;
        }
        let mut flat = String::new();
        self.flat_child(e, paren, &mut flat);
        (current_col(out) + extra + width(&flat) <= self.max_width()).then_some(flat)
    }

    // ---- pretty -----------------------------------------------------------

    /// Appends `e` at the current position; continuation lines are indented
    /// `indent` levels.
    fn pretty(&mut self, e: &Expr, indent: usize, out: &mut String) {
        if let Some(flat) = self.flat_if_fits(e, false, 0, out) {
            out.push_str(&flat);
            return;
        }
        match &e.kind {
            ExprKind::Call {
                name,
                raw_name,
                args,
            } if !args.is_empty() => {
                out.push_str(call_name(name, raw_name));
                self.pretty_list(args, indent, out);
            }
            ExprKind::Var { bindings, body } => self.pretty_var(bindings, body, indent, out),
            ExprKind::Binary { .. } => self.pretty_binary(e, indent, out),
            ExprKind::Unary { op, expr } => self.pretty_unary(*op, expr, indent, out),
            ExprKind::Paren(inner) => {
                out.push('(');
                self.newline(indent + 1, out);
                self.pretty(inner, indent + 1, out);
                self.newline(indent, out);
                out.push(')');
            }
            ExprKind::Tuple(items) => self.pretty_list(items, indent, out),
            ExprKind::TableConstructor { rows } if !rows.is_empty() => {
                self.pretty_table(rows, indent, out);
            }
            _ => self.flat(e, out),
        }
    }

    /// `(` newline, one item per line at `indent + 1`, newline `)`.
    fn pretty_list(&mut self, items: &[Expr], indent: usize, out: &mut String) {
        out.push('(');
        for (i, item) in items.iter().enumerate() {
            self.newline(indent + 1, out);
            self.pretty(item, indent + 1, out);
            if i + 1 < items.len() {
                out.push(',');
            }
        }
        self.newline(indent, out);
        out.push(')');
    }

    fn pretty_var(
        &mut self,
        bindings: &[VarBinding],
        body: &Expr,
        indent: usize,
        out: &mut String,
    ) {
        self.scopes.push(Vec::new());
        for (i, b) in bindings.iter().enumerate() {
            if i > 0 {
                self.newline(indent, out);
            }
            out.push_str("VAR ");
            out.push_str(&b.name);
            self.assignment_value(" =", &b.expr, indent, out);
            self.declare(&b.name);
        }
        if !bindings.is_empty() {
            self.newline(indent, out);
        }
        out.push_str("RETURN");
        self.newline(indent + 1, out);
        self.pretty(body, indent + 1, out);
        self.scopes.pop();
    }

    fn pretty_binary(&mut self, e: &Expr, indent: usize, out: &mut String) {
        let (p, first, rest) = left_chain(e);
        self.pretty_child(first, lhs_needs_parens(p, first), indent, out);
        for (op, rhs) in rest {
            let sym = op.symbol();
            let paren = rhs_needs_parens(p, rhs);
            if let Some(flat) = self.flat_if_fits(rhs, paren, sym.len() + 2, out) {
                out.push(' ');
                out.push_str(sym);
                out.push(' ');
                out.push_str(&flat);
            } else if is_block(rhs) && !paren {
                out.push(' ');
                out.push_str(sym);
                out.push(' ');
                self.pretty(rhs, indent, out);
            } else {
                self.newline(indent, out);
                out.push_str(sym);
                out.push(' ');
                self.pretty_child(rhs, paren, indent, out);
            }
        }
    }

    fn pretty_unary(&mut self, op: UnaryOp, operand: &Expr, indent: usize, out: &mut String) {
        let paren = operand_needs_parens(op.operand_precedence(), operand);
        let mark = push_unary_op(op, out);
        self.pretty_child(operand, paren, indent, out);
        fix_double_minus(op, mark, out);
    }

    fn pretty_child(&mut self, e: &Expr, paren: bool, indent: usize, out: &mut String) {
        if paren {
            out.push('(');
        }
        self.pretty(e, indent, out);
        if paren {
            out.push(')');
        }
    }

    fn pretty_table(&mut self, rows: &[Vec<Expr>], indent: usize, out: &mut String) {
        out.push('{');
        for (i, row) in rows.iter().enumerate() {
            self.newline(indent + 1, out);
            match row.as_slice() {
                [single] => self.pretty(single, indent + 1, out),
                _ => self.pretty_row(row, indent + 1, out),
            }
            if i + 1 < rows.len() {
                out.push(',');
            }
        }
        self.newline(indent, out);
        out.push('}');
    }

    fn pretty_row(&mut self, row: &[Expr], indent: usize, out: &mut String) {
        if !row.iter().any(contains_var) {
            let mut flat = String::new();
            self.flat_list(row, &mut flat);
            if current_col(out) + width(&flat) <= self.max_width() {
                out.push_str(&flat);
                return;
            }
        }
        self.pretty_list(row, indent, out);
    }

    /// Appends `{sep} value` after an already-written head (`VAR x`,
    /// `MEASURE T[m]`, …), moving the value to its own indented line when it
    /// does not fit.
    fn assignment_value(&mut self, sep: &str, expr: &Expr, indent: usize, out: &mut String) {
        out.push_str(sep);
        if self.opts.is_none() {
            out.push(' ');
            self.flat(expr, out);
        } else if let Some(flat) = self.flat_if_fits(expr, false, 1, out) {
            out.push(' ');
            out.push_str(&flat);
        } else {
            self.newline(indent + 1, out);
            self.pretty(expr, indent + 1, out);
        }
    }

    // ---- queries ------------------------------------------------------------

    fn query(&mut self, q: &Query) -> String {
        let mut out = String::new();
        self.scopes.push(Vec::new());
        if !q.defines.is_empty() {
            out.push_str("DEFINE");
            for d in &q.defines {
                self.newline(1, &mut out);
                self.define(d, &mut out);
            }
            out.push('\n');
        }
        for (i, ev) in q.evaluates.iter().enumerate() {
            if i > 0 || !q.defines.is_empty() {
                out.push('\n');
            }
            out.push_str("EVALUATE");
            self.newline(1, &mut out);
            if self.opts.is_some() {
                self.pretty(&ev.expr, 1, &mut out);
            } else {
                self.flat(&ev.expr, &mut out);
            }
            for (j, (key, dir)) in ev.order_by.iter().enumerate() {
                out.push_str(if j == 0 { "\nORDER BY " } else { ", " });
                self.flat(key, &mut out);
                if *dir == SortDir::Desc {
                    out.push_str(" DESC");
                }
            }
            for (j, value) in ev.start_at.iter().enumerate() {
                out.push_str(if j == 0 { "\nSTART AT " } else { ", " });
                self.flat(value, &mut out);
            }
            out.push('\n');
        }
        self.scopes.pop();
        out
    }

    fn define(&mut self, d: &Define, out: &mut String) {
        match d {
            Define::Measure {
                table, name, expr, ..
            }
            | Define::Column {
                table, name, expr, ..
            } => {
                out.push_str(if matches!(d, Define::Measure { .. }) {
                    "MEASURE "
                } else {
                    "COLUMN "
                });
                out.push_str(&self.table_name(table, false));
                out.push_str(&bracket(name));
                self.assignment_value(" =", expr, 1, out);
            }
            Define::Table { name, expr, .. } => {
                out.push_str("TABLE ");
                out.push_str(&self.table_name(name, false));
                self.assignment_value(" =", expr, 1, out);
            }
            Define::Var { name, expr, .. } => {
                out.push_str("VAR ");
                out.push_str(name);
                self.assignment_value(" =", expr, 1, out);
                self.declare(name);
            }
            Define::Function {
                name, params, body, ..
            } => {
                out.push_str("FUNCTION ");
                if function_name_needs_quote(name) {
                    out.push_str(&quote_name(name));
                } else {
                    out.push_str(name);
                }
                out.push_str(" = (");
                out.push_str(&format_params(params));
                out.push(')');
                self.scopes
                    .push(params.iter().map(|p| p.name.to_lowercase()).collect());
                self.assignment_value(" =>", body, 1, out);
                self.scopes.pop();
            }
        }
    }
}

fn format_params(params: &[FunctionParam]) -> String {
    params
        .iter()
        .map(|p| match &p.type_hint {
            Some(t) => format!("{} : {t}", p.name),
            None => p.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Prefer the source spelling when it still matches the normalised name.
fn call_name<'a>(name: &'a str, raw_name: &'a str) -> &'a str {
    if !raw_name.is_empty() && raw_name.to_uppercase() == name {
        raw_name
    } else {
        name
    }
}

/// Writes a prefix operator; returns the offset where its operand starts.
fn push_unary_op(op: UnaryOp, out: &mut String) -> usize {
    out.push_str(op.symbol());
    if op == UnaryOp::Not {
        out.push(' ');
    }
    out.len()
}

/// `--` would start a line comment: separate `- -x`.
fn fix_double_minus(op: UnaryOp, operand_start: usize, out: &mut String) {
    if op == UnaryOp::Minus && out.get(operand_start..).is_some_and(|s| s.starts_with('-')) {
        out.insert(operand_start, ' ');
    }
}

/// Lowest binary precedence that a trailing operator would need in order to
/// be absorbed into `e`'s right edge (e.g. `1 + 2` absorbs `*`; `-x`
/// absorbs `^`; a VAR block absorbs everything).
fn right_threshold(e: &Expr) -> u8 {
    let mut t = u8::MAX;
    let mut cur = e;
    loop {
        match &cur.kind {
            ExprKind::Binary { op, rhs, .. } => {
                let p = op.precedence();
                t = t.min(p + 1);
                if rhs_needs_parens(p, rhs) {
                    break;
                }
                cur = rhs;
            }
            ExprKind::Unary { op, expr } => {
                let m = op.operand_precedence();
                t = t.min(m);
                if operand_needs_parens(m, expr) {
                    break;
                }
                cur = expr;
            }
            ExprKind::Var { .. } => return 0,
            _ => break,
        }
    }
    t
}

/// Splits a left-associative chain of same-precedence operators
/// (`a + b - c + d`) into the chain precedence, its first operand and the
/// `(operator, operand)` pairs that follow — without recursion.
fn left_chain(e: &Expr) -> (u8, &Expr, Vec<(BinaryOp, &Expr)>) {
    let mut rest = Vec::new();
    let mut cur = e;
    let mut p = u8::MAX;
    if let ExprKind::Binary { op, .. } = &e.kind {
        p = op.precedence();
        while let ExprKind::Binary { op, lhs, rhs } = &cur.kind {
            if op.precedence() != p {
                break;
            }
            rest.push((*op, rhs.as_ref()));
            cur = lhs;
            if lhs_needs_parens(p, lhs) {
                break;
            }
        }
    }
    rest.reverse();
    (p, cur, rest)
}

fn lhs_needs_parens(op_prec: u8, lhs: &Expr) -> bool {
    op_prec >= right_threshold(lhs)
}

fn rhs_needs_parens(op_prec: u8, rhs: &Expr) -> bool {
    matches!(&rhs.kind, ExprKind::Binary { op, .. } if op.precedence() <= op_prec)
}

fn operand_needs_parens(min_prec: u8, operand: &Expr) -> bool {
    matches!(&operand.kind, ExprKind::Binary { op, .. } if op.precedence() < min_prec)
}

/// Nodes that render as a bracketed block and can break internally while
/// staying on the current line.
fn is_block(e: &Expr) -> bool {
    matches!(
        e.kind,
        ExprKind::Call { .. } | ExprKind::TableConstructor { .. } | ExprKind::Paren(_)
    )
}

fn contains_var(e: &Expr) -> bool {
    let mut stack = vec![e];
    while let Some(e) = stack.pop() {
        if matches!(e.kind, ExprKind::Var { .. }) {
            return true;
        }
        stack.extend(e.children());
    }
    false
}

fn width(s: &str) -> usize {
    s.chars().count()
}

/// Column (in chars) at the end of `out`.
fn current_col(out: &str) -> usize {
    width(
        out.rfind('\n')
            .and_then(|i| out.get(i + 1..))
            .unwrap_or(out),
    )
}

fn format_number(n: Number) -> String {
    match n {
        Number::Integer(i) => i.to_string(),
        Number::Decimal(d) if d.is_nan() => "(0.0 / 0.0)".to_string(),
        Number::Decimal(d) if d.is_infinite() => {
            if d > 0.0 { "1e999" } else { "-1e999" }.to_string()
        }
        // `{:?}` is the shortest representation that round-trips and always
        // keeps a `.` or exponent, so the literal stays a decimal.
        Number::Decimal(d) => format!("{d:?}"),
    }
}

fn quote_string(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

fn quote_name(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn bracket(s: &str) -> String {
    format!("[{}]", s.replace(']', "]]"))
}

/// Whether a table name must be written `'quoted'`.
fn needs_quote(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return true;
    };
    !is_ident_start(first)
        || !chars.all(|c| c == '_' || c.is_alphanumeric())
        || Keyword::from_ident(name).is_some()
        || is_enum_keyword(name)
}

fn function_name_needs_quote(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return true;
    };
    !is_ident_start(first) || !chars.all(is_ident_continue) || Keyword::from_ident(name).is_some()
}
