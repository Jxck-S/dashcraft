<script module lang="ts">
  export interface StatusZoom {
    /** Percent (100 = actual size). */
    percent: number
    min?: number
    max?: number
    onzoom: (percent: number) => void
    onfit: () => void
    fitTitle: string
  }

  export interface DataStatus {
    label: string
    title: string
    tone: 'ok' | 'warn' | 'error' | 'none'
    onclick?: () => void
  }
</script>

<script lang="ts">
  import Icon from '../Icon.svelte'
  import type { DocumentSummary } from '../../lib/bindings'
  import { plural } from '../../lib/format'

  // Desktop status bar: view-specific text on the left ("Page 1 of 2", "Table: …"), then
  // DashCraft's extras (status message, data state, diagnostics) kept unobtrusive, and the zoom
  // slider with the percentage and "Fit to page" on the right.
  let {
    text,
    message,
    isError = false,
    summary,
    data,
    zoom,
    version,
    hasBackend,
    diagnosticsOpen,
    ontoggleDiagnostics,
  }: {
    text: string
    message: string
    isError?: boolean
    summary: DocumentSummary | null
    data: DataStatus | null
    zoom: StatusZoom | null
    version: string
    hasBackend: boolean
    diagnosticsOpen: boolean
    ontoggleDiagnostics: () => void
  } = $props()

  const diag = $derived(summary?.diagnostics)
  const diagTotal = $derived(diag ? diag.errors + diag.warnings + diag.infos : 0)

  const STEPS = [10, 25, 50, 75, 100, 125, 150, 200, 300, 400]

  function step(dir: 1 | -1) {
    if (!zoom) return
    const p = Math.round(zoom.percent)
    const next = dir > 0 ? STEPS.find((s) => s > p) : [...STEPS].reverse().find((s) => s < p)
    zoom.onzoom(next ?? p)
  }
</script>

<footer class="status-bar">
  {#if text}<span class="text">{text}</span>{/if}
  <span class="message" class:error={isError} role="status" aria-live="polite" title={message}>{message}</span>
  {#if data}
    {#if data.onclick}
      <button class="cell data {data.tone}" onclick={data.onclick} title={data.title}>{data.label}</button>
    {:else}
      <span class="cell data {data.tone}" title={data.title}>{data.label}</span>
    {/if}
  {/if}
  {#if summary && diag}
    <button
      class="cell diag"
      class:active={diagnosticsOpen}
      onclick={ontoggleDiagnostics}
      title={`${summary.formatLabel} · ${summary.fileName}\n${plural(diagTotal, 'diagnostic')}: ${plural(diag.errors, 'error')}, ${plural(diag.warnings, 'warning')}, ${diag.infos} info — click to ${diagnosticsOpen ? 'hide' : 'show'}`}
      aria-pressed={diagnosticsOpen}
    >
      <span class:err={diag.errors > 0}><Icon name="error" size={13} /> {diag.errors}</span>
      <span class:warn={diag.warnings > 0}><Icon name="warning" size={13} /> {diag.warnings}</span>
      <span><Icon name="info" size={13} /> {diag.infos}</span>
    </button>
  {/if}
  {#if !hasBackend}
    <span class="cell warn" title="Running in a plain browser: IPC, dialogs and drag-and-drop are disabled.">Browser preview</span>
  {/if}
  {#if zoom}
    <span class="zoom">
      <button class="zbtn" onclick={() => step(-1)} title="Zoom out" aria-label="Zoom out"><Icon name="subtract" size={12} /></button>
      <input
        type="range"
        min={zoom.min ?? 10}
        max={zoom.max ?? 400}
        step="1"
        value={Math.round(zoom.percent)}
        oninput={(e) => zoom?.onzoom(Number((e.currentTarget as HTMLInputElement).value))}
        aria-label="Zoom"
      />
      <button class="zbtn" onclick={() => step(1)} title="Zoom in" aria-label="Zoom in"><Icon name="add" size={12} /></button>
      <span class="pct">{Math.round(zoom.percent)}%</span>
      <button class="zbtn fit" onclick={zoom.onfit} title={zoom.fitTitle} aria-label={zoom.fitTitle}><Icon name="fitPage" size={14} /></button>
    </span>
  {:else}
    <span class="version">DashCraft {version}</span>
  {/if}
</footer>

<style>
  .status-bar {
    display: flex;
    flex: none;
    align-items: center;
    height: var(--statusbar-height);
    padding: 0 6px 0 10px;
    border-top: 1px solid var(--color-border);
    background: var(--color-chrome);
    color: var(--color-text);
    font-size: 12px;
    white-space: nowrap;
  }

  .text {
    flex: none;
    max-width: 50%;
    margin-right: 14px;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .message {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    color: var(--color-text-muted);
    text-overflow: ellipsis;
  }

  .message.error {
    color: var(--color-danger);
  }

  .cell {
    display: inline-flex;
    align-items: center;
    height: 20px;
    margin-left: 6px;
    padding: 0 8px;
    border: none;
    border-radius: 3px;
    background: transparent;
    color: var(--color-text-muted);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 320px;
  }

  button.cell {
    cursor: pointer;
  }

  button.cell:hover,
  .diag.active {
    background: var(--color-hover);
  }

  .data::before {
    content: '';
    width: 7px;
    height: 7px;
    margin-right: 6px;
    border-radius: 50%;
    background: var(--color-text-disabled);
  }

  .data.ok::before {
    background: var(--color-brand);
  }

  .data.warn::before {
    background: var(--color-accent);
  }

  .data.error::before {
    background: var(--color-danger);
  }

  .diag {
    gap: 8px;
  }

  .diag span {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }

  .err {
    color: var(--color-danger);
  }

  .warn {
    color: var(--color-accent-dark);
  }

  .zoom {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-left: 12px;
  }

  .zoom input {
    width: 110px;
    accent-color: var(--color-text-muted);
  }

  .zbtn {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    padding: 0;
    border: none;
    border-radius: 3px;
    background: transparent;
    cursor: pointer;
  }

  .zbtn:hover {
    background: var(--color-hover);
  }

  .zbtn.fit {
    width: 24px;
    margin-left: 4px;
  }

  .pct {
    min-width: 40px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .version {
    margin-left: 10px;
    color: var(--color-text-subtle);
  }
</style>
