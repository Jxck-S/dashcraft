# Gaps scorecard

Where DashCraft stands against Power BI Desktop, area by area, so agents and maintainers can pick the
weakest areas first ([AGENTS.md](../AGENTS.md#8-picking-work-and-definition-of-done)). Numbers come
from the code and the tests, not from estimates; "unknown" means nobody has measured it.

As of **2026-10-23**, after the RLS "View as" commands and dialog. Re-scored in this pass: the
scatter, funnel, gauge, treemap and waterfall visuals, calculation groups, many-to-many
relationships and RLS, DAX 218 → 294 functions, AI in the app, conditional formatting and dynamic
format strings, scatter tooltips; the test fixtures (the synthetic Acme Outfitters example and the
MIT-licensed Microsoft samples) and render verification: no golden images exist, so visuals checked
by synthetic tests, Acme render snapshots and expected-value asserts score at most 3 (§5) and §7
scores the golden images themselves. The previous pass (2026-10-22, after the perf baseline and the
rename) scored the one-pass group-by, MCP, the AI layer and the app on the registry.

Update **2026-11-05**: Desktop ground truth ([ADR-0011](adr/0011-desktop-ground-truth.md)) re-scored
the DAX differential-testing and `FORMAT` rows; the other rows keep their scores, now backed by the
recordings where a suite covers them.

The first session-backed editing commands add a partial registry-editing capability.

Update **2026-11-06**: `doc.save` (in-place PBIP save) and `doc.saveAs` (a new PBIP project from
any source) re-scored the editing-commands row (§9) and the Save row (§10). The registry property
tests (M6.11) re-scored the parameter-fuzzing rows (§9, §12).

Update **2026-11-06** (later): report-level measures follow renames and are saved (M6.7, checked in Power BI
Desktop) re-scored the `reportExtensions.json` row (§1). Performance scenarios P09–P11 (edit and
re-render, save, end-to-end refresh) added three rows to §8.

Update **2026-11-07**: editing in the app (M6.8: Save, Save as, Undo and Redo, the editable
formula bar and Properties fields, the unsaved-changes prompt) re-scored the modeling, Save, undo
and accessibility rows (§10). Canvas selections now follow model renames (cross-filter row, §6;
score unchanged). Renames rewrite KPI and detail-rows DAX, saved in Desktop's form (editing and
TMDL rows, GT-6 s25; scores unchanged).

Update **2026-11-07** (later): the DAX formatter (`dax.format`, M7.2) added evidence to the
language-service row (§3; score unchanged); reformatted queries give the same
results on Power BI Desktop as the originals (2,902 of 2,902).

Update **2026-11-07** (later): DAX semantic tokens and references (`dax.tokens`,
`dax.references`, M7.1d) added evidence to the language-service row (§3; score unchanged), checked
on the 2,507 queries Power BI Desktop ran without error.

Update **2026-11-07** (later): DAX completion (`dax.complete`, M7.1c) added evidence to the
language-service row (§3; score unchanged) and P12 to §8 (4.7 → 4.8).

Update **2026-11-07** (later): DAX signature help and hover, and argument-count diagnostics checked
against Power BI Desktop (M7.1b), raised the language-service row 2 → 3 (§3; the §3 summary now
counts that row).

Update **2026-11-07** (later): M7 started. DAX diagnostics (`dax.diagnostics`, M7.1) added the DAX
language-service row (§3), and `NAMEOF` the function-library row (299 functions; score unchanged).

Update **2026-11-07** (later): the TMDL DashCraft saves after edits now equals Power BI Desktop's
for the same edits, byte for byte apart from new identifiers and two listed deviations (GT-4 s24,
M6 acceptance 14); new evidence in the TMDL, calculation-group and Save rows, no score change.
M6 is complete apart from the screen-reader runs (A6); data column delete and hierarchy level
edits moved to M14 and M10 (editing row, no score change).

Update **2026-11-07** (later): M ground truth (M7.3). Power BI Desktop's whole M library catalog and 177 M expressions recorded from its mashup engine; DashCraft's library matches the catalog, and number text and rounding now follow Desktop. Added the number-text and M differential-testing rows (§2: 3.1 → 3.3).

Update **2026-11-07** (M7.3 library growth, part 1): 69 more M functions checked against Desktop (461 library names, 335 of Desktop's 941 functions); 191 of 202 recorded M expressions match, 10 table functions remain ledgered (§2 library, differential-testing and cultures rows; no score change).

Update **2026-11-07** (M7.3b M diagnostics): names are bound when a query compiles, as in Desktop, `Expression.Evaluate`, Desktop's argument-count message, and `m.diagnostics`; 268 recorded M expressions (29 on name binding and argument counts) and the argument-count errors of all 941 library functions match Desktop. New §2 language-service row at 2 (§2: 3.3 → 3.2, the new row is below the average).

Update **2026-11-07** (M7.3 library growth, part 2): 49 table functions checked against Desktop (joins, grouping, pivots, expansions, splits, row lookups and editing; 514 library names, 384 of Desktop's 941 functions); all 239 recorded M expressions match apart from the one by-design decimal case, and the ledger's unsupported entries are gone (§2 library and differential-testing rows; no score change).

Update **2026-11-07** (M7.3c M hover, signature help and completion): `m.hover`, `m.signatureHelp` and `m.complete` with Desktop's scoping, the model's names and the library catalog; every free name and library call of the Acme and sample queries resolves (§2 language-service row 2 → 3; §2: 3.2 → 3.3).

## How to read it

- **Status**: ✅ done (behaves like Power BI for this capability, within the documented limits) ·
  🟡 partial · ❌ missing.
- **Score** (parity with Power BI Desktop for that row): **0** nothing · **1** detected or kept but
  not used · **2** outline or partial, many gaps · **3** usable for common cases, notable gaps ·
  **4** complete for the test fixtures, minor gaps · **5** complete and verified (round-trip,
  oracle, golden or exhaustive tests). `n/a` = out of scope by policy (never executed, never
  written, no live connections); `?` = unknown. Both are left out of the averages.
- **Evidence** names the file or test that proves the row. **Next step** is the smallest useful
  piece of work; "blocked" items wait for a human ([ROADMAP](ROADMAP.md#next)).

## Summary

| # | Area | Avg score | Rows ✅ / 🟡 / ❌ | Biggest gaps |
|---|---|---|---|---|
| 1 | [File formats](#1-file-formats) | 3.2 | 11 / 8 / 6 | Bookmarks, images, `DataMashup` only kept; report-level measures not evaluated; Fabric remote mode |
| 2 | [M language and library](#2-m-language-and-library) | 3.3 | 7 / 3 / 3 | Library breadth (384 of Desktop's 941 functions), the M formatter, cultures, other data sources |
| 3 | [DAX](#3-dax) | 3.8 | 5 / 6 / 0 | Financial, `INFO.*` and visual-calculation functions; exact auto-exist; relationship overrides in `RELATED`/`SUMMARIZE` |
| 4 | [Engine features](#4-engine-features) | 3.4 | 9 / 3 / 4 | Object-level security, self-referencing relationships, incremental refresh, drill-down |
| 5 | [Visuals](#5-visuals) | 2.1 | 15 / 6 / 8 | No visual checked against a Power BI render yet; image, maps, ribbon chart, conditional formats of headers/titles/cards and per-legend chart colors, Analytics pane, small multiples |
| 6 | [Interactions](#6-interactions) | 1.6 | 7 / 2 / 10 | Highlighting, bookmarks, drillthrough, drill-down, synced slicers |
| 7 | [Rendering fidelity](#7-rendering-fidelity) | 3.2 | 4 / 0 / 2 | No golden images (Power BI exports of the Acme Outfitters pages); DIN font |
| 8 | [Performance](#8-performance) | 4.8 | 12 / 0 / 1 | No app-open or webview canvas-latency scenarios; P09–P12 not in the baseline yet |
| 9 | [Command registry, MCP, AI](#9-command-registry-mcp-ai) | 3.4 | 8 / 3 / 1 | Broader editing commands |
| 10 | [App UX vs Power BI Desktop](#10-app-ux-vs-power-bi-desktop) | 2.0 | 6 / 11 / 6 | Report editing, modeling editors (new objects, relationships), Power Query editing |
| 11 | [Platform and packaging](#11-platform-and-packaging) | 2.7 | 7 / 2 / 4 | Signing and notarization, auto-update, ARM64 Windows/Linux |
| 12 | [Security and fuzzing](#12-security-and-fuzzing) | 3.3 | 8 / 2 / 2 | Legal review (ADR-0003), staged-file scanning in CI |

Overall: the **viewer** (formats, engines, rendering of the example and sample reports, performance) is at
3–5 and the automation surface (registry, MCP server, opt-in AI layer) at 4; the **Desktop**
surface (report and modeling editors, Power Query, signing) is at 0–2, which is where most of the remaining
work is.

## 1. File formats

Source: [FORMATS.md](../FORMATS.md), `crates/pbi-core/src/reader/`.

| Capability | Status | Score | Evidence | Next step |
|---|---|---|---|---|
| Content-based detection, safe containers (caps, zip-slip, no symlinks) | ✅ | 5 | `crates/pbi-core/tests/{detect,containers}.rs`; fuzz `detect`, `zip_container` | — |
| PBIP project (several models and reports) | 🟡 | 4 | `pbi-core::reader::project`; `crates/dashcraft-cli/tests/snapshots/cli__inspect_pbip.snap` | The app shows only the first model (`extraModels`) |
| TMDL read + write | ✅ | 5 | byte-identical round-trip of every fixture file (`crates/pbi-tmdl/tests/fixtures.rs`, `desktop_sample.rs`); after the same edits (four measures, a calculated column and table, a hierarchy, a relationship) the saved TMDL equals Power BI Desktop's byte for byte apart from new identifiers and the two entries of `reference/groundtruth/gt4/deviations.json`: Desktop's property order, CRLF line ends and blank last line (GT-4 s24, `crates/pbi-ops/tests/save.rs::edits_save_like_power_bi_desktop`); KPIs, detail rows and dynamic format strings in Desktop's form, also from TMSL (`kpi_and_detail_rows_are_written_in_desktop_form`, GT-6 s25) | Desktop's form of the constructs no edit creates yet (roles, partitions, calculation items, perspectives, translations; M26.1) |
| TMSL `model.bim` / PBIT `DataModelSchema` read + write | ✅ | 5 | `crates/pbi-tmsl/tests/tmsl.rs` (layout-preserving, UTF-16LE) | — |
| PBIR read + write | ✅ | 5 | every fixture file re-written byte-for-byte (`crates/pbi-pbir/tests/roundtrip.rs`) | — |
| PBIR-Legacy `report.json` / PBIX `Report/Layout` read + write | 🟡 | 3 | `pbi-pbir::legacy`, synthetic `crates/pbi-pbir/tests/legacy.rs`; legacy `Layout` reports in `reference/samples/microsoft/` | Themes of legacy reports in the CLI and `render.*` |
| PBIX read: report, model, embedded VertiPaq data | ✅ | 4 | `crates/pbi-vertipaq/tests/` on the Microsoft sample PBIX files (decoded tables cross-checked with pbixray by hash) | Legal review before release (ADR-0003) |
| PBIX write | ❌ | n/a | needs XPress9/ABF encoders (ADR-0003) | Out of scope: save as PBIP/PBIT |
| PBIT read + write (`export_pbit`) | ✅ | 4 | `crates/pbi-core/tests/readers.rs`; a Desktop-saved PBIT in `reference/samples/microsoft/` | Compare `export_pbit` output with a Desktop export (blocked: Windows) |
| PBIP `.pbi/cache.abf` data | ✅ | 4 | `pbi-session::cache`; `crates/pbi-session/tests/{acceptance,robustness}.rs` | A real cache fixture (blocked) |
| `.abf` backups | ✅ | 4 | `pbi-core::reader::single` + `pbi-vertipaq` | — |
| Power Pivot `.xlsx`/`.xlsm` (model + data) | 🟡 | 3 | `pbi-core::reader::workbook`; synthetic `pbi_vertipaq::test_support::power_pivot_sample` only | A real workbook fixture |
| `.vpax` (model + statistics) | ✅ | 4 | `pbi-core::reader::vpax`; synthetic | A DAX Studio fixture |
| `.rdl` paginated reports | 🟡 | 2 | outline only (`pbi-core::reader::rdl`) | Not planned: no RDL renderer |
| `.pq`/`.m`, `.pqt`, dataflow `model.json`, Dataflow Gen2 | 🟡 | 3 | `pbi-core::reader::mashup`, `crates/pbi-core/tests/formats.rs` | Refresh works only for offline-capable sources |
| `.pbids` | 🟡 | 2 | metadata only (`pbi-core::reader::single`) | — |
| `.pbiviz`, `.mez`, `.pqx` metadata and listing | ✅ | 4 | `pbi-core::reader::extension` | — |
| Executing custom visuals or connectors | ❌ | n/a | never executed (AGENTS.md rule 6) | Out of scope |
| Themes (base + custom, `ThemeDataColor`, `visualStyles`) | ✅ | 4 | `pbi-render::theme`, `crates/pbi-render/tests/theme.rs` | Legacy reports use the default theme in the CLI |
| `diagramLayout.json` | 🟡 | 3 | Model view positions (app) | Table colours; saving layouts |
| PBIR bookmarks | ❌ | 1 | kept verbatim on save (`pbi-pbir`) | Read and apply (ROADMAP Next) |
| `reportExtensions.json` (report-level measures) | 🟡 | 2 | typed in `pbi-pbir` and kept in the report AST (`Report::measures`); the linter counts them as defined, renames rewrite their table, DAX and `references`, saves write them back (`crates/pbi-pbir/tests/roundtrip.rs`, `crates/pbi-core/tests/{lint,save_in_place}.rs`, `crates/pbi-ops/tests/save.rs`; Power BI Desktop loads the renamed report and its card values are unchanged, GT-6) | Evaluate them in visuals and queries (M9.9); PBIR-Legacy/PBIX `modelExtensions` |
| `RegisteredResources` images | ❌ | 1 | kept verbatim | Draw `image` visuals and wallpapers |
| PBIX `DataMashup` | ❌ | 1 | listed with an Info diagnostic | Decode the package (partitions already carry the M) |
| Fabric remote mode (`byConnection`) | ❌ | 0 | shown as remote | Optional (ROADMAP Next); keychain credentials only |

## 2. M language and library

Source: `crates/pbi-m`; deviations in the `pbi_m::eval` module docs.

| Capability | Status | Score | Evidence | Next step |
|---|---|---|---|---|
| Lexer, parser, printer (full grammar, spans, recovery) | ✅ | 5 | `crates/pbi-m/tests/{parser,roundtrip}.rs`; fuzz `m_parse` | — |
| Lazy evaluator: closures, `each`, `try`/`otherwise`, `meta`, sections, types; names bound when a query compiles, as in Desktop (an unknown name anywhere fails the query, a step does not see itself, recursion needs `@`), `Expression.Evaluate` | ✅ | 4 | `crates/pbi-m/tests/eval.rs`, `names/*` cases of suite `m-functions`; fuzz `m_eval` | `table{[key]}?` with several matches; UTF-16 text positions |
| Standard library: **514 names** from `pbi_m::library_names()` = 384 functions + 50 types + 80 constants (kinds as in Desktop's library; plus 7 `#` intrinsics such as `#date`, `#table`), each one in Power BI Desktop's library with the same kind, parameter names, required-parameter count, type or constant value. Desktop 2.159's library has 1,142 dotted names (941 functions, 69 types, 132 constants and values; `pbi_m::service::catalog`), so DashCraft has 384 of its 941 functions (41%). Names by prefix: `Table.*` 95, `List.*` 69, `Text.*` 39, `Number.*` 33, `Date.*` 32, `Record.*` 18, `DateTimeZone.*` 15, `Binary.*` 12, `Duration.*` 12, `DateTime.*` 10, `Splitter.*` 10, `Time.*` 10, `Value.*` 10, others 149. Of Desktop's 114 `Table.*` functions, the 20 missing are fuzzy matching, keys, partition keys, `Table.View` and its helpers, `Table.Profile` and connector internals | 🟡 | 3 | `crates/pbi-m/tests/library.rs` (suite `m-library`), `crates/pbi-m/tests/stdlib.rs`; of the 239 expressions of suite `m-functions`, 238 give Desktop's result and 1 differs by design | By fixture demand: fuzzy matching, `Table.Profile`, the remaining `Text`/`List`/`Number` functions |
| Number text and rounding (`Text.From`, `Number.ToText` standard and custom formats, `Number.Round*`, error messages) | ✅ | 4 | .NET's round-trip and `G` layouts, `E`/`F`/`N`/`P`/`D`/`X` and custom formats from 15 significant digits, rounding in decimal with `RoundingMode` breaking ties and `Number.RoundUp`/`RoundDown`/`RoundAwayFromZero`/`RoundTowardZero` directed; the `format/*` and `Number.*` cases of suite `m-functions` match Desktop (`crates/pbi-m/src/value/digits.rs`) | Cultures |
| Differential testing against Power BI Desktop's mashup engine | ✅ | 4 | 268 expressions (literals, operators, errors, name binding, argument counts, every family of the implemented library), the whole library catalog and the argument-count errors of all 941 functions (`m-arity`) recorded from Desktop 2.159 and compared value by value, errors by reason and message; table results with each cell read inside its own `try`, so row and cell errors are compared too; 1 ledger entry (by design) (`crates/pbi-m/tests/{groundtruth,library}.rs`, [reference/groundtruth](../reference/groundtruth/README.md), ADR-0011) | Grow the corpus with the library; the partition queries of the samples |
| Language service (M7.3) | 🟡 | 3 | `m.diagnostics`: syntax, unknown names with Desktop's message and a "did you mean" from the model's queries, parameters and the library, wrong argument counts of library functions (Desktop's message; all 941 functions checked, `m-arity`), warnings for names DashCraft cannot evaluate and custom-connector names; sections; no false errors on the 44 queries of Acme and the committed samples (`pbi_m::service::diagnostics`, `crates/pbi-ops/tests/m_language.rs`). `m.hover`, `m.signatureHelp` and `m.complete` on half-typed text, scoped as Desktop binds names (steps, parameters, `each`, fields, sections, `@`), with the model's queries, parameters, functions and tables and Desktop's library catalog; on every query of Acme and the samples all 418 free names resolve and complete first and all 285 library calls get their signature (`pbi_m::service::{hover, signature_help, complete}`, `crates/pbi-session/tests/m_service.rs`) | `m.format` (M7.3d); semantic tokens and references |
| Example partitions load with the expected row counts (Acme Outfitters `expected.json`) | ✅ | 5 | `crates/pbi-m/tests/fixture_eval.rs`, `crates/pbi-session/tests/acceptance.rs` | — |
| `AzureStorage.Blobs` through the offline blob map | ✅ | 4 | `src/connectors/azure.rs`, `crates/pbi-m/tests/connectors.rs` | Real Azure access behind `BlobResolver` (optional, keychain) |
| Other data sources (`Sql.*`, `Web.*`, `File.*`, `SharePoint.*`, …) | ❌ | 0 | `DATA_ACCESS_PREFIXES` → "unsupported" + "Load CSV manually…" | Offline by policy; local `File.*` behind the path policy would need an ADR |
| Cultures (parsing, `Text.From`) | ❌ | 1 | ignored, invariant and en-US forms only, except `Date.MonthName` and `Date.DayOfWeekName`, which give Desktop's names for 18 languages (`src/stdlib/date_names.rs`) | Culture-aware conversions |
| `Decimal`/`Currency` precision | 🟡 | 2 | numbers are `f64`; `Number.Round*` rounds in decimal like Desktop, but Desktop keeps a number literal as a decimal until arithmetic (`9007199254740993` stays exact; ledgered by design, `m-functions` `literals/decimal-precision`) | A decimal number type |
| Query folding, streaming | ❌ | n/a | tables materialized, bounded by `Limits::max_rows` | Out of scope (no live sources) |
| Performance (P05: 100k-row CSV pipeline) | ✅ | 4 | 149 ms vs 250 ms target (`perf/baseline.json`) | — |

## 3. DAX

Source: `crates/pbi-dax`, [FUNCTIONS.md](../crates/pbi-dax/FUNCTIONS.md) (kept in sync by
`functions_md_lists_every_supported_function` and `every_supported_function_is_tested`).

**Functions: 299 registered** (`supported_functions()`; 275 supported, 24 partial; 218 → 299):

| Family | Registered | Partial |
|---|---|---|
| Logical and information | 32 | 0 |
| Math and statistics | 71 | 0 |
| Statistical distributions | 19 | 0 |
| Financial | 4 | 0 |
| Text | 22 | 1 |
| Date and time | 24 | 2 |
| Time intelligence | 37 | 5 |
| Aggregation and iterators | 20 | 0 |
| Filter and context | 27 | 4 |
| Table manipulation | 21 | 6 |
| Window functions | 8 | 5 |
| Parent-child | 5 | 0 |
| Calculation groups and security | 9 | 1 |

| Capability | Status | Score | Evidence | Next step |
|---|---|---|---|---|
| Lexer, parser, printer, `DEFINE`/`EVALUATE` | ✅ | 5 | `crates/pbi-dax/tests/{syntax,roundtrip}.rs`; fuzz `dax_parse` | — |
| Language service (M7.1) | 🟡 | 3 | `dax.diagnostics`: syntax, unknown tables, columns and measures with "did you mean", argument counts (checked against Power BI Desktop: 929 probes, `dax-arity`), unsupported functions; `dax.signatureHelp` and `dax.hover` from a catalog of all 299 functions with DashCraft-written descriptions; `dax.complete` by context (functions, model names, variables in scope, query `DEFINE`s, keywords; ranked by match and by the active parameter; P12 3.0 ms p95 over 2,000 measures); `dax.tokens` (14 kinds, declarations, unknown names) and `dax.references` (definition and every occurrence, model `objectId`); no error diagnostics or unknown names on the 2,507 queries Power BI Desktop ran without error (ground-truth corpora); UTF-8/UTF-16/line-column positions (`pbi_dax::service`, `crates/pbi-dax/tests/language_service.rs`); no false errors or warnings on the 136 expressions of Acme and the committed samples (`crates/pbi-ops/tests/dax_language.rs`); `dax.format` (line width, indentation, keyword and function casing, comments kept; idempotent and tree-preserving on every test file, generated text, the 136 expressions and the 2,507 Desktop queries; 2,902 reformatted queries give Desktop's original results; `crates/pbi-dax/tests/format.rs`); fuzz `dax_service` | M formatter and services (M7.3), the formula bar (M7.5), `dashcraft lsp` (M7.6) |
| Function library breadth | 🟡 | 4 | 299 functions (table above); added `NAMEOF` (matched against Power BI Desktop), logarithms, trigonometry, distributions (~1e-13 relative error against SciPy references), financial functions (`PMT`, `PV`, `FV`, `NPER`; DAX payment-type rounding and period bounds, stable tiny rates, long-horizon discounting and small growth ratios), `GEOMEAN`, `MAXA`/`MINA`/`AVERAGEA`, `YEARFRAC`, `NETWORKDAYS` and more, with a 33,200-call random-argument sweep (`crates/pbi-dax/tests/functions_library.rs`) | Missing: remaining financial functions (`XIRR`, `XNPV`, …), `LINEST`, visual-calculation functions, `INFO.*` (FUNCTIONS.md "Not supported yet") |
| Filter and row context, context transition, expanded tables, BLANK | ✅ | 4 | `crates/pbi-dax/tests/eval.rs`; the example's acceptance values | Context transition on duplicate rows of keyless tables |
| `CALCULATE` modifiers, `ALLSELECTED`, `USERELATIONSHIP`/`CROSSFILTER` | 🟡 | 4 | `ALLSELECTED` shadow contexts per iteration and `SUMMARIZECOLUMNS` row, `KEEPFILTERS` iterators, `IGNORE` and `CROSSFILTER` validity equal to Desktop (ground-truth suite `dax-functions`); FUNCTIONS.md "Known semantic gaps" | Overrides for `RELATED` and `SUMMARIZE` |
| `SUMMARIZECOLUMNS` auto-exist, rollups | 🟡 | 3 | approximated (cross product + empty-row removal) | Exact auto-exist |
| `FORMAT` and format strings | 🟡 | 4 | `format_value`; dynamic format strings per cell (`EvalContext::evaluate_measure_format_strings`, used by visuals); 747 `FORMAT` cases equal to Desktop's (ground-truth suite `format`); en-US only | Other cultures |
| User-defined functions, `DEFINE COLUMN` | 🟡 | 2 | `EXPR` parameters and `DEFINE COLUMN` missing | — |
| Calculation groups (`SELECTEDMEASURE` & co.) | ✅ | 4 | typed `CalculationGroup`/`CalculationItem`, lossless TMDL and TMSL (`crates/pbi-tmdl/tests/calc_groups.rs`); precedence, selection expressions, sideways recursion, dynamic format strings (`crates/pbi-dax/tests/model_features.rs`, `crates/pbi-session/tests/security.rs`); per-cell format strings in visuals (`crates/pbi-render/tests/conditional_format.rs`); items without an `ordinal` (as Desktop saves them) numbered by position, as Desktop answers (GT-4 s24) | Hide subtotals of `nonvisual` items; differential tests (blocked: Windows) |
| Differential testing against Analysis Services | ✅ | 4 | 2,259 queries recorded from Power BI Desktop 2.159's engine (Acme measures per page and filter, tables, metadata, calculation groups and RLS, `FORMAT`, every `functions.rs` case, three MIT samples) and compared value by value, plus 929 argument-count probes compared with the function catalog; 1 ledger entry (float summation order, by design) (`crates/pbi-session/tests/groundtruth.rs`, [reference/groundtruth](../reference/groundtruth/README.md), ADR-0011) | The queries Desktop's visuals send (Performance analyzer, manual) |
| Performance (P04: 5 measures over 1M rows) | ✅ | 5 | 26 ms vs 100 ms target (`perf/baseline.json`): `SUMX`/`AVERAGEX`/`MINX`/`MAXX`/`COUNTX`/`COUNTAX` compile simple column arithmetic to columnar loops (was 114 ms) | — |

## 4. Engine features

Source: `crates/pbi-engine`, `crates/pbi-session`, `crates/pbi-model`.

| Capability | Status | Score | Evidence | Next step |
|---|---|---|---|---|
| Arrow columnar store, case-insensitive dictionaries, DataFusion SQL | ✅ | 5 | `crates/pbi-engine/tests/engine.rs`, `src/sql.rs` | — |
| Relationships one-to-many and one-to-one, blank row | ✅ | 5 | `src/relationships.rs`; engine tests | — |
| Filter propagation (single and both directions, "no path, no effect") | ✅ | 5 | engine tests; the example's acceptance values (a card over an unrelated table keeps its value under every report filter) | — |
| Inactive relationships, `USERELATIONSHIP`/`CROSSFILTER` overrides | 🟡 | 3 | per-`FilterContext` overrides | `RELATED` navigation and `SUMMARIZE`/`GROUPBY` over related columns |
| Many-to-many relationships | ✅ | 4 | limited relationships (key values cross in the cross-filter direction, no blank row, `RELATED` refused, `RELATEDTABLE` works); `crates/pbi-engine/tests/m2m_security.rs`; `ENG003` is informational | Differential tests (blocked: Windows) |
| Self-referencing relationships | ❌ | 0 | ignored with `ENG007` (parent-child functions work) | — |
| Calculation groups | ✅ | 4 | see §3; calculation-group tables load their items (`crates/pbi-session/tests/security.rs`) | — |
| Row-level security ("View as" roles and user) | ✅ | 4 | `Session::set_view_as` / `set_effective_roles` restrict every query (DAX row filters, security-direction propagation, union over roles, on a restricted engine copy so `ALL` cannot escape; fails closed); `USERPRINCIPALNAME`/`USERNAME`/`USEROBJECTID`/`CUSTOMDATA`; `crates/pbi-session/tests/security.rs`, `crates/pbi-engine/tests/m2m_security.rs`; `security.roles` / `security.viewAs` commands (`crates/pbi-ops/tests/security.rs`) and the app's Modeling → View as | Role filters that read another secured table see all of its rows (evaluated against the unrestricted data) |
| Object-level security | ❌ | 0 | kept in `extra` | — |
| Translations, perspectives | 🟡 | 1 | read into the model, not applied | — |
| Refresh pipeline (M → calculated columns → calculated tables; transactional, cancellable) | ✅ | 5 | `crates/pbi-session/tests/acceptance.rs` | — |
| Async, cancellable engine APIs and snapshots | ✅ | 5 | `CancellationToken` throughout; app cancels superseded renders | — |
| Incremental refresh policies (`policyRange` partitions) | ❌ | ? | read as an unknown partition kind; behaviour unknown | Check with a fixture |
| User hierarchies | 🟡 | 2 | read into the model; matrix expand works on row groups | Drill-down/up |
| Group-by aggregates in one storage-engine pass | ✅ | 4 | `SUMMARIZECOLUMNS` with plain aggregations groups the visible fact rows once (`pbi-engine` `group_index`/`aggregate_groups`); differential tests against the general path (`crates/pbi-dax/src/eval/fast/tests.rs`); P02 13.6 s → 97 ms | Fast paths for more aggregation shapes (the general path still evaluates per group) |
| DirectQuery, Direct Lake, composite models | ❌ | n/a | imported data only | Out of scope |

## 5. Visuals

Source: [docs/VISUALS.md](VISUALS.md#render-verification), `crates/pbi-render/src/visuals/`. Score 4 =
checked against a Power BI render (golden SSIM ≥ 0.85); 3 = Full, checked by synthetic tests, the
Acme Outfitters render snapshots and expected-value asserts, but not against a Power BI render (no
golden images exist yet, §7).

| Capability | Status | Score | Evidence | Next step |
|---|---|---|---|---|
| `textbox` | ✅ | 3 | synthetic tests (`visuals_basics.rs`: runs, lists, wrapping, line pitch); Acme Outfitters render snapshots (`render_snapshots.rs`) | Golden of an Acme page |
| `shape`, `basicShape` (22 kinds) | ✅ | 3 | synthetic tests (`visuals_basics.rs`); Acme render snapshots | Golden of an Acme page |
| `actionButton` | ✅ | 3 | synthetic tests (`visuals_basics.rs`: icons, text, actions); the Acme buttons' actions (`buttons_carry_their_actions`); Acme render snapshots | Hover/press states; golden |
| `pageNavigator`, `bookmarkNavigator` | 🟡 | 2 | drawn as one labelled button | Page/bookmark list |
| `cardVisual` (new card) | ✅ | 3 | synthetic layout tests (`visuals_basics.rs`); every Acme card value drawn on the rendered pages (`visuals_basics_reference.rs`, 39 values of `expected.json`); render snapshots | Golden of an Acme page |
| `card`, `multiRowCard` | ✅ | 3 | `crates/pbi-render/tests/visuals_basics*.rs` | Golden from the service |
| `kpi` | 🟡 | 2 | synthetic tests; `TrendLine` is the trend axis (`kpi_trend_line_is_the_axis`); the Acme KPI's indicator and goal at the last trend point against `expected.json` (`query_reference.rs`, `visuals_charts_reference.rs`); basic layout | Compare the layout with a Power BI render |
| `slicer` list, dropdown, tile | ✅ | 3 | synthetic tests (`visuals_grids.rs`); the Acme slicers' saved selections (`visuals_grids_reference.rs`); render snapshots; DIN headers in Selawik | Golden of an Acme page |
| `slicer` range and relative date, `advancedSlicerVisual` | 🟡 | 2 | drawn; range as tiles/sliders not interactive | See §6 |
| `tableEx` | ✅ | 3 | synthetic tests (`visuals_grids.rs`); the Acme tables' rows, order and totals against `expected.json` (`visuals_grids_reference.rs`, `query_reference.rs`); render snapshots | Conditional formats of headers and totals; golden |
| `pivotTable` (matrix) | ✅ | 3 | `crates/pbi-render/tests/visuals_grids*.rs`; row and column levels in sort-by-column order (`matrix_levels_follow_the_sort_by_column`); the Acme matrices against `expected.json` | Golden |
| `pieChart`, `donutChart` | ✅ | 3 | synthetic tests (`visuals_charts.rs`); the Acme donut and pie against `expected.json` (`visuals_charts_reference.rs`, `query_reference.rs`); render snapshots | DIN font; golden |
| `clusteredBarChart` | ✅ | 3 | synthetic tests (`visuals_charts.rs`); the Acme bar chart's order and bars (`visuals_charts_reference.rs`); render snapshots | Golden of an Acme page |
| Other bar/column variants (stacked, 100 %) | ✅ | 3 | `crates/pbi-render/tests/visuals_charts*.rs` | Golden |
| `lineChart`, `areaChart`, `stackedAreaChart` | ✅ | 3 | synthetic | Golden |
| Line + column combos | ✅ | 3 | synthetic | Golden |
| `image` | ❌ | 0 | placeholder | Draw `RegisteredResources` images |
| Maps (`map`, `filledMap`, `shapeMap`, `azureMap`) | ❌ | 0 | placeholder | Offline tiles are a design question (no network) |
| `funnel`, `gauge`, `treemap`, `waterfallChart` | ✅ | 3 | synthetic (`crates/pbi-render/tests/visuals_more.rs`, `query_visuals.rs`): conversion-rate labels, min/max/target, squarified layout, sentiment colors and breakdown top N + Other; on the Acme pages: render snapshots, the gauge's values against `expected.json` | Golden of an Acme page (blocked: needs Power BI Desktop or the service) |
| `scatterChart` | 🟡 | 3 | synthetic (`visuals_more.rs`): Details × Legend points, numeric axes, bubbles, labels; first play-axis frame only; Acme render snapshots | Play-axis animation; Analytics pane lines; golden |
| `ribbonChart` | ❌ | 0 | placeholder | Draw it (stacked columns joined by ribbons) |
| AI visuals (decomposition tree, key influencers, Q&A, smart narrative) | ❌ | 0 | placeholder | After the AI layer (ADR-0008) |
| R/Python, Power Apps/Automate, custom `.pbiviz` visuals | ❌ | n/a | placeholder; never executed | Out of scope (a sandboxed host is a future ADR) |
| Conditional formatting of tables and matrices: background and font color (gradients with 2 or 3 colors, rules, field values), data bars, built-in icons | 🟡 | 3 | `pbi_render::conditional`; synthetic only (`crates/pbi-render/tests/conditional_format.rs`, `src/conditional/tests.rs`); rule inputs outside the visual are queried as hidden fields; unsupported variants render unformatted with a warning ([VISUALS.md](VISUALS.md#conditional-formatting)) | Other fill rules, `scopeId` selectors, web URLs and image icons; golden |
| Conditional chart colors (`dataPoint.fill`) | 🟡 | 2 | bar, column, pie, donut, treemap, scatter and funnel without a legend field (same tests) | Per-legend colors from a measure; line, area, combo, waterfall and gauge colors |
| Conditional formats of headers, titles, cards, gauges and KPIs | ❌ | 0 | not applied (warning) | ROADMAP Next |
| Dynamic format strings in visuals (calculation items, `formatStringDefinition`) | ✅ | 3 | each card, table, matrix and chart-label value formatted with its own format string (`calculation_items_format_cells_with_their_own_format_strings`, `chart_labels_use_each_cells_format_string` in `conditional_format.rs`) | Golden with a calculation group |
| Analytics pane lines, small multiples | ❌ | 0 | not read | — |
| Visual headers, hover and press states | ❌ | 0 | never drawn | — |

## 6. Interactions

Source: [VISUALS.md "Interaction summary"](VISUALS.md#interaction-summary), ADR-0006,
`crates/pbi-ops/src/canvas.rs` (the app's canvas runs through it).

| Capability | Status | Score | Evidence | Next step |
|---|---|---|---|---|
| Slicer selection (single, multi, Select all, search, clear, dropdown) | ✅ | 4 | `crates/pbi-ops/tests/canvas.rs`; app canvas tests | — |
| Cross-filter from pie, bar, column, line, scatter, funnel, treemap, waterfall, table, matrix per `visualInteractions` | ✅ | 4 | same; hit ids on every mark of the new chart types; selections follow model renames, undo and redo (`canvas_selections_follow_renames`) | — |
| Highlight interactions (`HighlightFilter`) | ❌ | 1 | filters instead, with a warning | Highlight values in DataViews (ROADMAP Next) |
| Default tooltips (data points, ⚠ errors with DAX) | ✅ | 3 | `canvas.tooltip`; scatter points show Details, Legend, X, Y and Size (`scatter_tooltips_show_details_legend_x_y_and_size` in `crates/pbi-ops/tests/canvas.rs`) | Tooltip formatting objects (`tooltips.*`) |
| Report-page tooltips | ❌ | 0 | not rendered | ROADMAP Next |
| Back and page-navigation buttons, Clear all slicers | ✅ | 4 | `canvas.click` button actions | — |
| Web URL buttons | ✅ | n/a | shown with Copy link, never opened (policy) | — |
| Bookmarks (buttons, navigator, pane) | ❌ | 0 | "not available" | ROADMAP Next |
| Drillthrough | ❌ | 0 | "not available" | ROADMAP Next |
| Drill-down/up on hierarchies | ❌ | 0 | — | — |
| Matrix expand/collapse, table/matrix/slicer scrolling | ✅ | 4 | `canvas.toggleMatrixRow`, `canvas.scroll` | — |
| Synced slicers (`syncGroup`), `selfFilter` | ❌ | 0 | not read | ROADMAP Next |
| Range-slider drag, relative-date pickers | ❌ | 0 | drawn, not interactive | ROADMAP Next |
| Filters pane: view cards (hidden, locked) | 🟡 | 3 | app Filters pane | Editing filter cards |
| Relative-date and measure filters at report/page level | 🟡 | 2 | reported "not applied" (`pbi_render::query` filter matrix) | Apply them |
| Visual header menus (focus mode, show as table, sort, export), right-click menus | ❌ | 0 | — | — |
| Q&A | ❌ | 0 | "not available" | After the AI layer |
| Persisting selections (save with the report, bookmarks) | ❌ | 0 | view state only (ADR-0006) | Commands that turn view state into document edits |
| Headless interaction (`canvas.*` registry commands) | ✅ | 4 | 11 `canvas.*` commands (`docs/COMMANDS.md`), the same ones the app uses; legacy reports' themes resolved | — |

## 7. Rendering fidelity

Source: `crates/pbi-render/tests/golden.rs`, `render_snapshots.rs`,
[VISUALS.md](VISUALS.md#render-verification).

| Capability | Status | Score | Evidence | Next step |
|---|---|---|---|---|
| Golden comparison: per-region and whole-page SSIM ≥ 0.85 (default threshold, no per-page exceptions) against Power BI PNG exports at `reference/golden/<example>/<report>/<page>.png` | ✅ | 4 | `crates/pbi-render/tests/golden.rs`: alignment, SSIM and heatmaps exercised on a render compared with itself (also padded, at another scale); pages without a golden print SKIP | — |
| Golden images (Power BI exports of the Acme Outfitters pages, which use 29 visual types, the synthetic-only funnel, gauge, scatter, treemap and waterfall among them) | ❌ | 0 | none exist (`reference/golden/` is gitignored, nothing is committed): every page prints SKIP and no render is checked against Power BI | Export the Acme pages from Power BI Desktop or the service into `reference/golden/acme-outfitters/<report>/<page>.png` (blocked: needs Power BI Desktop or the service) |
| Card values on rendered output match the example's expected values | ✅ | 5 | all 39 card values of `reference/examples/acme-outfitters/expected.json` drawn on the rendered pages (`visuals_basics_reference.rs`) and in the DataViews (`query_reference.rs`); tables, matrices, KPI, gauge and pie too | — |
| Fonts: Selawik (Segoe UI), Carlito (Calibri) | ✅ | 4 | `crates/pbi-render/assets/fonts/`; metric-compatible substitutes; font mapping and metrics tests (`calibri_maps_to_carlito` in `src/text/fonts.rs`, `crates/pbi-render/tests/text.rs`) | — |
| DIN text | ❌ | 1 | rendered with Selawik, ~4 % narrower | No open metric-compatible DIN is known |
| Determinism and render regressions (same bytes on every OS; cache-equivalent renders; per-page snapshots) | ✅ | 5 | `png_is_deterministic_and_sized` (`crates/dashcraft-cli/tests/render.rs`); visual-cache equivalence tests on report pages; `insta` render snapshots of every Acme page (`crates/pbi-render/tests/render_snapshots.rs`: per-visual texts, data points, warnings and SVG hashes) | — |

## 8. Performance

Source: [docs/perf.md](perf.md), `perf/budgets.toml`, `perf/baseline.json` (Apple M4 Pro, all eight
scenarios recorded 2026-10-23 on an idle machine). Score: 5 ≤ half the target · 4 ≤ target · 3 ≤ max · 1 over max.

| Scenario | Status | Score | Evidence (metric, ms) | Next step |
|---|---|---|---|---|
| P01 `table-scroll-step` (50k rows, p95; target 16 / max 33) | ✅ | 5 | 0.71 (was 25.6 before the render cache) | — |
| P02 `table-page-cold` (target 1000 / max 3000) | ✅ | 5 | 104 (was 13 567 before the one-pass group-by) | Lower `max_ms` to keep the gain ([perf.md](perf.md#changing-scenarios-and-budgets)) |
| P03 `example-page-no-data` (16 / 50; the Acme Outfitters Executive Summary page, 14 visuals) | ✅ | 5 | 0.77 | — |
| P04 `dax-measures-1m` (100 / 500) | ✅ | 5 | 26 (was 114 before the columnar iterators) | — |
| P05 `m-csv-100k` (250 / 1000) | ✅ | 4 | 149 | — |
| P06 `pbix-data-open` (20 / 100; the Revenue Opportunities sample PBIX, 9 tables) | ✅ | 5 | 4.5 | — |
| P07 `busy-page-svg` (4 / 16) | ✅ | 5 | 1.6 | — |
| P08 `example-table-scroll-step` (p95; 16 / 33; the example's Customer Detail table, 29 rows) | ✅ | 5 | 0.34 | — |
| P09 `edit-measure-requery` (50 / 150; edit `Total Sales`, re-render the Executive Summary page, through `Registry::execute`) | ✅ | 4 | 43.6 on a Windows x86-64 desktop; 53 on the CI runner's quick run, over target (no baseline entry yet) | Record on the baseline machine |
| P10 `save-pbip-acme` (100 / 300; one edit, `doc.save` of a copy of the example) | ✅ | 4 | 55 on the same desktop | Record on the baseline machine |
| P11 `refresh-acme-end-to-end` (1000 / 3000; open, refresh 11 tables from the CSVs, first page, fresh process) | ✅ | 5 | 184 on the same desktop | Record on the baseline machine |
| P12 `dax-complete` (p95; 20 / 50; DAX completion over 20 tables × 20 columns and 2,000 measures) | ✅ | 5 | 3.0 on the same desktop | Record on the baseline machine |
| Scenarios for app open and canvas latency in the webview | ❌ | ? | none (P22 and P21 in [DESKTOP-PLAN.md](DESKTOP-PLAN.md)) | Add with the app milestones |

## 9. Command registry, MCP, AI

Source: `crates/pbi-ops`, [ADR-0007](adr/0007-command-registry.md), [COMMANDS.md](COMMANDS.md).

| Capability | Status | Score | Evidence | Next step |
|---|---|---|---|---|
| Registry: 73 typed commands (including `model.setMeasureExpression`, `model.measure.create`/`update`/`delete`, `model.relationship.create`/`update`/`delete`, `model.column.create`/`update`/`delete`, `model.table.create`/`update`/`delete`, `model.hierarchy.create`/`delete`, `model.dependencies`, `model.object.rename`, `model.object.setDescription`, `report.visual.move`/`resize`, grouped `doc.applyEdits`, history, `doc.save` and `doc.saveAs`) plus 9 opt-in `ai.*` commands (4 agent-safe), JSON Schemas, `Workspace`, `OpError`, cancellation, progress | ✅ | 4 | `crates/pbi-ops/tests/{registry,e2e}.rs`; `dashcraft commands --json` | — |
| CLI `commands`, `run`, `run --batch` | ✅ | 4 | `crates/dashcraft-cli/tests/ops.rs` | — |
| JSON-RPC 2.0 over stdio (`serve --stdio`, `$/cancel`) | ✅ | 4 | `crates/pbi-ops/tests/rpc.rs` | — |
| Path policy for agents (read/write roots, `..` and symlink escapes) | 🟡 | 3 | `crates/pbi-ops/tests/path_policy.rs`; check-then-use (TOCTOU) | `cap-std` handles (ADR-0007) |
| MCP server (`dashcraft mcp`) | ✅ | 4 | `crates/pbi-ops/tests/mcp.rs`, `crates/dashcraft-cli/tests/mcp.rs`; protocol revisions 2024-11-05 to 2026-07-28, roots, `--allow`/`--deny`, hints from `CommandMeta` ([MCP.md](MCP.md)) | Tested with scripted clients only; check with real MCP clients |
| App on the registry | ✅ | 4 | every domain operation through `ops_execute` → `Registry::execute`; `app/src/lib/ops.gen.ts` (core and `ai.*` commands) kept in sync by the `ops_typescript_is_up_to_date` test in `crates/pbi-ai/tests/registry.rs` (generator in `crates/pbi-ai/tests/ops_ts/`); app tests through the mock runtime | App-only views (`document_views`, `object_details`, …) as registry commands where agents need them |
| CLI subcommands as thin registry wrappers | ✅ | 4 | `detect`, `inspect`, `lint` run `doc.detect`, `doc.open` + `doc.inspect`, `doc.lint` (`crates/dashcraft-cli/src/ops.rs`): 374 runs (17 inputs × 22 flag sets: Acme in five shapes, the four Microsoft samples, RDL, dataflow and Power Query files, broken, unreadable, junk, empty and missing inputs) byte-identical to the old CLI; `run --save` (`crates/dashcraft-cli/tests/ops.rs::run_save_writes_only_the_edited_file`) | `refresh`, `measures`, `export`, `render`, `convert` as wrappers where the output can stay identical |
| Editing commands (registry → `Document` commands) | 🟡 | 3 | `model.setMeasureExpression`; `model.measure.create`/`update`/`delete` (deletes list dependents across DAX, model structure and report fields, `dryRun`; undo keeps ids, ids never reused); `model.dependencies` (the same list for any table, column, measure, hierarchy or relationship); `model.relationship.create`/`update`/`delete` (validated, GUID names, engine warnings, `USERELATIONSHIP`/`CROSSFILTER` users as dependents, undo in place; filters follow at once; saved updates match Power BI Desktop on all 643 recorded cases, GT-6); `model.column.delete` and `model.table.delete` (calculated columns, any table with its relationships and role filters; data columns and columns used by relationships, hierarchies or sort-by refused; the data is set aside so it cannot be queried and undo restores it without a refresh; saved as removed TMDL; saved deletes match Power BI Desktop on all 643 recorded cases, GT-6); `model.column.create` (calculated columns computed at once over the loaded data, data type inferred from the values unless given, Desktop's `lineageTag` and summarize-by defaults, saved after the table's columns; saved columns match Power BI Desktop's types and all 643 recorded cases, GT-6); `model.column.update` (expression, data type re-inferred unless given, format, folder, description, visibility; saved in place); `model.table.update` (a calculated table's expression: kept columns keep ids and properties, new result columns added like a created table's, dropped ones removed unless something uses them; any table's description and visibility; saved in place; Power BI Desktop keeps saved column and table updates as written, its values after a refresh equal the ones recomputed in the session, and all 643 recorded cases match, GT-6); self-references are circular; recalculation after every edit, undo and redo (calculated tables and columns reached by the change recomputed in dependency order over the loaded data without an M refresh; ones that can no longer be calculated lose their values and report the error; `recalculated` in edit results); `model.hierarchy.create`/`delete` (levels on the table's columns, Desktop's `lineageTag`s, saved in place; deletes list the report fields on its levels; Power BI Desktop opens saved hierarchies with the same levels and all 643 recorded cases match, GT-6); `model.table.create` (calculated tables: columns and types from the result over the loaded data, computed at once, Desktop's `lineageTag`s, inferred names and types and source columns, saved as a new table file; Power BI Desktop keeps the saved columns as written, with the same rows, and all 643 recorded cases match, GT-6); `model.object.rename` (tables, columns, measures; rewrites DAX in measures, format strings, detail rows and KPI expressions, a table's default detail rows, calculated columns and tables, calculation items and role filters (Power BI Desktop opens renamed KPIs and detail rows without errors and saves the same TMDL, GT-6 s25); M queries, parameters and functions, rewriting the M that refers to them, with `PBI_QueryOrder` kept in step and names M cannot follow refused; PBIR report-level measures follow (table, DAX, `references`) and are saved into `reportExtensions.json`; keeps the loaded data; `dryRun` preview; Power BI Desktop refreshes saved query renames to the same values on all 643 recorded cases and loads renamed report-level measures with unchanged card values, GT-6); `model.object.setDescription`; `report.visual.move`/`resize` (a visual group takes its visuals along: offsets kept on a move, scaled on a resize unless it scrolls; Power BI Desktop saves the same positions for the same edit, GT-6); grouped `doc.applyEdits`, `doc.undo`, `doc.redo`, `doc.history`; DAX query outputs and canvas renders include the captured `documentRevision` (a render reads its report, page and data from one revision; edits publish the "View as" engine with the model; threaded tests find no mixed-revision result, M6 acceptance 8); dirty close guard; `doc.save` writes the edits into the opened PBIP project (only changed files, each atomically; untouched bytes kept; renames move table files and follow into report JSON, also for fields renamed and then deleted; files changed on disk are a `conflict` unless `overwrite`); `doc.saveAs` writes a new PBIP project from any source and moves the document there (later saves go there; the four committed samples save with an equal structure); conversions include unsaved edits; new measures get a random `lineageTag` where the model uses lineage tags (injectable generator, ADR-0014); committed edits and dirty-clearing saves announce themselves as `doc/changed` (a `serve` notification before the response, an MCP `info` log message); `crates/pbi-ops/tests/e2e.rs::{model_edit_commands_publish_revisions_and_history, rename_describe_and_move_through_the_registry, measure_create_update_delete_through_the_registry, relationship_create_and_delete_through_the_registry, relationship_update_through_the_registry, column_and_table_delete_through_the_registry, calculated_column_create_through_the_registry, edits_recalculate_dependants_without_a_refresh, hierarchy_create_and_delete_through_the_registry, calculated_table_create_through_the_registry, calculated_table_update_through_the_registry, renaming_line_amount_keeps_the_expected_values, query_renames_rewrite_m_and_keep_the_expected_values}`, `crates/pbi-ops/tests/save.rs`, `crates/pbi-core/tests/save_in_place.rs`, `crates/pbi-ops/tests/{rpc,mcp}.rs` (doc/changed; an MCP agent limited to `--allow edit` saves only inside its write roots) | Data column delete (with M editing, M14.7 and M14.9), hierarchy level edits (M10.5), change events in the app |
| Fuzzing command parameters ("panic hunt") | ✅ | 4 | `crates/pbi-ops/tests/property.rs`: every command with its examples, schema-derived and mutated parameters (8,688 calls, no `internal` error or hang); seeded random edit, undo and redo sequences (10,000 steps undo to an equal document; on loaded data the edits save, reopen with the same shape and undo to the same measure values); nightly in release with a new seed (`fuzz.yml`, [ci.md](ci.md#registry-property-tests)); found a save refusal after renaming then deleting a field a visual uses, now fixed | Random interleavings of edits and queries (M6 acceptance 8) |
| AI layer (`pbi-ai`: providers, consent, keychain secrets) | ✅ | 4 | ADR-0008, [AI.md](AI.md): OpenAI-compatible, Azure OpenAI and Ollama providers, per-provider/capability/data-class consent, keychain-only keys, `ai.preview`; `crates/pbi-ai/tests/`; CLI wiring `bedd3c7` (`run`/`serve`/`mcp`, `dashcraft ai login`) | Tested against scripted servers and a demo provider (`DASHCRAFT_AI_MOCK=1`, debug builds), not real providers |
| AI features (NL → DAX, explain a measure, lint fixes) | 🟡 | 3 | `ai.explainMeasure`, `ai.generateMeasure` (parsed, optionally evaluated locally, never applied); in the app (Explain with AI, Suggest a measure) | Lint-fix suggestions; NL → DAX in the DAX query view |
| RPC over TCP with authentication | ❌ | 0 | stdio only (parent trusted) | Not planned |

## 10. App UX vs Power BI Desktop

Source: [app/README.md](../app/README.md), [desktop-ui-reference.md](../app/docs/desktop-ui-reference.md).

| Capability | Status | Score | Evidence | Next step |
|---|---|---|---|---|
| Shell: ribbon, view rail, panes, status bar, search, recent files | 🟡 | 3 | desktop-ui-reference.md (Save, Undo, Redo, File → Save and Save as enabled; the other editing commands shown disabled) | Enable commands as features land |
| Report view (reading mode, interactive) | ✅ | 3 | §5–§6; app canvas tests | — |
| Report editing (add, move, resize, format visuals) | 🟡 | 1 | `report.visual.move`/`resize` registry commands (undoable; visual groups move and resize with their visuals as in Power BI Desktop, GT-6), saved by `doc.save` (only the `position` numbers change); no UI | Canvas drag handles, add/format commands |
| Table view (virtualized grid, SQL filter, column statistics) | ✅ | 3 | `data.page`, `data.columnStats` registry commands through `ops_execute` | Statistics of the filtered rows |
| Model view (diagram) | 🟡 | 2 | read-only; `diagramLayout.json` or auto-layout | Edit relationships, save layouts |
| Modeling: new measure/column/table, rename, properties | 🟡 | 2 | registry commands create, update, delete and rename (DAX and M follow); in the app the formula bar edits `Name = expression` of measures, calculated columns and calculated tables (a rename and the new DAX as one undo step) and the Properties pane edits descriptions, display folders and format strings (M6.8, `edits_save_and_close_through_the_registry` in `app/src-tauri/src/lib.rs`, `app/tests/editing.test.ts`) | New measure, column, table and relationship editors (M10) |
| DAX query view | 🟡 | 3 | editor, run/cancel, results ≤ 1000 rows | IntelliSense, formatter, find/replace |
| TMDL view | 🟡 | 2 | read-only | Apply |
| Power Query editor (Transform data) | 🟡 | 1 | read-only queries, parameters, blob map, Load CSV; a PBIT refreshes on open with its saved parameter values (Desktop's "Enter parameters" prompt is missing) | Applied-steps UI, data preview, parameter editing |
| Save / Save as (PBIP) | ✅ | 4 | `doc.save` (in-place PBIP save, conflicts, `overwrite`) and `doc.saveAs` (a new PBIP project from PBIX, PBIT, `model.bim` or PBIP; later saves go there) registry commands (`crates/pbi-ops/tests/save.rs`); Desktop opens a saved, edited Acme copy and computes the same values (GT-6, `reference/groundtruth/README.md`); saved TMDL equals Desktop's after the same edits (GT-4 s24; Desktop's line ends and blank last line kept); the app's Save (Quick Access Toolbar, File, Ctrl+S), Save as (Ctrl+Shift+S), `*` unsaved marker, conflict question and *Unsaved changes* prompt on close, open and window close (M6.8; `scripts/app-checks/editing-smoke.mjs`: a saved formula edit changes one line) | Recovery files (M8); the prompt on macOS Cmd+Q once Tauri can intercept it |
| Undo / redo | ✅ | 4 | `doc.undo`/`doc.redo` per document (`pbi-model::History`, one step per `doc.applyEdits`); the Quick Access Toolbar's Undo and Redo name the step, Ctrl+Z / Ctrl+Y (text fields keep their own undo) (M6.8) | Undo of report-authoring edits as they land (M9, M18) |
| Several documents, several models per project | ❌ | 1 | one document; first model only | — |
| Export from the app (data, PDF, PowerPoint) | 🟡 | 1 | File → Export writes a PBIP project or PBIT template (`convert.pbip`/`convert.pbit`); table data, PDF and PowerPoint: CLI or not at all | `export.table` from the Table view; PDF |
| Get data / connectors | ❌ | 0 | offline policy; "Load CSV manually…" | — |
| Performance analyzer, Bookmarks, Sync slicers panes, Mobile layout | ❌ | 0 | shown disabled | — |
| Settings persistence | 🟡 | 2 | recent files and AI settings (`ai.json`) persist; the blob map is per session (app/README "Known gaps") | Persist the other settings |
| Opt-in AI: Options → AI, consent dialog with the exact request, Explain with AI, Suggest a measure | ✅ | 3 | app/README, [AI.md](AI.md#in-the-desktop-app); hostile-input tests of the Markdown renderer (`app/tests/markdown.test.ts`) | AI in the DAX query view |
| Modeling → View as (RLS roles, other user) | ✅ | 3 | `app/src/components/ViewAsDialog.svelte` over `security.roles` / `security.viewAs` (§4) | — |
| Keyboard shortcuts | 🟡 | 3 | desktop-ui-reference.md "Keyboard" | — |
| Accessibility | 🟡 | 2 | the M6 editing surfaces pass A1–A5 (keyboard, focus, names and roles with no checker violation, contrast, reduced motion) on Windows / WebView2 ([accessibility/m6-editing.md](accessibility/m6-editing.md), `scripts/app-checks/editing-a11y.mjs`); the rest of the app is not audited | Screen-reader runs (A6: VoiceOver, Narrator, NVDA, Orca); audit (M31.8) |
| Localization | ❌ | 0 | English only; no i18n layer found | — |
| Cancellable open | ❌ | 0 | readers are synchronous | — |
| Publish to the service | ❌ | n/a | — | Out of scope (v1) |

## 11. Platform and packaging

Source: [docs/packaging.md](packaging.md), [docs/ci.md](ci.md), `.github/workflows/`.

| Capability | Status | Score | Evidence | Next step |
|---|---|---|---|---|
| CI matrix: macOS arm64, Windows x64, Ubuntu (fmt, layers, clippy, tests, app, deny, perf) | ✅ | 5 | `.github/workflows/ci.yml`; tested frontend-only selection; both full validation attempts passed, warm workflow 7m02s ([CI](ci.md#build-reuse-and-test-execution)) | Cache APT download archives; monitor cold builds and cache storage |
| macOS app + dmg (arm64, x64) | 🟡 | 3 | `release.yml`; ad-hoc signature without secrets | Signing + notarization |
| Windows msi + nsis | 🟡 | 3 | `release.yml`; unsigned | Code signing |
| Linux AppImage + deb | ✅ | 4 | `release.yml` (ubuntu-22.04 glibc); deb package `dash-craft` ([decision](packaging.md#linux-package-name-dash-craft-decided)) | — |
| Code signing and notarization | ❌ | 0 | gated on secrets | Blocked: certificates |
| Release workflow (tag checks, notes, SHA-256, CycloneDX + npm SBOMs, attestations) | ✅ | 4 | `release.yml`, `scripts/changelog-section.sh` | — |
| CLI archives per platform | ✅ | 4 | `release.yml` | — |
| File associations (`.pbix`, `.pbit`, `.pbip`, `.bim`, `.vpax`) | ✅ | 4 | docs/packaging.md | — |
| macOS Finder / "Open With" into the running app | ✅ | 4 | `RunEvent::Opened` → `open-file-requested` | — |
| Single-instance forwarding | ❌ | 0 | app/README "Known gaps" | — |
| Auto-update | ❌ | 0 | updater not enabled (docs/packaging.md) | Needs signing first |
| Windows ARM64, Linux ARM64 builds | ❌ | 0 | not in the release matrix | — |
| Licence inventory (`THIRD_PARTY_NOTICES.md`) | ✅ | 4 | `scripts/license-inventory.py --check` in CI (warning) | — |

## 12. Security and fuzzing

Source: [SECURITY.md](SECURITY.md), [docs/ci.md](ci.md#fuzz-targets).

| Capability | Status | Score | Evidence | Next step |
|---|---|---|---|---|
| No `unsafe`; no `unwrap`/`expect` outside tests | ✅ | 5 | workspace lints; clippy `-D warnings` | — |
| Untrusted-input hardening (caps, zip-slip, symlinks, DTDs refused) | ✅ | 5 | `crates/pbi-core/tests/containers.rs`; fuzzing found and fixed a drive-letter zip-slip | — |
| Fuzzing: 16 `cargo-fuzz` targets, nightly | ✅ | 4 | `fuzz/fuzz_targets/`, `.github/workflows/fuzz.yml` | Dedicated targets for RDL, VPAX, legacy XML metadata (only via `open_bytes` today) |
| Secret redaction in metadata and connection strings | ✅ | 4 | FORMATS.md; reader tests | — |
| App hardening (CSP, minimal capabilities, sanitized SVG) | ✅ | 4 | app/README "Security" | — |
| No script execution, no telemetry | ✅ | 5 | AGENTS.md rules 4–5 | — |
| Supply chain (`cargo-deny` licences, bans, sources, advisories) | ✅ | 4 | `deny.toml`, CI `deny` job (advisories non-blocking) | — |
| Agent path policy | 🟡 | 3 | see §9 | `cap-std` |
| Legal review of ADR-0003 UNVERIFIED rows | ❌ | 0 | ADR-0003 | Blocked: human decision |
| Staged-file / secret scanning in CI | ❌ | 0 | manual rule (AGENTS.md §8); `.gitignore` | A CI check for forbidden paths and secrets |
| Registry parameter fuzzing | ✅ | 4 | see §9 | — |
| Security policy / threat model document | 🟡 | 2 | README "Security posture", ARCHITECTURE "Security boundaries", ADR-0008 threat model; no `SECURITY.md` | Write a `SECURITY.md` |

## How to update this file

- Update the row when you change a capability, in the same change: status, score, evidence (a
  file or test that proves it) and the next step. Add a row for a new capability.
- Recompute the area's average (numeric scores only; `n/a` and `?` excluded, one decimal) and the
  ✅/🟡/❌ counts in the summary, and update the "As of" line.
- Take numbers from the code: DAX from `supported_functions()` / FUNCTIONS.md, M from
  `pbi_m::library_names()` or the `src/stdlib` registrations, commands from `dashcraft commands`,
  perf from `perf/baseline.json` (re-record it when a change moves the numbers), golden scores from
  `cargo test -p pbi-render --test golden -- --nocapture` (when local golden images exist).
- Never raise a score without evidence; lower it when a regression is found. Mention the change in
  the [dev log](DEVLOG.md).
