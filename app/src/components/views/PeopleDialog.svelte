<script lang="ts">
  import Modal from '../Modal.svelte'
  import * as backend from '../../lib/backend'
  import type { PeoplePreview } from '../../lib/backend'
  import type { DerivedColumnOut } from '../../lib/ops'

  // "Parse people field": preview the transform on a text column, then add the parsed column
  // (session only; the source files are never changed).
  let {
    table,
    column,
    onclose,
    onadded,
  }: {
    table: string
    column: string
    onclose: () => void
    onadded: (column: DerivedColumnOut, name: string) => void
  } = $props()

  let preview = $state.raw<PeoplePreview | null>(null)
  let error = $state<string | null>(null)
  let adding = $state(false)

  $effect(() => {
    const [t, c] = [table, column]
    void backend.peoplePreview(t, c, 50).then((result) => {
      if (result.ok) preview = result.value
      else error = result.error
    })
  })

  const OUTCOME: Record<string, string> = {
    names: 'Parsed',
    unparsed: 'Kept as is (not a people list)',
    empty: 'Empty',
  }

  async function add() {
    if (!preview) return
    adding = true
    error = null
    const result = await backend.addPeopleColumn(table, column)
    adding = false
    if (result.ok) onadded(result.value, preview.newColumn)
    else error = result.error
  }
</script>

<Modal title={`Parse people field — ${table}[${column}]`} width={720} {onclose}>
  <p class="intro muted">
    Turns people-picker lists such as <code>[{"{"}'firstName': 'Ada', 'lastName': 'Lovelace'{"}"}]</code> into
    <code>Ada Lovelace</code> (several people joined with <code>; </code>). Text that is not a list is kept.
  </p>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
  {#if preview}
    <p class="counts">
      <strong>{preview.rows.toLocaleString()}</strong> rows:
      <span class="ok">{preview.names.toLocaleString()} parsed</span> ·
      <span class:warn={preview.unparsed > 0}>{preview.unparsed.toLocaleString()} kept as is</span> ·
      <span class="muted">{preview.empty.toLocaleString()} empty</span>
    </p>
    <div class="table-scroll">
      <table>
        <thead>
          <tr><th>{column}</th><th>{preview.newColumn}</th></tr>
        </thead>
        <tbody>
          {#each preview.samples as s, i (i)}
            <tr class={s.outcome}>
              <td class="raw" title={s.raw ?? ''}>{s.raw ?? ''}</td>
              <td title={OUTCOME[s.outcome]}>{s.parsed ?? ''}</td>
            </tr>
          {:else}
            <tr><td colspan="2" class="muted">No non-empty values.</td></tr>
          {/each}
        </tbody>
      </table>
    </div>
    <p class="note muted">
      Adds <strong>{preview.newColumn}</strong> to the grid for this session: it sorts, filters and has statistics, is
      recomputed after each refresh, and is not part of the model or its measures.
    </p>
  {:else if !error}
    <p class="muted">Parsing…</p>
  {/if}
  {#snippet footer()}
    <button onclick={onclose}>Cancel</button>
    <button class="primary" onclick={add} disabled={!preview || adding || preview.names === 0}>
      {adding ? 'Adding…' : `Add “${preview?.newColumn ?? `${column} (parsed)`}”`}
    </button>
  {/snippet}
</Modal>

<style>
  .intro {
    margin: 0 0 8px;
    font-size: 12px;
  }

  .counts {
    margin: 6px 0;
  }

  .ok {
    color: var(--color-brand);
  }

  .warn {
    color: var(--color-accent-dark);
  }

  .error {
    color: var(--color-danger);
  }

  .table-scroll {
    max-height: 320px;
    overflow: auto;
    border: 1px solid var(--color-border);
  }

  table {
    width: 100%;
    border-collapse: collapse;
    table-layout: fixed;
    font-size: 12px;
  }

  th,
  td {
    padding: 3px 8px;
    border-bottom: 1px solid var(--color-border);
    overflow: hidden;
    text-align: left;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  th {
    position: sticky;
    top: 0;
    background: var(--color-chrome);
  }

  .raw {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--color-text-muted);
  }

  tr.unparsed td:last-child {
    color: var(--color-accent-dark);
    font-style: italic;
  }

  .note {
    margin: 8px 0 0;
    font-size: 12px;
  }
</style>
