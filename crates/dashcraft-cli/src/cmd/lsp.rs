//! `dashcraft lsp`: a Language Server Protocol server for `.dax`, `.m` and `.tmdl` files on
//! stdin/stdout, over the registry's language commands. The server lives in `crate::lsp`;
//! editor setup is in docs/CLI.md.

use std::path::PathBuf;

use anyhow::bail;

use crate::common::{Context, Status};
use crate::lsp::{self, Ending, Options};

#[derive(Debug, clap::Args)]
pub struct LspArgs {
    /// Talk LSP on stdin/stdout (the only transport; accepted because editors pass it).
    #[arg(long)]
    pub stdio: bool,
    /// Check DAX and M names against this model (a .pbip, .pbix, .pbit, .bim, a TMDL or
    /// PBIP folder, …). Without it only syntax and functions are checked. Saving a file
    /// under the model's folder opens it again.
    #[arg(long, value_name = "PATH")]
    pub model: Option<PathBuf>,
    /// The editor's process id (some clients pass it; not used).
    #[arg(long = "clientProcessId", value_name = "PID", hide = true)]
    pub client_process_id: Option<String>,
}

pub fn run(args: &LspArgs, _ctx: &Context) -> anyhow::Result<Status> {
    let opts = Options {
        model: args.model.clone(),
    };
    tracing::info!("serving LSP on stdin/stdout (`exit` or the end of input stops)");
    let stdout = std::io::stdout();
    match lsp::serve(std::io::stdin(), stdout.lock(), &opts)? {
        Ending::Clean => Ok(Status::Ok),
        Ending::WithoutShutdown => bail!("the client ended the session without `shutdown`"),
    }
}
