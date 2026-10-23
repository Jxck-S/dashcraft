<script lang="ts">
  import Icon from '../Icon.svelte'
  import Menu from './Menu.svelte'
  import { tick } from 'svelte'
  import type { MenuItem, RibbonButton, RibbonGroup, RibbonTab } from '../../lib/shell'

  // Desktop-style ribbon: File (opens the backstage) + view-dependent tabs (contextual tabs in
  // the accent color), groups of large / small buttons with their label underneath, and the
  // collapse chevron. Collapsed, a tab click shows the body floating over the content. When the
  // window is too narrow, groups collapse from the right into one dropdown button each, as
  // Desktop's ribbon does.
  let {
    tabs,
    active,
    collapsed,
    onselect,
    onfile,
    ontogglecollapse,
  }: {
    tabs: RibbonTab[]
    active: string
    collapsed: boolean
    onselect: (id: string) => void
    onfile: () => void
    ontogglecollapse: () => void
  } = $props()

  let floating = $state(false)
  let menu = $state<{ items: MenuItem[]; x: number; y: number; label: string } | null>(null)
  let root = $state<HTMLDivElement | null>(null)

  const tab = $derived(tabs.find((t) => t.id === active) ?? tabs[0] ?? null)
  const showBody = $derived(!collapsed || floating)

  // Group widths measured while expanded, per tab and group; the compacted groups.
  const COMPACT_WIDTH = 74
  const RESERVED = 44
  let body = $state<HTMLDivElement | null>(null)
  let bodyWidth = $state(0)
  let natural = $state<Record<string, number>>({})
  const key = (g: RibbonGroup) => `${tab?.id}:${g.id}:${g.controls.length}`
  const compact = $derived.by(() => {
    const set = new Set<string>()
    if (!tab || bodyWidth <= 0) return set
    const groups = tab.groups
    if (groups.some((g) => natural[key(g)] === undefined)) return set
    let total = groups.reduce((sum, g) => sum + natural[key(g)], 0)
    for (let i = groups.length - 1; i >= 0 && total > bodyWidth - RESERVED; i--) {
      total += COMPACT_WIDTH - natural[key(groups[i])]
      set.add(groups[i].id)
    }
    return set
  })

  // Measure the groups rendered expanded (after each render of the body).
  $effect(() => {
    void tab
    void compact
    void showBody
    void tick().then(() => {
      if (!body || !tab) return
      const next = { ...natural }
      let changed = false
      for (const el of body.querySelectorAll<HTMLElement>('.group[data-group]')) {
        if (el.dataset.compact === 'true') continue
        const g = tab.groups.find((x) => x.id === el.dataset.group)
        if (!g) continue
        const w = el.offsetWidth
        if (next[key(g)] !== w) {
          next[key(g)] = w
          changed = true
        }
      }
      if (changed) natural = next
    })
  })

  /** The menu of a compacted group: its buttons (and their own menus), read-only fields. */
  function groupMenu(g: RibbonGroup): MenuItem[] {
    const items: MenuItem[] = []
    const add = (b: RibbonButton) => {
      if (b.menu && !b.split) {
        items.push({ header: true, label: b.label }, ...b.menu)
      } else {
        items.push({ label: b.label, icon: b.icon, enabled: b.enabled, title: b.title, checked: b.pressed, onclick: b.onclick })
        if (b.menu) items.push(...b.menu.map((m) => ({ ...m })))
      }
    }
    for (const c of g.controls) {
      if (c.kind === 'button') add(c)
      else if (c.kind === 'stack') c.items.forEach(add)
      else if (c.kind === 'field') items.push({ label: `${c.label}: ${c.value}`, icon: c.icon, enabled: false, title: 'Read-only' })
      else items.push({ label: `Theme: ${c.theme?.customTheme ?? c.theme?.name ?? c.theme?.baseTheme ?? 'Default'}`, icon: 'themeGallery', enabled: false })
    }
    return items
  }

  function firstIcon(g: RibbonGroup) {
    for (const c of g.controls) {
      if (c.kind === 'button') return c.icon
      if (c.kind === 'stack' && c.items[0]) return c.items[0].icon
    }
    return 'more' as const
  }

  function openGroupMenu(g: RibbonGroup, event: MouseEvent) {
    const r = (event.currentTarget as HTMLElement).getBoundingClientRect()
    menu = { items: groupMenu(g), x: r.left, y: r.bottom + 2, label: g.label }
  }

  function selectTab(id: string) {
    onselect(id)
    if (collapsed) floating = !(floating && id === active)
  }

  function openMenu(b: RibbonButton, anchor: HTMLElement) {
    if (!b.menu) return
    const r = anchor.getBoundingClientRect()
    menu = { items: b.menu, x: r.left, y: r.bottom + 2, label: b.label }
  }

  function press(b: RibbonButton, event: MouseEvent) {
    if (!b.enabled) return
    const anchor = event.currentTarget as HTMLElement
    if (b.menu && !b.split) {
      openMenu(b, anchor.closest('.btn-wrap') ?? anchor)
      return
    }
    b.onclick?.()
    floating = false
  }

  function closeFloating(event: MouseEvent) {
    if (floating && root && !root.contains(event.target as Node)) floating = false
  }

  function tabKeydown(event: KeyboardEvent) {
    if (event.key !== 'ArrowRight' && event.key !== 'ArrowLeft') return
    const ids = tabs.map((t) => t.id)
    const i = ids.indexOf(active)
    const next = ids[(i + (event.key === 'ArrowRight' ? 1 : ids.length - 1)) % ids.length]
    if (next) onselect(next)
  }

  $effect(() => {
    if (!collapsed) floating = false
  })
</script>

<svelte:window onmousedown={closeFloating} />

{#snippet ribbonButton(b: RibbonButton)}
  {@const menuEnabled = !!b.menu && b.menu.some((m) => m.enabled !== false && !m.header && !m.separator)}
  {#if b.split && b.menu && b.size === 'large'}
    <!-- Large split button, as in Desktop: the icon runs the command, the label opens the menu. -->
    <span class="btn-wrap large split">
      <button class="btn split-top" disabled={!b.enabled} title={b.title ?? b.label} onclick={(e) => press(b, e)} aria-label={b.label}>
        <Icon name={b.icon} size={32} />
      </button>
      <button
        class="btn split-bottom"
        disabled={!menuEnabled}
        title={`${b.label} options`}
        aria-haspopup="menu"
        onclick={(e) => openMenu(b, (e.currentTarget as HTMLElement).closest('.btn-wrap') as HTMLElement)}
      >
        <span class="label">{b.label}<span class="caret"><Icon name="chevronDown" size={10} /></span></span>
      </button>
    </span>
  {:else}
    <span class="btn-wrap {b.size}" class:split={b.split && b.menu}>
      <button
        class="btn {b.size}"
        class:pressed={b.pressed}
        disabled={!b.enabled}
        title={b.title ?? b.label}
        aria-pressed={b.pressed === undefined ? undefined : b.pressed}
        aria-haspopup={b.menu && !b.split ? 'menu' : undefined}
        onclick={(e) => press(b, e)}
      >
        <Icon name={b.icon} size={b.size === 'large' ? 32 : 16} />
        <span class="label">
          {b.label}{#if b.menu && !b.split}<span class="caret"><Icon name="chevronDown" size={10} /></span>{/if}
        </span>
      </button>
      {#if b.split && b.menu}
        <button
          class="arrow {b.size}"
          disabled={!menuEnabled}
          title={`${b.label} options`}
          aria-haspopup="menu"
          aria-label={`${b.label} options`}
          onclick={(e) => openMenu(b, (e.currentTarget as HTMLElement).closest('.btn-wrap') as HTMLElement)}
        >
          <Icon name="chevronDown" size={10} />
        </button>
      {/if}
    </span>
  {/if}
{/snippet}

<div class="ribbon" class:collapsed bind:this={root}>
  <div class="tabs" role="tablist" aria-label="Ribbon" tabindex="-1" onkeydown={tabKeydown}>
    <button class="tab file" onclick={onfile} title="File: open, recent files, options and about (backstage)">File</button>
    {#each tabs as t (t.id)}
      <button
        role="tab"
        class="tab"
        class:active={showBody && t.id === tab?.id}
        class:selected={t.id === tab?.id}
        class:contextual={t.contextual}
        aria-selected={t.id === tab?.id}
        onclick={() => selectTab(t.id)}
        ondblclick={ontogglecollapse}
      >
        {t.label}
      </button>
    {/each}
  </div>

  {#if showBody && tab}
    <div class="body" class:floating role="tabpanel" aria-label={tab.label} bind:this={body} bind:clientWidth={bodyWidth}>
      {#each tab.groups as g (g.id)}
        {#if compact.has(g.id)}
          <div class="group" role="group" aria-label={g.label} data-group={g.id} data-compact="true">
            <div class="controls">
              <span class="btn-wrap large">
                <button class="btn large" onclick={(e) => openGroupMenu(g, e)} title={`${g.label} (more commands)`} aria-haspopup="menu">
                  <Icon name={firstIcon(g)} size={32} />
                  <span class="label">{g.label}<span class="caret"><Icon name="chevronDown" size={10} /></span></span>
                </button>
              </span>
            </div>
            <div class="group-label">{g.label}</div>
          </div>
        {:else}
        <div class="group" role="group" aria-label={g.label} data-group={g.id}>
          <div class="controls">
            {#each g.controls as c (c.id)}
              {#if c.kind === 'button'}
                {@render ribbonButton(c)}
              {:else if c.kind === 'stack'}
                <div class="stack">
                  {#each c.items as b (b.id)}
                    {@render ribbonButton(b)}
                  {/each}
                </div>
              {:else if c.kind === 'field'}
                <label class="field" title={c.title ?? `${c.label}: ${c.value} (read-only)`}>
                  {#if c.icon}<Icon name={c.icon} size={16} />{/if}
                  <span class="field-label">{c.label}</span>
                  <input value={c.value} readonly style:width={`${c.width ?? 150}px`} aria-label={c.label} />
                </label>
              {:else if c.kind === 'themes'}
                {@const colors = c.theme?.dataColors ?? []}
                <div class="themes" title={c.theme ? `Current theme: ${c.theme.name ?? c.theme.customTheme ?? c.theme.baseTheme ?? 'default'} (read-only)` : 'No report'}>
                  <div class="theme-tile selected" style:background={c.theme?.background ?? '#ffffff'} style:color={c.theme?.foreground ?? '#252423'}>
                    <span class="aa">Aa</span>
                    <span class="bars">
                      {#each (colors.length > 0 ? colors : ['#c8c8c8', '#b0b0b0', '#989898', '#808080', '#686868', '#505050']).slice(0, 6) as color, i (i)}
                        <span style:background={color} style:height={`${40 + ((i * 37) % 55)}%`}></span>
                      {/each}
                    </span>
                  </div>
                  <span class="theme-name">{c.theme?.customTheme ?? c.theme?.name ?? c.theme?.baseTheme ?? 'Default'}{c.theme?.customTheme ? ' (custom)' : ''}</span>
                </div>
              {/if}
            {/each}
          </div>
          <div class="group-label">{g.label}</div>
        </div>
        {/if}
      {/each}
      <button class="collapse" onclick={ontogglecollapse} title={collapsed ? 'Pin the ribbon (Ctrl+F1)' : 'Collapse the ribbon (Ctrl+F1)'} aria-label={collapsed ? 'Pin the ribbon' : 'Collapse the ribbon'}>
        <Icon name={collapsed ? 'chevronDown' : 'chevronUp'} size={14} />
      </button>
    </div>
  {/if}
</div>

{#if menu}
  <Menu items={menu.items} x={menu.x} y={menu.y} label={menu.label} onclose={() => (menu = null)} />
{/if}

<style>
  .ribbon {
    position: relative;
    z-index: 40;
    flex: none;
    background: var(--color-chrome);
  }

  .tabs {
    display: flex;
    align-items: stretch;
    gap: 2px;
    height: var(--ribbon-tabs-height);
    padding: 0 8px;
    background: var(--color-chrome);
  }

  .tab {
    position: relative;
    padding: 0 12px;
    border: none;
    background: transparent;
    color: var(--color-text);
    font-size: 14px;
    cursor: pointer;
  }

  .tab:hover {
    background: var(--color-hover);
  }

  .tab.active::after,
  .collapsed .tab.selected.active::after {
    content: '';
    position: absolute;
    right: 12px;
    bottom: 3px;
    left: 12px;
    height: 3px;
    border-radius: 2px;
    background: var(--color-brand);
  }

  .tab.selected {
    font-weight: 600;
  }

  .tab.contextual {
    color: var(--color-brand);
  }

  .tab.file {
    padding: 0 14px;
  }

  .body {
    position: relative;
    display: flex;
    align-items: stretch;
    height: var(--ribbon-body-height);
    padding: 4px 32px 0 6px;
    overflow: hidden;
    border-top: 1px solid var(--color-border);
    border-bottom: 1px solid var(--color-border);
    background: var(--color-ribbon);
  }

  .body.floating {
    position: absolute;
    top: var(--ribbon-tabs-height);
    right: 0;
    left: 0;
    box-shadow: 0 6px 12px rgb(0 0 0 / 0.14);
  }

  .group {
    display: flex;
    flex: none;
    flex-direction: column;
    padding: 0 6px;
    border-right: 1px solid var(--color-border);
  }

  .controls {
    display: flex;
    flex: 1;
    align-items: flex-start;
    gap: 2px;
    min-height: 0;
  }

  .group-label {
    padding: 2px 0 4px;
    color: var(--color-text-muted);
    font-size: 11px;
    text-align: center;
    white-space: nowrap;
  }

  .stack {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .btn-wrap {
    display: inline-flex;
    border: 1px solid transparent;
    border-radius: 4px;
  }

  .btn-wrap.large {
    flex-direction: column;
  }

  .btn-wrap.split:hover:has(button:not(:disabled)) {
    border-color: var(--color-border);
  }

  .btn {
    display: flex;
    align-items: center;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--color-text);
    cursor: pointer;
  }

  .btn.large {
    flex-direction: column;
    justify-content: flex-start;
    gap: 2px;
    min-width: 46px;
    max-width: 84px;
    height: 66px;
    padding: 3px 4px 0;
    font-size: 12px;
    line-height: 1.2;
  }

  .btn.small {
    gap: 6px;
    height: 22px;
    padding: 0 6px 0 4px;
    font-size: 12px;
    white-space: nowrap;
  }

  .btn:hover:not(:disabled),
  .arrow:hover:not(:disabled) {
    background: var(--color-hover);
  }

  .btn:active:not(:disabled),
  .arrow:active:not(:disabled) {
    background: var(--color-pressed);
  }

  .btn.pressed {
    background: var(--color-selected);
    box-shadow: inset 0 -2px 0 var(--color-brand);
  }

  .btn:disabled,
  .arrow:disabled {
    color: var(--color-text-disabled);
    cursor: default;
  }

  .btn.large .label {
    display: -webkit-box;
    overflow: hidden;
    text-align: center;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }

  .caret {
    display: inline-flex;
    margin-left: 2px;
    vertical-align: middle;
  }

  .arrow {
    display: grid;
    place-items: center;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--color-text);
    cursor: pointer;
  }

  .split-top {
    justify-content: center;
    min-width: 54px;
    height: 36px;
    padding-top: 3px;
    border-radius: 4px 4px 0 0;
  }

  .split-bottom {
    justify-content: center;
    max-width: 84px;
    height: 30px;
    padding: 0 4px;
    border-radius: 0 0 4px 4px;
    font-size: 12px;
    line-height: 1.2;
  }

  .split-bottom .label {
    display: -webkit-box;
    overflow: hidden;
    text-align: center;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }

  .arrow.small {
    width: 16px;
  }

  .field {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 24px;
    margin: 2px 4px;
    color: var(--color-text-muted);
    font-size: 12px;
  }

  .controls:has(.field) {
    flex-direction: column;
    flex-wrap: wrap;
    max-height: 70px;
    align-content: flex-start;
  }

  .field-label {
    min-width: 72px;
    white-space: nowrap;
  }

  .field input {
    height: 22px;
    padding: 0 6px;
    border: 1px solid var(--color-border-strong);
    border-radius: 3px;
    background: var(--color-chrome);
    color: var(--color-text);
    font-size: 12px;
  }

  .themes {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 3px;
    padding: 2px 6px 0;
  }

  .theme-tile {
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    width: 74px;
    height: 46px;
    padding: 2px 5px 3px;
    border: 1px solid var(--color-border-strong);
    border-radius: 3px;
  }

  .theme-tile.selected {
    outline: 2px solid var(--color-brand);
    outline-offset: 1px;
  }

  .aa {
    font-size: 11px;
    line-height: 1;
  }

  .bars {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    height: 24px;
  }

  .bars span {
    flex: 1;
    border-radius: 1px 1px 0 0;
  }

  .theme-name {
    max-width: 100px;
    overflow: hidden;
    color: var(--color-text-muted);
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .collapse {
    position: absolute;
    right: 6px;
    bottom: 4px;
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border: none;
    border-radius: 3px;
    background: transparent;
    color: var(--color-text-muted);
    cursor: pointer;
  }

  .collapse:hover {
    background: var(--color-hover);
  }
</style>
