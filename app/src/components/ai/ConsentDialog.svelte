<script lang="ts">
  import { onMount } from 'svelte'
  import Icon from '../Icon.svelte'
  import Modal from '../Modal.svelte'
  import { CAPABILITY_LABELS, EXPOSURE_LABELS, KIND_LABELS, placeOf, type ConsentNeeded } from '../../lib/ai'
  import { ops, type AiPreviewOut, type AiPreviewParams } from '../../lib/ops'

  // The consent dialog: shown when an AI request has no recorded consent. It previews the exact
  // request (`ai.preview`: endpoint, capability, kind of data and the JSON body, keys redacted)
  // and records the consent (`ai.consent.grant`) only when the person clicks "Allow and send".
  // Cancel, Esc and the backdrop decline: nothing is sent. While something else still blocks the
  // request (a missing API key), nothing can be allowed: the button only ever allows a request
  // that is then sent.
  let { need, preview, ondone }: { need: ConsentNeeded; preview: AiPreviewParams; ondone: (granted: boolean) => void } = $props()

  let loaded = $state.raw<AiPreviewOut | null>(null)
  let error = $state<string | null>(null)
  let granting = $state(false)

  const consent = $derived(loaded?.consent ?? null)
  const local = $derived(consent?.local ?? need.local)
  const host = $derived(consent?.endpointHost ?? need.endpointHost)
  const body = $derived(loaded ? JSON.stringify(loaded.request?.body ?? loaded.messages ?? null, null, 2) : '')
  // The missing consent is what this dialog is for; anything else still blocks the request.
  const otherBlockers = $derived((loaded?.blockers ?? []).filter((b) => !b.startsWith('consent required')))

  onMount(() => {
    void (async () => {
      const result = await ops.aiPreview(preview)
      if (result.ok) loaded = result.value
      else error = result.error
    })()
  })

  async function allow() {
    granting = true
    error = null
    const result = await ops.aiConsentGrant(need.grant)
    granting = false
    if (result.ok) ondone(true)
    else error = result.error
  }
</script>

<Modal title="Allow this AI request?" width={760} onclose={() => ondone(false)}>
  <p class="lead">
    DashCraft asks before anything is sent to an AI provider. This is <strong>exactly</strong> what “Allow and send” sends now; your choice is remembered for this provider, endpoint, feature and kind of data until you revoke it in Options → AI.
  </p>

  <dl class="facts">
    <dt>Provider</dt>
    <dd>
      <strong>{need.provider}</strong>
      {#if loaded?.provider}<span class="muted"> · {KIND_LABELS[loaded.provider.kind]}{loaded.provider.model ? ` · ${loaded.provider.model}` : ''}</span>{/if}
    </dd>
    <dt>Endpoint</dt>
    <dd>
      <code>{host}</code>
      <span class="badge" class:local class:remote={!local}>{local ? 'local' : 'remote'}</span>
      <span class="muted">{placeOf(local)}</span>
    </dd>
    <dt>Feature</dt>
    <dd>{CAPABILITY_LABELS[need.capability]} <span class="muted">({need.capability})</span></dd>
    <dt>Data sent</dt>
    <dd>
      <strong>{need.exposure}</strong>: {loaded?.exposureDescription ?? EXPOSURE_LABELS[need.exposure]}
    </dd>
    {#if loaded}
      <dt>Size</dt>
      <dd>{loaded.inputChars.toLocaleString()} characters of prompt{loaded.request ? `, ${loaded.request.bodyBytes.toLocaleString()} bytes of JSON` : ''} <span class="muted">(prompt {loaded.promptVersion})</span></dd>
    {/if}
  </dl>

  {#if error && !loaded}
    <p class="error" role="alert"><Icon name="error" size={14} /> {error}</p>
  {:else if !loaded}
    <p class="muted"><span class="spinner" aria-hidden="true"></span> Building the preview…</p>
  {:else}
    {#if otherBlockers.length > 0}
      <div class="warn" role="note">
        <Icon name="warning" size={14} />
        <div>
          <strong>It cannot be sent yet, so there is nothing to allow.</strong> Fix this in Options → AI, then try again:
          <ul>
            {#each otherBlockers as b (b)}<li>{b}</li>{/each}
          </ul>
        </div>
      </div>
    {/if}
    {#if loaded.request}
      <h3>Request</h3>
      <pre class="block request">{loaded.request.method} {loaded.request.url}
{#each loaded.request.headers as h (h.name)}{h.name}: {h.value}
{/each}</pre>
      <p class="muted small">API keys are never shown: they appear as <code>[redacted]</code>. Apart from transport headers (host, length, compression), nothing else is sent.</p>
    {/if}
    <h3>Body (JSON)</h3>
    <pre class="block body" aria-label="Request body">{body}</pre>
    {#if error}
      <p class="error" role="alert"><Icon name="error" size={14} /> {error}</p>
    {/if}
  {/if}

  {#snippet footer()}
    <button onclick={() => ondone(false)}>Cancel</button>
    <button
      class="primary"
      onclick={() => void allow()}
      disabled={!loaded || granting || otherBlockers.length > 0}
      title={otherBlockers.length > 0 ? 'The request cannot be sent yet (see above)' : 'Record this consent and send the request'}
    >
      {granting ? 'Allowing…' : 'Allow and send'}
    </button>
  {/snippet}
</Modal>

<style>
  .lead {
    margin: 0 0 10px;
  }

  .facts {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px 14px;
    margin: 0 0 10px;
  }

  dt {
    color: var(--color-text-muted);
  }

  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .badge {
    display: inline-block;
    margin: 0 4px;
    padding: 0 6px;
    border-radius: 8px;
    font-size: 11px;
    font-weight: 600;
    line-height: 16px;
  }

  .badge.local {
    background: var(--color-brand-soft);
    color: var(--color-brand);
  }

  .badge.remote {
    background: var(--color-warning-bar);
    color: var(--color-text);
    box-shadow: inset 0 0 0 1px var(--color-warning-border);
  }

  h3 {
    margin: 12px 0 4px;
    font-size: 13px;
  }

  .block {
    margin: 0;
    padding: 6px 8px;
    overflow: auto;
    border: 1px solid var(--color-border);
    border-radius: 3px;
    background: var(--color-code-bg);
    white-space: pre;
  }

  .request {
    max-height: 120px;
  }

  .body {
    max-height: 320px;
  }

  .small {
    margin: 4px 0 0;
    font-size: 12px;
  }

  .warn {
    display: flex;
    gap: 6px;
    align-items: flex-start;
    padding: 6px 8px;
    border: 1px solid var(--color-warning-border);
    border-radius: 3px;
    background: var(--color-warning-bar);
  }

  .warn ul {
    margin: 4px 0 0;
    padding-left: 18px;
    overflow-wrap: anywhere;
  }

  .error {
    color: var(--color-danger);
  }

  .spinner {
    display: inline-block;
    width: 12px;
    height: 12px;
    margin-right: 4px;
    border: 2px solid var(--color-border-strong);
    border-top-color: var(--color-brand);
    border-radius: 50%;
    vertical-align: -2px;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
