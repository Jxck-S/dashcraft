//! Format-agnostic report AST (PBIR, PBIR-Legacy and PBIX `Layout` all map
//! onto these types).

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::ids::ObjectId;
use crate::macros::string_enum;
use crate::names::names_eq;
use crate::props::{Annotation, Origin, PropertyBag};
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

    pub fn visit_fields(&self, f: &mut dyn FnMut(&FieldExpr)) {
        self.filters.iter().for_each(|x| x.visit_fields(f));
        self.visuals.iter().for_each(|v| v.visit_fields(f));
    }

    pub fn visit_fields_mut(&mut self, f: &mut dyn FnMut(&mut FieldExpr)) {
        self.filters.iter_mut().for_each(|x| x.visit_fields_mut(f));
        self.visuals.iter_mut().for_each(|v| v.visit_fields_mut(f));
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
