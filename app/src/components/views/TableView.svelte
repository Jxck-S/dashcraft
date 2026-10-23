<script module lang="ts">
  import type { GridColumn } from '../../lib/backend'

  /** What the Table view shows, for the status bar. */
  export interface TableInfo {
    totalRows: number
    filtered: boolean
    column: GridColumn | null
    distinct: number | null
  }
</script>

<script lang="ts">
  import { untrack } from 'svelte'
  import Icon from '../Icon.svelte'
  import ColumnStatsPanel from './ColumnStatsPanel.svelte'
  import DataGrid, { type GridInfo } from './DataGrid.svelte'
  import PeopleDialog from './PeopleDialog.svelte'
  import * as backend from '../../lib/backend'
  import type { Sort, TableDto, TableStatus } from '../../lib/backend'
  import { csvFallback, quoteIdent, statusHint, statusMessage } from '../../lib/data'
  import { nameTooltip, type SelectedId } from '../../lib/model'

  // Table view: the selected table's rows in a virtualized grid (paged, sorted and filtered by
  // the backend), column statistics, the "Parse people field" transform (column menu), and the
  // schema. The table is chosen in the Data pane; Refresh table, Load CSV and Rows / Schema are
  // on the Table tools ribbon tab.
  let {
    tables,
    table,
    statuses,
    version,
    refreshing,
    selected,
    mode,
    onselect,
    onloadcsv,
    ondatachanged,
    onerror,
    oninfo,
    parsePeople = $bindable(null),
  }: {
    tables: TableDto[]
    table: TableDto | null
    statuses: TableStatus[]
    /** Bumped whenever table data changes. */
    version: number
    refreshing: boolean
    selected: SelectedId
    mode: 'grid' | 'schema'
    onselect: (id: number) => void
    onloadcsv: (table: string, reason: string | null) => void
    /** A derived column was added or removed (statuses and grid need reloading). */
    ondatachanged: () => void
    onerror: (message: string) => void
    oninfo: (info: TableInfo) => void
    /** Set by the parent (Column tools → Parse people field…) to open the dialog. */
    parsePeople?: string | null
  } = $props()

  let sort = $state<Sort | null>(null)
  let filterInput = $state('')
  let appliedFilter = $state('')
  let helpOpen = $state(false)
  let info = $state.raw<GridInfo>({ columns: [], totalRows: 0, loading: false, error: null })
  let selectedColumn = $state<string | null>(null)
  let distinct = $state<number | null>(null)
  let menu = $state<{ column: GridColumn; x: number; y: number } | null>(null)
  let peopleColumn = $state<string | null>(null)

  $effect(() => {
    const requested = parsePeople
    if (requested) {
      untrack(() => {
        peopleColumn = requested
        parsePeople = null
      })
    }
  })

  $effect(() => {
    const column = info.columns.find((c) => c.name === selectedColumn) ?? null
    const next: TableInfo = { totalRows: info.totalRows, filtered: appliedFilter !== '', column, distinct: column ? distinct : null }
    untrack(() => oninfo(next))
  })

  const status = $derived(table ? (statuses.find((s) => s.name === table.name) ?? null) : null)
  const lastStatus = $derived(status?.lastRefresh?.status ?? null)
  const statsColumn = $derived(info.columns.find((c) => c.name === selectedColumn) ?? null)

  const tableName = $derived(table?.name ?? '')

  // A different table starts unsorted and unfiltered.
  $effect(() => {
    void tableName
    untrack(() => {
      sort = null
      filterInput = ''
      appliedFilter = ''
      selectedColumn = null
      distinct = null
      menu = null
    })
  })

  // Selecting a column anywhere (Data pane, diagnostics) shows its statistics.
  $effect(() => {
    const id = selected
    untrack(() => {
      const column = table?.columns.find((c) => c.id === id)
      if (column) selectedColumn = column.name
    })
  })

  function cycleSort(column: string) {
    if (sort?.column !== column) sort = { column, ascending: true }
    else if (sort.ascending) sort = { column, ascending: false }
    else sort = null
  }

  function selectColumn(c: GridColumn) {
    selectedColumn = c.name
    if (c.id != null) onselect(c.id)
  }

  function applyFilter() {
    appliedFilter = filterInput.trim()
  }

  function clearFilter() {
    filterInput = ''
    appliedFilter = ''
  }

  function filterKey(event: KeyboardEvent) {
    if (event.key === 'Enter') applyFilter()
    else if (event.key === 'Escape') clearFilter()
  }

  function insertColumn(name: string) {
    const ident = quoteIdent(name)
    filterInput = filterInput.trim() ? `${filterInput.trimEnd()} AND ${ident} ` : `${ident} `
  }

  function menuAction(action: () => void) {
    // The action may read the menu's column, so it runs before the menu closes.
    action()
    menu = null
  }

  async function removeDerived(column: string) {
    if (!table) return
    const result = await backend.removeDerivedColumn(table.name, column)
    if (!result.ok) {
      onerror(result.error)
      return
    }
    if (selectedColumn === column) selectedColumn = null
    if (sort?.column === column) sort = null
    ondatachanged()
  }

  function closeMenu(event: MouseEvent | KeyboardEvent) {
    if (!menu) return
    if (event instanceof KeyboardEvent ? event.key === 'Escape' : !(event.target as Element | null)?.closest('.column-menu')) menu = null
  }

  const KIND: Record<string, string> = {
    data: 'Data',
    calculated: 'Calculated (DAX)',
    calculatedTableColumn: 'Calculated table',
    rowNumber: 'Row number',
  }
</script>

<svelte:window onmousedown={closeMenu} onkeydown={closeMenu} />

<div class="table-view">
  {#if !table}
    <p class="empty-hint">{tables.length === 0 ? 'No tables to show.' : 'Select a table in the Data pane.'}</p>
  {:else}
    {#if table.kind === 'remote'}
      <p class="banner warn"><Icon name="remote" size={14} /> <span>Remote table: its data lives in the Power BI service and is not available offline.</span></p>
    {:else if lastStatus && lastStatus.state !== 'ok'}
      <div class="banner" class:error={lastStatus.state === 'error'} class:warn={lastStatus.state === 'skipped'} role="status">
        <Icon name={lastStatus.state === 'error' ? 'error' : 'warning'} size={14} />
        <span class="banner-text">
          <strong>{lastStatus.state === 'error' ? 'Refresh failed' : 'Not refreshed'}:</strong>
          {statusMessage(lastStatus)}
          {#if statusHint(lastStatus)}<span class="hint">{statusHint(lastStatus)}</span>{/if}
          {#if status?.loaded}<span class="hint">Showing the data loaded earlier.</span>{/if}
        </span>
        {#if csvFallback(lastStatus) && table}
          {@const name = table.name}
          <button class="tool" onclick={() => onloadcsv(name, statusMessage(lastStatus))} disabled={refreshing}><Icon name="csv" size={14} /> Load CSV manually…</button>
        {/if}
      </div>
    {:else if status && !status.loaded}
      <p class="banner">
        <Icon name="info" size={14} />
        <span>No data loaded yet. Use <strong>Refresh</strong> to run the table's queries{status.canLoadCsv ? ', or load a CSV' : ''}.</span>
      </p>
    {/if}

    {#if mode === 'grid'}
      <div class="filter-bar" title="DashCraft: filter the rows with a SQL WHERE expression">
        <Icon name="filter" size={14} />
        <input
          class="filter mono"
          class:invalid={info.error != null && appliedFilter !== ''}
          type="text"
          spellcheck="false"
          autocomplete="off"
          placeholder={'SQL filter, e.g. "Region" = \'West\''}
          bind:value={filterInput}
          onkeydown={filterKey}
          aria-label="SQL filter (WHERE expression)"
          aria-describedby="filter-help"
        />
        <button class="tool" onclick={applyFilter} disabled={filterInput.trim() === appliedFilter} title="Apply the filter (Enter)">Apply</button>
        {#if appliedFilter || filterInput}
          <button class="icon-btn" onclick={clearFilter} title="Clear the filter (Esc)" aria-label="Clear the filter"><Icon name="close" size={12} /></button>
        {/if}
        <button class="icon-btn" class:active={helpOpen} onclick={() => (helpOpen = !helpOpen)} title="Filter syntax" aria-label="Filter syntax" aria-expanded={helpOpen}>
          <Icon name="info" size={14} />
        </button>
        <span class="count muted" aria-live="polite">
          {#if info.loading}<span class="spinner" aria-hidden="true"></span>{/if}
          {info.totalRows.toLocaleString()} {appliedFilter ? 'matching rows' : 'rows'}
        </span>
      </div>
      <p id="filter-help" class="help" class:open={helpOpen}>
        {#if helpOpen}
          A SQL <code>WHERE</code> expression over this table's columns. Quote column names in double quotes
          (<code>"Product Name"</code>), text in single quotes (<code>'West'</code>). Examples:
          <code>"Amount" &gt; 100</code> · <code>"Product Name" LIKE 'A%'</code> · <code>"Owner" IS NULL</code> ·
          <code>lower("Status") = 'done' AND "Year" IN (2025, 2026)</code>. Insert a column:
          {#each info.columns as c (c.name)}
            <button class="col-chip" onclick={() => insertColumn(c.name)}>{c.name}</button>
          {/each}
        {/if}
      </p>
      {#if info.error}
        <p class="filter-error" role="alert"><Icon name="error" size={14} /> {info.error}</p>
      {/if}
      <div class="grid-area">
        <div class="grid-wrap">
          <DataGrid
            table={table.name}
            {sort}
            filter={appliedFilter}
            {version}
            {selectedColumn}
            onsort={cycleSort}
            onselectcolumn={selectColumn}
            oncolumnmenu={(column, x, y) => (menu = { column, x, y })}
            oninfo={(i) => (info = i)}
          />
        </div>
        {#if statsColumn}
          <ColumnStatsPanel
            table={table.name}
            column={statsColumn}
            {version}
            onclose={() => (selectedColumn = null)}
            onparsepeople={(c) => (peopleColumn = c)}
            onstats={(st) => (distinct = st?.distinct ?? null)}
          />
        {/if}
      </div>
    {:else}
      <div class="grid-scroll">
        <table class="schema">
          <thead>
            <tr>
              <th scope="col">Column</th>
              <th scope="col">Data type</th>
              <th scope="col">Kind</th>
              <th scope="col">Source column</th>
              <th scope="col">Format</th>
              <th scope="col">Summarize by</th>
              <th scope="col">Hidden</th>
            </tr>
          </thead>
          <tbody>
            {#each table.columns as c (c.id)}
              <tr class:selected={selected === c.id} class:hidden-row={c.hidden}>
                <td>
                  <button class="link" onclick={() => onselect(c.id)} title={nameTooltip(c.name, c.sourceColumn, c.description)}>
                    {c.name}
                  </button>
                </td>
                <td class="mono">{c.dataType}</td>
                <td>{KIND[c.kind] ?? c.kind}</td>
                <td class="mono" class:differs={c.sourceColumn != null && c.sourceColumn !== c.name}>{c.sourceColumn ?? ''}</td>
                <td class="mono">{c.formatString ?? ''}</td>
                <td class="mono">{c.summarizeBy ?? ''}</td>
                <td>{c.hidden ? 'Yes' : ''}</td>
              </tr>
            {/each}
            {#each status?.derivedColumns ?? [] as name (name)}
              <tr class="derived-row">
                <td><strong>{name}</strong></td>
                <td class="mono">string</td>
                <td>Derived (session only)</td>
                <td colspan="4" class="muted">Parse people field</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  {/if}
</div>

{#if menu && table}
  {@const c = menu.column}
  <div class="column-menu" role="menu" style:left={`${Math.min(menu.x, window.innerWidth - 230)}px`} style:top={`${Math.min(menu.y, window.innerHeight - 260)}px`}>
    <div class="menu-title" title={c.name}>{c.name}</div>
    <button role="menuitem" onclick={() => menuAction(() => (sort = { column: c.name, ascending: true }))}><Icon name="sortAsc" size={14} /> Sort ascending</button>
    <button role="menuitem" onclick={() => menuAction(() => (sort = { column: c.name, ascending: false }))}><Icon name="sortDesc" size={14} /> Sort descending</button>
    <button role="menuitem" disabled={sort?.column !== c.name} onclick={() => menuAction(() => (sort = null))}><Icon name="close" size={14} /> Clear sort</button>
    <hr />
    <button role="menuitem" onclick={() => menuAction(() => selectColumn(c))}><Icon name="stats" size={14} /> Column statistics</button>
    <button role="menuitem" onclick={() => menuAction(() => insertColumn(c.name))}><Icon name="filter" size={14} /> Filter on this column…</button>
    {#if c.dataType === 'string' && c.kind !== 'derived'}
      <button role="menuitem" onclick={() => menuAction(() => (peopleColumn = c.name))}><Icon name="people" size={14} /> Parse people field…</button>
    {/if}
    {#if c.kind === 'derived'}
      <button role="menuitem" onclick={() => menuAction(() => void removeDerived(c.name))}><Icon name="close" size={14} /> Remove derived column</button>
    {/if}
  </div>
{/if}

{#if peopleColumn && table}
  <PeopleDialog
    table={table.name}
    column={peopleColumn}
    onclose={() => (peopleColumn = null)}
    onadded={(_, column) => {
      peopleColumn = null
      selectedColumn = column
      ondatachanged()
    }}
  />
{/if}

<style>
  .table-view {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--color-pane);
  }

  .tool {
    display: inline-flex;
    flex: none;
    align-items: center;
    gap: 5px;
    height: 26px;
    padding: 0 9px;
    border: 1px solid var(--color-border-strong);
    border-radius: 2px;
    background: var(--color-pane);
    font-size: 12px;
    cursor: pointer;
  }

  .tool:hover:not(:disabled) {
    background: var(--color-hover);
  }

  .tool:disabled {
    color: var(--color-text-subtle);
    cursor: default;
  }

  .banner {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0;
    padding: 6px 12px;
    border-bottom: 1px solid var(--color-border);
    background: var(--color-info-bg);
    font-size: 12px;
  }

  .banner.error {
    background: var(--color-danger-bg);
  }

  .banner.warn {
    background: var(--color-warning-bg);
  }

  .banner-text {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .hint {
    display: block;
    color: var(--color-text-muted);
  }

  .filter-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--color-border);
    color: var(--color-text-muted);
  }

  .filter {
    flex: 1;
    min-width: 0;
    height: 26px;
    padding: 0 8px;
    border: 1px solid var(--color-border-strong);
    border-radius: 2px;
    color: var(--color-text);
    font-size: 12px;
  }

  .filter:focus {
    border-color: var(--color-brand);
    outline: none;
  }

  .filter.invalid {
    border-color: var(--color-danger);
  }

  .icon-btn {
    display: grid;
    flex: none;
    place-items: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border: none;
    border-radius: 2px;
    background: transparent;
    color: var(--color-text-muted);
    cursor: pointer;
  }

  .icon-btn:hover,
  .icon-btn.active {
    background: var(--color-hover);
  }

  .count {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 110px;
    justify-content: flex-end;
    font-size: 12px;
  }

  .spinner {
    width: 12px;
    height: 12px;
    border: 2px solid var(--color-border);
    border-top-color: var(--color-brand);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .help {
    display: none;
    margin: 0;
    padding: 6px 12px 8px 32px;
    border-bottom: 1px solid var(--color-border);
    background: var(--color-code-bg);
    font-size: 12px;
    line-height: 1.7;
  }

  .help.open {
    display: block;
  }

  .col-chip {
    margin: 0 2px;
    padding: 0 6px;
    border: 1px solid var(--color-border-strong);
    border-radius: 9px;
    background: var(--color-pane);
    font-size: 11px;
    cursor: pointer;
  }

  .filter-error {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    margin: 0;
    padding: 4px 12px;
    border-bottom: 1px solid var(--color-border);
    background: var(--color-danger-bg);
    color: var(--color-danger);
    font-size: 12px;
    overflow-wrap: anywhere;
  }

  .grid-area {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .grid-wrap {
    flex: 1;
    min-width: 0;
    min-height: 0;
  }

  .grid-scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }

  .schema {
    border-collapse: collapse;
    font-size: 12px;
    white-space: nowrap;
  }

  .schema th,
  .schema td {
    padding: 4px 10px;
    border-right: 1px solid var(--color-border);
    border-bottom: 1px solid var(--color-border);
    text-align: left;
  }

  .schema th {
    position: sticky;
    top: 0;
    background: var(--color-chrome);
    font-weight: 600;
  }

  .schema tbody tr:hover {
    background: var(--color-hover);
  }

  tr.selected {
    background: var(--color-selected);
  }

  .hidden-row {
    color: var(--color-text-subtle);
  }

  .derived-row {
    background: var(--color-info-bg);
  }

  .mono {
    font-family: var(--font-mono);
  }

  .differs {
    color: var(--color-brand);
  }

  .link {
    padding: 0;
    border: none;
    background: transparent;
    color: inherit;
    font-weight: 600;
    cursor: pointer;
  }

  .link:hover {
    text-decoration: underline;
  }

  .column-menu {
    position: fixed;
    z-index: 170;
    display: flex;
    flex-direction: column;
    min-width: 210px;
    padding: 4px 0;
    border: 1px solid var(--color-border-strong);
    border-radius: 3px;
    background: var(--color-pane);
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.16);
    font-size: 12px;
  }

  .menu-title {
    padding: 4px 12px 6px;
    overflow: hidden;
    color: var(--color-text-muted);
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .column-menu button {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 12px;
    border: none;
    background: transparent;
    text-align: left;
    cursor: pointer;
  }

  .column-menu button:hover:not(:disabled) {
    background: var(--color-hover);
  }

  .column-menu button:disabled {
    color: var(--color-text-subtle);
    cursor: default;
  }

  .column-menu hr {
    width: 100%;
    margin: 4px 0;
    border: none;
    border-top: 1px solid var(--color-border);
  }

  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation-duration: 2.4s;
    }
  }
</style>
