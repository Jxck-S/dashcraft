//! Data of deleted tables and columns. A delete takes the object's data out
//! of the engine, so DAX no longer sees it, and keeps it here by
//! [`ObjectId`]: undoing the delete puts the same data back instantly
//! instead of reloading or recomputing it.

use std::collections::HashMap;
use std::sync::Arc;

use pbi_engine::{Detached, Storage, StorageEdits, StorageRename, StoredObject};
use pbi_model::names::name_key;
use pbi_model::{Model, ObjectId, Table, names_eq};

use crate::edit::{ModelRenames, rename_results};
use crate::report::{ColumnRefresh, TableRefresh};

#[derive(Debug)]
enum Kept {
    Table {
        data: Detached,
        refresh: Option<TableRefresh>,
    },
    Column {
        data: Detached,
        /// The column's refresh result and its position in its table's.
        refresh: Option<(usize, ColumnRefresh)>,
    },
}

impl Kept {
    fn data(&self) -> &Detached {
        match self {
            Kept::Table { data, .. } | Kept::Column { data, .. } => data,
        }
    }
}

/// Stored data of deleted tables and columns, by model object id. Cloning
/// is cheap (shared entries).
#[derive(Debug, Clone, Default)]
pub(crate) struct RemovedData(HashMap<ObjectId, Arc<Kept>>);

/// The storage side of a model edit: the tables and columns to take out of
/// the engine and the kept ones to put back.
pub(crate) struct DataPlan {
    detach: Vec<(ObjectId, StoredObject)>,
    attach: Vec<(ObjectId, StoredObject)>,
}

impl DataPlan {
    /// Detaches the tables and columns `old` has and `new` no longer has (by
    /// id) and attaches the kept data of those `new` has again. A kept
    /// column whose table's row count changed since (a refresh) stays out.
    pub fn between(old: &Model, new: &Model, kept: &RemovedData, storage: &Storage) -> Self {
        let mut plan = DataPlan {
            detach: Vec::new(),
            attach: Vec::new(),
        };
        let old_tables = by_id(&old.tables);
        let new_tables = by_id(&new.tables);
        for table in old.tables.iter().filter(|t| t.id.is_assigned()) {
            let Some(after) = new_tables.get(&table.id) else {
                plan.detach
                    .push((table.id, StoredObject::Table(table.name.clone())));
                continue;
            };
            for c in table.columns.iter().filter(|c| c.id.is_assigned()) {
                if !after.columns.iter().any(|a| a.id == c.id) {
                    plan.detach.push((
                        c.id,
                        StoredObject::Column {
                            table: table.name.clone(),
                            column: c.name.clone(),
                        },
                    ));
                }
            }
        }
        for table in new.tables.iter().filter(|t| t.id.is_assigned()) {
            let Some(before) = old_tables.get(&table.id) else {
                if kept.0.get(&table.id).is_some_and(|k| k.data().is_table()) {
                    plan.attach
                        .push((table.id, StoredObject::Table(table.name.clone())));
                }
                continue;
            };
            let rows = storage.row_count(&before.name).ok();
            for c in table.columns.iter().filter(|c| c.id.is_assigned()) {
                if before.columns.iter().any(|b| b.id == c.id) {
                    continue;
                }
                if let Some(k) = kept.0.get(&c.id)
                    && !k.data().is_table()
                    && Some(k.data().rows()) == rows
                {
                    plan.attach.push((
                        c.id,
                        StoredObject::Column {
                            table: table.name.clone(),
                            column: c.name.clone(),
                        },
                    ));
                }
            }
        }
        plan
    }

    /// The engine edits: detach, then `renames`, then attach.
    pub fn edits(&self, kept: &RemovedData, renames: Vec<StorageRename>) -> StorageEdits {
        StorageEdits {
            detach: self.detach.iter().map(|(_, o)| o.clone()).collect(),
            renames,
            attach: self
                .attach
                .iter()
                .filter_map(|(id, o)| kept.0.get(id).map(|k| (o.clone(), k.data().clone())))
                .collect(),
        }
    }

    /// The kept data after the engine applied [`DataPlan::edits`]
    /// (`detached` is what it returned), and the per-table refresh results
    /// that follow: those of detached objects are kept with their data,
    /// renamed, and put back with attached ones.
    pub fn apply(
        self,
        kept: &RemovedData,
        detached: Vec<Option<Detached>>,
        tables: &mut HashMap<String, TableRefresh>,
        renames: &ModelRenames,
    ) -> RemovedData {
        let mut out = kept.clone();
        for ((id, object), data) in self.detach.into_iter().zip(detached) {
            let entry = match object {
                StoredObject::Table(name) => {
                    let refresh = tables.remove(&name_key(&name));
                    data.map(|data| Kept::Table { data, refresh })
                }
                StoredObject::Column { table, column } => {
                    let refresh = tables.get_mut(&name_key(&table)).and_then(|t| {
                        let i = t.columns.iter().position(|c| names_eq(&c.name, &column))?;
                        Some((i, t.columns.remove(i)))
                    });
                    data.map(|data| Kept::Column { data, refresh })
                }
            };
            if let Some(entry) = entry {
                out.0.insert(id, Arc::new(entry));
            }
        }
        rename_results(tables, renames);
        for (id, object) in self.attach {
            let Some(entry) = out.0.remove(&id) else {
                continue;
            };
            match (entry.as_ref(), object) {
                (
                    Kept::Table {
                        refresh: Some(r), ..
                    },
                    StoredObject::Table(name),
                ) => {
                    let mut r = r.clone();
                    r.name.clone_from(&name);
                    tables.insert(name_key(&name), r);
                }
                (
                    Kept::Column {
                        refresh: Some((i, r)),
                        ..
                    },
                    StoredObject::Column { table, column },
                ) => {
                    if let Some(t) = tables.get_mut(&name_key(&table)) {
                        let mut r = r.clone();
                        r.name = column;
                        t.columns.insert((*i).min(t.columns.len()), r);
                    }
                }
                _ => {}
            }
        }
        out
    }
}

fn by_id(tables: &[Table]) -> HashMap<ObjectId, &Table> {
    tables
        .iter()
        .filter(|t| t.id.is_assigned())
        .map(|t| (t.id, t))
        .collect()
}
