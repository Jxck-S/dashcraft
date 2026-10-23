<script lang="ts">
  import type { Snippet } from 'svelte'
  import Icon, { type IconName } from '../Icon.svelte'

  // A Desktop right-side pane: title + collapse chevron (»), or collapsed to a narrow strip
  // with « and the vertical title. Panes that can be hidden (Selection) also get ✕.
  let {
    title,
    icon,
    collapsed,
    ontogglecollapse,
    onclose,
    width = 252,
    header,
    children,
  }: {
    title: string
    icon?: IconName
    collapsed: boolean
    ontogglecollapse: () => void
    onclose?: () => void
    width?: number
    /** Extra controls under the title (tabs, search). */
    header?: Snippet
    children: Snippet
  } = $props()
</script>

{#if collapsed}
  <section class="strip" aria-label={`${title} (collapsed)`}>
    <button class="expand" onclick={ontogglecollapse} title={`Expand the ${title} pane`} aria-label={`Expand the ${title} pane`}>
      <Icon name="chevronDoubleLeft" size={16} />
    </button>
    <button class="vertical" onclick={ontogglecollapse} title={`Expand the ${title} pane`}>
      {#if icon}<span class="strip-icon"><Icon name={icon} size={16} /></span>{/if}
      <span class="vertical-text">{title}</span>
    </button>
  </section>
{:else}
  <section class="pane" style:width={`${width}px`} aria-label={title}>
    <header>
      <h2>{title}</h2>
      <span class="actions">
        {#if onclose}
          <button onclick={onclose} title={`Close the ${title} pane`} aria-label={`Close the ${title} pane`}><Icon name="close" size={14} /></button>
        {/if}
        <button onclick={ontogglecollapse} title={`Collapse the ${title} pane`} aria-label={`Collapse the ${title} pane`}>
          <Icon name="chevronDoubleRight" size={16} />
        </button>
      </span>
    </header>
    {#if header}<div class="sub">{@render header()}</div>{/if}
    <div class="content">{@render children()}</div>
  </section>
{/if}

<style>
  .pane {
    display: flex;
    flex: none;
    flex-direction: column;
    min-height: 0;
    border-left: 1px solid var(--color-border);
    background: var(--color-pane);
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 6px 4px 12px;
  }

  h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
  }

  .actions {
    display: flex;
    gap: 2px;
  }

  .actions button,
  .expand {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--color-text-muted);
    cursor: pointer;
  }

  .actions button:hover,
  .expand:hover,
  .vertical:hover {
    background: var(--color-hover);
    color: var(--color-text);
  }

  .sub {
    padding: 0 12px 6px;
  }

  .content {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }

  .strip {
    display: flex;
    flex: none;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    width: var(--pane-strip-width);
    padding-top: 8px;
    border-left: 1px solid var(--color-border);
    background: var(--color-pane-strip);
  }

  .vertical {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 6px 0;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--color-text);
    cursor: pointer;
  }

  .strip-icon {
    display: inline-flex;
    transform: rotate(90deg);
  }

  .vertical-text {
    font-size: 15px;
    font-weight: 600;
    writing-mode: vertical-rl;
  }
</style>
