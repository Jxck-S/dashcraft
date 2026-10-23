<script module lang="ts">
  import type { DaxQueryResult } from '../../lib/backend'

  /** One query tab of the DAX query view (kept by the parent across view switches). */
  export interface DaxTab {
    id: number
    name: string
    text: string
    result: DaxQueryResult | null
    error: string | null
    running: boolean
    cancelled: boolean
    resultIndex: number
  }

  export function newTab(id: number, text: string): DaxTab {
    return { id, name: `Query ${id}`, text, result: null, error: null, running: false, cancelled: false, resultIndex: 0 }
  }
</script>

<script lang="ts">
  import Icon from '../Icon.svelte'
  import * as backend from '../../lib/backend'

  // DAX query view: a query editor (line numbers, Ctrl/⌘ + / to comment), the command bar
  // (Run = F5 or Ctrl/⌘ + Enter, Cancel), the results grid ("Result n of m", copy as TSV) and
  // query tabs at the bottom. Queries run on the backend over the loaded data; nothing changes
  // the model.
  let {
    tabs = $bindable(),
    active = $bindable(),
    hasData,
    nextId,
  }: {
    tabs: DaxTab[]
    active: number
    hasData: boolean
    /** Allocates a query tab number. */
    nextId: () => number
  } = $props()

  let editor = $state<HTMLTextAreaElement | null>(null)
  let gutter = $state<HTMLDivElement | null>(null)
  let copied = $state(false)

  const tab = $derived(tabs.find((t) => t.id === active) ?? tabs[0])
  const lineCount = $derived((tab?.text.match(/\n/g)?.length ?? 0) + 1)
  const result = $derived(tab?.result?.tables[tab.resultIndex] ?? tab?.result?.tables[0] ?? null)

  function update(id: number, patch: Partial<DaxTab>) {
    tabs = tabs.map((t) => (t.id === id ? { ...t, ...patch } : t))
  }

  /** Runs the current query (or the selected text, as in Desktop). */
  export async function run() {
    const t = tab
    if (!t || t.running) return
    const selection = editor && editor.selectionStart !== editor.selectionEnd ? t.text.slice(editor.selectionStart, editor.selectionEnd) : ''
    const query = selection.trim() ? selection : t.text
    update(t.id, { running: true, cancelled: false, error: null })
    const outcome = await backend.runDaxQuery(query)
    if (outcome.ok) update(t.id, { running: false, result: outcome.value, resultIndex: 0, error: null })
    else if (outcome.kind === 'cancelled') update(t.id, { running: false, cancelled: true })
    else update(t.id, { running: false, error: outcome.error })
  }

  export async function cancel() {
    await backend.cancelDaxQuery()
  }

  /** Comments (`//`) or uncomments the selected lines. */
  export function comment(add: boolean) {
    const t = tab
    if (!t || !editor) return
    const { selectionStart: s, selectionEnd: e } = editor
    const start = t.text.lastIndexOf('\n', s - 1) + 1
    const endBreak = t.text.indexOf('\n', Math.max(s, e - 1))
    const end = endBreak < 0 ? t.text.length : endBreak
    const lines = t.text.slice(start, end).split('\n')
    const changed = lines.map((l) => (add ? `// ${l}` : l.replace(/^(\s*)\/\/ ?/, '$1'))).join('\n')
    update(t.id, { text: t.text.slice(0, start) + changed + t.text.slice(end) })
    queueMicrotask(() => editor?.setSelectionRange(start, start + changed.length))
  }

  /** Opens `text` in a new query tab (Quick queries) and optionally runs it. */
  export function addQuery(text: string, runNow: boolean) {
    const t = newTab(nextId(), text)
    tabs = [...tabs, t]
    active = t.id
    if (runNow) queueMicrotask(() => void run())
  }

  function closeTab(id: number) {
    if (tabs.length <= 1) return
    const i = tabs.findIndex((t) => t.id === id)
    tabs = tabs.filter((t) => t.id !== id)
    if (active === id) active = (tabs[Math.max(0, i - 1)] ?? tabs[0]).id
  }

  function keydown(event: KeyboardEvent) {
    const mod = event.ctrlKey || event.metaKey
    if (event.key === 'F5' || (mod && event.key === 'Enter')) {
      event.preventDefault()
      event.stopPropagation()
      void run()
    } else if (mod && event.key === '/') {
      event.preventDefault()
      const t = tab
      if (!t || !editor) return
      const start = t.text.lastIndexOf('\n', editor.selectionStart - 1) + 1
      comment(!/^\s*\/\//.test(t.text.slice(start)))
    } else if (event.key === 'Tab' && !event.shiftKey && editor) {
      event.preventDefault()
      const { selectionStart: s, selectionEnd: e } = editor
      const t = tab
      if (!t) return
      update(t.id, { text: `${t.text.slice(0, s)}\t${t.text.slice(e)}` })
      queueMicrotask(() => editor?.setSelectionRange(s + 1, s + 1))
    }
  }

  function syncScroll() {
    if (gutter && editor) gutter.scrollTop = editor.scrollTop
  }

  async function copyResult() {
    if (!result) return
    const head = result.columns.map((c) => c.name).join('\t')
    const body = result.rows.map((r) => r.map((c) => c.text).join('\t')).join('\n')
    try {
      await navigator.clipboard.writeText(`${head}\n${body}`)
      copied = true
      setTimeout(() => (copied = false), 1200)
    } catch {
      copied = false
    }
  }
</script>

<div class="dax-view">
  <div class="command-bar">
    {#if tab?.running}
      <button class="run cancel" onclick={() => void cancel()} title="Cancel the running query">
        <Icon name="stop" size={16} /> Cancel
      </button>
    {:else}
      <button class="run" onclick={() => void run()} disabled={!tab || !backend.hasBackend} title="Run the query, or the selected lines (F5, Ctrl+Enter)">
        <Icon name="play" size={16} /> Run
      </button>
    {/if}
    <span class="hint">
      {#if tab?.running}<span class="spinner" aria-hidden="true"></span> Running…
      {:else if !hasData}No data loaded: Refresh first, or queries return empty / blank results.
      {:else}Read-only: queries never change the model.{/if}
    </span>
  </div>

  <div class="editor-wrap">
    <div class="gutter" bind:this={gutter} aria-hidden="true">
      {#each Array.from({ length: lineCount }, (_, i) => i + 1) as n (n)}<div>{n}</div>{/each}
    </div>
    {#if tab}
      <textarea
        bind:this={editor}
        class="editor"
        spellcheck="false"
        autocomplete="off"
        wrap="off"
        value={tab.text}
        oninput={(e) => update(tab.id, { text: (e.currentTarget as HTMLTextAreaElement).value })}
        onkeydown={keydown}
        onscroll={syncScroll}
        aria-label="DAX query"
      ></textarea>
    {/if}
  </div>

  <section class="results" aria-label="Results">
    <header>
      <strong>Results</strong>
      {#if tab?.result && tab.result.tables.length > 1}
        <select value={tab.resultIndex} onchange={(e) => update(tab.id, { resultIndex: Number((e.currentTarget as HTMLSelectElement).value) })} aria-label="Result table">
          {#each tab.result.tables as _, i (i)}<option value={i}>Result {i + 1} of {tab.result.tables.length}</option>{/each}
        </select>
      {/if}
      {#if result}
        <span class="muted">
          {result.totalRows.toLocaleString()} row{result.totalRows === 1 ? '' : 's'}{result.truncated ? ` (first ${result.rows.length.toLocaleString()} shown)` : ''} · {tab?.result?.durationMs.toLocaleString()} ms
        </span>
        <button class="cmd" onclick={copyResult} title="Copy the result as tab-separated text"><Icon name="copy" size={14} /> {copied ? 'Copied' : 'Copy'}</button>
      {/if}
    </header>
    <div class="grid">
      {#if tab?.error}
        <p class="error" role="alert"><Icon name="error" size={14} /> {tab.error}</p>
      {:else if tab?.cancelled && !tab.result}
        <p class="empty-hint">The query was cancelled.</p>
      {:else if !tab?.result}
        <p class="empty-hint">Run a query to see its results here.</p>
      {:else if !result}
        <p class="empty-hint">The query returned no table.</p>
      {:else}
        <table>
          <thead>
            <tr>{#each result.columns as c, i (i)}<th class:num={c.numeric} title={`${c.name} · ${c.dataType}${c.formatString ? ` · ${c.formatString}` : ''}`}>{c.name}</th>{/each}</tr>
          </thead>
          <tbody>
            {#each result.rows as row, r (r)}
              <tr>{#each row as cell, i (i)}<td class:num={result.columns[i]?.numeric} class:blank={cell.isBlank}>{cell.isBlank ? '' : cell.text}</td>{/each}</tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </div>
  </section>

  <nav class="query-tabs" aria-label="Queries">
    {#each tabs as t (t.id)}
      <span class="qtab" class:active={t.id === tab?.id}>
        <button class="qname" onclick={() => (active = t.id)}>
          {#if t.running}<span class="spinner" aria-hidden="true" title="Running"></span>
          {:else if t.error}<span class="state error" title="Error"><Icon name="error" size={12} /></span>
          {:else if t.cancelled}<span class="state" title="Cancelled"><Icon name="stop" size={12} /></span>
          {:else if t.result}<span class="state ok" title="Succeeded"><Icon name="check" size={12} /></span>{/if}
          {t.name}
        </button>
        {#if tabs.length > 1}
          <button class="qclose" onclick={() => closeTab(t.id)} title="Close the query" aria-label={`Close ${t.name}`}><Icon name="close" size={10} /></button>
        {/if}
      </span>
    {/each}
    <button class="qadd" onclick={() => addQuery('EVALUATE\n\t', false)} title="New query" aria-label="New query"><Icon name="add" size={16} /></button>
  </nav>
</div>

<style>
  .dax-view {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--color-pane);
  }

  .command-bar {
    display: flex;
    flex: none;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--color-border);
  }

  .run {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 14px;
    border: none;
    border-radius: 4px;
    background: var(--color-brand);
    color: #fff;
    font-weight: 600;
    cursor: pointer;
  }

  .run:hover:not(:disabled) {
    background: var(--color-brand-hover);
  }

  .run:disabled {
    background: var(--color-border-strong);
    cursor: default;
  }

  .run.cancel {
    background: var(--color-danger);
  }

  .state {
    display: inline-flex;
    color: var(--color-text-muted);
  }

  .state.ok {
    color: var(--color-brand);
  }

  .state.error {
    color: var(--color-danger);
  }

  .cmd {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 26px;
    padding: 0 10px;
    border: 1px solid var(--color-border-strong);
    border-radius: 4px;
    background: var(--color-pane);
    font-size: 12px;
    cursor: pointer;
  }

  .cmd:hover:not(:disabled) {
    background: var(--color-hover);
  }

  .cmd:disabled {
    color: var(--color-text-disabled);
    cursor: default;
  }

  .hint {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-left: 8px;
    color: var(--color-text-muted);
    font-size: 12px;
  }

  .editor-wrap {
    display: flex;
    flex: 1 1 50%;
    min-height: 120px;
    border-bottom: 1px solid var(--color-border-strong);
  }

  .gutter {
    flex: none;
    width: 48px;
    padding: 8px 8px 8px 0;
    overflow: hidden;
    background: var(--color-code-bg);
    color: var(--color-text-subtle);
    font-family: var(--font-mono);
    font-size: 13px;
    line-height: 20px;
    text-align: right;
    user-select: none;
    -webkit-user-select: none;
  }

  .editor {
    flex: 1;
    min-width: 0;
    padding: 8px 10px;
    border: none;
    outline: none;
    resize: none;
    background: var(--color-pane);
    color: var(--color-text);
    font-family: var(--font-mono);
    font-size: 13px;
    line-height: 20px;
    tab-size: 4;
    white-space: pre;
  }

  .results {
    display: flex;
    flex: 1 1 50%;
    flex-direction: column;
    min-height: 100px;
  }

  .results header {
    display: flex;
    flex: none;
    align-items: center;
    gap: 10px;
    padding: 4px 10px;
    border-bottom: 1px solid var(--color-border);
    font-size: 12px;
  }

  .results select {
    font-size: 12px;
  }

  .grid {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }

  table {
    border-collapse: collapse;
    font-size: 12px;
    white-space: nowrap;
  }

  th,
  td {
    max-width: 360px;
    padding: 3px 10px;
    overflow: hidden;
    border-right: 1px solid var(--color-border);
    border-bottom: 1px solid var(--color-border);
    text-align: left;
    text-overflow: ellipsis;
  }

  th {
    position: sticky;
    top: 0;
    background: var(--color-chrome);
    font-weight: 600;
  }

  .num {
    font-variant-numeric: tabular-nums;
    text-align: right;
  }

  tbody tr:nth-child(even) {
    background: #fafafa;
  }

  .error {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    margin: 10px;
    color: var(--color-danger);
    font-family: var(--font-mono);
    font-size: 12px;
    white-space: pre-wrap;
  }

  .query-tabs {
    display: flex;
    flex: none;
    height: var(--pagetabs-height);
    border-top: 1px solid var(--color-border);
    background: var(--color-pagetabs);
  }

  .qtab {
    position: relative;
    display: inline-flex;
    align-items: center;
    border-right: 1px solid var(--color-border);
  }

  .qtab.active {
    background: var(--color-pane);
  }

  .qtab.active::after {
    content: '';
    position: absolute;
    right: 0;
    bottom: 0;
    left: 0;
    height: 3px;
    background: var(--color-brand);
  }

  .qname {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 100%;
    padding: 0 8px 0 16px;
    border: none;
    background: transparent;
    font-size: 13px;
    cursor: pointer;
  }

  .qtab.active .qname {
    font-weight: 600;
  }

  .qclose,
  .qadd {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    margin-right: 6px;
    border: none;
    border-radius: 3px;
    background: transparent;
    color: var(--color-text-muted);
    cursor: pointer;
  }

  .qadd {
    width: 40px;
    height: 100%;
    margin: 0;
  }

  .qclose:hover,
  .qadd:hover {
    background: var(--color-hover);
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
</style>
