//! App-only IPC commands: everything that is not a domain operation of the command registry
//! (those run through [`crate::ops::ops_execute`]). Every command is registered in
//! [`crate::specta_builder`], which also drives the TypeScript bindings export.
//!
//! - app and window plumbing: version, the path to open at startup (argv or Finder), whether
//!   the debug-only AI demo provider is active;
//! - UI projections of an open registry document (by `docId`): the Data pane tree with object
//!   ids, the report outline, diagnostics and detection, the Properties pane, the TMDL view,
//!   the Model view layout and the Format pane;
//! - app settings: the blob map (`DASHCRAFT_BLOB_MAP`, Data source settings) and File → Recent.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pbi_model::ObjectId;
use pbi_ops::DocEntry;
use pbi_session::CacheInfo;
use tauri::State;

use crate::blob::BlobMapSetting;
use crate::dto::{
    BlobMapDto, CacheInfoDto, DocumentViewsDto, EntryKind, ModelDiagramDto, ObjectDetailsDto,
    ObjectIdDto, RecentFileDto, TmdlViewDto, VisualFormatDto,
};
use crate::error::AppError;
use crate::mapping::{self, PathInfo};
use crate::state::AppState;
use crate::views;
use crate::{PendingOpen, StartupPath};

/// Version of the running app (the workspace version).
#[tauri::command]
#[specta::specta]
pub fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// AI requests are answered by the debug-only demo provider (`DASHCRAFT_AI_MOCK=1` in a debug
/// build): previews show the real request, answers are canned and nothing is sent. Always
/// `false` in release builds.
#[tauri::command]
#[specta::specta]
pub fn ai_demo_mode() -> bool {
    crate::ai::demo_active()
}

/// Path given on the command line (`dashcraft-app <path>`), if any. The UI calls this once
/// on startup and opens the path; it is not consumed so a webview reload reopens it.
#[tauri::command]
#[specta::specta]
pub fn startup_path(
    state: State<'_, StartupPath>,
    pending: State<'_, PendingOpen>,
) -> Option<String> {
    // A document the OS asked to open (macOS) wins over argv; it is handed out once.
    pending
        .take()
        .or_else(|| state.get().map(Path::to_path_buf))
        .map(|p| p.to_string_lossy().into_owned())
}

/// The open document `doc_id` of the workspace.
fn document(state: &AppState, doc_id: &str) -> Result<Arc<DocEntry>, AppError> {
    state
        .workspace
        .find(doc_id)
        .ok_or_else(|| AppError::NoDocument(format!("no open document '{doc_id}'")))
}

fn worker_failed(e: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!("worker failed: {e}"))
}

/// The local data cache, for the document summary.
fn cache_info(info: CacheInfo) -> CacheInfoDto {
    CacheInfoDto {
        entry: info.entry,
        size_bytes: info.size,
        modified: info
            .modified
            .map(|m| m.format("%Y-%m-%dT%H:%M:%SZ").to_string()),
    }
}

/// Everything the panes need about the open document `doc_id` (opened with `doc.open`): its
/// summary, the semantic model tree (tables with columns, measures, partitions, hierarchies;
/// relationships; named expressions), the reports (pages → visuals with positions and fields,
/// filters, theme), the reader diagnostics and how the document was recognised.
#[tauri::command]
#[specta::specta]
pub async fn document_views(
    doc_id: String,
    state: State<'_, AppState>,
) -> Result<DocumentViewsDto, AppError> {
    let entry = document(&state, &doc_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let info = path_info(entry.path())?;
        let session = entry.session();
        let opened = session.opened();
        let mut summary = mapping::summary(opened, entry.id(), &info);
        summary.cache = session.cache_info().map(cache_info);
        Ok(DocumentViewsDto {
            summary,
            tree: mapping::model_tree(opened),
            reports: mapping::report_outline(opened),
            diagnostics: mapping::diagnostics(opened),
            detection: mapping::detection(opened),
        })
    })
    .await
    .map_err(worker_failed)?
}

/// Everything about one model or report object, for the Properties pane: properties, full
/// DAX / M / JSON text, annotations, unmodelled properties and the source file.
#[tauri::command]
#[specta::specta]
pub async fn object_details(
    doc_id: String,
    id: ObjectIdDto,
    state: State<'_, AppState>,
) -> Result<ObjectDetailsDto, AppError> {
    let entry = document(&state, &doc_id)?;
    mapping::object_details(entry.session().opened(), ObjectId(u64::from(id)))
        .ok_or_else(|| AppError::UnknownObject(format!("no object #{id} in the open document")))
}

/// TMDL view: the model as TMDL scripts (read-only). The opened `.tmdl` files verbatim when the
/// model came from TMDL (PBIP, TMDL folder), else canonical TMDL generated from the model.
/// Fails with `unsupported` without a local model (remote / missing).
#[tauri::command]
#[specta::specta]
pub async fn tmdl_view(
    doc_id: String,
    state: State<'_, AppState>,
) -> Result<TmdlViewDto, AppError> {
    let entry = document(&state, &doc_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let opened = entry.session().opened();
        let model = opened.document.model.as_ref().ok_or_else(|| {
            AppError::Unsupported(
                "the document has no local semantic model (remote or missing)".to_owned(),
            )
        })?;
        Ok(views::tmdl_view(opened, model))
    })
    .await
    .map_err(worker_failed)?
}

/// Model view layouts from the semantic model's `diagramLayout.json`, if the document has one
/// (`null` otherwise: the UI lays the tables out itself).
#[tauri::command]
#[specta::specta]
pub async fn model_diagram(
    doc_id: String,
    state: State<'_, AppState>,
) -> Result<Option<ModelDiagramDto>, AppError> {
    let entry = document(&state, &doc_id)?;
    Ok(entry
        .session()
        .opened()
        .part("diagramLayout")
        .and_then(|p| p.json.as_ref())
        .map(views::model_diagram))
}

/// Format pane of a visual (read-only): its formatting objects (`visual.objects` →
/// `visual`, `visualContainerObjects` → `general`) as cards of readable properties, plus its
/// position, size and layer.
#[tauri::command]
#[specta::specta]
pub async fn visual_format(
    doc_id: String,
    id: ObjectIdDto,
    state: State<'_, AppState>,
) -> Result<VisualFormatDto, AppError> {
    let entry = document(&state, &doc_id)?;
    views::find_visual(&entry.session().document().reports, id)
        .map(views::visual_format)
        .ok_or_else(|| AppError::UnknownObject(format!("no visual #{id} in the open document")))
}

// ---------------------------------------------------------------------------
// Settings: blob map, File → Recent
// ---------------------------------------------------------------------------

fn blob_map_dto(setting: BlobMapSetting) -> BlobMapDto {
    BlobMapDto {
        path: setting.path.map(|p| p.to_string_lossy().into_owned()),
        from_environment: setting.env_var.is_some(),
        environment_variable: setting.env_var.map(str::to_owned),
        error: setting.error,
    }
}

/// The blob map used by `AzureStorage.Blobs` sources (`DASHCRAFT_BLOB_MAP` at startup, then
/// whatever `set_blob_map` chose). Applies to every document of the workspace.
#[tauri::command]
#[specta::specta]
pub async fn blob_map(state: State<'_, AppState>) -> Result<BlobMapDto, AppError> {
    Ok(blob_map_dto(state.blob_map.setting()))
}

/// Uses the `blob-map.json` at `path` (local files only; entries resolve relative to it) for
/// the next refresh, or goes offline with `null`. Loaded data is kept. An unreadable map fails
/// and leaves the previous one active; so does a running `data.refresh` (`busy`).
#[tauri::command]
#[specta::specta]
pub async fn set_blob_map(
    path: Option<String>,
    state: State<'_, AppState>,
) -> Result<BlobMapDto, AppError> {
    if state.is_running("data.refresh") {
        return Err(AppError::Busy(
            "the blob map cannot change while a refresh is running".to_owned(),
        ));
    }
    let setting = state.blob_map.set(path.as_deref().map(Path::new))?;
    Ok(blob_map_dto(setting))
}

/// File → Recent: recently opened files and folders, newest first.
#[tauri::command]
#[specta::specta]
pub async fn recent_files(state: State<'_, AppState>) -> Result<Vec<RecentFileDto>, AppError> {
    Ok(state.recent.list())
}

/// Adds a successfully opened file or folder to File → Recent. Returns the new list.
#[tauri::command]
#[specta::specta]
pub async fn add_recent_file(
    path: String,
    state: State<'_, AppState>,
) -> Result<Vec<RecentFileDto>, AppError> {
    let info = path_info(Path::new(&path))?;
    state.recent.add(&info.path);
    Ok(state.recent.list())
}

/// Removes one entry from File → Recent (`null`: clears the list). Returns the new list.
#[tauri::command]
#[specta::specta]
pub async fn remove_recent_file(
    path: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<RecentFileDto>, AppError> {
    state.recent.remove(path.as_deref());
    Ok(state.recent.list())
}

/// Validates `path` and gathers filesystem facts (absolute path, name, kind, size).
pub(crate) fn path_info(path: &Path) -> Result<PathInfo, AppError> {
    if path.as_os_str().is_empty() {
        return Err(AppError::InvalidArgument("path is empty".to_owned()));
    }
    let abs: PathBuf = std::path::absolute(path).map_err(|e| AppError::from_io(path, &e))?;
    let meta = std::fs::metadata(&abs).map_err(|e| AppError::from_io(&abs, &e))?;

    let (entry_kind, size_bytes) = if meta.is_file() {
        (EntryKind::File, Some(meta.len()))
    } else if meta.is_dir() {
        (EntryKind::Directory, None)
    } else {
        return Err(AppError::Unsupported(format!(
            "{} is neither a file nor a directory",
            abs.display()
        )));
    };

    let file_name = abs
        .file_name()
        .map_or_else(|| abs.to_string_lossy(), |n| n.to_string_lossy())
        .into_owned();

    Ok(PathInfo {
        path: abs.to_string_lossy().into_owned(),
        file_name,
        entry_kind,
        size_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspects_a_file() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let res = path_info(&manifest).unwrap();
        assert_eq!(res.entry_kind, EntryKind::File);
        assert_eq!(res.file_name, "Cargo.toml");
        assert!(res.size_bytes.unwrap() > 0);
    }

    #[test]
    fn inspects_a_directory() {
        let res = path_info(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        assert_eq!(res.entry_kind, EntryKind::Directory);
        assert_eq!(res.file_name, "src-tauri");
        assert_eq!(res.size_bytes, None);
    }

    #[test]
    fn reports_missing_and_empty_paths() {
        let missing = Path::new(env!("CARGO_MANIFEST_DIR")).join("does-not-exist.pbix");
        assert!(matches!(path_info(&missing), Err(AppError::NotFound(_))));
        assert!(matches!(
            path_info(Path::new("")),
            Err(AppError::InvalidArgument(_))
        ));
    }
}
