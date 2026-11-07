//! The M language service (M7.3): `m.diagnostics`, `m.hover`, `m.signatureHelp` and
//! `m.complete`.

use pbi_session::m_service::{self, Diagnostic, Environment, NameKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::dax::{MAX_COMPLETION_ITEMS, TextPositionIn, TextPositionOut, default_max_items};
use super::{BASE, bounded, doc};
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
        let (document_revision, env) = environment(ws, p.doc_id.as_deref())?;
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

/// The names of `doc_id`'s model and its revision; without a document, unknown names.
fn environment(
    ws: &Workspace,
    doc_id: Option<&str>,
) -> Result<(Option<u64>, Environment), OpError> {
    Ok(match doc_id {
        Some(id) => {
            let capture = doc(ws, id)?.session().revision_snapshot();
            (
                Some(capture.revision),
                pbi_session::m_environment(&capture.model),
            )
        }
        None => (None, Environment::unknown()),
    })
}

/// UTF-16 code units in `text` before byte `offset`.
fn utf16_len(text: &str, offset: usize) -> usize {
    text.get(..offset).map_or(0, |t| t.encode_utf16().count())
}

/// What an M name is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum MNameKind {
    /// A `let` variable of the text.
    Step,
    /// A function parameter of the text (`_` of `each` too).
    Parameter,
    /// A field of a record expression of the text.
    Field,
    /// A member of the section document.
    Member,
    /// Another query of the model.
    Query,
    /// A Power Query parameter of the model.
    QueryParameter,
    /// A query of the model that is a function.
    QueryFunction,
    /// A table of the model loaded by an M query.
    Table,
    /// A standard-library function or a `#date`-style constructor.
    Function,
    /// A library type constant (`Int64.Type`) or a primitive type (`text`).
    Type,
    /// A library constant or other value (`Number.PI`, `Culture.Current`).
    Constant,
    /// A keyword (`let`, `each`, also `nullable` and `optional`).
    Keyword,
}

impl From<NameKind> for MNameKind {
    fn from(k: NameKind) -> Self {
        match k {
            NameKind::Step => Self::Step,
            NameKind::Parameter => Self::Parameter,
            NameKind::Field => Self::Field,
            NameKind::Member => Self::Member,
            NameKind::Query => Self::Query,
            NameKind::QueryParameter => Self::QueryParameter,
            NameKind::QueryFunction => Self::QueryFunction,
            NameKind::Table => Self::Table,
            NameKind::Function => Self::Function,
            NameKind::Type => Self::Type,
            NameKind::Constant => Self::Constant,
            NameKind::Keyword => Self::Keyword,
        }
    }
}

// ---------------------------------------------------------------------------

/// `m.signatureHelp`.
#[derive(Debug)]
pub struct MSignatureHelp;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MSignatureHelpParams {
    /// Resolve calls of this document's function queries (`d1`, …).
    #[serde(default)]
    pub doc_id: Option<String>,
    /// The M text: a query or a section document.
    pub text: String,
    /// The cursor.
    pub position: TextPositionIn,
}

/// One parameter of an M signature.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MParameterOut {
    pub name: String,
    /// `nullable text`; `any` when the function does not say.
    #[serde(rename = "type")]
    pub ty: String,
    /// May be left out.
    pub optional: bool,
    /// Start of `name as type` in `label`, in UTF-16 code units.
    pub label_start: usize,
    /// End (exclusive), in UTF-16 code units.
    pub label_end: usize,
}

/// The function call around the cursor.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MSignatureOut {
    /// The called name as M writes it (`Table.AddColumn`, `#date`, `#"My Function"`).
    pub name: String,
    /// `function` for the library and `#date`-style constructors; `step`, `parameter`,
    /// `field`, `member` or `queryFunction` for functions of the text or model.
    pub kind: MNameKind,
    /// `Text.Upper(text as nullable text, optional culture as nullable text) as nullable text`.
    pub label: String,
    /// The return type.
    pub returns: String,
    /// Whether DashCraft evaluates the function.
    pub supported: bool,
    pub parameters: Vec<MParameterOut>,
    /// 0-based index of the argument the cursor is in.
    pub argument: usize,
    /// `argument` when the function has that many parameters; absent past the last one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_parameter: Option<usize>,
    /// Start of the called name in the text.
    pub name_start: TextPositionOut,
    /// End of the called name (exclusive).
    pub name_end: TextPositionOut,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MSignatureHelpOut {
    /// The document revision whose queries were used (with `docId`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_revision: Option<u64>,
    /// Absent outside a call of a known function, or in a comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<MSignatureOut>,
}

impl Command for MSignatureHelp {
    type Params = MSignatureHelpParams;
    type Output = MSignatureHelpOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "m.signatureHelp",
            title: "M signature help",
            summary: "The M function call around a cursor: library function, #date-style constructor, function step or function query, its parameters and the one the cursor is in; works on unparsed text.",
            category: Category::M,
            examples: &[
                (
                    "The second argument of Table.AddColumn",
                    r#"{"text": "Table.AddColumn(Source, ", "position": {"offset": 24}}"#,
                ),
                (
                    "A function query of the model",
                    r#"{"docId": "d1", "text": "fnCleanKey(", "position": {"line": 1, "column": 12}}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let (document_revision, env) = environment(ws, p.doc_id.as_deref())?;
        let src = p.text.as_str();
        let offset = p.position.resolve(src)?;
        let signature = m_service::signature_help(src, offset, &env).map(|h| {
            let label = h.label;
            MSignatureOut {
                name: h.name,
                kind: h.kind.into(),
                returns: h.returns,
                supported: h.supported,
                parameters: h
                    .parameters
                    .into_iter()
                    .map(|p| MParameterOut {
                        label_start: utf16_len(&label, p.label.start),
                        label_end: utf16_len(&label, p.label.end),
                        name: p.name,
                        ty: p.ty,
                        optional: p.optional,
                    })
                    .collect(),
                label,
                argument: h.argument,
                active_parameter: h.active_parameter,
                name_start: TextPositionOut::at(src, h.name_span.start),
                name_end: TextPositionOut::at(src, h.name_span.end),
            }
        });
        Ok(MSignatureHelpOut {
            document_revision,
            signature,
        })
    }
}

// ---------------------------------------------------------------------------

/// `m.hover`.
#[derive(Debug)]
pub struct MHover;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MHoverParams {
    /// Describe this document's queries, parameters and tables (`d1`, …). Without it only the
    /// text's own names, the library and keywords are described.
    #[serde(default)]
    pub doc_id: Option<String>,
    /// The M text: a query or a section document.
    pub text: String,
    /// The cursor.
    pub position: TextPositionIn,
}

/// The name under the cursor.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MHoverInfo {
    pub kind: MNameKind,
    /// The name as M writes it (`#"Changed Type"`).
    pub name: String,
    /// Start of the hovered name in the text.
    pub start: TextPositionOut,
    /// End (exclusive).
    pub end: TextPositionOut,
    /// Everything below as Markdown (code in `powerquery` fences).
    pub markdown: String,
    /// A function's signature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// A parameter's type, a library type's or constant's kind, a query parameter's type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    /// An optional function parameter.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
    /// A library constant's value as an M literal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// The M text of a step, field, member or query (shortened past 2000 bytes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    /// What a keyword or primitive type is, or the model's description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether DashCraft evaluates the name.
    pub supported: bool,
    /// Why DashCraft cannot evaluate it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsupported: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MHoverOut {
    /// The document revision whose queries were used (with `docId`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_revision: Option<u64>,
    /// Absent when there is nothing to describe at the cursor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hover: Option<MHoverInfo>,
}

impl Command for MHover {
    type Params = MHoverParams;
    type Output = MHoverOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "m.hover",
            title: "M hover",
            summary: "Describes the M name at a cursor as Power BI Desktop binds it: step, parameter or field of the text, query, parameter or table of the model, library name or keyword.",
            category: Category::M,
            examples: &[
                (
                    "A query parameter of the model",
                    r#"{"docId": "d1", "text": "Web.Contents(StorageContainerUrl)", "position": {"offset": 15}}"#,
                ),
                (
                    "A library function, without a document",
                    r#"{"text": "Table.RowCount(Source)", "position": {"line": 1, "column": 3}}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let (document_revision, env) = environment(ws, p.doc_id.as_deref())?;
        let src = p.text.as_str();
        let offset = p.position.resolve(src)?;
        let hover = m_service::hover(src, offset, &env).map(|h| MHoverInfo {
            markdown: h.markdown(),
            kind: h.kind.into(),
            start: TextPositionOut::at(src, h.span.start),
            end: TextPositionOut::at(src, h.span.end),
            name: h.name,
            signature: h.signature,
            type_name: h.type_name,
            optional: h.optional,
            value: h.value,
            expression: h.expression,
            description: h.description,
            supported: h.supported,
            unsupported: h.unsupported,
        });
        Ok(MHoverOut {
            document_revision,
            hover,
        })
    }
}

// ---------------------------------------------------------------------------

/// `m.complete`.
#[derive(Debug)]
pub struct MComplete;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MCompleteParams {
    /// Suggest this document's queries, parameters and tables (`d1`, …). Without it only the
    /// text's own names, the library and keywords are suggested.
    #[serde(default)]
    pub doc_id: Option<String>,
    /// The M text: a query or a section document.
    pub text: String,
    /// The cursor.
    pub position: TextPositionIn,
    /// Most items returned, best first (default 100, at most 1000).
    #[serde(default = "default_max_items")]
    pub max_items: usize,
}

/// One suggestion.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MCompletionItemOut {
    /// The name (`Changed Type`, `Table.AddColumn`, `#date`, `let`).
    pub label: String,
    pub kind: MNameKind,
    /// The text that replaces `start`..`end`: the name as M writes it (`#"Changed Type"`).
    pub insert_text: String,
    /// A function's signature; otherwise `step`, `parameter (number)`, `query`,
    /// `query parameter (Text)`, `table`, `type (number)`, `constant (number)`, `keyword`.
    pub detail: String,
    /// What a keyword or primitive type is, or the model's description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether DashCraft evaluates the name.
    pub supported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MCompleteOut {
    /// The document revision whose queries were used (with `docId`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_revision: Option<u64>,
    /// Start of the text a chosen item replaces (the name being typed, a whole `#"…"` inside
    /// one; the cursor when nothing is typed yet).
    pub start: TextPositionOut,
    /// End (exclusive) of the replaced text.
    pub end: TextPositionOut,
    /// The typed text the items match (`Sou`, `Changed` in `#"Changed`).
    pub prefix: String,
    /// Best match first: the same name, then names that start with the prefix, then a word
    /// that does, then names that contain it; among equals the text's names, then the model's,
    /// then keywords and the library. Empty in a comment, text or number, where a step or field
    /// name is written, and in `[…]` field access.
    pub items: Vec<MCompletionItemOut>,
    /// More items matched than `maxItems`.
    pub incomplete: bool,
}

impl Command for MComplete {
    type Params = MCompleteParams;
    type Output = MCompleteOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "m.complete",
            title: "M completion",
            summary: "The M names that fit at a cursor, best match first, with the text to insert: steps and parameters in scope, the model's queries, keywords, library names, or types after as.",
            category: Category::M,
            examples: &[
                (
                    "A query of the model",
                    r#"{"docId": "d1", "text": "let Source = Ord", "position": {"offset": 16}}"#,
                ),
                (
                    "Library functions, without a document",
                    r#"{"text": "Table.Sel", "position": {"line": 1, "column": 10}, "maxItems": 10}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let max_items = bounded("maxItems", p.max_items, MAX_COMPLETION_ITEMS)?;
        if max_items == 0 {
            return Err(OpError::invalid_params("maxItems must be at least 1"));
        }
        let (document_revision, env) = environment(ws, p.doc_id.as_deref())?;
        let src = p.text.as_str();
        let offset = p.position.resolve(src)?;
        let c = m_service::complete(src, offset, &env, max_items);
        let items = c
            .items
            .into_iter()
            .map(|i| MCompletionItemOut {
                label: i.label,
                kind: i.kind.into(),
                insert_text: i.insert_text,
                detail: i.detail,
                description: i.description,
                supported: i.supported,
            })
            .collect();
        Ok(MCompleteOut {
            document_revision,
            start: TextPositionOut::at(src, c.span.start),
            end: TextPositionOut::at(src, c.span.end),
            prefix: c.prefix,
            items,
            incomplete: c.incomplete,
        })
    }
}
