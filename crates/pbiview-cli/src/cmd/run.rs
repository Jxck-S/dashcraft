//! `dashcraft run`: runs one registry command (or a batch of them) and prints
//! its JSON result.
//!
//! Exit codes follow the other subcommands: 0 success; 1 fatal or usage
//! error (`invalidParams`, `notFound`, `io`, …); 2 when the input was read
//! but its data or expressions have errors: a `query` error, or a result
//! with `"ok": false` (failed tables, ⚠ measures, failed visuals, lint
//! findings).

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, anyhow, bail};
use clap::ArgAction;
use pbi_ops::batch::{BatchStep, run_batch};
use pbi_ops::{CallerKind, Ctx, OpError, Registry, Workspace};
use serde_json::{Map, Value, json};

use crate::common::{Context, Status, print_json, prog};
use crate::data::runtime;

#[derive(Debug, clap::Args)]
pub struct RunArgs {
    /// Command id, e.g. `report.pages` (list them with `dashcraft commands`).
    #[arg(
        value_name = "COMMAND",
        required_unless_present = "batch",
        conflicts_with = "batch"
    )]
    pub command: Option<String>,
    /// Parameters as a JSON object.
    #[arg(long, value_name = "JSON")]
    pub params: Option<String>,
    /// One parameter: `key=value` (repeatable; overrides --params). The value
    /// is parsed as JSON when it is valid JSON (`limit=10`, `ctrl=true`,
    /// `values=["A","B"]`), else taken as text. Dotted keys nest:
    /// `sort.column=Name`.
    #[arg(short = 'p', long = "param", value_name = "KEY=VALUE")]
    pub param: Vec<String>,
    /// Open this file or folder first and pass its `docId` to the command
    /// (and to batch steps that take one; also `${doc.docId}` in batches).
    #[arg(long, value_name = "PATH")]
    pub doc: Option<PathBuf>,
    /// Blob map for --doc and the workspace default (blob URLs → local
    /// files).
    #[arg(long, value_name = "FILE")]
    pub blob_map: Option<PathBuf>,
    /// Run the steps of a JSON file (`-` for stdin): `[{ "id", "params"?,
    /// "as"? }]` or `{ "steps": [...], "stopOnError"? }`. Strings like
    /// `"${name.field}"` reference earlier results.
    #[arg(long, value_name = "FILE")]
    pub batch: Option<PathBuf>,
    /// Stop a batch at the first failing step.
    #[arg(long, value_name = "BOOL", default_value_t = true, action = ArgAction::Set)]
    pub stop_on_error: bool,
    /// Pretty-print the JSON.
    #[arg(long)]
    pub pretty: bool,
}

/// Exit status of `run`: success, data errors (2), or a command error.
#[derive(Debug)]
pub enum RunStatus {
    Done(Status),
    Failed(u8),
}

impl From<RunStatus> for ExitCode {
    fn from(s: RunStatus) -> Self {
        match s {
            RunStatus::Done(status) => status.into(),
            RunStatus::Failed(code) => ExitCode::from(code),
        }
    }
}

/// Sets `key` (dotted path) in `obj`.
fn set_path(obj: &mut Map<String, Value>, key: &str, value: Value) -> anyhow::Result<()> {
    let mut parts = key.split('.').peekable();
    let mut cur = obj;
    while let Some(part) = parts.next() {
        if part.is_empty() {
            bail!("-p {key}: empty key part");
        }
        if parts.peek().is_none() {
            cur.insert(part.to_owned(), value);
            return Ok(());
        }
        let next = cur
            .entry(part.to_owned())
            .or_insert_with(|| Value::Object(Map::new()));
        cur = match next {
            Value::Object(map) => map,
            _ => bail!("-p {key}: '{part}' is not an object"),
        };
    }
    Ok(())
}

/// `--params` merged with `-p key=value`.
fn parameters(args: &RunArgs) -> anyhow::Result<Value> {
    let mut obj = match &args.params {
        Some(text) => match serde_json::from_str::<Value>(text)
            .with_context(|| "--params is not valid JSON")?
        {
            Value::Object(map) => map,
            Value::Null => Map::new(),
            _ => bail!("--params must be a JSON object"),
        },
        None => Map::new(),
    };
    for p in &args.param {
        let (key, value) = p
            .split_once('=')
            .ok_or_else(|| anyhow!("-p {p:?}: expected KEY=VALUE"))?;
        let value = serde_json::from_str::<Value>(value)
            .unwrap_or_else(|_| Value::String(value.to_owned()));
        set_path(&mut obj, key.trim(), value)?;
    }
    Ok(Value::Object(obj))
}

/// `true` if `command` takes a `docId` parameter.
fn takes_doc_id(registry: &Registry, command: &str) -> bool {
    registry
        .describe(command)
        .and_then(|s| s.params_schema.pointer("/properties/docId"))
        .is_some()
}

fn inject_doc_id(registry: &Registry, command: &str, params: &mut Value, doc_id: &str) {
    if let Value::Object(map) = params
        && !map.contains_key("docId")
        && takes_doc_id(registry, command)
    {
        map.insert("docId".into(), Value::String(doc_id.to_owned()));
    }
}

fn report_error(e: &OpError) {
    let name = prog();
    eprintln!("{name}: error: [{}] {}", e.code, e.message);
    if let Some(data) = &e.data {
        eprintln!("{name}: details: {data}");
    }
}

fn reports_not_ok(value: &Value) -> bool {
    value.get("ok") == Some(&Value::Bool(false))
}

fn read_batch(path: &Path) -> anyhow::Result<(Vec<BatchStep>, Option<bool>)> {
    let text = if path == Path::new("-") {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .context("cannot read the batch from stdin")?;
        s
    } else {
        fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?
    };
    let value: Value = serde_json::from_str(&text)
        .with_context(|| format!("{} is not valid JSON", path.display()))?;
    let (steps, stop) = match value {
        Value::Array(_) => (value, None),
        Value::Object(mut obj) => {
            let stop = obj.remove("stopOnError").and_then(|v| v.as_bool());
            let steps = obj.remove("steps").ok_or_else(|| {
                anyhow!(
                    "{}: expected an array of steps or {{\"steps\": [...]}}",
                    path.display()
                )
            })?;
            if let Some(extra) = obj.keys().next() {
                bail!("{}: unknown field '{extra}'", path.display());
            }
            (steps, stop)
        }
        _ => bail!("{}: expected an array of steps", path.display()),
    };
    let steps: Vec<BatchStep> = serde_json::from_value(steps)
        .with_context(|| format!("{}: invalid steps", path.display()))?;
    Ok((steps, stop))
}

pub fn run(args: &RunArgs, ctx: &Context) -> anyhow::Result<RunStatus> {
    let registry = pbi_ai::catalog();
    let ws = Workspace::new();
    if let Some(map) = &args.blob_map {
        ws.set_default_blob_map(Some(map))
            .map_err(|e| anyhow!("--blob-map: {e}"))?;
    }
    let verbose = ctx.verbose > 0;
    let op_ctx = Ctx::new(CallerKind::Cli).with_progress(move |e| {
        if verbose {
            tracing::info!("{}: {}", e.command, e.message);
        }
    });
    let rt = runtime()?;
    rt.block_on(async {
        let mut doc_summary = None;
        if let Some(path) = &args.doc {
            let mut open = json!({ "path": path });
            if let Some(map) = &args.blob_map {
                open["blobMap"] = json!(map);
            }
            match registry.execute(&ws, "doc.open", open, &op_ctx).await {
                Ok(summary) => doc_summary = Some(summary),
                Err(e) => {
                    report_error(&e);
                    return Ok(RunStatus::Failed(e.exit_code()));
                }
            }
        }
        let doc_id = doc_summary
            .as_ref()
            .and_then(|s| s.get("docId"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(batch) = &args.batch {
            if args.params.is_some() || !args.param.is_empty() {
                bail!("--params / -p do not apply to --batch (put params in the steps)");
            }
            let (mut steps, stop) = read_batch(batch)?;
            if let Some(id) = &doc_id {
                for step in &mut steps {
                    if step.params.is_null() {
                        step.params = Value::Object(Map::new());
                    }
                    inject_doc_id(&registry, &step.id, &mut step.params, id);
                }
            }
            let mut vars = Map::new();
            if let Some(summary) = doc_summary {
                vars.insert("doc".into(), summary);
            }
            let stop_on_error = stop.unwrap_or(args.stop_on_error);
            let results = run_batch(&registry, &ws, &steps, stop_on_error, vars, &op_ctx).await;
            print_json(&results, args.pretty)?;
            if let Some(e) = results.iter().find_map(|r| r.error.as_ref()) {
                report_error(e);
                return Ok(RunStatus::Failed(e.exit_code()));
            }
            let not_ok = results
                .iter()
                .any(|r| r.result.as_ref().is_some_and(reports_not_ok));
            return Ok(RunStatus::Done(if not_ok {
                Status::ErrorDiagnostics
            } else {
                Status::Ok
            }));
        }
        let Some(command) = &args.command else {
            bail!(
                "name a command ({} commands lists them) or pass --batch FILE",
                prog()
            );
        };
        let mut params = parameters(args)?;
        if let Some(id) = &doc_id {
            inject_doc_id(&registry, command, &mut params, id);
        }
        match registry.execute(&ws, command, params, &op_ctx).await {
            Ok(value) => {
                print_json(&value, args.pretty)?;
                Ok(RunStatus::Done(if reports_not_ok(&value) {
                    Status::ErrorDiagnostics
                } else {
                    Status::Ok
                }))
            }
            Err(e) => {
                report_error(&e);
                Ok(RunStatus::Failed(e.exit_code()))
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(params: Option<&str>, p: &[&str]) -> RunArgs {
        RunArgs {
            command: Some("x".into()),
            params: params.map(str::to_owned),
            param: p.iter().map(|s| (*s).to_owned()).collect(),
            doc: None,
            blob_map: None,
            batch: None,
            stop_on_error: true,
            pretty: false,
        }
    }

    #[test]
    fn params_merge_json_and_key_values() {
        let v = parameters(&args(
            Some(r#"{"docId": "d1", "limit": 5}"#),
            &[
                "limit=10",
                "sort.column=App Name",
                "values=[\"A\",null]",
                "ok=true",
            ],
        ))
        .unwrap();
        assert_eq!(
            v,
            json!({"docId": "d1", "limit": 10, "sort": {"column": "App Name"}, "values": ["A", null], "ok": true})
        );
        assert!(parameters(&args(Some("[1]"), &[])).is_err());
        assert!(parameters(&args(None, &["novalue"])).is_err());
        assert!(parameters(&args(None, &["a=1", "a.b=2"])).is_err());
    }
}
