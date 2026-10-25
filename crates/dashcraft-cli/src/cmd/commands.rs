//! `dashcraft commands`: the command catalog of `pbi-ops` (the commands
//! `dashcraft run` and `dashcraft serve` execute).

use anyhow::anyhow;
use pbi_ops::markdown::{catalog_markdown, command_markdown};
use pbi_ops::{Category, CommandInfo, Registry};

use crate::common::{Context, Status, print_json, prog, with_stdout};

#[derive(Debug, clap::Args)]
pub struct CommandsArgs {
    /// Describe one command (its parameters, result and examples).
    #[arg(value_name = "COMMAND")]
    pub id: Option<String>,
    /// Print JSON (`commands.list` / `commands.describe` output).
    #[arg(long)]
    pub json: bool,
    /// Include each command's parameter and result JSON Schemas (implies
    /// --json).
    #[arg(long)]
    pub schemas: bool,
    /// Print the catalog as Markdown (the content of docs/COMMANDS.md).
    #[arg(long, conflicts_with_all = ["json", "schemas", "id"])]
    pub markdown: bool,
    /// Only the commands offered to AI agents.
    #[arg(long)]
    pub agent_safe: bool,
    /// Pretty-print the JSON (implies --json).
    #[arg(long)]
    pub pretty: bool,
}

pub fn run(args: &CommandsArgs, _ctx: &Context) -> anyhow::Result<Status> {
    // `--markdown` documents the core catalog (docs/COMMANDS.md); the opt-in
    // `ai.*` commands are documented in docs/AI.md.
    let mut registry = Registry::default_catalog();
    if !args.markdown {
        pbi_ai::register(&mut registry);
    }
    if args.agent_safe {
        registry = registry.agent_catalog();
    }
    if args.markdown {
        let text = catalog_markdown(&registry);
        with_stdout(|w| w.write_all(text.as_bytes()))?;
        return Ok(Status::Ok);
    }
    let json = args.json || args.schemas || args.pretty;
    if let Some(id) = &args.id {
        let spec = registry
            .describe(id)
            .ok_or_else(|| anyhow!("{}", registry.unknown(id)))?;
        if json {
            print_json(&CommandInfo::of(spec, true), args.pretty)?;
        } else {
            let text = command_markdown(spec);
            with_stdout(|w| w.write_all(text.trim_start().as_bytes()))?;
        }
        return Ok(Status::Ok);
    }
    if json {
        let commands: Vec<CommandInfo> = registry
            .list()
            .iter()
            .map(|s| CommandInfo::of(s, args.schemas))
            .collect();
        print_json(
            &serde_json::json!({ "count": commands.len(), "commands": commands }),
            args.pretty,
        )?;
        return Ok(Status::Ok);
    }
    let width = registry
        .list()
        .iter()
        .map(|s| s.meta.id.len())
        .max()
        .unwrap_or(0);
    with_stdout(|w| {
        for category in Category::ALL {
            let specs: Vec<_> = registry
                .list()
                .iter()
                .filter(|s| s.meta.category == category)
                .collect();
            if specs.is_empty() {
                continue;
            }
            writeln!(w, "{category}")?;
            for s in specs {
                let m = &s.meta;
                let mut flags = Vec::new();
                if m.mutates_document {
                    flags.push("mutates");
                }
                if m.long_running {
                    flags.push("long");
                }
                if m.writes_files {
                    flags.push("writes");
                }
                if !m.agent_safe {
                    flags.push("not-agent");
                }
                let flags = if flags.is_empty() {
                    String::new()
                } else {
                    format!("  [{}]", flags.join(", "))
                };
                writeln!(w, "  {:<width$}  {}{flags}", m.id, m.summary)?;
            }
        }
        writeln!(
            w,
            "\n{} commands. `{name} commands <COMMAND>` describes one; `{name} run <COMMAND> -p key=value` runs it.",
            registry.len(),
            name = prog()
        )
    })?;
    Ok(Status::Ok)
}
