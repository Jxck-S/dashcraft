// M9.5 Bookmarks check of the desktop app (Windows, WebView2): after a refresh, on the Acme Stores report, the
// West region bookmark button sets the region slicer and the cards show the bookmark's values
// (expected.json `bookmarks`); View → Bookmarks lists the bookmarks, a click applies one; View
// starts the slideshow, whose bar steps through them by keyboard and closes with Esc.
// Accessibility A1, A2 and A5 always; A3 (the IBM checker) and A4 when the checker is installed.
//
//   (cd app && npx tauri build --debug --no-bundle)
//   npm install --prefix target/tmp/a11y --no-save accessibility-checker-engine   # optional
//   node scripts/app-checks/bookmarks.mjs [path/to/dashcraft-app.exe]
import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { aceJs, installChecker } from './a11y.mjs'
import { launch, reporter, root, sleep, title } from './cdp.mjs'

const { check, finish } = reporter()
const PORT = 9339
const expected = JSON.parse(readFileSync(join(root, 'reference', 'examples', 'acme-outfitters', 'expected.json'), 'utf8')).bookmarks
const values = (name) => expected.find((b) => b.bookmark === name).cards.map((c) => c.formatted)
const svgText = `(document.querySelector('.svg-layer')?.textContent ?? '')`
const message = `(document.querySelector('.status-bar .message')?.textContent ?? '')`
const pane = `document.querySelector('.bookmarks')`
const bar = `document.querySelector('.slideshow')`
const focusedName = `(() => { const e = document.activeElement; return (e?.getAttribute('aria-label') || e?.textContent || '').trim() })()`
/** Every expected card value of bookmark `name` is on the rendered page. */
const showsValues = (name) => `${JSON.stringify(values(name))}.every((v) => ${svgText}.includes(v))`
/** The expected card values of bookmark `name` missing from the rendered page. */
const missing = (name) => `${JSON.stringify(values(name))}.filter((v) => !${svgText}.includes(v)).join(', ') + ' missing in: ' + ${svgText}.slice(0, 600)`

let session
try {
  session = await launch(PORT)
  const { evaluate, waitFor, key, send } = session
  /** Enter with its text, so that a focused button activates natively. */
  const enter = async () => {
    await send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, text: '\r' })
    await send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13 })
  }
  const shows = async (label, name) => {
    const ok = await waitFor(showsValues(name), 15000)
    check(label, ok, ok ? values(name).join(', ') : await evaluate(missing(name)))
  }
  const opened = await waitFor(`${title}.includes('AcmeOutfitters') && ${title}`, 60000)
  check('the document opens', opened, opened ?? 'no title')
  const refresh = `document.querySelector('button[title^="Refresh all tables"]')`
  await waitFor(`${refresh} && !${refresh}.disabled && (${refresh}.click(), true)`, 10000)
  const refreshed = await waitFor(`${message}.startsWith('Refreshed') && ${message}`, 120000)
  check('Home → Refresh loads the data (blob map)', refreshed, (await evaluate(message)) || 'no message')

  const picked = await waitFor(`(() => { const s = document.querySelector('.report-pick select'); const o = s && [...s.options].find((o) => o.textContent.includes('Stores')); if (!o) return false; s.value = o.value; s.dispatchEvent(new Event('change', { bubbles: true })); return o.textContent })()`)
  check('the Stores report opens', picked, picked ?? 'no report picker')
  const page = expected[0].page
  await waitFor(`(() => { const t = [...document.querySelectorAll('.page-tab')].find((t) => t.textContent.trim() === ${JSON.stringify(page)}); t?.click(); return t?.getAttribute('aria-selected') === 'true' })()`)
  const button = `[...document.querySelectorAll('.svg-layer [data-hit$="|button"]')].find((g) => g.textContent.includes('West region'))`
  check(`${page} shows the West region bookmark button`, await waitFor(`!!${button}`, 60000))

  // A bookmark button on the canvas.
  await evaluate(`${button}.dispatchEvent(new MouseEvent('click', { bubbles: true })), true`)
  const said = await waitFor(`${message}.startsWith('Bookmark West region applied') && ${message}`, 10000)
  check('A1 the bookmark button says the bookmark is applied', said, (await evaluate(message)) || 'no message')
  await shows('the cards show the West region values', 'West region')

  // The Bookmarks pane.
  await evaluate(`[...document.querySelectorAll('[role="tab"]')].find((t) => t.textContent.trim() === 'View')?.click(), true`)
  const toggle = `document.querySelector('button[title="Show or hide the Bookmarks pane"]')`
  check('View → Bookmarks is available', await waitFor(`${toggle} && !${toggle}.disabled`, 5000))
  await evaluate(`${toggle}.click(), true`)
  const rows = await waitFor(`(() => { const r = [...(${pane}?.querySelectorAll('ul .row') ?? [])].map((b) => b.textContent.trim()); return r.length > 0 && r.join(', ') })()`, 10000)
  check('the pane lists the bookmarks in order', rows === expected.map((b) => b.bookmark).join(', '), rows || 'no rows')
  const current = await evaluate(`${pane}.querySelector('[aria-current="true"]')?.textContent.trim() ?? ''`)
  check('A1 the bookmark applied last is marked current', current === 'West region', current || 'none')
  await evaluate(`[...${pane}.querySelectorAll('.row')].find((b) => b.textContent.trim() === 'All regions').click(), true`)
  check('a click in the pane applies the bookmark', await waitFor(`${message}.startsWith('Bookmark All regions applied')`, 10000), await evaluate(message))
  await shows('the cards show the All regions values', 'All regions')

  // The slideshow, by keyboard.
  await evaluate(`${pane}.querySelector('button.view').focus(), true`)
  await enter()
  check('View opens the slideshow bar', await waitFor(`!!${bar}`, 5000))
  check('the slideshow starts at the first bookmark', await waitFor(`${bar}.textContent.includes('West region') && ${bar}.textContent.includes('1 of 2')`, 5000), await evaluate(`${bar}?.textContent ?? ''`))
  await shows('the cards show the first bookmark', 'West region')
  const first = await waitFor(`${bar}.contains(document.activeElement) && ${focusedName}`, 5000)
  check('A2 the focus moves to Next in the bar', first === 'Next bookmark', first ?? (await evaluate(focusedName)))
  await enter()
  check('Next shows the second bookmark', await waitFor(`${bar}.textContent.includes('All regions') && ${bar}.textContent.includes('2 of 2')`, 5000))
  await shows('the cards show the second bookmark', 'All regions')
  const after = await waitFor(`${bar}.contains(document.activeElement) && !document.activeElement.disabled && ${focusedName}`, 5000)
  check('A2 at the last bookmark the focus stays in the bar', after, after ?? (await evaluate(focusedName)))
  const order = []
  await evaluate(`${bar}.querySelector('button:not(:disabled)').focus(), true`)
  for (let i = 0; i < 2; i++) {
    order.push(await evaluate(focusedName))
    await key('Tab', 'Tab', 9)
  }
  check('A1 Tab goes Previous, Exit (Next disabled at the end)', order.join(', ') === 'Previous bookmark, Exit the bookmark slideshow', order.join(', '))
  const group = await evaluate(`${bar}.getAttribute('role') + ' ' + ${bar}.getAttribute('aria-label')`)
  check('A1 the bar is a named group', group === 'group Bookmark slideshow', group)
  if (existsSync(aceJs)) {
    await installChecker(session)
    for (const scope of ['.bookmarks', '.slideshow']) {
      const issues = await evaluate(`__a11y.check(${JSON.stringify(scope)})`)
      const violations = issues.filter((i) => i.level === 'VIOLATION/FAIL')
      check(`A3 checker (WCAG 2.2 A and AA), ${scope}: no violation`, violations.length === 0, violations.map((i) => `${i.rule} on ${i.where}: ${i.msg}`).join('; '))
    }
    const text = await evaluate(`[...document.querySelectorAll('.bookmarks .row .name, .bookmarks .view span, .slideshow span')].filter((e) => e.textContent.trim()).map((e) => [e.textContent.trim().slice(0, 20), __a11y.ratio(getComputedStyle(e).color, __a11y.bg(e))])`)
    const low = text.filter(([, r]) => r < 4.5)
    check('A4 text at least 4.5:1', low.length === 0, low.map(([t, r]) => `${t} ${r}:1`).join('; ') || `${text.length} elements`)
    const ring = await evaluate(`(document.querySelector('.slideshow .exit').focus(), __a11y.focused())`)
    check('A2 the focused bar button has a visible ring (2 px, 3:1)', ring?.ok && ring.ratio >= 3, JSON.stringify(ring))
  } else console.log(`SKIP: ${aceJs} missing (A3 checker, A4 contrast)`)
  const moving = await evaluate(`[...document.querySelectorAll('.bookmarks *, .slideshow *')].filter((e) => { const s = getComputedStyle(e); return s.animationName !== 'none' || s.transitionDuration.split(',').some((d) => parseFloat(d) > 0) }).map((e) => e.className || e.tagName)`)
  check('A5 no animation or transition', moving.length === 0, moving.join(', '))
  await evaluate(`document.querySelector('.slideshow .exit').focus(), true`)
  await key('Escape', 'Escape', 27)
  check('Esc closes the slideshow', await waitFor(`!${bar}`, 5000))
  await sleep(100)
  const back = await evaluate(`document.activeElement?.closest('.bookmarks') ? ${focusedName} : ${focusedName} + ' (outside the pane)'`)
  check('A2 the focus goes back to View in the pane', back === 'View', back)
  check('A1 the status bar says the slideshow closed', (await evaluate(message)) === 'Bookmark slideshow closed', await evaluate(message))
} catch (e) {
  check('no exception', false, e?.stack ?? String(e))
} finally {
  session?.close()
}
finish()
