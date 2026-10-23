//! `dashcraft mcp` end to end: the binary speaks MCP over pipes (initialize,
//! tools/list), keeps stdout to protocol messages and confines agents to
//! the current folder without `--read-root`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn tmp_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("pbiview-mcp-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap()
}

#[test]
fn mcp_initializes_and_lists_tools_over_stdio() {
    let dir = tmp_dir();
    let mut child = Command::new(env!("CARGO_BIN_EXE_dashcraft"))
        .args(["mcp", "--tool-names", "underscore"])
        .current_dir(dir.path())
        .env_remove("RUST_LOG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn pbiview mcp");
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut send = |msg: Value| {
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
    };
    let mut recv = || {
        let mut line = String::new();
        assert!(out.read_line(&mut line).unwrap() > 0, "server exited");
        let msg: Value = serde_json::from_str(&line)
            .unwrap_or_else(|e| panic!("stdout is not a JSON-RPC message ({e}): {line}"));
        assert_eq!(msg["jsonrpc"], "2.0", "{msg}");
        msg
    };

    send(
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": {"name": "cli-test", "version": "1"}}}),
    );
    let init = recv();
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(
        init["result"]["capabilities"]["tools"]["listChanged"],
        false
    );
    let canonical = dir.path().canonicalize().unwrap();
    assert!(
        init["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains(canonical.to_str().unwrap()),
        "{init}"
    );
    send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    send(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    let list = recv();
    assert_eq!(list["id"], 2);
    let tools = list["result"]["tools"].as_array().unwrap();
    assert!(tools.len() > 20, "{list}");
    assert!(tools.iter().any(|t| t["name"] == "doc_open"));
    assert!(tools.iter().all(|t| t["inputSchema"]["type"] == "object"));
    send(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": {"name": "doc_open", "arguments": {"path": "/no/such/model.pbip"}}}));
    let denied = recv();
    assert_eq!(denied["result"]["isError"], true, "{denied}");
    assert!(
        denied["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("pathDenied")
    );

    drop(stdin);
    let status = child.wait().unwrap();
    let mut rest = String::new();
    out.read_to_string(&mut rest).unwrap();
    assert!(rest.trim().is_empty(), "unexpected stdout: {rest}");
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(status.success(), "stderr: {stderr}");
    assert!(stderr.contains("no --read-root"), "{stderr}");
}

#[test]
fn mcp_rejects_unknown_filters_and_a_filesystem_root_default() {
    let dir = tmp_dir();
    let o = Command::new(env!("CARGO_BIN_EXE_dashcraft"))
        .args(["mcp", "--read-root", ".", "--allow", "rendr"])
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(o.stdout.is_empty());
    assert!(String::from_utf8_lossy(&o.stderr).contains("--allow rendr"));

    let root = dir.path().ancestors().last().unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_dashcraft"))
        .arg("mcp")
        .current_dir(root)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(o.stdout.is_empty());
    assert!(String::from_utf8_lossy(&o.stderr).contains("pass --read-root"));
}
