<script lang="ts">
  import { onMount } from 'svelte'
  import Icon from '../Icon.svelte'
  import type { MenuItem } from '../../lib/shell'

  // Popup menu (ribbon dropdowns, context menus): fixed at (x, y), kept inside the window,
  // closed by Esc, a click outside or choosing an item.
  let {
    items,
    x,
    y,
    onclose,
    label = 'Menu',
  }: { items: MenuItem[]; x: number; y: number; onclose: () => void; label?: string } = $props()

  let menu = $state<HTMLDivElement | null>(null)
  let left = $state(0)
  let top = $state(0)

  $effect(() => {
    const el = menu
    if (!el) return
    const rect = el.getBoundingClientRect()
    left = Math.max(4, Math.min(x, window.innerWidth - rect.width - 4))
    top = Math.max(4, Math.min(y, window.innerHeight - rect.height - 4))
  })

  onMount(() => {
    menu?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus()
    const down = (event: MouseEvent) => {
      if (menu && !menu.contains(event.target as Node)) onclose()
    }
    // Registered after the click that opened the menu has finished.
    const timer = setTimeout(() => window.addEventListener('mousedown', down, true))
    return () => {
      clearTimeout(timer)
      window.removeEventListener('mousedown', down, true)
    }
  })

  function keydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.stopPropagation()
      onclose()
      return
    }
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
    event.preventDefault()
    const buttons = [...(menu?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? [])]
    const i = buttons.indexOf(document.activeElement as HTMLButtonElement)
    const next = event.key === 'ArrowDown' ? (i + 1) % buttons.length : (i - 1 + buttons.length) % buttons.length
    buttons[next]?.focus()
  }

  /** Esc closes the menu even when no item has the focus (every item disabled). */
  function windowKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.stopPropagation()
      onclose()
    }
  }

  function choose(item: MenuItem) {
    onclose()
    item.onclick?.()
  }
</script>

<svelte:window onkeydown={windowKeydown} onresize={onclose} />

<div class="menu" role="menu" aria-label={label} tabindex="-1" bind:this={menu} style:left={`${left}px`} style:top={`${top}px`} onkeydown={keydown}>
  {#each items as item, i (i)}
    {#if item.separator}
      <hr />
    {:else if item.header}
      <div class="header">{item.label}</div>
    {:else}
      <button role="menuitem" disabled={item.enabled === false} title={item.title} onclick={() => choose(item)}>
        <span class="check">{#if item.checked}<Icon name="check" size={14} />{/if}</span>
        {#if item.icon}<Icon name={item.icon} size={16} />{:else}<span class="no-icon"></span>{/if}
        <span class="text">{item.label}</span>
      </button>
    {/if}
  {/each}
</div>

<style>
  .menu {
    position: fixed;
    z-index: 300;
    display: flex;
    flex-direction: column;
    min-width: 200px;
    max-width: 360px;
    max-height: calc(100vh - 8px);
    overflow: auto;
    padding: 4px 0;
    border: 1px solid var(--color-border-strong);
    border-radius: 4px;
    background: var(--color-pane);
    box-shadow: 0 8px 16px rgb(0 0 0 / 0.14), 0 0 2px rgb(0 0 0 / 0.12);
    font-size: 13px;
  }

  .menu:focus {
    outline: none;
  }

  button {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 30px;
    padding: 4px 14px 4px 6px;
    border: none;
    background: transparent;
    text-align: left;
    cursor: pointer;
  }

  button:hover:not(:disabled),
  button:focus-visible {
    background: var(--color-hover);
    outline: none;
  }

  button:disabled {
    color: var(--color-text-disabled);
    cursor: default;
  }

  .check {
    display: inline-flex;
    width: 14px;
    color: var(--color-brand);
  }

  .no-icon {
    width: 16px;
  }

  .text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .header {
    padding: 6px 14px 2px 44px;
    color: var(--color-text-muted);
    font-size: 12px;
    font-weight: 600;
  }

  hr {
    width: 100%;
    margin: 4px 0;
    border: none;
    border-top: 1px solid var(--color-border);
  }
</style>
