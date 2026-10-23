//! Lossless concrete syntax tree.
//!
//! Every byte of the input belongs to exactly one [`Line`] of the tree, so
//! `file.to_string()` reproduces the input byte for byte (BOM, CRLF/LF,
//! trailing whitespace, blank lines, comments). The tree is built from
//! indentation:
//!
//! - a [`Node`] is one structural line (object declaration or property)
//!   with its leading trivia (blank lines, `//` comments, `///`
//!   descriptions), an optional multi-line expression [`ExprBlock`], and the
//!   nodes indented one level deeper as children;
//! - trivia after the last node of the file is [`TmdlFile::trailing`].
//!
//! Blank lines after an expression block are not part of the block; they
//! are leading trivia of the next node, so a node's text is self-contained
//! (useful for future minimal-diff edits).

use std::fmt;

use pbi_model::{Diagnostic, Origin};

use crate::error::{SyntaxError, TmdlError};
use crate::expr::{decode_fenced, decode_indented, is_fence_line, unquote_name, unquote_text};
use crate::lexer::{
    Header, IndentStyle, Line, Operator, detect_indent_style, leading_ws_len, lex_header,
    measure_indent, split_lines,
};

/// Maximum indentation depth (Power BI models nest at most ~6 levels).
pub const MAX_DEPTH: usize = 32;

/// Diagnostic codes emitted by this crate.
pub mod codes {
    /// An object or property the typed model does not cover; kept in `extra`.
    pub const UNMODELED: &str = "TMDL001";
    /// A property value that could not be read (e.g. `isHidden: maybe`); kept
    /// in `extra`.
    pub const INVALID_VALUE: &str = "TMDL002";
    /// A `ref` to an object that is not declared.
    pub const MISSING_REF: &str = "TMDL003";
    /// The same property is set twice; the last value wins.
    pub const DUPLICATE_PROPERTY: &str = "TMDL004";
    /// `//` comment lines outside expressions (kept as trivia).
    pub const COMMENT: &str = "TMDL005";
    /// A `///` description that is not directly above an object.
    pub const ORPHAN_DESCRIPTION: &str = "TMDL006";
    /// Tabs and spaces mixed in indentation.
    pub const MIXED_INDENTATION: &str = "TMDL007";
}

/// A multi-line expression after `=`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExprBlock {
    /// ```` ``` ```` delimited; [`ExprBlock::lines`] then ends with the
    /// closing delimiter line.
    pub fenced: bool,
    /// The lines, verbatim.
    pub lines: Vec<Line>,
}

/// One structural line and everything that belongs to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// Blank, comment and description lines before [`Node::line`].
    pub leading: Vec<Line>,
    /// The declaration / property line.
    pub line: Line,
    /// Tokens of [`Node::line`].
    pub header: Header,
    /// Multi-line expression, if any.
    pub block: Option<ExprBlock>,
    /// Nodes indented one level deeper.
    pub children: Vec<Node>,
}

impl Node {
    /// Object type or property name as written (`table`, `dataType`, …).
    pub fn keyword(&self) -> &str {
        self.header.keyword.get(&self.line.text)
    }

    /// `ref <type> <name>` line.
    pub fn is_ref(&self) -> bool {
        self.header.is_ref
    }

    /// Name token as written, including quotes.
    pub fn name_raw(&self) -> Option<&str> {
        self.header.name.map(|s| s.get(&self.line.text))
    }

    /// Unquoted name.
    pub fn name(&self) -> Option<String> {
        self.name_raw().map(unquote_name)
    }

    pub fn operator(&self) -> Option<Operator> {
        self.header.operator
    }

    /// Text after `:` or `=` on the header line, trimmed.
    pub fn value_raw(&self) -> Option<&str> {
        self.header.value.map(|s| s.get(&self.line.text))
    }

    /// Decoded value of a `name: value` property (quotes removed).
    pub fn text_value(&self) -> Option<String> {
        match self.operator() {
            Some(Operator::Colon) => self.value_raw().map(unquote_text),
            _ => None,
        }
    }

    /// Decoded value after `=` (single-line, indented block or fenced; see
    /// [`crate::expr`] for the de-indentation rule).
    pub fn expression(&self) -> Option<String> {
        if self.operator() != Some(Operator::Equals) {
            return None;
        }
        let inline = self.value_raw().unwrap_or_default();
        Some(match &self.block {
            Some(block) if block.fenced => {
                let rest = inline.get(3..).unwrap_or_default();
                let (content, closing) = match block.lines.split_last() {
                    Some((closing, content)) => (content, closing.indentation()),
                    None => (&[][..], ""),
                };
                let texts: Vec<&str> = content.iter().map(|l| l.text.as_str()).collect();
                let body = decode_fenced(&texts, closing);
                if rest.trim().is_empty() {
                    body
                } else if texts.is_empty() {
                    rest.to_owned()
                } else {
                    format!("{rest}\n{body}")
                }
            }
            Some(block) => {
                let texts: Vec<&str> = block.lines.iter().map(|l| l.text.as_str()).collect();
                let body = decode_indented(&texts);
                if inline.is_empty() {
                    body
                } else {
                    format!("{inline}\n{body}")
                }
            }
            None => match inline.strip_prefix("```") {
                Some(rest) if rest.len() >= 3 && rest.ends_with("```") => {
                    rest.get(..rest.len() - 3).unwrap_or_default().to_owned()
                }
                _ => inline.to_owned(),
            },
        })
    }

    /// Description from the `///` lines directly above the node: the text
    /// after `///` and one optional space, trailing whitespace trimmed,
    /// lines joined with `\n`.
    pub fn description(&self) -> Option<String> {
        let n = self
            .leading
            .iter()
            .rev()
            .take_while(|l| l.is_description())
            .count();
        if n == 0 {
            return None;
        }
        let start = self.leading.len() - n;
        let lines: Vec<&str> = self
            .leading
            .iter()
            .skip(start)
            .filter_map(Line::description_text)
            .map(str::trim_end)
            .collect();
        Some(lines.join("\n"))
    }

    /// Indentation level.
    pub fn level(&self) -> usize {
        self.header.level
    }

    /// 1-based line number of the header line.
    pub fn line_number(&self) -> u32 {
        self.line.number
    }

    /// 1-based column of the keyword.
    pub fn column(&self) -> u32 {
        self.line.column_of(self.header.keyword.start)
    }

    /// Last line number covered by the node (expression and children
    /// included, leading trivia excluded).
    pub fn last_line_number(&self) -> u32 {
        let mut last = self.line.number;
        if let Some(l) = self.block.as_ref().and_then(|b| b.lines.last()) {
            last = last.max(l.number);
        }
        if let Some(c) = self.children.last() {
            last = last.max(c.last_line_number());
        }
        last
    }

    /// First child with the given keyword (case-insensitive).
    pub fn child(&self, keyword: &str) -> Option<&Node> {
        self.children
            .iter()
            .find(|c| c.keyword().eq_ignore_ascii_case(keyword))
    }

    fn write_to(&self, out: &mut String) {
        for l in &self.leading {
            l.write_to(out);
        }
        self.line.write_to(out);
        if let Some(block) = &self.block {
            for l in &block.lines {
                l.write_to(out);
            }
        }
        for c in &self.children {
            c.write_to(out);
        }
    }
}

impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = String::new();
        self.write_to(&mut s);
        f.write_str(&s)
    }
}

/// A parsed `.tmdl` file. `to_string()` returns the exact input.
#[derive(Debug, Clone, PartialEq)]
pub struct TmdlFile {
    /// Path as given to [`parse_file`] (used in errors, diagnostics and
    /// origins).
    pub path: String,
    /// The text started with a UTF-8 BOM.
    pub bom: bool,
    /// Detected indentation unit.
    pub indent: IndentStyle,
    /// Top-level nodes.
    pub nodes: Vec<Node>,
    /// Trivia after the last node.
    pub trailing: Vec<Line>,
    /// Lexical findings (comments, orphan descriptions, mixed indentation).
    pub diagnostics: Vec<Diagnostic>,
}

impl TmdlFile {
    /// Top-level nodes with the given keyword (case-insensitive).
    pub fn find<'a>(&'a self, keyword: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.nodes
            .iter()
            .filter(move |n| n.keyword().eq_ignore_ascii_case(keyword))
    }
}

impl fmt::Display for TmdlFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = String::new();
        if self.bom {
            s.push('\u{feff}');
        }
        for n in &self.nodes {
            n.write_to(&mut s);
        }
        for l in &self.trailing {
            l.write_to(&mut s);
        }
        f.write_str(&s)
    }
}

/// Parses one `.tmdl` file into a lossless CST.
pub fn parse_file(path: &str, text: &str) -> Result<TmdlFile, TmdlError> {
    let (bom, body, base) = match text.strip_prefix('\u{feff}') {
        Some(rest) => (true, rest, '\u{feff}'.len_utf8()),
        None => (false, text, 0),
    };
    let lines = split_lines(body, base);
    let indent = detect_indent_style(&lines);
    let mut parser = Parser {
        path,
        lines,
        pos: 0,
        indent,
        notes: Notes::default(),
    };
    let nodes = parser.parse_children(0)?;
    let end = parser.lines.len();
    let trailing = parser.take_range(parser.pos, end);
    parser.note_orphans(&trailing, true);
    let diagnostics = parser.notes.into_diagnostics(path);
    Ok(TmdlFile {
        path: path.to_owned(),
        bom,
        indent,
        nodes,
        trailing,
        diagnostics,
    })
}

#[derive(Default)]
struct Notes {
    comments: Vec<u32>,
    orphans: Vec<u32>,
    mixed: Vec<u32>,
}

impl Notes {
    fn into_diagnostics(self, path: &str) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let mut push = |lines: &[u32], make: &dyn Fn(usize) -> Diagnostic| {
            if let Some(&first) = lines.first() {
                out.push(make(lines.len()).with_origin(Origin::file(path).with_line(first)));
            }
        };
        push(&self.comments, &|n| {
            Diagnostic::info(
                codes::COMMENT,
                format!(
                    "{n} `//` comment line(s) outside expressions (kept as trivia, not part of the model)"
                ),
            )
        });
        push(&self.orphans, &|n| {
            Diagnostic::warning(
                codes::ORPHAN_DESCRIPTION,
                format!("{n} `///` description line(s) not directly above an object are ignored"),
            )
        });
        push(&self.mixed, &|n| {
            Diagnostic::warning(
                codes::MIXED_INDENTATION,
                format!("{n} line(s) mix tabs and spaces in their indentation"),
            )
        });
        out
    }
}

struct Parser<'a> {
    path: &'a str,
    lines: Vec<Line>,
    pos: usize,
    indent: IndentStyle,
    notes: Notes,
}

impl Parser<'_> {
    fn error(&self, line: &Line, byte: usize, kind: SyntaxError) -> TmdlError {
        TmdlError::Syntax {
            path: self.path.to_owned(),
            line: line.number,
            column: line.column_of(byte),
            kind,
        }
    }

    fn take_range(&mut self, start: usize, end: usize) -> Vec<Line> {
        let end = end.min(self.lines.len());
        (start..end)
            .filter_map(|i| self.lines.get_mut(i).map(std::mem::take))
            .collect()
    }

    fn skip_trivia(&self, mut i: usize) -> usize {
        while self.lines.get(i).is_some_and(Line::is_trivia) {
            i += 1;
        }
        i
    }

    /// Records comments and descriptions that are not directly above a node.
    fn note_orphans(&mut self, trivia: &[Line], all_orphans: bool) {
        let attached = if all_orphans {
            0
        } else {
            trivia
                .iter()
                .rev()
                .take_while(|l| l.is_description())
                .count()
        };
        let detached = trivia.len() - attached;
        for l in trivia.iter().take(detached) {
            if l.is_description() {
                self.notes.orphans.push(l.number);
            } else if l.is_comment() {
                self.notes.comments.push(l.number);
            }
        }
    }

    fn parse_children(&mut self, level: usize) -> Result<Vec<Node>, TmdlError> {
        let mut nodes = Vec::new();
        loop {
            let next = self.skip_trivia(self.pos);
            let Some(line) = self.lines.get(next) else {
                break;
            };
            let ind = measure_indent(&line.text, self.indent);
            if ind.level < level {
                break;
            }
            let ws = leading_ws_len(&line.text);
            if ind.level > level {
                return Err(self.error(
                    line,
                    ws,
                    SyntaxError::UnexpectedIndentation {
                        expected: level,
                        found: ind.level,
                    },
                ));
            }
            if ind.partial != 0 {
                return Err(self.error(
                    line,
                    ws,
                    SyntaxError::PartialIndentation {
                        spaces: ind.partial,
                        unit: self.indent.unit_spaces(),
                    },
                ));
            }
            if ind.mixed {
                self.notes.mixed.push(line.number);
            }
            let leading = self.take_range(self.pos, next);
            self.note_orphans(&leading, false);
            self.pos = next;
            nodes.push(self.parse_node(leading, level)?);
        }
        Ok(nodes)
    }

    fn parse_node(&mut self, leading: Vec<Line>, level: usize) -> Result<Node, TmdlError> {
        let line = self
            .lines
            .get_mut(self.pos)
            .map(std::mem::take)
            .unwrap_or_default();
        if level > MAX_DEPTH {
            return Err(self.error(&line, 0, SyntaxError::TooDeep { limit: MAX_DEPTH }));
        }
        self.pos += 1;
        let ws = leading_ws_len(&line.text);
        let header = lex_header(&line.text, ws, level).map_err(|(b, k)| self.error(&line, b, k))?;
        let mut block = None;
        if header.operator == Some(Operator::Equals) {
            let value = header.value.map(|s| s.get(&line.text)).unwrap_or_default();
            if let Some(rest) = value.strip_prefix("```") {
                let single_line = rest.len() >= 3 && rest.ends_with("```");
                if !single_line {
                    let at = header.value.map(|s| s.start).unwrap_or(ws);
                    block = Some(self.parse_fenced(&line, at)?);
                }
            } else {
                let shallow_ok = value.is_empty() && header.name.is_none();
                block = self.parse_indented(level, shallow_ok);
            }
        }
        let children = self.parse_children(level + 1)?;
        Ok(Node {
            leading,
            line,
            header,
            block,
            children,
        })
    }

    /// Lines of an indented expression block after a node at `level`: blank
    /// lines and lines at least two levels deeper (one level is accepted for
    /// nameless properties such as `source =` when the first line is there).
    fn parse_indented(&mut self, level: usize, shallow_ok: bool) -> Option<ExprBlock> {
        let first = self.skip_blank(self.pos);
        let first_level = measure_indent(&self.lines.get(first)?.text, self.indent).level;
        let threshold = if first_level >= level + 2 {
            level + 2
        } else if shallow_ok && first_level == level + 1 {
            level + 1
        } else {
            return None;
        };
        let mut end = self.pos;
        let mut j = self.pos;
        while let Some(l) = self.lines.get(j) {
            j += 1;
            if l.is_blank() {
                continue;
            }
            if measure_indent(&l.text, self.indent).level >= threshold {
                end = j;
            } else {
                break;
            }
        }
        let lines = self.take_range(self.pos, end);
        self.pos = end;
        Some(ExprBlock {
            fenced: false,
            lines,
        })
    }

    fn skip_blank(&self, mut i: usize) -> usize {
        while self.lines.get(i).is_some_and(Line::is_blank) {
            i += 1;
        }
        i
    }

    fn parse_fenced(&mut self, header: &Line, at: usize) -> Result<ExprBlock, TmdlError> {
        let start = self.pos;
        let mut j = start;
        while let Some(l) = self.lines.get(j) {
            if is_fence_line(&l.text) {
                let lines = self.take_range(start, j + 1);
                self.pos = j + 1;
                return Ok(ExprBlock {
                    fenced: true,
                    lines,
                });
            }
            j += 1;
        }
        Err(self.error(header, at, SyntaxError::UnterminatedFence))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> TmdlFile {
        let f = parse_file("t.tmdl", text).unwrap();
        assert_eq!(f.to_string(), text);
        f
    }

    #[test]
    fn builds_tree_and_keeps_every_byte() {
        let text = "\u{feff}/// Table\r\ntable 'Web Sessions'\r\n\tisHidden\r\n\r\n\t/// M\r\n\tmeasure X = 1  \r\n\t\tformatString: 0\r\n\r\n// tail\r\n";
        let f = parse(text);
        assert!(f.bom);
        let table = &f.nodes[0];
        assert_eq!(table.keyword(), "table");
        assert_eq!(table.name().as_deref(), Some("Web Sessions"));
        assert_eq!(table.description().as_deref(), Some("Table"));
        assert_eq!(table.children.len(), 2);
        let m = &table.children[1];
        assert_eq!(m.description().as_deref(), Some("M"));
        assert_eq!(m.expression().as_deref(), Some("1"));
        assert_eq!(m.children[0].text_value().as_deref(), Some("0"));
        assert_eq!((m.line_number(), m.column()), (6, 2));
        assert_eq!(f.trailing.len(), 2);
        assert_eq!(f.diagnostics[0].code, codes::COMMENT);
    }

    #[test]
    fn multi_line_expressions() {
        let text = "table T\n\tmeasure A =\n\t\t\tVAR x = 1\n\n\t\t\tRETURN\n\t\t\t    x\n\n\t\tformatString: 0\n\tpartition P = m\n\t\tsource =\n\t\t\tlet\n\t\t\tin\n\t\t\t    s\n\tmeasure B = ```\n\t\t\t  keep  \n\t\t\t```\n\tmeasure C = ```x```\n";
        let f = parse(text);
        let t = &f.nodes[0];
        assert_eq!(
            t.children[0].expression().as_deref(),
            Some("VAR x = 1\n\nRETURN\n    x")
        );
        assert_eq!(t.children[0].children.len(), 1);
        let source = t.children[1].child("source").unwrap();
        assert_eq!(source.expression().as_deref(), Some("let\nin\n    s"));
        assert_eq!(t.children[2].expression().as_deref(), Some("  keep  "));
        assert_eq!(t.children[3].expression().as_deref(), Some("x"));
    }

    #[test]
    fn syntax_errors_have_positions() {
        let err = parse_file("a.tmdl", "table A\n\t\tcolumn B\n").unwrap_err();
        assert!(matches!(
            err,
            TmdlError::Syntax {
                line: 2,
                column: 3,
                kind: SyntaxError::UnexpectedIndentation {
                    expected: 1,
                    found: 2
                },
                ..
            }
        ));
        let err = parse_file("a.tmdl", "table A\n\tmeasure B = ```\n\t\t\tx\n").unwrap_err();
        assert_eq!(err.syntax_kind(), Some(&SyntaxError::UnterminatedFence));
        assert_eq!(
            err.to_string(),
            "a.tmdl:2:14: unterminated ``` expression (missing closing ``` line)"
        );
        let err = parse_file("a.tmdl", "table A\n  column B\n   x\n").unwrap_err();
        assert!(matches!(
            err.syntax_kind(),
            Some(SyntaxError::PartialIndentation { .. })
        ));
    }

    #[test]
    fn deep_nesting_is_bounded() {
        let mut text = String::new();
        for i in 0..=MAX_DEPTH + 1 {
            text.push_str(&"\t".repeat(i));
            text.push_str("a b\n");
        }
        let err = parse_file("deep.tmdl", &text).unwrap_err();
        assert!(matches!(
            err.syntax_kind(),
            Some(SyntaxError::TooDeep { .. })
        ));
    }
}
