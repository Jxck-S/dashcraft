//! `commands.list`, `commands.describe`, `commands.batch`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::BASE;
use crate::batch::{BatchStep, BatchStepResult, MAX_STEPS, run_batch};
use crate::command::{Category, Command, CommandMeta};
use crate::ctx::Ctx;
use crate::error::OpError;
use crate::registry::{CommandInfo, Registry};
use crate::workspace::Workspace;

fn registry(ctx: &Ctx) -> Result<&Registry, OpError> {
    ctx.registry().ok_or_else(|| {
        OpError::internal("no registry in the context (run through Registry::execute)")
    })
}

/// `commands.list`.
#[derive(Debug)]
pub struct CommandsList;

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandsListParams {
    /// Include each command's parameter and result JSON Schemas.
    #[serde(default)]
    pub schemas: bool,
    /// Only this category.
    #[serde(default)]
    pub category: Option<Category>,
    /// Only the commands offered to agents.
    #[serde(default)]
    pub agent_safe: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommandsListOut {
    pub count: usize,
    pub commands: Vec<CommandInfo>,
}

impl Command for CommandsList {
    type Params = CommandsListParams;
    type Output = CommandsListOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "commands.list",
            title: "List commands",
            summary: "Lists every command with its metadata (and JSON Schemas with `schemas`).",
            category: Category::Meta,
            examples: &[
                ("All commands", "{}"),
                (
                    "Agent tools with schemas",
                    r#"{"schemas": true, "agentSafe": true}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(_ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let commands: Vec<CommandInfo> = registry(ctx)?
            .list()
            .iter()
            .filter(|s| p.category.is_none_or(|c| s.meta.category == c))
            .filter(|s| !p.agent_safe || s.meta.agent_safe)
            .map(|s| CommandInfo::of(s, p.schemas))
            .collect();
        Ok(CommandsListOut {
            count: commands.len(),
            commands,
        })
    }
}

/// `commands.describe`.
#[derive(Debug)]
pub struct CommandsDescribe;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandsDescribeParams {
    /// Command id, e.g. `doc.open`.
    pub id: String,
}

impl Command for CommandsDescribe {
    type Params = CommandsDescribeParams;
    type Output = CommandInfo;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "commands.describe",
            title: "Describe a command",
            summary: "One command's metadata, examples and parameter/result JSON Schemas.",
            category: Category::Meta,
            examples: &[("Describe doc.open", r#"{"id": "doc.open"}"#)],
            ..BASE
        }
    }

    async fn run(_ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let registry = registry(ctx)?;
        let spec = registry
            .describe(&p.id)
            .ok_or_else(|| registry.unknown(&p.id))?;
        Ok(CommandInfo::of(spec, true))
    }
}

/// `commands.batch`.
#[derive(Debug)]
pub struct CommandsBatch;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandsBatchParams {
    /// Steps, run in order on the same workspace.
    pub steps: Vec<BatchStep>,
    /// Stop at the first failing step (default `true`). Batches are not
    /// atomic: earlier steps stay applied.
    #[serde(default = "yes")]
    pub stop_on_error: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommandsBatchOut {
    /// Every step ran and succeeded.
    pub ok: bool,
    /// One entry per step that ran, in order.
    pub results: Vec<BatchStepResult>,
}

impl Command for CommandsBatch {
    type Params = CommandsBatchParams;
    type Output = CommandsBatchOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "commands.batch",
            title: "Run a batch",
            summary: "Runs ordered steps on the shared workspace; strings like \"${d.docId}\" reference earlier results named with \"as\".",
            category: Category::Meta,
            invokes_other_commands: true,
            examples: &[(
                "Open, then list pages",
                r#"{"steps": [{"id": "doc.open", "params": {"path": "examples/acme-outfitters/AcmeOutfitters.pbip"}, "as": "d"}, {"id": "report.pages", "params": {"docId": "${d.docId}"}}]}"#,
            )],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        if p.steps.len() > MAX_STEPS {
            return Err(OpError::invalid_params(format!(
                "a batch has at most {MAX_STEPS} steps"
            )));
        }
        let registry = registry(ctx)?.clone();
        let results = run_batch(
            &registry,
            ws,
            &p.steps,
            p.stop_on_error,
            serde_json::Map::new(),
            ctx,
        )
        .await;
        Ok(CommandsBatchOut {
            ok: results.len() == p.steps.len() && results.iter().all(|r| r.ok),
            results,
        })
    }
}
