//! The TMDL language service (M7.4): `tmdl.diagnostics`, `tmdl.complete` and `tmdl.hover`.

use pbi_core::tmdl_service::{
    self, FileText, FormatError, FormatOptions, IndentStyle, ItemKind, LineEnding,
    MAX_INDENT_WIDTH, Source, TmdlDiagnostic,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::dax::{MAX_COMPLETION_ITEMS, TextPositionIn, TextPositionOut, default_max_items};
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

// ---------------------------------------------------------------------------

/// What a suggestion or a hovered word is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum TmdlItemKind {
    /// A keyword that starts an object (`column`, `measure`).
    Object,
    /// A property keyword (`dataType`, `isHidden`).
    Property,
    /// A value of an enumeration, or `true` / `false`.
    Value,
    /// The object type of a top-level `ref` line.
    ObjectType,
    Table,
    Column,
    Measure,
    Hierarchy,
    /// Another declared object: a query group, relationship, role, culture, perspective,
    /// expression, data source or function.
    Name,
}

impl From<ItemKind> for TmdlItemKind {
    fn from(kind: ItemKind) -> Self {
        match kind {
            ItemKind::Object => Self::Object,
            ItemKind::Property => Self::Property,
            ItemKind::Value => Self::Value,
            ItemKind::ObjectType => Self::ObjectType,
            ItemKind::Table => Self::Table,
            ItemKind::Column => Self::Column,
            ItemKind::Measure => Self::Measure,
            ItemKind::Hierarchy => Self::Hierarchy,
            ItemKind::Name => Self::Name,
        }
    }
}

/// The other files of the folder: `files` without the one at `path` (the edited text
/// replaces it).
fn other_files<'a>(
    path: Option<&str>,
    files: &'a [TmdlFileIn],
) -> Result<Vec<FileText<'a>>, OpError> {
    bounded("files", files.len(), MAX_TMDL_FILES)?;
    Ok(files
        .iter()
        .filter(|f| Some(f.path.as_str()) != path)
        .map(|f| FileText {
            path: &f.path,
            text: &f.text,
        })
        .collect())
}

/// `tmdl.complete`.
#[derive(Debug)]
pub struct TmdlComplete;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TmdlCompleteParams {
    /// The TMDL text being edited.
    pub text: String,
    /// The cursor.
    pub position: TextPositionIn,
    /// The text's path relative to the `definition` folder; the file of `files` with this path
    /// is replaced by `text`.
    #[serde(default)]
    pub path: Option<String>,
    /// The other files of the model's `definition` folder: their tables, columns, measures and
    /// other objects are suggested where a reference names one, and their `database` sets the
    /// compatibility level.
    #[serde(default)]
    pub files: Vec<TmdlFileIn>,
    /// Most items returned, best first (default 100, at most 1000).
    #[serde(default = "default_max_items")]
    pub max_items: usize,
}

/// One suggestion.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TmdlCompletionItemOut {
    /// What is shown (`dataType`, `'Order Lines'.Quantity`).
    pub label: String,
    pub kind: TmdlItemKind,
    /// The text that replaces `start`..`end`: a keyword with `: `, ` ` or ` = ` after it on an
    /// otherwise empty line, a name quoted as TMDL writes it.
    pub insert_text: String,
    /// A keyword's forms (`dataType: <value>`), a value's enumeration, what a name is.
    pub detail: String,
    /// What a keyword takes and needs, or a declared object's `///` description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TmdlCompleteOut {
    /// Start of the word a chosen item replaces (the cursor when nothing is typed yet).
    pub start: TextPositionOut,
    /// End (exclusive) of the replaced word.
    pub end: TextPositionOut,
    /// The typed text the items match.
    pub prefix: String,
    /// Best match first: the same word, then words that start with the prefix, then a word
    /// part that does, then words that contain it. Keywords are those of the object the line
    /// belongs to, accepted at the database's compatibility level, without properties already
    /// set or properties after a child object; after `keyword:` the values of its type; after
    /// a reference keyword the objects it may name. Empty inside expressions.
    pub items: Vec<TmdlCompletionItemOut>,
    /// More items matched than `maxItems`.
    pub incomplete: bool,
}

impl Command for TmdlComplete {
    type Params = TmdlCompleteParams;
    type Output = TmdlCompleteOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "tmdl.complete",
            title: "TMDL completion",
            summary: "The TMDL keywords, values and object names that fit at a cursor, as Microsoft's TMDL reader accepts them there, best match first, with the text to insert.",
            category: Category::Tmdl,
            examples: &[
                (
                    "The properties of a column",
                    r#"{"text": "table Stores\n\tcolumn Region\n\t\tdata", "position": {"line": 3, "column": 7}}"#,
                ),
                (
                    "A column of another file",
                    r#"{"text": "table Stores\n\tcolumn Region\n\t\tsortByColumn: ", "position": {"line": 3, "column": 17}, "path": "tables/Stores.tmdl", "files": [{"path": "tables/Stores.tmdl", "text": ""}, {"path": "tables/More.tmdl", "text": "table Stores\n\tcolumn 'Region Order'\n"}]}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(_ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let max_items = bounded("maxItems", p.max_items, MAX_COMPLETION_ITEMS)?;
        if max_items == 0 {
            return Err(OpError::invalid_params("maxItems must be at least 1"));
        }
        let others = other_files(p.path.as_deref(), &p.files)?;
        let src = p.text.as_str();
        let offset = p.position.resolve(src)?;
        let c = tmdl_service::complete(src, offset, &others, max_items);
        Ok(TmdlCompleteOut {
            start: TextPositionOut::at(src, c.start),
            end: TextPositionOut::at(src, c.end),
            prefix: c.prefix,
            items: c
                .items
                .into_iter()
                .map(|i| TmdlCompletionItemOut {
                    label: i.label,
                    kind: i.kind.into(),
                    insert_text: i.insert_text,
                    detail: i.detail,
                    description: i.description,
                })
                .collect(),
            incomplete: c.incomplete,
        })
    }
}

// ---------------------------------------------------------------------------

/// `tmdl.hover`.
#[derive(Debug)]
pub struct TmdlHover;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TmdlHoverParams {
    /// The TMDL text.
    pub text: String,
    /// The cursor.
    pub position: TextPositionIn,
    /// The text's path relative to the `definition` folder; the file of `files` with this path
    /// is replaced by `text`.
    #[serde(default)]
    pub path: Option<String>,
    /// The other files of the model's `definition` folder, where referenced objects are
    /// declared.
    #[serde(default)]
    pub files: Vec<TmdlFileIn>,
}

/// Where a referenced object is declared.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TmdlDeclarationOut {
    /// The file's `path`; `path` of the parameters (or absent) for the text itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// 1-based line of the declaration.
    pub line: u32,
}

/// The word under the cursor.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TmdlHoverInfo {
    pub kind: TmdlItemKind,
    /// The keyword, the value, or the object's name (`'Order Lines'.Quantity`).
    pub name: String,
    /// Start of the hovered word.
    pub start: TextPositionOut,
    /// End (exclusive).
    pub end: TextPositionOut,
    /// A keyword's forms, value type and compatibility level; an enumeration's values; a
    /// referenced object's `///` description and declaration line, as Markdown.
    pub markdown: String,
    /// Where the object a reference names is declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declaration: Option<TmdlDeclarationOut>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TmdlHoverOut {
    /// Absent when there is nothing to describe at the cursor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hover: Option<TmdlHoverInfo>,
}

impl Command for TmdlHover {
    type Params = TmdlHoverParams;
    type Output = TmdlHoverOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "tmdl.hover",
            title: "TMDL hover",
            summary: "Describes the TMDL word at a cursor: a keyword's forms, value and level, an enumeration value, or the object a reference names and where it is declared.",
            category: Category::Tmdl,
            examples: &[
                (
                    "A property keyword",
                    r#"{"text": "table Stores\n\tcolumn Region\n\t\tsummarizeBy: none\n", "position": {"line": 3, "column": 5}}"#,
                ),
                (
                    "A column of another file",
                    r#"{"text": "relationship r1\n\tfromColumn: Orders.Store\n", "position": {"line": 2, "column": 22}, "files": [{"path": "tables/Orders.tmdl", "text": "table Orders\n\tcolumn Store\n"}]}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(_ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let others = other_files(p.path.as_deref(), &p.files)?;
        let src = p.text.as_str();
        let offset = p.position.resolve(src)?;
        let hover = tmdl_service::hover(src, offset, &others).map(|h| TmdlHoverInfo {
            kind: h.kind.into(),
            name: h.name,
            start: TextPositionOut::at(src, h.start),
            end: TextPositionOut::at(src, h.end),
            markdown: h.markdown,
            declaration: h.declaration.map(|d| TmdlDeclarationOut {
                path: match d.file {
                    None => p.path.clone(),
                    Some(i) => others.get(i).map(|f| f.path.to_owned()),
                },
                line: d.line,
            }),
        });
        Ok(TmdlHoverOut { hover })
    }
}

// ---------------------------------------------------------------------------

/// `tmdl.format`.
#[derive(Debug)]
pub struct TmdlFormat;

/// A line ending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TmdlLineEnding {
    /// `\n`.
    Lf,
    /// `\r\n`, as Power BI Desktop writes on Windows.
    Crlf,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TmdlFormatParams {
    /// One `.tmdl` file's text.
    pub text: String,
    /// Indent with this many spaces per level (1 to 16); absent: with tabs, as Power BI
    /// Desktop does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indent_width: Option<usize>,
    /// The line ending of every line; absent: the text's first one (`lf` when it has none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_ending: Option<TmdlLineEnding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TmdlFormatOut {
    /// The text parses (else `text` is the input unchanged and `diagnostic` says why).
    pub ok: bool,
    /// The formatted text; every line ends with a line ending.
    pub text: String,
    /// `text` differs from the input.
    pub changed: bool,
    /// The syntax error (`TMDL110`) when `ok` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<TmdlDiagnosticOut>,
}

impl Command for TmdlFormat {
    type Params = TmdlFormatParams;
    type Output = TmdlFormatOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "tmdl.format",
            title: "Format TMDL",
            summary: "Lays out a TMDL file as Power BI Desktop writes it: indentation, spacing, blank lines, expression blocks; only white space changes, checked by reading the result back.",
            category: Category::Tmdl,
            examples: &[
                (
                    "A table file",
                    r#"{"text": "table Stores\n  lineageTag:s1\n  column  Region\n    dataType :string\n  measure Count=COUNTROWS(Stores)\n"}"#,
                ),
                (
                    "Two-space indentation, CRLF",
                    r#"{"text": "table Stores\n\tcolumn Region\n\t\tdataType: string\n", "indentWidth": 2, "lineEnding": "crlf"}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(_ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let indent = match p.indent_width {
            None => IndentStyle::Tabs,
            Some(0) => {
                return Err(OpError::invalid_params(
                    "indentWidth is 0; give 1 to 16 spaces, or leave it out for tabs",
                ));
            }
            Some(n) => {
                let n = bounded("indentWidth", n, usize::from(MAX_INDENT_WIDTH))?;
                IndentStyle::Spaces(u8::try_from(n).unwrap_or(MAX_INDENT_WIDTH))
            }
        };
        let options = FormatOptions {
            indent,
            line_ending: p.line_ending.map(|e| match e {
                TmdlLineEnding::Lf => LineEnding::Lf,
                TmdlLineEnding::Crlf => LineEnding::CrLf,
            }),
        };
        match tmdl_service::format(&p.text, &options) {
            Ok(text) => Ok(TmdlFormatOut {
                ok: true,
                changed: text != p.text,
                text,
                diagnostic: None,
            }),
            Err(FormatError::Syntax(d)) => Ok(TmdlFormatOut {
                ok: false,
                changed: false,
                diagnostic: Some(TmdlDiagnosticOut::new(&p.text, *d)),
                text: p.text,
            }),
            Err(e @ FormatError::Unstable) => Err(OpError::internal(e.to_string())),
        }
    }
}
