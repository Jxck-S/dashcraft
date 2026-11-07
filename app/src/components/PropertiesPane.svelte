<script module lang="ts">
  /** The editable properties of the selected measure, column or table (`model.*.update`). */
  export interface PropertyEdit {
    /** The object's id (`details.id`): the fields belong to this object. */
    id: number
    description: string | null
    /** Shown for a measure or column; `undefined` hides the field (tables). */
    displayFolder?: string | null
    formatString?: string | null
    /** Applies one changed field (`''` clears it); resolves an error message or `null`. */
    commit: (field: 'description' | 'displayFolder' | 'formatString', value: string) => Promise<string | null>
  }
</script>

<script lang="ts">
  import CodeView from './CodeView.svelte'
  import Icon from './Icon.svelte'
  import PropertyField from './PropertyField.svelte'
  import type { ObjectDetailsDto, ObjectKindDto } from '../lib/bindings'

  let {
    details,
    loading,
    error,
    explain = null,
    edit = null,
  }: {
    details: ObjectDetailsDto | null
    loading: boolean
    error: string | null
    /** "Explain with AI" for the selected measure (opt-in AI; disabled with a reason until it is on). */
    explain?: { enabled: boolean; title: string; run: () => void } | null
    /** Editable fields of the selected object; read-only without. */
    edit?: PropertyEdit | null
  } = $props()

  // The edited object's fields replace their read-only rows.
  const editing = $derived(edit && details && edit.id === details.id ? edit : null)
  const EDITED_LABELS = ['Format string', 'Display folder']
  const properties = $derived(editing ? (details?.properties ?? []).filter((p) => !EDITED_LABELS.includes(p.label)) : (details?.properties ?? []))

  const KIND_LABELS: Record<ObjectKindDto, string> = {
    model: 'Model',
    database: 'Database',
    table: 'Table',
    column: 'Column',
    measure: 'Measure',
    partition: 'Partition',
    hierarchy: 'Hierarchy',
    level: 'Level',
    relationship: 'Relationship',
    expression: 'Expression',
    queryGroup: 'Query group',
    role: 'Role',
    tablePermission: 'Table permission',
    culture: 'Culture',
    perspective: 'Perspective',
    report: 'Report',
    page: 'Page',
    visual: 'Visual',
    filter: 'Filter',
  }
</script>

<section class="properties" aria-label="Properties" aria-busy={loading}>
  {#if error}
    <p class="error-hint">{error}</p>
  {:else if !details}
    <p class="empty-hint">{loading ? 'Loading…' : 'Select a table, field, query, page, visual or filter.'}</p>
  {:else}
    <header class="object-header" class:stale={loading}>
      <span class="kind">{KIND_LABELS[details.kind]}</span>
      <h3 title={details.name}>{details.name}</h3>
      {#if details.parent}<span class="parent muted" title={details.parent}>in {details.parent}</span>{/if}
      {#if explain && details.kind === 'measure'}
        <button class="explain" disabled={!explain.enabled} onclick={explain.run} title={explain.title}>
          <Icon name="ai" size={14} />
          Explain with AI
        </button>
      {/if}
    </header>

    {#if editing}
      {#key editing.id}
        <div class="edit" role="group" aria-label={`Edit ${details.name}`}>
          <PropertyField id="prop-description" label="Description" value={editing.description} multiline placeholder="Add a description" oncommit={(v) => editing.commit('description', v)} />
          {#if editing.displayFolder !== undefined}
            <PropertyField id="prop-display-folder" label="Display folder" value={editing.displayFolder} placeholder="None" oncommit={(v) => editing.commit('displayFolder', v)} />
          {/if}
          {#if editing.formatString !== undefined}
            <PropertyField id="prop-format-string" label="Format string" value={editing.formatString} mono placeholder="None" oncommit={(v) => editing.commit('formatString', v)} />
          {/if}
        </div>
      {/key}
    {:else if details.description}
      <p class="description">{details.description}</p>
    {/if}

    {#if properties.length > 0}
      <dl class="props">
        {#each properties as p, i (i)}
          <dt>{p.label}</dt>
          <dd class:mono={p.mono} title={p.tooltip ?? undefined}>{p.value}</dd>
        {/each}
      </dl>
    {/if}

    {#each details.code as block, i (i)}
      <div class="code-block">
        <CodeView text={block.text} language={block.language} label={block.label} compact />
      </div>
    {/each}

    {#if details.annotations.length > 0}
      <details class="section">
        <summary>Annotations ({details.annotations.length})</summary>
        <dl class="props">
          {#each details.annotations as a, i (i)}
            <dt class="mono">{a.label}</dt>
            <dd class="mono">{a.value}</dd>
          {/each}
        </dl>
      </details>
    {/if}

    {#if details.extra.length > 0}
      <details class="section">
        <summary title="Properties DashCraft does not model yet; kept for lossless round-tripping">Other properties ({details.extra.length})</summary>
        <dl class="props">
          {#each details.extra as a, i (i)}
            <dt class="mono">{a.label}</dt>
            <dd class="mono">{a.value}</dd>
          {/each}
        </dl>
      </details>
    {/if}

    {#if details.origin}
      <p class="origin" title="Source file in the opened folder or archive">
        <span class="muted">Source</span>
        <code>{details.origin}</code>
      </p>
    {/if}
  {/if}
</section>

<style>
  .properties {
    height: 100%;
    overflow: auto;
    padding-bottom: 16px;
  }

  .object-header {
    padding: 0 12px 8px;
    border-bottom: 1px solid var(--color-border);
  }

  .object-header.stale {
    opacity: 0.6;
  }

  .kind {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--color-brand);
  }

  h3 {
    margin: 2px 0 0;
    font-size: 15px;
    overflow-wrap: anywhere;
  }

  .parent {
    display: block;
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .explain {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-top: 6px;
    padding: 2px 9px;
    border: 1px solid var(--color-border-strong);
    border-radius: 3px;
    background: var(--color-pane);
    font-size: 12px;
    cursor: pointer;
  }

  .explain:hover:not(:disabled) {
    background: var(--color-hover);
  }

  .explain:disabled {
    color: var(--color-text-disabled);
    cursor: default;
  }

  .edit {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px 2px;
  }

  .description {
    margin: 8px 12px 0;
    color: var(--color-text-muted);
    white-space: pre-wrap;
  }

  .props {
    display: grid;
    grid-template-columns: minmax(80px, max-content) 1fr;
    gap: 5px 12px;
    margin: 0;
    padding: 10px 12px;
  }

  dt {
    color: var(--color-text-muted);
    overflow-wrap: anywhere;
  }

  dd {
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
    white-space: pre-wrap;
  }

  dd[title] {
    text-decoration: underline dotted var(--color-text-subtle);
    text-underline-offset: 3px;
    cursor: help;
  }

  .mono {
    font-family: var(--font-mono);
    font-size: 12px;
  }

  .code-block {
    margin: 0 12px 10px;
  }

  .section {
    margin: 0 12px 6px;
    border-top: 1px solid var(--color-border);
  }

  .section summary {
    padding: 6px 0;
    color: var(--color-text-muted);
    cursor: pointer;
  }

  .section .props {
    padding: 0 0 8px;
  }

  .origin {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 6px 12px 0;
    padding-top: 8px;
    border-top: 1px solid var(--color-border);
    font-size: 12px;
  }

  .origin code {
    overflow-wrap: anywhere;
  }

  .error-hint {
    margin: 12px;
    color: var(--color-danger);
  }
</style>
