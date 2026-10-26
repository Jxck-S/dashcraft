# DashCraft desktop app

Tauri 2 + Svelte 5 + TypeScript + Vite. The Rust side is the `dashcraft-app` crate (binary
`dashcraft-app`; the crate keeps its internal name, [ADR-0009](../docs/adr/0009-naming-dashcraft.md)) in
[`src-tauri/`](src-tauri/), a member of the root Cargo workspace. It is **not** in
`default-members`, so a plain `cargo build`/`cargo test` at the repo root skips it, and
headless CI does not need webview libraries.

**The app is a thin front end over the command registry** (`pbi-ops`,
[ADR-0007](../docs/adr/0007-command-registry.md)): documents live in a `pbi_ops::Workspace`, and
every domain operation — open / close, refresh, grid pages, statistics, CSV loads, derived
columns, DAX queries, measures, report filters, the interactive canvas, exports — is a registry
command run by the one generic IPC command `ops_execute` → `Registry::execute`, exactly as
`dashcraft run`, `dashcraft serve` and AI agents run it (see [Architecture](#architecture-an-adapter-over-the-command-registry)).

```
app/
  index.html, vite.config.ts, svelte.config.js, tsconfig*.json
  docs/desktop-ui-reference.md   Desktop element → Desktop location → DashCraft status, with sources
  src/                 Svelte UI
    App.svelte         the Desktop-style shell: title bar, ribbon, view rail, the five views, right
                       panes, page tabs, status bar; document, refresh, CSV and selection state
    app.css            theme tokens (light Desktop palette) and the Selawik / Carlito @font-face rules
    assets/icons/      the Fluent UI System Icons used (SVG, MIT — LICENSE)
    assets/fonts/      Selawik (SIL OFL 1.1 — OFL.txt), the Segoe UI fallback; carlito/: Carlito
                       (SIL OFL 1.1 — carlito/OFL.txt), the report canvas's Calibri
    components/shell/  TitleBar (QAT, title, search), Ribbon, Menu, Backstage (File), ViewRail,
                       Pane (collapsible right pane), PageTabs, FormulaBar, StatusBar
    components/panes/  DataPane, FiltersPane, VisualizationsPane, SelectionPane
    components/views/  ReportView + ReportCanvas (the rendered, interactive page), TableView (DataGrid,
                       ColumnStatsPanel, PeopleDialog), ModelView (diagram), DaxQueryView,
                       TmdlView, QueriesView, MeasuresPanel
    components/ai/     the opt-in AI features: OptionsDialog + AiOptionsPage (Options → AI),
                       ConsentDialog (previews the exact request), ExplainPane, SuggestMeasureDialog,
                       Markdown (safe rendering of answers)
    components/        PropertiesPane, PowerQueryDialog, RelationshipsDialog, RefreshPanel,
                       CsvDialog, DiagnosticsPanel, Modal, CodeView, Toasts, DropOverlay, Icon
    lib/bindings.ts    GENERATED typed IPC client + events (tauri-specta), do not edit
    lib/ops.gen.ts     GENERATED types + `makeOps` wrappers of every command of the app's registry
                       (core + `ai.*`), do not edit (`npm run gen:ops`)
    lib/ops.ts         `ops.<command>(params)` over `ops_execute`: request ids, progress, cancel
    lib/backend.ts     the UI's gateway: the current document, registry calls adapted to view models
                       (grid cells, tree ids, canvas pages by id), app commands, dialog, drag-drop
    lib/ai.ts          AI availability, status, the consent flow (`runAi`: refused → dialog → retry)
    lib/markdown.ts    minimal Markdown → fully escaped HTML for AI answers (no DOM; links as text)
    lib/ribbon.ts      the ribbon of each view (tabs, groups, buttons, enabled state, actions)
    lib/shell.ts       shell types (views, zoom modes, ribbon controls, menus)
    lib/icons.ts       semantic icon names → vendored Fluent SVGs
    lib/visuals.ts     Visualizations gallery (Desktop order) and visual-type icons
    lib/canvas.ts      report canvas helpers: SVG sanitizer (DOMParser, id prefixing), hit ids, hit-testing
    lib/model.ts       UI helpers over the DTOs: search filtering, labels, Queries-view items
    lib/data.ts        UI helpers over the data results: table states, refresh runs, tooltips, SQL quoting
  scripts/gen-ops.mjs  runs the ops.gen.ts generator (a pbi-ai test) with or without --write
  tests/markdown.test.ts  the Markdown renderer's escaping tests (`node --test`, part of `npm run check`)
  src-tauri/           Rust crate `dashcraft-app` (lib `dashcraft_app_lib` + bin `dashcraft-app`)
    src/ops.rs         `ops_execute` / `ops_cancel` / `ops_catalog`: the bridge to `pbi_ops::Registry`
    src/ai.rs          installs the AI host (`<app config dir>/ai.json`, OS keychain); ai/demo.rs: the
                       debug-only demo provider (`DASHCRAFT_AI_MOCK=1`, `cfg(debug_assertions)`)
    src/commands.rs    app-only IPC commands: document views by `docId`, blob map, recent files
    src/events.rs      typed events (`ops://progress`, `open-file-requested`)
    src/dto.rs         DTOs of the app-only views (`#[derive(Serialize, specta::Type)]`)
    src/mapping.rs     pbi-core / pbi-model → DTO mapping (pure, panic-free)
    src/views.rs       TMDL view, diagramLayout.json, Format pane cards, report theme (pure, capped)
    src/recent.rs      File → Recent (newest first, persisted in the app config folder)
    src/blob.rs        swappable offline blob map (`blob-map.json`, `DASHCRAFT_BLOB_MAP`)
    src/state.rs       AppState: the registry (`pbi_ai::catalog()`), the workspace, blob map, recent
                       files, running requests (cancel tokens), progress sink
    tauri.conf.json    window, CSP, bundle targets (app/dmg/msi/appimage/deb)
    capabilities/      ACL: core:default + window title + dialog:allow-open / allow-save only
    icons/             generated from `icons/app-icon.svg` with `scripts/make-app-icon.sh`
```

## The Desktop shell (m4-desktop-shell)

The window follows Power BI Desktop's layout, button names and locations, read-only.
[`docs/desktop-ui-reference.md`](docs/desktop-ui-reference.md) lists every element with its
Desktop location (sources: Microsoft Learn) and what DashCraft does with it. In short:

- **Title bar**: DashCraft mark, Quick Access Toolbar (Save / Undo / Redo, disabled), centred
  `<file> - DashCraft` (also the native window title), and **Search** (Alt+Q) over the current
  ribbon's commands, the views and the model's tables, columns and measures.
- **Ribbon**: **File** opens the backstage (Home, Open with **Recent**, Export, Options and
  settings → **Options** (the opt-in AI page) and Data source settings = **Blob map…**, Get help →
  About; Save / Publish disabled). The
  tabs depend on the view, as in Desktop: Report view **Home, Insert, Modeling, Design, View,
  Optimize, Help** (+ Measure / Column / Table tools for a selected field); Table view Home, Help,
  Table tools, Column tools; Model view Home, Help; DAX query and TMDL views Home (Clipboard,
  Editing), Help. Commands DashCraft does not implement are greyed with the tooltip "Not available
  in DashCraft (yet)". Collapse with the chevron, Ctrl+F1 or a tab double-click.
- **Enabled commands**: Home → **Transform data** (read-only Power Query Editor: M queries,
  parameters, DAX — the former Queries tab) with its menu (**Data source settings → Blob map…**,
  **Edit parameters**, **Load CSV manually…** per table), **Refresh**; **Manage relationships**
  (Home in Model view, Modeling); Design → Themes gallery (the report theme, read-only); View →
  **Page view** (Fit to page / Fit to width / Actual size), **Filters** and **Selection** panes,
  DashCraft's **Properties** pane, **Reset to default** and **Outline**; DAX query view **Comment / Uncomment**; Table
  tools **Refresh table**, **Load CSV…**, **Rows / Schema**; Column tools **Column statistics**,
  **Parse people field…**; Help → **About**; the opt-in **AI** group (Home, Modeling, Measure
  tools: **Suggest a measure**, **Explain measure**, **AI options**; disabled until AI is on).
- **View rail** (left): Report, Table, Model, DAX query, TMDL.
- **Report view**: the page on a gray surround (`ReportCanvas`: the page rendered by pbi-render,
  interactive, see *The interactive report canvas*), zoomed per Page view, the status bar slider
  or Ctrl+wheel; page tabs (hidden pages
  greyed, ‹ › and Ctrl+Page Up / Down, "+" disabled; a report picker when a PBIP has several
  reports); right panes **Filters** (cards on this visual / page / all pages as in reading view:
  cards hidden in view mode left out unless "Show n hidden filter cards" is ticked, locked cards
  with a lock; the canvas's slicer selections and cross-filter as "Slicer: …" / "Selection: …"
  cards with a clear button; search), **Visualizations** (Build: gallery with the selected visual's type, field
  wells; Format: the visual's formatting objects as read-only cards), **Data** (Desktop field
  icons, checkboxes for the selected visual's fields), each collapsible to a strip; the
  **Measures** panel (former KPI strip) docked under the canvas, collapsed by default, evaluated
  in the canvas's filter context (report + page filters, slicers, cross-filter) by default, also
  feeding the Data pane's measure tooltips.
- **Formula bar** (Report and Table views): the DAX of the selected measure, calculated column or
  calculated table, read-only.
- **Table view**: the data grid of the table chosen in the Data pane, SQL filter bar, column
  statistics, column menu (sort, statistics, filter, **Parse people field…**), Schema mode;
  status bar `Table: … (n rows) Column: … (n distinct values)`.
- **Model view**: diagram of table cards (positions from `diagramLayout.json`, else a layered
  auto-layout) and relationship lines with cardinality and cross-filter arrows (inactive dashed);
  drag to pan, Ctrl+wheel / status bar zoom, Fit to screen, layout tabs; Properties and Data panes.
- **DAX query view**: editor (F5 / Ctrl+Enter runs the query or the selected lines, Ctrl+/
  comments), Run ↔ Cancel, results grid (Result n of m, Copy as TSV), query tabs with status;
  Data pane **Quick queries** (Show top 100 rows, Evaluate, Define and evaluate, Show data
  preview).
- **TMDL view**: the model as TMDL scripts, read-only (the project's files verbatim, or TMDL
  generated from TMSL / PBIX models).
- **Status bar**: view text (Page X of Y, table / column, query status), the status message, data
  state (cached / loaded / refreshing / errors → refresh panel), diagnostics counters (→
  diagnostics panel), zoom slider with percentage and Fit.
- Look: Desktop's light palette (off-white chrome, white panes, gray canvas surround, teal
  selection, the yellow accent sparingly), Fluent UI System Icons (MIT), Selawik (OFL) after
  Segoe UI; report text in Calibri uses Carlito (OFL). No Microsoft Power BI icons, images, logos
  or CSS are used.

## Features (m1-app, m2-grid)

Everything is read offline by the registry's `doc.open` (`pbi_session::Session::open`:
content-based detection, safe containers) into the workspace; the webview only displays command
results and DTOs. Locations below refer to the Desktop shell above.

- **Open** a file or folder with File → Open / the start page (Ctrl/⌘+O, Ctrl/⌘+Shift+O), File →
  Recent, drag and drop, or `dashcraft-app <path>`. Parsing runs on a blocking worker; a spinner
  covers the panes and failures show as error toasts. After a webview reload the backend's
  document is shown again without reparsing (`doc.list` + `document_views`).
- **Data pane**: model tree. Tables (icons for regular / calculated / remote tables) expand to
  measures, columns and hierarchies sorted by name, as in Power BI Desktop. Hidden objects are
  greyed with an eye icon ("View hidden" in the ⋯ menu), and the search box matches table, field
  and source-column names. Tooltips show the **source column when it differs from the display
  name**, the data type, the description and, for measures, the current value.
- **Report view**: page tabs, the **rendered page** (see *The interactive report canvas*) or the
  **Outline** (visual, position, fields by role, visual filters), plus the Filters pane (visual /
  page / report filters with DAX-like summaries and hidden / locked icons). Field labels are the
  projection display names; the model reference (`'Order Lines'[Quantity]`, `Sum(T[C])`) is in the
  tooltip.
- **Data on open** (`DocumentSummary.onOpen`, as Power BI Desktop does): a PBIX, ABF or Power
  Pivot file loads its saved data right after opening (`data.refresh` with source `cache`, quietly,
  no banner); a PBIT refreshes from the source (`query`), with a banner about the template's saved
  parameter values when it has parameters; a PBIP without a cache stays empty until Refresh.
- **Local data cache**: when the opened PBIP has `<Model>.SemanticModel/.pbi/cache.abf` (Power BI
  Desktop's last refreshed data; `DocumentSummary.cache`), it is loaded right after opening
  (`data.refresh` with source `cache`: no M, no network, no blob map) and a banner says *Loaded cached
  data (as of <file time>)* with a **Refresh from source** button (runs the M queries). Tables
  missing from the cache fail with the CSV fallback; calculated objects missing from it are
  computed.
- **Refresh** (Home → Refresh; Table tools → Refresh table; Data pane → Refresh data): runs the M partitions offline
  (pbi-m, concurrently), decodes PBIX data, then computes calculated columns and tables. A panel
  shows per-table progress (the `ops://progress` events of the `data.refresh` request) with
  **Cancel** (`ops_cancel`; the session keeps its
  previous data), then the results: rows, source kind, duration, and for failures the message,
  the hint and a **Load CSV manually…** button when the status allows the CSV fallback. A
  successful refresh closes the panel with a toast; failures leave it open (or the status bar's
  *Refresh: n errors* reopens it).
- **Blob map…** (Transform data ▾ → Data source settings, or File → Options and settings): pick
  the `blob-map.json` that maps `AzureStorage.Blobs` URLs to local
  CSV files (no network). It applies to the next refresh without reopening the document; × goes
  back offline ("Stop using the blob map"). `DASHCRAFT_BLOB_MAP=<path>` sets it at startup. Without a map, blob tables fail with
  the offline hint and offer the CSV fallback.
- **Load CSV manually…** (refresh panel, Transform data ▾, Data pane menu, Table tools, Table view
  error banner): file picker (csv/tsv/txt),
  delimiter (comma, semicolon, tab, pipe, other), header row, encoding (detect, UTF-8,
  Windows-1252, UTF-16 LE). Dependent calculated objects are recomputed; conversion warnings show
  in a toast.
- **Data pane chips**: rows per table (`412`, `12.3K`, `—` when not loaded) coloured by the last
  result (ok / error / column errors / skipped); the tooltip lists the kind, duration, error,
  hint and failed calculated columns.
- **Table view** (*Rows* mode): a virtualized grid of the selected table. Only the visible rows are
  in the DOM; pages of 200 rows are fetched as you scroll (sorted and filtered by the engine;
  very tall tables scale the scrollbar). Sticky header and row numbers, resizable columns, numbers
  right-aligned, blanks greyed as `(Blank)`, cells formatted with the column format string in the
  model culture. Header tooltip: kind, **source name**, data type, format, expression. Click a
  header to sort ascending → descending → none; right-click (or ⋯) for sort, statistics, *Filter
  on this column*, *Parse people field…* (text columns) and *Remove derived column*. The filter
  box takes a SQL `WHERE` expression (Enter applies, Esc clears; ⓘ shows the syntax and inserts
  quoted column names); errors show inline. A status chip and banners show the table's state
  (not loaded, refresh error with hint and CSV fallback, remote table).
- **Column statistics** panel (click a cell, use the menu, or select the column in the Data pane):
  rows, count, blanks (%), distinct, min, max (whole table), type, source and format.
- **Parse people field…**: preview on a text column (outcome counts over the whole column and
  distinct samples, raw → parsed) then *Add* creates the session-only column `<column> (parsed)`
  (`First Last; …`), a `pbi-session` derived column. It sorts, filters and has statistics like any
  column, DAX measures and SQL filters can use it, it is recomputed after each refresh and CSV
  load, and it is never saved to the model.
- *Schema* mode of the Table view (Table tools → Schema): columns, data type, kind, source column, format, summarize-by,
  plus derived columns.
- **Measures panel** (former KPI strip, under the report canvas): every model measure evaluated by pbi-dax
  with its format string, ⚠ *Error* / *Unsupported* cards whose tooltip has the message and the
  DAX. The filter selector applies, by default, **as on the canvas** (the page's
  `effectiveFilters`: report + page filters, slicer selections and the cross-filter), or none,
  the active **report's filters**, or report + page filters (`filters_from_report` /
  `filters_from_page`), so filtered report variants show their own values; applied filters are
  listed as chips and untranslatable ones (Top N, advanced, range slicers, …) as ⚠ chips.
  Clicking a card selects the measure. Hidden measures are behind a toggle.
- **Model view** diagram and **Manage relationships** (read-only list): relationships (`from * ⇄ 1 to`, cardinality, single / both direction icon,
  inactive dashed) and table cards with relationship keys marked.
- **View as** (Modeling → Security, or Home → Security in the Model view; disabled without
  roles): the *View as roles* dialog (`components/ViewAsDialog.svelte`) lists the model's roles
  with their DAX table filters (`security.roles`); check roles and / or **Other user** (the
  `USERPRINCIPALNAME()` of dynamic roles), OK runs `security.viewAs` (a bad filter's error stays
  in the dialog). While active a yellow *Now viewing as: …* bar offers **Stop viewing**; the
  canvas, grid, Measures panel and Data pane re-query (`dataVersion`). Never saved.
- **Transform data** (read-only Power Query Editor): M partitions, shared M queries, parameters, DAX measures, calculated columns
  and calculated tables, filterable by name or text, in a read-only code view (monospace, line
  numbers, copy). No Monaco, to keep the bundle small.
- **Export** (File → Export): the document as a **PBIP project** (an empty folder) or a **PBIT
  template** of the current report (save dialog), through the registry's `convert.pbip` /
  `convert.pbit` (the opened file is never changed). PDF export stays disabled.
- **Properties pane** (Model view; View → Properties or the Data pane menu elsewhere): `object_details` for whatever is selected anywhere (table, column, measure,
  partition, relationship, expression, report, page, visual, filter): typed properties, full DAX /
  M / JSON text, annotations, unmodelled properties (`extra`) and the source file with line or
  JSON pointer.
- **Status bar**: view text, status message, data state and diagnostics by severity (format and
  file in the tooltip). Clicking the diagnostics counter opens the diagnostics panel (severity, code, message,
  origin; messages linked to an object select it) with a Detection tab (format, confidence,
  signals, models, reports, parts). Esc closes it.
- **Remote models** (`byConnection` / live connection): a banner "Remote model — not available
  offline"; pages, visuals and fields are still shown.

### The interactive report canvas (m4-interactive)

`components/views/ReportCanvas.svelte` shows the page rendered by the Rust renderer (pbi-render:
semantic query → DataView → built-in visuals → scene → SVG) and makes it interactive like Power
BI's reading view. Every interaction is a `canvas.*` registry command (the same commands as
`dashcraft run canvas.click …` and agents); the registry's canvas (`crates/pbi-ops/src/canvas.rs`)
keeps, per open document and per (report, page), an `InteractionState` (slicer selections and selected data points, seeded from the
selections saved in the report by `InteractionState::from_page`) and a view state (table / matrix
scroll and expansion, the open slicer dropdown with its search text and scroll); going back to a
page keeps them.

- **Rendering.** `canvas.render` runs on a blocking worker: it queries every visible visual
  (`pbi_render::visuals::query_page_data`), renders the scene with the view state
  (`render_interactive_page`) and serializes it with `pbi_render::svg::to_svg` (deterministic).
  See **Canvas update model** below for generations, caching and coalescing. The page re-renders
  on page change and on every data change (`dataVersion`); a shimmer covers the page until the
  first render and a thin progress bar shows slower re-renders.
- **Safe insertion.** The SVG string is parsed with `DOMParser`, then `lib/canvas.ts` removes
  scripts, `foreignObject`, styles, links, animations, `on*` attributes and non-local / non-image
  `href`s, prefixes every `id` (and its `url(#…)` references, so the page and the overlay share
  the document without clip-path clashes) and imports the element. It fills the page box, so
  zooming (Fit to page / width, Actual size, the slider, Ctrl+wheel) scales the vectors without a
  re-render.
- **Fonts.** The SVG carries no `@font-face`: its text uses `font-family="Selawik, 'Segoe UI',
  sans-serif"` with the bundled faces' weights (300 / 350 / 400 / 600 / 700), or
  `font-family="Carlito, Calibri, sans-serif"` (400 / 700, upright or italic) for text in
  Calibri, which `app.css` declares from the app's own font assets (`src/assets/fonts`,
  `font-src 'self'`), copies of the files the renderer measures with
  (`crates/pbi-render/assets/fonts/selawik` and `…/carlito`).
- **Hit-testing.** Pointer events on the SVG's `data-hit` groups (`<visual>|visual`,
  `…|dataPoint|<series>|<index>`, `…|slicerItem|<index>`, `…|button`); `hitRegions` carry the same
  ids with rectangles in page units.
- **Slicers.** A click on a dropdown box opens its popup, drawn in an **overlay layer** above the
  page and not clipped to it (`overlaySvg`, `overlayHitRegions`); a click elsewhere or Esc
  closes it. Items select per the slicer's settings: *Single select* (`strictSingleSelect`)
  replaces; *Multi-select with Ctrl* (`singleSelect`, the default) replaces on click (clicking the
  only selected item clears it) and toggles on Ctrl/⌘+click; otherwise every click toggles; with
  everything checked (inverted selection mode) unchecking an item excludes it; **Select all**
  switches between all and none. The wheel scrolls a list or popup; with `selfFilterEnabled`,
  clicking the search box opens a text field over it (`canvas.setSlicerSearch`, debounced; Esc clears,
  then closes). Hovering a slicer with a selection shows the **Clear selections** eraser at the
  right end of its header.
- **Cross-filtering.** A click on a data point (pie slice, bar, line point, table row, matrix
  row) selects it as a `PointSelection` (category and series fields; a table row's group
  columns; a matrix row's path) that filters the other visuals per the page's
  `visualInteractions` (`NoFilter` skips a target, `HighlightFilter` filters); the source visual
  dims its other points. Ctrl/⌘+click adds or removes points; clicking the only selected point
  again, the visual's background, the empty canvas or Esc clears the selection.
- **Tables and matrices.** The wheel (Shift+wheel: horizontally) scrolls the body; double-click a
  matrix row (or click its +/− button) to expand or collapse it.
- **Tooltips.** Hovering a data point shows a Desktop-like tooltip from the last render's
  DataView (`canvas.tooltip`: category, series, value and `Tooltips` fields; a table row's cells;
  a matrix row path and cell). Hovering a visual whose query failed shows ⚠ with the message and
  the measure's DAX (the visual itself draws "Can't display this visual"); buttons show their
  tooltip.
- **Buttons and navigation.** Action buttons return an action: **Back** goes to the previous page
  of the navigation history (page tabs, Ctrl+Page Up / Down and page navigation buttons push to
  it; nothing to go back to shows a toast), **Page navigation** goes to the page, *Clear all
  slicers* clears them, *Apply all slicers* does nothing (selections apply at once). **Web URL**
  opens a dialog that shows the URL with **Copy link**: DashCraft never opens links. Bookmarks,
  drillthrough and Q&A show a "not available" toast.
- **Selection.** Clicking a visual selects it for the Visualizations (Build / Format), Data,
  Filters ("Filters on this visual") and Properties panes, with a thin Desktop-like outline.
  Visuals are keyboard-focusable too (Tab, Enter selects), with a focus ring on keyboard focus
  only.
- **Reset to default** (View → DashCraft group, enabled when the page differs from the report's
  saved state): the saved slicer selections, no cross-filter, scroll and dropdowns reset.
- **Consistency.** Every render returns, from the state its visuals were queried with, the page's
  `selections` (slicer selections and the cross-filter, shown as Filters pane cards) and
  `effectiveFilters` (report + page filters + slicer selections + cross-filter as a `measures`
  filter spec). The Measures panel's default scope uses them, so its KPIs follow the cards
  (per-visual interactions aside): the same value selected in a slicer or clicked in a pie gives
  the same measure values in both. Parts that cannot be translated (range
  slicers, multi-field selections, multi-field cross-filters with several points) are listed as ⚠
  chips.

#### Canvas update model

Every interaction (click, wheel, search, popup, reset) changes the page's state in the backend and
re-renders the page; the newest completed render is shown and the page never moves backwards:

- **Generations (backend).** Each render bumps the page's generation. A change of the interaction
  state cancels the running renders of the previous state (their data is obsolete); renders of the
  same state (scrolling, dropdowns, search) run to completion. A finished render is kept — and is
  what clicks and tooltips read — when it is newer than the newest completed one, even while a
  later render still runs; an older one fails with `cancelled` (at most 4 renders per page run at
  once).
- **Generations (front end).** `ReportCanvas` drops any response for another page (renders name
  their report and page) or older than the one shown, so out-of-order completions are ignored.
- **Ordering (registry).** `canvas.*` commands change the document's view state, so
  `Registry::execute` runs them one at a time per document, in submission order (with one wheel
  request in flight this costs nothing; a click waits for a running render, and canvas commands
  wait for a running `data.refresh`). Through `ops_execute` a scroll step of a report table
  takes about 0.6 ms (p50, release, including the JSON result), a dropdown toggle about 0.7 ms.
- **Only what changed is re-rendered (backend).** Query results are cached per page while the
  interaction state and the data (engine snapshot and model) are unchanged, so view-only changes
  skip the queries. pbi-render's visual cache then reuses every visual whose inputs (definition,
  frame, data, view state, selection, theme) did not change: a scroll step re-renders the scrolled
  table only, from its cached grid model and layout (no work proportional to the table's rows), and
  a slicer click re-renders only the visuals whose data changed. See
  [docs/VISUALS.md](../docs/VISUALS.md#interactive-re-render-cost) for measurements.
- **Coalescing (front end).** Wheel deltas accumulate per visual while one scroll request is in
  flight and are sent as one delta when it returns. The SVG of the newest accepted render is parsed
  and swapped at most once per animation frame, and not at all when its text is unchanged (the
  overlay during a table scroll, a no-op click).
- **Whole-page replacement.** The page SVG is still replaced as a whole. Per-visual patches (send
  only the changed visuals' `<g>` and swap them in the DOM) were left out: they need a protocol
  change (fragments plus the generation they apply to, a full-render fallback when the front end
  shows another generation), and clip-path ids are numbered across the whole document, so a
  fragment's `<clipPath>`s need their own id scope and the replaced visual's old definitions must
  be removed. With the backend re-rendering only the changed visual, a scroll step costs about
  0.2 ms to render and 0.3 ms to serialize (release), so parsing the ~40 KB page SVG in the webview
  is the remaining cost.

`ReportCanvas` props: `report`, `page` (`page.width` × `page.height` page units), `zoom` (1 =
100%), `filters` (report + page filters, informational), `dataVersion` (re-render),
`selectedVisualId` / `onselectvisual(id | null)`, `incoming` (a render made elsewhere — Reset to
default, Filters pane clear buttons — shown if newer), `onrendered(page)`, `onaction(action)` and
`onerror(message)`. Without a backend (the `npm run dev` browser preview) it draws a wireframe.

## Run

Prerequisites: the Rust toolchain from `rust-toolchain.toml`, Node 20.19+ or 22.12+ (Vite 8), and npm.

```sh
cd app
npm install
npm run tauri dev                  # Vite on :1420 plus the native window, with hot reload
npm run tauri dev -- -- path.pbix  # open a file on startup (args after the 2nd `--` go to the app)
```

| Script | What it does |
|---|---|
| `npm run dev` | Vite dev server only. The UI runs in a browser in "browser preview" mode: no IPC/dialog/drag-drop, so no file can be opened |
| `npm run build` | Production build into `app/dist/` (embedded by `tauri build`). Run `npm run check` first for type errors |
| `npm run check` | `svelte-check` (app sources) + `tsc` (Vite config) |
| `npm run tauri -- build` | Release binary plus installers for the host OS (`src-tauri/tauri.conf.json` → `bundle.targets`) |
| `npm run bindings` | Regenerates `src/lib/bindings.ts` (see below) |

Rust checks for the crate (from the repo root):

```sh
cargo build -p dashcraft-app
cargo clippy -p dashcraft-app --all-targets -- -D warnings
cargo test -p dashcraft-app          # also regenerates bindings.ts
```

A plain `cargo build` runs in Tauri "dev" mode, which loads `devUrl` (http://localhost:1420).
Only `tauri build` embeds `dist/`.

### Opening files

- **File → Open** (Browse this device / Open a folder), the start page and Ctrl/⌘+O,
  Ctrl/⌘+Shift+O use `@tauri-apps/plugin-dialog`; **File → Recent** reopens a recent path.
- **Drag and drop** anywhere on the window. This uses the Tauri webview `onDragDropEvent`, which
  delivers real filesystem paths (HTML5 drop events do not).
- **Command line**: `dashcraft-app <path>`. The first non-flag argument is made absolute and stored
  in state. The UI fetches it on startup with the `startup_path` command.

All four (with macOS Finder's open requests, `open-file-requested`) end up in `doc.open`
(`autoRefresh: false`: nothing is loaded until a refresh) followed by `document_views`; the
previously shown document is closed (`doc.close`).

## Architecture: an adapter over the command registry

```
Svelte components ──► lib/backend.ts ──► lib/ops.ts  ops.dataPage({…})  ──► ops_execute(requestId, id, params)
                         │                (ops.gen.ts: types + wrappers)        │  Registry::execute(ws, id, params, Ctx{App})
                         │                                                       ▼
                         │                                            pbi_ops commands (same as CLI / RPC / MCP)
                         └──► bindings.ts commands.documentViews(docId) … ──► app-only commands (views, settings)
```

- **One code path.** `ops_execute(requestId, id, params)` runs registry command `id` with JSON
  parameters through `pbi_ops::Registry::execute` on the app's `Workspace` (unrestricted path
  policy, as in the CLI: files come from the OS dialog, drag and drop, the command line or
  Finder) with `CallerKind::App`. Parameter validation, name resolution, per-document ordering of
  mutating commands, cancellation and panic isolation are the registry's. Errors are
  `OpErrorDto { code, message, data }` with the registry's stable codes (`invalidParams`,
  `notFound`, `noDocument`, `unsupported`, `query`, `cancelled`, `busy`, `internal`, …).
- **Progress and cancellation.** `Ctx` progress events are emitted as `ops://progress`
  (`OpsProgress { requestId, command, message, fraction, data }`; for `data.refresh` `data` is the
  `pbi-session` refresh event). `ops_cancel(requestId)` cancels the run's token: the request fails
  with `cancelled`. `lib/ops.ts` gives every call a request id, routes its progress events, and
  `latest: key` cancels the previous request with the same key (measures, DAX queries).
- **Typed client.** `src/lib/ops.gen.ts` is generated from the registry's JSON Schemas (one
  TypeScript type per parameter / result type, the `OpsCommands` id → types map, `OPS_META`, and
  `makeOps(run)` with one camelCase wrapper per command). `lib/ops.ts` instantiates it over
  `ops_execute`: `await ops.canvasClick({ docId, report, page, hitId, renderUnchanged: false })`.
  It covers the app's whole catalog, `pbi_ai::catalog()` (the core commands plus the opt-in
  `ai.*`). The generator is the `ops_typescript_is_up_to_date` test of
  `crates/pbi-ai/tests/registry.rs` (`pbi-ops` cannot depend on `pbi-ai`; `pbi-ai` is a default
  member, so CI's `cargo test` still catches a stale file), and the app's
  `app_registry_is_the_generated_catalog` test checks that the app runs exactly that catalog (no
  npm dependency):

  ```sh
  npm run gen:ops     # regenerate src/lib/ops.gen.ts after a command type changes (commit it)
  npm run check:ops   # fail if it is stale (also part of `cargo test`, so CI catches it)
  ```
- **The current document.** The workspace can hold several documents; the UI shows one and
  `lib/backend.ts` keeps its `docId`, model tree and report outline (`setCurrent`). It adapts
  results to the components' view models: grid columns get their model metadata (kind, source
  name, object id) from the tree and the derived columns of `data.tables`; cells come from
  `data.page` with `formatted` + `blankAsNull`; measure values get their tree ids; canvas commands
  address reports and pages by name (the outline's ids are mapped).
- **App-only commands** (tauri-specta, below) are what the registry does not model: UI
  projections of an open document with session object ids for selection, the Properties and
  Format panes (`document_views`, `object_details`, `visual_format`, `tmdl_view`,
  `model_diagram`), the app-wide blob map (installed into every `doc.open` through the
  workspace's open hook, so it can change without reopening), File → Recent, and window plumbing.

| Registry command | Used for |
|---|---|
| `doc.open`, `doc.close`, `doc.list` | open (dialog, recent, drag and drop, argv, Finder), close, webview reload |
| `data.refresh`, `data.tables` | Refresh / cache load (progress, cancel), table statuses and chips |
| `data.page`, `data.columnStats` | Table view grid pages (sort, SQL filter), column statistics |
| `data.loadCsv`, `text.previewPeople`, `data.addDerivedColumn`, `data.removeDerivedColumn` | Load CSV manually…, Parse people field…, Remove derived column |
| `dax.query`, `measures.values`, `report.filters` | DAX query view, Measures panel (canvas, report or page filters) |
| `canvas.render`, `canvas.click`, `canvas.scroll`, `canvas.toggleMatrixRow`, `canvas.clearSelections`, `canvas.setSlicerSearch`, `canvas.closePopups`, `canvas.reset`, `canvas.tooltip` | the interactive report canvas |
| `convert.pbip`, `convert.pbit` | File → Export |
| `ai.status`, `ai.settings.set`, `ai.apiKey.set`, `ai.apiKey.clear`, `ai.consent.revoke` | Options → AI (opt-in; see [AI features](#ai-features-opt-in)) |
| `ai.preview`, `ai.consent.grant` | the consent dialog (the only place a consent is granted) |
| `ai.explainMeasure`, `ai.generateMeasure` | Explain with AI, Suggest a measure |

## Typed IPC (tauri-specta)

Each app command carries `#[tauri::command] #[specta::specta]` and is listed in
`specta_builder()` in `src-tauri/src/lib.rs`. The `export_bindings` unit test writes
`src/lib/bindings.ts` from that list. No window is launched.

```sh
npm run bindings   # = cargo test -p dashcraft-app --lib export_bindings
```

Run it after changing any command signature or IPC type, and commit the regenerated file.

| Command | Returns | Notes |
|---|---|---|
| `ops_execute(requestId, id, params)` | `Json` / `OpErrorDto` | runs registry command `id` (see above); `requestId` must be unique while running |
| `ops_cancel(requestId)` | `boolean` | cancels a running request (`false` if none) |
| `ops_catalog()` | `OpsCommandDto[]` | ids, titles, summaries, categories and flags of every command |
| `app_version()` | `string` | |
| `ai_demo_mode()` | `boolean` | the debug-only AI demo provider is active (`DASHCRAFT_AI_MOCK=1`); always `false` in release builds |
| `startup_path()` | `string \| null` | a document macOS asked to open (handed out once), else the `dashcraft-app <path>` argument |
| `document_views(docId)` | `DocumentViewsDto` | `{ summary, tree, reports, diagnostics, detection }` of an open document: `summary` (`docId`, path, file name, entry kind, size, format, model status, counts, diagnostics by severity, `cache { entry, sizeBytes, modified }` of a PBIP local data cache); `tree` (tables → columns / measures / partitions / hierarchies, relationships, named expressions; empty with `status.availability = remote \| missing`); `reports` (pages → visuals with type, position, title, fields, filters with a DAX-like summary, `theme`); reader `diagnostics`; `detection` (format, confidence, signals, items, parts) |
| `object_details(docId, id)` | `ObjectDetailsDto` | property sheet + code blocks + annotations + `extra` + origin for any object ID |
| `tmdl_view(docId)` | `TmdlViewDto` | `source: files` (the opened `.tmdl` files verbatim, paths relative to `definition/`) or `generated` (`pbi_tmdl::write_model` for TMSL / PBIX models); `files[{path, text, table}]` in Desktop folder order; `unsupported` without a local model |
| `model_diagram(docId)` | `ModelDiagramDto \| null` | Model view layouts from `diagramLayout.json`, read leniently; `null` without the file |
| `visual_format(docId, id)` | `VisualFormatDto` | Format pane: `visual` / `general` cards with readable values, plus `layout`; `unknownObject` for non-visual IDs |
| `blob_map()` | `BlobMapDto` | active `blob-map.json` (`path`, `fromEnvironment`, `environmentVariable` (`DASHCRAFT_BLOB_MAP`), `error`) |
| `set_blob_map(path \| null)` | `BlobMapDto` | reads and activates a map (local files only) or goes offline; keeps loaded data; `busy` while a `data.refresh` runs |
| `recent_files()` | `RecentFileDto[]` | File → Recent, newest first (`path`, `fileName`, `openedAtMs`, `exists`) |
| `add_recent_file(path)` | `RecentFileDto[]` | after a successful open |
| `remove_recent_file(path \| null)` | `RecentFileDto[]` | removes one entry, or clears the list with `null` |

| Event | Payload | Notes |
|---|---|---|
| `ops://progress` (`events.opsProgress`) | `OpsProgress { requestId, command, message, fraction, data }` | progress of a running `ops_execute` request |
| `open-file-requested` (`events.openFileRequested`) | `{ path }` | macOS Finder / "Open With" while the app runs |

App commands fail with `noDocument` for a `docId` that is not open, and `object_details` with
`unknownObject` for IDs that are not in the document.

Events are tauri-specta typed events: `events.rs` implements `tauri_specta::Event` by hand (no
`tauri-specta-macros`), `specta_builder()` registers them, and `attach()` in `lib.rs` mounts them
on the built app and installs the progress sink of `ops_execute`. Tests build the app the same
way (`configure` + `build` + `attach`).

### Data session

`doc.open` opens the document into a `pbi_session::Session` held by the workspace. The workspace's
open hook gives every session the app-wide `SwitchableResolver` (`src/blob.rs`), so the blob map
can change without reopening. Grid pages, row counts and statistics run on immutable engine
snapshots (page and total come from the same snapshot). The registry serializes results as JSON;
`ops_execute` carries them as `Json` (`JsonValue` in TS: specta refuses `serde_json::Value`'s
64-bit integers) and `ops.gen.ts` types them per command.

**Derived columns.** "Parse people field" is `data.addDerivedColumn` with `kind: people`
(`pbi_session::DerivedColumn::People`): the column lives in the session's engine table, so the
grid, statistics, DAX measures and queries, SQL filters and exports all see it. The session
re-applies it after every refresh, cache load and CSV load, and it is never written to the model
or saved.

pbi-model types are **not** sent over IPC by the app-only commands: their lenient string enums
have hand-written serde, so `specta::Type` cannot be derived on them. `src-tauri/src/dto.rs`
defines UI-shaped DTOs instead and `src-tauri/src/mapping.rs` maps them (unknown enum values become
their raw string; filter conditions and nested fields are formatted with depth and length caps).
Object IDs are the document-unique `pbi_model::ObjectId`s as `u32`.

App commands that return `Result<T, AppError>` come out as
`Promise<{ status: "ok"; data: T } | { status: "error"; error: AppError }>`, where `AppError` is
`{ kind: "notFound" | "unsupported" | "invalidFile" | "noDocument" | "busy" | …; message: string }`.
`lib/backend.ts` and `lib/ops.ts` turn both kinds of errors into `Outcome<T>` with a readable
message and the error kind or code. Specta forbids implicit 64-bit integers: sizes are exported as
TS `number` (`#[specta(type = Option<f64>)]`), and page sizes / positions as plain `number`
(non-finite values are mapped to 0, so JSON never carries `null`).

Tests (`cargo test -p dashcraft-app`) call the commands through Tauri's mock runtime, without a
window: the wire contract and error shapes of `ops_execute` / `ops_cancel` / `ops_catalog` and the
app commands, request tracking and cancellation, the Acme Outfitters example
(`reference/examples/acme-outfitters/AcmeOutfitters.pbip`) through `doc.open` + `document_views`
(tables, measures and relationships, the reports whose page names are the ones `report.pages`
returns, a measure's DAX and origin in `object_details`, the report theme and its colours, Format
pane cards and layout, the TMDL view's verbatim table files, `diagramLayout.json` handling, File →
Recent, reopening and `doc.close`), a live-connected (`byConnection`) report (remote model, no
TMDL view, no measures), and the adapter end to end: a blob map chosen after opening applies to
the next `data.refresh`, whose progress arrives as `ops://progress` events of the request, then
`data.page`, `canvas.render` of a report and `measures.values` over its `effectiveFilters`,
checked against the example's expected values. The domain behaviour formerly tested here moved to
`crates/pbi-ops/tests`: the canvas on the example's reports (dropdown and list slicer selections,
table-row and pie cross-filters, measures over the effective filters, table scrolling, tooltips,
Back button, reset; `e2e.rs`), the PBIP local data cache, the offline refresh with the CSV fallback and CSV loads
(`e2e.rs`), "Parse people field" on a synthetic model (`canvas.rs`), and the canvas's "newest
completed render wins" ordering (`src/canvas.rs` unit tests).

Unit tests cover the Format pane value formatting, lenient `diagramLayout.json` reading, TMDL
paths and order (`views.rs`) and the recent-files store (`recent.rs`, persisted to a scratch file).

Fixtures come from `$DASHCRAFT_REFERENCE` or `<workspace>/reference` (the Acme Outfitters example and the Microsoft samples); missing optional ones print `SKIP`.

Versions are pinned exactly (`tauri-specta =2.0.0-rc.26`, `specta =2.0.0-rc.26`,
`specta-typescript =0.0.13`), because the 2.0 line is still a release candidate. `specta` enables
its `serde_json` feature for the JSON-carrying DTO fields.

Internal dependencies: `pbi-ops` (the command registry and workspace: every domain operation),
`pbi-core` and `pbi-model` (the app-only views), `pbi-session` (`SessionOptions` for the open hook,
cache info), `pbi-m` (the `BlobResolver` trait for the swappable blob map) and `pbi-tmdl` (TMDL view:
TMDL generated from TMSL / PBIX models), all through `[workspace.dependencies]`.

File → Recent is the only state DashCraft writes on its own: `recent-files.json` (paths and open
times, at most 20) in the OS app config folder (named after the bundle identifier
`io.github.jxck-s.dashcraft`), written atomically; tests keep it in memory. File → Export writes
only where the user chose in the dialog.

## AI features (opt-in)

The registry is `pbi_ai::catalog()` and `run()` installs the AI host (`src-tauri/src/ai.rs`):
settings in `<app config dir>/ai.json` (`io.github.jxck-s.dashcraft`; `DASHCRAFT_CONFIG_DIR`
shares one file with the CLI), keys in the OS keychain (service `dashcraft`,
shared with `dashcraft ai login`). AI is **off by default**; the UI ([docs/AI.md](../docs/AI.md#in-the-desktop-app)):

- **Options → AI** (File → Options and settings → Options, or Home → AI → AI options): turn on,
  providers (OpenAI-compatible, Azure OpenAI, Ollama), write-only API keys (`ai.apiKey.set` /
  `clear`; `CallerKind::App` is the only caller allowed to pass a key), default provider, sample
  rows, consents with Revoke. Each change applies at once.
- **Consent dialog**: an AI command refused with `data.reason = "consentRequired"` opens it; it
  shows `ai.preview` (endpoint host, local/remote, capability, data-exposure class, headers with
  keys redacted, the JSON body) and only **Allow and send** runs `ai.consent.grant` and retries.
- **Explain with AI**: Data pane menu, formula bar, Properties pane, Home → AI, Measure tools; a
  pane with progress, Cancel and the answer. **Suggest a measure**: Home / Modeling → AI; DAX with
  diagnostics, local evaluation (`dax.query`), never applied.
- While AI is off or has no provider, the AI buttons stay visible but disabled ("Turn on AI in
  Options"). The Copilot group of Desktop's Home tab is replaced by this neutral **AI** group
  (Fluent "Sparkle" icon); the Microsoft Copilot name and logo are not used.
- Answers are rendered by `lib/markdown.ts`: every character escaped, only its own tags, links as
  text; the one `{@html}` of `components/ai/Markdown.svelte` receives only that output.
- Developers: `DASHCRAFT_AI_MOCK=1 npm run tauri dev` (debug builds only) answers every request
  with canned text after 1.5 s; previews and consents work as with a real provider. The code is
  `#[cfg(debug_assertions)]`, so release builds do not contain it.

## Security

- CSP (`tauri.conf.json` → `app.security.csp`): `default-src 'self'`, `script-src 'self'`, IPC only
  (`ipc:` / `http://ipc.localhost`), `object-src`/`frame-src`/`form-action` set to `'none'`, and no
  remote origins. `style-src` allows `'unsafe-inline'` because Svelte style directives and the
  backend-rendered SVG use inline styles. For the same reason, Tauri's nonce injection is turned
  off for `style-src` only (`dangerousDisableAssetCspModification: ["style-src"]`). `devCsp`
  additionally allows the Vite HMR websocket.
- Capabilities: `core:default`, `core:window:allow-set-title` (the `<file> - DashCraft` window title),
  `dialog:allow-open` and `dialog:allow-save` (File → Export). There is no fs, shell or http plugin.
  File access happens only in Rust commands (documents, `blob-map.json`, CSVs: read only; writes:
  File → Recent's `recent-files.json` in the app config folder, and File → Export's PBIP / PBIT to
  the path chosen in the dialog).
- Icons are inline SVG from the vendored Fluent files (`{@html}` of static assets, never of
  document data); fonts are bundled (`font-src 'self'`).
- Report pages are SVG produced by pbi-render (text XML-escaped, images only as `data:image/…`),
  still parsed with `DOMParser` and sanitized before insertion (never `{@html}`): no scripts,
  `foreignObject`, styles, links, animations, event handler attributes or external references.
- Web URL buttons are never opened: the URL is shown with a Copy link button.
- No telemetry and no network calls: `AzureStorage.Blobs` resolves only through the local blob
  map; without one, blob sources fail with an "offline" hint. The only exception is the opt-in
  AI layer: once a person turned it on and allowed a request in the consent dialog, `pbi-ai`
  (in Rust, not the webview: the CSP still allows IPC only) sends it to the configured provider.
- AI answers are untrusted text: rendered by `lib/markdown.ts` (escaped, no attributes from the
  text, links and images as text), never executed or applied to the model.
- The grid filter is a DataFusion SQL *expression* on an in-memory, read-only table (no DDL / DML
  / statements); page sizes, cell texts (32k characters) and preview samples are capped.

## Linux packages

The `.deb` package is named **`dash-craft`** (the binary is `dashcraft-app`): Tauri 2.12 derives
it from `productName` by kebab-casing it and has no package-name option. Install it with
`sudo apt install ./DashCraft_<version>_amd64.deb` and remove it with `sudo apt remove dash-craft`.
The name is kept on purpose (a Linux-only `productName` override would also lowercase the app
name and menu entry); see [Linux package name](../docs/packaging.md#linux-package-name-dash-craft-decided).

## Linux system dependencies

Tauri 2 uses WebKitGTK 4.1. On Debian/Ubuntu (22.04+):

```sh
sudo apt-get update
sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

Fedora: `sudo dnf install webkit2gtk4.1-devel openssl-devel curl wget file libappindicator-gtk3-devel librsvg2-devel libxdo-devel`, plus `sudo dnf group install "c-development"`.
Arch: `sudo pacman -S webkit2gtk-4.1 base-devel curl wget file openssl appmenu-gtk-module libappindicator-gtk3 librsvg xdotool`.
See https://v2.tauri.app/start/prerequisites/ for other distributions. macOS needs only the
Xcode Command Line Tools. Windows needs the MSVC build tools and WebView2, which is preinstalled
on Windows 10 and 11.

## Known gaps

- One document at a time in the UI (the workspace could hold several); extra models of a
  multi-model PBIP (`extra_models`) are not shown yet.
- Canvas commands wait for a running `data.refresh` of the same document (the registry orders
  mutating commands per document); before, the canvas kept rendering the previous data.
- Derived columns are session-only: they are never saved to the model.
- Column statistics cover the whole table, not the grid's filtered rows.
- Measures use report / page categorical filters plus single-field slicer selections and column
  cross-filters; advanced, Top N and relative-date filters, range slicers and multi-field
  selections are listed as not applied (pbi-session `filters_from_report`), and per-visual
  `visualInteractions` are ignored (the panel is page-wide).
- The blob map setting lasts for the app session (not persisted to disk); the CSV fallback reads
  local files picked in the dialog only.
- Only the grid's rows are virtualized; very wide tables (hundreds of columns) render every column
  of the visible rows.
- The Model view reads `diagramLayout.json` positions but not table colours or the "pin key
  fields" options; dragging cards does not save a layout.
- The DAX query view has no IntelliSense, formatter, find / replace or command palette; results
  are capped at 1000 rows per `EVALUATE` and use the model's format strings for model columns.
- Report canvas: highlight interactions (`HighlightFilter`) filter instead of highlighting, and a
  cross-filter source dims its other points but other visuals are not highlighted; bookmarks
  (and bookmark navigators), drillthrough, Q&A and Web URL buttons are not executed; synced
  slicers (`syncGroup`) and the newer slicers' `selfFilter` are not read, so each page keeps its
  own selection; range and relative-date slicers draw but their sliders / date pickers are not
  interactive (tiles select like list items); no visual header menus (focus mode, show as table,
  sort, export), drill-down / drill-up, right-click menus or filter card editing; cross-filters
  from matrices use the row path only; tooltips are the default ones (report-page tooltips are
  not rendered); interaction state lasts until the document closes. Bookmarks, Performance
  analyzer, Sync slicers, Mobile layout and the Analytics pane are shown disabled.
- The native window keeps the OS title bar; the in-app title bar row sits under it.
- Single-instance forwarding is not wired: opening a file from Windows Explorer or a Linux file
  manager while the app runs starts a second window. (macOS Finder opens, `RunEvent::Opened`,
  reach the running app as `open-file-requested` since v0.2.0.)
- Opening is not cancellable from the UI (the pbi-core readers are synchronous); refreshes are.
