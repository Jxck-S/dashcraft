# DashCraft user guide

DashCraft opens Power BI files offline. This guide covers the desktop app, then CLI recipes for scripts and CI. Every command and flag is in [CLI.md](CLI.md); supported formats are in [FORMATS.md](../FORMATS.md) and visuals in [VISUALS.md](VISUALS.md).

DashCraft is a **viewer**: nothing you do in it changes the file you opened. It never contacts the network (unless you turn on the opt-in [AI features](#ai-features-opt-in) and allow a request) and never runs scripts, custom visuals or connectors embedded in a file.

## Contents

- [Opening files](#opening-files)
- [The window](#the-window)
- [Views](#views)
- [Loading data](#loading-data)
- [Interacting with a report](#interacting-with-a-report)
- [View as roles](#view-as-roles)
- [Parse people field](#parse-people-field)
- [Diagnostics and linting](#diagnostics-and-linting)
- [AI features (opt-in)](#ai-features-opt-in)
- [Keyboard shortcuts](#keyboard-shortcuts)
- [CLI recipes](#cli-recipes)

## Opening files

Open a file or folder with any of:

- **File → Open** (Browse this device, or Open a folder), the start page, Ctrl/⌘+O (file) or Ctrl/⌘+Shift+O (folder);
- **File → Recent**;
- drag and drop onto the window;
- the command line: `dashcraft open <path>` (from the CLI) or `dashcraft-app <path>`.

What you can open: a `.pbip` file or project folder, a `*.SemanticModel` or `*.Report` folder, a `.pbix` or unzipped PBIX folder, a `.pbit`, `model.bim`, a TMDL folder, `.abf`, `.vpax`, an Excel workbook with a Power Pivot model, `.pq`/`.pqt`, dataflow definitions, `.rdl`, `.pbids`, `.pbiviz`, `.mez`/`.pqx`. DashCraft recognises the format from the content, not the extension.

One document is open at a time; opening another replaces it. A report connected to a model in the Power BI service (`byConnection`, or a live-connected PBIX) shows the banner *Remote model — not available offline*: pages, visuals and fields are listed, but there is no data to show.

## The window

The layout follows Power BI Desktop ([details](../app/docs/desktop-ui-reference.md)). Commands DashCraft does not implement are present but disabled, with the tooltip "Not available in DashCraft (yet)".

- **Title bar**: the file name and **Search** (Alt+Q) over the ribbon's commands, the views and the model's tables, columns and measures.
- **Ribbon**: the tabs depend on the view. Working commands include Home → **Transform data** (read-only Power Query editor and its menu: Data source settings → **Blob map…**, Edit parameters, **Load CSV manually…**), **Refresh**, **Manage relationships**, Design → Themes (the report's theme, read-only), View → **Page view**, **Filters**, **Selection**, **Properties**, **Reset to default**, **Outline**.
- **View rail** (left): Report, Table, Model, DAX query, TMDL.
- **Right panes** (Report view): **Filters**, **Visualizations**, **Data**. Each collapses to a strip.
- **Formula bar**: the DAX of the selected measure, calculated column or calculated table (read-only).
- **Status bar**: the view's status (Page X of Y, table and column, query status), the data state (cached, loaded, refreshing, errors), diagnostic counters and the zoom slider.

## Views

### Report view

The page as the Power BI service draws it, on a gray surround. Page tabs are below the canvas (hidden pages greyed; Ctrl/⌘+Page Up / Page Down move between pages); a report picker appears when a project has several reports. Zoom with View → Page view (Fit to page, Fit to width, Actual size), the status bar slider or Ctrl/⌘+wheel.

Panes:

- **Filters**: filter cards on the selected visual, this page and all pages, as in reading view: cards hidden in view mode are left out unless you tick "Show n hidden filter cards", locked cards show a lock. Your slicer selections and cross-filter appear as "Slicer: …" and "Selection: …" cards with a clear button.
- **Visualizations**: *Build* shows the gallery with the selected visual's type and its field wells; *Format* shows the visual's formatting objects (Visual and General cards) read-only.
- **Data**: tables, measures, columns and hierarchies with Power BI's icons; the selected visual's fields are ticked. Tooltips show the source column name when it differs from the display name, the data type, the description and a measure's current value. Hidden objects are behind "View hidden" in the ⋯ menu. Each table shows its row count (`412`, `12.3K`, `—` when not loaded), coloured by the last refresh result.
- **Selection**: the visuals in layer and tab order; click to select.
- **Properties** (View → Properties): every property of the selected table, column, measure, partition, relationship, expression, page, visual or filter, with its DAX, M or JSON, annotations, unmodelled properties and the source file and line or JSON pointer.

The **Measures** panel docked under the canvas (collapsed by default) lists every model measure. By default each value is computed as on the canvas (report and page filters, slicer selections, cross-filter); you can switch to no filters, the report's filters, or report and page filters. Measures that fail show ⚠ with the message and DAX in the tooltip. View → **Outline** shows the page as a table of visuals with positions and fields instead of the rendered page.

### Table view

A grid of the table chosen in the Data pane. Rows load as you scroll. Click a header to sort (ascending, descending, none); right-click or ⋯ for sort, column statistics, *Filter on this column*, *Parse people field…* and *Remove derived column*. The filter box takes a SQL `WHERE` expression (`"Status" = 'Shipped'`); Enter applies, Esc clears, ⓘ shows the syntax. Click a cell or use Column tools → **Column statistics** for rows, blanks, distinct values, min and max (over the whole table). Table tools → **Schema** shows columns, types, kinds, source columns and format strings.

### Model view

A diagram of table cards and relationship lines with cardinality and cross-filter arrows (inactive relationships dashed). Positions come from the project's `diagramLayout.json` when present, else an automatic layout. Drag to pan, Ctrl/⌘+wheel to zoom, Fit to screen. Home → **Manage relationships** lists every relationship.

### DAX query view

Run `EVALUATE` queries (with `DEFINE` and `ORDER BY`) over the loaded data. F5 or Ctrl/⌘+Enter runs the query, or only the selected lines; Ctrl/⌘+/ comments lines; Run turns into Cancel while a query runs. Results show up to 1,000 rows per `EVALUATE` ("Result n of m"), with **Copy** as tab-separated text. The Data pane's **Quick queries** (Show top 100 rows, Evaluate, Define and evaluate, Show data preview) write a query for you. Queries never change the model.

```dax
EVALUATE
SUMMARIZECOLUMNS ( Stores[Region], "Orders", [Order Count] )
ORDER BY [Orders] DESC
```

### TMDL view

The model as TMDL scripts, read-only: a project's own `.tmdl` files verbatim, or TMDL generated from a `model.bim`, PBIT or PBIX model. Selecting a table opens its script.

### Transform data

Home → **Transform data** opens a read-only Power Query editor with the M partitions, shared queries, parameters, measures, calculated columns and calculated tables, filterable by name or text.

## Loading data

Opening a file loads its model, not its data (except for the cache, below). Data comes from:

| Source | Used for | How |
|---|---|---|
| Embedded data | PBIX, `.abf`, Excel Power Pivot | **Refresh** decodes it; no M runs |
| Local data cache | PBIP with `<Model>.SemanticModel/.pbi/cache.abf` (saved by Power BI Desktop) | Loaded automatically on open; a banner says *Loaded cached data (as of …)* |
| M queries | PBIP without a cache, PBIT, `model.bim`, TMDL folders | **Refresh** runs the partitions in DashCraft's own M engine |
| A CSV you pick | any table whose refresh failed | **Load CSV manually…** |

**Refresh** (Home → Refresh, Table tools → Refresh table, or the Data pane menu) shows a panel with per-table progress and **Cancel**; cancelling keeps the previous data. Then it lists rows, source and duration per table, and for failures the message, a hint and **Load CSV manually…** when a CSV can stand in. Calculated columns and tables are computed after the tables they depend on; when a table fails, what depends on it is skipped with the reason.

**Cached data.** The cache is Power BI Desktop's last refresh, so it can be older than the source. **Refresh from source** in the banner runs the M queries instead. Calculated objects missing from the cache are computed; a table missing from it fails with the CSV fallback.

**Blob map.** DashCraft never reads Azure Blob Storage. M queries that call `AzureStorage.Blobs(url)` read local files listed in a `blob-map.json` instead. Pick it with Transform data ▾ → Data source settings → **Blob map…** (or File → Options and settings), or start the app with `DASHCRAFT_BLOB_MAP=<path>` (the former `PBIVIEW_BLOB_MAP` still works). It applies to the next refresh and lasts for the session; × stops using it. Without a map, those tables fail with an "offline" hint. Format (paths relative to the map; never put credentials in it):

```json
{
  "containers": {
    "https://<account>.blob.core.windows.net/<container>": {
      "folder/data.csv": "csv/data.csv"
    }
  }
}
```

**Load CSV manually…** (refresh panel, Transform data ▾, Data pane menu, Table tools, or the Table view's error banner) loads a CSV, TSV or TXT file into a table: choose the delimiter (comma, semicolon, tab, pipe, other), whether the first row is a header, and the encoding (detect, UTF-8, Windows-1252, UTF-16 LE). CSV columns are matched to the table's columns by name (source column, then display name, case-insensitive), or by position when there is no header row, and converted to the model's types. Missing columns and values that do not convert load as BLANK, with warnings in a toast. Dependent calculated objects are recomputed. Calculated tables cannot be loaded from CSV. A later refresh runs the table's query again and keeps the CSV data if it fails.

Other data-access functions (SQL, web, files, SharePoint, …) are never evaluated: their tables fail with "unsupported" and offer the CSV fallback.

## Interacting with a report

The Report view behaves like Power BI's reading view:

- **Slicers**: click an item to select it. With *Multi-select with Ctrl* (the default) a click replaces the selection and Ctrl/⌘+click adds or removes; *Single select* always replaces; without either, every click toggles. **Select all** switches between all and none. Click a dropdown to open its list, type in the search box when the slicer has one, scroll with the wheel. Hover the header and click the eraser to clear. Range and relative-date slicers are drawn but their sliders cannot be dragged.
- **Cross-filtering**: click a pie slice, bar, column, line point, table row or matrix row. The other visuals on the page filter per the page's interaction settings, and the source dims its other points. Ctrl/⌘+click adds or removes points. Click the point again, the visual's background, the empty canvas or press Esc to clear. Highlight interactions filter instead of highlighting.
- **Tooltips**: hover a data point for its category, series, value and tooltip fields; hover a visual with ⚠ for the error and the measure's DAX.
- **Tables and matrices**: scroll with the wheel (Shift+wheel horizontally); double-click a matrix row or click its +/− to expand or collapse.
- **Buttons**: Back returns to the previous page you visited; page navigation buttons go to their page; Clear all slicers clears them. Web URL buttons show the address with **Copy link**; DashCraft never opens links. Bookmarks, drillthrough and Q&A are not available.
- **Selection**: click a visual to select it for the Filters, Visualizations, Data and Properties panes. Tab and Enter select visuals with the keyboard.
- **Reset to default** (View tab): back to the selections saved in the report, with no cross-filter.

Selections are never saved to the file. They last until the document closes.

## View as roles

A model with row-level security roles can be viewed the way a member of those roles would see it, as with Power BI Desktop's **Modeling → View as** (also **Home → Security → View as** in the Model view). The button is disabled when the model has no roles.

1. Click **View as**. The *View as roles* dialog lists every role with its table filters (DAX). Check one or more roles: a row is visible if any checked role allows it.
2. For roles that filter by user (marked *per user*: they call `USERPRINCIPALNAME()`, `USERNAME()`, `USEROBJECTID()` or `CUSTOMDATA()`), check **Other user** and type the user principal name, for example `ann@contoso.com`. Other user without a role only sets the user.
3. Click **OK**. If a role's filter cannot be evaluated (a missing column, a DAX error), the dialog shows the error and nothing changes.

While a view is active, a yellow bar says **Now viewing as: <roles>** (and the other user). The report canvas, the Table view grid, the Measures panel, the Data pane values and DAX queries all show only the allowed rows; filters follow relationships in their security direction, and `ALL()` cannot see past them. Refreshes, CSV loads and derived columns keep the view (if its filters then fail, every table shows empty and the bar says why). **Stop viewing** shows all rows again; **View as…** changes the roles or the user.

The view is never saved to the file and ends when the document closes. Members listed in the model are shown in the dialog for information only: DashCraft does not look users up. Scripts and agents use `security.roles` and `security.viewAs` ([COMMANDS.md](COMMANDS.md)): `dashcraft run security.viewAs --doc model.pbip -p 'roles=["East"]'` prints the rows each table shows as East, and in a `dashcraft run --batch` file, `dashcraft serve` or `dashcraft mcp` session the following commands see only those rows.

## Parse people field

Some sources export people pickers as Python-style lists, for example `[{'firstName': 'Ada', 'lastName': 'Lovelace', …}]`, quoted with `'` or `"`. To turn such a text column into readable names, right-click its header in the Table view (or Column tools → **Parse people field…**). A preview shows how many cells parse into names, how many stay unparsed or empty, and samples (raw → parsed). **Add** creates `<column> (parsed)` with `First Last; First Last` values.

The new column behaves like any other column: it sorts and filters, has statistics, and DAX measures, DAX queries and SQL filters can use it. It is recomputed after each refresh or CSV load, removed with *Remove derived column*, and never saved to the model.

## Diagnostics and linting

- The status bar counts the **reader diagnostics** (errors, warnings, infos) found while opening the file: unreadable parts, unsupported properties, remote models, and so on. Click the counter for the list (severity, code, message, origin; clicking a message selects its object) and a **Detection** tab with the format, confidence, signals, models, reports and parts DashCraft found.
- **Broken references** (fields missing from the model, stale `queryRef`s, filters, sorts, interactions, DAX and M references, relationships, overlapping visuals) are checked by `dashcraft lint`; see [CI recipes](#lint-gate) and the [rule list](../crates/pbi-core/src/lint/RULES.md). The app does not run the linter.
- A visual whose data cannot be computed shows ⚠ "Can't display this visual"; hover it for the reason. Measures that fail show ⚠ in the Measures panel and the Data pane tooltip.

## AI features (opt-in)

DashCraft can ask a language model to **explain a measure** or **suggest a new measure**. It is **off by default**: until you turn it on, the AI buttons are visible but disabled ("Turn on AI in Options"), and DashCraft makes no network request at all. Details, providers and the threat model: [AI.md](AI.md).

1. **Turn it on**: File → Options and settings → **Options** → **AI** (or Home → AI → **AI options**). Tick **Turn on AI features**, then **Add provider**: OpenAI-compatible (OpenAI, GitHub Models, LM Studio, vLLM), Azure OpenAI (endpoint, deployment, API version) or Ollama (a model on this computer). For providers that need one, **Set key…** stores the API key in the OS keychain (Keychain on macOS, Credential Manager on Windows); the key field is write-only and the key is never shown again or written to the settings file. **Clear key** removes it. The page also sets the default provider and how many **sample rows** a request may include (0, the default, means none) and lists your **consents** with **Revoke**.
2. **Explain a measure**: right-click a measure in the Data pane → **Explain with AI**, or the ✦ button in the formula bar, **Explain with AI** in the Properties pane, Home → AI → **Explain measure**, or Measure tools → **Explain with AI**. A pane opens with progress and **Cancel**, then the summary, the explanation, filter-context notes, dependencies, caveats and what the measure references (computed locally). **Copy as Markdown** copies it.
3. **Suggest a measure**: Home (or Modeling) → AI → **Suggest a measure**. Describe the measure, pick its home table and press **Suggest**: you get the DAX (with **Copy**), the problems DashCraft's own DAX parser finds, an explanation and assumptions. **Evaluate locally** computes its value over the loaded data (no filters; nothing is sent) and **Open in DAX query view** opens it as a `DEFINE MEASURE … EVALUATE` query. Suggestions are **never written to the model**.

**Consent.** The first time a feature would send data to a provider, DashCraft stops and shows **Allow this AI request?**: the provider, its endpoint (labelled *local* when it is this computer, *remote* otherwise), the feature, the kind of data (model metadata — names, types, descriptions, relationships and DAX, never row values — or sample rows if you asked for them) and the exact JSON request, with keys shown as `[redacted]`. Only **Allow and send** records the consent and sends; **Cancel** sends nothing. The consent covers that provider endpoint, feature and kind of data until you revoke it in Options → AI; changing the provider's endpoint drops it.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| Ctrl/⌘+O, Ctrl/⌘+Shift+O | Open a file, open a folder |
| Alt+Q | Search |
| Ctrl+F1 | Collapse or pin the ribbon |
| Ctrl/⌘+Page Up / Page Down | Previous / next page |
| Ctrl/⌘+wheel | Zoom the canvas or the diagram |
| F5, Ctrl/⌘+Enter | Run the DAX query (or the selected lines) |
| Ctrl/⌘+/ | Comment or uncomment DAX lines |
| Esc | Close menus, popups, dialogs; clear the cross-filter |

## Privacy

The app reads the files you open, the blob map and the CSVs you pick. It writes its recent-files list (`recent-files.json`, at most 20 paths) and, once you change an AI option, the AI settings (`ai.json`, never keys) in the OS app configuration folder. It sends no telemetry. It makes no network requests unless you turn on the [AI features](#ai-features-opt-in) and allow a request in the consent dialog; then the request goes only to the provider you configured.

## CLI recipes

The `dashcraft` CLI runs the same readers, engines and renderer headlessly (`pbiview`, the former name, is an alias of it). Build it with `cargo build --release -p pbiview-cli`. Exit codes: 0 success, 1 fatal error, 2 the input has errors (see [CLI.md](CLI.md#exit-codes)), which makes most commands usable as CI gates as they are.

### Inspect in CI

`inspect --json` has a stable schema (`schemaVersion: 1`; fields are only ever added within a version):

```sh
dashcraft inspect --json report.pbip > inspect.json        # exit 2 on reader errors
jq -e '.counts.errors == 0' inspect.json
jq -e '[.reports[].pages[].visuals[]] | length == 86' inspect.json
jq -r '.reports[] | .name as $r | .pages[] | "\($r) / \(.displayName): \(.visuals | length) visuals"' inspect.json
dashcraft inspect --json --no-expressions report.pbix > outline.json   # no DAX or M: safe to share
```

### Lint gate

```sh
dashcraft lint report.pbip                         # fails (exit 2) on Error findings
dashcraft lint report.pbip --deny warnings         # also fails on warnings, e.g. stale queryRefs
dashcraft lint report.pbip --deny warnings --disable LINT016 --json > lint.json
jq -r '.findings[] | "\(.severity) \(.rule) \(.origin.path) \(.message)"' lint.json
```

`--no-layout` skips the layout rules (visuals outside the page, overlaps). Each finding has a file, a JSON pointer or line, and usually a suggested fix.

### Check KPI values

```sh
dashcraft measures report.pbip --blob-map blob-map.json --report "Overview" --json \
  | jq -e '.measures[] | select(.name == "Total Sales") | .formatted == "$1,234,567"'
```

`--report` applies that report's report-level filters (a unique part of the name is enough). Untranslatable filters are listed in `unsupportedFilters`. Exit code 2 means a table or measure failed.

### Render pages and compare with a baseline

Renders are deterministic: the same inputs and DashCraft version give byte-identical PNGs on every OS (bundled fonts only, white background, no timestamps). Pin the clock with `--now` when the model computes "last refresh" values.

```sh
dashcraft render report.pbip --blob-map blob-map.json --report Overview --all-pages \
  --out-dir target/tmp/render --now 2026-01-01T00:00:00Z --force --json > render.json
jq -e '.ok' render.json                                       # false if a visual failed (exit 2 too)
jq -r '.pages[].visuals[] | select(.status != "ok") | "\(.name) \(.visualType) \(.status) \(.error // "")"' render.json
cmp target/tmp/render/01-Overview.png approved/01-Overview.png   # pixel-exact regression check
```

Select slicer values headlessly with `--slicer "Region=West"` (repeatable; `!=` inverts, `=` alone clears). Use `--scale 1` for smaller files.

To compare against **Power BI service exports** (`ExportTo` PNGs) rather than a previous DashCraft render, use the SSIM comparison in `pbi_render::golden` (alignment, per-visual SSIM, heatmaps), as `crates/pbi-render/tests/golden.rs` does:

```sh
cargo test -p pbi-render --test golden -- --nocapture   # scores and heatmaps in target/tmp/golden/
```

Renders and goldens show the report's data: keep them out of version control.

### Exports

```sh
dashcraft export report.pbip --model --format md --out DATA-DICTIONARY.md --force     # model docs, no data
dashcraft export report.pbip --model --format json --no-expressions --out model.json
dashcraft export report.pbip --table Orders --out orders.csv --blob-map blob-map.json
dashcraft export report.pbip --table Orders --out orders.csv --formatted --bom --excel-safe --blob-map blob-map.json
dashcraft export report.pbix --table Sales --format parquet --out sales.parquet         # PBIX: embedded data
```

`--excel-safe` prefixes text cells that start with `=`, `+`, `-`, `@`, TAB or CR with `'`, so a spreadsheet does not run them as formulas. Parquet files carry the model metadata (types, descriptions, format strings).

### Convert for code review

```sh
dashcraft convert report.pbix --to pbip --out report-project     # TMDL + PBIR: diffable text
dashcraft convert project.pbip --to pbit --out template.pbit --report "Sales Report"
```

### Example GitHub Actions job

```yaml
- name: Check the Power BI project
  run: |
    dashcraft inspect --json reports/Sales.pbip > inspect.json
    dashcraft lint reports/Sales.pbip --deny warnings
    dashcraft render reports/Sales.pbip --all-pages --out-dir out/render --source m \
      --blob-map ci/blob-map.json --now 2026-01-01T00:00:00Z --json > render.json
```

Run it against fixtures you are allowed to store in that repository: inspect output, renders and exports contain the model and its data.
