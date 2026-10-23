<script lang="ts">
  import Icon, { type IconName } from './Icon.svelte'
  import { kindLabel, runProgress, type RefreshEntry, type RefreshRun, type StepState } from '../lib/data'
  import { plural } from '../lib/format'

  // Refresh progress (per table, from `refresh-progress` events) and results: errors with their
  // hint and "Load CSV manually…" where the source can be replaced by a CSV.
  let {
    run,
    oncancel,
    onclose,
    onloadcsv,
    onselecttable,
  }: {
    run: RefreshRun
    oncancel: () => void
    onclose: () => void
    onloadcsv: (table: string, reason: string | null) => void
    onselecttable: (table: string) => void
  } = $props()

  const progress = $derived(runProgress(run))
  const failed = $derived(run.entries.filter((e) => e.state === 'error' || e.state === 'partial').length)
  const skipped = $derived(run.entries.filter((e) => e.state === 'skipped').length)
  const running = $derived(run.outcome === 'running')
  let showDiagnostics = $state(false)

  const ICON: Record<Exclude<StepState, 'running'>, { icon: IconName; label: string }> = {
    pending: { icon: 'info', label: 'Waiting' },
    notLoaded: { icon: 'info', label: 'Not loaded' },
    ok: { icon: 'check', label: 'Done' },
    partial: { icon: 'warning', label: 'Loaded, some calculated columns failed' },
    error: { icon: 'error', label: 'Failed' },
    skipped: { icon: 'warning', label: 'Skipped' },
  }

  const title = $derived.by(() => {
    switch (run.outcome) {
      case 'running':
        return run.cancelling ? 'Cancelling…' : `Refreshing… ${progress.done}/${progress.total || '?'}`
      case 'ok':
        return `Refreshed ${plural(run.entries.length, 'table')}`
      case 'failed':
        return `Refresh finished: ${plural(failed, 'table')} failed${skipped ? `, ${skipped} skipped` : ''}`
      case 'cancelled':
        return 'Refresh cancelled — the previous data is kept'
      case 'error':
        return 'Refresh failed'
    }
  })

  function seconds(ms: number | null): string {
    return ms == null ? '' : ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`
  }

  function detail(e: RefreshEntry): string {
    const parts = [e.kind ? kindLabel(e.kind) : null, e.rows != null ? `${e.rows.toLocaleString()} rows` : null, seconds(e.durationMs) || null]
    return parts.filter(Boolean).join(' · ')
  }
</script>

<section class="refresh-panel" aria-label="Refresh">
  <header>
    <strong class:err={run.outcome === 'failed' || run.outcome === 'error'}>{title}</strong>
    {#if run.durationMs != null && !running}<span class="muted">{seconds(run.durationMs)}</span>{/if}
    <span class="spacer"></span>
    <button class="icon-btn" onclick={onclose} title={running ? 'Hide (the refresh continues)' : 'Close'} aria-label="Close"><Icon name="close" size={12} /></button>
  </header>
  {#if running}
    <div class="bar" role="progressbar" aria-valuemin={0} aria-valuemax={progress.total} aria-valuenow={progress.done}>
      <span style:width={`${progress.total ? (100 * progress.done) / progress.total : 5}%`}></span>
    </div>
  {/if}
  {#if run.error}
    <p class="call-error">{run.error}</p>
  {/if}
  <ul>
    {#each run.entries as e (e.name)}
      <li class={e.state}>
        <span class="state" title={e.state === 'running' ? 'Running' : e.state === 'pending' && !running ? 'Not refreshed' : ICON[e.state].label}>
          {#if e.state === 'running'}<span class="spinner" aria-hidden="true"></span>{:else}<Icon name={ICON[e.state].icon} size={14} />{/if}
        </span>
        <span class="main">
          <button class="name" onclick={() => onselecttable(e.name)} title="Show in the Data tab">{e.name}</button>
          <span class="detail muted">
            {detail(e)}{#if e.current}{detail(e) ? ' · ' : ''}computing [{e.current}]…{/if}
          </span>
          {#if e.message}
            <span class="message" title={e.message}>{e.message}</span>
          {/if}
          {#if e.hint}
            <span class="hint">{e.hint}</span>
          {/if}
          {#each e.columnErrors as c (c.name)}
            <span class="message column" title={c.message}>[{c.name}]: {c.message}</span>
          {/each}
        </span>
        {#if e.csvFallback && !running}
          <button class="csv" onclick={() => onloadcsv(e.name, e.message)} title="Replace this table's data with a local CSV">
            <Icon name="csv" size={14} /> Load CSV manually…
          </button>
        {/if}
      </li>
    {:else}
      <li class="pending muted">Starting…</li>
    {/each}
  </ul>
  {#if run.diagnostics.length > 0}
    <button class="diag-toggle" onclick={() => (showDiagnostics = !showDiagnostics)} aria-expanded={showDiagnostics}>
      <Icon name="warning" size={12} /> {plural(run.diagnostics.length, 'diagnostic')}
    </button>
    {#if showDiagnostics}
      <ul class="diagnostics">
        {#each run.diagnostics as d, i (i)}
          <li><code>{d.code}</code> {d.message}</li>
        {/each}
      </ul>
    {/if}
  {/if}
  {#if running}
    <footer>
      <button onclick={oncancel} disabled={run.cancelling}><Icon name="stop" size={12} /> {run.cancelling ? 'Cancelling…' : 'Cancel'}</button>
    </footer>
  {/if}
</section>

<style>
  .refresh-panel {
    position: fixed;
    top: calc(var(--toolbar-height) + 6px);
    right: 12px;
    z-index: 160;
    display: flex;
    flex-direction: column;
    width: min(460px, calc(100vw - 24px));
    max-height: calc(100vh - var(--toolbar-height) - var(--statusbar-height) - 24px);
    border: 1px solid var(--color-border-strong);
    border-radius: 4px;
    background: var(--color-pane);
    box-shadow: 0 10px 30px rgb(0 0 0 / 0.18);
    font-size: 12px;
  }

  header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 8px 6px 12px;
  }

  header strong {
    font-size: 13px;
  }

  .err {
    color: var(--color-danger);
  }

  .spacer {
    flex: 1;
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

  .bar {
    height: 3px;
    margin: 0 12px 4px;
    background: var(--color-chrome);
  }

  .bar span {
    display: block;
    height: 100%;
    background: var(--color-brand);
    transition: width 200ms ease;
  }

  .call-error {
    margin: 0 12px 6px;
    color: var(--color-danger);
  }

  ul {
    margin: 0;
    padding: 0 0 6px;
    overflow: auto;
    list-style: none;
  }

  li {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 5px 12px;
    border-top: 1px solid var(--color-border);
  }

  .state {
    display: grid;
    flex: none;
    place-items: center;
    width: 16px;
    height: 18px;
    color: var(--color-text-subtle);
  }

  li.ok .state {
    color: var(--color-brand);
  }

  li.error .state {
    color: var(--color-danger);
  }

  li.partial .state,
  li.skipped .state {
    color: var(--color-accent-dark);
  }

  .main {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }

  .name {
    align-self: flex-start;
    padding: 0;
    border: none;
    background: transparent;
    font-weight: 600;
    cursor: pointer;
  }

  .name:hover {
    text-decoration: underline;
  }

  .message {
    color: var(--color-danger);
    overflow-wrap: anywhere;
  }

  li.skipped .message,
  .message.column {
    color: var(--color-text-muted);
  }

  .hint {
    color: var(--color-text-muted);
    font-style: italic;
  }

  .csv {
    display: inline-flex;
    flex: none;
    align-items: center;
    gap: 5px;
    padding: 3px 8px;
    border: 1px solid var(--color-border-strong);
    border-radius: 2px;
    background: var(--color-pane);
    font-size: 12px;
    white-space: nowrap;
    cursor: pointer;
  }

  .csv:hover {
    background: var(--color-hover);
  }

  .diag-toggle {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin: 0 12px 6px;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--color-text-muted);
    cursor: pointer;
  }

  .diagnostics li {
    display: block;
    padding: 3px 12px;
    color: var(--color-text-muted);
  }

  footer {
    display: flex;
    justify-content: flex-end;
    padding: 8px 12px;
    border-top: 1px solid var(--color-border);
  }

  footer button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 12px;
    border: 1px solid var(--color-border-strong);
    border-radius: 2px;
    background: var(--color-pane);
    cursor: pointer;
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

  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation-duration: 2.4s;
    }
  }
</style>
