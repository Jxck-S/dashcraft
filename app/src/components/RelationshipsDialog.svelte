<script lang="ts">
  import Icon from './Icon.svelte'
  import Modal from './Modal.svelte'
  import type { RelationshipDto } from '../lib/bindings'
  import { CARDINALITY, CROSS_FILTER, daxColumnRef } from '../lib/model'
  import { NOT_AVAILABLE } from '../lib/shell'

  // Modeling / Home → Manage relationships (read-only): every relationship with its columns,
  // cardinality, cross-filter direction and state. Selecting one shows it in the Properties pane.
  let {
    relationships,
    selected,
    onselect,
    onclose,
  }: { relationships: RelationshipDto[]; selected: number | null; onselect: (id: number) => void; onclose: () => void } = $props()
</script>

<Modal title="Manage relationships" width={860} {onclose}>
  {#if relationships.length === 0}
    <p class="muted">The model has no relationships.</p>
  {:else}
    <table>
      <thead>
        <tr><th>Active</th><th>From: table (column)</th><th>To: table (column)</th><th>Cardinality</th><th>Cross-filter direction</th></tr>
      </thead>
      <tbody>
        {#each relationships as r (r.id)}
          <tr class:selected={selected === r.id} class:inactive={!r.active}>
            <td><span class="box" class:checked={r.active} aria-label={r.active ? 'Active' : 'Inactive'}>{#if r.active}<Icon name="check" size={10} />{/if}</span></td>
            <td><button class="link" onclick={() => onselect(r.id)} title="Show in the Properties pane"><code>{daxColumnRef(r.fromTable, r.fromColumn)}</code></button></td>
            <td><code>{daxColumnRef(r.toTable, r.toColumn)}</code></td>
            <td>{CARDINALITY[r.cardinality].long} ({CARDINALITY[r.cardinality].short})</td>
            <td>{CROSS_FILTER[r.crossFilter]}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
  {#snippet footer()}
    <button disabled title={NOT_AVAILABLE}>New relationship</button>
    <button disabled title={NOT_AVAILABLE}>Autodetect</button>
    <span class="spacer"></span>
    <button class="primary" onclick={onclose}>Close</button>
  {/snippet}
</Modal>

<style>
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }

  th,
  td {
    padding: 6px 8px;
    border-bottom: 1px solid var(--color-border);
    text-align: left;
  }

  th {
    background: var(--color-chrome);
    font-weight: 600;
  }

  tbody tr:hover {
    background: var(--color-hover);
  }

  tr.selected {
    background: var(--color-selected);
  }

  tr.inactive {
    color: var(--color-text-muted);
  }

  .box {
    display: inline-grid;
    place-items: center;
    width: 14px;
    height: 14px;
    border: 1px solid var(--color-text-muted);
    border-radius: 2px;
    color: #fff;
  }

  .box.checked {
    border-color: var(--color-text-muted);
    background: var(--color-text-muted);
  }

  .spacer {
    flex: 1;
  }

  button {
    height: 28px;
    padding: 0 14px;
    border: 1px solid var(--color-border-strong);
    border-radius: 4px;
    background: var(--color-pane);
    cursor: pointer;
  }

  button:disabled {
    color: var(--color-text-disabled);
    cursor: default;
  }

  button.link {
    height: auto;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--color-brand);
    text-align: left;
  }

  button.primary {
    border-color: var(--color-brand);
    background: var(--color-brand);
    color: #fff;
  }
</style>
