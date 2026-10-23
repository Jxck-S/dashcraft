# ADR-0008: An opt-in AI layer as a registry plugin (`pbi-ai`)

- Status: Accepted
- Date: 2026-10-22
- Deciders: pbiview maintainers
- Related: [ADR-0007](0007-command-registry.md) (command registry), [ADR-0004](0004-desktop-ready-architecture.md); research: [photocraft-review.md](../research/photocraft-review.md) §A and recommendation 14, [artcraft-infra-review.md](../research/artcraft-infra-review.md) §5; user guide: [AI.md](../AI.md)

## Context

Report authors want help reading and writing DAX: "what does this measure do?" and "write me a measure for …". Large language models do this well, but pbiview is an offline tool for files that often hold confidential data, its security policy forbids telemetry and keeps credentials in the OS keychain, and every capability must be reachable the same way from the CLI, the RPC server, MCP agents and the app (ADR-0007).

The *Craft family's shared AI layer (studied for design only; no code taken) separates a pure capability contract from provider adapters, scopes consent per destination, capability and kind of data, keeps keys out of shared settings, and states plainly what was tested against scripted servers versus real providers. Its weak points for us: it is broad (images, video, local model runtimes), unadopted, and git-pinned.

## Decision

A new crate, **`pbi-ai`** (layer 5, next to `pbi-ops`; edge `pbi-ai → pbi-ops`), holds the whole AI layer. It is a **registry plugin**: `pbi_ai::register(&mut Registry)` adds the `ai.*` commands, and `Registry::default_catalog()` stays network-free. Front ends opt in.

- **Capability contract.** `Capability` with stable ids `dax.explain`, `dax.generate`, `m.generate`, `report.lint`, `chart.suggest`; each has typed input and output structs (serde + `JsonSchema`) behind a `CapabilityContract` trait, and a `DataExposure` class: `metadata` (names, types, descriptions, format strings, relationships, DAX; no row values), `sample` (at most `sampleRows` rows of one table, off by default), `full` (never sent; cannot be granted). v1 implements `dax.explain` and `dax.generate`; the others return `unsupported` (`data.reason = "notImplemented"`). M partition sources are not sent in v1: they can embed literal rows and server names.
- **Providers.** A `Provider` trait (`http_request(&ChatRequest) -> HttpRequest`, `complete(ChatRequest, CancellationToken)`) with `OpenAiCompatible` (OpenAI, GitHub Models, LM Studio, vLLM; Bearer key), `AzureOpenAi` (deployment path, `api-version`, `api-key` header), `Ollama` (`/api/chat`, no key) and `MockProvider` (scripted or canned answers, records requests; used by every test). JSON output is requested as a JSON schema (`response_format: json_schema`, Ollama `format`), as `json_object`, or by prompt only; the answer is extracted (whole text, fenced block, first object), deserialized into the capability's output type (the type its schema is generated from) and checked; failures carry the raw answer, truncated and redacted. A `ProviderFactory` builds providers from settings (replaceable for tests and embedders).
- **Transport.** `ureq` 3 (blocking, rustls with webpki roots, gzip) on a detached thread awaited through a oneshot channel, with connect, response and body timeouts; cancelling the context's token returns `cancelled` immediately. Not `tokio::task::spawn_blocking`: a Tokio runtime waits for its blocking tasks when it shuts down, so a cancelled request would keep the CLI alive until the request's timeout; the abandoned thread ends at that timeout. No redirects are followed; loopback endpoints bypass proxies; endpoints must be `https` unless on loopback or explicitly `allowHttp`; answers are capped at 4 MiB.
- **Settings** (`AiSettings`, JSON, `deny_unknown_fields`): `enabled` (default `false`), providers (id, kind, endpoint, model, deployment, `apiVersion`, `structuredOutput`, `allowHttp`), default provider, per-capability limits (tokens, temperature, timeout, prompt size), `sampleRows` (default 0, at most 100) and the recorded consents. The path is supplied by the host (`AiHost::new(path)`); `default_settings_path()` resolves `$DASHCRAFT_CONFIG_DIR`/`$PBIVIEW_CONFIG_DIR`, else the platform configuration folder + `dashcraft/ai.json`. Writes are atomic (temporary file + rename, owner-only on Unix). Settings containing secret-looking fields are refused without echoing them.
- **Consent.** `Consent { provider, endpointHost, capability, exposure, local, grantedAt, grantedBy }`. A request is sent only when AI is enabled and a consent matches the provider id, endpoint `host[:port]`, capability and an exposure class at least as high as the request's. Local endpoints need consent too (recorded and reported as local). Changing a provider's endpoint drops its consents. The checks happen before any network access, in this order: enabled, prompt size, consent, key.
- **Keys.** Only in the OS keychain (`keyring`, service `dashcraft`, account `ai:<provider-id>`) or `DASHCRAFT_AI_API_KEY` (`PBIVIEW_AI_API_KEY`) for CI and headless use (after the keychain). `Secret` prints as `[redacted]` and is not `Serialize`; `HttpRequest` keeps keys in `HeaderValue::Secret`, so previews show `Bearer [redacted]`; a `Redactor` removes the key (and any `Bearer`/`api-key` value) from HTTP error bodies and transport errors, and the key from model answers. `store_api_key(provider, key)` is the library entry for `pbiview ai login` and the app.
- **Host.** Registry commands are stateless functions, so the host (settings path, secret store, provider factory, environment) is process-wide (`AiHost::install`) with a task-local override (`AiHost::scope`) for tests and embedders; without either, the default host uses the default path, the keychain and the built-in adapters.
- **Commands** (category `ai`, since 0.3.0):

  | Command | Agent-safe | Network |
  |---|---|---|
  | `ai.status` | yes | never |
  | `ai.preview` (the exact request, key redacted, exposure class, consent status, blockers) | yes | never; works while disabled |
  | `ai.explainMeasure` | yes | when enabled and consented |
  | `ai.generateMeasure` (parsed with `pbi-dax`, diagnostics, optional local evaluation, never applied) | yes | when enabled and consented |
  | `ai.consent.grant`, `ai.consent.revoke`, `ai.settings.set` | no (`writes_files`) | never |
  | `ai.apiKey.set` (desktop app callers only), `ai.apiKey.clear` | no (`writes_files`) | never |

  The settings commands write the host's settings file, not a path from the parameters, so the path policy does not apply; they refuse agent callers again inside `run` (defence in depth).
- **Prompts** are versioned Markdown templates in `crates/pbi-ai/prompts/` (`system`, `dax.explain`, `dax.generate`; header `<!-- dashcraft-prompt id=… version=… -->`), compiled in with `include_str!`, rendered in one pass, and reported in every result (`system@1+dax.explain@1`).
- **Supply chain.** `cargo xtask layers` gains a network rule: HTTP/TLS client and keychain crates (`ureq*`, `reqwest*`, `hyper*`, `h2`, `rustls*`, `native-tls`, `openssl*`, `webpki-roots`, `curl*`, `keyring*`, `*-keyring-store`, …) may only be reached by `pbi-ai` and crates that depend on it; `pbiview-app` is exempt because Tauri 2 already depends on `reqwest` and `hyper` (without a TLS backend). `cargo deny` passes unchanged (ring: Apache-2.0 AND ISC; webpki-roots: CDLA-Permissive-2.0, both allowed).

## Threat model

| Threat | Mitigation |
|---|---|
| Data leaves the machine without the user knowing | Off by default; per provider-endpoint, capability and exposure consent recorded with a timestamp; `ai.preview` shows the exact payload; `ai.status` lists consents; no background or automatic calls; local endpoints also need consent |
| More data than expected is sent | Exposure classes with ordering; v1 sends metadata only, sample rows only with a `sample` consent, a `sampleRows` setting and an explicit parameter; `full` cannot be granted; requests above `maxInputChars` are refused |
| Prompt injection through model metadata (a measure description saying "ignore previous instructions") | Metadata is JSON-encoded inside `<model_metadata>` blocks with `<`, `>` and `&` escaped, so it cannot close the block; templates are rendered in one pass, so values cannot add placeholders; the system prompt tells the model to treat data blocks as content; outputs are schema-validated text, never executed, never applied to the model; generated DAX is only parsed and, on request, evaluated read-only by pbiview's own engine |
| An agent (MCP) grants itself consent, enables AI or reads keys | Settings, consent and key commands are `agent_safe: false` (the registry refuses agent callers; the commands check again); no command returns a key; `ai.apiKey.set` accepts keys only from the app |
| Key disclosure | Keys only in the OS keychain or an environment variable; never in settings (refused), logs (only provider, host, capability, exposure and size are logged), errors, previews or results (redacted); `Secret` has no `Serialize` and a redacted `Debug` |
| Key sent to the wrong place | Keys are per provider id; redirects are not followed; plain `http` to another host needs `allowHttp`; credentials in URLs are refused |
| Hostile or broken provider answers | 4 MiB cap, timeouts, JSON extraction and typed validation with errors that quote the (redacted, truncated) answer |

## Alternatives considered

| Option | Why not chosen |
|---|---|
| AI commands in `pbi-ops` (in the default catalog) | Every front end, including agents with restricted roots, would link an HTTP client and a keychain; "offline by construction" could no longer be checked by `cargo xtask layers`. A plugin keeps the default build network-free and makes opting in one line. |
| `reqwest` + Tokio | Pulls `hyper`, `h2`, `tower` and a larger TLS stack for a handful of non-streaming requests; `ureq` with rustls is small, blocking (easy to reason about and to cancel by abandoning) and has no OpenSSL dependency, so builds stay reproducible on every OS. |
| `native-tls` / OpenSSL | System TLS differs per OS and needs OpenSSL on Linux; rustls with webpki roots behaves the same everywhere. A platform-verifier option can be added for intercepting proxies. |
| Depending on the *Craft `craft-ai-*` crates | Unadopted, git-pinned, far broader scope (image, video, local runtimes), and some repositories have no licence; the design is borrowed, not the code. |
| Consent as a CLI flag (`--allow-network`) | Not recorded, not scoped to a capability or data class, and trivially passed by an agent; recorded consents can be listed, revoked and audited. |
| Streaming responses | More complex cancellation and validation for little benefit with structured JSON answers; can be added per adapter later. |

## Consequences

- Front ends opt in with `pbi_ai::register(&mut registry)` (CLI `run`, `serve`, `commands`, `mcp`; the app) and may install a host with the app's configuration folder. The CLI's interactive `pbiview ai login` uses `pbi_ai::store_api_key`.
- `Category::Ai` exists in `pbi-ops` but has no command in `default_catalog()`; `docs/COMMANDS.md` documents the core catalog and `docs/AI.md` the `ai.*` commands (generated section kept in sync by a test).
- Adapters are tested against a local fake HTTP server and the mock only; real providers, intercepting proxies and Linux keychains are not verified yet ([AI.md](../AI.md#what-has-been-verified)).
- Next capabilities (`m.generate`, `report.lint`, `chart.suggest`) need a prompt template, a context builder and a command each; the contract types and consent model already exist.
