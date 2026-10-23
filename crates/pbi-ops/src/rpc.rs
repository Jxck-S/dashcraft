//! JSON-lines RPC over any byte stream (`pbiview serve --stdio`).
//!
//! One JSON-RPC 2.0 message per line, UTF-8, `\n`-terminated:
//!
//! ```text
//! → {"jsonrpc":"2.0","id":1,"method":"doc.open","params":{"path":"model.pbip"}}
//! ← {"jsonrpc":"2.0","id":1,"result":{"docId":"d1",…}}
//! → {"jsonrpc":"2.0","id":2,"method":"data.refresh","params":{"docId":"d1"}}
//! ← {"jsonrpc":"2.0","method":"progress","params":{"id":2,"command":"data.refresh","message":"…","fraction":0.5}}
//! → {"jsonrpc":"2.0","method":"$/cancel","params":{"id":2}}
//! ← {"jsonrpc":"2.0","id":2,"error":{"code":-32800,"message":"the operation was cancelled","data":{"code":"cancelled"}}}
//! ```
//!
//! - **Methods** are command ids (`commands.list` lists them). `params` is
//!   an object (absent or `null` = `{}`).
//! - **Errors** carry the JSON-RPC integer code, the message and `data: {
//!   code: "<stable string code>", details? }` (see
//!   [`OpError::to_jsonrpc`]). Malformed lines get `-32700` (parse error) or
//!   `-32600` (invalid request) with `id: null` when the id is unknown.
//! - **Concurrency.** Requests run concurrently (at most
//!   [`RpcOptions::max_concurrent`] at a time; beyond
//!   [`RpcOptions::max_pending`] in flight they fail with `busy`).
//!   Commands that mutate a document run one at a time per `docId`, in the
//!   order they were received; responses can arrive out of order (match
//!   them by `id`).
//! - **Notifications** (no `id`) run without a response.
//!   `$/cancel {"id": …}` cancels a running or queued request, which then
//!   answers with `cancelled`.
//! - **Progress** of requests with an `id` is sent as `progress`
//!   notifications (best effort: dropped when the client does not read).
//! - At end of input the server waits for the running requests, writes
//!   their responses and returns.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Semaphore, mpsc, oneshot};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::ctx::{CallerKind, Ctx};
use crate::error::OpError;
use crate::registry::Registry;
use crate::workspace::{Workspace, lock};

/// Server limits.
#[derive(Debug, Clone, Copy)]
pub struct RpcOptions {
    /// Requests running at the same time.
    pub max_concurrent: usize,
    /// Requests in flight (running or queued) before `busy`.
    pub max_pending: usize,
    /// Longest request line in bytes.
    pub max_line_bytes: usize,
    /// Caller kind of the requests (policy).
    pub caller: CallerKind,
}

impl Default for RpcOptions {
    fn default() -> Self {
        Self {
            max_concurrent: std::thread::available_parallelism()
                .map_or(4, std::num::NonZeroUsize::get)
                .clamp(2, 8),
            max_pending: 256,
            max_line_bytes: 16 * 1024 * 1024,
            caller: CallerKind::Rpc,
        }
    }
}

/// Outgoing lines (responses and notifications).
const OUTBOX: usize = 1024;

struct Shared {
    registry: Registry,
    ws: Arc<Workspace>,
    caller: CallerKind,
    outbox: mpsc::Sender<String>,
    inflight: Mutex<HashMap<String, CancellationToken>>,
    queue: DocQueue,
    permits: Semaphore,
    max_pending: usize,
}

/// Starts a task writing each line sent to the returned channel to
/// `writer` (`\n`-terminated, flushed). The task ends when every sender is
/// dropped or on the first write error.
pub(crate) fn spawn_writer<W>(
    mut writer: W,
) -> (
    mpsc::Sender<String>,
    tokio::task::JoinHandle<std::io::Result<()>>,
)
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (outbox, mut lines) = mpsc::channel::<String>(OUTBOX);
    let task = tokio::spawn(async move {
        while let Some(line) = lines.recv().await {
            writer.write_all(line.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }
        Ok::<(), std::io::Error>(())
    });
    (outbox, task)
}

/// What [`read_line`] read.
pub(crate) enum Line {
    /// End of input.
    Eof,
    /// A line longer than the limit (its rest was skipped).
    TooLong,
    /// One line (lossy UTF-8, untrimmed).
    Text(String),
}

/// Reads one `\n`-terminated line of at most `limit` bytes, using `buf` as
/// scratch space.
pub(crate) async fn read_line<R>(
    reader: &mut R,
    limit: usize,
    buf: &mut Vec<u8>,
) -> std::io::Result<Line>
where
    R: AsyncBufRead + Unpin + Send,
{
    let limit = u64::try_from(limit).unwrap_or(u64::MAX);
    buf.clear();
    let n = (&mut *reader)
        .take(limit.saturating_add(1))
        .read_until(b'\n', buf)
        .await?;
    if n == 0 {
        return Ok(Line::Eof);
    }
    if buf.len() as u64 > limit && buf.last() != Some(&b'\n') {
        // Skip the rest of the oversized line without buffering it.
        loop {
            let chunk = reader.fill_buf().await?;
            if chunk.is_empty() {
                break;
            }
            if let Some(end) = chunk.iter().position(|b| *b == b'\n') {
                reader.consume(end + 1);
                break;
            }
            let len = chunk.len();
            reader.consume(len);
        }
        return Ok(Line::TooLong);
    }
    Ok(Line::Text(String::from_utf8_lossy(buf).into_owned()))
}

/// Orders mutating requests per document: each waits for the previous one
/// on the same document, in arrival order.
#[derive(Default)]
pub(crate) struct DocQueue {
    /// Per document: completion of the last mutating request.
    chains: Mutex<HashMap<String, oneshot::Receiver<()>>>,
}

impl DocQueue {
    /// Queues a request on `doc` (`None`: not ordered).
    pub(crate) fn enter(&self, doc: Option<String>) -> Turn {
        match doc {
            Some(doc) => {
                let (tx, rx) = oneshot::channel::<()>();
                let prev = lock(&self.chains).insert(doc, rx);
                Turn {
                    prev,
                    done: Some(tx),
                }
            }
            None => Turn {
                prev: None,
                done: None,
            },
        }
    }
}

/// A request's place in a [`DocQueue`]; dropping it lets the next request
/// on the document run.
pub(crate) struct Turn {
    prev: Option<oneshot::Receiver<()>>,
    done: Option<oneshot::Sender<()>>,
}

impl Turn {
    /// Waits for the previous request on the document; `false` when `token`
    /// is cancelled first (the next request then still waits for the
    /// previous one).
    pub(crate) async fn wait(&mut self, token: &CancellationToken) -> bool {
        let waited = match self.prev.as_mut() {
            None => true,
            Some(rx) => tokio::select! {
                _ = rx => true,
                () = token.cancelled() => false,
            },
        };
        if waited {
            self.prev = None;
            return true;
        }
        if let Some(rx) = self.prev.take() {
            let done = self.done.take();
            tokio::spawn(async move {
                let _ = rx.await;
                drop(done);
            });
        }
        false
    }
}

fn response(id: &Value, result: Result<Value, OpError>) -> String {
    let msg = match result {
        Ok(v) => json!({ "jsonrpc": "2.0", "id": id, "result": v }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e.to_jsonrpc() }),
    };
    msg.to_string()
}

fn protocol_error(id: &Value, code: i64, message: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message, "data": { "code": "invalidParams" } },
    })
    .to_string()
}

/// Serves requests read from `reader`, writing responses to `writer`, on
/// `ws`, until end of input.
///
/// # Errors
/// I/O errors of the reader or writer.
pub async fn serve<R, W>(
    registry: Registry,
    ws: Arc<Workspace>,
    mut reader: R,
    writer: W,
    opts: RpcOptions,
) -> std::io::Result<()>
where
    R: AsyncBufRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (outbox, write_task) = spawn_writer(writer);
    let shared = Arc::new(Shared {
        registry,
        ws,
        caller: opts.caller,
        outbox,
        inflight: Mutex::new(HashMap::new()),
        queue: DocQueue::default(),
        permits: Semaphore::new(opts.max_concurrent.max(1)),
        max_pending: opts.max_pending.max(1),
    });
    let mut tasks = JoinSet::new();
    let mut buf = Vec::new();
    let read_result = loop {
        let text = match read_line(&mut reader, opts.max_line_bytes, &mut buf).await {
            Ok(Line::Eof) => break Ok(()),
            Ok(Line::TooLong) => {
                let _ = shared
                    .outbox
                    .send(protocol_error(
                        &Value::Null,
                        -32600,
                        "request line too long",
                    ))
                    .await;
                continue;
            }
            Ok(Line::Text(text)) => text,
            Err(e) => break Err(e),
        };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        handle_line(&shared, text, &mut tasks).await;
        while tasks.try_join_next().is_some() {}
    };
    while tasks.join_next().await.is_some() {}
    drop(shared);
    let written = write_task
        .await
        .unwrap_or_else(|e| Err(std::io::Error::other(e)));
    read_result.and(written)
}

async fn handle_line(shared: &Arc<Shared>, text: &str, tasks: &mut JoinSet<()>) {
    let msg: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            let _ = shared
                .outbox
                .send(protocol_error(
                    &Value::Null,
                    -32700,
                    &format!("parse error: {e}"),
                ))
                .await;
            return;
        }
    };
    let Value::Object(mut obj) = msg else {
        let _ = shared
            .outbox
            .send(protocol_error(
                &Value::Null,
                -32600,
                "a request must be a JSON object (batch arrays are not supported; use commands.batch)",
            ))
            .await;
        return;
    };
    let id = obj.remove("id");
    let reply_id = id.clone().unwrap_or(Value::Null);
    let invalid = |m: &str| protocol_error(&reply_id, -32600, m);
    if obj.get("jsonrpc").is_some_and(|v| v != "2.0") {
        let _ = shared.outbox.send(invalid("jsonrpc must be \"2.0\"")).await;
        return;
    }
    if id
        .as_ref()
        .is_some_and(|i| !(i.is_string() || i.is_number() || i.is_null()))
    {
        let _ = shared
            .outbox
            .send(invalid("id must be a string, a number or null"))
            .await;
        return;
    }
    let Some(method) = obj.get("method").and_then(Value::as_str).map(str::to_owned) else {
        if id.is_some() {
            let _ = shared.outbox.send(invalid("missing method")).await;
        }
        return;
    };
    let params = obj.remove("params").unwrap_or(Value::Null);
    if !(params.is_object() || params.is_null()) {
        if id.is_some() {
            let _ = shared
                .outbox
                .send(response(
                    &reply_id,
                    Err(OpError::invalid_params("params must be an object")),
                ))
                .await;
        }
        return;
    }
    if method == "$/cancel" {
        let target = params.get("id").map(Value::to_string);
        let cancelled = target
            .and_then(|t| lock(&shared.inflight).get(&t).cloned())
            .map(|t| t.cancel())
            .is_some();
        if id.is_some() {
            let _ = shared
                .outbox
                .send(response(&reply_id, Ok(json!({ "cancelled": cancelled }))))
                .await;
        }
        return;
    }
    let token = CancellationToken::new();
    let key = id.as_ref().map(Value::to_string);
    if let Some(key) = &key {
        let (full, dup) = {
            let inflight = lock(&shared.inflight);
            (
                inflight.len() >= shared.max_pending,
                inflight.contains_key(key),
            )
        };
        if full || dup {
            let error = if dup {
                OpError::invalid_params(format!("request id {key} is already in flight"))
            } else {
                OpError::busy(format!(
                    "{} requests are in flight; retry later",
                    shared.max_pending
                ))
            };
            let _ = shared.outbox.send(response(&reply_id, Err(error))).await;
            return;
        }
        lock(&shared.inflight).insert(key.clone(), token.clone());
    }
    // Per-document FIFO for mutating commands.
    let mutating_doc = shared
        .registry
        .describe(&method)
        .filter(|s| s.meta.mutates_document)
        .and_then(|_| {
            params
                .get("docId")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    let turn = shared.queue.enter(mutating_doc);
    let shared = Arc::clone(shared);
    tasks.spawn(async move {
        let result = run_request(&shared, &method, params, id.as_ref(), &token, turn).await;
        if let Some(key) = &key {
            lock(&shared.inflight).remove(key);
        }
        if id.is_some() {
            let _ = shared.outbox.send(response(&reply_id, result)).await;
        }
    });
}

async fn run_request(
    shared: &Arc<Shared>,
    method: &str,
    params: Value,
    id: Option<&Value>,
    token: &CancellationToken,
    mut turn: Turn,
) -> Result<Value, OpError> {
    if !turn.wait(token).await {
        // Cancelled while queued: the next request still waits for ours.
        return Err(OpError::cancelled());
    }
    let result = async {
        let _permit = tokio::select! {
            permit = shared.permits.acquire() => permit
                .map_err(|_| OpError::internal("the server is shutting down"))?,
            () = token.cancelled() => return Err(OpError::cancelled()),
        };
        let mut ctx = Ctx::new(shared.caller).with_cancel(token.clone());
        if let Some(id) = id.cloned() {
            let outbox = shared.outbox.clone();
            ctx = ctx.with_progress(move |event| {
                let mut params = serde_json::to_value(&event).unwrap_or_else(|_| json!({}));
                if let Value::Object(map) = &mut params {
                    map.insert("id".into(), id.clone());
                }
                let note = json!({ "jsonrpc": "2.0", "method": "progress", "params": params });
                let _ = outbox.try_send(note.to_string());
            });
        }
        shared
            .registry
            .execute(&shared.ws, method, params, &ctx)
            .await
    }
    .await;
    drop(turn);
    result
}
