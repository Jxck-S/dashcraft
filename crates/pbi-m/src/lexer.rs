//! Lexer for the Power Query M formula language.
//!
//! Follows the lexical grammar of the M language specification: whitespace, `//` and `/* */`
//! comments, (dotted) identifiers, quoted identifiers `#"..."`, keywords (including the `#`
//! keywords such as `#table`), decimal/hexadecimal number literals, text literals with `""` and
//! `#(...)` character escapes, verbatim literals `#!"..."`, and operators/punctuators.
//!
//! Comments are not tokens; they are returned in a side list ([`Lexed::comments`]) so that a
//! formatter can re-attach them.

use std::fmt;

use crate::error::ParseError;
use crate::span::Span;

/// M keywords (case-sensitive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Keyword {
    And,
    As,
    Each,
    Else,
    Error,
    False,
    If,
    In,
    Is,
    Let,
    Meta,
    Not,
    Null,
    Or,
    Otherwise,
    Section,
    Shared,
    Then,
    True,
    Try,
    Type,
    HashBinary,
    HashDate,
    HashDateTime,
    HashDateTimeZone,
    HashDuration,
    HashInfinity,
    HashNan,
    HashSections,
    HashShared,
    HashTable,
    HashTime,
}

impl Keyword {
    pub const ALL: [Keyword; 32] = [
        Keyword::And,
        Keyword::As,
        Keyword::Each,
        Keyword::Else,
        Keyword::Error,
        Keyword::False,
        Keyword::If,
        Keyword::In,
        Keyword::Is,
        Keyword::Let,
        Keyword::Meta,
        Keyword::Not,
        Keyword::Null,
        Keyword::Or,
        Keyword::Otherwise,
        Keyword::Section,
        Keyword::Shared,
        Keyword::Then,
        Keyword::True,
        Keyword::Try,
        Keyword::Type,
        Keyword::HashBinary,
        Keyword::HashDate,
        Keyword::HashDateTime,
        Keyword::HashDateTimeZone,
        Keyword::HashDuration,
        Keyword::HashInfinity,
        Keyword::HashNan,
        Keyword::HashSections,
        Keyword::HashShared,
        Keyword::HashTable,
        Keyword::HashTime,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Keyword::And => "and",
            Keyword::As => "as",
            Keyword::Each => "each",
            Keyword::Else => "else",
            Keyword::Error => "error",
            Keyword::False => "false",
            Keyword::If => "if",
            Keyword::In => "in",
            Keyword::Is => "is",
            Keyword::Let => "let",
            Keyword::Meta => "meta",
            Keyword::Not => "not",
            Keyword::Null => "null",
            Keyword::Or => "or",
            Keyword::Otherwise => "otherwise",
            Keyword::Section => "section",
            Keyword::Shared => "shared",
            Keyword::Then => "then",
            Keyword::True => "true",
            Keyword::Try => "try",
            Keyword::Type => "type",
            Keyword::HashBinary => "#binary",
            Keyword::HashDate => "#date",
            Keyword::HashDateTime => "#datetime",
            Keyword::HashDateTimeZone => "#datetimezone",
            Keyword::HashDuration => "#duration",
            Keyword::HashInfinity => "#infinity",
            Keyword::HashNan => "#nan",
            Keyword::HashSections => "#sections",
            Keyword::HashShared => "#shared",
            Keyword::HashTable => "#table",
            Keyword::HashTime => "#time",
        }
    }

    /// Looks up a keyword by its exact (case-sensitive) spelling, including the leading `#` for
    /// `#` keywords.
    pub fn lookup(text: &str) -> Option<Keyword> {
        Keyword::ALL.iter().copied().find(|k| k.as_str() == text)
    }

    /// `true` for the `#binary`, `#date`, ... keywords.
    pub const fn is_hash(self) -> bool {
        matches!(
            self,
            Keyword::HashBinary
                | Keyword::HashDate
                | Keyword::HashDateTime
                | Keyword::HashDateTimeZone
                | Keyword::HashDuration
                | Keyword::HashInfinity
                | Keyword::HashNan
                | Keyword::HashSections
                | Keyword::HashShared
                | Keyword::HashTable
                | Keyword::HashTime
        )
    }
}

impl fmt::Display for Keyword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The kind (and decoded payload) of a token.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    /// A regular identifier, possibly dotted (`Table.SelectRows`, `QuoteStyle.Csv`).
    Identifier(String),
    /// A quoted identifier `#"..."`; the payload is the decoded name.
    QuotedIdentifier(String),
    Keyword(Keyword),
    /// A decimal or hexadecimal number literal; the raw text is available from the span.
    Number(f64),
    /// A text literal; the payload is the decoded text.
    Text(String),
    /// A verbatim literal `#!"..."`; the payload is the decoded text.
    Verbatim(String),
    Comma,
    Semicolon,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Plus,
    Minus,
    Star,
    Slash,
    Ampersand,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    At,
    Bang,
    Question,
    QuestionQuestion,
    FatArrow,
    DotDot,
    Ellipsis,
    Eof,
}

impl TokenKind {
    /// The fixed spelling of punctuator tokens.
    pub const fn punct_str(&self) -> Option<&'static str> {
        Some(match self {
            TokenKind::Comma => ",",
            TokenKind::Semicolon => ";",
            TokenKind::Equal => "=",
            TokenKind::NotEqual => "<>",
            TokenKind::Less => "<",
            TokenKind::LessEqual => "<=",
            TokenKind::Greater => ">",
            TokenKind::GreaterEqual => ">=",
            TokenKind::Plus => "+",
            TokenKind::Minus => "-",
            TokenKind::Star => "*",
            TokenKind::Slash => "/",
            TokenKind::Ampersand => "&",
            TokenKind::LParen => "(",
            TokenKind::RParen => ")",
            TokenKind::LBracket => "[",
            TokenKind::RBracket => "]",
            TokenKind::LBrace => "{",
            TokenKind::RBrace => "}",
            TokenKind::At => "@",
            TokenKind::Bang => "!",
            TokenKind::Question => "?",
            TokenKind::QuestionQuestion => "??",
            TokenKind::FatArrow => "=>",
            TokenKind::DotDot => "..",
            TokenKind::Ellipsis => "...",
            _ => return None,
        })
    }

    /// A short description for error messages, e.g. `` `)` `` or ``identifier `x` ``.
    pub fn describe(&self) -> String {
        match self {
            TokenKind::Identifier(s) => format!("identifier `{s}`"),
            TokenKind::QuotedIdentifier(s) => format!("quoted identifier `#\"{s}\"`"),
            TokenKind::Keyword(k) => format!("keyword `{k}`"),
            TokenKind::Number(_) => "number literal".to_owned(),
            TokenKind::Text(_) => "text literal".to_owned(),
            TokenKind::Verbatim(_) => "verbatim literal".to_owned(),
            TokenKind::Eof => "end of input".to_owned(),
            other => format!("`{}`", other.punct_str().unwrap_or("?")),
        }
    }
}

/// A token with its source span.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommentKind {
    /// `// ...` (the span excludes the terminating new-line).
    Line,
    /// `/* ... */`
    Block,
}

/// A comment (trivia), kept so that formatters can preserve it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub kind: CommentKind,
    pub span: Span,
    /// The full comment text including its delimiters.
    pub text: String,
}

/// The result of [`tokenize`]: tokens (always terminated by a [`TokenKind::Eof`] token) and the
/// comments that were skipped.
#[derive(Debug, Clone, PartialEq)]
pub struct Lexed {
    pub tokens: Vec<Token>,
    pub comments: Vec<Comment>,
}

/// Splits M source text into tokens.
pub fn tokenize(src: &str) -> Result<Lexed, ParseError> {
    let mut lexer = Lexer {
        src,
        pos: 0,
        comments: Vec::new(),
    };
    if src.starts_with('\u{FEFF}') {
        lexer.pos = '\u{FEFF}'.len_utf8();
    }
    let mut tokens = Vec::new();
    loop {
        let token = lexer.next_token()?;
        let eof = token.kind == TokenKind::Eof;
        tokens.push(token);
        if eof {
            break;
        }
    }
    Ok(Lexed {
        tokens,
        comments: lexer.comments,
    })
}

/// `identifier-start-character`: letters (`Lu Ll Lt Lm Lo Nl`) and `_`.
pub fn is_identifier_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

/// `identifier-part-character`: letters, decimal digits, `_`, and connecting (`Pc`), combining
/// (`Mn Mc`) and formatting (`Cf`) characters.
pub fn is_identifier_part(c: char) -> bool {
    c == '_' || c.is_alphanumeric() || is_connecting_combining_or_formatting(c)
}

/// Approximation of the `Pc`, `Mn`, `Mc` and `Cf` general categories for characters that are not
/// already alphanumeric (the standard library does not expose general categories).
fn is_connecting_combining_or_formatting(c: char) -> bool {
    matches!(
        c as u32,
        0x00AD
            | 0x0300..=0x036F
            | 0x0483..=0x0489
            | 0x0591..=0x05BD
            | 0x0610..=0x061A
            | 0x064B..=0x065F
            | 0x0670
            | 0x06D6..=0x06DC
            | 0x0900..=0x0903
            | 0x093A..=0x094F
            | 0x0E31..=0x0E3A
            | 0x0E47..=0x0E4E
            | 0x1AB0..=0x1AFF
            | 0x1DC0..=0x1DFF
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x203F..=0x2040
            | 0x2054
            | 0x2060..=0x2064
            | 0x20D0..=0x20FF
            | 0x302A..=0x302F
            | 0x3099..=0x309A
            | 0xFE00..=0xFE0F
            | 0xFE20..=0xFE2F
            | 0xFE33..=0xFE34
            | 0xFE4D..=0xFE4F
            | 0xFEFF
            | 0xFF3F
    )
}

/// `new-line-character` from the M lexical grammar.
pub fn is_new_line(c: char) -> bool {
    matches!(c, '\r' | '\n' | '\u{0085}' | '\u{2028}' | '\u{2029}')
}

/// Returns `true` if `text` lexes as a single regular (non-keyword) identifier, i.e. it can be
/// written without `#"..."` quoting in identifier position.
pub fn is_regular_identifier(text: &str) -> bool {
    match tokenize(text) {
        Ok(lexed) => {
            lexed.comments.is_empty()
                && lexed.tokens.len() == 2
                && matches!(&lexed.tokens[0].kind, TokenKind::Identifier(s) if s == text)
                && lexed.tokens[0].span == Span::new(0, text.len())
        }
        Err(_) => false,
    }
}

/// Returns `true` if `text` can be written unquoted as a generalized identifier (record field
/// names and field selectors), e.g. `Order Ship Date`, `Column1.2`, `type`.
pub fn is_generalized_identifier(text: &str) -> bool {
    match tokenize(text) {
        Ok(lexed) if lexed.comments.is_empty() => {
            generalized_identifier_end(text, &lexed.tokens, 0).is_some_and(|(end, span)| {
                end + 1 == lexed.tokens.len() && span == Span::new(0, text.len())
            })
        }
        _ => false,
    }
}

/// Tokens that may form part of a generalized identifier.
fn is_generalized_word(kind: &TokenKind) -> bool {
    match kind {
        TokenKind::Identifier(_) | TokenKind::Number(_) => true,
        TokenKind::Keyword(k) => !k.is_hash(),
        _ => false,
    }
}

/// Assembles a generalized identifier from `tokens[start..]`: a run of identifier, keyword and
/// number tokens separated only by blanks (U+0020) or directly adjacent (`1st`, `Column1.2`),
/// containing at least one identifier or keyword. Returns the index of the first token after the
/// run and the run's span (whose source text is the identifier).
pub(crate) fn generalized_identifier_end(
    src: &str,
    tokens: &[Token],
    start: usize,
) -> Option<(usize, Span)> {
    let first = tokens.get(start)?;
    if !is_generalized_word(&first.kind) {
        return None;
    }
    let mut span = first.span;
    let mut has_word = !matches!(first.kind, TokenKind::Number(_));
    let mut i = start + 1;
    while let Some(tok) = tokens.get(i) {
        if !is_generalized_word(&tok.kind) {
            break;
        }
        let gap = Span::new(span.end, tok.span.start);
        if gap.start > gap.end || !gap.slice(src).bytes().all(|b| b == b' ') {
            break;
        }
        has_word |= !matches!(tok.kind, TokenKind::Number(_));
        span = span.to(tok.span);
        i += 1;
    }
    has_word.then_some((i, span))
}

struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    comments: Vec<Comment>,
}

impl Lexer<'_> {
    fn peek(&self) -> Option<char> {
        self.src.get(self.pos..).and_then(|s| s.chars().next())
    }

    fn peek_nth(&self, n: usize) -> Option<char> {
        self.src.get(self.pos..).and_then(|s| s.chars().nth(n))
    }

    fn rest(&self) -> &str {
        self.src.get(self.pos..).unwrap_or("")
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn err(&self, message: impl Into<String>, start: usize, end: usize) -> ParseError {
        ParseError::new(message, Span::new(start, end.max(start)))
    }

    fn skip_trivia(&mut self) -> Result<(), ParseError> {
        loop {
            let Some(c) = self.peek() else { return Ok(()) };
            if c.is_whitespace() {
                self.bump();
            } else if self.rest().starts_with("//") {
                let start = self.pos;
                while let Some(c) = self.peek() {
                    if is_new_line(c) {
                        break;
                    }
                    self.bump();
                }
                self.push_comment(CommentKind::Line, start);
            } else if self.rest().starts_with("/*") {
                let start = self.pos;
                self.pos += 2;
                match self.rest().find("*/") {
                    Some(i) => self.pos += i + 2,
                    None => {
                        return Err(self
                            .err("unterminated block comment", start, start + 2)
                            .with_expected(vec!["`*/`".to_owned()]));
                    }
                }
                self.push_comment(CommentKind::Block, start);
            } else {
                return Ok(());
            }
        }
    }

    fn push_comment(&mut self, kind: CommentKind, start: usize) {
        let span = Span::new(start, self.pos);
        self.comments.push(Comment {
            kind,
            span,
            text: span.slice(self.src).to_owned(),
        });
    }

    fn next_token(&mut self) -> Result<Token, ParseError> {
        self.skip_trivia()?;
        let start = self.pos;
        let Some(c) = self.peek() else {
            return Ok(Token {
                kind: TokenKind::Eof,
                span: Span::new(start, start),
            });
        };
        let kind = match c {
            '"' => {
                self.bump();
                TokenKind::Text(self.lex_text_body(start)?)
            }
            '#' => self.lex_hash(start)?,
            c if is_identifier_start(c) => self.lex_identifier(start),
            '0'..='9' => self.lex_number(start)?,
            '.' => match self.peek_nth(1) {
                Some('0'..='9') => self.lex_number(start)?,
                Some('.') => {
                    if self.peek_nth(2) == Some('.') {
                        self.pos += 3;
                        TokenKind::Ellipsis
                    } else {
                        self.pos += 2;
                        TokenKind::DotDot
                    }
                }
                _ => return Err(self.err("unexpected character `.`", start, start + 1)),
            },
            _ => self.lex_punct(start, c)?,
        };
        Ok(Token {
            kind,
            span: Span::new(start, self.pos),
        })
    }

    fn lex_punct(&mut self, start: usize, c: char) -> Result<TokenKind, ParseError> {
        let next = self.peek_nth(1);
        let (kind, len) = match (c, next) {
            ('<', Some('=')) => (TokenKind::LessEqual, 2),
            ('<', Some('>')) => (TokenKind::NotEqual, 2),
            ('<', _) => (TokenKind::Less, 1),
            ('>', Some('=')) => (TokenKind::GreaterEqual, 2),
            ('>', _) => (TokenKind::Greater, 1),
            ('=', Some('>')) => (TokenKind::FatArrow, 2),
            ('=', _) => (TokenKind::Equal, 1),
            ('?', Some('?')) => (TokenKind::QuestionQuestion, 2),
            ('?', _) => (TokenKind::Question, 1),
            (',', _) => (TokenKind::Comma, 1),
            (';', _) => (TokenKind::Semicolon, 1),
            ('+', _) => (TokenKind::Plus, 1),
            ('-', _) => (TokenKind::Minus, 1),
            ('*', _) => (TokenKind::Star, 1),
            ('/', _) => (TokenKind::Slash, 1),
            ('&', _) => (TokenKind::Ampersand, 1),
            ('(', _) => (TokenKind::LParen, 1),
            (')', _) => (TokenKind::RParen, 1),
            ('[', _) => (TokenKind::LBracket, 1),
            (']', _) => (TokenKind::RBracket, 1),
            ('{', _) => (TokenKind::LBrace, 1),
            ('}', _) => (TokenKind::RBrace, 1),
            ('@', _) => (TokenKind::At, 1),
            ('!', _) => (TokenKind::Bang, 1),
            _ => {
                let printable = if c.is_control() {
                    format!("U+{:04X}", c as u32)
                } else {
                    format!("`{c}`")
                };
                return Err(self.err(
                    format!("unexpected character {printable}"),
                    start,
                    start + c.len_utf8(),
                ));
            }
        };
        self.pos += len;
        Ok(kind)
    }

    fn lex_hash(&mut self, start: usize) -> Result<TokenKind, ParseError> {
        match self.peek_nth(1) {
            Some('"') => {
                self.pos += 2;
                Ok(TokenKind::QuotedIdentifier(self.lex_text_body(start)?))
            }
            Some('!') if self.peek_nth(2) == Some('"') => {
                self.pos += 3;
                Ok(TokenKind::Verbatim(self.lex_text_body(start)?))
            }
            Some(c) if c.is_ascii_alphabetic() => {
                self.bump();
                while self.peek().is_some_and(|c| c.is_ascii_alphanumeric()) {
                    self.bump();
                }
                let text = Span::new(start, self.pos).slice(self.src);
                match Keyword::lookup(text) {
                    Some(k) => Ok(TokenKind::Keyword(k)),
                    None => Err(self.err(format!("unknown keyword `{text}`"), start, self.pos)),
                }
            }
            _ => Err(self.err("unexpected character `#`", start, start + 1)),
        }
    }

    fn lex_identifier(&mut self, start: usize) -> TokenKind {
        self.bump();
        let mut dotted = false;
        loop {
            match self.peek() {
                Some(c) if is_identifier_part(c) => {
                    self.bump();
                }
                Some('.') if self.peek_nth(1).is_some_and(is_identifier_part) => {
                    dotted = true;
                    self.bump();
                }
                _ => break,
            }
        }
        let text = Span::new(start, self.pos).slice(self.src);
        if !dotted && let Some(k) = Keyword::lookup(text) {
            return TokenKind::Keyword(k);
        }
        TokenKind::Identifier(text.to_owned())
    }

    fn lex_number(&mut self, start: usize) -> Result<TokenKind, ParseError> {
        if self.peek() == Some('0')
            && matches!(self.peek_nth(1), Some('x' | 'X'))
            && self.peek_nth(2).is_some_and(|c| c.is_ascii_hexdigit())
        {
            self.pos += 2;
            let mut value = 0.0_f64;
            while let Some(d) = self.peek().and_then(|c| c.to_digit(16)) {
                value = value * 16.0 + f64::from(d);
                self.bump();
            }
            return Ok(TokenKind::Number(value));
        }
        self.eat_digits();
        if self.peek() == Some('.') && self.peek_nth(1).is_some_and(|c| c.is_ascii_digit()) {
            self.bump();
            self.eat_digits();
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            let has_exponent = match self.peek_nth(1) {
                Some('0'..='9') => true,
                Some('+' | '-') => self.peek_nth(2).is_some_and(|c| c.is_ascii_digit()),
                _ => false,
            };
            if has_exponent {
                self.bump();
                if matches!(self.peek(), Some('+' | '-')) {
                    self.bump();
                }
                self.eat_digits();
            }
        }
        let raw = Span::new(start, self.pos).slice(self.src);
        raw.parse::<f64>()
            .map(TokenKind::Number)
            .map_err(|_| self.err(format!("invalid number literal `{raw}`"), start, self.pos))
    }

    fn eat_digits(&mut self) {
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.bump();
        }
    }

    /// Lexes the characters of a text literal (or quoted identifier / verbatim literal) after the
    /// opening `"`, through the closing `"`, decoding escapes.
    fn lex_text_body(&mut self, start: usize) -> Result<String, ParseError> {
        let mut out = String::new();
        let mut decoder = Utf16Joiner::default();
        loop {
            let Some(c) = self.peek() else {
                return Err(self
                    .err("unterminated text literal", start, self.pos)
                    .with_expected(vec!["`\"`".to_owned()]));
            };
            match c {
                '"' => {
                    self.bump();
                    if self.peek() == Some('"') {
                        self.bump();
                        decoder.push_char(&mut out, '"');
                    } else {
                        decoder.flush(&mut out);
                        return Ok(out);
                    }
                }
                '#' if self.peek_nth(1) == Some('(') => self.lex_escape(&mut out, &mut decoder)?,
                c => {
                    self.bump();
                    decoder.push_char(&mut out, c);
                }
            }
        }
    }

    /// Lexes `#( escape-sequence-list )` at the current position.
    fn lex_escape(
        &mut self,
        out: &mut String,
        decoder: &mut Utf16Joiner,
    ) -> Result<(), ParseError> {
        let esc_start = self.pos;
        self.pos += 2;
        loop {
            let item_start = self.pos;
            while self
                .peek()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '#')
            {
                self.bump();
            }
            let item = Span::new(item_start, self.pos).slice(self.src);
            let is_hex = !item.is_empty() && item.chars().all(|c| c.is_ascii_hexdigit());
            match item {
                _ if item.eq_ignore_ascii_case("cr") => decoder.push_char(out, '\r'),
                _ if item.eq_ignore_ascii_case("lf") => decoder.push_char(out, '\n'),
                _ if item.eq_ignore_ascii_case("tab") => decoder.push_char(out, '\t'),
                "#" => decoder.push_char(out, '#'),
                _ if is_hex && (item.len() == 4 || item.len() == 8) => {
                    let value = u32::from_str_radix(item, 16)
                        .map_err(|_| self.err("invalid character escape", esc_start, self.pos))?;
                    if value > 0x10FFFF {
                        return Err(self.err(
                            format!("character escape `{item}` is not a valid Unicode code point"),
                            item_start,
                            self.pos,
                        ));
                    }
                    decoder.push_unit(out, value);
                }
                _ => {
                    return Err(self
                        .err(
                            "invalid character escape sequence",
                            item_start,
                            // The item, or else the offending character (none
                            // at the end of the input).
                            if item.is_empty() {
                                item_start + self.peek().map_or(0, char::len_utf8)
                            } else {
                                self.pos
                            },
                        )
                        .with_expected(vec![
                            "`cr`".to_owned(),
                            "`lf`".to_owned(),
                            "`tab`".to_owned(),
                            "`#`".to_owned(),
                            "4 or 8 hex digits".to_owned(),
                        ]));
                }
            }
            match self.peek() {
                Some(',') => {
                    self.bump();
                }
                Some(')') => {
                    self.bump();
                    return Ok(());
                }
                _ => {
                    return Err(self
                        .err(
                            "unterminated character escape sequence",
                            esc_start,
                            self.pos,
                        )
                        .with_expected(vec!["`,`".to_owned(), "`)`".to_owned()]));
                }
            }
        }
    }
}

/// Joins UTF-16 surrogate pairs written as separate `#(XXXX)` escapes; lone surrogates decode to
/// U+FFFD.
#[derive(Default)]
struct Utf16Joiner {
    pending_high: Option<u32>,
}

impl Utf16Joiner {
    fn flush(&mut self, out: &mut String) {
        if self.pending_high.take().is_some() {
            out.push(char::REPLACEMENT_CHARACTER);
        }
    }

    fn push_char(&mut self, out: &mut String, c: char) {
        self.flush(out);
        out.push(c);
    }

    fn push_unit(&mut self, out: &mut String, unit: u32) {
        match unit {
            0xD800..=0xDBFF => {
                self.flush(out);
                self.pending_high = Some(unit);
            }
            0xDC00..=0xDFFF => match self.pending_high.take() {
                Some(high) => {
                    let c = 0x10000 + ((high - 0xD800) << 10) + (unit - 0xDC00);
                    out.push(char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER));
                }
                None => out.push(char::REPLACEMENT_CHARACTER),
            },
            _ => {
                self.flush(out);
                out.push(char::from_u32(unit).unwrap_or(char::REPLACEMENT_CHARACTER));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        tokenize(src)
            .unwrap()
            .tokens
            .into_iter()
            .map(|t| t.kind)
            .collect()
    }

    fn ident(s: &str) -> TokenKind {
        TokenKind::Identifier(s.to_owned())
    }

    #[test]
    fn identifiers_keywords_and_dotted_names() {
        assert_eq!(
            kinds("Table.SelectRows QuoteStyle.Csv each _ #table #\"a \"\"b\"\"\""),
            vec![
                ident("Table.SelectRows"),
                ident("QuoteStyle.Csv"),
                TokenKind::Keyword(Keyword::Each),
                ident("_"),
                TokenKind::Keyword(Keyword::HashTable),
                TokenKind::QuotedIdentifier("a \"b\"".to_owned()),
                TokenKind::Eof,
            ]
        );
        assert_eq!(kinds("Column1.2")[0], ident("Column1.2"));
        assert_eq!(
            kinds("a..b"),
            vec![ident("a"), TokenKind::DotDot, ident("b"), TokenKind::Eof]
        );
        assert!(tokenize("#foo").is_err());
    }

    #[test]
    fn numbers() {
        assert_eq!(
            kinds("1 1.5 .5 1e3 2.5E-2 0xFF 0X1a 1..2"),
            vec![
                TokenKind::Number(1.0),
                TokenKind::Number(1.5),
                TokenKind::Number(0.5),
                TokenKind::Number(1000.0),
                TokenKind::Number(0.025),
                TokenKind::Number(255.0),
                TokenKind::Number(26.0),
                TokenKind::Number(1.0),
                TokenKind::DotDot,
                TokenKind::Number(2.0),
                TokenKind::Eof,
            ]
        );
        // `1e` is the number 1 followed by identifier `e`.
        assert_eq!(kinds("1e")[1], ident("e"));
    }

    #[test]
    fn text_escapes() {
        let text = |s: &str| match &kinds(s)[0] {
            TokenKind::Text(t) => t.clone(),
            other => panic!("not text: {other:?}"),
        };
        assert_eq!(text(r#""a""b""#), "a\"b");
        assert_eq!(text(r#""\""#), "\\");
        assert_eq!(
            text("\"#(cr,lf)#(tab)#(#)(#(0041)#(0001F600)\""),
            "\r\n\t#(A😀"
        );
        assert_eq!(text("\"#(D83D)#(DE00)\""), "😀");
        assert_eq!(text("\"#(D83D)x\""), "\u{FFFD}x");
        assert_eq!(text("\"a#b\""), "a#b");
        assert_eq!(text("\"line1\nline2\""), "line1\nline2");
        let err = tokenize("\"#(bogus)\"").unwrap_err();
        assert!(err.message.contains("escape"), "{err}");
        // Found by `cargo fuzz` (m_parse): the span of an empty escape item
        // ran one byte past the end of the input / into a multi-byte char.
        for (src, span) in [
            ("Text.Upper(\"a#(cr,", Span::new(18, 18)),
            ("\"#(é)\"", Span::new(3, 5)),
            ("\"#(cé", Span::new(3, 4)),
        ] {
            let err = tokenize(src).unwrap_err();
            assert_eq!(err.span, span, "{src:?}");
            assert!(src.get(err.span.start..err.span.end).is_some(), "{src:?}");
        }
        let err = tokenize("x = \"abc").unwrap_err();
        assert_eq!(err.span.start, 4);
        assert!(err.message.contains("unterminated"));
    }

    #[test]
    fn operators_and_comments() {
        let lexed = tokenize("a <> b // c\n/* d */ ?? => ... .. @x !").unwrap();
        let k: Vec<_> = lexed.tokens.iter().map(|t| t.kind.clone()).collect();
        assert_eq!(
            k,
            vec![
                ident("a"),
                TokenKind::NotEqual,
                ident("b"),
                TokenKind::QuestionQuestion,
                TokenKind::FatArrow,
                TokenKind::Ellipsis,
                TokenKind::DotDot,
                TokenKind::At,
                ident("x"),
                TokenKind::Bang,
                TokenKind::Eof,
            ]
        );
        assert_eq!(lexed.comments.len(), 2);
        assert_eq!(lexed.comments[0].text, "// c");
        assert_eq!(lexed.comments[1].text, "/* d */");
        assert_eq!(lexed.comments[1].kind, CommentKind::Block);
        assert!(tokenize("/* open").is_err());
    }

    #[test]
    fn identifier_classification() {
        assert!(is_regular_identifier("Table.SelectRows"));
        assert!(is_regular_identifier("_"));
        assert!(!is_regular_identifier("each"));
        assert!(!is_regular_identifier("a b"));
        assert!(!is_regular_identifier("a."));
        assert!(!is_regular_identifier("1a"));
        assert!(is_generalized_identifier("Store Name"));
        assert!(is_generalized_identifier("type"));
        assert!(is_generalized_identifier("Column1.2"));
        assert!(is_generalized_identifier("1st Place"));
        assert!(!is_generalized_identifier("Discount (%)"));
        assert!(!is_generalized_identifier("a..b"));
        assert!(!is_generalized_identifier(" a"));
        assert!(!is_generalized_identifier("2020"));
        assert!(!is_generalized_identifier(""));
    }
}
