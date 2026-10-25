# Visual support

How `pbi-render` draws each Power BI visual type, which formatting objects it honours, which Power BI service behaviours it reproduces, how it differs from the service, and what the app can do with it interactively. The summary matrix is in [FEATURES.md](FEATURES.md#supported-visuals); the implementation notes live in the module docs of `crates/pbi-render/src/visuals/`.

Contents: [pipeline](#pipeline) · [common behaviour](#common-behaviour) · [conditional formatting](#conditional-formatting) · [per visual](#per-visual) · [interaction](#interaction-summary) · [render verification](#render-verification) · [known gaps](#known-gaps)

## Pipeline

1. **Query** (`pbi_render::query`): the visual's `queryState` projections, report/page/visual filters, other slicers' selections and another visual's selected data points (per the page's `visualInteractions`) become `EVALUATE SUMMARIZECOLUMNS(…)` (or `ROW(…)`) run by `pbi-dax`. Inputs of [conditional formatting](#conditional-formatting) rules that are not in the visual are added as hidden values. The result is a Power BI-shaped **DataView** (`Single`, `Categorical`, `Table`, `Matrix`) with formatted cells (each with its [effective format string](#dynamic-format-strings)).
2. **Layout** (`pbi_render::layout`): page scaling (`FitToPage`, `FitToWidth`, `ActualSize`), paint order (z, then tab order), groups, and each visual's container (border, title, padding) → a content box.
3. **Draw** (`pbi_render::visuals`): `render_visual` dispatches on `visualType` and draws into a scene graph in page units, with theme lookups (`pbi_render::theme`) and text measured with the bundled fonts (`pbi_render::text`).
4. **Output**: deterministic SVG (`pbi_render::svg`, used by the app) or PNG (`pbi_render::png`, resvg; used by `pbiview render` and the golden tests). Interactive elements carry hit ids (`data-hit` in SVG).

Support levels used below: **Full** — the common formatting objects are honoured and the layout follows the service; **Partial** — the common options render, gaps are listed; **Placeholder** — a light box with the visual type and its bound fields (`Role: field, field`).

Verification ([details](#render-verification)): every type is covered by synthetic tests (`crates/pbi-render/tests/visuals_*.rs`); every page of the Acme Outfitters example (`reference/examples/acme-outfitters/`, which uses most visual types) is rendered by deterministic snapshot tests and checked against the example's expected values; every page of the Microsoft sample reports renders without panicking, with the expected visual counts. No Power BI renders are committed: pages are compared with Power BI exports only when optional local golden images exist. The "service behaviour reproduced" notes below describe what the renderer implements; the scatter, funnel, gauge, treemap and waterfall layouts follow Microsoft Learn's documentation and screenshots of those visuals. `cargo run -p pbi-render --example render_more_visuals` renders them end to end on a synthetic model (`target/tmp/more-visuals/page.png`). Conditional formatting and dynamic format strings are covered by `crates/pbi-render/tests/conditional_format.rs`; `cargo run -p pbi-render --example render_cond_format` renders them (`target/tmp/cond-format/{tables,formats,charts}.png`).

## Common behaviour

### Container (`visualContainerObjects`)

| Object | Honoured properties |
|---|---|
| `background` | `show`, `color`, `transparency` (an invisible, hit-testable rectangle when hidden) |
| `border` | `show`, `color`, `width`, `radius` (rounded corners clip the content) |
| `dropShadow` | `show`, `color`, `transparency`, `position`, `preset`, `angle`, `shadowDistance`, `shadowBlur`, `shadowSpread` |
| `padding` | per side; default 5 px (none for shapes) |
| `title` | `show`, `text` (default: an auto title from the fields), font family/size/color/bold/italic/underline, `alignment`, `titleWrap`, `background`, `transparency` |
| `visualHeader` | read, never drawn (the service shows it only on hover) |
| `visualLink` | button actions (see `actionButton`) |

Container lookups fall back to `objects` for schema ≤ 2.2 files without `visualContainerObjects`. With a 1 px border and the default 5 px padding, a table's header starts 6 px inside the visual and a chart title's cap height sits about 5 px below the top edge, as in the service.

Page: `background` and `outspace` colours and transparency; display option; hidden pages are listed but greyed in the app. Groups paint their own container before their first child; visuals inside a hidden group are hidden. Hidden visuals (`isHidden`) are neither queried nor drawn.

### Theme and property precedence

`pbi_render::theme` deep-merges the built-in fallback theme ← the report's base theme ← its custom theme. A property `object.prop` resolves from the first source that has it: the visual's `objects` / `visualContainerObjects` (default selector) → theme `visualStyles[visualType][preset]` → `visualStyles[visualType]["*"]` → `visualStyles["*"]["*"]` → built-in defaults. Colours accept literals, `ThemeDataColor{ColorId, Percent}` (ColorId 0 white, 1 black, 2–9 `dataColors[0..8]`, shaded by `Percent`) and theme colour names. Text classes (`callout`, `title`, `header`, `label` and the derived `largeTitle`, `semiboldLabel`, `boldLabel`, `largeLabel`, `smallLabel`, `lightLabel`, `largeLightLabel`, `smallLightLabel`) follow Power BI's theme documentation. Values that do not decode (`fontSize: 'abc'`) fall through to the next source.

### Fonts and text

- Only bundled faces are loaded, for measurement and PNG output, so renders are identical on every OS: **Selawik** (Light 300, Semilight 350, Regular 400, Semibold 600, Bold 700; SIL OFL 1.1), which has Segoe UI's metrics, and **Carlito** 1.104 (Regular, Italic, Bold, Bold Italic; SIL OFL 1.1), which has Calibri's advance widths and vertical metrics.
- `Segoe UI*` and `wf_segoe-ui_*` map to Selawik by weight; `DIN`, `Arial`, `Helvetica` and unknown families to Selawik Regular; `* Semibold`/`* Bold` names to those weights. The first family of a CSS list decides.
- **Calibri** (`Calibri`, `Calibri Light`, `Calibri Bold`, CSS lists starting with Calibri) renders with Carlito: Regular below weight 600 (Calibri Light has no Carlito counterpart and uses Regular), Bold from 600, italic faces for italic text. Text is measured and drawn at its nominal size with Calibri's line box (1950/550 per 2048 em: `line-height: normal` 1.22 em), in textboxes, cards and grids alike (Carlito is metric-compatible with Calibri).
- **DIN** (used by some themes' `title` class and slicer headers) has no open metric-compatible font: it renders as Selawik Regular, about 4% narrower than the service's DIN. Grids still condense Arial Narrow horizontally (width factor 0.82).
- Font sizes are points (`px = pt × 96/72`); lines use the family's `line-height: normal` (Segoe UI: 2724/2048 em, baseline at 2210/2048 em; Calibri: 2500/2048 em, baseline at 1950/2048 em).
- The SVG uses `font-family="Selawik, 'Segoe UI', sans-serif"`, or `font-family="Carlito, Calibri, sans-serif"` for Calibri; the app declares the same bundled faces (`app/src/app.css`), so on-screen text matches what was measured.

### Interactive re-render cost

The app re-renders the page on every interaction (`render_interactive_page`). Two caches keep that proportional to what changed:

- **Visual cache** (`pbi_render::visuals::cache`): each visual's content group, interactive geometry and dropdown popup are cached by a content fingerprint of its inputs: the visual definition (`objects`, position, …), its laid-out frame, the theme, its query result, its view state (scroll, expansion, dropdown, search) and the selections it reads (its own slicer selection; the point selection when it is the cross-filter source). Keys are content hashes, not revision counters: scrolling one table re-renders that table only, going back to an earlier state is a hit, and a cached group can never be stale. Output is byte-identical with or without the cache (tested on synthetic pages and on report pages). The cache is LRU-bounded (2048 renders, 250 000 scene nodes).
- **Scroll steps do no work proportional to the data.** `PageData::id` (set by `query_page_data`) lets renders remember each result's content hash instead of re-hashing it, and a table's or matrix's grid model, style and measured layout are kept per data, definition, frame and expansion ("prepared grids", up to 1 000 000 cells), so a scroll step only lays out and draws the visible rows. Interactive renders no longer copy every DataView into `RenderedPage::data` (only `render_report_page` fills it).

Measured with `cargo run --release -p pbi-render --example bench_scroll` on a 25-visual report page (Apple M4 Pro): a table scroll step renders in 0.19 ms instead of 0.75 ms (24 of 25 visuals reused; SVG 0.27 ms), an unchanged re-render takes 0.17 ms, and a slicer selection re-renders 16 of 25 visuals (0.46 ms instead of 0.54 ms, after its 1 ms query). On the 50 000-row synthetic table (`cargo xtask perf --scenario P01`) a scroll step (render + SVG) went from 25.6 ms to 0.75 ms at p95.

### States

| State | Drawing |
|---|---|
| Query error (unknown field, unsupported DAX function, cancelled, …) | ⚠ icon and "Can't display this visual"; the app tooltip shows the message and the failing measure's DAX. `pbiview render` reports it and exits 2 |
| Filter that cannot be applied (relative date, …) | the visual renders without it; the warning is listed in `render --json` (`warnings[]`) |
| Data cut at the query limits (30,000 rows; 1,000 categories; 60 series) | charts show a small "i" badge in the top-right corner of the plot; every visual reports `truncated` in `render --json` |
| BLANK | empty in value cells; `(Blank)` in single-value cards and in category, slicer and matrix header cells |

### Queries per visual type

| Visual type | Roles → query | DataView |
|---|---|---|
| `textbox`, `shape`, `basicShape`, `actionButton`, `image`, navigators, groups | no query | `Empty` |
| `card`, `cardVisual` | every role → values | `Single` (one field) or a one-row `Table` |
| `slicer`, `advancedSlicerVisual`, `listSlicer`, `textSlicer` | `Values` → group columns, evaluated without the slicer's own selection | `Categorical` + slicer selection |
| `tableEx`, `table`, `multiRowCard` | `Values`: columns → group, measures and aggregations → values | `Table` (+ totals) |
| `pivotTable`, `matrix` | `Rows`, `Columns` → levels; `Values` → values | `Matrix` (subtotals at every level) |
| pie, donut, `funnel`, `treemap` | `Category` (treemap: `Group` and `Details`) → categories; `Series` → series; `Y`/`Values` and others → values | `Categorical` |
| `scatterChart` | `Category`/`Details` (and `Play`) → categories; `Series`/`Legend` → series; `X`, `Y`, `Size`, `Tooltips` → values: one row per Details × Legend value | `Categorical` |
| `waterfallChart` | `Category` → categories; `Breakdown` → series; `Y` → values | `Categorical` |
| `gauge` | `Y`, `MinValue`, `MaxValue`, `TargetValue` → values (no category: one row) | `Categorical` |
| bar, column, line, area, combo, other `*Chart`, `kpi`, `map`, `filledMap` | `Category`/`Axis`/`Details`/`Group`/`Rows`/`Location`/`Play` → categories; `Series`/`Legend`/`ColorBy`/`Breakdown` → series; `Y`, `Y2`, `Tooltips`, `Size`, `X`, … → values | `Categorical` |
| anything else | columns → group, measures → values | `Table` (placeholder data) |

- **Implicit measures**: a plain column in a value role uses the model's `summarizeBy` when it is an aggregation; otherwise `SUM` for numbers and `COUNTA` for other types; on cards a text column shows its first value. Explicit aggregations map to `SUM`, `AVERAGE`, `MIN`, `MAX`, `COUNTA`, `DISTINCTCOUNT`, `MEDIAN`, `STDEV.P`, `VAR.P`.
- **Sorting**: the visual's `sortDefinition` (honouring a column's `sortByColumn`); otherwise bar/column/pie/donut/funnel/treemap/ribbon charts by the first value descending, other charts (line, area, scatter, waterfall, …) by category ascending, tables by the first column, slicers and matrix row and column levels ascending (by each column's `sortByColumn` when it has one). Text sorts case-insensitively, BLANK first.
- **Totals** are re-evaluations with fewer group columns (correct for non-additive measures). `ALLSELECTED` and `ISINSCOPE` work inside visuals.
- **Filters**: categorical and tuple `In`/`Not In`, advanced comparisons and `Between`, `And`/`Or`/`Not`, contains/starts-with, blank tests and Top N are applied at every level; measure conditions on the visual's rows; relative date/time filters, auto date/time levels and a few rare forms are reported as not applied. Report filters do not affect visuals whose tables have no relationship path to the filtered table (a card over an unrelated table keeps its unfiltered value). `isHiddenInViewMode` and `isLockedInViewMode` only affect the Filters pane.

### Dynamic format strings

A measure's format string can depend on the cell: a calculation item's `formatStringDefinition` (a `YOY %` item formats as a percent whatever the measure's own format), or the measure's own `formatStringDefinition` (for example by magnitude). As Power BI's visual queries carry a format-string column next to such a measure, the semantic query evaluates the **effective format string** of every cell of a measure that may have one (a measure with a `formatStringDefinition`, or any measure while a calculation group with format strings is filtered or grouped by the visual, or has a formatted `noSelectionExpression`) in one batch per measure and result set (`pbi_dax::EvalContext::evaluate_measure_format_strings`), formats the cell with it and keeps it in the cell (`CellValue::format`) when it differs from the column's format string. Measures without dynamic format strings cost nothing extra.

- **Cards, tables, matrices**: the cell text uses the effective format string (`82.4%` under `YOY %`, `$960` without it; totals and subtotals without the item use the measure's own).
- **Chart data labels and tooltips**: each value is formatted with its own format string (percent values stay percents when display units apply to currency values). Value axes keep the field's format string.
- Implicit measures (aggregations) never have dynamic format strings (calculation items do not apply to them).

## Conditional formatting

`pbi_render::conditional` reads the conditional formats that Power BI stores in a visual's `objects` (semantic-query expressions instead of static values) and applies them per cell; the query adds the fields the rules are based on as hidden values (so a rule "based on" `[Margin]` works without `[Margin]` in the visual, with the summarization of the rule's input). As in Power BI, a hidden formatting measure that is never BLANK keeps rows whose visible values are all BLANK.

| Format style | PBIR | Supported |
|---|---|---|
| Gradient (color scale) | `{"FillRule": {"Input", "FillRule": {"linearGradient2" \| "linearGradient3"}}}` | 2 or 3 colors; minimum/center/maximum from the data (lowest, mean of lowest and highest, highest over the formatted cells that are not totals) or numbers (`value`); values outside the range take the end colors; empty values (`nullColoringStrategy`) as zero (default), not colored or a specific color |
| Rules | `{"Conditional": {"Cases": [{"Condition", "Value"}], "DefaultValue"}}` | Comparisons (`=`, `>`, `>=`, `<`, `<=`), `And`, `Or`, `Not` over numbers, text (case-insensitive) and the input; first matching rule wins; "is blank" (`= null`) matches BLANK, which matches no numeric comparison; percent-of-range bounds written with `Arithmetic` over the visual-wide `Min`/`Max` (or `Percentile`) of the input, also inside a `ScopedEval` with an empty scope |
| Field value | a `Measure`, an `Aggregation` (First = `Min`, Last = `Max`, …) or a `Column` | CSS colors: `#RGB`, `#RRGGBB`, `#RRGGBBAA`, `rgb[a]()`, `hsl[a]()`, the CSS color names, `transparent` |
| Data bars | `dataBars: {positiveColor, negativeColor, axisColor, reverseDirection, hideText, minValue, maxValue}` | The cell's own value from the minimum (default: the lowest value, or 0 when all are positive) to the maximum (default: the highest, or 0 when all are negative); positive and negative colors (defaults: the first data color and `#D64554`), an axis line (default `#252423`) where the range crosses zero, right-to-left, "show bar only"; values only, never totals; drawn 2 px inside the cell under the text |
| Icons | `icon: {kind, layout, verticalAlignment, value}` with rules or a field value | Built-in icons drawn as simple vector shapes (circles, rimmed traffic lights, squares, diamonds, triangles up/down/dash, arrows in five directions, flags, check marks, crosses, exclamation marks, stars (empty to full), signal bars), from names such as `circle_green` or `ArrowUpGray` (word-based, case-insensitive); before or after the value or alone (`IconOnly`), top/middle/bottom aligned; they count in auto-sized column widths |

Where they apply:

| Visual | Objects | Cells |
|---|---|---|
| `tableEx`, `table` | `values` and `columnFormatting` entries with `selector.metadata` = the column's query ref: `backColor`, `fontColor`, `dataBars`, `icon` | `selector.data[].dataViewWildcard.matchingOption`: `1` values (default), `2` the totals row, `0` both |
| `pivotTable`, `matrix` | as tables, per value field | values = rows at the lowest visible level × non-total columns; totals = subtotal and grand-total rows and columns; expanded parents shown without subtotals are not formatted |
| Bar and column charts, `pieChart`, `donutChart`, `treemap`, `scatterChart`, `funnel` (without a legend field) | `dataPoint.fill`, for every value or the measure in `selector.metadata` | per category (per point for scatter); the conditional color wins over per-category and default colors, and points no rule colors keep their usual color; legends show the slice colors (pie) |

The conditional color replaces the column's background (`columnFormatting.backColor`), the row band and the text color, as in Power BI. Rendering stays deterministic (no images, no fonts other than the bundled ones), and the per-visual render cache keys on the formatting objects and on the query result, hidden values included, so changing a rule re-renders exactly the affected visual.

**Not supported** (the visual renders without that format and the query reports a `not applied` warning in `render --json`): fill rules other than the two linear gradients, `scopeId` (single data point) selectors, rules without a target column, conditional formats of other objects (column headers, totals' own objects, titles, card callouts, gauge and KPI colors, …) and of other visual types, conditional data colors of charts with a legend field (Power BI's per-legend colors from a measure) and of line, area, combo, waterfall and gauge charts, rule inputs that are not in the model or fail to evaluate, web URL formatting and image (URL) icons (a field-value icon that is not a built-in icon name draws no icon).

## Per visual

### `textbox` — Full

- **Honoured**: `objects.general.paragraphs[].textRuns[]` (`value`, `url`, `textStyle`: `fontFamily`, `fontSize` in pt/px/number, `fontWeight`, `fontStyle`, `textDecoration`, `color`), `horizontalTextAlignment` (`justify` renders left), `listType` (bullets, numbers), `indentLevel`. Unstyled runs use Segoe UI 10 pt in the theme foreground; links use the theme `hyperlink` colour, underlined.
- **Service behaviour reproduced**: greedy word wrap across runs (long words break between glyphs), `\n` hard breaks, no paragraph spacing; each line box includes a Segoe UI 12 px strut (the editor's paragraph font), which reproduces the service's baselines (9 pt Calibri bullets on a 16 px pitch, a 22 pt spacer paragraph 39 px tall); list markers right-aligned in a 1.2 em box; text top-aligned (textboxes have no vertical alignment); an 8 px `#C8C8C8` scrollbar thumb when the text overflows.
- **Differences**: Calibri renders with Carlito (same advance widths and line metrics, slightly lighter strokes than the service's Calibri); a single-line title textbox can sit 1 px higher than the service's.
- **Interaction**: none; links are not clickable.

### `shape`, `basicShape` — Full

- **Honoured**: `shape.tileShape` (also `shapeType`): rectangle, rounded rectangle, oval, line, triangle, right triangle, pentagon, hexagon, octagon, parallelogram, trapezoid, diamond, arrows (four directions), chevron, pentagon arrow, heart, plus, star, and four tab shapes; unknown names draw a rectangle. `roundEdge`, `fill` (`show`, `fillColor`, `transparency`), `outline` (or legacy `line`: `show`, `lineColor`, `weight`, `transparency`; drawn inside the bounds), `rotation.shapeAngle`, `text` (font, horizontal and vertical alignment).
- **Service behaviour reproduced**: shapes cover the whole container inside its border (padding does not apply) and are clipped to it. Defaults: fill theme data colour 1, outline that colour darkened 50%.
- **Verified**: synthetic tests (`visuals_basics.rs`: full-container fills, outlines, rotation, lines, text) and the Acme Outfitters render snapshots.
- **Interaction**: none.

### `actionButton` — Full; `pageNavigator`, `bookmarkNavigator` — Partial

- **Honoured** (default state only): `shape.tileShape`, `roundEdge`; `fill`; `outline`; `icon` (`show`, `shapeType`: blank, back, forward, left/right arrow, information, help, Q&A, bookmark, reset, filter, clear, apply; `lineColor`, `lineWeight`, `lineTransparency`, `fillColor`, `padding`); `text` (font, alignment; the icon sits on its left).
- **Service behaviour reproduced**: the icon is a square of the content's shorter side; the Back icon is an arrow in a circle (10% strokes, `#605E5C`), as on the service's 40×40 button.
- **Actions** (`visualLink`): `Back`, `PageNavigation`, `Bookmark`, `Drillthrough`, `WebUrl`, `QnA`, `ApplyAllSlicers`, `ClearAllSlicers`; `show = false` disables it.
- **Interaction**: Back returns to the previous page in the app's navigation history; page navigation opens the target page; Clear all slicers clears them; Apply all slicers does nothing (selections apply at once); Web URL shows the URL with Copy link (never opened); bookmarks, drillthrough and Q&A show "not available".
- **Gaps**: hover/pressed/disabled states are not drawn. Navigators render as one labelled button (the page or bookmark list is not available to visuals) and have no navigation action.

### `cardVisual` (new card) — Full

- **Honoured**: one tile per value field (or per category value) in a grid of up to `layout.maxTiles` per row (theme 3) `cellPadding` apart; `layout.paddingUniform`, `layout.alignment`; tile `padding`; `value.*` over the `callout` class and `label.*` over `label` (`position` `belowValue`/`aboveValue`), `spacing.verticalSpacing`; tile `border` and `outline`, `shapeCustomRectangle.tileShape` (rounded corners); `fillCustom`, `cardCalloutArea`; `accentBar`; reference labels (`*reference*` roles) with an optional `divider`.
- **Service quirks reproduced**:
  - **Inner tile border**: the tile draws its own border, narrower than the container, when `border.show` or `outline.show` (on by default, theme `backgroundLight`). Reports that turn both off usually draw the container border with padding 0 instead.
  - **Label clipping below ~100 px**: when value + label are taller than the tile, the block is top-aligned and clipped, so the label disappears first (with typical paddings, below about 100 px of card height). Values and labels clip at the tile edge without an ellipsis.
  - **Font sizes**: `value.fontSize` and `label.fontSize` are honoured.
- **Verified**: synthetic layout tests (`visuals_basics.rs`: value and label positions, clipping, tile borders) and every card value of the Acme Outfitters pages ([render verification](#render-verification)).
- **Interaction**: none (cards are not selectable data points).

### `card`, `multiRowCard` — Full; `kpi` — Partial

- **`card`**: the value (`labels.*` over `callout`) centred and shrunk to fit the width, with the category label below (`categoryLabels.*` over `largeLightLabel`, 12 pt `#605E5C`) when `categoryLabels.show`.
- **`multiRowCard`**: one block per row: an optional leading group column as the card title (`cardTitle.*`), then each field as value (`dataLabels.*`) over its name (`categoryLabels.*`), flowing left to right; accent bar (`card.barShow`, `barWeight`, `barColor`) and separator (`card.outlineColor`, `outlineWeight`).
- **`kpi`**: the last non-blank `Indicator` value (`indicator.*`), coloured by `Goal` with `status.direction` (theme `good`/`bad`), `Goal: … (±x%)` below, and the trend (the `Indicator` over the `TrendLine` axis) as a translucent area across the bottom (`trendline.show`, `transparency`). A basic layout, not checked against service renders.
- **Interaction**: none.

### `slicer` — Full (list, dropdown, tile); Partial (range, relative date); `advancedSlicerVisual` — Partial

- **Modes** from `objects.data.mode`: `Dropdown`; `Basic` (vertical list, or tiles when `general.orientation` is horizontal); `Between`/`Before`/`After`/`Relative` (range). Without a mode, numeric and date fields default to a range and others to a list. `advancedSlicerVisual` (button slicer) renders tiles.
- **Honoured**: `header` (`show`, `text`, `fontColor`, `fontSize`, `fontFamily`, `bold`, `italic`, `underline`, `background`, `outline`); `items` (font, colour, `padding`); `general.selfFilterEnabled` (search box), `selection.selectAllCheckboxEnabled`, `selection.singleSelect` / `strictSingleSelect`; saved selections (`isInvertedSelectionMode`, filter conditions); range bounds from the saved `Between` / `>=` / `<=` condition, else the data's min and max.
- **Service behaviour reproduced**: header in DIN 10 pt `#252423`, one line starting 8 px inside, ellipsized before a 26 px reserve for the eraser icon; controls start 23 px below the content top; dropdown box inset 5 px, `#EAEAEA` border, summary "All" / the single value / "Multiple selections", chevron 6.5 px from the right; list rows `line height + padding + 3` tall with checkboxes (multi-select) or radio buttons (single select) and a scrollbar.
- **Verified**: synthetic tests (`visuals_grids.rs`: dropdown, list, search, inverted and single selection, tiles, ranges) and the saved selections of the Acme Outfitters slicers (`visuals_grids_reference.rs`). DIN headers render in Selawik ([fonts](#fonts-and-text)).
- **Interaction**: click selects (single select replaces; multi-select with Ctrl replaces on click and toggles on Ctrl/⌘+click; otherwise every click toggles); Select all toggles all/none; inverted mode unchecks to exclude; dropdowns open a popup drawn above the page; search box filters items; wheel scrolls the list; the eraser clears the selection. The CLI selects headlessly with `render --slicer`.
- **Gaps**: range sliders and relative-date pickers are drawn but not draggable; tiles of a button slicer select like list items; synced slicers (`syncGroup`) and the newer slicers' `selfFilter` are not read, so each page keeps its own selection.

### `tableEx`, `table` — Full

- **Honoured**: `columnHeaders` (font, colours, `alignment`, `wordWrap`, `outline`/`outlineStyle`), `values` (primary and alternate row colours, default banding `#EEEDED`, font, `wordWrap`), `total` (font, colours, bold by default), `grid` (horizontal and vertical lines, colours, weights, `rowPadding`, `outlineColor` (theme `tableAccent`), `outlineWeight`, `textSize`), `columnWidth` per column, `columnFormatting` (`alignment`, `fontColor`, `backColor`), `stylePreset` (`None`, `Minimal`, `Sparse`, `BoldHeader`, `Condensed`, default). Sort arrows under the headers of an explicit `sortDefinition`.
- **Service behaviour reproduced**: browser line metrics (17 px lines for 10 pt Segoe UI); 6 px cell padding; the last header row reserves an 8 px strip for the sort arrow; auto-sized columns (widest of the header and the first rows, capped when values wrap, then wrapped); tables narrower than the visual stay left-aligned; a 9 px scrollbar band with a 7 px `#AEADAC` thumb; the header stays fixed while the body scrolls; the total row scrolls with the rows; numbers right-aligned.
- **Conditional formatting**: background and font colors (gradients, rules, field values), data bars and icons per column, on values, totals or both ([above](#conditional-formatting)).
- **Verified**: synthetic tests (`visuals_grids.rs`: auto-sizing, wrapping, banding, totals, scrollbars) and the Acme Outfitters tables' rows, order and totals against `expected.json` (`visuals_grids_reference.rs`).
- **Interaction**: click a row to cross-filter by its group columns (Ctrl/⌘ adds rows); wheel and Shift+wheel scroll; hover shows the row's cells.
- **Gaps**: URL icons and images, web URL formatting, header click-to-sort.

### `pivotTable`, `matrix` — Full

- **Honoured**: the table's objects (shared grid engine) plus `rowHeaders.stepped` (stepped layout with `steppedLayoutIndentation`, default 10 px; or one column per level), `subTotals.rowSubtotals`, `rowHeaders.showExpandCollapseButtons`, persisted `expansionStates`.
- **Behaviour**: only the first row level shows unless expanded; expanded parents show their subtotals in bold (stepped) or a bold "Total" row follows the group (tabular); the grand total row is pinned at the bottom; one header row per column level with cells spanning their leaf columns, a value-name row when there are several values, "Total" subtotal and grand-total columns.
- **Conditional formatting**: as tables, per value field; "values" are the cells of the lowest visible level, "totals" the subtotal and grand-total rows and columns ([above](#conditional-formatting)). Cells under a calculation item use the item's format string.
- **Interaction**: click a row to cross-filter by its row path; double-click or `+`/`−` to expand or collapse; scroll; hover shows the row path and cell.
- **Gaps**: drill-down and drill-up commands, conditional formatting of row and column headers (not available in Power BI either).

### `pieChart`, `donutChart` — Full

- **Honoured**: `legend`; `dataPoint.fill` per category (selector scope ids) or [conditional](#conditional-formatting) (gradient, rules, field value), `defaultColor`; `labels` (`show`, `color`, `fontSize`, `fontFamily`, `labelDisplayUnits`, `labelPrecision`, `labelStyle` — category, data value, percent of total, combinations, all detail labels —, `labelPosition` `outside`/`inside`/`preferOutside`); `slices.innerRadiusRatio` (donut, default 50%).
- **Service behaviour reproduced**: slices sorted by value descending from 12 o'clock clockwise; radius 80% of the half-size with outside labels; leader lines radial to 90% of the half-size, then 10 px horizontal; each label tried at ¾ of its arc, the middle, then ¼, on one line (`1,234 (61.70%)`), then on two lines; labels must stay clear of the outer ~27% (one line) or ~20% (two lines) of the half-width, the pie and earlier labels, otherwise they go where there is most room, truncated inside the parentheses (`766 (38.3...)`). Palette slots are consumed only by points without an explicit colour, so a pinned colour does not shift the others.
- **Differences**: Selawik's narrower digits can fit one more character in a truncated label, and DIN titles render in Selawik, about 4% narrower.
- **Interaction**: click a slice to cross-filter (Ctrl/⌘ adds); the source pie dims its other slices to 40%; hover shows category, value and tooltip fields.

### Bar and column charts — Full

Types: `clusteredBarChart`, `barChart`/`stackedBarChart`, `hundredPercentStackedBarChart`, `clusteredColumnChart`, `columnChart`/`stackedColumnChart`, `hundredPercentStackedColumnChart`.

- **Honoured**: `legend` (`show`, eight positions, title, colours, font); `categoryAxis` (`show`, title, colours, font, `gridlineShow`, `axisType`, `innerPadding`, `preferredCategoryWidth`, `maxMarginFactor`); `valueAxis` (`show`, title, `start`, `end`, `labelDisplayUnits`, `labelPrecision`, gridline show/colour/thickness/style); `dataPoint` colours per category, series or measure, and [conditional](#conditional-formatting) colours without a legend field; `labels` (`labelPosition` `OutsideEnd`/`InsideEnd`/`InsideCenter`/`InsideBase`/`Auto`, display units, precision; each value with its own format string).
- **Service behaviour reproduced**: horizontal bars with the category title rotated on the left, labels right-aligned 9 px left of the plot within 25% of the width, value axis at the bottom with dotted `#C8C6C4` gridlines; bands at least `max(preferredCategoryWidth, 1.4 × label line height)` with bars filling 70%; "nice" ticks (1, 2, 2.5, 5 × 10ⁿ); the value axis ends at the largest bar or stack so the longest bar reaches the plot edge; when categories do not fit, the first ones are drawn with a scrollbar. Column charts wrap category labels on two lines, then slant them −35° (−90° when scrolling).
- **Differences**: DIN chart and axis titles render in Selawik (no open metric-compatible DIN exists).
- **Interaction**: click a bar to cross-filter by category and series; tooltips. Category scrollbars are drawn but not scrollable.

### Line, area and combo charts — Full

Types: `lineChart`, `areaChart`, `stackedAreaChart`, `lineClusteredColumnComboChart`, `lineStackedColumnComboChart`.

- **Honoured**: the bar/column options above; `lineStyles` (`strokeWidth` (theme 3), `lineStyle`, `showMarker`, `markerSize`); `y2Axis` for combo lines (`Y2` role, secondary axis on the right).
- **Behaviour**: one polyline per series through category centres, or a continuous x scale for dates and numbers (day/week/month/quarter/year ticks) unless `categoryAxis.axisType` is `Categorical`; lines break at BLANK; areas fill to 0, stacked areas to the series below (separate positive and negative stacks).
- **Interaction**: click a point (invisible hit circles over the markers) or a column to cross-filter; tooltips.
- **Gaps**: the Analytics pane (trend, constant and average lines, forecast) and small multiples are not read.

### `scatterChart` — Partial

- **Roles**: `Category`/`Details` (one point per value), `Series`/`Legend` (one color per value), `X`, `Y`, `Size` (bubbles), `Play`, `Tooltips`. A point needs a numeric X and Y; BLANK, text, NaN and ±∞ are skipped; negative values are plotted.
- **Honoured**: `categoryAxis` (the X axis) and `valueAxis` (the Y axis): `show`, title (default: the X / Y field names), `start`, `end`, `labelDisplayUnits`, `labelPrecision`, fonts and colors, gridlines (on for both axes, dotted `#C8C6C4`); `legend` (shown with a legend field); `dataPoint.fill` per legend value or per point, `defaultColor`; `bubbles.bubbleSize` (radius × 2^(size/10)), `bubbles.markerShape` (`circle`, `square`, `diamond`, `triangle`), `bubbles.markerRangeType` (`auto`, `magnitude`, `dataRange`); `fillPoint.show` (hollow markers when off); `categoryLabels` (`show`, off by default: the Details value above the marker; font, color); `labels` (`show`, off by default: the Y value beside the marker; display units, precision, font, color).
- **Service behaviour reproduced**: numeric axes over the data extent (zero is not forced) with "nice" ticks, padded so markers stay inside the plot; 8 px markers; bubble areas proportional to the size — "magnitude" (0 to the largest magnitude) for single-sign sizes and "data range" (smallest to largest) when signs are mixed, as Power BI's Auto range scaling — from 2 px to 7.5% of the plot's shorter side; larger bubbles drawn first so small ones stay visible; at most 10,000 points.
- **Differences**: a play axis is not animated (only the first frame, the smallest play value, is drawn; the other frames are queried but not shown); labels that would overlap or leave the plot try the other sides of the marker, then are dropped.
- **Conditional colors**: `dataPoint.fill` by gradient, rules or field value, per point, without a legend field ([above](#conditional-formatting)).
- **Ignored**: the Analytics pane (trend, constant, average, median, percentile lines, symmetry shading, ratio line), `colorBorder`, the `Gradient` role (color saturation), log-scale axes (`axisScale`), zoom sliders, auto-fit markers (markers always fit), plot-area images, high-density sampling (a plain 10,000-point cap).
- **Interaction**: click a marker to cross-filter by its Details and legend values (Ctrl/⌘ adds); hover shows the Details, legend, X, Y and Size values, then any Tooltips fields.

### `funnel` — Full

- **Roles**: `Category` + the first `Y`/`Values` measure, or several measures without a category (one stage per measure).
- **Honoured**: `categoryAxis` (`show`, label font and color); `dataPoint` (`defaultColor`, `fill` per category); `labels` (`show` (default on), `color`, font, `labelDisplayUnits`, `labelPrecision`, `labelPosition` `outside`, contents in `funnelLabelStyle` or `labelStyle`: data value (default), percent of first, percent of previous, combinations, all detail labels); `percentBarLabel` (`show` (default on), font, color).
- **Service behaviour reproduced**: stages keep the query order (by value descending unless the visual is sorted), top to bottom; bars centred and as wide as their value relative to the largest; category labels left-aligned at the left, truncated with `...` within 30% of the width; one color for every bar (`dataColors[0]`); data labels centred inside the bar in a contrasting color, else right of it; the conversion-rate labels `100%` above a bracket over the first bar and the last stage as a percent of the first (`5.2%`) under a bracket over the last bar; stages that do not fit are cut with a scrollbar.
- **Differences**: negative values are drawn by magnitude; BLANK and zero stages keep their row and label without a bar.
- **Conditional colors**: `dataPoint.fill` by gradient, rules or field value, per stage ([above](#conditional-formatting)).
- **Ignored**: `labelPosition` `insideEnd`/`insideBase` (drawn centred), small multiples.
- **Interaction**: click a bar to cross-filter (Ctrl/⌘ adds); the source dims the other bars; tooltips.

### `gauge` — Full

- **Roles**: `Y` (value), `MinValue`, `MaxValue`, `TargetValue`.
- **Honoured**: `axis` (`min`, `max`, `target` when the matching field is not bound); `dataPoint` (`fill`, `target`); `labels` (minimum/maximum labels: `show`, color, font, display units, precision); `targetLabel` (`show`, color, font, display units, precision); `calloutValue` (`show`, `color`, `fontSize`, `fontFamily`, display units, precision).
- **Service behaviour reproduced**: by default the minimum is 0 and the maximum is double the value (Microsoft Learn: the value sits half-way); a 180° arc, inner radius 70% of the outer one, grey track `#E9E9E9`, filled from the minimum to the value in `dataColors[0]`; the target as a needle across the arc with its label outside; the minimum and maximum under the arc ends; the callout value in the hole, sized to the inner radius and shrunk to fit. All numbers share one display unit (Auto from the largest of value, bounds and target) and the decimals of the value's format string (`184.21K`, `0.00K`, `368.41K`).
- **Differences**: a negative value without bounds uses `[2 × value, 0]`; a BLANK value shows `(Blank)` over an empty track; the DIN callout renders in Selawik.
- **Ignored**: conditional fill colors (rules), callout and label positions.
- **Interaction**: hover shows the value; no cross-filtering (a gauge has no category).

### `treemap` — Full

- **Roles**: `Group` (or `Category`) → branches, `Details` → leaves inside them, `Values` → sizes, `Legend`/`Series` → leaves per series value colored by series.
- **Honoured**: `dataPoint` (`fill` per group value, `defaultColor`); `categoryLabels` (`show` (default on), color (default white), font); `labels` (`show` (default off), color (default white), font, display units, precision); `legend` (off by default).
- **Service behaviour reproduced**: rectangles arranged by size, largest branch at the top left, with the squarified algorithm (Bruls, Huizing & van Wijk 2000; deterministic, input order sorted by value); leaves inside each branch the same way; branches take palette colors in size order and leaves share their branch's color; 2 px gaps between branches and 1 px between leaves; the branch name at the top left, leaf names at the bottom left, truncated with `...` or dropped when the rectangle is too small. Zero, negative, BLANK and NaN values get no area.
- **Conditional colors**: `dataPoint.fill` by gradient, rules or field value, per leaf, without a legend field ([above](#conditional-formatting)).
- **Ignored**: the color saturation (`Gradient`) well, drill-down, label positions.
- **Interaction**: click a leaf to cross-filter by its group and details values (Ctrl/⌘ adds); the source dims the other leaves; tooltips.

### `waterfallChart` — Full

- **Roles**: `Category`, `Breakdown` (optional), `Y`.
- **Honoured**: `sentimentColors` (`increaseFill`, `decreaseFill`, `totalFill`, `otherFill`; defaults from the theme: `good`, `bad`, `dataColors[0]`, `neutral`, as Microsoft Learn's theme documentation says waterfalls use the status colors); `breakdown.maxBreakdowns` (default 5); `legend` (Increase, Decrease, Total, Other; on by default); `categoryAxis`, `valueAxis` and `labels` as the column charts (data labels off by default: the change, or the total).
- **Service behaviour reproduced**: categories in ascending order by default; each category floats from the running total before it to the one after it, and a final "Total" column goes from 0 to the sum. With a breakdown, the first category is a total column, then each later category shows the change from the previous one per breakdown value — the largest `maxBreakdowns` changes by magnitude plus "Other" for the rest, from the largest increase to the largest decrease — followed by its own total column. Thin connectors join each column's end to the next one; category labels wrap, slant and scroll like column charts.
- **Differences**: with a breakdown, category totals are the sum of the breakdown values (the service shows the same for additive measures); there is no extra grand-total column after the last category.
- **Ignored**: Analytics pane lines, log-scale axes, conditional colors.
- **Interaction**: click a column to cross-filter by its category (and breakdown value for a breakdown change); totals and "Other" select the category only; tooltips.

### Placeholders

Every other type renders as a placeholder box with the type and bound fields: `image`, `map`, `filledMap`, `shapeMap`, `azureMap`, `ribbonChart`, `decompositionTreeVisual`, `keyDriversVisual`, `qnaVisual`, R and Python visuals, Power Apps and Power Automate visuals, and custom visuals (`.pbiviz` packages are never executed). The query still runs, so a broken field reference in a placeholder visual is still reported.

## Interaction summary

The interactive canvas is in the app ([app/README.md](../app/README.md#the-interactive-report-canvas-m4-interactive)); the CLI renders the saved state or `--slicer` selections.

| Visual | Select | Cross-filter source | Scroll | Other |
|---|---|---|---|---|
| Slicers (list, dropdown, tiles) | items (single, multi, Ctrl/⌘, Select all), search | filters other visuals per `visualInteractions` | list, popup | eraser clears; Reset to default restores the saved selection |
| Pie, donut, bar, column, line, area, combo, scatter, funnel, treemap, waterfall | data points (Ctrl/⌘ for several) | yes | no | tooltips; source dims unselected points |
| Gauge | — | no (no category) | no | tooltip |
| Table | rows | yes (group columns) | both axes | row tooltips |
| Matrix | rows | yes (row path) | both axes | expand/collapse, tooltips |
| Action buttons | — | — | — | Back, page navigation, clear all slicers; Web URL shown only |
| Cards, textbox, shape, navigators, placeholders | — | — | — | clicking selects the visual for the panes |

`visualInteractions`: `NoFilter` skips a target; `DataFilter` (default) filters; `HighlightFilter` filters instead of highlighting (with a warning). Clicking the selected point again, the visual background, the empty canvas or Esc clears the cross-filter.

## Render verification

Every check below runs on committed fixtures, except the golden-image comparison, which needs optional local files. All renders use the bundled fonts and a fixed clock, so they are the same bytes on every OS.

**Render snapshots** (`crates/pbi-render/tests/render_snapshots.rs`): every page of the three Acme Outfitters reports, refreshed offline and rendered with its saved slicer selections, is summarised per visual (type, layout box, data points, warnings, every drawn text and a hash of the visual's SVG) plus a hash of the whole page's SVG, and compared with the `insta` snapshots in `crates/pbi-render/tests/snapshots/`. Each page is rendered twice and must give the same SVG. Any rendering change shows up as a snapshot diff naming the visuals that changed: review it with `cargo insta review` and accept it only when the change is intended.

**Expected-value assertions**: all 39 card values of the example's `expected.json` ([EXPECTED.md](../reference/examples/acme-outfitters/EXPECTED.md)) must be drawn above their labels on the rendered pages and appear in the SVG (`visuals_basics_reference.rs`), and must match in the visuals' DataViews (`query_reference.rs`), including a card over a table with no relationship path to a report filter, which keeps its unfiltered value. The tables, matrices, KPI, gauge and pie recorded in `expected.json` are checked in their DataViews (`query_reference.rs`: totals, rows, the KPI's last trend point) and several on the rendered pages (`visuals_grids_reference.rs`, `visuals_charts_reference.rs`: the Top N table's order, the calculation-group matrix's columns, the store tables, saved slicer selections, the KPI's indicator and goal). These asserts are never weakened (AGENTS.md rule 11).

**Microsoft samples** (`samples_reference.rs`): each sample report's structure matches its `expected.json` (pages, sizes, visibility, visual counts by type) and every page renders without panicking, with one render entry per visual; the larger fetched samples are checked when present ([README](../reference/samples/microsoft/README.md)).

**Synthetic tests** (`visuals_basics.rs`, `visuals_grids.rs`, `visuals_charts.rs`, `visuals_more.rs`, `conditional_format.rs`): each visual's layout rules on hand-built DataViews.

### Golden-image comparison

`crates/pbi-render/tests/golden.rs` renders each Acme Outfitters page at scale 2 and compares it with a PNG export of the same page made by Power BI (Desktop or the service), wherever one exists at `reference/golden/<example>/<report>/<page>.png`: the report's short name (`--report`) and the page's display name, for example `reference/golden/acme-outfitters/Overview/Executive Summary.png`. Golden images are optional local files: `reference/golden/` is gitignored, none are committed, and a page without one prints `SKIP: <file> missing`.

`pbi_render::golden` aligns the export with the render (the page scale from the page bounds, so any export scale works; the page at the top left, padding allowed: a 2560×1460 export of a 1280×720 page has the page in the top 2560×1440 px), then computes **SSIM** (Wang et al. 2004: 11×11 Gaussian window, σ = 1.5, K1 = 0.01, K2 = 0.03) on BT.601 luminance at one sample per page unit, per visual layout box and for the whole page. Each comparison writes the render, the aligned golden, a diff heatmap (regions outlined green or red) and the scores to `target/tmp/golden/` (never commit them).

Every visual region and the whole page must reach the default threshold, SSIM ≥ 0.85 (`pbi_render::golden::DEFAULT_THRESHOLD`); there are no per-page or per-region exceptions. A lower threshold needs the heatmaps inspected and the cause documented in `golden.rs` and here (AGENTS.md rule 11). Without golden images the same test still runs the pipeline on a render compared with itself (directly and inside export-like padding at another scale) and checks that renders through the visual cache match uncached ones. Run it with the scores printed: `cargo test -p pbi-render --test golden -- --nocapture`.

**Known differences from the service**:

- Text in DIN: no open metric-compatible DIN exists, so DIN text renders in Selawik; glyph shapes, advance widths and line metrics differ, so titles are about 4% narrower and text lands a few pixels off. Segoe UI text (Selawik) and Calibri text (Carlito) use metric-compatible fonts: same metrics, slightly different glyph shapes and stroke weights.
- Pie labels: truncation can keep one more digit.
- DashCraft never draws visual headers (the service shows them only on hover).

## Known gaps

- Highlight interactions filter instead of highlighting; a visual's own selection dims but does not highlight.
- Bookmarks, bookmark navigators, drillthrough, Q&A, report-page tooltips, visual header menus (focus mode, show as table, sort, export), drill-down/up and right-click menus are not available.
- Synced slicers, range-slider dragging, relative-date pickers.
- Conditional formatting: web URLs and image icons, conditional colors of charts with a legend field and of line, area, combo, waterfall and gauge charts, conditional formats of headers, titles and cards ([details](#conditional-formatting)); Analytics pane lines, small multiples.
- Scatter play axes are not animated (the first frame is drawn); `ribbonChart` and maps are placeholders.
- `image` visuals and page wallpaper images are not drawn; button hover/press states are not drawn.
- Report-level measures (`reportExtensions.json`) are not evaluated.
- PBIX `Report/Layout` and PBIR-Legacy reports render with the built-in default theme in the CLI.
- Data reduction is a plain cap, not Power BI's sampling algorithms.
- No DIN metric-compatible font is bundled (no open one exists); Calibri uses Carlito.
