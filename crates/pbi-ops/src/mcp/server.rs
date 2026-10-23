//! The MCP server loop: newline-delimited JSON-RPC 2.0 over a byte stream
//! (stdio), dispatching `tools/call` to [`Registry::execute`] with an
//! [`CallerKind::Agent`] caller.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufRead, AsyncWrite};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use super::protocol::{
    self, INVALID_PARAMS, INVALID_REQUEST, LATEST_LEGACY_VERSION, LogLevel, META_LOG_LEVEL,
    META_PROTOCOL_VERSION, META_SERVER_INFO, METHOD_NOT_FOUND, PARSE_ERROR, ProgressMeter,
    UNSUPPORTED_PROTOCOL_VERSION, at_least, is_modern, known_version, negotiate,
    supported_versions,
};
use super::tools::{ToolCatalog, ToolFilter, ToolNames};
use crate::ctx::{CallerKind, Ctx};
use crate::error::{OpError, OpErrorCode};
use crate::registry::Registry;
use crate::rpc::{DocQueue, Line, read_line, spawn_writer};
use crate::workspace::{Workspace, lock};

/// Default cap on the text of one tool result (1 MiB).
pub const DEFAULT_MAX_TEXT_BYTES: usize = 1024 * 1024;

/// Server settings.
#[derive(Debug, Clone)]
pub struct McpOptions {
    /// Tool naming (`doc_open` or `doc.open`).
    pub tool_names: ToolNames,
    /// Which agent-safe commands are exposed.
    pub filter: ToolFilter,
    /// Tool calls running at the same time.
    pub max_concurrent: usize,
    /// Tool calls in flight (running or queued) before `busy`.
    pub max_pending: usize,
    /// Longest message line in bytes.
    pub max_line_bytes: usize,
    /// Longest text of a tool result before it is cut.
    pub max_text_bytes: usize,
    /// Tools per `tools/list` page (0: all in one page).
    pub page_size: usize,
    /// `serverInfo.name`.
    pub server_name: String,
    /// `serverInfo.title`.
    pub server_title: String,
    /// `serverInfo.version`.
    pub server_version: String,
}

impl Default for McpOptions {
    fn default() -> Self {
        let rpc = crate::rpc::RpcOptions::default();
        Self {
            tool_names: ToolNames::default(),
            filter: ToolFilter::default(),
            max_concurrent: rpc.max_concurrent,
            max_pending: rpc.max_pending,
            max_line_bytes: rpc.max_line_bytes,
            max_text_bytes: DEFAULT_MAX_TEXT_BYTES,
            page_size: 100,
            server_name: "dashcraft".to_owned(),
            server_title: "DashCraft: Power BI files, offline".to_owned(),
            server_version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }
}

/// An MCP server over the agent catalog of a [`Registry`] and one
/// [`Workspace`] (documents stay open for the whole session).
#[derive(Debug)]
pub struct McpServer {
    tools: Arc<ToolCatalog>,
    ws: Arc<Workspace>,
    opts: McpOptions,
}

impl McpServer {
    /// A server exposing the agent-safe commands of `registry` (filtered by
    /// `opts.filter`) on `ws`.
    ///
    /// # Errors
    /// `invalidParams` when `ws` has an unrestricted path policy (agents
    /// must be confined to read/write roots), for a bad filter entry or a
    /// tool-name collision.
    pub fn new(registry: &Registry, ws: Arc<Workspace>, opts: McpOptions) -> Result<Self, OpError> {
        if !ws.policy().is_restricted() {
            return Err(OpError::invalid_params(
                "the MCP server needs a restricted path policy (read/write roots)",
            ));
        }
        let tools = ToolCatalog::new(registry, &opts.filter, opts.tool_names)?;
        Ok(Self {
            tools: Arc::new(tools),
            ws,
            opts,
        })
    }

    /// The exposed tools.
    #[must_use]
    pub fn tools(&self) -> &ToolCatalog {
        &self.tools
    }

    /// The `instructions` of `initialize`: how to start, and the roots.
    #[must_use]
    pub fn instructions(&self) -> String {
        let name = |id: &str| self.tools.name_of(id).map(|n| format!("`{n}`"));
        let roots = |r: Option<&[std::path::PathBuf]>| match r {
            Some([]) | None => "none".to_owned(),
            Some(r) => r
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
        };
        let policy = self.ws.policy();
        let mut out = String::from(
            "Reads Power BI files (PBIP, PBIX, PBIT, TMDL, …) offline: no network, no script execution.",
        );
        if let Some(open) = name("doc.open") {
            out.push_str(&format!(
                " Start with {open} {{\"path\": …}} (relative paths resolve against the first readable folder); \
                 it returns a docId that the other tools take. Documents stay open for the session."
            ));
        }
        let explore: Vec<String> = [
            "model.tables",
            "report.pages",
            "measures.values",
            "dax.query",
            "render.page",
        ]
        .iter()
        .filter_map(|id| name(id))
        .collect();
        if !explore.is_empty() {
            out.push_str(&format!(" Then use {}.", explore.join(", ")));
        }
        if let Some(render) = name("render.page") {
            out.push_str(&format!(
                " {render} with \"format\": \"png\" returns the page as an image."
            ));
        }
        out.push_str(&format!(
            " Readable folders: {}. Writable folders: {}.",
            roots(policy.read_roots()),
            roots(policy.write_roots())
        ));
        out
    }

    /// Serves messages read from `reader`, writing responses and
    /// notifications to `writer`, until end of input (which cancels the
    /// calls still running).
    ///
    /// # Errors
    /// I/O errors of the reader or writer.
    pub async fn serve<R, W>(self, mut reader: R, writer: W) -> std::io::Result<()>
    where
        R: AsyncBufRead + Unpin + Send,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (outbox, write_task) = spawn_writer(writer);
        let instructions = self.instructions();
        let shared = Arc::new(Shared {
            permits: Semaphore::new(self.opts.max_concurrent.max(1)),
            tools: self.tools,
            ws: self.ws,
            opts: self.opts,
            outbox,
            inflight: Mutex::new(HashMap::new()),
            queue: DocQueue::default(),
            session: Mutex::new(Session::default()),
            instructions,
        });
        let mut tasks = JoinSet::new();
        let mut buf = Vec::new();
        let read_result = loop {
            let text = match read_line(&mut reader, shared.opts.max_line_bytes, &mut buf).await {
                Ok(Line::Eof) => break Ok(()),
                Ok(Line::TooLong) => {
                    shared
                        .send(protocol::error(
                            &Value::Null,
                            INVALID_REQUEST,
                            "message line too long",
                            None,
                        ))
                        .await;
                    continue;
                }
                Ok(Line::Text(text)) => text,
                Err(e) => break Err(e),
            };
            let text = text.trim();
            if !text.is_empty() {
                handle_line(&shared, text, &mut tasks).await;
            }
            while tasks.try_join_next().is_some() {}
        };
        // The client is gone: stop the running calls (their responses
        // would not be read).
        for token in lock(&shared.inflight).values() {
            token.cancel();
        }
        while tasks.join_next().await.is_some() {}
        drop(shared);
        let written = write_task
            .await
            .unwrap_or_else(|e| Err(std::io::Error::other(e)));
        read_result.and(written)
    }
}

#[derive(Debug, Default)]
struct Session {
    /// Negotiated by `initialize`.
    version: Option<&'static str>,
    /// Set by `logging/setLevel`.
    log_level: Option<LogLevel>,
}

struct Shared {
    tools: Arc<ToolCatalog>,
    ws: Arc<Workspace>,
    opts: McpOptions,
    outbox: mpsc::Sender<String>,
    inflight: Mutex<HashMap<String, CancellationToken>>,
    queue: DocQueue,
    permits: Semaphore,
    session: Mutex<Session>,
    instructions: String,
}

/// How a request is served.
#[derive(Debug, Clone, Copy)]
struct Era {
    version: &'static str,
    /// Stateless revision: `resultType`, `_meta.serverInfo`, per-request
    /// log level.
    modern: bool,
    log_level: Option<LogLevel>,
}

/// A JSON-RPC error answer.
struct RpcError {
    code: i64,
    message: String,
    data: Option<Value>,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

impl Shared {
    async fn send(&self, line: String) {
        let _ = self.outbox.send(line).await;
    }

    fn server_info(&self, version: &str) -> Value {
        let mut info =
            json!({ "name": self.opts.server_name, "version": self.opts.server_version });
        if at_least(version, "2025-06-18")
            && let Value::Object(map) = &mut info
        {
            map.insert("title".into(), self.opts.server_title.clone().into());
        }
        info
    }

    fn capabilities(modern: bool) -> Value {
        if modern {
            json!({ "tools": { "listChanged": false } })
        } else {
            json!({ "tools": { "listChanged": false }, "logging": {} })
        }
    }

    /// Adds the fields every modern result carries.
    fn finish(&self, era: Era, mut result: Value) -> Value {
        if era.modern
            && let Value::Object(map) = &mut result
        {
            map.insert("resultType".into(), "complete".into());
            let meta = map
                .entry("_meta")
                .or_insert_with(|| Value::Object(Map::new()));
            if let Value::Object(meta) = meta {
                meta.insert(META_SERVER_INFO.into(), self.server_info(era.version));
            }
        }
        result
    }

    /// Sends `notifications/message` when `level` passes the threshold.
    fn log(&self, era: Era, level: LogLevel, data: Value) {
        if era.log_level.is_some_and(|min| level >= min) {
            let note = protocol::notification(
                "notifications/message",
                json!({ "level": level.as_str(), "logger": self.opts.server_name, "data": data }),
            );
            let _ = self.outbox.try_send(note);
        }
    }
}

async fn handle_line(shared: &Arc<Shared>, text: &str, tasks: &mut JoinSet<()>) {
    let msg: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            shared
                .send(protocol::error(
                    &Value::Null,
                    PARSE_ERROR,
                    &format!("parse error: {e}"),
                    None,
                ))
                .await;
            return;
        }
    };
    let Value::Object(mut obj) = msg else {
        shared
            .send(protocol::error(
                &Value::Null,
                INVALID_REQUEST,
                "a message must be a JSON object (batches are not supported)",
                None,
            ))
            .await;
        return;
    };
    let id = obj.remove("id");
    let reply_id = id.clone().unwrap_or(Value::Null);
    let invalid = |m: &str| protocol::error(&reply_id, INVALID_REQUEST, m, None);
    if obj.get("jsonrpc").is_some_and(|v| v != "2.0") {
        shared.send(invalid("jsonrpc must be \"2.0\"")).await;
        return;
    }
    if id
        .as_ref()
        .is_some_and(|i| !(i.is_string() || i.is_i64() || i.is_u64()))
    {
        shared
            .send(invalid("id must be a string or an integer"))
            .await;
        return;
    }
    let Some(method) = obj.get("method").and_then(Value::as_str).map(str::to_owned) else {
        if obj.contains_key("result") || obj.contains_key("error") {
            tracing::debug!("ignoring a response from the client");
        } else if id.is_some() {
            shared.send(invalid("missing method")).await;
        }
        return;
    };
    let params = match obj.remove("params") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(map)) => map,
        Some(_) => {
            if id.is_some() {
                shared
                    .send(protocol::error(
                        &reply_id,
                        INVALID_PARAMS,
                        "params must be an object",
                        None,
                    ))
                    .await;
            }
            return;
        }
    };
    let Some(id) = id else {
        notification(shared, &method, &params);
        return;
    };
    let era = match era_of(shared, &params) {
        Ok(era) => era,
        Err(e) => {
            shared
                .send(protocol::error(&id, e.code, &e.message, e.data))
                .await;
            return;
        }
    };
    let answer = match method.as_str() {
        "tools/call" => match start_call(shared, id.clone(), params, era, tasks) {
            Ok(()) => return,
            Err(e) => Err(e),
        },
        "initialize" => initialize(shared, &params),
        "ping" => Ok(json!({})),
        "server/discover" => Ok(json!({
            "supportedVersions": supported_versions(),
            "capabilities": Shared::capabilities(true),
            "instructions": shared.instructions,
            "ttlMs": 3_600_000,
            "cacheScope": "private",
        })),
        "tools/list" => {
            let cursor = params.get("cursor").and_then(Value::as_str);
            shared
                .tools
                .list(cursor, shared.opts.page_size, era.version)
                .map(|mut list| {
                    if era.modern
                        && let Value::Object(map) = &mut list
                    {
                        map.insert("ttlMs".into(), 3_600_000.into());
                        map.insert("cacheScope".into(), "private".into());
                    }
                    list
                })
                .map_err(|m| RpcError::new(INVALID_PARAMS, m))
        }
        "logging/setLevel" => set_level(shared, &params),
        other => Err(RpcError::new(
            METHOD_NOT_FOUND,
            format!("method not found: {other}"),
        )),
    };
    let line = match answer {
        Ok(result) => protocol::result(&id, shared.finish(era, result)),
        Err(e) => protocol::error(&id, e.code, &e.message, e.data),
    };
    shared.send(line).await;
}

/// The protocol version and log level of a request: the per-request
/// `_meta` version (modern clients) or the negotiated one.
fn era_of(shared: &Shared, params: &Map<String, Value>) -> Result<Era, RpcError> {
    let meta = params.get("_meta");
    let session = lock(&shared.session);
    match meta
        .and_then(|m| m.get(META_PROTOCOL_VERSION))
        .and_then(Value::as_str)
    {
        Some(requested) => {
            let Some(version) = known_version(requested) else {
                return Err(RpcError {
                    code: UNSUPPORTED_PROTOCOL_VERSION,
                    message: "Unsupported protocol version".to_owned(),
                    data: Some(json!({
                        "supported": supported_versions(),
                        "requested": requested,
                    })),
                });
            };
            let modern = is_modern(version);
            let log_level = if modern {
                meta.and_then(|m| m.get(META_LOG_LEVEL))
                    .and_then(Value::as_str)
                    .and_then(LogLevel::parse)
            } else {
                session.log_level
            };
            Ok(Era {
                version,
                modern,
                log_level,
            })
        }
        None => Ok(Era {
            version: session.version.unwrap_or(LATEST_LEGACY_VERSION),
            modern: false,
            log_level: session.log_level,
        }),
    }
}

fn initialize(shared: &Shared, params: &Map<String, Value>) -> Result<Value, RpcError> {
    let requested = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::new(INVALID_PARAMS, "initialize needs a protocolVersion"))?;
    let version = negotiate(requested);
    lock(&shared.session).version = Some(version);
    let client = params
        .get("clientInfo")
        .and_then(|c| c.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("unknown client");
    tracing::info!(client, requested, version, "MCP initialize");
    Ok(json!({
        "protocolVersion": version,
        "capabilities": Shared::capabilities(false),
        "serverInfo": shared.server_info(version),
        "instructions": shared.instructions,
    }))
}

fn set_level(shared: &Shared, params: &Map<String, Value>) -> Result<Value, RpcError> {
    let level = params
        .get("level")
        .and_then(Value::as_str)
        .and_then(LogLevel::parse)
        .ok_or_else(|| {
            RpcError::new(
                INVALID_PARAMS,
                "level must be one of debug, info, notice, warning, error, critical, alert, emergency",
            )
        })?;
    lock(&shared.session).log_level = Some(level);
    Ok(json!({}))
}

fn notification(shared: &Shared, method: &str, params: &Map<String, Value>) {
    match method {
        "notifications/initialized" => tracing::debug!("MCP client initialized"),
        "notifications/cancelled" => {
            let target = params.get("requestId").map(Value::to_string);
            let token = target.and_then(|t| lock(&shared.inflight).get(&t).cloned());
            if let Some(token) = token {
                tracing::debug!(reason = ?params.get("reason"), "MCP request cancelled");
                token.cancel();
            }
        }
        other => tracing::debug!(method = other, "ignoring an MCP notification"),
    }
}

/// Validates a `tools/call` and runs it in a task.
fn start_call(
    shared: &Arc<Shared>,
    id: Value,
    params: Map<String, Value>,
    era: Era,
    tasks: &mut JoinSet<()>,
) -> Result<(), RpcError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::new(INVALID_PARAMS, "tools/call needs a tool name"))?;
    let Some(command) = shared.tools.id_of(name) else {
        let mut e = RpcError::new(
            INVALID_PARAMS,
            format!(
                "unknown tool: {name} (tools/list lists the {} tools)",
                shared.tools.len()
            ),
        );
        e.data = Some(json!({ "name": name }));
        return Err(e);
    };
    let arguments = match params.get("arguments") {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(Value::Object(map)) => Value::Object(map.clone()),
        Some(_) => {
            return Err(RpcError::new(
                INVALID_PARAMS,
                "tools/call arguments must be an object",
            ));
        }
    };
    let progress_token = params
        .get("_meta")
        .and_then(|m| m.get("progressToken"))
        .filter(|t| t.is_string() || t.is_i64() || t.is_u64())
        .cloned();
    let key = id.to_string();
    let token = CancellationToken::new();
    {
        let mut inflight = lock(&shared.inflight);
        if inflight.contains_key(&key) {
            return Err(RpcError::new(
                INVALID_REQUEST,
                format!("request id {key} is already in flight"),
            ));
        }
        if inflight.len() >= shared.opts.max_pending.max(1) {
            let busy = OpError::busy(format!(
                "{} tool calls are in flight; retry later",
                inflight.len()
            ));
            let result = shared.finish(era, ToolCatalog::error_result(&busy));
            let _ = shared.outbox.try_send(protocol::result(&id, result));
            return Ok(());
        }
        inflight.insert(key.clone(), token.clone());
    }
    let mutating_doc = shared
        .tools
        .registry()
        .describe(command)
        .filter(|s| s.meta.mutates_document)
        .and_then(|_| arguments.get("docId").and_then(Value::as_str))
        .map(str::to_owned);
    let mut turn = shared.queue.enter(mutating_doc);
    let shared = Arc::clone(shared);
    tasks.spawn(async move {
        let meter = Arc::new(Mutex::new(ProgressMeter::default()));
        let started = Instant::now();
        let result = async {
            if !turn.wait(&token).await {
                return Err(OpError::cancelled());
            }
            let _permit = tokio::select! {
                permit = shared.permits.acquire() => permit
                    .map_err(|_| OpError::internal("the server is shutting down"))?,
                () = token.cancelled() => return Err(OpError::cancelled()),
            };
            let mut ctx = Ctx::new(CallerKind::Agent).with_cancel(token.clone());
            if let Some(progress_token) = progress_token {
                let outbox = shared.outbox.clone();
                let meter = Arc::clone(&meter);
                let token = token.clone();
                ctx = ctx.with_progress(move |event| {
                    if token.is_cancelled() {
                        return;
                    }
                    // Held while queueing, so no progress follows the response.
                    let mut meter = lock(&meter);
                    if let Some(progress) = meter.next(event.fraction) {
                        let _ = outbox.try_send(protocol::notification(
                            "notifications/progress",
                            json!({
                                "progressToken": progress_token,
                                "progress": progress,
                                "total": 100,
                                "message": event.message,
                            }),
                        ));
                    }
                });
            }
            shared
                .tools
                .registry()
                .execute(&shared.ws, command, arguments, &ctx)
                .await
        }
        .await;
        drop(turn);
        lock(&meter).close();
        lock(&shared.inflight).remove(&key);
        if token.is_cancelled() {
            // Cancelled by the client: no response (MCP cancellation).
            return;
        }
        let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let result = match result {
            Ok(output) => {
                shared.log(
                    era,
                    LogLevel::Debug,
                    json!({ "tool": name_of(&shared, command), "durationMs": duration_ms }),
                );
                shared.tools.call_result(
                    command,
                    output,
                    era.version,
                    shared.opts.max_text_bytes,
                )
            }
            Err(e) => {
                let level = if e.code == OpErrorCode::Internal {
                    LogLevel::Error
                } else {
                    LogLevel::Warning
                };
                shared.log(
                    era,
                    level,
                    json!({ "tool": name_of(&shared, command), "code": e.code, "message": e.message }),
                );
                ToolCatalog::error_result(&e)
            }
        };
        shared
            .send(protocol::result(&id, shared.finish(era, result)))
            .await;
    });
    Ok(())
}

fn name_of(shared: &Shared, id: &str) -> String {
    shared.tools.name_of(id).unwrap_or(id).to_owned()
}
