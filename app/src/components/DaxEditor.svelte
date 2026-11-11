<script lang="ts" module>
  /** Ctrl+M's "Tab moves focus", shared by the editors of the window. */
  const tabFocus = { on: false }
</script>

<script lang="ts">
  import { onMount, tick, untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import Markdown from './ai/Markdown.svelte'
  import type { IconName } from '../lib/icons'
  import type { DaxCompletionItemOut, DaxCompletionKind, DaxSignatureOut } from '../lib/ops.gen'
  import {
    completionEdit,
    copyLines,
    deleteLines,
    editorCommand,
    expandSelection,
    indentLines,
    lineAbove,
    lineBreak,
    lineColumn,
    lineText,
    matchingBracket,
    moveActive,
    moveLines,
    offsetAt,
    outdentLines,
    segments,
    selectedLines,
    selectLine,
    shiftOffset,
    shiftSpans,
    signatureParts,
    tabEdit,
    textDelta,
    TOKEN_COLORS,
    triggers,
    withFormatted,
    type DaxService,
    type EditorCommand,
    type MarkSpan,
    type Selection,
    type TextEdit,
    type TokenSpan,
  } from '../lib/daxEditor'

  // A DAX editor with IntelliSense (M7.5): the text is coloured and problems are squiggled from
  // `dax.tokens` and `dax.diagnostics`; typing a name, `[` or `'` (or Ctrl+Space) lists
  // `dax.complete`'s suggestions (↑ ↓ choose, Enter or Tab insert, Esc close); `(` and `,` (or
  // Ctrl+Shift+Space) show the function's parameters; pointing at a name (or Ctrl+K Ctrl+I) shows
  // its description and the problems there; Shift+Alt+F formats with `dax.format`. Desktop's
  // line keys (`editorCommand`) edit through the editor's own undo; Tab indents until Ctrl+M
  // makes it move focus. Only the part of the text from `start(text)` on is DAX (the formula
  // bar's `Name =` comes before it). Other keys, and Shift+Enter after its line break, go to
  // `onkeydown`.
  let {
    value = $bindable(),
    rows,
    label,
    service,
    start = () => 0,
    readonly = false,
    invalid = false,
    describedby = undefined,
    onkeydown = undefined,
    onformat = undefined,
  }: {
    value: string
    rows: number
    label: string
    service: DaxService | null
    /** Where the DAX starts in `value`, `null` when it cannot be found. */
    start?: (text: string) => number | null
    readonly?: boolean
    invalid?: boolean
    describedby?: string
    onkeydown?: (event: KeyboardEvent) => void
    /** After Shift+Alt+F changed the text. */
    onformat?: () => void
  } = $props()

  type Problem = MarkSpan & { message: string }
  type Completion = { start: number; end: number; items: DaxCompletionItemOut[] }
  type Tip = { x: number; y: number; at: number; problems: string[]; markdown: string | null }

  const LINE = 18
  const uid = Math.random().toString(36).slice(2)
  const listId = `dax-suggest-${uid}`
  const signatureId = `dax-signature-${uid}`
  const problemId = `dax-problem-${uid}`
  const statusId = `dax-status-${uid}`
  const keysId = `dax-keys-${uid}`
  const tipId = `dax-tip-${uid}`

  let editor = $state<HTMLTextAreaElement | null>(null)
  let gutter = $state<HTMLDivElement | null>(null)
  let layer = $state<HTMLDivElement | null>(null)
  let probe = $state<HTMLSpanElement | null>(null)
  let tipBox = $state<HTMLDivElement | null>(null)
  let charWidth = 7.2
  let tokens = $state.raw<TokenSpan[]>([])
  let problems = $state.raw<Problem[]>([])
  let completion = $state.raw<Completion | null>(null)
  let active = $state(0)
  let signature = $state.raw<{ sig: DaxSignatureOut; x: number; y: number } | null>(null)
  let tip = $state.raw<Tip | null>(null)
  let caret = $state(0)
  let status = $state('')
  let chord = false
  // An edit of our own (an accepted suggestion, formatting) is not typing: it opens no list.
  let applying = false
  // The text the token and problem ranges are aligned with.
  let aligned = untrack(() => value)
  let analysisTimer: ReturnType<typeof setTimeout> | undefined
  let hoverTimer: ReturnType<typeof setTimeout> | undefined
  let analysisSeq = 0
  // Ctrl+M, for every editor of the window as in Desktop.
  let tabMovesFocus = $state(tabFocus.on)
  // Shift+Alt+→ results, for Shift+Alt+← to go back.
  let expansions: Selection[] = []
  // Ctrl+C or Ctrl+X without a selection: the line, its deletion and the cursor to go back to.
  let lineClip: { text: string; edit: TextEdit; caret: Selection } | null = null

  const lines = $derived(value.split('\n'))
  const segs = $derived(segments(value, tokens, problems))
  const activeItem = $derived(completion?.items[active] ?? null)
  const sigParts = $derived(signature ? signatureParts(signature.sig) : null)
  /** The problem at the caret, else the first error: what a screen reader hears with the field. */
  const problem = $derived(problems.find((p) => p.start <= caret && caret <= p.end) ?? problems.find((p) => p.severity === 'error') ?? null)
  const problemText = $derived.by(() => {
    if (!problem) return ''
    const { line, column } = lineColumn(value, problem.start, 1)
    const more = problems.length > 1 ? ` (${problems.length} problems)` : ''
    return `${problem.severity === 'error' ? 'Error' : 'Warning'} at line ${line + 1}, column ${column + 1}: ${problem.message}${more}`
  })
  const keysText = $derived(readonly ? '' : tabMovesFocus ? 'Tab moves focus; Ctrl+M makes Tab indent.' : 'Tab indents; Ctrl+M makes Tab move focus.')
  const describedBy = $derived([describedby, problemText ? problemId : null, signature ? signatureId : null, tip ? tipId : null, keysText ? keysId : null].filter(Boolean).join(' ') || undefined)

  const KIND_ICONS: Record<DaxCompletionKind, IconName> = {
    function: 'code',
    userFunction: 'code',
    keyword: 'code',
    table: 'table',
    column: 'column',
    measure: 'measure',
    variable: 'parameter',
  }

  /** Lets the parent move focus here (after a failed commit). */
  export function focus() {
    editor?.focus()
  }

  onMount(() => {
    measure()
    void document.fonts?.ready.then(measure)
    analyze()
    return () => {
      clearTimeout(analysisTimer)
      clearTimeout(hoverTimer)
    }
  })

  // An outside change of the text (cancel, a new object): realign and analyse again. It reads
  // `value`, the reactive one: the textarea's own value would not re-run the effect, and the
  // problems of a cancelled draft would stay announced. Typing is not an outside change:
  // `input` runs in the capture phase, before `bind:value` updates `value`, and realigns first.
  $effect(() => {
    const text = value
    untrack(() => {
      if (text !== aligned) {
        realign(text)
        closePopups()
        scheduleAnalysis()
      }
    })
  })

  // Keep the active suggestion visible.
  $effect(() => {
    if (completion) document.getElementById(`${listId}-${active}`)?.scrollIntoView({ block: 'nearest' })
  })

  function measure() {
    if (probe) charWidth = probe.getBoundingClientRect().width / 10 || charWidth
  }

  /**
   * The text as the textarea has it now: `value` follows it only once `bind:value` has run,
   * which may be after this component's own input handler.
   */
  function current(): string {
    return editor?.value ?? value
  }

  function exprStart(text: string): number | null {
    return start(text)
  }

  /** Moves the ranges from the text they were computed for to `text`. */
  function realign(text: string) {
    const delta = textDelta(aligned, text)
    tokens = shiftSpans(tokens, delta)
    problems = shiftSpans(problems, delta)
    if (completion) {
      const s = shiftOffset(completion.start, delta, 'left')
      const e = shiftOffset(completion.end, delta, 'right')
      completion = { ...completion, start: s, end: Math.max(s, e) }
    }
    aligned = text
  }

  function scheduleAnalysis(delay = 150) {
    clearTimeout(analysisTimer)
    analysisTimer = setTimeout(analyze, delay)
  }

  /** Colours and problems of the current text from `dax.tokens` and `dax.diagnostics`. */
  async function analyze() {
    const text = current()
    const at = exprStart(text)
    const seq = ++analysisSeq
    if (!service || at == null) {
      tokens = []
      problems = []
      return
    }
    const expr = text.slice(at)
    const [found, checked] = await Promise.all([service.tokens(expr), service.diagnostics(expr)])
    if (seq !== analysisSeq) return
    const delta = textDelta(text, aligned)
    if (found) tokens = shiftSpans(found.map((t) => ({ start: t.start.utf16Offset + at, end: t.end.utf16Offset + at, kind: t.kind })), delta)
    if (checked) {
      const list: Problem[] = []
      for (const d of checked) {
        if (d.severity !== 'error' && d.severity !== 'warning') continue
        const message = d.suggestion ? `${d.message}; ${d.suggestion}` : d.message
        list.push({ start: d.start.utf16Offset + at, end: d.end.utf16Offset + at, severity: d.severity, message })
      }
      problems = shiftSpans(list, delta)
    }
  }

  function input() {
    if (!editor) return
    const text = editor.value
    const delta = textDelta(aligned, text)
    realign(text)
    caret = editor.selectionStart
    tip = null
    scheduleAnalysis()
    if (applying) return
    const t = triggers(text, delta, completion !== null)
    if (t.complete === 'open') void requestCompletion(false)
    else if (t.complete === 'close') completion = null
    if (t.signature || signature) void requestSignature()
  }

  /** The position of `offset` on screen: the left and top of its line. */
  function pointOf(offset: number): { x: number; y: number } {
    if (!editor) return { x: 0, y: 0 }
    const { line, column } = lineColumn(current(), offset)
    const r = editor.getBoundingClientRect()
    const cs = getComputedStyle(editor)
    return {
      x: r.left + parseFloat(cs.paddingLeft) + column * charWidth - editor.scrollLeft,
      y: r.top + parseFloat(cs.paddingTop) + line * LINE - editor.scrollTop,
    }
  }

  async function requestCompletion(explicit: boolean) {
    if (!service || !editor) return
    const text = current()
    const at = exprStart(text)
    const offset = editor.selectionStart
    if (at == null || offset < at || editor.selectionEnd !== offset) {
      completion = null
      return
    }
    const out = await service.complete(text.slice(at), offset - at)
    if (!out || current() !== text) return
    if (out.items.length === 0) {
      completion = null
      if (explicit) status = 'No suggestions'
      return
    }
    const opening = completion === null
    completion = { start: out.start.utf16Offset + at, end: out.end.utf16Offset + at, items: out.items }
    active = 0
    if (opening) status = `${out.items.length}${out.incomplete ? '+' : ''} suggestions: Up and Down to choose, Enter or Tab to insert`
  }

  async function requestSignature() {
    if (!service || !editor) return
    const text = current()
    const at = exprStart(text)
    const offset = editor.selectionStart
    if (at == null || offset < at) {
      signature = null
      return
    }
    const sig = await service.signatureHelp(text.slice(at), offset - at)
    if (current() !== text) return
    if (!sig) {
      signature = null
      return
    }
    const p = pointOf(offset)
    signature = { sig, x: p.x, y: p.y }
  }

  async function showTip(offset: number, point: { x: number; y: number } | null) {
    const text = current()
    const at = exprStart(text)
    const here = problems.filter((p) => p.start <= offset && offset <= Math.max(p.end, p.start + 1)).map((p) => p.message)
    let markdown: string | null = null
    if (service && at != null && offset >= at) {
      const info = await service.hover(text.slice(at), offset - at)
      if (current() !== text) return
      markdown = info?.markdown ?? null
    }
    if (!markdown && here.length === 0) {
      tip = null
      if (!point) status = 'Nothing to describe here'
      return
    }
    const p = point ?? pointOf(offset)
    tip = { x: p.x, y: p.y + LINE, at: offset, problems: here, markdown }
    // Asked for from the keyboard (Ctrl+K Ctrl+I): a screen reader hears the description too.
    if (!point) {
      await tick()
      const said = tipBox?.innerText.replace(/\s+/g, ' ').trim()
      if (said) status = said
    }
  }

  /** Replaces `[from, to)` as typing would (the editor's own undo keeps it). */
  function replace(from: number, to: number, text: string) {
    if (!editor) return
    editor.focus()
    editor.setSelectionRange(from, to)
    applying = true
    try {
      if (!document.execCommand('insertText', false, text)) {
        editor.setRangeText(text, from, to, 'end')
        editor.dispatchEvent(new Event('input', { bubbles: true }))
      }
    } finally {
      applying = false
    }
  }

  function select(sel: Selection) {
    if (!editor) return
    editor.setSelectionRange(sel.start, sel.end)
    caret = sel.start
  }

  /** Applies a line edit as typing would and selects what it says. */
  function applyEdit(edit: TextEdit | null) {
    if (!edit || !editor) return
    closePopups()
    expansions = []
    if (edit.text === '' && edit.from < edit.to) {
      editor.focus()
      editor.setSelectionRange(edit.from, edit.to)
      applying = true
      try {
        if (!document.execCommand('delete')) {
          editor.setRangeText('', edit.from, edit.to, 'end')
          editor.dispatchEvent(new Event('input', { bubbles: true }))
        }
      } finally {
        applying = false
      }
    } else if (edit.from !== edit.to || edit.text) replace(edit.from, edit.to, edit.text)
    select(edit.selection)
  }

  function selection(): Selection {
    return { start: editor?.selectionStart ?? 0, end: editor?.selectionEnd ?? 0 }
  }

  function accept(index: number) {
    const c = completion
    const item = c?.items[index]
    if (!c || !item || !editor) return
    const edit = completionEdit(current(), c.start, c.end, item.insertText)
    completion = null
    replace(edit.from, edit.to, edit.text)
    editor.setSelectionRange(edit.caret, edit.caret)
    caret = edit.caret
    status = `Inserted ${item.label}`
    if (item.insertText.endsWith('(')) void requestSignature()
  }

  async function format() {
    if (!service || !editor || readonly) return
    const text = current()
    const at = exprStart(text)
    if (at == null) {
      status = 'Write the formula as Name = expression to format it.'
      return
    }
    const out = await service.format(text.slice(at))
    if (current() !== text) return
    if (!out.ok) {
      status = `Not formatted: ${out.error}`
      return
    }
    const next = withFormatted(text, at, out.text)
    if (next === text) {
      status = 'Already formatted'
      return
    }
    replace(0, text.length, next)
    editor.setSelectionRange(0, 0)
    editor.scrollTop = 0
    status = 'Formatted'
    onformat?.()
  }

  /** Runs a command; false when it does not apply here (the key then does what it would). */
  function run(command: EditorCommand): boolean {
    if (!editor) return false
    const text = current()
    const sel = selection()
    const editing = !readonly
    switch (command) {
      case 'suggest':
        if (editing) void requestCompletion(true)
        return true
      case 'parameterHints':
        if (editing) void requestSignature()
        return true
      case 'hover':
        void showTip(sel.start, null)
        return true
      case 'format':
        if (editing) void format()
        return true
      case 'toggleTabFocus':
        tabMovesFocus = tabFocus.on = !tabMovesFocus
        status = tabMovesFocus ? 'Tab now moves focus' : 'Tab now inserts indentation'
        return true
      case 'tab':
      case 'shiftTab':
        if (!editing || tabMovesFocus) return false
        applyEdit(command === 'tab' ? tabEdit(text, sel) : outdentLines(text, sel))
        return true
      case 'lineBreak':
      case 'lineAbove':
      case 'indentLines':
      case 'outdentLines':
      case 'deleteLines':
      case 'moveLinesUp':
      case 'moveLinesDown':
      case 'copyLinesUp':
      case 'copyLinesDown':
        if (editing) applyEdit(lineEdit(command, text, sel))
        return true
      case 'selectLine':
        select(selectLine(text, sel))
        return true
      case 'jumpToBracket': {
        const to = matchingBracket(text, sel.start)
        if (to !== null) select({ start: to, end: to })
        return true
      }
      case 'expandSelection': {
        const last = expansions[expansions.length - 1]
        if (!last || last.start !== sel.start || last.end !== sel.end) expansions = [sel]
        const wider = expandSelection(text, sel)
        if (wider) {
          expansions = [...expansions, wider]
          select(wider)
        }
        return true
      }
      case 'shrinkSelection': {
        const last = expansions[expansions.length - 1]
        if (last && last.start === sel.start && last.end === sel.end && expansions.length > 1) {
          expansions = expansions.slice(0, -1)
          select(expansions[expansions.length - 1])
        }
        return true
      }
      case 'scrollLineUp':
      case 'scrollLineDown':
      case 'scrollPageUp':
      case 'scrollPageDown': {
        const page = Math.max(LINE, editor.clientHeight - LINE)
        const by = command === 'scrollLineUp' ? -LINE : command === 'scrollLineDown' ? LINE : command === 'scrollPageUp' ? -page : page
        editor.scrollTop += by
        return true
      }
    }
  }

  function lineEdit(command: EditorCommand, text: string, sel: Selection): TextEdit | null {
    switch (command) {
      case 'lineBreak':
        return lineBreak(text, sel)
      case 'lineAbove':
        return lineAbove(text, sel)
      case 'indentLines':
        return indentLines(text, sel)
      case 'outdentLines':
        return outdentLines(text, sel)
      case 'deleteLines':
        return text ? deleteLines(text, sel) : null
      case 'moveLinesUp':
        return moveLines(text, sel, -1)
      case 'moveLinesDown':
        return moveLines(text, sel, 1)
      case 'copyLinesUp':
        return copyLines(text, sel, -1)
      case 'copyLinesDown':
        return copyLines(text, sel, 1)
      default:
        return null
    }
  }

  /** Ctrl+C and Ctrl+X without a selection take the whole line, as in Desktop. */
  function lineClipboard(event: KeyboardEvent) {
    lineClip = null
    if (!editor || !(event.ctrlKey || event.metaKey) || event.shiftKey || event.altKey) return
    const key = event.key.toLowerCase()
    const cut = key === 'x' || event.code === 'KeyX'
    if (!(cut || key === 'c' || event.code === 'KeyC') || (cut && readonly)) return
    const text = current()
    const sel = selection()
    if (sel.start !== sel.end || !text) return
    const line = lineText(text, sel)
    const lines = selectedLines(text, sel)
    lineClip = { ...line, caret: sel }
    // The default copy or cut then runs on this selection; `clip` writes the line instead.
    editor.setSelectionRange(lines.start, Math.min(text.length, lines.end + 1))
  }

  function clip(event: ClipboardEvent) {
    const pending = lineClip
    lineClip = null
    if (!pending || !event.clipboardData) return
    event.preventDefault()
    event.clipboardData.setData('text/plain', pending.text)
    if (event.type === 'cut') setTimeout(() => applyEdit(pending.edit))
    else select(pending.caret)
  }

  function closePopups() {
    completion = null
    signature = null
    tip = null
  }

  function keydown(event: KeyboardEvent) {
    if (completion) {
      if (['ArrowDown', 'ArrowUp', 'PageDown', 'PageUp'].includes(event.key) && !event.altKey && !event.ctrlKey && !event.metaKey) {
        event.preventDefault()
        active = moveActive(active, completion.items.length, event.key)
        return
      }
      if ((event.key === 'Enter' || event.key === 'Tab') && !event.shiftKey && !event.altKey && !event.ctrlKey && !event.metaKey) {
        event.preventDefault()
        accept(active)
        return
      }
      if (event.key === 'Escape') {
        event.preventDefault()
        event.stopPropagation()
        completion = null
        return
      }
    }
    lineClipboard(event)
    const command = editorCommand(event, chord)
    chord = command === 'chord'
    if (command === 'chord') {
      event.preventDefault()
      return
    }
    if (command && run(command)) {
      event.preventDefault()
      // The formula bar grows when a line is added.
      if (command === 'lineBreak' || command === 'lineAbove') onkeydown?.(event)
      return
    }
    if (event.key === 'Escape' && (signature || tip)) {
      event.preventDefault()
      event.stopPropagation()
      signature = null
      tip = null
      return
    }
    onkeydown?.(event)
  }

  /** After the caret moved: close what no longer applies, follow the parameters. */
  function caretMoved() {
    if (!editor) return
    caret = editor.selectionStart
    if (completion && (caret < completion.start || caret > completion.end)) completion = null
    if (tip && tip.at !== caret) tip = null
    if (signature) void requestSignature()
  }

  function keyup(event: KeyboardEvent) {
    if (['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End', 'PageUp', 'PageDown'].includes(event.key) && !completion) caretMoved()
    else if (editor) caret = editor.selectionStart
  }

  function scrolled() {
    if (!editor) return
    if (gutter) gutter.scrollTop = editor.scrollTop
    // Moved rather than scrolled: the textarea's scroll range includes its scrollbars.
    if (layer) layer.style.transform = `translate(${-editor.scrollLeft}px, ${-editor.scrollTop}px)`
    closePopups()
  }

  function pointerMove(event: MouseEvent) {
    clearTimeout(hoverTimer)
    if (!editor || !service) return
    const r = editor.getBoundingClientRect()
    const cs = getComputedStyle(editor)
    const line = Math.floor((event.clientY - r.top - parseFloat(cs.paddingTop) + editor.scrollTop) / LINE)
    const column = Math.floor((event.clientX - r.left - parseFloat(cs.paddingLeft) + editor.scrollLeft) / charWidth)
    if (line < 0 || line >= lines.length || column < 0) return
    const text = current()
    const offset = offsetAt(text, line, column)
    if (lineColumn(text, offset).line !== line || offset >= text.length || text[offset] === '\n') {
      if (tip && tip.at !== caret) tip = null
      return
    }
    if (tip && tip.at === offset) return
    const point = { x: event.clientX, y: event.clientY - LINE / 2 }
    hoverTimer = setTimeout(() => void showTip(offset, point), 450)
  }

  function pointerLeave() {
    clearTimeout(hoverTimer)
    if (tip && editor && tip.at !== editor.selectionStart) tip = null
  }

  function listPointer(event: MouseEvent) {
    event.preventDefault()
    const option = (event.target as HTMLElement | null)?.closest('[data-index]')
    if (option) accept(Number(option.getAttribute('data-index')))
  }

  function blur() {
    chord = false
    closePopups()
  }
</script>

<div class="editor" class:invalid>
  <div class="gutter" bind:this={gutter} aria-hidden="true" style:height={`${rows * LINE + 8}px`}>{#each lines as _, i (i)}<span>{i + 1}</span>{/each}</div>
  <div class="code">
    <div class="highlight" aria-hidden="true"><div class="layer" bind:this={layer}>{#each segs as s, i (i)}<span class:error={s.severity === 'error'} class:warning={s.severity === 'warning'} style:color={s.token ? TOKEN_COLORS[s.token] : null}>{s.text}</span>{/each}{'\n'}<span class="probe" bind:this={probe}>0000000000</span></div></div>
    <textarea
      bind:this={editor}
      bind:value
      {rows}
      wrap="off"
      spellcheck="false"
      autocomplete="off"
      autocapitalize="off"
      {readonly}
      aria-label={label}
      aria-invalid={invalid || problems.some((p) => p.severity === 'error')}
      aria-describedby={describedBy}
      aria-autocomplete="list"
      aria-controls={completion ? listId : undefined}
      aria-activedescendant={completion ? `${listId}-${active}` : undefined}
      oninputcapture={input}
      onkeydown={keydown}
      onkeyup={keyup}
      onclick={caretMoved}
      onscroll={scrolled}
      onmousemove={pointerMove}
      onmouseleave={pointerLeave}
      onblur={blur}
      oncopy={clip}
      oncut={clip}
    ></textarea>
  </div>
</div>
<span class="visually-hidden" id={problemId}>{problemText}</span>
<span class="visually-hidden" id={statusId} role="status">{status}</span>
<span class="visually-hidden" id={keysId}>{keysText}</span>

{#if completion}
  {@const p = pointOf(completion.start)}
  <div class="suggest" style:left={`${Math.max(4, p.x - 26)}px`} style:top={`${p.y + LINE + 2}px`}>
    <ul id={listId} role="listbox" aria-label="Suggestions" tabindex="-1" onmousedown={listPointer}>
      {#each completion.items as item, i (i)}
        <li id={`${listId}-${i}`} role="option" aria-selected={i === active} class:active={i === active} data-index={i}>
          <Icon name={KIND_ICONS[item.kind]} size={14} />
          <span class="label">{item.label}</span>
          <span class="detail">{item.kind === 'function' || item.kind === 'userFunction' || item.kind === 'keyword' ? item.kind : (item.table ?? item.detail)}</span>
        </li>
      {/each}
    </ul>
    {#if activeItem && (activeItem.description || activeItem.kind === 'function' || activeItem.kind === 'userFunction')}
      <div class="doc" aria-hidden="true">
        <div class="doc-detail">{activeItem.detail}</div>
        {#if activeItem.description}<div>{activeItem.description}</div>{/if}
      </div>
    {/if}
  </div>
{/if}

{#if signature && sigParts}
  <div class="signature" id={signatureId} role="tooltip" style:left={`${Math.max(4, signature.x - 26)}px`} style:top={`${signature.y - 4}px`}>
    <div class="sig-label">{sigParts[0]}<strong>{sigParts[1]}</strong>{sigParts[2]}</div>
    {#if signature.sig.description}<div class="sig-doc">{signature.sig.description}</div>{/if}
    {#if !signature.sig.supported}<div class="sig-doc">DashCraft cannot evaluate {signature.sig.function} yet.</div>{/if}
  </div>
{/if}

{#if tip}
  <div class="tip" id={tipId} role="tooltip" bind:this={tipBox} style:left={`${Math.max(4, tip.x)}px`} style:top={`${tip.y + 2}px`}>
    {#each tip.problems as message, i (i)}<div class="tip-problem"><Icon name="error" size={14} />{message}</div>{/each}
    {#if tip.markdown}<Markdown text={tip.markdown} />{/if}
  </div>
{/if}

<style>
  .editor {
    display: flex;
    align-items: flex-start;
    min-width: 0;
    border: 1px solid var(--color-border);
    border-bottom-color: var(--color-border-accessible);
    border-radius: 3px;
    background: var(--color-code-bg);
    font-family: var(--font-mono);
    font-size: 12px;
    line-height: 18px;
  }

  .editor:focus-within {
    border-color: var(--color-brand);
    outline: 2px solid var(--color-brand);
  }

  .editor.invalid {
    border-color: var(--color-danger);
  }

  .gutter {
    box-sizing: border-box;
    flex: none;
    padding: 4px 0 4px 8px;
    overflow: hidden;
  }

  .gutter span {
    display: block;
    width: 2.5ch;
    color: var(--color-text-muted);
    text-align: right;
    user-select: none;
    -webkit-user-select: none;
  }

  .code {
    position: relative;
    flex: 1;
    min-width: 0;
  }

  .highlight,
  textarea {
    box-sizing: border-box;
    margin: 0;
    padding: 4px 8px;
    border: none;
    font: inherit;
    line-height: inherit;
    letter-spacing: normal;
    white-space: pre;
    tab-size: 4;
  }

  .highlight {
    position: absolute;
    inset: 0;
    overflow: hidden;
    color: var(--color-text);
    pointer-events: none;
  }

  .layer {
    will-change: transform;
  }

  .highlight .error {
    text-decoration: underline wavy var(--color-danger);
    text-decoration-skip-ink: none;
    text-underline-offset: 3px;
  }

  .highlight .warning {
    text-decoration: underline wavy #8a5a00;
    text-decoration-skip-ink: none;
    text-underline-offset: 3px;
  }

  .probe {
    position: absolute;
    visibility: hidden;
  }

  textarea {
    position: relative;
    display: block;
    width: 100%;
    outline: none;
    background: transparent;
    color: transparent;
    caret-color: var(--color-text);
    resize: none;
  }

  textarea::selection {
    background: rgba(17, 120, 101, 0.25);
  }

  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  .suggest,
  .signature,
  .tip {
    position: fixed;
    z-index: 50;
    border: 1px solid var(--color-border-strong);
    border-radius: 4px;
    background: var(--color-pane);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.14);
    color: var(--color-text);
    font-size: 12px;
  }

  .suggest {
    display: flex;
    align-items: flex-start;
    max-width: min(760px, calc(100vw - 16px));
  }

  ul {
    width: 340px;
    max-height: 216px;
    margin: 0;
    padding: 2px 0;
    overflow-y: auto;
    outline: none;
    list-style: none;
  }

  li {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px 8px;
    line-height: 18px;
    cursor: pointer;
  }

  li.active {
    background: var(--color-brand-soft);
    outline: 1px solid var(--color-brand);
    outline-offset: -1px;
  }

  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    font-family: var(--font-mono);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .detail {
    flex: none;
    max-width: 40%;
    overflow: hidden;
    color: var(--color-text-muted);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .doc {
    width: 300px;
    padding: 6px 8px;
    border-left: 1px solid var(--color-border);
  }

  .doc-detail,
  .sig-label {
    margin-bottom: 2px;
    font-family: var(--font-mono);
  }

  .signature {
    max-width: min(640px, calc(100vw - 16px));
    padding: 4px 8px;
    transform: translateY(-100%);
  }

  .sig-doc {
    color: var(--color-text-muted);
  }

  .tip {
    max-width: min(520px, calc(100vw - 16px));
    max-height: 260px;
    padding: 4px 10px;
    overflow: auto;
  }

  .tip-problem {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 4px 0;
    color: var(--color-danger);
  }
</style>
