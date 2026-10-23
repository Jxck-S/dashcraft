<script lang="ts">
  import { onMount, untrack } from 'svelte'
  import Icon from '../Icon.svelte'
  import { CAPABILITY_LABELS, EXPOSURE_LABELS, KIND_LABELS, loadStatus, when } from '../../lib/ai'
  import { hasBackend, ops, type AiStatusOut, type OpsResult, type ProviderKind, type ProviderSettings, type ProviderStatus, type StructuredOutput } from '../../lib/ops'

  // Options → AI: the opt-in AI layer's settings (pbi-ai, docs/AI.md). Every change is one
  // registry command and applies at once: `ai.settings.set` (on/off, providers, default
  // provider, sample rows), `ai.apiKey.set` / `ai.apiKey.clear` (the OS keychain; keys are
  // write-only and never shown again), `ai.consent.revoke`. Consents are granted only from the
  // consent dialog, which shows the exact request first.
  let { status, demo, onstatus }: { status: AiStatusOut | null; demo: boolean; onstatus: (s: AiStatusOut) => void } = $props()

  interface Draft {
    /** The id of the provider being edited; `null` for a new one. */
    original: string | null
    id: string
    idTouched: boolean
    kind: ProviderKind
    label: string
    endpoint: string
    model: string
    deployment: string
    apiVersion: string
    structuredOutput: StructuredOutput | ''
    allowHttp: boolean
  }

  const ENDPOINT_HINT: Record<ProviderKind, string> = {
    openAiCompatible: 'https://api.openai.com/v1 (default)',
    azureOpenAi: 'https://<resource>.openai.azure.com (required)',
    ollama: 'http://127.0.0.1:11434 (default)',
    mock: 'none: answers locally',
  }
  const DEFAULT_IDS: Record<ProviderKind, string> = { openAiCompatible: 'openai', azureOpenAi: 'azure', ollama: 'ollama', mock: 'demo' }
  const ID = /^[a-z0-9][a-z0-9_-]{0,63}$/

  let current = $state.raw<AiStatusOut | null>(untrack(() => status))
  let busy = $state(false)
  let message = $state<{ kind: 'error' | 'info'; text: string } | null>(null)
  let draft = $state<Draft | null>(null)
  let keyFor = $state<string | null>(null)
  let keyValue = $state('')
  let removing = $state<{ id: string; clearKey: boolean } | null>(null)
  let sampleDraft = $state<number>(untrack(() => status?.settings.sampleRows ?? 0))

  const settings = $derived(current?.settings ?? null)
  const providers = $derived(settings?.providers ?? [])
  const enabled = $derived(current?.enabled ?? false)
  const kinds = $derived<ProviderKind[]>(demo || draft?.kind === 'mock' ? ['openAiCompatible', 'azureOpenAi', 'ollama', 'mock'] : ['openAiCompatible', 'azureOpenAi', 'ollama'])
  const draftErrors = $derived(draft ? validate(draft) : [])
  const keychain = keychainName()

  onMount(() => void reload())

  function keychainName(): string {
    const platform = typeof navigator === 'undefined' ? '' : navigator.userAgent
    if (/Mac/i.test(platform)) return 'Keychain'
    if (/Windows/i.test(platform)) return 'Credential Manager'
    return 'the OS keychain'
  }

  async function reload() {
    const s = await loadStatus()
    if (!s) return
    current = s
    sampleDraft = s.settings.sampleRows
    onstatus(s)
  }

  /** Runs one command; reloads the status afterwards. */
  async function act<T>(run: () => Promise<OpsResult<T>>, note?: (value: T) => string | null): Promise<boolean> {
    busy = true
    message = null
    const result = await run()
    busy = false
    if (!result.ok) {
      message = { kind: 'error', text: result.error }
      return false
    }
    const text = note?.(result.value) ?? null
    if (text) message = { kind: 'info', text }
    await reload()
    return true
  }

  /** Runs a change from a checkbox or radio button; puts the control back if it fails. */
  async function toggle(input: HTMLInputElement, run: () => Promise<OpsResult<unknown>>) {
    const before = !input.checked
    if (!(await act(run))) input.checked = before
  }

  function statusOf(id: string): ProviderStatus | undefined {
    return current?.providers.find((p) => p.id === id)
  }

  function usesKey(kind: ProviderKind): boolean {
    return kind === 'openAiCompatible' || kind === 'azureOpenAi'
  }

  function keyLabel(p: ProviderStatus | undefined): string {
    switch (p?.apiKey) {
      case 'keychain':
        return `Key stored in ${keychain}`
      case 'environment':
        return 'Key from DASHCRAFT_AI_API_KEY'
      case 'missing':
        return p.local ? 'No key (not needed for a server on this computer)' : 'No key stored'
      case 'notRequired':
        return 'No key needed'
      default:
        return 'Saved after the provider is saved'
    }
  }

  // ---------------------------------------------------------------------------------------------
  // Providers
  // ---------------------------------------------------------------------------------------------

  function freeId(base: string, except: string | null = null): string {
    const taken = new Set(providers.map((p) => p.id).filter((id) => id !== except))
    if (!taken.has(base)) return base
    for (let n = 2; ; n += 1) if (!taken.has(`${base}-${n}`)) return `${base}-${n}`
  }

  function newProvider() {
    const kind: ProviderKind = 'ollama'
    draft = { original: null, id: freeId(DEFAULT_IDS[kind]), idTouched: false, kind, label: '', endpoint: '', model: '', deployment: '', apiVersion: '', structuredOutput: '', allowHttp: false }
    keyFor = null
  }

  function editProvider(p: ProviderSettings) {
    draft = {
      original: p.id,
      id: p.id,
      idTouched: true,
      kind: p.kind,
      label: p.label ?? '',
      endpoint: p.endpoint ?? '',
      model: p.model ?? '',
      deployment: p.deployment ?? '',
      apiVersion: p.apiVersion ?? '',
      structuredOutput: p.structuredOutput ?? '',
      allowHttp: p.allowHttp ?? false,
    }
    keyFor = null
  }

  function setKind(kind: ProviderKind) {
    if (!draft) return
    draft.kind = kind
    if (draft.original === null && !draft.idTouched) draft.id = freeId(DEFAULT_IDS[kind])
  }

  function validate(d: Draft): string[] {
    const errors: string[] = []
    if (!ID.test(d.id)) errors.push('Id: 1–64 lowercase letters, digits, “-” or “_”, starting with a letter or digit.')
    else if (providers.some((p) => p.id === d.id && p.id !== d.original)) errors.push(`A provider “${d.id}” already exists.`)
    const endpoint = d.endpoint.trim()
    if (endpoint && !/^https?:\/\/\S+$/i.test(endpoint)) errors.push('Endpoint: a URL starting with https:// (or http:// for this computer).')
    if (d.kind === 'azureOpenAi') {
      if (!endpoint) errors.push('Azure OpenAI needs the resource endpoint (https://<resource>.openai.azure.com).')
      if (!d.deployment.trim()) errors.push('Azure OpenAI needs a deployment name.')
    } else if (d.kind !== 'mock' && !d.model.trim()) errors.push('Enter the model name (for example gpt-4.1-mini or qwen2.5-coder:7b).')
    return errors
  }

  function toSettings(d: Draft): ProviderSettings {
    const original = providers.find((p) => p.id === d.original)
    const azure = d.kind === 'azureOpenAi'
    const mock = d.kind === 'mock'
    const opt = (v: string) => (v.trim() === '' ? undefined : v.trim())
    const p: ProviderSettings = {
      id: d.id,
      kind: d.kind,
      label: opt(d.label),
      endpoint: mock ? undefined : opt(d.endpoint),
      // Azure requests name the deployment; a model kept from the file is preserved.
      model: azure ? (original?.kind === d.kind ? (original.model ?? undefined) : undefined) : mock ? undefined : opt(d.model),
      deployment: azure ? opt(d.deployment) : undefined,
      apiVersion: azure ? opt(d.apiVersion) : undefined,
      structuredOutput: mock || d.structuredOutput === '' ? undefined : d.structuredOutput,
      allowHttp: !mock && d.allowHttp ? true : undefined,
    }
    return Object.fromEntries(Object.entries(p).filter(([, v]) => v !== undefined)) as unknown as ProviderSettings
  }

  async function saveDraft() {
    if (!draft || draftErrors.length > 0) return
    const next = toSettings(draft)
    const list = draft.original === null ? [...providers, next] : providers.map((p) => (p.id === draft?.original ? next : p))
    const ok = await act(
      () => ops.aiSettingsSet({ providers: list }),
      (out) => (out.consentsDropped > 0 ? `Saved. ${out.consentsDropped} consent(s) were dropped because the endpoint changed: you will be asked again.` : 'Provider saved.'),
    )
    if (ok) {
      const added = draft.original === null && usesKey(next.kind)
      draft = null
      if (added) keyFor = next.id
    }
  }

  async function remove(id: string, clearKey: boolean) {
    removing = null
    const ok = await act(
      () => ops.aiSettingsSet({ providers: providers.filter((p) => p.id !== id) }),
      (out) => `Removed ${id}${out.consentsDropped > 0 ? ` and its ${out.consentsDropped} consent(s)` : ''}.`,
    )
    if (ok && clearKey) await act(() => ops.aiApiKeyClear({ provider: id }))
  }

  // ---------------------------------------------------------------------------------------------
  // Keys (write-only)
  // ---------------------------------------------------------------------------------------------

  async function saveKey(provider: string) {
    const key = keyValue
    keyValue = ''
    if (!key.trim()) return
    const ok = await act(
      () => ops.aiApiKeySet({ provider, key }),
      (out) => `Key stored in ${keychain} (service “${out.service}”, account “${out.account}”). It is never shown again or written to the settings file.`,
    )
    if (ok) keyFor = null
  }

  async function clearKey(provider: string) {
    await act(
      () => ops.aiApiKeyClear({ provider }),
      (out) => (out.changed ? `Key of ${provider} removed from ${keychain}.` : `There was no key of ${provider} in ${keychain}.`),
    )
  }
</script>

<div class="ai-page" aria-busy={busy}>
  <h2><Icon name="ai" size={20} /> AI</h2>

  <div class="privacy" role="note">
    <Icon name="lock" size={16} />
    <div>
      <strong>Off by default. Nothing leaves this computer unless you allow it.</strong>
      <p>
        DashCraft works offline. With AI off it makes no network requests. With AI on, a request goes only to the provider you add here, only for a feature you allowed, and only with the kind of
        data you allowed: <em>model metadata</em> (table, column and measure names, types, descriptions, relationships and DAX), never whole tables. Before the first request to a provider DashCraft
        shows exactly what would be sent and asks you. There is no background activity, no telemetry and no DashCraft server: the provider you choose receives the request under its own terms. A local
        provider (Ollama, LM Studio) keeps everything on this computer.
      </p>
    </div>
  </div>

  {#if demo}
    <div class="demo" role="note">
      <Icon name="info" size={14} />
      <span><strong>Demo mode</strong> (<code>DASHCRAFT_AI_MOCK=1</code>, debug build): requests are previewed as configured, but answered with canned text. Nothing is sent.</span>
    </div>
  {/if}
  {#if !hasBackend}
    <p class="muted">AI settings need the desktop app.</p>
  {/if}
  {#if current?.settingsError}
    <p class="error" role="alert"><Icon name="error" size={14} /> The settings file could not be read, so AI stays off: {current.settingsError}</p>
  {/if}
  {#if message}
    <p class={message.kind} role={message.kind === 'error' ? 'alert' : 'status'}>
      <Icon name={message.kind === 'error' ? 'error' : 'check'} size={14} />
      {message.text}
    </p>
  {/if}

  <label class="switch">
    <input
      type="checkbox"
      checked={enabled}
      disabled={busy || !current}
      onchange={(e) => {
        const box = e.currentTarget as HTMLInputElement
        void toggle(box, () => ops.aiSettingsSet({ enabled: box.checked }))
      }}
    />
    <span>
      <strong>Turn on AI features</strong>
      <span class="muted">Explain with AI (measures) and Suggest a measure. Turning this off stops every request; your providers and consents are kept.</span>
    </span>
  </label>

  <section>
    <h3>Providers</h3>
    <p class="muted small">OpenAI-compatible servers (OpenAI, GitHub Models, LM Studio, vLLM, llama.cpp), Azure OpenAI, or Ollama on this computer. The default provider is used unless a feature asks for another.</p>
    {#if providers.length === 0}
      <p class="empty">No provider yet.</p>
    {:else}
      <table class="grid">
        <thead>
          <tr><th title="Default provider">Default</th><th>Provider</th><th>Endpoint</th><th>Model</th><th>API key</th><th></th></tr>
        </thead>
        <tbody>
          {#each providers as p (p.id)}
            {@const st = statusOf(p.id)}
            <tr>
              <td class="center">
                <input
                  type="radio"
                  name="ai-default"
                  checked={st?.isDefault ?? false}
                  disabled={busy}
                  aria-label={`Use ${p.id} by default`}
                  onchange={(e) => void toggle(e.currentTarget as HTMLInputElement, () => ops.aiSettingsSet({ defaultProvider: p.id }))}
                />
              </td>
              <td>
                <strong>{p.label ?? p.id}</strong>
                <span class="muted small block">{p.id} · {KIND_LABELS[p.kind]}</span>
              </td>
              <td>
                <code class="wrap">{st?.endpoint ?? p.endpoint ?? '—'}</code>
                {#if st}<span class="badge" class:local={st.local} class:remote={!st.local}>{st.local ? 'local' : 'remote'}</span>{/if}
              </td>
              <td>{st?.model ?? p.model ?? p.deployment ?? '—'}</td>
              <td class="small">
                {usesKey(p.kind) ? keyLabel(st) : 'No key needed'}
              </td>
              <td class="row-actions">
                <button class="small-btn" disabled={busy} onclick={() => editProvider(p)}>Edit</button>
                {#if usesKey(p.kind)}
                  <button class="small-btn" disabled={busy} onclick={() => ((keyFor = p.id), (keyValue = ''), (draft = null))}>{st?.apiKey === 'keychain' ? 'Replace key…' : 'Set key…'}</button>
                  {#if st?.apiKey === 'keychain'}<button class="small-btn" disabled={busy} onclick={() => void clearKey(p.id)}>Clear key</button>{/if}
                {/if}
                <button class="small-btn" disabled={busy} onclick={() => (removing = { id: p.id, clearKey: st?.apiKey === 'keychain' })}>Remove</button>
              </td>
            </tr>
            {#if removing?.id === p.id}
              <tr class="inline-form">
                <td colspan="6">
                  <span>Remove <strong>{p.id}</strong>? Its consents are dropped.</span>
                  {#if st?.apiKey === 'keychain'}
                    <label class="check"><input type="checkbox" bind:checked={removing.clearKey} /> Also remove its key from {keychain}</label>
                  {/if}
                  <button class="small-btn danger" disabled={busy} onclick={() => removing && void remove(p.id, removing.clearKey)}>Remove</button>
                  <button class="small-btn" onclick={() => (removing = null)}>Keep</button>
                </td>
              </tr>
            {/if}
            {#if keyFor === p.id}
              <tr class="inline-form">
                <td colspan="6">
                  <form class="key-form" onsubmit={(e) => (e.preventDefault(), void saveKey(p.id))}>
                    <label>
                      <span>API key for <strong>{p.id}</strong></span>
                      <input type="password" bind:value={keyValue} autocomplete="off" spellcheck="false" placeholder="Paste the key" aria-label={`API key for ${p.id}`} />
                    </label>
                    <button class="small-btn primary" type="submit" disabled={busy || keyValue.trim() === ''}>Save key</button>
                    <button class="small-btn" type="button" onclick={() => ((keyFor = null), (keyValue = ''))}>Cancel</button>
                  </form>
                  <p class="muted small">Stored only in {keychain} (service <code>dashcraft</code>, account <code>ai:{p.id}</code>), shared with <code>dashcraft ai login</code>. It is write-only: DashCraft never shows it again and never writes it to the settings file.</p>
                </td>
              </tr>
            {/if}
          {/each}
        </tbody>
      </table>
    {/if}

    {#if draft}
      {@const d = draft}
      <form class="editor" onsubmit={(e) => (e.preventDefault(), void saveDraft())}>
        <h4>{d.original === null ? 'Add a provider' : `Edit ${d.original}`}</h4>
        <div class="fields">
          <label>
            <span>Kind</span>
            <select value={d.kind} onchange={(e) => setKind((e.currentTarget as HTMLSelectElement).value as ProviderKind)}>
              {#each kinds as k (k)}<option value={k}>{KIND_LABELS[k]}</option>{/each}
            </select>
          </label>
          <label>
            <span>Id</span>
            <input bind:value={d.id} oninput={() => (d.idTouched = true)} readonly={d.original !== null} title={d.original !== null ? 'The id names the consents and the keychain entry: remove and add the provider to change it' : 'Lowercase letters, digits, - and _'} />
          </label>
          <label>
            <span>Display name (optional)</span>
            <input bind:value={d.label} placeholder={d.id} />
          </label>
          {#if d.kind !== 'mock'}
            <label class="wide">
              <span>Endpoint</span>
              <input bind:value={d.endpoint} placeholder={ENDPOINT_HINT[d.kind]} spellcheck="false" />
            </label>
          {/if}
          {#if d.kind === 'azureOpenAi'}
            <label>
              <span>Deployment</span>
              <input bind:value={d.deployment} placeholder="gpt-4o" spellcheck="false" />
            </label>
            <label>
              <span>API version</span>
              <input bind:value={d.apiVersion} placeholder="2024-10-21 (default)" spellcheck="false" />
            </label>
          {:else if d.kind !== 'mock'}
            <label>
              <span>Model</span>
              <input bind:value={d.model} placeholder={d.kind === 'ollama' ? 'qwen2.5-coder:7b' : 'gpt-4.1-mini'} spellcheck="false" />
            </label>
          {/if}
        </div>
        {#if d.kind !== 'mock'}
          <details class="advanced">
            <summary>Advanced</summary>
            <label>
              <span>JSON output</span>
              <select bind:value={d.structuredOutput}>
                <option value="">JSON schema (default)</option>
                <option value="jsonSchema">JSON schema</option>
                <option value="jsonObject">JSON object (servers without schema support)</option>
                <option value="none">Prompt only</option>
              </select>
            </label>
            <label class="check">
              <input type="checkbox" bind:checked={d.allowHttp} />
              Allow plain <code>http</code> to another computer (requests and keys travel unencrypted)
            </label>
          </details>
        {/if}
        {#if d.original !== null && (statusOf(d.original)?.consented.length ?? 0) > 0}
          <p class="muted small">Changing the endpoint drops this provider's consents; you will be asked again before the next request.</p>
        {/if}
        {#if draftErrors.length > 0}
          <ul class="draft-errors">
            {#each draftErrors as e (e)}<li>{e}</li>{/each}
          </ul>
        {/if}
        <div class="editor-actions">
          <button class="small-btn primary" type="submit" disabled={busy || draftErrors.length > 0}>Save provider</button>
          <button class="small-btn" type="button" onclick={() => (draft = null)}>Cancel</button>
        </div>
      </form>
    {:else}
      <button class="small-btn add" disabled={busy || !current} onclick={newProvider}><Icon name="add" size={14} /> Add provider</button>
    {/if}
  </section>

  <section>
    <h3>Sample rows</h3>
    <div class="sample">
      <input type="number" min="0" max="100" step="1" bind:value={sampleDraft} disabled={busy || !current} aria-label="Sample rows" />
      <button
        class="small-btn"
        disabled={busy || !current || sampleDraft === settings?.sampleRows || !Number.isInteger(sampleDraft) || sampleDraft < 0 || sampleDraft > 100}
        title="0–100 rows"
        onclick={() => void act(() => ops.aiSettingsSet({ sampleRows: sampleDraft }), () => 'Sample rows saved.')}
      >
        Apply
      </button>
    </div>
    <p class="muted small">
      At most this many rows of one table may be added to a “Suggest a measure” request when you tick “Include sample rows” (0 = never; at most 100). Row values need their own “sample” consent. Whole
      tables are never sent.
    </p>
  </section>

  <section>
    <h3>Consents</h3>
    <p class="muted small">What you allowed, per provider endpoint, feature and kind of data. Granted only from the “Allow this AI request?” dialog; revoke to be asked again.</p>
    {#if (current?.consents.length ?? 0) === 0}
      <p class="empty">No consent recorded: nothing has been allowed.</p>
    {:else}
      <table class="grid">
        <thead>
          <tr><th>Provider</th><th>Endpoint</th><th>Feature</th><th>Data</th><th>Granted</th><th></th></tr>
        </thead>
        <tbody>
          {#each current?.consents ?? [] as c (`${c.provider}|${c.endpointHost}|${c.capability}`)}
            <tr>
              <td>{c.provider}</td>
              <td><code class="wrap">{c.endpointHost}</code> <span class="badge" class:local={c.local} class:remote={!c.local}>{c.local ? 'local' : 'remote'}</span></td>
              <td>{CAPABILITY_LABELS[c.capability]}</td>
              <td title={EXPOSURE_LABELS[c.exposure]}>{c.exposure}</td>
              <td class="small">{when(c.grantedAt)}{c.grantedBy ? ` · ${c.grantedBy}` : ''}</td>
              <td class="row-actions">
                <button class="small-btn" disabled={busy} onclick={() => void act(() => ops.aiConsentRevoke({ provider: c.provider, capability: c.capability }), () => 'Consent revoked.')}>Revoke</button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>

  {#if current?.settingsPath}
    <p class="muted small path">Settings file: <code>{current.settingsPath}</code> (never contains keys)</p>
  {/if}
</div>

<style>
  .ai-page {
    max-width: 760px;
  }

  h2 {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0 0 10px;
    font-size: 20px;
    font-weight: 400;
  }

  h3 {
    margin: 18px 0 4px;
    font-size: 14px;
  }

  h4 {
    margin: 0 0 8px;
    font-size: 13px;
  }

  section {
    margin-top: 6px;
  }

  .privacy {
    display: flex;
    gap: 10px;
    padding: 10px 12px;
    border: 1px solid var(--color-border);
    border-radius: 4px;
    background: var(--color-info-bg);
  }

  .privacy p {
    margin: 4px 0 0;
    line-height: 1.45;
  }

  .demo {
    display: flex;
    gap: 6px;
    align-items: flex-start;
    margin: 8px 0 0;
    padding: 6px 10px;
    border: 1px solid var(--color-warning-border);
    border-radius: 4px;
    background: var(--color-warning-bar);
  }

  .switch {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    margin: 14px 0 4px;
  }

  .switch > span {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .switch input {
    margin-top: 3px;
  }

  .small {
    font-size: 12px;
  }

  .block {
    display: block;
  }

  .empty {
    color: var(--color-text-subtle);
    font-style: italic;
  }

  .error,
  .info {
    display: flex;
    gap: 6px;
    align-items: flex-start;
    margin: 8px 0;
    overflow-wrap: anywhere;
  }

  .error {
    color: var(--color-danger);
  }

  .info {
    color: var(--color-brand);
  }

  .grid {
    width: 100%;
    margin: 6px 0;
    border-collapse: collapse;
  }

  .grid th {
    padding: 4px 6px;
    border-bottom: 1px solid var(--color-border-strong);
    color: var(--color-text-muted);
    font-size: 12px;
    font-weight: 600;
    text-align: left;
  }

  .grid td {
    padding: 5px 6px;
    border-bottom: 1px solid var(--color-border);
    vertical-align: top;
  }

  .center {
    text-align: center;
  }

  .wrap {
    overflow-wrap: anywhere;
  }

  .row-actions {
    white-space: nowrap;
    text-align: right;
  }

  .inline-form td {
    background: var(--color-pane-strip);
  }

  .inline-form span,
  .inline-form .check {
    margin-right: 8px;
  }

  .key-form {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: flex-end;
  }

  .key-form label {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .key-form input {
    width: 320px;
  }

  input:not([type='checkbox']):not([type='radio']),
  select {
    padding: 3px 6px;
    border: 1px solid var(--color-border-strong);
    border-radius: 3px;
    background: var(--color-pane);
  }

  input[readonly] {
    background: var(--color-chrome);
    color: var(--color-text-muted);
  }

  .badge {
    display: inline-block;
    margin-left: 4px;
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
    box-shadow: inset 0 0 0 1px var(--color-warning-border);
  }

  .small-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 9px;
    border: 1px solid var(--color-border-strong);
    border-radius: 3px;
    background: var(--color-pane);
    font-size: 12px;
    cursor: pointer;
  }

  .small-btn:hover:not(:disabled) {
    background: var(--color-hover);
  }

  .small-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .small-btn.primary {
    border-color: var(--color-brand);
    background: var(--color-brand);
    color: #fff;
  }

  .small-btn.danger {
    border-color: var(--color-danger);
    color: var(--color-danger);
  }

  .add {
    margin-top: 4px;
  }

  .editor {
    margin: 8px 0;
    padding: 10px 12px;
    border: 1px solid var(--color-border-strong);
    border-radius: 4px;
    background: var(--color-pane-strip);
  }

  .fields {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 8px 12px;
  }

  .fields label,
  .advanced label:not(.check) {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .fields label > span,
  .advanced label > span {
    color: var(--color-text-muted);
    font-size: 12px;
  }

  .fields .wide {
    grid-column: 1 / -1;
  }

  .advanced {
    margin-top: 8px;
  }

  .advanced summary {
    color: var(--color-text-muted);
    cursor: pointer;
  }

  .advanced > label {
    margin: 6px 0;
  }

  .check {
    display: inline-flex;
    gap: 6px;
    align-items: center;
  }

  .draft-errors {
    margin: 8px 0 0;
    padding-left: 18px;
    color: var(--color-danger);
    font-size: 12px;
  }

  .editor-actions {
    display: flex;
    gap: 8px;
    margin-top: 10px;
  }

  .sample {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .sample input {
    width: 80px;
  }

  .path {
    margin-top: 18px;
    overflow-wrap: anywhere;
  }
</style>
