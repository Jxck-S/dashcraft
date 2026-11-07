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
// Keys
// ---------------------------------------------------------------------------------------------

/** The editor's own commands (the list's navigation keys are handled while it is open). */
export type EditorCommand = 'suggest' | 'parameterHints' | 'hover' | 'format'

export interface KeyLike {
  key: string
  code?: string
  ctrlKey: boolean
  metaKey: boolean
  shiftKey: boolean
  altKey: boolean
}

/**
 * The command of a key press, as Desktop's formula editor and DAX query view have them:
 * Ctrl+Space suggestions, Ctrl+Shift+Space parameter info, Ctrl+K Ctrl+I the description at the
 * cursor (`chord` is true after Ctrl+K), Shift+Alt+F format.
 */
export function editorCommand(event: KeyLike, chord = false): EditorCommand | 'chord' | null {
  const mod = event.ctrlKey || event.metaKey
  const key = event.key.length === 1 ? event.key.toLowerCase() : event.key
  const code = event.code ?? ''
  if (mod && !event.altKey && (key === ' ' || code === 'Space')) return event.shiftKey ? 'parameterHints' : 'suggest'
  if (event.shiftKey && event.altKey && !mod && (key === 'f' || code === 'KeyF')) return 'format'
  if (mod && !event.shiftKey && !event.altKey) {
    if (chord && (key === 'i' || code === 'KeyI')) return 'hover'
    if (key === 'k' || code === 'KeyK') return 'chord'
  }
  return null
}
