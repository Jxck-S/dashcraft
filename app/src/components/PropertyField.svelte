<script lang="ts">
  import { changedValue } from '../lib/editing'

  // One editable Properties pane field (description, display folder, format string). A single-line
  // field commits on Enter or when it loses focus; a multi-line one on Ctrl+Enter or focus loss.
  // Esc reverts. Empty text clears the property.
  let {
    id,
    label,
    value,
    multiline = false,
    mono = false,
    placeholder = '',
    oncommit,
  }: {
    id: string
    label: string
    value: string | null
    multiline?: boolean
    mono?: boolean
    placeholder?: string
    /** Applies the new value; resolves an error message, or `null` once it is applied. */
    oncommit: (value: string) => Promise<string | null>
  } = $props()

  // The draft follows the property until the user types; a new value resets it.
  let draft = $derived(value ?? '')
  let error = $derived.by((): string | null => {
    void value
    return null
  })
  let saving = $state(false)

  async function commit() {
    const next = changedValue(value, draft)
    if (next === undefined || saving) return
    saving = true
    error = await oncommit(next)
    saving = false
  }

  function keydown(event: KeyboardEvent) {
    if (event.key === 'Escape' && (draft !== (value ?? '') || error)) {
      event.preventDefault()
      event.stopPropagation()
      draft = value ?? ''
      error = null
    } else if (event.key === 'Enter' && (!multiline || event.ctrlKey || event.metaKey)) {
      event.preventDefault()
      void commit()
    }
  }
</script>

<div class="field">
  <label for={id}>{label}</label>
  {#if multiline}
    <textarea
      {id}
      class:mono
      rows="3"
      bind:value={draft}
      {placeholder}
      readonly={saving}
      aria-invalid={!!error}
      aria-describedby={error ? `${id}-error` : undefined}
      title={`${label}: Ctrl+Enter or leave the field to apply, Esc to revert`}
      onkeydown={keydown}
      onblur={() => void commit()}
    ></textarea>
  {:else}
    <input
      {id}
      class:mono
      type="text"
      bind:value={draft}
      {placeholder}
      readonly={saving}
      spellcheck="false"
      aria-invalid={!!error}
      aria-describedby={error ? `${id}-error` : undefined}
      title={`${label}: Enter or leave the field to apply, Esc to revert`}
      onkeydown={keydown}
      onblur={() => void commit()}
    />
  {/if}
  {#if error}<p class="error" id={`${id}-error`} role="alert">{error}</p>{/if}
</div>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  label {
    color: var(--color-text-muted);
    font-size: 12px;
  }

  input,
  textarea {
    width: 100%;
    box-sizing: border-box;
    padding: 4px 6px;
    border: 1px solid var(--color-border-strong);
    border-bottom-color: var(--color-border-accessible);
    border-radius: 2px;
    background: var(--color-pane);
    color: var(--color-text);
    font: inherit;
    resize: vertical;
  }

  input:focus,
  textarea:focus {
    border-color: var(--color-brand);
    outline: 2px solid var(--color-brand);
  }

  [aria-invalid='true'] {
    border-color: var(--color-danger);
  }

  .mono {
    font-family: var(--font-mono);
    font-size: 12px;
  }

  .error {
    margin: 0;
    color: var(--color-danger);
    font-size: 12px;
  }
</style>
