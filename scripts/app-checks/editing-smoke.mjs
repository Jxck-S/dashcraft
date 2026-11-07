// M6.8 editing smoke test of the desktop app (Windows, WebView2): edit a measure in the formula
// bar, undo and redo with the keyboard, keep a measure's DAX that does not parse with a warning,
// save with Ctrl+S, edit a description in the Properties pane, and answer the unsaved-changes
// question on File → Close.
//
//   (cd app && npx tauri build --debug --no-bundle)
//   node scripts/app-checks/editing-smoke.mjs [path/to/dashcraft-app.exe]
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { CTRL, launch, openFileMenu, openProperties, reporter, selectTotalSales, sleep, title } from './cdp.mjs'

const { check, finish } = reporter()
const qat = (label) => `(() => { const b = document.querySelector('.qat button[aria-label="${label}"]'); return b ? { disabled: b.disabled, title: b.title } : null })()`
let session
try {
  session = await launch(9333)
  const { evaluate, waitFor, key, type, copy } = session
  const tmdl = join(copy, 'AcmeOutfitters.SemanticModel', 'definition', 'tables', 'Orders.tmdl')
  const read = () => readFileSync(tmdl, 'utf8').replace(/\r\n/g, '\n')
  const original = read()

  const opened = await waitFor(`${title}.includes('AcmeOutfitters') && ${title}`, 60000)
  check('the document opens with a clean title', opened && !opened.includes('*'), opened)
  check('QAT: Save enabled, Undo and Redo disabled', await evaluate(`(() => { const [s, u, r] = [${qat('Save')}, ${qat('Undo')}, ${qat('Redo')}]; return !!s && !s.disabled && !!u && u.disabled && !!r && r.disabled })()`))

  const formula = await selectTotalSales(session)
  check('the formula bar edits a measure', formula, formula ?? 'no editor')
  await evaluate(`(() => { const t = document.querySelector('.formula-bar textarea'); t.focus(); t.select(); return true })()`)
  await type('Total Sales = SUM(Orders[Line Amount]) + 0')
  await key('Enter', 'Enter', 13)
  const dirty = await waitFor(`${title}.includes('*') && ${title}`)
  check('Enter commits and marks the title', dirty, dirty ?? (await evaluate(title)))
  const undo = await evaluate(qat('Undo'))
  check('QAT Undo names the edit', undo && !undo.disabled && undo.title.includes("Edit measure 'Total Sales'"), undo?.title)

  await evaluate(`document.activeElement?.blur(), true`)
  await key('z', 'KeyZ', 90, CTRL)
  const clean = await waitFor(`!${title}.includes('*') && ${title}`)
  check('Ctrl+Z returns to the saved state', clean, clean ?? (await evaluate(title)))
  check('the formula bar shows the original', await waitFor(`document.querySelector('.formula-bar textarea')?.value === ${JSON.stringify(formula)}`))
  await key('y', 'KeyY', 89, CTRL)
  check('Ctrl+Y redoes', await waitFor(`${title}.includes('*')`))
  check('QAT Redo disabled after the redo', (await evaluate(qat('Redo')))?.disabled === true)

  await evaluate(`(() => { const t = document.querySelector('.formula-bar textarea'); t.focus(); t.select(); return true })()`)
  await type('Total Sales = SUM(Orders[Line Amount]')
  await key('Enter', 'Enter', 13)
  const note = await waitFor(`[...document.querySelectorAll('.toast.warning')].map((t) => t.textContent).find((t) => t.includes('does not parse'))`)
  check('a measure keeps DAX that does not parse, with a warning', note, note ?? 'no warning toast')
  await evaluate(`document.activeElement?.blur(), true`)
  await key('z', 'KeyZ', 90, CTRL)
  check('Ctrl+Z restores the parsed expression', await waitFor(`document.querySelector('.formula-bar textarea')?.value.endsWith('SUM(Orders[Line Amount]) + 0')`))

  await key('s', 'KeyS', 83, CTRL)
  check('Ctrl+S saves and clears the marker', await waitFor(`!${title}.includes('*')`))
  await sleep(300)
  const saved = read()
  check('Orders.tmdl has the new expression', saved.includes("\tmeasure 'Total Sales' = SUM(Orders[Line Amount]) + 0\n") && !original.includes("'Total Sales' = SUM(Orders[Line Amount]) + 0"))

  check('the Properties pane shows the editable fields', await openProperties(session))
  await evaluate(`(() => { const t = document.querySelector('#prop-description'); t.focus(); t.select(); return true })()`)
  await type('Smoke test description')
  await key('Enter', 'Enter', 13, CTRL)
  check('Ctrl+Enter commits the description', await waitFor(`${title}.includes('*')`))
  check('QAT Undo names the description edit', ((await evaluate(qat('Undo')))?.title ?? '').includes("Change the description of measure 'Total Sales'"))

  const closeDoc = `[...document.querySelectorAll('.backstage button')].find((b) => b.textContent.trim() === 'Close').click(), true`
  const unsaved = `document.querySelector('[role="dialog"][aria-label="Unsaved changes"]')`
  const choose = (label) => `[...${unsaved}.querySelectorAll('button')].find((b) => b.textContent.trim() === ${JSON.stringify(label)}).click(), true`
  check('File opens', await openFileMenu(session))
  check('File → Save is enabled', await evaluate(`[...document.querySelectorAll('.backstage button')].some((b) => b.textContent.trim() === 'Save' && !b.disabled)`))
  await evaluate(closeDoc)
  const question = await waitFor(`${unsaved}?.textContent`, 5000)
  check('File → Close asks to save', question?.includes("Don't save"), question?.slice(0, 80))
  await evaluate(choose('Cancel'))
  await sleep(500)
  check('Cancel keeps the document open and dirty', (await evaluate(title)).includes('*') && !(await evaluate(`!!${unsaved}`)))
  await openFileMenu(session)
  await evaluate(closeDoc)
  await waitFor(`!!${unsaved}`, 5000)
  await evaluate(choose('Save'))
  check('Save in the question saves and closes', await waitFor(`!${title}.includes('AcmeOutfitters')`, 15000))
  await sleep(300)
  const final = read()
  check('Orders.tmdl has the description', final.includes("\t/// Smoke test description\n\tmeasure 'Total Sales'"))
  const changed = original.split('\n').filter((line, i) => final.split('\n')[i] !== line).length
  check('only the two edited lines changed', changed === 2 && original.split('\n').length === final.split('\n').length, `${changed} lines`)
} catch (e) {
  check('no error', false, e.message)
} finally {
  session?.close()
}
finish()
