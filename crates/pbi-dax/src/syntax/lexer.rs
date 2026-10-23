//! Hand-written, lossless DAX lexer.
//!
//! [`tokenize`] never fails: every byte of the input is covered by exactly one
//! token, including whitespace and comments (trivia) and
//! [`TokenKind::Error`] tokens for malformed input, so the token stream can
//! back syntax highlighting in an editor. The parser skips trivia and reports
//! the first error token.

use super::ast::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    // trivia
    Whitespace,
    /// `// …` or `-- …` up to (not including) the line break.
    LineComment,
    /// `/* … */`
    BlockComment,
    // names and literals
    /// Unquoted identifier: table, variable, function (`PERCENTILE.INC`) or
    /// keyword (see [`Token::keyword`]). Letters, digits, `_` and `.`.
    Ident,
    /// `'Web Sessions'` (`''` escapes a quote).
    QuotedIdent,
    /// `[Order Count]` (`]]` escapes a bracket).
    BracketIdent,
    /// `"text"` (`""` escapes a quote).
    String,
    /// `dt"2024-01-31"`
    DateTime,
    /// `1`, `1.5`, `.5`, `1e3`, `2.5E-3`
    Number,
    /// Query parameter `@name`.
    Parameter,
    // operators and punctuation
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Amp,
    AmpAmp,
    PipePipe,
    Eq,
    EqEq,
    Neq,
    Lt,
    Gt,
    Le,
    Ge,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Dot,
    Colon,
    /// `=>` (user-defined function body)
    Arrow,
    /// Malformed input.
    Error(LexError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LexError {
    UnterminatedString,
    UnterminatedQuotedIdent,
    UnterminatedBracketIdent,
    UnterminatedBlockComment,
    /// A single `|` (DAX only has `||`).
    SinglePipe,
    UnexpectedChar,
}

impl LexError {
    pub const fn message(self) -> &'static str {
        match self {
            LexError::UnterminatedString => "unterminated string literal",
            LexError::UnterminatedQuotedIdent => "unterminated quoted name",
            LexError::UnterminatedBracketIdent => "unterminated bracketed name",
            LexError::UnterminatedBlockComment => "unterminated block comment",
            LexError::SinglePipe => "unexpected '|' (did you mean '||'?)",
            LexError::UnexpectedChar => "unexpected character",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn text<'a>(&self, src: &'a str) -> &'a str {
        self.span.text(src)
    }

    pub fn is_trivia(&self) -> bool {
        matches!(
            self.kind,
            TokenKind::Whitespace | TokenKind::LineComment | TokenKind::BlockComment
        )
    }

    /// Keyword classification of an [`TokenKind::Ident`] token.
    pub fn keyword(&self, src: &str) -> Option<Keyword> {
        match self.kind {
            TokenKind::Ident => Keyword::from_ident(self.text(src)),
            _ => None,
        }
    }

    /// Decoded value of a name or literal token: unescaped string /
    /// quoted-name / bracketed-name contents, the name after `@`, the text
    /// inside `dt"…"`, or the raw text for anything else.
    pub fn value(&self, src: &str) -> String {
        let text = self.text(src);
        match self.kind {
            TokenKind::String => unescape(strip(text, 1, 1), "\"\"", "\""),
            TokenKind::QuotedIdent => unescape(strip(text, 1, 1), "''", "'"),
            TokenKind::BracketIdent => unescape(strip(text, 1, 1), "]]", "]"),
            TokenKind::DateTime => unescape(strip(text, 3, 1), "\"\"", "\""),
            TokenKind::Parameter => strip(text, 1, 0).to_string(),
            _ => text.to_string(),
        }
    }
}

fn strip(text: &str, front: usize, back: usize) -> &str {
    text.get(front..text.len().saturating_sub(back))
        .unwrap_or("")
}

fn unescape(text: &str, escaped: &str, plain: &str) -> String {
    text.replace(escaped, plain)
}

/// Case-insensitive DAX keywords. `VAR RETURN IN NOT TRUE FALSE DEFINE
/// EVALUATE ORDER START` are reserved in expressions; the rest are contextual
/// (only special inside queries) and otherwise behave like identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Keyword {
    Var,
    Return,
    In,
    Not,
    True,
    False,
    Define,
    Evaluate,
    Order,
    By,
    Asc,
    Desc,
    Start,
    At,
    Measure,
    Column,
    Table,
    Function,
}

impl Keyword {
    const ALL: [Keyword; 18] = [
        Keyword::Var,
        Keyword::Return,
        Keyword::In,
        Keyword::Not,
        Keyword::True,
        Keyword::False,
        Keyword::Define,
        Keyword::Evaluate,
        Keyword::Order,
        Keyword::By,
        Keyword::Asc,
        Keyword::Desc,
        Keyword::Start,
        Keyword::At,
        Keyword::Measure,
        Keyword::Column,
        Keyword::Table,
        Keyword::Function,
    ];

    pub fn from_ident(text: &str) -> Option<Keyword> {
        Self::ALL
            .into_iter()
            .find(|k| k.as_str().eq_ignore_ascii_case(text))
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Keyword::Var => "VAR",
            Keyword::Return => "RETURN",
            Keyword::In => "IN",
            Keyword::Not => "NOT",
            Keyword::True => "TRUE",
            Keyword::False => "FALSE",
            Keyword::Define => "DEFINE",
            Keyword::Evaluate => "EVALUATE",
            Keyword::Order => "ORDER",
            Keyword::By => "BY",
            Keyword::Asc => "ASC",
            Keyword::Desc => "DESC",
            Keyword::Start => "START",
            Keyword::At => "AT",
            Keyword::Measure => "MEASURE",
            Keyword::Column => "COLUMN",
            Keyword::Table => "TABLE",
            Keyword::Function => "FUNCTION",
        }
    }

    /// Reserved words cannot be used as bare table or variable names.
    pub const fn is_reserved(self) -> bool {
        matches!(
            self,
            Keyword::Var
                | Keyword::Return
                | Keyword::In
                | Keyword::Not
                | Keyword::True
                | Keyword::False
                | Keyword::Define
                | Keyword::Evaluate
                | Keyword::Order
                | Keyword::Start
        )
    }
}

/// Bare identifiers that DAX functions accept as enum-like arguments
/// (`TOPN(…, DESC)`, `RANKX(…, DENSE)`, `DATEADD(…, YEAR)`,
/// `DATATABLE(…, STRING, …)`, `CROSSFILTER(…, BOTH)`, `WINDOW(…, ABS, …)`).
/// When not followed by `(` or `[` and not shadowed by a variable they parse
/// as [`super::ast::ExprKind::Keyword`] rather than a table reference.
const ENUM_KEYWORDS: &[&str] = &[
    "ASC",
    "DESC",
    "SKIP",
    "DENSE",
    "BOTH",
    "NONE",
    "ONEWAY",
    "ONEWAY_LEFTFILTERSRIGHT",
    "ONEWAY_RIGHTFILTERSLEFT",
    "INTEGER",
    "DOUBLE",
    "STRING",
    "BOOLEAN",
    "CURRENCY",
    "DATETIME",
    "SECOND",
    "MINUTE",
    "HOUR",
    "DAY",
    "WEEK",
    "MONTH",
    "QUARTER",
    "YEAR",
    "ABS",
    "REL",
    "DEFAULT",
    "FIRST",
    "LAST",
    "KEEP",
    "LOWESTPARENT",
    "HIGHESTPARENT",
];

/// Whether `name` (case-insensitive) is an enum-like keyword argument.
pub fn is_enum_keyword(name: &str) -> bool {
    ENUM_KEYWORDS.iter().any(|k| k.eq_ignore_ascii_case(name))
}

pub(crate) fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

pub(crate) fn is_ident_continue(c: char) -> bool {
    c == '_' || c == '.' || c.is_alphanumeric()
}

fn is_whitespace(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

/// Tokenizes `src` losslessly (trivia included). Never fails or panics.
pub fn tokenize(src: &str) -> Vec<Token> {
    let mut lexer = Lexer { src, pos: 0 };
    let mut tokens = Vec::new();
    while lexer.pos < src.len() {
        let start = lexer.pos;
        let kind = lexer.next_kind();
        if lexer.pos <= start {
            // Defensive: guarantee progress even on an internal bug.
            lexer.bump();
        }
        tokens.push(Token {
            kind,
            span: Span::new(start, lexer.pos),
        });
    }
    tokens
}

struct Lexer<'a> {
    src: &'a str,
    pos: usize,
}

impl Lexer<'_> {
    fn rest(&self) -> &str {
        self.src.get(self.pos..).unwrap_or("")
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn peek2(&self) -> Option<char> {
        self.rest().chars().nth(1)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += c.len_utf8();
            true
        } else {
            false
        }
    }

    fn eat_while(&mut self, f: impl Fn(char) -> bool) {
        while let Some(c) = self.peek() {
            if !f(c) {
                break;
            }
            self.pos += c.len_utf8();
        }
    }

    /// Consumes a delimited literal whose closing delimiter is escaped by
    /// doubling. The opening delimiter has already been consumed.
    fn delimited(&mut self, close: char, unterminated: LexError, ok: TokenKind) -> TokenKind {
        loop {
            match self.bump() {
                None => return TokenKind::Error(unterminated),
                Some(c) if c == close => {
                    if !self.eat(close) {
                        return ok;
                    }
                }
                Some(_) => {}
            }
        }
    }

    fn next_kind(&mut self) -> TokenKind {
        let Some(c) = self.bump() else {
            return TokenKind::Error(LexError::UnexpectedChar);
        };
        match c {
            c if is_whitespace(c) => {
                self.eat_while(is_whitespace);
                TokenKind::Whitespace
            }
            '/' if self.peek() == Some('/') => self.line_comment(),
            '-' if self.peek() == Some('-') => self.line_comment(),
            '/' if self.peek() == Some('*') => {
                self.bump();
                loop {
                    match self.bump() {
                        None => return TokenKind::Error(LexError::UnterminatedBlockComment),
                        Some('*') if self.eat('/') => return TokenKind::BlockComment,
                        Some(_) => {}
                    }
                }
            }
            '"' => self.delimited('"', LexError::UnterminatedString, TokenKind::String),
            '\'' => self.delimited(
                '\'',
                LexError::UnterminatedQuotedIdent,
                TokenKind::QuotedIdent,
            ),
            '[' => self.delimited(
                ']',
                LexError::UnterminatedBracketIdent,
                TokenKind::BracketIdent,
            ),
            '0'..='9' => self.number(),
            '.' if self.peek().is_some_and(|d| d.is_ascii_digit()) => self.number(),
            '@' if self.peek().is_some_and(is_ident_start) => {
                self.eat_while(is_ident_continue);
                TokenKind::Parameter
            }
            c if is_ident_start(c) => {
                let start = self.pos - c.len_utf8();
                self.eat_while(is_ident_continue);
                let text = self.src.get(start..self.pos).unwrap_or("");
                if text.eq_ignore_ascii_case("dt") && self.eat('"') {
                    return self.delimited('"', LexError::UnterminatedString, TokenKind::DateTime);
                }
                TokenKind::Ident
            }
            '+' => TokenKind::Plus,
            '-' => TokenKind::Minus,
            '*' => TokenKind::Star,
            '/' => TokenKind::Slash,
            '^' => TokenKind::Caret,
            '&' => {
                if self.eat('&') {
                    TokenKind::AmpAmp
                } else {
                    TokenKind::Amp
                }
            }
            '|' => {
                if self.eat('|') {
                    TokenKind::PipePipe
                } else {
                    TokenKind::Error(LexError::SinglePipe)
                }
            }
            '=' => {
                if self.eat('=') {
                    TokenKind::EqEq
                } else if self.eat('>') {
                    TokenKind::Arrow
                } else {
                    TokenKind::Eq
                }
            }
            '<' => {
                if self.eat('>') {
                    TokenKind::Neq
                } else if self.eat('=') {
                    TokenKind::Le
                } else {
                    TokenKind::Lt
                }
            }
            '>' => {
                if self.eat('=') {
                    TokenKind::Ge
                } else {
                    TokenKind::Gt
                }
            }
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            ',' => TokenKind::Comma,
            '.' => TokenKind::Dot,
            ':' => TokenKind::Colon,
            _ => TokenKind::Error(LexError::UnexpectedChar),
        }
    }

    fn line_comment(&mut self) -> TokenKind {
        self.eat_while(|c| c != '\n' && c != '\r');
        TokenKind::LineComment
    }

    /// First char (a digit or `.`) has been consumed.
    fn number(&mut self) -> TokenKind {
        self.eat_while(|c| c.is_ascii_digit());
        if self.eat('.') {
            self.eat_while(|c| c.is_ascii_digit());
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            let has_exponent = match self.peek2() {
                Some(d) if d.is_ascii_digit() => true,
                Some('+' | '-') => self
                    .rest()
                    .chars()
                    .nth(2)
                    .is_some_and(|d| d.is_ascii_digit()),
                _ => false,
            };
            if has_exponent {
                self.bump();
                if !self.eat('+') {
                    self.eat('-');
                }
                self.eat_while(|c| c.is_ascii_digit());
            }
        }
        TokenKind::Number
    }
}
