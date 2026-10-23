//! Refresh planning: which step loads each table, and the dependency order
//! of calculated columns and calculated tables.

use std::collections::{BTreeSet, HashMap};

use pbi_dax::Dependency;
use pbi_model::{
    ColumnKind, Model, PartitionMode, PartitionSource, Table, TableKind, dax_column_ref,
    dax_table_ref, names_eq,
};

/// A refresh step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Node {
    /// The data of table `i` (M, PBIX, manual) or its DAX expression
    /// (calculated table).
    Table(usize),
    /// Calculated column `j` (index in `Table::columns`) of table `i`.
    Column(usize, usize),
}

/// How a table gets its data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    /// M partition expressions (import mode).
    M(Vec<String>),
    /// Embedded VertiPaq data.
    Pbix,
    /// The PBIP local data cache (`.pbi/cache.abf`).
    Cache,
    /// Expected in the local data cache but not found there; the message.
    NotCached(String),
    /// DAX calculated table.
    Calculated,
    /// Calculation-group table: one row per calculation item, built from the
    /// model.
    CalculationGroup,
    /// Cannot be loaded offline; the reason.
    Skip(String),
}

/// The refresh graph of a model.
#[derive(Debug)]
pub(crate) struct Plan {
    pub nodes: Vec<Node>,
    /// Direct dependencies of each node (indices into `nodes`).
    pub deps: Vec<Vec<usize>>,
    /// Topological order (indices into `nodes`); nodes on a cycle are not
    /// in it.
    pub order: Vec<usize>,
    /// Nodes on (or behind) a dependency cycle.
    pub cyclic: Vec<usize>,
    /// Source of each model table.
    pub sources: Vec<Source>,
    positions: HashMap<Node, usize>,
}

/// The data source of `table`.
pub(crate) fn table_source(table: &Table, pbix: bool) -> Source {
    let kind = table.kind();
    if kind == TableKind::Remote {
        return Source::Skip(
            "DirectQuery, Direct Lake and entity partitions are not available offline".into(),
        );
    }
    if pbix {
        return Source::Pbix;
    }
    match kind {
        TableKind::CalculatedTable => Source::Calculated,
        TableKind::CalculationGroup => Source::CalculationGroup,
        TableKind::Regular | TableKind::Remote => {
            let exprs: Vec<String> = table
                .partitions
                .iter()
                .filter(|p| {
                    !matches!(
                        p.mode,
                        PartitionMode::DirectQuery | PartitionMode::DirectLake
                    )
                })
                .filter_map(|p| match &p.source {
                    PartitionSource::M { expression } => Some(expression.clone()),
                    _ => None,
                })
                .collect();
            if !exprs.is_empty() {
                return Source::M(exprs);
            }
            match table.partitions.first() {
                None => Source::Skip("the table has no partition".into()),
                Some(p) => Source::Skip(format!(
                    "partitions of type '{}' cannot be evaluated offline",
                    p.source.kind_name()
                )),
            }
        }
    }
}

impl Plan {
    /// Builds the graph. With `pbix`, calculated columns and tables are
    /// stored data and get no DAX step.
    pub fn build(model: &Model, pbix: bool) -> Plan {
        let sources: Vec<Source> = model.tables.iter().map(|t| table_source(t, pbix)).collect();
        Self::build_with(model, sources, |_, _| !pbix)
    }

    /// Builds the graph from the given table sources; calculated column `j`
    /// of table `i` gets a DAX step when `column_step(i, j)` (otherwise its
    /// values come with the table data).
    pub fn build_with(
        model: &Model,
        sources: Vec<Source>,
        column_step: impl Fn(usize, usize) -> bool,
    ) -> Plan {
        let mut nodes = Vec::new();
        for (i, t) in model.tables.iter().enumerate() {
            nodes.push(Node::Table(i));
            for (j, c) in t.columns.iter().enumerate() {
                if c.is_calculated() && column_step(i, j) {
                    nodes.push(Node::Column(i, j));
                }
            }
        }
        let positions: HashMap<Node, usize> =
            nodes.iter().enumerate().map(|(i, n)| (*n, i)).collect();
        let index_of = |n: Node| positions.get(&n).copied();
        let mut deps: Vec<Vec<usize>> = Vec::with_capacity(nodes.len());
        for &node in &nodes {
            let mut set = BTreeSet::new();
            let (home, expr) = match node {
                Node::Table(i) => (
                    i,
                    match sources.get(i) {
                        Some(Source::Calculated) => {
                            model.tables.get(i).and_then(Table::calculated_expression)
                        }
                        _ => None,
                    },
                ),
                Node::Column(i, j) => {
                    set.insert(Node::Table(i));
                    (
                        i,
                        model
                            .tables
                            .get(i)
                            .and_then(|t| t.columns.get(j))
                            .and_then(|c| c.expression()),
                    )
                }
            };
            if let Some(expr) = expr {
                let home_name = model.tables.get(home).map(|t| t.name.as_str());
                let found =
                    pbi_dax::transitive_dependencies(model, home_name, expr).unwrap_or_default();
                for dep in found {
                    for n in dependency_nodes(model, &dep, home, node) {
                        // A calculated column without a step is stored data.
                        let n = match n {
                            Node::Column(k, _) if !positions.contains_key(&n) => Node::Table(k),
                            n => n,
                        };
                        set.insert(n);
                    }
                }
            }
            set.remove(&node);
            deps.push(set.into_iter().filter_map(index_of).collect());
        }
        let (order, cyclic) = topo_order(&deps);
        Plan {
            nodes,
            deps,
            order,
            cyclic,
            sources,
            positions,
        }
    }

    /// Index of `node`.
    pub fn index(&self, node: Node) -> Option<usize> {
        self.positions.get(&node).copied()
    }

    /// `affected[n]`: node `n` must run when `seeds` (node indices) run.
    /// Every node downstream of a seed is affected.
    pub fn affected(&self, seeds: &[usize]) -> Vec<bool> {
        let mut affected = vec![false; self.nodes.len()];
        for &s in seeds {
            if let Some(a) = affected.get_mut(s) {
                *a = true;
            }
        }
        // Fixed point (covers cyclic nodes too); the topological order makes
        // it converge in one pass for acyclic graphs.
        loop {
            let mut changed = false;
            for n in self.order.iter().chain(&self.cyclic).copied() {
                let hit = self
                    .deps
                    .get(n)
                    .is_some_and(|d| d.iter().any(|&x| affected.get(x).copied().unwrap_or(false)));
                if hit && let Some(a) = affected.get_mut(n).filter(|a| !**a) {
                    *a = true;
                    changed = true;
                }
            }
            if !changed {
                return affected;
            }
        }
    }
}

/// Graph nodes a DAX dependency of `home_node` (in table `home`) maps to.
fn dependency_nodes(model: &Model, dep: &Dependency, home: usize, home_node: Node) -> Vec<Node> {
    let table_index = |name: &str| model.tables.iter().position(|t| names_eq(&t.name, name));
    match dep {
        Dependency::Column { table, column } => {
            let Some(k) = table_index(table) else {
                return Vec::new();
            };
            let calc = model.tables.get(k).and_then(|t| {
                t.columns
                    .iter()
                    .position(|c| c.is_calculated() && names_eq(&c.name, column))
            });
            match calc {
                Some(m) => vec![Node::Column(k, m)],
                None => vec![Node::Table(k)],
            }
        }
        Dependency::Table(name) => {
            let Some(k) = table_index(name) else {
                return Vec::new();
            };
            let mut out = vec![Node::Table(k)];
            // A whole-table reference from another table needs every column
            // of that table; within the same table only its stored data
            // (otherwise sibling calculated columns would form a cycle).
            if k != home
                && home_node != Node::Table(k)
                && let Some(t) = model.tables.get(k)
            {
                out.extend(
                    t.columns
                        .iter()
                        .enumerate()
                        .filter(|(_, c)| matches!(c.kind, ColumnKind::Calculated { .. }))
                        .map(|(m, _)| Node::Column(k, m)),
                );
            }
            out
        }
        Dependency::Measure(_) => Vec::new(),
    }
}

/// Kahn's algorithm, preferring lower node indices (model order). Returns
/// the order and the nodes left on cycles.
fn topo_order(deps: &[Vec<usize>]) -> (Vec<usize>, Vec<usize>) {
    let n = deps.len();
    let mut indegree: Vec<usize> = deps.iter().map(Vec::len).collect();
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (node, ds) in deps.iter().enumerate() {
        for &d in ds {
            if let Some(list) = dependents.get_mut(d) {
                list.push(node);
            }
        }
    }
    let mut ready: BTreeSet<usize> = (0..n).filter(|&i| indegree.get(i) == Some(&0)).collect();
    let mut order = Vec::with_capacity(n);
    while let Some(next) = ready.pop_first() {
        order.push(next);
        for &m in dependents.get(next).map(Vec::as_slice).unwrap_or_default() {
            if let Some(d) = indegree.get_mut(m) {
                *d = d.saturating_sub(1);
                if *d == 0 {
                    ready.insert(m);
                }
            }
        }
    }
    let mut placed = vec![false; n];
    for &o in &order {
        if let Some(p) = placed.get_mut(o) {
            *p = true;
        }
    }
    let cyclic = (0..n)
        .filter(|&i| !placed.get(i).copied().unwrap_or(true))
        .collect();
    (order, cyclic)
}

/// User-facing name of a node: `'Web Sessions'` or `Customers[Has Orders]`.
pub(crate) fn node_label(model: &Model, node: Node) -> String {
    match node {
        Node::Table(i) => model
            .tables
            .get(i)
            .map(|t| dax_table_ref(&t.name))
            .unwrap_or_default(),
        Node::Column(i, j) => model
            .tables
            .get(i)
            .and_then(|t| t.columns.get(j).map(|c| dax_column_ref(&t.name, &c.name)))
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use pbi_model::{Column, ColumnRef, DataType, Partition, Relationship};

    fn calc_table(name: &str, expr: &str) -> Table {
        let mut t = Table::new(name);
        t.partitions.push(Partition {
            name: name.into(),
            source: PartitionSource::Calculated {
                expression: expr.into(),
            },
            ..Partition::default()
        });
        t
    }

    fn m_table(name: &str, columns: Vec<Column>) -> Table {
        let mut t = Table::new(name);
        t.columns = columns;
        t.partitions.push(Partition {
            name: name.into(),
            mode: PartitionMode::Import,
            source: PartitionSource::M {
                expression: "#table({}, {})".into(),
            },
            ..Partition::default()
        });
        t
    }

    /// The example's shape: Customers calc columns use
    /// RELATEDTABLE(Orders); the calculated table reads both tables.
    fn model() -> Model {
        let customers = m_table(
            "Customers",
            vec![
                Column::data("Customer ID", DataType::String),
                Column::calculated(
                    "Has Orders",
                    DataType::String,
                    "IF(ISEMPTY(RELATEDTABLE(Orders)), \"No\", \"Yes\")",
                ),
                Column::calculated("Flag", DataType::String, "[Has Orders] & \"!\""),
            ],
        );
        let orders = m_table(
            "Orders",
            vec![Column::data("Customer ID", DataType::String)],
        );
        let unmatched = calc_table(
            "Unknown Customers",
            "FILTER(VALUES(Orders[Customer ID]), ISEMPTY(FILTER(ALL(Customers[Customer ID]), Customers[Customer ID] = Orders[Customer ID])))",
        );
        let mut cyc = m_table(
            "Cyc",
            vec![
                Column::calculated("A", DataType::Int64, "[B] + 1"),
                Column::calculated("B", DataType::Int64, "[A] + 1"),
            ],
        );
        cyc.partitions.clear();
        Model {
            tables: vec![customers, orders, unmatched, cyc],
            relationships: vec![Relationship::new(
                "r",
                ColumnRef::new("Orders", "Customer ID"),
                ColumnRef::new("Customers", "Customer ID"),
            )],
            ..Model::default()
        }
    }

    #[test]
    fn calculated_objects_follow_their_dependencies() {
        let m = model();
        let plan = Plan::build(&m, false);
        let pos = |node: Node| {
            let i = plan.index(node).unwrap();
            plan.order.iter().position(|&o| o == i).unwrap()
        };
        assert!(pos(Node::Table(1)) < pos(Node::Column(0, 1)));
        assert!(pos(Node::Table(0)) < pos(Node::Column(0, 1)));
        assert!(pos(Node::Column(0, 1)) < pos(Node::Column(0, 2)));
        assert!(pos(Node::Table(0)) < pos(Node::Table(2)));
        assert!(pos(Node::Table(1)) < pos(Node::Table(2)));
        let cyclic: Vec<Node> = plan.cyclic.iter().map(|&c| plan.nodes[c]).collect();
        assert_eq!(cyclic, vec![Node::Column(3, 0), Node::Column(3, 1)]);
        assert_eq!(
            plan.sources[3],
            Source::Skip("the table has no partition".into())
        );
        assert_eq!(plan.sources[2], Source::Calculated);
        assert!(matches!(plan.sources[0], Source::M(_)));
    }

    #[test]
    fn affected_nodes_are_downstream_of_the_seeds() {
        let m = model();
        let plan = Plan::build(&m, false);
        let seed = plan.index(Node::Table(1)).unwrap();
        let affected = plan.affected(&[seed]);
        let hit: Vec<Node> = plan
            .nodes
            .iter()
            .zip(&affected)
            .filter(|(_, a)| **a)
            .map(|(n, _)| *n)
            .collect();
        assert_eq!(
            hit,
            vec![
                Node::Column(0, 1),
                Node::Column(0, 2),
                Node::Table(1),
                Node::Table(2)
            ]
        );
        assert_eq!(node_label(&m, Node::Column(0, 1)), "Customers[Has Orders]");
    }

    #[test]
    fn stored_calculated_columns_have_no_steps() {
        let m = model();
        let sources = vec![
            Source::Cache,
            Source::Cache,
            Source::Calculated,
            Source::NotCached("x".into()),
        ];
        // Customers[Has Orders] is cached, [Flag] is computed.
        let plan = Plan::build_with(&m, sources, |i, j| !(i == 0 && j == 1));
        assert!(plan.index(Node::Column(0, 1)).is_none());
        let flag = plan.index(Node::Column(0, 2)).unwrap();
        let deps: Vec<Node> = plan.deps[flag].iter().map(|&d| plan.nodes[d]).collect();
        assert_eq!(deps, vec![Node::Table(0)]);
        assert_eq!(plan.sources[3], Source::NotCached("x".into()));
    }

    #[test]
    fn pbix_tables_have_no_dax_steps() {
        let plan = Plan::build(&model(), true);
        assert!(plan.nodes.iter().all(|n| matches!(n, Node::Table(_))));
        assert!(plan.sources.iter().all(|s| *s == Source::Pbix));
    }
}
