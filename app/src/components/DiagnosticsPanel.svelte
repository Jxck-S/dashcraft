<script lang="ts">
  import Icon, { type IconName } from './Icon.svelte'
  import type { DetectionDto, DiagnosticDto, SeverityDto } from '../lib/bindings'

  let {
    diagnostics,
    detection,
    onselect,
    onclose,
  }: {
    diagnostics: DiagnosticDto[]
    detection: DetectionDto | null
    onselect: (id: number) => void
    onclose: () => void
  } = $props()

  let tab = $state<'diagnostics' | 'detection'>('diagnostics')

  const SEVERITY: Record<SeverityDto, { icon: IconName; rank: number }> = {
    error: { icon: 'error', rank: 0 },
    warning: { icon: 'warning', rank: 1 },
    info: { icon: 'info', rank: 2 },
  }

  const sorted = $derived([...diagnostics].sort((a, b) => SEVERITY[a.severity].rank - SEVERITY[b.severity].rank))
</script>

<section class="diagnostics-panel" aria-label="Diagnostics">
  <header>
    <div class="tabs" role="tablist">
      <button role="tab" aria-selected={tab === 'diagnostics'} class:active={tab === 'diagnostics'} onclick={() => (tab = 'diagnostics')}>
        Diagnostics ({diagnostics.length})
      </button>
      <button role="tab" aria-selected={tab === 'detection'} class:active={tab === 'detection'} onclick={() => (tab = 'detection')}>
        Detection
      </button>
    </div>
    <button class="close" onclick={onclose} title="Close (Esc)" aria-label="Close diagnostics"><Icon name="close" size={14} /></button>
  </header>

  <div class="body">
    {#if tab === 'diagnostics'}
      {#if sorted.length === 0}
        <p class="empty-hint">No diagnostics: the file was read without problems.</p>
      {:else}
        <table>
          <thead>
            <tr><th>Severity</th><th>Code</th><th>Message</th><th>Origin</th></tr>
          </thead>
          <tbody>
            {#each sorted as d, i (i)}
              <tr class={d.severity}>
                <td class="sev"><Icon name={SEVERITY[d.severity].icon} size={13} /> {d.severity}</td>
                <td class="mono">{d.code}</td>
                <td>
                  {#if d.objectId != null}
                    {@const objectId = d.objectId}
                    <button class="link" onclick={() => onselect(objectId)} title="Show the object in Properties">{d.message}</button>
                  {:else}
                    {d.message}
                  {/if}
                </td>
                <td class="mono origin" title={d.origin ?? ''}>{d.origin ?? ''}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    {:else if detection}
      <dl class="detection">
        <dt>Format</dt>
        <dd>{detection.label} <span class="muted mono">({detection.format}, {detection.confidence})</span></dd>
        {#if detection.pbipFile}
          <dt>Project file</dt>
          <dd class="mono">{detection.pbipFile}</dd>
        {/if}
        <dt>Entries</dt>
        <dd>{detection.entryCount}</dd>
        {#each detection.semanticModels as m (m.path)}
          <dt>Semantic model</dt>
          <dd><span class="mono">{m.path || '.'}</span> <span class="muted">({m.flavour}{m.detail ? `, ${m.detail}` : ''})</span></dd>
        {/each}
        {#each detection.reports as r (r.path)}
          <dt>Report</dt>
          <dd><span class="mono">{r.path || '.'}</span> <span class="muted">({r.flavour}{r.detail ? `, ${r.detail}` : ''})</span></dd>
        {/each}
        <dt>Signals</dt>
        <dd>
          <ul>
            {#each detection.signals as s, i (i)}<li>{s}</li>{/each}
          </ul>
        </dd>
        {#if detection.parts.length > 0}
          <dt>Parts</dt>
          <dd>
            <ul>
              {#each detection.parts as p (p.path)}<li><span class="mono">{p.path}</span> <span class="muted">{p.kind}</span></li>{/each}
            </ul>
          </dd>
        {/if}
      </dl>
    {/if}
  </div>
</section>

<style>
  .diagnostics-panel {
    display: flex;
    flex-direction: column;
    height: 220px;
    border-top: 1px solid var(--color-border-strong);
    background: var(--color-pane);
  }

  header {
    display: flex;
    align-items: center;
    padding: 0 6px 0 8px;
    border-bottom: 1px solid var(--color-border);
    background: var(--color-chrome);
  }

  .tabs {
    display: flex;
    flex: 1;
    gap: 2px;
  }

  .tabs button {
    padding: 5px 12px 3px;
    border: none;
    border-bottom: 2px solid transparent;
    background: transparent;
    color: var(--color-text-muted);
    cursor: pointer;
  }

  .tabs button.active {
    border-bottom-color: var(--color-accent);
    color: var(--color-text);
    font-weight: 600;
  }

  .close {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border: none;
    background: transparent;
    cursor: pointer;
  }

  .body {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }

  th,
  td {
    padding: 3px 10px;
    border-bottom: 1px solid var(--color-border);
    text-align: left;
    vertical-align: top;
  }

  th {
    position: sticky;
    top: 0;
    background: var(--color-pane);
    font-weight: 600;
  }

  .sev {
    white-space: nowrap;
    text-transform: capitalize;
  }

  tr.error .sev {
    color: var(--color-danger);
  }

  tr.warning .sev {
    color: var(--color-accent-dark);
  }

  .mono {
    font-family: var(--font-mono);
  }

  .origin {
    max-width: 360px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .link {
    padding: 0;
    border: none;
    background: transparent;
    color: var(--color-brand);
    text-align: left;
    text-decoration: underline;
    cursor: pointer;
  }

  .detection {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px 14px;
    margin: 0;
    padding: 8px 12px;
    font-size: 12px;
  }

  .detection dt {
    color: var(--color-text-muted);
  }

  .detection dd {
    margin: 0;
  }

  .detection ul {
    margin: 0;
    padding-left: 16px;
  }
</style>
