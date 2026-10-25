# Acme Outfitters (synthetic example)

A fictional outdoor-gear retailer, built to exercise DashCraft end to end: a PBIP project with a
TMDL semantic model, three PBIR (enhanced format) reports, the source CSVs, an offline blob map
and expected values. **Everything here is synthetic**: the company, people, stores, products,
e-mail addresses (`.example` domains) and the blob container URL are made up. The container
`https://acmeoutfittersdemo.blob.core.windows.net/sample-data` is a placeholder that is never
contacted; DashCraft resolves it offline through `blob-map.json`.

All files except this README are written by
[`scripts/examples/gen_acme.py`](../../../scripts/examples/gen_acme.py) (Python 3 standard library,
fixed seed, no wall clock: two runs give byte-identical files). Don't edit them by hand; change the
generator and run it again.

```sh
python3 scripts/examples/gen_acme.py              # regenerate everything below (keeps README.md)
python3 scripts/examples/check_acme.py            # run DashCraft on it and compare with expected.json
python3 scripts/examples/check_acme.py --bin target/debug/dashcraft   # skip `cargo run`
```

## Layout

| Path | What |
|---|---|
| `AcmeOutfitters.pbip` | Project file (its artifact is the Overview report; all three reports are siblings) |
| `AcmeOutfitters.SemanticModel/` | `definition.pbism`, `.platform`, TMDL `definition/`: `database.tmdl`, `model.tmdl`, `expressions.tmdl` (parameter `StorageContainerUrl`, functions `fnCleanKey`, `fnParseNames`), `relationships.tmdl`, `tables/*.tmdl` (11 tables), `roles/*.tmdl` (2 roles) |
| `Acme Outfitters - Overview.Report/` | Executive Summary, Product Detail, Time Intelligence |
| `Acme Outfitters - Stores.Report/` | Store Performance, Store Detail (hidden), Store Tooltip (tooltip page); bookmarks |
| `Acme Outfitters - Online.Report/` | Online Overview, Customer Detail |
| `*.Report/StaticResources/RegisteredResources/AcmeOutfitters.json` | The custom theme (original) |
| `data/*.csv` | Source data: orders (3,919 rows), customers, products, stores, targets, returns, web_sessions |
| `blob-map.json` | Blob paths of the placeholder container → `data/*.csv` |
| `expected.json`, `EXPECTED.md` | Expected values computed by the generator in plain Python, independently of DashCraft |
| `.gitignore` | Power BI Desktop's local files (`.pbi/localSettings.json`, `.pbi/cache.abf`) |

Use it with the CLI:

```sh
E=reference/examples/acme-outfitters
cargo run -q -p dashcraft-cli --bin dashcraft -- refresh $E/AcmeOutfitters.pbip --blob-map $E/blob-map.json
cargo run -q -p dashcraft-cli --bin dashcraft -- measures $E/AcmeOutfitters.pbip --blob-map $E/blob-map.json --report Stores
cargo run -q -p dashcraft-cli --bin dashcraft -- render $E/AcmeOutfitters.pbip --blob-map $E/blob-map.json \
  --report Overview --all-pages --out-dir target/tmp/acme
cargo run -q -p dashcraft-cli --bin dashcraft -- run security.viewAs --doc $E/AcmeOutfitters.pbip \
  --blob-map $E/blob-map.json -p roles='["Store Manager"]' -p userName=riley.chen@acmeoutfitters.example
```

## What it covers

**Data quality traps** (cleaned by M): exact and near duplicates (after trimming and casing),
order rows without an id, blank discounts, blank loyalty tiers, trailing spaces and mixed casing in
keys, statuses, regions, devices and e-mails, a blank target row, Python-style manager lists
(`['Ana Ruiz', "O'Neil Park"]`, `[]`, blank), unknown customer and product ids on orders, products
that never sold.

**M**: `AzureStorage.Blobs(StorageContainerUrl)` with `{[Name = "…"]}[Content]`, `Csv.Document`,
`Table.PromoteHeaders`, `SelectRows`, `TransformColumns`, `Distinct`, `TransformColumnTypes`,
`ReplaceValue`, `AddColumn`, `SelectColumns`, `RenameColumns`; typed functions
`(x as text) as text =>`; `each`, `??`, `let … in`, `meta`, `#table`, `#"quoted identifiers"`;
`Text.Trim/Upper/Lower/Proper/Start/Range/Length/Split/Combine/Contains/Replace/AfterDelimiter/
BeforeDelimiter`, `List.Select/Transform/IsEmpty/RemoveNulls/Count`, `Date.FromText`,
`DateTimeZone.UtcNow/RemoveZone` (only in `Last Refreshed`, which no expectation uses).

**Model**: a calculated `Date` table marked as the date table (hierarchy Year > Quarter > Month >
Date, `Month` sorted by `Month Number`); calculated columns with `RELATED`, `RELATEDTABLE`,
`ISEMPTY`, `IF`, `SWITCH`, `DATEDIFF`, `WEEKDAY` and context transition (`Customers[Lifetime Sales]`);
the calculated table `Products Never Sold` (`SELECTCOLUMNS`, `SUMMARIZE`, `ALL`, `EXCEPT`, `FILTER`,
`IN`, `ISEMPTY`); 33 measures in display folders (`DISTINCTCOUNTNOBLANK`, `CALCULATE` with
`KEEPFILTERS`, `ALL` and `USERELATIONSHIP`, `COUNTROWS`, `DIVIDE`, `MAX`, `SUMX`, `AVERAGEX`,
`AVERAGE`, `TOTALYTD`, `SAMEPERIODLASTYEAR`, `DATEADD`), currency, percent, signed-percent and date
format strings and a dynamic `formatStringDefinition` (`Sales (Scaled)`); the calculation group
`Time Calc` (Current, YTD, PY, YoY % with its own format string; precedence 10); relationships:
one-to-many, both directions (Orders–Customers), inactive (Ship Date), many-to-many on Region
(Targets–Stores), one-to-one (Returns–Orders), and no relationship at all (`Web Sessions`);
row-level security: `Region West` (static) and `Store Manager` (`USERPRINCIPALNAME()` on the
store manager's e-mail); hierarchies, `///` descriptions, annotations, lineage tags, query groups.

**Reports**: `cardVisual`, `card`, `multiRowCard`, `kpi`, slicers (dropdown, list, tile, between
for numbers and dates, relative date), `tableEx`, `pivotTable` with subtotals and hierarchy levels,
`pieChart`, `donutChart`, clustered, stacked and 100% bar and column charts, `lineChart`,
`areaChart`, `lineClusteredColumnComboChart`, `scatterChart`, `funnel`, `gauge`, `treemap`,
`waterfallChart`, `textbox` (runs, bullets, numbers, links), `shape`, `basicShape`,
`actionButton` (page navigation, back, bookmark, clear all slicers), `pageNavigator` and a `map`
(not supported offline: drawn as a placeholder). Conditional formatting: background gradients,
font-color rules, data bars, icons, and a chart `dataPoint.fill` rule. Report, page and visual
filters, including a report filter on the unrelated `Web Sessions` table, a Top N filter, a relative
date filter and calculation-item filters; `visualInteractions` (filter, highlight, none); saved slicer
selections (single, multi, inverted, ranges); a hidden page; a tooltip page used by a chart;
bookmarks; container formatting (titles, backgrounds, border radius, shadows); FitToPage,
FitToWidth and ActualSize pages. Every PBIR file validates against the schema in its `$schema`.

## Expected values

[`EXPECTED.md`](EXPECTED.md) lists them; [`expected.json`](expected.json) has the same with raw
values: row counts after M, every card value per report page under that page's filters and saved
slicers, table and matrix totals and rows, the calculation group for one month, visible rows and
totals per security role, and every measure unfiltered and under each report's filters. Semantics
the values rely on (as Power BI behaves):

- Orders with unknown keys belong to the blank row of the dimension: an `In` filter drops them, a
  `Not In` filter (the inverted loyalty-tier slicer) keeps them.
- Filters on `Web Sessions` never reach the sales tables (no relationship), and vice versa.
- A filter on Orders' dimensions reaches Customers through the both-directions relationship;
  security filters do not (security filtering is one-directional there).
- A store row shows its region's target (many-to-many), so the table total is not the sum of rows.
- Relative date filters ("last 10 years") keep every row: the data ends on 2025-12-31.

Expectations DashCraft does not meet yet carry `"knownEngineGap"` with the reason (listed at the end
of `EXPECTED.md`); `check_acme.py` reports them without failing.

## Licence

Original synthetic content generated for this repository, under the repository's terms. No
third-party data, themes or report definitions are included.
