//! The TMDL language service (M7.4): `tmdl.diagnostics`.

use pbi_core::tmdl_service::{self, Source, TmdlDiagnostic};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::dax::TextPositionOut;
use super::{BASE, bounded};
use crate::command::{Category, Command, CommandMeta};
use crate::ctx::Ctx;
use crate::error::OpError;
use crate::workspace::Workspace;

/// At most this many files per call.
pub const MAX_TMDL_FILES: usize = 10_000;

/// One TMDL file.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TmdlFileIn {
    /// The path relative to the model's `definition` folder (`tables/Orders.tmdl`); it names
    /// the file in diagnostics.
    pub path: String,
    /// The file's text.
    pub text: String,
}

/// `tmdl.diagnostics`.
#[derive(Debug)]
pub struct TmdlDiagnostics;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TmdlDiagnosticsParams {
    /// The files to check: a whole `definition` folder (references between files are then
    /// resolved) or single files (only their own structure and values are checked).
    pub files: Vec<TmdlFileIn>,
}

/// One problem in a file.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TmdlDiagnosticOut {
    /// The file's `path`.
    pub path: String,
    /// `error` (Microsoft's TMDL reader rejects the files) or `warning`.
    pub severity: String,
    /// `TMDL101` unknown keyword, `TMDL102` keyword of another object, `TMDL103` wrong form,
    /// `TMDL104` above the compatibility level, `TMDL105` invalid value, `TMDL106` duplicate
    /// property, `TMDL107` conflicting declarations, `TMDL108` unresolved reference, `TMDL109`
    /// property after a child object, `TMDL110` syntax, `TMDL111` lines under a property,
    /// `TMDL112` invalid `ref`, `TMDL113` second model; warnings: `TMDL120` `ref` to an
    /// undeclared object, the parser's `TMDL006` (orphan `///`) and `TMDL007` (mixed
    /// indentation).
    pub code: String,
    pub message: String,
    /// Start of the text the problem covers.
    pub start: TextPositionOut,
    /// End (exclusive).
    pub end: TextPositionOut,
    /// A replacement for the range: the closest keyword, value or name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TmdlDiagnosticsOut {
    /// No errors (warnings allowed).
    pub ok: bool,
    /// By file (in input order), then position; at most 1000.
    pub diagnostics: Vec<TmdlDiagnosticOut>,
}

impl Command for TmdlDiagnostics {
    type Params = TmdlDiagnosticsParams;
    type Output = TmdlDiagnosticsOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "tmdl.diagnostics",
            title: "Check TMDL",
            summary: "What Microsoft's TMDL reader rejects in TMDL files: unknown keywords, wrong forms and values, levels, duplicates and unresolved references, with the closest name.",
            category: Category::Tmdl,
            examples: &[
                (
                    "One table file",
                    r#"{"files": [{"path": "tables/Stores.tmdl", "text": "table Stores\n\tcolumn Region\n\t\tdataTyp: string\n"}]}"#,
                ),
                (
                    "A model folder",
                    r#"{"files": [{"path": "model.tmdl", "text": "model Model\n\tculture: en-US\n"}, {"path": "tables/Stores.tmdl", "text": "table Stores\n\tcolumn Region\n\t\tdataType: string\n\t\tsortByColumn: Regions\n"}]}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(_ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        bounded("files", p.files.len(), MAX_TMDL_FILES)?;
        let sources: Vec<Source> = p
            .files
            .into_iter()
            .map(|f| Source::new(f.path, f.text))
            .collect();
        let diagnostics: Vec<TmdlDiagnosticOut> = tmdl_service::diagnostics(&sources)
            .into_iter()
            .map(|d| {
                let text = sources.get(d.file).map_or("", |s| s.text.as_str());
                TmdlDiagnosticOut::new(text, d)
            })
            .collect();
        Ok(TmdlDiagnosticsOut {
            ok: diagnostics.iter().all(|d| d.severity != "error"),
            diagnostics,
        })
    }
}

impl TmdlDiagnosticOut {
    fn new(text: &str, d: TmdlDiagnostic) -> Self {
        Self {
            start: TextPositionOut::at(text, d.start),
            end: TextPositionOut::at(text, d.end),
            path: d.path,
            severity: d.severity.as_str().to_owned(),
            code: d.code.to_owned(),
            message: d.message,
            suggestion: d.suggestion,
        }
    }
}
