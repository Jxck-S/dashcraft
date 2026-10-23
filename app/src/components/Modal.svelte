<script lang="ts">
  import type { Snippet } from 'svelte'
  import Icon from './Icon.svelte'

  // Minimal modal dialog: backdrop, title bar, body and footer; Esc or the close button closes.
  let {
    title,
    width = 560,
    onclose,
    children,
    footer,
  }: {
    title: string
    width?: number
    onclose: () => void
    children: Snippet
    footer?: Snippet
  } = $props()

  let dialog = $state<HTMLDivElement | null>(null)

  $effect(() => {
    dialog?.focus()
  })

  function keydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.stopPropagation()
      onclose()
    }
  }
</script>

<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div
    class="dialog"
    bind:this={dialog}
    role="dialog"
    aria-modal="true"
    aria-label={title}
    tabindex="-1"
    style:width={`min(${width}px, calc(100vw - 48px))`}
    onkeydown={keydown}
  >
    <header>
      <h2>{title}</h2>
      <button class="close" onclick={onclose} title="Close (Esc)" aria-label="Close"><Icon name="close" size={14} /></button>
    </header>
    <div class="body">
      {@render children()}
    </div>
    {#if footer}
      <footer>{@render footer()}</footer>
    {/if}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 180;
    display: grid;
    place-items: center;
    background: rgb(0 0 0 / 0.25);
  }

  .dialog {
    display: flex;
    flex-direction: column;
    max-height: calc(100vh - 80px);
    border: 1px solid var(--color-border-strong);
    border-radius: 4px;
    background: var(--color-pane);
    box-shadow: 0 12px 36px rgb(0 0 0 / 0.2);
    outline: none;
  }

  header {
    display: flex;
    align-items: center;
    padding: 10px 12px 6px 16px;
  }

  h2 {
    flex: 1;
    margin: 0;
    font-size: 15px;
  }

  .close {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border: none;
    border-radius: 2px;
    background: transparent;
    color: var(--color-text-muted);
    cursor: pointer;
  }

  .close:hover {
    background: var(--color-hover);
  }

  .body {
    min-height: 0;
    overflow: auto;
    padding: 6px 16px 12px;
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 10px 16px 14px;
    border-top: 1px solid var(--color-border);
  }

  footer :global(button) {
    min-width: 80px;
    padding: 5px 14px;
    border: 1px solid var(--color-border-strong);
    border-radius: 2px;
    background: var(--color-pane);
    cursor: pointer;
  }

  footer :global(button.primary) {
    border-color: var(--color-brand);
    background: var(--color-brand);
    color: #fff;
  }

  footer :global(button:disabled) {
    opacity: 0.5;
    cursor: default;
  }
</style>
