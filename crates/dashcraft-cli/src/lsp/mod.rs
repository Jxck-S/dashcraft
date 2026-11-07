//! `dashcraft lsp`: a Language Server Protocol server for `.dax`, `.m` and `.tmdl` files in
//! external editors, over the language commands of the registry (`dax.*`, `m.*`, `tmdl.*`;
//! ADR-0007). It speaks LSP 3.17 on stdin/stdout with `Content-Length` framing; positions are
//! UTF-16, as LSP's default.
//!
//! Features: diagnostics (published when the input goes idle), completion, hover, signature
//! help, whole-document formatting, quick fixes for the "did you mean" suggestions of
//! diagnostics; for DAX also semantic tokens, go to definition, references and highlights; for
//! TMDL go to definition across the files of the model's `definition` folder, which is read
//! from disk (open files win). DAX and M names resolve against the model of `--model` (or the
//! `model` initialization option), opened with `doc.open` and opened again when a file under
//! its folder is saved.
//!
//! Requests are answered in order on one thread; a request cancelled before it is reached
//! gets `RequestCancelled`. Nothing is run, nothing is sent over the network, and only the
//! TMDL folder of an open file and the model are read.

mod features;
mod files;
mod text;
mod transport;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use anyhow::{Context as _, anyhow};
use pbi_ops::commands::{dax, m, tmdl, workspace};
use pbi_ops::{CallerKind, Ctx, Registry, Workspace};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use self::files::DiskFiles;
use self::text::{Language, LineIndex, Position, Range};
use self::transport::Frame;

/// What `dashcraft lsp` was started with.
#[derive(Debug, Default)]
pub struct Options {
    /// The model DAX and M names resolve against.
    pub model: Option<PathBuf>,
}

/// How the session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    /// `exit` after `shutdown`, or the end of the input after `shutdown`.
    Clean,
    /// `exit` or the end of the input without `shutdown`.
    WithoutShutdown,
}

// JSON-RPC and LSP error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;
const SERVER_NOT_INITIALIZED: i64 = -32002;
const REQUEST_CANCELLED: i64 = -32800;

/// `window/showMessage` types.
const MESSAGE_WARNING: u8 = 2;

/// Most cancelled request ids remembered.
const MAX_CANCELLED: usize = 4096;

/// An error answer to a request.
#[derive(Debug)]
struct RpcError {
    code: i64,
    message: String,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(INVALID_PARAMS, message)
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new(INTERNAL_ERROR, message)
    }
}

/// The commands the server runs.
fn registry() -> Registry {
    Registry::new()
        .with::<workspace::DocOpen>()
        .with::<workspace::DocClose>()
        .with::<dax::DaxDiagnostics>()
        .with::<dax::DaxComplete>()
        .with::<dax::DaxHover>()
        .with::<dax::DaxSignatureHelp>()
        .with::<dax::DaxFormat>()
        .with::<dax::DaxTokens>()
        .with::<dax::DaxReferences>()
        .with::<m::MDiagnostics>()
        .with::<m::MComplete>()
        .with::<m::MHover>()
        .with::<m::MSignatureHelp>()
        .with::<m::MFormat>()
        .with::<tmdl::TmdlDiagnostics>()
        .with::<tmdl::TmdlComplete>()
        .with::<tmdl::TmdlHover>()
        .with::<tmdl::TmdlFormat>()
}

/// An open text document.
#[derive(Debug)]
struct Document {
    language: Language,
    text: String,
    lines: LineIndex,
    version: Value,
    /// The local file, for `file:` URIs.
    path: Option<PathBuf>,
}

impl Document {
    fn new(language: Language, text: String, version: Value, uri: &str) -> Self {
        Self {
            language,
            lines: LineIndex::new(&text),
            text,
            version,
            path: text::uri_to_path(uri),
        }
    }

    fn range(&self, start: usize, end: usize) -> Range {
        self.lines.range(&self.text, start, end)
    }

    /// The UTF-8 offset of the request's `position`.
    fn offset_at(&self, params: &Value) -> Result<usize, RpcError> {
        let pos: Position = params
            .get("position")
            .cloned()
            .and_then(|p| serde_json::from_value(p).ok())
            .ok_or_else(|| RpcError::invalid_params("missing or invalid `position`"))?;
        Ok(self.lines.offset(&self.text, pos))
    }

    /// The model folder of a TMDL file.
    fn tmdl_root(&self) -> Option<PathBuf> {
        (self.language == Language::Tmdl)
            .then(|| self.path.as_deref().and_then(files::tmdl_root))
            .flatten()
    }
}

/// The model `--model` opened.
#[derive(Debug)]
struct OpenModel {
    path: PathBuf,
    /// Saving a file under this folder opens the model again.
    root: PathBuf,
    doc_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Uninitialized,
    Running,
    ShutDown,
}

/// What the loop does after a message.
enum Flow {
    Continue,
    Exit,
}

/// What the reader thread passes on.
enum Incoming {
    Message(Result<Value, serde_json::Error>),
    TooLarge(usize),
    End(Option<std::io::Error>),
}

struct Server {
    registry: Registry,
    ws: Workspace,
    ctx: Ctx,
    rt: tokio::runtime::Runtime,
    state: State,
    model: Option<OpenModel>,
    docs: BTreeMap<String, Document>,
    /// Open documents whose diagnostics are out of date.
    dirty: BTreeSet<String>,
    /// Requests cancelled before they were reached (their ids as JSON).
    cancelled: HashSet<String>,
    disk: RefCell<DiskFiles>,
    /// Messages to send after the current one.
    outbox: Vec<Value>,
}

impl Server {
    fn new(opts: &Options) -> anyhow::Result<Self> {
        let mut server = Self {
            registry: registry(),
            ws: Workspace::new(),
            ctx: Ctx::new(CallerKind::Cli),
            rt: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("cannot start the async runtime")?,
            state: State::Uninitialized,
            model: None,
            docs: BTreeMap::new(),
            dirty: BTreeSet::new(),
            cancelled: HashSet::new(),
            disk: RefCell::new(DiskFiles::default()),
            outbox: Vec::new(),
        };
        if let Some(path) = &opts.model {
            server
                .open_model(path)
                .map_err(|e| anyhow!("--model {}: {e}", path.display()))?;
        }
        Ok(server)
    }

    /// Runs command `id` and reads its result.
    fn run<T: DeserializeOwned>(&self, id: &str, params: Value) -> Result<T, RpcError> {
        let value = self
            .rt
            .block_on(self.registry.execute(&self.ws, id, params, &self.ctx))
            .map_err(|e| RpcError::internal(format!("{id}: {}", e.message)))?;
        serde_json::from_value(value)
            .map_err(|e| RpcError::internal(format!("{id}: unexpected result: {e}")))
    }

    fn doc_id(&self) -> Option<&str> {
        self.model.as_ref().map(|m| m.doc_id.as_str())
    }

    /// Opens the model at `path` (again), closing the one open before.
    fn open_model(&mut self, path: &Path) -> Result<(), String> {
        let path = std::path::absolute(path).map_err(|e| e.to_string())?;
        let text = path
            .to_str()
            .ok_or_else(|| "the path is not valid Unicode".to_owned())?;
        let summary: Value = self
            .run(
                "doc.open",
                json!({"path": text, "autoRefresh": false, "loadPbixSchema": true}),
            )
            .map_err(|e| e.message)?;
        let doc_id = summary
            .get("docId")
            .and_then(Value::as_str)
            .ok_or_else(|| "doc.open returned no docId".to_owned())?
            .to_owned();
        let root = if path.is_dir() {
            path.clone()
        } else {
            path.parent().map(Path::to_path_buf).unwrap_or_default()
        };
        tracing::info!(model = %path.display(), %doc_id, "opened the model");
        if let Some(old) = self.model.replace(OpenModel { path, root, doc_id }) {
            let closed: Result<Value, RpcError> = self.run(
                "doc.close",
                json!({"docId": old.doc_id, "discardChanges": true}),
            );
            if let Err(e) = closed {
                tracing::warn!("cannot close the previous model: {}", e.message);
            }
        }
        Ok(())
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.outbox
            .push(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    fn show_message(&mut self, kind: u8, message: String) {
        self.notify(
            "window/showMessage",
            json!({"type": kind, "message": message}),
        );
    }

    fn reply(&mut self, id: Value, result: Result<Value, RpcError>) {
        self.outbox.push(match result {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(e) => json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": e.code, "message": e.message}}),
        });
    }

    /// Notes the `$/cancelRequest`s waiting in `queue`.
    fn note_cancels(&mut self, queue: &VecDeque<Incoming>) {
        for item in queue {
            if let Incoming::Message(Ok(msg)) = item
                && msg.get("method").and_then(Value::as_str) == Some("$/cancelRequest")
                && let Some(id) = msg.pointer("/params/id")
            {
                self.cancelled.insert(id.to_string());
            }
        }
    }

    fn handle(&mut self, msg: Value) -> Flow {
        let Some(method) = msg.get("method").and_then(Value::as_str).map(str::to_owned) else {
            // A response: the server sends no requests.
            return Flow::Continue;
        };
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        match msg.get("id").cloned() {
            Some(id) => {
                if self.cancelled.remove(&id.to_string()) {
                    self.reply(
                        id,
                        Err(RpcError::new(
                            REQUEST_CANCELLED,
                            "the request was cancelled",
                        )),
                    );
                } else {
                    let result = self.request(&method, &params);
                    self.reply(id, result);
                }
                Flow::Continue
            }
            None => self.notification(&method, &params),
        }
    }

    fn request(&mut self, method: &str, params: &Value) -> Result<Value, RpcError> {
        match (self.state, method) {
            (State::Uninitialized, "initialize") => Ok(self.initialize(params)),
            (State::Uninitialized, _) => Err(RpcError::new(
                SERVER_NOT_INITIALIZED,
                "the server is not initialized yet",
            )),
            (State::ShutDown, _) => Err(RpcError::new(INVALID_REQUEST, "the server is shut down")),
            (State::Running, "initialize") => Err(RpcError::new(
                INVALID_REQUEST,
                "the server is already initialized",
            )),
            (State::Running, "shutdown") => {
                self.state = State::ShutDown;
                Ok(Value::Null)
            }
            (State::Running, _) => self.feature(method, params),
        }
    }

    fn initialize(&mut self, params: &Value) -> Value {
        self.state = State::Running;
        let option = params
            .pointer("/initializationOptions/model")
            .and_then(Value::as_str);
        if let (None, Some(model)) = (&self.model, option) {
            let mut path = PathBuf::from(model);
            if path.is_relative()
                && let Some(base) = workspace_root(params)
            {
                path = base.join(path);
            }
            if let Err(e) = self.open_model(&path) {
                self.show_message(
                    MESSAGE_WARNING,
                    format!(
                        "DashCraft cannot open the model {}: {e}. DAX and M names are not checked.",
                        path.display()
                    ),
                );
            }
        }
        json!({
            "capabilities": features::capabilities(),
            "serverInfo": {"name": "dashcraft", "version": env!("CARGO_PKG_VERSION")},
        })
    }

    fn notification(&mut self, method: &str, params: &Value) -> Flow {
        if method == "exit" {
            return Flow::Exit;
        }
        if self.state != State::Running {
            return Flow::Continue;
        }
        match method {
            "textDocument/didOpen" => self.did_open(params),
            "textDocument/didChange" => self.did_change(params),
            "textDocument/didClose" => self.did_close(params),
            "textDocument/didSave" => self.did_save(params),
            "$/cancelRequest" => {
                // Also for a request not read yet; ids are unique, so a stale one never matches.
                if self.cancelled.len() >= MAX_CANCELLED {
                    self.cancelled.clear();
                }
                if let Some(id) = params.get("id") {
                    self.cancelled.insert(id.to_string());
                }
            }
            _ => {}
        }
        Flow::Continue
    }

    fn did_open(&mut self, params: &Value) {
        let doc = params.get("textDocument");
        let field = |name: &str| doc.and_then(|d| d.get(name));
        let (Some(uri), Some(text)) = (
            field("uri").and_then(Value::as_str),
            field("text").and_then(Value::as_str),
        ) else {
            return;
        };
        let language_id = field("languageId").and_then(Value::as_str).unwrap_or("");
        let Some(language) = Language::detect(language_id, uri) else {
            return;
        };
        let version = field("version").cloned().unwrap_or(Value::Null);
        let doc = Document::new(language, text.to_owned(), version, uri);
        self.docs.insert(uri.to_owned(), doc);
        self.dirty.insert(uri.to_owned());
    }

    fn did_change(&mut self, params: &Value) {
        let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) else {
            return;
        };
        let Some(doc) = self.docs.get_mut(uri) else {
            return;
        };
        let changes = params
            .get("contentChanges")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        for change in changes {
            let Some(new_text) = change.get("text").and_then(Value::as_str) else {
                continue;
            };
            let range = change
                .get("range")
                .and_then(|r| serde_json::from_value::<Range>(r.clone()).ok());
            text::apply_change(&mut doc.text, &doc.lines, range, new_text);
            doc.lines = LineIndex::new(&doc.text);
        }
        if let Some(version) = params.pointer("/textDocument/version") {
            doc.version = version.clone();
        }
        self.dirty.insert(uri.to_owned());
    }

    fn did_close(&mut self, params: &Value) {
        let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) else {
            return;
        };
        let Some(doc) = self.docs.remove(uri) else {
            return;
        };
        self.dirty.remove(uri);
        self.notify(
            "textDocument/publishDiagnostics",
            json!({"uri": uri, "diagnostics": []}),
        );
        // The other files of the folder see the file on disk again.
        if let Some(root) = doc.tmdl_root() {
            let same: Vec<String> = self
                .docs
                .iter()
                .filter(|(_, d)| d.tmdl_root().as_deref() == Some(root.as_path()))
                .map(|(u, _)| u.clone())
                .collect();
            self.dirty.extend(same);
        }
    }

    fn did_save(&mut self, params: &Value) {
        let Some(path) = params
            .pointer("/textDocument/uri")
            .and_then(Value::as_str)
            .and_then(text::uri_to_path)
        else {
            return;
        };
        let Some(model) = &self.model else {
            return;
        };
        if !path.starts_with(&model.root) {
            return;
        }
        let model_path = model.path.clone();
        match self.open_model(&model_path) {
            Ok(()) => {
                let names: Vec<String> = self
                    .docs
                    .iter()
                    .filter(|(_, d)| d.language != Language::Tmdl)
                    .map(|(u, _)| u.clone())
                    .collect();
                self.dirty.extend(names);
            }
            Err(e) => self.show_message(
                MESSAGE_WARNING,
                format!(
                    "DashCraft cannot open the model {} again: {e}",
                    model_path.display()
                ),
            ),
        }
    }

    /// Publishes the diagnostics of the documents changed since the last time; `false` when
    /// there were none.
    fn publish_diagnostics(&mut self) -> bool {
        if self.dirty.is_empty() {
            return false;
        }
        let dirty = std::mem::take(&mut self.dirty);
        let mut roots_done: HashSet<PathBuf> = HashSet::new();
        let mut out = Vec::new();
        for uri in &dirty {
            let Some(doc) = self.docs.get(uri) else {
                continue;
            };
            let published = match doc.tmdl_root() {
                Some(root) if roots_done.insert(root.clone()) => {
                    self.tmdl_folder_diagnostics(&root)
                }
                Some(_) => Ok(Vec::new()),
                None => self
                    .diagnostics(uri, doc)
                    .map(|d| vec![publish(uri, doc, d)]),
            };
            match published {
                Ok(messages) => out.extend(messages),
                Err(e) => tracing::warn!("diagnostics of {uri}: {}", e.message),
            }
        }
        for message in out {
            self.notify("textDocument/publishDiagnostics", message);
        }
        true
    }
}

/// A `textDocument/publishDiagnostics` notification's parameters.
fn publish(uri: &str, doc: &Document, diagnostics: Vec<Value>) -> Value {
    json!({"uri": uri, "version": doc.version, "diagnostics": diagnostics})
}

/// The first workspace folder, else the root URI or path, of `initialize`.
fn workspace_root(params: &Value) -> Option<PathBuf> {
    let folder = params
        .pointer("/workspaceFolders/0/uri")
        .or_else(|| params.get("rootUri"))
        .and_then(Value::as_str)
        .and_then(text::uri_to_path);
    folder.or_else(|| {
        params
            .get("rootPath")
            .and_then(Value::as_str)
            .map(PathBuf::from)
    })
}

/// Serves LSP on `input` and `output` until `exit` or the end of the input.
pub fn serve<R, W>(input: R, mut output: W, opts: &Options) -> anyhow::Result<Ending>
where
    R: Read + Send + 'static,
    W: Write,
{
    let mut server = Server::new(opts)?;
    let (tx, rx) = mpsc::channel::<Incoming>();
    std::thread::Builder::new()
        .name("lsp-reader".to_owned())
        .spawn(move || {
            let mut reader = BufReader::new(input);
            loop {
                let item = match transport::read_message(&mut reader) {
                    Ok(Some(Frame::Body(body))) => Incoming::Message(serde_json::from_slice(&body)),
                    Ok(Some(Frame::TooLarge(n))) => Incoming::TooLarge(n),
                    Ok(None) => Incoming::End(None),
                    Err(e) => Incoming::End(Some(e)),
                };
                let end = matches!(item, Incoming::End(_));
                if tx.send(item).is_err() || end {
                    break;
                }
            }
        })
        .context("cannot start the reader thread")?;

    let mut queue: VecDeque<Incoming> = VecDeque::new();
    loop {
        queue.extend(rx.try_iter());
        let Some(item) = queue.pop_front() else {
            // Idle: diagnostics first, then wait.
            if server.publish_diagnostics() {
                flush(&mut server, &mut output)?;
                continue;
            }
            match rx.recv() {
                Ok(item) => queue.push_back(item),
                Err(_) => return Ok(ending(&server)),
            }
            continue;
        };
        match item {
            Incoming::Message(Ok(msg)) => {
                server.note_cancels(&queue);
                let flow = server.handle(msg);
                flush(&mut server, &mut output)?;
                if let Flow::Exit = flow {
                    return Ok(ending(&server));
                }
            }
            Incoming::Message(Err(e)) => {
                server.reply(
                    Value::Null,
                    Err(RpcError::new(PARSE_ERROR, format!("not JSON: {e}"))),
                );
                flush(&mut server, &mut output)?;
            }
            Incoming::TooLarge(n) => {
                tracing::warn!(
                    "a message of {n} bytes was dropped (at most {} bytes)",
                    transport::MAX_MESSAGE_BYTES
                );
            }
            Incoming::End(None) => return Ok(ending(&server)),
            Incoming::End(Some(e)) => {
                return Err(anyhow!("cannot read the client's messages: {e}"));
            }
        }
    }
}

fn ending(server: &Server) -> Ending {
    if server.state == State::ShutDown {
        Ending::Clean
    } else {
        Ending::WithoutShutdown
    }
}

fn flush(server: &mut Server, output: &mut impl Write) -> anyhow::Result<()> {
    for message in std::mem::take(&mut server.outbox) {
        transport::write_message(output, &message).context("cannot write to the client")?;
    }
    Ok(())
}

/// The method a request asks for, for errors.
fn method_not_found(method: &str) -> RpcError {
    RpcError::new(METHOD_NOT_FOUND, format!("unknown method `{method}`"))
}
