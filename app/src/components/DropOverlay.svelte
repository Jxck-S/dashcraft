<script lang="ts">
  let { active }: { active: boolean } = $props()
</script>

{#if active}
  <div class="drop-overlay" aria-hidden="true">
    <div class="drop-card">
      <strong>Drop to open</strong>
      <span>PBIX, PBIT, PBIP, BIM, TMDL file or a PBIP / TMDL folder</span>
    </div>
  </div>
{/if}

<style>
  .drop-overlay {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: grid;
    place-items: center;
    background: rgb(242 200 17 / 0.12);
    border: 3px dashed var(--color-accent-dark);
    pointer-events: none;
  }

  .drop-card {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 20px 28px;
    background: var(--color-pane);
    border: 1px solid var(--color-border-strong);
    border-radius: 4px;
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.12);
    text-align: center;
  }

  .drop-card strong {
    font-size: 16px;
  }

  .drop-card span {
    color: var(--color-text-muted);
  }
</style>
