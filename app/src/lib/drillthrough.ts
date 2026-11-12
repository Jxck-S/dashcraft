// The data point context menu of the report canvas (no DOM, so `npm run check` tests it in Node).

import type { DrillthroughTargetOut } from './ops.gen.ts'

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
      label: t.displayName,
      title: `Filtered on ${t.fields.join(', ')}${t.keepAllFilters ? ', keeping this page’s filters' : ''}`,
      onclick: () => pick(t),
    })),
  ]
}
