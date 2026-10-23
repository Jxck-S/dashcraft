//! DataFusion integration: ad-hoc SQL over the model tables and paging for
//! the explorer's virtualised grid. Not part of the DAX path.
//!
//! Every table is registered as a `MemTable` (zero-copy over the stored
//! batch) in the default catalog/schema. Table names resolve
//! **case-insensitively**; identifier normalisation is off, so unquoted
//! column names are case-sensitive and names with spaces must be quoted:
//! `SELECT "Store Name" FROM "Stores"`. Queries are read-only (DDL,
//! DML such as `COPY`/`INSERT`, and `SET` statements are rejected).
//! Every entry point is `async` and takes a `CancellationToken`.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};

use arrow::array::{ArrayRef, UInt64Array};
use arrow::compute::concat_batches;
use arrow::datatypes::{DataType as ArrowType, Field, Schema, SchemaRef};
use arrow::record_batch::{RecordBatch, RecordBatchOptions};
use datafusion::catalog::{SchemaProvider, TableProvider};
use datafusion::datasource::MemTable;
use datafusion::error::DataFusionError;
use datafusion::prelude::{SQLOptions, SessionConfig, SessionContext, ident, lower};
use indexmap::IndexMap;
use pbi_model::names::name_key;
use tokio_util::sync::CancellationToken;

use crate::engine::EngineSnapshot;
use crate::error::{EngineError, Result};
use crate::storage::{Storage, TableData};

/// Hidden row-number column used to make grid paging deterministic.
pub const ROW_ID_COLUMN: &str = "__pbi_row_id";

/// Sort order of a grid page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortSpec {
    pub column: String,
    pub ascending: bool,
}

/// A page of a table for the explorer grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRequest {
    pub table: String,
    pub offset: usize,
    pub limit: usize,
    /// Sort column (BLANKs first when ascending, text case-insensitive);
    /// rows with equal keys keep storage order.
    pub sort: Option<SortSpec>,
    /// SQL boolean expression over the table's columns, e.g.
    /// `"Region" = 'West'`.
    pub filter: Option<String>,
}

impl PageRequest {
    pub fn new(table: impl Into<String>, offset: usize, limit: usize) -> Self {
        Self {
            table: table.into(),
            offset,
            limit,
            sort: None,
            filter: None,
        }
    }

    pub fn sort_by(mut self, column: impl Into<String>, ascending: bool) -> Self {
        self.sort = Some(SortSpec {
            column: column.into(),
            ascending,
        });
        self
    }

    pub fn filter(mut self, expression: impl Into<String>) -> Self {
        self.filter = Some(expression.into());
        self
    }
}

/// Per-snapshot DataFusion state (built lazily).
#[derive(Default)]
pub(crate) struct SqlState {
    session: OnceLock<SessionContext>,
    paged: Mutex<HashMap<String, Arc<dyn TableProvider>>>,
}

/// Schema provider resolving model table names case-insensitively.
struct ModelSchema {
    tables: IndexMap<String, (String, Arc<dyn TableProvider>)>,
}

impl fmt::Debug for ModelSchema {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.tables.values().map(|(name, _)| name))
            .finish()
    }
}

type TableFuture<'a> = Pin<
    Box<dyn Future<Output = Result<Option<Arc<dyn TableProvider>>, DataFusionError>> + Send + 'a>,
>;

impl SchemaProvider for ModelSchema {
    fn table_names(&self) -> Vec<String> {
        self.tables.values().map(|(name, _)| name.clone()).collect()
    }

    // Hand-expanded `#[async_trait]` method (avoids an extra dependency).
    fn table<'life0, 'life1, 'async_trait>(
        &'life0 self,
        name: &'life1 str,
    ) -> TableFuture<'async_trait>
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        let found = self
            .tables
            .get(&name_key(name))
            .map(|(_, provider)| Arc::clone(provider));
        Box::pin(async move { Ok(found) })
    }

    fn table_exist(&self, name: &str) -> bool {
        self.tables.contains_key(&name_key(name))
    }
}

fn build_session(storage: &Storage) -> Result<SessionContext> {
    let mut config = SessionConfig::new().with_information_schema(true);
    config.options_mut().sql_parser.enable_ident_normalization = false;
    let catalog_name = config.options().catalog.default_catalog.clone();
    let schema_name = config.options().catalog.default_schema.clone();
    let ctx = SessionContext::new_with_config(config);
    let mut tables = IndexMap::new();
    for table in storage.tables() {
        let provider = MemTable::try_new(table.schema(), vec![vec![table.batch().clone()]])?;
        tables.insert(
            name_key(table.name()),
            (
                table.name().to_owned(),
                Arc::new(provider) as Arc<dyn TableProvider>,
            ),
        );
    }
    let catalog = ctx.catalog(&catalog_name).ok_or_else(|| {
        EngineError::InvalidArgument(format!("DataFusion catalog '{catalog_name}' missing"))
    })?;
    catalog.register_schema(&schema_name, Arc::new(ModelSchema { tables }))?;
    Ok(ctx)
}

/// Runs `fut` unless/until `cancel` fires; dropping the DataFusion future
/// cancels the query.
async fn cancellable<T>(
    cancel: &CancellationToken,
    fut: impl Future<Output = Result<T>>,
) -> Result<T> {
    if cancel.is_cancelled() {
        return Err(EngineError::Cancelled);
    }
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(EngineError::Cancelled),
        result = fut => result,
    }
}

fn is_text(t: &ArrowType) -> bool {
    matches!(
        t,
        ArrowType::Utf8 | ArrowType::LargeUtf8 | ArrowType::Utf8View
    )
}

impl EngineSnapshot {
    /// A DataFusion session with every table registered (for `pbi-dax`
    /// plans and tools). Each call returns an independent session state over
    /// the shared, read-only model schema, so configuration changes made by
    /// one caller do not leak to others.
    pub fn session(&self) -> Result<SessionContext> {
        let shared = match self.sql.session.get() {
            Some(ctx) => ctx,
            None => {
                let ctx = build_session(self.storage())?;
                let _ = self.sql.session.set(ctx);
                self.sql.session.get().ok_or_else(|| {
                    EngineError::InvalidArgument("DataFusion session unavailable".into())
                })?
            }
        };
        Ok(SessionContext::new_with_state(shared.state()))
    }

    /// Runs a read-only SQL query.
    pub async fn sql(&self, query: &str, cancel: CancellationToken) -> Result<Vec<RecordBatch>> {
        let ctx = self.session()?;
        let options = SQLOptions::new()
            .with_allow_ddl(false)
            .with_allow_dml(false)
            .with_allow_statements(false);
        cancellable(&cancel, async move {
            let df = ctx.sql_with_options(query, options).await?;
            Ok(df.collect().await?)
        })
        .await
    }

    /// One page of a table (optionally filtered and sorted) as a single
    /// batch with the table's columns.
    pub async fn page(
        &self,
        request: &PageRequest,
        cancel: CancellationToken,
    ) -> Result<RecordBatch> {
        let data = self.storage().table(&request.table)?;
        let columns: Vec<String> = data.column_names().map(str::to_owned).collect();
        let sort = match &request.sort {
            Some(s) => Some((
                data.column_name(&s.column)?.to_owned(),
                s.ascending,
                is_text(data.column_type(&s.column)?),
            )),
            None => None,
        };
        let provider = self.paged_table(data)?;
        let ctx = self.session()?;
        let filter = request.filter.clone();
        let (offset, limit) = (request.offset, request.limit);
        cancellable(&cancel, async move {
            let mut df = ctx.read_table(provider)?;
            if let Some(expr) = filter.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
                let predicate = df.parse_sql_expr(expr)?;
                df = df.filter(predicate)?;
            }
            let mut order = Vec::new();
            if let Some((column, ascending, text)) = &sort {
                if *text {
                    order.push(lower(ident(column)).sort(*ascending, *ascending));
                }
                order.push(ident(column).sort(*ascending, *ascending));
            }
            order.push(ident(ROW_ID_COLUMN).sort(true, false));
            let df = df
                .sort(order)?
                .limit(offset, Some(limit))?
                .select(columns.iter().map(ident))?;
            let schema: SchemaRef = Arc::clone(df.schema().inner());
            let batches = df.collect().await?;
            let schema = batches.first().map_or(schema, RecordBatch::schema);
            Ok(concat_batches(&schema, &batches)?)
        })
        .await
    }

    /// Number of rows of `table` passing `filter` (the grid's total).
    pub async fn page_row_count(
        &self,
        table: &str,
        filter: Option<&str>,
        cancel: CancellationToken,
    ) -> Result<usize> {
        let data = self.storage().table(table)?;
        let provider = self.paged_table(data)?;
        let ctx = self.session()?;
        cancellable(&cancel, async move {
            let mut df = ctx.read_table(provider)?;
            if let Some(expr) = filter.map(str::trim).filter(|f| !f.is_empty()) {
                let predicate = df.parse_sql_expr(expr)?;
                df = df.filter(predicate)?;
            }
            Ok(df.count().await?)
        })
        .await
    }

    /// The table plus a row-number column, cached per snapshot.
    fn paged_table(&self, data: &TableData) -> Result<Arc<dyn TableProvider>> {
        let key = name_key(data.name());
        if let Ok(cache) = self.sql.paged.lock()
            && let Some(provider) = cache.get(&key)
        {
            return Ok(Arc::clone(provider));
        }
        if data.has_column(ROW_ID_COLUMN) {
            return Err(EngineError::InvalidArgument(format!(
                "table '{}' has a column named {ROW_ID_COLUMN}",
                data.name()
            )));
        }
        let rows = data.row_count();
        let schema = data.schema();
        let mut fields: Vec<Field> = schema.fields().iter().map(|f| f.as_ref().clone()).collect();
        fields.push(Field::new(ROW_ID_COLUMN, ArrowType::UInt64, false));
        let mut columns = data.batch().columns().to_vec();
        columns.push(Arc::new(UInt64Array::from_iter_values(0..rows as u64)) as ArrayRef);
        let options = RecordBatchOptions::new().with_row_count(Some(rows));
        let batch =
            RecordBatch::try_new_with_options(Arc::new(Schema::new(fields)), columns, &options)?;
        let provider: Arc<dyn TableProvider> =
            Arc::new(MemTable::try_new(batch.schema(), vec![vec![batch]])?);
        if let Ok(mut cache) = self.sql.paged.lock() {
            cache.insert(key, Arc::clone(&provider));
        }
        Ok(provider)
    }
}
