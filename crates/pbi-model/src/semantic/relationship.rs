//! Relationships between tables.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::types::{Cardinality, CrossFilter, SecurityFilteringBehavior};
use crate::ids::ObjectId;
use crate::names::names_eq;
use crate::props::{Annotation, Origin, PropertyBag};
use crate::serde_util::{default_true, is_false, is_true};

/// Reference to a column by table and column name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ColumnRef {
    pub table: String,
    pub column: String,
}

impl ColumnRef {
    pub fn new(table: impl Into<String>, column: impl Into<String>) -> Self {
        Self {
            table: table.into(),
            column: column.into(),
        }
    }

    /// Case-insensitive comparison.
    pub fn matches(&self, table: &str, column: &str) -> bool {
        names_eq(&self.table, table) && names_eq(&self.column, column)
    }
}

/// A relationship (TOM `SingleColumnRelationship`).
///
/// Defaults follow TOM: `from` is the many side, `to` the one side,
/// single-direction cross-filtering, active.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Relationship {
    pub id: ObjectId,
    /// Usually a GUID in Power BI models.
    pub name: String,
    pub from: ColumnRef,
    pub to: ColumnRef,
    pub from_cardinality: Cardinality,
    pub to_cardinality: Cardinality,
    pub cross_filtering_behavior: CrossFilter,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub is_active: bool,
    pub security_filtering_behavior: SecurityFilteringBehavior,
    #[serde(skip_serializing_if = "is_false")]
    pub rely_on_referential_integrity: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    /// Untyped properties (e.g. `joinOnDateBehavior`).
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

impl Default for Relationship {
    fn default() -> Self {
        Self {
            id: ObjectId::default(),
            name: String::new(),
            from: ColumnRef::default(),
            to: ColumnRef::default(),
            from_cardinality: Cardinality::Many,
            to_cardinality: Cardinality::One,
            cross_filtering_behavior: CrossFilter::OneDirection,
            is_active: true,
            security_filtering_behavior: SecurityFilteringBehavior::OneDirection,
            rely_on_referential_integrity: false,
            annotations: Vec::new(),
            extra: PropertyBag::new(),
            origin: None,
        }
    }
}

impl Relationship {
    /// Many-to-one, single direction, active relationship `from → to`.
    pub fn new(name: impl Into<String>, from: ColumnRef, to: ColumnRef) -> Self {
        Self {
            name: name.into(),
            from,
            to,
            ..Self::default()
        }
    }

    /// `true` if either end is on `table` (case-insensitive).
    pub fn involves(&self, table: &str) -> bool {
        names_eq(&self.from.table, table) || names_eq(&self.to.table, table)
    }

    /// The end on `table`, if any (prefers `from` for self-relationships).
    pub fn end_for(&self, table: &str) -> Option<&ColumnRef> {
        if names_eq(&self.from.table, table) {
            Some(&self.from)
        } else if names_eq(&self.to.table, table) {
            Some(&self.to)
        } else {
            None
        }
    }

    /// The end opposite to `table`, if `table` participates.
    pub fn other_end(&self, table: &str) -> Option<&ColumnRef> {
        if names_eq(&self.from.table, table) {
            Some(&self.to)
        } else if names_eq(&self.to.table, table) {
            Some(&self.from)
        } else {
            None
        }
    }

    /// Filters propagate both ways.
    pub fn is_bidirectional(&self) -> bool {
        self.cross_filtering_behavior == CrossFilter::BothDirections
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_properties_deserialize_to_tom_defaults() {
        let r: Relationship = serde_json::from_str(
            r#"{"name":"r1","from":{"table":"Orders","column":"Store ID"},"to":{"table":"Stores","column":"Store ID"}}"#,
        )
        .unwrap();
        assert_eq!(r.from_cardinality, Cardinality::Many);
        assert_eq!(r.to_cardinality, Cardinality::One);
        assert_eq!(r.cross_filtering_behavior, CrossFilter::OneDirection);
        assert_eq!(
            r.security_filtering_behavior,
            SecurityFilteringBehavior::OneDirection
        );
        assert!(r.is_active);

        let inactive: Relationship = serde_json::from_str(r#"{"isActive":false}"#).unwrap();
        assert!(!inactive.is_active);
        let json = serde_json::to_value(&inactive).unwrap();
        assert_eq!(json["isActive"], false);
    }

    #[test]
    fn ends_are_resolved_case_insensitively() {
        let r = Relationship::new(
            "r",
            ColumnRef::new("Web Sessions", "Store ID"),
            ColumnRef::new("Stores", "Store ID"),
        );
        assert!(r.involves("web sessions"));
        assert_eq!(r.other_end("STORES").unwrap().table, "Web Sessions");
        assert_eq!(r.end_for("stores").unwrap().column, "Store ID");
        assert!(r.other_end("Last Refreshed").is_none());
        assert!(r.from.matches("WEB SESSIONS", "store id"));
    }
}
