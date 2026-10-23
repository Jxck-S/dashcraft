// UI helpers over the IPC DTOs generated from Rust (bindings.ts): selection, search filtering,
// labels and tooltips. No data lives here: everything comes from the backend.

import type {
  CodeLanguage,
  ColumnDto,
  CrossFilterDirection,
  DetectionDto,
  DiagnosticDto,
  DocumentSummary,
  FieldDto,
  HierarchyDto,
  MeasureDto,
  ModelTreeDto,
  RelationshipCardinality,
  ReportDto,
  TableDto,
} from './bindings'

/** A loaded document: the summary from `open_document` plus every pane's data. */
export interface LoadedDocument {
  summary: DocumentSummary
  tree: ModelTreeDto
  reports: ReportDto[]
  diagnostics: DiagnosticDto[]
  detection: DetectionDto
}

/** Selected object: its backend ID (document-unique) for `object_details`. */
export type SelectedId = number | null

/** A field (column, measure or hierarchy) as listed under a table in the Data pane. */
export type TreeField =
  | { kind: 'column'; item: ColumnDto }
  | { kind: 'measure'; item: MeasureDto }
  | { kind: 'hierarchy'; item: HierarchyDto }

export interface TreeTable {
  table: TableDto
  fields: TreeField[]
  /** The table itself matches the search (all its fields are shown). */
  tableMatches: boolean
}

const byName = (a: TreeField, b: TreeField) =>
  a.item.name.localeCompare(b.item.name, undefined, { sensitivity: 'base', numeric: true })

/**
 * Data-pane rows: tables with their measures, columns and hierarchies sorted by name (as Power
 * BI Desktop does), filtered by `query` (table, field or source-column names, case-insensitive).
 */
export function treeTables(tree: ModelTreeDto, query: string, showHidden: boolean): TreeTable[] {
  const q = query.trim().toLowerCase()
  const hit = (...names: Array<string | null | undefined>) => names.some((n) => n?.toLowerCase().includes(q))
  return tree.tables
    .filter((t) => showHidden || !t.hidden)
    .map((table) => {
      const tableMatches = !q || hit(table.name)
      const fields: TreeField[] = [
        ...table.measures.map((item): TreeField => ({ kind: 'measure', item })),
        ...table.columns.filter((c) => c.kind !== 'rowNumber').map((item): TreeField => ({ kind: 'column', item })),
        ...table.hierarchies.map((item): TreeField => ({ kind: 'hierarchy', item })),
      ]
        .filter((f) => showHidden || !f.item.hidden)
        .filter((f) => tableMatches || hit(f.item.name, f.kind === 'column' ? f.item.sourceColumn : null))
        .sort(byName)
      return { table, fields, tableMatches }
    })
    .filter((t) => t.tableMatches || t.fields.length > 0)
}

/** Tooltip text: the source name when it differs from the display name, then extra lines. */
export function nameTooltip(name: string, source: string | null | undefined, ...extra: Array<string | null | undefined>): string {
  const lines: string[] = []
  if (source && source !== name) lines.push(`Source: ${source}`)
  for (const e of extra) if (e) lines.push(e)
  return lines.join('\n')
}

/** Tooltip of a projected field: display name, model reference and role. */
export function fieldTooltip(f: FieldDto): string {
  return [`${f.role}: ${f.displayName}`, `Source: ${f.sourceName}`, f.aggregation ? `Aggregation: ${f.aggregation}` : '']
    .filter(Boolean)
    .join('\n')
}

/** Table that owns an object (table itself, column, measure, partition or hierarchy). */
export function ownerTables(tree: ModelTreeDto): Map<number, TableDto> {
  const owners = new Map<number, TableDto>()
  for (const t of tree.tables) {
    owners.set(t.id, t)
    for (const c of t.columns) owners.set(c.id, t)
    for (const m of t.measures) owners.set(m.id, t)
    for (const p of t.partitions) owners.set(p.id, t)
    for (const h of t.hierarchies) owners.set(h.id, t)
  }
  return owners
}

export const CARDINALITY: Record<RelationshipCardinality, { short: string; long: string }> = {
  manyToOne: { short: '*:1', long: 'Many to one' },
  oneToMany: { short: '1:*', long: 'One to many' },
  oneToOne: { short: '1:1', long: 'One to one' },
  manyToMany: { short: '*:*', long: 'Many to many' },
  unknown: { short: '?', long: 'Unknown cardinality' },
}

export const CROSS_FILTER: Record<CrossFilterDirection, string> = {
  single: 'Single direction',
  both: 'Both directions',
  automatic: 'Automatic',
  unknown: 'Unknown direction',
}

/** DAX table reference: `Orders`, `'Web Sessions'`. */
export function daxTableRef(table: string): string {
  return /^[A-Za-z_][A-Za-z0-9_]*$/.test(table) ? table : `'${table.replaceAll("'", "''")}'`
}

/** DAX column reference: `Orders[Status]`, `'Web Sessions'[Device]`. */
export function daxColumnRef(table: string, column: string): string {
  return `${daxTableRef(table)}[${column.replaceAll(']', ']]')}]`
}

/** An entry of the Queries view: an M query, parameter or DAX expression with its code. */
export interface QueryItem {
  /** Object to select (partition, expression, measure, column or table). */
  id: number
  name: string
  section: 'M queries' | 'Parameters' | 'Measures (DAX)' | 'Calculated columns (DAX)' | 'Calculated tables (DAX)' | 'Other'
  subtitle: string
  language: CodeLanguage
  code: string
}

export const QUERY_SECTIONS: ReadonlyArray<QueryItem['section']> = [
  'M queries',
  'Parameters',
  'Measures (DAX)',
  'Calculated columns (DAX)',
  'Calculated tables (DAX)',
  'Other',
]

export function queryItems(tree: ModelTreeDto): QueryItem[] {
  const items: QueryItem[] = []
  for (const e of tree.expressions) {
    items.push({
      id: e.id,
      name: e.name,
      section: e.isParameter ? 'Parameters' : e.language === 'm' ? 'M queries' : 'Other',
      subtitle: e.isParameter
        ? `Parameter${e.parameterType ? ` · ${e.parameterType}` : ''}`
        : `Shared ${e.language === 'm' ? 'M' : e.language.toUpperCase()} expression`,
      language: e.language,
      code: e.expression,
    })
  }
  for (const t of tree.tables) {
    for (const p of t.partitions) {
      if (p.kind === 'calculated') continue
      items.push({
        id: p.id,
        name: t.partitions.length > 1 ? `${t.name} · ${p.name}` : t.name,
        section: p.kind === 'm' ? 'M queries' : 'Other',
        subtitle: `${p.kind === 'm' ? 'M partition' : `${p.sourceType} partition`} of ${t.name} · ${p.mode}`,
        language: p.kind === 'm' ? 'm' : 'text',
        code: p.expression ?? '',
      })
    }
    if (t.expression != null) {
      items.push({
        id: t.id,
        name: t.name,
        section: 'Calculated tables (DAX)',
        subtitle: 'Calculated table',
        language: 'dax',
        code: t.expression,
      })
    }
    for (const m of t.measures) {
      items.push({
        id: m.id,
        name: m.name,
        section: 'Measures (DAX)',
        subtitle: `Measure in ${t.name}${m.formatString ? ` · format ${m.formatString}` : ''}`,
        language: 'dax',
        code: m.expression,
      })
    }
    for (const c of t.columns) {
      if (c.kind !== 'calculated' || c.expression == null) continue
      items.push({
        id: c.id,
        name: `${t.name}[${c.name}]`,
        section: 'Calculated columns (DAX)',
        subtitle: `Calculated column · ${c.dataType}`,
        language: 'dax',
        code: c.expression,
      })
    }
  }
  return items
}
