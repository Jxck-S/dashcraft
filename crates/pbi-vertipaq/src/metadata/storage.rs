//! Physical VertiPaq storage of a model: which ABF files hold each column's data.
//!
//! Built from the storage tables of `metadata.sqlitedb` (`TableStorage`,
//! `PartitionStorage`, `SegmentMapStorage`, `ColumnStorage`, `ColumnPartitionStorage`,
//! `SegmentStorage`, `DictionaryStorage`, `AttributeHierarchy(Storage)`, `StorageFile`,
//! `StorageFolder`). File paths are logical archive paths (`/`-separated), directly
//! usable with [`AbfArchive::read`](crate::abf::AbfArchive::read).
//!
//! Data values (`AttributeHierarchyStorage.MinValue`/`MaxValue`) are deliberately not
//! copied.

use std::collections::HashMap;

use pbi_model::{DataType, names_eq};

use super::codes::{self, ColumnKindCode};
use super::rows::{Row, Rows};
use crate::abf::AbfArchive;

/// How a column's data ids map to values (`DictionaryStorage.Type`, as observed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DictionaryEncoding {
    /// 0: no dictionary (internal `H$`/`R$` system columns).
    None,
    /// 1: hash encoding — a `.dictionary` file maps data ids to values.
    Hash,
    /// 2: value encoding — no file; `value = (data_id + base_id) / magnitude`.
    Value,
    /// Any other code.
    Other(i64),
}

impl DictionaryEncoding {
    fn from_code(code: i64) -> Self {
        match code {
            0 => Self::None,
            1 => Self::Hash,
            2 => Self::Value,
            other => Self::Other(other),
        }
    }
}

/// `DictionaryStorage` row of a column.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct DictionaryStorage {
    /// `DictionaryStorage.ID`.
    pub storage_id: i64,
    /// Decoded `Type`.
    pub encoding: DictionaryEncoding,
    /// `DataType`: physical value type (TOM `DataType` codes: 2 string, 6 int64,
    /// 8 double, 19 none). Dates are stored as 8 (double).
    pub data_type_code: i64,
    /// `DataVersion`.
    pub data_version: Option<i64>,
    /// `BaseId` (value encoding offset; 2 for hash dictionaries).
    pub base_id: i64,
    /// `Magnitude` (value encoding divisor; 0 for hash dictionaries).
    pub magnitude: f64,
    /// `LastId`.
    pub last_id: Option<i64>,
    /// `IsNullable`.
    pub is_nullable: bool,
    /// `IsUnique`.
    pub is_unique: bool,
    /// `IsOperatingOn32`: numeric dictionary values are 32-bit.
    pub is_operating_on_32: bool,
    /// `DictionaryFlags` (raw).
    pub dictionary_flags: i64,
    /// `Size` (bytes of the dictionary file).
    pub size: Option<i64>,
    /// Logical path of the `.dictionary` file. `None` for value encoding, and also for
    /// hash-encoded columns whose only value is BLANK (`Statistics_DistinctStates` = 1).
    pub path: Option<String>,
}

/// `AttributeHierarchyStorage` row of a column (sort order / hash index).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AttributeHierarchyStorage {
    /// `AttributeHierarchyStorage.ID`.
    pub storage_id: i64,
    /// `SortOrder`.
    pub sort_order: Option<i64>,
    /// `OptimizationLevel`.
    pub optimization_level: Option<i64>,
    /// `MaterializationType`.
    pub materialization_type: Option<i64>,
    /// `ColumnPositionToData` (column of the `H$` table, -1 if none).
    pub column_position_to_data: Option<i64>,
    /// `ColumnDataToPosition` (column of the `H$` table, -1 if none).
    pub column_data_to_position: Option<i64>,
    /// `DistinctDataCount`.
    pub distinct_data_count: Option<i64>,
    /// `DataVersion`.
    pub data_version: Option<i64>,
    /// `SystemTableID`: the `H$` table holding the sort-order columns.
    pub system_table_id: Option<i64>,
    /// `HasStatistics`.
    pub has_statistics: bool,
    /// Logical path of the `.hidx` hash index file, if any.
    pub path: Option<String>,
}

/// `ColumnStorage.Statistics_*` values.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ColumnStatistics {
    /// `Statistics_DistinctStates`: dictionary cardinality.
    pub distinct_states: Option<i64>,
    /// `Statistics_MinDataID`.
    pub min_data_id: Option<i64>,
    /// `Statistics_MaxDataID`.
    pub max_data_id: Option<i64>,
    /// `Statistics_OriginalMinSegmentDataID`.
    pub original_min_segment_data_id: Option<i64>,
    /// `Statistics_RLESortOrder`.
    pub rle_sort_order: Option<i64>,
    /// `Statistics_RowCount`.
    pub row_count: Option<i64>,
    /// `Statistics_HasNulls`.
    pub has_nulls: Option<bool>,
    /// `Statistics_RLERuns`.
    pub rle_runs: Option<i64>,
    /// `Statistics_OthersRLERuns`.
    pub others_rle_runs: Option<i64>,
    /// `Statistics_Usage`.
    pub usage: Option<i64>,
    /// `Statistics_DBType`: OLE DB type (130 WSTR, 20 I8, 5 R8, 7 DATE, 6 CY, 11 BOOL).
    pub db_type: Option<i64>,
    /// `Statistics_XMType` (observed: 0 integer, 1 real/date, 2 string).
    pub xm_type: Option<i64>,
    /// `Statistics_CompressionType`.
    pub compression_type: Option<i64>,
    /// `Statistics_CompressionParam`.
    pub compression_param: Option<i64>,
    /// `Statistics_EncodingHint`.
    pub encoding_hint: Option<i64>,
}

/// The files of one column in one partition.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ColumnPartitionFiles {
    /// `Partition.ID` (0 if unknown).
    pub partition_id: i64,
    /// `PartitionStorage.ID`.
    pub partition_storage_id: i64,
    /// `PartitionStorage.StoragePosition`.
    pub storage_position: i64,
    /// Rows in the partition (`SegmentMapStorage.RecordCount`).
    pub record_count: Option<u64>,
    /// Segments in the IDF (`SegmentStorage.SegmentCount`).
    pub segment_count: Option<u64>,
    /// `ColumnPartitionStorage.DataVersion`.
    pub data_version: Option<i64>,
    /// `ColumnPartitionStorage.State`.
    pub state: Option<i64>,
    /// Logical path of the `.idf` (data ids, RLE/bit-packed segments).
    pub idf: Option<String>,
    /// Logical path of the `.idfmeta` (per-segment encoding metadata).
    pub idfmeta: Option<String>,
    /// Per-segment metadata given inline instead of an `.idfmeta` file (legacy XML
    /// models describe segments in their `.tbl.xml`).
    pub meta: Option<crate::vertipaq::idfmeta::ColumnPartitionMeta>,
}

/// Physical storage of one column.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ColumnStorage {
    /// `Column.ID`.
    pub column_id: i64,
    /// Column name (`ExplicitName`, else `InferredName`).
    pub name: String,
    /// `Column.Type`.
    pub kind: ColumnKindCode,
    /// Effective logical type (`ExplicitDataType`, or `InferredDataType` when explicit
    /// is Automatic).
    pub data_type: DataType,
    /// TOM code of [`ColumnStorage::data_type`].
    pub data_type_code: i64,
    /// `ColumnStorage.ID` (0 if the column has no storage row).
    pub storage_id: i64,
    /// `ColumnStorage.Name` (e.g. `Amount (20)`).
    pub storage_name: Option<String>,
    /// `ColumnStorage.StoragePosition`.
    pub storage_position: i64,
    /// `ColumnStorage.Settings` (raw flags).
    pub settings: Option<i64>,
    /// `ColumnStorage.ColumnFlags` (raw flags).
    pub column_flags: Option<i64>,
    /// `ColumnStorage.Collation`.
    pub collation: Option<String>,
    /// `ColumnStorage.Locale` (LCID).
    pub locale: Option<i64>,
    /// `ColumnStorage.Statistics_*`.
    pub statistics: ColumnStatistics,
    /// Dictionary storage, if any.
    pub dictionary: Option<DictionaryStorage>,
    /// Attribute hierarchy storage, if any.
    pub attribute_hierarchy: Option<AttributeHierarchyStorage>,
    /// Per-partition IDF files in partition storage order.
    pub partitions: Vec<ColumnPartitionFiles>,
}

impl ColumnStorage {
    /// Logical path of the `.dictionary` file.
    pub fn dictionary_path(&self) -> Option<&str> {
        self.dictionary.as_ref()?.path.as_deref()
    }

    /// Logical path of the `.hidx` file.
    pub fn hidx_path(&self) -> Option<&str> {
        self.attribute_hierarchy.as_ref()?.path.as_deref()
    }

    /// Every file path referenced by this column.
    pub fn files(&self) -> impl Iterator<Item = &str> {
        self.dictionary_path()
            .into_iter()
            .chain(self.hidx_path())
            .chain(
                self.partitions
                    .iter()
                    .flat_map(|p| p.idf.as_deref().into_iter().chain(p.idfmeta.as_deref())),
            )
    }

    /// `true` for columns whose values are stored (data, calculated and calculated
    /// table columns; not the internal row-number column).
    pub fn has_values(&self) -> bool {
        matches!(
            self.kind,
            ColumnKindCode::Data
                | ColumnKindCode::Calculated
                | ColumnKindCode::CalculatedTableColumn
        )
    }
}

/// Storage of one partition.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PartitionStorage {
    /// `Partition.ID`.
    pub partition_id: i64,
    /// `Partition.Name`.
    pub name: String,
    /// `PartitionStorage.ID` (0 if missing).
    pub storage_id: i64,
    /// `PartitionStorage.StoragePosition`.
    pub storage_position: i64,
    /// Logical folder (`T (12).tbl/15.prt`).
    pub folder: Option<String>,
    /// `SegmentMapStorage.RecordCount`: rows in the partition.
    pub record_count: u64,
    /// `SegmentMapStorage.SegmentCount`.
    pub segment_count: u64,
    /// `SegmentMapStorage.RecordsPerSegment`.
    pub records_per_segment: u64,
    /// `SegmentMapStorage.Type`.
    pub segment_map_type: Option<i64>,
}

/// Storage of one table.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TableStorage {
    /// `Table.ID`.
    pub table_id: i64,
    /// `Table.Name`.
    pub name: String,
    /// `TableStorage.Name` (e.g. `Sales (12)`).
    pub storage_name: Option<String>,
    /// Logical folder (`Sales (12).tbl`).
    pub folder: Option<String>,
    /// `Table.SystemFlags` (bit 0: internal `H$`/`R$`/`U$` table; 2: calculated table).
    pub system_flags: i64,
    /// Internal storage table (`SystemFlags & 1`).
    pub is_internal: bool,
    /// Total rows (sum of the partitions' record counts).
    pub row_count: u64,
    /// Partitions in storage order.
    pub partitions: Vec<PartitionStorage>,
    /// Columns in storage order (including the row-number column).
    pub columns: Vec<ColumnStorage>,
}

impl TableStorage {
    /// Column by name (case-insensitive).
    pub fn column(&self, name: &str) -> Option<&ColumnStorage> {
        self.columns.iter().find(|c| names_eq(&c.name, name))
    }
}

/// Physical storage of every table of a model (including internal tables).
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct StorageMap {
    /// Tables in `ID` order.
    pub tables: Vec<TableStorage>,
}

impl StorageMap {
    /// Table by name (case-insensitive).
    pub fn table(&self, name: &str) -> Option<&TableStorage> {
        self.tables.iter().find(|t| names_eq(&t.name, name))
    }

    /// Column `table[column]` (case-insensitive).
    pub fn column(&self, table: &str, column: &str) -> Option<&ColumnStorage> {
        self.table(table)?.column(column)
    }

    /// Every file path referenced by the map.
    pub fn files(&self) -> impl Iterator<Item = &str> {
        self.tables
            .iter()
            .flat_map(|t| t.columns.iter())
            .flat_map(ColumnStorage::files)
    }

    /// Referenced paths that do not exist in `archive`.
    pub fn missing_files<'a>(&'a self, archive: &AbfArchive) -> Vec<&'a str> {
        self.files()
            .filter(|path| archive.find(path).is_none())
            .collect()
    }
}

/// Lookup tables over the storage-related metadata rows.
pub(crate) struct StorageTables {
    pub table_storage: Rows,
    pub partition_storage: Rows,
    pub segment_map_storage: Rows,
    pub column_storage: Rows,
    pub column_partition_storage: Rows,
    pub segment_storage: Rows,
    pub dictionary_storage: Rows,
    pub attribute_hierarchy: Rows,
    pub attribute_hierarchy_storage: Rows,
    pub storage_file: Rows,
    pub storage_folder: Rows,
}

fn group_by<'a>(rows: &'a Rows, column: &str) -> HashMap<i64, Vec<Row<'a>>> {
    let mut map: HashMap<i64, Vec<Row<'a>>> = HashMap::new();
    for row in rows.iter() {
        if let Some(key) = row.i64(column) {
            map.entry(key).or_default().push(row);
        }
    }
    map
}

fn non_negative(value: Option<i64>) -> Option<u64> {
    value.and_then(|v| u64::try_from(v).ok())
}

struct Resolver<'a> {
    st: &'a StorageTables,
}

impl Resolver<'_> {
    fn folder(&self, id: Option<i64>) -> Option<String> {
        let row = self.st.storage_folder.by_id(id.filter(|&i| i > 0)?)?;
        let path = crate::abf::sanitize_path(row.str("Path")?);
        (!path.is_empty()).then_some(path)
    }

    fn file(&self, id: Option<i64>) -> Option<String> {
        let row = self.st.storage_file.by_id(id.filter(|&i| i > 0)?)?;
        let name = crate::abf::sanitize_path(row.str("FileName")?);
        if name.is_empty() {
            return None;
        }
        Some(match self.folder(row.i64("StorageFolderID")) {
            Some(folder) => format!("{folder}/{name}"),
            None => name,
        })
    }
}

pub(crate) fn build_storage_map(
    tables: &Rows,
    columns: &Rows,
    partitions: &Rows,
    st: &StorageTables,
) -> StorageMap {
    let resolver = Resolver { st };
    let table_storage_by_table = group_by(&st.table_storage, "TableID");
    let partitions_by_table = group_by(partitions, "TableID");
    let segment_maps_by_partition_storage = group_by(&st.segment_map_storage, "PartitionStorageID");
    let column_storage_by_column = group_by(&st.column_storage, "ColumnID");
    let cps_by_column_storage = group_by(&st.column_partition_storage, "ColumnStorageID");
    let segments_by_cps = group_by(&st.segment_storage, "ColumnPartitionStorageID");
    let dictionary_by_column_storage = group_by(&st.dictionary_storage, "ColumnStorageID");
    let hierarchy_by_column = group_by(&st.attribute_hierarchy, "ColumnID");
    let columns_by_table = group_by(columns, "TableID");

    let segment_map = |partition_storage: Option<Row<'_>>| -> Option<Row<'_>> {
        let ps = partition_storage?;
        ps.id_ref("SegmentMapStorageID")
            .and_then(|id| st.segment_map_storage.by_id(id))
            .or_else(|| {
                segment_maps_by_partition_storage
                    .get(&ps.id())
                    .and_then(|v| v.first().copied())
            })
    };

    let mut out = StorageMap::default();
    for table in tables.iter() {
        let table_id = table.id();
        let system_flags = table.i64("SystemFlags").unwrap_or(0);
        let ts = table
            .id_ref("TableStorageID")
            .and_then(|id| st.table_storage.by_id(id))
            .or_else(|| {
                table_storage_by_table
                    .get(&table_id)
                    .and_then(|v| v.first().copied())
            });

        // Partitions in storage order.
        let mut partition_infos: Vec<PartitionStorage> = Vec::new();
        let mut partition_by_storage: HashMap<i64, (i64, i64, Option<u64>)> = HashMap::new();
        for partition in partitions_by_table.get(&table_id).into_iter().flatten() {
            let ps = partition
                .id_ref("PartitionStorageID")
                .and_then(|id| st.partition_storage.by_id(id));
            let sm = segment_map(ps);
            let record_count = sm.and_then(|r| non_negative(r.i64("RecordCount")));
            let info = PartitionStorage {
                partition_id: partition.id(),
                name: partition.string("Name").unwrap_or_default(),
                storage_id: ps.map(|r| r.id()).unwrap_or(0),
                storage_position: ps.and_then(|r| r.i64("StoragePosition")).unwrap_or(0),
                folder: resolver.folder(ps.and_then(|r| r.i64("StorageFolderID"))),
                record_count: record_count.unwrap_or(0),
                segment_count: sm
                    .and_then(|r| non_negative(r.i64("SegmentCount")))
                    .unwrap_or(0),
                records_per_segment: sm
                    .and_then(|r| non_negative(r.i64("RecordsPerSegment")))
                    .unwrap_or(0),
                segment_map_type: sm.and_then(|r| r.i64("Type")),
            };
            if info.storage_id != 0 {
                partition_by_storage.insert(
                    info.storage_id,
                    (info.partition_id, info.storage_position, record_count),
                );
            }
            partition_infos.push(info);
        }
        partition_infos.sort_by_key(|p| (p.storage_position, p.partition_id));
        let row_count = partition_infos
            .iter()
            .fold(0u64, |acc, p| acc.saturating_add(p.record_count));

        let mut column_infos: Vec<ColumnStorage> = Vec::new();
        for column in columns_by_table.get(&table_id).into_iter().flatten() {
            let column_id = column.id();
            let cs = column
                .id_ref("ColumnStorageID")
                .and_then(|id| st.column_storage.by_id(id))
                .or_else(|| {
                    column_storage_by_column
                        .get(&column_id)
                        .and_then(|v| v.first().copied())
                });
            let storage_id = cs.map(|r| r.id()).unwrap_or(0);
            let (data_type_code, _) = codes::effective_data_type(
                column.i64("ExplicitDataType"),
                column.i64("InferredDataType"),
            );

            let dictionary = cs
                .and_then(|cs| {
                    cs.id_ref("DictionaryStorageID")
                        .and_then(|id| st.dictionary_storage.by_id(id))
                        .or_else(|| {
                            dictionary_by_column_storage
                                .get(&cs.id())
                                .and_then(|v| v.first().copied())
                        })
                })
                .map(|ds| DictionaryStorage {
                    storage_id: ds.id(),
                    encoding: DictionaryEncoding::from_code(ds.i64("Type").unwrap_or(-1)),
                    data_type_code: ds.i64("DataType").unwrap_or(19),
                    data_version: ds.i64("DataVersion"),
                    base_id: ds.i64("BaseId").unwrap_or(0),
                    magnitude: ds.f64("Magnitude").unwrap_or(0.0),
                    last_id: ds.i64("LastId"),
                    is_nullable: ds.flag("IsNullable"),
                    is_unique: ds.flag("IsUnique"),
                    is_operating_on_32: ds.flag("IsOperatingOn32"),
                    dictionary_flags: ds.i64("DictionaryFlags").unwrap_or(0),
                    size: ds.i64("Size"),
                    path: resolver.file(ds.i64("StorageFileID")),
                });

            let attribute_hierarchy = column
                .id_ref("AttributeHierarchyID")
                .and_then(|id| st.attribute_hierarchy.by_id(id))
                .or_else(|| {
                    hierarchy_by_column
                        .get(&column_id)
                        .and_then(|v| v.first().copied())
                })
                .and_then(|ah| {
                    ah.id_ref("AttributeHierarchyStorageID")
                        .and_then(|id| st.attribute_hierarchy_storage.by_id(id))
                })
                .map(|ahs| AttributeHierarchyStorage {
                    storage_id: ahs.id(),
                    sort_order: ahs.i64("SortOrder"),
                    optimization_level: ahs.i64("OptimizationLevel"),
                    materialization_type: ahs.i64("MaterializationType"),
                    column_position_to_data: ahs.i64("ColumnPositionToData"),
                    column_data_to_position: ahs.i64("ColumnDataToPosition"),
                    distinct_data_count: ahs.i64("DistinctDataCount"),
                    data_version: ahs.i64("DataVersion"),
                    system_table_id: ahs.id_ref("SystemTableID"),
                    has_statistics: ahs.flag("HasStatistics"),
                    path: resolver.file(ahs.i64("StorageFileID")),
                });

            let mut column_partitions: Vec<ColumnPartitionFiles> = cps_by_column_storage
                .get(&storage_id)
                .into_iter()
                .flatten()
                .map(|cps| {
                    let partition_storage_id = cps.i64("PartitionStorageID").unwrap_or(0);
                    let (partition_id, storage_position, record_count) = partition_by_storage
                        .get(&partition_storage_id)
                        .copied()
                        .unwrap_or_else(|| {
                            let ps = st.partition_storage.by_id(partition_storage_id);
                            (
                                ps.and_then(|r| r.i64("PartitionID")).unwrap_or(0),
                                ps.and_then(|r| r.i64("StoragePosition")).unwrap_or(0),
                                segment_map(ps).and_then(|r| non_negative(r.i64("RecordCount"))),
                            )
                        });
                    let segment = cps
                        .id_ref("SegmentStorageID")
                        .and_then(|id| st.segment_storage.by_id(id))
                        .or_else(|| {
                            segments_by_cps
                                .get(&cps.id())
                                .and_then(|v| v.first().copied())
                        });
                    let idf = resolver.file(cps.i64("StorageFileID"));
                    let idfmeta = segment
                        .and_then(|s| resolver.file(s.i64("StorageFileID")))
                        .or_else(|| idf.as_ref().map(|p| format!("{p}meta")));
                    ColumnPartitionFiles {
                        partition_id,
                        partition_storage_id,
                        storage_position,
                        record_count,
                        segment_count: segment.and_then(|s| non_negative(s.i64("SegmentCount"))),
                        data_version: cps.i64("DataVersion"),
                        state: cps.i64("State"),
                        idf,
                        idfmeta,
                        meta: None,
                    }
                })
                .collect();
            column_partitions.sort_by_key(|p| (p.storage_position, p.partition_storage_id));

            column_infos.push(ColumnStorage {
                column_id,
                name: column
                    .string("ExplicitName")
                    .or_else(|| column.string("InferredName"))
                    .unwrap_or_default(),
                kind: ColumnKindCode::from_code(
                    column
                        .i64("Type")
                        .or_else(|| column.i64("BindingType"))
                        .unwrap_or(1),
                ),
                data_type: codes::data_type(data_type_code),
                data_type_code,
                storage_id,
                storage_name: cs.and_then(|r| r.string("Name")),
                storage_position: cs
                    .and_then(|r| r.i64("StoragePosition"))
                    .unwrap_or(i64::MAX),
                settings: cs.and_then(|r| r.i64("Settings")),
                column_flags: cs.and_then(|r| r.i64("ColumnFlags")),
                collation: cs.and_then(|r| r.string("Collation")),
                locale: cs.and_then(|r| r.i64("Locale")),
                statistics: cs.map(statistics).unwrap_or_default(),
                dictionary,
                attribute_hierarchy,
                partitions: column_partitions,
            });
        }
        column_infos.sort_by_key(|c| (c.storage_position, c.column_id));

        out.tables.push(TableStorage {
            table_id,
            name: table.string("Name").unwrap_or_default(),
            storage_name: ts.and_then(|r| r.string("Name")),
            folder: resolver.folder(ts.and_then(|r| r.i64("StorageFolderID"))),
            system_flags,
            is_internal: system_flags & 1 != 0,
            row_count,
            partitions: partition_infos,
            columns: column_infos,
        });
    }
    out
}

fn statistics(cs: Row<'_>) -> ColumnStatistics {
    ColumnStatistics {
        distinct_states: cs.i64("Statistics_DistinctStates"),
        min_data_id: cs.i64("Statistics_MinDataID"),
        max_data_id: cs.i64("Statistics_MaxDataID"),
        original_min_segment_data_id: cs.i64("Statistics_OriginalMinSegmentDataID"),
        rle_sort_order: cs.i64("Statistics_RLESortOrder"),
        row_count: cs.i64("Statistics_RowCount"),
        has_nulls: cs.opt_flag("Statistics_HasNulls"),
        rle_runs: cs.i64("Statistics_RLERuns"),
        others_rle_runs: cs.i64("Statistics_OthersRLERuns"),
        usage: cs.i64("Statistics_Usage"),
        db_type: cs.i64("Statistics_DBType"),
        xm_type: cs.i64("Statistics_XMType"),
        compression_type: cs.i64("Statistics_CompressionType"),
        compression_param: cs.i64("Statistics_CompressionParam"),
        encoding_hint: cs.i64("Statistics_EncodingHint"),
    }
}
