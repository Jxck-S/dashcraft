//! Subcommands: one module per command, registered in [`Command`].
//!
//! To add a command `foo`:
//! 1. create `cmd/foo.rs` with `pub struct FooArgs` (`#[derive(Debug,
//!    clap::Args)]`, doc comments become `--help`) and
//!    `pub fn run(args: &FooArgs, ctx: &Context) -> anyhow::Result<Status>`;
//! 2. add `pub mod foo;` below and a `Foo(foo::FooArgs)` variant (its doc
//!    comment is the one-line help) with a match arm in [`Command::run`].
//!
//! Return `Err` for fatal errors (exit 1, printed as `dashcraft: error: …`),
//! `Status::ErrorDiagnostics` when the input has errors (exit 2), else
//! `Status::Ok`. Write results to stdout with
//! [`common::with_stdout`](crate::common::with_stdout) /
//! [`common::print_json`](crate::common::print_json), and notes/warnings
//! to stderr through [`Context`] (honours `--quiet`).

pub mod ai;
pub mod commands;
pub mod convert;
pub mod detect;
pub mod export;
pub mod inspect;
pub mod lint;
pub mod mcp;
pub mod measures;
pub mod open;
pub mod refresh;
pub mod render;
pub mod run;
pub mod serve;

use clap::Subcommand;

use crate::common::{Context, Status};

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print the model, reports and diagnostics of a file or folder
    /// (human-readable tree, or a stable JSON schema with --json).
    Inspect(inspect::InspectArgs),
    /// Detect the format of a file or folder, without reading it fully.
    Detect(detect::DetectArgs),
    /// Open a file or folder in the DashCraft desktop app.
    Open(open::OpenArgs),
    /// Convert to a PBIP project (TMDL + PBIR) or a PBIT template.
    Convert(convert::ConvertArgs),
    /// Check for broken references: visual fields, queryRefs, filters,
    /// DAX/M, relationships, interactions and layout (exit 2 on errors).
    Lint(lint::LintArgs),
    /// Load the data offline (M partitions with --blob-map, embedded PBIX
    /// data, calculated columns and tables) and print rows, time and status
    /// per table (exit 2 if a table fails).
    Refresh(refresh::RefreshArgs),
    /// Refresh, then print every measure's formatted value (optionally under
    /// a report's filters); failed measures show ⚠ and their DAX.
    Measures(measures::MeasuresArgs),
    /// Export a table's data (CSV, Parquet) or the model documentation
    /// (JSON model doc, Markdown data dictionary).
    Export(export::ExportArgs),
    /// Refresh, then render report pages to PNG or SVG headlessly
    /// (exit 2 if the refresh or a visual fails).
    Render(render::RenderArgs),
    /// List the command catalog that `run` and `serve` execute (--json,
    /// --schemas, --markdown).
    Commands(commands::CommandsArgs),
    /// Run one command of the catalog (or a --batch of them) and print its
    /// JSON result.
    Run(run::RunArgs),
    /// Serve the command catalog as JSON-lines RPC on stdin/stdout, with a
    /// persistent workspace.
    Serve(serve::ServeArgs),
    /// Serve the agent-safe commands as Model Context Protocol (MCP) tools
    /// on stdin/stdout, confined to read/write roots.
    Mcp(mcp::McpArgs),
    /// Store or delete an AI provider's API key in the OS keychain
    /// (`ai login`, `ai logout`); the `ai.*` commands run through `run`.
    Ai(ai::AiArgs),
}

impl Command {
    pub fn run(&self, ctx: &Context) -> anyhow::Result<Status> {
        match self {
            Command::Commands(args) => commands::run(args, ctx),
            Command::Run(_) => anyhow::bail!("internal error: `run` is dispatched by main"),
            Command::Serve(args) => serve::run(args, ctx),
            Command::Mcp(args) => mcp::run(args, ctx),
            Command::Ai(args) => ai::run(args, ctx),
            Command::Inspect(args) => inspect::run(args, ctx),
            Command::Detect(args) => detect::run(args, ctx),
            Command::Open(args) => open::run(args, ctx),
            Command::Convert(args) => convert::run(args, ctx),
            Command::Lint(args) => lint::run(args, ctx),
            Command::Refresh(args) => refresh::run(args, ctx),
            Command::Measures(args) => measures::run(args, ctx),
            Command::Export(args) => export::run(args, ctx),
            Command::Render(args) => render::run(args, ctx),
        }
    }
}
