//! Command registry: every DashCraft capability as a named, typed,
//! schema-described command, shared by the CLI (`dashcraft run`, `dashcraft
//! serve`), the desktop app and AI agents (MCP). See ADR-0007
//! (`docs/adr/0007-command-registry.md`) and the generated catalog in
//! `docs/COMMANDS.md`.
//!
//! # Pieces
//!
//! - [`Command`]: a typed command (unit struct) with `Params`
//!   (`Deserialize + JsonSchema`, `deny_unknown_fields`, camelCase),
//!   `Output` (`Serialize + JsonSchema`), static [`CommandMeta`] and an
//!   async `run(&Workspace, Params, &Ctx)`.
//! - [`Registry`]: the type-erased catalog ([`CommandSpec`]: metadata, JSON
//!   Schemas, runner). [`Registry::execute`] is the one entry point: it
//!   validates the parameters, applies the caller policy, orders mutating
//!   commands per document, honours cancellation and turns panics into
//!   [`OpErrorCode::Internal`].
//! - [`Workspace`]: open documents ([`DocEntry`]: a `pbi_session::Session`,
//!   its canvas state and data generation) keyed by `docId`, plus the
//!   [`PathPolicy`] every path goes through.
//! - [`Ctx`]: cancellation, progress sink, [`CallerKind`].
//! - [`OpError`]: stable string codes with JSON-RPC and exit-code
//!   mappings.
//! - [`batch`]: ordered steps with `${name.field}` interpolation; [`rpc`]:
//!   the JSON-lines RPC server; [`markdown`]: the catalog as Markdown.
//! - [`mcp`]: the Model Context Protocol server (`dashcraft mcp`), a thin
//!   adapter exposing the agent catalog as MCP tools.
//!
//! ```no_run
//! # async fn demo() -> Result<(), pbi_ops::OpError> {
//! use pbi_ops::{CallerKind, Ctx, Registry, Workspace};
//! use serde_json::json;
//!
//! let registry = Registry::default_catalog();
//! let ws = Workspace::new();
//! let ctx = Ctx::new(CallerKind::Cli);
//! let doc = registry
//!     .execute(&ws, "doc.open", json!({"path": "model.pbip"}), &ctx)
//!     .await?;
//! let pages = registry
//!     .execute(&ws, "report.pages", json!({"docId": doc["docId"]}), &ctx)
//!     .await?;
//! println!("{pages}");
//! # Ok(()) }
//! ```

pub mod batch;
pub mod canvas;
mod command;
pub mod commands;
mod ctx;
pub mod detect_model;
pub mod dto;
mod error;
pub mod files;
pub mod inspect_model;
pub mod markdown;
pub mod mcp;
mod path_policy;
mod registry;
pub mod resolve;
pub mod rpc;
mod workspace;

pub use command::{
    BoxFuture, CallFn, Category, Command, CommandMeta, CommandSpec, output_schema, params_schema,
    parse_params,
};
pub use ctx::{CallerKind, Ctx, ProgressEvent, ProgressSink};
pub use error::{OpError, OpErrorCode};
pub use path_policy::PathPolicy;
pub use registry::{CommandInfo, ExampleInfo, Registry};
pub use workspace::{DEFAULT_MAX_DOCUMENTS, DocEntry, DocOptions, Workspace, WorkspaceSettings};
