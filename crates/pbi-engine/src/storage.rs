//! Columnar table storage: one Arrow [`RecordBatch`] per table plus lazily
//! built, VertiPaq-style dictionary encodings ([`EncodedColumn`]) used by the
//! storage-engine primitives.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, OnceLock};

use arrow::array::{Array, ArrayRef, AsArray};
use arrow::compute::concat_batches;
use arrow::datatypes::{DataType as ArrowType, Field, Schema, SchemaRef};
use arrow::record_batch::{RecordBatch, RecordBatchOptions};
use indexmap::IndexMap;
use pbi_model::names::name_key;
use pbi_model::{ColumnKind, Model};

use crate::error::{EngineError, Result};
use crate::value::{Value, ValueKey, arrow_type, read_canonical, to_canonical};

/// Maximum rows per table (row indexes are stored as `u32`; `u32::MAX` is
/// reserved as "no row").
pub const MAX_ROWS: usize = u32::MAX as usize - 1;

/// A dictionary-encoded column: distinct values plus one dictionary id per
/// row.
///
/// Like VertiPaq dictionaries, encoding is **case-insensitive** for text (and
/// merges numerically equal numbers): the first spelling seen becomes the
/// dictionary value, so `"abc"` and `"ABC"` rows both read back as `"abc"`.
/// Ids are assigned in order of first appearance.
pub struct EncodedColumn {
    values: Vec<Value>,
    ids: Vec<u32>,
    index: HashMap<ValueKey, u32>,
    blank_id: Option<u32>,
    /// Rows per dictionary id (CSR), built on first use.
    postings: OnceLock<Postings>,
}

/// Rows of every dictionary id, in CSR form: the rows of id `i` are
/// `rows[offsets[i]..offsets[i + 1]]`, ascending.
struct Postings {
    offsets: Vec<u32>,
    rows: Vec<u32>,
}

impl fmt::Debug for EncodedColumn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EncodedColumn")
            .field("rows", &self.ids.len())
            .field("cardinality", &self.values.len())
            .finish()
    }
}

impl EncodedColumn {
    /// Encodes an Arrow array (any type accepted by
    /// [`crate::value::canonical_type`]).
    pub fn build(array: &ArrayRef) -> Result<Self> {
        let canonical = to_canonical(array)?;
        let rows = canonical.len();
        if rows > MAX_ROWS {
            return Err(EngineError::TooManyRows {
                table: String::new(),
                rows,
                max: MAX_ROWS,
            });
        }
        let mut enc = EncodedColumn {
            values: Vec::new(),
            ids: Vec::with_capacity(rows),
            index: HashMap::new(),
            blank_id: None,
            postings: OnceLock::new(),
        };
        if let Some(strings) = canonical.as_string_opt::<i32>() {
            // Hash exact spellings first so each distinct string is folded once.
            let mut exact: HashMap<&str, u32> = HashMap::new();
            for row in 0..rows {
                let id = if strings.is_null(row) {
                    enc.intern(Value::Blank, ValueKey::Blank)?
                } else {
                    let text = strings.value(row);
                    match exact.get(text) {
                        Some(id) => *id,
                        None => {
                            let id = enc.intern(Value::from(text), ValueKey::text(text))?;
                            exact.insert(text, id);
                            id
                        }
                    }
                };
                enc.ids.push(id);
            }
        } else {
            for row in 0..rows {
                let value = read_canonical(canonical.as_ref(), row)?;
                let key = ValueKey::from(&value);
                let id = enc.intern(value, key)?;
                enc.ids.push(id);
            }
        }
        Ok(enc)
    }

    fn intern(&mut self, value: Value, key: ValueKey) -> Result<u32> {
        if let Some(id) = self.index.get(&key) {
            return Ok(*id);
        }
        let id = u32::try_from(self.values.len())
            .map_err(|_| EngineError::InvalidArgument("dictionary too large".into()))?;
        if value.is_blank() {
            self.blank_id = Some(id);
        }
        self.values.push(value);
        self.index.insert(key, id);
        Ok(id)
    }

    /// Number of rows.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Number of distinct values (including BLANK if present).
    pub fn cardinality(&self) -> usize {
        self.values.len()
    }

    /// Dictionary: distinct values indexed by id.
    pub fn values(&self) -> &[Value] {
        &self.values
    }

    /// Dictionary id of every row.
    pub fn ids(&self) -> &[u32] {
        &self.ids
    }

    /// Dictionary id of `row`.
    pub fn id_at(&self, row: usize) -> Option<u32> {
        self.ids.get(row).copied()
    }

    /// Value of `row` (dictionary spelling).
    pub fn value_at(&self, row: usize) -> Option<&Value> {
        self.values.get(*self.ids.get(row)? as usize)
    }

    /// Value of a dictionary id.
    pub fn value_of(&self, id: u32) -> Option<&Value> {
        self.values.get(id as usize)
    }

    /// Dictionary id of a key.
    pub fn id_of(&self, key: &ValueKey) -> Option<u32> {
        self.index.get(key).copied()
    }

    /// Dictionary id of BLANK, if the column has blanks.
    pub fn blank_id(&self) -> Option<u32> {
        self.blank_id
    }

    /// Rows whose value has dictionary id `id`, ascending (an inverted index
    /// built on first use, then cached).
    pub fn rows_of(&self, id: u32) -> &[u32] {
        let p = self.postings.get_or_init(|| self.build_postings());
        let i = id as usize;
        let (Some(&start), Some(&end)) = (p.offsets.get(i), p.offsets.get(i + 1)) else {
            return &[];
        };
        p.rows.get(start as usize..end as usize).unwrap_or_default()
    }

    fn build_postings(&self) -> Postings {
        let mut offsets = vec![0u32; self.values.len() + 1];
        for id in &self.ids {
            if let Some(c) = offsets.get_mut(*id as usize + 1) {
                *c += 1;
            }
        }
        let mut running = 0u32;
        for c in &mut offsets {
            running = running.saturating_add(*c);
            *c = running;
        }
        let mut cursor = offsets.clone();
        let mut rows = vec![0u32; self.ids.len()];
        for (row, id) in self.ids.iter().enumerate() {
            if let Some(pos) = cursor.get_mut(*id as usize) {
                if let Some(slot) = rows.get_mut(*pos as usize) {
                    // Row counts are capped at `MAX_ROWS` (< u32::MAX).
                    *slot = u32::try_from(row).unwrap_or(u32::MAX);
                }
                *pos += 1;
            }
        }
        Postings { offsets, rows }
    }

    /// Number of rows per dictionary id, counting only `rows` (all rows when
    /// `None`).
    pub fn counts(&self, rows: Option<&crate::rowset::RowSet>) -> Vec<u64> {
        let mut counts = vec![0u64; self.values.len()];
        let mut bump = |row: usize| {
            if let Some(c) = self
                .ids
                .get(row)
                .and_then(|id| counts.get_mut(*id as usize))
            {
                *c += 1;
            }
        };
        match rows {
            Some(rows) => rows.iter().for_each(&mut bump),
            None => (0..self.ids.len()).for_each(&mut bump),
        }
        counts
    }
}

/// One loaded table.
pub struct TableData {
    name: String,
    batch: RecordBatch,
    column_index: HashMap<String, usize>,
    encoded: Vec<OnceLock<Arc<EncodedColumn>>>,
    loaded: bool,
}

impl fmt::Debug for TableData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableData")
            .field("name", &self.name)
            .field("rows", &self.batch.num_rows())
            .field("columns", &self.column_names().collect::<Vec<_>>())
            .field("loaded", &self.loaded)
            .finish()
    }
}

impl TableData {
    /// A loaded table. Column names must be unique (case-insensitively).
    pub fn new(name: impl Into<String>, batch: RecordBatch) -> Result<Self> {
        Self::with_flag(name.into(), batch, true)
    }

    fn with_flag(name: String, batch: RecordBatch, loaded: bool) -> Result<Self> {
        if batch.num_rows() > MAX_ROWS {
            return Err(EngineError::TooManyRows {
                table: name,
                rows: batch.num_rows(),
                max: MAX_ROWS,
            });
        }
        let mut column_index = HashMap::new();
        for (i, field) in batch.schema().fields().iter().enumerate() {
            if column_index.insert(name_key(field.name()), i).is_some() {
                return Err(EngineError::DuplicateColumn {
                    table: name,
                    column: field.name().clone(),
                });
            }
        }
        let encoded = (0..batch.num_columns()).map(|_| OnceLock::new()).collect();
        Ok(Self {
            name,
            batch,
            column_index,
            encoded,
            loaded,
        })
    }

    /// Empty table with the model's column names and types (calculated and
    /// data columns; row-number columns skipped; duplicate names keep the
    /// first).
    fn from_model_table(table: &pbi_model::Table) -> Self {
        let mut seen = std::collections::HashSet::new();
        let fields: Vec<Field> = table
            .columns
            .iter()
            .filter(|c| !matches!(c.kind, ColumnKind::RowNumber))
            .filter(|c| seen.insert(name_key(&c.name)))
            .map(|c| Field::new(&c.name, arrow_type(&c.data_type), true))
            .collect();
        let batch = RecordBatch::new_empty(Arc::new(Schema::new(fields)));
        let column_index = batch
            .schema()
            .fields()
            .iter()
            .enumerate()
            .map(|(i, f)| (name_key(f.name()), i))
            .collect();
        Self {
            name: table.name.clone(),
            encoded: (0..batch.num_columns()).map(|_| OnceLock::new()).collect(),
            batch,
            column_index,
            loaded: false,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn schema(&self) -> SchemaRef {
        self.batch.schema()
    }

    pub fn batch(&self) -> &RecordBatch {
        &self.batch
    }

    pub fn row_count(&self) -> usize {
        self.batch.num_rows()
    }

    /// `false` for model tables that have not been loaded yet (empty, with
    /// the model schema).
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn column_names(&self) -> impl Iterator<Item = &str> {
        self.batch
            .schema_ref()
            .fields()
            .iter()
            .map(|f| f.name().as_str())
    }

    /// Column position by name (case-insensitive).
    pub fn column_index(&self, column: &str) -> Option<usize> {
        self.column_index.get(&name_key(column)).copied()
    }

    pub fn has_column(&self, column: &str) -> bool {
        self.column_index(column).is_some()
    }

    fn require_index(&self, column: &str) -> Result<usize> {
        self.column_index(column)
            .ok_or_else(|| EngineError::ColumnNotFound {
                table: self.name.clone(),
                column: column.to_owned(),
            })
    }

    /// Stored name of a column (case-insensitive lookup).
    pub fn column_name(&self, column: &str) -> Result<&str> {
        let i = self.require_index(column)?;
        Ok(self.batch.schema_ref().field(i).name())
    }

    /// Arrow type of a column.
    pub fn column_type(&self, column: &str) -> Result<&ArrowType> {
        let i = self.require_index(column)?;
        Ok(self.batch.schema_ref().field(i).data_type())
    }

    /// The column's Arrow array.
    pub fn column(&self, column: &str) -> Result<&ArrayRef> {
        let i = self.require_index(column)?;
        Ok(self.batch.column(i))
    }

    /// Dictionary encoding of a column (built on first use, then cached).
    pub fn encoded(&self, column: &str) -> Result<Arc<EncodedColumn>> {
        let i = self.require_index(column)?;
        self.encoded_at(i)
    }

    /// Dictionary encoding of the column at position `index`.
    pub fn encoded_at(&self, index: usize) -> Result<Arc<EncodedColumn>> {
        let (Some(cell), Some(array)) = (self.encoded.get(index), self.batch.columns().get(index))
        else {
            return Err(EngineError::InvalidArgument(format!(
                "column index {index} out of range for table '{}'",
                self.name
            )));
        };
        if let Some(enc) = cell.get() {
            return Ok(Arc::clone(enc));
        }
        let enc = Arc::new(EncodedColumn::build(array).map_err(|e| match e {
            EngineError::UnsupportedType { data_type, .. } => EngineError::UnsupportedType {
                data_type,
                context: format!(
                    "column '{}'[{}]",
                    self.name,
                    self.batch.schema_ref().field(index).name()
                ),
            },
            other => other,
        })?);
        // Another thread may have won the race; both encodings are identical.
        let _ = cell.set(Arc::clone(&enc));
        Ok(enc)
    }

    /// Value of `column` at `row` (dictionary spelling, see [`EncodedColumn`]).
    pub fn value_at(&self, column: &str, row: usize) -> Result<Value> {
        let enc = self.encoded(column)?;
        enc.value_at(row)
            .cloned()
            .ok_or_else(|| EngineError::RowOutOfRange {
                table: self.name.clone(),
                row,
                rows: self.row_count(),
            })
    }

    /// A copy with `column` added, or replaced if a column with that name
    /// exists (keeping its position and stored name).
    fn with_column(&self, column: &str, array: ArrayRef) -> Result<Self> {
        let rows = self.row_count();
        if array.len() != rows {
            return Err(EngineError::RowCountMismatch {
                table: self.name.clone(),
                column: column.to_owned(),
                expected: rows,
                actual: array.len(),
            });
        }
        let schema = self.batch.schema();
        let mut fields: Vec<Field> = schema.fields().iter().map(|f| f.as_ref().clone()).collect();
        let mut columns = self.batch.columns().to_vec();
        let mut encoded = self.encoded.clone();
        match self.column_index(column) {
            Some(i) => {
                let name = fields.get(i).map(|f| f.name().clone()).unwrap_or_default();
                if let (Some(f), Some(c), Some(e)) =
                    (fields.get_mut(i), columns.get_mut(i), encoded.get_mut(i))
                {
                    *f = Field::new(name, array.data_type().clone(), true);
                    *c = array;
                    *e = OnceLock::new();
                }
            }
            None => {
                fields.push(Field::new(column, array.data_type().clone(), true));
                columns.push(array);
                encoded.push(OnceLock::new());
            }
        }
        let options = RecordBatchOptions::new().with_row_count(Some(rows));
        let batch =
            RecordBatch::try_new_with_options(Arc::new(Schema::new(fields)), columns, &options)?;
        let mut out = Self::with_flag(self.name.clone(), batch, self.loaded)?;
        out.encoded = encoded;
        Ok(out)
    }

    /// A copy without the column at `index`, and that column.
    fn without_column(&self, index: usize) -> Result<(Self, DetachedColumn)> {
        let schema = self.batch.schema();
        let (Some(field), Some(array), Some(encoded)) = (
            schema.fields().get(index),
            self.batch.columns().get(index),
            self.encoded.get(index),
        ) else {
            return Err(EngineError::InvalidArgument(format!(
                "column index {index} out of range for table '{}'",
                self.name
            )));
        };
        let column = DetachedColumn {
            index,
            field: field.as_ref().clone(),
            array: Arc::clone(array),
            encoded: encoded.clone(),
        };
        let keep = |i: &usize| *i != index;
        let fields: Vec<Field> = (0..schema.fields().len())
            .filter(keep)
            .filter_map(|i| schema.fields().get(i).map(|f| f.as_ref().clone()))
            .collect();
        let columns: Vec<ArrayRef> = (0..self.batch.num_columns())
            .filter(keep)
            .filter_map(|i| self.batch.columns().get(i).cloned())
            .collect();
        let encoded = (0..self.encoded.len())
            .filter(keep)
            .filter_map(|i| self.encoded.get(i).cloned())
            .collect();
        let out = self.rebuilt(fields, columns, encoded)?;
        Ok((out, column))
    }

    /// A copy with a detached column put back at its old position (or last),
    /// named `name`.
    fn with_detached(&self, name: &str, column: DetachedColumn) -> Result<Self> {
        if self.has_column(name) {
            return Err(EngineError::DuplicateColumn {
                table: self.name.clone(),
                column: name.to_owned(),
            });
        }
        if column.array.len() != self.row_count() {
            return Err(EngineError::RowCountMismatch {
                table: self.name.clone(),
                column: name.to_owned(),
                expected: self.row_count(),
                actual: column.array.len(),
            });
        }
        let schema = self.batch.schema();
        let mut fields: Vec<Field> = schema.fields().iter().map(|f| f.as_ref().clone()).collect();
        let mut columns = self.batch.columns().to_vec();
        let mut encoded = self.encoded.clone();
        let at = column.index.min(fields.len());
        fields.insert(at, column.field.with_name(name));
        columns.insert(at, column.array);
        encoded.insert(at, column.encoded);
        self.rebuilt(fields, columns, encoded)
    }

    /// A copy with new fields, columns and encodings (same name, row count,
    /// schema metadata and loaded flag).
    fn rebuilt(
        &self,
        fields: Vec<Field>,
        columns: Vec<ArrayRef>,
        encoded: Vec<OnceLock<Arc<EncodedColumn>>>,
    ) -> Result<Self> {
        let schema = self.batch.schema();
        let options = RecordBatchOptions::new().with_row_count(Some(self.row_count()));
        let batch = RecordBatch::try_new_with_options(
            Arc::new(Schema::new_with_metadata(fields, schema.metadata().clone())),
            columns,
            &options,
        )?;
        let mut out = Self::with_flag(self.name.clone(), batch, self.loaded)?;
        out.encoded = encoded;
        Ok(out)
    }

    /// A copy under another name (same data and cached encodings).
    fn renamed(&self, name: &str) -> Self {
        Self {
            name: name.to_owned(),
            batch: self.batch.clone(),
            column_index: self.column_index.clone(),
            encoded: self.encoded.clone(),
            loaded: self.loaded,
        }
    }

    /// A copy with columns renamed simultaneously (`(old, new)` pairs; a
    /// swap works). Missing columns are ignored; data, order and cached
    /// encodings are kept.
    fn with_renamed_columns(&self, renames: &[(&str, &str)]) -> Result<Self> {
        let schema = self.batch.schema();
        let mut fields: Vec<Field> = schema.fields().iter().map(|f| f.as_ref().clone()).collect();
        let mut changed = false;
        for (old, new) in renames {
            let Some(field) = self.column_index(old).and_then(|i| fields.get_mut(i)) else {
                continue;
            };
            if field.name() != new {
                *field = field.clone().with_name(*new);
                changed = true;
            }
        }
        if !changed {
            return Ok(self.renamed(&self.name));
        }
        let options = RecordBatchOptions::new().with_row_count(Some(self.row_count()));
        let batch = RecordBatch::try_new_with_options(
            Arc::new(Schema::new_with_metadata(fields, schema.metadata().clone())),
            self.batch.columns().to_vec(),
            &options,
        )?;
        let mut out = Self::with_flag(self.name.clone(), batch, self.loaded)?;
        out.encoded = self.encoded.clone();
        Ok(out)
    }
}

/// A table or column rename applied to stored data ([`Storage::rename`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageRename {
    Table {
        old: String,
        new: String,
    },
    /// `table` is the table's name after the table renames of the batch.
    Column {
        table: String,
        old: String,
        new: String,
    },
}

/// A stored table or column, by name ([`Storage::detach`],
/// [`Storage::attach`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredObject {
    Table(String),
    Column { table: String, column: String },
}

/// A table or column taken out of [`Storage`] with its data and cached
/// encodings ([`Storage::detach`]), to put back unchanged
/// ([`Storage::attach`]), e.g. when the delete of a model object is undone.
/// Cloning is cheap (shared arrays).
#[derive(Debug, Clone)]
pub struct Detached(DetachedData);

#[derive(Debug, Clone)]
enum DetachedData {
    Table { index: usize, data: Arc<TableData> },
    Column(DetachedColumn),
}

#[derive(Debug, Clone)]
struct DetachedColumn {
    index: usize,
    field: Field,
    array: ArrayRef,
    encoded: OnceLock<Arc<EncodedColumn>>,
}

impl Detached {
    /// Rows of the detached table or column.
    #[must_use]
    pub fn rows(&self) -> usize {
        match &self.0 {
            DetachedData::Table { data, .. } => data.row_count(),
            DetachedData::Column(c) => c.array.len(),
        }
    }

    /// `true` for a table, `false` for a column.
    #[must_use]
    pub fn is_table(&self) -> bool {
        matches!(self.0, DetachedData::Table { .. })
    }
}

/// All tables of a model, keyed by case-insensitive name. Cloning is cheap
/// (tables are shared `Arc`s), which is how engine snapshots stay immutable.
#[derive(Debug, Clone, Default)]
pub struct Storage {
    tables: IndexMap<String, Arc<TableData>>,
}

impl Storage {
    pub fn new() -> Self {
        Self::default()
    }

    /// Empty tables (model schema, zero rows) for every model table.
    pub fn from_model(model: &Model) -> Self {
        let mut storage = Self::new();
        for table in &model.tables {
            let key = name_key(&table.name);
            if !storage.tables.contains_key(&key) {
                storage
                    .tables
                    .insert(key, Arc::new(TableData::from_model_table(table)));
            }
        }
        storage
    }

    /// Adds an empty model-schema table if `table` is not stored yet.
    pub(crate) fn ensure_model_table(&mut self, table: &pbi_model::Table) {
        let key = name_key(&table.name);
        if !self.tables.contains_key(&key) {
            self.tables
                .insert(key, Arc::new(TableData::from_model_table(table)));
        }
    }

    /// Loads (or reloads) a table. A table that already exists keeps its
    /// stored name spelling. The batch is stored as-is; its schema becomes
    /// the table's schema.
    pub fn load_table(&mut self, name: &str, batch: RecordBatch) -> Result<()> {
        let key = name_key(name);
        let stored_name = self
            .tables
            .get(&key)
            .map_or_else(|| name.to_owned(), |t| t.name.clone());
        let data = TableData::new(stored_name, batch)?;
        self.tables.insert(key, Arc::new(data));
        Ok(())
    }

    /// Loads a table from several batches (concatenated into one).
    pub fn load_batches(
        &mut self,
        name: &str,
        schema: SchemaRef,
        batches: &[RecordBatch],
    ) -> Result<()> {
        let batch = concat_batches(&schema, batches)?;
        self.load_table(name, batch)
    }

    /// Replaces the data of an existing table.
    pub fn replace_table(&mut self, name: &str, batch: RecordBatch) -> Result<()> {
        if !self.contains(name) {
            return Err(EngineError::TableNotFound(name.to_owned()));
        }
        self.load_table(name, batch)
    }

    /// Keeps only `rows` of a table (row-level security), preserving its
    /// loaded flag.
    pub fn restrict_table(&mut self, name: &str, rows: &crate::rowset::RowSet) -> Result<()> {
        let key = name_key(name);
        let data = self
            .tables
            .get(&key)
            .ok_or_else(|| EngineError::TableNotFound(name.to_owned()))?;
        let mask = rows.resized(data.row_count()).to_boolean_array();
        let batch = arrow::compute::filter_record_batch(data.batch(), &mask)?;
        let restricted = TableData::with_flag(data.name.clone(), batch, data.loaded)?;
        self.tables.insert(key, Arc::new(restricted));
        Ok(())
    }

    /// Adds a column (e.g. a calculated column) to a table, or replaces the
    /// column with the same name. The array must have the table's row count.
    pub fn add_column(&mut self, table: &str, column: &str, array: ArrayRef) -> Result<()> {
        let key = name_key(table);
        let data = self
            .tables
            .get(&key)
            .ok_or_else(|| EngineError::TableNotFound(table.to_owned()))?;
        let updated = data.with_column(column, array)?;
        self.tables.insert(key, Arc::new(updated));
        Ok(())
    }

    /// Removes a table.
    pub fn remove_table(&mut self, name: &str) -> Option<Arc<TableData>> {
        self.tables.shift_remove(&name_key(name))
    }

    /// Renames tables, then columns, keeping data, table order and cached
    /// encodings. Renames within a batch are simultaneous (swapping two names
    /// works); renames of tables or columns that are not stored are ignored.
    ///
    /// # Errors
    /// A new name collides with another table or column; nothing changes.
    pub fn rename(&mut self, renames: &[StorageRename]) -> Result<()> {
        let mut table_renames: HashMap<String, &str> = HashMap::new();
        let mut column_renames: IndexMap<String, Vec<(&str, &str)>> = IndexMap::new();
        for rename in renames {
            match rename {
                StorageRename::Table { old, new } => {
                    table_renames.insert(name_key(old), new);
                }
                StorageRename::Column { table, old, new } => column_renames
                    .entry(name_key(table))
                    .or_default()
                    .push((old, new)),
            }
        }
        let mut tables = IndexMap::with_capacity(self.tables.len());
        for (key, data) in &self.tables {
            let (key, data) = match table_renames.get(key) {
                Some(new) => (name_key(new), Arc::new(data.renamed(new))),
                None => (key.clone(), Arc::clone(data)),
            };
            let name = data.name.clone();
            if tables.insert(key, data).is_some() {
                return Err(EngineError::DuplicateTable(name));
            }
        }
        for (key, columns) in column_renames {
            if let Some(data) = tables.get_mut(&key) {
                *data = Arc::new(data.with_renamed_columns(&columns)?);
            }
        }
        self.tables = tables;
        Ok(())
    }

    /// Takes a table or column out of storage, keeping its data and cached
    /// encodings; `None` if it is not stored.
    ///
    /// # Errors
    /// Never in practice (an inconsistent column index).
    pub fn detach(&mut self, object: &StoredObject) -> Result<Option<Detached>> {
        match object {
            StoredObject::Table(name) => {
                let key = name_key(name);
                let Some(index) = self.tables.get_index_of(&key) else {
                    return Ok(None);
                };
                Ok(self
                    .tables
                    .shift_remove(&key)
                    .map(|data| Detached(DetachedData::Table { index, data })))
            }
            StoredObject::Column { table, column } => {
                let key = name_key(table);
                let Some(data) = self.tables.get(&key) else {
                    return Ok(None);
                };
                let Some(index) = data.column_index(column) else {
                    return Ok(None);
                };
                let (rest, detached) = data.without_column(index)?;
                self.tables.insert(key, Arc::new(rest));
                Ok(Some(Detached(DetachedData::Column(detached))))
            }
        }
    }

    /// Puts a detached table or column back under the name `object` (its
    /// name now) at its old position, or last when there are fewer tables or
    /// columns now.
    ///
    /// # Errors
    /// The name is taken, a column's table is not stored or has another row
    /// count, or `object` and `detached` are not the same kind; nothing
    /// changes.
    pub fn attach(&mut self, object: &StoredObject, detached: Detached) -> Result<()> {
        match (object, detached.0) {
            (StoredObject::Table(name), DetachedData::Table { index, data }) => {
                let key = name_key(name);
                if self.tables.contains_key(&key) {
                    return Err(EngineError::DuplicateTable(name.clone()));
                }
                let data = if data.name == *name {
                    data
                } else {
                    Arc::new(data.renamed(name))
                };
                let at = index.min(self.tables.len());
                self.tables.shift_insert(at, key, data);
                Ok(())
            }
            (StoredObject::Column { table, column }, DetachedData::Column(detached)) => {
                let key = name_key(table);
                let data = self
                    .tables
                    .get(&key)
                    .ok_or_else(|| EngineError::TableNotFound(table.clone()))?;
                let updated = data.with_detached(column, detached)?;
                self.tables.insert(key, Arc::new(updated));
                Ok(())
            }
            (object, _) => Err(EngineError::InvalidArgument(format!(
                "cannot attach a detached {} as {object:?}",
                if object_is_table(object) {
                    "column"
                } else {
                    "table"
                }
            ))),
        }
    }

    pub fn contains(&self, name: &str) -> bool {
        self.tables.contains_key(&name_key(name))
    }

    pub fn get(&self, name: &str) -> Option<&Arc<TableData>> {
        self.tables.get(&name_key(name))
    }

    pub fn table(&self, name: &str) -> Result<&Arc<TableData>> {
        self.get(name)
            .ok_or_else(|| EngineError::TableNotFound(name.to_owned()))
    }

    pub fn tables(&self) -> impl Iterator<Item = &Arc<TableData>> {
        self.tables.values()
    }

    pub fn table_names(&self) -> impl Iterator<Item = &str> {
        self.tables.values().map(|t| t.name())
    }

    pub fn row_count(&self, table: &str) -> Result<usize> {
        Ok(self.table(table)?.row_count())
    }

    pub fn column(&self, table: &str, column: &str) -> Result<ArrayRef> {
        Ok(Arc::clone(self.table(table)?.column(column)?))
    }

    pub fn value_at(&self, table: &str, column: &str, row: usize) -> Result<Value> {
        self.table(table)?.value_at(column, row)
    }

    pub fn encoded(&self, table: &str, column: &str) -> Result<Arc<EncodedColumn>> {
        self.table(table)?.encoded(column)
    }
}

fn object_is_table(object: &StoredObject) -> bool {
    matches!(object, StoredObject::Table(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Int64Array, StringArray};
    use pbi_model::{Column, DataType, Table};

    fn batch() -> RecordBatch {
        RecordBatch::try_from_iter([
            (
                "Key",
                Arc::new(StringArray::from(vec![
                    Some("a"),
                    Some("A"),
                    None,
                    Some("b"),
                ])) as ArrayRef,
            ),
            (
                "N",
                Arc::new(Int64Array::from(vec![1, 2, 3, 4])) as ArrayRef,
            ),
        ])
        .unwrap()
    }

    #[test]
    fn model_tables_start_empty_with_model_schema() {
        let mut t = Table::new("Stores");
        t.columns.push(Column::data("Store ID", DataType::String));
        t.columns
            .push(Column::calculated("Order Count", DataType::Int64, "1"));
        let model = Model {
            tables: vec![t],
            ..Model::default()
        };
        let s = Storage::from_model(&model);
        let stores = s.table("stores").unwrap();
        assert!(!stores.is_loaded());
        assert_eq!(stores.row_count(), 0);
        assert_eq!(
            stores.column_type("ORDER COUNT").unwrap(),
            &ArrowType::Int64
        );
        assert!(matches!(
            s.table("Nope"),
            Err(EngineError::TableNotFound(_))
        ));
    }

    #[test]
    fn encoding_is_case_insensitive_and_values_are_readable() {
        let mut s = Storage::new();
        s.load_table("T", batch()).unwrap();
        let enc = s.encoded("t", "key").unwrap();
        assert_eq!(enc.cardinality(), 3);
        assert_eq!(enc.ids(), &[0, 0, 1, 2]);
        assert_eq!(enc.blank_id(), Some(1));
        assert_eq!(s.value_at("T", "Key", 1).unwrap(), Value::from("a"));
        assert_eq!(s.value_at("T", "N", 3).unwrap(), Value::Integer(4));
        assert!(matches!(
            s.value_at("T", "N", 4),
            Err(EngineError::RowOutOfRange { .. })
        ));
        assert!(matches!(
            s.value_at("T", "Missing", 0),
            Err(EngineError::ColumnNotFound { .. })
        ));
    }

    #[test]
    fn add_column_checks_row_count_and_replaces_by_name() {
        let mut s = Storage::new();
        s.load_table("T", batch()).unwrap();
        let err = s
            .add_column("T", "C", Arc::new(Int64Array::from(vec![1])))
            .unwrap_err();
        assert!(matches!(
            err,
            EngineError::RowCountMismatch {
                expected: 4,
                actual: 1,
                ..
            }
        ));
        s.add_column("T", "C", Arc::new(Int64Array::from(vec![9, 9, 9, 9])))
            .unwrap();
        assert_eq!(s.value_at("T", "c", 0).unwrap(), Value::Integer(9));
        s.add_column("T", "c", Arc::new(StringArray::from(vec!["x"; 4])))
            .unwrap();
        let t = s.table("T").unwrap();
        assert_eq!(t.column_names().collect::<Vec<_>>(), ["Key", "N", "C"]);
        assert_eq!(s.value_at("T", "C", 0).unwrap(), Value::from("x"));
        assert!(
            s.add_column("Nope", "C", Arc::new(Int64Array::from(vec![1])))
                .is_err()
        );
    }

    #[test]
    fn duplicate_column_names_are_rejected() {
        let b = RecordBatch::try_from_iter([
            ("a", Arc::new(Int64Array::from(vec![1])) as ArrayRef),
            ("A", Arc::new(Int64Array::from(vec![2])) as ArrayRef),
        ])
        .unwrap();
        let mut s = Storage::new();
        assert!(matches!(
            s.load_table("T", b),
            Err(EngineError::DuplicateColumn { .. })
        ));
        assert!(s.replace_table("T", batch()).is_err());
    }

    #[test]
    fn rename_keeps_data_order_and_supports_swaps() {
        let mut s = Storage::new();
        s.load_table("A", batch()).unwrap();
        s.load_table("B", batch()).unwrap();
        s.load_table("C", batch()).unwrap();
        s.rename(&[
            StorageRename::Table {
                old: "a".into(),
                new: "B".into(),
            },
            StorageRename::Table {
                old: "B".into(),
                new: "A".into(),
            },
            StorageRename::Column {
                table: "b".into(),
                old: "Key".into(),
                new: "N".into(),
            },
            StorageRename::Column {
                table: "B".into(),
                old: "n".into(),
                new: "Key".into(),
            },
            StorageRename::Column {
                table: "C".into(),
                old: "Key".into(),
                new: "key".into(),
            },
            StorageRename::Table {
                old: "Missing".into(),
                new: "Other".into(),
            },
        ])
        .unwrap();
        assert_eq!(s.table_names().collect::<Vec<_>>(), ["B", "A", "C"]);
        let b = s.table("B").unwrap();
        assert!(b.is_loaded());
        assert_eq!(b.column_names().collect::<Vec<_>>(), ["N", "Key"]);
        assert_eq!(s.value_at("B", "N", 0).unwrap(), Value::from("a"));
        assert_eq!(s.value_at("B", "Key", 3).unwrap(), Value::Integer(4));
        assert_eq!(s.encoded("B", "N").unwrap().cardinality(), 3);
        assert_eq!(
            s.table("C").unwrap().column_names().collect::<Vec<_>>(),
            ["key", "N"]
        );
    }

    #[test]
    fn detach_and_attach_restore_tables_and_columns_in_place() {
        let mut s = Storage::new();
        s.load_table("A", batch()).unwrap();
        s.load_table("B", batch()).unwrap();
        s.load_table("C", batch()).unwrap();
        assert_eq!(s.encoded("A", "Key").unwrap().cardinality(), 3);
        let table = StoredObject::Table("b".into());
        let column = StoredObject::Column {
            table: "A".into(),
            column: "key".into(),
        };
        let b = s.detach(&table).unwrap().unwrap();
        let key = s.detach(&column).unwrap().unwrap();
        assert!(b.is_table() && !key.is_table());
        assert_eq!((b.rows(), key.rows()), (4, 4));
        assert!(s.detach(&table).unwrap().is_none());
        assert_eq!(s.table_names().collect::<Vec<_>>(), ["A", "C"]);
        assert_eq!(
            s.table("A").unwrap().column_names().collect::<Vec<_>>(),
            ["N"]
        );
        assert!(matches!(
            s.attach(&table, key.clone()),
            Err(EngineError::InvalidArgument(_))
        ));
        s.attach(&StoredObject::Table("B2".into()), b).unwrap();
        s.attach(&column, key).unwrap();
        assert_eq!(s.table_names().collect::<Vec<_>>(), ["A", "B2", "C"]);
        let a = s.table("A").unwrap();
        assert_eq!(a.column_names().collect::<Vec<_>>(), ["key", "N"]);
        assert_eq!(a.encoded("Key").unwrap().cardinality(), 3);
        assert_eq!(s.value_at("A", "Key", 3).unwrap(), Value::from("b"));
        assert_eq!(s.value_at("B2", "N", 3).unwrap(), Value::Integer(4));
        let n = s
            .detach(&StoredObject::Column {
                table: "C".into(),
                column: "N".into(),
            })
            .unwrap()
            .unwrap();
        assert!(matches!(
            s.attach(&column, n.clone()),
            Err(EngineError::DuplicateColumn { .. })
        ));
        s.restrict_table("A", &crate::rowset::RowSet::from_rows(4, [0, 1]))
            .unwrap();
        let other = StoredObject::Column {
            table: "A".into(),
            column: "M".into(),
        };
        assert!(matches!(
            s.attach(&other, n),
            Err(EngineError::RowCountMismatch { .. })
        ));
    }

    #[test]
    fn rename_collisions_are_rejected_without_changes() {
        let mut s = Storage::new();
        s.load_table("A", batch()).unwrap();
        s.load_table("B", batch()).unwrap();
        let err = s
            .rename(&[StorageRename::Table {
                old: "A".into(),
                new: "b".into(),
            }])
            .unwrap_err();
        assert!(matches!(err, EngineError::DuplicateTable(_)));
        let err = s
            .rename(&[StorageRename::Column {
                table: "A".into(),
                old: "Key".into(),
                new: "n".into(),
            }])
            .unwrap_err();
        assert!(matches!(err, EngineError::DuplicateColumn { .. }));
        assert_eq!(s.table_names().collect::<Vec<_>>(), ["A", "B"]);
        assert_eq!(
            s.table("A").unwrap().column_names().collect::<Vec<_>>(),
            ["Key", "N"]
        );
    }
}
