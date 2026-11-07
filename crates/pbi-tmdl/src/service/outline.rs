//! A tolerant reading of TMDL text for completion and hover.
//!
//! Unlike [`crate::parse_file`], [`Outline::new`] never fails: every line is a structural line
//! ([`Entry`]), a line of a multi-line expression, or trivia, decided by indentation as the
//! parser decides it. Half-typed text (an unterminated quote, a missing value, a line indented
//! too deep) still has a structure, and on text the parser accepts the outline has the parser's
//! tree.

use super::catalog::Form;
use crate::expr::{is_fence_line, unquote_name};
use crate::lexer::{
    IndentStyle, Line, LineEnding, Operator, Span, detect_indent_style, is_ident_char,
    is_ident_start, leading_ws_len, lex_header, measure_indent, skip_ws, split_lines, trimmed_rest,
};

/// A structural line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Entry {
    /// Index of the line in [`Outline::lines`].
    pub line: usize,
    pub level: usize,
    /// `ref <type> <name>`.
    pub is_ref: bool,
    /// The keyword (empty when the line starts with something else).
    pub keyword: Span,
    pub name: Option<Span>,
    pub operator: Option<Operator>,
    /// Byte index of the operator in the line.
    pub operator_at: usize,
    /// The value after the operator, trimmed.
    pub value: Option<Span>,
    /// The nearest earlier entry indented less.
    pub parent: Option<usize>,
}

/// What a line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Role {
    /// Blank, `//` comment or `///` description.
    Trivia,
    /// A structural line ([`Outline::entries`] index).
    Entry(usize),
    /// A line of the multi-line expression of an entry.
    Expression(usize),
}

/// A multi-line expression being read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Block {
    owner: usize,
    owner_level: usize,
    fenced: bool,
    /// The indentation of the block's lines, known from its first line.
    threshold: Option<usize>,
    /// A nameless `keyword =` with nothing after it: its expression may be one level deeper.
    shallow_ok: bool,
}

impl Block {
    /// Whether a line at `level` belongs to the block (and the block's threshold after it).
    fn takes(self, level: usize) -> Option<usize> {
        let threshold = match self.threshold {
            Some(t) => t,
            None if level >= self.owner_level + 2 => self.owner_level + 2,
            None if self.shallow_ok && level == self.owner_level + 1 => self.owner_level + 1,
            None => return None,
        };
        (level >= threshold).then_some(threshold)
    }
}

/// Where a cursor is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Place {
    /// Nowhere to complete: a comment, the indentation, a line indented too deep, past a name.
    None,
    /// In the expression after an entry's `=`.
    Expression(usize),
    /// The keyword of a line at `level` under `parent`; `line` is the cursor's line (index in
    /// [`Outline::lines`], or past the last), `own` its entry (none on a blank line). `span` is
    /// absolute.
    Keyword {
        parent: Option<usize>,
        level: usize,
        line: usize,
        own: Option<usize>,
        span: Span,
    },
    /// The name after an entry's keyword.
    Name { entry: usize, span: Span },
    /// The value after an entry's `:`.
    Value { entry: usize, span: Span },
    /// The object type of a top-level `ref` line.
    RefKind { span: Span },
}

/// The outline of one file.
#[derive(Debug, Clone)]
pub(super) struct Outline {
    pub lines: Vec<Line>,
    pub entries: Vec<Entry>,
    pub roles: Vec<Role>,
    /// The expression open at the start of each line, and after the last one.
    blocks: Vec<Option<Block>>,
    pub indent: IndentStyle,
}

impl Outline {
    pub(super) fn new(text: &str) -> Self {
        let (body, base) = match text.strip_prefix('\u{feff}') {
            Some(rest) => (rest, '\u{feff}'.len_utf8()),
            None => (text, 0),
        };
        let lines = split_lines(body, base);
        let indent = detect_indent_style(&lines);
        let mut entries: Vec<Entry> = Vec::new();
        let mut roles = Vec::with_capacity(lines.len());
        let mut blocks = Vec::with_capacity(lines.len() + 1);
        let mut block: Option<Block> = None;
        for (i, line) in lines.iter().enumerate() {
            blocks.push(block);
            if let Some(b) = block {
                if b.fenced {
                    if is_fence_line(&line.text) {
                        block = None;
                    }
                    roles.push(Role::Expression(b.owner));
                    continue;
                }
                if line.is_blank() {
                    roles.push(Role::Trivia);
                    continue;
                }
                let level = measure_indent(&line.text, indent).level;
                if let Some(threshold) = b.takes(level) {
                    block = Some(Block {
                        threshold: Some(threshold),
                        ..b
                    });
                    roles.push(Role::Expression(b.owner));
                    continue;
                }
                block = None;
            }
            if line.is_trivia() {
                roles.push(Role::Trivia);
                continue;
            }
            let level = measure_indent(&line.text, indent).level;
            let parent = parent_at(&entries, entries.len(), level);
            let mut entry = header(&line.text, leading_ws_len(&line.text));
            entry.line = i;
            entry.level = level;
            entry.parent = parent;
            let index = entries.len();
            if entry.operator == Some(Operator::Equals) {
                let value = entry.value.map(|s| s.get(&line.text)).unwrap_or_default();
                block = match value.strip_prefix("```") {
                    Some(rest) if rest.len() >= 3 && rest.ends_with("```") => None,
                    Some(_) => Some(Block {
                        owner: index,
                        owner_level: level,
                        fenced: true,
                        threshold: None,
                        shallow_ok: false,
                    }),
                    None => Some(Block {
                        owner: index,
                        owner_level: level,
                        fenced: false,
                        threshold: None,
                        shallow_ok: value.is_empty() && entry.name.is_none(),
                    }),
                };
            }
            entries.push(entry);
            roles.push(Role::Entry(index));
        }
        blocks.push(block);
        Outline {
            lines,
            entries,
            roles,
            blocks,
            indent,
        }
    }

    pub(super) fn entry(&self, index: usize) -> Option<&Entry> {
        self.entries.get(index)
    }

    /// The text of the entry's line.
    pub(super) fn line_text(&self, entry: &Entry) -> &str {
        self.lines.get(entry.line).map_or("", |l| l.text.as_str())
    }

    /// The keyword as written (`ref` is not part of it).
    pub(super) fn keyword(&self, entry: &Entry) -> &str {
        entry.keyword.get(self.line_text(entry))
    }

    /// The unquoted name.
    pub(super) fn name(&self, entry: &Entry) -> Option<String> {
        entry
            .name
            .map(|s| unquote_name(s.get(self.line_text(entry))))
    }

    /// The value after the operator as written.
    pub(super) fn value(&self, entry: &Entry) -> Option<&str> {
        entry.value.map(|s| s.get(self.line_text(entry)))
    }

    /// The form of the line.
    pub(super) fn form(entry: &Entry) -> Form {
        match (entry.is_ref || entry.name.is_some(), entry.operator) {
            (true, Some(Operator::Equals)) => Form::NamedEquals,
            (true, _) => Form::Named,
            (false, Some(Operator::Colon)) => Form::Colon,
            (false, Some(Operator::Equals)) => Form::Equals,
            (false, None) => Form::Bare,
        }
    }

    /// The entries whose parent is `parent` (top-level entries for `None`), at `level`.
    pub(super) fn children(
        &self,
        parent: Option<usize>,
        level: usize,
    ) -> impl Iterator<Item = (usize, &Entry)> + '_ {
        self.entries
            .iter()
            .enumerate()
            .filter(move |(_, e)| e.parent == parent && e.level == level)
    }

    /// `parent` and its ancestors, the outermost first.
    pub(super) fn path(&self, parent: Option<usize>) -> Vec<usize> {
        let mut path = Vec::new();
        let mut at = parent;
        while let Some(i) = at {
            if path.len() > self.entries.len() {
                break;
            }
            path.push(i);
            at = self.entries.get(i).and_then(|e| e.parent);
        }
        path.reverse();
        path
    }

    /// The text of the `///` lines directly above an entry.
    pub(super) fn description(&self, entry: &Entry) -> Option<String> {
        let mut lines: Vec<&str> = self
            .lines
            .get(..entry.line)
            .unwrap_or_default()
            .iter()
            .rev()
            .take_while(|l| l.is_description())
            .filter_map(Line::description_text)
            .map(str::trim_end)
            .collect();
        if lines.is_empty() {
            return None;
        }
        lines.reverse();
        Some(lines.join("\n"))
    }

    /// Where the cursor at byte `offset` (on a character boundary) is.
    pub(super) fn place(&self, offset: usize) -> Place {
        let k = self
            .lines
            .partition_point(|l| l.offset <= offset)
            .saturating_sub(1);
        let last = k + 1 >= self.lines.len();
        let (line_start, text, index) = match self.lines.get(k) {
            // A cursor after the last line's terminator is on a new, empty line.
            Some(l)
                if last
                    && l.ending != LineEnding::None
                    && offset >= l.offset + l.text.len() + l.ending.as_str().len() =>
            {
                (offset, "", self.lines.len())
            }
            Some(l) => (l.offset, l.text.as_str(), k),
            None => (offset, "", 0),
        };
        let col = offset.saturating_sub(line_start).min(text.len());
        let before = text.get(..col).unwrap_or_default();
        let at_cursor = Span {
            start: offset,
            end: offset,
        };
        if text.trim().is_empty() {
            let level = measure_indent(before, self.indent).level;
            if let Some(b) = self.blocks.get(index).copied().flatten()
                && (b.fenced || b.takes(level).is_some())
            {
                return Place::Expression(b.owner);
            }
            let first_after = self.entries.partition_point(|e| e.line < index);
            let parent = parent_at(&self.entries, first_after, level);
            if !self.well_indented(parent, level) {
                return Place::None;
            }
            return Place::Keyword {
                parent,
                level,
                line: index,
                own: None,
                span: at_cursor,
            };
        }
        let e = match self.roles.get(index) {
            Some(Role::Entry(e)) => *e,
            Some(Role::Expression(owner)) => return Place::Expression(*owner),
            Some(Role::Trivia) | None => return Place::None,
        };
        let Some(entry) = self.entries.get(e) else {
            return Place::None;
        };
        let absolute = |s: Span| Span {
            start: line_start + s.start,
            end: line_start + s.end,
        };
        let ws = leading_ws_len(text);
        if col < ws || !self.well_indented(entry.parent, entry.level) {
            return Place::None;
        }
        let keyword = entry.keyword.get(text);
        if entry.is_ref {
            if col < entry.keyword.start {
                return Place::None;
            }
            if col <= entry.keyword.end {
                return if entry.level == 0 {
                    Place::RefKind {
                        span: absolute(entry.keyword),
                    }
                } else {
                    Place::None
                };
            }
        } else if col <= entry.keyword.end {
            return Place::Keyword {
                parent: entry.parent,
                level: entry.level,
                line: index,
                own: Some(e),
                span: absolute(entry.keyword),
            };
        } else if keyword == "ref" && entry.level == 0 && entry.operator.is_none() {
            // `ref ` and nothing yet: the object type comes next.
            return match entry.name {
                Some(n) if col > n.end => Place::None,
                Some(n) => Place::RefKind { span: absolute(n) },
                None => Place::RefKind { span: at_cursor },
            };
        }
        if let Some(op) = entry.operator
            && col > entry.operator_at
        {
            return match op {
                Operator::Equals => Place::Expression(e),
                Operator::Colon => {
                    let span = entry
                        .value
                        .filter(|v| v.end > v.start && col >= v.start)
                        .map_or(at_cursor, absolute);
                    Place::Value { entry: e, span }
                }
            };
        }
        match entry.name {
            Some(n) if col > n.end => Place::None,
            Some(n) => Place::Name {
                entry: e,
                span: absolute(n),
            },
            None => Place::Name {
                entry: e,
                span: at_cursor,
            },
        }
    }

    /// A line at `level` under `parent` is one level deeper than it.
    fn well_indented(&self, parent: Option<usize>, level: usize) -> bool {
        match parent.and_then(|p| self.entries.get(p)) {
            Some(p) => p.level + 1 == level,
            None => level == 0,
        }
    }
}

/// The parent of a line at `level` that comes after the first `count` entries.
fn parent_at(entries: &[Entry], count: usize, level: usize) -> Option<usize> {
    let mut at = count.checked_sub(1);
    let mut steps = 0usize;
    while let Some(i) = at {
        let e = entries.get(i)?;
        if e.level < level {
            return Some(i);
        }
        steps += 1;
        if steps > entries.len() {
            return None;
        }
        at = e.parent;
    }
    None
}

/// The tokens of a structural line; never fails (what cannot be read is left out).
fn header(text: &str, ws: usize) -> Entry {
    let b = text.as_bytes();
    let mut entry = Entry {
        line: 0,
        level: 0,
        is_ref: false,
        keyword: Span { start: ws, end: ws },
        name: None,
        operator: None,
        operator_at: text.len(),
        value: None,
        parent: None,
    };
    if let Ok(h) = lex_header(text, ws, 0) {
        entry.is_ref = h.is_ref;
        entry.keyword = h.keyword;
        entry.name = h.name;
        entry.operator = h.operator;
        entry.value = h.value;
        if h.operator.is_some() {
            entry.operator_at = skip_ws(b, h.name.map_or(h.keyword.end, |n| n.end));
        }
        return entry;
    }
    let mut i = ws;
    if b.get(i..i + 3) == Some(b"ref".as_slice()) && matches!(b.get(i + 3), Some(b' ' | b'\t')) {
        let j = skip_ws(b, i + 3);
        if b.get(j).copied().is_some_and(is_ident_start) {
            entry.is_ref = true;
            i = j;
        }
    }
    let start = i;
    while b.get(i).copied().is_some_and(is_ident_char) {
        i += 1;
    }
    entry.keyword = Span { start, end: i };
    if i == start {
        return entry;
    }
    let mut j = skip_ws(b, i);
    match b.get(j) {
        None => return entry,
        Some(b':') => {
            entry.operator = Some(Operator::Colon);
            entry.operator_at = j;
            entry.value = Some(trimmed_rest(text, j + 1));
            return entry;
        }
        Some(b'=') => {
            entry.operator = Some(Operator::Equals);
            entry.operator_at = j;
            entry.value = Some(trimmed_rest(text, j + 1));
            return entry;
        }
        Some(_) => {}
    }
    // A name the lexer refused: an unterminated quote runs to the end of the line, anything
    // else to an `=` (or the end).
    let name_start = j;
    let mut quoted = false;
    while let Some(&c) = b.get(j) {
        match c {
            b'\'' => quoted = !quoted,
            b'=' if !quoted => break,
            _ => {}
        }
        j += 1;
    }
    let name_end = name_start + text.get(name_start..j).unwrap_or_default().trim_end().len();
    if name_end > name_start {
        entry.name = Some(Span {
            start: name_start,
            end: name_end,
        });
    }
    if b.get(j) == Some(&b'=') {
        entry.operator = Some(Operator::Equals);
        entry.operator_at = j;
        entry.value = Some(trimmed_rest(text, j + 1));
    }
    entry
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cst::{Node, parse_file};

    /// Every node of a parsed file has an entry with the same tokens, level and parent.
    fn same_as_parser(text: &str) {
        let file = parse_file("t.tmdl", text).unwrap();
        let outline = Outline::new(text);
        let mut nodes: Vec<(&Node, Option<u32>)> = Vec::new();
        fn walk<'a>(
            nodes: &'a [Node],
            parent: Option<u32>,
            out: &mut Vec<(&'a Node, Option<u32>)>,
        ) {
            for n in nodes {
                out.push((n, parent));
                walk(&n.children, Some(n.line_number()), out);
            }
        }
        walk(&file.nodes, None, &mut nodes);
        assert_eq!(nodes.len(), outline.entries.len(), "{text}");
        for ((node, parent), entry) in nodes.iter().zip(&outline.entries) {
            let line = &outline.lines[entry.line];
            assert_eq!(line.number, node.line_number());
            assert_eq!(entry.level, node.level());
            assert_eq!(entry.is_ref, node.is_ref());
            assert_eq!(entry.keyword, node.header.keyword);
            assert_eq!(entry.name, node.header.name);
            assert_eq!(entry.operator, node.header.operator);
            assert_eq!(entry.value, node.header.value);
            let entry_parent = entry
                .parent
                .map(|p| outline.lines[outline.entries[p].line].number);
            assert_eq!(entry_parent, *parent);
        }
    }

    #[test]
    fn matches_the_parser() {
        same_as_parser(
            "table T\n\tmeasure A =\n\t\t\tVAR x = 1\n\n\t\t\tRETURN\n\t\t\t    x\n\n\t\tformatString: 0\n\tpartition P = m\n\t\tmode: import\n\t\tsource =\n\t\t\tlet\n\t\t\tin\n\t\t\t    s\n\tmeasure B = ```\n\t\t\t  keep  \n\t\t\t```\n\tmeasure C = ```x```\n\tcolumn 'D E'\n\t\tdataType: string\n\n\t\tannotation X = 1\n",
        );
        same_as_parser(
            "\u{feff}/// d\r\nexpression P = \"x\" meta [IsParameterQuery=true]\r\n\tlineageTag: a\r\n\r\nref table T\r\n",
        );
        same_as_parser("model Model\n  culture: en-US\n  annotation A = 1\n");
    }

    #[test]
    fn half_typed_lines_keep_their_structure() {
        let text = "table T\n\tcolumn 'Order\n\t\tdataType: \n\tmeasure M =\n\t\t\tSUM(\n\t\tform";
        let o = Outline::new(text);
        let words: Vec<(&str, usize, Option<usize>)> = o
            .entries
            .iter()
            .map(|e| (o.keyword(e), e.level, e.parent))
            .collect();
        assert_eq!(
            words,
            [
                ("table", 0, None),
                ("column", 1, Some(0)),
                ("dataType", 2, Some(1)),
                ("measure", 1, Some(0)),
                ("form", 2, Some(3)),
            ]
        );
        assert_eq!(o.name(&o.entries[1]).as_deref(), Some("'Order"));
        assert_eq!(o.roles[4], Role::Expression(3));
    }

    #[test]
    fn places() {
        let text = "table T\n\tcolumn C\n\t\tdataType: str\n\tmeasure M =\n\t\t\tSUM(1)\n\t\t\n// c\nref tab\nref \n\t\t\t\tx\n";
        let o = Outline::new(text);
        let at = |needle: &str, back: usize| text.find(needle).unwrap() + needle.len() - back;
        assert!(matches!(
            o.place(at("\tcolumn", 2)),
            Place::Keyword {
                parent: Some(0),
                level: 1,
                own: Some(1),
                ..
            }
        ));
        assert!(matches!(
            o.place(at("column C", 0)),
            Place::Name { entry: 1, .. }
        ));
        match o.place(at("str", 1)) {
            Place::Value { entry: 2, span } => assert_eq!(span.get(text), "str"),
            other => panic!("{other:?}"),
        }
        assert_eq!(o.place(at("SUM(", 0)), Place::Expression(3));
        // The blank line after the expression, at the measure's property level.
        assert!(matches!(
            o.place(at("SUM(1)\n\t\t", 0)),
            Place::Keyword {
                parent: Some(3),
                level: 2,
                own: None,
                ..
            }
        ));
        assert_eq!(o.place(at("// c", 1)), Place::None);
        match o.place(at("ref tab", 0)) {
            Place::RefKind { span } => assert_eq!(span.get(text), "tab"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(o.place(at("ref \n", 1)), Place::RefKind { .. }));
        // Indented too deep for its parent.
        assert_eq!(o.place(at("\t\t\t\tx", 0)), Place::None);
        // After the last line's terminator: a new top-level line.
        assert!(matches!(
            o.place(text.len()),
            Place::Keyword {
                parent: None,
                level: 0,
                ..
            }
        ));
    }
}
