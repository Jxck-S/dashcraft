//! Legacy report layouts: the PBIX `Report/Layout` entry (UTF-16LE) and the
//! PBIR-Legacy `report.json` (UTF-8). Both share one shape: `sections[]` of
//! `visualContainers[]`, with `config`, `filters`, `query` and
//! `dataTransforms` stored as **stringified** JSON.
//!
//! [`read_legacy_layout`] parses the outer document losslessly
//! ([`LegacyLayoutJson`]) and the embedded strings into typed parts
//! ([`LegacySectionParts`], [`LegacyVisualParts`]) aligned by index with
//! `layout.sections[i].visualContainers[j]`.
//!
//! Numeric enums are mapped with the order of the matching PBIR schema
//! enums (best effort; unknown codes are kept as their number):
//! `displayOption` 0 DeprecatedDynamic, 1 FitToPage, 2 FitToWidth,
//! 3 ActualSize, 4 ActualSizeTopLeft; interaction `type` 0 Default,
//! 1 DataFilter, 2 HighlightFilter, 3 NoFilter; `groupMode` 0 ScaleMode,
//! 1 ScrollMode; resource package `type` 0 CustomVisual,
//! 1 RegisteredResources, 2 SharedResources, 3
//! OrganizationalStoreCustomVisual; resource item `type` 100 Image,
//! 200 ShapeMap, 201 CustomTheme, 202 BaseTheme; section `visibility`
//! 1 HiddenInViewMode; `OrderBy.Direction` 1 ascending, 2 descending.

use indexmap::IndexMap;
use pbi_model::{
    Diagnostic, DisplayOption, FieldExpr, Origin, Page, Position, Projection, PropertyBag, Report,
    ResourceItem, ResourcePackage, ResourceRef, SortDirection, SortItem, ThemeRef, Visual,
    VisualInteraction, VisualKind,
};
use serde::de::DeserializeOwned;
use serde_json::{Number, Value};

use crate::convert::{Ctx, field_expr, filters, interaction_kind, put_extra, put_opt};
use crate::error::{PbirError, codes};
use crate::json::{json_object, num_f64, num_i64};
use crate::pbir::{FilterJson, PositionJson};
use crate::query::{QueryDefinitionJson, QueryExpr};

json_object! {
    /// The outer legacy layout document.
    pub struct LegacyLayoutJson {
        "id" => id: Number,
        "reportId" => report_id: Number,
        "theme" => theme: String,
        "resourcePackages" => resource_packages: Vec<LegacyResourcePackageEntryJson>,
        "sections" => sections: Vec<LegacySectionJson>,
        /// Stringified [`LegacyReportConfigJson`].
        "config" => config: Value,
        "layoutOptimization" => layout_optimization: Number,
        /// Stringified `FilterJson[]`.
        "filters" => filters: Value,
        "publicCustomVisuals" => public_custom_visuals: Value,
        "pods" => pods: Value,
    }
}

json_object! {
    /// `resourcePackages[]` wrapper (`{"resourcePackage": {...}}`).
    pub struct LegacyResourcePackageEntryJson {
        "resourcePackage" => resource_package: LegacyResourcePackageJson,
    }
}

json_object! {
    /// A legacy resource package (numeric `type`).
    pub struct LegacyResourcePackageJson {
        "id" => id: Number,
        "name" => name: String,
        "type" => package_type: Value,
        "items" => items: Vec<LegacyResourceItemJson>,
        "disabled" => disabled: bool,
    }
}

json_object! {
    /// A legacy resource item (numeric `type`).
    pub struct LegacyResourceItemJson {
        "id" => id: Number,
        "name" => name: String,
        "path" => path: String,
        "type" => item_type: Value,
    }
}

json_object! {
    /// A page (`sections[]`).
    pub struct LegacySectionJson {
        "id" => id: Number,
        "name" => name: String,
        "displayName" => display_name: String,
        /// Stringified `FilterJson[]`.
        "filters" => filters: Value,
        "ordinal" => ordinal: Number,
        "visualContainers" => visual_containers: Vec<LegacyVisualContainerJson>,
        /// Stringified [`LegacySectionConfigJson`].
        "config" => config: Value,
        /// Numeric display option (see the module docs).
        "displayOption" => display_option: Value,
        "width" => width: Number,
        "height" => height: Number,
    }
}

json_object! {
    /// A visual container (`sections[].visualContainers[]`).
    pub struct LegacyVisualContainerJson {
        "id" => id: Number,
        "x" => x: Number,
        "y" => y: Number,
        "z" => z: Number,
        "width" => width: Number,
        "height" => height: Number,
        "tabOrder" => tab_order: Number,
        /// Stringified [`LegacyVisualConfigJson`].
        "config" => config: Value,
        /// Stringified `FilterJson[]`.
        "filters" => filters: Value,
        /// Stringified data query (kept verbatim).
        "query" => query: Value,
        /// Stringified data transforms (kept verbatim).
        "dataTransforms" => data_transforms: Value,
    }
}

json_object! {
    /// Report `config` (stringified).
    pub struct LegacyReportConfigJson {
        "version" => version: Value,
        "themeCollection" => theme_collection: LegacyThemeCollectionJson,
        "activeSectionIndex" => active_section_index: Number,
        "defaultDrillFilterOtherVisuals" => default_drill_filter_other_visuals: bool,
        "settings" => settings: Value,
        "objects" => objects: Value,
    }
}

json_object! {
    /// `themeCollection` (legacy).
    pub struct LegacyThemeCollectionJson {
        "baseTheme" => base_theme: LegacyThemeJson,
        "customTheme" => custom_theme: LegacyThemeJson,
    }
}

json_object! {
    /// A theme reference (legacy; numeric `type`).
    pub struct LegacyThemeJson {
        "name" => name: String,
        "version" => version: Value,
        "type" => package_type: Value,
    }
}

json_object! {
    /// Section `config` (stringified).
    pub struct LegacySectionConfigJson {
        /// Visual interactions.
        "relationships" => relationships: Vec<LegacyRelationshipJson>,
        /// 1 = hidden in view mode.
        "visibility" => visibility: Number,
        "objects" => objects: Value,
    }
}

json_object! {
    /// A visual interaction (legacy; numeric `type`).
    pub struct LegacyRelationshipJson {
        "source" => source: String,
        "target" => target: String,
        "type" => interaction_type: Value,
    }
}

json_object! {
    /// Visual container `config` (stringified).
    pub struct LegacyVisualConfigJson {
        "name" => name: String,
        "layouts" => layouts: Vec<LegacyVisualLayoutJson>,
        "singleVisual" => single_visual: LegacySingleVisualJson,
        "singleVisualGroup" => single_visual_group: LegacyVisualGroupJson,
        "parentGroupName" => parent_group_name: String,
    }
}

json_object! {
    /// `layouts[]` (one per layout; 0 is the desktop layout).
    pub struct LegacyVisualLayoutJson {
        "id" => id: Number,
        "position" => position: PositionJson,
    }
}

json_object! {
    /// `singleVisual`.
    pub struct LegacySingleVisualJson {
        "visualType" => visual_type: String,
        /// Role → `[{queryRef, active}]`.
        "projections" => projections: IndexMap<String, Vec<LegacyProjectionJson>>,
        "prototypeQuery" => prototype_query: QueryDefinitionJson,
        /// `queryRef` → `{displayName}` overrides.
        "columnProperties" => column_properties: IndexMap<String, LegacyColumnPropertiesJson>,
        "objects" => objects: Value,
        "vcObjects" => vc_objects: Value,
        /// `{"mode": "hidden"}` for hidden visuals.
        "display" => display: Value,
        "drillFilterOtherVisuals" => drill_filter_other_visuals: bool,
        "hasDefaultSort" => has_default_sort: bool,
    }
}

json_object! {
    /// A legacy projection (resolved through `prototypeQuery.Select`).
    pub struct LegacyProjectionJson {
        "queryRef" => query_ref: String,
        "active" => active: bool,
    }
}

json_object! {
    /// `columnProperties` entry.
    pub struct LegacyColumnPropertiesJson {
        "displayName" => display_name: String,
    }
}

json_object! {
    /// `singleVisualGroup`.
    pub struct LegacyVisualGroupJson {
        "displayName" => display_name: String,
        "groupMode" => group_mode: Value,
        "isHidden" => is_hidden: bool,
        "objects" => objects: Value,
    }
}

/// Text encoding of a legacy layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextEncoding {
    #[default]
    Utf8,
    Utf8Bom,
    /// PBIX `Report/Layout` (usually without BOM).
    Utf16Le,
    Utf16LeBom,
    Utf16BeBom,
}

/// Parsed embedded JSON of a section.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LegacySectionParts {
    pub config: Option<LegacySectionConfigJson>,
    pub filters: Vec<FilterJson>,
    /// Aligned with `visualContainers`.
    pub visuals: Vec<LegacyVisualParts>,
}

/// Parsed embedded JSON of a visual container.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LegacyVisualParts {
    pub config: Option<LegacyVisualConfigJson>,
    pub filters: Vec<FilterJson>,
    pub query: Option<Value>,
    pub data_transforms: Option<Value>,
}

/// A legacy layout with its embedded JSON parsed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LegacyReport {
    /// Source path used in [`Origin`]s (empty by default; `pbi-core` sets
    /// e.g. `Report/Layout` or `report.json`).
    pub path: String,
    pub encoding: TextEncoding,
    /// The outer document, lossless.
    pub layout: LegacyLayoutJson,
    pub config: Option<LegacyReportConfigJson>,
    pub filters: Vec<FilterJson>,
    /// Aligned with `layout.sections`.
    pub sections: Vec<LegacySectionParts>,
    /// Problems in embedded JSON.
    pub diagnostics: Vec<Diagnostic>,
}

/// Decodes legacy layout bytes: UTF-16LE/BE with BOM, UTF-16LE without BOM
/// (detected from a zero second byte), or UTF-8 (optional BOM).
pub fn decode_text(bytes: &[u8]) -> Result<(String, TextEncoding), PbirError> {
    let utf16 = |enc: &'static encoding_rs::Encoding, body: &[u8], kind| {
        if !body.len().is_multiple_of(2) {
            return Err(PbirError::Encoding("odd UTF-16 byte count".into()));
        }
        enc.decode_without_bom_handling_and_without_replacement(body)
            .map(|text| (text.into_owned(), kind))
            .ok_or_else(|| PbirError::Encoding("malformed UTF-16".into()))
    };
    if let Some(body) = bytes.strip_prefix(b"\xFF\xFE") {
        return utf16(encoding_rs::UTF_16LE, body, TextEncoding::Utf16LeBom);
    }
    if let Some(body) = bytes.strip_prefix(b"\xFE\xFF") {
        return utf16(encoding_rs::UTF_16BE, body, TextEncoding::Utf16BeBom);
    }
    if let Some(body) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        return std::str::from_utf8(body)
            .map(|s| (s.to_owned(), TextEncoding::Utf8Bom))
            .map_err(|e| PbirError::Encoding(e.to_string()));
    }
    if bytes.len() >= 2 && bytes[0] != 0 && bytes[1] == 0 {
        return utf16(encoding_rs::UTF_16LE, bytes, TextEncoding::Utf16Le);
    }
    std::str::from_utf8(bytes)
        .map(|s| (s.to_owned(), TextEncoding::Utf8))
        .map_err(|e| PbirError::Encoding(e.to_string()))
}

/// Parses a stringified (or already structured) embedded JSON value;
/// failures become diagnostics.
fn embedded<T: DeserializeOwned>(
    value: Option<&Value>,
    origin: impl FnOnce() -> Origin,
    diags: &mut Vec<Diagnostic>,
) -> Option<T> {
    let result = match value? {
        Value::Null => return None,
        Value::String(s) if s.trim().is_empty() => return None,
        Value::String(s) => serde_json::from_str(s),
        other => serde_json::from_value(other.clone()),
    };
    match result {
        Ok(v) => Some(v),
        Err(e) => {
            let origin = origin();
            diags.push(
                Diagnostic::warning(
                    codes::INVALID_EMBEDDED_JSON,
                    format!("invalid embedded JSON at {origin}: {e}"),
                )
                .with_origin(origin),
            );
            None
        }
    }
}

/// Reads a legacy layout (PBIX `Report/Layout` or PBIR-Legacy
/// `report.json`).
///
/// Fails if the bytes are not text, not JSON, or have no `sections` array;
/// invalid embedded JSON strings become diagnostics. Origins have an empty
/// path; use [`read_legacy_layout_at`] to set one.
pub fn read_legacy_layout(bytes: &[u8]) -> Result<LegacyReport, PbirError> {
    read_legacy_layout_at(bytes, "")
}

/// [`read_legacy_layout`] with the source `path` used in errors and
/// [`Origin`]s (e.g. `Report/Layout`).
pub fn read_legacy_layout_at(bytes: &[u8], path: &str) -> Result<LegacyReport, PbirError> {
    let (text, encoding) = decode_text(bytes)?;
    let layout: LegacyLayoutJson =
        serde_json::from_str(&text).map_err(|source| PbirError::Json {
            path: if path.is_empty() { "Layout" } else { path }.to_owned(),
            source,
        })?;
    if layout.sections.is_none() {
        return Err(PbirError::Legacy("no `sections` array".into()));
    }
    let mut diags = Vec::new();
    let at = |pointer: String| move || Origin::file(path).with_pointer(pointer);
    let config = embedded(layout.config.as_ref(), at("/config".into()), &mut diags);
    let filters =
        embedded(layout.filters.as_ref(), at("/filters".into()), &mut diags).unwrap_or_default();
    let mut sections = Vec::new();
    for (i, s) in layout.sections.iter().flatten().enumerate() {
        let mut parts = LegacySectionParts {
            config: embedded(
                s.config.as_ref(),
                at(format!("/sections/{i}/config")),
                &mut diags,
            ),
            filters: embedded(
                s.filters.as_ref(),
                at(format!("/sections/{i}/filters")),
                &mut diags,
            )
            .unwrap_or_default(),
            visuals: Vec::new(),
        };
        for (j, vc) in s.visual_containers.iter().flatten().enumerate() {
            let base = format!("/sections/{i}/visualContainers/{j}");
            parts.visuals.push(LegacyVisualParts {
                config: embedded(vc.config.as_ref(), at(format!("{base}/config")), &mut diags),
                filters: embedded(
                    vc.filters.as_ref(),
                    at(format!("{base}/filters")),
                    &mut diags,
                )
                .unwrap_or_default(),
                query: embedded(vc.query.as_ref(), at(format!("{base}/query")), &mut diags),
                data_transforms: embedded(
                    vc.data_transforms.as_ref(),
                    at(format!("{base}/dataTransforms")),
                    &mut diags,
                ),
            });
        }
        sections.push(parts);
    }
    Ok(LegacyReport {
        path: path.to_owned(),
        encoding,
        layout,
        config,
        filters,
        sections,
        diagnostics: diags,
    })
}

/// Maps a numeric legacy enum to its PBIR name; strings pass through.
fn enum_name(value: Option<&Value>, names: &[(i64, &str)]) -> Option<String> {
    match value? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(
            num_i64(n)
                .and_then(|i| names.iter().find(|(code, _)| *code == i))
                .map_or_else(|| n.to_string(), |(_, name)| (*name).to_owned()),
        ),
        _ => None,
    }
}

const DISPLAY_OPTIONS: [(i64, &str); 5] = [
    (0, "DeprecatedDynamic"),
    (1, "FitToPage"),
    (2, "FitToWidth"),
    (3, "ActualSize"),
    (4, "ActualSizeTopLeft"),
];
const INTERACTIONS: [(i64, &str); 4] = [
    (0, "Default"),
    (1, "DataFilter"),
    (2, "HighlightFilter"),
    (3, "NoFilter"),
];
const GROUP_MODES: [(i64, &str); 2] = [(0, "ScaleMode"), (1, "ScrollMode")];
const PACKAGE_TYPES: [(i64, &str); 4] = [
    (0, "CustomVisual"),
    (1, "RegisteredResources"),
    (2, "SharedResources"),
    (3, "OrganizationalStoreCustomVisual"),
];
const ITEM_TYPES: [(i64, &str); 4] = [
    (100, "Image"),
    (200, "ShapeMap"),
    (201, "CustomTheme"),
    (202, "BaseTheme"),
];

fn legacy_theme(theme: Option<&LegacyThemeJson>) -> Option<ResourceRef> {
    let t = theme?;
    let mut extra = PropertyBag::new();
    put_opt(&mut extra, "version", t.version.as_ref());
    put_extra(&mut extra, "", &t.extra);
    Some(ResourceRef {
        name: t.name.clone().unwrap_or_default(),
        kind: enum_name(t.package_type.as_ref(), &PACKAGE_TYPES),
        extra,
    })
}

fn f64_or(n: Option<&Number>, default: f64) -> f64 {
    n.and_then(num_f64).unwrap_or(default)
}

/// Converts a legacy layout into the report AST (same shape as
/// [`crate::to_report_ast`] for equivalent content).
pub fn legacy_to_report_ast(legacy: &LegacyReport) -> (Report, Vec<Diagnostic>) {
    let mut ctx = Ctx::default();
    let path = legacy.path.as_str();
    let layout = &legacy.layout;
    let mut report = Report {
        origin: Some(Origin::file(path)),
        ..Report::default()
    };
    put_opt(&mut report.extra, "id", layout.id.as_ref());
    put_opt(&mut report.extra, "reportId", layout.report_id.as_ref());
    put_opt(&mut report.extra, "theme", layout.theme.as_ref());
    put_opt(
        &mut report.extra,
        "layoutOptimization",
        layout.layout_optimization.as_ref(),
    );
    put_opt(
        &mut report.extra,
        "publicCustomVisuals",
        layout.public_custom_visuals.as_ref(),
    );
    put_opt(&mut report.extra, "pods", layout.pods.as_ref());
    put_extra(&mut report.extra, "", &layout.extra);

    if let Some(cfg) = &legacy.config {
        let tc = cfg.theme_collection.as_ref();
        report.theme = ThemeRef {
            base_theme: legacy_theme(tc.and_then(|t| t.base_theme.as_ref())),
            custom_theme: legacy_theme(tc.and_then(|t| t.custom_theme.as_ref())),
        };
        report.settings = cfg.settings.clone().unwrap_or(Value::Null);
        report.objects = cfg.objects.clone().unwrap_or(Value::Null);
        if let Some(Value::String(v)) = &cfg.version {
            report.schema_versions.insert("layout".into(), v.clone());
        }
        put_opt(
            &mut report.extra,
            "defaultDrillFilterOtherVisuals",
            cfg.default_drill_filter_other_visuals.as_ref(),
        );
        put_extra(&mut report.extra, "config.", &cfg.extra);
    }
    report.filters = filters(&legacy.filters, path, "/filters", &mut ctx);
    report.resource_packages = layout
        .resource_packages
        .iter()
        .flatten()
        .filter_map(|e| e.resource_package.as_ref())
        .map(|p| {
            let mut extra = PropertyBag::new();
            put_opt(&mut extra, "id", p.id.as_ref());
            put_opt(&mut extra, "disabled", p.disabled.as_ref());
            put_extra(&mut extra, "", &p.extra);
            ResourcePackage {
                name: p.name.clone().unwrap_or_default(),
                kind: enum_name(p.package_type.as_ref(), &PACKAGE_TYPES),
                items: p
                    .items
                    .iter()
                    .flatten()
                    .map(|i| {
                        let mut extra = PropertyBag::new();
                        put_opt(&mut extra, "id", i.id.as_ref());
                        put_extra(&mut extra, "", &i.extra);
                        ResourceItem {
                            name: i.name.clone().unwrap_or_default(),
                            path: i.path.clone().unwrap_or_default(),
                            kind: enum_name(i.item_type.as_ref(), &ITEM_TYPES),
                            extra,
                        }
                    })
                    .collect(),
                extra,
            }
        })
        .collect();

    let empty = LegacySectionParts::default();
    let mut pages: Vec<(f64, Page)> = Vec::new();
    for (i, section) in layout.sections.iter().flatten().enumerate() {
        let parts = legacy.sections.get(i).unwrap_or(&empty);
        #[allow(clippy::cast_precision_loss)]
        let ordinal = f64_or(section.ordinal.as_ref(), i as f64);
        pages.push((ordinal, legacy_page(legacy, i, section, parts, &mut ctx)));
    }
    pages.sort_by(|a, b| a.0.total_cmp(&b.0));
    report.pages = pages.into_iter().map(|(_, p)| p).collect();

    if let Some(index) = legacy
        .config
        .as_ref()
        .and_then(|c| c.active_section_index.as_ref())
        .and_then(num_i64)
        .and_then(|i| usize::try_from(i).ok())
    {
        report.active_page = layout
            .sections
            .as_ref()
            .and_then(|s| s.get(index))
            .and_then(|s| s.name.clone());
    }
    (report, ctx.diags)
}

fn legacy_page(
    legacy: &LegacyReport,
    index: usize,
    section: &LegacySectionJson,
    parts: &LegacySectionParts,
    ctx: &mut Ctx,
) -> Page {
    let path = legacy.path.as_str();
    let pointer = format!("/sections/{index}");
    let config = parts.config.as_ref();
    let mut extra = PropertyBag::new();
    put_opt(&mut extra, "id", section.id.as_ref());
    put_opt(&mut extra, "ordinal", section.ordinal.as_ref());
    put_extra(&mut extra, "", &section.extra);
    if let Some(c) = config {
        put_extra(&mut extra, "config.", &c.extra);
    }
    let empty = LegacyVisualParts::default();
    let visuals = section
        .visual_containers
        .iter()
        .flatten()
        .enumerate()
        .map(|(j, vc)| {
            let vparts = parts.visuals.get(j).unwrap_or(&empty);
            legacy_visual(
                legacy,
                &format!("{pointer}/visualContainers/{j}"),
                vc,
                vparts,
                ctx,
            )
        })
        .collect();
    Page {
        name: section.name.clone().unwrap_or_default(),
        display_name: section.display_name.clone().unwrap_or_default(),
        width: f64_or(section.width.as_ref(), 1280.0),
        height: f64_or(section.height.as_ref(), 720.0),
        display_option: enum_name(section.display_option.as_ref(), &DISPLAY_OPTIONS)
            .map(|s| DisplayOption::parse(&s))
            .unwrap_or_default(),
        filters: filters(&parts.filters, path, &format!("{pointer}/filters"), ctx),
        visuals,
        visual_interactions: config
            .and_then(|c| c.relationships.as_ref())
            .into_iter()
            .flatten()
            .map(|r| VisualInteraction {
                source: r.source.clone().unwrap_or_default(),
                target: r.target.clone().unwrap_or_default(),
                kind: interaction_kind(
                    &enum_name(r.interaction_type.as_ref(), &INTERACTIONS)
                        .unwrap_or_else(|| "Default".into()),
                ),
            })
            .collect(),
        objects: config
            .and_then(|c| c.objects.clone())
            .unwrap_or(Value::Null),
        is_hidden: config.and_then(|c| c.visibility.as_ref()).and_then(num_i64) == Some(1),
        extra,
        origin: Some(Origin::file(path).with_pointer(pointer)),
        ..Page::default()
    }
}

fn legacy_visual(
    legacy: &LegacyReport,
    pointer: &str,
    vc: &LegacyVisualContainerJson,
    parts: &LegacyVisualParts,
    ctx: &mut Ctx,
) -> Visual {
    let path = legacy.path.as_str();
    let origin = Origin::file(path).with_pointer(pointer);
    let config = parts.config.as_ref();
    let layout_pos = config
        .and_then(|c| c.layouts.as_ref())
        .and_then(|l| l.first())
        .and_then(|l| l.position.as_ref());
    let mut out = Visual {
        name: config.and_then(|c| c.name.clone()).unwrap_or_default(),
        position: Position {
            x: f64_or(vc.x.as_ref(), 0.0),
            y: f64_or(vc.y.as_ref(), 0.0),
            z: f64_or(vc.z.as_ref(), 0.0),
            width: f64_or(vc.width.as_ref(), 0.0),
            height: f64_or(vc.height.as_ref(), 0.0),
            tab_order: vc
                .tab_order
                .as_ref()
                .and_then(num_i64)
                .or_else(|| layout_pos.and_then(PositionJson::tab_order_i64)),
            angle: layout_pos.and_then(PositionJson::angle_f64),
        },
        parent_group_name: config.and_then(|c| c.parent_group_name.clone()),
        filters: filters(&parts.filters, path, &format!("{pointer}/filters"), ctx),
        origin: Some(origin.clone()),
        ..Visual::default()
    };
    put_opt(&mut out.extra, "id", vc.id.as_ref());
    put_opt(&mut out.extra, "query", parts.query.as_ref());
    put_opt(
        &mut out.extra,
        "dataTransforms",
        parts.data_transforms.as_ref(),
    );
    put_extra(&mut out.extra, "", &vc.extra);
    let Some(config) = config else {
        return out;
    };
    put_extra(&mut out.extra, "config.", &config.extra);
    if let Some(group) = &config.single_visual_group {
        out.kind = VisualKind::Group;
        out.display_name = group.display_name.clone();
        out.objects = group.objects.clone().unwrap_or(Value::Null);
        out.is_hidden = group.is_hidden.unwrap_or(false);
        if let Some(mode) = enum_name(group.group_mode.as_ref(), &GROUP_MODES) {
            put_opt(&mut out.extra, "groupMode", Some(&mode));
        }
        put_extra(&mut out.extra, "visualGroup.", &group.extra);
    }
    if let Some(sv) = &config.single_visual {
        out.visual_type = sv.visual_type.clone().unwrap_or_default();
        out.objects = sv.objects.clone().unwrap_or(Value::Null);
        out.container_objects = sv.vc_objects.clone().unwrap_or(Value::Null);
        out.is_hidden |= sv
            .display
            .as_ref()
            .and_then(|d| d.get("mode"))
            .and_then(Value::as_str)
            .is_some_and(|m| m.eq_ignore_ascii_case("hidden"));
        put_opt(
            &mut out.extra,
            "drillFilterOtherVisuals",
            sv.drill_filter_other_visuals.as_ref(),
        );
        put_opt(
            &mut out.extra,
            "isDefaultSort",
            sv.has_default_sort.as_ref(),
        );
        put_extra(&mut out.extra, "singleVisual.", &sv.extra);

        let query = sv.prototype_query.as_ref();
        let aliases = query.map(QueryDefinitionJson::aliases).unwrap_or_default();
        let select: &[QueryExpr] = query.and_then(|q| q.select.as_deref()).unwrap_or_default();
        for (role, list) in sv.projections.iter().flatten() {
            for p in list {
                let query_ref = p.query_ref.clone().unwrap_or_default();
                let item = select
                    .iter()
                    .find(|s| s.name.as_deref() == Some(query_ref.as_str()));
                let field = match item {
                    Some(item) => field_expr(item, &aliases, &origin, ctx),
                    None => {
                        ctx.diags.push(
                            Diagnostic::warning(
                                codes::UNRESOLVED_QUERY_REF,
                                format!(
                                    "projection `{query_ref}` has no prototypeQuery select item"
                                ),
                            )
                            .with_origin(origin.clone()),
                        );
                        FieldExpr::default()
                    }
                };
                let display_name = sv
                    .column_properties
                    .as_ref()
                    .and_then(|c| c.get(&query_ref))
                    .and_then(|c| c.display_name.clone());
                let mut extra = PropertyBag::new();
                put_extra(&mut extra, "", &p.extra);
                out.projections.push(Projection {
                    role: role.clone(),
                    field,
                    native_query_ref: item.and_then(|i| i.native_reference_name.clone()),
                    query_ref,
                    display_name,
                    active: p.active,
                    extra,
                });
            }
        }
        for clause in query
            .and_then(|q| q.order_by.as_ref())
            .into_iter()
            .flatten()
        {
            let Some(expr) = &clause.expression else {
                continue;
            };
            let descending = clause.direction.as_ref().and_then(num_i64) == Some(2);
            out.sort.push(SortItem {
                field: field_expr(expr, &aliases, &origin, ctx),
                direction: if descending {
                    SortDirection::Descending
                } else {
                    SortDirection::Ascending
                },
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf8_and_utf16() {
        let text = r#"{"sections":[]}"#;
        assert_eq!(decode_text(text.as_bytes()).unwrap().1, TextEncoding::Utf8);
        let le: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let (decoded, enc) = decode_text(&le).unwrap();
        assert_eq!((decoded.as_str(), enc), (text, TextEncoding::Utf16Le));
        let mut bom = vec![0xFF, 0xFE];
        bom.extend_from_slice(&le);
        assert_eq!(decode_text(&bom).unwrap().1, TextEncoding::Utf16LeBom);
        let be: Vec<u8> = [0xFE, 0xFF]
            .into_iter()
            .chain(text.encode_utf16().flat_map(u16::to_be_bytes))
            .collect();
        assert_eq!(decode_text(&be).unwrap().0, text);
        assert!(decode_text(&le[..le.len() - 1]).is_err());
        assert!(decode_text(&[0xFF, 0xFE, 0x00, 0xD8]).is_err());
        assert!(decode_text(&[0xC3, 0x28]).is_err());
    }

    #[test]
    fn enum_codes_map_to_names() {
        assert_eq!(
            enum_name(Some(&serde_json::json!(1)), &DISPLAY_OPTIONS).as_deref(),
            Some("FitToPage")
        );
        assert_eq!(
            enum_name(Some(&serde_json::json!(9)), &DISPLAY_OPTIONS).as_deref(),
            Some("9")
        );
        assert_eq!(
            enum_name(Some(&serde_json::json!("FitToWidth")), &DISPLAY_OPTIONS).as_deref(),
            Some("FitToWidth")
        );
    }
}
