//! `dashcraft lsp` end to end: a scripted Language Server Protocol client drives the binary
//! over pipes (`Content-Length` framing): the lifecycle, protocol errors, DAX, M and TMDL
//! features, a model from `--model` or the `model` initialization option, and a TMDL
//! `definition` folder read from disk.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{Value, json};

/// How long one answer may take (debug builds on CI runners are slow).
const TIMEOUT: Duration = Duration::from_secs(120);

fn reference_root() -> PathBuf {
    std::env::var_os("DASHCRAFT_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
}

fn tmp_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("dashcraft-lsp-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap()
}

/// The `file:` URI of a local path, as editors send it.
fn file_uri(path: &Path) -> String {
    let s = path
        .to_str()
        .unwrap()
        .replace('\\', "/")
        .replace(' ', "%20");
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("file:///{s}")
    }
}

fn read_frame(r: &mut impl BufRead) -> Option<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if r.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            if length.is_some() {
                break;
            }
            continue;
        }
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("content-length") {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0; length?];
    r.read_exact(&mut body).ok()?;
    Some(serde_json::from_slice(&body).expect("the server sent JSON"))
}

struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: mpsc::Receiver<Value>,
    stderr: Option<JoinHandle<String>>,
    /// Notifications read while waiting for a response.
    notifications: VecDeque<Value>,
    next_id: i64,
}

impl Client {
    fn start(args: &[&str], dir: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_dashcraft"))
            .arg("lsp")
            .args(args)
            .current_dir(dir)
            .env_remove("RUST_LOG")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn dashcraft lsp");
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            while let Some(msg) = read_frame(&mut r) {
                if tx.send(msg).is_err() {
                    break;
                }
            }
        });
        let mut stderr = child.stderr.take().unwrap();
        let stderr = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            s
        });
        let stdin = child.stdin.take();
        Self {
            child,
            stdin,
            rx,
            stderr: Some(stderr),
            notifications: VecDeque::new(),
            next_id: 1,
        }
    }

    fn write_raw(&mut self, bytes: &[u8]) {
        let stdin = self.stdin.as_mut().unwrap();
        stdin.write_all(bytes).unwrap();
        stdin.flush().unwrap();
    }

    fn send(&mut self, msg: &Value) {
        let body = msg.to_string();
        let frame = format!("Content-Length: {}\r\n\r\n{body}", body.len());
        self.write_raw(frame.as_bytes());
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    fn recv(&mut self) -> Value {
        let msg = self
            .rx
            .recv_timeout(TIMEOUT)
            .unwrap_or_else(|e| panic!("no message from the server: {e}"));
        assert_eq!(msg["jsonrpc"], "2.0", "{msg}");
        msg
    }

    /// The response to request `id`, keeping notifications for later.
    fn response(&mut self, id: &Value) -> Value {
        loop {
            let msg = self.recv();
            if msg.get("id") == Some(id) && msg.get("method").is_none() {
                return msg;
            }
            self.notifications.push_back(msg);
        }
    }

    /// Sends a request and returns its whole response.
    fn call(&mut self, method: &str, params: Value) -> Value {
        let id = json!(self.next_id);
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        self.response(&id)
    }

    /// Sends a request and returns its result, failing on an error.
    fn request(&mut self, method: &str, params: Value) -> Value {
        let msg = self.call(method, params);
        assert!(msg.get("error").is_none(), "{method}: {msg}");
        msg["result"].clone()
    }

    /// The next notification `method` matching `pred`.
    fn notification(&mut self, method: &str, pred: impl Fn(&Value) -> bool) -> Value {
        if let Some(i) = self
            .notifications
            .iter()
            .position(|n| n["method"] == method && pred(&n["params"]))
        {
            return self.notifications.remove(i).unwrap()["params"].clone();
        }
        loop {
            let msg = self.recv();
            if msg["method"] == method && pred(&msg["params"]) {
                return msg["params"].clone();
            }
            self.notifications.push_back(msg);
        }
    }

    /// The diagnostics published for `uri` (at `version`, when given).
    fn diagnostics(&mut self, uri: &str, version: Option<i64>) -> Vec<Value> {
        let params = self.notification("textDocument/publishDiagnostics", |p| {
            p["uri"] == uri && version.is_none_or(|v| p["version"] == v)
        });
        params["diagnostics"].as_array().unwrap().clone()
    }

    fn initialize(&mut self, params: Value) -> Value {
        let result = self.request("initialize", params);
        self.notify("initialized", json!({}));
        result
    }

    fn open(&mut self, uri: &str, language: &str, text: &str) {
        self.notify(
            "textDocument/didOpen",
            json!({"textDocument": {"uri": uri, "languageId": language, "version": 1, "text": text}}),
        );
    }

    /// `shutdown`, `exit`, and the process's end; returns the status and stderr.
    fn shut_down(mut self) -> (ExitStatus, String) {
        assert_eq!(self.request("shutdown", Value::Null), Value::Null);
        self.notify("exit", Value::Null);
        self.finish()
    }

    fn finish(mut self) -> (ExitStatus, String) {
        drop(self.stdin.take());
        let status = self.child.wait().unwrap();
        let stderr = self.stderr.take().unwrap().join().unwrap();
        (status, stderr)
    }
}

fn at(line: u32, character: u32) -> Value {
    json!({"line": line, "character": character})
}

fn doc_at(uri: &str, line: u32, character: u32) -> Value {
    json!({"textDocument": {"uri": uri}, "position": at(line, character)})
}

fn range(l1: u32, c1: u32, l2: u32, c2: u32) -> Value {
    json!({"start": at(l1, c1), "end": at(l2, c2)})
}

fn labels(completion: &Value) -> Vec<String> {
    completion["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap().to_owned())
        .collect()
}

fn item<'a>(completion: &'a Value, label: &str) -> &'a Value {
    completion["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["label"] == label)
        .unwrap_or_else(|| panic!("no `{label}` in {completion}"))
}

#[test]
fn lsp_serves_dax_without_a_model() {
    let dir = tmp_dir();
    let mut c = Client::start(&["--stdio"], dir.path());
    let init = c.initialize(json!({"processId": null, "rootUri": null, "capabilities": {}}));
    let caps = &init["capabilities"];
    assert_eq!(init["serverInfo"]["name"], "dashcraft", "{init}");
    assert_eq!(caps["positionEncoding"], "utf-16");
    assert_eq!(caps["textDocumentSync"]["change"], 2);
    assert!(
        caps["completionProvider"]["triggerCharacters"]
            .as_array()
            .unwrap()
            .contains(&json!("["))
    );
    assert_eq!(
        caps["semanticTokensProvider"]["legend"]["tokenTypes"][0],
        "keyword"
    );

    let uri = "untitled:Measure.dax";
    let text = "VAR total = ABS(-1)\nRETURN SUMM(total) + tot";
    c.open(uri, "dax", text);
    let diags = c.diagnostics(uri, Some(1));
    let summ = diags
        .iter()
        .find(|d| d["code"] == "unsupportedFunction")
        .unwrap_or_else(|| panic!("{diags:?}"));
    assert_eq!(summ["range"], range(1, 7, 1, 11));
    assert_eq!(summ["severity"], 2);
    assert_eq!(summ["source"], "dashcraft");
    assert!(
        summ["message"].as_str().unwrap().contains("`SUM`"),
        "{summ}"
    );
    assert_eq!(summ["data"]["replacement"], "SUM");

    let actions = c.request(
        "textDocument/codeAction",
        json!({"textDocument": {"uri": uri}, "range": summ["range"],
            "context": {"diagnostics": [summ]}}),
    );
    assert_eq!(actions[0]["kind"], "quickfix", "{actions}");
    assert_eq!(
        actions[0]["edit"]["changes"][uri][0],
        json!({"range": range(1, 7, 1, 11), "newText": "SUM"})
    );

    // Completion of the variable `total` from `tot` at the end.
    let done = c.request("textDocument/completion", doc_at(uri, 1, 24));
    let total = item(&done, "total");
    assert_eq!(total["kind"], 6, "{total}");
    assert_eq!(total["textEdit"]["range"], range(1, 21, 1, 24));
    assert_eq!(total["textEdit"]["newText"], "total");

    let hover = c.request("textDocument/hover", doc_at(uri, 0, 13));
    assert_eq!(hover["contents"]["kind"], "markdown");
    assert!(
        hover["contents"]["value"].as_str().unwrap().contains("ABS"),
        "{hover}"
    );
    assert_eq!(hover["range"], range(0, 12, 0, 15));

    let help = c.request("textDocument/signatureHelp", doc_at(uri, 0, 16));
    let sig = &help["signatures"][0];
    assert!(sig["label"].as_str().unwrap().starts_with("ABS("), "{help}");
    assert_eq!(help["activeParameter"], 0);
    let [s, e] = [
        &sig["parameters"][0]["label"][0],
        &sig["parameters"][0]["label"][1],
    ]
    .map(|v| usize::try_from(v.as_u64().unwrap()).unwrap());
    assert_eq!(s, 4);
    assert!(e > s);

    let def = c.request("textDocument/definition", doc_at(uri, 1, 13));
    assert_eq!(def, json!({"uri": uri, "range": range(0, 4, 0, 9)}));
    let refs = c.request(
        "textDocument/references",
        json!({"textDocument": {"uri": uri}, "position": at(1, 13),
            "context": {"includeDeclaration": false}}),
    );
    assert_eq!(refs, json!([{"uri": uri, "range": range(1, 12, 1, 17)}]));
    let marks = c.request("textDocument/documentHighlight", doc_at(uri, 0, 5));
    assert_eq!(
        marks,
        json!([{"range": range(0, 4, 0, 9), "kind": 3}, {"range": range(1, 12, 1, 17), "kind": 2}])
    );

    let tokens = c.request(
        "textDocument/semanticTokens/full",
        json!({"textDocument": {"uri": uri}}),
    );
    let data: Vec<u64> = tokens["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    assert_eq!(data.len() % 5, 0);
    // `VAR` keyword, then `total` declared as a variable.
    assert_eq!(data.get(..10), Some(&[0, 0, 3, 0, 0, 0, 4, 5, 5, 1][..]));
    // `ABS`: a built-in function.
    assert!(
        data.chunks(5).any(|t| t.get(2..) == Some(&[3, 1, 2][..])),
        "{data:?}"
    );

    // An incremental change fixes the name: the warning goes.
    c.notify(
        "textDocument/didChange",
        json!({"textDocument": {"uri": uri, "version": 2},
            "contentChanges": [{"range": range(1, 7, 1, 11), "text": "SUM"}]}),
    );
    let diags = c.diagnostics(uri, Some(2));
    assert!(
        diags.iter().all(|d| d["code"] != "unsupportedFunction"),
        "{diags:?}"
    );
    let hover = c.request("textDocument/hover", doc_at(uri, 1, 8));
    assert!(
        hover["contents"]["value"].as_str().unwrap().contains("SUM"),
        "{hover}"
    );

    // Formatting keeps the CRLF line breaks and the final one.
    let f = "untitled:f.dax";
    c.open(f, "dax", "abs( -1 )+2\r\n");
    let edits = c.request(
        "textDocument/formatting",
        json!({"textDocument": {"uri": f}, "options": {"tabSize": 4, "insertSpaces": true}}),
    );
    assert_eq!(
        edits,
        json!([{"range": range(0, 0, 1, 0), "newText": "ABS(-1) + 2\r\n"}])
    );
    c.notify(
        "textDocument/didChange",
        json!({"textDocument": {"uri": f, "version": 2},
            "contentChanges": [{"text": "ABS(-1) + 2\r\n"}]}),
    );
    let edits = c.request(
        "textDocument/formatting",
        json!({"textDocument": {"uri": f}, "options": {"tabSize": 4, "insertSpaces": true}}),
    );
    assert_eq!(edits, json!([]));

    c.notify("textDocument/didClose", json!({"textDocument": {"uri": f}}));
    assert!(c.diagnostics(f, None).is_empty());
    let (status, stderr) = c.shut_down();
    assert!(status.success(), "{status}: {stderr}");
}

#[test]
fn lsp_serves_m_with_quick_fixes() {
    let dir = tmp_dir();
    let mut c = Client::start(&[], dir.path());
    c.initialize(json!({"processId": null, "rootUri": null, "capabilities": {}}));
    let uri = file_uri(&dir.path().join("Query.pq"));
    let text = "let a = Text.Uper(\"x\") in a";
    c.open(&uri, "powerquery", text);
    let diags = c.diagnostics(&uri, Some(1));
    assert_eq!(diags.len(), 1, "{diags:?}");
    let d = &diags[0];
    assert_eq!(d["code"], "unknownName");
    assert_eq!(d["severity"], 1);
    assert_eq!(d["range"], range(0, 8, 0, 17));
    assert!(
        d["message"]
            .as_str()
            .unwrap()
            .ends_with("Did you mean `Text.Upper`?"),
        "{d}"
    );
    let actions = c.request(
        "textDocument/codeAction",
        json!({"textDocument": {"uri": uri}, "range": d["range"],
            "context": {"diagnostics": [d], "only": ["quickfix"]}}),
    );
    assert_eq!(
        actions[0]["edit"]["changes"][&uri][0]["newText"], "Text.Upper",
        "{actions}"
    );
    let none = c.request(
        "textDocument/codeAction",
        json!({"textDocument": {"uri": uri}, "range": d["range"],
            "context": {"diagnostics": [d], "only": ["refactor"]}}),
    );
    assert_eq!(none, json!([]));

    let done = c.request("textDocument/completion", doc_at(&uri, 0, 15));
    assert!(labels(&done).contains(&"Text.Upper".to_owned()), "{done}");
    let s = "untitled:s.m";
    c.open(s, "m", "Text.Upper(\"x\")");
    let help = c.request("textDocument/signatureHelp", doc_at(s, 0, 12));
    let sig = &help["signatures"][0];
    assert!(
        sig["label"].as_str().unwrap().starts_with("Text.Upper("),
        "{help}"
    );
    assert_eq!(help["activeParameter"], 0);
    assert_eq!(sig["parameters"][0]["label"][0], 11, "{help}");
    let f = "untitled:f.m";
    c.open(f, "m", "let a = 1 in a");
    let edits = c.request(
        "textDocument/formatting",
        json!({"textDocument": {"uri": f}, "options": {"tabSize": 2, "insertSpaces": true}}),
    );
    assert_eq!(edits[0]["newText"], "let\n  a = 1\nin\n  a", "{edits}");
    let tokens = c.request(
        "textDocument/semanticTokens/full",
        json!({"textDocument": {"uri": f}}),
    );
    assert_eq!(tokens, json!({"data": []}));
    let (status, stderr) = c.shut_down();
    assert!(status.success(), "{status}: {stderr}");
}

/// A small PBIP-style `definition` folder.
fn write_tmdl_model(def: &Path) {
    std::fs::create_dir_all(def.join("tables")).unwrap();
    std::fs::write(def.join("model.tmdl"), "model Model\n\tculture: en-US\n").unwrap();
    std::fs::write(
        def.join("tables").join("Orders.tmdl"),
        "table Orders\n\tmeasure 'Total Sales' = COUNTROWS(Orders)\n\n\tcolumn Store\n\t\tdataType: string\n\t\tsourceColumn: Store\n",
    )
    .unwrap();
    std::fs::write(
        def.join("tables").join("Stores.tmdl"),
        "table Stores\n\tcolumn Store\n\t\tdataType: string\n\t\tsourceColumn: Store\n",
    )
    .unwrap();
}

#[test]
fn lsp_resolves_tmdl_across_the_definition_folder() {
    let dir = tmp_dir();
    let def = dir.path().join("Shop.SemanticModel").join("definition");
    write_tmdl_model(&def);
    let rel_path = def.join("relationships.tmdl");
    let rel_text = "relationship r1\n\tfromColumn: Orders.Store\n\ttoColumn: Stores.Store\n";
    std::fs::write(&rel_path, rel_text).unwrap();

    let mut c = Client::start(&[], dir.path());
    c.initialize(json!({"processId": null, "rootUri": file_uri(dir.path()), "capabilities": {}}));
    let rel = file_uri(&rel_path);
    c.open(&rel, "tmdl", rel_text);
    assert_eq!(c.diagnostics(&rel, Some(1)), Vec::<Value>::new());

    // Go to the column in a file that is not open.
    let def_loc = c.request("textDocument/definition", doc_at(&rel, 1, 21));
    let target = def_loc["uri"].as_str().unwrap();
    assert!(
        target.starts_with("file://") && target.ends_with("/tables/Orders.tmdl"),
        "{def_loc}"
    );
    assert_eq!(def_loc["range"]["start"], at(3, 0), "{def_loc}");
    let hover = c.request("textDocument/hover", doc_at(&rel, 1, 21));
    assert!(
        hover["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("Store"),
        "{hover}"
    );

    // An open file wins over the disk: the column renamed there no longer resolves.
    let orders = file_uri(&def.join("tables").join("Orders.tmdl"));
    let orders_text = "table Orders\n\tmeasure 'Total Sales' = COUNTROWS(Orders)\n\n\tcolumn Shop\n\t\tdataTyp: string\n";
    c.open(&orders, "tmdl", orders_text);
    let rel_diags = c.diagnostics(&rel, Some(1));
    assert!(
        rel_diags.iter().any(|d| d["code"] == "TMDL108"),
        "{rel_diags:?}"
    );
    let order_diags = c.diagnostics(&orders, Some(1));
    let typo = order_diags
        .iter()
        .find(|d| d["code"] == "TMDL101")
        .unwrap_or_else(|| panic!("{order_diags:?}"));
    assert_eq!(typo["data"]["replacement"], "dataType", "{typo}");
    assert_eq!(typo["range"]["start"]["line"], 4);

    let done = c.request("textDocument/completion", doc_at(&rel, 2, 11));
    assert!(labels(&done).iter().any(|l| l.contains("Stores")), "{done}");

    // Formatting keeps Desktop's tabs.
    c.notify(
        "textDocument/didChange",
        json!({"textDocument": {"uri": rel, "version": 2},
            "contentChanges": [{"text": "relationship r1\n\tfromColumn:Orders.Store\n\ttoColumn:   Stores.Store\n"}]}),
    );
    let edits = c.request(
        "textDocument/formatting",
        json!({"textDocument": {"uri": rel}, "options": {"tabSize": 4, "insertSpaces": true}}),
    );
    assert_eq!(edits[0]["newText"], rel_text, "{edits}");

    let (status, stderr) = c.shut_down();
    assert!(status.success(), "{status}: {stderr}");
}

#[test]
fn lsp_checks_names_against_the_model_and_reopens_it_on_save() {
    let dir = tmp_dir();
    let def = dir.path().join("Shop.SemanticModel").join("definition");
    write_tmdl_model(&def);
    let mut c = Client::start(&[], dir.path());
    // The model option is relative to the workspace folder.
    c.initialize(json!({"processId": null, "capabilities": {},
        "workspaceFolders": [{"uri": file_uri(dir.path()), "name": "shop"}],
        "initializationOptions": {"model": "Shop.SemanticModel/definition"}}));
    let uri = "untitled:q.dax";
    c.open(uri, "dax", "COUNTROWS(Customers) + [Total Sales]");
    let diags = c.diagnostics(uri, Some(1));
    assert!(
        diags.iter().any(|d| d["code"] == "unknownTable"),
        "{diags:?}"
    );
    assert_eq!(diags.len(), 1, "{diags:?}");
    let typing = "untitled:typing.dax";
    c.open(typing, "dax", "COUNTROWS(Customers) + [Tot");
    let done = c.request("textDocument/completion", doc_at(typing, 0, 27));
    let measure = item(&done, "[Total Sales]");
    assert_eq!(measure["kind"], 2);
    assert_eq!(measure["textEdit"]["range"], range(0, 23, 0, 27));

    // A new table saved under the model's folder: the model is opened again.
    let customers = def.join("tables").join("Customers.tmdl");
    std::fs::write(
        &customers,
        "table Customers\n\tcolumn Name\n\t\tdataType: string\n\t\tsourceColumn: Name\n",
    )
    .unwrap();
    c.notify(
        "textDocument/didSave",
        json!({"textDocument": {"uri": file_uri(&customers)}}),
    );
    let diags = c.diagnostics(uri, Some(1));
    assert!(
        diags.iter().all(|d| d["code"] != "unknownTable"),
        "{diags:?}"
    );
    let (status, stderr) = c.shut_down();
    assert!(status.success(), "{status}: {stderr}");
}

#[test]
fn lsp_uses_the_acme_model_from_the_command_line() {
    let pbip = reference_root().join("examples/acme-outfitters/AcmeOutfitters.pbip");
    if !pbip.exists() {
        println!("SKIP: {} missing", pbip.display());
        return;
    }
    let dir = tmp_dir();
    let mut c = Client::start(&["--model", pbip.to_str().unwrap()], dir.path());
    c.initialize(json!({"processId": null, "rootUri": null, "capabilities": {}}));
    let uri = "untitled:acme.dax";
    c.open(uri, "dax", "[Total Salse] + 1");
    let diags = c.diagnostics(uri, Some(1));
    let unknown = diags
        .iter()
        .find(|d| d["code"] == "unknownMeasure")
        .unwrap_or_else(|| panic!("{diags:?}"));
    assert_eq!(unknown["data"]["replacement"], "[Total Sales]", "{unknown}");
    let typing = "untitled:typing.dax";
    c.open(typing, "dax", "[Total Sales] + [Tot");
    let done = c.request("textDocument/completion", doc_at(typing, 0, 20));
    assert_eq!(
        item(&done, "[Total Sales]")["textEdit"]["range"],
        range(0, 16, 0, 20)
    );
    let (status, stderr) = c.shut_down();
    assert!(status.success(), "{status}: {stderr}");
}

#[test]
fn lsp_follows_the_protocol_lifecycle() {
    let dir = tmp_dir();
    let mut c = Client::start(&[], dir.path());
    let early = c.call("textDocument/hover", doc_at("untitled:x.dax", 0, 0));
    assert_eq!(early["error"]["code"], -32002, "{early}");

    // Not JSON: an error with a null id, and the server goes on.
    let body = b"{not json";
    c.write_raw(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
    c.write_raw(body);
    let bad = c.recv();
    assert_eq!(bad["id"], Value::Null, "{bad}");
    assert_eq!(bad["error"]["code"], -32700);

    c.initialize(json!({"processId": null, "rootUri": null, "capabilities": {}}));
    let again = c.call("initialize", json!({"capabilities": {}}));
    assert_eq!(again["error"]["code"], -32600, "{again}");
    let unknown = c.call("workspace/symbol", json!({"query": "x"}));
    assert_eq!(unknown["error"]["code"], -32601, "{unknown}");
    // A request cancelled before it is read.
    c.notify("$/cancelRequest", json!({"id": 99}));
    c.send(
        &json!({"jsonrpc": "2.0", "id": 99, "method": "textDocument/hover",
        "params": doc_at("untitled:x.dax", 0, 0)}),
    );
    let cancelled = c.response(&json!(99));
    assert_eq!(cancelled["error"]["code"], -32800, "{cancelled}");
    // A document that is not open.
    assert_eq!(
        c.request("textDocument/hover", doc_at("untitled:none.dax", 0, 0)),
        Value::Null
    );
    // Responses and unknown notifications are ignored.
    c.send(&json!({"jsonrpc": "2.0", "id": 7, "result": null}));
    c.notify("workspace/didChangeConfiguration", json!({"settings": {}}));

    assert_eq!(c.request("shutdown", Value::Null), Value::Null);
    let late = c.call("textDocument/hover", doc_at("untitled:x.dax", 0, 0));
    assert_eq!(late["error"]["code"], -32600, "{late}");
    c.notify("exit", Value::Null);
    let (status, stderr) = c.finish();
    assert_eq!(status.code(), Some(0), "{stderr}");
}

#[test]
fn lsp_exits_with_an_error_without_shutdown() {
    let dir = tmp_dir();
    let mut c = Client::start(&[], dir.path());
    c.initialize(json!({"processId": null, "rootUri": null, "capabilities": {}}));
    c.notify("exit", Value::Null);
    let (status, stderr) = c.finish();
    assert_eq!(status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("without `shutdown`"), "{stderr}");

    // The end of the input without `shutdown` too.
    let mut c = Client::start(&[], dir.path());
    c.initialize(json!({"processId": null, "rootUri": null, "capabilities": {}}));
    let (status, _) = c.finish();
    assert_eq!(status.code(), Some(1));

    let o = Command::new(env!("CARGO_BIN_EXE_dashcraft"))
        .args(["lsp", "--model", "no-such-model.pbip"])
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(o.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("--model"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}
