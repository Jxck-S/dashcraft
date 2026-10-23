//! A Model Context Protocol (MCP) server over the command registry
//! (`dashcraft mcp`): a thin adapter, with no per-tool code.
//!
//! - **Tools** are the agent-safe commands
//!   ([`Registry::agent_catalog`](crate::Registry::agent_catalog)), minus
//!   `--deny`, limited to `--allow` ([`ToolFilter`]). Names derive from the
//!   ids ([`ToolNames`]: `doc.open` → `doc_open` by default, collision
//!   checked); `title`/`description` come from [`CommandMeta`](crate::CommandMeta),
//!   `inputSchema`/`outputSchema` from [`CommandSpec`](crate::CommandSpec)
//!   and the `annotations` from its flags ([`Hints`]; `openWorldHint` is
//!   always `false`: no network).
//! - **`tools/call`** runs [`Registry::execute`](crate::Registry::execute)
//!   with a [`CallerKind::Agent`](crate::CallerKind::Agent) caller on one
//!   [`Workspace`](crate::Workspace) for the whole session; the workspace
//!   must have a restricted [`PathPolicy`](crate::PathPolicy). Results are
//!   the output as JSON text plus `structuredContent`; inline PNG renders
//!   (`pngBase64`) become `image` content; text over
//!   [`McpOptions::max_text_bytes`] is cut with a note. Command failures
//!   are tool results with `isError: true` and the error (`{ code,
//!   message, data? }`) as text; unknown tools and malformed requests are
//!   JSON-RPC `-32602`.
//! - **Transport**: newline-delimited JSON-RPC 2.0 on any byte stream
//!   (stdio), sharing framing and per-document ordering with
//!   [`rpc`](crate::rpc). Nothing but MCP messages is written to the
//!   output; logs go to `tracing` (stderr in the CLI).
//! - **Protocol**: `initialize` (the client's revision when served, else
//!   [`LATEST_LEGACY_VERSION`](protocol::LATEST_LEGACY_VERSION)),
//!   `notifications/initialized`, `ping`, `tools/list` (cursor pagination),
//!   `tools/call`, `notifications/cancelled` (cancels the call's
//!   `CancellationToken`; no response follows), `notifications/progress`
//!   for calls with `_meta.progressToken`, `logging/setLevel` and
//!   `notifications/message`. Stateless 2026-07-28 clients are served too:
//!   `server/discover`, the per-request `_meta` protocol version
//!   (`-32022` for unknown versions), `resultType`, `ttlMs`/`cacheScope`.
//!
//! See `docs/MCP.md` for client configuration and a transcript.

pub mod protocol;
mod server;
mod tools;

pub use server::{DEFAULT_MAX_TEXT_BYTES, McpOptions, McpServer};
pub use tools::{Hints, META_PREFIX, ToolCatalog, ToolFilter, ToolNames};
