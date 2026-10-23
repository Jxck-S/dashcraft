<!-- Generated from the command registry (crates/pbi-ops). Do not edit by hand:
     cargo run -q -p pbiview-cli --bin dashcraft -- commands --markdown > docs/COMMANDS.md
     (the pbi-ops test `commands_md_is_up_to_date` fails when this file is stale). -->

# Command catalog

Every DashCraft capability is a **command**: a stable dotted id, typed parameters and a typed result, both described by JSON Schema (draft 2020-12). The CLI (`dashcraft run`, `dashcraft serve`, `dashcraft mcp`), the desktop app and AI agents (MCP) all run the same commands through one registry (`pbi-ops`, [ADR-0007](adr/0007-command-registry.md)). `pbiview` is an alias of the `dashcraft` binary.

## Running commands

```sh
dashcraft commands                                   # the catalog (--json, --schemas, --markdown)
dashcraft run doc.detect -p path=model.pbip          # -p key=value (values parsed as JSON when they are JSON)
dashcraft run report.pages --doc model.pbip          # --doc opens the file first and passes its docId
dashcraft run measures.values --doc model.pbip --blob-map blob-map.json -p report=Stores --pretty
dashcraft run dax.query --doc model.pbip --params '{"query": "EVALUATE ROW(\"n\", [Total Sales])"}'
dashcraft run --batch steps.json                     # ordered steps sharing one workspace
dashcraft serve --stdio                              # JSON-lines RPC (below)
dashcraft mcp                                        # MCP server for AI agents (MCP.md)
```

- Parameters are camelCase JSON objects; unknown fields are rejected (`invalidParams`).
- Documents are opened with `doc.open`, which returns a `docId` (`d1`, `d2`, …) for the other commands. Data commands (`data.page`, `dax.*`, `measures.values`, `render.*`, `canvas.*`, `export.table`) load the data on first use unless the document was opened with `"autoRefresh": false`; `data.refresh` reloads it.
- `dashcraft run` prints the result as JSON. Exit codes: 0 success; 1 fatal or usage error; 2 the input was read but data or expressions have errors (`query` errors, or a result with `"ok": false` — failed tables, ⚠ measures, failed visuals, lint findings).

### Batches

`dashcraft run --batch FILE` (or `-` for stdin) and `commands.batch` run ordered steps on one workspace:

```json
[
  { "id": "doc.open", "params": { "path": "model.pbip", "blobMap": "blob-map.json" }, "as": "d" },
  { "id": "measures.values", "params": { "docId": "${d.docId}", "report": "Stores" } },
  { "id": "render.page", "params": { "docId": "${d.docId}", "format": "png", "outputPath": "page.png", "overwrite": true } }
]
```

A string that is exactly `${name.path}` is replaced by the referenced value of the step named `name` (any JSON type); inside a longer string the value's text is spliced in; `$${` is a literal `${`. Paths are dotted, array items by index (`${p.pages.0.name}`). Batches stop at the first error unless `"stopOnError": false` (`--stop-on-error false`); they are not atomic. The output is an array of `{ index, id, as?, ok, result?, error? }`.

### JSON-lines RPC (`dashcraft serve`)

One JSON-RPC 2.0 message per line on stdin/stdout (logs go to stderr):

```text
→ {"jsonrpc":"2.0","id":1,"method":"doc.open","params":{"path":"model.pbip"}}
← {"jsonrpc":"2.0","id":1,"result":{"docId":"d1", …}}
→ {"jsonrpc":"2.0","id":2,"method":"data.refresh","params":{"docId":"d1"}}
← {"jsonrpc":"2.0","method":"progress","params":{"id":2,"command":"data.refresh","message":"Orders: 3900 rows in 82 ms","fraction":0.25}}
→ {"jsonrpc":"2.0","method":"$/cancel","params":{"id":2}}
← {"jsonrpc":"2.0","id":2,"error":{"code":-32800,"message":"the operation was cancelled","data":{"code":"cancelled"}}}
```

- Methods are command ids; `params` is an object (absent = `{}`).
- Requests run concurrently (bounded); commands that mutate a document run one at a time per `docId`, in arrival order. Responses may arrive out of order: match them by `id`.
- Notifications (no `id`) run without a response. `$/cancel {"id": …}` cancels a running or queued request. Long-running requests send `progress` notifications (best effort).
- Errors: `{ "code": <JSON-RPC integer>, "message", "data": { "code": "<string code>", "details"? } }`. Malformed lines get `-32700` (parse error) or `-32600` (invalid request).
- `--read-root DIR` / `--write-root DIR` restrict the files commands may read and write (agents); without them paths are unrestricted.

### Error codes

| Code | JSON-RPC | Exit | Meaning |
|---|---|---|---|
| `invalidParams` | -32602 | 1 | Parameters do not match the schema, or a value is out of range |
| `unknownCommand` | -32601 | 1 | No command with that id |
| `notFound` | -32001 | 1 | A file, table, column, measure, report, page or visual does not exist |
| `noDocument` | -32002 | 1 | The `docId` is not open, or the document has no model / report |
| `unsupported` | -32003 | 1 | Unsupported or refused input, or an operation DashCraft does not implement |
| `pathDenied` | -32004 | 1 | The path policy refused a path (outside the roots, symlink or `..` escape) |
| `io` | -32005 | 1 | A file could not be read or written (or already exists) |
| `query` | -32006 | 2 | A DAX, SQL, M or CSV evaluation failed |
| `cancelled` | -32800 | 1 | The command was cancelled |
| `busy` | -32007 | 1 | Too many requests in flight or documents open |
| `internal` | -32603 | 1 | A bug (a panic or an unexpected failure) |

### Agents (MCP)

Commands marked **agent-safe** are offered to AI agents by `dashcraft mcp`, a thin adapter over this registry: `tools/list` = the agent-safe commands (`id` → tool name, `summary` → description, `paramsSchema` → `inputSchema`, `outputSchema` → `outputSchema`), `tools/call` = `Registry::execute` with an `Agent` caller and a restricted path policy; progress and cancellation map to MCP `notifications/progress` and `notifications/cancelled`. Setup, client configuration, roots and tool filters: [MCP.md](MCP.md).

## Catalog

Flags: **M** mutates an open document (ordered per document) · **L** long-running (progress, cancellable) · **A** agent-safe · **W** writes files (path policy).

| Command | Summary | Flags |
|---|---|---|
| [`commands.list`](#commandslist) | Lists every command with its metadata (and JSON Schemas with `schemas`). | A |
| [`commands.describe`](#commandsdescribe) | One command's metadata, examples and parameter/result JSON Schemas. | A |
| [`commands.batch`](#commandsbatch) | Runs ordered steps on the shared workspace; strings like "${d.docId}" reference earlier results named with "as". | A |
| [`doc.detect`](#docdetect) | Content-based format detection of a file or folder (signals, models, reports) without reading it fully. | A |
| [`doc.open`](#docopen) | Opens a Power BI file or folder into the workspace (no data loaded yet) and returns its docId and summary. | A |
| [`doc.close`](#docclose) | Closes an open document and frees its data (running canvas renders are cancelled). | M A |
| [`doc.list`](#doclist) | The open documents of the workspace with their summaries. | A |
| [`doc.inspect`](#docinspect) | The full model, reports and reader diagnostics: the `dashcraft inspect --json` schema (schemaVersion 1). | A |
| [`model.tables`](#modeltables) | Every model table with its columns, measure names, partitions, hierarchies and loaded row count. | A |
| [`model.measures`](#modelmeasures) | Every measure with its table, DAX, format string, description and display folder. | A |
| [`model.relationships`](#modelrelationships) | Every relationship: from/to columns, cardinalities, cross-filter direction, active flag. | A |
| [`report.list`](#reportlist) | The document's reports: name, dataset reference, theme, page and visual counts, report filters. | A |
| [`report.pages`](#reportpages) | The pages of a report: names, size, visibility, visual and filter counts. | A |
| [`report.visuals`](#reportvisuals) | The visuals of a page: type, title, position, bound fields and visual filters. | A |
| [`report.filters`](#reportfilters) | A report's filters (plus a page's) translated to column filters, with the ones that cannot be applied (Top N, relative date…). | A |
| [`security.roles`](#securityroles) | The model's row-level security roles (permission, DAX table filters, members, dynamic or not) and the current "View as" state. | A |
| [`doc.lint`](#doclint) | Broken-reference checks (visual fields, queryRefs, filters, DAX/M, relationships, layout) as the `lint --json` report. | A |
| [`data.refresh`](#datarefresh) | Loads the data offline (PBIP cache, M partitions with the blob map, embedded PBIX data, calculated objects); per-table rows, time and status. | M L A |
| [`data.tables`](#datatables) | Loaded rows, columns and last refresh status of every table, the derived columns and the local data cache. | A |
| [`data.page`](#datapage) | One page of a table's rows (sorted, SQL-filtered), raw or formatted; loads the data on first use. | A |
| [`data.columnStats`](#datacolumnstats) | Rows, blanks, distinct count, min and max of one column; loads the data on first use. | A |
| [`data.loadCsv`](#dataloadcsv) | Replaces a table's data with a CSV ("Load CSV manually…") and recomputes what depends on it. | M L A |
| [`data.addDerivedColumn`](#dataaddderivedcolumn) | Adds a session-only column (people parsing or a DAX expression), re-applied after every refresh; never saved to the model. | M A |
| [`data.removeDerivedColumn`](#dataremovederivedcolumn) | Removes a session derived column from the table data and the session model. | M A |
| [`security.viewAs`](#securityviewas) | Views the data as security roles and/or a user (row-level security, union of roles); empty roles and no user stop it. Visible rows per table. | M A |
| [`dax.query`](#daxquery) | Evaluates a DAX query (EVALUATE …) and returns columns and rows per result table; loads the data on first use. | L A |
| [`dax.evaluateMeasure`](#daxevaluatemeasure) | One measure's value (raw and formatted, or ⚠ error with its DAX) under report/page and column filters. | A |
| [`measures.values`](#measuresvalues) | Every measure's formatted value under optional report/page filters; failed measures show ⚠ and their DAX. | L A |
| [`render.page`](#renderpage) | Renders a report page to SVG or PNG (inline, base64 for PNG, or to outputPath) with per-visual status; loads the data on first use. | L A W |
| [`render.visual`](#rendervisual) | Renders one visual of a page, cropped to its container, to SVG or PNG (with the page's filters, slicers and interactions). | L A W |
| [`canvas.render`](#canvasrender) | Renders a page with its current interaction state: SVG, hit regions, visuals, slicer/cross-filter selections and effective filters. | M L A |
| [`canvas.click`](#canvasclick) | Applies a click on a hit region (slicer item, data point, button, dropdown) and returns the re-rendered page and any button action. | M L A |
| [`canvas.scroll`](#canvasscroll) | Scrolls a table, matrix or slicer list by a delta (clamped to its range) and returns the re-rendered page. | M A |
| [`canvas.setSlicer`](#canvassetslicer) | Replaces a slicer's selection by value (no hit ids needed) and returns the re-rendered page. | M L A |
| [`canvas.clearSelections`](#canvasclearselections) | Clears a slicer's selection ("Clear selections") or the page's cross-filter, and returns the re-rendered page. | M L A |
| [`canvas.setSlicerSearch`](#canvassetslicersearch) | Filters a slicer's list (or open dropdown) by a search text and returns the re-rendered page. | M L A |
| [`canvas.toggleMatrixRow`](#canvastogglematrixrow) | Expands or collapses the matrix row of a cell hit and returns the re-rendered page. | M L A |
| [`canvas.closePopups`](#canvasclosepopups) | Closes the open slicer dropdowns of a page. | M A |
| [`canvas.reset`](#canvasreset) | Back to the report's saved slicer selections, no cross-filter, scroll and dropdowns reset; returns the re-rendered page. | M L A |
| [`canvas.tooltip`](#canvastooltip) | The hover tooltip (category, series, values) of a data point hit, from the last render. | A |
| [`canvas.state`](#canvasstate) | The page's current interaction state (slicer selections, cross-filter) and view state, as JSON. | A |
| [`export.table`](#exporttable) | Writes one table's data as CSV or Parquet (with model metadata); loads the data on first use. | L A W |
| [`export.model`](#exportmodel) | The model documentation as JSON (model doc) or a Markdown data dictionary, inline or to a file; no data needed. | A W |
| [`convert.pbip`](#convertpbip) | Writes the document as a PBIP project (TMDL + PBIR) into a folder. | L A W |
| [`convert.pbit`](#convertpbit) | Writes a PBIT template (model schema and one report, no data). | L A W |
| [`text.parsePeople`](#textparsepeople) | Parses one people-picker cell (a Python-style list of dicts) into `First Last` names (the "Parse people field" transform). | A |
| [`text.previewPeople`](#textpreviewpeople) | Parses every cell of a text column as a people list: outcome counts and distinct samples with their parsed form (before data.addDerivedColumn). | A |

## Meta

### `commands.list`

**List commands.** Lists every command with its metadata (and JSON Schemas with `schemas`).

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `schemas` (optional) | boolean | Include each command's parameter and result JSON Schemas. |
| `category` (optional) | `"meta"` \| `"workspace"` \| `"inspect"` \| `"lint"` \| `"data"` \| `"dax"` \| `"render"` \| `"canvas"` \| `"export"` \| `"convert"` \| `"transform"` \| `"ai"` \| null | Only this category. |
| `agentSafe` (optional) | boolean | Only the commands offered to agents. |

Result:

| Field | Type | Description |
|---|---|---|
| `count` | integer |  |
| `commands` | CommandInfo[] |  |

Examples:

- All commands: `{}`
- Agent tools with schemas: `{"schemas":true,"agentSafe":true}`

### `commands.describe`

**Describe a command.** One command's metadata, examples and parameter/result JSON Schemas.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `id` | string | Command id, e.g. `doc.open`. |

Result:

| Field | Type | Description |
|---|---|---|
| `id` | string | Stable dotted id (`doc.open`); the JSON-RPC method name. |
| `title` | string |  |
| `summary` | string | One line: what it does and returns. |
| `category` | `"meta"` \| `"workspace"` \| `"inspect"` \| `"lint"` \| `"data"` \| `"dax"` \| `"render"` \| `"canvas"` \| `"export"` \| `"convert"` \| `"transform"` \| `"ai"` | What a command is about; groups the catalog. |
| `mutatesDocument` | boolean | Changes the in-memory state of an open document (ordered per document). |
| `longRunning` | boolean | Reports progress and can take a while. |
| `agentSafe` | boolean | Offered to AI agents (MCP) by default. |
| `writesFiles` | boolean | Writes files (checked by the path policy). |
| `since` | string | Version that added it. |
| `examples` | ExampleInfo[] |  |
| `paramsSchema`? | any | JSON Schema of the parameters (with `schemas`). |
| `outputSchema`? | any | JSON Schema of the result (with `schemas`). |

Examples:

- Describe doc.open: `{"id":"doc.open"}`

### `commands.batch`

**Run a batch.** Runs ordered steps on the shared workspace; strings like "${d.docId}" reference earlier results named with "as".

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `steps` | BatchStep[] | Steps, run in order on the same workspace. |
| `stopOnError` (optional) | boolean | Stop at the first failing step (default `true`). Batches are not atomic: earlier steps stay applied. |

Result:

| Field | Type | Description |
|---|---|---|
| `ok` | boolean | Every step ran and succeeded. |
| `results` | BatchStepResult[] | One entry per step that ran, in order. |

Examples:

- Open, then list pages: `{"steps":[{"id":"doc.open","params":{"path":"examples/acme-outfitters/AcmeOutfitters.pbip"},"as":"d"},{"id":"report.pages","params":{"docId":"${d.docId}"}}]}`

## Workspace

### `doc.detect`

**Detect format.** Content-based format detection of a file or folder (signals, models, reports) without reading it fully.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `path` | string | File or folder. |

Result:

| Field | Type | Description |
|---|---|---|
| `schemaVersion` | integer |  |
| `path` | string | The path as given. |
| `sourceFormat` | string | The `source.format` `inspect` reports for this input. |
| `detection` | DetectionOut |  |
| `items` | ItemsOut | Structure found while detecting (paths relative to the input folder, the `.pbip` folder or the archive root). |

Examples:

- A PBIP project: `{"path":"examples/acme-outfitters/AcmeOutfitters.pbip"}`

### `doc.open`

**Open a document.** Opens a Power BI file or folder into the workspace (no data loaded yet) and returns its docId and summary.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `path` | string | File or folder: `.pbip`, `.pbix`, `.pbit`, `*.SemanticModel`, `model.bim`, a TMDL folder, … (see FORMATS.md). |
| `blobMap` (optional) | string \| null | Offline `AzureStorage.Blobs` mapping (`blob-map.json`: blob URLs → local files). Default: the workspace's blob map. |
| `source` (optional) | `"auto"` \| `"query"` \| `"cache"` \| null | Default data source of refreshes (default `auto`). |
| `loadPbixSchema` (optional) | boolean \| null | Decompress the PBIX `DataModel` / PBIP cache to read the model schema (default `true`). |
| `includeSystemTables` (optional) | boolean | Keep internal VertiPaq tables (`H$`, `R$`, `U$`) (default `false`). |
| `autoRefresh` (optional) | boolean \| null | Data commands load the data on first use when nothing was loaded (default `true`). Turn off to see empty tables until `data.refresh`. |
| `now` (optional) | string \| null | The time M sees as "now" (RFC 3339), for reproducible "last refreshed" values. Default: the system clock. |

Result:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The id to pass to other commands. |
| `path` | string | The path as given. |
| `format` | string | Source format (`pbip`, `pbix`, `pbit`, `bim`, `tmdl`, …). |
| `formatLabel` | string | Detected format, for people. |
| `hasModel` | boolean |  |
| `modelName`? | string \| null |  |
| `embeddedData` | boolean | Data comes from embedded VertiPaq storage (PBIX, ABF). |
| `hasCache` | boolean | A PBIP local data cache (`.pbi/cache.abf`) is available. |
| `blobMap` | boolean | A blob map is configured. |
| `source` | string | Default refresh source: `auto`, `query` or `cache`. |
| `autoRefresh` | boolean |  |
| `loaded` | boolean | Data was loaded. |
| `generation` | integer | Data generation (changes after every data change). |
| `tables` | integer |  |
| `measures` | integer |  |
| `relationships` | integer |  |
| `reports` | ReportBrief[] |  |
| `diagnostics` | DiagnosticCounts | Reader diagnostics. |

Examples:

- A PBIP project: `{"path":"examples/acme-outfitters/AcmeOutfitters.pbip"}`
- With a blob map and a pinned clock: `{"path":"examples/acme-outfitters/AcmeOutfitters.pbip","blobMap":"examples/acme-outfitters/blob-map.json","now":"2026-10-21T21:35:00Z"}`

### `doc.close`

**Close a document.** Closes an open document and frees its data (running canvas renders are cancelled).

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id returned by `doc.open` (`d1`, …). |

Result:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id. |
| `closed` | boolean |  |

Examples:

- Close d1: `{"docId":"d1"}`

### `doc.list`

**List open documents.** The open documents of the workspace with their summaries.

_agent-safe; since 0.2.0._

Parameters:

object

Result:

| Field | Type | Description |
|---|---|---|
| `documents` | DocSummary[] |  |

Examples:

- All open documents: `{}`

## Inspect

### `doc.inspect`

**Inspect a document.** The full model, reports and reader diagnostics: the `dashcraft inspect --json` schema (schemaVersion 1).

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `includeExpressions` (optional) | boolean \| null | Include DAX, M and query text (default `true`); `false` gives an outline that is safe to share. |

Result:

| Field | Type | Description |
|---|---|---|
| `schemaVersion` | integer |  |
| `source` | SourceOut |  |
| `modelStatus` | ModelStatusOut |  |
| `model` | ModelOut \| null | The document's semantic model; `null` if none is available locally. |
| `extraModels` | ModelOut[] | Other semantic models of a multi-model project. |
| `reports` | ReportOut[] |  |
| `diagnostics` | DiagnosticOut[] |  |
| `counts` | Counts | Totals over `model`, `extraModels` and `reports`. |
| `vpaxStats`? | VpaxStatsOut \| null | VertiPaq Analyzer statistics (`.vpax`). |
| `paginatedReport`? | PaginatedReportOut \| null | Paginated report outline (`.rdl`). |
| `mashup`? | MashupOut \| null | Power Query details (`.pq`, `.pqt`, dataflows). |
| `customVisual`? | CustomVisualOut \| null | Custom visual metadata (`.pbiviz`; never executed). |
| `connector`? | ConnectorOut \| null | Custom connector listing (`.mez`/`.pqx`; never executed). |

Examples:

- Everything: `{"docId":"d1"}`
- Without DAX/M: `{"docId":"d1","includeExpressions":false}`

### `model.tables`

**Model tables.** Every model table with its columns, measure names, partitions, hierarchies and loaded row count.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `includeExpressions` (optional) | boolean \| null | Include DAX and M text (default `true`). |

Result:

| Field | Type | Description |
|---|---|---|
| `tables` | ModelTableOut[] |  |

Examples:

- Tables of d1: `{"docId":"d1"}`

### `model.measures`

**Model measures.** Every measure with its table, DAX, format string, description and display folder.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `table` (optional) | string \| null | Only the measures of this table. |

Result:

| Field | Type | Description |
|---|---|---|
| `measures` | MeasureDefOut[] |  |

Examples:

- All measures: `{"docId":"d1"}`
- One table: `{"docId":"d1","table":"Orders"}`

### `model.relationships`

**Model relationships.** Every relationship: from/to columns, cardinalities, cross-filter direction, active flag.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id returned by `doc.open` (`d1`, …). |

Result:

| Field | Type | Description |
|---|---|---|
| `relationships` | RelationshipOut[] |  |

Examples:

- Relationships of d1: `{"docId":"d1"}`

### `report.list`

**Reports.** The document's reports: name, dataset reference, theme, page and visual counts, report filters.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id returned by `doc.open` (`d1`, …). |

Result:

| Field | Type | Description |
|---|---|---|
| `reports` | ReportInfoOut[] |  |

Examples:

- Reports of d1: `{"docId":"d1"}`

### `report.pages`

**Report pages.** The pages of a report: names, size, visibility, visual and filter counts.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `report` (optional) | string \| null | Report name (exact, case-insensitive, or a unique part; or 1-based index). Default: the first report. |

Result:

| Field | Type | Description |
|---|---|---|
| `report` | string |  |
| `pages` | PageInfoOut[] |  |

Examples:

- Pages of the first report: `{"docId":"d1"}`
- Pages of a report by partial name: `{"docId":"d1","report":"Stores"}`

### `report.visuals`

**Page visuals.** The visuals of a page: type, title, position, bound fields and visual filters.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `report` (optional) | string \| null | Report (default: the first). |
| `page` (optional) | string \| null | Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. |

Result:

| Field | Type | Description |
|---|---|---|
| `report` | string |  |
| `page` | string | Page internal name. |
| `pageDisplayName` | string |  |
| `visuals` | VisualOut[] | Visuals in page order: type, position, bound fields, filters. |

Examples:

- Active page of the first report: `{"docId":"d1"}`
- A page by number: `{"docId":"d1","report":"Overview","page":"1"}`

### `report.filters`

**Report filters.** A report's filters (plus a page's) translated to column filters, with the ones that cannot be applied (Top N, relative date…).

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `report` (optional) | string \| null | Report (default: the first). |
| `page` (optional) | string \| null | Also translate this page's filters: internal name, display name, 1-based index or a unique part of the display name. Default: report filters only. |

Result:

| Field | Type | Description |
|---|---|---|
| `report` | string |  |
| `page`? | string \| null | Page internal name, when a page was given. |
| `filters` | FilterSpecOut | The filters as column filters (pass them to `measures.values` or `dax.query`), and those that cannot be translated. |

Examples:

- Report-level filters: `{"docId":"d1","report":"Stores"}`
- Report and page filters: `{"docId":"d1","report":"Stores","page":"1"}`

### `security.roles`

**Security roles.** The model's row-level security roles (permission, DAX table filters, members, dynamic or not) and the current "View as" state.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id returned by `doc.open` (`d1`, …). |

Result:

| Field | Type | Description |
|---|---|---|
| `roles` | SecurityRoleOut[] | The model's roles, in model order (empty without row-level security). |
| `viewAs` | ViewAsOut | The current "View as" state. |

Examples:

- Roles of d1: `{"docId":"d1"}`

## Lint

### `doc.lint`

**Lint a document.** Broken-reference checks (visual fields, queryRefs, filters, DAX/M, relationships, layout) as the `lint --json` report.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `minSeverity` (optional) | `"error"` \| `"warning"` \| `"info"` \| null | Leave out findings below this severity (default `info`: all). |
| `disable` (optional) | string[] | Rules to skip, e.g. `["LINT016"]`. |
| `layout` (optional) | boolean \| null | Run the layout rules LINT015/LINT016 (default `true`). |
| `deny` (optional) | `"error"` \| `"warning"` \| `"info"` \| null | `ok` is `false` for findings at or above this severity (default `error`), as `dashcraft lint --deny`. |

Result:

| Field | Type | Description |
|---|---|---|
| `schemaVersion` | integer |  |
| `path` | string | The document path as given. |
| `ok` | boolean | No finding at or above `deny` and no reader error. |
| `summary` | LintSummaryOut | Counts per severity and rule. |
| `findings` | FindingOut[] | Findings sorted by file, location and rule. |
| `readerErrors` | DiagnosticOut[] | Reader errors (checks that could not run). |

Examples:

- All rules: `{"docId":"d1"}`
- Warnings and worse, no overlap rule: `{"docId":"d1","minSeverity":"warning","disable":["LINT016"]}`

## Data

### `data.refresh`

**Refresh data.** Loads the data offline (PBIP cache, M partitions with the blob map, embedded PBIX data, calculated objects); per-table rows, time and status.

_mutates the document, long-running, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `tables` (optional) | string[] \| null | Tables to refresh (what depends on them is recomputed too). Default: every table. |
| `source` (optional) | `"auto"` \| `"query"` \| `"cache"` \| null | Data source (default: the one given to `doc.open`). |

Result:

| Field | Type | Description |
|---|---|---|
| `ok` | boolean | Every table and calculated column in scope succeeded. |
| `source` | string | The data source used: `auto`, `query` or `cache` (`manual` for CSV loads). |
| `durationMs` | integer |  |
| `summary` | RefreshCounts | Table counts of a refresh (or of the data loaded so far). |
| `tables` | TableRefreshOut[] | Tables in scope, in model order. |
| `derived` | DerivedColumnOut[] | Session derived columns after the run, with their status. |
| `diagnostics` | DiagnosticOut[] | Refresh diagnostics (cycles, decoder notes, CSV conversion warnings). |
| `generation` | integer | The data generation after the run. |

Examples:

- Every table: `{"docId":"d1"}`
- Evaluate the M queries of one table: `{"docId":"d1","tables":["Orders"],"source":"query"}`

### `data.tables`

**Data status.** Loaded rows, columns and last refresh status of every table, the derived columns and the local data cache.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id returned by `doc.open` (`d1`, …). |

Result:

| Field | Type | Description |
|---|---|---|
| `tables` | DataTableOut[] |  |
| `derived` | DerivedColumnOut[] | Session derived columns. |
| `cache`? | CacheOut \| null |  |
| `generation` | integer | Data generation. |

Examples:

- Status of d1: `{"docId":"d1"}`

### `data.page`

**Table rows.** One page of a table's rows (sorted, SQL-filtered), raw or formatted; loads the data on first use.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `table` | string | Table name (case-insensitive). |
| `offset` (optional) | integer | First row (0-based, after sorting and filtering). |
| `limit` (optional) | integer | Rows to return (default 100, at most 5000). |
| `sort` (optional) | SortParam \| null |  |
| `filter` (optional) | string \| null | SQL boolean expression over the table's columns, e.g. `"Status" = 'Completed'` (quote names with spaces). |
| `formatted` (optional) | boolean | Cells as text formatted with each column's format string instead of raw values. |
| `blankAsNull` (optional) | boolean | With `formatted`, BLANK cells are `null` instead of `""` (tells an empty text from BLANK). |

Result:

| Field | Type | Description |
|---|---|---|
| `table` | string | Table name (model spelling). |
| `offset` | integer |  |
| `totalRows` | integer | Rows passing the filter. |
| `columns` | ResultColumnOut[] |  |
| `rows` | any[][] | Row-major cells: raw values (numbers, strings, booleans, ISO dates, `null` for BLANK) or formatted text. |

Examples:

- First 20 rows: `{"docId":"d1","table":"Orders","limit":20}`
- Filtered and sorted: `{"docId":"d1","table":"Stores","filter":"\"Region\" = 'West'","sort":{"column":"Store Name"},"formatted":true}`

### `data.columnStats`

**Column statistics.** Rows, blanks, distinct count, min and max of one column; loads the data on first use.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `table` | string | Table name (case-insensitive). |
| `column` | string | Column name (case-insensitive). |

Result:

| Field | Type | Description |
|---|---|---|
| `table` | string | Table name (model spelling). |
| `column` | string | Column name (model spelling). |
| `dataType`? | string \| null |  |
| `rows` | integer |  |
| `count` | integer | Non-blank values. |
| `blanks` | integer |  |
| `distinct` | integer | Distinct non-blank values (text compared case-insensitively). |
| `min` | any | Smallest non-blank value (`null` if none). |
| `max` | any |  |
| `minFormatted` | string |  |
| `maxFormatted` | string |  |

Examples:

- Order status: `{"docId":"d1","table":"Orders","column":"Status"}`

### `data.loadCsv`

**Load a CSV.** Replaces a table's data with a CSV ("Load CSV manually…") and recomputes what depends on it.

_mutates the document, long-running, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `table` | string | Table name (case-insensitive). |
| `path` | string | The CSV file. |
| `options` (optional) | CsvParams | CSV reading options. |

Result:

| Field | Type | Description |
|---|---|---|
| `ok` | boolean | Every table and calculated column in scope succeeded. |
| `source` | string | The data source used: `auto`, `query` or `cache` (`manual` for CSV loads). |
| `durationMs` | integer |  |
| `summary` | RefreshCounts | Table counts of a refresh (or of the data loaded so far). |
| `tables` | TableRefreshOut[] | Tables in scope, in model order. |
| `derived` | DerivedColumnOut[] | Session derived columns after the run, with their status. |
| `diagnostics` | DiagnosticOut[] | Refresh diagnostics (cycles, decoder notes, CSV conversion warnings). |
| `generation` | integer | The data generation after the run. |

Examples:

- Orders from a CSV: `{"docId":"d1","table":"Orders","path":"examples/acme-outfitters/data/orders.csv"}`

### `data.addDerivedColumn`

**Add a derived column.** Adds a session-only column (people parsing or a DAX expression), re-applied after every refresh; never saved to the model.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `table` | string | Table name (case-insensitive). |
| `name` | string | New column name (unique in the table). |
| `kind` | DerivedKindParam | How a derived column is computed. |

Result:

| Field | Type | Description |
|---|---|---|
| `table` | string |  |
| `name` | string |  |
| `kind` | string | `people`, `expression` or `array`. |
| `sourceColumn`? | string \| null | The parsed text column (`people`). |
| `dax`? | string \| null | The DAX expression (`expression`). |
| `dataType` | string | Model data type of the values (`string`, `int64`, …). |
| `active` | boolean | The column is in the table data now. |
| `status` | StatusOut | Outcome of one refresh step. |

Examples:

- Parse a people field: `{"docId":"d1","table":"Customers","name":"Account Managers (parsed)","kind":{"people":{"sourceColumn":"Account Managers"}}}`
- A DAX column: `{"docId":"d1","table":"Orders","name":"Is Completed","kind":{"dax":{"expression":"Orders[Status] = \"Completed\""}}}`

### `data.removeDerivedColumn`

**Remove a derived column.** Removes a session derived column from the table data and the session model.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `table` | string | Table name (case-insensitive). |
| `name` | string |  |

Result:

| Field | Type | Description |
|---|---|---|
| `table` | string |  |
| `name` | string |  |
| `kind` | string | `people`, `expression` or `array`. |
| `sourceColumn`? | string \| null | The parsed text column (`people`). |
| `dax`? | string \| null | The DAX expression (`expression`). |
| `dataType` | string | Model data type of the values (`string`, `int64`, …). |
| `active` | boolean | The column is in the table data now. |
| `status` | StatusOut | Outcome of one refresh step. |

Examples:

- Remove a parsed column: `{"docId":"d1","table":"Customers","name":"Account Managers (parsed)"}`

### `security.viewAs`

**View as roles.** Views the data as security roles and/or a user (row-level security, union of roles); empty roles and no user stop it. Visible rows per table.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `roles` | string[] | Roles whose row filters apply (role names, case-insensitive); a row is visible if any role allows it. Empty with no `userName` and no `customData`: stop viewing as (all rows again). |
| `userName` (optional) | string \| null | The effective user ("Other user"): what `USERPRINCIPALNAME()` and `USERNAME()` return, for dynamic roles. Without roles it only sets the user. |
| `customData` (optional) | string \| null | What `CUSTOMDATA()` returns. |

Result:

| Field | Type | Description |
|---|---|---|
| `viewAs` | ViewAsOut | The view now in effect. |
| `tables` | VisibleTableOut[] | Visible rows per table, in model order. |
| `generation` | integer | Data generation after the change (bumped when the view changed: cached results and renders are stale). |

Examples:

- One role: `{"docId":"d1","roles":["East"]}`
- A dynamic role as a user: `{"docId":"d1","roles":["Managers"],"userName":"ann@contoso.com"}`
- Stop viewing as: `{"docId":"d1","roles":[]}`

## Dax

### `dax.query`

**Run a DAX query.** Evaluates a DAX query (EVALUATE …) and returns columns and rows per result table; loads the data on first use.

_long-running, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `query` | string | A DAX query: `EVALUATE …`, with `DEFINE` / `ORDER BY`. |
| `maxRows` (optional) | integer | Rows returned per result table (default 1000, at most 5000). |
| `filters` (optional) | ColumnFilterParam[] | Outer filter context (`table[column] IN values`, combined with AND). |
| `formatted` (optional) | boolean | Cells as formatted text instead of raw values. |
| `blankAsNull` (optional) | boolean | With `formatted`, BLANK cells are `null` instead of `""`. |

Result:

| Field | Type | Description |
|---|---|---|
| `tables` | DaxTableOut[] | One table per `EVALUATE`. |
| `durationMs` | integer |  |

Examples:

- One value: `{"docId":"d1","query":"EVALUATE ROW(\"n\", [Total Sales])"}`
- Grouped, under a filter: `{"docId":"d1","query":"EVALUATE SUMMARIZECOLUMNS(Stores[Region], \"Sales\", [Total Sales])","filters":[{"table":"Stores","column":"Channel","values":["Retail"]}]}`

### `dax.evaluateMeasure`

**Evaluate a measure.** One measure's value (raw and formatted, or ⚠ error with its DAX) under report/page and column filters.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `measure` | string | `Name`, `[Name]` or `Table[Name]`. |
| `report` (optional) | string \| null | Apply this report's report-level filters. |
| `page` (optional) | string \| null | Also apply this page's filters (of `report`, default the first). |
| `filters` (optional) | ColumnFilterParam[] | Extra column filters (AND). |

Result:

| Field | Type | Description |
|---|---|---|
| `measure` | MeasureValueOut | The value of one measure. |
| `filters` | FilterSpecOut | The filters applied, and those that could not be. |

Examples:

- Unfiltered: `{"docId":"d1","measure":"Total Sales"}`
- Under a report's filters: `{"docId":"d1","measure":"[Margin %]","report":"Stores"}`

### `measures.values`

**Measure values.** Every measure's formatted value under optional report/page filters; failed measures show ⚠ and their DAX.

_long-running, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `report` (optional) | string \| null | Evaluate under this report's report-level filters. |
| `page` (optional) | string \| null | Also apply this page's filters. |
| `filters` (optional) | ColumnFilterParam[] | Extra column filters (AND). |

Result:

| Field | Type | Description |
|---|---|---|
| `ok` | boolean | Every measure evaluated without error. |
| `report`? | string \| null | Report name. |
| `page`? | string \| null | Page display name. |
| `filters` | FilterSpecOut | Filters applied, and those that could not be (Top N, relative date…). |
| `data` | RefreshCounts | What the values were computed on (tables loaded, failed, never loaded). |
| `measures` | MeasureValueOut[] | Every measure, in model order; failures carry `error` and `⚠` in `display`, with their DAX. |

Examples:

- Unfiltered: `{"docId":"d1"}`
- Under the Stores report's filters: `{"docId":"d1","report":"Stores"}`

## Render

### `render.page`

**Render a page.** Renders a report page to SVG or PNG (inline, base64 for PNG, or to outputPath) with per-visual status; loads the data on first use.

_long-running, agent-safe, writes files; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `report` (optional) | string \| null | Report (default: the first). |
| `page` (optional) | string \| null | Page: internal name, display name, 1-based index or a unique part of the display name (default: the active page). |
| `format` (optional) | `"svg"` \| `"png"` | `svg` (default) or `png`. |
| `scale` (optional) | number \| null | PNG pixels per page unit (default 1; greater than 0, at most 16). |
| `slicers` (optional) | SlicerParam[] | Slicer selections replacing the saved ones. |
| `outputPath` (optional) | string \| null | Write the output to this file instead of returning it inline. |
| `overwrite` (optional) | boolean | Replace an existing `outputPath`. |

Result:

| Field | Type | Description |
|---|---|---|
| `ok` | boolean | No visual failed and the data loaded. |
| `report` | string |  |
| `page` | RenderedPageInfo | The rendered page. |
| `visual`? | string \| null | The rendered visual (`render.visual`). |
| `format` | `"svg"` \| `"png"` | `svg` or `png`. |
| `svg`? | string \| null | The SVG document (inline SVG renders). |
| `pngBase64`? | string \| null | The PNG, base64 (inline PNG renders). |
| `outputPath`? | string \| null | The written file (with `outputPath`). |
| `bytes` | integer | Size of the SVG or PNG in bytes. |
| `pixelWidth`? | integer \| null | PNG size in pixels. |
| `pixelHeight`? | integer \| null |  |
| `summary` | VisualCounts | Visual counts of a render. |
| `visuals` | VisualStatusOut[] | Every visual of the page (or the one visual), in page order. |
| `slicers` | SlicerOut[] | Slicer selections applied. |
| `data`? | RefreshCounts \| null | What the data looked like (failed or never-loaded tables). |
| `durationMs` | integer | Query + render + encode wall time. |

Examples:

- Active page as SVG: `{"docId":"d1"}`
- A page as PNG with a slicer selection: `{"docId":"d1","report":"Overview","page":"1","format":"png","scale":1,"slicers":[{"field":"Region","values":["West"]}]}`
- To a file: `{"docId":"d1","report":"Stores","format":"png","scale":2,"outputPath":"target/tmp/stores.png","overwrite":true}`

### `render.visual`

**Render a visual.** Renders one visual of a page, cropped to its container, to SVG or PNG (with the page's filters, slicers and interactions).

_long-running, agent-safe, writes files; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `report` (optional) | string \| null | Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report. |
| `page` (optional) | string \| null | Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. |
| `visual` | string | The visual's name (see `report.visuals`). |
| `format` (optional) | `"svg"` \| `"png"` | Output format. |
| `scale` (optional) | number \| null |  |
| `slicers` (optional) | SlicerParam[] |  |
| `outputPath` (optional) | string \| null |  |
| `overwrite` (optional) | boolean |  |

Result:

| Field | Type | Description |
|---|---|---|
| `ok` | boolean | No visual failed and the data loaded. |
| `report` | string |  |
| `page` | RenderedPageInfo | The rendered page. |
| `visual`? | string \| null | The rendered visual (`render.visual`). |
| `format` | `"svg"` \| `"png"` | `svg` or `png`. |
| `svg`? | string \| null | The SVG document (inline SVG renders). |
| `pngBase64`? | string \| null | The PNG, base64 (inline PNG renders). |
| `outputPath`? | string \| null | The written file (with `outputPath`). |
| `bytes` | integer | Size of the SVG or PNG in bytes. |
| `pixelWidth`? | integer \| null | PNG size in pixels. |
| `pixelHeight`? | integer \| null |  |
| `summary` | VisualCounts | Visual counts of a render. |
| `visuals` | VisualStatusOut[] | Every visual of the page (or the one visual), in page order. |
| `slicers` | SlicerOut[] | Slicer selections applied. |
| `data`? | RefreshCounts \| null | What the data looked like (failed or never-loaded tables). |
| `durationMs` | integer | Query + render + encode wall time. |

Examples:

- One visual as PNG: `{"docId":"d1","page":"1","visual":"f7a9c9413fef68532313","format":"png","scale":2}`

## Canvas

### `canvas.render`

**Render the canvas.** Renders a page with its current interaction state: SVG, hit regions, visuals, slicer/cross-filter selections and effective filters.

_mutates the document, long-running, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `report` (optional) | string \| null | Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report. |
| `page` (optional) | string \| null | Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. |
| `viewport` (optional) | ViewportParam \| null | Fit the page into this viewport (kept for later renders). |

Result:

| Field | Type | Description |
|---|---|---|
| `docId` | string |  |
| `report` | string | Report name. |
| `page` | string | Page internal name. |
| `pageDisplayName` | string |  |
| `generation` | integer | Render number of this page (increasing): drop older responses. |
| `svg` | string | The page as a standalone SVG document (deterministic; `data-hit` attributes on interactive groups). |
| `overlaySvg` | string \| null | Open slicer dropdown popups, drawn above the page; `null` when none is open. |
| `width` | number |  |
| `height` | number |  |
| `hitRegions` | HitRegionOut[] | Hit regions in paint order (hit-test from the end). |
| `overlayHitRegions` | HitRegionOut[] | Hit regions of the overlay (above every page region). |
| `visuals` | CanvasVisualOut[] | Every visual of the page (hidden ones included), in page order. |
| `interactions` | CanvasInteractionOut[] |  |
| `selections` | SelectionOut[] | Slicer selections and the cross-filter. |
| `effectiveFilters` | FilterSpecOut | Report + page filters + selections as a filter spec (what cards are evaluated in, ignoring per-visual interactions). |
| `isDefault` | boolean | The state equals the report's saved state. |
| `warnings` | string[] | Unreadable saved slicer selections. |
| `durationMs` | integer | Query + render wall time. |

Examples:

- Active page: `{"docId":"d1"}`
- Fit a viewport: `{"docId":"d1","report":"Stores","viewport":{"width":1280,"height":720}}`

### `canvas.click`

**Click on the canvas.** Applies a click on a hit region (slicer item, data point, button, dropdown) and returns the re-rendered page and any button action.

_mutates the document, long-running, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `report` (optional) | string \| null | Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report. |
| `page` (optional) | string \| null | Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. |
| `hitId` (optional) | string \| null | A hit id from the last render (`hitRegions[].id`); `null` clicks the empty canvas (clears the cross-filter, closes dropdowns). |
| `ctrl` (optional) | boolean | Ctrl (⌘) held: add to / remove from the selection. |
| `shift` (optional) | boolean |  |
| `renderUnchanged` (optional) | boolean | Return the page even when nothing changed (default `true`). |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean | The interaction or view state changed. |
| `page` | CanvasPageOut \| null | The re-rendered page (also when unchanged, unless `renderUnchanged` is `false`). |
| `action`? | CanvasAction \| null | What a clicked button asks the caller to do. |

Examples:

- Select a slicer item: `{"docId":"d1","hitId":"9a5c3f0e1b|slicerItem|1"}`
- Ctrl+click a bar: `{"docId":"d1","page":"1","hitId":"1f2e3d|dataPoint|0|2","ctrl":true}`

### `canvas.scroll`

**Scroll a visual.** Scrolls a table, matrix or slicer list by a delta (clamped to its range) and returns the re-rendered page.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `report` (optional) | string \| null | Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report. |
| `page` (optional) | string \| null | Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. |
| `visual` | string | The table, matrix or slicer to scroll. |
| `dx` (optional) | number | Horizontal delta in page units. |
| `dy` (optional) | number | Vertical delta in page units (positive scrolls down). |
| `renderUnchanged` (optional) | boolean | Return the page even when nothing changed (default `true`). |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean | The interaction or view state changed. |
| `page` | CanvasPageOut \| null | The re-rendered page (also when unchanged, unless `renderUnchanged` is `false`). |
| `action`? | CanvasAction \| null | What a clicked button asks the caller to do. |

Examples:

- Scroll a table down: `{"docId":"d1","page":"1","visual":"a1b2c3","dy":200}`

### `canvas.setSlicer`

**Set a slicer.** Replaces a slicer's selection by value (no hit ids needed) and returns the re-rendered page.

_mutates the document, long-running, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `report` (optional) | string \| null | Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report. |
| `page` (optional) | string \| null | Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. |
| `visual` (optional) | string \| null | The slicer's visual name… |
| `field` (optional) | string \| null | …or its field (display name, column, `Table[Column]`). |
| `values` (optional) | any[] | Values to select (`null` = BLANK); empty without `invert` selects all. |
| `invert` (optional) | boolean | Select everything except `values`. |

Result:

| Field | Type | Description |
|---|---|---|
| `docId` | string |  |
| `report` | string | Report name. |
| `page` | string | Page internal name. |
| `pageDisplayName` | string |  |
| `generation` | integer | Render number of this page (increasing): drop older responses. |
| `svg` | string | The page as a standalone SVG document (deterministic; `data-hit` attributes on interactive groups). |
| `overlaySvg` | string \| null | Open slicer dropdown popups, drawn above the page; `null` when none is open. |
| `width` | number |  |
| `height` | number |  |
| `hitRegions` | HitRegionOut[] | Hit regions in paint order (hit-test from the end). |
| `overlayHitRegions` | HitRegionOut[] | Hit regions of the overlay (above every page region). |
| `visuals` | CanvasVisualOut[] | Every visual of the page (hidden ones included), in page order. |
| `interactions` | CanvasInteractionOut[] |  |
| `selections` | SelectionOut[] | Slicer selections and the cross-filter. |
| `effectiveFilters` | FilterSpecOut | Report + page filters + selections as a filter spec (what cards are evaluated in, ignoring per-visual interactions). |
| `isDefault` | boolean | The state equals the report's saved state. |
| `warnings` | string[] | Unreadable saved slicer selections. |
| `durationMs` | integer | Query + render wall time. |

Examples:

- Only the West region: `{"docId":"d1","field":"Region","values":["West"]}`
- Clear a slicer: `{"docId":"d1","field":"Region","values":[]}`

### `canvas.clearSelections`

**Clear selections.** Clears a slicer's selection ("Clear selections") or the page's cross-filter, and returns the re-rendered page.

_mutates the document, long-running, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `report` (optional) | string \| null | Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report. |
| `page` (optional) | string \| null | Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. |
| `visual` (optional) | string \| null | A slicer (clears its selection) or the cross-filter source; absent: clears the cross-filter and closes the dropdowns. |

Result:

| Field | Type | Description |
|---|---|---|
| `docId` | string |  |
| `report` | string | Report name. |
| `page` | string | Page internal name. |
| `pageDisplayName` | string |  |
| `generation` | integer | Render number of this page (increasing): drop older responses. |
| `svg` | string | The page as a standalone SVG document (deterministic; `data-hit` attributes on interactive groups). |
| `overlaySvg` | string \| null | Open slicer dropdown popups, drawn above the page; `null` when none is open. |
| `width` | number |  |
| `height` | number |  |
| `hitRegions` | HitRegionOut[] | Hit regions in paint order (hit-test from the end). |
| `overlayHitRegions` | HitRegionOut[] | Hit regions of the overlay (above every page region). |
| `visuals` | CanvasVisualOut[] | Every visual of the page (hidden ones included), in page order. |
| `interactions` | CanvasInteractionOut[] |  |
| `selections` | SelectionOut[] | Slicer selections and the cross-filter. |
| `effectiveFilters` | FilterSpecOut | Report + page filters + selections as a filter spec (what cards are evaluated in, ignoring per-visual interactions). |
| `isDefault` | boolean | The state equals the report's saved state. |
| `warnings` | string[] | Unreadable saved slicer selections. |
| `durationMs` | integer | Query + render wall time. |

Examples:

- Clear the cross-filter: `{"docId":"d1"}`
- Clear one slicer: `{"docId":"d1","visual":"9a5c3f0e1b"}`

### `canvas.setSlicerSearch`

**Search a slicer.** Filters a slicer's list (or open dropdown) by a search text and returns the re-rendered page.

_mutates the document, long-running, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `report` (optional) | string \| null | Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report. |
| `page` (optional) | string \| null | Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. |
| `visual` | string | The slicer. |
| `text` | string | Search text (at most 256 characters; empty clears it). |

Result:

| Field | Type | Description |
|---|---|---|
| `docId` | string |  |
| `report` | string | Report name. |
| `page` | string | Page internal name. |
| `pageDisplayName` | string |  |
| `generation` | integer | Render number of this page (increasing): drop older responses. |
| `svg` | string | The page as a standalone SVG document (deterministic; `data-hit` attributes on interactive groups). |
| `overlaySvg` | string \| null | Open slicer dropdown popups, drawn above the page; `null` when none is open. |
| `width` | number |  |
| `height` | number |  |
| `hitRegions` | HitRegionOut[] | Hit regions in paint order (hit-test from the end). |
| `overlayHitRegions` | HitRegionOut[] | Hit regions of the overlay (above every page region). |
| `visuals` | CanvasVisualOut[] | Every visual of the page (hidden ones included), in page order. |
| `interactions` | CanvasInteractionOut[] |  |
| `selections` | SelectionOut[] | Slicer selections and the cross-filter. |
| `effectiveFilters` | FilterSpecOut | Report + page filters + selections as a filter spec (what cards are evaluated in, ignoring per-visual interactions). |
| `isDefault` | boolean | The state equals the report's saved state. |
| `warnings` | string[] | Unreadable saved slicer selections. |
| `durationMs` | integer | Query + render wall time. |

Examples:

- Search: `{"docId":"d1","visual":"9a5c3f0e1b","text":"crit"}`

### `canvas.toggleMatrixRow`

**Expand/collapse a matrix row.** Expands or collapses the matrix row of a cell hit and returns the re-rendered page.

_mutates the document, long-running, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `report` (optional) | string \| null | Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report. |
| `page` (optional) | string \| null | Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. |
| `hitId` | string | A hit id from the last render. |

Result:

| Field | Type | Description |
|---|---|---|
| `docId` | string |  |
| `report` | string | Report name. |
| `page` | string | Page internal name. |
| `pageDisplayName` | string |  |
| `generation` | integer | Render number of this page (increasing): drop older responses. |
| `svg` | string | The page as a standalone SVG document (deterministic; `data-hit` attributes on interactive groups). |
| `overlaySvg` | string \| null | Open slicer dropdown popups, drawn above the page; `null` when none is open. |
| `width` | number |  |
| `height` | number |  |
| `hitRegions` | HitRegionOut[] | Hit regions in paint order (hit-test from the end). |
| `overlayHitRegions` | HitRegionOut[] | Hit regions of the overlay (above every page region). |
| `visuals` | CanvasVisualOut[] | Every visual of the page (hidden ones included), in page order. |
| `interactions` | CanvasInteractionOut[] |  |
| `selections` | SelectionOut[] | Slicer selections and the cross-filter. |
| `effectiveFilters` | FilterSpecOut | Report + page filters + selections as a filter spec (what cards are evaluated in, ignoring per-visual interactions). |
| `isDefault` | boolean | The state equals the report's saved state. |
| `warnings` | string[] | Unreadable saved slicer selections. |
| `durationMs` | integer | Query + render wall time. |

Examples:

- Toggle a row: `{"docId":"d1","hitId":"4e5f6a|dataPoint|0|3"}`

### `canvas.closePopups`

**Close dropdowns.** Closes the open slicer dropdowns of a page.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `report` (optional) | string \| null | Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report. |
| `page` (optional) | string \| null | Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. |
| `renderUnchanged` (optional) | boolean | Return the page even when nothing changed (default `true`). |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean | The interaction or view state changed. |
| `page` | CanvasPageOut \| null | The re-rendered page (also when unchanged, unless `renderUnchanged` is `false`). |
| `action`? | CanvasAction \| null | What a clicked button asks the caller to do. |

Examples:

- Close: `{"docId":"d1","renderUnchanged":false}`

### `canvas.reset`

**Reset to default.** Back to the report's saved slicer selections, no cross-filter, scroll and dropdowns reset; returns the re-rendered page.

_mutates the document, long-running, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `report` (optional) | string \| null | Report (default: the first). |
| `page` (optional) | string \| null | Page (default: the active page). |

Result:

| Field | Type | Description |
|---|---|---|
| `docId` | string |  |
| `report` | string | Report name. |
| `page` | string | Page internal name. |
| `pageDisplayName` | string |  |
| `generation` | integer | Render number of this page (increasing): drop older responses. |
| `svg` | string | The page as a standalone SVG document (deterministic; `data-hit` attributes on interactive groups). |
| `overlaySvg` | string \| null | Open slicer dropdown popups, drawn above the page; `null` when none is open. |
| `width` | number |  |
| `height` | number |  |
| `hitRegions` | HitRegionOut[] | Hit regions in paint order (hit-test from the end). |
| `overlayHitRegions` | HitRegionOut[] | Hit regions of the overlay (above every page region). |
| `visuals` | CanvasVisualOut[] | Every visual of the page (hidden ones included), in page order. |
| `interactions` | CanvasInteractionOut[] |  |
| `selections` | SelectionOut[] | Slicer selections and the cross-filter. |
| `effectiveFilters` | FilterSpecOut | Report + page filters + selections as a filter spec (what cards are evaluated in, ignoring per-visual interactions). |
| `isDefault` | boolean | The state equals the report's saved state. |
| `warnings` | string[] | Unreadable saved slicer selections. |
| `durationMs` | integer | Query + render wall time. |

Examples:

- Reset the active page: `{"docId":"d1"}`

### `canvas.tooltip`

**Data point tooltip.** The hover tooltip (category, series, values) of a data point hit, from the last render.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `report` (optional) | string \| null | Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report. |
| `page` (optional) | string \| null | Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. |
| `hitId` | string | A hit id from the last render. |

Result:

| Field | Type | Description |
|---|---|---|
| `lines` | TooltipLineOut[] | Label/value lines; empty when the hit has no tooltip. |

Examples:

- A bar's tooltip: `{"docId":"d1","hitId":"1f2e3d|dataPoint|0|2"}`

### `canvas.state`

**Canvas state.** The page's current interaction state (slicer selections, cross-filter) and view state, as JSON.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `report` (optional) | string \| null | Report (default: the first). |
| `page` (optional) | string \| null | Page (default: the active page). |

Result:

| Field | Type | Description |
|---|---|---|
| `interaction` | any | Slicer selections and selected data points (`InteractionState`). |
| `view` | any | Scroll, expansion and dropdown state per visual (`PageViewState`). |
| `isDefault` | boolean | The interaction state equals the report's saved state. |

Examples:

- State of the active page: `{"docId":"d1"}`

## Export

### `export.table`

**Export a table.** Writes one table's data as CSV or Parquet (with model metadata); loads the data on first use.

_long-running, agent-safe, writes files; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `table` | string | Table name (case-insensitive). |
| `format` | `"csv"` \| `"parquet"` | Table export format. |
| `path` | string | The output file. |
| `formatted` (optional) | boolean | CSV: values formatted with each column's format string (as Power BI shows them) instead of raw values. |
| `excelSafe` (optional) | boolean | CSV: prefix text cells starting with `=`, `+`, `-`, `@`, TAB or CR with `'` (spreadsheet formula-injection guard). |
| `bom` (optional) | boolean | CSV: start with a UTF-8 BOM. |
| `delimiter` (optional) | string \| null | CSV: field delimiter (default `,`). |
| `compression` (optional) | `"zstd"` \| `"snappy"` \| `"none"` | Parquet: compression (default `zstd`). |
| `overwrite` (optional) | boolean | Replace an existing file. |

Result:

| Field | Type | Description |
|---|---|---|
| `table` | string | Table name (model spelling). |
| `format` | `"csv"` \| `"parquet"` | Table export format. |
| `path` | string |  |
| `rows` | integer | Data rows (without a CSV header). |
| `columns` | integer |  |
| `bytes` | integer | File size. |

Examples:

- CSV for Excel: `{"docId":"d1","table":"Orders","format":"csv","path":"target/tmp/orders.csv","formatted":true,"excelSafe":true,"bom":true}`
- Parquet: `{"docId":"d1","table":"Returns","format":"parquet","path":"target/tmp/returns.parquet","overwrite":true}`

### `export.model`

**Document the model.** The model documentation as JSON (model doc) or a Markdown data dictionary, inline or to a file; no data needed.

_agent-safe, writes files; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `format` | `"json"` \| `"markdown"` | Model documentation format. |
| `path` (optional) | string \| null | Write to this file; without it the documentation is returned inline. |
| `includeExpressions` (optional) | boolean \| null | Include DAX and M (default `true`). |
| `includeHidden` (optional) | boolean \| null | Include hidden objects (default `true`). |
| `title` (optional) | string \| null | Markdown title (default: the file or project name). |
| `overwrite` (optional) | boolean | Replace an existing file. |

Result:

| Field | Type | Description |
|---|---|---|
| `format` | `"json"` \| `"markdown"` | Model documentation format. |
| `path`? | string \| null | The written file. |
| `bytes` | integer | Size of the documentation. |
| `json`? | any | The model doc (inline JSON; shape in docs/CLI.md, `export --model`). |
| `markdown`? | string \| null | The data dictionary (inline Markdown). |

Examples:

- Inline Markdown: `{"docId":"d1","format":"markdown"}`
- JSON to a file, no DAX/M: `{"docId":"d1","format":"json","path":"target/tmp/model.json","includeExpressions":false,"overwrite":true}`

## Convert

### `convert.pbip`

**Save as PBIP.** Writes the document as a PBIP project (TMDL + PBIR) into a folder.

_long-running, agent-safe, writes files; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `outDir` | string | Output folder (created if missing; must be empty unless `overwrite`). |
| `overwrite` (optional) | boolean | Write into a non-empty folder (files with the same paths are replaced). |

Result:

| Field | Type | Description |
|---|---|---|
| `outDir` | string |  |
| `files` | string[] | Files written, relative to `outDir`. |
| `bytes` | integer |  |

Examples:

- To a new folder: `{"docId":"d1","outDir":"target/tmp/project"}`

### `convert.pbit`

**Export a PBIT template.** Writes a PBIT template (model schema and one report, no data).

_long-running, agent-safe, writes files; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `outPath` | string | Output `.pbit` file. |
| `report` (optional) | string \| null | The report to include (a PBIT holds one; exact name, default the first). |
| `overwrite` (optional) | boolean | Replace an existing file. |

Result:

| Field | Type | Description |
|---|---|---|
| `outPath` | string |  |
| `report` | string | The report written. |
| `omittedReports` | string[] | Reports left out (a PBIT holds one). |
| `bytes` | integer |  |

Examples:

- One report: `{"docId":"d1","outPath":"target/tmp/stores.pbit","report":"Acme Outfitters - Stores","overwrite":true}`

## Transform

### `text.parsePeople`

**Parse a people field.** Parses one people-picker cell (a Python-style list of dicts) into `First Last` names (the "Parse people field" transform).

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `text` | string | One exported people-picker cell: a Python-style list of dicts, e.g. `[{'firstName': 'Ada', 'lastName': 'Lovelace'}]`. |

Result:

| Field | Type | Description |
|---|---|---|
| `outcome` | string | `names` (a people list), `empty` (blank or an empty list) or `unparsed` (not a people list; `display` is the trimmed text). |
| `names` | string[] | `First Last` per person. |
| `display` | string \| null | What the "Parse people field" column shows: names joined with `; `. |

Examples:

- Two people: `{"text":"[{'firstName': 'Ada', 'lastName': 'Lovelace'}, {'firstName': 'Alan', 'lastName': 'Turing'}]"}`

### `text.previewPeople`

**Preview "Parse people field".** Parses every cell of a text column as a people list: outcome counts and distinct samples with their parsed form (before data.addDerivedColumn).

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `table` | string | Table name (case-insensitive). |
| `column` | string | A text column of the table (case-insensitive). |
| `limit` (optional) | integer | Distinct samples returned (default 50, at most 200). |

Result:

| Field | Type | Description |
|---|---|---|
| `table` | string | Table name (model spelling). |
| `column` | string | Column name (stored spelling). |
| `newColumn` | string | The name `data.addDerivedColumn` should use: `<column> (parsed)`. |
| `rows` | integer |  |
| `names` | integer | Cells parsed into names. |
| `unparsed` | integer | Non-empty cells that are not a people list (shown trimmed). |
| `empty` | integer | Blank cells and empty lists. |
| `samples` | PeopleSampleOut[] | Distinct non-empty values in storage order, with their parsed form. |

Examples:

- Account managers: `{"docId":"d1","table":"Customers","column":"Account Managers","limit":20}`
