//! The JSON-lines RPC server in process (over an in-memory pipe): request /
//! response, protocol errors, notifications, progress, cancellation of a
//! queued request and per-document ordering.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{mini_pbip, tmp_dir};
use pbi_ops::rpc::{RpcOptions, serve};
use pbi_ops::{Command, CommandMeta, Ctx, OpError, Registry, Workspace};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HoldParams {
    doc_id: String,
    #[serde(default)]
    ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct HoldOut {
    done: bool,
}

/// A mutating command that holds its document for `ms`.
struct Hold;
impl Command for Hold {
    type Params = HoldParams;
    type Output = HoldOut;
    fn meta() -> CommandMeta {
        CommandMeta {
            id: "test.hold",
            title: "hold",
            summary: "hold",
            category: pbi_ops::Category::Meta,
            mutates_document: true,
            long_running: true,
            agent_safe: true,
            writes_files: false,
            invokes_other_commands: false,
            since: "0.0.0",
            examples: &[("t", r#"{"docId": "d1"}"#)],
        }
    }
    async fn run(_: &Workspace, p: HoldParams, ctx: &Ctx) -> Result<HoldOut, OpError> {
        ctx.progress("holding", Some(0.0));
        tokio::time::sleep(Duration::from_millis(p.ms)).await;
        Ok(HoldOut { done: true })
    }
}

struct Client {
    tx: tokio::io::WriteHalf<tokio::io::DuplexStream>,
    rx: tokio::io::Lines<BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl Client {
    fn start(registry: Registry) -> Self {
        let (client, server_end) = tokio::io::duplex(1 << 20);
        let (server_read, server_write) = tokio::io::split(server_end);
        let server = tokio::spawn(serve(
            registry,
            Arc::new(Workspace::new()),
            BufReader::new(server_read),
            server_write,
            RpcOptions::default(),
        ));
        let (read, tx) = tokio::io::split(client);
        Self {
            tx,
            rx: BufReader::new(read).lines(),
            server,
        }
    }

    async fn send(&mut self, line: &str) {
        self.tx.write_all(line.as_bytes()).await.unwrap();
        self.tx.write_all(b"\n").await.unwrap();
    }

    async fn request(&mut self, id: u64, method: &str, params: Value) {
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.send(&msg.to_string()).await;
    }

    /// The next message (with a timeout).
    async fn next(&mut self) -> Value {
        let line = tokio::time::timeout(Duration::from_secs(60), self.rx.next_line())
            .await
            .expect("timed out waiting for the server")
            .unwrap()
            .expect("server closed the stream");
        serde_json::from_str(&line).unwrap()
    }

    /// The response to `id`, collecting progress notifications.
    async fn response(&mut self, id: u64, progress: &mut Vec<Value>) -> Value {
        loop {
            let msg = self.next().await;
            if msg["method"] == "progress" {
                progress.push(msg);
            } else if msg["id"] == id {
                return msg;
            } else {
                panic!("unexpected message while waiting for {id}: {msg}");
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn requests_errors_and_notifications() {
    let dir = tmp_dir("pbi-ops-rpc-");
    let pbip = mini_pbip(dir.path());
    let mut c = Client::start(Registry::default_catalog());
    let mut progress = Vec::new();

    c.request(1, "doc.open", json!({"path": pbip})).await;
    let open = c.response(1, &mut progress).await;
    assert_eq!(open["jsonrpc"], "2.0");
    assert_eq!(open["result"]["docId"], "d1");

    c.request(2, "data.refresh", json!({"docId": "d1"})).await;
    let refresh = c.response(2, &mut progress).await;
    assert_eq!(refresh["result"]["ok"], true);
    assert!(
        progress.iter().any(|p| p["params"]["id"] == 2
            && p["params"]["command"] == "data.refresh"
            && p["params"]["fraction"] == 1.0),
        "{progress:?}"
    );

    c.send("not json").await;
    let parse = c.next().await;
    assert_eq!(parse["error"]["code"], -32700);
    assert_eq!(parse["id"], Value::Null);
    c.send("[1, 2]").await;
    assert_eq!(c.next().await["error"]["code"], -32600);
    c.request(3, "no.such", json!({})).await;
    let unknown = c.response(3, &mut progress).await;
    assert_eq!(unknown["error"]["code"], -32601);
    assert_eq!(unknown["error"]["data"]["code"], "unknownCommand");
    c.request(4, "report.pages", json!({"docId": "d1", "extra": 1}))
        .await;
    assert_eq!(c.response(4, &mut progress).await["error"]["code"], -32602);
    c.request(5, "measures.values", json!({"docId": "d9"}))
        .await;
    let no_doc = c.response(5, &mut progress).await;
    assert_eq!(no_doc["error"]["data"]["code"], "noDocument");

    // A notification runs without a response; the next request sees it.
    c.send(r#"{"jsonrpc":"2.0","method":"doc.close","params":{"docId":"d1"}}"#)
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    c.request(6, "doc.list", json!({})).await;
    assert_eq!(
        c.response(6, &mut progress).await["result"]["documents"],
        json!([])
    );

    c.tx.shutdown().await.unwrap();
    c.server.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn queued_requests_keep_order_and_can_be_cancelled() {
    let dir = tmp_dir("pbi-ops-rpc-order-");
    let pbip = mini_pbip(dir.path());
    let registry = Registry::default_catalog().with::<Hold>();
    let mut c = Client::start(registry);
    let mut progress = Vec::new();
    c.request(1, "doc.open", json!({"path": pbip, "autoRefresh": false}))
        .await;
    assert_eq!(c.response(1, &mut progress).await["result"]["docId"], "d1");

    // 10 holds the document; 11 and 12 queue behind it; 11 is cancelled
    // while queued and answers at once; 12 still waits for 10.
    c.request(10, "test.hold", json!({"docId": "d1", "ms": 400}))
        .await;
    c.request(11, "test.hold", json!({"docId": "d1", "ms": 0}))
        .await;
    c.request(12, "test.hold", json!({"docId": "d1", "ms": 0}))
        .await;
    c.request(13, "doc.list", json!({})).await;
    c.send(r#"{"jsonrpc":"2.0","method":"$/cancel","params":{"id":11}}"#)
        .await;
    let mut order = Vec::new();
    while order.len() < 4 {
        let msg = c.next().await;
        if msg["method"] == "progress" {
            continue;
        }
        order.push((msg["id"].as_u64().unwrap(), msg));
    }
    let ids: Vec<u64> = order.iter().map(|(id, _)| *id).collect();
    let pos = |id: u64| ids.iter().position(|i| *i == id).unwrap();
    assert!(
        pos(13) < pos(10),
        "reads do not wait for mutations: {ids:?}"
    );
    assert!(
        pos(11) < pos(10),
        "a cancelled queued request answers at once: {ids:?}"
    );
    assert!(pos(10) < pos(12), "mutations keep their order: {ids:?}");
    let cancelled = &order[pos(11)].1;
    assert_eq!(cancelled["error"]["code"], -32800);
    assert_eq!(cancelled["error"]["data"]["code"], "cancelled");
    assert_eq!(order[pos(12)].1["result"]["done"], true);

    c.tx.shutdown().await.unwrap();
    c.server.await.unwrap().unwrap();
}
