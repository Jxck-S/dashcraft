// The UI's gateway to the Rust backend. Domain operations run through the command registry
// (`ops`, typed wrappers generated from `pbi-ops` into ops.gen.ts): the same commands as
// `dashcraft run`, `dashcraft serve` and agents. App-only commands (bindings.ts) cover the UI
// projections of a document with object ids (model tree, report outline, Properties / Format
// panes, TMDL and Model views), settings (blob map, File → Recent) and window plumbing.
//
// The backend workspace can hold several documents; the UI shows one at a time: the *current*
// document (`setCurrent`), whose `docId` the functions below pass to the registry. This module
// also adapts registry results to the view models the components use (grid cells, ids looked
// up in the model tree, canvas pages addressed by report and page ids).
// Degrades gracefully when the UI runs in a plain browser (`npm run dev`) without the backend.

import { getCurrentWebview } from '@tauri-apps/api/webview'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { open, save as saveDialog } from '@tauri-apps/plugin-dialog'
import {
  commands,
  events,
  type AppError,
  type AutoRecoveryDto,
  type BlobMapDto,
  type ColumnDto,
  type DocumentViewsDto,
  type ModelDiagramDto,
  type ModelTreeDto,
  type ObjectDetailsDto,
  type RecentFileDto,
  type ReportDto,
  type TmdlViewDto,
  type VisualFormatDto,
} from './bindings'
import type { LoadedDocument } from './model'
import {
  cancel,
  cancelLatest,
  hasBackend,
  NO_BACKEND,
  ops,
  type BookmarkOut,
  type CanvasAction,
  type CanvasBookmarkOut,
  type CanvasPageOut,
  type ContextMenuOut,
  type ExportVisualDataOut,
  type ColumnFilterOut,
  type ConvertPbipOut,
  type ConvertPbitOut,
  type DerivedColumnOut,
  type DocDiffOut,
  type DocumentEditOut,
  type DocumentHistoryOut,
  type DocumentSaveAsOut,
  type DocumentSaveOut,
  type DrillActionParam,
  type DrillthroughTargetOut,
  type EditOperation,
  type FileChangeOut,
  type FilterSpecOut,
  type MeasureValueOut,
  type ObjectDiffOut,
  type OpsErrorKind,
  type OpsProgress,
  type PeoplePreviewOut,
  type RangeBound,
  type RecoveryJournalOut,
  type RecoveryListOut,
  type ReplayFailureOut,
  type RefreshOut,
  type RelativeAnchorArg,
  type RelativePeriodArg,
  type ResultColumnOut,
  type SecurityRoleOut,
  type SecurityRolesOut,
  type SecurityViewAsOut,
  type SortOrder,
  type SourceParam,
  type TableRefreshOut,
  type TooltipLineOut,
  type TooltipPageOut,
  type ViewAsOut,
  type VisualDataMode,
} from './ops'
import type { RefreshEvent } from './data'
import type { DaxService } from './daxEditor'

export type * from './bindings'
export { hasBackend } from './ops'

/** Error kinds: the app commands' `AppError` kinds, the registry's codes, and `backend`. */
export type ErrorKind = AppError['kind'] | OpsErrorKind

export type Outcome<T> = { ok: true; value: T } | { ok: false; error: string; kind: ErrorKind }

type Result<T> = { status: 'ok'; data: T } | { status: 'error'; error: AppError }

/** Extensions offered in the Open dialog (detection itself is content-based in pbi-core). */
const POWER_BI_EXTENSIONS = ['pbix', 'pbit', 'pbip', 'pbir', 'pbism', 'bim', 'tmdl', 'pbids', 'rdl', 'abf', 'vpax', 'json']

export function describeError(error: AppError): string {
  switch (error.kind) {
    case 'notFound':
      return `File not found: ${error.message}`
    case 'permissionDenied':
      return `Permission denied: ${error.message}`
    case 'unsupported':
      return `Cannot open: ${error.message}`
    case 'invalidFile':
      return `File refused: ${error.message}`
    case 'invalidArgument':
      return `Invalid request: ${error.message}`
    case 'io':
      return `I/O error: ${error.message}`
    case 'noDocument':
      return 'No document is open.'
    case 'unknownObject':
      return `Object not found: ${error.message}`
    case 'busy':
      return `Busy: ${error.message}`
    case 'internal':
      return `Internal error: ${error.message}`
  }
}

async function call<T>(invoke: () => Promise<Result<T>>): Promise<Outcome<T>> {
  if (!hasBackend) return { ok: false, error: NO_BACKEND, kind: 'backend' }
  try {
    const result = await invoke()
    return result.status === 'ok'
      ? { ok: true, value: result.data }
      : { ok: false, error: describeError(result.error), kind: result.error.kind }
  } catch (e) {
    // Framework-level failures (ACL denial, bad arguments) reject with a string or Error.
    return { ok: false, error: String(e), kind: 'backend' }
  }
}

function map<T, U>(outcome: Outcome<T>, f: (value: T) => U): Outcome<U> {
  return outcome.ok ? { ok: true, value: f(outcome.value) } : outcome
}

const NO_DOCUMENT: Outcome<never> = { ok: false, error: 'No document is open.', kind: 'noDocument' }

// ---------------------------------------------------------------------------
// The current document
// ---------------------------------------------------------------------------

interface Current {
  docId: string
  tree: ModelTreeDto
  reports: ReportDto[]
}

let current: Current | null = null
/** Session derived columns of the current document (from the last `tableStatus`). */
let derived: DerivedColumnOut[] = []

/** Makes `doc` the document the functions below work on (`null`: none). */
export function setCurrent(doc: LoadedDocument | null) {
  const same = current !== null && current.docId === doc?.summary.docId
  if (current && !same) {
    // Evaluations of the previous document are obsolete.
    void cancelLatest('measures')
    void cancelLatest('dax')
  }
  current = doc ? { docId: doc.summary.docId, tree: doc.tree, reports: doc.reports } : null
  // The same document reloaded after an edit keeps its session derived columns.
  if (!same) derived = []
}

export async function appVersion(): Promise<string> {
  return hasBackend ? commands.appVersion() : 'dev'
}

/** Path passed on the command line (`dashcraft-app <path>`), if any. */
export async function startupPath(): Promise<string | null> {
  return hasBackend ? commands.startupPath() : null
}

/** The panes' data of an open document (`document_views`). */
export function loadViews(docId: string): Promise<Outcome<DocumentViewsDto>> {
  return call(() => commands.documentViews(docId))
}

/**
 * Opens a file or folder in the workspace (`doc.open`; no data loaded until `data.refresh`, which
 * the caller starts as `summary.onOpen` says) and returns everything the panes need. The caller
 * makes it current and closes the previous one.
 */
export async function openDocument(path: string): Promise<Outcome<LoadedDocument>> {
  const opened = await ops.docOpen({ path, autoRefresh: false, source: 'query' })
  if (!opened.ok) return opened
  const views = await loadViews(opened.value.docId)
  if (!views.ok) {
    void ops.docClose({ docId: opened.value.docId })
    return views
  }
  void call(() => commands.addRecentFile(path))
  return views
}

/** The newest document the backend already holds (survives a webview reload). */
export async function currentDocument(): Promise<LoadedDocument | null> {
  const listed = await ops.docList({})
  const last = listed.ok ? listed.value.documents.at(-1) : undefined
  if (!last) return null
  const views = await loadViews(last.docId)
  return views.ok ? views.value : null
}

// ---------------------------------------------------------------------------
// Auto recovery (unsaved edits journaled for recovery after a crash)
// ---------------------------------------------------------------------------

export type RecoveryJournal = RecoveryJournalOut
export type ReplayFailure = ReplayFailureOut

/** File → Options → Auto recovery. */
export function autoRecovery(): Promise<Outcome<AutoRecoveryDto>> {
  return call(() => commands.autoRecovery())
}

/** Turns Auto recovery on or off (off deletes the journals of open documents). */
export function setAutoRecovery(enabled: boolean): Promise<Outcome<AutoRecoveryDto>> {
  return call(() => commands.setAutoRecovery(enabled))
}

/** Unsaved edits left by a session that ended without saving (`doc.recovery.list`). */
export function recoverableEdits(): Promise<Outcome<RecoveryListOut>> {
  return ops.docRecoveryList({})
}

/**
 * Reopens the document of journal `recoveryId` with its unsaved edits (`doc.recovery.restore`)
 * and returns the panes' data; `failed` names the edit that could not be replayed (the document
 * keeps the edits before it). The caller makes it current and closes the previous one.
 */
export async function restoreRecovery(
  recoveryId: string,
): Promise<Outcome<{ views: LoadedDocument; failed: ReplayFailure | null; replayed: number; edits: number }>> {
  const restored = await ops.docRecoveryRestore({ recoveryId })
  if (!restored.ok) return restored
  const { docId, failed, replayed, edits } = restored.value
  const views = await loadViews(docId)
  if (!views.ok) {
    void ops.docClose({ docId, discardChanges: true })
    return views
  }
  const path = restored.value.document.path
  if (path) void call(() => commands.addRecentFile(path))
  return { ok: true, value: { views: views.value, failed: failed ?? null, replayed, edits } }
}

/** Deletes journal `recoveryId` and the unsaved edits it holds (`doc.recovery.discard`). */
export async function discardRecovery(recoveryId: string): Promise<Outcome<null>> {
  return map(await ops.docRecoveryDiscard({ recoveryId }), () => null)
}

// ---------------------------------------------------------------------------
// External changes (project files changed on disk by another program)
// ---------------------------------------------------------------------------

export type FileChange = FileChangeOut
export type DocumentDiff = DocDiffOut
export type ObjectDiff = ObjectDiffOut

/**
 * The project files of document `docId` (default: the current one) added, changed or removed on
 * disk since the open or last save (`doc.externalChanges`); `null` when the document does not
 * track them (only PBIP projects opened from a folder do).
 */
export async function externalChanges(docId: string | null = current?.docId ?? null): Promise<Outcome<FileChange[] | null>> {
  if (!docId) return NO_DOCUMENT
  return map(await ops.docExternalChanges({ docId }), (r) => (r.tracked ? r.changes : null))
}

/**
 * Rereads the current document from disk (`doc.reload`; same `docId`, a new revision) and returns
 * the panes' data. Unsaved edits are discarded only with `discard` (the caller asked first);
 * loaded data is dropped: start the refresh `summary.onOpen` says, as after an open.
 */
export async function reloadDocument(discard: boolean): Promise<Outcome<LoadedDocument>> {
  const docId = current?.docId
  if (!docId) return NO_DOCUMENT
  // `doc.reload` waits for the document's running mutating command: end a refresh first.
  if (refreshRequest && refreshDoc === docId) await cancel(refreshRequest)
  const reloaded = await ops.docReload({ docId, discardChanges: discard })
  if (!reloaded.ok) return reloaded
  derived = []
  return loadViews(docId)
}

/** The current document, unsaved edits included, compared object by object with its files on disk (`doc.diff`). */
export function diffWithDisk(): Promise<Outcome<DocumentDiff>> {
  const docId = current?.docId
  return docId ? ops.docDiff({ docId }) : Promise.resolve(NO_DOCUMENT)
}

/** The current document, unsaved edits included, compared object by object with another file or folder (`doc.diff`, `againstPath`). */
export function diffWithPath(path: string): Promise<Outcome<DocumentDiff>> {
  const docId = current?.docId
  return docId ? ops.docDiff({ docId, againstPath: path }) : Promise.resolve(NO_DOCUMENT)
}

/**
 * Closes document `docId` (default: the current one), cancelling its refresh and renders. A
 * document with unsaved edits closes only with `discard` (the caller asked first); else it fails
 * with `unsupported`.
 */
export async function closeDocument(docId: string | null = current?.docId ?? null, discard = false): Promise<Outcome<null>> {
  if (!docId) return { ok: true, value: null }
  if (docId === current?.docId) setCurrent(null)
  // `doc.close` waits for the document's running mutating command: end a refresh first.
  if (refreshRequest && refreshDoc === docId) await cancel(refreshRequest)
  return map(await ops.docClose({ docId, discardChanges: discard }), () => null)
}

/** Properties-pane details for one object of the current document. */
export function objectDetails(id: number): Promise<Outcome<ObjectDetailsDto>> {
  const doc = current?.docId
  return doc ? call(() => commands.objectDetails(doc, id)) : Promise.resolve(NO_DOCUMENT)
}

/** TMDL view: the model as TMDL scripts (verbatim files or generated). */
export function tmdlView(): Promise<Outcome<TmdlViewDto>> {
  const doc = current?.docId
  return doc ? call(() => commands.tmdlView(doc)) : Promise.resolve(NO_DOCUMENT)
}

/** Model view layouts from `diagramLayout.json` (`null` without one). */
export function modelDiagram(): Promise<Outcome<ModelDiagramDto | null>> {
  const doc = current?.docId
  return doc ? call(() => commands.modelDiagram(doc)) : Promise.resolve(NO_DOCUMENT)
}

/** Format pane: a visual's formatting objects as readable cards. */
export function visualFormat(id: number): Promise<Outcome<VisualFormatDto>> {
  const doc = current?.docId
  return doc ? call(() => commands.visualFormat(doc, id)) : Promise.resolve(NO_DOCUMENT)
}

// ---------------------------------------------------------------------------
// Data (refresh, grid, statistics, measures, transforms)
// ---------------------------------------------------------------------------

export type RefreshSource = SourceParam
export type RefreshReport = RefreshOut
export type TableRefresh = TableRefreshOut
export type FilterSpec = FilterSpecOut
export type ColumnFilter = ColumnFilterOut

/** Data status of a model table (`data.tables`), with its tree id and derived columns. */
export interface TableStatus {
  id: number | null
  name: string
  /** `regular`, `calculatedTable`, `calculationGroup`, `remote`. */
  kind: string
  hidden: boolean
  rows: number
  columns: number
  loaded: boolean
  canLoadCsv: boolean
  lastRefresh: TableRefreshOut | null
  /** Session-only derived columns of this table. */
  derivedColumns: string[]
}

const sameName = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: 'accent' }) === 0

function treeTable(name: string) {
  return current?.tree.tables.find((t) => sameName(t.name, name)) ?? null
}

/** The offline blob map used by `AzureStorage.Blobs` sources. */
export async function blobMap(): Promise<BlobMapDto | null> {
  const result = await call(() => commands.blobMap())
  return result.ok ? result.value : null
}

/** Uses the `blob-map.json` at `path` for the next refresh (`null` = offline). */
export function setBlobMap(path: string | null): Promise<Outcome<BlobMapDto>> {
  return call(() => commands.setBlobMap(path))
}

let refreshSeq = 0
let refreshRequest: string | null = null
let refreshDoc: string | null = null

/**
 * Refreshes every table (`null`) or the named tables (`data.refresh`); `onEvent` gets the
 * progress events. `query` evaluates the M queries (PBIX: embedded data); `cache` loads the PBIP
 * local data cache.
 */
export async function refresh(
  tables: string[] | null,
  source: RefreshSource = 'query',
  onEvent?: (event: RefreshEvent) => void,
): Promise<Outcome<RefreshReport>> {
  const doc = current?.docId
  if (!doc) return NO_DOCUMENT
  const requestId = `refresh-${++refreshSeq}`
  refreshRequest = requestId
  refreshDoc = doc
  const onProgress = (p: OpsProgress) => {
    const event = p.data as RefreshEvent | null
    if (event && typeof event === 'object' && 'event' in event) onEvent?.(event)
  }
  try {
    return await ops.dataRefresh({ docId: doc, tables, source }, { requestId, onProgress })
  } finally {
    if (refreshRequest === requestId) refreshRequest = null
  }
}

/** Cancels the running refresh; `false` if none is running. */
export async function cancelRefresh(): Promise<Outcome<boolean>> {
  return { ok: true, value: refreshRequest ? await cancel(refreshRequest) : false }
}

/** Data status of every model table (`data.tables`). */
export async function tableStatus(): Promise<Outcome<TableStatus[]>> {
  const doc = current?.docId
  if (!doc) return NO_DOCUMENT
  const result = await ops.dataTables({ docId: doc })
  if (!result.ok) return result
  derived = result.value.derived
  return {
    ok: true,
    value: result.value.tables.map((t) => ({
      id: treeTable(t.name)?.id ?? null,
      name: t.name,
      kind: t.kind,
      hidden: t.hidden,
      rows: t.rows,
      columns: t.columns,
      loaded: t.loaded,
      canLoadCsv: t.canLoadCsv,
      lastRefresh: t.lastRefresh ?? null,
      derivedColumns: derived.filter((d) => sameName(d.table, t.name)).map((d) => d.name),
    })),
  }
}

export type GridColumnKind = 'data' | 'calculated' | 'calculatedTableColumn' | 'rowNumber' | 'derived' | 'other'

/** A grid column: the result column plus what the model says about it. */
export interface GridColumn {
  name: string
  /** The source column, when it differs from the display name (or the parsed column). */
  sourceName: string | null
  dataType: string
  formatString: string | null
  kind: GridColumnKind
  /** Model object id (Properties pane), for model columns. */
  id: number | null
  numeric: boolean
  hidden: boolean
  description: string | null
  expression: string | null
}

/** A grid cell: formatted text, or BLANK. */
export interface Cell {
  text: string
  isBlank: boolean
}

export interface Sort {
  column: string
  ascending: boolean
}

export interface TablePage {
  table: string
  offset: number
  columns: GridColumn[]
  rows: Cell[][]
  totalRows: number
}

const NUMERIC_TYPES = /^(int64|double|decimal|u?int(8|16|32|64)|float(16|32|64)|decimal(128|256)\b.*)$/i

/** TMDL-like spelling of an Arrow type name (`Utf8` → `string`). */
function arrowTypeName(t: string): string {
  if (/^(large)?utf8(view)?$/i.test(t)) return 'string'
  if (/^boolean$/i.test(t)) return 'boolean'
  if (/^float/i.test(t)) return 'double'
  if (/^decimal/i.test(t)) return 'decimal'
  if (/^(timestamp|date)/i.test(t)) return 'dateTime'
  if (/^u?int/i.test(t)) return 'int64'
  return t
}

function derivedColumn(name: string, numeric: boolean, d: DerivedColumnOut): GridColumn {
  const [description, expression] =
    d.kind === 'people'
      ? ['Session-only column added by "Parse people field" (not saved).', d.sourceColumn ? `Parse people field ([${d.sourceColumn}])` : null]
      : d.kind === 'expression'
        ? ['Session-only DAX column (not saved).', d.dax ?? null]
        : ['Session-only column (not saved).', null]
  return { name, sourceName: d.sourceColumn ?? null, dataType: d.dataType, formatString: null, kind: 'derived', id: null, numeric, hidden: false, description, expression }
}

function modelColumn(c: ColumnDto, numeric: boolean): GridColumn {
  return {
    name: c.name,
    sourceName: c.sourceColumn,
    dataType: c.dataType,
    formatString: c.formatString,
    kind: c.kind,
    id: c.id,
    numeric,
    hidden: c.hidden,
    description: c.description,
    expression: c.expression,
  }
}

/** Grid columns of a result: model columns of `table` (or `Table[Column]` names), derived columns, others. */
function gridColumns(table: string | null, columns: ResultColumnOut[]): GridColumn[] {
  return columns.map((rc) => {
    const numeric = NUMERIC_TYPES.test(rc.dataType)
    let owner = table
    let name = rc.name
    if (owner == null) {
      const m = /^'?(.*?)'?\[(.*)\]$/.exec(rc.name)
      if (m) [owner, name] = [m[1], m[2]]
    }
    if (owner != null) {
      const d = derived.find((x) => sameName(x.table, owner) && sameName(x.name, name))
      if (d && table != null) return derivedColumn(rc.name, numeric, d)
      const c = treeTable(owner)?.columns.find((x) => sameName(x.name, name))
      if (c) return table != null ? modelColumn(c, numeric) : { ...modelColumn(c, numeric), name: rc.name, kind: 'other', sourceName: null, hidden: false, description: null, expression: null }
    }
    return { name: rc.name, sourceName: null, dataType: arrowTypeName(rc.dataType), formatString: rc.formatString ?? null, kind: 'other', id: null, numeric, hidden: false, description: null, expression: null }
  })
}

/** Cells of `formatted` + `blankAsNull` rows. */
function cells(rows: unknown[][]): Cell[][] {
  return rows.map((row) => row.map((v) => (v == null ? { text: '', isBlank: true } : { text: String(v), isBlank: false })))
}

/** One page of a table (≤ 1000 rows), sorted and filtered by a SQL expression on the backend. */
export async function tablePage(table: string, offset: number, limit: number, sort: Sort | null, filter: string | null): Promise<Outcome<TablePage>> {
  const doc = current?.docId
  if (!doc) return NO_DOCUMENT
  const result = await ops.dataPage({ docId: doc, table, offset, limit, sort, filter, formatted: true, blankAsNull: true })
  return map(result, (p) => ({ table: p.table, offset: p.offset, totalRows: p.totalRows, columns: gridColumns(p.table, p.columns), rows: cells(p.rows) }))
}

export interface ColumnStats {
  table: string
  column: string
  dataType: string
  rows: number
  count: number
  blanks: number
  distinct: number
  min: Cell
  max: Cell
}

export async function columnStats(table: string, column: string): Promise<Outcome<ColumnStats>> {
  const doc = current?.docId
  if (!doc) return NO_DOCUMENT
  const result = await ops.dataColumnStats({ docId: doc, table, column })
  return map(result, (s) => ({
    table: s.table,
    column: s.column,
    dataType: s.dataType ?? derived.find((d) => sameName(d.table, s.table) && sameName(d.name, s.column))?.dataType ?? '',
    rows: s.rows,
    count: s.count,
    blanks: s.blanks,
    distinct: s.distinct,
    min: { text: s.minFormatted, isBlank: s.min == null },
    max: { text: s.maxFormatted, isBlank: s.max == null },
  }))
}

/** A measure value with the tree id of the measure (Properties pane, Data pane tooltips). */
export type MeasureValue = MeasureValueOut & { id: number | null }

/** Every measure in a filter context (`null` = unfiltered). A newer call cancels a running one. */
export async function measures(filter: FilterSpec | null): Promise<Outcome<MeasureValue[]>> {
  const doc = current?.docId
  if (!doc) return NO_DOCUMENT
  const filters = (filter?.filters ?? []).map(({ table, column, values, exclude }) => ({ table, column, values, exclude }))
  const result = await ops.measuresValues({ docId: doc, filters }, { latest: 'measures' })
  return map(result, (out) =>
    out.measures.map((m) => ({
      ...m,
      formatted: m.error == null ? (m.formatted ?? '') : null,
      id: treeTable(m.table)?.measures.find((x) => sameName(x.name, m.name))?.id ?? null,
    })),
  )
}

/** Report and page ids of the current document → the names the registry takes. */
function names(reportId: number, pageId: number | null): { report: string; page: string | null } | null {
  const reports = current?.reports ?? []
  const index = reports.findIndex((r) => r.id === reportId)
  const report = reports[index]
  if (!report) return null
  // Report names are matched case-insensitively; a duplicate name is addressed by position.
  const unique = reports.filter((r) => sameName(r.name, report.name)).length === 1 && !/^\d+$/.test(report.name.trim())
  const page = pageId == null ? null : report.pages.find((p) => p.id === pageId)
  if (pageId != null && !page) return null
  return { report: unique ? report.name : String(index + 1), page: page?.name ?? null }
}

const NO_PAGE: Outcome<never> = { ok: false, error: 'The page is not in the open document.', kind: 'notFound' }

/** Report-level (plus page-level) filters as a filter spec for `measures`. */
export async function reportFilters(reportId: number, pageId: number | null): Promise<Outcome<FilterSpec>> {
  const doc = current?.docId
  const target = names(reportId, pageId)
  if (!doc) return NO_DOCUMENT
  if (!target) return NO_PAGE
  return map(await ops.reportFilters({ docId: doc, report: target.report, page: target.page }), (r) => r.filters)
}

export interface CsvOptions {
  /** One ASCII character; `\t` / `tab` for a tab. */
  delimiter: string
  hasHeader: boolean
  encoding: string | null
}

export interface CsvLoad {
  table: string
  /** Rows loaded into the table. */
  rows: number
  report: RefreshReport
}

/** "Load CSV manually…" into a table (`data.loadCsv`). */
export async function loadCsv(table: string, path: string, options: CsvOptions): Promise<Outcome<CsvLoad>> {
  const doc = current?.docId
  if (!doc) return NO_DOCUMENT
  const delimiter = ['\t', '\\t', 'tab', 'TAB'].includes(options.delimiter) ? '\t' : options.delimiter
  if ([...delimiter].length !== 1 || delimiter.charCodeAt(0) > 127 || /[\r\n]/.test(delimiter)) {
    return { ok: false, error: `Invalid request: the delimiter must be one ASCII character, not ${JSON.stringify(options.delimiter)}`, kind: 'invalidParams' }
  }
  if (!path.trim()) return { ok: false, error: 'Invalid request: path is empty', kind: 'invalidParams' }
  const encoding = options.encoding?.trim() || null
  const result = await ops.dataLoadCsv({ docId: doc, table, path, options: { delimiter, hasHeader: options.hasHeader, encoding } })
  return map(result, (report) => {
    const t = report.tables.find((x) => sameName(x.name, table))
    return { table: t?.name ?? table, rows: t?.rows ?? 0, report }
  })
}

export type PeoplePreview = PeoplePreviewOut

/** "Parse people field" preview of a text column (`text.previewPeople`). */
export function peoplePreview(table: string, column: string, limit: number): Promise<Outcome<PeoplePreview>> {
  const doc = current?.docId
  return doc ? ops.textPreviewPeople({ docId: doc, table, column, limit }) : Promise.resolve(NO_DOCUMENT)
}

/** Adds the session-only column `<column> (parsed)` (`data.addDerivedColumn`). */
export async function addPeopleColumn(table: string, column: string): Promise<Outcome<DerivedColumnOut>> {
  const doc = current?.docId
  if (!doc) return NO_DOCUMENT
  const result = await ops.dataAddDerivedColumn({ docId: doc, table, name: `${column} (parsed)`, kind: { people: { sourceColumn: column } } })
  if (result.ok) derived = [...derived, result.value]
  return result
}

export async function removeDerivedColumn(table: string, column: string): Promise<Outcome<DerivedColumnOut>> {
  const doc = current?.docId
  if (!doc) return NO_DOCUMENT
  const result = await ops.dataRemoveDerivedColumn({ docId: doc, table, name: column })
  if (result.ok) derived = derived.filter((d) => !(sameName(d.table, result.value.table) && sameName(d.name, result.value.name)))
  return result
}

// ---------------------------------------------------------------------------
// Row-level security: Modeling → View as (`security.*`)
// ---------------------------------------------------------------------------

export type SecurityRoles = SecurityRolesOut
export type SecurityRole = SecurityRoleOut
export type ViewAsState = ViewAsOut
export type ViewAsResult = SecurityViewAsOut

/** The model's roles and the current "View as" state (`security.roles`). */
export async function securityRoles(): Promise<Outcome<SecurityRoles>> {
  const doc = current?.docId
  return doc ? ops.securityRoles({ docId: doc }) : NO_DOCUMENT
}

/**
 * Views the data as `roles` and / or the user `userName` (`security.viewAs`): every query of the
 * document (canvas, grid, measures, DAX) then sees only the rows the roles allow. No roles and no
 * user stop viewing as.
 */
export async function viewAs(roles: string[], userName: string | null): Promise<Outcome<ViewAsResult>> {
  const doc = current?.docId
  if (!doc) return NO_DOCUMENT
  const user = userName?.trim() || null
  return ops.securityViewAs({ docId: doc, roles, userName: user })
}

// ---------------------------------------------------------------------------
// DAX query view
// ---------------------------------------------------------------------------

export interface DaxResultTable {
  columns: GridColumn[]
  rows: Cell[][]
  totalRows: number
  truncated: boolean
}

export interface DaxQueryResult {
  tables: DaxResultTable[]
  durationMs: number
}

/** Runs a DAX query over the loaded data (`dax.query`); a newer query cancels a running one. */
export async function runDaxQuery(query: string): Promise<Outcome<DaxQueryResult>> {
  const doc = current?.docId
  if (!doc) return NO_DOCUMENT
  const result = await ops.daxQuery({ docId: doc, query, maxRows: 1000, formatted: true, blankAsNull: true }, { latest: 'dax' })
  return map(result, (r) => ({
    durationMs: r.durationMs,
    tables: r.tables.map((t) => ({ columns: gridColumns(null, t.columns), rows: cells(t.rows), totalRows: t.totalRows, truncated: t.truncated })),
  }))
}

export async function cancelDaxQuery(): Promise<Outcome<boolean>> {
  return { ok: true, value: await cancelLatest('dax') }
}

// ---------------------------------------------------------------------------
// DAX language services of the formula bar (`dax.complete`, `dax.tokens`, …; M7.5)
// ---------------------------------------------------------------------------

/**
 * The `dax.*` language commands against the current document's model. A newer request of the
 * same kind cancels a running one; a failed or cancelled request answers `null`.
 */
export const daxService: DaxService = {
  async complete(text, offset) {
    const r = await ops.daxComplete({ docId: current?.docId, text, position: { utf16Offset: offset }, maxItems: 50 }, { latest: 'dax-complete' })
    return r.ok ? r.value : null
  },
  async signatureHelp(text, offset) {
    const r = await ops.daxSignatureHelp({ text, position: { utf16Offset: offset } }, { latest: 'dax-signature' })
    return r.ok ? (r.value.signature ?? null) : null
  },
  async hover(text, offset) {
    const r = await ops.daxHover({ docId: current?.docId, text, position: { utf16Offset: offset } }, { latest: 'dax-hover' })
    return r.ok ? (r.value.hover ?? null) : null
  },
  async diagnostics(text) {
    const r = await ops.daxDiagnostics({ docId: current?.docId, text, kind: 'expression' }, { latest: 'dax-diagnostics' })
    return r.ok ? r.value.diagnostics : null
  },
  async tokens(text) {
    const r = await ops.daxTokens({ docId: current?.docId, text }, { latest: 'dax-tokens' })
    return r.ok ? r.value.tokens : null
  },
  async format(text) {
    const r = await ops.daxFormat({ text, kind: 'expression' })
    if (!r.ok) return { ok: false, error: r.error }
    if (!r.value.ok) return { ok: false, error: r.value.diagnostic?.message ?? 'The expression has a syntax error.' }
    return { ok: true, text: r.value.text }
  },
}

// ---------------------------------------------------------------------------
// Interactive report canvas (`canvas.*`)
// ---------------------------------------------------------------------------

export type RenderedPage = CanvasPageOut
export type { BookmarkOut, CanvasAction, CanvasBookmarkOut, DrillthroughTargetOut }
export type TooltipLine = TooltipLineOut
export type TooltipPage = TooltipPageOut

export interface ClickResult {
  /** The re-rendered page, `null` when nothing changed. */
  page: RenderedPage | null
  action: CanvasAction | null
}

async function onPage<T>(reportId: number, pageId: number, run: (target: { docId: string; report: string; page: string }) => Promise<Outcome<T>>): Promise<Outcome<T>> {
  const doc = current?.docId
  const target = names(reportId, pageId)
  if (!doc) return NO_DOCUMENT
  if (!target?.page) return NO_PAGE
  return run({ docId: doc, report: target.report, page: target.page })
}

/** Renders a report page (SVG + hit regions) with its interaction state. */
export function renderPage(reportId: number, pageId: number): Promise<Outcome<RenderedPage>> {
  return onPage(reportId, pageId, (t) => ops.canvasRender(t))
}

/** A click on a hit region (`null`: the empty canvas); returns the re-rendered page and / or a button action. */
export function canvasClick(reportId: number, pageId: number, hitId: string | null, modifiers: { ctrl: boolean; shift: boolean }): Promise<Outcome<ClickResult>> {
  return onPage(reportId, pageId, async (t) =>
    map(await ops.canvasClick({ ...t, hitId, ctrl: modifiers.ctrl, shift: modifiers.shift, renderUnchanged: false }), (r) => ({ page: r.page, action: r.action ?? null })),
  )
}

/** Scrolls a table, matrix or slicer list (page units); `null` when it cannot scroll further. */
export function canvasScroll(reportId: number, pageId: number, visual: string, dx: number, dy: number): Promise<Outcome<RenderedPage | null>> {
  return onPage(reportId, pageId, async (t) => map(await ops.canvasScroll({ ...t, visual, dx, dy, renderUnchanged: false }), (r) => r.page))
}

export function toggleMatrixRow(reportId: number, pageId: number, hitId: string): Promise<Outcome<RenderedPage>> {
  return onPage(reportId, pageId, (t) => ops.canvasToggleMatrixRow({ ...t, hitId }))
}

/** Clears a slicer (or a visual's selected points); `null` clears the cross-filter. */
export function clearSelections(reportId: number, pageId: number, visual: string | null): Promise<Outcome<RenderedPage>> {
  return onPage(reportId, pageId, (t) => ops.canvasClearSelections({ ...t, visual }))
}

export function setSlicerSearch(reportId: number, pageId: number, visual: string, text: string): Promise<Outcome<RenderedPage>> {
  return onPage(reportId, pageId, (t) => ops.canvasSetSlicerSearch({ ...t, visual, text }))
}

/** Filters a range slicer to `[low, high]` (either `null`: open; both: clear). */
export function setRange(reportId: number, pageId: number, visual: string, low: RangeBound | null, high: RangeBound | null): Promise<Outcome<RenderedPage>> {
  return onPage(reportId, pageId, (t) => ops.canvasSetRange({ ...t, visual, low, high }))
}

/** Filters a relative date slicer ("Last 3 months"); `null` clears it. */
export function setRelativeDate(
  reportId: number,
  pageId: number,
  visual: string,
  spec: { anchor: RelativeAnchorArg; duration: number; period: RelativePeriodArg; includeToday: boolean } | null,
): Promise<Outcome<RenderedPage>> {
  return onPage(reportId, pageId, (t) => ops.canvasSetRelativeDate({ ...t, visual, ...(spec ?? {}) }))
}

/** Closes the open slicer dropdowns; `null` if none was open. */
export function closePopups(reportId: number, pageId: number): Promise<Outcome<RenderedPage | null>> {
  return onPage(reportId, pageId, async (t) => map(await ops.canvasClosePopups({ ...t, renderUnchanged: false }), (r) => r.page))
}

/** "Reset to default": the report's saved slicer selections, no cross-filter. */
export function resetPage(reportId: number, pageId: number): Promise<Outcome<RenderedPage>> {
  return onPage(reportId, pageId, (t) => ops.canvasReset(t))
}

export function visualTooltip(reportId: number, pageId: number, hitId: string): Promise<Outcome<{ lines: TooltipLine[] }>> {
  return onPage(reportId, pageId, (t) => ops.canvasTooltip({ ...t, hitId }))
}

/** The report-page tooltip of a data point, rendered for it; `null` when the visual has none. */
export function tooltipPage(reportId: number, pageId: number, hitId: string): Promise<Outcome<TooltipPage | null>> {
  return onPage(reportId, pageId, async (t) => map(await ops.canvasTooltipPage({ ...t, hitId }), (r) => r.tooltip ?? null))
}

/** The drillthrough pages a data point can drill through to. */
export function drillthroughTargets(reportId: number, pageId: number, hitId: string): Promise<Outcome<DrillthroughTargetOut[]>> {
  return onPage(reportId, pageId, async (t) => map(await ops.canvasDrillthroughTargets({ ...t, hitId }), (r) => r.targets))
}

/**
 * Drills through from a data point to page `target` (of report `targetReport` for a cross-report
 * drillthrough); returns the `drillthrough` action (the target page is filtered).
 */
export function drillthrough(reportId: number, pageId: number, hitId: string, target: string, targetReport: string | null = null): Promise<Outcome<CanvasAction>> {
  return onPage(reportId, pageId, async (t) =>
    map(
      await ops.canvasDrillthrough({ ...t, hitId, target, targetReport }),
      (r): CanvasAction => ({ kind: 'drillthrough', page: r.page, displayName: r.displayName, report: r.report, notApplied: r.notApplied }),
    ),
  )
}

/** Focus mode on `visual` (`showAsTable`: with a table of its data below it); `null` returns to the report. */
export function focusVisual(reportId: number, pageId: number, visual: string | null, showAsTable = false): Promise<Outcome<RenderedPage>> {
  return onPage(reportId, pageId, (t) => ops.canvasFocus({ ...t, visual, showAsTable }))
}

/** Spotlight on `visual` (the others dimmed); `null` turns it off. */
export function setSpotlight(reportId: number, pageId: number, visual: string | null): Promise<Outcome<RenderedPage>> {
  return onPage(reportId, pageId, (t) => ops.canvasSetSpotlight({ ...t, visual }))
}

/**
 * Sorts `visual` by `field` (query ref) and / or `direction`; neither restores the saved sort
 * (`canvas.sortVisual`).
 */
export function sortVisual(reportId: number, pageId: number, visual: string, field: string | null, direction: SortOrder | null): Promise<Outcome<RenderedPage>> {
  return onPage(reportId, pageId, (t) => ops.canvasSortVisual({ ...t, visual, field, direction }))
}

/** "Copy value" / "Copy selection" texts of a data point (`canvas.contextMenu`). */
export function contextMenu(reportId: number, pageId: number, hitId: string): Promise<Outcome<ContextMenuOut>> {
  return onPage(reportId, pageId, (t) => ops.canvasContextMenu({ ...t, hitId }))
}

/** "Export data": writes the summarized data or the underlying rows of `visual` as CSV (`export.visualData`). */
export function exportVisualData(reportId: number, pageId: number, visual: string, mode: VisualDataMode, path: string): Promise<Outcome<ExportVisualDataOut>> {
  // The save dialog already asked before replacing an existing file.
  return onPage(reportId, pageId, (t) => ops.exportVisualData({ ...t, visual, mode, path, formatted: true, excelSafe: true, overwrite: true }))
}

/** Id of the report named `name` in the open document. */
export function reportIdOf(name: string): number | null {
  return current?.reports.find((r) => r.name === name)?.id ?? null
}

/** Drills `visual` up, down on data point `hitId`, to the next level or expands it one level (`canvas.drill`). */
export function drillVisual(reportId: number, pageId: number, visual: string, action: DrillActionParam, hitId: string | null = null): Promise<Outcome<RenderedPage>> {
  return onPage(reportId, pageId, (t) => ops.canvasDrill({ ...t, visual, action, hitId }))
}

/** The bookmarks of a report in the Bookmarks pane's order (`report.bookmarks`). */
export async function reportBookmarks(reportId: number): Promise<Outcome<BookmarkOut[]>> {
  const doc = current?.docId
  const target = names(reportId, null)
  if (!doc) return NO_DOCUMENT
  if (!target) return NO_PAGE
  return map(await ops.reportBookmarks({ docId: doc, report: target.report }), (r) => r.bookmarks)
}

/**
 * Applies bookmark `bookmark` (internal name) while page `pageId` is shown
 * (`canvas.applyBookmark`); the result renders the page the bookmark shows.
 */
export function applyBookmark(reportId: number, pageId: number, bookmark: string): Promise<Outcome<CanvasBookmarkOut>> {
  return onPage(reportId, pageId, (t) => ops.canvasApplyBookmark({ ...t, bookmark }))
}

/** The page id of `page` (internal name) in report `reportId` of the current document. */
export function pageIdOf(reportId: number, page: string): number | null {
  return current?.reports.find((r) => r.id === reportId)?.pages.find((p) => p.name === page)?.id ?? null
}

// ---------------------------------------------------------------------------
// Export (File → Export)
// ---------------------------------------------------------------------------

/** Writes the current document as a PBIP project into an empty folder (`convert.pbip`). */
export async function exportPbip(outDir: string): Promise<Outcome<ConvertPbipOut>> {
  const doc = current?.docId
  return doc ? ops.convertPbip({ docId: doc, outDir }) : NO_DOCUMENT
}

/** Writes a PBIT template of one report of the current document (`convert.pbit`). */
export async function exportPbit(outPath: string, reportId: number | null): Promise<Outcome<ConvertPbitOut>> {
  const doc = current?.docId
  if (!doc) return NO_DOCUMENT
  const report = reportId == null ? null : (current?.reports.find((r) => r.id === reportId)?.name ?? null)
  // The save dialog already asked before replacing an existing file.
  return ops.convertPbit({ docId: doc, outPath, report, overwrite: true })
}

/** Native "Save" dialog for one file with the given extensions. `null` when cancelled. */
export async function pickSavePath(title: string, defaultPath: string | undefined, filters: Array<{ name: string; extensions: string[] }>): Promise<string | null> {
  if (!hasBackend) return null
  const selected = await saveDialog({ title, defaultPath, filters })
  return typeof selected === 'string' ? selected : null
}

/** Native folder picker. `null` when cancelled. */
export async function pickFolder(title: string): Promise<string | null> {
  if (!hasBackend) return null
  const selected = await open({ directory: true, multiple: false, title })
  return typeof selected === 'string' ? selected : null
}

// ---------------------------------------------------------------------------
// Editing (formula bar, Properties pane, Quick Access Toolbar, File → Save / Save as)
// ---------------------------------------------------------------------------

export type DocumentHistory = DocumentHistoryOut
export type DocumentEdit = DocumentEditOut
export type DocumentSave = DocumentSaveOut
export type DocumentSaveAs = DocumentSaveAsOut
export type Edit = EditOperation

/** Undo / redo state and unsaved edits of the current document (`doc.history`). */
export async function history(): Promise<Outcome<DocumentHistoryOut>> {
  const doc = current?.docId
  return doc ? ops.docHistory({ docId: doc }) : NO_DOCUMENT
}

/** Undoes the newest edit step (`doc.undo`). */
export async function undo(): Promise<Outcome<DocumentEditOut>> {
  const doc = current?.docId
  return doc ? ops.docUndo({ docId: doc }) : NO_DOCUMENT
}

/** Redoes the newest undone step (`doc.redo`). */
export async function redo(): Promise<Outcome<DocumentEditOut>> {
  const doc = current?.docId
  return doc ? ops.docRedo({ docId: doc }) : NO_DOCUMENT
}

/** Applies `edits` atomically as one undo step named `label` (`doc.applyEdits`). */
export async function applyEdits(edits: EditOperation[], label: string): Promise<Outcome<DocumentEditOut>> {
  const doc = current?.docId
  return doc ? ops.docApplyEdits({ docId: doc, edits, label }) : NO_DOCUMENT
}

/**
 * Saves the edits into the opened PBIP project (`doc.save`). Fails with `conflict` when project
 * files changed on disk since the open or last save (retry with `overwrite` after asking), and
 * with `unsupported` for any other format (use `saveAs`).
 */
export async function save(overwrite = false): Promise<Outcome<DocumentSaveOut>> {
  const doc = current?.docId
  return doc ? ops.docSave({ docId: doc, overwrite }) : NO_DOCUMENT
}

/**
 * Saves the document as a new PBIP project in `outDir` (`doc.saveAs`), which it then is: the next
 * Save writes there. Fails with `io` for a non-empty folder unless `overwrite`. The project is
 * added to File → Recent.
 */
export async function saveAs(outDir: string, overwrite = false): Promise<Outcome<DocumentSaveAsOut>> {
  const doc = current?.docId
  if (!doc) return NO_DOCUMENT
  const saved = await ops.docSaveAs({ docId: doc, outDir, overwrite })
  if (saved.ok) {
    // `pbipFile` is relative to the project folder.
    const pbip = `${saved.value.projectDir.replace(/[\\/]+$/, '')}/${saved.value.pbipFile}`
    void call(() => commands.addRecentFile(pbip))
  }
  return saved
}

/**
 * Runs `confirm` when the window is asked to close (title-bar ✕, Alt+F4): it closes only when
 * `confirm` resolves `true` (e.g. no unsaved edits, or the user chose Save or Don't save).
 * Returns an unsubscribe function.
 */
export async function onCloseRequested(confirm: () => Promise<boolean>): Promise<() => void> {
  if (!hasBackend) return () => {}
  return getCurrentWindow().onCloseRequested(async (event) => {
    if (!(await confirm())) event.preventDefault()
  })
}

// ---------------------------------------------------------------------------
// File → Recent, window, dialogs, drag and drop
// ---------------------------------------------------------------------------

/** File → Recent (newest first). Empty without a backend. */
export async function recentFiles(): Promise<RecentFileDto[]> {
  const result = await call(() => commands.recentFiles())
  return result.ok ? result.value : []
}

/** Removes one recent entry (`null`: clears the list); returns the new list. */
export async function removeRecentFile(path: string | null): Promise<RecentFileDto[]> {
  const result = await call(() => commands.removeRecentFile(path))
  return result.ok ? result.value : []
}

/** Native window title (`<file> - DashCraft`); also the document title for the browser preview. */
export async function setWindowTitle(title: string): Promise<void> {
  document.title = title
  if (!hasBackend) return
  try {
    await getCurrentWindow().setTitle(title)
  } catch {
    // Missing permission or no window: the in-app title bar still shows it.
  }
}

/** Subscribes to `open-file-requested` events (macOS Finder opens while running). Returns an unsubscribe function. */
export async function onOpenFileRequested(handler: (path: string) => void): Promise<() => void> {
  if (!hasBackend) return () => {}
  return events.openFileRequested.listen((event) => handler(event.payload.path))
}

/** Native "Open" dialog for one file with the given extensions. `null` when cancelled. */
export async function pickFile(title: string, filters: Array<{ name: string; extensions: string[] }>): Promise<string | null> {
  if (!hasBackend) return null
  const selected = await open({ directory: false, multiple: false, title, filters: [...filters, { name: 'All files', extensions: ['*'] }] })
  return typeof selected === 'string' ? selected : null
}

/** Native "Open" dialog. Returns `null` when cancelled or without a backend. */
export async function pickPath(kind: 'file' | 'folder', title?: string): Promise<string | null> {
  if (!hasBackend) return null
  const selected = await open(
    kind === 'folder'
      ? { directory: true, multiple: false, title: title ?? 'Open PBIP / TMDL folder' }
      : {
          directory: false,
          multiple: false,
          title: title ?? 'Open Power BI file',
          filters: [
            { name: 'Power BI files', extensions: POWER_BI_EXTENSIONS },
            { name: 'All files', extensions: ['*'] },
          ],
        },
  )
  return typeof selected === 'string' ? selected : null
}

export interface DragDropHandlers {
  /** `true` while files hover over the window, `false` on leave/drop. */
  hover: (active: boolean) => void
  drop: (paths: string[]) => void
}

/**
 * Subscribes to native file drag-and-drop on the current webview (Tauri 2 delivers real
 * filesystem paths here; the HTML5 drop event does not). Returns an unsubscribe function.
 */
export async function onFileDrop(handlers: DragDropHandlers): Promise<() => void> {
  if (!hasBackend) return () => {}
  return getCurrentWebview().onDragDropEvent((event) => {
    switch (event.payload.type) {
      case 'enter':
      case 'over':
        handlers.hover(true)
        break
      case 'drop':
        handlers.hover(false)
        handlers.drop(event.payload.paths)
        break
      case 'leave':
        handlers.hover(false)
        break
    }
  })
}
