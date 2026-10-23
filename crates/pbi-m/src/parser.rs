//! Recursive-descent / operator-precedence parser for Power Query M.
//!
//! Implements the syntactic grammar of the M language specification (expression and section
//! documents). Binary operators are parsed with an explicit operator stack (no recursion per
//! operator), `else if` chains iteratively, and all remaining recursion is bounded by
//! [`MAX_NESTING`]; the depth of the produced tree is bounded by [`MAX_AST_DEPTH`]. Malformed or
//! hostile input therefore yields a [`ParseError`] rather than a stack overflow.
//!
//! Operator precedence (lowest to highest): `??`; `or`; `and`; `is`; `as`; `=` `<>`;
//! `<` `>` `<=` `>=`; `+` `-` `&`; `*` `/`; `meta`; unary `+` `-` `not`; `type`; primary
//! expressions with postfix invocation `f(x)`, item access `x{i}` and field access `x[f]`.
//! `let`, `if`, `each`, function literals, `try` and `error` are full expressions and need
//! parentheses when used as operands, as in the specification.

use std::collections::HashSet;

use crate::ast::{
    BinaryOp, Binding, Document, Expr, ExprKind, FieldSpec, FunctionType, Intrinsic, ListItem,
    Literal, Name, NullablePrimitiveType, NumberLiteral, Param, ParamSpec, PrimitiveType,
    RecordField, RecordType, Section, SectionMember, TableRowType, TryHandler, TypeExpr, TypeKind,
    UnaryOp, precedence,
};
use crate::error::ParseError;
use crate::lexer::{Comment, Keyword, Token, TokenKind, generalized_identifier_end, tokenize};
use crate::span::{Span, line_col};

/// Maximum syntactic nesting (parentheses, brackets, nested expressions, types) before the
/// parser reports an error instead of recursing further.
pub const MAX_NESTING: usize = 256;

/// Maximum depth of the produced syntax tree (including long left-associative operator chains,
/// postfix chains and `else if` chains). Consumers walking the tree recursively can rely on it.
pub const MAX_AST_DEPTH: u32 = 512;

/// Parses an M expression document (e.g. a query or partition source).
pub fn parse_expression(src: &str) -> Result<Expr, ParseError> {
    let lexed = tokenize(src)?;
    let mut p = Parser::new(src, lexed.tokens);
    let (expr, _) = p.parse_expr().map_err(|e| *e)?;
    p.expect_eof().map_err(|e| *e)?;
    Ok(*expr)
}

/// Parses an M document: a section document (`section S; shared x = ...;`) or an expression.
pub fn parse_document(src: &str) -> Result<Document, ParseError> {
    parse_document_with_comments(src).map(|d| d.document)
}

/// A parsed document together with its comments (trivia), for formatters.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedDocument {
    pub document: Document,
    pub comments: Vec<Comment>,
}

/// Like [`parse_document`], also returning the comments found in the source.
pub fn parse_document_with_comments(src: &str) -> Result<ParsedDocument, ParseError> {
    let lexed = tokenize(src)?;
    let mut p = Parser::new(src, lexed.tokens);
    let document = p.parse_document().map_err(|e| *e)?;
    Ok(ParsedDocument {
        document,
        comments: lexed.comments,
    })
}

type PResult<T> = Result<T, Box<ParseError>>;
/// An expression together with the depth of its tree.
type Node = (Box<Expr>, u32);
type TypeNode = (Box<TypeExpr>, u32);

fn too_deep(span: Span) -> Box<ParseError> {
    Box::new(ParseError::new(
        format!(
            "expression is nested too deeply (limit {MAX_NESTING} levels of nesting, \
             {MAX_AST_DEPTH} levels of operators)"
        ),
        span,
    ))
}

fn describe_list(items: &[String]) -> String {
    match items {
        [] => "something else".to_owned(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} or {last}", init.join(", ")),
    }
}

/// The binary-level operators recognised in [`Parser::parse_binary`].
#[derive(Clone, Copy)]
enum InfixOp {
    Binary(BinaryOp),
    As,
    Is,
}

impl InfixOp {
    fn precedence(self) -> u8 {
        match self {
            InfixOp::Binary(b) => b.precedence(),
            InfixOp::As => precedence::AS,
            InfixOp::Is => precedence::IS,
        }
    }
}

/// Operand and operator stacks of [`Parser::parse_binary`].
struct BinaryState {
    operands: Vec<Node>,
    ops: Vec<BinaryOp>,
    /// Highest operator precedence allowed next (lowered after `as`/`is`).
    ceiling: u8,
}

struct Parser<'a> {
    src: &'a str,
    tokens: Vec<Token>,
    eof: Token,
    pos: usize,
    nesting: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str, tokens: Vec<Token>) -> Self {
        let eof = Token {
            kind: TokenKind::Eof,
            span: Span::new(src.len(), src.len()),
        };
        Self {
            src,
            tokens,
            eof,
            pos: 0,
            nesting: 0,
        }
    }

    // ----------------------------------------------------------------------------------------
    // Token helpers
    // ----------------------------------------------------------------------------------------

    fn tok(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&self.eof)
    }

    fn peek_tok(&self, n: usize) -> &Token {
        self.tokens.get(self.pos + n).unwrap_or(&self.eof)
    }

    fn kind(&self) -> &TokenKind {
        &self.tok().kind
    }

    fn peek_kind(&self, n: usize) -> &TokenKind {
        &self.peek_tok(n).kind
    }

    fn at(&self, kind: &TokenKind) -> bool {
        self.kind() == kind
    }

    fn at_kw(&self, kw: Keyword) -> bool {
        matches!(self.kind(), TokenKind::Keyword(k) if *k == kw)
    }

    fn at_ident(&self, text: &str) -> bool {
        matches!(self.kind(), TokenKind::Identifier(t) if t == text)
    }

    /// Consumes the current token (never moves past end of input) and returns its span.
    fn advance(&mut self) -> Span {
        let span = self.tok().span;
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        span
    }

    fn eat(&mut self, kind: &TokenKind) -> Option<Span> {
        self.at(kind).then(|| self.advance())
    }

    fn eat_kw(&mut self, kw: Keyword) -> Option<Span> {
        self.at_kw(kw).then(|| self.advance())
    }

    fn unexpected(&self, expected: &[&str]) -> Box<ParseError> {
        let expected: Vec<String> = expected.iter().map(|s| (*s).to_owned()).collect();
        let tok = self.tok();
        Box::new(
            ParseError::new(
                format!(
                    "expected {}, found {}",
                    describe_list(&expected),
                    tok.kind.describe()
                ),
                tok.span,
            )
            .with_expected(expected),
        )
    }

    fn expect(&mut self, kind: &TokenKind, expected: &[&str]) -> PResult<Span> {
        self.eat(kind).ok_or_else(|| self.unexpected(expected))
    }

    fn expect_kw(&mut self, kw: Keyword, expected: &[&str]) -> PResult<Span> {
        self.eat_kw(kw).ok_or_else(|| self.unexpected(expected))
    }

    /// Expects the closing bracket `close` for the opener at `open`; `also` lists other tokens
    /// that would have been accepted (e.g. `` `,` ``).
    fn expect_close(&mut self, close: &TokenKind, open: Span, also: &[&str]) -> PResult<Span> {
        if let Some(span) = self.eat(close) {
            return Ok(span);
        }
        let close_str = format!("`{}`", close.punct_str().unwrap_or("?"));
        let open_str = open.slice(self.src);
        let mut expected: Vec<String> = also.iter().map(|s| (*s).to_owned()).collect();
        expected.push(close_str.clone());
        let (line, col) = line_col(self.src, open.start);
        let tok = self.tok();
        Err(Box::new(
            ParseError::new(
                format!(
                    "expected {} (to close `{open_str}` at line {line}, column {col}), found {}",
                    describe_list(&expected),
                    tok.kind.describe()
                ),
                tok.span,
            )
            .with_expected(expected),
        ))
    }

    fn expect_eof(&mut self) -> PResult<()> {
        if self.at(&TokenKind::Eof) {
            Ok(())
        } else {
            Err(self.unexpected(&["end of input"]))
        }
    }

    fn prev_end(&self) -> usize {
        self.pos
            .checked_sub(1)
            .and_then(|i| self.tokens.get(i))
            .map_or(0, |t| t.span.end)
    }

    // ----------------------------------------------------------------------------------------
    // Depth guards
    // ----------------------------------------------------------------------------------------

    fn enter(&mut self) -> PResult<()> {
        if self.nesting >= MAX_NESTING {
            return Err(too_deep(self.tok().span));
        }
        self.nesting += 1;
        Ok(())
    }

    fn leave(&mut self) {
        self.nesting = self.nesting.saturating_sub(1);
    }

    fn node(&self, kind: ExprKind, span: Span, child_depth: u32) -> PResult<Node> {
        let depth = child_depth.saturating_add(1);
        if depth > MAX_AST_DEPTH {
            return Err(too_deep(span));
        }
        Ok((Box::new(Expr::new(kind, span)), depth))
    }

    fn type_node(&self, kind: TypeKind, span: Span, child_depth: u32) -> PResult<TypeNode> {
        let depth = child_depth.saturating_add(1);
        if depth > MAX_AST_DEPTH {
            return Err(too_deep(span));
        }
        Ok((Box::new(TypeExpr { kind, span }), depth))
    }

    // ----------------------------------------------------------------------------------------
    // Names
    // ----------------------------------------------------------------------------------------

    /// `identifier`: a regular or quoted identifier (let variables, parameters, sections).
    fn parse_identifier_name(&mut self, what: &str) -> PResult<Name> {
        let (text, quoted) = match self.kind() {
            TokenKind::Identifier(s) => (s.clone(), false),
            TokenKind::QuotedIdentifier(s) => (s.clone(), true),
            _ => return Err(self.unexpected(&[what])),
        };
        let span = self.advance();
        Ok(Name { text, span, quoted })
    }

    /// Index just past the field name starting at token `pos + offset`, if there is one.
    fn field_name_end(&self, offset: usize) -> Option<usize> {
        let start = self.pos + offset;
        match &self.tokens.get(start)?.kind {
            TokenKind::QuotedIdentifier(_) => Some(start + 1),
            _ => generalized_identifier_end(self.src, &self.tokens, start).map(|(end, _)| end),
        }
    }

    /// `field-name`: a generalized identifier (`Order Ship Date`) or quoted identifier.
    fn parse_field_name(&mut self) -> PResult<Name> {
        if let TokenKind::QuotedIdentifier(s) = self.kind() {
            let text = s.clone();
            let span = self.advance();
            return Ok(Name {
                text,
                span,
                quoted: true,
            });
        }
        match generalized_identifier_end(self.src, &self.tokens, self.pos) {
            Some((end, span)) => {
                self.pos = end;
                Ok(Name {
                    text: span.slice(self.src).to_owned(),
                    span,
                    quoted: false,
                })
            }
            None => Err(self.unexpected(&["field name"])),
        }
    }

    fn check_unique(seen: &mut HashSet<String>, name: &Name, what: &str) -> PResult<()> {
        if seen.insert(name.text.clone()) {
            Ok(())
        } else {
            Err(Box::new(ParseError::new(
                format!("{what} `{}` is defined more than once", name.text),
                name.span,
            )))
        }
    }
}

// --------------------------------------------------------------------------------------------
// Expressions
// --------------------------------------------------------------------------------------------

impl Parser<'_> {
    /// `expression`
    ///
    /// Functions on the recursion path (`parse_expr` → `parse_binary` → `parse_unary` →
    /// `parse_postfix` → `parse_primary` → ...) are kept small, with construct-specific work in
    /// separate functions, so that unoptimized builds use little stack per nesting level.
    fn parse_expr(&mut self) -> PResult<Node> {
        self.enter()?;
        let result = match self.kind() {
            TokenKind::Keyword(Keyword::Let) => self.parse_let(),
            TokenKind::Keyword(Keyword::If) => self.parse_if(),
            TokenKind::Keyword(Keyword::Each) => self.parse_each_or_error(true),
            TokenKind::Keyword(Keyword::Error) => self.parse_each_or_error(false),
            TokenKind::Keyword(Keyword::Try) => self.parse_try(),
            TokenKind::LParen if self.is_function_start() => self.parse_function(),
            _ => self.parse_binary(),
        };
        self.leave();
        result
    }

    /// `each body` / `error body`
    fn parse_each_or_error(&mut self, each: bool) -> PResult<Node> {
        let start = self.advance();
        let (body, d) = self.parse_expr()?;
        let span = start.to(body.span);
        let kind = if each {
            ExprKind::Each(body)
        } else {
            ExprKind::Error(body)
        };
        self.node(kind, span, d)
    }

    /// `let a = 1, b = 2 in body`
    fn parse_let(&mut self) -> PResult<Node> {
        let start = self.advance();
        let mut bindings = Vec::new();
        let mut seen = HashSet::new();
        let mut depth = 0;
        loop {
            let name = self.parse_identifier_name("variable name")?;
            Self::check_unique(&mut seen, &name, "variable")?;
            self.expect(&TokenKind::Equal, &["`=`"])?;
            let (value, d) = self.parse_expr()?;
            depth = depth.max(d);
            push_binding(&mut bindings, name, value);
            if self.eat(&TokenKind::Comma).is_none() {
                break;
            }
        }
        self.expect_kw(Keyword::In, &["`,`", "`in`"])?;
        let (body, d) = self.parse_expr()?;
        let span = start.to(body.span);
        self.node(ExprKind::Let { bindings, body }, span, depth.max(d))
    }

    /// `if c then t else f`; `else if` chains are parsed iteratively.
    fn parse_if(&mut self) -> PResult<Node> {
        let mut heads = Vec::new();
        loop {
            let start = self.advance();
            let (condition, cd) = self.parse_expr()?;
            self.expect_kw(Keyword::Then, &["`then`"])?;
            let (then_branch, td) = self.parse_expr()?;
            self.expect_kw(Keyword::Else, &["`else`"])?;
            heads.push((start, condition, then_branch, cd.max(td)));
            if heads.len() > MAX_AST_DEPTH as usize {
                return Err(too_deep(start));
            }
            if !self.at_kw(Keyword::If) {
                break;
            }
        }
        let (mut acc, mut depth) = self.parse_expr()?;
        while let Some((start, condition, then_branch, d)) = heads.pop() {
            let span = start.to(acc.span);
            (acc, depth) = self.node(
                ExprKind::If {
                    condition,
                    then_branch,
                    else_branch: acc,
                },
                span,
                d.max(depth),
            )?;
        }
        Ok((acc, depth))
    }

    /// `try x`, `try x otherwise y`, `try x catch (e) => y`
    fn parse_try(&mut self) -> PResult<Node> {
        let start = self.advance();
        let (expr, mut depth) = self.parse_expr()?;
        let mut end = expr.span;
        let handler = if self.eat_kw(Keyword::Otherwise).is_some() {
            let (default, d) = self.parse_expr()?;
            depth = depth.max(d);
            end = default.span;
            Some(TryHandler::Otherwise(default))
        } else if self.at_ident("catch") {
            self.advance();
            let open = self.expect(&TokenKind::LParen, &["`(`"])?;
            let param = match self.kind() {
                TokenKind::Identifier(_) | TokenKind::QuotedIdentifier(_) => {
                    Some(self.parse_identifier_name("parameter name")?)
                }
                _ => None,
            };
            self.expect_close(&TokenKind::RParen, open, &[])?;
            self.expect(&TokenKind::FatArrow, &["`=>`"])?;
            let (body, d) = self.parse_expr()?;
            depth = depth.max(d);
            end = body.span;
            Some(TryHandler::Catch { param, body })
        } else {
            None
        };
        self.node(ExprKind::Try { expr, handler }, start.to(end), depth)
    }

    /// Bounded lookahead at `(`: is this a function expression `(params) as T => body`?
    fn is_function_start(&self) -> bool {
        let mut i = 1;
        loop {
            match self.peek_kind(i) {
                TokenKind::RParen => break,
                TokenKind::Identifier(_)
                | TokenKind::QuotedIdentifier(_)
                | TokenKind::Comma
                | TokenKind::Keyword(Keyword::As | Keyword::Null | Keyword::Type) => i += 1,
                _ => return false,
            }
        }
        i += 1;
        match self.peek_kind(i) {
            TokenKind::FatArrow => true,
            TokenKind::Keyword(Keyword::As) => {
                i += 1;
                if matches!(self.peek_kind(i), TokenKind::Identifier(s) if s == "nullable") {
                    i += 1;
                }
                matches!(
                    self.peek_kind(i),
                    TokenKind::Identifier(_) | TokenKind::Keyword(Keyword::Null | Keyword::Type)
                ) && *self.peek_kind(i + 1) == TokenKind::FatArrow
            }
            _ => false,
        }
    }

    /// `(a as text, optional b) as number => body`
    fn parse_function(&mut self) -> PResult<Node> {
        let open = self.advance();
        let mut params = Vec::new();
        let mut seen = HashSet::new();
        let mut seen_optional = false;
        if !self.at(&TokenKind::RParen) {
            loop {
                let optional = self.at_ident("optional")
                    && matches!(
                        self.peek_kind(1),
                        TokenKind::Identifier(_) | TokenKind::QuotedIdentifier(_)
                    );
                if optional {
                    self.advance();
                }
                let name = self.parse_identifier_name("parameter name")?;
                if seen_optional && !optional {
                    return Err(Box::new(ParseError::new(
                        format!(
                            "required parameter `{}` cannot follow an optional parameter",
                            name.text
                        ),
                        name.span,
                    )));
                }
                seen_optional |= optional;
                Self::check_unique(&mut seen, &name, "parameter")?;
                let ty = match self.eat_kw(Keyword::As) {
                    Some(_) => Some(self.parse_nullable_primitive()?),
                    None => None,
                };
                params.push(Param { name, optional, ty });
                if self.eat(&TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        self.expect_close(&TokenKind::RParen, open, &["`,`"])?;
        let return_type = match self.eat_kw(Keyword::As) {
            Some(_) => Some(self.parse_nullable_primitive()?),
            None => None,
        };
        self.expect(&TokenKind::FatArrow, &["`=>`"])?;
        let (body, d) = self.parse_expr()?;
        let span = open.to(body.span);
        self.node(
            ExprKind::Function {
                params,
                return_type,
                body,
            },
            span,
            d,
        )
    }

    /// `nullable? primitive-type` (after `as`/`is`, and in function-expression signatures).
    fn parse_nullable_primitive(&mut self) -> PResult<NullablePrimitiveType> {
        let start = self.tok().span;
        let is_primitive = |k: &TokenKind| match k {
            TokenKind::Identifier(s) => PrimitiveType::from_name(s).is_some(),
            TokenKind::Keyword(Keyword::Null | Keyword::Type) => true,
            _ => false,
        };
        let nullable = self.at_ident("nullable") && is_primitive(self.peek_kind(1));
        if nullable {
            self.advance();
        }
        let primitive = match self.kind() {
            TokenKind::Identifier(s) => PrimitiveType::from_name(s),
            TokenKind::Keyword(Keyword::Null) => Some(PrimitiveType::Null),
            TokenKind::Keyword(Keyword::Type) => Some(PrimitiveType::Type),
            _ => None,
        };
        let Some(primitive) = primitive else {
            return Err(self.unexpected(&["primitive type"]));
        };
        let end = self.advance();
        Ok(NullablePrimitiveType {
            nullable,
            primitive,
            span: start.to(end),
        })
    }

    fn infix_op(&self) -> Option<InfixOp> {
        Some(match self.kind() {
            TokenKind::QuestionQuestion => InfixOp::Binary(BinaryOp::Coalesce),
            TokenKind::Keyword(Keyword::Or) => InfixOp::Binary(BinaryOp::Or),
            TokenKind::Keyword(Keyword::And) => InfixOp::Binary(BinaryOp::And),
            TokenKind::Keyword(Keyword::Is) => InfixOp::Is,
            TokenKind::Keyword(Keyword::As) => InfixOp::As,
            TokenKind::Equal => InfixOp::Binary(BinaryOp::Equal),
            TokenKind::NotEqual => InfixOp::Binary(BinaryOp::NotEqual),
            TokenKind::Less => InfixOp::Binary(BinaryOp::LessThan),
            TokenKind::LessEqual => InfixOp::Binary(BinaryOp::LessThanOrEqual),
            TokenKind::Greater => InfixOp::Binary(BinaryOp::GreaterThan),
            TokenKind::GreaterEqual => InfixOp::Binary(BinaryOp::GreaterThanOrEqual),
            TokenKind::Plus => InfixOp::Binary(BinaryOp::Add),
            TokenKind::Minus => InfixOp::Binary(BinaryOp::Subtract),
            TokenKind::Ampersand => InfixOp::Binary(BinaryOp::Concatenate),
            TokenKind::Star => InfixOp::Binary(BinaryOp::Multiply),
            TokenKind::Slash => InfixOp::Binary(BinaryOp::Divide),
            TokenKind::Keyword(Keyword::Meta) => InfixOp::Binary(BinaryOp::Meta),
            _ => return None,
        })
    }

    /// Binary operators from `??` down to `meta`, via an explicit operator stack.
    fn parse_binary(&mut self) -> PResult<Node> {
        let first = self.parse_unary()?;
        if self.infix_op().is_none() {
            return Ok(first);
        }
        self.parse_binary_rest(first)
    }

    fn parse_binary_rest(&mut self, first: Node) -> PResult<Node> {
        let mut st = BinaryState {
            operands: vec![first],
            ops: Vec::new(),
            ceiling: u8::MAX,
        };
        while let Some(op) = self.infix_op() {
            self.binary_step(&mut st, op)?;
        }
        self.reduce(&mut st, 0)?;
        st.operands
            .pop()
            .ok_or_else(|| Box::new(ParseError::new("missing operand", self.tok().span)))
    }

    fn binary_step(&mut self, st: &mut BinaryState, op: InfixOp) -> PResult<()> {
        let prec = op.precedence();
        if prec > st.ceiling {
            return Err(self.ceiling_error());
        }
        self.reduce(st, prec)?;
        self.advance();
        match op {
            InfixOp::Binary(b) => {
                st.ops.push(b);
                let rhs = self.parse_unary()?;
                st.operands.push(rhs);
                st.ceiling = u8::MAX;
                Ok(())
            }
            InfixOp::As | InfixOp::Is => self.apply_type_test(st, op),
        }
    }

    fn ceiling_error(&self) -> Box<ParseError> {
        let tok = self.tok();
        Box::new(ParseError::new(
            format!(
                "{} cannot follow an `as`/`is` type test without parentheses",
                tok.kind.describe()
            ),
            tok.span,
        ))
    }

    /// `x as T` / `x is T` applied to the top operand. After it, only operators of lower or
    /// equal precedence may follow (`x as number = 1` is a syntax error, as in the spec).
    fn apply_type_test(&mut self, st: &mut BinaryState, op: InfixOp) -> PResult<()> {
        let ty = self.parse_nullable_primitive()?;
        let (expr, d) = st
            .operands
            .pop()
            .ok_or_else(|| Box::new(ParseError::new("missing operand", ty.span)))?;
        let span = expr.span.to(ty.span);
        let kind = match op {
            InfixOp::As => ExprKind::As { expr, ty },
            _ => ExprKind::Is { expr, ty },
        };
        st.operands.push(self.node(kind, span, d)?);
        st.ceiling = op.precedence();
        Ok(())
    }

    /// Pops operators of precedence `>= min_prec` (left associativity) into binary nodes.
    fn reduce(&self, st: &mut BinaryState, min_prec: u8) -> PResult<()> {
        while let Some(&op) = st.ops.last() {
            if op.precedence() < min_prec {
                break;
            }
            st.ops.pop();
            let (Some((rhs, rd)), Some((lhs, ld))) = (st.operands.pop(), st.operands.pop()) else {
                return Err(Box::new(ParseError::new(
                    "missing operand",
                    self.tok().span,
                )));
            };
            let span = lhs.span.to(rhs.span);
            let kind = ExprKind::Binary { op, lhs, rhs };
            st.operands.push(self.node(kind, span, ld.max(rd))?);
        }
        Ok(())
    }

    /// `unary-expression`: `+x`, `-x`, `not x`, `type T`, or a primary expression.
    fn parse_unary(&mut self) -> PResult<Node> {
        match self.kind() {
            TokenKind::Plus | TokenKind::Minus | TokenKind::Keyword(Keyword::Not) => {
                self.parse_unary_operators()
            }
            TokenKind::Keyword(Keyword::Type) => self.parse_type_expression(),
            _ => self.parse_postfix(),
        }
    }

    fn parse_unary_operators(&mut self) -> PResult<Node> {
        let mut ops = Vec::new();
        loop {
            let op = match self.kind() {
                TokenKind::Plus => UnaryOp::Plus,
                TokenKind::Minus => UnaryOp::Minus,
                TokenKind::Keyword(Keyword::Not) => UnaryOp::Not,
                _ => break,
            };
            let span = self.advance();
            if ops.len() >= MAX_AST_DEPTH as usize {
                return Err(too_deep(span));
            }
            ops.push((op, span));
        }
        let mut node = if self.at_kw(Keyword::Type) {
            self.parse_type_expression()?
        } else {
            self.parse_postfix()?
        };
        while let Some((op, span)) = ops.pop() {
            let (operand, depth) = node;
            let span = span.to(operand.span);
            let kind = ExprKind::Unary { op, operand };
            node = self.node(kind, span, depth)?;
        }
        Ok(node)
    }

    /// `type T`
    fn parse_type_expression(&mut self) -> PResult<Node> {
        let start = self.advance();
        let (ty, d) = self.parse_type(false)?;
        let span = start.to(ty.span);
        self.node(ExprKind::Type(ty), span, d)
    }

    /// A primary expression followed by any number of `(args)`, `{item}`, `[field]`,
    /// `[[projection]]` (each optionally followed by `?` where allowed).
    fn parse_postfix(&mut self) -> PResult<Node> {
        let mut node = self.parse_primary()?;
        loop {
            node = match self.kind() {
                TokenKind::LParen => self.parse_invoke(node)?,
                TokenKind::LBrace => self.parse_item_access(node)?,
                TokenKind::LBracket => self.parse_field_access(Some(node))?,
                _ => return Ok(node),
            };
        }
    }

    /// `f(args)`
    fn parse_invoke(&mut self, (function, depth): Node) -> PResult<Node> {
        let open = self.advance();
        let mut args = Vec::new();
        let mut d = depth;
        if !self.at(&TokenKind::RParen) {
            loop {
                let (arg, ad) = self.parse_expr()?;
                d = d.max(ad);
                args.push(*arg);
                if self.eat(&TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        let close = self.expect_close(&TokenKind::RParen, open, &["`,`"])?;
        let span = function.span.to(close);
        self.node(ExprKind::Invoke { function, args }, span, d)
    }

    /// `target{selector}` / `target{selector}?`
    fn parse_item_access(&mut self, (target, depth): Node) -> PResult<Node> {
        let open = self.advance();
        let (selector, sd) = self.parse_expr()?;
        let close = self.expect_close(&TokenKind::RBrace, open, &[])?;
        let (optional, end) = self.eat_optional_marker(close);
        let span = target.span.to(end);
        let kind = ExprKind::ItemAccess {
            target,
            selector,
            optional,
        };
        self.node(kind, span, depth.max(sd))
    }

    /// Consumes a trailing `?` (optional access) if present; returns it and the new end span.
    fn eat_optional_marker(&mut self, end: Span) -> (bool, Span) {
        match self.eat(&TokenKind::Question) {
            Some(q) => (true, q),
            None => (false, end),
        }
    }

    /// At `[`: `[name]`, `[name]?`, `[[a], [b]]`, `[[a]]?`, applied to `target` (or the implicit
    /// `_` when `target` is `None`).
    fn parse_field_access(&mut self, target: Option<Node>) -> PResult<Node> {
        let open = self.advance();
        let (target, depth, start) = match target {
            Some((e, d)) => {
                let start = e.span;
                (Some(e), d, start)
            }
            None => (None, 0, open),
        };
        if self.at(&TokenKind::LBracket) {
            let mut fields = Vec::new();
            loop {
                let inner = self.expect(&TokenKind::LBracket, &["`[`"])?;
                fields.push(self.parse_field_name()?);
                self.expect_close(&TokenKind::RBracket, inner, &[])?;
                if self.eat(&TokenKind::Comma).is_none() {
                    break;
                }
            }
            let close = self.expect_close(&TokenKind::RBracket, open, &["`,`"])?;
            let (optional, end) = self.eat_optional_marker(close);
            let kind = ExprKind::FieldProjection {
                target,
                fields,
                optional,
            };
            return self.node(kind, start.to(end), depth);
        }
        let field = self.parse_field_name()?;
        let close = self.expect_close(&TokenKind::RBracket, open, &[])?;
        let (optional, end) = self.eat_optional_marker(close);
        let kind = ExprKind::FieldAccess {
            target,
            field,
            optional,
        };
        self.node(kind, start.to(end), depth)
    }
}

// --------------------------------------------------------------------------------------------
// Primary expressions
// --------------------------------------------------------------------------------------------

fn intrinsic_of(kw: Keyword) -> Option<Intrinsic> {
    Some(match kw {
        Keyword::HashBinary => Intrinsic::Binary,
        Keyword::HashDate => Intrinsic::Date,
        Keyword::HashDateTime => Intrinsic::DateTime,
        Keyword::HashDateTimeZone => Intrinsic::DateTimeZone,
        Keyword::HashDuration => Intrinsic::Duration,
        Keyword::HashInfinity => Intrinsic::Infinity,
        Keyword::HashNan => Intrinsic::Nan,
        Keyword::HashSections => Intrinsic::Sections,
        Keyword::HashShared => Intrinsic::Shared,
        Keyword::HashTable => Intrinsic::Table,
        Keyword::HashTime => Intrinsic::Time,
        _ => return None,
    })
}

impl Parser<'_> {
    /// Literal tokens (`null`, logical, number, text, verbatim), if the current token is one.
    fn literal_here(&self) -> Option<Literal> {
        let tok = self.tok();
        Some(match &tok.kind {
            TokenKind::Number(value) => Literal::Number(NumberLiteral {
                raw: tok.span.slice(self.src).to_owned(),
                value: *value,
            }),
            TokenKind::Text(s) => Literal::Text(s.clone()),
            TokenKind::Verbatim(s) => Literal::Verbatim(s.clone()),
            TokenKind::Keyword(Keyword::True) => Literal::Logical(true),
            TokenKind::Keyword(Keyword::False) => Literal::Logical(false),
            TokenKind::Keyword(Keyword::Null) => Literal::Null,
            _ => return None,
        })
    }

    fn parse_primary(&mut self) -> PResult<Node> {
        match self.kind() {
            TokenKind::LParen => self.parse_parenthesized(),
            TokenKind::LBrace => self.parse_list(),
            TokenKind::LBracket => self.parse_bracket_primary(),
            TokenKind::Identifier(_) | TokenKind::QuotedIdentifier(_) | TokenKind::At => {
                self.parse_identifier_expression()
            }
            _ => self.parse_atom(),
        }
    }

    /// Literals, `#` keywords and `...`.
    fn parse_atom(&mut self) -> PResult<Node> {
        if let Some(lit) = self.literal_here() {
            let span = self.advance();
            return self.node(ExprKind::Literal(lit), span, 0);
        }
        let kind = match self.kind() {
            TokenKind::Keyword(kw) => intrinsic_of(*kw).map(ExprKind::Intrinsic),
            TokenKind::Ellipsis => Some(ExprKind::NotImplemented),
            _ => None,
        };
        match kind {
            Some(kind) => {
                let span = self.advance();
                self.node(kind, span, 0)
            }
            None => Err(self.unexpected(&["expression"])),
        }
    }

    /// `x`, `@x`, `Section!Member`
    fn parse_identifier_expression(&mut self) -> PResult<Node> {
        let at = self.eat(&TokenKind::At);
        let name = self.parse_identifier_name("identifier")?;
        if at.is_none() && self.eat(&TokenKind::Bang).is_some() {
            let member = self.parse_identifier_name("section member name")?;
            let span = name.span.to(member.span);
            let kind = ExprKind::SectionAccess {
                section: name,
                member,
            };
            return self.node(kind, span, 0);
        }
        let span = at.map_or(name.span, |a| a.to(name.span));
        let kind = ExprKind::Identifier {
            name,
            inclusive: at.is_some(),
        };
        self.node(kind, span, 0)
    }

    /// `( expression )`
    fn parse_parenthesized(&mut self) -> PResult<Node> {
        let open = self.advance();
        let (inner, d) = self.parse_expr()?;
        let close = self.expect_close(&TokenKind::RParen, open, &[])?;
        self.node(ExprKind::Parenthesized(inner), open.to(close), d)
    }

    /// `{ item, a..b, ... }`
    fn parse_list(&mut self) -> PResult<Node> {
        let open = self.advance();
        let mut items = Vec::new();
        let mut depth = 0;
        if !self.at(&TokenKind::RBrace) {
            loop {
                let (first, end, d) = self.parse_list_item()?;
                depth = depth.max(d);
                push_list_item(&mut items, first, end);
                if self.eat(&TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        let close = self.expect_close(&TokenKind::RBrace, open, &["`,`", "`..`"])?;
        self.node(ExprKind::List(items), open.to(close), depth)
    }

    /// `item` or `start..end`.
    fn parse_list_item(&mut self) -> PResult<(Box<Expr>, Option<Box<Expr>>, u32)> {
        let (first, d1) = self.parse_expr()?;
        if self.eat(&TokenKind::DotDot).is_none() {
            return Ok((first, None, d1));
        }
        let (end, d2) = self.parse_expr()?;
        Ok((first, Some(end), d1.max(d2)))
    }

    /// At `[` in primary position: record `[a = 1]`, empty record `[]`, implicit field access
    /// `[a]` (meaning `_[a]`) or implicit projection `[[a], [b]]`.
    fn parse_bracket_primary(&mut self) -> PResult<Node> {
        let is_record = match self.peek_kind(1) {
            TokenKind::RBracket => true,
            TokenKind::LBracket => false,
            _ => self.field_name_end(1).is_some_and(|end| {
                self.tokens.get(end).map(|t| &t.kind) == Some(&TokenKind::Equal)
            }),
        };
        if is_record {
            self.parse_record()
        } else {
            self.parse_field_access(None)
        }
    }

    /// `[ name = value, ... ]`
    fn parse_record(&mut self) -> PResult<Node> {
        let open = self.advance();
        let mut fields = Vec::new();
        let mut seen = HashSet::new();
        let mut depth = 0;
        if !self.at(&TokenKind::RBracket) {
            loop {
                let name = self.parse_field_name()?;
                Self::check_unique(&mut seen, &name, "field")?;
                self.expect(&TokenKind::Equal, &["`=`"])?;
                let (value, d) = self.parse_expr()?;
                depth = depth.max(d);
                push_record_field(&mut fields, name, value);
                if self.eat(&TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        let close = self.expect_close(&TokenKind::RBracket, open, &["`,`"])?;
        self.node(ExprKind::Record(fields), open.to(close), depth)
    }
}

// Constructing large values in separate small functions keeps them out of the stack frames of
// the recursive parsing functions in unoptimized builds; the boxed arguments are deliberate, so
// that callers only move pointers.

#[allow(clippy::boxed_local)]
fn push_list_item(items: &mut Vec<ListItem>, first: Box<Expr>, end: Option<Box<Expr>>) {
    items.push(match end {
        None => ListItem::Single(*first),
        Some(end) => ListItem::Range {
            start: *first,
            end: *end,
        },
    });
}

#[allow(clippy::boxed_local)]
fn push_record_field(fields: &mut Vec<RecordField>, name: Name, value: Box<Expr>) {
    fields.push(RecordField {
        name,
        value: *value,
    });
}

#[allow(clippy::boxed_local)]
fn push_binding(bindings: &mut Vec<Binding>, name: Name, value: Box<Expr>) {
    bindings.push(Binding {
        name,
        value: *value,
    });
}

// --------------------------------------------------------------------------------------------
// Types
// --------------------------------------------------------------------------------------------

impl Parser<'_> {
    /// `primary-type` (when `allow_expr` is false, i.e. directly after the `type` keyword) or
    /// `type` (`primary-type` or a primary expression such as `Int64.Type`).
    fn parse_type(&mut self, allow_expr: bool) -> PResult<TypeNode> {
        self.enter()?;
        let result = self.parse_type_inner(allow_expr);
        self.leave();
        result
    }

    fn can_start_type(kind: &TokenKind) -> bool {
        match kind {
            TokenKind::LBracket
            | TokenKind::LBrace
            | TokenKind::LParen
            | TokenKind::At
            | TokenKind::Identifier(_)
            | TokenKind::QuotedIdentifier(_)
            | TokenKind::Keyword(Keyword::Null | Keyword::Type) => true,
            TokenKind::Keyword(k) => k.is_hash(),
            _ => false,
        }
    }

    fn parse_type_inner(&mut self, allow_expr: bool) -> PResult<TypeNode> {
        let start = self.tok().span;
        match self.kind() {
            TokenKind::LBracket => {
                let (record, d, span) = self.parse_record_type(true)?;
                return self.type_node(TypeKind::Record(record), span, d);
            }
            TokenKind::LBrace => {
                let open = self.advance();
                let (item, d) = self.parse_type(true)?;
                let close = self.expect_close(&TokenKind::RBrace, open, &[])?;
                return self.type_node(TypeKind::List(item), open.to(close), d);
            }
            TokenKind::Identifier(s) => {
                let next = self.peek_kind(1);
                match s.as_str() {
                    "table"
                        if matches!(
                            next,
                            TokenKind::LBracket
                                | TokenKind::LParen
                                | TokenKind::At
                                | TokenKind::Identifier(_)
                                | TokenKind::QuotedIdentifier(_)
                        ) =>
                    {
                        self.advance();
                        if self.at(&TokenKind::LBracket) {
                            let (record, d, span) = self.parse_record_type(false)?;
                            return self.type_node(
                                TypeKind::Table(TableRowType::Fields(record)),
                                start.to(span),
                                d,
                            );
                        }
                        let (row, d) = self.parse_postfix()?;
                        let span = start.to(row.span);
                        return self.type_node(TypeKind::Table(TableRowType::Expr(row)), span, d);
                    }
                    "function" if *next == TokenKind::LParen => return self.parse_function_type(),
                    "nullable" if Self::can_start_type(next) => {
                        self.advance();
                        let (inner, d) = self.parse_type(true)?;
                        let span = start.to(inner.span);
                        return self.type_node(TypeKind::Nullable(inner), span, d);
                    }
                    name => {
                        if let Some(p) = PrimitiveType::from_name(name) {
                            let span = self.advance();
                            return self.type_node(TypeKind::Primitive(p), span, 0);
                        }
                    }
                }
            }
            TokenKind::Keyword(Keyword::Null) => {
                let span = self.advance();
                return self.type_node(TypeKind::Primitive(PrimitiveType::Null), span, 0);
            }
            TokenKind::Keyword(Keyword::Type) => {
                let span = self.advance();
                return self.type_node(TypeKind::Primitive(PrimitiveType::Type), span, 0);
            }
            _ => {}
        }
        if !allow_expr {
            return Err(self.unexpected(&["type"]));
        }
        let (expr, d) = self.parse_postfix()?;
        let span = expr.span;
        self.type_node(TypeKind::Expr(expr), span, d)
    }

    /// `[ field-spec, ..., ...? ]` for record types (`allow_open`) and table row types.
    fn parse_record_type(&mut self, allow_open: bool) -> PResult<(RecordType, u32, Span)> {
        let open_span = self.advance();
        let mut fields = Vec::new();
        let mut seen = HashSet::new();
        let mut open = false;
        let mut depth = 0;
        if !self.at(&TokenKind::RBracket) {
            loop {
                if allow_open && self.at(&TokenKind::Ellipsis) {
                    self.advance();
                    open = true;
                    break;
                }
                // `optional` followed by a field name is the optional marker; otherwise (e.g.
                // `[optional = text]`) it is itself the field name.
                let optional = self.at_ident("optional")
                    && match self.peek_kind(1) {
                        TokenKind::Identifier(_)
                        | TokenKind::QuotedIdentifier(_)
                        | TokenKind::Number(_) => true,
                        TokenKind::Keyword(k) => !k.is_hash(),
                        _ => false,
                    };
                if optional {
                    self.advance();
                }
                let name = self.parse_field_name()?;
                Self::check_unique(&mut seen, &name, "field")?;
                let ty = if self.eat(&TokenKind::Equal).is_some() {
                    let (t, d) = self.parse_type(true)?;
                    depth = depth.max(d);
                    Some(t)
                } else {
                    None
                };
                fields.push(FieldSpec {
                    name,
                    optional,
                    ty: ty.map(|t| *t),
                });
                if self.eat(&TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        let also: &[&str] = if allow_open && !open {
            &["`,`", "`...`"]
        } else if open {
            &[]
        } else {
            &["`,`"]
        };
        let close = self.expect_close(&TokenKind::RBracket, open_span, also)?;
        Ok((RecordType { fields, open }, depth, open_span.to(close)))
    }

    /// `function (a as text, optional b as number) as text`
    fn parse_function_type(&mut self) -> PResult<TypeNode> {
        let start = self.advance();
        let open = self.expect(&TokenKind::LParen, &["`(`"])?;
        let mut params = Vec::new();
        let mut seen = HashSet::new();
        let mut seen_optional = false;
        let mut depth = 0;
        if !self.at(&TokenKind::RParen) {
            loop {
                let optional = self.at_ident("optional")
                    && matches!(
                        self.peek_kind(1),
                        TokenKind::Identifier(_) | TokenKind::QuotedIdentifier(_)
                    );
                if optional {
                    self.advance();
                }
                let name = self.parse_identifier_name("parameter name")?;
                if seen_optional && !optional {
                    return Err(Box::new(ParseError::new(
                        format!(
                            "required parameter `{}` cannot follow an optional parameter",
                            name.text
                        ),
                        name.span,
                    )));
                }
                seen_optional |= optional;
                Self::check_unique(&mut seen, &name, "parameter")?;
                let ty = if self.eat_kw(Keyword::As).is_some() {
                    let (t, d) = self.parse_type(true)?;
                    depth = depth.max(d);
                    Some(t)
                } else {
                    None
                };
                params.push(ParamSpec {
                    name,
                    optional,
                    ty: ty.map(|t| *t),
                });
                if self.eat(&TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        self.expect_close(&TokenKind::RParen, open, &["`,`"])?;
        self.expect_kw(Keyword::As, &["`as`"])?;
        let (return_type, d) = self.parse_type(true)?;
        let span = start.to(return_type.span);
        self.type_node(
            TypeKind::Function(FunctionType {
                params,
                return_type,
            }),
            span,
            depth.max(d),
        )
    }
}

// --------------------------------------------------------------------------------------------
// Documents
// --------------------------------------------------------------------------------------------

impl Parser<'_> {
    fn parse_document(&mut self) -> PResult<Document> {
        let start_pos = self.pos;
        let attributes = if self.at(&TokenKind::LBracket) {
            match self.parse_literal_record() {
                Ok((record, _)) if self.at_kw(Keyword::Section) => Some(*record),
                _ => {
                    self.pos = start_pos;
                    None
                }
            }
        } else {
            None
        };
        if !self.at_kw(Keyword::Section) {
            self.pos = start_pos;
            let (expr, _) = self.parse_expr()?;
            self.expect_eof()?;
            return Ok(Document::Expression(*expr));
        }
        let section_start = attributes.as_ref().map_or(self.tok().span, |a| a.span);
        self.advance();
        let name = self.parse_identifier_name("section name")?;
        self.expect(&TokenKind::Semicolon, &["`;`"])?;
        let mut members = Vec::new();
        let mut seen = HashSet::new();
        while !self.at(&TokenKind::Eof) {
            let member_start = self.tok().span;
            let attributes = if self.at(&TokenKind::LBracket) {
                Some(*self.parse_literal_record()?.0)
            } else {
                None
            };
            let shared = self.eat_kw(Keyword::Shared).is_some();
            let member_name = self.parse_identifier_name("section member name")?;
            Self::check_unique(&mut seen, &member_name, "section member")?;
            self.expect(&TokenKind::Equal, &["`=`"])?;
            let (value, _) = self.parse_expr()?;
            let end = self.expect(&TokenKind::Semicolon, &["`;`"])?;
            members.push(SectionMember {
                attributes,
                shared,
                name: member_name,
                value: *value,
                span: member_start.to(end),
            });
        }
        let span = Span::new(section_start.start, self.prev_end().max(section_start.end));
        Ok(Document::Section(Section {
            attributes,
            name,
            members,
            span,
        }))
    }

    /// `record-literal` (literal attributes): `[name = any-literal, ...]`.
    fn parse_literal_record(&mut self) -> PResult<Node> {
        self.enter()?;
        let result = self.parse_literal_record_inner();
        self.leave();
        result
    }

    fn parse_literal_record_inner(&mut self) -> PResult<Node> {
        let open = self.expect(&TokenKind::LBracket, &["`[`"])?;
        let mut fields = Vec::new();
        let mut seen = HashSet::new();
        let mut depth = 0;
        if !self.at(&TokenKind::RBracket) {
            loop {
                let name = self.parse_field_name()?;
                Self::check_unique(&mut seen, &name, "field")?;
                self.expect(&TokenKind::Equal, &["`=`"])?;
                let (value, d) = self.parse_any_literal()?;
                depth = depth.max(d);
                fields.push(RecordField {
                    name,
                    value: *value,
                });
                if self.eat(&TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        let close = self.expect_close(&TokenKind::RBracket, open, &["`,`"])?;
        self.node(ExprKind::Record(fields), open.to(close), depth)
    }

    fn parse_any_literal(&mut self) -> PResult<Node> {
        if let Some(lit) = self.literal_here()
            && !matches!(lit, Literal::Verbatim(_))
        {
            let span = self.advance();
            return self.node(ExprKind::Literal(lit), span, 0);
        }
        match self.kind() {
            TokenKind::LBracket => self.parse_literal_record(),
            TokenKind::LBrace => {
                self.enter()?;
                let result = self.parse_literal_list();
                self.leave();
                result
            }
            _ => Err(self.unexpected(&["literal"])),
        }
    }

    fn parse_literal_list(&mut self) -> PResult<Node> {
        let open = self.advance();
        let mut items = Vec::new();
        let mut depth = 0;
        if !self.at(&TokenKind::RBrace) {
            loop {
                let (item, d) = self.parse_any_literal()?;
                depth = depth.max(d);
                items.push(ListItem::Single(*item));
                if self.eat(&TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        let close = self.expect_close(&TokenKind::RBrace, open, &["`,`"])?;
        self.node(ExprKind::List(items), open.to(close), depth)
    }
}
