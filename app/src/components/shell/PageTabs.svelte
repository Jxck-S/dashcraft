<script lang="ts">
  import Icon from '../Icon.svelte'
  import type { PageDto } from '../../lib/bindings'
  import { NOT_AVAILABLE } from '../../lib/shell'

  // Page navigator under the report canvas: layout switch (desktop / mobile), previous / next
  // page, one tab per page in report order (hidden pages greyed with the hidden icon, as in
  // Desktop's editing view) and the disabled "New page" button.
  let { pages, activeId, onselect }: { pages: PageDto[]; activeId: number | null; onselect: (id: number) => void } = $props()

  const index = $derived(pages.findIndex((p) => p.id === activeId))
  let strip = $state<HTMLDivElement | null>(null)

  function go(delta: number) {
    const next = pages[index + delta]
    if (next) onselect(next.id)
  }

  $effect(() => {
    void activeId
    strip?.querySelector<HTMLElement>('.page-tab.active')?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  })
</script>

<nav class="page-tabs" aria-label="Pages">
  <div class="layouts">
    <button class="icon-btn pressed" title="Desktop layout" aria-pressed="true" aria-label="Desktop layout"><Icon name="desktop" size={16} /></button>
    <button class="icon-btn" disabled title={`Mobile layout — ${NOT_AVAILABLE}`} aria-label="Mobile layout"><Icon name="mobileLayout" size={16} /></button>
  </div>
  <div class="nav">
    <button class="icon-btn" disabled={index <= 0} onclick={() => go(-1)} title="Previous page (Ctrl+Page Up)" aria-label="Previous page"><Icon name="chevronLeft" size={14} /></button>
    <button class="icon-btn" disabled={index < 0 || index >= pages.length - 1} onclick={() => go(1)} title="Next page (Ctrl+Page Down)" aria-label="Next page"><Icon name="chevron" size={14} /></button>
  </div>
  <div class="strip" bind:this={strip} role="tablist">
    {#each pages as p (p.id)}
      <button
        role="tab"
        class="page-tab"
        class:active={p.id === activeId}
        class:hidden-page={p.hidden}
        aria-selected={p.id === activeId}
        onclick={() => onselect(p.id)}
        title={`${p.displayName}${p.hidden ? ' (hidden in reading view)' : ''}\n${Math.round(p.width)} × ${Math.round(p.height)} · ${p.visuals.length} visual${p.visuals.length === 1 ? '' : 's'}`}
      >
        {#if p.hidden}<Icon name="hidden" size={14} />{/if}
        <span>{p.displayName}</span>
      </button>
    {/each}
    <button class="new-page" disabled title={`New page — ${NOT_AVAILABLE}`} aria-label="New page"><Icon name="add" size={16} /></button>
  </div>
</nav>

<style>
  .page-tabs {
    display: flex;
    flex: none;
    align-items: stretch;
    height: var(--pagetabs-height);
    border-top: 1px solid var(--color-border);
    background: var(--color-pagetabs);
  }

  .layouts,
  .nav {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 0 6px;
    border-right: 1px solid var(--color-border);
  }

  .icon-btn {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--color-text);
    cursor: pointer;
  }

  .icon-btn:hover:not(:disabled) {
    background: var(--color-hover);
  }

  .icon-btn.pressed {
    box-shadow: inset 0 -2px 0 var(--color-brand);
  }

  .icon-btn:disabled {
    color: var(--color-text-disabled);
    cursor: default;
  }

  .strip {
    display: flex;
    flex: 1;
    min-width: 0;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
  }

  .page-tab {
    position: relative;
    display: inline-flex;
    flex: none;
    align-items: center;
    gap: 6px;
    max-width: 240px;
    padding: 0 18px;
    border: none;
    border-right: 1px solid var(--color-border);
    background: transparent;
    color: var(--color-text);
    font-size: 13px;
    white-space: nowrap;
    cursor: pointer;
  }

  .page-tab span {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .page-tab:hover {
    background: var(--color-hover);
  }

  .page-tab.active {
    background: var(--color-pane);
    font-weight: 600;
  }

  .page-tab.active::after {
    content: '';
    position: absolute;
    right: 0;
    bottom: 0;
    left: 0;
    height: 3px;
    background: var(--color-brand);
  }

  .hidden-page {
    color: var(--color-text-muted);
    font-style: italic;
  }

  .new-page {
    display: grid;
    flex: none;
    place-items: center;
    width: 40px;
    border: none;
    background: transparent;
    color: var(--color-text-disabled);
  }
</style>
