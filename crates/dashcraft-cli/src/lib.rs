//! DashCraft command-line interface: the program behind the `dashcraft`
//! binary ([`run_main`]); help, usage and messages name it [`common::prog`].
//!
//! Layout: one module per subcommand under [`cmd`] (`cmd/<name>.rs`, each
//! with a clap `Args` struct and a `run` function), registered in the
//! [`cmd::Command`] enum. Shared helpers live in [`common`] (and, for the
//! commands that load data through `pbi-session`, in [`data`]); the stable
//! `inspect --json` schema lives in [`inspect_model`].
//!
//! Exit codes: 0 success (warnings allowed), 1 fatal error (unreadable,
//! refused or unsupported input, usage errors, I/O failures), 2 the input
//! was read but has Error diagnostics. Fatal errors are printed on stderr
//! as `dashcraft: error: …`.
//!
//! No telemetry and no network access: every command works offline (the
//! opt-in `ai.*` commands excepted, docs/AI.md).

mod cmd;
mod common;
mod data;
mod inspect_model;

use std::io::IsTerminal;
use std::process::ExitCode;

use clap::{ArgAction, CommandFactory as _, FromArgMatches as _, Parser};

use crate::common::{Context, EXIT_FATAL, prog};

/// DashCraft opens Power BI files offline (PBIP, PBIX, PBIT, TMDL, TMSL,
/// PBIR): inspect, lint, refresh, export, render, and the command registry.
#[derive(Debug, Parser)]
#[command(name = "dashcraft", version, propagate_version = true)]
struct Cli {
    /// Log progress on stderr (-v info, -vv debug, -vvv trace).
    #[arg(short, long, action = ArgAction::Count, global = true)]
    verbose: u8,
    /// Only print errors on stderr.
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    quiet: bool,
    #[command(subcommand)]
    command: cmd::Command,
}

/// Runs the CLI with the process arguments: the whole program behind the
/// `dashcraft` binary.
#[must_use]
pub fn run_main() -> ExitCode {
    let name = prog();
    let parsed = Cli::command()
        .name(name)
        .bin_name(name)
        .try_get_matches()
        .and_then(|matches| Cli::from_arg_matches(&matches));
    let cli = match parsed {
        Ok(cli) => cli,
        Err(e) => {
            // `--help`/`--version` print to stdout and succeed; usage errors
            // are fatal (exit 1, keeping 2 for "the input has errors").
            let _ = e.print();
            return if e.use_stderr() {
                ExitCode::from(EXIT_FATAL)
            } else {
                ExitCode::SUCCESS
            };
        }
    };
    let ctx = Context {
        verbose: cli.verbose,
        quiet: cli.quiet,
    };
    init_tracing(&ctx);
    if let cmd::Command::Run(args) = &cli.command {
        // `run` exits with the command's own error code.
        return match cmd::run::run(args, &ctx) {
            Ok(status) => status.into(),
            Err(e) => {
                eprintln!("{name}: error: {e:#}");
                ExitCode::from(EXIT_FATAL)
            }
        };
    }
    match cli.command.run(&ctx) {
        Ok(status) => status.into(),
        Err(e) => {
            eprintln!("{name}: error: {e:#}");
            ExitCode::from(EXIT_FATAL)
        }
    }
}

/// Tracing to stderr. `RUST_LOG` is honoured unless `-v` or `--quiet` is
/// given; the default level is `warn`.
fn init_tracing(ctx: &Context) {
    use tracing_subscriber::EnvFilter;
    let filter = match (ctx.quiet, ctx.verbose) {
        (true, _) => EnvFilter::new("error"),
        (false, 0) => EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        (false, 1) => EnvFilter::new("info"),
        (false, 2) => EnvFilter::new("debug"),
        (false, _) => EnvFilter::new("trace"),
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .without_time()
        .with_target(ctx.verbose >= 2)
        .try_init();
}
