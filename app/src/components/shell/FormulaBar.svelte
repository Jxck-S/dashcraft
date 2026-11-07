<script lang="ts">
  import Icon from '../Icon.svelte'
  import { formulaText } from '../../lib/editing'

  // Desktop's formula bar, shown under the ribbon when a measure, calculated column or calculated
  // table is selected: `Name = expression`. With `oncommit` it is editable: Enter or ✓ commits
  // (the name and the expression as one undo step), Shift+Enter starts a new line, Esc or ✕
  // cancels. Without it (no local model) it is read-only.
  let {
    name,
    expression,
    kind,
    explain = null,
    oncommit = null,
  }: {
    name: string
    expression: string
    kind: string
    /** "Explain with AI" for a measure (opt-in AI; disabled with a reason until it is on). */
    explain?: { enabled: boolean; title: string; run: () => void } | null
    /** Applies the edited formula; resolves an error message, or `null` once it is applied. */
    oncommit?: ((text: string) => Promise<string | null>) | null
  } = $props()

  let expanded = $state(false)
  let saving = $state(false)
  let editor = $state<HTMLTextAreaElement | null>(null)
  let gutter = $state<HTMLDivElement | null>(null)
  const text = $derived(formulaText(name, expression))
  // The draft follows the selected object until the user types; a new object resets it.
  let draft = $derived(text)
  let error = $derived.by((): string | null => {
    void text
    return null
  })
  const lines = $derived((oncommit ? draft : text).split(/\r\n?|\n/))
  const changed = $derived(draft !== text)
  const rows = $derived(expanded ? Math.min(Math.max(lines.length, 3), 14) : 1)
  const errorId = `formula-error-${Math.random().toString(36).slice(2)}`

  function cancel() {
    draft = text
    error = null
  }

  async function commit() {
    if (!oncommit || saving || !changed) return
    saving = true
    const failure = await oncommit(draft)
    saving = false
    if (failure) {
      error = failure
      editor?.focus()
    } else {
      draft = text
      error = null
    }
  }

  function keydown(event: KeyboardEvent) {
    if (event.key === 'Enter' && !event.shiftKey && !event.altKey && !event.ctrlKey && !event.metaKey) {
      event.preventDefault()
      void commit()
    } else if (event.key === 'Enter' && event.shiftKey && !expanded) {
      expanded = true
    } else if (event.key === 'Escape' && (changed || error)) {
      event.preventDefault()
      event.stopPropagation()
      cancel()
    }
  }

  function syncScroll() {
    if (gutter && editor) gutter.scrollTop = editor.scrollTop
  }
</script>

<div class="formula-bar" class:expanded class:editable={!!oncommit} title={oncommit ? undefined : `${kind} (read-only)`}>
  <button class="icon-btn" disabled={!oncommit || !changed || saving} onclick={cancel} title={oncommit ? 'Cancel (Esc)' : `${kind} (read-only)`} aria-label="Cancel"><Icon name="close" size={16} /></button>
  <button class="icon-btn" disabled={!oncommit || !changed || saving} onclick={() => void commit()} title={oncommit ? 'Commit (Enter)' : `${kind} (read-only)`} aria-label="Commit"><Icon name="check" size={16} /></button>
  {#if oncommit}
    <div class="editor-wrap">
      <div class="editor" class:invalid={!!error}>
        <div class="gutter" bind:this={gutter} aria-hidden="true" style:height={`${rows * 18 + 8}px`}>{#each lines as _, i (i)}<span>{i + 1}</span>{/each}</div>
        <textarea
          bind:this={editor}
          bind:value={draft}
          {rows}
          wrap="off"
          spellcheck="false"
          autocomplete="off"
          readonly={saving}
          aria-label={`${kind} formula`}
          aria-invalid={!!error}
          aria-describedby={error ? errorId : undefined}
          onkeydown={keydown}
          onscroll={syncScroll}
        ></textarea>
      </div>
      <div class="error" id={errorId} aria-live="polite">{error ?? ''}</div>
    </div>
  {:else}
    <div class="formula" role="textbox" aria-readonly="true" aria-multiline="true" aria-label={`${kind} formula`} tabindex="0">{#each expanded ? lines : lines.slice(0, 1) as line, i (i)}<span class="line"><span class="no">{i + 1}</span>{line}</span>{/each}</div>
  {/if}
  {#if explain}
    <button class="icon-btn" disabled={!explain.enabled} onclick={explain.run} title={explain.title} aria-label="Explain with AI"><Icon name="ai" size={16} /></button>
  {/if}
  {#if lines.length > 1 || oncommit}
    <button class="icon-btn" onclick={() => (expanded = !expanded)} title={expanded ? 'Collapse the formula bar' : 'Expand the formula bar'} aria-label={expanded ? 'Collapse the formula bar' : 'Expand the formula bar'} aria-expanded={expanded}>
      <Icon name={expanded ? 'chevronUp' : 'chevronDown'} size={14} />
    </button>
  {/if}
</div>

<style>
  .formula-bar {
    display: flex;
    flex: none;
    align-items: flex-start;
    gap: 2px;
    min-height: 32px;
    padding: 3px 6px;
    border-bottom: 1px solid var(--color-border);
    background: var(--color-pane);
  }

  .icon-btn {
    display: grid;
    flex: none;
    place-items: center;
    width: 26px;
    height: 26px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--color-text);
    cursor: pointer;
  }

  .icon-btn:disabled {
    color: var(--color-text-disabled);
    cursor: default;
  }

  .icon-btn:hover:not(:disabled) {
    background: var(--color-hover);
  }

  .formula {
    flex: 1;
    min-width: 0;
    max-height: 220px;
    margin: 0 0 0 6px;
    padding: 4px 8px;
    overflow: auto;
    border: 1px solid var(--color-border);
    border-radius: 3px;
    background: var(--color-code-bg);
    font-family: var(--font-mono);
    font-size: 12px;
    line-height: 1.5;
  }

  .formula-bar:not(.expanded) .formula {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .line {
    display: block;
    white-space: pre;
  }

  .no,
  .gutter span {
    display: inline-block;
    width: 2.5ch;
    margin-right: 8px;
    color: var(--color-text-muted);
    text-align: right;
    user-select: none;
    -webkit-user-select: none;
  }

  .editor-wrap {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
    margin-left: 6px;
  }

  .editor {
    display: flex;
    align-items: flex-start;
    min-width: 0;
    border: 1px solid var(--color-border);
    border-bottom-color: var(--color-border-accessible);
    border-radius: 3px;
    background: var(--color-code-bg);
    font-family: var(--font-mono);
    font-size: 12px;
    line-height: 18px;
  }

  .editor:focus-within {
    border-color: var(--color-brand);
    outline: 2px solid var(--color-brand);
  }

  .editor.invalid {
    border-color: var(--color-danger);
  }

  .gutter {
    box-sizing: border-box;
    flex: none;
    padding: 4px 0 4px 8px;
    overflow: hidden;
  }

  .gutter span {
    display: block;
    margin-right: 0;
  }

  textarea {
    flex: 1;
    min-width: 0;
    padding: 4px 8px;
    border: none;
    outline: none;
    background: transparent;
    color: var(--color-text);
    font: inherit;
    line-height: inherit;
    white-space: pre;
    resize: none;
  }

  .formula-bar:not(.expanded) textarea {
    overflow: hidden;
  }

  .error {
    color: var(--color-danger);
    font-size: 12px;
  }

  .error:empty {
    display: none;
  }
</style>
