// Tests of the range and relative date slicer controls (src/lib/slicerRange.ts). Run by `npm run check`.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { handleX, isoToOaDate, oaDateToIso, parseRangeInput, periodFor, relativePeriods, step, valueAt } from '../src/lib/slicerRange.ts'

const track = { x: 100, y: 50, width: 200, height: 10 }

test('OADate day numbers convert to and from ISO dates', () => {
  assert.equal(oaDateToIso(45658), '2025-01-01')
  assert.equal(oaDateToIso(45658.75), '2025-01-01')
  assert.equal(isoToOaDate('2025-01-01'), 45658)
  assert.equal(isoToOaDate('1899-12-30'), 0)
})

test('typed bounds parse as dates or numbers', () => {
  const date = { isDate: true }
  const number = { isDate: false }
  assert.equal(parseRangeInput(date, '2025-03-01'), '2025-03-01')
  assert.equal(parseRangeInput(date, '3/1/2025'), '2025-03-01')
  assert.equal(parseRangeInput(date, '2/30/2025'), null)
  assert.equal(parseRangeInput(date, 'soon'), null)
  assert.equal(parseRangeInput(number, '1,250.5'), 1250.5)
  assert.equal(parseRangeInput(number, ''), null)
  assert.equal(parseRangeInput(number, 'x'), null)
})

test('handles map values to the track and back', () => {
  const range = { track, min: 0, max: 50, integer: true, isDate: false }
  assert.equal(handleX(range, 25), 200)
  assert.equal(handleX(range, 80), 300)
  assert.equal(valueAt(range, 200), 25)
  assert.equal(valueAt(range, 203), 26)
  assert.equal(valueAt(range, 0), 0)
  assert.equal(valueAt({ ...range, integer: false }, 203), 25.75)
  assert.equal(valueAt({ ...range, track: null }, 200), null)
  assert.equal(step(range), 1)
  assert.equal(step({ ...range, integer: false }), 0.5)
})

test('relative periods follow the anchor and the time mode', () => {
  assert.deepEqual(
    relativePeriods('this', false).map((p) => p.value),
    ['days', 'weeks', 'months', 'years'],
  )
  assert.equal(relativePeriods('last', false).length, 7)
  assert.deepEqual(
    relativePeriods('next', true).map((p) => p.label),
    ['Minutes', 'Hours'],
  )
  assert.equal(periodFor('this', false, 'calendarMonths'), 'months')
  assert.equal(periodFor('last', false, 'calendarMonths'), 'calendarMonths')
  assert.equal(periodFor('last', true, 'years'), 'minutes')
})
