<script lang="ts">
  import Icon from '../Icon.svelte'
  import MeasuresPanel from './MeasuresPanel.svelte'
  import ReportCanvas from './ReportCanvas.svelte'
  import type { CanvasAction, FilterDto, FilterSpec, MeasureValue, PageDto, RenderedPage, ReportDto } from '../../lib/backend'
  import { fieldTooltip } from '../../lib/model'
  import type { ZoomMode } from '../../lib/shell'
  import { visualKind } from '../../lib/visuals'

  // Report view centre: the gray canvas surround with the page (ReportCanvas: the rendered,
  // interactive page, scaled per View → Page view or the status bar zoom), the DashCraft "Outline"
  // alternative, and the Measures panel docked at the bottom (DashCraft extra: every measure under
  // the canvas's filters, slicers and cross-filter, or the report / page filters only).
  let {
    report,
    page,
    selectedVisualId,
    onselectvisual,
    onselect,
    zoomMode,
    zoomPercent,
    onzoom,
    onzoomrequest,
    canvasMode,
    filters,
    dataVersion,
    dataLoaded,
    hasMeasures,
    onmeasures,
    canvasIncoming = null,
    canvasFilters = null,
    oncanvasrendered,
    oncanvasaction,
    oncanvaserror,
  }: {
    report: ReportDto | null
    page: PageDto | null
    selectedVisualId: number | null
    onselectvisual: (id: number | null) => void
    /** Selects any object (Properties / formula bar). */
    onselect: (id: number) => void
    zoomMode: ZoomMode
    /** Used with `zoomMode = custom`. */
    zoomPercent: number
    /** Reports the effective zoom in percent (after fitting). */
    onzoom: (percent: number) => void
    /** Ctrl/⌘ + wheel asks for a custom zoom (percent). */
    onzoomrequest: (percent: number) => void
    canvasMode: 'wireframe' | 'outline'
    filters: FilterSpec | null
    dataVersion: number
    dataLoaded: boolean
    hasMeasures: boolean
    onmeasures: (values: MeasureValue[]) => void
    /** A canvas render made outside the canvas (Reset to default, Filters pane). */
    canvasIncoming?: RenderedPage | null
    /** The canvas's effective filters (report + page filters, slicers, cross-filter). */
    canvasFilters?: FilterSpec | null
    oncanvasrendered?: (page: RenderedPage) => void
    oncanvasaction?: (action: CanvasAction) => void
    oncanvaserror?: (message: string) => void
  } = $props()

  const PAD = 24
  let viewportWidth = $state(0)
  let viewportHeight = $state(0)
  let surround = $state<HTMLDivElement | null>(null)

  const zoom = $derived.by(() => {
    if (!page || page.width <= 0 || page.height <= 0) return 1
    const w = Math.max(50, viewportWidth - 2 * PAD)
    const h = Math.max(50, viewportHeight - 2 * PAD)
    switch (zoomMode) {
      case 'fitPage':
        return Math.max(0.1, Math.min(w / page.width, h / page.height))
      case 'fitWidth':
        return Math.max(0.1, (w - 12) / page.width)
      case 'actual':
        return 1
      case 'custom':
        return Math.max(0.1, zoomPercent / 100)
    }
  })

  $effect(() => {
    if (viewportWidth > 0) onzoom(zoom * 100)
  })

  /** Ctrl/⌘ + wheel zooms the canvas, as in Desktop (a non-passive listener, to stop scrolling). */
  $effect(() => {
    const el = surround
    if (!el) return
    const wheel = (event: WheelEvent) => {
      if (!(event.ctrlKey || event.metaKey) || !page) return
      event.preventDefault()
      const factor = event.deltaY < 0 ? 1.1 : 1 / 1.1
      onzoomrequest(Math.max(10, Math.min(400, zoom * 100 * factor)))
    }
    el.addEventListener('wheel', wheel, { passive: false })
    return () => el.removeEventListener('wheel', wheel)
  })

  const outline = $derived(page ? [...page.visuals].sort((a, b) => a.position.y - b.position.y || a.position.x - b.position.x) : [])
  const label = (v: PageDto['visuals'][number]) => v.title ?? v.fields[0]?.displayName ?? visualKind(v.visualType).label
  const filterLabel = (f: FilterDto) => f.displayName ?? f.field ?? f.name
</script>

<div class="report-view">
  {#if !report}
    <div class="surround">
      <p class="empty">This file has no report pages. Use the Table, Model, DAX query or TMDL view.</p>
    </div>
  {:else if !page}
    <div class="surround"><p class="empty">The report has no pages.</p></div>
  {:else if canvasMode === 'wireframe'}
    <div class="surround" class:fit={zoomMode === 'fitPage'} bind:clientWidth={viewportWidth} bind:clientHeight={viewportHeight} bind:this={surround}>
      <div class="pad">
        <ReportCanvas
          {report}
          {page}
          {zoom}
          {filters}
          {dataVersion}
          {selectedVisualId}
          {onselectvisual}
          incoming={canvasIncoming}
          onrendered={oncanvasrendered}
          onaction={oncanvasaction}
          onerror={oncanvaserror}
        />
      </div>
    </div>
  {:else}
    <div class="outline-scroll">
      <table class="outline">
        <thead>
          <tr><th>Visual</th><th>Position</th><th>Fields</th><th>Filters</th></tr>
        </thead>
        <tbody>
          {#each outline as v (v.id)}
            <tr class:selected={selectedVisualId === v.id} class:hidden-visual={v.hidden}>
              <td>
                <button class="link visual-name" onclick={() => onselectvisual(v.id)} title={label(v)}>
                  <Icon name={visualKind(v.visualType).icon} size={14} />
                  {label(v)}
                </button>
                {#if v.hidden}<Icon name="hidden" size={12} title="Hidden" />{/if}
                <span class="sub mono">{v.visualType}{v.parentGroup ? ' · grouped' : ''}</span>
              </td>
              <td class="mono nowrap">
                {Math.round(v.position.x)}, {Math.round(v.position.y)}
                <span class="sub muted">{Math.round(v.position.width)}×{Math.round(v.position.height)}</span>
              </td>
              <td class="fields-cell">
                {#each v.fields as f, i (i)}
                  <span class="field-row" title={fieldTooltip(f)}>
                    <span class="role">{f.role}</span>
                    <span class:renamed={f.renamed}>{f.displayName}</span>
                  </span>
                {/each}
              </td>
              <td class="filters-cell">
                {#each v.filters as f (f.id)}
                  <button class="link mono small" onclick={() => onselect(f.id)} title={f.summary}>{filterLabel(f)}</button>
                {/each}
              </td>
            </tr>
          {:else}
            <tr><td colspan="4" class="empty-hint">No visuals on this page.</td></tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}

  {#if hasMeasures}
    <MeasuresPanel {report} {page} version={dataVersion} loaded={dataLoaded} {onselect} onvalues={onmeasures} {canvasFilters} />
  {/if}
</div>

<style>
  .report-view {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--color-canvas);
  }

  .surround {
    flex: 1;
    min-height: 0;
    overflow: auto;
    background: var(--color-canvas);
  }

  .surround.fit {
    overflow: hidden;
  }

  .pad {
    display: flex;
    justify-content: center;
    align-items: flex-start;
    width: max-content;
    min-width: 100%;
    min-height: 100%;
    padding: 24px;
  }

  .surround.fit .pad {
    align-items: center;
  }

  .empty {
    margin: 48px auto;
    color: var(--color-text-muted);
    text-align: center;
  }

  .outline-scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
    background: var(--color-pane);
  }

  .outline {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }

  .outline th,
  .outline td {
    padding: 4px 8px;
    border-bottom: 1px solid var(--color-border);
    text-align: left;
    vertical-align: top;
  }

  .outline th {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--color-chrome);
    font-weight: 600;
  }

  .outline tr.selected {
    background: var(--color-selected);
  }

  .hidden-visual {
    opacity: 0.6;
  }

  .sub {
    display: block;
    color: var(--color-text-subtle);
    font-size: 11px;
  }

  .fields-cell {
    min-width: 200px;
  }

  .filters-cell {
    min-width: 110px;
  }

  .field-row {
    display: block;
  }

  .role {
    margin-right: 4px;
    color: var(--color-text-subtle);
    font-size: 11px;
  }

  .renamed {
    text-decoration: underline dotted var(--color-text-subtle);
  }

  .mono {
    font-family: var(--font-mono);
  }

  .small {
    display: block;
    font-size: 11px;
  }

  .nowrap {
    white-space: nowrap;
  }

  .link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--color-brand);
    text-align: left;
    cursor: pointer;
  }

  .link:hover {
    text-decoration: underline;
  }

  .visual-name {
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    vertical-align: bottom;
  }
</style>
