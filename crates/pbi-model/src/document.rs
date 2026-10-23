//! The opened document: source info, semantic model, reports, diagnostics,
//! and ID-based object lookup.

use std::collections::HashSet;
use std::ops::ControlFlow;

use serde::{Deserialize, Serialize};

use crate::ids::{IdGen, ObjectId};
use crate::macros::string_enum;
use crate::names::names_eq;
use crate::props::Origin;
use crate::query::Filter;
use crate::report::{Page, Report, Visual};
use crate::semantic::{
    Column, Culture, Database, Hierarchy, Level, Measure, Model, NamedExpression, Partition,
    Perspective, QueryGroup, Relationship, Role, Table, TablePermission,
};

string_enum! {
    /// Detected input format.
    pub enum SourceFormat {
        Pbip = "pbip",
        Pbix = "pbix",
        Pbit = "pbit",
        /// TMSL `model.bim`.
        Bim = "bim",
        /// Standalone TMDL `definition/` folder.
        TmdlFolder = "tmdlFolder" | "tmdl",
        /// `*.SemanticModel` folder.
        SemanticModelFolder = "semanticModelFolder",
        /// `*.Report` folder (PBIR).
        ReportFolder = "reportFolder" | "pbir",
        /// PBIR-Legacy single `report.json`.
        PbirLegacy = "pbirLegacy",
        Pbids = "pbids",
        Abf = "abf",
        Vpax = "vpax",
        Rdl = "rdl",
        /// Excel workbook with a Power Pivot model.
        PowerPivot = "powerPivot" | "xlsx",
        /// Power Query template (`.pqt`).
        PowerQueryTemplate = "pqt",
        /// Power Query M script (`.pq`).
        PowerQueryScript = "pq",
        /// Dataflow `model.json` (CDM).
        DataflowModel = "dataflowModel",
        Pbiviz = "pbiviz",
        /// Custom connector (`.mez`/`.pqx`).
        Connector = "connector" | "mez" | "pqx",
    }
}

impl Default for SourceFormat {
    fn default() -> Self {
        SourceFormat::Unknown(String::new())
    }
}

/// What was opened.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SourceInfo {
    pub format: SourceFormat,
    /// Path as given by the user.
    pub path: String,
    /// Container entries (ZIP entries / files under the folder), `/`-separated.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<String>,
}

/// Availability of the semantic model.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ModelStatus {
    /// `Document::model` holds the model.
    Local,
    /// The model lives in the service (`byConnection`), not available offline.
    Remote {
        #[serde(default)]
        description: String,
    },
    /// No model found.
    #[default]
    Missing,
}

/// Diagnostic severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Error,
    Warning,
    #[default]
    Info,
}

/// A reader/linter message.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Diagnostic {
    pub severity: Severity,
    /// Stable code, e.g. `PBI001`.
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    /// Object the message is about, for UI navigation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<ObjectId>,
}

impl Diagnostic {
    pub fn new(severity: Severity, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity,
            code: code.into(),
            message: message.into(),
            origin: None,
            object: None,
        }
    }

    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(Severity::Error, code, message)
    }

    pub fn warning(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, code, message)
    }

    pub fn info(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(Severity::Info, code, message)
    }

    pub fn with_origin(mut self, origin: Origin) -> Self {
        self.origin = Some(origin);
        self
    }

    pub fn with_object(mut self, id: ObjectId) -> Self {
        self.object = Some(id);
        self
    }
}

/// A container part the document does not interpret (themes, images,
/// `DataModel`, `SecurityBindings`, …), listed for the explorer.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PartInfo {
    pub path: String,
    /// Free-form kind chosen by the reader, e.g. `theme`, `image`, `dataModel`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// An opened Power BI artifact.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Document {
    pub ids: IdGen,
    pub source: SourceInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<Model>,
    pub model_status: ModelStatus,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reports: Vec<Report>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extra_parts: Vec<PartInfo>,
}

/// Kind of a model or report object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObjectKind {
    Model,
    Database,
    Table,
    Column,
    Measure,
    Partition,
    Hierarchy,
    Level,
    Relationship,
    Expression,
    QueryGroup,
    Role,
    TablePermission,
    Culture,
    Perspective,
    Report,
    Page,
    Visual,
    Filter,
}

/// Index path of an object inside a [`Document`]. Valid until the document
/// is structurally modified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectPath {
    Model,
    Database,
    Table {
        table: usize,
    },
    Column {
        table: usize,
        column: usize,
    },
    Measure {
        table: usize,
        measure: usize,
    },
    Partition {
        table: usize,
        partition: usize,
    },
    Hierarchy {
        table: usize,
        hierarchy: usize,
    },
    Level {
        table: usize,
        hierarchy: usize,
        level: usize,
    },
    Relationship {
        relationship: usize,
    },
    Expression {
        expression: usize,
    },
    QueryGroup {
        group: usize,
    },
    Role {
        role: usize,
    },
    TablePermission {
        role: usize,
        permission: usize,
    },
    Culture {
        culture: usize,
    },
    Perspective {
        perspective: usize,
    },
    Report {
        report: usize,
    },
    ReportFilter {
        report: usize,
        filter: usize,
    },
    Page {
        report: usize,
        page: usize,
    },
    PageFilter {
        report: usize,
        page: usize,
        filter: usize,
    },
    Visual {
        report: usize,
        page: usize,
        visual: usize,
    },
    VisualFilter {
        report: usize,
        page: usize,
        visual: usize,
        filter: usize,
    },
}

impl ObjectPath {
    pub fn kind(&self) -> ObjectKind {
        match self {
            ObjectPath::Model => ObjectKind::Model,
            ObjectPath::Database => ObjectKind::Database,
            ObjectPath::Table { .. } => ObjectKind::Table,
            ObjectPath::Column { .. } => ObjectKind::Column,
            ObjectPath::Measure { .. } => ObjectKind::Measure,
            ObjectPath::Partition { .. } => ObjectKind::Partition,
            ObjectPath::Hierarchy { .. } => ObjectKind::Hierarchy,
            ObjectPath::Level { .. } => ObjectKind::Level,
            ObjectPath::Relationship { .. } => ObjectKind::Relationship,
            ObjectPath::Expression { .. } => ObjectKind::Expression,
            ObjectPath::QueryGroup { .. } => ObjectKind::QueryGroup,
            ObjectPath::Role { .. } => ObjectKind::Role,
            ObjectPath::TablePermission { .. } => ObjectKind::TablePermission,
            ObjectPath::Culture { .. } => ObjectKind::Culture,
            ObjectPath::Perspective { .. } => ObjectKind::Perspective,
            ObjectPath::Report { .. } => ObjectKind::Report,
            ObjectPath::Page { .. } => ObjectKind::Page,
            ObjectPath::Visual { .. } => ObjectKind::Visual,
            ObjectPath::ReportFilter { .. }
            | ObjectPath::PageFilter { .. }
            | ObjectPath::VisualFilter { .. } => ObjectKind::Filter,
        }
    }
}

/// Borrowed view of an object found by ID, with its parents.
#[derive(Debug, Clone, Copy)]
pub enum ObjectRef<'a> {
    Model(&'a Model),
    Database(&'a Database),
    Table(&'a Table),
    Column(&'a Table, &'a Column),
    Measure(&'a Table, &'a Measure),
    Partition(&'a Table, &'a Partition),
    Hierarchy(&'a Table, &'a Hierarchy),
    Level(&'a Table, &'a Hierarchy, &'a Level),
    Relationship(&'a Relationship),
    Expression(&'a NamedExpression),
    QueryGroup(&'a QueryGroup),
    Role(&'a Role),
    TablePermission(&'a Role, &'a TablePermission),
    Culture(&'a Culture),
    Perspective(&'a Perspective),
    Report(&'a Report),
    Page(&'a Report, &'a Page),
    Visual(&'a Report, &'a Page, &'a Visual),
    /// A filter; `page`/`visual` are set for page/visual-level filters.
    Filter {
        report: &'a Report,
        page: Option<&'a Page>,
        visual: Option<&'a Visual>,
        filter: &'a Filter,
    },
}

impl ObjectRef<'_> {
    pub fn kind(&self) -> ObjectKind {
        match self {
            ObjectRef::Model(_) => ObjectKind::Model,
            ObjectRef::Database(_) => ObjectKind::Database,
            ObjectRef::Table(_) => ObjectKind::Table,
            ObjectRef::Column(..) => ObjectKind::Column,
            ObjectRef::Measure(..) => ObjectKind::Measure,
            ObjectRef::Partition(..) => ObjectKind::Partition,
            ObjectRef::Hierarchy(..) => ObjectKind::Hierarchy,
            ObjectRef::Level(..) => ObjectKind::Level,
            ObjectRef::Relationship(_) => ObjectKind::Relationship,
            ObjectRef::Expression(_) => ObjectKind::Expression,
            ObjectRef::QueryGroup(_) => ObjectKind::QueryGroup,
            ObjectRef::Role(_) => ObjectKind::Role,
            ObjectRef::TablePermission(..) => ObjectKind::TablePermission,
            ObjectRef::Culture(_) => ObjectKind::Culture,
            ObjectRef::Perspective(_) => ObjectKind::Perspective,
            ObjectRef::Report(_) => ObjectKind::Report,
            ObjectRef::Page(..) => ObjectKind::Page,
            ObjectRef::Visual(..) => ObjectKind::Visual,
            ObjectRef::Filter { .. } => ObjectKind::Filter,
        }
    }

    /// Object name (query-group folder, permission table, database name or
    /// empty string where an object has no name).
    pub fn name(&self) -> &str {
        match self {
            ObjectRef::Model(m) => &m.name,
            ObjectRef::Database(d) => d.name.as_deref().unwrap_or_default(),
            ObjectRef::Table(t) => &t.name,
            ObjectRef::Column(_, c) => &c.name,
            ObjectRef::Measure(_, m) => &m.name,
            ObjectRef::Partition(_, p) => &p.name,
            ObjectRef::Hierarchy(_, h) => &h.name,
            ObjectRef::Level(_, _, l) => &l.name,
            ObjectRef::Relationship(r) => &r.name,
            ObjectRef::Expression(e) => &e.name,
            ObjectRef::QueryGroup(g) => &g.folder,
            ObjectRef::Role(r) => &r.name,
            ObjectRef::TablePermission(_, p) => &p.table,
            ObjectRef::Culture(c) => &c.name,
            ObjectRef::Perspective(p) => &p.name,
            ObjectRef::Report(r) => &r.name,
            ObjectRef::Page(_, p) => &p.name,
            ObjectRef::Visual(_, _, v) => &v.name,
            ObjectRef::Filter { filter, .. } => &filter.name,
        }
    }

    /// Description, for objects that have one.
    pub fn description(&self) -> Option<&str> {
        match self {
            ObjectRef::Model(m) => m.description.as_deref(),
            ObjectRef::Database(d) => d.description.as_deref(),
            ObjectRef::Table(t) => t.description.as_deref(),
            ObjectRef::Column(_, c) => c.description.as_deref(),
            ObjectRef::Measure(_, m) => m.description.as_deref(),
            ObjectRef::Partition(_, p) => p.description.as_deref(),
            ObjectRef::Hierarchy(_, h) => h.description.as_deref(),
            ObjectRef::Level(_, _, l) => l.description.as_deref(),
            ObjectRef::Expression(e) => e.description.as_deref(),
            ObjectRef::QueryGroup(g) => g.description.as_deref(),
            ObjectRef::Role(r) => r.description.as_deref(),
            ObjectRef::Perspective(p) => p.description.as_deref(),
            _ => None,
        }
    }
}

impl Document {
    pub fn new(source: SourceInfo) -> Self {
        Self {
            source,
            ..Self::default()
        }
    }

    /// Report by name (case-insensitive).
    pub fn report(&self, name: &str) -> Option<&Report> {
        self.reports.iter().find(|r| names_eq(&r.name, name))
    }

    pub fn push_diagnostic(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }

    /// Gives every object without an ID (or with a duplicate ID) a fresh
    /// one, and advances [`Document::ids`] past every ID in use. Readers call
    /// this once after building the document; it is idempotent.
    pub fn assign_ids(&mut self) {
        let mut all = Vec::new();
        self.walk(&mut |id, path| {
            all.push((id, path));
            ControlFlow::Continue(())
        });
        if let Some(max) = all.iter().map(|(id, _)| *id).max() {
            self.ids.reserve_through(max);
        }
        let mut seen = HashSet::with_capacity(all.len());
        for (id, path) in all {
            if id.is_assigned() && seen.insert(id) {
                continue;
            }
            let fresh = self.ids.next_id();
            seen.insert(fresh);
            if let Some(slot) = self.id_mut(path) {
                *slot = fresh;
            }
        }
    }

    /// Finds the path of the object with `id`.
    pub fn locate(&self, id: ObjectId) -> Option<ObjectPath> {
        if !id.is_assigned() {
            return None;
        }
        let mut found = None;
        self.walk(&mut |candidate, path| {
            if candidate == id {
                found = Some(path);
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        found
    }

    /// Finds the object with `id`.
    pub fn find(&self, id: ObjectId) -> Option<ObjectRef<'_>> {
        self.get(self.locate(id)?)
    }

    /// Resolves a path to the object it points at.
    pub fn get(&self, path: ObjectPath) -> Option<ObjectRef<'_>> {
        let model = || self.model.as_ref();
        let table = |t: usize| model()?.tables.get(t);
        let report = |r: usize| self.reports.get(r);
        let page = |r: usize, p: usize| Some((report(r)?, report(r)?.pages.get(p)?));
        Some(match path {
            ObjectPath::Model => ObjectRef::Model(model()?),
            ObjectPath::Database => ObjectRef::Database(model()?.database.as_ref()?),
            ObjectPath::Table { table: t } => ObjectRef::Table(table(t)?),
            ObjectPath::Column { table: t, column } => {
                let tb = table(t)?;
                ObjectRef::Column(tb, tb.columns.get(column)?)
            }
            ObjectPath::Measure { table: t, measure } => {
                let tb = table(t)?;
                ObjectRef::Measure(tb, tb.measures.get(measure)?)
            }
            ObjectPath::Partition {
                table: t,
                partition,
            } => {
                let tb = table(t)?;
                ObjectRef::Partition(tb, tb.partitions.get(partition)?)
            }
            ObjectPath::Hierarchy {
                table: t,
                hierarchy,
            } => {
                let tb = table(t)?;
                ObjectRef::Hierarchy(tb, tb.hierarchies.get(hierarchy)?)
            }
            ObjectPath::Level {
                table: t,
                hierarchy,
                level,
            } => {
                let tb = table(t)?;
                let h = tb.hierarchies.get(hierarchy)?;
                ObjectRef::Level(tb, h, h.levels.get(level)?)
            }
            ObjectPath::Relationship { relationship } => {
                ObjectRef::Relationship(model()?.relationships.get(relationship)?)
            }
            ObjectPath::Expression { expression } => {
                ObjectRef::Expression(model()?.expressions.get(expression)?)
            }
            ObjectPath::QueryGroup { group } => {
                ObjectRef::QueryGroup(model()?.query_groups.get(group)?)
            }
            ObjectPath::Role { role } => ObjectRef::Role(model()?.roles.get(role)?),
            ObjectPath::TablePermission { role, permission } => {
                let r = model()?.roles.get(role)?;
                ObjectRef::TablePermission(r, r.table_permissions.get(permission)?)
            }
            ObjectPath::Culture { culture } => ObjectRef::Culture(model()?.cultures.get(culture)?),
            ObjectPath::Perspective { perspective } => {
                ObjectRef::Perspective(model()?.perspectives.get(perspective)?)
            }
            ObjectPath::Report { report: r } => ObjectRef::Report(report(r)?),
            ObjectPath::ReportFilter { report: r, filter } => {
                let rp = report(r)?;
                ObjectRef::Filter {
                    report: rp,
                    page: None,
                    visual: None,
                    filter: rp.filters.get(filter)?,
                }
            }
            ObjectPath::Page { report: r, page: p } => {
                let (rp, pg) = page(r, p)?;
                ObjectRef::Page(rp, pg)
            }
            ObjectPath::PageFilter {
                report: r,
                page: p,
                filter,
            } => {
                let (rp, pg) = page(r, p)?;
                ObjectRef::Filter {
                    report: rp,
                    page: Some(pg),
                    visual: None,
                    filter: pg.filters.get(filter)?,
                }
            }
            ObjectPath::Visual {
                report: r,
                page: p,
                visual,
            } => {
                let (rp, pg) = page(r, p)?;
                ObjectRef::Visual(rp, pg, pg.visuals.get(visual)?)
            }
            ObjectPath::VisualFilter {
                report: r,
                page: p,
                visual,
                filter,
            } => {
                let (rp, pg) = page(r, p)?;
                let v = pg.visuals.get(visual)?;
                ObjectRef::Filter {
                    report: rp,
                    page: Some(pg),
                    visual: Some(v),
                    filter: v.filters.get(filter)?,
                }
            }
        })
    }

    /// Mutable access to the ID slot at `path`.
    fn id_mut(&mut self, path: ObjectPath) -> Option<&mut ObjectId> {
        if let ObjectPath::Report { .. }
        | ObjectPath::ReportFilter { .. }
        | ObjectPath::Page { .. }
        | ObjectPath::PageFilter { .. }
        | ObjectPath::Visual { .. }
        | ObjectPath::VisualFilter { .. } = path
        {
            return self.report_id_mut(path);
        }
        let model = self.model.as_mut()?;
        Some(match path {
            ObjectPath::Model => &mut model.id,
            ObjectPath::Database => &mut model.database.as_mut()?.id,
            ObjectPath::Table { table } => &mut model.tables.get_mut(table)?.id,
            ObjectPath::Column { table, column } => {
                &mut model.tables.get_mut(table)?.columns.get_mut(column)?.id
            }
            ObjectPath::Measure { table, measure } => {
                &mut model.tables.get_mut(table)?.measures.get_mut(measure)?.id
            }
            ObjectPath::Partition { table, partition } => {
                &mut model
                    .tables
                    .get_mut(table)?
                    .partitions
                    .get_mut(partition)?
                    .id
            }
            ObjectPath::Hierarchy { table, hierarchy } => {
                &mut model
                    .tables
                    .get_mut(table)?
                    .hierarchies
                    .get_mut(hierarchy)?
                    .id
            }
            ObjectPath::Level {
                table,
                hierarchy,
                level,
            } => {
                &mut model
                    .tables
                    .get_mut(table)?
                    .hierarchies
                    .get_mut(hierarchy)?
                    .levels
                    .get_mut(level)?
                    .id
            }
            ObjectPath::Relationship { relationship } => {
                &mut model.relationships.get_mut(relationship)?.id
            }
            ObjectPath::Expression { expression } => &mut model.expressions.get_mut(expression)?.id,
            ObjectPath::QueryGroup { group } => &mut model.query_groups.get_mut(group)?.id,
            ObjectPath::Role { role } => &mut model.roles.get_mut(role)?.id,
            ObjectPath::TablePermission { role, permission } => {
                &mut model
                    .roles
                    .get_mut(role)?
                    .table_permissions
                    .get_mut(permission)?
                    .id
            }
            ObjectPath::Culture { culture } => &mut model.cultures.get_mut(culture)?.id,
            ObjectPath::Perspective { perspective } => {
                &mut model.perspectives.get_mut(perspective)?.id
            }
            _ => return None,
        })
    }

    fn report_id_mut(&mut self, path: ObjectPath) -> Option<&mut ObjectId> {
        Some(match path {
            ObjectPath::Report { report } => &mut self.reports.get_mut(report)?.id,
            ObjectPath::ReportFilter { report, filter } => {
                &mut self.reports.get_mut(report)?.filters.get_mut(filter)?.id
            }
            ObjectPath::Page { report, page } => {
                &mut self.reports.get_mut(report)?.pages.get_mut(page)?.id
            }
            ObjectPath::PageFilter {
                report,
                page,
                filter,
            } => {
                &mut self
                    .reports
                    .get_mut(report)?
                    .pages
                    .get_mut(page)?
                    .filters
                    .get_mut(filter)?
                    .id
            }
            ObjectPath::Visual {
                report,
                page,
                visual,
            } => {
                &mut self
                    .reports
                    .get_mut(report)?
                    .pages
                    .get_mut(page)?
                    .visuals
                    .get_mut(visual)?
                    .id
            }
            ObjectPath::VisualFilter {
                report,
                page,
                visual,
                filter,
            } => {
                &mut self
                    .reports
                    .get_mut(report)?
                    .pages
                    .get_mut(page)?
                    .visuals
                    .get_mut(visual)?
                    .filters
                    .get_mut(filter)?
                    .id
            }
            _ => return None,
        })
    }

    /// Visits every object's ID and path in a deterministic order (model
    /// first, then reports). Stops early on `ControlFlow::Break`.
    pub fn walk(&self, f: &mut dyn FnMut(ObjectId, ObjectPath) -> ControlFlow<()>) {
        let _ = self.walk_inner(f);
    }

    fn walk_inner(
        &self,
        f: &mut dyn FnMut(ObjectId, ObjectPath) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        if let Some(m) = &self.model {
            f(m.id, ObjectPath::Model)?;
            if let Some(db) = &m.database {
                f(db.id, ObjectPath::Database)?;
            }
            for (ti, t) in m.tables.iter().enumerate() {
                f(t.id, ObjectPath::Table { table: ti })?;
                for (i, c) in t.columns.iter().enumerate() {
                    f(
                        c.id,
                        ObjectPath::Column {
                            table: ti,
                            column: i,
                        },
                    )?;
                }
                for (i, x) in t.measures.iter().enumerate() {
                    f(
                        x.id,
                        ObjectPath::Measure {
                            table: ti,
                            measure: i,
                        },
                    )?;
                }
                for (i, p) in t.partitions.iter().enumerate() {
                    f(
                        p.id,
                        ObjectPath::Partition {
                            table: ti,
                            partition: i,
                        },
                    )?;
                }
                for (hi, h) in t.hierarchies.iter().enumerate() {
                    f(
                        h.id,
                        ObjectPath::Hierarchy {
                            table: ti,
                            hierarchy: hi,
                        },
                    )?;
                    for (li, l) in h.levels.iter().enumerate() {
                        f(
                            l.id,
                            ObjectPath::Level {
                                table: ti,
                                hierarchy: hi,
                                level: li,
                            },
                        )?;
                    }
                }
            }
            for (i, r) in m.relationships.iter().enumerate() {
                f(r.id, ObjectPath::Relationship { relationship: i })?;
            }
            for (i, e) in m.expressions.iter().enumerate() {
                f(e.id, ObjectPath::Expression { expression: i })?;
            }
            for (i, g) in m.query_groups.iter().enumerate() {
                f(g.id, ObjectPath::QueryGroup { group: i })?;
            }
            for (ri, r) in m.roles.iter().enumerate() {
                f(r.id, ObjectPath::Role { role: ri })?;
                for (i, p) in r.table_permissions.iter().enumerate() {
                    f(
                        p.id,
                        ObjectPath::TablePermission {
                            role: ri,
                            permission: i,
                        },
                    )?;
                }
            }
            for (i, c) in m.cultures.iter().enumerate() {
                f(c.id, ObjectPath::Culture { culture: i })?;
            }
            for (i, p) in m.perspectives.iter().enumerate() {
                f(p.id, ObjectPath::Perspective { perspective: i })?;
            }
        }
        for (ri, r) in self.reports.iter().enumerate() {
            f(r.id, ObjectPath::Report { report: ri })?;
            for (i, x) in r.filters.iter().enumerate() {
                f(
                    x.id,
                    ObjectPath::ReportFilter {
                        report: ri,
                        filter: i,
                    },
                )?;
            }
            for (pi, p) in r.pages.iter().enumerate() {
                f(
                    p.id,
                    ObjectPath::Page {
                        report: ri,
                        page: pi,
                    },
                )?;
                for (i, x) in p.filters.iter().enumerate() {
                    f(
                        x.id,
                        ObjectPath::PageFilter {
                            report: ri,
                            page: pi,
                            filter: i,
                        },
                    )?;
                }
                for (vi, v) in p.visuals.iter().enumerate() {
                    f(
                        v.id,
                        ObjectPath::Visual {
                            report: ri,
                            page: pi,
                            visual: vi,
                        },
                    )?;
                    for (i, x) in v.filters.iter().enumerate() {
                        f(
                            x.id,
                            ObjectPath::VisualFilter {
                                report: ri,
                                page: pi,
                                visual: vi,
                                filter: i,
                            },
                        )?;
                    }
                }
            }
        }
        ControlFlow::Continue(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::sample_document;
    use pretty_assertions::assert_eq;

    #[test]
    fn assign_ids_gives_unique_ids_and_is_idempotent() {
        let mut doc = sample_document();
        let mut ids = Vec::new();
        doc.walk(&mut |id, _| {
            ids.push(id);
            ControlFlow::Continue(())
        });
        assert!(ids.iter().all(|id| id.is_assigned()));
        let unique: HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());

        let before = doc.clone();
        doc.assign_ids();
        assert_eq!(doc, before);
    }

    #[test]
    fn assign_ids_repairs_duplicates_and_keeps_generator_ahead() {
        let mut doc = sample_document();
        let first_visual = doc.reports[0].pages[0].visuals[0].id;
        doc.reports[0].pages[0].visuals[1].id = first_visual;
        doc.reports[0].pages[0].visuals[0].filters.clear();
        doc.ids = IdGen::new();
        doc.assign_ids();
        let v = &doc.reports[0].pages[0].visuals;
        assert_eq!(v[0].id, first_visual);
        assert_ne!(v[1].id, first_visual);
        let next = doc.ids.peek();
        doc.walk(&mut |id, _| {
            assert!(id < next);
            ControlFlow::Continue(())
        });
    }

    #[test]
    fn find_resolves_ids_to_objects_with_parents() {
        let doc = sample_document();
        let model = doc.model.as_ref().unwrap();
        let col = &model.table("Stores").unwrap().columns[1];
        match doc.find(col.id) {
            Some(ObjectRef::Column(t, c)) => {
                assert_eq!(t.name, "Stores");
                assert_eq!(c.name, col.name);
            }
            other => panic!("unexpected {other:?}"),
        }
        let visual = &doc.reports[0].pages[0].visuals[0];
        let found = doc.find(visual.id).unwrap();
        assert_eq!(found.kind(), ObjectKind::Visual);
        assert_eq!(found.name(), visual.name);
        let filter = &doc.reports[0].filters[0];
        assert_eq!(doc.locate(filter.id).unwrap().kind(), ObjectKind::Filter);
        assert!(doc.find(ObjectId(999_999)).is_none());
        assert!(doc.find(ObjectId::UNASSIGNED).is_none());
        assert_eq!(
            doc.find(model.tables[0].id).unwrap().description(),
            model.tables[0].description.as_deref()
        );
    }

    #[test]
    fn document_round_trips_through_json() {
        let doc = sample_document();
        let json = serde_json::to_string_pretty(&doc).unwrap();
        let back: Document = serde_json::from_str(&json).unwrap();
        assert_eq!(back, doc);
        // Compact form: defaults are omitted, IDs and names are present.
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let rel = &value["model"]["relationships"][0];
        assert_eq!(rel["crossFilteringBehavior"], "bothDirections");
        assert!(rel.get("isActive").is_none());
        assert_eq!(
            value["reports"][0]["datasetReference"]["kind"],
            "byConnection"
        );
        assert_eq!(value["modelStatus"]["status"], "local");
    }

    #[test]
    fn diagnostics_helpers() {
        let mut doc = Document::new(SourceInfo {
            format: SourceFormat::parse("PBIP"),
            path: "AcmeOutfitters.pbip".into(),
            entries: Vec::new(),
        });
        assert_eq!(doc.source.format, SourceFormat::Pbip);
        assert!(!doc.has_errors());
        doc.push_diagnostic(Diagnostic::warning("PBI002", "stale queryRef"));
        assert!(!doc.has_errors());
        doc.push_diagnostic(
            Diagnostic::error("PBI001", "missing field")
                .with_origin(Origin::file("definition/report.json"))
                .with_object(ObjectId(3)),
        );
        assert!(doc.has_errors());
    }
}
