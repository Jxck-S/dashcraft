//! `doc.recovery.list`, `doc.recovery.restore`, `doc.recovery.discard`:
//! the recovery journals of unsaved edits ([`crate::recovery`], M8.1).

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::BASE;
use super::workspace::{DocSummary, NoParams};
use crate::command::{Category, Command, CommandMeta};
use crate::ctx::Ctx;
use crate::error::OpError;
use crate::recovery::{JournalDepth, RecoveryStore};
use crate::registry::Registry;
use crate::workspace::Workspace;

fn store(ws: &Workspace) -> Result<RecoveryStore, OpError> {
    ws.recovery().ok_or_else(|| {
        OpError::unsupported("recovery is off in this workspace (the host turns it on)")
    })
}

/// Refuses a journal that an open document is still writing.
fn not_open(ws: &Workspace, id: &str) -> Result<(), OpError> {
    match ws
        .documents()
        .into_iter()
        .find(|d| d.recovery_id().as_deref() == Some(id))
    {
        Some(doc) => Err(OpError::invalid_params(format!(
            "recovery journal '{id}' belongs to the open document {}",
            doc.id()
        ))),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------------------

/// `doc.recovery.list`.
#[derive(Debug)]
pub struct RecoveryList;

/// A recovery journal: the unsaved edits of a document that was not saved,
/// reloaded or closed (DashCraft stopped unexpectedly).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryJournalOut {
    /// The id to pass to `doc.recovery.restore` and `doc.recovery.discard`.
    pub recovery_id: String,
    /// The document's path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The document still exists at `path`.
    pub source_exists: bool,
    /// When the first edit was journaled (RFC 3339, UTC).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    /// When the last edit was journaled (RFC 3339, UTC).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    /// Journaled commands (edits, undos and redos).
    pub edits: usize,
    /// What the last edit did (its undo label).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_edit: Option<String>,
    /// Why it cannot be restored (unreadable, written by a newer DashCraft).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryListOut {
    /// Recovery is on in this workspace.
    pub enabled: bool,
    /// The journal folder (in the per-user data folder, never in a project).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// Journals not held by an open document, oldest first.
    pub journals: Vec<RecoveryJournalOut>,
}

impl Command for RecoveryList {
    type Params = NoParams;
    type Output = RecoveryListOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "doc.recovery.list",
            title: "List recoverable edits",
            summary: "The recovery journals of unsaved edits left by documents that were not saved or closed (e.g. after a crash).",
            category: Category::Workspace,
            agent_safe: false,
            examples: &[("Recoverable edits", "{}")],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, _p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let Some(store) = ws.recovery() else {
            return Ok(RecoveryListOut {
                enabled: false,
                location: None,
                journals: Vec::new(),
            });
        };
        let held: Vec<String> = ws
            .documents()
            .iter()
            .filter_map(|d| d.recovery_id())
            .collect();
        let list = {
            let store = store.clone();
            tokio::task::spawn_blocking(move || store.list()).await??
        };
        let journals = list
            .into_iter()
            .filter(|j| !held.contains(&j.id))
            .map(|j| {
                let path = j.header.as_ref().map(|h| h.path.clone());
                RecoveryJournalOut {
                    recovery_id: j.id,
                    source_exists: path.as_deref().is_some_and(|p| Path::new(p).exists()),
                    path,
                    created: j.header.map(|h| h.created),
                    updated: j.updated,
                    edits: j.edits,
                    last_edit: j.last_edit,
                    problem: j.problem,
                }
            })
            .collect();
        Ok(RecoveryListOut {
            enabled: true,
            location: Some(store.dir().to_string_lossy().into_owned()),
            journals,
        })
    }
}

// ---------------------------------------------------------------------------

/// `doc.recovery.restore`.
#[derive(Debug)]
pub struct RecoveryRestore;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoveryIdParams {
    /// A recovery id from `doc.recovery.list`.
    pub recovery_id: String,
}

/// The journaled command that could not be replayed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReplayFailureOut {
    /// Its position in the journal (0-based).
    pub index: usize,
    pub command: String,
    /// The error code (`invalidParams`, `notFound`, …).
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryRestoreOut {
    pub recovery_id: String,
    /// The reopened document with the recovered edits (unsaved).
    pub doc_id: String,
    pub document: DocSummary,
    /// Journaled commands.
    pub edits: usize,
    /// Commands replayed; fewer than `edits` when one failed.
    pub replayed: usize,
    /// The command that failed; the document keeps the edits before it and
    /// the journal is kept (discard it explicitly).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<ReplayFailureOut>,
    /// The document revision after the replay.
    pub revision: u64,
    /// The document has unsaved edits.
    pub dirty: bool,
}

/// Leaves a replaying document journaling again, also when the replay is
/// cancelled.
struct ReplayGuard {
    entry: std::sync::Arc<crate::workspace::DocEntry>,
    journal: Option<(String, JournalDepth)>,
}

impl Drop for ReplayGuard {
    fn drop(&mut self) {
        self.entry.end_replay(self.journal.take());
    }
}

fn registry(ctx: &Ctx) -> Result<&Registry, OpError> {
    ctx.registry().ok_or_else(|| {
        OpError::internal("no registry in the context (run through Registry::execute)")
    })
}

impl Command for RecoveryRestore {
    type Params = RecoveryIdParams;
    type Output = RecoveryRestoreOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "doc.recovery.restore",
            title: "Restore recoverable edits",
            summary: "Reopens the document of a recovery journal and replays its unsaved edits; the result is unsaved until doc.save.",
            category: Category::Workspace,
            agent_safe: false,
            invokes_other_commands: true,
            examples: &[(
                "Restore a journal",
                r#"{"recoveryId": "4f1c2a9e0b7d3e65-4242-1762898400000-0"}"#,
            )],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let store = store(ws)?;
        not_open(ws, &p.recovery_id)?;
        let journal = {
            let store = store.clone();
            let id = p.recovery_id.clone();
            tokio::task::spawn_blocking(move || store.read(&id)).await??
        };
        let registry = registry(ctx)?;
        let opened = registry
            .execute(ws, "doc.open", journal.header.open.clone(), ctx)
            .await
            .map_err(|e| e.context(format!("cannot reopen {}", journal.header.path)))?;
        let doc_id = opened["docId"].as_str().unwrap_or_default().to_owned();
        let entry = ws.get(&doc_id)?;
        entry.begin_replay();
        let mut guard = ReplayGuard {
            entry: std::sync::Arc::clone(&entry),
            journal: None,
        };
        let mut replayed = 0;
        let mut failed = None;
        for (index, step) in journal.entries.iter().enumerate() {
            let mut params = step.params.clone();
            if let Some(object) = params.as_object_mut() {
                object.insert("docId".into(), json!(doc_id));
            }
            match registry.execute(ws, &step.command, params, ctx).await {
                Ok(_) => replayed += 1,
                Err(e) if e.code == crate::OpErrorCode::Cancelled => return Err(e),
                Err(e) => {
                    failed = Some(ReplayFailureOut {
                        index,
                        command: step.command.clone(),
                        code: e.code.as_str().to_owned(),
                        message: e.message,
                    });
                    break;
                }
            }
        }
        if failed.is_none() {
            guard.journal =
                JournalDepth::of(&journal.entries).map(|depth| (p.recovery_id.clone(), depth));
        }
        drop(guard);
        let history = entry.session().document_history();
        Ok(RecoveryRestoreOut {
            recovery_id: p.recovery_id,
            document: super::workspace::summary(&entry).await,
            doc_id,
            edits: journal.entries.len(),
            replayed,
            failed,
            revision: history.revision,
            dirty: history.dirty,
        })
    }
}

// ---------------------------------------------------------------------------

/// `doc.recovery.discard`.
#[derive(Debug)]
pub struct RecoveryDiscard;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryDiscardOut {
    pub recovery_id: String,
    /// `false` when there was no such journal.
    pub discarded: bool,
}

impl Command for RecoveryDiscard {
    type Params = RecoveryIdParams;
    type Output = RecoveryDiscardOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "doc.recovery.discard",
            title: "Discard recoverable edits",
            summary: "Deletes a recovery journal and the unsaved edits it holds.",
            category: Category::Workspace,
            agent_safe: false,
            writes_files: true,
            examples: &[(
                "Discard a journal",
                r#"{"recoveryId": "4f1c2a9e0b7d3e65-4242-1762898400000-0"}"#,
            )],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let store = store(ws)?;
        not_open(ws, &p.recovery_id)?;
        let id = p.recovery_id.clone();
        let discarded = tokio::task::spawn_blocking(move || store.discard(&id)).await??;
        Ok(RecoveryDiscardOut {
            recovery_id: p.recovery_id,
            discarded,
        })
    }
}
