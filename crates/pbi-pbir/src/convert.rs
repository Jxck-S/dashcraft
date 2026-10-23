//! Conversion of PBIR (and, via shared helpers, legacy layouts) into the
//! format-agnostic report AST of `pbi-model`.

use pbi_model::{
    AggregationFunction, Annotation, ComparisonKind, Condition, DatasetReference, Diagnostic,
    DisplayOption, FieldExpr, Filter, FilterKind, InteractionKind, Literal, Origin, Page, Position,
    Projection, PropValue, PropertyBag, PropertyVariation, Report, ResourceItem, ResourcePackage,
    ResourceRef, SortDirection, SortItem, ThemeRef, Visual, VisualInteraction, VisualKind,
};
use serde::Serialize;
use serde_json::Value;

use crate::error::codes;
use crate::json::{JsonMap, num_f64, num_i64};
use crate::pbir::{
    AnnotationJson, DefinitionPbir, FilterJson, ResourcePackageJson, ThemeCollectionJson,
    ThemeMetadataJson, schema_kind_version,
};
use crate::query::{ExprKind, QueryExpr, SourceAliases};
use crate::reader::{DEFINITION_PBIR, PbirPage, PbirReport, PbirVisual, REPORT_JSON, VERSION_JSON};

/// Collects conversion diagnostics.
#[derive(Debug, Default)]
pub(crate) struct Ctx {
    pub diags: Vec<Diagnostic>,
}

impl Ctx {
    fn warn(&mut self, code: &str, message: String, origin: &Origin) {
        self.diags
            .push(Diagnostic::warning(code, message).with_origin(origin.clone()));
    }
}

/// Verbatim JSON of a typed value (`Null` if it cannot be serialized).
pub(crate) fn raw<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// Adds `prefix + key → value` for every entry of `map`.
pub(crate) fn put_extra(bag: &mut PropertyBag, prefix: &str, map: &JsonMap) {
    for (k, v) in map {
        bag.insert(format!("{prefix}{k}"), PropValue::Json(v.clone()));
    }
}

/// Adds `key → value` if `value` is present.
pub(crate) fn put_opt<T: Serialize>(bag: &mut PropertyBag, key: &str, value: Option<&T>) {
    if let Some(v) = value {
        bag.insert(key.to_owned(), PropValue::Json(raw(v)));
    }
}

pub(crate) fn annotations(list: Option<&Vec<AnnotationJson>>) -> Vec<Annotation> {
    list.into_iter()
        .flatten()
        .map(|a| Annotation {
            name: a.name.clone().unwrap_or_default(),
            value: a.value.clone().unwrap_or_default(),
        })
        .collect()
}

/// Table of a `SourceRef` expression, resolving `Source` aliases.
fn source_entity(
    expr: &QueryExpr,
    aliases: &SourceAliases,
    origin: &Origin,
    ctx: &mut Ctx,
) -> Option<String> {
    let source_ref = expr.source_ref.as_ref()?;
    if let Some(entity) = &source_ref.entity {
        return Some(entity.clone());
    }
    let alias = source_ref.source.as_deref()?;
    match aliases.get(alias) {
        Some(entity) => Some(entity.to_owned()),
        None => {
            ctx.warn(
                codes::UNRESOLVED_ALIAS,
                format!("source alias `{alias}` is not declared in From"),
                origin,
            );
            None
        }
    }
}

/// Converts a semantic-query field expression. Unsupported shapes become
/// [`FieldExpr::Unknown`] with the source JSON.
pub(crate) fn field_expr(
    expr: &QueryExpr,
    aliases: &SourceAliases,
    origin: &Origin,
    ctx: &mut Ctx,
) -> FieldExpr {
    let unknown = || FieldExpr::Unknown(raw(expr));
    match expr.kind() {
        ExprKind::Column(p) | ExprKind::Measure(p) => {
            let entity = p
                .expression
                .as_deref()
                .and_then(|e| source_entity(e, aliases, origin, ctx));
            match (entity, &p.property) {
                (Some(entity), Some(property)) if expr.column.is_some() => {
                    FieldExpr::column(entity, property.clone())
                }
                (Some(entity), Some(property)) => FieldExpr::measure(entity, property.clone()),
                _ => unknown(),
            }
        }
        ExprKind::Aggregation(a) => {
            let code = a.function.as_ref().and_then(num_i64);
            match (a.expression.as_deref(), code) {
                (Some(inner), Some(code)) => FieldExpr::aggregation(
                    field_expr(inner, aliases, origin, ctx),
                    AggregationFunction::from_code(code),
                ),
                _ => unknown(),
            }
        }
        ExprKind::HierarchyLevel(hl) => {
            hierarchy_level(hl, aliases, origin, ctx).unwrap_or_else(unknown)
        }
        ExprKind::Literal(l) => match &l.value {
            Some(v) => FieldExpr::Literal(Literal::new(v.clone())),
            None => unknown(),
        },
        _ => unknown(),
    }
}

fn hierarchy_level(
    hl: &crate::query::HierarchyLevelExpr,
    aliases: &SourceAliases,
    origin: &Origin,
    ctx: &mut Ctx,
) -> Option<FieldExpr> {
    let level = hl.level.clone()?;
    let h = hl.expression.as_deref()?.hierarchy.as_ref()?;
    let hierarchy = h.hierarchy.clone()?;
    let source = h.expression.as_deref()?;
    let (entity, variation) = match source.kind() {
        ExprKind::SourceRef(_) => (source_entity(source, aliases, origin, ctx)?, None),
        ExprKind::PropertyVariationSource(pvs) => (
            source_entity(pvs.expression.as_deref()?, aliases, origin, ctx)?,
            Some(PropertyVariation {
                name: pvs.name.clone().unwrap_or_default(),
                property: pvs.property.clone().unwrap_or_default(),
            }),
        ),
        _ => return None,
    };
    Some(FieldExpr::HierarchyLevel {
        entity,
        hierarchy,
        level,
        variation,
    })
}

/// Converts a filter condition. Unsupported shapes become
/// [`Condition::Unknown`] with the source JSON.
pub(crate) fn condition(
    expr: &QueryExpr,
    aliases: &SourceAliases,
    origin: &Origin,
    ctx: &mut Ctx,
) -> Condition {
    let unknown = || Condition::Unknown(raw(expr));
    let fe = |e: &QueryExpr, ctx: &mut Ctx| field_expr(e, aliases, origin, ctx);
    match expr.kind() {
        ExprKind::In(i) => {
            if i.table.is_some() {
                return unknown();
            }
            let Some(expressions) = &i.expressions else {
                return unknown();
            };
            let mut values = Vec::new();
            for row in i.values.iter().flatten() {
                let mut tuple = Vec::with_capacity(row.len());
                for v in row {
                    match v.literal.as_ref().and_then(|l| l.value.as_ref()) {
                        Some(text) => tuple.push(Literal::new(text.clone())),
                        None => return unknown(),
                    }
                }
                values.push(tuple);
            }
            Condition::In {
                exprs: expressions.iter().map(|e| fe(e, ctx)).collect(),
                values,
            }
        }
        ExprKind::Comparison(c) => {
            let kind = c
                .comparison_kind
                .as_ref()
                .and_then(num_i64)
                .and_then(ComparisonKind::from_code);
            match (kind, c.left.as_deref(), c.right.as_deref()) {
                (Some(kind), Some(l), Some(r)) => Condition::Comparison {
                    kind,
                    left: fe(l, ctx),
                    right: fe(r, ctx),
                },
                _ => unknown(),
            }
        }
        ExprKind::And(b) | ExprKind::Or(b) => match (b.left.as_deref(), b.right.as_deref()) {
            (Some(l), Some(r)) => {
                let l = Box::new(condition(l, aliases, origin, ctx));
                let r = Box::new(condition(r, aliases, origin, ctx));
                if expr.and.is_some() {
                    Condition::And(l, r)
                } else {
                    Condition::Or(l, r)
                }
            }
            _ => unknown(),
        },
        ExprKind::Not(u) => match u.expression.as_deref() {
            Some(inner) => Condition::Not(Box::new(condition(inner, aliases, origin, ctx))),
            None => unknown(),
        },
        ExprKind::Contains(b) | ExprKind::StartsWith(b) => {
            match (b.left.as_deref(), b.right.as_deref()) {
                (Some(l), Some(r)) => {
                    let left = fe(l, ctx);
                    let right = fe(r, ctx);
                    if expr.contains.is_some() {
                        Condition::Contains { left, right }
                    } else {
                        Condition::StartsWith { left, right }
                    }
                }
                _ => unknown(),
            }
        }
        ExprKind::Between(b) => match (
            b.expression.as_deref(),
            b.lower_bound.as_deref(),
            b.upper_bound.as_deref(),
        ) {
            (Some(e), Some(lo), Some(hi)) => Condition::Between {
                expr: fe(e, ctx),
                lower: fe(lo, ctx),
                upper: fe(hi, ctx),
            },
            _ => unknown(),
        },
        _ => unknown(),
    }
}

/// Legacy numeric `howCreated` codes (schema enum order).
const HOW_CREATED: [&str; 6] = [
    "Auto",
    "User",
    "Drill",
    "Include",
    "Exclude",
    "Drillthrough",
];

fn how_created(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(
            num_i64(n)
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| HOW_CREATED.get(i))
                .map_or_else(|| n.to_string(), |s| (*s).to_owned()),
        ),
        _ => None,
    }
}

/// Converts a filter card. `origin` should point at the filter itself.
pub(crate) fn filter(f: &FilterJson, origin: Origin, ctx: &mut Ctx) -> Filter {
    let aliases = f
        .filter
        .as_ref()
        .map(crate::query::QueryDefinitionJson::aliases)
        .unwrap_or_default();
    let field = f
        .field
        .as_ref()
        .or(f.expression.as_ref())
        .map(|e| field_expr(e, &aliases, &origin, ctx));
    let condition = f.filter.as_ref().and_then(|def| {
        let conditions: Vec<Condition> = def
            .where_
            .iter()
            .flatten()
            .filter_map(|w| w.condition.as_ref())
            .map(|c| condition(c, &aliases, &origin, ctx))
            .collect();
        Condition::and_all(conditions)
    });
    Filter {
        name: f.name.clone().unwrap_or_default(),
        display_name: f.display_name.clone(),
        field,
        kind: f
            .filter_type
            .as_deref()
            .map(FilterKind::parse)
            .unwrap_or_default(),
        condition,
        is_hidden_in_view_mode: f.is_hidden_in_view_mode.unwrap_or(false),
        is_locked_in_view_mode: f.is_locked_in_view_mode.unwrap_or(false),
        how_created: how_created(f.how_created.as_ref()),
        raw: raw(f),
        origin: Some(origin),
        ..Filter::default()
    }
}

/// Converts a list of filters whose JSON pointer is `{pointer}/{index}`.
pub(crate) fn filters(
    list: &[FilterJson],
    path: &str,
    pointer: &str,
    ctx: &mut Ctx,
) -> Vec<Filter> {
    list.iter()
        .enumerate()
        .map(|(i, f)| {
            filter(
                f,
                Origin::file(path).with_pointer(format!("{pointer}/{i}")),
                ctx,
            )
        })
        .collect()
}

pub(crate) fn interaction_kind(text: &str) -> InteractionKind {
    InteractionKind::parse(text)
}

pub(crate) fn sort_direction(text: &str) -> SortDirection {
    if text.eq_ignore_ascii_case("descending") {
        SortDirection::Descending
    } else {
        SortDirection::Ascending
    }
}

fn theme_ref(meta: Option<&ThemeMetadataJson>) -> Option<ResourceRef> {
    let meta = meta?;
    let mut extra = PropertyBag::new();
    put_opt(
        &mut extra,
        "reportVersionAtImport",
        meta.report_version_at_import.as_ref(),
    );
    put_extra(&mut extra, "", &meta.extra);
    Some(ResourceRef {
        name: meta.name.clone().unwrap_or_default(),
        kind: meta.package_type.clone(),
        extra,
    })
}

fn theme_collection(tc: Option<&ThemeCollectionJson>) -> ThemeRef {
    ThemeRef {
        base_theme: theme_ref(tc.and_then(|t| t.base_theme.as_ref())),
        custom_theme: theme_ref(tc.and_then(|t| t.custom_theme.as_ref())),
    }
}

fn resource_package(p: &ResourcePackageJson) -> ResourcePackage {
    let mut extra = PropertyBag::new();
    put_opt(&mut extra, "id", p.id.as_ref());
    put_opt(&mut extra, "disabled", p.disabled.as_ref());
    put_extra(&mut extra, "", &p.extra);
    ResourcePackage {
        name: p.name.clone().unwrap_or_default(),
        kind: p.package_type.clone(),
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
                    kind: i.item_type.clone(),
                    extra,
                }
            })
            .collect(),
        extra,
    }
}

fn dataset_reference(def: &DefinitionPbir) -> DatasetReference {
    if let Some(path) = def.by_path() {
        return DatasetReference::ByPath {
            path: path.to_owned(),
        };
    }
    match def.by_connection() {
        Some(c) => DatasetReference::ByConnection {
            connection_string: c.connection_string.clone(),
            model_id: c.pbi_service_model_id.as_ref().and_then(|v| match v {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            }),
            database_name: c.pbi_model_database_name.clone(),
            connection_type: c.connection_type.clone(),
            name: c.name.clone(),
            raw: raw(c),
        },
        None => DatasetReference::None,
    }
}

fn schema_version(versions: &mut indexmap::IndexMap<String, String>, schema: Option<&String>) {
    if let Some((kind, version)) = schema.and_then(|s| schema_kind_version(s)) {
        versions.insert(kind.to_owned(), version.to_owned());
    }
}

/// Converts a PBIR report into the report AST.
///
/// - `name` comes from `.platform` (empty if absent; `pbi-core` falls back
///   to the folder name).
/// - `dataset_reference` comes from `definition.pbir` (`None` if absent,
///   e.g. inside a PBIX, where `pbi-core` sets `Embedded`).
/// - IDs are left default; `pbi_model::Document::assign_ids` assigns them.
///
/// Returns conversion diagnostics only; read diagnostics are in
/// [`PbirReport::diagnostics`].
pub fn to_report_ast(pbir: &PbirReport) -> (Report, Vec<Diagnostic>) {
    let mut ctx = Ctx::default();
    let mut report = Report::default();

    if let Some(platform) = &pbir.platform {
        let meta = platform.metadata.as_ref();
        report.name = meta
            .and_then(|m| m.display_name.clone())
            .unwrap_or_default();
        schema_version(&mut report.schema_versions, platform.schema.as_ref());
        put_opt(
            &mut report.extra,
            "logicalId",
            platform.config.as_ref().and_then(|c| c.logical_id.as_ref()),
        );
    }
    if let Some(def) = &pbir.definition_pbir {
        report.dataset_reference = dataset_reference(def);
        if let Some(v) = &def.version {
            report
                .schema_versions
                .insert(DEFINITION_PBIR.to_owned(), v.clone());
        }
        schema_version(&mut report.schema_versions, def.schema.as_ref());
    }
    if let Some(rj) = &pbir.report {
        report.schema = rj.schema.clone();
        schema_version(&mut report.schema_versions, rj.schema.as_ref());
        report.theme = theme_collection(rj.theme_collection.as_ref());
        report.filters = filters(rj.filters(), REPORT_JSON, "/filterConfig/filters", &mut ctx);
        report.settings = rj.settings.clone().unwrap_or(Value::Null);
        report.objects = rj.objects.clone().unwrap_or(Value::Null);
        report.resource_packages = rj
            .resource_packages
            .iter()
            .flatten()
            .map(resource_package)
            .collect();
        report.annotations = annotations(rj.annotations.as_ref());
        let extra = &mut report.extra;
        put_opt(extra, "reportSource", rj.report_source.as_ref());
        put_opt(
            extra,
            "publicCustomVisuals",
            rj.public_custom_visuals.as_ref(),
        );
        put_opt(
            extra,
            "organizationCustomVisuals",
            rj.organization_custom_visuals.as_ref(),
        );
        put_opt(
            extra,
            "dataSourceVariables",
            rj.data_source_variables.as_ref(),
        );
        put_opt(
            extra,
            "slowDataSourceSettings",
            rj.slow_data_source_settings.as_ref(),
        );
        if let Some(fc) = &rj.filter_config {
            put_opt(extra, "filterSortOrder", fc.filter_sort_order.as_ref());
        }
        put_extra(extra, "", &rj.extra);
        report.origin = Some(Origin::file(REPORT_JSON));
    }
    if let Some(version) = &pbir.version {
        schema_version(&mut report.schema_versions, version.schema.as_ref());
        if let Some(v) = &version.version {
            report
                .schema_versions
                .insert(VERSION_JSON.to_owned(), v.clone());
        }
    }
    if let Some(meta) = &pbir.pages_meta {
        schema_version(&mut report.schema_versions, meta.schema.as_ref());
        report.active_page = meta.active_page_name.clone();
        put_opt(
            &mut report.extra,
            "landingPageName",
            meta.landing_page_name.as_ref(),
        );
        put_extra(&mut report.extra, "pages.", &meta.extra);
    }
    report.pages = pbir.pages.iter().map(|p| page(p, &mut ctx)).collect();
    (report, ctx.diags)
}

fn page(p: &PbirPage, ctx: &mut Ctx) -> Page {
    let path = p.path();
    let pj = &p.page;
    let mut extra = PropertyBag::new();
    put_opt(&mut extra, "pageBinding", pj.page_binding.as_ref());
    put_opt(&mut extra, "type", pj.page_type.as_ref());
    put_opt(&mut extra, "howCreated", pj.how_created.as_ref());
    put_opt(
        &mut extra,
        "autoPageGenerationConfig",
        pj.auto_page_generation_config.as_ref(),
    );
    if let Some(fc) = &pj.filter_config {
        put_opt(&mut extra, "filterSortOrder", fc.filter_sort_order.as_ref());
    }
    let visibility = pj.visibility.as_deref();
    if !matches!(
        visibility,
        None | Some("HiddenInViewMode" | "AlwaysVisible")
    ) {
        put_opt(&mut extra, "visibility", pj.visibility.as_ref());
    }
    put_extra(&mut extra, "", &pj.extra);
    Page {
        name: pj.name.clone().unwrap_or_else(|| p.dir.clone()),
        display_name: pj.display_name.clone().unwrap_or_default(),
        schema: pj.schema.clone(),
        width: pj.width.as_ref().and_then(num_f64).unwrap_or(1280.0),
        height: pj.height.as_ref().and_then(num_f64).unwrap_or(720.0),
        display_option: pj
            .display_option
            .as_deref()
            .map(DisplayOption::parse)
            .unwrap_or_default(),
        filters: filters(pj.filters(), &path, "/filterConfig/filters", ctx),
        visuals: p.visuals.iter().map(|v| visual(p, v, ctx)).collect(),
        visual_interactions: pj
            .visual_interactions
            .iter()
            .flatten()
            .map(|vi| VisualInteraction {
                source: vi.source.clone().unwrap_or_default(),
                target: vi.target.clone().unwrap_or_default(),
                kind: interaction_kind(vi.interaction_type.as_deref().unwrap_or("Default")),
            })
            .collect(),
        objects: pj.objects.clone().unwrap_or(Value::Null),
        is_hidden: visibility == Some("HiddenInViewMode"),
        annotations: annotations(pj.annotations.as_ref()),
        extra,
        origin: Some(Origin::file(path)),
        ..Page::default()
    }
}

fn visual(page: &PbirPage, v: &PbirVisual, ctx: &mut Ctx) -> Visual {
    let path = page.visual_path(v);
    let origin = Origin::file(path.clone());
    let vc = &v.visual;
    let mut out = Visual {
        name: vc.name.clone().unwrap_or_else(|| v.dir.clone()),
        schema: vc.schema.clone(),
        is_hidden: vc.is_hidden.unwrap_or(false),
        parent_group_name: vc.parent_group_name.clone(),
        annotations: annotations(vc.annotations.as_ref()),
        filters: filters(vc.filters(), &path, "/filterConfig/filters", ctx),
        origin: Some(origin.clone()),
        ..Visual::default()
    };
    if let Some(pos) = &vc.position {
        out.position = Position {
            x: pos.x_f64(),
            y: pos.y_f64(),
            z: pos.z_f64(),
            width: pos.width_f64(),
            height: pos.height_f64(),
            tab_order: pos.tab_order_i64(),
            angle: pos.angle_f64(),
        };
        put_extra(&mut out.extra, "position.", &pos.extra);
    }
    if let Some(group) = &vc.visual_group {
        out.kind = VisualKind::Group;
        out.display_name = group.display_name.clone();
        out.objects = group.objects.clone().unwrap_or(Value::Null);
        put_opt(&mut out.extra, "groupMode", group.group_mode.as_ref());
        put_extra(&mut out.extra, "visualGroup.", &group.extra);
    }
    if let Some(vj) = &vc.visual {
        out.visual_type = vj.visual_type.clone().unwrap_or_default();
        out.objects = vj.objects.clone().unwrap_or(Value::Null);
        out.container_objects = vj.visual_container_objects.clone().unwrap_or(Value::Null);
        let extra = &mut out.extra;
        put_opt(
            extra,
            "autoSelectVisualType",
            vj.auto_select_visual_type.as_ref(),
        );
        put_opt(extra, "expansionStates", vj.expansion_states.as_ref());
        put_opt(extra, "syncGroup", vj.sync_group.as_ref());
        put_opt(
            extra,
            "drillFilterOtherVisuals",
            vj.drill_filter_other_visuals.as_ref(),
        );
        put_extra(extra, "visual.", &vj.extra);
        if let Some(query) = &vj.query {
            let none = SourceAliases::default();
            for (role, state) in query.query_state.iter().flatten() {
                for p in state.projections.iter().flatten() {
                    let mut pextra = PropertyBag::new();
                    put_opt(&mut pextra, "format", p.format.as_ref());
                    put_opt(&mut pextra, "hidden", p.hidden.as_ref());
                    put_extra(&mut pextra, "", &p.extra);
                    out.projections.push(Projection {
                        role: role.clone(),
                        field: p
                            .field
                            .as_ref()
                            .map(|f| field_expr(f, &none, &origin, ctx))
                            .unwrap_or_default(),
                        query_ref: p.query_ref.clone().unwrap_or_default(),
                        native_query_ref: p.native_query_ref.clone(),
                        display_name: p.display_name.clone(),
                        active: p.active,
                        extra: pextra,
                    });
                }
                put_opt(
                    &mut out.extra,
                    &format!("queryState.{role}.showAll"),
                    state.show_all.as_ref(),
                );
                put_opt(
                    &mut out.extra,
                    &format!("queryState.{role}.fieldParameters"),
                    state.field_parameters.as_ref(),
                );
                put_extra(&mut out.extra, &format!("queryState.{role}."), &state.extra);
            }
            if let Some(sd) = &query.sort_definition {
                for s in sd.sort.iter().flatten() {
                    out.sort.push(SortItem {
                        field: s
                            .field
                            .as_ref()
                            .map(|f| field_expr(f, &none, &origin, ctx))
                            .unwrap_or_default(),
                        direction: sort_direction(s.direction.as_deref().unwrap_or_default()),
                    });
                }
                put_opt(&mut out.extra, "isDefaultSort", sd.is_default_sort.as_ref());
                put_extra(&mut out.extra, "sortDefinition.", &sd.extra);
            }
            put_opt(&mut out.extra, "query.options", query.options.as_ref());
            put_opt(
                &mut out.extra,
                "query.isDrillDisabled",
                query.is_drill_disabled.as_ref(),
            );
            put_extra(&mut out.extra, "query.", &query.extra);
        }
    }
    put_opt(&mut out.extra, "howCreated", vc.how_created.as_ref());
    if let Some(fc) = &vc.filter_config {
        put_opt(
            &mut out.extra,
            "filterSortOrder",
            fc.filter_sort_order.as_ref(),
        );
    }
    put_extra(&mut out.extra, "", &vc.extra);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cond(json: &str, from: &str) -> (Condition, Vec<Diagnostic>) {
        let expr: QueryExpr = serde_json::from_str(json).unwrap();
        let def: crate::query::QueryDefinitionJson = serde_json::from_str(from).unwrap();
        let mut ctx = Ctx::default();
        let c = condition(&expr, &def.aliases(), &Origin::file("x"), &mut ctx);
        (c, ctx.diags)
    }

    #[test]
    fn conditions_resolve_aliases_and_operators() {
        let from = r#"{"From":[{"Name":"a","Entity":"Stores","Type":0}]}"#;
        let (c, d) = cond(
            r#"{"Not":{"Expression":{"Comparison":{"ComparisonKind":2,"Left":{"Column":{"Expression":{"SourceRef":{"Source":"a"}},"Property":"Square Feet"}},"Right":{"Literal":{"Value":"5L"}}}}}}"#,
            from,
        );
        assert!(d.is_empty());
        assert_eq!(
            c,
            Condition::Not(Box::new(Condition::Comparison {
                kind: ComparisonKind::GreaterThanOrEqual,
                left: FieldExpr::column("Stores", "Square Feet"),
                right: FieldExpr::Literal(Literal::new("5L")),
            }))
        );

        let (c, d) = cond(
            r#"{"Or":{"Left":{"Contains":{"Left":{"Column":{"Expression":{"SourceRef":{"Source":"b"}},"Property":"X"}},"Right":{"Literal":{"Value":"'q'"}}}},"Right":{"DateSpan":{}}}}"#,
            from,
        );
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].code, codes::UNRESOLVED_ALIAS);
        let Condition::Or(l, r) = c else {
            panic!("expected Or")
        };
        assert!(matches!(
            *l,
            Condition::Contains {
                left: FieldExpr::Unknown(_),
                ..
            }
        ));
        assert!(matches!(*r, Condition::Unknown(_)));
    }

    #[test]
    fn hierarchy_levels_and_aggregations() {
        let expr: QueryExpr = serde_json::from_str(
            r#"{"HierarchyLevel":{"Expression":{"Hierarchy":{"Expression":{"PropertyVariationSource":{"Expression":{"SourceRef":{"Entity":"Sales"}},"Name":"Variation","Property":"OrderDate"}},"Hierarchy":"Date Hierarchy"}},"Level":"Year"}}"#,
        )
        .unwrap();
        let mut ctx = Ctx::default();
        let f = field_expr(
            &expr,
            &SourceAliases::default(),
            &Origin::default(),
            &mut ctx,
        );
        assert_eq!(
            f.query_ref().as_deref(),
            Some("Sales.OrderDate.Date Hierarchy.Year")
        );
        let agg: QueryExpr = serde_json::from_str(
            r#"{"Aggregation":{"Expression":{"Column":{"Expression":{"SourceRef":{"Entity":"T"}},"Property":"C"}},"Function":2}}"#,
        )
        .unwrap();
        let f = field_expr(
            &agg,
            &SourceAliases::default(),
            &Origin::default(),
            &mut ctx,
        );
        assert_eq!(f.query_ref().as_deref(), Some("Count(T.C)"));
        assert!(ctx.diags.is_empty());
    }

    #[test]
    fn legacy_how_created_codes_map_to_names() {
        assert_eq!(
            how_created(Some(&serde_json::json!(1))).as_deref(),
            Some("User")
        );
        assert_eq!(
            how_created(Some(&serde_json::json!(42))).as_deref(),
            Some("42")
        );
        assert_eq!(
            how_created(Some(&serde_json::json!("Auto"))).as_deref(),
            Some("Auto")
        );
    }
}
