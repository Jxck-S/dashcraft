//! A tolerant scan of M text for the editor services: the names each scope binds, the role of
//! every identifier, the calls, and the state of the text at a cursor. It works on the tokens of
//! [`tokenize_tolerant`], so text that does not parse yet (the usual state while typing) still
//! gets answers.
//!
//! A stack of frames follows the nesting: `let` (variables, then the body after `in`), `(…)`,
//! `[…]` (record fields after `=`, otherwise field names), `{…}`, the bodies of `each` and of
//! functions (after `=>`), `if`/`then`/`else`, `try`/`otherwise` and section members. A
//! separator (`,` `;` `)` `]` `}` `in` `then` `else` `otherwise`) goes to the innermost frame that
//! takes it and ends the frames above it, which is how bodies without brackets (`each _ + 1, …`)
//! end. Scoping follows the M specification and Power BI Desktop: a `let` variable or record field
//! is in scope everywhere in its `let` or record except in its own initializer (`@name` reaches
//! it there), a section member everywhere in the document.

use crate::error::ParseError;
use crate::lexer::{Comment, CommentKind, Keyword, Token, TokenKind, tokenize_tolerant};
use crate::span::Span;

use super::names::NameKind;

/// The end of a scope or initializer that is still open.
const OPEN: usize = usize::MAX;

/// A name bound in the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Local {
    pub name: String,
    /// [`NameKind::Step`], [`NameKind::Parameter`], [`NameKind::Field`] or
    /// [`NameKind::Member`].
    pub kind: NameKind,
    /// Where the name is bound (the `each` keyword for its `_`).
    pub decl: Span,
    /// The initializer of a step, field or member (after `=`, up to the separator); the type of a
    /// parameter (after `as`).
    pub value: Option<Span>,
    /// An `optional` parameter.
    pub optional: bool,
    /// Where the name is in scope (both ends included).
    pub scope: Span,
    /// Out of scope in its own initializer ([`Self::value`]).
    pub exclusive: bool,
}

impl Local {
    /// In scope at `offset`; `inclusive` for `@name`, which also reaches the own initializer.
    pub fn visible_at(&self, offset: usize, inclusive: bool) -> bool {
        let within = |s: Span| s.start <= offset && offset <= s.end;
        within(self.scope) && (inclusive || !self.exclusive || !self.value.is_some_and(within))
    }
}

/// What an identifier token is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Other,
    /// A name that refers to something.
    Reference,
    /// The name a [`Local`] (by index) is bound with.
    Declaration(usize),
    /// A field name inside `[…]` that is not a record field being defined (`[Column]`,
    /// `_[Column]`, `[[a], [b]]`, record types).
    FieldName,
    /// The name of a section.
    SectionName,
}

/// A call `name(…)` in the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Call {
    /// As written (`Table.AddColumn`), decoded for `#"…"`; `#date` for constructors.
    pub name: String,
    /// The token of the name.
    pub token: usize,
    pub name_span: Span,
}

/// What kind of text belongs at a cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Position {
    /// Where a `let` variable, section member or section is named.
    Declaration,
    /// A field name inside `[…]`.
    FieldName,
    /// A type after `as`, `is`, `type` or `nullable`.
    Type,
    /// The type of a field of a record or table type (`type [a = |]`): a primitive type or any
    /// primary expression (`Int64.Type`).
    FieldType,
    /// An expression.
    Expression,
}

/// The state of the text at a cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AtCursor {
    pub position: Position,
    /// The innermost call whose `(` is open at the cursor (index into [`Scan::calls`]) and the
    /// 0-based argument the cursor is in.
    pub call: Option<(usize, usize)>,
    /// The token before the cursor.
    pub previous: Option<usize>,
}

/// The result of [`scan`].
#[derive(Debug)]
pub(crate) struct Scan<'a> {
    pub src: &'a str,
    pub tokens: Vec<Token>,
    pub comments: Vec<Comment>,
    pub errors: Vec<ParseError>,
    /// One per token.
    pub roles: Vec<Role>,
    pub locals: Vec<Local>,
    pub calls: Vec<Call>,
    /// The state before the first token that starts at or after the requested cursor.
    pub at_cursor: Option<AtCursor>,
}

/// Scans `src`; with `cursor`, also records [`Scan::at_cursor`] there.
pub(crate) fn scan(src: &str, cursor: Option<usize>) -> Scan<'_> {
    let (lexed, errors) = tokenize_tolerant(src);
    let mut s = Scanner {
        src,
        tokens: &lexed.tokens,
        stack: vec![Frame::Root],
        roles: vec![Role::Other; lexed.tokens.len()],
        locals: Vec::new(),
        calls: Vec::new(),
        last_paren: None,
        at_cursor: None,
    };
    for (i, t) in lexed.tokens.iter().enumerate() {
        if let Some(c) = cursor
            && s.at_cursor.is_none()
            && t.span.start >= c
        {
            s.at_cursor = Some(s.snapshot(i));
        }
        if t.kind == TokenKind::Eof {
            break;
        }
        s.token(i, t);
    }
    while s.stack.len() > 1 {
        s.pop(src.len());
    }
    if let Some(root) = s.stack.pop() {
        s.close(root, src.len());
    }
    let Scanner {
        roles,
        locals,
        calls,
        at_cursor,
        ..
    } = s;
    Scan {
        src,
        tokens: lexed.tokens,
        comments: lexed.comments,
        errors,
        roles,
        locals,
        calls,
        at_cursor,
    }
}

impl Scan<'_> {
    /// The locals in scope at `offset`, innermost scope first, without the ones an inner
    /// scope hides.
    pub fn visible(&self, offset: usize, inclusive: bool) -> Vec<&Local> {
        let mut found: Vec<&Local> = self
            .locals
            .iter()
            .filter(|l| l.visible_at(offset, inclusive))
            .collect();
        // A later scope start is a more inner scope (scopes nest).
        found.sort_by_key(|l| std::cmp::Reverse(l.scope.start));
        let mut seen = std::collections::HashSet::new();
        found.retain(|l| seen.insert(l.name.as_str()));
        found
    }

    /// The local that `name` at `offset` refers to.
    pub fn resolve(&self, name: &str, offset: usize, inclusive: bool) -> Option<&Local> {
        self.visible(offset, inclusive)
            .into_iter()
            .find(|l| l.name == name)
    }

    /// Whether `offset` is inside a comment (after its first character, or at the end of a line
    /// comment or unterminated block comment).
    pub fn in_comment(&self, offset: usize) -> bool {
        self.comments.iter().any(|c| {
            let open_end = c.kind == CommentKind::Line || !self.terminated_comment(c);
            c.span.start < offset && (offset < c.span.end || (open_end && offset == c.span.end))
        })
    }

    fn terminated_comment(&self, c: &Comment) -> bool {
        c.text.len() >= 4 && c.text.ends_with("*/")
    }

    /// Whether the text literal, quoted identifier or verbatim token starting at `start` misses
    /// its closing quote.
    pub fn unterminated(&self, start: usize) -> bool {
        self.errors
            .iter()
            .any(|e| e.span.start == start && e.message.starts_with("unterminated text"))
    }

    /// The text of a local's initializer or type, trimmed.
    pub fn value_text(&self, local: &Local) -> Option<&str> {
        local
            .value
            .map(|v| v.slice(self.src).trim())
            .filter(|t| !t.is_empty())
    }
}

/// The name text of an identifier token.
pub(crate) fn token_name(kind: &TokenKind) -> Option<&str> {
    match kind {
        TokenKind::Identifier(n) | TokenKind::QuotedIdentifier(n) => Some(n),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Clause {
    /// After `section`, before its `;`.
    Header,
    /// Where a name is written.
    Name,
    /// After `=`.
    Value,
    /// After `in`.
    Body,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IfState {
    Condition,
    Then,
    Else,
}

/// A comma-separated part of `(…)`, kept to find function parameters.
#[derive(Debug, Clone, Copy, Default)]
struct Segment {
    tokens: usize,
    optional: bool,
    name: Option<usize>,
    /// Where the type starts (after `as`).
    type_start: Option<usize>,
}

/// A possible function parameter: the name token, `optional`, the type.
#[derive(Debug, Clone, Copy)]
struct ParamCandidate {
    token: Option<usize>,
    optional: bool,
    ty: Option<Span>,
}

#[derive(Debug, Clone)]
enum Frame {
    Root,
    Section {
        state: Clause,
        pending: Option<usize>,
        open: Option<usize>,
        locals: Vec<usize>,
    },
    Let {
        start: usize,
        state: Clause,
        pending: Option<usize>,
        open: Option<usize>,
        locals: Vec<usize>,
    },
    Paren {
        call: Option<usize>,
        args: usize,
        segment: Segment,
        params: Vec<ParamCandidate>,
    },
    Bracket {
        start: usize,
        /// A record type: field names bind nothing and values are types.
        ty: bool,
        state: Clause,
        pending: Vec<usize>,
        open: Option<usize>,
        locals: Vec<usize>,
    },
    Brace,
    /// The body of `each` or of a function.
    Body {
        locals: Vec<usize>,
    },
    If(IfState),
    Try {
        otherwise: bool,
    },
}

struct Scanner<'a, 't> {
    src: &'a str,
    tokens: &'t [Token],
    stack: Vec<Frame>,
    roles: Vec<Role>,
    locals: Vec<Local>,
    calls: Vec<Call>,
    /// The last `)` (token index) and the parameters its group would declare before `=>`.
    last_paren: Option<(usize, Vec<ParamCandidate>)>,
    at_cursor: Option<AtCursor>,
}

fn is_separator(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Comma
            | TokenKind::Semicolon
            | TokenKind::RParen
            | TokenKind::RBracket
            | TokenKind::RBrace
            | TokenKind::Keyword(Keyword::In | Keyword::Then | Keyword::Else | Keyword::Otherwise)
    )
}

/// Whether `frame` takes the separator `kind` (instead of ending at it).
fn takes(frame: &Frame, kind: &TokenKind) -> bool {
    match frame {
        Frame::Section { .. } => *kind == TokenKind::Semicolon,
        Frame::Let { state, .. } => {
            *state != Clause::Body
                && matches!(kind, TokenKind::Comma | TokenKind::Keyword(Keyword::In))
        }
        Frame::Paren { .. } => matches!(kind, TokenKind::Comma | TokenKind::RParen),
        Frame::Bracket { .. } => matches!(kind, TokenKind::Comma | TokenKind::RBracket),
        Frame::Brace => matches!(kind, TokenKind::Comma | TokenKind::RBrace),
        Frame::If(IfState::Condition) => *kind == TokenKind::Keyword(Keyword::Then),
        Frame::If(IfState::Then) => *kind == TokenKind::Keyword(Keyword::Else),
        Frame::Try { otherwise } => !otherwise && *kind == TokenKind::Keyword(Keyword::Otherwise),
        Frame::Root | Frame::Body { .. } | Frame::If(IfState::Else) => false,
    }
}

impl Scanner<'_, '_> {
    fn kind(&self, i: usize) -> Option<&TokenKind> {
        self.tokens.get(i).map(|t| &t.kind)
    }

    fn snapshot(&self, i: usize) -> AtCursor {
        let previous = i.checked_sub(1);
        let prev = previous.and_then(|p| self.kind(p));
        let type_word = match prev {
            Some(TokenKind::Keyword(Keyword::As | Keyword::Is | Keyword::Type)) => true,
            Some(TokenKind::Identifier(n)) => n == "nullable",
            _ => false,
        };
        let position = match self.stack.last() {
            Some(Frame::Let {
                state: Clause::Name,
                ..
            })
            | Some(Frame::Section {
                state: Clause::Name | Clause::Header,
                ..
            }) => Position::Declaration,
            Some(Frame::Bracket {
                state: Clause::Name,
                ..
            }) => Position::FieldName,
            Some(Frame::Bracket {
                ty: true,
                state: Clause::Value,
                ..
            }) if matches!(prev, Some(TokenKind::Equal)) => Position::FieldType,
            _ if type_word => Position::Type,
            _ => Position::Expression,
        };
        let call = self.stack.iter().rev().find_map(|f| match f {
            Frame::Paren {
                call: Some(c),
                args,
                ..
            } => Some((*c, *args)),
            _ => None,
        });
        AtCursor {
            position,
            call,
            previous,
        }
    }

    fn token(&mut self, i: usize, t: &Token) {
        let in_field_names = matches!(
            self.stack.last(),
            Some(Frame::Bracket {
                state: Clause::Name,
                ..
            })
        );
        // `[Check in]`: keywords are words of a field name.
        if is_separator(&t.kind) && !(in_field_names && matches!(t.kind, TokenKind::Keyword(_))) {
            self.separator(i, t);
        } else {
            self.regular(i, t);
        }
    }

    fn separator(&mut self, i: usize, t: &Token) {
        let Some(j) = self.stack.iter().rposition(|f| takes(f, &t.kind)) else {
            return;
        };
        while self.stack.len() > j + 1 {
            self.pop(t.span.start);
        }
        let at = t.span.start;
        let mut close_value: Option<usize> = None;
        let mut pop = false;
        match self.stack.last_mut() {
            Some(Frame::Section { state, open, .. }) => {
                close_value = open.take();
                if *state != Clause::Header || t.kind == TokenKind::Semicolon {
                    *state = Clause::Name;
                }
            }
            Some(Frame::Let {
                state,
                pending,
                open,
                ..
            }) => {
                close_value = open.take();
                *pending = None;
                *state = if t.kind == TokenKind::Comma {
                    Clause::Name
                } else {
                    Clause::Body
                };
            }
            Some(Frame::Paren {
                args,
                segment,
                params,
                ..
            }) => {
                params.push(ParamCandidate {
                    token: segment.name.filter(|_| {
                        segment.tokens == 1 + usize::from(segment.optional)
                            || segment.type_start.is_some()
                    }),
                    optional: segment.optional,
                    ty: segment.type_start.map(|s| Span::new(s, at)),
                });
                *segment = Segment::default();
                if t.kind == TokenKind::Comma {
                    *args += 1;
                } else {
                    self.last_paren = Some((i, std::mem::take(params)));
                    pop = true;
                }
            }
            Some(Frame::Bracket {
                state,
                pending,
                open,
                ..
            }) => {
                close_value = open.take();
                pending.clear();
                *state = Clause::Name;
                pop = t.kind == TokenKind::RBracket;
            }
            Some(Frame::Brace) => pop = t.kind == TokenKind::RBrace,
            Some(Frame::If(state)) => {
                *state = if *state == IfState::Condition {
                    IfState::Then
                } else {
                    IfState::Else
                };
            }
            Some(Frame::Try { otherwise }) => *otherwise = true,
            Some(Frame::Root | Frame::Body { .. }) | None => {}
        }
        if let Some(l) = close_value {
            self.end_value(l, at);
        }
        if pop {
            // `)`, `]` and `}` end their own frame only.
            self.pop(at);
        }
    }

    fn regular(&mut self, i: usize, t: &Token) {
        if let Some(Frame::Paren { segment, .. }) = self.stack.last_mut() {
            segment.tokens += 1;
            match &t.kind {
                TokenKind::Identifier(n) if n == "optional" && segment.tokens == 1 => {
                    segment.optional = true;
                }
                TokenKind::Identifier(_) | TokenKind::QuotedIdentifier(_)
                    if segment.tokens == 1 + usize::from(segment.optional) =>
                {
                    segment.name = Some(i);
                }
                TokenKind::Keyword(Keyword::As)
                    if segment.name.is_some()
                        && segment.tokens == 2 + usize::from(segment.optional) =>
                {
                    segment.type_start = Some(t.span.end);
                }
                _ => {}
            }
        }
        if self.named(i, t) {
            return;
        }
        match &t.kind {
            TokenKind::LParen => {
                let call = self.callee(i).map(|(name, token, name_span)| {
                    self.calls.push(Call {
                        name,
                        token,
                        name_span,
                    });
                    self.calls.len() - 1
                });
                self.stack.push(Frame::Paren {
                    call,
                    args: 0,
                    segment: Segment::default(),
                    params: Vec::new(),
                });
            }
            TokenKind::LBracket => {
                let ty = self.type_bracket(i);
                self.stack.push(Frame::Bracket {
                    start: t.span.end,
                    ty,
                    state: Clause::Name,
                    pending: Vec::new(),
                    open: None,
                    locals: Vec::new(),
                });
            }
            TokenKind::LBrace => self.stack.push(Frame::Brace),
            TokenKind::Keyword(Keyword::Let) => self.stack.push(Frame::Let {
                start: t.span.start,
                state: Clause::Name,
                pending: None,
                open: None,
                locals: Vec::new(),
            }),
            TokenKind::Keyword(Keyword::Each) => {
                let l = self.add_local(Local {
                    name: "_".to_owned(),
                    kind: NameKind::Parameter,
                    decl: t.span,
                    value: None,
                    optional: false,
                    scope: Span::new(t.span.end, OPEN),
                    exclusive: false,
                });
                self.stack.push(Frame::Body { locals: vec![l] });
            }
            TokenKind::FatArrow => {
                let params = match self.last_paren.take() {
                    Some((close, params)) if self.returns_between(close, i) => params,
                    _ => Vec::new(),
                };
                let mut locals = Vec::new();
                for p in params {
                    let Some(tok) = p.token else { continue };
                    let Some(name) = self.kind(tok).and_then(token_name).map(str::to_owned) else {
                        continue;
                    };
                    let decl = self.tokens.get(tok).map_or_else(Span::default, |t| t.span);
                    let l = self.add_local(Local {
                        name,
                        kind: NameKind::Parameter,
                        decl,
                        value: p.ty,
                        optional: p.optional,
                        scope: Span::new(t.span.end, OPEN),
                        exclusive: false,
                    });
                    if let Some(r) = self.roles.get_mut(tok) {
                        *r = Role::Declaration(l);
                    }
                    locals.push(l);
                }
                self.stack.push(Frame::Body { locals });
            }
            TokenKind::Keyword(Keyword::If) => self.stack.push(Frame::If(IfState::Condition)),
            TokenKind::Keyword(Keyword::Try) => self.stack.push(Frame::Try { otherwise: false }),
            TokenKind::Keyword(Keyword::Section) => {
                if let Some(top @ Frame::Root) = self.stack.last_mut() {
                    *top = Frame::Section {
                        state: Clause::Header,
                        pending: None,
                        open: None,
                        locals: Vec::new(),
                    };
                }
            }
            TokenKind::Identifier(_) | TokenKind::QuotedIdentifier(_) => {
                if let Some(r) = self.roles.get_mut(i) {
                    *r = Role::Reference;
                }
            }
            _ => {}
        }
    }

    /// Handles a token where a name is written (`let` variables, section members, field names);
    /// `true` when it was consumed.
    fn named(&mut self, i: usize, t: &Token) -> bool {
        let is_name = token_name(&t.kind).is_some();
        let src = self.src;
        let tokens = self.tokens;
        let mut new_local: Option<(Local, Vec<usize>)> = None;
        let consumed = match self.stack.last_mut() {
            Some(Frame::Let {
                start,
                state: state @ Clause::Name,
                pending,
                ..
            }) => {
                if is_name {
                    *pending = Some(i);
                    true
                } else if t.kind == TokenKind::Equal {
                    *state = Clause::Value;
                    if let Some(p) = pending.take()
                        && let Some(tok) = tokens.get(p)
                        && let Some(name) = token_name(&tok.kind)
                    {
                        new_local = Some((
                            Local {
                                name: name.to_owned(),
                                kind: NameKind::Step,
                                decl: tok.span,
                                value: Some(Span::new(t.span.end, OPEN)),
                                optional: false,
                                scope: Span::new(*start, OPEN),
                                exclusive: true,
                            },
                            vec![p],
                        ));
                    }
                    true
                } else {
                    false
                }
            }
            Some(Frame::Section {
                state: Clause::Header,
                ..
            }) => {
                if is_name && let Some(r) = self.roles.get_mut(i) {
                    *r = Role::SectionName;
                }
                is_name
            }
            Some(Frame::Section {
                state: state @ Clause::Name,
                pending,
                ..
            }) => {
                if is_name {
                    *pending = Some(i);
                    true
                } else if t.kind == TokenKind::Equal {
                    *state = Clause::Value;
                    if let Some(p) = pending.take()
                        && let Some(tok) = tokens.get(p)
                        && let Some(name) = token_name(&tok.kind)
                    {
                        new_local = Some((
                            Local {
                                name: name.to_owned(),
                                kind: NameKind::Member,
                                decl: tok.span,
                                value: Some(Span::new(t.span.end, OPEN)),
                                optional: false,
                                scope: Span::new(0, OPEN),
                                exclusive: false,
                            },
                            vec![p],
                        ));
                    }
                    true
                } else {
                    t.kind == TokenKind::Keyword(Keyword::Shared)
                }
            }
            Some(Frame::Bracket {
                start,
                ty,
                state: state @ Clause::Name,
                pending,
                ..
            }) => match &t.kind {
                TokenKind::Identifier(_)
                | TokenKind::QuotedIdentifier(_)
                | TokenKind::Number(_)
                | TokenKind::Keyword(_)
                    if !matches!(&t.kind, TokenKind::Keyword(k) if k.is_hash()) =>
                {
                    pending.push(i);
                    if let Some(r) = self.roles.get_mut(i) {
                        *r = if is_name {
                            Role::FieldName
                        } else {
                            Role::Other
                        };
                    }
                    true
                }
                TokenKind::Equal => {
                    *state = Clause::Value;
                    let names = std::mem::take(pending);
                    if !*ty
                        && let (Some(first), Some(last)) = (names.first(), names.last())
                        && let (Some(a), Some(b)) = (tokens.get(*first), tokens.get(*last))
                    {
                        let decl = a.span.to(b.span);
                        let name = match (names.len(), &a.kind) {
                            (1, TokenKind::QuotedIdentifier(n)) => n.clone(),
                            _ => decl.slice(src).to_owned(),
                        };
                        new_local = Some((
                            Local {
                                name,
                                kind: NameKind::Field,
                                decl,
                                value: Some(Span::new(t.span.end, OPEN)),
                                optional: false,
                                scope: Span::new(*start, OPEN),
                                exclusive: true,
                            },
                            names,
                        ));
                    }
                    true
                }
                _ => false,
            },
            _ => false,
        };
        if let Some((local, decl_tokens)) = new_local {
            let l = self.add_local(local);
            for tok in decl_tokens {
                if let Some(r) = self.roles.get_mut(tok) {
                    *r = Role::Declaration(l);
                }
            }
            if let Some(
                Frame::Let { open, locals, .. }
                | Frame::Section { open, locals, .. }
                | Frame::Bracket { open, locals, .. },
            ) = self.stack.last_mut()
            {
                *open = Some(l);
                locals.push(l);
            }
        }
        consumed
    }

    fn add_local(&mut self, local: Local) -> usize {
        self.locals.push(local);
        self.locals.len() - 1
    }

    /// The called name before the `(` at token `i`.
    fn callee(&self, i: usize) -> Option<(String, usize, Span)> {
        let p = i.checked_sub(1)?;
        let tok = self.tokens.get(p)?;
        let name = match &tok.kind {
            TokenKind::Identifier(n) | TokenKind::QuotedIdentifier(n) => n.clone(),
            TokenKind::Keyword(k) if super::names::constructor(*k).is_some() => {
                k.as_str().to_owned()
            }
            _ => return None,
        };
        // `type function (…)` declares, `x as number (…)` is not a call of `number`.
        let before = p.checked_sub(1).and_then(|b| self.kind(b));
        if matches!(
            before,
            Some(TokenKind::Keyword(
                Keyword::Type | Keyword::As | Keyword::Is
            ))
        ) {
            return None;
        }
        Some((name, p, tok.span))
    }

    /// Whether `[` at token `i` starts a record type (`type [a = text]`, `type table [a = text]`,
    /// a record type inside one).
    fn type_bracket(&self, i: usize) -> bool {
        let prev = i.checked_sub(1).and_then(|p| self.kind(p));
        let before = i.checked_sub(2).and_then(|p| self.kind(p));
        match prev {
            Some(TokenKind::Keyword(Keyword::Type)) => true,
            Some(TokenKind::Identifier(n)) if n == "table" => {
                matches!(before, Some(TokenKind::Keyword(Keyword::Type)))
            }
            Some(TokenKind::Equal) => {
                matches!(self.stack.last(), Some(Frame::Bracket { ty: true, .. }))
            }
            _ => false,
        }
    }

    /// Whether only a return type (`as nullable text`) lies between the `)` at `close` and the
    /// `=>` at `arrow`.
    fn returns_between(&self, close: usize, arrow: usize) -> bool {
        let between = arrow.saturating_sub(close + 1);
        between <= 3
            && (close + 1..arrow).all(|k| {
                matches!(
                    self.kind(k),
                    Some(
                        TokenKind::Identifier(_)
                            | TokenKind::Keyword(Keyword::As | Keyword::Type | Keyword::Null)
                    )
                )
            })
    }

    fn end_value(&mut self, l: usize, at: usize) {
        if let Some(local) = self.locals.get_mut(l)
            && let Some(v) = &mut local.value
            && v.end == OPEN
        {
            v.end = at.max(v.start);
        }
    }

    fn pop(&mut self, end: usize) {
        if let Some(frame) = self.stack.pop() {
            self.close(frame, end);
        }
    }

    /// Ends the scopes and the open initializer of `frame` at `end`.
    fn close(&mut self, frame: Frame, end: usize) {
        let (locals, open) = match frame {
            Frame::Let { locals, open, .. }
            | Frame::Section { locals, open, .. }
            | Frame::Bracket { locals, open, .. } => (locals, open),
            Frame::Body { locals } => (locals, None),
            Frame::Root | Frame::Paren { .. } | Frame::Brace | Frame::If(_) | Frame::Try { .. } => {
                return;
            }
        };
        if let Some(l) = open {
            self.end_value(l, end);
        }
        for l in locals {
            if let Some(local) = self.locals.get_mut(l)
                && local.scope.end == OPEN
            {
                local.scope.end = end.max(local.scope.start);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `name kind scope` for every local, scopes as the text they cover.
    fn locals(src: &str) -> Vec<String> {
        let s = scan(src, None);
        s.locals
            .iter()
            .map(|l| {
                format!(
                    "{} {} [{}]{}",
                    l.name,
                    l.kind.as_str(),
                    l.scope.slice(src),
                    l.value
                        .map(|v| format!(" = {}", v.slice(src).trim()))
                        .unwrap_or_default()
                )
            })
            .collect()
    }

    fn visible_at(src: &str, marker: &str) -> Vec<String> {
        let offset = src.find(marker).unwrap_or(src.len());
        let s = scan(src, None);
        s.visible(offset, false)
            .iter()
            .map(|l| l.name.clone())
            .collect()
    }

    #[test]
    fn let_steps_functions_each_and_records_bind_names() {
        let src = "let a = 1, F = (x as number, optional y) as number => x + y, G = each _ * 2, r = [p = 1, q = p] in a";
        assert_eq!(
            locals(src),
            vec![
                format!("a step [{src}] = 1"),
                "F step [let a = 1, F = (x as number, optional y) as number => x + y, G = each _ * 2, r = [p = 1, q = p] in a] = (x as number, optional y) as number => x + y".to_owned(),
                "x parameter [ x + y] = number".to_owned(),
                "y parameter [ x + y]".to_owned(),
                "G step [let a = 1, F = (x as number, optional y) as number => x + y, G = each _ * 2, r = [p = 1, q = p] in a] = each _ * 2".to_owned(),
                "_ parameter [ _ * 2]".to_owned(),
                "r step [let a = 1, F = (x as number, optional y) as number => x + y, G = each _ * 2, r = [p = 1, q = p] in a] = [p = 1, q = p]".to_owned(),
                "p field [p = 1, q = p] = 1".to_owned(),
                "q field [p = 1, q = p] = p".to_owned(),
            ]
        );
    }

    #[test]
    fn scoping_is_exclusive_and_innermost_first() {
        let src = "let a = 1, b = let a = 2, c = /*1*/ 0 in /*2*/ a in /*3*/ b";
        // Neither `b` nor `c` is in scope in its own initializer; the inner `a` hides the outer.
        assert_eq!(visible_at(src, "/*1*/"), ["a"]);
        assert_eq!(visible_at(src, "/*2*/"), ["a", "c"]);
        assert_eq!(visible_at(src, "/*3*/"), ["a", "b"]);
        let s = scan(src, None);
        let inner = src.find("/*2*/").unwrap_or(0);
        assert_eq!(
            s.resolve("a", inner, false).map(|l| l.decl.start),
            src.find("a = 2")
        );
        // `@c` reaches its own initializer.
        let own = src.find("/*1*/").unwrap_or(0);
        assert!(s.resolve("c", own, false).is_none());
        assert!(s.resolve("c", own, true).is_some());
    }

    #[test]
    fn bodies_without_brackets_end_at_separators() {
        let src = "Table.AddColumn(t, \"x\", each if [A] then /*1*/ 1 else 2, type text) /*2*/";
        assert_eq!(visible_at(src, "/*1*/"), ["_"]);
        assert!(visible_at(src, "/*2*/").is_empty());
        let s = scan(src, None);
        assert_eq!(s.calls.len(), 1);
        let field = s
            .tokens
            .iter()
            .position(|t| t.kind == TokenKind::Identifier("A".to_owned()));
        assert_eq!(field.and_then(|f| s.roles.get(f)), Some(&Role::FieldName));
    }

    #[test]
    fn sections_and_type_records() {
        let src = "section S; shared A = B + 1; B = type [x = text]; C = (t as table) => t;";
        assert_eq!(
            locals(src),
            vec![
                format!("A member [{src}] = B + 1"),
                format!("B member [{src}] = type [x = text]"),
                format!("C member [{src}] = (t as table) => t"),
                "t parameter [ t] = table".to_owned(),
            ]
        );
        let s = scan(src, None);
        assert_eq!(s.roles.get(1), Some(&Role::SectionName));
    }

    #[test]
    fn the_cursor_state() {
        let at = |src: &str| {
            let offset = src.find('|').unwrap_or(src.len());
            let text = src.replacen('|', "", 1);
            let s = scan(&text, Some(offset));
            let c = s.at_cursor.unwrap();
            (
                c.position,
                c.call
                    .and_then(|(i, arg)| s.calls.get(i).map(|c| (c.name.clone(), arg))),
            )
        };
        let call = |name: &str, arg: usize| Some((name.to_owned(), arg));
        assert_eq!(at("let |"), (Position::Declaration, None));
        assert_eq!(at("let a = 1, |"), (Position::Declaration, None));
        assert_eq!(at("let a = |"), (Position::Expression, None));
        assert_eq!(at("let a = 1 in |"), (Position::Expression, None));
        assert_eq!(at("x as |"), (Position::Type, None));
        assert_eq!(at("(x as nullable |"), (Position::Type, None));
        assert_eq!(at("each [|"), (Position::FieldName, None));
        assert_eq!(at("type [a = |"), (Position::FieldType, None));
        assert_eq!(at("type table [a = |"), (Position::FieldType, None));
        assert_eq!(at("[a = |"), (Position::Expression, None));
        assert_eq!(
            at("Table.AddColumn(t, \"x\", each Text.Upper([A]), |"),
            (Position::Expression, call("Table.AddColumn", 3))
        );
        assert_eq!(
            at("Table.AddColumn(t, \"x\", each Text.Upper(|"),
            (Position::Expression, call("Text.Upper", 0))
        );
        assert_eq!(
            at("f(let a = 1, b = |"),
            (Position::Expression, call("f", 0))
        );
        assert_eq!(
            at("#date(2024, |)"),
            (Position::Expression, call("#date", 1))
        );
        assert_eq!(at("f(x)|"), (Position::Expression, None));
        assert_eq!(at("section S; |"), (Position::Declaration, None));
        assert_eq!(at("section S; a = |"), (Position::Expression, None));
    }

    #[test]
    fn every_prefix_of_messy_text_scans() {
        let src = "section S; shared A = let a = (x, optional y as text) => [p = x, q = {1..2}], b = try a(1) otherwise each _ in if b then #date(1, 2, 3) else type table [c = number]; B = \"open";
        for end in (0..=src.len()).filter(|i| src.is_char_boundary(*i)) {
            let text = &src[..end];
            for cursor in [0, end / 2, end] {
                let s = scan(text, Some(cursor));
                assert!(s.at_cursor.is_some());
                for l in &s.locals {
                    assert!(l.scope.end <= text.len() && l.scope.start <= l.scope.end);
                }
            }
        }
    }
}
