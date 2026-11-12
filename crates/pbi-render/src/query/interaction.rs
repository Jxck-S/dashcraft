//! Report interaction state: slicer selections, the selected data points
//! of one visual (cross-filtering), what a bookmark changes (visuals it
//! shows, hides or sorts and filter cards it sets) and the visual header's
//! view modes (spotlight, focus mode, "Show as a table").
//!
//! # Where slicer selections live in PBIR
//!
//! A slicer persists its selection in its own `visual.json`, as a
//! semantic-query filter definition in the formatting objects:
//!
//! ```json
//! "objects": {
//!   "general": [{ "properties": {
//!     "filter": { "filter": {
//!       "Version": 2,
//!       "From": [{ "Name": "p", "Entity": "Products", "Type": 0 }],
//!       "Where": [{ "Condition": { "In": {
//!         "Expressions": [{ "Column": { "Expression": { "SourceRef": { "Source": "p" } }, "Property": "Category" } }],
//!         "Values": [[{ "Literal": { "Value": "'Camping'" } }]]
//!       } } }]
//!     } }
//!   } }],
//!   "data": [{ "properties": { "isInvertedSelectionMode": { "expr": { "Literal": { "Value": "true" } } } } }]
//! }
//! ```
//!
//! - `In` → the selected values; `Not(In)` → an inverted selection
//!   ("select all" with the listed values unchecked). Other conditions
//!   (range sliders: `Comparison`/`Between`; relative dates) are kept as a
//!   condition and applied through the
//!   [`FilterBuilder`](super::filter::FilterBuilder).
//! - `isInvertedSelectionMode` (in `objects.data` or `objects.general`)
//!   without a `filter` means "everything selected": no filter (a dropdown
//!   showing "All"), so the persisted selection is empty.
//! - Slicer *visual-level* filters (`filterConfig`) only restrict the
//!   slicer's own value list; they are not its selection.
//! - `syncGroup` (`{groupName, fieldChanges, filterChanges}`; in a PBIX
//!   layout under `singleVisual`) links slicers across pages: a selection
//!   made in one is copied to the others ([`synced_slicers`]). Saved files
//!   already carry the same filter on every synced slicer.
//! - `general.selfFilter` (newer slicers) narrows the slicer's own item
//!   list only and is not its selection (applied in `context`).
//!
//! A drillthrough page keeps what it was drilled with in
//! [`InteractionState::drillthrough`] ([`super::drillthrough`]).

use std::borrow::Cow;

use indexmap::IndexMap;
use pbi_engine::Value;
use pbi_model::{
    Condition, FieldExpr, Filter, LiteralValue, Page, Position, PropValue, Report, SortItem, Visual,
};
use serde::{Deserialize, Serialize};

use crate::dataview::value_serde;

use super::drill::DrillState;
use super::drillthrough::DrillthroughContext;
use super::filter::coerce_literal;
use super::semantic;

/// Visual types whose selection filters other visuals like a slicer.
pub fn is_slicer_type(visual_type: &str) -> bool {
    matches!(
        visual_type,
        "slicer" | "advancedSlicerVisual" | "listSlicer" | "textSlicer"
    )
}

/// The selection of one slicer.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SlicerSelection {
    /// Selected tuples, one value per slicer field (the active `Values`
    /// projections; usually one). JSON accepts bare scalars for one field.
    #[serde(with = "value_serde::tuples")]
    pub values: Vec<Vec<Value>>,
    /// `values` are excluded instead of selected.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub inverted: bool,
    /// A persisted non-categorical selection (range, …), applied as a
    /// filter condition in addition to `values`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Condition>,
}

impl SlicerSelection {
    /// Selects `values` of a one-field slicer.
    pub fn values<I, V>(values: I) -> Self
    where
        I: IntoIterator<Item = V>,
        V: Into<Value>,
    {
        Self {
            values: values.into_iter().map(|v| vec![v.into()]).collect(),
            ..Self::default()
        }
    }

    /// Everything except `values` (inverted selection).
    pub fn excluding<I, V>(values: I) -> Self
    where
        I: IntoIterator<Item = V>,
        V: Into<Value>,
    {
        Self {
            inverted: true,
            ..Self::values(values)
        }
    }

    /// `true` if the selection does not filter.
    pub fn is_all(&self) -> bool {
        self.values.is_empty() && self.condition.is_none()
    }
}

/// One field of a data point identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PointField {
    /// The column (category / series field of the source visual).
    pub field: FieldExpr,
    #[serde(with = "value_serde")]
    pub value: Value,
}

/// Data points selected in one visual (click / ctrl-click), which
/// cross-filter the other visuals of the page.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PointSelection {
    /// Name of the visual the points were selected in.
    pub source: String,
    /// Selected points; each is the set of (field, value) pairs that
    /// identify it (category, plus series when the visual has one). Points
    /// are combined with OR, fields within a point with AND.
    pub points: Vec<Vec<PointField>>,
}

impl PointSelection {
    /// One selected point of `source`.
    pub fn point(source: impl Into<String>, fields: Vec<(FieldExpr, Value)>) -> Self {
        Self {
            source: source.into(),
            points: vec![
                fields
                    .into_iter()
                    .map(|(field, value)| PointField { field, value })
                    .collect(),
            ],
        }
    }
}

/// Interaction state of a report page.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InteractionState {
    /// Slicer visual name → its selection. Slicers not listed do not
    /// filter.
    pub slicer_selections: IndexMap<String, SlicerSelection>,
    /// Selected data points (cross-filter source).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_points: Option<PointSelection>,
    /// Visual or group name → hidden, where a bookmark changed the saved
    /// `isHidden` (see [`InteractionState::set_hidden`]).
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub visibility: IndexMap<String, bool>,
    /// Visual name → the sort order that replaces its `sortDefinition`,
    /// where a bookmark changed it (see [`InteractionState::set_sort`]).
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub sort: IndexMap<String, Vec<SortItem>>,
    /// Filter cards whose condition a bookmark changed (see
    /// [`InteractionState::set_filter`]).
    #[serde(skip_serializing_if = "FilterOverrides::is_empty")]
    pub filters: FilterOverrides,
    /// The drillthrough that filters this page (see
    /// [`drill_through`](super::drillthrough::drill_through)).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drillthrough: Option<DrillthroughContext>,
    /// Visual name → its drill range and path, where the reader drilled
    /// (see [`drill`](super::drill::drill)).
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub drill: IndexMap<String, DrillState>,
    /// The spotlighted visual (a bookmark's `spotlight` display mode): the
    /// other visuals of the page are drawn faded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spotlight: Option<String>,
    /// The visual in focus mode (the visual header's "Focus mode" and
    /// "Show as a table", a bookmark's `maximize` display mode): it fills
    /// the page and the other visuals are hidden (see
    /// [`InteractionState::page_with_visibility`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<FocusState>,
}

/// Focus mode of one visual.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FocusState {
    /// Name of the visual in focus.
    pub visual: String,
    /// "Show as a table": the visual's data as a table below it
    /// ([`data_table_visual`](super::export::data_table_visual), named
    /// [`data_table_name`](super::export::data_table_name)).
    pub show_as_table: bool,
}

/// Margin around a visual in focus mode (page units).
pub const FOCUS_MARGIN: f64 = 16.0;

/// Where a filter card lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterLevel<'a> {
    Report,
    Page,
    /// A filter of the named visual.
    Visual(&'a str),
}

/// Filter name → the condition that replaces the saved one (`None`: the
/// card does not filter), per level.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FilterOverrides {
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub report: IndexMap<String, Option<Condition>>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub page: IndexMap<String, Option<Condition>>,
    /// Visual name → its filters' overrides.
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub visuals: IndexMap<String, IndexMap<String, Option<Condition>>>,
}

impl FilterOverrides {
    /// No filter is overridden.
    pub fn is_empty(&self) -> bool {
        self.report.is_empty() && self.page.is_empty() && self.visuals.is_empty()
    }

    /// The override of filter `name` at `level`, if any.
    pub fn get(&self, level: FilterLevel<'_>, name: &str) -> Option<&Option<Condition>> {
        match level {
            FilterLevel::Report => self.report.get(name),
            FilterLevel::Page => self.page.get(name),
            FilterLevel::Visual(v) => self.visuals.get(v)?.get(name),
        }
    }

    /// Every overridden condition, for renaming fields.
    pub fn conditions_mut(&mut self) -> impl Iterator<Item = &mut Condition> {
        self.report
            .values_mut()
            .chain(self.page.values_mut())
            .chain(self.visuals.values_mut().flat_map(IndexMap::values_mut))
            .flatten()
    }

    /// `filter` as it applies: its condition replaced by the override.
    pub fn apply<'a>(&self, level: FilterLevel<'_>, filter: &'a Filter) -> Cow<'a, Filter> {
        match self.get(level, &filter.name) {
            Some(condition) if *condition != filter.condition => Cow::Owned(Filter {
                condition: condition.clone(),
                ..filter.clone()
            }),
            _ => Cow::Borrowed(filter),
        }
    }
}

impl InteractionState {
    /// No selection.
    pub fn new() -> Self {
        Self::default()
    }

    /// The persisted state of `page`: each slicer's saved selection (see
    /// the module docs). Unreadable selections are skipped and described
    /// in the returned messages.
    pub fn from_page(page: &Page) -> (Self, Vec<String>) {
        let mut state = Self::default();
        let mut problems = Vec::new();
        for v in page
            .visuals
            .iter()
            .filter(|v| is_slicer_type(&v.visual_type))
        {
            match persisted_selection(v) {
                Ok(Some(sel)) => {
                    state.slicer_selections.insert(v.name.clone(), sel);
                }
                Ok(None) => {}
                Err(reason) => problems.push(format!("slicer {}: {reason}", v.name)),
            }
        }
        (state, problems)
    }

    /// Sets (or replaces) a slicer's selection.
    pub fn select(&mut self, slicer: impl Into<String>, selection: SlicerSelection) -> &mut Self {
        self.slicer_selections.insert(slicer.into(), selection);
        self
    }

    /// Clears a slicer's selection.
    pub fn clear_slicer(&mut self, slicer: &str) -> &mut Self {
        self.slicer_selections.shift_remove(slicer);
        self
    }

    /// Shows or hides visual (or group) `name` of `page`; only a change
    /// from its saved `isHidden` is kept. Unknown names are ignored.
    pub fn set_hidden(&mut self, page: &Page, name: &str, hidden: bool) -> &mut Self {
        if let Some(v) = page.visuals.iter().find(|v| v.name == name) {
            if v.is_hidden == hidden {
                self.visibility.shift_remove(name);
            } else {
                self.visibility.insert(v.name.clone(), hidden);
            }
        }
        self
    }

    /// Sorts visual `name` of `page` by `sort` (`None`: its saved
    /// `sortDefinition`); only a change from the saved order is kept.
    /// Unknown names are ignored.
    pub fn set_sort(&mut self, page: &Page, name: &str, sort: Option<Vec<SortItem>>) -> &mut Self {
        if let Some(v) = page.visuals.iter().find(|v| v.name == name) {
            match sort {
                Some(sort) if sort != v.sort => {
                    self.sort.insert(v.name.clone(), sort);
                }
                _ => {
                    self.sort.shift_remove(name);
                }
            }
        }
        self
    }

    /// Replaces the condition of `filter` (a filter card at `level`) by
    /// `condition`; only a change from its saved condition is kept.
    pub fn set_filter(
        &mut self,
        level: FilterLevel<'_>,
        filter: &Filter,
        condition: Option<Condition>,
    ) -> &mut Self {
        let differs = condition != filter.condition;
        let map = match level {
            FilterLevel::Report => &mut self.filters.report,
            FilterLevel::Page => &mut self.filters.page,
            FilterLevel::Visual(v) => {
                if !differs {
                    if let Some(m) = self.filters.visuals.get_mut(v) {
                        m.shift_remove(&filter.name);
                        if m.is_empty() {
                            self.filters.visuals.shift_remove(v);
                        }
                    }
                    return self;
                }
                self.filters.visuals.entry(v.to_owned()).or_default()
            }
        };
        if differs {
            map.insert(filter.name.clone(), condition);
        } else {
            map.shift_remove(&filter.name);
        }
        self
    }

    /// `page` with [`Self::visibility`] applied to its visuals' `isHidden`
    /// and [`Self::focus`] applied (borrowed when nothing changes). In focus
    /// mode every other visual is hidden (hidden slicers keep filtering) and
    /// the visual, out of its group, fills the page within
    /// [`FOCUS_MARGIN`]; with "Show as a table" it takes the top half and
    /// its data table the bottom half.
    pub fn page_with_visibility<'a>(&self, page: &'a Page) -> Cow<'a, Page> {
        let differs = |v: &Visual| {
            self.visibility
                .get(&v.name)
                .is_some_and(|h| *h != v.is_hidden)
        };
        let focus = self.focus.as_ref().filter(|f| {
            page.visuals
                .iter()
                .any(|v| v.name == f.visual && !v.is_group())
        });
        if focus.is_none() && !page.visuals.iter().any(differs) {
            return Cow::Borrowed(page);
        }
        let mut page = page.clone();
        for v in &mut page.visuals {
            if let Some(hidden) = self.visibility.get(&v.name) {
                v.is_hidden = *hidden;
            }
        }
        if let Some(focus) = focus {
            self.focus_page(&mut page, focus);
        }
        Cow::Owned(page)
    }

    fn focus_page(&self, page: &mut Page, focus: &FocusState) {
        let m = FOCUS_MARGIN;
        let width = (page.width - 2.0 * m).max(1.0);
        let height = (page.height - 2.0 * m).max(1.0);
        let top = if focus.show_as_table {
            ((height - m) / 2.0).max(1.0)
        } else {
            height
        };
        let mut table = None;
        for v in &mut page.visuals {
            if v.name != focus.visual {
                v.is_hidden = true;
                continue;
            }
            v.is_hidden = false;
            v.parent_group_name = None;
            v.position = Position {
                x: m,
                y: m,
                width,
                height: top,
                angle: None,
                ..v.position.clone()
            };
            if focus.show_as_table {
                let drilled = super::drill::drilled_visual(v, self);
                let mut t = super::export::data_table_visual(&drilled);
                t.name = super::export::data_table_name(&v.name);
                if let Some(sort) = self.sort.get(&v.name) {
                    t.sort.clone_from(sort);
                }
                t.position.y = m + top + m;
                t.position.height = (height - top - m).max(1.0);
                table = Some(t);
            }
        }
        page.visuals.extend(table);
    }
}

/// The slicer's persisted selection, if any.
///
/// # Errors
/// A reason when the persisted filter cannot be read.
pub fn persisted_selection(visual: &Visual) -> Result<Option<SlicerSelection>, String> {
    let def = pbi_pbir::objects::property(&visual.objects, "general", "filter")
        .and_then(|f| f.get("filter"));
    let Some(def) = def else {
        return Ok(None);
    };
    let Some(condition) = semantic::where_condition(def) else {
        return Ok(None);
    };
    let fields = slicer_fields(visual);
    let (inner, inverted) = match &condition {
        Condition::Not(inner) => (inner.as_ref(), true),
        other => (other, false),
    };
    if let Condition::In { exprs, values } = inner
        && !fields.is_empty()
        && exprs.len() == fields.len()
        && exprs.iter().zip(&fields).all(|(a, b)| same_field(a, b))
    {
        let mut tuples = Vec::with_capacity(values.len());
        for row in values {
            let mut t = Vec::with_capacity(row.len());
            for lit in row {
                let lv: LiteralValue = lit
                    .value()
                    .map_err(|e| format!("cannot read selected value: {e}"))?;
                t.push(coerce_literal(&lv, None));
            }
            tuples.push(t);
        }
        return Ok(Some(SlicerSelection {
            values: tuples,
            inverted,
            condition: None,
        }));
    }
    Ok(Some(SlicerSelection {
        values: Vec::new(),
        inverted: false,
        condition: Some(condition),
    }))
}

/// The fields a slicer filters: its active `Values` projections.
pub fn slicer_fields(visual: &Visual) -> Vec<FieldExpr> {
    let all: Vec<_> = visual.projections_for("Values").collect();
    let any_active = all.iter().any(|p| p.active == Some(true));
    all.into_iter()
        .filter(|p| !any_active || p.active == Some(true))
        .map(|p| p.field.clone())
        .collect()
}

/// The sync group whose selection the slicer shares: `syncGroup.groupName`
/// unless `filterChanges` is `false`.
pub fn sync_group(visual: &Visual) -> Option<&str> {
    if !is_slicer_type(&visual.visual_type) {
        return None;
    }
    let group = ["syncGroup", "singleVisual.syncGroup"]
        .iter()
        .find_map(|k| match visual.extra.get(*k) {
            Some(PropValue::Json(v)) => Some(v),
            _ => None,
        })?;
    if group
        .get("filterChanges")
        .and_then(serde_json::Value::as_bool)
        == Some(false)
    {
        return None;
    }
    group
        .get("groupName")
        .and_then(serde_json::Value::as_str)
        .filter(|n| !n.is_empty())
}

/// The slicers whose selection follows slicer `visual` of page `page` (by
/// index): `(page index, visual name)` of every other slicer of the report
/// in the same [`sync_group`] over the same fields, on any page.
pub fn synced_slicers(report: &Report, page: usize, visual: &str) -> Vec<(usize, String)> {
    let Some(source) = report.pages.get(page).and_then(|p| p.visual(visual)) else {
        return Vec::new();
    };
    let Some(group) = sync_group(source) else {
        return Vec::new();
    };
    let fields = slicer_fields(source);
    let mut out = Vec::new();
    for (i, p) in report.pages.iter().enumerate() {
        for v in &p.visuals {
            if (i == page && v.name == visual) || sync_group(v) != Some(group) {
                continue;
            }
            let other = slicer_fields(v);
            if other.len() == fields.len()
                && other.iter().zip(&fields).all(|(a, b)| same_field(a, b))
            {
                out.push((i, v.name.clone()));
            }
        }
    }
    out
}

fn same_field(a: &FieldExpr, b: &FieldExpr) -> bool {
    match (a.entity(), a.property(), b.entity(), b.property()) {
        (Some(t1), Some(c1), Some(t2), Some(c2)) => {
            pbi_model::names_eq(t1, t2) && pbi_model::names_eq(c1, c2)
        }
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use pbi_model::Projection;
    use serde_json::json;

    fn slicer(objects: serde_json::Value) -> Visual {
        Visual {
            name: "s1".into(),
            visual_type: "slicer".into(),
            projections: vec![Projection {
                role: "Values".into(),
                field: FieldExpr::column("Products", "Category"),
                query_ref: "Products.Category".into(),
                active: Some(true),
                ..Projection::default()
            }],
            objects,
            ..Visual::default()
        }
    }

    fn filter_object(condition: serde_json::Value) -> serde_json::Value {
        json!({"general": [{"properties": {"filter": {"filter": {
            "Version": 2,
            "From": [{"Name": "p", "Entity": "Products", "Type": 0}],
            "Where": [{"Condition": condition}]
        }}}}]})
    }

    fn category_in(values: &[&str]) -> serde_json::Value {
        json!({"In": {
            "Expressions": [{"Column": {"Expression": {"SourceRef": {"Source": "p"}}, "Property": "Category"}}],
            "Values": values.iter().map(|v| json!([{"Literal": {"Value": format!("'{v}'")}}])).collect::<Vec<_>>()
        }})
    }

    #[test]
    fn synced_slicers_share_a_group_and_fields() {
        let synced = |name: &str, key: &str, group: serde_json::Value| {
            let mut v = slicer(serde_json::Value::Null);
            v.name = name.into();
            v.extra.insert(key.into(), PropValue::Json(group));
            v
        };
        let group = json!({"groupName": "G", "fieldChanges": true, "filterChanges": true});
        let mut other_field = synced("other", "syncGroup", group.clone());
        other_field.projections[0].field = FieldExpr::column("Products", "Name");
        let page = |name: &str, visuals: Vec<Visual>| Page {
            name: name.into(),
            visuals,
            ..Page::default()
        };
        let report = Report {
            pages: vec![
                page("p1", vec![synced("a", "syncGroup", group.clone())]),
                page(
                    "p2",
                    vec![
                        synced("b", "singleVisual.syncGroup", group.clone()),
                        synced(
                            "off",
                            "syncGroup",
                            json!({"groupName": "G", "filterChanges": false}),
                        ),
                        synced("elsewhere", "syncGroup", json!({"groupName": "H"})),
                        other_field,
                    ],
                ),
            ],
            ..Report::default()
        };
        assert_eq!(synced_slicers(&report, 0, "a"), vec![(1, "b".to_owned())]);
        assert_eq!(synced_slicers(&report, 1, "b"), vec![(0, "a".to_owned())]);
        assert!(synced_slicers(&report, 1, "off").is_empty());
    }

    #[test]
    fn persisted_in_and_not_in_selections_are_read() {
        let v = slicer(filter_object(category_in(&["Camping", "Hiking"])));
        let sel = persisted_selection(&v).unwrap().unwrap();
        assert_eq!(sel, SlicerSelection::values(["Camping", "Hiking"]));

        let v = slicer(filter_object(
            json!({"Not": {"Expression": category_in(&["Climbing"])}}),
        ));
        let sel = persisted_selection(&v).unwrap().unwrap();
        assert_eq!(sel, SlicerSelection::excluding(["Climbing"]));
    }

    #[test]
    fn inverted_mode_without_filter_selects_everything() {
        let v = slicer(
            json!({"data": [{"properties": {"isInvertedSelectionMode": {"expr": {"Literal": {"Value": "true"}}}}}]}),
        );
        assert_eq!(persisted_selection(&v).unwrap(), None);
        let page = Page {
            visuals: vec![v],
            ..Page::default()
        };
        let (state, problems) = InteractionState::from_page(&page);
        assert!(state.slicer_selections.is_empty());
        assert!(problems.is_empty());
    }

    #[test]
    fn other_conditions_are_kept_as_conditions() {
        let v = slicer(filter_object(json!({"Comparison": {
            "ComparisonKind": 2,
            "Left": {"Column": {"Expression": {"SourceRef": {"Source": "p"}}, "Property": "Category"}},
            "Right": {"Literal": {"Value": "'B'"}}
        }})));
        let sel = persisted_selection(&v).unwrap().unwrap();
        assert!(sel.values.is_empty());
        assert!(matches!(sel.condition, Some(Condition::Comparison { .. })));
    }

    #[test]
    fn state_round_trips_through_json() {
        let mut state = InteractionState::new();
        state.select("s1", SlicerSelection::values(["Camping"]));
        state.selected_points = Some(PointSelection::point(
            "pie",
            vec![(
                FieldExpr::column("Stores", "Channel"),
                Value::from("Online"),
            )],
        ));
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(
            json["slicerSelections"]["s1"]["values"],
            json!([["Camping"]])
        );
        let back: InteractionState = serde_json::from_value(json).unwrap();
        assert_eq!(back, state);
        let bare: SlicerSelection = serde_json::from_value(json!({"values": ["A", 1]})).unwrap();
        assert_eq!(
            bare.values,
            vec![vec![Value::from("A")], vec![Value::Integer(1)]]
        );
    }
}
