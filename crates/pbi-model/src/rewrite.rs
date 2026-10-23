//! Reference rewriting for renames.
//!
//! [`Rename`] updates every *structured* reference to a renamed table,
//! column or measure: relationship ends, role table permissions, report
//! field expressions (projections, sorts, filters, conditions), projection
//! `queryRef`/`nativeQueryRef`, and semantic-query JSON kept verbatim in
//! `Filter::raw`, visual `objects`/`visualContainerObjects` and `extra`
//! bags (`SourceRef.Entity`, `From[].Entity`, `Column`/`Measure`
//! `Property`, selector `metadata`).
//!
//! TODO: DAX (measures, calculated columns/tables, RLS filters) and M text
//! that mention the renamed object are not rewritten yet; that needs the
//! `pbi-dax` / `pbi-m` lexers to rewrite tokens safely.

use std::collections::HashMap;

use serde_json::Value;

use crate::names::names_eq;
use crate::props::{PropValue, PropertyBag};
use crate::query::{Condition, FieldExpr, Filter};
use crate::report::Visual;
use crate::semantic::{ColumnRef, Relationship};

/// A rename to propagate to references. Matching is case-insensitive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rename {
    Table {
        old: String,
        new: String,
    },
    Column {
        table: String,
        old: String,
        new: String,
    },
    Measure {
        table: String,
        old: String,
        new: String,
    },
}

fn set_if_different(slot: &mut String, value: &str) -> bool {
    if slot == value {
        false
    } else {
        value.clone_into(slot);
        true
    }
}

impl Rename {
    /// Rewrites a field expression. Returns `true` if anything changed.
    pub fn apply_to_field(&self, expr: &mut FieldExpr) -> bool {
        match (self, expr) {
            (
                Rename::Table { old, new },
                FieldExpr::Column { entity, .. }
                | FieldExpr::Measure { entity, .. }
                | FieldExpr::HierarchyLevel { entity, .. },
            ) if names_eq(entity, old) => set_if_different(entity, new),
            (Rename::Column { table, old, new }, FieldExpr::Column { entity, property })
            | (Rename::Measure { table, old, new }, FieldExpr::Measure { entity, property })
                if names_eq(entity, table) && names_eq(property, old) =>
            {
                set_if_different(property, new)
            }
            (
                Rename::Column { table, old, new },
                FieldExpr::HierarchyLevel {
                    entity,
                    variation: Some(variation),
                    ..
                },
            ) if names_eq(entity, table) && names_eq(&variation.property, old) => {
                set_if_different(&mut variation.property, new)
            }
            (_, FieldExpr::Aggregation { expr, .. }) => self.apply_to_field(expr),
            (_, FieldExpr::Unknown(value)) => self.apply_to_json(value),
            _ => false,
        }
    }

    /// Rewrites every field of a condition, including verbatim
    /// `Condition::Unknown` JSON at any depth.
    pub fn apply_to_condition(&self, condition: &mut Condition) -> bool {
        match condition {
            Condition::And(a, b) | Condition::Or(a, b) => {
                let left = self.apply_to_condition(a);
                let right = self.apply_to_condition(b);
                left | right
            }
            Condition::Not(inner) => self.apply_to_condition(inner),
            Condition::Unknown(value) => self.apply_to_json(value),
            other => {
                let mut changed = false;
                other.visit_fields_mut(&mut |f| changed |= self.apply_to_field(f));
                changed
            }
        }
    }

    /// Rewrites a filter's field, condition and raw JSON.
    pub fn apply_to_filter(&self, filter: &mut Filter) -> bool {
        let mut changed = false;
        if let Some(field) = &mut filter.field {
            changed |= self.apply_to_field(field);
        }
        if let Some(condition) = &mut filter.condition {
            changed |= self.apply_to_condition(condition);
        }
        changed |= self.apply_to_json(&mut filter.raw);
        changed
    }

    pub fn apply_to_filters(&self, filters: &mut [Filter]) -> bool {
        filters
            .iter_mut()
            .fold(false, |acc, f| self.apply_to_filter(f) | acc)
    }

    /// Rewrites projections (including `queryRef`/`nativeQueryRef`), sorts,
    /// filters, `objects`, `container_objects` and JSON in `extra`.
    pub fn apply_to_visual(&self, visual: &mut Visual) -> bool {
        let mut changed = false;
        for p in &mut visual.projections {
            let old_property = p.field.property().map(str::to_owned);
            if !self.apply_to_field(&mut p.field) {
                continue;
            }
            changed = true;
            if let Some(query_ref) = self.apply_to_query_ref(&p.query_ref) {
                p.query_ref = query_ref;
            }
            // nativeQueryRef is the bare field name for un-aggregated fields.
            let renamed_field = matches!(self, Rename::Column { .. } | Rename::Measure { .. })
                && !matches!(p.field, FieldExpr::Aggregation { .. });
            if let (true, Some(native), Some(old), Some(new)) = (
                renamed_field,
                p.native_query_ref.as_mut(),
                old_property,
                p.field.property(),
            ) && names_eq(native, &old)
            {
                set_if_different(native, new);
            }
        }
        for s in &mut visual.sort {
            changed |= self.apply_to_field(&mut s.field);
        }
        changed |= self.apply_to_filters(&mut visual.filters);
        changed |= self.apply_to_json(&mut visual.objects);
        changed |= self.apply_to_json(&mut visual.container_objects);
        changed |= self.apply_to_bag(&mut visual.extra);
        changed
    }

    /// Rewrites a relationship end.
    pub fn apply_to_column_ref(&self, column: &mut ColumnRef) -> bool {
        match self {
            Rename::Table { old, new } if names_eq(&column.table, old) => {
                set_if_different(&mut column.table, new)
            }
            Rename::Column { table, old, new }
                if names_eq(&column.table, table) && names_eq(&column.column, old) =>
            {
                set_if_different(&mut column.column, new)
            }
            _ => false,
        }
    }

    pub fn apply_to_relationship(&self, relationship: &mut Relationship) -> bool {
        let from = self.apply_to_column_ref(&mut relationship.from);
        let to = self.apply_to_column_ref(&mut relationship.to);
        from | to
    }

    /// Rewrites a `queryRef`-style string (`Table.Field` or
    /// `Func(Table.Field)`). Returns the new string if it changed.
    pub fn apply_to_query_ref(&self, query_ref: &str) -> Option<String> {
        let (prefix, inner, suffix) = match (query_ref.find('('), query_ref.strip_suffix(')')) {
            (Some(open), Some(body)) if open < body.len() => {
                (&query_ref[..=open], &body[open + 1..], ")")
            }
            _ => ("", query_ref, ""),
        };
        let rewritten = match self {
            Rename::Table { old, new } => {
                let head = inner.get(..old.len())?;
                let rest = inner.get(old.len()..)?;
                if !names_eq(head, old) || !rest.starts_with('.') {
                    return None;
                }
                format!("{new}{rest}")
            }
            Rename::Column { table, old, new } | Rename::Measure { table, old, new } => {
                let head = inner.get(..table.len())?;
                let rest = inner.get(table.len()..)?.strip_prefix('.')?;
                if !names_eq(head, table) || !names_eq(rest, old) {
                    return None;
                }
                format!("{head}.{new}")
            }
        };
        let out = format!("{prefix}{rewritten}{suffix}");
        (out != query_ref).then_some(out)
    }

    /// Rewrites semantic-query JSON (PBIR shapes) in place.
    pub fn apply_to_json(&self, value: &mut Value) -> bool {
        if value.is_null() {
            return false;
        }
        let mut aliases = HashMap::new();
        if !matches!(self, Rename::Table { .. }) {
            collect_aliases(value, &mut aliases);
        }
        self.rewrite_json(value, &aliases)
    }

    /// Rewrites JSON values inside a property bag.
    pub fn apply_to_bag(&self, bag: &mut PropertyBag) -> bool {
        bag.values_mut()
            .fold(false, |acc, v| self.apply_to_prop(v) | acc)
    }

    fn apply_to_prop(&self, value: &mut PropValue) -> bool {
        match value {
            PropValue::Json(v) => self.apply_to_json(v),
            PropValue::Object(o) => self.apply_to_bag(&mut o.properties),
            PropValue::List(items) => items
                .iter_mut()
                .fold(false, |acc, v| self.apply_to_prop(v) | acc),
            PropValue::Bool(_) | PropValue::Text(_) | PropValue::Expression(_) => false,
        }
    }

    fn rewrite_json(&self, value: &mut Value, aliases: &HashMap<String, String>) -> bool {
        let mut changed = false;
        match value {
            Value::Object(map) => {
                match self {
                    Rename::Table { old, new } => {
                        if let Some(Value::String(entity)) = map.get_mut("Entity")
                            && names_eq(entity, old)
                        {
                            changed |= set_if_different(entity, new);
                        }
                    }
                    Rename::Column { table, old, new } | Rename::Measure { table, old, new } => {
                        let keys: &[&str] = if matches!(self, Rename::Column { .. }) {
                            &["Column", "PropertyVariationSource"]
                        } else {
                            &["Measure"]
                        };
                        for key in keys {
                            let Some(Value::Object(field)) = map.get_mut(*key) else {
                                continue;
                            };
                            let entity = field
                                .get("Expression")
                                .and_then(|e| resolve_entity(e, aliases));
                            if !entity.is_some_and(|e| names_eq(&e, table)) {
                                continue;
                            }
                            if let Some(Value::String(property)) = field.get_mut("Property")
                                && names_eq(property, old)
                            {
                                changed |= set_if_different(property, new);
                            }
                        }
                    }
                }
                for key in ["metadata", "queryRef"] {
                    if let Some(Value::String(s)) = map.get_mut(key)
                        && let Some(rewritten) = self.apply_to_query_ref(s)
                    {
                        *s = rewritten;
                        changed = true;
                    }
                }
                for child in map.values_mut() {
                    changed |= self.rewrite_json(child, aliases);
                }
            }
            Value::Array(items) => {
                for child in items {
                    changed |= self.rewrite_json(child, aliases);
                }
            }
            _ => {}
        }
        changed
    }
}

/// Collects `From: [{Name, Entity}]` aliases anywhere in `value`.
fn collect_aliases(value: &Value, aliases: &mut HashMap<String, String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::Array(sources)) = map.get("From") {
                for s in sources {
                    if let (Some(Value::String(name)), Some(Value::String(entity))) =
                        (s.get("Name"), s.get("Entity"))
                    {
                        aliases.insert(name.clone(), entity.clone());
                    }
                }
            }
            map.values().for_each(|v| collect_aliases(v, aliases));
        }
        Value::Array(items) => items.iter().for_each(|v| collect_aliases(v, aliases)),
        _ => {}
    }
}

/// Table name of a `{"SourceRef": {"Entity"|"Source": …}}` expression.
fn resolve_entity(expression: &Value, aliases: &HashMap<String, String>) -> Option<String> {
    let source_ref = expression.get("SourceRef")?;
    if let Some(Value::String(entity)) = source_ref.get("Entity") {
        return Some(entity.clone());
    }
    match source_ref.get("Source") {
        Some(Value::String(alias)) => aliases.get(alias).cloned(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::AggregationFunction;
    use serde_json::json;

    fn table(old: &str, new: &str) -> Rename {
        Rename::Table {
            old: old.into(),
            new: new.into(),
        }
    }

    fn column(table: &str, old: &str, new: &str) -> Rename {
        Rename::Column {
            table: table.into(),
            old: old.into(),
            new: new.into(),
        }
    }

    #[test]
    fn field_expressions_are_rewritten_case_insensitively() {
        let mut f = FieldExpr::aggregation(
            FieldExpr::column("stores", "Region"),
            AggregationFunction::Sum,
        );
        assert!(table("Stores", "Shops").apply_to_field(&mut f));
        assert_eq!(f.entity(), Some("Shops"));
        assert!(column("SHOPS", "region", "Area").apply_to_field(&mut f));
        assert_eq!(f.property(), Some("Area"));
        // A measure rename does not touch columns with the same name.
        let mut c = FieldExpr::column("Shops", "Area");
        let measure = Rename::Measure {
            table: "Shops".into(),
            old: "Area".into(),
            new: "X".into(),
        };
        assert!(!measure.apply_to_field(&mut c));
        // Renaming to the identical name reports no change.
        assert!(!table("Shops", "Shops").apply_to_field(&mut c));
    }

    #[test]
    fn query_refs_are_rewritten() {
        let r = table("Web Sessions", "Sessions");
        assert_eq!(
            r.apply_to_query_ref("Web Sessions.Device").as_deref(),
            Some("Sessions.Device")
        );
        assert_eq!(
            r.apply_to_query_ref("CountNonNull(web sessions.Device)")
                .as_deref(),
            Some("CountNonNull(Sessions.Device)")
        );
        assert_eq!(r.apply_to_query_ref("Web Sessionsx.Device"), None);
        let c = column("Stores", "Region", "Area");
        assert_eq!(
            c.apply_to_query_ref("Stores.Region").as_deref(),
            Some("Stores.Area")
        );
        assert_eq!(c.apply_to_query_ref("Stores.Region2"), None);
        assert_eq!(c.apply_to_query_ref("Ä"), None);
    }

    #[test]
    fn semantic_query_json_is_rewritten_through_aliases() {
        let mut filter = json!({
            "Version": 2,
            "From": [{"Name": "s", "Entity": "Stores", "Type": 0}],
            "Where": [{"Condition": {"In": {
                "Expressions": [{"Column": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "Region"}}],
                "Values": [[{"Literal": {"Value": "'West'"}}]]
            }}}]
        });
        let original = filter.clone();
        assert!(column("Stores", "Region", "Area").apply_to_json(&mut filter));
        assert_eq!(
            filter["Where"][0]["Condition"]["In"]["Expressions"][0]["Column"]["Property"],
            "Area"
        );
        assert!(table("Stores", "Shops").apply_to_json(&mut filter));
        assert_eq!(filter["From"][0]["Entity"], "Shops");
        // Literal values are never touched.
        assert_eq!(
            filter["Where"][0]["Condition"]["In"]["Values"],
            original["Where"][0]["Condition"]["In"]["Values"]
        );

        let mut selector = json!({"selector": {"metadata": "Stores.Region"}});
        assert!(column("Stores", "Region", "Area").apply_to_json(&mut selector));
        assert_eq!(selector["selector"]["metadata"], "Stores.Area");
        assert!(!column("Other", "Area", "X").apply_to_json(&mut selector));
    }

    #[test]
    fn relationship_ends_are_rewritten() {
        let mut r = Relationship::new(
            "r",
            ColumnRef::new("Orders", "Store ID"),
            ColumnRef::new("Stores", "Store ID"),
        );
        assert!(column("orders", "store id", "Key").apply_to_relationship(&mut r));
        assert_eq!(r.from, ColumnRef::new("Orders", "Key"));
        assert_eq!(r.to, ColumnRef::new("Stores", "Store ID"));
        assert!(table("Stores", "Shops").apply_to_relationship(&mut r));
        assert_eq!(r.to.table, "Shops");
    }
}
