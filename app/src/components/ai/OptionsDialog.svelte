<script lang="ts">
  import Icon from '../Icon.svelte'
  import Modal from '../Modal.svelte'
  import AiOptionsPage from './AiOptionsPage.svelte'
  import { NOT_AVAILABLE } from '../../lib/shell'
  import type { AiStatusOut } from '../../lib/ops'

  // File → Options and settings → Options, as in Power BI Desktop: a dialog with the pages in a
  // left column (GLOBAL, CURRENT FILE). DashCraft has one page, AI; Desktop's other pages are
  // listed disabled. AI settings apply as soon as they are changed (each change is one registry
  // command), so the dialog has a single Close button.
  let { status, demo, onstatus, onclose }: { status: AiStatusOut | null; demo: boolean; onstatus: (s: AiStatusOut) => void; onclose: () => void } = $props()

  const GLOBAL = ['Data Load', 'Power Query Editor', 'DirectQuery', 'Security', 'Privacy', 'Regional settings', 'Diagnostics', 'Preview features', 'Auto recovery']
  const CURRENT = ['Data Load', 'Regional settings', 'Privacy', 'Auto recovery', 'Report settings']
</script>

<Modal title="Options" width={940} {onclose}>
  <div class="options">
    <nav class="pages" aria-label="Options pages">
      <h3>Global</h3>
      {#each GLOBAL as page (page)}
        <button class="page-item" disabled title={NOT_AVAILABLE}>{page}</button>
      {/each}
      <button class="page-item active" aria-current="page" title="AI features (DashCraft): providers, keys, consents">
        <Icon name="ai" size={14} />
        AI
      </button>
      <h3>Current file</h3>
      {#each CURRENT as page (page)}
        <button class="page-item" disabled title={NOT_AVAILABLE}>{page}</button>
      {/each}
    </nav>
    <div class="page">
      <AiOptionsPage {status} {demo} {onstatus} />
    </div>
  </div>
  {#snippet footer()}
    <button class="primary" onclick={onclose}>Close</button>
  {/snippet}
</Modal>

<style>
  .options {
    display: flex;
    height: min(620px, calc(100vh - 220px));
    margin: 0 -16px -12px;
    border-top: 1px solid var(--color-border);
  }

  .pages {
    display: flex;
    flex: none;
    flex-direction: column;
    width: 190px;
    padding: 8px 0;
    overflow: auto;
    border-right: 1px solid var(--color-border);
    background: var(--color-chrome);
  }

  h3 {
    margin: 10px 14px 4px;
    color: var(--color-text-muted);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  .page-item {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 14px 4px 22px;
    border: none;
    background: transparent;
    text-align: left;
    cursor: pointer;
  }

  .page-item:disabled {
    color: var(--color-text-disabled);
    cursor: default;
  }

  .page-item.active {
    background: var(--color-pane);
    box-shadow: inset 3px 0 0 var(--color-brand);
    font-weight: 600;
  }

  .page {
    flex: 1;
    min-width: 0;
    padding: 12px 20px 20px;
    overflow: auto;
  }
</style>
