# Changelog

All notable changes to DashCraft (named pbiview before [ADR-0009](docs/adr/0009-naming-dashcraft.md); older entries keep that name). The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- **Command registry (`pbi-ops`, [ADR-0007](docs/adr/0007-command-registry.md))**: 43 typed commands (`doc.*`, `model.*`, `report.*`, `data.*`, `dax.*`, `measures.values`, `render.*`, `canvas.*`, `export.*`, `convert.*`, `text.parsePeople`) with JSON Schemas for params and output, a `Workspace` of open documents, cancellation, progress events, caller kinds and a `PathPolicy` sandbox (read/write roots, `..` and symlink escapes refused). Catalog: [docs/COMMANDS.md](docs/COMMANDS.md).
- CLI: `pbiview commands` (catalog as text, JSON, schemas or Markdown), `pbiview run <id>` (single command or `--batch` file with `${step.field}` references), `pbiview serve --stdio` (JSON-RPC 2.0 lines, `$/cancel`, progress notifications, `--read-root`/`--write-root`/`--agent`).
- `pbiview mcp`: a Model Context Protocol server on stdio generated from the command registry (no per-tool code): agent-safe commands as tools (`doc_open`, `measures_values`, …; `--tool-names dotted`), JSON Schemas for inputs and outputs, hints from the command flags, PNG renders as image content, progress and cancellation, `--allow`/`--deny`, and read/write roots (default read root: the current folder; writes refused without `--write-root`). Protocol revisions 2024-11-05 to 2025-11-25 and stateless 2026-07-28. See [docs/MCP.md](docs/MCP.md).
- **Opt-in AI layer (`pbi-ai`, [ADR-0008](docs/adr/0008-ai-layer.md), [docs/AI.md](docs/AI.md))**: off by default; `ai.explainMeasure` and `ai.generateMeasure` (parsed and optionally evaluated locally, never applied) through OpenAI-compatible, Azure OpenAI or Ollama providers, only after a recorded per-provider, per-capability, per-data-class consent; `ai.preview` shows the exact request with keys redacted; keys only in the OS keychain or `DASHCRAFT_AI_API_KEY`; `cargo xtask layers` keeps HTTP/TLS and keychain crates inside `pbi-ai` and its dependents.
- CLI: the `ai.*` commands are available through `pbiview run`, `serve`, `mcp` (agent-safe ones) and `commands`; `pbiview ai login --provider <id>` stores a key in the OS keychain (prompted without echo, or `--stdin`), `pbiview ai logout` deletes it.
- `cargo xtask perf` (budgets in `perf/budgets.toml`, baseline in `perf/baseline.json`) and `cargo xtask layers` (crate layering and UI-toolkit containment), both in CI.
- Carlito (OFL-1.1) bundled as the metric-compatible Calibri substitute.
- **Calculation groups**: typed `CalculationGroup`/`CalculationItem` in the model, read and written losslessly in TMDL and TMSL; calculation-group tables load their items; measures apply the selected item (precedence, selection expressions, sideways recursion, dynamic format strings) with `SELECTEDMEASURE`, `SELECTEDMEASURENAME`, `ISSELECTEDMEASURE` and `SELECTEDMEASUREFORMATSTRING`.
- **Many-to-many relationships** are evaluated as limited relationships (filters cross as key values in the cross-filter direction, no blank row, `RELATED` refused); `ENG003` is now informational.
- **Row-level security "View as"**: `Session::set_view_as` / `set_effective_roles` restrict every query to the rows of the chosen roles (DAX row filters, security-direction propagation, union over roles) and set the user for `USERPRINCIPALNAME`, `USERNAME`, `USEROBJECTID` and `CUSTOMDATA`.
- **View as roles** in the desktop app (Modeling → View as: role checkboxes and Other user, a "Now viewing as" bar with Stop viewing; canvas, grid and measures re-query) and the registry commands `security.roles` (roles, DAX table filters, members, dynamic) and `security.viewAs` (apply or stop, visible rows per table; agent-safe).
- `NOTICE`: DashCraft is not affiliated with, endorsed or sponsored by Microsoft (Power BI and Microsoft are its trademarks, used only to describe file compatibility), nor with storytold/ArtCraft or other products named "DashCraft". Linked from the README, summarised in the app's About page and shipped with the app (`licenses/NOTICE`) and the CLI archives.
- Report renderer: real `scatterChart` (bubbles, legend, numeric axes; first play-axis frame), `funnel` (conversion-rate labels), `gauge` (min/max/target, default max = 2 × value), `treemap` (squarified, group/details) and `waterfallChart` (sentiment colors, breakdown top N + Other) visuals with hit ids, replacing their placeholders; `Breakdown` is queried as a series and waterfalls sort by category by default ([docs/VISUALS.md](docs/VISUALS.md)).
- **DAX function library: 218 → 294 functions**: logarithms, exponentials and trigonometry (`LN`, `LOG`, `EXP`, `SIN` … `ACOTH`), `FACT`/`COMBIN`/`PERMUT`/`GCD`/`LCM`/`QUOTIENT`/`EVEN`/`ODD`, `MROUND`/`CEILING`/`FLOOR`/`ISO.CEILING`, deterministic `RAND`/`RANDBETWEEN`, the bit functions, `GEOMEAN(X)`, `MAXA`/`MINA`/`AVERAGEA`, the `NORM`/`EXPON`/`POISSON`/`BETA`/`CHISQ`/`T`/`CONFIDENCE` distributions (own incomplete gamma/beta and inverses, ~1e-13 relative accuracy), `YEARFRAC`, `NETWORKDAYS`, `IF.EAGER`, `ERROR`, `EVALUATEANDLOG`, `USERCULTURE`, `ISAFTER`/`ISONORAFTER`; `EDATE` now truncates its months and `QUARTER(BLANK())` is BLANK ([FUNCTIONS.md](crates/pbi-dax/FUNCTIONS.md)).
- Desktop app: the opt-in AI features (still off by default): File → Options and settings → Options → **AI** (turn on, providers, write-only API keys in the OS keychain, default provider, sample rows, consents with Revoke), a consent dialog that shows the exact request (`ai.preview`) before anything is sent, **Explain with AI** for measures (Data pane, formula bar, Properties, ribbon; progress, Cancel, safely rendered Markdown) and Home/Modeling → AI → **Suggest a measure** (DAX with diagnostics, local evaluation, never applied); `ai.status` gains `settings`; the app's typed client covers `ai.*` (generator moved to `pbi-ai`'s tests); debug builds answer from a demo provider with `DASHCRAFT_AI_MOCK=1`.
- Report renderer: **conditional formatting** of tables and matrices (background and font color by gradient with 2 or 3 colors, rules with ranges, percent-of-range and blanks, or field values; data bars with negatives, axis, direction and "show bar only"; built-in icons by rules or field value, drawn as vector shapes) and of bar, column, pie, donut, treemap, scatter and funnel colors; rule inputs that are not in the visual are queried as hidden fields, unsupported variants render unformatted with a warning ([docs/VISUALS.md](docs/VISUALS.md#conditional-formatting)).
- Report renderer: **dynamic format strings**: measures under calculation items (or with a `formatStringDefinition`) format each cell with its effective format string (a `YOY %` item shows `82.4%` next to `$960`) in cards, tables, matrices and chart labels; `pbi-dax` gains `EvalContext::evaluate_measure_format_strings` (one batch per measure and result set).

### Changed
- Report rendering caches each visual by a content hash (theme, frame, definition, data, selections, view state); scroll, selection and search re-render only the affected visuals. 50k-row table scroll step p95 25.6 ms → 0.75 ms; small-table scroll steps 86 ms → under 1 ms.
- Text shaping is cached per (text, family, weight, size); grid layout is memoised by content fingerprint.
- DAX engine: one-pass grouped `SUMMARIZECOLUMNS` for plain aggregations (P02 50k-row first render 13.6 s → 0.1 s) and compiled columnar row expressions for `SUMX`/`AVERAGEX`/`MINX`/`MAXX`/`COUNTX`/`COUNTAX` (P04 114 → 22 ms); indexed `IN` filters and relationship-index propagation speed up the general path (~10×). Differential tests compare fast and general paths on random models.
- Golden-image comparison: better text line metrics raise the page SSIM, and a multi-line textbox region is back to the standard 0.85 threshold.
- **The desktop app is a thin front end over the command registry**: documents live in a `pbi_ops::Workspace` and every domain operation (open/close, refresh with progress and cancel, grid pages, statistics, CSV loads, derived columns, DAX queries, measures, the interactive canvas) runs through one generic `ops_execute` IPC command → `Registry::execute`, the same commands as `pbiview run`/`serve` and agents; a typed TypeScript client (`app/src/lib/ops.gen.ts`, `npm run gen:ops`) is generated from the registry's JSON Schemas and checked by `cargo test`. The app's duplicated canvas, grid, refresh and DTO code is gone (about 4,400 lines of app Rust, net); its "newest completed render wins" canvas ordering and legacy report themes moved into `pbi_ops::canvas`. New commands `report.filters` and `text.previewPeople`; `data.page`/`dax.query` gain `blankAsNull`, `data.tables` `canLoadCsv`, refresh statuses `offline`. File → Export now writes a PBIP project or PBIT template (`convert.pbip`/`convert.pbit`).
- **Renamed to DashCraft** ([ADR-0009](docs/adr/0009-naming-dashcraft.md)). The CLI binary is `dashcraft`; `pbiview` stays as an alias binary (same program; help, usage and messages use the name it was invoked as). The desktop app is "DashCraft" (binary `dashcraft-app`, bundle identifier `io.github.jxck-s.dashcraft`). `DASHCRAFT_APP` and `DASHCRAFT_BLOB_MAP` replace `PBIVIEW_APP` and `PBIVIEW_BLOB_MAP`, which still work (the new name wins). `dashcraft open` finds `dashcraft-app` / `DashCraft.app`, then the old names. Release artifacts are `DashCraft_<ver>_*` installers and `dashcraft-<tag>-<target>` CLI archives (with both binaries); Linux MIME types are `application/x-dashcraft-*`. The MCP server reports `serverInfo.name` `dashcraft` and `dashcraft/…` `_meta` keys. JSON outputs, schemas, command ids and exit codes are unchanged; crate names stay `pbi-*`.
- MCP tool hints for commands that run other commands come from a new `CommandMeta::invokes_other_commands` flag (set on `commands.batch`) instead of a hard-coded id list; behaviour unchanged.
- Scatter-point tooltips (`canvas.tooltip`, the app) show the X and Size values around Y, after the Details and Legend values; the last user-facing "pbiview" strings say DashCraft (the hint for unsupported DAX in refresh and derived-column statuses, the derived-column description, the lint rules title, temporary-file prefixes `.dashcraft-*.tmp`).
- Test fixtures replaced by the synthetic **Acme Outfitters** example (`reference/examples/acme-outfitters/`, generated by `scripts/examples/gen_acme.py`; `scripts/examples/check_acme.py` checks DashCraft against its expected values) and MIT-licensed Microsoft sample reports (`reference/samples/microsoft/`; larger ones through `scripts/fetch-samples.sh`). Rendering is checked by `insta` render snapshots of every Acme page and the 39 card values of its `expected.json` on rendered pages; the golden-image test compares a page only when an optional local `reference/golden/<example>/<report>/<page>.png` exists (SSIM ≥ 0.85, no exceptions).
- Perf scenarios P03, P06 and P08 run on the Acme Outfitters example and the Microsoft samples.
- The desktop app's bundle identifier is `io.github.jxck-s.dashcraft`.
- `.gitignore` refuses `*.pbix`, `*.pbit` and `*.abf` files except the committed Microsoft samples and fuzz seeds.

### Fixed
- TMDL ↔ TMSL conversion keeps dynamic format strings: the `formatStringDefinition` object form is converted in both directions.
- One-to-one relationships relate each side's unmatched rows to the other table's blank row.
- The people transform ("Parse people field", `text.parsePeople`) accepts plain lists of names as well as lists of person records.
- `kpi` visuals use the `TrendLine` field as the trend axis (indicator and goal at its last point).
- Matrix row and column levels follow each column's sort-by column.

### Documentation
- Test fixtures and doc examples use the synthetic **Acme Outfitters** example (`reference/examples/acme-outfitters/`: PBIP, CSV data, blob map, expected values) and MIT-licensed Microsoft sample reports (`reference/samples/microsoft/`; larger ones through `scripts/fetch-samples.sh`); AGENTS.md gains the rule that real or internal reports and data are never committed or described.
- [docs/VISUALS.md](docs/VISUALS.md#render-verification), README, AGENTS.md and `reference/README.md` describe render verification (render snapshots, expected values, Microsoft samples, optional local goldens); [docs/GAPS.md](docs/GAPS.md) re-scored without golden images (visuals 2.4 → 2.1, rendering fidelity 3.4 → 3.2).
- [AGENTS.md](AGENTS.md) is the canonical contributor and AI-agent guide (golden rules, parallel-agent protocol, validation, definition of done, naming and trademark policy), with a gaps scorecard ([docs/GAPS.md](docs/GAPS.md)) and a dev log ([docs/DEVLOG.md](docs/DEVLOG.md)); `.github/copilot-instructions.md` now points to it.
- Docs synced with the code: v0.2.0 is released and packaging done; golden SSIM results; 336 M library names; `pbi-ops`, `pbi-ai` and ADR-0007 to ADR-0009 in the README and the crate graphs (the app now depends on `pbi-ops`, not on `pbi-render`/`pbi-engine`); ADR-0007 status (MCP done, workspace dependency); the generated command catalog links [docs/MCP.md](docs/MCP.md); `inspect_model.rs` paths; macOS Finder opens are wired; [docs/GAPS.md](docs/GAPS.md) re-scored (MCP, app on the registry, AI layer, P02/P04, one-pass group-by).

## [0.2.0] — 2026-10-22 — Phase 2: Report Renderer

Renders report pages offline, headlessly (`pbiview render`) and interactively in a Power BI Desktop-style app, from the data and engines of Phase 1. Also adds the remaining formats of [FORMATS.md](FORMATS.md) §5, PBIP local-cache data, a larger DAX library and session-derived columns.

### Added

#### Report renderer (`pbi-render`)
- **Theme resolution**: built-in fallback, base and custom themes deep-merged; `ThemeDataColor{ColorId, Percent}`; `visualStyles` precedence (visual `objects` → theme per type and preset → `"*"` → built-in defaults); primary and derived text classes.
- **Layout**: page size and scaling (`FitToPage`, `FitToWidth`, `ActualSize`), page background and outspace, paint order (z, then tab order), visual groups, container chrome from `visualContainerObjects` (background, border with radius, drop shadow, padding, title with auto titles and wrapping).
- **Visual queries**: `queryState` projections, report/page/visual filters, slicer selections and selected data points (per `visualInteractions`) → `EVALUATE SUMMARIZECOLUMNS` → Power BI-shaped DataViews (`single`, `categorical`, `table`, `matrix`) with totals and subtotals, implicit measures, default sorts, query limits and formatted cells. Report filters skip visuals with no relationship path to the filtered table. Filters that can't be applied are reported, never dropped silently.
- **Visuals**: `textbox` (rich runs, lists, links, overflow scrollbar), `shape`/`basicShape` (22 shape kinds), `actionButton` (icons, text, actions), `cardVisual`, `card`, `multiRowCard`, `kpi`, `slicer` (dropdown, list, tiles, range) and `advancedSlicerVisual`, `tableEx` (auto-sized, wrapping, banded, scrollable), `pivotTable` (stepped or tabular, subtotals, expansion), pie and donut (leader-line labels), clustered/stacked/100% bar and column, line, area, stacked area, line + column combos (axes with nice ticks, legends, data labels, `dataPoint` colors, category scrolling). Other types draw a placeholder with their fields; failed queries draw ⚠ "Can't display this visual".
- **Service quirks reproduced**: the new card's inner tile border and label clipping below ~100 px, honoured card font sizes, slicer header truncation, table auto-sizing and wrapping, pie label placement and truncation, Calibri metrics, browser line metrics.
- **Output**: retained scene graph with hit ids → deterministic SVG and PNG (`resvg`, bundled Selawik fonts only: byte-identical on every OS).
- **Golden-image comparison** (`pbi_render::golden`): page alignment, per-region Gaussian SSIM, diff heatmaps (written under `target/` only).
- Interactive rendering API: `query_page_data` + `render_interactive_page` (scroll, expansion, open dropdowns as an overlay, per-visual interactive geometry), `InteractionState::from_page` (selections saved in the report).

#### CLI
- `pbiview render`: `--png`, `--svg`, `--out-dir` with `--format png|svg|both`, `--report`, `--page` (name, display name, index or part of it), `--all-pages`, `--scale` (default 2, at most 16), `--slicer "SLICER=V1,V2"` (`!=` to invert, `=` to clear), `--now` for reproducible clocks, `--json` per-visual status. Exit code 2 when the refresh or a visual fails.
- `refresh`, `measures`, `export --table` and `render` take `--source auto|m|cache`; `auto` (the default) loads a PBIP's `.pbi/cache.abf` when present and notes it on stderr. `refresh --json` reports `kind: "cache"`.
- `export --excel-safe` turns on the CSV formula-injection guard.

#### Desktop app
- **Power BI Desktop-style shell** (read-only): title bar with search (Alt+Q), ribbon with Desktop's tabs and groups (unsupported commands disabled with a tooltip), File backstage with persisted **Recent** files, view rail (Report, Table, Model, DAX query, TMDL), Filters / Visualizations / Data / Selection / Properties panes, formula bar, page tabs, status bar with zoom. Fluent UI System Icons (MIT) and Selawik (OFL); no Power BI assets.
- **Interactive report canvas**: the page rendered by `pbi-render` as sanitized SVG; slicers (single and multi-select, Select all, dropdown popups, search, clear), cross-filtering from pie, bar, column, line, table and matrix points per `visualInteractions`, data-point and error tooltips, table and matrix scrolling, matrix expand/collapse, Back and page-navigation buttons (Web URL shown, never opened), Reset to default, keyboard focus. Renders are cancellable and query results are cached per page.
- Filters pane cards follow view-mode rules (hidden cards, locks) and list the canvas's selections; the Measures panel evaluates in the canvas's filter context by default.
- **DAX query view** (`EVALUATE` with `DEFINE`/`ORDER BY`, run selection, cancel, results with copy, quick queries), **TMDL view** (verbatim or generated), **Model view** diagram (`diagramLayout.json` positions or auto-layout), read-only **Transform data** editor, Design → Themes, Format pane cards.
- **Local data cache**: a PBIP with `.pbi/cache.abf` loads its cached data on open, with a "Loaded cached data (as of …)" banner and **Refresh from source**.

#### Engines and session
- **DAX** (`pbi-dax`): 210 functions (187 supported, 23 partial; up from about 120), including time intelligence, `ALLSELECTED` (shadow filter context), `ISINSCOPE`, `ROLLUPADDISSUBTOTAL`/`ROLLUPGROUP` in `SUMMARIZECOLUMNS`, `RANKX`, window functions (`INDEX`, `OFFSET`, `WINDOW`, `RANK`, `ROWNUMBER`), statistics, parent-child functions, `USERELATIONSHIP` and `CROSSFILTER`. Coverage table: `crates/pbi-dax/FUNCTIONS.md`, kept in sync by tests.
- **Storage engine** (`pbi-engine`): per-filter-context relationship overrides (activate inactive relationships, change cross-filter direction).
- **Derived columns** (`pbi-session`): `Session::add_derived_column` / `remove_derived_column` / `derived_columns` with `DerivedColumn::People` ("Parse people field"), `Expression` (DAX) and `Array`. Stored in the engine table, so DAX, SQL, the grid, statistics and exports (Parquet field metadata `pbiview.derived = true`) see them; re-applied in creation order after every refresh, cache load and CSV load; never saved to the model. Name collisions with a column or measure are errors.
- **PBIP local data cache**: `Session::load_cached_data` maps `<Model>.SemanticModel/.pbi/cache.abf` onto the model by name (no M, no network). Calculated objects missing from the cache are computed; a missing table fails with a "refresh from source / Load CSV" hint; a missing data column loads as BLANK with `SESSION005`. `RefreshOptions::source` (`Query`, `Cache`, `Auto`), `Session::cache_info`, `has_cached_data`, `RefreshKind::Cache`.
- **CSV formula-injection guard**: `CsvExportOptions::escape_formulas` (off by default) prefixes text cells and header names that start with `=`, `+`, `-`, `@`, TAB or CR with `'`; numbers, dates and booleans are never changed.

#### Formats
- **Excel Power Pivot** (`.xlsx`/`.xlsm`): model **and data** from `xl/model/item.data`, with per-file Xpress (MS-XCA Plain LZ77) decompression and legacy XML metadata (compatibility level 1100/1103, `.tbl.xml` storage).
- **VPAX**: model (from `Model.bim` or rebuilt from `DaxModel.json`) and VertiPaq Analyzer statistics.
- **RDL** paginated reports: outline of data sources (connection strings always redacted), datasets, parameters and report items; DTDs refused, sizes capped.
- **Power Query**: `.pq`/`.m` scripts, `.pqt` templates, dataflow `model.json` (CDM entities as tables with M partitions), Dataflow Gen2 definitions.
- **Custom visuals and connectors**: `.pbiviz` metadata and `.mez`/`.pqx` listings; never executed.
- PBIP `.pbi/cache.abf` metadata part (`inspect` → `model.cacheAbf`). New `inspect --json` sections (additive, `schemaVersion` 1): `vpaxStats`, `paginatedReport`, `mashup`, `customVisual`, `connector`.

#### Packaging
- Original app icon (no Microsoft or Power BI marks), bundles for macOS (app + dmg, arm64 and x64), Windows (msi + nsis) and Linux (AppImage + deb), and file associations for `.pbix`, `.pbit`, `.pbip`, `.bim` and `.vpax` (viewer role, not claimed as default). The GUI binary is `pbiview-app`; the `pbiview` CLI ships as separate per-OS archives.
- macOS document opens (Finder double-click, "Open With") reach the running app as an `open-file-requested` event; Windows and Linux pass the path on the command line.
- Release workflow: tag/version consistency checks, release notes from this changelog, SHA-256 sums, CycloneDX + npm SBOMs, signing and notarization only when the secrets exist (unsigned builds otherwise). License inventory generated into `THIRD_PARTY_NOTICES.md` (`scripts/license-inventory.py`); see [docs/packaging.md](docs/packaging.md).

#### Tests and documentation
- Golden-image regression tests against Power BI service exports (per-region SSIM ≥ 0.85; regions whose text uses fonts pbiview does not bundle, such as DIN titles and Calibri text, have documented lower thresholds). Card values are asserted on the rendered scene and SVG for each report.
- Renderer unit and reference tests per visual family; CLI end-to-end `render` tests (determinism, JSON status, exit codes); app tests that drive the canvas through Tauri's mock runtime.
- Docs: rewritten README with format and visual matrices, [docs/CLI.md](docs/CLI.md), [docs/USER-GUIDE.md](docs/USER-GUIDE.md), [docs/VISUALS.md](docs/VISUALS.md), [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), final [FORMATS.md](FORMATS.md), [ADR-0006](docs/adr/0006-interaction-state.md).

### Changed
- The app's "Parse people field" uses the `pbi-session` derived-column API: the column is now visible to measures, DAX queries, SQL filters and exports, not only the grid.
- The app's former KPI strip is the **Measures** panel under the canvas, and the former Queries tab is Home → Transform data.
- ADR-0005's interaction design is superseded by [ADR-0006](docs/adr/0006-interaction-state.md): selections are view state kept per page, not document commands.

### Known gaps
- **Renderer**: highlight interactions filter instead of highlighting; bookmarks, drillthrough, Q&A, synced slicers, report-page tooltips, visual header menus, drill-down/up, conditional formatting, the Analytics pane and small multiples are not supported; range-slicer sliders are not draggable; `image` visuals and wallpaper images are not drawn; other visual types (maps, funnel, treemap, gauge, scatter, waterfall, AI and custom visuals) are placeholders.
- **Fonts**: DIN and Calibri render with Selawik (Calibri metrics are emulated), the main source of differences from the service.
- **Formats**: report-level measures (`reportExtensions.json`) are not evaluated; PBIX `Report/Layout` and PBIR-Legacy reports render with the default theme in the CLI; `DataMashup` is not decoded; no RDL renderer.
- **Engines**: calculation groups, many-to-many relationships, RLS and DirectQuery are not supported; DAX and M text are not rewritten by rename commands.
- **App**: one document at a time (extra models of a multi-model project are not shown); no exports from the app (use the CLI); column statistics cover the whole table; the blob map is not persisted.
- **Release**: installers are unsigned until signing certificates are provided; shipping PBIX data decoding needs the legal review recorded in ADR-0003. Validation against Power BI Desktop on Windows (DAX differential tests, Desktop-rendered references) is pending a Windows machine.

## [0.1.0] — 2026-10-21 — Phase 1: Data Explorer

Opens Power BI files offline on macOS, Windows and Linux, without Power BI Desktop, a Windows VM or a Fabric login.

### Formats
- **PBIP** projects (folder or `.pbip`):
  - Models in TMDL or legacy `model.bim`.
  - Reports in PBIR or PBIR-Legacy.
  - `byPath` and `byConnection` dataset references; a `byConnection` model is shown as remote.
  - Also reads `.platform`, `definition.pbism`, `.pbi/localSettings.json` and `diagramLayout.json`.
- **PBIX**:
  - Report from PBIR (`Report/definition/`) or the legacy `Report/Layout`.
  - Model schema and **embedded data** from `DataModel`, decoded by a clean-room XPress9 + ABF + VertiPaq implementation.
  - `SecurityBindings` is treated as opaque.
- **PBIT**: `DataModelSchema` (TMSL, UTF-16LE) plus the report.
- **Other inputs**: standalone `.bim`, TMDL folders, `.pbids`, and `.abf`.
- **Detection by content**: ZIP entries, JSON keys, `$schema` and encoding decide the format, not only the extension.
- **Lossless round-trips**:
  - TMDL re-emits byte-for-byte.
  - The PBIR writer reproduces all fixture files byte-for-byte.
  - TMSL preserves its layout.
  - Unknown fields are always preserved.

### Engines
- **M (Power Query) evaluator**:
  - Lazy, with closures, `each`, `try`/`otherwise`, `meta` and sections.
  - About 200 library functions, including `Csv.Document` and the `Table.*`, `Text.*` and `List.*` families.
  - `AzureStorage.Blobs` resolves through an offline blob map (`blob-map.json`). M code has no file or network access.
- **DAX formula engine**:
  - Filter and row context, context transition, expanded tables, blank semantics.
  - `CALCULATE` modifiers, about 120 functions including `FORMAT`.
  - `DEFINE`/`EVALUATE` queries, calculated columns and tables.
- **Storage engine**:
  - Arrow columnar store with relationship indexes (blank row included).
  - Single and bidirectional filter propagation; a table with no relationship path to the filter is unaffected.
  - DataFusion SQL for the grid.
- **Refresh**:
  - Order is M partitions, then calculated columns, then calculated tables, by dependency.
  - Transactional and cancellable, with a per-table status.
  - When a table fails, "Load CSV manually…" is offered as a fallback.

### Explorer
- **CLI** (`pbiview`): `inspect [--json]`, `detect`, `open`, `convert --to pbip|pbit`, `lint`, `refresh`, `measures [--report]`, `export`.
  - The `inspect` and `lint` JSON outputs carry `schemaVersion: 1`.
- **Desktop app** (Tauri 2 + Svelte):
  - Open by dialog, drag-and-drop or command-line argument.
  - Model tree, relationships, M/DAX code views, report outline with a wireframe preview and filters, Properties pane, diagnostics.
  - Virtualized data grid with sorting, filters and column statistics.
  - Refresh with progress; measures panel with values under report filters; "Parse people field".
- **Linter**: 17 broken-reference rules, each pointing at the file and JSON pointer, with did-you-mean suggestions.
  - It detects stale `queryRef`s left behind by renamed measures or columns.
- **Exports**: table to CSV or Parquet; model to a JSON or Markdown data dictionary, including `///` descriptions.

### Verified
- Measure and card values under report and page filters, row counts after the M steps, and the "no relationship path" filter behaviour are asserted in tests against independently computed expected values.
- Decoded PBIX data is cross-checked against pbixray (MIT) by hash.

### Security
- All input is treated as untrusted:
  - Zip-slip and decompression-bomb guards; symlinks are never followed.
  - Size, entry and recursion limits.
  - No `unsafe` code; parsers never panic.
- 14 cargo-fuzz targets, run nightly in CI. Fuzzing found and fixed a drive-letter zip-slip bypass.
- Embedded scripts, custom visuals and connectors are never executed. There is no telemetry.
- Connection strings and secrets are redacted from metadata.

### Known gaps
- **Report rendering** is Phase 2. The app shows a wireframe preview only.
- **M, DAX and storage engine coverage**:
  - Not supported: DAX time intelligence, `USERELATIONSHIP`/`CROSSFILTER`, calculation groups, and many-to-many relationships.
  - M ignores culture settings.
- **PBIP cache data**: `.pbi/cache.abf` data loading, `.vpax`, `.rdl`, `.pqt` and Excel Power Pivot are detected but not read yet.
- **Release status**:
  - The "Parse people field" derived column is session-only and visible to the grid only (not to DAX/SQL/exports yet); column statistics cover the whole table, not the filtered view.
- CSV exports do not yet offer an opt-in spreadsheet formula-injection guard.
- Release artifacts are unsigned until signing certificates are configured.
  - Shipping PBIX data decoding needs the legal review recorded in ADR-0003.
