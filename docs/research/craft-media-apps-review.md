<!-- Research by a Copilot research agent, 2026-10-22, from public storytold/* repos (read-only). -->
# storytold "Crafting Apps" Research Report — FilmCraft, EffectCraft, LightCraft, VectorCraft, SoundCraft, CADCraft, FigCraft, AniCraft

**Methodology note:** Repo names confirmed via `gh api orgs/storytold/repos --paginate`. All 8 target repos were deep-dived via GitHub API (`gh api`), raw file fetches, and GitHub code search — read-only, nothing modified. Cross-repo infrastructure (`craft-libs`, `craft-ai-integrations`, `craft-fonts`, `craft-i18n`, `craft-common-actions`, and patched `egui`/`winit` forks) was investigated directly. All dates are per the sandbox's clock (repo creation dates show 2026-09/10, i.e. ~3 weeks old at research time, pushed as late as 2026-10-22). Claims are tagged **[verified]** (read directly from source/docs/API) or **[inferred]** (reasonable extrapolation).

---

## Org repo confirmation

```
filmcraft, effectcraft, lightcraft, vectorcraft, soundcraft, cadcraft, figcraft, AniCraft
```
all exist exactly as named. No additional "*craft" **app** repos exist beyond the full family (`photocraft, pdfcraft, wordcraft, deckcraft, designcraft, gridcraft` — excluded per your scope — plus `artcraft`/`artcraftx`, which are a separate "crafting engine" product, not an Office-app clone). Shared infra repos: `craft-libs`, `craft-ai-integrations`, `craft-fonts`, `craft-i18n`, `craft-common-actions`, `craft-launcher`, `craft-repo-status-app`, a patched `egui` fork, a patched `winit` fork, and `winprint.rs` fork. `storytold/craftrules` (referenced from several AGENTS.md files as the cross-app standards repo) returns **404** — either private or renamed/removed **[verified absence, not publicly readable]**. **[source: `gh api orgs/storytold/repos`]**

---

## 1. FilmCraft (Premiere Pro clone)

**Purpose:** Full NLE — 3-point editing, ripple/roll/slip/slide/razor trims, multicam, Lumetri-equivalent color (wheels, curves, HSL secondary, HDR), 90+ effects/16 categories, ~110 transitions, Bézier masks/keyframing, BS.1770 loudness + Track/Clip Mixer automation, titles/captions (CEA-608/708), Whisper STT + Kokoro-82M TTS, broad interchange (FCP7XML, FCPXML, OTIO, EDL, AAF, OMF). `README.md`; `docs/architecture.md:§5.1-5.4` **[verified]**

**Maturity:** 15,702★ / 6,591 forks, created 2026‑09‑30, 2,109 commits in ~22 days, 45 crates, ~343k LOC, 2,520 tests, 69 contributors (top: `claude` 830 commits, `echelon` 420 — **AI-agent-authored**). Self-graded: **86% feature breadth, 56% "ready for real work."** `docs/gaps.md` G4: **no CI gate runs tests on PRs** — only release/packaging workflows exist. No `CHANGELOG.md`; progress tracked in `ROADMAP.md`/doc revision-history tables. `filmcraft:ROADMAP.md:19`, `docs/gaps.md:1-40,70-90` **[verified]**

**Tech stack:** `egui = "0.36"`, `eframe = "0.36"` (features `default_fonts, wgpu, accesskit`), no Tauri/iced/slint anywhere. GPU compositor (`filmcraft-gpu`, WGSL) implements 27 blend modes + ~20 effects vs CPU-linear-float reference, tested p99 diff ≤6/mean<1.5. **Video/audio decoders are clean-room pure-Rust** (own H.264/HEVC/VP8/VP9/AV1/ProRes/DNxHR/AAC/AC-3/Opus crates) — ffmpeg used **only as an external test oracle**, never linked/shipped; HW accel (VideoToolbox/MediaFoundation/VA-API) isolated in the one `unsafe`-permitted `platform` crate. Containers (MP4/MOV/MKV/MXF/AVI/MPEG-TS) are hand-written parsers. `filmcraft:Cargo.toml:49-54`; `crates/gpu/Cargo.toml:11-22`; `AGENTS.md:52-58` **[verified]**

**Architecture:** Strict L0–L6 layered workspace enforced by **`cargo xtask layers`** (reads `cargo metadata`, fails on upward deps or UI crates below L5) — the same pattern photocraft uses, confirmed shared across the family. `Project→Bin→ProjectItem` doc model, 200-entry `Arc<Project>` snapshot undo stack. **~650 commands**, one `CommandSpec` registry shared by UI/CLI/control-channel/**MCP server** (`rmcp` crate). `AGENTS.md`/`CLAUDE.md` "Never crash" rule (`unsafe_code="forbid"` except `platform`), clean-room rule. `docs/architecture.md:24-33,146-165`; `xtask/src/main.rs:108-153` **[verified]**

**Perf for scrubbing:** Prioritized frame-worker pool (on-screen > prefetch > thumbnails) with catch-up/draft-decode (½-res); GOP-decoder pool capped at 4 live decoders (3s idle eviction) + **global 1 GiB frame budget** (fixed a 12GB-footprint regression); `cargo xtask bench` headless perf harness. Explicitly **not yet virtualized**: "the transcript still lays out every word; list virtualization is future work." `docs/performance.md:13-68` **[verified]**

**Testing:** No `fuzz/`/cargo-fuzz — mutation/truncation fuzzing lives as ordinary `#[test]`s (`tests/robustness.rs`). Per-codec bit-exactness criteria vs ffmpeg oracle; golden-image tests (PSNR≥45dB) in `crates/golden`; GPU-vs-CPU parity tests (found a real GPU-cache bug); headless UI tests via `egui_kittest` driving the control-channel. **No media corpus repo yet** — `docs/gaps.md` G6 explicitly names PhotoCraft's `photocraft-corpus`/`cargo xtask corpus` as the pattern it hasn't built. `docs/testing.md` **[verified]**

**AI:** Local Whisper STT (candle, off by default in release) + Kokoro-82M TTS (candle). **No `craft-ai-integrations` dependency.** `crates/speech`, `crates/tts` **[verified]**

**Reusable crates:** Consumes `craft-ui`, `craft-docpath` from `craft-libs`. **Notably, `craft-media` (isobmff/matroska demux) was extracted FROM FilmCraft's own code** for reuse by LightCraft/FigCraft/SoundCraft/EffectCraft — but FilmCraft itself hasn't migrated back to consuming it (one-way extraction so far). `craft-libs:crates/media/README.md` **[verified]**

---

## 2. EffectCraft (After Effects clone)

**Purpose:** Compositing/layers (38 blend modes, nested comps, track mattes), full keyframing + Graph Editor, Roto Brush 2.0/3.0, JS expressions (boa engine, AE object model), 306 effects/9 categories, Classic+Advanced 3D (glTF/PBR/IBL), Motion/Warp/3D-Camera tracking, Lottie import/export, Content-Aware Fill, Scene Edit Detection. **[verified]**

**Maturity:** 7,680★/4,115 forks, created 2026‑10‑01, 97 contributors (top: `claude` 976). Self-graded **91% breadth, only 45% "ready for real work."** `.aep` import **entirely unsupported (0%)** — biggest adoption blocker. No PR-gating CI. `ROADMAP.md:9-36`, `docs/gaps.md` **[verified]**

**Tech stack:** Same egui 0.36/eframe 0.36/wgpu 30 stack, confirmed via `Cargo.lock` pins. **GPU compute compositor**: 280/306 effects run as WGSL compute kernels; an `Auto` backend picker **benchmarks CPU-vs-GPU per composition every 48 frames** and picks the faster one (light comps stay CPU to avoid readback overhead) — a technique directly relevant to render-canvas perf tuning. `crates/gpu`, `docs/architecture.md` §4 **[verified]**

**Architecture:** L0–L6 layering (`cargo xtask layers`), 34 crates. **Branching undo history** (not linear) — MCP `history{goto}` tool exposes every state with branch depth, jumpable without losing other branches. 587 menu entries, same command-registry-shared-by-UI/CLI/control/MCP pattern. `docs/architecture.md:94-156`, `docs/agents.md` **[verified]**

**Perf for scrubbing:** Richest documented caching of all 8 repos — **layer cache keyed by evaluated-input hash** (static/transform-only layers render once); **RAM-preview cache shows the newest completed revision while the exact one renders**, never regressing backward on out-of-order completions; resolution auto-scaling (Full/Half/Third/Quarter matching AE); **persistent LZ4 disk cache**, 128-bit content keys; deferred/async GPU readbacks for WebGPU workers; device-loss fallback halves GPU frame budget. `docs/architecture.md` §4 (longest section in the doc) **[verified]** — self-rated only ~35% perf readiness though, citing user-reported stutter issues.

**Testing:** Golden pixel-hash tests for GPU sim effects; export round-trip tests; **fidelity harness against a live After-Effects session via "AEsync" bridge** (`.jsx`+`.mjs` scripts) — found/fixed a real ease-curve bug (12.46°→<3e-11° error). No fuzz/ dir; embedded project-file mutation fuzzer as a regular test. `docs/fidelity/README.md`, `docs/testing.md` **[verified]**

**AI:** Roto Brush (MobileSAM/TinyViT) + Face Tracking (MediaPipe via pure-Rust TFLite interpreter), both **local-only, housed in `effectcraft-segment`**. **Does NOT use `craft-ai-integrations`** despite it existing — a notable ecosystem inconsistency. `docs/gaps.md` G7 **[verified]**

**Reusable crates:** Pulls `craft-pdf`, `craft-docpath`, `craft-ui` from craft-libs (plus patches `hayro-*`/`krilla`). Consumes `filmcraft-*` codec crates directly (pinned git rev) rather than via craft-media. **[verified]**

---

## 3. LightCraft (Lightroom clone)

**Purpose:** Catalog (smart albums, stacks, field-aware search), full Develop module, RAW decode for 27 vendor formats (DNG/CR2/CR3/ARW/NEF/RAF+X-Trans/RW2/ORF/etc., own demosaic: AHD/bilinear/PPG/X-Trans), masking (brush/linear/radial + AI Object/Describe via SAM3 + heuristic Subject/Sky), XMP sidecars + pure-Rust `.lrcat` reader, HDR/gain-map export, Print/Slideshow modules. **[verified]**

**Maturity:** 14,293★, created 2026‑09‑30, **~3,476 commits in 22 days** (highest velocity of the family), 181 contributors; `contributors/contributors.json` explicitly attributes commits to named AI models (Claude Opus 5.5: 747 commits/178k LOC added). Self-graded alpha, **81.6% breadth, 61% "ready for real work."** `docs/whats-new.md` gives daily user-facing changelog entries. **[verified]**

**Tech stack:** egui/eframe/wgpu confirmed; **candle-core/candle-nn for SAM3 inference**. GPU develop pipeline (WGSL blur/resize/mask/nn_conv/nn_pool shaders) checked within 1/255 of CPU oracle; **per-platform backend forcing** (Windows→DX12-only, macOS→Metal, Linux→Vulkan, **no GL compiled in at all**) to dodge a real Intel-UHD-630-Vulkan-driver crash (igvk64.dll). `docs/gpu-pipeline.md` **[verified]**

**Architecture:** 28 crates, L0–L6. Catalog is a **crash-safe append-only op-log + periodic snapshot** (`catalog.snap`+`catalog.log`, CRC-32 per line, documented torn-append recovery, background compaction thread) — a genuinely novel document-persistence design vs. the other apps' plain-JSON formats. 236 commands (`cmd!()` grep-verified). `crates/catalog/src/journal.rs` **[verified]**

**Perf — most relevant to our use case (large library scrolling):** `crates/ui-egui/src/panels/grid.rs` is **explicitly virtualized**: "only visible cells request thumbnails," with a `GridStats` test asserting an unchanged frame rebuilds nothing. Thumbnail disk cache is content-hash-keyed with LRU byte-budget pruning; a generic cost-bounded `Lru<K,V>` uses **a single global monotonic tick shared across multiple cache instances** so one memory budget evicts globally-LRU entries across caches. Per-slot job de-dup: "submitting for a slot replaces a job that hasn't started yet, so scrolling never builds a backlog." **Explicit perf budgets in AGENTS.md: slider update ≤16ms (draft)/loupe ≤60ms on 2.5MP; export ≤1s/24MP.** Scale-tested to 100k photos (smart-album lookup dropped from 0.6–2.7s to ~14ms via indexing). `crates/preview/src/{disk,lru,pool}.rs`, `AGENTS.md` **[verified]**

**Testing:** CPU pipeline is the explicit "oracle." No cargo-fuzz; `proptest`-based decoder fuzz tests instead. RAW fidelity measured against CC0 raw.pixls.us corpus with pinned SHA-256 hashes, ΔE vs. camera JPEGs. Gap: "tests don't run in CI on macOS/Windows/Linux x64; only FreeBSD runs `cargo test`." **[verified]**

**AI:** SAM3 masking (candle, optional 3.4GB download, gated by license acceptance, with a **remote-Metal-worker-over-SSH mode** for non-Mac machines), AI RAW denoise (user-supplied ONNX, RawNIND opt-in GPL download), face detection (hand-written pure-Rust ONNX interpreter, YuNet+SFace). **No `craft-ai-integrations` dependency.** **[verified]**

**UI pain points (richest in the family):** Extensive GPU driver workaround doc (`docs/gpu-pipeline.md`) incl. a found-and-fixed real **DX12 FXC shader-compiler bug that silently corrupted every-other-pixel's channels** in a denoise kernel, and a Metal WGSL shared-memory-write correctness bug. **Custom text-field widget built because IME/selection edge cases were a problem** (`docs/ui-parity.md:34`, rated only 70%). Display-P3/color-management workaround doc for macOS (`wgpu` leaves the Metal layer color-space-less, causing oversaturation vs. exports — worked around by independently polling the OS profile). **[verified]**

---

## 4. VectorCraft (Illustrator clone)

**Purpose:** Pen/Bézier, Pathfinder/Shape Builder/Live Paint, gradient mesh, typography (bidi/CJK), SVG/PDF/`.ai`/EPS/DXF/EMF-WMF import-export, Image Trace, 3D Revolve, Affinity-file (`.af`/`.afdesign`) import. **[verified]**

**Maturity:** 10,408★/5,095 forks, ~4,911 commits, **top contributor `bflatastic` alone has ~62% of all commits** (unusually centralized vs. siblings). Self-graded 88% breadth/75% depth/62% ready-for-real-work. **[verified]**

**Tech stack — the one real outlier in rendering pipeline:** egui+eframe confirmed, **but the canvas is rasterized by `vello_cpu` (SIMD sparse-strip CPU rasterizer), not GPU** — "the GPU only composites the result... at the cost of GPU-accelerated preview on huge documents" (explicit, named gap G15). **No `lyon` anywhere**; `tiny-skia`/`resvg` appear only as (a) `egui_extras`'s SVG-icon-loading transitive dep and (b) a **dev-dependency oracle** for SVG-import golden tests, never the main canvas. `docs/architecture.md:36-39`, `docs/hardware-parity.md:12` **[verified]**

**Architecture:** 8-layer hierarchy (L0–L7, one more layer than siblings), **~710-command registry**, MCP with "Remote" (forward to running app) and "Headless" (in-process) backends. Hand-written writers for EPS/DXF/EMF-WMF (fully custom, no external crate). **[verified]**

**Perf:** Named per-budget `FrameCache` LRU (separate byte budgets for text/clip/FX-art/placed-art/brush/live/mesh/pattern/freeform — 9 distinct caches!), 3-frame aging. **No spatial index (quadtree/R-tree) found** — hit-testing is explicit scene-tree traversal with bbox-culling (a prior O(n²) approach was documented as fixed). `vectorcraft-cli perf` enforces **concrete ms budgets on a synthetic 50k-path doc** (render-fit-page ≤100ms, 400%-zoom ≤16ms, hit-test ≤2ms, handle-drag ≤8ms/move) but is a **manual/local gate, not CI-wired**. `crates/render/src/lru.rs`, `apps/vectorcraft-cli/src/perf.rs:320-400` **[verified]**

**Testing:** `resvg`-as-oracle golden image tests; `cargo-fuzz` harness exists for the Affinity-file parser (`crates/affinity/fuzz/`) — **the only repo of the 8 with a literal cargo-fuzz target**; pinned/sha256-verified public Affinity-file corpus fetched in a dedicated CI workflow (`affinity-corpus.yml`). **[verified]**

**AI:** Zero — "Generative AI | 0% | 30–60h" explicitly listed as unstarted, pending an "owner decision on models/providers." Image Trace is classical CV (median-cut+k-means quantization, Oklab ΔE, Guo–Hall thinning), not ML. **No `craft-ai-integrations`.** **[verified]**

**UI pain points:** Hand-rolled docking is **actively migrating to the shared `craft-ui` docking surface** (ROADMAP UI.4/UI.5) — a live, in-progress cross-app convergence. Extensively documented Wayland **drag-and-drop and tablet/pen-input breakage** due to winit 0.30 lacking the Wayland DnD/tablet protocols (fixed upstream only in winit 0.31) — same root cause independently documented by FilmCraft/EffectCraft. Also documents an egui font-family panic risk ("egui panics drawing text in an unbound family") and a GPU-adapter-crash-recovery restart loop. `docs/development.md` **[verified]**

---

## 5. SoundCraft (Pro Tools clone)

**Purpose:** Multi-track timeline (playlists, crossfades, Shuffle/Slip/Spot/Grid edit modes), mixing console (full signal chain, up to 16-ch Atmos-style surround), CLAP/VST3/AU plugin hosting (no AAX), 7-mode automation, MIDI editing (no HW I/O yet), Beat Detective/Elastic Audio, video track with own H.264/ProRes decoders. **[verified]**

**Maturity:** 3,562★ (smallest of the 8), created 2026‑10‑07, 43 contributors. **Explicitly "pre-alpha"** (~40% ready) — 3 of 6 core workflows fail end-to-end (interface recording, MIDI recording, session interchange). **[verified]**

**Tech stack:** egui/eframe (`wgpu`, `glow` GL fallback). **Waveforms are custom `egui::Mesh` triangle strips, GPU-rasterized via egui's own wgpu backend** (not CPU-blitted images) — `crates/ui-egui/src/waveform.rs`. **Peak-mipmap structure**: level-0 32-sample blocks (min/max/RMS), each level merging 4 child blocks → **O(log n) exact range queries at any zoom** — the single cleanest "virtualization for scrubbing" primitive found across the whole family, but kept **in-memory only, no on-disk peak-file cache**. `crates/audio-io/src/peaks.rs` **[verified]**

**Architecture:** 14 crates, L0–L6. **Realtime-audio-thread discipline is the most rigorous in the family**: UI thread owns `Player`; audio callback owns `MixEngine` + an `Arc<Session>` snapshot swapped by the UI; commands flow in via channel, meters/position flow out via atomics + `try_lock` (**audio callback never blocks**); recording uses a **lock-free SPSC `CaptureRing`** built on `Box<[AtomicU32]>`; `rayon`-parallel independent-strip mixing; plugin instances are created/destroyed strictly off the audio thread. This realtime-safety pattern is directly transferable to any renderer thread in pbiview that must never stall. `crates/playback/src/lib.rs:4-9`, `record.rs:43-76` **[verified]**

**Perf:** Fixed 512-sample mix blocks (not yet buffer-size-adaptive — flagged gap); **no `cargo xtask bench` subcommand exists yet** (explicitly noted as missing); entire audio sources decoded wholly into RAM (flagged memory-risk gap for hour-long sessions). **[verified]**

**Testing:** **No oracle/golden-audio comparison exists against real Pro Tools** — explicitly flagged as entirely unbuilt (`docs/gaps.md` G-07, "nothing is compared… no fidelity harness"). Property-based (`proptest`) fuzz-style tests for plugins/offline-DSP/sessions are extensive; no `fuzz/` dir. **[verified]**

**AI:** Self-rated lowest of all 8 repos — **"AI features ~5%."** No noise reduction/stem separation/auto-mix. No `craft-ai-integrations`. **[verified]**

**Reusable crates:** Only 2/13 craft-libs crates consumed (`craft-ui`, `craft-docpath`). Notably **duplicates ISO-BMFF demux logic already available in craft-libs's `craft-media`** rather than depending on it — a concrete missed-reuse case. **[verified]**

---

## 6. CADCraft (AutoCAD clone)

**Purpose:** 2D entities (Line/Arc/Spline/Hatch/Dimension/Insert/MLeader/etc.), layers/blocks, model/paper-space layouts+viewports (no UCS yet), AutoCAD-faithful command-line/point-entry/osnap UX, DXF+DWG read/write. **No 3D solids/surfaces/ACIS yet** (3D modeling only ~2%). **[verified]**

**Maturity:** 6,419★, created 2026‑10‑07, 617 commits, 78 contributors. Self-graded **"pre-alpha," 38% ready for real work**; "0 'switched from AutoCAD' user reports" quoted directly from its own parity doc. **[verified]**

**Tech stack:** egui 0.36/eframe 0.36/egui-wgpu 0.36, GPU canvas via an `egui_wgpu` paint callback drawing batched TriangleList/LineList/instanced-quad geometry; **CPU fallback via `tiny-skia`+`lyon_tessellation`**. Geometry kernel and constraint solver are **fully custom (no nalgebra/parry/kurbo)** — a hand-written damped Gauss-Newton/Levenberg-Marquardt solver. **DWG support is NOT clean-room** — it wraps the third-party `acadrust` crate (MPL-2.0, unmodified, from crates.io) as a DWG↔DXF byte-bridge, isolated behind a byte-level API ("nothing else in CADCraft depends on its types"); DXF itself is a genuine from-scratch parser. **No `indd-utils`-style standalone DWG utility exists** in the org — DWG handling lives inside the app, depends on someone else's crate. `crates/dwg/src/lib.rs:1-6`, `NOTICE:9-12` **[verified]**

**Architecture:** 12 library crates, L0–L5. **Spatial indexing via the external `rstar` R-tree crate** (the only one of the 8 repos using an off-the-shelf spatial index crate rather than hand-rolled structures), **incrementally patched per-chunk rather than rebuilt** on edit, with a `MIN_INDEXED=256` brute-force threshold for small drawings. 295 commands, AutoCAD's command-line heritage explicitly preserved (`crates/ui-egui/src/cmdline.rs`) alongside AutoLISP support. `crates/engine/src/spatial.rs:1-145` **[verified]**

**Perf — a real, candidly documented regression-and-fix story:** On a 12k-entity/~340k-primitive hatched DWG, **zoom/pan used to block the UI for 2–4.5 seconds per step** while the display list/vertex buffers rebuilt; as of a dated fix, rebuilds now run on a **worker thread while the previous list keeps drawing**, cutting the worst stall to 26–64ms — but **edits still rebuild on the UI thread (~1.8s)**, with "per-entity caching is the next step" explicitly called out as unfinished. Adaptive tessellation tolerance (~½px at current zoom) is the closest thing to LOD; parallel `rayon`-chunked display-list building. `docs/hardware-parity.md:36-43`, `crates/render/src/lib.rs:198-253,556-573` **[verified]** — this before/after story is one of the most concrete, directly-transferable data points for pbiview's own canvas-redraw strategy.

**Testing:** **No fuzzing infrastructure anywhere** — notable because DWG parsers are a historically major CVE/fuzzing target, and this gap is self-acknowledged ("geometry fuzzing… partial, 30-50h, unstarted"). All 36 DXF/DWG round-trip tests use **synthetic inputs only**; "no test opens a file made by another program, and no external validator runs in CI." **No real-file corpus repo exists** (unlike PhotoCraft's). Hand-rolled xorshift property-tester for geometry (not `proptest`/`quickcheck`, neither in `Cargo.lock`). **[verified]**

**AI:** ~20% — only the MCP server (agent read/write access, e.g. `query_entities selected:true`); no local LLM/Smart-Blocks/Markup-Assist. **No `craft-ai-integrations`.** **[verified]**

**UI pain points:** IME flagged as an unresolved multi-PR cross-platform issue (#110–#119, "6–10h remaining"). A custom-drawn canvas crosshair **breaks OS screen magnifiers/screen-recorders** since it hides the system cursor — worked around with a `View▸Accessibility▸Show System Cursor` toggle / env var. **[verified]**

---

## 7. FigCraft (Figma-like, collaborative design)

**Purpose:** Frames/sections/auto-layout, components/variants/instances-with-overrides, vector networks + boolean ops, prototyping (transitions/conditional actions/presenter), **full real-time multiplayer** (cursors/selections/avatars/follow/spotlight/cursor-chat), comments, branching/merge, WASM plugin sandbox, Dev Mode (React codegen). **[verified]**

**Maturity:** 3,125★, created 2026‑10‑09 (**youngest functioning repo**, ~13 days old at research), ~1,450 commits, 75 contributors, commit messages show "Co-authored-by: Claude Opus 5.5"; **docs state the app "was built in ~25 wall-clock hours by 4–6 parallel agents."** Self-graded "early alpha," menu-breadth 100% (212/212), feature breadth ~85%, "ready for real work" ~53%. **[verified]**

**Tech stack — the most interesting contrast with the real Figma:** egui/eframe/wgpu confirmed, **explicitly framed as the inverse of Figma's own stack**: *"Figma's desktop app is Electron with a GPU helper process… FigCraft renders the canvas on the CPU and presents it through wgpu."* `docs/hardware-parity.md:7` **[verified]**. **Rendering is CPU-side `tiny-skia`**, not GPU compositing (a named gap: "CPU canvas where Figma uses the GPU," 30-60h estimate for a future vello/wgpu-shader path).

**Canvas virtualization (directly relevant to report-canvas perf):** `crates/render/src/lod.rs` — a layer whose entire subtree paints into less than `TINY_PX=2.0` device px is **splatted** (treated as a single blob); text below `GREEK_PX` is **greeked** (rendered as gray bars, not real glyphs). `crates/render/src/retained.rs` — retained rendering **reuses the prior frame** and re-renders only panned strips/edited regions, snapped to whole device pixels, with `MAX_CHANGED=4096`/`MAX_PARTIAL_SHARE=0.6` fallback thresholds to full-render. Measured: 50k-layer page full-render ~8ms (LOD); 10k-layer single-layer move re-render ~0.2ms. **[verified]**

**Real-time collaboration — the standout architectural finding of the whole family:** `docs/collab-protocol.md` **explicitly rejects both OT and general CRDTs** in favor of a **server-authoritative model**: *"a design document is a tree of objects with properties, not a text buffer… general CRDTs carry metadata and tombstones we don't need when a server orders everything. With per-property last-writer-wins plus a few tree rules… every client converges to the server's state."* No yrs/automerge dependency anywhere. Binary **postcard** encoding (43B vs 139B JSON per edit). Tree structure via **two properties**: `parent` + fractional-index `position` (base-62 strings, e.g. `"a0V"`) so concurrent inserts never conflict. `figcraft-collab` (pure, no networking) vs `figcraft-collab-net` (the only crate permitted `tungstenite`/`rustls`). Per-user undo interacting with live collaboration is formally specified: "a step waits for what came before it," created-then-undone nodes become deletes, deleted-then-undone nodes are recreated whole. Validated by a **deterministic-seeded in-memory multi-client simulation** (`crates/collab/src/sim.rs`) running "thousands of seeds," checking every undo/redo against the accepted-batch log. **[verified — the single most architecturally novel subsystem found across all 8 repos]**

**Architecture:** L0–L6, `imbl` persistent-map document model (cheap `Arc`-structural-sharing snapshots). 455 commands (as of a dated snapshot; likely higher now). A **standalone `figcraft-server` app** for collaboration hosting. **[verified]**

**Testing:** No cargo-fuzz; proptest + a "junk-parameter sweep over every command"; **collaboration correctness via the `sim.rs` simulation** described above — a genuinely good pattern for testing concurrent-edit systems. No real-Figma-file corpus (explicitly admitted gap: "every corpus in the repo is synthetic… no analogous `figcraft-corpus` repo exists"). **[verified]**

**AI:** 0% — explicitly "owner decision needed." No `craft-ai-integrations`. **[verified]**

**UI pain points:** A concrete, named **upstream egui-wgpu limitation blocking true Display-P3 color**: "`eframe`/`egui-wgpu` 0.36 configure an 8-bit sRGB surface with no option for format/color-space… needs upstream (or a patched egui-wgpu)." **IME pain point**: "an idle frame turned the platform IME off and macOS dropped each composition (#747)" — had to keep IME on every frame of an edit, not just input frames. AccessKit only builds a tree for the **main window**, forcing a design workaround (files stay as tabs, not separate windows) to keep multi-doc accessible. **[verified]**

---

## 8. AniCraft (2D animation) — essentially non-existent

**Critical finding: this repo is completely empty** — `size: 0`, `language: null`, zero commits, zero branches/tags/releases, 404 on contents/README/Cargo.toml on both `main` and `master`, `git ls-remote` returns **no refs at all**. The sole content is one GitHub Issue (#1) in which the maintainer states the original AniCraft codebase was **"discarded"** and work is being redirected to a separate, non-org repo, `Flare-Animate/Flare` ("a re-implementation/recreation of Adobe Animate/Flash," forked from OpenToonz), with discussion of folding Flare into the ArtCraft suite instead. **AniCraft is absent from `craft-repo-status-app`'s tracked-apps list** (which explicitly enumerates photocraft/lightcraft/filmcraft/figcraft/etc.) — organizational confirmation it isn't an active member of the suite. 54 stars exist despite zero code (likely Discord-driven speculative interest). **No tech stack, architecture, performance, testing, or AI features can be reported — none exist.** **[verified, multiply cross-checked via contents/commits/branches/tags/ls-remote APIs]**

**Implication for your research:** treat AniCraft as **out of scope / not a comparable data point** — it contributes no architectural evidence. If 2D-animation-specific patterns are wanted, `Flare-Animate/Flare` (outside `storytold`) would be the repo to examine instead, though that's outside this org's read-only scope as framed.

---

## Cross-repo evidence: UI toolkit choice

**Universal finding: every functioning app in the family (7 of 8, excluding empty AniCraft) uses egui 0.36 + eframe 0.36, rendered through wgpu 30 (Metal/Vulkan/DX12/WebGPU, GL avoided or compiled out), with zero Tauri, zero system webview, zero Electron, zero iced/Slint/Dioxus/GTK anywhere in any dependency tree.** This is enforced architecturally, not just by convention: every repo has an `xtask/src/main.rs` (or `layers.rs`) that walks `cargo metadata` and **fails the build** if `egui`/`eframe`/`winit`/`rfd`/`cpal`/`muda` leak below a designated layer (L5 or L6 depending on repo). Every repo's `AGENTS.md` states some variant of "the UI crate is swappable" / "nothing below L5/L6 knows about egui." All 7 also ship a WASM build (`apps/<app>-web`) that is the **same** egui codebase cross-compiled to `wasm32-unknown-unknown`, confirming this isn't a Tauri-hybrid anywhere — it's genuinely one UI stack, two targets.

**The org maintains real, hand-patched forks of both `egui` and `winit`** (`storytold/egui`, `storytold/winit`) — this is the clearest "we hit real toolkit limitations" signal in the whole family:

- **`storytold/egui`**'s `main` branch is an exact mirror of upstream (`emilk/egui`, 0 ahead/0 behind) — apps consume egui from crates.io, not this fork, in normal operation. But a separate **`craft/0.36.2` branch carries 4 real patches** (`PATCHES.md`), applied by every app that needs them, reconciled to "one commit per patch, dropped when upstream lands it":
  - **E1** (`eframe`): `NativeOptions::viewport_hooks` — exposes each viewport's native window + raw input pre-egui, "needed by craft-tablet to route native Wayland pen events" (not proposed upstream).
  - **E2+E3** (`egui-wgpu`): fits oversized surfaces within `max_texture_dimension_2d` (fixes a **panic** on HiDPI/multi-monitor-restored windows — PdfCraft#577/GridCraft#760) and wraps surface-configure/renderer-creation in wgpu error scopes so the app can **fall back to another renderer** instead of aborting.
  - **E4** (`egui-winit`): uses winit's own Wayland clipboard instead of starting `smithay-clipboard`, because Hyprland only serves the **first** `wl_data_device` per client — a hand-ported draft of an unmerged upstream egui PR.

- **`storytold/winit`**'s `master` is a plain, slightly-stale mirror (0 ahead / 20 behind upstream) — but a **`craft/0.30.13` branch carries 17 patches** (`PATCHES.md`), the single richest documented source of "real egui/winit pain points" in the whole research:
  - **6 of the 17 patches are IME fixes** spanning every platform: Wayland (seat text-input destroyed only on seat removal), Windows (Korean IME caret position from `GCS_CURSORPOS`), X11 (focus the replacement IME context), macOS (×3: NSTextInputClient commit flow, discard stale marked text when IME disabled, forward emoji-picker text as `Ime::Commit`, suppress duplicate Korean-IME space keystrokes, retry first Apple-Korean keypress). Several are cited as "Originally … in `wordcraft#787`" — i.e. **WordCraft (a Word clone, out of your scope but part of the family) did the initial IME legwork**, later generalized into this shared fork for all apps.
  - **Wayland drag-and-drop and clipboard were reimplemented from scratch** inside the fork (winit itself didn't support Wayland DnD until 0.31; the family needed it on 0.30) — this is the root cause independently documented as a pain point/workaround in FilmCraft, EffectCraft, and VectorCraft's own docs.
  - **A Wayland scroll-phase fix (W13)**: "wheel, wheel-tilt and continuous-source scroll events report `Moved` instead of starting a finger gesture that never ends; **egui then follows modifier release instead of keeping Ctrl-zoom latched**" — i.e., without this patch, Ctrl+scroll-zoom in egui apps could get stuck active on Wayland.
  - **A macOS window-drag-vs-title-bar fix (W12)**: AppKit's default `mouseDownCanMoveWindow` answer let any drag starting in a transparent title-bar band move the window before the app saw it, **breaking tab-reordering drag gestures** in PdfCraft.
  - A DPI-transition fix (Windows 11 `WM_DPICHANGED`), and an opt-in flag to **intercept native macOS app termination** and route it through the app's own close-request queue (DesignCraft needed this for unsaved-document prompts).

**Documented egui pain points, aggregated across all 7 live repos:**
| Pain point | Repos documenting it | Workaround |
|---|---|---|
| IME composition/caret bugs (all platforms) | LightCraft (custom text-field, "70%, IME edge cases"), FigCraft (#747 idle-frame IME drop), CADCraft (#110-119), WordCraft (origin of the winit IME patch series) | Patched `winit` fork (6 IME patches); custom text widgets |
| Wayland DnD/tablet/clipboard missing in winit 0.30 | VectorCraft, FilmCraft, EffectCraft (explicit README workaround: run via XWayland or `env -u WAYLAND_DISPLAY`) | Patched `winit` fork (reimplemented from scratch) |
| GPU adapter/driver crashes (AMD GL, Intel Vulkan, DX12 FXC/DXIL mismatch) | SoundCraft (`#185`), LightCraft (`#136`, `#471`, found a real FXC pixel-corruption bug), FilmCraft (2-crash→software-adapter fallback), VectorCraft (adapter-skip-and-retry loop) | Force single backend per OS (no GL), `WGPU_BACKEND` override, crash-sentinel files, patched `egui-wgpu` error-scope fork (E2/E3) |
| No off-the-shelf docking | All 7 (no `egui_dock` usage anywhere) — each built bespoke docking, now **converging onto `craft-ui::docking`** (shared transactional docking, in-flight migration visible in VectorCraft's ROADMAP and EffectCraft's progress log) | Shared `craft-ui` docking crate |
| Accessibility (AccessKit) covers only standard widgets / main window | VectorCraft ("canvas/panels aren't described, nothing tested with VoiceOver"), FigCraft (AccessKit builds a tree only for the main window — forced "files as tabs not windows" design), EffectCraft (self-rated 20%) | Manual per-widget AccessKit labeling; architectural workarounds |
| No true wide-gamut (Display P3) surface support | FigCraft (explicit named upstream egui-wgpu gap) | None yet — flagged as needing a patched egui-wgpu |
| RTL / complex-script text layout | FilmCraft (Arabic titles broken, #395), SoundCraft (Arabic/Hindi "no RTL layout, no shaping"), VectorCraft (bidi handled via `unicode-bidi` crate bolted on top) | `unicode-bidi` crate; egui itself has no native bidi |
| egui panics on unbound font family | VectorCraft | Font-registration guard tested in `cargo xtask ci` |

**No UI-toolkit migration was found anywhere** (no Tauri→egui or similar switch in any repo's history/CHANGELOG/revision tables) — egui/eframe/wgpu was the choice from each repo's inception. The "migration" evidence that *does* exist is lateral: several apps are mid-migration from **bespoke, per-app docking code onto the shared `craft-ui::docking` crate** (VectorCraft ROADMAP UI.4/UI.5, EffectCraft ROADMAP UI.3 progress-log entry citing a fixed float-clamp panic as the motivating bug).

---

## Shared infrastructure (`craft-libs`, `craft-ai-integrations`, `craft-fonts`, `craft-i18n`)

- **`craft-libs`** (328★, Cargo monorepo workspace): `craft-platform, craft-ai-core, craft-pdf, craft-atomic, craft-cms (ICC color mgmt), craft-docpath, craft-i18n, craft-media, craft-print, craft-quit, craft-settings, craft-ui, craft-window`. Consumed via **git-rev-pinned** deps (not path deps, not crates.io) — each app pins an exact commit SHA. **Adoption is extremely uneven**: every app takes `craft-ui`+`craft-docpath`; almost none take `craft-ai-core`, `craft-cms`, `craft-atomic`, `craft-settings`, or `craft-i18n` despite them existing — apps largely reimplement equivalent functionality locally (e.g. LightCraft's own color/print code parallels `craft-cms`/`craft-print` without using them; SoundCraft duplicates `craft-media`'s ISOBMFF demux). README states explicit policy: *"If any libraries grow beyond Craft App usage… they may be promoted out."* **[verified]**
- **`craft-ai-integrations`** (89★): a much larger, more mature shared AI runtime than any single app's local AI code — 16+ crates (`craft-ai-core/-native/-client/-features/-http/-media`, provider adapters for OpenAI/BFL/Vertex/Seedance/ArtCraft/ComfyUI/SAM3, a declarative no-code adapter format, local engines on candle/rten). Its own README states it "cannot switch to a pinned upstream [craft-libs] revision yet" — i.e. **even this repo hasn't fully converged with `craft-libs`'s `ai-core`**. **Zero of the 7 live app repos depend on it** — every app's AI features (Roto Brush, SAM3 masking, Whisper/Kokoro, face tracking) are self-contained, not routed through this shared layer. This is the single clearest "shared infra exists but isn't adopted yet" finding. **[verified]**
- **`craft-fonts`** / **`craft-i18n`**: both deliberately **optional, env-var-gated build inputs** (`CRAFT_FONTS_DIR`, `CRAFT_I18N_DIR`) rather than Cargo dependencies — "the apps don't list [these] in Cargo.toml (Cargo would fetch it for every build)… release builds check out a pinned commit and set the option." A clean pattern for keeping large/optional assets out of the main build graph while still sharing them at release time. **[verified]**
- **`craft-common-actions`** (Python, 206 forks across the family): shared reusable GitHub Actions workflows (`nix.yml`, `sonarcloud.yml`, `packaging-lint.yml` in every app delegate to it via `uses:`) — **none of the shared workflows run the actual test suite on PRs**, consistent with every single repo's independently-documented gap of "no CI gate runs `cargo xtask ci`/tests on pull requests."
- **`craftrules`** (referenced by nearly every AGENTS.md as the source of cross-app standards like `standards/never-crash.md`, `standards/test-corpora.md`) returns **404** — likely private. Its content could not be verified directly; only inferred from citations.

---

## Comparison table

| Repo | Target app | Stars | Created | Commits | Contributors | Self-rated "ready for real work" | UI toolkit | Render pipeline | Canvas/scrub virtualization | Spatial index | Fuzzing | Corpus repo | AI features | `craft-ai-integrations`? | MCP/automation |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **FilmCraft** | Premiere Pro | 15,702 | 09-30 | 2,109 | 69 | 56% | egui/eframe/wgpu | GPU (WGSL) + CPU oracle | Prioritized frame-worker pool, GOP cache, draft-decode, global 1GiB budget | — (list virt. not done) | tests-as-fuzz (no cargo-fuzz) | ✗ (gap, cites PhotoCraft) | Whisper+Kokoro (local) | ✗ | MCP (rmcp) + control-channel |
| **EffectCraft** | After Effects | 7,680 | 10-01 | ~2,000s | 97 | 45% | egui/eframe/wgpu | GPU compute (WGSL), **Auto CPU/GPU picker every 48 frames** | Layer-cache(hash-keyed) + RAM-preview-newest-completed + LZ4 disk cache | — | in-test fuzz only | ✗ (nascent, 2 cases only) | Roto Brush (MobileSAM)+Face (MediaPipe) local | ✗ | MCP + control-channel |
| **LightCraft** | Lightroom | 14,293 | 09-30 | ~3,476 | 181 | 61% | egui/eframe/wgpu+candle | GPU (WGSL) vs CPU oracle, **GL compiled out entirely** | **Virtualized photo grid** (visible-cell-only), content-hash disk LRU, global-tick LRU | — | proptest-based | ✗ (uses raw.pixls.us CC0 pins) | SAM3 masking, AI denoise, face ID, all local/optional | ✗ | MCP + control-channel |
| **VectorCraft** | Illustrator | 10,408 | 09-30 | 4,911 | ~30 (62% one person) | 62% | egui/eframe + `vello_cpu` (CPU!) | **CPU** SIMD sparse-strip, GPU composites only | 9 named per-category FrameCache budgets | ✗ none found | **cargo-fuzz** (Affinity parser) | ✓ pinned Affinity corpus + CI workflow | None (0%, explicit "owner decision") | ✗ | MCP (2 backends) |
| **SoundCraft** | Pro Tools | 3,562 | 10-07 | — | 43 | 40% (pre-alpha) | egui/eframe/wgpu+glow | GPU mesh waveforms | **O(log n) peak mipmap** (in-memory only, no disk cache) | — | proptest-based | ✗ | ~5% (none shipped) | ✗ | MCP + control-channel |
| **CADCraft** | AutoCAD | 6,419 | 10-07 | 617 | 78 | 38% (pre-alpha) | egui/eframe/egui-wgpu | GPU batched (Triangle/Line/instanced quads) + tiny-skia/lyon fallback | Documented 2-4.5s→26-64ms zoom-stall fix via worker-thread rebuild | **`rstar` R-tree**, incrementally patched | **none at all** (notable gap given DWG) | ✗ | ~20% (MCP only, no local models) | ✗ | MCP + AutoLISP + control-channel |
| **FigCraft** | Figma | 3,125 | 10-09 | ~1,450 | 75 | 53% (early alpha) | egui/eframe/wgpu | **CPU** tiny-skia, GPU presents only | **LOD splat/greek + retained/dirty-region re-render** | imbl persistent-map diffing | in-test fuzz + collab sim | ✗ (synthetic only) | 0% (explicit "owner decision") | ✗ | MCP + **real-time collab server** |
| **AniCraft** | 2D animation | 54 | 10-14 | **0** | 0 | N/A — **empty repo, abandoned** | N/A | N/A | N/A | N/A | N/A | N/A | N/A | N/A | N/A |

**Universal across all 7 live repos:** egui 0.36 + eframe 0.36 + wgpu 30, no Tauri/webview/Electron anywhere; strict `cargo xtask layers`-enforced crate layering; one `CommandSpec` registry shared verbatim by UI/CLI/JSON-control-channel/MCP; `AGENTS.md` "never crash" rule (`unsafe_code="forbid"` except narrowly-scoped FFI/plugin-host crates); clean-room-only implementation policy; no PR-gating test CI in any of the 7 (self-acknowledged gap in every single repo's own docs); no top-level `CHANGELOG.md` (progress tracked in `ROADMAP.md`/doc revision-history tables instead); heavy AI-agent commit authorship (top contributor is literally named `claude` in 4 of 7 repos).

---

## Prioritized recommendations for pbiview / DashCraft

Given pbiview is **Tauri 2 + Svelte 5** (not egui) and a **report canvas** (closer to FigCraft/VectorCraft's "many small objects, pan/zoom, edit-and-redraw" problem than to FilmCraft/SoundCraft's "scrub a timeline" problem), prioritize by closest architectural analogy:

**1. Adopt retained + LOD rendering for the report canvas (from FigCraft) — highest priority.** FigCraft's `retained.rs`/`lod.rs` pattern — reuse the prior frame and re-render only changed/visible regions, **splat** any object below a pixel-size threshold instead of re-tessellating it, **grey out** (skip real shaping for) tiny text — is the single most directly transferable technique for a dashboard with many visuals at varying zoom. Set an explicit `MAX_CHANGED`/`MAX_PARTIAL_SHARE`-style fallback-to-full-render threshold so the optimization never causes correctness bugs, exactly as documented there.

**2. Build a per-object/per-effect cache keyed by content hash, not by revision counter (from EffectCraft + CADCraft).** EffectCraft's layer cache ("keyed by a hash of its evaluated inputs, excluding the transform… static/transform-only layers render once") and CADCraft's R-tree cache ("reused only while the live store still shares every chunk with it") both avoid the classic bug of "stale cache because the revision number ticked but content didn't." For pbiview's report canvas, cache each visual's rendered output keyed by (visual spec hash, data hash, viewport scale), not by a global document revision — this lets panning/scrolling reuse everything unaffected.

**3. Show the newest-completed render, never regress (from EffectCraft's RAM-preview).** "During rapid edits at the same time and view, the viewer shows the newest completed revision while the exact requested revision renders; out-of-order completions cannot move it backwards." This single rule eliminates flicker/stutter during interactive slider-drag/filter-change scenarios — directly applicable to DAX/M recompute-and-redraw latency.

**4. Adopt an explicit, numeric interaction-latency budget enforced by a benchmark harness, early (from LightCraft + VectorCraft).** LightCraft's AGENTS.md states concrete numbers ("slider update ≤16ms draft / ≤60ms loupe; export ≤1s/24MP") and `cargo xtask bench` fails CI on >20% regression. VectorCraft's `vectorcraft-cli perf` runs a **synthetic worst-case document** (50k paths) with per-interaction budgets (pan/zoom/hit-test/drag, each with its own ms target) — but note both of these are **self-admitted as not CI-gated**; do better than the family here by actually wiring the perf harness into required CI from day one, since every single one of the 7 repos independently confirms this gap.

**5. Use a mipmap/multi-resolution structure for any time-series or large-array preview (from SoundCraft's `Peaks`).** The `BLOCK=32, FANOUT=4` min/max/RMS mipmap giving "O(log n) exact range queries regardless of zoom" is a clean, generalizable pattern beyond audio — e.g., for sparkline/preview rendering over large DataFusion result sets before the full visual renders.

**6. Background-thread the expensive rebuild step, and do it before you need per-object caching (from CADCraft's documented before/after fix).** CADCraft's own numbers (blocking 2-4.5s → 26-64ms just by moving display-list rebuild off the UI thread, while the *previous* frame keeps drawing) is a lower-effort, higher-leverage first step than full per-object caching, and should probably land in pbiview before more elaborate caching schemes.

**7. For real-time collaborative/multi-request-id scenarios (if pbiview ever adds collab editing or multi-client control), study FigCraft's server-authoritative model before reaching for a CRDT.** Their explicit rationale — "a design document is a tree of objects with properties, not a text buffer; general CRDTs carry metadata/tombstones we don't need when a server orders everything" — plus fractional-indexing for ordering and a seeded-simulation test harness (`sim.rs`) for correctness, is a lighter-weight alternative worth evaluating if pbiview's command/undo model is itself server/host-authoritative (it likely is, given DAX/M execution is centralized).

**8. Architecture to copy almost verbatim: one command registry, four surfaces.** Every repo in the family (all 7) shares a single `CommandSpec{id, label, menu, shortcut, params, enabled, run}`-style registry dispatched identically from UI, CLI, a JSON control-channel, and an MCP server. This maps directly onto pbiview's stated "CLI with schemaVersion'd JSON" design goal — the family's `docs/control-protocol.md`/`docs/mcp.md` pattern (one JSON-RPC-ish command surface, versioned, with an MCP adapter on top) is a proven, replicable blueprint, and these are genuinely good reference docs to read in full if/when pbiview adds MCP support.

**9. Enforce layering with a custom `xtask` from day one, not retrofitted.** Every repo's `cargo xtask layers` (reading `cargo metadata`, failing on upward deps or UI-crate leakage below a line) is cheap to build and was clearly worth it to this whole family — for pbiview (Rust engine crates + Tauri/Svelte UI), an equivalent check that DataFusion/DAX/M engine crates never pull in Tauri/Svelte-adjacent crates (and that the renderer crate stays swappable between native-preview and CLI-only usage) is directly analogous and cheap insurance.

**10. Testing: prioritize an oracle-based golden/fidelity harness over fuzzing, but don't skip fuzzing on file-format parsers.** The family's testing quality correlates strongly with investment, not framework choice: FilmCraft/EffectCraft's ffmpeg-oracle + golden-PSNR approach and LightCraft's raw.pixls.us ΔE-against-camera-JPEG corpus are the strongest fidelity stories in the family and are a good template for validating pbiview's DAX/M engines against real Power BI Desktop output (where legally/practically obtainable) or against a documented spec oracle. Conversely, **CADCraft's complete absence of fuzzing despite parsing a notoriously hostile format (DWG, via a third-party crate) is a cautionary tale** — if pbiview parses `.pbix`/PBIR or other binary/zip-based formats, a `cargo-fuzz` harness (as VectorCraft did for its Affinity parser) should be considered non-negotiable, not optional, from the start.

**11. Reusable-crate lesson: shared infra existing is not the same as shared infra being adopted — budget for the gap.** Across the family, `craft-libs`/`craft-ai-integrations` exist and are well-documented but are **inconsistently consumed** (SoundCraft duplicates `craft-media` rather than using it; 6 of 7 apps ignore `craft-ai-core`/`craft-cms`/`craft-settings`). If pbiview plans to factor out reusable crates (Arrow/DataFusion helpers, scene-graph primitives, SVG/PNG export) for other potential future tools, plan explicit adoption/migration work as a tracked backlog item — it does not happen automatically even within a tightly-coordinated, AI-agent-heavy org.

**Gaps/uncertainties in this research:** `storytold/craftrules` (the presumed source of org-wide written standards) is inaccessible (404) — its content is only known via citation from other repos' AGENTS.md files, so some "shared standard" claims are **[inferred]** rather than directly verified. `craft-tablet` (referenced by the egui-fork E1 patch as the Wayland-pen-input consumer) does not appear as a public repo in the org listing — likely private, unpublished, or embedded inside one of the apps; not independently located. Performance numbers throughout (FilmCraft's GPU speedups, LightCraft's slider-latency, VectorCraft's/CADCraft's perf-budget results, FigCraft's LOD timings) are uniformly **self-reported by each project on its own unspecified hardware**, not independently reproduced here — treat magnitudes as directional, not as cross-repo-comparable benchmarks. Subagent research occasionally hit GitHub's search-code rate limit (resolved by falling back to direct `contents`/raw-file fetches and full recursive tree listings) — a few planned exhaustive keyword sweeps (e.g., "Tauri"/"iced"/"slint" across every doc file in every repo) were done via targeted doc grep rather than full-repo code search in 2-3 of the sub-reports; I consider this low-risk for the negative-evidence claims (absence of Tauri/webview) given the consistent, independently-corroborated pattern across all 7 repos' `Cargo.lock` files, which were checked directly.