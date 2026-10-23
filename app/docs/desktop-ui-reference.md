# DashCraft vs. Power BI Desktop: UI reference

The app shell (`app/src`) mirrors Power BI Desktop's layout, ribbon and panes so the viewer can
grow into an editor without moving things around. This file records **where each element lives
in Desktop** (from the Microsoft Learn pages below, current as of 2026) and **what DashCraft does
with it**. Status: ✅ implemented (read-only where Desktop edits), ⛔ shown disabled
("Not available in DashCraft (yet)"), ✚ DashCraft-only extra.

Assets: icons are [Fluent UI System Icons](https://github.com/microsoft/fluentui-system-icons)
(MIT, `src/assets/icons/LICENSE`), the font stack is `"Segoe UI", Selawik, system-ui,
sans-serif` with Selawik bundled (SIL OFL 1.1, `src/assets/fonts/`). No Power BI icons, images,
logos or CSS are used; the app is named "DashCraft" everywhere.

## Sources (Microsoft Learn)

1. [Use the ribbon in Power BI Desktop](https://learn.microsoft.com/power-bi/create-reports/desktop-ribbon) — search bar, collapsible ribbon, keytips, contextual Measure / Column tools tabs, Ctrl+F6.
2. [Use the Design ribbon](https://learn.microsoft.com/power-bi/create-reports/power-bi-design-ribbon) and [Report themes](https://learn.microsoft.com/power-bi/create-reports/desktop-report-themes) — Themes gallery, Customize / Download / Import theme, Customize page (Page size, Page background, Wallpaper), Help.
3. [Optimize ribbon](https://learn.microsoft.com/power-bi/create-reports/desktop-optimize-ribbon) and [Performance analyzer](https://learn.microsoft.com/power-bi/create-reports/desktop-performance-analyzer) — Queries (Pause / Refresh visuals), Report (Optimization presets), Review (Performance analyzer), Apply (Apply all slicers button).
4. [Page view and page size](https://learn.microsoft.com/power-bi/create-reports/power-bi-report-display-settings) — View tab: Fit to page (default), Fit to width, Actual size; gridlines, snap to grid.
5. [Filters pane](https://learn.microsoft.com/power-bi/create-reports/power-bi-report-filter) — filter cards per visual / page / all pages, search, hide and lock, View → Filters toggles the pane.
6. [Bookmarks](https://learn.microsoft.com/power-bi/create-reports/desktop-bookmarks) — View tab: Bookmarks and Selection panes.
7. [Report view](https://learn.microsoft.com/power-bi/create-reports/desktop-report-view) — canvas, page tabs with New page "+", Visualizations pane (Build visual), Data pane.
8. [Table view](https://learn.microsoft.com/power-bi/transform-model/desktop-data-view) — grid, formula bar, Data pane, Table tools / Column tools, status bar "Table: … Column: … distinct values".
9. [Model view](https://learn.microsoft.com/power-bi/transform-model/desktop-relationship-view) — diagram of table cards and relationships, layout tabs, Properties and Data panes.
10. [DAX query view](https://learn.microsoft.com/power-bi/transform-model/dax-query-view) — Home: Clipboard, Editing (Format, Comment, Uncomment, Find, Replace, Command palette); command bar Run ↔ Cancel; Results grid with a Result dropdown and Copy (tab-delimited); query tabs with status; quick queries from the Data pane.
11. [TMDL view](https://learn.microsoft.com/power-bi/transform-model/desktop-tmdl-view) — fifth view in the left rail; script editor with Apply.

## Layout

| Element | Desktop location | DashCraft |
|---|---|---|
| Save / Undo / Redo | Quick Access Toolbar, title bar left | ⛔ (read-only viewer) |
| File name | title bar centre, `<file> - Power BI Desktop` | ✅ `<file> - DashCraft` (also the native window title) |
| Search (Alt+Q) | title bar centre [1] | ✅ ribbon commands of the current view, the views, Open, and model fields |
| Account / Share | title bar right | omitted |
| Ribbon tabs, Report view | File, Home, Insert, Modeling, Design, View, Optimize, Help, External tools [1][2][3] | ✅ all but External tools |
| Ribbon tabs, Table / Model view | File, Home, Help (+ Table tools, Column tools) [8][9] | ✅ Table tools always in Table view, Column tools with a grid column selected |
| Ribbon tabs, DAX query / TMDL view | File, Home (Clipboard, Editing), Help [10][11] | ✅ |
| Contextual tabs | Measure / Column / Table tools on selection [1] | ✅ read-only fields (Name, Home table, Data type, Format, Summarization, Data category) |
| Ribbon collapse | chevron at the ribbon's bottom right, Ctrl+F1, double-click a tab [1] | ✅ collapsed, a tab click shows the ribbon floating |
| File (backstage) | Home, New, Open, Save, Save as, Get data, Import, Export, Publish, Options and settings, Get help | ✅ Home, Open (browse, folder, **Recent** persisted in the app config folder), Export (PBIP, PBIT), Options and settings → **Options** (a Desktop-style Options dialog: pages on the left under Global / Current file, only ✚ **AI** implemented, the rest ⛔) and Data source settings (**Blob map…**), Get help → About; ⛔ New, Save, Save as, Get data, Import, Export to PDF, Publish; ✚ Close (closes the document) |
| View rail | left: Report, Table, Model, DAX query, TMDL [11] | ✅ same order |
| Formula bar | under the ribbon when a measure, calculated column or calculated table is selected | ✅ read-only (✕ / ✓ disabled), expandable; ✚ ✦ **Explain with AI** for measures (opt-in AI) |
| Notification bar | yellow bar under the ribbon | ✅ remote model, missing model, cached data (+ **Refresh from source**) |
| Right panes, Report view | Filters, Visualizations, Data; Selection / Bookmarks open to their left [5][6][7] | ✅ same order; each collapses to a vertical strip with « and its title |
| Page tabs | under the canvas: layout switch, ‹ ›, page tabs, "+" [7] | ✅ desktop / mobile switch (mobile ⛔), previous / next page (Ctrl+Page Up / Down), tabs (hidden pages greyed with the hidden icon), "+" ⛔; ✚ report picker when a PBIP holds several reports |
| Status bar | "Page X of Y" left; zoom slider, percentage, Fit to page right | ✅ plus ✚ status message, data state (cached / loaded / refreshing / errors → refresh panel), diagnostics counters |

## Home tab (Report view, Desktop order)

| Group | Buttons | DashCraft |
|---|---|---|
| Clipboard | Paste, Cut, Copy, Format painter | ⛔ |
| Data | Get data, Excel workbook, OneLake catalog, SQL Server, Enter data, Dataverse, Recent sources | ⛔ |
| Queries | **Transform data** (split button), **Refresh** | ✅ Transform data opens a read-only *Power Query Editor* (M queries, parameters, DAX expressions — the old Queries tab); its arrow menu has **Data source settings → Blob map…** (plus "Stop using the blob map"), **Edit parameters** (read-only) and ✚ **Load CSV manually…** per table; ✅ Refresh (all tables, offline; progress and Cancel in the refresh panel) |
| Insert | New visual, Text box, More visuals | ⛔ |
| Calculations | New measure, Quick measure | ⛔ |
| ✚ AI (DashCraft) | — | ✚ **Suggest a measure**, **Explain measure**, **AI options**: DashCraft's opt-in AI features ([docs/AI.md](../../docs/AI.md)), right after Calculations; visible but disabled ("Turn on AI in Options") until AI is turned on with a provider. Also in the Report view's Modeling tab and Measure tools (**Explain with AI**), and in the Table and Model views' Home tab |
| Sensitivity | Sensitivity | ⛔ |
| Share | Publish | ⛔ |
| Copilot | Copilot | replaced by the neutral ✚ AI group above (Fluent "Sparkle" icon); DashCraft does not use the Microsoft Copilot name or logo |

Table view Home: Clipboard, Data, Queries, Calculations, Sensitivity, Share. Model view Home:
Clipboard, Data, Queries, Relationships (**Manage relationships** ✅ read-only list),
Calculations, Security (**View as** ✅, Manage roles ⛔), Sensitivity, Share.

## Other Report view tabs

| Tab | Groups (Desktop) | DashCraft |
|---|---|---|
| Insert | Pages, Visuals, AI visuals, Power Platform, Elements, Sparklines | ⛔ |
| Modeling | Relationships, Calculations, Parameters, Page refresh, Security, Q&A | ✅ Manage relationships (read-only); ✅ Security → **View as** (the *View as roles* dialog: role checkboxes and Other user, OK / Cancel; then a yellow "Now viewing as" bar with **Stop viewing**; disabled without roles); ✚ AI (Suggest a measure, Explain measure, AI options) after Calculations; ⛔ the rest (Manage roles included) |
| Design [2] | Themes (gallery, Theme gallery, Customize / Download / Import theme), Customize page, Help | ✅ the gallery shows the report's current theme (name, data colors, background) read-only; ⛔ the rest. Older Desktop builds had the Themes gallery on the View tab; current builds moved it to Design |
| View [4][5][6] | Scale to fit (Page view ▾: Fit to page, Fit to width, Actual size), Mobile, Page options (Gridlines, Snap to grid, Lock objects), Show panes (Filters, Bookmarks, Selection, Performance analyzer, Sync slicers) | ✅ Page view (drives the canvas zoom), Filters and Selection toggles; ⛔ Mobile layout, Page options, Bookmarks, Performance analyzer, Sync slicers; ✚ group "DashCraft": **Properties** pane, **Reset to default** (the report's saved slicer selections, no cross-filter; the Power BI service has it in its action bar) and **Outline** (the page as a table of visuals) |
| Optimize [3] | Queries, Report, Review, Apply | ⛔ |
| Help | Help (Guided learning, Documentation, Training videos, Support, About), Community | ✅ About (version, licenses); ⛔ links (no network) |

## Panes

| Pane | Desktop | DashCraft |
|---|---|---|
| Filters [5] | search; "Filters on this visual" (with a visual selected), "on this page", "on all pages"; cards with hide / lock icons; "Add data fields here" | ✅ read-only cards as in reading view (`is X or Y` / `is not …`; expand for type, field and DAX-like condition): cards hidden in view mode left out (✚ "Show n hidden filter cards"), locked cards with a lock, search; ✚ the canvas's slicer selections and cross-filter as "Slicer: …" / "Selection: …" cards with a clear button; drop zones disabled |
| Visualizations [7] | Build visual (gallery + field wells), Format visual (Visual / General cards), Analytics | ✅ gallery with the selected visual's type highlighted, field wells (role → fields; click selects the field); Format: `visual.objects` (Visual) and `visualContainerObjects` + position / size / layer (General) as read-only cards (`visual_format`), Format page without a selection; ⛔ Analytics, changing the type |
| Data [7][8] | search, tables → measures / columns / hierarchies, field icons (Σ, calculator, calculated column, date, geography, hierarchy), hidden fields greyed, checkboxes for the selected visual, context menu | ✅ all; "View hidden" in the ⋯ menu (the Model view starts with hidden objects shown); ✚ table data chips; context menu: Properties, Refresh data, Load CSV manually…, View data, ✚ **Explain with AI** (measures; opt-in AI), **Quick queries** (Show top 100 rows, Evaluate, Define and evaluate, Show data preview) |
| ✚ Explain with AI | — | ✚ opened from a measure (Data pane menu, formula bar, Properties, ribbon): the measure's DAX, progress and Cancel, then the AI's summary, explanation (Markdown, rendered safely), filter context, dependencies, caveats and the locally computed references; left of the other right panes, collapsible, closable |
| Selection [6] | layer order / tab order, visibility | ✅ read-only, click selects |
| Properties [9] | Model view, the selected object | ✅ the existing Properties pane (properties, DAX / M / JSON, annotations, unmodelled properties, source file); ✚ also in the other views (View → Properties, or Properties in the Data pane menu) |
| Bookmarks, Performance analyzer, Sync slicers | View → Show panes | ⛔ |

## Views

| View | Desktop | DashCraft |
|---|---|---|
| Report [7] | canvas on a gray surround, page tabs, zoom | ✅ `ReportCanvas`: the page rendered by pbi-render (SVG), interactive as in reading view — slicer selection (Ctrl+click multi-select, Select all, dropdown popups, search, Clear selections), cross-filtering per `visualInteractions`, table / matrix / list scrolling, matrix expand / collapse, data point tooltips, ⚠ error tooltips with the DAX, Back / page navigation buttons (Web URL shown, never opened; bookmarks, drillthrough, Q&A not available), Esc; Page view / zoom slider / Ctrl+wheel, selection outline; ✚ View → **Reset to default**; ✚ Measures panel docked under the canvas (collapsed by default): every measure as on the canvas (filters + slicers + cross-filter) or under no / report / report + page filters; the values also show in the Data pane's measure tooltips |
| Table [8] | grid with column header menus, formula bar, Data pane, status bar | ✅ virtualized grid (sort, column menu, resize), column statistics, status bar `Table: … (n rows) Column: … (n distinct values)`; ✚ SQL filter bar, **Parse people field…** (column menu, Column tools, statistics panel), Rows / Schema (Table tools) |
| Model [9] | cards with fields, relationship lines with cardinality and cross-filter arrows, layout tabs, zoom / fit | ✅ positions from `diagramLayout.json`, else a layered auto-layout; cardinality marks, single / both arrows, inactive dashed; drag to pan, Ctrl+wheel and status bar zoom, Fit to screen, card collapse, layout tabs ("+" ⛔) |
| DAX query [10] | editor, Run ↔ Cancel, results (Result dropdown, Copy), query tabs with status, quick queries | ✅ editor (line numbers, Ctrl+/ comment, F5 / Ctrl+Enter, runs the selected lines if any), Run ↔ Cancel, results (≤ 1000 rows per EVALUATE, "Result n of m", Copy as TSV), query tabs with success / error / cancelled marks kept across view switches, quick queries; queries never change the model |
| TMDL [11] | script editor, Apply | ✅ read-only: the project's `.tmdl` files verbatim, or TMDL generated from TMSL / PBIX models; script list with filter; selecting a table opens its script; ⛔ Apply |

## Keyboard

Ctrl/⌘+O open, Ctrl/⌘+Shift+O open folder, Ctrl+F1 collapse / pin the ribbon, Alt+Q search,
Ctrl/⌘+Page Up / Page Down previous / next page, Ctrl/⌘+wheel zoom (canvas, diagram), F5 or
Ctrl/⌘+Enter run a DAX query, Ctrl/⌘+/ comment lines, Esc closes menus, File and dialogs.
