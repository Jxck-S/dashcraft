// UI helpers over the registry's data results (refresh status, grid columns, filters). No data
// lives here.

import type { ColumnFilter, GridColumn, RefreshReport, TableStatus } from './backend'
import { daxColumnRef } from './model'
import type { DiagnosticOut } from './ops'

/** Data state of a table, for chips and icons. */
export type TableState = 'notLoaded' | 'ok' | 'partial' | 'error' | 'skipped'

/**
 * Outcome of a refresh step: the registry's `StatusOut` (`data.refresh` results) or the
 * `pbi-session` status of a progress event (`reason` instead of `message` for skipped steps).
 */
export interface Status {
  /** `ok`, `error` or `skipped`. */
  state: string
  message?: string | null
  reason?: string | null
  unsupported?: boolean | null
  hint?: string | null
  csvFallback?: boolean | null
  /** Blob access is offline (no blob map). */
  offline?: boolean | null
}

/** One refresh step of a table, from a result or a progress event. */
export interface TableRefreshLike {
  name: string
  /** `m`, `calculated`, `pbix`, `cache`, `manual` or `skipped`. */
  kind: string
  rows: number
  durationMs: number
  status: Status
  columns?: Array<{ name: string; durationMs: number; status: Status }>
}

/** A `data.refresh` progress event (`OpsProgress.data`): the `pbi-session` refresh event. */
export type RefreshEvent =
  | { event: 'started'; tables: string[] }
  | { event: 'tableStarted'; name: string; kind: string }
  | { event: 'tableFinished'; table: TableRefreshLike }
  | { event: 'columnStarted'; table: string; column: string }
  | { event: 'columnFinished'; table: string; column: { name: string; durationMs: number; status: Status } }
  | { event: 'finished'; durationMs: number; ok: number; failed: number; skipped: number }

/** What to do about an offline blob source, in the app's words (as in the progress events). */
export const OFFLINE_HINT = 'offline: configure a blob map or Load CSV manually…'

export const REFRESH_KIND_LABEL: Record<string, string> = {
  m: 'Power Query (M)',
  calculated: 'Calculated (DAX)',
  pbix: 'Embedded data (PBIX)',
  cache: 'Local data cache (cache.abf)',
  manual: 'Loaded from CSV',
  skipped: 'Skipped',
}

export function kindLabel(kind: string): string {
  return REFRESH_KIND_LABEL[kind] ?? kind
}

/** `ok` with failed calculated columns is `partial`. */
export function refreshState(r: TableRefreshLike): TableState {
  if (r.status.state === 'error') return 'error'
  if (r.status.state === 'skipped') return 'skipped'
  return (r.columns ?? []).some((c) => c.status.state !== 'ok') ? 'partial' : 'ok'
}

/** The last refresh result wins: a failed refresh that kept earlier data still shows the error. */
export function tableState(t: TableStatus): TableState {
  if (t.lastRefresh) return refreshState(t.lastRefresh)
  return t.loaded ? 'ok' : 'notLoaded'
}

/** Error message or skip reason. */
export function statusMessage(s: Status): string | null {
  return s.state === 'ok' ? null : (s.message ?? s.reason ?? null)
}

/** What to do about a failed step. */
export function statusHint(s: Status): string | null {
  if (s.state !== 'error') return null
  return s.offline ? OFFLINE_HINT : (s.hint ?? null)
}

/** The status offers "Load CSV manually…". */
export function csvFallback(s: Status): boolean {
  return s.state === 'error' && !!s.csvFallback
}

/** `3,900`, `12.3K`, `1.2M`. */
export function compactCount(n: number): string {
  if (n < 10_000) return n.toLocaleString()
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 100_000 ? 1 : 0)}K`
  return `${(n / 1_000_000).toFixed(1)}M`
}

/** Tooltip of a table status chip. */
export function tableTooltip(t: TableStatus): string {
  const lines = [`${t.name}: ${t.loaded ? `${t.rows.toLocaleString()} rows, ${t.columns} columns` : 'no data loaded'}`]
  const r = t.lastRefresh
  if (r) {
    lines.push(`Last refresh: ${kindLabel(r.kind)}, ${r.durationMs.toLocaleString()} ms`)
    const message = statusMessage(r.status)
    if (message) lines.push(`${r.status.state === 'error' ? 'Error' : 'Skipped'}: ${message}`)
    const hint = statusHint(r.status)
    if (hint) lines.push(`Hint: ${hint}`)
    for (const c of r.columns) {
      const m = statusMessage(c.status)
      if (m) lines.push(`Column [${c.name}]: ${m}`)
    }
  } else if (!t.loaded) {
    lines.push('Refresh to load the data.')
  }
  if (t.derivedColumns.length > 0) lines.push(`Derived (session only): ${t.derivedColumns.join(', ')}`)
  return lines.join('\n')
}

/** SQL identifier for the grid filter: `"Store Name"`. */
export function quoteIdent(name: string): string {
  return `"${name.replaceAll('"', '""')}"`
}

const GRID_KIND_LABEL: Record<GridColumn['kind'], string> = {
  data: 'Column',
  calculated: 'Calculated column (DAX)',
  calculatedTableColumn: 'Calculated table column',
  rowNumber: 'Row number',
  derived: 'Derived column (session only)',
  other: 'Column (not in the model)',
}

/** Grid header tooltip: kind, source name, data type, format. */
export function columnTooltip(c: GridColumn): string {
  return [
    `${c.name} — ${GRID_KIND_LABEL[c.kind]}`,
    c.sourceName && c.sourceName !== c.name ? `Source: ${c.sourceName}` : null,
    `Type: ${c.dataType}`,
    c.formatString ? `Format: ${c.formatString}` : null,
    c.expression ? `Expression: ${c.expression}` : null,
    c.description,
    'Click to sort · right-click for more',
  ]
    .filter(Boolean)
    .join('\n')
}

/** `Stores[Channel] IN {"Retail"}`. */
export function filterSummary(f: ColumnFilter): string {
  const values = f.values.map((v) => (v == null ? 'BLANK' : typeof v === 'string' ? `"${v}"` : String(v)))
  return `${daxColumnRef(f.table, f.column)} ${f.exclude ? 'NOT IN' : 'IN'} {${values.join(', ')}}`
}

/** Display text of a column type (`int64` → `Whole number`). */
export function dataTypeLabel(dataType: string): string {
  const labels: Record<string, string> = {
    string: 'Text',
    int64: 'Whole number',
    double: 'Decimal number',
    decimal: 'Fixed decimal',
    dateTime: 'Date/time',
    boolean: 'True/false',
  }
  return labels[dataType] ?? dataType
}

// ---------------------------------------------------------------------------
// Refresh runs (progress panel)
// ---------------------------------------------------------------------------

export type StepState = 'pending' | 'running' | TableState

export interface RefreshEntry {
  name: string
  state: StepState
  /** `m`, `calculated`, `pbix`, `cache`, `manual` or `skipped`. */
  kind: string | null
  rows: number | null
  durationMs: number | null
  /** Error message or skip reason. */
  message: string | null
  hint: string | null
  csvFallback: boolean
  /** Calculated column being computed. */
  current: string | null
  columnsDone: number
  columnErrors: Array<{ name: string; message: string }>
}

export interface RefreshRun {
  /** The document (`docId`) the run belongs to. */
  docId: string
  /** Requested tables (`null` = all). */
  scope: string[] | null
  outcome: 'running' | 'ok' | 'failed' | 'cancelled' | 'error'
  cancelling: boolean
  entries: RefreshEntry[]
  durationMs: number | null
  diagnostics: DiagnosticOut[]
  /** Why the refresh call itself failed. */
  error: string | null
}

export function newRun(docId: string, scope: string[] | null): RefreshRun {
  return {
    docId,
    scope,
    outcome: 'running',
    cancelling: false,
    entries: (scope ?? []).map((name) => entry(name)),
    durationMs: null,
    diagnostics: [],
    error: null,
  }
}

function entry(name: string): RefreshEntry {
  return {
    name,
    state: 'pending',
    kind: null,
    rows: null,
    durationMs: null,
    message: null,
    hint: null,
    csvFallback: false,
    current: null,
    columnsDone: 0,
    columnErrors: [],
  }
}

function update(run: RefreshRun, name: string, f: (e: RefreshEntry) => RefreshEntry): RefreshRun {
  const exists = run.entries.some((e) => e.name === name)
  const entries = exists ? run.entries.map((e) => (e.name === name ? f(e) : e)) : [...run.entries, f(entry(name))]
  return { ...run, entries }
}

function finished(e: RefreshEntry, t: TableRefreshLike): RefreshEntry {
  const columns = t.columns ?? []
  return {
    ...e,
    state: refreshState(t),
    kind: t.kind,
    rows: t.rows,
    durationMs: t.durationMs,
    message: statusMessage(t.status),
    hint: statusHint(t.status),
    csvFallback: csvFallback(t.status),
    current: null,
    columnsDone: Math.max(e.columnsDone, columns.length),
    columnErrors: columns.flatMap((c) => {
      const message = statusMessage(c.status)
      return message ? [{ name: c.name, message }] : []
    }),
  }
}

/** Applies a `data.refresh` progress event. */
export function applyProgress(run: RefreshRun, event: RefreshEvent): RefreshRun {
  switch (event.event) {
    case 'started':
      return { ...run, entries: event.tables.map((name) => run.entries.find((e) => e.name === name) ?? entry(name)) }
    case 'tableStarted':
      return update(run, event.name, (e) => ({ ...e, state: 'running', kind: event.kind }))
    case 'tableFinished':
      return update(run, event.table.name, (e) => finished(e, event.table))
    case 'columnStarted':
      return update(run, event.table, (e) => ({
        ...e,
        state: e.state === 'error' || e.state === 'skipped' ? e.state : 'running',
        current: event.column,
      }))
    case 'columnFinished': {
      const message = statusMessage(event.column.status)
      return update(run, event.table, (e) => {
        const columnErrors = message ? [...e.columnErrors, { name: event.column.name, message }] : e.columnErrors
        const state: StepState = e.state !== 'running' ? e.state : columnErrors.length > 0 ? 'partial' : 'ok'
        return { ...e, state, current: null, columnsDone: e.columnsDone + 1, columnErrors }
      })
    }
    case 'finished':
      return { ...run, durationMs: event.durationMs }
  }
}

/** Merges final step results (the `data.refresh` / `data.loadCsv` report) into the run. */
export function applyReport(run: RefreshRun, report: RefreshReport, final = true): RefreshRun {
  let next = run
  for (const t of report.tables) next = update(next, t.name, (e) => finished(e, t))
  if (!final) {
    const failing = next.entries.some((e) => e.state === 'error' || e.state === 'partial' || e.state === 'skipped')
    return next.outcome === 'failed' && !failing ? { ...next, outcome: 'ok' } : next
  }
  return {
    ...next,
    outcome: report.ok ? 'ok' : 'failed',
    cancelling: false,
    durationMs: report.durationMs,
    diagnostics: report.diagnostics,
  }
}

/** Ends a run that did not complete: unfinished steps go back to pending (their data is unchanged). */
export function stopRun(run: RefreshRun, outcome: 'cancelled' | 'error', error: string | null = null): RefreshRun {
  return {
    ...run,
    outcome,
    cancelling: false,
    error,
    entries: run.entries.map((e) => (e.state === 'running' ? { ...e, state: 'pending', current: null } : e)),
  }
}

/** Steps done (not pending or running). */
export function runProgress(run: RefreshRun): { done: number; total: number } {
  const done = run.entries.filter((e) => e.state !== 'pending' && e.state !== 'running').length
  return { done, total: run.entries.length }
}
