//! Spanned DAX abstract syntax tree.
//!
//! Every node carries a byte-offset [`Span`] into the source it was parsed
//! from, so the exact source text is always recoverable with
//! [`Expr::text`]. Structural comparison that ignores spans is available via
//! [`Expr::eq_ignoring_spans`] / [`Query::eq_ignoring_spans`].

use std::fmt;

/// Half-open byte range `start..end` into the parsed source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// Zero-width span at `at`.
    pub const fn empty(at: usize) -> Self {
        Self { start: at, end: at }
    }

    pub const fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub const fn is_empty(self) -> bool {
        self.end <= self.start
    }

    /// Smallest span covering both `self` and `other`.
    pub fn to(self, other: Span) -> Span {
        Span::new(self.start.min(other.start), self.end.max(other.end))
    }

    /// Source text covered by this span (empty if out of range or not on a
    /// char boundary; never panics).
    pub fn text<'a>(&self, src: &'a str) -> &'a str {
        src.get(self.start..self.end).unwrap_or("")
    }
}

/// Operator precedence levels (higher binds tighter), per the Microsoft
/// "DAX operators" reference. All binary operators are left-associative.
pub mod prec {
    /// `||`
    pub const OR: u8 = 1;
    /// `&&`
    pub const AND: u8 = 2;
    /// Prefix `NOT` (its operand absorbs comparisons and tighter operators).
    pub const NOT: u8 = 3;
    /// `= == <> < > <= >= IN`
    pub const COMPARISON: u8 = 4;
    /// `&`
    pub const CONCAT: u8 = 5;
    /// Binary `+ -`
    pub const ADDITIVE: u8 = 6;
    /// `* /`
    pub const MULTIPLICATIVE: u8 = 7;
    /// Prefix sign `+ -` (binds looser than `^`: `-2^2` is `-(2^2)`).
    pub const SIGN: u8 = 8;
    /// `^`
    pub const POWER: u8 = 9;
}

/// Numeric literal. Integers that fit in `i64` stay integers (DAX Int64);
/// anything with a decimal point or exponent, or that overflows, is a double.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Number {
    Integer(i64),
    Decimal(f64),
}

impl Number {
    pub fn as_f64(self) -> f64 {
        match self {
            Number::Integer(i) => i as f64,
            Number::Decimal(d) => d,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    /// Prefix `+` (no-op).
    Plus,
    /// Prefix `-` (negation).
    Minus,
    /// Prefix `NOT` operator form. `NOT(x)` is parsed as a call instead.
    Not,
}

impl UnaryOp {
    pub const fn symbol(self) -> &'static str {
        match self {
            UnaryOp::Plus => "+",
            UnaryOp::Minus => "-",
            UnaryOp::Not => "NOT",
        }
    }

    /// Minimum binary precedence absorbed by the operand of this prefix
    /// operator.
    pub const fn operand_precedence(self) -> u8 {
        match self {
            UnaryOp::Plus | UnaryOp::Minus => prec::SIGN,
            UnaryOp::Not => prec::NOT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    /// `^`
    Pow,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `&` (text concatenation)
    Concat,
    /// `=` (blank-tolerant equality)
    Eq,
    /// `==` (strict equality: BLANK only equals BLANK)
    StrictEq,
    /// `<>`
    Neq,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `<=`
    Le,
    /// `>=`
    Ge,
    /// `IN` (row membership in a table expression)
    In,
    /// `&&`
    And,
    /// `||`
    Or,
}

impl BinaryOp {
    pub const fn precedence(self) -> u8 {
        match self {
            BinaryOp::Pow => prec::POWER,
            BinaryOp::Mul | BinaryOp::Div => prec::MULTIPLICATIVE,
            BinaryOp::Add | BinaryOp::Sub => prec::ADDITIVE,
            BinaryOp::Concat => prec::CONCAT,
            BinaryOp::Eq
            | BinaryOp::StrictEq
            | BinaryOp::Neq
            | BinaryOp::Lt
            | BinaryOp::Gt
            | BinaryOp::Le
            | BinaryOp::Ge
            | BinaryOp::In => prec::COMPARISON,
            BinaryOp::And => prec::AND,
            BinaryOp::Or => prec::OR,
        }
    }

    pub const fn symbol(self) -> &'static str {
        match self {
            BinaryOp::Pow => "^",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Concat => "&",
            BinaryOp::Eq => "=",
            BinaryOp::StrictEq => "==",
            BinaryOp::Neq => "<>",
            BinaryOp::Lt => "<",
            BinaryOp::Gt => ">",
            BinaryOp::Le => "<=",
            BinaryOp::Ge => ">=",
            BinaryOp::In => "IN",
            BinaryOp::And => "&&",
            BinaryOp::Or => "||",
        }
    }

    pub const fn is_comparison(self) -> bool {
        self.precedence() == prec::COMPARISON
    }
}

/// `VAR name = expr` inside a [`ExprKind::Var`] block.
#[derive(Debug, Clone, PartialEq)]
pub struct VarBinding {
    pub name: String,
    pub name_span: Span,
    pub expr: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Number(Number),
    /// String literal, unescaped (`""` → `"`).
    String(String),
    /// `dt"2024-01-31"` datetime literal (raw text between the quotes).
    DateTime(String),
    /// `TRUE` / `FALSE`, written as keywords or as `TRUE()` / `FALSE()`.
    Boolean(bool),
    /// Bare table reference: `Orders` or `'Web Sessions'` (unescaped).
    /// An unquoted identifier is only a table reference when it is not a
    /// variable in scope and not an enum keyword (see [`ExprKind::Keyword`]).
    TableRef {
        name: String,
    },
    /// `Table[Column]`, `'Web Sessions'[Device]`, or bare `[Name]` (`table:
    /// None`): a bare bracket reference is a measure *or* a row-context
    /// column — the evaluator resolves it against the model.
    ColumnRef {
        table: Option<String>,
        column: String,
    },
    /// Function call. `name` is upper-cased for lookup; `raw_name` keeps the
    /// source spelling (e.g. `Calculate`). Omitted arguments (`RANKX(T, [x], ,
    /// DESC)`) appear as [`ExprKind::Omitted`].
    Call {
        name: String,
        raw_name: String,
        args: Vec<Expr>,
    },
    /// `VAR a = … VAR b = … RETURN body`.
    Var {
        bindings: Vec<VarBinding>,
        body: Box<Expr>,
    },
    /// Reference to a `VAR` (or `DEFINE VAR` / function parameter) in scope.
    VarRef(String),
    /// Query parameter `@name`.
    Parameter(String),
    /// Bare enum-like keyword argument, upper-cased: `ASC`, `DESC`, `DENSE`,
    /// `SKIP`, `YEAR`, `STRING`, `BOTH`, … (see [`crate::syntax::is_enum_keyword`]).
    Keyword(String),
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    /// `{1, 2}` → rows `[[1], [2]]`; `{(1, "a"), (2, "b")}` → rows
    /// `[[1, "a"], [2, "b"]]`.
    TableConstructor {
        rows: Vec<Vec<Expr>>,
    },
    /// Parenthesised list of two or more expressions outside a table
    /// constructor, e.g. the left side of `(T[a], T[b]) IN {(1, 2)}`.
    Tuple(Vec<Expr>),
    /// `( expr )`
    Paren(Box<Expr>),
    /// Omitted function argument (zero-width span).
    Omitted,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Self { kind, span }
    }

    /// Exact source text of this expression.
    pub fn text<'a>(&self, src: &'a str) -> &'a str {
        self.span.text(src)
    }

    /// Strips any number of enclosing [`ExprKind::Paren`] nodes.
    pub fn unparen(&self) -> &Expr {
        let mut e = self;
        while let ExprKind::Paren(inner) = &e.kind {
            e = inner;
        }
        e
    }

    /// `Some((upper_name, args))` if this is a call.
    pub fn as_call(&self) -> Option<(&str, &[Expr])> {
        match &self.kind {
            ExprKind::Call { name, args, .. } => Some((name.as_str(), args.as_slice())),
            _ => None,
        }
    }

    /// Direct children in source order.
    pub fn children(&self) -> Vec<&Expr> {
        match &self.kind {
            ExprKind::Call { args, .. } => args.iter().collect(),
            ExprKind::Var { bindings, body } => bindings
                .iter()
                .map(|b| &b.expr)
                .chain(std::iter::once(body.as_ref()))
                .collect(),
            ExprKind::Unary { expr, .. } | ExprKind::Paren(expr) => vec![expr.as_ref()],
            ExprKind::Binary { lhs, rhs, .. } => vec![lhs.as_ref(), rhs.as_ref()],
            ExprKind::TableConstructor { rows } => rows.iter().flatten().collect(),
            ExprKind::Tuple(items) => items.iter().collect(),
            ExprKind::Number(_)
            | ExprKind::String(_)
            | ExprKind::DateTime(_)
            | ExprKind::Boolean(_)
            | ExprKind::TableRef { .. }
            | ExprKind::ColumnRef { .. }
            | ExprKind::VarRef(_)
            | ExprKind::Parameter(_)
            | ExprKind::Keyword(_)
            | ExprKind::Omitted => Vec::new(),
        }
    }

    /// Direct children in source order, mutably.
    pub fn children_mut(&mut self) -> Vec<&mut Expr> {
        match &mut self.kind {
            ExprKind::Call { args, .. } => args.iter_mut().collect(),
            ExprKind::Var { bindings, body } => bindings
                .iter_mut()
                .map(|b| &mut b.expr)
                .chain(std::iter::once(body.as_mut()))
                .collect(),
            ExprKind::Unary { expr, .. } | ExprKind::Paren(expr) => vec![expr.as_mut()],
            ExprKind::Binary { lhs, rhs, .. } => vec![lhs.as_mut(), rhs.as_mut()],
            ExprKind::TableConstructor { rows } => rows.iter_mut().flatten().collect(),
            ExprKind::Tuple(items) => items.iter_mut().collect(),
            ExprKind::Number(_)
            | ExprKind::String(_)
            | ExprKind::DateTime(_)
            | ExprKind::Boolean(_)
            | ExprKind::TableRef { .. }
            | ExprKind::ColumnRef { .. }
            | ExprKind::VarRef(_)
            | ExprKind::Parameter(_)
            | ExprKind::Keyword(_)
            | ExprKind::Omitted => Vec::new(),
        }
    }

    /// Resets every span in the tree to `Span::default()` (iterative; safe
    /// on arbitrarily deep trees).
    pub fn clear_spans(&mut self) {
        let mut stack: Vec<&mut Expr> = vec![self];
        while let Some(e) = stack.pop() {
            e.span = Span::default();
            if let ExprKind::Var { bindings, .. } = &mut e.kind {
                for b in bindings.iter_mut() {
                    b.name_span = Span::default();
                }
            }
            stack.extend(e.children_mut());
        }
    }

    /// Clone with all spans cleared.
    pub fn without_spans(&self) -> Expr {
        let mut e = self.clone();
        e.clear_spans();
        e
    }

    /// Structural equality ignoring spans.
    pub fn eq_ignoring_spans(&self, other: &Expr) -> bool {
        self.without_spans() == other.without_spans()
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&crate::syntax::printer::print_expr(self))
    }
}

/// A parsed DAX query: optional `DEFINE` block and one or more `EVALUATE`
/// statements.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub defines: Vec<Define>,
    pub evaluates: Vec<Evaluate>,
    pub span: Span,
}

/// One `DEFINE` entry.
#[derive(Debug, Clone, PartialEq)]
pub enum Define {
    /// `MEASURE 'Table'[Name] = expr`
    Measure {
        table: String,
        name: String,
        expr: Expr,
        span: Span,
    },
    /// `VAR name = expr` (query-scoped variable).
    Var {
        name: String,
        expr: Expr,
        span: Span,
    },
    /// `TABLE Name = expr` (query-scoped calculated table).
    Table {
        name: String,
        expr: Expr,
        span: Span,
    },
    /// `COLUMN 'Table'[Name] = expr` (query-scoped calculated column).
    Column {
        table: String,
        name: String,
        expr: Expr,
        span: Span,
    },
    /// `FUNCTION Name = (param : type, …) => body` (user-defined function).
    Function {
        name: String,
        params: Vec<FunctionParam>,
        body: Expr,
        span: Span,
    },
}

impl Define {
    pub fn span(&self) -> Span {
        match self {
            Define::Measure { span, .. }
            | Define::Var { span, .. }
            | Define::Table { span, .. }
            | Define::Column { span, .. }
            | Define::Function { span, .. } => *span,
        }
    }

    /// The definition's expression (function body for `FUNCTION`).
    pub fn expr(&self) -> &Expr {
        match self {
            Define::Measure { expr, .. }
            | Define::Var { expr, .. }
            | Define::Table { expr, .. }
            | Define::Column { expr, .. } => expr,
            Define::Function { body, .. } => body,
        }
    }

    fn expr_mut(&mut self) -> &mut Expr {
        match self {
            Define::Measure { expr, .. }
            | Define::Var { expr, .. }
            | Define::Table { expr, .. }
            | Define::Column { expr, .. } => expr,
            Define::Function { body, .. } => body,
        }
    }

    fn clear_spans(&mut self) {
        match self {
            Define::Measure { span, .. }
            | Define::Var { span, .. }
            | Define::Table { span, .. }
            | Define::Column { span, .. } => *span = Span::default(),
            Define::Function { span, params, .. } => {
                *span = Span::default();
                for p in params {
                    p.span = Span::default();
                }
            }
        }
        self.expr_mut().clear_spans();
    }
}

/// Parameter of a user-defined function: `name [: type words]`.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionParam {
    pub name: String,
    /// Raw type hint words joined by single spaces, e.g. `"SCALAR NUMERIC VAL"`.
    pub type_hint: Option<String>,
    pub span: Span,
}

/// `(params) => body` — a user-defined function definition.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDef {
    pub params: Vec<FunctionParam>,
    pub body: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SortDir {
    #[default]
    Asc,
    Desc,
}

/// `EVALUATE expr [ORDER BY expr [ASC|DESC], …] [START AT value, …]`
#[derive(Debug, Clone, PartialEq)]
pub struct Evaluate {
    pub expr: Expr,
    pub order_by: Vec<(Expr, SortDir)>,
    pub start_at: Vec<Expr>,
    pub span: Span,
}

impl Query {
    /// Every top-level expression in the query (definitions, EVALUATE
    /// expressions, ORDER BY keys and START AT values), in source order.
    pub fn expressions(&self) -> Vec<&Expr> {
        let mut out: Vec<&Expr> = self.defines.iter().map(Define::expr).collect();
        for ev in &self.evaluates {
            out.push(&ev.expr);
            out.extend(ev.order_by.iter().map(|(e, _)| e));
            out.extend(ev.start_at.iter());
        }
        out
    }

    pub fn clear_spans(&mut self) {
        self.span = Span::default();
        for d in &mut self.defines {
            d.clear_spans();
        }
        for ev in &mut self.evaluates {
            ev.span = Span::default();
            ev.expr.clear_spans();
            for (e, _) in &mut ev.order_by {
                e.clear_spans();
            }
            for e in &mut ev.start_at {
                e.clear_spans();
            }
        }
    }

    pub fn without_spans(&self) -> Query {
        let mut q = self.clone();
        q.clear_spans();
        q
    }

    pub fn eq_ignoring_spans(&self, other: &Query) -> bool {
        self.without_spans() == other.without_spans()
    }
}

impl fmt::Display for Query {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&crate::syntax::printer::print_query(self))
    }
}

impl FunctionDef {
    pub fn clear_spans(&mut self) {
        self.span = Span::default();
        for p in &mut self.params {
            p.span = Span::default();
        }
        self.body.clear_spans();
    }
}
