//! Drill down and up (M9.2): a visual whose axis (or a matrix whose rows)
//! holds several fields, such as the levels of a hierarchy, shows a range
//! of them. The saved range is the role's `active` projections (all of
//! them when none is marked); the reader changes it as view state
//! ([`InteractionState::drill`], ADR-0006):
//!
//! - **Drill down** on a data point: the point's values on the shown
//!   levels become the drill path (a filter) and the next level is shown.
//! - **Drill up**: an expanded range loses its last level; otherwise the
//!   level above is shown and its path entries are dropped.
//! - **Go to the next level**: the level below the last shown one alone,
//!   for every value of the levels above.
//! - **Expand all down one level**: the next level is added to the shown
//!   range (concatenated labels on charts, nested rows in matrices).
//!
//! The path filters the drilled visual and, when the visual (or the
//! report) sets `drillFilterOtherVisuals`, the other visuals of the page
//! that its interactions do not exclude. A sort on a level that is not
//! shown sorts by the first shown level in the same direction.

use std::borrow::Cow;

use pbi_engine::Value;
use pbi_model::{FieldExpr, InteractionKind, Page, Projection, Report, Visual};
use serde::{Deserialize, Serialize};

use crate::dataview::value_serde;

use super::interaction::{InteractionState, PointField, PointSelection};
use super::plan::same_field;

/// Chart roles that can form a drill hierarchy, in order of preference.
const CHART_ROLES: &[&str] = &["Category", "Axis", "Rows", "Group", "Details", "Location"];

/// One drilled value: level `level` of the drill role equals `value`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DrillStep {
    pub level: usize,
    pub field: FieldExpr,
    #[serde(with = "value_serde")]
    pub value: Value,
}

/// The shown levels of a visual's drill role and the drilled values.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DrillState {
    /// First shown level (index into the role's projections).
    pub first: usize,
    /// Last shown level (`>= first`; above `first`: expanded).
    pub last: usize,
    /// Drilled values of levels above `first`, top level first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<DrillStep>,
}

/// A drill command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DrillAction {
    /// Drill down on a data point.
    Down,
    Up,
    /// Go to the next level.
    Next,
    /// Expand all down one level.
    Expand,
}

/// What the reader can do with a drillable visual.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DrillInfo {
    /// The role whose fields are the levels.
    pub role: String,
    /// Display names of the levels.
    pub levels: Vec<String>,
    pub first: usize,
    pub last: usize,
    /// `Field = value` of each drilled level.
    pub path: Vec<String>,
    pub can_drill_up: bool,
    /// There is a level below the shown ones (drill down, next, expand).
    pub can_drill_down: bool,
}

/// The drill role of `visual` and its levels: a matrix's `Rows`, else
/// the first chart axis role, when it has two or more fields.
pub fn drill_levels(visual: &Visual) -> Option<(&str, Vec<&Projection>)> {
    let t = visual.visual_type.as_str();
    let roles: &[&str] = match t {
        "pivotTable" | "matrix" => &["Rows"],
        "tableEx" | "table" | "multiRowCard" | "card" | "cardVisual" | "kpi" => &[],
        _ if super::is_slicer_type(t) => &[],
        _ => CHART_ROLES,
    };
    roles.iter().find_map(|role| {
        let levels: Vec<&Projection> = visual.projections_for(role).collect();
        (levels.len() > 1).then_some((*role, levels))
    })
}

/// The saved range: the `active` projections, else every level.
pub fn saved_drill(visual: &Visual) -> Option<DrillState> {
    let (_, levels) = drill_levels(visual)?;
    let active: Vec<usize> = levels
        .iter()
        .enumerate()
        .filter(|(_, p)| p.active == Some(true))
        .map(|(i, _)| i)
        .collect();
    let (first, last) = match (active.first(), active.last()) {
        (Some(&f), Some(&l)) => (f, l),
        _ => (0, levels.len() - 1),
    };
    Some(DrillState {
        first,
        last,
        path: Vec::new(),
    })
}

/// The current range of `visual` in `state` (`None`: not drillable).
pub fn drill_state(visual: &Visual, state: &InteractionState) -> Option<DrillState> {
    let saved = saved_drill(visual)?;
    let n = drill_levels(visual).map_or(0, |(_, l)| l.len());
    match state.drill.get(&visual.name) {
        Some(d) if d.first <= d.last && d.last < n => Some(d.clone()),
        _ => Some(saved),
    }
}

/// The reader's options on `visual` in `state`.
pub fn drill_info(visual: &Visual, state: &InteractionState) -> Option<DrillInfo> {
    let (role, levels) = drill_levels(visual)?;
    let d = drill_state(visual, state)?;
    Some(DrillInfo {
        role: role.to_owned(),
        levels: levels.iter().map(|p| p.label().to_owned()).collect(),
        first: d.first,
        last: d.last,
        path: d
            .path
            .iter()
            .map(|s| {
                let name = levels.get(s.level).map_or("?", |p| p.label());
                format!("{name} = {}", value_text(&s.value))
            })
            .collect(),
        can_drill_up: d.last > d.first || d.first > 0,
        can_drill_down: d.last + 1 < levels.len(),
    })
}

fn value_text(v: &Value) -> String {
    match v {
        Value::Blank => "(Blank)".to_owned(),
        Value::String(s) => s.to_string(),
        other => other.to_string(),
    }
}

/// Applies `action` to `visual`'s drill state; `point` (the fields of a
/// data point, as [`PointSelection`] points) is required to drill down.
///
/// # Errors
/// A message when the visual is not drillable or the action is not
/// possible (top or lowest level, a point without drill values).
pub fn drill(
    state: &mut InteractionState,
    visual: &Visual,
    action: DrillAction,
    point: Option<&[PointField]>,
) -> Result<(), String> {
    let (_, levels) =
        drill_levels(visual).ok_or_else(|| "this visual has no fields to drill on".to_owned())?;
    let n = levels.len();
    let mut d = drill_state(visual, state).unwrap_or_default();
    let lowest = || "the visual already shows its lowest level".to_owned();
    match action {
        DrillAction::Up => {
            if d.last > d.first {
                d.last -= 1;
            } else if d.first > 0 {
                d.first -= 1;
                d.last = d.first;
                let first = d.first;
                d.path.retain(|s| s.level < first);
            } else {
                return Err("the visual already shows its top level".to_owned());
            }
        }
        DrillAction::Next => {
            if d.last + 1 >= n {
                return Err(lowest());
            }
            d.first = d.last + 1;
            d.last = d.first;
        }
        DrillAction::Expand => {
            if d.last + 1 >= n {
                return Err(lowest());
            }
            d.last += 1;
        }
        DrillAction::Down => {
            let point = point.ok_or_else(|| "drill down needs a data point".to_owned())?;
            let mut steps = Vec::new();
            for level in d.first..=d.last {
                let Some(p) = levels
                    .get(level)
                    .and_then(|proj| point.iter().find(|f| same_field(&f.field, &proj.field)))
                else {
                    break;
                };
                steps.push(DrillStep {
                    level,
                    field: p.field.clone(),
                    value: p.value.clone(),
                });
            }
            let deepest = steps
                .last()
                .map(|s| s.level)
                .ok_or_else(|| "this data point has no value to drill on".to_owned())?;
            if deepest + 1 >= n {
                return Err(lowest());
            }
            d.path.extend(steps);
            d.first = deepest + 1;
            d.last = d.last.max(d.first);
        }
    }
    if saved_drill(visual).as_ref() == Some(&d) {
        state.drill.shift_remove(&visual.name);
    } else {
        state.drill.insert(visual.name.clone(), d);
    }
    Ok(())
}

/// `visual` as queried in `state`: the drill role's shown levels marked
/// `active`, and sort items on hidden levels moved to the first shown
/// level.
pub(crate) fn drilled_visual<'a>(visual: &'a Visual, state: &InteractionState) -> Cow<'a, Visual> {
    let Some((role, levels)) = drill_levels(visual) else {
        return Cow::Borrowed(visual);
    };
    let Some(d) = drill_state(visual, state) else {
        return Cow::Borrowed(visual);
    };
    let shown = |i: usize| (d.first..=d.last).contains(&i);
    let level_of = |f: &FieldExpr| levels.iter().position(|p| same_field(&p.field, f));
    let sort_moves = visual
        .sort
        .iter()
        .any(|s| level_of(&s.field).is_some_and(|i| !shown(i)));
    let any_active = levels.iter().any(|p| p.active == Some(true));
    let saved = saved_drill(visual).is_some_and(|s| s.first == d.first && s.last == d.last);
    if saved && !sort_moves && (any_active || (d.first == 0 && d.last + 1 == levels.len())) {
        return Cow::Borrowed(visual);
    }
    let top = levels.get(d.first).map(|p| p.field.clone());
    let mut out = visual.clone();
    for (i, p) in out
        .projections
        .iter_mut()
        .filter(|p| p.role == role)
        .enumerate()
    {
        p.active = Some(shown(i));
    }
    if let Some(top) = top {
        let mut sort = Vec::with_capacity(out.sort.len());
        for mut s in std::mem::take(&mut out.sort) {
            if level_of(&s.field).is_some_and(|i| !shown(i)) {
                s.field = top.clone();
            }
            if !sort
                .iter()
                .any(|x: &pbi_model::SortItem| same_field(&x.field, &s.field))
            {
                sort.push(s);
            }
        }
        out.sort = sort;
    }
    Cow::Owned(out)
}

/// `true` when drilling `visual` filters the other visuals of its page:
/// its `drillFilterOtherVisuals`, else the report's
/// `defaultDrillFilterOtherVisuals`.
pub fn drill_filters_others(report: &Report, visual: &Visual) -> bool {
    visual
        .extra
        .get("drillFilterOtherVisuals")
        .and_then(pbi_model::PropValue::as_bool)
        .or_else(|| {
            report
                .settings
                .get("defaultDrillFilterOtherVisuals")
                .and_then(serde_json::Value::as_bool)
        })
        .unwrap_or(false)
}

/// The drill paths that filter `visual`: its own, and those of the other
/// visuals of `page` that filter other visuals (unless their interaction
/// with `visual` is "None").
pub(crate) fn drill_paths(
    report: &Report,
    page: &Page,
    visual: &Visual,
    state: &InteractionState,
) -> Vec<PointSelection> {
    let mut out = Vec::new();
    let target = super::export::source_name(&visual.name);
    for (name, d) in &state.drill {
        if d.path.is_empty() {
            continue;
        }
        let Some(source) = page.visuals.iter().find(|v| &v.name == name) else {
            continue;
        };
        if source.name != target
            && (!drill_filters_others(report, source)
                || page.interaction(&source.name, target) == Some(&InteractionKind::NoFilter))
        {
            continue;
        }
        out.push(PointSelection {
            source: source.name.clone(),
            points: vec![
                d.path
                    .iter()
                    .map(|s| PointField {
                        field: s.field.clone(),
                        value: s.value.clone(),
                    })
                    .collect(),
            ],
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pbi_model::{PropValue, SortDirection, SortItem};

    fn level(name: &str, active: Option<bool>) -> Projection {
        Projection {
            role: "Category".into(),
            field: FieldExpr::column("Date", name),
            query_ref: format!("Date.{name}"),
            active,
            ..Projection::default()
        }
    }

    fn visual(name: &str, visual_type: &str) -> Visual {
        Visual {
            name: name.into(),
            visual_type: visual_type.into(),
            ..Visual::default()
        }
    }

    fn chart(active: [Option<bool>; 3]) -> Visual {
        let mut v = visual("chart", "clusteredColumnChart");
        v.projections = vec![
            level("Year", active[0]),
            level("Quarter", active[1]),
            level("Month", active[2]),
        ];
        v.sort = vec![SortItem {
            field: FieldExpr::column("Date", "Quarter"),
            direction: SortDirection::Ascending,
        }];
        v
    }

    fn point(pairs: &[(&str, Value)]) -> Vec<PointField> {
        pairs
            .iter()
            .map(|(c, v)| PointField {
                field: FieldExpr::column("Date", *c),
                value: v.clone(),
            })
            .collect()
    }

    fn shown(v: &Visual, state: &InteractionState) -> Vec<String> {
        let v = drilled_visual(v, state);
        let any = v.projections.iter().any(|p| p.active == Some(true));
        v.projections
            .iter()
            .filter(|p| !any || p.active == Some(true))
            .map(|p| p.label().to_owned())
            .collect()
    }

    #[test]
    fn drill_up_down_next_and_expand_move_the_shown_levels() {
        let v = chart([None, Some(true), None]);
        let mut state = InteractionState::new();
        assert_eq!(shown(&v, &state), ["Quarter"]);
        drill(&mut state, &v, DrillAction::Up, None).unwrap();
        assert_eq!(shown(&v, &state), ["Year"]);
        // The Quarter sort follows the shown level.
        let sorted = drilled_visual(&v, &state);
        assert_eq!(sorted.sort[0].field, FieldExpr::column("Date", "Year"));
        assert!(drill(&mut state, &v, DrillAction::Up, None).is_err());

        let p = point(&[("Year", Value::Integer(2025))]);
        drill(&mut state, &v, DrillAction::Down, Some(&p)).unwrap();
        assert_eq!(shown(&v, &state), ["Quarter"]);
        assert_eq!(state.drill["chart"].path.len(), 1);
        let info = drill_info(&v, &state).unwrap();
        assert_eq!(info.path, ["Year = 2025"]);
        assert!(info.can_drill_up && info.can_drill_down);

        drill(&mut state, &v, DrillAction::Expand, None).unwrap();
        assert_eq!(shown(&v, &state), ["Quarter", "Month"]);
        assert!(drill(&mut state, &v, DrillAction::Next, None).is_err());
        drill(&mut state, &v, DrillAction::Up, None).unwrap();
        assert_eq!(shown(&v, &state), ["Quarter"]);
        // Back at the saved range, with the drill path still applied.
        assert_eq!(state.drill["chart"].path.len(), 1);
        drill(&mut state, &v, DrillAction::Up, None).unwrap();
        assert_eq!(shown(&v, &state), ["Year"]);
        assert!(state.drill["chart"].path.is_empty());
        drill(&mut state, &v, DrillAction::Next, None).unwrap();
        assert!(state.drill.is_empty(), "Quarter alone is the saved state");
    }

    #[test]
    fn a_visual_without_active_levels_shows_them_all_and_matrices_drill_rows() {
        let v = chart([None, None, None]);
        let state = InteractionState::new();
        assert_eq!(shown(&v, &state), ["Year", "Quarter", "Month"]);
        assert!(matches!(drilled_visual(&v, &state), Cow::Borrowed(_)));

        let mut m = visual("matrix", "pivotTable");
        m.projections = vec![level("Year", None), level("Quarter", None)];
        for p in &mut m.projections {
            p.role = "Rows".into();
        }
        let mut state = InteractionState::new();
        let row = point(&[("Year", Value::Integer(2024))]);
        drill(&mut state, &m, DrillAction::Down, Some(&row)).unwrap();
        let d = &state.drill["matrix"];
        assert_eq!((d.first, d.last, d.path.len()), (1, 1, 1));
        let mut table = visual("t", "tableEx");
        table.projections = m.projections.clone();
        assert!(drill_levels(&table).is_none());
    }

    #[test]
    fn drill_paths_filter_other_visuals_when_the_source_says_so() {
        let mut v = chart([None, Some(true), None]);
        let other = visual("other", "card");
        let mut page = Page::default();
        let mut state = InteractionState::new();
        let p = point(&[("Quarter", "Q1".into())]);
        drill(&mut state, &v, DrillAction::Down, Some(&p)).unwrap();
        page.visuals = vec![v.clone(), other.clone()];
        let report = Report::default();
        assert_eq!(drill_paths(&report, &page, &v, &state).len(), 1);
        assert!(drill_paths(&report, &page, &other, &state).is_empty());
        v.extra
            .insert("drillFilterOtherVisuals".into(), PropValue::Bool(true));
        page.visuals = vec![v, other.clone()];
        assert_eq!(drill_paths(&report, &page, &other, &state).len(), 1);
    }
}
