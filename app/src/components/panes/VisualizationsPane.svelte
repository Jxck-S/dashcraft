<script lang="ts">
  import Icon from '../Icon.svelte'
  import * as backend from '../../lib/backend'
  import type { FormatCardDto, PageDto, VisualDto, VisualFormatDto } from '../../lib/bindings'
  import { fieldTooltip } from '../../lib/model'
  import { NOT_AVAILABLE } from '../../lib/shell'
  import { GALLERY, galleryEntry, visualKind } from '../../lib/visuals'

  // Visualizations pane (read-only): "Build visual" shows the gallery with the selected visual's
  // type highlighted and its field wells (role → fields); "Format visual" shows its formatting
  // objects as cards (Visual / General); with no visual selected, the page's settings.
  let {
    visual,
    page,
    onselectfield,
  }: {
    visual: VisualDto | null
    page: PageDto | null
    /** A field chip was clicked: select the model object (by `Table[Field]`). */
    onselectfield?: (table: string | null, field: string | null) => void
  } = $props()

  let tab = $state<'build' | 'format'>('build')
  let formatTab = $state<'visual' | 'general'>('visual')
  let format = $state.raw<VisualFormatDto | null>(null)
  let formatError = $state<string | null>(null)
  let search = $state('')
  let openCards = $state<Record<string, boolean>>({})
  let seq = 0

  const entry = $derived(visual ? galleryEntry(visual.visualType) : null)
  const kind = $derived(visual ? visualKind(visual.visualType) : null)
  const roles = $derived.by(() => {
    if (!visual) return []
    const byRole = new Map<string, VisualDto['fields']>()
    for (const f of visual.fields) byRole.set(f.role, [...(byRole.get(f.role) ?? []), f])
    return [...byRole.entries()]
  })

  $effect(() => {
    const id = visual?.id ?? null
    const wanted = tab === 'format'
    if (id == null || !wanted) return
    const mine = ++seq
    formatError = null
    void backend.visualFormat(id).then((r) => {
      if (mine !== seq) return
      if (r.ok) format = r.value
      else {
        format = null
        formatError = r.error
      }
    })
  })

  const q = $derived(search.trim().toLowerCase())
  function cards(list: FormatCardDto[]): FormatCardDto[] {
    if (!q) return list
    return list
      .map((c) => ({
        ...c,
        entries: c.entries
          .map((e) => ({ ...e, properties: e.properties.filter((p) => c.label.toLowerCase().includes(q) || p.label.toLowerCase().includes(q)) }))
          .filter((e) => e.properties.length > 0),
      }))
      .filter((c) => c.entries.length > 0)
  }
  const shownCards = $derived(format ? cards(formatTab === 'visual' ? format.visual : format.general) : [])
</script>

<div class="viz">
  <div class="tabs" role="tablist" aria-label="Visualizations">
    <span class="tabs-label">{tab === 'build' ? 'Build visual' : 'Format visual'}</span>
    <span class="icons">
      <button role="tab" aria-selected={tab === 'build'} class:active={tab === 'build'} onclick={() => (tab = 'build')} title="Build visual" aria-label="Build visual">
        <Icon name="build" size={24} />
      </button>
      <button role="tab" aria-selected={tab === 'format'} class:active={tab === 'format'} onclick={() => (tab = 'format')} title="Format visual" aria-label="Format visual">
        <Icon name="format" size={24} />
      </button>
      <button role="tab" aria-selected="false" disabled title={`Analytics — ${NOT_AVAILABLE}`} aria-label="Analytics">
        <Icon name="analytics" size={24} />
      </button>
    </span>
  </div>

  {#if tab === 'build'}
    <div class="gallery" role="list" aria-label="Visual types">
      {#each GALLERY as g (g.label)}
        <span role="listitem" class="tile" class:current={entry === g} title={entry === g ? `${g.label} (selected visual)` : `${g.label} — changing the visual type is ${NOT_AVAILABLE.toLowerCase()}`}>
          <Icon name={g.icon} size={20} />
        </span>
      {/each}
      <span role="listitem" class="tile" title={`Get more visuals — ${NOT_AVAILABLE}`}><Icon name="more" size={20} /></span>
    </div>
    {#if visual && !entry && kind}
      <p class="other"><Icon name={kind.icon} size={16} /> <span>{kind.label}{kind.label !== visual.visualType ? ` (${visual.visualType})` : ' (custom or element)'}</span></p>
    {/if}

    <div class="wells">
      {#if !visual}
        <p class="hint">Select a visual on the canvas to see its fields.</p>
      {:else if roles.length === 0}
        <p class="hint">This visual has no data fields.</p>
      {:else}
        {#each roles as [role, fields] (role)}
          <h4>{role}</h4>
          <div class="well">
            {#each fields as f, i (i)}
              <button class="chip" class:measure={f.kind === 'measure'} title={fieldTooltip(f)} onclick={() => onselectfield?.(f.table, f.field)}>
                <span class="chip-name" class:renamed={f.renamed}>{f.displayName}</span>
                {#if f.aggregation}<span class="agg">{f.aggregation}</span>{/if}
              </button>
            {/each}
          </div>
        {/each}
      {/if}
      <h4 class="drill">Drill through</h4>
      <div class="toggle-row" title={NOT_AVAILABLE}><span>Cross-report</span><span class="toggle">Off</span></div>
      <div class="toggle-row" title={NOT_AVAILABLE}><span>Keep all filters</span><span class="toggle on">On</span></div>
      <div class="drop" title={NOT_AVAILABLE}>Add drill-through fields here</div>
    </div>
  {:else}
    <div class="format">
      <label class="search">
        <Icon name="search" size={14} />
        <input type="search" placeholder="Search" bind:value={search} aria-label="Search format settings" />
      </label>
      {#if !visual}
        {#if page}
          <h4 class="page-head">Format page</h4>
          <dl class="props">
            <dt>Page name</dt><dd>{page.displayName}</dd>
            <dt>Canvas size</dt><dd>{Math.round(page.width)} × {Math.round(page.height)}</dd>
            <dt>Page view</dt><dd>{page.displayOption}</dd>
            <dt>Hidden</dt><dd>{page.hidden ? 'Yes' : 'No'}</dd>
            <dt>Visuals</dt><dd>{page.visuals.length}</dd>
          </dl>
        {:else}
          <p class="hint">Select a visual on the canvas.</p>
        {/if}
      {:else}
        <div class="subtabs" role="tablist">
          <button role="tab" aria-selected={formatTab === 'visual'} class:active={formatTab === 'visual'} onclick={() => (formatTab = 'visual')}>Visual</button>
          <button role="tab" aria-selected={formatTab === 'general'} class:active={formatTab === 'general'} onclick={() => (formatTab = 'general')}>General</button>
        </div>
        {#if formatError}
          <p class="error">{formatError}</p>
        {:else if !format}
          <p class="hint">Loading…</p>
        {:else}
          {#if formatTab === 'general'}
            <details class="card" open>
              <summary>Properties</summary>
              <dl class="props">
                {#each format.layout as p (p.label)}
                  <dt>{p.label}</dt><dd>{p.value}</dd>
                {/each}
              </dl>
            </details>
          {/if}
          {#each shownCards as c (c.name)}
            <details class="card" open={q !== '' || openCards[c.name]} ontoggle={(e) => (openCards[c.name] = (e.currentTarget as HTMLDetailsElement).open)}>
              <summary title={c.name}>{c.label}</summary>
              {#each c.entries as e, i (i)}
                {#if e.selector}<div class="selector" title="Applies to">{e.selector}</div>{/if}
                <dl class="props">
                  {#each e.properties as p, j (j)}
                    <dt title={p.tooltip ?? ''}>{p.label}</dt>
                    <dd>
                      {#if /^#[0-9A-Fa-f]{6}$/.test(p.value.replace(/^'|'$/g, ''))}<span class="swatch" style:background={p.value.replace(/^'|'$/g, '')}></span>{/if}
                      {p.value}
                    </dd>
                  {/each}
                </dl>
              {/each}
            </details>
          {:else}
            <p class="hint">{q ? 'No matching settings.' : formatTab === 'visual' ? 'No visual formatting set: the theme defaults apply.' : 'No general formatting set.'}</p>
          {/each}
        {/if}
      {/if}
    </div>
  {/if}
</div>

<style>
  .viz {
    padding: 0 0 16px;
  }

  .tabs {
    padding: 0 12px;
    border-bottom: 1px solid var(--color-border);
  }

  .tabs-label {
    display: block;
    font-size: 13px;
    font-weight: 600;
  }

  .icons {
    display: flex;
    gap: 8px;
    padding: 4px 0 0;
  }

  .icons button {
    position: relative;
    display: grid;
    place-items: center;
    width: 40px;
    height: 36px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--color-text);
    cursor: pointer;
  }

  .icons button:hover:not(:disabled) {
    background: var(--color-hover);
  }

  .icons button.active {
    background: var(--color-selected);
  }

  .icons button.active::after {
    content: '';
    position: absolute;
    right: 8px;
    bottom: -1px;
    left: 8px;
    height: 3px;
    border-radius: 2px;
    background: var(--color-brand);
  }

  .icons button:disabled {
    color: var(--color-text-disabled);
    cursor: default;
  }

  .gallery {
    display: grid;
    grid-template-columns: repeat(6, 1fr);
    gap: 2px;
    padding: 8px 10px;
    border-bottom: 1px solid var(--color-border);
  }

  .tile {
    display: grid;
    place-items: center;
    height: 30px;
    border-radius: 3px;
    color: var(--color-text-muted);
  }

  .tile.current {
    background: var(--color-brand-soft);
    box-shadow: inset 0 0 0 1px var(--color-brand);
    color: var(--color-brand);
  }

  .other {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 6px 12px 0;
    color: var(--color-text-muted);
    font-size: 12px;
  }

  .wells {
    padding: 4px 12px;
  }

  h4 {
    margin: 10px 0 4px;
    font-size: 12px;
    font-weight: 600;
  }

  .well {
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: 4px;
    border: 1px solid var(--color-border);
    border-radius: 4px;
  }

  .chip {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 8px;
    border: 1px solid var(--color-border);
    border-radius: 3px;
    background: var(--color-chrome);
    font-size: 12px;
    text-align: left;
    cursor: pointer;
  }

  .chip:hover {
    border-color: var(--color-brand);
  }

  .chip.measure {
    border-left: 3px solid var(--color-accent);
  }

  .chip-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .renamed {
    text-decoration: underline dotted var(--color-text-subtle);
  }

  .agg {
    color: var(--color-text-subtle);
    font-size: 11px;
  }

  .drill {
    margin-top: 16px;
  }

  .toggle-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 3px 0;
    color: var(--color-text-muted);
    font-size: 12px;
  }

  .toggle {
    padding: 0 8px;
    border: 1px solid var(--color-border-strong);
    border-radius: 9px;
    color: var(--color-text-subtle);
    font-size: 11px;
  }

  .toggle.on {
    border-color: var(--color-text-subtle);
    background: var(--color-text-subtle);
    color: #fff;
  }

  .drop {
    margin-top: 4px;
    padding: 7px 10px;
    border: 1px dashed var(--color-border-strong);
    border-radius: 4px;
    color: var(--color-text-subtle);
    font-size: 12px;
  }

  .hint {
    margin: 10px 0;
    color: var(--color-text-subtle);
    font-size: 12px;
    font-style: italic;
  }

  .format {
    padding: 8px 12px;
  }

  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
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

  .subtabs {
    display: flex;
    gap: 4px;
    margin: 8px 0 4px;
  }

  .subtabs button {
    position: relative;
    padding: 4px 10px;
    border: none;
    background: transparent;
    cursor: pointer;
  }

  .subtabs button.active {
    font-weight: 600;
  }

  .subtabs button.active::after {
    content: '';
    position: absolute;
    right: 8px;
    bottom: 0;
    left: 8px;
    height: 2px;
    background: var(--color-brand);
  }

  .card {
    border-bottom: 1px solid var(--color-border);
  }

  summary {
    padding: 8px 2px;
    font-weight: 600;
    cursor: pointer;
  }

  .selector {
    margin: 2px 0 2px 8px;
    color: var(--color-text-muted);
    font-size: 11px;
    font-style: italic;
    overflow-wrap: anywhere;
  }

  .props {
    display: grid;
    grid-template-columns: minmax(70px, max-content) 1fr;
    gap: 4px 10px;
    margin: 0;
    padding: 2px 4px 10px 8px;
    font-size: 12px;
  }

  dt {
    color: var(--color-text-muted);
  }

  dd {
    display: flex;
    align-items: center;
    gap: 4px;
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .swatch {
    flex: none;
    width: 12px;
    height: 12px;
    border: 1px solid var(--color-border-strong);
    border-radius: 2px;
  }

  .page-head {
    margin-top: 12px;
  }

  .error {
    color: var(--color-danger);
    font-size: 12px;
  }
</style>
