//! Numeric enumeration codes stored in `metadata.sqlitedb`.
//!
//! The values are those of the Tabular Object Model (TOM) enumerations
//! (`Microsoft.AnalysisServices.Tabular.*`, Microsoft Learn API reference; facts only),
//! cross-checked against pbixray's queries (MIT, REFERENCES.md #3) and the fixture.

use pbi_model::{
    Cardinality, CrossFilter, DataType, PartitionMode, SecurityFilteringBehavior, SummarizeBy,
};

/// TOM `ObjectType` values used by `Annotation.ObjectType` and friends.
pub(crate) mod object_type {
    pub const MODEL: i64 = 1;
    pub const TABLE: i64 = 3;
    pub const COLUMN: i64 = 4;
    pub const PARTITION: i64 = 6;
    pub const RELATIONSHIP: i64 = 7;
    pub const MEASURE: i64 = 8;
    pub const HIERARCHY: i64 = 9;
    pub const LEVEL: i64 = 10;
    pub const CULTURE: i64 = 13;
    pub const PERSPECTIVE: i64 = 29;
    pub const ROLE: i64 = 34;
    pub const TABLE_PERMISSION: i64 = 36;
    pub const EXPRESSION: i64 = 41;
    pub const QUERY_GROUP: i64 = 51;
}

/// TOM `DataType`: 1 Automatic, 2 String, 6 Int64, 8 Double, 9 DateTime, 10 Decimal,
/// 11 Boolean, 17 Binary, 19 Unknown, 20 Variant.
pub fn data_type(code: i64) -> DataType {
    match code {
        1 => DataType::Automatic,
        2 => DataType::String,
        6 => DataType::Int64,
        8 => DataType::Double,
        9 => DataType::DateTime,
        10 => DataType::Decimal,
        11 => DataType::Boolean,
        17 => DataType::Binary,
        20 => DataType::Variant,
        19 => DataType::Unknown("unknown".into()),
        other => DataType::Unknown(format!("dataType{other}")),
    }
}

/// `true` for codes that name a concrete type (not Automatic/Unknown).
pub(crate) fn is_concrete_data_type(code: i64) -> bool {
    matches!(code, 2 | 6 | 8 | 9 | 10 | 11 | 17 | 20)
}

/// Effective data type code: `ExplicitDataType` unless it is Automatic (1), in which
/// case a concrete `InferredDataType` wins.
pub(crate) fn effective_data_type(explicit: Option<i64>, inferred: Option<i64>) -> (i64, bool) {
    match (explicit, inferred) {
        (Some(1) | None, Some(inferred)) if is_concrete_data_type(inferred) => (inferred, true),
        (Some(explicit), _) => (explicit, false),
        (None, Some(inferred)) => (inferred, false),
        (None, None) => (1, false),
    }
}

/// TOM `ColumnType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ColumnKindCode {
    /// 1: loaded from the partition source.
    Data,
    /// 2: DAX calculated column.
    Calculated,
    /// 3: internal row-number column.
    RowNumber,
    /// 4: column of a calculated table.
    CalculatedTableColumn,
    /// Any other code.
    Other(i64),
}

impl ColumnKindCode {
    pub fn from_code(code: i64) -> Self {
        match code {
            1 => Self::Data,
            2 => Self::Calculated,
            3 => Self::RowNumber,
            4 => Self::CalculatedTableColumn,
            other => Self::Other(other),
        }
    }
}

/// TOM `ModeType`: 0 Import, 1 DirectQuery, 2 Default, 3 Push, 4 Dual, 5 DirectLake.
pub(crate) fn partition_mode(code: i64) -> PartitionMode {
    match code {
        0 => PartitionMode::Import,
        1 => PartitionMode::DirectQuery,
        2 => PartitionMode::Default,
        4 => PartitionMode::Dual,
        5 => PartitionMode::DirectLake,
        3 => PartitionMode::Unknown("push".into()),
        other => PartitionMode::Unknown(format!("mode{other}")),
    }
}

/// TOM `PartitionSourceType`: 1 Query, 2 Calculated, 3 None, 4 M, 5 Entity,
/// 6 PolicyRange, 7 CalculationGroup, 8 Inferred.
pub(crate) fn partition_source_kind(code: i64) -> &'static str {
    match code {
        1 => "query",
        2 => "calculated",
        3 => "none",
        4 => "m",
        5 => "entity",
        6 => "policyRange",
        7 => "calculationGroup",
        8 => "inferred",
        9 => "parquet",
        _ => "unknown",
    }
}

/// TOM `RelationshipEndCardinality`: 1 One, 2 Many.
pub(crate) fn cardinality(code: Option<i64>, default: Cardinality) -> Cardinality {
    match code {
        Some(1) => Cardinality::One,
        Some(2) => Cardinality::Many,
        Some(0) | None => default,
        Some(other) => Cardinality::Unknown(format!("cardinality{other}")),
    }
}

/// TOM `CrossFilteringBehavior`: 1 OneDirection, 2 BothDirections, 3 Automatic.
pub(crate) fn cross_filter(code: Option<i64>) -> CrossFilter {
    match code {
        Some(2) => CrossFilter::BothDirections,
        Some(3) => CrossFilter::Automatic,
        Some(1) | Some(0) | None => CrossFilter::OneDirection,
        Some(other) => CrossFilter::Unknown(format!("crossFilter{other}")),
    }
}

/// TOM `SecurityFilteringBehavior`: 1 OneDirection, 2 BothDirections, 3 None.
pub(crate) fn security_filter(code: Option<i64>) -> SecurityFilteringBehavior {
    match code {
        Some(2) => SecurityFilteringBehavior::BothDirections,
        Some(3) => SecurityFilteringBehavior::None,
        Some(1) | Some(0) | None => SecurityFilteringBehavior::OneDirection,
        Some(other) => SecurityFilteringBehavior::Unknown(format!("securityFilter{other}")),
    }
}

/// TOM `AggregateFunction`: 1 Default, 2 None, 3 Sum, 4 Min, 5 Max, 6 Count, 7 Average,
/// 8 DistinctCount.
pub(crate) fn summarize_by(code: Option<i64>) -> Option<SummarizeBy> {
    Some(match code? {
        1 => SummarizeBy::Default,
        2 => SummarizeBy::None,
        3 => SummarizeBy::Sum,
        4 => SummarizeBy::Min,
        5 => SummarizeBy::Max,
        6 => SummarizeBy::Count,
        7 => SummarizeBy::Average,
        8 => SummarizeBy::DistinctCount,
        _ => return None,
    })
}

/// TOM `DateTimeRelationshipBehavior`: 1 DateAndTime, 2 DatePartOnly (TMDL spelling).
pub(crate) fn join_on_date_behavior(code: i64) -> Option<&'static str> {
    match code {
        1 => Some("dateAndTime"),
        2 => Some("datePartOnly"),
        _ => None,
    }
}

/// TOM `PowerBIDataSourceVersion`: 0 PowerBI_V1, 1 PowerBI_V2, 2 PowerBI_V3 (TMDL spelling).
pub(crate) fn power_bi_data_source_version(code: i64) -> Option<String> {
    match code {
        0 => Some("powerBI_V1".into()),
        1 => Some("powerBI_V2".into()),
        2 => Some("powerBI_V3".into()),
        _ => None,
    }
}

/// TOM `ModelPermission`: 1 None, 2 Read, 3 ReadRefresh, 4 Refresh, 5 Administrator.
pub(crate) fn model_permission(code: i64) -> Option<String> {
    Some(
        match code {
            1 => "none",
            2 => "read",
            3 => "readRefresh",
            4 => "refresh",
            5 => "administrator",
            _ => return None,
        }
        .into(),
    )
}

/// TOM `DataViewType`: 0 Full, 1 Sample, 3 Default (TMDL spelling).
pub(crate) fn data_view(code: i64) -> Option<&'static str> {
    match code {
        0 => Some("full"),
        1 => Some("sample"),
        3 => Some("default"),
        _ => None,
    }
}

/// TOM `EncodingHintType`: 0 Default, 1 Hash, 2 Value (TMDL spelling).
pub(crate) fn encoding_hint(code: i64) -> Option<&'static str> {
    match code {
        1 => Some("hash"),
        2 => Some("value"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_types_follow_tom() {
        assert_eq!(data_type(2), DataType::String);
        assert_eq!(data_type(6), DataType::Int64);
        assert_eq!(data_type(8), DataType::Double);
        assert_eq!(data_type(9), DataType::DateTime);
        assert_eq!(data_type(10), DataType::Decimal);
        assert_eq!(data_type(11), DataType::Boolean);
        assert_eq!(data_type(17), DataType::Binary);
        assert_eq!(data_type(20), DataType::Variant);
        assert_eq!(data_type(1), DataType::Automatic);
        assert!(matches!(data_type(19), DataType::Unknown(_)));
        assert!(matches!(data_type(99), DataType::Unknown(_)));
        assert_eq!(effective_data_type(Some(1), Some(6)), (6, true));
        assert_eq!(effective_data_type(Some(2), Some(19)), (2, false));
        assert_eq!(effective_data_type(Some(1), Some(19)), (1, false));
        assert_eq!(effective_data_type(None, None), (1, false));
    }

    #[test]
    fn relationship_codes() {
        assert_eq!(cardinality(Some(2), Cardinality::One), Cardinality::Many);
        assert_eq!(cardinality(None, Cardinality::One), Cardinality::One);
        assert_eq!(cross_filter(Some(2)), CrossFilter::BothDirections);
        assert_eq!(cross_filter(Some(1)), CrossFilter::OneDirection);
        assert_eq!(security_filter(Some(3)), SecurityFilteringBehavior::None);
        assert_eq!(summarize_by(Some(3)), Some(SummarizeBy::Sum));
        assert_eq!(summarize_by(Some(42)), None);
        assert_eq!(partition_mode(0), PartitionMode::Import);
        assert_eq!(partition_mode(2), PartitionMode::Default);
        assert_eq!(partition_source_kind(4), "m");
    }
}
