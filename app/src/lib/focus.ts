// Focus rules shared by the modal dialogs (src/lib/modal.ts). DOM-free, so `npm run test` covers it.

/** Where Tab (Shift+Tab with `back`) goes from `from` in a modal: `null` when the browser's own move stays inside. */
export function wrapFocus<T>(items: readonly T[], from: T | null, back: boolean): T | null {
  const first = items[0] ?? null
  const last = items[items.length - 1] ?? null
  const i = from === null ? -1 : items.indexOf(from)
  if (i < 0) return back ? last : first
  if (back && i === 0) return last
  if (!back && i === items.length - 1) return first
  return null
}
