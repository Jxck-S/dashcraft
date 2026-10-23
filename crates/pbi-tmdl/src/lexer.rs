//! Line-level lexer.
//!
//! TMDL is line oriented: every structural line is `indent [ref] keyword
//! [name] [(: | =) value]`. The lexer splits text into physical [`Line`]s
//! (keeping each line terminator so nothing is lost), measures indentation
//! and tokenizes structural lines into a [`Header`] whose [`Span`]s point
//! into the line text.

use crate::error::SyntaxError;

/// Line terminator of a physical line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LineEnding {
    /// Last line of a file without a terminator.
    #[default]
    None,
    Lf,
    CrLf,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::None => "",
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
        }
    }
}

/// One physical line, verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Line {
    /// 1-based line number.
    pub number: u32,
    /// Byte offset of the first character in the file (a BOM counts).
    pub offset: usize,
    /// Text without the terminator (a lone `\r` not followed by `\n` stays
    /// in the text).
    pub text: String,
    pub ending: LineEnding,
}

impl Line {
    /// Whitespace-only (or empty).
    pub fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }

    /// Leading tabs/spaces.
    pub fn indentation(&self) -> &str {
        self.text
            .get(..leading_ws_len(&self.text))
            .unwrap_or_default()
    }

    /// Text after the leading tabs/spaces.
    pub fn content(&self) -> &str {
        self.text
            .get(leading_ws_len(&self.text)..)
            .unwrap_or_default()
    }

    /// `///` description line.
    pub fn is_description(&self) -> bool {
        self.content().starts_with("///")
    }

    /// `//` comment line (TMDL has no comments outside expressions; they are
    /// kept as trivia). Description lines are comments too.
    pub fn is_comment(&self) -> bool {
        self.content().starts_with("//")
    }

    /// Blank, comment or description line.
    pub fn is_trivia(&self) -> bool {
        self.is_blank() || self.is_comment()
    }

    /// Text of a `///` line after the slashes and one optional space.
    pub fn description_text(&self) -> Option<&str> {
        let rest = self.content().strip_prefix("///")?;
        Some(rest.strip_prefix(' ').unwrap_or(rest))
    }

    /// 1-based character column of byte index `byte` in [`Line::text`].
    pub fn column_of(&self, byte: usize) -> u32 {
        let prefix = self.text.get(..byte.min(self.text.len())).unwrap_or("");
        u32::try_from(prefix.chars().count())
            .unwrap_or(u32::MAX)
            .saturating_add(1)
    }

    pub(crate) fn write_to(&self, out: &mut String) {
        out.push_str(&self.text);
        out.push_str(self.ending.as_str());
    }
}

/// Number of leading tab/space bytes.
pub(crate) fn leading_ws_len(s: &str) -> usize {
    s.bytes().take_while(|b| *b == b' ' || *b == b'\t').count()
}

/// Splits `text` into lines. `base` is the byte offset of `text` in the file.
pub fn split_lines(text: &str, base: usize) -> Vec<Line> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut number = 0u32;
    while start < text.len() {
        number = number.saturating_add(1);
        let rest = text.get(start..).unwrap_or_default();
        match rest.find('\n') {
            Some(rel) => {
                let nl = start + rel;
                let (end, ending) = if nl > start && bytes.get(nl - 1) == Some(&b'\r') {
                    (nl - 1, LineEnding::CrLf)
                } else {
                    (nl, LineEnding::Lf)
                };
                out.push(Line {
                    number,
                    offset: base + start,
                    text: text.get(start..end).unwrap_or_default().to_owned(),
                    ending,
                });
                start = nl + 1;
            }
            None => {
                out.push(Line {
                    number,
                    offset: base + start,
                    text: rest.to_owned(),
                    ending: LineEnding::None,
                });
                break;
            }
        }
    }
    out
}

/// Indentation unit of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum IndentStyle {
    /// One tab per level (what Power BI Desktop writes).
    #[default]
    Tabs,
    /// `n` spaces per level.
    Spaces(u8),
}

impl IndentStyle {
    /// Spaces that make up one level (tab files accept 4 spaces per level).
    pub fn unit_spaces(self) -> usize {
        match self {
            IndentStyle::Tabs => 4,
            IndentStyle::Spaces(n) => usize::from(n.max(1)),
        }
    }
}

/// Measured indentation of a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Indent {
    /// Whole levels (a tab, or one unit of spaces, per level).
    pub level: usize,
    /// Spaces left over after the last whole level.
    pub partial: usize,
    /// Both tabs and spaces were used.
    pub mixed: bool,
}

/// Measures the indentation of `text`. A tab always counts as one level and
/// absorbs pending spaces before it; `unit` spaces make one level.
pub fn measure_indent(text: &str, style: IndentStyle) -> Indent {
    let unit = style.unit_spaces();
    let (mut level, mut pending) = (0usize, 0usize);
    let (mut tabs, mut spaces) = (false, false);
    for b in text.bytes() {
        match b {
            b'\t' => {
                level += 1;
                pending = 0;
                tabs = true;
            }
            b' ' => {
                spaces = true;
                pending += 1;
                if pending == unit {
                    level += 1;
                    pending = 0;
                }
            }
            _ => break,
        }
    }
    Indent {
        level,
        partial: pending,
        mixed: tabs && spaces,
    }
}

/// Detects the indentation unit from the first indented line. If that line
/// starts an expression block (the previous line ends with `=` or a ```` ``` ````
/// fence), its indentation is two levels.
pub fn detect_indent_style(lines: &[Line]) -> IndentStyle {
    let mut opens_block = false;
    for line in lines {
        if line.is_blank() {
            continue;
        }
        let ws = line.indentation();
        if ws.is_empty() {
            let t = line.text.trim_end();
            opens_block = t.ends_with('=') || t.ends_with("```");
            continue;
        }
        if ws.contains('\t') {
            return IndentStyle::Tabs;
        }
        let n = ws.len();
        let n = if opens_block && n % 2 == 0 { n / 2 } else { n };
        return IndentStyle::Spaces(u8::try_from(n.clamp(1, 16)).unwrap_or(4));
    }
    IndentStyle::Tabs
}

/// Byte range inside a line's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn get(self, text: &str) -> &str {
        text.get(self.start..self.end).unwrap_or_default()
    }
}

/// Value delimiter of a structural line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operator {
    /// `name: value` (scalar property).
    Colon,
    /// `keyword [name] = value` (default property / expression).
    Equals,
}

/// Tokens of a structural line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Header {
    /// Indentation level.
    pub level: usize,
    /// Starts with the `ref` keyword (`ref table X`).
    pub is_ref: bool,
    /// Object type or property name (`table`, `dataType`, …).
    pub keyword: Span,
    /// Raw object name, including quotes (`'Web Sessions'`).
    pub name: Option<Span>,
    pub operator: Option<Operator>,
    /// Value after the operator, trimmed (may be empty).
    pub value: Option<Span>,
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// `true` if `s` is a valid TMDL keyword (object type / property name).
pub(crate) fn is_keyword(s: &str) -> bool {
    let b = s.as_bytes();
    b.first().copied().is_some_and(is_ident_start) && b.iter().copied().all(is_ident_char)
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while matches!(b.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    i
}

fn trimmed_rest(text: &str, from: usize) -> Span {
    let rest = text.get(from..).unwrap_or_default();
    let t = rest.trim();
    if t.is_empty() {
        return Span {
            start: text.len(),
            end: text.len(),
        };
    }
    let start = from + (rest.len() - rest.trim_start().len());
    Span {
        start,
        end: start + t.len(),
    }
}

/// Scans a (possibly dotted) name starting at `i`; returns the end index.
fn scan_name(b: &[u8], mut i: usize) -> Result<usize, (usize, SyntaxError)> {
    loop {
        if b.get(i) == Some(&b'\'') {
            let open = i;
            i += 1;
            loop {
                match b.get(i) {
                    None => return Err((open, SyntaxError::UnterminatedName)),
                    Some(b'\'') if b.get(i + 1) == Some(&b'\'') => i += 2,
                    Some(b'\'') => {
                        i += 1;
                        break;
                    }
                    Some(_) => i += 1,
                }
            }
        } else {
            while let Some(&c) = b.get(i) {
                if matches!(c, b' ' | b'\t' | b'=' | b':' | b'\'' | b'.') {
                    break;
                }
                i += 1;
            }
        }
        if b.get(i) == Some(&b'.') {
            i += 1;
            continue;
        }
        return Ok(i);
    }
}

/// Tokenizes a structural line whose indentation is `indent_len` bytes.
/// Errors carry the byte index of the problem.
pub fn lex_header(
    text: &str,
    indent_len: usize,
    level: usize,
) -> Result<Header, (usize, SyntaxError)> {
    let b = text.as_bytes();
    let mut i = indent_len;
    let mut header = Header {
        level,
        ..Header::default()
    };
    if b.get(i..i + 3) == Some(b"ref".as_slice()) && matches!(b.get(i + 3), Some(b' ' | b'\t')) {
        let j = skip_ws(b, i + 3);
        if b.get(j).copied().is_some_and(is_ident_start) {
            header.is_ref = true;
            i = j;
        }
    }
    if !b.get(i).copied().is_some_and(is_ident_start) {
        return Err((i, SyntaxError::ExpectedKeyword));
    }
    let kw_start = i;
    while b.get(i).copied().is_some_and(is_ident_char) {
        i += 1;
    }
    header.keyword = Span {
        start: kw_start,
        end: i,
    };
    i = skip_ws(b, i);
    match b.get(i) {
        None => return Ok(header),
        Some(b':') => {
            header.operator = Some(Operator::Colon);
            header.value = Some(trimmed_rest(text, i + 1));
            return Ok(header);
        }
        Some(b'=') => {
            header.operator = Some(Operator::Equals);
            header.value = Some(trimmed_rest(text, i + 1));
            return Ok(header);
        }
        Some(_) => {}
    }
    let name_start = i;
    let name_end = scan_name(b, i)?;
    header.name = Some(Span {
        start: name_start,
        end: name_end,
    });
    i = skip_ws(b, name_end);
    match b.get(i) {
        None => Ok(header),
        Some(b'=') => {
            header.operator = Some(Operator::Equals);
            header.value = Some(trimmed_rest(text, i + 1));
            Ok(header)
        }
        Some(_) => Err((i, SyntaxError::UnexpectedText)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(text: &str) -> Result<(Header, Vec<String>), SyntaxError> {
        let indent = leading_ws_len(text);
        let h = lex_header(text, indent, 0).map_err(|(_, e)| e)?;
        let mut parts = vec![h.keyword.get(text).to_owned()];
        parts.push(h.name.map(|s| s.get(text).to_owned()).unwrap_or_default());
        parts.push(h.value.map(|s| s.get(text).to_owned()).unwrap_or_default());
        Ok((h, parts))
    }

    #[test]
    fn lines_keep_terminators() {
        let lines = split_lines("a\r\nb\n\nc", 3);
        let texts: Vec<_> = lines
            .iter()
            .map(|l| (l.number, l.offset, l.text.as_str(), l.ending))
            .collect();
        assert_eq!(
            texts,
            [
                (1, 3, "a", LineEnding::CrLf),
                (2, 6, "b", LineEnding::Lf),
                (3, 8, "", LineEnding::Lf),
                (4, 9, "c", LineEnding::None),
            ]
        );
        assert!(split_lines("", 0).is_empty());
        assert_eq!(split_lines("x\n", 0).len(), 1);
    }

    #[test]
    fn headers_tokenize() {
        let (h, p) = lex("\tmeasure 'Margin %' = DIVIDE([a], [b], 0)").unwrap();
        assert_eq!(p, ["measure", "'Margin %'", "DIVIDE([a], [b], 0)"]);
        assert_eq!(h.operator, Some(Operator::Equals));

        let (h, p) = lex("\t\tformatString: yyyy-mm-dd hh:nn ").unwrap();
        assert_eq!(p, ["formatString", "", "yyyy-mm-dd hh:nn"]);
        assert_eq!(h.operator, Some(Operator::Colon));

        let (h, p) = lex("ref table 'Web Sessions'").unwrap();
        assert!(h.is_ref);
        assert_eq!(p, ["table", "'Web Sessions'", ""]);

        let (h, p) = lex("\t\tisHidden").unwrap();
        assert_eq!((h.operator, p[0].as_str()), (None, "isHidden"));

        let (_, p) = lex("\t\tsource =").unwrap();
        assert_eq!(p, ["source", "", ""]);

        let (_, p) = lex("relationship c79d4066-724d-5669-b57b-f6bf11da6f40").unwrap();
        assert_eq!(p[1], "c79d4066-724d-5669-b57b-f6bf11da6f40");

        let (_, p) = lex("\tcolumn 'It''s'.'a b' = 1").unwrap();
        assert_eq!(p[1], "'It''s'.'a b'");
        let (_, p) = lex("\tcolumn: Category").unwrap();
        assert_eq!(p, ["column", "", "Category"]);
    }

    #[test]
    fn header_errors() {
        assert_eq!(
            lex("table 'Web Sessions").unwrap_err(),
            SyntaxError::UnterminatedName
        );
        assert_eq!(
            lex("table Web Sessions").unwrap_err(),
            SyntaxError::UnexpectedText
        );
        assert_eq!(lex(")").unwrap_err(), SyntaxError::ExpectedKeyword);
        assert_eq!(lex("```").unwrap_err(), SyntaxError::ExpectedKeyword);
    }

    #[test]
    fn indentation_levels() {
        assert_eq!(measure_indent("\t\tx", IndentStyle::Tabs).level, 2);
        let i = measure_indent("\t\t\t\t    Source", IndentStyle::Tabs);
        assert_eq!((i.level, i.partial, i.mixed), (5, 0, true));
        let i = measure_indent("      x", IndentStyle::Spaces(4));
        assert_eq!((i.level, i.partial), (1, 2));
        let lines = split_lines("table A\n  column B\n", 0);
        assert_eq!(detect_indent_style(&lines), IndentStyle::Spaces(2));
        let lines = split_lines("expression A =\n        let\n    lineageTag: x\n", 0);
        assert_eq!(detect_indent_style(&lines), IndentStyle::Spaces(4));
        let lines = split_lines("table A\n\tcolumn B\n", 0);
        assert_eq!(detect_indent_style(&lines), IndentStyle::Tabs);
    }
}
