//! Raw PBIR semantic-query JSON (`{Version, From, Where}`) → report-AST
//! [`Condition`]s and [`FieldExpr`]s.
//!
//! The report AST already carries converted report/page/visual filters;
//! this module covers the JSON the AST keeps verbatim: a slicer's
//! persisted selection (`objects.general[0].properties.filter.filter`) and
//! Top N filter subqueries (`filter.From[].Expression.Subquery.Query`).
//! Anything it does not recognise becomes `Unknown(json)`.

use std::collections::HashMap;

use pbi_model::{
    AggregationFunction, ComparisonKind, Condition, FieldExpr, Literal, PropertyVariation,
};
use serde_json::Value as Json;

/// `From[]` alias → entity.
#[derive(Debug, Clone, Default)]
pub(crate) struct Aliases(HashMap<String, String>);

impl Aliases {
    /// Aliases of a query definition's `From` (table sources only).
    pub(crate) fn of(def: &Json) -> Self {
        let mut map = HashMap::new();
        for source in def
            .get("From")
            .and_then(Json::as_array)
            .into_iter()
            .flatten()
        {
            if let (Some(name), Some(entity)) = (
                source.get("Name").and_then(Json::as_str),
                source.get("Entity").and_then(Json::as_str),
            ) {
                map.insert(name.to_owned(), entity.to_owned());
            }
        }
        Self(map)
    }

    fn entity_of(&self, expr: &Json) -> Option<String> {
        let source_ref = expr.get("SourceRef")?;
        if let Some(entity) = source_ref.get("Entity").and_then(Json::as_str) {
            return Some(entity.to_owned());
        }
        let alias = source_ref.get("Source").and_then(Json::as_str)?;
        self.0.get(alias).cloned()
    }
}

/// A field expression.
pub(crate) fn field(expr: &Json, aliases: &Aliases) -> FieldExpr {
    let unknown = || FieldExpr::Unknown(expr.clone());
    if let Some(p) = expr.get("Column").or_else(|| expr.get("Measure")) {
        let entity = p.get("Expression").and_then(|e| aliases.entity_of(e));
        let property = p.get("Property").and_then(Json::as_str);
        return match (entity, property) {
            (Some(entity), Some(property)) if expr.get("Column").is_some() => {
                FieldExpr::column(entity, property)
            }
            (Some(entity), Some(property)) => FieldExpr::measure(entity, property),
            _ => unknown(),
        };
    }
    if let Some(a) = expr.get("Aggregation") {
        let function = a.get("Function").and_then(Json::as_i64);
        return match (a.get("Expression"), function) {
            (Some(inner), Some(code)) => {
                FieldExpr::aggregation(field(inner, aliases), AggregationFunction::from_code(code))
            }
            _ => unknown(),
        };
    }
    if let Some(hl) = expr.get("HierarchyLevel") {
        let level = hl.get("Level").and_then(Json::as_str);
        let hierarchy = hl.get("Expression").and_then(|e| e.get("Hierarchy"));
        let name = hierarchy
            .and_then(|h| h.get("Hierarchy"))
            .and_then(Json::as_str);
        let inner = hierarchy.and_then(|h| h.get("Expression"));
        let (entity, variation) = match inner {
            Some(e) if e.get("PropertyVariationSource").is_some() => {
                let pvs = &e["PropertyVariationSource"];
                (
                    pvs.get("Expression").and_then(|x| aliases.entity_of(x)),
                    Some(PropertyVariation {
                        name: pvs
                            .get("Name")
                            .and_then(Json::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        property: pvs
                            .get("Property")
                            .and_then(Json::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    }),
                )
            }
            Some(e) => (aliases.entity_of(e), None),
            None => (None, None),
        };
        return match (entity, name, level) {
            (Some(entity), Some(hierarchy), Some(level)) => FieldExpr::HierarchyLevel {
                entity,
                hierarchy: hierarchy.to_owned(),
                level: level.to_owned(),
                variation,
            },
            _ => unknown(),
        };
    }
    if let Some(value) = expr
        .get("Literal")
        .and_then(|l| l.get("Value"))
        .and_then(Json::as_str)
    {
        return FieldExpr::Literal(Literal::new(value));
    }
    unknown()
}

fn pair<'a>(expr: &'a Json, left: &str, right: &str) -> Option<(&'a Json, &'a Json)> {
    Some((expr.get(left)?, expr.get(right)?))
}

/// A boolean condition.
pub(crate) fn condition(expr: &Json, aliases: &Aliases) -> Condition {
    let unknown = || Condition::Unknown(expr.clone());
    if let Some(i) = expr.get("In") {
        if i.get("Table").is_some() {
            return unknown();
        }
        let Some(exprs) = i.get("Expressions").and_then(Json::as_array) else {
            return unknown();
        };
        let mut values = Vec::new();
        for row in i
            .get("Values")
            .and_then(Json::as_array)
            .into_iter()
            .flatten()
        {
            let Some(row) = row.as_array() else {
                return unknown();
            };
            let mut tuple = Vec::with_capacity(row.len());
            for v in row {
                match v
                    .get("Literal")
                    .and_then(|l| l.get("Value"))
                    .and_then(Json::as_str)
                {
                    Some(text) => tuple.push(Literal::new(text)),
                    None => return unknown(),
                }
            }
            values.push(tuple);
        }
        return Condition::In {
            exprs: exprs.iter().map(|e| field(e, aliases)).collect(),
            values,
        };
    }
    if let Some(c) = expr.get("Comparison") {
        let kind = c
            .get("ComparisonKind")
            .and_then(Json::as_i64)
            .and_then(ComparisonKind::from_code);
        return match (kind, pair(c, "Left", "Right")) {
            (Some(kind), Some((l, r))) => Condition::Comparison {
                kind,
                left: field(l, aliases),
                right: field(r, aliases),
            },
            _ => unknown(),
        };
    }
    for key in ["And", "Or"] {
        if let Some(b) = expr.get(key) {
            let Some((l, r)) = pair(b, "Left", "Right") else {
                return unknown();
            };
            let l = Box::new(condition(l, aliases));
            let r = Box::new(condition(r, aliases));
            return if key == "And" {
                Condition::And(l, r)
            } else {
                Condition::Or(l, r)
            };
        }
    }
    if let Some(n) = expr.get("Not") {
        return match n.get("Expression") {
            Some(inner) => Condition::Not(Box::new(condition(inner, aliases))),
            None => unknown(),
        };
    }
    for key in ["Contains", "StartsWith"] {
        if let Some(b) = expr.get(key) {
            let Some((l, r)) = pair(b, "Left", "Right") else {
                return unknown();
            };
            let left = field(l, aliases);
            let right = field(r, aliases);
            return if key == "Contains" {
                Condition::Contains { left, right }
            } else {
                Condition::StartsWith { left, right }
            };
        }
    }
    if let Some(b) = expr.get("Between") {
        return match (
            b.get("Expression"),
            b.get("LowerBound"),
            b.get("UpperBound"),
        ) {
            (Some(e), Some(lo), Some(hi)) => Condition::Between {
                expr: field(e, aliases),
                lower: field(lo, aliases),
                upper: field(hi, aliases),
            },
            _ => unknown(),
        };
    }
    unknown()
}

/// The `Where` conditions of a filter definition, combined with `And`.
pub(crate) fn where_condition(def: &Json) -> Option<Condition> {
    let aliases = Aliases::of(def);
    let conditions = def
        .get("Where")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
        .filter_map(|w| w.get("Condition"))
        .map(|c| condition(c, &aliases));
    Condition::and_all(conditions)
}

/// A parsed Top N filter: keep the `count` values of `columns` with the
/// highest (or lowest) `order_by`.
#[derive(Debug, Clone, PartialEq)]
pub struct TopNSpec {
    pub columns: Vec<FieldExpr>,
    pub order_by: FieldExpr,
    pub descending: bool,
    pub count: i64,
}

/// Parses a Top N filter definition (`filter` of a `TopN` filter card):
/// `From` has a subquery source with `Select`, `OrderBy` and `Top`, and
/// `Where` is `In { Expressions, Table: subquery }`.
pub(crate) fn top_n(def: &Json) -> Option<TopNSpec> {
    let outer = Aliases::of(def);
    let query = def
        .get("From")
        .and_then(Json::as_array)?
        .iter()
        .find_map(|s| s.get("Expression")?.get("Subquery")?.get("Query"))?;
    let inner = Aliases::of(query);
    let order = query.get("OrderBy").and_then(Json::as_array)?.first()?;
    let order_by = field(order.get("Expression")?, &inner);
    let descending = order.get("Direction").and_then(Json::as_i64) != Some(1);
    let count = query.get("Top").and_then(Json::as_i64)?;
    let columns: Vec<FieldExpr> = def
        .get("Where")
        .and_then(Json::as_array)
        .and_then(|w| w.first())
        .and_then(|w| w.get("Condition")?.get("In")?.get("Expressions"))
        .and_then(Json::as_array)
        .map(|exprs| exprs.iter().map(|e| field(e, &outer)).collect())
        .or_else(|| {
            query
                .get("Select")
                .and_then(Json::as_array)
                .map(|s| s.iter().map(|e| field(e, &inner)).collect())
        })?;
    if columns.is_empty() {
        return None;
    }
    Some(TopNSpec {
        columns,
        order_by,
        descending,
        count,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use serde_json::json;

    fn col(alias: &str, prop: &str) -> Json {
        json!({"Column": {"Expression": {"SourceRef": {"Source": alias}}, "Property": prop}})
    }

    #[test]
    fn slicer_filter_definitions_convert() {
        let def = json!({
            "Version": 2,
            "From": [{"Name": "c", "Entity": "Customers", "Type": 0}],
            "Where": [{"Condition": {"Not": {"Expression": {"In": {
                "Expressions": [col("c", "Loyalty Tier")],
                "Values": [[{"Literal": {"Value": "'Unassigned'"}}], [{"Literal": {"Value": "null"}}]]
            }}}}}]
        });
        let cond = where_condition(&def).unwrap();
        let Condition::Not(inner) = cond else {
            panic!("expected Not");
        };
        assert_eq!(
            *inner,
            Condition::In {
                exprs: vec![FieldExpr::column("Customers", "Loyalty Tier")],
                values: vec![vec![Literal::text("Unassigned")], vec![Literal::null()]],
            }
        );
    }

    #[test]
    fn comparisons_and_unknown_shapes() {
        let aliases = Aliases::of(&json!({"From": [{"Name": "w", "Entity": "Web Sessions"}]}));
        let c = condition(
            &json!({"And": {
                "Left": {"Comparison": {"ComparisonKind": 2, "Left": col("w", "N"), "Right": {"Literal": {"Value": "5L"}}}},
                "Right": {"Contains": {"Left": col("w", "Device"), "Right": {"Literal": {"Value": "'mob'"}}}}
            }}),
            &aliases,
        );
        let Condition::And(a, b) = c else {
            panic!("expected And");
        };
        assert!(matches!(
            *a,
            Condition::Comparison {
                kind: ComparisonKind::GreaterThanOrEqual,
                ..
            }
        ));
        assert!(matches!(*b, Condition::Contains { .. }));
        assert!(matches!(
            condition(&json!({"Exists": {}}), &aliases),
            Condition::Unknown(_)
        ));
        assert!(matches!(
            field(&col("zz", "x"), &aliases),
            FieldExpr::Unknown(_)
        ));
    }

    #[test]
    fn top_n_subqueries_parse() {
        let def = json!({
            "Version": 2,
            "From": [
                {"Name": "subquery", "Expression": {"Subquery": {"Query": {
                    "Version": 2,
                    "From": [{"Name": "p", "Entity": "Products", "Type": 0}],
                    "Select": [col("p", "Product Name")],
                    "OrderBy": [{"Direction": 2, "Expression": {"Measure": {"Expression": {"SourceRef": {"Source": "p"}}, "Property": "Total Sales"}}}],
                    "Top": 3
                }}}, "Type": 2},
                {"Name": "p", "Entity": "Products", "Type": 0}
            ],
            "Where": [{"Condition": {"In": {
                "Expressions": [col("p", "Product Name")],
                "Table": {"SourceRef": {"Source": "subquery"}}
            }}}]
        });
        let spec = top_n(&def).unwrap();
        assert_eq!(
            spec,
            TopNSpec {
                columns: vec![FieldExpr::column("Products", "Product Name")],
                order_by: FieldExpr::measure("Products", "Total Sales"),
                descending: true,
                count: 3,
            }
        );
        assert!(top_n(&json!({"From": []})).is_none());
    }
}
