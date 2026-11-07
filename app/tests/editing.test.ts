// Tests of the editing UI's pure helpers (src/lib/editing.ts): formula bar parsing, window title,
// Save / Save as decisions, Quick Access Toolbar tooltips and shortcuts. Run by `npm run check`.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import {
  canSaveInPlace,
  changedValue,
  editShortcut,
  folderNotEmpty,
  formulaText,
  historyTitles,
  isEditableTarget,
  needsSaveAs,
  parseFormula,
  windowTitle,
} from '../src/lib/editing.ts'

test('a formula splits into name and expression at the first =', () => {
  assert.deepEqual(parseFormula('Total Sales = SUM(Sales[Amount])'), { ok: true, name: 'Total Sales', expression: 'SUM(Sales[Amount])' })
  assert.deepEqual(parseFormula('Ratio = IF([A] = 0, BLANK(), [B] / [A])'), {
    ok: true,
    name: 'Ratio',
    expression: 'IF([A] = 0, BLANK(), [B] / [A])',
  })
})

test('a multi-line expression keeps its lines and indentation', () => {
  const parsed = parseFormula('Margin =\n    VAR c = [Cost]\n    RETURN [Sales] - c\n')
  assert.deepEqual(parsed, { ok: true, name: 'Margin', expression: '    VAR c = [Cost]\n    RETURN [Sales] - c' })
})

test('a bracketed name may contain = and escaped ]', () => {
  assert.deepEqual(parseFormula('[A = B]] x] = 1'), { ok: true, name: 'A = B] x', expression: '1' })
  assert.deepEqual(parseFormula('  [Total Sales] = 2'), { ok: true, name: 'Total Sales', expression: '2' })
})

test('formulaText and parseFormula round-trip awkward names', () => {
  for (const name of ['Total Sales', 'A = B', '[x]', 'a]b = c', ' padded ']) {
    const parsed = parseFormula(formulaText(name, 'SUM(T[c])'))
    assert.deepEqual(parsed, { ok: true, name, expression: 'SUM(T[c])' }, name)
  }
  assert.equal(formulaText('Total', '\nSUM(T[c])'), 'Total = SUM(T[c])')
})

test('malformed formulas are refused with a message', () => {
  for (const text of ['SUM(T[c])', ' = 1', 'Name =   ', 'Two\nLines = 1', '[Open = 1', '[Name] 1']) {
    const parsed = parseFormula(text)
    assert.equal(parsed.ok, false, text)
    if (!parsed.ok) assert.ok(parsed.error.length > 0)
  }
})

test('the window title marks unsaved edits', () => {
  assert.equal(windowTitle('Acme.pbip', false), 'Acme.pbip - DashCraft')
  assert.equal(windowTitle('Acme.pbip', true), '*Acme.pbip - DashCraft')
  assert.equal(windowTitle(null, true), 'DashCraft')
})

test('only a PBIP project saves in place; refusals are recognised', () => {
  assert.equal(canSaveInPlace('pbip'), true)
  for (const format of ['pbix', 'pbit', 'tmdl', null, undefined]) assert.equal(canSaveInPlace(format), false)
  assert.equal(needsSaveAs({ kind: 'unsupported', error: 'pbix documents cannot be saved in place; use doc.saveAs' }), true)
  assert.equal(needsSaveAs({ kind: 'unsupported', error: 'something else' }), false)
  assert.equal(folderNotEmpty({ kind: 'io', error: 'out is not empty (set overwrite to replace it)' }), true)
  assert.equal(folderNotEmpty({ kind: 'conflict', error: 'is not empty' }), false)
})

test('Quick Access Toolbar tooltips name the step', () => {
  const history = { dirty: true, canUndo: true, canRedo: true, undoLabel: "Edit measure 'Total'", redoLabel: 'Rename' }
  assert.deepEqual(historyTitles(history), { undo: "Undo Edit measure 'Total' (Ctrl+Z)", redo: 'Redo Rename (Ctrl+Y)' })
  assert.deepEqual(historyTitles(history, true), { undo: "Undo Edit measure 'Total' (⌘Z)", redo: 'Redo Rename (⌘⇧Z)' })
  assert.deepEqual(historyTitles(null), { undo: 'Nothing to undo (Ctrl+Z)', redo: 'Nothing to redo (Ctrl+Y)' })
})

const key = (k: string, mods: { ctrl?: boolean; meta?: boolean; shift?: boolean; alt?: boolean } = {}, target: object | null = null) => ({
  key: k,
  ctrlKey: mods.ctrl ?? false,
  metaKey: mods.meta ?? false,
  shiftKey: mods.shift ?? false,
  altKey: mods.alt ?? false,
  target: target as EventTarget | null,
})

/** A stand-in for a DOM element: only the fields `isEditableTarget` reads. */
const el = (fields: { tagName: string; type?: string; readOnly?: boolean; isContentEditable?: boolean }) => fields as unknown as EventTarget

test('shortcuts map to document commands', () => {
  assert.equal(editShortcut(key('s', { ctrl: true })), 'save')
  assert.equal(editShortcut(key('S', { ctrl: true, shift: true })), 'saveAs')
  assert.equal(editShortcut(key('z', { meta: true })), 'undo')
  assert.equal(editShortcut(key('Z', { ctrl: true, shift: true })), 'redo')
  assert.equal(editShortcut(key('y', { ctrl: true })), 'redo')
  assert.equal(editShortcut(key('z')), null)
  assert.equal(editShortcut(key('z', { ctrl: true, alt: true })), null)
  assert.equal(editShortcut(key('c', { ctrl: true })), null)
})

test('text fields keep their own undo; Save works everywhere', () => {
  const input = el({ tagName: 'INPUT', type: 'text' })
  const textarea = el({ tagName: 'textarea' })
  assert.equal(isEditableTarget(input), true)
  assert.equal(isEditableTarget(textarea), true)
  assert.equal(isEditableTarget(el({ tagName: 'DIV', isContentEditable: true })), true)
  assert.equal(isEditableTarget(el({ tagName: 'INPUT', type: 'checkbox' })), false)
  assert.equal(isEditableTarget(el({ tagName: 'TEXTAREA', readOnly: true })), false)
  assert.equal(isEditableTarget(el({ tagName: 'BUTTON' })), false)
  assert.equal(isEditableTarget(null), false)
  assert.equal(editShortcut(key('z', { ctrl: true }, input)), null)
  assert.equal(editShortcut(key('y', { ctrl: true }, textarea)), null)
  assert.equal(editShortcut(key('s', { ctrl: true }, input)), 'save')
})

test('a Properties field reports only a change; blank clears', () => {
  assert.equal(changedValue('0.00', '0.00'), undefined)
  assert.equal(changedValue(null, ''), undefined)
  assert.equal(changedValue(undefined, '   '), undefined)
  assert.equal(changedValue('Sales', '  '), '')
  assert.equal(changedValue('Sales', 'Finance'), 'Finance')
  assert.equal(changedValue(null, 'New'), 'New')
})
