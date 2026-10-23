<script module lang="ts">
  export interface Toast {
    id: number
    kind: 'error' | 'warning' | 'info'
    message: string
  }
</script>

<script lang="ts">
  import Icon from './Icon.svelte'

  let { toasts, ondismiss }: { toasts: Toast[]; ondismiss: (id: number) => void } = $props()
</script>

<div class="toasts" aria-live="assertive">
  {#each toasts as t (t.id)}
    <div class="toast {t.kind}" role={t.kind === 'error' ? 'alert' : 'status'}>
      <Icon name={t.kind} />
      <span class="text">{t.message}</span>
      <button onclick={() => ondismiss(t.id)} aria-label="Dismiss"><Icon name="close" size={12} /></button>
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    right: 16px;
    bottom: calc(var(--statusbar-height) + 12px);
    z-index: 200;
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: min(440px, calc(100vw - 32px));
    pointer-events: none;
  }

  .toast {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 10px 10px 10px 12px;
    border: 1px solid var(--color-border-strong);
    border-left-width: 4px;
    border-radius: 3px;
    background: var(--color-pane);
    box-shadow: 0 6px 20px rgb(0 0 0 / 0.14);
    pointer-events: auto;
  }

  .toast.error {
    border-left-color: var(--color-danger);
    color: var(--color-danger);
  }

  .toast.warning {
    border-left-color: var(--color-accent-dark);
  }

  .toast.info {
    border-left-color: var(--color-brand);
  }

  .text {
    flex: 1;
    color: var(--color-text);
    overflow-wrap: anywhere;
  }

  button {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--color-text-muted);
    cursor: pointer;
  }
</style>
