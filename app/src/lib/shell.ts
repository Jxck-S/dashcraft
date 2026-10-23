// Shell types shared by the title bar, ribbon, panes and views.

import type { IconName } from './icons'
import type { ReportThemeDto } from './bindings'

/** The five Desktop views of the left rail, in Desktop order. */
export type ViewId = 'report' | 'table' | 'model' | 'dax' | 'tmdl'

/** View → Page view, plus a custom zoom from the status bar slider. */
export type ZoomMode = 'fitPage' | 'fitWidth' | 'actual' | 'custom'

/** Tooltip of every command DashCraft does not implement. */
export const NOT_AVAILABLE = 'Not available in DashCraft (yet)'

export interface MenuItem {
  label: string
  icon?: IconName
  /** Default `true`. */
  enabled?: boolean
  title?: string
  checked?: boolean
  /** A non-clickable section title. */
  header?: boolean
  /** A separator line (other fields ignored). */
  separator?: boolean
  onclick?: () => void
}

export interface RibbonButton {
  kind: 'button'
  id: string
  label: string
  icon: IconName
  /** Large: 32px icon over a two-line label; small: 16px icon left of the label. */
  size: 'large' | 'small'
  enabled: boolean
  title?: string
  /** Toggle buttons (Show panes, Page view) show their state. */
  pressed?: boolean
  onclick?: () => void
  /** Dropdown items: the button opens a menu (`split`: only the arrow does). */
  menu?: MenuItem[]
  split?: boolean
}

/** Up to three small buttons stacked vertically (Clipboard: Cut / Copy / Format painter). */
export interface RibbonStack {
  kind: 'stack'
  id: string
  items: RibbonButton[]
}

/** Read-only property box of the contextual tabs (Name, Format, Data type, …). */
export interface RibbonField {
  kind: 'field'
  id: string
  label: string
  value: string
  icon?: IconName
  width?: number
  title?: string
}

/** Design → Themes gallery (read-only: the report's theme is highlighted). */
export interface RibbonThemes {
  kind: 'themes'
  id: string
  theme: ReportThemeDto | null
}

export type RibbonControl = RibbonButton | RibbonStack | RibbonField | RibbonThemes

export interface RibbonGroup {
  id: string
  label: string
  controls: RibbonControl[]
}

export interface RibbonTab {
  id: string
  label: string
  groups: RibbonGroup[]
  /** Contextual tab (Table tools, Column tools, Measure tools): shown in the accent color. */
  contextual?: boolean
}

/** A large or small ribbon button. Disabled buttons get the "not available" tooltip. */
export function button(
  id: string,
  label: string,
  icon: IconName,
  options: Partial<Omit<RibbonButton, 'kind' | 'id' | 'label' | 'icon'>> = {},
): RibbonButton {
  const enabled = options.enabled ?? false
  return {
    kind: 'button',
    id,
    label,
    icon,
    size: options.size ?? 'large',
    enabled,
    title: options.title ?? (enabled ? undefined : NOT_AVAILABLE),
    pressed: options.pressed,
    onclick: options.onclick,
    menu: options.menu,
    split: options.split,
  }
}

/** A small button (for stacks). */
export function small(id: string, label: string, icon: IconName, options: Partial<Omit<RibbonButton, 'kind' | 'id' | 'label' | 'icon' | 'size'>> = {}): RibbonButton {
  return button(id, label, icon, { ...options, size: 'small' })
}

export function stack(id: string, ...items: RibbonButton[]): RibbonStack {
  return { kind: 'stack', id, items }
}

export function field(id: string, label: string, value: string, options: Partial<Omit<RibbonField, 'kind' | 'id' | 'label' | 'value'>> = {}): RibbonField {
  return { kind: 'field', id, label, value, ...options }
}

export function group(id: string, label: string, ...controls: RibbonControl[]): RibbonGroup {
  return { id, label, controls }
}

/** Every button of a tab (stacks flattened), e.g. for the title bar command search. */
export function tabButtons(tab: RibbonTab): RibbonButton[] {
  return tab.groups.flatMap((g) => g.controls.flatMap((c) => (c.kind === 'button' ? [c] : c.kind === 'stack' ? c.items : [])))
}
