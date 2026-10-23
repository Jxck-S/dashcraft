//! [`Registry`]: the catalog of commands and the one way to run them.

use std::collections::HashMap;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::command::{CatchUnwind, Command, CommandMeta, CommandSpec};
use crate::ctx::{CallerKind, Ctx};
use crate::error::OpError;
use crate::workspace::Workspace;

#[derive(Clone, Default)]
struct Inner {
    specs: Vec<CommandSpec>,
    /// id → index of its first registration.
    index: HashMap<&'static str, usize>,
}

/// A catalog of commands. Cheap to clone (shared).
///
/// Every surface (CLI `run`/`serve`, the app, MCP) runs commands through
/// [`execute`](Self::execute), which validates the parameters, applies the
/// caller policy, orders mutating commands per document, honours
/// cancellation and turns panics into [`OpErrorCode::Internal`](crate::OpErrorCode::Internal).
#[derive(Clone, Default)]
pub struct Registry {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("commands", &self.inner.specs.len())
            .finish()
    }
}

/// One example of a command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExampleInfo {
    pub title: String,
    /// Example parameters.
    pub params: Value,
}

/// Serializable description of a command (`commands.list`,
/// `commands.describe`, `dashcraft commands --json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommandInfo {
    /// Stable dotted id (`doc.open`); the JSON-RPC method name.
    pub id: String,
    pub title: String,
    /// One line: what it does and returns.
    pub summary: String,
    pub category: crate::command::Category,
    /// Changes the in-memory state of an open document (ordered per
    /// document).
    pub mutates_document: bool,
    /// Reports progress and can take a while.
    pub long_running: bool,
    /// Offered to AI agents (MCP) by default.
    pub agent_safe: bool,
    /// Writes files (checked by the path policy).
    pub writes_files: bool,
    /// Version that added it.
    pub since: String,
    pub examples: Vec<ExampleInfo>,
    /// JSON Schema of the parameters (with `schemas`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params_schema: Option<Value>,
    /// JSON Schema of the result (with `schemas`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
}

impl CommandInfo {
    /// The description of `spec`, with its JSON Schemas when `schemas`.
    #[must_use]
    pub fn of(spec: &CommandSpec, schemas: bool) -> Self {
        let m = &spec.meta;
        Self {
            id: m.id.to_owned(),
            title: m.title.to_owned(),
            summary: m.summary.to_owned(),
            category: m.category,
            mutates_document: m.mutates_document,
            long_running: m.long_running,
            agent_safe: m.agent_safe,
            writes_files: m.writes_files,
            since: m.since.to_owned(),
            examples: m
                .examples
                .iter()
                .map(|(title, params)| ExampleInfo {
                    title: (*title).to_owned(),
                    params: serde_json::from_str(params).unwrap_or(Value::Null),
                })
                .collect(),
            params_schema: schemas.then(|| spec.params_schema.clone()),
            output_schema: schemas.then(|| spec.output_schema.clone()),
        }
    }
}

impl Registry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The v1 catalog (see `docs/COMMANDS.md`).
    #[must_use]
    pub fn default_catalog() -> Self {
        crate::commands::catalog()
    }

    /// Adds `C`. A second registration of an id is kept in
    /// [`list`](Self::list) (so tests catch it) but never dispatched.
    pub fn register<C: Command>(&mut self) -> &mut Self {
        self.insert(CommandSpec::of::<C>());
        self
    }

    /// Builder form of [`register`](Self::register).
    #[must_use]
    pub fn with<C: Command>(mut self) -> Self {
        self.register::<C>();
        self
    }

    fn insert(&mut self, spec: CommandSpec) {
        let inner = Arc::make_mut(&mut self.inner);
        let i = inner.specs.len();
        inner.index.entry(spec.meta.id).or_insert(i);
        inner.specs.push(spec);
    }

    /// The commands whose metadata passes `keep`, e.g. the agent catalog:
    /// `registry.filter(|m| m.agent_safe)`.
    #[must_use]
    pub fn filter(&self, keep: impl Fn(&CommandMeta) -> bool) -> Self {
        let mut out = Self::new();
        for spec in self.inner.specs.iter().filter(|s| keep(&s.meta)) {
            out.insert(spec.clone());
        }
        out
    }

    /// The commands offered to agents ([`CommandMeta::agent_safe`]).
    #[must_use]
    pub fn agent_catalog(&self) -> Self {
        self.filter(|m| m.agent_safe)
    }

    /// Every command, in registration order.
    #[must_use]
    pub fn list(&self) -> &[CommandSpec] {
        &self.inner.specs
    }

    /// The command `id`.
    #[must_use]
    pub fn describe(&self, id: &str) -> Option<&CommandSpec> {
        self.inner
            .index
            .get(id)
            .and_then(|&i| self.inner.specs.get(i))
    }

    /// Number of commands.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.specs.len()
    }

    /// `true` without commands.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.specs.is_empty()
    }

    /// [`OpErrorCode::UnknownCommand`](crate::OpErrorCode::UnknownCommand)
    /// for `id`, with close matches.
    #[must_use]
    pub fn unknown(&self, id: &str) -> OpError {
        let needle = id.to_lowercase();
        let tail = needle.rsplit('.').next().unwrap_or_default().to_owned();
        let close: Vec<&str> = self
            .inner
            .specs
            .iter()
            .map(|s| s.meta.id)
            .filter(|c| {
                let c = c.to_lowercase();
                let c_tail = c.rsplit('.').next().unwrap_or_default();
                (!needle.is_empty() && c.contains(&needle))
                    || edit_distance(&c, &needle) <= 2
                    || (tail.len() >= 4 && (c_tail == tail || edit_distance(c_tail, &tail) <= 1))
            })
            .take(5)
            .collect();
        let hint = if close.is_empty() {
            "; list them with commands.list (dashcraft commands)".to_owned()
        } else {
            format!("; did you mean {}?", close.join(", "))
        };
        OpError::unknown_command(format!("no command '{id}'{hint}"))
    }

    /// Runs command `id` with JSON `params` (`null` = `{}`) on `ws`.
    ///
    /// - unknown ids fail with `unknownCommand`, bad parameters with
    ///   `invalidParams` (before anything runs);
    /// - [`CallerKind::Agent`] callers may only run
    ///   [`agent_safe`](CommandMeta::agent_safe) commands;
    /// - commands that [mutate a document](CommandMeta::mutates_document)
    ///   wait for the previous mutating command on the same `docId`;
    /// - cancelling `ctx` ends the run with `cancelled`;
    /// - a panic becomes `internal`.
    ///
    /// # Errors
    /// The command's [`OpError`].
    pub async fn execute(
        &self,
        ws: &Workspace,
        id: &str,
        params: Value,
        ctx: &Ctx,
    ) -> Result<Value, OpError> {
        let spec = self.describe(id).ok_or_else(|| self.unknown(id))?;
        let meta = spec.meta;
        if ctx.caller() == CallerKind::Agent && !meta.agent_safe {
            return Err(OpError::unsupported(format!(
                "command {id} is not available to agents"
            )));
        }
        spec.validate(&params)?;
        let ctx = ctx.for_command(meta.id, self);
        ctx.check_cancelled()?;
        let lock = if meta.mutates_document {
            params
                .get("docId")
                .and_then(Value::as_str)
                .and_then(|d| ws.find(d))
                .map(|entry| entry.op_lock())
        } else {
            None
        };
        let _guard = match lock {
            Some(lock) => Some(tokio::select! {
                biased;
                () = ctx.cancel_token().cancelled() => return Err(OpError::cancelled()),
                guard = lock.lock_owned() => guard,
            }),
            None => None,
        };
        tracing::debug!(command = meta.id, caller = ?ctx.caller(), "running");
        let run = CatchUnwind::new(spec.call(ws, params, &ctx), meta.id);
        tokio::select! {
            biased;
            () = ctx.cancel_token().cancelled() => Err(OpError::cancelled()),
            result = run => result,
        }
    }

    /// [`execute`](Self::execute) with serializable parameters and a typed
    /// result.
    ///
    /// # Errors
    /// The command's [`OpError`]; `internal` if the result does not
    /// deserialize into `T`.
    pub async fn call<T: serde::de::DeserializeOwned>(
        &self,
        ws: &Workspace,
        id: &str,
        params: impl Serialize,
        ctx: &Ctx,
    ) -> Result<T, OpError> {
        let params = serde_json::to_value(params)
            .map_err(|e| OpError::invalid_params(format!("cannot serialize params: {e}")))?;
        let value = self.execute(ws, id, params, ctx).await?;
        serde_json::from_value(value)
            .map_err(|e| OpError::internal(format!("unexpected result of {id}: {e}")))
    }
}

/// Levenshtein distance (characters), for "did you mean".
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = Vec::with_capacity(b.len() + 1);
        cur.push(i + 1);
        for (j, cb) in b.iter().enumerate() {
            let substitute = prev.get(j).copied().unwrap_or(usize::MAX) + usize::from(ca != *cb);
            let delete = prev
                .get(j + 1)
                .copied()
                .unwrap_or(usize::MAX)
                .saturating_add(1);
            let insert = cur.get(j).copied().unwrap_or(usize::MAX).saturating_add(1);
            cur.push(substitute.min(delete).min(insert));
        }
        prev = cur;
    }
    prev.last().copied().unwrap_or(0)
}
