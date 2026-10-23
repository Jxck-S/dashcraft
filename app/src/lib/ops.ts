// The command registry from the UI: typed wrappers (`ops.dataPage(params)`, generated into
// ops.gen.ts from the registry's JSON Schemas by `npm run gen:ops`) over the one generic IPC
// command `ops_execute`, which runs `pbi_ops::Registry::execute` — the same commands as
// `dashcraft run`, `dashcraft serve` and agents. Progress arrives as `ops://progress` events of the
// request; `cancel` / `latest` cancel a running request.

import { isTauri } from '@tauri-apps/api/core'
import { commands, events, type Json, type OpErrorDto, type OpsProgress } from './bindings'
import { makeOps, type OpErrorCode, type OpsId, type OpsOutput, type OpsParams, type OpsResult } from './ops.gen'

export type * from './ops.gen'
export type { OpsProgress } from './bindings'

export const hasBackend: boolean = isTauri()

export const NO_BACKEND = 'No Rust backend: start the app with `npm run tauri dev`.'

/** Error kinds of a registry command, plus `backend` for IPC/framework failures. */
export type OpsErrorKind = OpErrorCode | 'backend'

export type OpsOutcome<T> = OpsResult<T>

export interface CallOptions {
  /** Names the run for `cancel` and progress events (default: a fresh id). */
  requestId?: string
  /** Progress events of this run (`data.refresh`: one per table / calculated column). */
  onProgress?: (progress: OpsProgress) => void
  /** Cancels the running request started with the same key: the newest request wins. */
  latest?: string
}

/** The message shown for a failed command. */
export function describeOpError(error: OpErrorDto): string {
  const m = error.message
  switch (error.code as OpErrorCode) {
    case 'invalidParams':
      return `Invalid request: ${m}`
    case 'notFound':
      return `Not found: ${m}`
    case 'unsupported':
      return `Not supported: ${m}`
    case 'pathDenied':
      return `Access denied: ${m}`
    case 'io':
      return `I/O error: ${m}`
    case 'cancelled':
      return `Cancelled: ${m}`
    case 'busy':
      return `Busy: ${m}`
    case 'unknownCommand':
    case 'internal':
      return `Internal error: ${m}`
    case 'noDocument':
    case 'query':
      return m
  }
  return m
}

let seq = 0
const latestRequests = new Map<string, string>()
const progressHandlers = new Map<string, (progress: OpsProgress) => void>()
let progressListener: Promise<unknown> | null = null

function listenProgress(): Promise<unknown> {
  progressListener ??= events.opsProgress.listen((event) => progressHandlers.get(event.payload.requestId)?.(event.payload))
  return progressListener
}

/** Runs registry command `id` with `params`. */
export async function execute<K extends OpsId>(id: K, params: OpsParams<K>, options: CallOptions = {}): Promise<OpsOutcome<OpsOutput<K>>> {
  if (!hasBackend) return { ok: false, error: NO_BACKEND, kind: 'backend' }
  const requestId = options.requestId ?? `ui-${++seq}`
  if (options.latest) {
    const previous = latestRequests.get(options.latest)
    if (previous) void cancel(previous)
    latestRequests.set(options.latest, requestId)
  }
  try {
    if (options.onProgress) {
      await listenProgress()
      progressHandlers.set(requestId, options.onProgress)
    }
    const result = await commands.opsExecute(requestId, id, params as unknown as Json)
    return result.status === 'ok'
      ? { ok: true, value: result.data as unknown as OpsOutput<K> }
      : { ok: false, error: describeOpError(result.error), kind: result.error.code as OpErrorCode, data: result.error.data }
  } catch (e) {
    // Framework-level failures (ACL denial, bad arguments) reject with a string or Error.
    return { ok: false, error: String(e), kind: 'backend' }
  } finally {
    progressHandlers.delete(requestId)
    if (options.latest && latestRequests.get(options.latest) === requestId) latestRequests.delete(options.latest)
  }
}

/** Cancels the running request `requestId` (it fails with `cancelled`); `false` if none. */
export async function cancel(requestId: string): Promise<boolean> {
  if (!hasBackend) return false
  try {
    return await commands.opsCancel(requestId)
  } catch {
    return false
  }
}

/** Cancels the running request started with `latest: key`; `false` if none. */
export async function cancelLatest(key: string): Promise<boolean> {
  const id = latestRequests.get(key)
  return id ? cancel(id) : false
}

/** Every registry command as a typed function: `ops.docOpen({ path })`. */
export const ops = makeOps<CallOptions>(execute)
