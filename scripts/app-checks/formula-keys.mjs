// M7.5b check of the desktop app (Windows, WebView2): Desktop's formula editor keys in the formula
// bar (Tab, Shift+Tab and Ctrl+M, Ctrl+] and Ctrl+[, line moves, copies and deletes, Ctrl+L,
// Ctrl+Shift+\, Shift+Alt+→ and ←, Ctrl+X and Ctrl+C on a line, native undo, Shift+Enter and
// Ctrl+Shift+Enter, Ctrl+↓) and accessibility checks A1 to A5 (ADR-0015) of the formula bar with
// its suggestion list, parameter info, description and squiggles open: the IBM Equal Access
// checker, UI Automation names, contrast, the focus ring and reduced motion. Works on
// Orders[Total Sales] of a copy of Acme Outfitters and cancels at the end (nothing is committed).
//
//   (cd app && npx tauri build --debug --no-bundle)
//   npm install --prefix target/tmp/a11y --no-save accessibility-checker-engine
//   node scripts/app-checks/formula-keys.mjs [path/to/dashcraft-app.exe]
import { installChecker, requireChecker } from './a11y.mjs'
import { ALT, CTRL, launch, reporter, selectTotalSales, sleep } from './cdp.mjs'

const SHIFT = 8
requireChecker()
const { check, finish } = reporter()
const bar = `document.querySelector('.formula-bar textarea')`
const listOpen = `!!document.querySelector('.formula-bar [role="listbox"]')`
const statusText = `[...document.querySelectorAll('.formula-bar [role="status"]')].map((s) => s.textContent).join(' ')`
const description = `(${bar}.getAttribute('aria-describedby') ?? '').split(' ').map((id) => document.getElementById(id)?.textContent ?? '').join(' ')`
// The elements under `root` that hold text themselves (what the contrast is measured on).
const textOf = (root) => `[...document.querySelectorAll('${root}, ${root} *')].filter((e) => [...e.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim()))`
const T = 'Total Sales =\nVAR x = SUM(Orders[Line Amount])\nRETURN\nx'
const line2 = T.indexOf('VAR')
const line3 = T.indexOf('RETURN')
const line4 = T.length - 1
let session
try {
  session = await launch(9336)
  const { evaluate, waitFor, key, type, uia } = session
  await installChecker(session)
  const state = () => evaluate(`(() => { const t = ${bar}; return { text: t.value, start: t.selectionStart, end: t.selectionEnd } })()`)
  const show = (s) => JSON.stringify(s)
  // Replaces the text as one edit, closes a suggestion list it opened and places the selection.
  const setText = async (text, start, end = start) => {
    await evaluate(`(() => { const t = ${bar}; t.focus(); t.select(); document.execCommand('insertText', false, ${JSON.stringify(text)}); return true })()`)
    await sleep(300)
    if (await evaluate(listOpen)) await key('Escape', 'Escape', 27)
    await evaluate(`(() => { const t = ${bar}; t.focus(); t.setSelectionRange(${start}, ${end}); t.dispatchEvent(new MouseEvent('click')); return true })()`)
    await sleep(50)
  }

  const formula = await selectTotalSales(session)
  check('the formula bar edits Total Sales', formula && !formula.includes('\n'), formula ?? 'no editor')

  // Shift+Enter: a new line in the formula (Enter commits), the bar grows.
  check('the formula bar starts collapsed', !(await evaluate(`!!document.querySelector('.formula-bar.expanded')`)))
  await setText(formula, formula.length)
  await key('Enter', 'Enter', 13, SHIFT)
  let s = await state()
  check('Shift+Enter adds a line instead of committing', s.text === `${formula}\n` && s.start === s.text.length, show(s))
  check('Shift+Enter expands the formula bar', await waitFor(`!!document.querySelector('.formula-bar.expanded')`, 3000))
  const indented = 'Total Sales =\n    SUM(Orders[Line Amount])'
  await setText(indented, indented.indexOf('['))
  await key('Enter', 'Enter', 13, SHIFT)
  s = await state()
  check('Shift+Enter keeps the indentation', s.text === 'Total Sales =\n    SUM(Orders\n    [Line Amount])' && s.start === indented.indexOf('[') + 5, show(s))
  await setText(T, line4 + 1)
  await key('Enter', 'Enter', 13, CTRL | SHIFT)
  s = await state()
  check('Ctrl+Shift+Enter inserts a line above', s.text === `${T.slice(0, line4)}\nx` && s.start === line4, show(s))

  // Tab indents, Ctrl+M makes it move focus (and back), announced and described.
  await setText(T, line4)
  check('the editor describes what Tab does', /Tab indents; Ctrl\+M makes Tab move focus/.test(await evaluate(description)), await evaluate(description))
  await key('Tab', 'Tab', 9)
  s = await state()
  check('Tab indents to the next tab stop', s.text === `${T.slice(0, line4)}    x` && s.start === line4 + 4, show(s))
  await key('Tab', 'Tab', 9, SHIFT)
  s = await state()
  check('Shift+Tab outdents', s.text === T && s.start === line4, show(s))
  await key('m', 'KeyM', 77, CTRL)
  check('Ctrl+M is announced', await waitFor(`${statusText}.includes('Tab now moves focus')`, 3000), await evaluate(statusText))
  check('the description follows Ctrl+M', /Tab moves focus; Ctrl\+M makes Tab indent/.test(await evaluate(description)), await evaluate(description))
  await key('Tab', 'Tab', 9)
  await sleep(100)
  const away = await evaluate(`({ left: document.activeElement !== ${bar}, to: document.activeElement?.getAttribute('aria-label') ?? document.activeElement?.tagName, text: ${bar}.value })`)
  check('after Ctrl+M, Tab moves focus and leaves the text alone', away.left && away.text === T, show(away))
  await evaluate(`${bar}.focus(), true`)
  await key('m', 'KeyM', 77, CTRL)
  check('Ctrl+M again brings Tab back to indenting', await waitFor(`${statusText}.includes('Tab now inserts indentation')`, 3000))

  // Ctrl+] and Ctrl+[ indent and outdent the selected lines.
  await setText(T, line2 + 2, line3 + 2)
  await key(']', 'BracketRight', 221, CTRL)
  s = await state()
  check('Ctrl+] indents the selected lines', s.text === T.replace('VAR', '    VAR').replace('RETURN', '    RETURN'), show(s))
  await key('[', 'BracketLeft', 219, CTRL)
  check('Ctrl+[ outdents them', (await state()).text === T)

  // Alt+↓ moves a line, Ctrl+Z undoes it natively; Shift+Alt+↓ copies; Ctrl+Shift+K deletes.
  await setText(T, line2 + 4)
  await key('ArrowDown', 'ArrowDown', 40, ALT)
  s = await state()
  check('Alt+↓ moves the line down with the caret', s.text === 'Total Sales =\nRETURN\nVAR x = SUM(Orders[Line Amount])\nx' && s.start === line2 + 7 + 4, show(s))
  await key('ArrowUp', 'ArrowUp', 38, ALT)
  check('Alt+↑ moves it back', (await state()).text === T)
  await key('ArrowDown', 'ArrowDown', 40, ALT)
  await key('z', 'KeyZ', 90, CTRL)
  s = await state()
  check('Ctrl+Z undoes a line move', s.text === T, show(s))
  await setText(T, line2 + 4)
  await key('ArrowDown', 'ArrowDown', 40, ALT | SHIFT)
  s = await state()
  const copied = T.replace('RETURN', 'VAR x = SUM(Orders[Line Amount])\nRETURN')
  check('Shift+Alt+↓ copies the line below', s.text === copied && s.start === line3 + 4, show(s))
  await setText(T, line3 + 2)
  await key('K', 'KeyK', 75, CTRL | SHIFT)
  s = await state()
  check('Ctrl+Shift+K deletes the line', s.text === T.replace('RETURN\n', '') && s.start === line3 + 1, show(s))

  // Ctrl+L, Ctrl+Shift+\, Shift+Alt+→ and ←.
  await setText(T, line2 + 4)
  await key('l', 'KeyL', 76, CTRL)
  s = await state()
  check('Ctrl+L selects the line', s.start === line2 && s.end === line3, show(s))
  const open = T.indexOf('(')
  await setText(T, open + 1)
  await key('|', 'Backslash', 220, CTRL | SHIFT)
  s = await state()
  check('Ctrl+Shift+\\ jumps to the matching bracket', s.start === T.indexOf(')') && s.end === s.start, show(s))
  await key('|', 'Backslash', 220, CTRL | SHIFT)
  check('and back', (await state()).start === open)
  const orders = T.indexOf('Orders')
  await setText(T, orders + 2)
  await key('ArrowRight', 'ArrowRight', 39, ALT | SHIFT)
  s = await state()
  const first = T.slice(s.start, s.end)
  await key('ArrowRight', 'ArrowRight', 39, ALT | SHIFT)
  s = await state()
  const second = T.slice(s.start, s.end)
  check('Shift+Alt+→ widens the selection', first === 'Orders' && second === 'Orders[Line Amount]', `${first} → ${second}`)
  await key('ArrowLeft', 'ArrowLeft', 37, ALT | SHIFT)
  s = await state()
  check('Shift+Alt+← narrows it again', T.slice(s.start, s.end) === 'Orders', show(s))

  // Ctrl+X and Ctrl+C without a selection take the whole line (read in the page's clipboard events).
  await evaluate(`(() => { window.__clip = []; for (const type of ['copy', 'cut']) document.addEventListener(type, (e) => window.__clip.push([type, e.clipboardData.getData('text/plain')])); return true })()`)
  await setText(T, line3 + 2)
  await key('x', 'KeyX', 88, CTRL)
  const cut = await waitFor(`window.__clip.length && window.__clip[0]`, 3000)
  await sleep(100)
  s = await state()
  check('Ctrl+X without a selection cuts the line', cut?.[0] === 'cut' && cut[1] === 'RETURN\n' && s.text === T.replace('RETURN\n', '') && s.start === line3 + 1, `${show(cut)} ${show(s)}`)
  await setText(T, line2 + 4)
  await key('c', 'KeyC', 67, CTRL)
  const copy = await waitFor(`window.__clip.length > 1 && window.__clip[1]`, 3000)
  s = await state()
  check('Ctrl+C without a selection copies the line and keeps the caret', copy?.[0] === 'copy' && copy[1] === 'VAR x = SUM(Orders[Line Amount])\n' && s.text === T && s.start === line2 + 4 && s.end === s.start, `${show(copy)} ${show(s)}`)

  // Ctrl+↓ scrolls a line without moving the caret.
  const long = Array.from({ length: 40 }, (_, i) => `// line ${i + 1}`).join('\n')
  await setText(`Total Sales =\n${long}\nSUM(Orders[Line Amount])`, 0)
  const before = await evaluate(`${bar}.scrollTop`)
  await key('ArrowDown', 'ArrowDown', 40, CTRL)
  await sleep(50)
  s = await state()
  const after = await evaluate(`${bar}.scrollTop`)
  check('Ctrl+↓ scrolls without moving the caret', after > before && s.start === 0, `scrollTop ${before} → ${after}, caret ${s.start}`)

  // A1 to A5 with the suggestion list, parameter info, description and squiggles open.
  const scope = '.formula-bar'
  const checker = async (surface) => {
    const issues = await evaluate(`__a11y.check(${JSON.stringify(scope)})`)
    const violations = issues.filter((i) => i.level === 'VIOLATION/FAIL')
    check(`A3 checker (WCAG 2.2 A and AA), ${surface}: no violation`, violations.length === 0, violations.map((i) => `${i.rule} on ${i.where}: ${i.msg}`).join('; '))
    const review = new Set(issues.filter((i) => i.level === 'VIOLATION/POTENTIAL').map((i) => i.rule))
    if (review.size) console.log(`  to review by hand: ${[...review].join(', ')}`)
  }
  const text = 'Total Sales = SUM(Orders[Nope]) + COUNTR'
  await setText(text.slice(0, -1), text.length - 1)
  await type('R')
  check('typing opens the suggestion list', await waitFor(listOpen, 10000))
  await waitFor(`!!document.querySelector('.formula-bar .highlight .error')`, 10000)
  check('A1 focus stays in the editor while the list is open', await evaluate(`document.activeElement === ${bar} && !!${bar}.getAttribute('aria-activedescendant')`))
  await checker('suggestion list and squiggle')
  const ring = await evaluate(`__a11y.ring(document.querySelector('.formula-bar .editor'))`)
  check('A2 the editor has a focus ring of at least 2px and 3:1', ring.ok && ring.ratio >= 3, show(ring))
  const listText = await evaluate(`[...document.querySelectorAll('.formula-bar [role="option"] .label, .formula-bar [role="option"] .detail, .formula-bar .suggest .doc *')].filter((e) => e.textContent.trim()).map((e) => [e.className, __a11y.ratio(getComputedStyle(e).color, __a11y.bg(e))])`)
  check('A4 suggestion text at least 4.5:1, the selected one included', listText.length > 0 && listText.every(([, r]) => r >= 4.5), `lowest ${Math.min(...listText.map(([, r]) => r))}:1 of ${listText.length}`)
  const marks = await evaluate(`(() => {
    const active = document.querySelector('.formula-bar [role="option"].active')
    const pane = __a11y.bg(active.parentElement)
    const error = document.querySelector('.formula-bar .highlight .error')
    const warning = error.cloneNode(true)
    warning.classList.replace('error', 'warning')
    error.parentElement.appendChild(warning)
    const code = __a11y.bg(document.querySelector('.formula-bar .editor'))
    const out = { selectedOutline: __a11y.ratio(getComputedStyle(active).outlineColor, pane), errorSquiggle: __a11y.ratio(getComputedStyle(error).textDecorationColor, code), warningSquiggle: __a11y.ratio(getComputedStyle(warning).textDecorationColor, code) }
    warning.remove()
    return out
  })()`)
  check('A4 the selected suggestion and the squiggles at least 3:1', Object.values(marks).every((r) => r >= 3), show(marks))
  uia(['Suggestions'])
  await sleep(1500)
  const tree = uia(['Measure formula', 'Suggestions']).trimEnd().split(/\r?\n/)
  for (const line of tree) console.log(`  uia ${line.replace(/\t/g, ' | ')}`)
  check('A3 UI Automation exposes the editor and the suggestion list', tree.some((l) => l.includes('Measure formula\tControlType.Edit')) && tree.some((l) => l.includes('Suggestions\tControlType.List')))

  await key('Tab', 'Tab', 9)
  check('the suggestion is accepted and parameter info opens', await waitFor(`!!document.querySelector('.formula-bar .signature')`, 5000))
  await checker('parameter info')
  const sigText = await evaluate(`${textOf('.formula-bar .signature')}.map((e) => __a11y.ratio(getComputedStyle(e).color, __a11y.bg(e)))`)
  check('A4 parameter info text at least 4.5:1', sigText.length > 0 && sigText.every((r) => r >= 4.5), `lowest ${Math.min(...sigText)}:1`)
  await type('Orders)')
  await sleep(300)
  if (await evaluate(listOpen)) await key('Escape', 'Escape', 27)
  await waitFor(`[...document.querySelectorAll('.formula-bar .highlight .error')].some((e) => e.textContent.includes('Nope'))`, 10000)
  await evaluate(`(() => { const t = ${bar}; const at = t.value.indexOf('Nope') + 1; t.setSelectionRange(at, at); t.dispatchEvent(new MouseEvent('click')); return true })()`)
  await key('k', 'KeyK', 75, CTRL)
  await key('i', 'KeyI', 73, CTRL)
  check('Ctrl+K Ctrl+I describes the problem at the caret', await waitFor(`document.querySelector('.formula-bar .tip')?.textContent.includes('unknown')`, 5000), `${await evaluate(`document.querySelector('.formula-bar .tip')?.textContent`)} / ${await evaluate(statusText)} / ${await evaluate(`${bar}.value`)}`)
  await checker('description')
  const tipText = await evaluate(`${textOf('.formula-bar .tip')}.map((e) => __a11y.ratio(getComputedStyle(e).color, __a11y.bg(e)))`)
  check('A4 description text at least 4.5:1', tipText.length > 0 && tipText.every((r) => r >= 4.5), `lowest ${Math.min(...tipText)}:1`)
  const moving = await evaluate(`[...document.querySelectorAll('.formula-bar, .formula-bar *')].filter((e) => { const s = getComputedStyle(e); return s.animationName !== 'none' || s.transitionDuration.split(',').some((d) => parseFloat(d) > 0) }).map((e) => e.className || e.tagName)`)
  check('A5 no animation or transition in the formula bar or its pop-ups', moving.length === 0, moving.join(', '))
  await key('Escape', 'Escape', 27)

  await evaluate(`${bar}.focus(), true`)
  await key('Escape', 'Escape', 27)
  check('Esc cancels the edit', await waitFor(`${bar}.value === ${JSON.stringify(formula)}`, 5000))
} catch (e) {
  check('no error', false, e.message)
} finally {
  session?.close()
}
finish()
