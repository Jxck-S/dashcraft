// A6: drives the debug app with a real screen reader through Guidepup (MIT) and checks what it
// says for the M6 editing and M7 formula-bar journeys: NVDA on Windows, VoiceOver on macOS,
// Orca on Linux. Narrator has no automation API; its evidence stays the UI Automation tree
// (`uia.ps1`, `editing-a11y.mjs`). Meant for the `screen-reader` CI workflow, which installs
// Guidepup and configures the runner (`npx @guidepup/setup`); it takes over the keyboard and the
// speech of the machine it runs on, so don't run it on your desktop while you work.
//
//   node scripts/app-checks/screen-reader.mjs [path/to/dashcraft-app]
//
// GUIDEPUP_DIR is the folder whose node_modules has @guidepup/guidepup (default
// target/tmp/screen-reader/tools). Works on a copy of Acme Outfitters under
// target/tmp/screen-reader/run and writes report.json and report.md there (and to the job
// summary on GitHub), with a screenshot per failed step on macOS. Exits 1 when a required step
// fails.
import { execFile, execFileSync, spawn } from 'node:child_process'
import { appendFileSync, cpSync, existsSync, mkdirSync, openSync, rmSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { join } from 'node:path'
import { root, sleep } from './cdp.mjs'

const platform = process.platform
const mac = platform === 'darwin'
const win = platform === 'win32'
const exe = process.argv[2] ?? join(root, 'target', 'debug', win ? 'dashcraft-app.exe' : 'dashcraft-app')
const tools = process.env.GUIDEPUP_DIR ?? join(root, 'target', 'tmp', 'screen-reader', 'tools')
const out = join(root, 'target', 'tmp', 'screen-reader', 'run')

const READERS = {
  win32: { name: 'NVDA', title: (sr) => sr.keyboardCommands.reportTitle, focus: (sr) => sr.keyboardCommands.reportCurrentFocus },
  // VoiceOver commands through AppleScript, not VO-key chords: after the synthetic VO-F4 chord
  // VoiceOver went silent in CI.
  darwin: { name: 'VoiceOver', title: (sr) => sr.commanderCommands.DESCRIBE_WINDOW, focus: (sr) => sr.commanderCommands.DESCRIBE_ITEM_WITH_KEYBOARD_FOCUS },
  // Orca's basic "where am I" leaves out the description; the detailed one reads it.
  linux: { name: 'Orca', title: (sr) => sr.keyboardCommands.WhereAmIPresenter.PresentTitle, focus: (sr) => sr.keyboardCommands.WhereAmIPresenter.WhereAmIDetailed },
}
const reader = READERS[platform]
if (!reader) throw new Error(`no screen reader for ${platform}`)
if (!existsSync(exe)) throw new Error(`${exe} missing: build it with (cd app && npx tauri build --debug --no-bundle)`)
if (!existsSync(join(tools, 'package.json'))) throw new Error(`${tools} has no package.json: npm install @guidepup/guidepup there (see .github/workflows/screen-reader.yml)`)
const guidepup = createRequire(join(tools, 'package.json'))('@guidepup/guidepup')
const sr = { win32: guidepup.nvda, darwin: guidepup.voiceOver, linux: guidepup.orca }[platform]

// Keys as each platform spells them: ⌘ for Ctrl on macOS (Control is VoiceOver's own key, and
// the app accepts either), Option+Q for Alt+Q, ⌘→ for End (End scrolls on macOS).
const MOD = mac ? 'Command' : 'Control'
const SEARCH = mac ? 'Option+q' : 'Alt+q'
const LINE_END = mac ? 'Command+ArrowRight' : 'End'
const REDO = mac ? 'Command+Shift+z' : 'Control+y'

rmSync(out, { recursive: true, force: true })
mkdirSync(out, { recursive: true })
const copy = join(out, 'acme-outfitters')
cpSync(join(root, 'reference', 'examples', 'acme-outfitters'), copy, { recursive: true })
const sendKeysVbs = join(out, 'sendkeys.vbs')
if (win) writeFileSync(sendKeysVbs, 'CreateObject("WScript.Shell").SendKeys WScript.Arguments(0)\r\n')

const results = []
let app = null

// Screen readers split the names: NVDA says "Acme Outfitters dot pbip - Dash Craft".
const DOCUMENT = /Acme ?Outfitters/i
const TITLE = /Acme ?Outfitters.*Dash ?Craft|Dash ?Craft.*Acme ?Outfitters/i
const DIRTY = /\*|\bstar\b|asterisk/i
const INVALID = /invalid entry|invalid data|Error at line|Nope/i

const said = async () =>
  (await sr.spokenPhraseLog())
    .filter(Boolean)
    .join(' | ')
    .replace(/\s+/g, ' ')
    .trim()

/** Waits `ms` while recording what the screen reader says (live regions, late announcements). */
const settle = (ms) => sr.capture(() => sleep(ms), { capture: true })

const press = (key) => sr.press(key)

/**
 * Types `text` into the focused field. Guidepup's NVDA `type` hands each character to WScript
 * SendKeys unescaped, where `+ ^ % ~ ( ) { } [ ]` are key codes: send the escaped text instead.
 */
async function type(text) {
  if (!win) return sr.type(text)
  const keys = text.replace(/[+^%~(){}[\]]/g, '{$&}')
  return sr.capture(() => new Promise((resolve, reject) => execFile('cscript', ['//nologo', sendKeysVbs, keys], (e) => (e ? reject(e) : resolve()))))
}

/** Escape; NVDA's first Escape in focus mode only switches to browse mode, so then a second one. */
async function escape() {
  await press('Escape')
  if (win && /browse mode/i.test(await said())) await press('Escape')
}

/** Tab (or Shift+Tab) until the screen reader says something matching `re`; the presses it took. */
async function tabUntil(re, max, shift = false) {
  for (let i = 1; i <= max; i++) {
    const before = (await sr.spokenPhraseLog()).length
    await press(shift ? 'Shift+Tab' : 'Tab')
    const now = (await sr.spokenPhraseLog()).slice(before).join(' ')
    if (re.test(now)) return i
  }
  return 0
}

/**
 * One step: runs `run`, then checks what was said since the step began against `expect` (a
 * RegExp, or a function of the text). Failures are recorded and the run goes on.
 */
async function step(id, name, keys, run, expect, { required = true } = {}) {
  await sr.clearSpokenPhraseLog()
  let error = ''
  let detail = ''
  try {
    const r = await run()
    if (typeof r === 'string') detail = r
  } catch (e) {
    error = e?.message ?? String(e)
  }
  const heard = await said()
  const ok = !error && (expect == null || (typeof expect === 'function' ? expect(heard) : expect.test(heard)))
  let screenshot = ''
  if (!ok && required && mac) {
    try {
      execFileSync('screencapture', ['-x', join(out, `${id}.png`)], { timeout: 20000 })
      screenshot = `${id}.png`
    } catch (e) {
      console.log(`screencapture: ${e.message.split('\n')[0]}`)
    }
  }
  results.push({ id, name, keys, ok, required, heard, detail, error, screenshot })
  console.log(`${ok ? 'PASS' : required ? 'FAIL' : 'NOTE'} ${id} ${name}${detail ? ` (${detail})` : ''}${error ? `: ${error}` : ''}\n     heard: ${heard.slice(0, 400)}`)
  return ok
}

/** Brings the app window to the front so the keys and the screen reader go to it; false if it can't. */
function activate() {
  try {
    if (win) {
      const ps = `Add-Type -Name W -Namespace U -MemberDefinition '[DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c); [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);'; $h = (Get-Process -Id ${app.pid}).MainWindowHandle; [void][U.W]::ShowWindow($h, 9); [void][U.W]::SetForegroundWindow($h); [void](New-Object -ComObject WScript.Shell).AppActivate(${app.pid})`
      execFileSync('powershell', ['-NoProfile', '-Command', ps], { timeout: 20000 })
    } else if (mac) {
      execFileSync('osascript', ['-e', `tell application "System Events" to set frontmost of (first process whose unix id is ${app.pid}) to true`], { timeout: 20000 })
    } else {
      const id = execFileSync('xdotool', ['search', '--sync', '--onlyvisible', '--pid', String(app.pid)], { encoding: 'utf8', timeout: 20000 }).trim().split('\n').pop()
      execFileSync('xdotool', ['windowfocus', '--sync', id], { timeout: 20000 })
      execFileSync('xdotool', ['mousemove', '--window', id, '400', '300'], { timeout: 20000 })
    }
    return true
  } catch (e) {
    console.log(`activate: ${e.message.split('\n')[0]}`)
    return false
  }
}

function writeReport(startedAt) {
  const failed = results.filter((r) => !r.ok && r.required)
  const report = { reader: reader.name, platform, startedAt, finishedAt: new Date().toISOString(), passed: results.filter((r) => r.ok).length, failed: failed.length, notes: results.filter((r) => !r.ok && !r.required).length, results }
  writeFileSync(join(out, 'report.json'), `${JSON.stringify(report, null, 2)}\n`)
  const cell = (s) => s.replace(/\|/g, '\\|').replace(/\s+/g, ' ').slice(0, 300)
  const md = [
    `## A6 screen reader: ${reader.name} on ${platform}`,
    '',
    `${report.passed} passed, ${report.failed} failed, ${report.notes} notes (informational steps that did not match).`,
    '',
    '| Step | Keys | Result | Heard |',
    '|---|---|---|---|',
    ...results.map((r) => `| ${r.id} ${cell(r.name)}${r.detail ? ` (${cell(r.detail)})` : ''} | ${cell(r.keys)} | ${r.ok ? 'pass' : r.required ? '**FAIL**' : 'note'}${r.error ? `: ${cell(r.error)}` : ''}${r.screenshot ? ` (${r.screenshot})` : ''} | ${cell(r.heard) || '(nothing)'} |`),
    '',
  ].join('\n')
  writeFileSync(join(out, 'report.md'), md)
  if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, md)
  return failed.length
}

async function journey() {
  const title = () => sr.perform(reader.title(sr))
  const focus = () => sr.perform(reader.focus(sr))

  // Wait for the window (System Events lists the process only once it has one) and the
  // document: the window title names the file once it is open.
  let ready = false
  for (let i = 0; i < 40 && !ready; i++) {
    const front = activate()
    await sr.clearSpokenPhraseLog()
    await title().catch(() => {})
    ready = front && DOCUMENT.test(await said())
    if (!ready) await sleep(3000)
  }
  await step('S1', 'A1 the window title names the document and DashCraft', 'title', title, TITLE)

  // M6: find the measure, edit its formula, hear the edit, undo and redo it.
  await step('S2', 'A2 Alt+Q focuses the search and says its name', SEARCH, () => press(SEARCH), /Search commands and fields/)
  await step('S3', 'A2 typing in the search', 'Total Sales', async () => {
    await type('Total Sales')
    await settle(1500)
  }, /Total Sales/, { required: false })
  await step('S4', 'A2 Enter selects the measure', 'Enter', () => press('Enter'), null)
  await step('S5', 'A2 Tab reaches the measure formula by its name', 'Tab ×n', async () => `${await tabUntil(/Measure formula/, 40)} Tab presses`, /Measure formula/)
  await step('S6', 'M7 the formula field describes its keys', 'focus report', focus, /Tab indents/)
  await step('S7', 'A1 committing the formula announces the edit', `${LINE_END}, " * 1", Enter`, async () => {
    await press(LINE_END)
    await type(' * 1')
    await press('Enter')
    await settle(1500)
  }, /Edit measure/)
  await step('S8', 'A1 the title marks unsaved changes', 'title', title, DIRTY, { required: false })
  await step('S9', 'A2 the quick access Undo button is reachable and named', `${SEARCH}, Shift+Tab ×n`, async () => {
    await press(SEARCH)
    return `${await tabUntil(/Undo/, 5, true)} Shift+Tab presses`
  }, /Undo/)
  await step('S10', 'A1 undo announces what it undid', `${MOD}+z`, async () => {
    await press(`${MOD}+z`)
    await settle(1000)
  }, /Undid/)
  await step('S11', 'A1 redo announces what it redid', REDO, async () => {
    await press(REDO)
    await settle(1000)
  }, /Redid/)

  // M7: IntelliSense, diagnostics, Ctrl+K Ctrl+I, line keys and the Tab-focus toggle.
  await step('S12', 'M7 back to the formula', `${SEARCH}, "Total Sales", Enter, Tab ×n`, async () => {
    await press(SEARCH)
    await type('Total Sales')
    await settle(1500)
    await press('Enter')
    return `${await tabUntil(/Measure formula/, 40)} Tab presses`
  }, /Measure formula/)
  await step('S13', 'M7 typing a name announces the suggestions', `${LINE_END}, " + COUNT"`, async () => {
    await press(LINE_END)
    await type(' + COUNT')
    await settle(1500)
  }, /suggestions/i)
  await step('S14', 'M7 arrows say the active suggestion', 'ArrowDown', () => press('ArrowDown'), /COUNT/)
  await step('S15', 'M7 Tab inserts the suggestion and says so', '"R", Tab', async () => {
    await type('R')
    await settle(1000)
    await press('Tab')
    await settle(1000)
  }, /Inserted COUNTROWS/)
  await step('S16', 'M7 a problem is part of the field description', '"Orders) * [Nope]", focus report', async () => {
    await type('Orders) * [Nope]')
    await settle(2500)
    await focus()
  }, /Error at line|unknown/i)
  await step('S17', 'M7 Ctrl+K Ctrl+I reads the description at the cursor', `ArrowLeft ×2, ${MOD}+k, ${MOD}+i`, async () => {
    await press('ArrowLeft')
    await press('ArrowLeft')
    await press(`${MOD}+k`)
    await press(`${MOD}+i`)
    await settle(1500)
  }, /unknown/i)
  await step('S18', 'M7 Shift+Enter and Alt+↑ edit lines', `${LINE_END}, Shift+Enter, Alt+ArrowUp`, async () => {
    await press(LINE_END)
    await press('Shift+Enter')
    await press('Alt+ArrowUp')
    await settle(1000)
  }, null, { required: false })
  await step('S19', 'M7 Ctrl+M makes Tab move focus and says so', `${MOD}+m`, async () => {
    await press(`${MOD}+m`)
    await settle(1000)
  }, /Tab now moves focus/)
  // Shift+Enter expanded the bar, so its button now says Collapse.
  await step('S20', 'M7 Tab then leaves the formula', 'Tab', () => press('Tab'), /(Expand|Collapse) the formula bar/)
  await step('S21', 'M7 Shift+Tab comes back to the formula', 'Shift+Tab', () => press('Shift+Tab'), /Measure formula/)
  // A pop-up still open (a description, the list) takes the first Esc.
  await step('S22', 'M7 Esc cancels the formula edit', 'Escape, focus report', async () => {
    await escape()
    await settle(1000)
    await sr.clearSpokenPhraseLog()
    await focus()
    if (!INVALID.test(await said())) return 'one Escape'
    await escape()
    await settle(1000)
    await sr.clearSpokenPhraseLog()
    await focus()
    return 'two Escapes'
  }, (heard) => /Measure formula/.test(heard) && !INVALID.test(heard), { required: false })

  // M6: the Properties pane, the File menu and its question, saving.
  await step('S23', 'A2 the search opens the Properties pane', `${SEARCH}, "Properties", Enter`, async () => {
    await press(SEARCH)
    await type('Properties')
    await settle(1500)
    await press('Enter')
  }, null, { required: false })
  await step('S24', 'A2 Shift+Tab reaches the Description field and its keys', 'Shift+Tab ×n', async () => `${await tabUntil(/Description/, 80, true)} Shift+Tab presses`, /Description/)
  await step('S25', 'A1 Ctrl+Enter applies the description and announces it', `"Sum of line amounts", ${MOD}+Enter`, async () => {
    await type('Sum of line amounts')
    await press(`${MOD}+Enter`)
    await settle(1500)
  }, /Change the description/)
  await step('S26', 'A2 the File button is reachable and named', `${SEARCH}, Tab`, async () => {
    await press(SEARCH)
    await press('Tab')
  }, /File/)
  await step('S27', 'A2 the File menu takes focus and says its name', 'Enter', async () => {
    await press('Enter')
    await settle(1000)
  }, /File/)
  await step('S28', 'A2 Tab reaches Close in the File menu', 'Tab ×n', async () => `${await tabUntil(/Close/, 20)} Tab presses`, /Close/)
  await step('S29', 'A1 closing with unsaved edits asks the question', 'Enter', async () => {
    await press('Enter')
    await settle(1500)
  }, /Unsaved changes|save your changes/i)
  await step('S30', 'A2 Esc cancels the question and focus goes back to File', 'Escape', async () => {
    await escape()
    await settle(1000)
  }, /File/)
  await step('S31', 'A1 saving announces the files written', `${MOD}+s`, async () => {
    await press(`${MOD}+s`)
    await settle(3000)
  }, /Saved/)
  await step('S32', 'A1 the title drops the unsaved mark', 'title', title, (heard) => DOCUMENT.test(heard) && !DIRTY.test(heard), { required: false })
}

/**
 * Starts the screen reader. On the macOS runners VoiceOver's start request sometimes fails for a
 * while (Guidepup retries twice within seconds), so wait and try again, and log why it failed.
 */
async function startReader(options) {
  for (let attempt = 1; ; attempt++) {
    try {
      return await sr.start(options)
    } catch (e) {
      const chain = []
      for (let x = e; x; x = x.cause) chain.push(x.message ?? String(x))
      console.log(`start attempt ${attempt}: ${chain.join(' <- ')}`)
      if (mac) {
        try {
          execFileSync('/usr/bin/osascript', ['-l', 'JavaScript', '-e', 'ObjC.bindFunction("dlopen", ["void *", ["char *", "int"]]); $.dlopen("/usr/lib/libScreenReader.dylib", 1) ? "libScreenReader loads" : "libScreenReader missing"'], { encoding: 'utf8', timeout: 20000 })
            .split('\n')
            .filter(Boolean)
            .forEach((line) => console.log(`osascript: ${line}`))
        } catch (d) {
          console.log(`osascript: ${String(d.stderr || d.message).trim()}`)
        }
      }
      if (attempt >= 3) throw e
      await sr.stop().catch(() => {})
      await sleep(20000)
    }
  }
}

const startedAt = new Date().toISOString()
let failed = 1
try {
  console.log(`starting ${reader.name}`)
  // Guidepup's default capture keeps only the first phrase after each key: NVDA and Orca say
  // "banner landmark" first and the field after it. VoiceOver says both in one phrase, and its
  // full capture polls AppleScript until the phrase is stable (seconds per key), so it keeps the
  // first phrase there; settle() captures everything for the late announcements.
  await startReader({ capture: mac ? 'initial' : true })
  const log = openSync(join(out, 'app.log'), 'w')
  const env = { ...process.env, DASHCRAFT_BLOB_MAP: join(copy, 'blob-map.json') }
  if (platform === 'linux') Object.assign(env, { WEBKIT_DISABLE_DMABUF_RENDERER: '1', WEBKIT_DISABLE_COMPOSITING_MODE: '1' })
  app = spawn(exe, [join(copy, 'AcmeOutfitters.pbip')], { env, stdio: ['ignore', log, log] })
  await journey()
} catch (e) {
  const error = [e, e?.cause].filter(Boolean).map((x) => x?.stack ?? String(x)).join('\nCaused by: ')
  results.push({ id: 'S0', name: 'the run', keys: '', ok: false, required: true, heard: '', detail: '', error })
  console.log(`FAIL S0 the run: ${error}`)
} finally {
  failed = writeReport(startedAt)
  await sr.stop().catch((e) => console.log(`stop: ${e.message}`))
  app?.kill()
}
console.log(`${results.filter((r) => r.ok).length} passed, ${failed} failed; report in ${join(out, 'report.md')}`)
process.exit(failed ? 1 : 0)
