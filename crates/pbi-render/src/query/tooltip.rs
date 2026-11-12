//! Report-page tooltips: a small page shown when hovering a data point.
//!
//! # Where tooltips live in PBIR
//!
//! - A tooltip page has `"type": "Tooltip"` and/or a `pageBinding` of
//!   type `Tooltip`. Its tooltip fields are page filters bound by the
//!   binding's `parameters` (`boundFilter`) or created as `Drillthrough`
//!   or `Tooltip` filters.
//! - A visual picks its tooltip with the `visualTooltip` object
//!   (`visualContainerObjects`, else `objects`): `show`, `type`
//!   (`ReportPage` or `Default`) and `section` (the page name).
//!
//! # What DashCraft does
//!
//! - [`visual_tooltip_page`] finds the page: none when `show` is false or
//!   `type` is `Default`; the `section` page when set; otherwise (Desktop's
//!   "Auto") the first tooltip page whose tooltip fields all have a value in
//!   the hovered point.
//! - [`tooltip_state`] builds the page's state: the hovered point (all its
//!   fields) filters every visual of the tooltip page, and the source
//!   page's filters, slicer selections, cross-filter and the source
//!   visual's own filters are kept, like a drillthrough with "Keep all
//!   filters" ([`super::drillthrough`]).

use pbi_model::Report;

use super::drillthrough::{DrillthroughContext, DrillthroughField, carried_filters, is_kind};
use super::interaction::{FilterLevel, InteractionState, PointField, PointSelection};
use super::plan::same_field;
use crate::theme::{as_bool, as_text};

/// A tooltip page of a report.
#[derive(Debug, Clone, PartialEq)]
pub struct TooltipPage {
    /// Index of the page in the report.
    pub index: usize,
    pub name: String,
    pub display_name: String,
    pub width: f64,
    pub height: f64,
    /// The tooltip fields (page filters) used to match it automatically.
    pub fields: Vec<DrillthroughField>,
}

/// Page `index` of `report` as a tooltip page (`None` when it is not one).
pub fn tooltip_page(report: &Report, index: usize) -> Option<TooltipPage> {
    let page = report.pages.get(index)?;
    let binding = page.extra.get("pageBinding").map(|b| b.to_json());
    let page_type = page.extra.get("type").map(|t| t.to_json());
    let binding_type = binding.as_ref().and_then(|b| b.get("type"));
    if !is_kind(page_type.as_ref(), "Tooltip") && !is_kind(binding_type, "Tooltip") {
        return None;
    }
    let bound: Vec<&str> = binding
        .as_ref()
        .and_then(|b| b.get("parameters"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|p| p.get("boundFilter").and_then(serde_json::Value::as_str))
        .collect();
    let fields = page
        .filters
        .iter()
        .filter(|f| {
            f.how_created.as_deref().is_some_and(|h| {
                h.eq_ignore_ascii_case("Drillthrough") || h.eq_ignore_ascii_case("Tooltip")
            }) || bound.contains(&f.name.as_str())
        })
        .filter_map(|f| {
            Some(DrillthroughField {
                field: f.field.clone()?,
                filter: f.name.clone(),
            })
        })
        .collect();
    Some(TooltipPage {
        index,
        name: page.name.clone(),
        display_name: page.display_name.clone(),
        width: page.width,
        height: page.height,
        fields,
    })
}

/// The `visualTooltip` property `prop` of a visual.
fn visual_tooltip_prop(visual: &pbi_model::Visual, prop: &str) -> Option<serde_json::Value> {
    pbi_pbir::objects::property(&visual.container_objects, "visualTooltip", prop)
        .or_else(|| pbi_pbir::objects::property(&visual.objects, "visualTooltip", prop))
        .cloned()
}

/// The report-page tooltip shown when hovering `point` of visual `visual`
/// on page `source_page` (see the module docs), if any.
pub fn visual_tooltip_page(
    report: &Report,
    source_page: usize,
    visual: &str,
    point: &[PointField],
) -> Option<TooltipPage> {
    let page = report.pages.get(source_page)?;
    let visual = page.visuals.iter().find(|v| v.name == visual)?;
    if visual_tooltip_prop(visual, "show")
        .as_ref()
        .and_then(as_bool)
        == Some(false)
    {
        return None;
    }
    let kind = visual_tooltip_prop(visual, "type")
        .as_ref()
        .and_then(as_text)
        .unwrap_or_default();
    let kind: String = kind
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if !kind.is_empty() && kind != "reportpage" {
        return None;
    }
    let section = visual_tooltip_prop(visual, "section")
        .as_ref()
        .and_then(as_text)
        .filter(|s| !s.trim().is_empty());
    let candidates = (0..report.pages.len()).filter(|&i| i != source_page);
    if let Some(section) = section {
        return candidates
            .filter_map(|i| tooltip_page(report, i))
            .find(|t| t.name == section || t.display_name == section);
    }
    candidates
        .filter_map(|i| tooltip_page(report, i))
        .find(|t| {
            !t.fields.is_empty()
                && t.fields
                    .iter()
                    .all(|f| point.iter().any(|p| same_field(&p.field, &f.field)))
        })
}

/// The state of tooltip page `target` for hovering `point` of visual
/// `visual` on page `source_page` (`source_state`); also the parts not
/// applied. The tooltip page's own stored state is not used.
pub fn tooltip_state(
    report: &Report,
    source_page: usize,
    source_state: &InteractionState,
    visual: &str,
    point: &[PointField],
    target: &TooltipPage,
) -> (InteractionState, Vec<String>) {
    let mut state = InteractionState::default();
    if let Some(page) = report.pages.get(target.index) {
        for f in page
            .filters
            .iter()
            .filter(|f| target.fields.iter().any(|d| d.filter == f.name))
        {
            state.set_filter(FilterLevel::Page, f, None);
        }
    }
    let hovered: Vec<DrillthroughField> = point
        .iter()
        .map(|p| DrillthroughField {
            field: p.field.clone(),
            filter: String::new(),
        })
        .collect();
    let mut not_applied = Vec::new();
    let carried = carried_filters(
        report,
        source_page,
        source_state,
        visual,
        &hovered,
        &mut not_applied,
    );
    let points: Vec<Vec<PointField>> = if point.is_empty() {
        Vec::new()
    } else {
        vec![point.to_vec()]
    };
    state.drillthrough = Some(DrillthroughContext {
        source_page: report
            .pages
            .get(source_page)
            .map(|p| p.name.clone())
            .unwrap_or_default(),
        source_report: None,
        selection: PointSelection {
            source: visual.to_owned(),
            points,
        },
        carried,
    });
    (state, not_applied)
}

#[cfg(test)]
mod tests {
    use pbi_engine::Value;
    use pbi_model::{Condition, FieldExpr, Filter, Literal, Page, PropValue, Visual};

    use super::*;

    fn tooltip_object(props: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "visualTooltip": [{ "properties": props }] })
    }

    fn lit(v: serde_json::Value) -> serde_json::Value {
        let text = match &v {
            serde_json::Value::String(s) => format!("'{s}'"),
            other => other.to_string(),
        };
        serde_json::json!({ "expr": { "Literal": { "Value": text } } })
    }

    fn report(objects: serde_json::Value) -> Report {
        let region = FieldExpr::column("Stores", "Region");
        let category = FieldExpr::column("Products", "Category");
        let mut source = Page {
            name: "source".into(),
            ..Page::default()
        };
        source.filters.push(Filter {
            name: "cat".into(),
            field: Some(category.clone()),
            condition: Some(Condition::In {
                exprs: vec![category.clone()],
                values: vec![vec![Literal::text("Camping")]],
            }),
            ..Filter::default()
        });
        source.visuals.push(Visual {
            name: "chart".into(),
            visual_type: "columnChart".into(),
            container_objects: objects,
            filters: vec![Filter {
                name: "vf".into(),
                field: Some(FieldExpr::column("Stores", "City")),
                condition: Some(Condition::In {
                    exprs: vec![FieldExpr::column("Stores", "City")],
                    values: vec![vec![Literal::text("Oslo")]],
                }),
                ..Filter::default()
            }],
            ..Visual::default()
        });
        let mut plain = Page {
            name: "plain".into(),
            display_name: "Plain tooltip".into(),
            width: 320.0,
            height: 240.0,
            ..Page::default()
        };
        plain
            .extra
            .insert("type".into(), PropValue::Json(serde_json::json!("Tooltip")));
        let mut by_region = Page {
            name: "byRegion".into(),
            ..Page::default()
        };
        by_region.extra.insert(
            "pageBinding".into(),
            PropValue::Json(serde_json::json!({
                "name": "b", "type": "Tooltip",
                "parameters": [{ "name": "p", "boundFilter": "tf" }]
            })),
        );
        by_region.filters.push(Filter {
            name: "tf".into(),
            field: Some(region.clone()),
            condition: Some(Condition::In {
                exprs: vec![region],
                values: vec![vec![Literal::text("East")]],
            }),
            ..Filter::default()
        });
        Report {
            pages: vec![source, plain, by_region],
            ..Report::default()
        }
    }

    fn point(fields: &[(&str, &str, &str)]) -> Vec<PointField> {
        fields
            .iter()
            .map(|(t, c, v)| PointField {
                field: FieldExpr::column(*t, *c),
                value: Value::from(*v),
            })
            .collect()
    }

    fn page_for(objects: serde_json::Value, p: &[PointField]) -> Option<String> {
        visual_tooltip_page(&report(objects), 0, "chart", p).map(|t| t.name)
    }

    #[test]
    fn picks_the_section_or_the_automatic_match() {
        let west = point(&[("Stores", "Region", "West")]);
        let explicit = tooltip_object(serde_json::json!({
            "show": lit(serde_json::json!(true)), "type": lit(serde_json::json!("ReportPage")),
            "section": lit(serde_json::json!("plain")),
        }));
        assert_eq!(page_for(explicit, &west).as_deref(), Some("plain"));
        // No visualTooltip: Auto, the page whose tooltip fields are in the point.
        assert_eq!(
            page_for(serde_json::Value::Null, &west).as_deref(),
            Some("byRegion")
        );
        assert_eq!(
            page_for(
                serde_json::Value::Null,
                &point(&[("Stores", "City", "Oslo")])
            ),
            None
        );
        let default =
            tooltip_object(serde_json::json!({ "type": lit(serde_json::json!("Default")) }));
        assert_eq!(page_for(default, &west), None);
        let hidden = tooltip_object(serde_json::json!({
            "show": lit(serde_json::json!(false)), "section": lit(serde_json::json!("plain")),
        }));
        assert_eq!(page_for(hidden, &west), None);
        let missing =
            tooltip_object(serde_json::json!({ "section": lit(serde_json::json!("nope")) }));
        assert_eq!(page_for(missing, &west), None);
        assert!(tooltip_page(&report(serde_json::Value::Null), 0).is_none());
    }

    #[test]
    fn the_state_filters_by_the_point_and_keeps_the_source_context() {
        let r = report(serde_json::Value::Null);
        let hovered = point(&[("Stores", "Region", "West"), ("Stores", "City", "Oslo")]);
        let Some(target) = visual_tooltip_page(&r, 0, "chart", &hovered) else {
            panic!("no tooltip page");
        };
        let (state, not_applied) = tooltip_state(
            &r,
            0,
            &InteractionState::default(),
            "chart",
            &hovered,
            &target,
        );
        assert!(not_applied.is_empty());
        let Some(drill) = &state.drillthrough else {
            panic!("no tooltip context");
        };
        assert_eq!(drill.source_page, "source");
        assert_eq!(drill.selection.source, "chart");
        assert_eq!(drill.selection.points, vec![hovered.clone()]);
        // The page filter is kept; the visual filter is on a hovered field.
        assert_eq!(drill.carried.len(), 1);
        // The tooltip field's saved condition is cleared.
        let tip_filter = &r.pages[2].filters[0];
        let applied = state.filters.apply(FilterLevel::Page, tip_filter);
        assert!(applied.condition.is_none());

        let region_only = point(&[("Stores", "Region", "West")]);
        let (state, _) = tooltip_state(
            &r,
            0,
            &InteractionState::default(),
            "chart",
            &region_only,
            &target,
        );
        let carried = state.drillthrough.map(|d| d.carried.len());
        assert_eq!(carried, Some(2), "the page and the visual filter are kept");
    }
}
