//! Diagnostics of M text: what an editor underlines, without evaluating anything.
//!
//! [`diagnostics`] parses a query (an expression or section document) and reports:
//!
//! * [`DiagnosticCode::Syntax`]: the document does not parse;
//! * [`DiagnosticCode::UnknownName`]: a name that is neither bound in the text, in the
//!   [`Environment`] nor in the standard library. Power BI Desktop rejects such a query when it
//!   compiles it, wherever the name is (an unused step, inside `try`), with the message used here;
//! * [`DiagnosticCode::ArgumentCount`]: a library function called with too few or too many
//!   arguments, with Desktop's message (the call fails whenever it is evaluated);
//! * [`DiagnosticCode::Unsupported`] (warning): a library name DashCraft does not evaluate
//!   (data access such as `Sql.Database`), so the query will not refresh in DashCraft;
//! * [`DiagnosticCode::ExtensionName`] (warning): a dotted name outside the library's namespaces,
//!   such as a custom connector's function, which DashCraft cannot evaluate.
//!
//! The messages and the binding rules were checked against Desktop (`reference/groundtruth`,
//! suites `m-functions` and `m-arity`).

use super::environment::Environment;
use super::library::{LibraryKind, catalog, catalog_entry};
use crate::ast::Document;
use crate::binding;
use crate::merror::MError;
use crate::refs::{FreeNames, free_names, section_free_names};
use crate::span::Span;

/// At most this many diagnostics are reported.
pub const MAX_DIAGNOSTICS: usize = 1000;

/// Suggestions are computed for this many unknown names at most (each compares the name with
/// the whole library).
const MAX_SUGGESTIONS: usize = 50;

/// How bad a diagnostic is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The query fails in Power BI Desktop.
    Error,
    /// The query is valid M, but DashCraft cannot evaluate it.
    Warning,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

/// What a diagnostic is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticCode {
    Syntax,
    UnknownName,
    ArgumentCount,
    Unsupported,
    ExtensionName,
}

impl DiagnosticCode {
    pub fn as_str(self) -> &'static str {
        match self {
            DiagnosticCode::Syntax => "syntax",
            DiagnosticCode::UnknownName => "unknownName",
            DiagnosticCode::ArgumentCount => "argumentCount",
            DiagnosticCode::Unsupported => "unsupported",
            DiagnosticCode::ExtensionName => "extensionName",
        }
    }
}

/// One problem in M text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: DiagnosticCode,
    pub message: String,
    /// Byte range in the text.
    pub span: Span,
    /// A replacement for `span`: the closest known name for an unknown name.
    pub suggestion: Option<String>,
}

/// The diagnostics of `text` (an expression or section document), sorted by position.
pub fn diagnostics(text: &str, env: &Environment) -> Vec<Diagnostic> {
    let doc = match crate::parse_document(text) {
        Ok(doc) => doc,
        Err(e) => {
            return vec![Diagnostic {
                severity: Severity::Error,
                code: DiagnosticCode::Syntax,
                message: e.message,
                span: e.span,
                suggestion: None,
            }];
        }
    };
    let found = match &doc {
        Document::Expression(e) => free_names(e),
        Document::Section(s) => section_free_names(s),
    };
    let mut out = names(&found, env);
    out.extend(calls(&found, env));
    out.sort_by_key(|d| (d.span.start, d.span.end));
    out.truncate(MAX_DIAGNOSTICS);
    out
}

fn names(found: &FreeNames<'_>, env: &Environment) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut suggested = 0;
    for name in &found.free {
        let text = name.text.as_str();
        if env.contains(text) {
            continue;
        }
        if binding::in_library(text) {
            if crate::stdlib::library().get(text).is_none() {
                out.push(Diagnostic {
                    severity: Severity::Warning,
                    code: DiagnosticCode::Unsupported,
                    message: unsupported_message(text),
                    span: name.span,
                    suggestion: None,
                });
            }
            continue;
        }
        if binding::is_extension_name(text) {
            if env.is_known() {
                out.push(Diagnostic {
                    severity: Severity::Warning,
                    code: DiagnosticCode::ExtensionName,
                    message: format!(
                        "'{text}' is not in the standard library (a custom connector?): DashCraft cannot evaluate it."
                    ),
                    span: name.span,
                    suggestion: None,
                });
            }
            continue;
        }
        if !env.is_known() && !text.contains('.') {
            continue;
        }
        let suggestion = (suggested < MAX_SUGGESTIONS)
            .then(|| {
                suggested += 1;
                suggest(text, env, &found.locals)
            })
            .flatten();
        out.push(Diagnostic {
            severity: Severity::Error,
            code: DiagnosticCode::UnknownName,
            message: binding::unrecognized_message(text),
            span: name.span,
            suggestion,
        });
    }
    out
}

pub(crate) fn unsupported_message(name: &str) -> String {
    match crate::stdlib::unknown_name(name) {
        MError::Unsupported { hint, .. } if !hint.is_empty() => {
            format!("DashCraft does not evaluate '{name}'. {hint}")
        }
        _ => format!("DashCraft does not evaluate '{name}'."),
    }
}

fn calls(found: &FreeNames<'_>, env: &Environment) -> Vec<Diagnostic> {
    found
        .calls
        .iter()
        .filter(|c| !env.contains(&c.name.text))
        .filter_map(|c| {
            let LibraryKind::Function(f) = &catalog_entry(&c.name.text)?.kind else {
                return None;
            };
            let (min, max) = (f.required, f.params.len());
            if (min..=max).contains(&c.args) {
                return None;
            }
            Some(Diagnostic {
                severity: Severity::Error,
                code: DiagnosticCode::ArgumentCount,
                message: argument_count_message(c.args, min, max),
                span: c.span,
                suggestion: None,
            })
        })
        .collect()
}

/// Desktop's message for a call with `passed` arguments to a function taking `min..=max`.
pub fn argument_count_message(passed: usize, min: usize, max: usize) -> String {
    let expects = if min == max {
        min.to_string()
    } else {
        format!("between {min} and {max}")
    };
    format!("{passed} arguments were passed to a function which expects {expects}.")
}

/// The closest known name: the environment, the library, then the text's own bindings.
fn suggest(name: &str, env: &Environment, locals: &[&str]) -> Option<String> {
    let library = catalog().iter().map(|e| e.name);
    let model = env.entries().iter().map(|e| e.name.as_str());
    let implemented = crate::stdlib::library_names();
    closest(
        name,
        model
            .chain(library)
            .chain(implemented)
            .chain(locals.iter().copied()),
    )
    .map(str::to_owned)
}

/// Names longer than this are not compared (bounds the work per pair).
const MAX_LEN: usize = 128;

/// The most plausible correction for `name` among `candidates`: the smallest case-insensitive
/// edit distance within about a quarter of the length (only a different case for a single
/// letter), or a word prefix or suffix (`Sales` / `Sales Orders`); ties go to the first
/// candidate in order.
fn closest<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let lower: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();
    if lower.is_empty() || lower.len() > MAX_LEN {
        return None;
    }
    let mut best: Option<(usize, &'a str)> = None;
    for c in candidates.filter(|c| *c != name) {
        let other: Vec<char> = c.chars().flat_map(char::to_lowercase).collect();
        if other.len() > MAX_LEN {
            continue;
        }
        let threshold = (lower.len().max(other.len()) + 2) / 4;
        let score = if lower.len().abs_diff(other.len()) <= threshold {
            Some(distance(&lower, &other)).filter(|d| *d <= threshold)
        } else {
            None
        }
        .or_else(|| {
            (word_affix(&lower, &other) || word_affix(&other, &lower)).then_some(threshold)
        });
        if let Some(score) = score
            && best.is_none_or(|(b, _)| score < b)
        {
            best = Some((score, c));
        }
    }
    best.map(|(_, c)| c)
}

/// Levenshtein distance over chars.
fn distance(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = Vec::with_capacity(b.len() + 1);
        cur.push(i + 1);
        for (j, cb) in b.iter().enumerate() {
            let replace = prev.get(j).copied().unwrap_or(usize::MAX - 1) + usize::from(ca != cb);
            let delete = prev.get(j + 1).copied().unwrap_or(usize::MAX - 1) + 1;
            let insert = cur.last().copied().unwrap_or(usize::MAX - 1) + 1;
            cur.push(replace.min(delete).min(insert));
        }
        prev = cur;
    }
    prev.last().copied().unwrap_or(0)
}

/// `long` starts or ends with `short` at a word boundary (`.`, space, `_`).
fn word_affix(short: &[char], long: &[char]) -> bool {
    if short.len() < 4 || short.len() >= long.len() {
        return false;
    }
    let boundary = |c: Option<&char>| c.is_none_or(|c| !c.is_alphanumeric());
    (long.starts_with(short) && boundary(long.get(short.len())))
        || (long.ends_with(short)
            && boundary(
                long.len()
                    .checked_sub(short.len() + 1)
                    .and_then(|i| long.get(i)),
            ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(text: &str, env: &Environment) -> Vec<(&'static str, String, Option<String>)> {
        diagnostics(text, env)
            .into_iter()
            .map(|d| {
                let snippet = text.get(d.span.start..d.span.end).unwrap_or("").to_owned();
                (d.code.as_str(), snippet, d.suggestion)
            })
            .collect()
    }

    #[test]
    fn syntax_errors_are_reported_alone() {
        let d = diagnostics("let a = in a", &Environment::unknown());
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, DiagnosticCode::Syntax);
        assert_eq!(d[0].severity, Severity::Error);
    }

    #[test]
    fn unknown_names_follow_the_environment() {
        let src = "let Source = Orders, n = Table.RowCnt(Source), t = text.upper(\"a\") in Rate";
        let model = Environment::with_names(["Orders", "Rates"]);
        assert_eq!(
            codes(src, &model),
            [
                (
                    "unknownName",
                    "Table.RowCnt".into(),
                    Some("Table.RowCount".into())
                ),
                (
                    "unknownName",
                    "text.upper".into(),
                    Some("Text.Upper".into())
                ),
                ("unknownName", "Rate".into(), Some("Rates".into())),
            ]
        );
        // Without a model, plain names may be other queries.
        assert_eq!(
            codes(src, &Environment::unknown())
                .into_iter()
                .map(|(c, s, _)| (c, s))
                .collect::<Vec<_>>(),
            [
                ("unknownName", "Table.RowCnt".to_owned()),
                ("unknownName", "text.upper".to_owned()),
            ]
        );
        let d = diagnostics("Missing", &Environment::with_names(Vec::<String>::new()));
        assert_eq!(
            d[0].message,
            "The name 'Missing' wasn't recognized.  Make sure it's spelled correctly."
        );
        // Exclusive scoping: a step does not see itself.
        assert_eq!(
            codes(
                "let x = x in x",
                &Environment::with_names(Vec::<String>::new())
            ),
            [("unknownName", "x".into(), None)]
        );
        assert!(
            codes(
                "let x = @x in x",
                &Environment::with_names(Vec::<String>::new())
            )
            .is_empty()
        );
    }

    #[test]
    fn argument_counts_of_library_functions() {
        let env = Environment::with_names(Vec::<String>::new());
        let d = diagnostics("Text.Upper() & Text.Upper(\"a\", \"en-US\", 3)", &env);
        let got: Vec<_> = d.iter().map(|d| (d.code, d.message.as_str())).collect();
        assert_eq!(
            got,
            [
                (
                    DiagnosticCode::ArgumentCount,
                    "0 arguments were passed to a function which expects between 1 and 2."
                ),
                (
                    DiagnosticCode::ArgumentCount,
                    "3 arguments were passed to a function which expects between 1 and 2."
                ),
            ]
        );
        assert_eq!(d[0].span, Span::new(0, 12));
        // A step or parameter shadowing a library name is not the library function.
        assert!(diagnostics("let Text.Upper = (x) => x in Text.Upper(1, 2)", &env).is_empty());
        assert!(diagnostics("((Text.Upper) => Text.Upper())(1)", &env).is_empty());
        assert!(diagnostics("Text.Upper(1)", &Environment::with_names(["Text.Upper"])).is_empty());
    }

    #[test]
    fn unsupported_and_extension_names_are_warnings() {
        let env = Environment::with_names(["Url"]);
        let d = diagnostics(
            "Table.Combine({Web.Contents(Url), MyConnector.Contents()})",
            &env,
        );
        let got: Vec<_> = d.iter().map(|d| (d.code, d.severity)).collect();
        assert_eq!(
            got,
            [
                (DiagnosticCode::Unsupported, Severity::Warning),
                (DiagnosticCode::ExtensionName, Severity::Warning),
            ]
        );
        assert!(
            d[0].message
                .starts_with("DashCraft does not evaluate 'Web.Contents'.")
        );
    }

    #[test]
    fn sections_bind_their_members() {
        let env = Environment::with_names(Vec::<String>::new());
        assert_eq!(
            codes("section S; A = B; shared B = 1; C = D;", &env),
            [("unknownName", "D".into(), None)]
        );
    }

    #[test]
    fn closest_names() {
        let names = ["Sales Orders", "Customers", "Text.Upper"];
        assert_eq!(closest("Customer", names.into_iter()), Some("Customers"));
        assert_eq!(closest("Sales", names.into_iter()), Some("Sales Orders"));
        assert_eq!(closest("TEXT.UPPER", names.into_iter()), Some("Text.Upper"));
        assert_eq!(closest("Zebra", names.into_iter()), None);
        assert_eq!(closest("d", ["a", "D"].into_iter()), Some("D"));
        assert_eq!(distance(&['a', 'b'], &['b']), 1);
    }
}
