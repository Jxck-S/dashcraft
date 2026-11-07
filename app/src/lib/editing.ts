// Pure helpers of the app's editing UI (formula bar, Properties pane, Quick Access Toolbar, File →
// Save / Save as): no backend calls, so they are unit-tested with `node --test`.

/**
 * The formula bar's text for an object: `Name = expression`, as Power BI Desktop shows it. A name
 * that would not parse back (contains `=`, starts with `[`, outer spaces) is bracketed.
 */
export function formulaText(name: string, expression: string): string {
  const shown = name.includes('=') || name.startsWith('[') || name.trim() !== name ? `[${name.replaceAll(']', ']]')}]` : name
  return `${shown} = ${expression.replace(/^\s*\n/, '')}`
}

export type ParsedFormula = { ok: true; name: string; expression: string } | { ok: false; error: string }

/**
 * Splits the formula bar's `Name = expression`. The name ends at the first `=`, or is bracketed
 * (`[Total = Sales] = …`, `]]` escapes `]`); the expression keeps its line breaks and indentation.
 */
export function parseFormula(text: string): ParsedFormula {
  const lead = text.length - text.trimStart().length
  let name: string
  let rest: string
  if (text.startsWith('[', lead)) {
    let i = lead + 1
    let close = -1
    while (i < text.length) {
      if (text[i] === ']') {
        if (text[i + 1] === ']') {
          i += 2
          continue
        }
        close = i
        break
      }
      i++
    }
    if (close < 0) return { ok: false, error: 'The name’s “[” has no closing “]”.' }
    name = text.slice(lead + 1, close).replaceAll(']]', ']')
    rest = text.slice(close + 1).trimStart()
    if (!rest.startsWith('=')) return { ok: false, error: 'Write the formula as Name = expression.' }
  } else {
    const eq = text.indexOf('=')
    if (eq < 0) return { ok: false, error: 'Write the formula as Name = expression.' }
    name = text.slice(0, eq).trim()
    rest = text.slice(eq)
  }
  if (!name.trim()) return { ok: false, error: 'The formula needs a name before “=”.' }
  if (/[\r\n]/.test(name)) return { ok: false, error: 'The name before “=” must be on one line.' }
  const expression = rest.slice(1).replace(/^[ \t]*(\r\n?|\n)?/, '').trimEnd()
  if (!expression.trim()) return { ok: false, error: 'The formula needs an expression after “=”.' }
  return { ok: true, name, expression }
}

/** The window title: the file name (`*` while there are unsaved edits) and the app name. */
export function windowTitle(fileName: string | null, dirty: boolean): string {
  return fileName ? `${dirty ? '*' : ''}${fileName} - DashCraft` : 'DashCraft'
}

/**
 * Whether File → Save writes the opened files in place: only a PBIP project (`doc.save`); any
 * other format is saved as a new PBIP project (`doc.saveAs`).
 */
export function canSaveInPlace(format: string | null | undefined): boolean {
  return format === 'pbip'
}

/** `doc.save`'s "use doc.saveAs" refusal: the document has to be saved as a new project. */
export function needsSaveAs(error: { kind: string; error: string }): boolean {
  return error.kind === 'unsupported' && error.error.includes('doc.saveAs')
}

/** `doc.saveAs`'s refusal of a non-empty folder (retried with `overwrite` after asking). */
export function folderNotEmpty(error: { kind: string; error: string }): boolean {
  return error.kind === 'io' && error.error.includes('is not empty')
}

/** Undo / redo availability as `doc.history` returns it. */
export interface HistoryState {
  dirty: boolean
  canUndo: boolean
  canRedo: boolean
  undoLabel?: string | null
  redoLabel?: string | null
}

/** Quick Access Toolbar tooltips: `Undo Change measure 'Total' (Ctrl+Z)`. */
export function historyTitles(history: HistoryState | null, mac = false): { undo: string; redo: string } {
  const mod = mac ? '⌘' : 'Ctrl+'
  const undo = history?.canUndo ? `Undo ${history.undoLabel ?? 'the last edit'}` : 'Nothing to undo'
  const redo = history?.canRedo ? `Redo ${history.redoLabel ?? 'the last undone edit'}` : 'Nothing to redo'
  return { undo: `${undo} (${mod}Z)`, redo: `${redo} (${mac ? '⌘⇧Z' : 'Ctrl+Y'})` }
}

/** A text field, text area or editable element: its own Ctrl+Z / Ctrl+Y win over the document's. */
export function isEditableTarget(target: EventTarget | null): boolean {
  if (!target || typeof target !== 'object') return false
  const el = target as { tagName?: string; isContentEditable?: boolean; type?: string; readOnly?: boolean }
  if (el.isContentEditable) return true
  const tag = el.tagName?.toUpperCase()
  if (tag === 'TEXTAREA') return !el.readOnly
  if (tag !== 'INPUT') return false
  const type = (el.type ?? 'text').toLowerCase()
  return !el.readOnly && !['button', 'checkbox', 'radio', 'range', 'color', 'file', 'submit', 'reset', 'image'].includes(type)
}

export type EditShortcut = 'save' | 'saveAs' | 'undo' | 'redo'

/** The document shortcut of a key press: Ctrl+S, Ctrl+Shift+S, Ctrl+Z, Ctrl+Y / Ctrl+Shift+Z. */
export function editShortcut(event: { key: string; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean; altKey: boolean; target: EventTarget | null }): EditShortcut | null {
  if (!(event.ctrlKey || event.metaKey) || event.altKey) return null
  const key = event.key.toLowerCase()
  if (key === 's') return event.shiftKey ? 'saveAs' : 'save'
  if (isEditableTarget(event.target)) return null
  if (key === 'z') return event.shiftKey ? 'redo' : 'undo'
  if (key === 'y' && !event.shiftKey) return 'redo'
  return null
}

/** Edited text of a Properties pane field: `undefined` when unchanged, `''` clears the value. */
export function changedValue(original: string | null | undefined, edited: string): string | undefined {
  const before = original ?? ''
  const after = edited.trim() === '' ? '' : edited
  return after === before ? undefined : after
}
