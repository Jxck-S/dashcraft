# ADR-0006: Report interaction state lives outside the document

- Status: Accepted (supersedes the interaction step, §6, of [ADR-0005](0005-rust-svg-renderer.md))
- Date: 2026-10-22
- Deciders: pbiview maintainers
- Related: [ADR-0004](0004-desktop-ready-architecture.md), [ADR-0005](0005-rust-svg-renderer.md)

## Context

[ADR-0005](0005-rust-svg-renderer.md) §6 planned that the app would apply slicer selections and cross-filter clicks to the report through `Document` commands ([ADR-0004](0004-desktop-ready-architecture.md)) and re-render only the affected visuals.

Building the interactive canvas (`m4-interactive`) showed that:

- In Power BI's reading view, slicer selections, selected data points, scroll positions, open dropdowns and matrix expansion are **view state**. They don't make the report dirty, aren't undone with Ctrl+Z, and aren't saved unless the author saves the report or a bookmark.
- Routing every click and scroll through the command history would fill undo stacks with non-edits and mark a read-only document as modified.
- A slicer change can affect every other visual on the page (through `visualInteractions` and relationship propagation), so per-visual invalidation saves little. Re-querying a page is fast on the data sizes we target, while scrolls and dropdowns don't need a query at all.

## Decision

- Interaction state is a renderer type, `pbi_render::query::InteractionState` (slicer selections and selected data points), plus `pbi_render::visuals::PageViewState` (table and matrix scroll and expansion, open slicer dropdowns with their search text and scroll).
- The app back end (`app/src-tauri/src/canvas.rs`) keeps one of each per (report, page) of the open document, seeded from the selections saved in the report (`InteractionState::from_page`). **Reset to default** returns to that seed. Nothing is written to the `Document` or the file.
- A state change re-queries the whole page (`query_page_data`) on one engine snapshot. Results are cached per page and reused while the interaction state and the data are unchanged, so view-only changes (scroll, dropdowns, search) only re-render the scene. Each render bumps a generation and cancels the previous render's queries.
- The CLI uses the same types: `render` starts from the saved selections and replaces those of the slicers named with `--slicer`.
- Every render returns the effective filters (report + page filters + slicer selections + cross-filter) so other panes (Filters pane cards, the Measures panel) use exactly the state the visuals were queried with.

## Alternatives considered

| Option | Why not chosen |
|---|---|
| Selections as `Document` commands (ADR-0005 §6) | Clicks would dirty the document and fill undo history; Power BI does not treat them as edits. |
| Per-visual invalidation | Most state changes affect most visuals on the page; the bookkeeping would cost more than it saves at our data sizes. |
| State in the front end (TypeScript) | Duplicates query semantics outside Rust and breaks the "webview only displays" rule of [ADR-0001](0001-ui-tauri-svelte.md). |

## Consequences

- Selections are lost when the document closes; there is no "persist selections on save" yet. A future save or bookmark feature turns the current `InteractionState` into report edits through commands.
- The undo history stays reserved for real edits.
- Page re-queries are the unit of work. Very large models may need per-visual caching keyed on the filters each visual actually depends on.
- ADR-0005's other decisions (Rust visuals, scene graph, SVG for the app, PNG through `resvg`, bundled fonts, DataView contract) stand. One implementation detail also differs from ADR-0005 §8: a visual whose query fails draws ⚠ "Can't display this visual" (with the message and DAX in the app's tooltip); only unknown visual types draw the type-and-fields placeholder.
