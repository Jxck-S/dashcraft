<script lang="ts">
  import Modal from './Modal.svelte'
  import type { RecoveryJournal } from '../lib/backend'
  import { baseName, formatTimestamp, plural } from '../lib/format'

  // Document recovery: unsaved edits left by a session that ended without saving (DashCraft
  // stopped unexpectedly), as Power BI Desktop offers after a crash. Restore reopens the file
  // with the edits (still unsaved); Discard deletes them; Decide later asks again next start.
  let {
    journals,
    pending,
    onrestore,
    ondiscard,
    onclose,
  }: {
    journals: RecoveryJournal[]
    /** The journal being restored or discarded. */
    pending: string | null
    onrestore: (id: string) => void
    ondiscard: (id: string) => void
    onclose: () => void
  } = $props()

  const fileName = (path: string | null | undefined) => (path ? baseName(path) : 'Unknown file')

  function blocker(j: RecoveryJournal): string | null {
    if (j.problem) return j.problem
    if (!j.sourceExists) return 'The file is no longer at this path.'
    return null
  }
</script>

<Modal title="Document recovery" width={620} {onclose}>
  <p>DashCraft closed before these files were saved. Restore one to reopen it with its unsaved edits, then save it.</p>
  <ul class="journals" aria-label="Recoverable files">
    {#each journals as j (j.recoveryId)}
      {@const problem = blocker(j)}
      {@const updated = formatTimestamp(j.updated)}
      <li>
        <div class="info">
          <strong>{fileName(j.path)}</strong>
          <span class="path" title={j.path ?? ''}>{j.path ?? ''}</span>
          <span class="meta">
            {plural(j.edits, 'unsaved edit')}{#if j.lastEdit}; last: {j.lastEdit}{/if}{#if updated}; {updated}{/if}
          </span>
          {#if problem}<span class="problem" role="note">⚠ {problem}</span>{/if}
        </div>
        <div class="actions">
          <button class="primary" disabled={problem !== null || pending !== null} onclick={() => onrestore(j.recoveryId)} aria-label={`Restore ${fileName(j.path)}`}>Restore</button>
          <button disabled={pending !== null} onclick={() => ondiscard(j.recoveryId)} aria-label={`Discard the unsaved edits of ${fileName(j.path)}`}>Discard</button>
        </div>
      </li>
    {/each}
  </ul>
  {#snippet footer()}
    <button onclick={onclose} disabled={pending !== null}>Decide later</button>
  {/snippet}
</Modal>

<style>
  .journals {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-height: min(360px, calc(100vh - 260px));
    margin: 8px 0 0;
    padding: 0;
    overflow: auto;
    list-style: none;
  }

  li {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 10px;
    border: 1px solid var(--color-border);
    border-radius: 4px;
  }

  .info {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
    gap: 2px;
  }

  .path {
    overflow: hidden;
    color: var(--color-text-muted);
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meta {
    font-size: 12px;
  }

  .problem {
    color: var(--color-danger);
    font-size: 12px;
  }

  .actions {
    display: flex;
    flex: none;
    gap: 6px;
  }
</style>
