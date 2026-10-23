//! `doc.lint`.

use pbi_core::lint::{LintOptions, lint};
use pbi_model::Severity;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{BASE, doc};
use crate::command::{Category, Command, CommandMeta};
use crate::ctx::Ctx;
use crate::error::OpError;
use crate::inspect_model::{
    DiagnosticOut, OriginOut, SCHEMA_VERSION, diagnostic_out, severity_name,
};
use crate::workspace::Workspace;

/// `doc.lint`.
#[derive(Debug)]
pub struct DocLint;

/// A diagnostic severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SeverityParam {
    Error,
    Warning,
    Info,
}

impl From<SeverityParam> for Severity {
    fn from(s: SeverityParam) -> Self {
        match s {
            SeverityParam::Error => Severity::Error,
            SeverityParam::Warning => Severity::Warning,
            SeverityParam::Info => Severity::Info,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocLintParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Leave out findings below this severity (default `info`: all).
    #[serde(default)]
    pub min_severity: Option<SeverityParam>,
    /// Rules to skip, e.g. `["LINT016"]`.
    #[serde(default)]
    pub disable: Vec<String>,
    /// Run the layout rules LINT015/LINT016 (default `true`).
    #[serde(default)]
    pub layout: Option<bool>,
    /// `ok` is `false` for findings at or above this severity (default
    /// `error`), as `dashcraft lint --deny`.
    #[serde(default)]
    pub deny: Option<SeverityParam>,
}

/// Counts per severity and rule.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LintSummaryOut {
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
    /// Findings per rule id.
    pub by_rule: std::collections::BTreeMap<String, usize>,
    /// Checks that were skipped and why.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// One finding.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FindingOut {
    /// Rule id, e.g. `LINT002` (see `crates/pbi-core/src/lint/RULES.md`).
    pub rule: String,
    /// `error`, `warning` or `info`.
    pub severity: String,
    pub message: String,
    /// Source file (container-relative) plus JSON pointer or line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<OriginOut>,
    /// A fix hint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

/// The `lint --json` report (`schemaVersion` 1) plus `ok`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LintOut {
    pub schema_version: u32,
    /// The document path as given.
    pub path: String,
    /// No finding at or above `deny` and no reader error.
    pub ok: bool,
    pub summary: LintSummaryOut,
    /// Findings sorted by file, location and rule.
    pub findings: Vec<FindingOut>,
    /// Reader errors (checks that could not run).
    pub reader_errors: Vec<DiagnosticOut>,
}

impl Command for DocLint {
    type Params = DocLintParams;
    type Output = LintOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "doc.lint",
            title: "Lint a document",
            summary: "Broken-reference checks (visual fields, queryRefs, filters, DAX/M, relationships, layout) as the `lint --json` report.",
            category: Category::Lint,
            examples: &[
                ("All rules", r#"{"docId": "d1"}"#),
                (
                    "Warnings and worse, no overlap rule",
                    r#"{"docId": "d1", "minSeverity": "warning", "disable": ["LINT016"]}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &p.doc_id)?;
        let mut opts = LintOptions {
            disabled_rules: p.disable.clone(),
            layout: p.layout.unwrap_or(true),
            ..LintOptions::default()
        };
        if let Some(min) = p.min_severity {
            opts.min_severity = min.into();
        }
        let session = entry.session().clone();
        let report = tokio::task::spawn_blocking(move || lint(session.opened(), &opts)).await?;
        let reader_errors: Vec<DiagnosticOut> = entry
            .session()
            .document()
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .map(diagnostic_out)
            .collect();
        let deny: Severity = p.deny.unwrap_or(SeverityParam::Error).into();
        Ok(LintOut {
            schema_version: SCHEMA_VERSION,
            path: entry.shown_path().to_owned(),
            ok: !report.fails(deny) && reader_errors.is_empty(),
            summary: LintSummaryOut {
                errors: report.summary.errors,
                warnings: report.summary.warnings,
                infos: report.summary.infos,
                by_rule: report.summary.by_rule.clone(),
                notes: report.summary.notes.clone(),
            },
            findings: report
                .findings
                .iter()
                .map(|f| FindingOut {
                    rule: f.rule.to_owned(),
                    severity: severity_name(f.severity).to_owned(),
                    message: f.message.clone(),
                    origin: f.origin.as_ref().map(|o| OriginOut {
                        path: o.path.clone(),
                        line: o.line,
                        pointer: o.pointer.clone(),
                    }),
                    suggestion: f.suggestion.clone(),
                })
                .collect(),
            reader_errors,
        })
    }
}
