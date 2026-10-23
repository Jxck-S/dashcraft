# ADR-0005: Report rendering in Rust (DataView → scene graph → SVG)

- Status: Accepted; §6 (interaction) superseded by [ADR-0006](0006-interaction-state.md)
- Date: 2026-10-21
- Deciders: pbiview maintainers
- Related: [ADR-0001](0001-ui-tauri-svelte.md), [ADR-0002](0002-arrow-datafusion.md), [ADR-0004](0004-desktop-ready-architecture.md)

## Context

Phase 2 renders PBIR report pages: absolute layout, z-order, `visualContainerObjects`, themes, and the visuals in our test reports (`textbox`, `shape`, `actionButton`, `cardVisual`/`card`, `slicer`, `tableEx`/`pivotTable`, pie/donut, bar/column, line, plus a placeholder for anything else). It needs:

- an interactive view in the app (slicers, cross-filtering, page navigation);
- `pbiview render <file> --page <name> --png out.png` for headless CI, with no browser;
- golden-image regression against service exports in `reference/golden/` (per-region SSIM of at least 0.85), plus exact card values;
- a future path to custom visuals (`.pbiviz`). Not in v1, and never executed in v1.

## Decision

Rendering lives entirely in `pbi-render` (Rust). The pipeline:

1. **Theme and layout.**
   - Resolve the base theme and custom theme, including `ThemeDataColor{ColorId, Percent}`.
   - Compute page scaling (`FitToPage`, `FitToWidth`, actual size), z-order and tab order.
   - Apply `visualContainerObjects`: background, border, title, padding, visualHeader.
2. **Semantic query → DataView.**
   - Each visual's `queryState` projections, plus report, page and visual filters, slicer state and cross-filter selections (`visualInteractions`), become a DAX query against `pbi-session`.
   - Report filters skip visuals whose tables have no relationship path to the filtered table, so a card over an unrelated table keeps its unfiltered value.
   - The result is a **PBI-shaped DataView** (`single`, `categorical`, `table`, `matrix`), modelled on the Power BI visuals API contract.
3. **Built-in visuals.** Rust implementations turn a DataView plus formatting `objects` into a **scene graph**: groups with transforms and clips, rects, paths and text runs with measured layout. Each node carries hit-test metadata (visual id, data-point identity).
4. **Output.**
   - **SVG** for the app webview. Hit-test metadata is emitted as `data-*` attributes.
   - **PNG** through `resvg`/`tiny-skia` for the CLI and golden tests.
   - Both come from the same scene graph.
5. **Text.** `cosmic-text` shapes and measures text using the bundled **Selawik** font (OFL-1.1, an open replacement for Segoe UI) and **Carlito** (OFL-1.1, metric-compatible with Calibri; added later for Calibri text). Headless rendering loads only bundled fonts, so output is the same on every OS.
6. **Interaction (app).**
   - The webview displays the SVG, maps clicks to visual or data-point ids, and sends them over typed IPC.
   - Rust updates the report state through `Document` commands ([ADR-0004](0004-desktop-ready-architecture.md)) and re-renders only the affected visuals.
7. **Service quirks** are encoded as rules with tests:
   - `cardVisual` draws an inner tile border unless `border`/`outline` are off.
   - Card labels clip below about 100px height.
   - Some font-size settings are ignored.
   - Slicer titles truncate with an ellipsis.
   - `tableEx` auto-sizes columns.
8. **Failure handling.** A visual that can't be queried or rendered becomes a placeholder box showing its type, its fields and the warning (for measures: a warning sign with the DAX text). Rendering never panics.
9. **Custom visuals (future).** Because the DataView is the contract, a later version could host `.pbiviz` packages in a sandboxed iframe with a visuals-API shim fed by the same DataViews. v1 reads `.pbiviz` metadata only and draws a placeholder.

## Alternatives considered

| Option | Why not chosen |
|---|---|
| Render in TypeScript (D3, ECharts, Vega-Lite) in the webview | Headless PNG would need a browser (headless Chromium in CI), query and formatting logic would be duplicated or moved into TS, and output would vary with each platform's webview. |
| Headless Chromium for PNG only | Heavy dependency and non-deterministic fonts. It also adds a second renderer to keep in sync. |
| Draw directly to a raster (`tiny-skia`) with no SVG | Deterministic, but the app would lose crisp zoom, DOM hit-testing and accessibility. SVG plus `resvg` gives both. |
| Generate Vega-Lite specs from visuals | Good for charts, but a poor fit for PBI-specific visuals (new card, slicers, textbox, shapes) and container chrome. It would also add a JS runtime for headless output. |

## Consequences

- One renderer for the CLI, CI and the app. Golden tests exercise exactly what users see.
- We implement each visual ourselves. Coverage grows by visual type, and unknown types fall back to placeholders.
- Text fidelity is limited by Selawik versus Segoe UI (Selawik lacks Segoe-matching kerning). Golden comparisons use per-region SSIM and layout boxes, not exact pixels.
- The `resvg`/`usvg` workspace features include `system-fonts` for the app, but headless and test rendering must load only bundled fonts to stay deterministic.
- The DataView contract is a stable seam: query-engine changes don't touch visuals, and a future custom-visual host can reuse it.
