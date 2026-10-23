//! `dashcraft mcp`: a Model Context Protocol server on stdin/stdout exposing
//! the agent-safe commands as tools, confined to read/write roots. The
//! adapter lives in `pbi_ops::mcp`; client setup is in docs/MCP.md.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, anyhow};
use pbi_ops::mcp::{McpOptions, McpServer, ToolFilter, ToolNames};
use pbi_ops::{PathPolicy, Workspace};
use tokio::io::BufReader;

use crate::common::{Context, Status};
use crate::data::runtime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ToolNamesArg {
    /// `doc_open`: dots become underscores (accepted by every client).
    Underscore,
    /// `doc.open`: the command ids as they are.
    Dotted,
}

#[derive(Debug, clap::Args)]
pub struct McpArgs {
    /// Agents may read files inside DIR (repeatable; default: the current
    /// folder). Relative paths resolve against the first root; symlink and
    /// `..` escapes are refused.
    #[arg(long = "read-root", value_name = "DIR")]
    pub read_roots: Vec<PathBuf>,
    /// Agents may write files inside DIR (repeatable; also readable).
    /// Without it every file write is refused.
    #[arg(long = "write-root", value_name = "DIR")]
    pub write_roots: Vec<PathBuf>,
    /// Default blob map of `doc_open` (blob URLs → local files inside the
    /// read roots).
    #[arg(long, value_name = "FILE")]
    pub blob_map: Option<PathBuf>,
    /// Only expose these tools: a category (`inspect`, `render`, …), a
    /// command id (`doc.open`) or a tool name (`doc_open`). Repeatable.
    #[arg(long, value_name = "CATEGORY|ID")]
    pub allow: Vec<String>,
    /// Never expose these tools (category, command id or tool name;
    /// repeatable; applied after --allow).
    #[arg(long, value_name = "CATEGORY|ID")]
    pub deny: Vec<String>,
    /// Tool names: `underscore` (`doc_open`, the default) or `dotted`
    /// (`doc.open`).
    #[arg(long, value_enum, value_name = "STYLE", default_value = "underscore")]
    pub tool_names: ToolNamesArg,
    /// Tool calls running at the same time (default: CPU count, 2 to 8).
    #[arg(long, value_name = "N")]
    pub max_concurrent: Option<usize>,
}

pub fn run(args: &McpArgs, ctx: &Context) -> anyhow::Result<Status> {
    let read_roots = if args.read_roots.is_empty() {
        let cwd = std::env::current_dir().context("cannot read the current folder")?;
        if cwd.parent().is_none() {
            anyhow::bail!(
                "no --read-root given and the current folder is the filesystem root {}; \
                 pass --read-root DIR (MCP clients often start servers in /)",
                cwd.display()
            );
        }
        ctx.note(format!(
            "mcp: no --read-root given; agents may read files under {}",
            cwd.display()
        ));
        vec![cwd]
    } else {
        args.read_roots.clone()
    };
    let policy = PathPolicy::restricted(&read_roots, &args.write_roots)
        .map_err(|e| anyhow!("--read-root/--write-root: {e}"))?;
    let ws = Workspace::with_policy(policy);
    if let Some(map) = &args.blob_map {
        ws.set_default_blob_map(Some(map))
            .map_err(|e| anyhow!("--blob-map: {e}"))?;
    }
    let mut opts = McpOptions {
        tool_names: match args.tool_names {
            ToolNamesArg::Underscore => ToolNames::Underscore,
            ToolNamesArg::Dotted => ToolNames::Dotted,
        },
        filter: ToolFilter {
            allow: args.allow.clone(),
            deny: args.deny.clone(),
        },
        ..McpOptions::default()
    };
    if let Some(n) = args.max_concurrent {
        opts.max_concurrent = n.max(1);
    }
    let server =
        McpServer::new(&pbi_ai::catalog(), Arc::new(ws), opts).map_err(|e| anyhow!("{e}"))?;
    tracing::info!(
        tools = server.tools().len(),
        "serving MCP on stdin/stdout (end of input stops)"
    );
    let rt = runtime()?;
    rt.block_on(server.serve(BufReader::new(tokio::io::stdin()), tokio::io::stdout()))
        .context("the MCP server failed")?;
    Ok(Status::Ok)
}
