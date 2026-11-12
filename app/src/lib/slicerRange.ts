// Range ("Between") and relative date slicer controls of the report canvas (no DOM, so `npm run
// check` tests them in Node). Date values are OADate day numbers, as `canvas.render` returns them.

import type { RangeBound, RangeOut, RelativeAnchorArg, RelativePeriodArg } from './ops.gen.ts'

const DAY_MS = 86_400_000
/** 1899-12-30 (OADate day 0) in Unix milliseconds. */
const OA_EPOCH_MS = Date.UTC(1899, 11, 30)

/** `YYYY-MM-DD` of an OADate day number (the fraction, a time of day, is dropped). */
export function oaDateToIso(days: number): string {
  return new Date(OA_EPOCH_MS + Math.floor(days) * DAY_MS).toISOString().slice(0, 10)
}

/** The OADate day number of a `YYYY-MM-DD` date. */
export function isoToOaDate(iso: string): number {
  return Math.round((Date.parse(`${iso}T00:00:00Z`) - OA_EPOCH_MS) / DAY_MS)
}

const pad = (n: number) => String(n).padStart(2, '0')

/** A typed date as `YYYY-MM-DD`: ISO (`2025-03-01`) or month/day/year (`3/1/2025`); `null` if invalid. */
export function parseDate(text: string): string | null {
  const t = text.trim()
  let m = /^(\d{4})-(\d{1,2})-(\d{1,2})/.exec(t)
  let [y, mo, d] = m ? [+m[1], +m[2], +m[3]] : [0, 0, 0]
  if (!m) {
    m = /^(\d{1,2})\/(\d{1,2})\/(\d{4})$/.exec(t)
    if (!m) return null
    ;[y, mo, d] = [+m[3], +m[1], +m[2]]
  }
  const date = new Date(Date.UTC(y, mo - 1, d))
  if (date.getUTCFullYear() !== y || date.getUTCMonth() !== mo - 1 || date.getUTCDate() !== d) return null
  return `${y}-${pad(mo)}-${pad(d)}`
}

/** The `canvas.setRange` bound typed into a range slicer's box; `null` if it is not a value. */
export function parseRangeInput(range: Pick<RangeOut, 'isDate'>, text: string): RangeBound | null {
  if (range.isDate) return parseDate(text)
  const n = Number(text.trim().replace(/[,\s]/g, ''))
  return text.trim() !== '' && Number.isFinite(n) ? n : null
}

/** A slider value as a `canvas.setRange` bound (dates as `YYYY-MM-DD`). */
export function rangeBound(range: Pick<RangeOut, 'isDate'>, value: number): RangeBound {
  return range.isDate ? oaDateToIso(value) : value
}

/** A slider value as box text while dragging. */
export function rangeText(range: Pick<RangeOut, 'isDate'>, value: number): string {
  return range.isDate ? oaDateToIso(value) : String(value)
}

type Slider = Pick<RangeOut, 'track' | 'min' | 'max' | 'integer' | 'isDate'>

/** Page x of a handle at `value` on the track. */
export function handleX(range: Slider, value: number): number {
  const { track, min, max } = range
  if (!track || min == null || max == null || max <= min) return track?.x ?? 0
  const t = Math.min(1, Math.max(0, (value - min) / (max - min)))
  return track.x + t * track.width
}

/** The value under page x on the track: whole days and integers snap; decimals keep 4 significant digits. */
export function valueAt(range: Slider, x: number): number | null {
  const { track, min, max } = range
  if (!track || min == null || max == null) return null
  if (max <= min || track.width <= 0) return min
  const t = Math.min(1, Math.max(0, (x - track.x) / track.width))
  const v = min + t * (max - min)
  if (range.integer || range.isDate) return Math.min(max, Math.max(min, Math.round(v)))
  return Number(v.toPrecision(4))
}

/** Arrow key step of a handle: a day, 1, or a hundredth of the range. */
export function step(range: Slider): number {
  if (range.integer || range.isDate || range.min == null || range.max == null) return 1
  return Number(((range.max - range.min) / 100).toPrecision(2)) || 1
}

export interface PeriodOption {
  value: RelativePeriodArg
  label: string
}

/** The period choices of a relative date (or time) slicer, as Power BI Desktop lists them. */
export function relativePeriods(anchor: RelativeAnchorArg, time: boolean): PeriodOption[] {
  if (time) {
    return anchor === 'this'
      ? [
          { value: 'minutes', label: 'Minute' },
          { value: 'hours', label: 'Hour' },
        ]
      : [
          { value: 'minutes', label: 'Minutes' },
          { value: 'hours', label: 'Hours' },
        ]
  }
  if (anchor === 'this') {
    return [
      { value: 'days', label: 'Day' },
      { value: 'weeks', label: 'Week' },
      { value: 'months', label: 'Month' },
      { value: 'years', label: 'Year' },
    ]
  }
  return [
    { value: 'days', label: 'Days' },
    { value: 'weeks', label: 'Weeks' },
    { value: 'calendarWeeks', label: 'Weeks (Calendar)' },
    { value: 'months', label: 'Months' },
    { value: 'calendarMonths', label: 'Months (Calendar)' },
    { value: 'years', label: 'Years' },
    { value: 'calendarYears', label: 'Years (Calendar)' },
  ]
}

/** The period after switching the anchor: "This" has no calendar periods, and an unknown period falls back to the first. */
export function periodFor(anchor: RelativeAnchorArg, time: boolean, period: RelativePeriodArg): RelativePeriodArg {
  const plain = (period.startsWith('calendar') ? period.slice(8).toLowerCase() : period) as RelativePeriodArg
  const options = relativePeriods(anchor, time).map((o) => o.value)
  if (options.includes(period)) return period
  if (options.includes(plain)) return plain
  return options[0]
}
