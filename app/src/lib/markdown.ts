// A minimal, safe Markdown renderer for AI answers (explanations, assumptions). Model and provider
// text is untrusted: every character of it is HTML-escaped before it reaches the output, and the
// renderer only emits its own fixed tags without attributes taken from the input. Supported:
// headings, paragraphs, bullet and numbered lists (nested by indentation), block quotes, fenced
// code blocks, pipe tables, horizontal rules, inline code, bold, italic, strikethrough. Links and
// images are shown as plain text (`label (url)`, `[image: alt]`) and never become anchors or
// requests. Pure functions over strings (no DOM): tested by `tests/markdown.test.ts`.

/** Longest input rendered; the rest is cut (answers are capped at 4 MiB by the backend). */
export const MAX_MARKDOWN_CHARS = 200_000

const MAX_DEPTH = 6

/** How far a link label or link target may reach (keeps hostile input linear). */
const WINDOW = 500

const ENTITIES: Record<string, string> = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }

/** Escapes `& < > " '` so `text` is inert in HTML text and attribute positions. */
export function escapeHtml(text: string): string {
  return text.replace(/[&<>"']/g, (c) => ENTITIES[c] ?? c)
}

/** Renders `source` to HTML in which every piece of `source` is escaped text. */
export function renderMarkdown(source: string): string {
  let text = source.replace(/\r\n?/g, '\n')
  let cut = ''
  if (text.length > MAX_MARKDOWN_CHARS) {
    text = text.slice(0, MAX_MARKDOWN_CHARS)
    cut = `<p class="md-cut">${escapeHtml(`… (${(source.length - MAX_MARKDOWN_CHARS).toLocaleString('en-US')} more characters not shown)`)}</p>`
  }
  return blocks(text.split('\n'), 0) + cut
}

// ---------------------------------------------------------------------------------------------
// Blocks
// ---------------------------------------------------------------------------------------------

const FENCE = /^ {0,3}(`{3,}|~{3,})(.*)$/
const HEADING = /^ {0,3}(#{1,6})[ \t]+(.*?)(?:[ \t]+#+)?[ \t]*$/
const RULE = /^ {0,3}([-*_])([ \t]*\1){2,}[ \t]*$/
const BULLET = /^([ \t]*)([-*+])[ \t]+(.*)$/
const ORDERED = /^([ \t]*)(\d{1,9})[.)][ \t]+(.*)$/
const QUOTE = /^ {0,3}>[ \t]?(.*)$/
const TABLE_SEPARATOR = /^[ \t]*\|?[ \t]*:?-{3,}:?[ \t]*(\|[ \t]*:?-{3,}:?[ \t]*)*\|?[ \t]*$/

function isBlank(line: string): boolean {
  return line.trim() === ''
}

function startsBlock(line: string): boolean {
  return FENCE.test(line) || HEADING.test(line) || RULE.test(line) || BULLET.test(line) || ORDERED.test(line) || QUOTE.test(line)
}

function indentOf(spaces: string): number {
  return spaces.replace(/\t/g, '    ').length
}

function blocks(lines: string[], depth: number): string {
  let out = ''
  let i = 0
  while (i < lines.length) {
    const line = lines[i]
    if (isBlank(line)) {
      i += 1
      continue
    }
    const fence = FENCE.exec(line)
    if (fence) {
      const marker = fence[1]
      const body: string[] = []
      i += 1
      while (i < lines.length) {
        const close = /^ {0,3}(`{3,}|~{3,})[ \t]*$/.exec(lines[i])
        if (close && close[1][0] === marker[0] && close[1].length >= marker.length) {
          i += 1
          break
        }
        body.push(lines[i])
        i += 1
      }
      out += `<pre class="md-code"><code>${escapeHtml(body.join('\n'))}</code></pre>`
      continue
    }
    const heading = HEADING.exec(line)
    if (heading) {
      // Answers sit inside a panel: # → h3 … #### and deeper → h6.
      const level = Math.min(heading[1].length + 2, 6)
      out += `<h${level}>${inline(heading[2], depth)}</h${level}>`
      i += 1
      continue
    }
    if (RULE.test(line)) {
      out += '<hr>'
      i += 1
      continue
    }
    if (QUOTE.test(line)) {
      const body: string[] = []
      while (i < lines.length && !isBlank(lines[i])) {
        const q = QUOTE.exec(lines[i])
        body.push(q ? q[1] : lines[i])
        i += 1
      }
      out += depth < MAX_DEPTH ? `<blockquote>${blocks(body, depth + 1)}</blockquote>` : `<blockquote><p>${inline(body.join('\n'), depth)}</p></blockquote>`
      continue
    }
    if (BULLET.test(line) || ORDERED.test(line)) {
      const [html, next] = list(lines, i, depth)
      out += html
      i = next
      continue
    }
    if (line.includes('|') && i + 1 < lines.length && TABLE_SEPARATOR.test(lines[i + 1]) && lines[i + 1].includes('-')) {
      const [html, next] = table(lines, i, depth)
      out += html
      i = next
      continue
    }
    const para: string[] = []
    while (i < lines.length && !isBlank(lines[i]) && (para.length === 0 || !startsBlock(lines[i]))) {
      para.push(lines[i].trim())
      i += 1
    }
    out += `<p>${inline(para.join('\n'), depth)}</p>`
  }
  return out
}

interface Item {
  indent: number
  ordered: boolean
  text: string[]
  children: Item[]
}

/** A list starting at `lines[start]`, nested by indentation; returns the HTML and the next line. */
function list(lines: string[], start: number, depth: number): [string, number] {
  const roots: Item[] = []
  const stack: Item[] = []
  let i = start
  let blank = false
  while (i < lines.length) {
    const line = lines[i]
    if (isBlank(line)) {
      blank = true
      i += 1
      continue
    }
    const m = BULLET.exec(line) ?? ORDERED.exec(line)
    if (m) {
      const item: Item = { indent: indentOf(m[1]), ordered: ORDERED.test(line) && !BULLET.test(line), text: [m[3]], children: [] }
      while (stack.length > 0 && stack[stack.length - 1].indent >= item.indent) stack.pop()
      if (stack.length === 0 && roots.length > 0 && roots[0].ordered !== item.ordered) break
      const parent = stack.length > 0 && stack.length < MAX_DEPTH ? stack[stack.length - 1] : null
      if (parent) parent.children.push(item)
      else {
        stack.length = 0
        roots.push(item)
      }
      stack.push(item)
      blank = false
      i += 1
      continue
    }
    // A continuation line belongs to the last item if indented (or directly following it).
    const last = stack[stack.length - 1]
    const indented = /^[ \t]{2,}/.test(line)
    if (last && (indented || !blank) && !startsBlock(line)) {
      last.text.push(line.trim())
      blank = false
      i += 1
      continue
    }
    break
  }
  return [items(roots, depth), i]
}

function items(list: Item[], depth: number): string {
  let out = ''
  let open: boolean | null = null
  for (const item of list) {
    if (open !== item.ordered) {
      if (open !== null) out += open ? '</ol>' : '</ul>'
      out += item.ordered ? '<ol>' : '<ul>'
      open = item.ordered
    }
    out += `<li>${inline(item.text.join('\n'), depth)}${item.children.length > 0 ? items(item.children, depth) : ''}</li>`
  }
  if (open !== null) out += open ? '</ol>' : '</ul>'
  return out
}

function cellsOf(line: string): string[] {
  let row = line.trim()
  if (row.startsWith('|')) row = row.slice(1)
  if (row.endsWith('|') && !row.endsWith('\\|')) row = row.slice(0, -1)
  const cells: string[] = []
  let cell = ''
  for (let k = 0; k < row.length; k += 1) {
    if (row[k] === '\\' && row[k + 1] === '|') {
      cell += '|'
      k += 1
    } else if (row[k] === '|') {
      cells.push(cell.trim())
      cell = ''
    } else cell += row[k]
  }
  cells.push(cell.trim())
  return cells
}

/** A pipe table whose header is `lines[start]`; returns the HTML and the next line. */
function table(lines: string[], start: number, depth: number): [string, number] {
  const header = cellsOf(lines[start])
  let out = `<table class="md-table"><thead><tr>${header.map((c) => `<th>${inline(c, depth)}</th>`).join('')}</tr></thead><tbody>`
  let i = start + 2
  while (i < lines.length && !isBlank(lines[i]) && lines[i].includes('|')) {
    const row = cellsOf(lines[i])
    out += `<tr>${header.map((_, k) => `<td>${inline(row[k] ?? '', depth)}</td>`).join('')}</tr>`
    i += 1
  }
  return [`${out}</tbody></table>`, i]
}

// ---------------------------------------------------------------------------------------------
// Inline
// ---------------------------------------------------------------------------------------------

const PUNCTUATION = '\\`*_{}[]()#+-.!|~<>"\''

function isWordChar(c: string | undefined): boolean {
  return c !== undefined && /[\p{L}\p{N}]/u.test(c)
}

/** The index of the `]` closing the `[` at `open` (nested brackets balanced), or -1. */
function closingBracket(text: string, open: number): number {
  let level = 0
  const end = Math.min(text.length, open + WINDOW)
  for (let k = open; k < end; k += 1) {
    const c = text[k]
    if (c === '\\') k += 1
    else if (c === '[') level += 1
    else if (c === ']') {
      level -= 1
      if (level === 0) return k
    } else if (c === '\n' && text[k + 1] === '\n') return -1
  }
  return -1
}

/** The index of the `)` closing the `(` at `open`, or -1. */
function closingParen(text: string, open: number): number {
  let level = 0
  const end = Math.min(text.length, open + WINDOW)
  for (let k = open; k < end; k += 1) {
    const c = text[k]
    if (c === '(') level += 1
    else if (c === ')') {
      level -= 1
      if (level === 0) return k
    } else if (c === '\n') return -1
  }
  return -1
}

/**
 * The closing `delim` of an emphasis opened before `from` (not after whitespace), or -1. Whether a
 * candidate closes does not depend on `from`, so once there is none after `from` there is none
 * after any later position either (`inline` remembers that, which keeps it linear).
 */
function closingDelimiter(text: string, from: number, delim: string): number {
  let k = from
  while (k < text.length) {
    const at = text.indexOf(delim, k)
    if (at < 0) return -1
    const before = text[at - 1]
    const after = text[at + delim.length]
    const escaped = before === '\\'
    const intraword = delim[0] === '_' && isWordChar(after)
    // `**` must not close a single `*` (and vice versa).
    const longer = delim.length === 1 && (after === delim || before === delim)
    if (at > from && !escaped && !intraword && !longer && before !== undefined && !/\s/.test(before)) return at
    k = at + delim.length
  }
  return -1
}

/** Inline Markdown in `text` → HTML; all of `text` is escaped. */
function inline(text: string, depth: number): string {
  let out = ''
  let i = 0
  // Per delimiter: the first position from which no closing delimiter exists.
  const unclosed = new Map<string, number>()
  while (i < text.length) {
    const c = text[i]
    if (c === '\\' && i + 1 < text.length && PUNCTUATION.includes(text[i + 1])) {
      out += escapeHtml(text[i + 1])
      i += 2
      continue
    }
    if (c === '\n') {
      out += '\n'
      i += 1
      continue
    }
    if (c === '`') {
      let n = 1
      while (text[i + n] === '`') n += 1
      const fence = '`'.repeat(n)
      let close = text.indexOf(fence, i + n)
      while (close >= 0 && text[close + n] === '`') close = text.indexOf(fence, close + n + 1)
      if (close >= 0) {
        out += `<code>${escapeHtml(text.slice(i + n, close).replace(/^ (.+) $/s, '$1'))}</code>`
        i = close + n
      } else {
        out += escapeHtml(fence)
        i += n
      }
      continue
    }
    if ((c === '[' || (c === '!' && text[i + 1] === '[')) && depth < MAX_DEPTH) {
      const image = c === '!'
      const open = image ? i + 1 : i
      const close = closingBracket(text, open)
      if (close > open && text[close + 1] === '(') {
        const end = closingParen(text, close + 1)
        if (end > close) {
          const label = text.slice(open + 1, close)
          const target = text.slice(close + 2, end).trim().split(/\s+/)[0] ?? ''
          if (image) out += `<span class="md-image">${escapeHtml(`[image: ${label || target}]`)}</span>`
          else if (target === '' || target === label) out += `<span class="md-link">${inline(label, depth + 1)}</span>`
          else out += `<span class="md-link">${inline(label, depth + 1)}</span> <span class="md-url">(${escapeHtml(target)})</span>`
          i = end + 1
          continue
        }
      }
    }
    if (c === '<') {
      const auto = /^<((?:https?|mailto):[^\s<>]+)>/.exec(text.slice(i, i + 2048))
      if (auto) {
        out += `<span class="md-url">${escapeHtml(auto[1])}</span>`
        i += auto[0].length
        continue
      }
    }
    if ((c === '*' || c === '_' || c === '~') && depth < MAX_DEPTH) {
      const double = text[i + 1] === c
      const delim = double ? c + c : c
      const prev = text[i - 1]
      const next = text[i + delim.length]
      const opens = next !== undefined && !/\s/.test(next) && !(c === '_' && isWordChar(prev)) && (c !== '~' || double)
      const from = i + delim.length
      if (opens && from < (unclosed.get(delim) ?? Infinity)) {
        const close = closingDelimiter(text, from, delim)
        if (close < 0) unclosed.set(delim, from)
        if (close > from) {
          const tag = c === '~' ? 'del' : double ? 'strong' : 'em'
          out += `<${tag}>${inline(text.slice(from, close), depth + 1)}</${tag}>`
          i = close + delim.length
          continue
        }
      }
      out += escapeHtml(delim)
      i += delim.length
      continue
    }
    out += escapeHtml(c)
    i += 1
  }
  return out
}
