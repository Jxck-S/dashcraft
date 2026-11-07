//! Semantic tokens and references: what every token of a DAX text is, and
//! where the name under a cursor occurs (go to definition, find references,
//! highlight). Token based like completion, so it answers while the text
//! does not parse yet.

use std::collections::{HashMap, HashSet};

use pbi_model::{dax_column_ref, dax_measure_ref, dax_table_ref};

use super::catalog::{self, is_info_function};
use super::help::floor_char_boundary;
use super::schema::{Schema, SchemaTable};
use super::scope::{self, Decl, DeclKind, DefineKind, QueryDefine, same_name};
use crate::syntax::{Keyword, LexError, Span, Token, TokenKind, is_enum_keyword, tokenize};

/// What a token is, for highlighting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SemanticKind {
    /// `VAR`, `RETURN`, `EVALUATE`, `TRUE`, `DESC`, a parameter type, …
    Keyword,
    /// A DAX function.
    Function,
    /// A user-defined function of the model or query.
    UserFunction,
    Table,
    Column,
    Measure,
    Variable,
    /// A parameter of a user-defined function.
    Parameter,
    /// A query parameter, `@name`.
    QueryParameter,
    String,
    /// `dt"2024-01-31"`.
    DateTime,
    Number,
    Comment,
    Operator,
}

impl SemanticKind {
    /// `keyword`, `function`, `userFunction`, `table`, `column`, `measure`,
    /// `variable`, `parameter`, `queryParameter`, `string`, `dateTime`,
    /// `number`, `comment` or `operator`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            SemanticKind::Keyword => "keyword",
            SemanticKind::Function => "function",
            SemanticKind::UserFunction => "userFunction",
            SemanticKind::Table => "table",
            SemanticKind::Column => "column",
            SemanticKind::Measure => "measure",
            SemanticKind::Variable => "variable",
            SemanticKind::Parameter => "parameter",
            SemanticKind::QueryParameter => "queryParameter",
            SemanticKind::String => "string",
            SemanticKind::DateTime => "dateTime",
            SemanticKind::Number => "number",
            SemanticKind::Comment => "comment",
            SemanticKind::Operator => "operator",
        }
    }
}

/// One classified token. Whitespace, punctuation (`(`, `,`, `:`) and
/// malformed tokens other than unterminated strings and comments have none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticToken {
    pub span: Span,
    pub kind: SemanticKind,
    /// The name is declared here: a `VAR`, a function parameter, or a
    /// query's `DEFINE MEASURE`, `COLUMN`, `TABLE` or `FUNCTION`.
    pub declaration: bool,
    /// A table, column, measure or function the schema does not know (never
    /// with [`Schema::unknown`], where names keep the kind their syntax
    /// gives them).
    pub unresolved: bool,
}

/// The thing a name refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Symbol {
    Function {
        name: String,
    },
    UserFunction {
        name: String,
    },
    Table {
        name: String,
    },
    /// `table` is `None` for an unqualified `[name]` that is not one
    /// table's column (several tables have it, or an expression adds it).
    Column {
        table: Option<String>,
        name: String,
    },
    /// `table` is the home table when the schema knows the measure.
    Measure {
        table: Option<String>,
        name: String,
    },
    Variable {
        name: String,
        /// The declared name.
        declaration: Span,
    },
    Parameter {
        name: String,
        /// The declared name.
        declaration: Span,
    },
}

impl Symbol {
    /// The kind a token naming it has.
    #[must_use]
    pub const fn kind(&self) -> SemanticKind {
        match self {
            Symbol::Function { .. } => SemanticKind::Function,
            Symbol::UserFunction { .. } => SemanticKind::UserFunction,
            Symbol::Table { .. } => SemanticKind::Table,
            Symbol::Column { .. } => SemanticKind::Column,
            Symbol::Measure { .. } => SemanticKind::Measure,
            Symbol::Variable { .. } => SemanticKind::Variable,
            Symbol::Parameter { .. } => SemanticKind::Parameter,
        }
    }

    /// The name, unquoted (the schema's spelling when it knows it).
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Symbol::Function { name }
            | Symbol::UserFunction { name }
            | Symbol::Table { name }
            | Symbol::Column { name, .. }
            | Symbol::Measure { name, .. }
            | Symbol::Variable { name, .. }
            | Symbol::Parameter { name, .. } => name,
        }
    }

    /// The table of a column, or the home table of a measure.
    #[must_use]
    pub fn table(&self) -> Option<&str> {
        match self {
            Symbol::Column { table, .. } | Symbol::Measure { table, .. } => table.as_deref(),
            _ => None,
        }
    }

    /// The declared name of a variable or parameter.
    #[must_use]
    pub const fn declaration(&self) -> Option<Span> {
        match self {
            Symbol::Variable { declaration, .. } | Symbol::Parameter { declaration, .. } => {
                Some(*declaration)
            }
            _ => None,
        }
    }

    /// As DAX: `SUM`, `Sales`, `'Product Category'[Category]`,
    /// `[Total Sales]`, a variable.
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            Symbol::Table { name } => dax_table_ref(name),
            Symbol::Column {
                table: Some(table),
                name,
            } => dax_column_ref(table, name),
            Symbol::Column { table: None, name } | Symbol::Measure { name, .. } => {
                dax_measure_ref(name)
            }
            _ => self.name().to_string(),
        }
    }

    /// Whether both name the same thing, ignoring case: a measure by name
    /// alone (measure names are unique in a model), a column by table and
    /// name, a variable or parameter by its declaration.
    #[must_use]
    pub fn same(&self, other: &Symbol) -> bool {
        match (self, other) {
            (Symbol::Function { name: a }, Symbol::Function { name: b })
            | (Symbol::UserFunction { name: a }, Symbol::UserFunction { name: b })
            | (Symbol::Table { name: a }, Symbol::Table { name: b })
            | (Symbol::Measure { name: a, .. }, Symbol::Measure { name: b, .. }) => same_name(a, b),
            (Symbol::Column { table: ta, name: a }, Symbol::Column { table: tb, name: b }) => {
                same_name(a, b)
                    && match (ta, tb) {
                        (Some(x), Some(y)) => same_name(x, y),
                        (None, None) => true,
                        _ => false,
                    }
            }
            (Symbol::Variable { declaration: a, .. }, Symbol::Variable { declaration: b, .. })
            | (
                Symbol::Parameter { declaration: a, .. },
                Symbol::Parameter { declaration: b, .. },
            ) => a == b,
            _ => false,
        }
    }
}

/// One place a symbol occurs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReferenceSite {
    pub span: Span,
    /// The text declares the symbol here.
    pub declaration: bool,
}

/// The symbol at a cursor and everywhere it occurs in the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct References {
    /// Spelled as the model, else as declared, else as first written.
    pub symbol: Symbol,
    /// The name at the cursor.
    pub span: Span,
    /// Where the text declares it (a `VAR`, a parameter, a query `DEFINE`);
    /// `None` for model objects and DAX functions.
    pub definition: Option<Span>,
    /// Every occurrence in source order, the declaration included.
    pub sites: Vec<ReferenceSite>,
}

/// Every token of `src` worth highlighting, in source order, with what it
/// is against `schema` (plus a query's own `DEFINE` entries). Resolution
/// follows hover: a name before `(` is a function; a bare name is a
/// variable or parameter in scope, else a table, else a keyword; `[name]`
/// is the measure, else the column of the one table that has it.
#[must_use]
pub fn semantic_tokens(src: &str, schema: &Schema) -> Vec<SemanticToken> {
    analyze(src, schema).into_iter().map(|(t, _)| t).collect()
}

/// The symbol at `offset` (a byte offset: the name that contains it, or
/// ends at it) and all its occurrences in `src`. `None` when no name is
/// there, or the name is a keyword.
#[must_use]
pub fn references(src: &str, offset: usize, schema: &Schema) -> Option<References> {
    let offset = floor_char_boundary(src, offset);
    let all = analyze(src, schema);
    let name_span = |(t, s): &&Classified| s.is_some().then_some(t.span);
    let (span, symbol) = all
        .iter()
        .find(|e| name_span(e).is_some_and(|s| s.start <= offset && offset < s.end))
        .or_else(|| {
            all.iter()
                .find(|e| name_span(e).is_some_and(|s| s.end == offset))
        })
        .and_then(|(t, s)| s.clone().map(|s| (t.span, s)))?;
    let matching: Vec<&Classified> = all
        .iter()
        .filter(|(_, s)| s.as_ref().is_some_and(|s| s.same(&symbol)))
        .collect();
    let sites: Vec<ReferenceSite> = matching
        .iter()
        .map(|(t, _)| ReferenceSite {
            span: t.span,
            declaration: t.declaration,
        })
        .collect();
    let definition = sites.iter().find(|s| s.declaration).map(|s| s.span);
    // Spelled as declared, else as first written: not as under the cursor.
    let symbol = matching
        .iter()
        .find(|(t, _)| t.declaration)
        .or_else(|| matching.first())
        .and_then(|(_, s)| s.clone())
        .unwrap_or(symbol);
    Some(References {
        symbol,
        span,
        definition,
        sites,
    })
}

type Classified = (SemanticToken, Option<Symbol>);

struct Analysis<'a> {
    src: &'a str,
    schema: &'a Schema,
    /// The schema is a model's: names it lacks are unresolved.
    known: bool,
    decls_at: HashMap<usize, &'a Decl>,
    decls_by_name: HashMap<String, Vec<&'a Decl>>,
    keywords: &'a HashSet<usize>,
    defines_at: HashMap<usize, &'a QueryDefine>,
    /// Lower-cased string literals: names an expression can add as columns
    /// (`ADDCOLUMNS(…, "Margin", …)`).
    strings: HashSet<String>,
    /// A table constructor or `GENERATESERIES`/`CALENDAR`: `[Value]`,
    /// `[Value1]`, `[Date]` exist.
    implicit_columns: bool,
    /// A call whose result columns are not known (`INFO.*`, a user-defined
    /// function): any bare `[name]` may be one of them.
    untyped_columns: bool,
    /// Bare names in `NAMEOF`'s component argument (`SELF`, `PARENT`, …).
    components: HashSet<usize>,
}

fn analyze(src: &str, schema: &Schema) -> Vec<Classified> {
    let tokens = tokenize(src);
    let query = scope::is_query(src, &tokens);
    let scopes = scope::scopes(src, &tokens, query);
    let defines = if query {
        scope::query_defines(src, &tokens)
    } else {
        Vec::new()
    };
    let local = scope::query_schema(&defines, schema);
    let schema = local.as_ref().unwrap_or(schema);
    let mut decls_by_name: HashMap<String, Vec<&Decl>> = HashMap::new();
    for d in &scopes.decls {
        decls_by_name
            .entry(d.name.to_lowercase())
            .or_default()
            .push(d);
    }
    let sig: Vec<usize> = (0..tokens.len())
        .filter(|&i| tokens.get(i).is_some_and(|t| !t.is_trivia()))
        .collect();
    let untyped_columns = sig.windows(2).any(|w| {
        let (Some(t), Some(n)) = (
            w.first().and_then(|&i| tokens.get(i)),
            w.last().and_then(|&i| tokens.get(i)),
        ) else {
            return false;
        };
        let text = t.text(src);
        t.kind == TokenKind::Ident
            && n.kind == TokenKind::LParen
            && (is_info_function(text) || schema.has_function(text))
    });
    let a = Analysis {
        src,
        schema,
        known: !schema.is_unknown(),
        decls_at: scopes.decls.iter().map(|d| (d.span.start, d)).collect(),
        decls_by_name,
        keywords: &scopes.keywords,
        defines_at: defines.iter().map(|d| (d.span.start, d)).collect(),
        strings: tokens
            .iter()
            .filter(|t| t.kind == TokenKind::String)
            .map(|t| t.value(src).to_lowercase())
            .collect(),
        implicit_columns: tokens.iter().any(|t| {
            t.kind == TokenKind::LBrace
                || (t.kind == TokenKind::Ident
                    && ["GENERATESERIES", "CALENDAR", "CALENDARAUTO"]
                        .iter()
                        .any(|f| t.text(src).eq_ignore_ascii_case(f)))
        }),
        untyped_columns,
        components: nameof_components(src, &tokens),
    };
    let mut out = Vec::new();
    let mut k = 0usize;
    for t in &tokens {
        let prev = if t.is_trivia() {
            None
        } else {
            let p = k.checked_sub(1).and_then(|p| sig.get(p));
            k += 1;
            p
        };
        let prev = prev.and_then(|&p| tokens.get(p));
        let next = sig.get(k).and_then(|&n| tokens.get(n));
        if let Some(c) = a.classify(t, prev, next) {
            out.push(c);
        }
    }
    out
}

fn plain(t: &Token, kind: SemanticKind) -> Option<Classified> {
    Some((
        SemanticToken {
            span: t.span,
            kind,
            declaration: false,
            unresolved: false,
        },
        None,
    ))
}

fn named(t: &Token, symbol: Symbol, declaration: bool, unresolved: bool) -> Option<Classified> {
    Some((
        SemanticToken {
            span: t.span,
            kind: symbol.kind(),
            declaration,
            unresolved,
        },
        Some(symbol),
    ))
}

impl Analysis<'_> {
    /// `t` with the significant tokens around it (`prev` only for a
    /// significant `t`).
    fn classify(
        &self,
        t: &Token,
        prev: Option<&Token>,
        next: Option<&Token>,
    ) -> Option<Classified> {
        match t.kind {
            TokenKind::LineComment
            | TokenKind::BlockComment
            | TokenKind::Error(LexError::UnterminatedBlockComment) => {
                plain(t, SemanticKind::Comment)
            }
            TokenKind::String | TokenKind::Error(LexError::UnterminatedString) => {
                plain(t, SemanticKind::String)
            }
            TokenKind::DateTime => plain(t, SemanticKind::DateTime),
            TokenKind::Number => plain(t, SemanticKind::Number),
            TokenKind::Parameter => plain(t, SemanticKind::QueryParameter),
            TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Caret
            | TokenKind::Amp
            | TokenKind::AmpAmp
            | TokenKind::PipePipe
            | TokenKind::Eq
            | TokenKind::EqEq
            | TokenKind::Neq
            | TokenKind::Lt
            | TokenKind::Gt
            | TokenKind::Le
            | TokenKind::Ge
            | TokenKind::Arrow => plain(t, SemanticKind::Operator),
            TokenKind::Ident => self.ident(t, next),
            TokenKind::QuotedIdent => self.quoted(t),
            TokenKind::BracketIdent => self.bracket(t, prev),
            _ => None,
        }
    }

    fn ident(&self, t: &Token, next: Option<&Token>) -> Option<Classified> {
        let text = t.text(self.src);
        if let Some(d) = self
            .decls_at
            .get(&t.span.start)
            .filter(|d| d.span == t.span)
        {
            return named(t, decl_symbol(d), true, false);
        }
        if let Some(d) = self.defines_at.get(&t.span.start) {
            let symbol = match d.kind {
                DefineKind::Function => Symbol::UserFunction {
                    name: d.name.clone(),
                },
                _ => Symbol::Table {
                    name: self.table_name(&d.name),
                },
            };
            return named(t, symbol, true, false);
        }
        let keyword = Keyword::from_ident(text);
        if self.keywords.contains(&t.span.start) || self.components.contains(&t.span.start) {
            return plain(t, SemanticKind::Keyword);
        }
        if next.is_some_and(|n| n.kind == TokenKind::LParen) {
            return self.call(t, text, keyword.is_some());
        }
        if keyword.is_some_and(Keyword::is_reserved) {
            return plain(t, SemanticKind::Keyword);
        }
        let qualifies = next.is_some_and(|n| {
            n.span.start == t.span.end
                && matches!(
                    n.kind,
                    TokenKind::BracketIdent | TokenKind::Error(LexError::UnterminatedBracketIdent)
                )
        });
        if !qualifies && let Some(d) = self.variable(text, t.span) {
            return named(t, decl_symbol(d), false, false);
        }
        if let Some(table) = self.schema.table(text) {
            let symbol = Symbol::Table {
                name: table.name().to_string(),
            };
            return named(t, symbol, false, false);
        }
        if !qualifies && (keyword.is_some() || is_enum_keyword(text)) {
            return plain(t, SemanticKind::Keyword);
        }
        let symbol = Symbol::Table {
            name: text.to_string(),
        };
        named(t, symbol, false, self.known)
    }

    /// A name before `(`.
    fn call(&self, t: &Token, text: &str, keyword: bool) -> Option<Classified> {
        if self.schema.has_function(text) {
            let name = self
                .schema
                .functions()
                .iter()
                .find(|f| same_name(f, text))
                .cloned()
                .unwrap_or_else(|| text.to_string());
            return named(t, Symbol::UserFunction { name }, false, false);
        }
        if let Some(f) = catalog::function(text) {
            let symbol = Symbol::Function {
                name: f.name.to_string(),
            };
            return named(t, symbol, false, false);
        }
        if keyword {
            return plain(t, SemanticKind::Keyword);
        }
        let name = text.to_string();
        if is_info_function(text) {
            return named(t, Symbol::Function { name }, false, false);
        }
        if self.known {
            named(t, Symbol::Function { name }, false, true)
        } else {
            // Without a model, an unknown name may be one of its functions.
            named(t, Symbol::UserFunction { name }, false, false)
        }
    }

    fn quoted(&self, t: &Token) -> Option<Classified> {
        let name = t.value(self.src);
        if let Some(d) = self.defines_at.get(&t.span.start) {
            let symbol = Symbol::Table {
                name: self.table_name(&d.name),
            };
            return named(t, symbol, true, false);
        }
        let table = self.schema.table(&name);
        let unresolved = self.known && table.is_none();
        let symbol = Symbol::Table {
            name: table.map_or(name, |t| t.name().to_string()),
        };
        named(t, symbol, false, unresolved)
    }

    fn bracket(&self, t: &Token, prev: Option<&Token>) -> Option<Classified> {
        let name = t.value(self.src);
        if let Some(d) = self.defines_at.get(&t.span.start) {
            let table = d.table.as_deref().map(|t| self.table_name(t));
            let symbol = match d.kind {
                DefineKind::Measure => Symbol::Measure {
                    table,
                    name: d.name.clone(),
                },
                _ => Symbol::Column {
                    table,
                    name: d.name.clone(),
                },
            };
            return named(t, symbol, true, false);
        }
        let qualifier = prev.filter(|p| {
            p.span.end == t.span.start
                && (p.kind == TokenKind::QuotedIdent
                    || (p.kind == TokenKind::Ident
                        && !p.keyword(self.src).is_some_and(Keyword::is_reserved)))
        });
        match qualifier {
            Some(q) => self.qualified(t, q, name),
            None => self.unqualified(t, name),
        }
    }

    /// `table[name]`.
    fn qualified(&self, t: &Token, q: &Token, name: String) -> Option<Classified> {
        let table_text = q.value(self.src);
        let Some(table) = self.schema.table(&table_text) else {
            let unresolved =
                self.known && self.variable(&table_text, q.span).is_none() && !self.added(&name);
            let symbol = Symbol::Column {
                table: Some(table_text),
                name,
            };
            return named(t, symbol, false, unresolved);
        };
        let table_name = Some(table.name().to_string());
        let spelled = table.spelling(&name).map(str::to_string);
        if table.has_measure(&name) {
            let symbol = Symbol::Measure {
                table: table_name,
                name: spelled.unwrap_or(name),
            };
            return named(t, symbol, false, false);
        }
        let unresolved =
            self.known && !table.has_column(&name) && !table.is_open() && !self.added(&name);
        let symbol = Symbol::Column {
            table: table_name,
            name: spelled.unwrap_or(name),
        };
        named(t, symbol, false, unresolved)
    }

    /// `[name]`: the measure, else the column of the one table that has
    /// it.
    fn unqualified(&self, t: &Token, name: String) -> Option<Classified> {
        if let Some(table) = self.schema.measure_table(&name) {
            let symbol = Symbol::Measure {
                table: Some(table.name().to_string()),
                name: table.spelling(&name).map_or(name, str::to_string),
            };
            return named(t, symbol, false, false);
        }
        let mut owners = self.schema.tables().iter().filter(|t| t.has_column(&name));
        if let Some(first) = owners.next() {
            let spelled = first
                .spelling(&name)
                .map_or_else(|| name.clone(), str::to_string);
            let table = owners.next().is_none().then(|| first.name().to_string());
            return named(
                t,
                Symbol::Column {
                    table,
                    name: spelled,
                },
                false,
                false,
            );
        }
        if self.added(&name)
            || self.untyped_columns
            || self.schema.tables().iter().any(SchemaTable::is_open)
        {
            return named(t, Symbol::Column { table: None, name }, false, false);
        }
        named(t, Symbol::Measure { table: None, name }, false, self.known)
    }

    /// The variable or parameter a bare name at `span` refers to.
    fn variable(&self, name: &str, span: Span) -> Option<&Decl> {
        self.decls_by_name
            .get(&name.to_lowercase())?
            .iter()
            .filter(|d| d.covers(span))
            .max_by_key(|d| d.span.start)
            .copied()
    }

    /// A name an expression can add to a table without the schema knowing.
    fn added(&self, name: &str) -> bool {
        let key = name.to_lowercase();
        self.strings.contains(&key)
            || (self.implicit_columns
                && (key == "date"
                    || key
                        .strip_prefix("value")
                        .is_some_and(|n| n.chars().all(|c| c.is_ascii_digit()))))
    }

    /// The schema's spelling of a table name.
    fn table_name(&self, name: &str) -> String {
        self.schema
            .table(name)
            .map_or_else(|| name.to_string(), |t| t.name().to_string())
    }
}

/// The starts of bare names given as `NAMEOF`'s second argument: component
/// keywords (`TABLE`, `COLUMN`, `SELF`, `PARENT`, `FULL`, …), as the
/// diagnostics treat them.
fn nameof_components(src: &str, tokens: &[Token]) -> HashSet<usize> {
    let sig: Vec<&Token> = tokens.iter().filter(|t| !t.is_trivia()).collect();
    let mut out = HashSet::new();
    // One entry per open `(` or `{`: whether it opens `NAMEOF(`, and the
    // commas seen at its level.
    let mut groups: Vec<(bool, usize)> = Vec::new();
    for (i, t) in sig.iter().enumerate() {
        let prev = i.checked_sub(1).and_then(|p| sig.get(p));
        match t.kind {
            TokenKind::LParen => groups.push((
                prev.is_some_and(|p| {
                    p.kind == TokenKind::Ident && p.text(src).eq_ignore_ascii_case("NAMEOF")
                }),
                0,
            )),
            TokenKind::LBrace => groups.push((false, 0)),
            TokenKind::RParen | TokenKind::RBrace => {
                groups.pop();
            }
            TokenKind::Comma => {
                if let Some(g) = groups.last_mut() {
                    g.1 += 1;
                }
            }
            TokenKind::Ident
                if groups.last() == Some(&(true, 1))
                    && prev.is_some_and(|p| p.kind == TokenKind::Comma)
                    && sig.get(i + 1).is_some_and(|n| {
                        matches!(n.kind, TokenKind::RParen | TokenKind::Comma)
                    }) =>
            {
                out.insert(t.span.start);
            }
            _ => {}
        }
    }
    out
}

fn decl_symbol(d: &Decl) -> Symbol {
    match d.kind {
        DeclKind::Var => Symbol::Variable {
            name: d.name.clone(),
            declaration: d.span,
        },
        DeclKind::Param => Symbol::Parameter {
            name: d.name.clone(),
            declaration: d.span,
        },
    }
}
