<script lang="ts">
  import { renderMarkdown } from '../../lib/markdown'

  // Markdown from an AI answer. `renderMarkdown` escapes every character of the text and emits
  // only its own fixed tags (no attributes from the input, links as plain text), so model text
  // such as `<script>` or `<img onerror>` is shown, never run (tests/markdown.test.ts).
  let { text }: { text: string } = $props()

  const html = $derived(renderMarkdown(text))
</script>

<!-- Fully escaped output of renderMarkdown (see above), never raw model text. -->
<!-- eslint-disable-next-line svelte/no-at-html-tags -->
<div class="markdown">{@html html}</div>

<style>
  .markdown {
    line-height: 1.5;
    overflow-wrap: anywhere;
  }

  .markdown :global(p) {
    margin: 0 0 8px;
    white-space: pre-line;
  }

  .markdown :global(h3),
  .markdown :global(h4),
  .markdown :global(h5),
  .markdown :global(h6) {
    margin: 12px 0 6px;
    font-size: 13px;
    font-weight: 600;
  }

  .markdown :global(h3) {
    font-size: 15px;
  }

  .markdown :global(h4) {
    font-size: 14px;
  }

  .markdown :global(ul),
  .markdown :global(ol) {
    margin: 0 0 8px;
    padding-left: 20px;
  }

  .markdown :global(li) {
    margin: 2px 0;
    white-space: pre-line;
  }

  .markdown :global(code) {
    padding: 0 3px;
    border-radius: 3px;
    background: var(--color-code-bg);
    box-shadow: inset 0 0 0 1px var(--color-border);
  }

  .markdown :global(pre.md-code) {
    margin: 0 0 8px;
    padding: 6px 8px;
    overflow: auto;
    border: 1px solid var(--color-border);
    border-radius: 3px;
    background: var(--color-code-bg);
  }

  .markdown :global(pre.md-code code) {
    padding: 0;
    background: none;
    box-shadow: none;
    white-space: pre;
  }

  .markdown :global(blockquote) {
    margin: 0 0 8px;
    padding: 0 0 0 10px;
    border-left: 3px solid var(--color-border-strong);
    color: var(--color-text-muted);
  }

  .markdown :global(hr) {
    margin: 10px 0;
    border: none;
    border-top: 1px solid var(--color-border);
  }

  .markdown :global(table.md-table) {
    margin: 0 0 8px;
    border-collapse: collapse;
    font-size: 12px;
  }

  .markdown :global(table.md-table th),
  .markdown :global(table.md-table td) {
    padding: 3px 6px;
    border: 1px solid var(--color-border);
    text-align: left;
    vertical-align: top;
  }

  .markdown :global(.md-url),
  .markdown :global(.md-image) {
    color: var(--color-text-muted);
    font-size: 12px;
  }

  .markdown :global(.md-link) {
    text-decoration: underline dotted var(--color-text-subtle);
  }

  .markdown :global(.md-cut) {
    color: var(--color-text-subtle);
    font-style: italic;
  }
</style>
