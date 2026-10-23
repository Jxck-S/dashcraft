<script lang="ts">
  import Icon from '../Icon.svelte'
  import type { PageDto, VisualDto } from '../../lib/bindings'
  import { visualKind } from '../../lib/visuals'

  // Selection pane (read-only): the page's visuals and groups in layer order (top first), with
  // their visibility; click to select. "Tab order" lists them in reading order instead.
  let { page, selectedId, onselect }: { page: PageDto | null; selectedId: number | null; onselect: (id: number) => void } = $props()

  let order = $state<'layer' | 'tab'>('layer')

  const label = (v: VisualDto) => v.title ?? v.fields[0]?.displayName ?? visualKind(v.visualType).label

  interface Row {
    visual: VisualDto
    depth: number
  }

  const rows = $derived.by((): Row[] => {
    if (!page) return []
    const sorted = [...page.visuals].sort((a, b) =>
      order === 'layer' ? b.position.z - a.position.z : a.position.y - b.position.y || a.position.x - b.position.x,
    )
    const byGroup = new Map<string, VisualDto[]>()
    for (const v of sorted) if (v.parentGroup) byGroup.set(v.parentGroup, [...(byGroup.get(v.parentGroup) ?? []), v])
    const out: Row[] = []
    const seen = new Set<number>()
    const add = (v: VisualDto, depth: number) => {
      if (seen.has(v.id) || depth > 16) return
      seen.add(v.id)
      out.push({ visual: v, depth })
      if (v.isGroup) for (const child of byGroup.get(v.name) ?? []) add(child, depth + 1)
    }
    const names = new Set(page.visuals.map((v) => v.name))
    for (const v of sorted) if (!v.parentGroup || !names.has(v.parentGroup)) add(v, 0)
    for (const v of sorted) add(v, 0)
    return out
  })
</script>

<div class="selection">
  <div class="subtabs" role="tablist">
    <button role="tab" aria-selected={order === 'layer'} class:active={order === 'layer'} onclick={() => (order = 'layer')}>Layer order</button>
    <button role="tab" aria-selected={order === 'tab'} class:active={order === 'tab'} onclick={() => (order = 'tab')}>Tab order</button>
  </div>
  {#if rows.length === 0}
    <p class="empty-hint">{page ? 'No visuals on this page.' : 'No page.'}</p>
  {/if}
  <ul>
    {#each rows as { visual: v, depth } (v.id)}
      <li>
        <button class="row" class:selected={selectedId === v.id} class:hidden-row={v.hidden} style:padding-left={`${8 + depth * 16}px`} onclick={() => onselect(v.id)} title={`${label(v)} (${v.visualType})`}>
          <Icon name={visualKind(v.visualType).icon} size={16} />
          <span class="name">{label(v)}</span>
          <span class="eye" title={v.hidden ? 'Hidden' : 'Visible'}><Icon name={v.hidden ? 'hidden' : 'eye'} size={14} /></span>
        </button>
      </li>
    {/each}
  </ul>
</div>

<style>
  .selection {
    padding: 0 0 12px;
  }

  .subtabs {
    display: flex;
    gap: 4px;
    padding: 0 12px 6px;
    border-bottom: 1px solid var(--color-border);
  }

  .subtabs button {
    position: relative;
    padding: 4px 8px;
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
    right: 6px;
    bottom: -6px;
    left: 6px;
    height: 2px;
    background: var(--color-brand);
  }

  ul {
    margin: 4px 0 0;
    padding: 0;
    list-style: none;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 28px;
    padding-right: 8px;
    border: none;
    background: transparent;
    text-align: left;
    cursor: pointer;
  }

  .row:hover {
    background: var(--color-hover);
  }

  .row.selected {
    background: var(--color-selected);
  }

  .hidden-row {
    color: var(--color-text-subtle);
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .eye {
    display: inline-flex;
    color: var(--color-text-muted);
  }
</style>
