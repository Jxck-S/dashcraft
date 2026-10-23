# pbi-vertipaq — references and licences

This file records every external reference consulted for `pbi-vertipaq`, the licence
we verified for it, and how it was used. It follows the clean-room policy in
`docs/adr/0003-vertipaq-xpress9-cleanroom.md`.

Policy summary:
* All code in this crate is original Rust. No third-party source was copied.
* Permissively licensed (MIT/Apache/BSD) sources may be studied and structurally ported
  with attribution. GPL/LGPL/unknown/proprietary sources are not read; for those we
  only compare outputs (black-box).
* Reference outputs (lengths, hashes) were produced by running the MIT-licensed Python
  packages read-only in a local venv **outside the repository**. The venv was not
  copied into the repo.

## Licence verification method

* PyPI wheels: `*.dist-info/METADATA` (`License`, classifiers) and the bundled `LICENSE`
  file, read in the local venv.
* GitHub: `gh api repos/<owner>/<repo>` (`license.spdx_id`), the `LICENSE` file, and the
  per-file headers of the C sources.
* Date of verification: 2026-10-21.

## References used for the XPress9 decoder (todo `m3-xpress9`)

| # | Reference | URL / version | Licence (verified) | Copyright line | Used for |
|---|---|---|---|---|---|
| 1 | **Microsoft XPress9 reference library** (C) in Extensible Storage Engine | https://github.com/microsoft/Extensible-Storage-Engine/tree/main/dev/ese/src/_xpress9 (last change to that dir `4db40470c4bb5a4a825acda2e65fc2f863bb3167`, 2021-06-02) | **MIT** (repo `LICENSE`; every file starts with `// Copyright (c) Microsoft Corporation. // Licensed under the MIT License.`) | `Copyright (c) Microsoft Corporation.` | Authoritative semantics of the XPress9 bit stream: block header and flags, CRC-32C header check, MTF state, code-length table encoding, canonical Huffman ordering, LZ77 symbol layout, MTF update rules, session/block sequencing, validation rules. **Structural port** (see below). |
| 2 | **xpress9-python** (Cython wrapper + vendored copy of #1) | https://github.com/Hugoberry/xpress9-python @ `ff81bd9f93601650243a2695d1ad5940c7fbadd0`; PyPI `xpress9` 0.3.9 | **MIT** (wrapper: `Copyright (c) 2025 Igor Cotruta`; vendored C files keep the Microsoft MIT headers and are byte-identical in substance to #1) | `Copyright (c) 2025 Igor Cotruta` (wrapper); `Copyright (c) Microsoft Corporation.` (C sources) | Read `Xpress9Wrapper.c` / `xpress9.pyx` to learn how a PBIX chunk maps onto decoder calls (one decoder instance per session, one block per `Decompress` call). Ran the binary read-only to produce the fixture reference hash and to **generate the synthetic test vectors** in `tests/data/` (encoder run on our own synthetic plaintext). |
| 3 | **pbixray** (Python) | https://github.com/Hugoberry/pbixray @ `c5da940b45d2806c8f8d3e109f0d1909117a7db0`; PyPI 0.15.5 | **MIT** | `Copyright (c) 2024 Igor Cotruta` | `pbixray/loader.py`: PBIX `DataModel` container framing — the 102-byte UTF-16LE signatures, single-threaded chunk loop, multi-threaded header (5 × u64) and prefix/main group order, uncompressed `STREAM_STORAGE_SIGNATURE` detection. `pbixray/abf/*.py` was skimmed only to describe the ABF layout in `FORMAT-NOTES.md` for the next todo (no code ported). Ran read-only to compute reference output. |
| 4 | **[MS-XLDM] Spreadsheet Data Model File Format** | https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-xldm/ (section "Xpress Compression", `3e3dac06-3f45-48a5-93d4-abacd4b34000`) | Microsoft Open Specifications documentation — *Intellectual Property Rights Notice for Open Specifications Documentation*: copies may be made to develop implementations; patent coverage via the Microsoft Open Specifications Promise / Community Promise where the document lists it | © Microsoft Corporation | Confirmed only that the spec does **not** describe XPress9: it says the model file is compressed with "Xpress compression as specified in [MS-WUSP] 2.1.1", i.e. the older Xpress codec, not the XPress9 format found in PBIX files. Not used for code. |
| 5 | **[MS-XCA] Xpress Compression Algorithm** | https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-xca/ | Microsoft Open Specifications documentation (as #4) | © Microsoft Corporation | Checked scope only: covers LZNT1, Plain LZ77 and LZ77+Huffman — **not** XPress9. Not used for code. |
| 6 | `sha2` crate 0.10.9 (dev-dependency) | https://crates.io/crates/sha2 | MIT OR Apache-2.0 | The RustCrypto Developers | SHA-256 of the decompressed sample `DataModel` streams in `tests/xpress9_fixture.rs` (tests only). |

### What "structural port" means here (reference #1)

The decoder in `src/xpress9/` re-implements the *behaviour* of Microsoft's
`Xpress9DecLz77.c`, `Xpress9Lz77Dec.i`, `Xpress9DecHuffman.c`, `Xpress9Misc.c`
(CRC-32C) and `Xpress9Internal.h` (constants, header layout) in idiomatic, safe Rust,
with a different architecture:

* single-level 12-bit lookup + canonical slow path instead of the reference's multi-level
  `HUFFMAN_DECODE_SYMBOL` tables;
* an LSB-first 64-bit bit reader with zero padding instead of the scratch-area/half-word
  shift register;
* direct append into a growable session buffer instead of the fixed 1.5×window ring with
  `Tail` handling and incremental "fetch" API;
* explicit, fallible bounds checks everywhere (the C code relies on `RETAIL_ASSERT` and
  caller-provided sizes).

Constants that define the format (magic `0x4E86D72A`, flag bit layout, alphabet sizes
704/256, long-length threshold 232, meta-alphabet opcodes 28..=32, zero-run minimum 5,
fill boundary 16, max codeword length 27, CRC-32C polynomial) are facts of the format
and are necessarily identical. The CRC table is generated at compile time from the
polynomial rather than copied.

### Text for `THIRD_PARTY_NOTICES.md`

Add the following when the root notices file is assembled (MIT requires the copyright
and permission notice to accompany substantial portions; we include it out of caution
for the structural port):

```
pbi-vertipaq (XPress9 decoder) is a Rust re-implementation of the decoding rules of the
Microsoft XPress9 reference library:
  https://github.com/microsoft/Extensible-Storage-Engine/tree/main/dev/ese/src/_xpress9
  Copyright (c) Microsoft Corporation.
  Licensed under the MIT License.

The PBIX DataModel container framing follows pbixray:
  https://github.com/Hugoberry/pbixray
  Copyright (c) 2024 Igor Cotruta
  Licensed under the MIT License.

<full MIT licence text>
```

## References used for the ABF container and metadata reader (todo `m3-abf`)

IDs continue the table above; #3 and #4 are reused.

| # | Reference | URL / version | Licence (verified) | Copyright line | Used for |
|---|---|---|---|---|---|
| 3 | **pbixray** (Python) | https://github.com/Hugoberry/pbixray @ `c5da940b45d2806c8f8d3e109f0d1909117a7db0`; PyPI 0.15.5 (local venv) | **MIT** | `Copyright (c) 2024 Igor Cotruta` | Read `pbixray/abf/{parser,backup_log_header,virtual_directory,backup_log,data_model}.py` and `utils.get_data_slice`: header at offset 72 in the first 4 KiB page, virtual directory at `m_cbOffsetHeader`/`DataSize`, log = last directory entry, `ErrorCode` ⇒ drop 4 trailing bytes, `ApplyCompression` ⇒ per-file XPress8, join `StoragePath` → directory `Path`, strip the persist location. Read `pbixray/meta/sqlite_source.py` (in-memory deserialize of `metadata.sqlitedb`; the storage join `Column → ColumnStorage → ColumnPartitionStorage → StorageFile`, `DictionaryStorage`, `AttributeHierarchy(Storage)`, `PartitionStorage`; legacy-schema fallbacks `BindingType`, `FromEnd…/ToEnd…`, `Level.Index`; `ExplicitDataType = 1 ⇒ InferredDataType`) and `pbixray/vertipaq_decoder.py` (only to write the notes for `m3-vertipaq` in `FORMAT-NOTES.md` §6). **Structural port** of the container join and the storage-map join in original Rust (different architecture: zero-copy archive, generic schema-tolerant row reader, exact folder+file paths instead of file-name matching). Ran read-only on the fixture to produce reference outputs (file list, schema, hashes of M/DAX texts) for verification; outputs stayed outside the repository. |
| 7 | **pbixray Kaitai specs and docs** (`docs/abf_xpress9.ksy`, `idfmeta.ksy`, `idf.ksy`, `dictionary.ksy`, `TMSCHEMA_MAPPING.md`, `metadata.sqlitedb.csv`) | https://github.com/Hugoberry/pbixray/tree/c5da940b45d2806c8f8d3e109f0d1909117a7db0/docs | `.ksy` files declare `meta.license: CC0-1.0`; repository MIT (treated as MIT) | `Copyright (c) 2024 Igor Cotruta` | `TMSCHEMA_MAPPING.md` / `metadata.sqlitedb.csv`: names of metadata tables and columns. `abf_xpress9.ksy` covers only the XPress9 wrapper (nothing new for ABF). `idfmeta.ksy`, `idf.ksy`, `dictionary.ksy` skimmed **only** to describe the next todo's inputs in `FORMAT-NOTES.md` §6; no code ported. |
| 4 | **[MS-XLDM]** | https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-xldm/ revision 8.3; pages `49ca8d56…` (§2.1 file format: 4096-byte pages), `d3e98d19…` (`BackupLogHeaderType`), `e10e8481…` (files section, CRC markers, virtual directory after the last page), `b379742e…` / `2baccaf9…` (CRC end marker and algorithm), `5440b19d…` (log file) | Microsoft Open Specifications IP Rights Notice; listed in the Open Specifications Promise (see ADR-0003) | © Microsoft Corporation | Authoritative header field list and semantics (`ErrorCode` = CRC end markers, `ApplyCompression`, `EncryptionFlag`), page alignment, order of the files section. |
| 8 | **TOM enumerations** (`Microsoft.AnalysisServices.Tabular`: `ObjectType`, `DataType`, `ColumnType`, `PartitionSourceType`, `ModeType`, `RelationshipEndCardinality`, `CrossFilteringBehavior`, `SecurityFilteringBehavior`, `AggregateFunction`, `DateTimeRelationshipBehavior`, `PowerBIDataSourceVersion`, `ModelPermission`, `DataViewType`, `EncodingHintType`, `ExpressionKind`) | https://learn.microsoft.com/en-us/dotnet/api/microsoft.analysisservices.tabular (package 19.118.3), retrieved 2026-10-21 | Microsoft Learn terms of use (UNVERIFIED for copying text; ADR-0003) | © Microsoft Corporation | **Facts only**: numeric enum values used in `src/metadata/codes.rs` (cross-checked with pbixray and the fixture). No text copied. |
| 9 | `roxmltree` crate 0.21.1 (dependency) | https://crates.io/crates/roxmltree | MIT OR Apache-2.0 | Yevhenii Reizner | Read-only XML DOM for the header, virtual directory, log and `.db.xml` (no DTDs, node limit). Already in `Cargo.lock` via `usvg`. |
| 10 | `rusqlite` 0.40.2 with feature `serialize` (workspace dependency, bundled SQLite) | https://crates.io/crates/rusqlite | MIT (SQLite: public domain) | The rusqlite developers | `Connection::deserialize_read_exact` opens `metadata.sqlitedb` from memory. Its `Statement::column_names()` and typed `Row::get` panic on non-UTF-8 column names, so the reader uses `PRAGMA table_info` and `get_ref` instead. |

No copyleft or unknown-licence source was read for this todo. No Microsoft binary was
inspected.

### Text for `THIRD_PARTY_NOTICES.md` (addition)

```
pbi-vertipaq (ABF container and metadata reader) follows the structure of pbixray's
ABF parser and metadata.sqlitedb queries:
  https://github.com/Hugoberry/pbixray
  Copyright (c) 2024 Igor Cotruta
  Licensed under the MIT License.
roxmltree: MIT OR Apache-2.0.
```

## References used for the VertiPaq column decoder (todo `m3-vertipaq`)

IDs continue the tables above; #3, #4, #6 and #7 are reused.

| # | Reference | URL / version | Licence (verified) | Copyright line | Used for |
|---|---|---|---|---|---|
| 3 | **pbixray** (Python) | https://github.com/Hugoberry/pbixray @ `c5da940b45d2806c8f8d3e109f0d1909117a7db0`; PyPI 0.15.5 (local venv, `pbixray-0.15.5.dist-info/METADATA`: `License: MIT`) | **MIT** | `Copyright (c) 2024 Igor Cotruta` | Read `vertipaq_decoder.py`, the Kaitai-generated `column_data/{idf,idfmeta,dictionary}.py` and `meta/sqlite_source.py::get_segment_meta`. **Structural port** of the decoding rules: NULL = data id 2 and dictionaries start at id 3 for every column/segment; bit-packing base = 2 for segments with nulls, else `SS.min_data_id`; the sliding `0xFFFFFFFF − offset` bit-packing sentinel; capping RLE runs at the segment's `records`; the one-zero-word subsegment case; per-page string assembly; `(id + BaseId) / Magnitude`; Currency = stored / 10⁴; DateTime origin 1899-12-30. Our architecture differs (u32 id vectors, typed Arrow builders, record-handle-driven string assembly, explicit limits and errors). **Ran read-only** via `tools/reference_hashes.py` to produce per-table row counts and per-column null/distinct counts and SHA-256 fingerprints of the Microsoft sample PBIX files (only those are stored, in `tests/data/reference_hashes/`, read by `tests/vertipaq_fixture.rs`). |
| 7 | **pbixray Kaitai specs** `docs/idfmeta.ksy`, `docs/idf.ksy`, `docs/dictionary.ksy` | https://github.com/Hugoberry/pbixray/tree/c5da940b45d2806c8f8d3e109f0d1909117a7db0/docs | `meta.license: CC0-1.0`; repository MIT | `Copyright (c) 2024 Igor Cotruta` | Field layouts of `.idfmeta` (CP/CS/SS/SDOs tags, RLE facet, compression class ids `0xABA37…0xABA5B`), `.idf` and `.dictionary` (hash info, page layout, compressed/uncompressed stores, record handles), implemented in `idfmeta.rs`, `idf.rs`, `dictionary.rs`. |
| 11 | **xmhuffman-cython** (Cython + C kernel) | https://github.com/Hugoberry/xmhuffman-cython @ `d17fa56add3b4e96535f001bd18116af4daa287c`; PyPI `xmhuffman` 0.3.2 (binary in the local venv) | **MIT** (`gh api repos/Hugoberry/xmhuffman-cython` → `MIT`; `LICENSE`; wheel `METADATA`) | `Copyright (c) 2026 Igor Cotruta` | Read `src/xmhuffman_kernel.c` and `xmhuffman.pyx`: nibble-packed 128-byte `encode_array` → 256 code lengths, canonical code assignment, Kraft check, byte-pair swap + MSB-first reading, single-charset high-byte reinsertion, string end = next offset / `store_total_bits`. **Structural port** in `huffman.rs` (single `2^max_len` table like the kernel; we accept incomplete codes, read through a bounds-checked window instead of an unaligned 64-bit load, and cap output). |
| 4 | **[MS-XLDM]** | https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-xldm/ — §2.3.1 `4d3887f8…`, §2.3.1.1.1–3 `c049dea7…`, `21b1aa15…`, `66a47541…`; §2.3.2 `d6de072d…`; §2.3.2.1.1.1–2 `7ee54b7d…`, `d40c9bcc…`; §2.3.2.1.2.3–5 `de4ba19d…`, `7a7bef7c…`, `64985b36…`, `410567de…`; §2.3.2.1.3.2–5 `7adea9d0…`, `e216fe78…`, `77780429…`, `b0d954db…`; §2.3.4.1 `a9c68abf…`; §2.7.1 `85c2024e…`, §2.7.1.1 `070c724e…`; §2.7.2 `c6e2e967…`; §2.7.3 `91ec6bb2…`, §2.7.3.1 `ef29562d…`; §2.7.4 `f70b41f2…` and §2.7.4.1.1–5 (retrieved 2026-10-21) | Microsoft Open Specifications IP Rights Notice / Open Specifications Promise (ADR-0003) | © Microsoft Corporation | Authoritative semantics: segment/subsegment layout and size rules, XMRENoSplit packing (low bits first, `Min` offset, no value across words), hybrid RLE entries and negative bit-packing offsets, zero padding, dictionary vectors, page size/count limits, page mask, Huffman modes 703121/703122, code lengths 2–15, record handles. |
| 6 | `sha2` 0.10.9 (dev-dependency, unchanged) | https://crates.io/crates/sha2 | MIT OR Apache-2.0 | The RustCrypto Developers | Column fingerprints in `tests/vertipaq_fixture.rs`. |

No new crate dependencies. Base64 decoding for `Binary` columns is a small original
implementation of RFC 4648 (no third-party code). No copyleft or unknown-licence source
was read for this todo; no Microsoft binary was inspected.

`tools/reference_hashes.py` imports pbixray from a separate local environment (it is not
a project dependency) and prints only counts and hashes.

### Text for `THIRD_PARTY_NOTICES.md` (addition)

```
pbi-vertipaq (VertiPaq column decoder) follows the decoding rules of pbixray and the
Huffman string-page decoding of xmhuffman:
  https://github.com/Hugoberry/pbixray
  Copyright (c) 2024 Igor Cotruta
  Licensed under the MIT License.
  https://github.com/Hugoberry/xmhuffman-cython
  Copyright (c) 2026 Igor Cotruta
  Licensed under the MIT License.
```

## References used for per-file compression, legacy XML metadata and the §5 readers (todos `m3-pbix-data`, `m5-formats-ext`)

IDs continue the tables above; #3, #4, #5 and #9 are reused. Verified 2026-10-21.

| # | Reference | URL / version | Licence (verified) | Copyright line | Used for |
|---|---|---|---|---|---|
| 5 | **[MS-XCA] Xpress Compression Algorithm** | https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-xca/ §2.3 "Plain LZ77 Compression", §2.4 "Plain LZ77 Decompression" (pseudocode) | Microsoft Open Specifications documentation (as #4) | © Microsoft Corporation | **Authoritative algorithm** of `src/xpress8.rs`: flag words, literal/match tokens, offset/length encoding, shared length nibble, byte/u16/u32 length extensions. Written from the spec. |
| 12 | **xpress8-python** (Cython wrapper + vendored copy of Microsoft ESE `_xpress`) | https://github.com/Hugoberry/xpress8-python (default branch, 2026-10-21); PyPI `xpress8` | **MIT** — the `LICENSE` file is the MIT text (`Copyright (c) 2026 Igor Cotruta`) plus a note that the vendored `xdecode.c`/`xdecode.i`/`xencode.*`/`xpress.h`/`xprs.h` are Microsoft's MIT-licensed ESE files; GitHub's licence detector reports "Other" only because of that appended note (`pyproject.toml`: `license = {text = "MIT"}`, classifier `MIT License`) | `Copyright (c) 2026 Igor Cotruta`; `Copyright (c) Microsoft Corporation.` | `xpress8.pyx` `decompress_chunked`: the chunk framing (u16 uncompressed + u16 compressed size, stored when equal); `Xpress8Wrapper.c`: one independent decode per chunk; `xprs.h` (`CODING_ALG=1` → `CODING_DIRECT2`, 13-bit offset / 3-bit length) and `xdecode.c`/`xdecode.i` (DIRECT2 loop) read to **confirm** that the codec is MS-XCA Plain LZ77 and how the reference treats the end of output (truncates a final match). No code ported; previously listed as "not consulted" on the basis of GitHub's licence label, now verified. |
| 1 | **Microsoft ESE `_xpress`** (as vendored in #12) | https://github.com/microsoft/Extensible-Storage-Engine/tree/main/dev/ese/src/_xpress | **MIT** (file headers) | `Copyright (c) Microsoft Corporation.` | Cross-check only (see #12). |
| 3 | **pbixray** (Python) | https://github.com/Hugoberry/pbixray (default branch, 2026-10-21) | **MIT** | `Copyright (c) 2024 Igor Cotruta` | `pbixray/utils.py::get_data_slice` (CRC marker then per-file `decompress_chunked`, size checked against the log), `pbixray/loader.py` (XLSX `xl/model/item.data` handling), `pbixray/meta/xml_source.py` and `pbixray/xldm/*.py` (which ASSL documents hold tables, attributes, relationships, measures, partitions; `CREATE MEASURE` in the MDX script; `ColumnBinding` IDs). Our XML reader (`src/metadata/xml.rs`) is original code with a different design (namespace-agnostic `roxmltree` lookups, `ParentObject`-based ownership). The two Power Pivot workbooks and the reference CSVs (`data/xlsx/*.csv`) in pbixray's `data/` folder were used **locally, read-only and outside the repository** to verify decompression, schema and decoded rows; neither they nor any output derived from them were committed. The `.tbl.xml` → segment-metadata mapping was derived from the files themselves and our `.idfmeta` notes (§7.1), not from pbixray code. |
| 13 | **VertiPaq Analyzer** (`Dax.Metadata`, `Dax.Vpax`, `Dax.ViewVpaExport`) | https://github.com/sql-bi/VertiPaq-Analyzer (master, 2026-10-21) | **MIT** (`LICENSE.md`; `gh api repos/sql-bi/VertiPaq-Analyzer` → `spdx_id: MIT`) | `Copyright (c) 2019 SQLBI` | Used by `pbi-core` (`reader/vpax.rs`): VPAX part names (`VpaxFormat.cs`), `DaxModel.json` serializer settings (`ExportVpax.cs`: `PreserveReferencesHandling.All`, `DefaultValueHandling.Ignore`), property names of `Model`, `Table`, `Column`, `ColumnSegment`, `ColumnHierarchy`, `Measure`, `Partition` (enum values), `Relationship`, `UserHierarchy`, the `DaxName`/`DaxNote`/`DaxExpression` converters, and how VertiPaq Analyzer derives table/column sizes. Facts only; no code ported. |
| 14 | **Microsoft Fabric dataflow definition** | https://learn.microsoft.com/en-us/rest/api/fabric/articles/item-management/definitions/dataflow-definition | Microsoft Learn terms of use (facts only) | © Microsoft Corporation | Field names of `queryMetadata.json` (`queriesMetadata`, `queryGroups`, `connections`) used by `pbi-core` (`reader/mashup.rs`). |
| 9 | `roxmltree` 0.21.1 | https://crates.io/crates/roxmltree | MIT OR Apache-2.0 | Yevhenii Reizner | Also used by `pbi-core` for RDL (DTDs refused, node limit). |

The RDL element names follow the published Report Definition Language schemas
(`…/sqlserver/reporting/<yyyy>/<mm>/reportdefinition`); CDM `model.json` and `.pbiviz`
field names were taken from publicly documented examples. No copyleft or unknown-licence
source was read; no Microsoft binary was inspected. There is still no encoder: test
vectors for `xpress8` are stored chunks plus hand-assembled LZ77 tokens
(`xpress8::test_support`).

### Text for `THIRD_PARTY_NOTICES.md` (addition)

```
pbi-vertipaq (per-file Xpress decompression) implements [MS-XCA] Plain LZ77; the ABF
chunk framing follows xpress8-python:
  https://github.com/Hugoberry/xpress8-python
  Copyright (c) 2026 Igor Cotruta
  Licensed under the MIT License.

pbi-core (VPAX reader) follows the DaxModel.json schema of VertiPaq Analyzer:
  https://github.com/sql-bi/VertiPaq-Analyzer
  Copyright (c) 2019 SQLBI
  Licensed under the MIT License.
```

## References identified but NOT consulted

| Reference | Licence | Why not used |
|---|---|---|
| `arkev/xpress9` (GitHub) | **none declared** (unknown) | No licence → not read, per policy. |
| `kaitaistruct` runtime 0.11 | MIT | Not needed (the generated parsers were read for `m3-vertipaq`, see #3). |
| `duckdb-pbix-extension` (Hugoberry) | MIT | Not needed. |
| `.hidx` hash-index layout (pbixray `column_data/hidx.py`, [MS-XLDM] §2.3.3) | MIT / Open Spec | Not needed to read values. |
| Any GPL/LGPL code | — | Not searched for, not read. |

## Open points for legal review

* The MIT licence has no express patent grant. Microsoft published XPress9 under MIT and
  ships the format in Power BI; we consider the residual patent risk low but it should
  be part of the legal review of the UNVERIFIED items in ADR-0003.
* The synthetic test vectors in `tests/data/*.bin` are encoder *outputs* of the MIT
  library on our own synthetic plaintext (no customer data); `gen_xpress9_vectors.py`
  documents their provenance.
