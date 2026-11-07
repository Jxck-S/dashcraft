//! The command registry behind `detect`, `inspect` and `lint`: these
//! subcommands run `doc.detect`, `doc.open`, `doc.inspect` and `doc.lint`
//! (ADR-0007) and only print the result, as the command's JSON or as text.

use std::path::Path;

use anyhow::{Context as _, anyhow};
use pbi_ops::commands::inspect::DocInspect;
use pbi_ops::commands::lint::DocLint;
use pbi_ops::commands::workspace::{DocDetect, DocOpen, DocSummary};
use pbi_ops::{CallerKind, Ctx, OpError, Registry, Workspace};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use crate::common::ReadArgs;

/// The commands these subcommands run, an empty workspace and a runtime to
/// run them on.
pub struct Ops {
    registry: Registry,
    ws: Workspace,
    ctx: Ctx,
    rt: Runtime,
}

impl Ops {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            // Only the commands used: building the catalog's schemas costs
            // more than the smaller inputs take to read.
            registry: Registry::new()
                .with::<DocDetect>()
                .with::<DocOpen>()
                .with::<DocInspect>()
                .with::<DocLint>(),
            ws: Workspace::new(),
            ctx: Ctx::new(CallerKind::Cli),
            // One thread: the commands run one at a time, and starting a
            // worker per core costs more than the smaller inputs take.
            rt: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("cannot start the async runtime")?,
        })
    }

    /// Runs command `id`; its error becomes the fatal error message.
    pub fn execute(&self, id: &str, params: Value) -> anyhow::Result<Value> {
        self.rt
            .block_on(self.registry.execute(&self.ws, id, params, &self.ctx))
            .map_err(fatal)
    }

    /// `doc.open` of `path` with the reader flags.
    pub fn open(&self, path: &Path, read: &ReadArgs) -> anyhow::Result<DocSummary> {
        let summary = self.execute(
            "doc.open",
            json!({
                "path": path_param(path)?,
                "includeSystemTables": read.include_system_tables,
                "loadPbixSchema": !read.no_pbix_schema,
            }),
        )?;
        typed(&summary)
    }
}

/// `path` as a JSON parameter. Command parameters are JSON, so a path that
/// is not valid Unicode cannot be passed.
pub fn path_param(path: &Path) -> anyhow::Result<&str> {
    path.to_str().ok_or_else(|| {
        anyhow!(
            "cannot open {}: the path is not valid Unicode",
            path.display()
        )
    })
}

/// A command result as its output type, for the text writers.
pub fn typed<T: DeserializeOwned>(value: &Value) -> anyhow::Result<T> {
    T::deserialize(value).map_err(|e| anyhow!("unexpected command result: {e}"))
}

fn fatal(e: OpError) -> anyhow::Error {
    anyhow!(e.message)
}
