# CI and release

All workflows live in `.github/workflows/`. Actions are pinned to major versions and
kept current by Dependabot (`.github/dependabot.yml`, weekly: cargo, npm in `app/`,
GitHub Actions). Every workflow defaults to `permissions: contents: read`; only the
release jobs get `contents: write`.

The Rust toolchain always comes from `rust-toolchain.toml` (installed with
`rustup toolchain install`), so CI and local builds use the same compiler. The
`cargo-fuzz` job of the fuzz workflow is the exception: it uses nightly.

## `ci.yml`: push and PR to `main`

CI is skipped when a change touches only Markdown, `docs/` (including `docs/images/`),
`release.yml`, `fuzz.yml` or `dependabot.yml`. It still runs for the Markdown the tests read:
`docs/COMMANDS.md`, `docs/AI.md`, everything under `crates/` (`FUNCTIONS.md`, the AI prompts)
and `reference/`, and `THIRD_PARTY_NOTICES.md`. A test that starts reading another file needs
that file added to the `paths` list. **Run workflow** runs CI on demand.

Known frontend-only changes skip the Rust and performance jobs, not the three-platform app
checks or dependency policies. `.github/scripts/ci_scope.py` classifies the changed paths;
its tests and `actionlint` run in `detect` before the build jobs start. It ignores the same
non-runtime documentation/configuration as the workflow trigger, but keeps generated
TypeScript contracts (`*.gen.ts`, `bindings.ts`) on full Rust CI because Rust tests verify
them. Rust sources/manifests/lockfiles, fixtures, CI configuration and every unknown path run full
validation. Git diffs disable rename detection so a removed Rust path cannot be hidden by
a frontend destination. Manual runs and ranges without a previous revision run everything;
an invalid or unavailable Git revision fails detection rather than silently skipping checks.

| Job | Runs on | What it does |
|---|---|---|
| `detect` | ubuntu-latest | Lints `ci.yml`, tests change selection, and selects full Rust validation or frontend-only validation |
| `rust` | macos-15 (arm64), windows-latest, ubuntu-latest; except known frontend-only changes | `cargo fmt --all --check` (Linux only), default-members guard (Linux only), `cargo xtask layers` (Linux only, [below](#architecture-layers-cargo-xtask-layers)), `cargo clippy --all-targets --locked -- -D warnings`, `cargo nextest run --locked --no-fail-fast`, `cargo test --doc --locked --no-fail-fast` |
| `perf` | ubuntu-latest; same change selection as `rust` | `cargo xtask perf --quick --no-fail --locked`: release build of the `perf_*` examples, scenarios vs `perf/budgets.toml`. **Informational** (`continue-on-error`): the table goes to the job summary and `target/perf/` is uploaded as the `perf-report` artifact (14 days). See [perf.md](perf.md) |
| `app` | same 3 OSes, only when `app/package.json` exists | Installs webview libraries on Linux, runs `npm ci`, `npm run check`, and `npm run build` in `app/`, then `cargo clippy -p dashcraft-app --all-targets --locked -- -D warnings`. On Linux it also runs `scripts/license-inventory.py --check` and emits a **warning** (not a failure) when the license inventory in `THIRD_PARTY_NOTICES.md` is stale ([packaging.md](packaging.md#license-inventory-and-sbom)) |
| `deny` | ubuntu-latest | `cargo deny check advisories` (non-blocking) and `cargo deny check bans licenses sources` (blocking), using `deny.toml` |

### Build reuse and test execution

The Rust jobs use sccache's content-addressed compiler cache for workspace library compilations
via `RUSTC_WORKSPACE_WRAPPER` on test steps only. `Swatinem/rust-cache` keeps dependency
artifacts, including Clippy's metadata; sccache does not store a second copy of those dependencies.
Rust dependency caches are saved on `main` and same-repository PRs, so repeated pushes can
reuse changed dependencies. Fork PRs can restore base-branch dependency caches but do not save
these archives. App and release-profile performance jobs retain their separate main-only
dependency caches. Compiler cache statistics are written to each Rust job's summary. Caches
remain scoped by GitHub's branch/PR isolation; no `pull_request_target`, new secrets or broader
permissions are needed.

Windows Rust checks use the LLD linker bundled with the pinned Rust toolchain and disable
development/test debug symbols. Compiler diagnostics and assertion locations remain available,
but Windows runtime backtraces have less source-location detail. macOS/Linux retain line-table
debug information; release builds and performance budgets are unchanged.

Nextest runs test binaries concurrently instead of waiting for each integration-test binary
in sequence. It keeps default-member selection and normal ignored-test handling, with no
retries or relaxed assertions. Doctests still run separately, including after a nextest test
failure; local `cargo test` remains supported.

Before these changes, PR #16's Windows job spent 24m16s compiling/linking tests and about four
minutes executing them. The identical tree took about 11 minutes for the whole workflow after
merging with warmer caches, versus about 34 minutes on the PR.

Two attempts of [CI run 41927081220](https://github.com/Jxck-S/dashcraft/actions/runs/41927081220)
tested the same PR #18 revision (`d71dcc1`); every one of the ten jobs passed both times:

| Check | First run (mixed caches) | Same-revision rerun |
|---|---|---|
| Complete workflow | 37m28s | 7m02s |
| Windows Rust | 37m14s | 6m46s |
| macOS Rust | 5m17s | 4m47s |
| Ubuntu Rust | 3m39s | 3m51s |
| Ubuntu app | 11m18s | 1m23s |

The new Windows configuration initially needed a full dependency build. On the rerun, its
test compilation fell from 25m46s to 1m45s, with all 1,228 unit/integration tests and twelve
doctests passing; the two ignored doctests match the previous `cargo test` run. This demonstrates
warm-cache reuse, not a controlled comparison against the old configuration's different
ref/cache scope. Cold builds can still take tens of minutes.

APT is not cached: Linux package setup varied from 10m12s to 23s because package downloads
were slow on the first runner. The Ubuntu app improvement is network variability, not a
compiler-cache benefit. Download-archive caching remains a follow-up; do not cache installed
system directories.

### Why Cargo uses default members

The workspace has `members = ["crates/*", "xtask", "app/src-tauri"]` and
`default-members = ["crates/*", "xtask"]`. Running Cargo without `--workspace` uses the
default members. That covers every library, the CLI and the `xtask` tooling, but not the
Tauri app, which needs WebKitGTK and other system libraries on Linux.

We use this instead of `--workspace --exclude dashcraft-app` for three reasons:

- Plain `cargo test` locally and nextest plus doctests in CI cover the same default members.
  `Cargo.toml` is the only place that defines what counts as "core".
- CI does not hard-code the app's package name.
- The `app` job lints the app separately, with its dependencies installed.

The weakness is that a new member outside `crates/*` could be skipped silently. A
guard step prevents that: it reads `cargo metadata` and fails the job if any
workspace member other than `dashcraft-app` is missing from `default-members`.

Tests use the committed Acme Outfitters example and Microsoft samples. Tests that
need optional gitignored files (larger samples from `scripts/fetch-samples.sh`,
golden images) must print `SKIP: ...` and return (see `reference/README.md`). CI
runs on a fresh clone, so those files are never present there.

### Architecture layers (`cargo xtask layers`)

`xtask/src/layers.rs` assigns every workspace crate to a layer. The check reads
`cargo metadata --all-features` (every platform, with `resolve`) and fails on:

- a dependency on a **higher** layer (normal, build or dev), unless it is in `EXCEPTIONS`
  (empty; put orchestration in `pbi-session` or `pbi-ops` instead);
- a dependency **inside** one layer that is not listed in `INTRA_LAYER`;
- a workspace crate in **no** layer (a new crate must be added to `LAYERS`);
- a dependency on build tooling (`xtask`);
- any workspace dependency of `pbi-model` or `pbi-m` (kept standalone; `pbi-m` is fuzzed alone);
- a crate other than `dashcraft-app` that reaches a **UI toolkit** crate (`tauri*`, `wry`,
  `tao`, `specta*`, `egui*`, `eframe`, `winit`, `muda`, `rfd`, `tray-icon`, `webkit2gtk*`,
  `webview2-com*`, `gtk`) through any chain of normal or build dependencies, or through its
  own dev-dependencies. The error prints the path.

| Layer | Crates |
|---|---|
| L6 front ends | `dashcraft-cli`, `dashcraft-app` |
| L5 operations | `pbi-ops` (command registry) |
| L4 render | `pbi-render` |
| L3 session | `pbi-session` |
| L2 DAX, readers | `pbi-dax`, `pbi-core` (allowed: `pbi-core` → `pbi-dax`) |
| L1 formats, M, storage | `pbi-tmdl`, `pbi-tmsl`, `pbi-pbir`, `pbi-m`, `pbi-vertipaq`, `pbi-engine` |
| L0 model | `pbi-model` |
| tooling | `xtask` |

A crate listed in `LAYERS` but missing from the workspace is reported, not an error. When an
edge is intended, change `xtask/src/layers.rs` and the crate graph in
[ARCHITECTURE.md](ARCHITECTURE.md) together. The checker's unit tests run with `cargo test`.

### cargo-deny policy (`deny.toml`)

- **Licenses:** only permissive licenses are allowed: MIT, MIT-0, Apache-2.0,
  Apache-2.0 WITH LLVM-exception, BSD-2/3-Clause, ISC, Zlib, Unicode-3.0,
  Unicode-DFS-2016, BSL-1.0, CC0-1.0, CDLA-Permissive-2.0, MPL-2.0 and OFL-1.1.
  - MPL-2.0 is file-level copyleft. Using MPL crates unmodified is fine, but don't
    patch or vendor them without review.
  - OFL-1.1 is allowed for bundled fonts only.
  - Any license not on the list fails the check, including GPL, AGPL and LGPL.
  - A crate with alternative licenses (for example `Apache-2.0 OR GPL-2.0-only`)
    passes if one of the alternatives is allowed.
  - Our own crates (`publish = false`, MIT) are skipped via
    `[licenses.private] ignore = true`.
  - Add any exception to `[licenses].exceptions` with a reason.
- **Bans:**
  - `*` version requirements are denied, except for internal path dependencies.
  - Duplicate crate versions produce a warning, not a failure.
- **Sources:** only crates.io is allowed. Unknown registries and git sources are
  denied.
- **Advisories:** RustSec advisories, with yanked versions denied. Unmaintained
  crates are reported only for direct dependencies. In CI this check is
  non-blocking, so a newly published advisory doesn't fail unrelated PRs.
- **Graph:** limited to the four shipped targets (macOS arm64/x64, Windows x64,
  Linux x64), with all features enabled.

## `release.yml`: tags `v*` or a manual run

Packaging details (bundle config, artifact names, verification, signing setup, license
inventory and SBOMs) are in [packaging.md](packaging.md). In short:

1. `prepare` resolves the version. A pushed tag must equal `v` + the
   `[workspace.package] version` in `Cargo.toml` (and `app/package.json`). A manual run
   (**Run workflow**) releases that version, or the `version` input (e.g. `0.3.0-rc.1`), which
   the build jobs apply with `scripts/set-version.sh`. It then:
   - takes the release notes from the version's `CHANGELOG.md` section. A stable tag fails
     without one; pre-releases fall back to `[Unreleased]`.
   - regenerates the license inventory and `THIRD_PARTY_LICENSES.txt`
     (`scripts/license-inventory.py`);
   - builds the CycloneDX SBOM zip with `cargo-cyclonedx` and `npm sbom`.
2. `create-release` creates the tag `v<version>` on the selected commit for a manual run
   (refused when the tag exists on another commit), then a **draft** GitHub release for the tag, or reuses the existing
   draft when the workflow is re-run, and uploads the notices and the SBOM zip. A tag
   containing `-` (such as `v0.2.0-rc.1`) is marked as a prerelease.
3. `bundle` builds the Tauri app with `npx tauri build --ci --target … --bundles …`,
   verifies it (macOS: codesign, document types, resources; Linux: deb contents, `.desktop`
   file, MIME XML), writes a `.sha256` per file and uploads:
   - macOS arm64 and x64 `.dmg`. Both are built on `macos-15`; x64 is cross-compiled. These
     are two thin builds, not a universal binary.
   - Windows x64 `.msi` (WiX) and `-setup.exe` (NSIS). Pre-release versions that WiX
     rejects (non-numeric, such as `-rc.1`) get NSIS only.
   - Windows arm64 `-setup.exe` (NSIS only), built on the native `windows-11-arm` runner.
   - Linux x64 and arm64 `.AppImage`, `.deb` and `.rpm`, built on `ubuntu-22.04` and
     `ubuntu-22.04-arm` for an older glibc baseline. rpm versions can't contain `-`, so
     pre-releases ship AppImage and deb only.
4. `cli` builds `dashcraft` (`crates/dashcraft-cli`) for the same six
   targets and smoke-tests the native ones. It uploads `dashcraft-<tag>-<target>.tar.gz`
   (`.zip` on Windows, with README, `NOTICE`, notices and license texts) and a `.sha256` file
   for each. The Windows CLI is
   statically linked against the CRT.
5. `finalize` uploads `SHA256SUMS.txt` for every asset. On public repositories, or with
   the repository variable `DASHCRAFT_ATTEST=true` on GitHub Enterprise Cloud, it also
   creates a build-provenance attestation.

Every job also stores its outputs as workflow artifacts (14 days). A manual run with
`dry_run` checked does the same builds without a tag or a GitHub release. Inputs and
release candidates: [packaging.md](packaging.md#release-workflow).

Review the draft, then publish it by hand.

### Signing secrets (optional)

Add these under **Settings → Secrets and variables → Actions**. Each group is
optional. If a group is missing, its platform builds **unsigned** and the
release still succeeds.

| Secret | Purpose |
|---|---|
| `APPLE_CERTIFICATE` | Base64 of the "Developer ID Application" `.p12` (`base64 -i cert.p12 \| pbcopy`) |
| `APPLE_CERTIFICATE_PASSWORD` | Password of that `.p12` |
| `APPLE_SIGNING_IDENTITY` | Optional, e.g. `Developer ID Application: Org (TEAMID)`; inferred from the certificate if omitted |
| `APPLE_ID` | Apple ID email used for notarization |
| `APPLE_PASSWORD` | **App-specific** password for that Apple ID |
| `APPLE_TEAM_ID` | 10-character team ID |
| `WINDOWS_CERTIFICATE` | Base64 of the code-signing `.pfx` (`[Convert]::ToBase64String([IO.File]::ReadAllBytes('cert.pfx'))`) |
| `WINDOWS_CERTIFICATE_PASSWORD` | Password of that `.pfx` |

How each platform behaves:

- **macOS:**
  - Signs when `APPLE_CERTIFICATE` and `APPLE_CERTIFICATE_PASSWORD` are set.
  - Also notarizes when `APPLE_ID`, `APPLE_PASSWORD` and `APPLE_TEAM_ID` are set
    too.
  - Without the certificate secrets, the app gets an ad-hoc signature
    (`APPLE_SIGNING_IDENTITY=-`). Users must approve it in Gatekeeper the first
    time they open it.
- **Windows:**
  - The `.pfx` is imported into the runner's certificate store.
  - Its thumbprint is passed to the build with an extra `tauri build --config` file,
    along with SHA-256 digest and DigiCert timestamping. Tauri signs the app
    executable and both installers.
  - Certificates issued after mid-2023 usually live on an HSM or a cloud signer
    (for example Azure Trusted Signing). For those, replace this step with
    Tauri's `bundle.windows.signCommand`.
- **Linux:** AppImage and `.deb` are not signed.
- **CLI archives:** not code-signed. Verify them with the `.sha256` files and
  `SHA256SUMS.txt`.

How to obtain the certificates and set up notarization:
[packaging.md → Signing and notarization](packaging.md#signing-and-notarization).

If the Tauri updater is enabled later, also add `TAURI_SIGNING_PRIVATE_KEY` and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, and pass them to the *Build bundles* step.

## `fuzz.yml`: nightly at 03:23 UTC, or manual

- Runs only when `fuzz/Cargo.toml` exists. Otherwise the job succeeds with a
  notice.
- Installs nightly and `cargo-fuzz`, builds every target, then runs each target
  from `cargo fuzz list` for 60 seconds through `fuzz/run.sh` (see below). You
  can set a different duration when starting the workflow manually.
- The corpus (`fuzz/corpus`) is cached between runs.
- On failure, crash inputs from `fuzz/artifacts` are uploaded as the
  `fuzz-artifacts` artifact.
- A second job, `registry property tests`, runs the property tests of the
  command registry (`crates/pbi-ops/tests/property.rs`, M6.11) in release with
  the pinned toolchain: `DASHCRAFT_PROPERTY_STEPS=10000` and
  `DASHCRAFT_PROPERTY_SEED` set to the run number, so every night walks new
  edit sequences (below).

### Registry property tests

`crates/pbi-ops/tests/property.rs` drives the command registry the way an
agent or the app does, through `Registry::execute`:

- every catalog command is called with its examples, with parameters derived
  from its JSON Schema and with mutated parameters (wrong types, missing and
  unknown fields, odd strings and numbers): no panic, which the registry reports
  as an `internal` error, and no call over 60 seconds;
- seeded random sequences of edits (some invalid), undos and redos on the Acme
  Outfitters example: a refused or no-op edit changes nothing, undoing
  everything gives a document equal to the original, and on loaded data the
  edited document saves as a new PBIP project that reads back with the same
  model and visuals, with every measure value back after the undos.

`cargo test` runs a quarter of the steps in debug builds (2,500 and 250) and
all of them in release (10,000 and 1,000). Two environment variables change
that:

| Variable | Effect |
|---|---|
| `DASHCRAFT_PROPERTY_STEPS` | Steps of each random test |
| `DASHCRAFT_PROPERTY_SEED` | Mixed into each test's seed; the log prints the seed |

To reproduce a nightly failure, rerun with the run's values:

```sh
DASHCRAFT_PROPERTY_STEPS=10000 DASHCRAFT_PROPERTY_SEED=<run number> \
  cargo test --release -p pbi-ops --test property -- --nocapture
```

### Fuzz targets

`fuzz/` is its own Cargo workspace (`[workspace]` in `fuzz/Cargo.toml`, own
`Cargo.lock` pinned to the root lock's versions), so root `cargo` commands
ignore it. Targets are in `fuzz/fuzz_targets/`. Every target must never panic,
time out or run out of memory; most also assert round-trip invariants, which
is where real bugs show up.

| Target | Code under test | Invariants beyond "no panic" |
|---|---|---|
| `xpress9` | `pbi_vertipaq::xpress9` (`fuzz_decompress`, one-shot and streaming decoders) | output within `Limits::SMALL`; streaming == one-shot; uncompressed ABF passes through |
| `abf` | `pbi_vertipaq::fuzz_abf` (ABF container, `metadata.sqlitedb`) | |
| `vertipaq` | `pbi_vertipaq::fuzz_vertipaq` (IDF, dictionaries, Huffman, whole pipeline) | |
| `tmdl` | `pbi_tmdl` parser, loader, both writers (NUL-separated files) | CST re-emits the input; `load(write(m)) == m` |
| `tmsl` | `pbi_tmsl` read/write (UTF-8/UTF-16) | `to_json(read(x))` equals `x` (key order and exponent spelling aside); written bytes read back to the same model and re-write identically |
| `pbir_layout` | `read_legacy_layout`, `legacy_to_report_ast` | outer layout re-serializes identically |
| `pbir_json` | typed PBIR files, `read_pbir`, `to_report_ast`, `write_pbir` | typed round-trip identical; `write_pbir` keeps files; AST stable after write + read |
| `m_parse` | M lexer, parser, printer | spans valid; `parse(print(parse(x))) == parse(x)`; printing is a fixed point |
| `m_eval` | `pbi_m::Engine` with tiny limits and `NoBlobResolver` (NUL-separated shared queries) | no `MError::Internal` |
| `dax_parse` | `pbi_dax::syntax` expression/query/function parsers, printers, visitor | spans valid and nested; compact and pretty printer round-trip |
| `dax_service` | `pbi_dax::service` diagnostics (both source kinds, a small and an unknown model), signature help, hover and completion at every offset of a short text (64 spread offsets of a long one); the first byte is the completion limit | spans inside the text on char boundaries; at most `MAX_DIAGNOSTICS` diagnostics; at most the requested completion items, none empty |
| `m_service` | `pbi_m::service` diagnostics (a small model and an unknown one), hover, signature help and completion at every offset of a short text (64 spread offsets of a long one, plus past the end), and formatting with the default options and with a width, indentation and tab choice taken from the first byte; the first byte is also the completion limit | spans inside the text on char boundaries; at most `MAX_DIAGNOSTICS` diagnostics; parameter labels inside the signature and the active parameter one of them; at most the requested completion items, none empty; the completion span holds the cursor; formatting never reports a changed tree, a formatted text formats to itself, and a syntax error's span is inside the text |
| `tmdl_service` | `pbi_tmdl::service::diagnostics` over NUL-separated files (the first named `model.tmdl`, so references resolve) | at most `MAX_DIAGNOSTICS` diagnostics; each names an input file and its range lies inside that file on char boundaries; 1-based lines and columns; no empty suggestion; the same result on a second run |
| `zip_container` | `ZipContainer` with `Limits::strict()`: raw ZIPs, or `arbitrary`-built archives with hostile names | only normalized relative paths; sorted and unique; sizes within limits; reads return the declared size; escaping lookups fail |
| `detect` | `pbi_core::detect::detect_bytes` | |
| `open_bytes` | `pbi_core::open_bytes` (strict limits), `inspect_json`, `pbip_files`, `pbit_bytes` | a PBIT written from an opened document opens again |
| `people` | `pbi_session::transforms::people` | unparsed text is the trimmed input; names trimmed and non-empty |

### Running locally

```sh
cargo install cargo-fuzz --locked
rustup toolchain install nightly --profile minimal
cargo +nightly fuzz list
cargo +nightly fuzz build              # all targets, ASan; DataFusion makes the first build slow
bash fuzz/run.sh tmdl 90               # one target as CI runs it, for 90 s
bash fuzz/run.sh m_eval 120 -jobs=4    # extra libFuzzer flags are passed through
rm -rf fuzz/target                     # ~10 GB of sanitizer builds; delete when done
```

`fuzz/run.sh <target> [seconds]` writes new inputs to `fuzz/corpus/<target>`,
replays the committed `fuzz/seeds/<target>`, uses the target's dictionary and
sets `-timeout=60 -rss_limit_mb=2048`.

### Seeds and dictionaries

- `fuzz/seeds/<target>/` is committed and kept small (under 2 MB in total). Seeds
  come only from committed test data (`crates/*/tests/data`, the TMDL desktop
  sample, `crates/pbi-m/tests/stdlib.rs` snippets, the XPress9 vectors),
  synthetic builders (the `pbi-vertipaq` ABF/IDF/dictionary test builders,
  hand-made ZIPs) and a few files of the synthetic example in
  `reference/examples/acme-outfitters/`. **Never** seed with gitignored data
  (`reference/private/`, `reference/samples/downloads/`, golden files) or with
  anything taken from a real or internal report.
- `regression-*` seeds are minimized inputs of fixed bugs, so every run replays
  them.
- `fuzz/corpus/` and `fuzz/artifacts/` are gitignored.
- `fuzz/dictionaries/`: `m.dict` (keywords and the `pbi-m` library names),
  `dax.dict` (keywords and the `pbi-dax` function names), `tmdl.dict` and
  `json.dict` (TMSL/PBIR keys and edge-case literals). `fuzz/run.sh` maps
  targets to them. Refresh the function lists when the libraries grow.

### Triage a crash

1. Get the input: the `fuzz-artifacts` CI artifact, or `fuzz/artifacts/<target>/`
   locally (`crash-*`, `timeout-*`, `oom-*`, `leak-*`).
2. Reproduce: `cargo +nightly fuzz run <target> fuzz/artifacts/<target>/crash-<hash>`.
3. Minimize: `cargo +nightly fuzz tmin <target> <file>` (writes
   `minimized-from-*`). The panic message names the broken invariant.
4. Fix it in the crate that owns the code and add a regression unit test there
   that fails without the fix. Keep `cargo fmt`, `cargo clippy -p <crate>
   --all-targets -- -D warnings` and `cargo test -p <crate>` green.
5. Copy the minimized input to `fuzz/seeds/<target>/regression-<what>` and check
   that `cargo +nightly fuzz run <target> <file>` passes.
6. Timeouts and OOMs are bugs too: usually a missing size cap or step count.
7. If the crate is owned by someone else at the moment, store the minimized
   input under `fuzz/regressions/<crate>/` with a short note instead.

## `screen-reader.yml`: screen readers, weekly or on app PRs

The screen-reader smoke tests of [ADR-0015](adr/0015-accessibility-target.md) (A6 in the
[accessibility notes](accessibility/)) run in CI, so they need no person and nothing installed on a
developer machine. Triggers: a pull request to `main` that touches `app/src/**`, the workflow or
its script; Mondays at 05:23 UTC; **Run workflow**.

| Job | Runs on | Screen reader | Gate |
|---|---|---|---|
| `NVDA (windows-latest)` | windows-latest, WebView2 | NVDA (portable, from `guidepup install`) | Blocking |
| `VoiceOver (macos-15)` | macos-15, WKWebView | VoiceOver (AppleScript control and the TCC grants from `guidepup setup --ci`) | Blocking |
| `Orca (ubuntu-26.04)` | ubuntu-26.04, WebKitGTK, under `dbus-run-session` and `xvfb-run` | Orca (the apt packages from `guidepup setup --ci`) | Informational (`continue-on-error`) |

Each job builds the debug app (`npx tauri build --debug --no-bundle`), installs
[Guidepup](https://github.com/guidepup/guidepup) (MIT; CI only, never bundled or installed by
DashCraft) into `target/tmp/screen-reader/tools`, runs `guidepup setup --ci` (macOS, Linux) and
`guidepup install` (the reader's assets in Guidepup's cache), then runs
`scripts/app-checks/screen-reader.mjs`. The script opens a copy of the Acme Outfitters example,
brings the window to the front and walks the M6 editing journey (search, measure formula, commit,
undo and redo) and the M7 formula-bar journey (suggestions, insert, problems, Ctrl+K Ctrl+I, Tab
mode, the Properties description, the File menu and its "Unsaved changes" question, save), using
the platform's keys (⌘ and ⌥ on macOS). Each step clears the reader's log, acts, and checks
what the reader said against a pattern; steps that depend on a reader's verbosity are marked
informational in the report. The job fails when a required step fails.

`report.md` goes to the job summary; `report.json`, `report.md` and the app log are uploaded as
`screen-reader-<reader>` (default retention). The script drives the real keyboard and screen
reader of the machine it runs on: don't run it on a workstation.

Narrator has no automation interface. Its evidence is the UI Automation tree that it reads
(`scripts/app-checks/uia.ps1`), recorded with each milestone's accessibility notes.

## Running the same checks locally

```sh
# Core (same as the `rust` job)
cargo fmt --all --check
cargo xtask layers
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked

# Performance budgets (the `perf` job runs the first line; see perf.md)
cargo xtask perf --quick --no-fail
cargo xtask perf --scenario P01        # one scenario, full iteration count

# App (same as the `app` job; on Linux, install the webkit2gtk-4.1 dev packages first)
(cd app && npm ci && npm run check && npm run build)
cargo clippy -p dashcraft-app --all-targets --locked -- -D warnings

# Supply chain (same as the `deny` job)
cargo install cargo-deny --locked
cargo deny check                         # advisories needs network
cargo deny check bans licenses sources   # works offline once the index is cached

# Fuzzing (same as `fuzz.yml`)
DASHCRAFT_PROPERTY_STEPS=10000 cargo test --release -p pbi-ops --test property
cargo install cargo-fuzz --locked
cargo +nightly fuzz list
bash fuzz/run.sh <target> 60

# Lint the workflows themselves
actionlint .github/workflows/*.yml
```
