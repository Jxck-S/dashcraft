//! `doc.inspect`, `model.tables`, `model.measures`, `model.relationships`,
//! `report.list`, `report.pages`, `report.visuals`, `report.filters`.

use pbi_model::names_eq;
use pbi_session::{filters_from_page, filters_from_report};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{BASE, DocParams, doc};
use crate::command::{Category, Command, CommandMeta};
use crate::ctx::Ctx;
use crate::dto::FilterSpecOut;
use crate::error::OpError;
use crate::inspect_model::{
    self, BuildOptions, ColumnOut, DatasetReferenceOut, FilterOut, HierarchyOut, Inspect,
    PartitionOut, RelationshipOut, VisualOut,
};
use crate::resolve;
use crate::workspace::Workspace;

/// `doc.inspect`.
#[derive(Debug)]
pub struct DocInspect;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocInspectParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Include DAX, M and query text (default `true`); `false` gives an
    /// outline that is safe to share.
    #[serde(default)]
    pub include_expressions: Option<bool>,
}

impl Command for DocInspect {
    type Params = DocInspectParams;
    type Output = Inspect;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "doc.inspect",
            title: "Inspect a document",
            summary: "The full model, reports and reader diagnostics: the `dashcraft inspect --json` schema (schemaVersion 1).",
            category: Category::Inspect,
            examples: &[
                ("Everything", r#"{"docId": "d1"}"#),
                (
                    "Without DAX/M",
                    r#"{"docId": "d1", "includeExpressions": false}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &p.doc_id)?;
        let opts = BuildOptions {
            expressions: p.include_expressions.unwrap_or(true),
        };
        Ok(inspect_model::build(
            entry.shown_path(),
            entry.session().opened(),
            opts,
        ))
    }
}

// ---------------------------------------------------------------------------

/// `model.tables`.
#[derive(Debug)]
pub struct ModelTables;

/// A model table with its loaded data.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelTableOut {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub is_hidden: bool,
    /// `regular`, `calculatedTable`, `calculationGroup`, `remote`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_category: Option<String>,
    pub columns: Vec<ColumnOut>,
    /// Measure names (`model.measures` has their DAX).
    pub measures: Vec<String>,
    pub partitions: Vec<PartitionOut>,
    pub hierarchies: Vec<HierarchyOut>,
    /// Rows loaded (0 before a refresh).
    pub rows: usize,
    /// Data has been loaded.
    pub loaded: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelTablesOut {
    pub tables: Vec<ModelTableOut>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Include DAX and M text (default `true`).
    #[serde(default)]
    pub include_expressions: Option<bool>,
}

impl Command for ModelTables {
    type Params = ModelParams;
    type Output = ModelTablesOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "model.tables",
            title: "Model tables",
            summary: "Every model table with its columns, measure names, partitions, hierarchies and loaded row count.",
            category: Category::Inspect,
            examples: &[("Tables of d1", r#"{"docId": "d1"}"#)],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &p.doc_id)?;
        let session = entry.session();
        let opts = BuildOptions {
            expressions: p.include_expressions.unwrap_or(true),
        };
        let data = session.tables();
        let model = session.base_model();
        let tables = model
            .tables
            .iter()
            .map(|t| {
                let out = inspect_model::table_out(t, opts);
                let d = data.iter().find(|d| names_eq(&d.name, &t.name));
                ModelTableOut {
                    name: out.name,
                    description: out.description,
                    is_hidden: out.is_hidden,
                    kind: out.kind,
                    data_category: out.data_category,
                    columns: out.columns,
                    measures: out.measures.into_iter().map(|m| m.name).collect(),
                    partitions: out.partitions,
                    hierarchies: out.hierarchies,
                    rows: d.map_or(0, |d| d.rows),
                    loaded: d.is_some_and(|d| d.loaded),
                }
            })
            .collect();
        Ok(ModelTablesOut { tables })
    }
}

// ---------------------------------------------------------------------------

/// `model.measures`.
#[derive(Debug)]
pub struct ModelMeasures;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelMeasuresParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Only the measures of this table.
    #[serde(default)]
    pub table: Option<String>,
}

/// A measure definition.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeasureDefOut {
    pub name: String,
    /// Home table.
    pub table: String,
    /// DAX expression.
    pub expression: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_string: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_folder: Option<String>,
    pub is_hidden: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelMeasuresOut {
    pub measures: Vec<MeasureDefOut>,
}

impl Command for ModelMeasures {
    type Params = ModelMeasuresParams;
    type Output = ModelMeasuresOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "model.measures",
            title: "Model measures",
            summary: "Every measure with its table, DAX, format string, description and display folder.",
            category: Category::Inspect,
            examples: &[
                ("All measures", r#"{"docId": "d1"}"#),
                ("One table", r#"{"docId": "d1", "table": "Orders"}"#),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &p.doc_id)?;
        let model = entry.session().base_model();
        let only = p
            .table
            .as_deref()
            .map(|t| resolve::table_name(&model, t))
            .transpose()?;
        let measures = model
            .tables
            .iter()
            .filter(|t| only.as_deref().is_none_or(|o| names_eq(o, &t.name)))
            .flat_map(|t| {
                t.measures.iter().map(|m| MeasureDefOut {
                    name: m.name.clone(),
                    table: t.name.clone(),
                    expression: m.expression.clone(),
                    format_string: m.format_string.clone(),
                    description: m.description.clone(),
                    display_folder: m.display_folder.clone(),
                    is_hidden: m.is_hidden,
                })
            })
            .collect();
        Ok(ModelMeasuresOut { measures })
    }
}

// ---------------------------------------------------------------------------

/// `model.relationships`.
#[derive(Debug)]
pub struct ModelRelationships;

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelRelationshipsOut {
    pub relationships: Vec<RelationshipOut>,
}

impl Command for ModelRelationships {
    type Params = DocParams;
    type Output = ModelRelationshipsOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "model.relationships",
            title: "Model relationships",
            summary: "Every relationship: from/to columns, cardinalities, cross-filter direction, active flag.",
            category: Category::Inspect,
            examples: &[("Relationships of d1", r#"{"docId": "d1"}"#)],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &p.doc_id)?;
        let model = entry.session().base_model();
        let out = inspect_model::model_out(&model, None, BuildOptions { expressions: false });
        Ok(ModelRelationshipsOut {
            relationships: out.relationships,
        })
    }
}

// ---------------------------------------------------------------------------

/// `report.list`.
#[derive(Debug)]
pub struct ReportList;

/// A report of the document.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportInfoOut {
    /// 1-based position.
    pub index: usize,
    pub name: String,
    pub dataset_reference: DatasetReferenceOut,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_theme: Option<String>,
    /// Internal name of the page shown first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_page: Option<String>,
    pub pages: usize,
    pub visuals: usize,
    /// Report-level filters.
    pub filters: Vec<FilterOut>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportListOut {
    pub reports: Vec<ReportInfoOut>,
}

impl Command for ReportList {
    type Params = DocParams;
    type Output = ReportListOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "report.list",
            title: "Reports",
            summary: "The document's reports: name, dataset reference, theme, page and visual counts, report filters.",
            category: Category::Inspect,
            examples: &[("Reports of d1", r#"{"docId": "d1"}"#)],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &p.doc_id)?;
        let reports = entry
            .session()
            .document()
            .reports
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let out = inspect_model::report_out(r);
                ReportInfoOut {
                    index: i + 1,
                    visuals: out.pages.iter().map(|p| p.visuals.len()).sum(),
                    pages: out.pages.len(),
                    name: out.name,
                    dataset_reference: out.dataset_reference,
                    base_theme: out.base_theme,
                    custom_theme: out.custom_theme,
                    active_page: out.active_page,
                    filters: out.filters,
                }
            })
            .collect();
        Ok(ReportListOut { reports })
    }
}

// ---------------------------------------------------------------------------

/// `report.pages`.
#[derive(Debug)]
pub struct ReportPages;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Report name (exact, case-insensitive, or a unique part; or 1-based
    /// index). Default: the first report.
    #[serde(default)]
    pub report: Option<String>,
}

/// A page of a report.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PageInfoOut {
    /// 1-based position.
    pub index: usize,
    /// Internal name.
    pub name: String,
    pub display_name: String,
    pub width: f64,
    pub height: f64,
    pub is_hidden: bool,
    /// The report opens on this page.
    pub active: bool,
    pub visuals: usize,
    /// Page-level filters.
    pub filters: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportPagesOut {
    pub report: String,
    pub pages: Vec<PageInfoOut>,
}

impl Command for ReportPages {
    type Params = ReportParams;
    type Output = ReportPagesOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "report.pages",
            title: "Report pages",
            summary: "The pages of a report: names, size, visibility, visual and filter counts.",
            category: Category::Inspect,
            examples: &[
                ("Pages of the first report", r#"{"docId": "d1"}"#),
                (
                    "Pages of a report by partial name",
                    r#"{"docId": "d1", "report": "Stores"}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &p.doc_id)?;
        let session = entry.session();
        let index = resolve::report_index(session, p.report.as_deref())?;
        let report = resolve::report_at(session, index)?;
        let active = resolve::page_index(report, None).ok();
        let pages = report
            .pages
            .iter()
            .enumerate()
            .map(|(i, pg)| PageInfoOut {
                index: i + 1,
                name: pg.name.clone(),
                display_name: pg.display_name.clone(),
                width: pg.width,
                height: pg.height,
                is_hidden: pg.is_hidden,
                active: active == Some(i),
                visuals: pg.visuals.len(),
                filters: pg.filters.len(),
            })
            .collect();
        Ok(ReportPagesOut {
            report: report.name.clone(),
            pages,
        })
    }
}

// ---------------------------------------------------------------------------

/// `report.visuals`.
#[derive(Debug)]
pub struct ReportVisuals;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PageParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Report (default: the first).
    #[serde(default)]
    pub report: Option<String>,
    /// Page: internal name, display name, 1-based index or a unique part of
    /// the display name. Default: the active page.
    #[serde(default)]
    pub page: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportVisualsOut {
    pub report: String,
    /// Page internal name.
    pub page: String,
    pub page_display_name: String,
    /// Visuals in page order: type, position, bound fields, filters.
    pub visuals: Vec<VisualOut>,
}

impl Command for ReportVisuals {
    type Params = PageParams;
    type Output = ReportVisualsOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "report.visuals",
            title: "Page visuals",
            summary: "The visuals of a page: type, title, position, bound fields and visual filters.",
            category: Category::Inspect,
            examples: &[
                ("Active page of the first report", r#"{"docId": "d1"}"#),
                (
                    "A page by number",
                    r#"{"docId": "d1", "report": "Overview", "page": "1"}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &p.doc_id)?;
        let session = entry.session();
        let ri = resolve::report_index(session, p.report.as_deref())?;
        let report = resolve::report_at(session, ri)?;
        let pi = resolve::page_index(report, p.page.as_deref())?;
        let page = report
            .pages
            .get(pi)
            .ok_or_else(|| OpError::not_found("no such page"))?;
        let out = inspect_model::page_out(page);
        Ok(ReportVisualsOut {
            report: report.name.clone(),
            page: out.name,
            page_display_name: out.display_name,
            visuals: out.visuals,
        })
    }
}

// ---------------------------------------------------------------------------

/// `report.filters`.
#[derive(Debug)]
pub struct ReportFilters;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReportFiltersParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Report (default: the first).
    #[serde(default)]
    pub report: Option<String>,
    /// Also translate this page's filters: internal name, display name,
    /// 1-based index or a unique part of the display name. Default: report
    /// filters only.
    #[serde(default)]
    pub page: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportFiltersOut {
    pub report: String,
    /// Page internal name, when a page was given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    /// The filters as column filters (pass them to `measures.values` or
    /// `dax.query`), and those that cannot be translated.
    pub filters: FilterSpecOut,
}

impl Command for ReportFilters {
    type Params = ReportFiltersParams;
    type Output = ReportFiltersOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "report.filters",
            title: "Report filters",
            summary: "A report's filters (plus a page's) translated to column filters, with the ones that cannot be applied (Top N, relative date…).",
            category: Category::Inspect,
            examples: &[
                (
                    "Report-level filters",
                    r#"{"docId": "d1", "report": "Stores"}"#,
                ),
                (
                    "Report and page filters",
                    r#"{"docId": "d1", "report": "Stores", "page": "1"}"#,
                ),
            ],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        let entry = doc(ws, &p.doc_id)?;
        let session = entry.session();
        let ri = resolve::report_index(session, p.report.as_deref())?;
        let report = resolve::report_at(session, ri)?;
        let mut spec = filters_from_report(report);
        let mut page_name = None;
        if p.page.is_some() {
            let pi = resolve::page_index(report, p.page.as_deref())?;
            let page = report
                .pages
                .get(pi)
                .ok_or_else(|| OpError::not_found("no such page"))?;
            spec.extend(filters_from_page(page));
            page_name = Some(page.name.clone());
        }
        Ok(ReportFiltersOut {
            report: report.name.clone(),
            page: page_name,
            filters: FilterSpecOut::from(&spec),
        })
    }
}
