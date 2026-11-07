//! DAX formatter (`dax.format`): lays out an expression or query
//! deterministically and idempotently.
//!
//! The syntax tree decides where lines break, but the output is the source's
//! own tokens: only whitespace changes, plus the case of keywords and
//! built-in function names when [`FormatOptions`] asks for it. Names,
//! literals and user-defined function names stay as written. Comments stay
//! where they were: after a token on its line, or on their own line before
//! the next token. A blank line between `VAR`s, query definitions or before
//! a comment is kept.
//!
//! A group (a call, parentheses, a table constructor, a chain of operators
//! of one precedence, the value of a `VAR`) stays on one line when it fits in
//! [`FormatOptions::max_width`]. Otherwise it breaks: one argument per line
//! with the closing parenthesis on its own line, operators at the start of
//! the continuation lines, and the value of a `VAR` on the next line.
//! `RETURN`, `EVALUATE` and `DEFINE` put their content on the next line.
//!
//! The result is checked before it is returned: it must tokenize to the
//! same tokens and parse to the same tree as the input. Otherwise [`format`]
//! returns [`FormatError::Unstable`] rather than a text with another meaning.

use std::borrow::Cow;
use std::fmt;

use super::catalog;
use super::diagnostics::{Diagnostic, SourceKind, syntax};
use crate::eval::is_supported_function;
use crate::syntax::{
    Define, Evaluate, Expr, ExprKind, FunctionParam, Keyword, Query, Token, TokenKind, UnaryOp,
    VarBinding, parse_expression, parse_query, tokenize,
};

/// Letter case of keywords or function names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Casing {
    /// `CALCULATE`, `VAR`.
    #[default]
    Upper,
    /// `calculate`, `var`.
    Lower,
    /// As written.
    Keep,
}

impl Casing {
    fn apply(self, text: &str) -> Cow<'_, str> {
        match self {
            Casing::Upper if text.bytes().any(|b| b.is_ascii_lowercase()) => {
                Cow::Owned(text.to_ascii_uppercase())
            }
            Casing::Lower if text.bytes().any(|b| b.is_ascii_uppercase()) => {
                Cow::Owned(text.to_ascii_lowercase())
            }
            _ => Cow::Borrowed(text),
        }
    }
}

/// Largest indentation width [`format`] uses (wider values are capped).
pub const MAX_INDENT_WIDTH: usize = 16;

/// Layout options of [`format`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatOptions {
    /// Preferred maximum line length in characters (default 80). A line is
    /// longer only where a single token, comment or unbreakable run is.
    pub max_width: usize,
    /// Spaces per indentation level (default 4, at most
    /// [`MAX_INDENT_WIDTH`]), or the width a tab counts for with `use_tabs`.
    pub indent_width: usize,
    /// Indent with tabs instead of spaces.
    pub use_tabs: bool,
    /// Case of keywords: `VAR`, `RETURN`, `IN`, `NOT`, `TRUE`, `FALSE`,
    /// `DEFINE`, `EVALUATE`, `ORDER BY`, `START AT`, `MEASURE`, …, enum
    /// arguments such as `DESC` or `YEAR`, and function parameter types.
    pub keyword_case: Casing,
    /// Case of built-in function names (`CALCULATE`, `INFO.TABLES`);
    /// user-defined functions keep their spelling.
    pub function_case: Casing,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            max_width: 80,
            indent_width: 4,
            use_tabs: false,
            keyword_case: Casing::Upper,
            function_case: Casing::Upper,
        }
    }
}

/// Why [`format`] returned no text.
#[derive(Debug, Clone, PartialEq)]
pub enum FormatError {
    /// The text does not parse: the syntax error, as [`super::diagnostics`]
    /// reports it.
    Syntax(Diagnostic),
    /// The layout would have changed the text's tokens or syntax tree (a
    /// formatter bug); the text is better left as it is.
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

/// Formats `src`, a DAX expression or query. A blank text formats to an
/// empty one. The output uses `\n` line breaks and has no trailing line
/// break.
pub fn format(src: &str, kind: SourceKind, opts: &FormatOptions) -> Result<String, FormatError> {
    if src.trim().is_empty() {
        return Ok(String::new());
    }
    let opts = FormatOptions {
        indent_width: opts.indent_width.min(MAX_INDENT_WIDTH),
        ..*opts
    };
    let tokens = tokenize(src);
    let out = match kind {
        SourceKind::Expression => {
            let mut expr =
                parse_expression(src).map_err(|e| FormatError::Syntax(syntax(src, &e)))?;
            let mut emitter = Emitter::new(src, &tokens, &opts);
            emitter.expr(&expr);
            let out = emitter.finish();
            let mut again = parse_expression(&out).map_err(|_| FormatError::Unstable)?;
            normalize_expr(&mut expr);
            normalize_expr(&mut again);
            if expr != again {
                return Err(FormatError::Unstable);
            }
            out
        }
        SourceKind::Query => {
            let mut query = parse_query(src).map_err(|e| FormatError::Syntax(syntax(src, &e)))?;
            let mut emitter = Emitter::new(src, &tokens, &opts);
            emitter.query(&query);
            let out = emitter.finish();
            let mut again = parse_query(&out).map_err(|_| FormatError::Unstable)?;
            normalize_query(&mut query);
            normalize_query(&mut again);
            if query != again {
                return Err(FormatError::Unstable);
            }
            out
        }
    };
    if !same_tokens(src, &tokens, &out) {
        return Err(FormatError::Unstable);
    }
    Ok(out)
}

/// Whether DashCraft knows `name` as a built-in DAX function (any case).
fn is_builtin(name: &str) -> bool {
    catalog::function(name).is_some()
        || catalog::is_info_function(name)
        || is_supported_function(&name.to_ascii_uppercase())
}

// ---- comments and tokens ---------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Comment<'a> {
    text: &'a str,
    line: bool,
    /// A blank line separates it from what came before.
    blank_before: bool,
    /// A line break follows it in the source.
    newline_after: bool,
}

/// A significant token with the comments attached to it.
#[derive(Debug)]
struct Sig<'a> {
    kind: TokenKind,
    start: usize,
    text: &'a str,
    keyword: Option<Keyword>,
    /// Comments on their own lines before the token.
    leading: Vec<Comment<'a>>,
    /// Comments after the token on its line.
    trailing: Vec<Comment<'a>>,
    /// A blank line separates it from what came before (its last leading
    /// comment, or the previous token).
    blank_before: bool,
}

/// Splits the token stream into significant tokens with their comments, and
/// the comments after the last token on later lines.
fn attach<'a>(src: &'a str, tokens: &[Token]) -> (Vec<Sig<'a>>, Vec<Comment<'a>>) {
    let mut sigs: Vec<Sig<'a>> = Vec::new();
    let mut pending: Vec<Comment<'a>> = Vec::new();
    let mut newlines = 0usize;
    let mut newline_since_sig = false;
    for t in tokens {
        let text = t.text(src);
        match t.kind {
            TokenKind::Whitespace => {
                let n = text.matches('\n').count();
                newlines = newlines.saturating_add(n);
                newline_since_sig |= n > 0;
            }
            TokenKind::LineComment | TokenKind::BlockComment => {
                let line = t.kind == TokenKind::LineComment;
                let newline_after = line
                    || src
                        .get(t.span.end..)
                        .unwrap_or("")
                        .chars()
                        .take_while(|c| c.is_whitespace() || *c == '\u{feff}')
                        .any(|c| c == '\n');
                let comment = Comment {
                    text: if line { text.trim_end() } else { text },
                    line,
                    blank_before: newlines >= 2,
                    newline_after,
                };
                newlines = 0;
                match sigs.last_mut() {
                    Some(last) if !newline_since_sig => last.trailing.push(comment),
                    _ => pending.push(comment),
                }
            }
            _ => {
                sigs.push(Sig {
                    kind: t.kind,
                    start: t.span.start,
                    text,
                    keyword: t.keyword(src),
                    leading: std::mem::take(&mut pending),
                    trailing: Vec::new(),
                    blank_before: newlines >= 2,
                });
                newlines = 0;
                newline_since_sig = false;
            }
        }
    }
    (sigs, pending)
}

// ---- layout commands -------------------------------------------------------

/// A layout command. Groups render flat (`Line` as a space, `SoftLine` as
/// nothing) when they fit on the rest of the line, else broken (both as line
/// breaks).
#[derive(Debug)]
enum Cmd<'a> {
    Text(Cow<'a, str>),
    Space,
    Line,
    SoftLine,
    HardLine,
    /// A line break with an empty line before the next text.
    Blank,
    /// A comment on its own line.
    Leading(Comment<'a>),
    /// A comment after the text on the line.
    Trailing(Comment<'a>),
    GroupStart,
    GroupEnd,
    Indent,
    Dedent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    Keep,
    Keyword,
    Function,
}

struct Emitter<'a, 'o> {
    sigs: Vec<Sig<'a>>,
    tail: Vec<Comment<'a>>,
    pos: usize,
    cmds: Vec<Cmd<'a>>,
    /// Trailing comments of the last token, placed after any group or
    /// indentation that ends right after it.
    trailing: Vec<Comment<'a>>,
    opts: &'o FormatOptions,
}

impl<'a, 'o> Emitter<'a, 'o> {
    fn new(src: &'a str, tokens: &[Token], opts: &'o FormatOptions) -> Self {
        let (sigs, tail) = attach(src, tokens);
        Self {
            sigs,
            tail,
            pos: 0,
            cmds: Vec::new(),
            trailing: Vec::new(),
            opts,
        }
    }

    fn finish(mut self) -> String {
        // Tokens the layout did not place (none for a parsed text) still go
        // out, so nothing is lost; the equivalence check then decides.
        while self.pos < self.sigs.len() {
            self.push(Cmd::Space);
            self.tok(Style::Keep);
        }
        self.flush_trailing();
        let tail = std::mem::take(&mut self.tail);
        self.cmds.extend(tail.into_iter().map(Cmd::Leading));
        render(&self.cmds, self.opts)
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

    fn peek_kind(&self) -> Option<TokenKind> {
        self.peek().map(|s| s.kind)
    }

    fn peek_keyword(&self) -> Option<Keyword> {
        self.peek().and_then(|s| s.keyword)
    }

    /// Places the next token's own-line comments now, ahead of any group
    /// that starts with the token.
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
    fn tok(&mut self, style: Style) {
        self.leading();
        let Some(sig) = self.sigs.get_mut(self.pos) else {
            return;
        };
        self.pos += 1;
        let text = sig.text;
        let trailing = std::mem::take(&mut sig.trailing);
        let text = match style {
            Style::Keep => Cow::Borrowed(text),
            Style::Keyword => self.opts.keyword_case.apply(text),
            Style::Function => self.opts.function_case.apply(text),
        };
        self.push(Cmd::Text(text));
        self.trailing = trailing;
    }

    /// Emits every token that starts before `end`, without spaces.
    fn leaf(&mut self, end: usize, style: Style) {
        while self.peek().is_some_and(|s| s.start < end) {
            self.tok(style);
        }
    }

    /// A line break, with an empty line when the source had one before the
    /// next token.
    fn statement_break(&mut self) {
        let blank = self
            .peek()
            .is_some_and(|s| s.blank_before && s.leading.is_empty());
        self.push(if blank { Cmd::Blank } else { Cmd::HardLine });
    }

    /// `,` and a line break or space.
    fn separator(&mut self) {
        if self.peek_kind() == Some(TokenKind::Comma) {
            self.tok(Style::Keep);
        }
        self.push(Cmd::Line);
    }

    /// The items of a list whose opener was just emitted, then its closer.
    fn items_then_close<T>(&mut self, items: &[T], mut each: impl FnMut(&mut Self, &T)) {
        if !items.is_empty() {
            self.push(Cmd::Indent);
            self.push(Cmd::SoftLine);
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    self.separator();
                }
                each(self, item);
            }
            self.leading();
            self.push(Cmd::Dedent);
            self.push(Cmd::SoftLine);
        }
        self.tok(Style::Keep);
    }

    /// `( … )`, `{ … }`: the next token is the opener.
    fn list<T>(&mut self, items: &[T], each: impl FnMut(&mut Self, &T)) {
        self.push(Cmd::GroupStart);
        self.tok(Style::Keep);
        self.items_then_close(items, each);
        self.push(Cmd::GroupEnd);
    }

    /// `= value` of a `VAR` or definition: on the line when it fits, else
    /// indented on the next.
    fn assign(&mut self, e: &Expr) {
        self.push(Cmd::GroupStart);
        self.push(Cmd::Indent);
        self.push(Cmd::Line);
        self.expr(e);
        self.push(Cmd::Dedent);
        self.push(Cmd::GroupEnd);
    }

    fn expr(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Call { name, args, .. } => {
                let style = if is_builtin(name) {
                    Style::Function
                } else {
                    Style::Keep
                };
                self.push(Cmd::GroupStart);
                self.tok(style);
                self.tok(Style::Keep);
                self.items_then_close(args, Self::expr);
                self.push(Cmd::GroupEnd);
            }
            ExprKind::Var { bindings, body } => self.var_block(bindings, body),
            ExprKind::Unary { op, expr } => {
                let style = if *op == UnaryOp::Not {
                    Style::Keyword
                } else {
                    Style::Keep
                };
                self.tok(style);
                // `NOT (x)` is not `NOT(x)`, and `- -x` is not a comment.
                if *op == UnaryOp::Not
                    || matches!(self.peek_kind(), Some(TokenKind::Minus | TokenKind::Plus))
                {
                    self.push(Cmd::Space);
                }
                self.expr(expr);
            }
            ExprKind::Binary { op, .. } => self.binary(e, op.precedence(), 0),
            ExprKind::TableConstructor { rows } => self.list(rows, |s, row| s.row(row)),
            ExprKind::Tuple(items) => self.list(items, Self::expr),
            ExprKind::Paren(inner) => self.list(std::slice::from_ref(inner.as_ref()), Self::expr),
            ExprKind::Boolean(_) | ExprKind::Keyword(_) => self.leaf(e.span.end, Style::Keyword),
            ExprKind::Omitted => {}
            ExprKind::Number(_)
            | ExprKind::String(_)
            | ExprKind::DateTime(_)
            | ExprKind::TableRef { .. }
            | ExprKind::ColumnRef { .. }
            | ExprKind::VarRef(_)
            | ExprKind::Parameter(_) => self.leaf(e.span.end, Style::Keep),
        }
    }

    /// A table constructor row: a value, or `(a, b, …)`.
    fn row(&mut self, row: &[Expr]) {
        match row {
            [single] => self.expr(single),
            _ => self.list(row, Self::expr),
        }
    }

    /// A chain of operators of one precedence (`a + b - c`): flat, or one
    /// operator and operand per continuation line. `extra` indents the
    /// continuation lines further: a tighter chain that is the first operand
    /// of another breaks one level deeper than it, so their operators never
    /// line up.
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
            let style = if self.peek_keyword() == Some(Keyword::In) {
                Style::Keyword
            } else {
                Style::Keep
            };
            self.tok(style);
            self.push(Cmd::Space);
            self.expr(operand);
        }
        for _ in 0..=extra {
            self.push(Cmd::Dedent);
        }
        self.push(Cmd::GroupEnd);
    }

    fn var_block(&mut self, bindings: &[VarBinding], body: &Expr) {
        for (i, b) in bindings.iter().enumerate() {
            if i > 0 {
                self.statement_break();
            }
            self.tok(Style::Keyword); // VAR
            self.push(Cmd::Space);
            self.tok(Style::Keep); // name
            self.push(Cmd::Space);
            self.tok(Style::Keep); // =
            self.assign(&b.expr);
        }
        self.statement_break();
        self.tok(Style::Keyword); // RETURN
        self.push(Cmd::Indent);
        self.push(Cmd::HardLine);
        self.expr(body);
        self.push(Cmd::Dedent);
    }

    fn query(&mut self, q: &Query) {
        if !q.defines.is_empty() {
            self.tok(Style::Keyword); // DEFINE
            self.push(Cmd::Indent);
            for (i, d) in q.defines.iter().enumerate() {
                if i == 0 {
                    self.push(Cmd::HardLine);
                } else {
                    self.statement_break();
                }
                self.define(d);
            }
            self.push(Cmd::Dedent);
        }
        for (i, ev) in q.evaluates.iter().enumerate() {
            if i > 0 || !q.defines.is_empty() {
                self.push(Cmd::Blank);
            }
            self.evaluate(ev);
        }
    }

    fn define(&mut self, d: &Define) {
        self.tok(Style::Keyword); // MEASURE, COLUMN, TABLE, VAR, FUNCTION
        self.push(Cmd::Space);
        match d {
            Define::Measure { expr, .. } | Define::Column { expr, .. } => {
                self.tok(Style::Keep); // table
                self.tok(Style::Keep); // [name]
                self.push(Cmd::Space);
                self.tok(Style::Keep); // =
                self.assign(expr);
            }
            Define::Table { expr, .. } | Define::Var { expr, .. } => {
                self.tok(Style::Keep); // name
                self.push(Cmd::Space);
                self.tok(Style::Keep); // =
                self.assign(expr);
            }
            Define::Function { params, body, .. } => {
                self.tok(Style::Keep); // name
                self.push(Cmd::Space);
                self.tok(Style::Keep); // =
                self.push(Cmd::Space);
                self.list(params, Self::param);
                self.push(Cmd::Space);
                self.tok(Style::Keep); // =>
                self.assign(body);
            }
        }
    }

    /// `name [: TYPE WORDS]`.
    fn param(&mut self, p: &FunctionParam) {
        self.tok(Style::Keep);
        if p.type_hint.is_some() {
            self.push(Cmd::Space);
            self.tok(Style::Keep); // :
            while self.peek().is_some_and(|s| s.start < p.span.end) {
                self.push(Cmd::Space);
                self.tok(Style::Keyword);
            }
        }
    }

    fn evaluate(&mut self, ev: &Evaluate) {
        self.tok(Style::Keyword); // EVALUATE
        self.push(Cmd::Indent);
        self.push(Cmd::HardLine);
        self.expr(&ev.expr);
        self.push(Cmd::Dedent);
        if ev.order_by.is_empty() {
            return;
        }
        let keys: Vec<&Expr> = ev.order_by.iter().map(|(key, _)| key).collect();
        self.clause(&keys, true);
        if !ev.start_at.is_empty() {
            let values: Vec<&Expr> = ev.start_at.iter().collect();
            self.clause(&values, false);
        }
    }

    /// `ORDER BY key [ASC|DESC], …` or `START AT value, …` on a new line:
    /// flat, or one item per indented line.
    fn clause(&mut self, items: &[&Expr], sort_keys: bool) {
        self.push(Cmd::HardLine);
        self.push(Cmd::GroupStart);
        self.tok(Style::Keyword); // ORDER / START
        self.push(Cmd::Space);
        self.tok(Style::Keyword); // BY / AT
        self.push(Cmd::Indent);
        for (i, item) in items.iter().enumerate() {
            if i == 0 {
                self.push(Cmd::Line);
            } else {
                self.separator();
            }
            self.expr(item);
            if sort_keys && matches!(self.peek_keyword(), Some(Keyword::Asc | Keyword::Desc)) {
                self.push(Cmd::Space);
                self.tok(Style::Keyword);
            }
        }
        self.push(Cmd::Dedent);
        self.push(Cmd::GroupEnd);
    }
}

// ---- rendering -------------------------------------------------------------

/// Width of anything that cannot be on one line.
const INF: usize = usize::MAX / 4;

fn text_width(s: &str) -> usize {
    if s.contains('\n') {
        INF
    } else {
        s.chars().count()
    }
}

fn first_line_width(s: &str) -> usize {
    s.split('\n').next().map_or(0, |l| l.chars().count())
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
            Break::Space => self.col + usize::from(self.line_has_text),
            Break::Newline | Break::Blank => self.level.saturating_mul(self.indent_width),
        }
    }

    fn write(&mut self, text: &str) {
        match self.pending {
            Break::None => {}
            Break::Space => {
                if self.line_has_text {
                    self.out.push(' ');
                    self.col += 1;
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
                    self.line_has_text = false;
                }
            }
        }
        self.pending = Break::None;
        self.out.push_str(text);
        match text.rfind('\n') {
            Some(at) => self.col = text.get(at + 1..).map_or(0, |t| t.chars().count()),
            None => self.col += text.chars().count(),
        }
        self.line_has_text = true;
    }
}

/// Lays the commands out (Wadler-style: a group is flat when it and the
/// text after it up to the next possible line break fit in the width).
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
            Cmd::Text(t) if t.contains('\n') => first_line_width(t),
            Cmd::Text(t) => acc.saturating_add(text_width(t)),
            Cmd::Space => acc.saturating_add(1),
            Cmd::Trailing(c) if !c.line => {
                if c.text.contains('\n') {
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
            Cmd::Indent => w.level += 1,
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

/// The output has the input's tokens, in order: names and literals exactly,
/// keywords and function names in any case, line comments without trailing
/// blanks.
fn same_tokens(src: &str, tokens: &[Token], out: &str) -> bool {
    let out_tokens = tokenize(out);
    let mut after = out_tokens
        .iter()
        .filter(|t| t.kind != TokenKind::Whitespace);
    for a in tokens.iter().filter(|t| t.kind != TokenKind::Whitespace) {
        let Some(b) = after.next() else {
            return false;
        };
        let (x, y) = (a.text(src), b.text(out));
        let same = a.kind == b.kind
            && match a.kind {
                TokenKind::Ident => x.eq_ignore_ascii_case(y),
                TokenKind::LineComment => x.trim_end() == y.trim_end(),
                _ => x == y,
            };
        if !same {
            return false;
        }
    }
    after.next().is_none()
}

/// Clears spans and the spelling of built-in function names (which casing
/// may change); user-defined function names must stay as written.
fn normalize_expr(e: &mut Expr) {
    e.clear_spans();
    let mut stack: Vec<&mut Expr> = vec![e];
    while let Some(e) = stack.pop() {
        if let ExprKind::Call { name, raw_name, .. } = &mut e.kind
            && is_builtin(name)
        {
            raw_name.clone_from(name);
        }
        stack.extend(e.children_mut());
    }
}

fn normalize_query(q: &mut Query) {
    q.clear_spans();
    for d in &mut q.defines {
        let expr = match d {
            Define::Measure { expr, .. }
            | Define::Var { expr, .. }
            | Define::Table { expr, .. }
            | Define::Column { expr, .. } => expr,
            Define::Function { params, body, .. } => {
                for p in params.iter_mut() {
                    if let Some(t) = &mut p.type_hint {
                        *t = t.to_ascii_uppercase();
                    }
                }
                body
            }
        };
        normalize_expr(expr);
    }
    for ev in &mut q.evaluates {
        normalize_expr(&mut ev.expr);
        for (key, _) in &mut ev.order_by {
            normalize_expr(key);
        }
        for value in &mut ev.start_at {
            normalize_expr(value);
        }
    }
}
