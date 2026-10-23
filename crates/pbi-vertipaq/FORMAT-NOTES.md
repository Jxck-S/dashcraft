# pbi-vertipaq — format notes

Notes on the on-disk formats handled (or about to be handled) by this crate. Sources
and licences: see `REFERENCES.md`. Observations marked *(sample)* come from the
`DataModel` of `reference/samples/microsoft/Revenue Opportunities.pbix` (Microsoft
Power BI Desktop sample, MIT; compatibility level 1606), unless another committed sample
is named; the tests in `tests/` assert them. *(observed)* marks behaviour seen in Power
BI Desktop files that the committed samples do not show.

## 1. PBIX `DataModel` container

The `DataModel` ZIP entry is normally *stored* (not deflated). All integers are
little-endian.

### 1.1 Signature (offset 0, 102 bytes)

| Bytes | Meaning |
|---|---|
| UTF-16LE `This backup was created using XPress9 compression.` + `00 00` | single-threaded XPress9 |
| UTF-16LE `This backup was created using multithreaded XPrs9.` + `00 00` | multi-threaded XPress9 |
| `FF FE` + UTF-16LE `STREAM_STORAGE_SIGNATURE_)!@#$%^&*(` (72 bytes) | **uncompressed** ABF (the stream *is* the backup) |

Both XPress9 texts are 50 UTF-16 code units, so text + NUL = 102 bytes. The decoder
matches the text at offset 0 and ignores the two NUL bytes. A raw `.abf` file and
`xl/model/item.data` in Power Pivot workbooks use the same three variants.

### 1.2 Chunks

```
chunk := u32 uncompressed_size, u32 compressed_size, u8[compressed_size] payload
```

The payload is one (in practice exactly one) or more whole XPress9 blocks; the decoded
size must equal `uncompressed_size`. A `(0, 0)` chunk is legal and empty.

*Single-threaded*: chunks follow the signature until end of stream. **All chunks form a
single XPress9 session**: block indices continue (0, 1, 2, ...) and back-references may
reach into earlier chunks (one reference-decoder instance is reused for the whole
stream).

*Multi-threaded*: after the signature comes a 40-byte header

| Offset | Type | Field |
|---|---|---|
| 102 | u64 | main chunks per thread |
| 110 | u64 | prefix chunks per thread |
| 118 | u64 | prefix thread count |
| 126 | u64 | main thread count |
| 134 | u64 | nominal chunk uncompressed size (informational) |

followed by `prefix_thread_count` groups of `prefix_chunks_per_thread` chunks, then
`main_thread_count` groups of `main_chunks_per_thread` chunks. **Each group is an
independent XPress9 session** (block index restarts at 0, no shared history), so groups
can be decoded in parallel; output order = stream order (prefix groups, then main
groups). Field order and group order follow pbixray; we have **no real multi-threaded
sample yet** — the layout is covered only by synthetic tests.

*(sample)* single-threaded, exactly **one chunk**: 1,572,864 bytes uncompressed from
210,588 bytes (210,698-byte stream). The other committed samples are single-threaded too.

## 2. XPress9 block

```
u32 magic            0x4E86D72A
u32 orig_size        decoded bytes of this block
u32 encoded_bits     total block size in BITS, including the 256 header bits
u32 flags            see below
u32 reserved         must be 0 (also the CRC seed)
u32 session          session signature, constant within a session
u32 block_index      0, 1, 2, ... within a session
u32 header_crc       CRC-32C (Castagnoli, reflected 0x82F63B78) of the first 28 bytes
bits                 ceil(encoded_bits / 8) - 32 bytes of LSB-first bit stream
```

`flags`: bits 0–12 table size in bits (MTF state + both code-length tables);
13–15 `log2(window) - 16` (window 64 KiB … 8 MiB); 16–17 MTF entries / 2 (0, 2, 4);
18 pointer min match − 3; 19 MTF min match − 2; 20–31 must be zero. All blocks of a
session must have identical window / MTF / min-match parameters.

*(sample)* flags `0x0006CB24`: table 2,852 bits, window 2^22 (4 MiB), 4 MTF entries,
pointer min match 4, MTF min match 2; encoded 1,684,704 bits. The MIT encoder in the
`xpress9` PyPI package produces the same MTF/min-match parameters (window 2^16 for
small inputs).

### 2.1 Bit stream

Bits are consumed LSB-first (bit *k* = bit *k mod 8* of byte *k / 8*); multi-bit fields
are little-endian. Huffman codewords are written most-significant bit first.

1. **MTF state** (only if MTF entries > 0): 1 bit "previous token was a match"; then per
   entry 5 bits `m` (must be < window log2) and `m` bits `v`; offset = `v + 2^m`.
2. **Code lengths** for the *short* alphabet (704 = 256 literals + 28 offset slots × 16
   lengths), then for the *long-length* alphabet (256). Each table starts with 3 bits:
   * `0` = stored: with `M = floor(log2(n))`, the first `2^(M+1) - n` symbols get `M`
     bits, the rest `M+1` (704 → 320×9 + 384×10; 256 → all 8).
   * `1` = meta-coded: 33 meta-code lengths (1 bit "same as previous", else 3 bits value,
     where values ≥ previous are incremented; start previous = 4; max 8), then meta
     symbols until the table is full (start previous = 8): `0..=27` literal length;
     `28` zeros up to the next multiple of 16; `29` zero run of `5 + 2 bits` (if the
     2 bits are 3: add 3-bit values while they equal 7) that must stay inside the
     current 16-symbol row; `30` repeat previous non-zero length; `31` length of the
     symbol 16 positions earlier (non-zero); `32` that length + 1 (≤ 27).
   * Other values are invalid.
   The consumed bits must equal the table size in `flags`.
3. **Codes**: canonical (DEFLATE ordering: by length, then symbol), max 27 bits, and the
   code must be complete (Kraft sum = 1) unless it has exactly one symbol, which then
   costs its declared length (1..=15) per use. *(observed)* short codes up to 17 bits,
   long-length codes up to 13 bits.
4. **Symbols** until `orig_size` bytes are produced:
   * `< 256`: literal.
   * else `c = sym - 256`, `slot = c >> 4`, `len = c & 15`; if `len == 15`:
     `L` = long-length symbol; if `L ≥ 232`: `L = read(L-232) + 2^(L-232) + 231`;
     `len = L + 15`.
   * `slot < mtf_entries`: MTF match, `len += mtf_min`; index = `slot + (previous
     token was a match)` (must be < entries); offset = entry[index], moved to front.
   * otherwise explicit offset: `len += ptr_min`; `n = slot - mtf_entries`;
     offset = `read(n) + 2^n`; pushed to the front of the MTF list (last entry drops).
   * Offsets must be ≤ the window size and ≤ the session history; matches may overlap
     their source and must not run past `orig_size`.
5. Afterwards the consumed bits must equal `encoded_bits` exactly.

## 3. Decoder design (this crate)

* `decompress_datamodel(bytes, &Limits)` → ABF bytes; `DataModelDecoder<R: Read>`
  streams chunk by chunk (also implements `Read`); `Xpress9Decoder` is a reusable
  session decoder; `decompress_block` decodes a standalone session-opening chunk;
  `fuzz_decompress` never panics; `detect_format` sniffs the signature.
* `Limits` (default): total output 4 GiB, per-chunk declared uncompressed and compressed
  size 2 GiB − 1 (the reference implementation uses `int` sizes). Sizes are checked
  *before* decoding; output memory grows only with bytes actually produced
  (`try_reserve`, no zero-fill), compressed chunk buffers grow with bytes actually read,
  and history is trimmed to the window (memory ≈ window + chunk in streaming mode).
* Every corrupt input maps to an `Xpress9Error`; chunk-level errors are wrapped in
  `InChunk { chunk, offset, .. }` (`innermost()` strips the context).
* Performance *(sample, Apple Silicon, release)*: ~1.4 ms for 1.6 MB (~1.1 GB/s, single
  run in `tests/xpress9_fixture.rs`); the downloaded AdventureWorks sample, 8.4 MB →
  22.9 MB, ~40 ms. Debug build ~7 ms for the sample.
* Not yet done: parallel decoding of multi-threaded groups (they are independent, so
  `std::thread::scope` over pre-scanned group boundaries is straightforward).

## 4. ABF backup container (`src/abf/`)

Sources: [MS-XLDM] §2.1 (page layout, `BackupLogHeaderType`, CRC end markers), pbixray
`abf/*.py` (structure of header, directory and log; join on `StoragePath`), and the
samples. *(sample)* = 1,572,864 bytes, SHA-256
`192622713e64c5ed70a9be3bb24a06e207e41c13411f24fc43340c1477c31dbd` (the same as
Microsoft's XPress9 library produces through pbixray's `xpress9` binding).

### 4.1 Layout

All sections are page aligned (page = 4096 bytes) and zero-padded.

| Offset | Content |
|---|---|
| 0 | `FF FE` + UTF-16LE `STREAM_STORAGE_SIGNATURE_)!@#$%^&*(` (72 bytes) |
| 72 | **Backup header**: UTF-16LE XML `<BackupLog>`, NUL-padded to 4096. We decode up to the first U+0000. |
| `m_cbOffsetData` (4096) | **Files section**: stored files back to back, in virtual-directory order: `ADDITIONAL_LOG`, `PARTITIONS`, the model files, and last the `LOG` |
| `m_cbOffsetHeader` | **Virtual directory**: UTF-8 XML `<VirtualDirectory>`, `DataSize` bytes, then zero padding |

*(sample)* header: `BackupRestoreSyncVersion` 140, `Fault` false, `faultcode` 0,
`ErrorCode` false, `EncryptionFlag` false, `EncryptionKey` 0, `ApplyCompression` false,
`m_cbOffsetHeader` 1,445,888, `DataSize` 125,788, `Files` 453, `ObjectID` (GUID),
`m_cbOffsetData` 4096. `ADDITIONAL_LOG` (112 bytes at 4096) and `PARTITIONS` (28 bytes)
precede the first model file at 4236 — files are **not** individually page aligned.

Header flags ([MS-XLDM] says Power Pivot files MUST set `ErrorCode` and
`ApplyCompression` to true; Power BI writes both false):

* `ErrorCode` = true: every stored entry (including `LOG`) ends with a 4-byte CRC marker
  (MSB-first CRC-32, polynomial `0x04C11DB7`, init `0xFFFFFFFF`, per [MS-XLDM]
  §2.1.2.2.1.1). The virtual-directory `Size` includes it; we strip it
  (`AbfFile::size = stored_size - 4`) and do **not** verify it (no Power BI sample has it).
* `ApplyCompression` = true: stored files are individually compressed ("Xpress", XPress8
  in pbixray); the log `Size` is then the uncompressed size. Decoded by `src/xpress8.rs`
  (§4.4): `AbfArchive::read_file` decompresses (and `metadata`/`vertipaq` read through it);
  `AbfArchive::read` stays zero-copy and returns `AbfError::CompressedFile`; `read_stored`
  gives the raw chunks.
* `EncryptionFlag` = true: rejected (`AbfError::Encrypted`).

### 4.2 Virtual directory

`<VirtualDirectory>` → `<BackupFile>`: `Path` (storage name), `Size`, `m_cbOffsetHeader`
(absolute offset), `Delete`, `CreatedTimestamp`, `Access`, `LastWriteTime` (FILETIME
ticks). Storage names are 20 upper-case hex digits, except `ADDITIONAL_LOG`, `PARTITIONS`
and `LOG`. *(sample)* 453 entries = 2 + 450 model files + `LOG` (397,370 bytes at
1,047,450, the last entry). Every entry's range is bounds-checked at parse time.

### 4.3 Backup log (`LOG` entry)

UTF-16LE with BOM `FF FE`, XML `<BackupLog>`: `BackupRestoreSyncVersion` (`11.53`),
`ServerRoot`, `SvrEncryptPwdFlag`, `ServerEnableBinaryXML`, `ServerEnableCompression`,
`CompressionFlag`, `EncryptionFlag`, `ObjectName`/`ObjectId` (database GUID), `Write`,
`OlapInfo`, `IsTabular`, `Collations/Collation`, `Languages/Language` (LCID 1033), and
`FileGroups/FileGroup` with `Class`, `ID`, `Name`, `ObjectVersion`, `PersistLocation`,
`PersistLocationPath`, `StorageLocationPath`, `ObjectID`, `FileList/BackupFile` (`Path`
= original absolute server path, `StoragePath` = storage name, `LastWriteTime`, `Size`).

*(sample)* two file groups: class 100002 (database: `<db>.<n>.db.xml` next to the
database folder and `<db>.<n>.db\<n>.CryptKey.bin`) and class 100069 (448 model files
under `<db>.<n>.db\`). Log `Size` equals directory `Size` for all 450 files.

**Logical paths** (`AbfFile::logical_path`): the original path relative to the longest
matching `PersistLocationPath`/`StorageLocationPath`/`ServerRoot` (case-insensitive),
`\` → `/`, empty/`.`/`..` segments dropped; other absolute paths keep only the file
name. So `metadata.sqlitedb`, `1.CryptKey.bin`, `<db>.1.db.xml` and
`<Table> (<id>).tbl/<partition id>.prt/<n>.<Table> (<id>).<Column> (<id>).0.idf`. The
absolute paths contain the workstation user profile and are never stored, logged or put
in error messages. Lookups (`find`, `read`) are case-insensitive and accept `\`.

Storage folder and file names are sanitised by the engine: `-` in object names becomes a
space (`RowNumber-2662979B-…` → `RowNumber 2662979B …`), and a table keeps the folder
name it was created with (*(sample)* `Calculations` lives in `Fact (2595).tbl` and
`Sales stage` in `SalesStage (2450).tbl`), so always resolve files through
`metadata.sqlitedb` (§5), never by building names from table/column names.

*(sample)* extensions: 189 `.idf`, 189 `.idfmeta`, 36 `.dictionary`, 33 `.hidx`, 1
`.sqlitedb` (712,704 bytes), 1 `.xml`, 1 `.bin`.

Verification: the 450 file names and sizes are identical to pbixray 0.15.5's
`file_log` (pbixray keeps the absolute path for the `.db.xml`, which names the author's
workspace folder; we make it relative).

### 4.4 Per-file compression (`ApplyCompression`, `src/xpress8.rs`)

Sources: [MS-XCA] §2.3/§2.4 (Plain LZ77, the authoritative algorithm), the chunk framing
of `xpress8-python` `decompress_chunked` and pbixray `utils.get_data_slice`, and a
cross-check of Microsoft's ESE `_xpress` decoder (`xdecode.c`/`xdecode.i`, compiled with
`CODING_ALG=1` = `CODING_DIRECT2`, vendored MIT in `xpress8-python`). See `REFERENCES.md`.

A compressed file (after the CRC marker is stripped) is a sequence of chunks:

| Bytes | Field |
|---|---|
| u16 LE | uncompressed size `U` (≤ 65,535) |
| u16 LE | compressed size `C` |
| `C` | payload: stored verbatim if `C == U`, else one Plain LZ77 block of `U` bytes |

Chunks are independent (each block's matches reach only into its own output). `C > U` is
invalid. The concatenation must equal the backup-log `Size`.

Plain LZ77 block: 32-bit LE flag words, consumed MSB first (bit 31 first), one flag per
token; `0` = literal byte, `1` = match: u16 LE `v`, offset = `(v >> 3) + 1` (≤ 8,192),
length code = `v & 7`; code 7 continues in a nibble that alternates between the low
nibble of a fresh byte and the high nibble of that same byte at the next long match;
nibble 15 continues in a byte, byte 255 in a u16 (0 → u32) holding the full length − 3
(must be ≥ 22); the length is the sum + 3. Matches may overlap. The reference decoder
stops when the declared output size is reached (a final match is truncated); we do the
same and report every other inconsistency (`Xpress8Error`).

*(verified locally, not committed)* the two Power Pivot workbooks in pbixray's `data/`
(`null_data_id.xlsx`, `Supplier Quality Analysis Sample-no-PV.xlsx`): header
`ErrorCode` and `ApplyCompression` true, 34 and 206 files, every file decompresses to its
log size. `ADDITIONAL_LOG`, `PARTITIONS`, `LOG` and the virtual directory are not
compressed.

## 5. `metadata.sqlitedb` (`src/metadata/`)

SQLite 3 database (*(sample)* `DBPROPERTIES.SCHEMAVERSION` = 123, 70 tables). Opened
in memory with `sqlite3_deserialize` (read-only, `SQLITE_DBCONFIG_DEFENSIVE`, untrusted
schema, triggers and views disabled, `query_only`). Tables are read with `SELECT *` and
columns looked up by name (column names come from `PRAGMA table_info`, because
rusqlite's `column_names()` panics on non-UTF-8 names in a corrupt schema).

### 5.1 TOM tables → `pbi_model::Model`

The tables mirror the TOM / `TMSCHEMA_*` rowsets: `Model`, `Table`, `Column`, `Measure`,
`Partition`, `Relationship`, `Expression`, `QueryGroup`, `Hierarchy`, `Level`,
`Annotation` (`ObjectType`, `ObjectID`), `Role`, `TablePermission`, `Culture`,
`LinguisticMetadata`, `Perspective`, `Variation`, `CalculationGroup`, `CalculationItem`,
`FormatStringDefinition`, `DetailRowsDefinition`, `KPI`, `DataSource`, ... Codes are the
TOM enumeration values (verified against the Learn API reference):

| Field | Codes |
|---|---|
| `Column.ExplicitDataType` / `InferredDataType` / `Measure.DataType` | 1 Automatic, 2 String, 6 Int64, 8 Double, 9 DateTime, 10 Decimal, 11 Boolean, 17 Binary, 19 Unknown, 20 Variant. Effective type = explicit unless Automatic, then inferred. |
| `Column.Type` (legacy `BindingType`) | 1 Data, 2 Calculated, 3 RowNumber, 4 CalculatedTableColumn (name only in `InferredName`) |
| `Partition.Type` | 1 Query, 2 Calculated, 3 None (internal tables), 4 M, 5 Entity, 6 PolicyRange, 7 CalculationGroup, 8 Inferred; text in `QueryDefinition` |
| `Partition.Mode` / `Model.DefaultMode` | 0 Import, 1 DirectQuery, 2 Default, 3 Push, 4 Dual, 5 DirectLake |
| `Partition.DataView` | 0 Full, 1 Sample, 3 Default |
| `Relationship.From/ToCardinality` (legacy `FromEnd…`) | 1 One, 2 Many |
| `CrossFilteringBehavior` | 1 OneDirection, 2 BothDirections, 3 Automatic |
| `SecurityFilteringBehavior` | 1 OneDirection, 2 BothDirections, 3 None |
| `JoinOnDateBehavior` | 1 DateAndTime, 2 DatePartOnly |
| `SummarizeBy` | 1 Default, 2 None, 3 Sum, 4 Min, 5 Max, 6 Count, 7 Average, 8 DistinctCount |
| `Expression.Kind` | 0 M, 1 DAX |
| `Model.DefaultPowerBIDataSourceVersion` | 0 PowerBI_V1, 1 PowerBI_V2, 2 PowerBI_V3 |
| `Annotation.ObjectType` | 1 Model, 3 Table, 4 Column, 6 Partition, 7 Relationship, 8 Measure, 9 Hierarchy, 10 Level, 13 Culture, 29 Perspective, 34 Role, 36 TablePermission, 41 Expression, 51 QueryGroup |
| `Table.SystemFlags` | bit 0 = internal storage table (`H$` attribute hierarchy, `R$` relationship index, `U$` user hierarchy); 2 = calculated table (incl. auto date tables) |

*(sample)* 84 `Table` rows: 8 with `SystemFlags` 0, 1 with 2 (the calculated
`Calendar`), 75 internal (69 `H$`, 6 `R$`). The model therefore has 9 TOM tables, 6
measures, 1 calculated column, 8 M partitions and 1 calculated-table partition, and 6
single-direction many-to-one relationships from the fact table `Calculations`.
`Expression` and `QueryGroup` are empty. The `.db.xml` gives the database name (GUID)
and `CompatibilityLevel` 1606. The tables with `__PBI_LocalDateTable` /
`__PBI_TemplateDateTable` annotations and the `joinOnDateBehavior: datePartOnly`
relationship of the auto date/time feature are covered by the synthetic metadata tests.

Verification: table names, kinds, column and measure counts, measure names and
relationship counts of all committed samples (and the downloaded AdventureWorks sample,
which has calculation groups) match `reference/samples/microsoft/expected.json`, which
was cross-checked with pbixray (`tests/abf_fixture.rs`).

### 5.2 Storage tables → `StorageMap`

```text
Table ─TableStorageID→ TableStorage ─StorageFolderID→ StorageFolder.Path  ("T (12).tbl")
Partition ─PartitionStorageID→ PartitionStorage (StoragePosition, StorageFolderID "T (12).tbl\15.prt")
    PartitionStorage ─SegmentMapStorageID→ SegmentMapStorage (RecordCount, SegmentCount, RecordsPerSegment)
Column ─ColumnStorageID→ ColumnStorage (StoragePosition, Statistics_*, DictionaryStorageID)
    ColumnStorage ←ColumnStorageID─ ColumnPartitionStorage (one per partition)
        ─StorageFileID→ StorageFile (.idf)
        ─SegmentStorageID→ SegmentStorage (SegmentCount) ─StorageFileID→ StorageFile (.idfmeta)
    ColumnStorage ─DictionaryStorageID→ DictionaryStorage ─StorageFileID→ StorageFile (.dictionary)
Column ─AttributeHierarchyID→ AttributeHierarchy → AttributeHierarchyStorage
        ─StorageFileID→ StorageFile (.hidx), SystemTableID → H$ table
StorageFile = (StorageFolderID → StorageFolder.Path) + "\" + FileName  = logical path
```

`StorageFile.OwnerType` *(sample)*: 22 dictionary, 23 idf, 24 idfmeta, 27 hidx;
`StorageFolder.OwnerType` 18 table folder, 20 partition folder. A `StorageFileID` of 0
means "no file".

`DictionaryStorage` *(sample)*:

| `Type` | `DataType` | Meaning | Rows |
|---|---|---|---|
| 0 | 19 | no dictionary (`H$` and row-number columns) | 111 |
| 1 | 2 | hash dictionary of strings (`DictionaryFlags` 3), `BaseId` 2 | 18 |
| 1 | 6 | hash dictionary of integers, `IsOperatingOn32` = 1 | 14 |
| 1 | 8 | hash dictionary of doubles (DateTime columns: dates are stored as doubles) | 4 |
| 2 | 6 | value encoding: no file, `BaseId` (e.g. −3, −2, 31, 41637), `Magnitude` 1.0 or 0.0001 | 39 |
| 2 | 8 | value encoding of doubles, `BaseId` −2, 41637, 42002, `Magnitude` 1.0 or 10.0 | 3 |

Hash-encoded columns whose only value is BLANK (`Statistics_DistinctStates` = 1) have
**no** dictionary file (`StorageFileID` 0) — *(sample)* one in `Regional Sales
Sample.pbix` (`Opportunities[Blank]`).

`ColumnStorage.Statistics_*`: `DistinctStates` (cardinality), `MinDataID`/`MaxDataID`
(*(sample)* data ids start at 3 for hash dictionaries), `RowCount`, `HasNulls`,
`RLERuns`, `DBType` (OLE DB: 130 WSTR, 20 I8, 7 DATE, 6 CY, 5 R8), `XMType` (0 int, 1
real, 2 string), `CompressionType`. `AttributeHierarchyStorage.MinValue`/`MaxValue` hold data
values and are **not** copied into the map.

*(sample)* every partition has exactly one segment; `SegmentMapStorage.RecordCount` is
the table's row count (e.g. `Opportunity` 486, the calculated `Calendar` 1,096). For the
69 data, calculated and calculated-table columns of the 9 TOM tables every referenced
file exists in the archive, and their decoded values match pbixray's (§7.7).

### 5.3 Legacy XML metadata (compatibility level 1100/1103, `src/metadata/xml.rs`)

Power Pivot workbooks (and SSAS 2012/2014 tabular backups) have no `metadata.sqlitedb`.
Each object is an ASSL `Load` document (`ParentObject` + `ObjectDefinition`, default
namespace `http://schemas.microsoft.com/analysisservices/2003/engine`, newer elements in
`ddl200`/`ddl200_200`/`ddl300`/`ddl300_300` namespaces; matched by local name):

| File | Read |
|---|---|
| `<db>.<n>.db.xml` | `Database/Name`, `CompatibilityLevel` (§5, `database.rs`) |
| `Model.<n>.cub.xml` | `Cube/Name`; `Dimensions/Dimension` (`DimensionID`, `Name`, `Visible`) = tables in order |
| `<dimension id>.<n>.dim.xml` | `Dimension/Description`, `Annotations`; `Attributes/Attribute` (`ID`, `Name`, `Type`, `KeyColumns/KeyColumn/DataType` + `Source` `ColumnBinding`/`ExpressionBinding`/`RowNumberBinding`, `AttributeHierarchyVisible`, `AttributeHierarchyDisplayFolder`, `FormatString`, `OrderBy`/`OrderByAttributeID`); `Hierarchies/Hierarchy/Levels/Level/SourceAttributeID`; `Relationships/Relationship` (`ID`, `Visible`, `From`/`ToRelationshipEnd` with `Multiplicity`, `DimensionID`, `Attributes/Attribute/AttributeID`) |
| `MdxScript.<n>.scr.xml` | `Commands/Command/Text` with `CREATE MEASURE 'T'[M]=<DAX>;` (annotations `Table`, `FullName`) |
| `<id>.<n>.prt.xml` | `ParentObject/MeasureGroupID` (= dimension ID) + `Partition/Name` + `Source` `QueryBinding` (`DataSourceID`, `QueryDefinition`) |
| `<id>.<n>.ds.xml` | `DataSource/ID`, `Name` only (connection strings are never read) |

Key data types: `WChar` → string, `BigInt`/`Integer`/… → int64, `Double` → double,
`Currency` → decimal, `Date` → dateTime, `Boolean`, `Binary`, `Empty` (calculated columns)
→ automatic. `ColumnID`s are XML-encoded (`Plant_x0020_ID` → `Plant ID`).

Each backup-log file group has its own root, so the logical paths of the cube's
measure groups and partitions are relative to *their* folders (several `info.<n>.xml`
share a path); files are therefore found by suffix and attached through `ParentObject`.

*(verified locally)* `Supplier Quality Analysis Sample-no-PV.xlsx`: 8 tables, 29 columns
(2 calculated), 11 measures, 7 relationships, compatibility level 1103 — the same schema
pbixray 0.15.5 reports.

**Storage (`<dim id>.<n>.tbl.xml`):** an `XMSimpleTable` `XMObject` tree (`class`
attribute, `Properties`, `Members/Member{Name, XMObject}`, `Collections/Collection{Name,
XMObject*}`, `DataObjects/DataObject/XMObject`):

* `Members/SegmentMap` (`XMMultiPartSegmentMap`) → `Partitions` (`XMSegment1Map`,
  `Records`) = rows per partition;
* `Collections/Columns` → one `XMRawColumn` per attribute (`name` = attribute ID, e.g.
  `CalculatedColumn1 1`), with `Members/ColumnStats` (`DistinctStates`, `MinDataID`, …),
  `Collections/Segments` → `XMColumnSegment` (`Records`; `Members/CompressionInfo` with
  the class `XMHybridRLECompressionInfo<class XMRENoSplitCompressionInfo<n>>` (members
  `RLECompression` = `BookmarkBits`, `StorageAllocSize`, `StorageUsedSize`,
  `SegmentNeedsResizing`, and `SubCompression`), `XMRENoSplitCompressionInfo<n>` or
  `XM123CompressionInfo`; `SubSegment` (`Records`); `ColumnSegmentStats` = the `SS`
  fields of `.idfmeta`), and `DataObjects`: `XMHashDataDictionary<XM_String|XM_Long|XM_Real>`
  (`name` = the `.dictionary` file; `LastId`, `Nullable`, `Unique`, `DictionaryFlags`)
  or `XMValueDataDictionary<XM_Long|XM_Real>` (`BaseId`, `Magnitude`), and one
  `XMRawColumnPartitionDataObject` per partition (`name` = the `.idf` file,
  `SegmentCount`).

These are exactly the `.idfmeta` fields (§7.1) in XML form; the loader builds the
`ColumnPartitionMeta` inline (`ColumnPartitionFiles::meta`, compression class names mapped
to the `.idfmeta` class codes) and resolves file names with `AbfArchive::find_file_name`
(the `.dim` folder is its own file group). The `.idf` and `.dictionary` files have the
same layout as in Power BI, so the regular decoder applies unchanged. Calculated columns
(key type `Empty`) get their type from the dictionary class. Internal `H$`/`R$` tables
are not mapped.

*(verified locally, not committed)* both sample workbooks decode without a diagnostic;
the 5 tables of the Supplier Quality sample for which pbixray ships reference CSVs
(`Category`, `Defect Type`, `Material Type`, `Metrics` (6,145 rows), `Plant`) are
identical row for row (as multisets; `Metrics` is in storage order).

## 6. Notes for the VertiPaq column decoder (`m3-vertipaq`)

Pre-implementation notes from `m3-abf`, kept for history. The implemented and verified
behaviour is in §7 (which supersedes the points below where they differ: BLANK is data
id 2 and dictionaries start at data id 3, not at the segment `min_data_id`).

Per column (`StorageMap` → `ColumnStorage`), for each `partitions[i]` in storage order:

1. `archive.read(idfmeta)` → per-segment metadata (pbixray `idfmeta.ksy`, CC0): tagged
   `<1:CP` / `<1:CS` / `<1:SS` / `<1:SDOs` blocks with segment record count, `min_data_id`,
   `has_nulls`, `bit_width`, the compression class (`0xABA5A` hybrid RLE with a
   bit-packed sub-segment, `0xABA37…0xABA56` fixed 1–32-bit packing, `0xABA57` general,
   `0xABA5B` "123") and the sub-segment record count.
2. `archive.read(idf)` → segments (`idf.ksy`): per segment a u64 primary-segment entry
   count, then (u32 data value, u32 repeat) RLE pairs, where data value + offset
   `0xFFFFFFFF` means "take the next `repeat` values from the bit-packed sub-segment";
   then a u64 sub-segment size in 8-byte words and the words (`64 / bit_width` values per
   word, add `min_data_id`). Cap the decoded rows at the segment's record count (pbixray
   notes garbage padding in some writers) and at `ColumnPartitionFiles::record_count`.
3. Data ids → values via `dictionary`:
   * `DictionaryEncoding::Value`: `value = (data_id + base_id) / magnitude` (integers;
     `magnitude` 1.0 for most columns; see §7.3).
   * `DictionaryEncoding::Hash` with a path: `archive.read(dictionary_path)`
     (`dictionary.ksy`, which follows [MS-XLDM] §2.3 column dictionaries): a leading
     `dictionary_type` (−1 invalid, 0 long, 1 real, 2 string — same values as
     `Statistics_XMType`); string dictionaries are pages of NUL-separated UTF-16LE strings,
     optionally Huffman-compressed (xmhuffman, MIT) and addressed by (u32 offset, u32
     page) record handles; numeric dictionaries are vectors of 4- or 8-byte elements
     (`is_operating_on_32` ⇒ 4). pbixray numbers dictionary entries from the segment
     `min_data_id`.
   * Hash without a path: every row is BLANK.
4. Logical type conversions (`ColumnStorage::data_type`): DateTime is stored as an OLE
   automation date (double days since 1899-12-30); Decimal (currency) as int64 × 10⁴;
   Boolean as int; String via the dictionary. Data id 0/1 handling for BLANK/null:
   check `Statistics_HasNulls` and `DictionaryStorage::is_nullable`.
5. `.hidx` (hash index, `AttributeHierarchyStorage::path`) is not needed to read values;
   sort order lives in the `H$` table given by `system_table_id`
   (`column_position_to_data` / `column_data_to_position` name its columns).
6. Row count of a table = `TableStorage::row_count`; skip `!has_values()` columns
   (row-number) and tables with `is_internal`.
7. `AbfArchive` is `Clone + Send + Sync` (shared `Arc`), so columns can be decoded in
   parallel with zero-copy `&[u8]` slices.

## 7. VertiPaq column decoding (`src/vertipaq/`)

Sources: [MS-XLDM] §2.3 (column data storage, dictionaries) and §2.7 (XMRENoSplit,
XMHybridRLE, Huffman), pbixray `vertipaq_decoder.py` + `idfmeta.ksy` / `idf.ksy` /
`dictionary.ksy`, xmhuffman `xmhuffman_kernel.c` (bit order), and the samples. All
integers little-endian.

### 7.1 `.idfmeta` (per column partition)

```text
"<1:CP\0" u64 segment_count  CS × n  "CP:1>\0"  ["<1:SDOs\0" CSDOs × n "SDOs:1>\0"]
CS   "<1:CS\0" u64 records u64 base_id u32 class u32 sub_class
     [class 0xABA5A: u64 bookmark_bits u64 alloc u64 used u8 needs_resizing]
     u32 first_run_value  SS  u8 has_subsegment  [subsegment CS: tag u64 records
     u64 base_id u8 0 "CS:1>\0"]  "CS:1>\0"
SS   "<1:SS\0" u64 distinct u32 min_data_id u32 max_data_id u32 original_min i64
     rle_sort_order u64 row_count u8 has_nulls u64 rle_runs u64 others "SS:1>\0"
```

Classes: `0xABA36 + n` = `XMRENoSplit<n>` (n = 1..10, 12, 16, 21, 32 defined; any
1..=32 accepted), `0xABA5A` hybrid RLE (inner class in `sub_class`), `0xABA57` general,
`0xABA5B` XM123 (row-number column only). `CS.records` is the authoritative row count;
the sum over segments must equal the partition's `SegmentMapStorage.RecordCount`
(checked). The `base_id` fields are not the bit-packing base (observed 0/1).
*(sample)* every user column has one hybrid segment (`0xABA5A`) with sub classes
`0xABA37`–`0xABA3F` (1–9 bits), `0xABA42` (12 bits) or `0xABA56` (32 bits), and an SDOs
block.

### 7.2 `.idf`

Per segment, in `.idfmeta` order: hybrid = `u64 n` + `n × (u32 value, u32 repeat)`
(primary/RLE segment) + `u64 m` + `m × u64` (bit-packed subsegment); plain
`XMRENoSplit` = `u64 m` + `m × u64` (spec-derived; no sample uses it). A primary entry with
`value + k == 0xFFFF_FFFF` — `k` = bit-packed values consumed so far in the segment,
i.e. the spec's negative 1-based offset — takes its `repeat` rows from the subsegment;
any other entry is an RLE run. Runs are capped at `records` (trailing slots are zeroed
allocation padding; some writers leave stale values) and must cover exactly `records`.
Words hold `64 / bit_width` values, low bits first (no value spans two words); a value
is `word bits + base`, where **base = 2 if the segment `has_nulls`, else
`SS.min_data_id`** (`min_data_id` excludes NULL). A subsegment of one zero word with
more packed rows than it can hold means "every value equals the base" (observed by
pbixray). A typical primary segment is `[(v, n), (0xFFFF_FFFF − k, m), …, (0, 0)…]`:
RLE runs and bit-packed stretches, padded with zero slots.

### 7.3 Data ids

* **2 = NULL / BLANK** (`XM_DATA_ID_NULL`), always; ids 0 and 1 never occur (invalid).
* Hash encoding: dictionary entry `i` has id `3 + i`, shared by all partitions (not
  derived from any segment's `min_data_id`). Ids past the dictionary are invalid.
* Value encoding (`DictionaryStorage.Type` 2, no file): `value = (id + BaseId) /
  Magnitude`; magnitude 1 keeps integers exact, other finite non-zero magnitudes give a
  double. *(sample)* `BaseId` −2, 31, 41637, 42002 (DateTime as days), `Magnitude` 1,
  0.0001 (an integer column stored in ten-thousandths) and 10.
* Hash encoding without a dictionary file: every row BLANK (*(sample)*
  `Opportunities[Blank]` of `Regional Sales Sample.pbix`, all ids 2; pbixray also yields
  all-null).
* Invalid ids (or values that cannot be converted, e.g. NaN to Int64): error by default
  (`InvalidIdPolicy::Error`), or BLANK + `VPQ002` warning (`InvalidIdPolicy::Blank`).

### 7.4 `.dictionary`

`i32 type` (0 long, 1 real, 2 string, −1 invalid) + 6 × `i32` hash info, then:

* long/real: `u64 count`, `u32 element_size` (long 4 or 8 → widened to i64; real 8;
  other sizes rejected), values. Trailing zero padding is ignored.
* string: `i64 string_count`, `i8 any_compressed`, `i64 longest`, `i64 page_count`
  (≤ 524,288), pages, then the record-handle vector `u64 count`, `u32 8`,
  `count × (u32 offset, u32 page)`. Page: `u64 mask` (1 = compressed), `u8
  contains_nulls`, `u64 start_index`, `u64 string_count`, `u8 compressed`,
  `DD CC BB AA`, store, `CD AB CD AB`.
  * Uncompressed store: `u64 remaining_chars`, `u64 used_chars`, `u64 allocation_bytes`,
    UTF-16LE buffer; the first `used_chars` units are NUL-terminated strings (the rest
    is slack). *(observed)* handle offsets of uncompressed pages count UTF-16 units
    (spec: bytes), so we split at NULs instead of trusting them.
  * Compressed store: `u32 total_bits`, `u32 mode` (703121 single charset, 703122
    multi), `u64 allocation`, `[u8 charset_byte]` (single only), `u32 decode_bits`
    (lookup hint, unused), 128-byte `encode_array` (4-bit code lengths, symbol `2i` in
    the low nibble of byte `i`), `u64 buffer_size`, buffer (`allocation` bytes).
    Canonical Huffman over bytes (lengths ≤ 15; over-subscribed codes rejected,
    incomplete codes accepted until an unused codeword is hit). Bit stream = 16-bit
    little-endian words read MSB first (swap each byte pair, read MSB first; a final odd
    byte is not swapped). String `k` of a page spans `[offset_k, offset_{k+1})` of the
    page's handles in handle order, the last ends at `total_bits`. Single charset: code
    unit = `charset_byte << 8 | symbol`; multi: symbols are UTF-16LE bytes (odd tail
    dropped). Invalid UTF-16 → U+FFFD.
  * Handle `k` = dictionary entry `k`; its string is the next unused string of its page.
* *(sample)* 36 dictionaries, all string pages uncompressed; `Regional Sales
  Sample.pbix` has one single-charset compressed page (charset byte 0) and the downloaded
  AdventureWorks sample seven. The multi-charset mode is covered by synthetic tests.
  Empty strings are distinct from BLANK (*(observed)* e.g. a one-entry `""` dictionary
  with NULL rows).

### 7.5 Types (`ColumnStorage::data_type` → Arrow, as `pbi-engine`)

| Logical | Arrow | Conversion |
|---|---|---|
| String | `Utf8` | dictionary text; numbers formatted |
| Int64 | `Int64` | integer; reals rounded |
| Double | `Float64` | as is |
| Decimal | `Decimal128(19, 4)` | stored integer = value × 10⁴ is the raw decimal |
| DateTime | `Timestamp(Microsecond, None)` | OLE days since 1899-12-30: whole days × 86 400 000 000 + round(fraction × 86 400 000 000) − 25 569 days |
| Boolean | `Boolean` | non-zero = true |
| Binary | `Binary` | base64 text (strict RFC 4648, whitespace ignored) |
| Variant / Automatic / unknown | `Int64` / `Float64` / `Utf8` by storage | |

A string dictionary behind a numeric/date logical type is returned as `Utf8` (no
parsing). Fully BLANK columns keep the logical Arrow type.

### 7.6 Tables and limits

`decode_table` = model columns in model order (row-number column skipped), names from
the model, partitions concatenated in storage order; each column must have exactly
`TableStorage::row_count` rows. Columns are decoded on up to 8 scoped threads for
tables with ≥ 65,536 rows. `decode_all_with_diagnostics` / `open_pbix_data` replace a
failing column with an all-BLANK column (`VPQ001`), skip tables that fail or whose
columns all fail (`VPQ003`), and stop at `max_total_bytes` (`VPQ004`).
`DecodeLimits::DEFAULT`: 2²⁸ rows per column/table (an RLE run can declare any row
count), 2³¹ dictionary entries, 4 GiB dictionary text, 2 GiB per string column
(`Utf8` offsets are `i32`), 16 GiB total; `SMALL` for fuzzing.

### 7.7 Verification and performance

*(samples)* Every table and column of the three committed samples (9 + 10 + 11 tables,
69 + 36 + 70 columns: every logical type but Binary, value encodings with magnitudes
0.0001 and 10, an all-BLANK column, calculated tables) and of the downloaded
AdventureWorks sample (10 tables incl. two calculation groups, 62 columns): row count,
Arrow type, null count, distinct count and SHA-256 of a canonical serialization
identical to pbixray 0.15.5
(`tools/reference_hashes.py`; only hashes/counts are stored, in
`tests/data/reference_hashes/`). Every dictionary has `Statistics_DistinctStates`
entries (minus one when the column has nulls).

Performance (Apple Silicon, release): `open_pbix_data` of the sample (XPress9, ABF,
metadata and every column) ~4.9 ms median of 30 (perf scenario P06). Synthetic 10 M rows
(10-bit hybrid, 1,000-entry Huffman dictionary): IDF ~5 ms, `Utf8` materialisation
~52 ms, `DateTime` ~17 ms. ~150k mutated real/synthetic inputs ran through
`fuzz_vertipaq` and full decoding without a panic.

Not implemented: `XM123` (row-number column, never decoded), `XMREGeneral` and unknown
compression classes (`Unsupported` error), real dictionaries with 4-byte elements,
the `.hidx` hash index and `H$`
sort-order tables (not needed for values), `DictionaryArray` output.
