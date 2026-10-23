// Icon catalog: semantic names used by the UI → Fluent UI System Icons (MIT, © Microsoft
// Corporation; see `src/assets/icons/LICENSE`). Only the SVGs listed here are vendored in
// `src/assets/icons/` as `<fluent>_20_regular.svg` (plus `_24_regular.svg` where Fluent has it,
// used for icons drawn larger than 20px). The SVGs carry no fill: `Icon.svelte` paints them
// with `currentColor`, so CSS decides the color.

export const ICONS = {
  // Model objects (Data pane, Model view)
  table: 'table',
  calcTable: 'table_calculator',
  column: 'column_triple',
  calcColumn: 'math_formula',
  measure: 'calculator',
  numeric: 'autosum',
  date: 'calendar_ltr',
  geo: 'globe',
  hierarchy: 'text_bullet_list_tree',
  key: 'key',
  parameter: 'braces',
  query: 'document_table',
  relationship: 'link',
  remote: 'cloud',
  // Navigation and chrome
  chevron: 'chevron_right',
  chevronDown: 'chevron_down',
  chevronUp: 'chevron_up',
  chevronLeft: 'chevron_left',
  chevronDoubleLeft: 'chevron_double_left',
  chevronDoubleRight: 'chevron_double_right',
  arrowLeft: 'arrow_left',
  arrowRight: 'arrow_right',
  arrowBoth: 'arrow_bidirectional_left_right',
  back: 'arrow_left',
  home: 'home',
  search: 'search',
  more: 'more_horizontal',
  close: 'dismiss',
  check: 'checkmark',
  add: 'add',
  subtract: 'subtract',
  settings: 'settings',
  // Views (left rail)
  report: 'data_bar_vertical',
  data: 'table',
  model: 'flowchart',
  queries: 'document_text',
  document: 'document',
  // Status and badges
  hidden: 'eye_off',
  eye: 'eye',
  warning: 'warning',
  error: 'error_circle',
  info: 'info',
  lock: 'lock_closed',
  filter: 'filter',
  // Quick access toolbar
  save: 'save',
  undo: 'arrow_undo',
  redo: 'arrow_redo',
  // File
  open: 'folder_open',
  folder: 'folder',
  recent: 'history',
  export: 'arrow_export',
  exit: 'door_arrow_left',
  // Clipboard
  paste: 'clipboard_paste',
  cut: 'cut',
  copy: 'copy',
  formatPainter: 'paint_brush',
  // Data
  getData: 'database',
  excel: 'document_table',
  onelake: 'database_multiple',
  sqlServer: 'server',
  enterData: 'table_add',
  dataverse: 'cube',
  recentSources: 'history',
  transformData: 'table_edit',
  refresh: 'arrow_clockwise',
  blob: 'cloud_database',
  csv: 'document_table',
  // Insert / calculations
  newVisual: 'data_bar_vertical_add',
  textBox: 'textbox',
  moreVisuals: 'apps_add_in',
  newMeasure: 'calculator',
  quickMeasure: 'flash',
  newColumn: 'table_insert_column',
  newTable: 'table_add',
  calcGroup: 'calculator_multiple',
  sensitivity: 'shield_badge',
  publish: 'cloud_arrow_up',
  /** DashCraft's opt-in AI features (Fluent "Sparkle"). */
  ai: 'sparkle',
  newPage: 'document_add',
  keyInfluencers: 'lightbulb_filament',
  decompositionTree: 'organization_horizontal',
  qna: 'chat_bubbles_question',
  narrative: 'text_description',
  paginated: 'document_bullet_list',
  powerApps: 'apps',
  powerAutomate: 'flow',
  buttons: 'cursor_click',
  shapes: 'shapes',
  image: 'image',
  sparklines: 'data_line',
  // Modeling
  manageRelationships: 'table_link',
  newParameter: 'options',
  pageRefresh: 'arrow_sync_circle',
  manageRoles: 'people_team',
  viewAs: 'person_key',
  qnaSetup: 'chat_settings',
  language: 'local_language',
  linguisticSchema: 'text_grammar_settings',
  markDateTable: 'calendar_ltr',
  sortByColumn: 'arrow_sort',
  dataGroups: 'group',
  tag: 'tag',
  // Design
  themeGallery: 'color',
  customizeTheme: 'color_line',
  downloadTheme: 'arrow_download',
  importTheme: 'arrow_upload',
  pageSize: 'slide_size',
  pageBackground: 'paint_bucket',
  wallpaper: 'image',
  help: 'question_circle',
  // View
  pageView: 'page_fit',
  mobileLayout: 'phone',
  desktop: 'desktop',
  gridlines: 'grid',
  snapToGrid: 'grid_dots',
  lockObjects: 'lock_closed',
  bookmarks: 'bookmark',
  selection: 'layer',
  performance: 'timer',
  syncSlicers: 'arrow_sync',
  zoomIn: 'zoom_in',
  zoomOut: 'zoom_out',
  fitPage: 'full_screen_maximize',
  // Optimize
  pause: 'pause',
  optimizationPresets: 'wand',
  applySlicers: 'cursor_click',
  // Help
  guidedLearning: 'hat_graduation',
  documentation: 'book_open',
  training: 'video',
  support: 'person_support',
  about: 'info',
  blog: 'news',
  community: 'people_community',
  idea: 'lightbulb',
  // Visualizations pane
  build: 'data_bar_vertical_add',
  format: 'paint_brush',
  analytics: 'data_trending',
  // DAX query / TMDL editors
  play: 'play',
  stop: 'stop',
  comment: 'comment',
  uncomment: 'comment_off',
  find: 'search_square',
  replace: 'arrow_swap',
  commandPalette: 'code_text',
  code: 'code',
  // Grid
  sortAsc: 'text_sort_ascending',
  sortDesc: 'text_sort_descending',
  stats: 'data_histogram',
  people: 'people',
  // Visual types (Visualizations gallery)
  vBar: 'data_bar_horizontal',
  vColumn: 'data_bar_vertical',
  vLine: 'data_line',
  vArea: 'data_area',
  vCombo: 'data_bar_vertical_ascending',
  vRibbon: 'data_bar_vertical_star',
  vWaterfall: 'data_waterfall',
  vFunnel: 'data_funnel',
  vScatter: 'data_scatter',
  vPie: 'data_pie',
  vDonut: 'data_usage',
  vTreemap: 'data_treemap',
  vMap: 'map',
  vAzureMap: 'location',
  vGauge: 'gauge',
  vCard: 'number_symbol_square',
  vMultiCard: 'text_bullet_list_square',
  vKpi: 'data_trending',
  vSlicer: 'filter',
  vTable: 'table',
  vMatrix: 'grid',
  vScript: 'code',
  vTextbox: 'textbox',
  vGroup: 'group',
} as const

export type IconName = keyof typeof ICONS

interface Glyph {
  viewBox: string
  body: string
}

const files = import.meta.glob('../assets/icons/*.svg', { query: '?raw', import: 'default', eager: true }) as Record<string, string>

const glyphs = new Map<string, Glyph>()
for (const [path, svg] of Object.entries(files)) {
  const file = path.slice(path.lastIndexOf('/') + 1, -'.svg'.length)
  const viewBox = /viewBox="([^"]+)"/.exec(svg)?.[1] ?? '0 0 20 20'
  const body = svg.replace(/^[\s\S]*?<svg[^>]*>/, '').replace(/<\/svg>\s*$/, '')
  glyphs.set(file, { viewBox, body })
}

/** The glyph for `name` drawn at `size` px: the 24px design above 20px when vendored. */
export function glyph(name: IconName, size: number): Glyph | null {
  const base = ICONS[name]
  return (size > 20 ? glyphs.get(`${base}_24_regular`) : undefined) ?? glyphs.get(`${base}_20_regular`) ?? null
}
