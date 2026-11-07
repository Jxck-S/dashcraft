//! TMDL formatter (`tmdl.format`): lays a file out as Power BI Desktop and [`crate::write_model`]
//! do, changing only white space.
//!
//! - Every structural line is indented by its depth (a tab per level, or
//!   [`FormatOptions::indent`] spaces) and spaced as the writer spaces it: `ref keyword name`,
//!   `keyword: value`, `keyword name = value`.
//! - An indented expression block moves to two levels below its line (one level for a nameless
//!   `property =` whose block was one level below, as Desktop writes a KPI's expressions),
//!   keeping the relative indentation of its lines; a fenced block moves with its closing
//!   delimiter. Expressions read back unchanged, trailing white space inside them included.
//! - One blank line comes before every named object and before the first `ref` line, none
//!   between `ref` lines of one type, before a `keyword: value` property or a flag. Where
//!   Microsoft's own tools differ, a blank line stays if there was one and is not added
//!   otherwise: between `ref` lines of different types (Desktop writes one, the TOM library
//!   none), before a perspective's `perspectiveColumn`, `perspectiveMeasure` and
//!   `perspectiveHierarchy` (TOM writes none) and before a nameless object (`kpi`,
//!   `formatStringDefinition =`, `dataAccessOptions`), which Desktop writes with or without.
//!   Runs of blank lines become one; there is no blank line at the start of the file and at
//!   most one at its end.
//! - `//` comments and `///` descriptions are indented with the line they precede, without
//!   trailing white space and with a space after `///`.
//! - Every line ends with the text's first line ending (or the one asked for); a BOM stays.
//!
//! Files written by Power BI Desktop, the TOM library and the writer come back unchanged. The
//! result is checked before it is returned: it must parse to the same tree, with the same
//! keywords, names, values, expressions, descriptions and comments. Otherwise [`format`]
//! returns [`FormatError::Unstable`] rather than a text with another meaning.

use std::fmt;

use super::catalog::{KeywordKind, catalog};
use super::diagnostics::{Severity, Source, TmdlDiagnostic, codes, diagnostics};
use crate::cst::{ExprBlock, Node, TmdlFile, parse_file};
use crate::expr::common_ws_prefix;
use crate::lexer::{IndentStyle, Line, LineEnding, Operator, measure_indent};

/// Largest number of spaces per level [`format`] indents with (wider values are capped).
pub const MAX_INDENT_WIDTH: u8 = 16;

/// Layout options of [`format`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FormatOptions {
    /// Tabs (the default, as Desktop and the writer indent) or spaces per level (at most
    /// [`MAX_INDENT_WIDTH`]).
    pub indent: IndentStyle,
    /// The line ending of every line; `None` (or [`LineEnding::None`]) keeps the text's first
    /// one (LF when it has none).
    pub line_ending: Option<LineEnding>,
}

/// Why [`format`] returned no text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatError {
    /// The text does not parse: the syntax error, as [`super::diagnostics`] reports it.
    Syntax(Box<TmdlDiagnostic>),
    /// The layout would have changed what the text means (a formatter bug); the text is better
    /// left as it is.
    Unstable,
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatError::Syntax(d) => {
                write!(f, "syntax error at {}:{}: {}", d.line, d.column, d.message)
            }
            FormatError::Unstable => {
                f.write_str("the formatted text would not be equivalent to the input")
            }
        }
    }
}

impl std::error::Error for FormatError {}

/// Formats one `.tmdl` file. A blank text formats to an empty one.
pub fn format(text: &str, options: &FormatOptions) -> Result<String, FormatError> {
    let file = match parse_file("", text) {
        Ok(file) => file,
        Err(e) => return Err(FormatError::Syntax(Box::new(syntax_diagnostic(text, &e)))),
    };
    let indent = match options.indent {
        IndentStyle::Tabs => "\t".to_owned(),
        IndentStyle::Spaces(n) => " ".repeat(usize::from(n.clamp(1, MAX_INDENT_WIDTH))),
    };
    let nl = match options.line_ending {
        Some(LineEnding::CrLf) => "\r\n",
        Some(LineEnding::Lf) => "\n",
        _ => match text.find('\n') {
            Some(i) if i > 0 && text.as_bytes().get(i - 1) == Some(&b'\r') => "\r\n",
            _ => "\n",
        },
    };
    let mut out = Emitter {
        out: String::with_capacity(text.len() + text.len() / 8),
        nl,
        indent,
        style: file.indent,
    };
    if file.bom {
        out.out.push('\u{feff}');
    }
    out.children(&file.nodes, true);
    out.trailing(&file.trailing, !file.nodes.is_empty());
    let formatted = out.out;
    let again = parse_file("", &formatted).map_err(|_| FormatError::Unstable)?;
    if shape(&file) != shape(&again) {
        return Err(FormatError::Unstable);
    }
    Ok(formatted)
}

fn syntax_diagnostic(text: &str, error: &crate::TmdlError) -> TmdlDiagnostic {
    diagnostics(&[Source::new("", text)])
        .into_iter()
        .find(|d| d.code == codes::SYNTAX)
        .unwrap_or_else(|| TmdlDiagnostic {
            file: 0,
            path: String::new(),
            severity: Severity::Error,
            code: codes::SYNTAX,
            message: error.to_string(),
            start: 0,
            end: 0,
            line: 1,
            column: 1,
            suggestion: None,
        })
}

struct Emitter {
    out: String,
    nl: &'static str,
    /// One level of indentation in the output.
    indent: String,
    /// The input's indentation unit (to measure its levels).
    style: IndentStyle,
}

impl Emitter {
    fn line(&mut self, level: usize, text: &str) {
        if !text.is_empty() {
            for _ in 0..level {
                self.out.push_str(&self.indent);
            }
            self.out.push_str(text);
        }
        self.out.push_str(self.nl);
    }

    fn blank(&mut self) {
        self.out.push_str(self.nl);
    }

    fn children(&mut self, nodes: &[Node], top: bool) {
        let mut prev: Option<&Node> = None;
        for node in nodes {
            let at_start = top && prev.is_none();
            self.node(node, prev, at_start);
            prev = Some(node);
        }
    }

    fn node(&mut self, node: &Node, prev: Option<&Node>, at_start: bool) {
        let level = node.level();
        self.leading(&node.leading, level, blank_before(node, prev), at_start);
        self.line(level, &header(node));
        if let Some(block) = &node.block {
            self.block(level, block);
        }
        self.children(&node.children, false);
    }

    /// The blank, comment and description lines before a node: `blank` says whether a blank
    /// line comes first (`None`: if one did).
    fn leading(&mut self, lines: &[Line], level: usize, blank: Option<bool>, at_start: bool) {
        let first = lines
            .iter()
            .position(|l| !l.is_blank())
            .unwrap_or(lines.len());
        if !at_start && blank.unwrap_or(first > 0) {
            self.blank();
        }
        self.notes(lines.get(first..).unwrap_or_default(), level, false);
    }

    /// Comment and description lines at `level`; a run of blank lines between them (and after
    /// the last one, which detaches a description) becomes one.
    fn notes(&mut self, lines: &[Line], level: usize, mut pending: bool) {
        for l in lines {
            if l.is_blank() {
                pending = true;
                continue;
            }
            if pending {
                self.blank();
                pending = false;
            }
            self.line(level, &note(l));
        }
        if pending {
            self.blank();
        }
    }

    /// The trivia after the last node of the file.
    fn trailing(&mut self, lines: &[Line], after_nodes: bool) {
        let first = lines
            .iter()
            .position(|l| !l.is_blank())
            .unwrap_or(lines.len());
        let rest = lines.get(first..).unwrap_or_default();
        if rest.is_empty() {
            if after_nodes && first > 0 {
                self.blank();
            }
            return;
        }
        self.notes(rest, 0, after_nodes && first > 0);
    }

    fn block(&mut self, level: usize, block: &ExprBlock) {
        if block.fenced {
            let Some((closing, content)) = block.lines.split_last() else {
                return;
            };
            let closing_ws = closing.indentation();
            let depth = if measure_indent(closing_ws, self.style).level == level + 1 {
                level + 1
            } else {
                level + 2
            };
            for l in content {
                let text = l.text.trim_end_matches('\r');
                let n = text
                    .bytes()
                    .zip(closing_ws.bytes())
                    .take_while(|(a, b)| a == b)
                    .count();
                self.line(depth, text.get(n..).unwrap_or(text));
            }
            self.line(depth, "```");
            return;
        }
        let texts: Vec<&str> = block.lines.iter().map(|l| l.text.as_str()).collect();
        let prefix = common_ws_prefix(&texts).len();
        let shallow = block
            .lines
            .iter()
            .find(|l| !l.is_blank())
            .is_some_and(|l| measure_indent(&l.text, self.style).level < level + 2);
        let depth = if shallow { level + 1 } else { level + 2 };
        let last = block.lines.iter().rposition(|l| !l.is_blank());
        for (i, l) in block.lines.iter().enumerate() {
            if l.is_blank() {
                self.blank();
                continue;
            }
            let text = l.text.trim_end_matches('\r');
            let rest = text.get(prefix..).unwrap_or(text);
            let rest = if Some(i) == last {
                rest.trim_end()
            } else {
                rest
            };
            self.line(depth, rest);
        }
    }
}

/// Whether a blank line comes before `node` (`None`: if one did).
fn blank_before(node: &Node, prev: Option<&Node>) -> Option<bool> {
    if node.is_ref() {
        return match prev {
            Some(p) if p.is_ref() && p.keyword().eq_ignore_ascii_case(node.keyword()) => {
                Some(false)
            }
            Some(p) if p.is_ref() => None,
            _ => Some(true),
        };
    }
    if node.name_raw().is_some() {
        let member = [
            "perspectiveColumn",
            "perspectiveMeasure",
            "perspectiveHierarchy",
        ]
        .iter()
        .any(|k| k.eq_ignore_ascii_case(node.keyword()));
        return if member { None } else { Some(true) };
    }
    match node.operator() {
        Some(Operator::Colon) => Some(false),
        None if node.children.is_empty() && is_property(node.keyword()) => Some(false),
        _ => None,
    }
}

/// Whether every context that knows `keyword` knows it as a property.
fn is_property(keyword: &str) -> bool {
    let mut known = false;
    for context in catalog().contexts_of(keyword) {
        known = true;
        if context
            .keyword(keyword)
            .is_none_or(|k| k.kind != KeywordKind::Property)
        {
            return false;
        }
    }
    known
}

/// A structural line spaced as the writer spaces it (without indentation).
fn header(node: &Node) -> String {
    let mut s = String::new();
    if node.is_ref() {
        s.push_str("ref ");
    }
    s.push_str(node.keyword());
    if let Some(name) = node.name_raw() {
        s.push(' ');
        s.push_str(name);
    }
    let value = node.value_raw().unwrap_or_default();
    match node.operator() {
        Some(Operator::Colon) => s.push(':'),
        Some(Operator::Equals) => s.push_str(" ="),
        None => return s,
    }
    if !value.is_empty() {
        s.push(' ');
        s.push_str(value);
    }
    s
}

/// A comment or description line without indentation or trailing white space, `///` followed
/// by a space.
fn note(line: &Line) -> String {
    let content = line.content().trim_end();
    match content.strip_prefix("///") {
        Some(rest) if !rest.is_empty() && !rest.starts_with(' ') => format!("/// {rest}"),
        _ => content.to_owned(),
    }
}

/// What a node means, to compare a file with its formatted text.
#[derive(Debug, PartialEq)]
struct Shape<'a> {
    is_ref: bool,
    keyword: &'a str,
    name: Option<&'a str>,
    operator: Option<Operator>,
    value: Option<&'a str>,
    expression: Option<String>,
    description: Option<String>,
    notes: Vec<String>,
    children: Vec<Shape<'a>>,
}

fn notes_of(lines: &[Line]) -> Vec<String> {
    lines
        .iter()
        .filter(|l| !l.is_blank())
        .map(|l| match l.description_text() {
            Some(d) => format!("///{}", d.trim_end()),
            None => l.content().trim_end().to_owned(),
        })
        .collect()
}

fn node_shape(node: &Node) -> Shape<'_> {
    Shape {
        is_ref: node.is_ref(),
        keyword: node.keyword(),
        name: node.name_raw(),
        operator: node.operator(),
        value: node.value_raw(),
        expression: node.expression(),
        description: node.description(),
        notes: notes_of(&node.leading),
        children: node.children.iter().map(node_shape).collect(),
    }
}

fn shape(file: &TmdlFile) -> (bool, Vec<Shape<'_>>, Vec<String>) {
    (
        file.bom,
        file.nodes.iter().map(node_shape).collect(),
        notes_of(&file.trailing),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(text: &str) -> String {
        format(text, &FormatOptions::default()).unwrap()
    }

    #[test]
    fn lays_out_lines_as_the_writer_does() {
        let messy = "table   'Web Sessions'  \n  lineageTag:abc\n\n\n  isHidden\n  measure  X=SUM(T[a])\n    formatString :  0  \n  column A\n    dataType:  string\n\n    annotation  S =  Automatic\n  ///Hidden key\n  column B\n";
        let want = "table 'Web Sessions'\n\tlineageTag: abc\n\tisHidden\n\n\tmeasure X = SUM(T[a])\n\t\tformatString: 0\n\n\tcolumn A\n\t\tdataType: string\n\n\t\tannotation S = Automatic\n\n\t/// Hidden key\n\tcolumn B\n";
        assert_eq!(tabs(messy), want);
        assert_eq!(tabs(want), want);
    }

    #[test]
    fn moves_expression_blocks_and_keeps_their_text() {
        let text = "table T\n\tmeasure A =\n\t\t\t\tVAR x = 1  \n\n\t\t\t\tRETURN\n\t\t\t\t    x   \n\tpartition P = m\n\t\tsource =\n\t\t\tlet\n\t\t\t    s = 1\n\t\t\tin\n\t\t\t    s\n\tmeasure B = ```\n\t\t\t\t  keep  \n\n\t\t\t\t```\n";
        let want = "table T\n\n\tmeasure A =\n\t\t\tVAR x = 1  \n\n\t\t\tRETURN\n\t\t\t    x\n\n\tpartition P = m\n\t\tsource =\n\t\t\tlet\n\t\t\t    s = 1\n\t\t\tin\n\t\t\t    s\n\n\tmeasure B = ```\n\t\t\t  keep  \n\n\t\t\t```\n";
        let out = tabs(text);
        assert_eq!(out, want);
        let before = parse_file("", text).unwrap();
        let after = parse_file("", &out).unwrap();
        let exprs = |f: &TmdlFile| -> Vec<Option<String>> {
            f.nodes[0]
                .children
                .iter()
                .map(|c| c.expression().or_else(|| c.children[0].expression()))
                .collect()
        };
        assert_eq!(exprs(&before), exprs(&after));
        assert_eq!(
            exprs(&after)[0].as_deref(),
            Some("VAR x = 1  \n\nRETURN\n    x")
        );
    }

    #[test]
    fn keeps_a_kpi_expression_one_level_below_its_property() {
        let text = "table T\n\n\tmeasure M = 1\n\n\t\tkpi\n\t\t\ttargetExpression =\n\t\t\t\tVAR t = 2\n\t\t\t\tRETURN t\n";
        assert_eq!(tabs(text), text);
    }

    #[test]
    fn spaces_ref_runs_and_keeps_nameless_objects_as_written() {
        let text = "model Model\n\tculture: en-US\n\tdataAccessOptions\n\t\tlegacyRedirects\nannotation A = 1\nref table X\n\nref table Y\n\n\nref role R\nref cultureInfo en-US\n";
        let want = "model Model\n\tculture: en-US\n\tdataAccessOptions\n\t\tlegacyRedirects\n\nannotation A = 1\n\nref table X\nref table Y\n\nref role R\nref cultureInfo en-US\n";
        assert_eq!(tabs(text), want);
        let culture = "cultureInfo en-US\n\n\tlinguisticMetadata =\n\t\t\t{\"Version\": \"1.0.0\"}\n\t\tcontentType: json\n";
        assert_eq!(tabs(culture), culture);
    }

    #[test]
    fn options_choose_indentation_and_line_endings() {
        let text = "\u{feff}table T\r\n\tcolumn C\r\n\t\tdataType: int64\r\n\r\n";
        let spaces = FormatOptions {
            indent: IndentStyle::Spaces(2),
            line_ending: Some(LineEnding::Lf),
        };
        let out = format(text, &spaces).unwrap();
        assert_eq!(
            out,
            "\u{feff}table T\n\n  column C\n    dataType: int64\n\n"
        );
        assert_eq!(format(&out, &spaces).unwrap(), out);
        assert_eq!(
            tabs(&out.replace('\n', "\r\n")),
            "\u{feff}table T\r\n\r\n\tcolumn C\r\n\t\tdataType: int64\r\n\r\n"
        );
        assert_eq!(tabs(""), "");
        assert_eq!(tabs("\n\n"), "");
    }

    #[test]
    fn keeps_comments_and_detached_descriptions() {
        let text =
            "// head\n\ntable T\n\t// about C\n\n\t/// orphan\n\n\tcolumn C\n\n\n// tail   \n\n\n";
        let want =
            "// head\n\ntable T\n\n\t// about C\n\n\t/// orphan\n\n\tcolumn C\n\n// tail\n\n";
        assert_eq!(tabs(text), want);
        let f = parse_file("", &tabs(text)).unwrap();
        assert_eq!(f.nodes[0].children[0].description(), None);
    }

    #[test]
    fn reports_syntax_errors() {
        let Err(FormatError::Syntax(d)) =
            format("table T\n\t\tcolumn C\n", &FormatOptions::default())
        else {
            panic!("expected a syntax error");
        };
        assert_eq!((d.code, d.line), (codes::SYNTAX, 2));
    }
}
