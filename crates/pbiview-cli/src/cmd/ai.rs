//! `dashcraft ai login|logout`: store or delete an AI provider's API key in
//! the OS keychain. The key is read without echo from the terminal (or from
//! stdin with `--stdin`), never from arguments, so it stays out of shell
//! history and process listings. Everything else about AI goes through the
//! `ai.*` registry commands (`dashcraft run ai.status`, …); see docs/AI.md.

use std::io::{BufRead as _, IsTerminal as _};

use anyhow::{Context as _, anyhow, bail};

use crate::common::{Context, Status};

#[derive(Debug, clap::Args)]
pub struct AiArgs {
    #[command(subcommand)]
    pub action: AiAction,
}

#[derive(Debug, clap::Subcommand)]
pub enum AiAction {
    /// Store a provider's API key in the OS keychain (prompted without echo).
    Login(LoginArgs),
    /// Delete a provider's API key from the OS keychain.
    Logout(LogoutArgs),
}

#[derive(Debug, clap::Args)]
pub struct LoginArgs {
    /// Provider id from the AI settings (`ai.status`), e.g. `openai`.
    #[arg(long, value_name = "ID")]
    pub provider: String,
    /// Read the key from the first line of stdin instead of prompting
    /// (scripts; the key is never accepted as an argument).
    #[arg(long)]
    pub stdin: bool,
}

#[derive(Debug, clap::Args)]
pub struct LogoutArgs {
    /// Provider id whose key to delete.
    #[arg(long, value_name = "ID")]
    pub provider: String,
}

pub fn run(args: &AiArgs, ctx: &Context) -> anyhow::Result<Status> {
    match &args.action {
        AiAction::Login(a) => login(a, ctx),
        AiAction::Logout(a) => logout(a, ctx),
    }
}

fn login(args: &LoginArgs, ctx: &Context) -> anyhow::Result<Status> {
    let key = if args.stdin {
        let mut line = String::new();
        std::io::stdin()
            .lock()
            .read_line(&mut line)
            .context("reading the key from stdin")?;
        line
    } else {
        if !std::io::stdin().is_terminal() {
            bail!("no terminal to prompt on; pipe the key with --stdin");
        }
        rpassword::prompt_password(format!("API key for `{}`: ", args.provider))
            .context("reading the key")?
    };
    let key = key.trim();
    if key.is_empty() {
        bail!("no key entered");
    }
    pbi_ai::store_api_key(&args.provider, key).map_err(|e| anyhow!("{e}"))?;
    if !ctx.quiet {
        eprintln!(
            "Stored the API key for `{}` in the OS keychain.",
            args.provider
        );
    }
    Ok(Status::Ok)
}

fn logout(args: &LogoutArgs, ctx: &Context) -> anyhow::Result<Status> {
    let removed = pbi_ai::clear_api_key(&args.provider).map_err(|e| anyhow!("{e}"))?;
    if !ctx.quiet {
        if removed {
            eprintln!("Deleted the API key for `{}`.", args.provider);
        } else {
            eprintln!("No stored API key for `{}`.", args.provider);
        }
    }
    Ok(Status::Ok)
}
