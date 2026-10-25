# Dev log

What changed, why, and the numbers, per wave or merged change. Newest first. Add an entry for
every wave or merged change ([AGENTS.md §9](../AGENTS.md#9-dev-log)); details live in the commits,
the [CHANGELOG](../CHANGELOG.md) and the docs. New entries name their commit by short hash
(`git show <hash>`) once it is known; the entries below predate the current history and carry none.

## 2026-10-24

### CI fix: app clippy on Linux and Windows; Dependabot majors

- **What:** `PendingOpen::set` (`app/src-tauri/src/lib.rs`) is compiled only on macOS/iOS and in
  tests, its only callers, so `dead_code` no longer fails `cargo clippy -p pbiview-app` on Linux and
  Windows. `.github/dependabot.yml` ignores arrow/parquet majors (DataFusion 55.2 is on arrow 59;
  the arrow 60 PR could not build) and TypeScript majors (`svelte-check` 4.7 peers on TS ≤ 6).
- **Why:** `main` CI was red (App jobs on ubuntu-latest and windows-latest), and so was every
  Dependabot PR.

### MIT License and publication

- **What:** the project is licensed under MIT (`LICENSE`; `license = "MIT"` in the workspace and
  fuzz manifests; NOTICE, README, the About page and the installed `licenses/LICENSE`); the
  repository URL is `https://github.com/Jxck-S/dashcraft`; golden rule 17 now says the repository is
  public and every push is reviewed for internal data.
- **Why:** the owner chose MIT (the *Craft projects use MIT OR Apache-2.0) and publishes the source.

### Desktop milestone plan and README refresh

- **What:** new [DESKTOP-PLAN.md](DESKTOP-PLAN.md): 26 milestones (M6 to M31; M27 to M30
  optional) from the viewer to a full Power BI Desktop equivalent, each with numbered steps,
  crates and registry commands, acceptance criteria on the Acme Outfitters example and the
  Microsoft samples, tests, dependencies, blockers and size; a Mermaid dependency graph, a
  summary table, 18 proposed ADRs (0010 to 0027), cross-cutting workstreams (ground truth,
  performance, security, accessibility), an output contract for canonical writers and explicit
  non-goals. ROADMAP "Next" items point at their plan steps. README rewritten for the current
  state, without emoji, with a milestone summary.
- **Why:** the owner asked for the full flow to a Power BI Desktop copy as a plan before more
  implementation.
- **Numbers:** a first draft (17 milestones) was reviewed (8 high, 8 medium, 2 low findings: one
  revisioned document state for edits, queries and saves; the Power Query track no longer waits
  for DAX engine parity; file-format verification depends on report authoring; broad milestones
  split; measurable coverage against pinned inventories; accessibility criteria in every UI
  milestone; sandbox, network and PBIX-writing gates tightened). Docs synced: 47 registry
  commands, 349 M library names, 294 DAX functions, perf numbers from `perf/baseline.json`.
- **Open:** the plan's open questions for the owner (committing Desktop-produced output from the
  synthetic example, the Windows ground-truth machine, PBIX writing, optional milestones,
  network exceptions for update checks and local files, versioning, pilot locale, map data).

## 2026-10-23

### Render verification without goldens, fixes and docs alignment

- **What:** fixtures are the synthetic Acme Outfitters example (generator
  `scripts/examples/gen_acme.py`, checker `scripts/examples/check_acme.py`) and MIT Microsoft
  sample reports (larger ones through `scripts/fetch-samples.sh`). No golden images are committed:
  `crates/pbi-render/tests/golden.rs` compares a page only when
  `reference/golden/<example>/<report>/<page>.png` exists locally (default SSIM threshold 0.85 per
  region and page, no exceptions). Rendering regressions are caught by `insta` render snapshots of
  every Acme page (`render_snapshots.rs`) and the card values of `expected.json` on rendered
  pages; the Microsoft samples render every page with the expected visual counts. Fixes: TMDL ↔
  TMSL dynamic format strings (`formatStringDefinition` object form both ways), one-to-one
  relationships relate unmatched rows to the blank row, the people transform accepts plain name
  lists, `kpi` uses `TrendLine` as the trend axis, matrix rows and columns follow sort-by columns.
  Perf scenarios P03, P06 and P08 run on the example and samples; the app bundle identifier is
  `io.github.jxck-s.dashcraft`; `.gitignore` refuses `*.pbix`/`*.pbit`/`*.abf` outside the
  committed samples and fuzz seeds. Docs: VISUALS "Render verification", GAPS, README,
  `reference/README.md`, AGENTS (rule 11, §7), CHANGELOG.
- **Why:** the repository holds only synthetic and licensed public test data, and the docs must
  describe the verification that actually runs.
- **Numbers:** `check_acme.py` 445/445 checks pass; 39 card values asserted on rendered pages; 8
  render snapshots. GAPS: visuals 2.4 → 2.1 (eight rows 4 → 3: no Power BI render to compare
  with), rendering fidelity 3.4 → 3.2 (4 / 0 / 2; the golden images row is ❌ 0).
- **Open:** export the Acme pages from Power BI Desktop or the service into
  `reference/golden/acme-outfitters/<report>/<page>.png` to verify fidelity.

### Synthetic example and public samples replace the previous fixtures

- **What:** tests, docs and examples move to the synthetic **Acme Outfitters** example
  (`reference/examples/acme-outfitters/`: PBIP with a TMDL model and three PBIR reports, CSV data,
  `blob-map.json`, expected values in `EXPECTED.md` / `expected.json`, generated by
  `scripts/examples/gen_acme.py`) and MIT-licensed Microsoft sample reports
  (`reference/samples/microsoft/`, larger ones through `scripts/fetch-samples.sh` into the
  gitignored `reference/samples/downloads/`); docs describe features generically. AGENTS.md
  gains golden rule 2 (never commit or describe real or internal reports or data) and
  a new fixture table (§7).
- **Why:** the repository must hold only synthetic and licensed public test data.
- **Open:** the crate test migrations run in the same wave (formats, engine and session, render
  and perf, front ends); the P03, P06 and P08 perf baselines need re-recording on the new data.

### Docs pass, rename leftovers and scatter tooltips (uncommitted at the time of writing)

- **What:** the last user-facing "pbiview" strings say DashCraft: the unsupported-DAX hint of
  refresh and derived-column statuses (one `DAX_UNSUPPORTED_HINT` in `pbi-session`), the
  derived-column description, the lint rules title, the CLI and `pbi-ops` temporary-file prefixes
  (`.dashcraft-*.tmp`) and worker thread name, `CallerKind` docs. `canvas.tooltip` on a scatter
  point adds the X and Size values around Y (after Details and Legend). Docs: GAPS re-scored,
  README visual matrix and features, the `pbiview-app → pbi-ai` edge in ARCHITECTURE and AGENTS
  §2 (`cargo xtask layers`: 15 crates, 50 edges, ok), ROADMAP "Next" follow-ups, the Linux package
  name decision in [packaging.md](packaging.md#linux-package-name-dash-craft-decided) (`dash-craft`
  stays: Tauri 2.12 derives it from `productName` and has no option for it).
- **Why:** rename follow-up (ADR-0009) and `docs-sync` after the visuals, engine, DAX, AI-in-app
  and conditional-formatting commits.
- **Numbers:** GAPS averages: DAX 2.8 → 3.3, engine 2.6 → 3.4, visuals 2.3 → 2.4 (28 rows),
  registry/MCP/AI 2.6 → 2.7, app UX 1.2 → 1.4. Registry: 47 core commands (all agent-safe, with
  `security.roles`/`security.viewAs`) plus 9 `ai.*`. New tests: scatter tooltip lines (pbi-ops
  `canvas`), the DAX hint text (pbi-session). No snapshot changed.
- **Open:** `docs/VISUALS.md` still says scatter tooltips lack X and Size (owned by the render
  wave); doc comments that mention `pbiview run`/`serve` in `pbi-ops`, `pbi-m`, `pbi-engine`,
  `pbi-model`, `pbi-pbir` and the app icon's SVG comment are developer-facing and left.

### Conditional formatting and dynamic format strings

- **What:** `pbi_render::conditional`: table and matrix background/font colors (2- or 3-color
  gradients, rules with ranges, percent-of-range and blanks, field values), data bars, built-in
  icons as vector shapes, values vs totals; conditional `dataPoint.fill` for bar, column, pie,
  donut, treemap, scatter and funnel without a legend field. Rule inputs outside the visual are
  queried as hidden fields; unsupported variants render plain with a warning. Dynamic format
  strings per cell through `EvalContext::evaluate_measure_format_strings` (one batch per measure
  and result set) in cards, tables, matrices, chart labels and tooltips.
- **Why:** GAPS §5 (conditional formatting was 0) and the calculation-group format strings that
  visuals still formatted statically.
- **Numbers:** 11 end-to-end tests (`crates/pbi-render/tests/conditional_format.rs`) plus 12 unit
  tests; golden SSIM unchanged; P02 and P07 flat.

### Network outage and restart

- A network outage stopped the running agents mid-wave. They were restarted from the tree and
  this log; nothing on disk was lost. Two agents that had not written anything yet started over.

### Opt-in AI in the desktop app

- **What:** the app installs `pbi_ai::AiHost` (`ai.json` in its configuration folder or
  `DASHCRAFT_CONFIG_DIR`) and runs `pbi_ai::catalog()`; File → Options → AI (on/off, providers,
  write-only keychain keys, default provider, sample rows, consents with Revoke); a consent dialog
  showing the exact `ai.preview` request; Explain with AI (Data pane, formula bar, Properties,
  ribbon) and Suggest a measure (never applied). The typed-client generator moved to
  `crates/pbi-ai/tests/ops_ts/` (covers `ai.*`); debug-only `DASHCRAFT_AI_MOCK=1` demo provider.
- **Why:** ROADMAP `ai-layer` (the app UI was the open part); GAPS §9.
- **Numbers:** 8 hostile-input tests for the Markdown renderer (`app/tests/markdown.test.ts`).

### DAX library 218 → 294 functions

- **What:** logarithms, exponentials, trigonometry and hyperbolics, `FACT`/`COMBIN`/`PERMUT`/`GCD`/
  `LCM`/`QUOTIENT`, rounding to multiples, deterministic `RAND`/`RANDBETWEEN`, bit functions,
  `GEOMEAN(X)`, `MAXA`/`MINA`/`AVERAGEA`, the `NORM`/`EXPON`/`POISSON`/`BETA`/`CHISQ`/`T`/
  `CONFIDENCE` distributions, `YEARFRAC`, `NETWORKDAYS`, `IF.EAGER`, `ERROR`, `EVALUATEANDLOG`,
  `USERCULTURE`, `ISAFTER`/`ISONORAFTER`; fixes for `EDATE` and `QUARTER(BLANK())`.
- **Why:** GAPS §3 "function library breadth" (the missing math/stat functions).
- **Numbers:** 294 functions (271 supported, 23 partial); distributions within 1.2e-13 relative
  error of 122 SciPy references; a 31,600-call random-argument sweep with no panic or non-finite
  result.

## 2026-10-22

### Calculation groups, many-to-many relationships, RLS "View as"

- **What:** typed `CalculationGroup`/`CalculationItem` (lossless TMDL and TMSL) with
  `SELECTEDMEASURE`, `SELECTEDMEASURENAME`, `ISSELECTEDMEASURE`, `SELECTEDMEASUREFORMATSTRING`,
  precedence, selection expressions and dynamic format strings (fast paths fall back when an item
  could apply); many-to-many as limited relationships (`ENG003` now informational);
  `Session::set_view_as` / `set_effective_roles` apply role filters on a restricted engine copy
  with `USERPRINCIPALNAME`/`USERNAME`/`USEROBJECTID`/`CUSTOMDATA`, failing closed.
- **Why:** GAPS §4 rows at 0–1 (calculation groups, M:M, RLS).
- **Numbers:** new tests: `pbi-tmdl` `calc_groups` (4), `pbi-dax` `model_features` (10),
  `pbi-engine` `m2m_security` (5), `pbi-session` `security` (5). The registry commands and the app
  dialog for "View as" followed on 2026-10-23.

### Scatter, funnel, gauge, treemap and waterfall visuals

- **What:** `scatterChart` (Details × Legend points, numeric axes, area-proportional bubbles,
  labels, first play-axis frame), `funnel` (conversion-rate labels), `gauge` (min 0 / max 2 ×
  value defaults, target needle, callout), `treemap` (squarified, nested details), `waterfallChart`
  (sentiment colors, connectors, total, breakdown top N + Other; sorted by category), with hit ids
  on every mark.
- **Why:** GAPS §5 and the ROADMAP "more visual types" item.
- **Numbers:** 19 tests in `crates/pbi-render/tests/visuals_more.rs` (edge cases, determinism,
  the per-visual cache); golden SSIM unchanged.

### Rebrand to DashCraft and docs sync

- **What:** product name **DashCraft** ([ADR-0009](adr/0009-naming-dashcraft.md), root
  [`NOTICE`](../NOTICE)). `pbiview-cli` is now a library (`run_main`) plus two binaries,
  `dashcraft` and the alias `pbiview` (usage, `--version` and messages use the invoked name). The
  app is `DashCraft` / `dashcraft-app` with identifier `io.github.jxck-s.dashcraft`.
  `DASHCRAFT_APP` / `DASHCRAFT_BLOB_MAP` with the `PBIVIEW_*` names as aliases; MCP
  `serverInfo.name` and `_meta` prefix `dashcraft`; Linux MIME types, packaging files, release
  artifact names and the license-inventory titles renamed. `CommandMeta::invokes_other_commands`
  (set on `commands.batch`) replaces the MCP special-case list. `roxmltree`, `sha2` and `toml`
  moved to `[workspace.dependencies]`. Docs synced: v0.2.0 released, golden SSIM results, 336 M names, `pbi-ops`/`pbi-ai` and ADR-0007–0009 in the README and crate graphs (the
  app depends on `pbi-ops`, not on `pbi-render`/`pbi-engine`), ADR-0007 status, `COMMANDS.md`
  links `MCP.md`, `inspect_model.rs` paths, macOS Finder opens, GAPS re-scored,
  `THIRD_PARTY_NOTICES.md` regenerated (it predated the `pbi-ai` dependencies).
- **Why:** the owner chose a public name without Microsoft's mark; `docs-sync` after the
  registry, MCP, AI and app waves.
- **Numbers:** JSON outputs, schemas and `inspect --json` snapshots unchanged (no snapshot
  updates); new tests: the `pbiview` alias, `DASHCRAFT_APP` precedence, blob-map variable
  precedence, five migration tests, the invokes-others hint test. GAPS: registry/MCP/AI 1.4 → 2.6,
  performance 4.1 → 4.9, DAX 2.6 → 2.8, engine 2.4 → 2.6. Registry: 45 core commands (all
  agent-safe) plus 9 `ai.*` (4 agent-safe).
- **Open:** the deb package name becomes `dash-craft` (Tauri kebab-cases the product name);
  NSIS/macOS/Linux installs of pbiview are not replaced; a trademark search before the first
  public release; user-facing "pbiview" strings left in crates other agents own this wave (see
  the wave report).

### Perf baseline after the render cache and DAX fast paths

- `perf/baseline.json` re-recorded on an idle M4 Pro: all 8 scenarios `ok`. P01 25.6 → 0.68 ms
  (p95), P02 13 567 → 97 ms, P04 114 → 22 ms, P08 1.61 → 0.50 ms ([perf.md](perf.md)).

### CLI wiring of the AI commands

- `run`, `serve` and `mcp` build their registry with `pbi_ai::catalog()` (`mcp` exposes only the
  agent-safe AI commands; `commands --markdown` stays the core catalog); `pbiview ai login|logout`
  stores or deletes a provider key in the OS keychain (no echo, `--stdin`, never argv).

### The app on the command registry

- **What:** `AppState` holds the `Registry` and one `Workspace`; every domain operation runs
  through `ops_execute` → `Registry::execute` with progress and cancel; typed TS client
  `app/src/lib/ops.gen.ts` generated from the JSON Schemas (staleness test). The app's canvas,
  grid, refresh and DTO copies are gone; render ordering and legacy report themes moved into
  `pbi_ops::canvas`. New commands `report.filters`, `text.previewPeople`; File → Export writes
  PBIP/PBIT.
- **Numbers:** about 4,450 lines of app code removed (net, 52 files).

### Opt-in AI layer (`pbi-ai`)

- **What:** capability contract (`dax.explain`, `dax.generate` implemented; others
  `unsupported`), OpenAI-compatible / Azure OpenAI / Ollama providers (`ureq` + rustls) and a
  scripted mock, consent per provider endpoint, capability and data class, `ai.preview`, keys
  only in the keychain or `DASHCRAFT_AI_API_KEY`; consent/settings/key commands not agent-safe;
  `cargo xtask layers` contains HTTP/TLS/keychain crates. ADR-0008, docs/AI.md.

### MCP server

- **What:** `pbi_ops::mcp` generated from the agent catalog (no per-tool code): tools with input
  and output schemas and hints, structured content, PNG renders as images, progress,
  cancellation, logging; protocol 2024-11-05 to 2025-11-25 and stateless 2026-07-28; read/write
  roots (default read root the current folder), `--allow`/`--deny`. Also AGENTS.md, GAPS.md and
  this dev log.

### One-pass grouped `SUMMARIZECOLUMNS`, columnar iterators

- **What:** plain aggregations group the visible fact rows in one pass (bit-identical results);
  `SUMX`/`AVERAGEX`/`MINX`/`MAXX`/`COUNTX`/`COUNTAX` compile simple row expressions over column
  dictionaries; general path: per-column value indexes for `IN`, relationship-index propagation;
  engine `GroupIndex`, `aggregate_groups` (additive). Differential tests fast vs general.
- **Numbers:** P02 13.6 s → 0.1 s, P04 114 → 22 ms.

### Agent guide, gaps scorecard and this dev log (uncommitted at the time of writing)

- **What:** `AGENTS.md` becomes the canonical guide for people and AI agents (layer map, golden
  rules, parallel-agent protocol, validation cheat-sheet, picking work, definition of done,
  naming and trademark policy); `.github/copilot-instructions.md` now points to it.
  [GAPS.md](GAPS.md) scores 12 areas against Power BI Desktop; this file seeds the history.
- **Why:** many agents work in parallel; the *Craft research
  ([artcraft-infra-review.md](research/artcraft-infra-review.md),
  [photocraft-review.md](research/photocraft-review.md)) recommends golden rules, a "picking work"
  order, a dev log and a gaps scorecard.
- **Numbers (counted from the code):** 210 DAX functions (187 supported, 23 partial); 336 M
  library names (268 native functions); 43 registry commands, all agent-safe; 14 fuzz targets;
  1 038 `#[test]`/`#[tokio::test]` functions (grep) across the workspace and the app.
- **Open:** `cargo xtask layers` fails because `pbi-ai` (added in this wave) is in no layer;
  `perf/baseline.json` still holds the pre-cache P01/P08 numbers; P02 is over its `max_ms` in the
  baseline (an uncommitted DAX fast-path change reports 13.6 s → 0.1 s).

### Workspace dependencies for `pbi-ops`, `pbi-ai` stub, `ureq`, `keyring`

- `pbi-ops` and `pbi-ai` in the root `[workspace.dependencies]` (`pbiview-cli` now takes `pbi-ops`
  from there); `pbi-ai` is an empty stub for the opt-in AI layer; `ureq` (rustls) and `keyring`
  reserved for it.

### Command registry (`pbi-ops`, ADR-0007)

- **What:** 43 typed, JSON-Schema-described commands with a `Workspace`, cancellation, progress,
  caller kinds and a `PathPolicy` sandbox; `pbiview commands`, `pbiview run` (incl. `--batch`),
  `pbiview serve --stdio` (JSON-RPC 2.0, `$/cancel`); `docs/COMMANDS.md` generated with a
  staleness test.
- **Why:** one contract for the CLI, RPC, the app and agents (MCP next) instead of per-surface
  copies.
- **Numbers:** CLI snapshot output unchanged; `inspect`/`detect` DTOs moved into `pbi-ops`.

### Per-visual render cache, O(1) scroll steps, Carlito for Calibri

- **What:** content-hashed per-visual render cache (LRU); grid model and layout reused across
  scroll steps; Carlito (OFL) bundled as the Calibri substitute.
- **Numbers:** P01 (50k-row scroll step, p95) 25.6 → 0.75 ms; P08 (report-page scroll step)
  1.61 → 0.49 ms; golden page SSIM up, a Calibri textbox back to the standard 0.85 threshold.
  `perf/baseline.json` was not re-recorded.

### Performance budgets and layer checks

- **What:** `cargo xtask perf` (P01–P08 in `perf/budgets.toml`, per-machine baseline, 20 % / 1 ms
  regression rule) and `cargo xtask layers` (7-layer crate map, UI toolkit only in the app); CI
  gates layers, perf is informational.
- **Numbers (M4 Pro baseline):** P02 first render of a 50k-row table 13.6 s (quadratic group +
  sum), P01 scroll 25.6 ms, P04 `SUMX` over 1M rows 114 ms; fixes queued
  ([perf.md](perf.md#fixed-misses)).
- Added the `xtask` crate (`cargo xtask` alias) and the `schemars`/`base64` deps.

### Research reviews of the *Craft apps

- PhotoCraft and the *Craft family, the ArtCraft umbrella and shared infrastructure, office-style
  and media apps ([docs/research/](research/)). Inputs to ADR-0007, the perf budgets, the layer
  check and this guide.

### 14× faster table scrolling

- **What:** shaping cache keyed by (text, size, family, weight, italic, wrap width); grid layout
  memoized by a content fingerprint; `bench_scroll` example.
- **Numbers:** small-table scroll step ~86 ms → ~6 ms (debug build); first render unchanged.

### Phase 2 and M5 → `v0.2.0`

- Formats: VPAX, Power Pivot model **and data** (MS-XCA Plain LZ77), RDL outline,
  `.pq`/`.pqt`, dataflows, `.pbiviz`/`.mez`/`.pqx` metadata (never executed).
- Visuals: textbox, shapes, buttons, cards, slicers, table, matrix, pie/donut, bar,
  column, line, area, combos.
- Power BI Desktop-style shell (read-only); notices (Fluent icons, Selawik).
- `pbiview render` and golden-image tests: per-region SSIM ≥ 0.85 with documented lower
  thresholds for font-limited regions; card values asserted on the rendered output.
- Interactive canvas: slicers, cross-filter per `visualInteractions`, tooltips,
  scrolling, matrix expand/collapse, navigation buttons, cancellable renders (ADR-0006).
- Packaging: app icon, dmg/msi/nsis/AppImage/deb, file associations, release workflow,
  SBOMs, signing gated on secrets.
- Docs: README, FORMATS, VISUALS, USER-GUIDE, ARCHITECTURE, CLI, ADR-0006.
- macOS Finder opens reach the running app; version 0.2.0, tag `v0.2.0`.

## 2026-10-21

### Phase 2 start (renderer and DAX expansion)

- `pbi-render` module skeleton for parallel work; visuals framework;
  the `render_page` example (output under `target/tmp`).
- Theme, layout, text (Selawik), scene → SVG → PNG, visual query engine
  (`SUMMARIZECOLUMNS` → DataViews); card values match the expected values.
- DAX grows by ~100 functions (`ALLSELECTED`, time intelligence, window functions,
  `USERELATIONSHIP`/`CROSSFILTER`, statistics; `FUNCTIONS.md`); engine relationship overrides;
  session derived columns; PBIP `.pbi/cache.abf` data; CSV formula-injection guard.

### Phase 1 → `v0.1.0`

- **Gate:** fmt clean; clippy `-D warnings` on the whole workspace including the app; 696 tests
  pass (59 suites); a clean clone without the optional data passes 676 (those tests skip);
  cargo-deny licences/bans/sources clean; app `svelte-check` and build clean.
- App data grid, refresh with progress, measures strip, "Load CSV manually…", people
  transform; exports (CSV, Parquet, model JSON/Markdown) and headless `refresh`,
  `measures`, `export` (`measures --report` applies a report's filters).
- Fuzzing: 14 `cargo-fuzz` targets; 4 bugs fixed (a drive-letter zip-slip bypass, TMSL
  UTF-16 detection and key placement, M escape spans).
- `pbi-session` refresh pipeline and query API; the Phase 1 acceptance values (row
  counts, measures under each report's filters) pass.
- App explorer wired to `pbi-core`; linter (17 rules, including stale
  `queryRef`s); CLI `inspect`/`detect`/`open`/`convert` with `insta`
  snapshots.
- DAX evaluator (~120 functions, filter/row context, context transition) and the people
  field transform.

### M1 and M3 foundations

- `pbi-core` safe containers, detection and readers into one `Document`, secret
  redaction, `save_as_pbip` + `export_pbit`; `pbi-vertipaq` ABF + `metadata.sqlitedb` + VertiPaq
  decoding: every decoded table matches pbixray (hash-verified).
- M evaluator, value model, ~200 library functions, offline `BlobResolver`; the fixture's
  CSV-backed partitions load with the expected row counts.
- Wave 2: lossless TMDL (byte-identical), TMSL, PBIR (every fixture file written
  byte-identical), Arrow storage engine with filter propagation and DataFusion SQL; dev profile
  without dependency debuginfo (`target/` had reached 18 GB).
- Wave 1: `pbi-model` (model, report AST, commands with undo/redo), M and DAX parsers,
  clean-room XPress9 decoder (output SHA-256 matches pbixray), Tauri 2 + Svelte 5 app scaffold,
  CI (3-OS matrix, cargo-deny, release, fuzz), ADRs 0001–0005.

### M0

- Cargo workspace with 12 crates (`pbi-session` added to break the dax/engine/refresh cycle),
  Rust 1.99.1, workspace lints, Arrow 59.2 / DataFusion 55.2; `reference/` fixture root with
  optional data gitignored; offline `blob-map.json`.
