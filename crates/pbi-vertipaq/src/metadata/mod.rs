//! Tabular model metadata: reads `metadata.sqlitedb` from an ABF backup and maps it to
//! [`pbi_model::Model`], plus a [`StorageMap`] that tells the VertiPaq decoder which
//! archive files hold each column.
//!
//! The SQLite database is opened **in memory** from the archive bytes
//! (`sqlite3_deserialize`, read-only; no temporary files) with SQLite's defensive mode,
//! untrusted-schema mode, and triggers and views disabled. Every metadata table is read
//! with `SELECT *` and columns are looked up by name, so schema versions with missing or
//! extra columns still load (missing values become `None`/defaults).
//!
//! Mapping notes:
//!
//! * Internal storage tables (`Table.SystemFlags & 1`: `H$` attribute-hierarchy, `R$`
//!   relationship and `U$` user-hierarchy tables) are not TOM tables and are skipped
//!   unless [`ModelOptions::include_system_tables`] is set. They are always present in
//!   the [`StorageMap`].
//! * Auto date/time tables (`LocalDateTable_*`, `DateTableTemplate_*`) are regular TOM
//!   tables; they are kept with their `isHidden`/`isPrivate`/`showAsVariationsOnly`
//!   flags and tagged `vertipaq.autoDateTable` in `extra`.
//! * Row-number columns are omitted unless [`ModelOptions::include_row_number_columns`].
//! * `model.rs` documents the `extra` keys (`vertipaq.systemFlags`, ...).
//!
//! Backups without `metadata.sqlitedb` but with legacy XML metadata (compatibility level
//! 1100/1103: Power Pivot workbooks, SSAS 2012/2014 tabular) load through the `xml`
//! module (schema from the ASSL object definitions, storage from the `.tbl.xml` files) ([`load_model`] falls back to it; [`LoadedModel::format`] tells which).
//!
//! References: TOM enumerations (Microsoft Learn API reference, facts only), pbixray
//! `meta/sqlite_source.py` and `docs/TMSCHEMA_MAPPING.md` (#3, MIT) for the table and
//! column names and legacy-schema fallbacks; see `REFERENCES.md`.

mod codes;
mod database;
mod error;
mod model;
mod rows;
mod storage;
#[cfg(test)]
mod tests;
mod xml;

use std::collections::HashSet;

use pbi_model::{Diagnostic, Model};
use rusqlite::Connection;
use rusqlite::config::DbConfig;

pub use codes::{ColumnKindCode, data_type};
pub use error::MetadataError;
pub use storage::{
    AttributeHierarchyStorage, ColumnPartitionFiles, ColumnStatistics, ColumnStorage,
    DictionaryEncoding, DictionaryStorage, PartitionStorage, StorageMap, TableStorage,
};

use crate::abf::AbfArchive;
use model::Catalog;
use rows::Rows;
use storage::StorageTables;

/// Upper bound on the cells (rows × columns) read from one metadata table.
pub const MAX_CELLS: u64 = 64 << 20;

const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";

/// Options for [`MetadataDb::read_model`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ModelOptions {
    /// Include internal `H$`/`R$`/`U$` storage tables (hidden, tagged
    /// `vertipaq.internalTable`). Default `false`.
    pub include_system_tables: bool,
    /// Include the internal `RowNumber-…` column of each table. Default `false`.
    pub include_row_number_columns: bool,
    /// Path used for [`pbi_model::Origin`]s (`pointer` = `/<MetadataTable>/<ID>`);
    /// `None` disables origins. Default `Some("metadata.sqlitedb")`.
    pub origin_path: Option<String>,
}

impl Default for ModelOptions {
    fn default() -> Self {
        Self {
            include_system_tables: false,
            include_row_number_columns: false,
            origin_path: Some(crate::abf::METADATA_FILE_NAME.to_owned()),
        }
    }
}

impl ModelOptions {
    /// Sets [`ModelOptions::include_system_tables`].
    #[must_use]
    pub fn with_system_tables(mut self, include: bool) -> Self {
        self.include_system_tables = include;
        self
    }

    /// Sets [`ModelOptions::include_row_number_columns`].
    #[must_use]
    pub fn with_row_number_columns(mut self, include: bool) -> Self {
        self.include_row_number_columns = include;
        self
    }

    /// Sets [`ModelOptions::origin_path`].
    #[must_use]
    pub fn with_origin_path(mut self, path: Option<String>) -> Self {
        self.origin_path = path;
        self
    }
}

/// Where a backup's model metadata came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum MetadataFormat {
    /// `metadata.sqlitedb` (compatibility level 1200+): schema and storage map.
    #[default]
    Sqlite,
    /// Legacy XML object definitions (compatibility level 1100/1103, Power Pivot):
    /// storage from the `.tbl.xml` files, segment metadata inline
    /// ([`ColumnPartitionFiles::meta`]).
    Xml,
}

/// A model read from a backup: schema, physical storage and schema version.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LoadedModel {
    /// The semantic model.
    pub model: Model,
    /// Physical storage (files per column).
    pub storage: StorageMap,
    /// `DBPROPERTIES.SCHEMAVERSION` of the metadata database.
    pub schema_version: Option<i64>,
    /// Which metadata the model was read from.
    pub format: MetadataFormat,
    /// Problems found while reading (codes in [`crate::vertipaq::codes`]).
    pub diagnostics: Vec<Diagnostic>,
}

/// A read-only, in-memory `metadata.sqlitedb`.
#[derive(Debug)]
pub struct MetadataDb {
    conn: Connection,
    tables: HashSet<String>,
}

impl MetadataDb {
    /// Largest database accepted (2 GiB).
    pub const MAX_SIZE: u64 = 2 << 30;

    /// Opens a `metadata.sqlitedb` image (copied into SQLite-owned memory).
    pub fn open(bytes: &[u8]) -> Result<Self, MetadataError> {
        let size = bytes.len() as u64;
        if size > Self::MAX_SIZE {
            return Err(MetadataError::TooLarge {
                size,
                limit: Self::MAX_SIZE,
            });
        }
        if !bytes.starts_with(SQLITE_MAGIC) {
            return Err(MetadataError::NotSqlite);
        }
        let mut conn = Connection::open_in_memory()?;
        conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
        conn.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
        conn.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)?;
        conn.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_VIEW, false)?;
        conn.deserialize_read_exact("main", bytes, bytes.len(), true)?;
        conn.pragma_update(None, "query_only", true)?;
        conn.pragma_update(None, "cell_size_check", true)?;
        let tables = rows::table_names(&conn)?;
        Ok(Self { conn, tables })
    }

    /// Opens the `metadata.sqlitedb` of an archive.
    pub fn from_archive(archive: &AbfArchive) -> Result<Self, MetadataError> {
        let file = archive
            .metadata_file()
            .ok_or(MetadataError::MissingMetadata)?;
        Self::open(&archive.read_file(file)?)
    }

    /// The underlying connection, for ad-hoc read-only queries.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// `true` if the database has a table with this name (case-insensitive).
    pub fn has_table(&self, name: &str) -> bool {
        self.tables.contains(&name.to_lowercase())
    }

    /// `DBPROPERTIES.SCHEMAVERSION`.
    pub fn schema_version(&self) -> Option<i64> {
        if !self.has_table("DBPROPERTIES") {
            return None;
        }
        // `get_ref` + manual conversion: rusqlite's typed `get` builds its error from
        // the column name, which panics on non-UTF-8 names in a corrupted schema.
        self.conn
            .query_row(
                "SELECT VALUE FROM DBPROPERTIES WHERE NAME = 'SCHEMAVERSION'",
                [],
                |row| {
                    Ok(match row.get_ref(0)? {
                        rusqlite::types::ValueRef::Integer(v) => Some(v),
                        _ => None,
                    })
                },
            )
            .ok()
            .flatten()
    }

    fn rows(&self, table: &'static str) -> Result<Rows, MetadataError> {
        Rows::load(&self.conn, &self.tables, table)
    }

    fn require(&self, table: &'static str) -> Result<Rows, MetadataError> {
        if !self.has_table(table) {
            return Err(MetadataError::MissingTable(table));
        }
        self.rows(table)
    }

    fn catalog(&self) -> Result<Catalog, MetadataError> {
        Ok(Catalog {
            model: self.rows("Model")?,
            tables: self.require("Table")?,
            columns: self.require("Column")?,
            measures: self.rows("Measure")?,
            partitions: self.rows("Partition")?,
            relationships: self.rows("Relationship")?,
            expressions: self.rows("Expression")?,
            hierarchies: self.rows("Hierarchy")?,
            levels: self.rows("Level")?,
            annotations: self.rows("Annotation")?,
            query_groups: self.rows("QueryGroup")?,
            data_sources: self.rows("DataSource")?,
            roles: self.rows("Role")?,
            table_permissions: self.rows("TablePermission")?,
            cultures: self.rows("Culture")?,
            linguistic_metadata: self.rows("LinguisticMetadata")?,
            perspectives: self.rows("Perspective")?,
            variations: self.rows("Variation")?,
            calculation_groups: self.rows("CalculationGroup")?,
            calculation_items: self.rows("CalculationItem")?,
            format_string_definitions: self.rows("FormatStringDefinition")?,
            detail_rows_definitions: self.rows("DetailRowsDefinition")?,
            kpis: self.rows("KPI")?,
        })
    }

    fn storage_tables(&self) -> Result<StorageTables, MetadataError> {
        Ok(StorageTables {
            table_storage: self.rows("TableStorage")?,
            partition_storage: self.rows("PartitionStorage")?,
            segment_map_storage: self.rows("SegmentMapStorage")?,
            column_storage: self.rows("ColumnStorage")?,
            column_partition_storage: self.rows("ColumnPartitionStorage")?,
            segment_storage: self.rows("SegmentStorage")?,
            dictionary_storage: self.rows("DictionaryStorage")?,
            attribute_hierarchy: self.rows("AttributeHierarchy")?,
            attribute_hierarchy_storage: self.rows("AttributeHierarchyStorage")?,
            storage_file: self.rows("StorageFile")?,
            storage_folder: self.rows("StorageFolder")?,
        })
    }

    /// Maps the metadata to a [`Model`] (without database info; see [`load_model`]).
    pub fn read_model(&self, options: &ModelOptions) -> Result<Model, MetadataError> {
        Ok(model::build_model(&self.catalog()?, options))
    }

    /// Builds the [`StorageMap`].
    pub fn storage_map(&self) -> Result<StorageMap, MetadataError> {
        let tables = self.require("Table")?;
        let columns = self.require("Column")?;
        let partitions = self.rows("Partition")?;
        Ok(storage::build_storage_map(
            &tables,
            &columns,
            &partitions,
            &self.storage_tables()?,
        ))
    }

    /// Model, storage map and schema version in one pass over the metadata tables.
    pub fn load(&self, options: &ModelOptions) -> Result<LoadedModel, MetadataError> {
        let catalog = self.catalog()?;
        let model = model::build_model(&catalog, options);
        let storage = storage::build_storage_map(
            &catalog.tables,
            &catalog.columns,
            &catalog.partitions,
            &self.storage_tables()?,
        );
        Ok(LoadedModel {
            model,
            storage,
            schema_version: self.schema_version(),
            format: MetadataFormat::Sqlite,
            diagnostics: Vec::new(),
        })
    }
}

/// Reads the model of an archive: `metadata.sqlitedb` → [`Model`] and [`StorageMap`],
/// plus `Model::database` (name, compatibility level) from the database definition
/// (`<db>.db.xml`) when present.
///
/// Without `metadata.sqlitedb`, legacy XML metadata (a `*.cub.xml`, as in Power Pivot
/// workbooks) is read instead, with a
/// [`codes::XML_METADATA`](crate::vertipaq::codes::XML_METADATA) note.
pub fn load_model(
    archive: &AbfArchive,
    options: &ModelOptions,
) -> Result<LoadedModel, MetadataError> {
    if archive.metadata_file().is_none() && xml::has_xml_metadata(archive) {
        return xml::load_xml_model(archive, options);
    }
    let db = MetadataDb::from_archive(archive)?;
    let mut loaded = db.load(options)?;
    loaded.model.database = database::database_from_archive(archive);
    Ok(loaded)
}
