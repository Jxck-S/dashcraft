//! Report interaction state: slicer selections and the selected data
//! points of one visual (cross-filtering).
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
//! - Not read yet: `syncGroup` (selections synced across pages; pass the
//!   same selection under each page's slicer name instead) and the
//!   `selfFilter` of the newer slicer visuals.

use indexmap::IndexMap;
use pbi_engine::Value;
use pbi_model::{Condition, FieldExpr, LiteralValue, Page, Visual};
use serde::{Deserialize, Serialize};

use crate::dataview::value_serde;

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
