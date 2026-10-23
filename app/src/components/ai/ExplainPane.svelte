<script module lang="ts">
  /** The measure the Explain pane is about; a new `seq` runs the request again. */
  export interface ExplainTarget {
    docId: string
    table: string
    measure: string
    expression: string
    seq: number
  }
</script>

<script lang="ts">
  import { onDestroy, untrack } from 'svelte'
  import CodeView from '../CodeView.svelte'
  import Icon from '../Icon.svelte'
  import Markdown from './Markdown.svelte'
  import { failureReason, placeOf, runAi, type AskConsent } from '../../lib/ai'
  import { cancelLatest, type ExplainMeasureOut } from '../../lib/ops'

  // "Explain with AI": runs `ai.explainMeasure` for the target measure (consent dialog first if
  // needed), with progress and Cancel, and shows the summary, the explanation (Markdown, rendered
  // safely), filter-context notes, dependencies, caveats and what the measure references
  // (computed locally). Nothing is written to the model.
  let { target, ask, onoptions }: { target: ExplainTarget; ask: AskConsent; onoptions: () => void } = $props()

  const LATEST = 'ai-explain'

  let phase = $state<'running' | 'done' | 'error' | 'cancelled'>('running')
  let progress = $state('Preparing the request…')
  let result = $state.raw<ExplainMeasureOut | null>(null)
  let error = $state<{ message: string; reason: string | null } | null>(null)
  let copied = $state(false)
  let seq = 0

  $effect(() => {
    void target.seq
    void target.measure
    untrack(() => void run())
  })

  onDestroy(() => {
    seq += 1
    if (phase === 'running') void cancelLatest(LATEST)
  })

  async function run() {
    const mine = ++seq
    const { docId, table, measure } = target
    phase = 'running'
    progress = 'Preparing the request…'
    result = null
    error = null
    const askHere: AskConsent = async (need, preview) => {
      progress = 'Waiting for your permission…'
      const ok = await ask(need, preview)
      // The pane was closed or moved on meanwhile: send nothing.
      if (mine !== seq) return false
      if (ok) progress = 'Sending…'
      return ok
    }
    const r = await runAi('ai.explainMeasure', { docId, measure, table }, { capability: 'dax.explain', docId, measure, table }, askHere, {
      latest: LATEST,
      onProgress: (p) => {
        if (mine === seq) progress = p.message
      },
    })
    if (mine !== seq) return
    if (r.ok) {
      result = r.value
      phase = 'done'
    } else {
      error = { message: r.error, reason: failureReason(r) }
      phase = r.kind === 'cancelled' ? 'cancelled' : 'error'
    }
  }

  async function cancel() {
    progress = 'Cancelling…'
    await cancelLatest(LATEST)
  }

  async function copy() {
    if (!result) return
    try {
      await navigator.clipboard.writeText(result.markdown)
      copied = true
      setTimeout(() => (copied = false), 1200)
    } catch {
      copied = false
    }
  }

  const bullets = (items: string[]) => items.filter((i) => i.trim() !== '').map((i) => `- ${i.trim().replace(/\n/g, ' ')}`).join('\n')
  const needsOptions = $derived(['disabled', 'noProvider', 'unknownProvider', 'apiKeyMissing', 'invalidSettings', 'keychain'].includes(error?.reason ?? ''))
</script>

<section class="explain" aria-label="Explain with AI" aria-busy={phase === 'running'}>
  <header class="object-header">
    <span class="kind">Measure</span>
    <h3 title={target.measure}>{target.measure}</h3>
    <span class="muted parent">in {target.table}</span>
  </header>

  <div class="code-block">
    <CodeView text={target.expression} language="dax" compact />
  </div>

  {#if phase === 'running'}
    <div class="progress" role="status">
      <span class="spinner" aria-hidden="true"></span>
      <span class="progress-text">{progress}</span>
      <button class="small-btn" onclick={() => void cancel()} title="Stop waiting for the answer">Cancel</button>
    </div>
  {:else if phase === 'cancelled'}
    <div class="note" role="status">
      <span>{error?.message ?? 'Cancelled.'}</span>
      <button class="small-btn" onclick={() => void run()}>Try again</button>
    </div>
  {:else if phase === 'error' && error}
    <div class="note error" role="alert">
      <Icon name="error" size={14} />
      <span>{error.message}</span>
    </div>
    <div class="actions">
      {#if needsOptions}<button class="small-btn" onclick={onoptions}><Icon name="settings" size={14} /> AI options</button>{/if}
      <button class="small-btn" onclick={() => void run()}>Try again</button>
    </div>
  {:else if result}
    {@const r = result}
    <div class="answer">
      <p class="summary">{r.summary}</p>
      <Markdown text={r.explanation} />
      {#if r.filterContext.length > 0}
        <h4>Filter context</h4>
        <Markdown text={bullets(r.filterContext)} />
      {/if}
      {#if r.dependencies.length > 0}
        <h4>Dependencies</h4>
        <ul class="deps">
          {#each r.dependencies as d, i (i)}
            <li><code>{d.name}</code> <Markdown text={d.note} /></li>
          {/each}
        </ul>
      {/if}
      {#if r.caveats.length > 0}
        <h4>Caveats</h4>
        <Markdown text={bullets(r.caveats)} />
      {/if}
      <details class="refs">
        <summary title="Computed by DashCraft from the model, not by the AI">References (from the model)</summary>
        <dl>
          {#each [{ label: 'Measures', items: r.references.measures }, { label: 'Columns', items: r.references.columns }, { label: 'Tables', items: r.references.tables }, { label: 'Relationships', items: r.references.relationships }] as group (group.label)}
            {#if group.items.length > 0}
              <dt>{group.label}</dt>
              <dd>{group.items.join('\n')}</dd>
            {/if}
          {/each}
        </dl>
      </details>
    </div>
    <p class="call muted">
      AI-generated by <strong>{r.call.provider}</strong>{r.call.model ? ` (${r.call.model})` : ''} at <code>{r.call.endpointHost}</code> — {placeOf(r.call.local)}; {(r.call.durationMs / 1000).toFixed(1)} s. Review before relying on it; nothing was changed in the model.
    </p>
    <div class="actions">
      <button class="small-btn" onclick={() => void copy()}><Icon name="copy" size={14} /> {copied ? 'Copied' : 'Copy as Markdown'}</button>
      <button class="small-btn" onclick={() => void run()} title="Ask again (a new request)"><Icon name="refresh" size={14} /> Explain again</button>
    </div>
  {/if}
</section>

<style>
  .explain {
    height: 100%;
    overflow: auto;
    padding-bottom: 16px;
  }

  .object-header {
    padding: 0 12px 8px;
    border-bottom: 1px solid var(--color-border);
  }

  .kind {
    color: var(--color-brand);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  h3 {
    margin: 2px 0 0;
    font-size: 15px;
    overflow-wrap: anywhere;
  }

  .parent {
    display: block;
    font-size: 12px;
  }

  .code-block {
    margin: 8px 12px;
  }

  .progress,
  .note {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 8px 12px;
  }

  .progress-text {
    flex: 1;
    min-width: 0;
    color: var(--color-text-muted);
  }

  .note.error {
    align-items: flex-start;
    color: var(--color-danger);
    overflow-wrap: anywhere;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin: 8px 12px;
  }

  .small-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 3px 10px;
    border: 1px solid var(--color-border-strong);
    border-radius: 3px;
    background: var(--color-pane);
    font-size: 12px;
    cursor: pointer;
  }

  .small-btn:hover {
    background: var(--color-hover);
  }

  .answer {
    margin: 4px 12px 0;
  }

  .summary {
    margin: 0 0 8px;
    font-weight: 600;
  }

  h4 {
    margin: 10px 0 4px;
    color: var(--color-text-muted);
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }

  .deps {
    margin: 0;
    padding-left: 18px;
  }

  .deps li :global(.markdown) {
    display: inline;
  }

  .deps li :global(.markdown p) {
    display: inline;
  }

  .refs {
    margin: 10px 0 0;
    border-top: 1px solid var(--color-border);
  }

  .refs summary {
    padding: 6px 0;
    color: var(--color-text-muted);
    cursor: pointer;
  }

  .refs dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px 10px;
    margin: 0 0 6px;
    font-size: 12px;
  }

  .refs dt {
    color: var(--color-text-muted);
  }

  .refs dd {
    margin: 0;
    font-family: var(--font-mono);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .call {
    margin: 10px 12px 0;
    font-size: 11px;
  }

  .spinner {
    flex: none;
    width: 14px;
    height: 14px;
    border: 2px solid var(--color-border-strong);
    border-top-color: var(--color-brand);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
