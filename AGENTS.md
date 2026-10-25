# AGENTS.md: working on DashCraft

Read this file first in every session, then the docs it points to. It is the canonical guide for
people and for AI coding agents (Copilot CLI, Claude, Codex, …). `CLAUDE.md`-style files and
`.github/copilot-instructions.md` only add tool-specific notes; **if they disagree, this file wins**.

## 1. What this is

DashCraft is a clean-room, offline, cross-platform (macOS, Windows, Linux) Rust app and CLI that
opens Power BI files (PBIP, PBIX, PBIT and related formats): readers, model tree, M and DAX
engines, data grid, exports, linter, report renderer (SVG/PNG) and an interactive canvas in a
Power BI Desktop-style shell. v1 is read-only; the architecture is built to grow into a Rust
reimplementation of Power BI Desktop ([ADR-0004](docs/adr/0004-desktop-ready-architecture.md)).

- Status: Phase 1 (Data Explorer) released as `v0.1.0`; Phase 2 (Report Renderer), M5 formats,
  docs and packaging released as `v0.2.0` (installers unsigned until certificates exist). Since
  then (`[Unreleased]`): the command registry, `run`/`serve`/`mcp`, the opt-in AI layer (also in
  the app), the app on the registry, perf budgets, the DashCraft name, scatter/funnel/gauge/
  treemap/waterfall visuals, conditional formatting, calculation groups, many-to-many
  relationships, RLS "View as" and 294 DAX functions. Details:
  [docs/ROADMAP.md](docs/ROADMAP.md), [CHANGELOG.md](CHANGELOG.md), [docs/GAPS.md](docs/GAPS.md).
- Public product name: **DashCraft** (§10, [ADR-0009](docs/adr/0009-naming-dashcraft.md); the
  project was called `pbiview`). The CLI binary is `dashcraft` with a `pbiview` alias, the app
  binary `dashcraft-app` (identifier `io.github.jxck-s.dashcraft`), user-facing env vars `DASHCRAFT_*` with
  `PBIVIEW_*` aliases. Crate names stay `pbi-*` / `pbiview-*`; developer and test variables keep
  their names (`PBIVIEW_REFERENCE`, `PBIVIEW_BLESS`, `PBIVIEW_TEST_RECORD`).

### Five-minute orientation

| Read | For |
|---|---|
| [README.md](README.md) | Front door: what DashCraft is, screenshots, quick start; details in [docs/FEATURES.md](docs/FEATURES.md) |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Pipeline, crate graph, responsibilities, extension points |
| [docs/ROADMAP.md](docs/ROADMAP.md) | Milestones M0–M5, the "Next" list, open human decisions |
| [docs/GAPS.md](docs/GAPS.md) | Gaps scorecard: what is missing, scored 0–5 per area |
| [docs/DEVLOG.md](docs/DEVLOG.md) | What changed per wave, why, and the numbers |
| [docs/adr/](docs/adr/README.md) | Decisions 0001–0009; new decisions get a new ADR |
| [reference/README.md](reference/README.md) | Test fixtures: the synthetic Acme Outfitters example, the Microsoft samples, optional local data (§7) |

## 2. Layer map and where things live

Dependencies point **down** only. `cargo xtask layers` (`xtask/src/layers.rs`) enforces it in CI:
no upward edge, no unlisted same-layer edge, every crate in a layer, `pbi-model` and `pbi-m` free
of workspace dependencies, UI-toolkit crates (`tauri*`, `wry`, `tao`, `specta*`, `egui*`, …) only
in `pbiview-app`, HTTP/TLS and keychain crates (`ureq`, `rustls`, `keyring`, …) only in `pbi-ai`
and the crates that depend on it (`pbiview-cli`, `pbiview-app`).

| Layer | Crates | Owns |
|---|---|---|
| L6 front ends | `pbiview-cli` (binaries `dashcraft` and its alias `pbiview`: `inspect`, `detect`, `open`, `convert`, `lint`, `refresh`, `measures`, `export`, `render`, `commands`, `run`, `serve`, `mcp`, `ai`), `pbiview-app` (`app/src-tauri`, binary `dashcraft-app`; workspace member, not a default member) | Adapters only: argument parsing, IPC (`ops_execute` → the registry, app-only views), UI |
| L5 operations | `pbi-ops` (command registry, MCP server; ADR-0007), `pbi-ai` (opt-in AI layer, `ai.*` commands; ADR-0008) | Every capability as a typed command; workspace of open documents; path policy; JSON-lines RPC; MCP |
| L4 render | `pbi-render` | Theme, layout, text, semantic query → DataView, visuals → scene graph → SVG/PNG, golden-image comparison |
| L3 session | `pbi-session` | Open → refresh (M / VertiPaq / `.pbi/cache.abf` → calculated columns → calculated tables → derived columns), CSV loading, query façade, exports. Breaks the dax ↔ engine cycle |
| L2 DAX, readers | `pbi-dax`, `pbi-core` (allowed: `pbi-core` → `pbi-dax`) | DAX lexer, parser, evaluator (filter and row context, context transition, BLANK); content detection, safe containers (dir/ZIP caps, zip-slip guard), readers → `Document` (incl. VPAX, Power Pivot, RDL, mashups, extensions), PBIP/PBIT writers, linter |
| L1 formats, M, storage | `pbi-tmdl`, `pbi-tmsl`, `pbi-pbir`, `pbi-m`, `pbi-vertipaq`, `pbi-engine` | Lossless TMDL (lexer → CST → model, writer), TMSL (`model.bim`, PBIT `DataModelSchema` UTF-16LE), PBIR/PBIR-Legacy/PBIX `Layout` types, semantic-query expressions, themes; M lexer, parser, lazy evaluator (`BlobResolver`, `Clock` traits); XPress9, ABF, `metadata.sqlitedb`, VertiPaq IDF/dictionary/HIDX → Arrow; Arrow store, relationships, filter propagation, DataFusion, async + cancellation |
| L0 model | `pbi-model` | TOM-like model, report AST, `ObjectId`s, `Document` + `Command`/undo |
| tooling | `xtask` (`cargo xtask layers`, `cargo xtask perf`) | No crate may depend on it |

Current edges: `cli → ops, ai, core, session, render, model`; `app → ops, ai, core, session, m,
tmdl, model`; `ai → ops, session, dax, engine, model`; `ops → core, session, render, engine, model`; `render → session, dax, engine, pbir,
model`; `session → core, m, dax, engine, vertipaq, model`; `core → tmdl, tmsl, pbir, vertipaq, dax,
m, model`; `dax → engine, model`; `engine, tmdl, tmsl, pbir, vertipaq → model`; `m, model → none`.
Never add an upward or cyclic edge: put orchestration in `pbi-session` or `pbi-ops`. When an edge is
intended, change `xtask/src/layers.rs` and the graph in `docs/ARCHITECTURE.md` together.

| Path | What |
|---|---|
| `crates/<crate>/` | Library crates above; `crates/pbi-dax/FUNCTIONS.md` (DAX coverage, checked by tests), `crates/pbi-core/src/lint/RULES.md` (17 lint rules), `crates/pbi-vertipaq/FORMAT-NOTES.md` and `REFERENCES.md` |
| `app/` | Tauri 2 + Svelte 5 + TypeScript front end; [app/README.md](app/README.md), [app/docs/desktop-ui-reference.md](app/docs/desktop-ui-reference.md) (every Desktop element and what we do with it) |
| `xtask/`, `perf/` | Repo automation; perf scenarios (`budgets.toml`), per-machine `baseline.json`, shared `harness.rs` |
| `fuzz/` | Own Cargo workspace, 14 `cargo-fuzz` targets, seeds, dictionaries, `run.sh` |
| `reference/` | Fixtures (`$PBIVIEW_REFERENCE` overrides; §7): `examples/acme-outfitters/` (synthetic PBIP + CSV; its `blob-map.json` maps `AzureStorage.Blobs` URLs to the local CSVs), `samples/microsoft/` (MIT samples); `reference/README.md` says which test uses what |
| `docs/` | `ARCHITECTURE`, `ROADMAP`, `GAPS`, `DEVLOG`, `CLI` (every flag and JSON schema), `COMMANDS` (generated), `USER-GUIDE`, `VISUALS` (per-visual support, render verification), `perf`, `ci` (CI, release, fuzzing), `packaging`, `MCP` (MCP server), `AI` (opt-in AI), `adr/`, `research/` (*Craft reviews) |
| `FORMATS.md` | Format support matrix; update when a reader or data path changes |
| `THIRD_PARTY_NOTICES.md`, `deny.toml` | Attribution for references and bundled assets (license inventory from `scripts/license-inventory.py`); `cargo-deny` license/advisory/source policy |
| `scripts/` | License inventory, changelog section extraction for releases, icon generation, Linux packaging, `examples/` (generator and checker of the Acme Outfitters example), `fetch-samples.sh` (larger Microsoft samples) |

## 3. Golden rules

1. **Stay clean-room.** No verbatim third-party code, comments or non-trivial tables. Permissive
   (MIT, Apache-2.0, BSD, CC0) references may be structurally ported only with attribution in
   `crates/pbi-vertipaq/REFERENCES.md` and `THIRD_PARTY_NOTICES.md` ([ADR-0003](docs/adr/0003-vertipaq-xpress9-cleanroom.md)).
   Don't read copyleft or unknown-licence sources. No disassembly, decompilation or debugging of
   Microsoft binaries. The storytold/ArtCraft (*Craft) repositories are inspiration for patterns
   only; the ones without a licence are strictly read-only: never copy or vendor from them.
2. **Never commit or describe real or internal reports or data.** Tests, fixtures, docs, examples,
   comments, snapshots and commit messages use only the synthetic Acme Outfitters example and
   licensed public samples (§7). No names, numbers, screenshots, storage URLs or observations
   taken from a real organisation's report, model or data, and no statement about who uses
   DashCraft or for what.
3. **Never commit private data.** Gitignored: `reference/golden/`, `reference/private/`,
   `reference/samples/downloads/`; also renders, heatmaps, exports and perf reports made from
   them (keep them under `target/`). Tests that need an optional file print
   `SKIP: <file> missing` and return successfully, so CI on a fresh clone stays green. Scan
   staged files before every commit (§8).
4. **Never read, copy or commit credentials**: the `azpbi/` folder (az login tokens), MSAL token
   caches, `.env`, `*.sess`, SAS tokens, keys. Credentials go only through the OS keychain.
5. **Offline-first, no telemetry.** No network access in tests, in M (its only external path is the
   `BlobResolver`) or in any crate except `pbi-ai`. AI is strictly opt-in, with explicit user consent
   and nothing sent without it ([ADR-0008](docs/adr/0008-ai-layer.md) and [docs/AI.md](docs/AI.md)).
6. **Never execute embedded content**: scripts, R/Python visuals, `.pbiviz`, `.mez`, `.pqx`. Read
   metadata or list only. Web URL buttons are shown, never opened. `SecurityBindings` is opaque (DPAPI).
7. **Never panic on untrusted input.** Parsers and decoders return `Result`, check lengths and
   offsets, use checked arithmetic on sizes read from files and cap allocations. Containers cap
   total size, entry count and compression ratio and refuse absolute, `..` and drive-letter paths
   (zip-slip). No `unsafe` (the workspace denies it); no `unwrap`/`expect`/`panic!`/`todo!`/`dbg!`
   outside tests (clippy warns; CI denies warnings). A new parser gets a fuzz target.
8. **Every new capability is a registry command first** (`pbi-ops`,
   [ADR-0007](docs/adr/0007-command-registry.md)). The CLI subcommands, `dashcraft run`/`serve`, MCP
   (`dashcraft mcp`, [docs/MCP.md](docs/MCP.md)) and the app are thin adapters over
   `Registry::execute`.
9. **Respect the layers** (§2). UI-toolkit crates only in `pbiview-app`; network and keychain crates
   (`ureq`, `keyring`) only in `pbi-ai` and the front ends that depend on it (`cargo xtask layers`
   checks both rules).
10. **Don't touch the root `Cargo.toml` `[workspace.dependencies]`** unless you are the orchestrator or
    the one agent designated for it in this wave; report the needed change instead. Crates take
    versions from there (`{ workspace = true }`; the app pins its Tauri crates in
    `app/src-tauri/Cargo.toml`) and set `[lints] workspace = true`.
11. **Don't lower the golden SSIM threshold** (0.85 per region and page, no per-page exceptions)
    without inspecting the heatmaps in `target/tmp/golden/` and documenting the cause in
    `crates/pbi-render/tests/golden.rs` and `docs/VISUALS.md`. Never weaken expected-value asserts;
    accept a render snapshot change only after reading its diff and only when it is intended.
12. **Don't regress performance.** `cargo xtask perf` budgets and the baseline must not get worse
    (no new `FAIL`, no regression). Don't raise a `target_ms` to turn a scenario green; lower
    `max_ms` when a fix lands ([docs/perf.md](docs/perf.md)).
13. **Keep rendering deterministic**: bundled fonts only (Selawik for Segoe UI, Carlito for Calibri;
    OFL, unmodified), no system fonts, no wall clock or randomness in output (`--now`), the same
    bytes on every OS.
14. **Never drop data silently.** Unknown fields stay round-trippable; mutations go through
    `Document` commands, never direct struct edits from the UI or CLI.
15. **Never bypass the gates**: `cargo fmt`, `cargo clippy … -D warnings`, tests, `cargo xtask
    layers`. No blanket `#[allow]`, no skipped hooks, no deleted tests to get green.
16. **Commit trailer** on every commit (agents commit only when asked; the orchestrator commits waves):
    `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>`.
17. **Public repository rule.** This repository is public (`github.com/Jxck-S/dashcraft`, MIT
    License). Everything committed is published: it must never contain internal data (rule 2): no
    internal IDs, URLs, tenant, workspace or storage names, report data, or commit messages naming
    them. Review the staged diff for that before every push.

## 4. Architecture decisions (summary)

- **Desktop-ready** (ADR-0004): TOM-like mutable model with stable `ObjectId`s, `lineageTag`,
  annotations and `extra` bags; lossless TMDL, TMSL and PBIR read **and write** (TMDL
  byte-identical, PBIR and TMSL JSON value-equal); `Document` + `Command` (apply/undo) with change
  events; real M and DAX engines; async, cancellable engine APIs (`CancellationToken`).
- **UI** (ADR-0001): Tauri 2 + Svelte 5 + TypeScript, `tauri-specta` typed IPC. The webview only
  displays Rust output and handles interaction. The shell mirrors Power BI Desktop; unsupported
  commands stay visible but disabled.
- **Data engine** (ADR-0002): Arrow 59.2 + DataFusion 55.2. `pbi-dax` (formula engine) owns DAX
  semantics and asks `pbi-engine` (storage engine) for row sets, group-bys and aggregates over
  Arrow; DataFusion serves SQL (grid filters, `Session::sql`).
- **XPress9 / VertiPaq** (ADR-0003): clean-room Rust decoders, no encoders; `pbixray` (MIT) and
  Microsoft Open Specifications are references only. Shipping PBIX data decoding needs the legal
  review of the UNVERIFIED rows.
- **Renderer** (ADR-0005): semantic query → Power BI-shaped DataView → built-in Rust visuals →
  scene graph → SVG (app) and PNG via `resvg` (CLI, golden tests). DIN has no open
  metric-compatible font.
- **Interaction state** (ADR-0006): slicer selections, cross-filters, scroll and expansion are view
  state (`InteractionState`, `PageViewState`) per page, not `Document` commands.
- **Command registry** (ADR-0007): typed params (`deny_unknown_fields`, camelCase) and outputs
  with generated JSON Schemas, `CommandMeta` (`agent_safe`, `mutates_document`, …), `Workspace`,
  `PathPolicy`, `OpError` codes. Ids, parameter names and result fields are a public contract.
- **AI layer** (ADR-0008): opt-in `ai.*` registry commands in `pbi-ai`, off by default; per
  provider, capability and data-class consent; keys only in the OS keychain (service `dashcraft`)
  or `DASHCRAFT_AI_API_KEY`; `ai.preview` shows the exact request.
- **Name** (ADR-0009): DashCraft (§10); reversible, the ADR lists what a rename touches.
- **Blob access**: offline only, through the `BlobResolver` trait and `blob-map.json`; real Azure
  access (SAS or az token from the OS keychain) may come later behind the same trait.
- **Save formats**: PBIP via `save_as_pbip`, PBIT via `export_pbit` (`dashcraft convert`). Writing
  PBIX is out of scope.
- **Toolchain**: Rust 1.99.1 pinned (`rust-toolchain.toml`), MSRV 1.94 (DataFusion), edition 2024.

## 5. Coding rules

- Library errors are `thiserror` enums per crate; `anyhow` only at the CLI and app edges.
- Unknown fields: PBIR types use `pbi-pbir`'s `json_object!` macro (typed known properties, the
  rest in `extra`, key order and number spellings kept; no `#[serde(flatten)]`, which mangles
  numbers under `arbitrary_precision`); TMSL keeps untyped keys in `extra` in source order; TMDL
  keeps unknown properties in the CST and model. PBIR `$schema` versions vary: parse tolerantly.
- Show display names (renamed column, projection `displayName`) with the source name in a
  tooltip; keep both in the model.
- Unsupported M or DAX returns a typed "unsupported" result with the source text; the UI shows ⚠
  and the expression. Never crash.
- Engine entry points are `async` and take a `CancellationToken`.
- CLI JSON outputs carry `schemaVersion`; within a version fields are only added.
- Comments only where code needs clarification; `rustfmt` defaults.
- Scratch output goes under `target/` (e.g. `target/tmp/`), never the system temp dir or the tree.

## 6. Parallel-agent protocol

Many agents work in this repository at once.

- **One owner per crate or directory per wave.** Declare the files you own at the start of a task
  and edit only those. Don't edit, reformat or "fix" another agent's files, even for a trivial
  warning; report the needed cross-cutting change (root `Cargo.toml`, `Cargo.lock`,
  `xtask/src/layers.rs`, shared docs) to the orchestrator in your final report instead.
- Uncommitted changes you didn't make belong to someone else: never revert, stash, `git checkout`
  or commit them.
- While others are mid-change, lint only your crate: `cargo clippy -p <crate> --all-targets
  --no-deps -- -D warnings`, and test with `cargo test -p <crate>`. Avoid full-workspace builds
  unless you are the integrator.
- A fuzz crash in a crate someone else owns: store the minimized input under
  `fuzz/regressions/<crate>/` with a note ([docs/ci.md](docs/ci.md#triage-a-crash)).
- **Disk hygiene**: `target/` grows to 20–40 GB (DataFusion, Tauri, many test binaries). Keep at
  least 15 GB free (`df -h .`); prune with `rm -rf target/debug/incremental`; the orchestrator runs
  `cargo clean` between waves; delete `fuzz/target` (~10 GB) after fuzzing and
  `target/tmp/golden/` when done.
- Sessions end abruptly (crashes, context limits): keep the tree building at every step. A green
  tree plus the dev log (§9) is how the next agent picks up.

## 7. Build, test, validate

```sh
# Core gates (CI `rust` job: macOS arm64, Windows x64, Ubuntu)
cargo fmt --all --check
cargo xtask layers                                   # crate layering (also --quiet)
cargo clippy --all-targets --locked -- -D warnings   # default-members: crates/* + xtask
cargo test --locked --no-fail-fast                   # tests needing optional data print SKIP and pass

# While iterating (one crate; preferred when other agents are building)
cargo clippy -p <crate> --all-targets --no-deps -- -D warnings
cargo test -p <crate> [--test <file>] [<name filter>]

# Performance budgets (docs/perf.md); CI runs the --quick line, informational
cargo xtask perf --list
cargo xtask perf --scenario P01            # id or name; repeatable; full iteration count
cargo xtask perf --quick --no-fail
cargo xtask perf --update-baseline         # idle machine, full run, commit perf/baseline.json

# Supply chain (CI `deny` job)
cargo deny check                           # install: cargo install cargo-deny --locked
cargo deny check bans licenses sources     # offline once the index is cached
python3 scripts/license-inventory.py       # regenerate THIRD_PARTY_NOTICES.md (--check to verify)

# Render checks: snapshots of every Acme page, expected values, Microsoft samples
cargo test -p pbi-render --test render_snapshots   # a change writes *.snap.new; cargo insta review
cargo test -p pbi-render                           # incl. the 39 card values of expected.json
# Golden images (SSIM only where reference/golden/<example>/<report>/<page>.png exists, else SKIP;
# outputs in target/tmp/golden/)
cargo test -p pbi-render --test golden -- --nocapture
cargo run -p pbiview-cli -q -- render reference/examples/acme-outfitters/AcmeOutfitters.pbip \
  --blob-map reference/examples/acme-outfitters/blob-map.json --report Overview \
  --png target/tmp/overview.png --force

# Snapshot tests (insta: pbiview-cli `inspect --json`, pbi-session exports, pbi-render page renders)
cargo test -p pbiview-cli --test cli       # a mismatch writes *.snap.new (gitignored)
cargo insta review                         # accept after reading the diff (cargo install cargo-insta)

# Command catalog (pbi-ops test `commands_md_is_up_to_date` fails when stale)
cargo run -q -p pbiview-cli --bin dashcraft -- commands --markdown > docs/COMMANDS.md

# App (CI `app` job; Linux needs the webkit2gtk-4.1 dev packages)
(cd app && npm ci && npm run check && npm run build)   # build produces app/dist, embedded by Tauri
cargo clippy -p pbiview-app --all-targets -- -D warnings
cargo test -p pbiview-app                  # mock runtime, no window; also regenerates bindings.ts
(cd app && npm run bindings)               # after changing an IPC command or DTO; commit bindings.ts
(cd app && npx tauri build --debug --no-bundle)        # debug app binary without installers

# Fuzzing (nightly; fuzz/ is its own workspace; docs/ci.md)
cargo +nightly fuzz build
bash fuzz/run.sh <target> 60
rm -rf fuzz/target
```

Required test kinds: TMDL byte-identical and PBIR JSON value-equal round-trips on **every**
fixture file; `insta` snapshots of `inspect --json`; one test per DAX function
(`crates/pbi-dax/tests/functions.rs`, enforced together with `FUNCTIONS.md`); the XPress9
known-good blob; render snapshots of every Acme Outfitters page; asserts of the example's expected
values (row counts, measures, all card values on rendered pages); golden SSIM ≥ 0.85 per region and
page where optional local golden images exist (no exceptions; none are committed); registry
commands tested through `Registry::execute`. Add tests for the behaviour you change, preferably
in existing test files.

### Fixtures and expected values

Fixture root: `$PBIVIEW_REFERENCE`, else `<workspace>/reference`; [reference/README.md](reference/README.md)
lists which tests use which files. The app reads a blob map from `DASHCRAFT_BLOB_MAP` (alias
`PBIVIEW_BLOB_MAP`) at startup; the CLI takes `--blob-map`.

| Path | Committed | What |
|---|---|---|
| `reference/examples/acme-outfitters/` | yes | Synthetic "Acme Outfitters" retail example generated by `scripts/examples/gen_acme.py` (`scripts/examples/check_acme.py` checks DashCraft against it): `AcmeOutfitters.pbip` (TMDL model, three PBIR reports), CSV data, `blob-map.json`. The acceptance values (row counts, card values per page, calculation-group, RLS and filter cases) are in [EXPECTED.md](reference/examples/acme-outfitters/EXPECTED.md) and `expected.json` |
| `reference/samples/microsoft/` | yes | Small MIT-licensed Microsoft sample PBIX/PBIT reports; source, hashes and what each exercises in [its README](reference/samples/microsoft/README.md) |
| `reference/samples/downloads/` | no (gitignored) | Larger MIT Microsoft samples, fetched with `scripts/fetch-samples.sh` (SHA-256 verified) |
| `reference/golden/` | no (gitignored) | Optional Power BI PNG exports of the example's pages for the SSIM comparison: `golden/<example>/<report>/<page>.png`, e.g. `golden/acme-outfitters/Overview/Executive Summary.png`; none are committed |
| `reference/private/` | no (gitignored) | Optional local files for your own experiments: never commit them and never cite them in tests, docs or commits (rule 2) |

Tests assert the values recorded in `expected.json` / `EXPECTED.md`; docs link to `EXPECTED.md`
instead of copying its numbers. A test that needs a gitignored file prints `SKIP: <file> missing` and passes.

## 8. Picking work and definition of done

**Order:**

1. Failing CI, failing tests, regressions (perf, golden, expected values, snapshots) on `main`.
2. Items marked **blocked** in [docs/ROADMAP.md](docs/ROADMAP.md) that the user has just unblocked
   (Windows ground truth, signing certificates, legal review).
3. The lowest-scoring rows in [docs/GAPS.md](docs/GAPS.md) that line up with the ROADMAP "Next"
   list (and the known misses in [docs/perf.md](docs/perf.md)).
4. New features.

Every piece of work ships with tests (synthetic data or the example; SKIP when optional data is
missing), docs and a CHANGELOG `[Unreleased]` line.

**Definition of done:**

- [ ] The capability is a `pbi-ops` command (typed params/output, metadata with an example, a test
      through `Registry::execute`), and `docs/COMMANDS.md` is regenerated; adapters stay thin.
- [ ] Tests cover the change and fail without it; fixture-dependent ones SKIP cleanly.
- [ ] `cargo fmt --all --check`, clippy `-D warnings` and tests pass for every touched crate;
      `cargo xtask layers` passes.
- [ ] Affected perf scenarios run with no new `FAIL` or regression; expected-value asserts pass,
      render snapshot changes are intended and reviewed, and goldens pass where the optional golden
      images exist (say so in the report when they don't).
- [ ] App changes: `npm run check`, `npm run build`, `cargo clippy -p pbiview-app`, bindings
      regenerated if IPC changed.
- [ ] Docs updated where relevant: `FORMATS.md`, `docs/VISUALS.md`, `FUNCTIONS.md`, `docs/CLI.md`,
      `docs/ARCHITECTURE.md`, `docs/ROADMAP.md` (tick items), the `docs/GAPS.md` row (status,
      score, evidence), a new ADR for a new decision (old ADRs are superseded, never rewritten).
- [ ] One line under `## [Unreleased]` in `CHANGELOG.md` for user-visible changes.
- [ ] A `docs/DEVLOG.md` entry (§9).
- [ ] New dependency: requested from the orchestrator, `cargo deny check` clean, notices updated.
- [ ] No private data or credentials staged. Before committing:
      `git diff --cached --name-only | grep -E '^reference/(golden/|private/|samples/downloads/)|DataModel$|SecurityBindings$|azpbi|msal_token_cache|\.env$|\.sess$'`
      must print nothing; also look for internal names, numbers or URLs in the diff (rule 2).

## 9. Dev log

Append one dated entry to [docs/DEVLOG.md](docs/DEVLOG.md) per wave or merged change, newest first
under a `## YYYY-MM-DD` heading: **what** changed (crates, commands, files), **why** (roadmap or
gap item), and **numbers** (tests, timings before → after, SSIM, function counts), plus the commit
hash once known and anything left open. Keep entries short; details go in the commit and the docs.

## 10. Naming and trademark policy

- Product name: **DashCraft**. Describe it as a tool that "opens Power BI files"; never use
  "Power BI" in the product name, app name or icon.
- "Power BI" and "Microsoft" are trademarks of Microsoft, used nominatively only, to say which
  files we read. No Microsoft or Power BI logos, icons, images or CSS, except the MIT-licensed
  Fluent UI System Icons already vendored under `app/src/assets/icons/`. Fonts: Selawik and Carlito
  are bundled unmodified (OFL Reserved Font Names).
- DashCraft is not affiliated with or endorsed by Microsoft, storytold/ArtCraft or any other
  product named "DashCraft"; the root [`NOTICE`](NOTICE) says so (linked from the README, shown
  in the app's About page and installed with it). Don't style the project like ArtCraft.
- Crate names stay `pbi-*` / `pbiview-*`; user-facing names are DashCraft's
  ([ADR-0009](docs/adr/0009-naming-dashcraft.md)): binary `dashcraft` with a `pbiview` alias,
  app `dashcraft-app`, `DASHCRAFT_*` env vars with `PBIVIEW_*` aliases (the new name wins), MCP
  server `dashcraft`. New user-facing variables get only the `DASHCRAFT_` prefix; developer and
  test variables (`PBIVIEW_REFERENCE`, `PBIVIEW_BLESS`, `PBIVIEW_TEST_RECORD`) keep their names.
  The name is reversible: ADR-0009 lists what a rename touches.

## 11. Commits

- One commit per milestone or coherent change; imperative subject with a scope or milestone prefix
  (`feat(ops): …`, `perf(render): …`, `docs: …`, `M1: …`).
- Run the gates of §7 for what you touched first, and include the trailer of rule 16.
