//! Drillthrough: report pages that show one item in detail.
//!
//! # Where drillthrough lives in PBIR
//!
//! - A drillthrough page has `"type": "Drillthrough"` and/or a
//!   `pageBinding` of type `Drillthrough`:
//!   `{name, type, referenceScope, parameters: [{name, boundFilter,
//!   fieldExpr, asAggregation}], acceptsFilterContext}`.
//! - Its drillthrough fields are page filter cards created as
//!   `"howCreated": "Drillthrough"` (legacy PBIX code 5); a binding
//!   parameter names its card in `boundFilter`.
//! - `acceptsFilterContext: "None"` is "Keep all filters" off; the default
//!   carries the source's filter context to the page.
//! - `referenceScope: "CrossReport"` offers the page to the other reports
//!   of the workspace when its report allows it (`settings.
//!   useCrossReportDrillthrough`); fields match by table and column name.
//!
//! # What DashCraft does
//!
//! - A page is offered for a data point when the point has a value for
//!   every drillthrough field (the fields must be in the source visual).
//! - Drilling sets [`InteractionState::drillthrough`] of the target page:
//!   the drilled points (OR of the points, AND within one) filter every
//!   visual, the saved conditions of the drillthrough cards are cleared,
//!   and with "Keep all filters" the source page's filters, the slicer
//!   selections, the cross-filter and the visual-level filters that apply to
//!   the source visual are carried as conditions (and, from another report,
//!   the source report's filters).
//! - Cross-report targets ([`cross_report_targets`]) are the cross-report
//!   pages of a report that allows them; the drilled context records the
//!   source report ([`DrillthroughContext::source_report`]).
//! - Not modelled: drillthrough on measures (`asAggregation`) and Top N
//!   filters (listed as not applied).

use pbi_engine::Value;
use pbi_model::{Condition, FieldExpr, Filter, FilterKind, InteractionKind, Literal, Report};
use serde::{Deserialize, Serialize};

use super::interaction::{
    FilterLevel, InteractionState, PointField, PointSelection, SlicerSelection, is_slicer_type,
    slicer_fields,
};
use super::plan::same_field;

/// A drillthrough field of a page and the filter card that shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct DrillthroughField {
    pub field: FieldExpr,
    /// Name of the page filter.
    pub filter: String,
}

/// A drillthrough page of a report.
#[derive(Debug, Clone, PartialEq)]
pub struct DrillthroughPage {
    /// Index of the page in the report.
    pub index: usize,
    pub name: String,
    pub display_name: String,
    pub fields: Vec<DrillthroughField>,
    /// "Keep all filters": the source's filter context flows to the page.
    pub keep_all_filters: bool,
    /// `referenceScope: "CrossReport"`: other reports may drill through to
    /// the page.
    pub cross_report: bool,
}

/// What a drillthrough set on its target page.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DrillthroughContext {
    /// Internal name of the page drilled from.
    pub source_page: String,
    /// Name of the report drilled from, when it is another report.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_report: Option<String>,
    /// The drilled points over the page's drillthrough fields; `source` is
    /// the visual drilled from.
    pub selection: PointSelection,
    /// Filters kept from the source ("Keep all filters").
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub carried: Vec<Condition>,
}

pub(super) fn is_kind(value: Option<&serde_json::Value>, kind: &str) -> bool {
    value
        .and_then(serde_json::Value::as_str)
        .is_some_and(|t| t.eq_ignore_ascii_case(kind))
}

/// Page `index` of `report` as a drillthrough page (`None` when it is not
/// one or has no drillthrough field).
pub fn drillthrough_page(report: &Report, index: usize) -> Option<DrillthroughPage> {
    let page = report.pages.get(index)?;
    let binding = page.extra.get("pageBinding").map(|b| b.to_json());
    let page_type = page.extra.get("type").map(|t| t.to_json());
    let binding_type = binding.as_ref().and_then(|b| b.get("type"));
    if is_kind(page_type.as_ref(), "Tooltip") || is_kind(binding_type, "Tooltip") {
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
    let fields: Vec<DrillthroughField> = page
        .filters
        .iter()
        .filter(|f| {
            f.how_created
                .as_deref()
                .is_some_and(|h| h.eq_ignore_ascii_case("Drillthrough"))
                || bound.contains(&f.name.as_str())
        })
        .filter_map(|f| {
            Some(DrillthroughField {
                field: f.field.clone()?,
                filter: f.name.clone(),
            })
        })
        .collect();
    if fields.is_empty() {
        return None;
    }
    let keep_all_filters = !is_kind(
        binding.as_ref().and_then(|b| b.get("acceptsFilterContext")),
        "None",
    );
    let cross_report = is_kind(
        binding.as_ref().and_then(|b| b.get("referenceScope")),
        "CrossReport",
    );
    Some(DrillthroughPage {
        index,
        name: page.name.clone(),
        display_name: page.display_name.clone(),
        fields,
        keep_all_filters,
        cross_report,
    })
}

/// `true` when other reports may drill through to `report`'s cross-report
/// pages (`settings.useCrossReportDrillthrough`).
pub fn allows_cross_report_drillthrough(report: &Report) -> bool {
    report
        .settings
        .get("useCrossReportDrillthrough")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn has_fields(page: &DrillthroughPage, point: &[PointField]) -> bool {
    page.fields
        .iter()
        .all(|f| point.iter().any(|p| same_field(&p.field, &f.field)))
}

/// The pages a data point of page `source_page` can drill through to, in
/// page order: every other drillthrough page whose fields all have a value
/// in `point`.
pub fn drillthrough_targets(
    report: &Report,
    source_page: usize,
    point: &[PointField],
) -> Vec<DrillthroughPage> {
    (0..report.pages.len())
        .filter(|&i| i != source_page)
        .filter_map(|i| drillthrough_page(report, i))
        .filter(|t| has_fields(t, point))
        .collect()
}

/// The pages of `target` (another report) a data point can drill through
/// to: its cross-report drillthrough pages whose fields all have a value in
/// `point`, when `target` allows cross-report drillthrough.
pub fn cross_report_targets(target: &Report, point: &[PointField]) -> Vec<DrillthroughPage> {
    if !allows_cross_report_drillthrough(target) {
        return Vec::new();
    }
    (0..target.pages.len())
        .filter_map(|i| drillthrough_page(target, i))
        .filter(|t| t.cross_report && has_fields(t, point))
        .collect()
}

/// The state of `target` (a page of `target_report`) after drilling through
/// from the points `selection` of page `source_page` of `report`
/// (`source_state`), starting from the target's current state; also the
/// parts not applied. `target_report` is `report` itself or, for a
/// cross-report drillthrough, another report.
///
/// # Errors
/// A reason when a point has no value for a drillthrough field.
pub fn drill_through(
    report: &Report,
    source_page: usize,
    source_state: &InteractionState,
    target_report: &Report,
    target: &DrillthroughPage,
    selection: &PointSelection,
    target_state: &InteractionState,
) -> Result<(InteractionState, Vec<String>), String> {
    let cross_report = report.id != target_report.id || report.name != target_report.name;
    let mut points: Vec<Vec<PointField>> = Vec::with_capacity(selection.points.len());
    for point in &selection.points {
        let mut drilled = Vec::with_capacity(target.fields.len());
        for f in &target.fields {
            let value = point
                .iter()
                .find(|p| same_field(&p.field, &f.field))
                .map(|p| p.value.clone())
                .ok_or_else(|| {
                    format!(
                        "the selection has no value for {}",
                        f.field.query_ref().unwrap_or_else(|| f.filter.clone())
                    )
                })?;
            drilled.push(PointField {
                field: f.field.clone(),
                value,
            });
        }
        if !points.contains(&drilled) {
            points.push(drilled);
        }
    }
    if points.is_empty() {
        return Err("no data point is selected".into());
    }
    let mut state = target_state.clone();
    state.selected_points = None;
    if let Some(page) = target_report.pages.get(target.index) {
        for f in page
            .filters
            .iter()
            .filter(|f| target.fields.iter().any(|d| d.filter == f.name))
        {
            state.set_filter(FilterLevel::Page, f, None);
        }
    }
    let mut not_applied = Vec::new();
    let carried = if target.keep_all_filters {
        let mut carried = Vec::new();
        if cross_report {
            carry(
                report
                    .filters
                    .iter()
                    .map(|f| state_filter(source_state, FilterLevel::Report, f)),
                &|f| target.fields.iter().any(|d| same_field(&d.field, f)),
                "report",
                &mut carried,
                &mut not_applied,
            );
        }
        carried.extend(carried_filters(
            report,
            source_page,
            source_state,
            &selection.source,
            &target.fields,
            &mut not_applied,
        ));
        carried
    } else {
        Vec::new()
    };
    state.drillthrough = Some(DrillthroughContext {
        source_page: report
            .pages
            .get(source_page)
            .map(|p| p.name.clone())
            .unwrap_or_default(),
        source_report: cross_report.then(|| report.name.clone()),
        selection: PointSelection {
            source: selection.source.clone(),
            points,
        },
        carried,
    });
    Ok((state, not_applied))
}

fn state_filter(state: &InteractionState, level: FilterLevel<'_>, f: &Filter) -> Filter {
    state.filters.apply(level, f).into_owned()
}

/// Adds the conditions of `filters` to `out`, except those on a field
/// `skip` accepts and Top N filters (noted in `not_applied`).
fn carry(
    filters: impl Iterator<Item = Filter>,
    skip: &dyn Fn(&FieldExpr) -> bool,
    level: &str,
    out: &mut Vec<Condition>,
    not_applied: &mut Vec<String>,
) {
    for f in filters {
        let Some(condition) = &f.condition else {
            continue;
        };
        if f.field.as_ref().is_some_and(skip) {
            continue;
        }
        if f.kind == FilterKind::TopN {
            not_applied.push(format!("the Top N {level} filter {}", filter_label(&f)));
            continue;
        }
        out.push(condition.clone());
    }
}

/// The filter context of visual `visual` on page `page` that "Keep all
/// filters" carries: page filters, slicer selections, the cross-filter
/// from another visual and the visual's own filters, except those on a
/// drillthrough field (the drilled value replaces them).
pub(super) fn carried_filters(
    report: &Report,
    page: usize,
    state: &InteractionState,
    visual: &str,
    drilled: &[DrillthroughField],
    not_applied: &mut Vec<String>,
) -> Vec<Condition> {
    let Some(page) = report.pages.get(page) else {
        return Vec::new();
    };
    let is_drilled = |field: &FieldExpr| drilled.iter().any(|d| same_field(&d.field, field));
    let mut out = Vec::new();
    carry(
        page.filters
            .iter()
            .map(|f| state_filter(state, FilterLevel::Page, f)),
        &is_drilled,
        "page",
        &mut out,
        not_applied,
    );
    for slicer in page
        .visuals
        .iter()
        .filter(|s| s.name != visual && is_slicer_type(&s.visual_type))
    {
        let Some(selection) = state.slicer_selections.get(&slicer.name) else {
            continue;
        };
        if selection.is_all()
            || page.interaction(&slicer.name, visual) == Some(&InteractionKind::NoFilter)
        {
            continue;
        }
        let fields = slicer_fields(slicer);
        if fields.iter().any(is_drilled) {
            continue;
        }
        out.extend(selection_condition(&fields, selection));
    }
    if let Some(points) = &state.selected_points
        && points.source != visual
        && page.interaction(&points.source, visual) != Some(&InteractionKind::NoFilter)
        && let Some(first) = points.points.first()
    {
        let fields: Vec<FieldExpr> = first.iter().map(|p| p.field.clone()).collect();
        let rows: Option<Vec<Vec<Literal>>> = points
            .points
            .iter()
            .map(|p| {
                (p.len() == fields.len())
                    .then(|| {
                        fields
                            .iter()
                            .map(|f| {
                                p.iter()
                                    .find(|x| same_field(&x.field, f))
                                    .map(|x| literal(&x.value))
                            })
                            .collect()
                    })
                    .flatten()
            })
            .collect();
        match rows {
            Some(values) if !fields.is_empty() && !fields.iter().any(is_drilled) => {
                out.push(Condition::In {
                    exprs: fields,
                    values,
                });
            }
            Some(_) => {}
            None => not_applied.push(format!("the selection in {}", points.source)),
        }
    }
    if let Some(v) = page.visuals.iter().find(|v| v.name == visual) {
        carry(
            v.filters
                .iter()
                .map(|f| state_filter(state, FilterLevel::Visual(visual), f)),
            &is_drilled,
            "visual",
            &mut out,
            not_applied,
        );
    }
    out
}

pub(super) fn filter_label(f: &Filter) -> String {
    f.display_name
        .clone()
        .or_else(|| f.field.as_ref().and_then(FieldExpr::query_ref))
        .unwrap_or_else(|| f.name.clone())
}

/// A slicer selection over `fields` as a condition.
fn selection_condition(fields: &[FieldExpr], selection: &SlicerSelection) -> Option<Condition> {
    let values = (!selection.values.is_empty() && !fields.is_empty()).then(|| {
        let tuples = Condition::In {
            exprs: fields.to_vec(),
            values: selection
                .values
                .iter()
                .map(|t| t.iter().map(literal).collect())
                .collect(),
        };
        if selection.inverted {
            Condition::Not(Box::new(tuples))
        } else {
            tuples
        }
    });
    Condition::and_all(values.into_iter().chain(selection.condition.clone()))
}

/// A value as a semantic-query literal.
pub fn literal(value: &Value) -> Literal {
    match value {
        Value::Blank => Literal::null(),
        Value::Boolean(b) => Literal::boolean(*b),
        Value::Integer(n) => Literal::integer(*n),
        Value::Double(f) => Literal::double(*f),
        Value::Decimal(_) => Literal::decimal(value.as_f64().unwrap_or_default()),
        Value::String(s) => Literal::text(s),
        Value::DateTime(_) => Literal::datetime(&value.to_string().replacen(' ', "T", 1)),
    }
}

#[cfg(test)]
mod tests {
    use pbi_model::{Page, PropValue, Visual};

    use super::super::filter::coerce_literal;
    use super::*;

    fn filter(name: &str, field: FieldExpr, how: Option<&str>) -> Filter {
        Filter {
            name: name.into(),
            field: Some(field),
            how_created: how.map(str::to_owned),
            ..Filter::default()
        }
    }

    fn report() -> Report {
        let region = FieldExpr::column("Stores", "Region");
        let mut overview = Page {
            name: "overview".into(),
            display_name: "Overview".into(),
            ..Page::default()
        };
        overview.filters.push(Filter {
            condition: Some(Condition::In {
                exprs: vec![FieldExpr::column("Products", "Category")],
                values: vec![vec![Literal::text("Camping")]],
            }),
            ..filter("cat", FieldExpr::column("Products", "Category"), None)
        });
        overview.visuals.push(Visual {
            name: "slicer".into(),
            visual_type: "slicer".into(),
            ..Visual::default()
        });
        overview.visuals.push(Visual {
            name: "bars".into(),
            visual_type: "clusteredBarChart".into(),
            filters: vec![
                Filter {
                    condition: Some(Condition::In {
                        exprs: vec![FieldExpr::column("Products", "Brand")],
                        values: vec![vec![Literal::text("Acme")]],
                    }),
                    ..filter("brand", FieldExpr::column("Products", "Brand"), None)
                },
                // On the drilled field: the drilled value replaces it.
                Filter {
                    condition: Some(Condition::In {
                        exprs: vec![region.clone()],
                        values: vec![vec![Literal::text("West")]],
                    }),
                    ..filter("region", region.clone(), None)
                },
            ],
            ..Visual::default()
        });
        let mut details = Page {
            name: "details".into(),
            display_name: "Region details".into(),
            ..Page::default()
        };
        details.extra.insert(
            "pageBinding".into(),
            PropValue::Json(serde_json::json!({
                "name": "b", "type": "Drillthrough", "acceptsFilterContext": "Default",
                "parameters": [{ "name": "p", "boundFilter": "dt" }]
            })),
        );
        details.filters.push(Filter {
            condition: Some(Condition::In {
                exprs: vec![region.clone()],
                values: vec![vec![Literal::text("East")]],
            }),
            ..filter("dt", region.clone(), None)
        });
        let mut legacy = Page {
            name: "legacy".into(),
            display_name: "Legacy".into(),
            ..Page::default()
        };
        legacy.filters.push(filter(
            "f",
            FieldExpr::column("Stores", "City"),
            Some("Drillthrough"),
        ));
        let mut tip = Page {
            name: "tip".into(),
            ..Page::default()
        };
        tip.extra
            .insert("type".into(), PropValue::Json(serde_json::json!("Tooltip")));
        tip.filters
            .push(filter("t", region.clone(), Some("Drillthrough")));
        Report {
            name: "Sales".into(),
            pages: vec![overview, details, legacy, tip],
            ..Report::default()
        }
    }

    /// A report whose "detail" page accepts drillthrough from other reports.
    fn stores_report(allow: bool) -> Report {
        let mut detail = Page {
            name: "detail".into(),
            display_name: "Store detail".into(),
            ..Page::default()
        };
        detail.extra.insert(
            "pageBinding".into(),
            PropValue::Json(serde_json::json!({
                "name": "b", "type": "Drillthrough", "referenceScope": "CrossReport",
                "parameters": [{ "name": "p", "boundFilter": "dt" }]
            })),
        );
        // The target report's saved drillthrough value is cleared.
        detail.filters.push(Filter {
            condition: Some(Condition::In {
                exprs: vec![FieldExpr::column("Stores", "Region")],
                values: vec![vec![Literal::text("East")]],
            }),
            ..filter("dt", FieldExpr::column("Stores", "Region"), None)
        });
        let mut local = Page {
            name: "local".into(),
            ..Page::default()
        };
        local.filters.push(filter(
            "l",
            FieldExpr::column("Stores", "Region"),
            Some("Drillthrough"),
        ));
        Report {
            name: "Stores".into(),
            pages: vec![detail, local],
            settings: serde_json::json!({ "useCrossReportDrillthrough": allow }),
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

    #[test]
    fn targets_need_every_drillthrough_field() {
        let r = report();
        let region = point(&[("stores", "region", "West")]);
        let names = |p: &[PointField]| -> Vec<String> {
            drillthrough_targets(&r, 0, p)
                .into_iter()
                .map(|t| t.name)
                .collect()
        };
        // Binding page; field names match case-insensitively; tooltips never.
        assert_eq!(names(&region), ["details"]);
        let both = point(&[("Stores", "Region", "West"), ("Stores", "City", "Oslo")]);
        assert_eq!(names(&both), ["details", "legacy"]);
        assert!(names(&point(&[("Products", "Category", "Camping")])).is_empty());
        // Never the page drilled from.
        assert!(
            drillthrough_targets(&r, 1, &region).is_empty(),
            "the source page is not a target"
        );
        assert!(drillthrough_page(&r, 0).is_none());
        assert!(drillthrough_page(&r, 3).is_none());
    }

    #[test]
    fn drilling_filters_clears_the_card_and_keeps_filters() {
        let r = report();
        let target = drillthrough_page(&r, 1).expect("drillthrough page");
        assert!(target.keep_all_filters);
        let mut source = InteractionState::new();
        source.select("slicer", SlicerSelection::excluding(["Online"]));
        let selection = PointSelection {
            source: "bars".into(),
            points: vec![point(&[
                ("Stores", "Region", "West"),
                ("Date", "Year", "2026"),
            ])],
        };
        let (state, not_applied) = drill_through(
            &r,
            0,
            &source,
            &r,
            &target,
            &selection,
            &InteractionState::new(),
        )
        .expect("drill");
        assert!(not_applied.is_empty());
        // The saved condition of the drillthrough card no longer filters.
        assert_eq!(
            state.filters.get(FilterLevel::Page, "dt"),
            Some(&None),
            "{state:?}"
        );
        let drill = state.drillthrough.expect("drillthrough context");
        assert_eq!(drill.source_page, "overview");
        assert_eq!(drill.source_report, None);
        assert_eq!(drill.selection.source, "bars");
        assert_eq!(
            drill.selection.points,
            vec![point(&[("Stores", "Region", "West")])]
        );
        // The page filter and the source visual's brand filter are carried
        // (not its region filter: the drilled value replaces it); the slicer
        // has no fields (nothing to carry).
        assert_eq!(drill.carried.len(), 2, "{:?}", drill.carried);
        let brand = |c: &Condition| matches!(c, Condition::In { exprs, .. } if exprs[0] == FieldExpr::column("Products", "Brand"));
        assert!(brand(&drill.carried[1]), "{:?}", drill.carried);

        let missing = PointSelection {
            source: "bars".into(),
            points: vec![point(&[("Date", "Year", "2026")])],
        };
        let err = drill_through(&r, 0, &source, &r, &target, &missing, &source).unwrap_err();
        assert!(
            err.contains("Stores.Region") || err.contains("Region"),
            "{err}"
        );
    }

    #[test]
    fn cross_report_pages_take_the_source_report_filters() {
        let mut sales = report();
        sales.filters.push(Filter {
            condition: Some(Condition::In {
                exprs: vec![FieldExpr::column("Date", "Year")],
                values: vec![vec![Literal::integer(2026)]],
            }),
            ..filter("year", FieldExpr::column("Date", "Year"), None)
        });
        let west = point(&[("Stores", "Region", "West")]);
        // Only cross-report pages, and only when the report allows them.
        assert!(cross_report_targets(&stores_report(false), &west).is_empty());
        let stores = stores_report(true);
        let targets = cross_report_targets(&stores, &west);
        assert_eq!(
            targets.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["detail"]
        );
        assert!(targets[0].cross_report && targets[0].keep_all_filters);
        let selection = PointSelection {
            source: "bars".into(),
            points: vec![west],
        };
        let (state, _) = drill_through(
            &sales,
            0,
            &InteractionState::new(),
            &stores,
            &targets[0],
            &selection,
            &InteractionState::new(),
        )
        .expect("drill");
        let drill = state.drillthrough.expect("drillthrough context");
        assert_eq!(drill.source_report.as_deref(), Some("Sales"));
        // The report filter, then the page and visual filters.
        assert_eq!(drill.carried.len(), 3, "{:?}", drill.carried);
        assert!(
            matches!(&drill.carried[0], Condition::In { exprs, .. } if exprs[0] == FieldExpr::column("Date", "Year")),
            "{:?}",
            drill.carried
        );
        assert_eq!(state.filters.get(FilterLevel::Page, "dt"), Some(&None));
    }

    #[test]
    fn literals_round_trip_values() {
        for v in [
            Value::Blank,
            Value::Boolean(true),
            Value::Integer(-4),
            Value::Double(1.5),
            Value::from("O'Neil"),
            Value::DateTime(1_767_225_600_000_000),
        ] {
            let lit = literal(&v).value().expect("literal");
            assert_eq!(coerce_literal(&lit, None), v, "{v:?}");
        }
    }
}
