<script lang="ts">
  import Icon from './Icon.svelte'
  import Modal from './Modal.svelte'
  import * as backend from '../lib/backend'
  import type { CsvLoad } from '../lib/backend'
  import { baseName } from '../lib/format'

  // "Load CSV manually…": stand-in data for a table whose source cannot be refreshed offline.
  let {
    table,
    reason,
    onclose,
    onloaded,
  }: {
    table: string
    /** Why the table needs it (the refresh error), if known. */
    reason?: string | null
    onclose: () => void
    onloaded: (result: CsvLoad) => void
  } = $props()

  const DELIMITERS = [
    { value: ',', label: 'Comma (,)' },
    { value: ';', label: 'Semicolon (;)' },
    { value: '\t', label: 'Tab' },
    { value: '|', label: 'Pipe (|)' },
    { value: 'other', label: 'Other…' },
  ]
  const ENCODINGS = [
    { value: '', label: 'Detect (BOM, UTF-8, else Windows-1252)' },
    { value: 'utf-8', label: 'UTF-8' },
    { value: 'windows-1252', label: 'Windows-1252' },
    { value: 'utf-16le', label: 'UTF-16 LE' },
  ]

  let path = $state('')
  let delimiter = $state(',')
  let otherDelimiter = $state('')
  let hasHeader = $state(true)
  let encoding = $state('')
  let loading = $state(false)
  let error = $state<string | null>(null)

  const effectiveDelimiter = $derived(delimiter === 'other' ? otherDelimiter : delimiter)

  async function choose() {
    try {
      const picked = await backend.pickFile(`CSV for ${table}`, [{ name: 'CSV / text', extensions: ['csv', 'tsv', 'txt'] }])
      if (picked) {
        path = picked
        if (picked.toLowerCase().endsWith('.tsv')) delimiter = '\t'
      }
    } catch (e) {
      error = `Open dialog failed: ${String(e)}`
    }
  }

  async function load() {
    loading = true
    error = null
    const result = await backend.loadCsv(table, path, { delimiter: effectiveDelimiter, hasHeader, encoding: encoding || null })
    loading = false
    if (result.ok) onloaded(result.value)
    else error = result.error
  }
</script>

<Modal title={`Load CSV manually — ${table}`} {onclose}>
  {#if reason}
    <p class="reason" title={reason}><Icon name="warning" size={14} /> {reason}</p>
  {/if}
  <p class="muted intro">
    Columns are matched by name (the model's source column, then the column name, case-insensitive), or by position
    without a header. Values that do not fit the column type load as blank. Calculated columns and tables that depend on
    the table are recomputed. The file is only read.
  </p>
  <div class="form">
    <span class="label">File</span>
    <div class="file">
      <span class="path mono" title={path}>{path ? baseName(path) : 'No file chosen'}</span>
      <button onclick={choose} disabled={!backend.hasBackend}><Icon name="csv" size={14} /> Choose…</button>
    </div>
    <label class="label" for="csv-delimiter">Delimiter</label>
    <div class="row">
      <select id="csv-delimiter" bind:value={delimiter}>
        {#each DELIMITERS as d (d.value)}
          <option value={d.value}>{d.label}</option>
        {/each}
      </select>
      {#if delimiter === 'other'}
        <input class="other mono" maxlength="1" bind:value={otherDelimiter} aria-label="Delimiter character" placeholder="e.g. ^" />
      {/if}
    </div>
    <span class="label">Header</span>
    <label class="check"><input type="checkbox" bind:checked={hasHeader} /> First row holds column names</label>
    <label class="label" for="csv-encoding">Encoding</label>
    <select id="csv-encoding" bind:value={encoding}>
      {#each ENCODINGS as e (e.value)}
        <option value={e.value}>{e.label}</option>
      {/each}
    </select>
  </div>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
  {#snippet footer()}
    <button onclick={onclose}>Cancel</button>
    <button class="primary" onclick={load} disabled={!path || !effectiveDelimiter || loading}>{loading ? 'Loading…' : 'Load'}</button>
  {/snippet}
</Modal>

<style>
  .reason {
    display: flex;
    gap: 6px;
    margin: 0 0 8px;
    padding: 6px 8px;
    border: 1px solid var(--color-warning-border);
    border-radius: 2px;
    background: var(--color-warning-bg);
    font-size: 12px;
    overflow-wrap: anywhere;
  }

  .intro {
    margin: 0 0 12px;
    font-size: 12px;
  }

  .form {
    display: grid;
    grid-template-columns: 90px minmax(0, 1fr);
    align-items: center;
    gap: 10px 12px;
  }

  .label {
    color: var(--color-text-muted);
  }

  .file,
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }

  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .file button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border: 1px solid var(--color-border-strong);
    border-radius: 2px;
    background: var(--color-pane);
    cursor: pointer;
  }

  select {
    font: inherit;
  }

  .other {
    width: 48px;
    text-align: center;
  }

  .check {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .mono {
    font-family: var(--font-mono);
  }

  .error {
    margin: 12px 0 0;
    color: var(--color-danger);
    overflow-wrap: anywhere;
  }
</style>
