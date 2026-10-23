//! Abstract syntax tree for Power Query M.
//!
//! Every [`Expr`] and [`TypeExpr`] node carries the byte [`Span`] of the source it was parsed
//! from. Parenthesized expressions are kept as [`ExprKind::Parenthesized`] nodes so that a
//! printer/formatter can reproduce the original grouping; an evaluator treats them as transparent.

use crate::span::Span;

/// A name: identifier, generalized identifier (record field names, field selectors) or quoted
/// identifier. `text` is the decoded name, e.g. `Store Name` for `[Store Name]` and
/// `Discount (%)` for `#"Discount (%)"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Name {
    pub text: String,
    pub span: Span,
    /// Whether the source spelled the name as a quoted identifier `#"..."`. Printers keep the
    /// quotes when set (and add them whenever the name requires quoting).
    pub quoted: bool,
}

impl Name {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            span: Span::default(),
            quoted: false,
        }
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }
}

/// A top-level M document: either a single expression (the body of a query, partition source or
/// TMDL `expression`) or a section document (`section X; shared a = 1;`, as in `.pq` files).
#[derive(Debug, Clone, PartialEq)]
pub enum Document {
    Expression(Expr),
    Section(Section),
}

/// `[attributes] section Name; member*`
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    /// Literal attribute record (`[Version = "1.0"]`), as an [`ExprKind::Record`] of literals.
    pub attributes: Option<Expr>,
    pub name: Name,
    pub members: Vec<SectionMember>,
    pub span: Span,
}

/// `[attributes] shared? Name = expression;`
#[derive(Debug, Clone, PartialEq)]
pub struct SectionMember {
    pub attributes: Option<Expr>,
    pub shared: bool,
    pub name: Name,
    pub value: Expr,
    pub span: Span,
}

/// An expression node.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Self { kind, span }
    }

    /// Strips any number of [`ExprKind::Parenthesized`] wrappers.
    pub fn unparenthesized(&self) -> &Expr {
        let mut e = self;
        while let ExprKind::Parenthesized(inner) = &e.kind {
            e = inner;
        }
        e
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    /// `null`, `true`, `1.5`, `0xFF`, `"text"`, `#!"verbatim"`.
    Literal(Literal),
    /// `x`, `Table.SelectRows`, `#"My Step"`, `_`; `@x` when `inclusive` (the scoping operator
    /// that makes a binding visible inside its own initializer, for recursion).
    Identifier { name: Name, inclusive: bool },
    /// `#date`, `#table`, `#infinity`, `#shared`, ... (keyword-valued identifiers).
    Intrinsic(Intrinsic),
    /// `Section1!Member`
    SectionAccess { section: Name, member: Name },
    /// `( expr )`
    Parenthesized(Box<Expr>),
    /// `let a = 1, b = a in b`
    Let {
        bindings: Vec<Binding>,
        body: Box<Expr>,
    },
    /// `if c then t else f`
    If {
        condition: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Box<Expr>,
    },
    /// `each body`: sugar for the function `(_) => body`.
    Each(Box<Expr>),
    /// `(a as text, optional b) as number => body`
    Function {
        params: Vec<Param>,
        return_type: Option<NullablePrimitiveType>,
        body: Box<Expr>,
    },
    /// `f(a, b)`
    Invoke {
        function: Box<Expr>,
        args: Vec<Expr>,
    },
    /// `[a = 1, #"b c" = 2]`
    Record(Vec<RecordField>),
    /// `{1, 2..5}`
    List(Vec<ListItem>),
    /// `target[field]`, `target[field]?`; `target: None` is the implicit-target form `[field]`,
    /// which means `_[field]`.
    FieldAccess {
        target: Option<Box<Expr>>,
        field: Name,
        optional: bool,
    },
    /// `target[[a], [b]]`, `target[[a]]?`; `target: None` is the implicit-target form.
    FieldProjection {
        target: Option<Box<Expr>>,
        fields: Vec<Name>,
        optional: bool,
    },
    /// `target{selector}`, `target{selector}?`. The selector is a number (position) or, for
    /// tables, a record of key values (`Source{[Name = "x"]}`).
    ItemAccess {
        target: Box<Expr>,
        selector: Box<Expr>,
        optional: bool,
    },
    /// `-x`, `+x`, `not x`
    Unary { op: UnaryOp, operand: Box<Expr> },
    /// `lhs op rhs` for arithmetic, comparison, `&`, `and`, `or`, `??` and `meta`.
    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    /// `expr as nullable text` (type assertion).
    As {
        expr: Box<Expr>,
        ty: NullablePrimitiveType,
    },
    /// `expr is nullable text` (type conformance test).
    Is {
        expr: Box<Expr>,
        ty: NullablePrimitiveType,
    },
    /// `type T`
    Type(Box<TypeExpr>),
    /// `try expr`, `try expr otherwise default`, `try expr catch (e) => handler`
    Try {
        expr: Box<Expr>,
        handler: Option<TryHandler>,
    },
    /// `error expr`
    Error(Box<Expr>),
    /// `...`
    NotImplemented,
}

/// A `let` variable: `name = value`.
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    pub name: Name,
    pub value: Expr,
}

/// A record expression field: `name = value`.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordField {
    pub name: Name,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ListItem {
    Single(Expr),
    /// `start..end` (inclusive integer range).
    Range {
        start: Expr,
        end: Expr,
    },
}

/// A function-expression parameter: `optional? name (as nullable? primitive)?`.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: Name,
    pub optional: bool,
    pub ty: Option<NullablePrimitiveType>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TryHandler {
    /// `otherwise default`
    Otherwise(Box<Expr>),
    /// `catch (e) => body` / `catch () => body`
    Catch {
        param: Option<Name>,
        body: Box<Expr>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Null,
    Logical(bool),
    Number(NumberLiteral),
    Text(String),
    /// `#!"..."`: an unparsed expression preserved verbatim; evaluates to an error.
    Verbatim(String),
}

/// A number literal: the source spelling (decimal, exponent or `0x` hex) and its value.
#[derive(Debug, Clone, PartialEq)]
pub struct NumberLiteral {
    /// Must be a valid M number literal (non-negative; no sign).
    pub raw: String,
    pub value: f64,
}

impl NumberLiteral {
    /// Builds a literal from a finite, non-negative value.
    pub fn from_f64(value: f64) -> Self {
        Self {
            raw: format!("{value}"),
            value,
        }
    }
}

/// Keyword-valued identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Intrinsic {
    /// `#binary(...)` function
    Binary,
    /// `#date(y, m, d)` function
    Date,
    /// `#datetime(y, m, d, h, mi, s)` function
    DateTime,
    /// `#datetimezone(y, m, d, h, mi, s, oh, om)` function
    DateTimeZone,
    /// `#duration(d, h, m, s)` function
    Duration,
    /// positive infinity number
    Infinity,
    /// NaN number
    Nan,
    /// record of all sections
    Sections,
    /// record of the shared members of the current environment
    Shared,
    /// `#table(columns, rows)` function
    Table,
    /// `#time(h, m, s)` function
    Time,
}

impl Intrinsic {
    pub const fn keyword(self) -> &'static str {
        match self {
            Intrinsic::Binary => "#binary",
            Intrinsic::Date => "#date",
            Intrinsic::DateTime => "#datetime",
            Intrinsic::DateTimeZone => "#datetimezone",
            Intrinsic::Duration => "#duration",
            Intrinsic::Infinity => "#infinity",
            Intrinsic::Nan => "#nan",
            Intrinsic::Sections => "#sections",
            Intrinsic::Shared => "#shared",
            Intrinsic::Table => "#table",
            Intrinsic::Time => "#time",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    Plus,
    Minus,
    Not,
}

impl UnaryOp {
    pub const fn symbol(self) -> &'static str {
        match self {
            UnaryOp::Plus => "+",
            UnaryOp::Minus => "-",
            UnaryOp::Not => "not",
        }
    }
}

/// Binary operators. See [`BinaryOp::precedence`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    /// `??`
    Coalesce,
    /// `or` (short-circuit)
    Or,
    /// `and` (short-circuit)
    And,
    /// `=`
    Equal,
    /// `<>`
    NotEqual,
    /// `<`
    LessThan,
    /// `<=`
    LessThanOrEqual,
    /// `>`
    GreaterThan,
    /// `>=`
    GreaterThanOrEqual,
    /// `+`
    Add,
    /// `-`
    Subtract,
    /// `&` (text/list/record/table combination)
    Concatenate,
    /// `*`
    Multiply,
    /// `/`
    Divide,
    /// `meta`
    Meta,
}

/// Precedence levels, lowest to highest, following the M specification's operator table.
pub mod precedence {
    pub const COALESCE: u8 = 1;
    pub const OR: u8 = 2;
    pub const AND: u8 = 3;
    pub const IS: u8 = 4;
    pub const AS: u8 = 5;
    pub const EQUALITY: u8 = 6;
    pub const RELATIONAL: u8 = 7;
    pub const ADDITIVE: u8 = 8;
    pub const MULTIPLICATIVE: u8 = 9;
    pub const META: u8 = 10;
    pub const UNARY: u8 = 11;
    pub const PRIMARY: u8 = 12;
}

impl BinaryOp {
    pub const fn symbol(self) -> &'static str {
        match self {
            BinaryOp::Coalesce => "??",
            BinaryOp::Or => "or",
            BinaryOp::And => "and",
            BinaryOp::Equal => "=",
            BinaryOp::NotEqual => "<>",
            BinaryOp::LessThan => "<",
            BinaryOp::LessThanOrEqual => "<=",
            BinaryOp::GreaterThan => ">",
            BinaryOp::GreaterThanOrEqual => ">=",
            BinaryOp::Add => "+",
            BinaryOp::Subtract => "-",
            BinaryOp::Concatenate => "&",
            BinaryOp::Multiply => "*",
            BinaryOp::Divide => "/",
            BinaryOp::Meta => "meta",
        }
    }

    /// Binding power (higher binds tighter). All binary operators are left-associative.
    pub const fn precedence(self) -> u8 {
        use precedence::*;
        match self {
            BinaryOp::Coalesce => COALESCE,
            BinaryOp::Or => OR,
            BinaryOp::And => AND,
            BinaryOp::Equal | BinaryOp::NotEqual => EQUALITY,
            BinaryOp::LessThan
            | BinaryOp::LessThanOrEqual
            | BinaryOp::GreaterThan
            | BinaryOp::GreaterThanOrEqual => RELATIONAL,
            BinaryOp::Add | BinaryOp::Subtract | BinaryOp::Concatenate => ADDITIVE,
            BinaryOp::Multiply | BinaryOp::Divide => MULTIPLICATIVE,
            BinaryOp::Meta => META,
        }
    }
}

/// Primitive type names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimitiveType {
    /// `action` (not in the published spec, accepted by Power Query).
    Action,
    Any,
    AnyNonNull,
    Binary,
    Date,
    DateTime,
    DateTimeZone,
    Duration,
    Function,
    List,
    Logical,
    None,
    Null,
    Number,
    Record,
    Table,
    Text,
    Time,
    Type,
}

impl PrimitiveType {
    pub const ALL: [PrimitiveType; 19] = [
        PrimitiveType::Action,
        PrimitiveType::Any,
        PrimitiveType::AnyNonNull,
        PrimitiveType::Binary,
        PrimitiveType::Date,
        PrimitiveType::DateTime,
        PrimitiveType::DateTimeZone,
        PrimitiveType::Duration,
        PrimitiveType::Function,
        PrimitiveType::List,
        PrimitiveType::Logical,
        PrimitiveType::None,
        PrimitiveType::Null,
        PrimitiveType::Number,
        PrimitiveType::Record,
        PrimitiveType::Table,
        PrimitiveType::Text,
        PrimitiveType::Time,
        PrimitiveType::Type,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            PrimitiveType::Action => "action",
            PrimitiveType::Any => "any",
            PrimitiveType::AnyNonNull => "anynonnull",
            PrimitiveType::Binary => "binary",
            PrimitiveType::Date => "date",
            PrimitiveType::DateTime => "datetime",
            PrimitiveType::DateTimeZone => "datetimezone",
            PrimitiveType::Duration => "duration",
            PrimitiveType::Function => "function",
            PrimitiveType::List => "list",
            PrimitiveType::Logical => "logical",
            PrimitiveType::None => "none",
            PrimitiveType::Null => "null",
            PrimitiveType::Number => "number",
            PrimitiveType::Record => "record",
            PrimitiveType::Table => "table",
            PrimitiveType::Text => "text",
            PrimitiveType::Time => "time",
            PrimitiveType::Type => "type",
        }
    }

    /// Looks up a primitive type by its exact (case-sensitive) name.
    pub fn from_name(name: &str) -> Option<PrimitiveType> {
        PrimitiveType::ALL
            .iter()
            .copied()
            .find(|p| p.name() == name)
    }
}

/// `nullable? primitive-type`, as used by `as`/`is` and function-expression parameter and return
/// types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NullablePrimitiveType {
    pub nullable: bool,
    pub primitive: PrimitiveType,
    pub span: Span,
}

/// A type in a type context (after the `type` keyword, inside record/list/table/function types).
#[derive(Debug, Clone, PartialEq)]
pub struct TypeExpr {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeKind {
    /// `text`, `number`, `any`, `table`, `null`, `type`, ...
    Primitive(PrimitiveType),
    /// `nullable T`
    Nullable(Box<TypeExpr>),
    /// `[a = text, optional b, ...]`
    Record(RecordType),
    /// `{T}`
    List(Box<TypeExpr>),
    /// `function (a as text, optional b as number) as text`
    Function(FunctionType),
    /// `table [a = text]` or `table RowTypeExpr`
    Table(TableRowType),
    /// A primary expression evaluating to a type value, e.g. `Int64.Type`, `(type text meta [])`.
    Expr(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordType {
    pub fields: Vec<FieldSpec>,
    /// Open record marker `...`.
    pub open: bool,
}

/// `optional? name (= type)?`; a missing type means `any`.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldSpec {
    pub name: Name,
    pub optional: bool,
    pub ty: Option<TypeExpr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionType {
    pub params: Vec<ParamSpec>,
    pub return_type: Box<TypeExpr>,
}

/// A function-type parameter: `optional? name (as type)?`.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamSpec {
    pub name: Name,
    pub optional: bool,
    pub ty: Option<TypeExpr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TableRowType {
    /// `table [a = text, b = number]` (never open).
    Fields(RecordType),
    /// `table RowType` where the primary expression evaluates to a record type.
    Expr(Box<Expr>),
}

// ---------------------------------------------------------------------------------------------
// Span utilities
// ---------------------------------------------------------------------------------------------

impl Document {
    /// Resets every span in the tree to `Span::default()`, e.g. to compare trees structurally.
    pub fn clear_spans(&mut self) {
        self.for_each_span_mut(&mut |s| *s = Span::default());
    }

    pub fn for_each_span_mut(&mut self, f: &mut dyn FnMut(&mut Span)) {
        match self {
            Document::Expression(e) => e.for_each_span_mut(f),
            Document::Section(section) => {
                f(&mut section.span);
                if let Some(a) = &mut section.attributes {
                    a.for_each_span_mut(f);
                }
                f(&mut section.name.span);
                for m in &mut section.members {
                    f(&mut m.span);
                    if let Some(a) = &mut m.attributes {
                        a.for_each_span_mut(f);
                    }
                    f(&mut m.name.span);
                    m.value.for_each_span_mut(f);
                }
            }
        }
    }
}

impl Expr {
    /// Resets every span in the tree to `Span::default()`, e.g. to compare trees structurally.
    pub fn clear_spans(&mut self) {
        self.for_each_span_mut(&mut |s| *s = Span::default());
    }

    /// Calls `f` on every span (expressions, names, types) in the tree.
    pub fn for_each_span_mut(&mut self, f: &mut dyn FnMut(&mut Span)) {
        f(&mut self.span);
        match &mut self.kind {
            ExprKind::Literal(_) | ExprKind::Intrinsic(_) | ExprKind::NotImplemented => {}
            ExprKind::Identifier { name, .. } => f(&mut name.span),
            ExprKind::SectionAccess { section, member } => {
                f(&mut section.span);
                f(&mut member.span);
            }
            ExprKind::Parenthesized(e) | ExprKind::Each(e) | ExprKind::Error(e) => {
                e.for_each_span_mut(f);
            }
            ExprKind::Let { bindings, body } => {
                for b in bindings {
                    f(&mut b.name.span);
                    b.value.for_each_span_mut(f);
                }
                body.for_each_span_mut(f);
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                condition.for_each_span_mut(f);
                then_branch.for_each_span_mut(f);
                else_branch.for_each_span_mut(f);
            }
            ExprKind::Function {
                params,
                return_type,
                body,
            } => {
                for p in params {
                    f(&mut p.name.span);
                    if let Some(t) = &mut p.ty {
                        f(&mut t.span);
                    }
                }
                if let Some(t) = return_type {
                    f(&mut t.span);
                }
                body.for_each_span_mut(f);
            }
            ExprKind::Invoke { function, args } => {
                function.for_each_span_mut(f);
                for a in args {
                    a.for_each_span_mut(f);
                }
            }
            ExprKind::Record(fields) => {
                for field in fields {
                    f(&mut field.name.span);
                    field.value.for_each_span_mut(f);
                }
            }
            ExprKind::List(items) => {
                for item in items {
                    match item {
                        ListItem::Single(e) => e.for_each_span_mut(f),
                        ListItem::Range { start, end } => {
                            start.for_each_span_mut(f);
                            end.for_each_span_mut(f);
                        }
                    }
                }
            }
            ExprKind::FieldAccess { target, field, .. } => {
                if let Some(t) = target {
                    t.for_each_span_mut(f);
                }
                f(&mut field.span);
            }
            ExprKind::FieldProjection { target, fields, .. } => {
                if let Some(t) = target {
                    t.for_each_span_mut(f);
                }
                for n in fields {
                    f(&mut n.span);
                }
            }
            ExprKind::ItemAccess {
                target, selector, ..
            } => {
                target.for_each_span_mut(f);
                selector.for_each_span_mut(f);
            }
            ExprKind::Unary { operand, .. } => operand.for_each_span_mut(f),
            ExprKind::Binary { lhs, rhs, .. } => {
                lhs.for_each_span_mut(f);
                rhs.for_each_span_mut(f);
            }
            ExprKind::As { expr, ty } | ExprKind::Is { expr, ty } => {
                expr.for_each_span_mut(f);
                f(&mut ty.span);
            }
            ExprKind::Type(t) => t.for_each_span_mut(f),
            ExprKind::Try { expr, handler } => {
                expr.for_each_span_mut(f);
                match handler {
                    None => {}
                    Some(TryHandler::Otherwise(e)) => e.for_each_span_mut(f),
                    Some(TryHandler::Catch { param, body }) => {
                        if let Some(p) = param {
                            f(&mut p.span);
                        }
                        body.for_each_span_mut(f);
                    }
                }
            }
        }
    }
}

impl TypeExpr {
    /// Calls `f` on every span in the type.
    pub fn for_each_span_mut(&mut self, f: &mut dyn FnMut(&mut Span)) {
        f(&mut self.span);
        match &mut self.kind {
            TypeKind::Primitive(_) => {}
            TypeKind::Nullable(t) | TypeKind::List(t) => t.for_each_span_mut(f),
            TypeKind::Record(r) | TypeKind::Table(TableRowType::Fields(r)) => {
                record_type_spans(r, f);
            }
            TypeKind::Table(TableRowType::Expr(e)) | TypeKind::Expr(e) => e.for_each_span_mut(f),
            TypeKind::Function(ft) => {
                for p in &mut ft.params {
                    f(&mut p.name.span);
                    if let Some(t) = &mut p.ty {
                        t.for_each_span_mut(f);
                    }
                }
                ft.return_type.for_each_span_mut(f);
            }
        }
    }
}

fn record_type_spans(r: &mut RecordType, f: &mut dyn FnMut(&mut Span)) {
    for field in &mut r.fields {
        f(&mut field.name.span);
        if let Some(t) = &mut field.ty {
            t.for_each_span_mut(f);
        }
    }
}
