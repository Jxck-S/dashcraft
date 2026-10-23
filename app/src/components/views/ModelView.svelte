<script lang="ts">
  import { untrack } from 'svelte'
  import Icon, { type IconName } from '../Icon.svelte'
  import type { ModelDiagramDto, ModelTreeDto, RelationshipDto, TableDto } from '../../lib/bindings'
  import { CARDINALITY, CROSS_FILTER, daxColumnRef, nameTooltip, type SelectedId } from '../../lib/model'
  import { NOT_AVAILABLE } from '../../lib/shell'

  // Model view: the diagram of table cards and relationship lines (cardinality at both ends,
  // cross-filter direction arrows in the middle, inactive dashed). Positions come from the
  // model's diagramLayout.json when it has one, else from a layered auto-layout (fact tables
  // left, the tables they look up to the right). Drag the background to pan, Ctrl/⌘ + wheel to
  // zoom; the layout tabs (All tables + saved layouts) are at the bottom.
  let {
    tree,
    diagram,
    selected,
    onselect,
    zoom,
    onzoom,
    fitToken,
  }: {
    tree: ModelTreeDto | null
    diagram: ModelDiagramDto | null
    selected: SelectedId
    onselect: (id: number) => void
    /** Percent (100 = 100%). */
    zoom: number
    onzoom: (percent: number) => void
    /** Bumped by the status bar's "Fit to screen". */
    fitToken: number
  } = $props()

  const CARD_W = 220
  const HEADER_H = 34
  const ROW_H = 22
  const FOOTER_H = 22
  const MAX_ROWS = 12

  let layoutName = $state<string | null>(null)
  let collapsed = $state<Record<number, boolean>>({})
  let pan = $state({ x: 0, y: 0 })
  let viewport = $state<HTMLDivElement | null>(null)
  let vw = $state(0)
  let vh = $state(0)

  const layouts = $derived(diagram?.diagrams ?? [])
  const layout = $derived(layouts.find((d) => d.name === layoutName) ?? layouts.find((d) => d.name === diagram?.defaultDiagram) ?? layouts[0] ?? null)
  const tables = $derived(tree?.tables ?? [])
  const scale = $derived(zoom / 100)

  interface Card {
    table: TableDto
    x: number
    y: number
    w: number
    h: number
    rows: Array<{ id: number; name: string; icon: IconName | null; hidden: boolean; key: boolean; measure: boolean }>
    collapsed: boolean
  }

  /** Columns used by a relationship, per table (lower-cased). */
  const keys = $derived.by(() => {
    const k = new Map<string, Set<string>>()
    for (const r of tree?.relationships ?? []) {
      for (const [t, c] of [[r.fromTable, r.fromColumn], [r.toTable, r.toColumn]]) {
        const set = k.get(t.toLowerCase()) ?? new Set<string>()
        set.add(c.toLowerCase())
        k.set(t.toLowerCase(), set)
      }
    }
    return k
  })

  function rowsOf(t: TableDto): Card['rows'] {
    const k = keys.get(t.name.toLowerCase())
    const cols = t.columns
      .filter((c) => c.kind !== 'rowNumber')
      .map((c) => ({
        id: c.id,
        name: c.name,
        icon: (c.kind === 'calculated' ? 'calcColumn' : c.dataType === 'dateTime' ? 'date' : ['int64', 'double', 'decimal'].includes(c.dataType) ? 'numeric' : null) as IconName | null,
        hidden: c.hidden,
        key: k?.has(c.name.toLowerCase()) ?? false,
        measure: false,
      }))
    const measures = t.measures.map((m) => ({ id: m.id, name: m.name, icon: 'measure' as IconName, hidden: m.hidden, key: false, measure: true }))
    const byName = (a: { name: string }, b: { name: string }) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base', numeric: true })
    return [...cols.sort(byName), ...measures.sort(byName)]
  }

  /** Layered auto-layout: rank = longest chain of many → one relationships leading to a table. */
  function autoLayout(list: TableDto[]): Map<string, { x: number; y: number }> {
    const rank = new Map<string, number>(list.map((t) => [t.name.toLowerCase(), 0]))
    const edges = (tree?.relationships ?? []).map((r) =>
      r.cardinality === 'oneToMany' ? [r.toTable.toLowerCase(), r.fromTable.toLowerCase()] : [r.fromTable.toLowerCase(), r.toTable.toLowerCase()],
    )
    for (let i = 0; i < list.length; i++) {
      let changed = false
      for (const [from, to] of edges) {
        const next = (rank.get(from) ?? 0) + 1
        if (rank.has(to) && next > (rank.get(to) ?? 0) && next < list.length) {
          rank.set(to, next)
          changed = true
        }
      }
      if (!changed) break
    }
    const linked = new Set(edges.flat())
    const columns = new Map<number, TableDto[]>()
    const loose: TableDto[] = []
    for (const t of list) {
      if (!linked.has(t.name.toLowerCase())) loose.push(t)
      else {
        const r = rank.get(t.name.toLowerCase()) ?? 0
        columns.set(r, [...(columns.get(r) ?? []), t])
      }
    }
    const pos = new Map<string, { x: number; y: number }>()
    const height = (t: TableDto) => HEADER_H + Math.min(MAX_ROWS, rowsOf(t).length) * ROW_H + FOOTER_H
    let maxRank = -1
    for (const [r, ts] of [...columns.entries()].sort((a, b) => a[0] - b[0])) {
      let y = 40
      for (const t of ts) {
        pos.set(t.name, { x: 40 + r * (CARD_W + 140), y })
        y += height(t) + 48
      }
      maxRank = Math.max(maxRank, r)
    }
    const startX = 40 + (maxRank + 1) * (CARD_W + 140)
    loose.forEach((t, i) => {
      const col = i % 3
      const row = Math.floor(i / 3)
      pos.set(t.name, { x: startX + col * (CARD_W + 48), y: 40 + row * (HEADER_H + MAX_ROWS * ROW_H + FOOTER_H + 48) })
    })
    return pos
  }

  const cards = $derived.by((): Card[] => {
    const fromLayout = new Map((layout?.nodes ?? []).map((n) => [n.table.toLowerCase(), n]))
    // The main layout ("All tables") shows every table, placing new ones to the right of the
    // saved ones; other saved layouts show only their tables.
    const isMain = !layout || layout === layouts[0] || layout.name.toLowerCase() === 'all tables'
    const placed = tables.filter((t) => fromLayout.has(t.name.toLowerCase()))
    const unplaced = tables.filter((t) => !fromLayout.has(t.name.toLowerCase()))
    const list = isMain || placed.length === 0 ? tables : placed
    const offsetX = placed.length > 0 ? Math.max(...(layout?.nodes ?? []).map((n) => n.x + Math.max(n.width, CARD_W))) + 80 : 0
    const auto = new Map([...autoLayout(placed.length > 0 ? unplaced : tables)].map(([k, p]) => [k, { x: p.x + offsetX, y: p.y }]))
    return list.map((t) => {
      const node = fromLayout.get(t.name.toLowerCase())
      const rows = rowsOf(t)
      const isCollapsed = collapsed[t.id] ?? node?.collapsed ?? false
      const natural = HEADER_H + Math.min(MAX_ROWS, rows.length) * ROW_H + FOOTER_H
      const p = node ? { x: node.x, y: node.y } : (auto.get(t.name) ?? { x: 40, y: 40 })
      return {
        table: t,
        x: p.x,
        y: p.y,
        w: node && node.width > 80 ? node.width : CARD_W,
        h: isCollapsed ? HEADER_H + FOOTER_H : node && node.height > HEADER_H ? Math.max(node.height, HEADER_H + FOOTER_H) : natural,
        rows,
        collapsed: isCollapsed,
      }
    })
  })

  const byName = $derived(new Map(cards.map((c) => [c.table.name.toLowerCase(), c])))
  const extent = $derived({
    w: Math.max(400, ...cards.map((c) => c.x + c.w + 60)),
    h: Math.max(300, ...cards.map((c) => c.y + c.h + 60)),
  })

  /** y of a column's row in a card (its header when collapsed or scrolled out of the visible rows). */
  function anchorY(card: Card, column: string): number {
    if (card.collapsed) return card.y + HEADER_H / 2
    const i = card.rows.findIndex((r) => !r.measure && r.name.toLowerCase() === column.toLowerCase())
    if (i < 0 || i >= MAX_ROWS) return card.y + HEADER_H / 2
    return card.y + HEADER_H + i * ROW_H + ROW_H / 2
  }

  interface Line {
    r: RelationshipDto
    path: string
    from: { x: number; y: number }
    to: { x: number; y: number }
    mid: { x: number; y: number }
    fromLeft: boolean
    toLeft: boolean
  }

  const lines = $derived.by((): Line[] => {
    const out: Line[] = []
    for (const r of tree?.relationships ?? []) {
      const a = byName.get(r.fromTable.toLowerCase())
      const b = byName.get(r.toTable.toLowerCase())
      if (!a || !b) continue
      const ay = anchorY(a, r.fromColumn)
      const by = anchorY(b, r.toColumn)
      let ax: number, bx: number, fromLeft: boolean, toLeft: boolean
      if (a.x + a.w < b.x) {
        ;[ax, bx, fromLeft, toLeft] = [a.x + a.w, b.x, false, true]
      } else if (b.x + b.w < a.x) {
        ;[ax, bx, fromLeft, toLeft] = [a.x, b.x + b.w, true, false]
      } else {
        ;[ax, bx, fromLeft, toLeft] = [a.x + a.w, b.x + b.w, false, false]
      }
      const midX = fromLeft === toLeft ? Math.max(ax, bx) + 40 : (ax + bx) / 2
      out.push({
        r,
        path: `M ${ax} ${ay} H ${midX} V ${by} H ${bx}`,
        from: { x: ax, y: ay },
        to: { x: bx, y: by },
        mid: { x: midX, y: (ay + by) / 2 },
        fromLeft,
        toLeft,
      })
    }
    return out
  })

  function ends(r: RelationshipDto) {
    const c = CARDINALITY[r.cardinality].short.split(':')
    return { from: c[0] ?? '?', to: c[1] ?? '?' }
  }

  function lineTitle(r: RelationshipDto): string {
    const dir = r.crossFilter === 'both' ? `${r.fromTable} and ${r.toTable} filter each other` : `${r.toTable} filters ${r.fromTable}`
    return `${daxColumnRef(r.fromTable, r.fromColumn)} ${CARDINALITY[r.cardinality].short} ${daxColumnRef(r.toTable, r.toColumn)}\n${CARDINALITY[r.cardinality].long} · ${CROSS_FILTER[r.crossFilter]} (${dir})${r.active ? '' : '\nInactive'}`
  }

  // Panning: drag the background.
  let drag = $state<{ x: number; y: number; px: number; py: number } | null>(null)
  function pointerdown(event: PointerEvent) {
    if ((event.target as Element).closest('.card, .rel-hit') || event.button !== 0) return
    drag = { x: event.clientX, y: event.clientY, px: pan.x, py: pan.y }
    ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
  }
  function pointermove(event: PointerEvent) {
    if (!drag) return
    pan = { x: drag.px + event.clientX - drag.x, y: drag.py + event.clientY - drag.y }
  }
  function pointerup() {
    drag = null
  }

  $effect(() => {
    const el = viewport
    if (!el) return
    const wheel = (event: WheelEvent) => {
      event.preventDefault()
      if (event.ctrlKey || event.metaKey) {
        const factor = event.deltaY < 0 ? 1.1 : 1 / 1.1
        onzoom(Math.max(10, Math.min(400, zoom * factor)))
      } else {
        pan = { x: pan.x - event.deltaX, y: pan.y - event.deltaY }
      }
    }
    el.addEventListener('wheel', wheel, { passive: false })
    return () => el.removeEventListener('wheel', wheel)
  })

  function fit() {
    if (vw <= 0 || vh <= 0 || cards.length === 0) return
    const minX = Math.min(...cards.map((c) => c.x)) - 20
    const minY = Math.min(...cards.map((c) => c.y)) - 20
    const w = extent.w - minX
    const h = extent.h - minY
    const s = Math.max(0.1, Math.min(1.5, vw / w, vh / h))
    onzoom(Math.round(s * 100))
    pan = { x: (vw - w * s) / 2 - minX * s, y: (vh - h * s) / 2 - minY * s }
  }

  // Fit on first show, on layout change and on request.
  let fittedFor = ''
  $effect(() => {
    const key = `${layout?.name ?? ''}|${fitToken}|${tables.length}`
    if (vw > 0 && key !== fittedFor) {
      untrack(() => {
        fittedFor = key
        if (layout && fitToken === 0) {
          onzoom(Math.round(layout.zoom))
          pan = { x: 0, y: 0 }
        } else fit()
      })
    }
  })
</script>

<div class="model-view">
  {#if !tree || tree.status.availability !== 'local'}
    <p class="empty-hint">{tree?.status.availability === 'remote' ? 'Remote model — not available offline.' : 'No semantic model loaded.'}</p>
  {:else}
    <div
      class="viewport"
      class:panning={drag !== null}
      bind:this={viewport}
      bind:clientWidth={vw}
      bind:clientHeight={vh}
      onpointerdown={pointerdown}
      onpointermove={pointermove}
      onpointerup={pointerup}
      role="application"
      aria-label="Model diagram"
    >
      <div class="world" style:transform={`translate(${pan.x}px, ${pan.y}px) scale(${scale})`} style:width={`${extent.w}px`} style:height={`${extent.h}px`}>
        <svg class="lines" width={extent.w} height={extent.h} aria-hidden="false">
          {#each lines as l (l.r.id)}
            {@const e = ends(l.r)}
            {@const sel = selected === l.r.id}
            <g class="rel" class:inactive={!l.r.active} class:selected={sel}>
              <path class="rel-line" d={l.path} />
              <path
                class="rel-hit"
                d={l.path}
                role="button"
                tabindex="0"
                aria-label={lineTitle(l.r)}
                onclick={() => onselect(l.r.id)}
                onkeydown={(ev) => (ev.key === 'Enter' || ev.key === ' ') && onselect(l.r.id)}
              >
                <title>{lineTitle(l.r)}</title>
              </path>
              <text class="end" x={l.from.x + (l.fromLeft ? -12 : 8)} y={l.from.y - 5}>{e.from}</text>
              <text class="end" x={l.to.x + (l.toLeft ? -12 : 8)} y={l.to.y - 5}>{e.to}</text>
              <g transform={`translate(${l.mid.x} ${l.mid.y})`} class="arrows">
                <rect x="-9" y="-12" width="18" height="24" rx="3" />
                {#if l.r.crossFilter === 'both'}
                  <path d="M -4 -3 L 0 -8 L 4 -3 M -4 3 L 0 8 L 4 3" />
                {:else}
                  <path d={l.from.y > l.to.y ? 'M -4 -1 L 0 4 L 4 -1' : 'M -4 1 L 0 -4 L 4 1'} transform={l.from.y > l.to.y ? 'translate(0 2)' : 'translate(0 -2)'} />
                {/if}
              </g>
            </g>
          {/each}
        </svg>
        {#each cards as c (c.table.id)}
          <div class="card" class:selected={selected === c.table.id} class:hidden-table={c.table.hidden} style:left={`${c.x}px`} style:top={`${c.y}px`} style:width={`${c.w}px`} style:height={`${c.h}px`}>
            <button class="card-header" onclick={() => onselect(c.table.id)} title={nameTooltip(c.table.name, null, c.table.description)}>
              <Icon name={c.table.kind === 'calculatedTable' ? 'calcTable' : c.table.kind === 'remote' ? 'remote' : 'table'} size={16} />
              <span class="name">{c.table.name}</span>
              {#if c.table.hidden}<Icon name="hidden" size={14} title="Hidden" />{:else}<Icon name="eye" size={14} />{/if}
            </button>
            {#if !c.collapsed}
              <ul class="fields">
                {#each c.rows as row (row.id)}
                  <li>
                    <button class:selected={selected === row.id} class:hidden-row={row.hidden} onclick={() => onselect(row.id)} title={row.name}>
                      <span class="ficon">{#if row.key}<Icon name="key" size={12} />{:else if row.icon}<Icon name={row.icon} size={14} />{/if}</span>
                      <span class="name">{row.name}</span>
                      {#if row.hidden}<Icon name="hidden" size={12} />{/if}
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
            <button class="collapse" onclick={() => (collapsed[c.table.id] = !c.collapsed)}>
              {c.collapsed ? 'Expand' : 'Collapse'}
              <Icon name={c.collapsed ? 'chevronDown' : 'chevronUp'} size={10} />
            </button>
          </div>
        {/each}
      </div>
    </div>
    <nav class="layout-tabs" aria-label="Layouts">
      {#each layouts as d (d.name)}
        <button class:active={layout?.name === d.name} onclick={() => (layoutName = d.name)} title={`Saved layout (diagramLayout.json): ${d.nodes.length} tables`}>{d.name}</button>
      {:else}
        <button class="active" title="Every table, arranged automatically (the model has no diagramLayout.json)">All tables</button>
      {/each}
      <button class="add" disabled title={`New layout — ${NOT_AVAILABLE}`} aria-label="New layout"><Icon name="add" size={16} /></button>
    </nav>
  {/if}
</div>

<style>
  .model-view {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--color-canvas);
  }

  .viewport {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: hidden;
    background-color: #f3f3f3;
    background-image: radial-gradient(#d6d6d6 1px, transparent 1px);
    background-size: 16px 16px;
    cursor: grab;
    touch-action: none;
  }

  .viewport.panning {
    cursor: grabbing;
  }

  .world {
    position: absolute;
    top: 0;
    left: 0;
    transform-origin: 0 0;
  }

  .lines {
    position: absolute;
    top: 0;
    left: 0;
    overflow: visible;
  }

  .rel-line {
    fill: none;
    stroke: #8a8a8a;
    stroke-width: 1.5;
  }

  .rel.inactive .rel-line {
    stroke-dasharray: 6 4;
  }

  .rel.selected .rel-line {
    stroke: var(--color-brand);
    stroke-width: 2.5;
  }

  .rel-hit {
    fill: none;
    stroke: transparent;
    stroke-width: 12;
    cursor: pointer;
    pointer-events: stroke;
  }

  .rel-hit:focus-visible {
    outline: none;
    stroke: rgb(17 120 101 / 0.25);
  }

  .end {
    fill: var(--color-text);
    font-size: 12px;
    font-weight: 700;
  }

  .arrows rect {
    fill: #fff;
    stroke: #8a8a8a;
  }

  .arrows path {
    fill: none;
    stroke: var(--color-text);
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .card {
    position: absolute;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--color-border-strong);
    border-radius: 4px;
    background: var(--color-pane);
    box-shadow: 0 1.6px 3.6px rgb(0 0 0 / 0.13), 0 0.3px 0.9px rgb(0 0 0 / 0.11);
    cursor: default;
  }

  .card.selected {
    border-color: var(--color-brand);
    box-shadow: 0 0 0 1px var(--color-brand), 0 2px 6px rgb(0 0 0 / 0.16);
  }

  .hidden-table {
    opacity: 0.75;
  }

  .card-header {
    display: flex;
    flex: none;
    align-items: center;
    gap: 6px;
    height: 34px;
    padding: 0 10px;
    border: none;
    border-bottom: 1px solid var(--color-border);
    border-radius: 4px 4px 0 0;
    background: transparent;
    color: var(--color-text);
    font-weight: 600;
    text-align: left;
    cursor: pointer;
  }

  .card-header :global(svg:last-child) {
    color: var(--color-text-muted);
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .fields {
    flex: 1;
    min-height: 0;
    margin: 0;
    padding: 0;
    overflow: auto;
    list-style: none;
  }

  .fields button {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    height: 22px;
    padding: 0 10px;
    border: none;
    background: transparent;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
  }

  .fields button:hover {
    background: var(--color-hover);
  }

  .fields button.selected {
    background: var(--color-selected);
  }

  .hidden-row {
    color: var(--color-text-subtle);
  }

  .ficon {
    display: inline-flex;
    flex: none;
    width: 14px;
    color: var(--color-text-muted);
  }

  .collapse {
    display: flex;
    flex: none;
    align-items: center;
    gap: 3px;
    height: 22px;
    padding: 0 10px;
    border: none;
    border-top: 1px solid var(--color-border);
    background: transparent;
    color: var(--color-brand);
    font-size: 11px;
    cursor: pointer;
  }

  .layout-tabs {
    display: flex;
    flex: none;
    height: var(--pagetabs-height);
    border-top: 1px solid var(--color-border);
    background: var(--color-pagetabs);
  }

  .layout-tabs button {
    position: relative;
    padding: 0 18px;
    border: none;
    border-right: 1px solid var(--color-border);
    background: transparent;
    font-size: 13px;
    cursor: pointer;
  }

  .layout-tabs button.active {
    background: var(--color-pane);
    font-weight: 600;
  }

  .layout-tabs button.active::after {
    content: '';
    position: absolute;
    right: 0;
    bottom: 0;
    left: 0;
    height: 3px;
    background: var(--color-brand);
  }

  .layout-tabs .add {
    width: 40px;
    padding: 0;
    color: var(--color-text-disabled);
    cursor: default;
  }
</style>
