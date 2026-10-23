<!-- Research by a Copilot research agent, 2026-10-22, from public storytold/* repos (read-only). -->
# ArtCraft / Craft Apps Ecosystem Research — Findings for pbiview ("DashCraft")

## Summary

The `storytold` GitHub org (public profile name **"ArtCraft"**, blog `getartcraft.com`, 68 public repos) runs a fast-moving, apparently heavily AI-agent-accelerated suite of 13 "Crafting Apps" (PhotoCraft, PdfCraft, FilmCraft, VectorCraft, LightCraft, DesignCraft, EffectCraft, GridCraft, CADCraft, DeckCraft, SoundCraft, WordCraft, FigCraft) plus the flagship **ArtCraft** AI creative tool and a parallel minimal client **ArtCraft‑X**, all backed by a closed-license SaaS (**artcraft‑services**) and a growing layer of shared infrastructure repos. Almost the entire shared-infra layer (craft-libs, craft-launcher, craft-fonts, craft-ai-integrations, craft-common-actions, craft-repo-status-app, craft-i18n) was created between **Oct 6–16, 2026** — i.e., in the ~2 weeks before this snapshot — while artcraft itself dates to 2022 and was only relicensed to MIT/Apache-2.0 on **2026‑10‑10** (`storytold/artcraft@8abde75`). There is direct, repeated evidence of AI-coding-agent authorship (`Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` in commits, an in-app "About ▸ Models" credit page, AGENTS.md/CLAUDE.md in nearly every repo). A private `storytold/craftrules` repo (confirmed to exist, confirmed inaccessible — 404 to our read-only token) holds the canonical cross-app standards that every public repo's AGENTS.md points to.

No org `.github` profile repo exists (`storytold/.github` → 404; `storytold/storytold` → 404), so there is no org-wide profile README — the org README equivalent is effectively `storytold/artcraft`'s README plus `storytold/craft-contrib`'s RFC process doc.

---

## Repositories discovered (umbrella / shared-infra scope)

| Repo | Purpose | License | Created | Stars/Forks/OpenIssues |
|---|---|---|---|---|
| `storytold/artcraft` | Flagship Tauri 2 desktop app ("the IDE for artists"); AI image/video/3D/audio creation | MIT OR Apache-2.0 (relicensed 2026-10-10) | 2022-03-31 | 35.5k / 5.6k / 490 |
| `storytold/artcraftx` | "ArtCraft-X" — minimal Tauri 2 desktop client, early/experimental | **none found** (no LICENSE file) | 2026-07-30 | 1003 / 873 / 7 |
| `storytold/artcraft-services` | Backend (Actix/Tokio/MySQL/Redis/ES/S3), web frontends (Nx/React/Vite) | Custom "fair source" (`LICENSE.md`, NOASSERTION) | 2026-09-11 | 1272 / 1322 / 32 |
| `storytold/craft-libs` | Shared Rust library workspace for all Craft apps | MIT OR Apache-2.0 | 2026-10-09 | 328 / 622 / 163 |
| `storytold/craft-common-actions` | Reusable GitHub Actions (release/CI) for all Craft apps | **none found** (no LICENSE file) | 2026-10-11 | 69 / 206 / 14 |
| `storytold/craft-fonts` | Shared font assets + Rust loader crate | MIT OR Apache-2.0 (code); per-font OFL-1.1 | 2026-10-06 | 384 / 637 / 36 |
| `storytold/craft-ai-integrations` | Shared AI provider/connection/consent runtime layer | MIT OR Apache-2.0 | 2026-10-11 | 89 / 112 / 8 |
| `storytold/craft-repo-status-app` | Internal Tauri 2 tray dashboard polling all `*craft` repos | MIT (Cargo.toml) — **no LICENSE file committed** | 2026-10-09 | 138 / 232 / 19 |
| `storytold/photocraft-corpus` | Only `*-corpus` repo found: pinned real-file PSD/XCF test oracles for PhotoCraft | MIT OR Apache-2.0 (code); brand assets excluded | n/a | 150 stars |
| `storytold/craft-i18n` | Shared TSV translation catalogs + CI checker | MIT OR Apache-2.0 | 2026-10-16 | 7 / 10 / 7 |
| `storytold/craft-launcher` | Cross-app installer/updater/launcher (egui, no Tauri) | MIT OR Apache-2.0 | 2026-10-09 | 3824 / 1232 / 486 |
| `storytold/craft-contrib` | Shared contributor/agent-guideline RFC meta-repo | **none found** | n/a | 26 stars |
| `storytold/crafthub` | Placeholder repo ("Placeholder for merges") | **none found** | n/a | 12 stars |
| `storytold/craft-launcher-assets` | Launcher-downloaded screenshot assets | — | n/a | 43 stars |
| `storytold/indd-utils` | Read INDD files without InDesign, convert to IDML | Rust | n/a | 48 stars |
| `storytold/winprint.rs` (fork) | Windows printer API crate, used by `craft-print` | fork | n/a | 48 stars |
| `storytold/winit` (fork), `storytold/egui` (fork) | Vendored/patched upstream GUI deps | forks | n/a | — |
| `storytold/craftrules` | **Referenced everywhere, not publicly visible** — `gh api repos/storytold/craftrules` → 404. Confirmed private by `storytold/photocraft:AGENTS.md:7`: *"the repository is not public — outside contributors should ask a maintainer."* | private | — | — |

Not found: no `storytold/.github`, no org profile README, no other `*-corpus` repos besides `photocraft-corpus`.

---

## 1. What is ArtCraft (artcraft / artcraftx)

**Product vision** [verified]: ArtCraft bills itself as *"The IDE for artists"* and *"the IDE for interactive AI image and video creation"* — not a launcher, a creative-suite/AI-aggregator hub (`storytold/artcraft:README.md`). It is explicitly positioned against "model aggregation websites" as its #1 competitive differentiator (`storytold/artcraft:ROADMAP.md:7-8`). It composites in 2D/3D, stages scenes, and routes prompts through 62 third-party and first-party models (image, video, music/sound, 3D mesh, world/Gaussian-splat generation) from providers including Grok, Midjourney, Sora/OpenAI, World Labs, FLUX, Seedream, Kling, Veo, Suno, Hunyuan3D, Meshy, Marble, etc. (`storytold/artcraft:README.md`, catalog sourced from `artcraft-services/crates/service/web/storyteller_web/src/configs/omni_gen/*`).

**Tech stack** [verified]:
- `storytold/artcraft` is a **Tauri 2** desktop app: `Cargo.toml:110` pins `tauri = { version = "2.11.2", features=["unstable","devtools","protocol-asset"] }`, plus `tauri-plugin-dialog`, `-fs`, `-log`, `-upload`, and a vendored `tauri-plugin-http` fork. `AGENTS.md:7` states flatly: *"Artcraft is a public-facing Tauri desktop app for generating AI images and video."* Frontend is TypeScript/React/Vite (Nx monorepo) per `AGENTS.md` code-style section.
- **Major architectural pivot in progress** [verified]: the same `artcraft` repo workspace now *also* contains `artcraft-native` — *"ArtCraft's desktop app in pure Rust: an egui UI over the same ArtCraft API client crates the Tauri app uses. There is no Tauri and no JavaScript"* (`storytold/artcraft:artcraft-native/README.md:1-3`). The root `Cargo.toml` comment literally flags it: `"artcraft-native", # Pure Rust (egui) desktop app; no Tauri or webview`. This mirrors PhotoCraft's architecture exactly (egui/eframe, wgpu, no webview) and reuses `artcraft_client`/`artcraft_router` crates. Commits (`b5614ad`, `bb27304`, `28a39a0`, `d4e4ac6`, all Oct 10–11 2026, author `bflatastic`) show Create Image/Video/Audio already functional in the egui client. **[inferred]**: ArtCraft is in the middle of the same Tauri→egui migration pbiview's prior review found in photocraft — i.e., storytold itself is moving *away* from Tauri for its own flagship product, which is directly relevant context for DashCraft's Tauri 2 choice.
- `storytold/artcraftx` ("ArtCraft-X") is a second, newer, *separate* Tauri 2 app: *"a minimal desktop app for AI creation... Early Experimental Release"* (`README.md`). Dev script explicitly requires "Node.js 20+, npm, Rust, and the Tauri 2 CLI."

**Business model** [verified]: `storytold/artcraft-services:README.md` describes the backend (`storyteller-web`, Actix Web + Tokio + MySQL/SQLx + Redis + Elasticsearch + S3/R2) handling auth, accounts, media, generation jobs, credits, and **Stripe billing**. It exposes two API surfaces: session-based app endpoints (`/v1/omni_gen/*`) and a programmatic **API-key** surface (`/v1/omni_api/*`, `Authorization: ******`) requiring "API access... enabled for the account" — i.e., a paid/metered API-key product on top of the consumer app.

**Licensing/brand carve-out** [verified — important finding]:
- `storytold/artcraft`, `craft-libs`, `craft-fonts`, `craft-i18n`, `craft-launcher`, `craft-ai-integrations`, `photocraft-corpus` are dual **MIT OR Apache-2.0**.
- `storytold/artcraft-services` is **not** open source — it ships a bespoke **"fair source"** license (`LICENSE.md:6`): *"ArtCraft is a form of 'fair source' software… like free beer."* Explicit restrictions (`LICENSE.md:19-23`): **"1. You cannot sell the ArtCraft software commercially… 2. You cannot use the ArtCraft code to develop a competing business or product (eg. selling AI image and video tools). 3. You cannot fork ArtCraft and remove ArtCraft community, donation links, or paid model services."** The rationale section cites Fair Source, Fair Code, the Defold License, and explicitly contrasts with OSI licenses ("exceptionally challenging to grow big things commercially… leave room for hyperscalers to cherry-pick"), name-checking Stability.ai's monetization failure as a cautionary tale, signed "~ Brandon Thomas" (org admin `echelon`).
- `storytold/artcraftx` has **no LICENSE file at all** — an open licensing gap/risk on the newest client repo.
- Brand/trademark carve-out is explicit and repeated verbatim across repos (`photocraft:docs/brand/LICENSE-brand.txt`, referenced identically from `artcraft:NOTICE`, `photocraft-corpus:README.md:392-395`): the **"ArtCraft Marks"** (name, wordmark, logos) are **trademarks, not covered by the MIT/Apache license**, usable only unmodified and only to attribute "made by ArtCraft," and **must be removed from any fork or modified version** — forks "must not call it ArtCraft or present it as an ArtCraft product."

**artcraft-services tech stack details** [verified]: Rust+TypeScript monorepo; HTTP service = `storyteller-web` (Actix Web/Tokio); data layer = MySQL+SQLx (`mysql_queries` shared crate), Redis (cache/rate-limit), Elasticsearch (search), S3-compatible/R2 (media); provider integrations under `crates/api_clients/*` (fal, gmicloud, grok, midjourney, openai_sora, worldlabs, kinovi); routing via `artcraft_router`; frontend is an Nx workspace (React/Vite/Zustand/Three.js) with `frontend/libs/tauri-api` kept only because *"shared web components still import their types… The native desktop app and libraries used only by that app live in the separate desktop repository"* (`README.md`).

---

## 2. craft-libs: crate inventory

`storytold/craft-libs` is a Cargo workspace, edition 2024, resolver 3, Rust 1.95+, `license = "MIT OR Apache-2.0"` inherited via `workspace.package` (`Cargo.toml`). Crates are **not published to crates.io**; apps consume them via pinned **git `rev`** (immutable commit SHA), e.g. `storytold/photocraft:Cargo.toml:13`: `craft-ui = { git = "https://github.com/storytold/craft-libs", rev = "5c874d51eeb3de70e735c8ce87a59a7dedd47675", features = ["serde"] }`, and separately `craft-print` at `rev="3d37da38..."` and `craft-docpath` at `rev="82a24b00..."` — **confirming each crate is pinned independently, not as a single workspace-wide revision**, even within one consuming app.

| Crate | Purpose | Maturity/notes | License | Tauri-usable? |
|---|---|---|---|---|
| `craft-platform` | Native dialog lifecycle (deferred-close guard), Linux/BSD XDG desktop-appearance observation | Feature-gated (`dialogs`, `egui`, `appearance`); default adds zero deps | MIT/Apache-2.0 | Yes — default feature has no egui/toolkit coupling |
| `craft-ai-core` | Data model/contract for AI (connections, models, capabilities, roles, consent, jobs); `serde`+`serde_json` only, no I/O, compiles `wasm32-unknown-unknown` | Pure contract, denies `unwrap/expect/panic`, workspace forbids `unsafe` | MIT/Apache-2.0 | **Yes — ideal fit**, zero I/O/async, wasm-safe |
| `craft-pdf` | Optional PDF parse/interpret/raster/guarded-write wrapping vendored `krilla`/`hayro` forks | All features off by default; `read`/`interpret`/`raster`/`write*` granular features | MIT/Apache-2.0 (krilla/hayro assets CC0/BSD-3) | Yes, headless |
| `craft-atomic` | Crash-safe atomic file writes (same-dir staging, `F_FULLFSYNC`, Windows lock-retry, fault injection) — extracted from PhotoCraft's storage engine | Mature/battle-tested (PhotoCraft refs #2, #14) | MIT/Apache-2.0 | **Yes — directly useful** for `.pbix`/workspace-file saves |
| `craft-cms` | Pure-Rust ICC v2/v4 colour management (profiles, transforms, intents, 3D LUTs, built-ins) — extracted from PhotoCraft | No `unsafe`, no C deps, from ICC.1 specs | MIT/Apache-2.0 | Not relevant to BI (no color-managed rendering need) |
| `craft-docpath` | Decides if a document-stored path/`file:` URL may be opened (same-share only; refuses device paths/non-file URLs) | Minimal (no README, see `src/lib.rs`) | MIT/Apache-2.0 | **Relevant** — pbiview opens data-source paths from `.pbix`; same threat model |
| `craft-i18n` | Lookup/fallback/plurals/system-locale over embedded TSV catalogs (data lives in separate `storytold/craft-i18n` repo, confusingly same name) | `tl!` macro pattern, offered-locale gating | MIT/Apache-2.0 | Yes, but tied to the craft-i18n *data* repo's TSV format |
| `craft-media` | Bit-level I/O + clean-room MP4/MOV (isobmff) and MKV/WebM (matroska) demux/mux, from FilmCraft; `wasm32` buildable | `default=["isobmff"]`; `matroska` opt-in | MIT/Apache-2.0 | Not relevant to BI |
| `craft-print` | Submit print-ready PDF to CUPS/Windows spooler (no PowerShell/script invocation; in-process XPS on Windows) | Used by PdfCraft, DesignCraft | MIT/Apache-2.0 | **Relevant** if pbiview adds "Print report" |
| `craft-quit` | Native macOS quit / egui close handling / optional app menu | No README (see `src/lib.rs`) | MIT/Apache-2.0 | egui-specific — not directly usable in a webview UI |
| `craft-settings` | `config.toml` settings store: atomic save, backup, damaged-file recovery, schema migration | Deps only `craft-atomic`+`serde`+`toml_edit`+`dirs` | MIT/Apache-2.0 | **Yes — directly useful** for pbiview app settings |
| `craft-ui` | egui SVG icon catalogs, buttons, tabs, docking, theme, printing widgets; adopted by 9 apps (DesignCraft, VectorCraft, PhotoCraft, FilmCraft, EffectCraft, CADCraft, PdfCraft, LightCraft, SoundCraft) | Requires Rust 1.95+ | MIT/Apache-2.0 | **egui-only — not usable from a Tauri+Svelte webview UI** |
| `craft-window` | Cross-desktop decision of who draws the window frame (native vs. app-drawn) per compositor | Linux/Wayland-focused logic | MIT/Apache-2.0 | egui/native-window specific — not applicable to Tauri's own window chrome handling |

**License audit** [verified]: workspace-wide `license = "MIT OR Apache-2.0"`, `unsafe_code = "forbid"` lint, `clippy::dbg_macro`/`clippy::todo` warn (`Cargo.toml`). Third-party bundled assets (krilla's 4 CC0-1.0 ICC profiles, hayro-interpret's BSD-3-Clause Foxit standard-14 fonts, Adobe/Hayro MIT-OR-Apache metrics tables, Fogra39 data under Fogra's own free-redistribution terms) are itemized with SHA-256 in `ATTRIBUTION.md`.

**Which crates matter for pbiview**: `craft-atomic` (crash-safe file saves — directly applicable to `.pbix`/workspace persistence), `craft-settings` (config storage), `craft-docpath` (safe external-path policy, relevant for external data-source paths in reports), and `craft-ai-core` (wasm-safe AI data model — see §5). `craft-ui`, `craft-window`, `craft-quit` are egui-coupled and **not usable** in pbiview's Svelte/webview front end. `craft-print`/`craft-pdf`/`craft-cms`/`craft-media` are plausible-but-lower-priority (export/printing, not core BI). None of these crates are published to crates.io — adopting any means taking a **git-rev pin dependency on a third-party org's repo**, which is a supply-chain/governance consideration (see §8).

---

## 3. craft-common-actions: release/CI workflows

`storytold/craft-common-actions` (Python-tooled meta-repo, **no LICENSE file found** at all) hosts five reusable workflows and nine composite actions consumed via `uses: storytold/craft-common-actions/.github/workflows/release.yml@main`.

**`release.yml` platform matrix** [verified, `README.md`]:
- **macOS**: universal (x86_64+aarch64) bundle, DMG, CLI, **Developer ID codesign + notarization**
- **Windows**: x64, x86, **ARM64** MSI via **WiX v5**, CLI, **Azure Trusted Signing**
- **Linux**: x86_64, aarch64 AppImage/deb/rpm/tar.gz via `nfpm`
- **Flatpak**: x86_64, aarch64
- **FreeBSD**: 15.1-STABLE VM build (aarch64 currently paused per issue #39)
- **Arch Linux**: `.pkg.tar.zst` via `makepkg`, toggle `has-arch: false`
- **WebAssembly**: wasm32-unknown-unknown via Trunk
- **Linux RISC-V 64**: cross-compiled riscv64gc, SHA256-verified `nfpm`
- Draft-release job computes `SHA256SUMS.txt`

**Inputs/secrets**: caller passes `app-name`, `app-title`, `version`; secrets are explicitly enumerated per-name (`APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `KEYCHAIN_PASSWORD`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`, `WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD`, `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `AZURE_SIGNING_ENDPOINT`, `AZURE_SIGNING_ACCOUNT`, `AZURE_CERT_PROFILE`). Signing jobs read only from the `release` GitHub Environment, and **on the `release` branch, empty signing secrets fail the build rather than shipping unsigned** — stricter than "degrade gracefully."
`craft-fonts-ref` input feeds desktop builds via `CRAFT_FONTS_DIR` + `CRAFT_FONTS_REQUIRED=1`, so a missing font checkout **fails the build**, not silently ships without fonts.

**SBOM/attestations**: `README.md` does not mention CycloneDX/SBOM generation as part of `release.yml` itself — SBOM generation appears to be each app's own responsibility (confirmed absent from the craft-common-actions feature list; contrast with pbiview below).

**Other workflows**: `packaging-lint.yml` (actionlint, shellcheck, WiX/plist/AppStream XML lint, `desktop-file-validate`, `appstreamcli validate`, Flatpak manifest check); `windows-arm64.yml` (cross-build + real-hardware smoke test: install, verify `0xAA64` arch, CLI test, uninstall, on a **Windows 11 ARM64 runner**); `freebsd.yml`; `nix.yml` (flake eval only). Composite actions: `checkout-craft-fonts`, `resolve-version`, `install-wix`, `install-nfpm`, `install-linux-deps`, `install-riscv64-cross`, `install-trunk`, `draft-release`, `packaging-lint`.

Job time limits are declared explicitly (`README.md` table): packaging-lint 15 min, release version/draft 10 min, release per-OS builds 90 min, release FreeBSD 300 min, Windows ARM64 package/install 90/30 min.

**Comparison with pbiview's `release.yml`** (`.github/workflows/release.yml`, 612 lines) [verified from local repo]:

| Dimension | craft-common-actions | pbiview (current) |
|---|---|---|
| macOS | universal DMG, Developer ID + notarization | arm64+x64 DMG, `HAS_APPLE_SIGNING`/`HAS_APPLE_NOTARIZATION` gated (same secret-gating pattern), ad-hoc signature fallback when unsigned |
| Windows | x64/x86/**ARM64** MSI (WiX v5), **Azure Trusted Signing** | x64 only via WiX/NSIS (no ARM64 build found), cert-based signing only (no Azure Trusted Signing secrets referenced) |
| Linux | AppImage/deb/rpm/tar.gz, **+ Flatpak, + RISC-V64, + Arch, + FreeBSD** | AppImage + deb only |
| SBOM | not mentioned in craft-common-actions | **CycloneDX SBOM per binary** (`cargo cyclonedx`) + npm SBOM for the app, zipped per release (`release.yml:136,147-163`) — more rigorous than craft-common-actions on this axis |
| License notices | not mentioned | `THIRD_PARTY_NOTICES.md`/`THIRD_PARTY_LICENSES.txt` shipped in release |
| Fonts | `CRAFT_FONTS_REQUIRED=1` fails build if missing | N/A (pbiview uses its own Selawik bundling, not craft-fonts) |
| Signing fail-fast on `release` branch | explicit requirement | same secret-presence-gating pattern already used (`HAS_APPLE_SIGNING`, `HAS_WINDOWS_SIGNING` env checks) |

**Recommendations for pbiview's release pipeline**: (a) add Windows ARM64 (craft-common-actions proves the pattern including real-hardware smoke tests), (b) add Linux RISC-V64 and/or Flatpak if targeting a broader Linux audience, (c) consider adopting Azure Trusted Signing for Windows (industry-preferred over cert-only signing, especially since standalone code-signing certs are becoming harder to obtain for individuals), (d) pbiview's SBOM practice is already *more* rigorous than craft-common-actions' public release workflow — worth highlighting rather than changing, (e) do **not** `uses:` this repo directly (no LICENSE — see §8), but the *patterns* (per-secret fail-fast gating, per-job time budgets, draft-release + SHA256SUMS) are freely reusable ideas.

---

## 4. craft-fonts

`storytold/craft-fonts` (MIT OR Apache-2.0 code; created 2026-10-06) ships **only 7 non-Latin font files**, all Google/Google-Fonts-sourced, all **OFL-1.1**:

| Family | Scripts | Use |
|---|---|---|
| BIZ UDPGothic (Regular, Bold) | Jpan, Latn | Japanese UI sans |
| Shippori Mincho | Jpan, Latn | Japanese document serif |
| BIZ UDMincho | Jpan, Latn | Japanese document serif |
| Noto Sans CJK SC | Hans, Latn | Simplified Chinese |
| Noto Sans Arabic | Arab | Arabic |
| Noto Sans Thai | Thai | Thai |
| Noto Sans Georgian | Geor | Georgian |

(`fonts/manifest.txt`, each line: family, style, path, ISO-15924 scripts, license, license-file, SHA-256, pinned upstream-commit source URL.)

**No Latin/Segoe-UI/DIN/Calibri-class fonts are present** — craft-fonts exists purely to solve CJK/Arabic/Thai/Georgian rendering gaps for Latin-centric apps, **not** as a general UI-font library. [verified]

**Consumption mechanism** [verified, `docs/integration.md`]: apps **never** add `craft-fonts` as a Cargo dependency — a git dependency would force every build (offline, forks, mirrors) to need this repo. Instead it's an **optional build input via `CRAFT_FONTS_DIR` env var**, read in each app's `build.rs`, embedding only `fonts/manifest.txt`-listed files as `include_bytes!`. Unset → app builds fine without it (falls back to system fonts). `CRAFT_FONTS_REQUIRED=1` turns a missing/invalid checkout into a hard build error (used by release CI). Web/wasm builds use a hand-picked subset (`WEB_FONTS` const, currently just BIZ UDPGothic) for size-budget reasons (Cloudflare's 25 MiB/file cap cited).

**Metric-compatible substitutes for Segoe UI/DIN/Calibri** [inferred — not sourced from craft-fonts, general knowledge since the repo doesn't address this need]: pbiview's existing choice of **Selawik** (Microsoft's own open-source, SIL-OFL, metric-compatible Segoe UI substitute) is already the standard answer and nothing in craft-fonts improves on it. For Calibri-class metrics, **Carlito** (Google/Google Fonts, OFL-1.1, used by LibreOffice as Calibri's metric-compatible substitute) is the standard candidate. For Cambria-class, **Caladea** (OFL-1.1) pairs with Carlito. There is no widely-known OFL/MIT metric-compatible clone of **DIN**-family fonts specifically (DIN itself is typically used for chart/axis labels in some BI tools, not Power BI Desktop's default UI, which uses Segoe UI); if DIN-metric compatibility is genuinely required, options are either licensing a commercial DIN-alike (e.g., FF DIN) or substituting a visually similar open alternative (e.g., **Fira Sans Condensed** or **Archivo**, used elsewhere in this same org per `artcraft-native/README.md`) without claiming metric compatibility. **Gap**: craft-fonts offers nothing for this specific need — pbiview should keep sourcing these independently rather than depending on craft-fonts.

---

## 5. craft-ai-integrations (deep dive)

`storytold/craft-ai-integrations` (MIT OR Apache-2.0, created 2026-10-11) is explicitly *"the runtime half of the Craft AI layer"*; the pure wasm-safe contract (`craft-ai-core`) has been promoted into `craft-libs` already, but this repo still carries a local fork with newer consent-term/model-folder additions pending a documented sync gate (`docs/CORE-SYNC.md`).

**Full crate list** (`README.md`):

| Crate | Role |
|---|---|
| `craft-ai-core` | Data model + provider trait; serde-only, no I/O, wasm-compilable |
| `craft-ai-native` | Desktop-only: OS keychain / passphrase-encrypted-file / env secret stores, locked atomic shared-store writer, consent+history files |
| `craft-ai-client` | App-facing API: `AiClient` (status/plan/consent/start/`Job`), `ToolDesk`, `ConnectionManager`, `ProviderFactory` |
| `craft-ai-features` | Wasm-safe tool catalogue: where a tool appears, how it can run locally (model files + size + SHA-256 + source + licence), what's still needed |
| `craft-ai-http` | One shared TLS setting (platform cert store) for every adapter |
| `craft-ai-media` | Durable local copies of hosted media links |
| `craft-ai-openai` | OpenAI-compatible text (streaming/tools/structured output) + images |
| `craft-ai-bfl` / `craft-ai-vertex` / `craft-ai-seedance` / `craft-ai-artcraft` | Black Forest Labs FLUX / Vertex-express Gemini / BytePlus ModelArk Seedance / ArtCraft Omni-API-key route |
| `craft-ai-declarative` | Data-file-described provider (no Rust needed) |
| `craft-ai-comfyui` / `craft-ai-sam3` | Local ComfyUI workflow adapter (incl. imported SAM-based segmentation graphs) / SAM 3 point-and-session clients |
| `craft-ai-engine`, `-engine-candle`, `-engine-rten` | Built-in on-device model runner contract + Candle (SAM 3, Apache-2.0-only) + rten (SAM 2.1, pure-Rust ONNX reader) engines |
| `craft-ai-comfy-env` | Finds existing ComfyUI Desktop install, inventories/downloads model files |
| `craft-ai-egui` | Theme-neutral egui widgets (status, picker, consent, progress, history) |

**Capability/role model** [verified]: a `Role` = stable `RoleId` + label + one `CapabilityId` + non-empty `InputRole` set — a pure **declaration**, not installed code (`docs/ADOPT-A-TOOL.md`, `docs/PLATFORM-CONTRACT.md`). One connection is configured once (launcher or any app) and every app with that tool reuses it; apps keep their own UI per tool. `ToolDesk::register_tool`/`choose`/`remove_tool` operate on one versioned shared store file. Mismatched capability/input contracts on reuse are **rejected at resolution time**, not silently coerced.

**Provider adapters**: OpenAI-compatible, BFL/FLUX, Vertex-express Gemini, BytePlus Seedance, ArtCraft Omni API key, generic ComfyUI (incl. imported segmentation graphs), SAM 3 (local, Candle-based), declarative (data-file) providers, plus a path for arbitrary custom OpenAI-compatible connections (`docs/custom-connections-and-sources.md`): HTTPS required except loopback, key stored apart from the connection record, a **"Test" action re-probes without the key to confirm auth enforcement** rather than trusting an unauthenticated success.

**Local runtimes**: `craft-ai-engine-candle` (SAM 3; Apache-2.0-only license note; CPU or Metal on macOS; ran on a real photo), `craft-ai-engine-rten` (SAM 2.1; pure-Rust ONNX reader; **CPU-only, pinned by size+SHA-256**; measured ~35 ms median inference after a ~0.7 s first-look cost on one Windows machine — explicitly caveated as "not a benchmark").

**Consent & credential storage** [verified]: `craft-ai-native` layers OS keychain → passphrase-encrypted file → environment variable secret stores (`ConnectionManager::standard()` currently **omits** the encrypted-file store — STATUS.md flags this as a real gap on keychain-less Linux). Consent is scoped per **app + connection + destination + capability + data-kind**, can expire, and a grant in one app does not transfer to another app (`docs/ADOPT-A-TOOL.md`). Model-file downloads require SHA-256 verification *when the source publishes one*; when it doesn't, installs proceed only after explicit consent and are marked **"observed, not verified"** rather than claiming cryptographic provenance (`docs/custom-connections-and-sources.md`). Non-safetensors/gguf/onnx formats (`.ckpt`,`.pt`,`.pth`,`.bin`,`.pkl`) require **explicit consent before install** because they can execute code when loaded elsewhere. GitHub node-pack installs pin to a **resolved commit SHA**, not a moving branch/tag — consent to an old SHA does not carry forward if the ref moves. Archive extraction treats downloaded tarballs as hostile (refuses symlinks/device files/path escape/reserved names/oversize).

**Model recipes with SHA-256 + licence acceptance** [verified]: four source types supported (Hugging Face, GitHub release asset, Civitai, local file), each tracked for size+checksum provenance per `docs/custom-connections-and-sources.md`'s table; licence acceptance is required for gated HF repos, any non-permissive licence (permissive allow-list: Apache-2.0/MIT/BSD-2/BSD-3/ISC/Unlicense/CC0-1.0), and always for Civitai/local files; acceptance is version-and-terms-scoped (changes require re-acceptance).

**Test strategy** [verified]: unit + conformance tests (`run_lifecycle`/`run_cancel`/`run_rejects`/`run_unknown_job` shared checks any `Provider` must pass); a **mutation-testing harness** (`tools/mutation/`) that deliberately breaks one safety rule at a time, confirms the relevant test fails, then restores the file — "every survivor is either a test gap or a bad mutant," run per OS (Ubuntu+macOS) via a named-group CI workflow. `cargo-deny` gates licenses/advisories (see deny.toml below). Keychain-touching tests are `#[ignore]`d and run on demand.

**`STATUS.md` honesty sections** [verified — notably rigorous]: three explicit tiers — *"Built and tested against scripted servers only"* (every hosted adapter, SAM3/SAM2.1 engines, ComfyUI generic adapter, declarative adapter, client flow), *"Run for real"* (only local SAM3 point/session clients on one Windows machine, evidence not retained in-repo), and *"Not verified"* (any real hosted provider in any adapter; TLS behind an intercepting proxy; macOS/Linux keychains — CI only runs Linux+Windows; model download from HF; **the recommended-models catalogue is literally empty because "which models to recommend... is a curation decision"**; and critically: **"App adoption. PhotoCraft's dev build uses the SAM 3 clients directly, not the shared role, connection and consent flow."**). This candor pattern (what's tested-against-mocks vs. run-for-real vs. unverified, stated plainly rather than implied) is a notable governance/documentation practice.

**No app currently consumes this layer** [verified]: a code search for `craft-ai-integrations` or `craft-ai` dependencies in `photocraft:Cargo.toml` and `craft-launcher:apps/artcraft-launcher/Cargo.toml` returned no matches — confirming the README's own admission that the shared AI layer is built but **not yet wired into any production app**.

**Applicability to BI AI features (NL→DAX/M, explain-measure, lint fixes)** [inferred]: the `craft-ai-core` contract (connections/models/roles/consent/jobs, wasm-safe, zero I/O) is a reasonable **design template** to borrow conceptually for pbiview's own NL→DAX feature — same idea of "declare a Role with one capability + required inputs, let the user bind any compatible model/provider, consent scoped per destination" maps cleanly onto "nl-to-dax" / "explain-measure" / "lint-fix" as named roles. However, **direct dependency reuse is not recommended**: (a) it's unadopted by any app yet (zero production mileage, by its own STATUS.md), (b) it's `egui`-flavored (the widget crate) and git-rev-only (no crates.io), (c) pbiview's AI surface is narrower (text-in/text-out DAX/M, no image/video/3D generation, no local ComfyUI/SAM engines) so the 16-crate surface is overkill. The more actionable path is to **study and adapt the pattern** (role/capability/consent/provider-factory separation) in a much smaller bespoke Rust module, rather than taking the dependency.

---

## 6. craft-repo-status-app

`storytold/craft-repo-status-app` (Cargo.toml declares `license = "MIT"`, but **no LICENSE file is actually committed** — a licensing gap) is an internal **tray dashboard** that polls every `storytold *craft` repo every 30 min via GitHub GraphQL/REST, scores open issues by an explicit urgency heuristic (crash/panic/hang/security keywords +40, `bug` label +15, external reporter +10, reaction/comment log-scaled up to +40, feature-request −25, stale 30d −5), tracks release cadence (unreleased-commit counts vs. last tag), and surfaces a "Fix first" ranked list.

**Stack** [verified, `README.md`]: Rust workspace — `craft-core` (pure model/heuristics/cache, no network/UI deps, unit-tested), `craft-github` (GraphQL/REST fetcher + debug CLI `craft-fetch`), **`craft-status-app`: "the Tauri 2 shell (poll thread, tray, global shortcut, window)."** UI is hand-rolled TypeScript/HTML/CSS (no framework/bundler), compiled by `tsc`, tested with `node --test` + happy-dom.

**Why Tauri here but not in the apps** [verified + inferred]: the repo's own docs give no explicit justification paragraph contrasting it with the egui-first app policy, but the structural reasoning is inferable from context: this is a small **tray/menu-bar utility** whose UI is "TypeScript, HTML and CSS... with no framework or bundler" — i.e., trivial static HTML/CSS/vanilla-TS, not a full creative-editor canvas. For a thin settings/table/tray surface, Tauri's native webview is proportionate (lighter dev overhead, no need for a heavyweight immediate-mode rendering pipeline); the apps' AGENTS.md rules (PhotoCraft: *"No Tauri, Electron or webview shells"*) are specifically about the **flagship creative-editor products**, where egui+wgpu gives tighter control over custom drawing/canvas/performance and a single Rust-only codebase drivable by agents end-to-end. The craftrules standard that would state this explicitly is not publicly visible (craftrules is private), so this reasoning is **[inferred]**, not directly quoted from a storytold source.

---

## 7. Governance & process across the org

**craftrules (private)** [verified existence, inaccessible]: `gh api repos/storytold/craftrules` → 404; confirmed private and referenced from essentially every public repo's `AGENTS.md`/`CLAUDE.md`/docs (`photocraft:AGENTS.md:7`, `craft-libs:AGENTS.md`, `craft-i18n:AGENTS.md`, `photocraft:docs/contributors.md`, `docs/release-playbook.md`, `pdfcraft`/`filmcraft`/`wordcraft`/`lightcraft`/`vectorcraft`/`effectcraft` AGENTS.md files — all surfaced via org-wide code search for "craftrules"). `photocraft:AGENTS.md:7` states plainly: *"the repository is not public — outside contributors should ask a maintainer for the standards relevant to their change."* Named standards consistently cited: `standards/never-crash.md`, `standards/engineering.md`, `standards/licensing.md`, `standards/fonts.md`, `standards/contributors.md`, `standards/progress-docs.md`, `standards/test-corpora.md`, `release/playbook.md`, plus a `scripts/contributors.py` generator.

**AGENTS.md conventions** [verified]: near-universal across repos (artcraft, artcraft-services, craft-libs, craft-fonts, craft-i18n, craft-launcher, photocraft-corpus, craft-ai-integrations via CONTRIBUTING.md, every `*craft` app). Common structural pattern: project overview → repo layout → code style ("never crash" rules, `#![deny(clippy::unwrap_used, ...)]`, `unsafe_code = "forbid"`) → test/lint commands to run before finishing → where to look for current work. PhotoCraft's is the most elaborate (10 numbered "golden rules," layering enforcement via `cargo xtask layers`, an MCP server + "control protocol" for agent-driven UI control at layer L6, a `panic_hunt` fuzz test that must stay green).

**Brand/trademark policy** [verified, see §1]: standardized `docs/brand/LICENSE-brand.txt` text, byte-identical across `photocraft` and referenced from `artcraft`/`photocraft-corpus` — name/wordmark/logo usable unmodified only to attribute "made by ArtCraft," never to imply endorsement of forks, and **must be stripped from any fork/derivative**.

**Contribution/credit model** [verified — distinctive]: `photocraft:docs/contributors.md` documents a two-file, build-time-compiled credit system: `contributors/contributors.json` (auto-generated by `craftrules/scripts/contributors.py` from git stats — commits/PRs/lines added/removed/binary-asset churn/first-last-commit-date) + `contributors/people.toml` (opt-in self-asserted display names, each contributor adds their own row in their own PR, script verifies the committer matches). Critically: **"About ▸ Models" credits the AI models named in `Co-Authored-By` trailers** — i.e., AI coding agents receive first-class, structurally-equal credit alongside human contributors in the shipped app's About dialog.

**AI-agent-driven development evidence** [verified]:
- `storytold/photocraft` commit history: **9 of the last 100 commits** carry `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` trailers.
- `photocraft:AGENTS.md`: *"every feature drivable by agents"* is a stated design goal, with a JSON "control protocol" (`docs/control-protocol.md`) letting agents drive and screenshot the running app, and an MCP server living at architecture layer L6 (`ui-egui automation`).
- Commit authors across `artcraft` are a small human core team (`echelon`=Brandon Thomas, `bflatastic`=BFlat, `dylanmtaylor`=Dylan M. Taylor, `tanish2k09`=Tanish Manku — all also the org's only 3 *public* members plus the founder), but the velocity is extreme for that team size: **photocraft shipped v0.2.0→v0.12.0 (10 releases) in 16 days (Oct 5→Oct 21, 2026)**.
- **Repo-creation timeline is a striking signal**: `craft-fonts` (Oct 6), `craft-libs`/`craft-launcher`/`craft-repo-status-app` (Oct 9), `craft-ai-integrations`/`craft-common-actions` (Oct 11), `craft-i18n` (Oct 16) — essentially the **entire shared-infrastructure layer was stood up in a ~10-day window** immediately preceding this research snapshot (Oct 22). Combined with the Claude co-authorship trailers and the explicit "models get in-app credit" policy, this is strong circumstantial evidence of a largely agent-driven build-out of the whole ecosystem in a short burst — **[inferred conclusion from verified data points]**.

**Release cadence / issue volume** [verified]: photocraft releases roughly every 1–3 days; `artcraft` has 490 open issues / 5621 forks on a 35.5k-star repo; `craft-launcher` (alpha-stage) already has 486 open issues against 3824 stars — unusually high issue-to-star ratio for a brand-new repo, consistent with rapid iterative/agent-assisted development generating many small tracked issues. `craft-repo-status-app` itself exists specifically to manage this issue/release velocity across ~15+ repos.

**Decision process**: `craft-contrib:rfcs/0000-decision-process.md` — changes affecting multiple apps go through RFC PRs; maintainers = GitHub team `craft-app-team`; comment periods are 7 days (new/removed app or process changes) or 3 days (everything else); merge requires a second maintainer's approval with no unresolved objection; org owners can short-circuit.

---

## 8. Recommendations for pbiview / DashCraft

**Alignment opportunities** (low-risk, high-value):
1. **README/brand style**: already modeled on photocraft per your context — this is validated as the right template; the org's README pattern (badges row: SonarCloud + CodeRabbit + Discord + license; feature table with GIF/video previews; explicit "License and credits" section separating code license from trademark carve-out; a closing "Star history" chart) is consistent across `artcraft`, `photocraft`, `craft-launcher`, `photocraft-corpus`.
2. **AGENTS.md / CLAUDE.md convention**: adopt the same structure (orientation table → workspace map → "golden rules" incl. never-crash/no-unsafe/clippy-deny → "picking work" pointers → pre-finish checklist). This is clearly the org's dominant and apparently effective agent-collaboration pattern, and pbiview's CLI/MCP/command-registry design (per the prior review) already rhymes with photocraft's L6 "everything is a command" + MCP-server architecture — worth making that parallel explicit in AGENTS.md.
3. **craft-common-actions *patterns*, not the dependency**: borrow per-secret fail-fast gating on the `release` branch, per-job time budgets, and the draft-release+SHA256SUMS flow. **Do not** `uses:` the repo directly or vendor its YAML verbatim — **it has no LICENSE file**, so its copyright status is ambiguous; treat it as read-only inspiration.
4. **craft-libs crates worth actually depending on**: `craft-atomic` (crash-safe save for the pbiview document/workspace format) and `craft-settings` (config.toml storage with atomic-save + schema migration) are mature, dual-MIT/Apache, headless, and framework-agnostic — genuinely reusable from a Tauri+Svelte app. `craft-docpath`'s same-share path-policy logic is worth reading (even if not depended on) given pbiview will resolve external data-source file paths from `.pbix`-equivalent documents, a similar trust boundary.
5. **AI-layer design, not dependency**: borrow the `craft-ai-core` *shape* (Role = id+label+one capability+inputs; per-app/connection/destination/data-kind consent scoping; Provider trait with shared conformance tests `run_lifecycle`/`run_cancel`/`run_rejects`/`run_unknown_job`; STATUS.md's three-tier honesty framework) for pbiview's NL→DAX/explain-measure/lint-fix AI features. Do not take the crate dependency — it's unadopted-in-production (its own admission), egui-coupled on the widget side, and far broader in scope (image/video/3D generation, local ComfyUI/SAM runtimes) than a text-only BI assistant needs.

**Risks**:
- **"Craft" naming/brand**: the `{Function}Craft` PascalCase naming convention and the "ArtCraft" trademark are explicitly protected and **not** licensed for third-party reuse (`docs/brand/LICENSE-brand.txt`: *"must never suggest that the ArtCraft Team made, sponsors, endorses... any other product... fork... or person"*). **DashCraft is an independent, non-affiliated project** — it must not imply storytold/ArtCraft affiliation, sponsorship, or membership in "the Crafting Apps" family. Given that `{X}Craft` is both (a) a generic English compound word and (b) heavily trademark-asserted by this specific, well-known, 35k-star org for an entire product family, there is real brand-confusion risk in using "DashCraft" alongside a README styled identically to photocraft's. **Recommend**: keep the placeholder name under review; if retained, add an explicit, prominent disclaimer ("DashCraft is an independent project, not affiliated with, sponsored by, or endorsed by ArtCraft/storytold or Microsoft") and avoid visually mimicking the ArtCraft badge/logo row.
- **Licensing of any copied patterns**: craft-common-actions, craft-contrib, crafthub, artcraftx, craft-repo-status-app all lack LICENSE files — treat any code/workflow snippets sourced from them as **view-only reference**, not copy-paste-safe, absent explicit permission.
- **Git-rev-pinned, unpublished dependencies**: every craft-libs crate is consumed via `git + rev`, not crates.io — if pbiview takes any craft-libs dependency, it inherits a supply-chain pattern (no semver, no crates.io audit trail, pin breakage risk if storytold force-pushes/deletes `main`) that your own `deny.toml`/Cargo policy should explicitly account for (e.g., vendor a copy, or mirror the specific commit internally, rather than trusting `git` resolution long-term).
- **No evidence of a stable, versioned public contract** for craft-ai-core/craft-ai-integrations — adopting early risks churn (the repo's own docs flag an unresolved fork-vs-upstream sync gate between craft-ai-integrations and craft-libs's copy of the same contract).

**Prioritized action list**:
1. Decide/clear the "DashCraft" naming risk before further public investment (trademark search + explicit non-affiliation disclaimer at minimum).
2. Write/expand `AGENTS.md` in pbiview using the photocraft/craft-libs structural template (orientation → map → golden rules → pre-finish checklist), cross-referencing the existing command-registry/CLI/MCP design.
3. Vendor-copy (not git-dependency) the design pattern from `craft-atomic` for pbiview's document save path if not already crash-safe; same for `craft-settings`'s atomic config pattern.
4. Add Windows ARM64 + (optionally) RISC-V64/Flatpak to the release matrix, modeled on craft-common-actions' proven jobs, while keeping pbiview's already-stronger CycloneDX SBOM practice.
5. For the planned NL→DAX/explain-measure AI features, draft a small bespoke Rust "role/capability/consent" contract inspired by `craft-ai-core`'s shape rather than depending on craft-ai-integrations, and apply its STATUS.md-style "tested against mocks / run-for-real / not verified" documentation discipline from day one.
6. Keep monitoring `storytold/artcraft`'s Tauri→egui (`artcraft-native`) migration as a leading indicator — if the org's own flagship product fully drops Tauri, it reinforces (but does not mandate) reconsidering Tauri 2 for pbiview's long-term UI architecture, echoing the prior review's photocraft finding.

---

## Gaps and uncertainties

- `storytold/craftrules` content is entirely inaccessible (private, 404) — all governance/standards claims attributed to it come from **secondhand references** in public AGENTS.md/docs files, not the primary source. Treat "never-crash.md," "engineering.md," "licensing.md," "fonts.md," "contributors.md," "progress-docs.md," "test-corpora.md," "release/playbook.md" as **named-but-unread** documents.
- No org-wide profile README exists (`storytold/.github` and `storytold/storytold` both 404) — there is no single canonical "org README" beyond `artcraft`'s own.
- Hit a GitHub code-search rate limit mid-session; a few planned cross-repo dependency searches (e.g., exhaustively confirming *zero* apps depend on craft-ai-integrations beyond the two checked) were done via direct Cargo.toml inspection of `photocraft` and `craft-launcher` only, not all 13 apps — reasonably representative but not exhaustive.
- craft-ai-integrations' own STATUS.md explicitly states its claims are self-limited (no macOS CI, no real hosted-provider testing, empty model-recommendation catalogue) — these are the project's own disclosed limits, not gaps in this research.
- The 60,444 org "followers" figure (`gh api orgs/storytold`) is unusually large relative to repo star counts and wasn't independently verified/explained; noted as an observed data point only, not analyzed further.
- Did not attempt to enumerate the full 13-app Craft suite's individual AGENTS.md/CLAUDE.md files beyond photocraft (used as the representative, most-detailed example) — a few spot-checks (wordcraft, filmcraft, lightcraft, vectorcraft, effectcraft, pdfcraft) were confirmed to also reference craftrules via code search but their content wasn't read in full, since they're apps rather than shared infra and outside this task's primary scope.