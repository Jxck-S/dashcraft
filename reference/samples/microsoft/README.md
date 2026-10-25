# Microsoft Power BI Desktop samples (public test fixtures)

Sample reports published by Microsoft in
[microsoft/powerbi-desktop-samples](https://github.com/microsoft/powerbi-desktop-samples) under the
MIT licence. DashCraft uses them as **public test fixtures only**: they are not read at runtime, not
embedded in any binary and not shipped in the app bundles or installers
(`app/src-tauri/tauri.conf.json` bundles only `NOTICE`, `THIRD_PARTY_NOTICES.md` and font/icon
licences). "Microsoft" and "Power BI" are trademarks of Microsoft, named here only to say where the
files come from; DashCraft is not affiliated with or endorsed by Microsoft (see [`NOTICE`](../../../NOTICE)).

- **Source:** https://github.com/microsoft/powerbi-desktop-samples, pinned at commit
  [`d5ec562d24b1890c2fd2976da625dd0920d35871`](https://github.com/microsoft/powerbi-desktop-samples/tree/d5ec562d24b1890c2fd2976da625dd0920d35871)
  (`main` on 2026-10-22, "Merge pull request #160 … Update Power BI Visuals report").
- **Licence:** MIT, "Copyright (c) Microsoft Corporation. All rights reserved." The full text is
  [`LICENSE`](LICENSE), the upstream file byte for byte. Attribution:
  [`THIRD_PARTY_NOTICES.md`](../../../THIRD_PARTY_NOTICES.md).
- **Facts for tests:** [`expected.json`](expected.json) (see [below](#expectedjson)).

## Files

The four small files and `LICENSE` are committed here. The two large ones (8–9.3 MB) are not: run
`scripts/fetch-samples.sh` to download them into `reference/samples/downloads/` (gitignored).

| File | In the repository | Upstream path (at the pinned commit) | Bytes | SHA-256 |
|---|---|---|---:|---|
| `LICENSE` | `samples/microsoft/` | `LICENSE` | 1,183 | `dea9265341829002e2c23a7372393eb2ed6e26085fb623f38a4ba0af833f30a6` |
| `Revenue Opportunities.pbix` | `samples/microsoft/` | `2026 Power BI Samples Revamp/Revenue Opportunities.pbix` | 245,930 | `3c95c4fb5a261e9d13a7152a2e14b18ea9e9efac0105dbd9ed46ba0137489c27` |
| `Corporate Spend.pbix` | `samples/microsoft/` | `2026 Power BI Samples Revamp/Corporate Spend.pbix` | 716,704 | `2e56785790d20bc4596d2aec8ede7128093b4affaaf2204198b382c9192f6ca6` |
| `Regional Sales Sample.pbix` | `samples/microsoft/` | `2026 Power BI Samples Revamp/Regional Sales Sample.pbix` | 1,018,743 | `050e6b8549c222489ef51ec6c7647609b9401d42da0daf793f91603dd2d8f3da` |
| `COVID-19 US Tracking Sample.pbit` | `samples/microsoft/` | `powerbi-service-samples/COVID-19 US Tracking Sample.pbit` | 1,276,277 | `714d3b00e5b02d4c0d87c73363b00829d14c77f609c49ea2c23b4f3392ea626d` |
| `AdventureWorks Sales.pbix` | `samples/downloads/` (fetched) | `2026 Power BI Samples Revamp/AdventureWorks Sales.pbix` | 8,390,782 | `76c89b64cd1c5a9514b43cc5ea356efc82f8e92f9a96b49f6679d9ccd57fc4f5` |
| `Store Sales.pbix` | `samples/downloads/` (fetched) | `2026 Power BI Samples Revamp/Store Sales.pbix` | 9,701,642 | `160b5b48b74ee1b5ea7259b6c8f0f1e2ec011e94c7b743742a14e6cad1153e6f` |

Verified on 2026-10-23: every file's git blob id (`git hash-object`) equals the upstream tree entry at
the pinned commit (`gh api repos/microsoft/powerbi-desktop-samples/git/trees/<commit>?recursive=1`),
and `scripts/fetch-samples.sh --all` re-downloaded all seven from `raw.githubusercontent.com` at that
commit with matching SHA-256. The upstream repository has older copies of some of these reports
under other folders (`Sample Reports/`, `new-power-bi-service-samples/`, `powerbi-service-samples/`);
they differ, so always use the paths above.

```sh
scripts/fetch-samples.sh         # download the two large samples (idempotent; verifies SHA-256)
scripts/fetch-samples.sh --all   # also re-fetch and verify the committed files (restores missing ones)
```

## Using them in tests

- Paths are relative to the fixture root: `$DASHCRAFT_REFERENCE`, else `<workspace>/reference`
  (e.g. `samples/microsoft/Revenue Opportunities.pbix`, `samples/downloads/Store Sales.pbix`).
- The committed files are always present. A test that needs a file under `samples/downloads/` must
  print `SKIP: <file> missing (run scripts/fetch-samples.sh)` and pass when it is absent.
- These samples contain no private data and may be used in snapshots, goldens and CI logs.

## Summary

Observed with `dashcraft` 0.2.0 (workspace commit `1f4ccf7`) on 2026-10-23: `detect`, `inspect`
(text and `--json`), `refresh`, `measures`, `lint` and `render --all-pages --out-dir … --now
2026-10-23T00:00:00Z`, debug and release builds. No command crashed, panicked or hung. The 21 PNG
pages are byte-identical between the debug and the release build. Times are wall-clock times of the
release CLI process on an Apple M4 Pro under heavy load from other jobs (load average 35–44 on 14
cores), so they are upper bounds; the debug build is 4–10× slower (`render` on Store Sales: 48 s).

| File | Format | Model: tables / columns / measures / relationships | Embedded data | Measures evaluated | Report: pages / visuals | Drawn / placeholder / with warnings | Lint E / W / I | refresh · measures · render (release) |
|---|---|---|---|---|---|---|---|---|
| Revenue Opportunities | PBIX, PBIR report | 9 / 69 / 6 / 6 | 9 tables, 2,986 rows | 6 / 6 | 3 / 12 | 12 / 1 / 2 | 0 / 14 / 3 | 0.02 s · 0.02 s · 0.31 s |
| Corporate Spend | PBIX, PBIR report | 10 / 36 / 19 / 7 | 10 tables, 167,099 rows | 19 / 19 | 3 / 20 | 20 / 2 / 7 | 0 / 21 / 17 | 0.05 s · 0.06 s · 1.24 s |
| Regional Sales Sample | PBIX, legacy `Report/Layout` | 11 / 70 / 15 / 8 | 11 tables, 21,911 rows | 15 / 15 | 9 / 147 | 144 (3 hidden) / 17 / 6 | 3 / 79 / 35 | 0.04 s · 0.64 s · 4.03 s |
| COVID-19 US Tracking Sample | PBIT (`DataModelSchema`, legacy `Report/Layout`) | 6 / 30 / 10 / 2 | none (template): 1 of 6 tables loads offline | 10 / 10 (6 BLANK, no data) | 2 / 67 | 44 (23 hidden) / 5 / 1 | 0 / 1 / 147 | 0.02 s · 0.02 s · 0.16 s |
| AdventureWorks Sales (fetched) | PBIX, PBIR report | 10 / 62 / 20 / 9 | 10 tables, 263,579 rows | 20 / 20 | 1 / 17 | 17 / 1 / 3 | 0 / 4 / 2 | 0.12 s · 0.14 s · 0.36 s |
| Store Sales (fetched) | PBIX, PBIR report | 7 / 69 / 32 / 5 | 7 tables, 1,289,872 rows | 32 / 32 | 3 / 14 | 14 / 2 / 4 | 0 / 13 / 4 | 0.19 s · 0.41 s · 5.32 s |

"Drawn" counts visuals with status `ok` or `warning` in `render --json` (groups included);
placeholders are the types DashCraft draws as a labelled box ([docs/VISUALS.md](../../../docs/VISUALS.md#placeholders)).
Every PBIX row count and measure name above matches an independent decoder, and the report
structure matches a raw parse of the package (see [Cross-checks](#cross-checks)).

## What each file exercises

### Revenue Opportunities.pbix

- **Format:** PBIX (package 1.32) with a PBIR report (`Report/definition/`) and a VertiPaq `DataModel`
  (XPress9 ABF, compatibility level 1606, en-US). Base theme `Fluent2-CY26SU08` is referenced but not
  in the package (`PBIR007` warning); custom theme `Classroom…json` (RegisteredResources).
- **Model:** 9 tables: 7 "Enter data" tables (M `Binary.Decompress` + `Json.Document`, loaded from the
  embedded data), `Calculations` = `Table.Combine` of `Fact A` + `Fact B` (458 = 228 + 230 rows), and a
  DAX calculated date table (`ADDCOLUMNS(CALENDAR(…))`, 1,096 rows); 1 calculated column; 6 star
  relationships; column descriptions.
- **Measures:** 6/6 (`SUM`, `AVERAGE`, `COUNTA`): Revenue $1,968,250,939, Opportunity Count 458.
- **Report:** 3 pages, one a hidden 600×450 tooltip page; `cardVisual`, bar charts, a line + column
  combo chart, `treemap`, `donutChart`, a matrix with data bars, a dropdown slicer, drill-through
  buttons; `shapeMap` is a placeholder.
- **Render warnings:** conditional formatting `dataPoint.fill` on the combo chart, a gradient whose
  input is not a model field on the matrix, a sort on a field that is not in the visual.
- **Lint:** 16 × LINT003 (stale `queryRef`s such as `Fact.Opportunity Count`), 1 × LINT016 overlap.

### Corporate Spend.pbix

- **Format:** PBIX (1.33), PBIR report, ABF model (1606).
- **Model:** 10 tables incl. two calculated "toggle" tables used by button slicers, 2 calculated
  columns (`Format([Date], "MMM")` sorted by `Period`; an `IF`/`TRIM` clean-up), 7 relationships, two
  of them with key types that differ (string → double, double → int64; LINT010 and ENG005 warnings).
- **Measures:** 19/19: `TOTALYTD`, `CALCULATE` with filters, variances and percentages, `SWITCH(TRUE(),
  ISINSCOPE(…))` + `SELECTEDVALUE` labels, a dynamic title text. Two are BLANK at the grand total by
  design (`IF` and `ISINSCOPE` branches).
- **Report:** 3 pages (one hidden tooltip page); `advancedSlicerVisual` (button slicers), new cards
  with reference labels, stacked area, clustered bar/column, line charts, a matrix with data bars and
  conditional colours; `azureMap` and `decompositionTreeVisual` are placeholders.
- **Render warnings:** conditional formatting not supported on `cardVisual` reference labels and
  accent bar, `lineChart` line styles, `stackedAreaChart` and `azureMap` data points.
- **Data:** 167,099 rows (`Calculations` 166,216).

### Regional Sales Sample.pbix

- **Format:** PBIX (1.33) with the **legacy `Report/Layout`** (UTF-16LE JSON), ABF model (1606).
  Custom theme `Fidex…json` (RegisteredResources, item type 201) with 480 data colours; a registered
  image; a custom visual package (`FlowVisual_…`, listed, never executed).
- **Model:** 11 tables incl. a measure-only calculated table (`{1}`), a what-if parameter table
  (`GENERATESERIES(-80, 20, 10)`) and a `VAR`-based DAX calendar; 5 calculated columns; 8
  relationships, one int64 → string.
- **Measures:** 15/15: `COUNTAX(FILTER(KEEPFILTERS(…)))`, `SUMX`, `VAR`, what-if parameter value,
  text measures (e.g. "Use the slicer to adjust the forecast, current value is 0").
- **Report:** 9 pages (8 hidden, reached through 21 page navigators and a bookmark navigator); groups,
  shapes, images, funnel, scatter, combo, ribbon chart, matrices and tables with conditional icons and
  data bars, key influencers, decomposition tree, smart narrative. Placeholders: 9 `image`, 3
  `keyDriversVisual`, `shapeMap`, `ribbonChart`, `decompositionTreeVisual`, `aiNarratives`, the custom
  visual.
- **Render warnings:** conditional formatting on line charts and the shape map; the `StartsWith` rule
  of a conditional icon.
- **Lint:** 3 errors that are genuine broken references in the sample (checked against the model):
  two matrices format with a table `Page Details` that does not exist (LINT001) and a page filter
  uses `Opportunities[Opportunity Owner Name]`, which does not exist (LINT004). Plus 108 × LINT003,
  1 × LINT010, 5 × LINT016.
- **Slowest page:** "Industries Overview" (1.9 s, the scatter chart takes 1.07 s).

### COVID-19 US Tracking Sample.pbit

- **Format:** PBIT template (package 1.25): TMSL `DataModelSchema` (UTF-16LE, compatibility level 1550)
  without data, legacy `Report/Layout`, a `DataMashup` (listed, not decoded), base theme `CY19SU12`.
- **Model:** 6 tables incl. two auto date tables (`DateTableTemplate_…`, `LocalDateTable_…`), 16
  calculated columns, 10 measures, 2 relationships, 5 shared M expressions and 1 parameter.
- **Offline refresh:** only `DateTableTemplate_…` loads (1 row, DAX `Calendar`). `COVID` and `StateDim`
  stop at the unsupported M function `Table.ExpandTableColumn` (after `Table.NestedJoin`; `COVID` also
  needs `Web.Contents` through `fxGetReportDetails`, which stays offline by design); `Table` and
  `COVID measures` are "Enter data" tables that stop at `Binary.Decompress`; `LocalDateTable_…` and
  the dependent calculated columns are skipped. `refresh`, `measures` and `render` exit 2, as
  documented for failed tables.
- **Measures:** all 10 evaluate (none unsupported); the 4 text measures return their text, the 6
  numeric ones BLANK because their tables have no data.
- **Report:** 2 pages, 1280×720 (older canvas size); 37 action buttons, 9 groups and bookmark-toggled
  hidden visuals (23 hidden), `filledMap`/`shapeMap`/`image` placeholders, empty column charts and
  matrices. Lint: 143 × LINT016 (stacked overlapping buttons, intentional), 5 × LINT003.

### AdventureWorks Sales.pbix (fetched)

- **Format:** PBIX (1.32), PBIR report, ABF model (1606); dark built-in theme `SoftDark`
  (SharedResources) over `Fluent2-CY26SU08` (not in the package, `PBIR007`).
- **Model:** 10 tables incl. **two calculation groups** (`Time Intelligence`: YTD/QTD/MTD/PY/YoY…;
  `Date Role`: order/due/ship date via `USERELATIONSHIP`), 2 inactive relationships, a one-to-one
  bidirectional relationship, 4 calculated columns, a hierarchy, 20 measures.
- **Data:** 263,579 rows (`Sales` and `Sales Order` 121,253 each, `Customer` 18,485).
- **Measures:** 20/20. `Sales YTD/QTD/MTD` are BLANK and `Sales PY` equals `Sales` at the grand total;
  that is correct, because the date table runs to 2021-06-30 and has no sales in its last year.
  `dax.query` by year shows the calculation items work (PY of 2019 = Sales of 2018; due and ship date
  roles give different totals per year).
- **Report:** 1 page, 17 visuals: funnel, donut, line, clustered bar/column, matrix with icons and
  data bars, button slicer, relative-date slicer, new card, shapes; `azureMap` placeholder.

### Store Sales.pbix (fetched)

- **Format:** PBIX (1.32), PBIR report, ABF model (1606).
- **Model:** 7 tables incl. a 20-column DAX date table with a fiscal hierarchy, 7 calculated columns,
  32 measures, 5 relationships (one inactive); `Sales[ReportingPeriodID]` (string) joins two int64 keys.
- **Data:** 1,289,872 rows (`Sales` 923,371, `Item` 364,184): the largest sample, a good refresh and
  query-performance fixture (refresh 126 ms in release).
- **Report:** 3 pages: `gauge`, `waterfallChart`, `scatterChart`, donut, line, bar, new cards, a
  3-level matrix; `azureMap` placeholders.
- **Performance:** "District Monthly Sales" (2.4 s) and "New Stores" (2.3 s) are the slowest pages of
  all samples. The bar chart groups by `District[District]` × `Store[Name]` with a `Min(District)`
  tooltip, which keeps all 900 combinations, as SUMMARIZECOLUMNS does (the DAX engine agrees: 900
  rows); the 3-level matrix takes 2.1 s.

## Bugs found (2026-10-23)

Reported, not fixed here (the crates belong to other owners).

1. **String ↔ number relationship keys never match** (`pbi-engine`, `crates/pbi-engine/src/relationships.rs`).
   ENG005 says "key types differ (Utf8 vs Int64); keys match by value", but rows on the many side
   find no row on the one side when one key is text and the other numeric, so every value lands on
   the BLANK row. Numeric ↔ numeric (Float64 ↔ Int64, Corporate Spend `Department`) works. Power BI
   built the relationship index for these (the `R$…` system table of the Regional Sales relationship
   exists). Effects: Regional Sales "Forecast by Territory" shows only "(Blank)"; Store Sales'
   fiscal-month line charts and waterfall show only "(Blank)"; Corporate Spend `Cost Element` filters
   nothing.
   ```sh
   dashcraft run dax.query --doc "reference/samples/microsoft/Regional Sales Sample.pbix" \
     -p 'query=EVALUATE SUMMARIZECOLUMNS(Territories[Territory], "Rev", [Revenue Won])'
   # one row: [null, 26435925]; expected one row per territory
   dashcraft run dax.query --doc "reference/samples/downloads/Store Sales.pbix" \
     -p 'query=EVALUATE SUMMARIZECOLUMNS('"'"'Date'"'"'[Year], "S", [Sales])'
   # one row: [null, 45184553.69]
   ```
2. **Custom themes from `RegisteredResources` are not applied**, so `ThemeDataColor` resolves against
   the default palette. Regional Sales: pages and matrices that should be `#EEEEEE`/`#FFFFFF`
   (Fidex `dataColors[7]`/`[6]`, `ColorId` 9/8) render red `#D64550` and yellow `#D9B300`. Revenue
   Opportunities: the combo chart uses `#118DFF`/`#E66C37` instead of the Classroom colours. Cause for
   PBIR: `read_themes` in `crates/pbi-pbir/src/reader.rs` loads only items whose `type` ends with
   `Theme`, and this file declares its custom theme item as `"type": "Image"`; the theme named in
   `themeCollection.customTheme` should be loaded regardless. For the legacy Layout (item type 201,
   CustomTheme) the cause is not yet found (`report_theme_values` / `read_theme_resource` in
   `crates/pbi-ops/src/canvas.rs`).
   ```sh
   dashcraft render "reference/samples/microsoft/Regional Sales Sample.pbix" --page 1 \
     --svg target/tmp/regional-p1.svg --force   # page background fill="#D64550"
   ```
3. **Matrix column headers ignore the column's sort-by column.** Corporate Spend "Trends details"
   (`Date[Month]`, sorted by `Period`) shows Apr, Aug, Dec, Feb, … alphabetically; the line chart on
   the same page orders Jan … Dec correctly. The visual has no `sortDefinition`.
   `dashcraft render "reference/samples/microsoft/Corporate Spend.pbix" --page 2 --png target/tmp/cs2.png --force`
4. **Banded table/matrix rows ignore the theme.** The alternate-row background is the fixed
   `BAND_COLOR` (`#EEEDED`, `crates/pbi-render/src/visuals/table/grid.rs`) while the text colour comes
   from the theme, so with `SoftDark` (AdventureWorks "Product details") the Clothing and Bikes rows
   are white on light grey and unreadable.
   `dashcraft render "reference/samples/downloads/AdventureWorks Sales.pbix" --page 1 --png target/tmp/aw.png --force`

Known gaps seen (not bugs): the placeholder types above; conditional formatting on objects listed in
the render warnings; the relative-date slicer (AdventureWorks, `mode: 'RelativeDatePicker'`) is drawn
as a clipped list of dates; new-card callouts are clipped rather than shrunk when the value is wider
than the card (Revenue, Store Sales, Regional Sales); M `Table.ExpandTableColumn` and
`Binary.Decompress` ("Enter data" tables); `refresh --include-system-tables` reports every `H$`, `R$`
and `U$` table as an error ("unsupported dictionary encoding: type 0").

## expected.json

Stable facts per file, for tests to assert (`schemaVersion` 1). Each entry of `files[]` has:

- `file` (relative to the fixture root), `committed`, `bytes`, `sha256`, `upstream.path`,
  `upstream.gitBlobSha1`;
- `detect`: `format` (`pbix`/`pbit`), `label`, `confidence`, `reportFlavour` (`pbir`/`legacyLayout`),
  as printed by `dashcraft detect --json`;
- `model`: `source` (`abf`/`tmsl`), `culture`, `compatibilityLevel`, `embeddedData`, `counts` (the
  `inspect --json` counts), `tables[]` (`name`, `kind`, `hidden`, `columns`, `measures`, `data`:
  `embedded`/`dax`/`m`, `rows`: the loaded row count, `null` when the table cannot load offline),
  `measures.count`, `measures.names` (`Table[Measure]`) and, for some files, `measures.values`
  (`value` within `valueTolerance`, and the exact `formatted` text);
- `report`: `name`, `activePage`, `pages[]` (`name`, `displayName`, `width`, `height`, `hidden`,
  `visuals` incl. groups, `visualsByType`);
- `knownBrokenReferences` (Regional Sales): the lint errors that are real defects of the sample;
- `crossChecked`: which of these facts were confirmed independently.

Render warnings, lint warning counts and timings are deliberately left out: they change as
DashCraft improves.

### Cross-checks

- **pbixray 0.15.5** (MIT, [Hugoberry/pbixray](https://github.com/Hugoberry/pbixray)), installed in a
  throwaway venv under `target/tmp/venv` and used only as an external oracle (no code copied): for
  all five PBIX files the table names, per-table row counts, per-table column counts and measure
  names equal DashCraft's. Measure values recomputed with pandas from pbixray's decoded tables (simple
  `SUM`/`COUNT`/`DISTINCTCOUNT`/filtered sums) equal DashCraft's within floating-point rounding; they
  are the `measures.values` entries. pbixray lists fewer relationships (5/7/4 instead of 6/8/5 for
  Revenue Opportunities, Regional Sales and Store Sales) because it omits relationships to
  calculated tables; `counts.relationships` is DashCraft's count, which matches the model.
- **Raw package parse** (Python `zipfile` + `json`, no DashCraft code): report flavour, page names,
  sizes, visibility and visual counts by type for all six files; for the PBIT also the
  `DataModelSchema` table names, per-table column counts, measure and relationship counts and the
  compatibility level.
- **By hand:** the `knownBrokenReferences` (the table and column do not exist in the model).

To refresh the facts after a DashCraft change, re-run the commands of [Summary](#summary) with
`--json` on each file and compare with `expected.json`; any difference in a cross-checked field is a
regression in DashCraft, not in the sample.
