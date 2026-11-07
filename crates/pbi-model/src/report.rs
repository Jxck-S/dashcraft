//! Format-agnostic report AST (PBIR, PBIR-Legacy and PBIX `Layout` all map
//! onto these types).

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::ids::ObjectId;
use crate::macros::string_enum;
use crate::names::names_eq;
use crate::props::{Annotation, Origin, PropValue, PropertyBag};
use crate::query::{FieldExpr, Filter, SortItem};
use crate::serde_util::{is_false, is_null};

/// A report.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Report {
    pub id: ObjectId,
    /// Display name (from `.platform` or the folder/file name).
    pub name: String,
    /// `$schema` URL of `report.json`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub theme: ThemeRef,
    /// Report-level filters (apply to every page).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<Filter>,
    /// Report-level measures (PBIR `definition/reportExtensions.json`), in
    /// file order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub measures: Vec<ReportMeasure>,
    /// Pages in display order (`pages.json` `pageOrder`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pages: Vec<Page>,
    /// Name of the page shown on open.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_page: Option<String>,
    #[serde(skip_serializing_if = "is_null")]
    pub settings: serde_json::Value,
    /// Report `objects` (outspace pane, section defaults, …), verbatim.
    #[serde(skip_serializing_if = "is_null")]
    pub objects: serde_json::Value,
    pub dataset_reference: DatasetReference,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub resource_packages: Vec<ResourcePackage>,
    /// Schema/format versions by file kind, e.g. `report` → `3.3.0`,
    /// `definition.pbir` → `4.0`.
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub schema_versions: IndexMap<String, String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

impl Report {
    /// Page by internal name (exact match; PBIR page names are IDs).
    pub fn page(&self, name: &str) -> Option<&Page> {
        self.pages.iter().find(|p| p.name == name)
    }

    /// Page by display name (case-insensitive).
    pub fn page_by_display_name(&self, display_name: &str) -> Option<&Page> {
        self.pages
            .iter()
            .find(|p| names_eq(&p.display_name, display_name))
    }

    /// Page by internal name, falling back to display name.
    pub fn find_page(&self, name_or_display_name: &str) -> Option<&Page> {
        self.page(name_or_display_name)
            .or_else(|| self.page_by_display_name(name_or_display_name))
    }

    /// The page shown on open: `active_page` if it exists, else the first
    /// visible page, else the first page.
    pub fn start_page(&self) -> Option<&Page> {
        self.active_page
            .as_deref()
            .and_then(|n| self.page(n))
            .or_else(|| self.pages.iter().find(|p| !p.is_hidden))
            .or_else(|| self.pages.first())
    }

    /// Every visual on every page.
    pub fn visuals(&self) -> impl Iterator<Item = (&Page, &Visual)> {
        self.pages
            .iter()
            .flat_map(|p| p.visuals.iter().map(move |v| (p, v)))
    }

    /// Resolves a theme reference to `StaticResources/<package>/<item path>`.
    pub fn static_resource_path(&self, resource: &ResourceRef) -> Option<String> {
        self.resource_packages
            .iter()
            .filter(|p| match (&resource.kind, &p.kind) {
                (Some(want), Some(have)) => names_eq(want, have),
                _ => true,
            })
            .find_map(|p| {
                p.items
                    .iter()
                    .find(|i| names_eq(&i.name, &resource.name))
                    .map(|i| format!("StaticResources/{}/{}", p.name, i.path))
            })
    }

    /// Visits every field expression in report filters, page filters and
    /// visuals.
    pub fn visit_fields(&self, f: &mut dyn FnMut(&FieldExpr)) {
        self.filters.iter().for_each(|x| x.visit_fields(f));
        self.pages.iter().for_each(|p| p.visit_fields(f));
    }

    /// Mutable variant of [`Report::visit_fields`].
    pub fn visit_fields_mut(&mut self, f: &mut dyn FnMut(&mut FieldExpr)) {
        self.filters.iter_mut().for_each(|x| x.visit_fields_mut(f));
        self.pages.iter_mut().for_each(|p| p.visit_fields_mut(f));
    }

    /// Report-level measure by name (case-insensitive; the names are unique
    /// across the model and the report's measures).
    pub fn measure(&self, name: &str) -> Option<&ReportMeasure> {
        self.measures.iter().find(|m| names_eq(&m.name, name))
    }
}

/// A report-level measure: a DAX measure defined in the report (PBIR
/// `definition/reportExtensions.json`) and shown in a model table. Visuals
/// refer to it with the extension's schema (`SourceRef.Schema`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ReportMeasure {
    /// The model table it is shown in (`entities[].name`).
    pub table: String,
    pub name: String,
    /// DAX.
    pub expression: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_string: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub hidden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_folder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The measures its expression uses (`references.measures`), in file
    /// order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<MeasureReference>,
}

/// A measure a report-level measure uses.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MeasureReference {
    /// Empty for a model measure; the extension's name for another
    /// report-level measure.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub schema: String,
    pub table: String,
    pub name: String,
}

/// Base and custom theme of a report.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThemeRef {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_theme: Option<ResourceRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_theme: Option<ResourceRef>,
}

/// Reference to a resource-package item (e.g. a theme).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ResourceRef {
    pub name: String,
    /// Package type: `SharedResources`, `RegisteredResources`, …
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// E.g. `reportVersionAtImport`.
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
}

/// A resource package (`resourcePackages[]`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ResourcePackage {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ResourceItem>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
}

/// One item of a [`ResourcePackage`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ResourceItem {
    pub name: String,
    /// Path relative to the package folder, e.g. `BaseThemes/CY24SU10.json`.
    pub path: String,
    /// `BaseTheme`, `CustomTheme`, `Image`, …
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
}

/// Which semantic model a report is bound to (`definition.pbir`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum DatasetReference {
    /// Local model folder, relative to the report folder.
    ByPath {
        #[serde(default)]
        path: String,
    },
    /// Service-hosted model ("remote, not available offline").
    ByConnection {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        connection_string: Option<String>,
        /// `pbiServiceModelId`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_id: Option<String>,
        /// `pbiModelDatabaseName`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        connection_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// The full `byConnection` object, verbatim.
        #[serde(default, skip_serializing_if = "is_null")]
        raw: serde_json::Value,
    },
    /// Model embedded in the same file (PBIX `DataModel`, PBIT schema).
    Embedded,
    /// No model reference.
    #[default]
    None,
}

impl DatasetReference {
    pub fn is_remote(&self) -> bool {
        matches!(self, DatasetReference::ByConnection { .. })
    }
}

string_enum! {
    /// Page scaling (`displayOption`).
    #[derive(Default)]
    pub enum DisplayOption {
        #[default]
        FitToPage = "FitToPage",
        FitToWidth = "FitToWidth",
        ActualSize = "ActualSize",
    }
}

string_enum! {
    /// How a source visual affects a target visual.
    #[derive(Default)]
    pub enum InteractionKind {
        #[default]
        DataFilter = "DataFilter" | "filter",
        HighlightFilter = "HighlightFilter" | "highlight",
        NoFilter = "NoFilter" | "none",
    }
}

/// A `visualInteractions` entry (visual names, not IDs).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VisualInteraction {
    pub source: String,
    pub target: String,
    pub kind: InteractionKind,
}

/// A report page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Page {
    pub id: ObjectId,
    /// Internal name (PBIR folder name / ID).
    pub name: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub width: f64,
    pub height: f64,
    pub display_option: DisplayOption,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<Filter>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub visuals: Vec<Visual>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub visual_interactions: Vec<VisualInteraction>,
    /// Page `objects` (background, outspace, …), verbatim.
    #[serde(skip_serializing_if = "is_null")]
    pub objects: serde_json::Value,
    /// `visibility: HiddenInViewMode`.
    #[serde(skip_serializing_if = "is_false")]
    pub is_hidden: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    /// E.g. `pageBinding`, `type` (tooltip/drillthrough), `howCreated`.
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

impl Default for Page {
    fn default() -> Self {
        Self {
            id: ObjectId::default(),
            name: String::new(),
            display_name: String::new(),
            schema: None,
            width: 1280.0,
            height: 720.0,
            display_option: DisplayOption::FitToPage,
            filters: Vec::new(),
            visuals: Vec::new(),
            visual_interactions: Vec::new(),
            objects: serde_json::Value::Null,
            is_hidden: false,
            annotations: Vec::new(),
            extra: PropertyBag::new(),
            origin: None,
        }
    }
}

impl Page {
    /// Visual by name (exact match; PBIR visual names are IDs).
    pub fn visual(&self, name: &str) -> Option<&Visual> {
        self.visuals.iter().find(|v| v.name == name)
    }

    /// Visuals in paint order: ascending `z`, ties by source order.
    pub fn visuals_in_z_order(&self) -> Vec<&Visual> {
        let mut out: Vec<&Visual> = self.visuals.iter().collect();
        out.sort_by(|a, b| a.position.z.total_cmp(&b.position.z));
        out
    }

    /// Explicit interaction from `source` to `target`, if configured.
    pub fn interaction(&self, source: &str, target: &str) -> Option<&InteractionKind> {
        self.visual_interactions
            .iter()
            .find(|i| i.source == source && i.target == target)
            .map(|i| &i.kind)
    }

    /// Visuals whose `parent_group_name` is `group`.
    pub fn children_of<'a>(&'a self, group: &'a str) -> impl Iterator<Item = &'a Visual> + 'a {
        self.visuals
            .iter()
            .filter(move |v| v.parent_group_name.as_deref() == Some(group))
    }

    /// The visual group `visual` belongs to (its `parent_group_name`), if
    /// that is on the page. With duplicate names the last one counts.
    pub fn parent_group(&self, visual: &Visual) -> Option<&Visual> {
        let name = visual
            .parent_group_name
            .as_deref()
            .filter(|p| *p != visual.name)?;
        self.visuals.iter().rev().find(|v| v.name == name)
    }

    /// Where `visual` sits on the page, in page units.
    ///
    /// Power BI stores the visuals of a group relative to the group; a
    /// member whose stored rectangle already lies inside its group's page
    /// rectangle is taken as stored in page units (both occur in files).
    /// Non-finite coordinates and negative sizes count as 0; nesting is
    /// followed [`MAX_GROUP_DEPTH`] levels deep.
    pub fn page_rect(&self, visual: &Visual) -> PageRect {
        self.placement(visual, 0).0
    }

    /// `true` when `visual`'s position is stored relative to its group (see
    /// [`Page::page_rect`]).
    pub fn is_group_relative(&self, visual: &Visual) -> bool {
        self.placement(visual, 0).1
    }

    fn placement(&self, visual: &Visual, depth: usize) -> (PageRect, bool) {
        let own = PageRect::of(&visual.position);
        let Some(group) = self
            .parent_group(visual)
            .filter(|_| depth < MAX_GROUP_DEPTH)
        else {
            return (own, false);
        };
        let (outer, _) = self.placement(group, depth + 1);
        let relative = own.translate(outer.x, outer.y);
        if outer.contains(&own) || !outer.contains(&relative) {
            (own, false)
        } else {
            (relative, true)
        }
    }

    /// Indices of the visuals inside the group at `group` (members of
    /// nested groups included), in page order.
    pub fn group_members(&self, group: usize) -> Vec<usize> {
        let Some(name) = self.visuals.get(group).map(|g| g.name.as_str()) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (i, visual) in self.visuals.iter().enumerate() {
            if i == group {
                continue;
            }
            let mut current = visual;
            for _ in 0..MAX_GROUP_DEPTH {
                let Some(parent) = self.parent_group(current) else {
                    break;
                };
                if parent.name == name {
                    out.push(i);
                    break;
                }
                current = parent;
            }
        }
        out
    }

    /// New positions for the visual at `index` placed at `to`, and for the
    /// visuals in it when it is a group, as `(index, position)` pairs (the
    /// visual first, then only the members whose position changes).
    ///
    /// Members keep their place in the group: moving the group moves them,
    /// and resizing a group in scale mode (`groupMode` other than
    /// `ScrollMode`, Power BI's default) scales their positions and sizes
    /// about the group's top-left corner. A scroll-mode group's members keep
    /// their size. Members stored relative to a group keep their stored
    /// offsets (scaled); members stored in page units get new page
    /// coordinates. Their `z`, tab order and angle are kept.
    pub fn placement_of(&self, index: usize, to: &Position) -> Vec<(usize, Position)> {
        let Some(visual) = self.visuals.get(index) else {
            return Vec::new();
        };
        let mut out = vec![(index, to.clone())];
        if !visual.is_group() {
            return out;
        }
        let from = &visual.position;
        let scroll =
            visual.extra.get("groupMode").and_then(PropValue::as_str) == Some("ScrollMode");
        let factor = |new: f64, old: f64| {
            let f = new / old;
            if scroll || !(old > 0.0 && f.is_finite() && f > 0.0) {
                1.0
            } else {
                f
            }
        };
        let (sx, sy) = (factor(to.width, from.width), factor(to.height, from.height));
        let old = self.page_rect(visual);
        let own = PageRect::of(from);
        let (nx, ny) = (old.x + (to.x - own.x), old.y + (to.y - own.y));
        for i in self.group_members(index) {
            let Some(member) = self.visuals.get(i) else {
                continue;
            };
            let p = &member.position;
            let (x, y) = if self.is_group_relative(member) {
                (p.x * sx, p.y * sy)
            } else {
                let at = PageRect::of(p);
                (nx + (at.x - old.x) * sx, ny + (at.y - old.y) * sy)
            };
            let moved = Position {
                x,
                y,
                width: p.width * sx,
                height: p.height * sy,
                ..p.clone()
            };
            if moved != *p {
                out.push((i, moved));
            }
        }
        out
    }

    pub fn visit_fields(&self, f: &mut dyn FnMut(&FieldExpr)) {
        self.filters.iter().for_each(|x| x.visit_fields(f));
        self.visuals.iter().for_each(|v| v.visit_fields(f));
    }

    pub fn visit_fields_mut(&mut self, f: &mut dyn FnMut(&mut FieldExpr)) {
        self.filters.iter_mut().for_each(|x| x.visit_fields_mut(f));
        self.visuals.iter_mut().for_each(|v| v.visit_fields_mut(f));
    }
}

/// Deepest visual group nesting followed (untrusted-input guard).
pub const MAX_GROUP_DEPTH: usize = 32;

/// A rectangle in page units (see [`Page::page_rect`]).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PageRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl PageRect {
    /// `position`'s rectangle; non-finite values and negative sizes count
    /// as 0.
    pub fn of(position: &Position) -> Self {
        let finite = |v: f64| if v.is_finite() { v } else { 0.0 };
        Self {
            x: finite(position.x),
            y: finite(position.y),
            width: finite(position.width).max(0.0),
            height: finite(position.height).max(0.0),
        }
    }

    fn translate(self, dx: f64, dy: f64) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            ..self
        }
    }

    /// `true` if `other` lies inside, with half a unit of tolerance.
    fn contains(&self, other: &PageRect) -> bool {
        const EPS: f64 = 0.5;
        other.x >= self.x - EPS
            && other.y >= self.y - EPS
            && other.x + other.width <= self.x + self.width + EPS
            && other.y + other.height <= self.y + self.height + EPS
    }
}

/// Visual container position, in page units.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Position {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub width: f64,
    pub height: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_order: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub angle: Option<f64>,
}

impl Position {
    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    /// `true` if the two rectangles overlap with positive area.
    pub fn overlaps(&self, other: &Position) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }
}

/// A field bound to a visual role (`queryState.<role>.projections[]`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Projection {
    /// Data role, e.g. `Values`, `Category`, `Y`.
    pub role: String,
    pub field: FieldExpr,
    pub query_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_query_ref: Option<String>,
    /// User-renamed label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
}

impl Projection {
    /// Label to show: display name, else native query ref, else the field
    /// name, else the query ref.
    pub fn label(&self) -> &str {
        self.display_name
            .as_deref()
            .or(self.native_query_ref.as_deref())
            .or_else(|| self.field.property())
            .unwrap_or(&self.query_ref)
    }
}

/// Visual or visual group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VisualKind {
    #[default]
    Visual,
    Group,
}

/// A visual container (`visual.json`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Visual {
    pub id: ObjectId,
    /// Internal name (PBIR folder name / ID).
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub kind: VisualKind,
    /// E.g. `cardVisual`, `slicer`, `tableEx`; empty for groups.
    pub visual_type: String,
    /// Group display name (`visualGroup.displayName`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub position: Position,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub projections: Vec<Projection>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sort: Vec<SortItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<Filter>,
    /// `visual.objects`, verbatim.
    #[serde(skip_serializing_if = "is_null")]
    pub objects: serde_json::Value,
    /// `visual.visualContainerObjects`, verbatim.
    #[serde(skip_serializing_if = "is_null")]
    pub container_objects: serde_json::Value,
    #[serde(skip_serializing_if = "is_false")]
    pub is_hidden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_group_name: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    /// E.g. `drillFilterOtherVisuals`, `howCreated`, `groupMode`.
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

impl Visual {
    pub fn is_group(&self) -> bool {
        self.kind == VisualKind::Group
    }

    /// Distinct data roles in first-seen order.
    pub fn roles(&self) -> Vec<&str> {
        let mut roles: Vec<&str> = Vec::new();
        for p in &self.projections {
            if !roles.contains(&p.role.as_str()) {
                roles.push(&p.role);
            }
        }
        roles
    }

    /// Projections bound to `role` (case-sensitive, as in PBIR).
    pub fn projections_for<'a>(&'a self, role: &'a str) -> impl Iterator<Item = &'a Projection> {
        self.projections.iter().filter(move |p| p.role == role)
    }

    /// Visits projection, sort and filter fields.
    pub fn visit_fields(&self, f: &mut dyn FnMut(&FieldExpr)) {
        self.projections.iter().for_each(|p| f(&p.field));
        self.sort.iter().for_each(|s| f(&s.field));
        self.filters.iter().for_each(|x| x.visit_fields(f));
    }

    pub fn visit_fields_mut(&mut self, f: &mut dyn FnMut(&mut FieldExpr)) {
        self.projections.iter_mut().for_each(|p| f(&mut p.field));
        self.sort.iter_mut().for_each(|s| f(&mut s.field));
        self.filters.iter_mut().for_each(|x| x.visit_fields_mut(f));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visual(name: &str, z: f64) -> Visual {
        Visual {
            name: name.into(),
            visual_type: "shape".into(),
            position: Position {
                z,
                width: 10.0,
                height: 10.0,
                ..Position::default()
            },
            ..Visual::default()
        }
    }

    #[test]
    fn page_helpers() {
        let mut page = Page {
            name: "p1".into(),
            display_name: "Store Performance".into(),
            visuals: vec![visual("a", 90000.0), visual("b", 0.0), visual("c", 3000.0)],
            visual_interactions: vec![VisualInteraction {
                source: "a".into(),
                target: "b".into(),
                kind: InteractionKind::parse("NoFilter"),
            }],
            ..Page::default()
        };
        page.visuals[2].parent_group_name = Some("g".into());
        let order: Vec<_> = page
            .visuals_in_z_order()
            .iter()
            .map(|v| v.name.as_str())
            .collect();
        assert_eq!(order, ["b", "c", "a"]);
        assert_eq!(page.interaction("a", "b"), Some(&InteractionKind::NoFilter));
        assert_eq!(page.interaction("b", "a"), None);
        assert_eq!(page.children_of("g").count(), 1);
        assert_eq!((page.width, page.height), (1280.0, 720.0));

        let report = Report {
            pages: vec![
                Page {
                    name: "hidden".into(),
                    is_hidden: true,
                    ..Page::default()
                },
                page,
            ],
            active_page: Some("missing".into()),
            ..Report::default()
        };
        assert_eq!(report.start_page().unwrap().name, "p1");
        assert_eq!(report.find_page("store performance").unwrap().name, "p1");
        assert_eq!(report.visuals().count(), 3);
    }

    fn placed(name: &str, parent: Option<&str>, [x, y, width, height]: [f64; 4]) -> Visual {
        Visual {
            name: name.into(),
            parent_group_name: parent.map(Into::into),
            position: Position {
                x,
                y,
                z: 7.0,
                width,
                height,
                tab_order: Some(3),
                angle: None,
            },
            ..Visual::default()
        }
    }

    fn group(name: &str, parent: Option<&str>, rect: [f64; 4]) -> Visual {
        Visual {
            kind: VisualKind::Group,
            ..placed(name, parent, rect)
        }
    }

    /// Group `g` at (100, 50) holding `r` (stored relative), `a` (stored in
    /// page units) and the nested group `n` (relative) with `m` (relative).
    fn grouped_page() -> Page {
        Page {
            visuals: vec![
                group("g", None, [100.0, 50.0, 400.0, 200.0]),
                placed("r", Some("g"), [10.0, 20.0, 100.0, 50.0]),
                placed("a", Some("g"), [300.0, 150.0, 100.0, 50.0]),
                group("n", Some("g"), [20.0, 120.0, 150.0, 60.0]),
                placed("m", Some("n"), [5.0, 5.0, 50.0, 30.0]),
                placed("free", None, [0.0, 0.0, 10.0, 10.0]),
            ],
            ..Page::default()
        }
    }

    fn rect(page: &Page, i: usize) -> [f64; 4] {
        let r = page.page_rect(&page.visuals[i]);
        [r.x, r.y, r.width, r.height]
    }

    #[test]
    fn group_members_resolve_to_page_rectangles() {
        let page = grouped_page();
        assert_eq!(rect(&page, 1), [110.0, 70.0, 100.0, 50.0]);
        assert_eq!(rect(&page, 2), [300.0, 150.0, 100.0, 50.0]);
        assert_eq!(rect(&page, 3), [120.0, 170.0, 150.0, 60.0]);
        assert_eq!(rect(&page, 4), [125.0, 175.0, 50.0, 30.0]);
        let relative: Vec<_> = page
            .visuals
            .iter()
            .map(|v| page.is_group_relative(v))
            .collect();
        assert_eq!(relative, [false, true, false, true, true, false]);
        assert_eq!(page.group_members(0), [1, 2, 3, 4]);
        assert_eq!(page.group_members(3), [4]);
        assert!(page.group_members(5).is_empty());
        assert!(page.group_members(99).is_empty());
    }

    #[test]
    fn moving_a_group_moves_page_unit_members_only() {
        let page = grouped_page();
        let to = Position {
            x: 140.0,
            y: 40.0,
            ..page.visuals[0].position.clone()
        };
        let out = page.placement_of(0, &to);
        // Relative members keep their stored offsets; `a` moves by (40, -10).
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], (0, to));
        assert_eq!(
            out[1],
            (
                2,
                Position {
                    x: 340.0,
                    y: 140.0,
                    ..page.visuals[2].position.clone()
                }
            )
        );
    }

    #[test]
    fn resizing_a_group_scales_its_members_unless_it_scrolls() {
        let mut page = grouped_page();
        let to = Position {
            width: 800.0,
            height: 100.0,
            ..page.visuals[0].position.clone()
        };
        let scaled: Vec<_> = page
            .placement_of(0, &to)
            .into_iter()
            .map(|(i, p)| (i, [p.x, p.y, p.width, p.height], p.z, p.tab_order))
            .collect();
        assert_eq!(
            scaled,
            [
                (0, [100.0, 50.0, 800.0, 100.0], 7.0, Some(3)),
                (1, [20.0, 10.0, 200.0, 25.0], 7.0, Some(3)),
                (2, [500.0, 100.0, 200.0, 25.0], 7.0, Some(3)),
                (3, [40.0, 60.0, 300.0, 30.0], 7.0, Some(3)),
                (4, [10.0, 2.5, 100.0, 15.0], 7.0, Some(3)),
            ]
        );

        page.visuals[0]
            .extra
            .insert("groupMode".into(), "ScrollMode".into());
        let moved = Position {
            x: 0.0,
            ..to.clone()
        };
        let out = page.placement_of(0, &moved);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].0, 2);
        assert_eq!(
            [out[1].1.x, out[1].1.y, out[1].1.width, out[1].1.height],
            [200.0, 150.0, 100.0, 50.0]
        );
    }

    #[test]
    fn placement_of_plain_visuals_and_cycles() {
        let mut page = grouped_page();
        let to = Position {
            x: 5.0,
            ..Position::default()
        };
        assert_eq!(page.placement_of(5, &to), [(5, to.clone())]);
        assert!(page.placement_of(99, &to).is_empty());

        // Groups that contain each other stay bounded.
        page.visuals[0].parent_group_name = Some("n".into());
        assert_eq!(page.group_members(0), [1, 2, 3, 4]);
        let _ = page.page_rect(&page.visuals[4]);
        assert!(!page.placement_of(0, &to).is_empty());
    }

    #[test]
    fn positions_overlap_only_with_positive_area() {
        let a = Position {
            width: 10.0,
            height: 10.0,
            ..Position::default()
        };
        let touching = Position {
            x: 10.0,
            ..a.clone()
        };
        let inside = Position {
            x: 5.0,
            y: 5.0,
            ..a.clone()
        };
        assert!(!a.overlaps(&touching));
        assert!(a.overlaps(&inside));
    }

    #[test]
    fn theme_resources_resolve_to_static_paths() {
        let report = Report {
            theme: ThemeRef {
                base_theme: Some(ResourceRef {
                    name: "CY24SU10".into(),
                    kind: Some("SharedResources".into()),
                    ..ResourceRef::default()
                }),
                custom_theme: None,
            },
            resource_packages: vec![ResourcePackage {
                name: "SharedResources".into(),
                kind: Some("SharedResources".into()),
                items: vec![ResourceItem {
                    name: "CY24SU10".into(),
                    path: "BaseThemes/CY24SU10.json".into(),
                    kind: Some("BaseTheme".into()),
                    ..ResourceItem::default()
                }],
                ..ResourcePackage::default()
            }],
            ..Report::default()
        };
        let base = report.theme.base_theme.as_ref().unwrap();
        assert_eq!(
            report.static_resource_path(base).as_deref(),
            Some("StaticResources/SharedResources/BaseThemes/CY24SU10.json")
        );
    }

    #[test]
    fn projection_label_prefers_display_name() {
        let mut p = Projection {
            role: "Values".into(),
            field: FieldExpr::column("Stores", "Store Name"),
            query_ref: "Stores.Store Name".into(),
            ..Projection::default()
        };
        assert_eq!(p.label(), "Store Name");
        p.display_name = Some("Store".into());
        assert_eq!(p.label(), "Store");
    }
}
