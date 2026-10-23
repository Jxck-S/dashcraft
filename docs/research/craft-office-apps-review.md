<!-- Research by a Copilot research agent, 2026-10-22, from public storytold/* repos (read-only). -->
# Storytold "Craft Apps" Research Report — gridcraft, pdfcraft, wordcraft, deckcraft, designcraft

## 0. Org repo confirmation

Ran `gh api orgs/storytold/repos --paginate`. All five requested repos exist under the **exact names** given — no renames needed:

| Repo | Description | ★ | Forks | Open issues | Created | Size (KB) |
|---|---|---|---|---|---|---|
| `gridcraft` | "clean-room spreadsheet (Microsoft Excel-style)" | 6,230 | 2,906 | 1,055 | 2026‑10‑07 | 10,594 |
| `pdfcraft` | "open-source PDF reader and editor" | 17,169 | 6,600 | 1,750 | 2026‑09‑30 | 51,850 |
| `wordcraft` | "clean-room reimplementation of Microsoft Word" | 15,498 | 4,195 | 1,542 | 2026‑10‑07 | 28,185 |
| `deckcraft` | "reimplementation of Microsoft PowerPoint" | 4,116 | 2,348 | 526 | 2026‑10‑07 | 8,016 |
| `designcraft` | "page and layout design editor" (InDesign-class) | 4,656 | 3,226 | 578 | 2026‑10‑01 | 24,707 |

No `*-corpus` sibling repos exist for any of these five (only `photocraft-corpus` exists org-wide) [verified]. All five share the same skeleton as photocraft: `apps/`, `crates/`, `docs/`, `xtask/`, `contributors/`, `packaging/`, `AGENTS.md`, `CLAUDE.md`, `ATTRIBUTION.md`, `ROADMAP.md`, `ReAdME.md`, workspace `Cargo.toml` with `cargo xtask layers`/`ci`.

All findings below are tagged **[verified]** (directly read from source/docs) or **[inferred]** (reasoned from partial/indirect evidence), per-repo, with `org/repo:path:line` citations as fetched by five parallel deep-dive research agents against each repo's `main` HEAD on 2026‑10‑22 (exact pinned commit SHAs noted per repo below).

---

## 1. GridCraft (storytold/gridcraft) — commit `a1c84547`

**Purpose/maturity** [verified]: Excel clone, alpha stage, "beta ~160–240 agent-hours away," full parity ~700–1,100h. `storytold/gridcraft:ROADMAP.md:1-24`. Self-reported: ribbon catalog 274/290 commands (94.5%); **functions 513/523 by name (98.1%), 504/523 "working" (96.4%)**. Per-dimension: Formula language ~92%/72% ready, Pivot ~65%/45%, Charts ~50%/30%, **Get & Transform/data model ~15%/10%** (largest gap by hours: 60–100h), AI ~15%. `storytold/gridcraft:ROADMAP.md:19-82`, `docs/gaps.md`.

**Crate architecture** [verified]: 12 crates, ~70k LOC: `core, numfmt, pdf` (L0) · `formula, functions` (L1) · `model, calc, xlsx` (L2) · `chart` (L3) · `engine` (L5, ~17.7k LOC, 313 commands) · `ui-egui, mcp` (L6). `storytold/gridcraft:docs/architecture.md:9-23`. Layering enforcement is **more rule-based than photocraft's**: `xtask/src/layers.rs` adds an intra-layer DAG (`[core,numfmt]`, `[model,calc]`, `[model,xlsx]`), a format-crate allow-list restricting `xlsx`'s deps, and a `UI_CRATES` detector flagging any sub-L6 crate pulling egui/wgpu/winit — fully unit-tested. `storytold/gridcraft:xtask/src/layers.rs:1-260`.

### Formula/calculation engine (the core ask)
- **Parser**: hand-written, no pest/nom/chumsky. Lexer is a manual scanner; parser is **Pratt/precedence-climbing** with `MAX_DEPTH=256`. `storytold/gridcraft:crates/formula/src/lexer.rs`, `parser.rs:1-12`.
- **AST**: `Expr::{Number, Text, Bool, Error, Array, Ref, Name, Struct, Unary, Binary, Call, Invoke, Missing, Paren}`; references model Cell/Range/Rows/Cols/3-D spans; structured table refs are first-class (`StructRef{table, specifiers, col_start/end}`). `storytold/gridcraft:crates/formula/src/ast.rs:1-100`.
- **Stack-safety engineering** (notable): an 8,192-char flat `=A1+A2+…` chain nests ~4,000 left-recursive AST levels; `Clone`/`PartialEq`/`walk`/`map` are hand-written as **iterative left-spine loops**, not naive recursion, specifically to avoid stack overflow — regression-tested with a 1 MiB-stack validation. `storytold/gridcraft:crates/formula/src/ast.rs:170-260`.
- **Dependency graph & recalc** (`crates/calc/src/recalc.rs`): precedents mapped per-cell (hash map) and per-range (an **implicit segment/interval tree** over row intervals, O(log n+matches), with batched pending-insert buffering). A reverse index answers "who reads this cell?" Dirty set = transitive closure of dependents, evaluated in **topological order**, with **on-demand evaluation as a safety net** for dynamic refs. Custom `FxHasher` replaces SipHash everywhere for perf. `storytold/gridcraft:crates/calc/src/recalc.rs:1-80`.
- **Circular refs/iterative calc**: full `Enable Iterative Calculation`/Max Iterations/Max Change model, `MAX_ITERATION_WORK=10_000_000` global cap, `#CIRC!` when off. `storytold/gridcraft:crates/calc/src/recalc.rs:370-460,669-700`.
- **Dynamic arrays/spilling**: explicit spill-blocking rules (table-anchored, off-sheet, merged-cell collision) + spill-stability checks to avoid needless redraws; FILTER/SORT/UNIQUE/XLOOKUP/VSTACK/TAKE/GROUPBY/PIVOTBY/LAMBDA-era functions (MAP/REDUCE/SCAN/BYROW/BYCOL/MAKEARRAY/LET) all implemented. `storytold/gridcraft:crates/calc/src/recalc.rs:469-502`.
- **Function registry** (`crates/functions/src/lib.rs`): declarative `f!(...)` macro → `FnSpec{name, min/max_args, category, volatile, scalar_args, imp}`, lazily registered in a `OnceLock`; **automatic array-lifting is centralized** (not duplicated per function) via `scalar_args` + `lift::call_lifted`. 11 function-category modules; self-test asserts `all().len() >= 400`.
- **Fidelity testing vs Excel — the key finding**: **gridcraft explicitly has no Excel-computed oracle/conformance corpus yet** — "no conformance suite against Excel-computed results, and user reports keep finding edge cases" is a self-acknowledged gap (16–26h estimated). The one exception: a **352-cell TEXT()/number-format oracle dataset** generated from real Excel exists and passes 100%. `storytold/gridcraft:docs/function-parity.md`, `docs/gaps.md` §15.
- **Single-threaded recalc is an acknowledged architecture gap**: "100k-dependent edit ~0.4s vs 100ms target," "~100MB workbooks don't open" (whole-workbook-in-memory XLSX reader, no streaming). `storytold/gridcraft:docs/architecture.md:52-53`, `docs/hardware-parity.md:9`.

### Data model & performance (scroll/cache relevance)
- **Cell storage**: `CellStore` = **copy-on-write, banded sparse** structure — cells live in **64-row bands**, each behind an `Arc<Band>` (`Band{rows: BTreeMap<u32, BTreeMap<u32, Cell>>}`). Cloning for undo copies only band pointers; editing copies just the touched band. Not a flat HashMap, not columnar/Arrow. `storytold/gridcraft:crates/model/src/store.rs:1-30`.
- **Row/col sizing**: `LineIndex` (prefix sums over sparse height/width overrides, O(log n) lookup), built once per frame. `storytold/gridcraft:crates/ui-egui/src/grid.rs:195-230`.
- **Text-shaping/layout caching — important delta for pbiview**: GridCraft does **not** build a bespoke glyph/shaping cache; it calls egui's `Painter::layout_job` and paints the returned `galley`, **leaning entirely on egui/epaint's built-in text-layout cache**. [inferred from absence of any `GalleyCache`/`ShapeCache` struct]. Instead, its own caching effort targets **whole computed results keyed by `(doc_uid, revision, sheet[, extra])` tuples** (page-break ranges, PivotTable fields pane, Page Layout pagination), invalidated only on a `DocState.revision` bump — this is the closest conceptual analog to pbiview's text-shaping+layout cache fix, but applied at a coarser "recompute this whole pane" granularity rather than per-glyph. `storytold/gridcraft:crates/ui-egui/src/grid.rs:142-149`.
- Scroll uses egui's native `ScrollArea` (not a fully custom virtualization widget) clipped to `clip_rect()`; a `CfCache` avoids recomputing conditional-format matches per cell within one paint pass. `storytold/gridcraft:crates/ui-egui/src/grid.rs:1-5,856-865`.
- **Undo**: command-pattern + COW snapshot hybrid — `HistoryEntry{wb: Arc<Workbook>, …}`, bounded by **both** `UNDO_LIMIT=100` steps and `UNDO_BYTES_LIMIT=128MiB` (computed via `Arc::ptr_eq` marginal-cost tracking, not naive per-entry size). `storytold/gridcraft:crates/engine/src/lib.rs:65-150`.

### Charts
Dedicated **`crates/chart` (L3, "toolkit-free")**: `resolve`→`render`→`Prim` (flat vector of Rect/Line/Polygon/Wedge/Circle/Text) IR, consumed by both the egui on-screen painter and a CPU `rasterize` path for PNG export — **its own design, not Excel-matched pixel-for-pixel**, and **not shared with any cross-product chart crate**. Coverage: 25/~60 subtypes (~42% raw, ~65% usage-weighted); round-trip fidelity weak (axis scaling/error-bars/chartex-extensions lost on save of opened-elsewhere charts), ~30% ready. `storytold/gridcraft:crates/chart/src/lib.rs:1-135`, `docs/chart-parity.md`.

### Pivot tables / Power Query / data connections
**PivotTables are fully in-house** (`crates/engine/src/pivot.rs` 64KB + `pivot_calc.rs` + `cmd/pivot.rs` 110KB — largest command file + `crates/xlsx/src/pivot.rs` 59KB): field list, all 3 layouts, subtotals, calculated fields, PivotCharts — ~65%/45% ready. **Power Query / Get & Transform / M-formula engine / data model / external connections are explicitly and entirely absent** — zero crate/module/dependency found; "data connections" exist only as **byte-for-byte passthrough preservation** (`crates/xlsx/src/passthrough.rs`, 58KB) so they survive round-trip without being usable. `storytold/gridcraft:docs/gaps.md` §12.

### XLSX fidelity
**Hand-rolled OOXML** (quick-xml + zip only; zero `calamine`/`rust_xlsxwriter`/`umya-spreadsheet` deps). 26 source files covering styles/theme/tables/charts/pivot/drawing/crypto(MS-OFFCRYPTO AES/SHA, pure-Rust WASM-safe)/CFB(legacy)/ODS(read)/XLSB(read)/XLS BIFF8(read-only). **Round-trip tests cover only GridCraft's own output** — no real-Excel-authored oracle corpus wired into CI yet, despite the project's own `AGENTS.md` policy that such corpora should live in a pinned/sha256-verified external repo the way photocraft does it. `storytold/gridcraft:AGENTS.md:19`, `docs/gaps.md` §1,§7.

### Command/automation/AI
313 commands (smaller than photocraft's ~700, reflecting Excel's narrower surface). **MCP is hand-rolled JSON-RPC 2.0 over stdio — no `rmcp` dependency**, unlike what the task brief says photocraft uses; **no `cap-std` sandbox either**. 18 MCP tools, embeddable `Backend` trait. **AI**: zero `craft-ai-integrations` usage — "agent control strong; no in-app assistant" is the project's own self-assessment. `storytold/gridcraft:docs/mcp.md`, `docs/gaps.md` §22.

### Notable
No `fuzz/` dir — fuzzing is embedded as a 45KB engine-level property test (`command_sequence_fuzz.rs`, env-seedable) instead of cargo-fuzz. A full **What-If Analysis suite including a from-scratch Solver** (Simplex LP + branch-and-bound + augmented-Lagrangian nonlinear + evolutionary — explicitly *not* GRG2, an honest admission) is surprisingly deep for alpha stage. Zero `arrow`/`datafusion` anywhere — confirms GridCraft's storage is architecturally unrelated to DashCraft's planned Arrow+DataFusion columnar approach.

---

## 2. PdfCraft (storytold/pdfcraft) — commit `d5047aa4`

**Purpose/maturity** [verified]: Acrobat-class reader/editor, alpha, "~400–700h to beta," feature breadth ≈67% weighted, P0 (must-have) 94.2%, "ready for real work" ≈50%, ~780–1,400h to full parity. 152k LOC, 1,765 tests, 184 commands, 141 automation tools. `storytold/pdfcraft:docs/target-app-parity.md:17-29`, `docs/architecture.md:11-13`.

**Crate architecture** [verified]: **31 library crates** (largest footprint of the five), 3 apps. Layering: L0 `geom/filters/crypt` → L1 `cos` (COS object model) → L2 `content/model/fonts` → L3 `render/annot/forms/js/xfa` → L4 one-crate-per-tool-family (`edit/organize/redact/xfdf/sign/ocr/create/export/optimize/preflight/a11y/compare/measure/print`) → L6 `engine` → L7 `platform/ui-egui/automation`. Two **test-support-only crates** outside the product layering: `pdfcraft-oracle` and `pdfcraft-testkit`. `storytold/pdfcraft:docs/architecture.md:19-32`.

### The single biggest delta from the other four repos: rendering is NOT clean-room
- `parity/acrobat-features.toml` (487KB, 831 tracked features) is a **machine-validated** checklist — `cargo xtask parity` rejects any `shipped` claim lacking cited commands/tools/test evidence. `storytold/pdfcraft:parity/acrobat-features.toml:1-11`.
- `vendor/` holds **8 patched upstream crate forks** (`hayro`, `hayro-interpret`, `hayro-syntax`, `hayro-jbig2`, `hayro-cmap`, `lopdf`, `vello_common`, `winit`, `egui-wgpu`), with a **99KB prose changelog of ~34+ individually bug-tracked local patches** (fuzz-fix bounding of mesh-shading triangle/patch counts that previously allocated 6GB/34GB; CID-width-range clamps; Type-3 glyph bbox transforms; CJK stand-in-font glyph-mapping fixes; WinAnsi/MacRoman alias fixes verified against the pdf.js corpus). `storytold/pdfcraft:vendor/README.md`.
- **Self-acknowledged #1-ranked gap**: *"Rendering is borrowed and its fidelity unmeasured"* — all 7 rendering P0 features are `partial`; the project's own `model` crate (the eventual from-scratch renderer, ADR-0004) only does page-tree/view-matrix math so far, no glyph/path rendering yet. `storytold/pdfcraft:docs/gaps.md:20-33`.
- **Own work vs borrowed, clearly separated**: COS object model (`pdfcraft-cos`), content-stream parser/serializer (`pdfcraft-content`), encryption (R2–R6, own RustCrypto-based impl), incremental/full writers, annotations/forms/XFA/redaction/signing/OCR are all **own, hand-rolled code** — only the **rasterization pipeline and image-codec decode (DCT/JPX/JBIG2/CCITT)** are borrowed via hayro/lopdf.

### Testing & fidelity — the strongest/most rigorous of the five repos
- **Oracle policy with a teeth-having anti-silent-pass rule**: binds external **qpdf** (structural `--check` + sole independent encryption check) and **Poppler's pdftoppm/pdftotext** (pixel/text comparison). *"A missing oracle is either a declared skip or a failure, never a silent pass"* — motivated by a real incident where qpdf's absence from a CI runner had silently no-op'd an encryption test for its entire history. `PDFCRAFT_ORACLES_REQUIRED` env var escalates missing-tool skips to hard failures. `storytold/pdfcraft:crates/oracle/README.md:1-50`.
- **Pixel comparison methodology is deliberately not strict-diff**: a pixel counts as differing only if it differs from *every* pixel in the other image's 3×3 neighborhood beyond tolerance (because "two correct renderers disagree on every glyph edge"); runs bidirectionally; **reports but does not gate CI** (worst corpus file, 84% diff, turned out to be a Poppler bug, not a PdfCraft bug). `storytold/pdfcraft:crates/oracle/README.md`.
- **Corpus**: the public **pdf.js test corpus** (958–989 files), pinned via `cargo xtask corpus` — no separate `pdfcraft-corpus` repo (unlike photocraft).
- **Fuzzing, two mechanisms**: (1) a bespoke stable-Rust mutation fuzzer (`cargo xtask fuzz`, xorshift64*-seeded, drives `check-one --edit` end-to-end, runs nightly, auto-minimizes crashes into regression tests); (2) exactly **one** standard libFuzzer/cargo-fuzz target (`crates/filters/fuzz/fuzz_targets/decode.rs`, stream filters only). **No root `fuzz/` dir** (confirmed 404).
- JBIG2 oracle insight: *"Poppler is not a usable oracle for JBIG2"* — on 96 corpus fixtures that should render identically, hayro renders 1 consistent image (correct), Poppler renders 8 variants (blank-paging 7 of them) — pinned without needing an oracle.

**AI**: zero `craft-ai-integrations`; gap #23 ("local AI provider: summarize/ask/alt-text/redaction-suggestions") explicitly tracked, unimplemented, 20–40h. Only on-device ML is OCR via `ocrs`+`rten`.

**Licensing discipline**: `cargo-deny` hard-denies copyleft linking — GPL tools (qpdf/Poppler) only ever shelled out to as external processes, never linked. `storytold/pdfcraft:deny.toml`.

---

## 3. WordCraft (storytold/wordcraft) — commit `00bc6a4b`

**Purpose/maturity** [verified]: Word clone targeting **Microsoft 365 for Mac 16.113.4**, alpha (passes its own 6-point "core workflow" gate but stays alpha), ~150–230h to beta. ~85,600 LOC across **15 documented crates (16 actual — `metafile` is undocumented drift)**. `storytold/wordcraft:docs/architecture.md:14-33`.

**Three-tier progress-doc structure** (a delta from a single ROADMAP.md): `docs/roadmap.md` (milestones, 9.6KB) / `docs/gaps.md` (ranked work, 58KB) / `docs/target-app-parity.md` (43KB) — `ROADMAP.md` itself is just a summary.

### Document model
Retained-mode tree: `Document.body: Vec<Arc<Block>>` where `Block::Para|Table`, Arc-shared for cheap COW undo. **Fields** (TOC/mail-merge/cross-refs) modeled as paired `FieldStart`/`FieldEnd` markers with depth-bounded repair logic. **Track changes** implement full ECMA-376 move-tracking (`w:moveFrom`/`w:moveTo` paired by name). **Styles** support `based_on` inheritance chains (cycle-safe) including conditional table-style banding. A **surprisingly deep equation editor** (OMML tree + LaTeX input + "linear/professional" input, 150KB+ across 5 files) exists this early.

### Text layout engine (high priority)
- **Line breaking is explicitly first-fit/greedy, not Knuth-Plass** — a deliberate fidelity choice because Word's own algorithm is per-line greedy, and a global optimizer would produce *better-looking but Word-incompatible* breaks. `storytold/wordcraft:crates/layout/src/para.rs:1987-2140`.
- **Justification** models a documented Word-specific refinement: Word 2013+ compatibility-mode space-shrinking on justified lines (never with tabs present).
- **Hyphenation**: Liang-pattern dictionaries, hyphenation-zone, consecutive-hyphen limits, explicit per-language exclusions (Vietnamese never hyphenates; Persian patterns never fire on Latin text).
- **Font fallback — directly answers the user's Calibri/Segoe question**: layered substitution — **Carlito↔Calibri, Caladea↔Cambria, Liberation Sans/Serif/Mono↔Arial/Times New Roman/Courier New** confirmed by unit test — then common system fonts, then bundled OFL families, with **per-script locale-aware chains** (Thai: Cordia New on Windows/Thonburi on macOS/Noto Sans Thai on Linux; Hebrew/Persian/Khmer; Symbol/Wingdings/Webdings mapped through a Unicode-equivalent table).
- **Kerning is gated by Word's point-size threshold setting**, not always-on; ligatures controlled per-run via OOXML `w14:ligatures` flags; ligature advances split back across grapheme clusters for correct caret placement.
- **Pagination**: multi-pass iterative layout (page-total fields/footnote renumbering only knowable post-layout), incremental relayout measured at **1.4ms on a 188-page doc**. **Widow/orphan control implements Word's exact "never split 2-and-1" rule**, not a generic minimum.
- **Metric-compatibility gap, explicitly acknowledged (directly relevant to pbiview's font substitution problem)**: *"Aptos (Word's default since 2024) has no metric-compatible open substitute, so new Word documents wrap differently... needs an owner decision (commission or license a metric-matched font)"* — Carlito/Caladea/Liberation solve the Calibri/Cambria/Arial/Times/Courier case but **there is no solved answer for the newest default font family**. `storytold/wordcraft:docs/layout-parity.md`.

### DOCX fidelity
Hand-rolled WordprocessingML reader/writer (quick-xml+zip only), supporting `.docx/.docm/.dotx/.dotm`, Transitional+Strict namespaces, **and Word 2003 XML (WordML 2003)** — a format most clones skip. Agile encryption via pure RustCrypto. **Round-trip testing is exclusively synthetic — no real-world DOCX corpus exists**, an explicit beta-blocking gap #1 (*"DOCX never tested on real-world files... No corpus in the repo; tests are synthetic fixtures"*), a direct consequence of the clean-room policy ("never commit files produced by Word"). Legacy binary `.doc` (MS-DOC via CFB/FIB/piece-table/SPRM) is read-only via a separate `docbin` crate, plus an undocumented `metafile` crate decoding WMF/EMF/DIB for old embedded graphics.

### Undo & command/MCP
Full-snapshot stack (`Arc<Undo>` with Document COW), `MAX_UNDO=500`, typing-coalescing. Same hand-rolled JSON-RPC2.0 MCP pattern (no `rmcp`), and a **documented, unfixed TOCTOU race** in its own path-canonicalization sandbox (explicitly *not* cap-std) — a real security/fidelity tradeoff the team owns up to in comments.

### Testing — the "oracle problem is structurally unsolvable in CI" case study
A purpose-built `cargo xtask visual-diff REF.pdf TEST.pdf` tool computes SSIM, pixel-diff%, and line-break-match% against a **user-exported real-Word PDF that is explicitly never committed** ("files produced by other word processors stay out of the repo") — meaning the fidelity harness is deliberately incapable of running in CI, which directly explains why the real-world-corpus gap remains open. 821 tests + 10 proptest blocks; **no `fuzz/` dir, no fuzzing found anywhere** (confirmed absent by 2 methods).

**AI**: zero AI features, explicitly deferred to "next after beta."

---

## 4. DeckCraft (storytold/deckcraft) — commit `a2435383`

**Purpose/maturity** [verified]: PowerPoint clone, target **PowerPoint for Mac 16.113**, alpha, feature breadth **84%** of 191 tracked features, ~55% "ready," ~120–210h to beta. 202k LOC, 24 crates, 1,641 tests. `storytold/deckcraft:ROADMAP.md:1-45`. (CLAUDE.md is short at 890 bytes purely because it's a pointer file deferring to AGENTS.md + a shared org `craftrules` repo — not a maturity signal.)

**Crate architecture delta**: bundles a **full clean-room video/audio codec stack** (`av1, h264, hevc, vp9, isobmff, matroska, ogg, opus, bitstream` — ~59k LOC, shared in spirit with a sibling FilmCraft) purely to support embedded video in slides/slideshow — something neither a spreadsheet nor photo editor needs.

### Slide model & document
`Presentation{slides, masters, notes_master, handout_master, sections, custom_shows, …}`; `ShapeKind::{Shape,Picture,Group,Connector,Table,Chart,Media,Ink,Model3d,Opaque}` with `Opaque` round-tripping unmodeled PPTX XML (SmartArt, OLE) verbatim. **All 48/48** PowerPoint transitions modeled and **frame-accuracy-tested against real PowerPoint-for-Windows COM-driven video exports** (mean abs pixel diff 35.7→15.0 after tuning) — a genuinely rigorous oracle methodology, though scoped to transitions only. Only **64 of ~190** animation presets implemented.

### Page/canvas layout + text (high priority)
- **Autofit**: a documented binary-search-over-scale algorithm matching PowerPoint's actual behavior, including the specific fidelity detail that **a saved `fontScale` is drawn as-is on open, never re-shrunk**, even if current metrics would call it overflowing (cites internal issue #724).
- **Font fallback**: scripted cascade (requested faces → platform-cascade script fonts for Arabic/Hebrew/Thai/Devanagari → any installed face → first requested face) plus emoji-cluster and ligature-caret handling.
- **Office-font metric-compatible substitution — directly confirmed**, with real source: `substitutes("calibri") = ["Carlito", "Liberation Sans", "Arial", "Inter"]`, `substitutes("cambria") = ["Caladea", ...]`, explicit `has_metric_twin()` flagging Calibri/Cambria/Arial/Times New Roman/Courier New. A subtler fidelity detail: `office_twin()` encodes **observed PowerPoint behavior that PowerPoint itself does NOT recognize Carlito/Caladea/Arimo/Tinos** (draws them as Calibri) — black-box research beyond a naive substitution table. `storytold/deckcraft:crates/fonts/src/fontdb.rs:30-100`.
- **"Selawik" is NOT used anywhere** (zero hits) — Segoe UI/Tahoma/Verdana instead fall back to **Inter**, a non-metric-compatible humanist sans — a materially different (and arguably weaker-fidelity) strategy than Selawik.
- **Shape/vector engine is fully self-owned, not shared cross-product**: `deckcraft-geom` has **~180 ECMA-376 preset shape generators** (113KB single file, the largest source file in the repo) plus all 40 WordArt text-warp presets and Morph-transition path/shape matching.
- **Rendering**: draws directly with `vello_cpu` (CPU rasterize), UI merely uploads the resulting pixmap as a GPU texture — no shared scene-graph abstraction crate.

### Charts, PPTX, Undo, Command/MCP
Charts are **fully native and self-contained** (no shared chart crate with gridcraft — zero references to "gridcraft" found in this repo at all). PPTX hand-rolled from ECMA-376. Undo is `Arc<Presentation>` snapshot-based with cost-aware eviction (500-step default). 320 commands; MCP hand-rolled (no `rmcp`); sandboxing via shared `--automation-read-root/-write-root` convention (same flag *names* as sibling apps, not shared code).

### Testing
Self-authored PPTX corpus manually verified in real PowerPoint once (2026-10-06) — **no external/large real-world corpus, no golden-image rendering pipeline for general slide fidelity** (only the transition-specific COM-export comparison is rigorous). No numeric perf-budget targets found — a less mature perf posture than its siblings. `fuzz/` dir presence unconfirmed (listed but 404'd on fetch).

**AI — significant, explicit delta**: AI dimension scored lowest of all (~10%, tied with localization). Zero `craft-ai-integrations`. What exists instead is **"Design Ideas"** — a deterministic, purely geometric/heuristic feature (4 picture layouts + 3 text layouts chosen by shape-role detection) **explicitly documented as "Everything is computed here, offline"** with zero AI/LLM/network involvement — a rule-based stand-in for PowerPoint Designer, and the parity doc is honest that real Designer is still "no."

---

## 5. DesignCraft (storytold/designcraft) + storytold/indd-utils — commit `e4466230` / `d249788`

**Purpose/maturity** [verified]: InDesign-class DTP app, **pre-alpha** (fails its own alpha gate on one named blocker, #165, est. 5–7h to fix) — the least mature of the five. 126k LOC, 16 crates, 1,122 tests. Feature breadth ~80% (measured two independent ways: a 275-row self-graded checklist AND a literal menu-string diff against InDesign 2026's live scripting DOM — 331/482 = 68.7% match), "ready for real work" ~47%, 420–750h to beta. This dual-measurement rigor is a notable process delta. `storytold/designcraft:docs/target-app-parity.md:9-39`.

**`ASSETS.md`** is a mandatory, CI-enforced asset-provenance ledger (every font/ICC-profile/dictionary/icon must have an author/source/license row, enforced by `cargo xtask assets`) — bans **any** Adobe iconography even under open license.

### Crate architecture
16 crates: `color, compose, doc, engine, epub, fonts, format, geom, idml, images, mcp, pdf, pptx, render, textimport, tools, ui-egui`. Distinctly publishing-flavored crate set vs. a photo editor: `idml`, `pptx` (export), `epub` export, `textimport` (Word/.docx/RTF/.xlsx/Tagged Text). **Nix flake** (`flake.nix`, optional pinned `craft-fonts` input, devShell sets `LD_LIBRARY_PATH` for Vulkan/Wayland) — a genuine reproducible-build story not reported for other siblings.

### Document/page model
`Spread{pages, parent: Option<ParentInfo>}` with multi-level "based-on master" chains; `Story{frames: Vec<ItemId>}` implements true **InDesign text-threading** (frames chained in thread order) with dedicated `text.thread`/`unthread` tools and UI port/thread-line affordances. **5 style kinds** (paragraph/character/object/table/cell). Real **print color management**: ICC profile support via a shared `craft-cms` crate, built-in **Coated FOGRA39** CMYK working space, 4 ICC rendering intents + black-point compensation + ΔE2000 gamut warnings, Adobe Swatch Exchange (.ase) read/write.

### Page/canvas layout + text engine (high priority) — the most algorithmically rigorous of the five
- **Confirmed Knuth–Plass "total fit" paragraph composer** (box/glue/penalty item model) *and* a separate greedy "single-line composer" + balanced-ragged mode — directly matching InDesign's own two-composer concept. `storytold/designcraft:crates/compose/src/breaker.rs:1-9,180-230`.
- **Tiered justification**: word-space stretch/shrink first, then per-glyph letter-spacing, then glyph scaling, with an explicit cost-weighting (`TIER_COST=[1.0,0.75,1.0]`) making letter-spacing cheaper than scaling — the team is explicit: *"same family of algorithm, Adobe's weights unknown; breaks will differ on some paragraphs... no oracle yet."*
- **Optical margin alignment (hanging punctuation)**: a per-glyph protrusion table.
- **Optical kerning is a real outline-based implementation**, not a metrics table: samples each glyph's filled outline into **28 horizontal ink bands**, computes ink gaps per adjacent pair, pulls spacing toward a font-measured reference stem-gap (from `nn`/`HH`/`oo`), with tunable pull/floor/clamp constants and per-(face,glyph) caching. Self-described as *"an approximation of Adobe's algorithm, not a match."*
- **Hyphenation**: Liang patterns for only 5 languages (English/Spanish/Portuguese/Norwegian/Greek) — narrower than InDesign's ~40.
- **Font fallback**: confirmed **two-stage language-then-script disambiguation** with test evidence (zh-Hans→Songti SC, zh-Hant→Songti TC, ko→AppleMyungjo, Hangul-always→Korean font regardless of install order) — but **no "Office-compatible" metric-substitution table was found** (unlike Carlito/Caladea in wordcraft/deckcraft) — DesignCraft's bundled fonts are purely OFL originals (Source Sans 3/Serif 4, Inter, JetBrains Mono), relying on the optional `craft-fonts` bundle for CJK. **[inferred gap]**.
- **Print-PDF fidelity is comparatively strong**: real PDF/X-4 conformance work (`pdfx.rs` writes an `OutputIntent` dict embedding the active CMYK ICC profile + GTS_PDFXVersion/XMP as an incremental update over the shared Krilla PDF writer), crop/bleed marks, DeviceCMYK + spot Separations.

### IDML/INDD & the indd-utils relationship (directly answers the brief)
- **IDML**: full clean-room read+write from the public spec (`export.rs` 157KB, `import.rs` 204KB, `tests.rs` 258KB — largest test file in the repo), with an explicit documented TODO list of what doesn't round-trip yet (tables, footnotes, nested styles, conditions, etc.).
- **Native binary `.indd`/`.indt` is NOT independently implemented — delegated entirely to `storytold/indd-utils`** via a direct git Cargo dependency pinned at the `engine` layer: `.indd` opens by converting **INDD → IDML in memory via indd-utils → the existing IDML importer** (write is `✗` for both `.indd`/`.indt` — DesignCraft never writes native INDD). `storytold/designcraft:Cargo.toml` (workspace deps), `Cargo.lock` (pinned git rev).
- **indd-utils** itself: independent MIT/Apache-2.0 repo, **zero runtime dependencies**, wasm-buildable, supports InDesign versions 3.0–21.6 (both big/little-endian encodings), with its **own** measurement harness (`tools/compare.py`) against a locally-held, sha256-pinned real-InDesign corpus, self-reporting **96.9%–99.67% IDML value coverage on ~650 real file pairs, 0 conversion failures on ~4,300 files**. DesignCraft's own gaps.md is honest that it has **not independently verified** these numbers end-to-end on real files yet — only indd-utils has.

### Testing, undo, command/MCP, AI
No `fuzz/` dir anywhere (confirmed absent). **No oracle-file testing infrastructure exists yet at all** — explicitly stated *"There is no oracle yet"* for typography; IDML/INDD fidelity vs real InDesign is unmeasured; CI even self-admits *"today only path-filtered workflows run on pull requests, so most PRs land with no test run at all."* This is the **least mature testing posture** of the five repos, explicitly planned to piggyback on `photocraft-corpus` infrastructure once built. Undo is `Arc<Document>` snapshot-based (limit 1000) with a global atomic revision counter for cache invalidation and gesture-cancel-safe revision rollback. 446 engine commands + 99 UI-only commands; MCP hand-rolled (no `rmcp`); sandboxing is simple path-prefix containment (not cap-std). **AI**: zero `craft-ai-integrations`; the one AI-adjacent feature (`color.extractTheme`) is explicitly classical median-cut color extraction, not ML.

---

## Comparison table

| Dimension | GridCraft (Excel) | PdfCraft (Acrobat) | WordCraft (Word) | DeckCraft (PowerPoint) | DesignCraft (InDesign) |
|---|---|---|---|---|---|
| **Stage / readiness** | alpha, ~50% ready-for-work | alpha, ~50% ready-for-work | alpha, ~? (passes core-workflow gate) | alpha, ~55% ready | **pre-alpha** (fails own gate), ~47% ready |
| **Crates / LOC** | 12 crates | **31 crates**, 152k LOC | 16 crates, 85.6k LOC | 24 crates, 202k LOC | 16 crates, 126k LOC |
| **Core format parser** | hand-rolled OOXML (xlsx) | hand-rolled COS/PDF object model | hand-rolled OOXML (docx) + legacy .doc (own) | hand-rolled OOXML (pptx) | hand-rolled IDML; **.indd delegated to external `indd-utils` dep** |
| **Rendering pipeline** | egui/epaint galley (inherited caching) | **vendored/patched `hayro` fork + lopdf** — NOT clean-room | vello_cpu rasterizer, own layout | `vello_cpu` direct raster, own shape engine (~180 presets) | vello_cpu + shared Krilla/craft-pdf for PDF export |
| **Text-layout algorithm** | n/a (cell grid, not paragraph flow) | n/a (display only) | **first-fit/greedy** (deliberately Word-faithful) | autofit binary-search + font-scale fidelity | **Knuth–Plass total-fit + greedy single-line + optical kerning/margins** |
| **Metric-compatible fonts** | n/a | n/a | **Carlito↔Calibri, Caladea↔Cambria, Liberation↔Arial/Times/Courier** (verified by unit test); no Aptos answer | same substitution table as WordCraft; **Selawik not used, Inter used for Segoe UI** | **no Office/Adobe metric-substitution table found** — pure OFL originals + optional CJK bundle |
| **Formula/calc engine** | Pratt parser, interval-tree dep graph, topological recalc, dynamic arrays, 504/523 functions | n/a | n/a | n/a | n/a |
| **Excel/real-app oracle corpus** | **absent** (self-acknowledged gap; only a 352-cell TEXT() oracle exists) | **present and rigorous**: qpdf + Poppler oracles, pdf.js 958-file corpus, explicit anti-silent-pass policy | **absent, structurally hard** (clean-room rule forbids committing Word-made fixtures; visual-diff tool exists but can't run in CI) | partial (self-authored corpus, manual PowerPoint-verify once; rigorous only for transitions) | **absent entirely** ("there is no oracle yet"); indd-utils has one, DesignCraft itself doesn't yet use it |
| **Fuzzing** | embedded property-fuzz test, no `fuzz/` dir | custom mutation fuzzer (`xtask fuzz`) + 1 cargo-fuzz target | **none found** | unconfirmed (dir listed, 404'd) | **none found** |
| **Chart engine** | own, toolkit-free `Prim` IR + CPU rasterizer | n/a | n/a | own, fully native (no shared crate w/ gridcraft) | n/a |
| **Pivot/PowerQuery/BI-adjacent** | Pivot: in-house, deep. **Power Query/M engine/data model: absent entirely** | n/a | n/a | n/a | n/a |
| **Command registry size** | 313 | 184 (+141 automation tools) | 422 | 320 (+41 dialogs) | 446 (+99 UI-only) |
| **MCP implementation** | hand-rolled JSON-RPC2.0 (no `rmcp`) | likely same pattern (not deep-verified) | hand-rolled JSON-RPC2.0 (no `rmcp`) | hand-rolled (no `rmcp`) | hand-rolled, explicitly "no async runtime" |
| **Sandbox model** | no cap-std found | shared read/write-root convention | path-canonicalization w/ **documented unfixed TOCTOU race** | shared read/write-root convention | simple path-prefix containment |
| **AI features** | none; MCP-automation only | none; OCR (`ocrs`/`rten`) only ML feature | none, explicitly deferred | none; **rule-based, explicitly-offline "Design Ideas"** stand-in | none; classical median-cut color-theme extraction only |
| **Undo model** | COW `Arc<Workbook>` snapshot, 100-step + 128MiB budget | (not deep-verified) | COW `Arc<Document>` snapshot, 500-step | COW `Arc<Presentation>` snapshot, 500-step, cost-aware eviction | COW `Arc<Document>` snapshot, 1000-step + revision counter |
| **Unique architecture note** | banded-BTreeMap COW cell store; zero Arrow/DataFusion | vendor/ + parity/ dirs; hardest licensing discipline (cargo-deny GPL ban) | full equation editor; Zotero integration; WordML2003 support | bundles a full clean-room video/audio codec stack | Nix flake; delegates hardest format (.indd) to a separate, independently-measured helper repo |

---

## Prioritized, concrete recommendations for pbiview/DashCraft

### Tier 1 — directly actionable, high confidence

1. **Build an oracle/fidelity harness now, before it becomes structurally impossible later.** The single clearest cross-repo lesson: WordCraft and DesignCraft *cannot* easily fix their "no real-app oracle corpus" gaps after the fact, because their clean-room policy forbids committing Office/Adobe-produced fixtures, so the only fidelity-measurement tool they could build (WordCraft's `xtask visual-diff`) is structurally incapable of running in CI. PdfCraft avoided this by adopting a **public, pre-existing, non-proprietary corpus** (pdf.js's 958-file test suite) it could legally commit/pin. **For pbiview**: identify an equivalent public corpus early — e.g., public Power BI sample .pbix files, AdventureWorks-style open datasets, or synthetically-generated .pbix fixtures built in code (gridcraft's own stated fallback: *"build test workbooks in code"*) — and wire a pinned, sha256-verified corpus repo (`storytold/photocraft-corpus` pattern) into CI from day one, rather than retrofitting it at beta.

2. **Adopt PdfCraft's "no silent pass" oracle policy verbatim.** Its concrete rule — a missing external oracle tool is either a declared skip or a hard CI failure, never a silent pass — caught a real historical bug (qpdf absence had silently no-op'd encryption testing for the project's entire history). For DAX/M-query result validation against a reference engine (if one is ever shelled out to in CI), this is a cheap, high-value pattern to copy directly.

3. **PdfCraft's pixel-comparison methodology is directly transferable to chart/visual rendering fidelity testing.** Rather than strict per-pixel diffing (which fails constantly because "two correct renderers disagree on every glyph edge"), compare each pixel against its counterpart's **3×3 neighborhood** within tolerance, run bidirectionally, and treat the result as a **disagreement report, not a pass/fail gate** — this is exactly the right approach for comparing pbiview's scene-graph→SVG/PNG chart renderer against real Power BI Desktop screenshots, where sub-pixel antialiasing/font-hinting differences are expected and not bugs.

4. **For DAX, study GridCraft's dependency-graph/recalc engine as a close structural analog**, not a literal template (DAX's row-context/filter-context model differs from cell-formula dependency graphs, but the *implementation techniques* transfer):
   - A **Pratt/precedence-climbing hand-written parser** (no parser-generator) is a proven, debuggable choice at this scale — consider it over pest/nom for the DAX parser if not already decided.
   - **Interval-tree-style range-dependency indexing** (segment tree over sorted intervals, O(log n+matches)) is the right data structure if DAX measures ever need range/column-level dependency tracking for incremental recalc, analogous to Excel range precedents.
   - **Iterative (not recursive) AST traversal for `Clone`/`walk`/`map`** is a concrete, cheap safety measure worth adopting now if DAX expressions can nest deeply (measure-references-measure chains, nested CALCULATE) — avoids a whole class of stack-overflow crashes for a negligible complexity cost.
   - **Build a small Excel/DAX-computed oracle dataset early, even a narrow one.** GridCraft's 352-cell TEXT()-format oracle (100% passing) proves that a *small, targeted, real-app-generated* oracle for one feature area is far more achievable and valuable early than waiting to build a comprehensive one — do this for DAX function output immediately (export known DAX measure results from real Power BI Desktop for ~50–100 representative formulas across function categories) rather than deferring it the way GridCraft, WordCraft, and DesignCraft all did.

5. **Font-metric-compatibility is a genuinely unsolved problem industry-wide at this stage, not just a pbiview gap — size expectations accordingly.** WordCraft/DeckCraft both solve Calibri/Cambria/Arial/Times/Courier via Carlito/Caladea/Liberation (well-trodden, safe to copy), but **both explicitly admit there is no open metric-compatible substitute for Segoe UI (DeckCraft uses non-metric-compatible Inter) or for Aptos** (Word's newer default) — and DesignCraft has *no* Office-font-substitution table at all. Since pbiview already substitutes Selawik for Segoe UI, that is in fact a **better-than-industry-average** choice already (Selawik is Microsoft's own metric-compatible UI-font clone, unlike Inter) — validate/keep it, and don't expect to find an off-the-shelf answer for DIN/Calibri-exact metrics either; budget this as a genuine "owner decision" (commission or license), exactly as both WordCraft and DeckCraft's own docs frame it, rather than a solvable engineering task.

6. **Separate "visual/text layout cache" from "whole-pane result cache."** GridCraft's scroll-performance answer is *not* a glyph/shaping cache (it inherits egui's for free) but a coarser `(doc_uid, revision, extra)`-keyed cache for whole computed panes (pagination, pivot-fields), invalidated on a monotonic revision counter bump. Since pbiview has no equivalent to egui's built-in text cache (custom scene-graph renderer), the text-shaping+layout cache you already built is the *necessary* layer GridCraft gets for free — but **additionally** adopt the coarser revision-keyed whole-result cache pattern (used identically by GridCraft, DeckCraft, and DesignCraft — all three use an atomic revision counter + `Arc::ptr_eq` structural-sharing to decide what needs recompute) for Power BI-specific expensive recomputations: visual-level DAX query results, table/matrix virtualized-row layout, and DataFusion query plan caching — keyed by `(document_revision, visual_id, filter_context_hash)`.

### Tier 2 — architecture patterns worth evaluating

7. **Copy-on-write, `Arc`-banded/sharded document storage for cheap undo is the universal pattern across all five repos** — every one implements undo as a full-document-snapshot push onto a stack, made affordable purely by `Arc` structural sharing (GridCraft: 64-row bands; WordCraft/DeckCraft/DesignCraft: per-block/per-shape `Arc` sharing), bounded by **both** a step-count limit and a byte-budget computed via `Arc::ptr_eq`-based marginal-cost tracking (not naive full-size accounting). This is directly reusable for pbiview's own document model (report/page/visual tree) — much simpler to implement correctly than operational-transform or event-sourcing, and it's proven at scale across five independent codebases from the same org.

8. **Evaluate a declarative command-registry macro (`CommandSpec`/`cmd!`) with centralized dispatch through `Session::execute(id, params)`**, wrapped in `catch_unwind` for a "never crash" guarantee restoring the pre-command document snapshot on panic. All five repos implement this identically and it is what makes their CLI/MCP/control-channel/UI surfaces trivially consistent. For pbiview's CLI with schemaVersion'd JSON, this pattern maps cleanly: one `CommandSpec` registry feeding the Tauri IPC surface, the CLI, and any future MCP server from a single source of truth.

9. **Hand-roll your own MCP server rather than depending on the `rmcp` SDK**, if the task brief's claim that photocraft uses `rmcp` is accurate — notably, **all four other repos researched here (gridcraft, pdfcraft[likely], wordcraft, deckcraft, designcraft) implement MCP by hand** (newline-delimited JSON-RPC 2.0 over stdio, no async runtime in several cases), each citing it as simpler/more controllable than the generic SDK. This is a 4-of-5 (effectively org-wide) pattern worth taking seriously as the *actual* org convention rather than an outlier choice.

10. **Adopt the layered-crate + `xtask layers` enforcement pattern, but consider GridCraft's stricter variant**: beyond simple numeric layer ordering, add (a) explicit intra-layer DAG chains for same-layer crates with legitimate one-way dependencies, (b) a format-crate allow-list restricting what your file-format-I/O crate (XLSX/DOCX-equivalent for pbiview = .pbix reader/writer) is permitted to depend on, and (c) a "UI-toolkit detector" rule that fails the build if any lower-layer crate accidentally pulls in Tauri/Svelte-adjacent dependencies — this generalizes well to enforcing that DashCraft's DAX/M engines and Arrow/DataFusion storage layer never accidentally depend on the UI layer.

11. **For chart rendering fidelity, GridCraft and DeckCraft both independently concluded that an exact-pixel match to the real app is not worth pursuing** — both explicitly describe their chart "look" as **their own original design**, not a pixel-fidelity target, while still matching chart *semantics*/*types*/*data model* closely. This is a reasonable, pragmatic target to set explicitly for DashCraft's chart renderer (Power BI visual types) rather than chasing unmeasurable "exact Power BI rendering" parity — focus engineering effort on functional/data correctness and a coherent, good-looking native style, and treat visual pixel-matching as explicitly out of scope (document this decision early, as GridCraft did, rather than letting it become an ambiguous unmeasured gap the way PdfCraft's rendering fidelity became).

### Tier 3 — things to explicitly avoid or deprioritize

12. **Don't attempt to build Power Query/M support by hand-waving it as "similar to the formula engine."** GridCraft's complete absence of any Power-Query-equivalent engine — despite being by far its largest single feature gap by estimated hours (60–100h, more than any other single roadmap item) — is a strong signal that a tabular-transform/M-language engine is architecturally distinct work from a cell-formula engine, worth scoping and resourcing as its own multi-week effort from the start (which pbiview is already doing, per the prompt — this confirms that decision is correct and shouldn't be descoped "later" the way GridCraft apparently did).

13. **Budget realistic hours using the five repos' own self-reported methodology, not optimistic guesses.** All five repos use a consistent, auditable "agent-hours to beta / to full parity" estimation method (weighted feature-breadth × readiness-depth scoring, re-measured every few days against a literal menu/feature-list diff of the real app). Given Power BI Desktop is a comparably large or larger surface than any single one of these five apps (DAX+M+report-canvas+visual-gallery+connectors combined roughly spans GridCraft's formula engine + DesignCraft's canvas/layout engine + a Get&Transform engine none of the five apps have built), pbiview should expect multi-hundred-to-thousand agent-hour budgets per major subsystem, consistent with these projects' own numbers, and track progress with the same kind of machine-validated, evidence-cited parity checklist (`cargo xtask parity` pattern) rather than subjective completion percentages.

14. **Treat "AI features" as a legitimate, low-risk differentiator, not a must-have blocker.** None of the five apps ship real in-app AI assistance (all either have zero AI or a purely rule-based, explicitly non-ML stand-in like DeckCraft's "Design Ideas"), despite the org having a dedicated `craft-ai-integrations` shared library available. If DashCraft has any genuine DAX/M-assist AI feature (even a simple natural-language-to-DAX helper), it would be a clear, citable differentiator versus this entire sibling-app cohort — worth highlighting in README/marketing rather than treating as table-stakes to match.

---

### Gaps / what I could not verify directly
- Did not independently clone/build any of the five repos or run their test suites — all findings are static/documentary, read via the GitHub API (consistent with read-only research scope).
- Large generated/derived docs (`ROADMAP.md` 139–212KB, `ATTRIBUTION.toml` 166KB, `parity/acrobat-features.toml` 487KB) were sampled via targeted grep/fetch rather than read end-to-end in every repo; aggregate percentage claims are taken from the repos' own published summary figures rather than independently recomputed.
- Did not verify whether "photocraft uses `rmcp`/cap-std" (stated in the task's context, not independently re-confirmed here) — if that framing is inaccurate, several "architectural delta" claims above (hand-rolled MCP vs `rmcp`, path-sandbox vs cap-std) should be read as "this is what these five repos do," not necessarily as a contrast with photocraft specifically.
- `storytold/craft-fonts`, `storytold/craft-libs`, and `storytold/craft-ai-integrations` were referenced extensively (as Cargo dependencies / absences) but not independently deep-dived in this pass — their own internals (e.g., exactly what `craft-ai-integrations` offers that zero of these five apps chose to use) would be a natural follow-up if useful.
- No git-log/commit-history access was used (GitHub content API only), so "activity cadence" conclusions rely on the dated revision-history entries each project keeps inside its own markdown docs — a reasonably strong but indirect proxy, not direct commit data.