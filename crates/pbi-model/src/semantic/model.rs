//! The model root and model-level collections.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::expression::NamedExpression;
use super::relationship::{ColumnRef, Relationship};
use super::table::{Column, Measure, Table};
use crate::ids::ObjectId;
use crate::names::names_eq;
use crate::props::{Annotation, Origin, PropertyBag};
use crate::serde_util::is_false;

/// The database that wraps a model (TMDL `database.tmdl`, the root object of
/// a TMSL `model.bim`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Database {
    pub id: ObjectId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// E.g. `1601`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility_level: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatibility_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

/// A Power Query folder (TOM `QueryGroup`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QueryGroup {
    pub id: ObjectId,
    /// Folder path, e.g. `Parameters` or `Sources\Sales`.
    pub folder: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
}

/// A security role (basic RLS support; members and OLS stay in `extra`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Role {
    pub id: ObjectId,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `read`, `readRefresh`, `administrator`, …
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_permission: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub table_permissions: Vec<TablePermission>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

/// Row filter of a role on one table.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TablePermission {
    pub id: ObjectId,
    /// Table name.
    pub table: String,
    /// DAX row filter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_expression: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
}

/// A culture (translations / linguistic schema). Translations stay opaque in
/// `extra`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Culture {
    pub id: ObjectId,
    /// Culture name, e.g. `en-US`.
    pub name: String,
    /// Linguistic metadata JSON text, verbatim.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linguistic_metadata: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
}

/// A perspective. Its table/column/measure membership stays opaque in `extra`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Perspective {
    pub id: ObjectId,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
}

/// Whether a field name resolves to a column or a measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldKind {
    Column,
    Measure,
}

/// The semantic model (TOM `Model`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Model {
    pub id: ObjectId,
    /// Usually `Model`.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub culture: Option<String>,
    #[serde(
        rename = "defaultPowerBIDataSourceVersion",
        skip_serializing_if = "Option::is_none"
    )]
    pub default_power_bi_data_source_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_query_culture: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub discourage_implicit_measures: bool,
    /// The enclosing database (compatibility level, …), if the source has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<Database>,
    /// Tables in source order (TMDL `ref table` order when present).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<Table>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub relationships: Vec<Relationship>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub expressions: Vec<NamedExpression>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub query_groups: Vec<QueryGroup>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<Role>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cultures: Vec<Culture>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub perspectives: Vec<Perspective>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    /// Untyped properties (e.g. `dataAccessOptions`, `defaultMode`).
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

impl Model {
    /// Table by name (case-insensitive).
    pub fn table(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|t| names_eq(&t.name, name))
    }

    /// Mutable table by name (case-insensitive).
    pub fn table_mut(&mut self, name: &str) -> Option<&mut Table> {
        self.tables.iter_mut().find(|t| names_eq(&t.name, name))
    }

    /// Index of a table by name (case-insensitive).
    pub fn table_index(&self, name: &str) -> Option<usize> {
        self.tables.iter().position(|t| names_eq(&t.name, name))
    }

    /// Table by stable ID.
    pub fn table_by_id(&self, id: ObjectId) -> Option<&Table> {
        self.tables.iter().find(|t| t.id == id)
    }

    /// Column `table[column]` (case-insensitive).
    pub fn column(&self, table: &str, column: &str) -> Option<&Column> {
        self.table(table)?.column(column)
    }

    /// Column referenced by a [`ColumnRef`] (case-insensitive).
    pub fn column_by_ref(&self, column: &ColumnRef) -> Option<&Column> {
        self.column(&column.table, &column.column)
    }

    /// Measure `table[name]` (case-insensitive).
    pub fn measure(&self, table: &str, name: &str) -> Option<&Measure> {
        self.table(table)?.measure(name)
    }

    /// Measure by name in any table (measure names are model-unique), with
    /// its home table. Used to resolve DAX `[Measure]` references.
    pub fn find_measure(&self, name: &str) -> Option<(&Table, &Measure)> {
        self.tables
            .iter()
            .find_map(|t| t.measure(name).map(|m| (t, m)))
    }

    /// Whether `name` in `table` is a column or a measure (columns win).
    pub fn find_field(&self, table: &str, name: &str) -> Option<FieldKind> {
        let t = self.table(table)?;
        if t.column(name).is_some() {
            Some(FieldKind::Column)
        } else if t.measure(name).is_some() {
            Some(FieldKind::Measure)
        } else {
            None
        }
    }

    /// Relationships with either end on `table` (case-insensitive).
    pub fn relationships_for<'a>(
        &'a self,
        table: &'a str,
    ) -> impl Iterator<Item = &'a Relationship> + 'a {
        self.relationships.iter().filter(move |r| r.involves(table))
    }

    /// Named expression by name (case-insensitive).
    pub fn expression(&self, name: &str) -> Option<&NamedExpression> {
        self.expressions.iter().find(|e| names_eq(&e.name, name))
    }

    /// Power Query parameters (named expressions with `IsParameterQuery=true`).
    pub fn parameters(&self) -> impl Iterator<Item = &NamedExpression> {
        self.expressions.iter().filter(|e| e.is_parameter())
    }

    /// Query group by folder (case-insensitive).
    pub fn query_group(&self, folder: &str) -> Option<&QueryGroup> {
        self.query_groups
            .iter()
            .find(|g| names_eq(&g.folder, folder))
    }

    /// Role by name (case-insensitive).
    pub fn role(&self, name: &str) -> Option<&Role> {
        self.roles.iter().find(|r| names_eq(&r.name, name))
    }

    /// `database.compatibilityLevel`, if known.
    pub fn compatibility_level(&self) -> Option<i64> {
        self.database.as_ref()?.compatibility_level
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::DataType;

    fn model() -> Model {
        let mut stores = Table::new("Stores");
        stores
            .columns
            .push(Column::data("StoreKey", DataType::String));
        stores.measures.push(Measure::new("Store Count", "1"));
        let mut sessions = Table::new("Web Sessions");
        sessions
            .columns
            .push(Column::data("StoreKey", DataType::String));
        Model {
            name: "Model".into(),
            tables: vec![stores, sessions, Table::new("Last Refreshed")],
            relationships: vec![Relationship::new(
                "r1",
                ColumnRef::new("Web Sessions", "StoreKey"),
                ColumnRef::new("Stores", "StoreKey"),
            )],
            expressions: vec![
                NamedExpression::m("StorageContainerUrl", "\"x\" meta [IsParameterQuery=true]"),
                NamedExpression::m("Helper", "1"),
            ],
            database: Some(Database {
                compatibility_level: Some(1601),
                ..Database::default()
            }),
            ..Model::default()
        }
    }

    #[test]
    fn lookups_are_case_insensitive() {
        let m = model();
        assert_eq!(m.table("web SESSIONS").unwrap().name, "Web Sessions");
        assert_eq!(m.table_index("LAST REFRESHED"), Some(2));
        assert!(m.column("stores", "storekey").is_some());
        assert!(
            m.column_by_ref(&ColumnRef::new("WEB SESSIONS", "STOREKEY"))
                .is_some()
        );
        assert_eq!(
            m.find_field("Stores", "STORE COUNT"),
            Some(FieldKind::Measure)
        );
        assert_eq!(m.find_field("stores", "StoreKey"), Some(FieldKind::Column));
        assert_eq!(m.find_field("Stores", "Nope"), None);
        assert_eq!(m.find_field("Nope", "StoreKey"), None);
        assert_eq!(m.find_measure("store count").unwrap().0.name, "Stores");
        assert_eq!(
            m.expression("storagecontainerurl").unwrap().name,
            "StorageContainerUrl"
        );
        assert_eq!(m.compatibility_level(), Some(1601));
    }

    #[test]
    fn relationships_and_parameters_are_filtered() {
        let mut m = model();
        assert_eq!(m.relationships_for("STORES").count(), 1);
        assert_eq!(m.relationships_for("Last Refreshed").count(), 0);
        let names: Vec<_> = m.parameters().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["StorageContainerUrl"]);
        m.table_mut("last refreshed").unwrap().is_hidden = true;
        assert!(m.tables[2].is_hidden);
    }
}
