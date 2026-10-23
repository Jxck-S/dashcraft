//! The bridge to the command registry: `ops_execute` runs any `pbi-ops` command by id with
//! JSON parameters (`pbi_ops::Registry::execute`, the same code path as `dashcraft run`,
//! `dashcraft serve` and agents), forwards its progress as `ops://progress` events and can be
//! cancelled with `ops_cancel`. The parameter and result types of every command are generated
//! from the registry's JSON Schemas into `app/src/lib/ops.gen.ts`.

use std::collections::BTreeMap;

use pbi_ops::{CallerKind, CommandInfo, Ctx, OpError, ProgressEvent};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::dto::OpsCommandDto;
use crate::events::OpsProgress;
use crate::state::AppState;

/// TS shape of any JSON value. `serde_json::Value` itself cannot be exported: specta refuses
/// its 64-bit integers (JSON numbers are JS `number`s on the UI side anyway).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(untagged)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsonValue>),
    Object(BTreeMap<String, JsonValue>),
}

/// A JSON value on the wire: the parameters and result of `ops_execute`, typed per command by
/// `app/src/lib/ops.gen.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct Json(#[specta(type = JsonValue)] pub serde_json::Value);

/// A failed command: a stable code (`invalidParams`, `unknownCommand`, `notFound`,
/// `noDocument`, `unsupported`, `pathDenied`, `io`, `query`, `cancelled`, `busy`, `internal`),
/// a message for people and optional structured details (e.g. the valid names).
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OpErrorDto {
    pub code: String,
    pub message: String,
    #[specta(type = Option<JsonValue>)]
    pub data: Option<serde_json::Value>,
}

impl From<OpError> for OpErrorDto {
    fn from(e: OpError) -> Self {
        Self {
            code: e.code.as_str().to_owned(),
            message: e.message,
            data: e.data,
        }
    }
}

/// Unregisters a request however its run ends.
struct RequestGuard<'a> {
    state: &'a AppState,
    id: String,
}

impl Drop for RequestGuard<'_> {
    fn drop(&mut self) {
        self.state.end_request(&self.id);
    }
}

/// Runs registry command `id` (`doc.open`, `data.refresh`, `canvas.click`, …) with `params`
/// and returns its result. `requestId` (unique while running) names the run for `ops_cancel`
/// and the `ops://progress` events.
#[tauri::command]
#[specta::specta]
pub async fn ops_execute(
    request_id: String,
    id: String,
    params: Json,
    state: State<'_, AppState>,
) -> Result<Json, OpErrorDto> {
    let cancel = state.begin_request(&request_id, &id)?;
    let _guard = RequestGuard {
        state: &state,
        id: request_id.clone(),
    };
    let mut ctx = Ctx::new(CallerKind::App).with_cancel(cancel);
    if let Some(sink) = state.progress_sink() {
        let request = request_id.clone();
        ctx = ctx.with_progress(move |event: ProgressEvent| {
            sink.send(OpsProgress {
                request_id: request.clone(),
                command: event.command,
                message: event.message,
                fraction: event.fraction,
                data: event.data,
            });
        });
    }
    Ok(Json(
        state
            .registry
            .execute(&state.workspace, &id, params.0, &ctx)
            .await?,
    ))
}

/// Cancels the running request `requestId` (it then fails with `cancelled`). `false` if it is
/// not running.
#[tauri::command]
#[specta::specta]
pub fn ops_cancel(request_id: String, state: State<'_, AppState>) -> bool {
    state.cancel_request(&request_id)
}

/// The command catalog: ids, titles, categories and flags (without schemas; the typed client
/// is generated from them).
#[tauri::command]
#[specta::specta]
pub fn ops_catalog(state: State<'_, AppState>) -> Vec<OpsCommandDto> {
    state
        .registry
        .list()
        .iter()
        .map(|spec| {
            let info = CommandInfo::of(spec, false);
            OpsCommandDto {
                id: info.id,
                title: info.title,
                summary: info.summary,
                category: info.category.as_str().to_owned(),
                mutates_document: info.mutates_document,
                long_running: info.long_running,
                agent_safe: info.agent_safe,
                writes_files: info.writes_files,
            }
        })
        .collect()
}
