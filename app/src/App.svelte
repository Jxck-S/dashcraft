<script lang="ts">
  import { onMount, untrack } from 'svelte'
  import type { ExplainTarget } from './components/ai/ExplainPane.svelte'
  import CsvDialog from './components/CsvDialog.svelte'
  import Modal from './components/Modal.svelte'
  import DiagnosticsPanel from './components/DiagnosticsPanel.svelte'
  import DropOverlay from './components/DropOverlay.svelte'
  import Icon from './components/Icon.svelte'
  import PowerQueryDialog from './components/PowerQueryDialog.svelte'
  import PropertiesPane from './components/PropertiesPane.svelte'
  import RefreshPanel from './components/RefreshPanel.svelte'
  import RelationshipsDialog from './components/RelationshipsDialog.svelte'
  import Toasts, { type Toast } from './components/Toasts.svelte'
  import ViewAsDialog from './components/ViewAsDialog.svelte'
  import DataPane, { fieldKey, type DataTarget } from './components/panes/DataPane.svelte'
  import FiltersPane from './components/panes/FiltersPane.svelte'
  import SelectionPane from './components/panes/SelectionPane.svelte'
  import VisualizationsPane from './components/panes/VisualizationsPane.svelte'
  import Backstage, { type BackstageSection } from './components/shell/Backstage.svelte'
  import FormulaBar from './components/shell/FormulaBar.svelte'
  import PageTabs from './components/shell/PageTabs.svelte'
  import Pane from './components/shell/Pane.svelte'
  import Ribbon from './components/shell/Ribbon.svelte'
  import StatusBar, { type DataStatus, type StatusZoom } from './components/shell/StatusBar.svelte'
  import TitleBar, { type SearchItem } from './components/shell/TitleBar.svelte'
  import ViewRail from './components/shell/ViewRail.svelte'
  import DaxQueryView, { newTab, type DaxTab } from './components/views/DaxQueryView.svelte'
  import ModelView from './components/views/ModelView.svelte'
  import ReportView from './components/views/ReportView.svelte'
  import TableView, { type TableInfo } from './components/views/TableView.svelte'
  import TmdlView from './components/views/TmdlView.svelte'
  import * as ai from './lib/ai'
  import * as backend from './lib/backend'
  import type {
    BlobMapDto,
    CanvasAction,
    CsvLoad,
    FilterSpec,
    MeasureDto,
    MeasureValue,
    ModelDiagramDto,
    ObjectDetailsDto,
    PageDto,
    RecentFileDto,
    RefreshSource,
    RenderedPage,
    SecurityRoles,
    TableDto,
    TableStatus,
    TmdlViewDto,
    ViewAsResult,
    ViewAsState,
  } from './lib/backend'
  import { applyProgress, applyReport, newRun, runProgress, stopRun, type RefreshRun } from './lib/data'
  import { baseName, formatTimestamp, plural } from './lib/format'
  import { daxColumnRef, daxTableRef, ownerTables, type LoadedDocument, type SelectedId } from './lib/model'
  import type { AiPreviewParams, AiStatusOut } from './lib/ops'
  import { buildRibbon, type RibbonSelection } from './lib/ribbon'
  import { tabButtons, type MenuItem, type ViewId, type ZoomMode } from './lib/shell'

  // ---------------------------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------------------------

  // DTOs are replaced wholesale, never mutated: no deep reactivity needed.
  let doc = $state.raw<LoadedDocument | null>(null)
  let activeView = $state<ViewId>('report')
  let selected = $state<SelectedId>(null)
  let lastTableId = $state<number | null>(null)
  let details = $state.raw<ObjectDetailsDto | null>(null)
  let detailsLoading = $state(false)
  let detailsError = $state<string | null>(null)
  let busy = $state(false)
  let busyLabel = $state('')
  let dragActive = $state(false)
  let diagnosticsOpen = $state(false)
  let version = $state('')
  let status = $state<{ message: string; isError: boolean }>({ message: 'Ready', isError: false })
  let toasts = $state<Toast[]>([])

  // Shell: ribbon, backstage (File), panes.
  let ribbonTab = $state('home')
  let ribbonCollapsed = $state(false)
  let backstage = $state<BackstageSection | null>(null)
  let recent = $state.raw<RecentFileDto[]>([])
  let panes = $state({ filters: true, selection: false, properties: false })
  let collapsed = $state({ filters: false, viz: false, data: false, selection: false, properties: false, explain: false })

  // Data session: table statuses, a version bumped on every data change, the refresh run.
  let statuses = $state.raw<TableStatus[]>([])
  let dataVersion = $state(0)
  let run = $state.raw<RefreshRun | null>(null)
  let refreshPanelOpen = $state(false)
  let blobMap = $state.raw<BlobMapDto | null>(null)
  let csvTarget = $state<{ table: string; reason: string | null } | null>(null)
  // The data shown comes from the PBIP local data cache (`.pbi/cache.abf`), loaded on open.
  let cacheNote = $state<{ asOf: string | null } | null>(null)
  // A template (PBIT) with parameters was refreshed on open with the values saved in it.
  let templateNote = $state(false)

  // Report view.
  let reportId = $state<number | null>(null)
  let pageId = $state<number | null>(null)
  let selectedVisualId = $state<number | null>(null)
  let zoomMode = $state<ZoomMode>('fitPage')
  let zoomPercent = $state(100)
  let effectiveZoom = $state(100)
  let canvasMode = $state<'wireframe' | 'outline'>('wireframe')
  let pageFilters = $state.raw<FilterSpec | null>(null)
  let measureValues = $state.raw<Map<number, string> | null>(null)
  // Interactive canvas: the last render of the current page (selections, effective filters,
  // Reset to default), a render made outside the canvas, the page navigation history (Back
  // buttons) and a web link waiting for the user (never opened automatically).
  let canvasInfo = $state.raw<RenderedPage | null>(null)
  let canvasIncoming = $state.raw<RenderedPage | null>(null)
  let canvasFilters = $state.raw<FilterSpec | null>(null)
  let navHistory = $state<number[]>([])
  let webLink = $state<string | null>(null)

  // Table view.
  let tableMode = $state<'grid' | 'schema'>('grid')
  let tableInfo = $state.raw<TableInfo | null>(null)
  let parsePeopleRequest = $state<string | null>(null)

  // Model view.
  let diagram = $state.raw<ModelDiagramDto | null>(null)
  let modelZoom = $state(100)
  let modelFitToken = $state(0)

  // DAX query view (tabs survive view switches).
  let daxTabs = $state<DaxTab[]>([])
  let daxActive = $state(1)
  let daxSeq = 0
  let daxView = $state<ReturnType<typeof DaxQueryView>>()

  // TMDL view.
  let tmdl = $state.raw<TmdlViewDto | null>(null)
  let tmdlError = $state<string | null>(null)
  let tmdlLoading = $state(false)
  let tmdlPath = $state<string | null>(null)

  // Dialogs.
  let powerQuery = $state<{ initial: number | null } | null>(null)
  let relationshipsOpen = $state(false)

  // Row-level security (Modeling → View as): the model's roles and the view in effect.
  let security = $state.raw<SecurityRoles | null>(null)
  let viewAsOpen = $state(false)
  let stoppingViewAs = $state(false)

  // Opt-in AI (pbi-ai; off by default): status, Options → AI, the Explain pane, Suggest a measure
  // and the consent dialog (the only place a consent is granted, after showing the request).
  let aiStatus = $state.raw<AiStatusOut | null>(null)
  let aiDemo = $state(false)
  let optionsOpen = $state(false)
  let explainTarget = $state.raw<ExplainTarget | null>(null)
  let explainSeq = 0
  let suggestOpen = $state(false)
  let consentRequest = $state.raw<{ need: ai.ConsentNeeded; preview: AiPreviewParams; resolve: (granted: boolean) => void } | null>(null)

  // Monotonic request counters: responses for superseded requests are dropped.
  let openSeq = 0
  let detailsSeq = 0
  let toastSeq = 0
  let filterSeq = 0

  // ---------------------------------------------------------------------------------------------
  // Derived
  // ---------------------------------------------------------------------------------------------

  const owners = $derived(doc ? ownerTables(doc.tree) : new Map<number, TableDto>())
  const tree = $derived(doc?.tree ?? null)
  const dataTable = $derived(doc ? (doc.tree.tables.find((t) => t.id === lastTableId) ?? doc.tree.tables[0] ?? null) : null)
  const modelStatus = $derived(doc?.summary.modelStatus ?? null)
  const localModel = $derived(modelStatus?.availability === 'local')
  const refreshing = $derived(run?.outcome === 'running')
  const canRefresh = $derived(backend.hasBackend && !busy && localModel && (doc?.summary.counts.tables ?? 0) > 0)
  const dataLoaded = $derived(statuses.some((s) => s.loaded))
  const hasMeasures = $derived((doc?.summary.counts.measures ?? 0) > 0 && localModel)
  const runCounts = $derived(run ? runProgress(run) : null)

  const reports = $derived(doc?.reports ?? [])
  const report = $derived(reports.find((r) => r.id === reportId) ?? reports[0] ?? null)
  const page = $derived.by((): PageDto | null => {
    if (!report) return null
    return report.pages.find((p) => p.id === pageId) ?? report.pages.find((p) => p.name === report.startPage) ?? report.pages[0] ?? null
  })
  const pageIndex = $derived(report && page ? report.pages.indexOf(page) : -1)
  const selectedVisual = $derived(page?.visuals.find((v) => v.id === selectedVisualId) ?? null)
  const usedFields = $derived.by(() => {
    if (activeView !== 'report') return null
    const set = new Set<string>()
    for (const f of selectedVisual?.fields ?? []) if (f.table && f.field) set.add(fieldKey(f.table, f.field))
    return set
  })

  /** The selected model object, for the contextual tabs and the formula bar. */
  const selection = $derived.by((): RibbonSelection | null => {
    if (selected == null) return null
    const table = owners.get(selected)
    if (!table) return null
    if (table.id === selected) return { kind: 'table', table }
    const measure = table.measures.find((m) => m.id === selected)
    if (measure) return { kind: 'measure', measure, table }
    const column = table.columns.find((c) => c.id === selected)
    return column ? { kind: 'column', column, table } : null
  })

  const formula = $derived.by(() => {
    if (activeView !== 'report' && activeView !== 'table') return null
    const s = selection
    if (s?.kind === 'measure') return { name: s.measure.name, expression: s.measure.expression, kind: 'Measure' }
    if (s?.kind === 'column' && s.column.kind === 'calculated' && s.column.expression != null) {
      return { name: s.column.name, expression: s.column.expression, kind: 'Calculated column' }
    }
    if (s?.kind === 'table' && s.table.expression != null) return { name: s.table.name, expression: s.table.expression, kind: 'Calculated table' }
    return null
  })

  const aiAvailability = $derived(ai.availability(aiStatus))

  /** "Explain with AI" for a measure: enabled once AI is on with a provider and the model is local. */
  function explainAction(measure: MeasureDto, table: TableDto): { enabled: boolean; title: string; run: () => void } {
    const enabled = aiAvailability.ready && localModel
    const title = !aiAvailability.ready ? (aiAvailability.reason ?? ai.AI_OFF_TITLE) : localModel ? `Explain ${measure.name} with AI` : 'Needs a document with a local model'
    return { enabled, title, run: () => explainMeasure(measure, table) }
  }

  const measureExplain = $derived(selection?.kind === 'measure' ? explainAction(selection.measure, selection.table) : null)

  /** "East, West as bob@contoso.com": who the data is viewed as. */
  function viewAsLabel(view: ViewAsState): string {
    const roles = view.roles.join(', ')
    if (view.userName) return roles ? `${roles} as ${view.userName}` : view.userName
    return roles || 'a user'
  }

  const viewing = $derived(security?.viewAs.active ? security.viewAs : null)

  const csvTables = $derived(statuses.filter((s) => s.canLoadCsv).map((s) => s.name))
  const tableStatus = $derived(dataTable ? (statuses.find((s) => s.name === dataTable.name) ?? null) : null)

  const ribbonTabs = $derived(
    buildRibbon({
      view: activeView,
      hasDoc: doc !== null,
      hasBackend: backend.hasBackend,
      localModel,
      canRefresh,
      refreshing,
      blobMap,
      csvTables,
      hasParameters: (doc?.tree.expressions ?? []).some((e) => e.isParameter),
      hasReport: report !== null && page !== null,
      theme: report?.theme ?? null,
      zoomMode,
      panes,
      canvasMode,
      canReset: canvasInfo !== null && !canvasInfo.isDefault,
      selection,
      table: activeView === 'table' ? dataTable : null,
      tableCanLoadCsv: tableStatus?.canLoadCsv ?? false,
      tableMode,
      column: tableInfo?.column
        ? { name: tableInfo.column.name, dataType: tableInfo.column.dataType, derived: tableInfo.column.kind === 'derived', formatString: tableInfo.column.formatString }
        : null,
      ai: aiAvailability,
      security: { roles: security ? security.roles.length : null, active: viewing !== null, label: viewing ? viewAsLabel(viewing) : '' },
      act: {
        refresh: () => void startRefresh(null),
        transformData: () => (powerQuery = { initial: null }),
        editParameters: () => (powerQuery = { initial: doc?.tree.expressions.find((e) => e.isParameter)?.id ?? null }),
        chooseBlobMap: () => void chooseBlobMap(),
        clearBlobMap: () => void clearBlobMap(),
        loadCsv: (table) => (csvTarget = { table, reason: reasonOf(table) }),
        manageRelationships: () => (relationshipsOpen = true),
        setZoomMode: (m) => (zoomMode = m),
        togglePane: (p) => {
          panes = { ...panes, [p]: !panes[p] }
          if (panes[p]) collapsed = { ...collapsed, [p]: false }
        },
        setCanvasMode: (m) => (canvasMode = m),
        resetToDefault: () => void resetCanvas(),
        daxComment: (add) => daxView?.comment(add),
        refreshTable: (name) => void startRefresh([name]),
        setTableMode: (m) => (tableMode = m),
        columnStats: (name) => {
          const id = dataTable?.columns.find((c) => c.name === name)?.id
          if (id != null) void select(id)
        },
        parsePeople: (name) => (parsePeopleRequest = name),
        about: () => (backstage = 'about'),
        suggestMeasure: () => (suggestOpen = true),
        explainMeasure: (m, t) => explainMeasure(m, t),
        aiOptions: () => openAiOptions(),
        viewAs: () => (viewAsOpen = true),
      },
    }),
  )

  // Keep the ribbon on an existing tab when the view or the selection changes.
  $effect(() => {
    const ids = ribbonTabs.map((t) => t.id)
    if (!ids.includes(untrack(() => ribbonTab))) ribbonTab = 'home'
  })

  const windowTitle = $derived(doc ? `${doc.summary.fileName} - DashCraft` : 'DashCraft')
  $effect(() => {
    void backend.setWindowTitle(windowTitle)
  })

  const searchItems = $derived.by((): SearchItem[] => {
    const items: SearchItem[] = [
      { kind: 'command', label: 'Open…', detail: 'File', icon: 'open', enabled: backend.hasBackend, run: () => void pickAndOpen('file') },
      { kind: 'command', label: 'Open folder…', detail: 'File', icon: 'folder', enabled: backend.hasBackend, run: () => void pickAndOpen('folder') },
      { kind: 'command', label: 'Report view', detail: 'Views', icon: 'report', enabled: true, run: () => setView('report') },
      { kind: 'command', label: 'Table view', detail: 'Views', icon: 'data', enabled: doc !== null, run: () => setView('table') },
      { kind: 'command', label: 'Model view', detail: 'Views', icon: 'model', enabled: doc !== null, run: () => setView('model') },
      { kind: 'command', label: 'DAX query view', detail: 'Views', icon: 'document', enabled: doc !== null, run: () => setView('dax') },
      { kind: 'command', label: 'TMDL view', detail: 'Views', icon: 'document', enabled: doc !== null, run: () => setView('tmdl') },
    ]
    const seen = new Set<string>()
    for (const tab of ribbonTabs) {
      for (const b of tabButtons(tab)) {
        if (seen.has(b.label)) continue
        seen.add(b.label)
        items.push({ kind: 'command', label: b.label, detail: tab.label, icon: b.icon, enabled: b.enabled && !!(b.onclick ?? b.menu), run: () => (b.onclick ? b.onclick() : (ribbonTab = tab.id)) })
      }
    }
    for (const t of doc?.tree.tables ?? []) {
      items.push({ kind: 'field', label: t.name, detail: 'Table', icon: 'table', enabled: true, run: () => void select(t.id) })
      for (const m of t.measures) items.push({ kind: 'field', label: m.name, detail: `Measure · ${t.name}`, icon: 'measure', enabled: true, run: () => void select(m.id) })
      for (const c of t.columns) {
        if (c.kind !== 'rowNumber') items.push({ kind: 'field', label: c.name, detail: `Column · ${t.name}`, icon: c.kind === 'calculated' ? 'calcColumn' : 'column', enabled: true, run: () => void select(c.id) })
      }
    }
    return items
  })

  const statusText = $derived.by(() => {
    if (!doc) return ''
    switch (activeView) {
      case 'report':
        return report && page ? `Page ${pageIndex + 1} of ${report.pages.length}` : ''
      case 'table': {
        if (!dataTable) return ''
        const rows = tableInfo ? `${tableInfo.totalRows.toLocaleString()} ${tableInfo.filtered ? 'matching rows' : 'rows'}` : ''
        const col = tableInfo?.column ? `   Column: ${tableInfo.column.name}${tableInfo.distinct != null ? ` (${tableInfo.distinct.toLocaleString()} distinct values)` : ''}` : ''
        return `Table: ${dataTable.name}${rows ? ` (${rows})` : ''}${col}`
      }
      case 'model':
        return `${plural(doc.summary.counts.tables, 'table')}, ${plural(doc.summary.counts.relationships, 'relationship')}`
      case 'dax': {
        const t = daxTabs.find((x) => x.id === daxActive)
        if (!t) return ''
        if (t.running) return `${t.name}: running…`
        if (t.error) return `${t.name}: error`
        if (t.cancelled) return `${t.name}: cancelled`
        const r = t.result?.tables[t.resultIndex] ?? t.result?.tables[0]
        return r && t.result ? `${t.name}: ${plural(r.totalRows, 'row')} in ${t.result.durationMs.toLocaleString()} ms` : t.name
      }
      case 'tmdl':
        return tmdlPath ?? ''
    }
  })

  const statusZoom = $derived.by((): StatusZoom | null => {
    if (activeView === 'report' && page && canvasMode === 'wireframe') {
      return {
        percent: effectiveZoom,
        onzoom: (p) => {
          zoomPercent = p
          zoomMode = 'custom'
        },
        onfit: () => (zoomMode = 'fitPage'),
        fitTitle: 'Fit to page',
      }
    }
    if (activeView === 'model' && localModel) {
      return { percent: modelZoom, onzoom: (p) => (modelZoom = p), onfit: () => (modelFitToken += 1), fitTitle: 'Fit to screen' }
    }
    return null
  })

  const dataStatus = $derived.by((): DataStatus | null => {
    if (!doc || !localModel) return null
    if (run && refreshing) {
      return { label: `Refreshing ${runCounts?.done ?? 0}/${runCounts?.total || '…'}`, title: 'Refresh in progress: show the details', tone: 'warn', onclick: () => (refreshPanelOpen = true) }
    }
    if (run && (run.outcome === 'failed' || run.outcome === 'error')) {
      const failed = run.entries.filter((e) => e.state === 'error' || e.state === 'partial').length
      return { label: `Refresh: ${plural(failed || 1, 'error')}`, title: 'The last refresh had errors: show them', tone: 'error', onclick: () => (refreshPanelOpen = true) }
    }
    if (cacheNote) return { label: 'Cached data', title: `Loaded from .pbi/cache.abf${cacheNote.asOf ? ` (as of ${cacheNote.asOf})` : ''}`, tone: 'warn' }
    if (dataLoaded) {
      const loaded = statuses.filter((s) => s.loaded)
      return { label: 'Data loaded', title: loaded.map((s) => `${s.name}: ${s.rows.toLocaleString()} rows`).join('\n'), tone: 'ok' }
    }
    return { label: 'No data loaded', title: 'Use Home → Refresh to load the data offline', tone: 'none' }
  })

  // ---------------------------------------------------------------------------------------------
  // Documents
  // ---------------------------------------------------------------------------------------------

  function toast(kind: Toast['kind'], message: string) {
    const id = ++toastSeq
    toasts = [...toasts, { id, kind, message }]
    setTimeout(() => dismiss(id), kind === 'error' ? 10_000 : 6_000)
  }

  function dismiss(id: number) {
    toasts = toasts.filter((t) => t.id !== id)
  }

  function fail(message: string) {
    status = { message, isError: true }
    toast('error', message)
  }

  function defaultQuery(): string {
    const t = doc?.tree.tables.find((x) => x.measures.length > 0) ?? null
    const m = t?.measures[0]
    const first = doc?.tree.tables[0]
    const body = m ? `ROW("${m.name.replaceAll('"', '""')}", [${m.name.replaceAll(']', ']]')}])` : first ? `ROW("Rows", COUNTROWS(${daxTableRef(first.name)}))` : '{ 1 }'
    return `// Write a DAX query and press Run (F5 or Ctrl+Enter). Right-click a table, column or measure\n// in the Data pane for quick queries. Queries are read-only.\nEVALUATE\n\t${body}\n`
  }

  function resetViews() {
    selected = null
    lastTableId = null
    details = null
    detailsError = null
    reportId = null
    pageId = null
    selectedVisualId = null
    pageFilters = null
    measureValues = null
    canvasInfo = null
    canvasIncoming = null
    canvasFilters = null
    navHistory = []
    webLink = null
    tableInfo = null
    diagram = null
    tmdl = null
    tmdlError = null
    tmdlPath = null
    daxSeq = 1
    daxTabs = [newTab(1, defaultQuery())]
    daxActive = 1
    powerQuery = null
    relationshipsOpen = false
    viewAsOpen = false
    explainTarget = null
    suggestOpen = false
  }

  /** Shows an opened document; the previously shown one is closed in the backend. */
  function showDocument(views: LoadedDocument) {
    const summary = views.summary
    const previous = doc?.summary.docId
    backend.setCurrent(views)
    doc = views
    if (previous && previous !== summary.docId) void backend.closeDocument(previous)
    resetViews()
    run = null
    refreshPanelOpen = false
    csvTarget = null
    cacheNote = null
    templateNote = false
    backstage = null
    statuses = []
    security = null
    dataVersion += 1
    void loadStatuses(summary.docId)
    void loadSecurity(summary.docId)
    void loadDiagram(summary.docId)
    void refreshRecent()
    const c = summary.counts
    status = {
      message: `Opened ${summary.fileName}: ${plural(c.tables, 'table')}, ${plural(c.measures, 'measure')}, ${plural(c.reports, 'report')}`,
      isError: false,
    }
    if (summary.diagnostics.errors > 0) {
      toast('warning', `${plural(summary.diagnostics.errors, 'error')} while reading ${summary.fileName}. See the diagnostics panel.`)
    }
    if (c.reports === 0 && activeView === 'report') activeView = 'model'
    // Load what Power BI Desktop shows on open: a PBIX's saved data and a PBIP's local data cache
    // (no M), or a template's refresh (a PBIT saves no data). A PBIP without a cache stays empty.
    if (backend.hasBackend && summary.onOpen !== 'none') {
      templateNote = summary.onOpen === 'query' && views.tree.expressions.some((e) => e.isParameter)
      void startRefresh(null, summary.onOpen === 'query' ? 'query' : 'cache')
    }
  }

  async function openPath(path: string) {
    const seq = ++openSeq
    busy = true
    busyLabel = `Opening ${baseName(path)}…`
    status = { message: busyLabel, isError: false }
    try {
      const opened = await backend.openDocument(path)
      if (seq !== openSeq) {
        // A newer open (or a close) superseded this one: drop its document.
        if (opened.ok) void backend.closeDocument(opened.value.summary.docId)
        return
      }
      if (!opened.ok) {
        fail(opened.error)
        return
      }
      showDocument(opened.value)
    } finally {
      if (seq === openSeq) busy = false
    }
  }

  async function closeDocument() {
    ++openSeq
    busy = false
    backstage = null
    await backend.closeDocument()
    doc = null
    resetViews()
    diagnosticsOpen = false
    statuses = []
    security = null
    run = null
    refreshPanelOpen = false
    csvTarget = null
    cacheNote = null
    templateNote = false
    status = { message: 'Ready', isError: false }
  }

  async function refreshRecent() {
    recent = await backend.recentFiles()
  }

  async function removeRecent(path: string | null) {
    recent = await backend.removeRecentFile(path)
  }

  async function loadStatuses(docId = doc?.summary.docId) {
    const result = await backend.tableStatus()
    if (docId !== doc?.summary.docId) return
    if (result.ok) statuses = result.value
  }

  /** The model's roles and the "View as" state (`security.roles`; no data needed). */
  async function loadSecurity(docId = doc?.summary.docId) {
    if (!docId || !backend.hasBackend || doc?.summary.modelStatus.availability !== 'local') return
    const result = await backend.securityRoles()
    if (docId === doc?.summary.docId) security = result.ok ? result.value : null
  }

  async function loadDiagram(docId: string) {
    const result = await backend.modelDiagram()
    if (docId === doc?.summary.docId && result.ok) diagram = result.value
  }

  async function loadTmdl() {
    if (!doc || tmdl || tmdlLoading) return
    const docId = doc.summary.docId
    tmdlLoading = true
    const result = await backend.tmdlView()
    tmdlLoading = false
    if (docId !== doc?.summary.docId) return
    if (result.ok) {
      tmdl = result.value
      tmdlError = null
      const table = owners.get(selected ?? -1)
      tmdlPath = (table && result.value.files.find((f) => f.table === table.name)?.path) ?? tmdlPath
    } else tmdlError = result.error
  }

  /** Table data changed (refresh, CSV load, derived column, View as): reload statuses and views. */
  async function dataChanged() {
    dataVersion += 1
    // A refresh re-applies an active view; it may now fail closed (`viewAs.error`).
    if (security?.viewAs.active) void loadSecurity()
    await loadStatuses()
  }

  // ---------------------------------------------------------------------------------------------
  // Row-level security: View as
  // ---------------------------------------------------------------------------------------------

  /** A view was applied or stopped: every view re-queries (canvas, grid, measures, Data pane). */
  function viewAsApplied(result: ViewAsResult) {
    viewAsOpen = false
    if (security) security = { ...security, viewAs: result.viewAs }
    const view = result.viewAs
    status = { message: view.active ? `Now viewing as ${viewAsLabel(view)}` : 'Stopped viewing as roles: all rows are shown again', isError: false }
    toast('info', status.message)
    void dataChanged()
  }

  async function stopViewAs() {
    if (stoppingViewAs) return
    stoppingViewAs = true
    const result = await backend.viewAs([], null)
    stoppingViewAs = false
    if (result.ok) viewAsApplied(result.value)
    else fail(result.error)
  }

  function reasonOf(table: string): string | null {
    const s = statuses.find((x) => x.name === table)?.lastRefresh?.status
    return s && s.state === 'error' ? (s.message ?? null) : null
  }

  // ---------------------------------------------------------------------------------------------
  // Refresh, blob map, CSV
  // ---------------------------------------------------------------------------------------------

  async function startRefresh(tables: string[] | null, source: RefreshSource = 'query') {
    if (!doc || refreshing) return
    const docId = doc.summary.docId
    const fromCache = source === 'cache'
    // `cache` without a PBIP cache is the data saved in a PBIX: Desktop shows it without a note.
    const pbipCache = fromCache && doc.summary.cache !== null
    const asOf = formatTimestamp(doc.summary.cache?.modified)
    run = newRun(docId, tables)
    // Loading saved data is quick and quiet: the panel opens only if something fails.
    refreshPanelOpen = !fromCache
    status = {
      message: pbipCache ? 'Loading cached data…' : fromCache ? 'Loading data…' : tables ? `Refreshing ${tables.join(', ')}…` : 'Refreshing all tables…',
      isError: false,
    }
    const result = await backend.refresh(tables, source, (event) => {
      if (run && run.outcome === 'running' && run.docId === docId) run = applyProgress(run, event)
    })
    if (!run || run.docId !== docId || doc?.summary.docId !== docId) return
    if (result.ok) {
      const report = result.value
      run = applyReport(run, report)
      const failed = report.tables.filter((t) => t.status.state !== 'ok' || t.columns.some((c) => c.status.state !== 'ok')).length
      const seconds = (report.durationMs / 1000).toFixed(1)
      if (pbipCache) cacheNote = { asOf }
      else if (report.ok && tables === null) cacheNote = null
      const done = pbipCache
        ? `Loaded cached data${asOf ? ` (as of ${asOf})` : ''}: ${plural(report.tables.length, 'table')} in ${seconds} s`
        : fromCache
          ? `Loaded ${plural(report.tables.length, 'table')} in ${seconds} s`
          : `Refreshed ${plural(report.tables.length, 'table')} in ${seconds} s`
      if (report.ok) {
        status = { message: done, isError: false }
        refreshPanelOpen = false
        toast('info', status.message)
      } else {
        status = {
          message: `${pbipCache ? 'Cached data loaded' : fromCache ? 'Data loaded' : 'Refresh finished'} in ${seconds} s: ${plural(failed, 'table')} with errors`,
          isError: true,
        }
        refreshPanelOpen = true
      }
    } else if (result.kind === 'cancelled') {
      run = stopRun(run, 'cancelled')
      status = { message: 'Refresh cancelled — the previous data is kept', isError: false }
    } else {
      run = stopRun(run, 'error', result.error)
      fail(result.error)
    }
    await dataChanged()
  }

  async function cancelRefresh() {
    if (!run || run.outcome !== 'running') return
    run = { ...run, cancelling: true }
    const result = await backend.cancelRefresh()
    if (!result.ok) fail(result.error)
  }

  async function chooseBlobMap() {
    try {
      const path = await backend.pickFile('Choose a blob map (blob-map.json)', [{ name: 'Blob map', extensions: ['json'] }])
      if (!path) return
      const result = await backend.setBlobMap(path)
      if (!result.ok) {
        fail(result.error)
        return
      }
      blobMap = result.value
      toast('info', `Blob map ${baseName(path)} will be used by the next refresh.`)
    } catch (e) {
      fail(`Open dialog failed: ${String(e)}`)
    }
  }

  async function clearBlobMap() {
    const result = await backend.setBlobMap(null)
    if (result.ok) {
      blobMap = result.value
      toast('info', 'Blob map cleared: blob sources are offline.')
    } else fail(result.error)
  }

  function csvLoaded(table: string, result: CsvLoad) {
    csvTarget = null
    if (run) run = applyReport(run, result.report, false)
    const warnings = result.report.diagnostics.filter((d) => d.severity !== 'info')
    const message = `Loaded ${plural(result.rows, 'row')} into ${table} from CSV`
    status = { message, isError: false }
    toast(warnings.length > 0 ? 'warning' : 'info', `${message}${warnings.length > 0 ? ` (${plural(warnings.length, 'warning')}: ${warnings[0].message})` : ''}.`)
    void dataChanged()
  }

  // ---------------------------------------------------------------------------------------------
  // Selection and navigation
  // ---------------------------------------------------------------------------------------------

  function setView(view: ViewId) {
    activeView = view
    backstage = null
    if (view === 'tmdl') void loadTmdl()
  }

  function showTable(name: string) {
    const table = doc?.tree.tables.find((t) => t.name === name)
    if (!table) return
    setView('table')
    void select(table.id)
  }

  async function select(id: number) {
    selected = id
    const owner = owners.get(id)
    if (owner) {
      lastTableId = owner.id
      if (activeView === 'tmdl' && tmdl) tmdlPath = tmdl.files.find((f) => f.table === owner.name)?.path ?? tmdlPath
    }
    const seq = ++detailsSeq
    detailsLoading = true
    detailsError = null
    const result = await backend.objectDetails(id)
    if (seq !== detailsSeq) return
    detailsLoading = false
    if (result.ok) {
      details = result.value
    } else {
      details = null
      detailsError = result.error
    }
  }

  function selectVisual(id: number | null) {
    selectedVisualId = id
    if (id != null) void select(id)
  }

  /** Shows a page of the current report; the page left is pushed on the Back history. */
  function selectPage(id: number, remember = true) {
    const current = page?.id ?? null
    if (remember && current != null && current !== id) navHistory = [...navHistory.slice(-49), current]
    pageId = id
    selectedVisualId = null
  }

  function selectReport(id: number) {
    reportId = id
    pageId = null
    selectedVisualId = null
    navHistory = []
  }

  // ---------------------------------------------------------------------------------------------
  // Interactive canvas
  // ---------------------------------------------------------------------------------------------

  // A new page starts without canvas state until its first render.
  $effect(() => {
    void page?.id
    void report?.id
    untrack(() => {
      canvasInfo = null
      canvasIncoming = null
      canvasFilters = null
    })
  })

  function canvasRendered(dto: RenderedPage) {
    if (dto.report !== report?.name || dto.page !== page?.name) return
    if (!canvasInfo && dto.warnings.length > 0) toast('warning', `Saved slicer selection not applied: ${dto.warnings[0]}`)
    canvasInfo = dto
    // Only a changed filter context re-evaluates the Measures panel (not every scroll).
    if (JSON.stringify(dto.effectiveFilters) !== JSON.stringify(canvasFilters)) canvasFilters = dto.effectiveFilters
  }

  function canvasAction(action: CanvasAction) {
    switch (action.kind) {
      case 'back': {
        const previous = navHistory.at(-1)
        if (previous == null || !report?.pages.some((p) => p.id === previous)) {
          toast('info', 'There is no previous page to go back to.')
          return
        }
        navHistory = navHistory.slice(0, -1)
        selectPage(previous, false)
        break
      }
      case 'navigate': {
        const target = report ? backend.pageIdOf(report.id, action.page) : null
        if (target != null) selectPage(target)
        else toast('info', `The page ${action.displayName} is not in this report.`)
        break
      }
      case 'webUrl':
        webLink = action.url
        break
      case 'notAvailable':
        toast('info', action.message)
        break
    }
  }

  async function resetCanvas() {
    if (!report || !page) return
    const result = await backend.resetPage(report.id, page.id)
    if (result.ok) {
      canvasIncoming = result.value
      status = { message: 'Reset to default: the report’s saved slicer selections', isError: false }
    } else if (result.kind !== 'cancelled') fail(result.error)
  }

  async function clearCanvasSelection(visual: string) {
    if (!report || !page) return
    const result = await backend.clearSelections(report.id, page.id, visual)
    if (result.ok) canvasIncoming = result.value
    else if (result.kind !== 'cancelled') fail(result.error)
  }

  function selectVisualByName(name: string) {
    const v = page?.visuals.find((x) => x.name === name)
    if (v) selectVisual(v.id)
  }

  async function copyLink(url: string) {
    try {
      await navigator.clipboard.writeText(url)
      toast('info', 'Link copied.')
    } catch {
      toast('warning', 'The link could not be copied; select it and copy it manually.')
    }
  }

  /** A field chip of the Visualizations pane: select the model object it refers to. */
  function selectField(table: string | null, field: string | null) {
    if (!table || !field) return
    const t = doc?.tree.tables.find((x) => x.name.toLowerCase() === table.toLowerCase())
    const item = t?.measures.find((m) => m.name.toLowerCase() === field.toLowerCase()) ?? t?.columns.find((c) => c.name.toLowerCase() === field.toLowerCase())
    if (item) void select(item.id)
  }

  // The page's filter state (report + page filters) for the canvas (renderer slot).
  $effect(() => {
    const r = report?.id ?? null
    const p = page?.id ?? null
    const mine = ++filterSeq
    pageFilters = null
    if (r == null || !localModel || !backend.hasBackend) return
    void backend.reportFilters(r, p).then((result) => {
      if (mine === filterSeq && result.ok) pageFilters = result.value
    })
  })

  // The page's own Page view setting is the default zoom mode.
  $effect(() => {
    const option = page?.displayOption ?? ''
    untrack(() => {
      if (zoomMode === 'custom') return
      zoomMode = option === 'FitToWidth' ? 'fitWidth' : option === 'ActualSize' ? 'actual' : 'fitPage'
    })
  })

  function onMeasures(values: MeasureValue[]) {
    measureValues = new Map(values.filter((m) => m.id != null && m.error == null).map((m) => [m.id as number, m.formatted ?? '']))
  }

  function quickQuery(text: string) {
    if (activeView !== 'dax') setView('dax')
    queueMicrotask(() => daxView?.addQuery(text, true))
  }

  function dataContext(target: DataTarget): MenuItem[] {
    const items: MenuItem[] = []
    if (target.kind === 'table') {
      const t = target.table
      const s = statuses.find((x) => x.name === t.name)
      items.push(
        ...properties(t.id),
        { label: 'Refresh data', icon: 'refresh', enabled: canRefresh && !refreshing && t.kind !== 'remote', onclick: () => void startRefresh([t.name]) },
        { label: 'Load CSV manually…', icon: 'csv', enabled: (s?.canLoadCsv ?? false) && backend.hasBackend && !refreshing, onclick: () => (csvTarget = { table: t.name, reason: reasonOf(t.name) }) },
        { label: 'View data', icon: 'data', onclick: () => showTable(t.name) },
        { separator: true, label: '' },
        { header: true, label: 'Quick queries' },
        { label: 'Show top 100 rows', icon: 'play', enabled: localModel, onclick: () => quickQuery(`EVALUATE\n\tTOPN(100, ${daxTableRef(t.name)})\n`) },
      )
    } else if (target.field.kind === 'measure') {
      const m = target.field.item
      const name = m.name.replaceAll(']', ']]')
      const explain = explainAction(m, target.table)
      items.push(
        ...properties(m.id),
        { label: 'Explain with AI', icon: 'ai', enabled: explain.enabled, title: explain.title, onclick: explain.run },
        { separator: true, label: '' },
        { header: true, label: 'Quick queries' },
        { label: 'Evaluate', icon: 'play', enabled: localModel, onclick: () => quickQuery(`EVALUATE\n\tROW("${m.name.replaceAll('"', '""')}", [${name}])\n`) },
        {
          label: 'Define and evaluate',
          icon: 'play',
          enabled: localModel,
          onclick: () => quickQuery(`DEFINE\n\tMEASURE ${daxColumnRef(target.table.name, m.name)} =\n\t\t${m.expression.trim().replaceAll('\n', '\n\t\t')}\n\nEVALUATE\n\tROW("${m.name.replaceAll('"', '""')}", [${name}])\n`),
        },
      )
    } else if (target.field.kind === 'column') {
      const c = target.field.item
      items.push(
        ...properties(c.id),
        { label: 'View data', icon: 'data', onclick: () => showTable(target.table.name) },
        { separator: true, label: '' },
        { header: true, label: 'Quick queries' },
        { label: 'Show data preview', icon: 'play', enabled: localModel, onclick: () => quickQuery(`EVALUATE\n\tTOPN(100, DISTINCT(${daxColumnRef(target.table.name, c.name)}))\n`) },
      )
    } else {
      items.push(...properties(target.field.item.id))
    }
    return items
  }

  /** "Properties" for the Data pane context menu (the Model view always shows the pane). */
  function properties(id: number): MenuItem[] {
    if (activeView === 'model') return []
    return [
      {
        label: 'Properties',
        icon: 'settings',
        onclick: () => {
          void select(id)
          panes = { ...panes, properties: true }
          collapsed = { ...collapsed, properties: false }
        },
      },
      { separator: true, label: '' },
    ]
  }

  // ---------------------------------------------------------------------------------------------
  // AI (opt-in)
  // ---------------------------------------------------------------------------------------------

  async function refreshAi() {
    aiStatus = await ai.loadStatus()
  }

  function openAiOptions() {
    backstage = null
    optionsOpen = true
  }

  /** Opens the Explain with AI pane for `measure` (a new request each time). */
  function explainMeasure(measure: MeasureDto, table: TableDto) {
    if (!doc) return
    explainTarget = { docId: doc.summary.docId, table: table.name, measure: measure.name, expression: measure.expression, seq: ++explainSeq }
    collapsed = { ...collapsed, explain: false }
  }

  /** Shows the consent dialog; resolves `true` only after "Allow and send" recorded the consent. */
  const askConsent: ai.AskConsent = (need, preview) =>
    new Promise<boolean>((resolve) => {
      consentRequest?.resolve(false)
      consentRequest = { need, preview, resolve }
    })

  function consentDone(granted: boolean) {
    const request = consentRequest
    consentRequest = null
    request?.resolve(granted)
    if (granted) void refreshAi()
  }

  /** File → Export: the document as a PBIP project or a PBIT template (`convert.*`). */
  async function exportDocument(kind: 'pbip' | 'pbit') {
    if (!doc) return
    const stem = doc.summary.fileName.replace(/\.[^.]+$/, '')
    try {
      if (kind === 'pbip') {
        const folder = await backend.pickFolder('Export as a Power BI project: choose an empty folder')
        if (!folder) return
        const result = await backend.exportPbip(folder)
        if (result.ok) toast('info', `Exported ${plural(result.value.files.length, 'file')} to ${folder}.`)
        else fail(result.error)
      } else {
        const path = await backend.pickSavePath('Export as a Power BI template', `${stem}.pbit`, [{ name: 'Power BI template', extensions: ['pbit'] }])
        if (!path) return
        const result = await backend.exportPbit(path, report?.id ?? null)
        if (result.ok) toast('info', `Exported the template of ${result.value.report} to ${baseName(path)}.`)
        else fail(result.error)
      }
      backstage = null
    } catch (e) {
      fail(`Export failed: ${String(e)}`)
    }
  }

  async function pickAndOpen(kind: 'file' | 'folder') {
    try {
      const path = await backend.pickPath(kind)
      if (path) await openPath(path)
    } catch (e) {
      fail(`Open dialog failed: ${String(e)}`)
    }
  }

  function openBackstage(section: BackstageSection = doc ? 'open' : 'home') {
    backstage = section
    void refreshRecent()
  }

  function handleKeydown(event: KeyboardEvent) {
    const mod = event.metaKey || event.ctrlKey
    if (mod && !event.altKey && event.key.toLowerCase() === 'o') {
      event.preventDefault()
      if (backend.hasBackend) void pickAndOpen(event.shiftKey ? 'folder' : 'file')
    } else if (event.ctrlKey && event.key === 'F1') {
      event.preventDefault()
      ribbonCollapsed = !ribbonCollapsed
    } else if (mod && (event.key === 'PageDown' || event.key === 'PageUp') && activeView === 'report' && report && pageIndex >= 0) {
      event.preventDefault()
      const next = report.pages[pageIndex + (event.key === 'PageDown' ? 1 : -1)]
      if (next) selectPage(next.id)
    } else if (event.key === 'F5' && activeView === 'dax') {
      event.preventDefault()
      void daxView?.run()
    } else if (event.key === 'Escape') {
      if (backstage) backstage = null
      else if (diagnosticsOpen) diagnosticsOpen = false
    }
  }

  onMount(() => {
    let disposed = false
    let unlisten: (() => void) | undefined
    let unlistenOpen: (() => void) | undefined
    daxTabs = [newTab(1, defaultQuery())]
    daxSeq = 1

    void (async () => {
      try {
        version = await backend.appVersion()
        aiDemo = await ai.demoMode()
        void refreshAi()
        const stopOpen = await backend.onOpenFileRequested((path) => void openPath(path))
        if (disposed) stopOpen()
        else unlistenOpen = stopOpen
        blobMap = await backend.blobMap()
        if (blobMap?.error) toast('warning', `Blob map not used: ${blobMap.error}`)
        if (!backend.hasBackend) {
          status = { message: 'Browser preview: start with `npm run tauri dev` to open files.', isError: false }
        }
        void refreshRecent()
        const fn = await backend.onFileDrop({
          hover: (active) => (dragActive = active),
          drop: (paths) => {
            // One document at a time: take the first dropped path.
            if (paths.length > 0) void openPath(paths[0])
          },
        })
        if (disposed) fn()
        else unlisten = fn
        // After a webview reload the backend workspace still holds the document: show it
        // without reparsing.
        const current = await backend.currentDocument()
        if (disposed) return
        if (current) {
          ++openSeq
          showDocument(current)
          return
        }
        const startup = await backend.startupPath()
        if (startup && !disposed) await openPath(startup)
      } catch (e) {
        fail(`Startup failed: ${String(e)}`)
      }
    })()

    return () => {
      disposed = true
      unlisten?.()
      unlistenOpen?.()
    }
  })
</script>

<svelte:window onkeydown={handleKeydown} />

{#snippet propertiesPane(closable: boolean)}
  <Pane
    title="Properties"
    icon="settings"
    collapsed={collapsed.properties}
    ontogglecollapse={() => (collapsed = { ...collapsed, properties: !collapsed.properties })}
    onclose={closable ? () => (panes = { ...panes, properties: false }) : undefined}
    width={280}
  >
    <PropertiesPane {details} loading={detailsLoading} error={detailsError} explain={measureExplain} />
  </Pane>
{/snippet}

{#snippet explainPane()}
  {#if explainTarget}
    <Pane
      title="Explain with AI"
      icon="ai"
      collapsed={collapsed.explain}
      ontogglecollapse={() => (collapsed = { ...collapsed, explain: !collapsed.explain })}
      onclose={() => (explainTarget = null)}
      width={340}
    >
      <!-- The AI UI is a separate chunk, loaded on first use. -->
      {#await import('./components/ai/ExplainPane.svelte') then { default: ExplainPane }}
        <ExplainPane target={explainTarget} ask={askConsent} onoptions={openAiOptions} />
      {/await}
    </Pane>
  {/if}
{/snippet}

{#snippet dataPane(initialShowHidden: boolean)}
  <Pane title="Data" icon="getData" collapsed={collapsed.data} ontogglecollapse={() => (collapsed = { ...collapsed, data: !collapsed.data })}>
    {#key doc?.summary.docId}
      <DataPane
        {tree}
        {selected}
        onselect={(id) => void select(id)}
        {statuses}
        used={usedFields}
        values={measureValues}
        current={activeView === 'table' ? (dataTable?.name ?? null) : null}
        contextItems={dataContext}
        {initialShowHidden}
      />
    {/key}
  </Pane>
{/snippet}

<div class="shell">
  <TitleBar title={windowTitle} items={searchItems} />
  <Ribbon tabs={ribbonTabs} active={ribbonTab} collapsed={ribbonCollapsed} onselect={(id) => (ribbonTab = id)} onfile={() => openBackstage()} ontogglecollapse={() => (ribbonCollapsed = !ribbonCollapsed)} />

  {#if viewing && doc}
    <div class="bar view-as" role="status" title="Every visual, table, measure and DAX query shows only the rows these roles allow (row-level security). Nothing is saved.">
      <Icon name="viewAs" />
      <span>
        <strong>Now viewing as: {viewing.roles.length > 0 ? viewing.roles.join(', ') : (viewing.userName ?? 'no role')}</strong>
        {#if viewing.userName && viewing.roles.length > 0}<span class="view-as-user">Other user: <code>{viewing.userName}</code></span>{/if}
        {#if viewing.error}<span class="view-as-error">The role filters could not be applied, so every table is shown empty: {viewing.error}</span>{/if}
      </span>
      <button class="bar-action" onclick={() => (viewAsOpen = true)} disabled={stoppingViewAs} title="Change the roles or the user">
        <Icon name="viewAs" size={14} />
        View as…
      </button>
      <button class="bar-action plain" onclick={() => void stopViewAs()} disabled={stoppingViewAs} title="Show all rows again">Stop viewing</button>
    </div>
  {/if}

  {#if modelStatus?.availability === 'remote'}
    <div class="bar" role="note" title={modelStatus.description ?? ''}>
      <Icon name="remote" />
      <span>
        <strong>Remote model — not available offline.</strong>
        The report is bound to a semantic model in the Power BI service (<code>byConnection</code>). Pages, visuals and fields are shown; data and the model need a local model.
      </span>
    </div>
  {:else if modelStatus?.availability === 'missing' && (doc?.summary.counts.reports ?? 0) > 0}
    <div class="bar" role="note">
      <Icon name="info" />
      <span>No semantic model was found next to this report; only the report layout is available.</span>
    </div>
  {/if}
  {#if cacheNote && doc}
    <div class="bar" role="note" title={doc.summary.cache?.entry ?? ''}>
      <Icon name="info" />
      <span>
        <strong>Loaded cached data{cacheNote.asOf ? ` (as of ${cacheNote.asOf})` : ''}.</strong>
        These are the values of Power BI Desktop's last refresh, stored in <code>.pbi/cache.abf</code>; no queries were run.
      </span>
      <button class="bar-action" onclick={() => startRefresh(null, 'query')} disabled={!canRefresh || refreshing} title="Run the M queries offline (through the blob map) and recompute the calculated columns and tables">
        <Icon name="refresh" size={14} />
        Refresh from source
      </button>
      <button class="bar-close" onclick={() => (cacheNote = null)} title="Hide this note" aria-label="Hide this note">
        <Icon name="close" size={12} />
      </button>
    </div>
  {/if}

  {#if templateNote && doc}
    <div class="bar" role="note">
      <Icon name="info" />
      <span>
        <strong>Loaded with the template's saved parameter values.</strong>
        Power BI Desktop asks for them when it opens a template; DashCraft does not edit parameters yet.
      </span>
      <button class="bar-action" onclick={() => (powerQuery = { initial: doc?.tree.expressions.find((e) => e.isParameter)?.id ?? null })} title="Show the parameters and their values">
        View parameters
      </button>
      <button class="bar-close" onclick={() => (templateNote = false)} title="Hide this note" aria-label="Hide this note">
        <Icon name="close" size={12} />
      </button>
    </div>
  {/if}

  {#if formula}
    <FormulaBar name={formula.name} expression={formula.expression} kind={formula.kind} explain={formula.kind === 'Measure' ? measureExplain : null} />
  {/if}

  <div class="workspace" aria-busy={busy}>
    <ViewRail active={activeView} onselect={setView} disabled={doc ? [] : ['table', 'model', 'dax', 'tmdl']} />

    <main class="center">
      {#if !doc}
        <div class="welcome">
          <div class="welcome-card">
            <h2>Open a Power BI file</h2>
            <p class="muted">PBIX, PBIT, PBIP, BIM and TMDL files or a PBIP / TMDL folder — read offline, nothing leaves this computer.</p>
            <div class="welcome-actions">
              <button onclick={() => pickAndOpen('file')} disabled={!backend.hasBackend} title="Ctrl+O"><Icon name="open" size={28} /><span>Open file</span></button>
              <button onclick={() => pickAndOpen('folder')} disabled={!backend.hasBackend} title="Ctrl+Shift+O"><Icon name="folder" size={28} /><span>Open folder</span></button>
              <button onclick={() => openBackstage('home')}><Icon name="recent" size={28} /><span>Recent files</span></button>
            </div>
            <p class="muted small">Or drop a file on the window, or start with <code>dashcraft-app &lt;path&gt;</code>.</p>
          </div>
        </div>
      {:else}
        {#key doc.summary.docId}
          {#if activeView === 'report'}
            <ReportView
              {report}
              {page}
              {selectedVisualId}
              onselectvisual={selectVisual}
              onselect={(id) => void select(id)}
              {zoomMode}
              {zoomPercent}
              onzoom={(p) => (effectiveZoom = p)}
              onzoomrequest={(p) => {
                zoomPercent = p
                zoomMode = 'custom'
              }}
              {canvasMode}
              filters={pageFilters}
              {dataVersion}
              {dataLoaded}
              {hasMeasures}
              onmeasures={onMeasures}
              {canvasIncoming}
              {canvasFilters}
              oncanvasrendered={canvasRendered}
              oncanvasaction={canvasAction}
              oncanvaserror={fail}
            />
          {:else if activeView === 'table'}
            <TableView
              tables={doc.tree.tables}
              table={dataTable}
              {statuses}
              version={dataVersion}
              {refreshing}
              {selected}
              mode={tableMode}
              onselect={(id) => void select(id)}
              onloadcsv={(table, reason) => (csvTarget = { table, reason })}
              ondatachanged={dataChanged}
              onerror={fail}
              oninfo={(i) => (tableInfo = i)}
              bind:parsePeople={parsePeopleRequest}
            />
          {:else if activeView === 'model'}
            <ModelView tree={doc.tree} {diagram} {selected} onselect={(id) => void select(id)} zoom={modelZoom} onzoom={(p) => (modelZoom = p)} fitToken={modelFitToken} />
          {:else if activeView === 'dax'}
            <DaxQueryView bind:this={daxView} bind:tabs={daxTabs} bind:active={daxActive} hasData={dataLoaded} nextId={() => ++daxSeq} />
          {:else}
            <TmdlView {tmdl} error={tmdlError} loading={tmdlLoading} bind:path={tmdlPath} />
          {/if}
        {/key}
      {/if}
      {#if activeView === 'report' && report}
        <div class="page-nav">
          {#if reports.length > 1}
            <label class="report-pick" title="This project has several reports">
              <Icon name="report" size={14} />
              <select value={report.id} onchange={(e) => selectReport(Number((e.currentTarget as HTMLSelectElement).value))} aria-label="Report">
                {#each reports as r (r.id)}<option value={r.id}>{r.name}</option>{/each}
              </select>
            </label>
          {/if}
          <PageTabs pages={report.pages} activeId={page?.id ?? null} onselect={selectPage} />
        </div>
      {/if}
    </main>

    {#if doc}
      {@render explainPane()}
      {#if activeView === 'report'}
        {#if panes.properties}{@render propertiesPane(true)}{/if}
        {#if panes.selection}
          <Pane title="Selection" icon="selection" collapsed={collapsed.selection} ontogglecollapse={() => (collapsed = { ...collapsed, selection: !collapsed.selection })} onclose={() => (panes = { ...panes, selection: false })}>
            <SelectionPane {page} selectedId={selectedVisualId} onselect={(id) => selectVisual(id)} />
          </Pane>
        {/if}
        {#if panes.filters}
          <Pane title="Filters" icon="filter" collapsed={collapsed.filters} ontogglecollapse={() => (collapsed = { ...collapsed, filters: !collapsed.filters })} width={232}>
            <FiltersPane
              {report}
              {page}
              visual={selectedVisual}
              {selected}
              onselect={(id) => void select(id)}
              selections={canvasInfo?.selections ?? []}
              onclearselection={(v) => void clearCanvasSelection(v)}
              onselectvisualname={selectVisualByName}
            />
          </Pane>
        {/if}
        <Pane title="Visualizations" icon="report" collapsed={collapsed.viz} ontogglecollapse={() => (collapsed = { ...collapsed, viz: !collapsed.viz })} width={248}>
          <VisualizationsPane visual={selectedVisual} {page} onselectfield={selectField} />
        </Pane>
        {@render dataPane(false)}
      {:else if activeView === 'model'}
        {@render propertiesPane(false)}
        {@render dataPane(true)}
      {:else}
        {#if panes.properties}{@render propertiesPane(true)}{/if}
        {@render dataPane(false)}
      {/if}
    {/if}
  </div>

  {#if run && refreshPanelOpen}
    <RefreshPanel
      {run}
      oncancel={cancelRefresh}
      onclose={() => (refreshPanelOpen = false)}
      onloadcsv={(table, reason) => (csvTarget = { table, reason })}
      onselecttable={showTable}
    />
  {/if}

  {#if csvTarget}
    {@const target = csvTarget}
    <CsvDialog table={target.table} reason={target.reason} onclose={() => (csvTarget = null)} onloaded={(result) => csvLoaded(target.table, result)} />
  {/if}

  {#if powerQuery && doc}
    <PowerQueryDialog tree={doc.tree} initial={powerQuery.initial} onclose={() => (powerQuery = null)} />
  {/if}

  {#if viewAsOpen && doc && security}
    <ViewAsDialog roles={security.roles} current={security.viewAs} onclose={() => (viewAsOpen = false)} onapplied={viewAsApplied} />
  {/if}

  {#if relationshipsOpen && doc}
    <RelationshipsDialog relationships={doc.tree.relationships} {selected} onselect={(id) => void select(id)} onclose={() => (relationshipsOpen = false)} />
  {/if}

  {#if suggestOpen && doc}
    {@const current = doc}
    {#await import('./components/ai/SuggestMeasureDialog.svelte') then { default: SuggestMeasureDialog }}
      <SuggestMeasureDialog
        docId={current.summary.docId}
        tables={current.tree.tables.filter((t) => t.kind !== 'remote').map((t) => t.name)}
        initialTable={selection?.table.name ?? null}
        status={aiStatus}
        ask={askConsent}
        onclose={() => (suggestOpen = false)}
        onoptions={openAiOptions}
        onopenquery={(query) => {
          suggestOpen = false
          quickQuery(query)
        }}
        ondatachanged={() => void dataChanged()}
      />
    {/await}
  {/if}

  {#if optionsOpen}
    {#await import('./components/ai/OptionsDialog.svelte') then { default: OptionsDialog }}
      <OptionsDialog status={aiStatus} demo={aiDemo} onstatus={(s) => (aiStatus = s)} onclose={() => (optionsOpen = false)} />
    {/await}
  {/if}

  {#if consentRequest}
    {@const request = consentRequest}
    {#key request}
      {#await import('./components/ai/ConsentDialog.svelte') then { default: ConsentDialog }}
        <ConsentDialog need={request.need} preview={request.preview} ondone={consentDone} />
      {/await}
    {/key}
  {/if}

  {#if webLink}
    {@const url = webLink}
    <Modal title="Web link" width={480} onclose={() => (webLink = null)}>
      <p>This button opens a web page. DashCraft works offline and does not open links; copy it to open it in your browser if you trust it:</p>
      <p class="web-link"><code>{url}</code></p>
      {#snippet footer()}
        <button onclick={() => void copyLink(url)}>Copy link</button>
        <button onclick={() => (webLink = null)}>Close</button>
      {/snippet}
    </Modal>
  {/if}

  {#if diagnosticsOpen && doc}
    <DiagnosticsPanel diagnostics={doc.diagnostics} detection={doc.detection} onselect={(id) => void select(id)} onclose={() => (diagnosticsOpen = false)} />
  {/if}

  <StatusBar
    text={statusText}
    message={status.message}
    isError={status.isError}
    summary={doc?.summary ?? null}
    data={dataStatus}
    zoom={statusZoom}
    {version}
    hasBackend={backend.hasBackend}
    {diagnosticsOpen}
    ontoggleDiagnostics={() => (diagnosticsOpen = !diagnosticsOpen)}
  />
</div>

{#if backstage}
  <Backstage
    section={backstage}
    {recent}
    {blobMap}
    {version}
    hasDoc={doc !== null}
    hasBackend={backend.hasBackend}
    onsection={(s) => (backstage = s)}
    onback={() => (backstage = null)}
    onopen={(kind) => void pickAndOpen(kind)}
    onopenpath={(path) => void openPath(path)}
    onremoverecent={(path) => void removeRecent(path)}
    onchooseblobmap={() => void chooseBlobMap()}
    onclearblobmap={() => void clearBlobMap()}
    onclosedoc={() => void closeDocument()}
    canExport={localModel}
    onexport={(kind) => void exportDocument(kind)}
    onoptions={openAiOptions}
    aiEnabled={aiStatus?.enabled ?? false}
  />
{/if}

{#if busy}
  <div class="busy" role="progressbar" aria-label={busyLabel}>
    <div class="busy-card">
      <span class="spinner" aria-hidden="true"></span>
      <span>{busyLabel}</span>
    </div>
  </div>
{/if}

<DropOverlay active={dragActive} />
<Toasts {toasts} ondismiss={dismiss} />

<style>
  .shell {
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  .web-link code {
    display: block;
    padding: 6px 8px;
    background: var(--color-code-bg);
    font-family: var(--font-mono);
    word-break: break-all;
    user-select: all;
  }

  .bar {
    display: flex;
    flex: none;
    align-items: center;
    gap: 8px;
    padding: 6px 14px;
    border-bottom: 1px solid var(--color-warning-border);
    background: var(--color-warning-bar);
    font-size: 12px;
  }

  .bar-action {
    display: inline-flex;
    flex: none;
    align-items: center;
    gap: 4px;
    margin-left: auto;
    padding: 3px 10px;
    border: 1px solid var(--color-border-strong);
    border-radius: 4px;
    background: var(--color-pane);
    font-size: 12px;
    cursor: pointer;
  }

  .bar-action.plain {
    margin-left: 0;
  }

  .view-as-user,
  .view-as-error {
    margin-left: 8px;
  }

  .view-as-error {
    color: var(--color-danger);
  }

  .bar-action:hover:not(:disabled) {
    background: var(--color-hover);
  }

  .bar-action:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .bar-close {
    display: inline-grid;
    flex: none;
    place-items: center;
    width: 22px;
    height: 22px;
    border: none;
    border-radius: 3px;
    background: transparent;
    color: var(--color-text-muted);
    cursor: pointer;
  }

  .bar-close:hover {
    background: rgb(0 0 0 / 0.06);
  }

  .workspace {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .center {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--color-canvas);
  }

  .center > :global(:first-child) {
    flex: 1;
    min-height: 0;
  }

  .page-nav {
    display: flex;
    flex: none !important;
    min-width: 0;
  }

  .page-nav :global(.page-tabs) {
    flex: 1;
    min-width: 0;
  }

  .report-pick {
    display: flex;
    flex: none;
    align-items: center;
    gap: 6px;
    padding: 0 8px;
    border-top: 1px solid var(--color-border);
    border-right: 1px solid var(--color-border);
    background: var(--color-pagetabs);
    color: var(--color-text-muted);
  }

  .report-pick select {
    max-width: 220px;
    border: 1px solid var(--color-border-strong);
    border-radius: 3px;
    background: var(--color-pane);
    font-size: 12px;
  }

  .welcome {
    display: grid;
    place-items: center;
    height: 100%;
    background: var(--color-canvas);
  }

  .welcome-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    max-width: 640px;
    padding: 32px 40px;
    border-radius: 6px;
    background: var(--color-pane);
    box-shadow: 0 1.6px 3.6px rgb(0 0 0 / 0.13), 0 0.3px 0.9px rgb(0 0 0 / 0.11);
    text-align: center;
  }

  .welcome-card h2 {
    margin: 0;
    font-size: 22px;
    font-weight: 600;
  }

  .welcome-card p {
    margin: 0;
  }

  .welcome-actions {
    display: flex;
    gap: 12px;
    margin: 16px 0 8px;
  }

  .welcome-actions button {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    width: 140px;
    height: 96px;
    border: 1px solid var(--color-border);
    border-radius: 6px;
    background: var(--color-pane);
    cursor: pointer;
  }

  .welcome-actions button:hover:not(:disabled) {
    border-color: var(--color-brand);
    background: var(--color-brand-soft);
  }

  .welcome-actions button:disabled {
    color: var(--color-text-disabled);
    cursor: default;
  }

  .small {
    font-size: 12px;
  }

  .busy {
    position: fixed;
    inset: var(--titlebar-height) 0 var(--statusbar-height);
    z-index: 150;
    display: grid;
    place-items: center;
    background: rgb(255 255 255 / 0.55);
  }

  .busy-card {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 14px 20px;
    border: 1px solid var(--color-border-strong);
    border-radius: 6px;
    background: var(--color-pane);
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.12);
  }

  .spinner {
    width: 20px;
    height: 20px;
    border: 3px solid var(--color-border);
    border-top-color: var(--color-brand);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation-duration: 2.4s;
    }
  }
</style>
