//! Typed Tauri events (tauri-specta): registered in [`crate::specta_builder`] and exported to
//! `bindings.ts` as `events.*`.

use serde::Serialize;
use specta::Type;

/// Progress of a running `ops_execute` request (`pbi_ops::ProgressEvent`), emitted as the
/// `ops://progress` event.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OpsProgress {
    /// The `requestId` given to `ops_execute`.
    pub request_id: String,
    /// The command id (`data.refresh`).
    pub command: String,
    /// What is happening, for people (`Orders: 3900 rows in 120 ms`).
    pub message: String,
    /// Completion between 0 and 1, when known.
    pub fraction: Option<f64>,
    /// Structured details: for `data.refresh` and `data.loadCsv` the `pbi-session` refresh
    /// event (`{ "event": "tableFinished", "table": { … } }`).
    #[specta(type = Option<crate::ops::JsonValue>)]
    pub data: Option<serde_json::Value>,
}

// Hand-written instead of `#[derive(tauri_specta::Event)]`, which would pull in the
// `tauri-specta-macros` crate for a single constant.
impl tauri_specta::Event for OpsProgress {
    const NAME: &'static str = "ops://progress";
}

/// A file the operating system asked the running app to open (macOS Finder double-click or
/// "Open With" on an associated document), emitted as the `open-file-requested` event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OpenFileRequested {
    /// Absolute path of the file to open.
    pub path: String,
}

impl tauri_specta::Event for OpenFileRequested {
    const NAME: &'static str = "open-file-requested";
}
