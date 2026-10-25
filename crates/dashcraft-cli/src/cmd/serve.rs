//! `dashcraft serve`: a persistent workspace answering JSON-lines RPC
//! (JSON-RPC 2.0, one message per line) on stdin/stdout. The protocol is
//! documented in `pbi_ops::rpc` and docs/COMMANDS.md.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, anyhow};
use pbi_ops::rpc::{RpcOptions, serve};
use pbi_ops::{CallerKind, PathPolicy, Workspace};
use tokio::io::BufReader;

use crate::common::{Context, Status};
use crate::data::runtime;

#[derive(Debug, clap::Args)]
pub struct ServeArgs {
    /// Serve on stdin/stdout (the default, and the only transport for now).
    #[arg(long)]
    pub stdio: bool,
    /// Only read files inside DIR (repeatable). With any --read-root or
    /// --write-root, paths are confined to the roots (symlink and `..`
    /// escapes refused); relative paths resolve against the first root.
    #[arg(long = "read-root", value_name = "DIR")]
    pub read_roots: Vec<PathBuf>,
    /// Only write files inside DIR (repeatable; also readable).
    #[arg(long = "write-root", value_name = "DIR")]
    pub write_roots: Vec<PathBuf>,
    /// Default blob map of `doc.open` (blob URLs → local files).
    #[arg(long, value_name = "FILE")]
    pub blob_map: Option<PathBuf>,
    /// Requests running at the same time (default: CPU count, 2 to 8).
    #[arg(long, value_name = "N")]
    pub max_concurrent: Option<usize>,
    /// Treat requests as coming from an AI agent: only agent-safe commands
    /// are listed and run.
    #[arg(long)]
    pub agent: bool,
}

pub fn run(args: &ServeArgs, ctx: &Context) -> anyhow::Result<Status> {
    let policy = if args.read_roots.is_empty() && args.write_roots.is_empty() {
        PathPolicy::unrestricted()
    } else {
        PathPolicy::restricted(&args.read_roots, &args.write_roots)
            .map_err(|e| anyhow!("--read-root/--write-root: {e}"))?
    };
    let ws = Workspace::with_policy(policy);
    if let Some(map) = &args.blob_map {
        ws.set_default_blob_map(Some(map))
            .map_err(|e| anyhow!("--blob-map: {e}"))?;
    }
    let mut registry = pbi_ai::catalog();
    let mut opts = RpcOptions::default();
    if args.agent {
        registry = registry.agent_catalog();
        opts.caller = CallerKind::Agent;
    }
    if let Some(n) = args.max_concurrent {
        opts.max_concurrent = n.max(1);
    }
    ctx.note(format!(
        "serving {} commands as JSON-lines RPC on stdin/stdout (end of input stops)",
        registry.len()
    ));
    let rt = runtime()?;
    rt.block_on(serve(
        registry,
        Arc::new(ws),
        BufReader::new(tokio::io::stdin()),
        tokio::io::stdout(),
        opts,
    ))
    .context("the RPC server failed")?;
    Ok(Status::Ok)
}
