//! Diagnostics: syntax errors, unknown tables, columns, measures and
//! variables, calls with the wrong number of arguments, and functions
//! DashCraft cannot evaluate.
//!
//! The reference checks never report a name that may be valid: a bare
//! `[Name]` is accepted when it is a measure, a column of any table, a column
//! an expression adds by name (`ADDCOLUMNS(…, "Name", …)`), or an implicit
//! column (`[Value]` of a table constructor or `GENERATESERIES`, `[Date]` of
//! `CALENDAR`).

use std::collections::HashSet;

use pbi_model::{Severity, closest_name, dax_column_ref, dax_measure_ref, dax_table_ref};

use super::{Schema, catalog};
use crate::eval::{is_supported_function, supported_functions};
use crate::syntax::{
    Define, Expr, ExprKind, Keyword, ParseError, Query, Span, TokenKind, for_each_expr, line_col,
    parse_expression, parse_query, tokenize,
};

/// At most this many diagnostics are returned for one text.
pub const MAX_DIAGNOSTICS: usize = 1000;

/// What a [`Diagnostic`] reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticCode {
    /// The text does not parse.
    Syntax,
    /// A table that is not in the model; unquoted, it may be a misspelt
    /// variable.
    UnknownTable,
    /// `Table[Name]` where the table has no such column or measure.
    UnknownColumn,
    /// `[Name]` that is neither a measure nor a column.
    UnknownMeasure,
    /// Too few or too many arguments for a function of the catalog.
    ArgumentCount,
    /// A function DashCraft does not know or cannot evaluate yet. A warning:
    /// the expression may still be valid DAX.
    UnsupportedFunction,
}

impl DiagnosticCode {
    /// Stable camelCase name (`syntax`, `unknownTable`, …).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            DiagnosticCode::Syntax => "syntax",
            DiagnosticCode::UnknownTable => "unknownTable",
            DiagnosticCode::UnknownColumn => "unknownColumn",
            DiagnosticCode::UnknownMeasure => "unknownMeasure",
            DiagnosticCode::ArgumentCount => "argumentCount",
            DiagnosticCode::UnsupportedFunction => "unsupportedFunction",
        }
    }
}

/// A problem in a DAX text, with the byte range it covers.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub severity: Severity,
    pub message: String,
    /// Byte range in the checked text.
    pub span: Span,
    /// A "did you mean …" correction.
    pub suggestion: Option<String>,
}

impl Diagnostic {
    /// 1-based `(line, column)` of the start and end of the span in `src`
    /// (columns count Unicode scalar values).
    #[must_use]
    pub fn range(&self, src: &str) -> ((usize, usize), (usize, usize)) {
        (line_col(src, self.span.start), line_col(src, self.span.end))
    }

    fn error(code: DiagnosticCode, message: String, span: Span) -> Self {
        Self {
            code,
            severity: Severity::Error,
            message,
            span,
            suggestion: None,
        }
    }
}

/// Whether a text is a single expression or a query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceKind {
    /// A measure, calculated column or table, calculation item, RLS filter
    /// or other single expression.
    #[default]
    Expression,
    /// `[DEFINE …] EVALUATE …`.
    Query,
}

impl SourceKind {
    /// [`SourceKind::Query`] when the first token is `DEFINE` or
    /// `EVALUATE`.
    #[must_use]
    pub fn detect(src: &str) -> Self {
        let first = tokenize(src).into_iter().find(|t| !t.is_trivia());
        match first {
            Some(t)
                if t.kind == TokenKind::Ident
                    && matches!(t.keyword(src), Some(Keyword::Define | Keyword::Evaluate)) =>
            {
                SourceKind::Query
            }
            _ => SourceKind::Expression,
        }
    }
}

/// Every diagnostic for `src` against `schema`, in source order (at most
/// [`MAX_DIAGNOSTICS`]). An empty or blank text has none.
#[must_use]
pub fn diagnostics(src: &str, kind: SourceKind, schema: &Schema) -> Vec<Diagnostic> {
    if src.trim().is_empty() {
        return Vec::new();
    }
    let mut out = match kind {
        SourceKind::Expression => match parse_expression(src) {
            Ok(expr) => {
                let names = Names::collect(&[&expr], &[]);
                let mut out = Vec::new();
                check(&expr, src, schema, &names, &mut out);
                out
            }
            Err(e) => vec![syntax(src, &e)],
        },
        SourceKind::Query => match parse_query(src) {
            Ok(query) => check_query(&query, src, schema),
            Err(e) => vec![syntax(src, &e)],
        },
    };
    out.sort_by_key(|d| (d.span.start, d.span.end));
    out.truncate(MAX_DIAGNOSTICS);
    out
}

fn syntax(src: &str, e: &ParseError) -> Diagnostic {
    let span = if e.span.is_empty() && e.span.start >= src.len() {
        // At the end of the text: cover the last character so it shows.
        let start = src.char_indices().next_back().map_or(0, |(i, _)| i);
        Span::new(start, src.len())
    } else {
        e.span
    };
    Diagnostic::error(DiagnosticCode::Syntax, e.to_string(), span)
}

fn check_query(query: &Query, src: &str, schema: &Schema) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let exprs = query.expressions();
    if query.defines.is_empty() {
        let names = Names::collect(&exprs, &[]);
        for e in exprs {
            check(e, src, schema, &names, &mut out);
        }
        return out;
    }
    // Every DEFINE entry is visible to every expression of the query.
    let mut local = schema.clone();
    let mut query_vars = Vec::new();
    for d in &query.defines {
        match d {
            Define::Table { name, .. } => {
                local.add_open_table(name);
            }
            Define::Function { name, params, .. } => {
                local.add_function(name);
                query_vars.extend(params.iter().map(|p| p.name.clone()));
            }
            Define::Var { name, .. } => query_vars.push(name.clone()),
            Define::Measure { .. } | Define::Column { .. } => {}
        }
    }
    for d in &query.defines {
        let (table, name, is_measure) = match d {
            Define::Measure { table, name, .. } => (table, name, true),
            Define::Column { table, name, .. } => (table, name, false),
            _ => continue,
        };
        if !local.is_unknown() && local.table(table).is_none() {
            let mut diag = Diagnostic::error(
                DiagnosticCode::UnknownTable,
                format!("unknown table `{}`", dax_table_ref(table)),
                define_header(d, src),
            );
            diag.suggestion = suggest_table(&local, table);
            out.push(diag);
        }
        if is_measure {
            local.add_measure(table, name);
        } else {
            local.add_column(table, name);
        }
    }
    let names = Names::collect(&exprs, &query_vars);
    for e in exprs {
        check(e, src, &local, &names, &mut out);
    }
    out
}

/// `MEASURE 'Table'[Name]` of a `DEFINE` entry, without the expression.
fn define_header(d: &Define, src: &str) -> Span {
    let start = d.span().start;
    let end = d.expr().span.start.max(start);
    let text = Span::new(start, end).text(src);
    let trimmed = text.trim_end().trim_end_matches('=').trim_end();
    Span::new(start, start + trimmed.len())
}

/// Names that make a reference valid besides the schema: variables and
/// function parameters, names added by string literal, implicit columns.
struct Names {
    vars: HashSet<String>,
    strings: HashSet<String>,
    implicit_columns: bool,
    /// Component keywords (`NAMEOF(…, TABLE)`) that parse as table names.
    keywords: HashSet<Span>,
}

impl Names {
    fn collect(exprs: &[&Expr], extra_vars: &[String]) -> Self {
        let mut names = Names {
            vars: extra_vars.iter().map(|v| v.to_lowercase()).collect(),
            strings: HashSet::new(),
            implicit_columns: false,
            keywords: HashSet::new(),
        };
        for root in exprs {
            for_each_expr(root, |e| match &e.kind {
                ExprKind::Var { bindings, .. } => {
                    names
                        .vars
                        .extend(bindings.iter().map(|b| b.name.to_lowercase()));
                }
                ExprKind::String(s) => {
                    names.strings.insert(s.to_lowercase());
                }
                ExprKind::TableConstructor { .. } => names.implicit_columns = true,
                ExprKind::Call { name, .. }
                    if matches!(
                        name.as_str(),
                        "GENERATESERIES" | "CALENDAR" | "CALENDARAUTO"
                    ) =>
                {
                    names.implicit_columns = true;
                }
                ExprKind::Call { name, args, .. } if name == "NAMEOF" => {
                    if let Some(component) = args.get(1).map(Expr::unparen)
                        && matches!(component.kind, ExprKind::TableRef { .. })
                    {
                        names.keywords.insert(component.span);
                    }
                }
                _ => {}
            });
        }
        names
    }

    fn is_var(&self, name: &str) -> bool {
        self.vars.contains(&name.to_lowercase())
    }

    /// A name an expression can add to a table without the model knowing.
    fn is_added_column(&self, name: &str) -> bool {
        let key = name.to_lowercase();
        self.strings.contains(&key) || (self.implicit_columns && is_implicit_column(&key))
    }
}

/// `value`, `value1`, `value2`, … and `date` (lower-cased).
fn is_implicit_column(key: &str) -> bool {
    key == "date"
        || key
            .strip_prefix("value")
            .is_some_and(|n| n.chars().all(|c| c.is_ascii_digit()))
}

fn check(root: &Expr, src: &str, schema: &Schema, names: &Names, out: &mut Vec<Diagnostic>) {
    for_each_expr(root, |e| {
        if out.len() >= MAX_DIAGNOSTICS {
            return;
        }
        match &e.kind {
            ExprKind::TableRef { .. } | ExprKind::ColumnRef { .. } if schema.is_unknown() => {}
            ExprKind::TableRef { name } => {
                if names.is_var(name)
                    || names.keywords.contains(&e.span)
                    || schema.table(name).is_some()
                {
                    return;
                }
                let quoted = e.text(src).starts_with('\'');
                let message = if quoted {
                    format!("unknown table `{}`", dax_table_ref(name))
                } else {
                    format!("unknown table or variable `{name}`")
                };
                let mut diag = Diagnostic::error(DiagnosticCode::UnknownTable, message, e.span);
                diag.suggestion = suggest_table(schema, name);
                out.push(diag);
            }
            ExprKind::ColumnRef {
                table: Some(table),
                column,
            } => {
                if names.is_var(table) {
                    return;
                }
                let (table_span, column_span) = split_column_ref(e.span, src);
                let Some(t) = schema.table(table) else {
                    let mut diag = Diagnostic::error(
                        DiagnosticCode::UnknownTable,
                        format!("unknown table `{}`", dax_table_ref(table)),
                        table_span,
                    );
                    diag.suggestion = suggest_table(schema, table);
                    out.push(diag);
                    return;
                };
                if t.is_open()
                    || t.has_column(column)
                    || t.has_measure(column)
                    || names.is_added_column(column)
                {
                    return;
                }
                let mut diag = Diagnostic::error(
                    DiagnosticCode::UnknownColumn,
                    format!("unknown column `{}`", dax_column_ref(t.name(), column)),
                    column_span,
                );
                diag.suggestion = closest_name(
                    column,
                    t.columns().iter().chain(t.measures()).map(String::as_str),
                )
                .map(|c| format!("did you mean `{}`?", dax_column_ref(t.name(), c)));
                out.push(diag);
            }
            ExprKind::ColumnRef {
                table: None,
                column,
            } => {
                if schema.measure_table(column).is_some()
                    || schema.has_column_anywhere(column)
                    || names.is_added_column(column)
                    || schema.tables().iter().any(|t| t.is_open())
                {
                    return;
                }
                let mut diag = Diagnostic::error(
                    DiagnosticCode::UnknownMeasure,
                    format!("unknown measure or column `{}`", dax_measure_ref(column)),
                    e.span,
                );
                let measures = schema
                    .tables()
                    .iter()
                    .flat_map(|t| t.measures().iter().map(String::as_str));
                diag.suggestion = closest_name(column, measures)
                    .map(|m| format!("did you mean `{}`?", dax_measure_ref(m)));
                out.push(diag);
            }
            ExprKind::Call {
                name,
                raw_name,
                args,
            } => {
                if schema.has_function(raw_name) {
                    return;
                }
                if let Some(f) = catalog::function(name)
                    && !f.accepts(args.len())
                {
                    out.push(argument_count(f, raw_name, e, args));
                }
                if is_supported_function(name) {
                    return;
                }
                let start = e.span.start;
                let span = Span::new(start, start + raw_name.len());
                let functions = supported_functions();
                let suggestion = closest_name(name, functions.iter().copied())
                    .map(|f| format!("did you mean `{f}`?"));
                out.push(Diagnostic {
                    code: DiagnosticCode::UnsupportedFunction,
                    severity: Severity::Warning,
                    message: format!(
                        "DashCraft cannot evaluate `{raw_name}`: an unknown function, or one not supported yet"
                    ),
                    span,
                    suggestion,
                });
            }
            _ => {}
        }
    });
}

fn argument_count(
    f: &catalog::FunctionInfo,
    raw_name: &str,
    call: &Expr,
    args: &[Expr],
) -> Diagnostic {
    let plural = |n: usize| if n == 1 { "argument" } else { "arguments" };
    let expected = match f.max_args {
        Some(max) if max == f.min_args => format!("{max} {}", plural(max)),
        Some(max) => format!("{} to {max} arguments", f.min_args),
        None => format!("at least {} {}", f.min_args, plural(f.min_args)),
    };
    let given = args.len();
    let verb = if given == 1 { "was" } else { "were" };
    let name_span = Span::new(call.span.start, call.span.start + raw_name.len());
    // Too many: underline the extra arguments; too few: the function name.
    let span = match (f.max_args, args.last()) {
        (Some(max), Some(last)) if given > max => args.get(max).map_or(name_span, |extra| {
            Span::new(extra.span.start, last.span.end.max(extra.span.start))
        }),
        _ => name_span,
    };
    Diagnostic::error(
        DiagnosticCode::ArgumentCount,
        format!(
            "`{raw_name}` takes {expected} but {given} {verb} given: {}",
            f.signature
        ),
        span,
    )
}

fn suggest_table(schema: &Schema, name: &str) -> Option<String> {
    closest_name(name, schema.tables().iter().map(|t| t.name()))
        .map(|t| format!("did you mean table `{}`?", dax_table_ref(t)))
}

/// The table and column parts of a `Table[Column]` span.
fn split_column_ref(span: Span, src: &str) -> (Span, Span) {
    let text = span.text(src);
    let table_end = if text.starts_with('\'') {
        // Closing quote of `'…'`, where `''` is an escaped quote.
        let bytes = text.as_bytes();
        let mut i = 1;
        loop {
            match bytes.get(i) {
                Some(b'\'') if bytes.get(i + 1) == Some(&b'\'') => i += 2,
                Some(b'\'') => break i + 1,
                Some(_) => i += 1,
                None => break text.len(),
            }
        }
    } else {
        text.find('[').map_or(text.len(), |i| {
            text.get(..i).unwrap_or_default().trim_end().len()
        })
    };
    let column_start = text
        .get(table_end..)
        .and_then(|rest| rest.find('['))
        .map_or(text.len(), |i| table_end + i);
    (
        Span::new(span.start, span.start + table_end),
        Span::new(span.start + column_start, span.end),
    )
}
