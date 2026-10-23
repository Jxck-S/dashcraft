# Roadmap

Milestones M0–M5 and their work items. Fixtures and their expected values are described in [AGENTS.md §7](../AGENTS.md#fixtures-and-expected-values). Architecture decisions are in [`docs/adr/`](adr/).

**Gate:** Phase 1 (Data Explorer) is tagged `v0.1.0` after M2, before Phase 2 (Report Renderer) work is merged. Phase 2 (M3–M4 plus M5 formats and docs) ships as `v0.2.0` ([CHANGELOG](../CHANGELOG.md)).

Legend: `[x]` done, `[ ]` not done. Item IDs match the work-tracking IDs used in sessions.

## M0: Foundation

- [x] `m0-scaffold`: git repository; Cargo workspace with all crate stubs; pre-declared `[workspace.dependencies]`; `rust-toolchain.toml` (1.99.1, with rustfmt and clippy); `.gitignore`; `cargo build` is green.
- [x] `m0-fixtures`: `reference/` fixture root (now the synthetic Acme Outfitters example with its CSVs, `blob-map.json` and expected values, and MIT-licensed Microsoft sample reports); optional data gitignored; `reference/README.md`; `PBIVIEW_REFERENCE` override.
- [x] `m0-docs`: ADR-0001 to ADR-0005 (including license research), `FORMATS.md`, `README.md`, `THIRD_PARTY_NOTICES.md`, `.github/copilot-instructions.md`, this roadmap.
- [x] `m0-ci`: GitHub Actions matrix (macOS arm64, Windows x64, Ubuntu): `cargo fmt --check`, `clippy -D warnings`, `cargo test`; app build job; `cargo-deny`; a release job stub with signing gated on secrets.

## M1: Readers, model, CLI inspect, explorer shell

- [x] `m1-model`: `pbi-model` TOM-like semantic model (tables, columns, measures, calculated columns and tables, M or calculated partitions, relationships, expressions and parameters, annotations, `lineageTag`, descriptions, unknown properties); report AST; `Document` with `Command` apply/undo and change events.
- [x] `m1-tmdl`: lossless TMDL lexer → CST → model, and a writer; byte-identical round-trip on every fixture `.tmdl`; multi-line M and DAX, `///` descriptions, `ref`, `meta`.
- [x] `m1-tmsl`: TMSL (`model.bim`, PBIT `DataModelSchema` UTF-16LE) ↔ model, with unknown-field preservation.
- [x] `m1-pbir`: PBIR serde types with `extra` for unknown fields; semantic-query expression types; JSON value-equality round-trip on every fixture JSON; PBIR-Legacy and PBIX `Layout` readers; optional validation against vendored Microsoft schemas.
- [x] `m1-containers`: `Container` trait (folder, ZIP) with zip-slip and symlink guards and caps on size, entry count and compression ratio; content-based detection for every supported format; tests with malicious ZIPs.
- [x] `m1-core`: content-based detection; safe containers (caps on size, entry count and compression ratio; zip-slip guard); readers for PBIP (`byPath`, and `byConnection` shown as remote), `.platform`, `.pbism`, `.pbi/localSettings.json`, `diagramLayout.json`, PBIT, PBIX phase 1a (listing, report, theme, `Version`), `.pbids`, `.bim`, standalone TMDL folder. Done in `pbi-core::reader` (`open_path`/`open_bytes` → `OpenedDocument`), plus PBIX/`.abf` schema through `pbi-vertipaq`, secret redaction, and the save groundwork `save_as_pbip` / `export_pbit` (the "PBIT exported from them" acceptance fixture is produced in `crates/pbi-core/tests/readers.rs`).
- [x] `m1-cli`: `dashcraft inspect <path> [--json]` lists every table, column, measure, relationship, page and visual; `dashcraft open <path>`; `insta` snapshot tests over all fixtures.
- [x] `m1-app-scaffold`: `app/` Tauri 2 + Svelte 5 + TypeScript + Vite; `pbiview-app` in workspace members (not default-members); `tauri-specta` typed IPC; dialog plugin; drag-drop and CLI-argument plumbing; three-pane layout shell.
- [x] `m1-app`: open by dialog, drag-drop or CLI; model tree; report outline; properties pane (display name, with the source name in a tooltip); `byConnection` shown as remote.

## M2: Engines, grid, exports, linter → `v0.1.0`

- [x] `m2-m-parser`: full M grammar (`let`/`in`, `each`, typed function literals, records, lists, field and item access including `{[Name=…]}`, `??`, `meta`, `#table`, `#"quoted"` identifiers, operators, comments); spans and error recovery.
- [x] `m2-m-eval`: lazy evaluator and value model (Arrow-backed tables); the stdlib subset used by the fixtures (`Text.*`, `List.*`, `Table.*`, `Csv.Document`, `AzureStorage.Blobs` through a local `BlobResolver`, `DateTimeZone.*`); parameters; clear unsupported-function errors with a "Load CSV manually…" fallback.
- [x] `m2-dax-parser`: full DAX grammar (`VAR`/`RETURN`, table constructors, operators and precedence, quoted table refs, column and measure refs); spans and errors.
- [x] `m2-engine`: Arrow columnar store; relationship index (single and both directions); filter-context propagation; DataFusion registration; async + cancellation API.
- [x] `m2-dax-eval`: filter context, row context, context transition, `BLANK` semantics; `IF`, `ISEMPTY`, `RELATEDTABLE`, `COUNTROWS`, `SELECTCOLUMNS`, `FILTER`, `SUMMARIZE`, `ALL`, `EXCEPT`, `DISTINCTCOUNTNOBLANK`, `CALCULATE`, `DIVIDE`, `MAX`; unsupported functions produce a warning with the DAX text and never panic.
- [x] `m2-refresh`: `pbi-session` refresh in dependency order (M partitions → calculated columns → calculated tables). Acceptance: the example's row counts and measure values ([acceptance values](#acceptance-values)).
- [x] `m2-grid`: virtualized, sortable grid with column stats; backend paging over Arrow.
- [x] `m2-export`: table → CSV/Parquet; model → JSON/Markdown (including `///` descriptions); `dashcraft export`.
- [x] `m2-lint`: broken-reference linter (projections, `queryRef`, filters, `sortDefinition` vs model); `dashcraft lint` exits non-zero on errors.
- [x] `m2-people`: "parse people field" transform for Python-repr lists of names (`'` or `"` quoting).
- [x] `m2-release`: all of M2 green on CI; README and FORMATS updated; tag `v0.1.0`.

## M3: PBIX data (VertiPaq), hardening

- [x] `m3-xpress9`: clean-room XPress9 decoder ([ADR-0003](adr/0003-vertipaq-xpress9-cleanroom.md)); known-good blob tests (synthetic vectors and a real PBIX `DataModel`).
- [x] `m3-abf`: ABF header, backup log and virtual directory; extract `metadata.sqlitedb` (`rusqlite`, bundled); schema → model.
- [x] `m3-vertipaq`: IDF bit-pack/RLE segments, dictionaries (including compressed string dictionaries), HIDX → Arrow; cross-checked against independently produced tables (pbixray output) by hash.
- [x] `m3-pbix-data`: PBIX `DataModel` → engine; `.abf`; PBIP `.pbi/cache.abf`; `.vpax` stats; Power Pivot `.xlsx` (stretch).
- [x] `m3-fuzz`: `cargo-fuzz` targets (TMDL, PBIR, M, DAX, ZIP container, XPress9, ABF); scheduled nightly CI job.

## M4: Report renderer

- [x] `m4-theme-layout`: base and custom themes, `ThemeDataColor` ColorId/Percent, FitToPage/FitToWidth, z-order and tab order, `visualContainerObjects`; `cosmic-text` with bundled Selawik.
- [x] `m4-semantic-query`: `queryState`, report/page/visual filters, slicer state and cross-filter → DAX → PBI-shaped DataView; report filters skip visuals with no relationship path. (`pbi_render::query`, `pbi_render::dataview`; role mapping, filter matrix and gaps in the `query` module docs.)
- [x] `m4-dax-expand`: every function the acceptance criteria name (`SUM`, `COUNTROWS`, `DISTINCTCOUNT`, `CALCULATE`, `DIVIDE`, `FILTER`, `RELATEDTABLE`, `COUNTBLANK`, `IF`, `FORMAT`, `MAX`, `ISBLANK`, `ALL`, `EXCEPT`), each with a unit test; `FORMAT` strings from `formatString`.
- [x] `m4-visuals`: textbox, shape, actionButton, cardVisual and card, slicer, tableEx, pivotTable, pie/donut, clustered bar/column, line, and a placeholder → scene graph → SVG; cardVisual inner-border and label-clipping quirks. Split into:
  - [x] `m4-visuals-basics`: textbox, shapes, buttons and navigators, cards (new, legacy, multi-row, KPI), error and placeholder states, page orchestration.
  - [x] `m4-visuals-grids`: slicers (dropdown, list, tiles, range) with the popup API, table, matrix.
  - [x] `m4-visuals-charts`: pie/donut, clustered/stacked/100% bar and column, line/area, combos; axes, legends, labels, scrolling, hit ids.
- [x] `m4-png`: `dashcraft render <file> --page <name> --png out.png` through `resvg`; optional golden SSIM against local service exports in `reference/golden/` (per-region SSIM ≥ 0.85; [VISUALS.md](VISUALS.md#golden-image-comparison)); render snapshots and card values asserted on the rendered example pages.
- [x] `m4-derived-columns`: `pbi-session` derived-column API (people parsing, DAX expressions, arrays) visible to DAX, SQL, the grid and exports, re-applied after refresh; the app uses it.
- [x] `m4-desktop-shell`: app shell matching Power BI Desktop's layout, ribbon, view rail (Report, Table, Model, DAX query, TMDL), panes and status bar, read-only; Fluent UI icons and Selawik.
- [x] `m4-interactive`: app canvas with SVG display, hit-testing, slicers, `visualInteractions` cross-filter, filter pane hidden/locked flags, page navigation, action buttons ([ADR-0006](adr/0006-interaction-state.md)).

## M5: Polish and ship

- [x] `m5-formats-ext`: `.rdl` outline, `.pq`/`.pqt`, dataflow `model.json`, `.pbiviz` metadata (never executed), `.mez`/`.pqx` listing.
- [x] `m5-csv-safe`: opt-in CSV formula-injection guard (`--excel-safe`, `CsvExportOptions::escape_formulas`).
- [x] `m5-packaging`: Tauri bundles (dmg/msi/AppImage) and a release workflow. Signing is blocked until certificates and secrets are provided.
- [x] `m5-docs`: final README (format notes, supported-visuals matrix, DAX and M coverage), `FORMATS.md`, [CLI.md](CLI.md), [USER-GUIDE.md](USER-GUIDE.md), [VISUALS.md](VISUALS.md), [ARCHITECTURE.md](ARCHITECTURE.md), changelog for `v0.2.0`.

## After v0.2.0 (unreleased)

- [x] `perf-budgets`: `cargo xtask perf` (P01–P08) and `cargo xtask layers`; all eight scenarios within target after the render cache and the DAX fast paths ([perf.md](perf.md)).
- [x] `ops-registry`: command registry `pbi-ops` ([ADR-0007](adr/0007-command-registry.md)) with `commands`, `run`, `serve`.
- [x] `mcp`: `dashcraft mcp`, the agent-safe commands as MCP tools ([MCP.md](MCP.md)).
- [x] `ai-layer`: opt-in `pbi-ai` ([ADR-0008](adr/0008-ai-layer.md), [AI.md](AI.md)) and its CLI wiring.
- [x] `app-on-registry`: the app runs every domain operation through the registry.
- [x] `rebrand-dashcraft`: product name DashCraft, `dashcraft` binary with a `pbiview` alias, `DASHCRAFT_*` variables, identifier `io.github.jxck-s.dashcraft`, `NOTICE` ([ADR-0009](adr/0009-naming-dashcraft.md)).
- [x] `visuals-more`: `scatterChart`, `funnel`, `gauge`, `treemap` and `waterfallChart` instead of placeholders, with hit ids and tooltips ([VISUALS.md](VISUALS.md)).
- [x] `engine-model-features`: calculation groups, many-to-many (limited) relationships and row-level security "View as" in `pbi-session`; the `security.roles` / `security.viewAs` commands and the app's Modeling → View as dialog.
- [x] `dax-library`: 218 → 294 DAX functions ([FUNCTIONS.md](../crates/pbi-dax/FUNCTIONS.md)).
- [x] `ai-in-app`: Options → AI, the consent dialog, Explain with AI and Suggest a measure in the desktop app ([AI.md](AI.md#in-the-desktop-app)).
- [x] `conditional-formatting`: conditional formatting of tables, matrices and chart colors, and dynamic format strings per cell ([VISUALS.md](VISUALS.md#conditional-formatting)).

## Next

Not scheduled; in rough priority order.

- [ ] `m4-windows-groundtruth`: a ground-truth session on a Windows machine with Power BI Desktop (**blocked** until one is available). Black-box only: no decompiling or debugging of Desktop binaries; screenshots and icons are never committed.
  - UI Automation dump of the ribbon tabs, groups, buttons, tooltips and positions, to check the Desktop-style shell.
  - DAX differential testing: random and curated queries over the example model, compared between `pbi-dax` and Desktop's local Analysis Services instance.
  - More golden images: Desktop-rendered pages of the example's reports and of the visual types that only have synthetic tests.
  - More Desktop-saved fixtures (synthetic content only): a PBIR-Legacy PBIP, a real `.pbi/cache.abf`, a Power Pivot workbook, a DAX Studio VPAX, a Report Builder RDL.
  - Desktop-canonical TMDL and PBIR output to validate the writers byte for byte.
- [ ] Editing and saving PBIP: the command layer (`pbi-model::command`, undo/redo, change events) and the PBIP writers exist; next are editing commands in the registry (`pbi-ops`: set a measure's expression, rename, move a visual, save), then editors in the UI wired to them, DAX/M-aware rename rewriting, and Save.
- [ ] Highlight interactions (`HighlightFilter` highlighting instead of filtering; highlight values in DataViews).
- [ ] Bookmarks and drillthrough (read `bookmarks/*.json`, apply bookmark state, drillthrough pages and filters); report-page tooltips.
- [ ] Pie and donut slice separators: adjacent slices with the same color (a conditional color, or repeated `dataPoint` colors) merge into one wedge; draw the service's thin separators between slices.
- [ ] Conditional formats of column headers, totals, titles, card callouts, gauges and KPIs (reported "not applied" today, [VISUALS.md](VISUALS.md#conditional-formatting)).
- [ ] Per-legend conditional chart colors (a measure colors each legend value) and conditional colors of line, area, combo, waterfall and gauge charts.
- [ ] Expression-based titles and other `fx` text properties (titles, subtitles, button text from a measure).
- [ ] Synced slicers (`syncGroup`) and draggable range / relative-date slicers.
- [ ] DIN- and Calibri-metric fonts under a permissive license, to close the remaining golden-image gaps.
- [ ] Report-level measures (`reportExtensions.json`), `image` visuals and more visual types (maps, `ribbonChart`, AI visuals); scatter play-axis animation.
- [ ] Object-level security; RLS role filters that read other secured tables.
- [ ] Fabric remote mode for `byConnection` reports (optional; device-code auth, credentials in the OS keychain).
- Out of scope: DirectQuery live execution; publishing to the Power BI service in v1; writing PBIX; executing custom visuals, R/Python visuals or custom connectors.

## Acceptance values

The acceptance values (row counts after the M steps, card and measure values per report page under
its filters and slicers, calculation-group, RLS and "no relationship path" cases) are those of the
synthetic Acme Outfitters example, computed independently of DashCraft's engines:
[EXPECTED.md](../reference/examples/acme-outfitters/EXPECTED.md) and `expected.json` in
`reference/examples/acme-outfitters/`.

## Open items needing a human decision

- Legal review of the UNVERIFIED rows in [ADR-0003](adr/0003-vertipaq-xpress9-cleanroom.md) before a release ships PBIX data decoding.
- Code-signing material (Apple Developer ID, Windows code-signing certificate) for `m5-packaging`.
- Project license (`LicenseRef-Proprietary` in `Cargo.toml`) and the real repository URL (currently a placeholder).
- A trademark search for the name DashCraft before the first public release ([ADR-0009](adr/0009-naming-dashcraft.md): other products already use the name; the decision is reversible).
