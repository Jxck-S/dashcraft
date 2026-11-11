// Tests of the modal-dialog focus rule (src/lib/focus.ts): where Tab and Shift+Tab go at the
// edges of a dialog. Run by `npm run check`.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { wrapFocus } from '../src/lib/focus.ts'

test('Tab and Shift+Tab wrap at the edges of a modal and stay with the browser inside it', () => {
  const items = ['Back', 'Home', 'Close']
  assert.equal(wrapFocus(items, 'Close', false), 'Back')
  assert.equal(wrapFocus(items, 'Back', true), 'Close')
  assert.equal(wrapFocus(items, 'Back', false), null)
  assert.equal(wrapFocus(items, 'Home', true), null)
})

test('from the dialog itself or from outside, Tab enters at the first control and Shift+Tab at the last', () => {
  const items = ['Back', 'Home', 'Close']
  assert.equal(wrapFocus(items, null, false), 'Back')
  assert.equal(wrapFocus(items, 'File button behind', false), 'Back')
  assert.equal(wrapFocus(items, 'File button behind', true), 'Close')
  assert.equal(wrapFocus(['Only'], 'Only', false), 'Only')
  assert.equal(wrapFocus([], null, false), null)
})
