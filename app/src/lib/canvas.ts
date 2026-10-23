// Helpers of the interactive report canvas (ReportCanvas.svelte): safe insertion of the
// backend's SVG, hit ids and hit-testing in page coordinates.

import type { CanvasInteractionOut as CanvasInteractionDto, HitRegionOut as HitRegionDto, RectOut as RectDto } from './ops'

/** Elements never inserted (the renderer does not emit them; defence in depth). */
const FORBIDDEN = new Set(['script', 'foreignobject', 'iframe', 'object', 'embed', 'style', 'a', 'animate', 'set', 'animatemotion', 'animatetransform', 'handler', 'listener'])

const SVG_NS = 'http://www.w3.org/2000/svg'

/**
 * Parses an SVG document produced by the Rust renderer and returns a sanitized element for the
 * current document: no scripts, foreign content, styles, links, animations or event handler
 * attributes, `href`s only to `data:image/…` or local `#ids`, and every `id` prefixed with
 * `prefix` (with the `url(#…)` / `#…` references rewritten) so several SVGs can live in one
 * document without clip-path clashes. `null` if the text is not a well-formed SVG.
 */
export function sanitizeSvg(text: string, prefix: string): SVGSVGElement | null {
  const parsed = new DOMParser().parseFromString(text, 'image/svg+xml')
  const root = parsed.documentElement
  if (!root || root.namespaceURI !== SVG_NS || root.localName !== 'svg' || parsed.getElementsByTagName('parsererror').length > 0) return null
  const walk = (el: Element) => {
    for (const child of Array.from(el.children)) {
      if (FORBIDDEN.has(child.localName.toLowerCase()) || child.namespaceURI !== SVG_NS) child.remove()
      else walk(child)
    }
    for (const attr of Array.from(el.attributes)) {
      const name = attr.name.toLowerCase()
      const value = attr.value
      if (name.startsWith('on')) {
        el.removeAttribute(attr.name)
      } else if (name === 'href' || name === 'xlink:href') {
        if (value.startsWith('#')) el.setAttribute(attr.name, `#${prefix}${value.slice(1)}`)
        else if (!/^data:image\/(png|jpeg|gif|webp|bmp|svg\+xml);/i.test(value)) el.removeAttribute(attr.name)
      } else if (name === 'id') {
        el.setAttribute('id', `${prefix}${value}`)
      } else if (name === 'style') {
        if (/url\s*\(|expression|javascript:/i.test(value)) el.removeAttribute(attr.name)
      } else if (value.includes('url(')) {
        // clip-path, fill, mask, filter, …: local references only.
        const local = value.replace(/url\(\s*#([^)\s]+)\s*\)/g, (_, id: string) => `url(#${prefix}${id})`)
        if (/url\(\s*[^#\s]/.test(local)) el.removeAttribute(attr.name)
        else el.setAttribute(attr.name, local)
      }
    }
  }
  walk(root)
  return document.importNode(root, true) as unknown as SVGSVGElement
}

export type HitKind = 'visual' | 'dataPoint' | 'slicerItem' | 'button'

export interface Hit {
  id: string
  visual: string
  kind: HitKind
  series?: number
  index?: number
}

/** Inverse of the Rust `HitId::encode` (`<visual>|dataPoint|<series>|<index>`, …). */
export function parseHit(id: string): Hit | null {
  const parts = id.split('|')
  const visual = (parts[0] ?? '').replace(/%7C/gi, '|').replace(/%25/g, '%')
  switch (parts[1]) {
    case 'visual':
    case 'button':
      return parts.length === 2 ? { id, visual, kind: parts[1] } : null
    case 'slicerItem':
      return parts.length === 3 ? { id, visual, kind: 'slicerItem', index: Number(parts[2]) } : null
    case 'dataPoint':
      return parts.length === 4 ? { id, visual, kind: 'dataPoint', series: Number(parts[2]), index: Number(parts[3]) } : null
    default:
      return null
  }
}

export function contains(r: RectDto | null | undefined, x: number, y: number): r is RectDto {
  return !!r && x >= r.x && y >= r.y && x <= r.x + r.width && y <= r.y + r.height
}

/** The topmost hit region at (`x`, `y`) (regions are in paint order). */
export function regionAt(regions: HitRegionDto[], x: number, y: number, kinds?: HitKind[]): Hit | null {
  for (let i = regions.length - 1; i >= 0; i--) {
    const r = regions[i]
    if (!contains(r.rect, x, y)) continue
    const hit = parseHit(r.id)
    if (hit && (!kinds || kinds.includes(hit.kind))) return hit
  }
  return null
}

/** The scrollable area under (`x`, `y`): an open popup first, then lists and grids. */
export function scrollableAt(interactions: CanvasInteractionDto[], x: number, y: number): CanvasInteractionDto | null {
  const open = interactions.find((i) => i.dropdownOpen && contains(i.popup, x, y))
  if (open) return open.scroll ? open : null
  for (let i = interactions.length - 1; i >= 0; i--) {
    const it = interactions[i]
    if (!it.scroll || (it.scroll.maxX <= 0 && it.scroll.maxY <= 0)) continue
    if (it.slicerMode === 'dropdown') continue
    if (contains(it.scroll.viewport, x, y)) return it
  }
  return null
}

/** Wheel delta in CSS pixels (line / page modes converted). */
export function wheelPixels(event: WheelEvent): { dx: number; dy: number } {
  const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? 400 : 1
  // Shift + wheel scrolls horizontally, as in browsers.
  const dx = event.shiftKey && event.deltaX === 0 ? event.deltaY : event.deltaX
  const dy = event.shiftKey && event.deltaX === 0 ? 0 : event.deltaY
  return { dx: dx * unit, dy: dy * unit }
}
