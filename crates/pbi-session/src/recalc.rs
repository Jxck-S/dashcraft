//! Recalculation after a model edit: the calculated tables and columns an
//! edit invalidates are recomputed in dependency order over the loaded
//! data, without an M refresh (as Power BI Desktop recalculates).
//!
//! An edit invalidates:
//! - the calculated tables and columns it creates (unless their data is
//!   kept, as when a delete is undone) or whose expression or data type it
//!   changes (*direct*);
//! - those that use a measure whose expression changed, those that use
//!   another table when the relationships changed, and those whose
//!   references resolve to other objects after tables, columns or measures
//!   were created or deleted (*indirect*);
//! - everything downstream of them.
//!
//! Renames alone invalidate nothing (expressions are compared after the
//! renames are applied). Measures need no recalculation: queries evaluate
//! them on the current model.
//!
//! A direct failure refuses the edit ([`CommandError::Rejected`]) unless
//! the step is lenient (undo and redo must not get stuck). Any other failure
//! keeps the edit: the values no longer match the model, so they are dropped
//! (a calculated table is emptied, a calculated column is taken out of the
//! table data) and the error is recorded, until an edit or a refresh
//! computes them again.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use pbi_dax::Dependency;
use pbi_engine::{CancellationToken, Engine};
use pbi_model::names::{dax_column_ref, name_key, names_eq};
use pbi_model::{ColumnRef, CommandError, Model, ObjectId, Relationship, Table, TableKind};

use crate::derived::{self, DerivedEntry, Failure, data_type_of, synthetic_column};
use crate::edit::ModelRenames;
use crate::refresh::plan::{Node, Plan, Source, node_label};
use crate::refresh::{dax_status, millis, plain_error};
use crate::report::{ColumnRefresh, Recalculated, RefreshKind, TableRefresh, TableStatus};

/// What an edit changed that calculated tables and columns may depend on.
#[derive(Debug, Default)]
pub(crate) struct Changes {
    /// Calculated tables and columns whose expression or data type changed.
    calc: HashSet<ObjectId>,
    /// Calculated tables and columns the edit created.
    created: HashSet<ObjectId>,
    /// Measures (name keys, new spelling) whose expression changed.
    measures: HashSet<String>,
    relationships: bool,
    /// Tables, columns or measures were created or deleted.
    objects: bool,
}

impl Changes {
    pub fn between(old: &Model, new: &Model, renames: &ModelRenames) -> Changes {
        let mut out = Changes::default();
        let old_tables: HashMap<ObjectId, &Table> = old
            .tables
            .iter()
            .filter(|t| t.id.is_assigned())
            .map(|t| (t.id, t))
            .collect();
        let normalized = |expr: &str, home: &str| {
            let mut text = expr.to_owned();
            if !renames.is_empty() {
                renames.rewrite_dax(&mut text, home, new);
            }
            text
        };
        for table in new.tables.iter().filter(|t| t.id.is_assigned()) {
            let before = old_tables.get(&table.id).copied();
            if table.kind() == TableKind::CalculatedTable {
                match before {
                    None => {
                        out.created.insert(table.id);
                    }
                    Some(b) => {
                        let old_expr = b
                            .calculated_expression()
                            .map(|e| normalized(e, &table.name));
                        if old_expr.as_deref() != table.calculated_expression() {
                            out.calc.insert(table.id);
                        }
                    }
                }
            }
            for column in table
                .columns
                .iter()
                .filter(|c| c.is_calculated() && c.id.is_assigned())
            {
                match before.and_then(|b| b.columns.iter().find(|c| c.id == column.id)) {
                    None => {
                        out.created.insert(column.id);
                    }
                    Some(prior) => {
                        let old_expr = prior.expression().map(|e| normalized(e, &table.name));
                        if old_expr.as_deref() != column.expression()
                            || prior.data_type != column.data_type
                        {
                            out.calc.insert(column.id);
                        }
                    }
                }
            }
            for measure in table.measures.iter().filter(|m| m.id.is_assigned()) {
                if let Some(prior) =
                    before.and_then(|b| b.measures.iter().find(|m| m.id == measure.id))
                    && normalized(&prior.expression, &table.name) != measure.expression
                {
                    out.measures.insert(name_key(&measure.name));
                }
            }
        }
        out.objects = object_ids(old) != object_ids(new);
        out.relationships = relationships_changed(old, new);
        out
    }

    /// `true` when no calculated table or column can be affected.
    pub fn is_empty(&self) -> bool {
        self.calc.is_empty()
            && self.created.is_empty()
            && self.measures.is_empty()
            && !self.relationships
            && !self.objects
    }
}

fn object_ids(model: &Model) -> HashSet<ObjectId> {
    model
        .tables
        .iter()
        .flat_map(|t| {
            std::iter::once(t.id)
                .chain(t.columns.iter().map(|c| c.id))
                .chain(t.measures.iter().map(|m| m.id))
        })
        .filter(|id| id.is_assigned())
        .collect()
}

/// What a reference resolves to: an object, or a name that resolves to
/// nothing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Target {
    Id(ObjectId),
    Missing(String),
}

fn target(id: Option<ObjectId>, missing: impl FnOnce() -> String) -> Target {
    match id.filter(|id| id.is_assigned()) {
        Some(id) => Target::Id(id),
        None => Target::Missing(missing()),
    }
}

fn endpoint(model: &Model, c: &ColumnRef) -> Target {
    target(model.column(&c.table, &c.column).map(|x| x.id), || {
        format!("{}\0{}", name_key(&c.table), name_key(&c.column))
    })
}

/// Whether relationships were added, removed or changed in a way that
/// affects filter propagation (renames of their columns do not count).
fn relationships_changed(old: &Model, new: &Model) -> bool {
    if old.relationships.len() != new.relationships.len() {
        return true;
    }
    if old.relationships == new.relationships {
        return false;
    }
    let same = |a: &Relationship, b: &Relationship| {
        endpoint(old, &a.from) == endpoint(new, &b.from)
            && endpoint(old, &a.to) == endpoint(new, &b.to)
            && a.from_cardinality == b.from_cardinality
            && a.to_cardinality == b.to_cardinality
            && a.cross_filtering_behavior == b.cross_filtering_behavior
            && a.is_active == b.is_active
            && a.extra == b.extra
    };
    let in_order = old
        .relationships
        .iter()
        .zip(&new.relationships)
        .all(|(a, b)| same(a, b));
    !in_order
        && !new
            .relationships
            .iter()
            .all(|b| old.relationships.iter().any(|a| same(a, b)))
}

/// The objects the (transitive) references of `expr` resolve to.
fn resolved(model: &Model, home: &str, expr: &str) -> HashSet<Target> {
    pbi_dax::transitive_dependencies(model, Some(home), expr)
        .unwrap_or_default()
        .iter()
        .map(|dep| match dep {
            Dependency::Table(t) => target(model.table(t).map(|x| x.id), || {
                format!("t\0{}", name_key(t))
            }),
            Dependency::Column { table, column } => {
                target(model.column(table, column).map(|x| x.id), || {
                    format!("c\0{}\0{}", name_key(table), name_key(column))
                })
            }
            Dependency::Measure(m) => target(model.find_measure(m).map(|(_, x)| x.id), || {
                format!("m\0{}", name_key(m))
            }),
        })
        .collect()
}

/// The id, home table and expression of a calculated node.
fn node_definition(model: &Model, node: Node) -> Option<(ObjectId, &Table, Option<&str>)> {
    match node {
        Node::Table(i) => {
            let t = model.tables.get(i)?;
            Some((t.id, t, t.calculated_expression()))
        }
        Node::Column(i, j) => {
            let t = model.tables.get(i)?;
            let c = t.columns.get(j)?;
            Some((c.id, t, c.expression()))
        }
    }
}

/// The same table or column in `old`, by id, with its home table and
/// expression.
fn old_definition(old: &Model, id: ObjectId) -> Option<(&Table, Option<&str>)> {
    old.tables.iter().find_map(|t| {
        if t.id == id {
            return Some((t, t.calculated_expression()));
        }
        t.columns
            .iter()
            .find(|c| c.id == id)
            .map(|c| (t, c.expression()))
    })
}

/// A recalculation in progress.
struct Run<'a> {
    engine: &'a mut Engine,
    new: &'a Model,
    eval_model: &'a Model,
    tables: &'a mut HashMap<String, TableRefresh>,
    plan: Plan,
    out: Vec<Recalculated>,
}

impl Run<'_> {
    /// Records a result (and drops the values of a failed step).
    fn record(&mut self, node: Node, id: ObjectId, duration_ms: u64, status: TableStatus) {
        match node {
            Node::Table(i) => {
                let Some(table) = self.new.tables.get(i) else {
                    return;
                };
                if !status.is_ok() {
                    self.engine.unload_table(table);
                }
                let rows = self.engine.row_count(&table.name).unwrap_or(0);
                let entry =
                    self.tables
                        .entry(name_key(&table.name))
                        .or_insert_with(|| TableRefresh {
                            name: table.name.clone(),
                            kind: RefreshKind::Calculated,
                            rows,
                            duration_ms,
                            status: TableStatus::Ok,
                            columns: Vec::new(),
                        });
                entry.kind = RefreshKind::Calculated;
                entry.rows = rows;
                entry.duration_ms = duration_ms;
                entry.status = status.clone();
                self.out.push(Recalculated {
                    table: table.name.clone(),
                    column: None,
                    id,
                    duration_ms,
                    status,
                });
            }
            Node::Column(i, j) => {
                let Some((table, column)) = self
                    .new
                    .tables
                    .get(i)
                    .and_then(|t| t.columns.get(j).map(|c| (t, c)))
                else {
                    return;
                };
                if !status.is_ok()
                    && let Err(e) = self.engine.remove_column(&table.name, &column.name)
                {
                    tracing::warn!(table = %table.name, column = %column.name, error = %e, "cannot drop a stale calculated column");
                }
                if let Some(entry) = self.tables.get_mut(&name_key(&table.name)) {
                    match entry
                        .columns
                        .iter_mut()
                        .find(|c| names_eq(&c.name, &column.name))
                    {
                        Some(c) => {
                            c.duration_ms = duration_ms;
                            c.status = status.clone();
                        }
                        None => entry.columns.push(ColumnRefresh {
                            name: column.name.clone(),
                            duration_ms,
                            status: status.clone(),
                        }),
                    }
                }
                self.out.push(Recalculated {
                    table: table.name.clone(),
                    column: Some(column.name.clone()),
                    id,
                    duration_ms,
                    status,
                });
            }
        }
    }

    /// Computes and stores a calculated table or column.
    fn compute(&mut self, node: Node) -> Result<(), TableStatus> {
        let snapshot = self.engine.snapshot();
        match node {
            Node::Table(i) => {
                let name = self
                    .new
                    .tables
                    .get(i)
                    .map(|t| t.name.as_str())
                    .unwrap_or_default();
                let batch = pbi_dax::compute_calculated_table(&snapshot, self.eval_model, name)
                    .map_err(|e| dax_status(&e))?;
                self.engine
                    .load_table(name, batch)
                    .map_err(|e| plain_error(format!("cannot store the table: {e}"), false))
            }
            Node::Column(i, j) => {
                let (table, column) = self
                    .new
                    .tables
                    .get(i)
                    .and_then(|t| t.columns.get(j).map(|c| (t.name.as_str(), c.name.as_str())))
                    .unwrap_or_default();
                let array =
                    pbi_dax::compute_calculated_column(&snapshot, self.eval_model, table, column)
                        .map_err(|e| dax_status(&e))?;
                self.engine
                    .add_column(table, column, array)
                    .map_err(|e| plain_error(format!("cannot store the column: {e}"), false))
            }
        }
    }
}

/// The user-facing name of a node in error messages: `table 'Name'` or
/// `'Table'[Column]`.
fn subject(model: &Model, node: Node) -> String {
    match node {
        Node::Table(i) => format!(
            "table '{}'",
            model
                .tables
                .get(i)
                .map(|t| t.name.as_str())
                .unwrap_or_default()
        ),
        Node::Column(i, j) => model
            .tables
            .get(i)
            .and_then(|t| t.columns.get(j).map(|c| dax_column_ref(&t.name, &c.name)))
            .unwrap_or_default(),
    }
}

/// Recomputes what `changes` invalidates (see the module docs) in `engine`
/// and records the results in `tables`. `eval_model` is `new` plus the
/// session's derived columns. Nothing runs on tables that are not loaded
/// (the next refresh computes them), except that a created calculated
/// column is added to an empty table so its schema follows the model.
pub(crate) fn recalculate(
    engine: &mut Engine,
    old: &Model,
    new: &Model,
    eval_model: &Model,
    changes: &Changes,
    tables: &mut HashMap<String, TableRefresh>,
    strict: bool,
) -> Result<Vec<Recalculated>, CommandError> {
    if changes.is_empty() {
        return Ok(Vec::new());
    }
    let plan = Plan::build(new, false);
    let data_loaded = !tables.is_empty();
    let mut direct = vec![false; plan.nodes.len()];
    let mut seeds = Vec::new();
    for (n, &node) in plan.nodes.iter().enumerate() {
        let calc_table =
            matches!(node, Node::Table(i) if plan.sources.get(i) == Some(&Source::Calculated));
        if matches!(node, Node::Table(_)) && !calc_table {
            continue;
        }
        let Some((id, home, expr)) = node_definition(new, node) else {
            continue;
        };
        let created = changes.created.contains(&id)
            && match node {
                Node::Table(_) => !tables.contains_key(&name_key(&home.name)),
                Node::Column(_, j) => home.columns.get(j).is_some_and(|c| {
                    !engine
                        .table(&home.name)
                        .is_ok_and(|d| d.has_column(&c.name))
                }),
            };
        if changes.calc.contains(&id) || created {
            direct[n] = true;
            seeds.push(n);
            continue;
        }
        if changes.created.contains(&id) {
            continue;
        }
        let Some(expr) = expr else {
            continue;
        };
        let home_index = match node {
            Node::Table(i) | Node::Column(i, _) => i,
        };
        let across_tables = plan.deps.get(n).is_some_and(|deps| {
            deps.iter().any(|&d| match plan.nodes.get(d) {
                Some(Node::Table(k) | Node::Column(k, _)) => calc_table || *k != home_index,
                None => false,
            })
        });
        let uses_changed_measure = !changes.measures.is_empty()
            && pbi_dax::transitive_dependencies(new, Some(&home.name), expr).is_ok_and(|deps| {
                deps.iter().any(
                    |d| matches!(d, Dependency::Measure(m) if changes.measures.contains(&name_key(m))),
                )
            });
        let resolves_elsewhere = changes.objects
            && old_definition(old, id).is_some_and(|(old_home, old_expr)| {
                old_expr.is_some_and(|old_expr| {
                    resolved(old, &old_home.name, old_expr) != resolved(new, &home.name, expr)
                })
            });
        if (changes.relationships && across_tables) || uses_changed_measure || resolves_elsewhere {
            seeds.push(n);
        }
    }
    if seeds.is_empty() {
        return Ok(Vec::new());
    }
    let affected = plan.affected(&seeds);
    let mut run = Run {
        engine,
        new,
        eval_model,
        tables,
        plan,
        out: Vec::new(),
    };
    let mut failed = vec![false; run.plan.nodes.len()];
    let is_affected = |n: usize| affected.get(n).copied().unwrap_or(false);
    for n in run.plan.cyclic.clone() {
        let Some(&node) = run.plan.nodes.get(n).filter(|_| is_affected(n)) else {
            continue;
        };
        let Some((id, _, _)) = node_definition(new, node) else {
            continue;
        };
        let message = format!(
            "A circular dependency was detected: {} depends (directly or indirectly) on itself.",
            node_label(new, node)
        );
        if strict && direct.get(n).copied().unwrap_or(false) {
            return Err(CommandError::Rejected(message));
        }
        if let Some(f) = failed.get_mut(n) {
            *f = true;
        }
        run.record(node, id, 0, plain_error(message, false));
    }
    for n in run.plan.order.clone() {
        let Some(&node) = run.plan.nodes.get(n).filter(|_| is_affected(n)) else {
            continue;
        };
        let Some((id, home, _)) = node_definition(new, node) else {
            continue;
        };
        let is_direct = direct.get(n).copied().unwrap_or(false);
        let report = match node {
            Node::Table(i) => {
                if run.plan.sources.get(i) != Some(&Source::Calculated) || !data_loaded {
                    continue;
                }
                true
            }
            Node::Column(..) => match run.engine.table(&home.name) {
                Ok(data) if data.is_loaded() => true,
                // An empty table gets a created column's (empty) values so
                // its schema follows the model.
                Ok(_) if is_direct && changes.created.contains(&id) => false,
                _ => continue,
            },
        };
        let blocked = run.plan.deps.get(n).and_then(|deps| {
            deps.iter()
                .find(|&&d| failed.get(d).copied().unwrap_or(false))
                .and_then(|&d| run.plan.nodes.get(d))
                .map(|&d| node_label(new, d))
        });
        let start = Instant::now();
        let result = match blocked {
            Some(label) => Err(TableStatus::Skipped {
                reason: format!("depends on {label}, which cannot be calculated"),
            }),
            None => run.compute(node),
        };
        let duration_ms = millis(start.elapsed());
        match result {
            Ok(()) if report => run.record(node, id, duration_ms, TableStatus::Ok),
            Ok(()) => {}
            Err(status) => {
                if strict && is_direct {
                    let why = match &status {
                        TableStatus::Skipped { reason } => format!("it {reason}"),
                        other => other.message().unwrap_or_default().to_owned(),
                    };
                    return Err(CommandError::Rejected(format!(
                        "{} cannot be calculated: {why}",
                        subject(new, node)
                    )));
                }
                if let Some(f) = failed.get_mut(n) {
                    *f = true;
                }
                if report {
                    run.record(node, id, duration_ms, status);
                }
            }
        }
    }
    Ok(run.out)
}

/// Recomputes the active derived columns a recalculation reached: those of
/// a recalculated table, and DAX ones that use a recalculated table or
/// column or a changed measure. One that fails is taken out of the data
/// and the model, like at a refresh ([`crate::derived`]). `model` is the
/// session model (`base` plus the active derived columns). Returns whether
/// any was recomputed.
pub(crate) fn recompute_derived(
    engine: &mut Engine,
    base: &Model,
    model: &mut Model,
    derived: &mut [DerivedEntry],
    recalculated: &[Recalculated],
    changes: &Changes,
) -> bool {
    if recalculated.is_empty() && changes.measures.is_empty() {
        return false;
    }
    let mut any = false;
    let reloaded = |table: &str| {
        recalculated
            .iter()
            .any(|r| r.column.is_none() && names_eq(&r.table, table))
    };
    let recomputed = |table: &str, column: Option<&str>| {
        recalculated.iter().any(|r| {
            names_eq(&r.table, table)
                && match (column, &r.column) {
                    (Some(c), Some(rc)) => names_eq(c, rc),
                    (None, _) => true,
                    (Some(_), None) => false,
                }
        })
    };
    for entry in derived.iter_mut().filter(|e| e.info.active) {
        let table = entry.info.table.clone();
        let uses_recalculated = match &entry.column {
            derived::DerivedColumn::Expression { dax } => {
                pbi_dax::transitive_dependencies(model, Some(&table), dax).is_ok_and(|deps| {
                    deps.iter().any(|d| match d {
                        Dependency::Measure(m) => changes.measures.contains(&name_key(m)),
                        Dependency::Table(t) => recomputed(t, None),
                        Dependency::Column { table, column } => recomputed(table, Some(column)),
                    })
                })
            }
            _ => false,
        };
        if !reloaded(&table) && !uses_recalculated {
            continue;
        }
        any = true;
        let mut without = model.clone();
        if let Some(t) = without.table_mut(&table) {
            t.columns.retain(|c| !names_eq(&c.name, &entry.info.name));
        }
        let result = derived::compute(
            engine.snapshot(),
            &without,
            &table,
            &entry.info.name,
            &entry.column,
            &CancellationToken::new(),
        );
        let status = match result {
            Ok(array) => {
                let data_type = data_type_of(array.data_type());
                match engine.add_column(&table, &entry.info.name, array) {
                    Ok(()) => {
                        entry.info.data_type = data_type;
                        TableStatus::Ok
                    }
                    Err(e) => plain_error(format!("cannot store the column: {e}"), false),
                }
            }
            Err(Failure::Status(status)) => status,
            Err(Failure::Cancelled) => plain_error("cancelled", false),
        };
        if status.is_ok() {
            if let Some(c) = model.table_mut(&table).and_then(|t| {
                t.columns
                    .iter_mut()
                    .find(|c| names_eq(&c.name, &entry.info.name))
            }) {
                *c = synthetic_column(&entry.info, &entry.column);
            }
        } else {
            if let Err(e) = derived::drop_column(engine, base, &table, &entry.info.name) {
                tracing::warn!(table = %table, column = %entry.info.name, error = %e, "cannot drop a derived column");
            }
            if let Some(t) = model.table_mut(&table) {
                t.columns.retain(|c| !names_eq(&c.name, &entry.info.name));
            }
            entry.info.active = false;
        }
        entry.info.status = status;
    }
    any
}
