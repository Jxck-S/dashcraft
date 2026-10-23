//! Calculation groups (TOM `CalculationGroup`, `CalculationItem`).
//!
//! A calculation-group table carries a [`CalculationGroup`]: its
//! calculation items are DAX expressions that replace a measure reference
//! (with `SELECTEDMEASURE()` standing for the measure) when the filter
//! context selects exactly one item of the group's column. The table's rows
//! are the items (`sourceColumn: Name` and `sourceColumn: Ordinal` columns).

use std::borrow::Cow;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::table::{Column, ColumnKind, Table};
use crate::names::names_eq;
use crate::props::{Annotation, Origin, PropValue, PropertyBag, RawObject};
use crate::serde_util::is_zero;

/// The calculation group of a calculation-group table (TOM
/// `CalculationGroup`; TMDL `calculationGroup`, TMSL `calculationGroup`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CalculationGroup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Order in which several calculation groups apply to one measure: the
    /// group with the highest precedence is applied first (outermost).
    #[serde(skip_serializing_if = "is_zero")]
    pub precedence: i64,
    /// Items in source order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub calculation_items: Vec<CalculationItem>,
    /// Applied when several items, or none that exists, are selected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiple_or_empty_selection_expression: Option<CalculationGroupExpression>,
    /// Applied when the group's column is not filtered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_selection_expression: Option<CalculationGroupExpression>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    /// Untyped properties.
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
}

/// One calculation item (TOM `CalculationItem`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CalculationItem {
    /// Item name: the value of the calculation-group column.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// DAX expression; `SELECTEDMEASURE()` is the measure it applies to.
    pub expression: String,
    /// Sort position (value of the `Ordinal` column), if set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ordinal: Option<i64>,
    /// DAX expression returning the format string of the result (dynamic
    /// format string), if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_string_definition: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    /// Untyped properties (e.g. `state`, `errorMessage`).
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

impl CalculationItem {
    pub fn new(name: impl Into<String>, expression: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            expression: expression.into(),
            ..Self::default()
        }
    }
}

/// A selection expression of a calculation group
/// (`multipleOrEmptySelectionExpression`, `noSelectionExpression`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CalculationGroupExpression {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub expression: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_string_definition: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
}

impl CalculationGroupExpression {
    pub fn new(expression: impl Into<String>) -> Self {
        Self {
            expression: expression.into(),
            ..Self::default()
        }
    }
}

impl CalculationGroup {
    /// Item by name (case-insensitive).
    pub fn item(&self, name: &str) -> Option<&CalculationItem> {
        self.calculation_items
            .iter()
            .find(|i| names_eq(&i.name, name))
    }

    /// Best-effort reading of an untyped `calculationGroup` property (as
    /// kept in `Table::extra` by readers that do not type it, e.g. the
    /// VertiPaq metadata reader, or as TMSL JSON).
    pub fn from_prop(value: &PropValue) -> Option<CalculationGroup> {
        match value {
            PropValue::Object(raw) => Some(Self::from_raw(raw)),
            PropValue::Json(serde_json::Value::Object(map)) => Some(Self::from_json(map)),
            PropValue::List(items) => items.iter().find_map(Self::from_prop),
            _ => None,
        }
    }

    fn from_raw(raw: &RawObject) -> CalculationGroup {
        let mut group = CalculationGroup {
            description: raw.description.clone(),
            ..CalculationGroup::default()
        };
        for (key, value) in &raw.properties {
            match key.as_str() {
                "precedence" => group.precedence = prop_int(value).unwrap_or_default(),
                "calculationItem" => {
                    let items = match value {
                        PropValue::List(items) => items.iter().collect::<Vec<_>>(),
                        other => vec![other],
                    };
                    for item in items {
                        if let PropValue::Object(o) = item {
                            group.calculation_items.push(CalculationItem {
                                name: o.name.clone().unwrap_or_default(),
                                description: o.description.clone(),
                                expression: o.expression.clone().unwrap_or_default(),
                                ordinal: o.properties.get("ordinal").and_then(prop_int),
                                format_string_definition: o
                                    .properties
                                    .get("formatStringDefinition")
                                    .and_then(prop_expression),
                                ..CalculationItem::default()
                            });
                        }
                    }
                }
                "multipleOrEmptySelectionExpression" => {
                    group.multiple_or_empty_selection_expression = raw_selection(value);
                }
                "noSelectionExpression" => group.no_selection_expression = raw_selection(value),
                _ => {}
            }
        }
        group
    }

    fn from_json(map: &serde_json::Map<String, serde_json::Value>) -> CalculationGroup {
        let text = |v: Option<&serde_json::Value>| v.and_then(json_text);
        let selection = |v: Option<&serde_json::Value>| {
            let obj = v?.as_object()?;
            Some(CalculationGroupExpression {
                description: text(obj.get("description")),
                expression: text(obj.get("expression")).unwrap_or_default(),
                format_string_definition: obj
                    .get("formatStringDefinition")
                    .and_then(|f| text(f.get("expression"))),
                ..CalculationGroupExpression::default()
            })
        };
        CalculationGroup {
            description: text(map.get("description")),
            precedence: map
                .get("precedence")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or_default(),
            calculation_items: map
                .get("calculationItems")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_object)
                        .map(|o| CalculationItem {
                            name: text(o.get("name")).unwrap_or_default(),
                            description: text(o.get("description")),
                            expression: text(o.get("expression")).unwrap_or_default(),
                            ordinal: o.get("ordinal").and_then(serde_json::Value::as_i64),
                            format_string_definition: o
                                .get("formatStringDefinition")
                                .and_then(|f| text(f.get("expression"))),
                            ..CalculationItem::default()
                        })
                        .collect()
                })
                .unwrap_or_default(),
            multiple_or_empty_selection_expression: selection(
                map.get("multipleOrEmptySelectionExpression"),
            ),
            no_selection_expression: selection(map.get("noSelectionExpression")),
            ..CalculationGroup::default()
        }
    }
}

fn raw_selection(value: &PropValue) -> Option<CalculationGroupExpression> {
    match value {
        PropValue::Expression(e) | PropValue::Text(e) => Some(CalculationGroupExpression::new(e)),
        PropValue::Object(o) => Some(CalculationGroupExpression {
            description: o.description.clone(),
            expression: o.expression.clone().unwrap_or_default(),
            format_string_definition: o
                .properties
                .get("formatStringDefinition")
                .and_then(prop_expression),
            ..CalculationGroupExpression::default()
        }),
        PropValue::Json(v) => {
            let obj = v.as_object()?;
            Some(CalculationGroupExpression {
                expression: obj.get("expression").and_then(json_text)?,
                format_string_definition: obj
                    .get("formatStringDefinition")
                    .and_then(|f| f.get("expression"))
                    .and_then(json_text),
                ..CalculationGroupExpression::default()
            })
        }
        _ => None,
    }
}

/// Text of a JSON string or array of lines.
fn json_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(lines) => lines
            .iter()
            .map(|l| l.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
            .map(|l| l.join("\n")),
        _ => None,
    }
}

fn prop_int(value: &PropValue) -> Option<i64> {
    match value {
        PropValue::Text(t) => t.trim().parse().ok(),
        PropValue::Json(v) => v.as_i64(),
        _ => None,
    }
}

/// A DAX expression stored as an untyped property (TMDL `= expr`, a JSON
/// `{"expression": …}` object, or a raw object with an expression).
pub(crate) fn prop_expression(value: &PropValue) -> Option<String> {
    match value {
        PropValue::Expression(e) | PropValue::Text(e) => Some(e.clone()),
        PropValue::Json(v) => match v {
            serde_json::Value::Object(map) => map.get("expression").and_then(json_text),
            other => json_text(other),
        },
        PropValue::Object(o) => o.expression.clone(),
        PropValue::List(items) => items.first().and_then(prop_expression),
        PropValue::Bool(_) => None,
    }
}

impl Table {
    /// The calculation group of a calculation-group table: the typed
    /// [`Table::calculation_group`], else an untyped `calculationGroup`
    /// property read best-effort ([`CalculationGroup::from_prop`]).
    pub fn effective_calculation_group(&self) -> Option<Cow<'_, CalculationGroup>> {
        if let Some(group) = &self.calculation_group {
            return Some(Cow::Borrowed(group.as_ref()));
        }
        self.extra
            .get("calculationGroup")
            .and_then(CalculationGroup::from_prop)
            .map(Cow::Owned)
    }

    /// The column holding the calculation item names of a
    /// calculation-group table (`sourceColumn: Name`; else the first text
    /// column that is not the ordinal column).
    pub fn calculation_group_column(&self) -> Option<&Column> {
        let source_is = |c: &Column, name: &str| matches!(&c.kind, ColumnKind::Data { source_column } if names_eq(source_column, name));
        self.columns
            .iter()
            .find(|c| source_is(c, "Name"))
            .or_else(|| {
                self.columns.iter().find(|c| {
                    !source_is(c, "Ordinal")
                        && matches!(c.data_type, super::DataType::String)
                        && !matches!(c.kind, ColumnKind::RowNumber)
                })
            })
    }

    /// The ordinal column of a calculation-group table
    /// (`sourceColumn: Ordinal`).
    pub fn calculation_group_ordinal_column(&self) -> Option<&Column> {
        self.columns.iter().find(|c| {
            matches!(&c.kind, ColumnKind::Data { source_column } if names_eq(source_column, "Ordinal"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::DataType;

    #[test]
    fn untyped_raw_and_json_groups_are_read() {
        let mut item = RawObject {
            name: Some("YTD".into()),
            expression: Some("CALCULATE(SELECTEDMEASURE(), DATESYTD(D[Date]))".into()),
            ..RawObject::default()
        };
        item.properties
            .insert("ordinal".into(), PropValue::Json(serde_json::json!(2)));
        item.properties.insert(
            "formatStringDefinition".into(),
            PropValue::Expression("\"0.0\"".into()),
        );
        let mut raw = RawObject::default();
        raw.properties
            .insert("precedence".into(), PropValue::Text("7".into()));
        raw.properties.insert(
            "calculationItem".into(),
            PropValue::List(vec![PropValue::Object(Box::new(item))]),
        );
        let group = CalculationGroup::from_prop(&PropValue::Object(Box::new(raw))).unwrap();
        assert_eq!(group.precedence, 7);
        let ytd = group.item("ytd").unwrap();
        assert_eq!(ytd.ordinal, Some(2));
        assert_eq!(ytd.format_string_definition.as_deref(), Some("\"0.0\""));

        let json = serde_json::json!({
            "precedence": 3,
            "calculationItems": [{"name": "PY", "expression": ["CALCULATE(", "SELECTEDMEASURE())"]}],
            "noSelectionExpression": {"expression": "SELECTEDMEASURE()"}
        });
        let group = CalculationGroup::from_prop(&PropValue::Json(json)).unwrap();
        assert_eq!(group.precedence, 3);
        assert_eq!(
            group.item("PY").unwrap().expression,
            "CALCULATE(\nSELECTEDMEASURE())"
        );
        assert!(group.no_selection_expression.is_some());
    }

    #[test]
    fn calculation_group_columns_are_found_by_source_column() {
        let mut t = Table::new("Time Calc");
        let mut ordinal = Column::data("Ordinal", DataType::Int64);
        ordinal.is_hidden = true;
        let mut name = Column::data("Time Calculation", DataType::String);
        name.kind = ColumnKind::Data {
            source_column: "Name".into(),
        };
        t.columns = vec![ordinal, name];
        t.calculation_group = Some(Box::default());
        assert_eq!(
            t.calculation_group_column().unwrap().name,
            "Time Calculation"
        );
        assert_eq!(
            t.calculation_group_ordinal_column().unwrap().name,
            "Ordinal"
        );
        assert!(t.effective_calculation_group().is_some());
        assert_eq!(t.kind(), crate::semantic::TableKind::CalculationGroup);
    }
}
