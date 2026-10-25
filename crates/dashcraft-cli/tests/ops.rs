//! `dashcraft commands`, `run` (single commands and batches) and `serve`
//! (JSON-lines RPC) end to end on the synthetic Acme Outfitters example,
//! against its `expected.json`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use common::{BLOB_MAP, PBIP, acme_with_stale_query_ref, expected};
use serde_json::{Value, json};

const CSV: &str = "examples/acme-outfitters/data/orders.csv";

fn reference_root() -> PathBuf {
    std::env::var_os("DASHCRAFT_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
}

fn has(rel: &str) -> bool {
    let ok = reference_root().join(rel).exists();
    if !ok {
        println!("SKIP: {rel} missing");
    }
    ok
}

fn dashcraft_cmd(args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dashcraft"));
    cmd.args(args)
        .current_dir(reference_root())
        .env_remove("RUST_LOG");
    cmd
}

fn dashcraft(args: &[&str]) -> Output {
    dashcraft_cmd(args).output().expect("run dashcraft")
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn json_out(o: &Output) -> Value {
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON ({e}): {}\nstderr: {}",
            String::from_utf8_lossy(&o.stdout),
            stderr(o)
        )
    })
}

fn assert_exit(o: &Output, code: i32) {
    assert_eq!(o.status.code(), Some(code), "stderr:\n{}", stderr(o));
}

#[test]
fn commands_lists_the_catalog() {
    if !has(PBIP) {
        return;
    }
    let o = dashcraft(&["commands", "--json"]);
    assert_exit(&o, 0);
    let v = json_out(&o);
    let commands = v["commands"].as_array().unwrap();
    assert_eq!(v["count"], json!(commands.len()));
    let open = commands.iter().find(|c| c["id"] == "doc.open").unwrap();
    assert_eq!(open["category"], "workspace");
    assert_eq!(open["agentSafe"], true);
    assert!(open.get("paramsSchema").is_none());

    let schemas = json_out(&dashcraft(&["commands", "--schemas", "doc.open"]));
    assert_eq!(schemas["paramsSchema"]["required"], json!(["path"]));

    let text = dashcraft(&["commands"]);
    assert_exit(&text, 0);
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("render.page") && text.contains("[long, writes]"));

    let md = dashcraft(&["commands", "--markdown"]);
    assert_exit(&md, 0);
    assert!(
        String::from_utf8(md.stdout)
            .unwrap()
            .contains("### `doc.open`")
    );

    let bad = dashcraft(&["commands", "doc.opne"]);
    assert_exit(&bad, 1);
    assert!(
        stderr(&bad).contains("did you mean doc.open"),
        "{}",
        stderr(&bad)
    );
}

#[test]
fn run_executes_one_command() {
    let (true, Some(exp)) = (has(PBIP), expected()) else {
        return;
    };
    let o = dashcraft(&["run", "doc.detect", "-p", &format!("path={PBIP}")]);
    assert_exit(&o, 0);
    assert_eq!(json_out(&o)["detection"]["format"], "pbip");

    let o = dashcraft(&["run", "report.pages", "--doc", PBIP, "-p", "report=Stores"]);
    assert_exit(&o, 0);
    let v = json_out(&o);
    let stores = exp.report("Stores");
    assert_eq!(v["report"], stores["report"]);
    let pages: Vec<&Value> = v["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| &p["displayName"])
        .collect();
    let want: Vec<&Value> = stores["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| &p["page"])
        .collect();
    assert_eq!(pages, want);

    let o = dashcraft(&[
        "run",
        "doc.inspect",
        "--doc",
        PBIP,
        "--params",
        r#"{"includeExpressions": false}"#,
    ]);
    assert_exit(&o, 0);
    assert_eq!(json_out(&o)["counts"]["visuals"], 80);
    let o = dashcraft(&["run", "doc.lint", "--doc", PBIP, "-p", "deny=warning"]);
    assert_exit(&o, 0);
    assert_eq!(json_out(&o)["summary"]["warnings"], 0);

    // Lint warnings do not fail; --deny-style `deny` makes `ok` false → 2.
    let dir = tempfile::Builder::new()
        .prefix("dashcraft-run-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let stale = dir.path().join("acme");
    acme_with_stale_query_ref(&stale);
    let stale = stale.join("AcmeOutfitters.pbip");
    let stale = stale.to_str().unwrap();
    let o = dashcraft(&["run", "doc.lint", "--doc", stale]);
    assert_exit(&o, 0);
    assert_eq!(json_out(&o)["summary"]["warnings"], 1);
    let o = dashcraft(&["run", "doc.lint", "--doc", stale, "-p", "deny=warning"]);
    assert_exit(&o, 2);

    // Errors: exit 1 with the string code; DAX errors exit 2.
    let o = dashcraft(&["run", "report.pages", "-p", "docId=d9"]);
    assert_exit(&o, 1);
    assert!(stderr(&o).contains("[noDocument]"), "{}", stderr(&o));
    let o = dashcraft(&["run", "report.pages", "--doc", PBIP, "-p", "colour=red"]);
    assert_exit(&o, 1);
    assert!(stderr(&o).contains("[invalidParams]"), "{}", stderr(&o));
    let o = dashcraft(&["run", "no.such"]);
    assert_exit(&o, 1);
    let o = dashcraft(&["run", "doc.open", "-p", "path=missing.pbip"]);
    assert_exit(&o, 1);
    assert!(stderr(&o).contains("[notFound]"), "{}", stderr(&o));
    let o = dashcraft(&[
        "run",
        "dax.query",
        "--doc",
        PBIP,
        "--params",
        r#"{"query": "EVALUATE NOPE("}"#,
    ]);
    assert_exit(&o, 2);
    assert!(stderr(&o).contains("[query]"), "{}", stderr(&o));
}

#[test]
fn run_batch_interpolates_earlier_results() {
    if !has(PBIP) {
        return;
    }
    let dir = tempfile::Builder::new()
        .prefix("dashcraft-batch-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let steps = dir.path().join("steps.json");
    std::fs::write(
        &steps,
        json!([
            {"id": "doc.open", "params": {"path": PBIP, "autoRefresh": false}, "as": "d"},
            {"id": "report.pages", "params": {"docId": "${d.docId}", "report": "Stores"}, "as": "p"},
            {"id": "report.visuals", "params": {"docId": "${d.docId}", "report": "Stores", "page": "${p.pages.1.name}"}},
            {"id": "text.parsePeople", "params": {"text": "doc ${d.docId}: $${literal}"}}
        ])
        .to_string(),
    )
    .unwrap();
    let o = dashcraft(&["run", "--batch", steps.to_str().unwrap()]);
    assert_exit(&o, 0);
    let results = json_out(&o);
    let results = results.as_array().unwrap();
    assert_eq!(results.len(), 4);
    assert!(results.iter().all(|r| r["ok"] == true), "{results:?}");
    assert_eq!(results[0]["as"], "d");
    assert_eq!(results[2]["result"]["pageDisplayName"], "Store Detail");
    assert_eq!(results[3]["result"]["display"], "doc d1: ${literal}");

    // `--doc` pre-opens and injects docId; a failing step stops the batch.
    std::fs::write(
        &steps,
        json!({"steps": [
            {"id": "report.list"},
            {"id": "report.pages", "params": {"report": "Nope"}},
            {"id": "doc.list"}
        ]})
        .to_string(),
    )
    .unwrap();
    let o = dashcraft(&["run", "--batch", steps.to_str().unwrap(), "--doc", PBIP]);
    assert_exit(&o, 1);
    let results = json_out(&o);
    assert_eq!(results.as_array().unwrap().len(), 2);
    assert_eq!(results[0]["result"]["reports"].as_array().unwrap().len(), 3);
    assert_eq!(results[1]["error"]["code"], "notFound");
    let o = dashcraft(&[
        "run",
        "--batch",
        steps.to_str().unwrap(),
        "--doc",
        PBIP,
        "--stop-on-error",
        "false",
    ]);
    assert_eq!(json_out(&o).as_array().unwrap().len(), 3);
}

/// A running `dashcraft serve`.
struct Server {
    child: std::process::Child,
    out: BufReader<std::process::ChildStdout>,
}

impl Server {
    fn start(extra: &[&str]) -> Self {
        let mut args = vec!["serve", "--stdio"];
        args.extend_from_slice(extra);
        let mut child = dashcraft_cmd(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn dashcraft serve");
        let out = BufReader::new(child.stdout.take().unwrap());
        Self { child, out }
    }

    fn send(&mut self, msg: &Value) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
    }

    /// The response to `id` (progress notifications are skipped).
    fn response(&mut self, id: i64) -> Value {
        loop {
            let mut line = String::new();
            assert!(self.out.read_line(&mut line).unwrap() > 0, "server exited");
            let msg: Value = serde_json::from_str(&line).unwrap();
            if msg["method"] == "progress" {
                continue;
            }
            assert_eq!(msg["id"], id, "{msg}");
            return msg;
        }
    }

    /// The next message.
    fn response_any(&mut self) -> Value {
        let mut line = String::new();
        assert!(self.out.read_line(&mut line).unwrap() > 0, "server exited");
        serde_json::from_str(&line).unwrap()
    }

    fn finish(mut self) {
        drop(self.child.stdin.take());
        let status = self.child.wait().unwrap();
        assert!(status.success());
    }
}

#[test]
fn serve_answers_json_lines_requests() {
    if !has(PBIP) {
        return;
    }
    let mut s = Server::start(&[]);
    s.send(&json!({"jsonrpc": "2.0", "id": 1, "method": "doc.open", "params": {"path": PBIP, "autoRefresh": false}}));
    let open = s.response(1);
    assert_eq!(open["jsonrpc"], "2.0");
    assert_eq!(open["result"]["docId"], "d1");
    s.send(&json!({"jsonrpc": "2.0", "id": 2, "method": "report.list", "params": {"docId": "d1"}}));
    let list = s.response(2);
    assert_eq!(list["result"]["reports"].as_array().unwrap().len(), 3);
    s.send(&json!({"jsonrpc": "2.0", "id": "x", "method": "report.pages", "params": {"docId": "d1", "report": 7}}));
    let bad = s.response_any();
    assert_eq!(bad["id"], "x");
    assert_eq!(bad["error"]["code"], -32602);
    assert_eq!(bad["error"]["data"]["code"], "invalidParams");
    s.finish();
}

#[test]
fn serve_cancels_a_long_refresh() {
    let (true, true, true, Some(exp)) = (has(PBIP), has(BLOB_MAP), has(CSV), expected()) else {
        return;
    };
    let mut s = Server::start(&[]);
    s.send(&json!({"jsonrpc": "2.0", "id": 1, "method": "doc.open", "params": {"path": PBIP, "blobMap": BLOB_MAP, "source": "query", "autoRefresh": false}}));
    assert_eq!(s.response(1)["result"]["docId"], "d1");
    s.send(
        &json!({"jsonrpc": "2.0", "id": 2, "method": "data.refresh", "params": {"docId": "d1"}}),
    );
    s.send(&json!({"jsonrpc": "2.0", "method": "$/cancel", "params": {"id": 2}}));
    let refresh = s.response(2);
    // The refresh is usually cancelled; on a very fast machine it may have
    // finished first. Either way the server answers and keeps serving.
    if refresh.get("error").is_some() {
        assert_eq!(refresh["error"]["code"], -32800, "{refresh}");
        assert_eq!(refresh["error"]["data"]["code"], "cancelled");
    } else {
        assert_eq!(refresh["result"]["ok"], true, "{refresh}");
    }
    s.send(
        &json!({"jsonrpc": "2.0", "id": 3, "method": "data.refresh", "params": {"docId": "d1"}}),
    );
    assert_eq!(s.response(3)["result"]["ok"], true);
    s.send(&json!({"jsonrpc": "2.0", "id": 4, "method": "dax.query", "params": {"docId": "d1", "query": "EVALUATE ROW(\"n\", [Order Count])"}}));
    let q = s.response(4);
    assert_eq!(
        q["result"]["tables"][0]["rows"][0][0],
        exp.0["measures"]["unfiltered"]["Order Count"]["value"],
        "{q}"
    );
    s.finish();
}

#[test]
fn ai_commands_are_wired_and_offline_by_default() {
    let cfg = tempfile::tempdir().unwrap();
    let status = dashcraft_cmd(&["run", "ai.status"])
        .env("DASHCRAFT_CONFIG_DIR", cfg.path())
        .output()
        .unwrap();
    assert_exit(&status, 0);
    assert_eq!(json_out(&status)["enabled"], false);

    // `commands --markdown` stays the core catalog (docs/COMMANDS.md).
    let md = dashcraft(&["commands", "--markdown"]);
    assert!(!String::from_utf8_lossy(&md.stdout).contains("ai.status"));
    let all = dashcraft(&["commands"]);
    assert!(String::from_utf8_lossy(&all.stdout).contains("ai.explainMeasure"));

    if !has(PBIP) {
        return;
    }
    let preview = dashcraft_cmd(&[
        "run",
        "ai.preview",
        "--doc",
        PBIP,
        "-p",
        "capability=dax.explain",
        "-p",
        "measure=Total Sales",
    ])
    .env("DASHCRAFT_CONFIG_DIR", cfg.path())
    .output()
    .unwrap();
    assert_exit(&preview, 0);
    let text = String::from_utf8_lossy(&preview.stdout);
    assert!(text.contains("Total Sales"), "{text}");
    assert!(text.contains("SUM(Orders[Line Amount])"), "{text}");
}

#[test]
fn ai_login_never_takes_the_key_from_arguments() {
    // No terminal and no --stdin: refused before touching the keychain.
    let o = dashcraft_cmd(&["ai", "login", "--provider", "openai"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_exit(&o, 1);
    assert!(stderr(&o).contains("--stdin"), "{}", stderr(&o));
    // Invalid provider ids are rejected before the keychain is used.
    let mut child = dashcraft_cmd(&["ai", "login", "--provider", "Not Valid!", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"sk-test\n").unwrap();
    let o = child.wait_with_output().unwrap();
    assert_exit(&o, 1);
    assert!(!stderr(&o).contains("sk-test"), "{}", stderr(&o));
}
