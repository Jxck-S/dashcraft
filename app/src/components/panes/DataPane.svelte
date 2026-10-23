<script module lang="ts">
  import type { TableDto } from '../../lib/bindings'
  import type { TreeField } from '../../lib/model'

  /** What a Data pane context menu is about. */
  export type DataTarget = { kind: 'table'; table: TableDto } | { kind: 'field'; table: TableDto; field: TreeField }

  /** Key of a field for `used`: lower-cased `table`, NUL, `field`. */
  export function fieldKey(table: string, field: string): string {
    return `${table.toLowerCase()}\u0000${field.toLowerCase()}`
  }
</script>

<script lang="ts">
  import { untrack } from 'svelte'
  import Icon, { type IconName } from '../Icon.svelte'
  import Menu from '../shell/Menu.svelte'
  import type { ModelTreeDto, TableStatus } from '../../lib/backend'
  import { compactCount, dataTypeLabel, tableState, tableTooltip } from '../../lib/data'
  import { nameTooltip, treeTables, type SelectedId } from '../../lib/model'
  import type { MenuItem } from '../../lib/shell'

  // Data pane (Desktop's field list): search, tables with their measures, columns and
  // hierarchies sorted by name, Desktop field icons (Σ numeric, calculator measure, fx
  // calculated column, calendar date, globe geography, hierarchy), hidden objects greyed (shown
  // with "View hidden"), the fields of the selected visual checked, table data chips, and
  // context menus (refresh, CSV, quick queries).
  let {
    tree,
    selected,
    onselect,
    statuses = [],
    used = null,
    values = null,
    current = null,
    contextItems,
    initialShowHidden = false,
  }: {
    tree: ModelTreeDto | null
    selected: SelectedId
    onselect: (id: number) => void
    statuses?: TableStatus[]
    /** Fields of the selected visual (Report view): shown with a checked box. `null`: no boxes. */
    used?: Set<string> | null
    /** Measure ID → evaluated value, for tooltips. */
    values?: Map<number, string> | null
    /** Table shown in Table view (highlighted). */
    current?: string | null
    contextItems?: (target: DataTarget) => MenuItem[]
    /** Start with hidden objects shown (Model view), as Desktop does. */
    initialShowHidden?: boolean
  } = $props()

  const statusByName = $derived(new Map(statuses.map((s) => [s.name, s])))

  let filter = $state('')
  let showHidden = $state(untrack(() => initialShowHidden))
  let expanded = $state<Record<number, boolean>>({})
  let menu = $state<{ items: MenuItem[]; x: number; y: number } | null>(null)

  const rows = $derived(tree ? treeTables(tree, filter, showHidden) : [])
  const searching = $derived(filter.trim() !== '')
  const startExpanded = $derived((tree?.tables.length ?? 0) <= 4)
  const hiddenCount = $derived(
    tree ? tree.tables.reduce((n, t) => n + (t.hidden ? 1 : 0) + t.columns.filter((c) => c.hidden && c.kind !== 'rowNumber').length + t.measures.filter((m) => m.hidden).length, 0) : 0,
  )

  function isOpen(table: TableDto): boolean {
    return searching || (expanded[table.id] ?? startExpanded)
  }

  function toggle(table: TableDto) {
    expanded[table.id] = !isOpen(table)
  }

  function setAll(open: boolean) {
    if (!tree) return
    expanded = Object.fromEntries(tree.tables.map((t) => [t.id, open]))
  }

  function tableIcon(table: TableDto): IconName {
    if (table.kind === 'calculatedTable') return 'calcTable'
    if (table.kind === 'remote') return 'remote'
    return 'table'
  }

  const GEO = new Set(['address', 'city', 'continent', 'country', 'county', 'latitude', 'longitude', 'place', 'postalcode', 'stateorprovince'])
  const NUMERIC = new Set(['int64', 'double', 'decimal'])

  function fieldIcon(f: TreeField): IconName | null {
    if (f.kind === 'measure') return 'measure'
    if (f.kind === 'hierarchy') return 'hierarchy'
    const c = f.item
    if (c.kind === 'calculated') return 'calcColumn'
    if (c.dataCategory && GEO.has(c.dataCategory.toLowerCase())) return 'geo'
    if (c.dataType === 'dateTime') return 'date'
    if (NUMERIC.has(c.dataType) && c.summarizeBy?.toLowerCase() !== 'none') return 'numeric'
    return null
  }

  function fieldTooltip(f: TreeField): string {
    switch (f.kind) {
      case 'measure': {
        const value = values?.get(f.item.id)
        return nameTooltip(
          f.item.name,
          null,
          'Measure (DAX)',
          value !== undefined ? `Value: ${value || '(Blank)'}` : null,
          f.item.formatString ? `Format: ${f.item.formatString}` : null,
          f.item.description,
        )
      }
      case 'hierarchy':
        return nameTooltip(f.item.name, null, `Hierarchy: ${f.item.levels.join(' › ')}`)
      case 'column':
        return nameTooltip(
          f.item.name,
          f.item.sourceColumn,
          `${f.item.kind === 'calculated' ? 'Calculated column (DAX)' : 'Column'} · ${dataTypeLabel(f.item.dataType)}`,
          f.item.description,
        )
    }
  }

  function openMenu(event: MouseEvent, target: DataTarget) {
    if (!contextItems) return
    event.preventDefault()
    const items = contextItems(target)
    if (items.length > 0) menu = { items, x: event.clientX, y: event.clientY }
  }

  function moreMenu(event: MouseEvent) {
    const r = (event.currentTarget as HTMLElement).getBoundingClientRect()
    menu = {
      x: r.left,
      y: r.bottom + 2,
      items: [
        { label: 'View hidden', checked: showHidden, title: `${hiddenCount} hidden object(s)`, onclick: () => (showHidden = !showHidden) },
        { separator: true, label: '' },
        { label: 'Expand all', icon: 'chevronDown', onclick: () => setAll(true) },
        { label: 'Collapse all', icon: 'chevron', onclick: () => setAll(false) },
      ],
    }
  }
</script>

<div class="data-pane">
  <div class="search-row">
    <label class="search">
      <Icon name="search" size={14} />
      <input type="search" placeholder="Search" bind:value={filter} aria-label="Search tables and fields" />
    </label>
    <button class="more" onclick={moreMenu} title="More options (View hidden, expand / collapse)" aria-label="More options"><Icon name="more" size={16} /></button>
  </div>

  <div class="tree-scroll">
    {#if !tree}
      <p class="empty-hint">Open a file to see its tables, columns and measures.</p>
    {:else if tree.status.availability === 'remote'}
      <p class="notice" title={tree.status.description ?? ''}>
        <Icon name="remote" />
        <span><strong>Remote model</strong> — not available offline. The report is bound to a semantic model in the Power BI service.</span>
      </p>
    {:else if tree.status.availability === 'missing'}
      <p class="empty-hint">No semantic model in this file.</p>
    {:else if rows.length === 0}
      <p class="empty-hint">{searching ? `No fields match “${filter}”.` : 'The model has no tables.'}</p>
    {/if}

    {#if tree && rows.length > 0}
      <ul class="tree" role="tree" aria-label={tree.name ?? 'Model'}>
        {#each rows as { table, fields } (table.id)}
          {@const open = isOpen(table)}
          {@const status = statusByName.get(table.name)}
          <li role="treeitem" aria-expanded={open} aria-selected={selected === table.id}>
            <div
              class="row table-row"
              class:selected={selected === table.id}
              class:current={current === table.name}
              class:hidden-item={table.hidden}
              role="presentation"
              oncontextmenu={(e) => openMenu(e, { kind: 'table', table })}
            >
              <button class="twisty" class:open onclick={() => toggle(table)} aria-label={open ? 'Collapse' : 'Expand'}>
                <Icon name="chevron" size={12} />
              </button>
              <button class="label" title={nameTooltip(table.name, null, table.kind === 'calculatedTable' ? 'Calculated table (DAX)' : table.kind === 'remote' ? 'Remote table (not available offline)' : 'Table', table.description)} onclick={() => onselect(table.id)} ondblclick={() => toggle(table)}>
                <Icon name={tableIcon(table)} size={16} />
                <span class="name">{table.name}</span>
                {#if table.hidden}<Icon name="hidden" size={12} title="Hidden" />{/if}
              </button>
              {#if status}
                {@const dataState = tableState(status)}
                <span class="status {dataState}" title={tableTooltip(status)}>
                  {#if dataState === 'error'}<Icon name="error" size={11} />{:else if dataState === 'partial' || dataState === 'skipped'}<Icon name="warning" size={11} />{/if}
                  {dataState === 'notLoaded' ? '—' : compactCount(status.rows)}
                </span>
              {/if}
            </div>
            {#if open}
              <ul role="group">
                {#each fields as f (f.item.id)}
                  {@const icon = fieldIcon(f)}
                  {@const isUsed = used?.has(fieldKey(table.name, f.item.name)) ?? false}
                  <li role="treeitem" aria-selected={selected === f.item.id}>
                    <button
                      class="row label leaf"
                      class:selected={selected === f.item.id}
                      class:hidden-item={f.item.hidden}
                      title={fieldTooltip(f)}
                      onclick={() => onselect(f.item.id)}
                      oncontextmenu={(e) => openMenu(e, { kind: 'field', table, field: f })}
                    >
                      {#if used}
                        <span class="box" class:checked={isUsed} aria-hidden="true">{#if isUsed}<Icon name="check" size={10} />{/if}</span>
                      {/if}
                      <span class="ficon">{#if icon}<Icon name={icon} size={16} />{/if}</span>
                      <span class="name">{f.item.name}</span>
                      {#if f.item.hidden}<Icon name="hidden" size={12} title="Hidden" />{/if}
                    </button>
                  </li>
                {/each}
                {#if fields.length === 0}
                  <li class="empty-leaf muted">No fields</li>
                {/if}
              </ul>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</div>

{#if menu}
  <Menu items={menu.items} x={menu.x} y={menu.y} onclose={() => (menu = null)} />
{/if}

<style>
  .data-pane {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  .search-row {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 8px 6px 12px;
  }

  .search {
    display: flex;
    flex: 1;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 8px;
    border: 1px solid var(--color-border-strong);
    border-radius: 4px;
    color: var(--color-text-muted);
    background: var(--color-pane);
  }

  .search input {
    flex: 1;
    min-width: 0;
    border: none;
    outline: none;
    background: transparent;
    color: var(--color-text);
  }

  .search:focus-within {
    border-color: var(--color-brand);
  }

  .more {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--color-text-muted);
    cursor: pointer;
  }

  .more:hover {
    background: var(--color-hover);
  }

  .tree-scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 0 0 12px;
  }

  .notice {
    display: flex;
    gap: 8px;
    margin: 12px;
    padding: 8px 10px;
    border: 1px solid var(--color-warning-border);
    border-radius: 4px;
    background: var(--color-warning-bar);
    font-size: 12px;
  }

  .tree,
  .tree ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .row {
    display: flex;
    align-items: center;
    width: 100%;
    min-height: 26px;
    border: none;
    background: transparent;
    text-align: left;
    cursor: default;
  }

  .row:hover {
    background: var(--color-hover);
  }

  .row.selected {
    background: var(--color-selected);
  }

  .row.current {
    box-shadow: inset 3px 0 0 var(--color-brand);
  }

  .twisty {
    display: grid;
    place-items: center;
    width: 22px;
    height: 26px;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--color-text-muted);
    cursor: pointer;
  }

  .twisty :global(svg) {
    transition: transform 120ms ease;
  }

  .twisty.open :global(svg) {
    transform: rotate(90deg);
  }

  .label {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 0;
    height: 26px;
    padding: 0 8px 0 0;
    border: none;
    background: transparent;
    font-size: 13px;
    text-align: left;
    cursor: pointer;
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .table-row .label {
    font-weight: 600;
  }

  .leaf {
    padding-left: 24px;
  }

  .box {
    display: grid;
    flex: none;
    place-items: center;
    width: 14px;
    height: 14px;
    border: 1px solid var(--color-text-muted);
    border-radius: 2px;
    background: var(--color-pane);
    color: #fff;
  }

  .box.checked {
    border-color: var(--color-brand);
    background: var(--color-brand);
  }

  .ficon {
    display: inline-flex;
    flex: none;
    width: 16px;
    color: var(--color-text-muted);
  }

  .empty-leaf {
    padding: 2px 0 4px 46px;
    font-size: 12px;
    font-style: italic;
  }

  .hidden-item {
    color: var(--color-text-subtle);
  }

  .status {
    display: inline-flex;
    flex: none;
    align-items: center;
    gap: 2px;
    margin-right: 8px;
    padding: 0 6px;
    border: 1px solid var(--color-border);
    border-radius: 9px;
    background: var(--color-chrome);
    color: var(--color-text-muted);
    font-size: 10px;
    font-variant-numeric: tabular-nums;
    font-weight: 400;
    line-height: 15px;
    cursor: default;
  }

  .status.ok {
    border-color: #9fd4c7;
    background: var(--color-brand-soft);
    color: var(--color-brand);
  }

  .status.error {
    border-color: var(--color-danger);
    background: var(--color-danger-bg);
    color: var(--color-danger);
  }

  .status.partial,
  .status.skipped {
    border-color: var(--color-warning-border);
    background: var(--color-warning-bar);
    color: var(--color-text);
  }
</style>
