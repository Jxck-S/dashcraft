# ADR-0003: Clean-room XPress9, ABF and VertiPaq decoding

- Status: Accepted (license research is recorded below; items marked UNVERIFIED need legal review before a release ships M3 decoding)
- Date: 2026-10-21
- Deciders: pbiview maintainers
- Related: `crates/pbi-vertipaq/REFERENCES.md` (per-source references kept by the XPress9/VertiPaq implementer), [THIRD_PARTY_NOTICES.md](../../THIRD_PARTY_NOTICES.md)

## Context

The PBIX `DataModel` entry is an Analysis Services backup (ABF) compressed with XPress9. A real PBIX `DataModel` starts with the UTF-16LE text `This backup was created using XPress9 compression…`. Inside the ABF:

- a backup log and a virtual directory list the files;
- `metadata.sqlitedb` holds the model schema;
- VertiPaq column files hold the data: `.idf` (bit-packed/RLE segments), `.idfmeta`, `.dictionary`, and `.hidx` (hash index).

The same ABF container appears in standalone `.abf` files, in PBIP `.pbi/cache.abf`, and in Power Pivot workbooks (`xl/model/item.data`).

Microsoft does not document XPress9. The ABF container and VertiPaq file layouts are partly documented in [MS-XLDM]. The best working reference is `pbixray` (Python), which depends on Python bindings around a Microsoft C implementation of XPress9.

The project requirements call for a license check before porting, and a clarifying question if licensing conflicts with project policy. The user chose a clean-room Rust implementation, with references used only under the rules below.

## Decision

1. Implement XPress9 decompression, ABF parsing and VertiPaq decoding **in Rust** in `pbi-vertipaq`. No FFI to C, no vendored third-party code, and no `unsafe` (the workspace denies `unsafe_code`).
2. **Decoders only.** No XPress9 or ABF encoders. Writing PBIX is out of scope. PBIP is the future save format ([ADR-0004](0004-desktop-ready-architecture.md)).
3. Order of preference for references:
   1. Microsoft Open Specifications ([MS-XLDM] first).
   2. Permissively licensed references (MIT, Apache-2.0, BSD, CC0) under the clean-room rules.
   3. Black-box observation of test files we may legally use (synthetic files and permissively licensed samples) and of outputs from tools we may legally run (for example comparing decoded tables against `pbixray` output or the source CSVs).
4. Record every reference consulted, with URL, commit or version, and license, in `crates/pbi-vertipaq/REFERENCES.md`. Summarize it in [THIRD_PARTY_NOTICES.md](../../THIRD_PARTY_NOTICES.md).
5. Development may proceed under the clean-room rules. **Shipping a release** that includes the M3 decoders requires a legal answer on the UNVERIFIED items below, or written risk acceptance by the maintainers.

### Clean-room rules

- **No verbatim copying** of third-party code, comments, or non-trivial tables. Format facts are not copied expression (magic numbers, signatures, field order, enum values, and lookup tables that the format requires). Record where each one came from.
- **Permissive sources** (MIT, Apache-2.0, BSD, CC0) may be read, and their structure may be ported to idiomatic Rust, with attribution. If a port is close enough to count as a "substantial portion", keep the original copyright and license notice in [THIRD_PARTY_NOTICES.md](../../THIRD_PARTY_NOTICES.md). (We do this by default for every permissive source we port from.)
- **Copyleft sources** (GPL, LGPL, AGPL, MPL, EPL, CDDL…) and sources with **unknown or missing licenses** are not read by anyone working on `pbi-vertipaq`. If one is found by accident, stop and note it in `REFERENCES.md`.
- **No disassembly or decompilation** of Microsoft binaries (Power BI Desktop, `msmdsrv`, `Microsoft.AnalysisServices.*` DLLs) and no debugging of them.
- **Microsoft Open Specifications** may be copied to the extent needed to implement them, per the Intellectual Property Rights Notice that ships with each document. Patent rights come only from the Open Specifications Promise or the Community Promise where a document is listed (see the table).
- Each module in `pbi-vertipaq` starts with a doc comment listing the references that informed it, using the IDs from `REFERENCES.md`.
- Only synthetic data and permissively licensed samples are committed (now: the MIT-licensed Microsoft sample PBIX files in `reference/samples/microsoft/`). Public test vectors may be produced from synthetic data with the MIT-licensed `xpress9` Python package.

### License findings

Checked on 2026-10-21 against the live repositories, PyPI metadata and Microsoft Learn pages.

| Component | URL | License (as found) | Allowed use under our rules | Notes | Verified |
|---|---|---|---|---|---|
| pbixray (Python library) | https://github.com/Hugoberry/pbixray | MIT. `LICENSE`: "Copyright (c) 2024 Igor Cotruta" | Read; structurally port with attribution | Main reference for ABF and VertiPaq decoding. Read-only extractor. Does not parse the report layer, PBIT or PBIP. | Yes (LICENSE file, GitHub API `MIT`) |
| pbixray on PyPI | https://pypi.org/project/pbixray/ | MIT (PyPI `license: MIT`, v0.15.5) | Run locally as a dev-only comparison oracle; never bundled | Runtime deps: `xpress8`, `xpress9`, `xmhuffman>=0.3.0`, `kaitaistruct`, `numpy`, `pandas`, `apsw`. Same version as the venv in CONTEXT.md §2. | Yes |
| pbixray Kaitai specs (`docs/abf_xpress9.ksy`, `idf.ksy`, `idfmeta.ksy`, `dictionary.ksy`) | https://github.com/Hugoberry/pbixray/tree/main/docs | Each `.ksy` declares `meta.license: CC0-1.0`; the repository is MIT | Read; port structure with attribution | Declarative layouts for the XPress9 ABF wrapper, IDF segments, IDF metadata and dictionaries. Also `docs/TMSCHEMA_MAPPING.md` and `metadata.sqlitedb.csv`. We treat them as MIT (the stricter of the two) and attribute. | Yes |
| xpress9 (PyPI) / xpress9-python | https://pypi.org/project/xpress9/ , https://github.com/Hugoberry/xpress9-python | MIT. Repo `LICENSE`: "Copyright (c) 2025 Igor Cotruta". PyPI v0.3.9 classifier MIT | Read; port structure with attribution; use to generate synthetic test vectors | Cython wrapper. The C sources under `src/` and `include/` carry "Copyright (c) Microsoft Corporation. Licensed under the MIT License." headers. | Yes |
| Microsoft XPress9 C implementation (upstream) | https://github.com/microsoft/Extensible-Storage-Engine (`dev/ese/src/_xpress9/`) | MIT. "Copyright (c) Microsoft Corporation." Repository license `MIT`; per-file MIT headers | Read; structurally port the **decoder** with attribution | This is the original published source of the files vendored by xpress9-python. Only the decoder parts (`Xpress9DecHuffman.c`, `Xpress9DecLz77.c`, `Xpress9Lz77Dec.i`, `Xpress9Misc.c`, headers) are relevant. Patent position: see the UNVERIFIED row below. | Yes (copyright); patents UNVERIFIED |
| xmhuffman (PyPI) / xmhuffman-cython | https://github.com/Hugoberry/xmhuffman-cython | MIT. PyPI license text "Copyright (c) 2026 Igor Cotruta" | Read; port structure with attribution | Canonical-Huffman decoding of compressed VertiPaq string dictionaries. | Yes |
| xpress8 (PyPI) / xpress8-python | https://github.com/Hugoberry/xpress8-python | MIT. `LICENSE` "Copyright (c) 2026 Igor Cotruta" (GitHub API reports `NOASSERTION`, but the file is MIT) | Not needed now | XPress8 (ESE) variant. Not used by the PBIX `DataModel` fixture. | Yes (LICENSE file) |
| duckdb-pbix-extension | https://github.com/Hugoberry/duckdb-pbix-extension | MIT (GitHub API) | Read; port with attribution | C++ prior art by the same author. Not consulted to date. | License only |
| ripbi | https://github.com/bgarcevic/ripbi | Apache-2.0 (GitHub API) | Read; port with attribution and NOTICE handling | Rust prior art. Its `ripbi-xpress9` crate appears to build the C decoder via `build.rs`. Not consulted to date. | License only |
| [MS-XLDM] Spreadsheet Data Model File Format | https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-xldm/ | Microsoft Open Specifications IP Rights Notice (copyright permission to implement). Listed under "Office File Formats" in the Open Specifications Promise | Read; copy as needed to implement; patents under OSP for conforming implementations | Documents the backup header, files section, CRC markers, virtual directory (§2.1), column data storage, dictionaries and hash indexes (§2.3), and XMObject metadata (§2.5). Revision 8.3 (2025-08-19). Primary source for ABF and VertiPaq layout. | Yes (copyright and OSP listing); scope UNVERIFIED |
| [MS-SSAS] SQL Server Analysis Services Protocol | https://learn.microsoft.com/en-us/openspecs/sql_server_protocols/ms-ssas/ | IP Rights Notice. Not found on the OSP or Community Promise lists | Read; copy as needed to implement | XMLA and rowset semantics. Background only; we do not implement the protocol. | Copyright yes; patents UNVERIFIED |
| [MS-SSAS-T] SSAS Tabular Protocol | https://learn.microsoft.com/en-us/openspecs/sql_server_protocols/ms-ssas-t/ | IP Rights Notice. Not found on the OSP or Community Promise lists | Read; copy as needed to implement | Tabular metadata objects (compatibility level 1200+). Useful to map `metadata.sqlitedb` tables to model objects. Revision 19.0 (2026-10-12). | Copyright yes; patents UNVERIFIED |
| [MS-XCA] Xpress Compression Algorithm | https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-xca/ | IP Rights Notice. Not found on the OSP or Community Promise lists | Read; copy as needed to implement | Specifies LZ77+Huffman, Plain LZ77 and LZNT1. It does **not** specify XPress9, so it is background only (related Huffman and LZ77 concepts). Revision 10.0 (2024-04-23). | Copyright yes; patents UNVERIFIED |
| Microsoft Open Specifications Promise | https://learn.microsoft.com/en-us/openspecs/dev_center/ms-devcentlp/1c24c7c8-28b0-4ce1-a47d-95fe1ff504bc | Patent non-assert for listed "Covered Specifications" (revised 2023-02-24) | n/a | Lists [MS-XLDM]. Does not list [MS-SSAS], [MS-SSAS-T] or [MS-XCA]. Applies "to the identified version" and only to required portions "described in detail". | Yes |
| Microsoft Community Promise | https://learn.microsoft.com/en-us/openspecs/dev_center/ms-devcentlp/8b8d1b7a-a10a-4667-9558-6d9c43adf60d | Patent non-assert for listed specs | n/a | No SSAS, XLDM or XCA entries found. | Yes |
| Microsoft Fabric JSON schemas | https://github.com/microsoft/json-schemas (`fabric/item/report/definition/*`, `fabric/gitIntegration/platformProperties/*`) | MIT. "Copyright (c) Microsoft Corporation." | Vendor for PBIR validation (keep MIT notice) | Matches the `$schema` URLs in our fixtures (visualContainer 2.4.0–2.14.0, page 2.1.0, report 3.3.0, pagesMetadata 1.1.0, versionMetadata 1.0.0, platformProperties 2.0.0). | Yes |
| Selawik font | https://github.com/microsoft/Selawik | SIL Open Font License 1.1. "Copyright 2015, Microsoft Corporation… with Reserved Font Name Selawik." | Bundle unmodified in the app and renderer; ship the OFL text | README: "an open source replacement for Segoe UI"; known issue: missing kerning to match Segoe UI. Must not be sold by itself. A modified version may not use the name "Selawik". Used in M4 for text metrics. | Yes |
| Microsoft Learn docs for TMDL, TMSL, PBIP, PBIR, the Power Query M spec and the DAX reference | https://learn.microsoft.com/ | Microsoft Learn terms of use. The source repos were not publicly reachable for a license check | Read for format facts; do not copy text into the repo | Format facts are used for interoperability. Copying prose or examples needs review. | UNVERIFIED — needs legal review |
| Patents on XPress9 and VertiPaq encodings | n/a | Unknown | n/a | XPress9 is not in any Open Specification we found, so OSP coverage of XPress9 is not established. MIT copyright licenses carry no express patent grant. | UNVERIFIED — needs legal review |
| OSP coverage of [MS-XLDM] for PBIX `DataModel`, `.abf` and `cache.abf` | n/a | n/a | n/a | [MS-XLDM] describes the Excel data model stream. Whether the OSP non-assert covers decoding the same container from Power BI files, and which document version is covered, is a legal question. | UNVERIFIED — needs legal review |
| Power BI and Microsoft product terms (reverse-engineering clauses) | n/a | Product terms | n/a | We analyze only synthetic files and permissively licensed samples, and use no Microsoft binaries. Confirm that this is outside any EULA restriction. | UNVERIFIED — needs legal review |

No copyleft or unknown-license source has been identified as necessary. None has been read.

## Alternatives considered

| Option | Why not chosen |
|---|---|
| FFI to Microsoft's MIT XPress9 C code (`cc` crate) | Fastest path, and the license allows it. But it brings `unsafe`, a C toolchain on every platform, and C parsing of untrusted input (fuzzing and memory-safety burden). Kept as a possible **dev-only differential-testing oracle**, never shipped. |
| Shell out to Python `pbixray` | Needs a Python runtime and is not shippable. Acceptable only as a manual cross-check tool. |
| Verbatim port of pbixray | The MIT license allows it with notice, but the clean-room policy prefers a spec-first Rust design. Structural porting with attribution is allowed where needed. |
| Skip PBIX data (metadata only) | Fails acceptance criterion 4 (XPress9 known-good blob) and the PBIX explorer use case. Phase 1a still ships without decoding `DataModel`. |
| Run the decoder in a WASM sandbox | Adds a runtime and complexity. Reconsider only if a native decoder can't be made robust under fuzzing. |

## Consequences

- M3 is isolated in `pbi-vertipaq`. Phase 1 (PBIP and PBIT with M refresh) and the renderer do not depend on it.
- Decoding is memory-safe and fuzzed (`m3-fuzz`: XPress9 and ABF targets). Decoders return `Result` and cap sizes.
- Correctness is checked against a real PBIX `DataModel` (now: the MIT-licensed Microsoft samples) and cross-checked against independently produced tables (`pbixray` output, source CSVs) by row count and hash.
- Attribution is maintained in `crates/pbi-vertipaq/REFERENCES.md` and [THIRD_PARTY_NOTICES.md](../../THIRD_PARTY_NOTICES.md).
- Open legal questions (UNVERIFIED rows) may block releasing M3 decoding. They do not block M0–M2, M4, or M3 development.
