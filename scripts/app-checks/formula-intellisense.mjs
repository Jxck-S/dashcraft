// M7.5 formula bar IntelliSense check of the desktop app (Windows, WebView2): token colours,
// the suggestion list (typing, Ctrl+Space, arrows, Tab/Enter, Esc), parameter info after `(`,
// squiggles with their description for a screen reader, Ctrl+K Ctrl+I descriptions and
// Shift+Alt+F formatting, all on Orders[Total Sales] of a copy of Acme Outfitters, cancelled at
// the end (nothing is committed).
//
//   (cd app && npx tauri build --debug --no-bundle)
//   node scripts/app-checks/formula-intellisense.mjs [path/to/dashcraft-app.exe]
import { ALT, CTRL, launch, reporter, selectTotalSales, sleep } from './cdp.mjs'

const SHIFT = 8
const { check, finish } = reporter()
const bar = `document.querySelector('.formula-bar textarea')`
const options = `[...document.querySelectorAll('.formula-bar [role="option"]')].map((o) => o.querySelector('.label').textContent)`
const listOpen = `!!document.querySelector('.formula-bar [role="listbox"]')`
let session
try {
  session = await launch(9335)
  const { evaluate, waitFor, key, type } = session
  const typeEach = async (text) => {
    for (const ch of text) {
      await type(ch)
      await sleep(30)
    }
  }
  const caretAt = (offset) => evaluate(`(() => { const t = ${bar}; t.focus(); const at = ${offset}; t.setSelectionRange(at, at); t.dispatchEvent(new MouseEvent('click')); return true })()`)

  const formula = await selectTotalSales(session)
  check('the formula bar edits Total Sales', formula, formula ?? 'no editor')

  const colours = await waitFor(`(() => { const s = [...document.querySelectorAll('.formula-bar .highlight span')].find((x) => x.textContent === 'SUM'); return s ? getComputedStyle(s).color : null })()`, 10000)
  check('function names are coloured', colours === 'rgb(122, 75, 0)', colours ?? 'no SUM span')
  check('the overlay shows the textarea text', await evaluate(`document.querySelector('.formula-bar .highlight .layer').textContent.startsWith(${bar}.value)`))

  await caretAt(`${bar}.value.length`)
  await typeEach(' + COUNTR')
  const typed = await waitFor(`${listOpen} && ${options}`, 10000)
  check('typing a name lists suggestions', typed?.includes('COUNTROWS'), typed?.slice(0, 5).join(', '))
  check('the editor points at the active suggestion', await evaluate(`(() => { const t = ${bar}; const id = t.getAttribute('aria-activedescendant'); return !!id && document.getElementById(id)?.getAttribute('aria-selected') === 'true' && t.getAttribute('aria-controls') === document.querySelector('.formula-bar [role="listbox"]').id })()`))
  const status = await evaluate(`[...document.querySelectorAll('.formula-bar [role="status"]')].map((s) => s.textContent).join(' ')`)
  check('a screen reader hears how many suggestions there are', /suggestions/.test(status), status)

  const index = await evaluate(`${options}.indexOf('COUNTROWS')`)
  for (let i = 0; i < index; i++) await key('ArrowDown', 'ArrowDown', 40)
  await key('Tab', 'Tab', 9)
  const accepted = await waitFor(`${bar}.value.endsWith('+ COUNTROWS(') && ${bar}.value`, 5000)
  check('Tab inserts the suggestion with its (', accepted, accepted ?? (await evaluate(`${bar}.value`)))
  check('accepting closes the list', !(await evaluate(listOpen)))
  const signature = await waitFor(`document.querySelector('.formula-bar .signature')?.textContent`, 5000)
  check('parameter info follows the (', signature?.startsWith('COUNTROWS('), signature)
  check('the parameter info describes the editor', await evaluate(`${bar}.getAttribute('aria-describedby')?.split(' ').includes(document.querySelector('.formula-bar .signature').id)`))

  await typeEach('Ord')
  const tables = await waitFor(`${listOpen} && ${options}`, 10000)
  check('tables are suggested', tables?.includes('Orders'), tables?.slice(0, 5).join(', '))
  await key('Escape', 'Escape', 27)
  check('Esc closes the list and keeps the text', !(await evaluate(listOpen)) && (await evaluate(`${bar}.value.endsWith('COUNTROWS(Ord')`)))
  await typeEach('ers) + ')
  await key(' ', 'Space', 32, CTRL)
  const explicit = await waitFor(`${listOpen} && ${options}.length`, 5000)
  check('Ctrl+Space lists suggestions where nothing is typed yet', explicit > 0, `${explicit} items`)
  await key('Escape', 'Escape', 27)

  await typeEach('[Nope]')
  const squiggle = await waitFor(`(() => { const s = [...document.querySelectorAll('.formula-bar .highlight .error')].map((s) => s.textContent).join(''); return s.includes('Nope') && s })()`, 10000)
  check('an unknown measure is squiggled', squiggle === '[Nope]', `${JSON.stringify(squiggle)} in ${JSON.stringify(await evaluate(`${bar}.value`))}`)
  const described = await waitFor(`(() => { const t = ${bar}; const ids = (t.getAttribute('aria-describedby') ?? '').split(' '); const text = ids.map((id) => document.getElementById(id)?.textContent ?? '').join(' '); return t.getAttribute('aria-invalid') === 'true' && text.includes('Error at line 1, column 62: unknown measure or column') && text })()`, 5000)
  check('the problem is part of the accessible description', described, described || 'not described')

  await caretAt(`${bar}.value.indexOf('SUM(') + 1`)
  await key('k', 'KeyK', 75, CTRL)
  await key('i', 'KeyI', 73, CTRL)
  const hover = await waitFor(`document.querySelector('.formula-bar .tip')?.textContent`, 5000)
  check('Ctrl+K Ctrl+I describes the function at the caret', /SUM/.test(hover ?? ''), hover?.slice(0, 80))
  await key('Escape', 'Escape', 27)
  check('Esc closes the description', await waitFor(`!document.querySelector('.formula-bar .tip')`, 3000))

  await evaluate(`(() => { const t = ${bar}; t.focus(); t.select(); return true })()`)
  await type('Total Sales = var x = sum(Orders[Line Amount]) return x')
  await sleep(200)
  await key('F', 'KeyF', 70, ALT | SHIFT)
  const formatted = await waitFor(`${bar}.value.includes('\\n') && ${bar}.value`, 10000)
  check('Shift+Alt+F formats the expression', formatted === 'Total Sales =\nVAR x = SUM(Orders[Line Amount])\nRETURN\n    x', JSON.stringify(formatted))
  check('formatting expands the formula bar', await evaluate(`!!document.querySelector('.formula-bar.expanded')`))

  await evaluate(`${bar}.focus(), true`)
  await key('Escape', 'Escape', 27)
  check('Esc cancels the edit', await waitFor(`${bar}.value === ${JSON.stringify(formula)}`, 5000))
} catch (e) {
  check('no error', false, e.message)
} finally {
  session?.close()
}
finish()
