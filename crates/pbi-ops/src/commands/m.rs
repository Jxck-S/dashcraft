//! `m.diagnostics`: the M language service (M7.3).

use pbi_session::m_service::{self, Diagnostic, Environment};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::dax::TextPositionOut;
use super::{BASE, doc};
use crate::command::{Category, Command, CommandMeta};
use crate::ctx::Ctx;
use crate::error::OpError;
use crate::workspace::Workspace;

/// `m.diagnostics`.
#[derive(Debug)]
pub struct MDiagnostics;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MDiagnosticsParams {
    /// Check names against this document's queries and parameters (`d1`, …).
    /// Without it, a plain name that is not in the standard library may be
    /// another query and is not reported.
    #[serde(default)]
    pub doc_id: Option<String>,
    /// The M text: a query (`let … in …`, any expression) or a section
    /// document (`section Section1; …`).
    pub text: String,
}

/// One problem in the text.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MDiagnosticOut {
    /// `error` (Power BI Desktop rejects the query) or `warning` (valid M
    /// that DashCraft cannot evaluate).
    pub severity: String,
    /// `syntax`, `unknownName`, `argumentCount`, `unsupported` or
    /// `extensionName`.
    pub code: String,
    /// Desktop's message for errors.
    pub message: String,
    /// Start of the text the problem covers.
    pub start: TextPositionOut,
    /// End (exclusive).
    pub end: TextPositionOut,
    /// A "did you mean …" correction of an unknown name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MDiagnosticsOut {
    /// The document revision whose names were used (with `docId`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_revision: Option<u64>,
    /// No errors (warnings allowed).
    pub ok: bool,
    /// In source order, at most 1000.
    pub diagnostics: Vec<MDiagnosticOut>,
}

impl Command for MDiagnostics {
    type Params = MDiagnosticsParams;
    type Output = MDiagnosticsOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "m.diagnostics",
            title: "Check M",
            summary: "Syntax errors, unknown names, wrong argument counts and unsupported names in an M query or section, with Power BI Desktop's messages.",
            category: Category::M,
            examples: &[
                (
                    "A query against the model",
                    r#"{"docId": "d1", "text": "let Source = Table.RowCnt(Orders) in Source"}"#,
                ),
                (
                    "Without a document",
                    r#"{"text": "let a = Text.Upper() in a"}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let (document_revision, env) = match p.doc_id.as_deref() {
            Some(id) => {
                let capture = doc(ws, id)?.session().revision_snapshot();
                (
                    Some(capture.revision),
                    pbi_session::m_environment(&capture.model),
                )
            }
            None => (None, Environment::unknown()),
        };
        let src = p.text.as_str();
        let diagnostics: Vec<MDiagnosticOut> = m_service::diagnostics(src, &env)
            .into_iter()
            .map(|d| MDiagnosticOut::new(src, d))
            .collect();
        Ok(MDiagnosticsOut {
            document_revision,
            ok: diagnostics.iter().all(|d| d.severity != "error"),
            diagnostics,
        })
    }
}

impl MDiagnosticOut {
    fn new(src: &str, d: Diagnostic) -> Self {
        Self {
            severity: d.severity.as_str().to_owned(),
            code: d.code.as_str().to_owned(),
            message: d.message,
            start: TextPositionOut::at(src, d.span.start),
            end: TextPositionOut::at(src, d.span.end),
            suggestion: d.suggestion,
        }
    }
}
