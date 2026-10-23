<script lang="ts">
  import Icon from '../Icon.svelte'
  import type { ViewId } from '../../lib/shell'

  // Left view switcher, in Desktop order: Report, Table, Model, DAX query, TMDL.
  let { active, onselect, disabled = [] }: { active: ViewId; onselect: (id: ViewId) => void; disabled?: ViewId[] } = $props()

  const VIEWS: Array<{ id: ViewId; label: string; hint: string }> = [
    { id: 'report', label: 'Report view', hint: 'Report pages: canvas, filters, visualizations' },
    { id: 'table', label: 'Table view', hint: 'The rows of a table (data grid)' },
    { id: 'model', label: 'Model view', hint: 'Tables and relationships (diagram)' },
    { id: 'dax', label: 'DAX query view', hint: 'Run DAX queries over the loaded data' },
    { id: 'tmdl', label: 'TMDL view', hint: 'The semantic model as TMDL script (read-only)' },
  ]

  function keydown(event: KeyboardEvent) {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
    event.preventDefault()
    const enabled = VIEWS.filter((v) => !disabled.includes(v.id))
    const i = enabled.findIndex((v) => v.id === active)
    const next = enabled[(i + (event.key === 'ArrowDown' ? 1 : enabled.length - 1)) % enabled.length]
    if (next) onselect(next.id)
  }
</script>

<div class="rail" role="toolbar" aria-label="Views" aria-orientation="vertical" tabindex="-1" onkeydown={keydown}>
  {#each VIEWS as v (v.id)}
    <button
      class:active={active === v.id}
      aria-current={active === v.id ? 'page' : undefined}
      disabled={disabled.includes(v.id)}
      title={`${v.label}\n${v.hint}`}
      aria-label={v.label}
      onclick={() => onselect(v.id)}
    >
      {#if v.id === 'report'}
        <Icon name="report" size={20} />
      {:else if v.id === 'table'}
        <Icon name="data" size={20} />
      {:else if v.id === 'model'}
        <Icon name="model" size={20} />
      {:else}
        <span class="doc-icon">
          <Icon name="document" size={20} />
          <span class="tag" class:long={v.id === 'tmdl'}>{v.id === 'dax' ? 'DAX' : 'TMDL'}</span>
        </span>
      {/if}
    </button>
  {/each}
</div>

<style>
  .rail {
    display: flex;
    flex: none;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    width: var(--rail-width);
    padding-top: 8px;
    border-right: 1px solid var(--color-border);
    background: var(--color-chrome);
  }

  button {
    position: relative;
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--color-text-muted);
    cursor: pointer;
  }

  button:hover:not(:disabled) {
    background: var(--color-hover);
    color: var(--color-text);
  }

  button.active {
    color: var(--color-text);
    background: var(--color-pane);
    box-shadow: 0 0 0 1px var(--color-border);
  }

  button.active::before {
    content: '';
    position: absolute;
    top: 10px;
    bottom: 10px;
    left: -4px;
    width: 3px;
    border-radius: 2px;
    background: var(--color-brand);
  }

  button:disabled {
    color: var(--color-text-disabled);
    cursor: default;
  }

  .doc-icon {
    position: relative;
    display: inline-grid;
    place-items: center;
  }

  .tag {
    position: absolute;
    bottom: 1px;
    left: 50%;
    padding: 0 1px;
    transform: translateX(-50%);
    background: var(--color-chrome);
    font-size: 7px;
    font-weight: 700;
    letter-spacing: 0.02em;
    line-height: 8px;
  }

  button.active .tag {
    background: var(--color-pane);
  }

  .tag.long {
    font-size: 6px;
  }
</style>
