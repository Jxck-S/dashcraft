//! `doc.detect`, `doc.open`, `doc.close`, `doc.list`.

use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use pbi_core::OpenOptions;
use pbi_core::container::Limits;
use pbi_model::Severity;
use pbi_session::{Clock, FixedClock, RefreshSource, Session, SessionOptions};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{BASE, DocParams, doc};
use crate::command::{Category, Command, CommandMeta};
use crate::ctx::Ctx;
use crate::detect_model::{DetectOut, detect_out};
use crate::error::OpError;
use crate::inspect_model::detection_out;
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
        let detection = tokio::task::spawn_blocking(move || {
            pbi_core::detect::detect_path(&path, &Limits::default())
        })
        .await?
        .map_err(|e| OpError::from(pbi_core::CoreError::from(e)))?;
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
    let model = session.base_model();
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
        path: entry.shown_path().to_owned(),
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
                .map_err(|e| {
                    OpError::from(e).context(format!("cannot open {}", p.path.display()))
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
#[serde(rename_all = "camelCase")]
pub struct DocCloseOut {
    /// The document id.
    pub doc_id: String,
    pub closed: bool,
}

impl Command for DocClose {
    type Params = DocParams;
    type Output = DocCloseOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "doc.close",
            title: "Close a document",
            summary: "Closes an open document and frees its data (running canvas renders are cancelled).",
            category: Category::Workspace,
            mutates_document: true,
            examples: &[("Close d1", r#"{"docId": "d1"}"#)],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        doc(ws, &p.doc_id)?;
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
