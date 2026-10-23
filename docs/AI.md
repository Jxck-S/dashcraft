# AI features (opt-in)

DashCraft (`dashcraft`) can ask a language model to **explain a measure** or **suggest a new measure**. These features are **off by default**, never send anything without a consent you record, and keep API keys in your operating system's keychain. Design and threat model: [ADR-0008](adr/0008-ai-layer.md).

> **Privacy statement.** DashCraft works offline. With AI off (the default) it makes no network requests at all. With AI on, a request goes only to the provider endpoint you configured, only for a capability you consented to, and only with the kind of data you consented to (model metadata by default, never whole tables). Nothing is sent in the background, there is no telemetry, and DashCraft operates no server of its own: the provider you choose (a model on your machine, your Azure OpenAI resource, OpenAI, GitHub Models, …) receives the request under its own terms. Use `ai.preview` to see the exact request before anything is sent.

## Capabilities

| Capability | Command | Sends | Status |
|---|---|---|---|
| `dax.explain` | `ai.explainMeasure` | metadata | implemented |
| `dax.generate` | `ai.generateMeasure` | metadata (+ sample rows if you allow them) | implemented |
| `m.generate` | — | metadata | contract only |
| `report.lint` | — | metadata | contract only |
| `chart.suggest` | — | metadata (+ sample rows) | contract only |

- **Explain a measure** returns a summary, a Markdown explanation, filter-context and context-transition notes, a note per dependency and caveats, plus a locally computed list of the measures (transitively), columns (with types), tables and relationships it references.
- **Suggest a measure** turns a request in plain language into a DAX measure. DashCraft parses the suggestion with its own DAX parser (errors with line and column), warns about unknown tables, columns, measures and unsupported functions, and evaluates it locally only if you ask (`evaluate: true`). Suggestions are **never written to the model**; copy them yourself.

### What "metadata" means

| Exposure class | Contains | Used by |
|---|---|---|
| `metadata` | table, column and measure names, data types, descriptions, format strings, display folders, relationships, DAX of measures, calculated columns and calculated tables | every capability |
| `sample` | metadata plus at most `sampleRows` rows of one table (the first rows) | `dax.generate` when you pass `sampleRows` |
| `full` | whole tables | nothing (it cannot be granted) |

Power Query (M) partition sources are not sent by v1 capabilities: they can embed literal data (`#table(…)`) and server or file names.

## Turning it on

All settings live in one JSON file, `ai.json`, in the DashCraft configuration folder:

| OS | Default location |
|---|---|
| macOS | `~/Library/Application Support/dashcraft/ai.json` |
| Linux | `$XDG_CONFIG_HOME/dashcraft/ai.json` (else `~/.config/dashcraft/ai.json`) |
| Windows | `%APPDATA%\dashcraft\ai.json` |

`DASHCRAFT_CONFIG_DIR` (or `PBIVIEW_CONFIG_DIR`) overrides the folder. `dashcraft run ai.status` shows the path in use.

The desktop app keeps its own `ai.json` in its configuration folder (macOS `~/Library/Application Support/io.github.jxck-s.dashcraft/`, Linux `$XDG_CONFIG_HOME/io.github.jxck-s.dashcraft/` or `~/.config/io.github.jxck-s.dashcraft/`, Windows `%APPDATA%\io.github.jxck-s.dashcraft\`), next to its recent-files list; with `DASHCRAFT_CONFIG_DIR` (or `PBIVIEW_CONFIG_DIR`) set, the app and the CLI share `$DASHCRAFT_CONFIG_DIR/ai.json`. API keys are shared either way: both use the same keychain entries. In the app everything below is done in **Options → AI** ([In the desktop app](#in-the-desktop-app)). The file never contains keys; a file or update that tries to add an `apiKey`, `token`, `secret` or similar field is refused.

Change settings with `ai.settings.set` (fields you leave out keep their value), or edit the file:

```sh
# A local model with Ollama (https://ollama.com): nothing leaves the machine.
dashcraft run ai.settings.set --params '{"enabled": true,
  "providers": [{"id": "ollama", "kind": "ollama", "model": "qwen2.5-coder:7b"}]}'
dashcraft run ai.consent.grant -p provider=ollama -p capability=dax.explain
dashcraft run ai.explainMeasure --doc model.pbip -p measure="Total Sales" | jq -r .markdown
```

```json
{
  "version": 1,
  "enabled": true,
  "defaultProvider": "ollama",
  "providers": [
    {"id": "ollama", "kind": "ollama", "model": "qwen2.5-coder:7b"},
    {"id": "openai", "kind": "openAiCompatible", "model": "gpt-4.1-mini"},
    {"id": "github", "kind": "openAiCompatible", "endpoint": "https://models.github.ai/inference", "model": "openai/gpt-4.1-mini"},
    {"id": "lmstudio", "kind": "openAiCompatible", "endpoint": "http://127.0.0.1:1234/v1", "model": "qwen2.5-coder-7b-instruct"},
    {"id": "azure", "kind": "azureOpenAi", "endpoint": "https://contoso.openai.azure.com", "deployment": "gpt-4o", "apiVersion": "2024-10-21"}
  ],
  "limits": {"dax.generate": {"maxTokens": 1500, "temperature": 0.1, "timeoutSecs": 90}},
  "sampleRows": 0,
  "consents": []
}
```

### Providers

| `kind` | Endpoint (default) | Key | Request |
|---|---|---|---|
| `ollama` | `http://127.0.0.1:11434` | none | `POST /api/chat`, `stream: false`, output schema as `format` |
| `openAiCompatible` | `https://api.openai.com/v1` | `Authorization: Bearer …` (optional for a server on this machine) | `POST {endpoint}/chat/completions`, `response_format: json_schema` |
| `azureOpenAi` | required: `https://<resource>.openai.azure.com` | `api-key: …` | `POST {endpoint}/openai/deployments/{deployment}/chat/completions?api-version=…` |
| `mock` | none | none | canned answers, no network (tests and demos) |

`openAiCompatible` covers OpenAI, GitHub Models, LM Studio, vLLM, the llama.cpp server and other compatible servers. Endpoints must use `https` unless the host is this machine; for a server on your network over plain `http`, set `"allowHttp": true` on that provider (requests, and keys, then travel unencrypted). `structuredOutput` (`jsonSchema`, the default; `jsonObject`; `none`) selects how JSON is requested from servers that lack JSON-schema output; the answer is validated either way. Requests to this machine bypass proxies; other requests honour `HTTPS_PROXY`/`NO_PROXY`. Redirects are never followed.

Per-capability `limits`: `maxTokens` (default 2000), `temperature` (0.2), `timeoutSecs` (120), `maxInputChars` (60000).

### API keys

Keys are stored only in the OS keychain (macOS Keychain, Windows Credential Manager, Secret Service on Linux) under service `dashcraft`, account `ai:<provider id>`:

- on the command line: `dashcraft ai login` (prompts without echo);
- in the app: File → Options and settings → Options → **AI** → **Set key…** next to the provider (a password field; the key goes to the keychain through `ai.apiKey.set` and is never shown again; **Clear key** runs `ai.apiKey.clear`);
- for CI or headless machines without a keychain: the environment variable `DASHCRAFT_AI_API_KEY` (alias `PBIVIEW_AI_API_KEY`), used when the keychain has no key for the provider.

`ai.apiKey.set` accepts a key in its parameters **only from the desktop app**; the CLI and RPC callers are refused so keys don't end up in shell history or logs. `ai.apiKey.clear` removes a stored key. Keys never appear in the settings file, logs, error messages, previews or results; provider error bodies are redacted before they are shown.

## Consent

A request is sent only when a recorded consent covers **all** of: the provider id, its endpoint host and port, the capability, and the exposure class (a `sample` consent also covers `metadata`). A consent for a local endpoint is required too; it is recorded with `"local": true`. Changing a provider's endpoint drops its consents.

```sh
dashcraft run ai.consent.grant -p provider=openai -p capability=dax.generate            # metadata
dashcraft run ai.consent.grant -p provider=openai -p capability=dax.generate -p exposure=sample
dashcraft run ai.consent.revoke -p provider=openai                                      # all of a provider
dashcraft run ai.status --pretty                                                         # providers, consents, limits
```

Without a consent the command fails with `unsupported` and `data.reason = "consentRequired"`, describing exactly what would be sent where and the `ai.consent.grant` parameters to allow it.

## In the desktop app

The app runs the same `ai.*` commands (its registry is `pbi_ai::catalog()`, called as the desktop app, so `ai.apiKey.set` is accepted). AI stays off until you turn it on; every AI button stays visible but disabled, with the tooltip "Turn on AI in Options (File → Options and settings → Options → AI)" (or "Add an AI provider in Options" when none is configured). Home → AI → **AI options** opens the page at any time.

**Options → AI** (File → Options and settings → **Options**: a Desktop-style dialog with the pages on the left; *AI* is the only page DashCraft implements, the others are listed disabled). From top to bottom:

- a privacy note: off by default, nothing leaves the computer unless you allow it, metadata only, no telemetry, no DashCraft server, local providers keep everything on the machine;
- **Turn on AI features** (`ai.settings.set {enabled}`); turning it off stops every request and keeps providers and consents;
- **Providers**: a table with a *Default* radio button (`defaultProvider`), the name and kind, the endpoint with a *local* or *remote* badge, the model or deployment, the key status ("Key stored in Keychain", "Key from DASHCRAFT_AI_API_KEY", "No key stored", "No key needed") and **Edit**, **Set key…** / **Replace key…**, **Clear key**, **Remove** (asks first; optionally clears the key too). **Add provider** opens a form: kind (OpenAI-compatible, Azure OpenAI, Ollama), id, display name, endpoint (placeholder: the kind's default), model — or, for Azure, deployment and API version — and, under *Advanced*, the JSON output mode and "Allow plain http to another computer". The form checks the id, required fields and the URL before **Save provider** sends the whole list with `ai.settings.set`; the backend validates again. Ids cannot be renamed (consents and the keychain entry use them);
- **Set key…** shows a password field (`autocomplete=off`); **Save key** calls `ai.apiKey.set` and clears the field. Keys are write-only: nothing reads them back;
- **Sample rows** (0–100, default 0) with **Apply**;
- **Consents**: provider, endpoint (*local* / *remote*), feature, data class, when and from where it was granted, and **Revoke** (`ai.consent.revoke`). Consents are never granted on this page;
- the settings file path.

Each change is one command and applies at once (the dialog has a single **Close** button); errors from the backend (an invalid endpoint, plain `http` to another machine, a keychain failure) are shown on the page.

**The consent dialog.** When an AI command fails with `data.reason = "consentRequired"`, the app opens **Allow this AI request?**. It calls `ai.preview` for the same request and shows the provider (kind, model), the endpoint host with a *local* or *remote* badge, the feature (capability id), the data-exposure class with its description, the prompt size, the HTTP method, URL and headers (keys as `[redacted]`) and the exact JSON body in a scrollable monospace block, plus anything else that would still block the request (a missing key: then **Allow and send** stays disabled, so a consent is only ever recorded for a request that is sent). **Allow and send** calls `ai.consent.grant` with the `data.grant.params` of the error and retries the command once; **Cancel**, Esc or a click outside send nothing. No other code path grants a consent.

**Explain with AI** (right-click a measure in the Data pane, the ✦ button of the formula bar, the Properties pane, Home → AI → **Explain measure**, Measure tools → **Explain with AI**) opens a pane beside the Data pane: the measure and its DAX, a spinner with the command's progress message and **Cancel** (`ops_cancel`), then the summary, the explanation, filter-context notes, dependencies, caveats, the references DashCraft computed from the model, who answered (provider, model, host, local or remote, duration) and **Copy as Markdown** / **Explain again**. Answers are Markdown rendered by a small renderer of the app's own (`app/src/lib/markdown.ts`): headings, paragraphs, lists, block quotes, code, tables, bold, italic; every character of the answer is HTML-escaped, it emits only its own tags without attributes from the text, and links and images are shown as text (`label (url)`, `[image: alt]`), never as anchors or requests. Its tests (`app/tests/markdown.test.ts`, part of `npm run check`) check that `<script>`, `<img onerror>`, `javascript:` links and quote tricks stay inert.

**Suggest a measure** (Home or Modeling → AI) opens a dialog: what the measure should compute, the home table, the provider (when there are several) and, if the settings allow sample rows, "Include *n* sample rows". **Suggest** runs `ai.generateMeasure` (with the consent flow above) and shows the name, the DAX in a monospace block with **Copy**, the parse and reference diagnostics, the explanation and assumptions. **Evaluate locally** runs a `DEFINE MEASURE … EVALUATE ROW(…)` query with `dax.query` over the loaded data (no AI call, nothing sent) and **Open in DAX query view** opens that query. The dialog says it plainly: suggestions are never written to the model.

**Demo provider (development).** In debug builds only, `DASHCRAFT_AI_MOCK=1 npm run tauri dev` (or a debug `dashcraft-app`) installs a demo provider factory: previews and the consent dialog show the real request for the configured provider, but answers are canned (after 1.5 s, so progress and Cancel can be tried) and nothing is sent; Options → AI shows a *Demo mode* note and also offers the `mock` kind. Enabling AI and consenting are still required. The demo code is behind `cfg(debug_assertions)` and is not compiled into release builds.

## Preview: see the exact request

`ai.preview` builds the request a capability would send — URL, headers (keys shown as `[redacted]`), the exact JSON body with the prompt — and reports the exposure class, the consent status and anything that would stop it (`blockers`). It never sends anything and works while AI is off.

```sh
dashcraft run ai.preview --doc model.pbip -p capability=dax.explain -p measure="Total Sales" --pretty
dashcraft run ai.preview --doc model.pbip -p capability=dax.generate \
  -p prompt="Share of orders shipped late" -p table=Orders --pretty | jq '.request.body.messages[1].content' -r
```

## Examples

```sh
# Explain a measure (Markdown for people, structured fields for tools).
dashcraft run ai.explainMeasure --doc model.pbip -p measure="Orders[Late %]" | jq -r .markdown

# Suggest a measure, parse it and evaluate it locally (unfiltered).
dashcraft run ai.generateMeasure --doc model.pbip --blob-map blob-map.json \
  -p prompt="Average order value for orders shipped this year" -p table=Orders -p evaluate=true --pretty

# With 5 sample rows of Orders (needs "sampleRows": 5 or more in the settings and a sample consent).
dashcraft run ai.generateMeasure --doc model.pbip -p prompt="…" -p table=Orders -p sampleRows=5
```

Results include `call`: provider, model, endpoint host, whether it was local, the exposure class, the prompt version (`system@1+dax.explain@1`), the duration and token usage.

## Agents (MCP) and the RPC server

Agents see `ai.status`, `ai.preview`, `ai.explainMeasure` and `ai.generateMeasure`. They can only use what a person has enabled and consented to: `ai.consent.*`, `ai.settings.set` and `ai.apiKey.*` are not agent-safe and are refused for agent callers. Model metadata is treated as untrusted: the prompt quotes it as JSON inside `<model_metadata>` blocks (with `<` and `>` escaped), the model is told to ignore instructions found in it, and its answers are validated, never executed or applied.

## Errors

AI errors use the registry's codes with a stable `data.reason`:

| `data.reason` | Code | Meaning |
|---|---|---|
| `disabled` | `unsupported` | AI is off |
| `consentRequired` | `unsupported` | no consent covers the request (details in `data`) |
| `apiKeyMissing` | `unsupported` | the provider needs a key and none is stored |
| `notImplemented` | `unsupported` | a contract-only capability |
| `forbidden` | `unsupported` | e.g. `ai.apiKey.set` from the CLI |
| `noProvider`, `unknownProvider` | `unsupported`, `notFound` | no or no such provider |
| `invalidSettings`, `invalidInput`, `inputTooLarge` | `invalidParams` | bad settings or parameters, prompt above `maxInputChars` |
| `connection`, `timeout`, `http`, `protocol`, `settingsIo`, `keychain` | `io` | the provider or the local store failed |
| `invalidOutput` | `query` | the answer is not valid JSON for the capability (raw answer, truncated, in `data.raw`) |

## What has been verified

- **Tested against scripted servers only:** the three HTTP adapters (request path, headers and body; response parsing; HTTP errors; timeouts; cancellation) against a local fake server, and every command against the mock provider.
- **The desktop app:** the `ai.*` commands through the app's IPC path (Tauri mock runtime: off by default, `ai.apiKey.set` from the app, the consent error, `ai.preview`, the grant and the answer) and the Markdown renderer's escaping. The Svelte pages and dialogs are checked by `svelte-check` and by hand with the demo provider, not by automated UI tests.
- **Not verified yet:** real OpenAI, GitHub Models, Azure OpenAI, Ollama, LM Studio or vLLM endpoints; TLS behind an intercepting proxy; keychains on Linux desktops without a Secret Service. Reasoning models that reject `temperature` or need other parameters may refuse requests.

## Command reference

<!-- BEGIN GENERATED: ai commands (cargo test -p pbi-ai --test registry; PBIVIEW_BLESS=1 to update) -->

### `ai.status`

**AI status.** Whether AI is enabled, configured providers (never keys), recorded consents, limits and which capabilities are implemented. No network.

_agent-safe; since 0.3.0._

Parameters:

object

Result:

| Field | Type | Description |
|---|---|---|
| `enabled` | boolean | AI features are on. Off by default; nothing is sent while off. |
| `settingsPath`? | string \| null | The settings file (`null` if no location could be determined). |
| `settingsError`? | string \| null | The settings file could not be read (AI then stays off). |
| `providers` | ProviderStatus[] |  |
| `consents` | Consent[] | Recorded consents. |
| `capabilities` | CapabilityInfo[] | The capability contract and what is implemented. |
| `sampleRows` | integer | Most sample rows a request may include (0 = none). |
| `limits` | object | Effective limits per capability. |
| `settings` | AiSettings | The settings as stored (defaults when the file is missing or unreadable). Never holds keys. A settings page edits these and sends them back with `ai.settings.set`. |

Examples:

- Status: `{}`

### `ai.preview`

**Preview an AI request.** The exact request an AI capability would send (key redacted), its data-exposure class and consent status. Never sends anything; works while AI is disabled.

_agent-safe; since 0.3.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `capability` | `"dax.explain"` \| `"dax.generate"` \| `"m.generate"` \| `"report.lint"` \| `"chart.suggest"` | `dax.explain` or `dax.generate` (the others are not implemented). |
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `measure` (optional) | string \| null | `dax.explain`: the measure (`Name`, `[Name]` or `Table[Name]`). |
| `table` (optional) | string \| null | `dax.explain`: the measure's table; `dax.generate`: the target table. |
| `prompt` (optional) | string \| null | `dax.generate`: what the measure should compute. |
| `provider` (optional) | string \| null | Provider id (default: the default provider). |
| `sampleRows` (optional) | integer \| null | `dax.generate`: include this many sample rows of `table`. |

Result:

| Field | Type | Description |
|---|---|---|
| `capability` | `"dax.explain"` \| `"dax.generate"` \| `"m.generate"` \| `"report.lint"` \| `"chart.suggest"` | An AI capability. The id (`dax.explain`, …) is stable. |
| `exposure` | `"metadata"` \| `"sample"` \| `"full"` | What the request contains. |
| `exposureDescription` | string | The exposure class for people. |
| `enabled` | boolean | AI is enabled. |
| `provider`? | ProviderStatus \| null | The provider it would go to (`null` when none is configured). |
| `consent`? | ConsentCheck \| null |  |
| `promptVersion` | string | Prompt templates used. |
| `inputChars` | integer | Characters of the prompt messages. |
| `maxInputChars` | integer |  |
| `request`? | HttpPreview \| null | The exact HTTP request (key redacted); `null` without a provider. |
| `messages`? | ChatMessage[] \| null | The prompt messages, when there is no provider (otherwise they are in `request.body`). |
| `wouldSend` | boolean | It would be sent now. |
| `blockers` | string[] | Why it would not be sent. |

Examples:

- Explaining a measure: `{"capability":"dax.explain","docId":"d1","measure":"Total Sales"}`
- Generating a measure: `{"capability":"dax.generate","docId":"d1","prompt":"Share of completed orders","table":"Orders"}`

### `ai.explainMeasure`

**Explain a measure (AI).** Explains a measure with an AI model: summary, Markdown explanation, filter-context notes, dependencies, caveats. Sends model metadata only; needs AI enabled and consent.

_long-running, agent-safe; since 0.3.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `measure` | string | `Name`, `[Name]` or `Table[Name]`. |
| `table` (optional) | string \| null | The measure's table (when names repeat). |
| `provider` (optional) | string \| null | Provider id (default: the default provider). |

Result:

| Field | Type | Description |
|---|---|---|
| `measure` | string |  |
| `table` | string |  |
| `expression` | string | The measure's DAX. |
| `formatString`? | string \| null |  |
| `summary` | string | One or two sentences. |
| `explanation` | string | The explanation (Markdown). |
| `filterContext` | string[] | Filter context and context-transition notes. |
| `dependencies` | DependencyNote[] | The model's note per dependency. |
| `caveats` | string[] |  |
| `references` | MeasureReferences | What the measure references (computed locally). |
| `markdown` | string | Everything above as one Markdown document. |
| `call` | CallInfo | What was called (part of every capability result). |

Examples:

- Explain a measure: `{"docId":"d1","measure":"Total Sales"}`
- With a named provider: `{"docId":"d1","measure":"Orders[Margin %]","provider":"ollama"}`

### `ai.generateMeasure`

**Suggest a measure (AI).** Natural language → a DAX measure suggestion, parsed locally with diagnostics and optionally evaluated; never written to the model. Needs AI enabled and consent.

_long-running, agent-safe; since 0.3.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `docId` | string | The document id from `doc.open` (`d1`, …). |
| `prompt` | string | What the measure should compute (at most 4000 characters). |
| `table` (optional) | string \| null | Home table of the new measure. |
| `provider` (optional) | string \| null | Provider id (default: the default provider). |
| `evaluate` (optional) | boolean | Evaluate the suggestion locally (unfiltered) when it parses; loads the data on first use. Nothing is written to the model. |
| `sampleRows` (optional) | integer \| null | Include this many rows of `table` (needs `sampleRows` in the settings and a `sample` consent). |

Result:

| Field | Type | Description |
|---|---|---|
| `name` | string | Suggested measure name. |
| `table` | string | Suggested home table. |
| `expression` | string | The DAX expression. |
| `formatString`? | string \| null |  |
| `explanation` | string | How it works (Markdown). |
| `assumptions` | string[] |  |
| `parsed` | boolean | The expression parses as DAX. |
| `diagnostics` | SuggestionDiagnostic[] | Parse errors and unknown references. |
| `value`? | SuggestionValue \| null | With `evaluate`: the value (unfiltered). |
| `applied` | boolean | Always `false`: suggestions are never written to the model. |
| `call` | CallInfo | What was called (part of every capability result). |

Examples:

- Suggest a measure: `{"docId":"d1","prompt":"Share of orders that are completed","table":"Orders"}`
- Suggest and evaluate: `{"docId":"d1","prompt":"Average number of returns per store","evaluate":true}`

### `ai.consent.grant`

**Grant AI consent.** Records a person's consent to send one capability's data (metadata or sample) to one provider endpoint. Not available to agents.

_not offered to agents, writes files; since 0.3.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `provider` | string | Provider id. |
| `capability` | `"dax.explain"` \| `"dax.generate"` \| `"m.generate"` \| `"report.lint"` \| `"chart.suggest"` | An AI capability. The id (`dax.explain`, …) is stable. |
| `exposure` (optional) | `"metadata"` \| `"sample"` \| `"full"` | The most that may be sent (default `metadata`; `full` is refused). |

Result:

| Field | Type | Description |
|---|---|---|
| `consent` | Consent | A recorded permission to send one capability's data of an exposure class to one provider endpoint. |
| `note` | string | What was allowed, for people. |
| `settingsPath`? | string \| null |  |

Examples:

- Explain with a local Ollama: `{"provider":"ollama","capability":"dax.explain"}`
- Generate with sample rows: `{"provider":"openai","capability":"dax.generate","exposure":"sample"}`

### `ai.consent.revoke`

**Revoke AI consent.** Removes recorded consents of a provider (one capability or all). Not available to agents.

_not offered to agents, writes files; since 0.3.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `provider` | string | Provider id. |
| `capability` (optional) | `"dax.explain"` \| `"dax.generate"` \| `"m.generate"` \| `"report.lint"` \| `"chart.suggest"` \| null | Only this capability (default: all of the provider's consents). |

Result:

| Field | Type | Description |
|---|---|---|
| `removed` | integer | Consents removed. |
| `consents` | Consent[] | Consents left. |

Examples:

- All of a provider: `{"provider":"openai"}`
- One capability: `{"provider":"openai","capability":"dax.generate"}`

### `ai.settings.set`

**Change AI settings.** Changes the AI settings (enable, providers, default provider, limits, sample rows); never holds keys. Not available to agents.

_not offered to agents, writes files; since 0.3.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `enabled` (optional) | boolean \| null | Turn AI on or off. |
| `providers` (optional) | ProviderSettings[] \| null | Replace the provider list (keys are never part of it). Consents of removed providers or changed endpoints are dropped. |
| `defaultProvider` (optional) | string \| null | Default provider id (`""` clears it). |
| `limits` (optional) | object \| null | Replace the per-capability limits. |
| `sampleRows` (optional) | integer \| null | Most sample rows per request (0 = none, at most 100). |

Result:

| Field | Type | Description |
|---|---|---|
| `settings` | AiSettings | The AI settings file (`ai.json`). |
| `consentsDropped` | integer | Consents dropped because their provider or endpoint changed. |
| `settingsPath`? | string \| null |  |

Examples:

- Enable with a local Ollama: `{"enabled":true,"providers":[{"id":"ollama","kind":"ollama","model":"qwen2.5-coder:7b"}]}`
- Azure OpenAI: `{"providers":[{"id":"azure","kind":"azureOpenAi","endpoint":"https://contoso.openai.azure.com","deployment":"gpt-4o"}],"defaultProvider":"azure"}`
- Disable: `{"enabled":false}`

### `ai.apiKey.set`

**Store an AI API key.** Stores a provider's API key in the OS keychain. Desktop app only (the key travels in the parameters); use `dashcraft ai login` on the command line.

_not offered to agents, writes files; since 0.3.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `provider` | string | Provider id. |
| `key` | string | The API key. Only accepted from the desktop app. |

Result:

| Field | Type | Description |
|---|---|---|
| `provider` | string |  |
| `changed` | boolean | A key was stored (`set`) or removed (`clear`). |
| `store` | string | `keychain` (or the host's store). |
| `service` | string | Keychain service and account. |
| `account` | string |  |

Examples:

- From the app's settings page: `{"provider":"openai","key":"<key typed by the user>"}`

### `ai.apiKey.clear`

**Remove an AI API key.** Deletes a provider's API key from the OS keychain (DASHCRAFT_AI_API_KEY is not affected). Not available to agents.

_not offered to agents, writes files; since 0.3.0._

Parameters:

| Field | Type | Description |
|---|---|---|
| `provider` | string | Provider id. |

Result:

| Field | Type | Description |
|---|---|---|
| `provider` | string |  |
| `changed` | boolean | A key was stored (`set`) or removed (`clear`). |
| `store` | string | `keychain` (or the host's store). |
| `service` | string | Keychain service and account. |
| `account` | string |  |

Examples:

- Remove the OpenAI key: `{"provider":"openai"}`
<!-- END GENERATED: ai commands -->
