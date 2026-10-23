//! The v1 command catalog, one module per category. Each command is a unit
//! struct implementing [`Command`](crate::Command); [`catalog`] registers
//! them in catalog order.

pub mod canvas;
pub mod data;
pub mod dax;
pub mod export;
pub mod inspect;
pub mod lint;
pub mod meta;
pub mod render;
pub mod security;
pub mod transform;
pub mod workspace;

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::command::{Category, CommandMeta};
use crate::ctx::Ctx;
use crate::error::OpError;
use crate::registry::Registry;
use crate::workspace::{DocEntry, Workspace};

/// Version that introduced the v1 catalog.
pub const SINCE: &str = "0.2.0";

/// Defaults of [`CommandMeta`]: agent-safe, read-only, quick.
pub(crate) const BASE: CommandMeta = CommandMeta {
    id: "",
    title: "",
    summary: "",
    category: Category::Meta,
    mutates_document: false,
    long_running: false,
    agent_safe: true,
    writes_files: false,
    invokes_other_commands: false,
    since: SINCE,
    examples: &[],
};

/// The v1 catalog.
#[must_use]
pub fn catalog() -> Registry {
    Registry::new()
        // meta
        .with::<meta::CommandsList>()
        .with::<meta::CommandsDescribe>()
        .with::<meta::CommandsBatch>()
        // workspace
        .with::<workspace::DocDetect>()
        .with::<workspace::DocOpen>()
        .with::<workspace::DocClose>()
        .with::<workspace::DocList>()
        // inspect
        .with::<inspect::DocInspect>()
        .with::<inspect::ModelTables>()
        .with::<inspect::ModelMeasures>()
        .with::<inspect::ModelRelationships>()
        .with::<inspect::ReportList>()
        .with::<inspect::ReportPages>()
        .with::<inspect::ReportVisuals>()
        .with::<inspect::ReportFilters>()
        .with::<security::SecurityRoles>()
        // lint
        .with::<lint::DocLint>()
        // data
        .with::<data::DataRefresh>()
        .with::<data::DataTables>()
        .with::<data::DataPage>()
        .with::<data::DataColumnStats>()
        .with::<data::DataLoadCsv>()
        .with::<data::DataAddDerivedColumn>()
        .with::<data::DataRemoveDerivedColumn>()
        .with::<security::SecurityViewAs>()
        // dax
        .with::<dax::DaxQuery>()
        .with::<dax::DaxEvaluateMeasure>()
        .with::<dax::MeasuresValues>()
        // render
        .with::<render::RenderPage>()
        .with::<render::RenderVisual>()
        // canvas
        .with::<canvas::CanvasRender>()
        .with::<canvas::CanvasClick>()
        .with::<canvas::CanvasScroll>()
        .with::<canvas::CanvasSetSlicer>()
        .with::<canvas::CanvasClearSelections>()
        .with::<canvas::CanvasSetSlicerSearch>()
        .with::<canvas::CanvasToggleMatrixRow>()
        .with::<canvas::CanvasClosePopups>()
        .with::<canvas::CanvasReset>()
        .with::<canvas::CanvasTooltip>()
        .with::<canvas::CanvasState>()
        // export / convert
        .with::<export::ExportTable>()
        .with::<export::ExportModel>()
        .with::<export::ConvertPbip>()
        .with::<export::ConvertPbit>()
        // transform
        .with::<transform::TextParsePeople>()
        .with::<transform::TextPreviewPeople>()
}

/// Parameters of commands that only need a document.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocParams {
    /// The document id returned by `doc.open` (`d1`, …).
    pub doc_id: String,
}

/// The document `id` of `ws`.
pub(crate) fn doc(ws: &Workspace, id: &str) -> Result<Arc<DocEntry>, OpError> {
    ws.get(id)
}

/// The document, its data loaded (automatic first refresh).
pub(crate) async fn loaded_doc(
    ws: &Workspace,
    id: &str,
    ctx: &Ctx,
) -> Result<Arc<DocEntry>, OpError> {
    let entry = ws.get(id)?;
    entry.ensure_loaded(ctx).await?;
    Ok(entry)
}

/// Rejects limits above `max` with a helpful message.
pub(crate) fn bounded(name: &str, value: usize, max: usize) -> Result<usize, OpError> {
    if value > max {
        Err(OpError::invalid_params(format!(
            "{name} is {value}; the maximum is {max}"
        )))
    } else {
        Ok(value)
    }
}

/// Milliseconds since `start`.
pub(crate) fn millis(start: std::time::Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}
