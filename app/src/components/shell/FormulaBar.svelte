<script lang="ts">
  import Icon from '../Icon.svelte'
  import { NOT_AVAILABLE } from '../../lib/shell'

  // Desktop's formula bar, read-only: shown under the ribbon when a measure, calculated column
  // or calculated table is selected. Cancel / commit are disabled (DashCraft does not edit yet).
  let {
    name,
    expression,
    kind,
    explain = null,
  }: {
    name: string
    expression: string
    kind: string
    /** "Explain with AI" for a measure (opt-in AI; disabled with a reason until it is on). */
    explain?: { enabled: boolean; title: string; run: () => void } | null
  } = $props()

  let expanded = $state(false)
  const text = $derived(`${name} = ${expression.replace(/^\s*\n/, '')}`)
  const lines = $derived(text.split(/\r\n?|\n/))
</script>

<div class="formula-bar" class:expanded title={`${kind} (read-only)`}>
  <button class="icon-btn" disabled title={`Cancel — ${NOT_AVAILABLE}`} aria-label="Cancel"><Icon name="close" size={16} /></button>
  <button class="icon-btn" disabled title={`Commit — ${NOT_AVAILABLE}`} aria-label="Commit"><Icon name="check" size={16} /></button>
  <div class="formula" role="textbox" aria-readonly="true" aria-multiline="true" aria-label={`${kind} formula`} tabindex="0">{#each expanded ? lines : lines.slice(0, 1) as line, i (i)}<span class="line"><span class="no">{i + 1}</span>{line}</span>{/each}</div>
  {#if explain}
    <button class="icon-btn" disabled={!explain.enabled} onclick={explain.run} title={explain.title} aria-label="Explain with AI"><Icon name="ai" size={16} /></button>
  {/if}
  {#if lines.length > 1}
    <button class="icon-btn" onclick={() => (expanded = !expanded)} title={expanded ? 'Collapse the formula bar' : 'Expand the formula bar'} aria-label={expanded ? 'Collapse the formula bar' : 'Expand the formula bar'}>
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

  .no {
    display: inline-block;
    width: 2.5ch;
    margin-right: 8px;
    color: var(--color-text-subtle);
    text-align: right;
    user-select: none;
    -webkit-user-select: none;
  }
</style>
