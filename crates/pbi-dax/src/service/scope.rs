//! Where variables and function parameters are visible, and the `DEFINE`
//! entries of a query, found on tokens so text that does not parse yet still
//! has them. Shared by completion, semantic tokens and references.

use std::collections::HashSet;

use super::schema::Schema;
use crate::syntax::{Keyword, Span, Token, TokenKind};

/// What a [`Decl`] declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeclKind {
    /// `VAR name = …`, in an expression or a query's `DEFINE`.
    Var,
    /// A parameter of a user-defined function: `(name : SCALAR) => …`.
    Param,
}

/// A declared variable or parameter and where it is visible.
#[derive(Debug, Clone)]
pub(super) struct Decl {
    /// As written.
    pub(super) name: String,
    pub(super) kind: DeclKind,
    /// The declared name.
    pub(super) span: Span,
    /// Visible from this offset; `None` when never (a `VAR` whose block
    /// has no next `VAR` or `RETURN` yet).
    from: Option<usize>,
    /// Visible up to this offset.
    to: usize,
    depth: usize,
}

impl Decl {
    /// Whether a name at `span` can refer to this declaration.
    pub(super) fn covers(&self, span: Span) -> bool {
        self.from.is_some_and(|f| f <= span.start) && span.end <= self.to
    }

    /// Whether this declaration is visible at a cursor `offset`.
    pub(super) fn visible_at(&self, offset: usize) -> bool {
        self.from.is_some_and(|f| f <= offset) && offset <= self.to
    }
}

/// The variables and parameters of a text.
#[derive(Debug, Clone, Default)]
pub(super) struct Scopes {
    /// In declaration order.
    pub(super) decls: Vec<Decl>,
    /// Starts of identifiers used as keywords where a name could stand: a
    /// parameter type (`SCALAR`, `TABLE`) and, in a query, the `MEASURE`,
    /// `COLUMN`, `TABLE` and `FUNCTION` that start a definition.
    pub(super) keywords: HashSet<usize>,
}

impl Scopes {
    /// The declarations visible at a cursor, one per name (the one declared
    /// last), in declaration order.
    pub(super) fn visible_at(&self, offset: usize) -> Vec<&Decl> {
        let mut seen = HashSet::new();
        let mut out: Vec<&Decl> = self
            .decls
            .iter()
            .filter(|d| d.visible_at(offset))
            .rev()
            .filter(|d| seen.insert(d.name.to_lowercase()))
            .collect();
        out.reverse();
        out
    }
}

pub(super) fn same_name(a: &str, b: &str) -> bool {
    a == b || a.to_lowercase() == b.to_lowercase()
}

/// Whether the text is a query: its first token is `DEFINE` or `EVALUATE`.
pub(super) fn is_query(src: &str, tokens: &[Token]) -> bool {
    tokens
        .iter()
        .find(|t| !t.is_trivia())
        .is_some_and(|t| matches!(t.keyword(src), Some(Keyword::Define | Keyword::Evaluate)))
}

/// Whether an expression can end with `t` (so a `VAR` after it continues a
/// block instead of starting one).
fn ends_operand(src: &str, t: &Token) -> bool {
    match t.kind {
        TokenKind::Ident => match t.keyword(src) {
            Some(Keyword::True | Keyword::False) | None => true,
            Some(k) => !k.is_reserved(),
        },
        TokenKind::QuotedIdent
        | TokenKind::BracketIdent
        | TokenKind::String
        | TokenKind::DateTime
        | TokenKind::Number
        | TokenKind::Parameter
        | TokenKind::RParen
        | TokenKind::RBrace => true,
        _ => false,
    }
}

/// A `VAR … RETURN …` block: its variables become visible one by one and
/// stay visible through the `RETURN` expression.
struct Block {
    depth: usize,
    decls: Vec<usize>,
    returned: bool,
}

struct Scan<'a> {
    src: &'a str,
    query: bool,
    decls: Vec<Decl>,
    /// Declarations whose end is not known yet (not the query-level ones,
    /// which reach the end of the text).
    open: Vec<usize>,
    blocks: Vec<Block>,
    /// Query-level `VAR`s waiting for the next definition.
    query_pending: Vec<usize>,
    depth: usize,
    /// Open `(` / `{`: index into the significant tokens.
    groups: Vec<usize>,
    /// The last closed group: its open and close index.
    last_group: Option<(usize, usize)>,
    keywords: HashSet<usize>,
}

/// The variables and parameters of `tokens` and where each is visible.
///
/// A `VAR` becomes visible at the next `VAR` or `RETURN` of its block (not
/// in its own expression) and stays visible through the `RETURN`
/// expression, which ends at the closing parenthesis or argument separator
/// around the block, or where an enclosing block continues. A parameter is
/// visible in the function body after `=>`. In a query, a `VAR` of the
/// `DEFINE` part is visible from the next definition to the end of the text,
/// and the variables and parameters of a definition end where the next one
/// (or `EVALUATE`, `ORDER BY`, `START AT`) starts.
pub(super) fn scopes(src: &str, tokens: &[Token], query: bool) -> Scopes {
    let sig: Vec<&Token> = tokens.iter().filter(|t| !t.is_trivia()).collect();
    let mut s = Scan {
        src,
        query,
        decls: Vec::new(),
        open: Vec::new(),
        blocks: Vec::new(),
        query_pending: Vec::new(),
        depth: 0,
        groups: Vec::new(),
        last_group: None,
        keywords: HashSet::new(),
    };
    for (k, t) in sig.iter().enumerate() {
        let prev = k.checked_sub(1).and_then(|p| sig.get(p)).copied();
        match t.kind {
            TokenKind::LParen | TokenKind::LBrace => {
                s.groups.push(k);
                s.depth += 1;
            }
            TokenKind::RParen | TokenKind::RBrace => {
                s.close_from(s.depth, t.span.start);
                s.depth = s.depth.saturating_sub(1);
                s.last_group = s.groups.pop().map(|open| (open, k));
            }
            TokenKind::Comma if s.depth > 0 => s.close_from(s.depth, t.span.start),
            TokenKind::Arrow => s.params(&sig, k, t),
            TokenKind::Ident => match t.keyword(src) {
                Some(Keyword::Var) => s.var(sig.get(k + 1).copied(), t, prev),
                Some(Keyword::Return) => s.ret(t),
                Some(Keyword::Define | Keyword::Evaluate) if query => {
                    // Never inside parentheses: unbalanced text before it
                    // does not leak into the next definition.
                    s.depth = 0;
                    s.groups.clear();
                    s.definition(t);
                }
                Some(Keyword::Order | Keyword::Start) if query && s.depth == 0 => {
                    s.definition(t);
                }
                Some(Keyword::Measure | Keyword::Column | Keyword::Table | Keyword::Function)
                    if query
                        && s.depth == 0
                        && prev.is_some_and(|p| {
                            p.keyword(src) == Some(Keyword::Define) || ends_operand(src, p)
                        }) =>
                {
                    s.keywords.insert(t.span.start);
                    s.definition(t);
                }
                _ => {}
            },
            _ => {}
        }
    }
    let end = src.len();
    for d in &mut s.decls {
        if d.to == usize::MAX {
            d.to = end;
        }
    }
    Scopes {
        decls: s.decls,
        keywords: s.keywords,
    }
}

impl Scan<'_> {
    fn declare(&mut self, name: &Token, kind: DeclKind, from: Option<usize>) -> usize {
        self.decls.push(Decl {
            name: name.text(self.src).to_string(),
            kind,
            span: name.span,
            from,
            to: usize::MAX,
            depth: self.depth,
        });
        self.decls.len() - 1
    }

    /// Ends every open declaration at `min_depth` or deeper at `at`.
    fn close_from(&mut self, min_depth: usize, at: usize) {
        let decls = &mut self.decls;
        self.open.retain(|&i| match decls.get_mut(i) {
            Some(d) if d.depth >= min_depth => {
                d.to = at;
                false
            }
            _ => true,
        });
        self.blocks.retain(|b| b.depth < min_depth);
    }

    /// Ends the blocks at the current depth whose `RETURN` expression has
    /// ended (an enclosing block continues at `at`).
    fn pop_returned(&mut self, at: usize) {
        let depth = self.depth;
        while let Some(b) = self.blocks.pop_if(|b| b.depth == depth && b.returned) {
            for i in b.decls {
                if let Some(d) = self.decls.get_mut(i) {
                    d.to = at;
                }
                self.open.retain(|&o| o != i);
            }
        }
    }

    /// Makes the waiting variables of the innermost block at the current
    /// depth visible from `at`; returns that block.
    fn commit_top(&mut self, at: usize) -> Option<&mut Block> {
        let block = self.blocks.last_mut().filter(|b| b.depth == self.depth)?;
        for &i in &block.decls {
            if let Some(d) = self.decls.get_mut(i)
                && d.from.is_none()
            {
                d.from = Some(at);
            }
        }
        Some(block)
    }

    fn var(&mut self, name: Option<&Token>, var: &Token, prev: Option<&Token>) {
        let name = name.filter(|n| {
            n.kind == TokenKind::Ident && !n.keyword(self.src).is_some_and(Keyword::is_reserved)
        });
        let continues = prev.is_some_and(|p| ends_operand(self.src, p));
        if continues {
            self.pop_returned(var.span.start);
        }
        let in_block = self.blocks.last().is_some_and(|b| b.depth == self.depth);
        let after_define = prev.is_some_and(|p| p.keyword(self.src) == Some(Keyword::Define));
        if self.query && self.depth == 0 && (after_define || (continues && !in_block)) {
            self.definition(var);
            if let Some(n) = name {
                let i = self.declare(n, DeclKind::Var, None);
                self.query_pending.push(i);
            }
            return;
        }
        if !(continues && in_block) {
            self.blocks.push(Block {
                depth: self.depth,
                decls: Vec::new(),
                returned: false,
            });
        }
        let Some(n) = name else {
            let _ = self.commit_top(var.span.end);
            return;
        };
        let i = self.declare(n, DeclKind::Var, None);
        self.open.push(i);
        if let Some(block) = self.commit_top(var.span.end) {
            block.decls.push(i);
        }
    }

    fn ret(&mut self, ret: &Token) {
        self.pop_returned(ret.span.start);
        if let Some(block) = self.commit_top(ret.span.end) {
            block.returned = true;
        }
    }

    /// A query definition (or `EVALUATE`, `ORDER BY`, `START AT`) starts at
    /// `t`: the previous one's variables and parameters end, the waiting
    /// query-level variables become visible.
    fn definition(&mut self, t: &Token) {
        for i in self.query_pending.drain(..) {
            if let Some(d) = self.decls.get_mut(i) {
                d.from = Some(t.span.end);
            }
        }
        self.close_from(0, t.span.start);
    }

    /// `( name [: TYPE …], … ) =>`: the names are parameters, visible after
    /// the arrow; their types are keywords.
    fn params(&mut self, sig: &[&Token], k: usize, arrow: &Token) {
        let Some((open, close)) = self.last_group else {
            return;
        };
        if close.checked_add(1) != Some(k)
            || sig.get(open).is_none_or(|t| t.kind != TokenKind::LParen)
        {
            return;
        }
        let Some(inside) = sig.get(open + 1..close) else {
            return;
        };
        let mut nested = 0usize;
        let mut expect_name = true;
        let mut in_type = false;
        for t in inside {
            match t.kind {
                TokenKind::LParen | TokenKind::LBrace => nested += 1,
                TokenKind::RParen | TokenKind::RBrace => nested = nested.saturating_sub(1),
                TokenKind::Comma if nested == 0 => {
                    expect_name = true;
                    in_type = false;
                }
                TokenKind::Colon if nested == 0 => in_type = true,
                TokenKind::Ident if nested == 0 && in_type => {
                    self.keywords.insert(t.span.start);
                }
                TokenKind::Ident if nested == 0 && expect_name => {
                    let i = self.declare(t, DeclKind::Param, Some(arrow.span.end));
                    self.open.push(i);
                    expect_name = false;
                }
                _ if nested == 0 => expect_name = false,
                _ => {}
            }
        }
    }
}

/// What a query `DEFINE` entry defines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DefineKind {
    Measure,
    Column,
    Table,
    Function,
}

/// A `MEASURE`, `COLUMN`, `TABLE` or `FUNCTION` of a query's `DEFINE`.
#[derive(Debug, Clone)]
pub(super) struct QueryDefine {
    pub(super) kind: DefineKind,
    /// The table of a measure or column, unquoted.
    pub(super) table: Option<String>,
    /// Unquoted.
    pub(super) name: String,
    /// The declared name: `[name]` of a measure or column, else the name.
    pub(super) span: Span,
}

/// The `DEFINE` entries of a query, in order.
pub(super) fn query_defines(src: &str, tokens: &[Token]) -> Vec<QueryDefine> {
    let sig: Vec<&Token> = tokens.iter().filter(|t| !t.is_trivia()).collect();
    let mut out = Vec::new();
    for (k, t) in sig.iter().enumerate() {
        let Some(kw) = t.keyword(src) else { continue };
        let Some(name) = sig.get(k + 1) else { continue };
        let member = sig
            .get(k + 2)
            .filter(|m| m.kind == TokenKind::BracketIdent && m.span.start == name.span.end);
        let named = matches!(name.kind, TokenKind::Ident | TokenKind::QuotedIdent);
        let entry = match kw {
            Keyword::Measure | Keyword::Column if named => member.map(|m| QueryDefine {
                kind: if kw == Keyword::Measure {
                    DefineKind::Measure
                } else {
                    DefineKind::Column
                },
                table: Some(name.value(src)),
                name: m.value(src),
                span: m.span,
            }),
            Keyword::Table if named => Some(QueryDefine {
                kind: DefineKind::Table,
                table: None,
                name: name.value(src),
                span: name.span,
            }),
            Keyword::Function if name.kind == TokenKind::Ident => Some(QueryDefine {
                kind: DefineKind::Function,
                table: None,
                name: name.text(src).to_string(),
                span: name.span,
            }),
            _ => None,
        };
        out.extend(entry);
    }
    out
}

/// `schema` plus the `DEFINE` entries of a query; `None` when it has none.
pub(super) fn query_schema(defines: &[QueryDefine], schema: &Schema) -> Option<Schema> {
    if defines.is_empty() {
        return None;
    }
    let mut local = schema.clone();
    for d in defines {
        match (d.kind, d.table.as_deref()) {
            (DefineKind::Measure, Some(table)) => local.add_measure(table, &d.name),
            (DefineKind::Column, Some(table)) => local.add_column(table, &d.name),
            (DefineKind::Table, _) => {
                local.add_open_table(&d.name);
            }
            (DefineKind::Function, _) => local.add_function(&d.name),
            _ => {}
        }
    }
    Some(local)
}
