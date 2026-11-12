<script lang="ts">
  import Icon from '../Icon.svelte'
  import type { BookmarkOut } from '../../lib/backend'

  // Bookmarks pane in reading view: the report's bookmarks and groups in pane order; a click
  // applies one, View starts the slideshow. Adding and updating bookmarks is authoring (M20.3).
  let {
    bookmarks,
    error = null,
    active = null,
    onapply,
    onview,
  }: {
    bookmarks: BookmarkOut[] | null
    error?: string | null
    active?: string | null
    onapply: (name: string) => void
    onview: () => void
  } = $props()

  interface Section {
    group: string | null
    items: BookmarkOut[]
  }

  // Consecutive bookmarks of the same group form one section, as in the pane.
  const sections = $derived.by((): Section[] => {
    const out: Section[] = []
    for (const b of bookmarks ?? []) {
      const group = b.group ?? null
      const last = out.at(-1)
      if (last && group != null && last.group === group) last.items.push(b)
      else out.push({ group, items: [b] })
    }
    return out
  })

  let closed = $state<Record<string, boolean>>({})

  function detail(b: BookmarkOut): string {
    const parts = [b.pageDisplayName ? `Shows ${b.pageDisplayName}` : 'Keeps the current page']
    const kinds = [b.data ? 'Data' : null, b.display ? 'Display' : null].filter(Boolean)
    parts.push(kinds.length > 0 ? kinds.join(' and ') : 'Neither data nor display')
    if (b.targetVisuals.length > 0) parts.push(`${b.targetVisuals.length} selected visual${b.targetVisuals.length === 1 ? '' : 's'} only`)
    return parts.join(' · ')
  }
</script>

<div class="bookmarks">
  <div class="toolbar">
    <button class="view" onclick={onview} disabled={!bookmarks || bookmarks.length === 0} title="Show the bookmarks one after the other, as a slideshow">
      <Icon name="play" size={14} />
      <span>View</span>
    </button>
  </div>
  {#if error}
    <p class="empty-hint" role="alert">{error}</p>
  {:else if bookmarks == null}
    <p class="empty-hint">Loading bookmarks…</p>
  {:else if bookmarks.length === 0}
    <p class="empty-hint">This report has no bookmarks.</p>
  {/if}
  <ul aria-label="Bookmarks">
    {#each sections as section, i (i)}
      {#if section.group != null}
        {@const key = `${i}:${section.group}`}
        <li>
          <button class="row group" aria-expanded={!closed[key]} onclick={() => (closed = { ...closed, [key]: !closed[key] })} title={`Bookmark group ${section.group}`}>
            <Icon name={closed[key] ? 'chevron' : 'chevronDown'} size={12} />
            <Icon name="folder" size={16} />
            <span class="name">{section.group}</span>
          </button>
          {#if !closed[key]}
            <ul aria-label={section.group}>
              {#each section.items as b (b.name)}
                <li>{@render row(b, 1)}</li>
              {/each}
            </ul>
          {/if}
        </li>
      {:else}
        {#each section.items as b (b.name)}
          <li>{@render row(b, 0)}</li>
        {/each}
      {/if}
    {/each}
  </ul>
</div>

{#snippet row(b: BookmarkOut, depth: number)}
  <button class="row" class:selected={active === b.name} aria-current={active === b.name ? 'true' : undefined} style:padding-left={`${8 + depth * 16}px`} onclick={() => onapply(b.name)} title={detail(b)}>
    <Icon name="bookmarks" size={16} />
    <span class="name">{b.displayName}</span>
  </button>
{/snippet}

<style>
  .bookmarks {
    padding: 0 0 12px;
  }

  .toolbar {
    display: flex;
    gap: 4px;
    padding: 0 12px 6px;
    border-bottom: 1px solid var(--color-border);
  }

  .view {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 4px 8px;
    border: 1px solid var(--color-border);
    border-radius: 4px;
    background: var(--color-surface, transparent);
    cursor: pointer;
  }

  .view:disabled {
    color: var(--color-text-subtle);
    cursor: default;
  }

  ul {
    margin: 4px 0 0;
    padding: 0;
    list-style: none;
  }

  ul ul {
    margin: 0;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 28px;
    padding-right: 8px;
    padding-left: 8px;
    border: none;
    background: transparent;
    text-align: left;
    cursor: pointer;
  }

  .row.group {
    gap: 4px;
    font-weight: 600;
  }

  .row:hover {
    background: var(--color-hover);
  }

  .row.selected {
    background: var(--color-selected);
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
