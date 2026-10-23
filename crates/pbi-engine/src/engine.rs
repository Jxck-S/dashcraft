//! The engine façade: owns the storage and relationship graph, publishes
//! immutable [`EngineSnapshot`]s for concurrent readers.

use std::collections::HashMap;
use std::fmt;
use std::ops::Deref;
use std::sync::{Arc, Mutex};

use arrow::array::ArrayRef;
use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use pbi_model::names::name_key;
use pbi_model::{Diagnostic, Model};

use crate::error::Result;
use crate::filter::{FilterContext, VisibleRows};
use crate::relationships::{RelationshipGraph, RelationshipOverride};
use crate::rowset::RowSet;
use crate::security::{SecurityContext, SecurityRole};
use crate::sql::SqlState;
use crate::storage::{Storage, TableData};
use crate::value::Value;

/// Maximum cached `visible` results per snapshot.
const CACHE_CAPACITY: usize = 4096;
/// Maximum cached relationship-override graphs per snapshot.
const GRAPH_CACHE_CAPACITY: usize = 64;

/// The storage engine of one open model.
///
/// Mutations (`load_table`, `add_column`, …) build a new snapshot
/// (copy-on-write: unchanged tables and relationship indexes are shared) and
/// rebuild the relationship indexes of the affected tables. Readers call
/// [`Engine::snapshot`] to get a cheap `Arc` that stays valid and consistent
/// while the engine keeps changing. `Engine` dereferences to the current
/// [`EngineSnapshot`], so all read operations are available directly.
/// Cloning is cheap: the clone shares the current snapshot and diverges
/// with its own mutations.
#[derive(Debug, Clone)]
pub struct Engine {
    current: Arc<EngineSnapshot>,
}

impl Engine {
    /// An engine for `model`: every model table starts empty (model schema),
    /// relationships are indexed as tables get loaded.
    pub fn new(model: &Model) -> Self {
        let storage = Storage::from_model(model);
        let mut diagnostics = Vec::new();
        let defs =
            RelationshipGraph::definitions_from_model(&model.relationships, &mut diagnostics);
        let graph = RelationshipGraph::build(defs, diagnostics, &storage);
        Self {
            current: Arc::new(EngineSnapshot::new(storage, Arc::new(graph), 0, None)),
        }
    }

    /// Sets (or clears) the security context the snapshots carry: the
    /// roles applied and the effective user (`USERPRINCIPALNAME()` & co.).
    /// It does not filter any data; see [`Engine::with_row_security`].
    pub fn set_security_context(&mut self, security: Option<SecurityContext>) {
        let generation = self.current.generation + 1;
        self.current = Arc::new(EngineSnapshot::new(
            self.current.storage.clone(),
            Arc::clone(&self.current.graph),
            generation,
            security.map(Arc::new),
        ));
    }

    /// A copy of this engine restricted by row-level security: every table
    /// keeps only the rows a member of `roles` may see
    /// ([`EngineSnapshot::security_masks`]: role filters propagate along
    /// relationships in their security direction; union over the roles),
    /// and relationship indexes and blank rows are rebuilt on the
    /// restricted data. The copy carries this engine's security context.
    ///
    /// # Errors
    /// A role filter on a table that does not exist.
    pub fn with_row_security(&self, roles: &[SecurityRole]) -> Result<Engine> {
        let masks = self.current.security_masks(roles)?;
        let mut storage = self.current.storage.clone();
        let mut changed: Vec<String> = Vec::new();
        let names: Vec<String> = storage.table_names().map(str::to_owned).collect();
        for name in names {
            let Some(mask) = masks.get(&name_key(&name)) else {
                continue;
            };
            if mask.rows.is_all() {
                continue;
            }
            storage.restrict_table(&name, &mask.rows)?;
            changed.push(name);
        }
        let changed: Vec<&str> = changed.iter().map(String::as_str).collect();
        let graph = self.current.graph.rebuild(&storage, &changed);
        Ok(Engine {
            current: Arc::new(EngineSnapshot::new(
                storage,
                Arc::new(graph),
                self.current.generation + 1,
                self.current.security.clone(),
            )),
        })
    }

    /// The current read-only snapshot (cheap; `Send + Sync`).
    pub fn snapshot(&self) -> Arc<EngineSnapshot> {
        Arc::clone(&self.current)
    }

    /// Loads (or reloads) a table's data.
    pub fn load_table(&mut self, name: &str, batch: RecordBatch) -> Result<()> {
        let mut storage = self.current.storage.clone();
        storage.load_table(name, batch)?;
        self.commit(storage, &[name]);
        Ok(())
    }

    /// Loads a table from several batches (concatenated).
    pub fn load_batches(
        &mut self,
        name: &str,
        schema: SchemaRef,
        batches: &[RecordBatch],
    ) -> Result<()> {
        let mut storage = self.current.storage.clone();
        storage.load_batches(name, schema, batches)?;
        self.commit(storage, &[name]);
        Ok(())
    }

    /// Loads several tables at once (relationships are rebuilt once). Either
    /// all tables load or none does.
    pub fn load_tables(
        &mut self,
        tables: impl IntoIterator<Item = (String, RecordBatch)>,
    ) -> Result<()> {
        let mut storage = self.current.storage.clone();
        let mut names = Vec::new();
        for (name, batch) in tables {
            storage.load_table(&name, batch)?;
            names.push(name);
        }
        let changed: Vec<&str> = names.iter().map(String::as_str).collect();
        self.commit(storage, &changed);
        Ok(())
    }

    /// Replaces the data of an existing table.
    pub fn replace_table(&mut self, name: &str, batch: RecordBatch) -> Result<()> {
        let mut storage = self.current.storage.clone();
        storage.replace_table(name, batch)?;
        self.commit(storage, &[name]);
        Ok(())
    }

    /// Adds (or replaces) a column, e.g. a calculated column computed by
    /// `pbi-dax`. The array must have the table's row count.
    pub fn add_column(&mut self, table: &str, column: &str, array: ArrayRef) -> Result<()> {
        let mut storage = self.current.storage.clone();
        storage.add_column(table, column, array)?;
        self.commit(storage, &[table]);
        Ok(())
    }

    /// Removes a table; returns `false` if it did not exist.
    pub fn remove_table(&mut self, name: &str) -> bool {
        let mut storage = self.current.storage.clone();
        let removed = storage.remove_table(name).is_some();
        if removed {
            self.commit(storage, &[name]);
        }
        removed
    }

    /// Rebuilds every relationship index from the current data.
    pub fn rebuild_relationships(&mut self) {
        let graph = self.current.graph.rebuild_all(&self.current.storage);
        self.publish(self.current.storage.clone(), graph);
    }

    /// Re-reads the relationships from `model` (e.g. after an edit) and adds
    /// empty tables for model tables that are not stored yet.
    pub fn sync_model(&mut self, model: &Model) {
        let mut storage = self.current.storage.clone();
        for table in &model.tables {
            storage.ensure_model_table(table);
        }
        let mut diagnostics = Vec::new();
        let defs =
            RelationshipGraph::definitions_from_model(&model.relationships, &mut diagnostics);
        let graph = RelationshipGraph::build(defs, diagnostics, &storage);
        self.publish(storage, graph);
    }

    fn commit(&mut self, storage: Storage, changed: &[&str]) {
        let graph = self.current.graph.rebuild(&storage, changed);
        self.publish(storage, graph);
    }

    fn publish(&mut self, storage: Storage, graph: RelationshipGraph) {
        let generation = self.current.generation + 1;
        tracing::debug!(generation, "pbi-engine: new snapshot");
        self.current = Arc::new(EngineSnapshot::new(
            storage,
            Arc::new(graph),
            generation,
            self.current.security.clone(),
        ));
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new(&Model::default())
    }
}

impl Deref for Engine {
    type Target = EngineSnapshot;

    fn deref(&self) -> &EngineSnapshot {
        &self.current
    }
}

type CacheEntries = HashMap<(u64, String), Vec<(FilterContext, VisibleRows)>>;

#[derive(Default)]
struct VisibleCache {
    entries: CacheEntries,
    len: usize,
}

/// An immutable, consistent view of the engine's data and relationships.
/// `Send + Sync`; share it as `Arc<EngineSnapshot>` across threads and async
/// tasks. All storage-engine reads (filter propagation, primitives in
/// [`crate::ops`], SQL) live here.
pub struct EngineSnapshot {
    storage: Storage,
    graph: Arc<RelationshipGraph>,
    generation: u64,
    cache: Mutex<VisibleCache>,
    override_graphs: Mutex<HashMap<Vec<RelationshipOverride>, Arc<RelationshipGraph>>>,
    security: Option<Arc<SecurityContext>>,
    pub(crate) sql: SqlState,
}

impl fmt::Debug for EngineSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EngineSnapshot")
            .field("generation", &self.generation)
            .field("tables", &self.storage.table_names().collect::<Vec<_>>())
            .field("relationships", &self.graph.definitions().len())
            .finish()
    }
}

impl EngineSnapshot {
    fn new(
        storage: Storage,
        graph: Arc<RelationshipGraph>,
        generation: u64,
        security: Option<Arc<SecurityContext>>,
    ) -> Self {
        Self {
            storage,
            graph,
            generation,
            cache: Mutex::new(VisibleCache::default()),
            override_graphs: Mutex::new(HashMap::new()),
            security,
            sql: SqlState::default(),
        }
    }

    /// The security context (roles applied, effective user), if one was
    /// set ([`Engine::set_security_context`]).
    pub fn security(&self) -> Option<&SecurityContext> {
        self.security.as_deref()
    }

    pub fn storage(&self) -> &Storage {
        &self.storage
    }

    pub fn graph(&self) -> &RelationshipGraph {
        &self.graph
    }

    /// The relationship graph in effect for `ctx`: [`EngineSnapshot::graph`]
    /// with the context's `USERELATIONSHIP`/`CROSSFILTER` overrides
    /// applied ([`RelationshipGraph::with_overrides`], cached per override
    /// set). Use it for expanded-table operations such as
    /// [`FilterContext::replace_table`] under overrides.
    pub fn graph_for(&self, ctx: &FilterContext) -> Arc<RelationshipGraph> {
        let overrides = ctx.relationship_overrides();
        if overrides.is_empty() {
            return Arc::clone(&self.graph);
        }
        if let Ok(cache) = self.override_graphs.lock()
            && let Some(g) = cache.get(overrides)
        {
            return Arc::clone(g);
        }
        let graph = Arc::new(self.graph.with_overrides(&self.storage, overrides));
        if let Ok(mut cache) = self.override_graphs.lock() {
            if cache.len() >= GRAPH_CACHE_CAPACITY {
                cache.clear();
            }
            cache.insert(overrides.to_vec(), Arc::clone(&graph));
        }
        graph
    }

    /// Increases with every engine mutation.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Relationship diagnostics (missing columns, duplicate keys, ambiguous
    /// paths, unsupported relationships).
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        self.graph.diagnostics()
    }

    pub fn table(&self, name: &str) -> Result<&Arc<TableData>> {
        self.storage.table(name)
    }

    pub fn table_names(&self) -> Vec<String> {
        self.storage.table_names().map(str::to_owned).collect()
    }

    pub fn row_count(&self, table: &str) -> Result<usize> {
        self.storage.row_count(table)
    }

    pub fn column(&self, table: &str, column: &str) -> Result<ArrayRef> {
        self.storage.column(table, column)
    }

    pub fn value_at(&self, table: &str, column: &str, row: usize) -> Result<Value> {
        self.storage.value_at(table, column, row)
    }

    /// Visible rows (and blank-row visibility) of `table` in `ctx`, cached
    /// per (context, table). See [`crate::filter`] for the rules.
    pub fn visible(&self, ctx: &FilterContext, table: &str) -> Result<VisibleRows> {
        if ctx.is_empty() && !ctx.has_relationship_overrides() {
            return self.graph.visible_on_graph(&self.storage, ctx, table);
        }
        let key = (ctx.fingerprint(), name_key(table));
        if let Ok(cache) = self.cache.lock()
            && let Some(bucket) = cache.entries.get(&key)
            && let Some((_, hit)) = bucket.iter().find(|(c, _)| c == ctx)
        {
            return Ok(hit.clone());
        }
        let visible = self
            .graph_for(ctx)
            .visible_on_graph(&self.storage, ctx, table)?;
        if let Ok(mut cache) = self.cache.lock() {
            if cache.len >= CACHE_CAPACITY {
                cache.entries.clear();
                cache.len = 0;
            }
            cache
                .entries
                .entry(key)
                .or_default()
                .push((ctx.clone(), visible.clone()));
            cache.len += 1;
        }
        Ok(visible)
    }

    /// Visible real rows of `table` in `ctx`.
    pub fn visible_rows(&self, ctx: &FilterContext, table: &str) -> Result<RowSet> {
        Ok(self.visible(ctx, table)?.rows)
    }

    /// `true` if `table`'s blank row exists and is visible in `ctx`.
    pub fn blank_row_visible(&self, ctx: &FilterContext, table: &str) -> Result<bool> {
        Ok(self.visible(ctx, table)?.blank_row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_and_snapshots_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Engine>();
        assert_send_sync::<EngineSnapshot>();
        assert_send_sync::<Arc<EngineSnapshot>>();
        assert_send_sync::<FilterContext>();
    }
}
