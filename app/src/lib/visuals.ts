// Visual types: the Visualizations pane gallery (Desktop's order) and icons per type.

import type { IconName } from './icons'

export interface GalleryVisual {
  /** PBIR `visualType`s shown by this gallery entry. */
  types: string[]
  label: string
  icon: IconName
}

/** The built-in visuals of Desktop's Visualizations pane, in gallery order. */
export const GALLERY: GalleryVisual[] = [
  { types: ['barChart'], label: 'Stacked bar chart', icon: 'vBar' },
  { types: ['columnChart'], label: 'Stacked column chart', icon: 'vColumn' },
  { types: ['clusteredBarChart'], label: 'Clustered bar chart', icon: 'vBar' },
  { types: ['clusteredColumnChart'], label: 'Clustered column chart', icon: 'vColumn' },
  { types: ['hundredPercentStackedBarChart'], label: '100% stacked bar chart', icon: 'vBar' },
  { types: ['hundredPercentStackedColumnChart'], label: '100% stacked column chart', icon: 'vColumn' },
  { types: ['lineChart'], label: 'Line chart', icon: 'vLine' },
  { types: ['areaChart'], label: 'Area chart', icon: 'vArea' },
  { types: ['stackedAreaChart', 'hundredPercentStackedAreaChart'], label: 'Stacked area chart', icon: 'vArea' },
  { types: ['lineStackedColumnComboChart'], label: 'Line and stacked column chart', icon: 'vCombo' },
  { types: ['lineClusteredColumnComboChart'], label: 'Line and clustered column chart', icon: 'vCombo' },
  { types: ['ribbonChart'], label: 'Ribbon chart', icon: 'vRibbon' },
  { types: ['waterfallChart'], label: 'Waterfall chart', icon: 'vWaterfall' },
  { types: ['funnel'], label: 'Funnel', icon: 'vFunnel' },
  { types: ['scatterChart'], label: 'Scatter chart', icon: 'vScatter' },
  { types: ['pieChart'], label: 'Pie chart', icon: 'vPie' },
  { types: ['donutChart'], label: 'Donut chart', icon: 'vDonut' },
  { types: ['treemap'], label: 'Treemap', icon: 'vTreemap' },
  { types: ['map'], label: 'Map', icon: 'vMap' },
  { types: ['filledMap', 'shapeMap'], label: 'Filled map', icon: 'vMap' },
  { types: ['azureMap'], label: 'Azure map', icon: 'vAzureMap' },
  { types: ['gauge'], label: 'Gauge', icon: 'vGauge' },
  { types: ['cardVisual'], label: 'Card', icon: 'vCard' },
  { types: ['card'], label: 'Card (legacy)', icon: 'vCard' },
  { types: ['multiRowCard'], label: 'Multi-row card', icon: 'vMultiCard' },
  { types: ['kpi'], label: 'KPI', icon: 'vKpi' },
  { types: ['slicer', 'advancedSlicerVisual', 'listSlicer', 'textSlicer'], label: 'Slicer', icon: 'vSlicer' },
  { types: ['tableEx'], label: 'Table', icon: 'vTable' },
  { types: ['pivotTable'], label: 'Matrix', icon: 'vMatrix' },
  { types: ['scriptVisual'], label: 'R script visual', icon: 'vScript' },
  { types: ['pythonVisual'], label: 'Python visual', icon: 'vScript' },
  { types: ['keyDriversVisual'], label: 'Key influencers', icon: 'keyInfluencers' },
  { types: ['decompositionTreeVisual'], label: 'Decomposition tree', icon: 'decompositionTree' },
  { types: ['qnaVisual'], label: 'Q&A', icon: 'qna' },
  { types: ['aiNarratives', 'narrativeVisual'], label: 'Narrative', icon: 'narrative' },
  { types: ['paginatedReportVisual'], label: 'Paginated report', icon: 'paginated' },
  { types: ['scorecard'], label: 'Metrics', icon: 'vKpi' },
  { types: ['powerApps'], label: 'Power Apps for Power BI', icon: 'powerApps' },
  { types: ['powerAutomate'], label: 'Power Automate for Power BI', icon: 'powerAutomate' },
]

const OTHER: Record<string, { label: string; icon: IconName }> = {
  textbox: { label: 'Text box', icon: 'vTextbox' },
  image: { label: 'Image', icon: 'image' },
  shape: { label: 'Shape', icon: 'shapes' },
  basicShape: { label: 'Shape', icon: 'shapes' },
  actionButton: { label: 'Button', icon: 'buttons' },
  bookmarkNavigator: { label: 'Bookmark navigator', icon: 'bookmarks' },
  pageNavigator: { label: 'Page navigator', icon: 'buttons' },
  group: { label: 'Group', icon: 'vGroup' },
}

/** Gallery entry of a visual type, if it is a built-in gallery visual. */
export function galleryEntry(visualType: string): GalleryVisual | null {
  return GALLERY.find((g) => g.types.includes(visualType)) ?? null
}

/** Display name and icon of any visual type (custom visuals: the type name, generic icon). */
export function visualKind(visualType: string): { label: string; icon: IconName } {
  const g = galleryEntry(visualType)
  if (g) return { label: g.label, icon: g.icon }
  return OTHER[visualType] ?? { label: visualType || 'Visual', icon: 'moreVisuals' }
}
