// M8.1 Auto recovery check of the desktop app (Windows, WebView2): edit a measure, end the app
// without saving (a crash), start it again and get Document recovery; Restore brings the edit
// back unsaved and undoable; after another crash Open saved opens the saved file and keeps the
// edit for the next start, where Discard deletes it for good.
//
//   (cd app && npx tauri build --debug --no-bundle)
//   node scripts/app-checks/recovery.mjs [path/to/dashcraft-app.exe]
import { existsSync, readdirSync } from 'node:fs'
import { join } from 'node:path'
import { aceJs, installChecker } from './a11y.mjs'
import { launch, reporter, selectTotalSales, sleep, title } from './cdp.mjs'

const { check, finish } = reporter()
const PORT = 9337
const EDITED = 'Total Sales = SUM(Orders[Line Amount]) + 0'
const dialog = `document.querySelector('[role="dialog"][aria-label="Document recovery"]')`
const journals = (data) => {
  try {
    return readdirSync(join(data, 'recovery')).length
  } catch {
    return 0
  }
}
const click = (selector, text) =>
  `(() => { const b = [...document.querySelectorAll(${JSON.stringify(selector)})].find((b) => b.textContent.trim() === ${JSON.stringify(text)}); if (!b || b.disabled) return false; b.click(); return true })()`
/** Ends the app as a crash would (no close request, no save question). */
const crash = async (session) => {
  session.close()
  await sleep(1500)
}

let session
try {
  session = await launch(PORT)
  const opened = await session.waitFor(`${title}.includes('AcmeOutfitters') && ${title}`, 60000)
  check('the document opens', opened, opened ?? 'no title')
  check('no Document recovery on a clean start', !(await session.evaluate(`!!${dialog}`)))
  const formula = await selectTotalSales(session)
  await session.evaluate(`(() => { const t = document.querySelector('.formula-bar textarea'); t.focus(); t.select(); return true })()`)
  await session.type(EDITED)
  await session.key('Enter', 'Enter', 13)
  check('the edit marks the title', await session.waitFor(`${title}.includes('*')`))
  await sleep(300)
  check('the edit is journaled', journals(session.data) === 1, `${journals(session.data)} journals`)
  await crash(session)

  session = await launch(PORT, undefined, { keep: true })
  const listed = await session.waitFor(`${dialog}?.textContent`, 60000)
  check('the next start offers Document recovery', listed?.includes('AcmeOutfitters.pbip') && listed.includes('1 unsaved edit'), listed ?? 'no dialog')

  // Accessibility (ADR-0015, A1 to A5) of Document recovery, with the IBM checker when installed.
  await sleep(300)
  check('A2 focus is in Document recovery', await session.evaluate(`${dialog}.contains(document.activeElement)`))
  const name = `(() => { const e = document.activeElement; return (e?.getAttribute('aria-label') || e?.textContent || '').trim() })()`
  const order = []
  for (let i = 0; i < 5; i++) {
    await session.key('Tab', 'Tab', 9)
    order.push(await session.evaluate(name))
  }
  const want = 'Close, Restore AcmeOutfitters.pbip, Open saved AcmeOutfitters.pbip, Discard the unsaved edits of AcmeOutfitters.pbip, Decide later'
  check('A1 Tab goes Close, Restore, Open saved, Discard, Decide later', order.join(', ') === want, order.join(', '))
  await session.key('Tab', 'Tab', 9)
  check('A1 and wraps inside the dialog (no way out behind it)', (await session.evaluate(name)) === 'Close', await session.evaluate(name))
  if (existsSync(aceJs)) {
    await installChecker(session)
    const issues = await session.evaluate(`__a11y.check('[role="dialog"][aria-label="Document recovery"]')`)
    const violations = issues.filter((i) => i.level === 'VIOLATION/FAIL')
    check('A3 checker (WCAG 2.2 A and AA), Document recovery: no violation', violations.length === 0, violations.map((i) => `${i.rule} on ${i.where}: ${i.msg}`).join('; '))
    const text = await session.evaluate(`[...${dialog}.querySelectorAll('h2, p, strong, span, button')].filter((e) => e.textContent.trim()).map((e) => [e.textContent.trim().slice(0, 20), __a11y.ratio(getComputedStyle(e).color, __a11y.bg(e))])`)
    const low = text.filter(([, r]) => r < 4.5)
    check('A4 text at least 4.5:1', low.length === 0, low.map(([t, r]) => `${t} ${r}:1`).join('; ') || `${text.length} elements`)
    const ring = await session.evaluate(`(document.querySelector('[aria-label="Restore AcmeOutfitters.pbip"]').focus(), __a11y.focused())`)
    check('A2 the focused button has a visible ring (2 px, 3:1)', ring?.ok && ring.ratio >= 3, JSON.stringify(ring))
  } else console.log(`SKIP: ${aceJs} missing (A3 checker, A4 contrast)`)
  const moving = await session.evaluate(`[...${dialog}.querySelectorAll('*')].filter((e) => { const s = getComputedStyle(e); return s.animationName !== 'none' || s.transitionDuration.split(',').some((d) => parseFloat(d) > 0) }).map((e) => e.className || e.tagName)`)
  check('A5 no animation or transition', moving.length === 0, moving.join(', '))
  check('Restore', await session.evaluate(click('[aria-label="Document recovery"] button', 'Restore')))
  check('the dialog closes', await session.waitFor(`!${dialog}`))
  check('the restored document is unsaved', await session.waitFor(`${title}.includes('*')`))
  const restoredSaid = await session.waitFor(`(() => { const m = document.querySelector('.status-bar .message')?.textContent ?? ''; return m.startsWith('Restored 1 unsaved edit') && m })()`, 5000)
  check('A1 the status bar says Restored 1 unsaved edit (after the focus moved)', !!restoredSaid, restoredSaid || 'no message')
  const restoreFocus = await session.evaluate(`document.activeElement && document.activeElement !== document.body ? (document.activeElement.getAttribute('role') ?? document.activeElement.tagName) : 'page'`)
  check('A2 after Restore the focus is not lost to the page', restoreFocus !== 'page', restoreFocus)
  const restored = await selectTotalSales(session)
  check('the formula bar shows the recovered edit', restored === EDITED, restored ?? 'no formula')
  const undo = await session.evaluate(`document.querySelector('.qat button[aria-label="Undo"]')?.title ?? ''`)
  check('the recovered edit can be undone', undo.includes("Edit measure 'Total Sales'"), undo)
  await crash(session)

  session = await launch(PORT, undefined, { keep: true })
  check('a restored, unsaved edit is offered again after another crash', await session.waitFor(`${dialog}?.textContent.includes('AcmeOutfitters.pbip')`, 60000))
  check('Open saved', await session.evaluate(click('[aria-label="Document recovery"] button', 'Open saved')))
  check('Open saved closes Document recovery', await session.waitFor(`!${dialog}`))
  check('the saved file is open, without unsaved edits', await session.waitFor(`${title}.includes('AcmeOutfitters') && !${title}.includes('*')`, 30000))
  const saved = await selectTotalSales(session)
  check('the formula bar shows the saved formula', saved === formula, saved ?? 'no formula')
  check('the edit is kept for the next start', journals(session.data) === 1, `${journals(session.data)} journals`)
  await crash(session)

  session = await launch(PORT, undefined, { keep: true })
  check('the kept edit is offered again', await session.waitFor(`${dialog}?.textContent.includes('1 unsaved edit')`, 60000))
  check('Discard asks first', await session.evaluate(click('[aria-label="Document recovery"] button', 'Discard')))
  check('the question appears', await session.waitFor(`!!document.querySelector('[role="dialog"][aria-label="Discard unsaved edits"]')`))
  await session.evaluate(click('[aria-label="Discard unsaved edits"] button', 'Discard'))
  check('Discard closes Document recovery', await session.waitFor(`!${dialog}`))
  await sleep(300)
  check('the journal is deleted', journals(session.data) === 0, `${journals(session.data)} journals`)
  const current = await selectTotalSales(session)
  check('the open document keeps the saved formula', current === formula, current ?? 'no formula')
} catch (e) {
  check('no exception', false, e?.stack ?? String(e))
} finally {
  session?.close()
}
finish()
