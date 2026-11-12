<script lang="ts">
  import Modal from './Modal.svelte'
  import type { DocumentDiff, FileChange, ObjectDiff } from '../lib/backend'
  import { plural } from '../lib/format'
  import { diffLines } from '../lib/lineDiff'

  // Compare (M8.2, M8.3): the project files another program changed on disk and the document
  // compared object by object with them (`doc.diff`: "before" is the disk, "after" this window,
  // unsaved edits included). Reload takes the disk's version; Keep mine keeps this one (Save then
  // asks before overwriting the files).
  let {
    fileName,
    changes,
    diff,
    error,
    onreload,
    onkeep,
    onclose,
  }: {
    fileName: string
    changes: FileChange[]
    /** `null` while the comparison runs. */
    diff: DocumentDiff | null
    error: string | null
    onreload: () => void
    onkeep: () => void
    onclose: () => void
  } = $props()

  const FILE_CHANGE = { added: 'Added', modified: 'Changed', removed: 'Deleted' } as const
  // `added`: only in this window; `removed`: only on disk.
  const OBJECT_CHANGE = { added: 'Only here', removed: 'Only on disk', changed: 'Different' } as const
  const MAX_VALUE = 4000

  const KIND_LABEL: Record<ObjectDiff['kind'], string> = {
    model: 'Model',
    database: 'Database',
    table: 'Table',
    column: 'Column',
    measure: 'Measure',
    partition: 'Partition',
    hierarchy: 'Hierarchy',
    calculationItem: 'Calculation item',
    relationship: 'Relationship',
    expression: 'Expression',
    queryGroup: 'Query group',
    role: 'Role',
    culture: 'Culture',
    perspective: 'Perspective',
    report: 'Report',
    reportMeasure: 'Report measure',
    page: 'Page',
    visual: 'Visual',
  }

  function show(value: unknown): string {
    if (value === undefined) return '—'
    const text = typeof value === 'string' ? value : JSON.stringify(value, null, 2)
    return text.length > MAX_VALUE ? `${text.slice(0, MAX_VALUE)}…` : text
  }

  function origin(o: ObjectDiff['before']): string {
    if (!o) return ''
    return o.line != null ? `${o.path}:${o.line}` : o.path
  }

  const summary = $derived(
    diff
      ? diff.identical
        ? 'The model and reports are the same on disk and here.'
        : [diff.changed > 0 ? `${diff.changed} different` : '', diff.removed > 0 ? `${diff.removed} only on disk` : '', diff.added > 0 ? `${diff.added} only here` : '']
            .filter((s) => s)
            .join(', ')
      : '',
  )
</script>

<Modal title={`Compare ${fileName} with the files on disk`} width={820} {onclose}>
  <section aria-labelledby="compare-files">
    <h3 id="compare-files">{plural(changes.length, 'file')} changed on disk by another program</h3>
    <ul class="files">
      {#each changes as c (c.path)}
        <li><span class="badge {c.change}">{FILE_CHANGE[c.change]}</span> <code>{c.path}</code></li>
      {/each}
    </ul>
  </section>
  <section aria-labelledby="compare-objects" aria-busy={diff === null && error === null}>
    <h3 id="compare-objects">Objects</h3>
    {#if error}
      <p class="error" role="alert">The comparison failed: {error}</p>
    {:else if !diff}
      <p role="status">Comparing…</p>
    {:else}
      <p role="status">{summary}</p>
      {#if diff.objects.length > 0}
        <ul class="objects" aria-label="Objects that differ">
          {#each diff.objects as o, i (`${o.kind}:${o.path}:${i}`)}
            <li>
              <details open={diff.objects.length <= 5}>
                <summary>
                  <span class="badge {o.change}">{OBJECT_CHANGE[o.change]}</span>
                  <span class="kind">{KIND_LABEL[o.kind]}</span>
                  <strong>{o.path}</strong>
                </summary>
                {#if o.before || o.after}
                  <p class="origins">
                    {#if o.before}On disk: <code>{origin(o.before)}</code>{/if}
                    {#if o.before && o.after}; {/if}
                    {#if o.after}Here: <code>{origin(o.after)}</code>{/if}
                  </p>
                {/if}
                {#if o.snippet}
                  {@const lines = diffLines(o.snippet.before ?? '', o.snippet.after ?? '')}
                  <div class="snippet" role="textbox" aria-readonly="true" aria-multiline="true" aria-label={`${o.snippet.language === 'tmdl' ? 'TMDL' : 'JSON'} of ${o.path}: lines with a minus are on disk only, with a plus here only`} tabindex="0">
                    {#each lines as l, k (k)}
                      <div class="line {l.kind}"><span class="mark">{l.kind === 'removed' ? '-' : l.kind === 'added' ? '+' : ' '}</span>{l.text}</div>
                    {/each}
                  </div>
                {/if}
                {#if o.properties.length > 0}
                  <table>
                    <thead>
                      <tr><th scope="col">Property</th><th scope="col">On disk</th><th scope="col">Here</th></tr>
                    </thead>
                    <tbody>
                      {#each o.properties as p (p.name)}
                        <tr>
                          <th scope="row">{p.name}</th>
                          <td><pre>{show(p.before)}</pre></td>
                          <td><pre>{show(p.after)}</pre></td>
                        </tr>
                      {/each}
                    </tbody>
                  </table>
                {/if}
              </details>
            </li>
          {/each}
        </ul>
      {/if}
    {/if}
  </section>
  {#snippet footer()}
    <button class="primary" onclick={onreload} title="Reread the files on disk; your unsaved edits are discarded">Reload from disk</button>
    <button onclick={onkeep} title="Keep this version; Save asks before overwriting the files on disk">Keep mine</button>
    <button onclick={onclose}>Close</button>
  {/snippet}
</Modal>

<style>
  section + section {
    margin-top: 12px;
  }

  h3 {
    margin: 0 0 6px;
    font-size: 13px;
    font-weight: 600;
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .files {
    max-height: 120px;
    overflow: auto;
    font-size: 12px;
  }

  .files li + li {
    margin-top: 2px;
  }

  .objects {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: min(420px, calc(100vh - 360px));
    overflow: auto;
  }

  .objects li {
    padding: 6px 8px;
    border: 1px solid var(--color-border);
    border-radius: 4px;
  }

  summary {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
  }

  .kind {
    color: var(--color-text-muted);
    font-size: 12px;
  }

  .badge {
    flex: none;
    padding: 0 6px;
    border: 1px solid var(--color-border);
    border-radius: 3px;
    font-size: 11px;
  }

  .origins {
    margin: 6px 0 0;
    color: var(--color-text-muted);
    font-size: 12px;
  }

  table {
    width: 100%;
    margin-top: 6px;
    border-collapse: collapse;
    table-layout: fixed;
    font-size: 12px;
  }

  th,
  td {
    padding: 3px 6px;
    border: 1px solid var(--color-border);
    text-align: left;
    vertical-align: top;
  }

  thead th:first-child,
  tbody th {
    width: 140px;
  }

  pre {
    max-height: 160px;
    margin: 0;
    overflow: auto;
    font-size: 12px;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .error {
    color: var(--color-danger);
  }

  .snippet {
    max-height: 240px;
    margin-top: 6px;
    padding: 4px 0;
    overflow: auto;
    border: 1px solid var(--color-border);
    background: var(--color-code-bg);
    font-family: var(--font-mono, Consolas, 'Courier New', monospace);
    font-size: 12px;
    tab-size: 4;
  }

  .line {
    padding: 0 6px;
    white-space: pre;
  }

  .mark {
    display: inline-block;
    width: 1.5ch;
    color: var(--color-text-muted);
  }

  .line.removed {
    background: var(--color-danger-bg);
  }

  .line.added {
    background: var(--color-brand-soft);
  }
</style>
