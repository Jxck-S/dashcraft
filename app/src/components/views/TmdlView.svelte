<script lang="ts">
  import CodeView from '../CodeView.svelte'
  import Icon from '../Icon.svelte'
  import type { TmdlViewDto } from '../../lib/bindings'
  import { NOT_AVAILABLE } from '../../lib/shell'

  // TMDL view (read-only): the semantic model as TMDL scripts — the project's .tmdl files
  // verbatim, or canonical TMDL generated from the model. A script list on the left, the
  // script on the right; selecting a table in the Data pane opens its script.
  let {
    tmdl,
    error,
    loading,
    path = $bindable(),
  }: {
    tmdl: TmdlViewDto | null
    error: string | null
    loading: boolean
    /** Path of the open script (`tables/Sales.tmdl`). */
    path: string | null
  } = $props()

  let filter = $state('')
  const q = $derived(filter.trim().toLowerCase())
  const files = $derived((tmdl?.files ?? []).filter((f) => !q || f.path.toLowerCase().includes(q) || f.text.toLowerCase().includes(q)))
  const current = $derived(tmdl?.files.find((f) => f.path === path) ?? tmdl?.files.find((f) => f.path === 'model.tmdl') ?? tmdl?.files[0] ?? null)
  const folders = $derived.by(() => {
    const groups = new Map<string, typeof files>()
    for (const f of files) {
      const i = f.path.lastIndexOf('/')
      const folder = i < 0 ? '' : f.path.slice(0, i)
      groups.set(folder, [...(groups.get(folder) ?? []), f])
    }
    return [...groups.entries()]
  })
</script>

<div class="tmdl-view">
  <div class="command-bar">
    <button class="cmd" disabled title={`Apply — ${NOT_AVAILABLE}`}><Icon name="play" size={14} /> Apply</button>
    <span class="note" title={tmdl?.source === 'files' ? 'Read from the opened .tmdl files' : 'Written by DashCraft from the model (pbi-tmdl)'}>
      <Icon name="info" size={14} />
      {#if loading}Loading…
      {:else if !tmdl}—
      {:else if tmdl.source === 'files'}Read-only · the project's TMDL files, verbatim ({tmdl.files.length})
      {:else}Read-only · TMDL generated from the model by DashCraft ({tmdl.files.length} scripts){/if}
    </span>
  </div>
  {#if error}
    <p class="error" role="alert"><Icon name="warning" size={16} /> {error}</p>
  {:else if tmdl}
    <div class="body">
      <nav class="scripts" aria-label="TMDL scripts">
        <label class="search">
          <Icon name="search" size={14} />
          <input type="search" placeholder="Filter by name or text" bind:value={filter} aria-label="Filter scripts" />
        </label>
        {#each folders as [folder, list] (folder)}
          {#if folder}<h3>{folder}</h3>{/if}
          {#each list as f (f.path)}
            <button class:active={current?.path === f.path} onclick={() => (path = f.path)} title={f.path}>
              <Icon name={f.table ? 'table' : 'document'} size={14} />
              <span>{f.path.slice(f.path.lastIndexOf('/') + 1)}</span>
            </button>
          {/each}
        {:else}
          <p class="empty-hint">No script matches “{filter}”.</p>
        {/each}
      </nav>
      <section class="script">
        {#if current}
          {#key current.path}
            <CodeView text={current.text} language="text" languageLabel="TMDL" label={current.path} />
          {/key}
        {/if}
      </section>
    </div>
  {/if}
</div>

<style>
  .tmdl-view {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--color-pane);
  }

  .command-bar {
    display: flex;
    flex: none;
    align-items: center;
    gap: 10px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--color-border);
  }

  .cmd {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 26px;
    padding: 0 10px;
    border: 1px solid var(--color-border-strong);
    border-radius: 4px;
    background: var(--color-pane);
    font-size: 12px;
  }

  .cmd:disabled {
    color: var(--color-text-disabled);
  }

  .note {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--color-text-muted);
    font-size: 12px;
  }

  .body {
    display: grid;
    flex: 1;
    grid-template-columns: 220px minmax(0, 1fr);
    min-height: 0;
  }

  .scripts {
    display: flex;
    flex-direction: column;
    min-height: 0;
    padding-bottom: 12px;
    overflow: auto;
    border-right: 1px solid var(--color-border);
    background: var(--color-chrome);
  }

  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 8px 8px 4px;
    padding: 3px 8px;
    border: 1px solid var(--color-border-strong);
    border-radius: 4px;
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
    padding: 10px 12px 2px;
    color: var(--color-text-muted);
    font-size: 11px;
    font-weight: 600;
  }

  .scripts button {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 12px;
    border: none;
    background: transparent;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
  }

  .scripts button span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .scripts button:hover {
    background: var(--color-hover);
  }

  .scripts button.active {
    background: var(--color-pane);
    box-shadow: inset 3px 0 0 var(--color-brand);
    font-weight: 600;
  }

  .script {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    padding: 8px;
  }

  .script :global(.code-view) {
    flex: 1;
  }

  .error {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 24px;
    color: var(--color-text-muted);
  }
</style>
