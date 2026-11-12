//! Filter context of one visual: report + page + visual filters (with the
//! conditions a bookmark set), slicer
//! selections of the other slicers, cross-filter from the selected visual
//! (honouring `visualInteractions`), then Top N filters. A drillthrough
//! page adds the drilled points and the filters the drillthrough carried
//! after its page filters. The visual's own `general.selfFilter` (the
//! filter newer slicers keep on their item list) applies to it alone.
//! Drill-down paths (the visual's own, and those of visuals that
//! drill-filter the page) follow the visual filters.
//!
//! Points selected in another visual highlight bar, column, combo, pie,
//! donut, treemap and funnel charts
//! (the default interaction, or `HighlightFilter`, unless the report sets
//! `defaultFilterActionIsDataFilter`): [`VisualFilters::highlight`] is the
//! visual's filter context plus the selection, and the visual keeps its
//! unselected values. Other targets are filtered.

use pbi_engine::FilterContext;
use pbi_model::{Condition, Filter, InteractionKind, Page, Report, Visual};

use crate::dataview::{QueryWarning, WarningScope};

use super::drillthrough::DrillthroughContext;
use super::filter::{FilterBuilder, FilterPlan};
use super::interaction::{
    FilterLevel, InteractionState, PointSelection, SlicerSelection, is_slicer_type, slicer_fields,
};
use super::plan::{same_field, value_field};
use super::semantic::TopNSpec;
use super::{QueryEnv, QueryLimits};

/// Everything that filters one visual.
#[derive(Debug, Clone, Default)]
pub(crate) struct VisualFilters {
    pub context: FilterContext,
    /// Visual-level conditions over measures / aggregations (applied to
    /// the visual's rows).
    pub value_conditions: Vec<Condition>,
    /// [`Self::context`] plus the points selected in the visual that
    /// highlights this one (`None` when nothing highlights it).
    pub highlight: Option<FilterContext>,
    pub warnings: Vec<QueryWarning>,
}

/// Visual types that draw highlighted values (bars, columns, the columns
/// of combos, pies, donuts, treemaps and funnels).
fn draws_highlights(visual_type: &str) -> bool {
    matches!(
        visual_type,
        "clusteredBarChart"
            | "barChart"
            | "stackedBarChart"
            | "hundredPercentStackedBarChart"
            | "clusteredColumnChart"
            | "columnChart"
            | "stackedColumnChart"
            | "hundredPercentStackedColumnChart"
            | "lineClusteredColumnComboChart"
            | "lineStackedColumnComboChart"
            | "pieChart"
            | "donutChart"
            | "treemap"
            | "funnel"
    )
}

/// The report sets "selections filter other visuals" instead of
/// highlighting them.
fn filters_by_default(report: &Report) -> bool {
    report
        .settings
        .get("defaultFilterActionIsDataFilter")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
}

struct Pending {
    scope: WarningScope,
    name: String,
    field: Option<String>,
    spec: TopNSpec,
}

pub(crate) fn visual_filters(
    env: &QueryEnv,
    report: &Report,
    page: &Page,
    visual: &Visual,
    state: &InteractionState,
    limits: &QueryLimits,
) -> VisualFilters {
    let builder = env.builder();
    let mut out = VisualFilters::default();
    let mut top_n = Vec::new();
    // A data table (focus mode's "Show as a table") filters as its visual.
    let name = super::export::source_name(&visual.name);
    for (scope, level, filters) in [
        (WarningScope::Report, FilterLevel::Report, &report.filters),
        (WarningScope::Page, FilterLevel::Page, &page.filters),
        (
            WarningScope::Visual,
            FilterLevel::Visual(name),
            &visual.filters,
        ),
    ] {
        for f in filters {
            let f = state.filters.apply(level, f);
            let plan = builder.plan(&f);
            apply_plan(&mut out, &mut top_n, scope, &f, plan);
        }
        if level == FilterLevel::Page
            && let Some(drill) = &state.drillthrough
        {
            apply_drillthrough(&builder, &mut out, drill);
        }
    }
    apply_self_filter(&builder, &mut out, visual);
    for path in super::drill::drill_paths(report, page, visual, state) {
        apply_selection(
            &builder,
            &mut out.context,
            &mut out.warnings,
            &path,
            WarningScope::Drill,
            "drill",
        );
    }

    let mut highlight_points = false;
    let mut highlight_noted = false;
    let mut note_highlight = |out: &mut VisualFilters, source: &str| {
        if !highlight_noted {
            highlight_noted = true;
            out.warnings.push(
                QueryWarning::new(
                    WarningScope::Interaction,
                    "highlight interactions are applied as filters",
                )
                .with_source(source),
            );
        }
    };

    for slicer in page
        .visuals
        .iter()
        .filter(|s| s.name != name && is_slicer_type(&s.visual_type))
    {
        let Some(selection) = state.slicer_selections.get(&slicer.name) else {
            continue;
        };
        if selection.is_all() {
            continue;
        }
        match page.interaction(&slicer.name, name) {
            Some(InteractionKind::NoFilter) => continue,
            Some(InteractionKind::HighlightFilter) => note_highlight(&mut out, &slicer.name),
            _ => {}
        }
        apply_slicer(&builder, &mut out, slicer, selection);
    }

    if let Some(points) = &state.selected_points
        && points.source != name
        && !points.points.is_empty()
    {
        let kind = page.interaction(&points.source, name);
        let highlight = match kind {
            Some(InteractionKind::NoFilter) => None,
            Some(InteractionKind::HighlightFilter) => Some(true),
            Some(_) => Some(false),
            None => Some(!filters_by_default(report)),
        };
        match highlight {
            Some(true) if draws_highlights(&visual.visual_type) => highlight_points = true,
            Some(highlight) => {
                // An explicit highlight on a visual that cannot draw one.
                if highlight && kind.is_some() {
                    note_highlight(&mut out, &points.source);
                }
                apply_points(&builder, &mut out.context, &mut out.warnings, points);
            }
            None => {}
        }
    }

    for p in top_n {
        if let Err(reason) = apply_top_n(env, &builder, &mut out.context, &p.spec, limits) {
            out.warnings.push(
                QueryWarning::new(p.scope, format!("Top N filter not applied: {reason}"))
                    .with_source(p.name)
                    .with_field(p.field)
                    .not_applied(),
            );
        }
    }
    if highlight_points && let Some(points) = &state.selected_points {
        let mut context = out.context.clone();
        apply_points(&builder, &mut context, &mut out.warnings, points);
        out.highlight = Some(context);
    }
    out
}

fn apply_plan(
    out: &mut VisualFilters,
    top_n: &mut Vec<Pending>,
    scope: WarningScope,
    f: &Filter,
    plan: FilterPlan,
) {
    let field = f.field.as_ref().and_then(pbi_model::FieldExpr::query_ref);
    let warn = |message: String| {
        QueryWarning::new(scope, message)
            .with_source(f.name.clone())
            .with_field(field.clone())
    };
    for filter in plan.filters {
        out.context.add(filter);
    }
    if scope == WarningScope::Visual {
        out.value_conditions.extend(plan.value_conditions);
    } else if !plan.value_conditions.is_empty() {
        out.warnings.push(
            warn("filter not applied: measure filters are only supported on visuals".into())
                .not_applied(),
        );
    }
    if let Some(spec) = plan.top_n {
        top_n.push(Pending {
            scope,
            name: f.name.clone(),
            field: field.clone(),
            spec,
        });
    }
    for reason in plan.skipped {
        out.warnings
            .push(warn(format!("filter not applied: {reason}")).not_applied());
    }
    for reason in plan.approximate {
        out.warnings
            .push(warn(format!("filter applied approximately: {reason}")));
    }
}

fn apply_slicer(
    builder: &FilterBuilder<'_>,
    out: &mut VisualFilters,
    slicer: &Visual,
    selection: &SlicerSelection,
) {
    let warn = |message: String| {
        QueryWarning::new(WarningScope::Slicer, message).with_source(slicer.name.clone())
    };
    if !selection.values.is_empty() {
        let fields = slicer_fields(slicer);
        match builder.tuple_filter(&fields, &selection.values, selection.inverted) {
            Ok((filters, approx)) => {
                for f in filters {
                    out.context.add(f);
                }
                for reason in approx {
                    out.warnings
                        .push(warn(format!("selection applied approximately: {reason}")));
                }
            }
            Err(reason) => out
                .warnings
                .push(warn(format!("selection not applied: {reason}")).not_applied()),
        }
    }
    if let Some(condition) = &selection.condition {
        let mut plan = FilterPlan::default();
        builder.plan_condition(condition, &mut plan);
        for f in plan.filters {
            out.context.add(f);
        }
        for reason in plan.skipped.into_iter().chain(
            (!plan.value_conditions.is_empty())
                .then(|| "measure conditions are not supported on slicers".to_owned()),
        ) {
            out.warnings
                .push(warn(format!("selection not applied: {reason}")).not_applied());
        }
    }
}

/// A visual's own `general.selfFilter` (the newer slicers keep the filter
/// that narrows their own item list there): applied to that visual only,
/// never to the visuals it filters.
fn apply_self_filter(builder: &FilterBuilder<'_>, out: &mut VisualFilters, visual: &Visual) {
    let Some(def) = pbi_pbir::objects::property(&visual.objects, "general", "selfFilter")
        .and_then(|f| f.get("filter"))
    else {
        return;
    };
    let warn = |message: String| {
        QueryWarning::new(WarningScope::Visual, message)
            .with_source(visual.name.clone())
            .not_applied()
    };
    let Some(condition) = super::semantic::where_condition(def) else {
        out.warnings.push(warn(
            "self filter not applied: cannot read its condition".into(),
        ));
        return;
    };
    let mut plan = FilterPlan::default();
    builder.plan_condition(&condition, &mut plan);
    for f in plan.filters {
        out.context.add(f);
    }
    out.value_conditions.extend(plan.value_conditions);
    for reason in plan.skipped {
        out.warnings
            .push(warn(format!("self filter not applied: {reason}")));
    }
}

/// The drilled points and the carried conditions of a drillthrough.
fn apply_drillthrough(
    builder: &FilterBuilder<'_>,
    out: &mut VisualFilters,
    drill: &DrillthroughContext,
) {
    let warn = |message: String| {
        QueryWarning::new(WarningScope::Drillthrough, message)
            .with_source(drill.source_page.clone())
    };
    for condition in &drill.carried {
        let mut plan = FilterPlan::default();
        builder.plan_condition(condition, &mut plan);
        for f in plan.filters {
            out.context.add(f);
        }
        for reason in plan.skipped.into_iter().chain(
            (!plan.value_conditions.is_empty())
                .then(|| "measure conditions are not kept by a drillthrough".to_owned()),
        ) {
            out.warnings
                .push(warn(format!("kept filter not applied: {reason}")).not_applied());
        }
        for reason in plan.approximate {
            out.warnings
                .push(warn(format!("kept filter applied approximately: {reason}")));
        }
    }
    apply_selection(
        builder,
        &mut out.context,
        &mut out.warnings,
        &drill.selection,
        WarningScope::Drillthrough,
        "drillthrough",
    );
}

fn apply_points(
    builder: &FilterBuilder<'_>,
    context: &mut FilterContext,
    warnings: &mut Vec<QueryWarning>,
    sel: &PointSelection,
) {
    apply_selection(
        builder,
        context,
        warnings,
        sel,
        WarningScope::CrossFilter,
        "cross-filter",
    );
}

/// Filters `context` to the points of `sel` (OR of the points, AND within
/// one).
fn apply_selection(
    builder: &FilterBuilder<'_>,
    context: &mut FilterContext,
    warnings: &mut Vec<QueryWarning>,
    sel: &PointSelection,
    scope: WarningScope,
    what: &str,
) {
    let warn = |message: String| QueryWarning::new(scope, message).with_source(sel.source.clone());
    let Some(first) = sel.points.first() else {
        return;
    };
    let fields: Vec<_> = first.iter().map(|p| p.field.clone()).collect();
    let mut tuples = Vec::with_capacity(sel.points.len());
    let mut dropped = 0usize;
    for point in &sel.points {
        let tuple: Option<Vec<_>> = fields
            .iter()
            .map(|f| {
                point
                    .iter()
                    .find(|p| same_field(&p.field, f))
                    .map(|p| p.value.clone())
            })
            .collect();
        match tuple {
            Some(t) if point.len() == fields.len() => tuples.push(t),
            _ => dropped += 1,
        }
    }
    if dropped > 0 {
        warnings.push(warn(format!(
            "{dropped} selected point(s) with different fields were ignored"
        )));
    }
    match builder.tuple_filter(&fields, &tuples, false) {
        Ok((filters, approx)) => {
            for f in filters {
                context.add(f);
            }
            for reason in approx {
                warnings.push(warn(format!("{what} applied approximately: {reason}")));
            }
        }
        Err(reason) => warnings.push(warn(format!("{what} not applied: {reason}")).not_applied()),
    }
}

/// Evaluates a Top N filter in `ctx` and adds the resulting `IN` filter.
fn apply_top_n(
    env: &QueryEnv,
    builder: &FilterBuilder<'_>,
    ctx: &mut FilterContext,
    spec: &TopNSpec,
    limits: &QueryLimits,
) -> Result<(), String> {
    let count = usize::try_from(spec.count).unwrap_or(0);
    if count == 0 {
        return Ok(());
    }
    let cols = spec
        .columns
        .iter()
        .map(|c| builder.resolve_column(c))
        .collect::<Result<Vec<_>, _>>()?;
    let (expr, _) = value_field(&env.model, builder, &spec.order_by, false)?;
    let groups: Vec<&_> = cols.iter().collect();
    let mut rows = env
        .summarize(ctx, &groups, &[expr.dax.as_str()], limits)
        .map_err(|e| e.to_string())?;
    let width = cols.len();
    let key = |r: &Vec<pbi_engine::Value>| r.get(width).cloned().unwrap_or_default();
    rows.sort_by(|a, b| {
        let o = key(a).total_cmp(&key(b));
        if spec.descending { o.reverse() } else { o }
    });
    let mut keep = rows.len().min(count);
    if let Some(last) = keep.checked_sub(1).and_then(|i| rows.get(i)).map(key) {
        while rows
            .get(keep)
            .is_some_and(|r| key(r).total_cmp(&last).is_eq())
        {
            keep += 1;
        }
    }
    let tuples: Vec<Vec<pbi_engine::Value>> = rows
        .iter()
        .take(keep)
        .map(|r| r.iter().take(width).cloned().collect())
        .collect();
    let (filters, _) = builder.tuple_filter(&spec.columns, &tuples, false)?;
    for f in filters {
        ctx.add(f);
    }
    Ok(())
}
