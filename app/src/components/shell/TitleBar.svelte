<script module lang="ts">
  import type { IconName } from '../../lib/icons'

  /** A title bar search result: a ribbon command or a model field. */
  export interface SearchItem {
    kind: 'command' | 'field'
    label: string
    detail?: string
    icon: IconName
    enabled: boolean
    run: () => void
  }
</script>

<script lang="ts">
  import Icon from '../Icon.svelte'
  import { NOT_AVAILABLE } from '../../lib/shell'

  // Title bar row: app mark, Quick Access Toolbar (Save / Undo / Redo: read-only viewer, so
  // disabled), the centered document title and the search box (commands and fields).
  let { title, items }: { title: string; items: SearchItem[] } = $props()

  let query = $state('')
  let open = $state(false)
  let input = $state<HTMLInputElement | null>(null)
  let active = $state(0)

  const q = $derived(query.trim().toLowerCase())
  const results = $derived.by(() => {
    if (!q) return []
    const hit = (i: SearchItem) => i.label.toLowerCase().includes(q) || (i.detail?.toLowerCase().includes(q) ?? false)
    const commands = items.filter((i) => i.kind === 'command' && hit(i)).slice(0, 8)
    const fields = items.filter((i) => i.kind === 'field' && hit(i)).slice(0, 10)
    return [...commands, ...fields]
  })

  function run(item: SearchItem) {
    if (!item.enabled) return
    item.run()
    query = ''
    open = false
    input?.blur()
  }

  function keydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      query = ''
      open = false
      input?.blur()
    } else if (event.key === 'ArrowDown') {
      event.preventDefault()
      active = Math.min(results.length - 1, active + 1)
    } else if (event.key === 'ArrowUp') {
      event.preventDefault()
      active = Math.max(0, active - 1)
    } else if (event.key === 'Enter') {
      const item = results[active]
      if (item) run(item)
    }
  }

  $effect(() => {
    void q
    active = 0
  })

  /** Alt+Q focuses the search box, as in Desktop. */
  function globalKey(event: KeyboardEvent) {
    if (event.altKey && !event.ctrlKey && !event.metaKey && event.key.toLowerCase() === 'q') {
      event.preventDefault()
      input?.focus()
    }
  }
</script>

<svelte:window onkeydown={globalKey} />

<header class="title-bar">
  <div class="left">
    <span class="mark" aria-hidden="true"></span>
    <div class="qat" role="toolbar" aria-label="Quick access toolbar">
      <button disabled title={`Save (Ctrl+S) — ${NOT_AVAILABLE}`} aria-label="Save"><Icon name="save" size={16} /></button>
      <button disabled title={`Undo (Ctrl+Z) — ${NOT_AVAILABLE}`} aria-label="Undo"><Icon name="undo" size={16} /></button>
      <button disabled title={`Redo (Ctrl+Y) — ${NOT_AVAILABLE}`} aria-label="Redo"><Icon name="redo" size={16} /></button>
    </div>
  </div>
  <div class="title" title={title}>{title}</div>
  <div class="search-wrap">
    <label class="search" class:focused={open}>
      <Icon name="search" size={16} />
      <input
        bind:this={input}
        type="search"
        placeholder="Search"
        bind:value={query}
        onfocus={() => (open = true)}
        onblur={() => setTimeout(() => (open = false), 150)}
        onkeydown={keydown}
        aria-label="Search commands and fields (Alt+Q)"
        title="Search commands and fields (Alt+Q)"
        role="combobox"
        aria-expanded={open && results.length > 0}
        aria-controls="title-search-results"
      />
    </label>
    {#if open && q}
      <div class="results" id="title-search-results" role="listbox">
        {#each results as item, i (i)}
          {#if i === 0 || results[i - 1].kind !== item.kind}
            <div class="section">{item.kind === 'command' ? 'Actions' : 'Fields'}</div>
          {/if}
          <button
            role="option"
            aria-selected={i === active}
            class:active={i === active}
            disabled={!item.enabled}
            title={item.enabled ? item.detail : NOT_AVAILABLE}
            onmousedown={(e) => e.preventDefault()}
            onclick={() => run(item)}
          >
            <Icon name={item.icon} size={16} />
            <span class="label">{item.label}</span>
            {#if item.detail}<span class="detail">{item.detail}</span>{/if}
          </button>
        {:else}
          <div class="none">No commands or fields match “{query}”.</div>
        {/each}
      </div>
    {/if}
  </div>
  <div class="right"></div>
</header>

<style>
  .title-bar {
    position: relative;
    z-index: 60;
    display: grid;
    flex: none;
    grid-template-columns: minmax(150px, 1fr) auto minmax(220px, 420px) minmax(40px, 1fr);
    align-items: center;
    gap: 16px;
    height: var(--titlebar-height);
    padding: 0 8px;
    background: var(--color-chrome);
  }

  .left {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .mark {
    width: 18px;
    height: 18px;
    margin-left: 4px;
    background: url('/favicon.svg') center / contain no-repeat;
  }

  .qat {
    display: flex;
    gap: 2px;
  }

  .qat button {
    display: grid;
    place-items: center;
    width: 28px;
    height: 26px;
    border: none;
    border-radius: 4px;
    background: transparent;
  }

  .qat button:disabled {
    color: var(--color-text-disabled);
  }

  .title {
    overflow: hidden;
    font-size: 13px;
    text-align: center;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .search-wrap {
    position: relative;
  }

  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 8px;
    border: 1px solid var(--color-border-strong);
    border-radius: 4px;
    background: var(--color-pane);
    color: var(--color-text-muted);
  }

  .search.focused {
    border-color: var(--color-brand);
  }

  .search input {
    flex: 1;
    min-width: 0;
    border: none;
    outline: none;
    background: transparent;
    color: var(--color-text);
    font-size: 13px;
  }

  .results {
    position: absolute;
    top: 30px;
    right: 0;
    left: 0;
    max-height: 420px;
    overflow: auto;
    padding: 4px 0;
    border: 1px solid var(--color-border-strong);
    border-radius: 4px;
    background: var(--color-pane);
    box-shadow: 0 8px 16px rgb(0 0 0 / 0.14);
  }

  .section {
    padding: 6px 12px 2px;
    color: var(--color-text-muted);
    font-size: 11px;
    font-weight: 600;
  }

  .results button {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 5px 12px;
    border: none;
    background: transparent;
    text-align: left;
    cursor: pointer;
  }

  .results button.active,
  .results button:hover:not(:disabled) {
    background: var(--color-hover);
  }

  .results button:disabled {
    color: var(--color-text-disabled);
    cursor: default;
  }

  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .detail {
    margin-left: auto;
    overflow: hidden;
    color: var(--color-text-subtle);
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .none {
    padding: 8px 12px;
    color: var(--color-text-subtle);
    font-style: italic;
  }
</style>
