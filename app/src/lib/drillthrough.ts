// The data point context menu of the report canvas (no DOM, so `npm run check` tests it in Node).

import type { DrillActionParam, DrillOut, DrillthroughTargetOut } from './ops.gen.ts'

/** A context menu item (a `MenuItem` of `lib/shell`). */
export interface DrillMenuItem {
  label: string
  enabled?: boolean
  title?: string
  header?: boolean
  onclick?: () => void
}

/**
 * The right-click menu of a data point: a "Drill through" section with one item per drillthrough
 * page it can go to (`canvas.drillthroughTargets`), or one disabled item when there is none, as
 * in Power BI's context menu.
 */
export function drillthroughMenu(targets: DrillthroughTargetOut[], pick: (target: DrillthroughTargetOut) => void): DrillMenuItem[] {
  if (targets.length === 0) {
    return [{ label: 'Drill through', enabled: false, title: 'No drillthrough page uses the fields of this data point' }]
  }
  return [
    { label: 'Drill through', header: true },
    ...targets.map((t) => ({
      label: t.report ? `${t.displayName} (${t.report})` : t.displayName,
      title: `${t.report ? `In the report ${t.report}. ` : ''}Filtered on ${t.fields.join(', ')}${t.keepAllFilters ? ', keeping this page’s filters' : ''}`,
      onclick: () => pick(t),
    })),
  ]
}

/** The drill commands of a visual with drill levels, as in Power BI's context menu and visual header. */
export const DRILL_COMMANDS: { action: DrillActionParam; label: string; title: string }[] = [
  { action: 'down', label: 'Drill down', title: 'Show the next level for this data point' },
  { action: 'up', label: 'Drill up', title: 'Show the level above' },
  { action: 'next', label: 'Show next level', title: 'Show the next level for all data points' },
  { action: 'expand', label: 'Expand to next level', title: 'Add the next level to the shown ones' },
]

/** Whether `action` is possible on a visual in drill state `drill` (`onPoint`: a data point was right-clicked). */
export function canDrill(drill: DrillOut, action: DrillActionParam, onPoint: boolean): boolean {
  if (action === 'up') return drill.canDrillUp
  return drill.canDrillDown && (action !== 'down' || onPoint)
}

/**
 * The drill section of a visual's right-click menu (`canvas.drill`): empty when the visual has no
 * levels to drill on; "Drill down" needs a data point.
 */
export function drillMenu(drill: DrillOut | null | undefined, onPoint: boolean, act: (action: DrillActionParam) => void): DrillMenuItem[] {
  if (!drill) return []
  return DRILL_COMMANDS.map((c) => ({
    label: c.label,
    title: c.title,
    enabled: canDrill(drill, c.action, onPoint),
    onclick: () => act(c.action),
  }))
}