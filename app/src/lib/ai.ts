// The opt-in AI layer from the UI (pbi-ai, docs/AI.md): status, availability of the AI buttons,
// and running `ai.explainMeasure` / `ai.generateMeasure` with the consent flow. AI is off by
// default; nothing is sent unless a person enabled it in Options → AI and recorded a consent for
// the provider endpoint, capability and kind of data. When a request lacks that consent the
// backend refuses it before any network access (`unsupported`, `data.reason: consentRequired`);
// `runAi` then asks the person through the consent dialog — which shows `ai.preview`, the exact
// request — and only an explicit "Allow and send" records the consent and retries once.

import { commands } from './bindings'
import { execute, hasBackend, ops, type CallOptions } from './ops'
import type { AiPreviewParams, AiStatusOut, Capability, ConsentGrantParams, DataExposure, OpsOutput, OpsParams, OpsResult, ProviderKind } from './ops'

/** Tooltip of the AI buttons while AI is off. */
export const AI_OFF_TITLE = 'Turn on AI in Options (File → Options and settings → Options → AI)'

/** Tooltip of the AI buttons while no provider is configured. */
export const AI_NO_PROVIDER_TITLE = 'Add an AI provider in Options (File → Options and settings → Options → AI)'

/** Whether the AI buttons can run, and why not. */
export interface AiAvailability {
  ready: boolean
  /** Tooltip when not ready. */
  reason: string | null
}

export function availability(status: AiStatusOut | null): AiAvailability {
  if (!hasBackend) return { ready: false, reason: 'AI needs the desktop app' }
  if (!status) return { ready: false, reason: AI_OFF_TITLE }
  if (!status.enabled) return { ready: false, reason: AI_OFF_TITLE }
  if (status.providers.length === 0) return { ready: false, reason: AI_NO_PROVIDER_TITLE }
  return { ready: true, reason: null }
}

/** `ai.status` (`null` without the backend or when it fails). */
export async function loadStatus(): Promise<AiStatusOut | null> {
  const result = await ops.aiStatus({})
  return result.ok ? result.value : null
}

/** The debug-only demo provider answers (`DASHCRAFT_AI_MOCK=1`). */
export async function demoMode(): Promise<boolean> {
  if (!hasBackend) return false
  try {
    return await commands.aiDemoMode()
  } catch {
    return false
  }
}

export const KIND_LABELS: Record<ProviderKind, string> = {
  openAiCompatible: 'OpenAI-compatible',
  azureOpenAi: 'Azure OpenAI',
  ollama: 'Ollama',
  mock: 'Mock (no network)',
}

export const CAPABILITY_LABELS: Record<Capability, string> = {
  'dax.explain': 'Explain a measure',
  'dax.generate': 'Suggest a measure',
  'm.generate': 'Write Power Query (M)',
  'report.lint': 'Review a report',
  'chart.suggest': 'Suggest a chart',
}

export const EXPOSURE_LABELS: Record<DataExposure, string> = {
  metadata: 'Model metadata (names, types, descriptions, relationships, DAX; no row values)',
  sample: 'Model metadata and sample rows of one table',
  full: 'Whole tables (never sent)',
}

/** What a refused request needs: the consent the backend asked for. */
export interface ConsentNeeded {
  provider: string
  endpointHost: string
  capability: Capability
  exposure: DataExposure
  local: boolean
  /** The `ai.consent.grant` parameters that would allow it. */
  grant: ConsentGrantParams
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v)
}

/** The consent a failed AI command needs, or `null` if it failed for another reason. */
export function consentNeeded(result: OpsResult<unknown>): ConsentNeeded | null {
  if (result.ok || result.kind !== 'unsupported' || !isRecord(result.data)) return null
  const d = result.data
  if (d.reason !== 'consentRequired' || !isRecord(d.grant) || !isRecord(d.grant.params)) return null
  const p = d.grant.params
  if (typeof p.provider !== 'string' || typeof p.capability !== 'string') return null
  const exposure = (typeof p.exposure === 'string' ? p.exposure : 'metadata') as DataExposure
  return {
    provider: p.provider,
    endpointHost: typeof d.endpointHost === 'string' ? d.endpointHost : '',
    capability: p.capability as Capability,
    exposure,
    local: d.local === true,
    grant: { provider: p.provider, capability: p.capability as Capability, exposure },
  }
}

/** The stable `data.reason` of a failed AI command (`disabled`, `apiKeyMissing`, …). */
export function failureReason(result: OpsResult<unknown>): string | null {
  return !result.ok && isRecord(result.data) && typeof result.data.reason === 'string' ? result.data.reason : null
}

/**
 * Asks the person for `need` (the consent dialog: shows `preview`, the exact request, and records
 * the consent only on "Allow and send"). Resolves `true` once the consent is recorded.
 */
export type AskConsent = (need: ConsentNeeded, preview: AiPreviewParams) => Promise<boolean>

type AiCommand = 'ai.explainMeasure' | 'ai.generateMeasure'

/** Nothing was sent: the person did not allow the request. */
export const NOT_ALLOWED = 'Nothing was sent: the request was not allowed.'

/**
 * Runs an AI command; if it needs a consent, asks (`ask`, with the preview of the same request)
 * and retries once after an explicit grant. `options.latest` names the run for `cancelLatest`.
 */
export async function runAi<K extends AiCommand>(id: K, params: OpsParams<K>, preview: AiPreviewParams, ask: AskConsent, options: CallOptions = {}): Promise<OpsResult<OpsOutput<K>>> {
  const first = await execute(id, params, options)
  const need = consentNeeded(first)
  if (!need) return first
  const allowed = await ask(need, { ...preview, provider: need.provider })
  if (!allowed) return { ok: false, error: NOT_ALLOWED, kind: 'cancelled' }
  return execute(id, { ...params, provider: need.provider }, options)
}

/** A short, human description of where requests go. */
export function placeOf(local: boolean): string {
  return local ? 'local — data stays on this computer' : 'remote — data leaves this computer'
}

/** `2026-10-22T18:07:24Z` → local date and time. */
export function when(rfc3339: string): string {
  const d = new Date(rfc3339)
  return Number.isNaN(d.getTime()) ? rfc3339 : d.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' })
}
