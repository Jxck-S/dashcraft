//! Pretty-printer: renders an AST back to valid M source.
//!
//! The output is not byte-identical to the original source (comments are dropped, whitespace and
//! text escapes are normalized) but re-parsing it yields the same tree modulo spans:
//! `parse(print(parse(x))) == parse(x)`. Parentheses present in the source are kept (as
//! [`ExprKind::Parenthesized`] nodes); additional parentheses are inserted only where a
//! programmatically built tree would otherwise print ambiguously.

use std::fmt;

use crate::ast::{
    BinaryOp, Binding, Document, Expr, ExprKind, ListItem, Literal, Name, NullablePrimitiveType,
    Param, RecordField, RecordType, TableRowType, TryHandler, TypeExpr, TypeKind, UnaryOp,
    precedence,
};
use crate::lexer::{TokenKind, is_generalized_identifier, is_regular_identifier, tokenize};

const INDENT: &str = "    ";

/// Prints an expression as M source.
pub fn print_expr(expr: &Expr) -> String {
    let mut p = Printer::default();
    p.expr(expr, 0);
    p.out
}

/// Prints a type as it would appear after the `type` keyword.
pub fn print_type(ty: &TypeExpr) -> String {
    let mut p = Printer::default();
    p.ty(ty);
    p.out
}

/// Prints a document (expression or section document) as M source.
pub fn print_document(doc: &Document) -> String {
    let mut p = Printer::default();
    match doc {
        Document::Expression(e) => p.expr(e, 0),
        Document::Section(section) => {
            if let Some(attrs) = &section.attributes {
                p.expr(attrs, 0);
                p.out.push(' ');
            }
            p.out.push_str("section ");
            p.ident(&section.name);
            p.out.push_str(";\n");
            for member in &section.members {
                p.out.push('\n');
                if let Some(attrs) = &member.attributes {
                    p.expr(attrs, 0);
                    p.out.push(' ');
                }
                if member.shared {
                    p.out.push_str("shared ");
                }
                p.ident(&member.name);
                p.out.push_str(" = ");
                p.expr(&member.value, 0);
                p.out.push_str(";\n");
            }
        }
    }
    p.out
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&print_expr(self))
    }
}

impl fmt::Display for TypeExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&print_type(self))
    }
}

impl fmt::Display for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&print_document(self))
    }
}

/// Precedence of the construct `e` prints as (see [`precedence`]); `0` for the expression-level
/// forms (`let`, `if`, `each`, functions, `try`, `error`) that must be parenthesized as operands.
fn expr_precedence(e: &Expr) -> u8 {
    match &e.kind {
        ExprKind::Let { .. }
        | ExprKind::If { .. }
        | ExprKind::Each(_)
        | ExprKind::Function { .. }
        | ExprKind::Try { .. }
        | ExprKind::Error(_) => 0,
        ExprKind::Binary { op, .. } => op.precedence(),
        ExprKind::As { .. } => precedence::AS,
        ExprKind::Is { .. } => precedence::IS,
        ExprKind::Unary { .. } => precedence::UNARY,
        ExprKind::Type(t) if !matches!(t.kind, TypeKind::Expr(_)) => precedence::UNARY,
        _ => precedence::PRIMARY,
    }
}

/// Whether `e` ends in a `try` without handler, which would capture a following
/// `otherwise`/`catch` clause.
fn ends_with_open_try(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Try { handler: None, .. } => true,
        ExprKind::Try {
            handler: Some(TryHandler::Otherwise(next)),
            ..
        }
        | ExprKind::Try {
            handler: Some(TryHandler::Catch { body: next, .. }),
            ..
        }
        | ExprKind::Let { body: next, .. }
        | ExprKind::Each(next)
        | ExprKind::Function { body: next, .. }
        | ExprKind::Error(next)
        | ExprKind::If {
            else_branch: next, ..
        } => ends_with_open_try(next),
        _ => false,
    }
}

/// Whether `e` prints starting with an identifier, `@` or `(` (required after `table` in a
/// table type).
fn starts_with_identifier_or_paren(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Identifier { .. }
        | ExprKind::SectionAccess { .. }
        | ExprKind::Parenthesized(_) => true,
        ExprKind::Invoke { function: t, .. }
        | ExprKind::ItemAccess { target: t, .. }
        | ExprKind::FieldAccess {
            target: Some(t), ..
        }
        | ExprKind::FieldProjection {
            target: Some(t), ..
        } => starts_with_identifier_or_paren(t),
        _ => false,
    }
}

/// Whether a record-type field name would be misread as `optional` + name.
fn starts_with_optional_word(text: &str) -> bool {
    tokenize(text).is_ok_and(|lexed| {
        lexed.tokens.len() > 2
            && matches!(&lexed.tokens[0].kind, TokenKind::Identifier(s) if s == "optional")
    })
}

#[derive(Default)]
struct Printer {
    out: String,
    indent: usize,
}

impl Printer {
    fn newline(&mut self) {
        self.out.push('\n');
        for _ in 0..self.indent {
            self.out.push_str(INDENT);
        }
    }

    fn expr(&mut self, e: &Expr, min_precedence: u8) {
        if expr_precedence(e) < min_precedence {
            self.out.push('(');
            self.expr_inner(e);
            self.out.push(')');
        } else {
            self.expr_inner(e);
        }
    }

    // Each construct is printed by its own small method so that the recursive printing path
    // uses little stack per tree level in unoptimized builds.
    fn expr_inner(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Literal(lit) => self.literal(lit),
            ExprKind::Identifier { name, inclusive } => {
                if *inclusive {
                    self.out.push('@');
                }
                self.ident(name);
            }
            ExprKind::Intrinsic(i) => self.out.push_str(i.keyword()),
            ExprKind::SectionAccess { section, member } => {
                self.ident(section);
                self.out.push('!');
                self.ident(member);
            }
            ExprKind::Parenthesized(inner) => self.parenthesized(inner),
            ExprKind::Let { bindings, body } => self.let_expr(bindings, body),
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.if_expr(condition, then_branch, else_branch),
            ExprKind::Each(body) => self.keyword_expr("each ", body),
            ExprKind::Error(body) => self.keyword_expr("error ", body),
            ExprKind::Function {
                params,
                return_type,
                body,
            } => self.function(params, return_type.as_ref(), body),
            ExprKind::Invoke { function, args } => self.invoke(function, args),
            ExprKind::Record(fields) => self.record(fields),
            ExprKind::List(items) => self.list(items),
            ExprKind::FieldAccess {
                target,
                field,
                optional,
            } => self.field_access(target.as_deref(), field, *optional),
            ExprKind::FieldProjection {
                target,
                fields,
                optional,
            } => self.field_projection(target.as_deref(), fields, *optional),
            ExprKind::ItemAccess {
                target,
                selector,
                optional,
            } => self.item_access(target, selector, *optional),
            ExprKind::Unary { op, operand } => self.unary(*op, operand),
            ExprKind::Binary { op, lhs, rhs } => self.binary(*op, lhs, rhs),
            ExprKind::As { expr, ty } => self.type_test(expr, " as ", precedence::AS, ty),
            ExprKind::Is { expr, ty } => self.type_test(expr, " is ", precedence::IS, ty),
            ExprKind::Type(t) => self.type_expr(t),
            ExprKind::Try { expr, handler } => self.try_expr(expr, handler.as_ref()),
            ExprKind::NotImplemented => self.out.push_str("..."),
        }
    }

    fn parenthesized(&mut self, inner: &Expr) {
        self.out.push('(');
        self.expr(inner, 0);
        self.out.push(')');
    }

    fn let_expr(&mut self, bindings: &[Binding], body: &Expr) {
        self.out.push_str("let");
        self.indent += 1;
        for (i, b) in bindings.iter().enumerate() {
            self.newline();
            self.ident(&b.name);
            self.out.push_str(" = ");
            self.expr(&b.value, 0);
            if i + 1 < bindings.len() {
                self.out.push(',');
            }
        }
        self.indent -= 1;
        self.newline();
        self.out.push_str("in");
        self.indent += 1;
        self.newline();
        self.expr(body, 0);
        self.indent -= 1;
    }

    fn if_expr(&mut self, condition: &Expr, then_branch: &Expr, else_branch: &Expr) {
        self.out.push_str("if ");
        self.expr(condition, 0);
        self.out.push_str(" then ");
        self.expr(then_branch, 0);
        self.out.push_str(" else ");
        self.expr(else_branch, 0);
    }

    fn keyword_expr(&mut self, keyword: &str, body: &Expr) {
        self.out.push_str(keyword);
        self.expr(body, 0);
    }

    fn function(
        &mut self,
        params: &[Param],
        return_type: Option<&NullablePrimitiveType>,
        body: &Expr,
    ) {
        self.out.push('(');
        for (i, p) in params.iter().enumerate() {
            if i > 0 {
                self.out.push_str(", ");
            }
            if p.optional {
                self.out.push_str("optional ");
            }
            self.ident(&p.name);
            if let Some(t) = &p.ty {
                self.out.push_str(" as ");
                self.nullable_primitive(t);
            }
        }
        self.out.push(')');
        if let Some(t) = return_type {
            self.out.push_str(" as ");
            self.nullable_primitive(t);
        }
        self.out.push_str(" => ");
        self.expr(body, 0);
    }

    fn invoke(&mut self, function: &Expr, args: &[Expr]) {
        self.expr(function, precedence::PRIMARY);
        self.out.push('(');
        for (i, a) in args.iter().enumerate() {
            if i > 0 {
                self.out.push_str(", ");
            }
            self.expr(a, 0);
        }
        self.out.push(')');
    }

    fn record(&mut self, fields: &[RecordField]) {
        self.out.push('[');
        for (i, f) in fields.iter().enumerate() {
            if i > 0 {
                self.out.push_str(", ");
            }
            self.field_name(&f.name, false);
            self.out.push_str(" = ");
            self.expr(&f.value, 0);
        }
        self.out.push(']');
    }

    fn list(&mut self, items: &[ListItem]) {
        self.out.push('{');
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                self.out.push_str(", ");
            }
            match item {
                ListItem::Single(e) => self.expr(e, 0),
                ListItem::Range { start, end } => {
                    self.expr(start, 0);
                    self.out.push_str("..");
                    self.expr(end, 0);
                }
            }
        }
        self.out.push('}');
    }

    fn optional_marker(&mut self, optional: bool) {
        if optional {
            self.out.push('?');
        }
    }

    fn field_access(&mut self, target: Option<&Expr>, field: &Name, optional: bool) {
        if let Some(t) = target {
            self.expr(t, precedence::PRIMARY);
        }
        self.out.push('[');
        self.field_name(field, false);
        self.out.push(']');
        self.optional_marker(optional);
    }

    fn field_projection(&mut self, target: Option<&Expr>, fields: &[Name], optional: bool) {
        if let Some(t) = target {
            self.expr(t, precedence::PRIMARY);
        }
        self.out.push('[');
        for (i, f) in fields.iter().enumerate() {
            if i > 0 {
                self.out.push_str(", ");
            }
            self.out.push('[');
            self.field_name(f, false);
            self.out.push(']');
        }
        self.out.push(']');
        self.optional_marker(optional);
    }

    fn item_access(&mut self, target: &Expr, selector: &Expr, optional: bool) {
        self.expr(target, precedence::PRIMARY);
        self.out.push('{');
        self.expr(selector, 0);
        self.out.push('}');
        self.optional_marker(optional);
    }

    fn unary(&mut self, op: UnaryOp, operand: &Expr) {
        self.out.push_str(op.symbol());
        if op == UnaryOp::Not {
            self.out.push(' ');
        }
        self.expr(operand, precedence::UNARY);
    }

    fn binary(&mut self, op: BinaryOp, lhs: &Expr, rhs: &Expr) {
        let p = op.precedence();
        self.expr(lhs, p);
        self.out.push(' ');
        self.out.push_str(op.symbol());
        self.out.push(' ');
        self.expr(rhs, p + 1);
    }

    fn type_test(&mut self, expr: &Expr, keyword: &str, prec: u8, ty: &NullablePrimitiveType) {
        self.expr(expr, prec);
        self.out.push_str(keyword);
        self.nullable_primitive(ty);
    }

    fn type_expr(&mut self, t: &TypeExpr) {
        match &t.kind {
            // `type <expr>` is not valid syntax; the expression is itself the type value.
            TypeKind::Expr(inner) => self.expr(inner, precedence::PRIMARY),
            _ => {
                self.out.push_str("type ");
                self.ty(t);
            }
        }
    }

    fn try_expr(&mut self, expr: &Expr, handler: Option<&TryHandler>) {
        self.out.push_str("try ");
        let protect = handler.is_some() && ends_with_open_try(expr);
        self.expr(expr, u8::from(protect));
        match handler {
            None => {}
            Some(TryHandler::Otherwise(default)) => {
                self.out.push_str(" otherwise ");
                self.expr(default, 0);
            }
            Some(TryHandler::Catch { param, body }) => {
                self.out.push_str(" catch (");
                if let Some(p) = param {
                    self.ident(p);
                }
                self.out.push_str(") => ");
                self.expr(body, 0);
            }
        }
    }

    fn literal(&mut self, lit: &Literal) {
        match lit {
            Literal::Null => self.out.push_str("null"),
            Literal::Logical(true) => self.out.push_str("true"),
            Literal::Logical(false) => self.out.push_str("false"),
            Literal::Number(n) => {
                if n.raw.is_empty() {
                    self.out.push_str(&format!("{}", n.value));
                } else {
                    self.out.push_str(&n.raw);
                }
            }
            Literal::Text(s) => self.quoted(s),
            Literal::Verbatim(s) => {
                self.out.push_str("#!");
                self.quoted(s);
            }
        }
    }

    /// Writes `"..."` with `""` and `#(...)` escapes.
    fn quoted(&mut self, s: &str) {
        self.out.push('"');
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '"' => self.out.push_str("\"\""),
                '#' if chars.peek() == Some(&'(') => self.out.push_str("#(#)"),
                '\r' if chars.peek() == Some(&'\n') => {
                    chars.next();
                    self.out.push_str("#(cr,lf)");
                }
                '\r' => self.out.push_str("#(cr)"),
                '\n' => self.out.push_str("#(lf)"),
                '\t' => self.out.push_str("#(tab)"),
                c if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') => {
                    let code = c as u32;
                    if code <= 0xFFFF {
                        self.out.push_str(&format!("#({code:04X})"));
                    } else {
                        self.out.push_str(&format!("#({code:08X})"));
                    }
                }
                c => self.out.push(c),
            }
        }
        self.out.push('"');
    }

    /// An identifier in identifier position (variables, parameters, sections).
    fn ident(&mut self, name: &Name) {
        if !name.quoted && is_regular_identifier(&name.text) {
            self.out.push_str(&name.text);
        } else {
            self.out.push('#');
            self.quoted(&name.text);
        }
    }

    /// A field name (generalized identifier position).
    fn field_name(&mut self, name: &Name, in_record_type: bool) {
        let bare = !name.quoted
            && is_generalized_identifier(&name.text)
            && !(in_record_type && starts_with_optional_word(&name.text));
        if bare {
            self.out.push_str(&name.text);
        } else {
            self.out.push('#');
            self.quoted(&name.text);
        }
    }

    fn nullable_primitive(&mut self, t: &NullablePrimitiveType) {
        if t.nullable {
            self.out.push_str("nullable ");
        }
        self.out.push_str(t.primitive.name());
    }

    fn ty(&mut self, t: &TypeExpr) {
        match &t.kind {
            TypeKind::Primitive(p) => self.out.push_str(p.name()),
            TypeKind::Nullable(inner) => {
                self.out.push_str("nullable ");
                self.ty(inner);
            }
            TypeKind::Record(r) => self.record_type(r),
            TypeKind::List(inner) => {
                self.out.push('{');
                self.ty(inner);
                self.out.push('}');
            }
            TypeKind::Function(f) => {
                self.out.push_str("function (");
                for (i, p) in f.params.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    if p.optional {
                        self.out.push_str("optional ");
                    }
                    self.ident(&p.name);
                    if let Some(t) = &p.ty {
                        self.out.push_str(" as ");
                        self.ty(t);
                    }
                }
                self.out.push_str(") as ");
                self.ty(&f.return_type);
            }
            TypeKind::Table(TableRowType::Fields(r)) => {
                self.out.push_str("table ");
                self.record_type(r);
            }
            TypeKind::Table(TableRowType::Expr(e)) => {
                self.out.push_str("table ");
                if starts_with_identifier_or_paren(e) {
                    self.expr(e, precedence::PRIMARY);
                } else {
                    self.out.push('(');
                    self.expr(e, 0);
                    self.out.push(')');
                }
            }
            TypeKind::Expr(e) => self.expr(e, precedence::PRIMARY),
        }
    }

    fn record_type(&mut self, r: &RecordType) {
        self.out.push('[');
        for (i, f) in r.fields.iter().enumerate() {
            if i > 0 {
                self.out.push_str(", ");
            }
            if f.optional {
                self.out.push_str("optional ");
            }
            self.field_name(&f.name, true);
            if let Some(t) = &f.ty {
                self.out.push_str(" = ");
                self.ty(t);
            }
        }
        if r.open {
            if !r.fields.is_empty() {
                self.out.push_str(", ");
            }
            self.out.push_str("...");
        }
        self.out.push(']');
    }
}
