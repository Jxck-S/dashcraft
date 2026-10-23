// The ribbon of each view, as in Power BI Desktop (tab, group and button names and order from
// Microsoft Learn: see app/docs/desktop-ui-reference.md). Commands DashCraft implements are
// enabled and wired to `RibbonContext.act`; everything else is shown disabled with the "Not
// available in DashCraft (yet)" tooltip.

import type { BlobMapDto, ColumnDto, MeasureDto, ReportThemeDto, TableDto } from './bindings'
import { dataTypeLabel } from './data'
import { button, field, group, small, stack, type MenuItem, type RibbonGroup, type RibbonTab, type ViewId, type ZoomMode } from './shell'

/** The model object selected in the Data pane, for the contextual tabs. */
export type RibbonSelection =
  | { kind: 'measure'; measure: MeasureDto; table: TableDto }
  | { kind: 'column'; column: ColumnDto; table: TableDto }
  | { kind: 'table'; table: TableDto }

export interface RibbonActions {
  refresh: () => void
  transformData: () => void
  editParameters: () => void
  chooseBlobMap: () => void
  clearBlobMap: () => void
  loadCsv: (table: string) => void
  manageRelationships: () => void
  setZoomMode: (mode: ZoomMode) => void
  togglePane: (pane: 'filters' | 'selection' | 'properties') => void
  setCanvasMode: (mode: 'wireframe' | 'outline') => void
  /** "Reset to default": the report's saved slicer selections, no cross-filter. */
  resetToDefault: () => void
  daxComment: (add: boolean) => void
  refreshTable: (table: string) => void
  setTableMode: (mode: 'grid' | 'schema') => void
  columnStats: (column: string) => void
  parsePeople: (column: string) => void
  about: () => void
  /** AI: the Suggest a measure dialog. */
  suggestMeasure: () => void
  /** AI: the Explain with AI pane for a measure. */
  explainMeasure: (measure: MeasureDto, table: TableDto) => void
  /** File → Options and settings → Options → AI. */
  aiOptions: () => void
  /** Modeling → View as: the "View as roles" dialog. */
  viewAs: () => void
}

export interface RibbonContext {
  view: ViewId
  hasDoc: boolean
  hasBackend: boolean
  /** A local semantic model is loaded. */
  localModel: boolean
  canRefresh: boolean
  refreshing: boolean
  blobMap: BlobMapDto | null
  /** Tables "Load CSV manually…" can fill. */
  csvTables: string[]
  hasParameters: boolean
  hasReport: boolean
  theme: ReportThemeDto | null
  zoomMode: ZoomMode
  panes: { filters: boolean; selection: boolean; properties: boolean }
  canvasMode: 'wireframe' | 'outline'
  /** The canvas has slicer selections or a cross-filter that differ from the report's saved state. */
  canReset: boolean
  /** Data pane selection (contextual tabs in the Report view). */
  selection: RibbonSelection | null
  /** Table view: the table shown, its grid mode and the selected grid column. */
  table: TableDto | null
  tableCanLoadCsv: boolean
  tableMode: 'grid' | 'schema'
  column: { name: string; dataType: string; derived: boolean; formatString: string | null } | null
  /** The opt-in AI features can run (enabled with a provider); else why not (tooltip). */
  ai: { ready: boolean; reason: string | null }
  /** Row-level security: the model's role count (`null` until known) and whether "View as" is active. */
  security: { roles: number | null; active: boolean; label: string }
  act: RibbonActions
}

const off = (id: string, label: string, icon: Parameters<typeof button>[2]) => button(id, label, icon)
const offSmall = (id: string, label: string, icon: Parameters<typeof button>[2]) => small(id, label, icon)

function clipboard(): RibbonGroup {
  return group(
    'clipboard',
    'Clipboard',
    button('paste', 'Paste', 'paste'),
    stack('clip-stack', offSmall('cut', 'Cut', 'cut'), offSmall('copy', 'Copy', 'copy'), offSmall('format-painter', 'Format painter', 'formatPainter')),
  )
}

function data(): RibbonGroup {
  return group(
    'data',
    'Data',
    off('get-data', 'Get data', 'getData'),
    off('excel', 'Excel workbook', 'excel'),
    off('onelake', 'OneLake catalog', 'onelake'),
    off('sql-server', 'SQL Server', 'sqlServer'),
    off('enter-data', 'Enter data', 'enterData'),
    off('dataverse', 'Dataverse', 'dataverse'),
    off('recent-sources', 'Recent sources', 'recentSources'),
  )
}

function queries(ctx: RibbonContext): RibbonGroup {
  const { act } = ctx
  const menu: MenuItem[] = [
    { label: 'Transform data', icon: 'transformData', enabled: ctx.localModel, title: 'Power Query Editor (read-only): M queries, parameters, DAX', onclick: act.transformData },
    { label: 'Data source settings', icon: 'blob', enabled: ctx.hasBackend && !ctx.refreshing, title: 'Blob map…: AzureStorage.Blobs URLs → local files (no network)', onclick: act.chooseBlobMap },
    ...(ctx.blobMap?.path ? [{ label: 'Stop using the blob map', icon: 'close' as const, enabled: !ctx.refreshing, onclick: act.clearBlobMap }] : []),
    { label: 'Edit parameters', icon: 'parameter', enabled: ctx.hasParameters, title: 'Show the parameters (read-only)', onclick: act.editParameters },
    { separator: true, label: '' },
    { header: true, label: 'Load CSV manually…' },
    ...(ctx.csvTables.length > 0
      ? ctx.csvTables.map((t): MenuItem => ({ label: t, icon: 'csv', enabled: ctx.hasBackend && !ctx.refreshing, onclick: () => act.loadCsv(t) }))
      : [{ label: 'No table can load a CSV', enabled: false }]),
  ]
  return group(
    'queries',
    'Queries',
    button('transform-data', 'Transform data', 'transformData', {
      enabled: ctx.localModel,
      split: true,
      menu,
      title: 'Transform data: the Power Query Editor (read-only). The arrow has Data source settings (blob map), Edit parameters and Load CSV manually…',
      onclick: act.transformData,
    }),
    button('refresh', 'Refresh', 'refresh', {
      enabled: ctx.canRefresh && !ctx.refreshing,
      title: ctx.refreshing ? 'A refresh is running' : 'Refresh all tables offline: M queries (through the blob map), then calculated columns and tables',
      onclick: act.refresh,
    }),
  )
}

function calculations(full: boolean): RibbonGroup {
  return group(
    'calculations',
    'Calculations',
    off('new-measure', 'New measure', 'newMeasure'),
    off('quick-measure', 'Quick measure', 'quickMeasure'),
    ...(full ? [off('new-column', 'New column', 'newColumn'), off('new-table', 'New table', 'newTable')] : []),
  )
}

/**
 * DashCraft's opt-in AI features (pbi-ai; neutral name, Fluent "Sparkle" icon). Visible but
 * disabled until AI is turned on with a provider in Options; "AI options" always opens them.
 */
function aiGroup(ctx: RibbonContext, measure: { measure: MeasureDto; table: TableDto } | null): RibbonGroup {
  const { act } = ctx
  const offTitle = ctx.ai.reason ?? undefined
  const suggestTitle = !ctx.ai.ready
    ? offTitle
    : ctx.localModel
      ? 'Suggest a measure (AI): describe it in plain language and get DAX, checked locally. Never written to the model.'
      : 'Needs a document with a local model'
  const explainTitle = !ctx.ai.ready
    ? offTitle
    : !ctx.localModel
      ? 'Needs a document with a local model'
      : measure
        ? `Explain ${measure.measure.name} with AI: what it computes, filter context, dependencies, caveats`
        : 'Select a measure in the Data pane first'
  return group(
    'ai',
    'AI',
    button('ai-suggest-measure', 'Suggest a measure', 'ai', { enabled: ctx.ai.ready && ctx.localModel, title: suggestTitle, onclick: act.suggestMeasure }),
    stack(
      'ai-stack',
      small('ai-explain-measure', 'Explain measure', 'ai', {
        enabled: ctx.ai.ready && ctx.localModel && measure !== null,
        title: explainTitle,
        onclick: () => measure && act.explainMeasure(measure.measure, measure.table),
      }),
      small('ai-options', 'AI options', 'settings', { enabled: ctx.hasBackend, title: 'File → Options and settings → Options → AI: turn AI on, providers, keys, consents', onclick: act.aiOptions }),
    ),
  )
}

function selectedMeasure(ctx: RibbonContext): { measure: MeasureDto; table: TableDto } | null {
  return ctx.selection?.kind === 'measure' ? { measure: ctx.selection.measure, table: ctx.selection.table } : null
}

function manageRelationships(ctx: RibbonContext) {
  return button('manage-relationships', 'Manage relationships', 'manageRelationships', {
    enabled: ctx.localModel,
    title: 'Manage relationships (read-only list)',
    onclick: ctx.act.manageRelationships,
  })
}

/** Security: Manage roles (not available) and View as (row-level security, `security.viewAs`). */
function security(ctx: RibbonContext): RibbonGroup {
  const { roles, active, label } = ctx.security
  const enabled = ctx.hasBackend && ctx.localModel && (roles ?? 0) > 0
  const title = !ctx.hasBackend
    ? undefined
    : !ctx.localModel
      ? 'View as: needs a document with a local model'
      : roles == null
        ? 'View as: reading the roles…'
        : roles === 0
          ? 'View as: this model has no roles (row-level security)'
          : active
            ? `View as roles: now viewing as ${label}. Change the roles or the user, or stop viewing`
            : 'View as roles: see the report and the data as members of security roles and / or another user (row-level security; nothing is saved)'
  return group(
    'security',
    'Security',
    off('manage-roles', 'Manage roles', 'manageRoles'),
    button('view-as', 'View as', 'viewAs', { enabled, pressed: active, title, onclick: ctx.act.viewAs }),
  )
}

function share(): RibbonGroup[] {
  return [group('sensitivity', 'Sensitivity', off('sensitivity', 'Sensitivity', 'sensitivity')), group('share', 'Share', off('publish', 'Publish', 'publish'))]
}

function homeTab(ctx: RibbonContext): RibbonTab {
  const groups: RibbonGroup[] = [clipboard()]
  switch (ctx.view) {
    case 'report':
      groups.push(
        data(),
        queries(ctx),
        group('insert', 'Insert', off('new-visual', 'New visual', 'newVisual'), off('text-box', 'Text box', 'textBox'), off('more-visuals', 'More visuals', 'moreVisuals')),
        calculations(false),
        aiGroup(ctx, selectedMeasure(ctx)),
        ...share(),
      )
      break
    case 'table':
      groups.push(data(), queries(ctx), calculations(false), aiGroup(ctx, selectedMeasure(ctx)), ...share())
      break
    case 'model':
      groups.push(
        data(),
        queries(ctx),
        group('relationships', 'Relationships', manageRelationships(ctx)),
        calculations(false),
        aiGroup(ctx, selectedMeasure(ctx)),
        security(ctx),
        ...share(),
      )
      break
    case 'dax':
    case 'tmdl': {
      const editable = ctx.view === 'dax'
      groups.push(
        group(
          'editing',
          'Editing',
          off('format', 'Format', 'format'),
          button('comment', 'Comment', 'comment', { enabled: editable, title: editable ? 'Comment the selected lines (Ctrl+/)' : undefined, onclick: () => ctx.act.daxComment(true) }),
          button('uncomment', 'Uncomment', 'uncomment', { enabled: editable, title: editable ? 'Uncomment the selected lines (Ctrl+/)' : undefined, onclick: () => ctx.act.daxComment(false) }),
          off('find', 'Find', 'find'),
          off('replace', 'Replace', 'replace'),
          off('command-palette', 'Command palette', 'commandPalette'),
        ),
      )
      break
    }
  }
  return { id: 'home', label: 'Home', groups }
}

function insertTab(): RibbonTab {
  return {
    id: 'insert',
    label: 'Insert',
    groups: [
      group('pages', 'Pages', off('new-page', 'New page', 'newPage')),
      group('visuals', 'Visuals', off('new-visual', 'New visual', 'newVisual'), off('more-visuals', 'More visuals', 'moreVisuals')),
      group(
        'ai-visuals',
        'AI visuals',
        off('key-influencers', 'Key influencers', 'keyInfluencers'),
        off('decomposition-tree', 'Decomposition tree', 'decompositionTree'),
        off('qna', 'Q&A', 'qna'),
        off('narrative', 'Narrative', 'narrative'),
      ),
      group('power-platform', 'Power Platform', off('paginated', 'Paginated report', 'paginated'), off('power-apps', 'Power Apps', 'powerApps'), off('power-automate', 'Power Automate', 'powerAutomate')),
      group('elements', 'Elements', off('text-box', 'Text box', 'textBox'), off('buttons', 'Buttons', 'buttons'), off('shapes', 'Shapes', 'shapes'), off('image', 'Image', 'image')),
      group('sparklines', 'Sparklines', off('add-sparkline', 'Add', 'sparklines')),
    ],
  }
}

function modelingTab(ctx: RibbonContext): RibbonTab {
  return {
    id: 'modeling',
    label: 'Modeling',
    groups: [
      group('relationships', 'Relationships', manageRelationships(ctx)),
      group(
        'calculations',
        'Calculations',
        off('new-measure', 'New measure', 'newMeasure'),
        off('quick-measure', 'Quick measure', 'quickMeasure'),
        off('new-column', 'New column', 'newColumn'),
        off('new-table', 'New table', 'newTable'),
        off('calculation-group', 'Calculation group', 'calcGroup'),
      ),
      aiGroup(ctx, selectedMeasure(ctx)),
      group('parameters', 'Parameters', off('new-parameter', 'New parameter', 'newParameter')),
      group('page-refresh', 'Page refresh', off('change-detection', 'Change detection', 'pageRefresh')),
      security(ctx),
      group(
        'qna',
        'Q&A',
        off('qna-setup', 'Q&A setup', 'qnaSetup'),
        stack('qna-stack', offSmall('language', 'Language', 'language'), offSmall('linguistic-schema', 'Linguistic schema', 'linguisticSchema')),
      ),
    ],
  }
}

function designTab(ctx: RibbonContext): RibbonTab {
  return {
    id: 'design',
    label: 'Design',
    groups: [
      group(
        'themes',
        'Themes',
        { kind: 'themes', id: 'theme-gallery-view', theme: ctx.theme },
        off('theme-gallery', 'Theme gallery', 'themeGallery'),
        stack('theme-stack', offSmall('customize-theme', 'Customize theme', 'customizeTheme'), offSmall('download-theme', 'Download theme', 'downloadTheme'), offSmall('import-theme', 'Import theme', 'importTheme')),
      ),
      group('customize-page', 'Customize page', off('page-size', 'Page size', 'pageSize'), off('page-background', 'Page background', 'pageBackground'), off('wallpaper', 'Wallpaper', 'wallpaper')),
      group('design-help', 'Help', off('theme-help', 'How to create a theme', 'help')),
    ],
  }
}

function viewTab(ctx: RibbonContext): RibbonTab {
  const { act } = ctx
  const mode = (m: ZoomMode, label: string): MenuItem => ({ label, checked: ctx.zoomMode === m, enabled: ctx.hasReport, onclick: () => act.setZoomMode(m) })
  return {
    id: 'view',
    label: 'View',
    groups: [
      group(
        'scale',
        'Scale to fit',
        button('page-view', 'Page view', 'pageView', {
          enabled: ctx.hasReport,
          title: 'Page view: Fit to page, Fit to width, Actual size',
          menu: [mode('fitPage', 'Fit to page'), mode('fitWidth', 'Fit to width'), mode('actual', 'Actual size')],
        }),
      ),
      group('mobile', 'Mobile', off('mobile-layout', 'Mobile layout', 'mobileLayout')),
      group('page-options', 'Page options', stack('page-options-stack', offSmall('gridlines', 'Gridlines', 'gridlines'), offSmall('snap-to-grid', 'Snap to grid', 'snapToGrid'), offSmall('lock-objects', 'Lock objects', 'lockObjects'))),
      group(
        'show-panes',
        'Show panes',
        button('pane-filters', 'Filters', 'filter', { enabled: ctx.hasReport, pressed: ctx.panes.filters, title: 'Show or hide the Filters pane', onclick: () => act.togglePane('filters') }),
        off('pane-bookmarks', 'Bookmarks', 'bookmarks'),
        button('pane-selection', 'Selection', 'selection', { enabled: ctx.hasReport, pressed: ctx.panes.selection, title: 'Show or hide the Selection pane', onclick: () => act.togglePane('selection') }),
        off('pane-performance', 'Performance analyzer', 'performance'),
        off('pane-sync-slicers', 'Sync slicers', 'syncSlicers'),
      ),
      group(
        'dashcraft-canvas',
        'DashCraft',
        button('pane-properties', 'Properties', 'settings', {
          enabled: ctx.hasDoc,
          pressed: ctx.panes.properties,
          title: 'DashCraft: show the Properties pane (everything about the selected object: properties, DAX / M / JSON, source file)',
          onclick: () => act.togglePane('properties'),
        }),
        button('reset-default', 'Reset to default', 'undo', {
          enabled: ctx.hasReport && ctx.canReset,
          title: ctx.canReset
            ? 'Reset to default: the slicer selections saved in the report, no cross-filter, scroll positions reset'
            : 'Reset to default: the page already shows the report’s saved state',
          onclick: act.resetToDefault,
        }),
        button('canvas-outline', 'Outline', 'hierarchy', {
          enabled: ctx.hasReport,
          pressed: ctx.canvasMode === 'outline',
          title: 'DashCraft: show the page as a table of visuals (position, fields, filters) instead of the canvas',
          onclick: () => act.setCanvasMode(ctx.canvasMode === 'outline' ? 'wireframe' : 'outline'),
        }),
      ),
    ],
  }
}

function optimizeTab(): RibbonTab {
  return {
    id: 'optimize',
    label: 'Optimize',
    groups: [
      group('optimize-queries', 'Queries', off('pause-visuals', 'Pause visuals', 'pause'), off('refresh-visuals', 'Refresh visuals', 'refresh')),
      group('optimize-report', 'Report', off('optimization-presets', 'Optimization presets', 'optimizationPresets')),
      group('review', 'Review', off('performance-analyzer', 'Performance analyzer', 'performance')),
      group('apply', 'Apply', off('apply-all-slicers', 'Apply all slicers button', 'applySlicers')),
    ],
  }
}

function helpTab(ctx: RibbonContext): RibbonTab {
  return {
    id: 'help',
    label: 'Help',
    groups: [
      group(
        'help',
        'Help',
        off('guided-learning', 'Guided learning', 'guidedLearning'),
        off('documentation', 'Documentation', 'documentation'),
        off('training-videos', 'Training videos', 'training'),
        off('support', 'Support', 'support'),
        button('about', 'About', 'about', { enabled: true, title: 'About DashCraft: version, licenses, notices', onclick: ctx.act.about }),
      ),
      group('community', 'Community', off('blog', 'Blog', 'blog'), off('community', 'Community', 'community'), off('submit-idea', 'Submit an idea', 'idea')),
    ],
  }
}

function measureTools(ctx: RibbonContext, m: MeasureDto, t: TableDto): RibbonTab {
  return {
    id: 'measure-tools',
    label: 'Measure tools',
    contextual: true,
    groups: [
      group('structure', 'Structure', field('name', 'Name', m.name, { icon: 'tag', width: 170 }), field('home-table', 'Home table', t.name, { icon: 'table', width: 170 })),
      group('formatting', 'Formatting', field('format', 'Format', m.formatString ?? 'General', { width: 150 }), field('display-folder', 'Display folder', m.displayFolder ?? '', { width: 150 })),
      group('calculations', 'Calculations', off('new-measure', 'New measure', 'newMeasure'), off('quick-measure', 'Quick measure', 'quickMeasure')),
      group(
        'ai',
        'AI',
        button('ai-explain', 'Explain with AI', 'ai', {
          enabled: ctx.ai.ready && ctx.localModel,
          title: ctx.ai.ready ? (ctx.localModel ? `Explain ${m.name} with AI` : 'Needs a document with a local model') : (ctx.ai.reason ?? undefined),
          onclick: () => ctx.act.explainMeasure(m, t),
        }),
      ),
    ],
  }
}

function columnTools(ctx: RibbonContext, c: { name: string; dataType: string; formatString: string | null; summarizeBy?: string | null; dataCategory?: string | null; sortByColumn?: string | null; derived?: boolean }): RibbonTab {
  const groups: RibbonGroup[] = [
    group('structure', 'Structure', field('name', 'Name', c.name, { icon: 'tag', width: 160 }), field('data-type', 'Data type', dataTypeLabel(c.dataType), { width: 160 })),
    group('formatting', 'Formatting', field('format', 'Format', c.formatString ?? 'General', { width: 140 })),
    group('properties', 'Properties', field('summarization', 'Summarization', c.summarizeBy ?? 'Default', { width: 120 }), field('data-category', 'Data category', c.dataCategory ?? 'Uncategorized', { width: 120 })),
    group('sort', 'Sort', off('sort-by-column', 'Sort by column', 'sortByColumn')),
    group('groups', 'Groups', off('data-groups', 'Data groups', 'dataGroups')),
    group('relationships', 'Relationships', manageRelationships(ctx)),
    group('calculations', 'Calculations', off('new-column', 'New column', 'newColumn')),
  ]
  if (ctx.view === 'table') {
    const text = c.dataType === 'string' && !c.derived
    groups.push(
      group(
        'dashcraft-analyze',
        'DashCraft',
        button('column-stats', 'Column statistics', 'stats', { enabled: true, title: 'Rows, blanks, distinct values, min and max of this column', onclick: () => ctx.act.columnStats(c.name) }),
        button('parse-people', 'Parse people field', 'people', {
          enabled: text && ctx.hasBackend,
          title: text ? 'Add a session-only "(parsed)" column with the people as First Last; …' : 'Needs a text column',
          onclick: () => ctx.act.parsePeople(c.name),
        }),
      ),
    )
  }
  return { id: 'column-tools', label: 'Column tools', contextual: true, groups }
}

function tableTools(ctx: RibbonContext, t: TableDto): RibbonTab {
  const groups: RibbonGroup[] = [
    group('structure', 'Structure', field('name', 'Name', t.name, { icon: 'tag', width: 170 })),
    group('calendars', 'Calendars', off('mark-date-table', 'Mark as date table', 'markDateTable')),
    group('relationships', 'Relationships', manageRelationships(ctx)),
    group('calculations', 'Calculations', off('new-measure', 'New measure', 'newMeasure'), off('quick-measure', 'Quick measure', 'quickMeasure'), off('new-column', 'New column', 'newColumn'), off('new-table', 'New table', 'newTable')),
  ]
  if (ctx.view === 'table') {
    const remote = t.kind === 'remote'
    groups.push(
      group(
        'dashcraft-data',
        'DashCraft',
        button('refresh-table', 'Refresh table', 'refresh', {
          enabled: !remote && ctx.canRefresh && !ctx.refreshing,
          title: 'Refresh this table and what depends on it (offline)',
          onclick: () => ctx.act.refreshTable(t.name),
        }),
        button('load-csv', 'Load CSV…', 'csv', {
          enabled: ctx.tableCanLoadCsv && ctx.hasBackend && !ctx.refreshing,
          title: ctx.tableCanLoadCsv ? "Replace this table's data with a local CSV" : 'Calculated tables cannot load a CSV',
          onclick: () => ctx.act.loadCsv(t.name),
        }),
        stack(
          'table-mode',
          small('mode-rows', 'Rows', 'data', { enabled: true, pressed: ctx.tableMode === 'grid', title: 'Show the rows (data grid)', onclick: () => ctx.act.setTableMode('grid') }),
          small('mode-schema', 'Schema', 'column', { enabled: true, pressed: ctx.tableMode === 'schema', title: 'Show the columns: type, kind, source, format', onclick: () => ctx.act.setTableMode('schema') }),
        ),
      ),
    )
  }
  return { id: 'table-tools', label: 'Table tools', contextual: true, groups }
}

/** The ribbon tabs for the current view and selection (File is rendered by the ribbon). */
export function buildRibbon(ctx: RibbonContext): RibbonTab[] {
  const tabs: RibbonTab[] = [homeTab(ctx)]
  if (ctx.view === 'report') tabs.push(insertTab(), modelingTab(ctx), designTab(ctx), viewTab(ctx), optimizeTab())
  tabs.push(helpTab(ctx))
  const sel = ctx.selection
  if (ctx.view === 'report' && sel) {
    if (sel.kind === 'measure') tabs.push(measureTools(ctx, sel.measure, sel.table))
    else if (sel.kind === 'column') tabs.push(columnTools(ctx, sel.column))
    else tabs.push(tableTools(ctx, sel.table))
  }
  if (ctx.view === 'table' && ctx.table) {
    tabs.push(tableTools(ctx, ctx.table))
    if (ctx.column) {
      const model = ctx.table.columns.find((c) => c.name === ctx.column?.name)
      tabs.push(
        columnTools(ctx, {
          name: ctx.column.name,
          dataType: ctx.column.dataType,
          formatString: ctx.column.formatString,
          summarizeBy: model?.summarizeBy,
          dataCategory: model?.dataCategory,
          derived: ctx.column.derived,
        }),
      )
    }
  }
  return tabs
}
