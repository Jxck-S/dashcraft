<script module lang="ts">
  export type BackstageSection = 'home' | 'open' | 'export' | 'options' | 'about'
</script>

<script lang="ts">
  import Icon, { type IconName } from '../Icon.svelte'
  import type { BlobMapDto, RecentFileDto } from '../../lib/bindings'
  import { baseName } from '../../lib/format'
  import { NOT_AVAILABLE } from '../../lib/shell'

  // File menu (backstage), as in Desktop: a left command column and a content page. Commands
  // that write the opened files (Save, Publish, …) are disabled in the viewer; Export writes
  // new files (PBIP project, PBIT template) through the registry's `convert.*` commands.
  let {
    section,
    recent,
    blobMap,
    version,
    hasDoc,
    hasBackend,
    onsection,
    onback,
    onopen,
    onopenpath,
    onremoverecent,
    onchooseblobmap,
    onclearblobmap,
    onclosedoc,
    canExport = false,
    onexport,
    onoptions,
    aiEnabled = false,
  }: {
    section: BackstageSection
    recent: RecentFileDto[]
    blobMap: BlobMapDto | null
    version: string
    hasDoc: boolean
    hasBackend: boolean
    onsection: (s: BackstageSection) => void
    onback: () => void
    onopen: (kind: 'file' | 'folder') => void
    onopenpath: (path: string) => void
    onremoverecent: (path: string | null) => void
    onchooseblobmap: () => void
    onclearblobmap: () => void
    onclosedoc: () => void
    /** The open document can be written as PBIP / PBIT (it has a local model). */
    canExport?: boolean
    onexport?: (kind: 'pbip' | 'pbit') => void
    /** Options and settings → Options (the AI page). */
    onoptions?: () => void
    /** The opt-in AI features are on (shown on the Options entry). */
    aiEnabled?: boolean
  } = $props()

  type Nav = { id: string; label: string; icon: IconName; section?: BackstageSection; enabled: boolean; onclick?: () => void }

  const nav = $derived<Array<Nav | null>>([
    { id: 'home', label: 'Home', icon: 'home', section: 'home', enabled: true },
    { id: 'new', label: 'New', icon: 'document', enabled: false },
    { id: 'open', label: 'Open', icon: 'open', section: 'open', enabled: true },
    { id: 'save', label: 'Save', icon: 'save', enabled: false },
    { id: 'saveas', label: 'Save as', icon: 'save', enabled: false },
    null,
    { id: 'getdata', label: 'Get data', icon: 'getData', enabled: false },
    { id: 'import', label: 'Import', icon: 'importTheme', enabled: false },
    { id: 'export', label: 'Export', icon: 'export', section: 'export', enabled: true },
    { id: 'publish', label: 'Publish', icon: 'publish', enabled: false },
    null,
    { id: 'options', label: 'Options and settings', icon: 'settings', section: 'options', enabled: true },
    { id: 'about', label: 'Get help', icon: 'help', section: 'about', enabled: true },
    { id: 'close', label: 'Close', icon: 'exit', enabled: hasDoc, onclick: onclosedoc },
  ])

  function when(ms: number): string {
    if (!ms) return ''
    const d = new Date(ms)
    return Number.isNaN(d.getTime()) ? '' : d.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })
  }

  function keydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.stopPropagation()
      onback()
    }
  }

  function dirName(path: string): string {
    const i = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'))
    return i > 0 ? path.slice(0, i) : path
  }
</script>

{#snippet recentList(limit: number)}
  {#if recent.length === 0}
    <p class="muted">Files and folders you open appear here.</p>
  {:else}
    <ul class="recent">
      {#each recent.slice(0, limit) as r (r.path)}
        <li class:missing={!r.exists}>
          <button class="recent-item" disabled={!r.exists || !hasBackend} onclick={() => onopenpath(r.path)} title={r.exists ? r.path : `${r.path}\n(not found)`}>
            <Icon name={/\.[a-z]+$/i.test(r.fileName) ? 'document' : 'folder'} size={20} />
            <span class="recent-text">
              <span class="name">{r.fileName}</span>
              <span class="path">{dirName(r.path)}</span>
            </span>
            <span class="date">{r.exists ? when(r.openedAtMs) : 'Not found'}</span>
          </button>
          <button class="remove" onclick={() => onremoverecent(r.path)} title="Remove from the list" aria-label={`Remove ${r.fileName} from the list`}>
            <Icon name="close" size={12} />
          </button>
        </li>
      {/each}
    </ul>
  {/if}
{/snippet}

<div class="backstage" role="dialog" aria-modal="true" aria-label="File" tabindex="-1" onkeydown={keydown}>
  <nav class="side">
    <button class="back" onclick={onback} title="Back (Esc)" aria-label="Back"><Icon name="back" size={20} /></button>
    {#each nav as n, i (n?.id ?? `sep-${i}`)}
      {#if n === null}
        <hr />
      {:else}
        <button
          class="nav-item"
          class:active={n.section === section}
          disabled={!n.enabled}
          title={n.enabled ? n.label : NOT_AVAILABLE}
          onclick={() => (n.section ? onsection(n.section) : n.onclick?.())}
        >
          <Icon name={n.icon} size={20} />
          <span>{n.label}</span>
        </button>
      {/if}
    {/each}
  </nav>

  <main class="page">
    {#if section === 'home'}
      <h1>Welcome to DashCraft</h1>
      <p class="lead muted">An offline viewer for Power BI files: PBIX, PBIT, PBIP, BIM and TMDL. Nothing leaves this computer.</p>
      <div class="tiles">
        <button class="tile" onclick={() => onopen('file')} disabled={!hasBackend} title="Open a Power BI file (Ctrl+O)">
          <Icon name="open" size={28} />
          <span>Open file</span>
        </button>
        <button class="tile" onclick={() => onopen('folder')} disabled={!hasBackend} title="Open a PBIP or TMDL folder (Ctrl+Shift+O)">
          <Icon name="folder" size={28} />
          <span>Open folder</span>
        </button>
        <button class="tile" disabled title={NOT_AVAILABLE}>
          <Icon name="document" size={28} />
          <span>Blank report</span>
        </button>
      </div>
      <h2>Recent</h2>
      {@render recentList(8)}
    {:else if section === 'open'}
      <h1>Open</h1>
      <div class="open-actions">
        <button class="action" onclick={() => onopen('file')} disabled={!hasBackend}>
          <Icon name="open" size={24} />
          <span><strong>Browse this device</strong><span class="muted">PBIX, PBIT, PBIP, BIM, TMDL, ABF, VPAX… (Ctrl+O)</span></span>
        </button>
        <button class="action" onclick={() => onopen('folder')} disabled={!hasBackend}>
          <Icon name="folder" size={24} />
          <span><strong>Open a folder</strong><span class="muted">A PBIP project or TMDL folder (Ctrl+Shift+O)</span></span>
        </button>
      </div>
      <h2>
        Recent
        {#if recent.length > 0}<button class="link" onclick={() => onremoverecent(null)}>Clear list</button>{/if}
      </h2>
      {@render recentList(20)}
    {:else if section === 'export'}
      <h1>Export</h1>
      <p class="muted">Writes new files; the opened file is never changed. The <code>dashcraft</code> command-line tool runs the same commands (<code>convert.pbip</code>, <code>convert.pbit</code>) and also exports data.</p>
      <div class="open-actions">
        <button class="action" disabled={!hasBackend || !canExport} onclick={() => onexport?.('pbip')} title={canExport ? 'Choose an empty folder for the project' : 'Open a document with a local model first'}>
          <Icon name="folder" size={24} />
          <span><strong>Power BI project (PBIP)</strong><span class="muted">TMDL model and PBIR reports, into an empty folder</span></span>
        </button>
        <button class="action" disabled={!hasBackend || !canExport} onclick={() => onexport?.('pbit')} title={canExport ? 'Save a template of the current report' : 'Open a document with a local model first'}>
          <Icon name="document" size={24} />
          <span><strong>Power BI template (PBIT)</strong><span class="muted">The model schema and the current report, without data</span></span>
        </button>
        <button class="action" disabled title={NOT_AVAILABLE}>
          <Icon name="document" size={24} />
          <span><strong>Export to PDF</strong><span class="muted">{NOT_AVAILABLE}</span></span>
        </button>
      </div>
    {:else if section === 'options'}
      <h1>Options and settings</h1>
      <div class="open-actions">
        <button class="action" onclick={onoptions} disabled={!hasBackend || !onoptions}>
          <Icon name="settings" size={24} />
          <span>
            <strong>Options</strong>
            <span class="muted">AI features (opt-in, {aiEnabled ? 'on' : 'off'}): providers, API keys in the OS keychain, consents. Other Desktop options are not available.</span>
          </span>
        </button>
        <button class="action" onclick={onchooseblobmap} disabled={!hasBackend}>
          <Icon name="blob" size={24} />
          <span>
            <strong>Data source settings — Blob map…</strong>
            <span class="muted">
              {blobMap?.path
                ? `Azure blob URLs resolve to local files through ${baseName(blobMap.path)}${blobMap.fromEnvironment ? ` (${blobMap.environmentVariable ?? 'DASHCRAFT_BLOB_MAP'})` : ''}.`
                : 'Choose a blob-map.json: AzureStorage.Blobs URLs → local CSV files (no network).'}
            </span>
          </span>
        </button>
        {#if blobMap?.path}
          <button class="action" onclick={onclearblobmap}>
            <Icon name="close" size={24} />
            <span><strong>Stop using the blob map</strong><span class="muted">Blob sources fail offline and offer “Load CSV manually…”.</span></span>
          </button>
        {/if}
      </div>
    {:else}
      <h1>About DashCraft</h1>
      <p><strong>DashCraft {version}</strong> — opens Power BI files offline. Read-only; no telemetry, no network.</p>
      <h2>Notice</h2>
      <p class="muted small">DashCraft is an independent project. It is not affiliated with, endorsed or sponsored by Microsoft. “Power BI” and “Microsoft” are trademarks of Microsoft Corporation, used only to describe which files DashCraft opens. DashCraft is not affiliated with storytold/ArtCraft or with other products named “DashCraft”. The full notice is the <code>NOTICE</code> file installed with the app (<code>licenses/NOTICE</code>).</p>
      <h2>Third-party assets</h2>
      <ul class="notices">
        <li><strong>Fluent UI System Icons</strong> — MIT License, © Microsoft Corporation (<code>src/assets/icons/LICENSE</code>).</li>
        <li><strong>Selawik</strong> font — SIL Open Font License 1.1, © Microsoft Corporation (<code>src/assets/fonts/OFL.txt</code>).</li>
      </ul>
      <p class="muted small">Every bundled component and its license: <code>licenses/THIRD_PARTY_NOTICES.md</code>.</p>
    {/if}
  </main>
</div>

<style>
  .backstage {
    position: fixed;
    top: var(--titlebar-height);
    right: 0;
    bottom: 0;
    left: 0;
    z-index: 200;
    display: flex;
    background: var(--color-pane);
  }

  .backstage:focus {
    outline: none;
  }

  .side {
    display: flex;
    flex: none;
    flex-direction: column;
    gap: 2px;
    width: 220px;
    padding: 8px 8px 16px;
    overflow: auto;
    background: var(--color-chrome);
    border-right: 1px solid var(--color-border);
  }

  .back {
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    margin-bottom: 8px;
    border: none;
    border-radius: 50%;
    background: transparent;
    cursor: pointer;
  }

  .back:hover {
    background: var(--color-hover);
  }

  .nav-item {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 38px;
    padding: 0 12px;
    border: none;
    border-radius: 4px;
    background: transparent;
    font-size: 14px;
    text-align: left;
    cursor: pointer;
  }

  .nav-item:hover:not(:disabled) {
    background: var(--color-hover);
  }

  .nav-item.active {
    background: var(--color-pane);
    box-shadow: inset 3px 0 0 var(--color-brand);
    font-weight: 600;
  }

  .nav-item:disabled {
    color: var(--color-text-disabled);
    cursor: default;
  }

  hr {
    width: 100%;
    margin: 6px 0;
    border: none;
    border-top: 1px solid var(--color-border);
  }

  .page {
    flex: 1;
    min-width: 0;
    padding: 28px 40px;
    overflow: auto;
  }

  h1 {
    margin: 0 0 8px;
    font-size: 28px;
    font-weight: 300;
  }

  h2 {
    display: flex;
    align-items: baseline;
    gap: 16px;
    margin: 28px 0 8px;
    font-size: 16px;
    font-weight: 600;
  }

  .lead {
    margin: 0 0 20px;
    font-size: 14px;
  }

  .tiles {
    display: flex;
    gap: 12px;
  }

  .tile {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    width: 150px;
    height: 104px;
    border: 1px solid var(--color-border);
    border-radius: 6px;
    background: var(--color-pane);
    box-shadow: 0 1px 2px rgb(0 0 0 / 0.08);
    cursor: pointer;
  }

  .tile:hover:not(:disabled) {
    border-color: var(--color-brand);
  }

  .tile:disabled {
    color: var(--color-text-disabled);
    cursor: default;
  }

  .open-actions {
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-width: 640px;
    margin-top: 12px;
  }

  .action {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 10px 14px;
    border: none;
    border-radius: 4px;
    background: transparent;
    text-align: left;
    cursor: pointer;
  }

  .action > span {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .action:hover:not(:disabled) {
    background: var(--color-hover);
  }

  .action:disabled {
    color: var(--color-text-disabled);
    cursor: default;
  }

  .recent {
    max-width: 860px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .recent li {
    display: flex;
    align-items: center;
    border-bottom: 1px solid var(--color-border);
  }

  .recent-item {
    display: flex;
    flex: 1;
    align-items: center;
    gap: 12px;
    min-width: 0;
    padding: 8px 10px;
    border: none;
    background: transparent;
    text-align: left;
    cursor: pointer;
  }

  .recent-item:hover:not(:disabled) {
    background: var(--color-hover);
  }

  .recent-item:disabled {
    cursor: default;
  }

  .missing .recent-item {
    color: var(--color-text-disabled);
  }

  .recent-text {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }

  .name {
    font-weight: 600;
  }

  .path {
    overflow: hidden;
    color: var(--color-text-muted);
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .date {
    flex: none;
    color: var(--color-text-muted);
    font-size: 12px;
  }

  .remove {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--color-text-muted);
    cursor: pointer;
  }

  .remove:hover {
    background: var(--color-hover);
  }

  .link {
    padding: 0;
    border: none;
    background: transparent;
    color: var(--color-brand);
    font-size: 12px;
    font-weight: 400;
    cursor: pointer;
  }

  .notices {
    padding-left: 18px;
    line-height: 1.8;
  }

  .small {
    font-size: 12px;
  }
</style>
