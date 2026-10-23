<script module lang="ts">
  export type { IconName } from '../lib/icons'
</script>

<script lang="ts">
  // Fluent UI System Icons (MIT), inlined as SVG: no icon font or external asset, so the CSP
  // stays strict. The glyph is painted with `currentColor`.
  import { glyph, type IconName } from '../lib/icons'

  let { name, size = 16, title }: { name: IconName; size?: number; title?: string } = $props()

  const g = $derived(glyph(name, size))
</script>

<svg
  class="icon"
  width={size}
  height={size}
  viewBox={g?.viewBox ?? '0 0 20 20'}
  fill="currentColor"
  role={title ? 'img' : undefined}
  aria-hidden={title ? undefined : 'true'}
  aria-label={title}
>
  {#if title}<title>{title}</title>{/if}
  <!-- Static, vendored SVG markup (src/assets/icons), never user data. -->
  <!-- eslint-disable-next-line svelte/no-at-html-tags -->
  {@html g?.body ?? ''}
</svg>

<style>
  .icon {
    flex: none;
    display: inline-block;
    vertical-align: middle;
  }
</style>
