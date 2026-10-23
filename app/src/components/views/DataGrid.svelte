<script module lang="ts">
  import type { GridColumn } from '../../lib/backend'

  /** What the grid knows about the current query, reported to the parent. */
  export interface GridInfo {
    columns: GridColumn[]
    totalRows: number
    loading: boolean
    error: string | null
  }
</script>

<script lang="ts">
  import { untrack } from 'svelte'
  import Icon from '../Icon.svelte'
  import * as backend from '../../lib/backend'
  import type { Cell, Sort } from '../../lib/backend'
  import { columnTooltip } from '../../lib/data'

  // Virtualized grid: only the visible rows are in the DOM, and rows arrive from the backend in
  // pages of PAGE_SIZE (sorted and filtered there). Very tall tables scale the scrollbar so the
  // spacer stays under browser height limits.
  let {
    table,
    sort,
    filter,
    version,
    selectedColumn,
    onsort,
    onselectcolumn,
    oncolumnmenu,
    oninfo,
  }: {
    table: string
    sort: Sort | null
    /** Applied SQL filter expression ('' = none). */
    filter: string
    /** Bumped when the table's data changes (refresh, CSV load, derived column). */
    version: number
    selectedColumn: string | null
    onsort: (column: string) => void
    onselectcolumn: (column: GridColumn) => void
    oncolumnmenu: (column: GridColumn, x: number, y: number) => void
    oninfo: (info: GridInfo) => void
  } = $props()

  const ROW_HEIGHT = 24
  const HEADER_HEIGHT = 30
  const PAGE_SIZE = 200
  const MAX_CACHED_PAGES = 30
  const MAX_SCROLL_PX = 8_000_000

  let scroller = $state<HTMLDivElement | null>(null)
  let scrollTop = $state(0)
  let viewportHeight = $state(0)
  let columns = $state.raw<GridColumn[]>([])
  let total = $state(0)
  let pages = $state.raw(new Map<number, Cell[][]>())
  let inflight = $state(0)
  let error = $state<string | null>(null)
  /** Bumped on every reset; responses of an older epoch are dropped. */
  let epoch = $state(0)
  let widths = $state<Record<string, number>>({})

  let requested = new Set<number>()
  /** End of the last column resize: the click that ends a drag must not sort. */
  let resizedAt = 0
  let lastTable = ''
  let lastQuery = ''

  const query = $derived(JSON.stringify([table, sort?.column ?? null, sort?.ascending ?? null, filter]))
  const bodyHeight = $derived(Math.max(0, viewportHeight - HEADER_HEIGHT))
  const fullHeight = $derived(total * ROW_HEIGHT)
  const contentHeight = $derived(Math.min(fullHeight, MAX_SCROLL_PX))
  const virtualTop = $derived(
    fullHeight > MAX_SCROLL_PX
      ? (scrollTop * Math.max(0, fullHeight - bodyHeight)) / Math.max(1, contentHeight - bodyHeight)
      : scrollTop,
  )
  const firstRow = $derived(Math.max(0, Math.min(total - 1, Math.floor(virtualTop / ROW_HEIGHT))))
  const lastRow = $derived(Math.min(total, firstRow + Math.ceil(bodyHeight / ROW_HEIGHT) + 2))
  const windowTop = $derived(scrollTop - (virtualTop - firstRow * ROW_HEIGHT))
  const visibleRows = $derived(Array.from({ length: Math.max(0, lastRow - firstRow) }, (_, i) => firstRow + i))
  const rowNumberWidth = $derived(Math.max(44, String(total).length * 8 + 18))
  const totalWidth = $derived(rowNumberWidth + columns.reduce((sum, c) => sum + widthOf(c), 0))

  function widthOf(c: GridColumn): number {
    const min = c.dataType === 'dateTime' ? 150 : c.numeric ? 90 : 110
    const badge = c.kind === 'calculated' || c.kind === 'derived' ? 22 : 0
    return widths[c.name] ?? Math.min(300, Math.max(min, Math.round(c.name.length * 7.5) + 48 + badge))
  }

  function rowAt(row: number): Cell[] | undefined {
    return pages.get(Math.floor(row / PAGE_SIZE))?.[row % PAGE_SIZE]
  }

  // Reset the cache when the query or the data changes; a new table/sort/filter also scrolls to
  // the top, a data change keeps the position.
  $effect(() => {
    const q = query
    void version
    untrack(() => {
      const queryChanged = q !== lastQuery
      if (table !== lastTable) {
        columns = []
        total = 0
      }
      lastTable = table
      lastQuery = q
      requested = new Set()
      pages = new Map()
      error = null
      epoch += 1
      if (queryChanged && scroller) {
        scroller.scrollTop = 0
        scrollTop = 0
      }
    })
  })

  // Fetch the pages covering the visible rows (page 0 first: it brings the row count).
  $effect(() => {
    const e = epoch
    if (!table) return
    const from = Math.floor(firstRow / PAGE_SIZE)
    const to = total === 0 ? 0 : Math.floor(Math.max(firstRow, lastRow - 1) / PAGE_SIZE)
    untrack(() => {
      for (let p = from; p <= to; p++) void fetchPage(p, e)
    })
  })

  $effect(() => {
    const info: GridInfo = { columns, totalRows: total, loading: inflight > 0, error }
    untrack(() => oninfo(info))
  })

  async function fetchPage(page: number, e: number) {
    if (pages.has(page) || requested.has(page)) return
    requested.add(page)
    inflight += 1
    try {
      const result = await backend.tablePage(table, page * PAGE_SIZE, PAGE_SIZE, sort, filter.trim() || null)
      if (e !== epoch) return
      if (!result.ok) {
        requested.delete(page)
        error = result.error
        return
      }
      const value = result.value
      if (value.columns.length !== columns.length || value.columns.some((c, i) => c.name !== columns[i]?.name || c.kind !== columns[i]?.kind)) {
        columns = value.columns
      }
      total = value.totalRows
      error = null
      const next = new Map(pages)
      next.set(page, value.rows)
      // Evict the pages farthest from the visible one.
      const current = Math.floor(firstRow / PAGE_SIZE)
      const byDistance = [...next.keys()].sort((a, b) => Math.abs(b - current) - Math.abs(a - current))
      for (const p of byDistance.slice(0, Math.max(0, next.size - MAX_CACHED_PAGES))) {
        next.delete(p)
        requested.delete(p)
      }
      pages = next
    } finally {
      inflight -= 1
    }
  }

  function onscroll() {
    if (scroller) scrollTop = scroller.scrollTop
  }

  function startResize(event: PointerEvent, c: GridColumn) {
    event.preventDefault()
    event.stopPropagation()
    const handle = event.currentTarget as HTMLElement
    handle.setPointerCapture(event.pointerId)
    const startX = event.clientX
    const start = widthOf(c)
    const move = (e: PointerEvent) => {
      widths = { ...widths, [c.name]: Math.max(48, Math.round(start + e.clientX - startX)) }
    }
    const up = () => {
      resizedAt = performance.now()
      handle.removeEventListener('pointermove', move)
      handle.removeEventListener('pointerup', up)
    }
    handle.addEventListener('pointermove', move)
    handle.addEventListener('pointerup', up)
  }

  function headerKey(event: KeyboardEvent, c: GridColumn) {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      onsort(c.name)
    } else if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) {
      event.preventDefault()
      const r = (event.currentTarget as HTMLElement).getBoundingClientRect()
      oncolumnmenu(c, r.left, r.bottom)
    }
  }
</script>

<div class="scroller" bind:this={scroller} bind:clientHeight={viewportHeight} {onscroll} role="grid" aria-rowcount={total} aria-colcount={columns.length}>
  <div class="header" style:width={`${totalWidth}px`} style:height={`${HEADER_HEIGHT}px`} role="row">
    <div class="corner rownum" style:width={`${rowNumberWidth}px`} role="columnheader">#</div>
    {#each columns as c (c.name)}
      {@const sorted = sort?.column === c.name ? (sort.ascending ? 'ascending' : 'descending') : 'none'}
      <div
        class="th"
        class:num={c.numeric}
        class:derived={c.kind === 'derived'}
        class:hidden-col={c.hidden}
        class:selected-col={selectedColumn === c.name}
        style:width={`${widthOf(c)}px`}
        role="columnheader"
        aria-sort={sorted}
        tabindex="0"
        title={columnTooltip(c)}
        onclick={() => performance.now() - resizedAt > 250 && onsort(c.name)}
        onkeydown={(e) => headerKey(e, c)}
        oncontextmenu={(e) => {
          e.preventDefault()
          oncolumnmenu(c, e.clientX, e.clientY)
        }}
      >
        <span class="th-name">{c.name}</span>
        {#if c.kind === 'calculated'}<span class="badge" title="Calculated column (DAX)">fx</span>{/if}
        {#if c.kind === 'derived'}<span class="badge" title="Derived column (session only)">⇢</span>{/if}
        {#if sorted !== 'none'}<Icon name={sorted === 'ascending' ? 'sortAsc' : 'sortDesc'} size={12} />{/if}
        <button
          class="menu-btn"
          title="Column actions"
          aria-label={`Actions for ${c.name}`}
          onclick={(e) => {
            e.stopPropagation()
            const r = (e.currentTarget as HTMLElement).getBoundingClientRect()
            oncolumnmenu(c, r.left, r.bottom)
          }}
        >
          <Icon name="chevronDown" size={12} />
        </button>
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <span class="resize" aria-hidden="true" title="Drag to resize" onpointerdown={(e) => startResize(e, c)}></span>
      </div>
    {/each}
  </div>
  <div class="body" style:height={`${contentHeight}px`} style:width={`${totalWidth}px`}>
    <div class="window" style:transform={`translateY(${windowTop}px)`}>
      {#each visibleRows as r (r)}
        {@const row = rowAt(r)}
        <div class="row" class:odd={r % 2 === 1} style:height={`${ROW_HEIGHT}px`} role="row" aria-rowindex={r + 1}>
          <div class="rownum" style:width={`${rowNumberWidth}px`}>{(r + 1).toLocaleString()}</div>
          {#each columns as c, i (c.name)}
            {@const cell = row?.[i]}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <div
              class="td"
              class:num={c.numeric}
              class:blank={cell?.isBlank}
              class:pending={!row}
              class:selected-col={selectedColumn === c.name}
              style:width={`${widthOf(c)}px`}
              role="gridcell"
              tabindex="-1"
              title={cell && !cell.isBlank && cell.text.length > 24 ? cell.text : undefined}
              onclick={() => onselectcolumn(c)}
            >
              {#if cell}{cell.isBlank ? '(Blank)' : cell.text}{/if}
            </div>
          {/each}
        </div>
      {/each}
    </div>
  </div>
</div>

<style>
  .scroller {
    position: relative;
    height: 100%;
    overflow: auto;
    font-size: 12px;
    background: var(--color-pane);
  }

  .header {
    position: sticky;
    top: 0;
    z-index: 2;
    display: flex;
    background: var(--color-chrome);
    border-bottom: 1px solid var(--color-border-strong);
  }

  .th,
  .corner {
    position: relative;
    display: flex;
    flex: none;
    align-items: center;
    gap: 4px;
    padding: 0 6px 0 8px;
    border-right: 1px solid var(--color-border);
    font-weight: 600;
    white-space: nowrap;
    user-select: none;
  }

  .th {
    cursor: pointer;
  }

  .th:hover {
    background: var(--color-hover);
  }

  .th:focus-visible {
    outline: 1px solid var(--color-brand);
    outline-offset: -1px;
  }

  .th.num {
    justify-content: flex-end;
  }

  .th-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .th.num .th-name {
    order: 1;
  }

  .th.num .menu-btn {
    order: -1;
    margin-left: 0;
    margin-right: auto;
  }

  .th.hidden-col .th-name {
    color: var(--color-text-subtle);
  }

  .th.derived {
    background: var(--color-info-bg);
  }

  .th.selected-col {
    background: var(--color-brand);
    color: #fff;
  }

  .th.selected-col .menu-btn {
    background: var(--color-pane);
  }

  .badge {
    flex: none;
    padding: 0 3px;
    border-radius: 2px;
    background: var(--color-pane);
    color: var(--color-brand);
    font-family: var(--font-mono);
    font-size: 10px;
    font-style: italic;
    font-weight: 400;
  }

  .menu-btn {
    display: grid;
    flex: none;
    place-items: center;
    width: 18px;
    height: 18px;
    margin-left: auto;
    padding: 0;
    border: none;
    border-radius: 2px;
    background: transparent;
    color: var(--color-text-muted);
    box-shadow: inset 0 0 0 1px var(--color-border-strong);
    cursor: pointer;
  }

  .menu-btn:hover {
    background: var(--color-selected);
  }

  .resize {
    position: absolute;
    top: 0;
    right: -3px;
    z-index: 1;
    width: 6px;
    height: 100%;
    cursor: col-resize;
  }

  .rownum {
    position: sticky;
    left: 0;
    z-index: 1;
    flex: none;
    padding: 0 6px;
    border-right: 1px solid var(--color-border-strong);
    background: var(--color-chrome);
    color: var(--color-text-subtle);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }

  .corner {
    z-index: 3;
    justify-content: flex-end;
    color: var(--color-text-subtle);
  }

  .body {
    position: relative;
    overflow: clip;
  }

  .window {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    will-change: transform;
  }

  .row {
    display: flex;
    align-items: stretch;
    border-bottom: 1px solid var(--color-border);
  }

  .row.odd {
    background: #fafafa;
  }

  .row:hover {
    background: var(--color-hover);
  }

  .row .rownum {
    display: flex;
    align-items: center;
    justify-content: flex-end;
  }

  .td {
    flex: none;
    padding: 0 8px;
    overflow: hidden;
    border-right: 1px solid var(--color-border);
    line-height: 23px;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: default;
  }

  .td.num {
    font-variant-numeric: tabular-nums;
    text-align: right;
  }

  .td.blank {
    color: var(--color-text-subtle);
    font-style: italic;
    background: rgb(0 0 0 / 0.025);
  }

  .td.selected-col {
    background: #ebebeb;
  }

  .td.pending {
    background: linear-gradient(90deg, transparent 8px, var(--color-chrome) 8px, var(--color-chrome) 60%, transparent 60%) no-repeat center / 100% 8px;
  }
</style>
