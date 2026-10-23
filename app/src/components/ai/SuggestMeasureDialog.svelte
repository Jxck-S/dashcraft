<script lang="ts">
  import { onDestroy, untrack } from 'svelte'
  import CodeView from '../CodeView.svelte'
  import Icon from '../Icon.svelte'
  import Modal from '../Modal.svelte'
  import Markdown from './Markdown.svelte'
  import { failureReason, placeOf, runAi, type AskConsent } from '../../lib/ai'
  import { daxColumnRef } from '../../lib/model'
  import { cancelLatest, ops, type AiStatusOut, type GenerateMeasureOut } from '../../lib/ops'

  // "Suggest a measure" (Home / Modeling → AI): a request in plain language → `ai.generateMeasure`
  // (consent dialog first if needed) → the suggested DAX, parsed locally with diagnostics. The
  // suggestion is never written to the model: copy it, open it in the DAX query view, or evaluate
  // it locally (`dax.query`, no AI call).
  let {
    docId,
    tables,
    initialTable,
    status,
    ask,
    onclose,
    onoptions,
    onopenquery,
    ondatachanged,
  }: {
    docId: string
    tables: string[]
    initialTable: string | null
    status: AiStatusOut | null
    ask: AskConsent
    onclose: () => void
    onoptions: () => void
    /** Opens a query in the DAX query view. */
    onopenquery: (query: string) => void
    /** The data was loaded by an evaluation. */
    ondatachanged: () => void
  } = $props()

  const LATEST = 'ai-generate'
  const EVAL = 'ai-evaluate'
  const SUGGESTION = '__DashCraftSuggestion'

  let prompt = $state('')
  let table = $state(untrack(() => initialTable ?? ''))
  let provider = $state('')
  let withSample = $state(false)
  let running = $state(false)
  let progress = $state('')
  let result = $state.raw<GenerateMeasureOut | null>(null)
  let error = $state<{ message: string; reason: string | null } | null>(null)
  let evaluation = $state<{ running: boolean; value: string | null; error: string | null } | null>(null)
  let seq = 0

  const providers = $derived(status?.providers ?? [])
  const sampleRows = $derived(status?.sampleRows ?? 0)
  const canSend = $derived(prompt.trim() !== '' && prompt.length <= 4000 && !running)
  const needsOptions = $derived(['disabled', 'noProvider', 'unknownProvider', 'apiKeyMissing', 'invalidSettings', 'keychain'].includes(error?.reason ?? ''))
  const errors = $derived(result?.diagnostics.filter((d) => d.severity === 'error') ?? [])

  onDestroy(() => {
    seq += 1
    if (running) void cancelLatest(LATEST)
    if (evaluation?.running) void cancelLatest(EVAL)
  })

  async function suggest() {
    if (!canSend) return
    const mine = ++seq
    running = true
    progress = 'Preparing the request…'
    result = null
    error = null
    evaluation = null
    const params = {
      docId,
      prompt: prompt.trim(),
      table: table || null,
      provider: provider || null,
      sampleRows: withSample && table && sampleRows > 0 ? sampleRows : null,
    }
    const askHere: AskConsent = async (need, preview) => {
      progress = 'Waiting for your permission…'
      const ok = await ask(need, preview)
      if (mine !== seq) return false
      if (ok) progress = 'Sending…'
      return ok
    }
    const r = await runAi('ai.generateMeasure', params, { capability: 'dax.generate', ...params }, askHere, {
      latest: LATEST,
      onProgress: (p) => {
        if (mine === seq) progress = p.message
      },
    })
    if (mine !== seq) return
    running = false
    if (r.ok) result = r.value
    else error = { message: r.error, reason: failureReason(r) }
  }

  function evaluationQuery(r: GenerateMeasureOut): string {
    return `DEFINE\n\tMEASURE ${daxColumnRef(r.table, SUGGESTION)} =\n\t\t${r.expression.trim().replaceAll('\n', '\n\t\t')}\n\nEVALUATE\n\tROW("${r.name.replaceAll('"', '""')}", [${SUGGESTION}])\n`
  }

  /** Evaluates the suggestion over the loaded data (unfiltered); local only, nothing is sent. */
  async function evaluate() {
    if (!result) return
    const query = evaluationQuery(result)
    evaluation = { running: true, value: null, error: null }
    const r = await ops.daxQuery({ docId, query, maxRows: 1, formatted: true, blankAsNull: true }, { latest: EVAL })
    if (!evaluation) return
    if (r.ok) {
      const cell = r.value.tables[0]?.rows[0]?.[0]
      evaluation = { running: false, value: cell == null ? '(Blank)' : String(cell), error: null }
    } else evaluation = { running: false, value: null, error: r.error }
    ondatachanged()
  }

  function keydown(event: KeyboardEvent) {
    if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
      event.preventDefault()
      void suggest()
    }
  }
</script>

<Modal title="Suggest a measure (AI)" width={720} {onclose}>
  <div class="info" role="note">
    <Icon name="info" size={14} />
    <span>Suggestions are <strong>never written to the model</strong>. DashCraft checks the DAX with its own parser; copy it, or open it in the DAX query view.</span>
  </div>

  <label class="field">
    <span>Describe the measure</span>
    <textarea bind:value={prompt} rows="3" maxlength="4000" placeholder="e.g. Share of orders shipped late, as a percentage" onkeydown={keydown} disabled={running}></textarea>
  </label>
  <div class="row">
    <label class="field inline">
      <span>Home table</span>
      <select bind:value={table} disabled={running}>
        <option value="">(any)</option>
        {#each tables as t (t)}<option value={t}>{t}</option>{/each}
      </select>
    </label>
    {#if providers.length > 1}
      <label class="field inline">
        <span>Provider</span>
        <select bind:value={provider} disabled={running}>
          <option value="">Default</option>
          {#each providers as p (p.id)}<option value={p.id}>{p.label ?? p.id}{p.local ? ' (local)' : ''}</option>{/each}
        </select>
      </label>
    {/if}
  </div>
  {#if sampleRows > 0}
    <label class="check" title={table ? 'Sends the first rows of the home table; needs a separate “sample” consent' : 'Choose a home table first'}>
      <input type="checkbox" bind:checked={withSample} disabled={running || !table} />
      Include {sampleRows} sample {sampleRows === 1 ? 'row' : 'rows'} of the home table (row values leave this computer for remote providers)
    </label>
  {/if}

  {#if running}
    <div class="progress" role="status">
      <span class="spinner" aria-hidden="true"></span>
      <span class="progress-text">{progress}</span>
      <button class="small-btn" onclick={() => void cancelLatest(LATEST)}>Cancel</button>
    </div>
  {:else if error}
    <div class="error" role="alert">
      <Icon name="error" size={14} />
      <span>{error.message}</span>
      {#if needsOptions}<button class="small-btn" onclick={onoptions}><Icon name="settings" size={14} /> AI options</button>{/if}
    </div>
  {/if}

  {#if result}
    {@const r = result}
    <section class="result" aria-label="Suggestion">
      <h3>{r.name} <span class="muted">in {r.table}{r.formatString ? ` · format ${r.formatString}` : ''}</span></h3>
      <CodeView text={r.expression} language="dax" label={r.name} />
      {#if r.diagnostics.length > 0}
        <ul class="diagnostics">
          {#each r.diagnostics as d, i (i)}
            <li class={d.severity}>
              <Icon name={d.severity === 'error' ? 'error' : 'warning'} size={14} />
              <span>{d.line != null ? `Line ${d.line}, column ${d.column ?? 1}: ` : ''}{d.message}{(d.expected ?? []).length > 0 ? ` (expected ${(d.expected ?? []).slice(0, 6).join(', ')})` : ''}</span>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="ok"><Icon name="check" size={14} /> Parses as DAX; every table, column and measure it names exists.</p>
      {/if}
      <Markdown text={r.explanation} />
      {#if r.assumptions.length > 0}
        <h4>Assumptions</h4>
        <Markdown text={r.assumptions.map((a) => `- ${a.trim().replace(/\n/g, ' ')}`).join('\n')} />
      {/if}
      <div class="eval">
        <button class="small-btn" onclick={() => void evaluate()} disabled={!r.parsed || evaluation?.running} title="Evaluates the suggestion on this computer over the loaded data (no filters); loads the data first if needed. Nothing is sent.">
          <Icon name="play" size={14} /> Evaluate locally
        </button>
        <button class="small-btn" onclick={() => onopenquery(evaluationQuery(r))} title="Open a DEFINE MEASURE … EVALUATE query in the DAX query view">
          <Icon name="code" size={14} /> Open in DAX query view
        </button>
        {#if evaluation?.running}
          <span class="muted"><span class="spinner" aria-hidden="true"></span> Evaluating…</span>
        {:else if evaluation?.error}
          <span class="eval-error">{evaluation.error}</span>
        {:else if evaluation}
          <span>Value (no filters): <strong>{evaluation.value}</strong></span>
        {/if}
      </div>
      <p class="call muted">
        AI-generated by <strong>{r.call.provider}</strong>{r.call.model ? ` (${r.call.model})` : ''} at <code>{r.call.endpointHost}</code> — {placeOf(r.call.local)}; {(r.call.durationMs / 1000).toFixed(1)} s. Not applied to the model{errors.length > 0 ? '; fix the parse errors before using it' : ''}.
      </p>
    </section>
  {/if}

  {#snippet footer()}
    <button onclick={onclose}>Close</button>
    <button class="primary" onclick={() => void suggest()} disabled={!canSend} title="Ask the AI provider (Ctrl+Enter)">{result ? 'Suggest again' : 'Suggest'}</button>
  {/snippet}
</Modal>

<style>
  .info {
    display: flex;
    gap: 6px;
    align-items: flex-start;
    margin: 0 0 10px;
    padding: 6px 8px;
    border-radius: 3px;
    background: var(--color-info-bg);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 0 0 8px;
  }

  .field > span {
    color: var(--color-text-muted);
    font-size: 12px;
  }

  textarea {
    resize: vertical;
    padding: 6px 8px;
    border: 1px solid var(--color-border-strong);
    border-radius: 3px;
  }

  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
  }

  .inline select {
    min-width: 200px;
    padding: 3px 6px;
    border: 1px solid var(--color-border-strong);
    border-radius: 3px;
    background: var(--color-pane);
  }

  .check {
    display: flex;
    gap: 6px;
    align-items: center;
    margin: 0 0 8px;
    font-size: 12px;
  }

  .progress,
  .error {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 6px 0;
  }

  .progress-text {
    flex: 1;
    color: var(--color-text-muted);
  }

  .error {
    align-items: flex-start;
    color: var(--color-danger);
    overflow-wrap: anywhere;
  }

  .error span {
    flex: 1;
  }

  .result {
    margin-top: 10px;
    padding-top: 10px;
    border-top: 1px solid var(--color-border);
  }

  h3 {
    margin: 0 0 6px;
    font-size: 14px;
  }

  h3 .muted {
    font-size: 12px;
    font-weight: 400;
  }

  h4 {
    margin: 8px 0 4px;
    color: var(--color-text-muted);
    font-size: 12px;
  }

  .diagnostics {
    margin: 6px 0;
    padding: 0;
    list-style: none;
  }

  .diagnostics li {
    display: flex;
    gap: 6px;
    align-items: flex-start;
    margin: 2px 0;
  }

  .diagnostics li.error {
    color: var(--color-danger);
  }

  .diagnostics li.warning {
    color: var(--color-accent-dark);
  }

  .ok {
    display: flex;
    gap: 6px;
    align-items: center;
    margin: 6px 0;
    color: var(--color-brand);
  }

  .eval {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
    margin: 8px 0;
  }

  .eval-error {
    color: var(--color-danger);
    overflow-wrap: anywhere;
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

  .small-btn:hover:not(:disabled) {
    background: var(--color-hover);
  }

  .small-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .call {
    margin: 8px 0 0;
    font-size: 11px;
  }

  .spinner {
    display: inline-block;
    flex: none;
    width: 14px;
    height: 14px;
    border: 2px solid var(--color-border-strong);
    border-top-color: var(--color-brand);
    border-radius: 50%;
    vertical-align: -3px;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
