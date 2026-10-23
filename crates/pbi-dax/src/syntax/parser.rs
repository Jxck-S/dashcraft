//! Hand-written Pratt parser for DAX expressions and queries.
//!
//! Precedence (tightest first), per Microsoft's "DAX operators" reference:
//! `^`; prefix sign `+ -`; `* /`; `+ -`; `&`; comparison `= == <> < > <= >=`
//! and `IN`; prefix `NOT`; `&&`; `||`. All binary operators are
//! left-associative (`2^3^2` is `(2^3)^2`), and `-2^2` is `-(2^2)`.
//!
//! The parser never panics. AST depth — each nesting level *and* each link of
//! a left-nested operator chain such as `a + b + c` — is capped at
//! [`MAX_DEPTH`]; deeper input produces a [`ParseError`]. Code that recurses
//! over parsed ASTs (printer, evaluator, derived `Clone`/`PartialEq`) can rely
//! on this bound.

use super::ast::{
    BinaryOp, Define, Evaluate, Expr, ExprKind, FunctionDef, FunctionParam, Number, Query, SortDir,
    Span, UnaryOp, VarBinding,
};
use super::error::ParseError;
use super::lexer::{Keyword, Token, TokenKind, is_enum_keyword, tokenize};

/// Maximum AST depth: nesting levels (parentheses, calls, prefix operators,
/// VAR blocks, table constructors …) plus chained binary operators. Also
/// bounds parser recursion.
pub const MAX_DEPTH: usize = 256;

/// Internal result type. Both sides are boxed so that the values copied
/// around at every `?` on the recursive descent path are 16 bytes rather than
/// ~100: unoptimised (debug) builds give every temporary its own stack slot,
/// and this keeps a nesting level at roughly 1–2 KiB of stack.
type PResult<T> = Result<T, Box<ParseError>>;
type BExpr = Box<Expr>;

/// Parses a single DAX expression (measure, calculated column / table,
/// filter expression …). Leading/trailing whitespace and comments are allowed.
pub fn parse_expression(src: &str) -> Result<Expr, ParseError> {
    let run = || -> PResult<Expr> {
        let mut p = Parser::new(src)?;
        let expr = p.parse_expr(0)?;
        p.expect_eof()?;
        Ok(*expr)
    };
    run().map_err(|e| *e)
}

/// Parses a DAX query: `[DEFINE …] EVALUATE … [ORDER BY …] [START AT …]`,
/// with one or more `EVALUATE` statements.
pub fn parse_query(src: &str) -> Result<Query, ParseError> {
    let run = || -> PResult<Query> {
        let mut p = Parser::new(src)?;
        p.scopes.push(Vec::new());
        let query = p.parse_query()?;
        p.expect_eof()?;
        Ok(query)
    };
    run().map_err(|e| *e)
}

/// Parses a user-defined function definition `(param [: type], …) => body`
/// (the right-hand side of a TMDL `function Name = …`).
pub fn parse_function_definition(src: &str) -> Result<FunctionDef, ParseError> {
    let run = || -> PResult<FunctionDef> {
        let mut p = Parser::new(src)?;
        let def = p.parse_function_def()?;
        p.expect_eof()?;
        Ok(def)
    };
    run().map_err(|e| *e)
}

struct Parser<'a> {
    src: &'a str,
    /// Non-trivia tokens.
    tokens: Vec<Token>,
    pos: usize,
    /// Current AST depth (see [`MAX_DEPTH`]).
    depth: usize,
    /// Lower-cased variable names in scope, innermost last.
    scopes: Vec<Vec<String>>,
}

/// How an identifier in operand position is parsed.
enum IdentForm {
    VarBlock,
    Call,
    Atom,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> PResult<Self> {
        let mut tokens = Vec::new();
        for tok in tokenize(src) {
            if let TokenKind::Error(err) = tok.kind {
                return Err(Box::new(ParseError::new(err.message(), tok.span)));
            }
            if !tok.is_trivia() {
                tokens.push(tok);
            }
        }
        Ok(Self {
            src,
            tokens,
            pos: 0,
            depth: 0,
            scopes: Vec::new(),
        })
    }

    // ---- token helpers -------------------------------------------------

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn peek_kind(&self) -> Option<TokenKind> {
        self.peek().map(|t| t.kind)
    }

    fn peek_kind_at(&self, offset: usize) -> Option<TokenKind> {
        self.tokens.get(self.pos + offset).map(|t| t.kind)
    }

    fn peek_keyword(&self) -> Option<Keyword> {
        self.peek().and_then(|t| t.keyword(self.src))
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.peek_kind() == Some(kind)
    }

    fn at_keyword(&self, kw: Keyword) -> bool {
        self.peek_keyword() == Some(kw)
    }

    fn bump(&mut self) -> Option<Token> {
        let tok = self.tokens.get(self.pos).copied();
        if tok.is_some() {
            self.pos += 1;
        }
        tok
    }

    /// Consumes the current token and returns its span (empty at end).
    fn bump_span(&mut self) -> Span {
        let span = self.current_span();
        self.bump();
        span
    }

    fn eat(&mut self, kind: TokenKind) -> Option<Token> {
        if self.at(kind) { self.bump() } else { None }
    }

    fn eat_keyword(&mut self, kw: Keyword) -> Option<Token> {
        if self.at_keyword(kw) {
            self.bump()
        } else {
            None
        }
    }

    fn expect(&mut self, kind: TokenKind, what: &str) -> PResult<Token> {
        match self.eat(kind) {
            Some(t) => Ok(t),
            None => Err(self.unexpected(&[what])),
        }
    }

    fn expect_one_of(&mut self, kind: TokenKind, expected: &[&str]) -> PResult<Token> {
        match self.eat(kind) {
            Some(t) => Ok(t),
            None => Err(self.unexpected(expected)),
        }
    }

    fn expect_keyword(&mut self, kw: Keyword) -> PResult<Token> {
        match self.eat_keyword(kw) {
            Some(t) => Ok(t),
            None => Err(self.unexpected(&[kw.as_str()])),
        }
    }

    fn expect_eof(&self) -> PResult<()> {
        if self.peek().is_none() {
            Ok(())
        } else {
            Err(self.unexpected(&["end of input"]))
        }
    }

    fn text(&self, tok: &Token) -> &'a str {
        tok.span.text(self.src)
    }

    /// End offset of the previously consumed token.
    fn prev_end(&self) -> usize {
        self.pos
            .checked_sub(1)
            .and_then(|i| self.tokens.get(i))
            .map_or(0, |t| t.span.end)
    }

    fn describe(&self, tok: &Token) -> String {
        let text = self.text(tok);
        match tok.kind {
            TokenKind::Ident => match tok.keyword(self.src) {
                Some(kw) if kw.is_reserved() => format!("keyword {}", kw.as_str()),
                _ => format!("identifier '{text}'"),
            },
            TokenKind::QuotedIdent => format!("name {text}"),
            TokenKind::BracketIdent => format!("column/measure reference {text}"),
            TokenKind::String => "string literal".to_string(),
            TokenKind::DateTime => "datetime literal".to_string(),
            TokenKind::Number => format!("number {text}"),
            TokenKind::Parameter => format!("parameter {text}"),
            _ => format!("'{text}'"),
        }
    }

    /// "unexpected X" error at the current token (or end of input).
    #[cold]
    #[inline(never)]
    fn unexpected(&self, expected: &[&str]) -> Box<ParseError> {
        let err = match self.peek() {
            Some(tok) => ParseError::new(format!("unexpected {}", self.describe(tok)), tok.span),
            None => ParseError::new("unexpected end of input", Span::empty(self.src.len())),
        };
        Box::new(err.with_expected(expected.iter().copied()))
    }

    fn current_span(&self) -> Span {
        self.peek().map_or(Span::empty(self.src.len()), |t| t.span)
    }

    // ---- depth limiting --------------------------------------------------

    /// Accounts for one more AST level (a nested expression or a chained
    /// binary operator).
    #[inline(never)]
    fn enter(&mut self) -> PResult<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(Box::new(ParseError::new(
                format!(
                    "expression is nested too deeply (limit {MAX_DEPTH} levels, counting \
                     chained operators)"
                ),
                self.current_span(),
            )));
        }
        Ok(())
    }

    // ---- scopes ------------------------------------------------------------

    fn declare(&mut self, name: &str) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.push(name.to_lowercase());
        }
    }

    fn is_var(&self, name: &str) -> bool {
        let lower = name.to_lowercase();
        self.scopes.iter().any(|s| s.contains(&lower))
    }

    // ---- expressions (recursive path: keep frames small) -------------------

    fn parse_expr(&mut self, min_prec: u8) -> PResult<BExpr> {
        let saved = self.depth;
        let result = self.parse_expr_inner(min_prec);
        self.depth = saved;
        result
    }

    fn parse_expr_inner(&mut self, min_prec: u8) -> PResult<BExpr> {
        self.enter()?;
        let mut lhs = self.parse_prefix()?;
        while let Some(op) = self.peek_binary_op() {
            if op.precedence() < min_prec {
                break;
            }
            self.enter()?;
            self.bump();
            let rhs = self.parse_expr(op.precedence() + 1)?;
            lhs = make_binary(op, lhs, rhs);
        }
        Ok(lhs)
    }

    fn peek_binary_op(&self) -> Option<BinaryOp> {
        Some(match self.peek_kind()? {
            TokenKind::Caret => BinaryOp::Pow,
            TokenKind::Star => BinaryOp::Mul,
            TokenKind::Slash => BinaryOp::Div,
            TokenKind::Plus => BinaryOp::Add,
            TokenKind::Minus => BinaryOp::Sub,
            TokenKind::Amp => BinaryOp::Concat,
            TokenKind::Eq => BinaryOp::Eq,
            TokenKind::EqEq => BinaryOp::StrictEq,
            TokenKind::Neq => BinaryOp::Neq,
            TokenKind::Lt => BinaryOp::Lt,
            TokenKind::Gt => BinaryOp::Gt,
            TokenKind::Le => BinaryOp::Le,
            TokenKind::Ge => BinaryOp::Ge,
            TokenKind::AmpAmp => BinaryOp::And,
            TokenKind::PipePipe => BinaryOp::Or,
            TokenKind::Ident if self.at_keyword(Keyword::In) => BinaryOp::In,
            _ => return None,
        })
    }

    fn peek_prefix_op(&self) -> Option<UnaryOp> {
        match self.peek_kind()? {
            TokenKind::Plus => Some(UnaryOp::Plus),
            TokenKind::Minus => Some(UnaryOp::Minus),
            // `NOT(` is the function form, handled as a call.
            TokenKind::Ident
                if self.at_keyword(Keyword::Not)
                    && self.peek_kind_at(1) != Some(TokenKind::LParen) =>
            {
                Some(UnaryOp::Not)
            }
            _ => None,
        }
    }

    fn parse_prefix(&mut self) -> PResult<BExpr> {
        let Some(op) = self.peek_prefix_op() else {
            return self.parse_primary();
        };
        let start = self.bump_span().start;
        let operand = self.parse_expr(op.operand_precedence())?;
        Ok(make_unary(op, start, operand))
    }

    fn parse_primary(&mut self) -> PResult<BExpr> {
        match self.peek_kind() {
            Some(TokenKind::LParen) => self.parse_paren(),
            Some(TokenKind::LBrace) => self.parse_table_constructor(),
            Some(TokenKind::Ident) => match self.ident_form() {
                IdentForm::VarBlock => self.parse_var_block(),
                IdentForm::Call => self.parse_call(),
                IdentForm::Atom => self.parse_ident_atom(),
            },
            _ => self.parse_atom(),
        }
    }

    fn ident_form(&self) -> IdentForm {
        match self.peek_keyword() {
            Some(Keyword::Var) => IdentForm::VarBlock,
            Some(Keyword::True | Keyword::False) => IdentForm::Atom,
            Some(kw) if kw.is_reserved() && kw != Keyword::Not => IdentForm::Atom,
            _ if self.peek_kind_at(1) == Some(TokenKind::LParen) => IdentForm::Call,
            _ => IdentForm::Atom,
        }
    }

    fn parse_call(&mut self) -> PResult<BExpr> {
        let name_span = self.bump_span();
        self.bump(); // (
        let mut args: Vec<Expr> = Vec::new();
        if !self.at(TokenKind::RParen) {
            loop {
                if self.at(TokenKind::Comma) || self.at(TokenKind::RParen) {
                    push_omitted(&mut args, self.current_span().start);
                } else {
                    let arg = self.parse_expr(0)?;
                    push_expr(&mut args, arg);
                }
                if self.eat(TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        let close = self.expect_one_of(TokenKind::RParen, &["','", "')'"])?;
        Ok(make_call(self.src, name_span, args, close.span))
    }

    fn parse_paren(&mut self) -> PResult<BExpr> {
        let open = self.bump_span();
        let first = self.parse_expr(0)?;
        if !self.at(TokenKind::Comma) {
            let close = self.expect_one_of(TokenKind::RParen, &["')'", "operator"])?;
            return Ok(make_paren(first, open.to(close.span)));
        }
        let mut items = Vec::new();
        push_expr(&mut items, first);
        while self.eat(TokenKind::Comma).is_some() {
            let item = self.parse_expr(0)?;
            push_expr(&mut items, item);
        }
        let close = self.expect_one_of(TokenKind::RParen, &["','", "')'"])?;
        Ok(make_node(ExprKind::Tuple(items), open.to(close.span)))
    }

    fn parse_table_constructor(&mut self) -> PResult<BExpr> {
        let open = self.bump_span();
        let mut rows = Vec::new();
        if !self.at(TokenKind::RBrace) {
            loop {
                let item = self.parse_expr(0)?;
                push_row(&mut rows, item);
                if self.eat(TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        let close = self.expect_one_of(TokenKind::RBrace, &["','", "'}'"])?;
        Ok(make_node(
            ExprKind::TableConstructor { rows },
            open.to(close.span),
        ))
    }

    fn parse_var_block(&mut self) -> PResult<BExpr> {
        self.scopes.push(Vec::new());
        let result = self.parse_var_block_inner();
        self.scopes.pop();
        result
    }

    fn parse_var_block_inner(&mut self) -> PResult<BExpr> {
        let start = self.current_span().start;
        let mut bindings = Vec::new();
        while self.eat_keyword(Keyword::Var).is_some() {
            let name_span = self.expect_name_span("variable name")?;
            self.expect(TokenKind::Eq, "'='")?;
            let expr = self.parse_expr(0)?;
            self.push_binding(&mut bindings, name_span, expr);
        }
        if self.eat_keyword(Keyword::Return).is_none() {
            return Err(self.unexpected(&["VAR", "RETURN"]));
        }
        let body = self.parse_expr(0)?;
        Ok(make_var(start, bindings, body))
    }

    #[allow(clippy::boxed_local)]
    #[inline(never)]
    fn push_binding(&mut self, bindings: &mut Vec<VarBinding>, name_span: Span, expr: BExpr) {
        let name = name_span.text(self.src).to_string();
        self.declare(&name);
        bindings.push(VarBinding {
            name,
            name_span,
            expr: *expr,
        });
    }

    // ---- non-recursive operands ------------------------------------------

    /// Literals, bracketed / quoted references and errors.
    #[inline(never)]
    fn parse_atom(&mut self) -> PResult<BExpr> {
        let Some(tok) = self.peek().copied() else {
            return Err(self.unexpected(&["expression"]));
        };
        let kind = match tok.kind {
            TokenKind::Number => match parse_number(self.text(&tok)) {
                Some(n) => ExprKind::Number(n),
                None => {
                    return Err(Box::new(ParseError::new(
                        "invalid number literal",
                        tok.span,
                    )));
                }
            },
            TokenKind::String => ExprKind::String(tok.value(self.src)),
            TokenKind::DateTime => ExprKind::DateTime(tok.value(self.src)),
            TokenKind::Parameter => ExprKind::Parameter(tok.value(self.src)),
            TokenKind::BracketIdent => ExprKind::ColumnRef {
                table: None,
                column: tok.value(self.src),
            },
            TokenKind::QuotedIdent => {
                self.bump();
                return Ok(self.table_or_column(tok.value(self.src), tok.span));
            }
            _ => return Err(self.unexpected(&["expression"])),
        };
        self.bump();
        Ok(Box::new(Expr::new(kind, tok.span)))
    }

    /// After a table name: `Table[Column]` or a bare table reference.
    fn table_or_column(&mut self, table: String, table_span: Span) -> BExpr {
        let expr = match self.eat(TokenKind::BracketIdent) {
            Some(col) => Expr::new(
                ExprKind::ColumnRef {
                    table: Some(table),
                    column: col.value(self.src),
                },
                table_span.to(col.span),
            ),
            None => Expr::new(ExprKind::TableRef { name: table }, table_span),
        };
        Box::new(expr)
    }

    /// Identifier that is not a call or VAR block: TRUE/FALSE, table or
    /// column reference, variable, enum keyword.
    #[inline(never)]
    fn parse_ident_atom(&mut self) -> PResult<BExpr> {
        let Some(tok) = self.peek().copied() else {
            return Err(self.unexpected(&["expression"]));
        };
        let text = self.text(&tok);
        match tok.keyword(self.src) {
            Some(kw @ (Keyword::True | Keyword::False)) => {
                self.bump();
                let mut span = tok.span;
                if self.eat(TokenKind::LParen).is_some() {
                    let close = self.expect(TokenKind::RParen, "')'")?;
                    span = span.to(close.span);
                }
                return Ok(Box::new(Expr::new(
                    ExprKind::Boolean(kw == Keyword::True),
                    span,
                )));
            }
            // Reserved words are only accepted as `Order[Id]`-style table
            // qualifiers.
            Some(kw)
                if kw.is_reserved() && self.peek_kind_at(1) != Some(TokenKind::BracketIdent) =>
            {
                return Err(self.unexpected(&["expression"]));
            }
            _ => {}
        }
        self.bump();
        if self.at(TokenKind::BracketIdent) {
            return Ok(self.table_or_column(text.to_string(), tok.span));
        }
        let kind = if self.is_var(text) {
            ExprKind::VarRef(text.to_string())
        } else if is_enum_keyword(text) {
            ExprKind::Keyword(text.to_ascii_uppercase())
        } else {
            ExprKind::TableRef {
                name: text.to_string(),
            }
        };
        Ok(Box::new(Expr::new(kind, tok.span)))
    }

    /// Span of an unquoted, non-reserved identifier (variable / parameter
    /// name).
    fn expect_name_span(&mut self, what: &str) -> PResult<Span> {
        match self.peek().copied() {
            Some(tok)
                if tok.kind == TokenKind::Ident
                    && !tok.keyword(self.src).is_some_and(Keyword::is_reserved) =>
            {
                self.bump();
                Ok(tok.span)
            }
            _ => Err(self.unexpected(&[what])),
        }
    }

    /// An unquoted, non-reserved identifier (variable / parameter name).
    fn expect_name(&mut self, what: &str) -> PResult<(String, Span)> {
        let span = self.expect_name_span(what)?;
        Ok((span.text(self.src).to_string(), span))
    }

    /// A table name: unquoted identifier or `'quoted name'`.
    fn expect_table_name(&mut self) -> PResult<(String, Span)> {
        match self.peek().copied() {
            Some(tok) if tok.kind == TokenKind::QuotedIdent => {
                self.bump();
                Ok((tok.value(self.src), tok.span))
            }
            Some(tok)
                if tok.kind == TokenKind::Ident
                    && !tok.keyword(self.src).is_some_and(Keyword::is_reserved) =>
            {
                self.bump();
                Ok((self.text(&tok).to_string(), tok.span))
            }
            _ => Err(self.unexpected(&["table name"])),
        }
    }

    /// `Table[Name]` as used by `DEFINE MEASURE` / `DEFINE COLUMN`.
    fn expect_qualified_name(&mut self) -> PResult<(String, String)> {
        let (table, _) = self.expect_table_name()?;
        let name = self.expect(TokenKind::BracketIdent, "'[name]'")?;
        Ok((table, name.value(self.src)))
    }

    // ---- user-defined functions ------------------------------------------

    fn parse_function_def(&mut self) -> PResult<FunctionDef> {
        let open = self.expect(TokenKind::LParen, "'('")?;
        let mut params = Vec::new();
        if !self.at(TokenKind::RParen) {
            loop {
                let (name, name_span) = self.expect_name("parameter name")?;
                let mut span = name_span;
                let mut type_hint = None;
                if self.eat(TokenKind::Colon).is_some() {
                    let mut words = Vec::new();
                    while let Some(word) = self.eat(TokenKind::Ident) {
                        words.push(self.text(&word).to_string());
                        span = span.to(word.span);
                    }
                    if words.is_empty() {
                        return Err(self.unexpected(&["parameter type"]));
                    }
                    type_hint = Some(words.join(" "));
                }
                params.push(FunctionParam {
                    name,
                    type_hint,
                    span,
                });
                if self.eat(TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        self.expect_one_of(TokenKind::RParen, &["','", "')'"])?;
        self.expect(TokenKind::Arrow, "'=>'")?;
        self.scopes
            .push(params.iter().map(|p| p.name.to_lowercase()).collect());
        let body = self.parse_expr(0);
        self.scopes.pop();
        let body = *body?;
        let span = open.span.to(body.span);
        Ok(FunctionDef { params, body, span })
    }

    // ---- queries -----------------------------------------------------------

    fn parse_query(&mut self) -> PResult<Query> {
        let start = self.current_span().start;
        let mut defines = Vec::new();
        if self.eat_keyword(Keyword::Define).is_some() {
            while !self.at_keyword(Keyword::Evaluate) {
                defines.push(self.parse_define()?);
            }
            if defines.is_empty() {
                return Err(self.unexpected(&["MEASURE", "VAR", "TABLE", "COLUMN", "FUNCTION"]));
            }
        }
        let mut evaluates = Vec::new();
        while self.at_keyword(Keyword::Evaluate) {
            evaluates.push(self.parse_evaluate()?);
        }
        if evaluates.is_empty() {
            let expected: &[&str] = if defines.is_empty() {
                &["DEFINE", "EVALUATE"]
            } else {
                &["EVALUATE"]
            };
            return Err(self.unexpected(expected));
        }
        let span = Span::new(start, self.prev_end());
        Ok(Query {
            defines,
            evaluates,
            span,
        })
    }

    fn parse_define(&mut self) -> PResult<Define> {
        let start = self.current_span().start;
        match self.peek_keyword() {
            Some(Keyword::Measure) => {
                self.bump();
                let (table, name) = self.expect_qualified_name()?;
                self.expect(TokenKind::Eq, "'='")?;
                let expr = *self.parse_expr(0)?;
                let span = Span::new(start, expr.span.end);
                Ok(Define::Measure {
                    table,
                    name,
                    expr,
                    span,
                })
            }
            Some(Keyword::Column) => {
                self.bump();
                let (table, name) = self.expect_qualified_name()?;
                self.expect(TokenKind::Eq, "'='")?;
                let expr = *self.parse_expr(0)?;
                let span = Span::new(start, expr.span.end);
                Ok(Define::Column {
                    table,
                    name,
                    expr,
                    span,
                })
            }
            Some(Keyword::Table) => {
                self.bump();
                let (name, _) = self.expect_table_name()?;
                self.expect(TokenKind::Eq, "'='")?;
                let expr = *self.parse_expr(0)?;
                let span = Span::new(start, expr.span.end);
                Ok(Define::Table { name, expr, span })
            }
            Some(Keyword::Var) => {
                self.bump();
                let (name, _) = self.expect_name("variable name")?;
                self.expect(TokenKind::Eq, "'='")?;
                let expr = *self.parse_expr(0)?;
                self.declare(&name);
                let span = Span::new(start, expr.span.end);
                Ok(Define::Var { name, expr, span })
            }
            Some(Keyword::Function) => {
                self.bump();
                let (name, _) = self.expect_table_name()?;
                self.expect(TokenKind::Eq, "'='")?;
                let def = self.parse_function_def()?;
                let span = Span::new(start, def.span.end);
                Ok(Define::Function {
                    name,
                    params: def.params,
                    body: def.body,
                    span,
                })
            }
            _ => {
                Err(self.unexpected(&["MEASURE", "VAR", "TABLE", "COLUMN", "FUNCTION", "EVALUATE"]))
            }
        }
    }

    fn parse_evaluate(&mut self) -> PResult<Evaluate> {
        let kw = self.expect_keyword(Keyword::Evaluate)?;
        let expr = *self.parse_expr(0)?;
        let mut order_by = Vec::new();
        let mut start_at = Vec::new();
        if self.eat_keyword(Keyword::Order).is_some() {
            self.expect_keyword(Keyword::By)?;
            loop {
                let key = *self.parse_expr(0)?;
                let dir = if self.eat_keyword(Keyword::Desc).is_some() {
                    SortDir::Desc
                } else {
                    self.eat_keyword(Keyword::Asc);
                    SortDir::Asc
                };
                order_by.push((key, dir));
                if self.eat(TokenKind::Comma).is_none() {
                    break;
                }
            }
            if self.eat_keyword(Keyword::Start).is_some() {
                self.expect_keyword(Keyword::At)?;
                loop {
                    start_at.push(*self.parse_expr(0)?);
                    if self.eat(TokenKind::Comma).is_none() {
                        break;
                    }
                }
            }
        } else if self.at_keyword(Keyword::Start) {
            return Err(Box::new(
                ParseError::new(
                    "START AT requires a preceding ORDER BY",
                    self.current_span(),
                )
                .with_expected(["ORDER BY"]),
            ));
        }
        let span = Span::new(kw.span.start, self.prev_end());
        Ok(Evaluate {
            expr,
            order_by,
            start_at,
            span,
        })
    }
}

// ---- node builders ---------------------------------------------------------
//
// Kept out of line so their (large, unoptimised-build) temporaries never sit
// in the frames of the recursive descent functions.

#[inline(never)]
fn make_node(kind: ExprKind, span: Span) -> BExpr {
    Box::new(Expr::new(kind, span))
}

#[inline(never)]
fn make_binary(op: BinaryOp, lhs: BExpr, rhs: BExpr) -> BExpr {
    let span = lhs.span.to(rhs.span);
    make_node(ExprKind::Binary { op, lhs, rhs }, span)
}

#[inline(never)]
fn make_unary(op: UnaryOp, start: usize, operand: BExpr) -> BExpr {
    let span = Span::new(start, operand.span.end);
    make_node(ExprKind::Unary { op, expr: operand }, span)
}

#[inline(never)]
fn make_paren(inner: BExpr, span: Span) -> BExpr {
    make_node(ExprKind::Paren(inner), span)
}

#[inline(never)]
fn make_call(src: &str, name_span: Span, args: Vec<Expr>, close: Span) -> BExpr {
    let raw_name = name_span.text(src).to_string();
    make_node(
        ExprKind::Call {
            name: raw_name.to_uppercase(),
            raw_name,
            args,
        },
        name_span.to(close),
    )
}

#[inline(never)]
fn make_var(start: usize, bindings: Vec<VarBinding>, body: BExpr) -> BExpr {
    let span = Span::new(start, body.span.end);
    make_node(ExprKind::Var { bindings, body }, span)
}

// Boxed on purpose: the caller's frame only holds the 8-byte pointer.
#[allow(clippy::boxed_local)]
#[inline(never)]
fn push_expr(items: &mut Vec<Expr>, item: BExpr) {
    items.push(*item);
}

#[inline(never)]
fn push_omitted(items: &mut Vec<Expr>, at: usize) {
    items.push(Expr::new(ExprKind::Omitted, Span::empty(at)));
}

/// A parenthesised list inside `{ … }` is a multi-column row.
#[allow(clippy::boxed_local)]
#[inline(never)]
fn push_row(rows: &mut Vec<Vec<Expr>>, item: BExpr) {
    match item.kind {
        ExprKind::Tuple(values) => rows.push(values),
        _ => rows.push(vec![*item]),
    }
}

fn parse_number(text: &str) -> Option<Number> {
    if text.bytes().all(|b| b.is_ascii_digit())
        && let Ok(i) = text.parse::<i64>()
    {
        return Some(Number::Integer(i));
    }
    // Normalise `.5`, `1.` and `1.e3` into forms `f64::from_str` accepts.
    let mut normalized = String::with_capacity(text.len() + 2);
    let mut prev = None;
    for c in text.chars() {
        if c == '.' && prev.is_none() {
            normalized.push('0');
        }
        if matches!(c, 'e' | 'E') && prev == Some('.') {
            normalized.push('0');
        }
        normalized.push(c);
        prev = Some(c);
    }
    if prev == Some('.') {
        normalized.push('0');
    }
    normalized.parse::<f64>().ok().map(Number::Decimal)
}
