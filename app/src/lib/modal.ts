// Modal dialogs (File menu, Modal, Power Query Editor): focus moves in when one opens, Tab and
// Shift+Tab stay inside, and focus goes back to the control that opened it when it closes
// (WCAG 2.4.3 and 2.1.2, ADR-0015). A screen reader found the File menu leaving focus behind.

import { wrapFocus } from './focus'

const TABBABLE = [
  'a[href]',
  'button:not([disabled])',
  'input:not([disabled]):not([type="hidden"])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  'summary',
  '[tabindex]:not([tabindex="-1"])',
].join(', ')

/** Per open modal, where focus goes back to: its opener, then the openers of the modals it came from. */
const returnTo = new WeakMap<HTMLElement, HTMLElement[]>()
/** The chain of a modal closing in this update, for one that replaces it (the File menu's question). */
let closing: HTMLElement[] | null = null

/** Svelte action for an element with `role="dialog"`, `aria-modal="true"` and `tabindex="-1"`. */
export function modal(node: HTMLElement) {
  const active = document.activeElement
  const opener = active instanceof HTMLElement && active !== document.body ? active : null
  // File → Close closes the File menu as it asks about unsaved changes: Cancel goes back to File.
  const outer = opener?.closest<HTMLElement>('[aria-modal="true"]')
  const chain = [...(opener ? [opener] : []), ...((outer && returnTo.get(outer)) ?? closing ?? [])]
  returnTo.set(node, chain)
  if (!node.contains(document.activeElement)) node.focus()

  function keydown(event: KeyboardEvent) {
    // A text field that uses Tab itself (the DAX editor's indentation) has already handled it.
    if (event.key !== 'Tab' || event.defaultPrevented || event.ctrlKey || event.altKey || event.metaKey) return
    const items = [...node.querySelectorAll<HTMLElement>(TABBABLE)].filter((el) => el.getClientRects().length > 0 && !el.closest('[inert]'))
    const active = document.activeElement instanceof HTMLElement ? document.activeElement : null
    const next = wrapFocus(items, active, event.shiftKey)
    if (next || items.length === 0) event.preventDefault()
    next?.focus()
  }

  node.addEventListener('keydown', keydown)
  return {
    destroy() {
      node.removeEventListener('keydown', keydown)
      closing = chain
      // Once the dialog has left the page, unless something else took focus meanwhile.
      queueMicrotask(() => {
        closing = null
        const now = document.activeElement
        if (now && now !== document.body) return
        chain.find((el) => el.isConnected)?.focus()
      })
    },
  }
}
