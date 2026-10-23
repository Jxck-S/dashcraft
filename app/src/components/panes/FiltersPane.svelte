<script lang="ts">
  import Icon from '../Icon.svelte'
  import type { FilterDto, PageDto, ReportDto, VisualDto } from '../../lib/bindings'
  import type { SelectionOut } from '../../lib/ops'
  import { NOT_AVAILABLE } from '../../lib/shell'
  import type { SelectedId } from '../../lib/model'

  // Filters pane (read-only, reading view): search, then "Filters on this visual" (with a visual
  // selected), "Filters on this page" and "Filters on all pages" filter cards. Cards hidden in
  // reading view (`isHiddenInViewMode`) are not shown (a footnote can reveal them); locked
  // cards (`isLockedInViewMode`) carry a lock. Cards expand to the filter type and the DAX-like
  // condition. The canvas's slicer selections and cross-filter are listed under the page's
  // filters as "Slicer: …" / "Selection: …" cards with a clear button.
  let {
    report,
    page,
    visual,
    selected,
    onselect,
    selections = [],
    onclearselection,
    onselectvisualname,
  }: {
    report: ReportDto | null
    page: PageDto | null
    visual: VisualDto | null
    selected: SelectedId
    onselect: (id: number) => void
    /** Slicer selections and the cross-filter of the rendered page. */
    selections?: SelectionOut[]
    /** Clears a slicer's selection or the cross-filter of a visual. */
    onclearselection?: (visual: string) => void
    /** Selects a visual of the page by name. */
    onselectvisualname?: (name: string) => void
  } = $props()

  let search = $state('')
  let open = $state<Record<number, boolean>>({})
  let showHidden = $state(false)

  const visible = (f: FilterDto) => showHidden || !f.hidden
  const hiddenCount = $derived([...(visual?.filters ?? []), ...(page?.filters ?? []), ...(report?.filters ?? [])].filter((f) => f.hidden).length)

  const q = $derived(search.trim().toLowerCase())
  const shownSelections = $derived(selections.filter((s) => !q || s.label.toLowerCase().includes(q) || s.summary.toLowerCase().includes(q)))
  const title = (f: FilterDto) => f.displayName ?? f.field?.replace(/^'?([^'[]*)'?\[(.*)\]$/, '$2') ?? f.name
  const matches = (f: FilterDto) => !q || title(f).toLowerCase().includes(q) || (f.field?.toLowerCase().includes(q) ?? false)

  /** `T[C] IN {"a", "b"}` → `is a or b`; `NOT IN` → `is not …`; else the condition. */
  function friendly(f: FilterDto): string {
    const m = /^.+?\s(NOT IN|IN)\s\{(.*)\}$/s.exec(f.summary)
    if (!m) return f.summary || `${f.filterType} filter`
    const values = m[2]
      .split(/,\s*(?=(?:[^"]*"[^"]*")*[^"]*$)/)
      .map((v) => v.trim().replace(/^"(.*)"$/, '$1'))
      .filter((v) => v !== '')
    if (values.length === 0) return 'is (All)'
    const list = values.length > 3 ? `${values.slice(0, 3).join(', ')} and ${values.length - 3} more` : values.join(values.length === 2 ? ' or ' : ', ')
    return `${m[1] === 'IN' ? 'is' : 'is not'} ${list}`
  }

  const sections = $derived([
    ...(visual ? [{ id: 'visual', label: 'Filters on this visual', filters: visual.filters }] : []),
    ...(page ? [{ id: 'page', label: 'Filters on this page', filters: page.filters }] : []),
    ...(report ? [{ id: 'report', label: 'Filters on all pages', filters: report.filters }] : []),
  ])
</script>

<div class="filters-pane">
  <label class="search">
    <Icon name="search" size={14} />
    <input type="search" placeholder="Search" bind:value={search} aria-label="Search filter cards" />
  </label>

  {#if !report}
    <p class="empty-hint">No report.</p>
  {/if}

  {#each sections as s (s.id)}
    {@const shown = s.filters.filter((f) => visible(f) && matches(f))}
    <section>
      <h3>{s.label}</h3>
      {#if s.id === 'page'}
        {#each shownSelections as sel (`${sel.kind}:${sel.visual}`)}
          <div class="card selection-card">
            <div class="card-head">
              <button class="card-title" onclick={() => onselectvisualname?.(sel.visual)} title={`${sel.fields.join(', ')}\n${sel.summary}`}>
                <span class="name">{sel.kind === 'slicer' ? 'Slicer' : 'Selection'}: {sel.label}</span>
                <span class="condition">{sel.summary}</span>
              </button>
              <span class="flags">
                <button class="chev" onclick={() => onclearselection?.(sel.visual)} title={sel.kind === 'slicer' ? 'Clear selections' : 'Clear the selection'} aria-label="Clear">
                  <Icon name="close" size={12} />
                </button>
              </span>
            </div>
          </div>
        {/each}
      {/if}
      {#each shown as f (f.id)}
        {@const expanded = open[f.id] ?? false}
        <div class="card" class:selected={selected === f.id} class:hidden-card={f.hidden}>
          <div class="card-head">
            <button class="card-title" onclick={() => onselect(f.id)} title={`${f.field ?? f.name}\n${f.summary}`}>
              <span class="name">{title(f)}</span>
              <span class="condition">{friendly(f)}</span>
            </button>
            <span class="flags">
              {#if f.locked}<span title="Locked: report readers cannot change this filter"><Icon name="lock" size={14} /></span>{/if}
              {#if f.hidden}<span title="Hidden: report readers do not see this filter"><Icon name="hidden" size={14} /></span>{/if}
              <button class="chev" onclick={() => (open[f.id] = !expanded)} aria-expanded={expanded} aria-label={expanded ? 'Collapse the filter card' : 'Expand the filter card'}>
                <Icon name={expanded ? 'chevronUp' : 'chevronDown'} size={14} />
              </button>
            </span>
          </div>
          {#if expanded}
            <dl class="details">
              <dt>Filter type</dt>
              <dd>{f.filterType}</dd>
              {#if f.field}<dt>Field</dt><dd class="mono">{f.field}</dd>{/if}
              <dt>Condition</dt>
              <dd class="mono">{f.summary || '—'}</dd>
            </dl>
          {/if}
        </div>
      {:else}
        {#if q && s.filters.length > 0 && !(s.id === 'page' && shownSelections.length > 0)}<p class="none">No matching filters.</p>{/if}
      {/each}
      <div class="drop" title={NOT_AVAILABLE}>Add data fields here</div>
    </section>
  {/each}

  {#if hiddenCount > 0}
    <label class="hidden-note" title="Filter cards hidden in reading view (isHiddenInViewMode) still apply">
      <input type="checkbox" bind:checked={showHidden} />
      Show {hiddenCount} hidden filter card{hiddenCount === 1 ? '' : 's'}
    </label>
  {/if}
</div>

<style>
  .filters-pane {
    padding: 0 12px 16px;
  }

  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    margin-bottom: 6px;
    padding: 0 8px;
    border: 1px solid var(--color-border-strong);
    border-radius: 4px;
    color: var(--color-text-muted);
  }

  .search input {
    flex: 1;
    min-width: 0;
    border: none;
    outline: none;
    background: transparent;
  }

  .search:focus-within {
    border-color: var(--color-brand);
  }

  h3 {
    margin: 12px 0 6px;
    font-size: 13px;
    font-weight: 600;
  }

  .card {
    margin-bottom: 6px;
    border: 1px solid var(--color-border-strong);
    border-radius: 4px;
    background: var(--color-pane);
  }

  .card.selected {
    border-color: var(--color-brand);
    box-shadow: 0 0 0 1px var(--color-brand);
  }

  .hidden-card .name {
    color: var(--color-text-muted);
  }

  .card-head {
    display: flex;
    align-items: flex-start;
  }

  .card-title {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    padding: 6px 4px 6px 10px;
    border: none;
    background: transparent;
    text-align: left;
    cursor: pointer;
  }

  .name {
    overflow: hidden;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .condition {
    overflow: hidden;
    color: var(--color-text-muted);
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .flags {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 4px 4px 0 0;
    color: var(--color-text-muted);
  }

  .flags > span {
    display: inline-flex;
  }

  .chev {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border: none;
    border-radius: 3px;
    background: transparent;
    color: var(--color-text);
    cursor: pointer;
  }

  .chev:hover {
    background: var(--color-hover);
  }

  .details {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 3px 10px;
    margin: 0;
    padding: 4px 10px 8px;
    border-top: 1px solid var(--color-border);
    font-size: 12px;
  }

  dt {
    color: var(--color-text-muted);
  }

  dd {
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .mono {
    font-family: var(--font-mono);
    font-size: 11px;
  }

  .drop {
    padding: 7px 10px;
    border: 1px dashed var(--color-border-strong);
    border-radius: 4px;
    color: var(--color-text-subtle);
    font-size: 12px;
  }

  .selection-card {
    border-left: 3px solid var(--color-brand);
  }

  .hidden-note {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 12px;
    color: var(--color-text-muted);
    font-size: 12px;
  }

  .none {
    margin: 0 0 6px;
    color: var(--color-text-subtle);
    font-size: 12px;
  }
</style>
