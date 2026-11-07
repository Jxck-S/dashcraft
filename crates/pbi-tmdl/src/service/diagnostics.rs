//! Diagnostics over a set of TMDL files: what an editor underlines before the model is saved.
//!
//! [`diagnostics`] parses every [`Source`] and checks it against the keyword [`catalog`]
//! recorded from Microsoft's TMDL reader (the TOM library's `TmdlSerializer`). It reports what
//! that reader rejects:
//!
//! | Code | Severity | Meaning |
//! |---|---|---|
//! | `TMDL101` | error | unknown keyword (with the closest keyword of the context) |
//! | `TMDL102` | error | a keyword of another object type (`level` directly in a table) |
//! | `TMDL103` | error | a keyword in a form it does not take (`isHidden: abc` is fine, `isHidden abc` is not) |
//! | `TMDL104` | error | a keyword the database's `compatibilityLevel` does not support yet |
//! | `TMDL105` | error | a value of the wrong type (`isKey: yes`, `dataType: strin`) |
//! | `TMDL106` | error | the same property twice in one object |
//! | `TMDL107` | error | two declarations of one object that cannot be merged (both set a property, or both have a `=` value) |
//! | `TMDL108` | error | a reference to an object that does not exist (`sortByColumn`, `fromColumn`, a role's `tablePermission`) |
//! | `TMDL109` | error | a property after a child object (the reader takes properties first) |
//! | `TMDL110` | error | a syntax error (indentation, unterminated quote) |
//! | `TMDL111` | error | lines indented below a property |
//! | `TMDL112` | error | a top-level `ref` line that is not `ref <type> <name>` |
//! | `TMDL113` | error | a second `model` or `database` with another name |
//! | `TMDL120` | warning | a top-level `ref` to an object that is not declared (the reader ignores it) |
//!
//! It also passes on the parser's warnings about orphan `///` descriptions (`TMDL006`) and
//! mixed indentation (`TMDL007`). References are only resolved when every file parsed and a
//! `model` is declared (a whole model folder); compatibility levels are only checked when a
//! `database` declares its `compatibilityLevel`. DAX and M expressions are not checked (the
//! TMDL reader does not check them either).
//!
//! The rules were checked against the TMDL reader (`reference/groundtruth`, suites
//! `tmdl-catalog` and `tmdl-validity`; the differences are listed in
//! `reference/groundtruth/deviations.json`).

use std::collections::{HashMap, HashSet};

use pbi_model::closest_name;
use pbi_model::names::{name_key, parse_tmdl_column_ref, tmdl_name, tmdl_unquote};

use super::catalog::{Catalog, Context, Form, Keyword, Support, ValueType, catalog};
use crate::cst::{Node, TmdlFile, codes as file_codes, parse_file};
use crate::error::TmdlError;
use crate::lexer::{Line, Operator, Span};

/// At most this many diagnostics are reported.
pub const MAX_DIAGNOSTICS: usize = 1000;

/// Diagnostic codes of the language service.
pub mod codes {
    pub const UNKNOWN_KEYWORD: &str = "TMDL101";
    pub const WRONG_CONTEXT: &str = "TMDL102";
    pub const WRONG_FORM: &str = "TMDL103";
    pub const COMPATIBILITY_LEVEL: &str = "TMDL104";
    pub const INVALID_VALUE: &str = "TMDL105";
    pub const DUPLICATE_PROPERTY: &str = "TMDL106";
    pub const MERGE_CONFLICT: &str = "TMDL107";
    pub const UNRESOLVED_REFERENCE: &str = "TMDL108";
    pub const PROPERTY_AFTER_OBJECT: &str = "TMDL109";
    pub const SYNTAX: &str = "TMDL110";
    pub const PROPERTY_WITH_CHILDREN: &str = "TMDL111";
    pub const INVALID_REF: &str = "TMDL112";
    pub const SECOND_MODEL: &str = "TMDL113";
    pub const MISSING_REF_TARGET: &str = "TMDL120";
}

/// How bad a diagnostic is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The TMDL reader rejects the files.
    Error,
    /// The files load, but probably not as intended.
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

/// One TMDL file to check.
#[derive(Debug)]
pub struct Source {
    pub path: String,
    pub text: String,
    parsed: Result<TmdlFile, TmdlError>,
}

impl Source {
    /// Parses `text` (`path` is only used in diagnostics).
    pub fn new(path: impl Into<String>, text: impl Into<String>) -> Self {
        let path = path.into();
        let text = text.into();
        let parsed = parse_file(&path, &text);
        Source { path, text, parsed }
    }

    /// The parsed file, if it parsed.
    pub fn file(&self) -> Option<&TmdlFile> {
        self.parsed.as_ref().ok()
    }
}

/// One problem in a TMDL file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmdlDiagnostic {
    /// The index of the [`Source`] in the input.
    pub file: usize,
    /// The [`Source::path`].
    pub path: String,
    pub severity: Severity,
    /// One of [`codes`] (or the parser's `TMDL006` / `TMDL007`).
    pub code: &'static str,
    pub message: String,
    /// Byte range in the file's text.
    pub start: usize,
    pub end: usize,
    /// 1-based line of `start`.
    pub line: u32,
    /// 1-based column (in characters) of `start`.
    pub column: u32,
    /// A replacement for the range: the closest keyword, value or name.
    pub suggestion: Option<String>,
}

/// The form of a structural line (`ref` lines below the top level are `ref` objects).
pub fn node_form(node: &Node) -> Form {
    match (node.is_ref() || node.name_raw().is_some(), node.operator()) {
        (true, Some(Operator::Equals)) => Form::NamedEquals,
        (true, _) => Form::Named,
        (false, Some(Operator::Colon)) => Form::Colon,
        (false, Some(Operator::Equals)) => Form::Equals,
        (false, None) => Form::Bare,
    }
}

/// The keyword a line is looked up with (`ref` for `ref <type> <name>`).
fn node_keyword(node: &Node) -> &str {
    if node.is_ref() { "ref" } else { node.keyword() }
}

/// Objects whose content is not checked: a culture's `translations` block, which TOM writes
/// for translated captions. The catalog has no record of it (the reader fails on an empty
/// block, so it cannot be probed keyword by keyword).
const OPAQUE: [(&str, &str); 1] = [("cultureInfo", "translations")];

fn is_opaque(ctx: &Context, node: &Node, word: &str) -> bool {
    node_form(node) == Form::Bare
        && OPAQUE
            .iter()
            .any(|(c, k)| ctx.name == *c && word.eq_ignore_ascii_case(k))
}

/// The diagnostics of a set of TMDL files, sorted by file (in input order) and position.
pub fn diagnostics(sources: &[Source]) -> Vec<TmdlDiagnostic> {
    let files: Vec<(usize, &TmdlFile)> = sources
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.file().map(|f| (i, f)))
        .collect();
    let index = Index::build(files.iter().map(|(_, f)| *f));
    let resolve = files.len() == sources.len() && index.has_model;
    let mut checker = Checker {
        catalog: catalog(),
        level: compatibility_level(files.iter().map(|(_, f)| *f)),
        index: &index,
        resolve,
        out: Vec::new(),
    };
    for (i, source) in sources.iter().enumerate() {
        match &source.parsed {
            Err(e) => checker.parse_error(i, source, e),
            Ok(file) => {
                checker.file_warnings(i, source, file);
                checker.check_file(i, file);
            }
        }
    }
    checker.check_merges(&files);
    let mut out = checker.out;
    out.sort_by_key(|(file, d)| (*file, d.start, d.end));
    out.dedup_by(|a, b| a.0 == b.0 && a.1.start == b.1.start && a.1.code == b.1.code);
    out.truncate(MAX_DIAGNOSTICS);
    out.into_iter()
        .map(|(file, d)| TmdlDiagnostic { file, ..d })
        .collect()
}

/// The `compatibilityLevel` of the top-level `database`.
fn compatibility_level<'a>(files: impl Iterator<Item = &'a TmdlFile>) -> Option<u32> {
    for file in files {
        for node in file.find("database") {
            if let Some(level) = node.child("compatibilityLevel") {
                let value = match level.operator() {
                    Some(Operator::Colon) => level.value_raw().map(str::to_owned),
                    Some(Operator::Equals) => level.expression(),
                    None => None,
                };
                return value.and_then(|v| v.trim().parse().ok());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------------------------
// Names declared in the files

/// Case-folded name → name as declared.
type Names = HashMap<String, String>;

#[derive(Debug, Default)]
struct TableInfo {
    columns: Names,
    measures: Names,
    hierarchies: Names,
    /// Has a `partition … = calculated`: its columns are calculated table columns.
    calculated: bool,
}

#[derive(Debug, Default)]
struct Index {
    has_model: bool,
    tables: HashMap<String, (String, TableInfo)>,
    /// Other top-level objects by kind (`queryGroup`, `relationship`, `role`, …; cultures under
    /// `culture`).
    objects: HashMap<&'static str, Names>,
}

/// Object kinds a top-level `ref` line may name, with the kind they are indexed under.
const ROOT_REF_KINDS: [(&str, &str); 10] = [
    ("table", "table"),
    ("role", "role"),
    ("culture", "culture"),
    ("cultureInfo", "culture"),
    ("perspective", "perspective"),
    ("expression", "expression"),
    ("queryGroup", "queryGroup"),
    ("dataSource", "dataSource"),
    ("relationship", "relationship"),
    ("function", "function"),
];

fn ref_kind(keyword: &str) -> Option<&'static str> {
    ROOT_REF_KINDS
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(keyword))
        .map(|(_, kind)| *kind)
}

impl Index {
    fn build<'a>(files: impl Iterator<Item = &'a TmdlFile>) -> Self {
        let mut index = Index::default();
        for file in files {
            for node in &file.nodes {
                index.add(node, true);
            }
        }
        index
    }

    fn add(&mut self, node: &Node, top: bool) {
        if node.is_ref() {
            return;
        }
        let keyword = node.keyword();
        if keyword.eq_ignore_ascii_case("model") {
            self.has_model = true;
            if top {
                for child in &node.children {
                    self.add(child, false);
                }
            }
            return;
        }
        let Some(name) = node.name() else {
            return;
        };
        if keyword.eq_ignore_ascii_case("table") {
            let entry = self
                .tables
                .entry(name_key(&name))
                .or_insert_with(|| (name.clone(), TableInfo::default()));
            let table = &mut entry.1;
            for child in &node.children {
                let Some(child_name) = child.name() else {
                    continue;
                };
                let k = child.keyword();
                let set = if k.eq_ignore_ascii_case("column") {
                    &mut table.columns
                } else if k.eq_ignore_ascii_case("measure") {
                    &mut table.measures
                } else if k.eq_ignore_ascii_case("hierarchy") {
                    &mut table.hierarchies
                } else {
                    if k.eq_ignore_ascii_case("partition")
                        && child
                            .expression()
                            .is_some_and(|v| v.trim().eq_ignore_ascii_case("calculated"))
                    {
                        table.calculated = true;
                    }
                    continue;
                };
                set.entry(name_key(&child_name)).or_insert(child_name);
            }
        } else if let Some(kind) = ref_kind(keyword) {
            self.objects
                .entry(kind)
                .or_default()
                .entry(name_key(&name))
                .or_insert(name);
        }
    }

    fn table(&self, name: &str) -> Option<&TableInfo> {
        self.tables.get(&name_key(name)).map(|(_, t)| t)
    }

    fn table_names(&self) -> impl Iterator<Item = &str> {
        self.tables.values().map(|(n, _)| n.as_str())
    }

    fn has_object(&self, kind: &str, name: &str) -> bool {
        if kind == "table" {
            return self.table(name).is_some();
        }
        self.objects
            .get(kind)
            .is_some_and(|names| names.contains_key(&name_key(name)))
    }

    fn object_names(&self, kind: &str) -> Vec<&str> {
        if kind == "table" {
            return self.table_names().collect();
        }
        self.objects
            .get(kind)
            .map(|names| names.values().map(String::as_str).collect())
            .unwrap_or_default()
    }

    fn is_calculated(&self, table: &str) -> bool {
        self.table(table).is_some_and(|t| t.calculated)
    }
}

// ---------------------------------------------------------------------------------------------
// Checks

struct Checker<'a> {
    catalog: &'static Catalog,
    level: Option<u32>,
    index: &'a Index,
    resolve: bool,
    out: Vec<(usize, TmdlDiagnostic)>,
}

/// Where a diagnostic goes.
struct At<'a> {
    file: usize,
    path: &'a str,
    line: &'a Line,
    span: Span,
}

impl<'a> At<'a> {
    fn keyword(file: usize, path: &'a str, node: &'a Node) -> Self {
        At {
            file,
            path,
            line: &node.line,
            span: node.header.keyword,
        }
    }

    fn name(file: usize, path: &'a str, node: &'a Node) -> Self {
        At {
            file,
            path,
            line: &node.line,
            span: node.header.name.unwrap_or(node.header.keyword),
        }
    }

    /// The value, or the keyword when the value is empty.
    fn value(file: usize, path: &'a str, node: &'a Node) -> Self {
        let span = node
            .header
            .value
            .filter(|s| s.end > s.start)
            .unwrap_or(node.header.keyword);
        At {
            file,
            path,
            line: &node.line,
            span,
        }
    }

    /// The whole line content.
    fn line(file: usize, path: &'a str, node: &'a Node) -> Self {
        let start = node.header.keyword.start.min(node.line.text.len());
        let trimmed = node.line.text.trim_end().len().max(start);
        At {
            file,
            path,
            line: &node.line,
            span: Span {
                start,
                end: trimmed,
            },
        }
    }
}

/// `a`, `b` or `c`.
fn or_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} or {last}", init.join(", ")),
    }
}

impl Checker<'_> {
    fn push(
        &mut self,
        at: At<'_>,
        severity: Severity,
        code: &'static str,
        message: String,
        suggestion: Option<String>,
    ) {
        let start = at.line.offset.saturating_add(at.span.start);
        let end = at
            .line
            .offset
            .saturating_add(at.span.end.max(at.span.start));
        self.out.push((
            at.file,
            TmdlDiagnostic {
                file: 0,
                path: at.path.to_owned(),
                severity,
                code,
                message,
                start,
                end,
                line: at.line.number,
                column: at.line.column_of(at.span.start),
                suggestion,
            },
        ));
    }

    fn error(&mut self, at: At<'_>, code: &'static str, message: String) {
        self.push(at, Severity::Error, code, message, None);
    }

    fn parse_error(&mut self, file: usize, source: &Source, error: &TmdlError) {
        let (line, column, message) = match error {
            TmdlError::Syntax {
                line, column, kind, ..
            } => (*line, *column, kind.to_string()),
            other => (1, 1, other.to_string()),
        };
        let start = byte_offset(&source.text, line, column);
        let end = source
            .text
            .get(start..)
            .and_then(|rest| rest.find(['\r', '\n']))
            .map_or(source.text.len(), |i| start + i);
        self.out.push((
            file,
            TmdlDiagnostic {
                file: 0,
                path: source.path.clone(),
                severity: Severity::Error,
                code: codes::SYNTAX,
                message,
                start,
                end,
                line,
                column,
                suggestion: None,
            },
        ));
    }

    fn file_warnings(&mut self, file: usize, source: &Source, parsed: &TmdlFile) {
        for d in &parsed.diagnostics {
            let code: &'static str = match d.code.as_str() {
                c if c == file_codes::ORPHAN_DESCRIPTION => file_codes::ORPHAN_DESCRIPTION,
                c if c == file_codes::MIXED_INDENTATION => file_codes::MIXED_INDENTATION,
                _ => continue,
            };
            let line = d.origin.as_ref().and_then(|o| o.line).unwrap_or(1);
            let start = byte_offset(&source.text, line, 1);
            self.out.push((
                file,
                TmdlDiagnostic {
                    file: 0,
                    path: source.path.clone(),
                    severity: Severity::Warning,
                    code,
                    message: d.message.clone(),
                    start,
                    end: start,
                    line,
                    column: 1,
                    suggestion: None,
                },
            ));
        }
    }

    fn check_file(&mut self, file: usize, parsed: &TmdlFile) {
        if let Some(root) = self.catalog.context("root") {
            self.check_children(file, &parsed.path, root, &parsed.nodes, None);
        }
    }

    fn check_children(
        &mut self,
        file: usize,
        path: &str,
        ctx: &'static Context,
        nodes: &[Node],
        table: Option<&str>,
    ) {
        let root = ctx.name == "root";
        let mut seen_object = false;
        let mut properties: HashSet<String> = HashSet::new();
        for node in nodes {
            if root && (node.is_ref() || node.keyword().eq_ignore_ascii_case("ref")) {
                self.check_root_ref(file, path, node);
                continue;
            }
            let word = node_keyword(node);
            if is_opaque(ctx, node, word) {
                self.check_translations(file, path, node);
                continue;
            }
            let Some(keyword) = ctx.keyword(word) else {
                self.unknown_keyword(file, path, ctx, node, word);
                continue;
            };
            let form = node_form(node);
            let support = keyword.support(form);
            if !support.accepted() {
                self.wrong_form(file, path, node, keyword, form);
                continue;
            }
            self.check_level(file, path, node, keyword, support);
            if keyword.is_object(form) {
                seen_object = true;
                self.check_value(file, path, node, nodes, ctx, keyword, form, table);
                self.check_named_reference(file, path, ctx, node, table);
                let own_table = self.scope_table(keyword.keyword, node);
                let table = own_table.as_deref().or(table);
                if let Some(child) = self.child_context(keyword.keyword, node, table) {
                    self.check_children(file, path, child, &node.children, table);
                }
            } else {
                if seen_object {
                    self.error(
                        At::keyword(file, path, node),
                        codes::PROPERTY_AFTER_OBJECT,
                        format!(
                            "property `{word}` comes after a child object; TMDL lists an object's \
                             properties before its child objects"
                        ),
                    );
                }
                if !properties.insert(word.to_ascii_lowercase()) {
                    self.error(
                        At::keyword(file, path, node),
                        codes::DUPLICATE_PROPERTY,
                        format!("property `{word}` is set more than once in this object"),
                    );
                }
                if let Some(child) = node.children.first() {
                    self.error(
                        At::line(file, path, child),
                        codes::PROPERTY_WITH_CHILDREN,
                        format!(
                            "this line is indented below `{word}`, which is a property and cannot \
                             have child lines"
                        ),
                    );
                }
                self.check_value(file, path, node, nodes, ctx, keyword, form, table);
            }
        }
    }

    /// The table that names inside `node` refer to (a table, a role's table permission, a
    /// perspective's table).
    fn scope_table(&self, keyword: &str, node: &Node) -> Option<String> {
        ["table", "tablePermission", "perspectiveTable"]
            .iter()
            .any(|k| k.eq_ignore_ascii_case(keyword))
            .then(|| node.name())
            .flatten()
    }

    /// The context of `node`'s child lines.
    fn child_context(
        &self,
        keyword: &str,
        node: &Node,
        table: Option<&str>,
    ) -> Option<&'static Context> {
        let name = if keyword.eq_ignore_ascii_case("column") {
            if node.operator() == Some(Operator::Equals) {
                "calculatedColumn"
            } else if table.is_some_and(|t| self.index.is_calculated(t)) {
                "calculatedTableColumn"
            } else {
                "column"
            }
        } else if keyword.eq_ignore_ascii_case("partition") {
            match node.expression().as_deref().map(str::trim) {
                Some(v) if v.eq_ignore_ascii_case("calculated") => "calculatedPartition",
                Some(v) if v.eq_ignore_ascii_case("calculationGroup") => {
                    "calculationGroupPartition"
                }
                _ => "partition",
            }
        } else {
            keyword
        };
        self.catalog.context(name)
    }

    fn unknown_keyword(&mut self, file: usize, path: &str, ctx: &Context, node: &Node, word: &str) {
        let place = context_label(ctx);
        let elsewhere: Vec<String> = self
            .catalog
            .contexts_of(word)
            .map(|c| format!("`{}`", context_label(c)))
            .collect();
        if !elsewhere.is_empty() {
            let shown: Vec<String> = elsewhere.iter().take(4).cloned().collect();
            let more = if elsewhere.len() > shown.len() {
                format!(" (and {} more)", elsewhere.len() - shown.len())
            } else {
                String::new()
            };
            self.error(
                At::keyword(file, path, node),
                codes::WRONG_CONTEXT,
                format!(
                    "`{word}` is not valid in {place}; it belongs in {}{more}",
                    or_list(&shown)
                ),
            );
            return;
        }
        if word.eq_ignore_ascii_case("description") {
            self.error(
                At::keyword(file, path, node),
                codes::UNKNOWN_KEYWORD,
                "TMDL has no `description` property: write the description as `///` lines \
                 directly above the object"
                    .to_owned(),
            );
            return;
        }
        let suggestion =
            closest_name(word, ctx.keywords.iter().map(|k| k.keyword)).map(str::to_owned);
        let hint = suggestion
            .as_deref()
            .map(|s| format!("; did you mean `{s}`?"))
            .unwrap_or_default();
        self.push(
            At::keyword(file, path, node),
            Severity::Error,
            codes::UNKNOWN_KEYWORD,
            format!("unknown keyword `{word}` in {place}{hint}"),
            suggestion,
        );
    }

    fn wrong_form(&mut self, file: usize, path: &str, node: &Node, keyword: &Keyword, form: Form) {
        let word = node_keyword(node);
        let forms: Vec<String> = keyword
            .forms()
            .map(|f| format!("`{}`", f.pattern(keyword.keyword)))
            .collect();
        let message = if forms.is_empty() {
            format!("`{word}` cannot be used here")
        } else {
            format!(
                "`{word}` cannot be written as `{}`; write {}",
                form.pattern(word),
                or_list(&forms)
            )
        };
        self.error(At::line(file, path, node), codes::WRONG_FORM, message);
    }

    fn check_level(
        &mut self,
        file: usize,
        path: &str,
        node: &Node,
        keyword: &Keyword,
        support: Support,
    ) {
        let Some(level) = self.level else {
            return;
        };
        if support.accepted_at(level) {
            return;
        }
        let needed = match support {
            Support::From(n) => format!("compatibility level {n} or higher"),
            Support::Above(n) => format!("a compatibility level above {n}"),
            Support::Yes | Support::No => return,
        };
        self.error(
            At::keyword(file, path, node),
            codes::COMPATIBILITY_LEVEL,
            format!(
                "`{}` needs {needed}; the database's compatibilityLevel is {level}",
                keyword.keyword
            ),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn check_value(
        &mut self,
        file: usize,
        path: &str,
        node: &Node,
        siblings: &[Node],
        ctx: &Context,
        keyword: &Keyword,
        form: Form,
        table: Option<&str>,
    ) {
        let ty = keyword.value_type(form);
        let value = match form {
            Form::Colon => node.value_raw().map(str::to_owned),
            Form::Equals | Form::NamedEquals => node.expression(),
            Form::Named | Form::Bare => None,
        };
        let Some(value) = value else {
            return;
        };
        let word = node_keyword(node);
        let json_content = if keyword.is_object(form) {
            content_is_json(&node.children)
        } else {
            content_is_json(siblings)
        };
        if let Some((problem, suggestion)) = self.value_problem(ty, &value, json_content) {
            self.push(
                At::value(file, path, node),
                Severity::Error,
                codes::INVALID_VALUE,
                format!("`{word}` {problem}"),
                suggestion,
            );
            return;
        }
        if ty == ValueType::Ref && !keyword.is_object(form) {
            self.check_reference(file, path, ctx, node, keyword.keyword, &value, table);
        }
    }

    /// What is wrong with `value` as a `ty`, with a replacement when there is a close one.
    fn value_problem(
        &self,
        ty: ValueType,
        value: &str,
        json_content: bool,
    ) -> Option<(String, Option<String>)> {
        let v = value.trim();
        let ok = match ty {
            ValueType::None | ValueType::Text | ValueType::Expr => true,
            ValueType::Bool => v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("false"),
            ValueType::Int => v.parse::<i32>().is_ok(),
            ValueType::Long => v.parse::<i64>().is_ok(),
            ValueType::Number => v.parse::<f64>().is_ok_and(f64::is_finite),
            ValueType::Ref => !v.is_empty(),
            ValueType::Json => {
                serde_json::from_str::<serde_json::Value>(v).is_ok_and(|j| j.is_object())
            }
            ValueType::Content if json_content => {
                serde_json::from_str::<serde_json::Value>(v).is_ok()
            }
            ValueType::Content => v.starts_with('<'),
            ValueType::Enum(name) => self.enum_ok(name, v),
        };
        if ok {
            return None;
        }
        let shown = if v.is_empty() {
            "nothing".to_owned()
        } else {
            format!("`{}`", truncate(v, 40))
        };
        let (expected, suggestion) = match ty {
            ValueType::Enum(name) => match self.catalog.enum_values(name) {
                Some(values) => {
                    let list: Vec<String> = values.iter().map(|v| (*v).to_owned()).collect();
                    let suggestion = closest_name(v, values.iter().copied()).map(str::to_owned);
                    (format!("one of {}", list.join(", ")), suggestion)
                }
                None => (ty.describe(), None),
            },
            ValueType::Content if json_content => ("JSON (contentType: json)".to_owned(), None),
            ValueType::Content => ("XML (or set contentType: json)".to_owned(), None),
            _ => (ty.describe(), None),
        };
        Some((format!("takes {expected}, not {shown}"), suggestion))
    }

    /// A value name, names separated by commas, or an integer the enumeration defines.
    fn enum_ok(&self, name: &str, value: &str) -> bool {
        if let Ok(n) = value.parse::<i64>() {
            // The recording probed −1 … 20: an enumeration that took all of them takes any
            // integer; the others only the ones they define.
            return self
                .catalog
                .enum_numbers(name)
                .is_none_or(|ns| ns.contains(&n) || (-1..=20).all(|p| ns.contains(&p)));
        }
        let values = self.catalog.enum_values(name);
        value.split(',').all(|part| {
            let part = part.trim();
            !part.is_empty()
                && values.is_none_or(|vs| vs.iter().any(|v| v.eq_ignore_ascii_case(part)))
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn check_reference(
        &mut self,
        file: usize,
        path: &str,
        ctx: &Context,
        node: &Node,
        keyword: &str,
        value: &str,
        table: Option<&str>,
    ) {
        if !self.resolve {
            return;
        }
        let index = self.index;
        let is = |c: &str, k: &str| ctx.name == c && keyword.eq_ignore_ascii_case(k);
        let column_of_table = matches!(
            ctx.name,
            "column" | "calculatedColumn" | "calculatedTableColumn"
        ) && keyword.eq_ignore_ascii_case("sortByColumn")
            || is("level", "column");
        let problem = if column_of_table {
            table.and_then(|t| self.column_problem(t, &tmdl_unquote(value)))
        } else if is("relatedColumnDetails", "groupByColumn") {
            let name = tmdl_unquote(value);
            let local = table
                .and_then(|t| index.table(t))
                .is_some_and(|t| t.columns.contains_key(&name_key(&name)));
            if local {
                None
            } else if parse_tmdl_column_ref(value).is_some() {
                self.path_problem(value, Member::Column)
            } else {
                table.and_then(|t| self.column_problem(t, &name))
            }
        } else if is("relationship", "fromColumn")
            || is("relationship", "toColumn")
            || is("expression", "parameterValuesColumn")
            || is("alternateOf", "baseColumn")
            || is("variation", "defaultColumn")
        {
            self.path_problem(value, Member::Column)
        } else if is("variation", "defaultHierarchy") {
            self.path_problem(value, Member::Hierarchy)
        } else if keyword.eq_ignore_ascii_case("queryGroup")
            && matches!(
                ctx.name,
                "expression" | "partition" | "calculatedPartition" | "calculationGroupPartition"
            )
        {
            self.object_problem("queryGroup", "query group", &tmdl_unquote(value))
        } else if is("variation", "relationship") {
            self.object_problem("relationship", "relationship", &tmdl_unquote(value))
        } else if is("alternateOf", "baseTable") {
            self.object_problem("table", "table", &tmdl_unquote(value))
        } else if is("model", "defaultMeasure") {
            self.measure_problem(value)
        } else {
            None
        };
        if let Some((message, suggestion)) = problem {
            self.push(
                At::value(file, path, node),
                Severity::Error,
                codes::UNRESOLVED_REFERENCE,
                format!("`{keyword}` refers to {message}"),
                suggestion,
            );
        }
    }

    /// `tablePermission X`, `perspectiveTable X` and their columns, measures and hierarchies.
    fn check_named_reference(
        &mut self,
        file: usize,
        path: &str,
        ctx: &Context,
        node: &Node,
        table: Option<&str>,
    ) {
        if !self.resolve || node.is_ref() {
            return;
        }
        let Some(name) = node.name() else {
            return;
        };
        let keyword = node.keyword();
        let is = |c: &str, k: &str| ctx.name == c && keyword.eq_ignore_ascii_case(k);
        let problem = if is("role", "tablePermission") || is("perspective", "perspectiveTable") {
            self.object_problem("table", "table", &name)
        } else if is("tablePermission", "columnPermission")
            || is("perspectiveTable", "perspectiveColumn")
        {
            table.and_then(|t| self.member_problem(t, &name, Member::Column))
        } else if is("perspectiveTable", "perspectiveMeasure") {
            table.and_then(|t| self.member_problem(t, &name, Member::Measure))
        } else if is("perspectiveTable", "perspectiveHierarchy") {
            table.and_then(|t| self.member_problem(t, &name, Member::Hierarchy))
        } else {
            None
        };
        if let Some((message, suggestion)) = problem {
            self.push(
                At::name(file, path, node),
                Severity::Error,
                codes::UNRESOLVED_REFERENCE,
                format!("`{keyword}` refers to {message}"),
                suggestion.map(|s| tmdl_name(&s).into_owned()),
            );
        }
    }

    /// The tables, columns, measures and hierarchies a culture's `translations` name exist.
    fn check_translations(&mut self, file: usize, path: &str, node: &Node) {
        if !self.resolve {
            return;
        }
        let tables = node
            .children
            .iter()
            .filter(|m| m.keyword().eq_ignore_ascii_case("model"))
            .flat_map(|m| &m.children)
            .filter(|t| t.keyword().eq_ignore_ascii_case("table"));
        for table in tables {
            let Some(name) = table.name() else {
                continue;
            };
            let problem = if self.index.table(&name).is_none() {
                self.object_problem("table", "table", &name)
                    .map(|p| (table, p))
                    .into_iter()
                    .collect()
            } else {
                let mut found = Vec::new();
                for child in &table.children {
                    let k = child.keyword();
                    let member = if k.eq_ignore_ascii_case("column") {
                        Member::Column
                    } else if k.eq_ignore_ascii_case("measure") {
                        Member::Measure
                    } else if k.eq_ignore_ascii_case("hierarchy") {
                        Member::Hierarchy
                    } else {
                        continue;
                    };
                    if let Some(child_name) = child.name()
                        && let Some(p) = self.member_problem(&name, &child_name, member)
                    {
                        found.push((child, p));
                    }
                }
                found
            };
            for (at, (message, suggestion)) in problem {
                self.push(
                    At::name(file, path, at),
                    Severity::Error,
                    codes::UNRESOLVED_REFERENCE,
                    format!("this translation refers to {message}"),
                    suggestion.map(|s| tmdl_name(&s).into_owned()),
                );
            }
        }
    }

    fn object_problem(
        &self,
        kind: &str,
        label: &str,
        name: &str,
    ) -> Option<(String, Option<String>)> {
        if self.index.has_object(kind, name) {
            return None;
        }
        let names = self.index.object_names(kind);
        let suggestion = closest_name(name, names).map(str::to_owned);
        Some((
            format!("{label} `{name}`, which is not declared"),
            suggestion,
        ))
    }

    fn column_problem(&self, table: &str, column: &str) -> Option<(String, Option<String>)> {
        self.member_problem(table, column, Member::Column)
    }

    /// `name` is not a column, measure or hierarchy of `table` (nothing when the table itself
    /// is missing: that is reported where the table is named).
    fn member_problem(
        &self,
        table: &str,
        name: &str,
        member: Member,
    ) -> Option<(String, Option<String>)> {
        let info = self.index.table(table)?;
        let names = member.of(info);
        if names.contains_key(&name_key(name)) {
            return None;
        }
        let suggestion = closest_name(name, names.values().map(String::as_str)).map(str::to_owned);
        Some((
            format!(
                "{} `{name}`, which is not in table `{table}`",
                member.label()
            ),
            suggestion,
        ))
    }

    /// A `Table.Member` path.
    fn path_problem(&self, value: &str, member: Member) -> Option<(String, Option<String>)> {
        let Some(parsed) = parse_tmdl_column_ref(value) else {
            return Some((
                format!(
                    "`{}`, which is not a `Table.{}` path",
                    truncate(value.trim(), 40),
                    member.label()
                ),
                None,
            ));
        };
        let (table, name) = (parsed.table, parsed.column);
        if self.index.table(&table).is_none() {
            let (message, suggestion) = self.object_problem("table", "table", &table)?;
            let suggestion = suggestion.map(|t| format!("{}.{}", tmdl_name(&t), tmdl_name(&name)));
            return Some((message, suggestion));
        }
        let (message, suggestion) = self.member_problem(&table, &name, member)?;
        let suggestion = suggestion.map(|c| format!("{}.{}", tmdl_name(&table), tmdl_name(&c)));
        Some((message, suggestion))
    }

    fn measure_problem(&self, value: &str) -> Option<(String, Option<String>)> {
        let name = tmdl_unquote(value);
        let key = name_key(&name);
        let anywhere = self
            .index
            .tables
            .values()
            .any(|(_, t)| t.measures.contains_key(&key));
        if anywhere {
            return None;
        }
        if let Some(parsed) = parse_tmdl_column_ref(value)
            && self
                .index
                .table(&parsed.table)
                .is_some_and(|t| t.measures.contains_key(&name_key(&parsed.column)))
        {
            return None;
        }
        let all: Vec<&str> = self
            .index
            .tables
            .values()
            .flat_map(|(_, t)| t.measures.values().map(String::as_str))
            .collect();
        let suggestion = closest_name(&name, all).map(str::to_owned);
        Some((
            format!("measure `{name}`, which is not declared"),
            suggestion,
        ))
    }

    fn check_root_ref(&mut self, file: usize, path: &str, node: &Node) {
        let kind = node.is_ref().then(|| ref_kind(node.keyword())).flatten();
        let (Some(kind), Some(name), None) = (kind, node.name(), node.operator()) else {
            self.error(
                At::line(file, path, node),
                codes::INVALID_REF,
                "a top-level `ref` line names an object: `ref table <name>` (also `role`, \
                 `cultureInfo`, `perspective`, `expression`, `queryGroup`, `dataSource`, \
                 `relationship`, `function`)"
                    .to_owned(),
            );
            return;
        };
        if !self.resolve || self.index.has_object(kind, &name) {
            return;
        }
        let names = self.index.object_names(kind);
        let suggestion = closest_name(&name, names).map(|s| tmdl_name(s).into_owned());
        self.push(
            At::name(file, path, node),
            Severity::Warning,
            codes::MISSING_REF_TARGET,
            format!(
                "`ref {} {}` names an object that is not declared; the reader ignores the line",
                node.keyword(),
                node.name_raw().unwrap_or_default()
            ),
            suggestion,
        );
    }

    // -----------------------------------------------------------------------------------------
    // Declarations of one object in several places

    fn check_merges(&mut self, files: &[(usize, &TmdlFile)]) {
        let Some(root) = self.catalog.context("root") else {
            return;
        };
        let decls: Vec<Decl<'_>> = files
            .iter()
            .flat_map(|(i, f)| {
                f.nodes.iter().map(move |node| Decl {
                    file: *i,
                    path: f.path.as_str(),
                    node,
                })
            })
            .collect();
        self.merge_children(root, &decls, None);
    }

    /// Groups the named child objects of all declarations of one object by type and name and
    /// checks every group that has more than one declaration.
    fn merge_children(
        &mut self,
        ctx: &'static Context,
        children: &[Decl<'_>],
        table: Option<&str>,
    ) {
        let root = ctx.name == "root";
        let mut groups: Vec<(String, Vec<Decl<'_>>)> = Vec::new();
        let mut by_key: HashMap<String, usize> = HashMap::new();
        for d in children {
            let node = d.node;
            if node.is_ref() {
                continue;
            }
            let Some(keyword) = ctx.keyword(node.keyword()) else {
                continue;
            };
            let form = node_form(node);
            if !keyword.support(form).accepted() || !keyword.is_object(form) {
                continue;
            }
            let singleton = root
                && (keyword.keyword.eq_ignore_ascii_case("model")
                    || keyword.keyword.eq_ignore_ascii_case("database"));
            let key = if singleton {
                keyword.keyword.to_ascii_lowercase()
            } else {
                match node.name() {
                    Some(name) => format!("{}\u{0}{}", keyword.keyword, name_key(&name)),
                    None => continue,
                }
            };
            match by_key.get(&key) {
                Some(&i) => {
                    if let Some(group) = groups.get_mut(i) {
                        group.1.push(*d);
                    }
                }
                None => {
                    by_key.insert(key.clone(), groups.len());
                    groups.push((key, vec![*d]));
                }
            }
        }
        for (_, decls) in &groups {
            let Some(first) = decls.first() else {
                continue;
            };
            let keyword = node_keyword(first.node).to_owned();
            if decls.len() > 1 && !self.merge_conflicts(root, &keyword, decls, table) {
                continue;
            }
            let own_table = self.scope_table(&keyword, first.node);
            let table = own_table.as_deref().or(table);
            let Some(child_ctx) = self.child_context(&keyword, first.node, table) else {
                continue;
            };
            let grandchildren: Vec<Decl<'_>> = decls
                .iter()
                .flat_map(|d| {
                    d.node.children.iter().map(move |node| Decl {
                        file: d.file,
                        path: d.path,
                        node,
                    })
                })
                .collect();
            self.merge_children(child_ctx, &grandchildren, table);
        }
    }

    /// Reports conflicts between the declarations of one object; `false` if they cannot be
    /// merged at all (a second model with another name).
    fn merge_conflicts(
        &mut self,
        root: bool,
        keyword: &str,
        decls: &[Decl<'_>],
        table: Option<&str>,
    ) -> bool {
        let Some(first) = decls.first() else {
            return true;
        };
        if root
            && (keyword.eq_ignore_ascii_case("model") || keyword.eq_ignore_ascii_case("database"))
        {
            let first_name = first.node.name().map(|n| name_key(&n));
            let mut same = true;
            for d in decls.iter().skip(1) {
                if d.node.name().map(|n| name_key(&n)) != first_name {
                    same = false;
                    self.error(
                        At::line(d.file, d.path, d.node),
                        codes::SECOND_MODEL,
                        format!(
                            "a second `{keyword}` with another name (the first is at {}:{}); \
                             a model has one `{keyword}`",
                            first.path,
                            first.node.line_number()
                        ),
                    );
                }
            }
            if !same {
                return false;
            }
        }
        let label = match first.node.name_raw() {
            Some(name) => format!("{keyword} {name}"),
            None => keyword.to_owned(),
        };
        let mut with_value: Option<&Decl<'_>> = None;
        let mut set: HashMap<String, &Decl<'_>> = HashMap::new();
        let child_ctx = self.child_context(keyword, first.node, table);
        for d in decls {
            if d.node.operator() == Some(Operator::Equals) {
                if let Some(prev) = with_value {
                    self.error(
                        At::line(d.file, d.path, d.node),
                        codes::MERGE_CONFLICT,
                        format!(
                            "`{label}` is declared again with a value (also at {}:{}); TMDL \
                             merges the declarations of an object, so only one may have a value",
                            prev.path,
                            prev.node.line_number()
                        ),
                    );
                    continue;
                }
                with_value = Some(d);
            }
            let Some(ctx) = child_ctx else {
                continue;
            };
            let mut own: HashSet<String> = HashSet::new();
            let mut conflict: Option<(String, &Decl<'_>)> = None;
            for child in &d.node.children {
                if child.is_ref() {
                    continue;
                }
                let Some(k) = ctx.keyword(child.keyword()) else {
                    continue;
                };
                let form = node_form(child);
                if !k.support(form).accepted() || k.is_object(form) {
                    continue;
                }
                let key = k.keyword.to_ascii_lowercase();
                if !own.insert(key.clone()) {
                    continue;
                }
                if conflict.is_none()
                    && let Some(prev) = set.get(&key)
                {
                    conflict = Some((k.keyword.to_owned(), *prev));
                }
            }
            if let Some((property, prev)) = conflict {
                self.error(
                    At::line(d.file, d.path, d.node),
                    codes::MERGE_CONFLICT,
                    format!(
                        "`{label}` is declared again and both declarations set `{property}` (also \
                         at {}:{}); TMDL merges the declarations of an object, so a property may \
                         be set in only one of them",
                        prev.path,
                        prev.node.line_number()
                    ),
                );
                continue;
            }
            for key in own {
                set.entry(key).or_insert(d);
            }
        }
        true
    }
}

/// One declaration of an object.
#[derive(Clone, Copy)]
struct Decl<'a> {
    file: usize,
    path: &'a str,
    node: &'a Node,
}

#[derive(Clone, Copy)]
enum Member {
    Column,
    Measure,
    Hierarchy,
}

impl Member {
    fn of(self, table: &TableInfo) -> &Names {
        match self {
            Member::Column => &table.columns,
            Member::Measure => &table.measures,
            Member::Hierarchy => &table.hierarchies,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Member::Column => "column",
            Member::Measure => "measure",
            Member::Hierarchy => "hierarchy",
        }
    }
}

/// `the top level` or `` `table` ``.
fn context_label(ctx: &Context) -> String {
    if ctx.name == "root" {
        "the top level of a file".to_owned()
    } else {
        format!("`{}`", ctx.name)
    }
}

/// A `contentType: json` (or `1`) line among `nodes`.
fn content_is_json(nodes: &[Node]) -> bool {
    nodes.iter().any(|n| {
        n.keyword().eq_ignore_ascii_case("contentType")
            && n.value_raw()
                .is_some_and(|v| v.eq_ignore_ascii_case("json") || v == "1")
    })
}

fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", s.get(..i).unwrap_or(s)),
        None => s.to_owned(),
    }
}

/// Byte offset of 1-based `line` and character `column` in `text` (a BOM is skipped).
fn byte_offset(text: &str, line: u32, column: u32) -> usize {
    let mut offset = if text.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    };
    for _ in 1..line {
        match text.get(offset..).and_then(|rest| rest.find('\n')) {
            Some(i) => offset += i + 1,
            None => return text.len(),
        }
    }
    let rest = text.get(offset..).unwrap_or_default();
    let line_text = rest.split(['\r', '\n']).next().unwrap_or_default();
    let skip = usize::try_from(column.saturating_sub(1)).unwrap_or(usize::MAX);
    offset
        + line_text
            .char_indices()
            .nth(skip)
            .map_or(line_text.len(), |(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(files: &[(&str, &str)]) -> Vec<(String, u32, &'static str)> {
        let sources: Vec<Source> = files.iter().map(|(p, t)| Source::new(*p, *t)).collect();
        diagnostics(&sources)
            .into_iter()
            .map(|d| (d.path, d.line, d.code))
            .collect()
    }

    fn codes_of(text: &str) -> Vec<(u32, &'static str)> {
        check(&[("t.tmdl", text)])
            .into_iter()
            .map(|(_, l, c)| (l, c))
            .collect()
    }

    #[test]
    fn keywords_forms_and_values() {
        let text = "table T\n\tlineageTagg: x\n\tlevel L\n\tisHidden abc\n\n\tcolumn C\n\t\tdataType: strin\n\t\tisKey: yes\n\t\tisKey\n\t\tsummarizeBy: none\n";
        assert_eq!(
            codes_of(text),
            [
                (2, codes::UNKNOWN_KEYWORD),
                (3, codes::WRONG_CONTEXT),
                (4, codes::WRONG_FORM),
                (7, codes::INVALID_VALUE),
                (8, codes::INVALID_VALUE),
                (9, codes::DUPLICATE_PROPERTY),
            ]
        );
        let sources = [Source::new("t.tmdl", text)];
        let all = diagnostics(&sources);
        assert_eq!(all[0].suggestion.as_deref(), Some("lineageTag"));
        assert_eq!(all[3].suggestion.as_deref(), Some("string"));
        assert_eq!(&text[all[3].start..all[3].end], "strin");
        assert_eq!((all[3].line, all[3].column), (7, 13));
    }

    #[test]
    fn values_follow_the_reader() {
        let ok = "table T\n\tcolumn C\n\t\tdataType: STRING\n\t\tisKey: True\n\t\tencodingHint: hash, value\n\t\tdisplayOrdinal: +10\n\t\tisHidden = true\n";
        assert_eq!(codes_of(ok), []);
        for bad in [
            "dataType: 'string'",
            "isKey: 1",
            "displayOrdinal: 99999999999",
            "displayOrdinal: 1.5",
            "dataType:",
        ] {
            let text = format!("table T\n\tcolumn C\n\t\t{bad}\n");
            assert_eq!(codes_of(&text), [(3, codes::INVALID_VALUE)], "{bad}");
        }
    }

    #[test]
    fn order_and_indentation() {
        assert_eq!(
            codes_of("table T\n\tcolumn C\n\t\tannotation X = 1\n\t\tdataType: string\n"),
            [(4, codes::PROPERTY_AFTER_OBJECT)]
        );
        assert_eq!(
            codes_of("table T\n\tcolumn C\n\t\tdataType: string\n\t\t\tisKey\n"),
            [(4, codes::PROPERTY_WITH_CHILDREN)]
        );
        assert_eq!(
            codes_of("table T\n\tcolumn C\n\t\tdataType: string\n\t  isKey\n"),
            [(4, codes::SYNTAX)]
        );
        let sources = [Source::new("t.tmdl", "table T\n\tcolumn 'C\n")];
        let d = diagnostics(&sources);
        assert_eq!((d[0].code, d[0].line, d[0].column), (codes::SYNTAX, 2, 9));
        assert_eq!(&sources[0].text[d[0].start..d[0].end], "'C");
    }

    #[test]
    fn merges_and_references() {
        let model = "model Model\n\nref table T\nref table X\n";
        let t = "table T\n\tcolumn A\n\t\tdataType: string\n\t\tsortByColumn: b\n\n\tcolumn B\n\t\tdataType: string\n\t\tsortByColumn: Bx\n\n\tcolumn A\n\t\tdataType: int64\n\n\tmeasure M = 1\n\n\tmeasure M = 2\n";
        let rel = "relationship r1\n\tfromColumn: T.A\n\ttoColumn: U.A\n";
        let found = check(&[("model.tmdl", model), ("T.tmdl", t), ("rel.tmdl", rel)]);
        assert_eq!(
            found,
            [
                ("model.tmdl".to_owned(), 4, codes::MISSING_REF_TARGET),
                ("T.tmdl".to_owned(), 8, codes::UNRESOLVED_REFERENCE),
                ("T.tmdl".to_owned(), 10, codes::MERGE_CONFLICT),
                ("T.tmdl".to_owned(), 15, codes::MERGE_CONFLICT),
                ("rel.tmdl".to_owned(), 3, codes::UNRESOLVED_REFERENCE),
            ]
        );
        // Without a model (a single file) references are not resolved.
        assert_eq!(check(&[("rel.tmdl", rel)]), []);
        // A partial declaration in another file merges.
        let extra = "table T\n\n\tcolumn E\n\t\tdataType: string\n";
        assert_eq!(
            check(&[
                ("model.tmdl", "model Model\n"),
                ("T.tmdl", t),
                ("E.tmdl", extra)
            ])
            .iter()
            .filter(|d| d.0 == "E.tmdl")
            .count(),
            0
        );
    }

    #[test]
    fn root_refs_singletons_and_levels() {
        let model = "model Model\n\nref abc\nref table\n";
        assert_eq!(
            codes_of(model),
            [(3, codes::INVALID_REF), (4, codes::INVALID_REF)]
        );
        let found = check(&[
            ("database.tmdl", "database\n\tcompatibilityLevel: 1601\n"),
            ("model.tmdl", "model Model\n\nfunction F = () => 1\n"),
            ("other.tmdl", "model Other\n"),
        ]);
        assert_eq!(
            found,
            [
                ("model.tmdl".to_owned(), 3, codes::COMPATIBILITY_LEVEL),
                ("other.tmdl".to_owned(), 1, codes::SECOND_MODEL),
            ]
        );
    }

    #[test]
    fn contexts_follow_the_declaration() {
        // A calculated table's columns take `isNameInferred`; a data column does not.
        let calc = "table T\n\tcolumn C\n\t\tisNameInferred\n\n\tpartition T = calculated\n\t\tsource = 1\n";
        assert_eq!(codes_of(calc), []);
        let data = "table T\n\tcolumn C\n\t\tisNameInferred\n\n\tpartition T = m\n\t\tsource = 1\n";
        assert_eq!(codes_of(data), [(3, codes::WRONG_CONTEXT)]);
        // Content follows the object's own contentType.
        let xml = "cultureInfo en-US\n\tlinguisticMetadata = <Linguistic />\n";
        assert_eq!(codes_of(xml), []);
        let json = "cultureInfo en-US\n\tlinguisticMetadata =\n\t\t\t{\"Version\": \"1.0.0\"}\n\t\tcontentType: json\n";
        assert_eq!(codes_of(json), []);
        let mismatch =
            "cultureInfo en-US\n\tlinguisticMetadata = <Linguistic />\n\t\tcontentType: json\n";
        assert_eq!(codes_of(mismatch), [(2, codes::INVALID_VALUE)]);
    }

    #[test]
    fn byte_offsets_skip_the_bom_and_count_characters() {
        let text = "\u{feff}ab\n\u{e9}xy\n";
        assert_eq!(byte_offset(text, 1, 1), 3);
        assert_eq!(byte_offset(text, 2, 2), 8);
        assert_eq!(byte_offset(text, 9, 1), text.len());
    }
}
