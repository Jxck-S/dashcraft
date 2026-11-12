//! Reading state → report JSON (M9.11): what a save writes so that the
//! report opens with the reader's slicer selections and filter cards.
//!
//! - [`filter_definition`] writes a [`Condition`] as a semantic-query
//!   filter definition (`{Version, From, Where}`), the inverse of the
//!   reader in `semantic`: one `From` source per table, aliased by the
//!   table's first letter (`s`, `s1`, … on collisions), each field's
//!   `SourceRef` naming its alias.
//! - [`slicer_objects`] returns a slicer's `objects` with a selection in
//!   `general.filter` (see [`interaction`](super::interaction) for the
//!   shape): `In` for selected values, `Not(In)` plus
//!   `general.isInvertedSelectionMode` for an inverted selection, the
//!   condition for range and relative-date selections; no selection
//!   removes the filter.
//! - [`filter_with`] returns a filter card with another condition (its
//!   `filter` definition rewritten; `None` removes it).
//!
//! Expressions DashCraft keeps verbatim (`Unknown`) are written as they are
//! unless they name a source alias, which would not exist in the new
//! `From`: such a condition is refused with a reason.

use pbi_model::{Condition, FieldExpr, Filter, Literal, Visual};
use serde_json::{Map, Value as Json, json};

use super::drillthrough::literal;
use super::interaction::{SlicerSelection, slicer_fields};

/// The semantic-query filter definition of `condition`.
///
/// # Errors
/// A reason when the condition holds an expression that cannot be written.
pub fn filter_definition(condition: &Condition) -> Result<Json, String> {
    let mut aliases = Aliases::default();
    condition.visit_fields(&mut |f| aliases.add(f));
    let condition = aliases.condition(condition)?;
    let from: Vec<Json> = aliases
        .0
        .iter()
        .map(|(entity, name)| json!({"Name": name, "Entity": entity, "Type": 0}))
        .collect();
    Ok(json!({"Version": 2, "From": from, "Where": [{"Condition": condition}]}))
}

/// The slicer's `objects` with `selection` persisted (`None`, or nothing
/// selected: the filter removed).
///
/// # Errors
/// A reason when the objects are not a JSON object, the selection does not
/// match the slicer's fields or its condition cannot be written.
pub fn slicer_objects(
    visual: &Visual,
    selection: Option<&SlicerSelection>,
) -> Result<Json, String> {
    let condition = match selection {
        Some(s) if s.condition.is_some() => s.condition.clone(),
        Some(s) if !s.values.is_empty() || s.inverted => {
            let fields = slicer_fields(visual);
            if fields.is_empty() || s.values.iter().any(|t| t.len() != fields.len()) {
                return Err("the selection does not match the slicer's fields".into());
            }
            let values = s
                .values
                .iter()
                .map(|t| t.iter().map(literal).collect())
                .collect();
            let selected = Condition::In {
                exprs: fields,
                values,
            };
            Some(if s.inverted {
                Condition::Not(Box::new(selected))
            } else {
                selected
            })
        }
        _ => None,
    };
    let inverted = selection.is_some_and(|s| s.condition.is_none() && s.inverted);
    let mut objects = match &visual.objects {
        Json::Null => Map::new(),
        Json::Object(map) => map.clone(),
        _ => return Err("the slicer's objects are not a JSON object".into()),
    };
    let general = objects
        .entry("general")
        .or_insert_with(|| Json::Array(Vec::new()));
    let Json::Array(entries) = general else {
        return Err("the slicer's `general` objects are not a list".into());
    };
    let index = match entries.iter().position(|e| e.get("selector").is_none()) {
        Some(i) => i,
        None => {
            entries.push(json!({"properties": {}}));
            entries.len() - 1
        }
    };
    let Some(entry) = entries.get_mut(index).and_then(Json::as_object_mut) else {
        return Err("the slicer's `general` object is not a JSON object".into());
    };
    let properties = entry
        .entry("properties")
        .or_insert_with(|| Json::Object(Map::new()));
    let Json::Object(properties) = properties else {
        return Err("the slicer's `general` properties are not a JSON object".into());
    };
    match &condition {
        Some(c) => {
            properties.insert("filter".into(), json!({ "filter": filter_definition(c)? }));
        }
        None => {
            properties.remove("filter");
        }
    }
    if inverted {
        properties.insert(
            "isInvertedSelectionMode".into(),
            json!({"expr": {"Literal": {"Value": "true"}}}),
        );
    } else if condition.is_some() {
        properties.remove("isInvertedSelectionMode");
    }
    // Drop what this function added when nothing is left in it.
    let empty = properties.is_empty() && entry.len() == 1;
    if empty {
        entries.remove(index);
    }
    if entries.is_empty() {
        objects.remove("general");
    }
    Ok(if objects.is_empty() && visual.objects.is_null() {
        Json::Null
    } else {
        Json::Object(objects)
    })
}

/// `filter` with `condition` as its condition and in its `filter`
/// definition (`None`: the card no longer filters).
///
/// # Errors
/// A reason when the condition cannot be written or the filter's source
/// JSON is not an object.
pub fn filter_with(filter: &Filter, condition: Option<&Condition>) -> Result<Filter, String> {
    let mut raw = match &filter.raw {
        Json::Null => Map::new(),
        Json::Object(map) => map.clone(),
        _ => return Err(format!("filter `{}` is not a JSON object", filter.name)),
    };
    match condition {
        Some(c) => {
            raw.insert("filter".into(), filter_definition(c)?);
        }
        None => {
            raw.remove("filter");
        }
    }
    Ok(Filter {
        condition: condition.cloned(),
        raw: Json::Object(raw),
        ..filter.clone()
    })
}

/// Entity → alias, in order of first use.
#[derive(Default)]
struct Aliases(Vec<(String, String)>);

impl Aliases {
    fn add(&mut self, field: &FieldExpr) {
        let Some(entity) = field.entity() else {
            return;
        };
        if self.0.iter().any(|(e, _)| e == entity) {
            return;
        }
        let stem: String = entity
            .chars()
            .find(char::is_ascii_alphabetic)
            .map_or_else(|| "t".to_owned(), |c| c.to_ascii_lowercase().to_string());
        let mut name = stem.clone();
        let mut n = 1;
        while self.0.iter().any(|(_, a)| *a == name) {
            name = format!("{stem}{n}");
            n += 1;
        }
        self.0.push((entity.to_owned(), name));
    }

    fn source(&self, entity: &str) -> Result<Json, String> {
        self.0
            .iter()
            .find(|(e, _)| e == entity)
            .map(|(_, a)| json!({"SourceRef": {"Source": a}}))
            .ok_or_else(|| format!("no source for table `{entity}`"))
    }

    fn field(&self, f: &FieldExpr) -> Result<Json, String> {
        Ok(match f {
            FieldExpr::Column { entity, property } => {
                json!({"Column": {"Expression": self.source(entity)?, "Property": property}})
            }
            FieldExpr::Measure { entity, property } => {
                json!({"Measure": {"Expression": self.source(entity)?, "Property": property}})
            }
            FieldExpr::Aggregation { expr, function } => {
                json!({"Aggregation": {"Expression": self.field(expr)?, "Function": function.code()}})
            }
            FieldExpr::HierarchyLevel {
                entity,
                hierarchy,
                level,
                variation,
            } => {
                let source = self.source(entity)?;
                let inner = match variation {
                    Some(v) => json!({"PropertyVariationSource": {
                        "Expression": source, "Name": v.name, "Property": v.property
                    }}),
                    None => source,
                };
                json!({"HierarchyLevel": {
                    "Expression": {"Hierarchy": {"Expression": inner, "Hierarchy": hierarchy}},
                    "Level": level
                }})
            }
            FieldExpr::Literal(l) => lit(l),
            FieldExpr::Unknown(j) => verbatim(j)?,
        })
    }

    fn condition(&self, c: &Condition) -> Result<Json, String> {
        let pair = |l: &FieldExpr, r: &FieldExpr| -> Result<Json, String> {
            Ok(json!({"Left": self.field(l)?, "Right": self.field(r)?}))
        };
        Ok(match c {
            Condition::In { exprs, values } => {
                let exprs = exprs
                    .iter()
                    .map(|e| self.field(e))
                    .collect::<Result<Vec<_>, _>>()?;
                let values: Vec<Json> = values
                    .iter()
                    .map(|t| Json::Array(t.iter().map(lit).collect()))
                    .collect();
                json!({"In": {"Expressions": exprs, "Values": values}})
            }
            Condition::Comparison { kind, left, right } => {
                let mut p = pair(left, right)?;
                if let Some(map) = p.as_object_mut() {
                    map.insert("ComparisonKind".into(), json!(kind.code()));
                }
                json!({ "Comparison": p })
            }
            Condition::And(a, b) => {
                json!({"And": {"Left": self.condition(a)?, "Right": self.condition(b)?}})
            }
            Condition::Or(a, b) => {
                json!({"Or": {"Left": self.condition(a)?, "Right": self.condition(b)?}})
            }
            Condition::Not(inner) => json!({"Not": {"Expression": self.condition(inner)?}}),
            Condition::Contains { left, right } => json!({ "Contains": pair(left, right)? }),
            Condition::StartsWith { left, right } => json!({ "StartsWith": pair(left, right)? }),
            Condition::Between { expr, lower, upper } => json!({"Between": {
                "Expression": self.field(expr)?,
                "LowerBound": self.field(lower)?,
                "UpperBound": self.field(upper)?
            }}),
            Condition::Unknown(j) => verbatim(j)?,
        })
    }
}

fn lit(l: &Literal) -> Json {
    json!({"Literal": {"Value": l.raw}})
}

/// A verbatim expression, unless it names a source alias.
fn verbatim(j: &Json) -> Result<Json, String> {
    fn names_source(j: &Json) -> bool {
        match j {
            Json::Object(map) => map
                .iter()
                .any(|(k, v)| (k == "SourceRef" && v.get("Source").is_some()) || names_source(v)),
            Json::Array(items) => items.iter().any(names_source),
            _ => false,
        }
    }
    if names_source(j) {
        Err("the condition has an expression DashCraft cannot write".into())
    } else {
        Ok(j.clone())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::query::interaction::persisted_selection;
    use crate::query::semantic::where_condition;
    use pbi_engine::Value;
    use pbi_model::{ComparisonKind, Projection};

    fn slicer(objects: Json) -> Visual {
        Visual {
            name: "s1".into(),
            visual_type: "slicer".into(),
            projections: vec![Projection {
                role: "Values".into(),
                field: FieldExpr::column("Sales", "Region"),
                query_ref: "Sales.Region".into(),
                ..Projection::default()
            }],
            objects,
            ..Visual::default()
        }
    }

    #[test]
    fn conditions_round_trip_through_the_reader() {
        let region = FieldExpr::column("Sales", "Region");
        let store = FieldExpr::column("Stores", "Size");
        let c = Condition::And(
            Box::new(Condition::Not(Box::new(Condition::In {
                exprs: vec![region.clone()],
                values: vec![vec![Literal::text("It's")]],
            }))),
            Box::new(Condition::Or(
                Box::new(Condition::Comparison {
                    kind: ComparisonKind::GreaterThanOrEqual,
                    left: FieldExpr::aggregation(
                        store.clone(),
                        pbi_model::AggregationFunction::Sum,
                    ),
                    right: FieldExpr::Literal(Literal::integer(3)),
                }),
                Box::new(Condition::Between {
                    expr: FieldExpr::column("Sample", "Day"),
                    lower: FieldExpr::Unknown(json!({"Now": {}})),
                    upper: FieldExpr::Literal(Literal::integer(9)),
                }),
            )),
        );
        let def = filter_definition(&c).unwrap();
        let names: Vec<_> = def["From"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["Name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["s", "s1", "s2"]);
        assert_eq!(where_condition(&def), Some(c));

        let aliased = Condition::Unknown(json!({"Exists": {"SourceRef": {"Source": "x"}}}));
        assert!(filter_definition(&aliased).is_err());
    }

    #[test]
    fn slicer_selections_round_trip() {
        let plain = slicer(Json::Null);
        let north = SlicerSelection {
            values: vec![vec![Value::from("North")]],
            ..SlicerSelection::default()
        };
        let objects = slicer_objects(&plain, Some(&north)).unwrap();
        let read = persisted_selection(&slicer(objects.clone()))
            .unwrap()
            .unwrap();
        assert_eq!(read, north);

        let inverted = SlicerSelection {
            inverted: true,
            ..north.clone()
        };
        let objects = slicer_objects(&slicer(objects), Some(&inverted)).unwrap();
        assert_eq!(
            objects["general"][0]["properties"]["isInvertedSelectionMode"]["expr"]["Literal"]["Value"],
            "true"
        );
        let read = persisted_selection(&slicer(objects.clone()))
            .unwrap()
            .unwrap();
        assert_eq!(read, inverted);

        // Cleared: back to nothing, other properties kept.
        assert_eq!(slicer_objects(&plain, None).unwrap(), Json::Null);
        let kept =
            slicer(json!({"general": [{"properties": {"keep": 1, "filter": {}}}], "data": []}));
        assert_eq!(
            slicer_objects(&kept, None).unwrap(),
            json!({"general": [{"properties": {"keep": 1}}], "data": []})
        );

        let wrong = SlicerSelection {
            values: vec![vec![Value::from("a"), Value::from("b")]],
            ..SlicerSelection::default()
        };
        assert!(slicer_objects(&plain, Some(&wrong)).is_err());
    }

    #[test]
    fn filters_get_a_new_definition() {
        let f = Filter {
            name: "f".into(),
            raw: json!({"name": "f", "type": "Categorical", "filter": {"Version": 2}}),
            ..Filter::default()
        };
        let c = Condition::In {
            exprs: vec![FieldExpr::column("Sales", "Region")],
            values: vec![vec![Literal::text("East")]],
        };
        let new = filter_with(&f, Some(&c)).unwrap();
        assert_eq!(new.condition.as_ref(), Some(&c));
        assert_eq!(where_condition(&new.raw["filter"]), Some(c));
        assert_eq!(new.raw["type"], "Categorical");
        let cleared = filter_with(&f, None).unwrap();
        assert!(cleared.raw.get("filter").is_none() && cleared.condition.is_none());
    }
}
