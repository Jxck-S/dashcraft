// Tests of the data point context menu (src/lib/drillthrough.ts): the "Drill through" section built from
// `canvas.drillthroughTargets`. Run by `npm run check`.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { drillMenu, drillthroughMenu } from '../src/lib/drillthrough.ts'

const target = (page: string, displayName: string, keepAllFilters = true) => ({ page, displayName, fields: ['Sales.Region'], keepAllFilters })

test('a data point lists its drillthrough pages under a Drill through header', () => {
  const picked: string[] = []
  const items = drillthroughMenu([target('details', 'Details'), target('stores', 'Stores', false)], (t) => picked.push(t.page))
  assert.deepEqual(
    items.map((i) => [i.label, i.header ?? false]),
    [
      ['Drill through', true],
      ['Details', false],
      ['Stores', false],
    ],
  )
  assert.match(items[1].title ?? '', /Sales\.Region, keeping/)
  assert.doesNotMatch(items[2].title ?? '', /keeping/)
  items[2].onclick?.()
  assert.deepEqual(picked, ['stores'])
})

test('without drillthrough pages the item is disabled', () => {
  const items = drillthroughMenu([], () => assert.fail('nothing to pick'))
  assert.equal(items.length, 1)
  assert.equal(items[0].enabled, false)
})

test('a cross-report drillthrough page names its report', () => {
  const items = drillthroughMenu([{ ...target('detail', 'Store Detail'), report: 'Stores' }], () => {})
  assert.equal(items[1].label, 'Store Detail (Stores)')
  assert.match(items[1].title ?? '', /^In the report Stores\. Filtered on Sales\.Region/)
})

const drill = (first: number, last: number, levels = ['Year', 'Quarter', 'Month']) => ({
  role: 'Category',
  levels,
  first,
  last,
  path: [],
  canDrillUp: last > first || first > 0,
  canDrillDown: last + 1 < levels.length,
})

test('a drillable visual lists the drill commands, enabled by its level', () => {
  const acted: string[] = []
  const items = drillMenu(drill(1, 1), true, (a) => acted.push(a))
  assert.deepEqual(
    items.map((i) => [i.label, i.enabled]),
    [
      ['Drill down', true],
      ['Drill up', true],
      ['Show next level', true],
      ['Expand to next level', true],
    ],
  )
  items[1].onclick?.()
  assert.deepEqual(acted, ['up'])
  const top = drillMenu(drill(0, 0), false, () => {})
  assert.deepEqual(
    top.map((i) => i.enabled),
    [false, false, true, true],
  )
  const lowest = drillMenu(drill(2, 2), true, () => {})
  assert.deepEqual(
    lowest.map((i) => i.enabled),
    [false, true, false, false],
  )
  assert.deepEqual(drillMenu(null, true, () => {}), [])
})