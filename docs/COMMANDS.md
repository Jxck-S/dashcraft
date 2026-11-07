<!-- Generated from the command registry (crates/pbi-ops). Do not edit by hand:
     cargo run -q -p dashcraft-cli --bin dashcraft -- commands --markdown > docs/COMMANDS.md
     (the pbi-ops test `commands_md_is_up_to_date` fails when this file is stale). -->

# Command catalog

Every DashCraft capability is a **command**: a stable dotted id, typed parameters and a typed result, both described by JSON Schema (draft 2020-12). The CLI (`dashcraft run`, `dashcraft serve`, `dashcraft mcp`), the desktop app and AI agents (MCP) all run the same commands through one registry (`pbi-ops`, [ADR-0007](adr/0007-command-registry.md)).

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
- Documents are opened with `doc.open`, which returns a `docId` (`d1`, `d2`, …) for the other commands. Data commands (`data.page`, `dax.query`, `dax.evaluateMeasure`, `measures.values`, `render.*`, `canvas.*`, `export.table`) load the data on first use unless the document was opened with `"autoRefresh": false`; `data.refresh` reloads it.
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
- Every committed edit sends `{"method":"doc/changed","params":{"docId","command","revision","dirty","changes":[{"objectId","kind":"added|changed|renamed|removed","oldName"?,"newName"?}],"path"?}}` before its response: one per command, also per `commands.batch` step and for notifications. A save that clears the dirty flag sends one with no changes (`doc.saveAs` adds the new `path`); no-op and failed edits send none.
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
| `conflict` | -32008 | 1 | Files changed on disk since the open or last save; nothing was written (`data.conflicts`) |

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
| [`doc.close`](#docclose) | Closes an open document and frees its data; unsaved edits require explicit discardChanges. | M A |
| [`doc.list`](#doclist) | The open documents of the workspace with their summaries. | A |
| [`doc.inspect`](#docinspect) | The full model, reports and reader diagnostics: the `dashcraft inspect --json` schema (schemaVersion 1). | A |
| [`model.tables`](#modeltables) | Every model table with its columns, measure names, partitions, hierarchies and loaded row count. | A |
| [`model.measures`](#modelmeasures) | Every measure with its table, DAX, format string, description and display folder. | A |
| [`model.relationships`](#modelrelationships) | Every relationship: id, from/to columns, cardinalities, cross-filter direction, active flag. | A |
| [`report.list`](#reportlist) | The document's reports: name, dataset reference, theme, page and visual counts, report filters. | A |
| [`report.pages`](#reportpages) | The pages of a report: names, size, visibility, visual and filter counts. | A |
| [`report.visuals`](#reportvisuals) | The visuals of a page: type, title, position, bound fields and visual filters. | A |
| [`report.filters`](#reportfilters) | A report's filters (plus a page's) translated to column filters, with the ones that cannot be applied (Top N, relative date…). | A |
| [`security.roles`](#securityroles) | The model's row-level security roles (permission, DAX table filters, members, dynamic or not) and the current "View as" state. | A |
| [`model.setMeasureExpression`](#modelsetmeasureexpression) | Replaces a measure's DAX expression as one undoable document edit. | M A |
| [`model.object.rename`](#modelobjectrename) | Renames a table, column, measure or M query/parameter as one undoable edit; DAX, M and report references follow and loaded data is kept. dryRun previews it. | M A |
| [`model.object.setDescription`](#modelobjectsetdescription) | Sets or clears the description of a table, column, measure or other model object as one undoable edit. | M A |
| [`model.measure.create`](#modelmeasurecreate) | Adds a DAX measure to a table as one undoable edit and returns its id. | M A |
| [`model.measure.update`](#modelmeasureupdate) | Changes a measure's expression, format string, display folder, description or visibility as one undoable edit. | M A |
| [`model.measure.delete`](#modelmeasuredelete) | Deletes a measure as one undoable edit and lists what referred to it (DAX, report fields); dryRun only lists them. | M A |
| [`model.relationship.create`](#modelrelationshipcreate) | Adds a relationship between two columns as one undoable edit; filters follow it at once and warnings report what the engine finds in the loaded data. | M A |
| [`model.relationship.update`](#modelrelationshipupdate) | Changes a relationship's cardinality, cross-filter direction, security filtering or active state as one undoable edit; filters follow it at once. | M A |
| [`model.relationship.delete`](#modelrelationshipdelete) | Deletes a relationship as one undoable edit and lists the DAX that names it (USERELATIONSHIP, CROSSFILTER); dryRun only lists it. | M A |
| [`model.column.create`](#modelcolumncreate) | Adds a DAX calculated column to a table as one undoable edit, computes it over the loaded data and infers its data type unless dataType is given. | M A |
| [`model.column.update`](#modelcolumnupdate) | Changes a calculated column's expression or data type, or a column's format, folder, description or visibility; what depends on it is recomputed over the loaded data. | M A |
| [`model.column.delete`](#modelcolumndelete) | Deletes a calculated column as one undoable edit and lists what referred to it; refuses data columns and columns used by relationships, hierarchies or sort orders. dryRun only checks. | M A |
| [`model.table.create`](#modeltablecreate) | Adds a DAX calculated table as one undoable edit: its columns and their data types come from the expression's result over the loaded data, and it is computed at once. | M A |
| [`model.table.update`](#modeltableupdate) | Changes a calculated table's DAX expression, its columns following the result, or a table's description or visibility; the table and what depends on it are recomputed. | M A |
| [`model.table.delete`](#modeltabledelete) | Deletes a table with its columns, measures, relationships and role row filters as one undoable edit and lists what referred to it (DAX, report fields); dryRun only lists them. | M A |
| [`model.hierarchy.create`](#modelhierarchycreate) | Adds a user hierarchy over columns of a table as one undoable edit; levels are listed top first and named after their columns unless named. | M A |
| [`model.hierarchy.delete`](#modelhierarchydelete) | Deletes a user hierarchy with its levels as one undoable edit and lists the report fields that used it; dryRun only lists them. | M A |
| [`model.dependencies`](#modeldependencies) | Lists what refers to a table, column, measure or relationship: DAX, relationships, hierarchies, sort-by columns, roles and report fields. | A |
| [`report.visual.move`](#reportvisualmove) | Moves a visual or a visual group (with the visuals in it) on its page as one undoable edit. | M A |
| [`report.visual.resize`](#reportvisualresize) | Resizes a visual or a visual group (top-left corner fixed; the visuals in a group scale with it unless it scrolls) as one undoable edit. | M A |
| [`doc.applyEdits`](#docapplyedits) | Applies a group of document edits atomically as one undoable history step. | M A |
| [`doc.undo`](#docundo) | Undoes the latest document edit. | M A |
| [`doc.redo`](#docredo) | Reapplies the latest undone document edit. | M A |
| [`doc.history`](#dochistory) | Returns the document revision, dirty state and undo/redo availability. | A |
| [`doc.save`](#docsave) | Saves the edits into the opened PBIP project, writing only changed files; files changed on disk since the open are a conflict unless overwrite. | M A W |
| [`doc.saveAs`](#docsaveas) | Saves the document, edits included, as a new PBIP project from any format and continues editing there: later doc.save writes to it; imported data is not carried over. | M L A W |
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
| [`dax.diagnostics`](#daxdiagnostics) | Syntax errors, unknown tables, columns and measures (with a did-you-mean suggestion), wrong argument counts and functions DashCraft cannot evaluate, for a DAX expression or query; no data is loaded. | A |
| [`dax.signatureHelp`](#daxsignaturehelp) | The DAX function call around a cursor: its signature, parameters, return type, description and the parameter the cursor is in; works on text that does not parse yet. | A |
| [`dax.hover`](#daxhover) | Describes the DAX function, table, column, measure, variable or keyword at a cursor: signature and description, data type, expression, format string; no data is loaded. | A |
| [`dax.complete`](#daxcomplete) | The DAX functions, tables, columns, measures, variables and keywords that fit at a cursor, best match first, with the text to insert; works on text that does not parse yet. | A |
| [`dax.tokens`](#daxtokens) | What every token of a DAX text is (keyword, function, table, column, measure, variable, parameter, literal, comment) for highlighting, with declarations and unknown names marked. | A |
| [`dax.references`](#daxreferences) | The symbol at a cursor in a DAX text, where the text declares it and every place it occurs (go to definition, find references); model objects carry their objectId. | A |
| [`dax.format`](#daxformat) | Lays out a DAX expression or query within a line width, keeping comments and meaning (only whitespace and keyword/function case change); idempotent, no data is loaded. | A |
| [`m.diagnostics`](#mdiagnostics) | Syntax errors, unknown names, wrong argument counts and unsupported names in an M query or section, with Power BI Desktop's messages. | A |
| [`m.hover`](#mhover) | Describes the M name at a cursor as Power BI Desktop binds it: step, parameter or field of the text, query, parameter or table of the model, library name or keyword. | A |
| [`m.signatureHelp`](#msignaturehelp) | The M function call around a cursor: library function, #date-style constructor, function step or function query, its parameters and the one the cursor is in; works on unparsed text. | A |
| [`m.complete`](#mcomplete) | The M names that fit at a cursor, best match first, with the text to insert: steps and parameters in scope, the model's queries, keywords, library names, or types after as. | A |
| [`m.format`](#mformat) | Lays out an M query or section within a line width, one let step per line, keeping comments and meaning (only whitespace changes); idempotent, nothing is evaluated. | A |
| [`tmdl.diagnostics`](#tmdldiagnostics) | What Microsoft's TMDL reader rejects in TMDL files: unknown keywords, wrong forms and values, levels, duplicates and unresolved references, with the closest name. | A |
| [`tmdl.complete`](#tmdlcomplete) | The TMDL keywords, values and object names that fit at a cursor, as Microsoft's TMDL reader accepts them there, best match first, with the text to insert. | A |
| [`tmdl.hover`](#tmdlhover) | Describes the TMDL word at a cursor: a keyword's forms, value and level, an enumeration value, or the object a reference names and where it is declared. | A |
| [`tmdl.format`](#tmdlformat) | Lays out a TMDL file as Power BI Desktop writes it: indentation, spacing, blank lines, expression blocks; only white space changes, checked by reading the result back. | A |
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
| [`convert.pbip`](#convertpbip) | Writes the document, unsaved edits included, as a PBIP project (TMDL + PBIR) into a folder; the open document keeps its source (doc.saveAs moves it). | L A W |
| [`convert.pbit`](#convertpbit) | Writes a PBIT template (model schema and one report, no data), unsaved edits included. | L A W |
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
| `category` (optional) | `"meta"` \| `"workspace"` \| `"inspect"` \| `"lint"` \| `"data"` \| `"dax"` \| `"m"` \| `"tmdl"` \| `"render"` \| `"canvas"` \| `"export"` \| `"convert"` \| `"transform"` \| `"edit"` \| `"ai"` \| null | Only this category. |
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
| `category` | `"meta"` \| `"workspace"` \| `"inspect"` \| `"lint"` \| `"data"` \| `"dax"` \| `"m"` \| `"tmdl"` \| `"render"` \| `"canvas"` \| `"export"` \| `"convert"` \| `"transform"` \| `"edit"` \| `"ai"` | What a command is about; groups the catalog. |
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

**Close a document.** Closes an open document and frees its data; unsaved edits require explicit discardChanges.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `discardChanges` (optional) | boolean | Explicitly discard unsaved in-memory edits. |

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

**Model relationships.** Every relationship: id, from/to columns, cardinalities, cross-filter direction, active flag.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id returned by `doc.open` (`d1`, …). |

Result:

| Field | Type | Description |
|---|---|---|
| `relationships` | ModelRelationshipOut[] |  |

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

### `model.dependencies`

**Object dependents.** Lists what refers to a table, column, measure or relationship: DAX, relationships, hierarchies, sort-by columns, roles and report fields.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `objectId` (optional) | integer \| null | The object's id (instead of `table`): also a relationship's id from `model.relationships`. |
| `table` (optional) | string \| null | The table, or the table of `column` / `measure`. |
| `column` (optional) | string \| null | A column of `table`. |
| `measure` (optional) | string \| null | A measure of `table`. |

Result:

| Field | Type | Description |
|---|---|---|
| `objectId` | integer |  |
| `kind` | string | `table`, `column`, `measure` or `relationship`. |
| `table` | string | The table, or the table of the column or measure (the from table of a relationship). |
| `name` | string |  |
| `revision` | integer | The document revision the list was computed on. |
| `dependents` | DependentOut[] | What refers to the object: DAX expressions, relationships, hierarchy levels, sort-by columns, role permissions, report, page and visual fields and filters. |
| `warnings` | string[] | References that cannot be confirmed (DAX that does not parse, ambiguous unqualified names, M queries naming a table). |

Examples:

- What uses a measure: `{"docId":"d1","table":"Orders","measure":"Total Sales"}`
- What uses a column: `{"docId":"d1","table":"Orders","column":"Line Amount"}`

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
| `documentRevision` | integer | Semantic document revision captured with the query engine snapshot. |
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
| `documentRevision` | integer | Semantic document revision captured with the query engine snapshot. |
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
| `documentRevision` | integer | Semantic document revision captured with the query engine snapshot. |
| `ok` | boolean | Every measure evaluated without error. |
| `report`? | string \| null | Report name. |
| `page`? | string \| null | Page display name. |
| `filters` | FilterSpecOut | Filters applied, and those that could not be (Top N, relative date…). |
| `data` | RefreshCounts | What the values were computed on (tables loaded, failed, never loaded). |
| `measures` | MeasureValueOut[] | Every measure, in model order; failures carry `error` and `⚠` in `display`, with their DAX. |

Examples:

- Unfiltered: `{"docId":"d1"}`
- Under the Stores report's filters: `{"docId":"d1","report":"Stores"}`

### `dax.diagnostics`

**Check DAX.** Syntax errors, unknown tables, columns and measures (with a did-you-mean suggestion), wrong argument counts and functions DashCraft cannot evaluate, for a DAX expression or query; no data is loaded.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` (optional) | string \| null | Check names against this document's model and report-level measures (`d1`, …). Without it only syntax and functions are checked. |
| `text` | string | The DAX text. |
| `kind` (optional) | `"auto"` \| `"expression"` \| `"query"` | `auto` (default), `expression` or `query`. |

Result:

| Field | Type | Description |
|---|---|---|
| `documentRevision`? | integer \| null | The document revision whose model names were used (with `docId`). |
| `kind` | `"expression"` \| `"query"` | How the text was checked. |
| `ok` | boolean | No errors (warnings allowed). |
| `diagnostics` | DaxDiagnosticOut[] | In source order, at most 1000. |

Examples:

- A measure against the model: `{"docId":"d1","text":"CALCULATE([Total Sales], Stores[Regoin] = \"West\")"}`
- Syntax only, without a document: `{"text":"EVALUATE ROW(\"n\", 1 +)"}`

### `dax.signatureHelp`

**DAX signature help.** The DAX function call around a cursor: its signature, parameters, return type, description and the parameter the cursor is in; works on text that does not parse yet.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `text` | string | The DAX text. |
| `position` | TextPositionIn | The cursor. |

Result:

| Field | Type | Description |
|---|---|---|
| `signature`? | DaxSignatureOut \| null | Absent outside a call of a known DAX function (or in a comment). |

Examples:

- The cursor after the first comma of IF: `{"text":"IF([Total Sales] > 0, ","position":{"offset":22}}`

### `dax.hover`

**DAX hover.** Describes the DAX function, table, column, measure, variable or keyword at a cursor: signature and description, data type, expression, format string; no data is loaded.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` (optional) | string \| null | Describe tables, columns and measures of this document's model and report-level measures (`d1`, …). Without it only functions, keywords, variables and the query's own definitions are described. |
| `text` | string | The DAX text. |
| `position` | TextPositionIn | The cursor. |

Result:

| Field | Type | Description |
|---|---|---|
| `documentRevision`? | integer \| null | The document revision whose model was used (with `docId`). |
| `hover`? | DaxHoverInfo \| null | Absent when there is nothing to describe at the cursor. |

Examples:

- A measure of the model: `{"docId":"d1","text":"[Total Sales] * 2","position":{"offset":3}}`
- A function, without a document: `{"text":"SUMX(Sales, 1)","position":{"line":1,"column":2}}`

### `dax.complete`

**DAX completion.** The DAX functions, tables, columns, measures, variables and keywords that fit at a cursor, best match first, with the text to insert; works on text that does not parse yet.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` (optional) | string \| null | Suggest the tables, columns and measures of this document's model and its report-level measures (`d1`, …). Without it only functions, keywords, variables and the query's own definitions are suggested. |
| `text` | string | The DAX text. |
| `position` | TextPositionIn | The cursor. |
| `maxItems` (optional) | integer | Most items returned, best first (default 100, at most 1000). |

Result:

| Field | Type | Description |
|---|---|---|
| `documentRevision`? | integer \| null | The document revision whose model was used (with `docId`). |
| `start` | TextPositionOut | Start of the text a chosen item replaces (the name being typed; the cursor when nothing is typed yet). |
| `end` | TextPositionOut | End (exclusive) of the replaced text. |
| `prefix` | string | The typed text the items match (`Tot` in `[Tot`). |
| `items` | DaxCompletionItemOut[] | Best match first: the same name, then names that start with the prefix, then a word that does, then names that contain it. Empty inside a comment, string or number, and after `VAR`. |
| `incomplete` | boolean | More items matched than `maxItems`. |

Examples:

- A measure of the model, inside brackets: `{"docId":"d1","text":"[Tot","position":{"offset":4}}`
- Functions, without a document: `{"text":"SUMX(Sales, SU","position":{"line":1,"column":15},"maxItems":10}`

### `dax.tokens`

**DAX semantic tokens.** What every token of a DAX text is (keyword, function, table, column, measure, variable, parameter, literal, comment) for highlighting, with declarations and unknown names marked.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` (optional) | string \| null | Resolve names against this document's model and report-level measures (`d1`, …). Without it names keep their syntactic kind and none is unresolved. |
| `text` | string | The DAX text. |

Result:

| Field | Type | Description |
|---|---|---|
| `documentRevision`? | integer \| null | The document revision whose model was used (with `docId`). |
| `tokens` | DaxTokenOut[] | In source order, without whitespace and punctuation. |

Examples:

- Names resolved against the model: `{"docId":"d1","text":"VAR x = [Total Sales] RETURN x * 2"}`
- Without a document: `{"text":"EVALUATE TOPN(5, Sales) ORDER BY Sales[Amount] DESC"}`

### `dax.references`

**DAX references.** The symbol at a cursor in a DAX text, where the text declares it and every place it occurs (go to definition, find references); model objects carry their objectId.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` (optional) | string \| null | Resolve names against this document's model and report-level measures (`d1`, …), and name model objects by `objectId`. |
| `text` | string | The DAX text. |
| `position` | TextPositionIn | The cursor. |

Result:

| Field | Type | Description |
|---|---|---|
| `documentRevision`? | integer \| null | The document revision whose model was used (with `docId`). |
| `symbol`? | DaxSymbolOut \| null | Absent when the cursor is not on a name (a keyword, literal, comment or operator). |
| `definition`? | DaxRangeOut \| null | Where the text declares the symbol (a `VAR`, a function parameter, a query `DEFINE`); absent for model objects and DAX functions. |
| `references` | DaxReferenceOut[] | Every occurrence in source order, the declaration included. |

Examples:

- A measure of the model: `{"docId":"d1","text":"[Total Sales] / CALCULATE([Total Sales], ALL(Stores))","position":{"offset":3}}`
- A variable, without a document: `{"text":"VAR x = 1 RETURN x + x","position":{"line":1,"column":5}}`

### `dax.format`

**Format DAX.** Lays out a DAX expression or query within a line width, keeping comments and meaning (only whitespace and keyword/function case change); idempotent, no data is loaded.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `text` | string | The DAX text. |
| `kind` (optional) | `"auto"` \| `"expression"` \| `"query"` | `auto` (default), `expression` or `query`. |
| `maxWidth` (optional) | integer | Preferred maximum line length in characters (default 80, at most 10000). Only a single long token, comment or unbreakable run makes a line longer. |
| `indentWidth` (optional) | integer | Spaces per indentation level (default 4, at most 16), or the width a tab counts for with `useTabs`. |
| `useTabs` (optional) | boolean | Indent with tabs instead of spaces. |
| `keywordCase` (optional) | `"upper"` \| `"lower"` \| `"keep"` | Case of keywords (`VAR`, `RETURN`, `IN`, `TRUE`, `EVALUATE`, `DESC`, parameter types, …): `upper` (default), `lower` or `keep`. |
| `functionCase` (optional) | `"upper"` \| `"lower"` \| `"keep"` | Case of built-in function names: `upper` (default), `lower` or `keep`. User-defined functions, tables, columns, measures and variables always keep their spelling. |

Result:

| Field | Type | Description |
|---|---|---|
| `kind` | `"expression"` \| `"query"` | How the text was parsed. |
| `ok` | boolean | The text parses (else `text` is the input unchanged and `diagnostic` says why). |
| `text` | string | The formatted text: `\n` line breaks, no trailing line break. |
| `changed` | boolean | `text` differs from the input. |
| `diagnostic`? | DaxDiagnosticOut \| null | The syntax error when `ok` is false. |

Examples:

- A measure: `{"text":"calculate(sum(Sales[Amount]),filter(all(Stores),Stores[Region]=\"West\"))"}`
- A query, narrow, lower-case keywords: `{"text":"EVALUATE SUMMARIZECOLUMNS(Stores[Region], \"Sales\", [Total Sales]) ORDER BY Stores[Region]","maxWidth":40,"keywordCase":"lower"}`

## M

### `m.diagnostics`

**Check M.** Syntax errors, unknown names, wrong argument counts and unsupported names in an M query or section, with Power BI Desktop's messages.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` (optional) | string \| null | Check names against this document's queries and parameters (`d1`, …). Without it, a plain name that is not in the standard library may be another query and is not reported. |
| `text` | string | The M text: a query (`let … in …`, any expression) or a section document (`section Section1; …`). |

Result:

| Field | Type | Description |
|---|---|---|
| `documentRevision`? | integer \| null | The document revision whose names were used (with `docId`). |
| `ok` | boolean | No errors (warnings allowed). |
| `diagnostics` | MDiagnosticOut[] | In source order, at most 1000. |

Examples:

- A query against the model: `{"docId":"d1","text":"let Source = Table.RowCnt(Orders) in Source"}`
- Without a document: `{"text":"let a = Text.Upper() in a"}`

### `m.hover`

**M hover.** Describes the M name at a cursor as Power BI Desktop binds it: step, parameter or field of the text, query, parameter or table of the model, library name or keyword.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` (optional) | string \| null | Describe this document's queries, parameters and tables (`d1`, …). Without it only the text's own names, the library and keywords are described. |
| `text` | string | The M text: a query or a section document. |
| `position` | TextPositionIn | The cursor. |

Result:

| Field | Type | Description |
|---|---|---|
| `documentRevision`? | integer \| null | The document revision whose queries were used (with `docId`). |
| `hover`? | MHoverInfo \| null | Absent when there is nothing to describe at the cursor. |

Examples:

- A query parameter of the model: `{"docId":"d1","text":"Web.Contents(StorageContainerUrl)","position":{"offset":15}}`
- A library function, without a document: `{"text":"Table.RowCount(Source)","position":{"line":1,"column":3}}`

### `m.signatureHelp`

**M signature help.** The M function call around a cursor: library function, #date-style constructor, function step or function query, its parameters and the one the cursor is in; works on unparsed text.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` (optional) | string \| null | Resolve calls of this document's function queries (`d1`, …). |
| `text` | string | The M text: a query or a section document. |
| `position` | TextPositionIn | The cursor. |

Result:

| Field | Type | Description |
|---|---|---|
| `documentRevision`? | integer \| null | The document revision whose queries were used (with `docId`). |
| `signature`? | MSignatureOut \| null | Absent outside a call of a known function, or in a comment. |

Examples:

- The second argument of Table.AddColumn: `{"text":"Table.AddColumn(Source, ","position":{"offset":24}}`
- A function query of the model: `{"docId":"d1","text":"fnCleanKey(","position":{"line":1,"column":12}}`

### `m.complete`

**M completion.** The M names that fit at a cursor, best match first, with the text to insert: steps and parameters in scope, the model's queries, keywords, library names, or types after as.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` (optional) | string \| null | Suggest this document's queries, parameters and tables (`d1`, …). Without it only the text's own names, the library and keywords are suggested. |
| `text` | string | The M text: a query or a section document. |
| `position` | TextPositionIn | The cursor. |
| `maxItems` (optional) | integer | Most items returned, best first (default 100, at most 1000). |

Result:

| Field | Type | Description |
|---|---|---|
| `documentRevision`? | integer \| null | The document revision whose queries were used (with `docId`). |
| `start` | TextPositionOut | Start of the text a chosen item replaces (the name being typed, a whole `#"…"` inside one; the cursor when nothing is typed yet). |
| `end` | TextPositionOut | End (exclusive) of the replaced text. |
| `prefix` | string | The typed text the items match (`Sou`, `Changed` in `#"Changed`). |
| `items` | MCompletionItemOut[] | Best match first: the same name, then names that start with the prefix, then a word that does, then names that contain it; among equals the text's names, then the model's, then keywords and the library. Empty in a comment, text or number, where a step or field name is written, and in `[…]` field access. |
| `incomplete` | boolean | More items matched than `maxItems`. |

Examples:

- A query of the model: `{"docId":"d1","text":"let Source = Ord","position":{"offset":16}}`
- Library functions, without a document: `{"text":"Table.Sel","position":{"line":1,"column":10},"maxItems":10}`

### `m.format`

**Format M.** Lays out an M query or section within a line width, one let step per line, keeping comments and meaning (only whitespace changes); idempotent, nothing is evaluated.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `text` | string | The M text: a query (`let … in …`, any expression) or a section document (`section Section1; …`). |
| `maxWidth` (optional) | integer | Preferred maximum line length in characters (default 100, at most 10000). Only a single long token, comment or unbreakable run makes a line longer. |
| `indentWidth` (optional) | integer | Spaces per indentation level (default 4, at most 16), or the width a tab counts for with `useTabs`. |
| `useTabs` (optional) | boolean | Indent with tabs instead of spaces. |

Result:

| Field | Type | Description |
|---|---|---|
| `ok` | boolean | The text parses (else `text` is the input unchanged and `diagnostic` says why). |
| `text` | string | The formatted text: `\n` line breaks, no trailing line break. |
| `changed` | boolean | `text` differs from the input. |
| `diagnostic`? | MDiagnosticOut \| null | The syntax error when `ok` is false, with Power BI Desktop's message. |

Examples:

- A query: `{"text":"let Source = Csv.Document(File.Contents(\"orders.csv\"),[Delimiter=\",\"]), Rows = Table.RowCount(Source) in Rows"}`
- Narrow, two-space indentation: `{"text":"Table.SelectRows(Orders, each [Quantity] > 1 and [Channel] = \"Online\")","maxWidth":40,"indentWidth":2}`

## TMDL

### `tmdl.diagnostics`

**Check TMDL.** What Microsoft's TMDL reader rejects in TMDL files: unknown keywords, wrong forms and values, levels, duplicates and unresolved references, with the closest name.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `files` | TmdlFileIn[] | The files to check: a whole `definition` folder (references between files are then resolved) or single files (only their own structure and values are checked). |

Result:

| Field | Type | Description |
|---|---|---|
| `ok` | boolean | No errors (warnings allowed). |
| `diagnostics` | TmdlDiagnosticOut[] | By file (in input order), then position; at most 1000. |

Examples:

- One table file: `{"files":[{"path":"tables/Stores.tmdl","text":"table Stores\n\tcolumn Region\n\t\tdataTyp: string\n"}]}`
- A model folder: `{"files":[{"path":"model.tmdl","text":"model Model\n\tculture: en-US\n"},{"path":"tables/Stores.tmdl","text":"table Stores\n\tcolumn Region\n\t\tdataType: string\n\t\tsortByColumn: Regions\n"}]}`

### `tmdl.complete`

**TMDL completion.** The TMDL keywords, values and object names that fit at a cursor, as Microsoft's TMDL reader accepts them there, best match first, with the text to insert.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `text` | string | The TMDL text being edited. |
| `position` | TextPositionIn | The cursor. |
| `path` (optional) | string \| null | The text's path relative to the `definition` folder; the file of `files` with this path is replaced by `text`. |
| `files` (optional) | TmdlFileIn[] | The other files of the model's `definition` folder: their tables, columns, measures and other objects are suggested where a reference names one, and their `database` sets the compatibility level. |
| `maxItems` (optional) | integer | Most items returned, best first (default 100, at most 1000). |

Result:

| Field | Type | Description |
|---|---|---|
| `start` | TextPositionOut | Start of the word a chosen item replaces (the cursor when nothing is typed yet). |
| `end` | TextPositionOut | End (exclusive) of the replaced word. |
| `prefix` | string | The typed text the items match. |
| `items` | TmdlCompletionItemOut[] | Best match first: the same word, then words that start with the prefix, then a word part that does, then words that contain it. Keywords are those of the object the line belongs to, accepted at the database's compatibility level, without properties already set or properties after a child object; after `keyword:` the values of its type; after a reference keyword the objects it may name. Empty inside expressions. |
| `incomplete` | boolean | More items matched than `maxItems`. |

Examples:

- The properties of a column: `{"text":"table Stores\n\tcolumn Region\n\t\tdata","position":{"line":3,"column":7}}`
- A column of another file: `{"text":"table Stores\n\tcolumn Region\n\t\tsortByColumn: ","position":{"line":3,"column":17},"path":"tables/Stores.tmdl","files":[{"path":"tables/Stores.tmdl","text":""},{"path":"tables/More.tmdl","text":"table Stores\n\tcolumn 'Region Order'\n"}]}`

### `tmdl.hover`

**TMDL hover.** Describes the TMDL word at a cursor: a keyword's forms, value and level, an enumeration value, or the object a reference names and where it is declared.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `text` | string | The TMDL text. |
| `position` | TextPositionIn | The cursor. |
| `path` (optional) | string \| null | The text's path relative to the `definition` folder; the file of `files` with this path is replaced by `text`. |
| `files` (optional) | TmdlFileIn[] | The other files of the model's `definition` folder, where referenced objects are declared. |

Result:

| Field | Type | Description |
|---|---|---|
| `hover`? | TmdlHoverInfo \| null | Absent when there is nothing to describe at the cursor. |

Examples:

- A property keyword: `{"text":"table Stores\n\tcolumn Region\n\t\tsummarizeBy: none\n","position":{"line":3,"column":5}}`
- A column of another file: `{"text":"relationship r1\n\tfromColumn: Orders.Store\n","position":{"line":2,"column":22},"files":[{"path":"tables/Orders.tmdl","text":"table Orders\n\tcolumn Store\n"}]}`

### `tmdl.format`

**Format TMDL.** Lays out a TMDL file as Power BI Desktop writes it: indentation, spacing, blank lines, expression blocks; only white space changes, checked by reading the result back.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `text` | string | One `.tmdl` file's text. |
| `indentWidth` (optional) | integer \| null | Indent with this many spaces per level (1 to 16); absent: with tabs, as Power BI Desktop does. |
| `lineEnding` (optional) | `"lf"` \| `"crlf"` \| null | The line ending of every line; absent: the text's first one (`lf` when it has none). |

Result:

| Field | Type | Description |
|---|---|---|
| `ok` | boolean | The text parses (else `text` is the input unchanged and `diagnostic` says why). |
| `text` | string | The formatted text; every line ends with a line ending. |
| `changed` | boolean | `text` differs from the input. |
| `diagnostic`? | TmdlDiagnosticOut \| null | The syntax error (`TMDL110`) when `ok` is false. |

Examples:

- A table file: `{"text":"table Stores\n  lineageTag:s1\n  column  Region\n    dataType :string\n  measure Count=COUNTROWS(Stores)\n"}`
- Two-space indentation, CRLF: `{"text":"table Stores\n\tcolumn Region\n\t\tdataType: string\n","indentWidth":2,"lineEnding":"crlf"}`

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
| `documentRevision` | integer | The semantic document revision the report, the page and its data were read at (one revision, even while edits commit). |
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
| `documentRevision` | integer | The semantic document revision the report, the page and its data were read at (one revision, even while edits commit). |
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
| `documentRevision` | integer | The semantic document revision the report, the page and its data were read at (one revision, even while edits commit). |
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
| `documentRevision` | integer | The semantic document revision the report, the page and its data were read at (one revision, even while edits commit). |
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
| `documentRevision` | integer | The semantic document revision the report, the page and its data were read at (one revision, even while edits commit). |
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
| `documentRevision` | integer | The semantic document revision the report, the page and its data were read at (one revision, even while edits commit). |
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

**Save as PBIP.** Writes the document, unsaved edits included, as a PBIP project (TMDL + PBIR) into a folder; the open document keeps its source (doc.saveAs moves it).

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

**Export a PBIT template.** Writes a PBIT template (model schema and one report, no data), unsaved edits included.

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

## Edit

### `model.setMeasureExpression`

**Edit a measure expression.** Replaces a measure's DAX expression as one undoable document edit.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `measureId` | integer | Stable measure id returned by `model.measures`. |
| `expression` | string | The replacement DAX expression. |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean |  |
| `revision` | integer |  |
| `dirty` | boolean |  |
| `changedObjectIds` | integer[] |  |
| `recalculated`? | RecalculatedOut[] | The calculated tables and columns the edit recomputed over the loaded data, in order (absent when none); a failed one has no values until an edit or a refresh computes it again. |
| `warnings`? | string[] | `doc.applyEdits`: problems that did not stop the edits (a measure's DAX that does not parse); absent when none. |

Examples:

- Update a measure: `{"docId":"d1","measureId":42,"expression":"SUM(Orders[Amount])"}`

### `model.object.rename`

**Rename a table, column, measure or query.** Renames a table, column, measure or M query/parameter as one undoable edit; DAX, M and report references follow and loaded data is kept. dryRun previews it.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `objectId` (optional) | integer \| null | The object's id (instead of `table`). |
| `table` (optional) | string \| null | The table, or the table of `column` / `measure`. |
| `column` (optional) | string \| null | A column of `table`. |
| `measure` (optional) | string \| null | A measure of `table`. |
| `expression` (optional) | string \| null | A shared expression (M query, parameter or function), instead of `table`. |
| `newName` | string | The new name. |
| `dryRun` (optional) | boolean | Only report what the rename would rewrite; the document is unchanged. |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean |  |
| `revision` | integer |  |
| `dirty` | boolean |  |
| `changedObjectIds` | integer[] |  |
| `recalculated`? | RecalculatedOut[] | The calculated tables and columns the edit recomputed over the loaded data, in order (absent when none); a failed one has no values until an edit or a refresh computes it again. |
| `objectId` | integer | The renamed object's id. |
| `kind` | string | `table`, `column`, `measure` or `expression`. |
| `table` | string | The renamed table's old name, or the table of the column or measure (empty for an expression). |
| `oldName` | string |  |
| `newName` | string |  |
| `dryRun` | boolean |  |
| `rewrites` | DaxRewriteOut[] | DAX expressions rewritten (or, with `dryRun`, to be rewritten). |
| `mRewrites`? | MRewriteOut[] | M expressions rewritten (or to be rewritten) when a query is renamed: a table with an M partition or a shared expression (absent when none). |
| `warnings` | string[] | References left as they are that may need a manual fix (DAX or M that does not parse, ambiguous unqualified names, refresh policies). |

Examples:

- Rename a column: `{"docId":"d1","table":"Orders","column":"Line Amount","newName":"Line Total"}`
- Preview a table rename: `{"docId":"d1","table":"Orders","newName":"Sales Orders","dryRun":true}`
- Rename a query parameter: `{"docId":"d1","expression":"StorageContainerUrl","newName":"Storage URL"}`

### `model.object.setDescription`

**Set an object's description.** Sets or clears the description of a table, column, measure or other model object as one undoable edit.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `objectId` (optional) | integer \| null | The object's id (instead of `table`): any model object with a description (tables, columns, measures, hierarchies, roles, …). |
| `table` (optional) | string \| null | The table, or the table of `column` / `measure`. |
| `column` (optional) | string \| null | A column of `table`. |
| `measure` (optional) | string \| null | A measure of `table`. |
| `text` (optional) | string \| null | The description; absent, `null` or empty clears it. |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean |  |
| `revision` | integer |  |
| `dirty` | boolean |  |
| `changedObjectIds` | integer[] |  |
| `recalculated`? | RecalculatedOut[] | The calculated tables and columns the edit recomputed over the loaded data, in order (absent when none); a failed one has no values until an edit or a refresh computes it again. |
| `warnings`? | string[] | `doc.applyEdits`: problems that did not stop the edits (a measure's DAX that does not parse); absent when none. |

Examples:

- Describe a measure: `{"docId":"d1","table":"Orders","measure":"Total Sales","text":"Sum of line amounts"}`
- Clear a description: `{"docId":"d1","objectId":42}`

### `model.measure.create`

**Create a measure.** Adds a DAX measure to a table as one undoable edit and returns its id.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `table` | string | The measure's home table. |
| `name` | string | The measure name: unique across the model and not a column of `table`. |
| `expression` | string | The DAX expression. |
| `formatString` (optional) | string \| null |  |
| `displayFolder` (optional) | string \| null |  |
| `description` (optional) | string \| null |  |
| `isHidden` (optional) | boolean |  |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean |  |
| `revision` | integer |  |
| `dirty` | boolean |  |
| `changedObjectIds` | integer[] |  |
| `recalculated`? | RecalculatedOut[] | The calculated tables and columns the edit recomputed over the loaded data, in order (absent when none); a failed one has no values until an edit or a refresh computes it again. |
| `measureId` | integer |  |
| `table` | string | The measure's table. |
| `name` | string | The measure's name. |
| `dependents`? | DependentOut[] \| null | `model.measure.delete` only: what refers to the measure (and breaks once it is deleted). |
| `dryRun`? | boolean \| null | `model.measure.delete` only: whether this was a dry run. |
| `warnings` | string[] | Problems that do not stop the edit (DAX that does not parse, references that cannot be confirmed). |

Examples:

- Add a measure: `{"docId":"d1","table":"Orders","name":"Order Count","expression":"COUNTROWS(Orders)","formatString":"#,0"}`

### `model.measure.update`

**Update a measure.** Changes a measure's expression, format string, display folder, description or visibility as one undoable edit.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `measureId` (optional) | integer \| null | The measure's id (instead of `table` and `measure`). |
| `table` (optional) | string \| null | The measure's table. |
| `measure` (optional) | string \| null | The measure's name. |
| `expression` (optional) | string \| null | The new DAX expression. |
| `formatString` (optional) | string \| null | The static format string; empty clears it. |
| `displayFolder` (optional) | string \| null | The display folder; empty clears it. |
| `description` (optional) | string \| null | The description; empty clears it. |
| `isHidden` (optional) | boolean \| null |  |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean |  |
| `revision` | integer |  |
| `dirty` | boolean |  |
| `changedObjectIds` | integer[] |  |
| `recalculated`? | RecalculatedOut[] | The calculated tables and columns the edit recomputed over the loaded data, in order (absent when none); a failed one has no values until an edit or a refresh computes it again. |
| `measureId` | integer |  |
| `table` | string | The measure's table. |
| `name` | string | The measure's name. |
| `dependents`? | DependentOut[] \| null | `model.measure.delete` only: what refers to the measure (and breaks once it is deleted). |
| `dryRun`? | boolean \| null | `model.measure.delete` only: whether this was a dry run. |
| `warnings` | string[] | Problems that do not stop the edit (DAX that does not parse, references that cannot be confirmed). |

Examples:

- New expression and format: `{"docId":"d1","table":"Orders","measure":"Total Sales","expression":"SUM(Orders[Line Amount])","formatString":"$#,0"}`
- Hide a measure: `{"docId":"d1","measureId":42,"isHidden":true}`

### `model.measure.delete`

**Delete a measure.** Deletes a measure as one undoable edit and lists what referred to it (DAX, report fields); dryRun only lists them.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `measureId` (optional) | integer \| null | The measure's id (instead of `table` and `measure`). |
| `table` (optional) | string \| null | The measure's table. |
| `measure` (optional) | string \| null | The measure's name. |
| `dryRun` (optional) | boolean | Only list what refers to the measure; the document is unchanged. |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean |  |
| `revision` | integer |  |
| `dirty` | boolean |  |
| `changedObjectIds` | integer[] |  |
| `recalculated`? | RecalculatedOut[] | The calculated tables and columns the edit recomputed over the loaded data, in order (absent when none); a failed one has no values until an edit or a refresh computes it again. |
| `measureId` | integer |  |
| `table` | string | The measure's table. |
| `name` | string | The measure's name. |
| `dependents`? | DependentOut[] \| null | `model.measure.delete` only: what refers to the measure (and breaks once it is deleted). |
| `dryRun`? | boolean \| null | `model.measure.delete` only: whether this was a dry run. |
| `warnings` | string[] | Problems that do not stop the edit (DAX that does not parse, references that cannot be confirmed). |

Examples:

- What uses a measure: `{"docId":"d1","table":"Orders","measure":"Total Sales","dryRun":true}`
- Delete by id: `{"docId":"d1","measureId":42}`

### `model.relationship.create`

**Create a relationship.** Adds a relationship between two columns as one undoable edit; filters follow it at once and warnings report what the engine finds in the loaded data.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `from` | ColumnParam | The from column (TOM `fromColumn`, usually the many side). |
| `to` | ColumnParam | The to column (TOM `toColumn`, usually the one side). |
| `fromCardinality` (optional) | `"one"` \| `"many"` \| null | Default `many`. |
| `toCardinality` (optional) | `"one"` \| `"many"` \| null | Default `one`. |
| `crossFilteringBehavior` (optional) | `"oneDirection"` \| `"bothDirections"` \| null | Default `oneDirection`. |
| `isActive` (optional) | boolean \| null | Default `true`. Two tables have at most one active relationship. |
| `name` (optional) | string \| null | Unique among relationships; a new GUID by default, as Power BI Desktop names them. |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean |  |
| `revision` | integer |  |
| `dirty` | boolean |  |
| `changedObjectIds` | integer[] |  |
| `recalculated`? | RecalculatedOut[] | The calculated tables and columns the edit recomputed over the loaded data, in order (absent when none); a failed one has no values until an edit or a refresh computes it again. |
| `relationshipId` | integer |  |
| `name` | string | The relationship's name (a GUID unless one was given). |
| `from` | ColumnParam | The from column (usually the many side). |
| `to` | ColumnParam | The to column (usually the one side). |
| `fromCardinality` | string | `many` or `one` (after the edit; before it for a delete). |
| `toCardinality` | string | `many` or `one`. |
| `crossFilteringBehavior` | string | `oneDirection`, `bothDirections` or `automatic`. |
| `securityFilteringBehavior` | string | `oneDirection`, `bothDirections` or `none`. |
| `isActive` | boolean |  |
| `dependents`? | DependentOut[] \| null | `model.relationship.delete` only: the DAX whose `USERELATIONSHIP` or `CROSSFILTER` calls name the relationship. |
| `dryRun`? | boolean \| null | `model.relationship.delete` only: whether this was a dry run. |
| `warnings` | string[] | Problems that do not stop the edit: what the engine finds in the loaded data (duplicate keys on the one side, differing key types, ambiguous filter paths), references that cannot be confirmed. |

Examples:

- Many-to-one, single direction: `{"docId":"d1","from":{"table":"Orders","column":"Store ID"},"to":{"table":"Stores","column":"Store ID"}}`
- Inactive, both directions: `{"docId":"d1","from":{"table":"Orders","column":"Ship Date"},"to":{"table":"Date","column":"Date"},"crossFilteringBehavior":"bothDirections","isActive":false}`

### `model.relationship.update`

**Update a relationship.** Changes a relationship's cardinality, cross-filter direction, security filtering or active state as one undoable edit; filters follow it at once.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `relationshipId` (optional) | integer \| null | The relationship's id from `model.relationships` (instead of `name`). |
| `name` (optional) | string \| null | The relationship's name (case-insensitive). |
| `fromCardinality` (optional) | `"one"` \| `"many"` \| null |  |
| `toCardinality` (optional) | `"one"` \| `"many"` \| null |  |
| `crossFilteringBehavior` (optional) | `"oneDirection"` \| `"bothDirections"` \| null |  |
| `securityFilteringBehavior` (optional) | `"oneDirection"` \| `"none"` \| `"bothDirections"` \| null | How row-level security filters cross it; `bothDirections` needs `crossFilteringBehavior: bothDirections`. |
| `isActive` (optional) | boolean \| null | Two tables have at most one active relationship: deactivate the other one first (both in one `doc.applyEdits` for one undo step). |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean |  |
| `revision` | integer |  |
| `dirty` | boolean |  |
| `changedObjectIds` | integer[] |  |
| `recalculated`? | RecalculatedOut[] | The calculated tables and columns the edit recomputed over the loaded data, in order (absent when none); a failed one has no values until an edit or a refresh computes it again. |
| `relationshipId` | integer |  |
| `name` | string | The relationship's name (a GUID unless one was given). |
| `from` | ColumnParam | The from column (usually the many side). |
| `to` | ColumnParam | The to column (usually the one side). |
| `fromCardinality` | string | `many` or `one` (after the edit; before it for a delete). |
| `toCardinality` | string | `many` or `one`. |
| `crossFilteringBehavior` | string | `oneDirection`, `bothDirections` or `automatic`. |
| `securityFilteringBehavior` | string | `oneDirection`, `bothDirections` or `none`. |
| `isActive` | boolean |  |
| `dependents`? | DependentOut[] \| null | `model.relationship.delete` only: the DAX whose `USERELATIONSHIP` or `CROSSFILTER` calls name the relationship. |
| `dryRun`? | boolean \| null | `model.relationship.delete` only: whether this was a dry run. |
| `warnings` | string[] | Problems that do not stop the edit: what the engine finds in the loaded data (duplicate keys on the one side, differing key types, ambiguous filter paths), references that cannot be confirmed. |

Examples:

- Filter both ways: `{"docId":"d1","relationshipId":7,"crossFilteringBehavior":"bothDirections"}`
- Deactivate: `{"docId":"d1","name":"d0c1e2f3-a4b5-4c6d-8e7f-000000000001","isActive":false}`

### `model.relationship.delete`

**Delete a relationship.** Deletes a relationship as one undoable edit and lists the DAX that names it (USERELATIONSHIP, CROSSFILTER); dryRun only lists it.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `relationshipId` (optional) | integer \| null | The relationship's id from `model.relationships` (instead of `name`). |
| `name` (optional) | string \| null | The relationship's name (case-insensitive). |
| `dryRun` (optional) | boolean | Only list what uses the relationship; the document is unchanged. |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean |  |
| `revision` | integer |  |
| `dirty` | boolean |  |
| `changedObjectIds` | integer[] |  |
| `recalculated`? | RecalculatedOut[] | The calculated tables and columns the edit recomputed over the loaded data, in order (absent when none); a failed one has no values until an edit or a refresh computes it again. |
| `relationshipId` | integer |  |
| `name` | string | The relationship's name (a GUID unless one was given). |
| `from` | ColumnParam | The from column (usually the many side). |
| `to` | ColumnParam | The to column (usually the one side). |
| `fromCardinality` | string | `many` or `one` (after the edit; before it for a delete). |
| `toCardinality` | string | `many` or `one`. |
| `crossFilteringBehavior` | string | `oneDirection`, `bothDirections` or `automatic`. |
| `securityFilteringBehavior` | string | `oneDirection`, `bothDirections` or `none`. |
| `isActive` | boolean |  |
| `dependents`? | DependentOut[] \| null | `model.relationship.delete` only: the DAX whose `USERELATIONSHIP` or `CROSSFILTER` calls name the relationship. |
| `dryRun`? | boolean \| null | `model.relationship.delete` only: whether this was a dry run. |
| `warnings` | string[] | Problems that do not stop the edit: what the engine finds in the loaded data (duplicate keys on the one side, differing key types, ambiguous filter paths), references that cannot be confirmed. |

Examples:

- What uses a relationship: `{"docId":"d1","relationshipId":7,"dryRun":true}`
- Delete by name: `{"docId":"d1","name":"d0c1e2f3-a4b5-4c6d-8e7f-000000000001"}`

### `model.column.create`

**Create a calculated column.** Adds a DAX calculated column to a table as one undoable edit, computes it over the loaded data and infers its data type unless dataType is given.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `table` | string | The column's table. |
| `name` | string | The column name: not a column or measure of `table`. |
| `expression` | string | The DAX expression, evaluated per row of `table`. |
| `dataType` (optional) | `"string"` \| `"int64"` \| `"double"` \| `"decimal"` \| `"dateTime"` \| `"boolean"` \| null | The data type; inferred from the values when omitted (after the automatic first load; required when the table has no rows loaded or every value is blank). |
| `formatString` (optional) | string \| null |  |
| `displayFolder` (optional) | string \| null |  |
| `description` (optional) | string \| null |  |
| `isHidden` (optional) | boolean |  |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean |  |
| `revision` | integer |  |
| `dirty` | boolean |  |
| `changedObjectIds` | integer[] |  |
| `recalculated`? | RecalculatedOut[] | The calculated tables and columns the edit recomputed over the loaded data, in order (absent when none); a failed one has no values until an edit or a refresh computes it again. |
| `columnId` | integer |  |
| `table` | string |  |
| `name` | string |  |
| `dataType` | string | `string`, `int64`, `double`, `decimal`, `dateTime` or `boolean`. |
| `dataTypeInferred` | boolean | Whether the data type was inferred from the values (no `dataType` given). |

Examples:

- Add a calculated column: `{"docId":"d1","table":"Orders","name":"Margin","expression":"Orders[Line Amount] - Orders[Line Cost]","formatString":"#,0.00"}`

### `model.column.update`

**Update a column.** Changes a calculated column's expression or data type, or a column's format, folder, description or visibility; what depends on it is recomputed over the loaded data.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `columnId` (optional) | integer \| null | The column's id (instead of `table` and `column`). |
| `table` (optional) | string \| null | The column's table. |
| `column` (optional) | string \| null | The column's name. |
| `expression` (optional) | string \| null | A calculated column's new DAX expression. Without `dataType` the data type is inferred again from its values (after the automatic first load; kept when the table has no rows loaded or every value is blank). |
| `dataType` (optional) | `"string"` \| `"int64"` \| `"double"` \| `"decimal"` \| `"dateTime"` \| `"boolean"` \| null | A calculated column's data type. |
| `formatString` (optional) | string \| null | The format string; empty clears it. |
| `displayFolder` (optional) | string \| null | The display folder; empty clears it. |
| `description` (optional) | string \| null | The description; empty clears it. |
| `isHidden` (optional) | boolean \| null |  |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean |  |
| `revision` | integer |  |
| `dirty` | boolean |  |
| `changedObjectIds` | integer[] |  |
| `columnId` | integer |  |
| `table` | string |  |
| `name` | string |  |
| `dataType` | string | The data type after the edit: `string`, `int64`, `double`, `decimal`, `dateTime` or `boolean`. |
| `dataTypeInferred` | boolean | Whether the data type was inferred from the values of the new expression (no `dataType` given). |
| `recalculated`? | RecalculatedOut[] | The column and what depends on it, recomputed over the loaded data, in order (absent when none); a failed one has no values until an edit or a refresh computes it again. |

Examples:

- New expression: `{"docId":"d1","table":"Orders","column":"Line Cost","expression":"Orders[Quantity] * RELATED(Products[Unit Cost]) * 1.1"}`
- Format a column: `{"docId":"d1","columnId":42,"formatString":"#,0.00"}`

### `model.column.delete`

**Delete a calculated column.** Deletes a calculated column as one undoable edit and lists what referred to it; refuses data columns and columns used by relationships, hierarchies or sort orders. dryRun only checks.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `columnId` (optional) | integer \| null | The column's id (instead of `table` and `column`). |
| `table` (optional) | string \| null | The column's table. |
| `column` (optional) | string \| null | The column's name. |
| `dryRun` (optional) | boolean | Only check the delete and list what refers to the column; the document is unchanged. |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean |  |
| `revision` | integer |  |
| `dirty` | boolean |  |
| `changedObjectIds` | integer[] |  |
| `recalculated`? | RecalculatedOut[] | The calculated tables and columns the edit recomputed over the loaded data, in order (absent when none); a failed one has no values until an edit or a refresh computes it again. |
| `objectId` | integer |  |
| `kind` | string | `column`, `table` or `hierarchy`. |
| `table` | string | The table, or the column's or hierarchy's table. |
| `name` | string |  |
| `dryRun` | boolean |  |
| `dependents` | DependentOut[] | What refers to the object and breaks once it is deleted (DAX, report fields). A table's own measures, columns and expressions go with it and are not listed. |
| `removed` | string[] | `model.table.delete`: what is deleted with the table (relationships that join it, role row filters on it). |
| `warnings` | string[] | References that cannot be confirmed (DAX that does not parse, ambiguous unqualified names, M queries naming a table). |

Examples:

- What uses a calculated column: `{"docId":"d1","table":"Orders","column":"Line Cost","dryRun":true}`
- Delete by id: `{"docId":"d1","columnId":42}`

### `model.table.create`

**Create a calculated table.** Adds a DAX calculated table as one undoable edit: its columns and their data types come from the expression's result over the loaded data, and it is computed at once.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `name` | string | The table name: not used by another table. |
| `expression` | string | The DAX table expression; its result's columns become the table's. |
| `description` (optional) | string \| null |  |
| `isHidden` (optional) | boolean |  |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean |  |
| `revision` | integer |  |
| `dirty` | boolean |  |
| `changedObjectIds` | integer[] |  |
| `recalculated`? | RecalculatedOut[] | The calculated tables and columns the edit recomputed over the loaded data, in order (absent when none); a failed one has no values until an edit or a refresh computes it again. |
| `tableId` | integer |  |
| `name` | string |  |
| `columns` | CreatedTableColumnOut[] | The columns, in order, with the data types inferred from the values. |

Examples:

- Add a calculated table: `{"docId":"d1","name":"Regions","expression":"VALUES(Stores[Region])"}`

### `model.table.update`

**Update a table.** Changes a calculated table's DAX expression, its columns following the result, or a table's description or visibility; the table and what depends on it are recomputed.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `tableId` (optional) | integer \| null | The table's id (instead of `table`). |
| `table` (optional) | string \| null | The table's name. |
| `expression` (optional) | string \| null | A calculated table's new DAX expression. A column its result still has (the same source column, else the same name) keeps its ID, name and properties; the result's other columns are added and the ones it no longer has are removed (refused while a relationship, hierarchy or sort order uses one). |
| `description` (optional) | string \| null | The description; empty clears it. |
| `isHidden` (optional) | boolean \| null |  |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean |  |
| `revision` | integer |  |
| `dirty` | boolean |  |
| `changedObjectIds` | integer[] |  |
| `recalculated`? | RecalculatedOut[] | The table and what depends on it, recomputed over the loaded data, in order (absent when none); a failed one has no values until an edit or a refresh computes it again. |
| `tableId` | integer |  |
| `name` | string |  |
| `columns` | CreatedTableColumnOut[] | The columns after the edit, in order. |
| `addedColumns`? | string[] | The columns the new expression added (absent when none). |
| `removedColumns`? | string[] | The columns the new expression no longer returns, removed (absent when none). |

Examples:

- New expression: `{"docId":"d1","table":"Regions","expression":"DISTINCT(Stores[Region])"}`
- Hide a table: `{"docId":"d1","tableId":42,"isHidden":true}`

### `model.table.delete`

**Delete a table.** Deletes a table with its columns, measures, relationships and role row filters as one undoable edit and lists what referred to it (DAX, report fields); dryRun only lists them.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `tableId` (optional) | integer \| null | The table's id (instead of `table`). |
| `table` (optional) | string \| null | The table's name. |
| `dryRun` (optional) | boolean | Only list what refers to the table and what goes with it; the document is unchanged. |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean |  |
| `revision` | integer |  |
| `dirty` | boolean |  |
| `changedObjectIds` | integer[] |  |
| `recalculated`? | RecalculatedOut[] | The calculated tables and columns the edit recomputed over the loaded data, in order (absent when none); a failed one has no values until an edit or a refresh computes it again. |
| `objectId` | integer |  |
| `kind` | string | `column`, `table` or `hierarchy`. |
| `table` | string | The table, or the column's or hierarchy's table. |
| `name` | string |  |
| `dryRun` | boolean |  |
| `dependents` | DependentOut[] | What refers to the object and breaks once it is deleted (DAX, report fields). A table's own measures, columns and expressions go with it and are not listed. |
| `removed` | string[] | `model.table.delete`: what is deleted with the table (relationships that join it, role row filters on it). |
| `warnings` | string[] | References that cannot be confirmed (DAX that does not parse, ambiguous unqualified names, M queries naming a table). |

Examples:

- What a table delete breaks: `{"docId":"d1","table":"Stores","dryRun":true}`
- Delete by id: `{"docId":"d1","tableId":3}`

### `model.hierarchy.create`

**Create a hierarchy.** Adds a user hierarchy over columns of a table as one undoable edit; levels are listed top first and named after their columns unless named.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `table` | string | The hierarchy's table. |
| `name` | string | The hierarchy name: not a column, measure or hierarchy of `table`. |
| `levels` | LevelParam[] | The levels, top level first: different columns of `table`. |
| `displayFolder` (optional) | string \| null |  |
| `description` (optional) | string \| null |  |
| `isHidden` (optional) | boolean |  |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean |  |
| `revision` | integer |  |
| `dirty` | boolean |  |
| `changedObjectIds` | integer[] |  |
| `hierarchyId` | integer |  |
| `table` | string |  |
| `name` | string |  |
| `levelIds` | integer[] | The levels' ids, top level first. |

Examples:

- Region over store: `{"docId":"d1","table":"Stores","name":"Places","levels":[{"column":"Region"},{"column":"Store Name","name":"Store"}]}`

### `model.hierarchy.delete`

**Delete a hierarchy.** Deletes a user hierarchy with its levels as one undoable edit and lists the report fields that used it; dryRun only lists them.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `hierarchyId` (optional) | integer \| null | The hierarchy's id (instead of `table` and `hierarchy`). |
| `table` (optional) | string \| null | The hierarchy's table. |
| `hierarchy` (optional) | string \| null | The hierarchy's name. |
| `dryRun` (optional) | boolean | Only list the report fields that use the hierarchy; the document is unchanged. |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean |  |
| `revision` | integer |  |
| `dirty` | boolean |  |
| `changedObjectIds` | integer[] |  |
| `recalculated`? | RecalculatedOut[] | The calculated tables and columns the edit recomputed over the loaded data, in order (absent when none); a failed one has no values until an edit or a refresh computes it again. |
| `objectId` | integer |  |
| `kind` | string | `column`, `table` or `hierarchy`. |
| `table` | string | The table, or the column's or hierarchy's table. |
| `name` | string |  |
| `dryRun` | boolean |  |
| `dependents` | DependentOut[] | What refers to the object and breaks once it is deleted (DAX, report fields). A table's own measures, columns and expressions go with it and are not listed. |
| `removed` | string[] | `model.table.delete`: what is deleted with the table (relationships that join it, role row filters on it). |
| `warnings` | string[] | References that cannot be confirmed (DAX that does not parse, ambiguous unqualified names, M queries naming a table). |

Examples:

- What uses a hierarchy: `{"docId":"d1","table":"Stores","hierarchy":"Geography","dryRun":true}`
- Delete by id: `{"docId":"d1","hierarchyId":57}`

### `report.visual.move`

**Move a visual.** Moves a visual or a visual group (with the visuals in it) on its page as one undoable edit.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `report` (optional) | string \| null | Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report. |
| `page` (optional) | string \| null | Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. |
| `visual` | string | The visual's name (see `report.visuals`). |
| `x` (optional) | number \| null | New left edge (page units, as in `report.visuals`); unchanged when absent. |
| `y` (optional) | number \| null | New top edge; unchanged when absent. |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean |  |
| `revision` | integer |  |
| `dirty` | boolean |  |
| `changedObjectIds` | integer[] |  |
| `visual` | string | The visual's name. |
| `position` | PositionOut | The visual's position after the edit (page units, as in `report.visuals`). |
| `members` | MemberPositionOut[] | For a visual group: the visuals in it whose position changed with it, with their new positions (as in `report.visuals`: stored relative to their group when the file does so). |

Examples:

- Move a visual: `{"docId":"d1","page":"1","visual":"f7a9c9413fef68532313","x":40,"y":120}`

### `report.visual.resize`

**Resize a visual.** Resizes a visual or a visual group (top-left corner fixed; the visuals in a group scale with it unless it scrolls) as one undoable edit.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `report` (optional) | string \| null | Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report. |
| `page` (optional) | string \| null | Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. |
| `visual` | string | The visual's name (see `report.visuals`). |
| `width` (optional) | number \| null | New width (page units, > 0); unchanged when absent. |
| `height` (optional) | number \| null | New height (> 0); unchanged when absent. |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean |  |
| `revision` | integer |  |
| `dirty` | boolean |  |
| `changedObjectIds` | integer[] |  |
| `visual` | string | The visual's name. |
| `position` | PositionOut | The visual's position after the edit (page units, as in `report.visuals`). |
| `members` | MemberPositionOut[] | For a visual group: the visuals in it whose position changed with it, with their new positions (as in `report.visuals`: stored relative to their group when the file does so). |

Examples:

- Resize a visual: `{"docId":"d1","page":"1","visual":"f7a9c9413fef68532313","width":320,"height":180}`

### `doc.applyEdits`

**Apply document edits.** Applies a group of document edits atomically as one undoable history step.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `edits` | EditOperation[] | Edits applied atomically and undone as one history step. Names are resolved against the document before the first edit. |
| `label` (optional) | string \| null | The history step's label (`doc.history` `undoLabel` and `redoLabel`, at most 200 characters); default `Apply document edits`. |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean |  |
| `revision` | integer |  |
| `dirty` | boolean |  |
| `changedObjectIds` | integer[] |  |
| `recalculated`? | RecalculatedOut[] | The calculated tables and columns the edit recomputed over the loaded data, in order (absent when none); a failed one has no values until an edit or a refresh computes it again. |
| `warnings`? | string[] | `doc.applyEdits`: problems that did not stop the edits (a measure's DAX that does not parse); absent when none. |

Examples:

- Update a measure and its description: `{"docId":"d1","edits":[{"type":"setMeasureExpression","measureId":42,"expression":"SUM(Orders[Amount])"},{"type":"setDescription","objectId":42,"text":"Total amount"}]}`
- Rename and edit a measure as one labelled step: `{"docId":"d1","label":"Edit measure 'Sales'","edits":[{"type":"renameObject","objectId":42,"newName":"Sales"},{"type":"updateMeasure","measureId":42,"expression":"SUM(Orders[Amount])"}]}`

### `doc.undo`

**Undo document edit.** Undoes the latest document edit.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean |  |
| `revision` | integer |  |
| `dirty` | boolean |  |
| `changedObjectIds` | integer[] |  |
| `recalculated`? | RecalculatedOut[] | The calculated tables and columns the edit recomputed over the loaded data, in order (absent when none); a failed one has no values until an edit or a refresh computes it again. |
| `warnings`? | string[] | `doc.applyEdits`: problems that did not stop the edits (a measure's DAX that does not parse); absent when none. |

Examples:

- Undo the latest edit: `{"docId":"d1"}`

### `doc.redo`

**Redo document edit.** Reapplies the latest undone document edit.

_mutates the document, agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |

Result:

| Field | Type | Description |
|---|---|---|
| `changed` | boolean |  |
| `revision` | integer |  |
| `dirty` | boolean |  |
| `changedObjectIds` | integer[] |  |
| `recalculated`? | RecalculatedOut[] | The calculated tables and columns the edit recomputed over the loaded data, in order (absent when none); a failed one has no values until an edit or a refresh computes it again. |
| `warnings`? | string[] | `doc.applyEdits`: problems that did not stop the edits (a measure's DAX that does not parse); absent when none. |

Examples:

- Redo the latest undone edit: `{"docId":"d1"}`

### `doc.history`

**Document edit history.** Returns the document revision, dirty state and undo/redo availability.

_agent-safe; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |

Result:

| Field | Type | Description |
|---|---|---|
| `revision` | integer |  |
| `dirty` | boolean |  |
| `canUndo` | boolean |  |
| `canRedo` | boolean |  |
| `undoLabel`? | string \| null |  |
| `redoLabel`? | string \| null |  |

Examples:

- Current edit history: `{"docId":"d1"}`

### `doc.save`

**Save.** Saves the edits into the opened PBIP project, writing only changed files; files changed on disk since the open are a conflict unless overwrite.

_mutates the document, agent-safe, writes files; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `overwrite` (optional) | boolean | Save even if project files changed on disk since the open or last save: files DashCraft writes get the document's version, other files are kept. Without it such a save fails with `conflict`. |

Result:

| Field | Type | Description |
|---|---|---|
| `projectDir` | string | The project folder. |
| `written` | string[] | Files written, relative to the project folder. |
| `deleted` | string[] | Files deleted (e.g. the `.tmdl` file of a renamed table). |
| `overwritten` | FileChangeOut[] | Files changed on disk since the open or last save (only with `overwrite`). |
| `revision` | integer | The revision saved. |
| `dirty` | boolean | Whether the document has edits after the save (always `false`). |

Examples:

- Save the edits: `{"docId":"d1"}`
- Save over external changes: `{"docId":"d1","overwrite":true}`

### `doc.saveAs`

**Save As.** Saves the document, edits included, as a new PBIP project from any format and continues editing there: later doc.save writes to it; imported data is not carried over.

_mutates the document, long-running, agent-safe, writes files; since 0.2.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `outDir` | string | The new project folder (created if missing; must be empty unless `overwrite`). |
| `overwrite` (optional) | boolean | Write into a non-empty folder (files with the same paths are replaced, others kept). |

Result:

| Field | Type | Description |
|---|---|---|
| `projectDir` | string | The new project folder. |
| `pbipFile` | string | The project's `.pbip` file, relative to `projectDir`. |
| `files` | string[] | Files written, relative to `projectDir`. |
| `bytes` | integer |  |
| `previousFormat` | string | The format the document had before (`pbix`, `pbit`, `pbip`, …). |
| `revision` | integer | The revision saved. |
| `dirty` | boolean | Whether the document has edits after the save (always `false`). |
| `warnings` | string[] | What the new project does not carry over (imported data, a local data cache). |

Examples:

- Save as a new project: `{"docId":"d1","outDir":"target/tmp/acme-copy"}`
- Into an existing folder: `{"docId":"d1","outDir":"target/tmp/acme-copy","overwrite":true}`
