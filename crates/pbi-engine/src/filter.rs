//! Filter context and filter propagation.
//!
//! A [`FilterContext`] is a list of filters that are all intersected (DAX
//! semantics: every filter in the context applies). Filters are either
//! - [`ColumnFilter`]: a set of allowed values of one column (`IN` list,
//!   predicate, or the values found at some rows), or
//! - [`TableFilter`]: a set of rows of one table. By default it filters the
//!   table's **expanded table** like DAX does (`CALCULATE(…, FILTER(Orders,
//!   …))` also restricts `Stores` to the stores of the remaining orders,
//!   whatever the cross-filter direction).
//!
//! The formula engine builds `CALCULATE` semantics from the operations here:
//! [`FilterContext::replace_column`] (a new filter overrides filters on the
//! same column), [`FilterContext::add`] (`KEEPFILTERS`: intersect),
//! [`FilterContext::remove_column`] / [`FilterContext::remove_table`] /
//! [`FilterContext::remove_expanded_table`] / [`FilterContext::clear`]
//! (`ALL`/`REMOVEFILTERS`).
//!
//! # Propagation ([`RelationshipGraph::visible`])
//!
//! The visible rows of table `T` are the rows that pass
//! 1. every filter on `T` itself, and
//! 2. for each table `X` whose filters reach `T` (its propagation tree, see
//!    [`crate::relationships`]): a semi-join with the rows of `X` that pass
//!    `X`'s own filters and, recursively, the filters reaching `X`:
//!    - one → many (every active relationship): a many-side row is visible
//!      if its one-side row is visible; rows with unmatched keys follow the
//!      one side's **blank row**, which is visible only if the one side's
//!      filters accept BLANK in every filtered column (so a filter like
//!      `Region = "West"` hides unmatched orders, while an
//!      unfiltered one side keeps them);
//!    - many → one (bidirectional only): a one-side row is visible if at
//!      least one related many-side row is visible. An unfiltered many side
//!      does not restrict the one side;
//!    - many-to-many (limited relationships): a row is visible if its key
//!      is among the keys of the visible rows on the other side (no blank
//!      row).
//!
//! Filters on tables without a path to `T` have no effect on `T`.
//!
//! # Relationship overrides
//!
//! A context may also carry [`RelationshipOverride`]s (DAX
//! `USERELATIONSHIP` and `CROSSFILTER`,
//! [`FilterContext::set_relationship_override`]). They are not filters:
//! [`FilterContext::clear`] (`ALL()`) keeps them and
//! [`FilterContext::is_empty`] ignores them. Propagation then runs on
//! [`RelationshipGraph::with_overrides`].

use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use pbi_model::names::names_eq;

use crate::error::{EngineError, Result};
use crate::relationships::{RelationshipGraph, RelationshipOverride};
use crate::rowset::RowSet;
use crate::storage::{Storage, TableData};
use crate::value::{Value, ValueKey};

/// Predicate over a column value.
pub type PredicateFn = dyn Fn(&Value) -> bool + Send + Sync;

static NEXT_PREDICATE_ID: AtomicU64 = AtomicU64::new(1);

/// A shareable value predicate with a process-unique identity (used for
/// equality and caching: two predicates are equal only if they are clones
/// of the same `Predicate`).
#[derive(Clone)]
pub struct Predicate {
    id: u64,
    label: Option<Arc<str>>,
    f: Arc<PredicateFn>,
}

impl Predicate {
    pub fn new(f: impl Fn(&Value) -> bool + Send + Sync + 'static) -> Self {
        Self {
            id: NEXT_PREDICATE_ID.fetch_add(1, Ordering::Relaxed),
            label: None,
            f: Arc::new(f),
        }
    }

    /// Adds a label shown by `Debug` (e.g. the DAX source text).
    pub fn with_label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn eval(&self, value: &Value) -> bool {
        (self.f)(value)
    }

    pub fn id(&self) -> u64 {
        self.id
    }
}

impl fmt::Debug for Predicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.label {
            Some(label) => write!(f, "Predicate#{}({label})", self.id),
            None => write!(f, "Predicate#{}", self.id),
        }
    }
}

impl PartialEq for Predicate {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

/// What a [`ColumnFilter`] allows.
#[derive(Debug, Clone, PartialEq)]
pub enum FilterKind {
    /// Values whose [`ValueKey`] is in the set (DAX `IN` / `==` semantics:
    /// text case-insensitive, BLANK only matches BLANK).
    In(Arc<HashSet<ValueKey>>),
    /// Values accepted by the predicate (evaluated once per distinct value).
    Predicate(Predicate),
    /// The values found in the column at these rows (value-closed: every row
    /// with one of those values is kept, as a DAX column filter would).
    Rows(Arc<RowSet>),
}

/// A filter on the values of one column.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnFilter {
    pub table: String,
    pub column: String,
    pub kind: FilterKind,
}

impl ColumnFilter {
    pub fn new(table: impl Into<String>, column: impl Into<String>, kind: FilterKind) -> Self {
        Self {
            table: table.into(),
            column: column.into(),
            kind,
        }
    }

    /// `table[column] IN {values}` (strict: BLANK ≠ 0).
    pub fn in_values(
        table: impl Into<String>,
        column: impl Into<String>,
        values: impl IntoIterator<Item = Value>,
    ) -> Self {
        Self::in_keys(table, column, values.into_iter().map(ValueKey::from))
    }

    /// `table[column] IN {keys}`.
    pub fn in_keys(
        table: impl Into<String>,
        column: impl Into<String>,
        keys: impl IntoIterator<Item = ValueKey>,
    ) -> Self {
        Self::new(
            table,
            column,
            FilterKind::In(Arc::new(keys.into_iter().collect())),
        )
    }

    /// `table[column] == value` (strict, case-insensitive for text).
    pub fn equals(
        table: impl Into<String>,
        column: impl Into<String>,
        value: impl Into<Value>,
    ) -> Self {
        Self::in_values(table, column, [value.into()])
    }

    /// `table[column] = value` with DAX `=` semantics (BLANK equals 0, "" and
    /// FALSE; incomparable types never match).
    pub fn dax_equals(
        table: impl Into<String>,
        column: impl Into<String>,
        value: impl Into<Value>,
    ) -> Self {
        let value = value.into();
        let label = format!("= {value:?}");
        Self::predicate(table, column, move |v| v.dax_eq(&value).unwrap_or(false)).with_label(label)
    }

    /// Values for which `f` returns `true`.
    pub fn predicate(
        table: impl Into<String>,
        column: impl Into<String>,
        f: impl Fn(&Value) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self::new(table, column, FilterKind::Predicate(Predicate::new(f)))
    }

    /// The values of the column at `rows`.
    pub fn rows(table: impl Into<String>, column: impl Into<String>, rows: RowSet) -> Self {
        Self::new(table, column, FilterKind::Rows(Arc::new(rows)))
    }

    /// Labels a predicate filter (no-op for other kinds).
    pub fn with_label(mut self, label: impl Into<Arc<str>>) -> Self {
        if let FilterKind::Predicate(p) = self.kind {
            self.kind = FilterKind::Predicate(p.with_label(label));
        }
        self
    }

    /// `true` if this filter is on `table[column]` (case-insensitive).
    pub fn is_on(&self, table: &str, column: &str) -> bool {
        names_eq(&self.table, table) && names_eq(&self.column, column)
    }
}

/// A filter on the rows of one table.
#[derive(Debug, Clone, PartialEq)]
pub struct TableFilter {
    pub table: String,
    pub rows: Arc<RowSet>,
    /// The table's blank row passes the filter (e.g. `FILTER(T, …)` kept it).
    pub include_blank_row: bool,
    /// Filter the expanded table (DAX table-filter semantics, default): the
    /// tables reachable over many → one relationships are restricted to the
    /// rows related to `rows`.
    pub expanded: bool,
    /// Expanded tables whose filter was removed (`ALL(Stores)` after a
    /// `FILTER(Orders, …)`).
    ignored: Vec<String>,
}

impl TableFilter {
    /// Expanded-table filter on `rows` of `table` (blank row excluded).
    pub fn new(table: impl Into<String>, rows: RowSet) -> Self {
        Self {
            table: table.into(),
            rows: Arc::new(rows),
            include_blank_row: false,
            expanded: true,
            ignored: Vec::new(),
        }
    }

    /// Filters only the base table, not its expanded table.
    pub fn base_only(mut self) -> Self {
        self.expanded = false;
        self
    }

    pub fn with_blank_row(mut self, include: bool) -> Self {
        self.include_blank_row = include;
        self
    }

    /// `true` if this filter restricts `table` through its expanded table.
    fn expands_to(&self, table: &str) -> bool {
        self.expanded && !self.ignored.iter().any(|t| names_eq(t, table))
    }
}

/// A filter in a [`FilterContext`].
#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
    Column(ColumnFilter),
    Table(TableFilter),
}

impl Filter {
    /// The filtered (base) table.
    pub fn table(&self) -> &str {
        match self {
            Filter::Column(f) => &f.table,
            Filter::Table(f) => &f.table,
        }
    }

    fn fingerprint(&self) -> u64 {
        let mut h = DefaultHasher::new();
        match self {
            Filter::Column(f) => {
                0u8.hash(&mut h);
                pbi_model::names::name_key(&f.table).hash(&mut h);
                pbi_model::names::name_key(&f.column).hash(&mut h);
                match &f.kind {
                    FilterKind::In(keys) => {
                        let mut sum = 0u64;
                        for k in keys.iter() {
                            let mut kh = DefaultHasher::new();
                            k.hash(&mut kh);
                            sum = sum.wrapping_add(kh.finish());
                        }
                        (1u8, keys.len(), sum).hash(&mut h);
                    }
                    FilterKind::Predicate(p) => (2u8, p.id).hash(&mut h),
                    FilterKind::Rows(rows) => (3u8, rows.as_ref()).hash(&mut h),
                }
            }
            Filter::Table(f) => {
                1u8.hash(&mut h);
                pbi_model::names::name_key(&f.table).hash(&mut h);
                f.rows.as_ref().hash(&mut h);
                (f.include_blank_row, f.expanded, &f.ignored).hash(&mut h);
            }
        }
        h.finish()
    }
}

impl From<ColumnFilter> for Filter {
    fn from(f: ColumnFilter) -> Self {
        Filter::Column(f)
    }
}

impl From<TableFilter> for Filter {
    fn from(f: TableFilter) -> Self {
        Filter::Table(f)
    }
}

/// The set of filters in effect (all intersected), plus the relationship
/// overrides of the evaluation. Cloning is cheap.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FilterContext {
    filters: Vec<Filter>,
    relationships: Vec<RelationshipOverride>,
}

impl FilterContext {
    /// No filters.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn filters(&self) -> &[Filter] {
        &self.filters
    }

    /// `true` when there is no filter (relationship overrides are not
    /// filters).
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }

    /// `USERELATIONSHIP` / `CROSSFILTER` overrides in effect, in the order
    /// they were set (later ones win).
    pub fn relationship_overrides(&self) -> &[RelationshipOverride] {
        &self.relationships
    }

    pub fn has_relationship_overrides(&self) -> bool {
        !self.relationships.is_empty()
    }

    /// Sets a relationship override (DAX `USERELATIONSHIP`/`CROSSFILTER`
    /// in `CALCULATE`), replacing an earlier override of the same kind on
    /// the same relationship. Visibility is then computed on
    /// [`RelationshipGraph::with_overrides`].
    pub fn set_relationship_override(&mut self, o: RelationshipOverride) {
        self.relationships.retain(|r| !r.same_slot(&o));
        self.relationships.push(o);
    }

    /// Builder form of [`FilterContext::set_relationship_override`].
    pub fn with_relationship_override(mut self, o: RelationshipOverride) -> Self {
        self.set_relationship_override(o);
        self
    }

    /// Removes every relationship override.
    pub fn clear_relationship_overrides(&mut self) {
        self.relationships.clear();
    }

    pub fn len(&self) -> usize {
        self.filters.len()
    }

    /// Adds a filter, intersecting with existing ones (`KEEPFILTERS`).
    pub fn add(&mut self, filter: impl Into<Filter>) {
        self.filters.push(filter.into());
    }

    /// Builder form of [`FilterContext::add`].
    pub fn with(mut self, filter: impl Into<Filter>) -> Self {
        self.add(filter);
        self
    }

    /// `CALCULATE` semantics for a column filter: removes the filters on the
    /// same column, then adds `filter`.
    pub fn replace_column(&mut self, filter: ColumnFilter) {
        self.remove_column(&filter.table, &filter.column);
        self.filters.push(Filter::Column(filter));
    }

    /// `CALCULATE` semantics for a table filter: removes the filters on the
    /// table (and, for an expanded filter, on its expanded table), then adds
    /// `filter`.
    pub fn replace_table(&mut self, filter: TableFilter, graph: &RelationshipGraph) {
        if filter.expanded {
            self.remove_expanded_table(graph, &filter.table);
        } else {
            self.remove_table(&filter.table);
        }
        self.filters.push(Filter::Table(filter));
    }

    /// `ALL(table[column])`: removes the column filters on that column.
    /// Table filters on the table are kept (multi-column filters are not
    /// projected yet).
    pub fn remove_column(&mut self, table: &str, column: &str) {
        self.filters
            .retain(|f| !matches!(f, Filter::Column(c) if c.is_on(table, column)));
    }

    /// Removes every filter on the base table `table` (column and table
    /// filters), and stops expanded table filters on other tables from
    /// restricting `table`.
    pub fn remove_table(&mut self, table: &str) {
        self.filters.retain(|f| !names_eq(f.table(), table));
        for f in &mut self.filters {
            if let Filter::Table(t) = f
                && t.expanded
                && !t.ignored.iter().any(|i| names_eq(i, table))
            {
                t.ignored.push(table.to_owned());
            }
        }
    }

    /// DAX `ALL(table)`: removes the filters on `table` and on every table
    /// of its expanded table.
    pub fn remove_expanded_table(&mut self, graph: &RelationshipGraph, table: &str) {
        let related: Vec<String> = graph
            .expanded_tables(table)
            .iter()
            .map(|(t, _)| t.clone())
            .collect();
        self.remove_table(table);
        for t in related {
            self.remove_table(&t);
        }
    }

    /// `ALL()`: removes every filter (relationship overrides stay).
    pub fn clear(&mut self) {
        self.filters.clear();
    }

    /// `true` if a filter applies directly to `table[column]` (a column filter
    /// on it, or a table filter on its table). Cross-filtering through
    /// relationships is not considered.
    pub fn is_column_filtered(&self, table: &str, column: &str) -> bool {
        self.filters.iter().any(|f| match f {
            Filter::Column(c) => c.is_on(table, column),
            Filter::Table(t) => names_eq(&t.table, table),
        })
    }

    /// `true` if any filter is directly on `table`.
    pub fn is_table_filtered(&self, table: &str) -> bool {
        self.filters.iter().any(|f| names_eq(f.table(), table))
    }

    /// Column filters on `table[column]`.
    pub fn column_filters<'a>(
        &'a self,
        table: &'a str,
        column: &'a str,
    ) -> impl Iterator<Item = &'a ColumnFilter> + 'a {
        self.filters.iter().filter_map(move |f| match f {
            Filter::Column(c) if c.is_on(table, column) => Some(c),
            _ => None,
        })
    }

    /// Order-independent hash of the filters (for caches; equal contexts
    /// have equal fingerprints).
    pub fn fingerprint(&self) -> u64 {
        let filters = self
            .filters
            .iter()
            .map(Filter::fingerprint)
            .fold(self.filters.len() as u64, u64::wrapping_add);
        if self.relationships.is_empty() {
            return filters;
        }
        let mut h = DefaultHasher::new();
        self.relationships.hash(&mut h);
        filters.wrapping_add(h.finish())
    }
}

/// Visible rows of a table in a filter context.
#[derive(Debug, Clone, PartialEq)]
pub struct VisibleRows {
    /// Visible real rows.
    pub rows: RowSet,
    /// The table's virtual blank row is visible (it exists and passes the
    /// filters). `VALUES`/`DISTINCTCOUNT`/`COUNTROWS(table)` count it.
    pub blank_row: bool,
}

impl RelationshipGraph {
    /// Rows of `table` visible in `ctx` (see the module docs), honouring
    /// the context's relationship overrides.
    pub fn visible(
        &self,
        storage: &Storage,
        ctx: &FilterContext,
        table: &str,
    ) -> Result<VisibleRows> {
        if ctx.has_relationship_overrides() {
            return self
                .with_overrides(storage, ctx.relationship_overrides())
                .visible_on_graph(storage, ctx, table);
        }
        self.visible_on_graph(storage, ctx, table)
    }

    /// [`RelationshipGraph::visible`] on this graph as is (overrides in
    /// `ctx` already applied by the caller).
    pub(crate) fn visible_on_graph(
        &self,
        storage: &Storage,
        ctx: &FilterContext,
        table: &str,
    ) -> Result<VisibleRows> {
        let data = storage.table(table)?;
        let n = data.row_count();
        validate(storage, ctx)?;
        if ctx.is_empty() {
            return Ok(VisibleRows {
                rows: RowSet::all(n),
                blank_row: self.has_blank_row(table),
            });
        }
        let single;
        let nodes = match self.tree(table) {
            Some(tree) => tree.nodes.as_slice(),
            None => {
                single = [crate::relationships::TreeNode {
                    table: data.name().to_owned(),
                    parent: None,
                }];
                &single[..]
            }
        };
        let mut acc: Vec<Option<RowSet>> = vec![None; nodes.len()];
        let mut result = None;
        for (i, node) in nodes.iter().enumerate().rev() {
            let node_data = storage.table(&node.table)?;
            let own = self.own_filter(storage, ctx, node_data)?;
            let mut combined = intersect(own, acc.get_mut(i).and_then(Option::take));
            if let Some(set) = combined.as_mut()
                && !self.has_blank_row(&node.table)
            {
                set.remove(node_data.row_count());
            }
            let Some((parent, rel, flow)) = node.parent else {
                result = combined;
                break;
            };
            let Some(rows) = combined else {
                continue;
            };
            let Some(projected) = self.project(rel, flow, &rows) else {
                continue;
            };
            if let Some(slot) = acc.get_mut(parent) {
                *slot = intersect(slot.take(), Some(projected));
            }
        }
        Ok(match result {
            Some(set) => VisibleRows {
                blank_row: set.contains(n) && self.has_blank_row(table),
                rows: set.resized(n),
            },
            None => VisibleRows {
                rows: RowSet::all(n),
                blank_row: self.has_blank_row(table),
            },
        })
    }

    /// Visible real rows of `table` in `ctx`.
    pub fn visible_rows(
        &self,
        storage: &Storage,
        ctx: &FilterContext,
        table: &str,
    ) -> Result<RowSet> {
        Ok(self.visible(storage, ctx, table)?.rows)
    }

    /// Rows (`n + 1` universe, last bit = blank row) of `data` passing the
    /// filters that apply to it directly: its column and table filters, and
    /// expanded table filters on tables whose expanded table contains it.
    /// `None` when nothing applies.
    fn own_filter(
        &self,
        storage: &Storage,
        ctx: &FilterContext,
        data: &TableData,
    ) -> Result<Option<RowSet>> {
        let n = data.row_count();
        let table = data.name();
        let mut result: Option<RowSet> = None;
        for filter in ctx.filters() {
            let rows = match filter {
                Filter::Column(f) if names_eq(&f.table, table) => column_filter_rows(data, f)?,
                Filter::Table(f) if names_eq(&f.table, table) => {
                    let mut rows = f.rows.resized(n + 1);
                    if f.include_blank_row {
                        rows.insert(n);
                    }
                    rows
                }
                Filter::Table(f) if f.expands_to(table) => {
                    let Some(path) = self.expanded_path(&f.table, table) else {
                        continue;
                    };
                    let source_rows = storage.row_count(&f.table)?;
                    let mut start = f.rows.resized(source_rows + 1);
                    if f.include_blank_row && self.has_blank_row(&f.table) {
                        start.insert(source_rows);
                    }
                    self.image(path, &start, n)
                }
                _ => continue,
            };
            result = intersect(result, Some(rows));
        }
        Ok(result)
    }
}

fn intersect(a: Option<RowSet>, b: Option<RowSet>) -> Option<RowSet> {
    match (a, b) {
        (Some(mut a), Some(b)) => {
            a.intersect_with(&b);
            Some(a)
        }
        (a, None) => a,
        (None, b) => b,
    }
}

/// Every filter must reference an existing table (and column).
fn validate(storage: &Storage, ctx: &FilterContext) -> Result<()> {
    for f in ctx.filters() {
        let data = storage.table(f.table())?;
        if let Filter::Column(c) = f
            && !data.has_column(&c.column)
        {
            return Err(EngineError::ColumnNotFound {
                table: c.table.clone(),
                column: c.column.clone(),
            });
        }
    }
    Ok(())
}

/// Rows (`n + 1` universe incl. the blank row, whose value is BLANK) whose
/// value passes a column filter.
fn column_filter_rows(data: &TableData, filter: &ColumnFilter) -> Result<RowSet> {
    let enc = data.encoded(&filter.column)?;
    let n = data.row_count();
    let cardinality = enc.cardinality();
    // Selective `IN` lists (the filters `SUMMARIZECOLUMNS` and context
    // transition add per data point) read the matching rows from the
    // column's inverted index instead of scanning every row.
    if let FilterKind::In(keys) = &filter.kind
        && keys.len().saturating_mul(SPARSE_RATIO) <= cardinality
    {
        let mut ids: Vec<u32> = keys.iter().filter_map(|k| enc.id_of(k)).collect();
        ids.sort_unstable();
        ids.dedup();
        let mut out = RowSet::empty(n + 1);
        for id in ids {
            for row in enc.rows_of(id) {
                out.insert(*row as usize);
            }
        }
        if keys.contains(&ValueKey::Blank) {
            out.insert(n);
        }
        return Ok(out);
    }
    let mut allowed = vec![false; cardinality];
    let blank_ok = match &filter.kind {
        FilterKind::In(keys) => {
            if keys.len() <= allowed.len() {
                for key in keys.iter() {
                    if let Some(slot) = enc.id_of(key).and_then(|id| allowed.get_mut(id as usize)) {
                        *slot = true;
                    }
                }
            } else {
                for (slot, value) in allowed.iter_mut().zip(enc.values()) {
                    *slot = keys.contains(&value.key());
                }
            }
            keys.contains(&ValueKey::Blank)
        }
        FilterKind::Predicate(p) => {
            for (slot, value) in allowed.iter_mut().zip(enc.values()) {
                *slot = p.eval(value);
            }
            p.eval(&Value::Blank)
        }
        FilterKind::Rows(rows) => {
            for row in rows.iter() {
                if let Some(slot) = enc.id_at(row).and_then(|id| allowed.get_mut(id as usize)) {
                    *slot = true;
                }
            }
            enc.blank_id()
                .and_then(|id| allowed.get(id as usize).copied())
                .unwrap_or(false)
        }
    };
    let mut out = RowSet::empty(n + 1);
    for (row, id) in enc.ids().iter().enumerate() {
        if allowed.get(*id as usize).copied().unwrap_or(false) {
            out.insert(row);
        }
    }
    if blank_ok {
        out.insert(n);
    }
    Ok(out)
}

/// An `IN` filter keeping at most 1/`SPARSE_RATIO` of a column's distinct
/// values is applied through the column's inverted index.
const SPARSE_RATIO: usize = 4;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_and_remove_follow_calculate_semantics() {
        let mut ctx = FilterContext::new()
            .with(ColumnFilter::equals("Stores", "Region", "West"))
            .with(ColumnFilter::equals("Stores", "Channel", "X"))
            .with(TableFilter::new("Orders", RowSet::all(3)));
        ctx.replace_column(ColumnFilter::equals("stores", "REGION", "East"));
        assert_eq!(ctx.len(), 3);
        assert_eq!(ctx.column_filters("Stores", "Region").count(), 1);
        assert!(ctx.is_column_filtered("Orders", "Store ID"));
        ctx.remove_column("Stores", "Channel");
        assert_eq!(ctx.len(), 2);
        ctx.remove_table("Stores");
        assert_eq!(ctx.len(), 1);
        let Filter::Table(t) = &ctx.filters()[0] else {
            panic!("expected table filter")
        };
        assert!(!t.expands_to("stores"));
        ctx.clear();
        assert!(ctx.is_empty());
    }

    #[test]
    fn fingerprints_are_order_independent_and_predicates_compare_by_identity() {
        let a = ColumnFilter::equals("T", "C", "x");
        let b = ColumnFilter::in_values("T", "D", [Value::Integer(1), Value::Integer(2)]);
        let ab = FilterContext::new().with(a.clone()).with(b.clone());
        let ba = FilterContext::new().with(b).with(a);
        assert_eq!(ab.fingerprint(), ba.fingerprint());
        let p = ColumnFilter::predicate("T", "C", |_| true);
        assert_eq!(p, p.clone());
        assert_ne!(p, ColumnFilter::predicate("T", "C", |_| true));
    }
}
