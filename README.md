# DashCraft

**DashCraft** (formerly pbiview) is a clean-room, cross-platform (macOS, Windows, Linux) desktop app and command-line tool (`dashcraft`) that opens Power BI files **offline**: no Power BI Desktop, no Windows VM, no Fabric login. It reads PBIP, PBIX, PBIT and related formats, loads their data, evaluates DAX and Power Query M with its own engines, renders report pages with filters, slicers and cross-filtering, and exposes every capability as a typed command for scripts and AI agents. Today it is a read-only viewer; the architecture is built to grow into a Rust reimplementation of Power BI Desktop ([ADR-0004](docs/adr/0004-desktop-ready-architecture.md)).

DashCraft is an independent project, not affiliated with, endorsed or sponsored by Microsoft; "Power BI" and "Microsoft" are Microsoft trademarks, used only to say which files it opens. It is not affiliated with storytold/ArtCraft or with other products named "DashCraft". See [NOTICE](NOTICE) and [ADR-0009](docs/adr/0009-naming-dashcraft.md) (the name).

## Status

| Release | Scope | State |
|---|---|---|
| v0.1.0 (2026-10-21) | Phase 1, Data Explorer: readers, model tree, M and DAX engines, data grid, exports, linter, CLI | Released |
| v0.2.0 (2026-10-22) | Phase 2, Report Renderer: visual queries, built-in visuals, SVG/PNG output, `render`, golden-image tooling, interactive canvas in a Power BI Desktop-style app. M5: the remaining [FORMATS.md](FORMATS.md) formats, docs, installers and the release workflow ([docs/packaging.md](docs/packaging.md)) | Released; installers are unsigned until signing certificates exist |
| Unreleased | The work below, in `main` ([CHANGELOG.md](CHANGELOG.md) `[Unreleased]`, [docs/ROADMAP.md](docs/ROADMAP.md)) | Not tagged yet |

### Since v0.2.0 (unreleased, in `main`)

| Theme | What landed |
|---|---|
| Command registry | `pbi-ops` ([ADR-0007](docs/adr/0007-command-registry.md)): 47 typed commands (all agent-safe) plus 9 opt-in `ai.*` commands, with JSON Schemas for parameters and results, a workspace of open documents, cancellation, progress events and a path policy; catalog in [docs/COMMANDS.md](docs/COMMANDS.md) |
| Automation | `dashcraft commands`, `run` (one command or a `--batch` file), `serve --stdio` (JSON-RPC 2.0 lines) and `mcp`, a Model Context Protocol server that offers the agent-safe commands as tools inside read/write roots ([docs/MCP.md](docs/MCP.md)) |
| Opt-in AI | `pbi-ai` ([ADR-0008](docs/adr/0008-ai-layer.md), [docs/AI.md](docs/AI.md)), off by default: explain a measure, and suggest a measure (parsed and optionally evaluated locally, never applied), through OpenAI-compatible, Azure OpenAI or Ollama providers, only after a recorded consent per provider, capability and data class; `ai.preview` shows the exact request; keys only in the OS keychain (or `DASHCRAFT_AI_API_KEY`). CLI: `ai login`/`logout`. App: Options → AI, a consent dialog showing the exact request, **Explain with AI**, **Suggest a measure** |
| App on the registry | The desktop app runs every domain operation through one `ops_execute` IPC command → `Registry::execute`, the same commands as `run`, `serve` and MCP, with a TypeScript client generated from the schemas; File → Export writes a PBIP project or PBIT template |
| Performance | `cargo xtask perf`: 8 scenarios with budgets and a per-machine baseline, all within target ([Performance](#performance)). A per-visual render cache and DAX fast paths took a 50,000-row table scroll step from 25.6 ms to under 1 ms and its cold first render from 13.6 s to about 0.1 s |
| Name | DashCraft ([ADR-0009](docs/adr/0009-naming-dashcraft.md)): `dashcraft` binary with a `pbiview` alias, app `dashcraft-app` (identifier `io.github.jxck-s.dashcraft`), `DASHCRAFT_*` variables with `PBIVIEW_*` aliases, [NOTICE](NOTICE) |
| Visuals | `scatterChart`, `funnel`, `gauge`, `treemap` and `waterfallChart` replace their placeholders; conditional formatting of tables, matrices and chart colors; dynamic format strings per cell |
| Engine | Calculation groups; many-to-many (limited) relationships; row-level security "View as" (roles and user) with the `security.roles` / `security.viewAs` commands and the app's Modeling → View as dialog |
| DAX | Library grown from 218 to 294 functions (logarithms, trigonometry, statistical distributions, `YEARFRAC`, `NETWORKDAYS`, bit functions, …) |
| Fixes | `kpi` uses `TrendLine` as its trend axis; matrix levels follow sort-by columns; one-to-one relationships relate unmatched rows to the blank row; TMDL ↔ TMSL conversion keeps dynamic format strings; the people transform accepts plain name lists |
| Fixtures | The synthetic Acme Outfitters example (PBIP, CSVs, blob map, expected values computed independently in Python) and MIT-licensed Microsoft sample reports replace the previous fixtures; render snapshots of every example page ([Test data policy](#test-data-policy)) |
| Repository guards | `.gitignore` refuses `*.pbix`, `*.pbit` and `*.abf` outside the committed samples and fuzz seeds; `cargo xtask layers` enforces the crate layers in CI |

### Where it is going

The goal is a full Rust reimplementation of Power BI Desktop: report and model editing, Power Query, Save, undo. The groundwork is in place (lossless TMDL, TMSL and PBIR writers, PBIP and PBIT export, `Document` commands with undo, a Desktop-shaped shell whose editing commands are shown disabled); the milestones are in [Roadmap to Power BI Desktop](#roadmap-to-power-bi-desktop). [docs/GAPS.md](docs/GAPS.md) scores each area 0–5 against Power BI Desktop:

| Area | Avg score | Biggest gap |
|---|---|---|
| File formats | 3.1 | Bookmarks, report-level measures, images, `DataMashup`; Fabric remote mode |
| M language and library | 3.1 | Library breadth, cultures, other data sources |
| DAX | 3.3 | No differential testing; financial, `INFO.*` and visual-calculation functions |
| Engine features | 3.4 | Object-level security, self-referencing relationships, incremental refresh, drill-down |
| Visuals | 2.1 | No visual checked against a Power BI render yet; image, maps, ribbon chart, small multiples |
| Interactions | 1.6 | Highlighting, bookmarks, drillthrough, drill-down, synced slicers |
| Rendering fidelity | 3.2 | No golden images (Power BI exports of the example pages); DIN font |
| Performance | 4.9 | No app-open, end-to-end refresh or webview canvas-latency scenarios |
| Command registry, MCP, AI | 2.7 | Editing commands, CLI subcommands as registry wrappers, parameter fuzzing |
| App UX vs Power BI Desktop | 1.4 | Every editing surface: report, modeling, Power Query, Save, undo |
| Platform and packaging | 2.7 | Signing and notarization, auto-update, ARM64 Windows/Linux |
| Security and fuzzing | 3.1 | Legal review (ADR-0003), registry parameter fuzzing, staged-file scanning in CI |

## Roadmap to Power BI Desktop

The milestone plan (M6 onward) is in [docs/DESKTOP-PLAN.md](docs/DESKTOP-PLAN.md).

The plan has 26 milestones, M6 to M31, ordered so every milestone ships. M27 to M30 are optional and not needed for 1.0; several steps are blocked on ground-truth sessions with Power BI Desktop on Windows, signing certificates or a legal review (see the plan for dependencies, acceptance criteria, the 18 proposed ADRs and the open questions).

| Milestone | Name | Size | Optional |
|---|---|---|---|
| [M6](docs/DESKTOP-PLAN.md#m6-editable-document-state-and-save) | Editable document state and save | XL |  |
| [M7](docs/DESKTOP-PLAN.md#m7-language-foundation) | Language foundation | L |  |
| [M8](docs/DESKTOP-PLAN.md#m8-recovery-external-changes-and-semantic-diff) | Recovery, external changes and semantic diff | M |  |
| [M9](docs/DESKTOP-PLAN.md#m9-reading-view-parity) | Reading view parity | XL |  |
| [M10](docs/DESKTOP-PLAN.md#m10-modeling-core) | Modeling core | XL |  |
| [M11](docs/DESKTOP-PLAN.md#m11-advanced-modeling-and-security) | Advanced modeling and security | L |  |
| [M12](docs/DESKTOP-PLAN.md#m12-dax-and-tmdl-authoring-views) | DAX and TMDL authoring views | M |  |
| [M13](docs/DESKTOP-PLAN.md#m13-dax-engine-parity) | DAX engine parity | L |  |
| [M14](docs/DESKTOP-PLAN.md#m14-power-query-editor-core) | Power Query Editor core | L |  |
| [M15](docs/DESKTOP-PLAN.md#m15-power-query-transform-parity) | Power Query transform parity | L |  |
| [M16](docs/DESKTOP-PLAN.md#m16-get-data-local-connectors-and-m-library) | Get Data, local connectors and M library | L |  |
| [M17](docs/DESKTOP-PLAN.md#m17-data-engine-at-scale) | Data engine at scale | XL |  |
| [M18](docs/DESKTOP-PLAN.md#m18-report-authoring-canvas-pages-and-pbir-creation) | Report authoring: canvas, pages and PBIR creation | L |  |
| [M19](docs/DESKTOP-PLAN.md#m19-format-pane-and-visual-property-descriptors) | Format pane and visual property descriptors | XL |  |
| [M20](docs/DESKTOP-PLAN.md#m20-report-authoring-filters-interactions-bookmarks-themes-mobile) | Report authoring: filters, interactions, bookmarks, themes, mobile | L |  |
| [M21](docs/DESKTOP-PLAN.md#m21-standard-visuals-completeness) | Standard visuals completeness | L |  |
| [M22](docs/DESKTOP-PLAN.md#m22-maps-and-geodata) | Maps and geodata | M |  |
| [M23](docs/DESKTOP-PLAN.md#m23-statistical-visuals-qa-and-insights) | Statistical visuals, Q&A and insights | L |  |
| [M24](docs/DESKTOP-PLAN.md#m24-desktop-shell-ribbon-settings-performance-analyzer-and-documents) | Desktop shell: ribbon, settings, Performance analyzer and documents | L |  |
| [M25](docs/DESKTOP-PLAN.md#m25-accessibility-and-localization) | Accessibility and localization | L |  |
| [M26](docs/DESKTOP-PLAN.md#m26-file-formats-and-desktop-interoperability) | File formats and Desktop interoperability | L |  |
| [M27](docs/DESKTOP-PLAN.md#m27-pbix-writing-optional) | PBIX writing | XL | yes |
| [M28](docs/DESKTOP-PLAN.md#m28-ai-assisted-editing-optional) | AI-assisted editing | L | yes |
| [M29](docs/DESKTOP-PLAN.md#m29-custom-visuals-sandbox-optional) | Custom visuals sandbox | XL | yes |
| [M30](docs/DESKTOP-PLAN.md#m30-online-integration-optional) | Online integration | XL | yes |
| [M31](docs/DESKTOP-PLAN.md#m31-verification-and-10) | Verification and 1.0 | L |  |

## Acceptance criteria

| # | Criterion | Result |
|---|---|---|
| 1 | Opens PBIX, PBIP (one semantic model with several `*.Report` folders) and PBIT files on macOS, Windows and Linux | Met: tests on the CI matrix (macOS arm64, Windows x64, Ubuntu) with the synthetic example and the public Microsoft samples |
| 2 | `inspect --json` lists every object; CSV-backed partitions load with the expected row counts | Met: `insta` snapshots; `pbi-session` acceptance tests against the example's expected values |
| 3 | `render --png` within SSIM tolerance of Power BI exports; card values match the expected values | Card values met: all 39 card values of the example's `expected.json` are asserted on the rendered pages. SSIM not verified yet: the per-region comparison (threshold 0.85, no exceptions) runs only against optional local golden images, and none are committed; render snapshots of every example page catch regressions ([details](docs/VISUALS.md#render-verification)) |
| 4 | TMDL and PBIR round-trips, XPress9 known-good blob, one test per DAX function | Met |
| 5 | CI matrix with fmt, clippy `-D warnings`, tests; signed release artifacts | CI met ([docs/ci.md](docs/ci.md)); release artifacts are unsigned until signing certificates exist |
| 6 | No script execution, untrusted-input hardening, fuzzing, no telemetry | Met (14 `cargo-fuzz` targets, nightly) |

## Quick start

Prerequisites: the Rust toolchain pinned in `rust-toolchain.toml` (installed automatically by `rustup`). The app also needs Node 20.19+ or 22.12+ and npm, plus WebKitGTK on Linux ([app/README.md](app/README.md#linux-system-dependencies)).

```sh
# CLI
cargo build --release -p pbiview-cli              # target/release/dashcraft and its pbiview alias
target/release/dashcraft inspect path/to/report.pbip
target/release/dashcraft render path/to/report.pbix --png target/tmp/page.png

# The synthetic example in this repository
E=reference/examples/acme-outfitters
target/release/dashcraft refresh  $E/AcmeOutfitters.pbip --blob-map $E/blob-map.json
target/release/dashcraft measures $E/AcmeOutfitters.pbip --blob-map $E/blob-map.json --report Stores
target/release/dashcraft render   $E/AcmeOutfitters.pbip --blob-map $E/blob-map.json \
  --report Overview --all-pages --out-dir target/tmp/acme
target/release/dashcraft run security.viewAs --doc $E/AcmeOutfitters.pbip \
  --blob-map $E/blob-map.json -p roles='["Region West"]'
python3 scripts/examples/check_acme.py            # compare DashCraft with the expected values

# Desktop app
cd app && npm install
npm run tauri dev -- -- path/to/report.pbix       # development build with a file open
npm run tauri -- build                            # release binary and installers for this OS
```

`dashcraft open <path>` starts the installed app on a file. `pbiview` remains an alias of the `dashcraft` binary and the former `PBIVIEW_*` environment variables still work. Screenshots are not committed; open the example in the app to see it. Recipes: [docs/USER-GUIDE.md](docs/USER-GUIDE.md); every flag and JSON schema: [docs/CLI.md](docs/CLI.md).

## Features

**Data Explorer**

- Content-based format detection; safe readers for PBIP (TMDL or `model.bim`, PBIR or PBIR-Legacy), PBIX (PBIR or legacy `Layout`, embedded VertiPaq data), PBIT and the other formats below.
- Model tree: tables, columns (display and source names), measures, calculated columns and tables, relationships, M partitions and parameters, hierarchies, roles, calculation groups, `///` descriptions.
- Data offline: M partitions run in DashCraft's own M engine (`AzureStorage.Blobs` through a local blob map); PBIX, ABF and Power Pivot data is decoded from VertiPaq; a PBIP's `.pbi/cache.abf` loads without M; "Load CSV manually…" as a fallback.
- DAX engine for measures, calculated columns and tables, and `EVALUATE` queries, with calculation groups, many-to-many relationships and row-level security ("View as" roles and a user).
- Virtualized data grid with sort, SQL filter and column statistics; session-derived columns ("Parse people field", DAX expressions).
- Exports: table to CSV (optional formula-injection guard) or Parquet; model to JSON or a Markdown data dictionary. Conversion to a PBIP project or PBIT template.
- Broken-reference linter ([17 rules](crates/pbi-core/src/lint/RULES.md)), including stale `queryRef`s.

**Report Renderer**

- Each visual's `queryState`, report/page/visual filters, slicer selections and cross-filter selections become a DAX `SUMMARIZECOLUMNS` query; results are Power BI-shaped DataViews.
- Built-in Rust visuals draw into a scene graph, serialized to deterministic SVG (app) and PNG (CLI, tests) with bundled fonts only (Selawik for Segoe UI, Carlito for Calibri).
- Themes (base and custom, `ThemeDataColor`, `visualStyles`, text classes), page display options, z-order, groups and container chrome; conditional formatting and dynamic format strings.
- Service rendering quirks emulated ([docs/VISUALS.md](docs/VISUALS.md)); verified by render snapshots and expected values of the example, and by SSIM only where optional local golden images exist.

**Command registry, automation and MCP**

- Every capability is a registry command with a dotted id (`doc.open`, `measures.values`, `dax.query`, `render.page`, `canvas.click`, `security.viewAs`, `export.table`, …), typed parameters and results with JSON Schemas, and metadata (agent-safe, mutates the document, long-running, writes files).
- Front ends are thin adapters: `dashcraft run` (and `--batch` with `${step.field}` references), `dashcraft serve` (JSON-RPC 2.0 over stdio with progress and `$/cancel`), `dashcraft mcp` and the app.
- `dashcraft mcp` serves the agent-safe commands as MCP tools (PNG renders as images), with read/write roots, `--allow`/`--deny` filters and protocol revisions 2024-11-05 to 2025-11-25 plus the stateless 2026-07-28 ([docs/MCP.md](docs/MCP.md)).

**AI (opt-in)**

- Off by default; with AI off DashCraft makes no network requests at all ([docs/AI.md](docs/AI.md)).
- `ai.explainMeasure` and `ai.generateMeasure` (suggestions are parsed, checked against the model and optionally evaluated locally, never written to it); `ai.preview` shows the exact request with keys redacted; consents per provider, capability and data class (model metadata by default, never whole tables).
- Providers: OpenAI-compatible, Azure OpenAI, Ollama (local). Keys live only in the OS keychain (`dashcraft ai login`) or `DASHCRAFT_AI_API_KEY`; settings files never hold keys.

**Desktop app** ([app/README.md](app/README.md))

- Power BI Desktop's layout: ribbon, view rail (Report, Table, Model, DAX query, TMDL), Filters / Visualizations / Data / Selection / Properties panes, page tabs, status bar, recent files. Editing commands are shown disabled.
- Interactive report canvas: slicers (single and multi-select, dropdowns, search), cross-filtering per `visualInteractions`, tooltips, table and matrix scrolling, matrix expand/collapse, Back and page-navigation buttons.
- DAX query view, read-only Transform data editor, Model view diagram, local data cache banner with Refresh from source, Modeling → View as, File → Export (PBIP or PBIT), Options → AI.
- Open by dialog, drag and drop, command line or file association (`.pbix`, `.pbit`, `.pbip`, `.bim`, `.vpax`).

## Supported formats

Detection uses content (ZIP entries, JSON keys, `$schema`, encodings), not just the extension. Full matrix with detection signals, readers, fixtures and gaps: [FORMATS.md](FORMATS.md).

| Format | Model | Data | Report rendering |
|---|---|---|---|
| PBIP project (TMDL or `model.bim`; PBIR or PBIR-Legacy) | Full | M refresh (offline subset), or `.pbi/cache.abf` | Full (`byPath`); `byConnection` shown as remote |
| PBIX | Full (from `DataModel`) | Embedded VertiPaq data | Full (PBIR or legacy `Layout`) |
| PBIT | Full (`DataModelSchema`) | M refresh | Full |
| Standalone `.bim`, TMDL folder | Full | M refresh | n/a |
| `.abf` backup | Full | Embedded VertiPaq data | n/a |
| Excel workbook with Power Pivot (`.xlsx`/`.xlsm`) | Full | Embedded VertiPaq data | n/a |
| `.vpax` (VertiPaq Analyzer) | Model and size/cardinality statistics | None | n/a |
| `.pq`/`.m`, `.pqt`, dataflow `model.json`, Dataflow Gen2 | M queries (CDM entities as tables) | M refresh where sources are offline-capable | n/a |
| `.rdl` paginated report | Outline (data sources, datasets, parameters, items) | None | Not rendered |
| `.pbids` | Connection details | None | n/a |
| `.pbiviz`, `.mez`/`.pqx` | Metadata or listing only, **never executed** | None | `.pbiviz` visuals: placeholder |

## Supported visuals

Levels: **Supported** (called Full in VISUALS.md): the common formatting objects are honoured and the layout follows the service; **Partial**: the common options render, notable gaps listed; **Placeholder**: a box with the visual type and its bound fields (the query still runs). Details and differences from the service: [docs/VISUALS.md](docs/VISUALS.md).

| Visual type | Level | Notable options and limits | Interaction (app) |
|---|---|---|---|
| `textbox` | Supported | Text runs (font, size, weight, style, decoration, color, links), alignment, lists, scrollbar on overflow | None |
| `shape`, `basicShape` | Supported | 22 shape kinds, fill, outline, rotation, text | None |
| `actionButton` | Supported | Shape, vector icon, text; actions Back, page navigation, bookmark, drillthrough, Web URL, Q&A, apply/clear all slicers | Back, page navigation and Clear all slicers run; Web URL shown with Copy link, never opened; bookmark, drillthrough, Q&A "not available" |
| `pageNavigator`, `bookmarkNavigator` | Partial | Drawn as one labelled button (no page or bookmark list) | None |
| `cardVisual` (new card) | Supported | Tiles grid, value/label styles, inner tile border, label clipping below ~100 px, reference labels, accent bar | None |
| `card`, `multiRowCard` | Supported | Legacy card value and category label; multi-row card blocks with accent bar | None |
| `kpi` | Partial | Indicator, goal and status color, trend area on the `TrendLine` axis | None |
| `slicer` | Supported (list, dropdown, tile); Partial (range, relative date) | Header, search box, Select all, single/multi-select, scrollbar | Select, Ctrl+click, Select all, dropdown, search, scroll, clear; range sliders not interactive |
| `advancedSlicerVisual` (button slicer) | Partial | Drawn as tiles | Tile selection |
| `tableEx`, `table` | Supported | Auto-sized columns, word wrap, banding, grid lines, totals, per-column formatting, sort arrows; conditional formatting | Row click cross-filters; scroll; tooltips |
| `pivotTable`, `matrix` | Supported | Stepped or tabular rows, subtotals, grand total, column hierarchies, `+`/`−` buttons, sort-by columns; conditional formatting | Row click cross-filters (row path); expand/collapse; scroll; tooltips |
| `pieChart`, `donutChart` | Supported | Leader-line labels with the service's placement and truncation, legend, `dataPoint` colors, inner radius | Slice click cross-filters; tooltips |
| Bar and column charts (clustered, stacked, 100% stacked) | Supported | Axes with nice ticks, gridlines, titles, data labels, legend, category scrolling | Bar click cross-filters; tooltips |
| `lineChart`, `areaChart`, `stackedAreaChart` | Supported | Categorical or continuous (date/number) axis, markers, line styles | Point click cross-filters; tooltips |
| `lineClusteredColumnComboChart`, `lineStackedColumnComboChart` | Supported | Columns on `Y`, lines on `Y2` with a secondary axis | Bar and point clicks cross-filter |
| `scatterChart` | Partial | Points per Details × Legend value, numeric axes, bubbles sized by `Size`, marker shapes, labels; first play-axis frame only, no Analytics pane | Point click cross-filters; tooltips (Details, Legend, X, Y, Size) |
| `funnel` | Supported | Centred bars in query order, data labels, conversion-rate labels | Bar click cross-filters; tooltips |
| `gauge` | Supported | 180° arc, minimum/maximum (default maximum = 2 × value), target needle, callout value | Tooltip |
| `treemap` | Supported | Squarified groups and details, theme colors, labels | Tile click cross-filters; tooltips |
| `waterfallChart` | Supported | Increase/decrease/total colors, connectors, total column, breakdown top N + Other | Column click cross-filters; tooltips |
| Anything else (`image`, maps, `ribbonChart`, AI visuals, R/Python, custom `.pbiviz`, …) | Placeholder | Visual type and bound fields | None |

**Conditional formatting** (tables, matrices, and the colors of bar, column, pie, donut, treemap, scatter and funnel charts without a legend field): color scales with 2 or 3 colors, rules, field values, data bars and built-in icons. Unsupported variants (headers, titles, cards, per-legend chart colors, URL icons, …) render unformatted with a warning ([details](docs/VISUALS.md#conditional-formatting)). Measures with dynamic format strings (calculation items, `formatStringDefinition`) format each value with its own format string. A visual whose query fails draws "Can't display this visual" with a warning icon; the app's tooltip shows the message and the failing measure's DAX. Rendering never panics.

## DAX and M coverage

- **DAX** (`pbi-dax`): 294 functions in twelve families (logical and information, math and statistics, statistical distributions, text, date and time, time intelligence, aggregation and iterators, filter and context, table manipulation, window functions, parent-child, calculation groups and security); 271 supported, 23 partial. `DEFINE`/`EVALUATE` queries, calculated columns and tables, `FORMAT` strings (en-US), `CALCULATE` modifiers, `ALLSELECTED`, `USERELATIONSHIP`/`CROSSFILTER`, `SUMMARIZECOLUMNS` with rollups, calculation groups (precedence, selection expressions, dynamic format strings), many-to-many (limited) relationships, row-level security (`USERPRINCIPALNAME`, `CUSTOMDATA`). Not supported: financial, `INFO.*` and visual-calculation functions, object-level security, DirectQuery. An unsupported function returns a typed error, shown with a warning and the DAX text. List and semantic gaps: [crates/pbi-dax/FUNCTIONS.md](crates/pbi-dax/FUNCTIONS.md).
- **Power Query M** (`pbi-m`): full grammar; lazy evaluator with closures, `each`, `try`/`otherwise`, `meta`, sections. 349 library names (`pbi_m::library_names()`): 261 functions (`Table.*` 45, `List.*` 41, `Text.*` 36, `Number.*` 20, `Date.*` 17, `Record.*` 15, `DateTimeZone.*` 13, `Duration.*` 11, `Value.*` 10, `DateTime.*` 8, `Time.*` 6, `Csv.Document`, `Json.Document`, …), 32 `*.Type` values and 56 constants, plus the `#date`/`#table`-style intrinsics. `AzureStorage.Blobs` resolves through the offline blob map only; every other data-access function (`Sql.*`, `Web.*`, `File.*`, `SharePoint.*`, …) returns "unsupported" and the table offers "Load CSV manually…". Deviations (numbers are `f64`, no query folding, cultures ignored) are listed in the `pbi_m::eval` module docs.
- **Visual filters** (`pbi-render`): categorical and tuple `In`/`Not In`, advanced comparisons and `Between`, contains/starts-with, blank tests and Top N are applied; relative-date filters and a few rare forms are reported as "not applied" (matrix in the `pbi_render::query` module docs).

## Performance

`cargo xtask perf` runs 8 scenarios in a release build against budgets (`perf/budgets.toml`) and a per-machine baseline (`perf/baseline.json`); CI runs a quick, informational pass ([docs/perf.md](docs/perf.md)). Committed baseline (Apple M4 Pro), every scenario within target:

| ID | Scenario | Result (ms) | Target / max (ms) |
|---|---|--:|---|
| P01 | Scroll step of a 50,000-row table (p95) | 0.71 | 16 / 33 |
| P02 | First render of that page in a fresh process | 104 | 1000 / 3000 |
| P03 | Example page without data (14 visuals) | 0.77 | 16 / 50 |
| P04 | 5 DAX measures over a 1,000,000-row fact table | 26 | 100 / 500 |
| P05 | M pipeline over a 100,000-row CSV | 149 | 250 / 1000 |
| P06 | Data of a Microsoft sample PBIX (XPress9 → ABF → VertiPaq → Arrow) | 4.5 | 20 / 100 |
| P07 | SVG of a busy synthetic dashboard | 1.6 | 4 / 16 |
| P08 | Scroll step of the example's Customer Detail table (p95) | 0.34 | 16 / 33 |

## CLI reference

| Command | Purpose | Key flags |
|---|---|---|
| `inspect <path>` | Model, reports and reader diagnostics | `--json`, `--pretty`, `--no-expressions` |
| `detect <path>` | Format detection only | `--json` |
| `open <path>` | Opens the file in the desktop app (`dashcraft-app`) | |
| `convert <path>` | Writes a PBIP project or a PBIT template | `--to pbip\|pbit`, `--out`, `--report`, `--force` |
| `lint <path>` | Broken-reference linter | `--json`, `--deny errors\|warnings\|infos`, `--disable <RULE>`, `--no-layout` |
| `refresh <path>` | Loads the data, per-table status | `--source auto\|m\|cache`, `--blob-map`, `--tables`, `--json` |
| `measures <path>` | Every measure's formatted value | `--report`, `--source`, `--blob-map`, `--json` |
| `export <path>` | Table data or model documentation | `--table` + `--format csv\|parquet`, `--model` + `--format json\|md`, `--out`, `--formatted`, `--bom`, `--excel-safe`, `--force` |
| `render <path>` | Report pages to PNG or SVG | `--png`, `--svg`, `--out-dir`, `--report`, `--page`, `--all-pages`, `--scale`, `--slicer`, `--now`, `--json` |
| `commands [<id>]` | The command catalog (47 commands plus 9 opt-in `ai.*`, with JSON Schemas) | `--json`, `--schemas`, `--markdown`, `--agent-safe` |
| `run <id>` | Runs one catalog command, or a batch, and prints its JSON result | `--params <json>`, `-p key=value`, `--doc <path>`, `--blob-map`, `--batch <file>`, `--stop-on-error`, `--pretty` |
| `serve` | JSON-lines RPC (JSON-RPC 2.0) over stdin/stdout with a persistent workspace | `--stdio`, `--read-root`, `--write-root`, `--blob-map`, `--max-concurrent`, `--agent` |
| `mcp` | MCP server for AI agents: the agent-safe commands as tools, confined to read/write roots | `--read-root`, `--write-root`, `--blob-map`, `--allow`, `--deny`, `--tool-names`, `--max-concurrent` |
| `ai login` / `ai logout` | Stores or deletes an AI provider's API key in the OS keychain (the `ai.*` commands run through `run`, `serve` and `mcp`) | `--provider`, `--stdin` |

Example: `dashcraft run measures.values --doc model.pbip --blob-map blob-map.json -p report=Overview`. Global flags: `-v` (repeatable), `-q`. Exit codes: **0** success (warnings allowed); **1** fatal error (unreadable or unsupported input, usage error, output exists); **2** the input was read but has errors (Error diagnostics, lint findings at the `--deny` level, failed tables, measures or visuals). Details: [docs/CLI.md](docs/CLI.md), [docs/COMMANDS.md](docs/COMMANDS.md).

## Security posture

- Every input file is **untrusted**. Parsers return errors and never panic; the workspace denies `unsafe` code. ZIP readers cap total size, entry count and compression ratio and refuse absolute, `..` and drive-letter paths (zip-slip); folders are walked without following symlinks. 14 `cargo-fuzz` targets run nightly.
- **No script execution:** R/Python visuals, `.pbiviz` custom visuals and `.mez`/`.pqx` connectors are listed or drawn as placeholders, never run. `SecurityBindings` is opaque. M code cannot read files or the network.
- **Offline-first, no telemetry.** Blob sources resolve only through a local `blob-map.json`; Web URL buttons are shown, never opened.
- **AI strictly opt-in:** `pbi-ai` is the only crate with network access (`cargo xtask layers` keeps HTTP/TLS and keychain crates inside it and its front ends); it sends nothing without a recorded consent per provider, capability and data class, and keys live only in the OS keychain.
- **Path policy for agents:** `serve --read-root/--write-root` and `mcp` confine every path to read and write roots (`..` and symlink escapes refused; `mcp` refuses writes without `--write-root`); agent callers reach only agent-safe commands.
- **Secrets are redacted** from metadata parts and connection strings before they are stored or displayed.
- **App hardening:** strict CSP with no remote origins, minimal Tauri capabilities (no file system, shell or HTTP plugins), report SVG sanitized before insertion. The app writes only its recent-files list, its AI settings (`ai.json`, no keys) and the PBIP/PBIT files you export.
- **Exports** can neutralize spreadsheet formulas (`--excel-safe`), are written atomically and never overwrite without `--force`.

## Test data policy

- Tests, fixtures, docs and examples use only the synthetic **Acme Outfitters** example ([reference/examples/acme-outfitters/](reference/examples/acme-outfitters/README.md): a PBIP with a TMDL model and three PBIR reports, CSVs and a blob map, generated by `scripts/examples/gen_acme.py`; expected values in [EXPECTED.md](reference/examples/acme-outfitters/EXPECTED.md) and `expected.json`, computed in plain Python independently of DashCraft) and MIT-licensed Microsoft sample reports ([reference/samples/microsoft/](reference/samples/microsoft/README.md): three PBIX and one PBIT committed; two larger PBIX files fetched by `scripts/fetch-samples.sh`, SHA-256 checked, into the gitignored `reference/samples/downloads/`).
- Real or internal reports and data are never committed or described ([AGENTS.md golden rules](AGENTS.md#3-golden-rules) 2 to 4).
- `reference/golden/` (optional Power BI exports for the SSIM comparison) and `reference/private/` (your own files) are local-only and gitignored; renders, exports and perf reports go under `target/`.
- `.gitignore` refuses `*.pbix`, `*.pbit` and `*.abf` anywhere except the committed Microsoft samples and the fuzz seeds.
- Tests that need an optional file print `SKIP: <file> missing` and pass, so CI on a fresh clone stays green. The fixture root is `$PBIVIEW_REFERENCE`, else `reference/` ([reference/README.md](reference/README.md)).

## Architecture

Pipeline: containers → format readers → `Document` (model + report AST) → session (M refresh, VertiPaq decode, DAX) → visual queries → DataView → visuals → scene graph → SVG/PNG; the command registry serves it all to the CLI, the app and MCP agents. Details, crate graph and extension points: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

Dependencies point down only; `cargo xtask layers` enforces it in CI (no upward edges, UI-toolkit crates only in the app, network and keychain crates only in `pbi-ai` and its front ends).

| Layer | Crates | Responsibility |
|---|---|---|
| L6 front ends | `pbiview-cli` (`dashcraft`, alias `pbiview`), `pbiview-app` (`app/src-tauri`, `dashcraft-app`) | Adapters: argument parsing, IPC (`ops_execute` → the registry, app-only views), Svelte 5 UI |
| L5 operations | `pbi-ops`, `pbi-ai` | Command registry, workspace, path policy, JSON-lines RPC, MCP server; opt-in `ai.*` commands, providers, consent, keychain-only keys |
| L4 render | `pbi-render` | Theme, layout, text, visual queries → DataView, visuals → scene → SVG/PNG, golden comparison |
| L3 session | `pbi-session` | Open → refresh (M, VertiPaq, `.pbi/cache.abf`, calculated objects, derived columns), CSV loading, RLS, query façade, exports |
| L2 DAX, readers | `pbi-dax`, `pbi-core` | DAX lexer, parser, formula engine; detection, safe containers, readers → `Document`, PBIP/PBIT writers, linter |
| L1 formats, M, storage | `pbi-tmdl`, `pbi-tmsl`, `pbi-pbir`, `pbi-m`, `pbi-vertipaq`, `pbi-engine` | Lossless TMDL, TMSL and PBIR; M lexer, parser and evaluator; clean-room XPress9, ABF and VertiPaq decoding; Arrow storage engine, relationships, filter propagation, DataFusion SQL |
| L0 model | `pbi-model` | TOM-like model, report AST, `ObjectId`s, `Document` + `Command` (undo/redo) |

Edges: `cli → ops, ai, core, session, render`; `app → ops, ai, core, session, m, tmdl`; `ai → ops, session, dax, engine`; `ops → core, session, render, engine`; `render → session, dax, engine, pbir`; `session → core, m, dax, engine, vertipaq`; `core → tmdl, tmsl, pbir, vertipaq, dax, m`; `dax → engine`. Every crate above L0 except `pbi-m` also depends on `pbi-model`; `pbi-m` and `pbi-model` have no workspace dependencies. `xtask` (`cargo xtask layers`, `cargo xtask perf`) is tooling that no crate depends on.

## Build and test

```sh
cargo fmt --all --check
cargo xtask layers                                   # crate layering
cargo clippy --all-targets --locked -- -D warnings   # default members: crates/* and xtask
cargo test --locked --no-fail-fast                   # tests needing optional data print SKIP and pass
cargo xtask perf --quick --no-fail                   # performance budgets (docs/perf.md)
cargo deny check                                     # licenses, bans, advisories, sources
cargo test -p pbi-render --test golden -- --nocapture   # SSIM only where local golden images exist
(cd app && npm ci && npm run check && npm run build)    # app front end
cargo clippy -p pbiview-app --all-targets -- -D warnings
cargo test -p pbiview-app
```

The app crate is not a default member (it needs the built front end and, on Linux, the WebKitGTK development packages). CI runs these on macOS arm64, Windows x64 and Ubuntu ([docs/ci.md](docs/ci.md)). Every gate, snapshot workflow (`cargo insta review`) and fuzzing command is in [AGENTS.md §7](AGENTS.md#7-build-test-validate).

- **Tests** in every crate: TMDL byte-identical and PBIR JSON-identical round-trips on every fixture file, `insta` snapshots of `inspect --json`, one test per DAX function, M standard-library tests, XPress9 known-good vectors, engine filter-propagation tests, registry commands through `Registry::execute`, CLI end-to-end tests.
- **Render tests** (`crates/pbi-render/tests/`): `insta` render snapshots of all 8 example pages, the 39 card values of `expected.json` on the rendered pages, every page of the Microsoft samples; golden SSIM (≥ 0.85 per region and page, no exceptions) only where `reference/golden/<example>/<report>/<page>.png` exists ([method](docs/VISUALS.md#render-verification)).
- **Fuzzing**: 14 `cargo-fuzz` targets (ZIP containers, detection, `open_bytes`, TMDL, TMSL, PBIR, legacy layout, M parser and evaluator, DAX, XPress9, ABF, VertiPaq, people parser), nightly ([docs/ci.md](docs/ci.md#fuzz-targets)).

## Licensing and third-party notes

DashCraft is open source under the [MIT License](LICENSE). Rust dependencies are limited to permissive licenses by `cargo-deny` ([deny.toml](deny.toml)). Bundled assets: Selawik and Carlito fonts (SIL OFL 1.1, unmodified) and Fluent UI System Icons (MIT); no Power BI icons, images or CSS. The Microsoft sample reports are MIT-licensed test fixtures only, never shipped. The XPress9, ABF and VertiPaq decoders are clean-room implementations; references and the open legal questions are in [ADR-0003](docs/adr/0003-vertipaq-xpress9-cleanroom.md), [crates/pbi-vertipaq/REFERENCES.md](crates/pbi-vertipaq/REFERENCES.md) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Shipping PBIX data decoding in a release needs the legal review recorded in ADR-0003. Trademark and non-affiliation notice: [NOTICE](NOTICE).

## Documentation

- [AGENTS.md](AGENTS.md): start here to contribute (people and AI agents): golden rules, parallel-agent protocol, validation, definition of done
- [docs/DESKTOP-PLAN.md](docs/DESKTOP-PLAN.md): the plan from viewer to Power BI Desktop reimplementation (M6 onward)
- [docs/ROADMAP.md](docs/ROADMAP.md), [docs/GAPS.md](docs/GAPS.md) (gaps scorecard), [docs/DEVLOG.md](docs/DEVLOG.md) (dev log per wave), [CHANGELOG.md](CHANGELOG.md)
- [docs/USER-GUIDE.md](docs/USER-GUIDE.md): the app and CLI recipes; [docs/CLI.md](docs/CLI.md): every command, flag, exit code and JSON schema
- [docs/COMMANDS.md](docs/COMMANDS.md): the command catalog (generated); [docs/MCP.md](docs/MCP.md): the MCP server; [docs/AI.md](docs/AI.md): the opt-in AI features
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): pipeline, crates, extension points; [docs/perf.md](docs/perf.md): performance budgets
- [FORMATS.md](FORMATS.md): format support matrix; [docs/VISUALS.md](docs/VISUALS.md): per-visual support, service quirks, render verification
- [crates/pbi-dax/FUNCTIONS.md](crates/pbi-dax/FUNCTIONS.md): DAX functions; [crates/pbi-core/src/lint/RULES.md](crates/pbi-core/src/lint/RULES.md): lint rules; [crates/pbi-vertipaq/FORMAT-NOTES.md](crates/pbi-vertipaq/FORMAT-NOTES.md): XPress9, ABF and VertiPaq notes
- [app/README.md](app/README.md) and [app/docs/desktop-ui-reference.md](app/docs/desktop-ui-reference.md): the desktop app and its Power BI Desktop mapping
- [reference/README.md](reference/README.md): test fixtures; [docs/ci.md](docs/ci.md) (CI, release, fuzzing); [docs/packaging.md](docs/packaging.md) (installers); [docs/research/](docs/research/) (*Craft reviews); [NOTICE](NOTICE)
- Architecture decisions ([index](docs/adr/README.md)):
  - [ADR-0001: UI with Tauri 2 + Svelte 5 + TypeScript](docs/adr/0001-ui-tauri-svelte.md)
  - [ADR-0002: Arrow + DataFusion data engine](docs/adr/0002-arrow-datafusion.md)
  - [ADR-0003: Clean-room XPress9 / VertiPaq and license research](docs/adr/0003-vertipaq-xpress9-cleanroom.md)
  - [ADR-0004: Desktop-ready architecture](docs/adr/0004-desktop-ready-architecture.md)
  - [ADR-0005: Rust SVG renderer](docs/adr/0005-rust-svg-renderer.md)
  - [ADR-0006: Report interaction state outside the document](docs/adr/0006-interaction-state.md)
  - [ADR-0007: Command registry (pbi-ops) for the CLI, RPC, app and agents](docs/adr/0007-command-registry.md)
  - [ADR-0008: Opt-in AI layer (pbi-ai)](docs/adr/0008-ai-layer.md)
  - [ADR-0009: Product name DashCraft](docs/adr/0009-naming-dashcraft.md)
