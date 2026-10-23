//! Per-visual render cache for the interactive canvas.
//!
//! Re-rendering a page after an interaction (scrolling one table, opening a
//! dropdown, a slicer click that changes some visuals' data) used to redraw
//! every visual. [`VisualCache`] keeps each visual's rendered content
//! [`Group`], its interactive geometry ([`VisualInteraction`]) and its open
//! dropdown popup, keyed by a **content fingerprint** of everything the
//! visual's renderer reads:
//!
//! - the visual definition (type, projections, `objects`,
//!   `visualContainerObjects`, position, …: the whole serialized
//!   [`pbi_model::Visual`]);
//! - its laid-out frame ([`crate::layout::VisualFrame`]: content box,
//!   title, border, … in page units; the viewport scale is applied by the
//!   page transform outside the cached group);
//! - the theme's content hash (memoized per
//!   [`ThemeResolver`](crate::theme::ThemeResolver));
//! - its query result (every DataView cell, or the error);
//! - its view state ([`VisualViewState`]: scroll, expansion, dropdown,
//!   search);
//! - the selection state it reads: its own slicer selection and the point
//!   selection when it is the cross-filter source (other visuals see
//!   selections only through their filtered data).
//!
//! Keys are content hashes, not revision counters, so a cached group is
//! reused exactly when the inputs are equal (scrolling one table re-renders
//! that table only; going back to an earlier state is a hit) and can never
//! be stale because a counter did not move. Rendering is deterministic, so a
//! cached group is identical to a fresh render and pages render to the same
//! bytes with or without the cache.
//!
//! # No work proportional to the data on a scroll step
//!
//! Hashing a 30 000-row table on every render would make each scroll step
//! O(rows). Two things keep it O(visible rows):
//!
//! - **Query-result identity.** [`super::PageData::id`] is set by
//!   [`super::query_page_data`] (process-unique). The content hash of each
//!   visual's result is memoized by `(id, visual)` and revalidated against
//!   the result's address and buffer pointers, so it is computed once per
//!   query, not per render. Results with id 0 (built by hand) are hashed
//!   on every render.
//! - **Prepared grids.** A table's or matrix's [`GridModel`](super::table)
//!   (one cell per value), style and measured layout are cached by the
//!   visual's key without its scroll offset (plus the matrix expansion), so
//!   a scroll step only lays out and draws the visible rows. The layout's
//!   per-row vectors are shared, not copied.
//!
//! The cache is bounded ([`CacheLimits`]): beyond `max_entries` entries or
//! `max_nodes` scene nodes, the least recently used quarter-plus is evicted;
//! prepared grids are bounded by `max_grid_cells`. [`VisualCache::shared`]
//! is the process-wide instance [`super::render_interactive_page`] uses.
//!
//! **Invariant for visual renderers:** a renderer may only read
//! [`VisualCtx`] — anything else it depends on must be added to the key
//! (`VisualKey::new`). From [`VisualCtx::state`] it may only read its own
//! slicer selection and the point selection whose source it is.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use crate::dataview::DataView;
use crate::fingerprint::Fingerprint;
use crate::query::VisualQueryError;
use crate::scene::{Group, Node};

use super::VisualCtx;
use super::page::{VisualInteraction, VisualViewState};
use super::table::grid::PreparedGrid;

/// Size limits of a [`VisualCache`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheLimits {
    /// Most cached visual renders.
    pub max_entries: usize,
    /// Most scene nodes held in total (a rough memory bound: a node is a
    /// few hundred bytes).
    pub max_nodes: usize,
    /// Most grid cells held by prepared tables and matrices (about 100
    /// bytes each); a larger grid is rebuilt on every render.
    pub max_grid_cells: usize,
}

impl Default for CacheLimits {
    /// 2048 renders, 250 000 nodes, 1 000 000 grid cells (tens of MB).
    fn default() -> Self {
        Self {
            max_entries: 2048,
            max_nodes: 250_000,
            max_grid_cells: 1_000_000,
        }
    }
}

/// Counters of a [`VisualCache`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    /// Entries dropped to stay within the limits.
    pub evictions: u64,
    /// Visuals rendered without caching (an input could not be hashed).
    pub uncacheable: u64,
    /// Prepared grids reused / built.
    pub grid_hits: u64,
    pub grid_misses: u64,
    /// Current entries.
    pub entries: usize,
    /// Scene nodes held by the current entries.
    pub nodes: usize,
}

/// Identity of one visual render: the visual's name and a fingerprint of
/// every input (see the module docs).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct VisualKey {
    visual: String,
    fingerprint: u64,
}

impl VisualKey {
    /// The key of rendering `ctx` with `view` for the interactive canvas
    /// (content, geometry and popup), given the fingerprint of its query
    /// result ([`data_fingerprint`]), and the fingerprint of every input
    /// but the view state (the prepared-grid key). `None` when an input
    /// cannot be hashed.
    pub(crate) fn new(
        ctx: &VisualCtx<'_>,
        view: Option<&VisualViewState>,
        data: u64,
    ) -> Option<(Self, u64)> {
        let name = &ctx.visual.name;
        let mut fp = Fingerprint::new();
        fp.add_u64(ctx.theme.fingerprint()?)
            .add(ctx.frame)
            .add(ctx.visual)
            .add_u64(data)
            .add(&ctx.state.slicer_selections.get(name))
            .add(
                &ctx.state
                    .selected_points
                    .as_ref()
                    .filter(|p| p.source == *name),
            );
        let base = fp.finish()?;
        let fingerprint = Fingerprint::new()
            .add_u64(base)
            .add(&view_fingerprint(view)?)
            .finish()?;
        Some((
            Self {
                visual: name.clone(),
                fingerprint,
            },
            base,
        ))
    }
}

/// Query results memoized by [`data_fingerprint`] before the memo is
/// cleared.
const DATA_MEMO_MAX: usize = 4096;

/// Cheap identity of a query result in memory: its address and the
/// addresses and lengths of its main buffers. A replaced result (new
/// buffers) never matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Shape([usize; 11]);

impl Shape {
    fn of(result: &Result<DataView, VisualQueryError>, dv: &DataView) -> Self {
        let slice = |p: *const u8, len: usize| (p.addr(), len);
        let (meta, meta_len) = slice(dv.metadata.as_ptr().cast(), dv.metadata.len());
        let (rows, rows_len) = dv
            .table
            .as_ref()
            .map_or((0, 0), |t| slice(t.rows.as_ptr().cast(), t.rows.len()));
        let (cats, cats_len) = dv.categorical.as_ref().map_or((0, 0), |c| {
            slice(c.categories.as_ptr().cast(), c.categories.len())
        });
        let (vals, vals_len) = dv
            .categorical
            .as_ref()
            .map_or((0, 0), |c| slice(c.values.as_ptr().cast(), c.values.len()));
        let (tree, tree_len) = dv.matrix.as_ref().map_or((0, 0), |m| {
            slice(m.rows.children.as_ptr().cast(), m.rows.children.len())
        });
        Self([
            std::ptr::from_ref(result).addr(),
            meta,
            meta_len,
            rows,
            rows_len,
            cats,
            cats_len,
            vals,
            vals_len,
            tree,
            tree_len,
        ])
    }
}

type DataMemo = HashMap<(u64, String), (Shape, u64)>;

/// Content fingerprint of a visual's query result (`None`: no query).
/// With a non-zero [`super::PageData::id`] the hash is memoized per
/// `(page_id, visual)` and revalidated by the result's [`Shape`], so it is
/// computed once per query instead of once per render.
pub(crate) fn data_fingerprint(
    page_id: u64,
    visual: &str,
    result: Option<&Result<DataView, VisualQueryError>>,
) -> Option<u64> {
    static MEMO: OnceLock<Mutex<DataMemo>> = OnceLock::new();
    let compute = || {
        let mut fp = Fingerprint::new();
        match result {
            None => fp.add_u64(0),
            Some(Ok(dv)) => fp.add_u64(1).add_dataview(dv),
            Some(Err(e)) => fp.add_u64(2).add(e),
        };
        fp.finish()
    };
    let (Some(r @ Ok(dv)), true) = (result, page_id != 0) else {
        return compute();
    };
    let shape = Shape::of(r, dv);
    let memo = MEMO.get_or_init(|| Mutex::new(HashMap::new()));
    let key = (page_id, visual.to_owned());
    if let Some((s, fp)) = memo
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&key)
        && *s == shape
    {
        return Some(*fp);
    }
    let fp = compute()?;
    let mut memo = memo.lock().unwrap_or_else(PoisonError::into_inner);
    if memo.len() >= DATA_MEMO_MAX {
        memo.clear();
    }
    memo.insert(key, (shape, fp));
    Some(fp)
}

/// Identity of a prepared grid: the visual, every input but the view state
/// (`base` from [`VisualKey::new`]) and the part of the view state the
/// grid's model depends on (a matrix's expansion; 0 for tables).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct GridKey {
    pub visual: String,
    pub base: u64,
    pub variant: u64,
}

/// Fingerprint of a view state; the default state renders like no state,
/// so both map to `None` (bitwise: `-0.0` scroll is not the default).
fn view_fingerprint(view: Option<&VisualViewState>) -> Option<Option<u64>> {
    static DEFAULT: OnceLock<Option<u64>> = OnceLock::new();
    let Some(view) = view else {
        return Some(None);
    };
    let fp = Fingerprint::new().add(view).finish()?;
    let default =
        *DEFAULT.get_or_init(|| Fingerprint::new().add(&VisualViewState::default()).finish());
    Some((Some(fp) != default).then_some(fp))
}

/// One cached visual render.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct CachedVisual {
    /// The content group ([`super::render_visual_with`]).
    pub content: Option<Group>,
    /// Interactive geometry (slicers, tables, matrices).
    pub interaction: Option<VisualInteraction>,
    /// The open dropdown popup (overlay).
    pub popup: Option<Group>,
}

impl CachedVisual {
    fn nodes(&self) -> usize {
        [&self.content, &self.popup]
            .into_iter()
            .flatten()
            .map(count_nodes)
            .sum::<usize>()
            + 1
    }
}

/// Nodes in `group`, itself included (iterative: no recursion limit).
fn count_nodes(group: &Group) -> usize {
    let mut n = 0;
    let mut stack: Vec<&Group> = vec![group];
    while let Some(g) = stack.pop() {
        n += 1;
        for child in &g.children {
            match child {
                Node::Group(c) => stack.push(c),
                _ => n += 1,
            }
        }
    }
    n
}

struct Entry {
    value: Arc<CachedVisual>,
    last_used: u64,
    nodes: usize,
}

struct GridEntry {
    grid: Arc<PreparedGrid>,
    last_used: u64,
    cells: usize,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<VisualKey, Entry>,
    tick: u64,
    nodes: usize,
    grids: HashMap<GridKey, GridEntry>,
    grid_cells: usize,
    stats: CacheStats,
}

/// A bounded cache of rendered visuals (see the module docs). Thread-safe;
/// renders on several threads may share one.
pub struct VisualCache {
    inner: Mutex<Inner>,
    limits: CacheLimits,
}

impl std::fmt::Debug for VisualCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VisualCache")
            .field("limits", &self.limits)
            .field("stats", &self.stats())
            .finish()
    }
}

impl Default for VisualCache {
    fn default() -> Self {
        Self::new()
    }
}

impl VisualCache {
    /// An empty cache with the default limits.
    pub fn new() -> Self {
        Self::with_limits(CacheLimits::default())
    }

    /// An empty cache with `limits` (a zero limit disables caching).
    pub fn with_limits(limits: CacheLimits) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            limits,
        }
    }

    /// The process-wide cache used by [`super::render_interactive_page`].
    pub fn shared() -> &'static VisualCache {
        static CACHE: OnceLock<VisualCache> = OnceLock::new();
        CACHE.get_or_init(VisualCache::new)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Drops every entry (counters are kept).
    pub fn clear(&self) {
        let mut inner = self.lock();
        inner.entries.clear();
        inner.nodes = 0;
        inner.grids.clear();
        inner.grid_cells = 0;
    }

    /// The prepared grid cached under `key` (counts a hit or a miss).
    pub(crate) fn grid(&self, key: &GridKey) -> Option<Arc<PreparedGrid>> {
        let mut inner = self.lock();
        inner.tick += 1;
        let tick = inner.tick;
        let found = inner.grids.get_mut(key).map(|e| {
            e.last_used = tick;
            Arc::clone(&e.grid)
        });
        if found.is_some() {
            inner.stats.grid_hits += 1;
        } else {
            inner.stats.grid_misses += 1;
        }
        found
    }

    /// Stores a prepared grid, evicting least recently used ones beyond
    /// `max_grid_cells` (a grid larger than that is not kept).
    pub(crate) fn insert_grid(&self, key: GridKey, grid: PreparedGrid) -> Arc<PreparedGrid> {
        let grid = Arc::new(grid);
        let cells = grid.cells();
        let max = self.limits.max_grid_cells;
        if cells > max || self.limits.max_entries == 0 {
            return grid;
        }
        let mut inner = self.lock();
        inner.tick += 1;
        let entry = GridEntry {
            grid: Arc::clone(&grid),
            last_used: inner.tick,
            cells,
        };
        if let Some(old) = inner.grids.insert(key, entry) {
            inner.grid_cells -= old.cells;
        }
        inner.grid_cells += cells;
        if inner.grid_cells > max {
            let mut by_age: Vec<(u64, GridKey)> = inner
                .grids
                .iter()
                .map(|(k, e)| (e.last_used, k.clone()))
                .collect();
            by_age.sort_unstable_by_key(|(t, _)| *t);
            for (_, key) in by_age {
                if inner.grid_cells <= max {
                    break;
                }
                if let Some(old) = inner.grids.remove(&key) {
                    inner.grid_cells -= old.cells;
                    inner.stats.evictions += 1;
                }
            }
        }
        grid
    }

    /// Hit / miss counters and the current size.
    pub fn stats(&self) -> CacheStats {
        let inner = self.lock();
        CacheStats {
            entries: inner.entries.len(),
            nodes: inner.nodes,
            ..inner.stats
        }
    }

    /// The render cached under `key` (counts a hit or a miss).
    pub(crate) fn get(&self, key: &VisualKey) -> Option<Arc<CachedVisual>> {
        let mut inner = self.lock();
        inner.tick += 1;
        let tick = inner.tick;
        let found = inner.entries.get_mut(key).map(|e| {
            e.last_used = tick;
            Arc::clone(&e.value)
        });
        if found.is_some() {
            inner.stats.hits += 1;
        } else {
            inner.stats.misses += 1;
        }
        found
    }

    /// Counts a visual rendered without a key.
    pub(crate) fn note_uncacheable(&self) {
        self.lock().stats.uncacheable += 1;
    }

    /// Stores `value` under `key`, evicting least recently used entries
    /// beyond the limits.
    pub(crate) fn insert(&self, key: VisualKey, value: CachedVisual) -> Arc<CachedVisual> {
        let value = Arc::new(value);
        let nodes = value.nodes();
        if self.limits.max_entries == 0 || nodes > self.limits.max_nodes {
            return value;
        }
        let mut inner = self.lock();
        inner.tick += 1;
        let entry = Entry {
            value: Arc::clone(&value),
            last_used: inner.tick,
            nodes,
        };
        if let Some(old) = inner.entries.insert(key, entry) {
            inner.nodes -= old.nodes;
        }
        inner.nodes += nodes;
        if inner.entries.len() > self.limits.max_entries || inner.nodes > self.limits.max_nodes {
            self.evict(&mut inner);
        }
        value
    }

    /// Drops least recently used entries until the cache is at most 3/4 of
    /// its limits.
    fn evict(&self, inner: &mut Inner) {
        let max_entries = self.limits.max_entries / 4 * 3;
        let max_nodes = self.limits.max_nodes / 4 * 3;
        let mut by_age: Vec<(u64, VisualKey)> = inner
            .entries
            .iter()
            .map(|(k, e)| (e.last_used, k.clone()))
            .collect();
        by_age.sort_unstable_by_key(|(t, _)| *t);
        for (_, key) in by_age {
            if inner.entries.len() <= max_entries && inner.nodes <= max_nodes {
                break;
            }
            if let Some(old) = inner.entries.remove(&key) {
                inner.nodes -= old.nodes;
                inner.stats.evictions += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str, fp: u64) -> VisualKey {
        VisualKey {
            visual: name.into(),
            fingerprint: fp,
        }
    }

    fn render(nodes: usize) -> CachedVisual {
        let mut g = Group::new();
        for _ in 0..nodes.saturating_sub(1) {
            g.push(Group::new());
        }
        CachedVisual {
            content: Some(g),
            ..CachedVisual::default()
        }
    }

    #[test]
    fn evicts_least_recently_used_entries() {
        let cache = VisualCache::with_limits(CacheLimits {
            max_entries: 4,
            max_nodes: 1000,
            ..CacheLimits::default()
        });
        for i in 0..4 {
            cache.insert(key("v", i), render(2));
        }
        // Touch 0 so 1 is the oldest.
        assert!(cache.get(&key("v", 0)).is_some());
        cache.insert(key("v", 4), render(2));
        // 5 > 4 entries: down to 3, dropping 1 and 2.
        let s = cache.stats();
        assert_eq!((s.entries, s.evictions), (3, 2));
        assert!(cache.get(&key("v", 1)).is_none());
        assert!(cache.get(&key("v", 0)).is_some());
        assert!(cache.get(&key("v", 4)).is_some());
        assert_eq!(cache.stats().nodes, 3 * 3);
    }

    #[test]
    fn node_budget_bounds_the_cache() {
        let cache = VisualCache::with_limits(CacheLimits {
            max_entries: 100,
            max_nodes: 40,
            ..CacheLimits::default()
        });
        // Larger than the whole budget: returned but not kept.
        cache.insert(key("big", 0), render(100));
        assert_eq!(cache.stats().entries, 0);
        for i in 0..10 {
            cache.insert(key("v", i), render(9));
        }
        let s = cache.stats();
        assert!(s.nodes <= 40 && s.entries >= 3, "{s:?}");
        cache.clear();
        assert_eq!(cache.stats().entries, 0);
    }
}
