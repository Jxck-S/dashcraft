//! Semantic-query JSON (`QueryExpressionContainer` and friends) as used by
//! PBIR projections, sorts and filters, the legacy `prototypeQuery`, and
//! formatting `objects` expressions.
//!
//! Like the schema, an expression is a container object with one populated
//! member (`Column`, `Measure`, `In`, …) plus optional `Name` /
//! `NativeReferenceName`. [`QueryExpr::kind`] gives an enum view. Members
//! not modelled here (`DateSpan`, `Arithmetic`, …) stay in `extra` and show
//! up as [`ExprKind::Other`].
//!
//! [`FieldExprJson`] and [`ConditionJson`] are aliases of [`QueryExpr`]: the
//! schema uses the same container for fields and conditions.

use indexmap::IndexMap;
use serde_json::{Number, Value};

use crate::json::{json_object, num_i64};

json_object! {
    /// A semantic-query expression container.
    pub struct QueryExpr {
        /// Select-item name (legacy `prototypeQuery.Select[].Name`).
        "Name" => name: String,
        "NativeReferenceName" => native_reference_name: String,
        "Annotations" => annotations: Value,
        "SourceRef" => source_ref: SourceRefExpr,
        "Column" => column: PropertyExpr,
        "Measure" => measure: PropertyExpr,
        "Aggregation" => aggregation: AggregationExpr,
        "Hierarchy" => hierarchy: HierarchyExpr,
        "HierarchyLevel" => hierarchy_level: HierarchyLevelExpr,
        "PropertyVariationSource" => property_variation_source: PropertyVariationSourceExpr,
        "Literal" => literal: LiteralExpr,
        "In" => in_: InExpr,
        "Comparison" => comparison: ComparisonExpr,
        "And" => and: BinaryExpr,
        "Or" => or: BinaryExpr,
        "Not" => not: UnaryExpr,
        "Contains" => contains: BinaryExpr,
        "StartsWith" => starts_with: BinaryExpr,
        "Between" => between: BetweenExpr,
        "Exists" => exists: UnaryExpr,
        "ThemeDataColor" => theme_data_color: ThemeDataColorExpr,
    }
}

/// A semantic-query expression used as a field (projection, sort, filter
/// target).
pub type FieldExprJson = QueryExpr;
/// A semantic-query expression used as a filter condition.
pub type ConditionJson = QueryExpr;

json_object! {
    /// `SourceRef`: either a query alias (`Source`) or a table (`Entity`).
    pub struct SourceRefExpr {
        "Source" => source: String,
        "Entity" => entity: String,
        "Schema" => schema: String,
    }
}

json_object! {
    /// `Column` / `Measure`: `{Expression, Property}`.
    pub struct PropertyExpr {
        "Expression" => expression: Box<QueryExpr>,
        "Property" => property: String,
    }
}

json_object! {
    /// `Aggregation`: `{Expression, Function}` (function code, see
    /// [`pbi_model::AggregationFunction`]).
    pub struct AggregationExpr {
        "Expression" => expression: Box<QueryExpr>,
        "Function" => function: Number,
    }
}

json_object! {
    /// `Hierarchy`: `{Expression, Hierarchy}`.
    pub struct HierarchyExpr {
        "Expression" => expression: Box<QueryExpr>,
        "Hierarchy" => hierarchy: String,
    }
}

json_object! {
    /// `HierarchyLevel`: `{Expression (a Hierarchy), Level}`.
    pub struct HierarchyLevelExpr {
        "Expression" => expression: Box<QueryExpr>,
        "Level" => level: String,
    }
}

json_object! {
    /// `PropertyVariationSource` (auto date/time): `{Expression, Name, Property}`.
    pub struct PropertyVariationSourceExpr {
        "Expression" => expression: Box<QueryExpr>,
        "Name" => name: String,
        "Property" => property: String,
    }
}

json_object! {
    /// `Literal`: `{Value}` with PBIR literal text (`'West'`, `240L`, …).
    pub struct LiteralExpr {
        "Value" => value: String,
    }
}

json_object! {
    /// `In`: tuple membership against literal rows or a table.
    pub struct InExpr {
        "Expressions" => expressions: Vec<QueryExpr>,
        "Values" => values: Vec<Vec<QueryExpr>>,
        "Table" => table: Box<QueryExpr>,
    }
}

json_object! {
    /// `Comparison`: `{ComparisonKind, Left, Right}`.
    pub struct ComparisonExpr {
        "ComparisonKind" => comparison_kind: Number,
        "Left" => left: Box<QueryExpr>,
        "Right" => right: Box<QueryExpr>,
    }
}

json_object! {
    /// `And` / `Or` / `Contains` / `StartsWith`: `{Left, Right}`.
    pub struct BinaryExpr {
        "Left" => left: Box<QueryExpr>,
        "Right" => right: Box<QueryExpr>,
    }
}

json_object! {
    /// `Not` / `Exists`: `{Expression}`.
    pub struct UnaryExpr {
        "Expression" => expression: Box<QueryExpr>,
    }
}

json_object! {
    /// `Between`: `{Expression, LowerBound, UpperBound}`.
    pub struct BetweenExpr {
        "Expression" => expression: Box<QueryExpr>,
        "LowerBound" => lower_bound: Box<QueryExpr>,
        "UpperBound" => upper_bound: Box<QueryExpr>,
    }
}

json_object! {
    /// `ThemeDataColor`: `{ColorId, Percent}`; see
    /// [`crate::theme::resolve_theme_data_color`].
    pub struct ThemeDataColorExpr {
        "ColorId" => color_id: Number,
        "Percent" => percent: Number,
    }
}

/// Enum view of a [`QueryExpr`] (the first populated member).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExprKind<'a> {
    SourceRef(&'a SourceRefExpr),
    Column(&'a PropertyExpr),
    Measure(&'a PropertyExpr),
    Aggregation(&'a AggregationExpr),
    Hierarchy(&'a HierarchyExpr),
    HierarchyLevel(&'a HierarchyLevelExpr),
    PropertyVariationSource(&'a PropertyVariationSourceExpr),
    Literal(&'a LiteralExpr),
    In(&'a InExpr),
    Comparison(&'a ComparisonExpr),
    And(&'a BinaryExpr),
    Or(&'a BinaryExpr),
    Not(&'a UnaryExpr),
    Contains(&'a BinaryExpr),
    StartsWith(&'a BinaryExpr),
    Between(&'a BetweenExpr),
    Exists(&'a UnaryExpr),
    ThemeDataColor(&'a ThemeDataColorExpr),
    /// A member this crate does not model (`DateSpan`, `Now`, …), verbatim.
    Other(&'a str, &'a Value),
    /// No expression member.
    Empty,
}

impl QueryExpr {
    /// The populated expression member.
    pub fn kind(&self) -> ExprKind<'_> {
        macro_rules! first {
            ($($field:ident => $variant:ident),* $(,)?) => {
                $( if let Some(x) = &self.$field { return ExprKind::$variant(x); } )*
            };
        }
        first!(
            source_ref => SourceRef,
            column => Column,
            measure => Measure,
            aggregation => Aggregation,
            hierarchy => Hierarchy,
            hierarchy_level => HierarchyLevel,
            property_variation_source => PropertyVariationSource,
            literal => Literal,
            in_ => In,
            comparison => Comparison,
            and => And,
            or => Or,
            not => Not,
            contains => Contains,
            starts_with => StartsWith,
            between => Between,
            exists => Exists,
            theme_data_color => ThemeDataColor,
        );
        self.extra
            .iter()
            .find(|(_, v)| !v.is_null())
            .map_or(ExprKind::Empty, |(k, v)| ExprKind::Other(k, v))
    }

    /// `{"SourceRef": {"Entity": entity}}`.
    pub fn entity_ref(entity: impl Into<String>) -> Self {
        Self {
            source_ref: Some(SourceRefExpr {
                entity: Some(entity.into()),
                ..SourceRefExpr::default()
            }),
            ..Self::default()
        }
    }

    /// `{"Column": {"Expression": {"SourceRef": {"Entity": …}}, "Property": …}}`.
    pub fn column(entity: impl Into<String>, property: impl Into<String>) -> Self {
        Self {
            column: Some(PropertyExpr::new(Self::entity_ref(entity), property)),
            ..Self::default()
        }
    }

    /// `{"Measure": {"Expression": {"SourceRef": {"Entity": …}}, "Property": …}}`.
    pub fn measure(entity: impl Into<String>, property: impl Into<String>) -> Self {
        Self {
            measure: Some(PropertyExpr::new(Self::entity_ref(entity), property)),
            ..Self::default()
        }
    }

    /// `{"Literal": {"Value": raw}}`.
    pub fn literal(raw: impl Into<String>) -> Self {
        Self {
            literal: Some(LiteralExpr {
                value: Some(raw.into()),
                ..LiteralExpr::default()
            }),
            ..Self::default()
        }
    }
}

impl PropertyExpr {
    pub fn new(expression: QueryExpr, property: impl Into<String>) -> Self {
        Self {
            expression: Some(Box::new(expression)),
            property: Some(property.into()),
            ..Self::default()
        }
    }
}

impl ThemeDataColorExpr {
    /// `(ColorId, Percent)` if both are valid numbers.
    pub fn values(&self) -> Option<(i64, f64)> {
        let id = num_i64(self.color_id.as_ref()?)?;
        let percent = crate::json::num_f64(self.percent.as_ref()?)?;
        Some((id, percent))
    }
}

json_object! {
    /// An entity source of a query or filter (`From[]`).
    pub struct EntitySourceJson {
        /// Alias used by `SourceRef.Source`.
        "Name" => name: String,
        "Entity" => entity: String,
        "Schema" => schema: String,
        "Expression" => expression: QueryExpr,
        /// 0 table, 1 presentation object, 2 expression.
        "Type" => source_type: Number,
    }
}

json_object! {
    /// A `Where[]` item.
    pub struct QueryFilterJson {
        "Target" => target: Vec<QueryExpr>,
        "Condition" => condition: ConditionJson,
        "Annotations" => annotations: Value,
    }
}

json_object! {
    /// An `OrderBy[]` item (`Direction` 1 ascending, 2 descending).
    pub struct QuerySortClauseJson {
        "Direction" => direction: Number,
        "Expression" => expression: QueryExpr,
    }
}

json_object! {
    /// A semantic query or filter definition (`{Version, From, Where, …}`).
    ///
    /// Used for filter `filter` objects and the legacy `prototypeQuery`.
    pub struct QueryDefinitionJson {
        "Version" => version: Number,
        "From" => from: Vec<EntitySourceJson>,
        "Where" => where_: Vec<QueryFilterJson>,
        "OrderBy" => order_by: Vec<QuerySortClauseJson>,
        "Select" => select: Vec<QueryExpr>,
        "GroupBy" => group_by: Vec<QueryExpr>,
        "Top" => top: Number,
    }
}

/// Filter definitions use the same shape as queries.
pub type FilterDefinitionJson = QueryDefinitionJson;

impl QueryDefinitionJson {
    /// Alias → entity map from `From` (only table sources with an entity).
    pub fn aliases(&self) -> SourceAliases {
        let mut map = IndexMap::new();
        for source in self.from.iter().flatten() {
            if let (Some(name), Some(entity)) = (&source.name, &source.entity) {
                map.insert(name.clone(), entity.clone());
            }
        }
        SourceAliases(map)
    }
}

/// Query alias → table name (`From: [{Name: "o", Entity: "Orders"}]`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceAliases(pub IndexMap<String, String>);

impl SourceAliases {
    pub fn get(&self, alias: &str) -> Option<&str> {
        self.0.get(alias).map(String::as_str)
    }
}
