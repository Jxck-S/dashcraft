<script lang="ts">
  import CodeView from '../CodeView.svelte'
  import Icon, { type IconName } from '../Icon.svelte'
  import type { ModelTreeDto } from '../../lib/bindings'
  import { QUERY_SECTIONS, queryItems, type QueryItem, type SelectedId } from '../../lib/model'

  // Read-only M queries, parameters and DAX expressions. M evaluation (pbi-m) arrives in M2.
  let {
    tree,
    selected,
    onselect,
  }: { tree: ModelTreeDto | null; selected: SelectedId; onselect: (id: number) => void } = $props()

  const items = $derived(tree ? queryItems(tree) : [])
  const sections = $derived(
    QUERY_SECTIONS.map((section) => ({ section, items: items.filter((i) => i.section === section) })).filter(
      (s) => s.items.length > 0,
    ),
  )
  let filter = $state('')
  const q = $derived(filter.trim().toLowerCase())
  const visible = (item: QueryItem) => !q || item.name.toLowerCase().includes(q) || item.code.toLowerCase().includes(q)
  const current = $derived(items.find((i) => i.id === selected) ?? items[0] ?? null)

  const ICONS: Record<QueryItem['section'], IconName> = {
    'M queries': 'query',
    Parameters: 'parameter',
    'Measures (DAX)': 'measure',
    'Calculated columns (DAX)': 'calcColumn',
    'Calculated tables (DAX)': 'calcTable',
    Other: 'queries',
  }
</script>

<div class="queries-view">
  <nav class="query-list" aria-label="Queries and expressions">
    <label class="search">
      <Icon name="search" size={14} />
      <input type="search" placeholder="Filter by name or text" bind:value={filter} aria-label="Filter queries" />
    </label>
    {#if items.length === 0}
      <p class="empty-hint">{tree ? 'No M queries or DAX expressions.' : 'Open a file to see its queries.'}</p>
    {/if}
    {#each sections as { section, items: sectionItems } (section)}
      {@const shown = sectionItems.filter(visible)}
      {#if shown.length > 0}
        <h3>{section} <span class="muted">[{shown.length}]</span></h3>
        {#each shown as item (item.id)}
          <button class:active={current?.id === item.id} onclick={() => onselect(item.id)} title={item.subtitle}>
            <Icon name={ICONS[item.section]} size={14} />
            <span>{item.name}</span>
          </button>
        {/each}
      {/if}
    {/each}
  </nav>
  <section class="query-body">
    {#if current}
      <header>
        <h3>{current.name}</h3>
        <span class="muted">{current.subtitle}</span>
      </header>
      <div class="code">
        {#key current.id}
          <CodeView text={current.code} language={current.language} />
        {/key}
      </div>
    {:else}
      <p class="empty-hint">Nothing selected.</p>
    {/if}
  </section>
</div>

<style>
  .queries-view {
    display: grid;
    grid-template-columns: 250px minmax(0, 1fr);
    height: 100%;
    background: var(--color-pane);
  }

  .query-list {
    display: flex;
    flex-direction: column;
    min-height: 0;
    padding-bottom: 12px;
    border-right: 1px solid var(--color-border);
    background: var(--color-chrome);
    overflow: auto;
  }

  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 10px 10px 4px;
    padding: 3px 8px;
    border: 1px solid var(--color-border-strong);
    border-radius: 2px;
    background: var(--color-pane);
    color: var(--color-text-muted);
  }

  .search input {
    flex: 1;
    min-width: 0;
    border: none;
    outline: none;
    background: transparent;
  }

  h3 {
    margin: 0;
    padding: 10px 12px 4px;
    font-size: 12px;
  }

  .query-list button {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 12px;
    border: none;
    background: transparent;
    text-align: left;
    cursor: pointer;
  }

  .query-list button span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .query-list button:hover {
    background: var(--color-hover);
  }

  .query-list button.active {
    background: var(--color-selected);
    font-weight: 600;
  }

  .query-body {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }

  .query-body header {
    padding: 4px 12px 8px;
  }

  .query-body header h3 {
    padding: 6px 0 2px;
    font-size: 14px;
  }

  .code {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
    margin: 0 12px 12px;
  }
</style>
