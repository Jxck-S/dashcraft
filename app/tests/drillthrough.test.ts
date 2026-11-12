// Tests of the data point context menu (src/lib/drillthrough.ts): the "Drill through" section built from
// `canvas.drillthroughTargets`. Run by `npm run check`.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { drillthroughMenu } from '../src/lib/drillthrough.ts'

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
