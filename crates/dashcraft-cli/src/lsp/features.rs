//! The LSP requests and diagnostics, each over a registry command.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use pbi_ops::commands::dax::{
    DaxCompleteOut, DaxCompletionKind, DaxDiagnosticsOut, DaxFormatOut, DaxHoverOut,
    DaxReferencesOut, DaxSignatureHelpOut, DaxTokenKind, DaxTokensOut,
};
use pbi_ops::commands::m::{
    MCompleteOut, MDiagnosticsOut, MFormatOut, MHoverOut, MNameKind, MSignatureHelpOut,
};
use pbi_ops::commands::tmdl::{
    TmdlCompleteOut, TmdlDiagnosticsOut, TmdlFormatOut, TmdlHoverOut, TmdlItemKind,
};
use serde_json::{Value, json};

use super::text::{self, Language, LineIndex, Position, Range};
use super::{Document, RpcError, Server, files, method_not_found, publish};

/// Most completion items returned.
const MAX_COMPLETIONS: usize = 200;

/// Semantic token types, in legend order.
const TOKEN_TYPES: [&str; 11] = [
    "keyword",
    "function",
    "class",
    "property",
    "method",
    "variable",
    "parameter",
    "string",
    "number",
    "comment",
    "operator",
];
/// Semantic token modifiers, in legend order.
const TOKEN_MODIFIERS: [&str; 2] = ["declaration", "defaultLibrary"];
const MOD_DECLARATION: u32 = 1;
const MOD_DEFAULT_LIBRARY: u32 = 2;

/// What the server offers, for `initialize`.
pub(super) fn capabilities() -> Value {
    json!({
        "positionEncoding": "utf-16",
        "textDocumentSync": {
            "openClose": true,
            "change": 2,
            "save": {"includeText": false},
        },
        "completionProvider": {"triggerCharacters": ["[", "'", "#", ".", ":"]},
        "hoverProvider": true,
        "signatureHelpProvider": {
            "triggerCharacters": ["(", ","],
            "retriggerCharacters": [")"],
        },
        "definitionProvider": true,
        "referencesProvider": true,
        "documentHighlightProvider": true,
        "documentFormattingProvider": true,
        "codeActionProvider": {"codeActionKinds": ["quickfix"]},
        "semanticTokensProvider": {
            "legend": {"tokenTypes": TOKEN_TYPES, "tokenModifiers": TOKEN_MODIFIERS},
            "full": true,
        },
    })
}

fn severity(s: &str) -> u8 {
    match s {
        "error" => 1,
        "warning" => 2,
        _ => 3,
    }
}

/// One LSP diagnostic. `replacement` is the text a quick fix puts in `range`.
fn diagnostic(
    range: Range,
    severity_name: &str,
    code: &str,
    message: String,
    replacement: Option<String>,
) -> Value {
    let mut d = json!({
        "range": range,
        "severity": severity(severity_name),
        "code": code,
        "source": "dashcraft",
        "message": message,
    });
    if let (Some(r), Some(obj)) = (replacement, d.as_object_mut()) {
        obj.insert("data".to_owned(), json!({"replacement": r}));
    }
    d
}

/// A message with its suggested replacement named, unless it already names it.
fn with_suggestion(message: &str, suggestion: Option<&str>) -> String {
    match suggestion {
        Some(s) if !s.is_empty() && !message.contains(s) => {
            format!("{} Did you mean `{s}`?", message.trim_end())
        }
        _ => message.to_owned(),
    }
}

/// A markdown `MarkupContent`.
fn markdown(value: &str) -> Value {
    json!({"kind": "markdown", "value": value})
}

/// The text that replaces `range_text` for a DAX "did you mean `X`?" suggestion, when `X`
/// fits it: a function name as is, the `[Column]` part for a bracketed name, a table name
/// quoted when it needs to be.
fn dax_replacement(code: &str, range_text: &str, suggestion: &str) -> Option<String> {
    let x = suggestion.split('`').nth(1).filter(|x| !x.is_empty())?;
    if code == "unsupportedFunction" {
        return Some(x.to_owned());
    }
    if range_text.starts_with('[') {
        return bracket_part(x).map(str::to_owned);
    }
    if x.contains('[') || range_text.contains('[') {
        return None;
    }
    let simple = x
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && x.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    Some(if simple && !range_text.starts_with('\'') {
        x.to_owned()
    } else {
        format!("'{}'", x.replace('\'', "''"))
    })
}

/// The `[Column]` part of `Table[Column]`, `'My Table'[Column]` or `[Measure]`.
fn bracket_part(x: &str) -> Option<&str> {
    let bytes = x.as_bytes();
    let mut i = 0;
    if bytes.first() == Some(&b'\'') {
        i = 1;
        loop {
            match bytes.get(i) {
                Some(b'\'') if bytes.get(i + 1) == Some(&b'\'') => i += 2,
                Some(b'\'') => break,
                Some(_) => i += 1,
                None => return None,
            }
        }
    }
    let open = x.get(i..)?.find('[')?.checked_add(i)?;
    x.get(open..).filter(|part| part.ends_with(']'))
}

fn completion_kind_dax(kind: DaxCompletionKind) -> u8 {
    match kind {
        DaxCompletionKind::Function | DaxCompletionKind::UserFunction => 3,
        DaxCompletionKind::Table => 7,
        DaxCompletionKind::Column => 5,
        DaxCompletionKind::Measure => 2,
        DaxCompletionKind::Variable => 6,
        DaxCompletionKind::Keyword => 14,
    }
}

fn completion_kind_m(kind: MNameKind) -> u8 {
    match kind {
        MNameKind::Step | MNameKind::Parameter | MNameKind::Member | MNameKind::QueryParameter => 6,
        MNameKind::Field => 5,
        MNameKind::Query => 9,
        MNameKind::QueryFunction | MNameKind::Function => 3,
        MNameKind::Table => 7,
        MNameKind::Type => 25,
        MNameKind::Constant => 21,
        MNameKind::Keyword => 14,
    }
}

fn completion_kind_tmdl(kind: TmdlItemKind) -> u8 {
    match kind {
        TmdlItemKind::Object | TmdlItemKind::ObjectType => 14,
        TmdlItemKind::Property => 10,
        TmdlItemKind::Value => 20,
        TmdlItemKind::Table => 7,
        TmdlItemKind::Column => 5,
        TmdlItemKind::Measure => 2,
        TmdlItemKind::Hierarchy => 22,
        TmdlItemKind::Name => 18,
    }
}

/// One completion item replacing `range`.
fn completion_item(
    rank: usize,
    label: String,
    kind: u8,
    detail: String,
    insert_text: String,
    range: Range,
    documentation: Option<String>,
) -> Value {
    let mut item = json!({
        "label": label,
        "kind": kind,
        "detail": detail,
        "sortText": format!("{rank:04}"),
        "textEdit": {"range": range, "newText": insert_text},
    });
    if let (Some(doc), Some(obj)) = (documentation, item.as_object_mut()) {
        obj.insert("documentation".to_owned(), markdown(&doc));
    }
    item
}

fn token_type(kind: DaxTokenKind) -> (u32, u32) {
    match kind {
        DaxTokenKind::Keyword => (0, 0),
        DaxTokenKind::Function => (1, MOD_DEFAULT_LIBRARY),
        DaxTokenKind::UserFunction => (1, 0),
        DaxTokenKind::Table => (2, 0),
        DaxTokenKind::Column => (3, 0),
        DaxTokenKind::Measure => (4, 0),
        DaxTokenKind::Variable => (5, 0),
        DaxTokenKind::Parameter | DaxTokenKind::QueryParameter => (6, 0),
        DaxTokenKind::String | DaxTokenKind::DateTime => (7, 0),
        DaxTokenKind::Number => (8, 0),
        DaxTokenKind::Comment => (9, 0),
        DaxTokenKind::Operator => (10, 0),
    }
}

/// Delta-encodes tokens given as (start, end, type, modifiers), splitting the ones that span
/// lines.
fn encode_tokens(doc: &Document, tokens: &[(usize, usize, u32, u32)]) -> Vec<u32> {
    let mut data = Vec::with_capacity(tokens.len() * 5);
    let (mut prev_line, mut prev_char) = (0u32, 0u32);
    for &(start, end, ty, mods) in tokens {
        let Range { start: s, end: e } = doc.range(start, end);
        for line in s.line..=e.line {
            let from = if line == s.line { s.character } else { 0 };
            let to = if line == e.line {
                e.character
            } else {
                doc.lines.line_length(&doc.text, line)
            };
            if to <= from {
                continue;
            }
            let delta_line = line - prev_line;
            let delta_char = if delta_line == 0 {
                from - prev_char
            } else {
                from
            };
            data.extend([delta_line, delta_char, to - from, ty, mods]);
            prev_line = line;
            prev_char = from;
        }
    }
    data
}

/// The line break a text uses first.
fn line_break(text: &str) -> &'static str {
    match text.find('\n') {
        Some(i) if i > 0 && text.as_bytes().get(i - 1) == Some(&b'\r') => "\r\n",
        _ => "\n",
    }
}

/// A formatter's `\n` output with the input's line breaks and final line break.
fn restore_breaks(input: &str, formatted: &str) -> String {
    let brk = line_break(input);
    let mut out = if brk == "\n" {
        formatted.to_owned()
    } else {
        formatted.replace('\n', brk)
    };
    if input.ends_with('\n') && !out.ends_with('\n') {
        out.push_str(brk);
    }
    out
}

/// Whether a TMDL text indents with spaces.
fn space_indented(text: &str) -> bool {
    text.lines()
        .find_map(|l| match l.as_bytes().first() {
            Some(b' ') => Some(true),
            Some(b'\t') => Some(false),
            _ => None,
        })
        .unwrap_or(false)
}

/// The key a relative path is compared by: case-insensitive on Windows.
fn path_key(rel: &str) -> String {
    if cfg!(windows) {
        rel.to_lowercase()
    } else {
        rel.to_owned()
    }
}

/// The TMDL files of a folder for the commands: the open ones first, as edited, then the
/// rest from disk.
struct Folder {
    /// `{path, text}` objects.
    files: Vec<Value>,
    /// Relative path key → URI of the open documents.
    open: HashMap<String, String>,
    /// Relative path key → text, of every file.
    texts: HashMap<String, String>,
}

impl Server {
    pub(super) fn feature(&mut self, method: &str, params: &Value) -> Result<Value, RpcError> {
        match method {
            "textDocument/completion" => self.completion(params),
            "textDocument/hover" => self.hover(params),
            "textDocument/signatureHelp" => self.signature_help(params),
            "textDocument/definition" => self.definition(params),
            "textDocument/references" => self.references(params),
            "textDocument/documentHighlight" => self.highlights(params),
            "textDocument/formatting" => self.formatting(params),
            "textDocument/codeAction" => Ok(code_actions(params)),
            "textDocument/semanticTokens/full" => self.semantic_tokens(params),
            _ => Err(method_not_found(method)),
        }
    }

    /// The open document a request is about.
    fn target(&self, params: &Value) -> Option<(&str, &Document)> {
        let uri = params
            .pointer("/textDocument/uri")
            .and_then(Value::as_str)?;
        self.docs.get_key_value(uri).map(|(u, d)| (u.as_str(), d))
    }

    fn folder(&self, root: &Path) -> Folder {
        let mut files = Vec::new();
        let mut open = HashMap::new();
        let mut texts = HashMap::new();
        for (uri, doc) in &self.docs {
            if doc.tmdl_root().as_deref() != Some(root) {
                continue;
            }
            let Some(rel) = doc.path.as_deref().and_then(|p| files::relative(root, p)) else {
                continue;
            };
            let key = path_key(&rel);
            if open.insert(key.clone(), uri.clone()).is_none() {
                files.push(json!({"path": rel, "text": doc.text}));
                texts.insert(key, doc.text.clone());
            }
        }
        for (rel, text) in self.disk.borrow_mut().folder(root) {
            let key = path_key(&rel);
            if !open.contains_key(&key) {
                files.push(json!({"path": rel, "text": text}));
                texts.insert(key, text);
            }
        }
        Folder { files, open, texts }
    }

    /// The `path` and `files` of `tmdl.complete` and `tmdl.hover` for a TMDL document.
    fn tmdl_context(&self, uri: &str, doc: &Document) -> (String, Vec<Value>, Option<PathBuf>) {
        if let Some(root) = doc.tmdl_root()
            && let Some(rel) = doc.path.as_deref().and_then(|p| files::relative(&root, p))
        {
            let folder = self.folder(&root);
            return (rel, folder.files, Some(root));
        }
        let name = text::uri_file_name(uri).unwrap_or_else(|| "file.tmdl".to_owned());
        (name, Vec::new(), None)
    }

    /// The diagnostics of a DAX or M document, or of a TMDL file outside a model folder.
    pub(super) fn diagnostics(&self, uri: &str, doc: &Document) -> Result<Vec<Value>, RpcError> {
        let doc_id = self.doc_id();
        Ok(match doc.language {
            Language::Dax => {
                let out: DaxDiagnosticsOut = self.run(
                    "dax.diagnostics",
                    json!({"docId": doc_id, "text": doc.text}),
                )?;
                out.diagnostics
                    .into_iter()
                    .map(|d| {
                        // A DAX suggestion is a sentence: "did you mean `X`?".
                        let replacement = d.suggestion.as_deref().and_then(|s| {
                            let range_text = doc.text.get(d.start.offset..d.end.offset)?;
                            dax_replacement(&d.code, range_text, s)
                        });
                        let message = match &d.suggestion {
                            Some(s) if !d.message.contains(s.as_str()) => {
                                format!("{} ({s})", d.message.trim_end())
                            }
                            _ => d.message,
                        };
                        let range = doc.range(d.start.offset, d.end.offset);
                        diagnostic(range, &d.severity, &d.code, message, replacement)
                    })
                    .collect()
            }
            Language::M => {
                let out: MDiagnosticsOut =
                    self.run("m.diagnostics", json!({"docId": doc_id, "text": doc.text}))?;
                out.diagnostics
                    .into_iter()
                    .map(|d| {
                        let message = with_suggestion(&d.message, d.suggestion.as_deref());
                        let range = doc.range(d.start.offset, d.end.offset);
                        diagnostic(range, &d.severity, &d.code, message, d.suggestion)
                    })
                    .collect()
            }
            Language::Tmdl => {
                let name = text::uri_file_name(uri).unwrap_or_else(|| "file.tmdl".to_owned());
                let out: TmdlDiagnosticsOut = self.run(
                    "tmdl.diagnostics",
                    json!({"files": [{"path": name, "text": doc.text}]}),
                )?;
                out.diagnostics
                    .into_iter()
                    .map(|d| {
                        let message = with_suggestion(&d.message, d.suggestion.as_deref());
                        let range = doc.range(d.start.offset, d.end.offset);
                        diagnostic(range, &d.severity, &d.code, message, d.suggestion)
                    })
                    .collect()
            }
        })
    }

    /// `publishDiagnostics` parameters for every open document of a TMDL folder, checked
    /// together so references across files resolve.
    pub(super) fn tmdl_folder_diagnostics(&self, root: &Path) -> Result<Vec<Value>, RpcError> {
        let folder = self.folder(root);
        let out: TmdlDiagnosticsOut =
            self.run("tmdl.diagnostics", json!({"files": folder.files}))?;
        let mut by_key: HashMap<String, Vec<Value>> = HashMap::new();
        for d in out.diagnostics {
            let key = path_key(&d.path);
            let Some(uri) = folder.open.get(&key) else {
                continue;
            };
            let Some(doc) = self.docs.get(uri) else {
                continue;
            };
            let message = with_suggestion(&d.message, d.suggestion.as_deref());
            let range = doc.range(d.start.offset, d.end.offset);
            by_key.entry(key).or_default().push(diagnostic(
                range,
                &d.severity,
                &d.code,
                message,
                d.suggestion,
            ));
        }
        let mut published = Vec::new();
        for (key, uri) in &folder.open {
            if let Some(doc) = self.docs.get(uri) {
                let list = by_key.remove(key).unwrap_or_default();
                published.push(publish(uri, doc, list));
            }
        }
        Ok(published)
    }

    fn completion(&self, params: &Value) -> Result<Value, RpcError> {
        let Some((uri, doc)) = self.target(params) else {
            return Ok(Value::Null);
        };
        let position = json!({"offset": doc.offset_at(params)?});
        let doc_id = self.doc_id();
        let (start, end, incomplete, items): (usize, usize, bool, Vec<_>) = match doc.language {
            Language::Dax => {
                let out: DaxCompleteOut = self.run(
                    "dax.complete",
                    json!({"docId": doc_id, "text": doc.text, "position": position,
                        "maxItems": MAX_COMPLETIONS}),
                )?;
                let items = out
                    .items
                    .into_iter()
                    .map(|i| {
                        let documentation = match (&i.table, i.description) {
                            (Some(t), Some(d)) => Some(format!("`{t}`\n\n{d}")),
                            (Some(t), None) => Some(format!("`{t}`")),
                            (None, d) => d,
                        };
                        (
                            i.label,
                            completion_kind_dax(i.kind),
                            i.detail,
                            i.insert_text,
                            documentation,
                        )
                    })
                    .collect();
                (out.start.offset, out.end.offset, out.incomplete, items)
            }
            Language::M => {
                let out: MCompleteOut = self.run(
                    "m.complete",
                    json!({"docId": doc_id, "text": doc.text, "position": position,
                        "maxItems": MAX_COMPLETIONS}),
                )?;
                let items = out
                    .items
                    .into_iter()
                    .map(|i| {
                        (
                            i.label,
                            completion_kind_m(i.kind),
                            i.detail,
                            i.insert_text,
                            i.description,
                        )
                    })
                    .collect();
                (out.start.offset, out.end.offset, out.incomplete, items)
            }
            Language::Tmdl => {
                let (path, files, _) = self.tmdl_context(uri, doc);
                let out: TmdlCompleteOut = self.run(
                    "tmdl.complete",
                    json!({"text": doc.text, "position": position, "path": path,
                        "files": files, "maxItems": MAX_COMPLETIONS}),
                )?;
                let items = out
                    .items
                    .into_iter()
                    .map(|i| {
                        (
                            i.label,
                            completion_kind_tmdl(i.kind),
                            i.detail,
                            i.insert_text,
                            i.description,
                        )
                    })
                    .collect();
                (out.start.offset, out.end.offset, out.incomplete, items)
            }
        };
        let range = doc.range(start, end);
        let items: Vec<Value> = items
            .into_iter()
            .enumerate()
            .map(|(rank, (label, kind, detail, insert, documentation))| {
                completion_item(rank, label, kind, detail, insert, range, documentation)
            })
            .collect();
        Ok(json!({"isIncomplete": incomplete, "items": items}))
    }

    fn hover(&self, params: &Value) -> Result<Value, RpcError> {
        let Some((uri, doc)) = self.target(params) else {
            return Ok(Value::Null);
        };
        let position = json!({"offset": doc.offset_at(params)?});
        let doc_id = self.doc_id();
        let found = match doc.language {
            Language::Dax => {
                let out: DaxHoverOut = self.run(
                    "dax.hover",
                    json!({"docId": doc_id, "text": doc.text, "position": position}),
                )?;
                out.hover
                    .map(|h| (h.markdown, h.start.offset, h.end.offset))
            }
            Language::M => {
                let out: MHoverOut = self.run(
                    "m.hover",
                    json!({"docId": doc_id, "text": doc.text, "position": position}),
                )?;
                out.hover
                    .map(|h| (h.markdown, h.start.offset, h.end.offset))
            }
            Language::Tmdl => {
                let (path, files, _) = self.tmdl_context(uri, doc);
                let out: TmdlHoverOut = self.run(
                    "tmdl.hover",
                    json!({"text": doc.text, "position": position, "path": path,
                        "files": files}),
                )?;
                out.hover
                    .map(|h| (h.markdown, h.start.offset, h.end.offset))
            }
        };
        Ok(match found {
            Some((md, start, end)) => {
                json!({"contents": markdown(&md), "range": doc.range(start, end)})
            }
            None => Value::Null,
        })
    }

    fn signature_help(&self, params: &Value) -> Result<Value, RpcError> {
        let Some((_, doc)) = self.target(params) else {
            return Ok(Value::Null);
        };
        let position = json!({"offset": doc.offset_at(params)?});
        let signature = match doc.language {
            Language::Dax => {
                let out: DaxSignatureHelpOut = self.run(
                    "dax.signatureHelp",
                    json!({"text": doc.text, "position": position}),
                )?;
                out.signature.map(|s| {
                    // `label` is `signature` and the return type: the offsets hold in both.
                    let label = if s.label.starts_with(&s.signature) {
                        s.label
                    } else {
                        s.signature
                    };
                    let mut documentation = format!("{}\n\n*{}*", s.description, s.category);
                    if !s.supported {
                        documentation
                            .push_str("\n\n\u{26a0} DashCraft cannot evaluate this function yet.");
                    }
                    let parameters: Vec<Value> = s
                        .parameters
                        .iter()
                        .map(|p| json!({"label": [p.label_start, p.label_end]}))
                        .collect();
                    let active = s.active_parameter.unwrap_or(parameters.len());
                    (label, Some(documentation), parameters, active)
                })
            }
            Language::M => {
                let out: MSignatureHelpOut = self.run(
                    "m.signatureHelp",
                    json!({"docId": self.doc_id(), "text": doc.text, "position": position}),
                )?;
                out.signature.map(|s| {
                    let documentation = (!s.supported).then(|| {
                        "\u{26a0} DashCraft cannot evaluate this function yet.".to_owned()
                    });
                    let parameters: Vec<Value> = s
                        .parameters
                        .iter()
                        .map(|p| json!({"label": [p.label_start, p.label_end]}))
                        .collect();
                    let active = s.active_parameter.unwrap_or(parameters.len());
                    (s.label, documentation, parameters, active)
                })
            }
            Language::Tmdl => None,
        };
        Ok(match signature {
            Some((label, documentation, parameters, active)) => {
                let mut sig = json!({"label": label, "parameters": parameters});
                if let (Some(d), Some(obj)) = (documentation, sig.as_object_mut()) {
                    obj.insert("documentation".to_owned(), markdown(&d));
                }
                json!({"signatures": [sig], "activeSignature": 0, "activeParameter": active})
            }
            None => Value::Null,
        })
    }

    /// `dax.references` at the request's position.
    fn dax_references(&self, doc: &Document, params: &Value) -> Result<DaxReferencesOut, RpcError> {
        let position = json!({"offset": doc.offset_at(params)?});
        self.run(
            "dax.references",
            json!({"docId": self.doc_id(), "text": doc.text, "position": position}),
        )
    }

    fn definition(&self, params: &Value) -> Result<Value, RpcError> {
        let Some((uri, doc)) = self.target(params) else {
            return Ok(Value::Null);
        };
        match doc.language {
            Language::Dax => {
                let out = self.dax_references(doc, params)?;
                Ok(match out.definition {
                    Some(d) => {
                        json!({"uri": uri, "range": doc.range(d.start.offset, d.end.offset)})
                    }
                    None => Value::Null,
                })
            }
            Language::Tmdl => self.tmdl_definition(uri, doc, params),
            Language::M => Ok(Value::Null),
        }
    }

    fn tmdl_definition(
        &self,
        uri: &str,
        doc: &Document,
        params: &Value,
    ) -> Result<Value, RpcError> {
        let position = json!({"offset": doc.offset_at(params)?});
        let (path, files, root) = self.tmdl_context(uri, doc);
        let out: TmdlHoverOut = self.run(
            "tmdl.hover",
            json!({"text": doc.text, "position": position, "path": path, "files": files}),
        )?;
        let Some(decl) = out.hover.and_then(|h| h.declaration) else {
            return Ok(Value::Null);
        };
        let line = decl.line.saturating_sub(1);
        let same_file = decl
            .path
            .as_deref()
            .is_none_or(|p| path_key(p) == path_key(&path));
        if same_file {
            let end = doc.lines.line_length(&doc.text, line);
            return Ok(location(uri, line, end));
        }
        let (Some(root), Some(rel)) = (root, decl.path) else {
            return Ok(Value::Null);
        };
        let folder = self.folder(&root);
        let key = path_key(&rel);
        let end = folder
            .texts
            .get(&key)
            .map_or(0, |t| LineIndex::new(t).line_length(t, line));
        if let Some(open) = folder.open.get(&key) {
            return Ok(location(open, line, end));
        }
        let file = rel.split('/').fold(root, |p, part| p.join(part));
        Ok(match text::path_to_uri(&file) {
            Some(target) => location(&target, line, end),
            None => Value::Null,
        })
    }

    fn references(&self, params: &Value) -> Result<Value, RpcError> {
        let Some((uri, doc)) = self.target(params) else {
            return Ok(Value::Null);
        };
        if doc.language != Language::Dax {
            return Ok(Value::Null);
        }
        let include_declaration = params
            .pointer("/context/includeDeclaration")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let out = self.dax_references(doc, params)?;
        let list: Vec<Value> = out
            .references
            .iter()
            .filter(|r| include_declaration || !r.declaration)
            .map(|r| json!({"uri": uri, "range": doc.range(r.start.offset, r.end.offset)}))
            .collect();
        Ok(Value::Array(list))
    }

    fn highlights(&self, params: &Value) -> Result<Value, RpcError> {
        let Some((_, doc)) = self.target(params) else {
            return Ok(Value::Null);
        };
        if doc.language != Language::Dax {
            return Ok(Value::Null);
        }
        let out = self.dax_references(doc, params)?;
        let list: Vec<Value> = out
            .references
            .iter()
            .map(|r| {
                let kind = if r.declaration { 3 } else { 2 };
                json!({"range": doc.range(r.start.offset, r.end.offset), "kind": kind})
            })
            .collect();
        Ok(Value::Array(list))
    }

    fn formatting(&self, params: &Value) -> Result<Value, RpcError> {
        let Some((_, doc)) = self.target(params) else {
            return Ok(Value::Null);
        };
        let tab_size = params
            .pointer("/options/tabSize")
            .and_then(Value::as_u64)
            .map_or(4, |n| n.clamp(1, 16));
        let insert_spaces = params
            .pointer("/options/insertSpaces")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let formatted = match doc.language {
            Language::Dax => {
                let out: DaxFormatOut = self.run(
                    "dax.format",
                    json!({"text": doc.text, "indentWidth": tab_size, "useTabs": !insert_spaces}),
                )?;
                out.ok.then(|| restore_breaks(&doc.text, &out.text))
            }
            Language::M => {
                let out: MFormatOut = self.run(
                    "m.format",
                    json!({"text": doc.text, "indentWidth": tab_size, "useTabs": !insert_spaces}),
                )?;
                out.ok.then(|| restore_breaks(&doc.text, &out.text))
            }
            Language::Tmdl => {
                // Tabs, as Desktop writes, unless the file already indents with spaces.
                let indent = (insert_spaces && space_indented(&doc.text)).then_some(tab_size);
                let out: TmdlFormatOut = self.run(
                    "tmdl.format",
                    json!({"text": doc.text, "indentWidth": indent}),
                )?;
                out.ok.then_some(out.text)
            }
        };
        Ok(match formatted {
            Some(text) if text != doc.text => {
                json!([{"range": doc.lines.whole(&doc.text), "newText": text}])
            }
            _ => json!([]),
        })
    }

    fn semantic_tokens(&self, params: &Value) -> Result<Value, RpcError> {
        let Some((_, doc)) = self.target(params) else {
            return Ok(Value::Null);
        };
        if doc.language != Language::Dax {
            return Ok(json!({"data": []}));
        }
        let out: DaxTokensOut = self.run(
            "dax.tokens",
            json!({"docId": self.doc_id(), "text": doc.text}),
        )?;
        let tokens: Vec<(usize, usize, u32, u32)> = out
            .tokens
            .iter()
            .map(|t| {
                let (ty, mut mods) = token_type(t.kind);
                if t.declaration {
                    mods |= MOD_DECLARATION;
                }
                (t.start.offset, t.end.offset, ty, mods)
            })
            .collect();
        Ok(json!({"data": encode_tokens(doc, &tokens)}))
    }
}

/// A location covering line `line` up to character `end`.
fn location(uri: &str, line: u32, end: u32) -> Value {
    let range = Range {
        start: Position { line, character: 0 },
        end: Position {
            line,
            character: end,
        },
    };
    json!({"uri": uri, "range": range})
}

/// Quick fixes for the client's diagnostics that carry a replacement.
fn code_actions(params: &Value) -> Value {
    let wants_quickfix = params
        .pointer("/context/only")
        .and_then(Value::as_array)
        .is_none_or(|only| {
            only.iter()
                .filter_map(Value::as_str)
                .any(|k| k.is_empty() || k == "quickfix")
        });
    let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) else {
        return json!([]);
    };
    if !wants_quickfix {
        return json!([]);
    }
    let diagnostics = params
        .pointer("/context/diagnostics")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let mut seen = HashSet::new();
    let mut actions = Vec::new();
    for d in diagnostics {
        if d.get("source").and_then(Value::as_str) != Some("dashcraft") {
            continue;
        }
        let (Some(replacement), Some(range)) = (
            d.pointer("/data/replacement").and_then(Value::as_str),
            d.get("range"),
        ) else {
            continue;
        };
        if !seen.insert((range.to_string(), replacement.to_owned())) {
            continue;
        }
        actions.push(json!({
            "title": format!("Change to `{replacement}`"),
            "kind": "quickfix",
            "diagnostics": [d],
            "isPreferred": true,
            "edit": {"changes": {uri: [{"range": range, "newText": replacement}]}},
        }));
    }
    Value::Array(actions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_line_breaks_after_formatting() {
        assert_eq!(restore_breaks("a\r\nb\r\n", "a\nb"), "a\r\nb\r\n");
        assert_eq!(restore_breaks("a\nb", "a\n  b"), "a\n  b");
        assert_eq!(restore_breaks("ab\n", "ab"), "ab\n");
        assert_eq!(line_break("x"), "\n");
        assert!(space_indented("table A\n  column B\n"));
        assert!(!space_indented("table A\n\tcolumn B\n"));
        assert!(!space_indented("table A\n"));
    }

    #[test]
    fn suggestions_are_named_once() {
        assert_eq!(
            with_suggestion("The name 'X' wasn't recognized.", Some("Y")),
            "The name 'X' wasn't recognized. Did you mean `Y`?"
        );
        assert_eq!(
            with_suggestion(
                "unknown property, did you mean `dataType`?",
                Some("dataType")
            ),
            "unknown property, did you mean `dataType`?"
        );
        assert_eq!(with_suggestion("m", None), "m");
    }

    #[test]
    fn dax_suggestions_become_replacements_that_fit_the_range() {
        let fix = |code, range, s| dax_replacement(code, range, s);
        assert_eq!(
            fix(
                "unsupportedFunction",
                "PERCENTILE.IN",
                "did you mean `PERCENTILE.INC`?"
            ),
            Some("PERCENTILE.INC".to_owned())
        );
        assert_eq!(
            fix(
                "unknownColumn",
                "[Quantty]",
                "did you mean `Orders[Quantity]`?"
            ),
            Some("[Quantity]".to_owned())
        );
        assert_eq!(
            fix(
                "unknownColumn",
                "[Nme]",
                "did you mean `'It''s [x]'[Name]`?"
            ),
            Some("[Name]".to_owned())
        );
        assert_eq!(
            fix(
                "unknownMeasure",
                "[Total Salse]",
                "did you mean `[Total Sales]`?"
            ),
            Some("[Total Sales]".to_owned())
        );
        assert_eq!(
            fix("unknownTable", "Ordrs", "did you mean table `Orders`?"),
            Some("Orders".to_owned())
        );
        assert_eq!(
            fix("unknownTable", "'Ordrs'", "did you mean table `Orders`?"),
            Some("'Orders'".to_owned())
        );
        assert_eq!(
            fix(
                "unknownTable",
                "Prodct",
                "did you mean table `Product Category`?"
            ),
            Some("'Product Category'".to_owned())
        );
        assert_eq!(
            fix("unknownTable", "Ordrs", "did you mean `Orders[Qty]`?"),
            None
        );
        assert_eq!(fix("unknownColumn", "[Qty]", "no idea"), None);
    }

    #[test]
    fn quick_fixes_respect_the_requested_kinds() {
        let diag = json!({"range": {"start": {"line": 0, "character": 0},
            "end": {"line": 0, "character": 3}}, "source": "dashcraft",
            "data": {"replacement": "Text.Upper"}});
        let params = |only: Value| {
            json!({"textDocument": {"uri": "file:///q.m"},
                "context": {"diagnostics": [diag.clone(), diag.clone()], "only": only}})
        };
        let all = code_actions(&params(Value::Null));
        assert_eq!(all.as_array().map(Vec::len), Some(1));
        assert_eq!(
            all.pointer("/0/edit/changes/file:~1~1~1q.m/0/newText"),
            Some(&json!("Text.Upper"))
        );
        assert_eq!(code_actions(&params(json!(["refactor"]))), json!([]));
        assert_eq!(
            code_actions(&params(json!(["quickfix"])))
                .as_array()
                .map(Vec::len),
            Some(1)
        );
    }
}
