//! The MCP server in process (over an in-memory pipe): version
//! negotiation, the tool list (schemas, names, annotations, pagination),
//! calls and their errors, the path policy, cancellation, progress, image
//! content, truncation and logging. The expected-values case runs on the
//! synthetic Acme Outfitters example.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use common::{BLOB_MAP, PBIP, expected, fixture, mini_pbip, reference_root, tmp_dir};
use pbi_ops::mcp::protocol::LATEST_LEGACY_VERSION;
use pbi_ops::mcp::{McpOptions, McpServer, ToolCatalog, ToolFilter, ToolNames};
use pbi_ops::{
    Category, Command, CommandMeta, Ctx, OpError, OpErrorCode, PathPolicy, Registry, Workspace,
};
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

/// A mutating, agent-safe command that holds its document for `ms`.
struct Hold;
impl Command for Hold {
    type Params = HoldParams;
    type Output = HoldOut;
    fn meta() -> CommandMeta {
        CommandMeta {
            id: "test.hold",
            title: "hold",
            summary: "hold",
            category: Category::Meta,
            mutates_document: true,
            long_running: true,
            agent_safe: true,
            writes_files: false,
            invokes_other_commands: false,
            since: "0.0.0",
            examples: &[],
        }
    }
    async fn run(_: &Workspace, p: HoldParams, ctx: &Ctx) -> Result<HoldOut, OpError> {
        ctx.progress("holding", Some(0.0));
        tokio::time::sleep(Duration::from_millis(p.ms)).await;
        Ok(HoldOut { done: true })
    }
}

macro_rules! named_command {
    ($ty:ident, $id:literal) => {
        struct $ty;
        impl Command for $ty {
            type Params = serde_json::Map<String, Value>;
            type Output = HoldOut;
            fn meta() -> CommandMeta {
                CommandMeta {
                    id: $id,
                    agent_safe: true,
                    ..Hold::meta()
                }
            }
            async fn run(
                _: &Workspace,
                _: serde_json::Map<String, Value>,
                _: &Ctx,
            ) -> Result<HoldOut, OpError> {
                Ok(HoldOut { done: true })
            }
        }
    };
}

named_command!(AbC, "a.b_c");
named_command!(ABc, "a_b.c");

/// A read-only command flagged as running other commands (like
/// `commands.batch`).
struct Runner;
impl Command for Runner {
    type Params = serde_json::Map<String, Value>;
    type Output = HoldOut;
    fn meta() -> CommandMeta {
        CommandMeta {
            id: "test.runner",
            mutates_document: false,
            long_running: false,
            invokes_other_commands: true,
            ..Hold::meta()
        }
    }
    async fn run(
        _: &Workspace,
        _: serde_json::Map<String, Value>,
        _: &Ctx,
    ) -> Result<HoldOut, OpError> {
        Ok(HoldOut { done: true })
    }
}

/// A workspace confined to `root`.
fn confined(root: &std::path::Path) -> Workspace {
    Workspace::with_policy(PathPolicy::restricted([root], Vec::<&str>::new()).unwrap())
}

struct Client {
    tx: tokio::io::WriteHalf<tokio::io::DuplexStream>,
    rx: tokio::io::Lines<BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
    /// Notifications received while waiting for responses.
    notes: Vec<Value>,
    next_id: i64,
}

impl Client {
    fn start(registry: &Registry, ws: Workspace, opts: McpOptions) -> Self {
        let server = McpServer::new(registry, Arc::new(ws), opts).unwrap();
        let (client, server_end) = tokio::io::duplex(1 << 20);
        let (server_read, server_write) = tokio::io::split(server_end);
        let server = tokio::spawn(server.serve(BufReader::new(server_read), server_write));
        let (read, tx) = tokio::io::split(client);
        Self {
            tx,
            rx: BufReader::new(read).lines(),
            server,
            notes: Vec::new(),
            next_id: 100,
        }
    }

    /// A server on the default catalog, confined to `root`.
    fn in_root(root: &std::path::Path) -> Self {
        Self::start(
            &Registry::default_catalog(),
            confined(root),
            McpOptions::default(),
        )
    }

    async fn send(&mut self, msg: &Value) {
        self.tx.write_all(msg.to_string().as_bytes()).await.unwrap();
        self.tx.write_all(b"\n").await.unwrap();
    }

    async fn next(&mut self) -> Value {
        let line = tokio::time::timeout(Duration::from_secs(120), self.rx.next_line())
            .await
            .expect("timed out waiting for the server")
            .unwrap()
            .expect("server closed the stream");
        serde_json::from_str(&line).unwrap()
    }

    /// Sends a request and returns its response (notifications are kept in
    /// `notes`).
    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await;
        self.response(id).await
    }

    async fn response(&mut self, id: i64) -> Value {
        loop {
            let msg = self.next().await;
            if msg.get("method").is_some() {
                self.notes.push(msg);
            } else if msg["id"] == id {
                return msg;
            } else {
                panic!("unexpected message while waiting for {id}: {msg}");
            }
        }
    }

    async fn initialize(&mut self, version: &str) -> Value {
        let init = self
            .request(
                "initialize",
                json!({"protocolVersion": version, "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}),
            )
            .await;
        self.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await;
        init
    }

    /// `tools/call` → its `result` (panics on a JSON-RPC error).
    async fn tool(&mut self, name: &str, arguments: Value) -> Value {
        let msg = self
            .request("tools/call", json!({"name": name, "arguments": arguments}))
            .await;
        assert!(msg.get("error").is_none(), "{name}: {msg}");
        msg["result"].clone()
    }

    /// A successful call's `structuredContent`.
    async fn ok(&mut self, name: &str, arguments: Value) -> Value {
        let result = self.tool(name, arguments).await;
        assert_eq!(result["isError"], false, "{name}: {result}");
        result["structuredContent"].clone()
    }

    async fn finish(mut self) {
        self.tx.shutdown().await.unwrap();
        tokio::time::timeout(Duration::from_secs(30), self.server)
            .await
            .expect("the server did not stop")
            .unwrap()
            .unwrap();
    }
}

/// The parsed error of an `isError` result.
fn tool_error(result: &Value) -> Value {
    assert_eq!(result["isError"], true, "{result}");
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn initialize_negotiates_the_protocol_version() {
    let dir = tmp_dir("pbi-ops-mcp-init-");
    let mut c = Client::in_root(dir.path());
    let init = c.initialize("2025-06-18").await;
    let r = &init["result"];
    assert_eq!(r["protocolVersion"], "2025-06-18");
    assert_eq!(r["capabilities"]["tools"]["listChanged"], false);
    assert_eq!(r["serverInfo"]["name"], "dashcraft");
    assert!(r["instructions"].as_str().unwrap().contains("doc_open"));
    assert_eq!(c.request("ping", json!({})).await["result"], json!({}));
    let list = c.request("tools/list", json!({})).await;
    let tool = &list["result"]["tools"][0];
    assert!(tool.get("title").is_some() && tool.get("outputSchema").is_some());
    c.finish().await;

    // An unknown revision gets our latest; older revisions get older shapes.
    let mut c = Client::in_root(dir.path());
    let init = c.initialize("2099-01-01").await;
    assert_eq!(init["result"]["protocolVersion"], LATEST_LEGACY_VERSION);
    c.finish().await;
    let mut c = Client::in_root(dir.path());
    assert_eq!(
        c.initialize("2024-11-05").await["result"]["protocolVersion"],
        "2024-11-05"
    );
    let list = c.request("tools/list", json!({})).await;
    let tool = &list["result"]["tools"][0];
    assert!(tool.get("outputSchema").is_none() && tool.get("annotations").is_none());
    let mut init = c.request("initialize", json!({})).await;
    assert_eq!(init["error"]["code"], -32602);
    init = c.request("no/such", json!({})).await;
    assert_eq!(init["error"]["code"], -32601);
    c.finish().await;

    // Stateless (2026-07-28) clients: discovery and per-request versions.
    let mut c = Client::in_root(dir.path());
    let meta = json!({"_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28"}});
    let discover = c.request("server/discover", meta.clone()).await;
    let r = &discover["result"];
    assert_eq!(r["resultType"], "complete");
    assert!(
        r["supportedVersions"]
            .as_array()
            .unwrap()
            .contains(&json!("2025-06-18"))
    );
    assert_eq!(
        r["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "dashcraft"
    );
    let list = c.request("tools/list", meta).await;
    assert_eq!(list["result"]["resultType"], "complete");
    assert!(list["result"]["ttlMs"].is_number());
    let bad = c
        .request(
            "tools/list",
            json!({"_meta": {"io.modelcontextprotocol/protocolVersion": "1900-01-01"}}),
        )
        .await;
    assert_eq!(bad["error"]["code"], -32022);
    assert_eq!(bad["error"]["data"]["requested"], "1900-01-01");
    c.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tools_list_derives_schemas_names_and_annotations() {
    let dir = tmp_dir("pbi-ops-mcp-list-");
    let registry = Registry::default_catalog();
    let opts = McpOptions {
        page_size: 10,
        ..McpOptions::default()
    };
    let mut c = Client::start(&registry, confined(dir.path()), opts);
    c.initialize("2025-11-25").await;
    let mut tools = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let params = cursor
            .as_ref()
            .map_or_else(|| json!({}), |c| json!({"cursor": c}));
        let page = c.request("tools/list", params).await;
        let page = &page["result"];
        let items = page["tools"].as_array().unwrap();
        assert!(items.len() <= 10);
        tools.extend(items.iter().cloned());
        match page["nextCursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => break,
        }
    }
    let agent: Vec<&str> = registry
        .agent_catalog()
        .list()
        .iter()
        .map(|s| s.meta.id)
        .collect();
    assert_eq!(tools.len(), agent.len());
    let valid = |n: &str| {
        (1..=64).contains(&n.len())
            && n.chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    };
    let by_name = |n: &str| tools.iter().find(|t| t["name"] == n).unwrap().clone();
    for t in &tools {
        let name = t["name"].as_str().unwrap();
        assert!(valid(name), "{name}");
        assert_eq!(t["inputSchema"]["type"], "object", "{name}");
        assert_eq!(t["outputSchema"]["type"], "object", "{name}");
        assert!(!t["description"].as_str().unwrap().is_empty());
        assert_eq!(t["annotations"]["openWorldHint"], false);
        let id = t["_meta"]["dashcraft/commandId"].as_str().unwrap();
        assert_eq!(id.replace('.', "_"), name);
        assert!(agent.contains(&id));
    }
    let hints = |n: &str| {
        let a = by_name(n)["annotations"].clone();
        (
            a["readOnlyHint"].as_bool().unwrap(),
            a["destructiveHint"].as_bool().unwrap(),
            a["idempotentHint"].as_bool().unwrap(),
        )
    };
    assert_eq!(hints("doc_open"), (true, false, true));
    assert_eq!(hints("measures_values"), (true, false, true));
    assert_eq!(hints("canvas_click"), (false, false, false));
    assert_eq!(hints("export_table"), (false, true, true));
    assert_eq!(
        hints("commands_batch"),
        (false, true, false),
        "a batch may run any tool"
    );
    let bad = c.request("tools/list", json!({"cursor": "zz"})).await;
    assert_eq!(bad["error"]["code"], -32602);
    c.finish().await;
}

#[test]
fn tool_names_are_reversible_and_collision_checked() {
    let registry = Registry::default_catalog();
    let none = ToolFilter::default();
    for style in [ToolNames::Underscore, ToolNames::Dotted] {
        let tools = ToolCatalog::new(&registry, &none, style).unwrap();
        assert_eq!(tools.len(), registry.agent_catalog().len());
        for (name, id) in tools.iter() {
            assert!(style.is_valid(name), "{name}");
            assert_eq!(tools.id_of(name), Some(id));
            assert_eq!(tools.name_of(id), Some(name));
            if style == ToolNames::Dotted {
                assert_eq!(name, id);
            }
        }
    }
    let clash = Registry::new().with::<AbC>().with::<ABc>();
    let err = ToolCatalog::new(&clash, &none, ToolNames::Underscore).unwrap_err();
    assert_eq!(err.code, OpErrorCode::InvalidParams);
    assert!(err.message.contains("a_b_c"), "{}", err.message);
    assert_eq!(
        ToolCatalog::new(&clash, &none, ToolNames::Dotted)
            .unwrap()
            .len(),
        2
    );

    // --allow / --deny: categories, ids and tool names; typos are errors.
    let filter = |allow: &[&str], deny: &[&str]| ToolFilter {
        allow: allow.iter().map(|s| (*s).to_owned()).collect(),
        deny: deny.iter().map(|s| (*s).to_owned()).collect(),
    };
    let only = ToolCatalog::new(
        &registry,
        &filter(
            &["inspect", "doc.open", "measures_values"],
            &["report.visuals"],
        ),
        ToolNames::Underscore,
    )
    .unwrap();
    let names: Vec<&str> = only.iter().map(|(n, _)| n).collect();
    assert!(names.contains(&"doc_open") && names.contains(&"measures_values"));
    assert!(names.contains(&"model_tables") && !names.contains(&"report_visuals"));
    assert!(!names.contains(&"render_page"), "{names:?}");
    assert!(only.registry().describe("render.page").is_none());
    let typo = ToolCatalog::new(&registry, &filter(&["rendr"], &[]), ToolNames::Underscore);
    assert_eq!(typo.unwrap_err().code, OpErrorCode::InvalidParams);

    // Agents must be confined.
    let open = McpServer::new(&registry, Arc::new(Workspace::new()), McpOptions::default());
    assert_eq!(open.unwrap_err().code, OpErrorCode::InvalidParams);
}

#[test]
fn commands_that_invoke_others_get_the_catalog_hints() {
    let registry = Registry::new().with::<Hold>().with::<Runner>();
    let tools = ToolCatalog::new(&registry, &ToolFilter::default(), ToolNames::Underscore).unwrap();
    let runner = Runner::meta();
    let hints = tools.hints(&runner);
    // Read-only itself, but a step may be the mutating `test.hold`.
    assert!(!hints.read_only && !hints.idempotent && !hints.destructive);
    let plain = CommandMeta {
        invokes_other_commands: false,
        ..runner
    };
    let own = tools.hints(&plain);
    assert!(own.read_only && own.idempotent && !own.destructive);

    let catalog = Registry::default_catalog();
    let flagged: Vec<&str> = catalog
        .list()
        .iter()
        .filter(|s| s.meta.invokes_other_commands)
        .map(|s| s.meta.id)
        .collect();
    assert_eq!(flagged, ["commands.batch"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn calls_report_failures_as_tool_errors() {
    let dir = tmp_dir("pbi-ops-mcp-errors-");
    let outside = tmp_dir("pbi-ops-mcp-outside-");
    let root = dir.path().join("root");
    std::fs::create_dir_all(&root).unwrap();
    mini_pbip(&root);
    let elsewhere = mini_pbip(outside.path());
    let mut c = Client::in_root(&root);
    c.initialize("2025-06-18").await;

    // Relative paths resolve against the read root.
    let doc = c.ok("doc_open", json!({"path": "Mini.pbip"})).await;
    assert_eq!(doc["docId"], "d1");
    let pages = c.tool("report_pages", json!({"docId": "d1"})).await;
    assert_eq!(pages["isError"], false);
    let text: Value = serde_json::from_str(pages["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, pages["structuredContent"]);

    let denied = c.tool("doc_open", json!({"path": elsewhere})).await;
    assert_eq!(tool_error(&denied)["code"], "pathDenied");
    let escape = c.tool("doc_open", json!({"path": "../Mini.pbip"})).await;
    assert_eq!(tool_error(&escape)["code"], "pathDenied");
    let write = c
        .tool(
            "render_page",
            json!({"docId": "d1", "outputPath": "page.svg"}),
        )
        .await;
    assert_eq!(
        tool_error(&write)["code"],
        "pathDenied",
        "no write root: writes are refused"
    );
    let no_doc = c.tool("report_pages", json!({"docId": "d9"})).await;
    assert_eq!(tool_error(&no_doc)["code"], "noDocument");
    let bad_args = c
        .tool("report_pages", json!({"docId": "d1", "bogus": 1}))
        .await;
    assert_eq!(tool_error(&bad_args)["code"], "invalidParams");
    assert!(bad_args.get("structuredContent").is_none());

    let unknown = c
        .request(
            "tools/call",
            json!({"name": "no_such_tool", "arguments": {}}),
        )
        .await;
    assert_eq!(unknown["error"]["code"], -32602);
    let dotted = c
        .request("tools/call", json!({"name": "doc.open", "arguments": {}}))
        .await;
    assert_eq!(
        dotted["error"]["code"], -32602,
        "underscore names only by default"
    );
    let nameless = c.request("tools/call", json!({"arguments": {}})).await;
    assert_eq!(nameless["error"]["code"], -32602);
    let not_object = c
        .request("tools/call", json!({"name": "doc_list", "arguments": [1]}))
        .await;
    assert_eq!(not_object["error"]["code"], -32602);
    c.tx.write_all(b"{not json\n").await.unwrap();
    assert_eq!(c.next().await["error"]["code"], -32700);

    // Logging: failures at warning, successes at debug.
    let set = c
        .request("logging/setLevel", json!({"level": "debug"}))
        .await;
    assert_eq!(set["result"], json!({}));
    let bad_level = c
        .request("logging/setLevel", json!({"level": "loud"}))
        .await;
    assert_eq!(bad_level["error"]["code"], -32602);
    c.notes.clear();
    c.tool("doc_list", json!({})).await;
    c.tool("report_pages", json!({"docId": "d9"})).await;
    let logs: Vec<&Value> = c
        .notes
        .iter()
        .filter(|n| n["method"] == "notifications/message")
        .collect();
    assert!(
        logs.iter()
            .any(|l| l["params"]["level"] == "debug" && l["params"]["data"]["tool"] == "doc_list"),
        "{logs:?}"
    );
    assert!(
        logs.iter()
            .any(|l| l["params"]["level"] == "warning"
                && l["params"]["data"]["code"] == "noDocument"),
        "{logs:?}"
    );
    c.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelled_calls_stop_without_a_response() {
    let dir = tmp_dir("pbi-ops-mcp-cancel-");
    let pbip = mini_pbip(dir.path());
    let registry = Registry::default_catalog().with::<Hold>();
    let mut c = Client::start(&registry, confined(dir.path()), McpOptions::default());
    c.initialize("2025-06-18").await;
    c.ok("doc_open", json!({"path": pbip, "autoRefresh": false}))
        .await;

    let started = Instant::now();
    c.send(
        &json!({"jsonrpc": "2.0", "id": "long", "method": "tools/call",
        "params": {"name": "test_hold", "arguments": {"docId": "d1", "ms": 60_000}}}),
    )
    .await;
    c.send(
        &json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
        "params": {"requestId": "long", "reason": "test"}}),
    )
    .await;
    // The next mutation of the document runs once the cancelled one let go.
    c.send(
        &json!({"jsonrpc": "2.0", "id": "next", "method": "tools/call",
        "params": {"name": "test_hold", "arguments": {"docId": "d1", "ms": 0}}}),
    )
    .await;
    loop {
        let msg = c.next().await;
        assert_ne!(msg["id"], "long", "a cancelled call gets no response");
        if msg["id"] == "next" {
            assert_eq!(msg["result"]["structuredContent"]["done"], true, "{msg}");
            break;
        }
    }
    assert!(started.elapsed() < Duration::from_secs(30));
    let ping = c.request("ping", json!({})).await;
    assert_eq!(ping["result"], json!({}));
    assert!(c.notes.iter().all(|n| n["params"]["requestId"] != "long"));
    c.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_tokens_get_progress_notifications() {
    let dir = tmp_dir("pbi-ops-mcp-progress-");
    let pbip = mini_pbip(dir.path());
    let mut c = Client::in_root(dir.path());
    c.initialize("2025-06-18").await;
    c.ok("doc_open", json!({"path": pbip, "autoRefresh": false}))
        .await;
    let msg = c
        .request(
            "tools/call",
            json!({"name": "data_refresh", "arguments": {"docId": "d1"}, "_meta": {"progressToken": "p1"}}),
        )
        .await;
    assert_eq!(msg["result"]["structuredContent"]["ok"], true, "{msg}");
    let progress: Vec<&Value> = c
        .notes
        .iter()
        .filter(|n| n["method"] == "notifications/progress")
        .map(|n| &n["params"])
        .collect();
    assert!(!progress.is_empty());
    assert!(
        progress
            .iter()
            .all(|p| p["progressToken"] == "p1" && p["total"] == 100 && p["message"].is_string())
    );
    let values: Vec<f64> = progress
        .iter()
        .map(|p| p["progress"].as_f64().unwrap())
        .collect();
    assert!(values.windows(2).all(|w| w[1] > w[0]), "{values:?}");
    assert!(values.iter().all(|v| (0.0..=100.0).contains(v)));

    c.notes.clear();
    c.ok("data_refresh", json!({"docId": "d1"})).await;
    assert!(
        c.notes
            .iter()
            .all(|n| n["method"] != "notifications/progress"),
        "no token, no progress"
    );
    c.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn png_renders_are_image_content_and_large_text_is_cut() {
    let dir = tmp_dir("pbi-ops-mcp-render-");
    let pbip = mini_pbip(dir.path());
    let mut c = Client::in_root(dir.path());
    c.initialize("2025-06-18").await;
    c.ok("doc_open", json!({"path": pbip})).await;
    let render = c
        .tool("render_page", json!({"docId": "d1", "format": "png"}))
        .await;
    assert_eq!(render["isError"], false, "{render}");
    let content = render["content"].as_array().unwrap();
    let image = content.iter().find(|b| b["type"] == "image").unwrap();
    assert_eq!(image["mimeType"], "image/png");
    let png = base64::engine::general_purpose::STANDARD
        .decode(image["data"].as_str().unwrap())
        .unwrap();
    assert!(png.starts_with(b"\x89PNG"));
    let text = content[0]["text"].as_str().unwrap();
    assert!(
        text.len() < 16 * 1024 && !text.contains("pngBase64"),
        "{text}"
    );
    let structured = &render["structuredContent"];
    assert_eq!(structured["format"], "png");
    assert!(structured.get("pngBase64").is_none());
    assert_eq!(structured["bytes"].as_u64(), Some(png.len() as u64));
    c.finish().await;

    let opts = McpOptions {
        max_text_bytes: 1000,
        ..McpOptions::default()
    };
    let mut c = Client::start(&Registry::default_catalog(), confined(dir.path()), opts);
    c.initialize("2025-06-18").await;
    c.ok("doc_open", json!({"path": pbip})).await;
    let big = c
        .tool(
            "data_page",
            json!({"docId": "d1", "table": "Sales", "limit": 60}),
        )
        .await;
    assert_eq!(big["isError"], true);
    assert!(big.get("structuredContent").is_none());
    let text = big["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("[truncated: the result is"), "{text}");
    assert!(text.contains("limit, offset"), "{text}");
    c.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn expected_values_through_mcp() {
    let (Some(_pbip), Some(_map), Some(exp)) = (fixture(PBIP), fixture(BLOB_MAP), expected())
    else {
        return;
    };
    let mut c = Client::in_root(&reference_root());
    c.initialize("2025-06-18").await;
    let doc = c
        .ok(
            "doc_open",
            json!({"path": PBIP, "blobMap": BLOB_MAP, "source": "query"}),
        )
        .await;
    let doc_id = doc["docId"].as_str().unwrap().to_owned();
    let values = c.ok("measures_values", json!({"docId": doc_id})).await;
    let total = values["measures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "Total Sales")
        .unwrap();
    assert_eq!(
        total["formatted"],
        exp.measure(None, "Total Sales"),
        "{total}"
    );
    let stores = c
        .ok(
            "measures_values",
            json!({"docId": doc_id, "report": "Stores"}),
        )
        .await;
    let total = stores["measures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "Total Sales")
        .unwrap();
    assert_eq!(
        total["display"],
        exp.measure(Some("Stores"), "Total Sales"),
        "{total}"
    );
    c.finish().await;
}
