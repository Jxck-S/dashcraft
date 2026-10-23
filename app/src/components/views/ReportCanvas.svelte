<script lang="ts">
  import { tick, untrack } from 'svelte'
  import Icon from '../Icon.svelte'
  import * as backend from '../../lib/backend'
  import type { CanvasAction, FilterSpec, PageDto, RenderedPage, ReportDto, TooltipLine, VisualDto } from '../../lib/backend'
  import type { CanvasInteractionOut } from '../../lib/ops'
  import { contains, parseHit, sanitizeSvg, scrollableAt, wheelPixels, type Hit } from '../../lib/canvas'
  import { visualKind } from '../../lib/visuals'

  /*
   * ReportCanvas — the report page surface of the Report view: the page rendered by the Rust
   * renderer (pbi-render → SVG, the registry's `canvas.render`), made interactive like Power BI's
   * reading view; every interaction is a `canvas.*` registry command (shared with the CLI and
   * agents).
   *
   * - The SVG is parsed with DOMParser and sanitized (no scripts / handlers / external links,
   *   ids prefixed) before insertion, then scaled to `zoom` by the page box (vector, so crisp).
   * - Hit-testing uses the SVG's `data-hit` groups: slicer items select (Ctrl+click toggles),
   *   slicer dropdown boxes open their popup (drawn in an overlay layer above the page, not
   *   clipped to it), data points cross-filter the other visuals (click again or the empty
   *   canvas to clear), action buttons report their action (`onaction`).
   * - Wheel over a table, matrix, slicer list or open popup scrolls it; hovering data points
   *   shows a tooltip; Esc closes popups, then clears the cross-filter.
   * - Every render is reported with `onrendered` (selections and effective filters for the
   *   Filters pane and the Measures panel); `incoming` injects a render made elsewhere (Reset
   *   to default, Filters pane clear buttons).
   * - Update model: the newest render wins. Responses older than the shown one (by the page's
   *   render generation) are dropped, so out-of-order completions never move the page back;
   *   the SVG is parsed and swapped at most once per animation frame (renders arriving faster
   *   than the display refreshes are skipped) and only when its text changed. Wheel deltas
   *   are coalesced per visual with one scroll request in flight. The backend re-renders only
   *   the visuals whose inputs changed (pbi-render's visual cache); the page SVG is still
   *   replaced whole (see app/README.md, "Canvas update model").
   *
   * Props: `report`, `page` (the page to draw); `zoom` (1 = 100%, page units → CSS px);
   * `filters` (report + page filters, informational); `dataVersion` (bumped when table data
   * changes: re-render); `selectedVisualId` / `onselectvisual(id | null)` (selection, null =
   * the page itself). Without a backend (browser preview) the page is drawn as a wireframe.
   */
  let {
    report,
    page,
    zoom,
    filters,
    dataVersion,
    selectedVisualId,
    onselectvisual,
    incoming = null,
    onrendered,
    onaction,
    onerror,
  }: {
    report: ReportDto
    page: PageDto
    zoom: number
    filters: FilterSpec | null
    dataVersion: number
    selectedVisualId: number | null
    onselectvisual: (id: number | null) => void
    /** A render produced outside the canvas (reset, Filters pane): shown if it is newer. */
    incoming?: RenderedPage | null
    onrendered?: (page: RenderedPage) => void
    onaction?: (action: CanvasAction) => void
    onerror?: (message: string) => void
  } = $props()

  const wireframe = !backend.hasBackend

  let rendered = $state.raw<RenderedPage | null>(null)
  let failure = $state<string | null>(null)
  let pending = $state(0)
  let slow = $state(false)
  let pageEl = $state<HTMLDivElement | null>(null)
  let svgHost = $state<HTMLDivElement | null>(null)
  let overlayHost = $state<HTMLDivElement | null>(null)
  let svgSeq = 0
  let loadSeq = 0
  let slowTimer: ReturnType<typeof setTimeout> | undefined

  const width = $derived(rendered?.width || page.width)
  const height = $derived(rendered?.height || page.height)
  const px = (v: number) => `${Math.round(v * zoom * 100) / 100}px`
  const byName = $derived(new Map(page.visuals.map((v) => [v.name, v])))
  const selectedName = $derived(page.visuals.find((v) => v.id === selectedVisualId)?.name ?? null)
  const visualInfo = $derived(new Map((rendered?.visuals ?? []).map((v) => [v.name, v])))

  // ---------------------------------------------------------------------------------------------
  // Requests
  // ---------------------------------------------------------------------------------------------

  /** `dto` is a render of the page this canvas shows (renders name pages by report and page name). */
  const isShown = (dto: RenderedPage) => dto.report === report.name && dto.page === page.name

  /** Shows `dto` unless it is for another page or older than what is shown. */
  function accept(dto: RenderedPage | null | undefined): boolean {
    if (!dto || !isShown(dto)) return false
    if (rendered && isShown(rendered) && dto.generation < rendered.generation) return false
    rendered = dto
    failure = null
    tipCache.clear()
    onrendered?.(dto)
    return true
  }

  /** Runs a canvas command with the busy indicator; errors other than `cancelled` are reported. */
  async function run<T>(op: () => Promise<backend.Outcome<T>>, fatal = false): Promise<T | null> {
    pending += 1
    if (pending === 1) {
      clearTimeout(slowTimer)
      slowTimer = setTimeout(() => (slow = pending > 0), 250)
    }
    try {
      const result = await op()
      if (result.ok) return result.value
      if (result.kind !== 'cancelled') {
        if (fatal) failure = result.error
        else onerror?.(result.error)
      }
      return null
    } finally {
      pending -= 1
      if (pending === 0) {
        clearTimeout(slowTimer)
        slow = false
      }
    }
  }

  async function load(reportId: number, pageId: number) {
    const mine = ++loadSeq
    const dto = await run(() => backend.renderPage(reportId, pageId), true)
    if (mine === loadSeq) accept(dto)
  }

  // Render on page change and whenever table data changes.
  $effect(() => {
    const r = report.id
    const p = page.id
    void dataVersion
    if (wireframe) return
    untrack(() => {
      if (rendered && !isShown(rendered)) {
        rendered = null
        tip = null
        searchEdit = null
        scrollQueue.clear()
      }
      failure = null
      void load(r, p)
    })
  })

  $effect(() => {
    const dto = incoming
    if (dto) untrack(() => accept(dto))
  })

  // ---------------------------------------------------------------------------------------------
  // SVG insertion
  // ---------------------------------------------------------------------------------------------

  function insert(host: HTMLDivElement | null, text: string | null | undefined, label: string) {
    if (!host) return
    const el = text ? sanitizeSvg(text, `pbi${++svgSeq}-`) : null
    if (el) {
      el.setAttribute('width', '100%')
      el.setAttribute('height', '100%')
      el.setAttribute('aria-label', label)
      el.setAttribute('role', 'img')
      host.replaceChildren(el)
    } else {
      host.replaceChildren()
      if (text) failure = 'The renderer returned an unreadable SVG.'
    }
  }

  /** What each layer shows (host element and SVG text), to skip re-parsing identical SVG. */
  const shown = new Map<string, { host: HTMLDivElement | null; text: string | null }>()
  let insertFrame = 0

  function update(layer: string, host: HTMLDivElement | null, text: string | null, label: string) {
    const prev = shown.get(layer)
    if (prev && prev.host === host && prev.text === text) return
    insert(host, text, label)
    shown.set(layer, { host, text })
  }

  /** Inserts the newest render's SVGs (once per animation frame). */
  function flushInsert() {
    insertFrame = 0
    const dto = rendered
    update('page', svgHost, dto?.svg ?? null, `${report.name} › ${page.displayName}`)
    update('overlay', overlayHost, dto?.overlaySvg ?? null, 'Slicer dropdown')
  }

  $effect(() => {
    void rendered
    void svgHost
    void overlayHost
    if (!insertFrame) insertFrame = requestAnimationFrame(flushInsert)
  })

  $effect(() => () => cancelAnimationFrame(insertFrame))

  // ---------------------------------------------------------------------------------------------
  // Hit-testing
  // ---------------------------------------------------------------------------------------------

  /** Pointer position in page units. */
  function toPage(event: MouseEvent): { x: number; y: number } {
    const box = pageEl?.getBoundingClientRect()
    if (!box || box.width <= 0 || box.height <= 0) return { x: -1, y: -1 }
    return { x: ((event.clientX - box.left) * width) / box.width, y: ((event.clientY - box.top) * height) / box.height }
  }

  function hitOf(event: Event): Hit | null {
    const target = event.target as Element | null
    const el = target?.closest?.('[data-hit]')
    if (!el || !pageEl?.contains(el)) return null
    return parseHit(el.getAttribute('data-hit') ?? '')
  }

  const inOverlay = (event: Event) => !!(event.target as Element | null)?.closest?.('.overlay-layer')
  const modifiers = (event: MouseEvent | KeyboardEvent) => ({ ctrl: event.ctrlKey || event.metaKey, shift: event.shiftKey })

  function selectVisual(name: string | null) {
    const v = name ? byName.get(name) : undefined
    onselectvisual(v ? v.id : null)
  }

  async function click(hitId: string | null, mods: { ctrl: boolean; shift: boolean }) {
    if (!rendered) return
    const { id: reportId } = report
    const { id: pageId } = page
    const result = await run(() => backend.canvasClick(reportId, pageId, hitId, mods))
    if (!result) return
    accept(result.page)
    if (result.action) onaction?.(result.action)
  }

  function onclick(event: MouseEvent) {
    if (!rendered || wireframe) return
    const point = toPage(event)
    const search = rendered.interactions.find((i) => contains(i.search, point.x, point.y))
    if (search) {
      void openSearch(search)
      return
    }
    const hit = hitOf(event)
    if (!hit && inOverlay(event)) return
    tip = null
    if (hit?.kind === 'dataPoint' && (event.target as Element).closest('.pbi-expander')) {
      void toggleRow(hit.id)
      return
    }
    selectVisual(hit?.visual ?? null)
    void click(hit?.id ?? null, modifiers(event))
  }

  function ondblclick(event: MouseEvent) {
    const hit = hitOf(event)
    const type = hit ? visualInfo.get(hit.visual)?.visualType : undefined
    if (hit?.kind === 'dataPoint' && (type === 'pivotTable' || type === 'matrix') && !(event.target as Element).closest('.pbi-expander')) void toggleRow(hit.id)
  }

  async function toggleRow(hitId: string) {
    if (!rendered) return
    const { id: reportId } = report
    const { id: pageId } = page
    accept(await run(() => backend.toggleMatrixRow(reportId, pageId, hitId)))
  }

  // ---------------------------------------------------------------------------------------------
  // Wheel scrolling (tables, matrices, slicer lists, open popups)
  // ---------------------------------------------------------------------------------------------

  const scrollQueue = new Map<string, { dx: number; dy: number }>()
  let scrolling = false

  async function flushScroll() {
    if (scrolling) return
    scrolling = true
    try {
      while (scrollQueue.size > 0 && rendered) {
        const [visual, delta] = scrollQueue.entries().next().value as [string, { dx: number; dy: number }]
        scrollQueue.delete(visual)
        const { id: reportId } = report
        const { id: pageId } = page
        const dto = await run(() => backend.canvasScroll(reportId, pageId, visual, delta.dx, delta.dy))
        accept(dto)
      }
    } finally {
      scrolling = false
    }
  }

  function canScroll(it: CanvasInteractionOut, dx: number, dy: number): { x: boolean; y: boolean } {
    const s = it.scroll
    if (!s) return { x: false, y: false }
    return {
      x: (dx > 0 && s.x < s.maxX - 0.5) || (dx < 0 && s.x > 0.5),
      y: (dy > 0 && s.y < s.maxY - 0.5) || (dy < 0 && s.y > 0.5),
    }
  }

  $effect(() => {
    const el = pageEl
    if (!el || wireframe) return
    const wheel = (event: WheelEvent) => {
      // Ctrl/⌘ + wheel zooms the canvas (ReportView).
      if (event.ctrlKey || event.metaKey || !rendered) return
      const point = toPage(event)
      const it = scrollableAt(rendered.interactions, point.x, point.y)
      if (!it) return
      const { dx, dy } = wheelPixels(event)
      const can = canScroll(it, dx, dy)
      const queued = scrollQueue.has(it.visual)
      if (!can.x && !can.y && !queued) return
      event.preventDefault()
      const scale = zoom > 0 ? zoom : 1
      const prev = scrollQueue.get(it.visual) ?? { dx: 0, dy: 0 }
      scrollQueue.set(it.visual, { dx: prev.dx + dx / scale, dy: prev.dy + dy / scale })
      void flushScroll()
    }
    el.addEventListener('wheel', wheel, { passive: false })
    return () => el.removeEventListener('wheel', wheel)
  })

  // ---------------------------------------------------------------------------------------------
  // Tooltips
  // ---------------------------------------------------------------------------------------------

  interface Tip {
    x: number
    y: number
    lines: TooltipLine[]
    text: string | null
    code: string | null
    warn: boolean
  }

  let tip = $state<Tip | null>(null)
  let hoverId: string | null = null
  let hoverVisual = $state<string | null>(null)
  let tipTimer: ReturnType<typeof setTimeout> | undefined
  const tipCache = new Map<string, TooltipLine[]>()

  /** Visual-level tooltip text: the query error with its DAX, or a button's tooltip. */
  function visualTip(hit: Hit): Omit<Tip, 'x' | 'y'> | null {
    const info = visualInfo.get(hit.visual)
    if (!info) return null
    if (info.error) {
      const full = info.tooltip ?? info.error
      const cut = full.indexOf('\n\n')
      return { lines: [], text: cut >= 0 ? full.slice(0, cut) : full, code: cut >= 0 ? full.slice(cut + 2) : null, warn: true }
    }
    if (hit.kind === 'button' && info.tooltip) return { lines: [], text: info.tooltip, code: null, warn: false }
    if (info.truncated && hit.kind === 'visual') return { lines: [], text: 'Not all data is shown: the query was cut at the row limit.', code: null, warn: false }
    return null
  }

  async function showTip(hit: Hit, x: number, y: number) {
    const local = visualTip(hit)
    if (local) {
      tip = { ...local, x, y }
      return
    }
    if (hit.kind !== 'dataPoint' || !rendered) return
    const { generation } = rendered
    const { id: reportId } = report
    const { id: pageId } = page
    let lines = tipCache.get(hit.id)
    if (!lines) {
      const result = await backend.visualTooltip(reportId, pageId, hit.id)
      if (!result.ok || hoverId !== hit.id || rendered?.generation !== generation) return
      lines = result.value.lines
      tipCache.set(hit.id, lines)
    }
    if (lines.length > 0 && hoverId === hit.id) tip = { lines, text: null, code: null, warn: false, x, y }
  }

  function onpointermove(event: PointerEvent) {
    if (!rendered || (event.target as Element | null)?.closest?.('.chrome-control')) return
    const hit = hitOf(event)
    hoverVisual = hit?.visual ?? null
    const key = hit?.id ?? null
    if (key !== hoverId) {
      hoverId = key
      clearTimeout(tipTimer)
      tip = null
      if (hit) {
        const { clientX, clientY } = event
        tipTimer = setTimeout(() => void showTip(hit, clientX, clientY), hit.kind === 'dataPoint' ? 250 : 500)
      }
    } else if (tip) {
      tip = { ...tip, x: event.clientX, y: event.clientY }
    }
  }

  function onpointerleave() {
    hoverId = null
    hoverVisual = null
    clearTimeout(tipTimer)
    tip = null
  }

  const tipLeft = $derived(tip ? (tip.x + 300 > window.innerWidth ? Math.max(4, tip.x - 300) : tip.x + 14) : 0)
  const tipTop = $derived(tip ? (tip.y + 160 > window.innerHeight ? Math.max(4, tip.y - 140) : tip.y + 16) : 0)

  // ---------------------------------------------------------------------------------------------
  // Slicer search box and "Clear selections"
  // ---------------------------------------------------------------------------------------------

  let searchEdit = $state<{ visual: string; text: string } | null>(null)
  let searchInput = $state<HTMLInputElement | null>(null)
  let searchTimer: ReturnType<typeof setTimeout> | undefined
  const searchRect = $derived(searchEdit ? (rendered?.interactions.find((i) => i.visual === searchEdit?.visual)?.search ?? null) : null)

  async function openSearch(it: CanvasInteractionOut) {
    searchEdit = { visual: it.visual, text: it.searchText }
    await tick()
    searchInput?.focus()
    searchInput?.select()
  }

  function applySearch(delay: number) {
    const edit = searchEdit
    if (!edit || !rendered) return
    clearTimeout(searchTimer)
    const { id: reportId } = report
    const { id: pageId } = page
    searchTimer = setTimeout(() => {
      void run(() => backend.setSlicerSearch(reportId, pageId, edit.visual, edit.text)).then(accept)
    }, delay)
  }

  function searchKeydown(event: KeyboardEvent) {
    if (!searchEdit) return
    if (event.key === 'Escape') {
      event.stopPropagation()
      if (searchEdit.text) {
        searchEdit.text = ''
        applySearch(0)
      } else searchEdit = null
    } else if (event.key === 'Enter') {
      applySearch(0)
    }
  }

  /** The hovered slicer's "Clear selections" eraser (right end of its header). */
  const eraser = $derived.by(() => {
    const it = rendered?.interactions.find((i) => i.visual === hoverVisual && i.kind === 'slicer' && i.hasSelection)
    return it?.header ? { visual: it.visual, x: it.header.x + it.header.width - 22, y: it.header.y + 1 } : null
  })

  async function clearSlicer(visual: string) {
    if (!rendered) return
    const { id: reportId } = report
    const { id: pageId } = page
    accept(await run(() => backend.clearSelections(reportId, pageId, visual)))
  }

  // ---------------------------------------------------------------------------------------------
  // Keyboard
  // ---------------------------------------------------------------------------------------------

  async function onkeydown(event: KeyboardEvent) {
    if (event.key !== 'Escape' || event.defaultPrevented || !rendered) return
    const target = event.target as HTMLElement | null
    if (target && (target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName))) return
    if (document.querySelector('[aria-modal="true"]')) return
    tip = null
    const { id: reportId } = report
    const { id: pageId } = page
    if (rendered.interactions.some((i) => i.dropdownOpen)) {
      accept(await run(() => backend.closePopups(reportId, pageId)))
    } else if (rendered.selections.some((s) => s.kind === 'crossFilter')) {
      accept(await run(() => backend.clearSelections(reportId, pageId, null)))
    }
  }

  function focusKey(event: KeyboardEvent, v: VisualDto) {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      onselectvisual(v.id)
    }
  }

  // Focusable visuals (keyboard navigation): visible, non-group, in tab order.
  const focusables = $derived(
    (rendered?.visuals ?? [])
      .filter((v) => !v.hidden && v.rect && v.rect.width > 0 && !byName.get(v.name)?.isGroup)
      .map((v) => ({ info: v, dto: byName.get(v.name) }))
      .filter((v): v is { info: (typeof v)['info']; dto: VisualDto } => !!v.dto),
  )
  const selectedRect = $derived(selectedName ? (visualInfo.get(selectedName)?.rect ?? null) : null)

  // Wireframe (browser preview without the Rust backend).
  const label = (v: VisualDto) => v.title ?? v.fields[0]?.displayName ?? visualKind(v.visualType).label
</script>

<svelte:window {onkeydown} />

<!-- Keyboard: the focusable visual targets below and Esc (window). -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<div
  class="page"
  class:busy={slow}
  bind:this={pageEl}
  style:width={px(width)}
  style:height={px(height)}
  role="region"
  aria-label={`${report.name} › ${page.displayName}`}
  aria-busy={pending > 0}
  data-page-id={page.id}
  data-data-version={dataVersion}
  data-filter-count={filters?.filters.length ?? 0}
  {onclick}
  {ondblclick}
  {onpointermove}
  onpointerleave={onpointerleave}
>
  {#if wireframe}
    {#each page.visuals.filter((v) => !v.isGroup && !v.hidden) as v (v.id)}
      <button
        class="wire"
        class:selected={selectedVisualId === v.id}
        style:left={px(v.position.x)}
        style:top={px(v.position.y)}
        style:width={px(v.position.width)}
        style:height={px(v.position.height)}
        onclick={(e) => {
          e.stopPropagation()
          onselectvisual(v.id)
        }}
        title={`${label(v)} (${visualKind(v.visualType).label})`}
      >
        <Icon name={visualKind(v.visualType).icon} size={14} />
        <span>{label(v)}</span>
      </button>
    {/each}
  {:else}
    <div class="svg-layer" bind:this={svgHost}></div>
    <div class="overlay-layer" bind:this={overlayHost}></div>

    <div class="chrome-layer">
      {#if selectedRect}
        <div class="selected-outline" style:left={px(selectedRect.x)} style:top={px(selectedRect.y)} style:width={px(selectedRect.width)} style:height={px(selectedRect.height)}>
          <span class="handle tl"></span><span class="handle tr"></span><span class="handle bl"></span><span class="handle br"></span>
        </div>
      {/if}
      {#each focusables as { info, dto } (info.name)}
        {#if info.rect}
          <div
            class="focus-target"
            tabindex="0"
            role="button"
            aria-label={`${info.title ?? visualKind(info.visualType).label} (${visualKind(info.visualType).label})${info.error ? ' — error' : ''}`}
            style:left={px(info.rect.x)}
            style:top={px(info.rect.y)}
            style:width={px(info.rect.width)}
            style:height={px(info.rect.height)}
            onkeydown={(e) => focusKey(e, dto)}
          ></div>
        {/if}
      {/each}
      {#if eraser}
        <button
          class="chrome-control eraser"
          style:left={px(eraser.x)}
          style:top={px(eraser.y)}
          title="Clear selections"
          aria-label="Clear selections"
          onclick={(e) => {
            e.stopPropagation()
            void clearSlicer(eraser.visual)
          }}
        >
          <Icon name="close" size={12} />
        </button>
      {/if}
      {#if searchEdit && searchRect}
        <input
          class="chrome-control slicer-search"
          bind:this={searchInput}
          type="search"
          placeholder="Search"
          aria-label="Search the slicer"
          style:left={px(searchRect.x)}
          style:top={px(searchRect.y)}
          style:width={px(searchRect.width)}
          style:height={px(searchRect.height)}
          style:font-size={px(12)}
          bind:value={searchEdit.text}
          oninput={() => applySearch(200)}
          onkeydown={searchKeydown}
          onblur={() => (searchEdit = null)}
          onclick={(e) => e.stopPropagation()}
        />
      {/if}
    </div>

    {#if !rendered && pending > 0}
      <div class="shimmer" aria-hidden="true"></div>
    {/if}
    {#if slow && rendered}
      <div class="progress" aria-hidden="true"></div>
    {/if}
    {#if failure && !rendered}
      <div class="failure" role="alert">
        <Icon name="warning" size={20} />
        <p>The page could not be rendered.</p>
        <p class="detail">{failure}</p>
        <button
          onclick={(e) => {
            e.stopPropagation()
            void load(report.id, page.id)
          }}>Try again</button
        >
      </div>
    {/if}
  {/if}
</div>

{#if tip}
  <div class="tooltip" class:warn={tip.warn} role="tooltip" style:left={`${tipLeft}px`} style:top={`${tipTop}px`}>
    {#if tip.text}
      <p class="tip-text">{#if tip.warn}<Icon name="warning" size={14} />{/if}{tip.text}</p>
    {/if}
    {#if tip.code}<pre>{tip.code}</pre>{/if}
    {#each tip.lines as l, i (i)}
      <div class="tip-line"><span class="tip-label">{l.label}</span><span class="tip-value">{l.value}</span></div>
    {/each}
  </div>
{/if}

<style>
  .page {
    position: relative;
    flex: none;
    background: #fff;
    box-shadow:
      0 1.6px 3.6px rgb(0 0 0 / 0.13),
      0 0.3px 0.9px rgb(0 0 0 / 0.11);
    user-select: none;
  }

  .svg-layer,
  .overlay-layer,
  .chrome-layer {
    position: absolute;
    inset: 0;
  }

  .svg-layer :global(svg) {
    display: block;
  }

  /* Popups overflow the page; only the popup itself takes pointer events. */
  .overlay-layer {
    z-index: 2;
    pointer-events: none;
  }

  .overlay-layer :global(svg) {
    display: block;
    overflow: visible;
    pointer-events: none;
  }

  .overlay-layer :global(.pbi-slicer-popup) {
    pointer-events: auto;
  }

  .page :global([data-hit*='|dataPoint|']),
  .page :global([data-hit*='|slicerItem|']),
  .page :global([data-hit$='|button']),
  .page :global(.pbi-expander) {
    cursor: pointer;
  }

  .chrome-layer {
    z-index: 3;
    pointer-events: none;
  }

  .selected-outline {
    position: absolute;
    border: 1px solid #8a8886;
    pointer-events: none;
  }

  .handle {
    position: absolute;
    width: 6px;
    height: 6px;
    border: 1px solid #605e5c;
    background: #fff;
  }

  .tl {
    top: -4px;
    left: -4px;
  }

  .tr {
    top: -4px;
    right: -4px;
  }

  .bl {
    bottom: -4px;
    left: -4px;
  }

  .br {
    right: -4px;
    bottom: -4px;
  }

  /* Keyboard focus only (reading view shows no outline on click). */
  .focus-target {
    position: absolute;
    pointer-events: none;
  }

  .focus-target:focus {
    outline: none;
  }

  .focus-target:focus-visible {
    outline: 2px solid var(--color-brand);
    outline-offset: 1px;
  }

  .chrome-control {
    position: absolute;
    pointer-events: auto;
  }

  .eraser {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    padding: 0;
    border: none;
    border-radius: 2px;
    background: rgb(255 255 255 / 0.85);
    color: #605e5c;
    cursor: pointer;
  }

  .eraser:hover {
    background: #f3f2f1;
    color: #252423;
  }

  .slicer-search {
    box-sizing: border-box;
    padding: 0 6px 0 24px;
    border: 1px solid var(--color-brand);
    border-radius: 0;
    outline: none;
    background: #fff;
    color: #252423;
    font-family: Selawik, 'Segoe UI', sans-serif;
  }

  .shimmer {
    position: absolute;
    inset: 0;
    background: linear-gradient(100deg, #fff 30%, #f3f2f1 50%, #fff 70%);
    background-size: 300% 100%;
    animation: shimmer 1.2s linear infinite;
  }

  @keyframes shimmer {
    from {
      background-position: 100% 0;
    }
    to {
      background-position: 0 0;
    }
  }

  .progress {
    position: absolute;
    top: 0;
    left: 0;
    z-index: 4;
    width: 100%;
    height: 2px;
    overflow: hidden;
    background: linear-gradient(90deg, transparent, var(--color-brand), transparent);
    background-size: 40% 100%;
    background-repeat: no-repeat;
    animation: progress 1s ease-in-out infinite;
  }

  @keyframes progress {
    from {
      background-position: -40% 0;
    }
    to {
      background-position: 140% 0;
    }
  }

  .failure {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 24px;
    color: var(--color-text-muted);
    text-align: center;
  }

  .failure p {
    margin: 0;
  }

  .failure .detail {
    max-width: 60ch;
    font-size: 12px;
    word-break: break-word;
  }

  /* Desktop-like visual tooltip: white card, gray labels, dark values. */
  .tooltip {
    position: fixed;
    z-index: 300;
    max-width: 320px;
    padding: 8px 10px;
    border: 1px solid #e1dfdd;
    border-radius: 2px;
    background: #fff;
    box-shadow: 0 3.2px 7.2px rgb(0 0 0 / 0.13), 0 0.6px 1.8px rgb(0 0 0 / 0.11);
    color: #252423;
    font-family: 'Segoe UI', Selawik, sans-serif;
    font-size: 12px;
    pointer-events: none;
  }

  .tooltip.warn {
    max-width: 420px;
  }

  .tip-line {
    display: flex;
    gap: 12px;
    justify-content: space-between;
    line-height: 18px;
  }

  .tip-label {
    color: #605e5c;
  }

  .tip-value {
    font-weight: 600;
    text-align: right;
    overflow-wrap: anywhere;
  }

  .tip-text {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    margin: 0;
    white-space: pre-wrap;
  }

  .warn .tip-text :global(svg) {
    flex: none;
    color: #c50f1f;
  }

  .tooltip pre {
    max-height: 200px;
    margin: 6px 0 0;
    padding: 6px;
    overflow: hidden;
    background: var(--color-code-bg);
    font-family: var(--font-mono);
    font-size: 11px;
    white-space: pre-wrap;
  }

  /* Wireframe (browser preview). */
  .wire {
    position: absolute;
    display: flex;
    align-items: flex-start;
    gap: 5px;
    padding: 6px 8px;
    overflow: hidden;
    border: 1px solid #e6e6e6;
    border-radius: 2px;
    background: #fff;
    color: var(--color-text-muted);
    font-size: 12px;
    text-align: left;
    cursor: pointer;
  }

  .wire.selected {
    border-color: #666;
  }
</style>
