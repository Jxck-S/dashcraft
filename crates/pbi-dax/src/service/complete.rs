//! Completion: the functions, tables, columns, measures, variables and
//! keywords that fit at a cursor. Token based, so it answers while the text
//! does not parse yet.

use std::collections::HashSet;

use pbi_model::{dax_column_ref, dax_measure_ref, dax_table_ref};

use super::catalog;
use super::help::{floor_char_boundary, in_comment, keyword_description, signature_help};
use super::schema::{Schema, SchemaTable};
use crate::syntax::{Keyword, LexError, Span, Token, TokenKind, tokenize};

/// What a completion item inserts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompletionKind {
    Function,
    /// A user-defined function of the model or query.
    UserFunction,
    Table,
    Column,
    Measure,
    Variable,
    Keyword,
}

impl CompletionKind {
    /// `function`, `userFunction`, `table`, `column`, `measure`,
    /// `variable` or `keyword`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            CompletionKind::Function => "function",
            CompletionKind::UserFunction => "userFunction",
            CompletionKind::Table => "table",
            CompletionKind::Column => "column",
            CompletionKind::Measure => "measure",
            CompletionKind::Variable => "variable",
            CompletionKind::Keyword => "keyword",
        }
    }
}

/// One suggestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    /// What a list shows: `SUM`, `Sales`, `'Product Category'[Category]`,
    /// `[Total Sales]`, `total`, `ORDER BY`.
    pub label: String,
    pub kind: CompletionKind,
    /// The text that replaces [`Completions::span`]: `SUM(` (`PI()` for a
    /// function without arguments, `SUM` when a `(` already follows), the
    /// label for anything else.
    pub insert_text: String,
    /// `SUM(column) → number`, `column (string)`, `measure`, `table`,
    /// `variable`, `keyword`, `user-defined function`.
    pub detail: String,
    /// The home table of a column or measure.
    pub table: Option<String>,
    /// A function's or keyword's description (DashCraft's words), or the
    /// model description of a table, column or measure.
    pub description: Option<String>,
}

/// The suggestions at a cursor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Completions {
    /// The text a chosen item replaces: the name being typed, or an empty
    /// span at the cursor.
    pub span: Span,
    /// The typed text the items match (`Tot` in `[Tot`); empty when nothing
    /// is typed yet.
    pub prefix: String,
    /// Best match first.
    pub items: Vec<CompletionItem>,
    /// More items matched than the limit allowed.
    pub incomplete: bool,
}

/// The completions at `offset` (a byte offset; clamped to the text), best
/// first and at most `max_items` (0: no limit).
///
/// Matches ignore case and rank an exact name first, then names that start
/// with the typed text, then a word of the name that does, then names that
/// contain it. Inside a comment, string or number, and where a new name is
/// being declared (after `VAR`), there are none. In a query, the `DEFINE`
/// entries count too, even while the text does not parse.
#[must_use]
pub fn complete(src: &str, offset: usize, schema: &Schema, max_items: usize) -> Completions {
    let offset = floor_char_boundary(src, offset);
    let tokens = tokenize(src);
    let query = is_query(src, &tokens);
    let empty = Completions {
        span: Span::empty(offset),
        ..Completions::default()
    };
    let Some(cx) = context(src, &tokens, offset, query) else {
        return empty;
    };
    if matches!(cx.want, Want::Nothing) {
        return Completions {
            span: cx.span,
            prefix: cx.prefix,
            ..Completions::default()
        };
    }
    let local = if query {
        query_schema(src, &tokens, schema)
    } else {
        None
    };
    let schema = local.as_ref().unwrap_or(schema);

    let mut boost = HashSet::new();
    if let Some(p) = signature_help(src, cx.span.start)
        .and_then(|h| h.active_parameter.and_then(|i| h.function.params.get(i)))
    {
        let name = p.name.to_ascii_lowercase();
        if name.contains("table") {
            boost.insert(CompletionKind::Table);
        }
        if name.contains("column") {
            boost.insert(CompletionKind::Column);
        }
    }
    let mut c = Collector {
        prefix: cx.prefix.to_lowercase(),
        boost,
        bracket_order: false,
        out: Vec::new(),
    };
    match &cx.want {
        Want::Nothing => {}
        Want::Expression { start } => {
            for v in variables(src, &tokens, cx.span.start) {
                c.offer(CompletionKind::Variable, &[&v], || {
                    item(&v, CompletionKind::Variable, v.clone(), "variable")
                });
            }
            offer_functions(&mut c, schema, cx.paren_follows);
            offer_tables(&mut c, schema);
            offer_qualified_columns(&mut c, schema);
            for t in schema.tables() {
                offer_members(&mut c, t, false);
            }
            let words: &[&str] = if *start {
                &["VAR", "RETURN", "DEFINE", "EVALUATE"]
            } else {
                &["VAR", "RETURN"]
            };
            offer_keywords(&mut c, words);
        }
        Want::AfterOperand => {
            let words: &[&str] = if query {
                &[
                    "IN", "VAR", "RETURN", "EVALUATE", "ORDER BY", "ASC", "DESC", "START AT",
                    "MEASURE", "COLUMN", "TABLE", "FUNCTION",
                ]
            } else {
                &["IN", "VAR", "RETURN"]
            };
            offer_keywords(&mut c, words);
        }
        Want::Tables => offer_tables(&mut c, schema),
        Want::Quoted => {
            offer_tables(&mut c, schema);
            offer_qualified_columns(&mut c, schema);
        }
        Want::Members(Some(table)) => {
            c.bracket_order = true;
            if let Some(t) = schema.table(table) {
                offer_members(&mut c, t, true);
            }
        }
        Want::Members(None) => {
            c.bracket_order = true;
            for t in schema.tables() {
                offer_members(&mut c, t, true);
            }
        }
        Want::Keywords(words) => offer_keywords(&mut c, words),
    }
    let (items, incomplete) = c.finish(max_items);
    Completions {
        span: cx.span,
        prefix: cx.prefix,
        items,
        incomplete,
    }
}

/// What fits at the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Want {
    /// Inside a declaration (`VAR na…`) or where only an operator fits.
    Nothing,
    /// Any expression: variables, functions, tables, qualified columns,
    /// measures and the keywords that start an expression (and a query, at
    /// the start of the text).
    Expression {
        start: bool,
    },
    /// After a complete operand: only keywords continue.
    AfterOperand,
    /// A table name (a qualifier, the table after `MEASURE` in a query).
    Tables,
    /// Inside `'…`: tables and their qualified columns.
    Quoted,
    /// Inside `[…` (or right after `'Table'`): columns and measures as
    /// `[name]`, of one table when qualified.
    Members(Option<String>),
    Keywords(&'static [&'static str]),
}

struct Context {
    want: Want,
    span: Span,
    prefix: String,
    /// A `(` follows the replaced text directly.
    paren_follows: bool,
}

fn is_query(src: &str, tokens: &[Token]) -> bool {
    tokens
        .iter()
        .find(|t| !t.is_trivia())
        .is_some_and(|t| matches!(t.keyword(src), Some(Keyword::Define | Keyword::Evaluate)))
}

fn context(src: &str, tokens: &[Token], offset: usize, query: bool) -> Option<Context> {
    let Some(i) = tokens
        .iter()
        .position(|t| t.span.start < offset && offset <= t.span.end)
    else {
        return Some(gap(src, tokens, 0, offset, query));
    };
    let t = tokens.get(i)?;
    if in_comment(t, offset) {
        return None;
    }
    let inside = offset < t.span.end;
    let qualifies = |t: &Token| {
        tokens.get(i + 1).is_some_and(|n| {
            n.span.start == t.span.end
                && matches!(
                    n.kind,
                    TokenKind::BracketIdent | TokenKind::Error(LexError::UnterminatedBracketIdent)
                )
        })
    };
    match t.kind {
        TokenKind::Ident => {
            let paren_follows = tokens
                .get(i + 1)
                .is_some_and(|n| n.kind == TokenKind::LParen && n.span.start == t.span.end);
            let want = if qualifies(t) {
                Want::Tables
            } else {
                let mut sig = significant_before(tokens, i);
                after(src, sig.next(), sig.next(), query)
            };
            Some(Context {
                want,
                span: t.span,
                prefix: src.get(t.span.start..offset)?.to_string(),
                paren_follows,
            })
        }
        TokenKind::QuotedIdent | TokenKind::Error(LexError::UnterminatedQuotedIdent)
            if inside || t.kind != TokenKind::QuotedIdent =>
        {
            let span = if t.kind == TokenKind::QuotedIdent {
                t.span
            } else {
                Span::new(t.span.start, offset)
            };
            Some(Context {
                want: if qualifies(t) {
                    Want::Tables
                } else {
                    Want::Quoted
                },
                span,
                prefix: typed_name(src, t.span.start, offset, "''", "'"),
                paren_follows: false,
            })
        }
        TokenKind::BracketIdent | TokenKind::Error(LexError::UnterminatedBracketIdent)
            if inside || t.kind != TokenKind::BracketIdent =>
        {
            let span = if t.kind == TokenKind::BracketIdent {
                t.span
            } else {
                Span::new(t.span.start, offset)
            };
            let table = i
                .checked_sub(1)
                .and_then(|p| tokens.get(p))
                .filter(|p| {
                    matches!(p.kind, TokenKind::Ident | TokenKind::QuotedIdent)
                        && p.span.end == t.span.start
                })
                .map(|p| p.value(src));
            Some(Context {
                want: Want::Members(table),
                span,
                prefix: typed_name(src, t.span.start, offset, "]]", "]"),
                paren_follows: false,
            })
        }
        // Inside a literal, or a malformed token.
        TokenKind::String
        | TokenKind::DateTime
        | TokenKind::QuotedIdent
        | TokenKind::BracketIdent
            if inside =>
        {
            None
        }
        TokenKind::Number | TokenKind::Parameter | TokenKind::Error(_) => None,
        _ => Some(gap(src, tokens, i + 1, offset, query)),
    }
}

/// The cursor is between tokens; `upto` tokens lie before it.
fn gap(src: &str, tokens: &[Token], upto: usize, offset: usize, query: bool) -> Context {
    let mut sig = significant_before(tokens, upto);
    let prev = sig.next();
    let want = match prev {
        // `'Sales'|`: the table's columns and measures.
        Some(p) if p.kind == TokenKind::QuotedIdent && p.span.end == offset => {
            Want::Members(Some(p.value(src)))
        }
        _ => after(src, prev, sig.next(), query),
    };
    Context {
        want,
        span: Span::empty(offset),
        prefix: String::new(),
        paren_follows: false,
    }
}

/// The significant tokens before `tokens[upto]`, nearest first.
fn significant_before(tokens: &[Token], upto: usize) -> impl Iterator<Item = &Token> {
    tokens
        .get(..upto)
        .unwrap_or(tokens)
        .iter()
        .rev()
        .filter(|t| !t.is_trivia())
}

/// What fits after the token `prev` (preceded by `before`).
fn after(src: &str, prev: Option<&Token>, before: Option<&Token>, query: bool) -> Want {
    let Some(p) = prev else {
        return Want::Expression { start: true };
    };
    match p.kind {
        TokenKind::Ident => match p.keyword(src) {
            Some(Keyword::Var) => Want::Nothing,
            Some(Keyword::Define) => {
                Want::Keywords(&["MEASURE", "VAR", "TABLE", "COLUMN", "FUNCTION"])
            }
            Some(Keyword::Order) => Want::Keywords(&["BY"]),
            Some(Keyword::Start) => Want::Keywords(&["AT"]),
            Some(Keyword::Measure | Keyword::Column) if query => Want::Tables,
            Some(Keyword::Table | Keyword::Function) if query => Want::Nothing,
            Some(
                Keyword::True
                | Keyword::False
                | Keyword::Asc
                | Keyword::Desc
                | Keyword::Measure
                | Keyword::Column
                | Keyword::Table
                | Keyword::Function,
            ) => Want::AfterOperand,
            Some(_) => Want::Expression { start: false },
            // The name a `VAR` declares: `=` comes next.
            None if before.is_some_and(|b| b.keyword(src) == Some(Keyword::Var)) => Want::Nothing,
            None => Want::AfterOperand,
        },
        TokenKind::QuotedIdent
        | TokenKind::BracketIdent
        | TokenKind::String
        | TokenKind::DateTime
        | TokenKind::Number
        | TokenKind::Parameter
        | TokenKind::RParen
        | TokenKind::RBrace => Want::AfterOperand,
        _ => Want::Expression { start: false },
    }
}

/// The text of a `'…` or `[…` name from after its opening character to
/// `offset`, unescaped.
fn typed_name(src: &str, start: usize, offset: usize, escaped: &str, plain: &str) -> String {
    src.get(start.saturating_add(1)..offset)
        .unwrap_or("")
        .replace(escaped, plain)
}

/// The variables declared before `before` whose scope reaches it, in
/// declaration order. A `VAR` becomes visible at the next `VAR` or `RETURN`
/// of its block (not inside its own expression) and goes out of scope at the
/// end of the argument or parenthesis that contains the block.
fn variables(src: &str, tokens: &[Token], before: usize) -> Vec<String> {
    struct Var {
        name: String,
        depth: usize,
    }
    let sig: Vec<&Token> = tokens
        .iter()
        .filter(|t| !t.is_trivia() && t.span.end <= before)
        .collect();
    let mut depth = 0usize;
    let mut visible: Vec<Var> = Vec::new();
    let mut pending: Vec<Var> = Vec::new();
    let commit = |pending: &mut Vec<Var>, visible: &mut Vec<Var>, depth: usize| {
        let (now, later): (Vec<Var>, Vec<Var>) = pending.drain(..).partition(|v| v.depth == depth);
        visible.extend(now);
        *pending = later;
    };
    for (k, t) in sig.iter().enumerate() {
        match t.kind {
            TokenKind::LParen | TokenKind::LBrace => depth += 1,
            TokenKind::RParen | TokenKind::RBrace => {
                depth = depth.saturating_sub(1);
                visible.retain(|v| v.depth <= depth);
                pending.retain(|v| v.depth <= depth);
            }
            TokenKind::Comma if depth > 0 => {
                visible.retain(|v| v.depth < depth);
                pending.retain(|v| v.depth < depth);
            }
            TokenKind::Ident => match t.keyword(src) {
                Some(Keyword::Var) => {
                    commit(&mut pending, &mut visible, depth);
                    if let Some(n) = sig.get(k + 1).filter(|n| n.kind == TokenKind::Ident) {
                        pending.push(Var {
                            name: n.text(src).to_string(),
                            depth,
                        });
                    }
                }
                Some(
                    Keyword::Return
                    | Keyword::Define
                    | Keyword::Evaluate
                    | Keyword::Measure
                    | Keyword::Column
                    | Keyword::Table
                    | Keyword::Function,
                ) => commit(&mut pending, &mut visible, depth),
                _ => {}
            },
            _ => {}
        }
    }
    // A later declaration of the same name shadows an earlier one.
    let mut seen = HashSet::new();
    let mut names: Vec<String> = visible
        .into_iter()
        .rev()
        .filter(|v| seen.insert(v.name.to_lowercase()))
        .map(|v| v.name)
        .collect();
    names.reverse();
    names
}

/// `schema` plus the `DEFINE` entries of a query, found on tokens so a
/// query that does not parse yet still has them. `None` when it has none.
fn query_schema(src: &str, tokens: &[Token], schema: &Schema) -> Option<Schema> {
    let sig: Vec<&Token> = tokens.iter().filter(|t| !t.is_trivia()).collect();
    let mut local: Option<Schema> = None;
    for (k, t) in sig.iter().enumerate() {
        let Some(kw) = t.keyword(src) else { continue };
        let Some(name) = sig.get(k + 1) else { continue };
        let member = sig
            .get(k + 2)
            .filter(|m| m.kind == TokenKind::BracketIdent && m.span.start == name.span.end);
        let named = matches!(name.kind, TokenKind::Ident | TokenKind::QuotedIdent);
        match kw {
            Keyword::Measure | Keyword::Column if named => {
                if let Some(m) = member {
                    let s = local.get_or_insert_with(|| schema.clone());
                    let (table, member) = (name.value(src), m.value(src));
                    if kw == Keyword::Measure {
                        s.add_measure(&table, &member);
                    } else {
                        s.add_column(&table, &member);
                    }
                }
            }
            Keyword::Table if named => {
                local
                    .get_or_insert_with(|| schema.clone())
                    .add_open_table(&name.value(src));
            }
            Keyword::Function if name.kind == TokenKind::Ident => {
                local
                    .get_or_insert_with(|| schema.clone())
                    .add_function(name.text(src));
            }
            _ => {}
        }
    }
    local
}

fn item(label: &str, kind: CompletionKind, insert_text: String, detail: &str) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind,
        insert_text,
        detail: detail.to_string(),
        table: None,
        description: None,
    }
}

fn offer_functions(c: &mut Collector, schema: &Schema, paren_follows: bool) {
    for f in catalog::functions() {
        c.offer(CompletionKind::Function, &[f.name], || {
            let insert = if paren_follows {
                f.name.to_string()
            } else if f.max_args == Some(0) {
                format!("{}()", f.name)
            } else {
                format!("{}(", f.name)
            };
            CompletionItem {
                description: Some(f.description.to_string()),
                ..item(f.name, CompletionKind::Function, insert, &f.label())
            }
        });
    }
    for name in schema.functions() {
        c.offer(CompletionKind::UserFunction, &[name], || {
            let insert = if paren_follows {
                name.clone()
            } else {
                format!("{name}(")
            };
            item(
                name,
                CompletionKind::UserFunction,
                insert,
                "user-defined function",
            )
        });
    }
}

fn offer_tables(c: &mut Collector, schema: &Schema) {
    for t in schema.tables() {
        c.offer(CompletionKind::Table, &[t.name()], || {
            let r = dax_table_ref(t.name());
            CompletionItem {
                description: t.details().description.clone(),
                ..item(&r, CompletionKind::Table, r.clone(), "table")
            }
        });
    }
}

fn offer_qualified_columns(c: &mut Collector, schema: &Schema) {
    for t in schema.tables() {
        for col in t.columns() {
            c.offer(CompletionKind::Column, &[t.name(), col], || {
                let r = dax_column_ref(t.name(), col);
                column_item(t, col, r)
            });
        }
    }
}

/// The measures of `t` and, with `columns`, its columns, as `[name]`.
fn offer_members(c: &mut Collector, t: &SchemaTable, columns: bool) {
    if columns {
        for col in t.columns() {
            c.offer(CompletionKind::Column, &[col], || {
                column_item(t, col, dax_measure_ref(col))
            });
        }
    }
    for m in t.measures() {
        c.offer(CompletionKind::Measure, &[m], || {
            let r = dax_measure_ref(m);
            let d = t.measure_details(m);
            CompletionItem {
                table: Some(t.name().to_string()),
                description: d.and_then(|d| d.description.clone()),
                ..item(&r, CompletionKind::Measure, r.clone(), "measure")
            }
        });
    }
}

fn column_item(t: &SchemaTable, col: &str, label: String) -> CompletionItem {
    let d = t.column_details(col);
    let detail = match d.and_then(|d| d.data_type.as_deref()) {
        Some(ty) => format!("column ({ty})"),
        None => "column".to_string(),
    };
    CompletionItem {
        table: Some(t.name().to_string()),
        description: d.and_then(|d| d.description.clone()),
        ..item(&label, CompletionKind::Column, label.clone(), &detail)
    }
}

fn offer_keywords(c: &mut Collector, words: &[&str]) {
    for w in words {
        c.offer(CompletionKind::Keyword, &[w], || {
            let first = w.split(' ').next().unwrap_or(w);
            CompletionItem {
                description: Keyword::from_ident(first).map(|k| keyword_description(k).to_string()),
                ..item(w, CompletionKind::Keyword, (*w).to_string(), "keyword")
            }
        });
    }
}

struct Candidate {
    quality: u8,
    rank: u8,
    key: String,
    item: CompletionItem,
}

struct Collector {
    /// Lower-cased typed text.
    prefix: String,
    /// Kinds the active parameter asks for (a `table` or `column`
    /// parameter); they rank first.
    boost: HashSet<CompletionKind>,
    /// Inside `[…`: measures before columns, unless a column is asked for.
    bracket_order: bool,
    out: Vec<Candidate>,
}

impl Collector {
    /// Adds the item `make` builds when one of `keys` (the names it is made
    /// of: a qualified column's table and column) matches the prefix. Items
    /// sort by those names, so `[Region]` comes before `[Region Code]`.
    fn offer(
        &mut self,
        kind: CompletionKind,
        keys: &[&str],
        make: impl FnOnce() -> CompletionItem,
    ) {
        let Some(quality) = keys.iter().filter_map(|k| quality(k, &self.prefix)).min() else {
            return;
        };
        let rank = if self.boost.contains(&kind) {
            0
        } else if self.bracket_order {
            match kind {
                CompletionKind::Measure => 1,
                _ => 2,
            }
        } else {
            match kind {
                CompletionKind::Variable => 1,
                CompletionKind::Function
                | CompletionKind::UserFunction
                | CompletionKind::Keyword => 2,
                CompletionKind::Table => 3,
                CompletionKind::Column => 4,
                CompletionKind::Measure => 5,
            }
        };
        let key = keys.join("\u{1}").to_lowercase();
        self.out.push(Candidate {
            quality,
            rank,
            key,
            item: make(),
        });
    }

    fn finish(mut self, max_items: usize) -> (Vec<CompletionItem>, bool) {
        self.out.sort_by(|a, b| {
            (a.quality, a.rank, &a.key, &a.item.label).cmp(&(
                b.quality,
                b.rank,
                &b.key,
                &b.item.label,
            ))
        });
        // The same `[Date]` column of several tables shows once.
        let mut seen = HashSet::new();
        let mut items: Vec<CompletionItem> = self
            .out
            .into_iter()
            .filter(|c| seen.insert((c.item.kind, c.key.clone())))
            .map(|c| c.item)
            .collect();
        let incomplete = max_items > 0 && items.len() > max_items;
        if incomplete {
            items.truncate(max_items);
        }
        (items, incomplete)
    }
}

/// How well `candidate` matches the lower-cased `prefix`: 0 the same name,
/// 1 starts with it, 2 a word of it starts with it, 3 contains it; `None`
/// when it does not match. Every name matches an empty prefix (1).
fn quality(candidate: &str, prefix: &str) -> Option<u8> {
    if prefix.is_empty() {
        return Some(1);
    }
    let lower = candidate.to_lowercase();
    if lower == prefix {
        return Some(0);
    }
    if lower.starts_with(prefix) {
        return Some(1);
    }
    let mut prev: Option<char> = None;
    for (i, ch) in candidate.char_indices() {
        let word_start = match prev {
            None => false,
            Some(p) => !p.is_alphanumeric() || (p.is_lowercase() && ch.is_uppercase()),
        };
        if word_start
            && ch.is_alphanumeric()
            && candidate
                .get(i..)
                .is_some_and(|rest| rest.to_lowercase().starts_with(prefix))
        {
            return Some(2);
        }
        prev = Some(ch);
    }
    lower.contains(prefix).then_some(3)
}
