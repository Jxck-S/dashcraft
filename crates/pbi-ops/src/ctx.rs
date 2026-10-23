//! [`Ctx`]: what a command run gets besides its parameters — cancellation,
//! a progress sink and who is calling.

use std::fmt;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::error::OpError;
use crate::registry::Registry;

/// Who runs a command; used for policy (agents only see
/// [`agent_safe`](crate::CommandMeta::agent_safe) commands).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum CallerKind {
    /// `dashcraft run` (a person or a script on the command line).
    #[default]
    Cli,
    /// The JSON-lines RPC server (`dashcraft serve`).
    Rpc,
    /// The desktop app.
    App,
    /// An AI agent (MCP server, automation with restricted roots).
    Agent,
}

/// A progress notification of a running command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    /// The command id (`data.refresh`).
    pub command: String,
    /// What is happening, for people (`Orders: 3900 rows`).
    pub message: String,
    /// Completion between 0 and 1, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fraction: Option<f64>,
    /// Structured details (e.g. the refresh event).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Receives [`ProgressEvent`]s; called from the command's task, keep it
/// cheap and non-blocking.
pub type ProgressSink = Arc<dyn Fn(ProgressEvent) + Send + Sync>;

/// The context of one command run. Cheap to clone.
#[derive(Clone, Default)]
pub struct Ctx {
    cancel: CancellationToken,
    progress: Option<ProgressSink>,
    caller: CallerKind,
    command: Option<&'static str>,
    registry: Option<Registry>,
}

impl fmt::Debug for Ctx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ctx")
            .field("caller", &self.caller)
            .field("command", &self.command)
            .field("cancelled", &self.cancel.is_cancelled())
            .field("progress", &self.progress.is_some())
            .finish_non_exhaustive()
    }
}

impl Ctx {
    /// A context for `caller`, with a fresh cancellation token and no
    /// progress sink.
    #[must_use]
    pub fn new(caller: CallerKind) -> Self {
        Self {
            caller,
            ..Self::default()
        }
    }

    /// Uses `token` for cancellation.
    #[must_use]
    pub fn with_cancel(mut self, token: CancellationToken) -> Self {
        self.cancel = token;
        self
    }

    /// Sends progress to `sink`.
    #[must_use]
    pub fn with_progress(mut self, sink: impl Fn(ProgressEvent) + Send + Sync + 'static) -> Self {
        self.progress = Some(Arc::new(sink));
        self
    }

    /// Sends progress to `sink` (shared).
    #[must_use]
    pub fn with_progress_sink(mut self, sink: Option<ProgressSink>) -> Self {
        self.progress = sink;
        self
    }

    /// The context of one step of `registry`'s command `id` (set by
    /// [`Registry::execute`]).
    #[must_use]
    pub(crate) fn for_command(&self, id: &'static str, registry: &Registry) -> Self {
        Self {
            command: Some(id),
            registry: Some(registry.clone()),
            ..self.clone()
        }
    }

    /// The cancellation token of this run.
    #[must_use]
    pub fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }

    /// `true` once the run is cancelled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// `Err(Cancelled)` once the run is cancelled.
    ///
    /// # Errors
    /// [`OpErrorCode::Cancelled`](crate::OpErrorCode::Cancelled).
    pub fn check_cancelled(&self) -> Result<(), OpError> {
        if self.is_cancelled() {
            Err(OpError::cancelled())
        } else {
            Ok(())
        }
    }

    /// Who is calling.
    #[must_use]
    pub fn caller(&self) -> CallerKind {
        self.caller
    }

    /// The id of the running command (inside a run).
    #[must_use]
    pub fn command(&self) -> Option<&'static str> {
        self.command
    }

    /// The registry running the command (inside a run): commands that list
    /// or run other commands (`commands.list`, `commands.batch`) use it.
    #[must_use]
    pub fn registry(&self) -> Option<&Registry> {
        self.registry.as_ref()
    }

    /// The progress sink, if any.
    #[must_use]
    pub fn progress_sink(&self) -> Option<&ProgressSink> {
        self.progress.as_ref()
    }

    /// Reports progress (no-op without a sink).
    pub fn progress(&self, message: impl Into<String>, fraction: Option<f64>) {
        self.progress_with(message, fraction, None);
    }

    /// Reports progress with structured details.
    pub fn progress_with(
        &self,
        message: impl Into<String>,
        fraction: Option<f64>,
        data: Option<serde_json::Value>,
    ) {
        if let Some(sink) = &self.progress {
            sink(ProgressEvent {
                command: self.command.unwrap_or_default().to_owned(),
                message: message.into(),
                fraction: fraction
                    .filter(|f| f.is_finite())
                    .map(|f| f.clamp(0.0, 1.0)),
                data,
            });
        }
    }
}
