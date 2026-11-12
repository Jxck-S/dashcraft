// Line diff of two short texts (Compare's TMDL and JSON snippets, at most a few hundred lines
// each): the longest common subsequence of lines, then the lines of each side that are not in it.

export type DiffLine = { kind: 'same' | 'removed' | 'added'; text: string }

/** The lines of `before` and `after` in order: kept, only in `before` (removed) and only in
 *  `after` (added); at a change, the removed lines come first. */
export function diffLines(before: string, after: string): DiffLine[] {
  const a = before === '' ? [] : before.split('\n')
  const b = after === '' ? [] : after.split('\n')
  const n = a.length
  const m = b.length
  // lcs[i * (m + 1) + j]: the common lines of a[i..] and b[j..].
  const lcs = new Uint32Array((n + 1) * (m + 1))
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i * (m + 1) + j] =
        a[i] === b[j] ? lcs[(i + 1) * (m + 1) + j + 1] + 1 : Math.max(lcs[(i + 1) * (m + 1) + j], lcs[i * (m + 1) + j + 1])
    }
  }
  const out: DiffLine[] = []
  let i = 0
  let j = 0
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push({ kind: 'same', text: a[i] })
      i++
      j++
    } else if (lcs[(i + 1) * (m + 1) + j] >= lcs[i * (m + 1) + j + 1]) {
      out.push({ kind: 'removed', text: a[i++] })
    } else {
      out.push({ kind: 'added', text: b[j++] })
    }
  }
  while (i < n) out.push({ kind: 'removed', text: a[i++] })
  while (j < m) out.push({ kind: 'added', text: b[j++] })
  return out
}
