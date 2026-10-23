//! TOM-like (Tabular Object Model) semantic model.
//!
//! Hierarchy: [`Model`] → [`Table`] → [`Column`] / [`Measure`] /
//! [`Partition`] / [`Hierarchy`] / [`CalculationGroup`] (of a
//! calculation-group table, with its [`CalculationItem`]s), plus model-level [`Relationship`]s,
//! [`NamedExpression`]s (M queries and parameters), [`QueryGroup`]s,
//! [`Role`]s, [`Culture`]s and [`Perspective`]s. The optional [`Database`]
//! carries `database.tmdl` / `model.bim` root properties.
//!
//! Names are case-insensitive, as in Power BI; every lookup helper compares
//! with [`crate::names::names_eq`].

mod calc_group;
mod expression;
mod model;
mod relationship;
mod table;
mod types;

pub use calc_group::{CalculationGroup, CalculationGroupExpression, CalculationItem};
pub use expression::{NamedExpression, parse_m_record, split_m_meta};
pub use model::{
    Culture, Database, FieldKind, Model, Perspective, QueryGroup, Role, TablePermission,
};
pub use relationship::{ColumnRef, Relationship};
pub use table::{
    Column, ColumnKind, Hierarchy, Level, Measure, Partition, PartitionSource, Table, TableKind,
};
pub use types::{
    Cardinality, CrossFilter, DataType, ExpressionKind, PartitionMode, SecurityFilteringBehavior,
    SummarizeBy,
};
