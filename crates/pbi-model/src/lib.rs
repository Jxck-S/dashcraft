//! Canonical TOM-like semantic model, report AST, document and command
//! (undo) layer for DashCraft.
//!
//! # Design
//!
//! - **TOM-like model** ([`semantic`]): [`Model`] → [`Table`] →
//!   [`Column`]/[`Measure`]/[`Partition`]/[`Hierarchy`], plus relationships,
//!   named expressions (M queries and parameters), query groups, roles,
//!   cultures and perspectives. Readers for TMDL (`pbi-tmdl`), TMSL
//!   (`pbi-tmsl`) and VertiPaq metadata all produce these types.
//! - **Format-agnostic report AST** ([`report`], [`query`]): PBIR,
//!   PBIR-Legacy and the PBIX `Layout` map to [`Report`] → [`Page`] →
//!   [`Visual`], with resolved [`FieldExpr`] references (table names, not
//!   query aliases) and typed [`Filter`]s/[`Condition`]s.
//! - **Lossless-friendly**: every object has an `extra` [`PropertyBag`]
//!   (insertion-ordered) for properties the typed model does not cover, plus
//!   `annotations`, `description` and `lineage_tag` where the source format
//!   has them. Report objects also keep verbatim JSON (`objects`,
//!   `Filter::raw`). Byte-exact round-tripping is the writers' job (e.g. a
//!   TMDL CST); the model only guarantees nothing is dropped.
//! - **Stable identity** ([`ids`]): every object has an [`ObjectId`] assigned
//!   on load ([`Document::assign_ids`]) that survives renames; UI selection,
//!   diagnostics and commands use it.
//! - **Command layer** ([`command`]): edits are [`Command`]s whose `apply`
//!   returns the exact inverse; [`History`] provides undo/redo and dirty
//!   tracking, and commands emit [`ChangeEvent`]s for the UI. Renames
//!   rewrite structured references ([`rewrite`]).
//! - **Names are case-insensitive** everywhere ([`names::names_eq`]);
//!   [`names`] also has DAX/TMDL quoting helpers.
//!
//! All data types derive `Debug, Clone, PartialEq, Serialize, Deserialize`
//! with camelCase JSON (used by `inspect --json` and the Tauri IPC). Default
//! values are omitted when serializing and restored when deserializing.

mod macros;
mod serde_util;

pub mod command;
pub mod document;
pub mod ids;
pub mod names;
pub mod props;
pub mod query;
pub mod report;
pub mod rewrite;
pub mod semantic;

#[cfg(test)]
pub(crate) mod test_support;

pub use command::{
    Applied, ChangeEvent, Command, CommandError, CommandGroup, History, RenameColumn,
    RenameMeasure, RenameTable, ReplaceObjects, SetDescription, SetMeasureExpression,
    SetVisualPosition, Snapshot,
};
pub use document::{
    Diagnostic, Document, ModelStatus, ObjectKind, ObjectPath, ObjectRef, PartInfo, Severity,
    SourceFormat, SourceInfo,
};
pub use ids::{IdGen, ObjectId};
pub use names::{dax_column_ref, dax_measure_ref, dax_table_ref, names_eq, tmdl_name};
pub use props::{Annotation, Origin, PropValue, PropertyBag, RawObject};
pub use query::{
    AggregationFunction, ComparisonKind, Condition, FieldExpr, Filter, FilterKind, Literal,
    LiteralError, LiteralValue, PropertyVariation, SortDirection, SortItem,
};
pub use report::{
    DatasetReference, DisplayOption, InteractionKind, Page, Position, Projection, Report,
    ResourceItem, ResourcePackage, ResourceRef, ThemeRef, Visual, VisualInteraction, VisualKind,
};
pub use rewrite::Rename;
pub use semantic::{
    CalculationGroup, CalculationGroupExpression, CalculationItem, Cardinality, Column, ColumnKind,
    ColumnRef, CrossFilter, Culture, DataType, Database, ExpressionKind, FieldKind, Hierarchy,
    Level, Measure, Model, NamedExpression, Partition, PartitionMode, PartitionSource, Perspective,
    QueryGroup, Relationship, Role, SecurityFilteringBehavior, SummarizeBy, Table, TableKind,
    TablePermission,
};
