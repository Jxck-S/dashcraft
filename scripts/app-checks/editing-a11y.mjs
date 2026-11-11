// Accessibility checks A1 to A5 (docs/DESKTOP-PLAN.md, W5; ADR-0015) of the M6.8 editing surfaces
// on Windows and WebView2: keyboard-only editing, focus order and focus rings, names and roles in
// the DOM and in UI Automation (what Narrator and NVDA read), the IBM Equal Access checker
// (Apache-2.0, WCAG 2.2 A and AA rules), text and component contrast, and reduced motion. A6, the
// screen-reader run, is screen-reader.mjs (CI, .github/workflows/screen-reader.yml).
//
//   (cd app && npx tauri build --debug --no-bundle)
//   npm install --prefix target/tmp/a11y --no-save accessibility-checker-engine
//   node scripts/app-checks/editing-a11y.mjs [path/to/dashcraft-app.exe]
import { installChecker, requireChecker } from './a11y.mjs'
import { launch, openProperties, reporter, selectTotalSales, sleep, title } from './cdp.mjs'

requireChecker()
const field = '.field:has(> [id^="prop-"])'
const scope = `.title-bar .qat, .formula-bar, ${field}, [role="dialog"][aria-label="Unsaved changes"], .backstage`

const { check, finish } = reporter()
let session
try {
  session = await launch(9334)
  const { evaluate, waitFor, key, type, uia } = session
  const checker = async (surface) => {
    const issues = await evaluate(`__a11y.check(${JSON.stringify(scope)})`)
    const violations = issues.filter((i) => i.level === 'VIOLATION/FAIL')
    check(`A3 checker (WCAG 2.2 A and AA), ${surface}: no violation`, violations.length === 0, violations.map((i) => `${i.rule} on ${i.where}: ${i.msg}`).join('; '))
    const review = new Set(issues.filter((i) => i.level === 'VIOLATION/POTENTIAL').map((i) => i.rule))
    if (review.size) console.log(`  to review by hand: ${[...review].join(', ')}`)
  }
  const tree = (names) => {
    const lines = uia(names).trimEnd().split(/\r?\n/)
    for (const line of lines) console.log(`  uia ${line.replace(/\t/g, ' | ')}`)
    return lines
  }

  await waitFor(`${title}.includes('AcmeOutfitters')`, 60000)
  await installChecker(session)
  await selectTotalSales(session)
  await openProperties(session)

  // A1: edit and commit with the keyboard only.
  await evaluate(`document.querySelector('.formula-bar textarea').focus(), true`)
  await key('End', 'End', 35, 2)
  await type(' + 0')
  await key('Enter', 'Enter', 13)
  check('A1 a formula edit is typed and committed with the keyboard', await waitFor(`${title}.includes('*')`))

  // A2: Tab order and focus rings.
  await evaluate(`document.activeElement?.blur(), document.querySelector('.qat button').focus(), true`)
  const qatOrder = [await evaluate('__a11y.focused()')]
  for (let i = 0; i < 2; i++) {
    await key('Tab', 'Tab', 9)
    qatOrder.push(await evaluate('__a11y.focused()'))
  }
  check('A2 Tab goes Save, Undo and on (Redo disabled, skipped)', qatOrder[0]?.name === 'Save' && qatOrder[1]?.name === 'Undo' && qatOrder[2]?.name !== 'Redo', qatOrder.map((f) => f?.name).join(', '))
  await evaluate(`document.querySelector('#prop-description').focus(), true`)
  const fieldOrder = [await evaluate('__a11y.focused()')]
  for (let i = 0; i < 2; i++) {
    await key('Tab', 'Tab', 9)
    fieldOrder.push(await evaluate('__a11y.focused()'))
  }
  check('A2 Tab goes Description, Display folder, Format string', fieldOrder.map((f) => f?.name).join(', ') === 'Description, Display folder, Format string', fieldOrder.map((f) => f?.name).join(', '))
  await evaluate(`document.querySelector('.formula-bar textarea').focus(), true`)
  const editor = { name: 'Measure formula', ...(await evaluate(`__a11y.ring(document.querySelector('.formula-bar .editor'))`)) }
  const rings = [...qatOrder.slice(0, 2), ...fieldOrder, editor]
  for (const r of rings) console.log(`  focus ring ${r?.name}: ${r?.outline}, ${r?.ratio}:1`)
  check('A2 focus rings at least 2 px and 3:1', rings.every((r) => r?.ok && r.ratio >= 3))

  // A1: Esc cancels a draft without trapping focus.
  await evaluate(`document.querySelector('#prop-format-string').focus(), true`)
  await type('x')
  await key('Escape', 'Escape', 27)
  check('A1 Esc cancels a Properties draft and keeps focus', await evaluate(`document.activeElement?.id === 'prop-format-string' && !document.activeElement.value.endsWith('x')`))
  await evaluate(`document.activeElement?.blur(), true`)

  // A3: names and roles in the DOM, the checker and UI Automation.
  const names = await evaluate(`[...document.querySelectorAll('.qat button, .formula-bar button, .formula-bar textarea, ${field} input, ${field} textarea')].map((e) => e.tagName.toLowerCase() + ':' + (__a11y.name(e) || '?'))`)
  check('A3 every new control has an accessible name', names.every((n) => !n.endsWith(':?')), names.join(', '))
  await checker('QAT, formula bar and Properties fields')
  uia(['Save'])
  await sleep(1500)
  const uiaControls = tree(['Quick access toolbar', 'Save', 'Undo', 'Redo', 'Measure formula', 'Commit', 'Description', 'Display folder', 'Format string'])
  check('A3 UI Automation exposes the controls with roles and values', !uiaControls.some((l) => l.startsWith('MISSING')) && uiaControls.some((l) => l.includes('Measure formula\tControlType.Edit')) && uiaControls.some((l) => /Redo\tControlType.Button\tenabled=False/.test(l)))

  // A4: text and component contrast (one shell theme).
  const text = await evaluate(`[...document.querySelectorAll('.qat button:not(:disabled), .title-bar .title, .formula-bar .highlight .layer > span:not(.probe), .formula-bar .gutter span, ${field} label, ${field} input, ${field} textarea')].map((e) => [__a11y.name(e) || e.className, __a11y.ratio(getComputedStyle(e).color, __a11y.bg(e))])`)
  check('A4 text at least 4.5:1', text.every(([, r]) => r >= 4.5), `lowest ${Math.min(...text.map(([, r]) => r))}:1`)
  const parts = await evaluate(`[...document.querySelectorAll('.qat button:not(:disabled) svg, .formula-bar .icon-btn:not(:disabled) svg')].map((s) => __a11y.ratio(getComputedStyle(s).color, __a11y.bg(s))).concat([...document.querySelectorAll('${field} input, ${field} textarea, .formula-bar .editor')].map((e) => __a11y.ratio(getComputedStyle(e).borderBottomColor, __a11y.bg(e.parentElement))))`)
  check('A4 icons and input borders at least 3:1', parts.every((r) => r >= 3), `lowest ${Math.min(...parts)}:1`)

  // The File menu and the unsaved-changes question, opened from the File button as a keyboard
  // user does (focus it, press Enter).
  const fileButton = `[...document.querySelectorAll('button')].find((b) => b.textContent.trim() === 'File')`
  const onFile = `document.activeElement === ${fileButton}`
  const openFile = () => evaluate(`(${fileButton}.focus(), ${fileButton}.click(), true)`).then(() => waitFor(`!!document.querySelector('.backstage')`, 3000))
  await openFile()
  await checker('File menu')
  const inMenu = `document.querySelector('.backstage')?.contains(document.activeElement) ?? false`
  check('A2 the File menu takes focus', await evaluate(inMenu))
  const stays = []
  for (let i = 0; i < 50; i++) {
    await key('Tab', 'Tab', 9, i < 40 ? 0 : 8)
    stays.push(await evaluate(inMenu))
  }
  check('A2 Tab and Shift+Tab stay in the File menu', stays.every(Boolean), `${stays.filter((s) => !s).length} of 50 presses left it`)
  await key('Escape', 'Escape', 27)
  await sleep(300)
  check('A1 Esc closes the File menu and focus goes back to File', !(await evaluate(`!!document.querySelector('.backstage')`)) && (await evaluate(onFile)))
  await openFile()
  await evaluate(`[...document.querySelectorAll('.backstage button')].find((b) => b.textContent.trim() === 'Close').focus(), true`)
  await evaluate(`document.activeElement.click(), true`)
  const dialog = `document.querySelector('[role="dialog"][aria-label="Unsaved changes"]')`
  await waitFor(`!!${dialog}`, 5000)
  await sleep(300)
  check('A2 focus moves into the question', await evaluate(`${dialog}.contains(document.activeElement)`))
  await checker('unsaved-changes question')
  const uiaQuestion = tree(['Unsaved changes', "Don't save"])
  check('A3 UI Automation exposes the question as a window with its buttons', uiaQuestion.some((l) => l.includes('Unsaved changes\tControlType.Window')) && uiaQuestion.some((l) => l.includes("Don't save\tControlType.Button")))
  const dialogText = await evaluate(`[...${dialog}.querySelectorAll('button, p')].map((e) => __a11y.ratio(getComputedStyle(e).color, __a11y.bg(e)))`)
  check('A4 question text and buttons at least 4.5:1', dialogText.every((r) => r >= 4.5), `lowest ${Math.min(...dialogText)}:1`)
  await key('Escape', 'Escape', 27)
  await sleep(400)
  check('A1 Esc answers Cancel', !(await evaluate(`!!${dialog}`)) && (await evaluate(title)).includes('*'))
  check('A2 focus goes back to File after the question (the File menu closed with it)', await evaluate(onFile))

  // A5: nothing on the new controls moves.
  const moving = await evaluate(`[...document.querySelectorAll('.qat, .qat *, .formula-bar, .formula-bar *, ${field}, ${field} *')].filter((e) => { const s = getComputedStyle(e); return s.animationName !== 'none' || s.transitionDuration.split(',').some((d) => parseFloat(d) > 0) }).map((e) => e.className || e.tagName)`)
  check('A5 no animation or transition on the new controls', moving.length === 0, moving.join(', '))
} catch (e) {
  check('no error', false, e.message)
} finally {
  session?.close()
}
finish()
