<script lang="ts">
  import Icon from '../Icon.svelte'
  import * as backend from '../../lib/backend'
  import type { ColumnStats, GridColumn } from '../../lib/backend'
  import { dataTypeLabel } from '../../lib/data'

  // Column statistics for the selected grid column (computed by the engine on the whole table,
  // not on the filtered rows).
  let {
    table,
    column,
    version,
    onclose,
    onparsepeople,
    onstats,
  }: {
    table: string
    column: GridColumn
    version: number
    onclose: () => void
    onparsepeople: (column: string) => void
    /** The loaded statistics (status bar: "Column: X (N distinct values)"). */
    onstats?: (stats: ColumnStats | null) => void
  } = $props()

  let stats = $state.raw<ColumnStats | null>(null)
  let error = $state<string | null>(null)
  let loading = $state(false)
  let seq = 0

  $effect(() => {
    const [t, c] = [table, column.name]
    void version
    const mine = ++seq
    loading = true
    error = null
    void backend.columnStats(t, c).then((result) => {
      if (mine !== seq) return
      loading = false
      if (result.ok) {
        stats = result.value
      } else {
        stats = null
        error = result.error
      }
      onstats?.(stats)
    })
  })

  const pct = (n: number, of: number) => (of > 0 ? ` (${((100 * n) / of).toFixed(1)}%)` : '')
</script>

<aside class="stats" aria-label={`Statistics of ${column.name}`}>
  <header>
    <Icon name="stats" size={14} />
    <h3 title={column.name}>{column.name}</h3>
    <button class="icon-btn" onclick={onclose} title="Close statistics" aria-label="Close statistics"><Icon name="close" size={12} /></button>
  </header>
  <dl>
    <dt>Type</dt>
    <dd>{dataTypeLabel(column.dataType)} <span class="muted mono">{column.dataType}</span></dd>
    {#if column.sourceName && column.sourceName !== column.name}
      <dt>Source</dt>
      <dd class="mono">{column.sourceName}</dd>
    {/if}
    {#if column.formatString}
      <dt>Format</dt>
      <dd class="mono">{column.formatString}</dd>
    {/if}
  </dl>
  {#if error}
    <p class="error">{error}</p>
  {:else if stats}
    <dl class="numbers" class:stale={loading}>
      <dt>Rows</dt>
      <dd>{stats.rows.toLocaleString()}</dd>
      <dt>Count</dt>
      <dd>{stats.count.toLocaleString()}</dd>
      <dt>Blanks</dt>
      <dd>{stats.blanks.toLocaleString()}<span class="muted">{pct(stats.blanks, stats.rows)}</span></dd>
      <dt>Distinct</dt>
      <dd>{stats.distinct.toLocaleString()}</dd>
      <dt>Min</dt>
      <dd class="value" class:blank={stats.min.isBlank} title={stats.min.text}>{stats.min.isBlank ? '(Blank)' : stats.min.text}</dd>
      <dt>Max</dt>
      <dd class="value" class:blank={stats.max.isBlank} title={stats.max.text}>{stats.max.isBlank ? '(Blank)' : stats.max.text}</dd>
    </dl>
    <p class="note muted">Whole table; the grid filter does not apply.</p>
  {:else if loading}
    <p class="muted">Computing…</p>
  {/if}
  {#if column.dataType === 'string' && column.kind !== 'derived'}
    <button class="action" onclick={() => onparsepeople(column.name)} title="Preview parsing people-picker lists into “First Last; …” names">
      <Icon name="people" size={14} /> Parse people field…
    </button>
  {/if}
</aside>

<style>
  .stats {
    display: flex;
    flex: none;
    flex-direction: column;
    gap: 8px;
    width: 240px;
    padding: 8px 12px 12px;
    overflow: auto;
    border-left: 1px solid var(--color-border);
    background: var(--color-pane);
    font-size: 12px;
  }

  header {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  h3 {
    flex: 1;
    margin: 0;
    overflow: hidden;
    font-size: 13px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  dl {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: 3px 10px;
    margin: 0;
  }

  dt {
    color: var(--color-text-muted);
  }

  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .numbers dd {
    font-variant-numeric: tabular-nums;
    text-align: right;
  }

  .numbers.stale {
    opacity: 0.55;
  }

  .value {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .blank {
    color: var(--color-text-subtle);
    font-style: italic;
  }

  .mono {
    font-family: var(--font-mono);
  }

  .note {
    margin: 0;
    font-size: 11px;
  }

  .error {
    margin: 0;
    color: var(--color-danger);
  }

  .icon-btn {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    padding: 0;
    border: none;
    border-radius: 2px;
    background: transparent;
    color: var(--color-text-muted);
    cursor: pointer;
  }

  .icon-btn:hover {
    background: var(--color-hover);
  }

  .action {
    display: inline-flex;
    align-items: center;
    align-self: flex-start;
    gap: 6px;
    padding: 4px 10px;
    border: 1px solid var(--color-border-strong);
    border-radius: 2px;
    background: var(--color-pane);
    cursor: pointer;
  }

  .action:hover {
    background: var(--color-hover);
  }
</style>
