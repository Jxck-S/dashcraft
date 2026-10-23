//! Broken-reference linter.
//!
//! [`lint`] (for an [`OpenedDocument`]) and [`lint_document`] (for a bare
//! [`Document`]) check that reports and models reference things that exist:
//! visual fields, `queryRef`s, filters, sorts and `visualInteractions`
//! against the bound semantic model, DAX and M references inside the model,
//! relationships and query groups, plus a few layout sanity checks. Every
//! rule is listed in [`RULES`] and documented in `src/lint/RULES.md`.
//!
//! Model-dependent report checks are skipped (with a note in
//! [`LintSummary::notes`]) when a report's model is remote (`byConnection`)
//! or was not loaded. Self-contained checks (`queryRef` consistency,
//! interactions, duplicates, layout) always run.
//!
//! ```no_run
//! use pbi_core::lint::{LintOptions, lint};
//! use pbi_core::{OpenOptions, open_path};
//!
//! let doc = open_path("AcmeOutfitters.pbip".as_ref(), &OpenOptions::default()).unwrap();
//! let report = lint(&doc, &LintOptions::default());
//! for f in &report.findings {
//!     println!("{} {:?} {}", f.rule, f.severity, f.message);
//! }
//! ```

mod dax;
mod m;
mod model;
mod report;
mod suggest;

use std::collections::BTreeMap;
use std::sync::Arc;

use pbi_model::{
    DatasetReference, Diagnostic, Document, Model, ObjectId, Origin, Report, Severity,
};
use serde::Serialize;
use serde_json::Value;

use crate::reader::{OpenedDocument, ReportSource};

/// Rule identifiers.
pub mod rules {
    /// A visual projection (or formatting expression) references a table
    /// missing from the model.
    pub const MISSING_TABLE: &str = "LINT001";
    /// A visual projection references a column, measure or hierarchy level
    /// missing from its table.
    pub const MISSING_FIELD: &str = "LINT002";
    /// A projection's `queryRef`/`nativeQueryRef` does not match its field.
    pub const STALE_QUERY_REF: &str = "LINT003";
    /// A report, page or visual filter references a missing table or field.
    pub const FILTER_REF: &str = "LINT004";
    /// A visual sort definition references a missing table or field.
    pub const SORT_REF: &str = "LINT005";
    /// A `visualInteractions` entry references a visual missing from the page.
    pub const INTERACTION_REF: &str = "LINT006";
    /// A DAX expression references an unknown table, column or measure.
    pub const DAX_REF: &str = "LINT007";
    /// A DAX expression does not parse.
    pub const DAX_SYNTAX: &str = "LINT008";
    /// A relationship references a missing table or column.
    pub const RELATIONSHIP_REF: &str = "LINT009";
    /// The two key columns of a relationship have different data types.
    pub const RELATIONSHIP_TYPE: &str = "LINT010";
    /// A partition or expression references an undefined query group.
    pub const QUERY_GROUP_REF: &str = "LINT011";
    /// A report's dataset reference does not resolve (or is remote).
    pub const DATASET_REF: &str = "LINT012";
    /// An M query references an undefined step, parameter or query.
    pub const M_REF: &str = "LINT013";
    /// Duplicate page or visual names.
    pub const DUPLICATE_NAME: &str = "LINT014";
    /// A visual is outside its page or has no size.
    pub const VISUAL_BOUNDS: &str = "LINT015";
    /// Two visuals overlap.
    pub const VISUAL_OVERLAP: &str = "LINT016";
    /// A column's `sortByColumn` or a hierarchy level references a missing
    /// column.
    pub const MODEL_COLUMN_REF: &str = "LINT017";
}

/// A lint rule: stable id, default severity and a one-line title.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub id: &'static str,
    /// Usual severity; individual findings may be lower (see `RULES.md`).
    pub severity: Severity,
    pub title: &'static str,
}

/// Every rule, in id order.
pub const RULES: &[Rule] = &[
    Rule {
        id: rules::MISSING_TABLE,
        severity: Severity::Error,
        title: "visual field references a table missing from the model",
    },
    Rule {
        id: rules::MISSING_FIELD,
        severity: Severity::Error,
        title: "visual field references a column/measure missing from its table",
    },
    Rule {
        id: rules::STALE_QUERY_REF,
        severity: Severity::Warning,
        title: "queryRef / nativeQueryRef does not match the projection's field",
    },
    Rule {
        id: rules::FILTER_REF,
        severity: Severity::Error,
        title: "filter references a missing table or field",
    },
    Rule {
        id: rules::SORT_REF,
        severity: Severity::Error,
        title: "sort definition references a missing table or field",
    },
    Rule {
        id: rules::INTERACTION_REF,
        severity: Severity::Warning,
        title: "visualInteractions references a visual missing from the page",
    },
    Rule {
        id: rules::DAX_REF,
        severity: Severity::Error,
        title: "DAX references an unknown table, column or measure",
    },
    Rule {
        id: rules::DAX_SYNTAX,
        severity: Severity::Error,
        title: "DAX expression does not parse",
    },
    Rule {
        id: rules::RELATIONSHIP_REF,
        severity: Severity::Error,
        title: "relationship references a missing table or column",
    },
    Rule {
        id: rules::RELATIONSHIP_TYPE,
        severity: Severity::Warning,
        title: "relationship key columns have different data types",
    },
    Rule {
        id: rules::QUERY_GROUP_REF,
        severity: Severity::Error,
        title: "partition/expression references an undefined query group",
    },
    Rule {
        id: rules::DATASET_REF,
        severity: Severity::Error,
        title: "report dataset reference does not resolve",
    },
    Rule {
        id: rules::M_REF,
        severity: Severity::Warning,
        title: "M query references an undefined step, parameter or query",
    },
    Rule {
        id: rules::DUPLICATE_NAME,
        severity: Severity::Error,
        title: "duplicate page or visual name",
    },
    Rule {
        id: rules::VISUAL_BOUNDS,
        severity: Severity::Warning,
        title: "visual outside its page or zero-size",
    },
    Rule {
        id: rules::VISUAL_OVERLAP,
        severity: Severity::Info,
        title: "visuals overlap",
    },
    Rule {
        id: rules::MODEL_COLUMN_REF,
        severity: Severity::Error,
        title: "sortByColumn / hierarchy level references a missing column",
    },
];

/// The rule with `id` (case-insensitive).
pub fn rule(id: &str) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.id.eq_ignore_ascii_case(id))
}

/// Numeric rank for comparisons: `Info` 0, `Warning` 1, `Error` 2.
pub fn severity_rank(severity: Severity) -> u8 {
    match severity {
        Severity::Info => 0,
        Severity::Warning => 1,
        Severity::Error => 2,
    }
}

/// Options for [`lint`] and [`lint_document`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LintOptions {
    /// Rule ids to skip (case-insensitive), e.g. `LINT016`.
    pub disabled_rules: Vec<String>,
    /// Findings below this severity are dropped. Default `Info` (keep all).
    pub min_severity: Severity,
    /// Run the layout rules (LINT015, LINT016). Default `true`.
    pub layout: bool,
}

impl Default for LintOptions {
    fn default() -> Self {
        Self {
            disabled_rules: Vec::new(),
            min_severity: Severity::Info,
            layout: true,
        }
    }
}

impl LintOptions {
    /// `true` if `rule` runs under these options.
    pub fn is_enabled(&self, rule: &str) -> bool {
        if !self.layout && (rule == rules::VISUAL_BOUNDS || rule == rules::VISUAL_OVERLAP) {
            return false;
        }
        !self
            .disabled_rules
            .iter()
            .any(|r| r.trim().eq_ignore_ascii_case(rule))
    }
}

/// One lint finding.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// Rule id, e.g. `LINT002`.
    pub rule: &'static str,
    pub severity: Severity,
    pub message: String,
    /// Source file (container-relative) plus JSON pointer or line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    /// The offending object (visual, measure, relationship, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<ObjectId>,
    /// A fix hint, e.g. "did you mean `Products[Category Name]`?".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

impl Finding {
    pub fn new(rule: &'static str, severity: Severity, message: impl Into<String>) -> Self {
        Self {
            rule,
            severity,
            message: message.into(),
            origin: None,
            object: None,
            suggestion: None,
        }
    }

    #[must_use]
    pub fn at(mut self, origin: Option<Origin>) -> Self {
        self.origin = origin;
        self
    }

    #[must_use]
    pub fn on(mut self, object: ObjectId) -> Self {
        self.object = object.is_assigned().then_some(object);
        self
    }

    #[must_use]
    pub fn suggest(mut self, suggestion: Option<String>) -> Self {
        self.suggestion = suggestion;
        self
    }

    /// As a [`Diagnostic`] (`code` = rule id; the suggestion is appended to
    /// the message).
    pub fn to_diagnostic(&self) -> Diagnostic {
        let message = match &self.suggestion {
            Some(s) => format!("{} ({s})", self.message),
            None => self.message.clone(),
        };
        Diagnostic {
            severity: self.severity,
            code: self.rule.to_owned(),
            message,
            origin: self.origin.clone(),
            object: self.object,
        }
    }
}

impl From<&Finding> for Diagnostic {
    fn from(f: &Finding) -> Self {
        f.to_diagnostic()
    }
}

impl From<Finding> for Diagnostic {
    fn from(f: Finding) -> Self {
        f.to_diagnostic()
    }
}

/// Counts per severity and rule, plus notes about skipped checks.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LintSummary {
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
    /// Findings per rule id.
    pub by_rule: BTreeMap<String, usize>,
    /// Checks that were skipped and why (remote or missing model, …).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// Result of [`lint`] / [`lint_document`].
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LintReport {
    /// Findings sorted by file, location and rule.
    pub findings: Vec<Finding>,
    pub summary: LintSummary,
}

impl LintReport {
    /// Number of findings with `severity`.
    pub fn count(&self, severity: Severity) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == severity)
            .count()
    }

    pub fn has_errors(&self) -> bool {
        self.summary.errors > 0
    }

    /// `true` if any finding is at least as severe as `deny`.
    pub fn fails(&self, deny: Severity) -> bool {
        self.findings
            .iter()
            .any(|f| severity_rank(f.severity) >= severity_rank(deny))
    }

    /// Findings of one rule.
    pub fn of_rule<'a>(&'a self, rule: &'a str) -> impl Iterator<Item = &'a Finding> + 'a {
        self.findings
            .iter()
            .filter(move |f| f.rule.eq_ignore_ascii_case(rule))
    }

    /// Findings grouped by origin file (`""` for findings without one), in
    /// file order.
    pub fn by_file(&self) -> Vec<(&str, Vec<&Finding>)> {
        let mut out: Vec<(&str, Vec<&Finding>)> = Vec::new();
        for f in &self.findings {
            let path = f.origin.as_ref().map_or("", |o| o.path.as_str());
            match out.last_mut() {
                Some((p, list)) if *p == path => list.push(f),
                _ => out.push((path, vec![f])),
            }
        }
        out
    }

    /// Every finding as a [`Diagnostic`].
    pub fn to_diagnostics(&self) -> Vec<Diagnostic> {
        self.findings.iter().map(Finding::to_diagnostic).collect()
    }

    /// Stable JSON: `{schemaVersion: 1, summary, findings}`.
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "schemaVersion": 1,
            "summary": self.summary,
            "findings": self.findings,
        })
    }
}

/// Collects findings, applying [`LintOptions`].
pub(crate) struct Sink<'o> {
    opts: &'o LintOptions,
    findings: Vec<Finding>,
    notes: Vec<String>,
}

impl<'o> Sink<'o> {
    fn new(opts: &'o LintOptions) -> Self {
        Self {
            opts,
            findings: Vec::new(),
            notes: Vec::new(),
        }
    }

    pub(crate) fn enabled(&self, rule: &str) -> bool {
        self.opts.is_enabled(rule)
    }

    pub(crate) fn push(&mut self, finding: Finding) {
        if self.enabled(finding.rule)
            && severity_rank(finding.severity) >= severity_rank(self.opts.min_severity)
        {
            self.findings.push(finding);
        }
    }

    pub(crate) fn note(&mut self, note: String) {
        if !self.notes.contains(&note) {
            self.notes.push(note);
        }
    }

    fn finish(mut self) -> LintReport {
        self.findings.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
        self.findings.dedup();
        let mut summary = LintSummary {
            notes: self.notes,
            ..LintSummary::default()
        };
        for f in &self.findings {
            match f.severity {
                Severity::Error => summary.errors += 1,
                Severity::Warning => summary.warnings += 1,
                Severity::Info => summary.infos += 1,
            }
            *summary.by_rule.entry(f.rule.to_owned()).or_default() += 1;
        }
        LintReport {
            findings: self.findings,
            summary,
        }
    }
}

type SortKey<'a> = (&'a str, u32, &'a str, &'a str, &'a str);

fn sort_key(f: &Finding) -> SortKey<'_> {
    let (path, line, pointer) = match &f.origin {
        Some(o) => (
            o.path.as_str(),
            o.line.unwrap_or(0),
            o.pointer.as_deref().unwrap_or(""),
        ),
        None => ("", 0, ""),
    };
    (path, line, pointer, f.rule, &f.message)
}

/// The model a report is checked against.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Binding<'a> {
    Model(&'a Model),
    /// `byConnection`: the model lives in the service.
    Remote,
    /// `byPath` that matches no semantic model folder.
    Unresolved,
    /// No model loaded (PBIX without `DataModel`, model failed to load, …).
    Missing,
}

/// How one report is checked.
struct ReportCtx<'a> {
    folder: String,
    binding: Binding<'a>,
    ext: report::ExtensionMeasures,
}

/// Lints an opened document. Reports are checked against the model their
/// `byPath` points to (the document model or one of the extra models);
/// report-level measures (`reportExtensions.json`) count as defined.
pub fn lint(doc: &OpenedDocument, opts: &LintOptions) -> LintReport {
    let document = &doc.document;
    let mut models: Vec<(Option<&str>, &Model)> = Vec::new();
    if let Some(m) = &document.model {
        // A lone report folder's byPath model is read from its own container
        // (next to the opened folder); it is the one the report points to.
        let folder = match &doc.model_source {
            Some(s) if !Arc::ptr_eq(s.container(), &doc.container) => None,
            Some(s) => Some(s.folder()),
            None => Some(""),
        };
        models.push((folder, m));
    }
    for extra in &doc.extra_models {
        models.push((Some(extra.source.folder()), &extra.model));
    }
    let reports: Vec<ReportCtx<'_>> = document
        .reports
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let source = doc.report_sources.get(i);
            let folder = source.map_or_else(|| report_folder(r), |s| s.folder().to_owned());
            let binding = bind(document, r, &folder, &models);
            ReportCtx {
                folder,
                binding,
                ext: source.map(extension_measures).unwrap_or_default(),
            }
        })
        .collect();
    let all: Vec<&Model> = models.iter().map(|(_, m)| *m).collect();
    run(document, &reports, &all, opts)
}

/// Lints a document without container information: every local report is
/// checked against `doc.model` (multi-model projects and report-level
/// measures need [`lint`]).
pub fn lint_document(doc: &Document, opts: &LintOptions) -> LintReport {
    let models: Vec<(Option<&str>, &Model)> = doc.model.iter().map(|m| (Some(""), m)).collect();
    let reports: Vec<ReportCtx<'_>> = doc
        .reports
        .iter()
        .map(|r| {
            let folder = report_folder(r);
            let binding = match (&r.dataset_reference, &doc.model) {
                (DatasetReference::ByConnection { .. }, _) => Binding::Remote,
                (_, Some(m)) => Binding::Model(m),
                (DatasetReference::ByPath { .. }, None) => bind(doc, r, &folder, &models),
                (_, None) => Binding::Missing,
            };
            ReportCtx {
                folder,
                binding,
                ext: report::ExtensionMeasures::new(),
            }
        })
        .collect();
    let all: Vec<&Model> = doc.model.iter().collect();
    run(doc, &reports, &all, opts)
}

fn run(
    doc: &Document,
    reports: &[ReportCtx<'_>],
    models: &[&Model],
    opts: &LintOptions,
) -> LintReport {
    let mut sink = Sink::new(opts);
    if models.is_empty() {
        sink.note("no local semantic model: model checks skipped".to_owned());
    }
    for m in models {
        model::check_model(&mut sink, m);
    }
    for (r, ctx) in doc.reports.iter().zip(reports) {
        report::check_report(&mut sink, r, &ctx.folder, ctx.binding, &ctx.ext);
    }
    sink.finish()
}

/// Report-level measures declared in a PBIR `definition/reportExtensions.json`.
fn extension_measures(source: &ReportSource) -> report::ExtensionMeasures {
    let mut out = report::ExtensionMeasures::new();
    let ReportSource::Pbir { report, .. } = source else {
        return out;
    };
    let Some(file) = report.files.get("definition/reportExtensions.json") else {
        return out;
    };
    let Ok(json) = serde_json::from_slice::<Value>(&file.bytes) else {
        return out;
    };
    for entity in json
        .get("entities")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(table) = entity.get("name").and_then(Value::as_str) else {
            continue;
        };
        for m in entity
            .get("measures")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(name) = m.get("name").and_then(Value::as_str) {
                out.insert((table.to_lowercase(), name.to_lowercase()));
            }
        }
    }
    out
}

/// Binds a report to one of `models` (`(folder, model)`; a `None` folder
/// matches any `byPath`) through its dataset reference.
fn bind<'a>(
    doc: &Document,
    report: &Report,
    folder: &str,
    models: &[(Option<&str>, &'a Model)],
) -> Binding<'a> {
    match &report.dataset_reference {
        DatasetReference::ByConnection { .. } => Binding::Remote,
        DatasetReference::ByPath { path } => {
            let raw = path.replace('\\', "/").trim_end_matches('/').to_owned();
            let resolved = resolve_relative(folder, path).unwrap_or_else(|| raw.clone());
            let hit = models
                .iter()
                .find(|(f, _)| {
                    f.is_none_or(|f| {
                        let f = f.trim_end_matches('/');
                        f == resolved || f == raw
                    })
                })
                .map(|(_, m)| *m);
            match hit {
                Some(m) => Binding::Model(m),
                None => {
                    let prefix = format!("{resolved}/");
                    let exists = !resolved.is_empty()
                        && doc.source.entries.iter().any(|e| e.starts_with(&prefix));
                    if exists {
                        Binding::Missing
                    } else {
                        Binding::Unresolved
                    }
                }
            }
        }
        DatasetReference::Embedded | DatasetReference::None => match models.first() {
            Some((_, m)) if doc.model.is_some() => Binding::Model(m),
            _ => Binding::Missing,
        },
    }
}

/// `base_dir` joined with the relative path `rel`; `None` if `rel` climbs
/// above the container root.
fn resolve_relative(base_dir: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = base_dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in rel.split(['/', '\\']) {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

/// Report folder derived from the report's origin
/// (`X.Report/definition/report.json` → `X.Report`).
fn report_folder(report: &Report) -> String {
    let Some(origin) = &report.origin else {
        return String::new();
    };
    let path = origin.path.as_str();
    for suffix in ["definition/report.json", "report.json"] {
        if let Some(rest) = path.strip_suffix(suffix) {
            return rest.trim_end_matches('/').to_owned();
        }
    }
    path.rsplit_once('/')
        .map_or_else(String::new, |(dir, _)| dir.to_owned())
}

/// Where an expression's text starts relative to its object's
/// `origin.line`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineBase {
    /// TMDL `measure X = expr` / `expression X = expr`: single-line values
    /// sit on the declaration line, multi-line values start on the next one.
    Declaration,
    /// Unknown (e.g. a partition's `source` property): keep the origin line
    /// and give the expression-relative position in the message.
    Unknown,
}

/// One DAX or M expression to check.
pub(crate) struct ExprSite<'a> {
    /// What owns the expression, e.g. ``measure `Orders[Total Sales]` ``.
    pub what: String,
    pub expression: &'a str,
    pub origin: Option<&'a Origin>,
    pub line_base: LineBase,
    pub object: ObjectId,
}

impl ExprSite<'_> {
    /// Origin of an expression-relative 1-based `(line, column)`, plus that
    /// position as text for the message.
    pub(crate) fn locate(&self, (line, col): (usize, usize)) -> (Option<Origin>, String) {
        let position = format!("line {line}, column {col}");
        let Some(origin) = self.origin else {
            return (None, position);
        };
        let mut o = origin.clone();
        if self.line_base == LineBase::Declaration
            && let Some(base) = origin.line
        {
            let rel = u32::try_from(line).unwrap_or(u32::MAX);
            o.line = Some(if self.expression.contains('\n') {
                base.saturating_add(rel)
            } else {
                base.saturating_add(rel.saturating_sub(1))
            });
        }
        (Some(o), position)
    }
}

/// `{folder}/{file}` (or just `file` for the container root).
pub(crate) fn join(folder: &str, file: &str) -> String {
    if folder.is_empty() {
        file.to_owned()
    } else {
        format!("{}/{file}", folder.trim_end_matches('/'))
    }
}

/// Escapes one JSON-pointer reference token (`~` → `~0`, `/` → `~1`).
pub(crate) fn pointer_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_resolve_like_the_detector() {
        assert_eq!(
            resolve_relative(
                "Acme Outfitters - Stores.Report",
                "../AcmeOutfitters.SemanticModel"
            )
            .as_deref(),
            Some("AcmeOutfitters.SemanticModel")
        );
        assert_eq!(resolve_relative("", "../X.SemanticModel"), None);
        assert_eq!(
            resolve_relative("a/b.Report", "..\\m.SemanticModel").as_deref(),
            Some("a/m.SemanticModel")
        );
    }

    #[test]
    fn report_folders_come_from_origins() {
        let mut r = Report {
            origin: Some(Origin::file("X.Report/definition/report.json")),
            ..Report::default()
        };
        assert_eq!(report_folder(&r), "X.Report");
        r.origin = Some(Origin::file("definition/report.json"));
        assert_eq!(report_folder(&r), "");
        r.origin = Some(Origin::file("Report/Layout"));
        assert_eq!(report_folder(&r), "Report");
    }

    #[test]
    fn options_filter_rules_and_severities() {
        let opts = LintOptions {
            disabled_rules: vec!["lint016".into()],
            min_severity: Severity::Warning,
            layout: true,
        };
        let mut sink = Sink::new(&opts);
        sink.push(Finding::new(rules::VISUAL_OVERLAP, Severity::Info, "a"));
        sink.push(Finding::new(rules::STALE_QUERY_REF, Severity::Info, "b"));
        sink.push(Finding::new(rules::STALE_QUERY_REF, Severity::Warning, "c"));
        let report = sink.finish();
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.summary.warnings, 1);
        assert!(report.fails(Severity::Warning));
        assert!(!report.fails(Severity::Error));
        assert!(
            !LintOptions {
                layout: false,
                ..LintOptions::default()
            }
            .is_enabled(rules::VISUAL_BOUNDS)
        );
    }

    #[test]
    fn findings_convert_to_diagnostics() {
        let f = Finding::new(rules::MISSING_FIELD, Severity::Error, "column missing")
            .at(Some(Origin::file("a.json").with_pointer("/x")))
            .on(ObjectId(7))
            .suggest(Some("did you mean `T[C]`?".into()));
        let d = Diagnostic::from(&f);
        assert_eq!(d.code, "LINT002");
        assert_eq!(d.message, "column missing (did you mean `T[C]`?)");
        assert_eq!(d.object, Some(ObjectId(7)));
        assert_eq!(
            Finding::new(rules::MISSING_FIELD, Severity::Error, "x")
                .on(ObjectId::UNASSIGNED)
                .object,
            None
        );
        assert!(rule("lint001").is_some());
        assert_eq!(RULES.len(), 17);
    }
}
