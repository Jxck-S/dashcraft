// Drives the debug app over the WebView2 DevTools protocol (Windows only) for the app checks in
// this folder. Each run works on a fresh copy of the Acme Outfitters example under
// target/tmp/app-checks/, so saves never touch the committed fixture.
import { spawn, execFileSync } from 'node:child_process'
import { cpSync, existsSync, rmSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..')
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
export const CTRL = 2
export const ALT = 1

/**
 * Copies Acme, starts the app on it with DevTools on `port` and connects to the page. `keep`
 * starts again on the copy and data folder of the previous launch (a restart after a crash).
 */
export async function launch(port, exe = process.argv[2] ?? join(root, 'target', 'debug', 'dashcraft-app.exe'), { keep = false } = {}) {
  if (process.platform !== 'win32') throw new Error('the app checks drive WebView2 and run on Windows only')
  if (!existsSync(exe)) throw new Error(`${exe} missing: build it with (cd app && npx tauri build --debug --no-bundle)`)
  const copy = join(root, 'target', 'tmp', 'app-checks', 'acme-outfitters')
  // A fresh data folder too: no Auto recovery journals of an earlier run (Document recovery dialog).
  const data = join(root, 'target', 'tmp', 'app-checks', 'data')
  if (!keep) {
    rmSync(copy, { recursive: true, force: true })
    rmSync(data, { recursive: true, force: true })
    cpSync(join(root, 'reference', 'examples', 'acme-outfitters'), copy, { recursive: true })
  }
  const app = spawn(exe, [join(copy, 'AcmeOutfitters.pbip')], {
    env: {
      ...process.env,
      DASHCRAFT_BLOB_MAP: join(copy, 'blob-map.json'),
      DASHCRAFT_DATA_DIR: data,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
    },
    stdio: 'ignore',
  })
  let target
  for (let i = 0; i < 150 && !target; i++) {
    try {
      target = (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).find((t) => t.type === 'page')
    } catch {}
    if (!target) await sleep(200)
  }
  if (!target) {
    app.kill()
    throw new Error('no DevTools target')
  }
  const ws = new WebSocket(target.webSocketDebuggerUrl)
  await new Promise((r, j) => {
    ws.onopen = r
    ws.onerror = j
  })
  let seq = 0
  const pending = new Map()
  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data)
    const p = pending.get(msg.id)
    if (!p) return
    pending.delete(msg.id)
    msg.error ? p.reject(new Error(JSON.stringify(msg.error))) : p.resolve(msg.result)
  }
  const send = (method, params = {}) => {
    const id = ++seq
    ws.send(JSON.stringify({ id, method, params }))
    return new Promise((resolve, reject) => pending.set(id, { resolve, reject }))
  }
  async function evaluate(expression) {
    const r = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true })
    if (r.exceptionDetails) throw new Error(`${expression.slice(0, 80)}: ${r.exceptionDetails.exception?.description ?? r.exceptionDetails.text}`)
    return r.result.value
  }
  async function waitFor(expression, timeout = 30000) {
    const start = Date.now()
    for (;;) {
      const v = await evaluate(expression).catch(() => null)
      if (v) return v
      if (Date.now() - start > timeout) return null
      await sleep(200)
    }
  }
  async function key(k, code, keyCode, modifiers = 0) {
    await send('Input.dispatchKeyEvent', { type: 'keyDown', key: k, code, windowsVirtualKeyCode: keyCode, modifiers })
    await send('Input.dispatchKeyEvent', { type: 'keyUp', key: k, code, windowsVirtualKeyCode: keyCode, modifiers })
  }
  const type = (text) => send('Input.insertText', { text })
  /** UI Automation elements with these names (what Narrator and NVDA read), one line each. */
  const uia = (names) => execFileSync('pwsh', ['-NoProfile', '-File', join(root, 'scripts', 'app-checks', 'uia.ps1'), String(app.pid), ...names], { encoding: 'utf8' })
  const close = () => {
    ws.close()
    app.kill()
  }
  await send('Runtime.enable')
  return { app, copy, data, send, evaluate, waitFor, key, type, uia, close }
}

/** The window title as the title bar shows it. */
export const title = `(document.querySelector('.title-bar .title')?.textContent ?? '')`

/** Selects Orders[Total Sales] in the Data pane and waits for the editable formula bar. */
export async function selectTotalSales({ evaluate, waitFor }) {
  await waitFor(`[...document.querySelectorAll('.table-row .label .name')].some((n) => n.textContent === 'Orders')`)
  await evaluate(`(() => { const row = [...document.querySelectorAll('.table-row')].find((r) => r.querySelector('.label .name')?.textContent === 'Orders'); if (row.parentElement.getAttribute('aria-expanded') !== 'true') row.querySelector('.twisty').click(); return true })()`)
  await waitFor(`[...document.querySelectorAll('.leaf .name')].some((n) => n.textContent === 'Total Sales')`)
  await evaluate(`[...document.querySelectorAll('.leaf')].find((b) => b.querySelector('.name')?.textContent === 'Total Sales').click(), true`)
  return waitFor(`document.querySelector('.formula-bar textarea')?.value`)
}

/** Opens the Properties pane (View → Properties) unless it is open. */
export async function openProperties({ evaluate, waitFor }) {
  if (await evaluate(`!!document.querySelector('#prop-description')`)) return true
  await evaluate(`[...document.querySelectorAll('[role="tab"]')].find((t) => t.textContent.trim() === 'View')?.click(), true`)
  await waitFor(`!!document.querySelector('button[title^="DashCraft: show the Properties pane"]')`, 5000)
  await evaluate(`document.querySelector('button[title^="DashCraft: show the Properties pane"]')?.click(), true`)
  return waitFor(`!!document.querySelector('#prop-description')`, 5000)
}

/** Opens File (Alt+F, else the File tab). */
export async function openFileMenu({ evaluate, waitFor, key }) {
  await evaluate(`document.activeElement?.blur(), true`)
  await key('f', 'KeyF', 70, ALT)
  if (await waitFor(`!!document.querySelector('.backstage')`, 3000)) return true
  await evaluate(`[...document.querySelectorAll('button')].find((b) => b.textContent.trim() === 'File')?.click(), true`)
  return waitFor(`!!document.querySelector('.backstage')`, 3000)
}

/** PASS/FAIL lines and the exit code. */
export function reporter() {
  const results = []
  return {
    check(name, ok, detail = '') {
      results.push(!!ok)
      console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? `: ${detail}` : ''}`)
    },
    finish() {
      const failed = results.filter((r) => !r).length
      console.log(`${results.length - failed} passed, ${failed} failed`)
      process.exit(failed ? 1 : 0)
    },
  }
}
