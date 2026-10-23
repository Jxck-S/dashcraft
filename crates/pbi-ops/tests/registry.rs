//! The catalog's invariants and `Registry::execute`'s guarantees: schemas,
//! examples, error mapping, caller policy, panics, cancellation, per-document
//! ordering; and that docs/COMMANDS.md is up to date.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{Runner, mini_pbip, tmp_dir};
use pbi_ops::markdown::catalog_markdown;
use pbi_ops::{
    CallerKind, Category, Command, CommandMeta, Ctx, OpError, OpErrorCode, Registry, Workspace,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

fn id_is_dotted(id: &str) -> bool {
    let mut parts = id.split('.');
    let first = parts.next().unwrap_or_default();
    let ok_part = |p: &str| {
        !p.is_empty()
            && p.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && p.chars().all(|c| c.is_ascii_alphanumeric())
    };
    ok_part(first) && parts.clone().count() >= 1 && parts.all(ok_part)
}

#[test]
fn every_command_has_metadata_schemas_and_valid_examples() {
    let registry = Registry::default_catalog();
    assert!(registry.len() >= 40, "{} commands", registry.len());
    let mut ids = HashSet::new();
    for spec in registry.list() {
        let m = &spec.meta;
        assert!(ids.insert(m.id), "duplicate id {}", m.id);
        assert!(id_is_dotted(m.id), "{}: not a dotted camelCase id", m.id);
        assert!(!m.title.is_empty(), "{}: title", m.id);
        assert!(
            !m.summary.is_empty() && !m.summary.contains('\n') && m.summary.len() <= 200,
            "{}: summary must be one short line",
            m.id
        );
        assert!(!m.since.is_empty(), "{}: since", m.id);
        assert!(
            registry.describe(m.id).is_some_and(|s| s.meta == *m),
            "{}: describe",
            m.id
        );

        // Parameters: a closed object schema.
        let params = &spec.params_schema;
        assert_eq!(params["type"], "object", "{}: params type", m.id);
        assert_eq!(
            params["additionalProperties"],
            json!(false),
            "{}: params must deny unknown fields",
            m.id
        );
        assert!(
            params["$schema"]
                .as_str()
                .is_some_and(|s| s.contains("2020-12")),
            "{}: draft 2020-12",
            m.id
        );
        let props = params["properties"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        let required: Vec<&str> = params["required"]
            .as_array()
            .map(|r| r.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        if m.mutates_document {
            assert!(
                required.contains(&"docId"),
                "{}: mutating commands are ordered by their required docId",
                m.id
            );
        }
        // Results: an object schema.
        assert!(
            spec.output_schema.is_object() && spec.output_schema["type"] == "object",
            "{}: output schema",
            m.id
        );

        // Examples parse, validate and fit the schema's top level.
        assert!(!m.examples.is_empty(), "{}: no example", m.id);
        for (title, text) in m.examples {
            let example: Value = serde_json::from_str(text)
                .unwrap_or_else(|e| panic!("{} example {title:?} is not JSON: {e}", m.id));
            spec.validate(&example)
                .unwrap_or_else(|e| panic!("{} example {title:?}: {}", m.id, e.message));
            let obj = example.as_object().unwrap();
            for key in obj.keys() {
                assert!(
                    props.contains_key(key),
                    "{} example {title:?}: unknown {key}",
                    m.id
                );
            }
            for key in &required {
                assert!(
                    obj.contains_key(*key),
                    "{} example {title:?}: missing {key}",
                    m.id
                );
            }
        }
    }
    // Every category is used, except opt-in ones that front ends register themselves (`ai`,
    // `pbi_ai::register`).
    for c in Category::ALL.into_iter().filter(|c| c.as_str() != "ai") {
        assert!(
            registry.list().iter().any(|s| s.meta.category == c),
            "no command in {c}"
        );
    }
}

#[test]
fn catalog_covers_the_v1_surface() {
    let registry = Registry::default_catalog();
    for id in [
        "commands.list",
        "commands.describe",
        "doc.detect",
        "doc.open",
        "doc.close",
        "doc.list",
        "doc.inspect",
        "model.tables",
        "model.measures",
        "model.relationships",
        "report.list",
        "report.pages",
        "report.visuals",
        "report.filters",
        "doc.lint",
        "data.refresh",
        "data.tables",
        "data.page",
        "data.columnStats",
        "data.loadCsv",
        "data.addDerivedColumn",
        "data.removeDerivedColumn",
        "dax.query",
        "dax.evaluateMeasure",
        "measures.values",
        "render.page",
        "render.visual",
        "canvas.click",
        "canvas.scroll",
        "canvas.reset",
        "export.table",
        "export.model",
        "convert.pbip",
        "convert.pbit",
        "text.parsePeople",
        "text.previewPeople",
    ] {
        assert!(registry.describe(id).is_some(), "missing {id}");
    }
    let writers: Vec<&str> = registry
        .list()
        .iter()
        .filter(|s| s.meta.writes_files)
        .map(|s| s.meta.id)
        .collect();
    assert!(writers.contains(&"convert.pbip") && writers.contains(&"export.table"));
}

#[test]
fn commands_md_is_up_to_date() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/COMMANDS.md");
    let generated = catalog_markdown(&Registry::default_catalog());
    if std::env::var_os("PBIVIEW_BLESS").is_some() {
        std::fs::write(&path, &generated).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "docs/COMMANDS.md is stale: run `cargo run -q -p pbiview-cli --bin dashcraft -- commands --markdown > docs/COMMANDS.md` \
         (or PBIVIEW_BLESS=1 cargo test -p pbi-ops --test registry)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_requests_fail_before_running() {
    let r = Runner::new();
    let e = r.try_run("doc.opne", json!({})).await.unwrap_err();
    assert_eq!(e.code, OpErrorCode::UnknownCommand);
    assert!(e.message.contains("doc.open"), "{}", e.message);
    let e = r
        .try_run("report.pages", json!({"docId": "d1", "bogus": 1}))
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::InvalidParams);
    assert!(e.message.contains("bogus"), "{}", e.message);
    let e = r.try_run("report.pages", Value::Null).await.unwrap_err();
    assert_eq!(e.code, OpErrorCode::InvalidParams, "missing docId");
    let e = r
        .try_run("report.pages", json!({"docId": "d9"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::NoDocument);
    // `null` params are `{}`.
    let list = r.run("doc.list", Value::Null).await;
    assert_eq!(list["documents"], json!([]));
    let e = r
        .try_run(
            "data.page",
            json!({"docId": "d1", "table": "T", "limit": 100_000}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::InvalidParams);
}

#[tokio::test(flavor = "multi_thread")]
async fn meta_commands_describe_the_registry() {
    let r = Runner::new();
    let list = r.run("commands.list", json!({})).await;
    assert_eq!(list["count"], json!(r.registry.len()));
    assert!(list["commands"][0].get("paramsSchema").is_none());
    let with = r
        .run("commands.list", json!({"schemas": true, "category": "dax"}))
        .await;
    assert!(
        with["commands"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["category"] == "dax")
    );
    assert!(with["commands"][0]["paramsSchema"].is_object());
    let one = r.run("commands.describe", json!({"id": "doc.open"})).await;
    assert_eq!(one["id"], "doc.open");
    assert_eq!(one["paramsSchema"]["required"], json!(["path"]));
    assert!(one["outputSchema"]["properties"]["docId"].is_object());
}

// ---------------------------------------------------------------------------
// Execution guarantees, with test commands
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TestParams {
    #[serde(default)]
    doc_id: Option<String>,
    #[serde(default)]
    tag: String,
    #[serde(default)]
    sleep_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct TestOut {
    tag: String,
}

const TEST_META: CommandMeta = CommandMeta {
    id: "",
    title: "test",
    summary: "test",
    category: Category::Meta,
    mutates_document: false,
    long_running: false,
    agent_safe: true,
    writes_files: false,
    invokes_other_commands: false,
    since: "0.0.0",
    examples: &[("t", "{}")],
};

struct Panics;
impl Command for Panics {
    type Params = TestParams;
    type Output = TestOut;
    fn meta() -> CommandMeta {
        CommandMeta {
            id: "test.panics",
            ..TEST_META
        }
    }
    async fn run(_: &Workspace, p: TestParams, _: &Ctx) -> Result<TestOut, OpError> {
        if p.tag != "never" {
            panic!("boom {}", p.tag);
        }
        Ok(TestOut { tag: p.tag })
    }
}

struct Private;
impl Command for Private {
    type Params = TestParams;
    type Output = TestOut;
    fn meta() -> CommandMeta {
        CommandMeta {
            id: "test.private",
            agent_safe: false,
            ..TEST_META
        }
    }
    async fn run(_: &Workspace, p: TestParams, _: &Ctx) -> Result<TestOut, OpError> {
        Ok(TestOut { tag: p.tag })
    }
}

struct Sleeps;
impl Command for Sleeps {
    type Params = TestParams;
    type Output = TestOut;
    fn meta() -> CommandMeta {
        CommandMeta {
            id: "test.sleeps",
            ..TEST_META
        }
    }
    async fn run(_: &Workspace, p: TestParams, ctx: &Ctx) -> Result<TestOut, OpError> {
        ctx.progress("sleeping", Some(0.5));
        tokio::time::sleep(Duration::from_millis(p.sleep_ms)).await;
        Ok(TestOut { tag: p.tag })
    }
}

static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());
static RUNNING: AtomicBool = AtomicBool::new(false);

struct Mutates;
impl Command for Mutates {
    type Params = TestParams;
    type Output = TestOut;
    fn meta() -> CommandMeta {
        CommandMeta {
            id: "test.mutates",
            mutates_document: true,
            ..TEST_META
        }
    }
    async fn run(_: &Workspace, p: TestParams, _: &Ctx) -> Result<TestOut, OpError> {
        assert!(
            !RUNNING.swap(true, Ordering::SeqCst),
            "overlapping mutations"
        );
        LOG.lock().unwrap().push(format!("start {}", p.tag));
        tokio::time::sleep(Duration::from_millis(p.sleep_ms)).await;
        LOG.lock().unwrap().push(format!("end {}", p.tag));
        RUNNING.store(false, Ordering::SeqCst);
        Ok(TestOut { tag: p.tag })
    }
}

fn test_registry() -> Registry {
    Registry::new()
        .with::<Panics>()
        .with::<Private>()
        .with::<Sleeps>()
        .with::<Mutates>()
        .with::<pbi_ops::commands::workspace::DocOpen>()
}

#[tokio::test(flavor = "multi_thread")]
async fn panics_become_internal_errors() {
    let r = Runner::with(test_registry(), Workspace::new());
    let e = r
        .try_run("test.panics", json!({"tag": "x"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::Internal);
    assert!(e.message.contains("boom x"), "{}", e.message);
    // The registry still works afterwards.
    let ok = r.run("test.sleeps", json!({"tag": "after"})).await;
    assert_eq!(ok["tag"], "after");
}

#[tokio::test(flavor = "multi_thread")]
async fn agents_only_run_agent_safe_commands() {
    let registry = test_registry();
    let ws = Workspace::new();
    let agent = Ctx::new(CallerKind::Agent);
    let e = registry
        .execute(&ws, "test.private", json!({}), &agent)
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::Unsupported);
    let cli = Ctx::new(CallerKind::Cli);
    assert!(
        registry
            .execute(&ws, "test.private", json!({}), &cli)
            .await
            .is_ok()
    );
    assert!(registry.agent_catalog().describe("test.private").is_none());
    assert!(registry.agent_catalog().describe("test.sleeps").is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn cancellation_ends_a_run_and_progress_is_reported() {
    let registry = test_registry();
    let ws = Workspace::new();
    let token = CancellationToken::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let ctx = Ctx::new(CallerKind::Rpc)
        .with_cancel(token.clone())
        .with_progress(move |e| sink.lock().unwrap().push(e));
    let canceller = {
        let token = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            token.cancel();
        })
    };
    let e = registry
        .execute(&ws, "test.sleeps", json!({"sleepMs": 10_000}), &ctx)
        .await
        .unwrap_err();
    canceller.await.unwrap();
    assert_eq!(e.code, OpErrorCode::Cancelled);
    {
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].command, "test.sleeps");
        assert_eq!(events[0].fraction, Some(0.5));
    }
    // A cancelled context refuses to start.
    let e = registry
        .execute(&ws, "test.sleeps", json!({}), &ctx)
        .await
        .unwrap_err();
    assert_eq!(e.code, OpErrorCode::Cancelled);
}

#[tokio::test(flavor = "multi_thread")]
async fn mutating_commands_run_one_at_a_time_per_document() {
    let dir = tmp_dir("pbi-ops-order-");
    let pbip = mini_pbip(dir.path());
    let r = Arc::new(Runner::with(test_registry(), Workspace::new()));
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    let first = {
        let (r, doc) = (Arc::clone(&r), doc.clone());
        tokio::spawn(async move {
            r.run(
                "test.mutates",
                json!({"docId": doc, "tag": "a", "sleepMs": 150}),
            )
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(30)).await;
    let second = r
        .run(
            "test.mutates",
            json!({"docId": doc, "tag": "b", "sleepMs": 0}),
        )
        .await;
    assert_eq!(second["tag"], "b");
    first.await.unwrap();
    assert_eq!(
        *LOG.lock().unwrap(),
        ["start a", "end a", "start b", "end b"]
    );
}
