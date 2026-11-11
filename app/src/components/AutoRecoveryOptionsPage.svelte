<script lang="ts">
  import { onMount } from 'svelte'
  import Icon from './Icon.svelte'
  import { autoRecovery, setAutoRecovery, type AutoRecoveryDto } from '../lib/backend'

  // Options → Auto recovery (Global): whether unsaved edits are kept so they can be restored after
  // DashCraft stops unexpectedly. Desktop saves a recovery copy every few minutes; DashCraft
  // journals each edit as it happens (`doc.recovery.*`), so there is no interval to choose.
  let current = $state.raw<AutoRecoveryDto | null>(null)
  let busy = $state(false)
  let error = $state<string | null>(null)

  onMount(() => {
    void (async () => {
      const got = await autoRecovery()
      if (got.ok) current = got.value
      else error = got.error
    })()
  })

  async function toggle(box: HTMLInputElement) {
    busy = true
    error = null
    const set = await setAutoRecovery(box.checked)
    busy = false
    if (set.ok) current = set.value
    else {
      error = set.error
      box.checked = current?.enabled ?? false
    }
  }
</script>

<div class="recovery-page" aria-busy={busy}>
  <h2>Auto recovery</h2>
  {#if error}
    <p class="error" role="alert"><Icon name="error" size={14} /> {error}</p>
  {/if}
  <label class="switch">
    <input type="checkbox" checked={current?.enabled ?? false} disabled={busy || !current?.available} onchange={(e) => void toggle(e.currentTarget as HTMLInputElement)} />
    <span>
      <strong>Keep unsaved edits for recovery</strong>
      <span class="muted">Every edit is recorded as you make it. If DashCraft stops before you save, it offers to restore the edits the next time it starts. Turning this off deletes the recovery files of open documents.</span>
    </span>
  </label>
  <h3>Recovery file location</h3>
  {#if current?.location}
    <p><code class="location">{current.location}</code></p>
    <p class="muted small">In your user data folder, never next to your projects. Recovery files are deleted when you save, reload or close a document.</p>
  {:else if current}
    <p class="muted">There is no user data folder on this computer, so Auto recovery is unavailable.</p>
  {/if}
</div>

<style>
  .recovery-page {
    max-width: 760px;
  }

  h2 {
    margin: 0 0 10px;
    font-size: 20px;
    font-weight: 400;
  }

  h3 {
    margin: 18px 0 4px;
    font-size: 14px;
  }

  .switch {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    margin: 14px 0 4px;
  }

  .switch > span {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .switch input {
    margin-top: 3px;
  }

  .muted {
    color: var(--color-text-muted);
  }

  .small {
    font-size: 12px;
  }

  .location {
    word-break: break-all;
  }

  .error {
    display: flex;
    gap: 6px;
    align-items: center;
    color: var(--color-danger);
  }
</style>
