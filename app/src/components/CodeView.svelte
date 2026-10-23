<script lang="ts">
  import type { CodeLanguage } from '../lib/bindings'

  // Read-only code view: monospace text with line numbers. No editor (Monaco) yet, to keep the
  // bundle small. Line numbers are CSS counters, so copying the text never includes them.
  let {
    text,
    language,
    label,
    compact = false,
    languageLabel,
  }: { text: string; language: CodeLanguage; label?: string; compact?: boolean; languageLabel?: string } = $props()

  const LANGUAGE_LABELS: Record<CodeLanguage, string> = { dax: 'DAX', m: 'M', json: 'JSON', text: 'Text' }

  const lines = $derived(text.replace(/\r\n?/g, '\n').split('\n'))
  const gutter = $derived(`${String(lines.length).length + 1}ch`)
  let copied = $state(false)

  async function copy() {
    try {
      await navigator.clipboard.writeText(text)
      copied = true
      setTimeout(() => (copied = false), 1200)
    } catch {
      copied = false
    }
  }
</script>

<figure class="code-view" class:compact>
  <figcaption>
    <span class="lang">{languageLabel ?? LANGUAGE_LABELS[language]}</span>
    {#if label}<span class="label">{label}</span>{/if}
    <span class="spacer"></span>
    <span class="muted">{lines.length} {lines.length === 1 ? 'line' : 'lines'}</span>
    <button class="copy" onclick={copy} title="Copy to clipboard">{copied ? 'Copied' : 'Copy'}</button>
  </figcaption>
  <pre style:--gutter={gutter}><code>{#each lines as line, i (i)}<span class="line">{line}</span>{/each}</code></pre>
</figure>

<style>
  .code-view {
    display: flex;
    flex-direction: column;
    min-height: 0;
    margin: 0;
    border: 1px solid var(--color-border);
    border-radius: 2px;
    background: var(--color-code-bg);
  }

  figcaption {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 6px 3px 8px;
    border-bottom: 1px solid var(--color-border);
    background: var(--color-chrome);
    font-size: 11px;
  }

  .lang {
    font-weight: 600;
    letter-spacing: 0.04em;
    color: var(--color-brand);
  }

  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .spacer {
    flex: 1;
  }

  .copy {
    padding: 1px 8px;
    border: 1px solid var(--color-border-strong);
    border-radius: 2px;
    background: var(--color-pane);
    font-size: 11px;
    cursor: pointer;
  }

  pre {
    flex: 1;
    min-height: 0;
    margin: 0;
    padding: 6px 0;
    overflow: auto;
    counter-reset: line;
    line-height: 1.5;
    tab-size: 4;
  }

  .compact pre {
    max-height: 320px;
  }

  code {
    display: block;
    width: max-content;
    min-width: 100%;
  }

  .line {
    display: block;
    padding-right: 12px;
    white-space: pre;
  }

  .line::before {
    counter-increment: line;
    content: counter(line);
    display: inline-block;
    box-sizing: content-box;
    width: var(--gutter);
    margin-right: 12px;
    padding: 0 6px 0 4px;
    border-right: 1px solid var(--color-border);
    text-align: right;
    color: var(--color-text-subtle);
    user-select: none;
    -webkit-user-select: none;
  }
</style>
