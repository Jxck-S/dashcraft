// Pure helpers of the formula bar's DAX editor (M7.5): keeping the language services' ranges
// aligned while the user types, the coloured and squiggled segments of the overlay, caret geometry, when to ask for
// completion or signature help, and the key commands. No backend calls, so they are unit-tested
// with `node --test` (tests/daxEditor.test.ts).

import type { DaxCompleteOut, DaxDiagnosticOut, DaxHoverInfo, DaxSignatureOut, DaxTokenKind, DaxTokenOut } from './ops.gen.ts'

/**
 * The DAX language services an editor draws on (the `dax.*` registry commands against the
 * current document); each answer is for the text it was given, `null` when there is none or the
 * request failed or was superseded.
 */
export interface DaxService {
  complete(text: string, offset: number): Promise<DaxCompleteOut | null>
  signatureHelp(text: string, offset: number): Promise<DaxSignatureOut | null>
  hover(text: string, offset: number): Promise<DaxHoverInfo | null>
  diagnostics(text: string): Promise<DaxDiagnosticOut[] | null>
  tokens(text: string): Promise<DaxTokenOut[] | null>
  format(text: string): Promise<{ ok: true; text: string } | { ok: false; error: string }>
}

/** A half-open range of UTF-16 offsets (JavaScript string indices). */
export interface Span {
  start: number
  end: number
}

/** An edit between two texts: `removed` code units at `start` replaced by `inserted`. */
export interface Delta {
  start: number
  removed: number
  inserted: number
}

/** The edit that turns `before` into `after` (the longest common prefix and suffix kept). */
export function textDelta(before: string, after: string): Delta {
  const max = Math.min(before.length, after.length)
  let start = 0
  while (start < max && before.charCodeAt(start) === after.charCodeAt(start)) start++
  let tail = 0
  while (tail < max - start && before.charCodeAt(before.length - 1 - tail) === after.charCodeAt(after.length - 1 - tail)) tail++
  return { start, removed: before.length - start - tail, inserted: after.length - start - tail }
}

/**
 * Where `offset` of the old text is in the new one. Text inserted exactly at `offset` lands
 * after it with `bias: 'right'` (a range end that grows as the user types) and before it with
 * `'left'`; an offset inside replaced text moves to the end of the replacement.
 */
export function shiftOffset(offset: number, delta: Delta, bias: 'left' | 'right' = 'left'): number {
  const end = delta.start + delta.removed
  if (offset < delta.start) return offset
  if (offset > end) return offset - delta.removed + delta.inserted
  if (delta.removed === 0) return bias === 'right' ? offset + delta.inserted : offset
  return offset === delta.start ? offset : delta.start + delta.inserted
}

/**
 * Ranges of the old text moved into the new one: a range the edit only inserts into grows, a
 * range whose text was replaced is dropped (the next answer of the service brings it back).
 */
export function shiftSpans<T extends Span>(spans: readonly T[], delta: Delta): T[] {
  if (delta.removed === 0 && delta.inserted === 0) return [...spans]
  const out: T[] = []
  const end = delta.start + delta.removed
  for (const s of spans) {
    if (s.end <= delta.start) out.push(s)
    else if (s.start >= end) out.push({ ...s, start: s.start - delta.removed + delta.inserted, end: s.end - delta.removed + delta.inserted })
    else if (delta.removed === 0 && s.start < delta.start && delta.start < s.end) out.push({ ...s, end: s.end + delta.inserted })
  }
  return out
}

// ---------------------------------------------------------------------------------------------
// The coloured overlay
// ---------------------------------------------------------------------------------------------

export type Severity = 'error' | 'warning'

/** A token of `dax.tokens` or a diagnostic of `dax.diagnostics`, in formula offsets. */
export interface TokenSpan extends Span {
  kind: DaxTokenKind
}

export interface MarkSpan extends Span {
  severity: Severity
}

/** A run of the overlay: one colour, one squiggle. `extra` text is not in the formula. */
export interface Segment {
  text: string
  token?: DaxTokenKind
  severity?: Severity
  extra?: boolean
}

/**
 * Token colours on the editor's background (`--color-code-bg`, #fbfbfb), each at least 4.5:1
 * (tests/daxEditor.test.ts); names and operators keep the text colour.
 */
export const TOKEN_COLORS: Record<DaxTokenKind, string | null> = {
  keyword: '#0037da',
  function: '#7a4b00',
  userFunction: '#7a4b00',
  string: '#a31515',
  number: '#0b6a3d',
  dateTime: '#0b6a3d',
  comment: '#3b6e22',
  variable: '#5c2e91',
  parameter: '#5c2e91',
  queryParameter: '#5c2e91',
  measure: '#0f5b8f',
  table: null,
  column: null,
  operator: null,
}

/** The text background the token colours are checked against. */
export const CODE_BACKGROUND = '#fbfbfb'

/**
 * Splits `text` into runs of one token colour and one squiggle. Tokens do not overlap; marks
 * may (an error wins over a warning). An empty mark squiggles the character after it, else the
 * one before, else an extra space at the end of its line.
 */
export function segments(text: string, tokens: readonly TokenSpan[], marks: readonly MarkSpan[]): Segment[] {
  const clamp = (n: number) => Math.max(0, Math.min(text.length, n))
  const extras: number[] = []
  const shown: MarkSpan[] = []
  for (const m of marks) {
    let start = clamp(m.start)
    let end = clamp(Math.max(m.start, m.end))
    if (start === end) {
      if (end < text.length && text[end] !== '\n') end++
      else if (start > 0 && text[start - 1] !== '\n') start--
      else {
        extras.push(start)
        shown.push({ start, end: start, severity: m.severity })
        continue
      }
    }
    shown.push({ start, end, severity: m.severity })
  }
  const cuts = new Set<number>([0, text.length, ...extras])
  for (const s of [...tokens, ...shown]) {
    cuts.add(clamp(s.start))
    cuts.add(clamp(s.end))
  }
  const points = [...cuts].sort((a, b) => a - b)
  const out: Segment[] = []
  const push = (seg: Segment) => {
    const last = out[out.length - 1]
    if (last && !last.extra && !seg.extra && last.token === seg.token && last.severity === seg.severity) last.text += seg.text
    else out.push(seg)
  }
  const severityAt = (from: number, to: number): Severity | undefined => {
    let found: Severity | undefined
    for (const m of shown) {
      if (m.start < to && from < m.end) {
        if (m.severity === 'error') return 'error'
        found = 'warning'
      }
    }
    return found
  }
  const sorted = [...tokens].sort((a, b) => a.start - b.start)
  let next = 0
  for (let i = 0; i < points.length; i++) {
    const from = points[i]
    if (extras.includes(from)) {
      const severity = shown.some((m) => m.start === from && m.end === from && m.severity === 'error') ? 'error' : 'warning'
      push({ text: ' ', severity, extra: true })
    }
    const to = points[i + 1]
    if (to === undefined || to === from) continue
    while (next < sorted.length && sorted[next].end <= from) next++
    const t = sorted[next]
    const token = t && t.start <= from && to <= t.end ? t.kind : undefined
    const seg: Segment = { text: text.slice(from, to) }
    if (token) seg.token = token
    const severity = severityAt(from, to)
    if (severity) seg.severity = severity
    push(seg)
  }
  return out
}

// ---------------------------------------------------------------------------------------------
// Caret geometry
// ---------------------------------------------------------------------------------------------

/** The 0-based line and visual column of `offset` (tabs to the next multiple of `tabSize`). */
export function lineColumn(text: string, offset: number, tabSize = 4): { line: number; column: number } {
  const at = Math.max(0, Math.min(text.length, offset))
  const before = text.slice(0, at)
  const lineStart = before.lastIndexOf('\n') + 1
  let line = 0
  for (let i = 0; i < lineStart; i++) if (text.charCodeAt(i) === 10) line++
  return { line, column: visualColumn(text.slice(lineStart, at), tabSize) }
}

function visualColumn(prefix: string, tabSize: number): number {
  let column = 0
  for (const ch of prefix) column = ch === '\t' ? column + tabSize - (column % tabSize) : column + 1
  return column
}

/** The offset nearest to visual `column` of 0-based `line` (past the end: the line's end). */
export function offsetAt(text: string, line: number, column: number, tabSize = 4): number {
  const lines = text.split('\n')
  const target = Math.max(0, Math.min(lines.length - 1, line))
  let offset = 0
  for (let i = 0; i < target; i++) offset += lines[i].length + 1
  let visual = 0
  for (const ch of lines[target]) {
    const width = ch === '\t' ? tabSize - (visual % tabSize) : 1
    if (column < visual + width / 2) return offset
    visual += width
    offset += ch.length
  }
  return offset
}

// ---------------------------------------------------------------------------------------------
// Completion and signature help
// ---------------------------------------------------------------------------------------------

const WORD = /[\p{L}\p{N}_]/u

/** What a typed edit does to the completion list and the signature help. */
export interface Triggers {
  complete: 'open' | 'close' | 'keep'
  signature: boolean
}

/**
 * Typing a name character, `[` or `'` asks for completion (the service decides whether a list
 * applies there; a number does not), anything else closes the list; deleting refreshes an open
 * list. `(` and `,` ask for signature help, and any edit refreshes it while it is shown.
 */
export function triggers(after: string, delta: Delta, listOpen: boolean): Triggers {
  if (delta.inserted === 1 && delta.removed === 0) {
    const ch = after[delta.start]
    if (ch === '[' || ch === "'") return { complete: 'open', signature: false }
    if (WORD.test(ch)) {
      let i = delta.start
      while (i > 0 && WORD.test(after[i - 1])) i--
      const word = after.slice(i, delta.start + 1)
      return { complete: /^\d/.test(word) && !listOpen ? 'close' : 'open', signature: false }
    }
    return { complete: 'close', signature: ch === '(' || ch === ',' || ch === ')' }
  }
  if (delta.inserted === 0 && delta.removed > 0) return { complete: listOpen ? 'open' : 'keep', signature: false }
  return { complete: 'close', signature: false }
}

/**
 * The replacement for accepting a completion over `[start, end)`: a function's `(` is not
 * doubled when one already follows.
 */
export function completionEdit(text: string, start: number, end: number, insert: string): { from: number; to: number; text: string; caret: number } {
  if (insert.endsWith('(') && text[end] === '(') return { from: start, to: end, text: insert.slice(0, -1), caret: start + insert.length }
  return { from: start, to: end, text: insert, caret: start + insert.length }
}

/**
 * `text` with its DAX from `at` on replaced by `formatted`: what comes before (`Name =`) is kept,
 * and a multi-line expression starts on the next line, as the DAX formatter lays it out.
 */
export function withFormatted(text: string, at: number, formatted: string): string {
  const body = formatted.trim()
  if (at <= 0) return body
  return `${text.slice(0, at).trimEnd()}${body.includes('\n') ? '\n' : ' '}${body}`
}

/** The active item after a navigation key in a list of `count` items (wrapping at the ends). */
export function moveActive(active: number, count: number, key: string, page = 8): number {
  if (count <= 0) return -1
  switch (key) {
    case 'ArrowDown':
      return (active + 1) % count
    case 'ArrowUp':
      return (active - 1 + count) % count
    case 'PageDown':
      return Math.min(count - 1, active + page)
    case 'PageUp':
      return Math.max(0, active - page)
    default:
      return active
  }
}

/** The signature's label in three parts, the active parameter in the middle. */
export function signatureParts(sig: Pick<DaxSignatureOut, 'label' | 'signature' | 'parameters' | 'activeParameter'>): [string, string, string] {
  const p = sig.activeParameter == null ? undefined : sig.parameters[sig.activeParameter]
  if (!p) return [sig.label, '', '']
  const start = utf16Index(sig.signature, p.labelStart)
  const end = utf16Index(sig.signature, p.labelEnd)
  if (start > end || end > sig.label.length) return [sig.label, '', '']
  return [sig.label.slice(0, start), sig.label.slice(start, end), sig.label.slice(end)]
}

/** The UTF-16 index of UTF-8 byte offset `bytes` in `text`. */
export function utf16Index(text: string, bytes: number): number {
  let seen = 0
  let i = 0
  for (const ch of text) {
    if (seen >= bytes) return i
    const cp = ch.codePointAt(0) ?? 0
    seen += cp < 0x80 ? 1 : cp < 0x800 ? 2 : cp < 0x10000 ? 3 : 4
    i += ch.length
  }
  return i
}

// ---------------------------------------------------------------------------------------------
// Line editing (Desktop's formula editor keys)
// ---------------------------------------------------------------------------------------------

/** A selection as `selectionStart`/`selectionEnd` give it (`start <= end`). */
export interface Selection {
  start: number
  end: number
}

/** Replace `[from, to)` with `text` (as typing would), then select `selection`. */
export interface TextEdit {
  from: number
  to: number
  text: string
  selection: Selection
}

/** One indentation step: four spaces, as `dax.format` and Desktop's editor indent. */
export const INDENT = '    '

function lineStartOf(text: string, offset: number): number {
  return text.lastIndexOf('\n', offset - 1) + 1
}

function lineEndOf(text: string, offset: number): number {
  const i = text.indexOf('\n', offset)
  return i < 0 ? text.length : i
}

function leadingSpace(line: string): string {
  return /^[ \t]*/.exec(line)?.[0] ?? ''
}

/**
 * The lines a selection touches, from the start of the first to the end of the last (without
 * its line break). A selection that ends at the start of a line leaves that line out.
 */
export function selectedLines(text: string, sel: Selection): { start: number; end: number } {
  const start = lineStartOf(text, sel.start)
  const last = sel.end > sel.start && text[sel.end - 1] === '\n' ? sel.end - 1 : sel.end
  return { start, end: lineEndOf(text, Math.max(start, last)) }
}

/**
 * Rewrites the selected lines with `rewrite` (line → new line and how far a column moves) and
 * maps the selection: a column follows its text; a selection that starts a line keeps it.
 */
function rewriteLines(text: string, sel: Selection, rewrite: (line: string, many: boolean) => { line: string; shift: number }): TextEdit {
  const { start, end } = selectedLines(text, sel)
  const lines = text.slice(start, end).split('\n')
  const out = lines.map((l) => rewrite(l, lines.length > 1))
  const map = (offset: number, keepLineStart: boolean): number => {
    let old = start
    let now = start
    for (let i = 0; i < lines.length; i++) {
      const lineEnd = old + lines[i].length
      if (offset <= lineEnd || i === lines.length - 1) {
        const column = Math.max(0, Math.min(lines[i].length, offset - old))
        if (keepLineStart && column === 0) return now
        return now + Math.max(0, Math.min(out[i].line.length, column + out[i].shift))
      }
      old = lineEnd + 1
      now += out[i].line.length + 1
    }
    return now
  }
  const collapsed = sel.start === sel.end
  const startAt = map(sel.start, !collapsed)
  const endAt = sel.end > end ? sel.end + out.reduce((n, o, i) => n + o.line.length - lines[i].length, 0) : map(sel.end, false)
  return { from: start, to: end, text: out.map((o) => o.line).join('\n'), selection: { start: startAt, end: collapsed ? startAt : endAt } }
}

/** Ctrl+]: indents the selected lines one step (empty lines too when only one is selected). */
export function indentLines(text: string, sel: Selection, unit = INDENT): TextEdit {
  return rewriteLines(text, sel, (line, many) => (many && line.length === 0 ? { line, shift: 0 } : { line: unit + line, shift: unit.length }))
}

/** Ctrl+[ and Shift+Tab: removes one step of indentation (a tab or up to `unit`'s spaces). */
export function outdentLines(text: string, sel: Selection, unit = INDENT): TextEdit {
  return rewriteLines(text, sel, (line) => {
    const lead = leadingSpace(line)
    let cut = 0
    if (lead.startsWith('\t')) cut = 1
    else while (cut < unit.length && cut < lead.length && lead[cut] === ' ') cut++
    return { line: line.slice(cut), shift: -cut }
  })
}

/**
 * Tab: a selection over several lines indents them; otherwise the selection is replaced by
 * spaces up to the next tab stop.
 */
export function tabEdit(text: string, sel: Selection, unit = INDENT): TextEdit {
  const lines = selectedLines(text, sel)
  if (sel.start !== sel.end && text.slice(lines.start, lines.end).includes('\n')) return indentLines(text, sel, unit)
  const column = visualColumn(text.slice(lineStartOf(text, sel.start), sel.start), unit.length)
  const spaces = ' '.repeat(unit.length - (column % unit.length))
  const caret = sel.start + spaces.length
  return { from: sel.start, to: sel.end, text: spaces, selection: { start: caret, end: caret } }
}

/** Shift+Enter: a line break at the cursor that keeps the line's indentation. */
export function lineBreak(text: string, sel: Selection): TextEdit {
  const lineStart = lineStartOf(text, sel.start)
  const indent = leadingSpace(text.slice(lineStart, sel.start))
  const caret = sel.start + 1 + indent.length
  return { from: sel.start, to: sel.end, text: `\n${indent}`, selection: { start: caret, end: caret } }
}

/** Ctrl+Shift+Enter: an empty line above the cursor's line, with its indentation. */
export function lineAbove(text: string, sel: Selection): TextEdit {
  const lineStart = lineStartOf(text, sel.start)
  const indent = leadingSpace(text.slice(lineStart, lineEndOf(text, lineStart)))
  const caret = lineStart + indent.length
  return { from: lineStart, to: lineStart, text: `${indent}\n`, selection: { start: caret, end: caret } }
}

/** Ctrl+Shift+K: deletes the selected lines; the cursor keeps its column on the next line. */
export function deleteLines(text: string, sel: Selection): TextEdit {
  const { start, end } = selectedLines(text, sel)
  const column = sel.start - lineStartOf(text, sel.start)
  const from = end < text.length ? start : Math.max(0, start - 1)
  const to = end < text.length ? end + 1 : end
  const rest = text.slice(0, from) + text.slice(to)
  const lineStart = end < text.length ? from : lineStartOf(rest, from)
  const caret = Math.min(lineEndOf(rest, lineStart), lineStart + column)
  return { from, to, text: '', selection: { start: caret, end: caret } }
}

/** Alt+↑ / Alt+↓: moves the selected lines past the line above or below (null at the edge). */
export function moveLines(text: string, sel: Selection, direction: -1 | 1): TextEdit | null {
  const { start, end } = selectedLines(text, sel)
  const block = text.slice(start, end)
  if (direction < 0) {
    if (start === 0) return null
    const prevStart = lineStartOf(text, start - 1)
    const prev = text.slice(prevStart, start - 1)
    const by = prev.length + 1
    return { from: prevStart, to: end, text: `${block}\n${prev}`, selection: { start: sel.start - by, end: sel.end - by } }
  }
  if (end >= text.length) return null
  const nextEnd = lineEndOf(text, end + 1)
  const next = text.slice(end + 1, nextEnd)
  const by = next.length + 1
  return { from: start, to: nextEnd, text: `${next}\n${block}`, selection: { start: sel.start + by, end: Math.min(text.length, sel.end + by) } }
}

/** Shift+Alt+↑ / Shift+Alt+↓: copies the selected lines above or below; the copy is selected. */
export function copyLines(text: string, sel: Selection, direction: -1 | 1): TextEdit {
  const { start, end } = selectedLines(text, sel)
  const block = text.slice(start, end)
  if (direction < 0) return { from: start, to: start, text: `${block}\n`, selection: { ...sel } }
  const by = block.length + 1
  return { from: end, to: end, text: `\n${block}`, selection: { start: sel.start + by, end: sel.end + by } }
}

/** Ctrl+L: selects the cursor's line with its line break; again, the next line too. */
export function selectLine(text: string, sel: Selection): Selection {
  const { start, end } = selectedLines(text, sel)
  const whole = { start, end: Math.min(text.length, end + 1) }
  if (whole.start === sel.start && whole.end === sel.end && whole.end < text.length) {
    return { start, end: Math.min(text.length, lineEndOf(text, whole.end) + 1) }
  }
  return whole
}

/** The text of the selected lines, with a line break, for Ctrl+C and Ctrl+X without a selection. */
export function lineText(text: string, sel: Selection): { text: string; edit: TextEdit } {
  const { start, end } = selectedLines(text, sel)
  return { text: `${text.slice(start, end)}\n`, edit: deleteLines(text, sel) }
}

interface Scanned {
  /** `()`, `{}` and `[]` pairs, by opening offset. */
  pairs: Array<[number, number]>
  /** Strings and quoted table names: the offsets of the two quotes. */
  quoted: Array<[number, number]>
}

/** Brackets and quoted text of DAX, outside comments (a quote or bracket in a string is text). */
function scanDax(text: string): Scanned {
  const pairs: Array<[number, number]> = []
  const quoted: Array<[number, number]> = []
  const stack: Array<[string, number]> = []
  const closing: Record<string, string> = { ')': '(', '}': '{' }
  const closeOf = (from: number, quote: string): number => {
    let j = from + 1
    while (j < text.length) {
      if (text[j] !== quote) j++
      else if (text[j + 1] === quote) j += 2
      else return j
    }
    return -1
  }
  let i = 0
  while (i < text.length) {
    const ch = text[i]
    const next = text[i + 1]
    if ((ch === '/' && next === '/') || (ch === '-' && next === '-')) {
      i = lineEndOf(text, i)
    } else if (ch === '/' && next === '*') {
      const close = text.indexOf('*/', i + 2)
      i = close < 0 ? text.length : close + 2
    } else if (ch === '"' || ch === "'" || ch === '[') {
      const close = closeOf(i, ch === '[' ? ']' : ch)
      if (close < 0) break
      ;(ch === '[' ? pairs : quoted).push([i, close])
      i = close + 1
    } else {
      if (ch === '(' || ch === '{') stack.push([ch, i])
      else if (ch === ')' || ch === '}') {
        const top = stack[stack.length - 1]
        if (top && top[0] === closing[ch]) {
          stack.pop()
          pairs.push([top[1], i])
        }
      }
      i++
    }
  }
  pairs.sort((x, y) => x[0] - y[0])
  return { pairs, quoted }
}

/** The bracket pairs `()`, `{}` and `[]` of DAX text, outside strings, comments and quoted names. */
export function bracketPairs(text: string): Array<[number, number]> {
  return scanDax(text).pairs
}

/**
 * Ctrl+Shift+\: from a bracket next to the cursor to its partner (before it); elsewhere to the
 * closing bracket of the innermost pair around the cursor, else the next bracket.
 */
export function matchingBracket(text: string, caret: number): number | null {
  const pairs = bracketPairs(text)
  for (const at of [caret, caret - 1]) {
    for (const [open, close] of pairs) {
      if (open === at) return close
      if (close === at) return open
    }
  }
  let inner: [number, number] | null = null
  for (const p of pairs) if (p[0] < caret && caret <= p[1] && (!inner || p[0] > inner[0])) inner = p
  if (inner) return inner[1]
  let next: number | null = null
  for (const [open, close] of pairs) for (const at of [open, close]) if (at >= caret && (next === null || at < next)) next = at
  return next
}

/**
 * Shift+Alt+→: the smallest larger range around the selection: the word, string or name at the
 * cursor, the inside of a bracket pair, the pair, the line, then everything.
 */
export function expandSelection(text: string, sel: Selection): Selection | null {
  const candidates: Selection[] = []
  const word = /[\p{L}\p{N}_.]/u
  let ws = sel.start
  let we = sel.end
  while (ws > 0 && word.test(text[ws - 1])) ws--
  while (we < text.length && word.test(text[we])) we++
  candidates.push({ start: ws, end: we })
  const { pairs, quoted } = scanDax(text)
  for (const [open, close] of [...pairs, ...quoted]) {
    candidates.push({ start: open + 1, end: close }, { start: open, end: close + 1 })
  }
  const lines = selectedLines(text, sel)
  candidates.push({ start: lineStartOf(text, sel.start) + leadingSpace(text.slice(lines.start, lines.end)).length, end: lines.end }, lines, { start: 0, end: text.length })
  let best: Selection | null = null
  for (const c of candidates) {
    if (c.start > sel.start || c.end < sel.end || c.end - c.start <= sel.end - sel.start) continue
    if (!best || c.end - c.start < best.end - best.start) best = c
  }
  return best
}

// ---------------------------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------------------------

/** The editor's own commands (the list's navigation keys are handled while it is open). */
export type EditorCommand =
  | 'suggest'
  | 'parameterHints'
  | 'hover'
  | 'format'
  | 'tab'
  | 'shiftTab'
  | 'toggleTabFocus'
  | 'lineBreak'
  | 'lineAbove'
  | 'indentLines'
  | 'outdentLines'
  | 'deleteLines'
  | 'moveLinesUp'
  | 'moveLinesDown'
  | 'copyLinesUp'
  | 'copyLinesDown'
  | 'selectLine'
  | 'jumpToBracket'
  | 'expandSelection'
  | 'shrinkSelection'
  | 'scrollLineUp'
  | 'scrollLineDown'
  | 'scrollPageUp'
  | 'scrollPageDown'

export interface KeyLike {
  key: string
  code?: string
  ctrlKey: boolean
  metaKey: boolean
  shiftKey: boolean
  altKey: boolean
}

/**
 * The command of a key press, as Microsoft documents Desktop's formula editor ("Formula editor in
 * Power BI Desktop") and its DAX query view: Ctrl+Space suggestions, Ctrl+Shift+Space parameter
 * info, Ctrl+K Ctrl+I the description at the cursor (`chord` is true after Ctrl+K), Shift+Alt+F
 * format; Tab and Shift+Tab indent, Ctrl+M makes Tab move focus instead; Shift+Enter and
 * Ctrl+Shift+Enter insert a line; Ctrl+] and Ctrl+[ indent and outdent; Ctrl+Shift+K deletes,
 * Alt+↑ ↓ moves and Shift+Alt+↑ ↓ copies the lines; Ctrl+L selects the line; Ctrl+Shift+\ jumps
 * to the matching bracket; Shift+Alt+→ ← expand and shrink the selection; Ctrl+↑ ↓ and
 * Alt+PgUp PgDn scroll.
 */
export function editorCommand(event: KeyLike, chord = false): EditorCommand | 'chord' | null {
  const mod = event.ctrlKey || event.metaKey
  const key = event.key.length === 1 ? event.key.toLowerCase() : event.key
  const code = event.code ?? ''
  const { shiftKey: shift, altKey: alt } = event
  if (mod && !alt && (key === ' ' || code === 'Space')) return shift ? 'parameterHints' : 'suggest'
  if (shift && alt && !mod) {
    if (key === 'f' || code === 'KeyF') return 'format'
    if (key === 'ArrowUp') return 'copyLinesUp'
    if (key === 'ArrowDown') return 'copyLinesDown'
    if (key === 'ArrowRight') return 'expandSelection'
    if (key === 'ArrowLeft') return 'shrinkSelection'
  }
  if (alt && !shift && !mod) {
    if (key === 'ArrowUp') return 'moveLinesUp'
    if (key === 'ArrowDown') return 'moveLinesDown'
    if (key === 'PageUp') return 'scrollPageUp'
    if (key === 'PageDown') return 'scrollPageDown'
  }
  if (key === 'Tab' && !mod && !alt) return shift ? 'shiftTab' : 'tab'
  if (key === 'Enter' && !alt) {
    if (shift && !mod) return 'lineBreak'
    if (shift && mod) return 'lineAbove'
  }
  if (mod && shift && !alt) {
    if (key === 'k' || code === 'KeyK') return 'deleteLines'
    if (code === 'Backslash' || key === '\\' || key === '|') return 'jumpToBracket'
  }
  if (mod && !shift && !alt) {
    if (chord && (key === 'i' || code === 'KeyI')) return 'hover'
    if (key === 'k' || code === 'KeyK') return 'chord'
    if (key === 'm' || code === 'KeyM') return 'toggleTabFocus'
    if (key === 'l' || code === 'KeyL') return 'selectLine'
    if (key === ']' || code === 'BracketRight') return 'indentLines'
    if (key === '[' || code === 'BracketLeft') return 'outdentLines'
    if (key === 'ArrowUp') return 'scrollLineUp'
    if (key === 'ArrowDown') return 'scrollLineDown'
  }
  return null
}
