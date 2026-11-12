// M8.2 External changes check of the desktop app (Windows, WebView2): another program edits a
// TMDL file of the open project; the notification bar appears within two seconds; Compare lists
// the changed measure; Keep mine hides the bar and Save then refuses to overwrite silently; a
// second external edit brings the bar back and Reload (discarding the unsaved edit) rereads it.
// File > Compare then compares with the files on disk (the same: no Reload / Keep mine) and with a
// changed copy of the project folder, picked in the native dialog by pick.ps1 (UI Automation).
// Accessibility A1, A2 and A5 of the bar and Compare always; A3 (the IBM checker, also on File >
// Compare) and A4 when the checker is installed.
//
//   (cd app && npx tauri build --debug --no-bundle)
//   npm install --prefix target/tmp/a11y --no-save accessibility-checker-engine   # optional
//   node scripts/app-checks/external-changes.mjs [path/to/dashcraft-app.exe]
import { spawnSync } from 'node:child_process'
import { cpSync, existsSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { aceJs, installChecker } from './a11y.mjs'
import { CTRL, launch, openFileMenu, reporter, root, selectTotalSales, sleep, title } from './cdp.mjs'

const { check, finish } = reporter()
const PORT = 9338
const SAVED = 'Total Sales = SUM(Orders[Line Amount])'
const EXTERNAL = 'Total Sales = SUM(Orders[Line Amount]) * 1'
const MINE = 'Total Sales = SUM(Orders[Line Amount]) + 0'
const bar = `document.querySelector('.bar.external')`
const compare = `document.querySelector('[role="dialog"][aria-label^="Compare "]')`
const click = (selector, text) =>
  `(() => { const b = [...document.querySelectorAll(${JSON.stringify(selector)})].find((b) => b.textContent.trim() === ${JSON.stringify(text)}); if (!b || b.disabled) return false; b.click(); return true })()`
/** Clicks the File menu action whose heading is `heading`. */
const action = (heading) =>
  `(() => { const b = [...document.querySelectorAll('.backstage .action')].find((b) => b.querySelector('strong')?.textContent.trim() === ${JSON.stringify(heading)}); if (!b || b.disabled) return false; b.click(); return true })()`
/** Edits a file of the project copy as another program would. */
const edit = (copy, table, from, to) => {
  const file = join(copy, 'AcmeOutfitters.SemanticModel', 'definition', 'tables', `${table}.tmdl`)
  const text = readFileSync(file, 'utf8')
  if (!text.includes(from)) throw new Error(`${from} not in ${table}.tmdl`)
  writeFileSync(file, text.replace(from, to))
}

let session
try {
  session = await launch(PORT)
  const { evaluate, waitFor, key, type, copy } = session
  const opened = await waitFor(`${title}.includes('AcmeOutfitters') && ${title}`, 60000)
  check('the document opens', opened, opened ?? 'no title')
  const formula = await selectTotalSales(session)
  check('the formula bar shows the saved measure', formula === SAVED, formula ?? 'no formula')
  await sleep(2000)
  check('no notification bar while nothing changed', !(await evaluate(`!!${bar}`)))

  edit(copy, 'Orders', "measure 'Total Sales' = SUM(Orders[Line Amount])", "measure 'Total Sales' = SUM(Orders[Line Amount]) * 1")
  const started = Date.now()
  const shown = await waitFor(`${bar}?.textContent`, 5000)
  const elapsed = Date.now() - started
  check('the notification bar appears within two seconds', shown && elapsed <= 2000, `${elapsed} ms`)
  check('the bar names the changed file', shown?.includes('Orders.tmdl'), shown ?? 'no bar')
  const role = await evaluate(`${bar}?.getAttribute('role')`)
  check('the bar is a status region (announced politely)', role === 'status', role ?? 'none')
  // A status region inserted with its text is not announced: the status bar's live region says it.
  const said = await evaluate(`document.querySelector('.status-bar .message')?.textContent ?? ''`)
  check('the status bar announces the change and the bar', said.includes('Files changed on disk') && said.includes('Orders.tmdl'), said || 'no message')

  // Accessibility (ADR-0015, A1 to A5) of the bar and Compare, with the IBM checker when installed.
  const focusedName = `(document.activeElement?.textContent ?? '').trim()`
  await evaluate(`${bar}.querySelector('button').focus(), true`)
  const order = [await evaluate(focusedName)]
  for (let i = 0; i < 2; i++) {
    await key('Tab', 'Tab', 9)
    order.push(await evaluate(focusedName))
  }
  check('A1 Tab goes Reload, Keep mine, Compare', order.join(', ') === 'Reload, Keep mine, Compare', order.join(', '))
  await evaluate(`document.activeElement.click(), true`)
  await waitFor(`!!${compare}`, 5000)
  await sleep(300)
  check('A2 focus moves into Compare', await evaluate(`${compare}.contains(document.activeElement)`))
  await waitFor(`${compare}?.querySelector('.objects')`, 15000)
  if (existsSync(aceJs)) {
    await installChecker(session)
    for (const [surface, scope] of [
      ['notification bar', '.bar.external'],
      ['Compare', '[role="dialog"][aria-label^="Compare "]'],
    ]) {
      const issues = await evaluate(`__a11y.check(${JSON.stringify(scope)})`)
      const violations = issues.filter((i) => i.level === 'VIOLATION/FAIL')
      check(`A3 checker (WCAG 2.2 A and AA), ${surface}: no violation`, violations.length === 0, violations.map((i) => `${i.rule} on ${i.where}: ${i.msg}`).join('; '))
    }
    const text = await evaluate(`[...document.querySelectorAll('.bar.external span, .bar.external button, [role="dialog"][aria-label^="Compare "] :is(h2, h3, p, summary, th, td, pre, button, code, .line)')].map((e) => [e.textContent.trim().slice(0, 20), __a11y.ratio(getComputedStyle(e).color, __a11y.bg(e))])`)
    const low = text.filter(([, r]) => r < 4.5)
    check('A4 text at least 4.5:1', low.length === 0, low.map(([t, r]) => `${t} ${r}:1`).join('; ') || `${text.length} elements`)
  } else console.log(`SKIP: ${aceJs} missing (A3 checker, A4 contrast)`)
  const moving = await evaluate(`[...document.querySelectorAll('.bar.external, .bar.external *, [role="dialog"][aria-label^="Compare "] *')].filter((e) => { const s = getComputedStyle(e); return s.animationName !== 'none' || s.transitionDuration.split(',').some((d) => parseFloat(d) > 0) }).map((e) => e.className || e.tagName)`)
  check('A5 no animation or transition', moving.length === 0, moving.join(', '))
  await key('Escape', 'Escape', 27)
  await sleep(300)
  check('A1 Esc closes Compare and focus goes back to Compare', !(await evaluate(`!!${compare}`)) && (await evaluate(focusedName)) === 'Compare')

  check('Compare', await evaluate(click('.bar.external button', 'Compare')))
  const listed = await waitFor(`${compare}?.querySelector('.objects') && ${compare}.textContent`, 15000)
  check('Compare lists the changed measure as different', listed?.includes('Orders[Total Sales]') && listed.includes('Different'), listed?.slice(0, 200) ?? 'no list')
  check('Compare shows both expressions', listed?.includes('SUM(Orders[Line Amount]) * 1') && listed.includes('expression'), listed?.slice(0, 200) ?? '')
  check('Compare names the changed file', listed?.includes('Orders.tmdl'))
  const lines = await evaluate(`[...${compare}.querySelectorAll('.snippet .line')].map((l) => l.textContent)`)
  check('Compare shows the TMDL: the disk line minus, this line plus', lines?.includes("-measure 'Total Sales' = SUM(Orders[Line Amount]) * 1") && lines.includes("+measure 'Total Sales' = SUM(Orders[Line Amount])"), JSON.stringify(lines?.slice(0, 4)))
  check('unchanged TMDL lines are kept once', lines?.some((l) => l.startsWith(' \tformatString')), JSON.stringify(lines?.slice(0, 6)))
  check('Keep mine in Compare', await evaluate(click('[role="dialog"][aria-label^="Compare "] button', 'Keep mine')))
  check('Keep mine closes Compare and the bar', await waitFor(`!${compare} && !${bar}`, 5000))
  await sleep(300)
  const after = await evaluate(`document.activeElement?.getAttribute('role') === 'tab' ? 'ribbon tab' : (document.activeElement?.tagName ?? 'none')`)
  check('A2 focus goes to the ribbon tab before the closed bar, not the page', after === 'ribbon tab', after)
  await sleep(2000)
  check('the kept changes are not reported again', !(await evaluate(`!!${bar}`)))

  await evaluate(`(() => { const t = document.querySelector('.formula-bar textarea'); t.focus(); t.select(); return true })()`)
  await type(MINE)
  await key('Enter', 'Enter', 13)
  check('an in-app edit marks the title', await waitFor(`${title}.includes('*')`))
  await evaluate(`document.activeElement?.blur(), true`)
  await key('s', 'KeyS', 83, CTRL)
  const question = await waitFor(`document.querySelector('[role="dialog"][aria-label="Files changed on disk"]')?.textContent`, 10000)
  check('Save asks before overwriting the external change', question?.includes('Overwrite'), question ?? 'no question')
  await evaluate(click('[aria-label="Files changed on disk"] button', 'Cancel'))
  check('Cancel keeps the edit unsaved', await waitFor(`!document.querySelector('[aria-label="Files changed on disk"]') && ${title}.includes('*')`, 5000))
  check('the file on disk keeps the external change', readFileSync(join(copy, 'AcmeOutfitters.SemanticModel', 'definition', 'tables', 'Orders.tmdl'), 'utf8').includes("'Total Sales' = SUM(Orders[Line Amount]) * 1"))

  edit(copy, 'Customers', "measure 'Customer Count' = COUNTROWS(Customers)", "measure 'Customer Count' = COUNTROWS(Customers) + 0")
  const again = await waitFor(`${bar}?.textContent`, 5000)
  check('another external change brings the bar back', again?.includes('2 project files') && again.includes('unsaved edits'), again ?? 'no bar')
  check('Reload', await evaluate(click('.bar.external button', 'Reload')))
  const ask = await waitFor(`document.querySelector('[role="dialog"][aria-label="Reload from disk"]')?.textContent`, 5000)
  check('Reload asks before discarding the unsaved edit', ask?.includes('discards your unsaved edits'), ask ?? 'no question')
  await evaluate(click('[aria-label="Reload from disk"] button', 'Reload and discard edits'))
  check('the reloaded document is saved (no *)', await waitFor(`!document.querySelector('[aria-label="Reload from disk"]') && ${title}.includes('AcmeOutfitters') && !${title}.includes('*')`, 30000))
  check('the bar is gone', await waitFor(`!${bar}`, 5000))
  const reloaded = await selectTotalSales(session)
  check('the formula bar shows the version on disk', reloaded === EXTERNAL, reloaded ?? 'no formula')
  await sleep(2000)
  check('after Reload nothing is reported', !(await evaluate(`!!${bar}`)))

  // File > Compare (M8.3): with the files on disk (nothing to resolve) and with another folder, a
  // copy of the project where another program changed the measure, picked in the native folder
  // dialog through UI Automation (pick.ps1).
  const other = join(copy, '..', 'acme-outfitters-other')
  rmSync(other, { recursive: true, force: true })
  cpSync(copy, other, { recursive: true })
  edit(other, 'Orders', "measure 'Total Sales' = SUM(Orders[Line Amount]) * 1", "measure 'Total Sales' = SUM(Orders[Line Amount]) * 2")
  const compareSection = async () => (await openFileMenu(session)) && evaluate(click('.backstage .nav-item', 'Compare')) && waitFor(`!!document.querySelector('.backstage .page h1')?.textContent.includes('Compare')`, 3000)
  check('File > Compare', await compareSection())
  if (existsSync(aceJs)) {
    const issues = await evaluate(`__a11y.check('.backstage')`)
    const violations = issues.filter((i) => i.level === 'VIOLATION/FAIL')
    check('A3 checker (WCAG 2.2 A and AA), File > Compare: no violation', violations.length === 0, violations.map((i) => `${i.rule} on ${i.where}: ${i.msg}`).join('; '))
  }
  check('With the files on disk', await evaluate(action('With the files on disk')))
  const same = await waitFor(`!document.querySelector('.backstage') && ${compare}?.querySelector('[role="status"]')?.textContent.includes('same') && ${compare}.textContent`, 15000)
  check('the saved document is the same as on disk', same?.includes('The model and reports are the same on disk and here.'), same?.slice(0, 200) ?? 'no dialog')
  const buttons = await evaluate(`[...${compare}.querySelectorAll('button')].map((b) => b.textContent.trim()).filter((t) => t)`)
  check('no Reload or Keep mine without an external change', !buttons.includes('Reload from disk') && !buttons.includes('Keep mine'), buttons.join(', '))
  check('no changed-files list', !(await evaluate(`!!${compare}.querySelector('#compare-files')`)))
  await key('Escape', 'Escape', 27)
  check('Esc closes it', await waitFor(`!${compare}`, 3000))

  check('File > Compare again', await compareSection())
  check('With another folder', await evaluate(action('With another folder')))
  const pick = spawnSync('pwsh', ['-NoProfile', '-File', join(root, 'scripts', 'app-checks', 'pick.ps1'), String(session.app.pid), 'Compare with a PBIP or TMDL folder', other], { encoding: 'utf8' })
  const picked = `${pick.stdout}${pick.stderr}`.trim()
  check('the folder is picked in the native dialog', picked === 'PICKED', picked || `status ${pick.status}, ${pick.error ?? 'no error'}`)
  const versus = await waitFor(`${compare}?.querySelector('.objects') && ${compare}.getAttribute('aria-label')`, 15000)
  check('the dialog names the other folder', versus === 'Compare AcmeOutfitters.pbip with acme-outfitters-other', versus ?? 'no dialog')
  const text = await evaluate(`${compare}.textContent`)
  check('the changed measure is different, labelled with the folder', text.includes('Orders[Total Sales]') && text.includes('1 different') && text.includes('In acme-outfitters-other'), text.slice(0, 300))
  const otherLines = await evaluate(`[...${compare}.querySelectorAll('.snippet .line')].map((l) => l.textContent)`)
  check("its TMDL: the other folder's line minus, this line plus", otherLines.includes("-measure 'Total Sales' = SUM(Orders[Line Amount]) * 2") && otherLines.includes("+measure 'Total Sales' = SUM(Orders[Line Amount]) * 1"), JSON.stringify(otherLines.slice(0, 4)))
  const otherButtons = await evaluate(`[...${compare}.querySelectorAll('button')].map((b) => b.textContent.trim()).filter((t) => t)`)
  check('only Close, no Reload or Keep mine', !otherButtons.includes('Reload from disk') && !otherButtons.includes('Keep mine') && otherButtons.includes('Close'), otherButtons.join(', '))
  await evaluate(click('[role="dialog"][aria-label^="Compare "] button', 'Close'))
  check('Close closes it and nothing changed', (await waitFor(`!${compare}`, 3000)) && !(await evaluate(`!!${bar} || ${title}.includes('*')`)))
  rmSync(other, { recursive: true, force: true })
} catch (e) {
  check('no exception', false, e?.stack ?? String(e))
} finally {
  session?.close()
}
finish()
