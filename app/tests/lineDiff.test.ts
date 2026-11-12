// Tests of Compare's line diff (src/lib/lineDiff.ts). Run by `npm run check`.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { diffLines } from '../src/lib/lineDiff.ts'

const show = (before: string, after: string) =>
  diffLines(before, after).map((l) => `${l.kind === 'same' ? ' ' : l.kind === 'removed' ? '-' : '+'}${l.text}`)

test('a changed line shows as removed then added, around the kept lines', () => {
  const before = 'measure Total = SUM(Sales[Amount])\n\tformatString: 0\n\tlineageTag: x'
  const after = 'measure Total = SUM(Sales[Amount]) * 2\n\tformatString: 0\n\tlineageTag: x'
  assert.deepEqual(show(before, after), [
    '-measure Total = SUM(Sales[Amount])',
    '+measure Total = SUM(Sales[Amount]) * 2',
    ' \tformatString: 0',
    ' \tlineageTag: x',
  ])
})

test('inserted and deleted lines keep the rest aligned', () => {
  assert.deepEqual(show('a\nb\nc\nd', 'a\nx\nc\nd\ne'), [' a', '-b', '+x', ' c', ' d', '+e'])
})

test('a missing side is all added or all removed', () => {
  assert.deepEqual(show('', 'a\nb'), ['+a', '+b'])
  assert.deepEqual(show('a', ''), ['-a'])
  assert.deepEqual(show('', ''), [])
})
