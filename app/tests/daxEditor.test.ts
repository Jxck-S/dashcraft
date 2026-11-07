// Tests of the DAX editor's pure helpers (src/lib/daxEditor.ts): keeping ranges aligned while
// typing, the coloured overlay, caret geometry, completion and signature triggers, key commands.
// Run by `npm run check`.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  CODE_BACKGROUND,
  completionEdit,
  editorCommand,
  lineColumn,
  moveActive,
  offsetAt,
  segments,
  shiftOffset,
  shiftSpans,
  signatureParts,
  textDelta,
  TOKEN_COLORS,
  triggers,
  utf16Index,
  withFormatted,
  type KeyLike,
} from '../src/lib/daxEditor.ts'

test('textDelta finds the edit between two texts', () => {
  assert.deepEqual(textDelta('SUM(x)', 'SUMX(x)'), { start: 3, removed: 0, inserted: 1 })
  assert.deepEqual(textDelta('SUMX(x)', 'SUM(x)'), { start: 3, removed: 1, inserted: 0 })
  assert.deepEqual(textDelta('[Tot]', '[Total Sales]'), { start: 4, removed: 0, inserted: 8 })
  assert.deepEqual(textDelta('a + b', 'a * b'), { start: 2, removed: 1, inserted: 1 })
  assert.deepEqual(textDelta('same', 'same'), { start: 4, removed: 0, inserted: 0 })
  // Repeated characters: the prefix is taken first, the suffix never overlaps it.
  assert.deepEqual(textDelta('aa', 'aaa'), { start: 2, removed: 0, inserted: 1 })
})

test('shiftOffset moves offsets after the edit and keeps those before it', () => {
  const insert = { start: 5, removed: 0, inserted: 3 }
  assert.equal(shiftOffset(2, insert), 2)
  assert.equal(shiftOffset(8, insert), 11)
  assert.equal(shiftOffset(5, insert, 'left'), 5)
  assert.equal(shiftOffset(5, insert, 'right'), 8)
  const replace = { start: 5, removed: 4, inserted: 1 }
  assert.equal(shiftOffset(5, replace), 5)
  assert.equal(shiftOffset(7, replace), 6)
  assert.equal(shiftOffset(9, replace), 6)
  assert.equal(shiftOffset(12, replace), 9)
})

test('shiftSpans keeps, moves, grows or drops ranges', () => {
  const spans = [
    { start: 0, end: 3, kind: 'a' },
    { start: 4, end: 10, kind: 'b' },
    { start: 12, end: 14, kind: 'c' },
  ]
  // Typing inside the second range grows it and moves the third.
  assert.deepEqual(shiftSpans(spans, { start: 6, removed: 0, inserted: 2 }), [
    { start: 0, end: 3, kind: 'a' },
    { start: 4, end: 12, kind: 'b' },
    { start: 14, end: 16, kind: 'c' },
  ])
  // Replacing text of the second range drops it until the service answers again.
  assert.deepEqual(shiftSpans(spans, { start: 5, removed: 2, inserted: 1 }), [
    { start: 0, end: 3, kind: 'a' },
    { start: 11, end: 13, kind: 'c' },
  ])
  // Typing right after a range does not grow it.
  assert.deepEqual(shiftSpans(spans, { start: 3, removed: 0, inserted: 1 })[0], { start: 0, end: 3, kind: 'a' })
})

test('segments colour tokens and squiggle problems without losing text', () => {
  const text = 'SUM(Sales[Amount]) + 1'
  const segs = segments(
    text,
    [
      { start: 0, end: 3, kind: 'function' },
      { start: 4, end: 17, kind: 'column' },
      { start: 21, end: 22, kind: 'number' },
    ],
    [{ start: 4, end: 9, severity: 'error' }],
  )
  assert.equal(segs.map((s) => s.text).join(''), text)
  assert.deepEqual(segs[0], { text: 'SUM', token: 'function' })
  assert.deepEqual(segs[1], { text: '(' })
  assert.deepEqual(segs[2], { text: 'Sales', token: 'column', severity: 'error' })
  assert.deepEqual(segs[3], { text: '[Amount]', token: 'column' })
  assert.deepEqual(segs.at(-1), { text: '1', token: 'number' })
})

test('an error wins over a warning where they overlap', () => {
  const segs = segments('ABCDEF', [], [
    { start: 0, end: 4, severity: 'warning' },
    { start: 2, end: 6, severity: 'error' },
  ])
  assert.deepEqual(segs, [
    { text: 'AB', severity: 'warning' },
    { text: 'CDEF', severity: 'error' },
  ])
})

test('an empty problem range squiggles a neighbour, or an extra space at a line end', () => {
  // Before a character: that character.
  assert.deepEqual(segments('A B', [], [{ start: 1, end: 1, severity: 'error' }]), [{ text: 'A' }, { text: ' ', severity: 'error' }, { text: 'B' }])
  // At the end of the text: the character before.
  assert.deepEqual(segments('SUM(', [], [{ start: 4, end: 4, severity: 'error' }]), [{ text: 'SUM' }, { text: '(', severity: 'error' }])
  // On an empty line: an extra space, not part of the formula.
  const segs = segments('A\n\nB', [], [{ start: 2, end: 2, severity: 'warning' }])
  assert.deepEqual(segs, [{ text: 'A\n' }, { text: ' ', severity: 'warning', extra: true }, { text: '\nB' }])
  assert.equal(segs.filter((s) => !s.extra).map((s) => s.text).join(''), 'A\n\nB')
})

test('lineColumn and offsetAt agree, with tabs to the next stop', () => {
  const text = 'M =\n\tVAR x = 1\n\tRETURN x'
  assert.deepEqual(lineColumn(text, 0), { line: 0, column: 0 })
  assert.deepEqual(lineColumn(text, 3), { line: 0, column: 3 })
  assert.deepEqual(lineColumn(text, 5), { line: 1, column: 4 })
  assert.deepEqual(lineColumn(text, 6), { line: 1, column: 5 })
  assert.deepEqual(lineColumn('ab\tc', 3), { line: 0, column: 4 })
  assert.deepEqual(lineColumn(text, 999), lineColumn(text, text.length))
  assert.equal(offsetAt(text, 1, 4), 5)
  assert.equal(offsetAt(text, 1, 1), 4)
  assert.equal(offsetAt(text, 1, 3), 5)
  assert.equal(offsetAt(text, 0, 50), 3)
  assert.equal(offsetAt(text, 9, 0), text.lastIndexOf('\n') + 1)
})

test('typing names, [ or quote asks for completion; ( and , ask for parameter info', () => {
  const typed = (before: string, after: string, open = false) => triggers(after, textDelta(before, after), open)
  assert.deepEqual(typed('SU', 'SUM'), { complete: 'open', signature: false })
  assert.deepEqual(typed('SUM(', 'SUM(['), { complete: 'open', signature: false })
  assert.deepEqual(typed('SUM(', "SUM('"), { complete: 'open', signature: false })
  assert.deepEqual(typed('SUM', 'SUM('), { complete: 'close', signature: true })
  assert.deepEqual(typed('IF(a', 'IF(a,'), { complete: 'close', signature: true })
  assert.deepEqual(typed('a', 'a '), { complete: 'close', signature: false })
  // A number does not open the list, but does not close one the user asked for.
  assert.deepEqual(typed('x + ', 'x + 1'), { complete: 'close', signature: false })
  assert.deepEqual(typed('x + ', 'x + 1', true), { complete: 'open', signature: false })
  // Deleting refreshes an open list and leaves a closed one closed.
  assert.deepEqual(typed('SUMX', 'SUM', true), { complete: 'open', signature: false })
  assert.deepEqual(typed('SUMX', 'SUM'), { complete: 'keep', signature: false })
  // A paste closes it.
  assert.deepEqual(typed('', 'SUM(x)', true), { complete: 'close', signature: false })
})

test('accepting a function does not double the ( that follows', () => {
  assert.deepEqual(completionEdit('SU', 0, 2, 'SUM('), { from: 0, to: 2, text: 'SUM(', caret: 4 })
  assert.deepEqual(completionEdit('SU(x)', 0, 2, 'SUM('), { from: 0, to: 2, text: 'SUM', caret: 4 })
  assert.deepEqual(completionEdit('[Tot', 0, 4, '[Total Sales]'), { from: 0, to: 4, text: '[Total Sales]', caret: 13 })
})

test('the active suggestion wraps with arrows and stops at the ends with pages', () => {
  assert.equal(moveActive(0, 5, 'ArrowDown'), 1)
  assert.equal(moveActive(4, 5, 'ArrowDown'), 0)
  assert.equal(moveActive(0, 5, 'ArrowUp'), 4)
  assert.equal(moveActive(1, 20, 'PageDown'), 9)
  assert.equal(moveActive(15, 20, 'PageDown'), 19)
  assert.equal(moveActive(3, 20, 'PageUp'), 0)
  assert.equal(moveActive(2, 5, 'Home'), 2)
  assert.equal(moveActive(0, 0, 'ArrowDown'), -1)
})

test('the signature splits around the active parameter', () => {
  const sig = {
    signature: 'IF(logicalTest, valueIfTrue, [valueIfFalse])',
    label: 'IF(logicalTest, valueIfTrue, [valueIfFalse]) → any',
    parameters: [
      { name: 'logicalTest', optional: false, repeating: false, labelStart: 3, labelEnd: 14 },
      { name: 'valueIfTrue', optional: false, repeating: false, labelStart: 16, labelEnd: 27 },
      { name: 'valueIfFalse', optional: true, repeating: false, labelStart: 30, labelEnd: 42 },
    ],
  }
  assert.deepEqual(signatureParts({ ...sig, activeParameter: 1 }), ['IF(logicalTest, ', 'valueIfTrue', ', [valueIfFalse]) → any'])
  assert.deepEqual(signatureParts({ ...sig, activeParameter: null }), [sig.label, '', ''])
  assert.deepEqual(signatureParts({ ...sig, activeParameter: 7 }), [sig.label, '', ''])
})

test('utf16Index converts UTF-8 byte offsets', () => {
  assert.equal(utf16Index('abc', 2), 2)
  assert.equal(utf16Index('é→x', 2), 1)
  assert.equal(utf16Index('é→x', 5), 2)
  assert.equal(utf16Index('😀x', 4), 2)
  assert.equal(utf16Index('ab', 10), 2)
})

test('the editor keys are those of Desktop’s formula editor', () => {
  const key = (k: string, mods: Partial<KeyLike> = {}): KeyLike => ({ key: k, ctrlKey: false, metaKey: false, shiftKey: false, altKey: false, ...mods })
  assert.equal(editorCommand(key(' ', { ctrlKey: true })), 'suggest')
  assert.equal(editorCommand(key(' ', { metaKey: true })), 'suggest')
  assert.equal(editorCommand(key(' ', { ctrlKey: true, shiftKey: true })), 'parameterHints')
  assert.equal(editorCommand(key('F', { shiftKey: true, altKey: true })), 'format')
  // With a layout that types another character on Alt: the physical key.
  assert.equal(editorCommand(key('Ï', { shiftKey: true, altKey: true, code: 'KeyF' })), 'format')
  assert.equal(editorCommand(key('k', { ctrlKey: true })), 'chord')
  assert.equal(editorCommand(key('i', { ctrlKey: true }), true), 'hover')
  assert.equal(editorCommand(key('i', { ctrlKey: true })), null)
  assert.equal(editorCommand(key(' ')), null)
  assert.equal(editorCommand(key('f', { altKey: true })), null)
})

test('formatting keeps the name and puts a multi-line expression on the next line', () => {
  assert.equal(withFormatted('M = sum ( x )', 3, 'SUM ( x )'), 'M = SUM ( x )')
  assert.equal(withFormatted('M =  VAR a = 1 RETURN a', 3, 'VAR a = 1\nRETURN\n    a\n'), 'M =\nVAR a = 1\nRETURN\n    a')
  assert.equal(withFormatted('sum(x)', 0, 'SUM ( x )'), 'SUM ( x )')
})

test('token colours are readable on the editor background (WCAG 1.4.3, 4.5:1)', () => {
  const luminance = (hex: string) => {
    const [r, g, b] = [1, 3, 5].map((i) => {
      const c = parseInt(hex.slice(i, i + 2), 16) / 255
      return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
    })
    return 0.2126 * r + 0.7152 * g + 0.0722 * b
  }
  const background = luminance(CODE_BACKGROUND)
  for (const [kind, color] of Object.entries(TOKEN_COLORS)) {
    if (!color) continue
    const ratio = (background + 0.05) / (luminance(color) + 0.05)
    assert.ok(ratio >= 4.5, `${kind} ${color}: ${ratio.toFixed(2)}:1`)
  }
})
