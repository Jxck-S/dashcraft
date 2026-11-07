//! M formatter (`m.format`): lays out an expression or section document deterministically and
//! idempotently.
//!
//! The syntax tree decides where lines break, but the output is the source's own tokens and
//! comments: only the white space between them changes (M is case-sensitive, so there are no
//! casing options). Comments stay where they were: after a token on its line, or on their own
//! lines before the next token. A blank line between `let` steps or before a comment is kept.
//!
//! `let` puts each step on its own line and the expression after `in` on the next, indented, as
//! Power BI Desktop writes queries. A group (the arguments of a call, a record, a list,
//! parentheses, a chain of operators of one precedence, an `if`, a `try`, the value of a step or
//! field) stays on one line when it fits in [`FormatOptions::max_width`]. Otherwise it breaks:
//! one item per line with the closing bracket on its own line, operators at the start of the
//! continuation lines, `then` and `else` branches indented, and the value of a step on the next
//! line.
//!
//! The result is checked before it is returned: it must tokenize to the same tokens and
//! comments and parse to the same tree as the input. Otherwise [`format`] returns
//! [`FormatError::Unstable`] rather than a text with another meaning.

use std::fmt;

use super::diagnostics::{Diagnostic, DiagnosticCode, Severity};
use crate::ast::{
    Binding, Document, Expr, ExprKind, FieldSpec, ListItem, Name, Param, ParamSpec, RecordField,
    RecordType, Section, TableRowType, TryHandler, TypeExpr, TypeKind, UnaryOp,
};
use crate::error::ParseError;
use crate::lexer::{CommentKind, Lexed, Token, TokenKind, tokenize};

/// Largest indentation width [`format`] uses (wider values are capped).
pub const MAX_INDENT_WIDTH: usize = 16;

/// Layout options of [`format`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatOptions {
    /// Preferred maximum line length in characters (default 100). A line is longer only where a
    /// single token, comment or unbreakable run is.
    pub max_width: usize,
    /// Spaces per indentation level (default 4, at most [`MAX_INDENT_WIDTH`]), or the width a
    /// tab counts for with `use_tabs`.
    pub indent_width: usize,
    /// Indent with tabs instead of spaces.
    pub use_tabs: bool,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            max_width: 100,
            indent_width: 4,
            use_tabs: false,
        }
    }
}

/// Why [`format`] returned no text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatError {
    /// The text does not parse: the syntax error, as [`super::diagnostics`] reports it.
    Syntax(Diagnostic),
    /// The layout would have changed the text's tokens or syntax tree (a formatter bug); the
    /// text is better left as it is.
    Unstable,
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatError::Syntax(d) => write!(f, "syntax error: {}", d.message),
            FormatError::Unstable => {
                f.write_str("the formatted text would not be equivalent to the input")
            }
        }
    }
}

impl std::error::Error for FormatError {}

fn syntax(e: ParseError) -> FormatError {
    FormatError::Syntax(Diagnostic {
        severity: Severity::Error,
        code: DiagnosticCode::Syntax,
        message: e.message,
        span: e.span,
        suggestion: None,
    })
}

/// Formats `src`, an M expression or section document. A blank text formats to an empty one.
/// The output uses `\n` line breaks (line breaks inside text literals and comments stay as
/// written) and has no trailing line break.
pub fn format(src: &str, opts: &FormatOptions) -> Result<String, FormatError> {
    if src.trim().is_empty() {
        return Ok(String::new());
    }
    let opts = FormatOptions {
        indent_width: opts.indent_width.min(MAX_INDENT_WIDTH),
        ..*opts
    };
    let mut doc = crate::parse_document(src).map_err(syntax)?;
    let lexed = tokenize(src).map_err(syntax)?;
    let mut emitter = Emitter::new(src, &lexed);
    emitter.document(&doc);
    let out = emitter.finish(&opts);
    let mut again = crate::parse_document(&out).map_err(|_| FormatError::Unstable)?;
    doc.clear_spans();
    again.clear_spans();
    if doc != again || !same_tokens(src, &lexed, &out) {
        return Err(FormatError::Unstable);
    }
    Ok(out)
}

// ---- comments and tokens ---------------------------------------------------

/// Line breaks in `s`, as the M lexical grammar counts them (`CR LF` is one).
fn line_breaks(s: &str) -> usize {
    let mut n = 0usize;
    let mut prev_cr = false;
    for c in s.chars() {
        match c {
            '\n' if prev_cr => {}
            '\r' | '\n' | '\u{0085}' | '\u{2028}' | '\u{2029}' => n = n.saturating_add(1),
            _ => {}
        }
        prev_cr = c == '\r';
    }
    n
}

#[derive(Debug, Clone, Copy)]
struct Note<'a> {
    text: &'a str,
    line: bool,
    /// A blank line separates it from what came before.
    blank_before: bool,
    /// A line break follows it in the source.
    newline_after: bool,
}

/// A token with the comments attached to it.
#[derive(Debug)]
struct Sig<'a> {
    kind: &'a TokenKind,
    start: usize,
    end: usize,
    text: &'a str,
    /// Comments on their own lines before the token.
    leading: Vec<Note<'a>>,
    /// Comments after the token on its line.
    trailing: Vec<Note<'a>>,
    /// A blank line separates it from what came before (its last leading comment, or the
    /// previous token).
    blank_before: bool,
}

/// A token or a comment, in source order.
#[derive(Debug, Clone, Copy)]
enum Item<'a> {
    Token(&'a Token),
    Comment(CommentKind),
}

/// Tokens (without the end marker) and comments in source order: `(start, end, item)`.
fn items<'a>(lexed: &'a Lexed) -> Vec<(usize, usize, Item<'a>)> {
    let mut out: Vec<(usize, usize, Item<'a>)> = lexed
        .tokens
        .iter()
        .filter(|t| t.kind != TokenKind::Eof)
        .map(|t| (t.span.start, t.span.end, Item::Token(t)))
        .chain(
            lexed
                .comments
                .iter()
                .map(|c| (c.span.start, c.span.end, Item::Comment(c.kind))),
        )
        .collect();
    out.sort_by_key(|&(start, end, _)| (start, end));
    out
}

/// Splits the source into tokens with their comments, and the comments after the last token.
fn attach<'a>(src: &'a str, lexed: &'a Lexed) -> (Vec<Sig<'a>>, Vec<Note<'a>>) {
    let all = items(lexed);
    let mut sigs: Vec<Sig<'a>> = Vec::new();
    let mut pending: Vec<Note<'a>> = Vec::new();
    let mut newline_since_sig = false;
    let mut prev_end = 0usize;
    for (i, &(start, end, item)) in all.iter().enumerate() {
        let breaks = line_breaks(src.get(prev_end..start).unwrap_or(""));
        newline_since_sig |= breaks > 0;
        let text = src.get(start..end).unwrap_or("");
        match item {
            Item::Comment(kind) => {
                let line = kind == CommentKind::Line;
                let next = all.get(i + 1).map_or(src.len(), |&(s, _, _)| s);
                let newline_after = line || line_breaks(src.get(end..next).unwrap_or("")) > 0;
                let note = Note {
                    text: if line { text.trim_end() } else { text },
                    line,
                    blank_before: breaks >= 2,
                    newline_after,
                };
                match sigs.last_mut() {
                    Some(last) if !newline_since_sig => last.trailing.push(note),
                    _ => pending.push(note),
                }
            }
            Item::Token(t) => {
                sigs.push(Sig {
                    kind: &t.kind,
                    start,
                    end,
                    text,
                    leading: std::mem::take(&mut pending),
                    trailing: Vec::new(),
                    blank_before: breaks >= 2,
                });
                newline_since_sig = false;
            }
        }
        prev_end = end;
    }
    (sigs, pending)
}

// ---- layout commands -------------------------------------------------------

/// A layout command. Groups render flat (`Line` as a space, `SoftLine` as nothing) when they
/// fit on the rest of the line, else broken (both as line breaks).
#[derive(Debug)]
enum Cmd<'a> {
    Text(&'a str),
    Space,
    Line,
    SoftLine,
    HardLine,
    /// A line break with an empty line before the next text.
    Blank,
    /// A comment on its own line.
    Leading(Note<'a>),
    /// A comment after the text on the line.
    Trailing(Note<'a>),
    GroupStart,
    GroupEnd,
    Indent,
    Dedent,
}

struct Emitter<'a> {
    src: &'a str,
    sigs: Vec<Sig<'a>>,
    tail: Vec<Note<'a>>,
    pos: usize,
    cmds: Vec<Cmd<'a>>,
    /// Trailing comments of the last token, placed after any group or indentation that ends
    /// right after it.
    trailing: Vec<Note<'a>>,
}

impl<'a> Emitter<'a> {
    fn new(src: &'a str, lexed: &'a Lexed) -> Self {
        let (sigs, tail) = attach(src, lexed);
        Self {
            src,
            sigs,
            tail,
            pos: 0,
            cmds: Vec::new(),
            trailing: Vec::new(),
        }
    }

    fn finish(mut self, opts: &FormatOptions) -> String {
        // Tokens the layout did not place (none for a parsed text) still go out, so nothing is
        // lost; the equivalence check then decides.
        while self.pos < self.sigs.len() {
            self.push(Cmd::Space);
            self.tok();
        }
        self.flush_trailing();
        let tail = std::mem::take(&mut self.tail);
        self.cmds.extend(tail.into_iter().map(Cmd::Leading));
        render(&self.cmds, opts)
    }

    fn flush_trailing(&mut self) {
        if !self.trailing.is_empty() {
            let trailing = std::mem::take(&mut self.trailing);
            self.cmds.extend(trailing.into_iter().map(Cmd::Trailing));
        }
    }

    fn push(&mut self, cmd: Cmd<'a>) {
        if !matches!(cmd, Cmd::GroupEnd | Cmd::Dedent) {
            self.flush_trailing();
        }
        self.cmds.push(cmd);
    }

    fn peek(&self) -> Option<&Sig<'a>> {
        self.sigs.get(self.pos)
    }

    fn peek_is(&self, kind: &TokenKind) -> bool {
        self.peek().is_some_and(|s| s.kind == kind)
    }

    /// Places the next token's own-line comments now, ahead of any group that starts with the
    /// token.
    fn leading(&mut self) {
        let Some(sig) = self.sigs.get_mut(self.pos) else {
            return;
        };
        if sig.leading.is_empty() {
            return;
        }
        let comments = std::mem::take(&mut sig.leading);
        let blank_after = std::mem::take(&mut sig.blank_before);
        self.flush_trailing();
        let mut at = self.cmds.len();
        while at > 0 && matches!(self.cmds.get(at - 1), Some(Cmd::GroupStart)) {
            at -= 1;
        }
        let mut insert: Vec<Cmd<'a>> = comments.into_iter().map(Cmd::Leading).collect();
        if blank_after {
            insert.push(Cmd::Blank);
        }
        self.cmds.splice(at..at, insert);
    }

    /// Emits the next token.
    fn tok(&mut self) {
        self.leading();
        let Some(sig) = self.sigs.get_mut(self.pos) else {
            return;
        };
        self.pos += 1;
        let text = sig.text;
        let trailing = std::mem::take(&mut sig.trailing);
        self.push(Cmd::Text(text));
        self.trailing = trailing;
    }

    /// Emits every token that starts before `end`, without spaces.
    fn leaf(&mut self, end: usize) {
        while self.peek().is_some_and(|s| s.start < end) {
            self.tok();
        }
    }

    /// Emits every token that starts before `end`, separated by spaces (`nullable text`).
    fn words(&mut self, end: usize) {
        let mut first = true;
        while self.peek().is_some_and(|s| s.start < end) {
            if !first {
                self.push(Cmd::Space);
            }
            self.tok();
            first = false;
        }
    }

    /// A name. A generalized identifier (`[Order Date]`) is several tokens whose spaces are part
    /// of the name, so it goes out as written.
    fn name(&mut self, name: &Name) {
        let end = name.span.end;
        if !self.peek().is_some_and(|s| s.start < end) {
            return;
        }
        let mut last = self.pos;
        while self.sigs.get(last + 1).is_some_and(|s| s.start < end) {
            last += 1;
        }
        if last == self.pos {
            self.tok();
            return;
        }
        self.leading();
        let start = self.peek().map_or(0, |s| s.start);
        let Some(last_sig) = self.sigs.get_mut(last) else {
            return;
        };
        let trailing = std::mem::take(&mut last_sig.trailing);
        let text = self.src.get(start..last_sig.end).unwrap_or("");
        self.pos = last + 1;
        self.push(Cmd::Text(text));
        self.trailing = trailing;
    }

    /// A line break, with an empty line when the source had one before the next token.
    fn statement_break(&mut self) {
        let blank = self
            .peek()
            .is_some_and(|s| s.blank_before && s.leading.is_empty());
        self.push(if blank { Cmd::Blank } else { Cmd::HardLine });
    }

    /// `,` and a line break or space.
    fn separator(&mut self) {
        if self.peek_is(&TokenKind::Comma) {
            self.tok();
        }
        self.push(Cmd::Line);
    }

    /// `n` items of a list whose opener was just emitted, then its closer.
    fn items_then_close(&mut self, n: usize, mut each: impl FnMut(&mut Self, usize)) {
        if n > 0 {
            self.push(Cmd::Indent);
            self.push(Cmd::SoftLine);
            for i in 0..n {
                if i > 0 {
                    self.separator();
                }
                each(self, i);
            }
            self.leading();
            self.push(Cmd::Dedent);
            self.push(Cmd::SoftLine);
        }
        self.tok();
    }

    /// `( … )`, `[ … ]`, `{ … }`: the next token is the opener.
    fn list(&mut self, n: usize, each: impl FnMut(&mut Self, usize)) {
        self.push(Cmd::GroupStart);
        self.tok();
        self.items_then_close(n, each);
        self.push(Cmd::GroupEnd);
    }

    /// The value of a step, field or member after its `=` (or a body after `each`, `try`,
    /// `error`): bracketed and simple values follow on the line and break inside;
    /// `let`, `if`, `try` and operator chains move to the next line, indented, when they do not
    /// fit.
    fn value(&mut self, e: &Expr) {
        match e.kind {
            ExprKind::Let { .. }
            | ExprKind::If { .. }
            | ExprKind::Try { .. }
            | ExprKind::Binary { .. }
            | ExprKind::As { .. }
            | ExprKind::Is { .. }
            | ExprKind::Error(_) => self.assign(e),
            _ => {
                self.push(Cmd::Space);
                self.expr(e);
            }
        }
    }

    /// `e` on this line when it fits, else on the next line, indented.
    fn assign(&mut self, e: &Expr) {
        self.push(Cmd::GroupStart);
        self.push(Cmd::Indent);
        self.push(Cmd::Line);
        self.expr(e);
        self.push(Cmd::Dedent);
        self.push(Cmd::GroupEnd);
    }

    fn document(&mut self, doc: &Document) {
        match doc {
            Document::Expression(e) => self.expr(e),
            Document::Section(s) => self.section(s),
        }
    }

    fn section(&mut self, s: &Section) {
        if let Some(attributes) = &s.attributes {
            self.expr(attributes);
            self.push(Cmd::Space);
        }
        self.tok(); // section
        self.push(Cmd::Space);
        self.name(&s.name);
        self.tok(); // ;
        for m in &s.members {
            self.push(Cmd::Blank);
            if let Some(attributes) = &m.attributes {
                self.expr(attributes);
                self.push(Cmd::Space);
            }
            if m.shared {
                self.tok();
                self.push(Cmd::Space);
            }
            self.name(&m.name);
            self.push(Cmd::Space);
            self.tok(); // =
            self.value(&m.value);
            self.tok(); // ;
        }
    }

    fn expr(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Literal(_)
            | ExprKind::Identifier { .. }
            | ExprKind::Intrinsic(_)
            | ExprKind::SectionAccess { .. }
            | ExprKind::NotImplemented => self.leaf(e.span.end),
            ExprKind::Parenthesized(inner) => self.list(1, |s, _| s.expr(inner)),
            ExprKind::Let { bindings, body } => self.let_expr(bindings, body),
            ExprKind::If { .. } => self.if_expr(e),
            ExprKind::Each(body) => {
                self.tok();
                self.value(body);
            }
            ExprKind::Function {
                params,
                return_type,
                body,
            } => {
                self.list(params.len(), |s, i| {
                    if let Some(p) = params.get(i) {
                        s.param(p);
                    }
                });
                if let Some(ty) = return_type {
                    self.push(Cmd::Space);
                    self.tok(); // as
                    self.push(Cmd::Space);
                    self.words(ty.span.end);
                }
                self.push(Cmd::Space);
                self.tok(); // =>
                // The body, not the parameters, moves to the next line.
                self.assign(body);
            }
            ExprKind::Invoke { function, args } => {
                self.expr(function);
                self.list(args.len(), |s, i| {
                    if let Some(a) = args.get(i) {
                        s.expr(a);
                    }
                });
            }
            ExprKind::Record(fields) => self.list(fields.len(), |s, i| {
                if let Some(f) = fields.get(i) {
                    s.field(f);
                }
            }),
            ExprKind::List(items) => self.list(items.len(), |s, i| match items.get(i) {
                Some(ListItem::Single(e)) => s.expr(e),
                Some(ListItem::Range { start, end }) => {
                    s.expr(start);
                    s.tok(); // ..
                    // `1.. .5` must not become `1...5`.
                    if s.peek().is_some_and(|t| t.text.starts_with('.')) {
                        s.push(Cmd::Space);
                    }
                    s.expr(end);
                }
                None => {}
            }),
            ExprKind::FieldAccess {
                target,
                field,
                optional,
            } => {
                if let Some(target) = target {
                    self.expr(target);
                }
                self.tok(); // [
                self.name(field);
                self.tok(); // ]
                if *optional {
                    self.tok();
                }
            }
            ExprKind::FieldProjection {
                target,
                fields,
                optional,
            } => {
                if let Some(target) = target {
                    self.expr(target);
                }
                self.list(fields.len(), |s, i| {
                    if let Some(f) = fields.get(i) {
                        s.tok(); // [
                        s.name(f);
                        s.tok(); // ]
                    }
                });
                if *optional {
                    self.tok();
                }
            }
            ExprKind::ItemAccess {
                target,
                selector,
                optional,
            } => {
                self.expr(target);
                self.tok(); // {
                self.expr(selector);
                self.tok(); // }
                if *optional {
                    self.tok();
                }
            }
            ExprKind::Unary { op, operand } => {
                self.tok();
                if *op == UnaryOp::Not {
                    self.push(Cmd::Space);
                }
                self.expr(operand);
            }
            ExprKind::Binary { op, .. } => self.binary(e, op.precedence(), 0),
            ExprKind::As { expr, ty } | ExprKind::Is { expr, ty } => {
                self.expr(expr);
                self.push(Cmd::Space);
                self.tok(); // as / is
                self.push(Cmd::Space);
                self.words(ty.span.end);
            }
            ExprKind::Type(ty) => {
                self.tok(); // type
                self.push(Cmd::Space);
                self.ty(ty);
            }
            ExprKind::Try { expr, handler } => {
                self.push(Cmd::GroupStart);
                self.tok(); // try
                self.value(expr);
                match handler {
                    None => {}
                    Some(TryHandler::Otherwise(default)) => {
                        self.push(Cmd::Indent);
                        self.push(Cmd::Line);
                        self.tok(); // otherwise
                        self.value(default);
                        self.push(Cmd::Dedent);
                    }
                    Some(TryHandler::Catch { param, body }) => {
                        self.push(Cmd::Indent);
                        self.push(Cmd::Line);
                        self.tok(); // catch
                        self.push(Cmd::Space);
                        self.tok(); // (
                        if let Some(p) = param {
                            self.name(p);
                        }
                        self.tok(); // )
                        self.push(Cmd::Space);
                        self.tok(); // =>
                        self.value(body);
                        self.push(Cmd::Dedent);
                    }
                }
                self.push(Cmd::GroupEnd);
            }
            ExprKind::Error(inner) => {
                self.tok();
                self.value(inner);
            }
        }
    }

    /// `name = value` of a record.
    fn field(&mut self, f: &RecordField) {
        self.name(&f.name);
        self.push(Cmd::Space);
        self.tok(); // =
        self.value(&f.value);
    }

    /// `optional? name (as nullable? type)?` of a function expression.
    fn param(&mut self, p: &Param) {
        if p.optional {
            self.tok();
            self.push(Cmd::Space);
        }
        self.name(&p.name);
        if let Some(ty) = &p.ty {
            self.push(Cmd::Space);
            self.tok(); // as
            self.push(Cmd::Space);
            self.words(ty.span.end);
        }
    }

    /// `let` with one step per line, then `in` and the result on the next line.
    fn let_expr(&mut self, bindings: &[Binding], body: &Expr) {
        self.tok(); // let
        self.push(Cmd::Indent);
        self.push(Cmd::HardLine);
        for (i, b) in bindings.iter().enumerate() {
            if i > 0 {
                if self.peek_is(&TokenKind::Comma) {
                    self.tok();
                }
                self.statement_break();
            }
            self.name(&b.name);
            self.push(Cmd::Space);
            self.tok(); // =
            self.value(&b.value);
        }
        self.leading();
        self.push(Cmd::Dedent);
        self.push(Cmd::HardLine);
        self.tok(); // in
        self.push(Cmd::Indent);
        self.push(Cmd::HardLine);
        self.expr(body);
        self.push(Cmd::Dedent);
    }

    /// `if … then … else …`, flat or with indented branches; `else if` chains stay at one level.
    fn if_expr(&mut self, e: &Expr) {
        self.push(Cmd::GroupStart);
        let mut e = e;
        while let ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } = &e.kind
        {
            self.tok(); // if
            self.push(Cmd::Space);
            self.expr(condition);
            self.push(Cmd::Space);
            self.tok(); // then
            self.push(Cmd::Indent);
            self.push(Cmd::Line);
            self.expr(then_branch);
            self.push(Cmd::Dedent);
            self.push(Cmd::Line);
            self.tok(); // else
            if matches!(else_branch.kind, ExprKind::If { .. }) {
                self.push(Cmd::Space);
                e = else_branch;
                continue;
            }
            self.push(Cmd::Indent);
            self.push(Cmd::Line);
            self.expr(else_branch);
            self.push(Cmd::Dedent);
            break;
        }
        self.push(Cmd::GroupEnd);
    }

    /// A chain of operators of one precedence (`a & b & c`): flat, or one operator and operand
    /// per continuation line. `extra` indents the continuation lines further: a tighter chain
    /// that is the first operand of another breaks one level deeper than it, so their operators
    /// never line up.
    fn binary(&mut self, e: &Expr, prec: u8, extra: usize) {
        let mut operands: Vec<&Expr> = Vec::new();
        let mut first = e;
        while let ExprKind::Binary { op, lhs, rhs } = &first.kind
            && op.precedence() == prec
        {
            operands.push(rhs);
            first = lhs;
        }
        self.push(Cmd::GroupStart);
        match &first.kind {
            ExprKind::Binary { op, .. } => {
                self.binary(first, op.precedence(), extra.saturating_add(1));
            }
            _ => self.expr(first),
        }
        for _ in 0..=extra {
            self.push(Cmd::Indent);
        }
        for operand in operands.into_iter().rev() {
            self.push(Cmd::Line);
            self.tok();
            self.push(Cmd::Space);
            self.expr(operand);
        }
        for _ in 0..=extra {
            self.push(Cmd::Dedent);
        }
        self.push(Cmd::GroupEnd);
    }

    fn ty(&mut self, t: &TypeExpr) {
        match &t.kind {
            TypeKind::Primitive(_) => self.leaf(t.span.end),
            TypeKind::Nullable(inner) => {
                self.tok(); // nullable
                self.push(Cmd::Space);
                self.ty(inner);
            }
            TypeKind::Record(record) => self.record_type(record),
            TypeKind::List(item) => {
                self.tok(); // {
                self.ty(item);
                self.tok(); // }
            }
            TypeKind::Function(f) => {
                self.tok(); // function
                self.push(Cmd::Space);
                self.list(f.params.len(), |s, i| {
                    if let Some(p) = f.params.get(i) {
                        s.param_spec(p);
                    }
                });
                self.push(Cmd::Space);
                self.tok(); // as
                self.push(Cmd::Space);
                self.ty(&f.return_type);
            }
            TypeKind::Table(row) => {
                self.tok(); // table
                self.push(Cmd::Space);
                match row {
                    TableRowType::Fields(record) => self.record_type(record),
                    TableRowType::Expr(e) => self.expr(e),
                }
            }
            TypeKind::Expr(e) => self.expr(e),
        }
    }

    /// `[a = text, optional b, ...]`.
    fn record_type(&mut self, record: &RecordType) {
        let n = record.fields.len() + usize::from(record.open);
        self.list(n, |s, i| match record.fields.get(i) {
            Some(f) => s.field_spec(f),
            None => s.tok(), // ...
        });
    }

    fn field_spec(&mut self, f: &FieldSpec) {
        if f.optional {
            self.tok();
            self.push(Cmd::Space);
        }
        self.name(&f.name);
        if let Some(ty) = &f.ty {
            self.push(Cmd::Space);
            self.tok(); // =
            self.push(Cmd::Space);
            self.ty(ty);
        }
    }

    fn param_spec(&mut self, p: &ParamSpec) {
        if p.optional {
            self.tok();
            self.push(Cmd::Space);
        }
        self.name(&p.name);
        if let Some(ty) = &p.ty {
            self.push(Cmd::Space);
            self.tok(); // as
            self.push(Cmd::Space);
            self.ty(ty);
        }
    }
}

// ---- rendering -------------------------------------------------------------

/// Width of anything that cannot be on one line.
const INF: usize = usize::MAX / 4;

fn has_line_break(s: &str) -> bool {
    s.contains(['\r', '\n', '\u{0085}', '\u{2028}', '\u{2029}'])
}

fn text_width(s: &str) -> usize {
    if has_line_break(s) {
        INF
    } else {
        s.chars().count()
    }
}

/// Width of the last line of `s` (the column after writing it at column 0 of a new line).
fn last_line_width(s: &str) -> usize {
    s.rsplit(['\r', '\n', '\u{0085}', '\u{2028}', '\u{2029}'])
        .next()
        .map_or(0, |l| l.chars().count())
}

fn first_line_width(s: &str) -> usize {
    s.split(['\r', '\n', '\u{0085}', '\u{2028}', '\u{2029}'])
        .next()
        .map_or(0, |l| l.chars().count())
}

/// Width of a command on a flat line.
fn flat_width(cmd: &Cmd<'_>) -> usize {
    match cmd {
        Cmd::Text(t) => text_width(t),
        Cmd::Space | Cmd::Line => 1,
        Cmd::Trailing(c) if !c.line => text_width(c.text).saturating_add(1),
        Cmd::HardLine | Cmd::Blank | Cmd::Leading(_) | Cmd::Trailing(_) => INF,
        Cmd::SoftLine | Cmd::GroupStart | Cmd::GroupEnd | Cmd::Indent | Cmd::Dedent => 0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Break {
    None,
    Space,
    Newline,
    Blank,
}

struct Writer {
    out: String,
    col: usize,
    line_has_text: bool,
    level: usize,
    pending: Break,
    unit: String,
    indent_width: usize,
}

impl Writer {
    fn raise(&mut self, b: Break) {
        self.pending = self.pending.max(b);
    }

    /// The column the next text starts at.
    fn next_col(&self) -> usize {
        match self.pending {
            Break::None => self.col,
            Break::Space => self.col.saturating_add(usize::from(self.line_has_text)),
            Break::Newline | Break::Blank => self.level.saturating_mul(self.indent_width),
        }
    }

    fn write(&mut self, text: &str) {
        match self.pending {
            Break::None => {}
            Break::Space => {
                if self.line_has_text {
                    self.out.push(' ');
                    self.col = self.col.saturating_add(1);
                }
            }
            Break::Newline | Break::Blank => {
                if !self.out.is_empty() {
                    self.out.push('\n');
                    if self.pending == Break::Blank {
                        self.out.push('\n');
                    }
                    for _ in 0..self.level {
                        self.out.push_str(&self.unit);
                    }
                    self.col = self.level.saturating_mul(self.indent_width);
                }
            }
        }
        self.pending = Break::None;
        self.out.push_str(text);
        if has_line_break(text) {
            self.col = last_line_width(text);
        } else {
            self.col = self.col.saturating_add(text.chars().count());
        }
        self.line_has_text = true;
    }
}

/// Lays the commands out (Wadler-style: a group is flat when it and the text after it up to the
/// next possible line break fit in the width).
fn render(cmds: &[Cmd<'_>], opts: &FormatOptions) -> String {
    let n = cmds.len();
    // Flat width and end of every group (indexed by its GroupStart).
    let mut width = vec![0usize; n];
    let mut end = vec![0usize; n];
    let mut open: Vec<(usize, usize)> = Vec::new();
    for (i, cmd) in cmds.iter().enumerate() {
        match cmd {
            Cmd::GroupStart => open.push((i, 0)),
            Cmd::GroupEnd => {
                if let Some((start, w)) = open.pop() {
                    if let Some(slot) = width.get_mut(start) {
                        *slot = w;
                    }
                    if let Some(slot) = end.get_mut(start) {
                        *slot = i;
                    }
                    if let Some(parent) = open.last_mut() {
                        parent.1 = parent.1.saturating_add(w).min(INF);
                    }
                }
            }
            _ => {
                if let Some(parent) = open.last_mut() {
                    parent.1 = parent.1.saturating_add(flat_width(cmd)).min(INF);
                }
            }
        }
    }
    // Width of the text after each command up to the next possible break.
    let mut rest = vec![0usize; n];
    let mut acc = 0usize;
    for (i, cmd) in cmds.iter().enumerate().rev() {
        if let Some(slot) = rest.get_mut(i) {
            *slot = acc;
        }
        acc = match cmd {
            Cmd::Text(t) if has_line_break(t) => first_line_width(t),
            Cmd::Text(t) => acc.saturating_add(text_width(t)),
            Cmd::Space => acc.saturating_add(1),
            Cmd::Trailing(c) if !c.line => {
                if has_line_break(c.text) {
                    first_line_width(c.text).saturating_add(1)
                } else {
                    acc.saturating_add(text_width(c.text)).saturating_add(1)
                }
            }
            Cmd::Line
            | Cmd::SoftLine
            | Cmd::HardLine
            | Cmd::Blank
            | Cmd::Leading(_)
            | Cmd::Trailing(_) => 0,
            Cmd::GroupStart | Cmd::GroupEnd | Cmd::Indent | Cmd::Dedent => acc,
        }
        .min(INF);
    }

    let mut w = Writer {
        out: String::new(),
        col: 0,
        line_has_text: false,
        level: 0,
        pending: Break::None,
        unit: if opts.use_tabs {
            "\t".to_owned()
        } else {
            " ".repeat(opts.indent_width)
        },
        indent_width: opts.indent_width,
    };
    let mut flat: Vec<bool> = Vec::new();
    for (i, cmd) in cmds.iter().enumerate() {
        let in_flat = flat.last().copied().unwrap_or(false);
        match cmd {
            Cmd::GroupStart => {
                let fits = in_flat || {
                    let group = width.get(i).copied().unwrap_or(INF);
                    let after = end.get(i).and_then(|&e| rest.get(e)).copied().unwrap_or(0);
                    group < INF
                        && w.next_col().saturating_add(group).saturating_add(after)
                            <= opts.max_width
                };
                flat.push(fits);
            }
            Cmd::GroupEnd => {
                flat.pop();
            }
            Cmd::Indent => w.level = w.level.saturating_add(1),
            Cmd::Dedent => w.level = w.level.saturating_sub(1),
            Cmd::Space => w.raise(Break::Space),
            Cmd::Line => w.raise(if in_flat {
                Break::Space
            } else {
                Break::Newline
            }),
            Cmd::SoftLine => {
                if !in_flat {
                    w.raise(Break::Newline);
                }
            }
            Cmd::HardLine => w.raise(Break::Newline),
            Cmd::Blank => w.raise(Break::Blank),
            Cmd::Text(t) => w.write(t),
            Cmd::Trailing(c) => {
                w.raise(Break::Space);
                w.write(c.text);
                if c.line {
                    w.raise(Break::Newline);
                }
            }
            Cmd::Leading(c) => {
                if c.blank_before {
                    w.raise(Break::Blank);
                } else if w.line_has_text {
                    w.raise(Break::Newline);
                }
                w.write(c.text);
                w.raise(if c.newline_after {
                    Break::Newline
                } else {
                    Break::Space
                });
            }
        }
    }
    w.out
}

// ---- equivalence check -----------------------------------------------------

/// The output has the input's tokens and comments, in order and spelled the same (line
/// comments without trailing blanks).
fn same_tokens(src: &str, lexed: &Lexed, out: &str) -> bool {
    let Ok(again) = tokenize(out) else {
        return false;
    };
    let spelled = |text: &str, all: &Lexed| -> Vec<(Option<TokenKind>, String)> {
        items(all)
            .into_iter()
            .map(|(start, end, item)| {
                let s = text.get(start..end).unwrap_or("");
                match item {
                    Item::Token(t) => (Some(t.kind.clone()), s.to_owned()),
                    Item::Comment(_) => (None, s.trim_end().to_owned()),
                }
            })
            .collect()
    };
    spelled(src, lexed) == spelled(out, &again)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(src: &str) -> String {
        fmt_with(src, &FormatOptions::default())
    }

    fn fmt_with(src: &str, opts: &FormatOptions) -> String {
        let out = format(src, opts).unwrap_or_else(|e| panic!("{e} for {src:?}"));
        assert_eq!(
            format(&out, opts).as_deref(),
            Ok(out.as_str()),
            "not idempotent for {src:?}"
        );
        out
    }

    fn narrow(width: usize) -> FormatOptions {
        FormatOptions {
            max_width: width,
            ..FormatOptions::default()
        }
    }

    #[test]
    fn queries_get_one_step_per_line() {
        let src = r#"let Source = Csv.Document(File.Contents("x.csv"),[Delimiter=",", Columns=3]), #"Promoted Headers" = Table.PromoteHeaders(Source, [PromoteAllScalars=true]),#"Changed Type" = Table.TransformColumnTypes(#"Promoted Headers",{{"a", type text}, {"b", Int64.Type}}) in #"Changed Type""#;
        assert_eq!(
            fmt(src),
            r#"let
    Source = Csv.Document(File.Contents("x.csv"), [Delimiter = ",", Columns = 3]),
    #"Promoted Headers" = Table.PromoteHeaders(Source, [PromoteAllScalars = true]),
    #"Changed Type" = Table.TransformColumnTypes(
        #"Promoted Headers",
        {{"a", type text}, {"b", Int64.Type}}
    )
in
    #"Changed Type""#
        );
        assert_eq!(
            fmt_with(src, &narrow(60)),
            r#"let
    Source = Csv.Document(
        File.Contents("x.csv"),
        [Delimiter = ",", Columns = 3]
    ),
    #"Promoted Headers" = Table.PromoteHeaders(
        Source,
        [PromoteAllScalars = true]
    ),
    #"Changed Type" = Table.TransformColumnTypes(
        #"Promoted Headers",
        {{"a", type text}, {"b", Int64.Type}}
    )
in
    #"Changed Type""#
        );
        // Short and nested lets; a step's `let` moves to its own line.
        assert_eq!(fmt("let a=1 in a"), "let\n    a = 1\nin\n    a");
        assert_eq!(
            fmt("let a = let b = 1 in b in a"),
            "let\n    a =\n        let\n            b = 1\n        in\n            b\nin\n    a"
        );
    }

    #[test]
    fn comments_and_blank_lines_stay() {
        let src = "// Orders\nlet\n    Source = 1, // first\n\n\n    /* the\n       second */\n    Next = Source  +  1\n    // before in\nin\n    Next /* done */\n// end";
        assert_eq!(
            fmt(src),
            "// Orders\nlet\n    Source = 1, // first\n\n    /* the\n       second */\n    Next = Source + 1\n    // before in\nin\n    Next /* done */\n// end"
        );
        let src = "let\n    a = 1,\n\n    b = 2\nin\n    b";
        assert_eq!(fmt(src), src);
        assert_eq!(fmt("f(a, // x\n b)"), "f(\n    a, // x\n    b\n)");
    }

    #[test]
    fn conditions_functions_and_errors() {
        assert_eq!(fmt("if a then 1 else 2"), "if a then 1 else 2");
        assert_eq!(
            fmt_with(
                "if a > 1 then \"big\" else if a > 0 then \"small\" else \"none\"",
                &narrow(30)
            ),
            "if a > 1 then\n    \"big\"\nelse if a > 0 then\n    \"small\"\nelse\n    \"none\""
        );
        assert_eq!(
            fmt("(x as number, optional y as nullable text) as number => x+1"),
            "(x as number, optional y as nullable text) as number => x + 1"
        );
        assert_eq!(
            fmt("(k) => let t = Text.Trim(k) in t"),
            "(k) =>\n    let\n        t = Text.Trim(k)\n    in\n        t"
        );
        assert_eq!(
            fmt_with(
                "(c as binary) as table => Table.PromoteHeaders(Csv.Document(c))",
                &narrow(42)
            ),
            "(c as binary) as table =>\n    Table.PromoteHeaders(Csv.Document(c))"
        );
        assert_eq!(
            fmt("Table.AddColumn(Source,\"x\",each [a]*[b])"),
            "Table.AddColumn(Source, \"x\", each [a] * [b])"
        );
        assert_eq!(
            fmt("try Number.From(x) otherwise null"),
            "try Number.From(x) otherwise null"
        );
        assert_eq!(
            fmt_with("try Number.From(x) catch (e) => e[Message]", &narrow(20)),
            "try Number.From(x)\n    catch (e) => e[Message]"
        );
        assert_eq!(fmt("error \"bad\""), "error \"bad\"");
        assert_eq!(
            fmt("not (a and b) or -x <> - -1"),
            "not (a and b) or -x <> --1"
        );
        assert_eq!(
            fmt_with("aaaa & bbbb & cccc * dddd * eeee", &narrow(12)),
            "aaaa\n    & bbbb\n    & cccc\n        * dddd\n        * eeee"
        );
    }

    #[test]
    fn records_lists_access_and_types() {
        assert_eq!(
            fmt("Source{[Name=\"orders.csv\"]}[Content]"),
            "Source{[Name = \"orders.csv\"]}[Content]"
        );
        assert_eq!(
            fmt("[Order   Date] & x[Ship  Date]? & [[a],[b c]]"),
            "[Order   Date] & x[Ship  Date]? & [[a], [b c]]"
        );
        assert_eq!(fmt("{1..10, a .. .5, {}}"), "{1..10, a.. .5, {}}");
        assert_eq!(
            fmt("type table [A=text, optional B=nullable number]"),
            "type table [A = text, optional B = nullable number]"
        );
        assert_eq!(
            fmt("type [a = {text}, ...] meta [Documentation.Name=\"x\"]"),
            "type [a = {text}, ...] meta [Documentation.Name = \"x\"]"
        );
        assert_eq!(
            fmt("type function (x as text, optional y as number) as text"),
            "type function (x as text, optional y as number) as text"
        );
        assert_eq!(
            fmt("x as nullable text is text"),
            "x as nullable text is text"
        );
        assert_eq!(
            fmt("#table({\"a\"},{{1},{2}})"),
            "#table({\"a\"}, {{1}, {2}})"
        );
        assert_eq!(
            fmt("@f(x) + Section1!Member + ..."),
            "@f(x) + Section1!Member + ..."
        );
        assert_eq!(
            fmt(
                "\"https://example.com\" meta [IsParameterQuery=true, Type=\"Text\", IsParameterQueryRequired=true]"
            ),
            "\"https://example.com\" meta [IsParameterQuery = true, Type = \"Text\", IsParameterQueryRequired = true]"
        );
        // A multi-line text literal keeps its line breaks and breaks its group.
        assert_eq!(fmt("f(\"a\nb\", 1)"), "f(\n    \"a\nb\",\n    1\n)");
    }

    #[test]
    fn sections_and_options() {
        assert_eq!(
            fmt(
                "[Version=\"1.0\"] section Section1; shared Orders = let a = 1 in a; [Hidden=true] B = 2;"
            ),
            "[Version = \"1.0\"] section Section1;\n\nshared Orders =\n    let\n        a = 1\n    in\n        a;\n\n[Hidden = true] B = 2;"
        );
        let two = FormatOptions {
            indent_width: 2,
            ..FormatOptions::default()
        };
        assert_eq!(fmt_with("let a=1 in a", &two), "let\n  a = 1\nin\n  a");
        let tabs = FormatOptions {
            use_tabs: true,
            ..FormatOptions::default()
        };
        assert_eq!(fmt_with("let a=1 in a", &tabs), "let\n\ta = 1\nin\n\ta");
        let wide = FormatOptions {
            indent_width: 1000,
            ..FormatOptions::default()
        };
        assert_eq!(
            fmt_with("let a=1 in a", &wide),
            format!("let\n{0}a = 1\nin\n{0}a", " ".repeat(MAX_INDENT_WIDTH))
        );
        // Width 0 breaks every group and stays stable.
        fmt_with("let a = f(1, [b = {2, 3}]) in a", &narrow(0));
    }

    #[test]
    fn blank_and_invalid_text() {
        assert_eq!(format("", &FormatOptions::default()), Ok(String::new()));
        assert_eq!(
            format(" \n\t", &FormatOptions::default()),
            Ok(String::new())
        );
        match format("let a = in a", &FormatOptions::default()) {
            Err(FormatError::Syntax(d)) => {
                assert_eq!(d.code, DiagnosticCode::Syntax);
                assert!(d.span.start >= 8, "{d:?}");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            format("\"open", &FormatOptions::default()),
            Err(FormatError::Syntax(_))
        ));
        // A BOM and CR LF line breaks go; text literal contents stay.
        assert_eq!(
            fmt("\u{feff}let\r\n  a = \"x\r\ny\"\r\nin\r\n  a"),
            "let\n    a = \"x\r\ny\"\nin\n    a"
        );
    }
}
