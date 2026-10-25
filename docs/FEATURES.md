# Features

What DashCraft does today, by area, with the format and visual matrices. The [README](../README.md)
has the short version; the gaps against Power BI Desktop are scored in [GAPS.md](GAPS.md) and the
plan to close them is in [DESKTOP-PLAN.md](DESKTOP-PLAN.md).

## By area

**Data Explorer**

- Content-based format detection; safe readers for PBIP (TMDL or `model.bim`, PBIR or PBIR-Legacy), PBIX (PBIR or legacy `Layout`, embedded VertiPaq data), PBIT and the other formats below.
- Model tree: tables, columns (display and source names), measures, calculated columns and tables, relationships, M partitions and parameters, hierarchies, roles, calculation groups, `///` descriptions.
- Data offline: M partitions run in DashCraft's own M engine (`AzureStorage.Blobs` through a local blob map); PBIX, ABF and Power Pivot data is decoded from VertiPaq; a PBIP's `.pbi/cache.abf` loads without M; "Load CSV manually…" as a fallback.
- DAX engine for measures, calculated columns and tables, and `EVALUATE` queries, with calculation groups, many-to-many relationships and row-level security ("View as" roles and a user).
- Virtualized data grid with sort, SQL filter and column statistics; session-derived columns ("Parse people field", DAX expressions).
- Exports: table to CSV (optional formula-injection guard) or Parquet; model to JSON or a Markdown data dictionary. Conversion to a PBIP project or PBIT template.
- Broken-reference linter ([17 rules](../crates/pbi-core/src/lint/RULES.md)), including stale `queryRef`s.

**Report Renderer**

- Each visual's `queryState`, report/page/visual filters, slicer selections and cross-filter selections become a DAX `SUMMARIZECOLUMNS` query; results are Power BI-shaped DataViews.
- Built-in Rust visuals draw into a scene graph, serialized to deterministic SVG (app) and PNG (CLI, tests) with bundled fonts only (Selawik for Segoe UI, Carlito for Calibri).
- Themes (base and custom, `ThemeDataColor`, `visualStyles`, text classes), page display options, z-order, groups and container chrome; conditional formatting and dynamic format strings.
- Service rendering quirks emulated ([docs/VISUALS.md](VISUALS.md)); verified by render snapshots and expected values of the example, and by SSIM only where optional local golden images exist.

**Command registry, automation and MCP**

- Every capability is a registry command with a dotted id (`doc.open`, `measures.values`, `dax.query`, `render.page`, `canvas.click`, `security.viewAs`, `export.table`, …), typed parameters and results with JSON Schemas, and metadata (agent-safe, mutates the document, long-running, writes files).
- Front ends are thin adapters: `dashcraft run` (and `--batch` with `${step.field}` references), `dashcraft serve` (JSON-RPC 2.0 over stdio with progress and `$/cancel`), `dashcraft mcp` and the app.
- `dashcraft mcp` serves the agent-safe commands as MCP tools (PNG renders as images), with read/write roots, `--allow`/`--deny` filters and protocol revisions 2024-11-05 to 2025-11-25 plus the stateless 2026-07-28 ([docs/MCP.md](MCP.md)).

**AI (opt-in)**

- Off by default; with AI off DashCraft makes no network requests at all ([docs/AI.md](AI.md)).
- `ai.explainMeasure` and `ai.generateMeasure` (suggestions are parsed, checked against the model and optionally evaluated locally, never written to it); `ai.preview` shows the exact request with keys redacted; consents per provider, capability and data class (model metadata by default, never whole tables).
- Providers: OpenAI-compatible, Azure OpenAI, Ollama (local). Keys live only in the OS keychain (`dashcraft ai login`) or `DASHCRAFT_AI_API_KEY`; settings files never hold keys.

**Desktop app** ([app/README.md](../app/README.md))

- Power BI Desktop's layout: ribbon, view rail (Report, Table, Model, DAX query, TMDL), Filters / Visualizations / Data / Selection / Properties panes, page tabs, status bar, recent files. Editing commands are shown disabled.
- Interactive report canvas: slicers (single and multi-select, dropdowns, search), cross-filtering per `visualInteractions`, tooltips, table and matrix scrolling, matrix expand/collapse, Back and page-navigation buttons.
- DAX query view, read-only Transform data editor, Model view diagram, local data cache banner with Refresh from source, Modeling → View as, File → Export (PBIP or PBIT), Options → AI.
- Open by dialog, drag and drop, command line or file association (`.pbix`, `.pbit`, `.pbip`, `.bim`, `.vpax`).

## Supported formats

Detection uses content (ZIP entries, JSON keys, `$schema`, encodings), not just the extension. Full matrix with detection signals, readers, fixtures and gaps: [FORMATS.md](../FORMATS.md).

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

Levels: **Supported** (called Full in VISUALS.md): the common formatting objects are honoured and the layout follows the service; **Partial**: the common options render, notable gaps listed; **Placeholder**: a box with the visual type and its bound fields (the query still runs). Details and differences from the service: [docs/VISUALS.md](VISUALS.md).

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

**Conditional formatting** (tables, matrices, and the colors of bar, column, pie, donut, treemap, scatter and funnel charts without a legend field): color scales with 2 or 3 colors, rules, field values, data bars and built-in icons. Unsupported variants (headers, titles, cards, per-legend chart colors, URL icons, …) render unformatted with a warning ([details](VISUALS.md#conditional-formatting)). Measures with dynamic format strings (calculation items, `formatStringDefinition`) format each value with its own format string. A visual whose query fails draws "Can't display this visual" with a warning icon; the app's tooltip shows the message and the failing measure's DAX. Rendering never panics.

## DAX and M coverage

- **DAX** (`pbi-dax`): 294 functions in twelve families (logical and information, math and statistics, statistical distributions, text, date and time, time intelligence, aggregation and iterators, filter and context, table manipulation, window functions, parent-child, calculation groups and security); 271 supported, 23 partial. `DEFINE`/`EVALUATE` queries, calculated columns and tables, `FORMAT` strings (en-US), `CALCULATE` modifiers, `ALLSELECTED`, `USERELATIONSHIP`/`CROSSFILTER`, `SUMMARIZECOLUMNS` with rollups, calculation groups (precedence, selection expressions, dynamic format strings), many-to-many (limited) relationships, row-level security (`USERPRINCIPALNAME`, `CUSTOMDATA`). Not supported: financial, `INFO.*` and visual-calculation functions, object-level security, DirectQuery. An unsupported function returns a typed error, shown with a warning and the DAX text. List and semantic gaps: [crates/pbi-dax/FUNCTIONS.md](../crates/pbi-dax/FUNCTIONS.md).
- **Power Query M** (`pbi-m`): full grammar; lazy evaluator with closures, `each`, `try`/`otherwise`, `meta`, sections. 349 library names (`pbi_m::library_names()`): 261 functions (`Table.*` 45, `List.*` 41, `Text.*` 36, `Number.*` 20, `Date.*` 17, `Record.*` 15, `DateTimeZone.*` 13, `Duration.*` 11, `Value.*` 10, `DateTime.*` 8, `Time.*` 6, `Csv.Document`, `Json.Document`, …), 32 `*.Type` values and 56 constants, plus the `#date`/`#table`-style intrinsics. `AzureStorage.Blobs` resolves through the offline blob map only; every other data-access function (`Sql.*`, `Web.*`, `File.*`, `SharePoint.*`, …) returns "unsupported" and the table offers "Load CSV manually…". Deviations (numbers are `f64`, no query folding, cultures ignored) are listed in the `pbi_m::eval` module docs.
- **Visual filters** (`pbi-render`): categorical and tuple `In`/`Not In`, advanced comparisons and `Between`, contains/starts-with, blank tests and Top N are applied; relative-date filters and a few rare forms are reported as "not applied" (matrix in the `pbi_render::query` module docs).
