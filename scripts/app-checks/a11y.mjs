// Accessibility helpers of the app checks: the IBM Equal Access checker (accessibility-checker-engine,
// Apache-2.0, installed under target/tmp/a11y and not a dependency of the repository) and, in the
// page, `window.__a11y`: contrast ratios, focus rings, accessible names and a checker run limited
// to a CSS scope.
import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { root } from './cdp.mjs'

export const aceJs = join(root, 'target', 'tmp', 'a11y', 'node_modules', 'accessibility-checker-engine', 'ace.js')

// In the page: contrast ratios, the focused element, and a checker run limited to `scope`.
const helpers = `
window.__a11y = {
  lum(c) { const m = c.match(/[\\d.]+/g).map(Number); const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4 }; return 0.2126 * f(m[0]) + 0.7152 * f(m[1]) + 0.0722 * f(m[2]) },
  ratio(a, b) { const [x, y] = [this.lum(a), this.lum(b)].sort((p, q) => q - p); return Math.round((x + 0.05) / (y + 0.05) * 100) / 100 },
  bg(el) { for (let e = el; e; e = e.parentElement) { const c = getComputedStyle(e).backgroundColor; const a = c.match(/[\\d.]+/g); if (a && (a.length < 4 || Number(a[3]) > 0.5)) return c } return 'rgb(255, 255, 255)' },
  ring(el, inner = el) { const s = getComputedStyle(el); return { outline: s.outlineStyle + ' ' + s.outlineWidth, ratio: s.outlineStyle === 'none' ? 0 : Math.min(this.ratio(s.outlineColor, this.bg(inner)), this.ratio(s.outlineColor, this.bg(el.parentElement))), ok: s.outlineStyle !== 'none' && parseFloat(s.outlineWidth) >= 2 } },
  name(el) { return el.getAttribute('aria-label') || el.labels?.[0]?.textContent?.trim() || el.textContent.trim().slice(0, 40) || el.title },
  focused() { const el = document.activeElement; return !el || el === document.body ? null : { name: this.name(el), ...this.ring(el) } },
  async check(scope) {
    const report = await new ace.Checker().check(document, ['WCAG_2_2'])
    const out = []
    for (const r of report.results) {
      if (r.value[1] === 'PASS') continue
      const node = document.evaluate(r.path.dom, document, null, XPathResult.FIRST_ORDERED_NODE_TYPE, null).singleNodeValue
      const el = node?.nodeType === 1 ? node : node?.parentElement
      if (el?.closest(scope)) out.push({ rule: r.ruleId, level: r.value.join('/'), msg: r.message, where: String(el.getAttribute('aria-label') || el.id || el.className || el.tagName).slice(0, 50) })
    }
    return out
  },
}
true`

/** Exits with 2 when the checker is not installed (call it before starting the app). */
export function requireChecker() {
  if (existsSync(aceJs)) return
  console.error(`${aceJs} missing: npm install --prefix target/tmp/a11y --no-save accessibility-checker-engine`)
  process.exit(2)
}

/** Loads the checker and `window.__a11y` into the page. */
export async function installChecker({ evaluate }) {
  await evaluate(readFileSync(aceJs, 'utf8') + '\n;window.ace = ace; true')
  await evaluate(helpers)
}
