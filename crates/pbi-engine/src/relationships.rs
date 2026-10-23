//! Relationship indexes and the filter-propagation graph.
//!
//! Each model relationship is normalised to a [`RelationshipDef`] with a
//! *many* side and a *one* side, and indexed ([`RelationshipIndex`]) as a
//! hash join on the key values: `many_row → Option<one_row>` plus the reverse
//! `one_row → [many_row]` in CSR form. Keys match with
//! [`crate::value::ValueKey`] semantics (text case-insensitively, BLANK only
//! with BLANK), like VertiPaq.
//!
//! **Blank row.** Many-side rows whose key has no match on the one side
//! relate to the one side's *blank row* — a virtual row whose columns are all
//! BLANK. The index models it as `None`; the one-side table "has a blank row"
//! ([`RelationshipGraph::has_blank_row`]) when any active relationship has
//! unmatched many-side rows.
//!
//! **Many-to-many relationships** (`fromCardinality` and `toCardinality`
//! both `many`) are *limited* relationships, as in Power BI: they are
//! indexed by key value ([`ManyToManyIndex`]) and a filter crosses them as
//! the set of distinct key values of the visible rows (like `TREATAS`):
//! rows of the other side whose key is not among them are hidden, no blank
//! row is added on either side, and they are not part of expanded tables
//! (`RELATED` cannot follow them). With single cross-filtering the `to`
//! side filters the `from` side; `bothDirections` filters both ways. An
//! unfiltered side does not restrict the other, so totals include rows
//! whose key has no match.
//!
//! **Propagation trees.** For every table `T` the graph precomputes which
//! tables' filters reach `T` and through which relationships
//! ([`PropagationTree`]). Filters flow one → many over every active
//! relationship, and many → one only over bidirectional (and one-to-one)
//! relationships. Each source table reaches `T` along exactly one path:
//! paths are ranked like Power BI (fewest many → one "reverse" hops, then
//! fewest many-to-many hops, then fewest hops); equally ranked alternatives
//! are reported as an ambiguity diagnostic and the first (model order)
//! wins. A table with no path to `T` never affects `T`.
//!
//! **Security.** [`RelationshipGraph::for_security`] is the graph row-level
//! security filters travel on: the same relationships with the direction
//! given by `securityFilteringBehavior` (`oneDirection`: one → many only,
//! `bothDirections`: both ways, `none`: not crossed).
//!
//! **Overrides.** DAX `USERELATIONSHIP` and `CROSSFILTER` change the
//! relationships for one evaluation. They travel in the
//! [`crate::FilterContext`] as [`RelationshipOverride`]s;
//! [`RelationshipGraph::with_overrides`] derives the graph they describe
//! (same indexes, different active set and directions), which
//! [`RelationshipGraph::visible`] and [`crate::EngineSnapshot::visible`]
//! (cached per override set, see [`crate::EngineSnapshot::graph_for`]) use.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use pbi_model::names::{name_key, names_eq};
use pbi_model::{
    Cardinality, ColumnRef, CrossFilter, Diagnostic, ObjectId, Relationship,
    SecurityFilteringBehavior,
};

use crate::rowset::RowSet;
use crate::storage::{EncodedColumn, Storage};
use crate::value::canonical_type;

/// Sentinel for "no matching one-side row" (the blank row).
const NO_ROW: u32 = u32::MAX;

/// [`RelationshipIndex::propagate_to_many`] walks the CSR index when at most
/// 1/`SPARSE_RATIO` of the one-side rows are selected.
const SPARSE_RATIO: usize = 4;

/// Normalised cardinality of a relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelationshipKind {
    ManyToOne,
    OneToOne,
    /// A limited relationship: filters cross it as key values
    /// ([`ManyToManyIndex`]); no blank row, no table expansion.
    ManyToMany,
}

/// Filter direction set by DAX `CROSSFILTER` for one evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CrossFilterDirection {
    /// `None`: the relationship is not used (no propagation, no table
    /// expansion).
    None,
    /// `OneWay`: the one side filters the many side.
    OneWay,
    /// `Both`: filters flow in both directions.
    Both,
    /// Only the many side filters the one side (`OneWay_LeftFiltersRight` /
    /// `OneWay_RightFiltersLeft` naming the many-side column as the
    /// filtering side).
    ManyToOne,
}

/// A per-evaluation change to the model relationships, carried by a
/// [`crate::FilterContext`] (see
/// [`crate::FilterContext::set_relationship_override`]). Relationships are
/// identified by their index in [`RelationshipGraph::definitions`] (see
/// [`RelationshipGraph::find_relationship`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RelationshipOverride {
    /// DAX `USERELATIONSHIP`: the relationship becomes active and every
    /// other active relationship between the same two tables is
    /// deactivated.
    Activate(usize),
    /// DAX `CROSSFILTER`: the relationship's filter direction.
    CrossFilter(usize, CrossFilterDirection),
}

impl RelationshipOverride {
    /// Index of the relationship (into [`RelationshipGraph::definitions`]).
    pub fn relationship(&self) -> usize {
        match self {
            RelationshipOverride::Activate(i) | RelationshipOverride::CrossFilter(i, _) => *i,
        }
    }

    /// `true` when both overrides are of the same kind (activation or
    /// direction) on the same relationship (the later one replaces the
    /// earlier one).
    pub fn same_slot(&self, other: &RelationshipOverride) -> bool {
        matches!(
            (self, other),
            (RelationshipOverride::Activate(a), RelationshipOverride::Activate(b))
                | (RelationshipOverride::CrossFilter(a, _), RelationshipOverride::CrossFilter(b, _))
                if a == b
        )
    }
}

/// A model relationship normalised so that `many` is the many side.
#[derive(Debug, Clone, PartialEq)]
pub struct RelationshipDef {
    /// Position in `Model::relationships`.
    pub ordinal: usize,
    pub id: ObjectId,
    pub name: String,
    /// Many side (`fromColumn` of a regular many-to-one relationship).
    pub many: ColumnRef,
    /// One side (`toColumn`).
    pub one: ColumnRef,
    pub kind: RelationshipKind,
    /// Filters also flow many → one (`bothDirections`, and every one-to-one
    /// relationship).
    pub bidirectional: bool,
    pub is_active: bool,
    /// How row-level security filters cross the relationship.
    pub security: SecurityFilteringBehavior,
}

impl RelationshipDef {
    /// Normalises a model relationship; unusual settings add diagnostics.
    pub fn from_model(
        ordinal: usize,
        rel: &Relationship,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Self {
        let label = describe(rel);
        let (many, one, kind) = match (&rel.from_cardinality, &rel.to_cardinality) {
            (Cardinality::Many, Cardinality::One) => (
                rel.from.clone(),
                rel.to.clone(),
                RelationshipKind::ManyToOne,
            ),
            (Cardinality::One, Cardinality::Many) => (
                rel.to.clone(),
                rel.from.clone(),
                RelationshipKind::ManyToOne,
            ),
            (Cardinality::One, Cardinality::One) => {
                (rel.from.clone(), rel.to.clone(), RelationshipKind::OneToOne)
            }
            (Cardinality::Many, Cardinality::Many) => (
                rel.from.clone(),
                rel.to.clone(),
                RelationshipKind::ManyToMany,
            ),
            (from, to) => {
                diagnostics.push(
                    Diagnostic::warning(
                        "ENG006",
                        format!(
                            "relationship {label}: unknown cardinality {}→{}; treated as many-to-one",
                            from.as_str(),
                            to.as_str()
                        ),
                    )
                    .with_object(rel.id),
                );
                (
                    rel.from.clone(),
                    rel.to.clone(),
                    RelationshipKind::ManyToOne,
                )
            }
        };
        let both = match &rel.cross_filtering_behavior {
            CrossFilter::BothDirections => true,
            CrossFilter::OneDirection | CrossFilter::Automatic => false,
            CrossFilter::Unknown(raw) => {
                diagnostics.push(
                    Diagnostic::warning(
                        "ENG006",
                        format!(
                            "relationship {label}: unknown cross-filter direction '{raw}'; treated as single"
                        ),
                    )
                    .with_object(rel.id),
                );
                false
            }
        };
        Self {
            ordinal,
            id: rel.id,
            name: rel.name.clone(),
            many,
            one,
            bidirectional: both || kind == RelationshipKind::OneToOne,
            kind,
            is_active: rel.is_active,
            security: rel.security_filtering_behavior.clone(),
        }
    }

    /// `Orders[Store ID] → Stores[Store ID]`.
    pub fn label(&self) -> String {
        format!(
            "{} → {}",
            pbi_model::dax_column_ref(&self.many.table, &self.many.column),
            pbi_model::dax_column_ref(&self.one.table, &self.one.column)
        )
    }
}

fn describe(rel: &Relationship) -> String {
    format!(
        "{} → {}",
        pbi_model::dax_column_ref(&rel.from.table, &rel.from.column),
        pbi_model::dax_column_ref(&rel.to.table, &rel.to.column)
    )
}

/// Hash-join index of one relationship over the current table data.
#[derive(Debug, Clone)]
pub struct RelationshipIndex {
    many_rows: usize,
    one_rows: usize,
    targets: Vec<u32>,
    offsets: Vec<u32>,
    sources: Vec<u32>,
    unmatched: usize,
    /// Many-side rows without a match (ascending).
    unmatched_rows: Vec<u32>,
    unreferenced: usize,
    /// One-side rows no many-side row refers to (ascending).
    unreferenced_rows: Vec<u32>,
    duplicate_keys: usize,
    duplicate_many_keys: usize,
}

impl RelationshipIndex {
    /// Joins the many-side key column with the one-side key column. Duplicate
    /// one-side keys are counted (the first row wins).
    pub fn build(many: &EncodedColumn, one: &EncodedColumn) -> Self {
        let mut first_row = vec![NO_ROW; one.cardinality()];
        let mut duplicate_keys = 0;
        for (row, id) in one.ids().iter().enumerate() {
            if let Some(slot) = first_row.get_mut(*id as usize) {
                if *slot == NO_ROW {
                    *slot = u32::try_from(row).unwrap_or(NO_ROW);
                } else {
                    duplicate_keys += 1;
                }
            }
        }
        let by_many_id: Vec<u32> = many
            .values()
            .iter()
            .map(|v| {
                one.id_of(&v.key())
                    .and_then(|id| first_row.get(id as usize).copied())
                    .unwrap_or(NO_ROW)
            })
            .collect();
        let targets: Vec<u32> = many
            .ids()
            .iter()
            .map(|id| by_many_id.get(*id as usize).copied().unwrap_or(NO_ROW))
            .collect();

        let one_rows = one.len();
        let mut counts = vec![0u32; one_rows + 1];
        let mut unmatched = 0;
        for t in &targets {
            match counts.get_mut((*t as usize).saturating_add(1)) {
                Some(c) if *t != NO_ROW => *c += 1,
                _ => unmatched += 1,
            }
        }
        let mut running = 0u32;
        for c in &mut counts {
            running = running.saturating_add(*c);
            *c = running;
        }
        let offsets = counts;
        let mut cursor = offsets.clone();
        let mut sources = vec![0u32; targets.len() - unmatched];
        for (row, t) in targets.iter().enumerate() {
            if *t == NO_ROW {
                continue;
            }
            if let Some(pos) = cursor.get_mut(*t as usize) {
                if let Some(slot) = sources.get_mut(*pos as usize) {
                    *slot = u32::try_from(row).unwrap_or(NO_ROW);
                }
                *pos += 1;
            }
        }
        let unmatched_rows: Vec<u32> = targets
            .iter()
            .enumerate()
            .filter(|(_, t)| **t == NO_ROW)
            .map(|(row, _)| u32::try_from(row).unwrap_or(NO_ROW))
            .collect();
        let unreferenced_rows: Vec<u32> = offsets
            .windows(2)
            .enumerate()
            .filter(|(_, w)| w[0] == w[1])
            .map(|(row, _)| u32::try_from(row).unwrap_or(NO_ROW))
            .collect();
        let unreferenced = unreferenced_rows.len();
        let mut seen = vec![false; one_rows];
        let mut duplicate_many_keys = 0;
        for t in &targets {
            if let Some(s) = seen.get_mut(*t as usize) {
                if *s {
                    duplicate_many_keys += 1;
                }
                *s = true;
            }
        }
        Self {
            many_rows: many.len(),
            one_rows,
            targets,
            offsets,
            sources,
            unmatched,
            unmatched_rows,
            unreferenced,
            unreferenced_rows,
            duplicate_keys,
            duplicate_many_keys,
        }
    }

    pub fn many_rows(&self) -> usize {
        self.many_rows
    }

    pub fn one_rows(&self) -> usize {
        self.one_rows
    }

    /// The one-side row related to `many_row`; `None` = the blank row (or
    /// `many_row` out of range).
    pub fn target(&self, many_row: usize) -> Option<usize> {
        match self.targets.get(many_row) {
            Some(&t) if t != NO_ROW => Some(t as usize),
            _ => None,
        }
    }

    /// Many-side rows related to `one_row` (ascending).
    pub fn sources(&self, one_row: usize) -> &[u32] {
        let (Some(&start), Some(&end)) = (self.offsets.get(one_row), self.offsets.get(one_row + 1))
        else {
            return &[];
        };
        self.sources
            .get(start as usize..end as usize)
            .unwrap_or_default()
    }

    /// Many-side rows with no matching one-side row (they relate to the
    /// blank row).
    pub fn unmatched_count(&self) -> usize {
        self.unmatched
    }

    /// One-side rows no many-side row refers to.
    pub fn unreferenced_count(&self) -> usize {
        self.unreferenced
    }

    /// Extra one-side rows whose key duplicates an earlier row.
    pub fn duplicate_keys(&self) -> usize {
        self.duplicate_keys
    }

    /// Filter flowing one → many: many-side rows (`many_rows + 1` universe,
    /// last bit = many side's blank row) whose one-side row is in `one`
    /// (`one_rows + 1` universe, last bit = blank row).
    pub(crate) fn propagate_to_many(&self, one: &RowSet) -> RowSet {
        let mut out = RowSet::empty(self.many_rows + 1);
        let blank_visible = one.contains(self.one_rows);
        let selected = one.count() - usize::from(blank_visible);
        if selected.saturating_mul(SPARSE_RATIO) <= self.one_rows {
            // Few one-side rows: walk their many-side rows (CSR) instead of
            // every many-side row.
            for one_row in one.iter().take_while(|r| *r < self.one_rows) {
                for row in self.sources(one_row) {
                    out.insert(*row as usize);
                }
            }
            if blank_visible {
                for row in &self.unmatched_rows {
                    out.insert(*row as usize);
                }
                out.insert(self.many_rows);
            }
            return out;
        }
        for (row, t) in self.targets.iter().enumerate() {
            let visible = if *t == NO_ROW {
                blank_visible
            } else {
                one.contains(*t as usize)
            };
            if visible {
                out.insert(row);
            }
        }
        if blank_visible {
            out.insert(self.many_rows);
        }
        out
    }

    /// Filter flowing many → one (cross-filter): one-side rows (`one_rows + 1`
    /// universe) related to at least one row of `many`.
    pub(crate) fn propagate_to_one(&self, many: &RowSet) -> RowSet {
        let mut out = RowSet::empty(self.one_rows + 1);
        for row in many.iter() {
            match self.targets.get(row) {
                Some(&t) if t != NO_ROW => out.insert(t as usize),
                _ => out.insert(self.one_rows),
            };
        }
        out
    }

    /// [`Self::propagate_to_many`] of a one-to-one relationship, where the
    /// one-side rows no many-side row refers to relate to the many side's
    /// blank row: it is visible when one of them is.
    pub(crate) fn propagate_to_many_one_to_one(&self, one: &RowSet) -> RowSet {
        let mut out = self.propagate_to_many(one);
        if self
            .unreferenced_rows
            .iter()
            .any(|row| one.contains(*row as usize))
        {
            out.insert(self.many_rows);
        }
        out
    }

    /// [`Self::propagate_to_one`] of a one-to-one relationship: a visible
    /// many-side blank row keeps the one-side rows no many-side row refers
    /// to.
    pub(crate) fn propagate_to_one_one_to_one(&self, many: &RowSet) -> RowSet {
        let mut out = self.propagate_to_one(many);
        if many.contains(self.many_rows) {
            for row in &self.unreferenced_rows {
                out.insert(*row as usize);
            }
        }
        out
    }
}

/// Key-value index of a many-to-many relationship: every row of both sides
/// mapped to a shared key id (text case-insensitive, BLANK matching BLANK,
/// like [`RelationshipIndex`]).
#[derive(Debug, Clone)]
pub struct ManyToManyIndex {
    /// Rows of the `from` side (the [`RelationshipDef::many`] end).
    many_keys: Vec<u32>,
    /// Rows of the `to` side (the [`RelationshipDef::one`] end).
    one_keys: Vec<u32>,
    keys: usize,
    blank_key: Option<u32>,
    unmatched_many: usize,
    unmatched_one: usize,
}

impl ManyToManyIndex {
    /// Joins the two key columns by value.
    pub fn build(many: &EncodedColumn, one: &EncodedColumn) -> Self {
        let mut ids: HashMap<crate::value::ValueKey, u32> = HashMap::new();
        let mut map = |column: &EncodedColumn| -> Vec<u32> {
            column
                .values()
                .iter()
                .map(|v| {
                    let next = u32::try_from(ids.len()).unwrap_or(NO_ROW);
                    *ids.entry(v.key()).or_insert(next)
                })
                .collect()
        };
        let many_dict = map(many);
        let one_dict = map(one);
        let keys = ids.len();
        let blank_key = ids.get(&crate::value::ValueKey::Blank).copied();
        let rows = |column: &EncodedColumn, dict: &[u32]| -> Vec<u32> {
            column
                .ids()
                .iter()
                .map(|id| dict.get(*id as usize).copied().unwrap_or(NO_ROW))
                .collect()
        };
        let many_keys = rows(many, &many_dict);
        let one_keys = rows(one, &one_dict);
        let present = |keys_of: &[u32]| {
            let mut seen = vec![false; keys];
            for k in keys_of {
                if let Some(s) = seen.get_mut(*k as usize) {
                    *s = true;
                }
            }
            seen
        };
        let (in_many, in_one) = (present(&many_keys), present(&one_keys));
        let unmatched = |keys_of: &[u32], other: &[bool]| {
            keys_of
                .iter()
                .filter(|k| !other.get(**k as usize).copied().unwrap_or(false))
                .count()
        };
        Self {
            unmatched_many: unmatched(&many_keys, &in_one),
            unmatched_one: unmatched(&one_keys, &in_many),
            many_keys,
            one_keys,
            keys,
            blank_key,
        }
    }

    /// Rows of the `from` side.
    pub fn many_rows(&self) -> usize {
        self.many_keys.len()
    }

    /// Rows of the `to` side.
    pub fn one_rows(&self) -> usize {
        self.one_keys.len()
    }

    /// `(from-side rows, to-side rows)` whose key has no match on the other
    /// side.
    pub fn unmatched_counts(&self) -> (usize, usize) {
        (self.unmatched_many, self.unmatched_one)
    }

    /// Filter flowing `to` → `from`: `from` rows (`many_rows + 1` universe,
    /// last bit = BLANK passes) whose key is a key of a row in `one`
    /// (`one_rows + 1` universe, last bit = the `to` side's blank row, key
    /// BLANK).
    pub(crate) fn propagate_to_many(&self, one: &RowSet) -> RowSet {
        Self::project(
            &self.one_keys,
            one,
            &self.many_keys,
            self.keys,
            self.blank_key,
        )
    }

    /// Filter flowing `from` → `to` (see [`Self::propagate_to_many`]).
    pub(crate) fn propagate_to_one(&self, many: &RowSet) -> RowSet {
        Self::project(
            &self.many_keys,
            many,
            &self.one_keys,
            self.keys,
            self.blank_key,
        )
    }

    fn project(
        source_keys: &[u32],
        source: &RowSet,
        target_keys: &[u32],
        keys: usize,
        blank_key: Option<u32>,
    ) -> RowSet {
        let n = source_keys.len();
        let mut selected = vec![false; keys];
        for row in source.iter() {
            let key = if row < n {
                source_keys.get(row).copied()
            } else {
                blank_key
            };
            if let Some(slot) = key.and_then(|k| selected.get_mut(k as usize)) {
                *slot = true;
            }
        }
        let blank = source.contains(n)
            || blank_key.is_some_and(|b| selected.get(b as usize).copied().unwrap_or(false));
        let m = target_keys.len();
        let mut out = RowSet::empty(m + 1);
        for (row, key) in target_keys.iter().enumerate() {
            if selected.get(*key as usize).copied().unwrap_or(false) {
                out.insert(row);
            }
        }
        if blank {
            out.insert(m);
        }
        out
    }
}

/// Direction a filter flows over a relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Flow {
    /// From the one side to the many side (every active relationship).
    OneToMany,
    /// From the many side to the one side (bidirectional cross-filtering).
    ManyToOne,
}

/// A node of a [`PropagationTree`].
#[derive(Debug, Clone, PartialEq)]
pub struct TreeNode {
    /// Table name (stored spelling).
    pub table: String,
    /// `(parent node, relationship, flow from this node to the parent)`;
    /// `None` for the root.
    pub parent: Option<(usize, usize, Flow)>,
}

/// The tables whose filters reach a root table, as a tree rooted at that
/// table. Nodes are in settle order: a parent always precedes its children.
#[derive(Debug, Clone, PartialEq)]
pub struct PropagationTree {
    pub nodes: Vec<TreeNode>,
}

/// One hop of an expanded-table path (many → one chain).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hop {
    /// Relationship (index into [`RelationshipGraph::definitions`]).
    pub relationship: usize,
    /// `true` when walking from the many side to the one side (the normal
    /// direction); `false` only for the reverse walk of a one-to-one
    /// relationship.
    pub to_one: bool,
}

#[derive(Debug, Clone)]
struct RelState {
    index: Option<Arc<RelationshipIndex>>,
    many_to_many: Option<Arc<ManyToManyIndex>>,
    diagnostics: Vec<Diagnostic>,
}

impl RelState {
    fn failed(diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            index: None,
            many_to_many: None,
            diagnostics,
        }
    }

    fn is_usable(&self) -> bool {
        self.index.is_some() || self.many_to_many.is_some()
    }
}

/// Relationship indexes plus the propagation trees and expanded-table paths
/// derived from the active relationships.
#[derive(Debug, Clone, Default)]
pub struct RelationshipGraph {
    defs: Vec<RelationshipDef>,
    states: Vec<RelState>,
    def_diagnostics: Vec<Diagnostic>,
    graph_diagnostics: Vec<Diagnostic>,
    /// Usable relationships (active and indexed).
    active: Vec<usize>,
    trees: HashMap<String, Arc<PropagationTree>>,
    expanded: HashMap<String, Vec<(String, Vec<Hop>)>>,
    blank_rows: HashSet<String>,
    /// Relationships whose one → many flow is disabled (`CROSSFILTER` with
    /// only the many side filtering).
    reverse_only: HashSet<usize>,
}

impl RelationshipGraph {
    /// Normalised definitions of every model relationship (active or not).
    pub fn definitions_from_model(
        relationships: &[Relationship],
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Vec<RelationshipDef> {
        relationships
            .iter()
            .enumerate()
            .map(|(i, r)| RelationshipDef::from_model(i, r, diagnostics))
            .collect()
    }

    /// Builds indexes for every relationship (inactive ones too, for
    /// `USERELATIONSHIP`) and the propagation structures for the active
    /// ones.
    pub fn build(
        defs: Vec<RelationshipDef>,
        def_diagnostics: Vec<Diagnostic>,
        storage: &Storage,
    ) -> Self {
        let states = defs.iter().map(|d| build_state(d, storage)).collect();
        Self::assemble(defs, states, def_diagnostics, storage, HashSet::new())
    }

    /// Rebuilds after the data of `changed` tables changed, reusing the
    /// indexes of relationships that do not involve them.
    pub fn rebuild(&self, storage: &Storage, changed: &[&str]) -> Self {
        let states = self
            .defs
            .iter()
            .zip(&self.states)
            .map(|(d, s)| {
                let touched = changed
                    .iter()
                    .any(|t| names_eq(t, &d.many.table) || names_eq(t, &d.one.table));
                if touched {
                    build_state(d, storage)
                } else {
                    s.clone()
                }
            })
            .collect();
        Self::assemble(
            self.defs.clone(),
            states,
            self.def_diagnostics.clone(),
            storage,
            HashSet::new(),
        )
    }

    /// Rebuilds every relationship index from `storage`.
    pub fn rebuild_all(&self, storage: &Storage) -> Self {
        let states = self.defs.iter().map(|d| build_state(d, storage)).collect();
        Self::assemble(
            self.defs.clone(),
            states,
            self.def_diagnostics.clone(),
            storage,
            HashSet::new(),
        )
    }

    /// The graph with `overrides` (`USERELATIONSHIP` / `CROSSFILTER`)
    /// applied in order: same indexes, different active relationships and
    /// filter directions. Overrides naming unknown relationships are
    /// ignored. Diagnostics are those of `self`.
    pub fn with_overrides(&self, storage: &Storage, overrides: &[RelationshipOverride]) -> Self {
        if overrides.is_empty() {
            return self.clone();
        }
        let mut defs = self.defs.clone();
        let mut reverse_only = HashSet::new();
        for o in overrides {
            match *o {
                RelationshipOverride::Activate(i) => {
                    let Some(target) = defs.get(i).cloned() else {
                        continue;
                    };
                    for (j, d) in defs.iter_mut().enumerate() {
                        if j != i && same_tables(d, &target) {
                            d.is_active = false;
                        }
                    }
                    if let Some(d) = defs.get_mut(i) {
                        d.is_active = true;
                    }
                }
                RelationshipOverride::CrossFilter(i, direction) => {
                    let Some(d) = defs.get_mut(i) else { continue };
                    reverse_only.remove(&i);
                    match direction {
                        CrossFilterDirection::None => d.is_active = false,
                        CrossFilterDirection::OneWay => d.bidirectional = false,
                        CrossFilterDirection::Both => d.bidirectional = true,
                        CrossFilterDirection::ManyToOne => {
                            d.bidirectional = true;
                            reverse_only.insert(i);
                        }
                    }
                }
            }
        }
        let mut graph = Self::assemble(
            defs,
            self.states.clone(),
            self.def_diagnostics.clone(),
            storage,
            reverse_only,
        );
        graph.graph_diagnostics = self.graph_diagnostics.clone();
        graph
    }

    /// The graph row-level security filters propagate on: the same
    /// relationships and indexes, with directions from
    /// `securityFilteringBehavior` (`oneDirection`: one → many, and `to` →
    /// `from` for many-to-many; `bothDirections`: both ways; `none`: the
    /// relationship is not crossed).
    pub fn for_security(&self, storage: &Storage) -> Self {
        let mut defs = self.defs.clone();
        for d in &mut defs {
            match d.security {
                SecurityFilteringBehavior::BothDirections => d.bidirectional = true,
                SecurityFilteringBehavior::None => d.is_active = false,
                SecurityFilteringBehavior::OneDirection | SecurityFilteringBehavior::Unknown(_) => {
                    d.bidirectional = false;
                }
            }
        }
        let mut graph = Self::assemble(
            defs,
            self.states.clone(),
            self.def_diagnostics.clone(),
            storage,
            HashSet::new(),
        );
        graph.graph_diagnostics = self.graph_diagnostics.clone();
        graph
    }

    /// Index (into [`RelationshipGraph::definitions`]) of the relationship
    /// joining `table1[column1]` and `table2[column2]` (either order,
    /// case-insensitive, active or not).
    pub fn find_relationship(
        &self,
        table1: &str,
        column1: &str,
        table2: &str,
        column2: &str,
    ) -> Option<usize> {
        self.defs.iter().position(|d| {
            (d.many.matches(table1, column1) && d.one.matches(table2, column2))
                || (d.many.matches(table2, column2) && d.one.matches(table1, column1))
        })
    }

    fn assemble(
        defs: Vec<RelationshipDef>,
        states: Vec<RelState>,
        def_diagnostics: Vec<Diagnostic>,
        storage: &Storage,
        reverse_only: HashSet<usize>,
    ) -> Self {
        let mut graph_diagnostics = Vec::new();
        let mut active = Vec::new();
        let mut pairs: HashMap<(String, String), usize> = HashMap::new();
        for (i, (d, s)) in defs.iter().zip(&states).enumerate() {
            if !d.is_active || !s.is_usable() {
                continue;
            }
            let (a, b) = (name_key(&d.many.table), name_key(&d.one.table));
            let pair = if a <= b { (a, b) } else { (b, a) };
            if let Some(&first) = pairs.get(&pair) {
                graph_diagnostics.push(
                    Diagnostic::warning(
                        "ENG004",
                        format!(
                            "relationship {} is a second active relationship between the same tables as {}; ignored",
                            d.label(),
                            defs.get(first).map(RelationshipDef::label).unwrap_or_default()
                        ),
                    )
                    .with_object(d.id),
                );
                continue;
            }
            pairs.insert(pair, i);
            active.push(i);
        }

        let mut blank_rows = HashSet::new();
        for &i in &active {
            let (Some(d), Some(index)) =
                (defs.get(i), states.get(i).and_then(|s| s.index.as_ref()))
            else {
                continue;
            };
            if index.unmatched_count() > 0 {
                blank_rows.insert(name_key(&d.one.table));
            }
            if d.kind == RelationshipKind::OneToOne && index.unreferenced_count() > 0 {
                blank_rows.insert(name_key(&d.many.table));
            }
        }

        let mut graph = Self {
            defs,
            states,
            def_diagnostics,
            graph_diagnostics,
            active,
            trees: HashMap::new(),
            expanded: HashMap::new(),
            blank_rows,
            reverse_only,
        };
        let mut seen_ambiguities = HashSet::new();
        let mut table_names: Vec<String> = storage.table_names().map(str::to_owned).collect();
        for d in &graph.defs {
            for t in [&d.many.table, &d.one.table] {
                if !table_names.iter().any(|n| names_eq(n, t)) {
                    table_names.push(t.clone());
                }
            }
        }
        for table in &table_names {
            let tree = graph.build_tree(table, &mut seen_ambiguities);
            graph.trees.insert(name_key(table), Arc::new(tree));
            let expanded = graph.build_expanded(table);
            graph.expanded.insert(name_key(table), expanded);
        }
        graph
    }

    /// Filter-flow edges into `table`: `(relationship, source table, flow)`.
    fn edges_into(&self, table: &str) -> Vec<(usize, &str, Flow)> {
        let mut out = Vec::new();
        for &i in &self.active {
            let Some(d) = self.defs.get(i) else { continue };
            if names_eq(&d.many.table, &d.one.table) {
                continue;
            }
            if names_eq(&d.many.table, table) && !self.reverse_only.contains(&i) {
                out.push((i, d.one.table.as_str(), Flow::OneToMany));
            }
            if d.bidirectional && names_eq(&d.one.table, table) {
                out.push((i, d.many.table.as_str(), Flow::ManyToOne));
            }
        }
        out
    }

    fn build_tree(&mut self, root: &str, seen: &mut HashSet<(String, String)>) -> PropagationTree {
        struct Candidate {
            table: String,
            parent: usize,
            rel: usize,
            flow: Flow,
        }
        let mut nodes = vec![TreeNode {
            table: root.to_owned(),
            parent: None,
        }];
        // settled table key → (cost, relationship used); cost = (reverse
        // hops, many-to-many hops, hops).
        let mut settled: HashMap<String, (Cost, Option<usize>)> = HashMap::new();
        settled.insert(name_key(root), ((0, 0, 0), None));
        let mut candidates: Vec<Candidate> = Vec::new();
        let mut heap = BinaryHeap::new();
        let push = |heap: &mut BinaryHeap<Reverse<(Cost, usize)>>,
                    candidates: &mut Vec<Candidate>,
                    graph: &Self,
                    parent: usize,
                    table: &str,
                    cost: Cost,
                    skip_rel: Option<usize>| {
            for (rel, source, flow) in graph.edges_into(table) {
                if Some(rel) == skip_rel {
                    continue;
                }
                let m2m = graph
                    .defs
                    .get(rel)
                    .is_some_and(|d| d.kind == RelationshipKind::ManyToMany);
                let reverse = u32::from(flow == Flow::ManyToOne && !m2m);
                let next = (cost.0 + reverse, cost.1 + u32::from(m2m), cost.2 + 1);
                heap.push(Reverse((next, candidates.len())));
                candidates.push(Candidate {
                    table: source.to_owned(),
                    parent,
                    rel,
                    flow,
                });
            }
        };
        push(&mut heap, &mut candidates, self, 0, root, (0, 0, 0), None);
        while let Some(Reverse((cost, ci))) = heap.pop() {
            let Some(c) = candidates.get(ci) else {
                continue;
            };
            let key = name_key(&c.table);
            if let Some((settled_cost, settled_rel)) = settled.get(&key) {
                if *settled_cost == cost && *settled_rel != Some(c.rel) && settled_rel.is_some() {
                    let pair = (key.clone(), name_key(root));
                    if seen.insert(pair) {
                        let used = settled_rel
                            .and_then(|r| self.defs.get(r))
                            .map(RelationshipDef::label)
                            .unwrap_or_default();
                        let other = self.defs.get(c.rel);
                        let mut diag = Diagnostic::warning(
                            "ENG004",
                            format!(
                                "ambiguous filter paths from '{}' to '{}'; using the path through {used}, ignoring {}",
                                c.table,
                                root,
                                other.map(RelationshipDef::label).unwrap_or_default()
                            ),
                        );
                        if let Some(o) = other {
                            diag = diag.with_object(o.id);
                        }
                        self.graph_diagnostics.push(diag);
                    }
                }
                continue;
            }
            settled.insert(key, (cost, Some(c.rel)));
            let node = nodes.len();
            nodes.push(TreeNode {
                table: c.table.clone(),
                parent: Some((c.parent, c.rel, c.flow)),
            });
            let (table, rel) = (c.table.clone(), c.rel);
            push(
                &mut heap,
                &mut candidates,
                self,
                node,
                &table,
                cost,
                Some(rel),
            );
        }
        PropagationTree { nodes }
    }

    /// Tables of `table`'s expanded table (reachable over many → one hops,
    /// both ways for one-to-one), with their paths.
    fn build_expanded(&self, table: &str) -> Vec<(String, Vec<Hop>)> {
        let mut out: Vec<(String, Vec<Hop>)> = Vec::new();
        let mut visited: HashSet<String> = HashSet::from([name_key(table)]);
        let mut queue: VecDeque<(String, Vec<Hop>)> =
            VecDeque::from([(table.to_owned(), Vec::new())]);
        while let Some((current, path)) = queue.pop_front() {
            for &i in &self.active {
                let Some(d) = self.defs.get(i) else { continue };
                if d.kind == RelationshipKind::ManyToMany {
                    continue;
                }
                let step = if names_eq(&d.many.table, &current) {
                    Some((&d.one.table, true))
                } else if d.kind == RelationshipKind::OneToOne && names_eq(&d.one.table, &current) {
                    Some((&d.many.table, false))
                } else {
                    None
                };
                let Some((next, to_one)) = step else { continue };
                if !visited.insert(name_key(next)) {
                    continue;
                }
                let mut next_path = path.clone();
                next_path.push(Hop {
                    relationship: i,
                    to_one,
                });
                out.push((next.clone(), next_path.clone()));
                queue.push_back((next.clone(), next_path));
            }
        }
        out
    }

    /// All relationship definitions (index = position in the model).
    pub fn definitions(&self) -> &[RelationshipDef] {
        &self.defs
    }

    /// Index of relationship `i`, if it could be built (many-to-one and
    /// one-to-one relationships).
    pub fn index(&self, i: usize) -> Option<&Arc<RelationshipIndex>> {
        self.states.get(i)?.index.as_ref()
    }

    /// Key-value index of many-to-many relationship `i`, if it could be
    /// built.
    pub fn many_to_many_index(&self, i: usize) -> Option<&Arc<ManyToManyIndex>> {
        self.states.get(i)?.many_to_many.as_ref()
    }

    /// Projects visible rows across relationship `rel` in direction `flow`
    /// (`n + 1` universes, last bit = blank row); `None` if it has no
    /// index.
    pub(crate) fn project(&self, rel: usize, flow: Flow, rows: &RowSet) -> Option<RowSet> {
        let state = self.states.get(rel)?;
        if let Some(index) = &state.index {
            return Some(self.propagate(rel, index, flow == Flow::ManyToOne, rows));
        }
        let index = state.many_to_many.as_ref()?;
        Some(match flow {
            Flow::OneToMany => index.propagate_to_many(rows),
            Flow::ManyToOne => index.propagate_to_one(rows),
        })
    }

    /// `true` if an active many-to-many relationship joins `a` and `b`.
    pub fn many_to_many_between(&self, a: &str, b: &str) -> bool {
        self.active.iter().any(|&i| {
            self.defs.get(i).is_some_and(|d| {
                d.kind == RelationshipKind::ManyToMany
                    && ((names_eq(&d.many.table, a) && names_eq(&d.one.table, b))
                        || (names_eq(&d.many.table, b) && names_eq(&d.one.table, a)))
            })
        })
    }

    /// Relationships used for filter propagation (active, indexed and
    /// supported).
    pub fn active_relationships(&self) -> &[usize] {
        &self.active
    }

    /// Precomputed propagation tree of `table`.
    pub fn tree(&self, table: &str) -> Option<&Arc<PropagationTree>> {
        self.trees.get(&name_key(table))
    }

    /// `true` if `table` has a blank row (some active relationship has
    /// many-side keys without a match in `table`).
    pub fn has_blank_row(&self, table: &str) -> bool {
        self.blank_rows.contains(&name_key(table))
    }

    /// Tables in `table`'s expanded table (excluding itself) with the hop
    /// path to each.
    pub fn expanded_tables(&self, table: &str) -> &[(String, Vec<Hop>)] {
        self.expanded
            .get(&name_key(table))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Many → one hop path from `from` to `to` (`RELATED` direction), or
    /// `None` if `to` is not in `from`'s expanded table. An empty path means
    /// `from == to`.
    pub fn expanded_path(&self, from: &str, to: &str) -> Option<&[Hop]> {
        if names_eq(from, to) {
            return Some(&[]);
        }
        self.expanded_tables(from)
            .iter()
            .find(|(t, _)| names_eq(t, to))
            .map(|(_, p)| p.as_slice())
    }

    /// `true` when filters on `to` reach `from` exactly along the reverse of
    /// [`RelationshipGraph::expanded_path`]`(from, to)`: every hop of that
    /// path is many → one, and in `from`'s propagation tree `to` reaches the
    /// root through the same relationships, flowing one → many. Then the
    /// rows of `from` a filter on `to` leaves visible are exactly the rows
    /// whose [`RelationshipGraph::follow`] row (or blank row) passes it.
    /// `true` for `from == to`.
    pub fn filters_follow_expanded_path(&self, from: &str, to: &str) -> bool {
        let Some(path) = self.expanded_path(from, to) else {
            return false;
        };
        if path.is_empty() {
            return true;
        }
        if !path.iter().all(|h| h.to_one) {
            return false;
        }
        let Some(tree) = self.tree(from) else {
            return false;
        };
        let Some(mut node) = tree.nodes.iter().position(|n| names_eq(&n.table, to)) else {
            return false;
        };
        for hop in path.iter().rev() {
            match tree.nodes.get(node).and_then(|n| n.parent) {
                Some((parent, rel, Flow::OneToMany)) if rel == hop.relationship => node = parent,
                _ => return false,
            }
        }
        node == 0
    }

    /// The rows of `rows` (rows of `from`) that a filter on `to` can keep:
    /// rows whose many → one path to `to` ([`RelationshipGraph::follow`])
    /// ends on a real row, or ends on `to`'s blank row through blank rows
    /// that exist. A row relating to the blank row of a table on the path
    /// while a later table of the path has no blank row is hidden by every
    /// filter on `to` (propagation drops the blank row of a table that has
    /// none). All of `rows` when `to` is not in `from`'s expanded table.
    pub fn rows_reachable_from(&self, from: &str, to: &str, rows: &RowSet) -> RowSet {
        let Some(path) = self.expanded_path(from, to) else {
            return rows.clone();
        };
        // `blank_tail[m]`: every table reached by hops `m..` has a blank row.
        let mut blank_tail = vec![true; path.len() + 1];
        for (m, hop) in path.iter().enumerate().rev() {
            let has_blank = self.defs.get(hop.relationship).is_some_and(|d| {
                let dest = if hop.to_one {
                    &d.one.table
                } else {
                    &d.many.table
                };
                self.has_blank_row(dest)
            });
            let next = blank_tail.get(m + 1).copied().unwrap_or(true);
            if let Some(slot) = blank_tail.get_mut(m) {
                *slot = has_blank && next;
            }
        }
        if blank_tail.iter().all(|b| *b) {
            return rows.clone();
        }
        let mut out = rows.clone();
        for row in rows.iter() {
            let mut current = row;
            for (m, hop) in path.iter().enumerate() {
                let next = self.index(hop.relationship).and_then(|index| {
                    if hop.to_one {
                        index.target(current)
                    } else {
                        index.sources(current).first().map(|r| *r as usize)
                    }
                });
                match next {
                    Some(r) => current = r,
                    None => {
                        if !blank_tail.get(m).copied().unwrap_or(true) {
                            out.remove(row);
                        }
                        break;
                    }
                }
            }
        }
        out
    }

    /// Follows a hop path from `row`; `None` = a blank row was reached.
    pub fn follow(&self, path: &[Hop], row: usize) -> Option<usize> {
        let mut current = row;
        for hop in path {
            let index = self.index(hop.relationship)?;
            current = if hop.to_one {
                index.target(current)?
            } else {
                *index.sources(current).first()? as usize
            };
        }
        Some(current)
    }

    /// `rows` of one side of relationship `rel` mapped to the other side
    /// (to the one side if `to_one`), honouring one-to-one blank rows.
    fn propagate(
        &self,
        rel: usize,
        index: &RelationshipIndex,
        to_one: bool,
        rows: &RowSet,
    ) -> RowSet {
        let one_to_one = self
            .defs
            .get(rel)
            .is_some_and(|d| d.kind == RelationshipKind::OneToOne);
        match (to_one, one_to_one) {
            (true, false) => index.propagate_to_one(rows),
            (true, true) => index.propagate_to_one_one_to_one(rows),
            (false, false) => index.propagate_to_many(rows),
            (false, true) => index.propagate_to_many_one_to_one(rows),
        }
    }

    /// Maps a row set of the path's start table (`n + 1` universe incl. its
    /// blank row) to the path's end table (`m + 1` universe).
    pub(crate) fn image(&self, path: &[Hop], rows: &RowSet, end_rows: usize) -> RowSet {
        let mut current = rows.clone();
        for hop in path {
            let Some(index) = self.index(hop.relationship) else {
                return RowSet::empty(end_rows + 1);
            };
            current = self.propagate(hop.relationship, index, hop.to_one, &current);
            if let Some(d) = self.defs.get(hop.relationship) {
                let dest = if hop.to_one {
                    &d.one.table
                } else {
                    &d.many.table
                };
                if !self.has_blank_row(dest) {
                    let n = current.universe().saturating_sub(1);
                    current.remove(n);
                }
            }
        }
        current.resized(end_rows + 1)
    }

    /// All diagnostics: relationship normalisation, missing tables/columns,
    /// duplicate keys, type mismatches, ambiguous paths.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        let mut out = self.def_diagnostics.clone();
        for s in &self.states {
            out.extend(s.diagnostics.iter().cloned());
        }
        out.extend(self.graph_diagnostics.iter().cloned());
        out
    }
}

/// `a` and `b` join the same two tables.
fn same_tables(a: &RelationshipDef, b: &RelationshipDef) -> bool {
    (names_eq(&a.many.table, &b.many.table) && names_eq(&a.one.table, &b.one.table))
        || (names_eq(&a.many.table, &b.one.table) && names_eq(&a.one.table, &b.many.table))
}

fn build_state(d: &RelationshipDef, storage: &Storage) -> RelState {
    let mut diagnostics = Vec::new();
    let label = d.label();
    if names_eq(&d.many.table, &d.one.table) {
        diagnostics.push(
            Diagnostic::warning(
                "ENG007",
                format!("relationship {label}: self-referencing relationships are not supported; ignored"),
            )
            .with_object(d.id),
        );
        return RelState::failed(diagnostics);
    }
    let column = |c: &ColumnRef| {
        storage
            .get(&c.table)
            .filter(|t| t.has_column(&c.column))
            .map(Arc::clone)
    };
    let (Some(many), Some(one)) = (column(&d.many), column(&d.one)) else {
        let missing = [&d.many, &d.one]
            .into_iter()
            .filter(|c| column(c).is_none())
            .map(|c| pbi_model::dax_column_ref(&c.table, &c.column))
            .collect::<Vec<_>>()
            .join(", ");
        diagnostics.push(
            Diagnostic::warning(
                "ENG001",
                format!("relationship {label}: column(s) not loaded: {missing}; ignored"),
            )
            .with_object(d.id),
        );
        return RelState::failed(diagnostics);
    };
    let encoded = many
        .encoded(&d.many.column)
        .and_then(|m| one.encoded(&d.one.column).map(|o| (m, o)));
    let (many_enc, one_enc) = match encoded {
        Ok(pair) => pair,
        Err(err) => {
            diagnostics.push(
                Diagnostic::warning(
                    "ENG008",
                    format!("relationship {label}: key column cannot be indexed: {err}; ignored"),
                )
                .with_object(d.id),
            );
            return RelState::failed(diagnostics);
        }
    };
    let types = (
        many.column_type(&d.many.column)
            .ok()
            .and_then(canonical_type),
        one.column_type(&d.one.column).ok().and_then(canonical_type),
    );
    if let (Some(a), Some(b)) = &types
        && a != b
        && many.row_count() > 0
        && one.row_count() > 0
    {
        diagnostics.push(
            Diagnostic::warning(
                "ENG005",
                format!("relationship {label}: key types differ ({a} vs {b}); keys match by value"),
            )
            .with_object(d.id),
        );
    }
    if d.kind == RelationshipKind::ManyToMany {
        let index = ManyToManyIndex::build(&many_enc, &one_enc);
        diagnostics.push(
            Diagnostic::info(
                "ENG003",
                format!(
                    "relationship {label}: many-to-many (limited) relationship: filters cross it as key values, no blank row is added and RELATED cannot follow it"
                ),
            )
            .with_object(d.id),
        );
        return RelState {
            index: None,
            many_to_many: Some(Arc::new(index)),
            diagnostics,
        };
    }
    let index = RelationshipIndex::build(&many_enc, &one_enc);
    if index.duplicate_keys() > 0 {
        diagnostics.push(
            Diagnostic::error(
                "ENG002",
                format!(
                    "relationship {label}: {} duplicate key value(s) on the one side ({}); the first row with each key is used",
                    index.duplicate_keys(),
                    pbi_model::dax_column_ref(&d.one.table, &d.one.column)
                ),
            )
            .with_object(d.id),
        );
    }
    if d.kind == RelationshipKind::OneToOne && index.duplicate_many_keys > 0 {
        diagnostics.push(
            Diagnostic::error(
                "ENG002",
                format!(
                    "relationship {label}: one-to-one relationship has {} duplicate key value(s) in {}",
                    index.duplicate_many_keys,
                    pbi_model::dax_column_ref(&d.many.table, &d.many.column)
                ),
            )
            .with_object(d.id),
        );
    }
    RelState {
        index: Some(Arc::new(index)),
        many_to_many: None,
        diagnostics,
    }
}

/// Path cost: (reverse many → one hops, many-to-many hops, hops).
type Cost = (u32, u32, u32);

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{ArrayRef, StringArray};
    use arrow::record_batch::RecordBatch;

    fn keys(values: &[Option<&str>]) -> RecordBatch {
        RecordBatch::try_from_iter([(
            "K",
            Arc::new(StringArray::from(values.to_vec())) as ArrayRef,
        )])
        .unwrap()
    }

    fn rel(from: &str, to: &str) -> Relationship {
        Relationship::new("r", ColumnRef::new(from, "K"), ColumnRef::new(to, "K"))
    }

    fn graph(rels: &[Relationship], tables: &[(&str, &[Option<&str>])]) -> RelationshipGraph {
        let mut storage = Storage::new();
        for (name, values) in tables {
            storage.load_table(name, keys(values)).unwrap();
        }
        let mut diags = Vec::new();
        let defs = RelationshipGraph::definitions_from_model(rels, &mut diags);
        RelationshipGraph::build(defs, diags, &storage)
    }

    #[test]
    fn index_maps_many_rows_to_one_rows_case_insensitively() {
        let g = graph(
            &[rel("M", "O")],
            &[
                ("M", &[Some("a"), Some("B"), Some("zz"), None, Some("b")]),
                ("O", &[Some("A"), Some("b"), Some("c")]),
            ],
        );
        let ix = g.index(0).unwrap();
        assert_eq!(
            (0..5).map(|r| ix.target(r)).collect::<Vec<_>>(),
            [Some(0), Some(1), None, None, Some(1)]
        );
        assert_eq!(ix.sources(1), &[1, 4]);
        assert!(ix.sources(2).is_empty());
        assert!(ix.sources(99).is_empty());
        assert_eq!(ix.unmatched_count(), 2);
        assert!(g.has_blank_row("o"));
        assert!(!g.has_blank_row("M"));
        assert!(g.diagnostics().is_empty());
    }

    #[test]
    fn duplicate_one_side_keys_are_an_error_diagnostic() {
        let g = graph(
            &[rel("M", "O")],
            &[("M", &[Some("a")]), ("O", &[Some("a"), Some("A")])],
        );
        let diags = g.diagnostics();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "ENG002");
        assert_eq!(diags[0].severity, pbi_model::Severity::Error);
        assert_eq!(g.index(0).unwrap().target(0), Some(0));
    }

    #[test]
    fn missing_tables_are_ignored_and_many_to_many_is_limited() {
        let mut m2m = rel("M", "O");
        m2m.from_cardinality = Cardinality::Many;
        m2m.to_cardinality = Cardinality::Many;
        let g = graph(
            &[rel("M", "Nope"), m2m],
            &[
                ("M", &[Some("a"), Some("A"), Some("z")]),
                ("O", &[Some("a"), Some("b"), Some("b")]),
            ],
        );
        let diags = g.diagnostics();
        let codes: Vec<_> = diags.iter().map(|d| d.code.as_str()).collect();
        assert_eq!(codes, ["ENG001", "ENG003"]);
        assert_eq!(diags[1].severity, pbi_model::Severity::Info);
        assert_eq!(g.active_relationships(), [1]);
        // `to` (O) filters `from` (M); nothing flows back, nothing expands.
        assert_eq!(g.tree("M").unwrap().nodes.len(), 2);
        assert_eq!(g.tree("O").unwrap().nodes.len(), 1);
        assert!(g.expanded_path("M", "O").is_none());
        assert!(!g.has_blank_row("O") && !g.has_blank_row("M"));
        assert!(g.many_to_many_between("o", "m"));
        let ix = g.many_to_many_index(1).unwrap();
        assert_eq!(ix.unmatched_counts(), (1, 2));
        // O row 0 ("a") keeps M rows 0 and 1 (case-insensitive); the blank
        // bits stay clear (no BLANK key selected).
        let to_many = ix.propagate_to_many(&RowSet::from_rows(4, [0]));
        assert_eq!(to_many.iter().collect::<Vec<_>>(), [0, 1]);
        let to_one = ix.propagate_to_one(&RowSet::from_rows(4, [2]));
        assert!(to_one.is_empty());
    }

    #[test]
    fn trees_follow_filter_direction_transitively() {
        // A (many) → B (one) → C (one): filters on C reach A through B.
        let g = graph(
            &[rel("A", "B"), rel("B", "C")],
            &[
                ("A", &[Some("x")]),
                ("B", &[Some("x")]),
                ("C", &[Some("x")]),
            ],
        );
        let tables: Vec<_> = g
            .tree("A")
            .unwrap()
            .nodes
            .iter()
            .map(|n| n.table.clone())
            .collect();
        assert_eq!(tables, ["A", "B", "C"]);
        // Single direction: nothing reaches C.
        assert_eq!(g.tree("C").unwrap().nodes.len(), 1);
        assert_eq!(g.expanded_path("A", "C").map(<[Hop]>::len), Some(2));
        assert!(g.expanded_path("C", "A").is_none());
    }

    #[test]
    fn ambiguous_paths_are_diagnosed() {
        // Diamond: D → B → A and D → C → A (A is the shared one side).
        let g = graph(
            &[rel("D", "B"), rel("D", "C"), rel("B", "A"), rel("C", "A")],
            &[
                ("A", &[Some("x")]),
                ("B", &[Some("x")]),
                ("C", &[Some("x")]),
                ("D", &[Some("x")]),
            ],
        );
        assert!(g.diagnostics().iter().any(|d| d.code == "ENG004"));
        let tree = g.tree("D").unwrap();
        assert_eq!(tree.nodes.len(), 4);
    }
}
