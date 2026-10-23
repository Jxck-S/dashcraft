/** Human-readable byte size, e.g. `1.4 MB`. */
export function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null || !Number.isFinite(bytes)) return ''
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  return unit === 0 ? `${value} ${units[unit]}` : `${value.toFixed(1)} ${units[unit]}`
}

/** `1 table`, `4 tables`, `1 relationship`, … (English plural with `s`). */
export function plural(count: number, noun: string): string {
  return `${count.toLocaleString()} ${noun}${count === 1 ? '' : 's'}`
}

/** Last path component (either separator), for labels while a file opens. */
export function baseName(path: string): string {
  const parts = path.split(/[\\/]/).filter((p) => p !== '')
  return parts[parts.length - 1] ?? path
}

/** Local date and time of an RFC 3339 timestamp, e.g. `10/21/2026, 2:05 PM`; `null` if invalid. */
export function formatTimestamp(iso: string | null | undefined): string | null {
  if (!iso) return null
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return null
  return date.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })
}
