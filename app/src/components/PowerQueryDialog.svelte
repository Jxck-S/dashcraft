<script lang="ts">
  import { untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import QueriesView from './views/QueriesView.svelte'
  import type { ModelTreeDto } from '../lib/bindings'
  import { NOT_AVAILABLE } from '../lib/shell'

  // Home → Transform data: a read-only stand-in for Desktop's Power Query Editor window — the
  // M queries, parameters and DAX expressions of the model with their code.
  let {
    tree,
    initial,
    onclose,
  }: {
    tree: ModelTreeDto
    /** Object to show first (a parameter for Edit parameters). */
    initial: number | null
    onclose: () => void
  } = $props()

  let selected = $state<number | null>(untrack(() => initial))
  let root = $state<HTMLDivElement | null>(null)

  $effect(() => {
    root?.focus()
  })

  function keydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.stopPropagation()
      onclose()
    }
  }
</script>

<div class="pq" role="dialog" aria-modal="true" aria-label="Power Query Editor (read-only)" tabindex="-1" bind:this={root} onkeydown={keydown}>
  <header>
    <Icon name="transformData" size={18} />
    <h2>Power Query Editor <span class="muted">— read-only</span></h2>
    <span class="note">Queries are evaluated offline by DashCraft's M engine on Refresh; editing is {NOT_AVAILABLE.toLowerCase()}.</span>
    <button class="close" onclick={onclose} title="Close (Esc)" aria-label="Close"><Icon name="close" size={16} /></button>
  </header>
  <div class="body">
    <QueriesView {tree} {selected} onselect={(id) => (selected = id)} />
  </div>
</div>

<style>
  .pq {
    position: fixed;
    inset: 24px;
    z-index: 190;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--color-border-strong);
    border-radius: 6px;
    background: var(--color-pane);
    box-shadow: 0 16px 48px rgb(0 0 0 / 0.28);
    outline: none;
  }

  header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px 8px 14px;
    border-bottom: 1px solid var(--color-border);
    background: var(--color-chrome);
    border-radius: 6px 6px 0 0;
  }

  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }

  .note {
    flex: 1;
    overflow: hidden;
    color: var(--color-text-muted);
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .close {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border: none;
    border-radius: 4px;
    background: transparent;
    cursor: pointer;
  }

  .close:hover {
    background: var(--color-hover);
  }

  .body {
    flex: 1;
    min-height: 0;
  }
</style>
