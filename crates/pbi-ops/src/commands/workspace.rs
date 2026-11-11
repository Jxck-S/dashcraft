//! `doc.detect`, `doc.open`, `doc.close`, `doc.list`, `doc.status`,
//! `doc.externalChanges`, `doc.reload`, `doc.diff`.

use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use pbi_core::OpenOptions;
use pbi_core::container::Limits;
use pbi_model::Severity;
use pbi_session::{Clock, FixedClock, RefreshSource, Session, SessionError, SessionOptions};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::edit::FileChangeOut;
use super::{BASE, DocParams, doc};
use crate::command::{Category, Command, CommandMeta};
use crate::ctx::{Ctx, DocChangedEvent};
use crate::detect_model::{DetectOut, detect_out};
use crate::error::{OpError, open_error};
use crate::inspect_model::{OriginOut, detection_out};
use crate::workspace::{DocEntry, DocOptions, Workspace};

/// Where a document's data comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SourceParam {
    /// A PBIP's local data cache (`.pbi/cache.abf`, Power BI Desktop's last
    /// refresh) when there is one, else the M queries.
    #[default]
    Auto,
    /// Evaluate the M queries (PBIX: their embedded data).
    #[serde(alias = "m")]
    Query,
    /// The local data cache only (fails without one).
    Cache,
}

impl From<SourceParam> for RefreshSource {
    fn from(s: SourceParam) -> Self {
        match s {
            SourceParam::Auto => RefreshSource::Auto,
            SourceParam::Query => RefreshSource::Query,
            SourceParam::Cache => RefreshSource::Cache,
        }
    }
}

/// `auto`, `query` or `cache`.
#[must_use]
pub fn source_name(s: RefreshSource) -> &'static str {
    match s {
        RefreshSource::Auto => "auto",
        RefreshSource::Query => "query",
        RefreshSource::Cache => "cache",
    }
}

// ---------------------------------------------------------------------------

/// `doc.detect`.
#[derive(Debug)]
pub struct DocDetect;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PathParams {
    /// File or folder.
    pub path: PathBuf,
}

impl Command for DocDetect {
    type Params = PathParams;
    type Output = DetectOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "doc.detect",
            title: "Detect format",
            summary: "Content-based format detection of a file or folder (signals, models, reports) without reading it fully.",
            category: Category::Workspace,
            examples: &[(
                "A PBIP project",
                r#"{"path": "examples/acme-outfitters/AcmeOutfitters.pbip"}"#,
            )],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let path = ws.policy().check_read(&p.path)?;
        let shown = p.path.to_string_lossy().into_owned();
        let read = path.clone();
        let detection = tokio::task::spawn_blocking(move || {
            pbi_core::detect::detect_path(&read, &Limits::default())
        })
        .await?
        .map_err(|e| open_error(&p.path, &path, e.into()))?;
        Ok(detect_out(&shown, &detection))
    }
}

// ---------------------------------------------------------------------------

/// `doc.open`.
#[derive(Debug)]
pub struct DocOpen;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocOpenParams {
    /// File or folder: `.pbip`, `.pbix`, `.pbit`, `*.SemanticModel`,
    /// `model.bim`, a TMDL folder, … (see FORMATS.md).
    pub path: PathBuf,
    /// Offline `AzureStorage.Blobs` mapping (`blob-map.json`: blob URLs →
    /// local files). Default: the workspace's blob map.
    #[serde(default)]
    pub blob_map: Option<PathBuf>,
    /// Default data source of refreshes (default `auto`).
    #[serde(default)]
    pub source: Option<SourceParam>,
    /// Decompress the PBIX `DataModel` / PBIP cache to read the model schema
    /// (default `true`).
    #[serde(default)]
    pub load_pbix_schema: Option<bool>,
    /// Keep internal VertiPaq tables (`H$`, `R$`, `U$`) (default `false`).
    #[serde(default)]
    pub include_system_tables: bool,
    /// Data commands load the data on first use when nothing was loaded
    /// (default `true`). Turn off to see empty tables until `data.refresh`.
    #[serde(default)]
    pub auto_refresh: Option<bool>,
    /// The time M sees as "now" (RFC 3339), for reproducible "last refreshed"
    /// values. Default: the system clock.
    #[serde(default)]
    pub now: Option<String>,
}

/// Counts of diagnostics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticCounts {
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
}

/// A report of an open document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportBrief {
    pub name: String,
    pub pages: usize,
}

/// Summary of an open document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocSummary {
    /// The id to pass to other commands.
    pub doc_id: String,
    /// The path as given.
    pub path: String,
    /// Source format (`pbip`, `pbix`, `pbit`, `bim`, `tmdl`, …).
    pub format: String,
    /// Detected format, for people.
    pub format_label: String,
    pub has_model: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    /// Data comes from embedded VertiPaq storage (PBIX, ABF).
    pub embedded_data: bool,
    /// A PBIP local data cache (`.pbi/cache.abf`) is available.
    pub has_cache: bool,
    /// A blob map is configured.
    pub blob_map: bool,
    /// Default refresh source: `auto`, `query` or `cache`.
    pub source: String,
    pub auto_refresh: bool,
    /// Data was loaded.
    pub loaded: bool,
    /// Data generation (changes after every data change).
    pub generation: u64,
    pub tables: usize,
    pub measures: usize,
    pub relationships: usize,
    pub reports: Vec<ReportBrief>,
    /// Reader diagnostics.
    pub diagnostics: DiagnosticCounts,
}

/// The summary of `entry`.
pub async fn summary(entry: &DocEntry) -> DocSummary {
    let session = entry.session();
    let doc = session.document();
    let opened = session.opened();
    let empty = pbi_model::Model::default();
    let model = opened.document.model.as_ref().unwrap_or(&empty);
    let mut diagnostics = DiagnosticCounts::default();
    for d in &doc.diagnostics {
        match d.severity {
            Severity::Error => diagnostics.errors += 1,
            Severity::Warning => diagnostics.warnings += 1,
            Severity::Info => diagnostics.infos += 1,
        }
    }
    DocSummary {
        doc_id: entry.id().to_owned(),
        path: entry.shown_path(),
        format: doc.source.format.as_str().to_owned(),
        format_label: detection_out(&session.opened().detection).label,
        has_model: session.has_model(),
        model_name: doc.model.as_ref().map(|m| m.name.clone()),
        embedded_data: session.is_pbix(),
        has_cache: session.has_cached_data(),
        blob_map: session.has_blob_map(),
        source: source_name(entry.options().source).to_owned(),
        auto_refresh: entry.options().auto_refresh,
        loaded: entry.is_loaded().await,
        generation: entry.generation(),
        tables: model.tables.len(),
        measures: model.tables.iter().map(|t| t.measures.len()).sum(),
        relationships: model.relationships.len(),
        reports: doc
            .reports
            .iter()
            .map(|r| ReportBrief {
                name: r.name.clone(),
                pages: r.pages.len(),
            })
            .collect(),
        diagnostics,
    }
}

fn parse_now(text: &str) -> Result<Arc<dyn Clock>, OpError> {
    let t = DateTime::parse_from_rfc3339(text.trim()).map_err(|e| {
        OpError::invalid_params(format!(
            "now: '{text}' is not an RFC 3339 date-time ({e}), e.g. 2026-10-21T21:35:00Z"
        ))
    })?;
    Ok(Arc::new(FixedClock {
        now: t.with_timezone(&Utc),
        offset: *t.offset(),
    }))
}

impl Command for DocOpen {
    type Params = DocOpenParams;
    type Output = DocSummary;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "doc.open",
            title: "Open a document",
            summary: "Opens a Power BI file or folder into the workspace (no data loaded yet) and returns its docId and summary.",
            category: Category::Workspace,
            examples: &[
                (
                    "A PBIP project",
                    r#"{"path": "examples/acme-outfitters/AcmeOutfitters.pbip"}"#,
                ),
                (
                    "With a blob map and a pinned clock",
                    r#"{"path": "examples/acme-outfitters/AcmeOutfitters.pbip", "blobMap": "examples/acme-outfitters/blob-map.json", "now": "2026-10-21T21:35:00Z"}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let path = ws.policy().check_read(&p.path)?;
        let blob_map = match &p.blob_map {
            Some(m) => Some(ws.policy().check_blob_map(m)?),
            None => ws.settings().blob_map,
        };
        let clock = p.now.as_deref().map(parse_now).transpose()?;
        let options = DocOptions {
            source: p.source.unwrap_or_default().into(),
            auto_refresh: p.auto_refresh.unwrap_or(true),
            blob_map: blob_map.clone(),
            load_pbix_schema: p.load_pbix_schema.unwrap_or(true),
            include_system_tables: p.include_system_tables,
            now: p.now.clone(),
        };
        let mut session_options = SessionOptions {
            open: OpenOptions {
                include_system_tables: options.include_system_tables,
                load_pbix_schema: options.load_pbix_schema,
                ..OpenOptions::default()
            },
            blob_map,
            clock,
            ..SessionOptions::default()
        };
        if p.blob_map.is_none() {
            ws.apply_open_hook(&mut session_options);
        }
        let open_path = path.clone();
        let session =
            tokio::task::spawn_blocking(move || Session::open(&open_path, session_options))
                .await?
                .map_err(|e| match e {
                    SessionError::Open(e) => open_error(&p.path, &path, e),
                    e => OpError::from(e).context(format!("cannot open {}", p.path.display())),
                })?;
        let entry = ws.insert(
            session,
            path,
            p.path.to_string_lossy().into_owned(),
            options,
        )?;
        Ok(summary(&entry).await)
    }
}

// ---------------------------------------------------------------------------

/// `doc.close`.
#[derive(Debug)]
pub struct DocClose;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocCloseParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Explicitly discard unsaved in-memory edits.
    #[serde(default)]
    pub discard_changes: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocCloseOut {
    /// The document id.
    pub doc_id: String,
    pub closed: bool,
}

impl Command for DocClose {
    type Params = DocCloseParams;
    type Output = DocCloseOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "doc.close",
            title: "Close a document",
            summary: "Closes an open document and frees its data; unsaved edits require explicit discardChanges.",
            category: Category::Workspace,
            mutates_document: true,
            examples: &[("Close d1", r#"{"docId": "d1"}"#)],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &p.doc_id)?;
        if entry.session().is_document_dirty() && !p.discard_changes {
            return Err(OpError::unsupported(
                "document has unsaved edits; pass \"discardChanges\": true to close and discard them",
            ));
        }
        let closed = ws.remove(&p.doc_id).is_some();
        Ok(DocCloseOut {
            doc_id: p.doc_id,
            closed,
        })
    }
}

// ---------------------------------------------------------------------------

/// `doc.list`.
#[derive(Debug)]
pub struct DocList;

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoParams {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocListOut {
    pub documents: Vec<DocSummary>,
}

impl Command for DocList {
    type Params = NoParams;
    type Output = DocListOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "doc.list",
            title: "List open documents",
            summary: "The open documents of the workspace with their summaries.",
            category: Category::Workspace,
            examples: &[("All open documents", "{}")],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, _p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let mut documents = Vec::new();
        for entry in ws.documents() {
            documents.push(summary(&entry).await);
        }
        Ok(DocListOut { documents })
    }
}

// ---------------------------------------------------------------------------

/// The project files changed on disk since the open or last save, `None`
/// when the document does not track them.
async fn external_changes(entry: &DocEntry) -> Result<Option<Vec<FileChangeOut>>, OpError> {
    let changes = entry.session().external_changes().await.map_err(|e| {
        OpError::from(e).context(format!("cannot check {} for changes", entry.shown_path()))
    })?;
    Ok(changes.map(|c| c.into_iter().map(FileChangeOut::from).collect()))
}

/// `doc.status`.
#[derive(Debug)]
pub struct DocStatus;

/// The state of an open document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocStatusOut {
    /// The document id.
    pub doc_id: String,
    /// The path as given (moved by a Save As).
    pub path: String,
    /// Source format (`pbip`, `pbix`, …).
    pub format: String,
    /// The document revision.
    pub revision: u64,
    /// The document has unsaved edits.
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    /// `doc.save` can save in place (a PBIP project opened from a folder);
    /// otherwise use `doc.saveAs`.
    pub can_save: bool,
    /// Project files are tracked for external changes (PBIP projects).
    pub tracks_external_changes: bool,
    /// Project files added, changed or removed on disk by another program
    /// since the open or last save: `doc.save` refuses them as a conflict
    /// without `overwrite`; `doc.reload` rereads them.
    pub external_changes: Vec<FileChangeOut>,
    /// Data was loaded.
    pub loaded: bool,
    /// Data generation (changes after every data change and reload).
    pub generation: u64,
}

impl Command for DocStatus {
    type Params = DocParams;
    type Output = DocStatusOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "doc.status",
            title: "Document status",
            summary: "Revision, unsaved edits, undo/redo, whether doc.save can save in place, and the project files changed on disk by other programs since the open or last save.",
            category: Category::Workspace,
            examples: &[("Status of d1", r#"{"docId": "d1"}"#)],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &p.doc_id)?;
        let session = entry.session();
        let history = session.document_history();
        let opened = session.opened();
        let can_save = opened.document.source.format == pbi_model::SourceFormat::Pbip
            && opened.container.root_dir().is_some();
        let changes = external_changes(&entry).await?;
        Ok(DocStatusOut {
            doc_id: entry.id().to_owned(),
            path: entry.shown_path(),
            format: opened.document.source.format.as_str().to_owned(),
            revision: history.revision,
            dirty: history.dirty,
            can_undo: history.can_undo,
            can_redo: history.can_redo,
            can_save,
            tracks_external_changes: changes.is_some(),
            external_changes: changes.unwrap_or_default(),
            loaded: entry.is_loaded().await,
            generation: entry.generation(),
        })
    }
}

// ---------------------------------------------------------------------------

/// `doc.externalChanges`.
#[derive(Debug)]
pub struct DocExternalChanges;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocExternalChangesOut {
    /// The document id.
    pub doc_id: String,
    /// Project files are tracked (PBIP projects); other documents report
    /// no changes.
    pub tracked: bool,
    /// Files added, changed or removed on disk by another program since
    /// the open or last save, by path.
    pub changes: Vec<FileChangeOut>,
    /// `doc.save` without `overwrite` would refuse these changes as a
    /// conflict.
    pub conflict: bool,
}

impl Command for DocExternalChanges {
    type Params = DocParams;
    type Output = DocExternalChangesOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "doc.externalChanges",
            title: "External changes",
            summary: "The project files added, changed or removed on disk by other programs (Git, editors, Power BI Desktop) since the open or last save.",
            category: Category::Workspace,
            examples: &[("Changes under d1", r#"{"docId": "d1"}"#)],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &p.doc_id)?;
        let changes = external_changes(&entry).await?;
        let tracked = changes.is_some();
        let changes = changes.unwrap_or_default();
        Ok(DocExternalChangesOut {
            doc_id: entry.id().to_owned(),
            tracked,
            conflict: !changes.is_empty(),
            changes,
        })
    }
}

// ---------------------------------------------------------------------------

/// `doc.reload`.
#[derive(Debug)]
pub struct DocReload;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocReloadParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Explicitly discard unsaved in-memory edits.
    #[serde(default)]
    pub discard_changes: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocReloadOut {
    /// The document id (unchanged).
    pub doc_id: String,
    /// The new revision: a saved state, newer than any before.
    pub revision: u64,
    /// Unsaved edits were discarded.
    pub discarded_edits: bool,
    /// Loaded data was dropped: data commands load it again on first use
    /// (`autoRefresh`), or refresh with `data.refresh`.
    pub data_unloaded: bool,
    /// The reloaded document.
    pub summary: DocSummary,
}

impl Command for DocReload {
    type Params = DocReloadParams;
    type Output = DocReloadOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "doc.reload",
            title: "Reload from disk",
            summary: "Rereads an open document from its files (e.g. after external changes), keeping its docId; drops its undo history, loaded data and canvas state. Unsaved edits require explicit discardChanges.",
            category: Category::Workspace,
            mutates_document: true,
            agent_safe: false,
            examples: &[
                ("Reload d1", r#"{"docId": "d1"}"#),
                (
                    "Discard unsaved edits",
                    r#"{"docId": "d1", "discardChanges": true}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &p.doc_id)?;
        let dirty = entry.session().is_document_dirty();
        if dirty && !p.discard_changes {
            return Err(OpError::unsupported(
                "document has unsaved edits; pass \"discardChanges\": true to reload and discard them, or save them first",
            ));
        }
        let (revision, data_unloaded) = entry.reload(ctx).await.map_err(|e| match e {
            SessionError::Open(e) => {
                let path = entry.path();
                open_error(&path, &path, e).context("cannot reload; the open document is unchanged")
            }
            e => OpError::from(e),
        })?;
        ctx.doc_changed(DocChangedEvent {
            doc_id: entry.id().to_owned(),
            command: ctx.command().unwrap_or_default().to_owned(),
            revision,
            dirty: false,
            changes: Vec::new(),
            path: None,
            reloaded: true,
        });
        Ok(DocReloadOut {
            doc_id: entry.id().to_owned(),
            revision,
            discarded_edits: dirty,
            data_unloaded,
            summary: summary(&entry).await,
        })
    }
}

// ---------------------------------------------------------------------------

/// `doc.diff`.
#[derive(Debug)]
pub struct DocDiff;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocDiffParams {
    /// The document id (`d1`, …): its current revision, unsaved edits
    /// included, is the "after" side.
    pub doc_id: String,
    /// Compare with another open document's current revision instead of
    /// the document's own files on disk.
    #[serde(default)]
    pub against_doc_id: Option<String>,
    /// Compare with a file or folder instead of the document's own files
    /// on disk (read with the document's reader options).
    #[serde(default)]
    pub against_path: Option<PathBuf>,
}

/// The kind of a compared object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum DiffKindOut {
    Model,
    Database,
    Table,
    Column,
    Measure,
    Partition,
    Hierarchy,
    CalculationItem,
    Relationship,
    Expression,
    QueryGroup,
    Role,
    Culture,
    Perspective,
    Report,
    ReportMeasure,
    Page,
    Visual,
}

impl From<pbi_core::DiffKind> for DiffKindOut {
    fn from(k: pbi_core::DiffKind) -> Self {
        use pbi_core::DiffKind as K;
        match k {
            K::Model => Self::Model,
            K::Database => Self::Database,
            K::Table => Self::Table,
            K::Column => Self::Column,
            K::Measure => Self::Measure,
            K::Partition => Self::Partition,
            K::Hierarchy => Self::Hierarchy,
            K::CalculationItem => Self::CalculationItem,
            K::Relationship => Self::Relationship,
            K::Expression => Self::Expression,
            K::QueryGroup => Self::QueryGroup,
            K::Role => Self::Role,
            K::Culture => Self::Culture,
            K::Perspective => Self::Perspective,
            K::Report => Self::Report,
            K::ReportMeasure => Self::ReportMeasure,
            K::Page => Self::Page,
            K::Visual => Self::Visual,
        }
    }
}

/// How an object differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum DiffChangeOut {
    /// Only in the document (`docId`).
    Added,
    /// Only in what it is compared with.
    Removed,
    /// In both, with different properties.
    Changed,
}

impl From<pbi_core::DiffChange> for DiffChangeOut {
    fn from(c: pbi_core::DiffChange) -> Self {
        match c {
            pbi_core::DiffChange::Added => Self::Added,
            pbi_core::DiffChange::Removed => Self::Removed,
            pbi_core::DiffChange::Changed => Self::Changed,
        }
    }
}

/// One property of a compared object, as serialized by `inspect --json`
/// style outputs; a side is absent where the property (or the object) is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PropertyDiffOut {
    /// camelCase property name (`expression`, `formatString`, `position`, …).
    pub name: String,
    /// The value in what the document is compared with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<serde_json::Value>,
    /// The value in the document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<serde_json::Value>,
}

/// One added, removed or changed object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ObjectDiffOut {
    pub kind: DiffKindOut,
    /// A readable path: `Orders`, `Orders[Total Sales]`,
    /// `Orders[Store ID] -> Stores[Store ID]`, `<report>/<page>`,
    /// `<report>/<page>/<visual name>`, …
    pub path: String,
    pub change: DiffChangeOut,
    /// The differing properties (changed), or all of them (added, removed;
    /// children included).
    pub properties: Vec<PropertyDiffOut>,
    /// Where the object is in what the document is compared with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<OriginOut>,
    /// Where the object is in the document's source files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<OriginOut>,
}

/// What a document was compared with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum DiffAgainst {
    /// The document's own files on disk (its saved state, plus any external
    /// changes).
    Disk,
    /// Another open document (`againstDocId`).
    Document,
    /// A file or folder (`againstPath`).
    Path,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocDiffOut {
    /// The document id.
    pub doc_id: String,
    /// The compared revision of the document.
    pub revision: u64,
    pub against: DiffAgainst,
    /// The other document's id or the path compared with (for `disk`, the
    /// document's path).
    pub against_name: String,
    /// No object differs.
    pub identical: bool,
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
    /// The objects that differ, in document order (model, tables with
    /// their columns, measures, hierarchies, partitions and calculation
    /// items, relationships, expressions, …, then reports with their
    /// measures, pages and visuals).
    pub objects: Vec<ObjectDiffOut>,
}

fn origin_out(o: pbi_model::Origin) -> OriginOut {
    OriginOut {
        path: o.path,
        line: o.line,
        pointer: o.pointer,
    }
}

impl Command for DocDiff {
    type Params = DocDiffParams;
    type Output = DocDiffOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "doc.diff",
            title: "Compare documents",
            summary: "Semantic diff of a document against its files on disk, another open document or a path: the objects added, removed and changed, with the differing properties.",
            category: Category::Workspace,
            examples: &[
                ("Unsaved and external changes of d1", r#"{"docId": "d1"}"#),
                (
                    "Against another open document",
                    r#"{"docId": "d2", "againstDocId": "d1"}"#,
                ),
                (
                    "Against another copy of the project",
                    r#"{"docId": "d1", "againstPath": "examples/acme-outfitters/AcmeOutfitters.pbip"}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &p.doc_id)?;
        let session = entry.session();
        let (revision, after) = {
            let snapshot = session.document_revision();
            (snapshot, session.document())
        };
        let (against, against_name, before) = match (&p.against_doc_id, &p.against_path) {
            (Some(_), Some(_)) => {
                return Err(OpError::invalid_params(
                    "pass at most one of againstDocId and againstPath",
                ));
            }
            (Some(other), None) => {
                let other = doc(ws, other)?;
                (
                    DiffAgainst::Document,
                    other.id().to_owned(),
                    other.session().document(),
                )
            }
            (None, Some(path)) => {
                let checked = ws.policy().check_read(path)?;
                let document = session.read_path(&checked).await.map_err(|e| match e {
                    SessionError::Open(e) => open_error(path, &checked, e),
                    e => OpError::from(e),
                })?;
                (
                    DiffAgainst::Path,
                    path.to_string_lossy().into_owned(),
                    Arc::new(document),
                )
            }
            (None, None) => {
                let path = entry.path();
                let document = session.read_path(&path).await.map_err(|e| match e {
                    SessionError::Open(e) => open_error(&path, &path, e)
                        .context(format!("cannot read {} from disk", entry.shown_path())),
                    e => OpError::from(e),
                })?;
                (DiffAgainst::Disk, entry.shown_path(), Arc::new(document))
            }
        };
        let diff =
            tokio::task::spawn_blocking(move || pbi_core::diff_documents(&before, &after)).await?;
        let added = diff.count(pbi_core::DiffChange::Added);
        let removed = diff.count(pbi_core::DiffChange::Removed);
        let changed = diff.count(pbi_core::DiffChange::Changed);
        Ok(DocDiffOut {
            doc_id: entry.id().to_owned(),
            revision,
            against,
            against_name,
            identical: diff.is_empty(),
            added,
            removed,
            changed,
            objects: diff
                .objects
                .into_iter()
                .map(|o| ObjectDiffOut {
                    kind: o.kind.into(),
                    path: o.path,
                    change: o.change.into(),
                    properties: o
                        .properties
                        .into_iter()
                        .map(|p| PropertyDiffOut {
                            name: p.name,
                            before: p.before,
                            after: p.after,
                        })
                        .collect(),
                    before: o.before.map(origin_out),
                    after: o.after.map(origin_out),
                })
                .collect(),
        })
    }
}
