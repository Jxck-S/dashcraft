//! Signature help and hover. Both work on tokens, so they also answer for
//! text that does not parse yet (the usual state while typing).

use pbi_model::{dax_column_ref, dax_measure_ref, dax_table_ref};

use super::catalog::{self, FunctionInfo};
use super::diagnostics::SourceKind;
use super::schema::{NameDetails, Schema};
use crate::eval::is_supported_function;
use crate::syntax::{
    Define, Keyword, LexError, Span, Token, TokenKind, is_enum_keyword, parse_query, tokenize,
};

/// The function call around a cursor.
#[derive(Debug, Clone, PartialEq)]
pub struct SignatureHelp {
    pub function: &'static FunctionInfo,
    /// The function name in the text.
    pub name_span: Span,
    /// 0-based index of the argument the cursor is in.
    pub argument: usize,
    /// The parameter that argument fills ([`FunctionInfo::active_parameter`]).
    pub active_parameter: Option<usize>,
}

/// The innermost call of a catalog function whose argument list contains
/// `offset` (a byte offset; clamped to the text). `None` outside any call,
/// inside a comment, or when the innermost call is of a function the
/// catalog does not know (a user-defined function, a typo).
#[must_use]
pub fn signature_help(src: &str, offset: usize) -> Option<SignatureHelp> {
    let offset = floor_char_boundary(src, offset);
    let tokens = tokenize(src);
    struct Frame {
        name: Option<Token>,
        commas: usize,
    }
    let mut frames: Vec<Frame> = Vec::new();
    let mut prev: Option<Token> = None;
    for t in &tokens {
        if t.span.start >= offset {
            break;
        }
        if in_comment(t, offset) {
            return None;
        }
        if t.span.end > offset {
            // The token being typed.
            break;
        }
        if t.is_trivia() {
            continue;
        }
        match t.kind {
            TokenKind::LParen => frames.push(Frame {
                // `RETURN (…)` groups; `NOT(…)` is a call.
                name: prev.filter(|p| {
                    p.kind == TokenKind::Ident
                        && (p.keyword(src).is_none() || catalog::function(p.text(src)).is_some())
                }),
                commas: 0,
            }),
            TokenKind::LBrace => frames.push(Frame {
                name: None,
                commas: 0,
            }),
            TokenKind::RParen | TokenKind::RBrace => {
                frames.pop();
            }
            TokenKind::Comma => {
                if let Some(f) = frames.last_mut() {
                    f.commas += 1;
                }
            }
            _ => {}
        }
        prev = Some(*t);
    }
    let frame = frames.iter().rev().find(|f| f.name.is_some())?;
    let name = frame.name?;
    let function = catalog::function(name.text(src))?;
    Some(SignatureHelp {
        function,
        name_span: name.span,
        argument: frame.commas,
        active_parameter: function.active_parameter(frame.commas),
    })
}

/// What a hover is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HoverKind {
    Function,
    /// A user-defined function of the model or query.
    UserFunction,
    Table,
    Column,
    Measure,
    Variable,
    Keyword,
}

impl HoverKind {
    /// `function`, `userFunction`, `table`, `column`, `measure`,
    /// `variable` or `keyword`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            HoverKind::Function => "function",
            HoverKind::UserFunction => "userFunction",
            HoverKind::Table => "table",
            HoverKind::Column => "column",
            HoverKind::Measure => "measure",
            HoverKind::Variable => "variable",
            HoverKind::Keyword => "keyword",
        }
    }
}

/// Information about the name under a cursor.
#[derive(Debug, Clone, PartialEq)]
pub struct Hover {
    /// The hovered token.
    pub span: Span,
    pub kind: HoverKind,
    /// The name as DAX: `SUM`, `Sales`, `'Product Category'[Category]`,
    /// `[Total Sales]`, `total` (a variable), `VAR`.
    pub name: String,
    /// The home table of a column or measure.
    pub table: Option<String>,
    /// A function's signature, `SUM(column)`.
    pub signature: Option<String>,
    /// A function's return type.
    pub returns: Option<&'static str>,
    /// A function's or keyword's description (DashCraft's words).
    pub description: Option<String>,
    /// For a function: whether DashCraft can evaluate it.
    pub supported: bool,
    /// Model details of a table, column or measure; a variable's
    /// expression.
    pub details: NameDetails,
    /// Columns and measures of a table.
    pub counts: Option<(usize, usize)>,
}

impl Hover {
    fn new(span: Span, kind: HoverKind, name: String) -> Self {
        Self {
            span,
            kind,
            name,
            table: None,
            signature: None,
            returns: None,
            description: None,
            supported: true,
            details: NameDetails::default(),
            counts: None,
        }
    }

    /// The hover as Markdown (code in `dax` fences).
    #[must_use]
    pub fn markdown(&self) -> String {
        let mut out = String::new();
        match self.kind {
            HoverKind::Function => {
                let sig = self.signature.as_deref().unwrap_or(&self.name);
                out.push_str(&format!("```dax\n{sig}\n```\n"));
                if let Some(r) = self.returns {
                    out.push_str(&format!("Returns {r}."));
                }
            }
            HoverKind::UserFunction => {
                out.push_str(&format!("`{}`: user-defined function", self.name));
            }
            HoverKind::Table => {
                out.push_str(&format!("`{}`: table", self.name));
                if let Some((c, m)) = self.counts {
                    out.push_str(&format!(
                        ", {c} column{}, {m} measure{}",
                        if c == 1 { "" } else { "s" },
                        if m == 1 { "" } else { "s" }
                    ));
                }
            }
            HoverKind::Column => {
                out.push_str(&format!("`{}`: column", self.name));
                if let Some(t) = &self.details.data_type {
                    out.push_str(&format!(" ({t})"));
                }
            }
            HoverKind::Measure => {
                out.push_str(&format!("`{}`: measure", self.name));
                if let Some(t) = &self.table {
                    out.push_str(&format!(" in `{}`", dax_table_ref(t)));
                }
            }
            HoverKind::Variable => out.push_str(&format!("`{}`: variable", self.name)),
            HoverKind::Keyword => out.push_str(&format!("`{}`: keyword", self.name)),
        }
        if self.details.hidden {
            out.push_str(" (hidden)");
        }
        if let Some(d) = self
            .description
            .as_deref()
            .or(self.details.description.as_deref())
        {
            out.push_str(&format!("\n\n{d}"));
        }
        if !self.supported {
            out.push_str("\n\nDashCraft cannot evaluate this function yet.");
        }
        if let Some(e) = self.details.expression.as_deref() {
            out.push_str(&format!("\n\n```dax\n{}\n```", e.trim()));
        }
        if let Some(f) = &self.details.format_string {
            out.push_str(&format!("\n\nFormat: `{f}`"));
        }
        out
    }
}

/// Information about the function, table, column, measure, variable or
/// keyword at `offset` (a byte offset): the token that contains it, or ends
/// at it. `None` for anything else, including names the schema does not
/// know. In a query that parses, the `DEFINE` entries count too.
#[must_use]
pub fn hover(src: &str, offset: usize, schema: &Schema) -> Option<Hover> {
    let local = query_schema(src, schema);
    let schema = local.as_ref().unwrap_or(schema);
    let offset = floor_char_boundary(src, offset);
    let tokens = tokenize(src);
    let names = |t: &Token| {
        matches!(
            t.kind,
            TokenKind::Ident | TokenKind::QuotedIdent | TokenKind::BracketIdent
        )
    };
    let i = tokens
        .iter()
        .position(|t| t.span.start <= offset && offset < t.span.end && names(t))
        .or_else(|| tokens.iter().position(|t| t.span.end == offset && names(t)))?;
    let token = *tokens.get(i)?;
    let next = tokens.iter().skip(i + 1).find(|t| !t.is_trivia()).copied();
    let prev = tokens
        .iter()
        .take(i)
        .rev()
        .find(|t| !t.is_trivia())
        .copied();
    let text = token.text(src);
    match token.kind {
        TokenKind::Ident if next.is_some_and(|n| n.kind == TokenKind::LParen) => {
            function_hover(token.span, text, schema)
        }
        TokenKind::Ident => {
            if let Some(expr) = variable_expression(&tokens, src, text, token.span.start) {
                let mut h = Hover::new(token.span, HoverKind::Variable, text.to_string());
                h.details.expression = Some(expr);
                return Some(h);
            }
            if let Some(t) = schema.table(text) {
                return Some(table_hover(token.span, t));
            }
            keyword_hover(token.span, text)
        }
        TokenKind::QuotedIdent => schema
            .table(&token.value(src))
            .map(|t| table_hover(token.span, t)),
        TokenKind::BracketIdent => {
            let name = token.value(src);
            let qualifier = prev.filter(|p| {
                matches!(p.kind, TokenKind::Ident | TokenKind::QuotedIdent)
                    && p.span.end == token.span.start
            });
            match qualifier {
                Some(q) => {
                    let t = schema.table(&q.value(src))?;
                    member_hover(token.span, t, &name)
                }
                None => {
                    if let Some(t) = schema.measure_table(&name) {
                        return member_hover(token.span, t, &name);
                    }
                    let t = schema.tables().iter().find(|t| t.has_column(&name))?;
                    member_hover(token.span, t, &name)
                }
            }
        }
        _ => None,
    }
}

/// `schema` plus the `DEFINE` entries of `src`, when it is a query that
/// parses.
fn query_schema(src: &str, schema: &Schema) -> Option<Schema> {
    if SourceKind::detect(src) != SourceKind::Query {
        return None;
    }
    let query = parse_query(src).ok()?;
    if query.defines.is_empty() {
        return None;
    }
    let mut local = schema.clone();
    let details = |e: &crate::syntax::Expr| NameDetails {
        expression: src
            .get(e.span.start..e.span.end)
            .map(|t| t.trim().to_string()),
        ..NameDetails::default()
    };
    for d in &query.defines {
        match d {
            Define::Table { name, .. } => {
                local.add_open_table(name);
            }
            Define::Function { name, .. } => local.add_function(name),
            Define::Measure {
                table, name, expr, ..
            } => local.add_measure_with(table, name, details(expr)),
            Define::Column {
                table, name, expr, ..
            } => local.add_column_with(table, name, details(expr)),
            Define::Var { .. } => {}
        }
    }
    Some(local)
}

fn function_hover(span: Span, text: &str, schema: &Schema) -> Option<Hover> {
    if schema.has_function(text) {
        let name = schema
            .functions()
            .iter()
            .find(|f| f.eq_ignore_ascii_case(text))
            .cloned()
            .unwrap_or_else(|| text.to_string());
        return Some(Hover::new(span, HoverKind::UserFunction, name));
    }
    let f = catalog::function(text)?;
    let mut h = Hover::new(span, HoverKind::Function, f.name.to_string());
    h.signature = Some(f.signature.to_string());
    h.returns = Some(f.returns.as_str());
    h.description = Some(f.description.to_string());
    h.supported = is_supported_function(f.name);
    Some(h)
}

fn table_hover(span: Span, t: &super::SchemaTable) -> Hover {
    let mut h = Hover::new(span, HoverKind::Table, dax_table_ref(t.name()));
    h.details = t.details().clone();
    h.counts = Some((t.columns().len(), t.measures().len()));
    h
}

fn member_hover(span: Span, t: &super::SchemaTable, name: &str) -> Option<Hover> {
    let spelled = t.spelling(name)?.to_string();
    let (kind, display, details) = if t.has_measure(name) {
        (
            HoverKind::Measure,
            dax_measure_ref(&spelled),
            t.measure_details(name),
        )
    } else {
        (
            HoverKind::Column,
            dax_column_ref(t.name(), &spelled),
            t.column_details(name),
        )
    };
    let mut h = Hover::new(span, kind, display);
    h.table = Some(t.name().to_string());
    h.details = details.cloned().unwrap_or_default();
    Some(h)
}

fn keyword_hover(span: Span, text: &str) -> Option<Hover> {
    let description = match Keyword::from_ident(text) {
        Some(k) => keyword_description(k),
        None if is_enum_keyword(text) => {
            "A keyword argument that picks an option of the function it is passed to."
        }
        None => return None,
    };
    let mut h = Hover::new(span, HoverKind::Keyword, text.to_ascii_uppercase());
    h.description = Some(description.to_string());
    Some(h)
}

const fn keyword_description(k: Keyword) -> &'static str {
    match k {
        Keyword::Var => {
            "Names a value or table so later expressions can reuse it; a RETURN ends the block."
        }
        Keyword::Return => "Gives the result of a VAR block.",
        Keyword::In => {
            "TRUE when the value on the left is a row of the table or list on the right."
        }
        Keyword::Not => "Negates the condition that follows.",
        Keyword::True => "The logical value TRUE.",
        Keyword::False => "The logical value FALSE.",
        Keyword::Define => {
            "Starts the definitions of a query: MEASURE, VAR, TABLE, COLUMN and FUNCTION."
        }
        Keyword::Evaluate => "Runs a table expression and returns its rows.",
        Keyword::Order | Keyword::By => "ORDER BY sorts the rows an EVALUATE returns.",
        Keyword::Asc => "Sorts from the smallest value up.",
        Keyword::Desc => "Sorts from the largest value down.",
        Keyword::Start | Keyword::At => "START AT makes the sorted rows begin at the given values.",
        Keyword::Measure => "Defines a measure for the query.",
        Keyword::Column => "Defines a calculated column for the query.",
        Keyword::Table => "Defines a calculated table for the query.",
        Keyword::Function => "Defines a user-defined function for the query.",
    }
}

/// The expression of the closest `VAR name = …` before `before` (up to the
/// next `VAR` or `RETURN` at the same nesting), trimmed.
fn variable_expression(tokens: &[Token], src: &str, name: &str, before: usize) -> Option<String> {
    let significant: Vec<&Token> = tokens.iter().filter(|t| !t.is_trivia()).collect();
    let mut found = None;
    for (i, t) in significant.iter().enumerate() {
        if t.span.start >= before {
            break;
        }
        let is_var = t.keyword(src) == Some(Keyword::Var);
        let (Some(n), Some(eq)) = (significant.get(i + 1), significant.get(i + 2)) else {
            continue;
        };
        if is_var
            && n.kind == TokenKind::Ident
            && n.text(src).eq_ignore_ascii_case(name)
            && eq.kind == TokenKind::Eq
        {
            found = Some(i + 3);
        }
    }
    let start = found?;
    let mut depth = 0usize;
    let mut first_start = None;
    let mut last_end = None;
    for t in significant.iter().skip(start) {
        match t.kind {
            TokenKind::LParen | TokenKind::LBrace => depth += 1,
            TokenKind::RParen | TokenKind::RBrace => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            TokenKind::Comma if depth == 0 => break,
            _ => {}
        }
        if depth == 0
            && matches!(
                t.keyword(src),
                Some(
                    Keyword::Var
                        | Keyword::Return
                        | Keyword::Define
                        | Keyword::Evaluate
                        | Keyword::Measure
                        | Keyword::Column
                        | Keyword::Table
                        | Keyword::Function
                )
            )
        {
            break;
        }
        last_end = Some(t.span.end);
        first_start = first_start.or(Some(t.span.start));
    }
    let text = src.get(first_start?..last_end?)?.trim();
    Some(clip(text, 500))
}

fn clip(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        Some((i, _)) => format!("{}…", s.get(..i).unwrap_or(s)),
        None => s.to_string(),
    }
}

/// Whether `offset` lies inside the comment `t` (a line comment includes
/// its end, an unterminated block comment everything after it).
fn in_comment(t: &Token, offset: usize) -> bool {
    match t.kind {
        TokenKind::LineComment | TokenKind::Error(LexError::UnterminatedBlockComment) => {
            t.span.start < offset && offset <= t.span.end
        }
        TokenKind::BlockComment => t.span.start < offset && offset < t.span.end,
        _ => false,
    }
}

fn floor_char_boundary(src: &str, offset: usize) -> usize {
    let mut i = offset.min(src.len());
    while !src.is_char_boundary(i) {
        i = i.saturating_sub(1);
    }
    i
}
