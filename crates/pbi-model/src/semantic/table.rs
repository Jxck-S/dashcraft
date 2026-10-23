//! Tables and their children: columns, measures, partitions, hierarchies.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::calc_group::{CalculationGroup, prop_expression};
use super::types::{DataType, PartitionMode, SummarizeBy};
use crate::ids::ObjectId;
use crate::macros::lenient_eq;
use crate::names::names_eq;
use crate::props::{Annotation, Origin, PropertyBag};
use crate::serde_util::is_false;

/// What kind of table this is, derived from its partitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TableKind {
    /// Data comes from M (Power Query) or a native query; can be refreshed locally.
    Regular,
    /// DAX calculated table (`partition … = calculated`).
    CalculatedTable,
    /// Calculation group.
    CalculationGroup,
    /// Data lives elsewhere and is not available offline: entity partitions
    /// (composite models / remote semantic models), DirectQuery or Direct Lake.
    Remote,
}

/// A model table (TOM `Table`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Table {
    pub id: ObjectId,
    pub name: String,
    /// From TMDL `///` lines or TMSL `description`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lineage_tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_lineage_tag: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub is_hidden: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub is_private: bool,
    /// E.g. `Time` for date tables.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_category: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<Column>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub measures: Vec<Measure>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub partitions: Vec<Partition>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hierarchies: Vec<Hierarchy>,
    /// The calculation group of a calculation-group table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calculation_group: Option<Box<CalculationGroup>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    /// Untyped properties (e.g. `showAsVariationsOnly`; a `calculationGroup`
    /// a reader does not type).
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

impl Table {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// Classifies the table from its partitions (see [`TableKind`]).
    pub fn kind(&self) -> TableKind {
        let sources = self.partitions.iter().map(|p| &p.source);
        if sources
            .clone()
            .any(|s| matches!(s, PartitionSource::Calculated { .. }))
        {
            return TableKind::CalculatedTable;
        }
        if self.calculation_group.is_some()
            || self.extra.contains_key("calculationGroup")
            || sources.clone().any(
                |s| matches!(s, PartitionSource::Unknown { kind, .. } if lenient_eq(kind, "calculationGroup")),
            )
        {
            return TableKind::CalculationGroup;
        }
        let remote = self.partitions.iter().any(|p| {
            matches!(p.source, PartitionSource::Entity { .. })
                || matches!(
                    p.mode,
                    PartitionMode::DirectQuery | PartitionMode::DirectLake
                )
        });
        if remote {
            TableKind::Remote
        } else {
            TableKind::Regular
        }
    }

    /// DAX expression of a calculated table.
    pub fn calculated_expression(&self) -> Option<&str> {
        self.partitions.iter().find_map(|p| match &p.source {
            PartitionSource::Calculated { expression } => Some(expression.as_str()),
            _ => None,
        })
    }

    /// Column by name (case-insensitive).
    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| names_eq(&c.name, name))
    }

    pub fn column_mut(&mut self, name: &str) -> Option<&mut Column> {
        self.columns.iter_mut().find(|c| names_eq(&c.name, name))
    }

    /// Measure by name (case-insensitive).
    pub fn measure(&self, name: &str) -> Option<&Measure> {
        self.measures.iter().find(|m| names_eq(&m.name, name))
    }

    pub fn measure_mut(&mut self, name: &str) -> Option<&mut Measure> {
        self.measures.iter_mut().find(|m| names_eq(&m.name, name))
    }

    /// Hierarchy by name (case-insensitive).
    pub fn hierarchy(&self, name: &str) -> Option<&Hierarchy> {
        self.hierarchies.iter().find(|h| names_eq(&h.name, name))
    }

    /// Partition by name (case-insensitive).
    pub fn partition(&self, name: &str) -> Option<&Partition> {
        self.partitions.iter().find(|p| names_eq(&p.name, name))
    }
}

/// How a column gets its values (TOM `ColumnType`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ColumnKind {
    /// Loaded from the partition's source column.
    Data {
        #[serde(default)]
        source_column: String,
    },
    /// DAX calculated column (`column X = <dax>`).
    Calculated {
        #[serde(default)]
        expression: String,
    },
    /// Column of a calculated table; `source_column` is the DAX output name
    /// (often bracketed, e.g. `[Product Name]`).
    CalculatedTableColumn {
        #[serde(default)]
        source_column: String,
    },
    /// Hidden internal row-number column (TMSL only).
    RowNumber,
}

impl Default for ColumnKind {
    fn default() -> Self {
        ColumnKind::Data {
            source_column: String::new(),
        }
    }
}

/// A table column (TOM `Column`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Column {
    pub id: ObjectId,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lineage_tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_lineage_tag: Option<String>,
    pub data_type: DataType,
    pub kind: ColumnKind,
    #[serde(skip_serializing_if = "is_false")]
    pub is_hidden: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub is_key: bool,
    /// Calculated-table columns: name was inferred from the DAX expression.
    #[serde(skip_serializing_if = "is_false")]
    pub is_name_inferred: bool,
    /// Calculated-table columns: type was inferred from the DAX expression.
    #[serde(skip_serializing_if = "is_false")]
    pub is_data_type_inferred: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_string: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summarize_by: Option<SummarizeBy>,
    /// Name of the sort-by column in the same table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_by_column: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_folder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_category: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    /// Untyped properties (e.g. `isAvailableInMdx`, `variation`, `encodingHint`).
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

impl Column {
    /// A data column whose source column has the same name.
    pub fn data(name: impl Into<String>, data_type: DataType) -> Self {
        let name = name.into();
        Self {
            kind: ColumnKind::Data {
                source_column: name.clone(),
            },
            name,
            data_type,
            ..Self::default()
        }
    }

    /// A DAX calculated column.
    pub fn calculated(
        name: impl Into<String>,
        data_type: DataType,
        expression: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            data_type,
            kind: ColumnKind::Calculated {
                expression: expression.into(),
            },
            ..Self::default()
        }
    }

    pub fn is_calculated(&self) -> bool {
        matches!(self.kind, ColumnKind::Calculated { .. })
    }

    /// DAX expression of a calculated column.
    pub fn expression(&self) -> Option<&str> {
        match &self.kind {
            ColumnKind::Calculated { expression } => Some(expression),
            _ => None,
        }
    }

    /// Source column of a data or calculated-table column.
    pub fn source_column(&self) -> Option<&str> {
        match &self.kind {
            ColumnKind::Data { source_column }
            | ColumnKind::CalculatedTableColumn { source_column } => Some(source_column),
            _ => None,
        }
    }
}

/// A DAX measure (TOM `Measure`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Measure {
    pub id: ObjectId,
    pub name: String,
    /// DAX text, verbatim (including line breaks and comments).
    pub expression: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_string: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_folder: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub is_hidden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lineage_tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_category: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    /// Untyped properties (e.g. `formatStringDefinition`, `kpi`).
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

impl Measure {
    pub fn new(name: impl Into<String>, expression: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            expression: expression.into(),
            ..Self::default()
        }
    }

    /// DAX of the measure's dynamic format string (`formatStringDefinition`,
    /// kept in `extra`), if any.
    pub fn format_string_definition(&self) -> Option<String> {
        self.extra
            .get("formatStringDefinition")
            .and_then(prop_expression)
            .filter(|e| !e.trim().is_empty())
    }
}

/// Where a partition's rows come from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PartitionSource {
    /// Power Query M (`partition X = m`).
    M {
        #[serde(default)]
        expression: String,
    },
    /// DAX calculated table (`partition X = calculated`).
    Calculated {
        #[serde(default)]
        expression: String,
    },
    /// Entity in a remote model / Direct Lake (`partition X = entity`).
    Entity {
        #[serde(default)]
        entity_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expression_source: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        schema_name: Option<String>,
    },
    /// Legacy provider/native query (`partition X = query`).
    Query {
        #[serde(default)]
        query: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data_source: Option<String>,
    },
    /// Any other source type (`calculationGroup`, `policyRange`, …).
    Unknown {
        #[serde(default)]
        kind: String,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        props: PropertyBag,
    },
}

impl Default for PartitionSource {
    fn default() -> Self {
        PartitionSource::M {
            expression: String::new(),
        }
    }
}

impl PartitionSource {
    /// TMDL source type keyword (`m`, `calculated`, `entity`, `query`, …).
    pub fn kind_name(&self) -> &str {
        match self {
            PartitionSource::M { .. } => "m",
            PartitionSource::Calculated { .. } => "calculated",
            PartitionSource::Entity { .. } => "entity",
            PartitionSource::Query { .. } => "query",
            PartitionSource::Unknown { kind, .. } => kind,
        }
    }

    /// The M, DAX or query text, if the source has one.
    pub fn expression(&self) -> Option<&str> {
        match self {
            PartitionSource::M { expression } | PartitionSource::Calculated { expression } => {
                Some(expression)
            }
            PartitionSource::Query { query, .. } => Some(query),
            PartitionSource::Entity {
                expression_source, ..
            } => expression_source.as_deref(),
            PartitionSource::Unknown { .. } => None,
        }
    }
}

/// A table partition (TOM `Partition`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Partition {
    pub id: ObjectId,
    pub name: String,
    pub mode: PartitionMode,
    pub source: PartitionSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

/// A user hierarchy.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Hierarchy {
    pub id: ObjectId,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lineage_tag: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub is_hidden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_folder: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub levels: Vec<Level>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

impl Hierarchy {
    /// Level by name (case-insensitive).
    pub fn level(&self, name: &str) -> Option<&Level> {
        self.levels.iter().find(|l| names_eq(&l.name, name))
    }
}

/// One level of a [`Hierarchy`]; `column` names a column of the same table.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Level {
    pub id: ObjectId,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ordinal: Option<i64>,
    pub column: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lineage_tag: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub extra: PropertyBag,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn partition(mode: PartitionMode, source: PartitionSource) -> Partition {
        Partition {
            name: "p".into(),
            mode,
            source,
            ..Partition::default()
        }
    }

    #[test]
    fn table_kind_is_derived_from_partitions() {
        let mut t = Table::new("Orders");
        t.partitions.push(partition(
            PartitionMode::Import,
            PartitionSource::M {
                expression: "let Source = 1 in Source".into(),
            },
        ));
        assert_eq!(t.kind(), TableKind::Regular);

        t.partitions[0].mode = PartitionMode::DirectQuery;
        assert_eq!(t.kind(), TableKind::Remote);

        let mut calc = Table::new("Products Never Sold");
        calc.partitions.push(partition(
            PartitionMode::Import,
            PartitionSource::Calculated {
                expression: "SELECTCOLUMNS(...)".into(),
            },
        ));
        assert_eq!(calc.kind(), TableKind::CalculatedTable);
        assert_eq!(calc.calculated_expression(), Some("SELECTCOLUMNS(...)"));

        let mut remote = Table::new("Sales");
        remote.partitions.push(partition(
            PartitionMode::Default,
            PartitionSource::Entity {
                entity_name: "Sales".into(),
                expression_source: Some("DatabaseQuery".into()),
                schema_name: None,
            },
        ));
        assert_eq!(remote.kind(), TableKind::Remote);

        let mut group = Table::new("Time Intelligence");
        group.partitions.push(partition(
            PartitionMode::Import,
            PartitionSource::Unknown {
                kind: "calculationGroup".into(),
                props: PropertyBag::new(),
            },
        ));
        assert_eq!(group.kind(), TableKind::CalculationGroup);
    }

    #[test]
    fn column_and_measure_lookup_is_case_insensitive() {
        let mut t = Table::new("Products");
        t.columns
            .push(Column::data("Product Name", DataType::String));
        t.columns.push(Column::calculated(
            "Sales Status",
            DataType::String,
            "IF(ISEMPTY(RELATEDTABLE('Orders')), \"Never Sold\", \"Sold\")",
        ));
        t.measures
            .push(Measure::new("Product Count", "COUNTROWS(Products)"));

        assert_eq!(
            t.column("product NAME").and_then(Column::source_column),
            Some("Product Name")
        );
        let status = t.column("sales status").unwrap();
        assert!(status.is_calculated());
        assert!(status.expression().unwrap().starts_with("IF("));
        assert!(t.measure("PRODUCT count").is_some());
        assert!(t.measure("Product Name").is_none());
        t.column_mut("PRODUCT NAME").unwrap().is_hidden = true;
        assert!(t.columns[0].is_hidden);
    }

    #[test]
    fn tagged_enums_with_json_numbers_round_trip() {
        // The workspace enables serde_json `arbitrary_precision`; numbers
        // inside internally tagged enums must still deserialize.
        let mut props = PropertyBag::new();
        props.insert(
            "precision".into(),
            serde_json::json!({"n": 1.5, "i": 7}).into(),
        );
        let p = Partition {
            source: PartitionSource::Unknown {
                kind: "policyRange".into(),
                props,
            },
            ..Partition::default()
        };
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Partition>(&json).unwrap(), p);
    }

    #[test]
    fn column_kind_json_shape() {
        let c = Column::calculated("X", DataType::Int64, "1");
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(
            v["kind"],
            serde_json::json!({"type": "calculated", "expression": "1"})
        );
        assert_eq!(v["dataType"], "int64");
        let back: Column = serde_json::from_value(v).unwrap();
        assert_eq!(back, c);
    }
}
