//! Storage engine of DashCraft: Arrow columnar store, relationship indexes,
//! filter-context propagation and storage-engine primitives, plus DataFusion
//! for ad-hoc SQL and the explorer grid.
//!
//! # Storage engine vs formula engine
//!
//! Like Analysis Services, DAX evaluation is split in two:
//!
//! - **Formula engine (FE)** — `pbi-dax`: parses DAX, owns row context,
//!   context transition, `CALCULATE` argument semantics, measure evaluation,
//!   type coercion and arithmetic with BLANK.
//! - **Storage engine (SE)** — this crate: stores tables ([`Storage`]),
//!   indexes relationships ([`RelationshipGraph`]), computes which rows a
//!   [`FilterContext`] leaves visible in any table ([`EngineSnapshot::visible`])
//!   and runs scans, distinct, group-by, aggregates and relationship
//!   navigation ([`ops`]). It knows nothing about DAX syntax.
//!
//! Grouped work stays proportional to group sizes: [`GroupIndex`] maps rows
//! to groups and [`EngineSnapshot::aggregate_groups`] aggregates every group
//! in one pass; selective `IN` filters read a per-column inverted index
//! ([`EncodedColumn::rows_of`]); filters on a few one-side rows propagate
//! through the relationship index instead of scanning the many side.
//!
//! The FE builds a [`FilterContext`] (`replace_column` for `CALCULATE`
//! filters, `add` for `KEEPFILTERS`, `remove_*`/`clear` for `ALL` and
//! `REMOVEFILTERS`), asks for visible [`RowSet`]s and aggregates over them.
//!
//! # Values and BLANK
//!
//! [`Value`] is the scalar type shared with `pbi-dax` (see [`value`] for the
//! DAX comparison rules: `BLANK = 0` is true, `BLANK == 0` is false, text
//! comparisons are case-insensitive). Fixed decimals are `i128` scaled by
//! 10⁴, stored as Arrow `Decimal128(19, 4)`. Column dictionaries
//! ([`EncodedColumn`]) are case-insensitive like VertiPaq's: `"abc"` and
//! `"ABC"` in one column are one value (first spelling wins). Aggregates
//! follow DAX: `SUM` over no values and `COUNTROWS` of nothing are BLANK.
//!
//! # Filter propagation
//!
//! - A filter on a table restricts that table, and **flows one → many**
//!   over every active relationship, transitively (`A → B → C` chains).
//! - With `bothDirections` (and for one-to-one relationships) it also
//!   **flows many → one**: the one side keeps rows with at least one
//!   visible related row.
//! - Each table reaches another along one path, chosen like Power BI
//!   (fewest reverse hops, then fewest hops); ties are reported as an
//!   ambiguity diagnostic. Cycles cannot loop: a relationship is never
//!   walked back.
//! - **No path, no effect**: a filter on `Orders[Status]` does not change
//!   an unrelated table such as `'Web Sessions'`.
//! - Table filters ([`TableFilter`]) filter the *expanded* table, as in DAX.
//!
//! # Blank row
//!
//! When many-side keys have no match, the one side gets a virtual **blank
//! row** (all columns BLANK) that the unmatched rows relate to
//! ([`RelationshipIndex::target`] returns `None`). It is visible when the
//! one side is unfiltered or its filters accept BLANK, so orders with an
//! unknown customer count under no filter and disappear under
//! `Customers[Segment] = "Corporate"`. [`VisibleRows::blank_row`] reports it, and
//! [`EngineSnapshot::values`] / [`EngineSnapshot::aggregate_with_blank_row`]
//! include it (`VALUES`, `DISTINCTCOUNT`, `COUNTROWS(table)`).
//!
//! # Concurrency and async
//!
//! [`Engine`] owns the data; every mutation publishes a new immutable
//! [`EngineSnapshot`] (`Arc`, `Send + Sync`) that readers can keep using.
//! SQL entry points ([`EngineSnapshot::sql`], [`EngineSnapshot::page`],
//! [`EngineSnapshot::page_row_count`]) are `async` and take a
//! `tokio_util::sync::CancellationToken`.
//!
//! # Relationship overrides
//!
//! `USERELATIONSHIP` and `CROSSFILTER` are [`RelationshipOverride`]s set on
//! a [`FilterContext`] ([`FilterContext::set_relationship_override`]);
//! every visibility computation in that context runs on the graph they
//! describe ([`EngineSnapshot::graph_for`]). Relationship navigation
//! (`related_row`, `group_by` through relationships) always uses the
//! model's active relationships.
//!
//! # Many-to-many relationships
//!
//! Limited relationships: filters cross them as the distinct key values of
//! the visible rows, in the cross-filter direction (`to` → `from` when
//! single), without blank rows or table expansion (see
//! [`relationships`]; diagnostic `ENG003` is informational).
//!
//! # Row-level security
//!
//! [`Engine::with_row_security`] restricts every table to the rows a set of
//! roles may see (role filters propagate in their security direction,
//! union over roles); snapshots carry the [`SecurityContext`] (roles and
//! effective user for `USERPRINCIPALNAME()` & co.). See [`security`].
//!
//! # Not implemented yet
//!
//! Self-referencing relationships, object-level security,
//! multi-column (tuple) filters other than whole-table filters, projecting
//! a table filter when one of its columns is overridden (`remove_column`
//! leaves table filters intact), and VertiPaq-style compressed storage
//! (tables are plain Arrow batches plus lazily built dictionaries).

pub mod engine;
pub mod error;
pub mod filter;
pub mod ops;
pub mod relationships;
pub mod rowset;
pub mod security;
pub mod sql;
pub mod storage;
pub mod value;

pub use engine::{Engine, EngineSnapshot};
pub use error::{EngineError, Result};
pub use filter::{
    ColumnFilter, Filter, FilterContext, FilterKind, Predicate, TableFilter, VisibleRows,
};
pub use ops::{Agg, ColumnStats, Group, GroupIndex};
pub use relationships::{
    CrossFilterDirection, Flow, Hop, ManyToManyIndex, PropagationTree, RelationshipDef,
    RelationshipGraph, RelationshipIndex, RelationshipKind, RelationshipOverride,
};
pub use rowset::RowSet;
pub use security::{RoleFilter, SecurityContext, SecurityRole, UserIdentity};
pub use sql::{PageRequest, ROW_ID_COLUMN, SortSpec};
pub use storage::{EncodedColumn, Storage, TableData};
pub use tokio_util::sync::CancellationToken;
pub use value::{Value, ValueKey, ValueType, arrow_type};
