/* eslint-disable */
// Generated from the app's command registry (pbi-ops + pbi-ai, `pbi_ai::catalog()`) by
// `npm run gen:ops` (crates/pbi-ai/tests/registry.rs, `ops_typescript_is_up_to_date`).
// Do not edit: change the Rust command types and regenerate. See docs/COMMANDS.md and
// docs/AI.md.

/** Any JSON value (`serde_json::Value`). */
export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue }

export interface AiPreviewOut {
  capability: Capability
  /** What the request contains. */
  exposure: DataExposure
  /** The exposure class for people. */
  exposureDescription: string
  /** AI is enabled. */
  enabled: boolean
  /** The provider it would go to (`null` when none is configured). */
  provider?: ProviderStatus | null
  consent?: ConsentCheck | null
  /** Prompt templates used. */
  promptVersion: string
  /** Characters of the prompt messages. */
  inputChars: number
  maxInputChars: number
  /** The exact HTTP request (key redacted); `null` without a provider. */
  request?: HttpPreview | null
  /**
   * The prompt messages, when there is no provider (otherwise they are in
   * `request.body`).
   */
  messages?: ChatMessage[] | null
  /** It would be sent now. */
  wouldSend: boolean
  /** Why it would not be sent. */
  blockers: string[]
}

export interface AiPreviewParams {
  /** `dax.explain` or `dax.generate` (the others are not implemented). */
  capability: Capability
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** `dax.explain`: the measure (`Name`, `[Name]` or `Table[Name]`). */
  measure?: string | null
  /** `dax.explain`: the measure's table; `dax.generate`: the target table. */
  table?: string | null
  /** `dax.generate`: what the measure should compute. */
  prompt?: string | null
  /** Provider id (default: the default provider). */
  provider?: string | null
  /** `dax.generate`: include this many sample rows of `table`. */
  sampleRows?: number | null
}

/** The AI settings file (`ai.json`). */
export interface AiSettings {
  /** Format version (1). */
  version: number
  /** Master switch (default `false`): nothing is sent while off. */
  enabled: boolean
  /** Provider used when a command names none (default: the first). */
  defaultProvider?: string | null
  providers: ProviderSettings[]
  /** Per-capability limits, keyed by capability id. */
  limits?: { [key: string]: CapabilityLimits }
  /**
   * Most sample rows a request may include (default 0: none; at most
   * 100). Sample rows also need a `sample` consent.
   */
  sampleRows: number
  /** Recorded consents (changed only through `ai.consent.*`). */
  consents: Consent[]
}

export interface AiStatusOut {
  /** AI features are on. Off by default; nothing is sent while off. */
  enabled: boolean
  /** The settings file (`null` if no location could be determined). */
  settingsPath?: string | null
  /** The settings file could not be read (AI then stays off). */
  settingsError?: string | null
  providers: ProviderStatus[]
  /** Recorded consents. */
  consents: Consent[]
  /** The capability contract and what is implemented. */
  capabilities: CapabilityInfo[]
  /** Most sample rows a request may include (0 = none). */
  sampleRows: number
  /** Effective limits per capability. */
  limits: { [key: string]: EffectiveLimits }
  /**
   * The settings as stored (defaults when the file is missing or unreadable). Never holds
   * keys. A settings page edits these and sends them back with `ai.settings.set`.
   */
  settings: AiSettings
}

export interface ApiKeyClearParams {
  /** Provider id. */
  provider: string
}

export interface ApiKeyOut {
  provider: string
  /** A key was stored (`set`) or removed (`clear`). */
  changed: boolean
  /** `keychain` (or the host's store). */
  store: string
  /** Keychain service and account. */
  service: string
  account: string
}

export interface ApiKeySetParams {
  /** Provider id. */
  provider: string
  /** The API key. Only accepted from the desktop app. */
  key: string
}

export interface ApplyEditsParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /**
   * Edits applied atomically and undone as one history step. Names are
   * resolved against the document before the first edit.
   */
  edits: EditOperation[]
  /**
   * The history step's label (`doc.history` `undoLabel` and `redoLabel`,
   * at most 200 characters); default `Apply document edits`.
   */
  label?: string | null
}

/** One step of a batch. */
export interface BatchStep {
  /** Command id. */
  id: string
  /**
   * Parameters; strings may reference earlier results as
   * `${name.field}`.
   */
  params?: JsonValue
  /** Name this step's result for later steps. */
  as?: string | null
}

/** Result of one step. */
export interface BatchStepResult {
  /** 0-based position in the batch. */
  index: number
  id: string
  as?: string | null
  /**
   * The command succeeded (its result may still report data problems,
   * e.g. `"ok": false`).
   */
  ok: boolean
  result?: JsonValue
  error?: OpError | null
}

/** Schema summary of a PBIP local data cache (`.pbi/cache.abf`). */
export interface CacheAbfOut {
  /** Path relative to the opened folder. */
  path: string
  size: number
  /** Visible tables in the cache. */
  tableCount: number
  tables: string[]
  /** `true`/`false` when compared with the model definition. */
  matchesDefinition?: boolean | null
  /** `sqlite` or `xml`. */
  metadataFormat?: string | null
  compatibilityLevel?: number | null
}

/** The PBIP local data cache. */
export interface CacheOut {
  /** Entry in the opened folder, e.g. `X.SemanticModel/.pbi/cache.abf`. */
  entry: string
  size: number
  /** Last modification (Power BI Desktop's last refresh), RFC 3339. */
  modified?: string | null
}

/** What was called (part of every capability result). */
export interface CallInfo {
  capability: Capability
  /** Provider id. */
  provider: string
  kind: ProviderKind
  /** The model that answered (as reported, else as configured). */
  model?: string | null
  /** `host[:port]` the request went to. */
  endpointHost: string
  /** The endpoint is this machine. */
  local: boolean
  /** What the request contained. */
  exposure: DataExposure
  /** Prompt templates used (`system@1+dax.explain@1`). */
  promptVersion: string
  durationMs: number
  usage: Usage
  finishReason?: string | null
}

/** What a click on an action button asks the caller to do. */
export type CanvasAction = {
  kind: 'back'
} | {
  kind: 'navigate'
  page: string
  displayName: string
} | {
  kind: 'webUrl'
  url: string
} | {
  kind: 'notAvailable'
  message: string
}

/** Result of a state change. */
export interface CanvasChangeOut {
  /** The interaction or view state changed. */
  changed: boolean
  /**
   * The re-rendered page (also when unchanged, unless
   * `renderUnchanged` is `false`).
   */
  page: CanvasPageOut | null
  /** What a clicked button asks the caller to do. */
  action?: CanvasAction | null
}

export interface CanvasClearParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report. */
  report?: string | null
  /** Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. */
  page?: string | null
  /**
   * A slicer (clears its selection) or the cross-filter source; absent:
   * clears the cross-filter and closes the dropdowns.
   */
  visual?: string | null
}

export interface CanvasClickParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report. */
  report?: string | null
  /** Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. */
  page?: string | null
  /**
   * A hit id from the last render (`hitRegions[].id`); `null` clicks the
   * empty canvas (clears the cross-filter, closes dropdowns).
   */
  hitId?: string | null
  /** Ctrl (⌘) held: add to / remove from the selection. */
  ctrl?: boolean
  shift?: boolean
  /** Return the page even when nothing changed (default `true`). */
  renderUnchanged?: boolean
}

export interface CanvasClosePopupsParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report. */
  report?: string | null
  /** Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. */
  page?: string | null
  /** Return the page even when nothing changed (default `true`). */
  renderUnchanged?: boolean
}

export interface CanvasHitParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report. */
  report?: string | null
  /** Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. */
  page?: string | null
  /** A hit id from the last render. */
  hitId: string
}

/** Interactive geometry of a slicer, table or matrix. */
export interface CanvasInteractionOut {
  visual: string
  /** `slicer`, `table` or `matrix`. */
  kind: string
  scroll: ScrollOut | null
  /** Slicers: `dropdown`, `list`, `tile` or `range`. */
  slicerMode: string | null
  /** Slicer header band. */
  header: RectOut | null
  /** Collapsed dropdown box. */
  dropdown: RectOut | null
  /** The open dropdown popup (in the overlay), `null` when closed. */
  popup: RectOut | null
  /** Search box, when the slicer has one. */
  search: RectOut | null
  dropdownOpen: boolean
  searchText: string
  /** The slicer filters something (not "All"). */
  hasSelection: boolean
}

/** A rendered page of the interactive canvas. */
export interface CanvasPageOut {
  docId: string
  /** Report name. */
  report: string
  /** Page internal name. */
  page: string
  pageDisplayName: string
  /** Render number of this page (increasing): drop older responses. */
  generation: number
  /**
   * The semantic document revision the report, the page and its data
   * were read at (one revision, even while edits commit).
   */
  documentRevision: number
  /**
   * The page as a standalone SVG document (deterministic; `data-hit`
   * attributes on interactive groups).
   */
  svg: string
  /**
   * Open slicer dropdown popups, drawn above the page; `null` when none
   * is open.
   */
  overlaySvg: string | null
  width: number
  height: number
  /** Hit regions in paint order (hit-test from the end). */
  hitRegions: HitRegionOut[]
  /** Hit regions of the overlay (above every page region). */
  overlayHitRegions: HitRegionOut[]
  /** Every visual of the page (hidden ones included), in page order. */
  visuals: CanvasVisualOut[]
  interactions: CanvasInteractionOut[]
  /** Slicer selections and the cross-filter. */
  selections: SelectionOut[]
  /**
   * Report + page filters + selections as a filter spec (what cards are
   * evaluated in, ignoring per-visual interactions).
   */
  effectiveFilters: FilterSpecOut
  /** The state equals the report's saved state. */
  isDefault: boolean
  /** Unreadable saved slicer selections. */
  warnings: string[]
  /** Query + render wall time. */
  durationMs: number
}

/** The page of a canvas command. */
export interface CanvasPageParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Report (default: the first). */
  report?: string | null
  /** Page (default: the active page). */
  page?: string | null
}

export interface CanvasRenderParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report. */
  report?: string | null
  /** Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. */
  page?: string | null
  /** Fit the page into this viewport (kept for later renders). */
  viewport?: ViewportParam | null
}

export interface CanvasScrollParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report. */
  report?: string | null
  /** Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. */
  page?: string | null
  /** The table, matrix or slicer to scroll. */
  visual: string
  /** Horizontal delta in page units. */
  dx?: number
  /** Vertical delta in page units (positive scrolls down). */
  dy?: number
  /** Return the page even when nothing changed (default `true`). */
  renderUnchanged?: boolean
}

export interface CanvasSearchParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report. */
  report?: string | null
  /** Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. */
  page?: string | null
  /** The slicer. */
  visual: string
  /** Search text (at most 256 characters; empty clears it). */
  text: string
}

export interface CanvasSetSlicerParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report. */
  report?: string | null
  /** Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. */
  page?: string | null
  /** The slicer's visual name… */
  visual?: string | null
  /** …or its field (display name, column, `Table[Column]`). */
  field?: string | null
  /**
   * Values to select (`null` = BLANK); empty without `invert` selects
   * all.
   */
  values?: JsonValue[]
  /** Select everything except `values`. */
  invert?: boolean
}

export interface CanvasStateOut {
  /** Slicer selections and selected data points (`InteractionState`). */
  interaction: JsonValue
  /** Scroll, expansion and dropdown state per visual (`PageViewState`). */
  view: JsonValue
  /** The interaction state equals the report's saved state. */
  isDefault: boolean
}

/** One visual of a rendered page. */
export interface CanvasVisualOut {
  /** Internal name, the `visual` part of hit ids. */
  name: string
  visualType: string
  /** Title text, else the automatic title, else the visual type. */
  title: string | null
  /** Container rectangle (`null` for hidden visuals). */
  rect: RectOut | null
  hidden: boolean
  /** Query error (`⚠ field: message`). */
  error: string | null
  /** Hover text: the error with its DAX, or a button's tooltip. */
  tooltip: string | null
  /** Filters and interactions not applied (or only approximately). */
  warnings: string[]
  /** The data was cut at the query limits. */
  truncated: boolean
  /** A button whose click does something (`canvas.click` returns it). */
  hasAction: boolean
}

/** An AI capability. The id (`dax.explain`, …) is stable. */
export type Capability = 'dax.explain' | 'dax.generate' | 'm.generate' | 'report.lint' | 'chart.suggest'

/** Description of a capability (`ai.status`). */
export interface CapabilityInfo {
  id: Capability
  title: string
  /** Exposure class of a request without sample rows. */
  exposure: DataExposure
  /** The most a request may expose. */
  maxExposure: DataExposure
  /** Implemented in this version. */
  implemented: boolean
  /** The registry command that runs it, when implemented. */
  command?: string | null
}

/** Limits of one capability (unset fields take the defaults). */
export interface CapabilityLimits {
  /** Most tokens the model may answer (default 2000, at most 32768). */
  maxTokens?: number | null
  /** Sampling temperature, 0–2 (default 0.2). */
  temperature?: number | null
  /** Request timeout in seconds, 1–600 (default 120). */
  timeoutSecs?: number | null
  /** Largest prompt in characters (default 60000, at most 1000000). */
  maxInputChars?: number | null
}

/** One end of a relationship. */
export type CardinalityParam = 'one' | 'many'

/** What a command is about; groups the catalog. */
export type Category = 'meta' | 'workspace' | 'inspect' | 'lint' | 'data' | 'dax' | 'render' | 'canvas' | 'export' | 'convert' | 'transform' | 'edit' | 'ai'

/** One chat message. */
export interface ChatMessage {
  role: Role
  content: string
}

/** Result of `model.column.create`. */
export interface ColumnCreateOut {
  changed: boolean
  revision: number
  dirty: boolean
  changedObjectIds: number[]
  /**
   * The calculated tables and columns the edit recomputed over the
   * loaded data, in order (absent when none); a failed one has no values
   * until an edit or a refresh computes it again.
   */
  recalculated?: RecalculatedOut[]
  columnId: number
  table: string
  name: string
  /** `string`, `int64`, `double`, `decimal`, `dateTime` or `boolean`. */
  dataType: string
  /**
   * Whether the data type was inferred from the values (no `dataType`
   * given).
   */
  dataTypeInferred: boolean
}

/** An applied column filter. */
export interface ColumnFilterOut {
  table: string
  column: string
  /** Values (`null` = BLANK). */
  values: JsonValue[]
  /** `NOT IN` instead of `IN`. */
  exclude: boolean
}

/** `table[column] IN values` (or `NOT IN` with `exclude`). Parameter form. */
export interface ColumnFilterParam {
  /** Table name (case-insensitive). */
  table: string
  /** Column name (case-insensitive). */
  column: string
  /**
   * Values: strings, numbers, booleans or `null` (BLANK); converted to
   * the column type (`"42"` matches an integer column, ISO text a date).
   */
  values: JsonValue[]
  /** Keep the rows whose value is *not* in `values`. */
  exclude?: boolean
}

export interface ColumnOut {
  name: string
  /** `string`, `int64`, `double`, `decimal`, `dateTime`, `boolean`, … */
  dataType: string
  /** `data`, `calculated`, `calculatedTableColumn` or `rowNumber`. */
  kind: string
  isHidden: boolean
  isKey: boolean
  description?: string | null
  sourceColumn?: string | null
  /** DAX of a calculated column (omitted with `--no-expressions`). */
  expression?: string | null
  formatString?: string | null
  summarizeBy?: string | null
  sortByColumn?: string | null
  displayFolder?: string | null
  dataCategory?: string | null
}

/** A column named by table and column (case-insensitive). */
export interface ColumnParam {
  table: string
  column: string
}

export interface ColumnParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Table name (case-insensitive). */
  table: string
  /** Column name (case-insensitive). */
  column: string
}

export interface ColumnRefOut {
  table: string
  column: string
}

/** A recomputed calculated column. */
export interface ColumnRefreshOut {
  name: string
  durationMs: number
  status: StatusOut
}

export interface ColumnStatsOut {
  /** Table name (model spelling). */
  table: string
  /** Column name (model spelling). */
  column: string
  dataType?: string | null
  rows: number
  /** Non-blank values. */
  count: number
  blanks: number
  /** Distinct non-blank values (text compared case-insensitively). */
  distinct: number
  /** Smallest non-blank value (`null` if none). */
  min: JsonValue
  max: JsonValue
  minFormatted: string
  maxFormatted: string
}

/** Result of `model.column.update`. */
export interface ColumnUpdateOut {
  changed: boolean
  revision: number
  dirty: boolean
  changedObjectIds: number[]
  columnId: number
  table: string
  name: string
  /**
   * The data type after the edit: `string`, `int64`, `double`,
   * `decimal`, `dateTime` or `boolean`.
   */
  dataType: string
  /**
   * Whether the data type was inferred from the values of the new
   * expression (no `dataType` given).
   */
  dataTypeInferred: boolean
  /**
   * The column and what depends on it, recomputed over the loaded data,
   * in order (absent when none); a failed one has no values until an
   * edit or a refresh computes it again.
   */
  recalculated?: RecalculatedOut[]
}

/**
 * Serializable description of a command (`commands.list`,
 * `commands.describe`, `dashcraft commands --json`).
 */
export interface CommandInfo {
  /** Stable dotted id (`doc.open`); the JSON-RPC method name. */
  id: string
  title: string
  /** One line: what it does and returns. */
  summary: string
  category: Category
  /**
   * Changes the in-memory state of an open document (ordered per
   * document).
   */
  mutatesDocument: boolean
  /** Reports progress and can take a while. */
  longRunning: boolean
  /** Offered to AI agents (MCP) by default. */
  agentSafe: boolean
  /** Writes files (checked by the path policy). */
  writesFiles: boolean
  /** Version that added it. */
  since: string
  examples: ExampleInfo[]
  /** JSON Schema of the parameters (with `schemas`). */
  paramsSchema?: JsonValue
  /** JSON Schema of the result (with `schemas`). */
  outputSchema?: JsonValue
}

export interface CommandsBatchOut {
  /** Every step ran and succeeded. */
  ok: boolean
  /** One entry per step that ran, in order. */
  results: BatchStepResult[]
}

export interface CommandsBatchParams {
  /** Steps, run in order on the same workspace. */
  steps: BatchStep[]
  /**
   * Stop at the first failing step (default `true`). Batches are not
   * atomic: earlier steps stay applied.
   */
  stopOnError?: boolean
}

export interface CommandsDescribeParams {
  /** Command id, e.g. `doc.open`. */
  id: string
}

export interface CommandsListOut {
  count: number
  commands: CommandInfo[]
}

export interface CommandsListParams {
  /** Include each command's parameter and result JSON Schemas. */
  schemas?: boolean
  /** Only this category. */
  category?: Category | null
  /** Only the commands offered to agents. */
  agentSafe?: boolean
}

/** Parquet compression. */
export type CompressionParam = 'zstd' | 'snappy' | 'none'

export interface ConnectorMemberOut {
  name: string
  shared: boolean
  dataSourceKind?: string | null
  publish?: string | null
}

export interface ConnectorOut {
  files: string[]
  signed: boolean
  sources: ConnectorSourceOut[]
}

export interface ConnectorSourceOut {
  path: string
  section?: string | null
  members: ConnectorMemberOut[]
  error?: string | null
}

/**
 * A recorded permission to send one capability's data of an exposure class
 * to one provider endpoint.
 */
export interface Consent {
  /** Provider id. */
  provider: string
  /**
   * `host[:port]` the consent was granted for; a changed endpoint needs
   * a new consent.
   */
  endpointHost: string
  capability: Capability
  /** The most the provider may receive for this capability. */
  exposure: DataExposure
  /** The endpoint is this machine (data does not leave it). */
  local: boolean
  /** When it was granted (RFC 3339, UTC). */
  grantedAt: string
  /** Who granted it (`cli`, `app`, `rpc`). */
  grantedBy?: string | null
}

/** The consent a request needs and whether it is recorded. */
export interface ConsentCheck {
  granted: boolean
  provider: string
  endpointHost: string
  capability: Capability
  exposure: DataExposure
  /** The endpoint is this machine. */
  local: boolean
  /** The recorded consent that covers it. */
  consent?: Consent | null
}

export interface ConsentGrantOut {
  consent: Consent
  /** What was allowed, for people. */
  note: string
  settingsPath?: string | null
}

export interface ConsentGrantParams {
  /** Provider id. */
  provider: string
  capability: Capability
  /** The most that may be sent (default `metadata`; `full` is refused). */
  exposure?: DataExposure
}

export interface ConsentRevokeOut {
  /** Consents removed. */
  removed: number
  /** Consents left. */
  consents: Consent[]
}

export interface ConsentRevokeParams {
  /** Provider id. */
  provider: string
  /** Only this capability (default: all of the provider's consents). */
  capability?: Capability | null
}

export interface ConvertPbipOut {
  outDir: string
  /** Files written, relative to `outDir`. */
  files: string[]
  bytes: number
}

export interface ConvertPbipParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /**
   * Output folder (created if missing; must be empty unless
   * `overwrite`).
   */
  outDir: string
  /**
   * Write into a non-empty folder (files with the same paths are
   * replaced).
   */
  overwrite?: boolean
}

export interface ConvertPbitOut {
  outPath: string
  /** The report written. */
  report: string
  /** Reports left out (a PBIT holds one). */
  omittedReports: string[]
  bytes: number
}

export interface ConvertPbitParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Output `.pbit` file. */
  outPath: string
  /**
   * The report to include (a PBIT holds one; exact name, default the
   * first).
   */
  report?: string | null
  /** Replace an existing file. */
  overwrite?: boolean
}

/** Totals over `model`, `extraModels` and `reports`. */
export interface Counts {
  tables: number
  columns: number
  calculatedColumns: number
  calculatedTables: number
  measures: number
  relationships: number
  expressions: number
  parameters: number
  reports: number
  pages: number
  visuals: number
  /** Report-, page- and visual-level filters. */
  filters: number
  errors: number
  warnings: number
  infos: number
}

export interface CreateColumnParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** The column's table. */
  table: string
  /** The column name: not a column or measure of `table`. */
  name: string
  /** The DAX expression, evaluated per row of `table`. */
  expression: string
  /**
   * The data type; inferred from the values when omitted (after the
   * automatic first load; required when the table has no rows loaded or
   * every value is blank).
   */
  dataType?: DataTypeParam | null
  formatString?: string | null
  displayFolder?: string | null
  description?: string | null
  isHidden?: boolean
}

export interface CreateHierarchyParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** The hierarchy's table. */
  table: string
  /** The hierarchy name: not a column, measure or hierarchy of `table`. */
  name: string
  /** The levels, top level first: different columns of `table`. */
  levels: LevelParam[]
  displayFolder?: string | null
  description?: string | null
  isHidden?: boolean
}

export interface CreateMeasureParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** The measure's home table. */
  table: string
  /** The measure name: unique across the model and not a column of `table`. */
  name: string
  /** The DAX expression. */
  expression: string
  formatString?: string | null
  displayFolder?: string | null
  description?: string | null
  isHidden?: boolean
}

export interface CreateRelationshipParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** The from column (TOM `fromColumn`, usually the many side). */
  from: ColumnParam
  /** The to column (TOM `toColumn`, usually the one side). */
  to: ColumnParam
  /** Default `many`. */
  fromCardinality?: CardinalityParam | null
  /** Default `one`. */
  toCardinality?: CardinalityParam | null
  /** Default `oneDirection`. */
  crossFilteringBehavior?: CrossFilterParam | null
  /** Default `true`. Two tables have at most one active relationship. */
  isActive?: boolean | null
  /**
   * Unique among relationships; a new GUID by default, as Power BI
   * Desktop names them.
   */
  name?: string | null
}

export interface CreateTableParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** The table name: not used by another table. */
  name: string
  /** The DAX table expression; its result's columns become the table's. */
  expression: string
  description?: string | null
  isHidden?: boolean
}

/**
 * A column of a table created or changed by `model.table.create` or
 * `model.table.update`.
 */
export interface CreatedTableColumnOut {
  columnId: number
  name: string
  /** `string`, `int64`, `double`, `decimal`, `dateTime` or `boolean`. */
  dataType: string
  /**
   * Where the values come from: in a calculated table's expression
   * result, as Power BI Desktop writes it, `Web Sessions[Device]` (the
   * table unquoted) or `[Total]`; the query's column for a data column;
   * empty for a calculated column.
   */
  sourceColumn: string
}

/** Which way filters cross a relationship. */
export type CrossFilterParam = 'oneDirection' | 'bothDirections'

/** CSV reading options. */
export interface CsvParams {
  /** Field delimiter, one ASCII character (default `,`). */
  delimiter?: string | null
  /** The first record holds column names (default `true`). */
  hasHeader?: boolean | null
  /**
   * Encoding: WHATWG label (`utf-8`, `windows-1252`, …) or code page
   * (`65001`, `1252`). Default: BOM, else UTF-8, else Windows-1252.
   */
  encoding?: string | null
}

export interface CustomVisualOut {
  name?: string | null
  displayName?: string | null
  guid?: string | null
  version?: string | null
  description?: string | null
  author?: string | null
  apiVersion?: string | null
  supportUrl?: string | null
  dataRoles: VisualDataRoleOut[]
  objects: string[]
  privileges: string[]
  scriptBytes: number
}

export interface DataAddDerivedColumnParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Table name (case-insensitive). */
  table: string
  /** New column name (unique in the table). */
  name: string
  kind: DerivedKindParam
}

/**
 * What kind of data a request contains. Ordered: a consent for a class
 * covers the classes below it.
 */
export type DataExposure = 'metadata' | 'sample' | 'full'

export interface DataLoadCsvParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Table name (case-insensitive). */
  table: string
  /** The CSV file. */
  path: string
  options?: CsvParams
}

export interface DataPageOut {
  /** Table name (model spelling). */
  table: string
  offset: number
  /** Rows passing the filter. */
  totalRows: number
  columns: ResultColumnOut[]
  /**
   * Row-major cells: raw values (numbers, strings, booleans, ISO dates,
   * `null` for BLANK) or formatted text.
   */
  rows: JsonValue[][]
}

export interface DataPageParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Table name (case-insensitive). */
  table: string
  /** First row (0-based, after sorting and filtering). */
  offset?: number
  /** Rows to return (default 100, at most 5000). */
  limit?: number
  sort?: SortParam | null
  /**
   * SQL boolean expression over the table's columns, e.g.
   * `"Status" = 'Completed'` (quote names with spaces).
   */
  filter?: string | null
  /**
   * Cells as text formatted with each column's format string instead of
   * raw values.
   */
  formatted?: boolean
  /**
   * With `formatted`, BLANK cells are `null` instead of `""` (tells an
   * empty text from BLANK).
   */
  blankAsNull?: boolean
}

export interface DataRefreshParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /**
   * Tables to refresh (what depends on them is recomputed too). Default:
   * every table.
   */
  tables?: string[] | null
  /** Data source (default: the one given to `doc.open`). */
  source?: SourceParam | null
}

/** Data status of one table. */
export interface DataTableOut {
  name: string
  /** `regular`, `calculatedTable`, `calculationGroup`, `remote`. */
  kind: string
  hidden: boolean
  rows: number
  /** Columns stored (data, calculated and derived). */
  columns: number
  loaded: boolean
  /**
   * `data.loadCsv` can replace its data (every table of a PBIX; tables
   * other than calculated tables otherwise).
   */
  canLoadCsv: boolean
  /** Result of the last refresh or CSV load of this table. */
  lastRefresh?: TableRefreshOut | null
}

export interface DataTablesOut {
  tables: DataTableOut[]
  /** Session derived columns. */
  derived: DerivedColumnOut[]
  cache?: CacheOut | null
  /** Data generation. */
  generation: number
}

/** A column data type. */
export type DataTypeParam = 'string' | 'int64' | 'double' | 'decimal' | 'dateTime' | 'boolean'

export interface DataflowEntityOut {
  name: string
  entityType?: string | null
  attributes: number
  partitions: DataflowPartitionOut[]
}

export interface DataflowPartitionOut {
  name: string
  /** Secret parameters masked. */
  location?: string | null
  refreshTime?: string | null
}

export interface DatasetReferenceOut {
  /** `byPath`, `byConnection` (remote), `embedded` (PBIX/PBIT) or `none`. */
  kind: string
  /** `byPath`: model folder relative to the report folder. */
  path?: string | null
  /** `byConnection`: connection string (secrets masked). */
  connectionString?: string | null
  modelId?: string | null
  databaseName?: string | null
}

/** Letter case `dax.format` gives keywords or function names. */
export type DaxCasing = 'upper' | 'lower' | 'keep'

/** How a DAX text was checked. */
export type DaxCheckedKind = 'expression' | 'query'

export interface DaxCompleteOut {
  /** The document revision whose model was used (with `docId`). */
  documentRevision?: number | null
  /**
   * Start of the text a chosen item replaces (the name being typed; the
   * cursor when nothing is typed yet).
   */
  start: TextPositionOut
  /** End (exclusive) of the replaced text. */
  end: TextPositionOut
  /** The typed text the items match (`Tot` in `[Tot`). */
  prefix: string
  /**
   * Best match first: the same name, then names that start with the
   * prefix, then a word that does, then names that contain it. Empty
   * inside a comment, string or number, and after `VAR`.
   */
  items: DaxCompletionItemOut[]
  /** More items matched than `maxItems`. */
  incomplete: boolean
}

export interface DaxCompleteParams {
  /**
   * Suggest the tables, columns and measures of this document's model and
   * its report-level measures (`d1`, …). Without it only functions,
   * keywords, variables and the query's own definitions are suggested.
   */
  docId?: string | null
  /** The DAX text. */
  text: string
  /** The cursor. */
  position: TextPositionIn
  /** Most items returned, best first (default 100, at most 1000). */
  maxItems?: number
}

/** One suggestion. */
export interface DaxCompletionItemOut {
  /**
   * What a list shows: `SUM`, `Sales`, `'Product Category'[Category]`,
   * `[Total Sales]`, a variable, `ORDER BY`.
   */
  label: string
  kind: DaxCompletionKind
  /**
   * The text that replaces `start`..`end`: `SUM(`, `PI()`, `SUM` when a
   * `(` already follows, the label for anything else.
   */
  insertText: string
  /**
   * `SUM(column) → number`, `column (string)`, `measure`, `table`,
   * `variable`, `parameter` (of a user-defined function), `keyword`,
   * `user-defined function`.
   */
  detail: string
  /** The home table of a column or measure. */
  table?: string | null
  /** A function's or keyword's description, or the model description. */
  description?: string | null
}

/** What a completion item inserts. */
export type DaxCompletionKind = 'function' | 'table' | 'column' | 'measure' | 'variable' | 'keyword' | 'userFunction'

/** One problem in the text. */
export interface DaxDiagnosticOut {
  /** `error` or `warning` (a function DashCraft cannot evaluate yet). */
  severity: string
  /**
   * `syntax`, `unknownTable`, `unknownColumn`, `unknownMeasure`,
   * `argumentCount` or `unsupportedFunction`.
   */
  code: string
  message: string
  /** Start of the text the problem covers. */
  start: TextPositionOut
  /** End (exclusive). */
  end: TextPositionOut
  /** A "did you mean …" correction. */
  suggestion?: string | null
}

export interface DaxDiagnosticsOut {
  /** The document revision whose model names were used (with `docId`). */
  documentRevision?: number | null
  /** How the text was checked. */
  kind: DaxCheckedKind
  /** No errors (warnings allowed). */
  ok: boolean
  /** In source order, at most 1000. */
  diagnostics: DaxDiagnosticOut[]
}

export interface DaxDiagnosticsParams {
  /**
   * Check names against this document's model and report-level measures
   * (`d1`, …). Without it only syntax and functions are checked.
   */
  docId?: string | null
  /** The DAX text. */
  text: string
  /** `auto` (default), `expression` or `query`. */
  kind?: DaxTextKind
}

export interface DaxFormatOut {
  /** How the text was parsed. */
  kind: DaxCheckedKind
  /**
   * The text parses (else `text` is the input unchanged and `diagnostic`
   * says why).
   */
  ok: boolean
  /** The formatted text: `\n` line breaks, no trailing line break. */
  text: string
  /** `text` differs from the input. */
  changed: boolean
  /** The syntax error when `ok` is false. */
  diagnostic?: DaxDiagnosticOut | null
}

export interface DaxFormatParams {
  /** The DAX text. */
  text: string
  /** `auto` (default), `expression` or `query`. */
  kind?: DaxTextKind
  /**
   * Preferred maximum line length in characters (default 80, at most
   * 10000). Only a single long token, comment or unbreakable run makes a
   * line longer.
   */
  maxWidth?: number
  /**
   * Spaces per indentation level (default 4, at most 16), or the width a
   * tab counts for with `useTabs`.
   */
  indentWidth?: number
  /** Indent with tabs instead of spaces. */
  useTabs?: boolean
  /**
   * Case of keywords (`VAR`, `RETURN`, `IN`, `TRUE`, `EVALUATE`, `DESC`,
   * parameter types, …): `upper` (default), `lower` or `keep`.
   */
  keywordCase?: DaxCasing
  /**
   * Case of built-in function names: `upper` (default), `lower` or
   * `keep`. User-defined functions, tables, columns, measures and
   * variables always keep their spelling.
   */
  functionCase?: DaxCasing
}

/** The name under the cursor. */
export interface DaxHoverInfo {
  kind: DaxHoverKind
  /**
   * The name as DAX: `SUM`, `Sales`, `'Product Category'[Category]`,
   * `[Total Sales]`, a variable, `VAR`.
   */
  name: string
  /** The home table of a column or measure. */
  table?: string | null
  /** Start of the hovered name in the text. */
  start: TextPositionOut
  /** End (exclusive). */
  end: TextPositionOut
  /** Everything below as Markdown (code in `dax` fences). */
  markdown: string
  /** A function's signature, `SUM(column)`. */
  signature?: string | null
  /** A function's return type. */
  returns?: string | null
  /** A function's or keyword's description, or the model description. */
  description?: string | null
  /** For a function: whether DashCraft can evaluate it. */
  supported?: boolean | null
  /** A column's data type (`string`, `int64`, `dateTime`, …). */
  dataType?: string | null
  /** The DAX of a measure, calculated column or variable. */
  expression?: string | null
  formatString?: string | null
  hidden?: boolean
  /** A table's column count. */
  columnCount?: number | null
  /** A table's measure count. */
  measureCount?: number | null
}

/** What a hover describes. */
export type DaxHoverKind = 'function' | 'table' | 'column' | 'measure' | 'variable' | 'keyword' | 'userFunction'

export interface DaxHoverOut {
  /** The document revision whose model was used (with `docId`). */
  documentRevision?: number | null
  /** Absent when there is nothing to describe at the cursor. */
  hover?: DaxHoverInfo | null
}

export interface DaxHoverParams {
  /**
   * Describe tables, columns and measures of this document's model and
   * report-level measures (`d1`, …). Without it only functions,
   * keywords, variables and the query's own definitions are described.
   */
  docId?: string | null
  /** The DAX text. */
  text: string
  /** The cursor. */
  position: TextPositionIn
}

/** One parameter of a signature. */
export interface DaxParameterOut {
  name: string
  /** May be left out. */
  optional: boolean
  /** Part of a group that may repeat. */
  repeating: boolean
  /**
   * Start of the name in `signature` (the signature is ASCII, so bytes,
   * characters and UTF-16 units agree).
   */
  labelStart: number
  /** End of the name in `signature` (exclusive). */
  labelEnd: number
}

export interface DaxQueryOut {
  /** Semantic document revision captured with the query engine snapshot. */
  documentRevision: number
  /** One table per `EVALUATE`. */
  tables: DaxTableOut[]
  durationMs: number
}

export interface DaxQueryParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** A DAX query: `EVALUATE …`, with `DEFINE` / `ORDER BY`. */
  query: string
  /** Rows returned per result table (default 1000, at most 5000). */
  maxRows?: number
  /** Outer filter context (`table[column] IN values`, combined with AND). */
  filters?: ColumnFilterParam[]
  /** Cells as formatted text instead of raw values. */
  formatted?: boolean
  /** With `formatted`, BLANK cells are `null` instead of `""`. */
  blankAsNull?: boolean
}

/** A span of the text. */
export interface DaxRangeOut {
  start: TextPositionOut
  /** End (exclusive). */
  end: TextPositionOut
}

/** One occurrence of the symbol. */
export interface DaxReferenceOut {
  start: TextPositionOut
  /** End (exclusive). */
  end: TextPositionOut
  /** The text declares the symbol here. */
  declaration?: boolean
}

export interface DaxReferencesOut {
  /** The document revision whose model was used (with `docId`). */
  documentRevision?: number | null
  /**
   * Absent when the cursor is not on a name (a keyword, literal,
   * comment or operator).
   */
  symbol?: DaxSymbolOut | null
  /**
   * Where the text declares the symbol (a `VAR`, a function parameter,
   * a query `DEFINE`); absent for model objects and DAX functions.
   */
  definition?: DaxRangeOut | null
  /** Every occurrence in source order, the declaration included. */
  references: DaxReferenceOut[]
}

export interface DaxReferencesParams {
  /**
   * Resolve names against this document's model and report-level
   * measures (`d1`, …), and name model objects by `objectId`.
   */
  docId?: string | null
  /** The DAX text. */
  text: string
  /** The cursor. */
  position: TextPositionIn
}

/** One DAX expression a rename rewrites. */
export interface DaxRewriteOut {
  /**
   * The object holding the expression, with the names after the rename
   * (e.g. `measure 'Orders'[Total Sales]`).
   */
  location: string
  /** `expression`, `formatStringDefinition`, `filterExpression`, … */
  property: string
  before: string
  after: string
}

export interface DaxSignatureHelpOut {
  /** Absent outside a call of a known DAX function (or in a comment). */
  signature?: DaxSignatureOut | null
}

export interface DaxSignatureHelpParams {
  /** The DAX text. */
  text: string
  /** The cursor. */
  position: TextPositionIn
}

/** The function call around the cursor. */
export interface DaxSignatureOut {
  /** Upper-case name (`SUM`). */
  function: string
  /** `IF(logicalTest, valueIfTrue, [valueIfFalse])`. */
  signature: string
  /** The signature with its return type, `SUM(column) → number`. */
  label: string
  /**
   * `any`, `boolean`, `currency`, `datetime`, `integer`, `number`,
   * `table` or `text`.
   */
  returns: string
  /** The function family (`Text`, `Time intelligence`, …). */
  category: string
  /** One sentence. */
  description: string
  /** Whether DashCraft can evaluate it. */
  supported: boolean
  minArgs: number
  /** Absent when unlimited. */
  maxArgs?: number | null
  parameters: DaxParameterOut[]
  /** 0-based index of the argument the cursor is in. */
  argument: number
  /**
   * The parameter that argument fills (repeating groups cycle); absent
   * past the last parameter.
   */
  activeParameter?: number | null
  /** Start of the function name in the text. */
  nameStart: TextPositionOut
  /** End of the function name (exclusive). */
  nameEnd: TextPositionOut
}

/** What a symbol is. */
export type DaxSymbolKind = 'function' | 'table' | 'column' | 'measure' | 'variable' | 'userFunction' | 'parameter'

/** The symbol at the cursor. */
export interface DaxSymbolOut {
  kind: DaxSymbolKind
  /**
   * The name as DAX, spelled as in the model, else as declared, else as
   * first written: `SUM`, `Sales`, `'Product Category'[Category]`,
   * `[Total Sales]`, a variable.
   */
  name: string
  /** The home table of a column or measure, when known. */
  table?: string | null
  /**
   * The model's table, column or measure of that name (with `docId`;
   * the `objectId` of `model.*` edit commands). A query `DEFINE` of the
   * same name overrides it within the text.
   */
  objectId?: number | null
}

/** One result table of a DAX query. */
export interface DaxTableOut {
  columns: ResultColumnOut[]
  /** Row-major cells (raw values, `null` for BLANK; or formatted text). */
  rows: JsonValue[][]
  totalRows: number
  /** More rows than `maxRows`. */
  truncated: boolean
}

/** What a DAX text is. */
export type DaxTextKind = 'auto' | 'expression' | 'query'

/** What a token is. */
export type DaxTokenKind = 'function' | 'table' | 'column' | 'measure' | 'variable' | 'string' | 'number' | 'comment' | 'operator' | 'keyword' | 'userFunction' | 'parameter' | 'queryParameter' | 'dateTime'

/** One token. */
export interface DaxTokenOut {
  start: TextPositionOut
  /** End (exclusive). */
  end: TextPositionOut
  kind: DaxTokenKind
  /**
   * The text declares the name here: a `VAR`, a function parameter, a
   * query `DEFINE`.
   */
  declaration?: boolean
  /** A name the document's model and the text do not have. */
  unresolved?: boolean
}

export interface DaxTokensOut {
  /** The document revision whose model was used (with `docId`). */
  documentRevision?: number | null
  /** In source order, without whitespace and punctuation. */
  tokens: DaxTokenOut[]
}

export interface DaxTokensParams {
  /**
   * Resolve names against this document's model and report-level
   * measures (`d1`, …). Without it names keep their syntactic kind and
   * none is unresolved.
   */
  docId?: string | null
  /** The DAX text. */
  text: string
}

export interface DeleteColumnParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** The column's id (instead of `table` and `column`). */
  columnId?: number | null
  /** The column's table. */
  table?: string | null
  /** The column's name. */
  column?: string | null
  /**
   * Only check the delete and list what refers to the column; the
   * document is unchanged.
   */
  dryRun?: boolean
}

export interface DeleteHierarchyParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** The hierarchy's id (instead of `table` and `hierarchy`). */
  hierarchyId?: number | null
  /** The hierarchy's table. */
  table?: string | null
  /** The hierarchy's name. */
  hierarchy?: string | null
  /**
   * Only list the report fields that use the hierarchy; the document is
   * unchanged.
   */
  dryRun?: boolean
}

export interface DeleteMeasureParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** The measure's id (instead of `table` and `measure`). */
  measureId?: number | null
  /** The measure's table. */
  table?: string | null
  /** The measure's name. */
  measure?: string | null
  /** Only list what refers to the measure; the document is unchanged. */
  dryRun?: boolean
}

export interface DeleteRelationshipParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** The relationship's id from `model.relationships` (instead of `name`). */
  relationshipId?: number | null
  /** The relationship's name (case-insensitive). */
  name?: string | null
  /** Only list what uses the relationship; the document is unchanged. */
  dryRun?: boolean
}

export interface DeleteTableParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** The table's id (instead of `table`). */
  tableId?: number | null
  /** The table's name. */
  table?: string | null
  /**
   * Only list what refers to the table and what goes with it; the
   * document is unchanged.
   */
  dryRun?: boolean
}

/** Result of `model.dependencies`. */
export interface DependenciesOut {
  objectId: number
  /** `table`, `column`, `measure` or `relationship`. */
  kind: string
  /**
   * The table, or the table of the column or measure (the from table of
   * a relationship).
   */
  table: string
  name: string
  /** The document revision the list was computed on. */
  revision: number
  /**
   * What refers to the object: DAX expressions, relationships, hierarchy
   * levels, sort-by columns, role permissions, report, page and visual
   * fields and filters.
   */
  dependents: DependentOut[]
  /**
   * References that cannot be confirmed (DAX that does not parse,
   * ambiguous unqualified names, M queries naming a table).
   */
  warnings: string[]
}

export interface DependenciesParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /**
   * The object's id (instead of `table`): also a relationship's id from
   * `model.relationships`.
   */
  objectId?: number | null
  /** The table, or the table of `column` / `measure`. */
  table?: string | null
  /** A column of `table`. */
  column?: string | null
  /** A measure of `table`. */
  measure?: string | null
}

/** A note about one dependency. */
export interface DependencyNote {
  /** `[Measure]`, `Table[Column]` or `Table`. */
  name: string
  /** Its role in the calculation. */
  note: string
}

/** An object property that refers to another object. */
export interface DependentOut {
  /**
   * E.g. `measure 'Orders'[Twice]` or `visual 'card1' (card) on page
   * 'Overview' of report 'Sales'`.
   */
  location: string
  /** E.g. `expression`, `fields`, `filters`, `filterExpression`. */
  property: string
}

/** A session derived column. */
export interface DerivedColumnOut {
  table: string
  name: string
  /** `people`, `expression` or `array`. */
  kind: string
  /** The parsed text column (`people`). */
  sourceColumn?: string | null
  /** The DAX expression (`expression`). */
  dax?: string | null
  /** Model data type of the values (`string`, `int64`, …). */
  dataType: string
  /** The column is in the table data now. */
  active: boolean
  status: StatusOut
}

/** How a derived column is computed. */
export type DerivedKindParam = {
  people: {
    sourceColumn: string
  }
} | {
  dax: {
    expression: string
  }
}

export interface DerivedRefParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Table name (case-insensitive). */
  table: string
  name: string
}

/** `detect --json` output. */
export interface DetectOut {
  schemaVersion: number
  /** The path as given. */
  path: string
  /** The `source.format` `inspect` reports for this input. */
  sourceFormat: string
  detection: DetectionOut
  items: ItemsOut
}

export interface DetectionOut {
  /**
   * Detected format kind: `pbip`, `pbipSemanticModelFolder`,
   * `pbipReportFolder`, `pbix`, `pbit`, `bim`, `tmdlFolder`, …,
   * `unknown`.
   */
  format: string
  /** Human-readable label, e.g. `PBIP project`. */
  label: string
  /** `noSignal`, `extensionOnly`, `likely` or `certain`. */
  confidence: string
  /** Why the format was recognised, in detection order. */
  signals: string[]
}

/** Counts of diagnostics. */
export interface DiagnosticCounts {
  errors: number
  warnings: number
  infos: number
}

export interface DiagnosticOut {
  /** `error`, `warning` or `info`. */
  severity: string
  /** Stable code, e.g. `CORE004`, `TMDL001`. */
  code: string
  message: string
  origin?: OriginOut | null
}

export interface DocCloseOut {
  /** The document id. */
  docId: string
  closed: boolean
}

export interface DocCloseParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Explicitly discard unsaved in-memory edits. */
  discardChanges?: boolean
}

export interface DocInspectParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /**
   * Include DAX, M and query text (default `true`); `false` gives an
   * outline that is safe to share.
   */
  includeExpressions?: boolean | null
}

export interface DocLintParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Leave out findings below this severity (default `info`: all). */
  minSeverity?: SeverityParam | null
  /** Rules to skip, e.g. `["LINT016"]`. */
  disable?: string[]
  /** Run the layout rules LINT015/LINT016 (default `true`). */
  layout?: boolean | null
  /**
   * `ok` is `false` for findings at or above this severity (default
   * `error`), as `dashcraft lint --deny`.
   */
  deny?: SeverityParam | null
}

export interface DocListOut {
  documents: DocSummary[]
}

export interface DocOpenParams {
  /**
   * File or folder: `.pbip`, `.pbix`, `.pbit`, `*.SemanticModel`,
   * `model.bim`, a TMDL folder, … (see FORMATS.md).
   */
  path: string
  /**
   * Offline `AzureStorage.Blobs` mapping (`blob-map.json`: blob URLs →
   * local files). Default: the workspace's blob map.
   */
  blobMap?: string | null
  /** Default data source of refreshes (default `auto`). */
  source?: SourceParam | null
  /**
   * Decompress the PBIX `DataModel` / PBIP cache to read the model schema
   * (default `true`).
   */
  loadPbixSchema?: boolean | null
  /** Keep internal VertiPaq tables (`H$`, `R$`, `U$`) (default `false`). */
  includeSystemTables?: boolean
  /**
   * Data commands load the data on first use when nothing was loaded
   * (default `true`). Turn off to see empty tables until `data.refresh`.
   */
  autoRefresh?: boolean | null
  /**
   * The time M sees as "now" (RFC 3339), for reproducible "last refreshed"
   * values. Default: the system clock.
   */
  now?: string | null
}

/** Parameters of commands that only need a document. */
export interface DocParams {
  /** The document id returned by `doc.open` (`d1`, …). */
  docId: string
}

/** Summary of an open document. */
export interface DocSummary {
  /** The id to pass to other commands. */
  docId: string
  /** The path as given. */
  path: string
  /** Source format (`pbip`, `pbix`, `pbit`, `bim`, `tmdl`, …). */
  format: string
  /** Detected format, for people. */
  formatLabel: string
  hasModel: boolean
  modelName?: string | null
  /** Data comes from embedded VertiPaq storage (PBIX, ABF). */
  embeddedData: boolean
  /** A PBIP local data cache (`.pbi/cache.abf`) is available. */
  hasCache: boolean
  /** A blob map is configured. */
  blobMap: boolean
  /** Default refresh source: `auto`, `query` or `cache`. */
  source: string
  autoRefresh: boolean
  /** Data was loaded. */
  loaded: boolean
  /** Data generation (changes after every data change). */
  generation: number
  tables: number
  measures: number
  relationships: number
  reports: ReportBrief[]
  /** Reader diagnostics. */
  diagnostics: DiagnosticCounts
}

/** Result of a document edit or history operation. */
export interface DocumentEditOut {
  changed: boolean
  revision: number
  dirty: boolean
  changedObjectIds: number[]
  /**
   * The calculated tables and columns the edit recomputed over the
   * loaded data, in order (absent when none); a failed one has no values
   * until an edit or a refresh computes it again.
   */
  recalculated?: RecalculatedOut[]
  /**
   * `doc.applyEdits`: problems that did not stop the edits (a measure's
   * DAX that does not parse); absent when none.
   */
  warnings?: string[]
}

export interface DocumentHistoryOut {
  revision: number
  dirty: boolean
  canUndo: boolean
  canRedo: boolean
  undoLabel?: string | null
  redoLabel?: string | null
}

export interface DocumentParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
}

export interface DocumentSaveAsOut {
  /** The new project folder. */
  projectDir: string
  /** The project's `.pbip` file, relative to `projectDir`. */
  pbipFile: string
  /** Files written, relative to `projectDir`. */
  files: string[]
  bytes: number
  /** The format the document had before (`pbix`, `pbit`, `pbip`, …). */
  previousFormat: string
  /** The revision saved. */
  revision: number
  /** Whether the document has edits after the save (always `false`). */
  dirty: boolean
  /**
   * What the new project does not carry over (imported data, a local
   * data cache).
   */
  warnings: string[]
}

export interface DocumentSaveAsParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /**
   * The new project folder (created if missing; must be empty unless
   * `overwrite`).
   */
  outDir: string
  /**
   * Write into a non-empty folder (files with the same paths are
   * replaced, others kept).
   */
  overwrite?: boolean
}

export interface DocumentSaveOut {
  /** The project folder. */
  projectDir: string
  /** Files written, relative to the project folder. */
  written: string[]
  /** Files deleted (e.g. the `.tmdl` file of a renamed table). */
  deleted: string[]
  /**
   * Files changed on disk since the open or last save (only with
   * `overwrite`).
   */
  overwritten: FileChangeOut[]
  /** The revision saved. */
  revision: number
  /** Whether the document has edits after the save (always `false`). */
  dirty: boolean
}

export interface DocumentSaveParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /**
   * Save even if project files changed on disk since the open or last
   * save: files DashCraft writes get the document's version, other
   * files are kept. Without it such a save fails with `conflict`.
   */
  overwrite?: boolean
}

/** One reversible edit accepted by `doc.applyEdits`. */
export type EditOperation = {
  type: 'setMeasureExpression'
  measureId: number
  expression: string
} | {
  type: 'setDescription'
  objectId: number
  /** `null` clears the description. */
  text?: string | null
} | {
  type: 'renameObject'
  objectId?: number | null
  table?: string | null
  column?: string | null
  measure?: string | null
  expression?: string | null
  newName: string
} | {
  type: 'setVisualPosition'
  report?: string | null
  page?: string | null
  visual: string
  x?: number | null
  y?: number | null
  width?: number | null
  height?: number | null
} | {
  type: 'createMeasure'
  table: string
  name: string
  expression: string
  formatString?: string | null
  displayFolder?: string | null
  description?: string | null
  isHidden?: boolean
} | {
  type: 'updateMeasure'
  measureId?: number | null
  table?: string | null
  measure?: string | null
  expression?: string | null
  formatString?: string | null
  displayFolder?: string | null
  description?: string | null
  isHidden?: boolean | null
} | {
  type: 'deleteMeasure'
  measureId?: number | null
  table?: string | null
  measure?: string | null
} | {
  type: 'createRelationship'
  from: ColumnParam
  to: ColumnParam
  fromCardinality?: CardinalityParam | null
  toCardinality?: CardinalityParam | null
  crossFilteringBehavior?: CrossFilterParam | null
  isActive?: boolean | null
  name?: string | null
} | {
  type: 'updateRelationship'
  relationshipId?: number | null
  name?: string | null
  fromCardinality?: CardinalityParam | null
  toCardinality?: CardinalityParam | null
  crossFilteringBehavior?: CrossFilterParam | null
  securityFilteringBehavior?: SecurityFilterParam | null
  isActive?: boolean | null
} | {
  type: 'deleteRelationship'
  relationshipId?: number | null
  name?: string | null
} | {
  type: 'createColumn'
  table: string
  name: string
  expression: string
  dataType?: DataTypeParam | null
  formatString?: string | null
  displayFolder?: string | null
  description?: string | null
  isHidden?: boolean
} | {
  type: 'updateColumn'
  columnId?: number | null
  table?: string | null
  column?: string | null
  expression?: string | null
  dataType?: DataTypeParam | null
  formatString?: string | null
  displayFolder?: string | null
  description?: string | null
  isHidden?: boolean | null
} | {
  type: 'deleteColumn'
  columnId?: number | null
  table?: string | null
  column?: string | null
} | {
  type: 'createTable'
  name: string
  expression: string
  description?: string | null
  isHidden?: boolean
} | {
  type: 'updateTable'
  tableId?: number | null
  table?: string | null
  expression?: string | null
  description?: string | null
  isHidden?: boolean | null
} | {
  type: 'deleteTable'
  tableId?: number | null
  table?: string | null
} | {
  type: 'createHierarchy'
  table: string
  name: string
  levels: LevelParam[]
  displayFolder?: string | null
  description?: string | null
  isHidden?: boolean
} | {
  type: 'deleteHierarchy'
  hierarchyId?: number | null
  table?: string | null
  hierarchy?: string | null
}

/** [`CapabilityLimits`] with the defaults applied. */
export interface EffectiveLimits {
  maxTokens: number
  temperature: number
  timeoutSecs: number
  maxInputChars: number
}

export interface EvaluateMeasureOut {
  /** Semantic document revision captured with the query engine snapshot. */
  documentRevision: number
  measure: MeasureValueOut
  /** The filters applied, and those that could not be. */
  filters: FilterSpecOut
}

export interface EvaluateMeasureParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** `Name`, `[Name]` or `Table[Name]`. */
  measure: string
  /** Apply this report's report-level filters. */
  report?: string | null
  /** Also apply this page's filters (of `report`, default the first). */
  page?: string | null
  /** Extra column filters (AND). */
  filters?: ColumnFilterParam[]
}

/** One example of a command. */
export interface ExampleInfo {
  title: string
  /** Example parameters. */
  params: JsonValue
}

export interface ExplainMeasureOut {
  measure: string
  table: string
  /** The measure's DAX. */
  expression: string
  formatString?: string | null
  /** One or two sentences. */
  summary: string
  /** The explanation (Markdown). */
  explanation: string
  /** Filter context and context-transition notes. */
  filterContext: string[]
  /** The model's note per dependency. */
  dependencies: DependencyNote[]
  caveats: string[]
  /** What the measure references (computed locally). */
  references: MeasureReferences
  /** Everything above as one Markdown document. */
  markdown: string
  call: CallInfo
}

export interface ExplainMeasureParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** `Name`, `[Name]` or `Table[Name]`. */
  measure: string
  /** The measure's table (when names repeat). */
  table?: string | null
  /** Provider id (default: the default provider). */
  provider?: string | null
}

export interface ExportModelOut {
  format: ModelFormat
  /** The written file. */
  path?: string | null
  /** Size of the documentation. */
  bytes: number
  /** The model doc (inline JSON; shape in docs/CLI.md, `export --model`). */
  json?: JsonValue
  /** The data dictionary (inline Markdown). */
  markdown?: string | null
}

export interface ExportModelParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  format: ModelFormat
  /** Write to this file; without it the documentation is returned inline. */
  path?: string | null
  /** Include DAX and M (default `true`). */
  includeExpressions?: boolean | null
  /** Include hidden objects (default `true`). */
  includeHidden?: boolean | null
  /** Markdown title (default: the file or project name). */
  title?: string | null
  /** Replace an existing file. */
  overwrite?: boolean
}

export interface ExportTableOut {
  /** Table name (model spelling). */
  table: string
  format: TableFormat
  path: string
  /** Data rows (without a CSV header). */
  rows: number
  columns: number
  /** File size. */
  bytes: number
}

export interface ExportTableParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Table name (case-insensitive). */
  table: string
  format: TableFormat
  /** The output file. */
  path: string
  /**
   * CSV: values formatted with each column's format string (as Power BI
   * shows them) instead of raw values.
   */
  formatted?: boolean
  /**
   * CSV: prefix text cells starting with `=`, `+`, `-`, `@`, TAB or CR
   * with `'` (spreadsheet formula-injection guard).
   */
  excelSafe?: boolean
  /** CSV: start with a UTF-8 BOM. */
  bom?: boolean
  /** CSV: field delimiter (default `,`). */
  delimiter?: string | null
  /** Parquet: compression (default `zstd`). */
  compression?: CompressionParam
  /** Replace an existing file. */
  overwrite?: boolean
}

export interface ExpressionOut {
  name: string
  /** `m` or `dax`. */
  kind: string
  /** Power Query parameter (`IsParameterQuery=true`). */
  isParameter: boolean
  /** Parameter type from the metadata record (`Text`, `Number`, …). */
  parameterType?: string | null
  description?: string | null
  queryGroup?: string | null
  /**
   * Expression text without the `meta [...]` record; for parameters the
   * value (omitted with `--no-expressions`).
   */
  expression?: string | null
}

/** A field bound to a visual data role. */
export interface FieldOut {
  /** Data role, e.g. `Values`, `Category`, `Y`. */
  role: string
  /**
   * `column`, `measure`, `aggregation`, `hierarchyLevel`, `literal` or
   * `unknown`.
   */
  kind: string
  /** Table (for aggregations, of the aggregated field). */
  table?: string | null
  /** Column, measure or hierarchy-level name. */
  name?: string | null
  /**
   * `aggregation`: `sum`, `avg`, `distinctCount`, `min`, `max`,
   * `countNonNull`, … (or the PBIR code as text).
   */
  aggregation?: string | null
  /** `hierarchyLevel`: the hierarchy name. */
  hierarchy?: string | null
  /**
   * DAX-style reference, e.g. `'Web Sessions'[Device]`,
   * `Sum(Orders[Quantity])`.
   */
  ref: string
  queryRef: string
  nativeQueryRef?: string | null
  /** User-renamed label shown instead of the field name. */
  displayName?: string | null
}

/** A field reference (column, measure, aggregation, hierarchy level, …). */
export interface FieldRefOut {
  /**
   * `column`, `measure`, `aggregation`, `hierarchyLevel`, `literal` or
   * `unknown`.
   */
  kind: string
  /** Table (for aggregations, of the aggregated field). */
  table?: string | null
  /** Column, measure or hierarchy-level name. */
  name?: string | null
  /**
   * `aggregation`: `sum`, `avg`, `distinctCount`, `min`, `max`,
   * `countNonNull`, … (or the PBIR code as text).
   */
  aggregation?: string | null
  /** `hierarchyLevel`: the hierarchy name. */
  hierarchy?: string | null
  /**
   * DAX-style reference, e.g. `'Web Sessions'[Device]`,
   * `Sum(Orders[Quantity])`.
   */
  ref: string
}

/** How a project file changed on disk. */
export type FileChangeKind = 'added' | 'modified' | 'removed'

/** A project file changed on disk by another program. */
export interface FileChangeOut {
  /** Path relative to the project folder. */
  path: string
  change: FileChangeKind
}

export interface FilterOut {
  name: string
  displayName?: string | null
  /** Filter type: `Categorical`, `Advanced`, `TopN`, `RelativeDate`, … */
  kind: string
  /** The field shown in the filter pane. */
  field?: FieldRefOut | null
  /**
   * Condition summary, e.g. `Stores[Channel] IN ('Retail')`;
   * omitted when the filter card is not filtering.
   */
  condition?: string | null
  isHiddenInViewMode: boolean
  isLockedInViewMode: boolean
  howCreated?: string | null
}

/** A filter specification: column filters (AND) plus what was not applied. */
export interface FilterSpecOut {
  filters: ColumnFilterOut[]
  unsupported: UnsupportedFilterOut[]
}

/** One finding. */
export interface FindingOut {
  /** Rule id, e.g. `LINT002` (see `crates/pbi-core/src/lint/RULES.md`). */
  rule: string
  /** `error`, `warning` or `info`. */
  severity: string
  message: string
  /** Source file (container-relative) plus JSON pointer or line. */
  origin?: OriginOut | null
  /**
   * The offending object's id (visual, measure, relationship, …), as in
   * `model.dependencies` and the edit commands.
   */
  object?: number | null
  /** A fix hint. */
  suggestion?: string | null
}

export interface GenerateMeasureOut {
  /** Suggested measure name. */
  name: string
  /** Suggested home table. */
  table: string
  /** The DAX expression. */
  expression: string
  formatString?: string | null
  /** How it works (Markdown). */
  explanation: string
  assumptions: string[]
  /** The expression parses as DAX. */
  parsed: boolean
  /** Parse errors and unknown references. */
  diagnostics: SuggestionDiagnostic[]
  /** With `evaluate`: the value (unfiltered). */
  value?: SuggestionValue | null
  /** Always `false`: suggestions are never written to the model. */
  applied: boolean
  call: CallInfo
}

export interface GenerateMeasureParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** What the measure should compute (at most 4000 characters). */
  prompt: string
  /** Home table of the new measure. */
  table?: string | null
  /** Provider id (default: the default provider). */
  provider?: string | null
  /**
   * Evaluate the suggestion locally (unfiltered) when it parses; loads the
   * data on first use. Nothing is written to the model.
   */
  evaluate?: boolean
  /**
   * Include this many rows of `table` (needs `sampleRows` in the settings
   * and a `sample` consent).
   */
  sampleRows?: number | null
}

/** One header of a preview. */
export interface HeaderPreview {
  name: string
  /** The value, with keys replaced by `[redacted]`. */
  value: string
}

/** Result of `model.hierarchy.create`. */
export interface HierarchyCreateOut {
  changed: boolean
  revision: number
  dirty: boolean
  changedObjectIds: number[]
  hierarchyId: number
  table: string
  name: string
  /** The levels' ids, top level first. */
  levelIds: number[]
}

export interface HierarchyOut {
  name: string
  isHidden: boolean
  description?: string | null
  levels: LevelOut[]
}

/** A hit region: the `data-hit` id of an SVG group and its bounding box. */
export interface HitRegionOut {
  /** Hit id, e.g. `<visual>|slicerItem|2`; pass it to `canvas.click`. */
  id: string
  rect: RectOut
}

/** An [`HttpRequest`] with its secrets redacted (`ai.preview`). */
export interface HttpPreview {
  method: string
  url: string
  headers: HeaderPreview[]
  /** The exact JSON body. */
  body: JsonValue
  /** Size of the body as sent, in bytes. */
  bodyBytes: number
}

/** Root object of `inspect --json`. */
export interface Inspect {
  schemaVersion: number
  source: SourceOut
  modelStatus: ModelStatusOut
  /** The document's semantic model; `null` if none is available locally. */
  model: ModelOut | null
  /** Other semantic models of a multi-model project. */
  extraModels: ModelOut[]
  reports: ReportOut[]
  diagnostics: DiagnosticOut[]
  counts: Counts
  /** VertiPaq Analyzer statistics (`.vpax`). */
  vpaxStats?: VpaxStatsOut | null
  /** Paginated report outline (`.rdl`). */
  paginatedReport?: PaginatedReportOut | null
  /** Power Query details (`.pq`, `.pqt`, dataflows). */
  mashup?: MashupOut | null
  /** Custom visual metadata (`.pbiviz`; never executed). */
  customVisual?: CustomVisualOut | null
  /** Custom connector listing (`.mez`/`.pqx`; never executed). */
  connector?: ConnectorOut | null
}

/**
 * Structure found while detecting (paths relative to the input folder,
 * the `.pbip` folder or the archive root).
 */
export interface ItemsOut {
  pbipFile?: string | null
  semanticModels: ModelItemOut[]
  reports: ReportItemOut[]
  otherItems: OtherItemOut[]
}

/** Where an API key came from. */
export type KeySource = 'notRequired' | 'keychain' | 'environment' | 'missing'

export interface LevelOut {
  name: string
  column: string
}

/** A hierarchy level: a column of the hierarchy's table. */
export interface LevelParam {
  /** The level's column. */
  column: string
  /** The level name; the column name when omitted. */
  name?: string | null
}

/** The `lint --json` report (`schemaVersion` 1) plus `ok`. */
export interface LintOut {
  schemaVersion: number
  /** The document path as given. */
  path: string
  /** No finding at or above `deny` and no reader error. */
  ok: boolean
  summary: LintSummaryOut
  /** Findings sorted by file, location and rule. */
  findings: FindingOut[]
  /** Reader errors (checks that could not run). */
  readerErrors: DiagnosticOut[]
}

/** Counts per severity and rule. */
export interface LintSummaryOut {
  errors: number
  warnings: number
  infos: number
  /** Findings per rule id. */
  byRule: { [key: string]: number }
  /** Checks that were skipped and why. */
  notes?: string[]
}

/** One M expression a query rename rewrites. */
export interface MRewriteOut {
  /**
   * The object holding the expression, with the names after the rename:
   * `table 'Orders'` (`table 'Orders' partition 'P'` when the table has
   * several partitions) or `expression 'Clean Key'`.
   */
  location: string
  /** `source` (a partition) or `expression` (a shared expression). */
  property: string
  before: string
  after: string
}

export interface MashupConnectionOut {
  kind?: string | null
  path?: string | null
}

export interface MashupOut {
  /** `script`, `template`, `dataflowGen2` or `dataflowCdm`. */
  kind: string
  name?: string | null
  description?: string | null
  culture?: string | null
  section?: string | null
  queries: MashupQueryOut[]
  entities: DataflowEntityOut[]
  connections: MashupConnectionOut[]
}

export interface MashupQueryOut {
  name: string
  group?: string | null
  loadEnabled?: boolean | null
  hidden?: boolean | null
  entity: boolean
  syntaxError: boolean
}

/** A measure definition. */
export interface MeasureDefOut {
  /** Stable document object id for edits. */
  id: number
  name: string
  /** Home table. */
  table: string
  /** DAX expression. */
  expression: string
  formatString?: string | null
  description?: string | null
  displayFolder?: string | null
  isHidden: boolean
}

/**
 * Result of `model.measure.create`, `model.measure.update` and
 * `model.measure.delete`.
 */
export interface MeasureEditOut {
  changed: boolean
  revision: number
  dirty: boolean
  changedObjectIds: number[]
  /**
   * The calculated tables and columns the edit recomputed over the
   * loaded data, in order (absent when none); a failed one has no values
   * until an edit or a refresh computes it again.
   */
  recalculated?: RecalculatedOut[]
  measureId: number
  /** The measure's table. */
  table: string
  /** The measure's name. */
  name: string
  /**
   * `model.measure.delete` only: what refers to the measure (and breaks
   * once it is deleted).
   */
  dependents?: DependentOut[] | null
  /** `model.measure.delete` only: whether this was a dry run. */
  dryRun?: boolean | null
  /**
   * Problems that do not stop the edit (DAX that does not parse,
   * references that cannot be confirmed).
   */
  warnings: string[]
}

export interface MeasureOut {
  name: string
  /** DAX text, verbatim (omitted with `--no-expressions`). */
  expression?: string | null
  formatString?: string | null
  description?: string | null
  isHidden: boolean
  displayFolder?: string | null
}

/** What a measure references (local analysis, not the model's claims). */
export interface MeasureReferences {
  /** Measures reached, transitively (`Table[Measure]`). */
  measures: string[]
  /** Columns with their data types. */
  columns: string[]
  tables: string[]
  /** Relationships touching those tables (`from → to (cardinality, filter)`). */
  relationships: string[]
}

/** The value of one measure. */
export interface MeasureValueOut {
  /** Home table. */
  table: string
  name: string
  /**
   * Raw value: number, string, boolean, ISO date/time text, or `null`
   * (BLANK, or an error).
   */
  value: JsonValue
  /**
   * The value formatted with the format string (`""` for BLANK); absent
   * on error.
   */
  formatted?: string | null
  /** What to show: the formatted value, or `⚠ <error>`. */
  display: string
  formatString?: string | null
  /** Why the measure could not be evaluated. */
  error?: string | null
  /** The error is a DAX feature DashCraft does not implement yet. */
  unsupported: boolean
  /** The measure's DAX expression. */
  dax: string
  hidden: boolean
  displayFolder?: string | null
}

export interface MeasuresValuesOut {
  /** Semantic document revision captured with the query engine snapshot. */
  documentRevision: number
  /** Every measure evaluated without error. */
  ok: boolean
  /** Report name. */
  report?: string | null
  /** Page display name. */
  page?: string | null
  /** Filters applied, and those that could not be (Top N, relative date…). */
  filters: FilterSpecOut
  /**
   * What the values were computed on (tables loaded, failed, never
   * loaded).
   */
  data: RefreshCounts
  /**
   * Every measure, in model order; failures carry `error` and `⚠` in
   * `display`, with their DAX.
   */
  measures: MeasureValueOut[]
}

export interface MeasuresValuesParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Evaluate under this report's report-level filters. */
  report?: string | null
  /** Also apply this page's filters. */
  page?: string | null
  /** Extra column filters (AND). */
  filters?: ColumnFilterParam[]
}

/** A visual moved or resized with its group. */
export interface MemberPositionOut {
  visual: string
  position: PositionOut
}

/** Model documentation format. */
export type ModelFormat = 'json' | 'markdown'

export interface ModelItemOut {
  path: string
  /** `tmdl`, `bim` or `unknown`. */
  flavour: string
  hasCacheAbf: boolean
  displayName?: string | null
}

export interface ModelMeasuresOut {
  measures: MeasureDefOut[]
}

export interface ModelMeasuresParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Only the measures of this table. */
  table?: string | null
}

export interface ModelOut {
  name: string
  description?: string | null
  culture?: string | null
  compatibilityLevel?: number | null
  /** Where the model definition was read from. */
  source?: ModelSourceOut | null
  tables: TableOut[]
  relationships: RelationshipOut[]
  /** Named expressions: M queries and parameters (`isParameter`). */
  expressions: ExpressionOut[]
  roles: RoleOut[]
  /** PBIP `.pbi/cache.abf` of this model folder (schema summary). */
  cacheAbf?: CacheAbfOut | null
}

export interface ModelParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Include DAX and M text (default `true`). */
  includeExpressions?: boolean | null
}

/**
 * A relationship with its stable id (for `model.relationship.delete` and
 * `model.dependencies`).
 */
export interface ModelRelationshipOut {
  relationshipId: number
  name: string
  /** The many side (TOM `fromColumn`). */
  from: ColumnRefOut
  /** The one side (TOM `toColumn`). */
  to: ColumnRefOut
  /** `many` or `one`. */
  fromCardinality: string
  /** `many` or `one`. */
  toCardinality: string
  /** `oneDirection`, `bothDirections` or `automatic`. */
  crossFilteringBehavior: string
  isActive: boolean
  /** `oneDirection`, `bothDirections` or `none`. */
  securityFilteringBehavior: string
}

export interface ModelRelationshipsOut {
  relationships: ModelRelationshipOut[]
}

export interface ModelSourceOut {
  /** `tmdl`, `tmsl` or `abf`. */
  kind: string
  /** Model folder relative to the opened folder/archive (`""` = root). */
  folder: string
}

export interface ModelStatusOut {
  /** `local`, `remote` (e.g. `byConnection`) or `missing`. */
  status: string
  description?: string | null
}

/** A model table with its loaded data. */
export interface ModelTableOut {
  name: string
  description?: string | null
  isHidden: boolean
  /** `regular`, `calculatedTable`, `calculationGroup`, `remote`. */
  kind: string
  dataCategory?: string | null
  columns: ColumnOut[]
  /** Measure names (`model.measures` has their DAX). */
  measures: string[]
  partitions: PartitionOut[]
  hierarchies: HierarchyOut[]
  /** Rows loaded (0 before a refresh). */
  rows: number
  /** Data has been loaded. */
  loaded: boolean
}

export interface ModelTablesOut {
  tables: ModelTableOut[]
}

export interface MoveVisualParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report. */
  report?: string | null
  /** Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. */
  page?: string | null
  /** The visual's name (see `report.visuals`). */
  visual: string
  /** New left edge (page units, as in `report.visuals`); unchanged when absent. */
  x?: number | null
  /** New top edge; unchanged when absent. */
  y?: number | null
}

export type NoParams = { [key: string]: never }

/**
 * Result of `model.column.delete`, `model.table.delete` and
 * `model.hierarchy.delete`.
 */
export interface ObjectDeleteOut {
  changed: boolean
  revision: number
  dirty: boolean
  changedObjectIds: number[]
  /**
   * The calculated tables and columns the edit recomputed over the
   * loaded data, in order (absent when none); a failed one has no values
   * until an edit or a refresh computes it again.
   */
  recalculated?: RecalculatedOut[]
  objectId: number
  /** `column`, `table` or `hierarchy`. */
  kind: string
  /** The table, or the column's or hierarchy's table. */
  table: string
  name: string
  dryRun: boolean
  /**
   * What refers to the object and breaks once it is deleted (DAX,
   * report fields). A table's own measures, columns and expressions go
   * with it and are not listed.
   */
  dependents: DependentOut[]
  /**
   * `model.table.delete`: what is deleted with the table (relationships
   * that join it, role row filters on it).
   */
  removed: string[]
  /**
   * References that cannot be confirmed (DAX that does not parse,
   * ambiguous unqualified names, M queries naming a table).
   */
  warnings: string[]
}

/**
 * The error of a command: a stable [`code`](OpErrorCode), a user-facing
 * message and optional structured details.
 */
export interface OpError {
  code: OpErrorCode
  message: string
  /**
   * Structured details (e.g. the failing DAX, the candidates of an
   * ambiguous name).
   */
  data?: JsonValue
}

/**
 * Stable error codes. The string form (`invalidParams`, …) is part of the
 * command contract: it never changes for an existing code; new codes may
 * be added.
 */
export type OpErrorCode = 'invalidParams' | 'unknownCommand' | 'notFound' | 'noDocument' | 'unsupported' | 'pathDenied' | 'io' | 'query' | 'cancelled' | 'busy' | 'internal' | 'conflict'

export interface OriginOut {
  /** File relative to the opened folder/archive. */
  path: string
  line?: number | null
  /** JSON pointer inside the file. */
  pointer?: string | null
}

export interface OtherItemOut {
  path: string
  itemType: string
}

/** A page of a report. */
export interface PageInfoOut {
  /** 1-based position. */
  index: number
  /** Internal name. */
  name: string
  displayName: string
  width: number
  height: number
  isHidden: boolean
  /** The report opens on this page. */
  active: boolean
  visuals: number
  /** Page-level filters. */
  filters: number
}

export interface PageOut {
  /** Internal name (PBIR folder name / ID). */
  name: string
  displayName: string
  width: number
  height: number
  /** `FitToPage`, `FitToWidth` or `ActualSize`. */
  displayOption: string
  isHidden: boolean
  filters: FilterOut[]
  visuals: VisualOut[]
}

export interface PageParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Report (default: the first). */
  report?: string | null
  /**
   * Page: internal name, display name, 1-based index or a unique part of
   * the display name. Default: the active page.
   */
  page?: string | null
}

export interface PaginatedReportOut {
  /** RDL schema, e.g. `2016/01`. */
  schema?: string | null
  description?: string | null
  author?: string | null
  dataSources: RdlDataSourceOut[]
  datasets: RdlDataSetOut[]
  parameters: RdlParameterOut[]
  sections: RdlSectionOut[]
  embeddedImages: string[]
  itemCount: number
  truncated: boolean
}

export interface ParsePeopleOut {
  /**
   * `names` (a people list), `empty` (blank or an empty list) or
   * `unparsed` (not a people list; `display` is the trimmed text).
   */
  outcome: string
  /** `First Last` per person. */
  names: string[]
  /** What the "Parse people field" column shows: names joined with `; `. */
  display: string | null
}

export interface ParsePeopleParams {
  /**
   * One exported people-picker cell: a Python-style list of dicts, e.g.
   * `[{'firstName': 'Ada', 'lastName': 'Lovelace'}]`.
   */
  text: string
}

export interface PartitionOut {
  name: string
  /**
   * Source kind: `m`, `calculated`, `entity`, `query`, or another TMDL
   * source keyword.
   */
  kind: string
  /** `import`, `directQuery`, `dual`, `directLake` or `default`. */
  mode: string
  /** M, DAX or native query text (omitted with `--no-expressions`). */
  expression?: string | null
  queryGroup?: string | null
  /** Entity partitions: the remote entity name. */
  entityName?: string | null
  /** Entity partitions: the named expression holding the source. */
  expressionSource?: string | null
}

export interface PathParams {
  /** File or folder. */
  path: string
}

export interface PeoplePreviewOut {
  /** Table name (model spelling). */
  table: string
  /** Column name (stored spelling). */
  column: string
  /** The name `data.addDerivedColumn` should use: `<column> (parsed)`. */
  newColumn: string
  rows: number
  /** Cells parsed into names. */
  names: number
  /** Non-empty cells that are not a people list (shown trimmed). */
  unparsed: number
  /** Blank cells and empty lists. */
  empty: number
  /** Distinct non-empty values in storage order, with their parsed form. */
  samples: PeopleSampleOut[]
}

/** One distinct value of a people preview. */
export interface PeopleSampleOut {
  /** The cell text (cut at 2000 characters). */
  raw: string
  /** What the parsed column shows (`First Last; …`, or the trimmed text). */
  parsed: string | null
  /** `names` or `unparsed`. */
  outcome: string
}

export interface PositionOut {
  x: number
  y: number
  z: number
  width: number
  height: number
  tabOrder?: number | null
}

export interface PreviewPeopleParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Table name (case-insensitive). */
  table: string
  /** A text column of the table (case-insensitive). */
  column: string
  /** Distinct samples returned (default 50, at most 200). */
  limit?: number
}

/** The kind of a provider (which adapter talks to it). */
export type ProviderKind = 'openAiCompatible' | 'azureOpenAi' | 'ollama' | 'mock'

/** One configured provider. Never holds a key. */
export interface ProviderSettings {
  /**
   * Stable id (`[a-z0-9][a-z0-9_-]*`, at most 64): consents and the
   * keychain account (`ai:<id>`) refer to it.
   */
  id: string
  kind: ProviderKind
  /**
   * Base URL. Defaults: OpenAI-compatible `https://api.openai.com/v1`,
   * Ollama `http://127.0.0.1:11434`; required for Azure
   * (`https://<resource>.openai.azure.com`). Must be `https` unless the
   * host is this machine or `allowHttp` is set.
   */
  endpoint?: string | null
  /** Model name (OpenAI-compatible, Ollama). */
  model?: string | null
  /** Azure deployment name. */
  deployment?: string | null
  /** Azure `api-version` (default `2024-10-21`). */
  apiVersion?: string | null
  /** How to ask for JSON (default `jsonSchema`). */
  structuredOutput?: StructuredOutput | null
  /**
   * Allow plain `http` to a host that is not this machine (e.g. a LAN
   * vLLM server). Requests and keys then travel unencrypted.
   */
  allowHttp?: boolean
  /** Display name. */
  label?: string | null
}

/** A configured provider (never its key). */
export interface ProviderStatus {
  id: string
  kind: ProviderKind
  label?: string | null
  /** Base URL (`null` if invalid). */
  endpoint?: string | null
  /** `host[:port]`: what consents bind to. */
  endpointHost?: string | null
  /** The endpoint is this machine (data does not leave it). */
  local: boolean
  model?: string | null
  /** Where its API key would come from (never the key). */
  apiKey: KeySource
  isDefault: boolean
  /** Capabilities with a consent for this provider and endpoint. */
  consented: Capability[]
}

export interface RdlDataSetOut {
  name: string
  dataSource?: string | null
  commandType?: string | null
  /** Query text (omitted with `--no-expressions`). */
  commandText?: string | null
  sharedReference?: string | null
  fields: RdlFieldOut[]
  queryParameters: RdlQueryParameterOut[]
  filters: number
}

export interface RdlDataSourceOut {
  name: string
  dataProvider?: string | null
  /** Always `<redacted>` when the source embeds one. */
  connectionString?: string | null
  sharedReference?: string | null
  integratedSecurity?: boolean | null
}

export interface RdlFieldOut {
  name: string
  dataField?: string | null
  /** Calculated field expression (omitted with `--no-expressions`). */
  value?: string | null
  typeName?: string | null
}

export interface RdlItemOut {
  /** `tablix`, `textbox`, `chart`, `image`, `rectangle`, `subreport`, … */
  kind: string
  name: string
  dataset?: string | null
  /** Textbox text (omitted with `--no-expressions`). */
  value?: string | null
  reportName?: string | null
  children: RdlItemOut[]
}

export interface RdlParameterOut {
  name: string
  dataType?: string | null
  prompt?: string | null
  nullable: boolean
  allowBlank: boolean
  multiValue: boolean
  hidden: boolean
  defaultValues: string[]
  defaultDataset?: string | null
  validValuesDataset?: string | null
  validValues: number
}

export interface RdlQueryParameterOut {
  name: string
  /** Omitted with `--no-expressions`. */
  value?: string | null
}

export interface RdlSectionOut {
  pageWidth?: string | null
  pageHeight?: string | null
  body: RdlItemOut[]
  header: RdlItemOut[]
  footer: RdlItemOut[]
}

/** A calculated table or column an edit recomputed over the loaded data. */
export interface RecalculatedOut {
  /** The calculated table's or column's id. */
  objectId: number
  table: string
  /** The calculated column; absent for a calculated table. */
  column?: string | null
  durationMs: number
  /**
   * A failed one has no values (the table is empty, the column has no
   * data) until an edit or a refresh computes it again.
   */
  status: StatusOut
}

/**
 * A rectangle in page units (canvas pixels at 100% zoom without a
 * viewport).
 */
export interface RectOut {
  x: number
  y: number
  width: number
  height: number
}

/** Table counts of a refresh (or of the data loaded so far). */
export interface RefreshCounts {
  /** Tables in scope. */
  tables: number
  /** Tables whose steps all succeeded. */
  ok: number
  /** Tables with a failed step (the table or a calculated column). */
  failed: number
  /** Tables not attempted (or, for data status, not loaded yet). */
  skipped: number
}

/** Result of `data.refresh` and `data.loadCsv`. */
export interface RefreshOut {
  /** Every table and calculated column in scope succeeded. */
  ok: boolean
  /**
   * The data source used: `auto`, `query` or `cache` (`manual` for CSV
   * loads).
   */
  source: string
  durationMs: number
  summary: RefreshCounts
  /** Tables in scope, in model order. */
  tables: TableRefreshOut[]
  /** Session derived columns after the run, with their status. */
  derived: DerivedColumnOut[]
  /** Refresh diagnostics (cycles, decoder notes, CSV conversion warnings). */
  diagnostics: DiagnosticOut[]
  /** The data generation after the run. */
  generation: number
}

/**
 * Result of `model.relationship.create`, `model.relationship.update` and
 * `model.relationship.delete`.
 */
export interface RelationshipEditOut {
  changed: boolean
  revision: number
  dirty: boolean
  changedObjectIds: number[]
  /**
   * The calculated tables and columns the edit recomputed over the
   * loaded data, in order (absent when none); a failed one has no values
   * until an edit or a refresh computes it again.
   */
  recalculated?: RecalculatedOut[]
  relationshipId: number
  /** The relationship's name (a GUID unless one was given). */
  name: string
  /** The from column (usually the many side). */
  from: ColumnParam
  /** The to column (usually the one side). */
  to: ColumnParam
  /** `many` or `one` (after the edit; before it for a delete). */
  fromCardinality: string
  /** `many` or `one`. */
  toCardinality: string
  /** `oneDirection`, `bothDirections` or `automatic`. */
  crossFilteringBehavior: string
  /** `oneDirection`, `bothDirections` or `none`. */
  securityFilteringBehavior: string
  isActive: boolean
  /**
   * `model.relationship.delete` only: the DAX whose `USERELATIONSHIP`
   * or `CROSSFILTER` calls name the relationship.
   */
  dependents?: DependentOut[] | null
  /** `model.relationship.delete` only: whether this was a dry run. */
  dryRun?: boolean | null
  /**
   * Problems that do not stop the edit: what the engine finds in the
   * loaded data (duplicate keys on the one side, differing key types,
   * ambiguous filter paths), references that cannot be confirmed.
   */
  warnings: string[]
}

export interface RelationshipOut {
  name: string
  /** The many side (TOM `fromColumn`). */
  from: ColumnRefOut
  /** The one side (TOM `toColumn`). */
  to: ColumnRefOut
  /** `many` or `one`. */
  fromCardinality: string
  /** `many` or `one`. */
  toCardinality: string
  /** `oneDirection`, `bothDirections` or `automatic`. */
  crossFilteringBehavior: string
  isActive: boolean
  /** `oneDirection`, `bothDirections` or `none`. */
  securityFilteringBehavior: string
}

/** Result of `model.object.rename`. */
export interface RenameObjectOut {
  changed: boolean
  revision: number
  dirty: boolean
  changedObjectIds: number[]
  /**
   * The calculated tables and columns the edit recomputed over the
   * loaded data, in order (absent when none); a failed one has no values
   * until an edit or a refresh computes it again.
   */
  recalculated?: RecalculatedOut[]
  /** The renamed object's id. */
  objectId: number
  /** `table`, `column`, `measure` or `expression`. */
  kind: string
  /**
   * The renamed table's old name, or the table of the column or measure
   * (empty for an expression).
   */
  table: string
  oldName: string
  newName: string
  dryRun: boolean
  /** DAX expressions rewritten (or, with `dryRun`, to be rewritten). */
  rewrites: DaxRewriteOut[]
  /**
   * M expressions rewritten (or to be rewritten) when a query is renamed:
   * a table with an M partition or a shared expression (absent when
   * none).
   */
  mRewrites?: MRewriteOut[]
  /**
   * References left as they are that may need a manual fix (DAX or M
   * that does not parse, ambiguous unqualified names, refresh policies).
   */
  warnings: string[]
}

export interface RenameObjectParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** The object's id (instead of `table`). */
  objectId?: number | null
  /** The table, or the table of `column` / `measure`. */
  table?: string | null
  /** A column of `table`. */
  column?: string | null
  /** A measure of `table`. */
  measure?: string | null
  /**
   * A shared expression (M query, parameter or function), instead of
   * `table`.
   */
  expression?: string | null
  /** The new name. */
  newName: string
  /** Only report what the rename would rewrite; the document is unchanged. */
  dryRun?: boolean
}

/** Output format. */
export type RenderFormat = 'svg' | 'png'

/** Result of `render.page` / `render.visual`. */
export interface RenderOut {
  /** No visual failed and the data loaded. */
  ok: boolean
  report: string
  page: RenderedPageInfo
  /** The rendered visual (`render.visual`). */
  visual?: string | null
  /** `svg` or `png`. */
  format: RenderFormat
  /** The SVG document (inline SVG renders). */
  svg?: string | null
  /** The PNG, base64 (inline PNG renders). */
  pngBase64?: string | null
  /** The written file (with `outputPath`). */
  outputPath?: string | null
  /** Size of the SVG or PNG in bytes. */
  bytes: number
  /** PNG size in pixels. */
  pixelWidth?: number | null
  pixelHeight?: number | null
  summary: VisualCounts
  /** Every visual of the page (or the one visual), in page order. */
  visuals: VisualStatusOut[]
  /** Slicer selections applied. */
  slicers: SlicerOut[]
  /** What the data looked like (failed or never-loaded tables). */
  data?: RefreshCounts | null
  /** Query + render + encode wall time. */
  durationMs: number
}

export interface RenderPageParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Report (default: the first). */
  report?: string | null
  /**
   * Page: internal name, display name, 1-based index or a unique part of
   * the display name (default: the active page).
   */
  page?: string | null
  /** `svg` (default) or `png`. */
  format?: RenderFormat
  /** PNG pixels per page unit (default 1; greater than 0, at most 16). */
  scale?: number | null
  /** Slicer selections replacing the saved ones. */
  slicers?: SlicerParam[]
  /** Write the output to this file instead of returning it inline. */
  outputPath?: string | null
  /** Replace an existing `outputPath`. */
  overwrite?: boolean
}

export interface RenderVisualParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report. */
  report?: string | null
  /** Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. */
  page?: string | null
  /** The visual's name (see `report.visuals`). */
  visual: string
  format?: RenderFormat
  scale?: number | null
  slicers?: SlicerParam[]
  outputPath?: string | null
  overwrite?: boolean
}

/** The rendered page. */
export interface RenderedPageInfo {
  /** 1-based position in the report. */
  index: number
  name: string
  displayName: string
  /** Page size in page units. */
  width: number
  height: number
}

/** A report of an open document. */
export interface ReportBrief {
  name: string
  pages: number
}

export interface ReportFiltersOut {
  report: string
  /** Page internal name, when a page was given. */
  page?: string | null
  /**
   * The filters as column filters (pass them to `measures.values` or
   * `dax.query`), and those that cannot be translated.
   */
  filters: FilterSpecOut
}

export interface ReportFiltersParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Report (default: the first). */
  report?: string | null
  /**
   * Also translate this page's filters: internal name, display name,
   * 1-based index or a unique part of the display name. Default: report
   * filters only.
   */
  page?: string | null
}

/** A report of the document. */
export interface ReportInfoOut {
  /** 1-based position. */
  index: number
  name: string
  datasetReference: DatasetReferenceOut
  baseTheme?: string | null
  customTheme?: string | null
  /** Internal name of the page shown first. */
  activePage?: string | null
  pages: number
  visuals: number
  /** Report-level filters. */
  filters: FilterOut[]
}

export interface ReportItemOut {
  path: string
  /** `pbir`, `pbirLegacy`, `legacyLayout` or `unknown`. */
  flavour: string
  /** `byPath`, `byConnection` or `unknown`. */
  dataset: string
  datasetPath?: string | null
  displayName?: string | null
}

export interface ReportListOut {
  reports: ReportInfoOut[]
}

export interface ReportOut {
  name: string
  datasetReference: DatasetReferenceOut
  baseTheme?: string | null
  customTheme?: string | null
  /** Internal name of the page shown on open. */
  activePage?: string | null
  /** Report-level filters (apply to every page). */
  filters: FilterOut[]
  /** Pages in display order. */
  pages: PageOut[]
}

export interface ReportPagesOut {
  report: string
  pages: PageInfoOut[]
}

export interface ReportParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /**
   * Report name (exact, case-insensitive, or a unique part; or 1-based
   * index). Default: the first report.
   */
  report?: string | null
}

export interface ReportVisualsOut {
  report: string
  /** Page internal name. */
  page: string
  pageDisplayName: string
  /** Visuals in page order: type, position, bound fields, filters. */
  visuals: VisualOut[]
}

export interface ResizeVisualParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Report: name (exact, case-insensitive, or a unique part) or 1-based index. Default: the first report. */
  report?: string | null
  /** Page: internal name, display name, 1-based index or a unique part of the display name. Default: the active page. */
  page?: string | null
  /** The visual's name (see `report.visuals`). */
  visual: string
  /** New width (page units, > 0); unchanged when absent. */
  width?: number | null
  /** New height (> 0); unchanged when absent. */
  height?: number | null
}

/** A column of a tabular result. */
export interface ResultColumnOut {
  name: string
  /**
   * Model data type (`string`, `int64`, `double`, `decimal`, `dateTime`,
   * `boolean`) or the Arrow type for computed columns.
   */
  dataType: string
  /** The model format string, when the column is a model column. */
  formatString?: string | null
}

/** Who wrote a chat message. */
export type Role = 'system' | 'user' | 'assistant'

export interface RoleOut {
  name: string
  description?: string | null
  modelPermission?: string | null
  tablePermissions: TablePermissionOut[]
}

/** Scroll position and range of a table / matrix body or slicer list. */
export interface ScrollOut {
  x: number
  y: number
  maxX: number
  maxY: number
  /** The scrolling area. */
  viewport: RectOut
}

/** Which way row-level security filters cross a relationship. */
export type SecurityFilterParam = 'oneDirection' | 'none' | 'bothDirections'

/** A security role of the model. */
export interface SecurityRoleOut {
  name: string
  description?: string | null
  /**
   * `read`, `readRefresh`, `administrator`, `none`, … (absent when the
   * model does not say).
   */
  modelPermission?: string | null
  /**
   * Table permissions with their DAX row filters (rows for which the
   * filter is TRUE are visible), in model order.
   */
  tablePermissions: TablePermissionOut[]
  /**
   * Members listed in the model (absent when it lists none: members are
   * usually assigned in the Power BI service).
   */
  members?: number | null
  /**
   * A filter calls `USERPRINCIPALNAME()`, `USERNAME()`, `USEROBJECTID()`
   * or `CUSTOMDATA()`: view it with a `userName` (Desktop's "Other
   * user").
   */
  dynamic: boolean
}

export interface SecurityRolesOut {
  /**
   * The model's roles, in model order (empty without row-level
   * security).
   */
  roles: SecurityRoleOut[]
  /** The current "View as" state. */
  viewAs: ViewAsOut
}

export interface SecurityViewAsOut {
  /** The view now in effect. */
  viewAs: ViewAsOut
  /** Visible rows per table, in model order. */
  tables: VisibleTableOut[]
  /**
   * Data generation after the change (bumped when the view changed:
   * cached results and renders are stale).
   */
  generation: number
}

export interface SecurityViewAsParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /**
   * Roles whose row filters apply (role names, case-insensitive); a row
   * is visible if any role allows it. Empty with no `userName` and no
   * `customData`: stop viewing as (all rows again).
   */
  roles: string[]
  /**
   * The effective user ("Other user"): what `USERPRINCIPALNAME()` and
   * `USERNAME()` return, for dynamic roles. Without roles it only sets
   * the user.
   */
  userName?: string | null
  /** What `CUSTOMDATA()` returns. */
  customData?: string | null
}

/** Kind of a Filters pane selection card. */
export type SelectionKind = 'slicer' | 'crossFilter'

/** A slicer selection or the cross-filter, as a Filters pane card. */
export interface SelectionOut {
  /** Slicer, or the cross-filter source visual. */
  visual: string
  kind: SelectionKind
  label: string
  /** `Table[Column]` of each field. */
  fields: string[]
  /** `is West or East`, `is not Online`, … */
  summary: string
}

export interface SetMeasureExpressionParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** Stable measure id returned by `model.measures`. */
  measureId: number
  /** The replacement DAX expression. */
  expression: string
}

export interface SetObjectDescriptionParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /**
   * The object's id (instead of `table`): any model object with a
   * description (tables, columns, measures, hierarchies, roles, …).
   */
  objectId?: number | null
  /** The table, or the table of `column` / `measure`. */
  table?: string | null
  /** A column of `table`. */
  column?: string | null
  /** A measure of `table`. */
  measure?: string | null
  /** The description; absent, `null` or empty clears it. */
  text?: string | null
}

export interface SettingsSetOut {
  settings: AiSettings
  /** Consents dropped because their provider or endpoint changed. */
  consentsDropped: number
  settingsPath?: string | null
}

/** Fields to change; absent fields keep their value. */
export interface SettingsSetParams {
  /** Turn AI on or off. */
  enabled?: boolean | null
  /**
   * Replace the provider list (keys are never part of it). Consents of
   * removed providers or changed endpoints are dropped.
   */
  providers?: ProviderSettings[] | null
  /** Default provider id (`""` clears it). */
  defaultProvider?: string | null
  /** Replace the per-capability limits. */
  limits?: { [key: string]: CapabilityLimits } | null
  /** Most sample rows per request (0 = none, at most 100). */
  sampleRows?: number | null
}

/** A diagnostic severity. */
export type SeverityParam = 'error' | 'warning' | 'info'

/** A slicer selection applied to the render. */
export interface SlicerOut {
  visual: string
  /** Selected values (`null` = BLANK); arrays for multi-field slicers. */
  values: JsonValue[]
  inverted: boolean
  /** A saved non-categorical condition (range, …) applies too. */
  condition: boolean
  /** `report` (saved) or `params`. */
  origin: string
}

/** A slicer selection for the render (replaces the saved selection). */
export interface SlicerParam {
  /** The slicer's visual name. */
  visual?: string | null
  /**
   * Or the slicer's field: display name, column, `Table[Column]`
   * (case-insensitive).
   */
  field?: string | null
  /**
   * Values to select (`null` = BLANK). Empty without `invert`: select
   * all (clears the slicer).
   */
  values?: JsonValue[]
  /** Select everything except `values`. */
  invert?: boolean
}

/** Sort order of `data.page`. */
export interface SortParam {
  /** Column name (case-insensitive). */
  column: string
  /** Default `true` (BLANKs first). */
  ascending?: boolean
}

export interface SourceOut {
  /** The path as given on the command line. */
  path: string
  /**
   * Document format: `pbip`, `pbix`, `pbit`, `semanticModelFolder`,
   * `reportFolder`, `bim`, `tmdlFolder`, …
   */
  format: string
  detection: DetectionOut
  /** PBIX/PBIT `Version`, e.g. `1.32`. */
  packageVersion?: string | null
}

/** Where a document's data comes from. */
export type SourceParam = 'auto' | 'query' | 'cache'

/** Outcome of one refresh step. */
export interface StatusOut {
  /** `ok`, `error` or `skipped`. */
  state: string
  /** Error message or skip reason. */
  message?: string | null
  /** Errors: an M or DAX feature DashCraft does not implement. */
  unsupported?: boolean | null
  /** Errors: what to do about it. */
  hint?: string | null
  /** Errors: the table can be loaded from a CSV instead (`data.loadCsv`). */
  csvFallback?: boolean | null
  /**
   * Errors: blob access is offline (no blob map configured); `true` or
   * absent.
   */
  offline?: boolean | null
}

/** How the provider is asked for JSON. */
export type StructuredOutput = 'jsonSchema' | 'jsonObject' | 'none'

/** A problem found locally in the suggestion. */
export interface SuggestionDiagnostic {
  /** `error` (does not parse) or `warning`. */
  severity: string
  message: string
  /** 1-based position in `expression`. */
  line?: number | null
  column?: number | null
  /** What the parser expected there. */
  expected?: string[]
}

/** The suggestion's value, evaluated locally. */
export interface SuggestionValue {
  /** Raw value (`null` for BLANK or on error). */
  value: JsonValue
  formatted?: string | null
  error?: string | null
}

/** Result of `model.table.create`. */
export interface TableCreateOut {
  changed: boolean
  revision: number
  dirty: boolean
  changedObjectIds: number[]
  /**
   * The calculated tables and columns the edit recomputed over the
   * loaded data, in order (absent when none); a failed one has no values
   * until an edit or a refresh computes it again.
   */
  recalculated?: RecalculatedOut[]
  tableId: number
  name: string
  /** The columns, in order, with the data types inferred from the values. */
  columns: CreatedTableColumnOut[]
}

/** Table export format. */
export type TableFormat = 'csv' | 'parquet'

export interface TableOut {
  name: string
  description?: string | null
  isHidden: boolean
  /** `regular`, `calculatedTable`, `calculationGroup` or `remote`. */
  kind: string
  dataCategory?: string | null
  columns: ColumnOut[]
  measures: MeasureOut[]
  partitions: PartitionOut[]
  hierarchies: HierarchyOut[]
}

export interface TablePermissionOut {
  table: string
  /** DAX row filter (omitted with `--no-expressions`). */
  filterExpression?: string | null
}

/** One table of a refresh. */
export interface TableRefreshOut {
  name: string
  /** `m`, `calculated`, `pbix`, `cache`, `manual` or `skipped`. */
  kind: string
  /** Rows after the step (the previous data if it failed). */
  rows: number
  durationMs: number
  status: StatusOut
  /** Calculated columns recomputed for this table. */
  columns: ColumnRefreshOut[]
}

/** Result of `model.table.update`. */
export interface TableUpdateOut {
  changed: boolean
  revision: number
  dirty: boolean
  changedObjectIds: number[]
  /**
   * The table and what depends on it, recomputed over the loaded data,
   * in order (absent when none); a failed one has no values until an
   * edit or a refresh computes it again.
   */
  recalculated?: RecalculatedOut[]
  tableId: number
  name: string
  /** The columns after the edit, in order. */
  columns: CreatedTableColumnOut[]
  /** The columns the new expression added (absent when none). */
  addedColumns?: string[]
  /**
   * The columns the new expression no longer returns, removed (absent
   * when none).
   */
  removedColumns?: string[]
}

/**
 * A cursor in a DAX text: `offset`, `utf16Offset`, or `line` with
 * `column`.
 */
export interface TextPositionIn {
  /** UTF-8 byte offset (rounded down to a character boundary). */
  offset?: number | null
  /** UTF-16 code-unit offset (JavaScript string index). */
  utf16Offset?: number | null
  /** 1-based line (lines end at `\n`); needs `column`. */
  line?: number | null
  /**
   * 1-based column in characters; past the end of the line means its
   * end.
   */
  column?: number | null
}

/** A position in the checked text. */
export interface TextPositionOut {
  /** UTF-8 byte offset. */
  offset: number
  /** UTF-16 code-unit offset (JavaScript string index). */
  utf16Offset: number
  /** 1-based line (lines end at `\n`). */
  line: number
  /** 1-based column, in characters. */
  column: number
}

/** One line of a data point tooltip. */
export interface TooltipLineOut {
  label: string
  value: string
}

export interface TooltipOut {
  /** Label/value lines; empty when the hit has no tooltip. */
  lines: TooltipLineOut[]
}

/** A filter that could not be applied. */
export interface UnsupportedFilterOut {
  name: string
  /** `Categorical`, `Advanced`, `TopN`, `Slicer`, `CrossFilter`, … */
  kind: string
  field?: string | null
  /** Why it was not applied. */
  reason: string
}

export interface UpdateColumnParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** The column's id (instead of `table` and `column`). */
  columnId?: number | null
  /** The column's table. */
  table?: string | null
  /** The column's name. */
  column?: string | null
  /**
   * A calculated column's new DAX expression. Without `dataType` the data
   * type is inferred again from its values (after the automatic first
   * load; kept when the table has no rows loaded or every value is
   * blank).
   */
  expression?: string | null
  /** A calculated column's data type. */
  dataType?: DataTypeParam | null
  /** The format string; empty clears it. */
  formatString?: string | null
  /** The display folder; empty clears it. */
  displayFolder?: string | null
  /** The description; empty clears it. */
  description?: string | null
  isHidden?: boolean | null
}

export interface UpdateMeasureParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** The measure's id (instead of `table` and `measure`). */
  measureId?: number | null
  /** The measure's table. */
  table?: string | null
  /** The measure's name. */
  measure?: string | null
  /** The new DAX expression. */
  expression?: string | null
  /** The static format string; empty clears it. */
  formatString?: string | null
  /** The display folder; empty clears it. */
  displayFolder?: string | null
  /** The description; empty clears it. */
  description?: string | null
  isHidden?: boolean | null
}

export interface UpdateRelationshipParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** The relationship's id from `model.relationships` (instead of `name`). */
  relationshipId?: number | null
  /** The relationship's name (case-insensitive). */
  name?: string | null
  fromCardinality?: CardinalityParam | null
  toCardinality?: CardinalityParam | null
  crossFilteringBehavior?: CrossFilterParam | null
  /**
   * How row-level security filters cross it; `bothDirections` needs
   * `crossFilteringBehavior: bothDirections`.
   */
  securityFilteringBehavior?: SecurityFilterParam | null
  /**
   * Two tables have at most one active relationship: deactivate the
   * other one first (both in one `doc.applyEdits` for one undo step).
   */
  isActive?: boolean | null
}

export interface UpdateTableParams {
  /** The document id from `doc.open` (`d1`, …). */
  docId: string
  /** The table's id (instead of `table`). */
  tableId?: number | null
  /** The table's name. */
  table?: string | null
  /**
   * A calculated table's new DAX expression. A column its result still
   * has (the same source column, else the same name) keeps its ID, name
   * and properties; the result's other columns are added and the ones
   * it no longer has are removed (refused while a relationship,
   * hierarchy or sort order uses one).
   */
  expression?: string | null
  /** The description; empty clears it. */
  description?: string | null
  isHidden?: boolean | null
}

/** Token counts reported by the provider. */
export interface Usage {
  promptTokens?: number | null
  completionTokens?: number | null
}

/** The "View as" state of a document. */
export interface ViewAsOut {
  /**
   * A view is applied: queries see only the rows `roles` allow, as
   * `userName`.
   */
  active: boolean
  /** The roles applied (model spelling); empty when only a user is set. */
  roles: string[]
  /** What `USERPRINCIPALNAME()` and `USERNAME()` return. */
  userName?: string | null
  /** What `CUSTOMDATA()` returns. */
  customData?: string | null
  /**
   * Why the view shows no rows at all: its role filters could not be
   * re-applied after a refresh or a derived column change (it fails
   * closed).
   */
  error?: string | null
}

/** A viewport in CSS pixels. */
export interface ViewportParam {
  width: number
  height: number
}

/** Rows of one table under the view. */
export interface VisibleTableOut {
  name: string
  /** Rows visible under the view (all rows when no view is active). */
  rows: number
  /**
   * Rows loaded by the last refresh or CSV load, before row-level
   * security (absent before the first load).
   */
  totalRows?: number | null
  /** Data has been loaded. */
  loaded: boolean
}

/** Visual counts of a render. */
export interface VisualCounts {
  visuals: number
  ok: number
  /** Rendered with filters not applied (or applied approximately). */
  warnings: number
  errors: number
  hidden: number
  /** Data cut at the query limits. */
  truncated: number
}

export interface VisualDataRoleOut {
  name: string
  displayName?: string | null
  kind?: string | null
}

export interface VisualOut {
  /** Internal name (PBIR folder name / ID). */
  name: string
  /** E.g. `cardVisual`, `slicer`, `tableEx`; `group` for visual groups. */
  visualType: string
  isGroup: boolean
  /** Group display name. */
  displayName?: string | null
  isHidden: boolean
  parentGroupName?: string | null
  position: PositionOut
  /** Bound fields (`queryState` projections), in role order. */
  fields: FieldOut[]
  /** Visual-level filters. */
  filters: FilterOut[]
}

/** Result of `report.visual.move` and `report.visual.resize`. */
export interface VisualPositionOut {
  changed: boolean
  revision: number
  dirty: boolean
  changedObjectIds: number[]
  /** The visual's name. */
  visual: string
  /**
   * The visual's position after the edit (page units, as in
   * `report.visuals`).
   */
  position: PositionOut
  /**
   * For a visual group: the visuals in it whose position changed with
   * it, with their new positions (as in `report.visuals`: stored relative
   * to their group when the file does so).
   */
  members: MemberPositionOut[]
}

/** Status of one visual. */
export interface VisualStatusOut {
  name: string
  visualType: string
  /** `ok`, `warning`, `error` or `hidden`. */
  status: string
  /** The query error (the visual shows ⚠). */
  error?: string | null
  warnings?: string[]
  truncated: boolean
  queryMicros: number
  renderMicros: number
}

export interface VpaxColumnOut {
  name: string
  dataType?: string | null
  /** `HASH` or `VALUE`. */
  encoding?: string | null
  cardinality: number
  dataSize: number
  dictionarySize: number
  hierarchiesSize: number
  totalSize: number
  segments: number
  isHidden: boolean
}

export interface VpaxRelationshipOut {
  from: ColumnRefOut
  to: ColumnRefOut
  usedSize: number
  missingKeys: number
  invalidRows: number
}

export interface VpaxStatsOut {
  /** `DaxModel.json` or `DaxVpaView.json`. */
  source: string
  modelName?: string | null
  compatibilityLevel?: number | null
  extractor?: string | null
  extractionDate?: string | null
  lastDataRefresh?: string | null
  /** Bytes. */
  totalSize: number
  tables: VpaxTableOut[]
  relationships: VpaxRelationshipOut[]
}

export interface VpaxTableOut {
  name: string
  rowsCount: number
  columnsSize: number
  userHierarchiesSize: number
  relationshipsSize: number
  tableSize: number
  isHidden: boolean
  columns: VpaxColumnOut[]
}

/** Every command: its parameters and result. */
export interface OpsCommands {
  /** Lists every command with its metadata (and JSON Schemas with `schemas`). */
  'commands.list': { params: CommandsListParams; output: CommandsListOut }
  /** One command's metadata, examples and parameter/result JSON Schemas. */
  'commands.describe': { params: CommandsDescribeParams; output: CommandInfo }
  /** Runs ordered steps on the shared workspace; strings like "${d.docId}" reference earlier results named with "as". */
  'commands.batch': { params: CommandsBatchParams; output: CommandsBatchOut }
  /** Content-based format detection of a file or folder (signals, models, reports) without reading it fully. */
  'doc.detect': { params: PathParams; output: DetectOut }
  /** Opens a Power BI file or folder into the workspace (no data loaded yet) and returns its docId and summary. */
  'doc.open': { params: DocOpenParams; output: DocSummary }
  /** Closes an open document and frees its data; unsaved edits require explicit discardChanges. */
  'doc.close': { params: DocCloseParams; output: DocCloseOut }
  /** The open documents of the workspace with their summaries. */
  'doc.list': { params: NoParams; output: DocListOut }
  /** The full model, reports and reader diagnostics: the `dashcraft inspect --json` schema (schemaVersion 1). */
  'doc.inspect': { params: DocInspectParams; output: Inspect }
  /** Every model table with its columns, measure names, partitions, hierarchies and loaded row count. */
  'model.tables': { params: ModelParams; output: ModelTablesOut }
  /** Every measure with its table, DAX, format string, description and display folder. */
  'model.measures': { params: ModelMeasuresParams; output: ModelMeasuresOut }
  /** Every relationship: id, from/to columns, cardinalities, cross-filter direction, active flag. */
  'model.relationships': { params: DocParams; output: ModelRelationshipsOut }
  /** The document's reports: name, dataset reference, theme, page and visual counts, report filters. */
  'report.list': { params: DocParams; output: ReportListOut }
  /** The pages of a report: names, size, visibility, visual and filter counts. */
  'report.pages': { params: ReportParams; output: ReportPagesOut }
  /** The visuals of a page: type, title, position, bound fields and visual filters. */
  'report.visuals': { params: PageParams; output: ReportVisualsOut }
  /** A report's filters (plus a page's) translated to column filters, with the ones that cannot be applied (Top N, relative date…). */
  'report.filters': { params: ReportFiltersParams; output: ReportFiltersOut }
  /** The model's row-level security roles (permission, DAX table filters, members, dynamic or not) and the current "View as" state. */
  'security.roles': { params: DocParams; output: SecurityRolesOut }
  /** Replaces a measure's DAX expression as one undoable document edit. */
  'model.setMeasureExpression': { params: SetMeasureExpressionParams; output: DocumentEditOut }
  /** Renames a table, column, measure or M query/parameter as one undoable edit; DAX, M and report references follow and loaded data is kept. dryRun previews it. */
  'model.object.rename': { params: RenameObjectParams; output: RenameObjectOut }
  /** Sets or clears the description of a table, column, measure or other model object as one undoable edit. */
  'model.object.setDescription': { params: SetObjectDescriptionParams; output: DocumentEditOut }
  /** Adds a DAX measure to a table as one undoable edit and returns its id. */
  'model.measure.create': { params: CreateMeasureParams; output: MeasureEditOut }
  /** Changes a measure's expression, format string, display folder, description or visibility as one undoable edit. */
  'model.measure.update': { params: UpdateMeasureParams; output: MeasureEditOut }
  /** Deletes a measure as one undoable edit and lists what referred to it (DAX, report fields); dryRun only lists them. */
  'model.measure.delete': { params: DeleteMeasureParams; output: MeasureEditOut }
  /** Adds a relationship between two columns as one undoable edit; filters follow it at once and warnings report what the engine finds in the loaded data. */
  'model.relationship.create': { params: CreateRelationshipParams; output: RelationshipEditOut }
  /** Changes a relationship's cardinality, cross-filter direction, security filtering or active state as one undoable edit; filters follow it at once. */
  'model.relationship.update': { params: UpdateRelationshipParams; output: RelationshipEditOut }
  /** Deletes a relationship as one undoable edit and lists the DAX that names it (USERELATIONSHIP, CROSSFILTER); dryRun only lists it. */
  'model.relationship.delete': { params: DeleteRelationshipParams; output: RelationshipEditOut }
  /** Adds a DAX calculated column to a table as one undoable edit, computes it over the loaded data and infers its data type unless dataType is given. */
  'model.column.create': { params: CreateColumnParams; output: ColumnCreateOut }
  /** Changes a calculated column's expression or data type, or a column's format, folder, description or visibility; what depends on it is recomputed over the loaded data. */
  'model.column.update': { params: UpdateColumnParams; output: ColumnUpdateOut }
  /** Deletes a calculated column as one undoable edit and lists what referred to it; refuses data columns and columns used by relationships, hierarchies or sort orders. dryRun only checks. */
  'model.column.delete': { params: DeleteColumnParams; output: ObjectDeleteOut }
  /** Adds a DAX calculated table as one undoable edit: its columns and their data types come from the expression's result over the loaded data, and it is computed at once. */
  'model.table.create': { params: CreateTableParams; output: TableCreateOut }
  /** Changes a calculated table's DAX expression, its columns following the result, or a table's description or visibility; the table and what depends on it are recomputed. */
  'model.table.update': { params: UpdateTableParams; output: TableUpdateOut }
  /** Deletes a table with its columns, measures, relationships and role row filters as one undoable edit and lists what referred to it (DAX, report fields); dryRun only lists them. */
  'model.table.delete': { params: DeleteTableParams; output: ObjectDeleteOut }
  /** Adds a user hierarchy over columns of a table as one undoable edit; levels are listed top first and named after their columns unless named. */
  'model.hierarchy.create': { params: CreateHierarchyParams; output: HierarchyCreateOut }
  /** Deletes a user hierarchy with its levels as one undoable edit and lists the report fields that used it; dryRun only lists them. */
  'model.hierarchy.delete': { params: DeleteHierarchyParams; output: ObjectDeleteOut }
  /** Lists what refers to a table, column, measure or relationship: DAX, relationships, hierarchies, sort-by columns, roles and report fields. */
  'model.dependencies': { params: DependenciesParams; output: DependenciesOut }
  /** Moves a visual or a visual group (with the visuals in it) on its page as one undoable edit. */
  'report.visual.move': { params: MoveVisualParams; output: VisualPositionOut }
  /** Resizes a visual or a visual group (top-left corner fixed; the visuals in a group scale with it unless it scrolls) as one undoable edit. */
  'report.visual.resize': { params: ResizeVisualParams; output: VisualPositionOut }
  /** Applies a group of document edits atomically as one undoable history step. */
  'doc.applyEdits': { params: ApplyEditsParams; output: DocumentEditOut }
  /** Undoes the latest document edit. */
  'doc.undo': { params: DocumentParams; output: DocumentEditOut }
  /** Reapplies the latest undone document edit. */
  'doc.redo': { params: DocumentParams; output: DocumentEditOut }
  /** Returns the document revision, dirty state and undo/redo availability. */
  'doc.history': { params: DocumentParams; output: DocumentHistoryOut }
  /** Saves the edits into the opened PBIP project, writing only changed files; files changed on disk since the open are a conflict unless overwrite. */
  'doc.save': { params: DocumentSaveParams; output: DocumentSaveOut }
  /** Saves the document, edits included, as a new PBIP project from any format and continues editing there: later doc.save writes to it; imported data is not carried over. */
  'doc.saveAs': { params: DocumentSaveAsParams; output: DocumentSaveAsOut }
  /** Broken-reference checks (visual fields, queryRefs, filters, DAX/M, relationships, layout) as the `lint --json` report. */
  'doc.lint': { params: DocLintParams; output: LintOut }
  /** Loads the data offline (PBIP cache, M partitions with the blob map, embedded PBIX data, calculated objects); per-table rows, time and status. */
  'data.refresh': { params: DataRefreshParams; output: RefreshOut }
  /** Loaded rows, columns and last refresh status of every table, the derived columns and the local data cache. */
  'data.tables': { params: DocParams; output: DataTablesOut }
  /** One page of a table's rows (sorted, SQL-filtered), raw or formatted; loads the data on first use. */
  'data.page': { params: DataPageParams; output: DataPageOut }
  /** Rows, blanks, distinct count, min and max of one column; loads the data on first use. */
  'data.columnStats': { params: ColumnParams; output: ColumnStatsOut }
  /** Replaces a table's data with a CSV ("Load CSV manually…") and recomputes what depends on it. */
  'data.loadCsv': { params: DataLoadCsvParams; output: RefreshOut }
  /** Adds a session-only column (people parsing or a DAX expression), re-applied after every refresh; never saved to the model. */
  'data.addDerivedColumn': { params: DataAddDerivedColumnParams; output: DerivedColumnOut }
  /** Removes a session derived column from the table data and the session model. */
  'data.removeDerivedColumn': { params: DerivedRefParams; output: DerivedColumnOut }
  /** Views the data as security roles and/or a user (row-level security, union of roles); empty roles and no user stop it. Visible rows per table. */
  'security.viewAs': { params: SecurityViewAsParams; output: SecurityViewAsOut }
  /** Evaluates a DAX query (EVALUATE …) and returns columns and rows per result table; loads the data on first use. */
  'dax.query': { params: DaxQueryParams; output: DaxQueryOut }
  /** One measure's value (raw and formatted, or ⚠ error with its DAX) under report/page and column filters. */
  'dax.evaluateMeasure': { params: EvaluateMeasureParams; output: EvaluateMeasureOut }
  /** Every measure's formatted value under optional report/page filters; failed measures show ⚠ and their DAX. */
  'measures.values': { params: MeasuresValuesParams; output: MeasuresValuesOut }
  /** Syntax errors, unknown tables, columns and measures (with a did-you-mean suggestion), wrong argument counts and functions DashCraft cannot evaluate, for a DAX expression or query; no data is loaded. */
  'dax.diagnostics': { params: DaxDiagnosticsParams; output: DaxDiagnosticsOut }
  /** The DAX function call around a cursor: its signature, parameters, return type, description and the parameter the cursor is in; works on text that does not parse yet. */
  'dax.signatureHelp': { params: DaxSignatureHelpParams; output: DaxSignatureHelpOut }
  /** Describes the DAX function, table, column, measure, variable or keyword at a cursor: signature and description, data type, expression, format string; no data is loaded. */
  'dax.hover': { params: DaxHoverParams; output: DaxHoverOut }
  /** The DAX functions, tables, columns, measures, variables and keywords that fit at a cursor, best match first, with the text to insert; works on text that does not parse yet. */
  'dax.complete': { params: DaxCompleteParams; output: DaxCompleteOut }
  /** What every token of a DAX text is (keyword, function, table, column, measure, variable, parameter, literal, comment) for highlighting, with declarations and unknown names marked. */
  'dax.tokens': { params: DaxTokensParams; output: DaxTokensOut }
  /** The symbol at a cursor in a DAX text, where the text declares it and every place it occurs (go to definition, find references); model objects carry their objectId. */
  'dax.references': { params: DaxReferencesParams; output: DaxReferencesOut }
  /** Lays out a DAX expression or query within a line width, keeping comments and meaning (only whitespace and keyword/function case change); idempotent, no data is loaded. */
  'dax.format': { params: DaxFormatParams; output: DaxFormatOut }
  /** Renders a report page to SVG or PNG (inline, base64 for PNG, or to outputPath) with per-visual status; loads the data on first use. */
  'render.page': { params: RenderPageParams; output: RenderOut }
  /** Renders one visual of a page, cropped to its container, to SVG or PNG (with the page's filters, slicers and interactions). */
  'render.visual': { params: RenderVisualParams; output: RenderOut }
  /** Renders a page with its current interaction state: SVG, hit regions, visuals, slicer/cross-filter selections and effective filters. */
  'canvas.render': { params: CanvasRenderParams; output: CanvasPageOut }
  /** Applies a click on a hit region (slicer item, data point, button, dropdown) and returns the re-rendered page and any button action. */
  'canvas.click': { params: CanvasClickParams; output: CanvasChangeOut }
  /** Scrolls a table, matrix or slicer list by a delta (clamped to its range) and returns the re-rendered page. */
  'canvas.scroll': { params: CanvasScrollParams; output: CanvasChangeOut }
  /** Replaces a slicer's selection by value (no hit ids needed) and returns the re-rendered page. */
  'canvas.setSlicer': { params: CanvasSetSlicerParams; output: CanvasPageOut }
  /** Clears a slicer's selection ("Clear selections") or the page's cross-filter, and returns the re-rendered page. */
  'canvas.clearSelections': { params: CanvasClearParams; output: CanvasPageOut }
  /** Filters a slicer's list (or open dropdown) by a search text and returns the re-rendered page. */
  'canvas.setSlicerSearch': { params: CanvasSearchParams; output: CanvasPageOut }
  /** Expands or collapses the matrix row of a cell hit and returns the re-rendered page. */
  'canvas.toggleMatrixRow': { params: CanvasHitParams; output: CanvasPageOut }
  /** Closes the open slicer dropdowns of a page. */
  'canvas.closePopups': { params: CanvasClosePopupsParams; output: CanvasChangeOut }
  /** Back to the report's saved slicer selections, no cross-filter, scroll and dropdowns reset; returns the re-rendered page. */
  'canvas.reset': { params: CanvasPageParams; output: CanvasPageOut }
  /** The hover tooltip (category, series, values) of a data point hit, from the last render. */
  'canvas.tooltip': { params: CanvasHitParams; output: TooltipOut }
  /** The page's current interaction state (slicer selections, cross-filter) and view state, as JSON. */
  'canvas.state': { params: CanvasPageParams; output: CanvasStateOut }
  /** Writes one table's data as CSV or Parquet (with model metadata); loads the data on first use. */
  'export.table': { params: ExportTableParams; output: ExportTableOut }
  /** The model documentation as JSON (model doc) or a Markdown data dictionary, inline or to a file; no data needed. */
  'export.model': { params: ExportModelParams; output: ExportModelOut }
  /** Writes the document, unsaved edits included, as a PBIP project (TMDL + PBIR) into a folder; the open document keeps its source (doc.saveAs moves it). */
  'convert.pbip': { params: ConvertPbipParams; output: ConvertPbipOut }
  /** Writes a PBIT template (model schema and one report, no data), unsaved edits included. */
  'convert.pbit': { params: ConvertPbitParams; output: ConvertPbitOut }
  /** Parses one people-picker cell (a Python-style list of dicts) into `First Last` names (the "Parse people field" transform). */
  'text.parsePeople': { params: ParsePeopleParams; output: ParsePeopleOut }
  /** Parses every cell of a text column as a people list: outcome counts and distinct samples with their parsed form (before data.addDerivedColumn). */
  'text.previewPeople': { params: PreviewPeopleParams; output: PeoplePreviewOut }
  /** Whether AI is enabled, configured providers (never keys), recorded consents, limits and which capabilities are implemented. No network. */
  'ai.status': { params: NoParams; output: AiStatusOut }
  /** The exact request an AI capability would send (key redacted), its data-exposure class and consent status. Never sends anything; works while AI is disabled. */
  'ai.preview': { params: AiPreviewParams; output: AiPreviewOut }
  /** Explains a measure with an AI model: summary, Markdown explanation, filter-context notes, dependencies, caveats. Sends model metadata only; needs AI enabled and consent. */
  'ai.explainMeasure': { params: ExplainMeasureParams; output: ExplainMeasureOut }
  /** Natural language → a DAX measure suggestion, parsed locally with diagnostics and optionally evaluated; never written to the model. Needs AI enabled and consent. */
  'ai.generateMeasure': { params: GenerateMeasureParams; output: GenerateMeasureOut }
  /** Records a person's consent to send one capability's data (metadata or sample) to one provider endpoint. Not available to agents. */
  'ai.consent.grant': { params: ConsentGrantParams; output: ConsentGrantOut }
  /** Removes recorded consents of a provider (one capability or all). Not available to agents. */
  'ai.consent.revoke': { params: ConsentRevokeParams; output: ConsentRevokeOut }
  /** Changes the AI settings (enable, providers, default provider, limits, sample rows); never holds keys. Not available to agents. */
  'ai.settings.set': { params: SettingsSetParams; output: SettingsSetOut }
  /** Stores a provider's API key in the OS keychain. Desktop app only (the key travels in the parameters); use `dashcraft ai login` on the command line. */
  'ai.apiKey.set': { params: ApiKeySetParams; output: ApiKeyOut }
  /** Deletes a provider's API key from the OS keychain (DASHCRAFT_AI_API_KEY is not affected). Not available to agents. */
  'ai.apiKey.clear': { params: ApiKeyClearParams; output: ApiKeyOut }
}

export type OpsId = keyof OpsCommands
export type OpsParams<K extends OpsId> = OpsCommands[K]['params']
export type OpsOutput<K extends OpsId> = OpsCommands[K]['output']

/** Catalog metadata of every command (`commands.describe` without schemas). */
export const OPS_META = {
  'commands.list': { category: 'meta', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'commands.describe': { category: 'meta', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'commands.batch': { category: 'meta', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'doc.detect': { category: 'workspace', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'doc.open': { category: 'workspace', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'doc.close': { category: 'workspace', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'doc.list': { category: 'workspace', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'doc.inspect': { category: 'inspect', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'model.tables': { category: 'inspect', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'model.measures': { category: 'inspect', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'model.relationships': { category: 'inspect', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'report.list': { category: 'inspect', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'report.pages': { category: 'inspect', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'report.visuals': { category: 'inspect', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'report.filters': { category: 'inspect', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'security.roles': { category: 'inspect', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'model.setMeasureExpression': { category: 'edit', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'model.object.rename': { category: 'edit', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'model.object.setDescription': { category: 'edit', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'model.measure.create': { category: 'edit', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'model.measure.update': { category: 'edit', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'model.measure.delete': { category: 'edit', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'model.relationship.create': { category: 'edit', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'model.relationship.update': { category: 'edit', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'model.relationship.delete': { category: 'edit', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'model.column.create': { category: 'edit', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'model.column.update': { category: 'edit', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'model.column.delete': { category: 'edit', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'model.table.create': { category: 'edit', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'model.table.update': { category: 'edit', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'model.table.delete': { category: 'edit', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'model.hierarchy.create': { category: 'edit', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'model.hierarchy.delete': { category: 'edit', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'model.dependencies': { category: 'inspect', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'report.visual.move': { category: 'edit', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'report.visual.resize': { category: 'edit', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'doc.applyEdits': { category: 'edit', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'doc.undo': { category: 'edit', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'doc.redo': { category: 'edit', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'doc.history': { category: 'edit', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'doc.save': { category: 'edit', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: true },
  'doc.saveAs': { category: 'edit', mutatesDocument: true, longRunning: true, agentSafe: true, writesFiles: true },
  'doc.lint': { category: 'lint', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'data.refresh': { category: 'data', mutatesDocument: true, longRunning: true, agentSafe: true, writesFiles: false },
  'data.tables': { category: 'data', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'data.page': { category: 'data', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'data.columnStats': { category: 'data', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'data.loadCsv': { category: 'data', mutatesDocument: true, longRunning: true, agentSafe: true, writesFiles: false },
  'data.addDerivedColumn': { category: 'data', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'data.removeDerivedColumn': { category: 'data', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'security.viewAs': { category: 'data', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'dax.query': { category: 'dax', mutatesDocument: false, longRunning: true, agentSafe: true, writesFiles: false },
  'dax.evaluateMeasure': { category: 'dax', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'measures.values': { category: 'dax', mutatesDocument: false, longRunning: true, agentSafe: true, writesFiles: false },
  'dax.diagnostics': { category: 'dax', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'dax.signatureHelp': { category: 'dax', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'dax.hover': { category: 'dax', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'dax.complete': { category: 'dax', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'dax.tokens': { category: 'dax', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'dax.references': { category: 'dax', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'dax.format': { category: 'dax', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'render.page': { category: 'render', mutatesDocument: false, longRunning: true, agentSafe: true, writesFiles: true },
  'render.visual': { category: 'render', mutatesDocument: false, longRunning: true, agentSafe: true, writesFiles: true },
  'canvas.render': { category: 'canvas', mutatesDocument: true, longRunning: true, agentSafe: true, writesFiles: false },
  'canvas.click': { category: 'canvas', mutatesDocument: true, longRunning: true, agentSafe: true, writesFiles: false },
  'canvas.scroll': { category: 'canvas', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'canvas.setSlicer': { category: 'canvas', mutatesDocument: true, longRunning: true, agentSafe: true, writesFiles: false },
  'canvas.clearSelections': { category: 'canvas', mutatesDocument: true, longRunning: true, agentSafe: true, writesFiles: false },
  'canvas.setSlicerSearch': { category: 'canvas', mutatesDocument: true, longRunning: true, agentSafe: true, writesFiles: false },
  'canvas.toggleMatrixRow': { category: 'canvas', mutatesDocument: true, longRunning: true, agentSafe: true, writesFiles: false },
  'canvas.closePopups': { category: 'canvas', mutatesDocument: true, longRunning: false, agentSafe: true, writesFiles: false },
  'canvas.reset': { category: 'canvas', mutatesDocument: true, longRunning: true, agentSafe: true, writesFiles: false },
  'canvas.tooltip': { category: 'canvas', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'canvas.state': { category: 'canvas', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'export.table': { category: 'export', mutatesDocument: false, longRunning: true, agentSafe: true, writesFiles: true },
  'export.model': { category: 'export', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: true },
  'convert.pbip': { category: 'convert', mutatesDocument: false, longRunning: true, agentSafe: true, writesFiles: true },
  'convert.pbit': { category: 'convert', mutatesDocument: false, longRunning: true, agentSafe: true, writesFiles: true },
  'text.parsePeople': { category: 'transform', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'text.previewPeople': { category: 'transform', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'ai.status': { category: 'ai', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'ai.preview': { category: 'ai', mutatesDocument: false, longRunning: false, agentSafe: true, writesFiles: false },
  'ai.explainMeasure': { category: 'ai', mutatesDocument: false, longRunning: true, agentSafe: true, writesFiles: false },
  'ai.generateMeasure': { category: 'ai', mutatesDocument: false, longRunning: true, agentSafe: true, writesFiles: false },
  'ai.consent.grant': { category: 'ai', mutatesDocument: false, longRunning: false, agentSafe: false, writesFiles: true },
  'ai.consent.revoke': { category: 'ai', mutatesDocument: false, longRunning: false, agentSafe: false, writesFiles: true },
  'ai.settings.set': { category: 'ai', mutatesDocument: false, longRunning: false, agentSafe: false, writesFiles: true },
  'ai.apiKey.set': { category: 'ai', mutatesDocument: false, longRunning: false, agentSafe: false, writesFiles: true },
  'ai.apiKey.clear': { category: 'ai', mutatesDocument: false, longRunning: false, agentSafe: false, writesFiles: true },
} as const satisfies Record<OpsId, unknown>

/** The outcome of a command: its result, or a failure with the error code (`backend`: the IPC call itself failed). */
export type OpsResult<T> = { ok: true; value: T } | { ok: false; error: string; kind: OpErrorCode | 'backend'; data?: unknown }

/** Runs command `id` (the app's `ops_execute` IPC command). */
export type OpsRunner<O> = <K extends OpsId>(id: K, params: OpsParams<K>, options?: O) => Promise<OpsResult<OpsOutput<K>>>

/** Typed wrappers over a runner: `ops.docOpen(params)`. */
export function makeOps<O = undefined>(run: OpsRunner<O>) {
  return {
    /** Lists every command with its metadata (and JSON Schemas with `schemas`). */
    commandsList: (params: CommandsListParams = {}, options?: O) => run('commands.list', params, options),
    /** One command's metadata, examples and parameter/result JSON Schemas. */
    commandsDescribe: (params: CommandsDescribeParams, options?: O) => run('commands.describe', params, options),
    /** Runs ordered steps on the shared workspace; strings like "${d.docId}" reference earlier results named with "as". */
    commandsBatch: (params: CommandsBatchParams, options?: O) => run('commands.batch', params, options),
    /** Content-based format detection of a file or folder (signals, models, reports) without reading it fully. */
    docDetect: (params: PathParams, options?: O) => run('doc.detect', params, options),
    /** Opens a Power BI file or folder into the workspace (no data loaded yet) and returns its docId and summary. */
    docOpen: (params: DocOpenParams, options?: O) => run('doc.open', params, options),
    /** Closes an open document and frees its data; unsaved edits require explicit discardChanges. */
    docClose: (params: DocCloseParams, options?: O) => run('doc.close', params, options),
    /** The open documents of the workspace with their summaries. */
    docList: (params: NoParams = {}, options?: O) => run('doc.list', params, options),
    /** The full model, reports and reader diagnostics: the `dashcraft inspect --json` schema (schemaVersion 1). */
    docInspect: (params: DocInspectParams, options?: O) => run('doc.inspect', params, options),
    /** Every model table with its columns, measure names, partitions, hierarchies and loaded row count. */
    modelTables: (params: ModelParams, options?: O) => run('model.tables', params, options),
    /** Every measure with its table, DAX, format string, description and display folder. */
    modelMeasures: (params: ModelMeasuresParams, options?: O) => run('model.measures', params, options),
    /** Every relationship: id, from/to columns, cardinalities, cross-filter direction, active flag. */
    modelRelationships: (params: DocParams, options?: O) => run('model.relationships', params, options),
    /** The document's reports: name, dataset reference, theme, page and visual counts, report filters. */
    reportList: (params: DocParams, options?: O) => run('report.list', params, options),
    /** The pages of a report: names, size, visibility, visual and filter counts. */
    reportPages: (params: ReportParams, options?: O) => run('report.pages', params, options),
    /** The visuals of a page: type, title, position, bound fields and visual filters. */
    reportVisuals: (params: PageParams, options?: O) => run('report.visuals', params, options),
    /** A report's filters (plus a page's) translated to column filters, with the ones that cannot be applied (Top N, relative date…). */
    reportFilters: (params: ReportFiltersParams, options?: O) => run('report.filters', params, options),
    /** The model's row-level security roles (permission, DAX table filters, members, dynamic or not) and the current "View as" state. */
    securityRoles: (params: DocParams, options?: O) => run('security.roles', params, options),
    /** Replaces a measure's DAX expression as one undoable document edit. */
    modelSetMeasureExpression: (params: SetMeasureExpressionParams, options?: O) => run('model.setMeasureExpression', params, options),
    /** Renames a table, column, measure or M query/parameter as one undoable edit; DAX, M and report references follow and loaded data is kept. dryRun previews it. */
    modelObjectRename: (params: RenameObjectParams, options?: O) => run('model.object.rename', params, options),
    /** Sets or clears the description of a table, column, measure or other model object as one undoable edit. */
    modelObjectSetDescription: (params: SetObjectDescriptionParams, options?: O) => run('model.object.setDescription', params, options),
    /** Adds a DAX measure to a table as one undoable edit and returns its id. */
    modelMeasureCreate: (params: CreateMeasureParams, options?: O) => run('model.measure.create', params, options),
    /** Changes a measure's expression, format string, display folder, description or visibility as one undoable edit. */
    modelMeasureUpdate: (params: UpdateMeasureParams, options?: O) => run('model.measure.update', params, options),
    /** Deletes a measure as one undoable edit and lists what referred to it (DAX, report fields); dryRun only lists them. */
    modelMeasureDelete: (params: DeleteMeasureParams, options?: O) => run('model.measure.delete', params, options),
    /** Adds a relationship between two columns as one undoable edit; filters follow it at once and warnings report what the engine finds in the loaded data. */
    modelRelationshipCreate: (params: CreateRelationshipParams, options?: O) => run('model.relationship.create', params, options),
    /** Changes a relationship's cardinality, cross-filter direction, security filtering or active state as one undoable edit; filters follow it at once. */
    modelRelationshipUpdate: (params: UpdateRelationshipParams, options?: O) => run('model.relationship.update', params, options),
    /** Deletes a relationship as one undoable edit and lists the DAX that names it (USERELATIONSHIP, CROSSFILTER); dryRun only lists it. */
    modelRelationshipDelete: (params: DeleteRelationshipParams, options?: O) => run('model.relationship.delete', params, options),
    /** Adds a DAX calculated column to a table as one undoable edit, computes it over the loaded data and infers its data type unless dataType is given. */
    modelColumnCreate: (params: CreateColumnParams, options?: O) => run('model.column.create', params, options),
    /** Changes a calculated column's expression or data type, or a column's format, folder, description or visibility; what depends on it is recomputed over the loaded data. */
    modelColumnUpdate: (params: UpdateColumnParams, options?: O) => run('model.column.update', params, options),
    /** Deletes a calculated column as one undoable edit and lists what referred to it; refuses data columns and columns used by relationships, hierarchies or sort orders. dryRun only checks. */
    modelColumnDelete: (params: DeleteColumnParams, options?: O) => run('model.column.delete', params, options),
    /** Adds a DAX calculated table as one undoable edit: its columns and their data types come from the expression's result over the loaded data, and it is computed at once. */
    modelTableCreate: (params: CreateTableParams, options?: O) => run('model.table.create', params, options),
    /** Changes a calculated table's DAX expression, its columns following the result, or a table's description or visibility; the table and what depends on it are recomputed. */
    modelTableUpdate: (params: UpdateTableParams, options?: O) => run('model.table.update', params, options),
    /** Deletes a table with its columns, measures, relationships and role row filters as one undoable edit and lists what referred to it (DAX, report fields); dryRun only lists them. */
    modelTableDelete: (params: DeleteTableParams, options?: O) => run('model.table.delete', params, options),
    /** Adds a user hierarchy over columns of a table as one undoable edit; levels are listed top first and named after their columns unless named. */
    modelHierarchyCreate: (params: CreateHierarchyParams, options?: O) => run('model.hierarchy.create', params, options),
    /** Deletes a user hierarchy with its levels as one undoable edit and lists the report fields that used it; dryRun only lists them. */
    modelHierarchyDelete: (params: DeleteHierarchyParams, options?: O) => run('model.hierarchy.delete', params, options),
    /** Lists what refers to a table, column, measure or relationship: DAX, relationships, hierarchies, sort-by columns, roles and report fields. */
    modelDependencies: (params: DependenciesParams, options?: O) => run('model.dependencies', params, options),
    /** Moves a visual or a visual group (with the visuals in it) on its page as one undoable edit. */
    reportVisualMove: (params: MoveVisualParams, options?: O) => run('report.visual.move', params, options),
    /** Resizes a visual or a visual group (top-left corner fixed; the visuals in a group scale with it unless it scrolls) as one undoable edit. */
    reportVisualResize: (params: ResizeVisualParams, options?: O) => run('report.visual.resize', params, options),
    /** Applies a group of document edits atomically as one undoable history step. */
    docApplyEdits: (params: ApplyEditsParams, options?: O) => run('doc.applyEdits', params, options),
    /** Undoes the latest document edit. */
    docUndo: (params: DocumentParams, options?: O) => run('doc.undo', params, options),
    /** Reapplies the latest undone document edit. */
    docRedo: (params: DocumentParams, options?: O) => run('doc.redo', params, options),
    /** Returns the document revision, dirty state and undo/redo availability. */
    docHistory: (params: DocumentParams, options?: O) => run('doc.history', params, options),
    /** Saves the edits into the opened PBIP project, writing only changed files; files changed on disk since the open are a conflict unless overwrite. */
    docSave: (params: DocumentSaveParams, options?: O) => run('doc.save', params, options),
    /** Saves the document, edits included, as a new PBIP project from any format and continues editing there: later doc.save writes to it; imported data is not carried over. */
    docSaveAs: (params: DocumentSaveAsParams, options?: O) => run('doc.saveAs', params, options),
    /** Broken-reference checks (visual fields, queryRefs, filters, DAX/M, relationships, layout) as the `lint --json` report. */
    docLint: (params: DocLintParams, options?: O) => run('doc.lint', params, options),
    /** Loads the data offline (PBIP cache, M partitions with the blob map, embedded PBIX data, calculated objects); per-table rows, time and status. */
    dataRefresh: (params: DataRefreshParams, options?: O) => run('data.refresh', params, options),
    /** Loaded rows, columns and last refresh status of every table, the derived columns and the local data cache. */
    dataTables: (params: DocParams, options?: O) => run('data.tables', params, options),
    /** One page of a table's rows (sorted, SQL-filtered), raw or formatted; loads the data on first use. */
    dataPage: (params: DataPageParams, options?: O) => run('data.page', params, options),
    /** Rows, blanks, distinct count, min and max of one column; loads the data on first use. */
    dataColumnStats: (params: ColumnParams, options?: O) => run('data.columnStats', params, options),
    /** Replaces a table's data with a CSV ("Load CSV manually…") and recomputes what depends on it. */
    dataLoadCsv: (params: DataLoadCsvParams, options?: O) => run('data.loadCsv', params, options),
    /** Adds a session-only column (people parsing or a DAX expression), re-applied after every refresh; never saved to the model. */
    dataAddDerivedColumn: (params: DataAddDerivedColumnParams, options?: O) => run('data.addDerivedColumn', params, options),
    /** Removes a session derived column from the table data and the session model. */
    dataRemoveDerivedColumn: (params: DerivedRefParams, options?: O) => run('data.removeDerivedColumn', params, options),
    /** Views the data as security roles and/or a user (row-level security, union of roles); empty roles and no user stop it. Visible rows per table. */
    securityViewAs: (params: SecurityViewAsParams, options?: O) => run('security.viewAs', params, options),
    /** Evaluates a DAX query (EVALUATE …) and returns columns and rows per result table; loads the data on first use. */
    daxQuery: (params: DaxQueryParams, options?: O) => run('dax.query', params, options),
    /** One measure's value (raw and formatted, or ⚠ error with its DAX) under report/page and column filters. */
    daxEvaluateMeasure: (params: EvaluateMeasureParams, options?: O) => run('dax.evaluateMeasure', params, options),
    /** Every measure's formatted value under optional report/page filters; failed measures show ⚠ and their DAX. */
    measuresValues: (params: MeasuresValuesParams, options?: O) => run('measures.values', params, options),
    /** Syntax errors, unknown tables, columns and measures (with a did-you-mean suggestion), wrong argument counts and functions DashCraft cannot evaluate, for a DAX expression or query; no data is loaded. */
    daxDiagnostics: (params: DaxDiagnosticsParams, options?: O) => run('dax.diagnostics', params, options),
    /** The DAX function call around a cursor: its signature, parameters, return type, description and the parameter the cursor is in; works on text that does not parse yet. */
    daxSignatureHelp: (params: DaxSignatureHelpParams, options?: O) => run('dax.signatureHelp', params, options),
    /** Describes the DAX function, table, column, measure, variable or keyword at a cursor: signature and description, data type, expression, format string; no data is loaded. */
    daxHover: (params: DaxHoverParams, options?: O) => run('dax.hover', params, options),
    /** The DAX functions, tables, columns, measures, variables and keywords that fit at a cursor, best match first, with the text to insert; works on text that does not parse yet. */
    daxComplete: (params: DaxCompleteParams, options?: O) => run('dax.complete', params, options),
    /** What every token of a DAX text is (keyword, function, table, column, measure, variable, parameter, literal, comment) for highlighting, with declarations and unknown names marked. */
    daxTokens: (params: DaxTokensParams, options?: O) => run('dax.tokens', params, options),
    /** The symbol at a cursor in a DAX text, where the text declares it and every place it occurs (go to definition, find references); model objects carry their objectId. */
    daxReferences: (params: DaxReferencesParams, options?: O) => run('dax.references', params, options),
    /** Lays out a DAX expression or query within a line width, keeping comments and meaning (only whitespace and keyword/function case change); idempotent, no data is loaded. */
    daxFormat: (params: DaxFormatParams, options?: O) => run('dax.format', params, options),
    /** Renders a report page to SVG or PNG (inline, base64 for PNG, or to outputPath) with per-visual status; loads the data on first use. */
    renderPage: (params: RenderPageParams, options?: O) => run('render.page', params, options),
    /** Renders one visual of a page, cropped to its container, to SVG or PNG (with the page's filters, slicers and interactions). */
    renderVisual: (params: RenderVisualParams, options?: O) => run('render.visual', params, options),
    /** Renders a page with its current interaction state: SVG, hit regions, visuals, slicer/cross-filter selections and effective filters. */
    canvasRender: (params: CanvasRenderParams, options?: O) => run('canvas.render', params, options),
    /** Applies a click on a hit region (slicer item, data point, button, dropdown) and returns the re-rendered page and any button action. */
    canvasClick: (params: CanvasClickParams, options?: O) => run('canvas.click', params, options),
    /** Scrolls a table, matrix or slicer list by a delta (clamped to its range) and returns the re-rendered page. */
    canvasScroll: (params: CanvasScrollParams, options?: O) => run('canvas.scroll', params, options),
    /** Replaces a slicer's selection by value (no hit ids needed) and returns the re-rendered page. */
    canvasSetSlicer: (params: CanvasSetSlicerParams, options?: O) => run('canvas.setSlicer', params, options),
    /** Clears a slicer's selection ("Clear selections") or the page's cross-filter, and returns the re-rendered page. */
    canvasClearSelections: (params: CanvasClearParams, options?: O) => run('canvas.clearSelections', params, options),
    /** Filters a slicer's list (or open dropdown) by a search text and returns the re-rendered page. */
    canvasSetSlicerSearch: (params: CanvasSearchParams, options?: O) => run('canvas.setSlicerSearch', params, options),
    /** Expands or collapses the matrix row of a cell hit and returns the re-rendered page. */
    canvasToggleMatrixRow: (params: CanvasHitParams, options?: O) => run('canvas.toggleMatrixRow', params, options),
    /** Closes the open slicer dropdowns of a page. */
    canvasClosePopups: (params: CanvasClosePopupsParams, options?: O) => run('canvas.closePopups', params, options),
    /** Back to the report's saved slicer selections, no cross-filter, scroll and dropdowns reset; returns the re-rendered page. */
    canvasReset: (params: CanvasPageParams, options?: O) => run('canvas.reset', params, options),
    /** The hover tooltip (category, series, values) of a data point hit, from the last render. */
    canvasTooltip: (params: CanvasHitParams, options?: O) => run('canvas.tooltip', params, options),
    /** The page's current interaction state (slicer selections, cross-filter) and view state, as JSON. */
    canvasState: (params: CanvasPageParams, options?: O) => run('canvas.state', params, options),
    /** Writes one table's data as CSV or Parquet (with model metadata); loads the data on first use. */
    exportTable: (params: ExportTableParams, options?: O) => run('export.table', params, options),
    /** The model documentation as JSON (model doc) or a Markdown data dictionary, inline or to a file; no data needed. */
    exportModel: (params: ExportModelParams, options?: O) => run('export.model', params, options),
    /** Writes the document, unsaved edits included, as a PBIP project (TMDL + PBIR) into a folder; the open document keeps its source (doc.saveAs moves it). */
    convertPbip: (params: ConvertPbipParams, options?: O) => run('convert.pbip', params, options),
    /** Writes a PBIT template (model schema and one report, no data), unsaved edits included. */
    convertPbit: (params: ConvertPbitParams, options?: O) => run('convert.pbit', params, options),
    /** Parses one people-picker cell (a Python-style list of dicts) into `First Last` names (the "Parse people field" transform). */
    textParsePeople: (params: ParsePeopleParams, options?: O) => run('text.parsePeople', params, options),
    /** Parses every cell of a text column as a people list: outcome counts and distinct samples with their parsed form (before data.addDerivedColumn). */
    textPreviewPeople: (params: PreviewPeopleParams, options?: O) => run('text.previewPeople', params, options),
    /** Whether AI is enabled, configured providers (never keys), recorded consents, limits and which capabilities are implemented. No network. */
    aiStatus: (params: NoParams = {}, options?: O) => run('ai.status', params, options),
    /** The exact request an AI capability would send (key redacted), its data-exposure class and consent status. Never sends anything; works while AI is disabled. */
    aiPreview: (params: AiPreviewParams, options?: O) => run('ai.preview', params, options),
    /** Explains a measure with an AI model: summary, Markdown explanation, filter-context notes, dependencies, caveats. Sends model metadata only; needs AI enabled and consent. */
    aiExplainMeasure: (params: ExplainMeasureParams, options?: O) => run('ai.explainMeasure', params, options),
    /** Natural language → a DAX measure suggestion, parsed locally with diagnostics and optionally evaluated; never written to the model. Needs AI enabled and consent. */
    aiGenerateMeasure: (params: GenerateMeasureParams, options?: O) => run('ai.generateMeasure', params, options),
    /** Records a person's consent to send one capability's data (metadata or sample) to one provider endpoint. Not available to agents. */
    aiConsentGrant: (params: ConsentGrantParams, options?: O) => run('ai.consent.grant', params, options),
    /** Removes recorded consents of a provider (one capability or all). Not available to agents. */
    aiConsentRevoke: (params: ConsentRevokeParams, options?: O) => run('ai.consent.revoke', params, options),
    /** Changes the AI settings (enable, providers, default provider, limits, sample rows); never holds keys. Not available to agents. */
    aiSettingsSet: (params: SettingsSetParams = {}, options?: O) => run('ai.settings.set', params, options),
    /** Stores a provider's API key in the OS keychain. Desktop app only (the key travels in the parameters); use `dashcraft ai login` on the command line. */
    aiApiKeySet: (params: ApiKeySetParams, options?: O) => run('ai.apiKey.set', params, options),
    /** Deletes a provider's API key from the OS keychain (DASHCRAFT_AI_API_KEY is not affected). Not available to agents. */
    aiApiKeyClear: (params: ApiKeyClearParams, options?: O) => run('ai.apiKey.clear', params, options),
  }
}
