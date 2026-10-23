//! The local data cache of a PBIP: `<Model>.SemanticModel/.pbi/cache.abf`.
//!
//! Power BI Desktop saves the data of its last refresh there (an
//! uncompressed Analysis Services backup, the same format as a PBIX
//! `DataModel` after XPress9 decompression). [`Session::load_cached_data`]
//! decodes it with `pbi-vertipaq` and maps its tables and columns onto the
//! PBIP model by name (case-insensitive), without evaluating any M or
//! touching the network:
//!
//! - a model table found in the cache loads its columns from there (values
//!   are converted to the model data type when it differs); data columns
//!   missing from the cache load as BLANK and are reported (a skipped column
//!   entry plus a warning);
//! - calculated columns and calculated tables missing from the cache are
//!   computed by `pbi-dax` as in a normal refresh; stored ones are used as
//!   they are;
//! - a regular table missing from the cache fails with a hint to refresh
//!   from the source (or load a CSV); what depends on it is skipped.

use std::path::PathBuf;
use std::sync::Arc;

use arrow::array::{ArrayRef, new_null_array};
use arrow::compute::{CastOptions, cast_with_options};
use arrow::datatypes::{Field, Schema};
use arrow::record_batch::{RecordBatch, RecordBatchOptions};
use chrono::{DateTime, Utc};
use pbi_core::ModelSource;
use pbi_core::container::{Container, ContainerKind};
use pbi_model::names::name_key;
use pbi_model::{ColumnKind, DataType, Diagnostic, Model, Origin, TableKind, names_eq};
use serde::{Deserialize, Serialize};

use crate::error::SessionError;
use crate::options::{RefreshOptions, RefreshSource};
use crate::refresh::codes;
use crate::refresh::plan::{Plan, Source, table_source};
use crate::report::RefreshReport;
use crate::session::Session;

/// Path of the cache relative to a semantic model folder.
pub const CACHE_ENTRY: &str = ".pbi/cache.abf";

/// The local data cache of the open document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheInfo {
    /// The cache entry in the opened folder, e.g.
    /// `AcmeOutfitters.SemanticModel/.pbi/cache.abf`.
    pub entry: String,
    /// The file on disk (folders only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// Size in bytes.
    pub size: u64,
    /// Last modification time of the file (the last refresh in Power BI
    /// Desktop), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<DateTime<Utc>>,
}

/// One model table's data from the cache.
#[derive(Debug, Clone)]
pub(crate) struct CachedTable {
    /// Model columns in model order: those found in the cache, missing data
    /// columns as BLANK; missing calculated columns are left out.
    pub batch: RecordBatch,
    /// Per model column: a calculated column whose values are in the cache.
    pub stored: Vec<bool>,
    /// Data columns missing from the cache (loaded as BLANK).
    pub missing: Vec<String>,
}

/// The cache mapped onto the model.
#[derive(Debug, Clone, Default)]
pub(crate) struct CacheData {
    /// The cache entry (for messages).
    pub entry: String,
    /// Per model table (same order as `Model::tables`).
    pub tables: Vec<Option<CachedTable>>,
    pub diagnostics: Vec<Diagnostic>,
}

impl CacheData {
    /// Table sources and calculated-column steps for a refresh from the
    /// cache.
    pub fn plan(&self, model: &Model) -> Plan {
        let sources: Vec<Source> = model
            .tables
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let cached = self.tables.get(i).is_some_and(Option::is_some);
                match t.kind() {
                    TableKind::Remote | TableKind::CalculationGroup => table_source(t, false),
                    _ if cached => Source::Cache,
                    TableKind::CalculatedTable => Source::Calculated,
                    TableKind::Regular => Source::NotCached(format!(
                        "'{}' is not in the local data cache ({}); the cache may be older than the model",
                        t.name, self.entry
                    )),
                }
            })
            .collect();
        Plan::build_with(model, sources, |i, j| {
            !self
                .tables
                .get(i)
                .and_then(Option::as_ref)
                .and_then(|t| t.stored.get(j))
                .copied()
                .unwrap_or(false)
        })
    }

    /// Takes the batch of table `i`.
    pub fn take(&mut self, i: usize) -> Option<CachedTable> {
        self.tables.get_mut(i).and_then(Option::take)
    }
}

/// The cache entry of the document's model in its container.
fn cache_location(session: &Session) -> Option<(Arc<dyn Container>, String)> {
    let (container, folder) = match &session.opened().model_source {
        Some(ModelSource::Tmdl {
            container, folder, ..
        })
        | Some(ModelSource::Tmsl {
            container, folder, ..
        }) => (container, folder),
        _ => return None,
    };
    let entry = if folder.is_empty() {
        CACHE_ENTRY.to_owned()
    } else {
        format!("{}/{CACHE_ENTRY}", folder.trim_end_matches('/'))
    };
    container
        .is_file(&entry)
        .then(|| (Arc::clone(container), entry))
}

/// Converts a cached column to the model data type when they differ (values
/// that do not convert become BLANK); `None` if it cannot be converted.
fn to_model_type(array: &ArrayRef, data_type: &DataType) -> Option<ArrayRef> {
    if matches!(
        data_type,
        DataType::Automatic | DataType::Variant | DataType::Binary | DataType::Unknown(_)
    ) {
        return Some(Arc::clone(array));
    }
    let target = pbi_engine::arrow_type(data_type);
    if array.data_type() == &target {
        return Some(Arc::clone(array));
    }
    cast_with_options(array, &target, &CastOptions::default()).ok()
}

/// Maps decoded cache tables onto `model` (pure; see the module docs).
pub(crate) fn map_cache(
    model: &Model,
    entry: &str,
    decoded: Vec<(String, RecordBatch)>,
    mut diagnostics: Vec<Diagnostic>,
) -> CacheData {
    let origin = Origin::file(entry);
    let mut used = vec![false; decoded.len()];
    let mut tables = Vec::with_capacity(model.tables.len());
    for t in &model.tables {
        if matches!(t.kind(), TableKind::Remote | TableKind::CalculationGroup) {
            tables.push(None);
            continue;
        }
        let Some(k) = decoded.iter().position(|(name, _)| names_eq(name, &t.name)) else {
            if t.kind() == TableKind::Regular {
                diagnostics.push(
                    Diagnostic::warning(
                        codes::CACHE_MISSING,
                        format!("'{}' is not in the local data cache", t.name),
                    )
                    .with_origin(origin.clone()),
                );
            }
            tables.push(None);
            continue;
        };
        if let Some(u) = used.get_mut(k) {
            *u = true;
        }
        let Some((_, batch)) = decoded.get(k) else {
            tables.push(None);
            continue;
        };
        let rows = batch.num_rows();
        let schema = batch.schema();
        let mut seen = std::collections::HashSet::new();
        let (mut fields, mut arrays) = (Vec::new(), Vec::new());
        let mut stored = vec![false; t.columns.len()];
        let mut missing = Vec::new();
        for (j, c) in t.columns.iter().enumerate() {
            if matches!(c.kind, ColumnKind::RowNumber) || !seen.insert(name_key(&c.name)) {
                continue;
            }
            let found = schema
                .fields()
                .iter()
                .position(|f| names_eq(f.name(), &c.name))
                .and_then(|p| batch.columns().get(p));
            let array = match found {
                Some(array) => {
                    if c.is_calculated()
                        && let Some(s) = stored.get_mut(j)
                    {
                        *s = true;
                    }
                    match to_model_type(array, &c.data_type) {
                        Some(converted) => converted,
                        None => {
                            diagnostics.push(
                                Diagnostic::warning(
                                    codes::CACHE_CONVERSION,
                                    format!(
                                        "'{}'[{}]: the cached values ({}) cannot be converted to {}; kept as they are",
                                        t.name,
                                        c.name,
                                        array.data_type(),
                                        c.data_type.as_str()
                                    ),
                                )
                                .with_origin(origin.clone()),
                            );
                            Arc::clone(array)
                        }
                    }
                }
                // Computed by DAX after the load.
                None if c.is_calculated() => continue,
                None => {
                    diagnostics.push(
                        Diagnostic::warning(
                            codes::CACHE_MISSING,
                            format!(
                                "'{}'[{}] is not in the local data cache; loaded as blank",
                                t.name, c.name
                            ),
                        )
                        .with_origin(origin.clone()),
                    );
                    missing.push(c.name.clone());
                    new_null_array(&pbi_engine::arrow_type(&c.data_type), rows)
                }
            };
            fields.push(Field::new(&c.name, array.data_type().clone(), true));
            arrays.push(array);
        }
        let options = RecordBatchOptions::new().with_row_count(Some(rows));
        match RecordBatch::try_new_with_options(Arc::new(Schema::new(fields)), arrays, &options) {
            Ok(batch) => tables.push(Some(CachedTable {
                batch,
                stored,
                missing,
            })),
            Err(e) => {
                diagnostics.push(
                    Diagnostic::error(
                        codes::CACHE_MISSING,
                        format!("'{}': cannot build the table from the cache: {e}", t.name),
                    )
                    .with_origin(origin.clone()),
                );
                tables.push(None);
            }
        }
    }
    let extra: Vec<&str> = decoded
        .iter()
        .zip(&used)
        .filter(|(_, u)| !**u)
        .map(|((name, _), _)| name.as_str())
        .filter(|name| {
            !name.starts_with("LocalDateTable_") && !name.starts_with("DateTableTemplate_")
        })
        .collect();
    if !extra.is_empty() {
        diagnostics.push(
            Diagnostic::info(
                codes::CACHE_MISSING,
                format!(
                    "the local data cache has tables that are not in the model: {}",
                    extra.join(", ")
                ),
            )
            .with_origin(origin),
        );
    }
    CacheData {
        entry: entry.to_owned(),
        tables,
        diagnostics,
    }
}

impl Session {
    /// The local data cache of the document's semantic model, if it has one:
    /// a PBIP's `<Model>.SemanticModel/.pbi/cache.abf`, written by Power BI
    /// Desktop's last refresh. PBIX and ABF files carry their data inside
    /// and have no separate cache.
    #[must_use]
    pub fn cache_info(&self) -> Option<CacheInfo> {
        let (container, entry) = cache_location(self)?;
        let size = container.entry(&entry).map_or(0, |e| e.size);
        let path = (container.kind() == ContainerKind::Directory)
            .then(|| PathBuf::from(container.root_label()).join(&entry));
        let modified = path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).ok())
            .and_then(|m| m.modified().ok())
            .map(DateTime::<Utc>::from);
        Some(CacheInfo {
            entry,
            path,
            size,
            modified,
        })
    }

    /// `true` if [`cache_info`](Self::cache_info) finds a local data cache.
    #[must_use]
    pub fn has_cached_data(&self) -> bool {
        cache_location(self).is_some()
    }

    /// Loads every table from the local data cache (see the
    /// [module docs](crate::cache)): no M evaluation, no network. Calculated
    /// columns and tables missing from the cache are computed; derived
    /// columns are re-applied. Same as [`refresh`](Self::refresh) with
    /// [`RefreshSource::Cache`]; PBIX and ABF files load their embedded data.
    ///
    /// # Errors
    /// [`SessionError::NoCache`], [`SessionError::Cache`] if the cache cannot
    /// be read, [`SessionError::Cancelled`], [`SessionError::Internal`].
    pub async fn load_cached_data(&self) -> Result<RefreshReport, SessionError> {
        self.refresh(RefreshOptions::all().with_source(RefreshSource::Cache))
            .await
    }

    /// Reads, decodes and maps the cache (blocking).
    pub(crate) fn read_cache(&self) -> Result<CacheData, SessionError> {
        let (container, entry) = cache_location(self).ok_or(SessionError::NoCache)?;
        let fail = |reason: String| SessionError::Cache {
            entry: entry.clone(),
            reason,
        };
        let bytes = container.read(&entry).map_err(|e| fail(e.to_string()))?;
        let limits = self.limits().decode;
        let archive = pbi_vertipaq::AbfArchive::open(bytes, &limits.container)
            .map_err(|e| fail(e.to_string()))?;
        let loaded = pbi_vertipaq::load_model(&archive, &pbi_vertipaq::ModelOptions::default())
            .map_err(|e| fail(e.to_string()))?;
        let decoded = pbi_vertipaq::decode_all_with_diagnostics(
            &archive,
            &loaded.storage,
            &loaded.model,
            &limits,
        );
        Ok(map_cache(
            &self.base_model(),
            &entry,
            decoded.tables,
            decoded.diagnostics,
        ))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use arrow::array::{Array, AsArray, Int32Array, StringArray};
    use arrow::datatypes::Int64Type;
    use pbi_model::{Column, Partition, PartitionSource, Table};

    fn model() -> Model {
        let mut stores = Table::new("Stores");
        stores.columns = vec![
            Column::data("Store ID", DataType::String),
            Column::data("Count", DataType::Int64),
            Column::data("New", DataType::String),
            Column::calculated("Stored", DataType::String, "\"x\""),
            Column::calculated("Computed", DataType::String, "\"y\""),
        ];
        let mut sessions = Table::new("Web Sessions");
        sessions.columns = vec![Column::data("Session ID", DataType::String)];
        let mut calc = Table::new("Calc");
        calc.partitions.push(Partition {
            name: "Calc".into(),
            source: PartitionSource::Calculated {
                expression: "Stores".into(),
            },
            ..Partition::default()
        });
        Model {
            tables: vec![stores, sessions, calc],
            ..Model::default()
        }
    }

    fn batch(columns: Vec<(&str, ArrayRef)>) -> RecordBatch {
        RecordBatch::try_from_iter(columns).unwrap()
    }

    #[test]
    fn cache_tables_map_onto_the_model_by_name() {
        let decoded = vec![
            (
                "STORES".to_owned(),
                batch(vec![
                    (
                        "stored",
                        Arc::new(StringArray::from(vec!["s1", "s2"])) as ArrayRef,
                    ),
                    ("store id", Arc::new(StringArray::from(vec!["a", "b"]))),
                    ("Count", Arc::new(Int32Array::from(vec![1, 2]))),
                ]),
            ),
            (
                "Old".to_owned(),
                batch(vec![("x", Arc::new(Int32Array::from(vec![1])) as ArrayRef)]),
            ),
        ];
        let m = model();
        let data = map_cache(&m, "M.SemanticModel/.pbi/cache.abf", decoded, Vec::new());
        let stores = data.tables[0].as_ref().unwrap();
        let names: Vec<&str> = stores
            .batch
            .schema_ref()
            .fields()
            .iter()
            .map(|f| f.name().as_str())
            .collect();
        assert_eq!(names, ["Store ID", "Count", "New", "Stored"]);
        assert_eq!(stores.batch.num_rows(), 2);
        assert_eq!(
            stores.batch.column(1).as_primitive::<Int64Type>().value(1),
            2
        );
        assert!(stores.batch.column(2).is_null(0));
        assert_eq!(stores.missing, ["New"]);
        assert_eq!(stores.stored, [false, false, false, true, false]);
        assert!(data.tables[1].is_none() && data.tables[2].is_none());
        let messages: Vec<&str> = data
            .diagnostics
            .iter()
            .map(|d| d.message.as_str())
            .collect();
        assert!(messages.iter().any(|m| m.contains("'Stores'[New] is not")));
        assert!(messages.iter().any(|m| m.contains("'Web Sessions' is not")));
        assert!(messages.iter().any(|m| m.contains("not in the model: Old")));

        let plan = data.plan(&m);
        assert_eq!(plan.sources[0], Source::Cache);
        assert!(
            matches!(&plan.sources[1], Source::NotCached(msg) if msg.contains("'Web Sessions'"))
        );
        assert_eq!(plan.sources[2], Source::Calculated);
        use crate::refresh::plan::Node;
        assert!(plan.index(Node::Column(0, 3)).is_none());
        assert!(plan.index(Node::Column(0, 4)).is_some());
    }
}
