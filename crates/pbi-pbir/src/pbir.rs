//! Lossless serde types for the files of a PBIR report folder.
//!
//! Every type is declared with [`json_object!`](crate::json): known
//! properties are typed `Option`s, everything else stays in `extra`, and key
//! order is preserved. Formatting `objects` are kept as raw
//! [`serde_json::Value`]s (see [`crate::objects`] for typed accessors).

use indexmap::IndexMap;
use serde_json::{Number, Value};

use crate::json::{json_object, num_f64, num_i64};
use crate::query::{FieldExprJson, FilterDefinitionJson};

json_object! {
    /// `definition.pbir`: which semantic model the report uses.
    pub struct DefinitionPbir {
        "$schema" => schema: String,
        /// `4.0` for current PBIP.
        "version" => version: String,
        "datasetReference" => dataset_reference: DatasetReferenceJson,
    }
}

json_object! {
    /// `datasetReference`: `byPath` (local model) or `byConnection` (service).
    pub struct DatasetReferenceJson {
        "byPath" => by_path: ByPathJson,
        "byConnection" => by_connection: ByConnectionJson,
    }
}

json_object! {
    /// Local model folder, relative to the report folder.
    pub struct ByPathJson {
        "path" => path: String,
    }
}

json_object! {
    /// Service-hosted model. Explicit `null`s (e.g. `pbiServiceModelId`)
    /// are kept in `extra` and written back.
    pub struct ByConnectionJson {
        "connectionString" => connection_string: String,
        "pbiServiceModelId" => pbi_service_model_id: Value,
        "pbiModelVirtualServerName" => pbi_model_virtual_server_name: String,
        "pbiModelDatabaseName" => pbi_model_database_name: String,
        "name" => name: String,
        "connectionType" => connection_type: String,
    }
}

json_object! {
    /// `.platform`: Fabric Git-integration item metadata.
    pub struct Platform {
        "$schema" => schema: String,
        "metadata" => metadata: PlatformMetadata,
        "config" => config: PlatformConfig,
    }
}

json_object! {
    /// `.platform` `metadata`.
    pub struct PlatformMetadata {
        /// Item type, e.g. `Report`, `SemanticModel`.
        "type" => item_type: String,
        "displayName" => display_name: String,
        "description" => description: String,
    }
}

json_object! {
    /// `.platform` `config`.
    pub struct PlatformConfig {
        "version" => version: String,
        /// Item GUID; must be unique per item.
        "logicalId" => logical_id: String,
    }
}

json_object! {
    /// `definition/version.json`.
    pub struct VersionJson {
        "$schema" => schema: String,
        "version" => version: String,
    }
}

json_object! {
    /// `definition/report.json`.
    pub struct ReportJson {
        "$schema" => schema: String,
        "themeCollection" => theme_collection: ThemeCollectionJson,
        "filterConfig" => filter_config: FilterConfigJson,
        "objects" => objects: Value,
        "reportSource" => report_source: String,
        "publicCustomVisuals" => public_custom_visuals: Vec<String>,
        "resourcePackages" => resource_packages: Vec<ResourcePackageJson>,
        "organizationCustomVisuals" => organization_custom_visuals: Value,
        "annotations" => annotations: Vec<AnnotationJson>,
        "dataSourceVariables" => data_source_variables: String,
        "settings" => settings: Value,
        "slowDataSourceSettings" => slow_data_source_settings: Value,
    }
}

json_object! {
    /// `themeCollection`.
    pub struct ThemeCollectionJson {
        "baseTheme" => base_theme: ThemeMetadataJson,
        "customTheme" => custom_theme: ThemeMetadataJson,
    }
}

json_object! {
    /// A theme reference (`baseTheme` / `customTheme`).
    pub struct ThemeMetadataJson {
        "name" => name: String,
        "reportVersionAtImport" => report_version_at_import: Value,
        /// `SharedResources` or `RegisteredResources`.
        "type" => package_type: String,
    }
}

json_object! {
    /// `resourcePackages[]`.
    pub struct ResourcePackageJson {
        "id" => id: Number,
        "name" => name: String,
        "type" => package_type: String,
        "items" => items: Vec<ResourcePackageItemJson>,
        "disabled" => disabled: bool,
    }
}

json_object! {
    /// `resourcePackages[].items[]`.
    pub struct ResourcePackageItemJson {
        "id" => id: Number,
        "name" => name: String,
        /// Relative to `StaticResources/<package name>/`.
        "path" => path: String,
        /// `BaseTheme`, `CustomTheme`, `Image`, …
        "type" => item_type: String,
    }
}

json_object! {
    /// `annotations[]`.
    pub struct AnnotationJson {
        "name" => name: String,
        "value" => value: String,
    }
}

json_object! {
    /// `definition/pages/pages.json`.
    pub struct PagesJson {
        "$schema" => schema: String,
        "pageOrder" => page_order: Vec<String>,
        "activePageName" => active_page_name: String,
        "landingPageName" => landing_page_name: String,
    }
}

json_object! {
    /// `definition/pages/<id>/page.json`.
    pub struct PageJson {
        "$schema" => schema: String,
        "name" => name: String,
        "displayName" => display_name: String,
        /// `FitToPage`, `FitToWidth`, `ActualSize`, …
        "displayOption" => display_option: String,
        "height" => height: Number,
        "width" => width: Number,
        "filterConfig" => filter_config: FilterConfigJson,
        "pageBinding" => page_binding: PageBindingJson,
        "objects" => objects: Value,
        /// `Drillthrough` or `Tooltip`.
        "type" => page_type: String,
        /// `AlwaysVisible` or `HiddenInViewMode`.
        "visibility" => visibility: String,
        "visualInteractions" => visual_interactions: Vec<VisualInteractionJson>,
        "autoPageGenerationConfig" => auto_page_generation_config: Value,
        "annotations" => annotations: Vec<AnnotationJson>,
        "howCreated" => how_created: String,
    }
}

json_object! {
    /// `pageBinding` (drillthrough / tooltip parameters).
    pub struct PageBindingJson {
        "name" => name: String,
        "type" => binding_type: String,
        "referenceScope" => reference_scope: String,
        "parameters" => parameters: Value,
        "acceptsFilterContext" => accepts_filter_context: String,
    }
}

json_object! {
    /// `visualInteractions[]`: visual names, not IDs.
    pub struct VisualInteractionJson {
        "source" => source: String,
        "target" => target: String,
        /// `Default`, `DataFilter`, `HighlightFilter` or `NoFilter`.
        "type" => interaction_type: String,
    }
}

json_object! {
    /// `definition/pages/<id>/visuals/<id>/visual.json`.
    pub struct VisualContainerJson {
        "$schema" => schema: String,
        "name" => name: String,
        "position" => position: PositionJson,
        "visual" => visual: VisualJson,
        "visualGroup" => visual_group: VisualGroupJson,
        "parentGroupName" => parent_group_name: String,
        "filterConfig" => filter_config: FilterConfigJson,
        "isHidden" => is_hidden: bool,
        "annotations" => annotations: Vec<AnnotationJson>,
        "howCreated" => how_created: String,
    }
}

json_object! {
    /// Visual container position in page units.
    pub struct PositionJson {
        "x" => x: Number,
        "y" => y: Number,
        "z" => z: Number,
        "height" => height: Number,
        "width" => width: Number,
        "tabOrder" => tab_order: Number,
        "angle" => angle: Number,
    }
}

json_object! {
    /// `visual` (visual configuration).
    pub struct VisualJson {
        "visualType" => visual_type: String,
        "autoSelectVisualType" => auto_select_visual_type: bool,
        "query" => query: VisualQueryJson,
        "expansionStates" => expansion_states: Value,
        /// Formatting objects, verbatim.
        "objects" => objects: Value,
        /// Container formatting (title, background, border, …), verbatim.
        "visualContainerObjects" => visual_container_objects: Value,
        "syncGroup" => sync_group: Value,
        "drillFilterOtherVisuals" => drill_filter_other_visuals: bool,
    }
}

json_object! {
    /// `visual.query`.
    pub struct VisualQueryJson {
        /// Role name (`Values`, `Category`, `Y`, …) → projections, in order.
        "queryState" => query_state: IndexMap<String, ProjectionStateJson>,
        "sortDefinition" => sort_definition: SortDefinitionJson,
        "options" => options: Value,
        "isDrillDisabled" => is_drill_disabled: bool,
    }
}

json_object! {
    /// `queryState.<role>`.
    pub struct ProjectionStateJson {
        "projections" => projections: Vec<ProjectionJson>,
        "showAll" => show_all: bool,
        "fieldParameters" => field_parameters: Value,
    }
}

json_object! {
    /// A field bound to a visual role.
    pub struct ProjectionJson {
        "field" => field: FieldExprJson,
        "queryRef" => query_ref: String,
        "nativeQueryRef" => native_query_ref: String,
        "displayName" => display_name: String,
        "format" => format: String,
        "active" => active: bool,
        "hidden" => hidden: bool,
    }
}

json_object! {
    /// `query.sortDefinition`.
    pub struct SortDefinitionJson {
        "sort" => sort: Vec<SortJson>,
        "isDefaultSort" => is_default_sort: bool,
    }
}

json_object! {
    /// `sortDefinition.sort[]`.
    pub struct SortJson {
        "field" => field: FieldExprJson,
        /// `Ascending` or `Descending`.
        "direction" => direction: String,
    }
}

json_object! {
    /// `visualGroup`.
    pub struct VisualGroupJson {
        "displayName" => display_name: String,
        /// `ScaleMode` or `ScrollMode`.
        "groupMode" => group_mode: String,
        "objects" => objects: Value,
    }
}

json_object! {
    /// `filterConfig` (report, page or visual level).
    pub struct FilterConfigJson {
        "$schema" => schema: String,
        "filters" => filters: Vec<FilterJson>,
        "filterSortOrder" => filter_sort_order: String,
    }
}

json_object! {
    /// A filter card (`filterConfig.filters[]`; also the legacy layout's
    /// stringified `filters` arrays, which use `expression` for the field and
    /// a numeric `howCreated`).
    pub struct FilterJson {
        "name" => name: String,
        "displayName" => display_name: String,
        "ordinal" => ordinal: Number,
        "field" => field: FieldExprJson,
        /// Legacy spelling of `field`.
        "expression" => expression: FieldExprJson,
        /// `Categorical`, `Advanced`, `TopN`, …
        "type" => filter_type: String,
        "filter" => filter: FilterDefinitionJson,
        "restatement" => restatement: String,
        /// PBIR: `Auto`, `User`, …; legacy: a number.
        "howCreated" => how_created: Value,
        "isHiddenInViewMode" => is_hidden_in_view_mode: bool,
        "isLockedInViewMode" => is_locked_in_view_mode: bool,
        "objects" => objects: Value,
    }
}

/// Last path segment of a `$schema` URL that ends in
/// `/<kind>/<version>/schema.json`, as `(kind, version)`.
pub fn schema_kind_version(schema: &str) -> Option<(&str, &str)> {
    let mut parts = schema.rsplit('/');
    let file = parts.next()?;
    if !file.starts_with("schema") {
        return None;
    }
    let version = parts.next()?;
    let kind = parts.next()?;
    Some((kind, version))
}

impl PositionJson {
    pub fn x_f64(&self) -> f64 {
        self.x.as_ref().and_then(num_f64).unwrap_or(0.0)
    }

    pub fn y_f64(&self) -> f64 {
        self.y.as_ref().and_then(num_f64).unwrap_or(0.0)
    }

    pub fn z_f64(&self) -> f64 {
        self.z.as_ref().and_then(num_f64).unwrap_or(0.0)
    }

    pub fn width_f64(&self) -> f64 {
        self.width.as_ref().and_then(num_f64).unwrap_or(0.0)
    }

    pub fn height_f64(&self) -> f64 {
        self.height.as_ref().and_then(num_f64).unwrap_or(0.0)
    }

    pub fn tab_order_i64(&self) -> Option<i64> {
        self.tab_order.as_ref().and_then(num_i64)
    }

    pub fn angle_f64(&self) -> Option<f64> {
        self.angle.as_ref().and_then(num_f64)
    }
}

impl VisualContainerJson {
    /// `visual.visualType`, if this is a visual (not a group).
    pub fn visual_type(&self) -> Option<&str> {
        self.visual.as_ref()?.visual_type.as_deref()
    }

    /// Visual-level filters.
    pub fn filters(&self) -> &[FilterJson] {
        self.filter_config
            .as_ref()
            .and_then(|f| f.filters.as_deref())
            .unwrap_or_default()
    }
}

impl PageJson {
    /// Page-level filters.
    pub fn filters(&self) -> &[FilterJson] {
        self.filter_config
            .as_ref()
            .and_then(|f| f.filters.as_deref())
            .unwrap_or_default()
    }
}

impl ReportJson {
    /// Report-level filters.
    pub fn filters(&self) -> &[FilterJson] {
        self.filter_config
            .as_ref()
            .and_then(|f| f.filters.as_deref())
            .unwrap_or_default()
    }

    /// `StaticResources/<package>/<item path>` of the resource-package item
    /// named `name` in a package of type `package_type` (any type if
    /// `None`).
    pub fn resource_path(&self, name: &str, package_type: Option<&str>) -> Option<String> {
        self.resource_packages.iter().flatten().find_map(|p| {
            let type_ok = match (package_type, p.package_type.as_deref()) {
                (Some(want), Some(have)) => want.eq_ignore_ascii_case(have),
                _ => true,
            };
            if !type_ok {
                return None;
            }
            let item = p.items.iter().flatten().find(|i| {
                i.name
                    .as_deref()
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            })?;
            Some(format!(
                "StaticResources/{}/{}",
                p.name.as_deref()?,
                item.path.as_deref()?
            ))
        })
    }
}

impl DefinitionPbir {
    pub fn by_path(&self) -> Option<&str> {
        self.dataset_reference
            .as_ref()?
            .by_path
            .as_ref()?
            .path
            .as_deref()
    }

    pub fn by_connection(&self) -> Option<&ByConnectionJson> {
        self.dataset_reference.as_ref()?.by_connection.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_urls_split_into_kind_and_version() {
        assert_eq!(
            schema_kind_version(
                "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/visualContainer/2.4.0/schema.json"
            ),
            Some(("visualContainer", "2.4.0"))
        );
        assert_eq!(schema_kind_version("nonsense"), None);
    }

    #[test]
    fn by_connection_keeps_explicit_null() {
        let src = r#"{"version":"4.0","datasetReference":{"byConnection":{"connectionString":null,"pbiServiceModelId":null,"pbiModelVirtualServerName":"acme_virtualserver","pbiModelDatabaseName":"0000ac","name":"EntityDataSource","connectionType":"pbiServiceXmlaStyleLive"}}}"#;
        let def: DefinitionPbir = serde_json::from_str(src).unwrap();
        let conn = def.by_connection().unwrap();
        assert_eq!(
            conn.connection_type.as_deref(),
            Some("pbiServiceXmlaStyleLive")
        );
        assert!(conn.connection_string.is_none());
        assert_eq!(serde_json::to_string(&def).unwrap(), src);
    }
}
