<script lang="ts">
  import Icon from '../Icon.svelte'
  import * as backend from '../../lib/backend'
  import type { FilterSpec, MeasureValue, PageDto, ReportDto } from '../../lib/backend'
  import { filterSummary } from '../../lib/data'

  // Measures panel (DashCraft extra, docked under the report canvas, collapsed by default): every
  // model measure evaluated by pbi-dax, by default in the canvas's filter context (report and
  // page filters plus the slicer selections and the cross-filter, so the values follow the
  // cards), or under the report's (and page's) filters only, or unfiltered. The values also
  // feed the Data pane's measure tooltips (`onvalues`).
  let {
    report,
    page,
    version,
    loaded,
    onselect,
    onvalues,
    canvasFilters = null,
  }: {
    report: ReportDto | null
    page: PageDto | null
    /** Bumped whenever table data changes. */
    version: number
    /** Some table has data (otherwise every measure is blank). */
    loaded: boolean
    onselect: (id: number) => void
    onvalues?: (measures: MeasureValue[]) => void
    /** The canvas's effective filters (report + page filters, slicers, cross-filter). */
    canvasFilters?: FilterSpec | null
  } = $props()

  type Scope = 'none' | 'report' | 'page' | 'canvas'

  let scope = $state<Scope>('canvas')
  let collapsed = $state(true)
  let showHidden = $state(false)
  let measures = $state.raw<MeasureValue[] | null>(null)
  let applied = $state.raw<FilterSpec | null>(null)
  let error = $state<string | null>(null)
  let loading = $state(false)
  let seq = 0

  const shown = $derived((measures ?? []).filter((m) => showHidden || !m.hidden))
  const hiddenCount = $derived((measures ?? []).filter((m) => m.hidden).length)
  const errors = $derived((measures ?? []).filter((m) => m.error != null).length)

  $effect(() => {
    const s = scope
    const reportId = report?.id ?? null
    const pageId = s === 'page' || s === 'canvas' ? (page?.id ?? null) : null
    const canvas = s === 'canvas' ? canvasFilters : null
    void version
    void evaluate(s, reportId, pageId, canvas)
  })

  async function evaluate(s: Scope, reportId: number | null, pageId: number | null, canvas: FilterSpec | null) {
    const mine = ++seq
    loading = true
    let spec: FilterSpec | null = canvas
    // Until the canvas has rendered, its filters are the report + page filters.
    if (!spec && s !== 'none' && reportId != null) {
      const filters = await backend.reportFilters(reportId, s === 'report' ? null : pageId)
      if (mine !== seq) return
      if (!filters.ok) {
        loading = false
        error = filters.error
        return
      }
      spec = filters.value
    }
    const result = await backend.measures(spec)
    if (mine !== seq) return
    loading = false
    if (result.ok) {
      measures = result.value
      applied = spec
      error = null
      onvalues?.(result.value)
    } else if (result.kind !== 'cancelled') {
      error = result.error
    }
  }

  function tooltip(m: MeasureValue): string {
    const head = m.error ? `⚠ ${m.error}${m.unsupported ? ' (not supported yet)' : ''}` : `${m.name} = ${m.formatted || '(Blank)'}`
    return [head, `${m.table}[${m.name}]${m.formatString ? ` · format ${m.formatString}` : ''}`, '', m.dax].join('\n')
  }
</script>

<section class="kpis" class:collapsed aria-label="Measures">
  <header>
    <button class="toggle" onclick={() => (collapsed = !collapsed)} aria-expanded={!collapsed} title={collapsed ? 'Show measures' : 'Hide measures'}>
      <span class="chev" class:open={!collapsed}><Icon name="chevron" size={12} /></span>
      <Icon name="measure" size={14} />
      <strong>Measures</strong>
      {#if measures}<span class="muted">{shown.length}</span>{/if}
      {#if errors > 0}<span class="err-count" title={`${errors} measure(s) failed`}><Icon name="warning" size={12} /> {errors}</span>{/if}
    </button>
    <label class="scope">
      Filter
      <select bind:value={scope} aria-label="Filter context of the measures">
        <option value="none">None (whole model)</option>
        <option value="report" disabled={!report}>Report filters</option>
        <option value="page" disabled={!page}>Report + page filters</option>
        <option value="canvas" disabled={!page}>As on the canvas (+ slicers, selection)</option>
      </select>
    </label>
    {#if applied}
      <span class="applied">
        {#each applied.filters as f, i (i)}
          <code class="filter" title={filterSummary(f)}>{filterSummary(f)}</code>
        {:else}
          <span class="muted">no translatable filters</span>
        {/each}
        {#each applied.unsupported as u, i (i)}
          <span class="unsupported" title={`${u.kind} filter${u.field ? ` on ${u.field}` : ''} not applied: ${u.reason}`}>
            <Icon name="warning" size={12} /> {u.field ?? u.name}
          </span>
        {/each}
      </span>
    {/if}
    <span class="spacer"></span>
    {#if loading}<span class="spinner" aria-label="Evaluating"></span>{/if}
    {#if hiddenCount > 0}
      <label class="hidden-toggle"><input type="checkbox" bind:checked={showHidden} /> Hidden ({hiddenCount})</label>
    {/if}
  </header>
  {#if !collapsed}
    <div class="cards" class:stale={loading}>
      {#if error}
        <p class="error" role="alert">{error}</p>
      {:else if measures && measures.length === 0}
        <p class="muted none">The model has no measures.</p>
      {:else}
        {#if measures && !loaded}
          <p class="muted none">No data loaded yet: use <strong>Refresh</strong> to compute the values.</p>
        {/if}
        {#each shown as m (`${m.table}[${m.name}]`)}
          <button class="card" class:failed={m.error != null} class:hidden-measure={m.hidden} title={tooltip(m)} onclick={() => m.id != null && onselect(m.id)}>
            <span class="name">{m.name}</span>
            {#if m.error}
              <span class="value warn"><Icon name="warning" size={14} /> {m.unsupported ? 'Unsupported' : 'Error'}</span>
            {:else if !m.formatted}
              <span class="value blank">(Blank)</span>
            {:else}
              <span class="value">{m.formatted}</span>
            {/if}
          </button>
        {/each}
      {/if}
    </div>
  {/if}
</section>

<style>
  .kpis {
    flex: none;
    border-top: 1px solid var(--color-border);
    background: var(--color-pane);
  }

  header {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 32px;
    padding: 2px 12px;
    font-size: 12px;
  }

  .toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 4px;
    border: none;
    background: transparent;
    cursor: pointer;
  }

  .chev :global(svg) {
    transition: transform 120ms ease;
  }

  .chev.open :global(svg) {
    transform: rotate(90deg);
  }

  .err-count {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    color: var(--color-accent-dark);
  }

  .scope {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--color-text-muted);
  }

  select {
    font: inherit;
  }

  .applied {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    min-width: 0;
  }

  .filter {
    max-width: 320px;
    overflow: hidden;
    padding: 0 6px;
    border: 1px solid var(--color-border);
    border-radius: 9px;
    background: var(--color-chrome);
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .unsupported {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    padding: 0 6px;
    border: 1px solid var(--color-warning-border);
    border-radius: 9px;
    background: var(--color-warning-bg);
    font-size: 11px;
  }

  .spacer {
    flex: 1;
  }

  .hidden-toggle {
    display: flex;
    align-items: center;
    gap: 4px;
    color: var(--color-text-muted);
  }

  .spinner {
    width: 12px;
    height: 12px;
    border: 2px solid var(--color-border);
    border-top-color: var(--color-brand);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .cards {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    max-height: 168px;
    overflow: auto;
    padding: 4px 12px 10px;
  }

  .cards.stale {
    opacity: 0.6;
  }

  .card {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    min-width: 120px;
    max-width: 220px;
    padding: 6px 10px;
    border: 1px solid var(--color-border);
    border-left: 3px solid var(--color-accent);
    border-radius: 2px;
    background: var(--color-pane);
    text-align: left;
    cursor: pointer;
  }

  .card:hover {
    background: var(--color-hover);
  }

  .card.failed {
    border-left-color: var(--color-danger);
  }

  .card.hidden-measure {
    opacity: 0.6;
  }

  .name {
    max-width: 100%;
    overflow: hidden;
    color: var(--color-text-muted);
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .value {
    max-width: 100%;
    overflow: hidden;
    font-size: 18px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .value.warn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--color-danger);
    font-size: 13px;
  }

  .value.blank {
    color: var(--color-text-subtle);
    font-size: 13px;
    font-style: italic;
    font-weight: 400;
  }

  .none,
  .error {
    margin: 4px 0;
    font-size: 12px;
  }

  .none {
    width: 100%;
  }

  .error {
    color: var(--color-danger);
  }

  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation-duration: 2.4s;
    }
  }
</style>
